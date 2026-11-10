//! `Book::load`: read `book.json` + `volumes/*.json` and fail fast on broken data.
//!
//! Ordering rule: volumes are read in `book.volumeOrder` sequence. File names are
//! only used to locate a volume, never to order them.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

use crate::data::BookData;
use crate::error::{LoadError, Result};
use crate::types::{Book, Volume};

impl Book {
    /// Load a book directory (`book.json` + `volumes/<vol-id>.json`).
    ///
    /// The returned `BookData` is already validated: paragraph ids are globally
    /// unique, the file set matches `volumeOrder` exactly, every group reference
    /// resolves, and no paragraph has empty `text`.
    pub fn load(dir: &Path) -> Result<BookData> {
        let book_path = dir.join("book.json");
        let book: Book = read_json(&book_path)?;

        if book.volume_order.is_empty() {
            return Err(LoadError::EmptyVolumeOrder { book: book_path });
        }

        let mut declared: HashSet<&str> = HashSet::new();
        for volume_id in &book.volume_order {
            if volume_id.is_empty()
                || volume_id.contains(['/', '\\'])
                || volume_id == "."
                || volume_id == ".."
            {
                return Err(LoadError::InvalidVolumeId {
                    volume_id: volume_id.clone(),
                });
            }
            if !declared.insert(volume_id.as_str()) {
                return Err(LoadError::DuplicateVolumeOrderId {
                    volume_id: volume_id.clone(),
                });
            }
        }

        let volumes_dir = dir.join("volumes");
        let mut volumes = Vec::with_capacity(book.volume_order.len());
        for volume_id in &book.volume_order {
            let path = volumes_dir.join(format!("{volume_id}.json"));
            let volume: Volume = match read_json_optional(&path)? {
                Some(volume) => volume,
                None => {
                    return Err(LoadError::MissingVolumeFile {
                        volume_id: volume_id.clone(),
                        path,
                    })
                }
            };
            if volume.id != *volume_id {
                return Err(LoadError::VolumeIdMismatch {
                    expected: volume_id.clone(),
                    actual: volume.id,
                    path,
                });
            }
            volumes.push(volume);
        }

        // Fail when volumes/ holds a JSON volume that volumeOrder never mentions:
        // silently ignoring it would make the reading order a lie.
        let mut unexpected: Vec<(String, PathBuf)> = Vec::new();
        let entries =
            std::fs::read_dir(&volumes_dir).map_err(|error| LoadError::io(&volumes_dir, error))?;
        for entry in entries {
            let entry = entry.map_err(|error| LoadError::io(&volumes_dir, error))?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if !declared.contains(stem) {
                unexpected.push((stem.to_string(), path));
            }
        }
        unexpected.sort();
        if let Some((volume_id, path)) = unexpected.into_iter().next() {
            return Err(LoadError::UnexpectedVolumeFile { volume_id, path });
        }

        BookData::from_parts(book, volumes)
    }
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = std::fs::read(path).map_err(|error| LoadError::io(path, error))?;
    serde_json::from_slice(&bytes).map_err(|error| LoadError::json(path, error))
}

fn read_json_optional<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| LoadError::json(path, error)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(LoadError::io(path, error)),
    }
}
