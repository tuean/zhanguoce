//! Reader preferences: layout, original-text size, translation size, leading.
//!
//! Resolution order is query parameter, then cookie, then default. A preference
//! that arrives in the query is echoed back as a Set-Cookie so the choice
//! survives navigation (every pager link carries only the paragraph id).

use crate::urlcodec::{self, Cookies, Query};

pub const COOKIE_LAYOUT: &str = "zgc_layout";
pub const COOKIE_SCRIPT: &str = "zgc_script";
pub const COOKIE_ORIG: &str = "zgc_orig";
pub const COOKIE_TRAN: &str = "zgc_tran";
pub const COOKIE_LEADING: &str = "zgc_leading";
pub const COOKIE_LAST: &str = "zgc_last";

/// Cookie lifetime for preferences and the reading position: one year.
pub const COOKIE_MAX_AGE: u32 = 31_536_000;

/// Column layout of the reading page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// 原文 + 白話 + 注疏.
    Three,
    /// 原文 + 白話.
    Two,
    /// 原文 only.
    One,
}

impl Layout {
    pub const ALL: [Layout; 3] = [Layout::Three, Layout::Two, Layout::One];

    pub fn key(self) -> &'static str {
        match self {
            Layout::Three => "tri",
            Layout::Two => "dual",
            Layout::One => "single",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Layout::Three => "三欄",
            Layout::Two => "雙欄",
            Layout::One => "單欄",
        }
    }

    pub fn css_class(self) -> &'static str {
        match self {
            Layout::Three => "layout-tri",
            Layout::Two => "layout-dual",
            Layout::One => "layout-single",
        }
    }

    pub fn parse(value: &str) -> Option<Layout> {
        Layout::ALL
            .into_iter()
            .find(|layout| layout.key() == value.trim())
    }
}

/// Display script for the traditional-side text (原文, 注疏, 系年, 謀略).
///
/// `Both` prints each 原文 line twice — the 繁體 source, then its 簡體
/// conversion — so the reader can compare glyph by glyph. The 白話 column is
/// 簡體 in the source data and is never converted, so this knob never touches
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script {
    /// 繁 / 簡 side by side (two lines per 原文 line).
    Both,
    Trad,
    Simp,
}

impl Script {
    pub const ALL: [Script; 3] = [Script::Both, Script::Trad, Script::Simp];

    pub fn key(self) -> &'static str {
        match self {
            Script::Both => "both",
            Script::Trad => "trad",
            Script::Simp => "simp",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Script::Both => "繁簡對照",
            Script::Trad => "繁體",
            Script::Simp => "簡體",
        }
    }

    /// Short label for the three-state toggle in the toolbar.
    pub fn toggle_label(self) -> &'static str {
        match self {
            Script::Both => "對照",
            Script::Trad => "繁",
            Script::Simp => "簡",
        }
    }

    /// Query and cookie spelling: the ASCII keys, plus the glyphs a reader
    /// might reasonably type. 簡體/繁體 keep both the 简 and 簡 spellings.
    pub fn parse(value: &str) -> Option<Script> {
        match value.trim() {
            "both" => Some(Script::Both),
            "trad" | "繁" | "繁體" => Some(Script::Trad),
            "simp" | "简" | "簡" | "簡體" => Some(Script::Simp),
            _ => None,
        }
    }
}

/// Text size steps; four steps, three of which are already "readable".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontStep {
    Small,
    Medium,
    Large,
    Huge,
}

impl FontStep {
    pub const ALL: [FontStep; 4] = [
        FontStep::Small,
        FontStep::Medium,
        FontStep::Large,
        FontStep::Huge,
    ];

    pub fn key(self) -> &'static str {
        match self {
            FontStep::Small => "s",
            FontStep::Medium => "m",
            FontStep::Large => "l",
            FontStep::Huge => "xl",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FontStep::Small => "小",
            FontStep::Medium => "中",
            FontStep::Large => "大",
            FontStep::Huge => "特大",
        }
    }

    /// Size for the 繁體 original (the main reading surface).
    pub fn orig_rem(self) -> &'static str {
        match self {
            FontStep::Small => "1.10",
            FontStep::Medium => "1.28",
            FontStep::Large => "1.46",
            FontStep::Huge => "1.66",
        }
    }

    /// Size for the 簡體 translation — adjusted independently of the original.
    pub fn tran_rem(self) -> &'static str {
        match self {
            FontStep::Small => "0.92",
            FontStep::Medium => "1.00",
            FontStep::Large => "1.10",
            FontStep::Huge => "1.22",
        }
    }

    pub fn parse(value: &str) -> Option<FontStep> {
        FontStep::ALL
            .into_iter()
            .find(|step| step.key() == value.trim())
    }
}

