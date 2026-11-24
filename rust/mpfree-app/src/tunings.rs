//! What each instrument plays in (step T5 of the tunings plan,
//! [tunings.md](../../../docs/dev/design/tunings.md)): the song's tuning,
//! and the tunings of the instruments that have one of their own, by
//! their lasting id in the project's list ([`InstrumentId`]). An
//! instrument without one plays the song's ("Song's", the default), so a
//! change to the song's tuning moves every instrument on Song's at once
//! and none with its own.
//!
//! - **A tuning is the instrument's,** not the track's: tracks that play
//!   one instrument share its tuning, and a copy (Duplicate) copies it
//!   ([`Tunings::copy`]). A preset never changes it.
//! - **Fixed storage.** At most [`MOST`] instruments have their own, as
//!   many as the project's list holds at once, kept with no heap, so the
//!   audio loop reads it every period ([`Tunings::pick`]).
//! - **It follows the list.** An instrument taken out of the project loses
//!   its tuning with its id ([`Tunings::follow`]); ids are never given
//!   again, so a new instrument starts on Song's.
//! - **Saved** as the project's `tunings` part ([`Tunings::text`],
//!   [`Tunings::parse`]): a line for each instrument with its own,
//!   written only while one has. The song's tuning stays in the synth
//!   part, where it was before instruments had their own.
//!
//! Which tuning a pick plays (a built-in or a tuning file, its root and
//! A4) is `mpfree_tuning::TuningPick`'s; the audio loop turns picks into
//! tunings and hands each instrument its own (`main.rs`,
//! `TuningsHanded`).

use std::fmt::Write as _;

use mpfree_arp::scale::NOTE_NAMES;
use mpfree_tuning::{Builtin, TuningPick, A4_MAX, A4_MIN};

use crate::instrument_list::{InstrumentId, InstrumentList};

/// The tunings part's first line.
pub const FORMAT: &str = "mpfree-tunings 1";

/// The most instruments with a tuning of their own: every instrument the
/// project's list can hold at once (the shared synth, each track's own,
/// the free places', the kits').
pub const MOST: usize = 1
    + crate::instrument_list::TRACKS
    + crate::instrument_list::FREE
    + crate::instrument_list::KITS;

/// A built-in tuning's word in a saved part and the Synth tab's settings
/// (`tuning = just`).
pub fn builtin_key(b: Builtin) -> &'static str {
    match b {
        Builtin::Equal12 => "12-tet",
        Builtin::Just => "just",
        Builtin::Pythagorean => "pythagorean",
        Builtin::Meantone => "meantone",
        Builtin::Equal24 => "24-tet",
    }
}

/// The built-in tuning a word names ([`builtin_key`]).
pub fn builtin_of(word: &str) -> Option<Builtin> {
    Builtin::ALL
        .iter()
        .copied()
        .find(|&b| builtin_key(b) == word)
}

/// A note name ("C#", any case) as 0 to 11.
pub fn note_of(v: &str) -> Option<u8> {
    NOTE_NAMES
        .iter()
        .position(|n| n.eq_ignore_ascii_case(v))
        .map(|i| i as u8)
}

/// The song's tuning and every instrument's own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tunings {
    song: TuningPick,
    /// The instruments with their own: (id, pick), the first `count` in
    /// the order they got it.
    own: [(u32, TuningPick); MOST],
    count: usize,
}

impl Default for Tunings {
    fn default() -> Tunings {
        Tunings::new()
    }
}

impl Tunings {
    /// A new project's: the song in 12-TET at 440 Hz, every instrument on
    /// Song's.
    pub fn new() -> Tunings {
        Tunings {
            song: TuningPick::default(),
            own: [(0, TuningPick::default()); MOST],
            count: 0,
        }
    }

    /// The song's tuning.
    pub fn song(&self) -> TuningPick {
        self.song
    }

    /// Sets the song's tuning; whether it changed.
    pub fn set_song(&mut self, pick: TuningPick) -> bool {
        let changed = self.song != pick;
        self.song = pick;
        changed
    }

    /// Instrument `id`'s own tuning; None: it plays the song's.
    pub fn own(&self, id: InstrumentId) -> Option<TuningPick> {
        self.at(id).map(|i| self.own[i].1)
    }

    /// What instrument `id` plays in: its own, or else the song's. None (an
    /// instrument without an id yet) plays the song's.
    pub fn pick(&self, id: Option<InstrumentId>) -> TuningPick {
        id.and_then(|id| self.own(id)).unwrap_or(self.song)
    }

