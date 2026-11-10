//! Integration tests against the real corpus in `data/zhanguoce`.
//!
//! Acceptance numbers (33 volumes / 11 groups / 497 chapters / 6663 paragraphs)
//! are asserted here, so a data-layer regression or an accidental re-sort is
//! caught by `cargo test`. Group membership is asserted per volume too: every
//! volume must belong to exactly one group (宋衛 is one volume of the 宋衛策 group).

use std::path::PathBuf;

use serde_json::Value;
use zgc_core::{Book, BookData, FieldSource, Hit, Position, Scope, SearchIndex};

fn data_dir() -> PathBuf {
    // ZGC_DATA_DIR lets the same test run against a mutated copy (used to prove
    // the group-membership assertion really fires on a duplicated volume).
    std::env::var_os("ZGC_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/zhanguoce"))
}

fn load() -> BookData {
    Book::load(&data_dir()).expect("real data must load")
}

fn hit_text<'a>(book: &'a BookData, hit: &Hit) -> &'a str {
    for (_, paragraph) in book.iter_paragraphs() {
        if paragraph.id == hit.paragraph_id {
            return match hit.field {
                FieldSource::Text => paragraph.text.as_str(),
                FieldSource::Translation => paragraph
                    .translation
                    .as_deref()
                    .expect("translation hit must come from a translation"),
                FieldSource::Annotation => {
                    &paragraph.annotations[hit.field_index.expect("annotation index")].text
                }
                FieldSource::Glossary => {
                    &paragraph.glossary[hit.field_index.expect("glossary index")].term
                }
            };
        }
    }
    panic!("hit paragraph {} not found", hit.paragraph_id);
}

fn assert_hit_points_at_query(book: &BookData, hit: &Hit, query: &str) {
    let text = hit_text(book, hit);
    assert_eq!(
        text[..hit.byte_offset].chars().count(),
        hit.char_offset,
        "char/byte offsets disagree for {}",
        hit.paragraph_id
    );
    let matched: String = text[hit.byte_offset..]
        .chars()
        .take(query.chars().count())
        .collect();
    assert_eq!(
        matched, query,
        "hit {} must start at the query",
        hit.paragraph_id
    );
}

#[test]
fn real_data_counts_match_acceptance_numbers() {
    let data = load();
    println!(
        "loaded {} volumes / {} groups / {} chapters / {} paragraphs",
        data.volume_count(),
        data.group_count(),
        data.chapter_count(),
        data.paragraph_count()
    );
    assert_eq!(data.volume_count(), 33, "volumes");
    assert_eq!(data.group_count(), 11, "groups");
    assert_eq!(data.chapter_count(), 497, "chapters");
    assert_eq!(data.paragraph_count(), 6663, "paragraphs");
    assert_eq!(data.axis().as_str(), "國別體");

    // Every volume must belong to exactly one group. 宋衛 was once listed by both
    // `wei` and `weiwei` (12 groups); the data side is fixed, but without this
    // per-volume check the kernel would not notice the same defect coming back.
    let mut memberships: Vec<Vec<String>> = vec![Vec::new(); data.volume_count()];
    for group in data.groups() {
        assert!(
            !group.volumes.is_empty(),
            "group {} lists no volumes",
            group.id
        );
        for volume_id in &group.volumes {
            let index = data
                .volume_index(volume_id)
                .unwrap_or_else(|| panic!("group {} lists unknown volume {}", group.id, volume_id));
            memberships[index].push(group.id.clone());
        }
    }

    let mut distribution: std::collections::BTreeMap<usize, usize> =
        std::collections::BTreeMap::new();
    for (index, groups) in memberships.iter().enumerate() {
        let volume_id = &data.volumes()[index].id;
        assert_eq!(
            groups.len(),
            1,
            "卷 {volume_id} 必须恰好属于 1 个 group，实际属于 {groups:?}"
        );
        *distribution.entry(groups.len()).or_insert(0) += 1;
    }
    println!(
        "卷归属分布: {} 个卷，按“所属 group 个数”统计 {:?}",
        memberships.len(),
        distribution
    );
    for (index, groups) in memberships.iter().enumerate() {
        println!("  {} → {}", data.volumes()[index].id, groups[0]);
    }
}

