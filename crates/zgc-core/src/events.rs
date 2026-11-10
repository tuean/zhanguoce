//! Event table (M3c-1): the 戰國 chapters of 資治通鑑 as one dated timeline.
//!
//! The on-disk contract is data/zhanguoce/events/tongjian.json, produced by
//! tools/build_events.py from the seven 周紀/秦紀 source files. One `years`
//! record per `div.yt` and one `events` record per body paragraph, so a year
//! can carry many events and both sides can be cross-checked:
//!
//! 1. `yearBc` equals the number in the div id (`y0403` → 403);
//! 2. `yearBc` equals the year inside the `（戊寅、前四○三年）` label;
//! 3. `ganzhi` equals the stem-branch in that same label;
//! 4. `ruler`/`rulerYear` come from the source `data-head-*` attributes and are
//!    never empty, and every event repeats them verbatim from its year record;
//! 5. `id` is unique and equals `tj-<div>-<anchor>`, with a non-empty anchor.
//!
//! Loading is fail-fast on all five: a timeline that quietly disagrees with the
//! book is worse than no timeline at all. The one deliberate exception is a
//! **missing file** — other classics will never have an event table, so
//! `Events::load` returns `Events::empty()` instead of an error and every
//! accessor answers from an empty index.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Event table, relative to the book directory.
pub const EVENT_FILE: &str = "events/tongjian.json";

/// Only schemaVersion 1 exists; anything else is a contract we do not know.
const SUPPORTED_SCHEMA_VERSION: u32 = 1;

fn default_schema_version() -> u32 {
    SUPPORTED_SCHEMA_VERSION
}

/// One `div.yt` year record: the heading the events hang off.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventYear {
    /// Absolute year, positive = BCE.
    pub year_bc: i32,
    /// Source div id, e.g. `y0403`.
    pub div: String,
    /// Verbatim `（戊寅、前四○三年）` label.
    pub label: String,
    /// Stem-branch, e.g. `戊寅`.
    pub ganzhi: String,
    /// `data-head-description`, e.g. `威烈王`.
    pub ruler: String,
    /// `data-head-title`, e.g. `二十三年`.
    pub ruler_year: String,
    /// 卷名, e.g. `周紀一`.
    pub juan: String,
    /// Source file stem, e.g. `5282`.
    pub source_file: String,
    /// Anchor of the year heading paragraph inside the div.
    pub anchor: String,
    /// Year-level interlinear note, when the heading had one.
    #[serde(default)]
    pub note: Option<String>,
}

/// One body paragraph: the events themselves.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    /// Stable anchor id: `tj-<div>-<anchor>`.
    pub id: String,
    pub year_bc: i32,
    pub ruler: String,
    pub ruler_year: String,
    pub ganzhi: String,
    pub juan: String,
    pub source_file: String,
    /// Paragraph id inside the source section, e.g. `p4`.
    pub anchor: String,
    /// Traditional-script body text, markup stripped.
    pub text: String,
    /// `span.jiazhu` note, when the paragraph had one.
    #[serde(default)]
    pub note: Option<String>,
    /// `span.jiaozhusup` collation note, when the paragraph had one.
    #[serde(default)]
    pub collation: Option<String>,
}

/// The JSON file as written by tools/build_events.py.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventFile {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub book_id: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub generated_by: Option<String>,
    #[serde(default)]
    pub years: Vec<EventYear>,
    #[serde(default)]
    pub events: Vec<Event>,
}

/// Why an event table was rejected.
#[derive(Debug)]
pub enum EventError {
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
    EmptyField {
        id: String,
        field: &'static str,
    },
    DivYearMismatch {
        div: String,
        parsed: Option<i32>,
        year_bc: i32,
    },
    LabelMismatch {
        year_bc: i32,
        label: String,
        detail: String,
    },
    DuplicateYear {
        year_bc: i32,
    },
    DuplicateEventId {
        id: String,
    },
    IdShapeMismatch {
        id: String,
        expected: String,
    },
    UnknownYear {
        id: String,
        year_bc: i32,
    },
    YearFieldMismatch {
        id: String,
        field: &'static str,
    },
}

/// Alias kept short at the call site, mirroring `EntityResult`.
pub type EventResult<T> = Result<T, EventError>;

