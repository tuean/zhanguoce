//! Provider-agnostic client for any OpenAI-compatible chat completions endpoint.
//!
//! Nothing here knows which vendor is behind the URL: base, key, model and the
//! two numeric knobs all come from the environment, and the request body uses
//! only the fields every compatible endpoint documents.
//!
//! The key never leaves this module except as an \`Authorization\` header: it is
//! redacted from \`Debug\`, absent from the startup log (which prints only
//! \`set\`/\`unset\`), and scrubbed out of any upstream error text before it can
//! reach a page.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// Endpoint base used when \`ZGC_AI_BASE\` is unset.
pub const DEFAULT_BASE: &str = "https://api.openai.com/v1";
/// Read/connect timeout when \`ZGC_AI_TIMEOUT_SECS\` is unset.
pub const DEFAULT_TIMEOUT_SECS: u64 = 60;
/// Completion budget when \`ZGC_AI_MAX_TOKENS\` is unset.
pub const DEFAULT_MAX_TOKENS: u32 = 1200;
/// Longest upstream error body kept for the page (characters).
const MAX_DETAIL_CHARS: usize = 300;
/// Longest answer kept for the page (characters); a runaway completion must not
/// turn one request into a megabyte of HTML.
const MAX_ANSWER_CHARS: usize = 20_000;

/// Everything the client needs, resolved once at startup.
#[derive(Clone)]
pub struct AiConfig {
    pub base: String,
    pub key: Option<String>,
    pub model: Option<String>,
    pub timeout: Duration,
    pub max_tokens: u32,
}

impl AiConfig {
    /// Read the configuration from the process environment.
    pub fn from_env() -> AiConfig {
        AiConfig::from_lookup(|name| std::env::var(name).ok())
    }

    /// Same, with an injected lookup — the whole parsing policy is testable
    /// without touching the process environment.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> AiConfig {
        let base = clean(lookup("ZGC_AI_BASE")).unwrap_or_else(|| DEFAULT_BASE.to_string());
        let key = clean(lookup("ZGC_AI_KEY"));
        // No default model on purpose: guessing a model name would fail at the
        // provider instead of telling the reader what to configure.
        let model = clean(lookup("ZGC_AI_MODEL"));
        let timeout = clean(lookup("ZGC_AI_TIMEOUT_SECS"))
            .and_then(|raw| raw.parse::<u64>().ok())
            .filter(|secs| (1..=600).contains(secs))
            .unwrap_or(DEFAULT_TIMEOUT_SECS);
        let max_tokens = clean(lookup("ZGC_AI_MAX_TOKENS"))
            .and_then(|raw| raw.parse::<u32>().ok())
            .filter(|tokens| (1..=32_000).contains(tokens))
            .unwrap_or(DEFAULT_MAX_TOKENS);
        AiConfig {
            base: base.trim_end_matches('/').to_string(),
            key,
            model,
            timeout: Duration::from_secs(timeout),
            max_tokens,
        }
    }

    /// Key configured? Never reveals the value.
    pub fn key_state(&self) -> &'static str {
        if self.key.is_some() {
            "set"
        } else {
            "unset"
        }
    }

    /// Environment variables that still have to be filled in.
    pub fn missing(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if self.key.is_none() {
            missing.push("ZGC_AI_KEY");
        }
        if self.model.is_none() {
            missing.push("ZGC_AI_MODEL");
        }
        missing
    }

    /// True when a request can be attempted at all.
    pub fn ready(&self) -> bool {
        self.missing().is_empty()
    }

    /// Full URL of the chat completions endpoint.
    pub fn endpoint(&self) -> String {
        if self.base.ends_with("/chat/completions") {
            self.base.clone()
        } else {
            format!("{}/chat/completions", self.base)
        }
    }
}

impl fmt::Debug for AiConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiConfig")
            .field("base", &self.base)
            .field("key", &self.key_state())
            .field("model", &self.model)
            .field("timeout", &self.timeout)
            .field("max_tokens", &self.max_tokens)
            .finish()
    }
}

