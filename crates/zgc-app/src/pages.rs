//! Server-side HTML rendering: one page per request.
//!
//! No template engine and no JavaScript — each page is a String assembled from
//! escaped fragments. Every value that comes from the corpus or from the
//! request passes through html::escape or html::escape_highlight first.

use std::borrow::Cow;
use std::collections::HashMap;

use zgc_core::{
    build_prompt, AskContext, Chapter, Entity, EntityKind, Event, EventYear, EvidenceText,
    FieldSource, Hit, Paragraph, Position, Prompt, SpanView, MAX_QUESTION_CHARS, MAX_USER_CHARS,
};

use crate::ai::{AiConfig, AiError, AiReply, ExternalSource};
use crate::app::App;
use crate::html::{escape, escape_highlight};
use crate::prefs::{FontStep, Layout, Leading, Prefs, Script, COOKIE_LAST};
use crate::urlcodec::{self, Cookies};

const CSS: &str = include_str!("style.css");

/// Longest error detail echoed back on a 404 page.
const MAX_DETAIL_CHARS: usize = 160;

// ---------------------------------------------------------------- page shell

fn document(
    app: &App,
    prefs: Option<&Prefs>,
    title: &str,
    root_style: &str,
    search_value: &str,
    selected_group: &str,
    body: &str,
) -> String {
    let mut options = String::new();
    for group in app.data().groups() {
        let selected = if group.id == selected_group {
            " selected"
        } else {
            ""
        };
        options.push_str(&format!(
            "<option value=\"{}\"{}>{}</option>",
            escape(&group.id),
            selected,
            escape(&display(app, prefs, &group.title))
        ));
    }
    let book_title = escape(&display(app, prefs, &app.data().book().title));
    let mut out = String::with_capacity(body.len() + CSS.len() + 1024);
    let lang = match prefs {
        Some(prefs) if simp_mode(app, prefs) => "zh-Hans",
        _ => "zh-Hant",
    };
    out.push_str(&format!("<!DOCTYPE html>\n<html lang=\"{}\"", lang));
    if !root_style.is_empty() {
        out.push_str(&format!(" style=\"{}\"", root_style));
    }
    out.push_str(">\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str(&format!(
        "<title>{}</title>\n<style>\n{}\n</style>\n</head>\n<body>\n",
        escape(title),
        CSS
    ));
    out.push_str("<header class=\"top\">");
    out.push_str(&format!("<a class=\"brand\" href=\"/\">{}</a>", book_title));
    out.push_str("<nav class=\"ent-nav\"><a href=\"/entities\">實體索引</a></nav>");
    out.push_str("<form class=\"find\" action=\"/search\" method=\"get\" role=\"search\">");
    out.push_str(&format!(
        "<input type=\"search\" name=\"q\" value=\"{}\" placeholder=\"檢索原文 / 白話 / 注疏\" aria-label=\"檢索詞\">",
        escape(search_value)
    ));
    out.push_str(&format!(
        "<select name=\"group\" aria-label=\"限定國策\"><option value=\"\">全書</option>{}</select>",
        options
    ));
    out.push_str("<button type=\"submit\">檢索</button></form></header>\n<main>\n");
    out.push_str(body);
    out.push_str("\n</main>\n<footer class=\"foot\">底本繁體 · 白話簡體 · 注疏多家並存 · 本機離線閱讀</footer>\n</body>\n</html>\n");
    out
}

/// Title plus its simplified form when the two differ. In 簡體 mode the title
/// itself is converted and no parenthetical is added: there is nothing left to
/// gloss.
fn labelled(app: &App, prefs: &Prefs, title: &str, title_simp: Option<&str>) -> String {
    let title = title.trim();
    if simp_mode(app, prefs) {
        return escape(&simplified(app, prefs, title));
    }
    let simplified = title_simp
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != title);
    match simplified {
        Some(simplified) => format!(
            "{}<span class=\"simp\">（{}）</span>",
            escape(title),
            escape(simplified)
        ),
        None => escape(title),
    }
}

/// The four fragments every paragraph view needs. Shared by /read and /story
/// so the 三態 original, 白話, 注疏 and 訓釋 rules can never drift apart.
struct ParagraphParts {
    original: String,
    translation: String,
    notes: String,
    glossary: String,
}

fn paragraph_parts(
    app: &App,
    paragraph: &Paragraph,
    paragraph_id: &str,
    prefs: &Prefs,
    highlight: Option<(usize, usize)>,
) -> ParagraphParts {
    let original = original_html(app, &paragraph.text, paragraph_id, highlight, prefs);

    let translation = paragraph
        .translation
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty());
    let translation = match translation {
        Some(text) => format!("<p class=\"tran\">{}</p>", escape(text)),
        None => "<p class=\"empty\">暫無白話</p>".to_string(),
    };

    let mut notes = String::new();
    if paragraph.annotations.is_empty() {
        notes.push_str("<p class=\"empty\">暫無注疏</p>");
    } else {
        notes.push_str("<ol class=\"notes\">");
        for annotation in &paragraph.annotations {
            let source = annotation
                .source
                .as_deref()
                .map(str::trim)
                .filter(|source| !source.is_empty())
                .unwrap_or("未署");
            // 見證本標記：缺省即四部叢刊（底本），只有第二見證才加 chip，
            // 免得 8006 條底本注全部掛上同一個標籤。
            let edition = annotation
                .edition
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| {
                    format!(
                        "<span class=\"ed\">{}</span>",
                        escape(&simplified(app, prefs, value))
                    )
                })
                .unwrap_or_default();
            notes.push_str(&format!(
                "<li><span class=\"src\">{}</span>{}<p class=\"note-text\">{}</p></li>",
                escape(&simplified(app, prefs, source)),
                edition,
                escape(&simplified(app, prefs, &annotation.text))
            ));
        }
        notes.push_str("</ol>");
    }

    let mut glossary = String::new();
    if !paragraph.glossary.is_empty() {
        glossary.push_str("<div class=\"glossary\"><h3>字詞訓釋</h3><ul>");
        for entry in &paragraph.glossary {
            let pinyin = entry
                .pinyin
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| format!("<span class=\"py\">{}</span>", escape(value)))
                .unwrap_or_default();
            let gloss = entry
                .gloss
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| escape(&simplified(app, prefs, value)))
                .unwrap_or_default();
            glossary.push_str(&format!(
                "<li><span class=\"term\">{}</span>{} {} </li>",
                escape(&simplified(app, prefs, &entry.term)),
                pinyin,
                gloss
            ));
        }
        glossary.push_str("</ul></div>");
    }

    ParagraphParts {
        original,
        translation,
        notes,
        glossary,
    }
}

/// The 原文 heading names the script actually shown, and a converted column
/// says so: the table is a reading aid, not a critical edition. 對照 names
/// both scripts because the column carries both.
fn original_heading(app: &App, prefs: &Prefs) -> (&'static str, &'static str) {
    if simp_mode(app, prefs) {
        (
            "原文（簡體）",
            "<p class=\"simp-note\">機器轉換，非校勘</p>",
        )
    } else if both_mode(app, prefs) {
        (
            "原文（繁 / 簡）",
            "<p class=\"simp-note\">簡體行為機器轉換，非校勘</p>",
        )
    } else {
        ("原文（繁體）", "")
    }
}

fn read_href(paragraph_id: &str) -> String {
    format!("/read?p={}", urlcodec::percent_encode(paragraph_id))
}

fn story_href_chapter(chapter_id: &str) -> String {
    format!("/story?c={}", urlcodec::percent_encode(chapter_id))
}

fn story_href_paragraph(paragraph_id: &str) -> String {
    format!("/story?p={}", urlcodec::percent_encode(paragraph_id))
}

// ------------------------------------------------------------ table of contents

pub fn toc_page(app: &App, prefs: &Prefs, cookies: &Cookies) -> String {
    let data = app.data();
    let mut body = String::new();
    body.push_str("<h1 class=\"page-title\">目錄</h1>");
    body.push_str(&format!(
        "<p class=\"stats\">{} 國 · {} 卷 · {} 章 · {} 段</p>",
        data.group_count(),
        data.volume_count(),
        data.chapter_count(),
        data.paragraph_count()
    ));
    body.push_str(&format!(
        "<section class=\"story-promo\"><h2>{}</h2><p>{}</p><p><a class=\"chip\" href=\"{}\">{}</a></p></section>",
        escape(&simplified(app, prefs, "故事視界")),
        escape(&simplified(
            app,
            prefs,
            "以章（一篇策文）為單位通讀，並生成背景 / 寓意 / 可學。"
        )),
        story_href_chapter("dongzhou-01"),
        escape(&simplified(app, prefs, "進入故事視界 →"))
    ));
    body.push_str(&continue_reading(app, prefs, cookies));

    body.push_str("<nav class=\"tree\" data-arrow-step=\"div.vol[data-vol-step]\" data-arrow-keys=\"lr\">");
    for group in data.groups() {
        let volume_indices = data.group_volume_indices(&group.id);
        let chapter_count: usize = volume_indices
            .iter()
            .filter_map(|index| data.volume(*index))
            .map(|volume| volume.chapters.len())
            .sum();
        body.push_str(&format!(
            "<section class=\"group\"><h2><span class=\"g-title\">{}</span><span class=\"count\">{} 卷 · {} 章</span></h2>",
            labelled(app, prefs, &group.title, group.title_simp.as_deref()),
            volume_indices.len(),
            chapter_count
        ));
        for &volume_index in volume_indices {
            let Some(volume) = data.volume(volume_index) else {
                continue;
            };
            let paragraph_count: usize = volume.chapters.iter().map(|c| c.paragraphs.len()).sum();
            let heading = match data
                .volume_start(volume_index)
                .and_then(|position| app.paragraph_id_at(position))
            {
                Some(paragraph_id) => format!(
                    "<a href=\"{}\">{}</a>",
                    read_href(paragraph_id),
                    labelled(app, prefs, &volume.title, volume.title_simp.as_deref())
                ),
                None => format!(
                    "<span>{}</span>",
                    labelled(app, prefs, &volume.title, volume.title_simp.as_deref())
                ),
            };
            body.push_str(&format!(
                "<div class=\"vol\" data-vol-step><h3>{}<span class=\"count\">{} 章 · {} 段</span></h3><ol class=\"chapters\">",
                heading,
                volume.chapters.len(),
                paragraph_count
            ));
            for chapter in &volume.chapters {
                match app
                    .chapter_start(&chapter.id)
                    .and_then(|position| app.paragraph_id_at(position))
                {
                    Some(paragraph_id) => body.push_str(&format!(
                        "<li><a href=\"{}\">{}</a><a class=\"story-link\" href=\"{}\">{}</a></li>",
                        read_href(paragraph_id),
                        escape(&simplified(app, prefs, &chapter.title)),
                        story_href_chapter(&chapter.id),
                        escape(&simplified(app, prefs, "故事"))
                    )),
                    None => body.push_str(&format!(
                        "<li><span class=\"empty\">{}</span><a class=\"story-link\" href=\"{}\">{}</a></li>",
                        escape(&simplified(app, prefs, &chapter.title)),
                        story_href_chapter(&chapter.id),
                        escape(&simplified(app, prefs, "故事"))
                    )),
                }
            }
            body.push_str("</ol></div>");
        }
        body.push_str("</section>");
    }
    body.push_str("</nav>");
    body.push_str(&format!(
        "<p class=\"keyhint\">{}</p>{}",
        escape(&simplified(app, prefs, "← → 鍵：上一卷 / 下一卷")),
        ARROW_STEP_SCRIPT
    ));

    document(app, Some(prefs), "目錄", "", "", "", &body)
}

fn continue_reading(app: &App, prefs: &Prefs, cookies: &Cookies) -> String {
    let Some(stored) = urlcodec::cookie_text(cookies, COOKIE_LAST) else {
        return String::new();
    };
    let Some(position) = app.position_of(stored.trim()) else {
        return String::new();
    };
    let Some(paragraph_id) = app.paragraph_id_at(position) else {
        return String::new();
    };
    let data = app.data();
    let volume_title = data
        .volume(position.volume)
        .map(|volume| volume.title.clone())
        .unwrap_or_default();
    let chapter_title = data
        .chapter(position.volume, position.chapter)
        .map(|chapter| chapter.title.clone())
        .unwrap_or_default();
    format!(
        "<section class=\"resume\"><h2>繼續閱讀</h2><p><a href=\"{}\">{}<span class=\"sep\">·</span>{}<span class=\"sep\">·</span>第 {} 段</a></p></section>",
        read_href(paragraph_id),
        escape(&simplified(app, prefs, &volume_title)),
        escape(&simplified(app, prefs, &chapter_title)),
        position.paragraph + 1
    )
}

// ------------------------------------------------------------ chapter dating

/// What each evidence kind is actually claiming, in one line.
pub(crate) fn evidence_kind_label(kind: &str) -> &'static str {
    match kind {
        "substring" => "章文與通鑑記事逐字相合",
        "substring-range" => "章文與通鑑記事相合（跨年）",
        "name-overlap" => "同一組人物共見於此年",
        "name-window" => "同一組人物只在此年共見",
        "commentary-date" => "注疏直記王年",
        _ => "證據",
    }
}

/// Escape and cut to max characters, marking the cut with an ellipsis.
fn clip_escape(text: &str, max: usize) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        out.push('…');
    }
    escape(&out)
}

