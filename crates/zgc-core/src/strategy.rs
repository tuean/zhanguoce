//! Strategy (M4): the persuasion structure inside one chapter — who spoke to
//! whom, and which visible rhetorical devices the text actually carries.
//!
//! The on-disk contract is data/zhanguoce/strategy/chapters.json, produced by
//! tools/build_strategy.py from rules alone (Q5=C: 只用實時 API 追問，不做離線
//! 預生成). Every turn and every device therefore has to be checkable against
//! 原文, and this loader rejects anything that is not:
//!
//! 1. a turn's marker must be exactly the characters at offset in its paragraph,
//!    and its text must be a verbatim substring of that paragraph containing the
//!    marker;
//! 2. speaker and speakerId are null together — half a reference is a bug, not a
//!    display problem. addressee may keep the original glyphs (e.g. 楚王) while
//!    addresseeId is null: the words are in the text, the identity is not
//!    established, and the page says 未識別 rather than guessing;
//! 3. every non-null entity id exists, and the glyphs shown are one of that
//!    entity's own names or aliases — a link naming the wrong person is worse
//!    than no link;
//! 4. a device carries matched + offset + len with
//!    text[offset..offset+len] == matched, and its kind is declared in
//!    deviceKinds (an undeclared kind would render with no rationale);
//! 5. devices never overlap inside one paragraph — two links onto the same
//!    glyphs means the generator disagreed with itself;
//! 6. stats agrees with the arrays (stats summarise, they are not a second
//!    source of truth);
//! 7. unresolvedTurns names exactly the turns whose speaker or addressee could
//!    not be established — no more (nothing is silently dropped) and no fewer
//!    (no invented attribution);
//! 8. the file covers every chapter of the book, so a half-written generator
//!    run cannot look complete.
//!
//! Offsets are character offsets (Unicode scalars) into Paragraph::text, the
//! same coordinate system as docs/ENTITIES.md and the entity layer.
//!
//! A missing file is not an error: a book without strategy data keeps
//! rendering, and the chapter page simply shows nothing.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::data::BookData;
use crate::entities::Entities;

/// Strategy table, relative to the book directory.
pub const STRATEGY_FILE: &str = "strategy/chapters.json";

/// Only schemaVersion 1 exists; anything else is a contract we do not know.
const SUPPORTED_SCHEMA_VERSION: u32 = 1;

fn default_schema_version() -> u32 {
    SUPPORTED_SCHEMA_VERSION
}

pub type StrategyResult<T> = Result<T, StrategyError>;

/// One persuasion turn: 「A 謂 B 曰」, recovered from the text itself.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Turn {
    /// Position inside the chapter's turns array, starting at 0.
    pub index: usize,
    /// Paragraph id (the only anchor, per docs/SCHEMA.md).
    pub p: String,
    /// Speaker glyphs as they appear in the text; None when not identifiable.
    #[serde(default)]
    pub speaker: Option<String>,
    #[serde(default)]
    pub speaker_id: Option<String>,
    /// Addressee glyphs (may be present while addressee_id is None).
    #[serde(default)]
    pub addressee: Option<String>,
    #[serde(default)]
    pub addressee_id: Option<String>,
    /// The verb that marked the turn, e.g. 謂.
    pub marker: String,
    /// Character offset of marker inside the paragraph.
    pub offset: usize,
    /// The whole clause, e.g. 范雎謂秦昭王曰.
    pub text: String,
}

impl Turn {
    /// Speaker glyphs, or the page's stand-in for "not established".
    pub fn speaker_label(&self) -> &str {
        self.speaker.as_deref().unwrap_or("未識別")
    }

    /// Addressee glyphs, or the page's stand-in for "not established".
    pub fn addressee_label(&self) -> &str {
        self.addressee.as_deref().unwrap_or("未識別")
    }

    pub fn speaker_resolved(&self) -> bool {
        self.speaker_id.is_some()
    }

    pub fn addressee_resolved(&self) -> bool {
        self.addressee_id.is_some()
    }
}

/// One visible rhetorical device: a span of text that carries it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    /// One of the declared kinds, e.g. 利害威嚇.
    pub kind: String,
    pub p: String,
    pub offset: usize,
    /// The exact glyphs at that span; always equal to the slice.
    pub matched: String,
    /// Length of matched in characters.
    pub len: usize,
}

/// Per-chapter counters. A summary of the arrays, never a second truth.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterStats {
    #[serde(default)]
    pub turns: usize,
    #[serde(default)]
    pub devices: usize,
}

/// Everything the strategy layer says about one chapter.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterStrategy {
    pub chapter_id: String,
    #[serde(default)]
    pub turns: Vec<Turn>,
    #[serde(default)]
    pub devices: Vec<Device>,
    #[serde(default)]
    pub stats: ChapterStats,
}

