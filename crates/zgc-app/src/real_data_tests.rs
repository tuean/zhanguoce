//! Rendering checks against the real corpus in data/zhanguoce.
//!
//! Each test degrades to a printed skip when the corpus is absent, so the
//! workspace suite still passes on a checkout without data.

use std::path::PathBuf;

use zgc_core::{Book, ChapterYears, Entities, Events, Strategy};

use crate::app::App;
use crate::http::Request;

fn real_app() -> Option<App> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/zhanguoce");
    if !dir.join("book.json").is_file() {
        eprintln!("skip：找不到 {}", dir.display());
        return None;
    }
    let data = Book::load(&dir).expect("real corpus must load");
    Some(App::new(data))
}

/// Units of the four counts stated on the table-of-contents page, in order.
const STATS_UNITS: [&str; 4] = ["國", "卷", "章", "段"];

/// Pull the stats line out of the TOC markup (the text inside the stats
/// paragraph), independent of the exact values it holds.
fn stats_line(html: &str) -> Option<&str> {
    const OPEN: &str = "<p class=\"stats\">";
    let start = html.find(OPEN)? + OPEN.len();
    let rest = &html[start..];
    Some(&rest[..rest.find("</p>")?])
}

/// The number that precedes a unit inside the stats line ("11 國" → 11).
fn count_of(stats: &str, unit: &str) -> Option<usize> {
    stats.split('·').find_map(|piece| {
        let piece = piece.trim();
        if !piece.ends_with(unit) {
            return None;
        }
        let digits: String = piece
            .chars()
            .take_while(|character| character.is_ascii_digit())
            .collect();
        digits.parse().ok()
    })
}

/// Compare the four counts one item at a time, so a failure names the broken
/// item instead of complaining about a whole Chinese sentence. A count change
/// is then a one-line, self-explaining failure rather than noise.
fn check_stats(stats: &str, expected: [usize; 4]) -> Result<(), String> {
    for (unit, expected) in STATS_UNITS.iter().zip(expected) {
        let actual = count_of(stats, unit)
            .ok_or_else(|| format!("stats 行缺少「N {unit}」这一项：{stats}"))?;
        if actual != expected {
            return Err(format!(
                "{unit} 数不符：页面写 {actual}，期望 {expected}（整行：{stats}）"
            ));
        }
    }
    Ok(())
}

#[test]
fn table_of_contents_matches_the_acceptance_counts_and_volume_order() {
    let Some(app) = real_app() else { return };
    let html = app.handle(&Request::get("/")).body;

    let stats = stats_line(&html).expect("目录页必须有 stats 行");
    check_stats(stats, [11, 33, 497, 6663])
        .unwrap_or_else(|error| panic!("目录页计数不对：{error}"));

    // Cross-check against the loaded structure, so the numbers above are not
    // just the test agreeing with a hard-coded copy of itself.
    let data = app.data();
    let structural = [
        data.group_count(),
        data.volume_count(),
        data.chapter_count(),
        data.paragraph_count(),
    ];
    for (unit, expected) in STATS_UNITS.iter().zip(structural) {
        assert_eq!(
            count_of(stats, unit),
            Some(expected),
            "{unit} 数应与 BookData 一致（stats 行：{stats}）"
        );
    }

    // Every volume heading and every chapter must be listed exactly once
    // (宋衛并卷后不再出现同一卷被两个 group 各展开一次).
    let volume_links = html.matches("<h3><a href=\"/read?p=").count();
    let chapter_links = html.matches("<li><a href=\"/read?p=").count();
    assert_eq!(
        volume_links,
        data.volume_count(),
        "卷标题链接数应与卷数一致"
    );
    assert_eq!(chapter_links, data.chapter_count(), "章链接数应与章数一致");

    let tree = &html[html.find("<nav class=\"tree\" data-arrow-step").expect("tree")..];
    let dongzhou = tree
        .find("/read?p=dongzhou-")
        .expect("first volume is 東周");
    let xizhou = tree.find("/read?p=xizhou-").expect("then 西周");
    let qin = tree.find("/read?p=qin-1-").expect("then 秦策一");
    assert!(
        dongzhou < xizhou && xizhou < qin,
        "volumeOrder must drive the tree"
    );
}