/// The 系年 sidebar block: the year, the evidence behind it, or the reason it
/// could not be dated. Empty when the book carries no chapter-year table at all.
fn chapter_when(app: &App, chapter_id: &str, prefs: &Prefs) -> String {
    let years = app.chapter_years();
    if years.is_empty() {
        return String::new();
    }
    let Some(record) = years.get(chapter_id) else {
        return match years.unresolved_reason(chapter_id) {
            Some(reason) => format!(
                "<div class=\"chap-when\"><p class=\"when unknown\">未能系年<span class=\"why-hint\">{}</span></p></div>",
                escape(zgc_core::reason_text(reason))
            ),
            None => String::new(),
        };
    };
    let [start, end] = record.year_range;
    let mut html = String::from("<div class=\"chap-when\">");
    if start == end {
        html.push_str(&format!("<p class=\"when\">約前{}年", record.year_bc));
    } else {
        html.push_str(&format!(
            "<p class=\"when\">約前{}年<span class=\"span\">（前{}–前{}年間）</span>",
            record.year_bc, start, end
        ));
    }
    html.push_str(&format!(
        "<span class=\"conf\">置信度 {}</span></p>",
        record.confidence.label()
    ));
    html.push_str("<details><summary>憑什麼是這一年</summary><ul class=\"why\">");
    for evidence in &record.evidence {
        let (event_text, event_link) = match app.events().event(&evidence.event_id) {
            Some(event) => (
                clip_escape(&simplified(app, prefs, &event.text), 60),
                format!(
                    "<a href=\"/timeline?e={}\">通鑑前{}年</a>",
                    urlcodec::percent_encode(&evidence.event_id),
                    event.year_bc
                ),
            ),
            None => (String::from("事件表未載此條"), evidence.event_id.clone()),
        };
        // Only a verbatim span may wear 「」. A name list is a claim about who
        // appears, not a sentence of the chapter, so it is neither quoted nor
        // called 章文.
        let claim = match &evidence.text {
            EvidenceText::Names(names) => format!(
                "<p class=\"match names\">共現人物：{}</p>",
                clip_escape(&simplified(app, prefs, &names.join("、")), 40)
            ),
            EvidenceText::Matched(matched) if evidence.kind.starts_with("substring") => {
                format!(
                    "<p class=\"match\">章文「{}」</p>",
                    clip_escape(&simplified(app, prefs, matched), 40)
                )
            }
            // commentary-date quotes the 注疏, not the chapter.
            EvidenceText::Matched(text) => {
                format!(
                    "<p class=\"match note\">注疏「{}」</p>",
                    clip_escape(&simplified(app, prefs, text), 40)
                )
            }
        };
        html.push_str(&format!(
            "<li><p class=\"kind\">{}</p>{claim}<p class=\"ev\">通鑑：{} {}</p></li>",
            evidence_kind_label(&evidence.kind),
            event_text,
            event_link
        ));
    }
    html.push_str("</ul></details></div>");
    html
}

// ------------------------------------------------------------ strategy block

/// The 謀略結構 block: who spoke to whom, and which rhetorical devices the text
/// visibly carries. Rules only — every row and every link lands on the glyphs
/// it names, and nothing here judges whose plan was better.
fn chapter_strategy(app: &App, chapter_id: &str, prefs: &Prefs) -> String {
    let strategy = app.strategy();
    if strategy.is_empty() {
        return String::new();
    }
    let Some(chapter) = strategy.chapter(chapter_id) else {
        return String::new();
    };
    if chapter.turns.is_empty() && chapter.devices.is_empty() {
        return String::from(
            "<section class=\"chap-plan\"><p class=\"when unknown\">本章未檢出游說標記</p></section>",
        );
    }

    let mut html = String::from("<section class=\"chap-plan\"><details><summary>謀略結構");
    html.push_str(&format!(
        "<span class=\"counts\">{} 輪遊說 · {} 處手段</span>",
        chapter.turns.len(),
        chapter.devices.len()
    ));
    html.push_str("</summary>");
    html.push_str(
        "<p class=\"plan-note\">只列原文可見的修辭手段與「謂…曰」輪次，不判斷謀略高下。點段或字串可回原文。</p>",
    );

    if !chapter.turns.is_empty() {
        html.push_str("<h3 class=\"plan-head\">遊說輪次</h3>");
        html.push_str(
            "<table class=\"turns\"><thead><tr><th>#</th><th>說話人</th><th>受話人</th><th>章文</th></tr></thead><tbody>",
        );
        for turn in &chapter.turns {
            html.push_str(&format!(
                "<tr><td class=\"idx\">{}</td><td>{}</td><td>{}</td><td><a class=\"turn-text\" href=\"{}\">{}</a></td></tr>",
                turn.index + 1,
                person_cell(
                    app,
                    prefs,
                    turn.speaker_id.as_deref(),
                    turn.speaker.as_deref(),
                    unresolved_note(app, chapter_id, turn.index, "speaker"),
                ),
                person_cell(
                    app,
                    prefs,
                    turn.addressee_id.as_deref(),
                    turn.addressee.as_deref(),
                    unresolved_note(app, chapter_id, turn.index, "addressee"),
                ),
                read_href(&turn.p),
                escape(&simplified(app, prefs, &turn.text))
            ));
        }
        html.push_str("</tbody></table>");
    }

    if !chapter.devices.is_empty() {
        html.push_str("<h3 class=\"plan-head\">論證手段</h3>");
        for kind in strategy.kinds() {
            let hits: Vec<&zgc_core::Device> = chapter
                .devices
                .iter()
                .filter(|device| device.kind == kind.kind)
                .collect();
            if hits.is_empty() {
                continue;
            }
            html.push_str(&format!(
                "<div class=\"devices\"><h4 class=\"dev-kind\">{}</h4><ul>",
                escape(&kind.kind)
            ));
            for device in hits {
                let at = app
                    .position_of(&device.p)
                    .map(|position| {
                        format!(
                            "<span class=\"dev-p\">第 {} 段</span>",
                            position.paragraph + 1
                        )
                    })
                    .unwrap_or_default();
                html.push_str(&format!(
                    "<li><a href=\"{}\"><mark>{}</mark></a>{}</li>",
                    read_hl_href(&device.p, device.offset, device.len),
                    escape(&simplified(app, prefs, &device.matched)),
                    at
                ));
            }
            html.push_str("</ul></div>");
        }
    }

    html.push_str("</details></section>");
    html
}

/// Why one field of one turn stayed 未識別, when the generator recorded it.
fn unresolved_note(
    app: &App,
    chapter_id: &str,
    turn_index: usize,
    field: &str,
) -> Option<&'static str> {
    app.strategy()
        .unresolved_of(chapter_id)
        .find(|item| item.turn_index == turn_index && item.field == field)
        .map(|item| zgc_core::strategy_reason_text(&item.reason))
}

/// One name cell: a link when the entity is established, the glyphs plus
/// 未識別 when only the words are, and a bare 未識別 otherwise.
fn person_cell(
    app: &App,
    prefs: &Prefs,
    id: Option<&str>,
    glyphs: Option<&str>,
    note: Option<&str>,
) -> String {
    match (id, glyphs) {
        (Some(id), Some(name)) => format!(
            "<a href=\"/entity?id={}\">{}</a>",
            urlcodec::percent_encode(id),
            escape(&simplified(app, prefs, name))
        ),
        (_, Some(name)) => format!(
            "{}<span class=\"unknown\" title=\"{}\">（未識別）</span>",
            escape(&simplified(app, prefs, name)),
            escape(note.unwrap_or("未能確認"))
        ),
        _ => format!(
            "<span class=\"unknown\" title=\"{}\">未識別</span>",
            escape(note.unwrap_or("未能確認"))
        ),
    }
}

/// A /read link that asks the reader to mark one exact span.
fn read_hl_href(paragraph_id: &str, offset: usize, len: usize) -> String {
    format!(
        "/read?p={}&amp;hl={},{}",
        urlcodec::percent_encode(paragraph_id),
        offset,
        len
    )
}

/// True when the 繁體-side text must be shown through the 繁 → 簡 table: the
/// reader asked for 簡體 and the table actually loaded.
fn simp_mode(app: &App, prefs: &Prefs) -> bool {
    prefs.script == Script::Simp && !app.variants().is_empty()
}

/// True when every 原文 line is followed by its 簡體 conversion. Without the
/// table there is nothing to compare, so 對照 degrades to 繁體.
fn both_mode(app: &App, prefs: &Prefs) -> bool {
    prefs.script == Script::Both && !app.variants().is_empty()
}

/// Display form of one corpus string. 白話 never goes through here: it is
/// already 簡體 in the data.
fn simplified<'a>(app: &App, prefs: &Prefs, text: &'a str) -> Cow<'a, str> {
    if simp_mode(app, prefs) {
        Cow::Owned(app.variants().to_simplified(text))
    } else {
        Cow::Borrowed(text)
    }
}

/// Display form for the page shell. Error pages are rendered before a reader
/// preference exists; they keep the corpus glyphs rather than guess a script.
fn display<'a>(app: &App, prefs: Option<&Prefs>, text: &'a str) -> Cow<'a, str> {
    match prefs {
        Some(prefs) => simplified(app, prefs, text),
        None => Cow::Borrowed(text),
    }
}

// ------------------------------------------------------------ reading notes

/// The per-paragraph note UI. The server renders empty shells only: a note is
/// browser-local data, so no note text can ever reach the HTML. Every shell
/// starts hidden and the script unhides it, so a no-JS page is exactly the page
/// it was before notes existed. Shared by /read and /story.
fn note_block(app: &App, prefs: &Prefs, paragraph_id: &str) -> String {
    let id = escape(paragraph_id);
    let toggle = escape(&simplified(app, prefs, "筆記"));
    let placeholder = escape(&simplified(app, prefs, "寫下這段的想法…"));
    let quote = escape(&simplified(app, prefs, "引用所選原文"));
    let save = escape(&simplified(app, prefs, "保存"));
    let delete = escape(&simplified(app, prefs, "刪除"));
    format!(
        "<div class=\"note-block\">\
<button type=\"button\" class=\"note-toggle\" data-note-toggle=\"{id}\" aria-expanded=\"false\" hidden>{toggle}</button>\
<div class=\"note-edit\" data-note-edit=\"{id}\" hidden>\
<textarea class=\"note-input\" data-note-input=\"{id}\" rows=\"3\" maxlength=\"4000\" placeholder=\"{placeholder}\"></textarea>\
<p class=\"note-quote\" data-note-quote-text=\"{id}\" hidden></p>\
<p class=\"note-actions\"><button type=\"button\" data-note-quote=\"{id}\">{quote}</button>\
<button type=\"button\" data-note-save=\"{id}\">{save}</button>\
<button type=\"button\" data-note-delete=\"{id}\">{delete}</button></p>\
</div>\
<p class=\"note-status\" data-note-status=\"{id}\" role=\"status\"></p>\
<div class=\"note-view\" data-note-view=\"{id}\" hidden></div>\
</div>"
    )
}

/// The 「此段已有筆記」 dot in the paragraph head. Hidden until the script finds
/// a stored note, so the server never leaks whether one exists.
fn note_dot(app: &App, prefs: &Prefs, paragraph_id: &str) -> String {
    format!(
        "<span class=\"note-dot\" data-note-dot=\"{}\" title=\"{}\" hidden></span>",
        escape(paragraph_id),
        escape(&simplified(app, prefs, "此段已有筆記"))
    )
}

/// 匯出 / 匯入, rendered into the /read toolbar. The file input stays hidden:
/// the button opens the picker.
fn note_controls(app: &App, prefs: &Prefs) -> String {
    format!(
        "<span class=\"seg seg-notes\" data-note-tools hidden>\
<button type=\"button\" class=\"note-tool\" data-note-export>{}</button>\
<button type=\"button\" class=\"note-tool\" data-note-import-btn>{}</button>\
<input type=\"file\" data-note-import accept=\".json,application/json\" hidden></span>",
        escape(&simplified(app, prefs, "匯出筆記")),
        escape(&simplified(app, prefs, "匯入筆記"))
    )
}

/// One page-level status line plus the script's own strings. The strings live
/// in the HTML, converted by the same 三態 rule as the buttons, so a 簡體
/// reader never gets 繁體 status text out of a .js literal.
fn note_runtime(app: &App, prefs: &Prefs) -> String {
    let message = |text: &str| escape(&simplified(app, prefs, text));
    format!(
        "<p class=\"note-banner\" data-note-banner role=\"status\" hidden></p>\
<div class=\"note-i18n\" data-note-i18n hidden data-saved=\"{}\" data-deleted=\"{}\" data-empty=\"{}\" \
data-no-selection=\"{}\" data-blocked=\"{}\" data-corrupt=\"{}\" data-exported=\"{}\" \
data-imported=\"{}\" data-import-invalid=\"{}\" data-import-empty=\"{}\" \
data-import-unreadable=\"{}\"></div>",
        message("已保存。"),
        message("已刪除。"),
        message("請先寫點什麼，或按「刪除」。"),
        message("未選中本段原文：請先在原文裡劃選，再按「引用所選原文」。"),
        message("瀏覽器不允許本頁保存筆記（localStorage 不可用）。"),
        message("筆記資料已損壞，已清空 zgc_notes_v1，請重新開始。"),
        message("已匯出 {n} 條筆記。"),
        message("已匯入 {added} 條筆記，現共 {total} 條。"),
        message("匯入失敗：檔案不是合法的筆記 JSON。"),
        message("匯入失敗：檔案裡沒有可用的筆記。"),
        message("匯入失敗：無法讀取檔案。")
    )
}

/// Notes live in localStorage and are written only through textContent / value.
/// Nothing here touches innerHTML: every user string is a text node or a form
/// value, and every selector is a static attribute name.

