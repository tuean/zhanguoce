//! Serde types mirroring `docs/SCHEMA.md` (v1), plus the structure-axis enum
//! derived from `book.form`.
//!
//! Every struct uses `rename_all = "camelCase"` so the on-disk JSON keys stay
//! exactly as documented. Fields that may legitimately be absent default to
//! `None` (options) or an empty `Vec` (`annotations`, `glossary`).

use serde::{Deserialize, Serialize};

fn default_schema_version() -> u32 {
    1
}

/// Reading-order structure axis, decided by `book.form` (never hard-coded per book).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureAxis {
    /// 國別體: group (state) → volume → chapter → paragraph.
    Polity,
    /// 編年體: reserved for later books. Traversal still works because groups,
    /// volumes, chapters and paragraphs stay generic; only presentation differs.
    Chronology,
}

impl StructureAxis {
    /// Map the free-form `form` string to an axis. Unknown forms fall back to
    /// `Polity`, which is the schema's documented default.
    pub fn from_form(form: &str) -> Self {
        match form.trim() {
            "編年體" | "编年体" => StructureAxis::Chronology,
            _ => StructureAxis::Polity,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            StructureAxis::Polity => "國別體",
            StructureAxis::Chronology => "編年體",
        }
    }
}

/// `book.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Book {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub title_simp: Option<String>,
    #[serde(default)]
    pub compiler: Option<String>,
    #[serde(default)]
    pub era: Option<String>,
    /// 國別體 / 編年體 — decides the navigation axis. Empty when absent.
    #[serde(default)]
    pub form: String,
    #[serde(default)]
    pub sources: Vec<Source>,
    #[serde(default)]
    pub groups: Vec<Group>,
    /// Canonical reading order of volume ids. Never re-sorted by the loader.
    #[serde(default)]
    pub volume_order: Vec<String>,
}

impl Book {
    /// Structure axis for this book; unknown/absent `form` degrades to 國別體.
    pub fn axis(&self) -> StructureAxis {
        StructureAxis::from_form(&self.form)
    }

    /// Look up a group by id.
    pub fn group(&self, id: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    /// Group ids in book order.
    pub fn group_ids(&self) -> impl Iterator<Item = &str> {
        self.groups.iter().map(|g| g.id.as_str())
    }
}

/// Provenance entry for a book.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
}

/// One navigation group (a state for 國別體, a reign era for 編年體).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub title_simp: Option<String>,
    /// Volume ids belonging to this group, in reading order.
    #[serde(default)]
    pub volumes: Vec<String>,
}

/// One volume file (`volumes/<vol-id>.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Volume {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub id: String,
    #[serde(default)]
    pub book_id: String,
    #[serde(default)]
    pub group_id: String,
    pub title: String,
    #[serde(default)]
    pub title_simp: Option<String>,
    #[serde(default)]
    pub index: u32,
    #[serde(default)]
    pub chapters: Vec<Chapter>,
}

/// One chapter (a single 策 piece) inside a volume.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chapter {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub paragraphs: Vec<Paragraph>,
}

/// The smallest anchor in the whole system: everything else hangs off its id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Paragraph {
    pub id: String,
    /// 繁体原文.
    pub text: String,
    /// 简体白话; absent when the translation has not been written yet.
    #[serde(default)]
    pub translation: Option<String>,
    /// 注疏; empty is normal (see docs/SCHEMA.md hard constraint 3).
    #[serde(default)]
    pub annotations: Vec<Annotation>,
    /// 字词训释; empty is normal.
    #[serde(default)]
    pub glossary: Vec<Glossary>,
}

/// One commentary note attached to a paragraph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Annotation {
    /// Commentator, e.g. 鮑彪 / 吳師道. Absent when the source is unknown.
    #[serde(default)]
    pub source: Option<String>,
    pub text: String,
    /// Witness edition the note was read from: 四部叢刊 (base) or 四庫全書.
    /// Absent means 四部叢刊, so pre-M1d data still parses unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edition: Option<String>,
}

