//! Room for one period's events in a fixed table, kept so that no ending
//! is ever lost (step sequencer refactor A2; docs/dev/design/playback.md,
//! "Endings always get through").
//!
//! A table takes a period's events in the order they come, and a [`Room`]
//! says which go in. There are three kinds:
//!
//! - **Starts and values**: a note-on, up to [`NOTE_STARTS`] a period; a
//!   parameter lock, a move's value, pressure or a program change, up to
//!   [`VALUES`] a period, apart, so a crowd of locks never keeps a note
//!   out (nor notes a lock). Past that the moment is too busy: something
//!   new must give, so it is refused and counted ([`Room::refused`]).
//!   Nothing hangs from it.
//! - **Note endings**: a note-off. The room keeps, by stream, track and
//!   note, how many notes it let start that have not ended ("owed"). A
//!   note-on gets in only while the owed notes and the note-offs passed
//!   this period are fewer than [`NOTE_ENDS`]. A note-off that ends an
//!   owed note only moves one from owed to passed, so the two together
//!   never go over [`NOTE_ENDS`]: every owed note-off fits. A note-off the
//!   room doesn't owe (its note-on was refused here) is left out
//!   ([`Room::unowed`]); nothing sounds for it further on, so nothing
//!   hangs.
//! - **Unlocks and all-offs.** The player holds at most [`MAX_LOCKED`]
//!   parameters, and asks before it holds a new one ([`Room::hold`],
//!   through [`crate::Sink::hold`]); the room grants [`MAX_LOCKED`] new
//!   holds a period. So a period ends at most what was held as it began
//!   and what it held since: [`LOCK_ENDS`] unlocks. An all-off (a panic)
//!   has [`ALL_OFFS`] places of its own: the player sends one for each
//!   STOP, and the transport takes at most [`ALL_OFFS`] commands a period.
//!
//! So a table of [`ROOM`] places never runs out of room for an ending.
//! [`Room::lost`] counts an ending that found none anyway: a bug, which
//! must stay 0 (tests fail on it). Starts, values and endings share one
//! table in the order they came, so an ending never leaves its frame.
//!
//! A **stream** is one source whose notes pair up on their own: the
//! player's (0), and the arps' (1: the live pads, latched arps and clips
//! through an arp), whose engines end their own notes. A note held by a
//! clip and by a live pad on the same track and note is two notes here,
//! one a stream, as it is further on. An all-off ends its own stream's
//! notes; the arps' engines send their own note-offs for theirs.
//!
//! A [`Table`] is such a table with its room: the arps' (`TrackArps`)
//! and the time engine's period table in the app.
//!
//! No allocation after [`Room::new`] and [`Table::new`] (control thread),
//! no locks, no panics.

use mpfree_seq::MAX_TRACKS;
use mpfree_transport::transport::MAX_COMMANDS_PER_PERIOD;

use crate::player::{Event, Out, Sink, ALL, MAX_LOCKED};

/// Note-ons a period takes.
pub const NOTE_STARTS: usize = 512;
/// Values a period takes: locks, move values, pressure, program changes.
pub const VALUES: usize = 512;
/// Notes a room owes (started, not ended) and note-offs it passed this
/// period, together: the most note-offs a period can carry. Twice the
/// note-ons, so a period can end every note it let in and start as many
/// again (a crowd of notes retriggered on one frame).
pub const NOTE_ENDS: usize = 2 * NOTE_STARTS;
/// Unlocks a period can carry: what was held as it began and what it
/// held since, at most [`MAX_LOCKED`] each.
pub const LOCK_ENDS: usize = 2 * MAX_LOCKED;
/// All-offs (panics) a period can carry: one for each STOP, and the
/// transport takes at most this many commands a period.
pub const ALL_OFFS: usize = MAX_COMMANDS_PER_PERIOD;
/// Places in a table a [`Room`] keeps: enough for every event it lets in.
pub const ROOM: usize = NOTE_STARTS + VALUES + NOTE_ENDS + LOCK_ENDS + ALL_OFFS;
/// Streams a room keeps apart: the player's notes and the arps'.
pub const STREAMS: usize = 2;
/// The player's stream.
pub const PLAYER: usize = 0;
/// The arps' stream: the live pads, latched arps and clips through an arp.
pub const ARPS: usize = 1;