/// One successful completion plus what the page needs to judge it.
#[derive(Debug, Clone)]
pub struct AiReply {
    pub content: String,
    pub model: Option<String>,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub elapsed_ms: u128,
}

/// Anything that can go wrong, already phrased for a human.
#[derive(Debug, Clone)]
pub enum AiError {
    /// The endpoint answered with a non-2xx status.
    Http {
        status: u16,
        detail: String,
    },
    Timeout,
    Transport(String),
    /// 2xx, but the body was not the documented shape.
    Malformed(String),
    /// Called without a key or a model; the page should have explained first.
    NotConfigured,
}

impl AiError {
    /// One sentence for the page. Never contains the key.
    pub fn human(&self) -> String {
        match self {
            AiError::Http { status, detail } => {
                let reason = match status {
                    401 | 403 => "金鑰無效或無權限",
                    404 => "端點路徑不存在（檢查 ZGC_AI_BASE）",
                    429 => "請求過於頻繁或超出配額",
                    500..=599 => "上游服務器故障",
                    _ => "上游拒絕了這次請求",
                };
                if detail.trim().is_empty() {
                    format!("上游返回 {status}：{reason}")
                } else {
                    format!("上游返回 {status}：{reason}。上游說明：{}", detail.trim())
                }
            }
            AiError::Timeout => {
                "上游在限定時間內沒有回應（可用 ZGC_AI_TIMEOUT_SECS 放寬）".to_string()
            }
            AiError::Transport(detail) => {
                format!("無法連接上游（檢查 ZGC_AI_BASE 與網絡）：{detail}")
            }
            AiError::Malformed(detail) => {
                format!("上游回應不是預期的 chat/completions 形狀：{detail}")
            }
            AiError::NotConfigured => "尚未配置 ZGC_AI_KEY 與 ZGC_AI_MODEL".to_string(),
        }
    }
}

/// Ask the endpoint one question. Blocking: the caller runs one request per
/// thread, and the page is only rendered after the answer (or the failure)
/// exists.
pub fn chat(config: &AiConfig, system: &str, user: &str) -> Result<AiReply, AiError> {
    let (Some(key), Some(model)) = (config.key.as_deref(), config.model.as_deref()) else {
        return Err(AiError::NotConfigured);
    };
    let body = json!({
        "model": model,
        "max_tokens": config.max_tokens,
        "temperature": 0.2,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
    })
    .to_string();

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(config.timeout)
        .timeout_read(config.timeout)
        .timeout_write(config.timeout)
        .build();
    let started = Instant::now();
    let response = agent
        .post(&config.endpoint())
        .set("content-type", "application/json")
        .set("accept", "application/json")
        .set("authorization", &format!("Bearer {key}"))
        .send_string(&body);

    match response {
        Ok(response) => {
            let status = response.status();
            let elapsed_ms = started.elapsed().as_millis();
            let body = response
                .into_string()
                .map_err(|err| AiError::Transport(scrub(&err.to_string(), key)))?;
            if !(200..300).contains(&status) {
                return Err(AiError::Http {
                    status,
                    detail: scrub(&summarise(&body), key),
                });
            }
            parse_reply(&body, model, elapsed_ms, key)
        }
        Err(ureq::Error::Status(status, response)) => {
            let body = response.into_string().unwrap_or_default();
            Err(AiError::Http {
                status,
                detail: scrub(&summarise(&body), key),
            })
        }
        Err(ureq::Error::Transport(transport)) => {
            let message = scrub(&transport.to_string(), key);
            let lowered = message.to_ascii_lowercase();
            if lowered.contains("timed out") || lowered.contains("timeout") {
                Err(AiError::Timeout)
            } else {
                Err(AiError::Transport(message))
            }
        }
    }
}