/// Line height steps. Every step stays at or above the 1.9 floor the task
/// requires for comfortable Classical Chinese reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leading {
    Tight,
    Normal,
    Loose,
}

impl Leading {
    pub const ALL: [Leading; 3] = [Leading::Tight, Leading::Normal, Leading::Loose];

    pub fn key(self) -> &'static str {
        match self {
            Leading::Tight => "tight",
            Leading::Normal => "normal",
            Leading::Loose => "loose",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Leading::Tight => "緊",
            Leading::Normal => "中",
            Leading::Loose => "鬆",
        }
    }

    pub fn factor(self) -> &'static str {
        match self {
            Leading::Tight => "1.9",
            Leading::Normal => "2.08",
            Leading::Loose => "2.3",
        }
    }

    pub fn parse(value: &str) -> Option<Leading> {
        Leading::ALL
            .into_iter()
            .find(|step| step.key() == value.trim())
    }
}

/// The four reader knobs, already resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prefs {
    pub layout: Layout,
    pub script: Script,
    pub orig: FontStep,
    pub tran: FontStep,
    pub leading: Leading,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            layout: Layout::Three,
            script: Script::Both,
            orig: FontStep::Medium,
            tran: FontStep::Medium,
            leading: Leading::Normal,
        }
    }
}

impl Prefs {
    /// Resolve from query then cookie. Returns the prefs plus the cookies the
    /// caller should write back (only for values the query explicitly asked for).
    pub fn resolve(query: &Query, cookies: &Cookies) -> (Prefs, Vec<(&'static str, &'static str)>) {
        let mut prefs = Prefs::default();
        let mut persist = Vec::new();

        if let Some(layout) = urlcodec::get(query, "layout").and_then(Layout::parse) {
            prefs.layout = layout;
            persist.push((COOKIE_LAYOUT, layout.key()));
        } else if let Some(layout) =
            cookie(cookies, COOKIE_LAYOUT).and_then(|value| Layout::parse(&value))
        {
            prefs.layout = layout;
        }

        if let Some(step) = urlcodec::get(query, "orig").and_then(FontStep::parse) {
            prefs.orig = step;
            persist.push((COOKIE_ORIG, step.key()));
        } else if let Some(step) =
            cookie(cookies, COOKIE_ORIG).and_then(|value| FontStep::parse(&value))
        {
            prefs.orig = step;
        }

        if let Some(step) = urlcodec::get(query, "tran").and_then(FontStep::parse) {
            prefs.tran = step;
            persist.push((COOKIE_TRAN, step.key()));
        } else if let Some(step) =
            cookie(cookies, COOKIE_TRAN).and_then(|value| FontStep::parse(&value))
        {
            prefs.tran = step;
        }

        if let Some(leading) = urlcodec::get(query, "lh").and_then(Leading::parse) {
            prefs.leading = leading;
            persist.push((COOKIE_LEADING, leading.key()));
        } else if let Some(leading) =
            cookie(cookies, COOKIE_LEADING).and_then(|value| Leading::parse(&value))
        {
            prefs.leading = leading;
        }

        if let Some(script) = urlcodec::get(query, "sc").and_then(Script::parse) {
            prefs.script = script;
            persist.push((COOKIE_SCRIPT, script.key()));
        } else if let Some(script) =
            cookie(cookies, COOKIE_SCRIPT).and_then(|value| Script::parse(&value))
        {
            prefs.script = script;
        }

        (prefs, persist)
    }

    /// Inline custom-property declaration for the html element.
    pub fn root_style(&self) -> String {
        format!(
            "--orig-size:{}rem;--tran-size:{}rem;--leading:{}",
            self.orig.orig_rem(),
            self.tran.tran_rem(),
            self.leading.factor()
        )
    }
}