#[test]
fn volume_order_comes_from_book_json_not_file_names() {
    let dir = data_dir();
    let raw: Value =
        serde_json::from_slice(&std::fs::read(dir.join("book.json")).unwrap()).unwrap();
    let declared: Vec<String> = raw["volumeOrder"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect();
    let data = load();
    let loaded: Vec<String> = data
        .volumes()
        .iter()
        .map(|volume| volume.id.clone())
        .collect();
    assert_eq!(loaded, declared, "loader must preserve volumeOrder");
    assert_eq!(loaded[0], "dongzhou");
    assert_eq!(loaded[2], "qin-1");
    assert_eq!(loaded[32], "zhongshan");
}

#[test]
fn loading_and_indexing_are_deterministic() {
    let first = load();
    let second = load();

    let ids = |data: &BookData| -> Vec<String> {
        data.volumes()
            .iter()
            .map(|volume| volume.id.clone())
            .chain(
                data.iter_paragraphs()
                    .map(|(_, paragraph)| paragraph.id.clone()),
            )
            .collect()
    };
    assert_eq!(ids(&first), ids(&second));

    let index_a = SearchIndex::build(&first);
    let index_b = SearchIndex::build(&second);
    assert_eq!(index_a.bigram_count(), index_b.bigram_count());
    assert_eq!(index_a.posting_count(), index_b.posting_count());
    assert_eq!(
        index_a.search_limited(&first, "秦", &Scope::All, 100),
        index_b.search_limited(&second, "秦", &Scope::All, 100)
    );
}

#[test]
fn paragraph_ids_encode_volume_chapter_and_are_unique() {
    let data = load();
    let mut seen = std::collections::HashSet::new();
    for (position, paragraph) in data.iter_paragraphs() {
        let volume = data.volume(position.volume).unwrap();
        let chapter = data.chapter(position.volume, position.chapter).unwrap();
        assert!(
            paragraph.id.starts_with(&chapter.id),
            "{} must be prefixed by chapter {}",
            paragraph.id,
            chapter.id
        );
        assert!(
            chapter.id.starts_with(&volume.id),
            "{} must be prefixed by volume {}",
            chapter.id,
            volume.id
        );
        assert!(!paragraph.text.trim().is_empty());
        assert!(
            seen.insert(paragraph.id.clone()),
            "duplicate {}",
            paragraph.id
        );
    }
    assert_eq!(seen.len(), 6663);
}

#[test]
fn navigation_crosses_chapter_and_volume_boundaries_on_real_data() {
    let data = load();
    let qin1 = data.volume_index("qin-1").expect("qin-1 exists");
    let qin2 = data.volume_index("qin-2").expect("qin-2 exists");

    // Last paragraph of qin-1 → first paragraph of qin-2.
    let last_qin1_chapter = data.volume(qin1).unwrap().chapters.len() - 1;
    let last_qin1_paragraph = data
        .chapter(qin1, last_qin1_chapter)
        .unwrap()
        .paragraphs
        .len()
        - 1;
    let last_of_qin1 = Position::new(qin1, last_qin1_chapter, last_qin1_paragraph);
    let next = data
        .next_paragraph(last_of_qin1)
        .expect("there is a next volume");
    assert_eq!(next, Position::new(qin2, 0, 0));
    let next_paragraph = data.paragraph(next).unwrap();
    assert_eq!(next_paragraph.id, "qin-2-01-p0001");
    assert_eq!(data.next_volume(last_of_qin1), Some(next));

    // ... and back again, from qin-2's very first paragraph.
    let back = data
        .prev_paragraph(Position::new(qin2, 0, 0))
        .expect("there is a previous paragraph");
    assert_eq!(back, last_of_qin1);
    assert_eq!(
        data.paragraph(back).unwrap().id,
        data.paragraph(last_of_qin1).unwrap().id
    );
    assert_eq!(
        data.prev_volume(Position::new(qin2, 0, 0)),
        data.volume_start(qin1)
    );

    // Chapter-level hop inside qin-1: chapter 1 → chapter 2, aligned to first paragraph.
    let next_chapter = data
        .next_chapter(Position::new(qin1, 0, 0))
        .expect("qin-1 has more chapters");
    assert_eq!(next_chapter, Position::new(qin1, 1, 0));
    let next_chapter_id = &data.chapter(qin1, 1).unwrap().id;
    assert!(data
        .paragraph(next_chapter)
        .unwrap()
        .id
        .starts_with(next_chapter_id));

    // Chapter-level hop backwards crosses into the previous volume.
    let prev_chapter = data
        .prev_chapter(Position::new(qin1, 0, 0))
        .expect("qin-1 has a previous chapter");
    assert!(prev_chapter.volume < qin1);
    assert_eq!(
        prev_chapter.paragraph, 0,
        "previous chapter aligns to its first paragraph"
    );
    let prev_volume_id = &data.volume(prev_chapter.volume).unwrap().id;
    assert!(data
        .paragraph(prev_chapter)
        .unwrap()
        .id
        .starts_with(prev_volume_id.as_str()));

    // Full walk visits every paragraph exactly once, in reading order.
    let mut cursor = data.first_position();
    let mut walked = 0;
    while let Some(position) = cursor {
        cursor = data.next_paragraph(position);
        walked += 1;
    }
    assert_eq!(walked, data.paragraph_count());
    assert!(data.next_paragraph(data.last_position().unwrap()).is_none());
    assert!(data
        .prev_paragraph(data.first_position().unwrap())
        .is_none());
}

#[test]
fn real_searches_hit_text_translation_and_respect_scope() {
    let data = load();
    let index = SearchIndex::build(&data);

    // 衞鞅 (note the 衞 variant used by the 底本): the opening of 秦策一.
    let wei_yang = index.search_limited(&data, "衞鞅", &Scope::All, 20);
    assert!(!wei_yang.is_empty(), "衞鞅 must be found");
    assert_eq!(wei_yang[0].volume_id, "qin-1");
    assert_eq!(wei_yang[0].field, FieldSource::Text);
    assert_eq!(wei_yang[0].paragraph_id, "qin-1-01-p0001");
    assert_eq!(wei_yang[0].char_offset, 0);
    for hit in &wei_yang {
        assert_hit_points_at_query(&data, hit, "衞鞅");
    }

    // 蘇秦: many paragraphs, at least in 齊策 and 楚策.
    let su_qin = index.search_limited(&data, "蘇秦", &Scope::All, 30);
    assert!(su_qin.len() > 5, "expected several 蘇秦 hits");
    for hit in &su_qin {
        assert_hit_points_at_query(&data, hit, "蘇秦");
        assert!(hit.snippet.contains("蘇秦"), "{}", hit.snippet);
    }

    // Translation (simplified) is a separate field: 秦国 is not in the 繁体 text.
    let qin_country = index.search_limited(&data, "秦国", &Scope::All, 30);
    assert!(
        !qin_country.is_empty(),
        "白话 translations must be searchable"
    );
    assert!(qin_country
        .iter()
        .any(|hit| hit.field == FieldSource::Translation));
    for hit in qin_country
        .iter()
        .filter(|hit| hit.field == FieldSource::Translation)
    {
        assert_hit_points_at_query(&data, hit, "秦国");
    }

    // Scope: one group / one volume.
    let qin_group = index.search_limited(&data, "秦王", &Scope::group("qin"), 50);
    assert!(!qin_group.is_empty());
    let qin_volumes: Vec<&str> = data
        .group_volumes("qin")
        .map(|volume| volume.id.as_str())
        .collect();
    assert!(qin_group
        .iter()
        .all(|hit| qin_volumes.contains(&hit.volume_id.as_str())));
    let qi1 = index.search_limited(&data, "秦王", &Scope::volume("qi-1"), 50);
    assert!(!qi1.is_empty());
    assert!(qi1.iter().all(|hit| hit.volume_id == "qi-1"));
    assert!(index
        .search(&data, "秦王", &Scope::group("no-such-group"))
        .is_empty());
}

#[test]
fn annotation_and_glossary_are_indexed_when_present_and_empty_today_is_fine() {
    // M1b writes 注疏 concurrently: this test must pass for both states.
    let data = load();
    let index = SearchIndex::build(&data);

    let mut annotation_checks = 0;
    let mut glossary_checks = 0;
    for (position, paragraph) in data.iter_paragraphs() {
        let volume_id = data.volume(position.volume).unwrap().id.clone();
        for (index_in_paragraph, annotation) in paragraph.annotations.iter().enumerate() {
            let needle: String = annotation.text.chars().take(2).collect();
            if needle.chars().count() < 2 {
                continue;
            }
            let hits = index.search(&data, &needle, &Scope::volume(volume_id.clone()));
            assert!(
                hits.iter().any(|hit| hit.field == FieldSource::Annotation
                    && hit.paragraph_id == paragraph.id
                    && hit.field_index == Some(index_in_paragraph)),
                "annotation {} of {} must be searchable",
                index_in_paragraph,
                paragraph.id
            );
            annotation_checks += 1;
        }
        for (index_in_paragraph, entry) in paragraph.glossary.iter().enumerate() {
            let needle: String = entry.term.chars().take(2).collect();
            if needle.chars().count() < 2 {
                continue;
            }
            let hits = index.search(&data, &needle, &Scope::volume(volume_id.clone()));
            assert!(
                hits.iter().any(|hit| hit.field == FieldSource::Glossary
                    && hit.paragraph_id == paragraph.id
                    && hit.field_index == Some(index_in_paragraph)),
                "glossary {} of {} must be searchable",
                index_in_paragraph,
                paragraph.id
            );
            glossary_checks += 1;
        }
    }
    println!(
        "annotation checks: {annotation_checks}, glossary checks: {glossary_checks} (0 is expected while M1b is still writing)"
    );

    if annotation_checks == 0 {
        // No commentary yet: a commentator name must not appear out of thin air.
        assert!(index.search(&data, "鮑彪", &Scope::All).is_empty());
    }
}
