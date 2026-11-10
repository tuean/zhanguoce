//! Coverage: run the whole real corpus through the real 繁 → 簡 table.
//!
//! One pass over 33 volumes / 497 chapters / 6663 paragraphs answers three
//! questions that unit tests cannot: does any character that a character rule
//! covers survive conversion of the fields the reader converts (原文 + 注疏),
//! is conversion stable under a second pass, and does 白話 really stay out of
//! the table's reach?
//!
//! Pinned leftovers: EMPTY. Longer sources win, so a phrase rule can protect a
//! multi-character word and leave its characters in the output even though a
//! character rule also covers them. The whole corpus produces none of those
//! today; if this test fails, either the table gained a rule that leaves a
//! known word untranslated or the corpus gained new text — both are decisions
//! that deserve a human look, so the set is asserted, not printed and ignored.

use std::collections::BTreeSet;
use std::path::PathBuf;

use zgc_core::{Book, Variants};

/// Paragraphs whose 白話 already carries a character-rule source. The reader
/// never runs 白話 through the table (it is authored in 簡體), so these stay as
/// written; the pin records the four exceptions rather than asserting a
/// rule-free field the corpus does not actually have.
const PINNED_TRAN: [(&str, char); 4] = [
    ("chu-1-05-p0012", '拚'),
    ("chu-1-18-p0014", '扞'),
    ("han-1-19-p0004", '拚'),
    ("qin-2-12-p0010", '谿'),
];

fn data_dir() -> PathBuf {
    std::env::var_os("ZGC_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/zhanguoce"))
}

/// data/zhanguoce -> data/shared/t2s.tsv, the same sibling layout main.rs uses.
fn table_path() -> PathBuf {
    data_dir()
        .parent()
        .expect("the data directory has a parent")
        .join("shared")
        .join("t2s.tsv")
}

fn table_text() -> String {
    let path = table_path();
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn variants() -> Variants {
    let path = table_path();
    Variants::load(&path).unwrap_or_else(|error| panic!("load {}: {error}", path.display()))
}

/// Sources of single-character rules that actually change the glyph. Phrase
/// rules and identity rules are excluded on purpose: they are the protections.
fn char_rule_sources(table: &str) -> BTreeSet<char> {
    let mut sources = BTreeSet::new();
    for line in table.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((source, target)) = line.split_once('\t') else {
            continue;
        };
        let source = source.trim();
        let target = target.trim();
        if source.chars().count() == 1 && source != target {
            if let Some(ch) = source.chars().next() {
                sources.insert(ch);
            }
        }
    }
    sources
}

#[test]
fn corpus_converts_clean_and_idempotently() {
    let table = table_text();
    let sources = char_rule_sources(&table);
    let variants = variants();
    let (chars, phrases) = variants.rule_counts();
    assert_eq!(
        (chars, phrases),
        (3229, 311),
        "the shipped table changed shape; re-run the coverage numbers before editing this pin"
    );

    let book = Book::load(&data_dir()).expect("real data must load");
    let mut paragraphs = 0usize;
    let mut leftovers: BTreeSet<char> = BTreeSet::new();
    let mut leftover_where: Vec<(String, char)> = Vec::new();
    let mut unstable: Vec<String> = Vec::new();

    for (_, paragraph) in book.iter_paragraphs() {
        paragraphs += 1;
        // Exactly the 繁體-side fields the reading page converts.
        let mut pieces: Vec<&str> = vec![paragraph.text.as_str()];
        for annotation in &paragraph.annotations {
            if let Some(source) = annotation.source.as_deref() {
                pieces.push(source);
            }
            pieces.push(&annotation.text);
        }
        for piece in pieces {
            let once = variants.to_simplified(piece);
            let twice = variants.to_simplified(&once);
            if once != twice {
                unstable.push(paragraph.id.clone());
            }
            for ch in once.chars() {
                if sources.contains(&ch) {
                    if leftovers.insert(ch) {
                        leftover_where.push((paragraph.id.clone(), ch));
                    }
                }
            }
        }
    }

    assert_eq!(paragraphs, 6663, "the corpus this test covers");
    assert!(
        leftovers.is_empty(),
        "converted 原文/注疏 still contains character-rule sources: {} leftover char(s) {:?} (first seen {:?})",
        leftovers.len(),
        leftovers.iter().collect::<Vec<_>>(),
        leftover_where
    );
    assert!(
        unstable.is_empty(),
        "conversion is not idempotent for {} paragraph(s): {:?}",
        unstable.len(),
        unstable
    );
    println!(
        "coverage: {paragraphs} paragraphs, {} char rules, {} phrase rules, 0 leftovers",
        chars, phrases
    );
}

