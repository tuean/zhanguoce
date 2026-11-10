//! A minimal HTTP/1.1 server-side slice: read one request head, write one
//! response, close the connection. No keep-alive, no chunking, no body parsing.
//!
//! Deliberately std-only: TcpListener + Read + Write, no web framework.

use std::io::{ErrorKind, Read};

/// Upper bound for a request head; anything larger is rejected with 431.
pub const MAX_REQUEST_BYTES: usize = 16 * 1024;

/// A parsed request head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// Upper-cased method (GET, POST, …).
    pub method: String,
    /// Path without the query string.
    pub path: String,
    /// Raw (still percent-encoded) query string, empty when absent.
    pub query: String,
    /// Header names are lower-cased; values are trimmed.
    pub headers: Vec<(String, String)>,
}

impl Request {
    /// Case-insensitive header lookup.
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }

    /// Build a request without a socket (the server's parser reuses this, and
    /// tests build requests with it directly).
    pub fn from_target(method: &str, target: &str) -> Request {
        let (path, query) = match target.split_once('?') {
            Some((path, query)) => (path.to_string(), query.to_string()),
            None => (target.to_string(), String::new()),
        };
        Request {
            method: method.to_ascii_uppercase(),
            path,
            query,
            headers: Vec::new(),
        }
    }

    /// Shorthand for a GET request built from a target like /read?p=x.
    #[cfg(test)]
    pub fn get(target: &str) -> Request {
        Request::from_target("GET", target)
    }

    /// Attach a header (builder style, for tests).
    #[cfg(test)]
    pub fn with_header(mut self, name: &str, value: &str) -> Request {
        self.headers
            .push((name.to_ascii_lowercase(), value.to_string()));
        self
    }
}

/// Why reading a request head failed.
#[derive(Debug, PartialEq, Eq)]
pub enum ReadError {
    /// The peer closed without sending anything — normal for port probes.
    Empty,
    /// Head exceeded MAX_REQUEST_BYTES.
    TooLarge,
    /// Head was unparsable or truncated.
    Malformed,
    /// Socket-level failure.
    Io,
}

impl ReadError {
    pub fn status(&self) -> u16 {
        match self {
            ReadError::TooLarge => 431,
            _ => 400,
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            ReadError::Empty => "空請求",
            ReadError::TooLarge => "請求頭部過大",
            ReadError::Malformed => "請求格式無法解析",
            ReadError::Io => "讀取請求失敗",
        }
    }
}

/// Read and parse one request head (bytes up to and including the blank line).
pub fn read_request(stream: &mut impl Read) -> Result<Request, ReadError> {
    let mut buffer: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        if let Some(end) = head_end(&buffer) {
            if end > MAX_REQUEST_BYTES {
                return Err(ReadError::TooLarge);
            }
            return parse_head(&buffer[..end]);
        }
        if buffer.len() > MAX_REQUEST_BYTES {
            return Err(ReadError::TooLarge);
        }
        match stream.read(&mut chunk) {
            Ok(0) => {
                return if buffer.is_empty() {
                    Err(ReadError::Empty)
                } else {
                    Err(ReadError::Malformed)
                };
            }
            Ok(count) => buffer.extend_from_slice(&chunk[..count]),
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return Err(ReadError::Io),
        }
    }
}

/// Byte offset just past the blank line that ends the request head.
fn head_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
        .or_else(|| {
            buffer
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|index| index + 2)
        })
}

fn parse_head(head: &[u8]) -> Result<Request, ReadError> {
    let text = String::from_utf8_lossy(head);
    let mut lines = text.split('\n').map(|line| line.trim_end_matches('\r'));
    let request_line = lines.next().ok_or(ReadError::Malformed)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or(ReadError::Malformed)?;
    let target = parts.next().ok_or(ReadError::Malformed)?;
    let mut request = Request::from_target(method, target);
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            request
                .headers
                .push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    Ok(request)
}

