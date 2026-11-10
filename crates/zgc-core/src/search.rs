//! CJK full-text search: an adjacent-character (bigram) inverted index.
//!
//! Chinese has no word delimiters, so instead of a tokenizer/dictionary this
//! crate indexes every adjacent character pair of every indexed field and
//! verifies candidates against the literal query. That keeps the dependency
//! surface at `serde`/`serde_json` while still supporting arbitrary substrings.

use std::collections::HashMap;
use std::fmt;

use crate::data::BookData;
use crate::types::Paragraph;

/// Sentinel for "this field is not an array item".
const NOT_AN_ITEM: u32 = u32::MAX;
/// Characters of context kept on each side of a hit snippet.
const SNIPPET_RADIUS: usize = 10;

/// Which indexed text a hit came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FieldSource {
    /// 繁体原文.
    Text,
    /// 简体白话.
    Translation,
    /// 注疏（`annotations[].text`）.
    Annotation,
    /// 词条（`glossary[].term`）.
    Glossary,
}

impl FieldSource {
    pub fn as_str(self) -> &'static str {
        match self {
            FieldSource::Text => "text",
            FieldSource::Translation => "translation",
            FieldSource::Annotation => "annotation",
            FieldSource::Glossary => "glossary",
        }
    }
}

impl fmt::Display for FieldSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Two adjacent characters, packed into one u64 key.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Bigram(u64);

impl Bigram {
    fn new(first: char, second: char) -> Self {
        Bigram(((first as u32 as u64) << 32) | second as u32 as u64)
    }
}

impl fmt::Debug for Bigram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Bigram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let first = char::from_u32((self.0 >> 32) as u32).unwrap_or('?');
        let second = char::from_u32(self.0 as u32).unwrap_or('?');
        write!(f, "{first}{second}")
    }
}

/// One occurrence of a bigram, pointing at the exact paragraph/field/offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Posting {
    /// Flat paragraph index (`BookData::position_at` turns it back into a Position).
    paragraph: u32,
    field: FieldSource,
    /// Index inside `annotations` / `glossary`, `NOT_AN_ITEM` otherwise.
    field_index: u32,
    char_offset: u32,
    byte_offset: u32,
}

/// Restrict a query to the whole book, one group, or one volume.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Scope {
    #[default]
    All,
    Group(String),
    Volume(String),
}

impl Scope {
    pub fn group(id: impl Into<String>) -> Self {
        Scope::Group(id.into())
    }

    pub fn volume(id: impl Into<String>) -> Self {
        Scope::Volume(id.into())
    }

    pub fn all() -> Self {
        Scope::All
    }
}

/// One search result: ids are enough to re-read the passage, offsets/snippet are
/// enough to highlight it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub volume_id: String,
    pub chapter_id: String,
    pub paragraph_id: String,
    pub field: FieldSource,
    /// Index inside `annotations` / `glossary`; `None` for `text`/`translation`.
    pub field_index: Option<usize>,
    /// Offset in Unicode scalar values (what an editor/UI cursor uses).
    pub char_offset: usize,
    /// Byte offset (safe slicing: always a char boundary).
    pub byte_offset: usize,
    /// Surrounding context with `…` markers.
    pub snippet: String,
}

/// The immutable bigram index. Build once per loaded book, query many times.
#[derive(Debug, Clone)]
pub struct SearchIndex {
    postings: HashMap<Bigram, Vec<Posting>>,
    /// Total bigram occurrences (evidence for "index size" reporting).
    occurrences: usize,
}

impl SearchIndex {
    /// Build the index over `text`, `translation`, `annotations[].text` and
    /// `glossary[].term` of every paragraph, in reading order.
    pub fn build(book: &BookData) -> Self {
        let mut postings: HashMap<Bigram, Vec<Posting>> = HashMap::new();
        for (flat, position) in book.iter_positions().enumerate() {
            let Some(paragraph) = book.paragraph(position) else {
                continue;
            };
            for (field, field_index, text) in field_texts(paragraph) {
                index_text(&mut postings, flat as u32, field, field_index, text);
            }
        }
        let occurrences = postings.values().map(Vec::len).sum();
        SearchIndex {
            postings,
            occurrences,
        }
    }

