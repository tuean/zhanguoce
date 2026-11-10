//! Loader tests against hand-written fixture directories (real IO, fail-fast paths).
//!
//! These deliberately avoid the real corpus: they pin the *contract* (ordering,
//! file-set equality, id uniqueness, non-empty text), not the current data.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use zgc_core::{Book, LoadError};

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("zgc-core-{label}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        TempDir { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn write_json(dir: &Path, relative: &str, value: &Value) {
    let path = dir.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let bytes = serde_json::to_vec_pretty(value).unwrap();
    std::fs::write(&path, bytes).unwrap();
}

fn book_json(volume_order: &[&str], group_volumes: &[&str]) -> Value {
    json!({
        "schemaVersion": 1,
        "id": "toy",
        "title": "玩具",
        "titleSimp": "玩具",
        "compiler": null,
        "era": null,
        "form": "國別體",
        "sources": [],
        "groups": [{ "id": "a", "title": "甲", "titleSimp": "甲", "volumes": group_volumes }],
        "volumeOrder": volume_order
    })
}

fn paragraph_json(id: &str, text: &str) -> Value {
    // annotations / glossary deliberately absent: missing ⇒ empty, never null.
    json!({ "id": id, "text": text, "translation": "白话" })
}

fn chapter_json(id: &str, paragraphs: &[Value]) -> Value {
    json!({ "id": id, "title": id, "paragraphs": paragraphs })
}

fn volume_json(id: &str, index: u32, chapters: &[Value]) -> Value {
    json!({
        "schemaVersion": 1,
        "id": id,
        "bookId": "toy",
        "groupId": "a",
        "title": id,
        "titleSimp": id,
        "index": index,
        "chapters": chapters
    })
}

fn expect_load_error(dir: &Path) -> LoadError {
    Book::load(dir).expect_err("fixture must be rejected")
}

#[test]
fn volume_order_wins_over_file_names_and_index() {
    let dir = TempDir::new("order");
    // volumeOrder says b-vol first; index says a-vol (1) < b-vol (9).
    write_json(
        &dir.path,
        "book.json",
        &book_json(&["b-vol", "a-vol"], &["b-vol", "a-vol"]),
    );
    write_json(
        &dir.path,
        "volumes/a-vol.json",
        &volume_json(
            "a-vol",
            1,
            &[chapter_json(
                "a-vol-01",
                &[paragraph_json("a-vol-01-p0001", "秦伐魏")],
            )],
        ),
    );
    write_json(
        &dir.path,
        "volumes/b-vol.json",
        &volume_json(
            "b-vol",
            9,
            &[chapter_json(
                "b-vol-01",
                &[paragraph_json("b-vol-01-p0001", "齊伐燕")],
            )],
        ),
    );

    let data = Book::load(&dir.path).expect("fixture loads");
    let ids: Vec<&str> = data
        .volumes()
        .iter()
        .map(|volume| volume.id.as_str())
        .collect();
    assert_eq!(ids, vec!["b-vol", "a-vol"], "must follow volumeOrder");
    assert_eq!(data.volume_count(), 2);
    assert_eq!(data.paragraph_count(), 2);
    assert_eq!(data.axis().as_str(), "國別體");
}

#[test]
fn missing_volume_file_fails_fast_with_path() {
    let dir = TempDir::new("missing");
    write_json(&dir.path, "book.json", &book_json(&["ghost"], &["ghost"]));
    std::fs::create_dir_all(dir.path.join("volumes")).unwrap();

    let error = expect_load_error(&dir.path);
    assert!(matches!(error, LoadError::MissingVolumeFile { .. }));
    let message = error.to_string();
    assert!(message.contains("ghost"), "{message}");
    assert!(message.contains("ghost.json"), "{message}");
}

#[test]
fn extra_volume_file_fails_fast() {
    let dir = TempDir::new("extra");
    write_json(&dir.path, "book.json", &book_json(&["a-vol"], &["a-vol"]));
    write_json(
        &dir.path,
        "volumes/a-vol.json",
        &volume_json(
            "a-vol",
            1,
            &[chapter_json(
                "a-vol-01",
                &[paragraph_json("a-vol-01-p0001", "秦伐魏")],
            )],
        ),
    );
    write_json(
        &dir.path,
        "volumes/stray.json",
        &volume_json("stray", 2, &[]),
    );

    let error = expect_load_error(&dir.path);
    assert!(matches!(error, LoadError::UnexpectedVolumeFile { .. }));
    assert!(error.to_string().contains("stray"), "{error}");
}