    /// Gives instrument `id` a tuning of its own, or with None puts it back
    /// on Song's. Returns whether anything changed; false too when
    /// [`MOST`] instruments have their own already (none is taken).
    pub fn set_own(&mut self, id: InstrumentId, pick: Option<TuningPick>) -> bool {
        match (self.at(id), pick) {
            (Some(i), Some(p)) => {
                let changed = self.own[i].1 != p;
                self.own[i].1 = p;
                changed
            }
            (Some(i), None) => {
                self.own.copy_within(i + 1..self.count, i);
                self.count -= 1;
                true
            }
            (None, Some(p)) if self.count < MOST => {
                self.own[self.count] = (id.number(), p);
                self.count += 1;
                true
            }
            (None, _) => false,
        }
    }

    /// Instrument `to` takes `from`'s pick (Duplicate): its own tuning if
    /// `from` has one, else Song's. Whether `to`'s changed.
    pub fn copy(&mut self, from: InstrumentId, to: InstrumentId) -> bool {
        let pick = self.own(from);
        self.set_own(to, pick)
    }

    /// Lets go of the tunings of instruments `list` hasn't got any more;
    /// whether any went. Never allocates.
    pub fn follow(&mut self, list: &InstrumentList) -> bool {
        let before = self.count;
        let mut kept = 0;
        for i in 0..self.count {
            let keep =
                InstrumentId::from_number(self.own[i].0).is_some_and(|id| list.kind(id).is_some());
            if keep {
                self.own[kept] = self.own[i];
                kept += 1;
            }
        }
        self.count = kept;
        kept != before
    }

    /// How many instruments have a tuning of their own.
    pub fn owned(&self) -> usize {
        self.count
    }

    /// The instruments with their own, each with its pick, in the order
    /// they got it.
    pub fn iter_own(&self) -> impl Iterator<Item = (InstrumentId, TuningPick)> + '_ {
        self.own[..self.count]
            .iter()
            .filter_map(|&(n, p)| InstrumentId::from_number(n).map(|id| (id, p)))
    }

    fn at(&self, id: InstrumentId) -> Option<usize> {
        self.own[..self.count]
            .iter()
            .position(|&(n, _)| n == id.number())
    }

    /// The tunings part, `file_name` naming each tuning file by its place
    /// in the list offered, by id:
    ///
    /// ```text
    /// mpfree-tunings 1
    /// instrument 5 root C a4 415 meantone
    /// instrument 7 root D a4 440 file werckmeister 3
    /// ```
    ///
    /// The tuning comes last, so a file's name may have spaces. A pick of
    /// a file `file_name` doesn't know is saved as its built-in tuning.
    /// Just the first line while no instrument has its own: then the
    /// project has no tunings part.
    pub fn text<'a>(&self, file_name: impl Fn(u8) -> Option<&'a str>) -> String {
        let mut s = format!("{FORMAT}\n");
        let mut own: Vec<(u32, TuningPick)> = self.own[..self.count].to_vec();
        own.sort_unstable_by_key(|&(n, _)| n);
        for (n, p) in own {
            let root = NOTE_NAMES[usize::from(p.root % 12)];
            let _ = write!(s, "instrument {n} root {root} a4 {} ", p.a4_hz);
            match p.file.and_then(&file_name) {
                Some(name) => {
                    let _ = writeln!(s, "file {name}");
                }
                None => {
                    let _ = writeln!(s, "{}", builtin_key(p.builtin));
                }
            }
        }
        s
    }

    /// Reads a tunings part ([`Tunings::text`]): the instruments' own, the
    /// song's left as new (12-TET; the synth part has the project's).
    /// `file_of` finds a tuning file by its name in the list offered.
    /// Refuses a part with a line it can't read or an instrument twice
    /// (the opener then refuses the project, as for the list of
    /// instruments). A tuning file this MPFree hasn't got is not refused:
    /// the instrument plays the song's, and a line for the log says so.
    pub fn parse(
        text: &str,
        file_of: impl Fn(&str) -> Option<u8>,
    ) -> Result<(Tunings, Vec<String>), String> {
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
        if lines.next() != Some(FORMAT) {
            return Err(format!("not a {FORMAT} part"));
        }
        let mut t = Tunings::new();
        let mut notes = Vec::new();
        for line in lines {
            let bad = || format!("\"{line}\" can't be read");
            let rest = line.strip_prefix("instrument ").ok_or_else(bad)?;
            let (n, rest) = rest.split_once(' ').ok_or_else(bad)?;
            let id = n
                .parse()
                .ok()
                .and_then(InstrumentId::from_number)
                .ok_or_else(bad)?;
            let (pick, missing) = parse_pick(rest, &file_of).ok_or_else(bad)?;
            if t.at(id).is_some() {
                return Err(format!("instrument {id} has two tunings"));
            }
            match missing {
                Some(name) => notes.push(format!(
                    "tunings: instrument {id}'s tuning file {name} isn't here: it plays the song's"
                )),
                None => {
                    if !t.set_own(id, Some(pick)) {
                        return Err(format!("more than {MOST} instruments have a tuning"));
                    }
                }
            }
        }
        Ok((t, notes))
    }
}