    /// Number of distinct bigrams in the index.
    pub fn bigram_count(&self) -> usize {
        self.postings.len()
    }

    /// Number of indexed bigram occurrences.
    pub fn posting_count(&self) -> usize {
        self.occurrences
    }

    /// All hits, in stable reading order.
    pub fn search(&self, book: &BookData, query: &str, scope: &Scope) -> Vec<Hit> {
        self.search_limited(book, query, scope, usize::MAX)
    }

    /// Like `search` but stops after `limit` hits (still stable).
    pub fn search_limited(
        &self,
        book: &BookData,
        query: &str,
        scope: &Scope,
        limit: usize,
    ) -> Vec<Hit> {
        let query_chars: Vec<char> = query.trim().chars().map(fold).collect();
        if query_chars.is_empty() || limit == 0 {
            return Vec::new();
        }
        let mask = scope_mask(book, scope);
        let mut raw: Vec<(u32, Posting)> = Vec::new();

        let pairs = query_pairs(&query_chars);
        if pairs.is_empty() {
            // Single-character or whitespace-only query: literal scan.
            raw = scan_literal(book, &query_chars, &mask);
        } else {
            // Drive the lookup from the rarest bigram so common queries stay cheap.
            // Tie-break on the smaller relative offset → deterministic.
            let (driver, driver_rel) = pairs
                .iter()
                .min_by_key(|(bigram, rel)| (self.postings.get(bigram).map_or(0, Vec::len), *rel))
                .copied()
                .expect("pairs is non-empty");

            if let Some(postings) = self.postings.get(&driver) {
                for posting in postings {
                    if posting.char_offset < driver_rel as u32 {
                        continue;
                    }
                    let start = posting.char_offset - driver_rel as u32;
                    let Some(position) = book.position_at(posting.paragraph as usize) else {
                        continue;
                    };
                    if !allowed(&mask, position.volume) {
                        continue;
                    }
                    let Some(text) = field_text(book, *posting) else {
                        continue;
                    };
                    let Some(byte_offset) = byte_offset_of_char(text, start) else {
                        continue;
                    };
                    if !matches_at(text, byte_offset, &query_chars) {
                        continue;
                    }
                    raw.push((
                        posting.paragraph,
                        Posting {
                            char_offset: start,
                            byte_offset: byte_offset as u32,
                            ..*posting
                        },
                    ));
                }
            }
        }

        raw.sort_unstable();
        raw.dedup();

        let mut hits = Vec::new();
        for (flat, posting) in raw {
            if hits.len() >= limit {
                break;
            }
            let Some(position) = book.position_at(flat as usize) else {
                continue;
            };
            let (Some(volume), Some(chapter), Some(paragraph)) = (
                book.volume(position.volume),
                book.chapter(position.volume, position.chapter),
                book.paragraph(position),
            ) else {
                continue;
            };
            let Some(text) = field_text(book, posting) else {
                continue;
            };
            let snippet = snippet(
                text,
                posting.char_offset as usize,
                query_chars.len(),
                SNIPPET_RADIUS,
            );
            hits.push(Hit {
                volume_id: volume.id.clone(),
                chapter_id: chapter.id.clone(),
                paragraph_id: paragraph.id.clone(),
                field: posting.field,
                field_index: (posting.field_index != NOT_AN_ITEM)
                    .then_some(posting.field_index as usize),
                char_offset: posting.char_offset as usize,
                byte_offset: posting.byte_offset as usize,
                snippet,
            });
        }
        hits
    }
}

fn field_texts(paragraph: &Paragraph) -> Vec<(FieldSource, u32, &str)> {
    let mut fields = Vec::with_capacity(
        1 + usize::from(paragraph.translation.is_some())
            + paragraph.annotations.len()
            + paragraph.glossary.len(),
    );
    fields.push((FieldSource::Text, NOT_AN_ITEM, paragraph.text.as_str()));
    if let Some(translation) = paragraph.translation.as_deref() {
        fields.push((FieldSource::Translation, NOT_AN_ITEM, translation));
    }
    for (index, annotation) in paragraph.annotations.iter().enumerate() {
        fields.push((
            FieldSource::Annotation,
            index as u32,
            annotation.text.as_str(),
        ));
    }
    for (index, glossary) in paragraph.glossary.iter().enumerate() {
        fields.push((FieldSource::Glossary, index as u32, glossary.term.as_str()));
    }
    fields
}