#[test]
fn volume_id_mismatch_is_rejected() {
    let dir = TempDir::new("mismatch");
    write_json(&dir.path, "book.json", &book_json(&["a-vol"], &["a-vol"]));
    write_json(
        &dir.path,
        "volumes/a-vol.json",
        &volume_json("other-id", 1, &[]),
    );

    let error = expect_load_error(&dir.path);
    assert!(matches!(error, LoadError::VolumeIdMismatch { .. }));
    assert!(error.to_string().contains("other-id"), "{error}");
}

#[test]
fn duplicate_paragraph_ids_across_volumes_name_both_locations() {
    let dir = TempDir::new("dup-paragraph");
    write_json(
        &dir.path,
        "book.json",
        &book_json(&["a-vol", "b-vol"], &["a-vol", "b-vol"]),
    );
    write_json(
        &dir.path,
        "volumes/a-vol.json",
        &volume_json(
            "a-vol",
            1,
            &[chapter_json(
                "a-vol-01",
                &[paragraph_json("dup-p0001", "秦伐魏")],
            )],
        ),
    );
    write_json(
        &dir.path,
        "volumes/b-vol.json",
        &volume_json(
            "b-vol",
            2,
            &[chapter_json(
                "b-vol-01",
                &[paragraph_json("dup-p0001", "齊伐燕")],
            )],
        ),
    );

    let error = expect_load_error(&dir.path);
    match &error {
        LoadError::DuplicateParagraphId {
            paragraph_id,
            first,
            second,
        } => {
            assert_eq!(paragraph_id, "dup-p0001");
            assert!(first.to_string().contains("a-vol"));
            assert!(second.to_string().contains("b-vol"));
        }
        other => panic!("unexpected error: {other}"),
    }
    assert!(error.to_string().contains("卷 b-vol"), "{error}");
}

#[test]
fn empty_or_blank_text_is_rejected_with_location() {
    for (label, text) in [("empty", ""), ("blank", "   ")] {
        let dir = TempDir::new(label);
        write_json(&dir.path, "book.json", &book_json(&["a-vol"], &["a-vol"]));
        write_json(
            &dir.path,
            "volumes/a-vol.json",
            &volume_json(
                "a-vol",
                1,
                &[chapter_json(
                    "a-vol-01",
                    &[paragraph_json("a-vol-01-p0001", text)],
                )],
            ),
        );

        let error = expect_load_error(&dir.path);
        assert!(matches!(error, LoadError::EmptyParagraphText { .. }));
        let message = error.to_string();
        assert!(message.contains("a-vol"), "{message}");
        assert!(message.contains("a-vol-01-p0001"), "{message}");
    }
}

#[test]
fn group_referencing_unknown_volume_is_rejected() {
    let dir = TempDir::new("group-ref");
    // group a references ghost-9, which volumeOrder never declares.
    write_json(
        &dir.path,
        "book.json",
        &book_json(&["a-vol"], &["a-vol", "ghost-9"]),
    );
    write_json(
        &dir.path,
        "volumes/a-vol.json",
        &volume_json(
            "a-vol",
            1,
            &[chapter_json(
                "a-vol-01",
                &[paragraph_json("a-vol-01-p0001", "秦伐魏")],
            )],
        ),
    );

    let error = expect_load_error(&dir.path);
    assert!(matches!(
        error,
        LoadError::GroupReferencesUnknownVolume { .. }
    ));
    assert!(error.to_string().contains("ghost-9"), "{error}");
}

#[test]
fn book_without_volume_order_is_rejected() {
    let dir = TempDir::new("no-order");
    write_json(&dir.path, "book.json", &book_json(&[], &[]));
    std::fs::create_dir_all(dir.path.join("volumes")).unwrap();

    let error = expect_load_error(&dir.path);
    assert!(matches!(error, LoadError::EmptyVolumeOrder { .. }));
}

#[test]
fn malformed_json_names_the_file() {
    let dir = TempDir::new("bad-json");
    std::fs::create_dir_all(dir.path.join("volumes")).unwrap();
    std::fs::write(dir.path.join("book.json"), b"{ not json").unwrap();

    let error = expect_load_error(&dir.path);
    assert!(matches!(error, LoadError::Json { .. }));
    assert!(error.to_string().contains("book.json"), "{error}");
}