#[test]
fn stats_check_reacts_to_a_stale_group_count_and_names_the_unit() {
    // 反证：页面若仍写 12 國（旧数据形态），新断言必须失败，且失败信息指出是「國」。
    let stale_markup = "<p class=\"stats\">12 國 · 33 卷 · 497 章 · 6663 段</p>";
    let stats = stats_line(stale_markup).expect("stats_line 应能从真实标记里取出 stats 行");
    assert_eq!(stats, "12 國 · 33 卷 · 497 章 · 6663 段");

    let error = check_stats(stats, [11, 33, 497, 6663]).expect_err("12 國 必须判为不符");
    assert!(error.contains("國"), "失败信息应指出是哪一项：{error}");
    assert!(
        error.contains("12") && error.contains("11"),
        "失败信息应同时给出实际值与期望值：{error}"
    );

    // 同一份标记把「國」改对之后必须通过，确保断言不是永远失败的死逻辑。
    let fixed = stale_markup.replace("12 國", "11 國");
    let fixed_stats = stats_line(&fixed).expect("stats_line");
    assert_eq!(check_stats(fixed_stats, [11, 33, 497, 6663]), Ok(()));

    // 缺项也要报出缺失的单位，而不是静默通过。
    assert!(check_stats("11 國 · 33 卷", [11, 33, 497, 6663])
        .expect_err("缺章/段必须失败")
        .contains("章"));
}

#[test]
fn reading_page_renders_original_translation_and_annotations() {
    let Some(app) = real_app() else { return };
    let response = app.handle(&Request::get("/read?p=qin-1-01-p0001"));
    assert_eq!(response.status, 200);
    let html = response.body;
    assert!(
        html.contains("衞鞅亡魏入秦，孝公以爲相，封之于商，號曰商君。"),
        "原文"
    );
    assert!(html.contains("卫鞅从魏国逃亡到秦国"), "白话");
    assert!(html.contains("<span class=\"src\">"), "注疏来源");
    assert!(
        !html.contains("暫無注疏"),
        "this paragraph does have commentary"
    );
    assert!(html.contains("第 1 段"));
    assert!(html.contains("秦策一"));
}

#[test]
fn paragraphs_without_commentary_say_so_instead_of_rendering_nothing() {
    let Some(app) = real_app() else { return };
    // Search the real corpus for a paragraph with no annotations and no
    // translation at all, then check both placeholders render.
    let mut target = None;
    for (_, paragraph) in app.data().iter_paragraphs() {
        if paragraph.annotations.is_empty() && paragraph.translation.is_none() {
            target = Some(paragraph.id.clone());
            break;
        }
    }
    let Some(paragraph_id) = target else {
        eprintln!("skip：corpus has no paragraph without translation and annotation");
        return;
    };
    let html = app
        .handle(&Request::get(&format!("/read?p={paragraph_id}")))
        .body;
    assert!(html.contains("暫無注疏"), "{paragraph_id}");
    assert!(html.contains("暫無白話"), "{paragraph_id}");
}

#[test]
fn real_search_hits_wei_yang_with_the_base_text_character() {
    use zgc_core::{FieldSource, Scope};

    let Some(app) = real_app() else { return };
    let hits = app.search("衞鞅", &Scope::All);
    let text_hits = hits
        .iter()
        .filter(|hit| hit.field == FieldSource::Text)
        .count();
    assert_eq!(text_hits, 5, "base text has five 衞鞅 occurrences");
    let page = app
        .handle(&Request::get("/search?q=%E8%A1%9E%E9%9E%85"))
        .body;
    assert!(
        page.contains("<mark>衞鞅</mark>"),
        "衞鞅 must be highlighted"
    );
    assert!(page.contains("命中"), "{page:.400}");

    // 「衛」 is not the base text's character: no original-text hit may appear.
    // (Commentary files do use 衛, so annotation hits are legitimate and allowed.)
    let wrong = app.search("衛鞅", &Scope::All);
    assert!(
        wrong.iter().all(|hit| hit.field != FieldSource::Text),
        "衛鞅 must not match the original text"
    );
}