fn cookie(cookies: &Cookies, name: &str) -> Option<String> {
    cookies
        .get(name)
        .map(|value| urlcodec::percent_decode(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(raw: &str) -> Query {
        urlcodec::parse_query(raw)
    }

    fn cookies(raw: &str) -> Cookies {
        urlcodec::parse_cookies(raw)
    }

    #[test]
    fn defaults_are_three_columns_medium_normal() {
        let (prefs, persist) = Prefs::resolve(&query(""), &cookies(""));
        assert_eq!(prefs, Prefs::default());
        assert_eq!(prefs.layout, Layout::Three);
        assert!(persist.is_empty());
    }

    #[test]
    fn query_wins_over_cookie_and_is_persisted() {
        let (prefs, persist) = Prefs::resolve(
            &query("layout=single&orig=xl&tran=s&lh=loose"),
            &cookies("zgc_layout=dual; zgc_orig=s; zgc_tran=xl; zgc_leading=tight"),
        );
        assert_eq!(prefs.layout, Layout::One);
        assert_eq!(prefs.orig, FontStep::Huge);
        assert_eq!(prefs.tran, FontStep::Small);
        assert_eq!(prefs.leading, Leading::Loose);
        assert_eq!(persist.len(), 4);
    }

    #[test]
    fn cookie_applies_when_query_is_absent() {
        let (prefs, persist) = Prefs::resolve(
            &query("p=x"),
            &cookies("zgc_layout=dual; zgc_leading=tight"),
        );
        assert_eq!(prefs.layout, Layout::Two);
        assert_eq!(prefs.leading, Leading::Tight);
        assert!(persist.is_empty(), "cookies are not re-set on every page");
    }

    #[test]
    fn invalid_values_are_ignored_not_panicked() {
        let (prefs, persist) = Prefs::resolve(
            &query("layout=%3Cscript%3E&orig=999&lh=&tran="),
            &cookies("zgc_layout=../../etc"),
        );
        assert_eq!(prefs, Prefs::default());
        assert!(persist.is_empty());
    }

    #[test]
    fn every_leading_step_respects_the_1_9_floor() {
        for step in Leading::ALL {
            let factor: f64 = step.factor().parse().unwrap();
            assert!(factor >= 1.9, "{step:?} → {factor}");
        }
    }

    #[test]
    fn font_steps_are_ordered_and_grow() {
        let sizes: Vec<f64> = FontStep::ALL
            .iter()
            .map(|step| step.orig_rem().parse().unwrap())
            .collect();
        assert!(sizes.len() >= 3, "at least three steps are required");
        assert!(sizes.windows(2).all(|pair| pair[0] < pair[1]), "{sizes:?}");
    }

    #[test]
    fn both_is_the_default_script() {
        assert_eq!(Prefs::default().script, Script::Both);
        let (prefs, persist) = Prefs::resolve(&query(""), &cookies(""));
        assert_eq!(prefs.script, Script::Both);
        assert!(persist.is_empty());
    }

    #[test]
    fn script_query_wins_over_cookie_and_is_persisted() {
        let (prefs, persist) = Prefs::resolve(&query("sc=trad"), &cookies("zgc_script=simp"));
        assert_eq!(prefs.script, Script::Trad);
        assert_eq!(persist, vec![(COOKIE_SCRIPT, "trad")]);
    }

    #[test]
    fn script_cookie_applies_when_query_is_absent() {
        let (prefs, persist) = Prefs::resolve(&query("p=x"), &cookies("zgc_script=trad"));
        assert_eq!(prefs.script, Script::Trad);
        assert!(persist.is_empty());
    }

    #[test]
    fn script_parse_accepts_keys_and_glyphs() {
        assert_eq!(Script::parse("both"), Some(Script::Both));
        assert_eq!(Script::parse("trad"), Some(Script::Trad));
        assert_eq!(Script::parse("simp"), Some(Script::Simp));
        assert_eq!(Script::parse("繁"), Some(Script::Trad));
        assert_eq!(Script::parse("繁體"), Some(Script::Trad));
        assert_eq!(Script::parse("简"), Some(Script::Simp));
        assert_eq!(Script::parse("簡"), Some(Script::Simp));
        assert_eq!(Script::parse("簡體"), Some(Script::Simp));
        assert_eq!(Script::parse("  simp  "), Some(Script::Simp));
        assert_eq!(Script::parse("bogus"), None);
        assert_eq!(Script::Both.label(), "繁簡對照");
        assert_eq!(Script::Trad.label(), "繁體");
        assert_eq!(Script::Simp.label(), "簡體");
        assert_eq!(Script::Both.toggle_label(), "對照");
        assert_eq!(Script::Trad.toggle_label(), "繁");
        assert_eq!(Script::Simp.toggle_label(), "簡");
    }

    #[test]
    fn every_script_key_round_trips_through_parse() {
        for script in Script::ALL {
            assert_eq!(Script::parse(script.key()), Some(script), "{script:?}");
        }
    }

    #[test]
    fn root_style_carries_resolved_sizes() {
        let (prefs, _) = Prefs::resolve(&query("orig=l&tran=s&lh=tight"), &cookies(""));
        assert_eq!(
            prefs.root_style(),
            "--orig-size:1.46rem;--tran-size:0.92rem;--leading:1.9"
        );
    }
}