/// Which events of a period go in a fixed table; see the module notes.
#[derive(Clone, Debug)]
pub struct Room {
    /// Notes started here and not ended, by stream and track, then note.
    owed: Box<[[u8; 128]; STREAMS * MAX_TRACKS]>,
    n_owed: usize,
    /// This period's note-ons, values, note-offs, new holds, unlocks
    /// and all-offs passed.
    starts: usize,
    values: usize,
    ended: usize,
    holds: usize,
    unlocks: usize,
    all_offs: usize,
    /// Starts and values refused for room, so far.
    pub refused: u64,
    /// Note-offs left out because their note-on was refused here.
    pub unowed: u64,
    /// Endings with no room: a bug (should stay 0).
    pub lost: u64,
}

impl Default for Room {
    fn default() -> Self {
        Room::new()
    }
}

impl Room {
    /// An empty room. Allocates (32 KB): make it on the control thread.
    pub fn new() -> Room {
        Room {
            owed: Box::new([[0; 128]; STREAMS * MAX_TRACKS]),
            n_owed: 0,
            starts: 0,
            values: 0,
            ended: 0,
            holds: 0,
            unlocks: 0,
            all_offs: 0,
            refused: 0,
            unowed: 0,
            lost: 0,
        }
    }

    /// A new period: its starts, endings and holds count from 0. What is
    /// owed stays owed.
    pub fn period(&mut self) {
        self.starts = 0;
        self.values = 0;
        self.ended = 0;
        self.holds = 0;
        self.unlocks = 0;
        self.all_offs = 0;
    }

    /// Notes started here that have not ended.
    pub fn owed(&self) -> usize {
        self.n_owed
    }

    /// Whether `stream` started `note` on `track` here and has not ended it.
    pub fn owes(&self, stream: usize, track: usize, note: u8) -> bool {
        key(stream.min(STREAMS - 1), track, note).is_some_and(|(row, n)| self.owed[row][n] > 0)
    }

    /// Whether a new hold (a parameter the player starts to hold, which
    /// will need its unlock) fits this period, with room for its lock
    /// now. Asked before the lock is sent; a yes keeps its place.
    pub fn hold(&mut self) -> bool {
        if self.holds < MAX_LOCKED && self.values < VALUES {
            self.holds += 1;
            true
        } else {
            self.refused += 1;
            false
        }
    }

    /// Whether `o`, from `stream` ([`PLAYER`] or [`ARPS`]), goes in the
    /// table. Counts what it leaves out.
    #[inline]
    pub fn admit(&mut self, o: &Out, stream: usize) -> bool {
        let stream = stream.min(STREAMS - 1);
        let track = usize::from(o.track);
        match o.event {
            Event::NoteOn { note, .. } => {
                let room = self.starts < NOTE_STARTS && self.n_owed + self.ended < NOTE_ENDS;
                match key(stream, track, note) {
                    Some((row, n)) if room && self.owed[row][n] < u8::MAX => {
                        self.owed[row][n] += 1;
                        self.n_owed += 1;
                        self.starts += 1;
                        true
                    }
                    _ => {
                        self.refused += 1;
                        false
                    }
                }
            }
            Event::NoteOff { note } => match key(stream, track, note) {
                Some((row, n)) if self.owed[row][n] > 0 => {
                    self.owed[row][n] -= 1;
                    self.n_owed -= 1;
                    self.ended += 1;
                    true
                }
                _ => {
                    self.unowed += 1;
                    false
                }
            },
            Event::Unlock { .. } => {
                if self.unlocks < LOCK_ENDS {
                    self.unlocks += 1;
                    true
                } else {
                    self.lost += 1;
                    false
                }
            }
            Event::AllOff => {
                if o.track == ALL {
                    for t in 0..MAX_TRACKS {
                        self.clear(stream, t);
                    }
                } else if track < MAX_TRACKS {
                    self.clear(stream, track);
                }
                if self.all_offs < ALL_OFFS {
                    self.all_offs += 1;
                    true
                } else {
                    self.lost += 1;
                    false
                }
            }
            Event::Lock { .. }
            | Event::Lane { .. }
            | Event::Pressure { .. }
            | Event::Program { .. } => {
                if self.values < VALUES {
                    self.values += 1;
                    true
                } else {
                    self.refused += 1;
                    false
                }
            }
        }
    }

