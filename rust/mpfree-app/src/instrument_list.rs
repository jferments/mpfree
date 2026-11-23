//! The project's list of instruments: step 2 of the instrument redesign
//! ([instruments.md](../../../docs/dev/design/instruments.md)).
//!
//! Instruments make the sound; tracks record and play back to them, and
//! the pads play them. To point at an instrument, a track (and later a
//! knob's owner, a script or a MIDI port) needs a name for it that stays
//! put when tracks are added, moved or taken away: an [`InstrumentId`]. It
//! is not a track number, a mixer channel or a place in a list. A project
//! gives each new instrument the next number, keeps that counter in its
//! file, and never gives a number out again once its instrument is gone,
//! so something that named an old instrument can't reach a new one by
//! mistake.
//!
//! - **Step 2a:** the shared synth (the pads' synth, which tracks on the
//!   Synth kit play) is instrument 1 in every project. Its sound stays in
//!   the synth part.
//! - **Step 2b:** each track's own instrument (`instruments.rs`) has an id
//!   too, from the moment it goes into a track that had none until it is
//!   taken out. A new sound or preset loaded into it keeps the id; taken
//!   out, its id is never given again. Its sound stays in the instruments
//!   part, which still names it by its track: a track has one at most, so
//!   the list says which id is which track's ([`Kind::Own`]).
//! - Step 2c gives tracks lasting ids.
//! - **Step 6:** an instrument no track has, which the pads play (picked
//!   in the list of synths), is in a free place (`instruments::FREE`):
//!   it gets the next id when it goes into an empty place, keeps it while
//!   the place holds an instrument, and loses it for good once taken out
//!   ([`Kind::Free`], [`InstrumentList::follow_free`]). Its sound is in
//!   the instruments part by its place.
//! - **Kits** (the sampler refactor's step 14): every kit is an instrument
//!   in its own right, so each kit place of the sampler (the factory
//!   kits, Grains and the empty kits kept for New kit, fixed while MPFree
//!   runs) has an id from the moment the project has it
//!   ([`Kind::Kit`], [`InstrumentList::follow_kits`]). A kit loaded into
//!   its place keeps the place's id, as a new sound in a track's own
//!   instrument keeps its. Its sounds are the sampler part's, by place.
//!
//! The list says what an instrument is, not how it sounds now or whether
//! it is playing: the running engines are `synth::Synth` and the track
//! host's slots, in the audio loop.
//!
//! **Threads.** A list is made and read on the screen thread
//! (`project::opening`), moved into the audio loop whole as a project
//! opens, and the loop's old one goes back to be freed there. Its storage
//! is fixed (an id for each track), so the loop gives and takes ids
//! ([`InstrumentList::follow`]) without allocating; only its text, for a
//! save, allocates, as every part's does.

use std::fmt;

/// The instrument list part's first line.
/// 2 since kits are in the list (the sampler refactor's step 14); a
/// part of format 1 is refused, as its project is.
pub const FORMAT: &str = "mpfree-instrument-list 2";

/// The tracks that can have their own instrument.
pub const TRACKS: usize = crate::instruments::TRACKS;

/// The free places (step 6).
pub const FREE: usize = crate::instruments::FREE;

/// The sampler's kit places (step 14): the most kits the engine holds.
pub const KITS: usize = mpfree_sampler::engine::MAX_KITS;

// A bit for each track in `follow`.
const _: () = assert!(TRACKS <= u128::BITS as usize);

/// An instrument's lasting number in its project, from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InstrumentId(u32);

impl InstrumentId {
    /// The shared synth: instrument 1 in every project.
    pub const SHARED_SYNTH: InstrumentId = InstrumentId(1);

    /// Instrument `n` (None for 0: numbers start at 1).
    pub fn from_number(n: u32) -> Option<InstrumentId> {
        (n >= 1).then_some(InstrumentId(n))
    }

