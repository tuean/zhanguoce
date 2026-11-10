//! Chapter years (M3c-2): which year a 戰國策 chapter is talking about, and
//! the evidence that says so.
//!
//! The on-disk contract is data/zhanguoce/events/chapter_years.json, produced
//! by tools/build_chapter_years.py. Every decided chapter must carry at least
//! one piece of evidence that points at a real event in the 通鑑 timeline, so
//! the page can answer "憑什麼是這一年" instead of asserting a bare date.
//!
//! Loading is fail-fast on the invariants that would turn a wrong date into a
//! plausible-looking one:
//!
//! 1. every decided chapter has a non-empty evidence list;
//! 2. every eventId exists in the loaded timeline (skipped when the timeline
//!    itself is absent — the book still renders, just without the cross-check);
//! 3. yearBc sits inside its own yearRange and inside the timeline year bounds;
//! 4. yearRange[0] <= yearRange[1];
//! 5. chapter ids are unique across decided and unresolved entries;
//! 6. unresolved entries name a reason;
//! 7. confidence is only high/medium — a "low" confidence is not a date worth
//!    showing, so it is rejected rather than rendered;
//! 8. every evidence carries exactly one shape — a verbatim span (`matched`)
//!    or a name list (`names`). Both at once, or neither, is rejected: a name
//!    list shown as a quotation is the false-quotation bug this guards.
//!
//! A **missing file** is not an error: books without 系年 data keep rendering,
//! and both accessors simply answer None.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::events::Events;

/// Chapter-year table, relative to the book directory.
pub const CHAPTER_YEAR_FILE: &str = "events/chapter_years.json";

/// Only schemaVersion 1 exists; anything else is a contract we do not know.
const SUPPORTED_SCHEMA_VERSION: u32 = 1;

fn default_schema_version() -> u32 {
    SUPPORTED_SCHEMA_VERSION
}

/// How much the evidence is trusted. There is deliberately no Low: a guess
/// that weak belongs in unresolved, where the page can say why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    High,
    Medium,
}

impl Confidence {
    /// Chinese label for the page badge.
    pub fn label(self) -> &'static str {
        match self {
            Confidence::High => "高",
            Confidence::Medium => "中",
        }
    }
}

/// What an evidence entry actually hands the reader: a span taken from a text,
/// or a set of person names. The two are never interchangeable — a span may be
/// shown in quotation marks, a name list may not: joined names are glyphs the
/// chapter does not contain, and quoting them invents evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceText {
    /// Glyphs that occur verbatim in the text its kind names.
    Matched(String),
    /// Person names the chapter and the event share. Not a quotation.
    Names(Vec<String>),
}

/// One pointer from a chapter to a dated event.
#[derive(Debug, Clone)]
pub struct Evidence {
    /// Event id, e.g. tj-y0333-p12. Must exist in the timeline.
    pub event_id: String,
    /// Method that produced it: substring / substring-range / name-overlap /
    /// name-window / commentary-date.
    pub kind: String,
    /// The evidence itself, in one of its two shapes.
    pub text: EvidenceText,
    /// Similarity in 0..=1, when the method produces one.
    pub score: Option<f64>,
}

/// The on-disk shape: `matched` or `names`, exactly one of them.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EvidenceJson {
    event_id: String,
    kind: String,
    #[serde(default)]
    matched: Option<String>,
    #[serde(default)]
    names: Option<Vec<String>>,
    #[serde(default)]
    score: Option<f64>,
}

impl<'de> Deserialize<'de> for Evidence {
    fn deserialize<D>(deserializer: D) -> Result<Evidence, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;