#[test]
fn translation_is_left_alone_apart_from_the_pinned_sources() {
    let table = table_text();
    let sources = char_rule_sources(&table);
    let book = Book::load(&data_dir()).expect("real data must load");

    let mut found: BTreeSet<(String, char)> = BTreeSet::new();
    for (_, paragraph) in book.iter_paragraphs() {
        let Some(translation) = paragraph.translation.as_deref() else {
            continue;
        };
        for ch in translation.chars() {
            if sources.contains(&ch) {
                found.insert((paragraph.id.clone(), ch));
            }
        }
    }
    let expected: BTreeSet<(String, char)> = PINNED_TRAN
        .iter()
        .map(|(id, ch)| ((*id).to_string(), *ch))
        .collect();
    assert_eq!(
        found,
        expected,
        "白話 gained or lost a character-rule source; pages.rs never converts 白話, so update PINNED_TRAN only together with a data change"
    );
}

/// Titles are corpus data too: the reading page, the 目錄 and the volume
/// listing print `chapter.title` / `volume.title` / `group.title` verbatim,
/// so in 簡體 mode they must survive the same table as 原文. This walks every
/// title in the real corpus and asserts the same "no character-rule source
/// survives" property the paragraph test asserts.
#[test]
fn every_title_converts_clean() {
    let table = table_text();
    let sources = char_rule_sources(&table);
    let variants = variants();
    let data = Book::load(&data_dir()).expect("real data must load");

    let mut titles: Vec<(String, &str)> = vec![("book".to_string(), data.book().title.as_str())];
    for group in data.groups() {
        titles.push((format!("group:{}", group.id), group.title.as_str()));
    }
    for volume in data.volumes() {
        titles.push((format!("volume:{}", volume.id), volume.title.as_str()));
        for chapter in &volume.chapters {
            titles.push((format!("chapter:{}", chapter.id), chapter.title.as_str()));
        }
    }

    let mut leftovers: BTreeSet<char> = BTreeSet::new();
    let mut leftover_where: Vec<(String, char)> = Vec::new();
    let mut unstable: Vec<String> = Vec::new();
    for (id, title) in &titles {
        let once = variants.to_simplified(title);
        if once != variants.to_simplified(&once) {
            unstable.push(id.clone());
        }
        for ch in once.chars() {
            if sources.contains(&ch) {
                if leftovers.insert(ch) {
                    leftover_where.push((id.clone(), ch));
                }
            }
        }
    }

    assert_eq!(
        titles.len(),
        1 + data.group_count() + data.volume_count() + data.chapter_count(),
        "every group, volume and chapter title in the book is covered"
    );
    assert!(
        leftovers.is_empty(),
        "converted titles still contain character-rule sources: {} leftover char(s) {:?} (first seen {:?})",
        leftovers.len(),
        leftovers.iter().collect::<Vec<_>>(),
        leftover_where
    );
    assert!(
        unstable.is_empty(),
        "title conversion is not idempotent for {} title(s): {:?}",
        unstable.len(),
        unstable
    );
    println!(
        "titles: {} checked (1 book, {} groups, {} volumes, {} chapters), 0 leftovers",
        titles.len(),
        data.group_count(),
        data.volume_count(),
        data.chapter_count()
    );
}

/// 對照 mode prints the 簡體 line directly under its 繁體 source and reuses the
/// source's character offsets for the highlight, so conversion must never
/// change a string's length. A rule that did would shift every mark in the
/// column; the renderer falls back to unmarked text, but the table is expected
/// to be length-preserving in the first place.
#[test]
fn conversion_preserves_the_character_count() {
    let variants = variants();
    let book = Book::load(&data_dir()).expect("real data must load");
    let mut shifted: BTreeSet<String> = BTreeSet::new();

    for (_, paragraph) in book.iter_paragraphs() {
        let mut pieces: Vec<&str> = vec![paragraph.text.as_str()];
        for annotation in &paragraph.annotations {
            if let Some(source) = annotation.source.as_deref() {
                pieces.push(source);
            }
            pieces.push(&annotation.text);
        }
        for piece in pieces {
            let converted = variants.to_simplified(piece);
            if converted.chars().count() != piece.chars().count() {
                shifted.insert(paragraph.id.clone());
            }
        }
    }

    assert!(
        shifted.is_empty(),
        "conversion changed the character count of {} paragraph(s): {:?}",
        shifted.len(),
        shifted
    );
    println!("length: every 原文/注疏 string keeps its character count");
}
