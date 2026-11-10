//! Entity layer (M3): curated people/places mounted on exact paragraph offsets.
//!
//! The on-disk contract is docs/ENTITIES.md: every occurrence is
//! { p, o, len, form }, where o is a **character** offset (Unicode scalar
//! values, never bytes) into the traditional-script paragraph text. Loading is
//! fail-fast: a form that does not slice back out of the paragraph, an unknown
//! paragraph id, a duplicate entity id or an undeclared form is a hard error —
//! a silently mis-mounted entity is worse than a missing one.
//!
//! Rendering is deliberately not here. This module builds one index at load
//! time (paragraph id → occurrences) and answers queries from it, so a request
//! never rescans the ten-thousand-odd occurrence records.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::data::BookData;

/// Person entity file, relative to the book directory.
pub const PERSON_FILE: &str = "entities/people.json";
/// Place entity file, relative to the book directory.
pub const PLACE_FILE: &str = "entities/places.json";

/// What an entity is. Kept small and copyable: it drives both the file it is
/// loaded from and the CSS class the reader uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntityKind {
    Person,
    Place,
}

impl EntityKind {
    /// Wire/file spelling: person / place.
    pub const fn as_str(self) -> &'static str {
        match self {
            EntityKind::Person => "person",
            EntityKind::Place => "place",
        }
    }

    /// Chinese label used by the reader.
    pub const fn label(self) -> &'static str {
        match self {
            EntityKind::Person => "人物",
            EntityKind::Place => "地名",
        }
    }

    /// Parse the ?kind= query values accepted by the index page.
    pub fn parse(value: &str) -> Option<EntityKind> {
        match value.trim() {
            "person" | "人物" => Some(EntityKind::Person),
            "place" | "地名" => Some(EntityKind::Place),
            _ => None,
        }
    }
}

impl fmt::Display for EntityKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

fn default_schema_version() -> u32 {
    1
}

/// One validated occurrence of an entity in the corpus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    /// Paragraph id (the only anchor, per docs/SCHEMA.md).
    pub paragraph_id: String,
    /// Character offset into Paragraph::text (0-based, Unicode scalars).
    pub offset: usize,
    /// Matched length in characters.
    pub len: usize,
    /// The exact glyphs matched; always the entity's name or one alias.
    pub form: String,
}

/// A curated entity plus every place it occurs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entity {
    pub id: String,
    pub kind: EntityKind,
    /// Traditional head form, in the base text's own characters.
    pub name: String,
    pub name_simp: Option<String>,
    pub aliases: Vec<String>,
    pub tags: Vec<String>,
    /// One sentence; None means "not established" and must never be invented.
    pub summary: Option<String>,
    /// Sorted by reading order (volume → chapter → paragraph → offset).
    pub occurrences: Vec<Occurrence>,
}

impl Entity {
    /// Number of mounted occurrences.
    pub fn occurrence_count(&self) -> usize {
        self.occurrences.len()
    }

    /// Trimmed summary, or None when absent/blank.
    pub fn summary_text(&self) -> Option<&str> {
        self.summary
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
    }

    /// Trimmed simplified name, or None when absent/blank/equal to name.
    pub fn name_simp_text(&self) -> Option<&str> {
        self.name_simp
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty() && *text != self.name)
    }
}

/// One span ready for rendering: the entity, its character offset and length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpanView<'a> {
    pub entity: &'a Entity,
    pub offset: usize,
    pub len: usize,
}

/// Iterator over the spans of one paragraph. Empty for an unknown paragraph.
pub struct Spans<'a> {
    entities: &'a [Entity],
    spans: std::slice::Iter<'a, StoredSpan>,
}

impl<'a> Iterator for Spans<'a> {
    type Item = SpanView<'a>;

    fn next(&mut self) -> Option<SpanView<'a>> {
        let span = self.spans.next()?;
        let entity = self.entities.get(span.entity as usize)?;
        Some(SpanView {
            entity,
            offset: span.offset as usize,
            len: span.len as usize,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.spans.size_hint()
    }
}

const NO_SPANS: &[StoredSpan] = &[];

/// Index stable across requests: entities by id and spans by paragraph id.
#[derive(Debug, Clone, Default)]
pub struct Entities {
    entities: Vec<Entity>,
    by_id: HashMap<String, u32>,
    raw: HashMap<String, Vec<StoredSpan>>,
    resolved: HashMap<String, Vec<StoredSpan>>,
    covered_paragraphs: usize,
    overlapping_paragraphs: usize,
    occurrence_total: usize,
}

impl Entities {
    /// An empty index — used by tests and by callers that run without entities.
    pub fn empty() -> Entities {
        Entities::default()
    }