/// The whole note feature: one inline script, no dependency. /read and /story
/// share it because both render the same shells; the 匯出/匯入 controls simply
/// do not exist on a page that does not render them.
const NOTE_SCRIPT: &str = r#"<script>
(function () {
  var KEY = "zgc_notes_v1";
  var LIMIT = 4000;
  var QUOTE_LIMIT = 200;
  var rows = {};
  var labels = {};
  var pending = {};

  function text(name) {
    return labels[name] || "";
  }

  function fill(template, values) {
    var out = template;
    for (var key in values) {
      if (Object.prototype.hasOwnProperty.call(values, key)) {
        out = out.split("{" + key + "}").join(String(values[key]));
      }
    }
    return out;
  }

  function banner(message) {
    var node = document.querySelector("[data-note-banner]");
    if (!node) return;
    node.textContent = message;
    node.hidden = false;
  }

  function normalize(entry) {
    if (!entry || typeof entry !== "object" || Array.isArray(entry)) return null;
    if (typeof entry.text !== "string") return null;
    var body = entry.text.trim();
    if (!body) return null;
    var quote = typeof entry.quote === "string" ? entry.quote.trim() : "";
    var stamp = typeof entry.updatedAt === "string" ? entry.updatedAt : "";
    return {
      text: body.slice(0, LIMIT),
      quote: quote.slice(0, QUOTE_LIMIT),
      updatedAt: stamp
    };
  }

  function load() {
    var raw = null;
    try {
      raw = window.localStorage.getItem(KEY);
    } catch (error) {
      banner(text("blocked"));
      return {};
    }
    if (!raw) return {};
    var parsed = null;
    var damaged = false;
    try {
      parsed = JSON.parse(raw);
    } catch (error) {
      damaged = true;
    }
    if (!damaged && (!parsed || typeof parsed !== "object" || Array.isArray(parsed))) {
      damaged = true;
    }
    var store = {};
    if (!damaged) {
      for (var id in parsed) {
        if (!Object.prototype.hasOwnProperty.call(parsed, id)) continue;
        var entry = normalize(parsed[id]);
        if (!entry) {
          damaged = true;
          continue;
        }
        store[id] = entry;
      }
    }
    if (damaged) {
      try {
        window.localStorage.removeItem(KEY);
      } catch (error) {}
      banner(text("corrupt"));
      return {};
    }
    return store;
  }

  function persist(store) {
    try {
      window.localStorage.setItem(KEY, JSON.stringify(store));
      return true;
    } catch (error) {
      banner(text("blocked"));
      return false;
    }
  }

  function total(store) {
    var count = 0;
    for (var id in store) {
      if (Object.prototype.hasOwnProperty.call(store, id)) count += 1;
    }
    return count;
  }

  function now() {
    var date = new Date();
    function pad(value) {
      return value < 10 ? "0" + value : String(value);
    }
    return date.getUTCFullYear() + "-" + pad(date.getUTCMonth() + 1) + "-" + pad(date.getUTCDate()) +
      "T" + pad(date.getUTCHours()) + ":" + pad(date.getUTCMinutes()) + ":" +
      pad(date.getUTCSeconds()) + "Z";
  }

  var FIELDS = [
    ["data-note-toggle", "toggle"],
    ["data-note-dot", "dot"],
    ["data-note-edit", "edit"],
    ["data-note-input", "input"],
    ["data-note-view", "view"],
    ["data-note-status", "status"],
    ["data-note-quote-text", "quoteText"],
    ["data-note-quote", "quoteButton"],
    ["data-note-save", "saveButton"],
    ["data-note-delete", "deleteButton"]
  ];

  function row(id) {
    if (!rows[id]) rows[id] = {};
    return rows[id];
  }

  function collectLabels() {
    var node = document.querySelector("[data-note-i18n]");
    if (!node) return;
    var names = ["saved", "deleted", "empty", "no-selection", "blocked", "corrupt",
      "exported", "imported", "import-invalid", "import-empty", "import-unreadable"];
    for (var i = 0; i < names.length; i++) {
      labels[names[i]] = node.getAttribute("data-" + names[i]) || "";
    }
  }

  function collectNodes() {
    for (var f = 0; f < FIELDS.length; f++) {
      var attribute = FIELDS[f][0];
      var name = FIELDS[f][1];
      var list = document.querySelectorAll("[" + attribute + "]");
      for (var i = 0; i < list.length; i++) {
        var node = list[i];
        var id = node.getAttribute(attribute);
        if (!id) continue;
        row(id)[name] = node;
      }
    }
  }

  function paint(id, entry) {
    var target = rows[id];
    if (!target) return;
    if (target.dot) target.dot.hidden = !entry;
    if (target.view) {
      target.view.textContent = entry ? entry.text : "";
      target.view.hidden = !entry;
    }
  }

  function paintAll(store) {
    for (var id in rows) {
      if (!Object.prototype.hasOwnProperty.call(rows, id)) continue;
      var entry = Object.prototype.hasOwnProperty.call(store, id) ? store[id] : null;
      paint(id, entry);
    }
  }

  function status(target, message) {
    if (target.status) target.status.textContent = message;
  }

  function open(target, entry) {
    if (!target.edit) return;
    target.edit.hidden = false;
    if (target.toggle) target.toggle.setAttribute("aria-expanded", "true");
    if (target.input) {
      target.input.value = entry ? entry.text : "";
      target.input.focus();
    }
    target.quote = entry ? entry.quote : "";
    if (target.quoteText) {
      target.quoteText.textContent = target.quote;
      target.quoteText.hidden = !target.quote;
    }
    status(target, "");
  }

  function close(target) {
    if (!target.edit) return;
    target.edit.hidden = true;
    if (target.toggle) target.toggle.setAttribute("aria-expanded", "false");
  }

  function toggle(id) {
    var target = rows[id];
    if (!target || !target.edit) return;
    if (target.edit.hidden) {
      open(target, load()[id] || null);
    } else {
      close(target);
      status(target, "");
    }
  }

  function save(id) {
    var target = rows[id];
    var value = target.input ? target.input.value.trim() : "";
    if (!value) {
      status(target, text("empty"));
      return;
    }
    var store = load();
    store[id] = {
      text: value.slice(0, LIMIT),
      quote: target.quote || "",
      updatedAt: now()
    };
    if (!persist(store)) return;
    paint(id, store[id]);
    status(target, text("saved"));
    close(target);
  }

  function remove(id) {
    var target = rows[id];
    var store = load();
    if (Object.prototype.hasOwnProperty.call(store, id)) {
      delete store[id];
      if (!persist(store)) return;
    }
    paint(id, null);
    if (target.input) target.input.value = "";
    close(target);
    status(target, text("deleted"));
  }

  function remember() {
    var selection = window.getSelection ? window.getSelection() : null;
    if (!selection || selection.rangeCount === 0 || selection.isCollapsed) return;
    var picked = selection.toString().replace(/\s+/g, " ").trim();
    if (!picked) return;
    var anchor = selection.anchorNode;
    var focus = selection.focusNode;
    if (!anchor || !focus) return;
    var scopes = document.querySelectorAll("[data-note-scope]");
    for (var i = 0; i < scopes.length; i++) {
      if (scopes[i].contains(anchor) && scopes[i].contains(focus)) {
        pending[scopes[i].getAttribute("data-note-scope")] = picked;
        return;
      }
    }
  }

  function captureQuote(id) {
    var target = rows[id];
    var picked = pending[id] || "";
    if (!picked) {
      status(target, text("no-selection"));
      return;
    }
    target.quote = picked.slice(0, QUOTE_LIMIT);
    if (target.quoteText) {
      target.quoteText.textContent = target.quote;
      target.quoteText.hidden = false;
    }
    status(target, "");
  }

  function exportNotes() {
    var store = load();
    var blob = new Blob([JSON.stringify(store, null, 2)], { type: "application/json" });
    var url = URL.createObjectURL(blob);
    var link = document.createElement("a");
    link.href = url;
    link.download = "zgc-notes.json";
    document.body.appendChild(link);
    link.click();
    document.body.removeChild(link);
    window.setTimeout(function () {
      URL.revokeObjectURL(url);
    }, 1000);
    banner(fill(text("exported"), { n: total(store) }));
  }

  function importNotes(file) {
    var reader = new FileReader();
    reader.onload = function () {
      var parsed = null;
      try {
        parsed = JSON.parse(String(reader.result));
      } catch (error) {
        banner(text("import-invalid"));
        return;
      }
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
        banner(text("import-invalid"));
        return;
      }
      var store = load();
      var added = 0;
      for (var id in parsed) {
        if (!Object.prototype.hasOwnProperty.call(parsed, id)) continue;
        var entry = normalize(parsed[id]);
        if (!entry) continue;
        if (!entry.updatedAt) entry.updatedAt = now();
        store[id] = entry;
        added += 1;
      }
      if (!added) {
        banner(text("import-empty"));
        return;
      }
      if (!persist(store)) return;
      paintAll(store);
      banner(fill(text("imported"), { added: added, total: total(store) }));
    };
    reader.onerror = function () {
      banner(text("import-unreadable"));
    };
    reader.readAsText(file);
  }

  function bindRow(id) {
    var target = rows[id];
    if (target.toggle) {
      target.toggle.addEventListener("click", function () {
        toggle(id);
      });
    }
    if (target.saveButton) {
      target.saveButton.addEventListener("click", function () {
        save(id);
      });
    }
    if (target.deleteButton) {
      target.deleteButton.addEventListener("click", function () {
        remove(id);
      });
    }
    if (target.quoteButton) {
      target.quoteButton.addEventListener("click", function () {
        captureQuote(id);
      });
    }
  }

  function bind() {
    for (var id in rows) {
      if (Object.prototype.hasOwnProperty.call(rows, id)) bindRow(id);
    }
    var exportButton = document.querySelector("[data-note-export]");
    if (exportButton) exportButton.addEventListener("click", exportNotes);
    var importButton = document.querySelector("[data-note-import-btn]");
    var importInput = document.querySelector("[data-note-import]");
    if (importButton && importInput) {
      importButton.addEventListener("click", function () {
        importInput.click();
      });
      importInput.addEventListener("change", function () {
        var file = importInput.files && importInput.files[0];
        importInput.value = "";
        if (file) importNotes(file);
      });
    }
    var tools = document.querySelectorAll("[data-note-tools]");
    for (var i = 0; i < tools.length; i++) tools[i].hidden = false;
  }

  function init() {
    collectLabels();
    collectNodes();
    var toggles = document.querySelectorAll("[data-note-toggle]");
    for (var i = 0; i < toggles.length; i++) toggles[i].hidden = false;
    bind();
    document.addEventListener("selectionchange", remember);
    paintAll(load());
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
})();
</script>"#;

// --------------------------------------------------------------- reading page

pub fn read_page(
    app: &App,
    position: Position,
    prefs: &Prefs,
    highlight: Option<(usize, usize)>,
) -> String {
    let data = app.data();
    let Some(paragraph) = data.paragraph(position) else {
        return bad_request_page(app, "內部定位失效：該段不存在。");
    };
    let chapter = data.chapter(position.volume, position.chapter);
    let volume = data.volume(position.volume);
    let Some(paragraph_id) = app.paragraph_id_at(position) else {
        return bad_request_page(app, "內部定位失效：該段不存在。");
    };

    let mut body = String::new();
    body.push_str("<nav class=\"crumbs\"><a href=\"/\">目錄</a>");
    if let Some(group_title) = volume
        .and_then(|volume| data.group(&volume.group_id))
        .map(|group| group.title.clone())
    {
        body.push_str(&format!(
            "<span class=\"sep\">/</span><span class=\"grp\">{}</span>",
            escape(&simplified(app, prefs, &group_title))
        ));
    }
    if let Some(volume) = volume {
        match data
            .volume_start(position.volume)
            .and_then(|start| app.paragraph_id_at(start))
        {
            Some(start_id) => body.push_str(&format!(
                "<span class=\"sep\">/</span><a class=\"vol-link\" href=\"{}\">{}</a>",
                read_href(start_id),
                escape(&simplified(app, prefs, &volume.title))
            )),
            None => body.push_str(&format!(
                "<span class=\"sep\">/</span><span>{}</span>",
                escape(&simplified(app, prefs, &volume.title))
            )),
        }
    }
    let paragraph_total = chapter.map(|chapter| chapter.paragraphs.len()).unwrap_or(0);
    if let Some(chapter) = chapter {
        body.push_str(&format!(
            "<span class=\"sep\">/</span><span class=\"chap\">{}</span>",
            escape(&simplified(app, prefs, &chapter.title))
        ));
    }
    body.push_str(&format!(
        "<span class=\"pos\">第 {} 段 · 共 {} 段{}</span></nav>",
        position.paragraph + 1,
        paragraph_total,
        note_dot(app, prefs, paragraph_id)
    ));

    body.push_str(&format!(
        "<p class=\"story-cta\"><a href=\"{}\">{}</a></p>",
        story_href_paragraph(paragraph_id),
        escape(&simplified(app, prefs, "整章故事視界 →"))
    ));

    body.push_str(&toolbar(app, paragraph_id, prefs));
    body.push_str(&note_runtime(app, prefs));
    // One link, no script: the form and the answer are rendered server-side.
    body.push_str(&format!(
        "<p class=\"ask-cta\"><a href=\"/ask?p={}\">問 AI · 就此段追問</a></p>",
        urlcodec::percent_encode(paragraph_id)
    ));

    if position.paragraph == 0 {
        if let Some(chapter) = chapter {
            body.push_str(&chapter_when(app, &chapter.id, prefs));
            body.push_str(&chapter_strategy(app, &chapter.id, prefs));
        }
    }

    let parts = paragraph_parts(app, paragraph, paragraph_id, prefs, highlight);
    let (orig_heading, simp_note) = original_heading(app, prefs);
    body.push_str(&format!(
        "<div class=\"cols {}\"><section class=\"col col-orig\"><h2>{}</h2><div class=\"note-scope\" data-note-scope=\"{}\">{}{}</div></section><section class=\"col col-tran\"><h2>白話（簡體）</h2>{}</section><section class=\"col col-note\"><h2>注疏<span class=\"count\">{}</span></h2>{}{}</section></div>",
        prefs.layout.css_class(),
        orig_heading,
        escape(paragraph_id),
        parts.original,
        simp_note,
        parts.translation,
        paragraph.annotations.len(),
        parts.notes,
        parts.glossary
    ));
    body.push_str(&note_block(app, prefs, paragraph_id));

    body.push_str("<nav class=\"pager\">");
    body.push_str("<div class=\"pager-row\">");
    body.push_str(&pager_item(
        app,
        prefs,
        "← 上一段",
        data.prev_paragraph(position),
        "已是首段",
        Some("prev"),
    ));
    body.push_str(&pager_item(
        app,
        prefs,
        "下一段 →",
        data.next_paragraph(position),
        "已是末段",
        Some("next"),
    ));
    body.push_str("</div><div class=\"pager-row\">");
    body.push_str(&pager_item(
        app,
        prefs,
        "⇤ 上一章",
        data.prev_chapter(position),
        "已是第一章",
        None,
    ));
    body.push_str(&pager_item(
        app,
        prefs,
        "下一章 ⇥",
        data.next_chapter(position),
        "已是末章",
        None,
    ));
    body.push_str(&pager_item(
        app,
        prefs,
        "⌃ 上一卷",
        data.prev_volume(position),
        "已是首卷",
        None,
    ));
    body.push_str(&pager_item(
        app,
        prefs,
        "下一卷 ⌄",
        data.next_volume(position),
        "已是末卷",
        None,
    ));
    body.push_str("</div><p class=\"keyhint\">← → 鍵：上一段 / 下一段</p></nav>");
    body.push_str(NOTE_SCRIPT);
    body.push_str(KEY_NAV_SCRIPT);

    body.push_str(&format!(
        "<p class=\"pid\">段 id：{} · 卷 {} · 章 {} · 段 {}</p>",
        escape(paragraph_id),
        position.volume,
        position.chapter,
        position.paragraph
    ));

    let chapter_label = chapter
        .map(|chapter| simplified(app, prefs, &chapter.title).into_owned())
        .unwrap_or_else(|| "閱讀".to_string());
    let title = format!("{} · 第 {} 段", chapter_label, position.paragraph + 1);
    document(app, Some(prefs), &title, &prefs.root_style(), "", "", &body)
}