impl ChapterStrategy {
    pub fn is_empty(&self) -> bool {
        self.turns.is_empty() && self.devices.is_empty()
    }
}

/// One declared device kind and why its word list cannot fire falsely.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceKind {
    pub kind: String,
    #[serde(default)]
    pub rationale: String,
}

/// A field the generator could not establish, and why.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvedTurn {
    pub chapter_id: String,
    pub p: String,
    pub turn_index: usize,
    /// "speaker" or "addressee".
    pub field: String,
    pub reason: String,
}

/// The on-disk shape of data/zhanguoce/strategy/chapters.json.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyFile {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub book_id: String,
    #[serde(default)]
    pub device_kinds: Vec<DeviceKind>,
    #[serde(default)]
    pub chapters: Vec<ChapterStrategy>,
    #[serde(default)]
    pub unresolved_turns: Vec<UnresolvedTurn>,
}

#[derive(Debug)]
pub enum StrategyError {
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
    BookMismatch {
        found: String,
        expected: String,
    },
    NoDeviceKinds,
    DuplicateChapter {
        chapter_id: String,
    },
    UnknownChapter {
        chapter_id: String,
    },
    MissingChapter {
        chapter_id: String,
    },
    EmptyField {
        chapter_id: String,
        field: String,
    },
    UnknownParagraph {
        chapter_id: String,
        p: String,
    },
    BadOffset {
        chapter_id: String,
        p: String,
        offset: usize,
        len: usize,
        text_len: usize,
    },
    SliceMismatch {
        chapter_id: String,
        p: String,
        offset: usize,
        len: usize,
        expected: String,
        found: String,
    },
    MarkerOutsideText {
        chapter_id: String,
        p: String,
    },
    TurnIndexMismatch {
        chapter_id: String,
        found: usize,
        expected: usize,
    },
    TurnsOutOfOrder {
        chapter_id: String,
    },
    DuplicateTurn {
        chapter_id: String,
        p: String,
        offset: usize,
    },
    NullFieldMismatch {
        chapter_id: String,
        p: String,
        field: String,
    },
    UnknownEntity {
        id: String,
    },
    EntityFormMismatch {
        id: String,
        form: String,
    },
    UnknownDeviceKind {
        chapter_id: String,
        kind: String,
    },
    DeviceOverlap {
        chapter_id: String,
        p: String,
    },
    StatsMismatch {
        chapter_id: String,
        field: String,
    },
    UnresolvedOutOfRange {
        chapter_id: String,
        turn_index: usize,
    },
    UnresolvedFieldUnknown {
        chapter_id: String,
        field: String,
    },
    UnresolvedMismatch {
        chapter_id: String,
    },
}