    /// Load entities/people.json and entities/places.json under book_dir.
    ///
    /// data supplies the paragraph texts the occurrences are verified against.
    pub fn load(book_dir: &Path, data: &BookData) -> EntityResult<Entities> {
        let person_path = book_dir.join(PERSON_FILE);
        let place_path = book_dir.join(PLACE_FILE);
        let people: EntityFile = read_json(&person_path)?;
        let places: EntityFile = read_json(&place_path)?;

        if people.kind != EntityKind::Person {
            return Err(EntityError::FileKindMismatch {
                path: person_path,
                expected: EntityKind::Person,
                actual: people.kind,
            });
        }
        if places.kind != EntityKind::Place {
            return Err(EntityError::FileKindMismatch {
                path: place_path,
                expected: EntityKind::Place,
                actual: places.kind,
            });
        }
        Entities::build(data, vec![people, places])
    }

    /// Validate arbitrary in-memory bundles and build the index.
    ///
    /// Book::load and programmatic construction share this one validator, so
    /// tests exercise exactly the code the server runs.
    pub fn build(data: &BookData, files: Vec<EntityFile>) -> EntityResult<Entities> {
        // Paragraph id → (text, flat reading index). Built once; the whole
        // validation below walks it instead of the tree.
        let mut texts: HashMap<&str, &str> = HashMap::with_capacity(data.paragraph_count());
        let mut flat: HashMap<&str, usize> = HashMap::with_capacity(data.paragraph_count());
        for (index, (_, paragraph)) in data.iter_paragraphs().enumerate() {
            texts.insert(paragraph.id.as_str(), paragraph.text.as_str());
            flat.insert(paragraph.id.as_str(), index);
        }

        let mut entities: Vec<Entity> = Vec::new();
        let mut by_id: HashMap<String, u32> = HashMap::new();

        for file in &files {
            for record in &file.entities {
                if record.id.trim().is_empty() {
                    return Err(EntityError::EmptyId);
                }
                if record.kind != file.kind {
                    return Err(EntityError::KindMismatch {
                        id: record.id.clone(),
                        file_kind: file.kind,
                        entity_kind: record.kind,
                    });
                }
                if record.name.trim().is_empty() {
                    return Err(EntityError::EmptyName {
                        id: record.id.clone(),
                    });
                }
                if record.occurrences.is_empty() {
                    return Err(EntityError::NoOccurrences {
                        id: record.id.clone(),
                    });
                }
                if by_id.contains_key(&record.id) {
                    return Err(EntityError::DuplicateId {
                        id: record.id.clone(),
                    });
                }

                let mut occurrences = Vec::with_capacity(record.occurrences.len());
                for found in &record.occurrences {
                    let Some(text) = texts.get(found.p.as_str()).copied() else {
                        return Err(EntityError::UnknownParagraph {
                            id: record.id.clone(),
                            paragraph_id: found.p.clone(),
                        });
                    };
                    if found.len == 0 {
                        return Err(EntityError::EmptyMatch {
                            id: record.id.clone(),
                            paragraph_id: found.p.clone(),
                            offset: found.o,
                        });
                    }
                    if found.form != record.name && !record.aliases.iter().any(|a| a == &found.form)
                    {
                        return Err(EntityError::FormNotDeclared {
                            id: record.id.clone(),
                            form: found.form.clone(),
                        });
                    }
                    let char_count = text.chars().count();
                    let Some(actual) = char_slice(text, found.o, found.len) else {
                        return Err(EntityError::OffsetOutOfRange {
                            id: record.id.clone(),
                            paragraph_id: found.p.clone(),
                            offset: found.o,
                            len: found.len,
                            char_count,
                        });
                    };
                    if actual != found.form {
                        return Err(EntityError::FormMismatch {
                            id: record.id.clone(),
                            paragraph_id: found.p.clone(),
                            offset: found.o,
                            len: found.len,
                            form: found.form.clone(),
                            actual: actual.to_string(),
                        });
                    }
                    occurrences.push(Occurrence {
                        paragraph_id: found.p.clone(),
                        offset: found.o,
                        len: found.len,
                        form: found.form.clone(),
                    });
                }

                // Reading order: volume → chapter → paragraph → offset. The
                // detail page relies on this to group without re-sorting.
                occurrences.sort_by(|a, b| {
                    let left = flat
                        .get(a.paragraph_id.as_str())
                        .copied()
                        .unwrap_or(usize::MAX);
                    let right = flat
                        .get(b.paragraph_id.as_str())
                        .copied()
                        .unwrap_or(usize::MAX);
                    left.cmp(&right)
                        .then(a.offset.cmp(&b.offset))
                        .then(a.len.cmp(&b.len))
                });

                let index = entities.len() as u32;
                by_id.insert(record.id.clone(), index);
                entities.push(Entity {
                    id: record.id.clone(),
                    kind: record.kind,
                    name: record.name.clone(),
                    name_simp: record.name_simp.clone(),
                    aliases: record.aliases.clone(),
                    tags: record.tags.clone(),
                    summary: record.summary.clone(),
                    occurrences,
                });
            }
        }

        let mut raw: HashMap<String, Vec<StoredSpan>> = HashMap::new();
        let mut occurrence_total = 0usize;
        for (index, entity) in entities.iter().enumerate() {
            for occurrence in &entity.occurrences {
                occurrence_total += 1;
                raw.entry(occurrence.paragraph_id.clone())
                    .or_default()
                    .push(StoredSpan {
                        entity: index as u32,
                        offset: occurrence.offset as u32,
                        len: occurrence.len as u32,
                    });
            }
        }

        let mut resolved: HashMap<String, Vec<StoredSpan>> = HashMap::with_capacity(raw.len());
        let mut overlapping_paragraphs = 0usize;
        for (paragraph_id, spans) in &raw {
            let kept = resolve(spans);
            if kept.len() < spans.len() {
                overlapping_paragraphs += 1;
            }
            resolved.insert(paragraph_id.clone(), kept);
        }

        let covered_paragraphs = resolved.len();
        Ok(Entities {
            entities,
            by_id,
            raw,
            resolved,
            covered_paragraphs,
            overlapping_paragraphs,
            occurrence_total,
        })
    }

