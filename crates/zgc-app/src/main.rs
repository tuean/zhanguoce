//! zgc-app — the server-rendered reading application for the 戰國策 study app.
//!
//! One binary: load the book once, build the search index once, then answer
//! GET requests over a hand-written HTTP/1.1 loop bound to 127.0.0.1.
//!
//! Deliberately plain: no web framework, no front-end framework, no wasm, no
//! JavaScript on the page. The only dependency is the UI-agnostic zgc-core.

#![forbid(unsafe_code)]

mod ai;
mod app;
mod html;
mod http;
mod pages;
mod prefs;
#[cfg(test)]
mod real_data_tests;
mod urlcodec;

use std::io::Write;
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use zgc_core::{
    Book, ChapterYears, Entities, EntityKind, Events, Strategy, Variants, CHAPTER_YEAR_FILE,
    EVENT_FILE,
};

/// Default port: high, unprivileged, not 80/443.
const DEFAULT_PORT: u16 = 8099;
/// Default data directory, relative to where the binary is started.
const DEFAULT_DATA_DIR: &str = "data/zhanguoce";

const USAGE: &str = "\
用法：zgc-app [--port <端口>] [--data-dir <書目目錄>]

  --port <端口>          監聽端口（默認 8099，僅綁定 127.0.0.1）
  --data-dir <目錄>      書目目錄，須含 book.json 與 volumes/（默認 data/zhanguoce）
  -h, --help             顯示本說明

環境變量：ZGC_PORT、ZGC_DATA_DIR（命令行參數優先）
          ZGC_AI_BASE（默認 https://api.openai.com/v1）、ZGC_AI_KEY、ZGC_AI_MODEL、
          ZGC_AI_TIMEOUT_SECS（默認 60）、ZGC_AI_MAX_TOKENS（默認 1200）";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(MainError::Usage(message)) => {
            eprintln!("參數錯誤：{message}\n\n{USAGE}");
            ExitCode::from(2)
        }
        Err(MainError::Fatal(message)) => {
            eprintln!("錯誤：{message}");
            ExitCode::FAILURE
        }
    }
}

enum MainError {
    /// Bad command line; usage is printed alongside.
    Usage(String),
    /// Cannot start (data, port, …).
    Fatal(String),
}

#[derive(Debug)]
enum ConfigError {
    Help,
    Invalid(String),
}

struct Config {
    port: u16,
    data_dir: PathBuf,
}

impl Config {
    fn parse<I: Iterator<Item = String>>(args: I) -> Result<Config, ConfigError> {
        Config::parse_with(args, |name| std::env::var(name).ok())
    }

    fn parse_with<I: IntoIterator<Item = String>>(
        args: I,
        get_env: impl Fn(&str) -> Option<String>,
    ) -> Result<Config, ConfigError> {
        let mut args = args.into_iter();
        let mut port = clean_env(&get_env, "ZGC_PORT");
        let mut data_dir = clean_env(&get_env, "ZGC_DATA_DIR");
        while let Some(argument) = args.next() {
            match argument.as_str() {
                "-h" | "--help" => return Err(ConfigError::Help),
                "--port" => {
                    port =
                        Some(args.next().ok_or_else(|| {
                            ConfigError::Invalid("--port 後面缺少端口號".to_string())
                        })?)
                }
                "--data-dir" => {
                    data_dir = Some(args.next().ok_or_else(|| {
                        ConfigError::Invalid("--data-dir 後面缺少目錄".to_string())
                    })?)
                }
                other if other.starts_with("--port=") => {
                    port = Some(other["--port=".len()..].to_string())
                }
                other if other.starts_with("--data-dir=") => {
                    data_dir = Some(other["--data-dir=".len()..].to_string())
                }
                other => {
                    return Err(ConfigError::Invalid(format!(
                        "未知參數「{other}」（試試 --help）"
                    )))
                }
            }
        }
        let port = match port {
            Some(raw) => raw.trim().parse::<u16>().map_err(|_| {
                ConfigError::Invalid(format!("端口必須是 1–65535 的整數，收到「{raw}」"))
            })?,
            None => DEFAULT_PORT,
        };
        if port == 0 {
            return Err(ConfigError::Invalid("端口不能是 0".to_string()));
        }
        Ok(Config {
            port,
            data_dir: PathBuf::from(data_dir.unwrap_or_else(|| DEFAULT_DATA_DIR.to_string())),
        })
    }
}

