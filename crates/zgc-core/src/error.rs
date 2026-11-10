//! Fail-fast load/validation errors.
//!
//! Every error names the offending volume and, where applicable, the chapter and
//! paragraph, so a data bug can be located without re-parsing the JSON by hand.

use std::fmt;
use std::path::{Path, PathBuf};

/// Convenience alias used throughout this crate.
pub type Result<T, E = LoadError> = std::result::Result<T, E>;

/// Where a problem was found inside the book tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub volume_id: String,
    pub chapter_id: Option<String>,
    pub paragraph_id: Option<String>,
}

impl Location {
    pub fn volume(volume_id: impl Into<String>) -> Self {
        Location {
            volume_id: volume_id.into(),
            chapter_id: None,
            paragraph_id: None,
        }
    }

    pub fn chapter(volume_id: impl Into<String>, chapter_id: impl Into<String>) -> Self {
        Location {
            volume_id: volume_id.into(),
            chapter_id: Some(chapter_id.into()),
            paragraph_id: None,
        }
    }

    pub fn paragraph(
        volume_id: impl Into<String>,
        chapter_id: Option<&str>,
        paragraph_id: impl Into<String>,
    ) -> Self {
        Location {
            volume_id: volume_id.into(),
            chapter_id: chapter_id.map(str::to_owned),
            paragraph_id: Some(paragraph_id.into()),
        }
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "卷 {}", self.volume_id)?;
        if let Some(chapter) = &self.chapter_id {
            write!(f, " / 章 {chapter}")?;
        }
        if let Some(paragraph) = &self.paragraph_id {
            write!(f, " / 段 {paragraph}")?;
        }
        Ok(())
    }
}

/// Everything that can stop a load.
#[derive(Debug)]
pub enum LoadError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    /// `book.json` has no `volumeOrder` entries at all.
    EmptyVolumeOrder {
        book: PathBuf,
    },
    /// The same volume id appears twice in `volumeOrder`.
    DuplicateVolumeOrderId {
        volume_id: String,
    },
    /// A `volumeOrder` entry is not a safe single path segment.
    InvalidVolumeId {
        volume_id: String,
    },
    /// A volume id in `volumeOrder` has no `volumes/<id>.json`.
    MissingVolumeFile {
        volume_id: String,
        path: PathBuf,
    },
    /// `volumes/` contains a JSON file that `volumeOrder` does not mention.
    UnexpectedVolumeFile {
        volume_id: String,
        path: PathBuf,
    },
    /// A volume file's internal `id` disagrees with its file name.
    VolumeIdMismatch {
        expected: String,
        actual: String,
        path: PathBuf,
    },
    /// `volumeOrder` and the supplied volumes do not cover each other exactly.
    VolumeOrderMismatch {
        missing: Vec<String>,
        extra: Vec<String>,
    },
    DuplicateVolumeId {
        volume_id: String,
    },
    DuplicateGroupId {
        group_id: String,
    },
    /// A group lists a volume id that `volumeOrder` does not contain.
    GroupReferencesUnknownVolume {
        group_id: String,
        volume_id: String,
    },
    /// A volume's `groupId` does not exist in `book.json`.
    VolumeReferencesUnknownGroup {
        volume_id: String,
        group_id: String,
    },
    /// A volume is listed by more than one group (each volume belongs to one).
    VolumeInMultipleGroups {
        volume_id: String,
        groups: Vec<String>,
    },
    DuplicateChapterId {
        chapter_id: String,
        first: Location,
        second: Location,
    },
    DuplicateParagraphId {
        paragraph_id: String,
        first: Location,
        second: Location,
    },
    /// `text` is empty or whitespace-only.
    EmptyParagraphText {
        location: Location,
    },
}