    /// Number of entities (people + places).
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    pub fn entity(&self, id: &str) -> Option<&Entity> {
        self.by_id
            .get(id)
            .map(|index| &self.entities[*index as usize])
    }

    /// Every entity, person file first then place file, in file order.
    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        self.entities.iter()
    }

    /// Entities of one kind, in file order.
    pub fn of_kind(&self, kind: EntityKind) -> impl Iterator<Item = &Entity> {
        self.entities
            .iter()
            .filter(move |entity| entity.kind == kind)
    }

    /// Total mounted occurrences across all entities.
    pub fn occurrence_total(&self) -> usize {
        self.occurrence_total
    }

    /// Paragraphs that carry at least one entity.
    pub fn covered_paragraph_count(&self) -> usize {
        self.covered_paragraphs
    }

    /// Paragraphs where raw occurrences overlap and the determinism rule had to
    /// drop at least one span.
    pub fn overlapping_paragraph_count(&self) -> usize {
        self.overlapping_paragraphs
    }

    /// Raw occurrences of one paragraph — every record as loaded, overlaps
    /// included. Empty for unknown paragraphs or paragraphs without entities.
    pub fn occurrences_in(&self, paragraph_id: &str) -> Spans<'_> {
        self.spans(&self.raw, paragraph_id)
    }

    /// Renderable spans of one paragraph: overlapping occurrences removed by
    /// the deterministic rule in Entities::build (longest first, ties
    /// leftmost, shorter/overlapped spans dropped), sorted by offset.
    pub fn spans_in(&self, paragraph_id: &str) -> Spans<'_> {
        self.spans(&self.resolved, paragraph_id)
    }

    fn spans<'a>(&'a self, map: &'a HashMap<String, Vec<StoredSpan>>, id: &str) -> Spans<'a> {
        Spans {
            entities: &self.entities,
            spans: map.get(id).map(Vec::as_slice).unwrap_or(NO_SPANS).iter(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StoredSpan {
    entity: u32,
    offset: u32,
    len: u32,
}

/// Deterministic overlap resolution for one paragraph.
///
/// Order: longer spans first, then the earlier start, then entity index — a
/// total order, so the outcome never depends on hash iteration. A span that
/// collides with an already accepted one is dropped, which is what keeps
/// anchor/mark elements from nesting and is stable across runs.
fn resolve(spans: &[StoredSpan]) -> Vec<StoredSpan> {
    let mut order: Vec<StoredSpan> = spans.to_vec();
    order.sort_by(|a, b| {
        b.len
            .cmp(&a.len)
            .then(a.offset.cmp(&b.offset))
            .then(a.entity.cmp(&b.entity))
    });
    let mut kept: Vec<StoredSpan> = Vec::with_capacity(order.len());
    for span in order {
        let end = span.offset as usize + span.len as usize;
        let clashes = kept.iter().any(|other| {
            let other_end = other.offset as usize + other.len as usize;
            (span.offset as usize) < other_end && (other.offset as usize) < end
        });
        if !clashes {
            kept.push(span);
        }
    }
    kept.sort_by_key(|span| (span.offset, span.len));
    kept
}

/// Character slice text[offset .. offset + len], None when out of range.
fn char_slice(text: &str, offset: usize, len: usize) -> Option<&str> {
    if len == 0 {
        return None;
    }
    let start = text.char_indices().nth(offset)?.0;
    let mut seen = 0usize;
    let mut end = text.len();
    for (byte, _) in text[start..].char_indices() {
        if seen == len {
            end = start + byte;
            break;
        }
        seen += 1;
    }
    if seen < len {
        return None;
    }
    Some(&text[start..end])
}

// ------------------------------------------------------------ on-disk files

/// One entity JSON file (people or places).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityFile {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub book_id: String,
    pub kind: EntityKind,
    #[serde(default)]
    pub entities: Vec<EntityRecord>,
}

impl EntityFile {
    pub fn new(kind: EntityKind, entities: Vec<EntityRecord>) -> EntityFile {
        EntityFile {
            schema_version: 1,
            book_id: String::new(),
            kind,
            entities,
        }
    }
}

/// One entity as written on disk.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityRecord {
    pub id: String,
    pub kind: EntityKind,
    pub name: String,
    #[serde(default)]
    pub name_simp: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub occurrences: Vec<OccurrenceRecord>,
}

impl EntityRecord {
    /// Minimal valid record; summary None means "not established".
    pub fn new(
        id: impl Into<String>,
        kind: EntityKind,
        name: impl Into<String>,
        occurrences: Vec<OccurrenceRecord>,
    ) -> EntityRecord {
        EntityRecord {
            id: id.into(),
            kind,
            name: name.into(),
            name_simp: None,
            aliases: Vec::new(),
            tags: Vec::new(),
            summary: None,
            occurrences,
        }
    }
}

/// One occurrence as written on disk: paragraph id, char offset, length, glyphs.
#[derive(Debug, Clone, Deserialize)]
pub struct OccurrenceRecord {
    pub p: String,
    pub o: usize,
    pub len: usize,
    pub form: String,
}

impl OccurrenceRecord {
    pub fn new(p: impl Into<String>, o: usize, len: usize, form: impl Into<String>) -> Self {
        OccurrenceRecord {
            p: p.into(),
            o,
            len,
            form: form.into(),
        }
    }
}

// ------------------------------------------------------------------- errors

/// Result of loading/validating the entity layer.
pub type EntityResult<T> = std::result::Result<T, EntityError>;

/// Fail-fast entity loading error. Every variant names the offending entity or
/// paragraph so a broken data file is fixable from the message alone.
#[derive(Debug)]
pub enum EntityError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    FileKindMismatch {
        path: PathBuf,
        expected: EntityKind,
        actual: EntityKind,
    },
    EmptyId,
    EmptyName {
        id: String,
    },
    DuplicateId {
        id: String,
    },
    KindMismatch {
        id: String,
        file_kind: EntityKind,
        entity_kind: EntityKind,
    },
    NoOccurrences {
        id: String,
    },
    UnknownParagraph {
        id: String,
        paragraph_id: String,
    },
    EmptyMatch {
        id: String,
        paragraph_id: String,
        offset: usize,
    },
    FormNotDeclared {
        id: String,
        form: String,
    },
    OffsetOutOfRange {
        id: String,
        paragraph_id: String,
        offset: usize,
        len: usize,
        char_count: usize,
    },
    FormMismatch {
        id: String,
        paragraph_id: String,
        offset: usize,
        len: usize,
        form: String,
        actual: String,
    },
}