impl fmt::Display for EventError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventError::Read { path, source } => {
                write!(f, "讀取事件表失敗（{}）：{source}", path.display())
            }
            EventError::Json { path, source } => {
                write!(f, "事件表 JSON 解析失敗（{}）：{source}", path.display())
            }
            EventError::UnsupportedSchemaVersion { found, supported } => {
                write!(f, "事件表 schemaVersion={found}，本內核只認 {supported}")
            }
            EventError::EmptyField { id, field } => {
                write!(f, "事件「{id}」缺少必填欄位 {field}")
            }
            EventError::DivYearMismatch {
                div,
                parsed,
                year_bc,
            } => match parsed {
                Some(parsed) => write!(
                    f,
                    "年份自校驗失敗：div id「{div}」= 前{parsed}年，但 yearBc={year_bc}"
                ),
                None => write!(f, "年份自校驗失敗：div id「{div}」無法解析成數字"),
            },
            EventError::LabelMismatch {
                year_bc,
                label,
                detail,
            } => write!(
                f,
                "年份自校驗失敗：yearBc={year_bc} 與標籤「{label}」不符（{detail}）"
            ),
            EventError::DuplicateYear { year_bc } => write!(f, "年份重複：前{year_bc}年出現兩次"),
            EventError::DuplicateEventId { id } => write!(f, "事件 id 重複：{id}"),
            EventError::IdShapeMismatch { id, expected } => {
                write!(f, "事件 id 形狀不符：{id}（應為 {expected}）")
            }
            EventError::UnknownYear { id, year_bc } => {
                write!(f, "事件「{id}」的 yearBc={year_bc} 沒有對應的年份記錄")
            }
            EventError::YearFieldMismatch { id, field } => {
                write!(f, "事件「{id}」的 {field} 與所屬年份記錄不一致")
            }
        }
    }
}