/// Read timeout for the external search step.
pub const SEARCH_TIMEOUT_SECS: u64 = 15;
/// Read timeout for one external fetch.
pub const FETCH_TIMEOUT_SECS: u64 = 20;
/// Longest fetched page kept for the prompt (characters).
pub const MAX_FETCH_CHARS: usize = 3_000;

/// One hit from the provider's web search tool.
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub title: String,
    pub site_name: String,
    pub url: String,
    pub summary: String,
    /// Provider relevance score (0..=3); 1.5 and above is high quality.
    /// Kept so merged result lists can be ranked instead of keeping API order.
    pub rerank_score: f32,
}

/// One fetched page, already truncated.
#[derive(Debug, Clone)]
pub struct FetchedPage {
    pub content: String,
}

/// A source the story page can both cite and link to.
#[derive(Debug, Clone)]
pub struct ExternalSource {
    pub title: String,
    pub site_name: String,
    pub url: String,
    pub summary: String,
    /// Full page text when the fetch succeeded; `None` means the summary is all
    /// the material there is.
    pub content: Option<String>,
}

/// The three outbound calls behind a story reading. Production uses
/// [`HttpClients`]; tests inject a fake so no test touches the network.
pub trait AiClients: Send + Sync {
    fn chat(&self, config: &AiConfig, system: &str, user: &str) -> Result<AiReply, AiError>;
    fn search(
        &self,
        config: &AiConfig,
        query: &str,
        count: usize,
    ) -> Result<Vec<SearchHit>, AiError>;
    fn fetch(&self, config: &AiConfig, url: &str) -> Result<FetchedPage, AiError>;