#[test]
fn chronology_form_loads_and_traverses_without_panicking() {
    // 編年體 is only reserved for later books: it must load and navigate, not crash.
    let dir = TempDir::new("chronology");
    let mut book = book_json(&["a-vol"], &["a-vol"]);
    book["form"] = json!("編年體");
    write_json(&dir.path, "book.json", &book);
    write_json(
        &dir.path,
        "volumes/a-vol.json",
        &volume_json(
            "a-vol",
            1,
            &[
                chapter_json(
                    "a-vol-01",
                    &[paragraph_json("a-vol-01-p0001", "元年，秦伐魏")],
                ),
                chapter_json(
                    "a-vol-02",
                    &[paragraph_json("a-vol-02-p0001", "二年，齊伐燕")],
                ),
            ],
        ),
    );

    let data = Book::load(&dir.path).expect("chronology book loads");
    assert_eq!(data.axis().as_str(), "編年體");
    let first = data.first_position().unwrap();
    assert_eq!(data.next_chapter(first).unwrap().chapter, 1);
    assert_eq!(data.next_paragraph(first).unwrap().paragraph, 0);
    assert_eq!(data.next_paragraph(first).unwrap().chapter, 1);
    assert_eq!(data.paragraph_count(), 2);
}

#[test]
fn annotations_and_glossary_accept_both_empty_and_populated() {
    let dir = TempDir::new("annotations");
    write_json(&dir.path, "book.json", &book_json(&["a-vol"], &["a-vol"]));
    let empty = paragraph_json("a-vol-01-p0001", "衛鞅亡魏入秦");
    let populated = json!({
        "id": "a-vol-01-p0002",
        "text": "蘇秦說齊王",
        "translation": "苏秦游说齐王",
        "annotations": [{ "source": "鮑彪", "text": "蘇秦，洛陽人" }],
        "glossary": [{ "term": "蘇秦", "pinyin": "su qin", "gloss": "纵横家" }]
    });
    let anonymous_source = json!({ "id": "a-vol-01-p0003", "text": "秦伐魏", "annotations": [{ "text": "無出處之注" }] });
    write_json(
        &dir.path,
        "volumes/a-vol.json",
        &volume_json(
            "a-vol",
            1,
            &[chapter_json(
                "a-vol-01",
                &[empty, populated, anonymous_source],
            )],
        ),
    );

    let data = Book::load(&dir.path).expect("fixture loads");
    let paragraph = data.paragraph(zgc_core::Position::new(0, 0, 0)).unwrap();
    assert!(paragraph.annotations.is_empty());
    assert!(paragraph.glossary.is_empty());

    let populated = data.paragraph(zgc_core::Position::new(0, 0, 1)).unwrap();
    assert_eq!(populated.annotations.len(), 1);
    assert_eq!(populated.annotations[0].source.as_deref(), Some("鮑彪"));
    assert_eq!(populated.glossary[0].term, "蘇秦");

    let anonymous = data.paragraph(zgc_core::Position::new(0, 0, 2)).unwrap();
    assert!(
        anonymous.annotations[0].source.is_none(),
        "source is optional"
    );
}

#[test]
fn volume_listed_by_two_groups_fails_at_load() {
    // Shape of the 宋衛 defect: one volume, two groups. The loader itself — not a
    // later rendering step — must refuse it.
    let dir = TempDir::new("multi-group");
    let book = json!({
        "schemaVersion": 1,
        "id": "toy",
        "title": "玩具",
        "form": "國別體",
        "groups": [
            { "id": "a", "title": "甲", "volumes": ["a-vol"] },
            { "id": "b", "title": "乙", "volumes": ["a-vol"] }
        ],
        "volumeOrder": ["a-vol"]
    });
    write_json(&dir.path, "book.json", &book);
    write_json(
        &dir.path,
        "volumes/a-vol.json",
        &volume_json(
            "a-vol",
            1,
            &[chapter_json(
                "a-vol-01",
                &[paragraph_json("a-vol-01-p0001", "秦伐魏")],
            )],
        ),
    );

    let error = expect_load_error(&dir.path);
    assert!(matches!(error, LoadError::VolumeInMultipleGroups { .. }));
    let message = error.to_string();
    assert!(message.contains("a-vol"), "{message}");
    assert!(message.contains("a, b"), "{message}");
}