impl std::error::Error for EventError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            EventError::Read { source, .. } => Some(source),
            EventError::Json { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Chinese year digits as they appear in the source labels. `○`/`〇`/`零`
/// are all zero; the source prints 403 as `四○三`, never `四〇三` alone.
fn cn_digit(c: char) -> Option<i32> {
    Some(match c {
        '○' | '〇' | '零' => 0,
        '一' => 1,
        '二' => 2,
        '三' => 3,
        '四' => 4,
        '五' => 5,
        '六' => 6,
        '七' => 7,
        '八' => 8,
        '九' => 9,
        _ => return None,
    })
}

/// `y0403` → `403`. The div id is the absolute BCE year, zero-padded to four.
pub fn parse_div_year(div: &str) -> Option<i32> {
    let digits = div.strip_prefix('y')?;
    if digits.is_empty() || digits.len() > 4 {
        return None;
    }
    let mut value = 0i32;
    for c in digits.chars() {
        value = value.checked_mul(10)?.checked_add(c.to_digit(10)? as i32)?;
    }
    Some(value)
}

/// `（戊寅、前四○三年）` → `("戊寅", 403)`.
pub fn parse_year_label(label: &str) -> Option<(String, i32)> {
    let trimmed = label
        .trim()
        .trim_matches(|c: char| matches!(c, '（' | '）' | '(' | ')'))
        .trim();
    let mut parts = trimmed.split('、');
    let ganzhi = parts.next()?.trim();
    let year_part = parts.next()?.trim();
    if parts.next().is_some() {
        return None;
    }
    let digits = year_part.strip_prefix('前')?.strip_suffix('年')?;
    let mut value = 0i32;
    let mut seen = 0usize;
    for c in digits.chars() {
        value = value.checked_mul(10)?.checked_add(cn_digit(c)?)?;
        seen += 1;
    }
    if seen == 0 {
        return None;
    }
    Some((ganzhi.to_string(), value))
}

/// `p12` → `12`, so events inside a year keep their source order.
fn anchor_number(anchor: &str) -> u32 {
    anchor
        .strip_prefix('p')
        .and_then(|digits| digits.parse().ok())
        .unwrap_or(0)
}

/// The validated, indexed timeline.
///
/// Years and events are both held **descending** (前403 → 前209) and each
/// year's events are contiguous, so a page can slice one year out of
/// `events()` without regrouping.
#[derive(Debug, Clone, Default)]
pub struct Events {
    source: String,
    years: Vec<EventYear>,
    events: Vec<Event>,
    by_year: HashMap<i32, (usize, usize)>,
    by_id: HashMap<String, usize>,
}

impl Events {
    /// An empty timeline — what a book without an event table gets.
    pub fn empty() -> Events {
        Events::default()
    }

    /// True when there is no timeline: no file, no years, nothing to render.
    pub fn is_empty(&self) -> bool {
        self.years.is_empty()
    }

    /// Load `events/tongjian.json` under book_dir.
    ///
    /// A missing file is **not** an error: the reader degrades to "no timeline
    /// page data" instead of refusing to start. A present-but-wrong file is.
    pub fn load(book_dir: &Path) -> EventResult<Events> {
        let path = book_dir.join(EVENT_FILE);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Events::empty())
            }
            Err(source) => return Err(EventError::Read { path, source }),
        };
        let file: EventFile =
            serde_json::from_slice(&bytes).map_err(|source| EventError::Json { path, source })?;
        Events::build(file)
    }

    /// The single validator: `load` and programmatic construction share it, so
    /// tests exercise exactly the code the server runs.
    pub fn build(file: EventFile) -> EventResult<Events> {
        if file.schema_version != SUPPORTED_SCHEMA_VERSION {
            return Err(EventError::UnsupportedSchemaVersion {
                found: file.schema_version,
                supported: SUPPORTED_SCHEMA_VERSION,
            });
        }

        let mut years = file.years;
        let mut by_year_value: HashMap<i32, usize> = HashMap::with_capacity(years.len());
        for (index, year) in years.iter().enumerate() {
            // (1) div id and yearBc agree.
            match parse_div_year(&year.div) {
                Some(parsed) if parsed == year.year_bc => {}
                parsed => {
                    return Err(EventError::DivYearMismatch {
                        div: year.div.clone(),
                        parsed,
                        year_bc: year.year_bc,
                    })
                }
            }
            // (2)+(3) the label agrees on both the year and the stem-branch.
            let Some((label_ganzhi, label_year)) = parse_year_label(&year.label) else {
                return Err(EventError::LabelMismatch {
                    year_bc: year.year_bc,
                    label: year.label.clone(),
                    detail: "無「干支、前X年」".to_string(),
                });
            };
            if label_year != year.year_bc {
                return Err(EventError::LabelMismatch {
                    year_bc: year.year_bc,
                    label: year.label.clone(),
                    detail: format!("標籤年是前{label_year}年"),
                });
            }
            if label_ganzhi != year.ganzhi {
                return Err(EventError::LabelMismatch {
                    year_bc: year.year_bc,
                    label: year.label.clone(),
                    detail: format!("標籤干支是{label_ganzhi}"),
                });
            }
            // (4) the ruler fields are present; they come from data-head-*.
            for (field, value) in [("ruler", &year.ruler), ("rulerYear", &year.ruler_year)] {
                if value.trim().is_empty() {
                    return Err(EventError::EmptyField {
                        id: year.div.clone(),
                        field,
                    });
                }
            }
            if by_year_value.insert(year.year_bc, index).is_some() {
                return Err(EventError::DuplicateYear {
                    year_bc: year.year_bc,
                });
            }
        }

        let mut events = file.events;
        let mut event_ids: HashSet<&str> = HashSet::with_capacity(events.len());
        for event in &events {
            // (5) id uniqueness and shape: tj-<div>-<anchor>.
            if event.id.trim().is_empty() {
                return Err(EventError::EmptyField {
                    id: event.anchor.clone(),
                    field: "id",
                });
            }
            if event.anchor.trim().is_empty() {
                return Err(EventError::EmptyField {
                    id: event.id.clone(),
                    field: "anchor",
                });
            }
            if event.text.trim().is_empty() {
                return Err(EventError::EmptyField {
                    id: event.id.clone(),
                    field: "text",
                });
            }
            let year = by_year_value
                .get(&event.year_bc)
                .map(|index| &years[*index])
                .ok_or_else(|| EventError::UnknownYear {
                    id: event.id.clone(),
                    year_bc: event.year_bc,
                })?;
            let expected = format!("tj-{}-{}", year.div, event.anchor);
            if event.id != expected {
                return Err(EventError::IdShapeMismatch {
                    id: event.id.clone(),
                    expected,
                });
            }
            if !event_ids.insert(event.id.as_str()) {
                return Err(EventError::DuplicateEventId {
                    id: event.id.clone(),
                });
            }
            // The redundant per-event year fields must match the year record.
            for (field, same) in [
                ("ruler", event.ruler == year.ruler),
                ("rulerYear", event.ruler_year == year.ruler_year),
                ("ganzhi", event.ganzhi == year.ganzhi),
                ("juan", event.juan == year.juan),
                ("sourceFile", event.source_file == year.source_file),
            ] {
                if !same {
                    return Err(EventError::YearFieldMismatch {
                        id: event.id.clone(),
                        field,
                    });
                }
            }
        }

        years.sort_by(|left, right| right.year_bc.cmp(&left.year_bc));
        events.sort_by(|left, right| {
            right
                .year_bc
                .cmp(&left.year_bc)
                .then_with(|| anchor_number(&left.anchor).cmp(&anchor_number(&right.anchor)))
        });

        let mut by_year: HashMap<i32, (usize, usize)> = HashMap::with_capacity(years.len());
        let mut by_id: HashMap<String, usize> = HashMap::with_capacity(events.len());
        for (index, event) in events.iter().enumerate() {
            by_id.insert(event.id.clone(), index);
            by_year
                .entry(event.year_bc)
                .and_modify(|range| range.1 = index + 1)
                .or_insert((index, index + 1));
        }

        Ok(Events {
            source: file.source,
            years,
            events,
            by_year,
            by_id,
        })
    }

    /// The `source` string from the file, empty for an empty timeline.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Year records, descending (前403 → 前209).
    pub fn years(&self) -> &[EventYear] {
        &self.years
    }

    /// Every event, descending by year and in source order inside a year.
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// The events of one year, in source order; empty when the year has none.
    pub fn events_of_year(&self, year_bc: i32) -> &[Event] {
        match self.by_year.get(&year_bc) {
            Some((start, end)) => &self.events[*start..*end],
            None => &[],
        }
    }

    /// Look one event up by its stable id.
    pub fn event(&self, id: &str) -> Option<&Event> {
        self.by_id.get(id).map(|index| &self.events[*index])
    }

    /// The year record that owns `year_bc`.
    pub fn year(&self, year_bc: i32) -> Option<&EventYear> {
        self.years.iter().find(|year| year.year_bc == year_bc)
    }

    pub fn year_count(&self) -> usize {
        self.years.len()
    }

    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// `(earliest, latest)` in chronological order — `(403, 209)` here, because
    /// 前403 is the older end and a *larger* BCE number means an earlier date.
    pub fn year_bounds(&self) -> Option<(i32, i32)> {
        let earliest = self.years.iter().map(|year| year.year_bc).max()?;
        let latest = self.years.iter().map(|year| year.year_bc).min()?;
        Some((earliest, latest))
    }

    /// Years inside the span that carry no events, ascending.
    pub fn years_without_events(&self) -> Vec<i32> {
        let Some((earliest, latest)) = self.year_bounds() else {
            return Vec::new();
        };
        (latest..=earliest)
            .rev()
            .filter(|year_bc| !self.by_year.contains_key(year_bc))
            .collect()
    }

    /// Substring filter over text, note and collation, in timeline order.
    ///
    /// Deliberately plain: the corpus is Chinese, so case folding buys nothing
    /// and the bigram index is paragraph-scoped, not event-scoped.
    pub fn find(&self, needle: &str) -> Vec<&Event> {
        let needle = needle.trim();
        if needle.is_empty() {
            return Vec::new();
        }
        self.events
            .iter()
            .filter(|event| {
                event.text.contains(needle)
                    || event
                        .note
                        .as_deref()
                        .is_some_and(|note| note.contains(needle))
                    || event
                        .collation
                        .as_deref()
                        .is_some_and(|collation| collation.contains(needle))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CN: [char; 10] = ['○', '一', '二', '三', '四', '五', '六', '七', '八', '九'];

    fn cn_year(value: i32) -> String {
        value
            .to_string()
            .chars()
            .map(|c| CN[c.to_digit(10).expect("ascii digit") as usize])
            .collect()
    }

    fn year(year_bc: i32, anchor: &str) -> EventYear {
        EventYear {
            year_bc,
            div: format!("y{year_bc:04}"),
            label: format!("（戊寅、前{}年）", cn_year(year_bc)),
            ganzhi: "戊寅".into(),
            ruler: "威烈王".into(),
            ruler_year: "二十三年".into(),
            juan: "周紀一".into(),
            source_file: "5282".into(),
            anchor: anchor.into(),
            note: None,
        }
    }

    fn event(year_bc: i32, anchor: &str, text: &str) -> Event {
        Event {
            id: format!("tj-y{year_bc:04}-{anchor}"),
            year_bc,
            ruler: "威烈王".into(),
            ruler_year: "二十三年".into(),
            ganzhi: "戊寅".into(),
            juan: "周紀一".into(),
            source_file: "5282".into(),
            anchor: anchor.into(),
            text: text.into(),
            note: None,
            collation: None,
        }
    }

    fn file(years: Vec<EventYear>, events: Vec<Event>) -> EventFile {
        EventFile {
            schema_version: 1,
            book_id: "zhanguoce".into(),
            source: "test".into(),
            generated_by: None,
            years,
            events,
        }
    }

    #[test]
    fn div_ids_and_labels_parse_back_to_the_same_year() {
        assert_eq!(parse_div_year("y0403"), Some(403));
        assert_eq!(parse_div_year("y0209"), Some(209));
        assert_eq!(parse_div_year("y400"), Some(400));
        assert_eq!(parse_div_year("0403"), None);
        assert_eq!(parse_div_year("y"), None);
        assert_eq!(parse_div_year("yö403"), None);

        assert_eq!(
            parse_year_label("（戊寅、前四○三年）"),
            Some(("戊寅".into(), 403))
        );
        // 〇 and 零 are the other two spellings of zero in the same slot.
        assert_eq!(
            parse_year_label("（庚辰、前四〇〇年）"),
            Some(("庚辰".into(), 400))
        );
        // The source is purely positional (305 prints as 三○五), so the 百/十
        // notation is rejected outright rather than half-parsed.
        assert_eq!(parse_year_label("（辛巳、前三百零五年）"), None);
        assert_eq!(
            parse_year_label("（辛巳、前三○五年）"),
            Some(("辛巳".into(), 305))
        );
        assert_eq!(parse_year_label("前四○三年"), None, "needs the ganzhi too");
        assert_eq!(parse_year_label("（戊寅、前四○三）"), None, "needs the 年");
    }

    #[test]
    fn build_holds_years_and_events_in_descending_order() {
        let table = Events::build(file(
            vec![year(209, "p1"), year(403, "p3")],
            vec![
                event(209, "p2", "二世廢衞君角"),
                event(403, "p5", "初命晉大夫"),
                event(403, "p4", "初命晉大夫魏斯"),
            ],
        ))
        .expect("valid table");

        let years: Vec<i32> = table.years().iter().map(|record| record.year_bc).collect();
        assert_eq!(years, vec![403, 209], "newest year first");
        let ids: Vec<&str> = table
            .events()
            .iter()
            .map(|event| event.id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec!["tj-y0403-p4", "tj-y0403-p5", "tj-y0209-p2"],
            "year desc, then source anchor order"
        );
        assert_eq!(table.events_of_year(403).len(), 2);
        assert_eq!(table.events_of_year(209).len(), 1);
        assert!(table.events_of_year(402).is_empty());
        assert_eq!(
            table.event("tj-y0403-p5").map(|event| event.text.as_str()),
            Some("初命晉大夫")
        );
        assert!(table.event("tj-y0403-p9").is_none());
        assert_eq!(table.year_bounds(), Some((403, 209)));
        assert_eq!(table.year_count(), 2);
        assert_eq!(table.event_count(), 3);
        assert!(table.years_without_events().contains(&402));
        assert!(!table.years_without_events().contains(&403));
    }

    #[test]
    fn find_matches_text_note_and_collation_only() {
        let mut with_note = event(403, "p4", "初命晉大夫魏斯");
        with_note.note = Some("溫公書法所由始".into());
        let mut with_collation = event(403, "p5", "三家分晉");
        with_collation.collation = Some("〔章：十二行本作「分」〕".into());
        let table = Events::build(file(
            vec![year(403, "p3")],
            vec![with_note, with_collation, event(403, "p6", "智伯請地")],
        ))
        .expect("valid table");

        assert_eq!(table.find("書法").len(), 1);
        assert_eq!(table.find("十二行本").len(), 1);
        assert_eq!(table.find("智伯").len(), 1);
        assert_eq!(table.find("   ").len(), 0, "blank needle matches nothing");
        assert_eq!(table.find("不存在的詞").len(), 0);
    }

    #[test]
    fn a_missing_file_degrades_to_an_empty_timeline() {
        let table = Events::load(Path::new("/nonexistent/zgc-events-test"))
            .expect("missing file is not an error");
        assert!(table.is_empty());
        assert_eq!(table.year_count(), 0);
        assert_eq!(table.event_count(), 0);
        assert_eq!(table.year_bounds(), None);
        assert!(table.events_of_year(403).is_empty());
        assert!(table.find("anything").is_empty());
        assert!(table.years_without_events().is_empty());
    }

    #[test]
    fn a_div_id_that_disagrees_with_year_bc_is_rejected() {
        let mut broken = year(403, "p3");
        broken.div = "y0404".into();
        let error = Events::build(file(vec![broken], Vec::new())).expect_err("must fail");
        assert!(matches!(
            error,
            EventError::DivYearMismatch {
                parsed: Some(404),
                year_bc: 403,
                ..
            }
        ));
    }

    #[test]
    fn a_label_that_disagrees_with_year_bc_is_rejected() {
        let mut broken = year(403, "p3");
        broken.label = "（戊寅、前四○四年）".into();
        let error = Events::build(file(vec![broken], Vec::new())).expect_err("must fail");
        assert!(matches!(
            error,
            EventError::LabelMismatch { year_bc: 403, .. }
        ));
    }

    #[test]
    fn a_label_whose_ganzhi_disagrees_is_rejected() {
        let mut broken = year(403, "p3");
        broken.ganzhi = "己卯".into();
        let error = Events::build(file(vec![broken], Vec::new())).expect_err("must fail");
        assert!(matches!(
            error,
            EventError::LabelMismatch { year_bc: 403, .. }
        ));
    }

    #[test]
    fn a_duplicate_year_is_rejected() {
        let error = Events::build(file(vec![year(403, "p3"), year(403, "p9")], Vec::new()))
            .expect_err("must fail");
        assert!(matches!(error, EventError::DuplicateYear { year_bc: 403 }));
    }

    #[test]
    fn an_event_without_a_year_record_is_rejected() {
        let error = Events::build(file(vec![year(403, "p3")], vec![event(402, "p4", "孤兒")]))
            .expect_err("must fail");
        assert!(matches!(
            error,
            EventError::UnknownYear { year_bc: 402, .. }
        ));
    }

    #[test]
    fn an_event_id_that_does_not_mirror_its_anchor_is_rejected() {
        let mut broken = event(403, "p4", "初命晉大夫");
        broken.id = "tj-y0403-p5".into();
        let error =
            Events::build(file(vec![year(403, "p3")], vec![broken])).expect_err("must fail");
        assert!(matches!(error, EventError::IdShapeMismatch { .. }));
    }

    #[test]
    fn a_duplicate_event_id_is_rejected() {
        let duplicate = event(403, "p4", "別的文字");
        let error = Events::build(file(
            vec![year(403, "p3")],
            vec![event(403, "p4", "初命晉大夫"), duplicate.clone()],
        ))
        .expect_err("must fail");
        assert!(matches!(error, EventError::DuplicateEventId { .. }));
    }

    #[test]
    fn an_event_that_rewrites_its_year_fields_is_rejected() {
        let mut broken = event(403, "p4", "初命晉大夫");
        broken.ruler_year = "二十四年".into();
        let error =
            Events::build(file(vec![year(403, "p3")], vec![broken])).expect_err("must fail");
        assert!(matches!(
            error,
            EventError::YearFieldMismatch {
                field: "rulerYear",
                ..
            }
        ));
    }

    #[test]
    fn an_empty_text_or_unknown_schema_version_is_rejected() {
        let error = Events::build(file(vec![year(403, "p3")], vec![event(403, "p4", "   ")]))
            .expect_err("must fail");
        assert!(matches!(
            error,
            EventError::EmptyField { field: "text", .. }
        ));

        let mut other = file(vec![year(403, "p3")], Vec::new());
        other.schema_version = 2;
        let error = Events::build(other).expect_err("must fail");
        assert!(matches!(
            error,
            EventError::UnsupportedSchemaVersion {
                found: 2,
                supported: 1
            }
        ));
    }
}