// --------------------------------------------------------- story reading page

/// What happened to the requested AI reading. Owned so the handler can build it
/// from a local result and still render the rest of the page on failure.
pub enum StoryOutcome {
    NotConfigured,
    Answer(AiReply),
    Failure(AiError),
}

/// Everything the story page needs to know about the AI layer.
pub struct StoryView<'a> {
    pub requested: bool,
    pub config: &'a AiConfig,
    pub outcome: Option<StoryOutcome>,
    pub sources: &'a [ExternalSource],
    pub evidence_note: Option<&'a str>,
    /// The 背景史料 sidebar block: lazy unless the reader asked with `bg=1`.
    pub background: BackgroundView,
}

/// What the 背景史料 block has to say. Owned so the handler can hand back either
/// five hits or the reason there are none, and the page still renders.
pub enum BackgroundView {
    NotRequested,
    NotConfigured,
    Hits(Vec<ExternalSource>),
    Failure(AiError),
}

/// The system prompt is part of the page contract: four sections, source ids
/// after every claim, and an explicit refusal to invent when material is thin.
pub const STORY_SYSTEM_PROMPT: &str =
    "你是《戰國策》的註釋者。材料只有兩類：【原文】（含注疏）與【外部資料】。\
只依據所給材料作答，不得引入材料以外的史實、人物、年代。\
輸出四節，標題依次為「背景」「寓意」「可學之處」「參考來源」；標題單獨一行，不加 Markdown 符號（#、**）或代碼圍欄。\
前三節的每一條判斷後面用〔段 id〕標注原文依據；用到外部資料時同時標注【來源編號】。\
外部資料與原文衝突時，以原文為準。\
材料不足時直接寫「材料不足」，不得推測或編造。\
「參考來源」只列正文實際用到的外部資料，逐條寫 title · site_name · url；若無外部資料，寫「無」。\
全文中文字數 360–700 字，不作開場白。";

/// Search query for the evidence layer: chapter name, country and reading
/// keywords, capped at the provider's 128-character query limit.
pub fn story_search_query(app: &App, position: Position) -> String {
    let data = app.data();
    let chapter = data
        .chapter(position.volume, position.chapter)
        .map(|chapter| chapter.title.as_str())
        .unwrap_or_default();
    let group = data
        .volume(position.volume)
        .and_then(|volume| data.group(&volume.group_id))
        .map(|group| group.title.as_str())
        .unwrap_or_default();
    format!("{chapter} {group} 戰國策 寓意 賞析 背景")
        .chars()
        .take(128)
        .collect()
}

/// The user prompt: chapter identity, every paragraph keyed by its id, the
/// 注疏 attached to each, then the external material when there is any.
pub fn story_prompt(app: &App, position: Position, sources: &[ExternalSource]) -> String {
    let data = app.data();
    let Some(chapter) = data.chapter(position.volume, position.chapter) else {
        return String::new();
    };
    let group = data
        .volume(position.volume)
        .and_then(|volume| data.group(&volume.group_id))
        .map(|group| group.title.clone())
        .unwrap_or_default();
    let volume = data
        .volume(position.volume)
        .map(|volume| volume.title.clone())
        .unwrap_or_default();

    let mut out = String::new();
    out.push_str("《戰國策》故事解讀材料\n");
    out.push_str(&format!(
        "國別：{}\n卷：{}\n章名：{}\n\n【原文】\n",
        group, volume, chapter.title
    ));
    for paragraph in &chapter.paragraphs {
        out.push_str(&format!("〔{}〕{}\n", paragraph.id, paragraph.text));
        for annotation in &paragraph.annotations {
            let source = annotation
                .source
                .as_deref()
                .map(str::trim)
                .filter(|source| !source.is_empty())
                .unwrap_or("未署");
            let edition = annotation
                .edition
                .as_deref()
                .map(str::trim)
                .filter(|edition| !edition.is_empty())
                .unwrap_or("四部叢刊");
            out.push_str(&format!(
                "　注疏（{}·{}）：{}\n",
                source, edition, annotation.text
            ));
        }
    }
    if !sources.is_empty() {
        out.push_str("\n【外部資料】\n");
        for (index, source) in sources.iter().enumerate() {
            out.push_str(&format!(
                "【來源 {}】{} · {} · {}\n",
                index + 1,
                source.title,
                source.site_name,
                source.url
            ));
            if !source.summary.trim().is_empty() {
                out.push_str(&format!("摘要：{}\n", source.summary.trim()));
            }
            if let Some(content) = source.content.as_deref() {
                out.push_str(&format!("全文：{}\n", content.trim()));
            }
        }
    }
    out.push_str("\n請依系統指示輸出四節。\n");
    out
}

/// One story: the whole chapter in order, with one anchor per paragraph.
pub fn story_page(app: &App, position: Position, prefs: &Prefs, view: &StoryView<'_>) -> String {
    let data = app.data();
    let Some(chapter) = data.chapter(position.volume, position.chapter) else {
        return bad_request_page(app, "內部定位失效：該章不存在。");
    };
    let volume = data.volume(position.volume);

    let mut body = String::new();
    body.push_str("<nav class=\"crumbs\"><a href=\"/\">目錄</a>");
    if let Some(group_title) = volume
        .and_then(|volume| data.group(&volume.group_id))
        .map(|group| group.title.clone())
    {
        body.push_str(&format!(
            "<span class=\"sep\">/</span><span class=\"grp\">{}</span>",
            escape(&simplified(app, prefs, &group_title))
        ));
    }
    if let Some(volume) = volume {
        match data
            .volume_start(position.volume)
            .and_then(|start| app.paragraph_id_at(start))
        {
            Some(start_id) => body.push_str(&format!(
                "<span class=\"sep\">/</span><a class=\"vol-link\" href=\"{}\">{}</a>",
                read_href(start_id),
                escape(&simplified(app, prefs, &volume.title))
            )),
            None => body.push_str(&format!(
                "<span class=\"sep\">/</span><span>{}</span>",
                escape(&simplified(app, prefs, &volume.title))
            )),
        }
    }
    body.push_str(&format!(
        "<span class=\"sep\">/</span><span class=\"chap\">{}</span></nav>",
        escape(&simplified(app, prefs, &chapter.title))
    ));
    body.push_str(&format!(
        "<h1 class=\"page-title story-title\">{}</h1>",
        escape(&simplified(app, prefs, &chapter.title))
    ));
    body.push_str(&format!(
        "<p class=\"story-meta\">本故事共 {} 段</p>",
        chapter.paragraphs.len()
    ));
    body.push_str(&story_chips(app, &chapter.id, prefs));
    body.push_str(&note_runtime(app, prefs));

    // Sidebar and story share one layout: the reading column first in the
    // markup, the sidebar beside it, and CSS lifts the sidebar above the story
    // when the screen is too narrow for two columns.
    body.push_str("<div class=\"story-layout\" data-arrow-step=\"section[data-story-para]\" data-arrow-keys=\"ud\" data-arrow-nav=\"chapter\"><div class=\"story-main\">");
    for (index, paragraph) in chapter.paragraphs.iter().enumerate() {
        body.push_str(&story_paragraph(app, prefs, paragraph, index + 1));
    }

    body.push_str(&story_ai_block(&chapter.id, view));

    body.push_str("<nav class=\"pager story-pager\"><div class=\"pager-row\">");
    body.push_str(&story_pager_item(
        app,
        prefs,
        "← 上一章",
        data.prev_chapter(position),
        "已是第一章",
        "prev",
    ));
    body.push_str(&story_pager_item(
        app,
        prefs,
        "下一章 →",
        data.next_chapter(position),
        "已是末章",
        "next",
    ));
    body.push_str(&format!(
        "</div><p class=\"keyhint\">{}</p></nav>",
        escape(&simplified(app, prefs, "← → 鍵：上一章 / 下一章 · ↑ ↓ 鍵：前後段"))
    ));
    body.push_str(NOTE_SCRIPT);
    body.push_str(ARROW_STEP_SCRIPT);
    body.push_str("</div>");
    body.push_str(&story_sidebar(app, prefs, chapter, &chapter.id, view));
    body.push_str("</div>");

    let title = format!(
        "{} · 故事視界",
        simplified(app, prefs, &chapter.title).into_owned()
    );
    document(app, Some(prefs), &title, &prefs.root_style(), "", "", &body)
}

fn story_paragraph(app: &App, prefs: &Prefs, paragraph: &Paragraph, ordinal: usize) -> String {
    let parts = paragraph_parts(app, paragraph, &paragraph.id, prefs, None);
    let (orig_heading, simp_note) = original_heading(app, prefs);
    let glossary = if parts.glossary.is_empty() {
        "<p class=\"empty\">暫無字詞訓釋</p>".to_string()
    } else {
        parts.glossary
    };
    format!(
        "<section class=\"story-para\" id=\"{}\" data-story-para>\
<p class=\"story-ord\">第 {} 段 · <code>{}</code>{}</p>\
<h2 class=\"story-h\">{}</h2><div class=\"note-scope\" data-note-scope=\"{}\">{}{}</div>\
<h2 class=\"story-h\">白話（簡體）</h2>{}\
<h2 class=\"story-h\">注疏<span class=\"count\">{}</span></h2>{}\
<h2 class=\"story-h\">字詞訓釋</h2>{}\
<p class=\"story-more\"><a href=\"{}\">單段精讀 →</a></p>{}</section>",
        escape(&paragraph.id),
        ordinal,
        escape(&paragraph.id),
        note_dot(app, prefs, &paragraph.id),
        orig_heading,
        escape(&paragraph.id),
        parts.original,
        simp_note,
        parts.translation,
        paragraph.annotations.len(),
        parts.notes,
        glossary,
        read_href(&paragraph.id),
        note_block(app, prefs, &paragraph.id)
    )
}

/// Sidebar roster cap: the twelve most frequent names of the chapter.
const STORY_SIDE_LIMIT: usize = 12;

/// One chapter-local entity row: the curated record plus its span count.
struct ChapterEntity<'a> {
    entity: &'a Entity,
    count: usize,
}

/// Count the spans the chapter actually carries, per entity, and keep the most
/// frequent. One pass over the chapter's own paragraphs and their spans — the
/// whole book is never walked.
fn chapter_entities<'a>(
    app: &'a App,
    chapter: &Chapter,
    kind: EntityKind,
) -> Vec<ChapterEntity<'a>> {
    let entities = app.entities();
    let mut counts: HashMap<&'a str, (&'a Entity, usize)> = HashMap::new();
    for paragraph in &chapter.paragraphs {
        for span in entities.spans_in(&paragraph.id) {
            if span.entity.kind != kind {
                continue;
            }
            let row = counts
                .entry(span.entity.id.as_str())
                .or_insert((span.entity, 0));
            row.1 += 1;
        }
    }
    let mut rows: Vec<ChapterEntity<'a>> = counts
        .into_values()
        .map(|(entity, count)| ChapterEntity { entity, count })
        .collect();
    // Most frequent first; the name breaks ties so the order never depends on
    // hash iteration.
    rows.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.entity.name.cmp(&right.entity.name))
    });
    rows.truncate(STORY_SIDE_LIMIT);
    rows
}

/// One name in the reader's script state: 繁體 by default, the curated 簡體 in
/// 簡體 mode, and both in 對照 mode. Never invents a name the data lacks.
fn entity_name(app: &App, prefs: &Prefs, entity: &Entity) -> String {
    if simp_mode(app, prefs) {
        let name = entity.name_simp_text().unwrap_or(&entity.name);
        return escape(&simplified(app, prefs, name));
    }
    let mut out = escape(&entity.name);
    if both_mode(app, prefs) {
        if let Some(simp) = entity.name_simp_text() {
            out.push_str(&format!("<span class=\"simp\">{}</span>", escape(simp)));
        }
    }
    out
}