fn field_text<'a>(book: &'a BookData, posting: Posting) -> Option<&'a str> {
    let position = book.position_at(posting.paragraph as usize)?;
    let paragraph = book.paragraph(position)?;
    match posting.field {
        FieldSource::Text => Some(paragraph.text.as_str()),
        FieldSource::Translation => paragraph.translation.as_deref(),
        FieldSource::Annotation => paragraph
            .annotations
            .get(posting.field_index as usize)
            .map(|annotation| annotation.text.as_str()),
        FieldSource::Glossary => paragraph
            .glossary
            .get(posting.field_index as usize)
            .map(|entry| entry.term.as_str()),
    }
}

fn index_text(
    postings: &mut HashMap<Bigram, Vec<Posting>>,
    paragraph: u32,
    field: FieldSource,
    field_index: u32,
    text: &str,
) {
    let mut previous: Option<(char, u32, u32)> = None;
    for (char_offset, (byte_offset, character)) in text.char_indices().enumerate() {
        let current = fold(character);
        if let Some((previous_char, previous_char_offset, previous_byte_offset)) = previous {
            if !is_space(previous_char) && !is_space(current) {
                postings
                    .entry(Bigram::new(previous_char, current))
                    .or_default()
                    .push(Posting {
                        paragraph,
                        field,
                        field_index,
                        char_offset: previous_char_offset,
                        byte_offset: previous_byte_offset,
                    });
            }
        }
        previous = Some((current, char_offset as u32, byte_offset as u32));
    }
}

/// Query bigrams plus their relative char offset inside the trimmed query.
fn query_pairs(query_chars: &[char]) -> Vec<(Bigram, usize)> {
    let mut pairs = Vec::with_capacity(query_chars.len().saturating_sub(1));
    for index in 0..query_chars.len().saturating_sub(1) {
        let first = query_chars[index];
        let second = query_chars[index + 1];
        if is_space(first) || is_space(second) {
            continue;
        }
        pairs.push((Bigram::new(first, second), index));
    }
    pairs
}

fn scan_literal(
    book: &BookData,
    query_chars: &[char],
    mask: &Option<Vec<bool>>,
) -> Vec<(u32, Posting)> {
    let mut found = Vec::new();
    for (flat, position) in book.iter_positions().enumerate() {
        if !allowed(mask, position.volume) {
            continue;
        }
        let Some(paragraph) = book.paragraph(position) else {
            continue;
        };
        for (field, field_index, text) in field_texts(paragraph) {
            let chars: Vec<(usize, char)> = text.char_indices().collect();
            if chars.len() < query_chars.len() {
                continue;
            }
            for start in 0..=chars.len() - query_chars.len() {
                let hit = query_chars
                    .iter()
                    .enumerate()
                    .all(|(offset, expected)| fold(chars[start + offset].1) == *expected);
                if hit {
                    found.push((
                        flat as u32,
                        Posting {
                            paragraph: flat as u32,
                            field,
                            field_index,
                            char_offset: start as u32,
                            byte_offset: chars[start].0 as u32,
                        },
                    ));
                }
            }
        }
    }
    found
}

fn scope_mask(book: &BookData, scope: &Scope) -> Option<Vec<bool>> {
    match scope {
        Scope::All => None,
        Scope::Group(group_id) => {
            let mut mask = vec![false; book.volume_count()];
            for index in book.group_volume_indices(group_id) {
                mask[*index] = true;
            }
            Some(mask)
        }
        Scope::Volume(volume_id) => {
            let mut mask = vec![false; book.volume_count()];
            if let Some(index) = book.volume_index(volume_id) {
                mask[index] = true;
            }
            Some(mask)
        }
    }
}