fn clean_env(get_env: &impl Fn(&str) -> Option<String>, name: &str) -> Option<String> {
    get_env(name)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn run() -> Result<(), MainError> {
    let config = match Config::parse(std::env::args().skip(1)) {
        Ok(config) => config,
        Err(ConfigError::Help) => {
            println!("{USAGE}");
            return Ok(());
        }
        Err(ConfigError::Invalid(message)) => return Err(MainError::Usage(message)),
    };

    let load_started = Instant::now();
    let data = Book::load(&config.data_dir).map_err(|error| {
        MainError::Fatal(format!(
            "讀取書目失敗（{}）：{error}\n提示：用 --data-dir 或 ZGC_DATA_DIR 指向含 book.json 與 volumes/ 的目錄。",
            config.data_dir.display()
        ))
    })?;
    let load_ms = millis(load_started.elapsed());

    let entity_started = Instant::now();
    let entities = Entities::load(&config.data_dir, &data).map_err(|error| {
        MainError::Fatal(format!(
            "讀取實體層失敗（{}）：{error}",
            config.data_dir.display()
        ))
    })?;
    let entity_ms = millis(entity_started.elapsed());

    let event_started = Instant::now();
    let events = Events::load(&config.data_dir).map_err(|error| {
        MainError::Fatal(format!(
            "讀取事件表失敗（{}）：{error}\n提示：events/tongjian.json 由 tools/build_events.py 生成；不需要年表時刪掉該文件即可，服務照常啟動。",
            config.data_dir.display()
        ))
    })?;
    let event_ms = millis(event_started.elapsed());

    let chapter_started = Instant::now();
    let chapter_years = ChapterYears::load(&config.data_dir, &events).map_err(|error| {
        MainError::Fatal(format!(
            "讀取系年表失敗（{}）：{error}\n提示：events/chapter_years.json 由 tools/build_chapter_years.py 生成；不需要時刪掉該文件即可。",
            config.data_dir.display()
        ))
    })?;
    let chapter_ms = millis(chapter_started.elapsed());

    let strategy_started = Instant::now();
    let strategy = Strategy::load(&config.data_dir, &data, &entities).map_err(|error| {
        MainError::Fatal(format!(
            "讀取謀略表失敗（{}）：{error}\n提示：strategy/chapters.json 由 tools/build_strategy.py 生成；不需要時刪掉該文件即可。",
            config.data_dir.display()
        ))
    })?;
    let strategy_ms = millis(strategy_started.elapsed());

    // The 繁 → 簡 table sits beside the book directory (data/shared/t2s.tsv).
    // A missing or unreadable table is not fatal: the reader keeps serving in
    // 繁體 and the toggle stays hidden.
    let variants_file = variants_path(&config.data_dir);
    let variants = match Variants::load(&variants_file) {
        Ok(variants) => variants,
        Err(error) => {
            eprintln!(
                "簡繁：讀取 {} 失敗（{error}）；本頁面只提供繁體。",
                variants_file.display()
            );
            Variants::default()
        }
    };

    // Read once: /ask never re-reads the environment, so a running server has
    // exactly one configuration.
    let ai_config = ai::AiConfig::from_env();

    let build_started = Instant::now();
    let state = Arc::new(
        app::App::with_events(data, entities, events)
            .with_chapter_years(chapter_years)
            .with_strategy(strategy)
            .with_variants(variants)
            .with_ai(ai_config.clone()),
    );
    let build_ms = millis(build_started.elapsed());

    let (bigrams, postings) = state.index_stats();
    let stats = state.data();
    println!("書目：{}（{}）", stats.book().title, stats.book().id);
    println!(
        "結構：{} 國 · {} 卷 · {} 章 · {} 段",
        stats.group_count(),
        stats.volume_count(),
        stats.chapter_count(),
        stats.paragraph_count()
    );
    println!("索引：{bigrams} 個 bigram / {postings} 次出現（啟動時建一次，之後每次請求只讀）");
    println!(
        "謀略：{} 章有輪次 · {} 條輪次 · {} 處手段 · {} 條未識別欄位",
        state.strategy().chapters_with_turns(),
        state.strategy().turn_total(),
        state.strategy().device_total(),
        state.strategy().unresolved_count()
    );
    println!(
        "實體：{} 個（人物 {} · 地名 {}）· {} 次出現 · 見於 {} 段 · {} 段有重疊",
        state.entities().len(),
        state.entities().of_kind(EntityKind::Person).count(),
        state.entities().of_kind(EntityKind::Place).count(),
        state.entities().occurrence_total(),
        state.entities().covered_paragraph_count(),
        state.entities().overlapping_paragraph_count()
    );
    match state.events().year_bounds() {
        Some((earliest, latest)) => println!(
            "事件：{} 年 · {} 條（前{}–前{}）· {} 年無條目 · 見 /timeline",
            state.events().year_count(),
            state.events().event_count(),
            earliest,
            latest,
            state.events().years_without_events().len()
        ),
        None => println!("事件：無（未找到 {EVENT_FILE}，/timeline 顯示空年表）"),
    }
    if state.chapter_years().is_empty() {
        println!("系年：無（未找到 {}, 章頁不顯示年份）", CHAPTER_YEAR_FILE);
    } else {
        println!(
            "系年：已判 {} 章 · 未判 {} 章 · 見章頁",
            state.chapter_years().resolved_count(),
            state.chapter_years().unresolved_count()
        );
    }
    {
        let variants = state.variants();
        if variants.is_empty() {
            println!("簡繁：未載入（繁體顯示）");
        } else {
            let (chars, phrases) = variants.rule_counts();
            println!(
                "簡繁：{chars} 字 / {phrases} 詞（{}）",
                variants_file.display()
            );
        }
    }
    // base and model only: the key is reported as set/unset and never printed.
    println!(
        "AI：{} · 端點 {} · 模型 {} · 金鑰 {} · 超時 {} s · 上限 {} tokens",
        if ai_config.ready() {
            "就緒"
        } else {
            "未配置（/ask 顯示配置說明）"
        },
        ai_config.base,
        ai_config.model.as_deref().unwrap_or("未設置"),
        ai_config.key_state(),
        ai_config.timeout.as_secs(),
        ai_config.max_tokens
    );
    println!("耗時：load {load_ms} ms · entity {entity_ms} ms · event {event_ms} ms · chapter {chapter_ms} ms · strategy {strategy_ms} ms · index {build_ms} ms");

    let listener = TcpListener::bind(("127.0.0.1", config.port))
        .map_err(|error| MainError::Fatal(bind_message(config.port, &error)))?;
    println!(
        "服務：http://127.0.0.1:{}/ （僅監聽本機，Ctrl-C 結束）",
        config.port
    );

    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let state = Arc::clone(&state);
                std::thread::spawn(move || serve(&state, stream));
            }
            Err(error) => eprintln!("接受連接失敗：{error}"),
        }
    }
    Ok(())
}