    /// Run both background queries, merge, drop duplicate URLs, rank by score
    /// and keep the strong ones; fall back to the full merged list when fewer
    /// than three clear the bar, so a weak-but-only result still reaches the
    /// page. A default method so test clients inherit the ranking behaviour
    /// instead of re-implementing it.
    fn search_background(
        &self,
        config: &AiConfig,
        chapter_title: &str,
        count: usize,
    ) -> Result<Vec<SearchHit>, AiError> {
        let mut merged: Vec<SearchHit> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut last_error = None;
        for query in [
            background_query(chapter_title),
            background_query_wikisource(chapter_title),
        ] {
            match self.search(config, &query, count) {
                Ok(hits) => {
                    for hit in hits {
                        if seen.insert(hit.url.clone()) {
                            merged.push(hit);
                        }
                    }
                }
                Err(error) => last_error = Some(error),
            }
        }
        if merged.is_empty() {
            return match last_error {
                Some(error) => Err(error),
                None => Ok(merged),
            };
        }
        merged.sort_by(|left, right| {
            right
                .rerank_score
                .partial_cmp(&left.rerank_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let strong: Vec<SearchHit> = merged
            .iter()
            .filter(|hit| hit.rerank_score >= BACKGROUND_MIN_SCORE)
            .cloned()
            .collect();
        let ranked = if strong.len() >= 3 { strong } else { merged };
        Ok(diversify(ranked, BACKGROUND_LIMIT, BACKGROUND_PER_SITE))
    }
}

/// Real HTTP implementation of [`AiClients`].
pub struct HttpClients;

impl AiClients for HttpClients {
    fn chat(&self, config: &AiConfig, system: &str, user: &str) -> Result<AiReply, AiError> {
        chat(config, system, user)
    }

    fn search(
        &self,
        config: &AiConfig,
        query: &str,
        count: usize,
    ) -> Result<Vec<SearchHit>, AiError> {
        search(config, query, count)
    }

    fn fetch(&self, config: &AiConfig, url: &str) -> Result<FetchedPage, AiError> {
        fetch(config, url)
    }
}

/// `/v1/tools/search` and `/v1/tools/fetch`, derived from the chat base so both
/// calls share one `ZGC_AI_BASE`.
pub fn tools_endpoint(config: &AiConfig, tool: &str) -> String {
    let base = config.base.trim_end_matches('/');
    let base = base.strip_suffix("/chat/completions").unwrap_or(base);
    if base.ends_with("/v1") {
        format!("{base}/tools/{tool}")
    } else {
        format!("{base}/v1/tools/{tool}")
    }
}

/// Query for the story sidebar's 背景史料 block: the chapter name plus the two
/// histories that carry the same story. Kept here so the wording is one string,
/// not a literal buried in the handler.
pub fn background_query(chapter_title: &str) -> String {
    format!("\"{}\" 史記 左傳 戰國策", chapter_title.trim())
        .chars()
        .take(128)
        .collect()
}

/// Second query for the same block: the Wikisource corpus holds the 四庫 and
/// 士禮居 witnesses that the general query rarely surfaces. The title is quoted
/// so it matches as a phrase rather than as loose characters.
pub fn background_query_wikisource(chapter_title: &str) -> String {
    format!("\"{}\" site:zh.wikisource.org", chapter_title.trim())
        .chars()
        .take(128)
        .collect()
}

/// Lowest provider score kept once a merged list is long enough to be picky.
const BACKGROUND_MIN_SCORE: f32 = 1.2;
/// How many hits the 背景史料 block shows.
pub const BACKGROUND_LIMIT: usize = 6;
/// At most this many hits from one site: a phrase query against a single corpus
/// otherwise fills the whole block with its own pages.
const BACKGROUND_PER_SITE: usize = 2;

/// Keep the ranked order but cap how many entries one site may contribute,
/// backfilling from the leftovers when the caps leave room.
fn diversify(hits: Vec<SearchHit>, limit: usize, per_site: usize) -> Vec<SearchHit> {
    let mut picked: Vec<SearchHit> = Vec::new();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for hit in &hits {
        if picked.len() >= limit {
            break;
        }
        let seen = counts.entry(hit.site_name.as_str()).or_insert(0);
        if *seen < per_site {
            *seen += 1;
            picked.push(hit.clone());
        }
    }
    picked
}

/// Ask the provider's search tool. Blocking like [`chat`].
pub fn search(config: &AiConfig, query: &str, count: usize) -> Result<Vec<SearchHit>, AiError> {
    let Some(key) = config.key.as_deref() else {
        return Err(AiError::NotConfigured);
    };
    let count = count.clamp(1, 50);
    let body = json!({
        "query": query,
        "count": count,
        "need_content": false,
        "ori_question": query,
    })
    .to_string();
    let timeout = config.timeout.min(Duration::from_secs(SEARCH_TIMEOUT_SECS));
    let text = post_json(&tools_endpoint(config, "search"), key, &body, timeout)?;
    parse_search(&text, key)
}

/// Fetch one page through the provider's tool. Blocking like [`chat`].
pub fn fetch(config: &AiConfig, url: &str) -> Result<FetchedPage, AiError> {
    let Some(key) = config.key.as_deref() else {
        return Err(AiError::NotConfigured);
    };
    let body = json!({ "url": url }).to_string();
    let timeout = config.timeout.min(Duration::from_secs(FETCH_TIMEOUT_SECS));
    let text = post_json(&tools_endpoint(config, "fetch"), key, &body, timeout)?;
    parse_fetched(&text, key)
}

fn post_json(url: &str, key: &str, body: &str, timeout: Duration) -> Result<String, AiError> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(timeout)
        .timeout_read(timeout)
        .timeout_write(timeout)
        .build();
    let response = agent
        .post(url)
        .set("content-type", "application/json")
        .set("accept", "application/json")
        .set("authorization", &format!("Bearer {key}"))
        .send_string(body);
    match response {
        Ok(response) => {
            let status = response.status();
            let text = response
                .into_string()
                .map_err(|err| AiError::Transport(scrub(&err.to_string(), key)))?;
            if (200..300).contains(&status) {
                Ok(text)
            } else {
                Err(AiError::Http {
                    status,
                    detail: scrub(&summarise(&text), key),
                })
            }
        }
        Err(ureq::Error::Status(status, response)) => {
            let text = response.into_string().unwrap_or_default();
            Err(AiError::Http {
                status,
                detail: scrub(&summarise(&text), key),
            })
        }
        Err(ureq::Error::Transport(transport)) => {
            let message = scrub(&transport.to_string(), key);
            let lowered = message.to_ascii_lowercase();
            if lowered.contains("timed out") || lowered.contains("timeout") {
                Err(AiError::Timeout)
            } else {
                Err(AiError::Transport(message))
            }
        }
    }
}

fn parse_search(body: &str, key: &str) -> Result<Vec<SearchHit>, AiError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| AiError::Malformed("檢索回應不是 JSON".to_string()))?;
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| AiError::Malformed("檢索回應缺少 results".to_string()))?;
    let mut hits = Vec::with_capacity(results.len());
    for item in results {
        let Some(url) = item.get("url").and_then(Value::as_str) else {
            continue;
        };
        let url = url.trim();
        if url.is_empty() {
            continue;
        }
        hits.push(SearchHit {
            title: text_field(item, "title", "（無標題）"),
            site_name: text_field(item, "site_name", "（未署站點）"),
            url: url.to_string(),
            summary: scrub(&text_field(item, "summary", ""), key),
            rerank_score: item
                .get("rerank_score")
                .and_then(Value::as_f64)
                .unwrap_or(0.0) as f32,
        });
    }
    Ok(hits)
}

