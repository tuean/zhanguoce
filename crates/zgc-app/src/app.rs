//! Application state and routing.
//!
//! The book, the bigram index and the id lookups are built once at startup and
//! then shared read-only by every request thread — rendering never rebuilds an
//! index.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use zgc_core::{
    AskContext, BookData, ChapterYears, Entities, EntityContext, EntityKind, Events, EvidenceText,
    GlossaryText, Hit, NoteText, Position, Scope, SearchIndex, Strategy, Variants, YearContext,
};

use crate::ai::{self, AiClients, AiConfig, AiError, ExternalSource, HttpClients};
use crate::http::{Request, Response};
use crate::pages;
use crate::prefs::{Prefs, COOKIE_LAST, COOKIE_MAX_AGE};
use crate::urlcodec::{self, Cookies};

/// Longest accepted query string for search (characters, not bytes).
pub const MAX_QUERY_CHARS: usize = 64;
/// Hits rendered per result page; the total count is always reported.
pub const MAX_HITS: usize = 200;

/// Everything a request needs, built once.
pub struct App {
    data: BookData,
    index: SearchIndex,
    entities: Entities,
    events: Events,
    chapter_years: ChapterYears,
    strategy: Strategy,
    /// 繁 → 簡 display table; empty when it could not be loaded.
    variants: Variants,
    positions: HashMap<String, Position>,
    chapter_starts: HashMap<String, Position>,
    /// AI endpoint configuration; not-ready by default, so tests never call out.
    ai: AiConfig,
    /// Chat, search and fetch transports. HTTP in production; a fake in tests.
    clients: Box<dyn AiClients>,
}

impl App {
    /// Build the search index and the id → position lookups for one loaded book.
    ///
    /// No entity layer: the reader renders plain text. Tests and callers that do
    /// not need entities keep using this constructor.
    #[cfg(test)]
    pub fn new(data: BookData) -> App {
        App::with_entities(data, Entities::empty())
    }

    /// Same, with the entity index mounted and no timeline. Tests only: the
    /// server always goes through `with_events`, so this never ships unused.
    #[cfg(test)]
    pub fn with_entities(data: BookData, entities: Entities) -> App {
        App::with_events(data, entities, Events::empty())
    }

    /// Same, with the event timeline mounted too. Books without an event table
    /// pass `Events::empty()` and `/timeline` degrades to an explanatory page.
    pub fn with_events(data: BookData, entities: Entities, events: Events) -> App {
        let index = SearchIndex::build(&data);
        let mut positions = HashMap::with_capacity(data.paragraph_count());
        let mut chapter_starts = HashMap::new();
        for position in data.iter_positions() {
            let Some(paragraph) = data.paragraph(position) else {
                continue;
            };
            positions.entry(paragraph.id.clone()).or_insert(position);
            if let Some(chapter) = data.chapter(position.volume, position.chapter) {
                chapter_starts.entry(chapter.id.clone()).or_insert(position);
            }
        }
        App {
            data,
            index,
            entities,
            events,
            chapter_years: ChapterYears::empty(),
            strategy: Strategy::empty(),
            variants: Variants::default(),
            positions,
            chapter_starts,
            ai: AiConfig::from_lookup(|_| None),
            clients: Box::new(HttpClients),
        }
    }

    /// Mount the chapter-year index. Chainable so the two existing
    /// constructors keep their signatures (and their tests) unchanged.
    pub fn with_chapter_years(mut self, chapter_years: ChapterYears) -> App {
        self.chapter_years = chapter_years;
        self
    }

    /// Mount the strategy index (turns and devices). Chainable like the year
    /// index, and empty by default so the earlier constructors keep working.
    pub fn with_strategy(mut self, strategy: Strategy) -> App {
        self.strategy = strategy;
        self
    }

    /// Mount the 繁 → 簡 table. Chainable, and empty by default: without a
    /// table the reader shows 繁體 and the toggle is hidden.
    pub fn with_variants(mut self, variants: Variants) -> App {
        self.variants = variants;
        self
    }

    /// Mount the AI endpoint configuration. Chainable, and absent by default so
    /// every existing constructor (and test) keeps working without a network.
    pub fn with_ai(mut self, ai: AiConfig) -> App {
        self.ai = ai;
        self
    }

    /// Inject the outbound transports. Tests only: shipping code always uses
    /// [`HttpClients`], and no test may reach the network.
    #[cfg(test)]
    pub fn with_ai_clients(mut self, clients: Box<dyn AiClients>) -> App {
        self.clients = clients;
        self
    }

    pub fn data(&self) -> &BookData {
        &self.data
    }

    /// The entity index built at startup (empty when the book has no entities).
    pub fn entities(&self) -> &Entities {
        &self.entities
    }

    /// The event timeline built at startup (empty when the book has none).
    pub fn events(&self) -> &Events {
        &self.events
    }

    /// The chapter-year index (empty when the book has no 系年 table).
    pub fn chapter_years(&self) -> &ChapterYears {
        &self.chapter_years
    }

    /// The strategy index (empty when the book has no strategy table).
    pub fn strategy(&self) -> &Strategy {
        &self.strategy
    }

    /// The 繁 → 簡 table loaded at startup (empty when the file was missing).
    pub fn variants(&self) -> &Variants {
        &self.variants
    }

    /// The AI configuration resolved at startup (key redacted in Debug).
    pub fn ai(&self) -> &AiConfig {
        &self.ai
    }

    fn ai_clients(&self) -> &dyn AiClients {
        self.clients.as_ref()
    }

    /// Bigram index size, for the startup log.
    pub fn index_stats(&self) -> (usize, usize) {
        (self.index.bigram_count(), self.index.posting_count())
    }

    /// Stable anchor (paragraph id) → structural handle.
    pub fn position_of(&self, paragraph_id: &str) -> Option<Position> {
        self.positions.get(paragraph_id).copied()
    }

    /// Structural handle → stable anchor.
    pub fn paragraph_id_at(&self, position: Position) -> Option<&str> {
        self.data.paragraph(position).map(|p| p.id.as_str())
    }

    /// First paragraph of a chapter, or None for an empty chapter.
    pub fn chapter_start(&self, chapter_id: &str) -> Option<Position> {
        self.chapter_starts.get(chapter_id).copied()
    }

    /// Chapter id → its first position. Unlike `chapter_start`, an empty
    /// chapter still resolves (to paragraph 0), because /story renders the
    /// chapter shell even when it carries no paragraphs.
    pub fn chapter_position(&self, chapter_id: &str) -> Option<Position> {
        if let Some(position) = self.chapter_starts.get(chapter_id) {
            return Some(*position);
        }
        for (volume_index, volume) in self.data.volumes().iter().enumerate() {
            for (chapter_index, chapter) in volume.chapters.iter().enumerate() {
                if chapter.id == chapter_id {
                    return Some(Position::new(volume_index, chapter_index, 0));
                }
            }
        }
        None
    }

    /// Search with the index built at startup.
    pub fn search(&self, query: &str, scope: &Scope) -> Vec<Hit> {
        self.index.search(&self.data, query, scope)
    }

    /// The whole router: one request in, one response out.
    pub fn handle(&self, request: &Request) -> Response {
        let cookies = urlcodec::parse_cookies(request.header("cookie").unwrap_or_default());
        if request.method != "GET" {
            return Response::html(
                405,
                pages::bad_request_page(self, "本服務只支持 GET 請求。"),
            )
            .with_header("Allow", "GET");
        }
        match request.path.as_str() {
            "/" => {
                let query = urlcodec::parse_query(&request.query);
                let (prefs, _) = Prefs::resolve(&query, &cookies);
                Response::html(200, pages::toc_page(self, &prefs, &cookies))
            }
            "/read" => self.handle_read(&request.query, &cookies),
            "/story" => self.handle_story(&request.query, &cookies),
            "/ask" => self.handle_ask(&request.query, &cookies),
            "/search" => self.handle_search(&request.query, &cookies),
            "/entity" => self.handle_entity(&request.query, &cookies),
            "/entities" => self.handle_entities(&request.query, &cookies),
            "/timeline" => self.handle_timeline(&request.query, &cookies),
            "/favicon.ico" => Response::empty(204),
            other => Response::html(404, pages::not_found_page(self, "頁面不存在", other)),
        }
    }

    fn handle_read(&self, raw_query: &str, cookies: &Cookies) -> Response {
        let query = urlcodec::parse_query(raw_query);
        let (prefs, persist) = Prefs::resolve(&query, cookies);
        let Some(raw_paragraph_id) = urlcodec::get(&query, "p") else {
            return Response::html(
                400,
                pages::bad_request_page(self, "缺少參數 p：請從目錄或檢索結果進入閱讀頁。"),
            );
        };
        let paragraph_id = raw_paragraph_id.trim();
        let Some(position) = self.position_of(paragraph_id) else {
            return Response::html(
                404,
                pages::not_found_page(self, "找不到這一段", paragraph_id),
            );
        };
        let highlight = parse_span(urlcodec::get(&query, "hl"));
        let mut response = Response::html(200, pages::read_page(self, position, &prefs, highlight));
        response = response.with_header(
            "Set-Cookie",
            format!(
                "{COOKIE_LAST}={}; Path=/; Max-Age={COOKIE_MAX_AGE}; SameSite=Lax; HttpOnly",
                urlcodec::percent_encode(paragraph_id)
            ),
        );
        for (name, value) in persist {
            response = response.with_header(
                "Set-Cookie",
                format!("{name}={value}; Path=/; Max-Age={COOKIE_MAX_AGE}; SameSite=Lax"),
            );
        }
        response
    }