        let raw = EvidenceJson::deserialize(deserializer)?;
        let text = match (raw.matched, raw.names) {
            (Some(matched), None) => {
                if matched.trim().is_empty() {
                    return Err(D::Error::custom(format!(
                        "證據 {} 的 matched 是空白；沒有原文就不要假裝有",
                        raw.event_id
                    )));
                }
                EvidenceText::Matched(matched)
            }
            (None, Some(names)) => {
                if names.is_empty() || names.iter().any(|name| name.trim().is_empty()) {
                    return Err(D::Error::custom(format!(
                        "證據 {} 的 names 必須是逐個的非空姓名",
                        raw.event_id
                    )));
                }
                EvidenceText::Names(names)
            }
            (Some(_), Some(_)) => {
                return Err(D::Error::custom(format!(
                    "證據 {} 同時帶 matched 與 names；一份證據只能是原文片段或共現人物其中之一",
                    raw.event_id
                )))
            }
            (None, None) => {
                return Err(D::Error::custom(format!(
                    "證據 {} 既無 matched 也無 names；空證據不得進表",
                    raw.event_id
                )))
            }
        };
        Ok(Evidence {
            event_id: raw.event_id,
            kind: raw.kind,
            text,
            score: raw.score,
        })
    }
}

/// A chapter whose year is decided.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterYear {
    pub chapter_id: String,
    /// The single year shown on the page, positive = BCE.
    pub year_bc: i32,
    /// Inclusive [earliest, latest] the chapter may span.
    pub year_range: [i32; 2],
    pub confidence: Confidence,
    /// Method that decided it: event-text-match / name-window / commentary-date.
    pub method: String,
    pub evidence: Vec<Evidence>,
    /// Rejection note carried over from review, when the generator kept one.
    #[serde(default)]
    pub reason: Option<String>,
}

/// A chapter that cannot be dated, and the reason why.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvedChapter {
    pub chapter_id: String,
    /// Fixed vocabulary, see reason_text.
    pub reason: String,
    #[serde(default)]
    pub note: Option<String>,
}

/// The JSON file as written by tools/build_chapter_years.py.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterYearFile {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub book_id: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub generated_by: Option<String>,
    #[serde(default)]
    pub chapters: Vec<ChapterYear>,
    #[serde(default)]
    pub unresolved: Vec<UnresolvedChapter>,
}

/// Why a chapter-year table was rejected.
#[derive(Debug)]
pub enum ChapterYearError {
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    UnsupportedSchemaVersion {
        found: u32,
        supported: u32,
    },
    EmptyField {
        chapter_id: String,
        field: &'static str,
    },
    EmptyEvidence {
        chapter_id: String,
    },
    UnknownEvent {
        chapter_id: String,
        event_id: String,
    },
    YearOutOfBounds {
        chapter_id: String,
        year_bc: i32,
        earliest: i32,
        latest: i32,
    },
    BadYearRange {
        chapter_id: String,
        detail: String,
    },
    DuplicateChapter {
        chapter_id: String,
    },
    EmptyReason {
        chapter_id: String,
    },
}

/// Alias kept short at the call site, mirroring EventResult.
pub type ChapterYearResult<T> = Result<T, ChapterYearError>;

impl fmt::Display for ChapterYearError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChapterYearError::Read { path, source } => {
                write!(f, "讀取系年表失敗（{}）：{source}", path.display())
            }
            ChapterYearError::Json { path, source } => {
                write!(f, "系年表 JSON 解析失敗（{}）：{source}", path.display())
            }
            ChapterYearError::UnsupportedSchemaVersion { found, supported } => {
                write!(f, "系年表 schemaVersion={found}，本內核只認 {supported}")
            }
            ChapterYearError::EmptyField { chapter_id, field } => {
                write!(f, "章「{chapter_id}」缺少必填欄位 {field}")
            }
            ChapterYearError::EmptyEvidence { chapter_id } => write!(
                f,
                "章「{chapter_id}」判了年份卻沒有任何證據；判不出就應進 unresolved"
            ),
            ChapterYearError::UnknownEvent {
                chapter_id,
                event_id,
            } => write!(f, "章「{chapter_id}」的證據指向不存在的事件 {event_id}"),
            ChapterYearError::YearOutOfBounds {
                chapter_id,
                year_bc,
                earliest,
                latest,
            } => write!(
                f,
                "章「{chapter_id}」的年份前{year_bc}年不在事件表範圍前{earliest}–前{latest}年內"
            ),
            ChapterYearError::BadYearRange { chapter_id, detail } => {
                write!(f, "章「{chapter_id}」的年區間不合法：{detail}")
            }
            ChapterYearError::DuplicateChapter { chapter_id } => {
                write!(f, "章「{chapter_id}」在系年表裡出現多次")
            }
            ChapterYearError::EmptyReason { chapter_id } => {
                write!(f, "章「{chapter_id}」列在 unresolved 卻沒寫原因")
            }
        }
    }
}