/// One roster: heading, then name + count rows linking to the entity page.
fn story_entity_block(
    app: &App,
    prefs: &Prefs,
    heading: &str,
    empty: &str,
    rows: &[ChapterEntity<'_>],
) -> String {
    let mut out = format!(
        "<section class=\"side-block\"><h2>{}</h2>",
        escape(&simplified(app, prefs, heading))
    );
    if rows.is_empty() {
        out.push_str(&format!(
            "<p class=\"side-empty\">{}</p>",
            escape(&simplified(app, prefs, empty))
        ));
    } else {
        out.push_str("<ol class=\"side-list\">");
        for row in rows {
            out.push_str(&format!(
                "<li><a href=\"/entity?id={}\">{}</a><span class=\"side-count\">{}</span></li>",
                urlcodec::percent_encode(&row.entity.id),
                entity_name(app, prefs, row.entity),
                row.count
            ));
        }
        out.push_str("</ol>");
    }
    out.push_str("</section>");
    out
}

/// The 背景史料 link carries the reader's current display state, so asking for
/// the search does not reset 三態 or the layout.
fn story_bg_href(chapter_id: &str, prefs: &Prefs) -> String {
    format!(
        "/story?c={}&amp;bg=1&amp;sc={}&amp;layout={}&amp;orig={}&amp;tran={}&amp;lh={}",
        urlcodec::percent_encode(chapter_id),
        prefs.script.key(),
        prefs.layout.key(),
        prefs.orig.key(),
        prefs.tran.key(),
        prefs.leading.key()
    )
}

/// The 背景史料 block: a lazy link by default, five cited hits after `bg=1`,
/// and a reasoned degradation — never a blank — when the search cannot run.
fn story_background_block(
    app: &App,
    prefs: &Prefs,
    chapter_id: &str,
    view: &StoryView<'_>,
) -> String {
    let mut out = format!(
        "<section class=\"side-block side-bg\"><h2>{}</h2>",
        escape(&simplified(app, prefs, "背景史料"))
    );
    match &view.background {
        BackgroundView::NotRequested => {
            out.push_str(&format!(
                "<p class=\"side-cta\"><a href=\"{}\">{}</a></p>",
                story_bg_href(chapter_id, prefs),
                escape(&simplified(app, prefs, "檢索背景史料 →"))
            ));
        }
        BackgroundView::NotConfigured => {
            out.push_str(&format!(
                "<p class=\"side-empty\">未取到史料：AI 未配置（缺少 {}）</p>",
                escape(&view.config.missing().join(" / "))
            ));
        }
        BackgroundView::Failure(error) => {
            out.push_str(&format!(
                "<p class=\"side-empty\">未取到史料：{}</p><p class=\"hint\">其餘內容照常顯示，可稍後重試。</p>",
                escape(&error.human())
            ));
        }
        BackgroundView::Hits(hits) if hits.is_empty() => {
            out.push_str("<p class=\"side-empty\">未取到史料：檢索沒有結果。</p>");
        }
        BackgroundView::Hits(hits) => {
            out.push_str("<p class=\"side-note\">外部資料來自公開網頁，未經校勘。</p><ol class=\"side-sources\">");
            for hit in hits {
                out.push_str("<li>");
                let title = escape(&hit.title);
                if http_url(&hit.url) {
                    out.push_str(&format!(
                        "<a href=\"{}\" target=\"_blank\" rel=\"noreferrer noopener\">{}</a>",
                        escape(&hit.url),
                        title
                    ));
                } else {
                    out.push_str(&title);
                }
                if !hit.site_name.trim().is_empty() {
                    out.push_str(&format!(
                        "<span class=\"site\">{}</span>",
                        escape(&hit.site_name)
                    ));
                }
                if !hit.summary.trim().is_empty() {
                    out.push_str(&format!(
                        "<p class=\"sum\">{}</p>",
                        clip_escape(hit.summary.trim(), 200)
                    ));
                }
                out.push_str("</li>");
            }
            out.push_str("</ol>");
        }
    }
    out.push_str("</section>");
    out
}

/// The sticky sidebar: 本章人物, 本章地點, 系年, 背景史料 — all static HTML, so a
/// reader without JavaScript sees the same three blocks.
fn story_sidebar(
    app: &App,
    prefs: &Prefs,
    chapter: &Chapter,
    chapter_id: &str,
    view: &StoryView<'_>,
) -> String {
    let mut out = String::from("<aside class=\"story-side\">");
    out.push_str(&story_entity_block(
        app,
        prefs,
        "本章人物",
        "本章未見人物",
        &chapter_entities(app, chapter, EntityKind::Person),
    ));
    out.push_str(&story_entity_block(
        app,
        prefs,
        "本章地點",
        "本章未見地點",
        &chapter_entities(app, chapter, EntityKind::Place),
    ));
    let when = chapter_when(app, chapter_id, prefs);
    out.push_str(&format!(
        "<section class=\"side-block side-when\"><h2>{}</h2>",
        escape(&simplified(app, prefs, "系年"))
    ));
    if when.is_empty() {
        out.push_str(&format!(
            "<p class=\"side-empty\">{}</p>",
            escape(&simplified(app, prefs, "此書未附系年表"))
        ));
    } else {
        out.push_str(&when);
    }
    out.push_str("</section>");
    out.push_str(&story_background_block(app, prefs, chapter_id, view));
    out.push_str("</aside>");
    out
}

fn story_chips(app: &App, chapter_id: &str, prefs: &Prefs) -> String {
    let strategy = app.strategy();
    let Some(chapter) = strategy.chapter(chapter_id) else {
        return String::new();
    };
    if chapter.turns.is_empty() && chapter.devices.is_empty() {
        return String::new();
    }
    let mut out = String::from("<div class=\"story-plan\"><span class=\"seg-label\">謀略</span>");
    if !chapter.turns.is_empty() {
        out.push_str(&format!(
            "<span class=\"chip\">{} 輪遊說</span>",
            chapter.turns.len()
        ));
    }
    for kind in strategy.kinds() {
        let Some(first) = chapter
            .devices
            .iter()
            .find(|device| device.kind == kind.kind)
        else {
            continue;
        };
        let count = chapter
            .devices
            .iter()
            .filter(|device| device.kind == kind.kind)
            .count();
        out.push_str(&format!(
            "<a class=\"chip\" href=\"{}\">{} · {}</a>",
            read_hl_href(&first.p, first.offset, first.len),
            escape(&simplified(app, prefs, &kind.kind)),
            count
        ));
    }
    out.push_str("</div>");
    out
}

fn story_ai_block(chapter_id: &str, view: &StoryView<'_>) -> String {
    let mut out = String::from("<section class=\"story-ai\"><h2>故事解讀</h2>");
    out.push_str("<p class=\"privacy\">解讀由模型實時生成，可能出錯，須以原文與注疏為準。</p>");
    match &view.outcome {
        None => {
            if view.requested {
                out.push_str("<div class=\"ai-error\"><h2>故事解讀沒有生成</h2><p>沒有可用的上游位址。</p><p class=\"hint\">其餘內容照常顯示；解讀沒有寫入任何檔案。</p></div>");
            } else {
                out.push_str(&format!(
                    "<p class=\"story-ai-cta\"><a class=\"chip\" href=\"/story?c={}&amp;ai=1\">生成解讀（背景 / 寓意 / 可學）</a></p>",
                    urlcodec::percent_encode(chapter_id)
                ));
            }
        }
        Some(StoryOutcome::NotConfigured) => {
            out.push_str(&config_block(view.config));
        }
        Some(StoryOutcome::Failure(error)) => {
            out.push_str(&format!(
                "<div class=\"ai-error\"><h2>故事解讀沒有生成</h2><p>{}</p><p class=\"hint\">其餘內容照常顯示；解讀沒有寫入任何檔案，可以稍後重試。</p></div>",
                escape(&error.human())
            ));
        }
        Some(StoryOutcome::Answer(reply)) => {
            if let Some(note) = view.evidence_note {
                out.push_str(&format!(
                    "<p class=\"story-evidence-note\">{}</p>",
                    escape(note)
                ));
            }
            out.push_str(&story_answer_block(reply));
            if !view.sources.is_empty() {
                out.push_str("<p class=\"story-source-note\">外部資料來自公開網頁，未經校勘。</p>");
                out.push_str(&story_sources(view.sources));
            }
        }
    }
    out.push_str("</section>");
    out
}

fn story_sources(sources: &[ExternalSource]) -> String {
    let mut out = String::from("<h3>參考來源</h3><ol class=\"story-sources\">");
    for source in sources {
        out.push_str("<li>");
        let title = escape(&source.title);
        if http_url(&source.url) {
            out.push_str(&format!(
                "<a href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\">{}</a>",
                escape(&source.url),
                title
            ));
        } else {
            out.push_str(&title);
        }
        if !source.site_name.trim().is_empty() {
            out.push_str(&format!(
                "<span class=\"site\">{}</span>",
                escape(&source.site_name)
            ));
        }
        out.push_str("</li>");
    }
    out.push_str("</ol>");
    out
}

fn http_url(url: &str) -> bool {
    let url = url.trim();
    (url.starts_with("http://") || url.starts_with("https://"))
        && !url.chars().any(char::is_whitespace)
}

fn story_pager_item(
    app: &App,
    prefs: &Prefs,
    label: &str,
    target: Option<Position>,
    disabled_label: &str,
    nav: &str,
) -> String {
    let Some(position) = target else {
        return format!(
            "<span class=\"pager-item disabled\">{}</span>",
            escape(disabled_label)
        );
    };
    let Some(chapter) = app.data().chapter(position.volume, position.chapter) else {
        return format!(
            "<span class=\"pager-item disabled\">{}</span>",
            escape(disabled_label)
        );
    };
    format!(
        "<a class=\"pager-item\" data-story-nav=\"{}\" href=\"/story?c={}\">{}<span class=\"to\">{}</span></a>",
        nav,
        urlcodec::percent_encode(&chapter.id),
        escape(label),
        escape(&simplified(app, prefs, &chapter.title))
    )
}

/// The one arrow-key rule, shared by 目錄 and /story: ← → walk a list of
/// step markers, ↑ ↓ scroll between paragraphs on the story page. The nearest
/// marker to the viewport top is the cursor, so a page never tracks state.
///
/// /read is not here on purpose: its arrows follow links to another paragraph
/// page, which is a different move (KEY_NAV_SCRIPT).
const ARROW_STEP_SCRIPT: &str = r#"<script>
(function () {
  // Each page declares its own arrow contract in markup, so one script serves
  // the catalogue and the story page without guessing which page it landed on.
  var root = document.querySelector("[data-arrow-step]");
  if (!root) return;
  var stepSelector = root.getAttribute("data-arrow-step");
  var horizontal = root.getAttribute("data-arrow-keys") === "lr";
  var chapterNav = root.getAttribute("data-arrow-nav") === "chapter";
  function nearest(list) {
    var best = 0;
    var bestDistance = Infinity;
    for (var i = 0; i < list.length; i++) {
      var distance = Math.abs(list[i].getBoundingClientRect().top);
      if (distance < bestDistance) { bestDistance = distance; best = i; }
    }
    return best;
  }
  function step(delta) {
    var list = document.querySelectorAll(stepSelector);
    if (!list.length) return false;
    var next = Math.max(0, Math.min(list.length - 1, nearest(list) + delta));
    list[next].scrollIntoView({ behavior: "smooth", block: "start" });
    return true;
  }
  document.addEventListener("keydown", function (event) {
    if (event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) return;
    var target = event.target;
    if (target && (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName))) return;
    var left = event.key === "ArrowLeft";
    var right = event.key === "ArrowRight";
    var up = event.key === "ArrowUp";
    var down = event.key === "ArrowDown";
    if (left || right) {
      // 章 navigation is horizontal by nature, even on a page whose own steps
      // run vertically (the story page scrolls paragraphs with ↑ ↓).
      if (chapterNav) {
        var link = document.querySelector('a[data-story-nav="' + (left ? "prev" : "next") + '"]');
        if (!link) return;
        event.preventDefault();
        window.location.href = link.getAttribute("href");
        return;
      }
      if (horizontal && step(left ? -1 : 1)) event.preventDefault();
      return;
    }
    if (!horizontal && (up || down)) {
      if (step(up ? -1 : 1)) event.preventDefault();
    }
  });
})();
</script>"#;

fn toolbar(app: &App, paragraph_id: &str, prefs: &Prefs) -> String {
    let mut layout_chips = String::new();
    for layout in Layout::ALL {
        layout_chips.push_str(&chip(
            paragraph_id,
            "layout",
            layout.key(),
            layout.label(),
            prefs.layout == layout,
        ));
    }
    let mut orig_chips = String::new();
    for step in FontStep::ALL {
        orig_chips.push_str(&chip(
            paragraph_id,
            "orig",
            step.key(),
            step.label(),
            prefs.orig == step,
        ));
    }
    let mut tran_chips = String::new();
    for step in FontStep::ALL {
        tran_chips.push_str(&chip(
            paragraph_id,
            "tran",
            step.key(),
            step.label(),
            prefs.tran == step,
        ));
    }
    let mut leading_chips = String::new();
    for leading in Leading::ALL {
        leading_chips.push_str(&chip(
            paragraph_id,
            "lh",
            leading.key(),
            leading.label(),
            prefs.leading == leading,
        ));
    }
    // Three-state 對照 / 繁 / 简 toggle. Hidden when no table loaded: there is
    // nothing to switch to, and a dead link would be a lie.
    let mut script_seg = String::new();
    if !app.variants().is_empty() {
        let mut script_chips = String::new();
        for script in Script::ALL {
            script_chips.push_str(&chip_titled(
                paragraph_id,
                "sc",
                script.key(),
                script.toggle_label(),
                prefs.script == script,
                Some(script.label()),
            ));
        }
        script_seg = format!(
            "<span class=\"seg seg-script\"><span class=\"seg-label\">字形</span>{script_chips}</span>"
        );
    }
    format!(
        "<div class=\"toolbar\"><span class=\"seg\"><span class=\"seg-label\">版面</span>{}</span><span class=\"seg\"><span class=\"seg-label\">原文字號</span>{}</span><span class=\"seg\"><span class=\"seg-label\">白話字號</span>{}</span><span class=\"seg\"><span class=\"seg-label\">行距</span>{}</span>{}{}</div>",
        layout_chips,
        orig_chips,
        tran_chips,
        leading_chips,
        script_seg,
        note_controls(app, prefs)
    )
}

fn chip(paragraph_id: &str, parameter: &str, value: &str, label: &str, current: bool) -> String {
    chip_titled(paragraph_id, parameter, value, label, current, None)
}

