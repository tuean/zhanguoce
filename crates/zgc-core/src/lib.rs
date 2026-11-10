//! zgc-core — UI-agnostic core for the 戰國策 reader.
//!
//! Nothing in this crate knows about rendering: it provides the schema types
//! (`types`), the fail-fast loader (`Book::load`), the validated tree with
//! reading-order navigation (`BookData`), and a CJK bigram search index
//! (`SearchIndex`). Any UI (Leptos, egui, …) can drive it as-is.
//!
//! ```no_run
//! use std::path::Path;
//! use zgc_core::{Book, Scope, SearchIndex};
//!
//! let data = Book::load(Path::new("data/zhanguoce"))?;
//! let index = SearchIndex::build(&data);
//! for hit in index.search_limited(&data, "衛鞅", &Scope::All, 5) {
//!     println!("{}/{}/{} · {}", hit.volume_id, hit.chapter_id, hit.paragraph_id, hit.snippet);
//! }
//! # Ok::<(), zgc_core::LoadError>(())
//! ```

mod chapter_years;
mod data;
mod entities;
mod error;
mod events;
mod load;
mod nav;
mod prompt;
mod search;
mod strategy;
mod types;
pub mod variants;

pub use chapter_years::{
    reason_text, ChapterYear, ChapterYearError, ChapterYearFile, ChapterYearResult, ChapterYears,
    Confidence, Evidence, EvidenceText, UnresolvedChapter, CHAPTER_YEAR_FILE,
};
pub use data::{BookData, Position};
pub use entities::{
    Entities, Entity, EntityError, EntityFile, EntityKind, EntityRecord, EntityResult, Occurrence,
    OccurrenceRecord, SpanView,
};
pub use error::{LoadError, Location, Result};
pub use events::{
    parse_div_year, parse_year_label, Event, EventError, EventFile, EventResult, EventYear, Events,
    EVENT_FILE,
};
pub use prompt::{
    build as build_prompt, AskContext, Budget, EntityContext, GlossaryText, NoteText, Prompt,
    YearContext, MAX_ENTITY_CHARS, MAX_NOTES_CHARS, MAX_ORIGINAL_CHARS, MAX_QUESTION_CHARS,
    MAX_TRANSLATION_CHARS, MAX_USER_CHARS, MAX_YEAR_CHARS, SYSTEM_PROMPT, TRUNCATION_MARK,
};
pub use search::{FieldSource, Hit, Scope, SearchIndex};
pub use strategy::{
    reason_text as strategy_reason_text, ChapterStats, ChapterStrategy, Device, DeviceKind,
    Strategy, StrategyError, StrategyFile, StrategyResult, Turn, UnresolvedTurn, STRATEGY_FILE,
};
pub use types::{
    Annotation, Book, Chapter, Glossary, GlossaryTerm, Group, Paragraph, Source, StructureAxis,
    Volume,
};
pub use variants::Variants;
