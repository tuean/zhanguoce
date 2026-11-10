//! Validated, indexed, in-memory book tree.
//!
//! `BookData` keeps volumes in `book.json`'s `volumeOrder` (never by file name or
//! `index`), stores a flat paragraph order for the reader and for search, and
//! exposes the group → volume → chapter → paragraph traversal required by any UI.

use std::collections::HashMap;

use crate::error::{LoadError, Location, Result};
use crate::types::{Book, Chapter, Group, Paragraph, StructureAxis, Volume};

/// A position in the book tree, as indices into the validated structure.
///
/// Indices are stable for one loaded `BookData`; the stable public anchor is
/// always the paragraph id (`Position` is a navigation handle, not an id).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Position {
    pub volume: usize,
    pub chapter: usize,
    pub paragraph: usize,
}

impl Position {
    pub const fn new(volume: usize, chapter: usize, paragraph: usize) -> Self {
        Position {
            volume,
            chapter,
            paragraph,
        }
    }
}

/// A loaded, validated book plus its derived indexes.
#[derive(Debug, Clone)]
pub struct BookData {
    book: Book,
    /// Volumes in `volumeOrder` (the only canonical reading order).
    volumes: Vec<Volume>,
    /// Per group (same order as `book.groups`): volume indices in reading order.
    group_volume_indices: Vec<Vec<usize>>,
    group_index_by_id: HashMap<String, usize>,
    volume_index_by_id: HashMap<String, usize>,
    /// `[volume][chapter]` → flat index of that chapter's first paragraph.
    chapter_flat_starts: Vec<Vec<usize>>,
    /// Flat reading order of every paragraph in the book.
    flat_positions: Vec<Position>,
}