/// `root D a4 432 just` or `root C a4 440 file NAME` as a pick, and the
/// name of a tuning file `file_of` doesn't know (then the pick is the
/// default). None for words that don't read.
fn parse_pick(
    rest: &str,
    file_of: &impl Fn(&str) -> Option<u8>,
) -> Option<(TuningPick, Option<String>)> {
    let rest = rest.strip_prefix("root ")?;
    let (root, rest) = rest.split_once(' ')?;
    let rest = rest.strip_prefix("a4 ")?;
    let (a4, tuning) = rest.split_once(' ')?;
    let root = note_of(root)?;
    let a4_hz = a4
        .parse::<u16>()
        .ok()
        .filter(|a| (A4_MIN..=A4_MAX).contains(a))?;
    let mut pick = TuningPick {
        root,
        a4_hz,
        ..TuningPick::default()
    };
    match tuning.strip_prefix("file ") {
        Some(name) => {
            let name = name.trim();
            if name.is_empty() {
                return None;
            }
            match file_of(name) {
                Some(f) => pick.file = Some(f),
                None => return Some((TuningPick::default(), Some(name.to_string()))),
            }
        }
        None => pick.builtin = builtin_of(tuning.trim())?,
    }
    Some((pick, None))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u32) -> InstrumentId {
        InstrumentId::from_number(n).unwrap()
    }

    fn just_d() -> TuningPick {
        TuningPick {
            builtin: Builtin::Just,
            root: 2,
            ..TuningPick::default()
        }
    }

    fn meantone_415() -> TuningPick {
        TuningPick {
            builtin: Builtin::Meantone,
            a4_hz: 415,
            ..TuningPick::default()
        }
    }

    const FILES: [&str; 2] = ["werckmeister 3", "dorian"];

    fn file_name(f: u8) -> Option<&'static str> {
        FILES.get(usize::from(f)).copied()
    }

    fn file_of(name: &str) -> Option<u8> {
        FILES.iter().position(|&n| n == name).map(|i| i as u8)
    }

    #[test]
    fn every_instrument_plays_the_songs_until_it_has_its_own() {
        let mut t = Tunings::new();
        assert_eq!(t.song(), TuningPick::default());
        assert_eq!(
            t.pick(Some(InstrumentId::SHARED_SYNTH)),
            TuningPick::default()
        );
        assert_eq!(t.pick(None), TuningPick::default());
        assert!(t.set_song(just_d()));
        assert!(!t.set_song(just_d()));
        assert!(t.set_own(id(5), Some(meantone_415())));
        assert!(!t.set_own(id(5), Some(meantone_415())));
        assert_eq!(t.pick(Some(id(5))), meantone_415());
        assert_eq!(t.pick(Some(id(4))), just_d());
        assert_eq!(t.pick(None), just_d());
        // The song's change moves only those on Song's.
        t.set_song(TuningPick {
            a4_hz: 432,
            ..TuningPick::default()
        });
        assert_eq!(t.pick(Some(id(4))).a4_hz, 432);
        assert_eq!(t.pick(Some(id(5))), meantone_415());
        // Back on Song's.
        assert!(t.set_own(id(5), None));
        assert!(!t.set_own(id(5), None));
        assert_eq!(t.pick(Some(id(5))).a4_hz, 432);
        assert_eq!(t.owned(), 0);
    }

    #[test]
    fn a_copy_takes_its_instruments_tuning() {
        let mut t = Tunings::new();
        t.set_song(just_d());
        t.set_own(id(3), Some(meantone_415()));
        assert!(t.copy(id(3), id(9)));
        assert_eq!(t.own(id(9)), Some(meantone_415()));
        // A copy of one on Song's is on Song's too.
        assert!(!t.copy(id(4), id(10)));
        assert_eq!(t.own(id(10)), None);
        t.set_own(id(11), Some(meantone_415()));
        assert!(t.copy(id(4), id(11)));
        assert_eq!(t.own(id(11)), None);
    }

    #[test]
    fn an_instrument_taken_out_loses_its_tuning() {
        let mut list = InstrumentList::new();
        list.follow_kits(2);
        // Tracks 1 and 3 have their own instruments: ids 4 and 5 (kits
        // took 2 and 3).
        list.follow(0b101);
        let (track1, track3) = (list.own_of(0).unwrap(), list.own_of(2).unwrap());
        let kit2 = list.of_kit(1).unwrap();
        let mut t = Tunings::new();
        for i in [track1, track3, kit2, InstrumentId::SHARED_SYNTH] {
            t.set_own(i, Some(meantone_415()));
        }
        assert!(!t.follow(&list));
        assert_eq!(t.owned(), 4);
        // Track 1's goes.
        list.follow(0b100);
        assert!(t.follow(&list));
        assert_eq!(t.own(track1), None);
        assert_eq!(t.own(track3), Some(meantone_415()));
        assert_eq!(t.own(kit2), Some(meantone_415()));
        assert_eq!(t.own(InstrumentId::SHARED_SYNTH), Some(meantone_415()));
        // Track 1's next instrument has a new id, on Song's.
        list.follow(0b101);
        assert_ne!(list.own_of(0), Some(track1));
        assert_eq!(t.pick(list.own_of(0)), TuningPick::default());
    }

    #[test]
    fn as_many_instruments_as_the_list_holds_can_have_their_own() {
        let mut t = Tunings::new();
        for n in 1..=MOST as u32 {
            assert!(t.set_own(id(n), Some(meantone_415())));
        }
        assert!(!t.set_own(id(MOST as u32 + 1), Some(meantone_415())));
        assert_eq!(t.own(id(MOST as u32 + 1)), None);
        assert!(t.set_own(id(7), None));
        assert!(t.set_own(id(MOST as u32 + 1), Some(meantone_415())));
        assert_eq!(t.owned(), MOST);
        assert_eq!(t.own(id(8)), Some(meantone_415()));
    }

    #[test]
    fn the_part_saves_and_reads_back() {
        let mut t = Tunings::new();
        assert_eq!(t.text(file_name), "mpfree-tunings 1\n");
        t.set_song(just_d());
        t.set_own(
            id(7),
            Some(TuningPick {
                file: Some(0),
                root: 2,
                ..TuningPick::default()
            }),
        );
        t.set_own(id(5), Some(meantone_415()));
        t.set_own(
            InstrumentId::SHARED_SYNTH,
            Some(TuningPick {
                builtin: Builtin::Equal24,
                a4_hz: 432,
                ..TuningPick::default()
            }),
        );
        let text = t.text(file_name);
        assert_eq!(
            text,
            "mpfree-tunings 1\n\
             instrument 1 root C a4 432 24-tet\n\
             instrument 5 root C a4 415 meantone\n\
             instrument 7 root D a4 440 file werckmeister 3\n"
        );
        let (back, notes) = Tunings::parse(&text, file_of).unwrap();
        assert!(notes.is_empty());
        // The song's is the synth part's, not this one's.
        assert_eq!(back.song(), TuningPick::default());
        for n in [1, 5, 7, 9] {
            assert_eq!(back.own(id(n)), t.own(id(n)), "instrument {n}");
        }
        let (bare, _) = Tunings::parse("mpfree-tunings 1\n", file_of).unwrap();
        assert_eq!(bare, Tunings::new());
    }

    #[test]
    fn a_tuning_file_thats_gone_is_said_and_plays_the_songs() {
        let text = "mpfree-tunings 1\n\
                    instrument 4 root C a4 440 file gone too\n\
                    instrument 6 root C a4 440 file dorian\n";
        let (t, notes) = Tunings::parse(text, file_of).unwrap();
        assert_eq!(t.own(id(4)), None);
        assert_eq!(t.own(id(6)).and_then(|p| p.file), Some(1));
        assert_eq!(
            notes,
            ["tunings: instrument 4's tuning file gone too isn't here: it plays the song's"]
        );
    }

    #[test]
    fn a_part_that_doesnt_read_is_refused() {
        let part = |lines: &str| Tunings::parse(&format!("{FORMAT}\n{lines}"), file_of);
        assert!(Tunings::parse("mpfree-tunings 2\n", file_of).is_err());
        for bad in [
            "instrument 4 root H a4 440 just",
            "instrument 4 root C a4 500 just",
            "instrument 4 root C a4 440 just-ish",
            "instrument 4 root C 440 just",
            "instrument 4 root C a4 440 file ",
            "instrument 0 root C a4 440 just",
            "instrument x root C a4 440 just",
            "instrument 4",
            "song root C a4 440 just",
            "instrument 4 root C a4 440 just\ninstrument 4 root D a4 440 just",
        ] {
            assert!(part(bad).is_err(), "{bad}");
        }
        // Any case for the root, as the Synth tab's settings.
        let (t, _) = part("instrument 4 root c# a4 440 just").unwrap();
        assert_eq!(t.own(id(4)).map(|p| p.root), Some(1));
    }
}