fn allowed(mask: &Option<Vec<bool>>, volume: usize) -> bool {
    match mask {
        None => true,
        Some(mask) => mask.get(volume).copied().unwrap_or(false),
    }
}

fn byte_offset_of_char(text: &str, char_index: u32) -> Option<usize> {
    if char_index == 0 {
        return Some(0);
    }
    text.char_indices()
        .nth(char_index as usize)
        .map(|(byte_offset, _)| byte_offset)
}

fn matches_at(text: &str, byte_offset: usize, query_chars: &[char]) -> bool {
    let mut chars = text[byte_offset..].chars();
    for expected in query_chars {
        match chars.next() {
            Some(actual) if fold(actual) == *expected => {}
            _ => return false,
        }
    }
    true
}

fn snippet(text: &str, char_offset: usize, match_len: usize, radius: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let start = char_offset.saturating_sub(radius).min(chars.len());
    let end = char_offset
        .saturating_add(match_len)
        .saturating_add(radius)
        .min(chars.len())
        .max(start);
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(&chars[start..end]);
    if end < chars.len() {
        out.push('…');
    }
    out
}

fn fold(character: char) -> char {
    character.to_ascii_lowercase()
}

fn is_space(character: char) -> bool {
    character.is_whitespace()
}

#[cfg(test)]
mod tests {
    use crate::data::BookData;
    use crate::types::{Annotation, Book, Chapter, Glossary, Group, Paragraph, Volume};

    use super::{FieldSource, Scope, SearchIndex};

    fn paragraph(id: &str, text: &str, translation: Option<&str>) -> Paragraph {
        Paragraph {
            id: id.into(),
            text: text.into(),
            translation: translation.map(str::to_owned),
            annotations: Vec::new(),
            glossary: Vec::new(),
        }
    }

