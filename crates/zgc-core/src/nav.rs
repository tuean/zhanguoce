//! Reading-order navigation: ordered traversal plus paragraph / chapter / volume
//! prev-next with correct boundary hopping.
//!
//! Boundary semantics (deliberately documented, because readers differ):
//! * `next_paragraph` / `prev_paragraph` cross chapter and volume boundaries and
//!   land on the adjacent paragraph (previous chapter → its **last** paragraph).
//! * `next_chapter` / `prev_chapter` cross volume boundaries and align to the
//!   target chapter's **first** paragraph.
//! * `next_volume` / `prev_volume` align to the target volume's first paragraph.
//!
//! Empty chapters/volumes are skipped instead of producing a dead stop.

use crate::data::{BookData, Position};

impl BookData {
    /// First paragraph of a volume, or `None` when the volume is empty/unknown.
    pub fn volume_start(&self, volume: usize) -> Option<Position> {
        self.first_in_volume(volume)
    }

    /// Next paragraph in reading order; crosses chapters and volumes.
    pub fn next_paragraph(&self, position: Position) -> Option<Position> {
        let chapter = self.chapter(position.volume, position.chapter)?;
        if position.paragraph + 1 < chapter.paragraphs.len() {
            return Some(Position::new(
                position.volume,
                position.chapter,
                position.paragraph + 1,
            ));
        }
        let (volume, chapter) =
            self.next_nonempty_chapter(position.volume, position.chapter.saturating_add(1))?;
        self.first_in_chapter(volume, chapter)
    }

    /// Previous paragraph in reading order; crosses chapters and volumes.
    pub fn prev_paragraph(&self, position: Position) -> Option<Position> {
        let chapter = self.chapter(position.volume, position.chapter)?;
        if position.paragraph > 0 && position.paragraph < chapter.paragraphs.len() {
            return Some(Position::new(
                position.volume,
                position.chapter,
                position.paragraph - 1,
            ));
        }
        let (volume, chapter) = self.prev_nonempty_chapter(position.volume, position.chapter)?;
        self.last_in_chapter(volume, chapter)
    }

    /// First paragraph of the next non-empty chapter (may be in the next volume).
    pub fn next_chapter(&self, position: Position) -> Option<Position> {
        let (volume, chapter) =
            self.next_nonempty_chapter(position.volume, position.chapter.saturating_add(1))?;
        self.first_in_chapter(volume, chapter)
    }

    /// First paragraph of the previous non-empty chapter (may be in a earlier volume).
    pub fn prev_chapter(&self, position: Position) -> Option<Position> {
        let (volume, chapter) = self.prev_nonempty_chapter(position.volume, position.chapter)?;
        self.first_in_chapter(volume, chapter)
    }

    /// First paragraph of the next non-empty volume.
    pub fn next_volume(&self, position: Position) -> Option<Position> {
        (position.volume.saturating_add(1)..self.volume_count())
            .find_map(|volume| self.first_in_volume(volume))
    }

    /// First paragraph of the previous non-empty volume.
    pub fn prev_volume(&self, position: Position) -> Option<Position> {
        (0..position.volume)
            .rev()
            .find_map(|volume| self.first_in_volume(volume))
    }

    fn first_in_volume(&self, volume: usize) -> Option<Position> {
        let chapters = &self.volume(volume)?.chapters;
        chapters
            .iter()
            .position(|chapter| !chapter.paragraphs.is_empty())
            .map(|chapter| Position::new(volume, chapter, 0))
    }

    fn first_in_chapter(&self, volume: usize, chapter: usize) -> Option<Position> {
        if self.chapter(volume, chapter)?.paragraphs.is_empty() {
            return None;
        }
        Some(Position::new(volume, chapter, 0))
    }

    fn last_in_chapter(&self, volume: usize, chapter: usize) -> Option<Position> {
        let paragraphs = self.chapter(volume, chapter)?.paragraphs.len();
        paragraphs
            .checked_sub(1)
            .map(|last| Position::new(volume, chapter, last))
    }

    /// First non-empty chapter at or after `(volume, chapter)`, in reading order.
    fn next_nonempty_chapter(&self, volume: usize, chapter: usize) -> Option<(usize, usize)> {
        for current_volume in volume..self.volume_count() {
            let chapters = &self.volumes()[current_volume].chapters;
            let start = if current_volume == volume { chapter } else { 0 };
            if start >= chapters.len() {
                continue;
            }
            if let Some(found) =
                (start..chapters.len()).find(|&index| !chapters[index].paragraphs.is_empty())
            {
                return Some((current_volume, found));
            }
        }
        None
    }