impl fmt::Display for StrategyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StrategyError::Read { path, source } => {
                write!(f, "讀不到謀略表 {}：{source}", path.display())
            }
            StrategyError::Json { path, source } => {
                write!(f, "謀略表 {} 不是合法 JSON：{source}", path.display())
            }
            StrategyError::UnsupportedSchemaVersion { found, supported } => write!(
                f,
                "謀略表的 schemaVersion 是 {found}，本程式只認 {supported}"
            ),
            StrategyError::BookMismatch { found, expected } => {
                write!(f, "謀略表屬於「{found}」，不是「{expected}」")
            }
            StrategyError::NoDeviceKinds => {
                write!(f, "謀略表沒有宣告任何手段類別；標籤就沒有依據可講")
            }
            StrategyError::DuplicateChapter { chapter_id } => {
                write!(f, "章「{chapter_id}」在謀略表裡出現多次")
            }
            StrategyError::UnknownChapter { chapter_id } => {
                write!(f, "謀略表裡的章「{chapter_id}」不存在於正文")
            }
            StrategyError::MissingChapter { chapter_id } => {
                write!(f, "正文的章「{chapter_id}」沒有謀略資料；表被截斷了")
            }
            StrategyError::EmptyField { chapter_id, field } => {
                write!(f, "章「{chapter_id}」的 {field} 是空的")
            }
            StrategyError::UnknownParagraph { chapter_id, p } => {
                write!(f, "章「{chapter_id}」引用了不屬於它的段「{p}」")
            }
            StrategyError::BadOffset { chapter_id, p, offset, len, text_len } => write!(
                f,
                "章「{chapter_id}」段「{p}」的 offset {offset}+{len} 超出原文長度 {text_len}"
            ),
            StrategyError::SliceMismatch { chapter_id, p, offset, len, expected, found } => write!(
                f,
                "章「{chapter_id}」段「{p}」在 {offset},{len} 處應是「{expected}」，原文卻是「{found}」"
            ),
            StrategyError::MarkerOutsideText { chapter_id, p } => write!(
                f,
                "章「{chapter_id}」段「{p}」的 marker 沒有落在它自己的 text 片段內"
            ),
            StrategyError::TurnIndexMismatch { chapter_id, found, expected } => write!(
                f,
                "章「{chapter_id}」的輪次 index 是 {found}，應為 {expected}"
            ),
            StrategyError::TurnsOutOfOrder { chapter_id } => {
                write!(f, "章「{chapter_id}」的輪次沒有依段落與偏移排序")
            }
            StrategyError::DuplicateTurn { chapter_id, p, offset } => {
                write!(f, "章「{chapter_id}」段「{p}」在偏移 {offset} 有兩條輪次")
            }
            StrategyError::NullFieldMismatch { chapter_id, p, field } => write!(
                f,
                "章「{chapter_id}」段「{p}」的 {field} 只能同時為空或同時有名"
            ),
            StrategyError::UnknownEntity { id } => {
                write!(f, "謀略表引用了不存在的實體 {id}")
            }
            StrategyError::EntityFormMismatch { id, form } => write!(
                f,
                "「{form}」不是實體 {id} 的名字或別名；指錯人比不指更糟"
            ),
            StrategyError::UnknownDeviceKind { chapter_id, kind } => {
                write!(f, "章「{chapter_id}」用了沒有宣告的手段類別「{kind}」")
            }
            StrategyError::DeviceOverlap { chapter_id, p } => write!(
                f,
                "章「{chapter_id}」段「{p}」有兩條手段標記重疊；讀者點同一個字會看到兩個說法"
            ),
            StrategyError::StatsMismatch { chapter_id, field } => {
                write!(f, "章「{chapter_id}」的 stats.{field} 與實際條數不符")
            }
            StrategyError::UnresolvedOutOfRange { chapter_id, turn_index } => write!(
                f,
                "章「{chapter_id}」的 unresolvedTurns 指向不存在的輪次 #{turn_index}"
            ),
            StrategyError::UnresolvedFieldUnknown { chapter_id, field } => write!(
                f,
                "章「{chapter_id}」的 unresolvedTurns 欄位「{field}」只能是 speaker 或 addressee"
            ),
            StrategyError::UnresolvedMismatch { chapter_id } => write!(
                f,
                "章「{chapter_id}」的 unresolvedTurns 與未識別的欄位對不上"
            ),
        }
    }
}

impl std::error::Error for StrategyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StrategyError::Read { source, .. } => Some(source),
            StrategyError::Json { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Fixed reason vocabulary → the sentence shown on the page.
pub fn reason_text(reason: &str) -> &'static str {
    match reason {
        "speaker-not-in-entities" => "說話人不在人物表內，未回填",
        "speaker-anonymous" => "原文只說「或」「客」，本無名字",
        "addressee-not-in-entities" => "受話人不在人物表內，未回填",
        "addressee-anonymous" => "原文未給受話人之名",
        _ => "未能確認",
    }
}

/// What the strategy layer knows: per chapter turns and devices.
#[derive(Debug, Clone, Default)]
pub struct Strategy {
    source: String,
    book_id: String,
    kinds: Vec<DeviceKind>,
    chapters: Vec<ChapterStrategy>,
    by_chapter: HashMap<String, usize>,
    unresolved: Vec<UnresolvedTurn>,
    /// Paragraph id → device spans (offset, len), for the read page highlight.
    device_spans: HashMap<String, Vec<(usize, usize)>>,
    turn_total: usize,
    device_total: usize,
}

impl Strategy {
    /// No data at all: every accessor answers empty and the page shows nothing.
    pub fn empty() -> Strategy {
        Strategy::default()
    }