    /// Its number, as it is saved (a MIDI route names it by this).
    pub fn number(self) -> u32 {
        self.0
    }
}

impl fmt::Display for InstrumentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// What an instrument of the list is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The pads' synth, whichever engine the Synth tab plays. Its sound is
    /// the project's synth part, and it sounds on the Synth strip.
    SharedSynth,
    /// Track `track`'s own instrument (from 0): its sound is that track's
    /// block of the instruments part, and it sounds on the strip its slot
    /// feeds (`Instruments::strip`), the track's channel today.
    Own { track: usize },
    /// The instrument in free place `place` (from 0): one no track has,
    /// which the pads or tracks play by a send (step 6). Its sound is that
    /// place's block of the instruments part, and it sounds on the place's
    /// own strip (`mixing::free_channel`).
    Free { place: usize },
    /// The kit in the sampler's kit place `kit` (from 0): its sounds are
    /// the sampler part's kit of that place, and it sounds on the pad
    /// channels or the channel of the first track that plays it
    /// (`TrackKits::homes`).
    Kit { kit: usize },
}

impl Kind {
    /// Its holder in the instrument host (`instruments::Instruments`): a
    /// track, or a free place (`instruments::free`). None for the shared
    /// synth and a kit, which the host doesn't hold.
    pub fn holder(self) -> Option<usize> {
        match self {
            Kind::SharedSynth | Kind::Kit { .. } => None,
            Kind::Own { track } => Some(track),
            Kind::Free { place } => Some(crate::instruments::free(place)),
        }
    }
}

/// The project's instruments, each with its lasting id, and the number
/// the next one will get. The shared synth is always instrument 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstrumentList {
    /// Track `t`'s own instrument's id (0: it has none).
    own: [u32; TRACKS],
    /// Free place `n`'s instrument's id (0: it holds none).
    free: [u32; FREE],
    /// Kit place `k`'s id (0: the sampler hasn't that place).
    kits: [u32; KITS],
    /// The id the next instrument gets: past every id the project ever
    /// gave, so none is given twice.
    next: u32,
}

/// Every kit place's id in a project's list of instruments, by place
/// ([`InstrumentList::kit_ids`]): what the Sample page names its kits by
/// in its commands (sampler refactor phase 3, step 14), as the audio
/// loop last said. It holds no heap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KitIds([u32; KITS]);

impl KitIds {
    /// The id of the kit in place `kit`, if the list gives that place one.
    pub fn of(&self, kit: usize) -> Option<InstrumentId> {
        self.0.get(kit).and_then(|&o| InstrumentId::from_number(o))
    }

    /// The place, among the first `kits`, of the kit that answers to `id`:
    /// None for an id that isn't a kit's, or a kit's past `kits`.
    pub fn kit(&self, id: InstrumentId, kits: usize) -> Option<usize> {
        self.0[..kits.min(KITS)].iter().position(|&o| o == id.0)
    }

    /// How many places from `kits` on have an id.
    pub fn listed_from(&self, kits: usize) -> usize {
        self.0
            .get(kits..)
            .map_or(0, |o| o.iter().filter(|&&o| o != 0).count())
    }
}

/// Every place's id as a new project's list gives them
/// ([`InstrumentList::follow_kits`] with every place there): what the
/// Sample page goes by until the audio loop says the project's.
impl Default for KitIds {
    fn default() -> KitIds {
        let mut list = InstrumentList::new();
        list.follow_kits(KITS);
        list.kit_ids()
    }
}

impl Default for InstrumentList {
    fn default() -> InstrumentList {
        InstrumentList::new()
    }
}

impl InstrumentList {
    /// A new project's list before its kits get their ids
    /// ([`InstrumentList::follow_kits`]): the shared synth as instrument 1.
    pub fn new() -> InstrumentList {
        InstrumentList {
            own: [0; TRACKS],
            free: [0; FREE],
            kits: [0; KITS],
            next: 2,
        }
    }