#[test]
fn unknown_paragraph_returns_a_404_page_on_real_data() {
    let Some(app) = real_app() else { return };
    let response = app.handle(&Request::get("/read?p=qin-9-99-p9999"));
    assert_eq!(response.status, 404);
    assert!(response.body.contains("找不到這一段"));
}

#[test]
fn layout_switches_apply_to_real_pages() {
    let Some(app) = real_app() else { return };
    let single = app
        .handle(&Request::get(
            "/read?p=qin-1-01-p0001&layout=single&orig=xl&lh=loose",
        ))
        .body;
    assert!(single.contains("layout-single"));
    assert!(single.contains("--orig-size:1.66rem"));
    assert!(single.contains("--leading:2.3"));
}

// ------------------------------------------------------------- entities

fn real_app_with_entities() -> Option<App> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/zhanguoce");
    if !dir.join("book.json").is_file() {
        eprintln!("skip：找不到 {}", dir.display());
        return None;
    }
    let data = Book::load(&dir).expect("real corpus must load");
    let entities = Entities::load(&dir, &data).expect("real entities must load");
    Some(App::with_entities(data, entities))
}

fn original_column(html: &str) -> &str {
    const OPEN: &str = "<section class=\"col col-orig\">";
    const NEXT: &str = "<section class=\"col col-tran\">";
    let start = html.find(OPEN).expect("原文列必須存在") + OPEN.len();
    let end = start + html[start..].find(NEXT).expect("白話列必須存在");
    &html[start..end]
}

#[test]
fn real_reading_page_links_entities_inside_the_original_column() {
    let Some(app) = real_app_with_entities() else {
        return;
    };
    let html = app.handle(&Request::get("/read?p=qin-1-01-p0001")).body;
    let col = original_column(&html);

    assert!(
        col.contains("href=\"/entity?id=person-wei-yang\""),
        "衞鞅 must become a link：{col}"
    );
    assert!(col.contains("title=\"人物 · 衞鞅\">衞鞅</a>"), "{col}");
    assert!(
        col.contains("</a>亡"),
        "text right after the anchor stays plain"
    );
    assert_eq!(
        col.matches("<a ").count(),
        col.matches("</a>").count(),
        "no nested anchors"
    );
}

#[test]
fn real_entity_page_lists_every_occurrence_of_the_entity() {
    let Some(app) = real_app_with_entities() else {
        return;
    };
    let response = app.handle(&Request::get("/entity?id=person-wei-yang"));
    assert_eq!(response.status, 200);
    let html = &response.body;

    assert!(
        html.contains("衞鞅") && html.contains("卫鞅"),
        "both names shown"
    );
    assert!(html.contains("出現 23 次"), "23 occurrences：{html}");
    assert_eq!(
        html.matches("class=\"occ\"").count(),
        23,
        "one link per occurrence"
    );
    assert!(
        html.contains("/read?p=qin-1-01-p0001&amp;hl=0,2"),
        "first occurrence links back with its span"
    );
    assert!(html.contains("第 1 段"), "each row names its paragraph");

    let unknown = app.handle(&Request::get("/entity?id=person-nobody"));
    assert_eq!(unknown.status, 404);
}

#[test]
fn real_entity_index_reports_totals_and_filters_by_kind() {
    let Some(app) = real_app_with_entities() else {
        return;
    };
    let html = app.handle(&Request::get("/entities")).body;
    assert!(html.contains("274 個實體"), "{html}");
    assert!(html.contains("10685 次出現"), "{html}");
    assert!(html.contains("見於 4195 段"), "{html}");
    assert!(html.contains("38 段有重疊"), "overlap disclosure");
    assert!(
        html.contains("href=\"/entity?id=place-qin\""),
        "people and places are listed"
    );

    let people = app
        .handle(&Request::get("/entities?kind=person&sort=name"))
        .body;
    assert!(people.contains("href=\"/entity?id=person-wei-yang\""));
    assert!(
        people.contains("kind=person&amp;sort=count"),
        "filter survives the sort chips"
    );
    assert!(
        !people.contains("href=\"/entity?id=place-qin\""),
        "kind filter excludes places"
    );

    let places = app.handle(&Request::get("/entities?kind=place")).body;
    assert!(places.contains("href=\"/entity?id=place-qin\""));
    assert!(!places.contains("href=\"/entity?id=person-wei-yang\""));
}