    fn search_book() -> BookData {
        let mut annotated = paragraph("v-1-01-p0001", "衛鞅亡魏入秦", Some("卫鞅从魏国逃到秦国"));
        annotated.annotations.push(Annotation {
            source: Some("鮑彪".into()),
            text: "衛鞅即商鞅".into(),
            edition: None,
        });
        annotated.glossary.push(Glossary {
            term: "衛鞅".into(),
            pinyin: Some("wei yang".into()),
            gloss: Some("即商鞅".into()),
        });

        let volume_one = Volume {
            schema_version: 1,
            id: "v-1".into(),
            book_id: "toy".into(),
            group_id: "a".into(),
            title: "甲一".into(),
            title_simp: None,
            index: 1,
            chapters: vec![Chapter {
                id: "v-1-01".into(),
                title: "第一章".into(),
                paragraphs: vec![annotated, paragraph("v-1-01-p0002", "蘇秦說齊王", None)],
            }],
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
                title: "第二章".into(),
                paragraphs: vec![paragraph("v-2-01-p0001", "秦伐魏", Some("秦国攻打魏国"))],
            }],
        };
        let book = Book {
            schema_version: 1,
            id: "toy".into(),
            title: "玩具".into(),
            title_simp: None,
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
        BookData::from_parts(book, vec![volume_one, volume_two]).expect("search fixture is valid")
    }

    #[test]
    fn text_translation_annotation_and_glossary_are_all_indexed() {
        let book = search_book();
        let index = SearchIndex::build(&book);

        let text_hit = index.search(&book, "衛鞅", &Scope::All);
        assert!(text_hit
            .iter()
            .any(|hit| hit.field == FieldSource::Text && hit.paragraph_id == "v-1-01-p0001"));
        assert!(text_hit
            .iter()
            .any(|hit| hit.field == FieldSource::Annotation));
        assert!(text_hit
            .iter()
            .any(|hit| hit.field == FieldSource::Glossary));

        let translation_hits = index.search(&book, "魏国", &Scope::All);
        assert_eq!(translation_hits.len(), 2, "{translation_hits:?}");
        assert!(translation_hits
            .iter()
            .all(|hit| hit.field == FieldSource::Translation));
        assert_eq!(translation_hits[0].paragraph_id, "v-1-01-p0001");

        let annotation_hits = index.search(&book, "商鞅", &Scope::All);
        assert!(annotation_hits
            .iter()
            .all(|hit| hit.field == FieldSource::Annotation || hit.field == FieldSource::Glossary));
    }

    #[test]
    fn hit_offsets_point_at_the_literal_match() {
        let book = search_book();
        let index = SearchIndex::build(&book);
        let hits = index.search(&book, "亡魏", &Scope::All);
        assert!(!hits.is_empty());
        for hit in &hits {
            let position = book
                .iter_positions()
                .find(|position| book.paragraph(*position).unwrap().id == hit.paragraph_id)
                .unwrap();
            let paragraph = book.paragraph(position).unwrap();
            let text = match hit.field {
                FieldSource::Text => paragraph.text.as_str(),
                FieldSource::Translation => paragraph.translation.as_deref().unwrap(),
                FieldSource::Annotation => &paragraph.annotations[hit.field_index.unwrap()].text,
                FieldSource::Glossary => &paragraph.glossary[hit.field_index.unwrap()].term,
            };
            assert_eq!(&text[hit.byte_offset..], "亡魏入秦");
            assert_eq!(
                text[..hit.byte_offset].chars().count(),
                hit.char_offset,
                "char and byte offsets must stay consistent"
            );
            assert!(hit.snippet.contains("亡魏"));
        }
    }

    #[test]
    fn scope_limits_hits_to_group_or_volume() {
        let book = search_book();
        let index = SearchIndex::build(&book);

        let all = index.search(&book, "秦", &Scope::All);
        assert!(all.len() >= 3, "秦 should hit text, translation and more");

        let only_second_volume = index.search(&book, "秦", &Scope::Volume("v-2".into()));
        assert!(!only_second_volume.is_empty());
        assert!(only_second_volume.iter().all(|hit| hit.volume_id == "v-2"));

        let only_group_a = index.search(&book, "秦", &Scope::Group("a".into()));
        assert!(!only_group_a.is_empty());
        assert!(only_group_a.iter().all(|hit| hit.volume_id == "v-1"));

        let unknown = index.search(&book, "秦", &Scope::Group("ghost".into()));
        assert!(unknown.is_empty());
    }

    #[test]
    fn result_order_is_stable_across_runs() {
        let book = search_book();
        let index = SearchIndex::build(&book);
        let first = index.search(&book, "秦", &Scope::All);
        let second = index.search(&book, "秦", &Scope::All);
        assert_eq!(first, second);

        let mut sorted = first.clone();
        sorted.sort_by(|a, b| {
            (
                a.volume_id.as_str(),
                a.chapter_id.as_str(),
                a.paragraph_id.as_str(),
                a.field,
                a.field_index,
                a.char_offset,
            )
                .cmp(&(
                    b.volume_id.as_str(),
                    b.chapter_id.as_str(),
                    b.paragraph_id.as_str(),
                    b.field,
                    b.field_index,
                    b.char_offset,
                ))
        });
        assert_eq!(first, sorted, "hits must already be in reading order");
    }

    #[test]
    fn limit_caps_hits_without_changing_order() {
        let book = search_book();
        let index = SearchIndex::build(&book);
        let all = index.search(&book, "秦", &Scope::All);
        assert!(all.len() > 1);
        let limited = index.search_limited(&book, "秦", &Scope::All, 1);
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0], all[0]);
    }

    #[test]
    fn unknown_query_returns_nothing() {
        let book = search_book();
        let index = SearchIndex::build(&book);
        assert!(index.search(&book, "秦始皇", &Scope::All).is_empty());
        assert!(index.search(&book, "   ", &Scope::All).is_empty());
        assert!(index.search(&book, "", &Scope::All).is_empty());
    }

    #[test]
    fn single_character_query_falls_back_to_literal_scan() {
        let book = search_book();
        let index = SearchIndex::build(&book);
        let hits = index.search(&book, "衛", &Scope::All);
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|hit| hit.snippet.contains('衛')));
    }

    #[test]
    fn index_reports_its_size() {
        let book = search_book();
        let index = SearchIndex::build(&book);
        assert!(index.bigram_count() > 0);
        assert!(index.posting_count() >= index.bigram_count());
    }
}