    /// Every note `stream` owes on `track` ends (an all-off).
    fn clear(&mut self, stream: usize, track: usize) {
        for n in self.owed[stream * MAX_TRACKS + track].iter_mut() {
            self.n_owed -= usize::from(*n);
            *n = 0;
        }
    }
}

/// One period's events in a fixed table of [`ROOM`] places, let in by its
/// [`Room`], each with a mark (the arps': whether an arp played it). The
/// player's [`Sink`]: [`Sink::send`] puts in the player's stream, and
/// [`Sink::hold`] asks the room.
pub struct Table {
    out: Box<[Out]>,
    marks: Box<[bool]>,
    n: usize,
    /// What this period lets in.
    pub room: Room,
    /// Events with no place left: a bug (the room lets in no more than
    /// the table holds), which must stay 0.
    pub lost: u64,
}

impl Default for Table {
    fn default() -> Self {
        Table::new()
    }
}

impl Table {
    /// An empty table. Allocates (about 120 KB): make it on the control
    /// thread.
    pub fn new() -> Table {
        let blank = Out {
            offset: 0,
            frame: 0,
            track: 0,
            dest: 0,
            event: Event::AllOff,
        };
        Table {
            out: vec![blank; ROOM].into_boxed_slice(),
            marks: vec![false; ROOM].into_boxed_slice(),
            n: 0,
            room: Room::new(),
            lost: 0,
        }
    }

    /// `o` from `stream`, if the room lets it in (marked when `stream` is
    /// [`ARPS`]). Whether it went in.
    #[inline]
    pub fn put(&mut self, o: Out, stream: usize) -> bool {
        self.room.admit(&o, stream) && self.store(o, stream == ARPS)
    }

    /// `o`, which a room has let in already (another table's, passing its
    /// period on), with its mark. Whether it found a place.
    #[inline]
    pub fn store(&mut self, o: Out, mark: bool) -> bool {
        if self.n == ROOM {
            self.lost += 1;
            return false;
        }
        self.out[self.n] = o;
        self.marks[self.n] = mark;
        self.n += 1;
        true
    }

    /// Events in the table.
    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// The events, in the order they went in (or as sorted).
    pub fn events(&self) -> &[Out] {
        &self.out[..self.n]
    }

    /// Event `i` and its mark.
    pub fn get(&self, i: usize) -> Option<(Out, bool)> {
        (i < self.n).then(|| (self.out[i], self.marks[i]))
    }

    /// Sorts the events from `from` on into the ones before by offset,
    /// stably (events on the same offset keep their order, the earlier
    /// ones first): merges what went in from several sources, each in
    /// order already. No allocation.
    pub fn merge_from(&mut self, from: usize) {
        let (out, marks) = (&mut self.out[..self.n], &mut self.marks[..self.n]);
        for i in from.max(1)..out.len() {
            let mut j = i;
            while j > 0 && out[j - 1].offset > out[j].offset {
                out.swap(j - 1, j);
                marks.swap(j - 1, j);
                j -= 1;
            }
        }
    }

    /// A new period: the table empties, and its room counts from 0.
    pub fn clear(&mut self) {
        self.n = 0;
        self.room.period();
    }

    /// Starts and values left out for room ([`Room::refused`]).
    pub fn refused(&self) -> u64 {
        self.room.refused
    }

    /// Endings that found no room or place: a bug, which must stay 0.
    pub fn lost(&self) -> u64 {
        self.room.lost + self.lost
    }
}

impl Sink for Table {
    fn send(&mut self, out: Out) {
        self.put(out, PLAYER);
    }

    fn hold(&mut self) -> bool {
        self.room.hold()
    }
}