#[test]
fn real_overlapping_paragraph_renders_one_disjoint_anchor_per_span() {
    let Some(app) = real_app_with_entities() else {
        return;
    };
    let entities = app.entities();
    let (pid, raw, spans) = app
        .data()
        .iter_paragraphs()
        .find_map(|(_, paragraph)| {
            let id = paragraph.id.as_str();
            let raw = entities.occurrences_in(id).count();
            let spans = entities.spans_in(id).count();
            (spans > 0 && spans < raw).then(|| (id.to_string(), raw, spans))
        })
        .expect("real corpus has at least one overlapping paragraph");

    let html = app.handle(&Request::get(&format!("/read?p={pid}"))).body;
    let col = original_column(&html);
    assert_eq!(
        col.matches("class=\"ent ").count(),
        spans,
        "one anchor per kept span"
    );
    assert_eq!(
        col.matches("<a ").count(),
        col.matches("</a>").count(),
        "never nested"
    );
    assert!(
        spans < raw,
        "the fixture paragraph really did drop {} span(s)",
        raw - spans
    );
}

#[test]
fn real_page_ignores_an_illegal_hl_and_still_escapes_the_paragraph_id() {
    let Some(app) = real_app_with_entities() else {
        return;
    };
    for hl in ["", "0", "0,0", "abc", "12,9999", "99999,3"] {
        let html = app
            .handle(&Request::get(&format!("/read?p=qin-1-01-p0001&hl={hl}")))
            .body;
        assert!(
            !html.contains("class=\"ent ent-person ent-hl\""),
            "hl={hl} must be ignored"
        );
        assert!(!html.contains("<mark>"), "hl={hl} must not mark");
    }
    let marked = app
        .handle(&Request::get("/read?p=qin-1-01-p0001&hl=0,2"))
        .body;
    assert!(
        marked.contains("<mark>衞鞅</mark>"),
        "a legal hl marks the entity"
    );

    let xss = app.handle(&Request::get("/read?p=<script>alert(1)</script>"));
    assert_eq!(xss.status, 404);
    assert!(!xss.body.contains("<script>alert(1)</script>"));
    assert!(xss.body.contains("&lt;script&gt;"));
}

// ------------------------------------------------------------- timeline

fn real_app_with_events() -> Option<App> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/zhanguoce");
    if !dir.join("book.json").is_file() {
        eprintln!("skip：找不到 {}", dir.display());
        return None;
    }
    if !dir.join(zgc_core::EVENT_FILE).is_file() {
        eprintln!("skip：{} 沒有事件表", dir.display());
        return None;
    }
    let data = Book::load(&dir).expect("real corpus must load");
    let entities = Entities::load(&dir, &data).expect("real entities must load");
    let events = Events::load(&dir).expect("real event table must load");
    Some(App::with_events(data, entities, events))
}

#[test]
fn real_timeline_page_lists_every_year_newest_first() {
    let Some(app) = real_app_with_events() else {
        return;
    };
    let response = app.handle(&Request::get("/timeline"));
    assert_eq!(response.status, 200);
    let html = &response.body;

    let table = app.events();
    assert_eq!(table.year_count(), 180);
    assert_eq!(table.event_count(), 583);
    assert_eq!(table.year_bounds(), Some((403, 209)));
    assert_eq!(table.years_without_events().len(), 15);

    assert!(html.contains("583 條事件"), "{}", &html[..600]);
    assert!(html.contains("前403–前209"), "the lede states the span");
    assert!(
        html.contains("id=\"tj-y0403-p4\""),
        "first event of the corpus"
    );
    assert!(
        html.contains("初命晉大夫魏斯、趙籍、韓虔爲諸侯。"),
        "verbatim text"
    );
    let newer = html.find("id=\"y0403\"").expect("year 403 block");
    let older = html.find("id=\"y0209\"").expect("year 209 block");
    assert!(newer < older, "years render in descending order");
}