fn parse_fetched(body: &str, key: &str) -> Result<FetchedPage, AiError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| AiError::Malformed("抓取回應不是 JSON".to_string()))?;
    let content = value
        .get("content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|content| !content.is_empty())
        .ok_or_else(|| AiError::Malformed("抓取回應缺少 content".to_string()))?;
    let content: String = content.chars().take(MAX_FETCH_CHARS).collect();
    Ok(FetchedPage {
        content: scrub(&content, key),
    })
}

fn text_field(value: &Value, name: &str, fallback: &str) -> String {
    value
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn parse_reply(
    body: &str,
    requested_model: &str,
    elapsed_ms: u128,
    key: &str,
) -> Result<AiReply, AiError> {
    let value: Value =
        serde_json::from_str(body).map_err(|_| AiError::Malformed("回應不是 JSON".to_string()))?;
    if let Some(message) = value
        .pointer("/error/message")
        .and_then(Value::as_str)
        .map(|message| summarise(message))
    {
        return Err(AiError::Malformed(format!(
            "上游錯誤：{}",
            scrub(&message, key)
        )));
    }
    let content = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if content.is_empty() {
        return Err(AiError::Malformed(
            "缺少 choices[0].message.content".to_string(),
        ));
    }
    let content: String = content.chars().take(MAX_ANSWER_CHARS).collect();
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|model| !model.trim().is_empty())
        .unwrap_or_else(|| requested_model.to_string());
    let prompt_tokens = value
        .pointer("/usage/prompt_tokens")
        .and_then(Value::as_u64);
    let completion_tokens = value
        .pointer("/usage/completion_tokens")
        .and_then(Value::as_u64);
    Ok(AiReply {
        content,
        model: Some(model),
        prompt_tokens,
        completion_tokens,
        elapsed_ms,
    })
}