    /// Last non-empty chapter strictly before `(volume, chapter)`, in reading order.
    fn prev_nonempty_chapter(&self, volume: usize, chapter: usize) -> Option<(usize, usize)> {
        let mut current_volume = volume;
        let mut upper = chapter;
        loop {
            let chapters = &self.volumes()[current_volume].chapters;
            let upper_now = upper.min(chapters.len());
            if let Some(found) = (0..upper_now)
                .rev()
                .find(|&index| !chapters[index].paragraphs.is_empty())
            {
                return Some((current_volume, found));
            }
            if current_volume == 0 {
                return None;
            }
            current_volume -= 1;
            upper = usize::MAX;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::data::test_support::nav_book;
    use crate::data::Position;

    #[test]
    fn paragraph_step_crosses_chapter_within_volume() {
        let data = nav_book();
        let last_of_chapter_one = Position::new(0, 0, 1);
        let next = data.next_paragraph(last_of_chapter_one).unwrap();
        assert_eq!(next, Position::new(0, 1, 0));

        let back = data.prev_paragraph(Position::new(0, 1, 0)).unwrap();
        assert_eq!(
            back, last_of_chapter_one,
            "preceding chapter → last paragraph"
        );
    }

    #[test]
    fn paragraph_step_crosses_volume_and_skips_empty_chapter() {
        let data = nav_book();
        // a-1 last paragraph (chapter 2 has one paragraph) → a-2 first paragraph.
        let last_of_a1 = Position::new(0, 1, 0);
        let next = data.next_paragraph(last_of_a1).unwrap();
        assert_eq!(next, Position::new(1, 0, 0));
        assert_eq!(
            data.paragraph(next).unwrap().id,
            "a-2-01-p0001",
            "must land on the next volume's first paragraph"
        );

        // a-2 chapter 2 is empty → next paragraph must be b-1's first.
        let last_of_a2 = Position::new(1, 0, 0);
        let next = data.next_paragraph(last_of_a2).unwrap();
        assert_eq!(next, Position::new(2, 0, 0));

        // And backwards over the same boundaries.
        let back = data.prev_paragraph(Position::new(2, 0, 0)).unwrap();
        assert_eq!(back, last_of_a2);
        let back = data.prev_paragraph(Position::new(1, 0, 0)).unwrap();
        assert_eq!(back, last_of_a1);
    }

    #[test]
    fn chapter_step_aligns_to_first_paragraph_and_crosses_volume() {
        let data = nav_book();
        let next = data.next_chapter(Position::new(0, 0, 1)).unwrap();
        assert_eq!(next, Position::new(0, 1, 0));

        // a-1 last chapter → a-2 first chapter (crosses volume).
        let next = data.next_chapter(Position::new(0, 1, 0)).unwrap();
        assert_eq!(next, Position::new(1, 0, 0));

        // Previous chapter aligns to its first paragraph, not its last.
        let prev = data.prev_chapter(Position::new(0, 1, 0)).unwrap();
        assert_eq!(prev, Position::new(0, 0, 0));
        assert_eq!(data.paragraph(prev).unwrap().id, "a-1-01-p0001");
    }

    #[test]
    fn volume_step_aligns_to_first_paragraph() {
        let data = nav_book();
        let next = data.next_volume(Position::new(0, 0, 0)).unwrap();
        assert_eq!(next, Position::new(1, 0, 0));
        let prev = data.prev_volume(Position::new(1, 0, 0)).unwrap();
        assert_eq!(prev, Position::new(0, 0, 0));
        assert_eq!(data.prev_volume(Position::new(0, 0, 0)), None);
        let last_volume_start = data.volume_start(2).unwrap();
        assert_eq!(data.next_volume(last_volume_start), None);
    }

    #[test]
    fn navigation_bounds_are_none_at_the_ends() {
        let data = nav_book();
        let first = data.first_position().unwrap();
        let last = data.last_position().unwrap();
        assert_eq!(data.prev_paragraph(first), None);
        assert_eq!(data.prev_chapter(first), None);
        assert_eq!(data.next_paragraph(last), None);
        assert_eq!(data.next_chapter(last), None);
    }

    #[test]
    fn traversal_visits_every_paragraph_once_in_reading_order() {
        let data = nav_book();
        let mut position = data.first_position();
        let mut seen = Vec::new();
        while let Some(current) = position {
            seen.push(data.paragraph(current).unwrap().id.clone());
            position = data.next_paragraph(current);
        }
        let expected: Vec<String> = data
            .iter_paragraphs()
            .map(|(_, paragraph)| paragraph.id.clone())
            .collect();
        assert_eq!(seen, expected);
        assert_eq!(seen.len(), data.paragraph_count());
    }
}