/// One glossary entry (term → reading + explanation) attached to a paragraph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Glossary {
    pub term: String,
    #[serde(default)]
    pub pinyin: Option<String>,
    #[serde(default)]
    pub gloss: Option<String>,
}

/// Readable alias for a single glossary entry.
pub type GlossaryTerm = Glossary;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraph_defaults_are_empty_collections() {
        let p: Paragraph = serde_json::from_str(r#"{"id":"v-01-p0001","text":"秦"}"#).unwrap();
        assert_eq!(p.text, "秦");
        assert!(p.translation.is_none());
        assert!(p.annotations.is_empty());
        assert!(p.glossary.is_empty());
    }

    #[test]
    fn book_parses_camel_case_keys() {
        let b: Book = serde_json::from_str(
            r#"{
                "schemaVersion": 1,
                "id": "zhanguoce",
                "title": "戰國策",
                "titleSimp": "战国策",
                "form": "國別體",
                "groups": [{"id":"qin","title":"秦策","volumes":["qin-1"]}],
                "volumeOrder": ["qin-1"]
            }"#,
        )
        .unwrap();
        assert_eq!(b.title_simp.as_deref(), Some("战国策"));
        assert_eq!(b.volume_order, vec!["qin-1"]);
        assert_eq!(b.groups[0].volumes, vec!["qin-1"]);
        assert_eq!(b.axis(), StructureAxis::Polity);
    }

    #[test]
    fn missing_schema_version_defaults_to_one() {
        let v: Volume = serde_json::from_str(r#"{"id":"qin-1","title":"秦策一"}"#).unwrap();
        assert_eq!(v.schema_version, 1);
        assert_eq!(v.index, 0);
        assert!(v.chapters.is_empty());
    }

    #[test]
    fn form_drives_structure_axis_and_chronology_is_reserved() {
        assert_eq!(StructureAxis::from_form("國別體"), StructureAxis::Polity);
        assert_eq!(
            StructureAxis::from_form("編年體"),
            StructureAxis::Chronology
        );
        assert_eq!(
            StructureAxis::from_form("编年体"),
            StructureAxis::Chronology
        );
        assert_eq!(StructureAxis::from_form(""), StructureAxis::Polity);
        assert_eq!(StructureAxis::from_form("未知"), StructureAxis::Polity);
    }

    #[test]
    fn serde_round_trip_keeps_keys() {
        let p = Paragraph {
            id: "qin-1-01-p0001".into(),
            text: "衛鞅亡魏入秦".into(),
            translation: None,
            annotations: vec![Annotation {
                source: Some("鮑彪".into()),
                text: "注".into(),
                edition: None,
            }],
            glossary: vec![Glossary {
                term: "衛鞅".into(),
                pinyin: Some("wei yang".into()),
                gloss: Some("即商鞅".into()),
            }],
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains(r#""annotations""#));
        assert!(json.contains(r#""translation":null"#));
        let back: Paragraph = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn edition_is_optional_and_absent_means_the_base_witness() {
        let old =
            r#"{"id":"p1","text":"衞鞅亡魏入秦","annotations":[{"source":"鮑彪","text":"注"}]}"#;
        let para: Paragraph = serde_json::from_str(old).unwrap();
        assert_eq!(para.annotations[0].edition, None);
        let again = serde_json::to_string(&para).unwrap();
        assert!(
            !again.contains("edition"),
            "absent edition must stay absent: {again}"
        );

        let wyg = r#"{"id":"p1","text":"衞鞅亡魏入秦","annotations":[{"source":"吴师道正","text":"注","edition":"四庫全書"}]}"#;
        let para: Paragraph = serde_json::from_str(wyg).unwrap();
        assert_eq!(para.annotations[0].edition.as_deref(), Some("四庫全書"));
    }
}