/// Trim an environment value; empty means "not set".
fn clean(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// One line, bounded: upstream error bodies are sometimes whole HTML pages.
fn summarise(body: &str) -> String {
    let flat: String = body
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .take(MAX_DETAIL_CHARS)
        .collect();
    flat.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Remove the key from anything that may reach a page or a log line.
fn scrub(text: &str, key: &str) -> String {
    if key.is_empty() {
        return text.to_string();
    }
    text.replace(key, "[redacted]")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn config(base: String, key: &str, model: &str) -> AiConfig {
        AiConfig {
            base,
            key: Some(key.to_string()),
            model: Some(model.to_string()),
            timeout: Duration::from_millis(400),
            max_tokens: 64,
        }
    }

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| {
            owned
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        }
    }

    /// One-shot HTTP/1.1 server: answers exactly one request, then exits.
    fn serve(response: Vec<u8>, delay: Option<Duration>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while stream.read(&mut byte).unwrap_or(0) == 1 {
                head.push(byte[0]);
                if head.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let text = String::from_utf8_lossy(&head).to_ascii_lowercase();
            let length = text
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            let mut body = vec![0u8; length];
            let _ = stream.read_exact(&mut body);
            if let Some(delay) = delay {
                thread::sleep(delay);
            }
            let _ = stream.write_all(&response);
            let _ = stream.flush();
        });
        format!("http://127.0.0.1:{port}/v1")
    }

    fn reply(status: &str, body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    fn ok_body() -> String {
        r#"{"model":"mock-model-2","choices":[{"message":{"role":"assistant","content":"此處史料不足。"}}],"usage":{"prompt_tokens":123,"completion_tokens":7}}"#.to_string()
    }

    #[test]
    fn defaults_are_provider_neutral() {
        let config = AiConfig::from_lookup(|_| None);
        assert_eq!(config.base, DEFAULT_BASE);
        assert_eq!(config.timeout, Duration::from_secs(60));
        assert_eq!(config.max_tokens, 1200);
        assert_eq!(config.key_state(), "unset");
        assert_eq!(config.missing(), vec!["ZGC_AI_KEY", "ZGC_AI_MODEL"]);
        assert!(!config.ready());
        assert_eq!(
            config.endpoint(),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn environment_overrides_are_trimmed_and_bounded() {
        let config = AiConfig::from_lookup(lookup(&[
            ("ZGC_AI_BASE", " http://127.0.0.1:9999/v1/ "),
            ("ZGC_AI_KEY", " sk-test "),
            ("ZGC_AI_MODEL", "mock-model"),
            ("ZGC_AI_TIMEOUT_SECS", "5"),
            ("ZGC_AI_MAX_TOKENS", "77"),
        ]));
        assert_eq!(config.base, "http://127.0.0.1:9999/v1");
        assert_eq!(config.key.as_deref(), Some("sk-test"));
        assert_eq!(config.timeout, Duration::from_secs(5));
        assert_eq!(config.max_tokens, 77);
        assert!(config.ready());
        assert_eq!(
            config.endpoint(),
            "http://127.0.0.1:9999/v1/chat/completions"
        );
    }

    #[test]
    fn nonsense_numbers_fall_back_instead_of_panicking() {
        let config = AiConfig::from_lookup(lookup(&[
            ("ZGC_AI_TIMEOUT_SECS", "0"),
            ("ZGC_AI_MAX_TOKENS", "-4"),
        ]));
        assert_eq!(config.timeout, Duration::from_secs(DEFAULT_TIMEOUT_SECS));
        assert_eq!(config.max_tokens, DEFAULT_MAX_TOKENS);
    }

    #[test]
    fn debug_never_shows_the_key() {
        let config = config("http://127.0.0.1:1/v1".to_string(), "sk-secret", "m");
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("sk-secret"), "{rendered}");
        assert!(rendered.contains("key: \"set\""), "{rendered}");
    }

    #[test]
    fn a_full_url_is_used_as_is() {
        let config =
            AiConfig::from_lookup(lookup(&[("ZGC_AI_BASE", "http://x/v1/chat/completions")]));
        assert_eq!(config.endpoint(), "http://x/v1/chat/completions");
    }

    #[test]
    fn a_successful_completion_is_parsed() {
        let base = serve(reply("200 OK", &ok_body()), None);
        let reply = chat(&config(base, "sk-test", "requested"), "系統", "問題").expect("ok");
        assert_eq!(reply.content, "此處史料不足。");
        assert_eq!(reply.model.as_deref(), Some("mock-model-2"));
        assert_eq!(reply.prompt_tokens, Some(123));
        assert_eq!(reply.completion_tokens, Some(7));
    }

    #[test]
    fn a_401_is_explained_and_never_echoes_the_key() {
        let body = r#"{"error":{"message":"Incorrect API key provided: sk-secret-123"}}"#;
        let base = serve(reply("401 Unauthorized", body), None);
        let error = chat(&config(base, "sk-secret-123", "m"), "s", "u").expect_err("401");
        let human = error.human();
        assert!(human.contains("401"), "{human}");
        assert!(human.contains("金鑰"), "{human}");
        assert!(!human.contains("sk-secret-123"), "{human}");
        assert!(human.contains("[redacted]"), "{human}");
    }

    #[test]
    fn a_500_is_reported_as_an_upstream_fault() {
        let base = serve(
            reply("500 Internal Server Error", "{\"error\":\"boom\"}"),
            None,
        );
        let error = chat(&config(base, "sk-test", "m"), "s", "u").expect_err("500");
        match &error {
            AiError::Http { status, .. } => assert_eq!(*status, 500),
            other => panic!("{other:?}"),
        }
        assert!(
            error.human().contains("上游服務器故障"),
            "{}",
            error.human()
        );
    }

    #[test]
    fn a_non_json_body_is_malformed_not_a_panic() {
        let base = serve(reply("200 OK", "<html>proxy says hi</html>"), None);
        let error = chat(&config(base, "sk-test", "m"), "s", "u").expect_err("html");
        assert!(matches!(error, AiError::Malformed(_)), "{error:?}");
        assert!(error.human().contains("不是 JSON"), "{}", error.human());
    }

    #[test]
    fn a_missing_content_field_is_malformed() {
        let base = serve(reply("200 OK", r#"{"choices":[]}"#), None);
        let error = chat(&config(base, "sk-test", "m"), "s", "u").expect_err("empty");
        assert!(
            error.human().contains("choices[0].message.content"),
            "{}",
            error.human()
        );
    }

    #[test]
    fn a_stalled_endpoint_times_out() {
        let base = serve(
            reply("200 OK", &ok_body()),
            Some(Duration::from_millis(1_500)),
        );
        let error = chat(&config(base, "sk-test", "m"), "s", "u").expect_err("timeout");
        assert!(matches!(error, AiError::Timeout), "{error:?}");
        assert!(error.human().contains("沒有回應"), "{}", error.human());
    }

    #[test]
    fn a_refused_connection_is_a_transport_error() {
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            listener.local_addr().expect("addr").port()
        };
        let base = format!("http://127.0.0.1:{port}/v1");
        let error = chat(&config(base, "sk-test", "m"), "s", "u").expect_err("refused");
        assert!(matches!(error, AiError::Transport(_)), "{error:?}");
    }

    #[test]
    fn an_incomplete_configuration_is_refused_before_the_network() {
        let config = AiConfig::from_lookup(|_| None);
        assert!(matches!(
            chat(&config, "s", "u"),
            Err(AiError::NotConfigured)
        ));
    }

    #[test]
    fn error_details_are_flattened_and_bounded() {
        let long = format!("{}\n\n{}", "x".repeat(MAX_DETAIL_CHARS * 2), "y".repeat(50));
        let summary = summarise(&long);
        assert!(summary.chars().count() <= MAX_DETAIL_CHARS);
        assert!(!summary.contains('\n'));
    }

    #[test]
    fn scrub_removes_every_occurrence() {
        assert_eq!(scrub("a sk-1 b sk-1", "sk-1"), "a [redacted] b [redacted]");
        assert_eq!(scrub("nothing", "sk-1"), "nothing");
    }

    /// Records the queries it was asked for and answers with per-query hits, so
    /// the merged ranking can be checked without a network.
    struct QueryClients {
        asked: std::sync::Mutex<Vec<String>>,
        answers: Vec<(String, Vec<SearchHit>)>,
    }

    impl AiClients for QueryClients {
        fn chat(&self, _: &AiConfig, _: &str, _: &str) -> Result<AiReply, AiError> {
            Err(AiError::NotConfigured)
        }

        fn search(&self, _: &AiConfig, query: &str, _: usize) -> Result<Vec<SearchHit>, AiError> {
            self.asked.lock().expect("ask lock").push(query.to_string());
            for (needle, hits) in &self.answers {
                if query.contains(needle.as_str()) {
                    return Ok(hits.clone());
                }
            }
            Ok(Vec::new())
        }

        fn fetch(&self, _: &AiConfig, _: &str) -> Result<FetchedPage, AiError> {
            Err(AiError::NotConfigured)
        }
    }

    fn ranked(url: &str, score: f32) -> SearchHit {
        // One distinct site per URL, so the per-site cap does not interfere
        // with ranking assertions; the cap has its own test.
        SearchHit {
            title: url.to_string(),
            site_name: url.split('/').nth(2).unwrap_or("站").to_string(),
            url: url.to_string(),
            summary: String::new(),
            rerank_score: score,
        }
    }

    #[test]
    fn background_merges_both_queries_ranks_by_score_and_dedupes() {
        let clients = QueryClients {
            asked: std::sync::Mutex::new(Vec::new()),
            answers: vec![
                (
                    "site:zh.wikisource.org".to_string(),
                    vec![
                        ranked("https://a.example/1", 1.66),
                        ranked("https://b.example/2", 0.24),
                    ],
                ),
                (
                    "史記".to_string(),
                    vec![
                        ranked("https://a.example/1", 1.66),
                        ranked("https://c.example/3", 1.42),
                        ranked("https://e.example/5", 1.3),
                        ranked("https://d.example/4", 0.5),
                    ],
                ),
            ],
        };
        let picked = clients
            .search_background(&AiConfig::from_lookup(|_| None), "蘇秦始將連橫", 8)
            .expect("merged search");
        let urls: Vec<&str> = picked.iter().map(|hit| hit.url.as_str()).collect();
        // Three hits clear the bar, so the two weak ones are dropped and the
        // Wikisource-only hit never appears.
        assert_eq!(
            urls,
            [
                "https://a.example/1",
                "https://c.example/3",
                "https://e.example/5"
            ]
        );
        let asked = clients.asked.lock().expect("ask lock").clone();
        assert_eq!(asked.len(), 2, "one query per endpoint form: {asked:?}");
        assert!(asked.iter().any(|q| q.contains("site:zh.wikisource.org")));
        assert!(asked.iter().all(|q| q.starts_with('"')));
    }

    #[test]
    fn background_caps_one_site_at_two_entries() {
        let mut hits = Vec::new();
        for index in 0..4 {
            let mut hit = ranked(
                &format!("https://wiki.example/{index}"),
                1.8 - index as f32 * 0.1,
            );
            hit.site_name = "维基文库".to_string();
            hits.push(hit);
        }
        let mut other = ranked("https://ctext.example/9", 1.1);
        other.site_name = "Chinese Text Project".to_string();
        hits.push(other);
        let picked = diversify(hits, 6, BACKGROUND_PER_SITE);
        let wiki = picked
            .iter()
            .filter(|hit| hit.site_name == "维基文库")
            .count();
        assert_eq!(wiki, 2, "one site must not own the block");
        assert_eq!(picked.len(), 3, "a capped site is skipped, not backfilled");
        assert_eq!(picked[0].url, "https://wiki.example/0");
    }

    #[test]
    fn background_keeps_weak_hits_when_nothing_is_strong() {
        let clients = QueryClients {
            asked: std::sync::Mutex::new(Vec::new()),
            answers: vec![(
                "史記".to_string(),
                vec![
                    ranked("https://a.example/1", 0.9),
                    ranked("https://b.example/2", 0.2),
                ],
            )],
        };
        let picked = clients
            .search_background(&AiConfig::from_lookup(|_| None), "某章", 8)
            .expect("merged search");
        assert_eq!(
            picked.len(),
            2,
            "weak-only list must survive the score filter"
        );
        assert_eq!(picked[0].url, "https://a.example/1");
    }
}