    /// What instrument `id` is, or None if the list hasn't got it.
    pub fn kind(&self, id: InstrumentId) -> Option<Kind> {
        if id == InstrumentId::SHARED_SYNTH {
            return Some(Kind::SharedSynth);
        }
        if let Some(track) = self.own.iter().position(|&o| o == id.0) {
            return Some(Kind::Own { track });
        }
        if let Some(place) = self.free.iter().position(|&o| o == id.0) {
            return Some(Kind::Free { place });
        }
        let kit = self.kits.iter().position(|&o| o == id.0)?;
        Some(Kind::Kit { kit })
    }

    /// The id of the kit in kit place `kit`, if the sampler has that place.
    pub fn of_kit(&self, kit: usize) -> Option<InstrumentId> {
        self.kits
            .get(kit)
            .and_then(|&o| InstrumentId::from_number(o))
    }

    /// Every kit place's id, by place: what the Sample page names its
    /// kits by in its commands.
    pub fn kit_ids(&self) -> KitIds {
        KitIds(self.kits)
    }

    /// The id of the instrument in free place `place`, if it holds one.
    pub fn free_of(&self, place: usize) -> Option<InstrumentId> {
        self.free
            .get(place)
            .and_then(|&o| InstrumentId::from_number(o))
    }

    /// The id of the instrument holder `holder` has in the instrument
    /// host: a track's own, or a free place's.
    pub fn of_holder(&self, holder: usize) -> Option<InstrumentId> {
        if holder < TRACKS {
            self.own_of(holder)
        } else {
            self.free_of(holder - TRACKS)
        }
    }

    /// The id of track `track`'s own instrument, if it has one.
    pub fn own_of(&self, track: usize) -> Option<InstrumentId> {
        self.own
            .get(track)
            .and_then(|&o| InstrumentId::from_number(o))
    }

    /// The tracks with an own instrument now (`held`, a bit each:
    /// `Instruments::held_tracks`): one that has just got its first gets
    /// the next id, one whose instrument was taken out loses its id for
    /// good, and one that still has one keeps its id, whatever sound went
    /// into it. True if any changed. Never allocates.
    pub fn follow(&mut self, held: u128) -> bool {
        let mut changed = false;
        for (t, o) in self.own.iter_mut().enumerate() {
            match (held >> t & 1 == 1, *o) {
                (true, 0) => {
                    *o = self.next;
                    self.next = self.next.saturating_add(1);
                    changed = true;
                }
                (false, id) if id != 0 => {
                    *o = 0;
                    changed = true;
                }
                _ => {}
            }
        }
        changed
    }

    /// The free places that hold an instrument now (`held`, a bit each:
    /// `Instruments::held_free`), as [`InstrumentList::follow`] does for
    /// the tracks: a place that has just got one gives it the next id, one
    /// whose instrument was taken out loses its id for good, and one that
    /// still holds one keeps its id. True if any changed. Never
    /// allocates.
    pub fn follow_free(&mut self, held: u16) -> bool {
        let mut changed = false;
        for (n, o) in self.free.iter_mut().enumerate() {
            match (held >> n & 1 == 1, *o) {
                (true, 0) => {
                    *o = self.next;
                    self.next = self.next.saturating_add(1);
                    changed = true;
                }
                (false, id) if id != 0 => {
                    *o = 0;
                    changed = true;
                }
                _ => {}
            }
        }
        changed
    }

    /// The sampler has `kits` kit places (they never move while MPFree
    /// runs): each of the first `kits` without an id gets the next one, in
    /// order, and a place past them loses its id for good. True if any
    /// changed. Never allocates.
    pub fn follow_kits(&mut self, kits: usize) -> bool {
        let mut changed = false;
        for (k, o) in self.kits.iter_mut().enumerate() {
            match (k < kits, *o) {
                (true, 0) => {
                    *o = self.next;
                    self.next = self.next.saturating_add(1);
                    changed = true;
                }
                (false, id) if id != 0 => {
                    *o = 0;
                    changed = true;
                }
                _ => {}
            }
        }
        changed
    }