/// A chip with hover text. The 字形 toggle shows one glyph (繁 / 简), so the
/// full name belongs in the title rather than in the toolbar row.
fn chip_titled(
    paragraph_id: &str,
    parameter: &str,
    value: &str,
    label: &str,
    current: bool,
    title: Option<&str>,
) -> String {
    let title = title
        .map(|title| format!(" title=\"{}\"", escape(title)))
        .unwrap_or_default();
    if current {
        format!(
            "<span class=\"chip on\" aria-current=\"true\"{title}>{}</span>",
            escape(label)
        )
    } else {
        format!(
            "<a class=\"chip\" href=\"{}&amp;{}={}\"{title}>{}</a>",
            read_href(paragraph_id),
            escape(parameter),
            escape(value),
            escape(label)
        )
    }
}

/// Arrow keys move one paragraph at a time, mirroring the 上一段/下一段 links.
/// Modified keys and typing targets are left alone so the shortcut never steals input.
const KEY_NAV_SCRIPT: &str = r#"<script>
(function () {
  var keys = { ArrowLeft: "prev", ArrowRight: "next" };
  document.addEventListener("keydown", function (event) {
    var kind = keys[event.key];
    if (!kind || event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) return;
    var target = event.target;
    if (target && (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName))) return;
    var link = document.querySelector('a[data-nav="' + kind + '"]');
    if (!link) return;
    event.preventDefault();
    window.location.href = link.getAttribute("href");
  });
})();
</script>"#;

fn pager_item(
    app: &App,
    prefs: &Prefs,
    label: &str,
    target: Option<Position>,
    disabled_label: &str,
    nav: Option<&str>,
) -> String {
    let Some(position) = target else {
        return format!(
            "<span class=\"pager-item disabled\">{}</span>",
            escape(disabled_label)
        );
    };
    let Some(paragraph_id) = app.paragraph_id_at(position) else {
        return format!(
            "<span class=\"pager-item disabled\">{}</span>",
            escape(disabled_label)
        );
    };
    let target_title = app
        .data()
        .chapter(position.volume, position.chapter)
        .map(|chapter| chapter.title.clone())
        .unwrap_or_default();
    let nav_attr = nav
        .map(|kind| format!(" data-nav=\"{}\"", kind))
        .unwrap_or_default();
    format!(
        "<a class=\"pager-item\"{} href=\"{}\">{}<span class=\"to\">{}</span></a>",
        nav_attr,
        read_href(paragraph_id),
        escape(label),
        escape(&simplified(app, prefs, &target_title))
    )
}

// ------------------------------------------------------- entity-aware text

/// Render the traditional-text column line by line, turning entity spans into
/// links. Offsets are characters in the whole paragraph; every visual line
/// starts at its own character offset because '\n' counts as one character.
fn original_html(
    app: &App,
    text: &str,
    paragraph_id: &str,
    highlight: Option<(usize, usize)>,
    prefs: &Prefs,
) -> String {
    let simp = simp_mode(app, prefs);
    let spans: Vec<SpanView> = app.entities().spans_in(paragraph_id).collect();
    // hl must land on an entity or on a recorded device span, or it is silently
    // ignored. An exact entity match wins; a range inside one span is accepted;
    // a device mark is only ever the exact span the strategy table recorded, so
    // a hand-written hl can never paint arbitrary text.
    let highlighted = highlight.and_then(|(offset, len)| {
        spans
            .iter()
            .find(|span| span.offset == offset && span.len == len)
            .or_else(|| {
                spans
                    .iter()
                    .find(|span| span.offset <= offset && offset + len <= span.offset + span.len)
            })
            .map(|span| (span.offset, span.len))
    });
    let marks: Vec<(usize, usize)> = match highlighted {
        Some(_) => Vec::new(),
        None => app
            .strategy()
            .devices_in(paragraph_id)
            .iter()
            .copied()
            .filter(|span| highlight == Some(*span))
            .collect(),
    };

    let both = both_mode(app, prefs);
    let mut out = String::with_capacity(text.len() * if both { 3 } else { 2 });
    let mut base = 0usize;
    for line in text.split('\n') {
        let original_len = line.chars().count();
        let converted = if simp || both {
            Some(app.variants().to_simplified(line))
        } else {
            None
        };
        let source: Vec<char> = line.chars().collect();
        let converted_chars: Vec<char> = converted
            .as_deref()
            .map(|text| text.chars().collect())
            .unwrap_or_default();
        // Entity offsets are computed on the 繁體 source. The table never
        // changes the character count, but a table edited by hand could, and a
        // shifted link would point at the wrong glyphs: in that case drop the
        // anchors and marks for this line rather than misplace them.
        let shown: &[char] = if simp { &converted_chars } else { &source };
        let aligned = shown.len() == original_len;
        if !line.trim().is_empty() {
            let segments = line_segments(&spans, &marks, base, base + original_len);
            out.push_str("<p class=\"orig-line\">");
            if aligned {
                out.push_str(&render_line(
                    app,
                    prefs,
                    shown,
                    base,
                    &segments,
                    highlighted,
                ));
            } else {
                out.push_str(&escape(&shown.iter().collect::<String>()));
            }
            out.push_str("</p>");
            // 對照: the 簡體 counterpart directly beneath, its own line but
            // never its own link target — anchors and ids stay on the 繁體
            // line above.
            if both {
                out.push_str("<p class=\"simp-line\">");
                if aligned {
                    out.push_str(&render_plain_line(
                        &converted_chars,
                        base,
                        &segments,
                        highlighted,
                    ));
                } else {
                    out.push_str(&escape(&converted_chars.iter().collect::<String>()));
                }
                out.push_str("</p>");
            }
        }
        base += original_len + 1;
    }
    out
}

/// The drawing plan for one visual line: entity spans and recorded device
/// marks that start inside it, clamped to its end and ordered so the wider
/// segment wins an overlap. 對照 mode reuses the plan for the 簡體 line: the
/// table keeps the character count, so the same offsets still name the same
/// glyph range.
fn line_segments<'a, 's>(
    spans: &'s [SpanView<'a>],
    marks: &[(usize, usize)],
    base: usize,
    line_end: usize,
) -> Vec<(usize, usize, Option<&'s SpanView<'a>>)> {
    let mut segments: Vec<(usize, usize, Option<&'s SpanView<'a>>)> = spans
        .iter()
        .filter(|span| span.offset >= base && span.offset < line_end)
        .map(|span| {
            (
                span.offset,
                (span.offset + span.len).min(line_end),
                Some(span),
            )
        })
        .collect();
    for &(offset, len) in marks {
        if offset >= base && offset < line_end {
            let stop = (offset + len).min(line_end);
            if stop > offset {
                segments.push((offset, stop, None));
            }
        }
    }
    segments.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    segments
}

/// One visual line: plain text (escaped) with entity anchors and, when the
/// reader asked for one, a plain mark over a recorded device span. An entity
/// wins any overlap: its link is the more specific claim about the glyphs.
fn render_line(
    app: &App,
    prefs: &Prefs,
    line: &[char],
    base: usize,
    segments: &[(usize, usize, Option<&SpanView>)],
    highlighted: Option<(usize, usize)>,
) -> String {
    let mut out = String::with_capacity(line.len() * 3 + 32);
    let mut cursor = base;
    for &(start, stop, span) in segments {
        if start < cursor {
            continue; // overlaps something already drawn: the wider one won
        }
        out.push_str(&escape(
            &line[cursor - base..start - base].iter().collect::<String>(),
        ));
        let glyphs: String = line[start - base..stop - base].iter().collect();
        match span {
            Some(span) => out.push_str(&entity_anchor(
                span,
                &glyphs,
                highlighted == Some((span.offset, span.len)),
                &simplified(app, prefs, &span.entity.name),
            )),
            None => out.push_str(&format!("<mark>{}</mark>", escape(&glyphs))),
        }
        cursor = stop;
    }
    out.push_str(&escape(&line[cursor - base..].iter().collect::<String>()));
    out
}

/// The 簡體 line under its 繁體 source: the same segments, the same highlight,
/// but no anchors and no ids — links and tab stops belong to the 繁體 line, so
/// the page never carries a duplicate target.
fn render_plain_line(
    line: &[char],
    base: usize,
    segments: &[(usize, usize, Option<&SpanView>)],
    highlighted: Option<(usize, usize)>,
) -> String {
    let mut out = String::with_capacity(line.len() * 2 + 32);
    let mut cursor = base;
    for &(start, stop, span) in segments {
        if start < cursor {
            continue; // overlaps something already drawn: the wider one won
        }
        out.push_str(&escape(
            &line[cursor - base..start - base].iter().collect::<String>(),
        ));
        let glyphs: String = line[start - base..stop - base].iter().collect();
        let marked = match span {
            Some(span) => highlighted == Some((span.offset, span.len)),
            None => true,
        };
        if marked {
            out.push_str(&format!("<mark>{}</mark>", escape(&glyphs)));
        } else {
            out.push_str(&escape(&glyphs));
        }
        cursor = stop;
    }
    out.push_str(&escape(&line[cursor - base..].iter().collect::<String>()));
    out
}

fn entity_anchor(span: &SpanView, glyphs: &str, highlighted: bool, name: &str) -> String {
    let entity = span.entity;
    let mut out = format!(
        "<a class=\"ent ent-{}{}\" href=\"/entity?id={}\" title=\"{} · {}\">",
        entity.kind.as_str(),
        if highlighted { " ent-hl" } else { "" },
        urlcodec::percent_encode(&entity.id),
        escape(entity.kind.label()),
        escape(name)
    );
    if highlighted {
        out.push_str("<mark>");
    }
    out.push_str(&escape(glyphs));
    if highlighted {
        out.push_str("</mark>");
    }
    out.push_str("</a>");
    out
}

// ---------------------------------------------------------------- entity pages

/// How the entity index is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntitySort {
    /// Most occurrences first (the default: it surfaces the important names).
    Count,
    /// By traditional name, for lookup.
    Name,
}