impl BookData {
    /// Validate arbitrary in-memory parts and build a `BookData`.
    ///
    /// `volumes` may arrive in any order: they are re-ordered to match
    /// `book.volume_order`. All hard invariants from `docs/SCHEMA.md` that do not
    /// need file paths are enforced here, so `Book::load` and programmatic
    /// construction share one validator.
    pub fn from_parts(book: Book, volumes: Vec<Volume>) -> Result<Self> {
        let mut group_index_by_id = HashMap::new();
        for (index, group) in book.groups.iter().enumerate() {
            if group_index_by_id.insert(group.id.clone(), index).is_some() {
                return Err(LoadError::DuplicateGroupId {
                    group_id: group.id.clone(),
                });
            }
        }

        let mut order_index = HashMap::new();
        for (index, volume_id) in book.volume_order.iter().enumerate() {
            if order_index.insert(volume_id.clone(), index).is_some() {
                return Err(LoadError::DuplicateVolumeOrderId {
                    volume_id: volume_id.clone(),
                });
            }
        }

        let mut volume_index_by_id = HashMap::new();
        for (index, volume) in volumes.iter().enumerate() {
            if volume_index_by_id
                .insert(volume.id.clone(), index)
                .is_some()
            {
                return Err(LoadError::DuplicateVolumeId {
                    volume_id: volume.id.clone(),
                });
            }
        }

        let missing: Vec<String> = book
            .volume_order
            .iter()
            .filter(|id| !volume_index_by_id.contains_key(*id))
            .cloned()
            .collect();
        let extra: Vec<String> = volumes
            .iter()
            .filter(|volume| !order_index.contains_key(&volume.id))
            .map(|volume| volume.id.clone())
            .collect();
        if !missing.is_empty() || !extra.is_empty() {
            return Err(LoadError::VolumeOrderMismatch { missing, extra });
        }

        let mut ordered: Vec<Volume> = volumes;
        ordered.sort_by_key(|volume| order_index[&volume.id]);

        for group in &book.groups {
            for volume_id in &group.volumes {
                if !order_index.contains_key(volume_id) {
                    return Err(LoadError::GroupReferencesUnknownVolume {
                        group_id: group.id.clone(),
                        volume_id: volume_id.clone(),
                    });
                }
            }
        }
        for volume in &ordered {
            if !volume.group_id.is_empty() && !group_index_by_id.contains_key(&volume.group_id) {
                return Err(LoadError::VolumeReferencesUnknownGroup {
                    volume_id: volume.id.clone(),
                    group_id: volume.group_id.clone(),
                });
            }
        }

        // Each volume belongs to at most one group: a volume listed twice would
        // render twice in the table of contents, and no later layer could tell
        // which membership is canonical. Zero memberships stays legal (an
        // ungrouped volume is still ordered by `volumeOrder`).
        let mut volume_groups: HashMap<String, Vec<String>> = HashMap::new();
        for group in &book.groups {
            for volume_id in &group.volumes {
                let groups = volume_groups.entry(volume_id.clone()).or_default();
                // The same group listing a volume twice is deduped, not an error.
                if groups.last() != Some(&group.id) {
                    groups.push(group.id.clone());
                }
            }
        }
        for volume_id in &book.volume_order {
            if let Some(groups) = volume_groups.get(volume_id) {
                if groups.len() > 1 {
                    return Err(LoadError::VolumeInMultipleGroups {
                        volume_id: volume_id.clone(),
                        groups: groups.clone(),
                    });
                }
            }
        }

        let mut chapter_ids: HashMap<String, Location> = HashMap::new();
        let mut paragraph_ids: HashMap<String, Location> = HashMap::new();
        let mut flat_positions: Vec<Position> = Vec::new();
        let mut chapter_flat_starts: Vec<Vec<usize>> = Vec::with_capacity(ordered.len());

        for (volume_index, volume) in ordered.iter().enumerate() {
            let mut starts = Vec::with_capacity(volume.chapters.len());
            for (chapter_index, chapter) in volume.chapters.iter().enumerate() {
                let chapter_location = Location::chapter(&volume.id, &chapter.id);
                if let Some(first) =
                    chapter_ids.insert(chapter.id.clone(), chapter_location.clone())
                {
                    return Err(LoadError::DuplicateChapterId {
                        chapter_id: chapter.id.clone(),
                        first,
                        second: chapter_location,
                    });
                }
                starts.push(flat_positions.len());
                for (paragraph_index, paragraph) in chapter.paragraphs.iter().enumerate() {
                    let paragraph_location =
                        Location::paragraph(&volume.id, Some(&chapter.id), &paragraph.id);
                    if paragraph.text.trim().is_empty() {
                        return Err(LoadError::EmptyParagraphText {
                            location: paragraph_location,
                        });
                    }
                    if let Some(first) =
                        paragraph_ids.insert(paragraph.id.clone(), paragraph_location.clone())
                    {
                        return Err(LoadError::DuplicateParagraphId {
                            paragraph_id: paragraph.id.clone(),
                            first,
                            second: paragraph_location,
                        });
                    }
                    flat_positions.push(Position {
                        volume: volume_index,
                        chapter: chapter_index,
                        paragraph: paragraph_index,
                    });
                }
            }
            chapter_flat_starts.push(starts);
        }

        let group_volume_indices: Vec<Vec<usize>> = book
            .groups
            .iter()
            .map(|group| {
                let mut indices: Vec<usize> = group
                    .volumes
                    .iter()
                    .filter_map(|id| volume_index_by_id.get(id).copied())
                    .collect();
                indices.sort_unstable();
                indices.dedup();
                indices
            })
            .collect();

        Ok(BookData {
            book,
            volumes: ordered,
            group_volume_indices,
            group_index_by_id,
            volume_index_by_id,
            chapter_flat_starts,
            flat_positions,
        })
    }

    /// Book metadata.
    pub fn book(&self) -> &Book {
        &self.book
    }

    /// Structure axis chosen by `book.form`.
    pub fn axis(&self) -> StructureAxis {
        self.book.axis()
    }

    /// Volumes in `volumeOrder`.
    pub fn volumes(&self) -> &[Volume] {
        &self.volumes
    }

    pub fn volume_count(&self) -> usize {
        self.volumes.len()
    }

    pub fn group_count(&self) -> usize {
        self.book.groups.len()
    }

    pub fn chapter_count(&self) -> usize {
        self.volumes
            .iter()
            .map(|volume| volume.chapters.len())
            .sum()
    }

    pub fn paragraph_count(&self) -> usize {
        self.flat_positions.len()
    }

    /// Groups in `book.json` order.
    pub fn groups(&self) -> impl Iterator<Item = &Group> {
        self.book.groups.iter()
    }

    pub fn group(&self, id: &str) -> Option<&Group> {
        self.group_index_by_id
            .get(id)
            .map(|index| &self.book.groups[*index])
    }

    /// Volume indices of a group, in reading order. Empty for unknown groups.
    pub fn group_volume_indices(&self, group_id: &str) -> &[usize] {
        match self.group_index_by_id.get(group_id) {
            Some(index) => &self.group_volume_indices[*index],
            None => &[],
        }
    }