impl LoadError {
    /// Structured location, when the error is tied to one place in the data.
    pub fn location(&self) -> Option<&Location> {
        match self {
            LoadError::DuplicateChapterId { second, .. }
            | LoadError::DuplicateParagraphId { second, .. } => Some(second),
            LoadError::EmptyParagraphText { location } => Some(location),
            _ => None,
        }
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Io { path, source } => write!(f, "读取 {} 失败：{source}", path.display()),
            LoadError::Json { path, source } => {
                write!(f, "解析 {} 失败：{source}", path.display())
            }
            LoadError::EmptyVolumeOrder { book } => {
                write!(f, "{} 的 volumeOrder 为空，无法确定卷序", book.display())
            }
            LoadError::DuplicateVolumeOrderId { volume_id } => {
                write!(f, "book.json 的 volumeOrder 里卷 id {volume_id} 重复")
            }
            LoadError::InvalidVolumeId { volume_id } => {
                write!(f, "volumeOrder 里的卷 id {volume_id:?} 不是合法的文件名词")
            }
            LoadError::MissingVolumeFile { volume_id, path } => write!(
                f,
                "volumeOrder 声明了卷 {volume_id}，但文件 {} 不存在",
                path.display()
            ),
            LoadError::UnexpectedVolumeFile { volume_id, path } => write!(
                f,
                "卷文件 {}（id {volume_id}）不在 book.json 的 volumeOrder 中",
                path.display()
            ),
            LoadError::VolumeIdMismatch {
                expected,
                actual,
                path,
            } => write!(
                f,
                "卷文件 {} 的内部 id 为 {actual}，与文件名/volumOrder 的 {expected} 不一致",
                path.display()
            ),
            LoadError::VolumeOrderMismatch { missing, extra } => {
                write!(f, "volumeOrder 与实际卷集合不一致")?;
                if !missing.is_empty() {
                    write!(f, "；缺少：{}", missing.join(", "))?;
                }
                if !extra.is_empty() {
                    write!(f, "；多出：{}", extra.join(", "))?;
                }
                Ok(())
            }
            LoadError::DuplicateVolumeId { volume_id } => {
                write!(f, "卷 id {volume_id} 重复")
            }
            LoadError::DuplicateGroupId { group_id } => {
                write!(f, "group id {group_id} 重复")
            }
            LoadError::GroupReferencesUnknownVolume {
                group_id,
                volume_id,
            } => write!(
                f,
                "group {group_id} 引用了不存在的卷 {volume_id}（volumeOrder 中无此卷）"
            ),
            LoadError::VolumeReferencesUnknownGroup {
                volume_id,
                group_id,
            } => write!(
                f,
                "卷 {volume_id} 的 groupId={group_id} 在 book.json 的 groups 中不存在"
            ),
            LoadError::VolumeInMultipleGroups { volume_id, groups } => write!(
                f,
                "卷 {volume_id} 被多个 group 列出：{}（每卷必须恰好归属一个 group）",
                groups.join(", ")
            ),
            LoadError::DuplicateChapterId {
                chapter_id,
                first,
                second,
            } => write!(
                f,
                "章 id {chapter_id} 重复：先出现于 {first}，又出现于 {second}"
            ),
            LoadError::DuplicateParagraphId {
                paragraph_id,
                first,
                second,
            } => write!(
                f,
                "段 id {paragraph_id} 重复：先出现于 {first}，又出现于 {second}"
            ),
            LoadError::EmptyParagraphText { location } => {
                write!(f, "{location} 的 text 为空")
            }
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LoadError::Io { source, .. } => Some(source),
            LoadError::Json { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl LoadError {
    pub(crate) fn io(path: &Path, source: std::io::Error) -> Self {
        LoadError::Io {
            path: path.to_path_buf(),
            source,
        }
    }

    pub(crate) fn json(path: &Path, source: serde_json::Error) -> Self {
        LoadError::Json {
            path: path.to_path_buf(),
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_display_names_volume_chapter_paragraph() {
        let loc = Location::paragraph("qin-1", Some("qin-1-01"), "qin-1-01-p0001");
        assert_eq!(
            loc.to_string(),
            "卷 qin-1 / 章 qin-1-01 / 段 qin-1-01-p0001"
        );
        assert_eq!(Location::volume("qin-1").to_string(), "卷 qin-1");
    }

    #[test]
    fn duplicate_paragraph_error_points_at_both_sides() {
        let err = LoadError::DuplicateParagraphId {
            paragraph_id: "qin-1-01-p0001".into(),
            first: Location::chapter("qin-1", "qin-1-01"),
            second: Location::chapter("qin-2", "qin-2-01"),
        };
        let text = err.to_string();
        assert!(text.contains("qin-1-01-p0001"));
        assert!(text.contains("卷 qin-2 / 章 qin-2-01"));
        assert!(err.location().is_some());
    }
}