pub fn entity_page(app: &App, prefs: &Prefs, entity: &Entity) -> String {
    let data = app.data();
    let kind = entity.kind;
    let mut body = String::new();
    body.push_str(&format!(
        "<nav class=\"crumbs\"><a href=\"/\">目錄</a><span class=\"sep\">/</span><a href=\"/entities\">實體索引</a><span class=\"sep\">/</span><a href=\"/entities?kind={}\">{}</a></nav>",
        kind.as_str(),
        escape(kind.label())
    ));
    body.push_str(&format!(
        "<h1 class=\"page-title\">{}</h1>",
        labelled(app, prefs, &entity.name, entity.name_simp_text())
    ));

    let mut paragraph_count = 0usize;
    let mut last = "";
    for occurrence in &entity.occurrences {
        if occurrence.paragraph_id != last {
            paragraph_count += 1;
            last = occurrence.paragraph_id.as_str();
        }
    }
    body.push_str(&format!(
        "<p class=\"stats\"><span class=\"tag tag-kind\">{}</span> 出現 {} 次 · 見於 {} 段</p>",
        escape(kind.label()),
        entity.occurrence_count(),
        paragraph_count
    ));

    let aliases = if entity.aliases.is_empty() {
        "<span class=\"empty\">暫無</span>".to_string()
    } else {
        entity
            .aliases
            .iter()
            .map(|alias| escape(&simplified(app, prefs, alias)))
            .collect::<Vec<_>>()
            .join("、")
    };
    let tags = if entity.tags.is_empty() {
        "<span class=\"empty\">暫無</span>".to_string()
    } else {
        entity
            .tags
            .iter()
            .map(|tag| {
                format!(
                    "<span class=\"tag\">{}</span>",
                    escape(&simplified(app, prefs, tag))
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    let summary = match entity.summary_text() {
        Some(summary) => format!(
            "<p class=\"summary\">{}</p>",
            escape(&simplified(app, prefs, summary))
        ),
        None => "<p class=\"empty\">暫無</p>".to_string(),
    };
    body.push_str(&format!(
        "<dl class=\"facts\"><div><dt>別名</dt><dd>{}</dd></div><div><dt>標籤</dt><dd>{}</dd></div><div><dt>概述</dt><dd>{}</dd></div></dl>",
        aliases, tags, summary
    ));

    body.push_str(&format!(
        "<section class=\"occurrences\"><h2>出現處<span class=\"count\">{}</span></h2>",
        entity.occurrence_count()
    ));
    if entity.occurrences.is_empty() {
        body.push_str("<p class=\"empty\">暫無</p>");
    }
    for (volume_index, chapters) in grouped_occurrences(app, entity) {
        let volume = data.volume(volume_index);
        let volume_title = volume
            .map(|volume| volume.title.clone())
            .unwrap_or_default();
        body.push_str("<section class=\"occ-vol\"><h3>");
        body.push_str(&labelled(
            app,
            prefs,
            &volume_title,
            volume.and_then(|v| v.title_simp.as_deref()),
        ));
        body.push_str("</h3>");
        for (chapter_index, list) in chapters {
            let chapter_title = volume
                .and_then(|volume| volume.chapters.get(chapter_index))
                .map(|chapter| chapter.title.clone())
                .unwrap_or_default();
            body.push_str(&format!(
                "<section class=\"occ-chap\"><h4>{}</h4><ol class=\"occ-list\">",
                escape(&simplified(app, prefs, &chapter_title))
            ));
            for occurrence in list {
                let Some(position) = app.position_of(&occurrence.paragraph_id) else {
                    continue;
                };
                let text = data
                    .paragraph(position)
                    .map(|paragraph| paragraph.text.as_str())
                    .unwrap_or_default();
                body.push_str(&format!(
                    "<li><a class=\"occ\" href=\"{}&amp;hl={},{}\">{}</a><span class=\"where\">第 {} 段</span></li>",
                    read_href(&occurrence.paragraph_id),
                    occurrence.offset,
                    occurrence.len,
                    excerpt_html(app, prefs, text, occurrence.offset, occurrence.len),
                    position.paragraph + 1
                ));
            }
            body.push_str("</ol></section>");
        }
        body.push_str("</section>");
    }
    body.push_str("</section>");

    let title = simplified(app, prefs, &entity.name);
    document(app, Some(prefs), &title, "", "", "", &body)
}

/// Occurrences grouped volume → chapter, relying on the reading-order sort
/// that Entities::build already guarantees.
fn grouped_occurrences<'a>(
    app: &App,
    entity: &'a Entity,
) -> Vec<(usize, Vec<(usize, Vec<&'a zgc_core::Occurrence>)>)> {
    let mut groups: Vec<(usize, Vec<(usize, Vec<&zgc_core::Occurrence>)>)> = Vec::new();
    for occurrence in &entity.occurrences {
        let Some(position) = app.position_of(&occurrence.paragraph_id) else {
            continue;
        };
        match groups.last_mut() {
            Some((volume, chapters)) if *volume == position.volume => match chapters.last_mut() {
                Some((chapter, list)) if *chapter == position.chapter => list.push(occurrence),
                _ => chapters.push((position.chapter, vec![occurrence])),
            },
            _ => groups.push((position.volume, vec![(position.chapter, vec![occurrence])])),
        }
    }
    groups
}

/// Character-exact excerpt around one occurrence, with the match marked.
fn excerpt_html(app: &App, prefs: &Prefs, text: &str, offset: usize, len: usize) -> String {
    const RADIUS: usize = 14;
    // to_simplified keeps the character count, so the occurrence offsets the
    // index recorded stay valid against the converted text.
    let text = simplified(app, prefs, text);
    let chars: Vec<char> = text.chars().collect();
    let end = offset.saturating_add(len);
    if offset >= chars.len() || end > chars.len() || len == 0 {
        return String::new();
    }
    let start = offset.saturating_sub(RADIUS);
    let stop = end.saturating_add(RADIUS).min(chars.len());
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.push_str(&escape(&chars[start..offset].iter().collect::<String>()));
    out.push_str("<mark>");
    out.push_str(&escape(&chars[offset..end].iter().collect::<String>()));
    out.push_str("</mark>");
    out.push_str(&escape(&chars[end..stop].iter().collect::<String>()));
    if stop < chars.len() {
        out.push('…');
    }
    out
}

pub fn entities_page(
    app: &App,
    prefs: &Prefs,
    kind: Option<EntityKind>,
    sort: EntitySort,
    notice: Option<&str>,
) -> String {
    let entities = app.entities();
    let mut body = String::new();
    body.push_str("<h1 class=\"page-title\">實體索引</h1>");
    body.push_str(&format!(
        "<p class=\"stats\">{} 個實體 · {} 次出現 · 見於 {} 段{}</p>",
        entities.len(),
        entities.occurrence_total(),
        entities.covered_paragraph_count(),
        if entities.overlapping_paragraph_count() > 0 {
            format!(
                "（{} 段有重疊，已按最長匹配取捨）",
                entities.overlapping_paragraph_count()
            )
        } else {
            String::new()
        }
    ));
    if let Some(notice) = notice {
        body.push_str(&format!("<p class=\"notice\">{}</p>", escape(notice)));
    }
    if entities.is_empty() {
        body.push_str("<p class=\"empty\">本書未附實體資料。</p>");
        return document(app, Some(prefs), "實體索引", "", "", "", &body);
    }

    let kind_counts = |kind: EntityKind| {
        let list: Vec<&Entity> = entities.of_kind(kind).collect();
        let occurrences: usize = list.iter().map(|entity| entity.occurrence_count()).sum();
        (list.len(), occurrences)
    };
    let (person_count, person_occurrences) = kind_counts(EntityKind::Person);
    let (place_count, place_occurrences) = kind_counts(EntityKind::Place);

    let kind_chip = |label: &str, target: Option<EntityKind>, active: bool, count: usize| {
        let href = match target {
            Some(kind) => format!(
                "/entities?kind={}&amp;sort={}",
                kind.as_str(),
                sort_key(sort)
            ),
            None => format!("/entities?sort={}", sort_key(sort)),
        };
        if active {
            format!(
                "<span class=\"chip on\" aria-current=\"true\">{} {}</span>",
                escape(label),
                count
            )
        } else {
            format!(
                "<a class=\"chip\" href=\"{}\">{} {}</a>",
                href,
                escape(label),
                count
            )
        }
    };
    body.push_str(&format!(
        "<div class=\"toolbar\"><span class=\"seg\"><span class=\"seg-label\">類型</span>{}{}{}</span>",
        kind_chip("全部", None, kind.is_none(), entities.len()),
        kind_chip("人物", Some(EntityKind::Person), kind == Some(EntityKind::Person), person_count),
        kind_chip("地名", Some(EntityKind::Place), kind == Some(EntityKind::Place), place_count)
    ));
    let sort_chip = |label: &str, target: EntitySort, active: bool| {
        let href = match kind {
            Some(kind) => format!(
                "/entities?kind={}&amp;sort={}",
                kind.as_str(),
                sort_key(target)
            ),
            None => format!("/entities?sort={}", sort_key(target)),
        };
        if active {
            format!(
                "<span class=\"chip on\" aria-current=\"true\">{}</span>",
                escape(label)
            )
        } else {
            format!("<a class=\"chip\" href=\"{}\">{}</a>", href, escape(label))
        }
    };
    body.push_str(&format!(
        "<span class=\"seg\"><span class=\"seg-label\">排序</span>{}{}</span></div>",
        sort_chip("按出現次數", EntitySort::Count, sort == EntitySort::Count),
        sort_chip("按名稱", EntitySort::Name, sort == EntitySort::Name)
    ));
    body.push_str(&format!(
        "<p class=\"hint\">人物 {person_count} 個 / {person_occurrences} 次 · 地名 {place_count} 個 / {place_occurrences} 次</p>"
    ));

    let mut rows: Vec<&Entity> = entities
        .entities()
        .filter(|entity| kind.map_or(true, |wanted| entity.kind == wanted))
        .collect();
    match sort {
        EntitySort::Count => rows.sort_by(|left, right| {
            right
                .occurrence_count()
                .cmp(&left.occurrence_count())
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.id.cmp(&right.id))
        }),
        EntitySort::Name => rows.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.id.cmp(&right.id))
        }),
    }

    body.push_str("<ol class=\"ent-list\">");
    for entity in rows {
        body.push_str(&format!(
            "<li><a class=\"ent-name\" href=\"/entity?id={}\">{}</a><span class=\"tag tag-kind\">{}</span><span class=\"count\">{} 次</span></li>",
            urlcodec::percent_encode(&entity.id),
            labelled(app, prefs, &entity.name, entity.name_simp_text()),
            escape(entity.kind.label()),
            entity.occurrence_count()
        ));
    }
    body.push_str("</ol>");

    document(app, Some(prefs), "實體索引", "", "", "", &body)
}

fn sort_key(sort: EntitySort) -> &'static str {
    match sort {
        EntitySort::Count => "count",
        EntitySort::Name => "name",
    }
}

// ---------------------------------------------------------------- search page

pub fn search_page(
    app: &App,
    prefs: &Prefs,
    text: &str,
    selected_group: &str,
    hits: &[Hit],
    total: usize,
    notice: Option<&str>,
) -> String {
    let mut body = String::new();
    body.push_str("<h1 class=\"page-title\">檢索</h1>");

    if text.is_empty() {
        body.push_str("<p class=\"hint\">在上方輸入檢索詞：原文（繁體）、白話（簡體）、注疏與字詞訓釋全部在建索引的範圍內。</p>");
        body.push_str("<p class=\"hint\">提示：底本用字須與原文一致——「衞鞅」而非「衛鞅」；檢索逐字比對，不做繁簡轉換。</p>");
        if total > 0 {
            body.push_str(&format!("<p class=\"hint\">共 {} 條結果。</p>", total));
        }
        return document(app, Some(prefs), "檢索", "", text, selected_group, &body);
    }

    let scope_label = if selected_group.is_empty() {
        "全書".to_string()
    } else {
        app.data()
            .group(selected_group)
            .map(|group| group.title.clone())
            .unwrap_or_else(|| "全書".to_string())
    };
    let truncated = if total > hits.len() {
        format!("（僅顯示前 {} 條）", hits.len())
    } else {
        String::new()
    };
    body.push_str(&format!(
        "<p class=\"search-meta\">「<span class=\"q\">{}</span>」命中 {} 條{} · 範圍：{}</p>",
        escape(text),
        total,
        truncated,
        escape(&simplified(app, prefs, &scope_label))
    ));
    if let Some(notice) = notice {
        body.push_str(&format!("<p class=\"notice\">{}</p>", escape(notice)));
    }

    if hits.is_empty() {
        body.push_str(&format!(
            "<p class=\"empty\">沒有找到「{}」。</p>",
            escape(text)
        ));
        body.push_str(
            "<p class=\"hint\">可換用原文（繁體）用字再試，例如「衞鞅」「秦國」「蘇秦」。</p>",
        );
        return document(app, Some(prefs), "檢索", "", text, selected_group, &body);
    }

    body.push_str("<ol class=\"hits\">");
    for hit in hits {
        let position = app.position_of(&hit.paragraph_id);
        let volume_title = app
            .data()
            .volume_by_id(&hit.volume_id)
            .map(|volume| volume.title.clone())
            .unwrap_or_else(|| hit.volume_id.clone());
        let chapter_title = position
            .and_then(|position| app.data().chapter(position.volume, position.chapter))
            .map(|chapter| chapter.title.clone())
            .unwrap_or_else(|| hit.chapter_id.clone());
        let paragraph_number = position.map(|position| position.paragraph + 1).unwrap_or(0);
        let (field_label, field_class) = field_label(hit.field);
        let mut hit_entities = String::new();
        let mut seen: Vec<&str> = Vec::new();
        for span in app.entities().spans_in(&hit.paragraph_id) {
            if seen.contains(&span.entity.id.as_str()) {
                continue;
            }
            seen.push(span.entity.id.as_str());
            hit_entities.push_str(&format!(
                "<a class=\"ent ent-{}\" href=\"/entity?id={}\">{}</a>",
                span.entity.kind.as_str(),
                urlcodec::percent_encode(&span.entity.id),
                escape(&simplified(app, prefs, &span.entity.name))
            ));
        }
        let hit_entities = if hit_entities.is_empty() {
            String::new()
        } else {
            format!("<div class=\"hit-ents\">{}</div>", hit_entities)
        };
        body.push_str(&format!(
            "<li class=\"hit\"><a class=\"hit-head\" href=\"{}\"><span class=\"vol\">{}</span><span class=\"sep\">·</span><span class=\"chap\">{}</span><span class=\"sep\">·</span><span class=\"para\">第 {} 段</span><span class=\"tag {}\">{}</span></a><p class=\"snippet\">{}</p>{}</li>",
            read_href(&hit.paragraph_id),
            escape(&simplified(app, prefs, &volume_title)),
            escape(&simplified(app, prefs, &chapter_title)),
            paragraph_number,
            field_class,
            field_label,
            escape_highlight(
                &simplified(app, prefs, &hit.snippet),
                &simplified(app, prefs, text)
            ),
            hit_entities
        ));
    }
    body.push_str("</ol>");

    document(
        app,
        Some(prefs),
        &format!("檢索「{}」", text),
        "",
        text,
        selected_group,
        &body,
    )
}

fn field_label(field: FieldSource) -> (&'static str, &'static str) {
    match field {
        FieldSource::Text => ("原文", "tag-text"),
        FieldSource::Translation => ("白話", "tag-tran"),
        FieldSource::Annotation => ("注疏", "tag-note"),
        FieldSource::Glossary => ("詞條", "tag-gloss"),
    }
}

// ------------------------------------------------------------------- timeline

/// The 戰國 event timeline: descending years, one card per event, zero JS.
///
/// \`year\` narrows to a single year, \`text\` filters by substring, \`focus\`
/// marks one event (from \`?e=\`), and \`notice\` reports a rejected parameter
/// instead of failing the page.
pub fn timeline_page(
    app: &App,
    prefs: &Prefs,
    year: Option<i32>,
    text: &str,
    focus: Option<&str>,
    notice: Option<&str>,
) -> String {
    let events = app.events();
    let mut body = String::new();

    if events.is_empty() {
        body.push_str(
            "<h1 class=\"page-title\">戰國年表</h1><p class=\"empty\">此書沒有事件表（events/tongjian.json），年表暫不可用；閱讀與檢索不受影響。</p>",
        );
        return document(app, Some(prefs), "戰國年表", "", "", "", &body);
    }

    let (earliest, latest) = events.year_bounds().unwrap_or((0, 0));
    body.push_str(&format!(
        "<h1 class=\"page-title\">戰國年表</h1><p class=\"tl-lede\">{} · {} 年 · {} 條事件 · 前{}–前{}（年份降序）</p>",
        escape(events.source()),
        events.year_count(),
        events.event_count(),
        earliest,
        latest
    ));

    let year_field = match year {
        Some(selected) => format!("<input type=\"hidden\" name=\"y\" value=\"{selected}\">"),
        None => String::new(),
    };
    body.push_str(&format!(
        "<form class=\"find tl-find\" action=\"/timeline\" method=\"get\" role=\"search\"><input type=\"search\" name=\"q\" value=\"{}\" placeholder=\"在事件正文 / 注 / 校記中過濾\" aria-label=\"事件關鍵詞\">{year_field}<button type=\"submit\">過濾</button></form>",
        escape(text)
    ));

    if let Some(notice) = notice {
        body.push_str(&format!("<p class=\"notice\">{}</p>", escape(notice)));
    }

    let matched = if text.is_empty() {
        None
    } else {
        Some(events.find(text))
    };
    if let Some(list) = &matched {
        body.push_str(&format!(
            "<p class=\"tl-count\">「{}」命中 {} 條</p>",
            escape(text),
            list.len()
        ));
    }
    let matched_ids: Option<std::collections::HashSet<&str>> = matched
        .as_ref()
        .map(|list| list.iter().map(|event| event.id.as_str()).collect());

    if let Some(selected) = year {
        body.push_str("<nav class=\"tl-nav\"><a href=\"/timeline\">全部年份</a>");
        if let Some(index) = events
            .years()
            .iter()
            .position(|record| record.year_bc == selected)
        {
            if let Some(earlier) = index.checked_sub(1).and_then(|i| events.years().get(i)) {
                body.push_str(&format!(
                    "<a data-nav=\"prev\" href=\"/timeline?y={}\">← 前{}年</a>",
                    earlier.year_bc, earlier.year_bc
                ));
            }
            if let Some(later) = events.years().get(index + 1) {
                body.push_str(&format!(
                    "<a data-nav=\"next\" href=\"/timeline?y={}\">前{}年 →</a>",
                    later.year_bc, later.year_bc
                ));
            }
        }
        body.push_str(&format!(
            "</nav><p class=\"keyhint\">{}</p>{}",
            escape(&simplified(app, prefs, "← → 鍵：前一年 / 後一年")),
            KEY_NAV_SCRIPT
        ));
    }

    let mut blocks = String::new();
    for record in events.years() {
        if year.is_some_and(|selected| selected != record.year_bc) {
            continue;
        }
        let mut cards = String::new();
        let mut shown = 0usize;
        for event in events.events_of_year(record.year_bc) {
            if matched_ids
                .as_ref()
                .is_some_and(|ids| !ids.contains(event.id.as_str()))
            {
                continue;
            }
            shown += 1;
            cards.push_str(&event_card(app, prefs, event, text, focus));
        }
        if shown == 0 {
            continue;
        }
        blocks.push_str(&year_block(app, prefs, record, shown, &cards));
    }

    if blocks.is_empty() {
        body.push_str("<p class=\"empty\">沒有符合條件的事件。</p>");
    } else {
        body.push_str(&blocks);
    }

    let title = match year {
        Some(selected) => format!("戰國年表 · 前{selected}年"),
        None => "戰國年表".to_string(),
    };
    document(app, Some(prefs), &title, "", text, "", &body)
}