#[test]
fn real_timeline_year_and_event_queries_narrow_to_one_year() {
    let Some(app) = real_app_with_events() else {
        return;
    };

    let one_year = app.handle(&Request::get("/timeline?y=403")).body;
    assert_eq!(one_year.matches("class=\"tl-year\"").count(), 1);
    assert!(
        one_year.contains("前四○三年"),
        "the year label survives verbatim"
    );
    assert!(!one_year.contains("id=\"y0209\""), "other years stay out");
    assert!(one_year.contains("id=\"tj-y0403-p4\""));
    assert!(
        one_year.contains(r#"<a data-nav="prev""#) || one_year.contains(r#"<a data-nav="next""#),
        "the year pager is keyboard-navigable"
    );
    assert!(one_year.contains("← → 鍵：前一年 / 後一年"));

    let focused = app.handle(&Request::get("/timeline?e=tj-y0209-p108")).body;
    assert_eq!(focused.matches("class=\"tl-year\"").count(), 1);
    assert!(
        focused.contains("class=\"tl-ev focus\" id=\"tj-y0209-p108\""),
        "?e= marks its event"
    );
    assert!(focused.contains("是歲，二世廢衞君角爲庶人"));

    let unknown = app.handle(&Request::get("/timeline?e=tj-y0000-p0")).body;
    assert!(unknown.contains("找不到事件"), "a bad ?e= degrades in-page");
}

#[test]
fn real_timeline_filter_marks_matches_and_survives_an_empty_table() {
    let Some(app) = real_app_with_events() else {
        return;
    };

    let filtered = app
        .handle(&Request::get("/timeline?q=%E7%A7%A6%E4%BC%90"))
        .body;
    assert!(filtered.contains("<mark>秦伐</mark>"), "matches are marked");
    assert!(filtered.contains("命中"), "the hit count is stated");
    assert!(
        !filtered.contains("id=\"tj-y0403-p4\""),
        "non-matching events drop out"
    );

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/zhanguoce");
    let data = Book::load(&dir).expect("real corpus must load");
    let bare = App::with_events(data, Entities::empty(), Events::empty());
    let response = bare.handle(&Request::get("/timeline"));
    assert_eq!(
        response.status, 200,
        "a book without a timeline still serves"
    );
    assert!(response.body.contains("此書沒有事件表"));
    assert_eq!(bare.events().event_count(), 0);
}

// ---------------------------------------------------------------- strategy

fn real_app_with_strategy() -> Option<App> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/zhanguoce");
    if !dir.join("book.json").is_file() {
        eprintln!("skip：找不到 {}", dir.display());
        return None;
    }
    let data = Book::load(&dir).expect("real corpus must load");
    let entities = Entities::load(&dir, &data).expect("real entities must load");
    let strategy = Strategy::load(&dir, &data, &entities).expect("real strategy table must load");
    if strategy.is_empty() {
        eprintln!(
            "skip：找不到 {}",
            dir.join("strategy/chapters.json").display()
        );
        return None;
    }
    Some(App::with_entities(data, entities).with_strategy(strategy))
}

/// The chapter page is paragraph 0, so the strategy block is asked for there.
fn chapter_entry(app: &App, chapter_id: &str) -> String {
    let start = app
        .chapter_start(chapter_id)
        .expect("chapter must have a start");
    let paragraph_id = app.paragraph_id_at(start).expect("start must resolve");
    app.handle(&Request::get(&format!("/read?p={paragraph_id}")))
        .body
}

