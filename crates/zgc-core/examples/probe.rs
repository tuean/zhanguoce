//! Real-data probe: load the corpus, build the index, run sample searches and
//! prove the cross-chapter / cross-volume jumps with concrete paragraph ids.
//!
//! Run: `cargo run --release --example probe`
//! Override the data directory with `ZGC_DATA_DIR`.

use std::path::PathBuf;
use std::time::Instant;

use zgc_core::{Book, BookData, FieldSource, Hit, Position, Scope, SearchIndex};

fn data_dir() -> PathBuf {
    match std::env::var("ZGC_DATA_DIR") {
        Ok(path) => PathBuf::from(path),
        Err(_) => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/zhanguoce"),
    }
}

fn paragraph_label(data: &BookData, position: Position) -> String {
    let volume = data.volume(position.volume).unwrap();
    let chapter = data.chapter(position.volume, position.chapter).unwrap();
    let paragraph = data.paragraph(position).unwrap();
    format!("{}/{}/{}", volume.id, chapter.id, paragraph.id)
}

fn print_hits(query: &str, field: FieldSource, hits: &[Hit], printed: usize, elapsed_ms: f64) {
    println!(
        "  query “{query}” 字段={field} 命中 {} 处（出现次数；打印前 {printed} 处，耗时 {elapsed_ms:.3} ms）",
        hits.len()
    );
    for hit in hits.iter().take(printed) {
        let index = match hit.field_index {
            Some(index) => format!("#{index}"),
            None => String::new(),
        };
        println!(
            "    - {}/{}/{} [{}{}] char={} byte={}  {}",
            hit.volume_id,
            hit.chapter_id,
            hit.paragraph_id,
            hit.field,
            index,
            hit.char_offset,
            hit.byte_offset,
            hit.snippet
        );
    }
}

fn main() {
    let dir = data_dir();
    println!("data dir: {}", dir.display());

    let load_start = Instant::now();
    let data = Book::load(&dir).expect("real data must load");
    let load_ms = load_start.elapsed().as_secs_f64() * 1000.0;

    println!(
        "book: {}（{}） form={} axis={}",
        data.book().title,
        data.book().id,
        data.book().form,
        data.axis().as_str()
    );
    println!(
        "load: {} 卷 / {} 组 / {} 章 / {} 段，耗时 {:.3} ms",
        data.volume_count(),
        data.group_count(),
        data.chapter_count(),
        data.paragraph_count(),
        load_ms
    );

    let index_start = Instant::now();
    let index = SearchIndex::build(&data);
    let index_ms = index_start.elapsed().as_secs_f64() * 1000.0;
    println!(
        "index: {} 个 distinct bigram / {} 次 bigram 出现，建索引耗时 {:.3} ms",
        index.bigram_count(),
        index.posting_count(),
        index_ms
    );

    println!("\n== 检索样例（全书） ==");
    let queries: [(&str, FieldSource); 3] = [
        ("衞鞅", FieldSource::Text),
        ("蘇秦", FieldSource::Text),
        ("秦國", FieldSource::Text),
    ];
    for (query, field) in queries {
        let start = Instant::now();
        let hits = index.search(&data, query, &Scope::All);
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        print_hits(query, field, &hits, 3, elapsed);
    }

    let start = Instant::now();
    let translation_hits = index.search(&data, "秦国", &Scope::All);
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
    print_hits(
        "秦国",
        FieldSource::Translation,
        &translation_hits,
        3,
        elapsed,
    );

    println!("\n== 范围检索（限定国策） ==");
    for group in ["qin", "qi"] {
        let hits = index.search_limited(&data, "秦王", &Scope::group(group), usize::MAX);
        let volumes: Vec<&str> = data
            .group_volumes(group)
            .map(|volume| volume.id.as_str())
            .collect();
        let all_inside = hits
            .iter()
            .all(|hit| volumes.contains(&hit.volume_id.as_str()));
        println!(
            "  group {group}（{} 卷）: “秦王” {} 条，全部落在本组卷内={all_inside}",
            volumes.len(),
            hits.len()
        );
    }

    println!("\n== 跨章 / 跨卷跳转（真实 id） ==");
    let qin1 = data.volume_index("qin-1").expect("qin-1");
    let last_chapter = data.volume(qin1).unwrap().chapters.len() - 1;
    let last_paragraph = data.chapter(qin1, last_chapter).unwrap().paragraphs.len() - 1;
    let last_of_qin1 = Position::new(qin1, last_chapter, last_paragraph);
    println!("  qin-1 末章末段: {}", paragraph_label(&data, last_of_qin1));
    println!(
        "    next_paragraph → {}（跨卷）",
        paragraph_label(&data, data.next_paragraph(last_of_qin1).unwrap())
    );
    println!(
        "    next_volume    → {}（跨卷，对齐首段）",
        paragraph_label(&data, data.next_volume(last_of_qin1).unwrap())
    );

    let qin2_first = Position::new(data.volume_index("qin-2").unwrap(), 0, 0);
    println!("  qin-2 首章首段: {}", paragraph_label(&data, qin2_first));
    println!(
        "    prev_paragraph → {}（跨卷，落到上一卷末段）",
        paragraph_label(&data, data.prev_paragraph(qin2_first).unwrap())
    );
    println!(
        "    prev_volume    → {}（跨卷，对齐首段）",
        paragraph_label(&data, data.prev_volume(qin2_first).unwrap())
    );

    let qin1_first = Position::new(qin1, 0, 0);
    println!("  qin-1 首章首段: {}", paragraph_label(&data, qin1_first));
    println!(
        "    next_chapter   → {}（跨章，对齐首段）",
        paragraph_label(&data, data.next_chapter(qin1_first).unwrap())
    );
    println!(
        "    prev_chapter   → {}（跨卷，对齐首段）",
        paragraph_label(&data, data.prev_chapter(qin1_first).unwrap())
    );

    println!("\n== 遍历顺序（秦策五卷） ==");
    let qin_volumes: Vec<&str> = data
        .group_volumes("qin")
        .map(|volume| volume.id.as_str())
        .collect();
    println!("  group qin → {}", qin_volumes.join(" → "));
    let sample_volume = data.volume_by_id("qin-1").unwrap();
    println!(
        "  volume qin-1 → 前 5 章: {}",
        sample_volume
            .chapters
            .iter()
            .take(5)
            .map(|chapter| chapter.id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
}