    /// Load strategy/chapters.json. A missing file degrades to empty; a present
    /// but inconsistent file is an error.
    pub fn load(book_dir: &Path, data: &BookData, entities: &Entities) -> StrategyResult<Strategy> {
        let path = book_dir.join(STRATEGY_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Strategy::empty())
            }
            Err(source) => return Err(StrategyError::Read { path, source }),
        };
        let file: StrategyFile =
            serde_json::from_str(&text).map_err(|source| StrategyError::Json {
                path: path.clone(),
                source,
            })?;
        Strategy::build(file, data, entities)
    }

    /// Validate and index an already-parsed file.
    pub fn build(
        file: StrategyFile,
        data: &BookData,
        entities: &Entities,
    ) -> StrategyResult<Strategy> {
        if file.schema_version != SUPPORTED_SCHEMA_VERSION {
            return Err(StrategyError::UnsupportedSchemaVersion {
                found: file.schema_version,
                supported: SUPPORTED_SCHEMA_VERSION,
            });
        }
        if file.book_id != data.book().id {
            return Err(StrategyError::BookMismatch {
                found: file.book_id,
                expected: data.book().id.clone(),
            });
        }
        if file.device_kinds.is_empty() {
            return Err(StrategyError::NoDeviceKinds);
        }

        // Reading order of the book, plus each chapter's paragraphs in order.
        let mut chapter_order: Vec<&str> = Vec::with_capacity(data.chapter_count());
        let mut paragraphs_by_chapter: HashMap<&str, Vec<(&str, &str)>> = HashMap::new();
        for volume in data.volumes() {
            for chapter in &volume.chapters {
                chapter_order.push(chapter.id.as_str());
                paragraphs_by_chapter.insert(
                    chapter.id.as_str(),
                    chapter
                        .paragraphs
                        .iter()
                        .map(|paragraph| (paragraph.id.as_str(), paragraph.text.as_str()))
                        .collect(),
                );
            }
        }

        let declared: HashSet<&str> = file.device_kinds.iter().map(|k| k.kind.as_str()).collect();
        let mut by_chapter: HashMap<String, usize> = HashMap::with_capacity(file.chapters.len());
        let mut device_spans: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
        let mut turn_total = 0usize;
        let mut device_total = 0usize;
        // (chapter, turn index, field) of every field the generator left null.
        let mut null_fields: HashSet<(String, usize, &'static str)> = HashSet::new();

        for (position, chapter) in file.chapters.iter().enumerate() {
            let chapter_id = chapter.chapter_id.as_str();
            if chapter_id.trim().is_empty() {
                return Err(StrategyError::EmptyField {
                    chapter_id: chapter_id.to_string(),
                    field: "chapterId".to_string(),
                });
            }
            let Some(paragraphs) = paragraphs_by_chapter.get(chapter_id) else {
                return Err(StrategyError::UnknownChapter {
                    chapter_id: chapter_id.to_string(),
                });
            };
            if chapter.stats.turns != chapter.turns.len() {
                return Err(StrategyError::StatsMismatch {
                    chapter_id: chapter_id.to_string(),
                    field: "turns".to_string(),
                });
            }
            if chapter.stats.devices != chapter.devices.len() {
                return Err(StrategyError::StatsMismatch {
                    chapter_id: chapter_id.to_string(),
                    field: "devices".to_string(),
                });
            }

            let mut seen_positions: HashSet<(usize, usize)> = HashSet::new();
            let mut previous_key: Option<(usize, usize)> = None;
            for (slot, turn) in chapter.turns.iter().enumerate() {
                if turn.index != slot {
                    return Err(StrategyError::TurnIndexMismatch {
                        chapter_id: chapter_id.to_string(),
                        found: turn.index,
                        expected: slot,
                    });
                }
                let Some(paragraph_index) = paragraphs.iter().position(|(id, _)| *id == turn.p)
                else {
                    return Err(StrategyError::UnknownParagraph {
                        chapter_id: chapter_id.to_string(),
                        p: turn.p.clone(),
                    });
                };
                let (_, text) = paragraphs[paragraph_index];
                let text_len = text.chars().count();
                let marker_len = turn.marker.chars().count();
                if turn.marker.trim().is_empty() {
                    return Err(StrategyError::EmptyField {
                        chapter_id: chapter_id.to_string(),
                        field: "marker".to_string(),
                    });
                }
                if turn.offset + marker_len > text_len {
                    return Err(StrategyError::BadOffset {
                        chapter_id: chapter_id.to_string(),
                        p: turn.p.clone(),
                        offset: turn.offset,
                        len: marker_len,
                        text_len,
                    });
                }
                let found = char_slice(text, turn.offset, marker_len).unwrap_or_default();
                if found != turn.marker {
                    return Err(StrategyError::SliceMismatch {
                        chapter_id: chapter_id.to_string(),
                        p: turn.p.clone(),
                        offset: turn.offset,
                        len: marker_len,
                        expected: turn.marker.clone(),
                        found,
                    });
                }
                if turn.text.trim().is_empty() {
                    return Err(StrategyError::EmptyField {
                        chapter_id: chapter_id.to_string(),
                        field: "text".to_string(),
                    });
                }
                let inside = text.find(&turn.text).is_some_and(|byte_start| {
                    let start = text[..byte_start].chars().count();
                    let span = turn.text.chars().count();
                    start <= turn.offset && turn.offset < start + span
                });
                if !inside {
                    return Err(StrategyError::MarkerOutsideText {
                        chapter_id: chapter_id.to_string(),
                        p: turn.p.clone(),
                    });
                }

                let key = (paragraph_index, turn.offset);
                if !seen_positions.insert(key) {
                    return Err(StrategyError::DuplicateTurn {
                        chapter_id: chapter_id.to_string(),
                        p: turn.p.clone(),
                        offset: turn.offset,
                    });
                }
                if previous_key.is_some_and(|previous| previous > key) {
                    return Err(StrategyError::TurnsOutOfOrder {
                        chapter_id: chapter_id.to_string(),
                    });
                }
                previous_key = Some(key);

                require_pair(
                    chapter_id,
                    &turn.p,
                    "speaker",
                    turn.speaker.is_some(),
                    turn.speaker_id.is_some(),
                )?;
                // addressee may keep its glyphs while the id is unknown, but an
                // id without glyphs is impossible to show.
                if turn.addressee.is_none() && turn.addressee_id.is_some() {
                    return Err(StrategyError::NullFieldMismatch {
                        chapter_id: chapter_id.to_string(),
                        p: turn.p.clone(),
                        field: "addressee".to_string(),
                    });
                }
                if turn
                    .speaker
                    .as_deref()
                    .is_some_and(|value| value.trim().is_empty())
                    || turn
                        .addressee
                        .as_deref()
                        .is_some_and(|value| value.trim().is_empty())
                {
                    return Err(StrategyError::EmptyField {
                        chapter_id: chapter_id.to_string(),
                        field: "speaker/addressee".to_string(),
                    });
                }

                if !entities.is_empty() {
                    check_entity(
                        entities,
                        turn.speaker_id.as_deref(),
                        turn.speaker.as_deref(),
                    )?;
                    check_entity(
                        entities,
                        turn.addressee_id.as_deref(),
                        turn.addressee.as_deref(),
                    )?;
                }

                if turn.speaker.is_none() {
                    null_fields.insert((chapter_id.to_string(), slot, "speaker"));
                }
                if turn.addressee_id.is_none() {
                    null_fields.insert((chapter_id.to_string(), slot, "addressee"));
                }
                turn_total += 1;
            }

            let mut spans_by_paragraph: HashMap<&str, Vec<(usize, usize)>> = HashMap::new();
            for device in &chapter.devices {
                if !declared.contains(device.kind.as_str()) {
                    return Err(StrategyError::UnknownDeviceKind {
                        chapter_id: chapter_id.to_string(),
                        kind: device.kind.clone(),
                    });
                }
                let Some((_, text)) = paragraphs.iter().find(|(id, _)| *id == device.p) else {
                    return Err(StrategyError::UnknownParagraph {
                        chapter_id: chapter_id.to_string(),
                        p: device.p.clone(),
                    });
                };
                let matched_len = device.matched.chars().count();
                if matched_len == 0 || matched_len != device.len {
                    return Err(StrategyError::SliceMismatch {
                        chapter_id: chapter_id.to_string(),
                        p: device.p.clone(),
                        offset: device.offset,
                        len: device.len,
                        expected: device.matched.clone(),
                        found: format!("長度 {matched_len} 的字串"),
                    });
                }
                let text_len = text.chars().count();
                if device.offset + device.len > text_len {
                    return Err(StrategyError::BadOffset {
                        chapter_id: chapter_id.to_string(),
                        p: device.p.clone(),
                        offset: device.offset,
                        len: device.len,
                        text_len,
                    });
                }
                let found = char_slice(text, device.offset, device.len).unwrap_or_default();
                if found != device.matched {
                    return Err(StrategyError::SliceMismatch {
                        chapter_id: chapter_id.to_string(),
                        p: device.p.clone(),
                        offset: device.offset,
                        len: device.len,
                        expected: device.matched.clone(),
                        found,
                    });
                }
                spans_by_paragraph
                    .entry(device.p.as_str())
                    .or_default()
                    .push((device.offset, device.offset + device.len));
                device_total += 1;
            }
            for (p, mut spans) in spans_by_paragraph {
                spans.sort_unstable();
                for pair in spans.windows(2) {
                    if pair[0].1 > pair[1].0 {
                        return Err(StrategyError::DeviceOverlap {
                            chapter_id: chapter_id.to_string(),
                            p: p.to_string(),
                        });
                    }
                }
            }
            for device in &chapter.devices {
                device_spans
                    .entry(device.p.clone())
                    .or_default()
                    .push((device.offset, device.len));
            }

            if by_chapter
                .insert(chapter.chapter_id.clone(), position)
                .is_some()
            {
                return Err(StrategyError::DuplicateChapter {
                    chapter_id: chapter.chapter_id.clone(),
                });
            }
        }

        for chapter_id in &chapter_order {
            if !by_chapter.contains_key(*chapter_id) {
                return Err(StrategyError::MissingChapter {
                    chapter_id: chapter_id.to_string(),
                });
            }
        }

        // unresolvedTurns must name exactly the null fields: no more, no fewer.
        let mut reported: HashSet<(String, usize, &'static str)> = HashSet::new();
        for item in &file.unresolved_turns {
            let field: &'static str = match item.field.as_str() {
                "speaker" => "speaker",
                "addressee" => "addressee",
                other => {
                    return Err(StrategyError::UnresolvedFieldUnknown {
                        chapter_id: item.chapter_id.clone(),
                        field: other.to_string(),
                    })
                }
            };
            if item.reason.trim().is_empty() {
                return Err(StrategyError::EmptyField {
                    chapter_id: item.chapter_id.clone(),
                    field: "reason".to_string(),
                });
            }
            let Some(index) = by_chapter.get(&item.chapter_id) else {
                return Err(StrategyError::UnknownChapter {
                    chapter_id: item.chapter_id.clone(),
                });
            };
            let Some(turn) = file.chapters[*index].turns.get(item.turn_index) else {
                return Err(StrategyError::UnresolvedOutOfRange {
                    chapter_id: item.chapter_id.clone(),
                    turn_index: item.turn_index,
                });
            };
            if turn.p != item.p
                || !reported.insert((item.chapter_id.clone(), item.turn_index, field))
            {
                return Err(StrategyError::UnresolvedMismatch {
                    chapter_id: item.chapter_id.clone(),
                });
            }
        }
        if reported != null_fields {
            let chapter_id = null_fields
                .symmetric_difference(&reported)
                .next()
                .map(|(chapter_id, _, _)| chapter_id.clone())
                .unwrap_or_default();
            return Err(StrategyError::UnresolvedMismatch { chapter_id });
        }

        for spans in device_spans.values_mut() {
            spans.sort_unstable();
        }

        Ok(Strategy {
            source: STRATEGY_FILE.to_string(),
            book_id: file.book_id,
            kinds: file.device_kinds,
            chapters: file.chapters,
            by_chapter,
            unresolved: file.unresolved_turns,
            device_spans,
            turn_total,
            device_total,
        })
    }

    /// One chapter's strategy, when the table covers it.
    pub fn chapter(&self, chapter_id: &str) -> Option<&ChapterStrategy> {
        self.by_chapter
            .get(chapter_id)
            .map(|index| &self.chapters[*index])
    }

    /// Every chapter row, in book order (the page and the tests both want it).
    pub fn chapters(&self) -> &[ChapterStrategy] {
        &self.chapters
    }

    /// Declared kinds with their rationale, in generator order.
    pub fn kinds(&self) -> &[DeviceKind] {
        &self.kinds
    }

    /// Device spans of one paragraph, sorted by offset — the highlight lookup.
    pub fn devices_in(&self, paragraph_id: &str) -> &[(usize, usize)] {
        self.device_spans
            .get(paragraph_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// The unresolved fields recorded for one chapter.
    pub fn unresolved_of<'a>(
        &'a self,
        chapter_id: &'a str,
    ) -> impl Iterator<Item = &'a UnresolvedTurn> + 'a {
        self.unresolved
            .iter()
            .filter(move |item| item.chapter_id == chapter_id)
    }

    pub fn turn_total(&self) -> usize {
        self.turn_total
    }

    pub fn device_total(&self) -> usize {
        self.device_total
    }

    pub fn chapter_count(&self) -> usize {
        self.chapters.len()
    }

    pub fn unresolved_count(&self) -> usize {
        self.unresolved.len()
    }

    /// Chapters that actually carry at least one turn.
    pub fn chapters_with_turns(&self) -> usize {
        self.chapters.iter().filter(|c| !c.turns.is_empty()).count()
    }

    /// Chapters that actually carry at least one device.
    pub fn chapters_with_devices(&self) -> usize {
        self.chapters
            .iter()
            .filter(|c| !c.devices.is_empty())
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.chapters.is_empty()
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn book_id(&self) -> &str {
        &self.book_id
    }
}

/// speaker/speakerId (and any strict pair) are null together.
fn require_pair(
    chapter_id: &str,
    p: &str,
    field: &str,
    has_text: bool,
    has_id: bool,
) -> StrategyResult<()> {
    if has_text == has_id {
        Ok(())
    } else {
        Err(StrategyError::NullFieldMismatch {
            chapter_id: chapter_id.to_string(),
            p: p.to_string(),
            field: field.to_string(),
        })
    }
}

/// A non-null id must exist, and the glyphs shown must be one of its names.
fn check_entity(entities: &Entities, id: Option<&str>, form: Option<&str>) -> StrategyResult<()> {
    let (Some(id), Some(form)) = (id, form) else {
        return Ok(());
    };
    let Some(entity) = entities.entity(id) else {
        return Err(StrategyError::UnknownEntity { id: id.to_string() });
    };
    if entity.name == form || entity.aliases.iter().any(|alias| alias == form) {
        Ok(())
    } else {
        Err(StrategyError::EntityFormMismatch {
            id: id.to_string(),
            form: form.to_string(),
        })
    }
}

/// Characters offset..offset+len of text, or None when the span is short.
fn char_slice(text: &str, offset: usize, len: usize) -> Option<String> {
    let mut chars = text.chars().skip(offset);
    let mut out = String::with_capacity(len);
    for _ in 0..len {
        out.push(chars.next()?);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::BookData;

    /// One book, one volume, one chapter, two paragraphs:
    /// 范雎謂秦昭王曰：臣聞昔者，秦必危矣。 / 秦昭王曰：善。
    fn book() -> BookData {
        let book: crate::types::Book = serde_json::from_str(
            r#"{"schemaVersion":1,"id":"zhanguoce","title":"戰國策",
                "groups":[{"id":"qin","title":"秦","volumes":["qin-3"]}],
                "volumeOrder":["qin-3"]}"#,
        )
        .expect("book");
        let volume: crate::types::Volume = serde_json::from_str(
            r#"{"schemaVersion":1,"id":"qin-3","bookId":"zhanguoce","groupId":"qin",
                "title":"秦三","index":3,"chapters":[
                  {"id":"qin-3-11","title":"范雎至秦","paragraphs":[
                    {"id":"p1","text":"范雎謂秦昭王曰：臣聞昔者，秦必危矣。"},
                    {"id":"p2","text":"秦昭王曰：善。"}]}]}"#,
        )
        .expect("volume");
        BookData::from_parts(book, vec![volume]).expect("data")
    }

    fn people() -> Entities {
        let file: crate::entities::EntityFile = serde_json::from_str(
            r#"{"schemaVersion":1,"bookId":"zhanguoce","kind":"person","entities":[
                {"id":"person-fan-ju","kind":"person","name":"范雎","aliases":["范睢"],
                 "occurrences":[{"p":"p1","o":0,"len":2,"form":"范雎"}]},
                {"id":"person-qin-zhaowang","kind":"person","name":"秦昭王",
                 "occurrences":[{"p":"p1","o":3,"len":3,"form":"秦昭王"}]}]}"#,
        )
        .expect("entities");
        Entities::build(&book(), vec![file]).expect("build entities")
    }

    fn sample() -> StrategyFile {
        serde_json::from_str(
            r#"{"schemaVersion":1,"bookId":"zhanguoce",
                "deviceKinds":[{"kind":"利害威嚇","rationale":"後果斷言"}],
                "chapters":[{"chapterId":"qin-3-11",
                  "turns":[{"index":0,"p":"p1","speaker":"范雎","speakerId":"person-fan-ju",
                            "addressee":"秦昭王","addresseeId":"person-qin-zhaowang",
                            "marker":"謂","offset":2,"text":"范雎謂秦昭王曰"}],
                  "devices":[{"kind":"利害威嚇","p":"p1","offset":14,"matched":"必危","len":2}],
                  "stats":{"turns":1,"devices":1}}],
                "unresolvedTurns":[]}"#,
        )
        .expect("file")
    }

    #[test]
    fn valid_file_builds_and_indexes() {
        let strategy = Strategy::build(sample(), &book(), &people()).expect("build");
        assert_eq!(strategy.turn_total(), 1);
        assert_eq!(strategy.device_total(), 1);
        assert_eq!(strategy.devices_in("p1"), &[(14, 2)]);
        assert_eq!(strategy.devices_in("p2"), &[] as &[(usize, usize)]);
        let chapter = strategy.chapter("qin-3-11").expect("chapter");
        assert_eq!(chapter.turns[0].speaker_label(), "范雎");
        assert!(chapter.turns[0].speaker_resolved());
        assert_eq!(strategy.chapters_with_turns(), 1);
        assert_eq!(strategy.chapters_with_devices(), 1);
    }

    #[test]
    fn a_turn_offset_pointing_at_the_wrong_glyph_is_rejected() {
        let mut file = sample();
        file.chapters[0].turns[0].offset = 3;
        let error = Strategy::build(file, &book(), &people()).expect_err("must fail");
        assert!(
            matches!(error, StrategyError::SliceMismatch { .. }),
            "{error}"
        );
    }

    #[test]
    fn a_device_that_does_not_match_its_paragraph_is_rejected() {
        let mut file = sample();
        file.chapters[0].devices[0].offset = 13;
        let error = Strategy::build(file, &book(), &people()).expect_err("must fail");
        assert!(
            matches!(error, StrategyError::SliceMismatch { .. }),
            "{error}"
        );
    }

    #[test]
    fn a_paragraph_from_another_chapter_is_rejected() {
        let mut file = sample();
        file.chapters[0].turns[0].p = "p9".to_string();
        let error = Strategy::build(file, &book(), &people()).expect_err("must fail");
        assert!(
            matches!(error, StrategyError::UnknownParagraph { .. }),
            "{error}"
        );
    }

    #[test]
    fn an_entity_id_must_exist_and_keep_its_own_name() {
        let mut file = sample();
        file.chapters[0].turns[0].speaker_id = Some("person-nobody".to_string());
        let error = Strategy::build(file, &book(), &people()).expect_err("must fail");
        assert!(
            matches!(error, StrategyError::UnknownEntity { .. }),
            "{error}"
        );

        let mut file = sample();
        file.chapters[0].turns[0].speaker = Some("秦昭王".to_string());
        let error = Strategy::build(file, &book(), &people()).expect_err("must fail");
        assert!(
            matches!(error, StrategyError::EntityFormMismatch { .. }),
            "{error}"
        );
    }

    #[test]
    fn an_undeclared_device_kind_is_rejected() {
        let mut file = sample();
        file.chapters[0].devices[0].kind = "妖言惑眾".to_string();
        let error = Strategy::build(file, &book(), &people()).expect_err("must fail");
        assert!(
            matches!(error, StrategyError::UnknownDeviceKind { .. }),
            "{error}"
        );
    }

    #[test]
    fn stats_must_agree_with_the_arrays() {
        let mut file = sample();
        file.chapters[0].stats.devices = 2;
        let error = Strategy::build(file, &book(), &people()).expect_err("must fail");
        assert!(
            matches!(error, StrategyError::StatsMismatch { .. }),
            "{error}"
        );
    }

    #[test]
    fn an_unresolved_field_must_be_missing_and_reported() {
        // Null speaker that nobody reported.
        let mut value = sample();
        value.chapters[0].turns[0].speaker = None;
        value.chapters[0].turns[0].speaker_id = None;
        let error = Strategy::build(value, &book(), &people()).expect_err("must fail");
        assert!(
            matches!(error, StrategyError::UnresolvedMismatch { .. }),
            "{error}"
        );

        // Reported field that is not null.
        let mut value = sample();
        value.unresolved_turns.push(UnresolvedTurn {
            chapter_id: "qin-3-11".to_string(),
            p: "p1".to_string(),
            turn_index: 0,
            field: "speaker".to_string(),
            reason: "speaker-not-in-entities".to_string(),
        });
        let error = Strategy::build(value, &book(), &people()).expect_err("must fail");
        assert!(
            matches!(error, StrategyError::UnresolvedMismatch { .. }),
            "{error}"
        );
    }

    #[test]
    fn an_addressee_may_keep_its_glyphs_without_an_id() {
        let mut value = sample();
        value.chapters[0].turns[0].addressee = Some("楚王".to_string());
        value.chapters[0].turns[0].addressee_id = None;
        value.unresolved_turns.push(UnresolvedTurn {
            chapter_id: "qin-3-11".to_string(),
            p: "p1".to_string(),
            turn_index: 0,
            field: "addressee".to_string(),
            reason: "addressee-not-in-entities".to_string(),
        });
        let strategy = Strategy::build(value, &book(), &people()).expect("build");
        assert_eq!(
            strategy.chapter("qin-3-11").unwrap().turns[0].addressee_label(),
            "楚王"
        );
        assert!(!strategy.chapter("qin-3-11").unwrap().turns[0].addressee_resolved());
        assert_eq!(strategy.unresolved_count(), 1);
    }

    #[test]
    fn a_missing_chapter_is_reported_instead_of_looking_complete() {
        let mut file = sample();
        file.chapters.clear();
        let error = Strategy::build(file, &book(), &people()).expect_err("must fail");
        assert!(
            matches!(error, StrategyError::MissingChapter { .. }),
            "{error}"
        );
    }

    #[test]
    fn a_missing_file_degrades_to_empty() {
        let dir = std::env::temp_dir().join("zgc-strategy-absent");
        let strategy = Strategy::load(&dir, &book(), &people()).expect("load");
        assert!(strategy.is_empty());
        assert_eq!(strategy.turn_total(), 0);
        assert_eq!(strategy.devices_in("p1"), &[] as &[(usize, usize)]);
    }
}