impl fmt::Display for EntityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntityError::Io { path, source } => {
                write!(f, "讀取實體檔失敗（{}）：{source}", path.display())
            }
            EntityError::Json { path, source } => {
                write!(f, "實體檔不是合法 JSON（{}）：{source}", path.display())
            }
            EntityError::FileKindMismatch {
                path,
                expected,
                actual,
            } => write!(
                f,
                "實體檔類型不符（{}）：應為 {expected}，實為 {actual}",
                path.display()
            ),
            EntityError::EmptyId => write!(f, "實體 id 不能為空"),
            EntityError::EmptyName { id } => write!(f, "實體 {id} 的 name 不能為空"),
            EntityError::DuplicateId { id } => write!(f, "實體 id 重複：{id}"),
            EntityError::KindMismatch {
                id,
                file_kind,
                entity_kind,
            } => write!(
                f,
                "實體 {id} 的 kind（{entity_kind}）與所在檔案（{file_kind}）不符"
            ),
            EntityError::NoOccurrences { id } => write!(f, "實體 {id} 沒有任何出現記錄"),
            EntityError::UnknownParagraph { id, paragraph_id } => write!(
                f,
                "實體 {id} 掛在不存在的段上：{paragraph_id}"
            ),
            EntityError::EmptyMatch {
                id,
                paragraph_id,
                offset,
            } => write!(
                f,
                "實體 {id} 在段 {paragraph_id} 的出現長度為 0（偏移 {offset}）"
            ),
            EntityError::FormNotDeclared { id, form } => {
                write!(f, "實體 {id} 的出現字形「{form}」不在 name/aliases 中")
            }
            EntityError::OffsetOutOfRange {
                id,
                paragraph_id,
                offset,
                len,
                char_count,
            } => write!(
                f,
                "實體 {id} 在段 {paragraph_id} 的偏移越界：o={offset} len={len}，該段只有 {char_count} 個字"
            ),
            EntityError::FormMismatch {
                id,
                paragraph_id,
                offset,
                len,
                form,
                actual,
            } => write!(
                f,
                "實體 {id} 在段 {paragraph_id} 的偏移無法複核：text[{offset}..{}]=\" {actual} \"，form=\"{form}\"",
                offset + len
            ),
        }
    }
}