/// One response: status + body, plus extra headers (Set-Cookie, Allow, …).
#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Response {
    pub fn html(status: u16, body: String) -> Response {
        Response {
            status,
            content_type: "text/html; charset=utf-8",
            headers: Vec::new(),
            body,
        }
    }

    pub fn text(status: u16, body: String) -> Response {
        Response {
            status,
            content_type: "text/plain; charset=utf-8",
            headers: Vec::new(),
            body,
        }
    }

    pub fn empty(status: u16) -> Response {
        Response {
            status,
            content_type: "text/plain; charset=utf-8",
            headers: Vec::new(),
            body: String::new(),
        }
    }

    pub fn with_header(mut self, name: &str, value: impl Into<String>) -> Response {
        self.headers.push((name.to_string(), value.into()));
        self
    }

    /// Serialize status line + headers + body into the exact bytes to write.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut head = String::with_capacity(256);
        head.push_str("HTTP/1.1 ");
        head.push_str(&self.status.to_string());
        head.push(' ');
        head.push_str(status_text(self.status));
        head.push_str("\r\n");
        head.push_str("Content-Type: ");
        head.push_str(self.content_type);
        head.push_str("\r\n");
        head.push_str("Content-Length: ");
        head.push_str(&self.body.len().to_string());
        head.push_str("\r\n");
        head.push_str("Cache-Control: no-store\r\n");
        head.push_str("X-Content-Type-Options: nosniff\r\n");
        head.push_str("Referrer-Policy: no-referrer\r\n");
        for (name, value) in &self.headers {
            head.push_str(name);
            head.push_str(": ");
            head.push_str(value);
            head.push_str("\r\n");
        }
        head.push_str("Connection: close\r\n\r\n");
        let mut bytes = head.into_bytes();
        bytes.extend_from_slice(self.body.as_bytes());
        bytes
    }
}

pub fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        431 => "Request Header Fields Too Large",
        502 => "Bad Gateway",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn reads_request_line_and_headers() {
        let raw = b"GET /read?p=qin-1-01-p0001 HTTP/1.1\r\nHost: 127.0.0.1\r\nCookie: zgc_last=x; zgc_layout=dual\r\n\r\n";
        let request = read_request(&mut Cursor::new(raw.to_vec())).unwrap();
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/read");
        assert_eq!(request.query, "p=qin-1-01-p0001");
        assert_eq!(request.header("Host"), Some("127.0.0.1"));
        assert_eq!(
            request.header("cookie"),
            Some("zgc_last=x; zgc_layout=dual")
        );
        assert_eq!(request.header("Missing"), None);
    }

    #[test]
    fn accepts_bare_lf_headers() {
        let raw = b"GET / HTTP/1.1\nHost: localhost\n\n";
        let request = read_request(&mut Cursor::new(raw.to_vec())).unwrap();
        assert_eq!(request.path, "/");
        assert_eq!(request.query, "");
    }

    #[test]
    fn empty_connection_is_not_an_error_page() {
        assert_eq!(
            read_request(&mut Cursor::new(Vec::new())),
            Err(ReadError::Empty)
        );
    }

    #[test]
    fn truncated_head_is_malformed() {
        let raw = b"GET / HTTP/1.1\r\nHost: localhost\r\n";
        assert_eq!(
            read_request(&mut Cursor::new(raw.to_vec())),
            Err(ReadError::Malformed)
        );
    }

    #[test]
    fn oversized_head_is_rejected() {
        let raw = format!(
            "GET / HTTP/1.1\r\nX: {}\r\n\r\n",
            "a".repeat(MAX_REQUEST_BYTES + 32)
        );
        assert_eq!(
            read_request(&mut Cursor::new(raw.into_bytes())),
            Err(ReadError::TooLarge)
        );
    }

    #[test]
    fn response_serializes_content_length_in_bytes() {
        let response = Response::html(200, "衞鞅".to_string());
        let text = String::from_utf8(response.to_bytes()).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("Content-Type: text/html; charset=utf-8\r\n"));
        assert!(
            text.contains("Content-Length: 6\r\n"),
            "byte length, not char count"
        );
        assert!(text.contains("Connection: close\r\n\r\n"));
        assert!(text.ends_with("衞鞅"));
    }

    #[test]
    fn from_target_splits_path_and_query() {
        let request = Request::get("/search?q=%E8%A1%9E&group=qin");
        assert_eq!(request.path, "/search");
        assert_eq!(request.query, "q=%E8%A1%9E&group=qin");
        assert_eq!(Request::get("/").query, "");
    }
}