fn year_block(app: &App, prefs: &Prefs, record: &EventYear, shown: usize, cards: &str) -> String {
    let label = record
        .label
        .trim()
        .trim_matches(|c: char| matches!(c, '（' | '）' | '(' | ')'));
    let year_note = record
        .note
        .as_deref()
        .map(str::trim)
        .filter(|note| !note.is_empty())
        .map(|note| {
            format!(
                "<details class=\"tl-sub tl-year-note\"><summary>年注</summary><p>{}</p></details>",
                escape(&simplified(app, prefs, note))
            )
        })
        .unwrap_or_default();
    format!(
        "<section class=\"tl-year\" id=\"y{:04}\"><h2 class=\"tl-head\"><span class=\"tl-label\">{}</span><span class=\"sep\">·</span><span class=\"tl-juan\">{}</span><span class=\"sep\">·</span><span class=\"tl-ruler\">{}{}</span><span class=\"tl-n\">{} 條</span></h2>{}{}</section>",
        record.year_bc,
        escape(&simplified(app, prefs, label)),
        escape(&simplified(app, prefs, &record.juan)),
        escape(&simplified(app, prefs, &record.ruler)),
        escape(&simplified(app, prefs, &record.ruler_year)),
        shown,
        year_note,
        cards
    )
}

fn event_card(
    app: &App,
    prefs: &Prefs,
    event: &Event,
    needle: &str,
    focus: Option<&str>,
) -> String {
    let highlight = |value: &str| {
        let value = simplified(app, prefs, value);
        if needle.is_empty() {
            escape(&value)
        } else {
            escape_highlight(&value, needle)
        }
    };
    let focused = if focus == Some(event.id.as_str()) {
        " focus"
    } else {
        ""
    };
    let mut out = format!(
        "<article class=\"tl-ev{focused}\" id=\"{}\"><p class=\"tl-text\">{}</p>",
        escape(&event.id),
        highlight(&event.text)
    );
    if let Some(note) = event
        .note
        .as_deref()
        .map(str::trim)
        .filter(|note| !note.is_empty())
    {
        out.push_str(&format!(
            "<details class=\"tl-sub tl-note\"><summary>注</summary><p>{}</p></details>",
            highlight(note)
        ));
    }
    if let Some(collation) = event
        .collation
        .as_deref()
        .map(str::trim)
        .filter(|collation| !collation.is_empty())
    {
        out.push_str(&format!(
            "<details class=\"tl-sub tl-coll\"><summary>校記</summary><p>{}</p></details>",
            highlight(collation)
        ));
    }
    out.push_str(&format!(
        "<p class=\"tl-meta\"><a href=\"#{}\">{}</a><span class=\"sep\">·</span>源 {} · 段 {}</p></article>",
        escape(&event.id),
        escape(&event.id),
        escape(&event.source_file),
        escape(&event.anchor)
    ));
    out
}

// ------------------------------------------------------------------- error pages

pub fn not_found_page(app: &App, heading: &str, detail: &str) -> String {
    let detail: String = detail.chars().take(MAX_DETAIL_CHARS).collect();
    let body = format!(
        "<h1 class=\"page-title\">{}</h1><p class=\"empty\">找不到：<code>{}</code></p><p><a class=\"back\" href=\"/\">回到目錄</a></p>",
        escape(heading),
        escape(&detail)
    );
    document(app, None, heading, "", "", "", &body)
}

pub fn bad_request_page(app: &App, message: &str) -> String {
    let body = format!(
        "<h1 class=\"page-title\">請求無法處理</h1><p class=\"empty\">{}</p><p><a class=\"back\" href=\"/\">回到目錄</a></p>",
        escape(message)
    );
    document(app, None, "請求無法處理", "", "", "", &body)
}

// --------------------------------------------------------------- AI companion

/// What GET /ask shows: the form, the answer, or the upstream failure.
pub struct AskView<'a> {
    pub context: &'a AskContext,
    pub config: &'a AiConfig,
    /// Echoed back into the textarea; empty for the first visit.
    pub question: &'a str,
    /// Present once a request was actually assembled and sent.
    pub prompt: Option<&'a Prompt>,
    pub outcome: Option<AskOutcome<'a>>,
}

pub enum AskOutcome<'a> {
    Answer(&'a AiReply),
    Failure(&'a AiError),
}

/// One page for every state of a question: no JS, no partial updates.
pub fn ask_page(app: &App, prefs: &Prefs, view: &AskView<'_>) -> String {
    let context = view.context;
    let mut body = String::new();

    body.push_str(&format!(
        "<nav class=\"crumbs\"><a href=\"/\">目錄</a><span class=\"sep\">/</span><a href=\"{}\">回到原文</a><span class=\"pos\">{}</span></nav>",
        read_href(&context.paragraph_id),
        escape(&context.paragraph_id)
    ));
    body.push_str("<h1 class=\"page-title\">向 AI 追問</h1>");
    body.push_str(&format!(
        "<p class=\"privacy\">本段原文與注疏將傳送至 {}</p>",
        escape(&view.config.base)
    ));

    let mut heading = escape(&simplified(app, prefs, &context.book_title));
    for part in [
        context.group_title.as_deref(),
        context.volume_title.as_deref(),
        context.chapter_title.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        let part = part.trim();
        if !part.is_empty() {
            heading.push_str(" · ");
            heading.push_str(&escape(&simplified(app, prefs, part)));
        }
    }
    body.push_str(&format!(
        "<p class=\"ask-where\">{} · 第 {}／{} 段</p>",
        heading, context.paragraph_ordinal, context.paragraph_total
    ));

    body.push_str(&format!(
        "<p class=\"ask-original\">{}</p>",
        escape(&simplified(app, prefs, context.original.trim()))
    ));

    body.push_str(&context_summary(context, view.prompt));

    if !view.config.ready() {
        body.push_str(&config_block(view.config));
    }

    body.push_str("<form class=\"ask-form\" method=\"get\" action=\"/ask\">");
    body.push_str(&format!(
        "<input type=\"hidden\" name=\"p\" value=\"{}\">",
        escape(&context.paragraph_id)
    ));
    body.push_str("<label class=\"ask-label\" for=\"q\">就此段提問</label>");
    body.push_str(&format!(
        "<textarea id=\"q\" name=\"q\" rows=\"3\" maxlength=\"{}\" placeholder=\"例：顏率為什麼能說動秦王？\">{}</textarea>",
        MAX_QUESTION_CHARS,
        escape(view.question)
    ));
    body.push_str("<button type=\"submit\">送出</button></form>");

    match &view.outcome {
        Some(AskOutcome::Answer(reply)) => body.push_str(&answer_block(reply)),
        Some(AskOutcome::Failure(error)) => body.push_str(&failure_block(error)),
        None => {}
    }

    let title = match &context.chapter_title {
        Some(chapter) => format!("向 AI 追問 · {}", simplified(app, prefs, chapter)),
        None => "向 AI 追問".to_string(),
    };
    document(app, Some(prefs), &title, "", "", "", &body)
}

/// The context that travels with the question, with the character counts that
/// let a reader judge whether the answer had anything to stand on. Once a
/// prompt exists the counts are the ones actually sent.
fn context_summary(context: &AskContext, prompt: Option<&Prompt>) -> String {
    // Without a question there is no prompt yet, so measure what *would* be
    // sent: the same renderer, the same budget. Otherwise the counts jump
    // between the form and the answered page for no visible reason.
    let probe = build_prompt(context, "");
    let budget = prompt.map(|prompt| &prompt.budget).unwrap_or(&probe.budget);
    let (notes_chars, year_chars, entity_chars) = (budget.notes, budget.year, budget.entities);

    let mut out = String::from("<section class=\"ask-context\"><h2>本次注入的上下文</h2><ul>");
    out.push_str(&format!(
        "<li>原文　{} 字</li>",
        count_chars(context.original.trim())
    ));
    out.push_str(&format!(
        "<li>白話　{} 字</li>",
        count_chars(context.translation.as_deref().unwrap_or("").trim())
    ));
    if budget.notes_full > notes_chars {
        out.push_str(&format!(
            "<li>注疏　{} 則 · 送出 {} 字（全 {} 字）</li>",
            budget.note_count, notes_chars, budget.notes_full
        ));
    } else {
        out.push_str(&format!(
            "<li>注疏　{} 則 · {} 字</li>",
            context.notes.len(),
            notes_chars
        ));
    }
    out.push_str(&format!("<li>系年　{} 字</li>", year_chars));
    out.push_str(&format!(
        "<li>實體　{} 條 · {} 字</li>",
        context.entities.len(),
        entity_chars
    ));
    if !context.glossary.is_empty() {
        out.push_str(&format!("<li>訓釋　{} 條</li>", context.glossary.len()));
    }
    out.push_str("</ul>");
    if let Some(prompt) = prompt {
        out.push_str(&format!(
            "<p class=\"ask-total\">送出全文 {} 字（上限 {}）</p>",
            prompt.budget.total, MAX_USER_CHARS
        ));
        if !prompt.budget.truncated.is_empty() {
            let mut names: Vec<&str> = prompt.budget.truncated.clone();
            names.sort_unstable();
            names.dedup();
            out.push_str(&format!(
                "<p class=\"ask-truncated\">已截斷：{}（原文未全部送出）</p>",
                escape(&names.join("、"))
            ));
        }
    }
    out.push_str("</section>");
    out
}

fn answer_block(reply: &AiReply) -> String {
    answer_block_with(reply, false)
}

fn story_answer_block(reply: &AiReply) -> String {
    answer_block_with(reply, true)
}

fn answer_block_with(reply: &AiReply, strip_headings: bool) -> String {
    let mut meta = format!(
        "模型 {} · 耗時 {} ms",
        escape(reply.model.as_deref().unwrap_or("未回報")),
        reply.elapsed_ms
    );
    match (reply.prompt_tokens, reply.completion_tokens) {
        (Some(input), Some(output)) => {
            meta.push_str(&format!(" · tokens {input} → {output}"));
        }
        (Some(input), None) => meta.push_str(&format!(" · tokens {input} → ?")),
        (None, Some(output)) => meta.push_str(&format!(" · tokens ? → {output}")),
        (None, None) => meta.push_str(" · 上游未回報 token 用量"),
    }
    let mut out = String::from("<section class=\"answer\">");
    out.push_str(&format!("<p class=\"ai-meta\">{meta}</p>"));
    out.push_str("<div class=\"answer-body\">");
    for line in reply.content.lines() {
        let line = if strip_headings {
            plain_heading(line)
        } else {
            line.trim()
        };
        if !line.is_empty() {
            out.push_str(&format!("<p>{}</p>", escape(line)));
        }
    }
    out.push_str("</div></section>");
    out
}

fn plain_heading(line: &str) -> &str {
    let line = line.trim();
    let line = line
        .strip_prefix("**")
        .and_then(|rest| rest.strip_suffix("**"))
        .unwrap_or(line);
    line.trim_start_matches('#').trim()
}

fn failure_block(error: &AiError) -> String {
    format!(
        "<section class=\"ai-error\"><h2>上游沒有給出回答</h2><p>{}</p><p class=\"hint\">問題已送出，但沒有寫入任何檔案；可以改動配置後重試。</p></section>",
        escape(&error.human())
    )
}

fn config_block(config: &AiConfig) -> String {
    let mut out = String::from("<section class=\"ai-config\"><h2>尚未配置 AI</h2>");
    out.push_str("<p>本頁不猜任何模型名，只讀環境變量。啟動 zgc-app 之前設置：</p><ul>");
    if config.key.is_none() {
        out.push_str("<li><code>ZGC_AI_KEY</code> — 上游 API 金鑰（目前未設置）</li>");
    }
    if config.model.is_none() {
        out.push_str("<li><code>ZGC_AI_MODEL</code> — 模型名（目前未設置）</li>");
    }
    out.push_str(&format!(
        "</ul><p>端點：<code>ZGC_AI_BASE</code>（目前 {}）</p>",
        escape(&config.base)
    ));
    out.push_str(&format!(
        "<p>可選：<code>ZGC_AI_TIMEOUT_SECS</code>（目前 {} 秒）、<code>ZGC_AI_MAX_TOKENS</code>（目前 {}）。金鑰只以請求頭傳送，不寫入日誌。</p></section>",
        config.timeout.as_secs(),
        config.max_tokens
    ));
    out
}

fn count_chars(text: &str) -> usize {
    text.chars().count()
}