impl std::error::Error for EntityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            EntityError::Io { source, .. } => Some(source),
            EntityError::Json { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn read_json(path: &Path) -> EntityResult<EntityFile> {
    let bytes = std::fs::read(path).map_err(|source| EntityError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| EntityError::Json {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Book, Chapter, Group, Paragraph, Volume};

    /// One volume, one chapter, the given (id, text) paragraphs.
    fn fixture(paragraphs: &[(&str, &str)]) -> BookData {
        let volume = Volume {
            schema_version: 1,
            id: "v-1".into(),
            book_id: "toy".into(),
            group_id: "a".into(),
            title: "甲一".into(),
            title_simp: None,
            index: 1,
            chapters: vec![Chapter {
                id: "v-1-01".into(),
                title: "甲一之一".into(),
                paragraphs: paragraphs
                    .iter()
                    .map(|(id, text)| Paragraph {
                        id: (*id).into(),
                        text: (*text).into(),
                        translation: None,
                        annotations: Vec::new(),
                        glossary: Vec::new(),
                    })
                    .collect(),
            }],
        };
        let book = Book {
            schema_version: 1,
            id: "toy".into(),
            title: "玩具策".into(),
            title_simp: None,
            compiler: None,
            era: None,
            form: "國別體".into(),
            sources: Vec::new(),
            groups: vec![Group {
                id: "a".into(),
                title: "甲".into(),
                title_simp: None,
                volumes: vec!["v-1".into()],
            }],
            volume_order: vec!["v-1".into()],
        };
        BookData::from_parts(book, vec![volume]).expect("fixture is valid")
    }

    fn person(id: &str, name: &str, occ: Vec<OccurrenceRecord>) -> EntityRecord {
        EntityRecord::new(id, EntityKind::Person, name, occ)
    }

    fn place(id: &str, name: &str, occ: Vec<OccurrenceRecord>) -> EntityRecord {
        EntityRecord::new(id, EntityKind::Place, name, occ)
    }

    #[test]
    fn loads_entities_and_answers_paragraph_and_id_queries() {
        let data = fixture(&[("v-1-01-p0001", "衞鞅亡魏入秦。")]);
        let entities = Entities::build(
            &data,
            vec![EntityFile::new(
                EntityKind::Person,
                vec![person(
                    "person-wei-yang",
                    "衞鞅",
                    vec![OccurrenceRecord::new("v-1-01-p0001", 0, 2, "衞鞅")],
                )],
            )],
        )
        .expect("valid");

        let entity = entities.entity("person-wei-yang").expect("by id");
        assert_eq!(entity.name, "衞鞅");
        assert_eq!(entity.occurrence_count(), 1);
        assert_eq!(entities.len(), 1);
        assert!(!entities.is_empty());
        assert_eq!(entities.occurrence_total(), 1);
        assert_eq!(entities.covered_paragraph_count(), 1);

        let spans: Vec<(usize, usize, &str)> = entities
            .spans_in("v-1-01-p0001")
            .map(|span| (span.offset, span.len, span.entity.id.as_str()))
            .collect();
        assert_eq!(spans, vec![(0, 2, "person-wei-yang")]);

        // Unknown ids answer empty/None instead of panicking.
        assert!(entities.entity("ghost").is_none());
        assert_eq!(entities.spans_in("ghost").count(), 0);
        assert_eq!(entities.occurrences_in("ghost").count(), 0);
    }

    #[test]
    fn offsets_are_characters_not_bytes() {
        // 「(0) 秦(1) 」(2) 😀(3) 衞(4) 鞅(5) 入(6) 秦(7) 。(8) — the emoji is 4
        // bytes and every CJK glyph is 3, so a byte offset would land wrong.
        let text = "「秦」😀衞鞅入秦。";
        let data = fixture(&[("v-1-01-p0001", text)]);
        let entities = Entities::build(
            &data,
            vec![EntityFile::new(
                EntityKind::Person,
                vec![person(
                    "person-wei-yang",
                    "衞鞅",
                    vec![OccurrenceRecord::new("v-1-01-p0001", 4, 2, "衞鞅")],
                )],
            )],
        )
        .expect("char offsets validate");

        let span = entities.spans_in("v-1-01-p0001").next().expect("one span");
        let chars: Vec<char> = text.chars().collect();
        assert_eq!(span.offset, 4);
        assert_eq!(
            chars[span.offset..span.offset + span.len]
                .iter()
                .collect::<String>(),
            "衞鞅"
        );
        // Character 4 begins at byte 13 (3 + 3 + 3 + 4), so a byte offset would
        // not even land on a char boundary — which is why chars are mandatory.
        assert_eq!(text.char_indices().nth(4).map(|(byte, _)| byte), Some(13));
        assert!(!text.is_char_boundary(4));
    }

    #[test]
    fn char_slice_handles_emoji_and_fullwidth_punctuation() {
        let text = "「甲」😀乙，丙。";
        assert_eq!(char_slice(text, 4, 1), Some("乙"));
        assert_eq!(char_slice(text, 6, 2), Some("丙。"));
        assert_eq!(char_slice(text, 7, 2), None, "past the end");
        assert_eq!(char_slice(text, 4, 0), None, "zero length is not a match");
        assert_eq!(char_slice("", 0, 1), None);
    }

    #[test]
    fn overlapping_spans_keep_the_longest_match_deterministically() {
        // 東周(0,2) and 東周君(0,3) start together: the longer one wins.
        // 齊閔王(6,3) and 閔王(7,2) nest: the enclosing one wins.
        let data = fixture(&[("v-1-01-p0001", "東周君與齊閔王會。")]);
        let entities = Entities::build(
            &data,
            vec![
                EntityFile::new(
                    EntityKind::Person,
                    vec![
                        person(
                            "person-dongzhou-jun",
                            "東周君",
                            vec![OccurrenceRecord::new("v-1-01-p0001", 0, 3, "東周君")],
                        ),
                        person(
                            "person-qi-minwang",
                            "齊閔王",
                            vec![OccurrenceRecord::new("v-1-01-p0001", 4, 3, "齊閔王")],
                        ),
                        person(
                            "person-minwang",
                            "閔王",
                            vec![OccurrenceRecord::new("v-1-01-p0001", 5, 2, "閔王")],
                        ),
                    ],
                ),
                EntityFile::new(
                    EntityKind::Place,
                    vec![place(
                        "place-dongzhou",
                        "東周",
                        vec![OccurrenceRecord::new("v-1-01-p0001", 0, 2, "東周")],
                    )],
                ),
            ],
        )
        .expect("valid");

        // Raw keeps all five records; the render index drops the overlapped two.
        assert_eq!(entities.occurrences_in("v-1-01-p0001").count(), 4);
        let spans: Vec<(usize, usize, &str)> = entities
            .spans_in("v-1-01-p0001")
            .map(|span| (span.offset, span.len, span.entity.id.as_str()))
            .collect();
        assert_eq!(
            spans,
            vec![(0, 3, "person-dongzhou-jun"), (4, 3, "person-qi-minwang"),]
        );
        assert_eq!(entities.overlapping_paragraph_count(), 1);

        // Nothing nests and no offset is reused: the renderer's invariant.
        let mut previous_end = 0;
        for span in entities.spans_in("v-1-01-p0001") {
            assert!(span.offset >= previous_end, "spans must not overlap");
            previous_end = span.offset + span.len;
        }

        // Rebuilding yields the same order regardless of input order/hashing.
        let first: Vec<(usize, usize)> = entities
            .spans_in("v-1-01-p0001")
            .map(|span| (span.offset, span.len))
            .collect();
        for _ in 0..8 {
            let rebuilt = Entities::build(
                &data,
                vec![
                    EntityFile::new(
                        EntityKind::Person,
                        vec![
                            person(
                                "person-minwang",
                                "閔王",
                                vec![OccurrenceRecord::new("v-1-01-p0001", 5, 2, "閔王")],
                            ),
                            person(
                                "person-qi-minwang",
                                "齊閔王",
                                vec![OccurrenceRecord::new("v-1-01-p0001", 4, 3, "齊閔王")],
                            ),
                            person(
                                "person-dongzhou-jun",
                                "東周君",
                                vec![OccurrenceRecord::new("v-1-01-p0001", 0, 3, "東周君")],
                            ),
                        ],
                    ),
                    EntityFile::new(
                        EntityKind::Place,
                        vec![place(
                            "place-dongzhou",
                            "東周",
                            vec![OccurrenceRecord::new("v-1-01-p0001", 0, 2, "東周")],
                        )],
                    ),
                ],
            )
            .expect("valid");
            let again: Vec<(usize, usize)> = rebuilt
                .spans_in("v-1-01-p0001")
                .map(|span| (span.offset, span.len))
                .collect();
            assert_eq!(again, first, "resolution is deterministic");
        }
    }

    #[test]
    fn adjacent_spans_are_both_kept() {
        // 秦(0,1) and 昭王(1,2) touch but do not overlap.
        let data = fixture(&[("v-1-01-p0001", "秦昭王立。")]);
        let entities = Entities::build(
            &data,
            vec![
                EntityFile::new(
                    EntityKind::Place,
                    vec![place(
                        "place-qin",
                        "秦",
                        vec![OccurrenceRecord::new("v-1-01-p0001", 0, 1, "秦")],
                    )],
                ),
                EntityFile::new(
                    EntityKind::Person,
                    vec![person(
                        "person-zhaowang",
                        "昭王",
                        vec![OccurrenceRecord::new("v-1-01-p0001", 1, 2, "昭王")],
                    )],
                ),
            ],
        )
        .expect("valid");
        let spans: Vec<(usize, usize)> = entities
            .spans_in("v-1-01-p0001")
            .map(|span| (span.offset, span.len))
            .collect();
        assert_eq!(spans, vec![(0, 1), (1, 2)]);
        assert_eq!(entities.overlapping_paragraph_count(), 0);
    }

    #[test]
    fn an_offset_that_does_not_slice_back_to_the_form_is_rejected() {
        let data = fixture(&[("v-1-01-p0001", "衞鞅亡魏入秦。")]);
        // 「鞅亡」 is at 1..3, not 衞鞅 — the check is on the glyphs, not just bounds.
        let error = Entities::build(
            &data,
            vec![EntityFile::new(
                EntityKind::Person,
                vec![person(
                    "person-wei-yang",
                    "衞鞅",
                    vec![OccurrenceRecord::new("v-1-01-p0001", 1, 2, "衞鞅")],
                )],
            )],
        )
        .expect_err("form mismatch must fail fast");
        match error {
            EntityError::FormMismatch { actual, form, .. } => {
                assert_eq!(form, "衞鞅");
                assert_eq!(actual, "鞅亡");
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn empty_duplicate_and_undeclared_records_are_rejected() {
        let data = fixture(&[("v-1-01-p0001", "衞鞅亡魏入秦。")]);
        let build = |record: EntityRecord| {
            Entities::build(
                &data,
                vec![EntityFile::new(EntityKind::Person, vec![record])],
            )
        };

        assert!(matches!(
            build(person(
                "",
                "衞鞅",
                vec![OccurrenceRecord::new("v-1-01-p0001", 0, 2, "衞鞅")]
            )),
            Err(EntityError::EmptyId)
        ));
        assert!(matches!(
            build(person(
                "p",
                " ",
                vec![OccurrenceRecord::new("v-1-01-p0001", 0, 2, "衞鞅")]
            )),
            Err(EntityError::EmptyName { .. })
        ));
        assert!(matches!(
            build(person("p", "衞鞅", Vec::new())),
            Err(EntityError::NoOccurrences { .. })
        ));
        assert!(matches!(
            build(person(
                "p",
                "衞鞅",
                vec![OccurrenceRecord::new("ghost", 0, 2, "衞鞅")]
            )),
            Err(EntityError::UnknownParagraph { .. })
        ));
        assert!(matches!(
            build(person(
                "p",
                "衞鞅",
                vec![OccurrenceRecord::new("v-1-01-p0001", 0, 0, "衞鞅")]
            )),
            Err(EntityError::EmptyMatch { .. })
        ));
        assert!(matches!(
            build(person(
                "p",
                "衞鞅",
                vec![OccurrenceRecord::new("v-1-01-p0001", 0, 9, "衞鞅")]
            )),
            Err(EntityError::OffsetOutOfRange { .. })
        ));
        assert!(matches!(
            build(person(
                "p",
                "衞鞅",
                vec![OccurrenceRecord::new("v-1-01-p0001", 4, 2, "魏入")]
            )),
            Err(EntityError::FormNotDeclared { .. })
        ));

        let first = person(
            "dup",
            "衞鞅",
            vec![OccurrenceRecord::new("v-1-01-p0001", 0, 2, "衞鞅")],
        );
        let second = person(
            "dup",
            "衞鞅",
            vec![OccurrenceRecord::new("v-1-01-p0001", 0, 2, "衞鞅")],
        );
        assert!(matches!(
            Entities::build(
                &data,
                vec![EntityFile::new(EntityKind::Person, vec![first, second])]
            ),
            Err(EntityError::DuplicateId { .. })
        ));
    }

    #[test]
    fn kind_mismatches_are_rejected() {
        let data = fixture(&[("v-1-01-p0001", "衞鞅亡魏入秦。")]);
        let mut record = person(
            "person-wei-yang",
            "衞鞅",
            vec![OccurrenceRecord::new("v-1-01-p0001", 0, 2, "衞鞅")],
        );
        record.kind = EntityKind::Place;
        let error = Entities::build(
            &data,
            vec![EntityFile::new(EntityKind::Person, vec![record])],
        )
        .expect_err("kind mismatch");
        assert!(matches!(error, EntityError::KindMismatch { .. }));
        assert!(error.to_string().contains("person-wei-yang"));
    }

    #[test]
    fn summary_and_simplified_name_treat_null_and_blank_as_absent() {
        let mut record = person(
            "person-wei-yang",
            "衞鞅",
            vec![OccurrenceRecord::new("v-1-01-p0001", 0, 2, "衞鞅")],
        );
        record.summary = None;
        record.name_simp = Some("衞鞅".into());
        let data = fixture(&[("v-1-01-p0001", "衞鞅亡魏入秦。")]);
        let entities = Entities::build(
            &data,
            vec![EntityFile::new(EntityKind::Person, vec![record])],
        )
        .expect("valid");
        let entity = entities.entity("person-wei-yang").expect("entity");
        assert_eq!(entity.summary_text(), None);
        assert_eq!(
            entity.name_simp_text(),
            None,
            "same as name is not shown twice"
        );

        let mut blank = person(
            "person-blank",
            "衞",
            vec![OccurrenceRecord::new("v-1-01-p0001", 0, 1, "衞")],
        );
        blank.summary = Some("  ".into());
        let entities = Entities::build(
            &data,
            vec![EntityFile::new(EntityKind::Person, vec![blank])],
        )
        .expect("valid");
        assert_eq!(
            entities.entity("person-blank").unwrap().summary_text(),
            None
        );
    }

    #[test]
    fn occurrences_come_back_in_reading_order() {
        let data = fixture(&[("v-1-01-p0001", "秦伐魏。"), ("v-1-01-p0002", "魏與秦。")]);
        let entities = Entities::build(
            &data,
            vec![EntityFile::new(
                EntityKind::Place,
                vec![place(
                    "place-qin",
                    "秦",
                    vec![
                        OccurrenceRecord::new("v-1-01-p0002", 2, 1, "秦"),
                        OccurrenceRecord::new("v-1-01-p0001", 0, 1, "秦"),
                    ],
                )],
            )],
        )
        .expect("valid");
        let order: Vec<&str> = entities
            .entity("place-qin")
            .unwrap()
            .occurrences
            .iter()
            .map(|occurrence| occurrence.paragraph_id.as_str())
            .collect();
        assert_eq!(order, vec!["v-1-01-p0001", "v-1-01-p0002"]);
    }

    #[test]
    fn of_kind_filters_without_reordering() {
        let data = fixture(&[("v-1-01-p0001", "秦昭王立。")]);
        let entities = Entities::build(
            &data,
            vec![
                EntityFile::new(
                    EntityKind::Person,
                    vec![person(
                        "person-zhaowang",
                        "昭王",
                        vec![OccurrenceRecord::new("v-1-01-p0001", 1, 2, "昭王")],
                    )],
                ),
                EntityFile::new(
                    EntityKind::Place,
                    vec![place(
                        "place-qin",
                        "秦",
                        vec![OccurrenceRecord::new("v-1-01-p0001", 0, 1, "秦")],
                    )],
                ),
            ],
        )
        .expect("valid");
        let people: Vec<&str> = entities
            .of_kind(EntityKind::Person)
            .map(|e| e.id.as_str())
            .collect();
        let places: Vec<&str> = entities
            .of_kind(EntityKind::Place)
            .map(|e| e.id.as_str())
            .collect();
        assert_eq!(people, vec!["person-zhaowang"]);
        assert_eq!(places, vec!["place-qin"]);
        assert_eq!(EntityKind::parse("person"), Some(EntityKind::Person));
        assert_eq!(EntityKind::parse("地名"), Some(EntityKind::Place));
        assert_eq!(EntityKind::parse("ghost"), None);
    }

    #[test]
    fn empty_index_answers_everything_without_panicking() {
        let entities = Entities::empty();
        assert!(entities.is_empty());
        assert_eq!(entities.len(), 0);
        assert_eq!(entities.occurrence_total(), 0);
        assert_eq!(entities.covered_paragraph_count(), 0);
        assert_eq!(entities.overlapping_paragraph_count(), 0);
        assert!(entities.entity("anything").is_none());
        assert_eq!(entities.spans_in("p").count(), 0);
        assert_eq!(entities.of_kind(EntityKind::Place).count(), 0);
    }
}