/// Where a room keeps what `stream` owes for `note` on `track`: its row
/// and place. None for a track past the last.
#[inline]
fn key(stream: usize, track: usize, note: u8) -> Option<(usize, usize)> {
    (track < MAX_TRACKS).then_some((stream * MAX_TRACKS + track, usize::from(note & 127)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_param::{Owner, ParamAddr};

    fn out(track: u16, event: Event) -> Out {
        Out {
            offset: 0,
            frame: 0,
            track,
            dest: 0,
            event,
        }
    }

    fn on(track: u16, note: u8) -> Out {
        out(
            track,
            Event::NoteOn {
                note,
                velocity: 100,
            },
        )
    }

    fn off(track: u16, note: u8) -> Out {
        out(track, Event::NoteOff { note })
    }

    /// A period too busy for its starts refuses the new ones, and still
    /// takes the note-off of every note it let start; a note-off whose
    /// note-on it refused is left out.
    #[test]
    fn a_full_period_refuses_starts_and_takes_every_owed_ending() {
        let mut r = Room::new();
        let mut passed = 0;
        // 600 notes on 8 tracks: the first 512 start.
        for i in 0..600u16 {
            passed += usize::from(r.admit(&on(i % 8, (i / 8) as u8), PLAYER));
        }
        assert_eq!(
            (passed, r.refused, r.owed()),
            (NOTE_STARTS, 88, NOTE_STARTS)
        );
        // Every one ends in the same period: the 512 owed get in, the 88
        // refused are left out.
        let mut ended = 0;
        for i in 0..600u16 {
            ended += usize::from(r.admit(&off(i % 8, (i / 8) as u8), PLAYER));
        }
        assert_eq!((ended, r.unowed, r.lost, r.owed()), (NOTE_STARTS, 88, 0, 0));
        // Still that period: no room for another start (its starts are
        // used), the next period has it.
        assert!(!r.admit(&on(0, 1), PLAYER));
        r.period();
        assert!(r.admit(&on(0, 1), PLAYER));
    }

    /// A crowd of notes retriggered on one frame: every note ends and as
    /// many start again in the same period.
    #[test]
    fn a_period_can_end_every_note_and_start_as_many_again() {
        let mut r = Room::new();
        let note = |n: usize| ((n / 128) as u16, (n % 128) as u8);
        for n in 0..NOTE_STARTS {
            let (t, k) = note(n);
            assert!(r.admit(&on(t, k), PLAYER));
        }
        r.period();
        for n in 0..NOTE_STARTS {
            let (t, k) = note(n);
            assert!(
                r.admit(&off(t, k), PLAYER) && r.admit(&on(t, k), PLAYER),
                "note {n}"
            );
        }
        assert_eq!((r.owed(), r.refused, r.lost), (NOTE_STARTS, 0, 0));
    }

    /// The streams keep their notes apart: a live note-off never ends a
    /// clip's note on the same track and note, so neither is left out.
    #[test]
    fn the_player_and_the_arps_owe_their_own_notes() {
        let mut r = Room::new();
        assert!(r.admit(&on(2, 60), PLAYER));
        assert!(!r.admit(&off(2, 60), ARPS), "the arps never started it");
        assert!(r.admit(&on(2, 60), ARPS));
        assert!(r.admit(&off(2, 60), ARPS));
        assert!(r.admit(&off(2, 60), PLAYER));
        assert_eq!((r.owed(), r.unowed), (0, 1));
        // An all-off ends its own stream's notes.
        assert!(r.admit(&on(1, 1), PLAYER) && r.admit(&on(1, 2), ARPS));
        assert!(r.admit(&out(ALL, Event::AllOff), PLAYER));
        assert_eq!(r.owed(), 1, "the arp's note ends with its own note-off");
        assert!(r.admit(&off(1, 2), ARPS));
        assert!(!r.admit(&off(1, 1), PLAYER), "already ended by the all-off");
    }

    /// A model player over many periods: notes that start and end on 16
    /// tracks, ratchets, locks it asks to hold and unlocks, stops that end
    /// everything, and a panic, far past the room's limits. No ending is
    /// ever lost, a period never takes more than the table holds, and
    /// every note that got in gets its note-off.
    #[test]
    fn no_ending_is_ever_lost_whatever_the_period_holds() {
        let mut r = Room::new();
        let mut seed = 0x2545_f491u32;
        let mut rand = move |n: u32| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed % n
        };
        // What the player sounds and holds (its own tables), and what got
        // through the room.
        let mut sounding: Vec<(u16, u8)> = Vec::new();
        let mut held: Vec<u16> = Vec::new();
        let mut through = vec![[0u32; 128]; MAX_TRACKS];
        let addr = |p: u16| ParamAddr::new(Owner::Inst(0), p);
        for period in 0..400 {
            r.period();
            let mut stored = 0;
            let mut put = |r: &mut Room, o: Out, through: &mut Vec<[u32; 128]>| {
                if r.admit(&o, PLAYER) {
                    stored += 1;
                    match o.event {
                        Event::NoteOn { note, .. } => {
                            through[usize::from(o.track)][note as usize] += 1
                        }
                        Event::NoteOff { note } => {
                            through[usize::from(o.track)][note as usize] -= 1
                        }
                        _ => {}
                    }
                }
            };
            let busy = period % 50 > 40;
            for _ in 0..if busy { 3000 } else { 40 } {
                match rand(10) {
                    0..=3 if sounding.len() < MAX_LOCKED => {
                        let (t, n) = (rand(16) as u16, rand(128) as u8);
                        // One note a track and pitch: a second ends the first.
                        if let Some(i) = sounding.iter().position(|&s| s == (t, n)) {
                            sounding.swap_remove(i);
                            put(&mut r, off(t, n), &mut through);
                        }
                        sounding.push((t, n));
                        put(&mut r, on(t, n), &mut through);
                    }
                    4..=5 if !sounding.is_empty() => {
                        let (t, n) = sounding.swap_remove(rand(sounding.len() as u32) as usize);
                        put(&mut r, off(t, n), &mut through);
                    }
                    6 if held.len() < MAX_LOCKED => {
                        let p = rand(1000) as u16;
                        if !held.contains(&p) && r.hold() {
                            held.push(p);
                            put(
                                &mut r,
                                out(
                                    0,
                                    Event::Lock {
                                        addr: addr(p),
                                        value: 1.0,
                                    },
                                ),
                                &mut through,
                            );
                        }
                    }
                    7 if !held.is_empty() => {
                        let p = held.swap_remove(rand(held.len() as u32) as usize);
                        put(
                            &mut r,
                            out(0, Event::Unlock { addr: addr(p) }),
                            &mut through,
                        );
                    }
                    8 => put(
                        &mut r,
                        out(0, Event::Pressure { note: 1, value: 9 }),
                        &mut through,
                    ),
                    _ => {}
                }
            }
            // A stop ends every note and every hold, now and then on the
            // busiest periods.
            if busy && period % 7 == 0 {
                for (t, n) in sounding.drain(..) {
                    put(&mut r, off(t, n), &mut through);
                }
                for p in held.drain(..) {
                    put(
                        &mut r,
                        out(0, Event::Unlock { addr: addr(p) }),
                        &mut through,
                    );
                }
            }
            if period == 333 {
                sounding.clear();
                held.clear();
                put(&mut r, out(ALL, Event::AllOff), &mut through);
                through.iter_mut().for_each(|t| *t = [0; 128]);
            }
            assert!(stored <= ROOM, "period {period}: {stored} events");
            assert_eq!(r.lost, 0, "period {period}");
        }
        // The rest end: every note that got through ends exactly once.
        r.period();
        for (t, n) in sounding.drain(..) {
            if r.admit(&off(t, n), PLAYER) {
                through[usize::from(t)][n as usize] -= 1;
            }
        }
        assert!(through.iter().all(|t| t.iter().all(|&c| c == 0)));
        assert_eq!((r.owed(), r.lost), (0, 0));
        assert!(r.refused > 0 && r.unowed > 0, "the busy periods gave way");
    }
}