    fn handle_story(&self, raw_query: &str, cookies: &Cookies) -> Response {
        let query = urlcodec::parse_query(raw_query);
        let (prefs, persist) = Prefs::resolve(&query, cookies);
        let chapter_raw = urlcodec::get(&query, "c").unwrap_or_default().trim();
        let paragraph_raw = urlcodec::get(&query, "p").unwrap_or_default().trim();

        let position = if !chapter_raw.is_empty() {
            let Some(position) = self.chapter_position(chapter_raw) else {
                return Response::html(
                    404,
                    pages::not_found_page(self, "找不到這一章", chapter_raw),
                );
            };
            position
        } else if !paragraph_raw.is_empty() {
            let Some(position) = self.position_of(paragraph_raw) else {
                return Response::html(
                    404,
                    pages::not_found_page(self, "找不到這一段", paragraph_raw),
                );
            };
            position
        } else {
            return Response::html(
                400,
                pages::bad_request_page(self, "缺少參數 c 或 p：請從目錄或閱讀頁進入故事視界。"),
            );
        };

        let Some(chapter) = self.data().chapter(position.volume, position.chapter) else {
            return Response::html(
                400,
                pages::bad_request_page(self, "內部定位失效：該章不存在。"),
            );
        };
        let ai_requested = urlcodec::get(&query, "ai")
            .map(|value| value.trim() == "1")
            .unwrap_or(false);

        let mut sources: Vec<ExternalSource> = Vec::new();
        let mut evidence_note: Option<String> = None;
        let mut outcome: Option<pages::StoryOutcome> = None;
        if ai_requested {
            let config = self.ai();
            if !config.ready() {
                outcome = Some(pages::StoryOutcome::NotConfigured);
            } else {
                // One deadline for search + fetch + chat: a slow provider
                // degrades to a partial reading instead of holding the page.
                let deadline = Instant::now() + Duration::from_secs(45);
                let search_query = pages::story_search_query(self, position);
                match self.ai_clients().search(config, &search_query, 5) {
                    Ok(hits) if !hits.is_empty() => {
                        for hit in hits.into_iter().take(2) {
                            let content = if fetchable_url(&hit.url) && Instant::now() < deadline {
                                let remaining = deadline.saturating_duration_since(Instant::now());
                                let mut fetch_config = config.clone();
                                fetch_config.timeout = fetch_config.timeout.min(remaining);
                                self.ai_clients()
                                    .fetch(&fetch_config, &hit.url)
                                    .ok()
                                    .map(|page| page.content)
                            } else {
                                None
                            };
                            sources.push(ExternalSource {
                                title: hit.title,
                                site_name: hit.site_name,
                                url: hit.url,
                                summary: hit.summary,
                                content,
                            });
                        }
                    }
                    _ => {
                        evidence_note = Some("未能取得外部資料，本解讀僅據原文與注疏".to_string());
                    }
                }

                let prompt = pages::story_prompt(self, position, &sources);
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining < Duration::from_secs(1) {
                    outcome = Some(pages::StoryOutcome::Failure(AiError::Timeout));
                } else {
                    let mut chat_config = config.clone();
                    chat_config.timeout = chat_config.timeout.min(remaining);
                    let reply =
                        self.ai_clients()
                            .chat(&chat_config, pages::STORY_SYSTEM_PROMPT, &prompt);
                    outcome = Some(match reply {
                        Ok(reply) => pages::StoryOutcome::Answer(reply),
                        Err(error) => pages::StoryOutcome::Failure(error),
                    });
                }
            }
        }

        // 背景史料 is opt-in: without bg=1 the page only carries the link, so a
        // plain chapter view never calls the provider.
        let bg_requested = urlcodec::get(&query, "bg")
            .map(|value| value.trim() == "1")
            .unwrap_or(false);
        let background = if bg_requested {
            let config = self.ai();
            if !config.ready() {
                pages::BackgroundView::NotConfigured
            } else {
                match self
                    .ai_clients()
                    .search_background(config, &chapter.title, 8)
                {
                    Ok(hits) => pages::BackgroundView::Hits(
                        hits.into_iter()
                            .take(ai::BACKGROUND_LIMIT)
                            .map(|hit| ExternalSource {
                                title: hit.title,
                                site_name: hit.site_name,
                                url: hit.url,
                                summary: hit.summary,
                                content: None,
                            })
                            .collect(),
                    ),
                    Err(error) => pages::BackgroundView::Failure(error),
                }
            }
        } else {
            pages::BackgroundView::NotRequested
        };

        let view = pages::StoryView {
            background,
            requested: ai_requested,
            config: self.ai(),
            outcome,
            sources: &sources,
            evidence_note: evidence_note.as_deref(),
        };
        let mut response = Response::html(200, pages::story_page(self, position, &prefs, &view));
        if let Some(paragraph_id) = chapter
            .paragraphs
            .first()
            .map(|paragraph| paragraph.id.as_str())
        {
            response = response.with_header(
                "Set-Cookie",
                format!(
                    "{COOKIE_LAST}={}; Path=/; Max-Age={COOKIE_MAX_AGE}; SameSite=Lax; HttpOnly",
                    urlcodec::percent_encode(paragraph_id)
                ),
            );
        }
        for (name, value) in persist {
            response = response.with_header(
                "Set-Cookie",
                format!("{name}={value}; Path=/; Max-Age={COOKIE_MAX_AGE}; SameSite=Lax"),
            );
        }
        response
    }

    fn handle_entity(&self, raw_query: &str, cookies: &Cookies) -> Response {
        let query = urlcodec::parse_query(raw_query);
        let (prefs, _) = Prefs::resolve(&query, cookies);
        let Some(raw_id) = urlcodec::get(&query, "id") else {
            return Response::html(
                400,
                pages::bad_request_page(self, "缺少參數 id：請從閱讀頁或實體索引進入。"),
            );
        };
        let id = raw_id.trim();
        let Some(entity) = self.entities().entity(id) else {
            return Response::html(404, pages::not_found_page(self, "找不到這個實體", id));
        };
        Response::html(200, pages::entity_page(self, &prefs, entity))
    }

    fn handle_entities(&self, raw_query: &str, cookies: &Cookies) -> Response {
        let query = urlcodec::parse_query(raw_query);
        let (prefs, _) = Prefs::resolve(&query, cookies);
        let kind_raw = urlcodec::get(&query, "kind").unwrap_or_default().trim();
        let kind = EntityKind::parse(kind_raw);
        let sort = match urlcodec::get(&query, "sort").unwrap_or_default().trim() {
            "name" => pages::EntitySort::Name,
            _ => pages::EntitySort::Count,
        };
        let notice = if kind.is_none() && !kind_raw.is_empty() {
            Some(format!("未知實體類型「{kind_raw}」，已列出全部。"))
        } else {
            None
        };
        Response::html(
            200,
            pages::entities_page(self, &prefs, kind, sort, notice.as_deref()),
        )
    }

    fn handle_search(&self, raw_query: &str, cookies: &Cookies) -> Response {
        let query = urlcodec::parse_query(raw_query);
        let (prefs, _) = Prefs::resolve(&query, cookies);
        let raw_text = urlcodec::get(&query, "q").unwrap_or_default().trim();
        let group_raw = urlcodec::get(&query, "group").unwrap_or_default().trim();

        let mut notice = None;
        if raw_text.chars().count() > MAX_QUERY_CHARS {
            notice = Some(format!("檢索詞過長，只取前 {MAX_QUERY_CHARS} 個字。"));
        }
        let search_text: String = raw_text.chars().take(MAX_QUERY_CHARS).collect();

        let scope = if group_raw.is_empty() {
            Scope::All
        } else if self.data().group(group_raw).is_some() {
            Scope::Group(group_raw.to_string())
        } else {
            let extra = format!("未知國策「{group_raw}」，已改為檢索全書。");
            notice = Some(match notice {
                Some(existing) => format!("{existing}{extra}"),
                None => extra,
            });
            Scope::All
        };

        let hits = if search_text.is_empty() {
            Vec::new()
        } else {
            self.search(&search_text, &scope)
        };
        let total = hits.len();
        let shown: Vec<Hit> = hits.into_iter().take(MAX_HITS).collect();
        Response::html(
            200,
            pages::search_page(
                self,
                &prefs,
                &search_text,
                group_raw,
                &shown,
                total,
                notice.as_deref(),
            ),
        )
    }

    /// /timeline: the 戰國 segment as one dated list.
    ///
    /// Shape of the query string:
    /// - `?y=403` — one year only (前403 too, both spellings);
    /// - `?e=tj-y0403-p4` — the year of that event, with the event marked, so an
    ///   anchor link `/timeline#tj-y0403-p4` and a shareable `?e=` land alike;
    /// - `?q=衛鞅` — substring filter over text, note and collation.
    ///
    /// Nothing here 404s: a year outside the table is reported in-page, because
    /// a timeline is a place to look, not a document that can be "missing".
    fn handle_timeline(&self, raw_query: &str, cookies: &Cookies) -> Response {
        let query = urlcodec::parse_query(raw_query);
        let (prefs, _) = Prefs::resolve(&query, cookies);
        let year_raw = urlcodec::get(&query, "y").unwrap_or_default().trim();
        let text = urlcodec::get(&query, "q").unwrap_or_default().trim();
        let event_raw = urlcodec::get(&query, "e").unwrap_or_default().trim();

        let mut notice: Option<String> = None;
        let mut focus: Option<&str> = None;
        let mut year: Option<i32> = None;

        if !event_raw.is_empty() {
            match self.events().event(event_raw) {
                Some(event) => {
                    year = Some(event.year_bc);
                    focus = Some(event.id.as_str());
                }
                None => notice = Some(format!("找不到事件「{event_raw}」，已顯示全部年份。")),
            }
        }

        if year.is_none() && !year_raw.is_empty() {
            match parse_bce_year(year_raw) {
                Some(parsed) if self.events().year(parsed).is_some() => year = Some(parsed),
                Some(parsed) => {
                    notice = Some(format!("前{parsed}年沒有條目，已顯示全部年份。"));
                }
                None => notice = Some(format!("年份「{year_raw}」無法解析，已顯示全部年份。")),
            }
        }

        Response::html(
            200,
            pages::timeline_page(self, &prefs, year, text, focus, notice.as_deref()),
        )
    }
}

/// Accept `403`, `前403` and the full-width `－` spellings the page emits.
fn parse_bce_year(raw: &str) -> Option<i32> {
    let digits = raw.trim().trim_start_matches(['前', '－', '-', '+']).trim();
    let value = digits.parse::<i32>().ok()?;
    if value <= 0 || value > 9999 {
        return None;
    }
    Some(value)
}

/// Parse an hl=offset,len pair (characters). Anything malformed — missing
/// comma, non-numeric, zero length, overflowing — is None, and the reader
/// simply highlights nothing: a bad hl must never fail the page.
fn parse_span(raw: Option<&str>) -> Option<(usize, usize)> {
    let (offset, len) = raw?.trim().split_once(',')?;
    let offset = offset.trim().parse::<usize>().ok()?;
    let len = len.trim().parse::<usize>().ok()?;
    if len == 0 || offset.checked_add(len).is_none() {
        return None;
    }
    Some((offset, len))
}

