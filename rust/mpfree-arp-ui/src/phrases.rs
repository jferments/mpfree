//! The phrases a track picks from, as the screens read them: the app's
//! one list with whose each phrase is, and the list as the Phrase tab's
//! boards show it, a shelf at a time with the phrases that suit the key
//! first (docs/dev/design/arpeggiator.md, "Phrases").
//!
//! Nothing here draws. The arp chip and the quick band name the phrase
//! from it; the Phrase tab's own screen is built once the owner has
//! answered its boards, on [`PhraseList::rows`].

use mpfree_arp::{Library, Phrase, PhraseKind, Scale};

/// Whose a phrase in the list is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// One of MPFree's own. (It plays the owner's notes when the owner's
    /// library names it again: it keeps its place and its shelf.)
    Mpfree,
    /// The owner's, kept in the store.
    Owner,
    /// One only the open project has: it was saved with a phrase the list
    /// doesn't have.
    Project,
}

/// The app's list of phrases, in its three runs: MPFree's own, the
/// owner's, then the open project's. A track's pick is a place in
/// `phrases`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PhraseList {
    pub phrases: Vec<(String, Phrase)>,
    /// MPFree's own are the first `built`.
    pub built: usize,
    /// The owner's go from there up to `own`; the rest are the project's.
    pub own: usize,
}

/// A shelf of the list: MPFree's phrases by kind, then one for the
/// owner's, with the open project's own after them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shelf {
    Kind(PhraseKind),
    Own,
}

impl Shelf {
    pub const ALL: [Shelf; 5] = [
        Shelf::Kind(PhraseKind::Bass),
        Shelf::Kind(PhraseKind::Riff),
        Shelf::Kind(PhraseKind::Chords),
        Shelf::Kind(PhraseKind::Line),
        Shelf::Own,
    ];

    /// The name the list shows.
    pub const fn label(self) -> &'static str {
        match self {
            Shelf::Kind(k) => k.label(),
            Shelf::Own => "Yours",
        }
    }
}

/// A row of a shelf.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    /// The phrase's place in the list: what the app picks by.
    pub index: usize,
    pub origin: Origin,
    /// Written in the other kind of scale than the key (a major phrase in
    /// a minor key): it comes after the ones that suit the key, dimmed,
    /// and says the scale it was written in.
    pub other_scale: bool,
}

impl PhraseList {
    /// MPFree's own phrases alone: the list before the app has sent its
    /// own, and the sample screens'.
    pub fn built_in() -> PhraseList {
        let phrases: Vec<(String, Phrase)> = Library::built_in()
            .phrases()
            .map(|(n, p)| (n.to_string(), *p))
            .collect();
        PhraseList {
            built: phrases.len(),
            own: phrases.len(),
            phrases,
        }
    }

    /// The name of phrase `i`; none past the end.
    pub fn name(&self, i: usize) -> &str {
        self.phrases.get(i).map_or("", |(n, _)| n.as_str())
    }

    /// Whose phrase `i` is.
    pub fn origin(&self, i: usize) -> Origin {
        if i < self.built {
            Origin::Mpfree
        } else if i < self.own {
            Origin::Owner
        } else {
            Origin::Project
        }
    }

    /// The shelf phrase `i` is on: its kind's for one of MPFree's, the
    /// owner's shelf for any other.
    pub fn shelf_of(&self, i: usize) -> Shelf {
        match self.phrases.get(i) {
            Some((_, p)) if i < self.built => Shelf::Kind(p.kind),
            _ => Shelf::Own,
        }
    }

    /// A shelf's rows in the order shown: the phrases that suit `key`
    /// (the scale a phrase follows In key now; `Written::suits`) in the
    /// list's order, then the ones written in the other kind of scale.
    /// With no key, or one of no kind, it is the list's order.
    pub fn rows(&self, shelf: Shelf, key: Option<Scale>) -> Vec<Row> {
        let mut rows: Vec<Row> = (0..self.phrases.len())
            .filter(|&i| self.shelf_of(i) == shelf)
            .map(|index| Row {
                index,
                origin: self.origin(index),
                other_scale: !self.phrases[index].1.written.suits(key),
            })
            .collect();
        // Stable: each half keeps the list's order.
        rows.sort_by_key(|r| r.other_scale);
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_arp::scale::{MAJOR, MINOR};
    use mpfree_arp::Written;

    fn names(list: &PhraseList, rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|r| list.name(r.index).to_string())
            .collect()
    }