    /// The list as its part's text, by id:
    ///
    /// ```text
    /// mpfree-instrument-list 2
    /// next 6
    /// instrument 1 shared-synth
    /// instrument 2 kit 1
    /// instrument 3 kit 2
    /// instrument 4 own track 3
    /// instrument 5 free 1
    /// ```
    ///
    /// Tracks, free places and kits are numbered from 1, as the screen
    /// shows them.
    pub fn text(&self) -> String {
        use std::fmt::Write as _;
        let mut s = format!(
            "{FORMAT}\nnext {}\ninstrument {} shared-synth\n",
            self.next,
            InstrumentId::SHARED_SYNTH
        );
        // By id, each with what it is.
        let own = (0..TRACKS).map(|t| (self.own[t], "own track", t));
        let free = (0..FREE).map(|n| (self.free[n], "free", n));
        let kits = (0..KITS).map(|k| (self.kits[k], "kit", k));
        let mut all: Vec<(u32, &str, usize)> = own
            .chain(free)
            .chain(kits)
            .filter(|&(id, ..)| id != 0)
            .collect();
        all.sort_unstable();
        for (id, what, at) in all {
            let _ = writeln!(s, "instrument {id} {what} {}", at + 1);
        }
        s
    }

    /// Reads a list part ([`InstrumentList::text`]). Refuses a part with
    /// a line it can't read, an id or a track given twice, an id not below
    /// `next`, or without the shared synth as instrument 1 (the opener
    /// then says so and gives the new project's list).
    pub fn parse(text: &str) -> Result<InstrumentList, String> {
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
        if lines.next() != Some(FORMAT) {
            return Err(format!("not a {FORMAT} part"));
        }
        let mut list = InstrumentList::new();
        let (mut next, mut synth, mut ids) = (None, false, Vec::new());
        for line in lines {
            let w: Vec<&str> = line.split_whitespace().collect();
            let id = |n: &str| n.parse().ok().and_then(InstrumentId::from_number);
            let track = |n: &str| {
                n.parse::<usize>()
                    .ok()
                    .filter(|&n| (1..=TRACKS).contains(&n))
            };
            let id = match w.as_slice() {
                ["next", n] if next.is_none() => match n.parse::<u32>() {
                    Ok(n) if n >= 2 => {
                        next = Some(n);
                        continue;
                    }
                    _ => None,
                },
                ["instrument", n, "shared-synth"] if !synth => {
                    synth = true;
                    id(n).filter(|&i| i == InstrumentId::SHARED_SYNTH)
                }
                ["instrument", n, "own", "track", t] => match (id(n), track(t)) {
                    (Some(i), Some(t)) if i != InstrumentId::SHARED_SYNTH => {
                        if list.own[t - 1] != 0 {
                            return Err(format!("track {t} has two instruments of its own"));
                        }
                        list.own[t - 1] = i.0;
                        Some(i)
                    }
                    _ => None,
                },
                ["instrument", n, "free", k] => {
                    let place = k.parse::<usize>().ok().filter(|&k| (1..=FREE).contains(&k));
                    match (id(n), place) {
                        (Some(i), Some(k)) if i != InstrumentId::SHARED_SYNTH => {
                            if list.free[k - 1] != 0 {
                                return Err(format!("free place {k} holds two instruments"));
                            }
                            list.free[k - 1] = i.0;
                            Some(i)
                        }
                        _ => None,
                    }
                }
                ["instrument", n, "kit", k] => {
                    let place = k.parse::<usize>().ok().filter(|&k| (1..=KITS).contains(&k));
                    match (id(n), place) {
                        (Some(i), Some(k)) if i != InstrumentId::SHARED_SYNTH => {
                            if list.kits[k - 1] != 0 {
                                return Err(format!("kit {k} has two ids"));
                            }
                            list.kits[k - 1] = i.0;
                            Some(i)
                        }
                        _ => None,
                    }
                }
                _ => None,
            };
            let Some(id) = id else {
                return Err(format!("can't read \"{line}\""));
            };
            if ids.contains(&id) {
                return Err(format!("instrument {id} is listed twice"));
            }
            ids.push(id);
        }
        let Some(next) = next else {
            return Err("no \"next\" line".to_string());
        };
        if !synth {
            return Err("the shared synth isn't instrument 1".to_string());
        }
        if let Some(id) = ids.iter().find(|i| i.0 >= next) {
            return Err(format!("instrument {id} isn't below next ({next})"));
        }
        list.next = next;
        Ok(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A new project's list is the shared synth as instrument 1, and its
    /// text reads back to the same list.
    #[test]
    fn a_new_list_is_the_shared_synth_as_instrument_1() {
        let l = InstrumentList::new();
        assert_eq!(l.kind(InstrumentId::SHARED_SYNTH), Some(Kind::SharedSynth));
        assert_eq!(l.kind(InstrumentId(2)), None);
        assert_eq!(l.own_of(0), None);
        assert_eq!(
            l.text(),
            "mpfree-instrument-list 2\nnext 2\ninstrument 1 shared-synth\n"
        );
        assert_eq!(InstrumentList::parse(&l.text()), Ok(l));
        assert_eq!(InstrumentId::from_number(0), None);
        assert_eq!(
            InstrumentId::from_number(1),
            Some(InstrumentId::SHARED_SYNTH)
        );
    }

    /// Step 2b: a track's own instrument gets the next id when it goes
    /// into a track that had none, keeps it while the track has one (a new
    /// sound in its place is the same instrument), and loses it for good
    /// when taken out: a new one gets a new id, never an old one.
    #[test]
    fn a_tracks_own_instrument_keeps_its_id_and_ids_are_never_given_twice() {
        let mut l = InstrumentList::new();
        assert!(!l.follow(0));
        assert!(l.follow(1 << 2 | 1 << 4));
        assert_eq!(
            (l.own_of(2), l.own_of(4)),
            (Some(InstrumentId(2)), Some(InstrumentId(3)))
        );
        assert_eq!(l.kind(InstrumentId(3)), Some(Kind::Own { track: 4 }));
        assert!(
            !l.follow(1 << 2 | 1 << 4),
            "the same tracks: nothing changes"
        );
        assert!(l.follow(1 << 4), "track 3's taken out");
        assert_eq!((l.own_of(2), l.kind(InstrumentId(2))), (None, None));
        assert!(l.follow(1 << 2 | 1 << 4), "and a new one put in");
        assert_eq!(l.own_of(2), Some(InstrumentId(4)), "a new id, not 2 again");
        assert_eq!(l.own_of(4), Some(InstrumentId(3)));
        assert!(l.follow(1 << 2 | 1 << 4 | 1 << 127));
        assert_eq!(l.own_of(127), Some(InstrumentId(5)));
        assert_eq!(
            l.text(),
            "mpfree-instrument-list 2\nnext 6\ninstrument 1 shared-synth\n\
             instrument 3 own track 5\ninstrument 4 own track 3\ninstrument 5 own track 128\n"
        );
        assert_eq!(InstrumentList::parse(&l.text()), Ok(l));
    }

    /// Step 6: a free place's instrument gets the next id, keeps it while
    /// the place holds one, and loses it for good once taken out, with
    /// the tracks' own in one numbering; its text reads back.
    #[test]
    fn a_free_places_instrument_has_an_id_like_a_tracks_own() {
        let mut l = InstrumentList::new();
        assert!(!l.follow_free(0));
        assert!(l.follow(1 << 3));
        assert!(l.follow_free(1 << 1));
        assert_eq!(l.free_of(1), Some(InstrumentId(3)));
        assert_eq!(l.kind(InstrumentId(3)), Some(Kind::Free { place: 1 }));
        assert_eq!(
            Kind::Free { place: 1 }.holder(),
            Some(crate::instruments::free(1))
        );
        assert_eq!(l.of_holder(crate::instruments::free(1)), l.free_of(1));
        assert_eq!(l.of_holder(3), Some(InstrumentId(2)));
        assert!(!l.follow_free(1 << 1), "the same places: nothing changes");
        assert_eq!(
            l.text(),
            "mpfree-instrument-list 2\nnext 4\ninstrument 1 shared-synth\n\
             instrument 2 own track 4\ninstrument 3 free 2\n"
        );
        assert_eq!(InstrumentList::parse(&l.text()), Ok(l.clone()));
        assert!(l.follow_free(1 << 0), "place 2 emptied, place 1 filled");
        assert_eq!((l.free_of(1), l.kind(InstrumentId(3))), (None, None));
        assert_eq!(l.free_of(0), Some(InstrumentId(4)), "a new id");
        let refused = |body: &str| InstrumentList::parse(&format!("{FORMAT}\n{body}")).unwrap_err();
        let head = "next 9\ninstrument 1 shared-synth\n";
        assert!(refused(&format!("{head}instrument 2 free 0\n")).contains("can't read"));
        assert!(refused(&format!("{head}instrument 2 free 17\n")).contains("can't read"));
        assert!(refused(&format!("{head}instrument 1 free 1\n")).contains("can't read"));
        assert!(
            refused(&format!("{head}instrument 2 free 1\ninstrument 3 free 1\n"))
                .contains("two instruments")
        );
        assert!(refused(&format!(
            "{head}instrument 2 free 1\ninstrument 2 own track 1\n"
        ))
        .contains("twice"));
    }

    /// Step 14: every kit place gets an id, in order, with the tracks'
    /// own and the free places' in one numbering; a kit keeps its id, a
    /// place the sampler no longer has loses it for good, and the text
    /// reads back.
    #[test]
    fn every_kit_place_has_an_id() {
        let mut l = InstrumentList::new();
        assert!(l.follow_kits(3));
        assert_eq!(
            (l.of_kit(0), l.of_kit(2), l.of_kit(3)),
            (Some(InstrumentId(2)), Some(InstrumentId(4)), None)
        );
        assert_eq!(l.kind(InstrumentId(3)), Some(Kind::Kit { kit: 1 }));
        assert_eq!(Kind::Kit { kit: 1 }.holder(), None);
        assert!(!l.follow_kits(3), "the same places: nothing changes");
        assert!(l.follow(1 << 0));
        assert_eq!(l.own_of(0), Some(InstrumentId(5)));
        let ids = l.kit_ids();
        assert_eq!(
            (0..4).map(|k| ids.of(k)).collect::<Vec<_>>(),
            [
                Some(InstrumentId(2)),
                Some(InstrumentId(3)),
                Some(InstrumentId(4)),
                None
            ]
        );
        assert_eq!(
            (ids.kit(InstrumentId(4), 3), ids.kit(InstrumentId(4), 2)),
            (Some(2), None)
        );
        assert_eq!(
            ids.kit(InstrumentId(5), KITS),
            None,
            "track 1's own is no kit"
        );
        assert_eq!(ids.kit(InstrumentId::SHARED_SYNTH, KITS), None);
        assert_eq!(
            (
                ids.listed_from(1),
                ids.listed_from(3),
                ids.listed_from(KITS)
            ),
            (2, 0, 0)
        );
        assert_eq!(
            KitIds::default().of(KITS - 1),
            InstrumentId::from_number(KITS as u32 + 1)
        );
        assert_eq!(
            l.text(),
            "mpfree-instrument-list 2\nnext 6\ninstrument 1 shared-synth\n\
             instrument 2 kit 1\ninstrument 3 kit 2\ninstrument 4 kit 3\ninstrument 5 own track 1\n"
        );
        assert_eq!(InstrumentList::parse(&l.text()), Ok(l.clone()));
        assert!(l.follow_kits(2), "a build with fewer kits");
        assert_eq!((l.of_kit(2), l.kind(InstrumentId(4))), (None, None));
        assert!(l.follow_kits(4), "and more again: new ids, never 4");
        assert_eq!(
            (l.of_kit(2), l.of_kit(3)),
            (Some(InstrumentId(6)), Some(InstrumentId(7)))
        );
        let refused = |body: &str| InstrumentList::parse(&format!("{FORMAT}\n{body}")).unwrap_err();
        let head = "next 9\ninstrument 1 shared-synth\n";
        assert!(refused(&format!("{head}instrument 2 kit 0\n")).contains("can't read"));
        assert!(refused(&format!("{head}instrument 2 kit 129\n")).contains("can't read"));
        assert!(refused(&format!("{head}instrument 1 kit 1\n")).contains("can't read"));
        assert!(
            refused(&format!("{head}instrument 2 kit 1\ninstrument 3 kit 1\n")).contains("two ids")
        );
        assert!(
            refused(&format!("{head}instrument 2 kit 1\ninstrument 2 free 1\n")).contains("twice")
        );
    }

    /// The counter is kept: a list whose project once gave out higher ids
    /// comes back with the same next number, so none is given twice.
    #[test]
    fn the_next_id_is_kept_through_the_text() {
        let text = "mpfree-instrument-list 2\nnext 7\ninstrument 1 shared-synth\n";
        let mut l = InstrumentList::parse(text).expect("reads");
        assert_eq!(l.text(), text);
        assert!(l.follow(1));
        assert_eq!(l.own_of(0), Some(InstrumentId(7)));
    }

    /// A part that doesn't hold together is refused, with why.
    #[test]
    fn a_list_that_does_not_hold_together_is_refused() {
        let refused = |body: &str| InstrumentList::parse(&format!("{FORMAT}\n{body}")).unwrap_err();
        let synth = "instrument 1 shared-synth\n";
        assert!(InstrumentList::parse("mpfree-instrument-list 1\nnext 2\n").is_err());
        assert!(InstrumentList::parse("mpfree-instrument-list 3\nnext 2\n").is_err());
        assert!(refused(synth).contains("next"));
        assert!(refused(&format!("next 2\nnext 3\n{synth}")).contains("can't read"));
        assert!(refused(&format!("next 1\n{synth}")).contains("can't read"));
        assert!(refused("next 3\n").contains("shared synth"));
        assert!(refused("next 3\ninstrument 2 shared-synth\n").contains("can't read"));
        assert!(refused("next 2\ninstrument 0 shared-synth\n").contains("can't read"));
        assert!(refused("next 2\ninstrument 1 kazoo\n").contains("can't read"));
        assert!(refused(&format!("next 3\n{synth}{synth}")).contains("can't read"));
        assert!(
            refused(&format!("next 2\n{synth}instrument 2 own track 1\n")).contains("below next")
        );
        assert!(
            refused(&format!("next 9\n{synth}instrument 1 own track 1\n")).contains("can't read")
        );
        assert!(
            refused(&format!("next 9\n{synth}instrument 2 own track 0\n")).contains("can't read")
        );
        assert!(
            refused(&format!("next 9\n{synth}instrument 2 own track 129\n")).contains("can't read")
        );
        assert!(refused(&format!(
            "next 9\n{synth}instrument 2 own track 3\ninstrument 3 own track 3\n"
        ))
        .contains("two instruments"));
        assert!(refused(&format!(
            "next 9\n{synth}instrument 2 own track 3\ninstrument 2 own track 4\n"
        ))
        .contains("twice"));
        assert!(refused(&format!("next 2\n{synth}hello\n")).contains("can't read"));
    }
}