    /// Volumes of a group, in reading order.
    pub fn group_volumes(&self, group_id: &str) -> impl Iterator<Item = &Volume> {
        self.group_volume_indices(group_id)
            .iter()
            .map(move |index| &self.volumes[*index])
    }

    pub fn volume_index(&self, id: &str) -> Option<usize> {
        self.volume_index_by_id.get(id).copied()
    }

    pub fn volume(&self, index: usize) -> Option<&Volume> {
        self.volumes.get(index)
    }

    pub fn volume_by_id(&self, id: &str) -> Option<&Volume> {
        self.volume_index(id).and_then(|index| self.volume(index))
    }

    pub fn chapter(&self, volume: usize, chapter: usize) -> Option<&Chapter> {
        self.volumes.get(volume)?.chapters.get(chapter)
    }

    pub fn paragraph(&self, position: Position) -> Option<&Paragraph> {
        self.chapter(position.volume, position.chapter)?
            .paragraphs
            .get(position.paragraph)
    }

    /// Flat index of the chapter's first paragraph (used by search postings).
    pub fn chapter_start_index(&self, volume: usize, chapter: usize) -> Option<usize> {
        self.chapter_flat_starts.get(volume)?.get(chapter).copied()
    }

    /// Map a tree position to its flat reading index.
    pub fn flat_index(&self, position: Position) -> Option<usize> {
        let chapter = self.chapter(position.volume, position.chapter)?;
        if position.paragraph >= chapter.paragraphs.len() {
            return None;
        }
        self.chapter_start_index(position.volume, position.chapter)
            .map(|start| start + position.paragraph)
    }

    /// Map a flat reading index back to a tree position.
    pub fn position_at(&self, flat: usize) -> Option<Position> {
        self.flat_positions.get(flat).copied()
    }