#[test]
fn real_strategy_counts_cover_every_chapter() {
    let Some(app) = real_app_with_strategy() else {
        return;
    };
    let strategy = app.strategy();
    assert_eq!(strategy.chapter_count(), app.data().chapter_count());
    assert!(strategy.turn_total() > 0 && strategy.device_total() > 0);

    let mut turns = 0;
    let mut devices = 0;
    let mut with_turns = 0;
    let mut with_devices = 0;
    let mut silent = 0;
    for chapter in strategy.chapters() {
        turns += chapter.turns.len();
        devices += chapter.devices.len();
        match (chapter.turns.is_empty(), chapter.devices.is_empty()) {
            (false, false) => {
                with_turns += 1;
                with_devices += 1;
            }
            (false, true) => with_turns += 1,
            (true, false) => with_devices += 1,
            (true, true) => silent += 1,
        }
    }
    assert_eq!(turns, strategy.turn_total());
    assert_eq!(devices, strategy.device_total());
    assert_eq!(with_turns, strategy.chapters_with_turns());
    assert_eq!(with_devices, strategy.chapters_with_devices());
    assert!(
        silent > 0,
        "unidentified chapters must stay counted, not hidden"
    );
}

#[test]
fn real_strategy_page_lists_turns_and_links_each_device_span() {
    let Some(app) = real_app_with_strategy() else {
        return;
    };
    let chapter = app
        .strategy()
        .chapters()
        .iter()
        .find(|chapter| !chapter.turns.is_empty() && !chapter.devices.is_empty())
        .expect("some chapter has both");
    let html = chapter_entry(&app, &chapter.chapter_id);

    assert!(
        html.contains("謀略結構"),
        "the block is on the chapter page"
    );
    assert!(html.contains("遊說輪次"), "turn table");
    assert!(html.contains("論證手段"), "device groups");
    let turn = &chapter.turns[0];
    assert!(
        html.contains(&format!(
            "hl={},{}",
            turn.offset,
            turn.marker.chars().count()
        )) || html.contains("turn-text")
    );
    let device = &chapter.devices[0];
    assert!(
        html.contains(&format!("hl={},{}", device.offset, device.len)),
        "device link must carry its own span"
    );
    assert!(
        html.contains(&format!("<mark>{}</mark>", device.matched)),
        "the matched glyphs are shown verbatim"
    );
}

#[test]
fn real_strategy_device_link_highlights_exactly_that_span() {
    let Some(app) = real_app_with_strategy() else {
        return;
    };
    let (chapter, device) = app
        .strategy()
        .chapters()
        .iter()
        .find_map(|chapter| {
            chapter
                .devices
                .iter()
                .find(|device| device.len >= 2)
                .map(|device| (chapter, device))
        })
        .expect("some multi-glyph device");
    let html = app
        .handle(&Request::get(&format!(
            "/read?p={}&hl={},{}",
            device.p, device.offset, device.len
        )))
        .body;
    assert!(
        html.contains(&format!("<mark>{}</mark>", device.matched)),
        "the recorded span must be marked"
    );
    let chapter_html = chapter_entry(&app, &chapter.chapter_id);
    assert!(
        chapter_html.contains(&format!(
            "/read?p={}&amp;hl={},{}",
            device.p, device.offset, device.len
        )),
        "the chapter page links back to this exact highlight"
    );
}

#[test]
fn real_strategy_silent_chapter_says_so_instead_of_showing_an_empty_table() {
    let Some(app) = real_app_with_strategy() else {
        return;
    };
    let chapter = app
        .strategy()
        .chapters()
        .iter()
        .find(|chapter| chapter.turns.is_empty() && chapter.devices.is_empty())
        .expect("some chapter has neither");
    let html = chapter_entry(&app, &chapter.chapter_id);
    assert!(html.contains("本章未檢出游說標記"), "{html}");
    assert!(!html.contains("遊說輪次"), "no empty table");
}

// ------------------------------------------------------------ story sidebar

fn real_story_app() -> Option<App> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/zhanguoce");
    if !dir.join("book.json").is_file() {
        eprintln!("skip：找不到 {}", dir.display());
        return None;
    }
    let data = Book::load(&dir).expect("real corpus must load");
    let entities = Entities::load(&dir, &data).expect("real entities must load");
    let events = Events::load(&dir).expect("real event table must load");
    let years = ChapterYears::load(&dir, &events).expect("real year table must load");
    Some(App::with_events(data, entities, events).with_chapter_years(years))
}