impl std::error::Error for ChapterYearError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ChapterYearError::Read { source, .. } => Some(source),
            ChapterYearError::Json { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Fixed reason vocabulary → the sentence shown on the page.
pub fn reason_text(reason: &str) -> &'static str {
    match reason {
        "no-event-before-453" => "通鑑起於前403年，此前無事可繫",
        "no-tongjian-match" => "通鑑沒有相近的記載",
        "below-threshold" => "僅有零星字句相合，不足以為據",
        "formula-match" => "只對上策文套語，非本事",
        "ambiguous-multi-year" => "可繫於數年，無法取捨",
        "tongjian-bucket-year" => "通鑑將此事併入前403年一條，非其真年",
        "commentary-conflict" => "注疏所記之年與正文不合",
        "manual-review-reject" => "复核時認定證據不成立",
        _ => "無足夠證據",
    }
}

/// The chapter-year index: decided chapters plus the reason each other chapter
/// could not be dated.
#[derive(Debug, Clone, Default)]
pub struct ChapterYears {
    source: String,
    chapters: Vec<ChapterYear>,
    by_chapter: HashMap<String, usize>,
    unresolved: HashMap<String, String>,
    bounds: Option<(i32, i32)>,
}

impl ChapterYears {
    /// No data at all: every accessor answers None and the page shows nothing.
    pub fn empty() -> ChapterYears {
        ChapterYears::default()
    }