    pub fn iter_positions(&self) -> impl Iterator<Item = Position> + '_ {
        self.flat_positions.iter().copied()
    }

    /// Every paragraph in reading order, paired with its position.
    pub fn iter_paragraphs(&self) -> impl Iterator<Item = (Position, &Paragraph)> + '_ {
        self.flat_positions.iter().copied().map(move |position| {
            (
                position,
                &self.volumes[position.volume].chapters[position.chapter].paragraphs
                    [position.paragraph],
            )
        })
    }

    pub fn first_position(&self) -> Option<Position> {
        self.flat_positions.first().copied()
    }

    pub fn last_position(&self) -> Option<Position> {
        self.flat_positions.last().copied()
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::types::{Chapter, Group, Paragraph, Volume};

    pub(crate) fn paragraph(volume: &str, chapter: &str, index: usize, text: &str) -> Paragraph {
        Paragraph {
            id: format!("{volume}-{chapter}-p{index:04}"),
            text: text.to_string(),
            translation: Some(format!("译文{index}")),
            annotations: Vec::new(),
            glossary: Vec::new(),
        }
    }

    /// Minimal 國別體 book: two groups, three volumes, one chapter each.
    pub(crate) fn toy_book() -> BookData {
        let book = Book {
            schema_version: 1,
            id: "toy".into(),
            title: "玩具".into(),
            title_simp: None,
            compiler: None,
            era: None,
            form: "國別體".into(),
            sources: Vec::new(),
            groups: vec![
                Group {
                    id: "a".into(),
                    title: "甲".into(),
                    title_simp: None,
                    volumes: vec!["a-1".into(), "a-2".into()],
                },
                Group {
                    id: "b".into(),
                    title: "乙".into(),
                    title_simp: None,
                    volumes: vec!["b-1".into()],
                },
            ],
            volume_order: vec!["a-1".into(), "a-2".into(), "b-1".into()],
        };
        let volume = |id: &str, group: &str, texts: &[&str]| Volume {
            schema_version: 1,
            id: id.into(),
            book_id: "toy".into(),
            group_id: group.into(),
            title: id.into(),
            title_simp: None,
            index: 0,
            chapters: vec![Chapter {
                id: format!("{id}-01"),
                title: format!("{id} 首章"),
                paragraphs: texts
                    .iter()
                    .enumerate()
                    .map(|(index, text)| paragraph(id, "01", index + 1, text))
                    .collect(),
            }],
        };
        BookData::from_parts(
            book,
            vec![
                volume("a-1", "a", &["秦伐魏", "衛鞅入秦"]),
                volume("a-2", "a", &["蘇秦說齊"]),
                volume("b-1", "b", &["楚人有兩妻"]),
            ],
        )
        .expect("toy book is valid")
    }

    /// 國別體 book with multi-chapter volumes and one empty chapter, used to
    /// exercise chapter/volume boundary navigation.
    pub(crate) fn nav_book() -> BookData {
        fn chapter(volume: &str, number: usize, texts: &[&str]) -> Chapter {
            let label = format!("{number:02}");
            Chapter {
                id: format!("{volume}-{label}"),
                title: format!("{volume}-{label} 章"),
                paragraphs: texts
                    .iter()
                    .enumerate()
                    .map(|(index, text)| paragraph(volume, &label, index + 1, text))
                    .collect(),
            }
        }

        fn volume(id: &str, group: &str, chapters: Vec<Chapter>) -> Volume {
            Volume {
                schema_version: 1,
                id: id.into(),
                book_id: "toy".into(),
                group_id: group.into(),
                title: id.into(),
                title_simp: None,
                index: 0,
                chapters,
            }
        }

        let book = Book {
            schema_version: 1,
            id: "toy".into(),
            title: "玩具".into(),
            title_simp: None,
            compiler: None,
            era: None,
            form: "國別體".into(),
            sources: Vec::new(),
            groups: vec![
                Group {
                    id: "a".into(),
                    title: "甲".into(),
                    title_simp: None,
                    volumes: vec!["a-1".into(), "a-2".into()],
                },
                Group {
                    id: "b".into(),
                    title: "乙".into(),
                    title_simp: None,
                    volumes: vec!["b-1".into()],
                },
            ],
            volume_order: vec!["a-1".into(), "a-2".into(), "b-1".into()],
        };

        BookData::from_parts(
            book,
            vec![
                volume(
                    "a-1",
                    "a",
                    vec![
                        chapter("a-1", 1, &["秦伐魏", "衛鞅入秦"]),
                        chapter("a-1", 2, &["蘇秦說齊"]),
                    ],
                ),
                volume(
                    "a-2",
                    "a",
                    vec![chapter("a-2", 1, &["楚人有兩妻"]), chapter("a-2", 2, &[])],
                ),
                volume(
                    "b-1",
                    "b",
                    vec![chapter("b-1", 1, &["魏王欲攻邯鄲", "趙且伐燕"])],
                ),
            ],
        )
        .expect("nav book is valid")
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::toy_book;
    use super::*;

    #[test]
    fn traversal_follows_volume_order_and_counts_match() {
        let data = toy_book();
        assert_eq!(data.volume_count(), 3);
        assert_eq!(data.group_count(), 2);
        assert_eq!(data.chapter_count(), 3);
        assert_eq!(data.paragraph_count(), 4);

        let ids: Vec<&str> = data
            .volumes()
            .iter()
            .map(|volume| volume.id.as_str())
            .collect();
        assert_eq!(ids, vec!["a-1", "a-2", "b-1"]);

        let group_a: Vec<&str> = data
            .group_volumes("a")
            .map(|volume| volume.id.as_str())
            .collect();
        assert_eq!(group_a, vec!["a-1", "a-2"]);
        assert!(data.group_volumes("nope").next().is_none());
    }

    #[test]
    fn flat_order_and_position_mapping_round_trip() {
        let data = toy_book();
        let flat: Vec<String> = data
            .iter_paragraphs()
            .map(|(position, paragraph)| format!("{position:?}:{}", paragraph.id))
            .collect();
        assert_eq!(flat.len(), 4);
        assert!(flat[0].contains("a-1-01-p0001"));
        assert!(flat[3].contains("b-1-01-p0001"));

        for position in data.iter_positions() {
            let index = data.flat_index(position).expect("position maps to flat");
            assert_eq!(data.position_at(index), Some(position));
        }
        assert_eq!(data.first_position(), Some(Position::new(0, 0, 0)));
        assert_eq!(data.last_position(), Some(Position::new(2, 0, 0)));
    }

    #[test]
    fn from_parts_reorders_volumes_to_volume_order() {
        let data = toy_book();
        // Feeding the same volumes shuffled still yields volumeOrder order.
        let mut shuffled: Vec<Volume> = data.volumes().to_vec();
        shuffled.reverse();
        let rebuilt = BookData::from_parts(data.book().clone(), shuffled).unwrap();
        let ids: Vec<&str> = rebuilt.volumes().iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, vec!["a-1", "a-2", "b-1"]);
    }

    #[test]
    fn duplicate_paragraph_id_is_rejected_with_both_locations() {
        let data = toy_book();
        let mut volumes = data.volumes().to_vec();
        let mut duplicate = volumes[0].chapters[0].paragraphs[0].clone();
        duplicate.text = "另一个文本".into();
        volumes[0].chapters[0].paragraphs.push(duplicate);

        let err = BookData::from_parts(data.book().clone(), volumes).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("段 id"), "{message}");
        assert!(message.contains("重复"), "{message}");
    }

    #[test]
    fn empty_text_is_rejected_with_location() {
        let data = toy_book();
        let mut volumes = data.volumes().to_vec();
        volumes[1].chapters[0].paragraphs[0].text = "   ".into();
        let err = BookData::from_parts(data.book().clone(), volumes).unwrap_err();
        assert!(matches!(err, LoadError::EmptyParagraphText { .. }));
        assert!(err.to_string().contains("a-2"), "{err}");
    }

    #[test]
    fn group_reference_to_unknown_volume_is_rejected() {
        let data = toy_book();
        let mut book = data.book().clone();
        book.groups[0].volumes.push("ghost-9".into());
        let err = BookData::from_parts(book, data.volumes().to_vec()).unwrap_err();
        assert!(matches!(
            err,
            LoadError::GroupReferencesUnknownVolume { .. }
        ));
        assert!(err.to_string().contains("ghost-9"));
    }

    #[test]
    fn volume_group_id_must_exist() {
        let data = toy_book();
        let mut volumes = data.volumes().to_vec();
        volumes[0].group_id = "ghost".into();
        let err = BookData::from_parts(data.book().clone(), volumes).unwrap_err();
        assert!(matches!(
            err,
            LoadError::VolumeReferencesUnknownGroup { .. }
        ));
    }

    #[test]
    fn volume_listed_by_two_groups_is_rejected() {
        let data = toy_book();
        let mut book = data.book().clone();
        // a-2 already belongs to group a; listing it under b must fail.
        book.groups[1].volumes.push("a-2".into());
        let err = BookData::from_parts(book, data.volumes().to_vec()).unwrap_err();
        match &err {
            LoadError::VolumeInMultipleGroups { volume_id, groups } => {
                assert_eq!(volume_id, "a-2");
                assert_eq!(groups, &vec!["a".to_string(), "b".to_string()]);
            }
            other => panic!("unexpected error: {other}"),
        }
        let message = err.to_string();
        assert!(message.contains("a-2"), "{message}");
        assert!(message.contains("a, b"), "{message}");
    }

    #[test]
    fn duplicate_listing_inside_one_group_is_deduped_not_rejected() {
        let data = toy_book();
        let mut book = data.book().clone();
        book.groups[0].volumes.push("a-1".into());
        let deduped = BookData::from_parts(book, data.volumes().to_vec())
            .expect("the same group listing a volume twice is deduped");
        let ids: Vec<&str> = deduped
            .group_volumes("a")
            .map(|volume| volume.id.as_str())
            .collect();
        assert_eq!(ids, vec!["a-1", "a-2"]);
    }

    #[test]
    fn chapter_ids_are_globally_unique() {
        let data = toy_book();
        let mut volumes = data.volumes().to_vec();
        volumes[1].chapters[0].id = volumes[0].chapters[0].id.clone();
        let err = BookData::from_parts(data.book().clone(), volumes).unwrap_err();
        assert!(matches!(err, LoadError::DuplicateChapterId { .. }));
    }

    #[test]
    fn volume_order_and_volume_set_must_match_both_ways() {
        let data = toy_book();

        // Declared in volumeOrder but absent from the volume list.
        let mut book = data.book().clone();
        book.volume_order.push("ghost".into());
        let err = BookData::from_parts(book, data.volumes().to_vec()).unwrap_err();
        match err {
            LoadError::VolumeOrderMismatch { missing, extra } => {
                assert_eq!(missing, vec!["ghost".to_string()]);
                assert!(extra.is_empty());
            }
            other => panic!("unexpected error: {other}"),
        }

        // Present in the volume list but not declared in volumeOrder.
        let mut book = data.book().clone();
        book.volume_order.pop();
        let err = BookData::from_parts(book, data.volumes().to_vec()).unwrap_err();
        match err {
            LoadError::VolumeOrderMismatch { missing, extra } => {
                assert!(missing.is_empty());
                assert_eq!(extra, vec!["b-1".to_string()]);
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}