/// Path of the 繁 → 簡 table: a sibling of the book directory, at
/// <parent>/shared/t2s.tsv (data/shared/t2s.tsv for the default data/zhanguoce).
fn variants_path(data_dir: &Path) -> PathBuf {
    match data_dir.parent() {
        Some(parent) => parent.join("shared").join("t2s.tsv"),
        None => PathBuf::from("data/shared/t2s.tsv"),
    }
}

fn millis(duration: Duration) -> String {
    format!("{:.2}", duration.as_secs_f64() * 1000.0)
}

/// Clear diagnosis for the two failures that actually happen.
fn bind_message(port: u16, error: &std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::AddrInUse => format!(
            "端口 {port} 已被佔用（可能已有一個 zgc-app 在跑，或系統佔用了該端口）。\n換一個端口：zgc-app --port {} 或 ZGC_PORT={} zgc-app",
            port.saturating_add(1),
            port.saturating_add(1)
        ),
        std::io::ErrorKind::PermissionDenied => format!(
            "端口 {port} 需要更高權限（1024 以下為特權端口）。\n換一個高位端口：zgc-app --port {DEFAULT_PORT}"
        ),
        _ => format!("無法綁定 127.0.0.1:{port}：{error}"),
    }
}

/// One connection: read one request head, write one response, close.
fn serve(state: &app::App, mut stream: TcpStream) {
    let peer = stream
        .peer_addr()
        .map(|address| address.ip().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let started = Instant::now();

    match http::read_request(&mut stream) {
        Ok(request) => {
            let render_started = Instant::now();
            let response = state.handle(&request);
            let render_ms = render_started.elapsed().as_secs_f64() * 1000.0;
            let bytes = response.to_bytes();
            let length = bytes.len();
            if stream
                .write_all(&bytes)
                .and_then(|_| stream.flush())
                .is_ok()
            {
                println!(
                    "{peer} {} {} -> {} · {length} B · render {render_ms:.2} ms · total {:.2} ms",
                    request.method,
                    target_for_log(&request),
                    response.status,
                    started.elapsed().as_secs_f64() * 1000.0
                );
            }
        }
        Err(http::ReadError::Empty) => {}
        Err(error) => {
            let response = http::Response::text(
                error.status(),
                format!("{} {}\n", error.status(), error.message()),
            );
            let _ = stream.write_all(&response.to_bytes());
            println!(
                "{peer} 無法解析的請求 -> {}（{}）",
                error.status(),
                error.message()
            );
            let _ = stream.flush();
        }
    }
    let _ = stream.shutdown(Shutdown::Both);
}

/// Request target for the log line, bounded so a hostile URL cannot flood it.
fn target_for_log(request: &http::Request) -> String {
    let mut target = request.path.clone();
    if !request.query.is_empty() {
        target.push('?');
        target.push_str(&request.query);
    }
    let mut bounded: String = target.chars().take(120).collect();
    if bounded.len() < target.len() {
        bounded.push('…');
    }
    bounded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str], env: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let env: Vec<(String, String)> = env
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        Config::parse_with(
            args.iter().map(|argument| argument.to_string()),
            move |name| {
                env.iter()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| value.clone())
            },
        )
    }

    #[test]
    fn defaults_are_local_port_and_repo_data_dir() {
        let config = parse(&[], &[]).unwrap();
        assert_eq!(config.port, DEFAULT_PORT);
        assert_ne!(config.port, 80);
        assert_ne!(config.port, 443);
        assert_eq!(config.data_dir, PathBuf::from(DEFAULT_DATA_DIR));
    }

    #[test]
    fn command_line_beats_environment() {
        let config = parse(&["--port", "9123"], &[("ZGC_PORT", "7000")]).unwrap();
        assert_eq!(config.port, 9123);
        let config = parse(&["--port=9124"], &[]).unwrap();
        assert_eq!(config.port, 9124);
        let config = parse(&[], &[("ZGC_PORT", "7001"), ("ZGC_DATA_DIR", "/tmp/book")]).unwrap();
        assert_eq!(config.port, 7001);
        assert_eq!(config.data_dir, PathBuf::from("/tmp/book"));
    }

    #[test]
    fn bad_port_is_a_readable_error_not_a_panic() {
        assert!(matches!(
            parse(&["--port", "abc"], &[]),
            Err(ConfigError::Invalid(_))
        ));
        assert!(matches!(
            parse(&["--port", "0"], &[]),
            Err(ConfigError::Invalid(_))
        ));
        assert!(matches!(
            parse(&["--port", "70000"], &[]),
            Err(ConfigError::Invalid(_))
        ));
        assert!(matches!(
            parse(&["--port"], &[]),
            Err(ConfigError::Invalid(_))
        ));
        assert!(matches!(
            parse(&["--nope"], &[]),
            Err(ConfigError::Invalid(_))
        ));
    }

    #[test]
    fn help_is_recognised() {
        assert!(matches!(parse(&["--help"], &[]), Err(ConfigError::Help)));
        assert!(matches!(parse(&["-h"], &[]), Err(ConfigError::Help)));
    }

    #[test]
    fn variants_table_sits_beside_the_book_directory() {
        assert_eq!(
            variants_path(Path::new("data/zhanguoce")),
            PathBuf::from("data/shared/t2s.tsv")
        );
        assert_eq!(
            variants_path(Path::new("/srv/books/zgc")),
            PathBuf::from("/srv/books/shared/t2s.tsv")
        );
    }

    #[test]
    fn port_in_use_message_names_the_port_and_a_way_out() {
        let error = std::io::Error::from(std::io::ErrorKind::AddrInUse);
        let message = bind_message(8099, &error);
        assert!(message.contains("8099"));
        assert!(message.contains("--port"));
        assert!(!message.contains("panic"));
    }
}
