//! Percent-encoding and query-string / cookie parsing.
//!
//! Hand-written on purpose: the only user input this server accepts is the
//! query string and the cookie header, so a whole `url` dependency would buy
//! nothing. Decoding is total (never fails, never panics): a malformed escape
//! stays literal and invalid UTF-8 degrades to U+FFFD.

/// `name=value` pairs from a query string, in order, already decoded.
pub type Query = Vec<(String, String)>;

/// Cookie name → value (last one wins, as browsers send duplicates only by accident).
pub type Cookies = std::collections::HashMap<String, String>;

/// Decode `%XX` escapes and `+` (form-style space). Never fails.
pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                match (hex_value(bytes[index + 1]), hex_value(bytes[index + 2])) {
                    (Some(high), Some(low)) => {
                        out.push(high * 16 + low);
                        index += 3;
                    }
                    _ => {
                        out.push(b'%');
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encode everything outside the RFC 3986 unreserved set, so the result
/// is always safe inside an HTML attribute and a URL.
pub fn percent_encode(input: &str) -> String {
    const HEX: [u8; 16] = *b"0123456789ABCDEF";
    let mut out = String::with_capacity(input.len());
    for &byte in input.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push('%');
                out.push(HEX[(byte >> 4) as usize] as char);
                out.push(HEX[(byte & 0x0f) as usize] as char);
            }
        }
    }
    out
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Parse `a=1&b=2`; empty segments and segments without `=` are ignored.
pub fn parse_query(raw: &str) -> Query {
    let mut pairs = Vec::new();
    for segment in raw.split('&') {
        if segment.is_empty() {
            continue;
        }
        let (name, value) = match segment.split_once('=') {
            Some((name, value)) => (name, value),
            None => (segment, ""),
        };
        pairs.push((percent_decode(name), percent_decode(value)));
    }
    pairs
}

/// First value for `key` (query keys are case-sensitive, as in every server).
pub fn get<'a>(query: &'a Query, key: &str) -> Option<&'a str> {
    query
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

/// Parse the `Cookie` header (`a=b; c=d`).
pub fn parse_cookies(raw: &str) -> Cookies {
    let mut cookies = Cookies::new();
    for segment in raw.split(';') {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        let (name, value) = match segment.split_once('=') {
            Some((name, value)) => (name.trim(), value.trim()),
            None => continue,
        };
        if name.is_empty() {
            continue;
        }
        cookies.insert(name.to_string(), value.to_string());
    }
    cookies
}

/// Cookie value read back as text (values are stored percent-encoded).
pub fn cookie_text<'a>(cookies: &'a Cookies, name: &str) -> Option<String> {
    cookies.get(name).map(|value| percent_decode(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_cjk_escapes() {
        assert_eq!(percent_decode("%E8%A1%9E%E9%9E%85"), "衞鞅");
        assert_eq!(percent_decode("%e8%a1%9e"), "衞");
    }

    #[test]
    fn decodes_plus_as_space_and_keeps_broken_escapes_literal() {
        assert_eq!(percent_decode("a+b"), "a b");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("%4"), "%4");
    }

    #[test]
    fn encode_decode_round_trips() {
        for sample in ["衞鞅亡魏入秦", "a b&c=d", "qin-1-01-p0001", "../..", "囗"] {
            assert_eq!(percent_decode(&percent_encode(sample)), sample);
        }
        assert_eq!(percent_encode("a b"), "a%20b");
        assert_eq!(percent_encode("qin-1"), "qin-1");
    }

    #[test]
    fn parses_query_pairs_in_order() {
        let query = parse_query("p=qin-1-01-p0001&layout=single&empty=&flag");
        assert_eq!(get(&query, "p"), Some("qin-1-01-p0001"));
        assert_eq!(get(&query, "layout"), Some("single"));
        assert_eq!(get(&query, "empty"), Some(""));
        assert_eq!(get(&query, "flag"), Some(""));
        assert_eq!(get(&query, "missing"), None);
        assert!(parse_query("").is_empty());
        assert!(parse_query("&&").is_empty());
    }

    #[test]
    fn query_values_are_decoded_once() {
        let query = parse_query("q=%E8%A1%9E%E9%9E%85%26x");
        assert_eq!(get(&query, "q"), Some("衞鞅&x"));
    }

    #[test]
    fn parses_cookie_header() {
        let cookies = parse_cookies("zgc_last=qin-1-01-p0001; zgc_layout=dual");
        assert_eq!(
            cookies.get("zgc_last").map(String::as_str),
            Some("qin-1-01-p0001")
        );
        assert_eq!(cookie_text(&cookies, "zgc_layout").as_deref(), Some("dual"));
        assert!(parse_cookies("").is_empty());
        assert!(parse_cookies(";;").is_empty());
        assert!(parse_cookies("noequals").is_empty());
    }
}