/// One sidebar block, cut out by its heading.
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
fn real_story_sidebar_carries_people_places_and_the_year_block() {
    let Some(app) = real_story_app() else {
        return;
    };
    let response = app.handle(&Request::get("/story?c=qin-1-02"));
    assert_eq!(response.status, 200);
    let html = &response.body;

    assert!(html.contains("<aside class=\"story-side\">"), "{html}");
    // Independent count from data/zhanguoce/entities/people.json: 蘇秦 8 spans,
    // 秦惠王 1 — two distinct people in these 39 paragraphs, nothing else.
    let people = side_block(html, "本章人物");
    assert_eq!(
        people.matches("/entity?id=").count(),
        2,
        "人物數必須與資料一致：{people}"
    );
    assert!(
        people.contains("href=\"/entity?id=person-su-qin\""),
        "{people}"
    );
    assert!(
        people.contains("<span class=\"side-count\">8</span>"),
        "蘇秦 8 次：{people}"
    );
    assert!(
        people.contains("href=\"/entity?id=person-qin-huiwang\""),
        "{people}"
    );

    // places.json for the same paragraphs: 11 distinct, 秦 6 and 趙 4 on top.
    let places = side_block(html, "本章地點");
    assert_eq!(
        places.matches("/entity?id=").count(),
        11,
        "地點數必須與資料一致：{places}"
    );
    assert!(places.contains("href=\"/entity?id=place-qin\""), "{places}");
    assert!(
        places.contains("<span class=\"side-count\">6</span>"),
        "秦 6 次：{places}"
    );

    // events/chapter_years.json lists qin-1-02 as unresolved (below-threshold):
    // the block says so and invents no year.
    let when = side_block(html, "系年");
    assert!(when.contains("未能系年"), "{when}");
    assert!(!when.contains("約前"), "未定年的章不得寫出年份：{when}");
    let aside = html.find("<aside class=\"story-side\">").expect("aside");
    assert!(
        html.find("未能系年").expect("year block") > aside,
        "系年 must sit in the sidebar, not the header"
    );
}

#[test]
fn real_story_sidebar_lists_every_person_of_a_crowded_chapter() {
    let Some(app) = real_story_app() else {
        return;
    };
    // people.json counts for qin-2-06's 26 paragraphs: nine distinct people.
    let html = app.handle(&Request::get("/story?c=qin-2-06")).body;
    let people = side_block(&html, "本章人物");
    assert_eq!(
        people.matches("/entity?id=").count(),
        9,
        "九位人物：{people}"
    );
    for id in ["person-gan-mao", "person-xiang-shou", "person-yue-yang"] {
        assert!(
            people.contains(&format!("href=\"/entity?id={id}\"")),
            "缺少 {id}：{people}"
        );
    }
    assert!(
        people.contains("<span class=\"side-count\">8</span>"),
        "{people}"
    );
}

#[test]
fn real_story_background_block_is_lazy_and_degrades_without_ai() {
    let Some(app) = real_story_app() else {
        return;
    };
    let plain = app.handle(&Request::get("/story?c=qin-1-02")).body;
    let lazy = side_block(&plain, "背景史料");
    assert!(lazy.contains("檢索背景史料"), "{lazy}");
    assert!(
        lazy.contains("bg=1&amp;sc="),
        "link keeps the reader state：{lazy}"
    );

    // No ZGC_AI_* in the test app: the block degrades, the page stays 200.
    let response = app.handle(&Request::get("/story?c=qin-1-02&bg=1"));
    assert_eq!(response.status, 200);
    let block = side_block(&response.body, "背景史料");
    assert!(block.contains("未取到史料"), "{block}");
    assert!(block.contains("ZGC_AI_KEY"), "{block}");
    assert!(
        response.body.contains("<div class=\"story-main\">"),
        "the chapter itself still renders"
    );
}