    #[test]
    fn mpfrees_phrases_are_sixteen_to_a_shelf_and_the_owners_shelf_starts_empty() {
        let list = PhraseList::built_in();
        assert_eq!((list.phrases.len(), list.built, list.own), (64, 64, 64));
        for shelf in Shelf::ALL {
            let rows = list.rows(shelf, None);
            let want = if shelf == Shelf::Own { 0 } else { 16 };
            assert_eq!(rows.len(), want, "{}", shelf.label());
            assert!(rows
                .iter()
                .all(|r| r.origin == Origin::Mpfree && !r.other_scale));
            // No key: the list's own order.
            assert!(rows.windows(2).all(|w| w[0].index < w[1].index));
        }
        let labels: Vec<&str> = Shelf::ALL.iter().map(|s| s.label()).collect();
        assert_eq!(labels, ["Bass", "Riff", "Chords", "Arp lines", "Yours"]);
        assert_eq!(list.name(0), "Offbeat pump");
        assert_eq!(list.name(64), "");
    }

    /// In a minor key the phrases written in major come last, each half
    /// in the list's order; in a major key the minor ones do; and every
    /// phrase is on its shelf once.
    #[test]
    fn the_phrases_that_suit_the_key_come_first() {
        let list = PhraseList::built_in();
        for (mask, other) in [(MINOR, Written::Major), (MAJOR, Written::Minor)] {
            let key = Some(Scale::new(9, mask));
            let mut seen = 0;
            for shelf in Shelf::ALL {
                let rows = list.rows(shelf, key);
                seen += rows.len();
                let split = rows
                    .iter()
                    .position(|r| r.other_scale)
                    .unwrap_or(rows.len());
                let (suit, rest) = rows.split_at(split);
                assert!(rest.iter().all(|r| r.other_scale), "{}", shelf.label());
                for half in [suit, rest] {
                    assert!(half.windows(2).all(|w| w[0].index < w[1].index));
                }
                for r in &rows {
                    let written = list.phrases[r.index].1.written;
                    assert_eq!(r.other_scale, written == other, "{}", list.name(r.index));
                }
            }
            assert_eq!(seen, 64);
        }
        // The riffs in A minor: the first is the list's first riff, and
        // one written in major is after every one that suits.
        let riffs = list.rows(Shelf::Kind(PhraseKind::Riff), Some(Scale::new(9, MINOR)));
        let first_riff = list
            .phrases
            .iter()
            .position(|(_, p)| p.kind == PhraseKind::Riff && p.written != Written::Major)
            .unwrap();
        assert_eq!(riffs[0].index, first_riff);
        assert!(
            riffs.iter().any(|r| r.other_scale),
            "{:?}",
            names(&list, &riffs)
        );
    }

    /// The owner's and the open project's phrases share the last shelf,
    /// whatever their kind, the owner's first; one of MPFree's that plays
    /// the owner's notes stays on its kind's shelf.
    #[test]
    fn the_owners_and_the_projects_phrases_are_on_the_last_shelf() {
        let mut list = PhraseList::built_in();
        let bass = Phrase::parse("bass minor : 1 . 1 . 5- . 1 . 1 . 1 . 5- . b7- .").unwrap();
        let riff = Phrase::parse("riff major : 1 3 5 3 1 3 5 3 1 3 5 3 1 3 5 3").unwrap();
        list.phrases.push(("Verse bass".into(), bass));
        list.own += 1;
        list.phrases.push(("My riff".into(), riff));
        let rows = list.rows(Shelf::Own, None);
        assert_eq!(names(&list, &rows), ["Verse bass", "My riff"]);
        assert_eq!(
            rows.iter().map(|r| r.origin).collect::<Vec<_>>(),
            [Origin::Owner, Origin::Project]
        );
        assert_eq!(list.shelf_of(64), Shelf::Own);
        assert_eq!(list.shelf_of(0), Shelf::Kind(list.phrases[0].1.kind));
        // A major key: the minor bass goes after the major riff.
        let rows = list.rows(Shelf::Own, Some(Scale::new(0, MAJOR)));
        assert_eq!(names(&list, &rows), ["My riff", "Verse bass"]);
        assert!(rows[1].other_scale && !rows[0].other_scale);
        // MPFree's shelves still hold 16 each.
        assert_eq!(list.rows(Shelf::Kind(PhraseKind::Bass), None).len(), 16);
    }
}
