//! Integration tests for the entity layer against the real corpus.
//!
//! The numbers here are acceptance numbers: 176 people + 98 places = 274
//! entities and 10,685 mounted occurrences. They also re-verify the mount
//! itself — every span must slice back to its declared form, in characters.

use std::collections::HashMap;
use std::path::PathBuf;

use zgc_core::{Book, BookData, Entities, EntityKind};

fn data_dir() -> PathBuf {
    std::env::var_os("ZGC_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/zhanguoce"))
}

fn load() -> (BookData, Entities) {
    let dir = data_dir();
    let data = Book::load(&dir).expect("real data must load");
    let entities = Entities::load(&dir, &data).expect("real entities must load");
    (data, entities)
}

fn texts(data: &BookData) -> HashMap<&str, &str> {
    data.iter_paragraphs()
        .map(|(_, paragraph)| (paragraph.id.as_str(), paragraph.text.as_str()))
        .collect()
}

#[test]
fn real_entity_files_load_with_the_expected_shape() {
    let (_, entities) = load();
    assert_eq!(entities.len(), 274, "176 people + 98 places");
    assert_eq!(entities.of_kind(EntityKind::Person).count(), 176);
    assert_eq!(entities.of_kind(EntityKind::Place).count(), 98);
    assert_eq!(entities.occurrence_total(), 10_685);
    assert_eq!(entities.covered_paragraph_count(), 4_195);
}

#[test]
fn every_occurrence_and_every_render_span_slices_back_to_its_form() {
    let (data, entities) = load();
    let texts = texts(&data);

    for entity in entities.entities() {
        for occurrence in &entity.occurrences {
            let text = texts
                .get(occurrence.paragraph_id.as_str())
                .unwrap_or_else(|| panic!("unknown paragraph {}", occurrence.paragraph_id));
            let chars: Vec<char> = text.chars().collect();
            let end = occurrence.offset + occurrence.len;
            assert!(
                end <= chars.len(),
                "{} out of range in {}",
                entity.id,
                occurrence.paragraph_id
            );
            let slice: String = chars[occurrence.offset..end].iter().collect();
            assert_eq!(
                slice, occurrence.form,
                "{} in {}",
                entity.id, occurrence.paragraph_id
            );
        }
    }

    // Render spans are disjoint and inside the paragraph, for all 4,195 paragraphs.
    let mut checked = 0usize;
    for (_, paragraph) in data.iter_paragraphs() {
        let char_count = paragraph.text.chars().count();
        let mut previous_end = 0usize;
        for span in entities.spans_in(&paragraph.id) {
            assert!(
                span.offset >= previous_end,
                "overlap left in {}",
                paragraph.id
            );
            assert!(
                span.offset + span.len <= char_count,
                "span past end in {}",
                paragraph.id
            );
            let slice: String = paragraph
                .text
                .chars()
                .skip(span.offset)
                .take(span.len)
                .collect();
            assert!(
                slice == span.entity.name
                    || span.entity.aliases.iter().any(|alias| alias == &slice),
                "span glyphs {:?} are not a declared form of {}",
                slice,
                span.entity.id
            );
            previous_end = span.offset + span.len;
            checked += 1;
        }
    }
    assert!(
        checked > 4_000,
        "expected thousands of render spans, saw {checked}"
    );
}

#[test]
fn known_entities_answer_by_id_with_their_occurrence_counts() {
    let (_, entities) = load();

    let wei_yang = entities.entity("person-wei-yang").expect("person-wei-yang");
    assert_eq!(wei_yang.name, "衞鞅");
    assert_eq!(wei_yang.kind, EntityKind::Person);
    assert_eq!(wei_yang.occurrence_count(), 23);
    let first = &wei_yang.occurrences[0];
    assert_eq!(first.paragraph_id, "qin-1-01-p0001");
    assert_eq!((first.offset, first.len), (0, 2));
    assert_eq!(first.form, "衞鞅");

    assert_eq!(
        entities
            .entity("person-su-qin")
            .map(|e| e.occurrence_count()),
        Some(133)
    );
    assert_eq!(
        entities
            .entity("person-qin-zhaowang")
            .map(|e| e.occurrence_count()),
        Some(7)
    );
    assert_eq!(
        entities.entity("place-qin").map(|e| e.occurrence_count()),
        Some(1_910)
    );
    assert_eq!(
        entities
            .entity("place-dongzhou")
            .map(|e| e.occurrence_count()),
        Some(36)
    );

    // Unknown ids are a plain None — the app turns that into a 404.
    assert!(entities.entity("person-ghost").is_none());
    assert!(entities.entity("").is_none());
}

#[test]
fn real_overlaps_are_resolved_and_reported() {
    let (data, entities) = load();

    let mut raw_total = 0usize;
    let mut dropped = 0usize;
    let mut dropped_paragraphs = 0usize;
    for (_, paragraph) in data.iter_paragraphs() {
        let raw = entities.occurrences_in(&paragraph.id).count();
        let kept = entities.spans_in(&paragraph.id).count();
        raw_total += raw;
        if kept < raw {
            dropped_paragraphs += 1;
            dropped += raw - kept;
        }
    }
    assert_eq!(raw_total, 10_685);
    assert_eq!(dropped_paragraphs, 38, "paragraphs whose raw spans overlap");
    assert_eq!(entities.overlapping_paragraph_count(), dropped_paragraphs);
    assert!(dropped > 0);
    assert!(
        dropped < 60,
        "the overlap rule must not eat the corpus: dropped {dropped}"
    );

    // A concrete collision: 東周 and 東周君 start at the same glyph.
    let east_zhou = entities.entity("place-dongzhou").expect("place-dongzhou");
    let collision = east_zhou
        .occurrences
        .iter()
        .find(|occurrence| {
            let spans: Vec<_> = entities
                .spans_in(&occurrence.paragraph_id)
                .filter(|span| span.offset == occurrence.offset)
                .collect();
            spans.len() == 1 && spans[0].entity.id != "place-dongzhou"
        })
        .map(|occurrence| {
            entities
                .spans_in(&occurrence.paragraph_id)
                .find(|span| span.offset == occurrence.offset)
                .map(|span| span.entity.id.clone())
                .unwrap_or_default()
        });
    assert_eq!(
        collision.as_deref(),
        Some("person-dongzhou-jun"),
        "at the same offset the longer 東周君 must win over 東周"
    );
}

#[test]
fn loading_twice_is_byte_for_byte_deterministic() {
    let (_, first) = load();
    let (_, second) = load();
    let snapshot = |entities: &Entities| {
        let mut rows: Vec<(String, usize, Vec<(String, usize, usize, String)>)> = entities
            .entities()
            .map(|entity| {
                (
                    entity.id.clone(),
                    entity.kind as usize,
                    entity
                        .occurrences
                        .iter()
                        .map(|occurrence| {
                            (
                                occurrence.paragraph_id.clone(),
                                occurrence.offset,
                                occurrence.len,
                                occurrence.form.clone(),
                            )
                        })
                        .collect(),
                )
            })
            .collect();
        rows.sort();
        rows
    };
    assert_eq!(snapshot(&first), snapshot(&second));
}