    /// Load events/chapter_years.json. A missing file degrades to empty; a
    /// present but inconsistent file is an error.
    pub fn load(book_dir: &Path, events: &Events) -> ChapterYearResult<ChapterYears> {
        let path = book_dir.join(CHAPTER_YEAR_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ChapterYears::empty())
            }
            Err(source) => return Err(ChapterYearError::Read { path, source }),
        };
        let file: ChapterYearFile =
            serde_json::from_str(&text).map_err(|source| ChapterYearError::Json {
                path: path.clone(),
                source,
            })?;
        ChapterYears::build(file, events)
    }

    /// Validate and index an already-parsed file.
    pub fn build(file: ChapterYearFile, events: &Events) -> ChapterYearResult<ChapterYears> {
        if file.schema_version != SUPPORTED_SCHEMA_VERSION {
            return Err(ChapterYearError::UnsupportedSchemaVersion {
                found: file.schema_version,
                supported: SUPPORTED_SCHEMA_VERSION,
            });
        }
        let bounds = events.year_bounds();
        let mut by_chapter: HashMap<String, usize> = HashMap::with_capacity(file.chapters.len());
        for (index, chapter) in file.chapters.iter().enumerate() {
            if chapter.chapter_id.trim().is_empty() {
                return Err(ChapterYearError::EmptyField {
                    chapter_id: chapter.chapter_id.clone(),
                    field: "chapterId",
                });
            }
            if chapter.evidence.is_empty() {
                return Err(ChapterYearError::EmptyEvidence {
                    chapter_id: chapter.chapter_id.clone(),
                });
            }
            if chapter.method.trim().is_empty() {
                return Err(ChapterYearError::EmptyField {
                    chapter_id: chapter.chapter_id.clone(),
                    field: "method",
                });
            }
            let [start, end] = chapter.year_range;
            if start > end || chapter.year_bc < start || chapter.year_bc > end {
                return Err(ChapterYearError::BadYearRange {
                    chapter_id: chapter.chapter_id.clone(),
                    detail: format!("yearRange=[{start},{end}] 不含 yearBc={}", chapter.year_bc),
                });
            }
            if let Some((earliest, latest)) = bounds {
                // BCE numbers invert: `earliest` (前403) is numerically larger than `latest` (前209).
                if chapter.year_bc > earliest || chapter.year_bc < latest {
                    return Err(ChapterYearError::YearOutOfBounds {
                        chapter_id: chapter.chapter_id.clone(),
                        year_bc: chapter.year_bc,
                        earliest,
                        latest,
                    });
                }
            }
            for evidence in &chapter.evidence {
                if evidence.event_id.trim().is_empty() {
                    return Err(ChapterYearError::EmptyField {
                        chapter_id: chapter.chapter_id.clone(),
                        field: "eventId",
                    });
                }
                if !events.is_empty() && events.event(&evidence.event_id).is_none() {
                    return Err(ChapterYearError::UnknownEvent {
                        chapter_id: chapter.chapter_id.clone(),
                        event_id: evidence.event_id.clone(),
                    });
                }
            }
            if by_chapter
                .insert(chapter.chapter_id.clone(), index)
                .is_some()
            {
                return Err(ChapterYearError::DuplicateChapter {
                    chapter_id: chapter.chapter_id.clone(),
                });
            }
        }

        let mut unresolved = HashMap::with_capacity(file.unresolved.len());
        for chapter in &file.unresolved {
            if chapter.chapter_id.trim().is_empty() {
                return Err(ChapterYearError::EmptyField {
                    chapter_id: chapter.chapter_id.clone(),
                    field: "chapterId",
                });
            }
            if chapter.reason.trim().is_empty() {
                return Err(ChapterYearError::EmptyReason {
                    chapter_id: chapter.chapter_id.clone(),
                });
            }
            if by_chapter.contains_key(&chapter.chapter_id)
                || unresolved
                    .insert(chapter.chapter_id.clone(), chapter.reason.clone())
                    .is_some()
            {
                return Err(ChapterYearError::DuplicateChapter {
                    chapter_id: chapter.chapter_id.clone(),
                });
            }
        }

        Ok(ChapterYears {
            source: file.source,
            chapters: file.chapters,
            by_chapter,
            unresolved,
            bounds,
        })
    }

    /// The decided year for a chapter, when there is one.
    pub fn get(&self, chapter_id: &str) -> Option<&ChapterYear> {
        self.by_chapter
            .get(chapter_id)
            .map(|index| &self.chapters[*index])
    }

    /// Why a chapter could not be dated, when the generator recorded it.
    pub fn unresolved_reason(&self, chapter_id: &str) -> Option<&str> {
        self.unresolved.get(chapter_id).map(String::as_str)
    }

    /// Decided chapters, in file order.
    pub fn chapters(&self) -> &[ChapterYear] {
        &self.chapters
    }

    pub fn resolved_count(&self) -> usize {
        self.chapters.len()
    }

    pub fn unresolved_count(&self) -> usize {
        self.unresolved.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chapters.is_empty() && self.unresolved.is_empty()
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// Year bounds inherited from the timeline, when it is loaded.
    pub fn year_bounds(&self) -> Option<(i32, i32)> {
        self.bounds
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{Event, EventFile, EventYear};

    /// A one-year, one-event timeline: 前333 年 顯王三十六年.
    fn timeline() -> Events {
        let year = EventYear {
            year_bc: 333,
            div: "y0333".to_string(),
            label: "（戊子、前三三三年）".to_string(),
            ganzhi: "戊子".to_string(),
            ruler: "顯王".to_string(),
            ruler_year: "三十六年".to_string(),
            juan: "周紀二".to_string(),
            source_file: "5282".to_string(),
            anchor: "yt1".to_string(),
            note: None,
        };
        let event = Event {
            id: "tj-y0333-p1".to_string(),
            year_bc: 333,
            ruler: "顯王".to_string(),
            ruler_year: "三十六年".to_string(),
            ganzhi: "戊子".to_string(),
            juan: "周紀二".to_string(),
            source_file: "5282".to_string(),
            anchor: "p1".to_string(),
            text: "秦敗趙師于彫陰".to_string(),
            note: None,
            collation: None,
        };
        Events::build(EventFile {
            schema_version: 1,
            book_id: "zhanguoce".to_string(),
            source: "test".to_string(),
            generated_by: None,
            years: vec![year],
            events: vec![event],
        })
        .expect("fixture timeline must be valid")
    }

    fn file(json: &str) -> ChapterYearFile {
        serde_json::from_str(json).expect("fixture json must parse")
    }

    const GOOD: &str = r#"{
        "schemaVersion": 1,
        "source": "test",
        "chapters": [{
            "chapterId": "qin-1-02",
            "yearBc": 333,
            "yearRange": [333, 335],
            "confidence": "high",
            "method": "event-text-match",
            "evidence": [{
                "eventId": "tj-y0333-p1",
                "kind": "substring",
                "matched": "秦敗趙師",
                "score": 0.86
            }],
            "reason": null
        }],
        "unresolved": [{ "chapterId": "dongzhou-01", "reason": "no-event-before-453" }]
    }"#;

    #[test]
    fn builds_and_answers_by_chapter() {
        let years = ChapterYears::build(file(GOOD), &timeline()).expect("valid table");
        assert_eq!(years.resolved_count(), 1);
        assert_eq!(years.unresolved_count(), 1);
        assert!(!years.is_empty());
        let chapter = years.get("qin-1-02").expect("decided chapter");
        assert_eq!(chapter.year_bc, 333);
        assert_eq!(chapter.year_range, [333, 335]);
        assert_eq!(chapter.confidence, Confidence::High);
        assert_eq!(chapter.evidence[0].event_id, "tj-y0333-p1");
        assert_eq!(
            years.unresolved_reason("dongzhou-01"),
            Some("no-event-before-453")
        );
        assert_eq!(years.unresolved_reason("qin-1-02"), None);
        assert!(years.get("nope").is_none());
        assert_eq!(years.year_bounds(), Some((333, 333)));
    }

    #[test]
    fn rejects_a_date_without_evidence() {
        let json = GOOD.replace(
            r#""evidence": [{
                "eventId": "tj-y0333-p1",
                "kind": "substring",
                "matched": "秦敗趙師",
                "score": 0.86
            }]"#,
            r#""evidence": []"#,
        );
        match ChapterYears::build(file(&json), &timeline()) {
            Err(ChapterYearError::EmptyEvidence { chapter_id }) => {
                assert_eq!(chapter_id, "qin-1-02")
            }
            other => panic!("expected EmptyEvidence, got {other:?}"),
        }
    }

    #[test]
    fn rejects_evidence_pointing_at_a_missing_event() {
        let json = GOOD.replace("tj-y0333-p1", "tj-y0333-p99");
        match ChapterYears::build(file(&json), &timeline()) {
            Err(ChapterYearError::UnknownEvent {
                chapter_id,
                event_id,
            }) => {
                assert_eq!(chapter_id, "qin-1-02");
                assert_eq!(event_id, "tj-y0333-p99");
            }
            other => panic!("expected UnknownEvent, got {other:?}"),
        }
    }

    /// The names shape: a name-overlap, which must never be quoted as chapter text.
    const NAMES: &str = r#"{
        "schemaVersion": 1,
        "source": "test",
        "chapters": [{
            "chapterId": "chu-1-08",
            "yearBc": 333,
            "yearRange": [333, 333],
            "confidence": "medium",
            "method": "event-text-match",
            "evidence": [{
                "eventId": "tj-y0333-p1",
                "kind": "name-overlap",
                "names": ["昭奚恤", "江乙"],
                "score": 0.5
            }],
            "reason": null
        }],
        "unresolved": []
    }"#;

    #[test]
    fn keeps_a_name_list_as_a_list() {
        let years = ChapterYears::build(file(NAMES), &timeline()).expect("names evidence is valid");
        let evidence = &years.get("chu-1-08").expect("decided").evidence[0];
        assert_eq!(evidence.kind, "name-overlap");
        assert_eq!(
            evidence.text,
            EvidenceText::Names(vec!["昭奚恤".to_string(), "江乙".to_string()])
        );
    }

    #[test]
    fn rejects_evidence_with_neither_matched_nor_names() {
        let json = NAMES.replace(r#""names": ["昭奚恤", "江乙"],"#, "");
        let error = serde_json::from_str::<ChapterYearFile>(&json)
            .expect_err("an evidence with neither shape must be rejected");
        assert!(
            error.to_string().contains("既無 matched 也無 names"),
            "{error}"
        );
    }

    #[test]
    fn rejects_evidence_with_both_matched_and_names() {
        let json = NAMES.replace(
            r#""names": ["昭奚恤", "江乙"],"#,
            r#""matched": "昭奚恤江乙", "names": ["昭奚恤", "江乙"],"#,
        );
        let error = serde_json::from_str::<ChapterYearFile>(&json)
            .expect_err("an evidence with both shapes must be rejected");
        assert!(
            error.to_string().contains("同時帶 matched 與 names"),
            "{error}"
        );
    }

    #[test]
    fn rejects_an_empty_name_list() {
        let json = NAMES.replace(r#"["昭奚恤", "江乙"]"#, "[]");
        let error = serde_json::from_str::<ChapterYearFile>(&json)
            .expect_err("a name list with no names must be rejected");
        assert!(error.to_string().contains("非空姓名"), "{error}");
    }

    #[test]
    fn rejects_low_confidence_at_parse_time() {
        let json = GOOD.replace(r#""confidence": "high""#, r#""confidence": "low""#);
        assert!(serde_json::from_str::<ChapterYearFile>(&json).is_err());
    }

    #[test]
    fn rejects_year_outside_the_timeline() {
        let json = GOOD
            .replace(r#""yearBc": 333"#, r#""yearBc": 400"#)
            .replace(r#""yearRange": [333, 335]"#, r#""yearRange": [400, 400]"#);
        match ChapterYears::build(file(&json), &timeline()) {
            Err(ChapterYearError::YearOutOfBounds {
                chapter_id,
                year_bc,
                ..
            }) => {
                assert_eq!(chapter_id, "qin-1-02");
                assert_eq!(year_bc, 400);
            }
            other => panic!("expected YearOutOfBounds, got {other:?}"),
        }
    }

    /// A timeline spanning the whole run, 前403–前209, one event at each end.
    fn span_timeline() -> Events {
        let year = |year_bc: i32, div: &str, label: &str| EventYear {
            year_bc,
            div: div.to_string(),
            label: label.to_string(),
            ganzhi: "甲子".to_string(),
            ruler: "威烈王".to_string(),
            ruler_year: "二十三年".to_string(),
            juan: "周紀一".to_string(),
            source_file: "5282".to_string(),
            anchor: "yt1".to_string(),
            note: None,
        };
        let event = |id: &str, year_bc: i32| Event {
            id: id.to_string(),
            year_bc,
            ruler: "威烈王".to_string(),
            ruler_year: "二十三年".to_string(),
            ganzhi: "甲子".to_string(),
            juan: "周紀一".to_string(),
            source_file: "5282".to_string(),
            anchor: "p1".to_string(),
            text: "初命晉大夫魏斯、趙籍、韓虔爲諸侯".to_string(),
            note: None,
            collation: None,
        };
        Events::build(EventFile {
            schema_version: 1,
            book_id: "zhanguoce".to_string(),
            source: "test".to_string(),
            generated_by: None,
            years: vec![
                year(403, "y0403", "（甲子、前四〇三年）"),
                year(209, "y0209", "（甲子、前二〇九年）"),
            ],
            events: vec![event("tj-y0403-p1", 403), event("tj-y0209-p1", 209)],
        })
        .expect("span fixture timeline must be valid")
    }

    #[test]
    fn accepts_a_year_inside_the_span_that_carries_no_event() {
        // BCE numbers invert: 前353 sits *between* 前403 (earliest) and 前209 (latest).
        let json = GOOD
            .replace(r#""yearBc": 333"#, r#""yearBc": 353"#)
            .replace(r#""yearRange": [333, 335]"#, r#""yearRange": [353, 353]"#)
            .replace("tj-y0333-p1", "tj-y0403-p1");
        let years = ChapterYears::build(file(&json), &span_timeline()).expect("前353 在區間內");
        assert_eq!(years.get("qin-1-02").expect("decided").year_bc, 353);
        assert_eq!(years.year_bounds(), Some((403, 209)));
    }

    #[test]
    fn rejects_a_year_older_than_the_timeline() {
        let json = GOOD
            .replace(r#""yearBc": 333"#, r#""yearBc": 404"#)
            .replace(r#""yearRange": [333, 335]"#, r#""yearRange": [404, 404]"#);
        match ChapterYears::build(file(&json), &span_timeline()) {
            Err(ChapterYearError::YearOutOfBounds { year_bc, .. }) => assert_eq!(year_bc, 404),
            other => panic!("expected YearOutOfBounds for 前404, got {other:?}"),
        }
    }

    #[test]
    fn rejects_a_range_that_excludes_the_shown_year() {
        let json = GOOD.replace(r#""yearRange": [333, 335]"#, r#""yearRange": [320, 321]"#);
        match ChapterYears::build(file(&json), &timeline()) {
            Err(ChapterYearError::BadYearRange { .. }) => {}
            other => panic!("expected BadYearRange, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unresolved_without_a_reason() {
        let json = GOOD.replace(r#""reason": "no-event-before-453""#, r#""reason": """#);
        match ChapterYears::build(file(&json), &timeline()) {
            Err(ChapterYearError::EmptyReason { chapter_id }) => {
                assert_eq!(chapter_id, "dongzhou-01")
            }
            other => panic!("expected EmptyReason, got {other:?}"),
        }
    }

    #[test]
    fn rejects_a_chapter_listed_twice() {
        let json = GOOD.replace(
            r#""unresolved": [{ "chapterId": "dongzhou-01", "reason": "no-event-before-453" }]"#,
            r#""unresolved": [{ "chapterId": "qin-1-02", "reason": "no-tongjian-match" }]"#,
        );
        match ChapterYears::build(file(&json), &timeline()) {
            Err(ChapterYearError::DuplicateChapter { chapter_id }) => {
                assert_eq!(chapter_id, "qin-1-02")
            }
            other => panic!("expected DuplicateChapter, got {other:?}"),
        }
    }

    #[test]
    fn a_missing_file_degrades_to_empty() {
        let years = ChapterYears::load(Path::new("/nonexistent-zgc-book"), &Events::empty())
            .expect("missing file is not an error");
        assert!(years.is_empty());
        assert!(years.get("qin-1-02").is_none());
        assert_eq!(years.unresolved_reason("dongzhou-01"), None);
        assert_eq!(years.year_bounds(), None);
    }

    #[test]
    fn an_empty_timeline_skips_event_existence_checking() {
        let years =
            ChapterYears::build(file(GOOD), &Events::empty()).expect("shape is still valid");
        assert_eq!(years.resolved_count(), 1);
    }

    #[test]
    fn reason_text_covers_the_fixed_vocabulary() {
        for reason in [
            "no-event-before-453",
            "no-tongjian-match",
            "below-threshold",
            "formula-match",
            "ambiguous-multi-year",
            "tongjian-bucket-year",
            "commentary-conflict",
            "manual-review-reject",
        ] {
            assert!(!reason_text(reason).is_empty(), "no text for {reason}");
        }
        assert_eq!(reason_text("something-new"), "無足夠證據");
    }
}