/// Only http(s) pages are fetched and only those may become links; a
/// javascript: or data: hit stays plain text.
fn fetchable_url(url: &str) -> bool {
    let url = url.trim();
    (url.starts_with("http://") || url.starts_with("https://"))
        && !url.chars().any(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zgc_core::{
        Annotation, Book, Chapter, Entities, EntityFile, EntityKind, EntityRecord, Glossary, Group,
        OccurrenceRecord, Paragraph, Volume,
    };

    fn paragraph(id: &str, text: &str, translation: Option<&str>) -> Paragraph {
        Paragraph {
            id: id.into(),
            text: text.into(),
            translation: translation.map(str::to_owned),
            annotations: Vec::new(),
            glossary: Vec::new(),
        }
    }

    fn fixture() -> App {
        App::new(toy_data())
    }

    fn toy_data() -> BookData {
        let mut annotated = paragraph(
            "v-1-01-p0001",
            "衞鞅亡魏入秦，孝公以爲相。",
            Some("卫鞅从魏国逃到秦国，秦孝公任命他为相。"),
        );
        annotated.annotations.push(Annotation {
            source: Some("鮑彪".into()),
            text: "孝公，秦君也。".into(),
            edition: None,
        });
        // 第二見證：四庫全書本補注（M1d）。展示用玩具資料，故與底本注同段。
        annotated.annotations.push(Annotation {
            source: Some("吴师道正".into()),
            text: "四庫本補正：此章底本缺葉。".into(),
            edition: Some("四庫全書".into()),
        });
        annotated.glossary.push(Glossary {
            term: "衞鞅".into(),
            pinyin: Some("wei yang".into()),
            gloss: Some("即商鞅。".into()),
        });

        let volume_one = Volume {
            schema_version: 1,
            id: "v-1".into(),
            book_id: "toy".into(),
            group_id: "a".into(),
            title: "甲一".into(),
            title_simp: Some("甲一".into()),
            index: 1,
            chapters: vec![
                Chapter {
                    id: "v-1-01".into(),
                    title: "衞鞅亡魏入秦".into(),
                    paragraphs: vec![
                        annotated,
                        paragraph("v-1-01-p0002", "秦伐魏，取河西之地。", None),
                    ],
                },
                Chapter {
                    id: "v-1-02".into(),
                    title: "蘇秦說齊王".into(),
                    paragraphs: vec![paragraph(
                        "v-1-02-p0001",
                        "蘇秦說齊王曰",
                        Some("苏秦游说齐王说"),
                    )],
                },
            ],
        };
        let volume_two = Volume {
            schema_version: 1,
            id: "v-2".into(),
            book_id: "toy".into(),
            group_id: "b".into(),
            title: "乙一".into(),
            title_simp: None,
            index: 2,
            chapters: vec![Chapter {
                id: "v-2-01".into(),
                title: "秦惠王立".into(),
                paragraphs: vec![paragraph(
                    "v-2-01-p0001",
                    "秦惠王立，張儀爲相。",
                    Some("秦惠王即位，张仪为相。"),
                )],
            }],
        };
        let book = Book {
            schema_version: 1,
            id: "toy".into(),
            title: "玩具策".into(),
            title_simp: Some("玩具策".into()),
            compiler: None,
            era: None,
            form: "國別體".into(),
            sources: Vec::new(),
            groups: vec![
                Group {
                    id: "a".into(),
                    title: "甲".into(),
                    title_simp: None,
                    volumes: vec!["v-1".into()],
                },
                Group {
                    id: "b".into(),
                    title: "乙".into(),
                    title_simp: None,
                    volumes: vec!["v-2".into()],
                },
            ],
            volume_order: vec!["v-1".into(), "v-2".into()],
        };
        BookData::from_parts(book, vec![volume_one, volume_two]).expect("fixture is valid")
    }

    fn body(response: &Response) -> String {
        response.body.clone()
    }

    fn header<'a>(response: &'a Response, name: &str) -> Vec<&'a str> {
        response
            .headers
            .iter()
            .filter(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
            .collect()
    }

    #[test]
    fn table_of_contents_lists_group_volume_and_chapter_in_reading_order() {
        let app = fixture();
        let response = app.handle(&Request::get("/"));
        assert_eq!(response.status, 200);
        let html = body(&response);
        let tree = &html[html.find("<nav class=\"tree\" data-arrow-step").expect("tree")..];
        let group_a = tree.find("class=\"g-title\">甲<").expect("group a");
        let volume_one = tree.find("甲一").expect("volume one");
        let chapter_one = tree.find("衞鞅亡魏入秦").expect("chapter one");
        let group_b = tree.find("class=\"g-title\">乙<").expect("group b");
        let volume_two = tree.find("乙一").expect("volume two");
        assert!(group_a < volume_one, "group before its volumes");
        assert!(volume_one < chapter_one, "volume before its chapters");
        assert!(
            chapter_one < group_b && group_b < volume_two,
            "volumeOrder decides group order"
        );
        assert!(html.contains("href=\"/read?p=v-1-01-p0001\""));
        assert!(html.contains("2 國 · 2 卷 · 3 章 · 4 段"), "{html}");
    }

    #[test]
    fn reading_page_shows_all_three_columns() {
        let app = fixture();
        let response = app.handle(&Request::get("/read?p=v-1-01-p0001"));
        assert_eq!(response.status, 200);
        let html = body(&response);
        assert!(html.contains("衞鞅亡魏入秦，孝公以爲相。"), "原文");
        assert!(html.contains("卫鞅从魏国逃到秦国"), "白话");
        assert!(html.contains("孝公，秦君也。"), "注疏");
        assert!(html.contains("鮑彪"), "注疏来源");
        assert!(html.contains("即商鞅。"), "训释");
        assert!(!html.contains("暫無注疏"));
        assert!(html.contains("第 1 段"), "position");
        assert!(html.contains("共 2 段"));
        assert!(html.contains("甲一") && html.contains("甲"));
        assert!(
            html.contains("layout-tri"),
            "default layout is three columns"
        );
    }

    #[test]
    fn notes_show_the_witness_edition_only_for_the_second_witness() {
        // The table has to name every glyph this page converts; without one the
        // reader degrades to 繁體 and the 簡體 assertions could never hold.
        let variants = Variants::parse("衞\t卫\n爲\t为\n庫\t库\n書\t书\n補\t补\n叢\t丛\n");
        let app = entity_fixture().with_variants(variants);

        let html = body(&app.handle(&Request::get("/read?p=v-1-01-p0001&sc=trad")));
        assert!(
            html.contains("<span class=\"ed\">四庫全書</span>"),
            "{html}"
        );
        assert!(
            !html.contains("四部叢刊"),
            "base witness stays unmarked: {html}"
        );
        assert!(html.contains("四庫本補正"), "{html}");

        let simp = body(&app.handle(&Request::get("/read?p=v-1-01-p0001&sc=simp")));
        assert!(
            simp.contains("<span class=\"ed\">四库全书</span>"),
            "{simp}"
        );
        assert!(
            simp.contains("四库本补正"),
            "note text follows the script: {simp}"
        );
        assert!(
            !simp.contains("四庫全書"),
            "no 繁體 glyph left in 簡體 mode: {simp}"
        );
    }

    #[test]
    fn reading_page_marks_missing_translation_and_annotation() {
        let app = fixture();
        let html = body(&app.handle(&Request::get("/read?p=v-1-01-p0002")));
        assert!(html.contains("暫無白話"), "{html}");
        assert!(html.contains("暫無注疏"), "{html}");
        assert!(html.contains("第 2 段"));
    }

    #[test]
    fn reading_page_carries_the_note_shells_but_never_note_text() {
        let app = fixture();
        let html = body(&app.handle(&Request::get("/read?p=v-1-01-p0001")));
        assert!(
            html.contains("<button type=\"button\" class=\"note-toggle\" data-note-toggle=\"v-1-01-p0001\" aria-expanded=\"false\" hidden>筆記</button>"),
            "note toggle for this paragraph: {html}"
        );
        assert!(html.contains("data-note-input=\"v-1-01-p0001\""), "{html}");
        assert!(html.contains("data-note-save=\"v-1-01-p0001\""), "{html}");
        assert!(html.contains("data-note-delete=\"v-1-01-p0001\""), "{html}");
        assert!(html.contains("data-note-dot=\"v-1-01-p0001\""), "{html}");
        assert!(
            html.contains("data-note-scope=\"v-1-01-p0001\""),
            "the quote is scoped to this paragraph's 原文: {html}"
        );
        // The view container starts hidden and empty: the note body is browser
        // data, so no note text can reach the server's HTML at all.
        assert!(
            html.contains("<div class=\"note-view\" data-note-view=\"v-1-01-p0001\" hidden></div>"),
            "{html}"
        );
        assert!(
            html.contains("data-note-export>匯出筆記</button>"),
            "{html}"
        );
        assert!(
            html.contains("data-note-import-btn>匯入筆記</button>"),
            "{html}"
        );
        assert!(
            html.contains("<input type=\"file\" data-note-import accept="),
            "{html}"
        );
        assert!(html.contains("zgc_notes_v1"), "the notes script is inline");
        // A field missing from the script's FIELDS table is a button that never
        // binds: the toggle was once left out and clicking 筆記 did nothing.
        assert!(
            html.contains("[\"data-note-toggle\", \"toggle\"]"),
            "the toggle must be registered in FIELDS: {html}"
        );
        // The banner templates are format! arguments, not format! strings, so a
        // doubled brace here would reach fill() literally and print {added}.
        assert!(
            html.contains("data-imported=\"已匯入 {added} 條筆記，現共 {total} 條。\""),
            "the import banner must keep single braces: {html}"
        );
        assert!(html.contains("new Blob("), "export builds a Blob");
        assert!(
            html.contains("new FileReader()"),
            "import reads a FileReader"
        );
        assert!(
            !html.contains("innerHTML"),
            "user text never goes through innerHTML"
        );
        // Author display rules outrank the UA [hidden] rule, so the stylesheet
        // has to restate it or the dots show without JavaScript.
        assert!(
            html.contains(".note-dot[hidden]") && html.contains(".seg-notes[hidden]"),
            "the hidden state must beat the display rules: {html}"
        );
    }

    #[test]
    fn story_page_carries_one_note_shell_per_paragraph() {
        let app = fixture();
        let response = app.handle(&Request::get("/story?c=v-1-01"));
        assert_eq!(response.status, 200);
        let html = body(&response);
        assert_eq!(html.matches("data-note-toggle=\"v-1-01-p0001\"").count(), 1);
        assert_eq!(html.matches("data-note-toggle=\"v-1-01-p0002\"").count(), 1);
        assert_eq!(html.matches("data-note-input=\"v-1-01-p0002\"").count(), 1);
        assert!(
            html.contains("<div class=\"note-view\" data-note-view=\"v-1-01-p0002\" hidden></div>"),
            "{html}"
        );
        assert!(html.contains("zgc_notes_v1"));
        // 匯出 / 匯入 are /read toolbar entries, not part of the story column.
        assert!(!html.contains("data-note-export>"), "{html}");
        assert!(!html.contains("data-note-import-btn>"), "{html}");
    }

    #[test]
    fn note_labels_follow_the_script_mode() {
        let variants = Variants::parse("筆\t笔\n記\t记\n匯\t汇\n刪\t删\n");
        let app = entity_fixture().with_variants(variants);
        let simp = body(&app.handle(&Request::get("/read?p=v-1-01-p0001&sc=simp")));
        assert!(
            simp.contains(">笔记</button>"),
            "toggle label converted: {simp}"
        );
        assert!(simp.contains("汇出笔记"), "export label converted: {simp}");
        assert!(simp.contains("data-saved=\"已保存。\""), "{simp}");
    }

    #[test]
    fn unknown_paragraph_id_is_404_and_escaped() {
        let app = fixture();
        let response = app.handle(&Request::get("/read?p=%3Cscript%3Eghost%3C/script%3E"));
        assert_eq!(response.status, 404);
        let html = body(&response);
        assert!(html.contains("&lt;script&gt;ghost"), "escaped echo: {html}");
        assert!(!html.contains("<script>"));
    }

    #[test]
    fn missing_paragraph_parameter_is_400() {
        let app = fixture();
        assert_eq!(app.handle(&Request::get("/read")).status, 400);
        assert_eq!(app.handle(&Request::get("/read?p=")).status, 404);
    }

    #[test]
    fn layout_and_font_controls_switch_and_persist() {
        let app = fixture();
        let response = app.handle(&Request::get(
            "/read?p=v-1-01-p0001&layout=single&orig=xl&tran=s&lh=tight",
        ));
        assert_eq!(response.status, 200);
        let html = body(&response);
        assert!(html.contains("layout-single"));
        assert!(html.contains("--orig-size:1.66rem"));
        assert!(html.contains("--tran-size:0.92rem"));
        assert!(html.contains("--leading:1.9"));
        let cookies = header(&response, "Set-Cookie");
        assert!(
            cookies
                .iter()
                .any(|value| value.starts_with("zgc_layout=single;")),
            "{cookies:?}"
        );
        assert!(cookies
            .iter()
            .any(|value| value.starts_with("zgc_orig=xl;")));
        assert!(cookies.iter().any(|value| value.starts_with("zgc_tran=s;")));
        assert!(cookies
            .iter()
            .any(|value| value.starts_with("zgc_leading=tight;")));
        assert!(cookies
            .iter()
            .any(|value| value.starts_with("zgc_last=v-1-01-p0001;")));
    }

    #[test]
    fn layout_can_come_from_the_cookie() {
        let app = fixture();
        let request = Request::get("/read?p=v-1-01-p0001").with_header("Cookie", "zgc_layout=dual");
        let response = app.handle(&request);
        assert!(body(&response).contains("layout-dual"));
        assert!(
            header(&response, "Set-Cookie").is_empty()
                || !header(&response, "Set-Cookie")
                    .iter()
                    .any(|v| v.starts_with("zgc_layout")),
            "a cookie-sourced value is not re-set"
        );
    }

    #[test]
    fn navigation_uses_the_core_nav_api_across_chapters_and_volumes() {
        let app = fixture();
        let html = body(&app.handle(&Request::get("/read?p=v-1-01-p0002")));
        assert!(
            html.contains("href=\"/read?p=v-1-01-p0001\""),
            "prev paragraph"
        );
        assert!(
            html.contains("href=\"/read?p=v-1-02-p0001\""),
            "next paragraph + chapter"
        );
        assert!(
            html.contains("href=\"/read?p=v-2-01-p0001\""),
            "next volume"
        );
        let first = body(&app.handle(&Request::get("/read?p=v-1-01-p0001")));
        assert!(first.contains("已是首段"), "first paragraph has no prev");
        assert!(first.contains("已是首卷"));
    }

    #[test]
    fn read_page_wires_arrow_keys_to_paragraph_nav() {
        let app = fixture();
        let html = body(&app.handle(&Request::get("/read?p=v-1-01-p0002")));
        assert!(html.contains(r#"data-nav="prev""#), "{html}");
        assert!(html.contains(r#"data-nav="next""#), "{html}");
        assert!(html.contains("ArrowLeft") && html.contains("ArrowRight"));
        assert!(html.contains("← → 鍵：上一段 / 下一段"));

        let first = body(&app.handle(&Request::get("/read?p=v-1-01-p0001")));
        assert!(
            !first.contains(r#"data-nav="prev""#),
            "first paragraph has no prev"
        );
        assert!(first.contains(r#"data-nav="next""#));
    }

    #[test]
    fn story_page_maps_arrows_to_chapters_and_up_down_to_paragraphs() {
        let app = fixture();
        let html = body(&app.handle(&Request::get("/story?c=v-1-02")));
        assert!(html.contains(r#"data-arrow-keys="ud""#), "{html}");
        assert!(html.contains(r#"data-arrow-nav="chapter""#), "{html}");
        assert!(html.contains(r#"data-story-nav="prev""#), "{html}");
        assert!(html.contains(r#"data-story-nav="next""#), "{html}");
        assert!(html.contains("ArrowUp") && html.contains("ArrowDown"), "{html}");
        assert!(html.contains("← → 鍵：上一章 / 下一章"), "{html}");
        // The first chapter still shows the disabled span, so no prev href exists.
        let first = body(&app.handle(&Request::get("/story?c=v-1-01")));
        assert!(!first.contains(r#"data-story-nav="prev""#), "{first}");
        assert!(first.contains("已是第一章"));
    }

    #[test]
    fn table_of_contents_walks_volumes_with_left_and_right() {
        let app = fixture();
        let html = body(&app.handle(&Request::get("/")));
        assert!(html.contains(r#"data-arrow-step="div.vol[data-vol-step]""#), "{html}");
        assert!(html.contains(r#"class="vol" data-vol-step"#), "{html}");
        assert!(html.contains("← → 鍵：上一卷 / 下一卷"), "{html}");
    }

    #[test]
    fn table_of_contents_offers_continue_reading_from_the_cookie() {
        let app = fixture();
        let request = Request::get("/").with_header("Cookie", "zgc_last=v-1-02-p0001");
        let html = body(&app.handle(&request));
        assert!(html.contains("繼續閱讀"), "{html}");
        assert!(html.contains("蘇秦說齊王"));

        let stale = Request::get("/").with_header("Cookie", "zgc_last=ghost-p0001");
        let html = body(&app.handle(&stale));
        assert!(!html.contains("繼續閱讀"));
    }

    #[test]
    fn search_finds_real_hits_with_highlighting() {
        let app = fixture();
        let response = app.handle(&Request::get("/search?q=%E8%A1%9E%E9%9E%85"));
        assert_eq!(response.status, 200);
        let html = body(&response);
        assert!(html.contains("<mark>衞鞅</mark>"), "{html}");
        assert!(html.contains("甲一") && html.contains("衞鞅亡魏入秦"));
        assert!(html.contains("命中"));
    }

    #[test]
    fn search_scope_limits_to_one_group() {
        let app = fixture();
        let scoped = body(&app.handle(&Request::get("/search?q=%E7%A7%A6&group=b")));
        assert!(scoped.contains("張儀"));
        assert!(
            !scoped.contains("河西"),
            "group b must exclude group a hits"
        );

        let all = body(&app.handle(&Request::get("/search?q=%E7%A7%A6")));
        assert!(all.contains("河西"));
        assert!(all.contains("張儀"));
    }

    #[test]
    fn search_escapes_user_input() {
        let app = fixture();
        let html = body(&app.handle(&Request::get("/search?q=%3Cscript%3Ealert(1)%3C/script%3E")));
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("沒有找到"));
    }

    #[test]
    fn search_without_a_query_renders_a_hint_not_an_error() {
        let app = fixture();
        let response = app.handle(&Request::get("/search"));
        assert_eq!(response.status, 200);
        assert!(body(&response).contains("輸入檢索詞"));
    }

    #[test]
    fn unknown_group_falls_back_to_the_whole_book_with_a_notice() {
        let app = fixture();
        let html = body(&app.handle(&Request::get("/search?q=%E7%A7%A6&group=ghost")));
        assert!(html.contains("未知國策"), "{html}");
        assert!(html.contains("河西"));
    }

    #[test]
    fn unknown_route_is_404_and_non_get_is_405() {
        let app = fixture();
        assert_eq!(app.handle(&Request::get("/nope")).status, 404);
        let post = app.handle(&Request::from_target("POST", "/read?p=v-1-01-p0001"));
        assert_eq!(post.status, 405);
        assert_eq!(header(&post, "Allow"), vec!["GET"]);
    }

    #[test]
    fn favicon_short_circuits_with_no_body() {
        let app = fixture();
        let response = app.handle(&Request::get("/favicon.ico"));
        assert_eq!(response.status, 204);
        assert!(response.body.is_empty());
    }

    // ------------------------------------------------------------- entities

    fn entity_fixture() -> App {
        let data = toy_data();
        let mut wei_yang = EntityRecord::new(
            "person-wei-yang",
            EntityKind::Person,
            "衞鞅",
            vec![OccurrenceRecord::new("v-1-01-p0001", 0, 2, "衞鞅")],
        );
        wei_yang.name_simp = Some("卫鞅".into());
        wei_yang.aliases = vec!["商君".into()];
        wei_yang.tags = vec!["法家".into()];
        wei_yang.summary = Some("法家代表，相秦。".into());

        let mut su_qin = EntityRecord::new(
            "person-su-qin",
            EntityKind::Person,
            "蘇秦",
            vec![OccurrenceRecord::new("v-1-02-p0001", 0, 2, "蘇秦")],
        );
        su_qin.summary = Some("<script>alert(1)</script>".into());

        // Same start as 秦 in v-1-01-p0002, and longer: the overlap rule must
        // pick this one and drop 秦 there, leaving no nested anchors.
        let mut qin_fa = EntityRecord::new(
            "place-qin-fa",
            EntityKind::Place,
            "秦伐",
            vec![OccurrenceRecord::new("v-1-01-p0002", 0, 2, "秦伐")],
        );
        qin_fa.summary = None;

        let mut qin = EntityRecord::new(
            "place-qin",
            EntityKind::Place,
            "秦",
            vec![
                OccurrenceRecord::new("v-1-01-p0002", 0, 1, "秦"),
                OccurrenceRecord::new("v-2-01-p0001", 0, 1, "秦"),
            ],
        );
        qin.summary = None;

        let entities = Entities::build(
            &data,
            vec![
                EntityFile::new(EntityKind::Person, vec![wei_yang, su_qin]),
                EntityFile::new(EntityKind::Place, vec![qin_fa, qin]),
            ],
        )
        .expect("entity fixture is valid");
        App::with_entities(data, entities)
    }

    #[test]
    fn reading_page_links_entity_spans_and_keeps_the_rest_escaped() {
        let app = entity_fixture();
        let html = body(&app.handle(&Request::get("/read?p=v-1-01-p0001")));
        assert!(
            html.contains(
                "<a class=\"ent ent-person\" href=\"/entity?id=person-wei-yang\" title=\"人物 · 衞鞅\">衞鞅</a>亡魏入秦"
            ),
            "{html}"
        );
        assert!(
            html.contains("<a href=\"/entities\">實體索引</a>"),
            "index is reachable"
        );
    }

    #[test]
    fn reading_page_resolves_overlaps_to_one_anchor_and_never_nests() {
        let app = entity_fixture();
        let html = body(&app.handle(&Request::get("/read?p=v-1-01-p0002")));
        let main_start = html.find("<main>").expect("main");
        let main_end = html.find("</main>").expect("main end");
        let col = &html[main_start..main_end];
        assert_eq!(col.matches("class=\"ent ent-place\"").count(), 1, "{col}");
        assert!(col.contains("title=\"地名 · 秦伐\">秦伐</a>"), "{col}");
        assert!(
            !col.contains("place-qin\""),
            "the shorter 秦 must be dropped here"
        );
        assert_eq!(
            col.matches("<a ").count(),
            col.matches("</a>").count(),
            "no nesting"
        );
    }

    #[test]
    fn entity_page_shows_fields_and_every_occurrence() {
        let app = entity_fixture();
        let response = app.handle(&Request::get("/entity?id=person-wei-yang"));
        assert_eq!(response.status, 200);
        let html = body(&response);
        assert!(html.contains("衞鞅"), "{html}");
        assert!(html.contains("卫鞅"), "simplified name");
        assert!(html.contains("商君"), "alias");
        assert!(html.contains("法家"), "tag");
        assert!(html.contains("法家代表，相秦。"), "summary");
        assert!(html.contains("出現 1 次 · 見於 1 段"), "{html}");
        assert!(
            html.contains("href=\"/read?p=v-1-01-p0001&amp;hl=0,2\""),
            "occurrence links back with hl"
        );
        let vol = html.find("甲一").expect("volume heading");
        let chap = html.find("衞鞅亡魏入秦").expect("chapter heading");
        assert!(vol < chap, "volume groups above chapter");
        assert!(
            html.contains("<mark>衞鞅</mark>"),
            "excerpt marks the match"
        );
    }

    #[test]
    fn entity_page_prints_none_as_a_placeholder_not_blank() {
        let app = entity_fixture();
        let html = body(&app.handle(&Request::get("/entity?id=place-qin")));
        assert!(html.contains("暫無"), "{html}");
        assert!(html.contains("出現 2 次 · 見於 2 段"), "{html}");
        assert!(html.contains("href=\"/read?p=v-2-01-p0001&amp;hl=0,1\""));
    }

    #[test]
    fn entity_page_escapes_corpus_text() {
        let app = entity_fixture();
        let html = body(&app.handle(&Request::get("/entity?id=person-su-qin")));
        assert!(!html.contains("<script>"), "{html}");
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    }

    #[test]
    fn unknown_entity_id_is_404_and_escaped_while_missing_id_is_400() {
        let app = entity_fixture();
        let response = app.handle(&Request::get("/entity?id=%3Cscript%3Eghost%3C/script%3E"));
        assert_eq!(response.status, 404);
        let html = body(&response);
        assert!(html.contains("&lt;script&gt;ghost"), "{html}");
        assert!(!html.contains("<script>"));

        assert_eq!(app.handle(&Request::get("/entity")).status, 400);
        assert_eq!(app.handle(&Request::get("/entity?id=")).status, 404);
        assert_eq!(
            app.handle(&Request::get("/entity?id=person-ghost")).status,
            404
        );
    }

    #[test]
    fn entity_index_lists_counts_filters_by_kind_and_sorts() {
        let app = entity_fixture();
        let html = body(&app.handle(&Request::get("/entities")));
        assert!(html.contains("4 個實體"), "{html}");
        assert!(html.contains("5 次出現"), "{html}");
        assert!(html.contains("人物 2 個 / 2 次"), "{html}");
        assert!(html.contains("地名 2 個 / 3 次"), "{html}");
        // Default order: most occurrences first.
        let qin = html
            .find("href=\"/entity?id=place-qin\"")
            .expect("place-qin row");
        let wei = html
            .find("href=\"/entity?id=person-wei-yang\"")
            .expect("person row");
        assert!(qin < wei, "2 occurrences before 1");

        let places = body(&app.handle(&Request::get("/entities?kind=place")));
        assert!(places.contains("place-qin") && places.contains("place-qin-fa"));
        assert!(
            !places.contains("person-wei-yang"),
            "kind filter excludes people"
        );
        assert!(
            places.contains("kind=place&amp;sort=name"),
            "filter is preserved in sort links"
        );

        let by_name = body(&app.handle(&Request::get("/entities?kind=place&sort=name")));
        let fa = by_name.find("place-qin-fa").expect("秦伐");
        let qin_name = by_name.find("href=\"/entity?id=place-qin\"").expect("秦");
        assert!(
            qin_name < fa,
            "name order: 秦 is a prefix of 秦伐, so 秦 comes first"
        );
    }

    #[test]
    fn read_hl_marks_the_entity_and_ignores_everything_illegal() {
        let app = entity_fixture();
        let marked = body(&app.handle(&Request::get("/read?p=v-1-01-p0001&hl=0,2")));
        assert!(
            marked.contains("class=\"ent ent-person ent-hl\""),
            "{marked}"
        );
        assert!(marked.contains("<mark>衞鞅</mark>"));

        for hl in [
            "",
            "abc",
            "0",
            "0,0",
            "2,1",
            "999,1",
            "-1,2",
            "0,2,3",
            "18446744073709551615,2",
        ] {
            let target = format!("/read?p=v-1-01-p0001&hl={hl}");
            let response = app.handle(&Request::get(&target));
            assert_eq!(response.status, 200, "hl={hl} must not fail the page");
            let html = body(&response);
            assert!(
                !html.contains("class=\"ent ent-person ent-hl\""),
                "hl={hl} must be ignored: {html}"
            );
            assert!(!html.contains("<mark>"), "hl={hl} must not mark: {html}");
        }
        // A range inside the span still resolves to it; outside draws nothing.
        assert!(body(&app.handle(&Request::get("/read?p=v-1-01-p0001&hl=1,1"))).contains("<mark>"));
    }

    /// A table with just the two glyph pairs the fixture text needs.
    fn toy_variants() -> Variants {
        Variants::parse("衞\t卫\n爲\t为\n")
    }

    /// Every id attribute on the page, in document order.
    fn ids(html: &str) -> Vec<String> {
        html.match_indices(" id=\"")
            .map(|(at, _)| {
                let rest = &html[at + " id=\"".len()..];
                rest[..rest.find('"').expect("closing quote")].to_string()
            })
            .collect()
    }

    /// The inner markup of each 簡體 line, in document order.
    fn simp_lines(html: &str) -> Vec<&str> {
        html.split("<p class=\"simp-line\">")
            .skip(1)
            .filter_map(|rest| rest.split("</p>").next())
            .collect()
    }

    #[test]
    fn both_mode_prints_the_traditional_line_then_its_simplified_twin() {
        let app = entity_fixture().with_variants(toy_variants());
        let html = body(&app.handle(&Request::get("/read?p=v-1-01-p0001")));
        assert!(
            html.contains("<html lang=\"zh-Hant\""),
            "對照 is a 繁體 page: {html}"
        );
        assert!(html.contains("<h2>原文（繁 / 簡）</h2>"), "{html}");
        assert!(html.contains("簡體行為機器轉換，非校勘"), "{html}");
        let trad = html.find("<p class=\"orig-line\">").expect("trad line");
        let simp = html.find("<p class=\"simp-line\">").expect("simp line");
        assert!(trad < simp, "the 簡體 line follows its 繁體 source");
        assert!(
            html.contains("<p class=\"simp-line\">卫鞅亡魏入秦，孝公以为相。</p>"),
            "{html}"
        );
        assert_eq!(
            html.matches("<p class=\"orig-line\">").count(),
            html.matches("<p class=\"simp-line\">").count(),
            "every 原文 line gets exactly one counterpart"
        );
    }

    #[test]
    fn both_mode_keeps_anchors_and_ids_on_the_traditional_line_only() {
        let app = entity_fixture().with_variants(toy_variants());
        let html = body(&app.handle(&Request::get("/read?p=v-1-01-p0001&sc=both&hl=0,2")));
        let mut seen = std::collections::BTreeSet::new();
        let duplicates: Vec<String> = ids(&html)
            .into_iter()
            .filter(|id| !seen.insert(id.clone()))
            .collect();
        assert!(duplicates.is_empty(), "duplicate ids: {duplicates:?}");
        assert!(
            html.contains("class=\"ent ent-person ent-hl\""),
            "anchor stays on the 繁體 line"
        );
        let lines = simp_lines(&html);
        assert!(!lines.is_empty(), "{html}");
        assert!(
            lines
                .iter()
                .all(|line| !line.contains("<a ") && !line.contains("href=")),
            "the 簡體 line is text, not a target: {lines:?}"
        );
        // Same offsets, same highlight: the table keeps the character count.
        assert!(
            lines.iter().any(|line| line.contains("<mark>卫鞅</mark>")),
            "{lines:?}"
        );
    }

    #[test]
    fn trad_and_simp_modes_print_one_line_and_name_their_script() {
        let app = entity_fixture().with_variants(toy_variants());
        let response = app.handle(&Request::get("/read?p=v-1-01-p0001&sc=trad"));
        let trad = body(&response);
        assert!(trad.contains("<h2>原文（繁體）</h2>"), "{trad}");
        assert!(!trad.contains("<p class=\"simp-line\">"), "one line only");
        assert!(!trad.contains("非校勘"), "no conversion, no note");
        assert!(trad.contains("衞鞅"), "繁體 glyphs");
        assert!(
            header(&response, "Set-Cookie")
                .iter()
                .any(|value| value.starts_with("zgc_script=trad")),
            "the choice is carried back in a cookie"
        );
        assert!(trad.contains("<html lang=\"zh-Hant\""));

        let simp = body(&app.handle(&Request::get("/read?p=v-1-01-p0001&sc=simp")));
        assert!(simp.contains("<h2>原文（簡體）</h2>"), "{simp}");
        assert!(!simp.contains("<p class=\"simp-line\">"), "one line only");
        assert!(simp.contains("機器轉換，非校勘"), "{simp}");
        assert!(
            simp.contains("<p class=\"orig-line\"><a class=\"ent ent-person\" href=\"/entity?id=person-wei-yang\" title=\"人物 · 卫鞅\">卫鞅</a>亡魏入秦，孝公以为相。</p>"),
            "the 簡體 line keeps its anchors: {simp}"
        );
        assert!(!simp.contains("衞鞅"), "簡體 mode shows no 繁體 glyph");
        assert!(simp.contains("<html lang=\"zh-Hans\""));
    }

    #[test]
    fn script_chips_offer_three_states_and_mark_the_current_one() {
        let app = entity_fixture().with_variants(toy_variants());
        let html = body(&app.handle(&Request::get("/read?p=v-1-01-p0001")));
        assert!(
            html.contains("title=\"繁簡對照\">對照</span>"),
            "對照 is current: {html}"
        );
        assert!(
            html.contains("?p=v-1-01-p0001&amp;sc=trad\" title=\"繁體\">繁</a>"),
            "{html}"
        );
        assert!(
            html.contains("?p=v-1-01-p0001&amp;sc=simp\" title=\"簡體\">簡</a>"),
            "{html}"
        );

        let trad = body(&app.handle(&Request::get("/read?p=v-1-01-p0001&sc=trad")));
        assert!(
            trad.contains("title=\"繁體\">繁</span>"),
            "繁 is current: {trad}"
        );
        assert!(
            trad.contains("?p=v-1-01-p0001&amp;sc=both\" title=\"繁簡對照\">對照</a>"),
            "{trad}"
        );
    }

    #[test]
    fn without_the_table_both_degrades_to_traditional() {
        let app = entity_fixture();
        let html = body(&app.handle(&Request::get("/read?p=v-1-01-p0001")));
        assert!(html.contains("<h2>原文（繁體）</h2>"), "{html}");
        assert!(!html.contains("<p class=\"simp-line\">"));
        assert!(
            !html.contains("class=\"seg seg-script\""),
            "no dead 字形 switch without a table"
        );
    }

    #[test]
    fn search_results_name_the_entities_of_each_hit() {
        let app = entity_fixture();
        let html = body(&app.handle(&Request::get("/search?q=%E8%A1%9E%E9%9E%85")));
        assert!(html.contains("hit-ents"), "{html}");
        assert!(html.contains("href=\"/entity?id=person-wei-yang\""));
    }

    #[test]
    fn reading_page_without_entities_renders_plain_text() {
        let app = fixture();
        let html = body(&app.handle(&Request::get("/read?p=v-1-01-p0001")));
        assert!(html.contains("衞鞅亡魏入秦，孝公以爲相。"));
        assert!(!html.contains("class=\"ent "));
        assert_eq!(app.entities().len(), 0);
    }

    // ------------------------------------------------------ story view (M6)

    use std::sync::{Arc, Mutex};

    use crate::ai::{AiReply, FetchedPage, SearchHit};

    struct FakeClients {
        reply: String,
        hits: Vec<SearchHit>,
        fetch_fails: bool,
        /// Make the search tool fail, so the sidebar's degradation path is
        /// exercised without a network.
        search_fails: bool,
        user_prompt: Arc<Mutex<String>>,
    }

    impl AiClients for FakeClients {
        fn chat(&self, _config: &AiConfig, _system: &str, user: &str) -> Result<AiReply, AiError> {
            *self.user_prompt.lock().expect("prompt lock") = user.to_string();
            Ok(AiReply {
                content: self.reply.clone(),
                model: Some("fake-model".to_string()),
                prompt_tokens: Some(12),
                completion_tokens: Some(34),
                elapsed_ms: 5,
            })
        }

        fn search(
            &self,
            _config: &AiConfig,
            _query: &str,
            _count: usize,
        ) -> Result<Vec<SearchHit>, AiError> {
            if self.search_fails {
                Err(AiError::Transport("離線測試".to_string()))
            } else {
                Ok(self.hits.clone())
            }
        }

        fn fetch(&self, _config: &AiConfig, _url: &str) -> Result<FetchedPage, AiError> {
            if self.fetch_fails {
                Err(AiError::Transport("離線測試".to_string()))
            } else {
                Ok(FetchedPage {
                    content: "外部全文內容".to_string(),
                })
            }
        }
    }

    fn fake_clients(
        reply: &str,
        hits: Vec<SearchHit>,
        fetch_fails: bool,
    ) -> (FakeClients, Arc<Mutex<String>>) {
        let user_prompt = Arc::new(Mutex::new(String::new()));
        (
            FakeClients {
                reply: reply.to_string(),
                hits,
                fetch_fails,
                search_fails: false,
                user_prompt: Arc::clone(&user_prompt),
            },
            user_prompt,
        )
    }

    fn search_hit(title: &str, url: &str, summary: &str) -> SearchHit {
        SearchHit {
            title: title.to_string(),
            site_name: "測試站".to_string(),
            url: url.to_string(),
            summary: summary.to_string(),
            rerank_score: 2.0,
        }
    }

    fn story_ai_app(clients: FakeClients) -> App {
        fixture()
            .with_variants(Variants::parse("衞\t卫\n爲\t为\n"))
            .with_ai(AiConfig::from_lookup(|name| match name {
                "ZGC_AI_BASE" => Some("https://api.deepseek.com".to_string()),
                "ZGC_AI_KEY" => Some("sk-test-secret".to_string()),
                "ZGC_AI_MODEL" => Some("mock-model".to_string()),
                _ => None,
            }))
            .with_ai_clients(Box::new(clients))
    }

    /// Cut one sidebar block out of the chapter markup by its heading, so a
    /// failure shows that block instead of the whole page.
    fn side_block<'a>(html: &'a str, heading: &str) -> &'a str {
        let marker = format!("<h2>{heading}</h2>");
        let head = html
            .find(&marker)
            .unwrap_or_else(|| panic!("側欄缺少「{heading}」塊"));
        let start = html[..head].rfind("<section").expect("塊必須是 section");
        let end = head + html[head..].find("</section>").expect("塊必須閉合") + "</section>".len();
        &html[start..end]
    }

    #[test]
    fn story_sidebar_lists_the_chapters_people_and_places() {
        let app = entity_fixture();
        let html = body(&app.handle(&Request::get("/story?c=v-1-01")));

        assert!(html.contains("<aside class=\"story-side\">"), "{html}");
        let people = side_block(&html, "本章人物");
        assert!(
            people.contains("href=\"/entity?id=person-wei-yang\""),
            "{people}"
        );
        assert!(
            people.contains("<span class=\"side-count\">1</span>"),
            "{people}"
        );
        let places = side_block(&html, "本章地點");
        assert!(
            places.contains("href=\"/entity?id=place-qin-fa\""),
            "{places}"
        );
        // 系年 belongs to the sidebar: the year block sits inside the aside,
        // after the rosters, and the header no longer carries it.
        let when = side_block(&html, "系年");
        assert!(when.contains("此書未附系年表"), "{when}");
        let aside = html.find("<aside class=\"story-side\">").expect("aside");
        assert!(html.find("此書未附系年表").expect("year") > aside);
    }

    #[test]
    fn story_sidebar_names_follow_the_script_state() {
        let app = entity_fixture().with_variants(toy_variants());

        let trad = body(&app.handle(&Request::get("/story?c=v-1-01&sc=trad")));
        assert!(side_block(&trad, "本章人物").contains(">衞鞅</a>"));

        let simp = body(&app.handle(&Request::get("/story?c=v-1-01&sc=simp")));
        let people = side_block(&simp, "本章人物");
        assert!(people.contains(">卫鞅</a>"), "{people}");
        assert!(!people.contains("衞鞅"), "簡體態不該再出現繁體名：{people}");

        let both = body(&app.handle(&Request::get("/story?c=v-1-01&sc=both")));
        let people = side_block(&both, "本章人物");
        assert!(
            people.contains("衞鞅<span class=\"simp\">卫鞅</span>"),
            "{people}"
        );
    }

    #[test]
    fn story_background_block_stays_a_link_until_bg_is_asked_for() {
        let app = entity_fixture();
        let html = body(&app.handle(&Request::get("/story?c=v-1-01&sc=both")));
        let block = side_block(&html, "背景史料");
        assert!(block.contains("檢索背景史料"), "{block}");
        assert!(
            block.contains("/story?c=v-1-01&amp;bg=1&amp;sc=both"),
            "the link keeps the reader state：{block}"
        );
        assert!(!block.contains("未取到史料"), "lazy block must not degrade");
    }

    #[test]
    fn story_background_block_cites_http_hits_and_refuses_other_schemes() {
        let (clients, _) = fake_clients(
            "解讀",
            vec![
                search_hit("甲篇", "https://example.com/a", "摘要甲"),
                search_hit("乙篇", "javascript:alert(1)", "摘要乙"),
            ],
            false,
        );
        let app = story_ai_app(clients);
        let html = body(&app.handle(&Request::get("/story?c=v-1-01&bg=1")));
        let block = side_block(&html, "背景史料");
        assert!(
            block.contains("target=\"_blank\" rel=\"noreferrer noopener\""),
            "{block}"
        );
        assert!(block.contains("href=\"https://example.com/a\""), "{block}");
        assert!(block.contains("摘要甲"), "{block}");
        assert!(
            !block.contains("href=\"javascript"),
            "只有 http(s) 可以成為連結：{block}"
        );
        assert!(block.contains("乙篇"), "非 http(s) 仍保留標題文字：{block}");
    }

    #[test]
    fn story_background_block_reports_a_failed_search() {
        let clients = FakeClients {
            reply: "解讀".to_string(),
            hits: Vec::new(),
            fetch_fails: false,
            search_fails: true,
            user_prompt: Arc::new(Mutex::new(String::new())),
        };
        let app = story_ai_app(clients);
        let response = app.handle(&Request::get("/story?c=v-1-01&bg=1"));
        assert_eq!(response.status, 200);
        let html = body(&response);
        let block = side_block(&html, "背景史料");
        assert!(block.contains("未取到史料"), "{block}");
        assert!(
            html.contains("story-para"),
            "the story itself still renders"
        );
    }

    #[test]
    fn story_background_block_degrades_when_ai_is_not_configured() {
        let app = entity_fixture();
        let response = app.handle(&Request::get("/story?c=v-1-01&bg=1"));
        assert_eq!(response.status, 200);
        let html = body(&response);
        let block = side_block(&html, "背景史料");
        assert!(block.contains("未取到史料"), "{block}");
        assert!(block.contains("ZGC_AI_KEY"), "{block}");
    }

    #[test]
    fn story_page_lists_every_paragraph_in_order() {
        let app = fixture();
        let response = app.handle(&Request::get("/story?c=v-1-01"));
        assert_eq!(response.status, 200);
        let html = body(&response);
        assert!(html.contains("本故事共 2 段"), "{html}");
        assert!(html.contains("故事視界"));
        let first = html.find("id=\"v-1-01-p0001\"").expect("first anchor");
        let second = html.find("id=\"v-1-01-p0002\"").expect("second anchor");
        assert!(first < second, "paragraph order: {html}");
        assert!(html.contains("單段精讀 →"));
        assert!(html.contains("href=\"/read?p=v-1-01-p0002\""));
        assert_eq!(
            html.matches("<script>").count(),
            2,
            "story script plus the notes script"
        );
        assert!(html.contains("data-story-para"));
    }

    #[test]
    fn story_without_c_or_p_is_a_bad_request() {
        let app = fixture();
        let response = app.handle(&Request::get("/story"));
        assert_eq!(response.status, 400);
        assert!(body(&response).contains("缺少參數 c 或 p"));
    }

    #[test]
    fn story_for_an_unknown_chapter_is_404_and_escaped() {
        let app = fixture();
        let response = app.handle(&Request::get("/story?c=nope"));
        assert_eq!(response.status, 404);
        assert!(body(&response).contains("找不到這一章"));
        let escaped = app.handle(&Request::get("/story?c=%3Cscript%3E"));
        assert_eq!(escaped.status, 404);
        let html = body(&escaped);
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>"));
    }

    #[test]
    fn story_accepts_a_paragraph_id_and_resolves_its_chapter() {
        let app = fixture();
        let response = app.handle(&Request::get("/story?p=v-1-01-p0002"));
        assert_eq!(response.status, 200);
        let html = body(&response);
        assert!(html.contains("衞鞅亡魏入秦"));
        assert!(html.contains("id=\"v-1-01-p0001\""));
        assert!(html.contains("id=\"v-1-01-p0002\""));
    }

    #[test]
    fn story_original_follows_all_three_script_states() {
        let app = fixture().with_variants(Variants::parse("衞\t卫\n爲\t为\n"));
        let trad = body(&app.handle(&Request::get("/story?c=v-1-01&sc=trad")));
        let simp = body(&app.handle(&Request::get("/story?c=v-1-01&sc=simp")));
        let both = body(&app.handle(&Request::get("/story?c=v-1-01&sc=both")));
        assert!(trad.contains("衞鞅亡魏入秦"), "{trad}");
        assert!(simp.contains("卫鞅亡魏入秦"), "{simp}");
        assert!(!simp.contains("衞鞅亡魏入秦"), "{simp}");
        assert!(both.contains("class=\"orig-line\""), "{both}");
        assert!(both.contains("class=\"simp-line\""), "{both}");
        assert_ne!(trad, simp);
        assert_ne!(trad, both);
    }

    #[test]
    fn story_ai_renders_the_four_sections_from_the_fake_upstream() {
        let reply = "背景\n秦孝公任用衞鞅。〔v-1-01-p0001〕\n寓意\n變法須有信任。〔v-1-01-p0001〕\n可學之處\n先立信而後行法。〔v-1-01-p0001〕\n參考來源\n無";
        let (clients, prompt) = fake_clients(reply, Vec::new(), false);
        let app = story_ai_app(clients);
        let response = app.handle(&Request::get("/story?c=v-1-01&ai=1"));
        assert_eq!(response.status, 200);
        let html = body(&response);
        for heading in ["背景", "寓意", "可學之處", "參考來源"] {
            assert!(html.contains(heading), "missing {heading}: {html}");
        }
        let prompt = prompt.lock().expect("prompt lock").clone();
        assert!(prompt.contains("【原文】"));
        assert!(prompt.contains("〔v-1-01-p0001〕"));
        assert!(prompt.contains("注疏（鮑彪·四部叢刊）"), "{prompt}");
        assert!(!prompt.contains("【外部資料】"));
    }

    #[test]
    fn story_ai_prompt_carries_external_material_and_source_numbers() {
        let (clients, prompt) = fake_clients(
            "背景\n外部說法。〔v-1-01-p0001〕【來源 1】\n寓意\n材料不足。\n可學之處\n材料不足。\n參考來源\n外部標題 · 測試站 · https://example.com/a",
            vec![search_hit("外部標題", "https://example.com/a", "摘要可作材料")],
            false,
        );
        let app = story_ai_app(clients);
        let response = app.handle(&Request::get("/story?c=v-1-01&ai=1"));
        assert_eq!(response.status, 200);
        let prompt = prompt.lock().expect("prompt lock").clone();
        assert!(prompt.contains("【外部資料】"), "{prompt}");
        assert!(prompt.contains("【來源 1】外部標題"), "{prompt}");
        assert!(prompt.contains("https://example.com/a"), "{prompt}");
        assert!(prompt.contains("外部全文內容"), "{prompt}");
        let html = body(&response);
        assert!(html.contains("href=\"https://example.com/a\""), "{html}");
        assert!(html.contains("target=\"_blank\""));
        assert!(html.contains("外部資料來自公開網頁，未經校勘。"));
    }

    #[test]
    fn story_ai_without_search_results_notes_the_degradation() {
        let (clients, prompt) = fake_clients(
            "背景\n材料不足。\n寓意\n材料不足。\n可學之處\n材料不足。\n參考來源\n無",
            Vec::new(),
            false,
        );
        let app = story_ai_app(clients);
        let response = app.handle(&Request::get("/story?c=v-1-01&ai=1"));
        assert_eq!(response.status, 200);
        let html = body(&response);
        assert!(
            html.contains("未能取得外部資料，本解讀僅據原文與注疏"),
            "{html}"
        );
        let prompt = prompt.lock().expect("prompt lock").clone();
        assert!(!prompt.contains("【外部資料】"), "{prompt}");
    }

    #[test]
    fn story_ai_never_links_a_non_http_source() {
        let hits = vec![
            search_hit("腳本來源", "javascript:alert(1)", "不應成為連結"),
            search_hit("資料來源", "data:text/html,hi", "也不應成為連結"),
        ];
        let (clients, _prompt) = fake_clients(
            "背景\n材料不足。\n寓意\n材料不足。\n可學之處\n材料不足。\n參考來源\n無",
            hits,
            true,
        );
        let app = story_ai_app(clients);
        let response = app.handle(&Request::get("/story?c=v-1-01&ai=1"));
        assert_eq!(response.status, 200);
        let html = body(&response);
        assert!(!html.contains("href=\"javascript:"), "{html}");
        assert!(!html.contains("href=\"data:"), "{html}");
        assert!(html.contains("腳本來源"), "{html}");
        assert!(html.contains("資料來源"), "{html}");
    }

    #[test]
    fn story_ai_uses_the_search_summary_when_fetch_fails() {
        let (clients, prompt) = fake_clients(
            "背景\n材料不足。\n寓意\n材料不足。\n可學之處\n材料不足。\n參考來源\n無",
            vec![search_hit(
                "摘要來源",
                "https://example.com/b",
                "摘要可作材料",
            )],
            true,
        );
        let app = story_ai_app(clients);
        let response = app.handle(&Request::get("/story?c=v-1-01&ai=1"));
        assert_eq!(response.status, 200);
        let prompt = prompt.lock().expect("prompt lock").clone();
        assert!(prompt.contains("摘要可作材料"), "{prompt}");
        assert!(!prompt.contains("外部全文內容"), "{prompt}");
        assert!(body(&response).contains("href=\"https://example.com/b\""));
    }

    #[test]
    fn story_ai_without_configuration_still_renders_the_chapter() {
        let app = fixture();
        let response = app.handle(&Request::get("/story?c=v-1-01&ai=1"));
        assert_eq!(response.status, 200);
        let html = body(&response);
        assert!(html.contains("尚未配置 AI"), "{html}");
        assert!(html.contains("id=\"v-1-01-p0001\""), "{html}");
    }

    #[test]
    fn story_ai_strips_markdown_heading_markers() {
        let reply = "## 背景\n**寓意**\n材料不足。\n### 可學之處\n材料不足。\n**參考來源**\n無";
        let (clients, _prompt) = fake_clients(reply, Vec::new(), false);
        let app = story_ai_app(clients);
        let html = body(&app.handle(&Request::get("/story?c=v-1-01&ai=1")));
        for heading in ["背景", "寓意", "可學之處", "參考來源"] {
            assert!(
                html.contains(&format!("<p>{heading}</p>")),
                "{heading}: {html}"
            );
        }
        assert!(!html.contains("## 背景"), "{html}");
        assert!(!html.contains("**寓意**"), "{html}");
    }

    #[test]
    fn read_page_links_to_the_chapter_story() {
        let app = fixture();
        let html = body(&app.handle(&Request::get("/read?p=v-1-01-p0001")));
        assert!(
            html.contains("href=\"/story?p=v-1-01-p0001\""),
            "the reading page needs a story entrance: {html}"
        );
        assert!(html.contains("整章故事視界 →"), "{html}");
        let cta = html.find("class=\"story-cta\"").expect("story cta");
        let toolbar = html.find("class=\"toolbar\"").expect("toolbar");
        assert!(cta < toolbar, "the story link sits above the toolbar");
    }

    #[test]
    fn toc_and_home_link_to_story_chapters() {
        let app = fixture();
        let html = body(&app.handle(&Request::get("/")));
        assert!(html.contains("故事視界"), "{html}");
        assert!(
            html.contains("以章（一篇策文）為單位通讀，並生成背景 / 寓意 / 可學。"),
            "{html}"
        );
        assert!(html.contains("href=\"/story?c=dongzhou-01\""), "{html}");
        for chapter in ["v-1-01", "v-1-02", "v-2-01"] {
            let link = format!("class=\"story-link\" href=\"/story?c={chapter}\"");
            assert!(html.contains(&link), "missing {link}: {html}");
        }
        // The chapter title still opens the per-paragraph reader.
        assert!(html.contains("href=\"/read?p=v-1-01-p0001\""), "{html}");
    }

    #[test]
    fn story_link_text_is_simplified_in_simp_mode() {
        let app = fixture().with_variants(Variants::parse("視\t视\n進\t进\n為\t为\n"));
        let read = body(&app.handle(&Request::get("/read?p=v-1-01-p0001&sc=simp")));
        assert!(read.contains("整章故事视界 →"), "{read}");
        assert!(!read.contains("整章故事視界"), "{read}");
        let home = body(&app.handle(&Request::get("/?sc=simp")));
        assert!(home.contains("故事视界"), "{home}");
        assert!(home.contains("进入故事视界 →"), "{home}");
        assert!(!home.contains("故事視界"), "{home}");
    }

    // ------------------------------------------------------ AI companion (M5)

    /// One-shot OpenAI-compatible endpoint. Returns the base URL to point the
    /// client at; the server answers exactly one request and closes.
    fn mock_upstream(status: &str, body: &str) -> String {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
        let port = listener.local_addr().expect("addr").port();
        let status = status.to_string();
        let body = body.to_string();
        std::thread::spawn(move || {
            let Ok((mut socket, _)) = listener.accept() else {
                return;
            };
            // Drain the head and the body before answering: closing the write
            // half while ureq is still sending makes the client see a reset.
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while socket.read(&mut byte).unwrap_or(0) == 1 {
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
            let mut payload = vec![0u8; length];
            let _ = socket.read_exact(&mut payload);
            {
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes());
            }
        });
        format!("http://127.0.0.1:{port}/v1")
    }

    fn configured_app(base: &str) -> App {
        let base = base.to_string();
        fixture().with_ai(AiConfig::from_lookup(move |name| match name {
            "ZGC_AI_BASE" => Some(base.clone()),
            "ZGC_AI_KEY" => Some("sk-test-secret".to_string()),
            "ZGC_AI_MODEL" => Some("mock-model".to_string()),
            _ => None,
        }))
    }

    #[test]
    fn ask_without_a_paragraph_is_a_bad_request() {
        let app = fixture();
        let response = app.handle(&Request::get("/ask"));
        assert_eq!(response.status, 400);
        assert!(body(&response).contains("缺少參數 p"));
    }

    #[test]
    fn ask_for_an_unknown_paragraph_is_not_found() {
        let app = fixture();
        let response = app.handle(&Request::get("/ask?p=v-9-99-p0009"));
        assert_eq!(response.status, 404);
        assert!(body(&response).contains("找不到這一段"));
    }

    #[test]
    fn ask_without_a_question_shows_the_form_and_the_context() {
        let app = fixture();
        let response = app.handle(&Request::get("/ask?p=v-1-01-p0001"));
        assert_eq!(response.status, 200);
        let html = body(&response);
        assert!(html.contains("name=\x22q\x22"), "textarea missing: {html}");
        assert!(
            html.contains("name=\x22p\x22 value=\x22v-1-01-p0001\x22")
                || html.contains("value=\x22v-1-01-p0001\x22")
        );
        assert!(html.contains("action=\x22/ask\x22"));
        assert!(html.contains("method=\x22get\x22"));
        // Unconfigured: the page explains what to set, and never 500s.
        assert!(html.contains("ZGC_AI_KEY"), "{html}");
        assert!(html.contains("ZGC_AI_MODEL"), "{html}");
        assert!(html.contains("衞鞅亡魏入秦，孝公以爲相。"));
    }

    #[test]
    fn ask_reaches_the_configured_endpoint_and_renders_the_answer() {
        let reply = r#"{"model":"mock-model","choices":[{"message":{"role":"assistant","content":"此段言衞鞅入秦。"}}],"usage":{"prompt_tokens":120,"completion_tokens":18}}"#;
        let base = mock_upstream("200 OK", reply);
        let app = configured_app(&base);
        let response = app.handle(&Request::get(
            "/ask?p=v-1-01-p0001&q=%E6%AD%A4%E6%AE%B5%E4%BD%95%E6%84%8F",
        ));
        assert_eq!(response.status, 200);
        let html = body(&response);
        assert!(html.contains("此段言衞鞅入秦。"), "{html}");
        assert!(html.contains("mock-model"));
        assert!(html.contains("120"), "token usage missing: {html}");
        assert!(html.contains(&format!("本段原文與注疏將傳送至 {base}")));
        assert!(!html.contains("sk-test-secret"), "key leaked into the page");
    }

    #[test]
    fn an_upstream_failure_is_reported_in_words_without_the_key() {
        let base = mock_upstream(
            "401 Unauthorized",
            r#"{"error":{"message":"bad key sk-test-secret"}}"#,
        );
        let app = configured_app(&base);
        let response = app.handle(&Request::get("/ask?p=v-1-01-p0001&q=%E4%BD%95%E6%84%8F"));
        assert_eq!(response.status, 502);
        let html = body(&response);
        assert!(html.contains("金鑰無效"), "{html}");
        assert!(html.contains("401"));
        assert!(!html.contains("sk-test-secret"), "key leaked into the page");
    }
}

// ------------------------------------------------------------- AI companion

/// Lift the paragraph, its commentary, its dating and its entities into the
/// plain context the prompt module consumes. Pure: no network, no environment.
impl App {
    pub fn ask_context(&self, position: Position) -> Option<AskContext> {
        let data = self.data();
        let paragraph = data.paragraph(position)?;
        let paragraph_id = self.paragraph_id_at(position)?;
        let volume = data.volume(position.volume);
        let chapter = data.chapter(position.volume, position.chapter);

        let notes = paragraph
            .annotations
            .iter()
            .map(|annotation| NoteText {
                source: annotation.source.clone().unwrap_or_default(),
                text: annotation.text.clone(),
            })
            .collect();
        let glossary = paragraph
            .glossary
            .iter()
            .map(|entry| GlossaryText {
                term: entry.term.clone(),
                pinyin: entry.pinyin.clone(),
                gloss: entry.gloss.clone(),
            })
            .collect();

        let mut seen: Vec<&str> = Vec::new();
        let mut entities = Vec::new();
        for span in self.entities().spans_in(paragraph_id) {
            let entity = span.entity;
            if seen.contains(&entity.id.as_str()) {
                continue;
            }
            seen.push(&entity.id);
            let mut note = String::new();
            let aliases: Vec<&str> = entity
                .aliases
                .iter()
                .map(|alias| alias.trim())
                .filter(|alias| !alias.is_empty() && *alias != entity.name)
                .collect();
            if !aliases.is_empty() {
                note.push_str(&format!("別稱 {}", aliases.join("、")));
            }
            if let Some(summary) = entity.summary_text() {
                if !note.is_empty() {
                    note.push_str("；");
                }
                note.push_str(summary);
            }
            entities.push(EntityContext {
                kind: entity.kind.label().to_string(),
                name: entity.name.clone(),
                note: Some(note).filter(|note| !note.trim().is_empty()),
            });
        }

        Some(AskContext {
            book_title: data.book().title.clone(),
            group_title: volume
                .and_then(|volume| data.group(&volume.group_id))
                .map(|group| group.title.clone()),
            volume_title: volume.map(|volume| volume.title.clone()),
            chapter_title: chapter.map(|chapter| chapter.title.clone()),
            paragraph_id: paragraph_id.to_string(),
            paragraph_ordinal: position.paragraph + 1,
            paragraph_total: chapter.map(|chapter| chapter.paragraphs.len()).unwrap_or(0),
            original: paragraph.text.clone(),
            translation: paragraph.translation.clone(),
            notes,
            glossary,
            year: chapter.and_then(|chapter| self.year_context(&chapter.id)),
            entities,
        })
    }

    /// The chapter's dating, flattened to display lines. Absent when the book
    /// carries no year table at all — then nothing claims to know.
    fn year_context(&self, chapter_id: &str) -> Option<YearContext> {
        let years = self.chapter_years();
        if years.is_empty() {
            return None;
        }
        let Some(record) = years.get(chapter_id) else {
            return years
                .unresolved_reason(chapter_id)
                .map(|reason| YearContext {
                    headline: format!("未能系年：{}", zgc_core::reason_text(reason)),
                    evidence: Vec::new(),
                });
        };
        let [start, end] = record.year_range;
        let headline = if start == end {
            format!(
                "約前{}年 · 置信度 {}",
                record.year_bc,
                record.confidence.label()
            )
        } else {
            format!(
                "約前{}年（前{}–前{}年間）· 置信度 {}",
                record.year_bc,
                start,
                end,
                record.confidence.label()
            )
        };
        Some(YearContext {
            headline,
            evidence: record
                .evidence
                .iter()
                .map(|row| self.evidence_line(row))
                .collect(),
        })
    }

    fn evidence_line(&self, evidence: &zgc_core::Evidence) -> String {
        let claim = match &evidence.text {
            EvidenceText::Names(names) => format!("共見人物：{}", clip(&names.join("、"), 80)),
            EvidenceText::Matched(matched) if evidence.kind.starts_with("substring") => {
                format!("章文「{}」", clip(matched, 80))
            }
            EvidenceText::Matched(text) => format!("注疏「{}」", clip(text, 80)),
        };
        let event = match self.events().event(&evidence.event_id) {
            Some(event) => format!("通鑑前{}年：{}", event.year_bc, clip(&event.text, 80)),
            None => format!("事件表未載 {}", evidence.event_id),
        };
        format!(
            "{}；{}；{}",
            pages::evidence_kind_label(&evidence.kind),
            claim,
            event
        )
    }

    /// GET /ask — either the form, the configuration notice, the answer, or the
    /// upstream failure. Only a missing/unknown paragraph is an error status;
    /// everything the upstream does is reported on a page a human can read.
    fn handle_ask(&self, raw_query: &str, cookies: &Cookies) -> Response {
        let query = urlcodec::parse_query(raw_query);
        let (prefs, _) = Prefs::resolve(&query, cookies);
        let Some(raw_paragraph_id) = urlcodec::get(&query, "p") else {
            return Response::html(
                400,
                pages::bad_request_page(self, "缺少參數 p：請從閱讀頁的「問 AI」進入。"),
            );
        };
        let paragraph_id = raw_paragraph_id.trim();
        let Some(position) = self.position_of(paragraph_id) else {
            return Response::html(
                404,
                pages::not_found_page(self, "找不到這一段", paragraph_id),
            );
        };
        let Some(context) = self.ask_context(position) else {
            return Response::html(
                404,
                pages::not_found_page(self, "找不到這一段", paragraph_id),
            );
        };
        let config = self.ai();
        let question = urlcodec::get(&query, "q").unwrap_or("").trim().to_string();

        if question.is_empty() || !config.ready() {
            return Response::html(
                200,
                pages::ask_page(
                    self,
                    &prefs,
                    &pages::AskView {
                        context: &context,
                        config,
                        question: &question,
                        prompt: None,
                        outcome: None,
                    },
                ),
            );
        }

        let prompt = zgc_core::build_prompt(&context, &question);
        match ai::chat(config, &prompt.system, &prompt.user) {
            Ok(reply) => Response::html(
                200,
                pages::ask_page(
                    self,
                    &prefs,
                    &pages::AskView {
                        context: &context,
                        config,
                        question: &question,
                        prompt: Some(&prompt),
                        outcome: Some(pages::AskOutcome::Answer(&reply)),
                    },
                ),
            ),
            Err(error) => Response::html(
                502,
                pages::ask_page(
                    self,
                    &prefs,
                    &pages::AskView {
                        context: &context,
                        config,
                        question: &question,
                        prompt: Some(&prompt),
                        outcome: Some(pages::AskOutcome::Failure(&error)),
                    },
                ),
            ),
        }
    }
}

/// Cut to max characters, counting characters rather than bytes.
fn clip(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(max).collect();
    out.push('…');
    out
}
