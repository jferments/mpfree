//! "This track's strip" in a clip goes where the track's instrument
//! sounds (instrument redesign, after step 4d). A clip names its track's
//! strip without the track (`OnTrack::Strip`), and the player puts the
//! playing track's own strip in (`mpfree_param::track_strip`). A track
//! that plays another track's instrument (step 4a) is heard on that
//! instrument's strip, not its own, so its locks there would move a strip
//! nothing plays through. Here, in the audio half's sink, where the track
//! a lock comes from is known, such a lock goes to the strip its
//! instrument sounds on (the host says which: `Pads::sounds_on`):
//!
//! - **Only "this track's strip".** A lock counts when it names the
//!   playing track's own strip, which is how a clip recorded on that
//!   track names it (`Target::for_track`). Locks on the master, a submix,
//!   a return or another track's strip go on as they came, and so do
//!   those on a strip's effects (`OnTrack::Insert`): an effect's number
//!   is its own strip's, so the same number on the instrument's strip
//!   would be another effect.
//! - **A lock ends where it began.** Each lock is kept with the strip it
//!   went to, and its end goes there, even when the instrument has moved
//!   since. A new value for a lock still held after the instrument moved
//!   (a move's next point, the next locked step) first ends it on the old
//!   strip, then starts it on the new one.
//! - **Two tracks on one strip take turns**, as on a shared instrument's
//!   knobs (step 4b, `locks.rs`): the track with the lowest lasting id
//!   owns the control, and the others' locks are kept, not sent. When the
//!   owner's lock ends, the next owner's value is sent, as a lock (the
//!   mixer's own 4 ms smoothing carries the change); when the last one
//!   ends, the control is let go. A track's locks on its own strip count
//!   too, so the instrument's own track and the tracks that play it share
//!   one line. A hand on the control is the mod engine's, as before.
//! - **Fixed tables.** Nothing allocates after [`StripLocks::new`]. A lock
//!   that finds the table full goes on as it came, counted.

use mpfree_play::mpfree_param::{track_strip, Owner, ParamAddr, Strip};

/// The most locks held at once: as many as the player holds.
pub const HOLDS: usize = mpfree_play::MAX_LOCKED;

/// What a clip sends on a parameter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ev {
    /// A step's lock, at this value.
    Lock(f32),
    /// A move's value.
    Lane(f32),
    /// The lock or move ends.
    End,
}

/// What goes on to the mixer's mod engine: a lock (`Some`) or its end
/// (`None`), or a move's value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sent {
    Lock(ParamAddr, Option<f32>),
    Lane(ParamAddr, f32),
}

/// One track's lock on its strip: as the clip named it, where it went,
/// and its value.
#[derive(Clone, Copy, Debug)]
struct Hold {
    src: u8,
    from: ParamAddr,
    to: ParamAddr,
    value: f32,
    lane: bool,
}

const NO_HOLD: Hold = Hold {
    src: 0,
    from: ParamAddr::new(Owner::Strip(Strip::Master), 0),
    to: ParamAddr::new(Owner::Strip(Strip::Master), 0),
    value: 0.0,
    lane: false,
};

/// The tracks' locks on "this track's strip", with where each went.
#[derive(Clone, Debug)]
pub struct StripLocks {
    h: [Hold; HOLDS],
    n: usize,
    /// Locks that went to the strip their track's instrument sounds on.
    pub moved: u64,
    /// Locks kept but not sent: another track owned the control.
    pub waited: u64,
    /// Locks that found the table full (they went on as they came).
    pub full: u64,
}

impl Default for StripLocks {
    fn default() -> StripLocks {
        StripLocks::new()
    }
}

impl StripLocks {
    pub fn new() -> StripLocks {
        StripLocks {
            h: [NO_HOLD; HOLDS],
            n: 0,
            moved: 0,
            waited: 0,
            full: 0,
        }
    }

    /// What it did so far, for the control half's status line.
    pub fn counts(&self) -> super::link::StripCounts {
        super::link::StripCounts {
            moved: self.moved,
            waited: self.waited,
            full: self.full,
        }
    }

    /// Locks held now.
    #[cfg(test)]
    pub fn held(&self) -> usize {
        self.n
    }

    /// Track `track`'s `ev` on `addr`, where `sounds_on` is the mixer
    /// track its instrument sounds on (None: its own strip, or the host
    /// doesn't say) and `rank` a track's lasting id (0: none, last in
    /// line). False when `addr` isn't the track's own strip, or the table
    /// was full: it goes on as it came. Otherwise what to send goes to
    /// `out` (nothing, while another track owns the control).
    pub fn take(
        &mut self,
        track: u16,
        addr: ParamAddr,
        ev: Ev,
        sounds_on: Option<u8>,
        rank: impl Fn(u8) -> u32,
        out: &mut impl FnMut(Sent),
    ) -> bool {
        let Ok(src) = u8::try_from(track) else {
            return false;
        };
        let own = track_strip(src);
        if addr.owner != Owner::Strip(own) {
            return false;
        }
        let at = self.h[..self.n]
            .iter()
            .position(|h| h.src == src && h.from == addr);
        let (value, lane) = match ev {
            Ev::Lock(v) => (v, false),
            Ev::Lane(v) => (v, true),
            Ev::End => {
                // One never kept (the table was full) went on as it came.
                let Some(i) = at else {
                    return false;
                };
                self.end(i, &rank, out);
                return true;
            }
        };
        let to = ParamAddr::new(
            Owner::Strip(sounds_on.map_or(own, Strip::Track)),
            addr.param,
        );
        let i = match at {
            // Still where it went: a new value.
            Some(i) if self.h[i].to == to => {
                (self.h[i].value, self.h[i].lane) = (value, lane);
                i
            }
            // The instrument moved: it ends where it began first (which
            // leaves room for it again).
            Some(i) => {
                self.end(i, &rank, out);
                match self.hold(src, addr, to, value, lane) {
                    Some(i) => i,
                    None => return true,
                }
            }
            None => match self.hold(src, addr, to, value, lane) {
                Some(i) => i,
                None => {
                    self.full += 1;
                    return false;
                }
            },
        };
        if at != Some(i) && to != addr {
            self.moved += 1;
        }
        if self.owner(to, &rank) == Some(i) {
            out(if lane {
                Sent::Lane(to, value)
            } else {
                Sent::Lock(to, Some(value))
            });
        } else {
            self.waited += 1;
        }
        true
    }

    /// A new lock, if there is room.
    fn hold(
        &mut self,
        src: u8,
        from: ParamAddr,
        to: ParamAddr,
        value: f32,
        lane: bool,
    ) -> Option<usize> {
        if self.n == HOLDS {
            return None;
        }
        self.h[self.n] = Hold {
            src,
            from,
            to,
            value,
            lane,
        };
        self.n += 1;
        Some(self.n - 1)
    }

    /// The lock that owns `to`: the first in line of those on it, by
    /// lasting id (one without last), then by track.
    fn owner(&self, to: ParamAddr, rank: &impl Fn(u8) -> u32) -> Option<usize> {
        let place = |src: u8| {
            let id = rank(src);
            (if id == 0 { u32::MAX } else { id }, src)
        };
        (0..self.n)
            .filter(|&i| self.h[i].to == to)
            .min_by_key(|&i| place(self.h[i].src))
    }

    /// Lock `i` ends on the strip it went to: the next owner's value
    /// there, the control let go when none is left, or nothing when it
    /// didn't own it.
    fn end(&mut self, i: usize, rank: &impl Fn(u8) -> u32, out: &mut impl FnMut(Sent)) {
        let to = self.h[i].to;
        let owned = self.owner(to, rank) == Some(i);
        self.h.copy_within(i + 1..self.n, i);
        self.n -= 1;
        if !owned {
            return;
        }
        match self.owner(to, rank) {
            Some(j) => out(Sent::Lock(to, Some(self.h[j].value))),
            None => out(Sent::Lock(to, None)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Track `t`'s own strip's parameter `param` (0 is track 1).
    fn strip(t: u8, param: u16) -> ParamAddr {
        ParamAddr::new(Owner::Strip(track_strip(t)), param)
    }

    fn on(mixer_track: u8, param: u16) -> ParamAddr {
        ParamAddr::new(Owner::Strip(Strip::Track(mixer_track)), param)
    }

    /// Lasting ids: track `t` has `ids[t]`.
    fn ranks(ids: &[u32]) -> impl Fn(u8) -> u32 + '_ {
        move |t| ids.get(usize::from(t)).copied().unwrap_or(0)
    }

    struct Run {
        s: StripLocks,
        ids: Vec<u32>,
        sent: Vec<Sent>,
    }

    impl Run {
        fn new(ids: &[u32]) -> Run {
            Run {
                s: StripLocks::new(),
                ids: ids.to_vec(),
                sent: Vec::new(),
            }
        }

        /// Track `t`'s `ev` on its own strip's `param`, its instrument on
        /// mixer track `sounds_on`; what went out.
        fn ev(&mut self, t: u8, param: u16, ev: Ev, sounds_on: Option<u8>) -> Vec<Sent> {
            self.at(t, strip(t, param), ev, sounds_on)
        }

        fn at(&mut self, t: u8, addr: ParamAddr, ev: Ev, sounds_on: Option<u8>) -> Vec<Sent> {
            let mut got = Vec::new();
            let took = self.s.take(
                u16::from(t),
                addr,
                ev,
                sounds_on,
                ranks(&self.ids),
                &mut |s| got.push(s),
            );
            assert!(took, "taken");
            self.sent.extend(&got);
            got
        }
    }

    #[test]
    fn a_lock_on_this_tracks_strip_goes_where_its_instrument_sounds() {
        // Track 5 (4) plays track 3's (2) instrument, which sounds on
        // track 3's strip, mixer track 6.
        let mut r = Run::new(&[1, 2, 3, 4, 5]);
        let there = on(6, 0);
        assert_eq!(
            r.ev(4, 0, Ev::Lock(0.25), Some(6)),
            [Sent::Lock(there, Some(0.25))]
        );
        assert_eq!(r.s.moved, 1);
        // Its next value and its end go there too.
        assert_eq!(r.ev(4, 0, Ev::Lane(0.5), Some(6)), [Sent::Lane(there, 0.5)]);
        assert_eq!(r.ev(4, 0, Ev::End, Some(6)), [Sent::Lock(there, None)]);
        assert_eq!(r.s.held(), 0);
        // On its own strip (the host says nothing, or the same strip), as
        // it came.
        assert_eq!(
            r.ev(4, 0, Ev::Lock(0.7), None),
            [Sent::Lock(strip(4, 0), Some(0.7))]
        );
        assert_eq!(r.ev(4, 0, Ev::End, None), [Sent::Lock(strip(4, 0), None)]);
        let own = match track_strip(2) {
            Strip::Track(n) => n,
            s => panic!("{s:?}"),
        };
        assert_eq!(
            r.ev(2, 1, Ev::Lock(0.1), Some(own)),
            [Sent::Lock(strip(2, 1), Some(0.1))]
        );
        assert_eq!(r.s.moved, 1, "only the first went elsewhere");
    }

    #[test]
    fn only_this_tracks_own_strip_is_moved() {
        let mut s = StripLocks::new();
        let mut got = Vec::new();
        let none = |_: u8| 0;
        // The master, a submix, another track's strip, an effect on the
        // track's own strip, an instrument: not this table's.
        for addr in [
            ParamAddr::new(Owner::Strip(Strip::Master), 0),
            ParamAddr::new(Owner::Strip(Strip::Bus(1)), 0),
            strip(7, 0),
            ParamAddr::new(Owner::Insert(track_strip(4), 1), 2),
            ParamAddr::new(Owner::Inst(4), 3),
        ] {
            for ev in [Ev::Lock(0.5), Ev::Lane(0.5), Ev::End] {
                assert!(
                    !s.take(4, addr, ev, Some(6), none, &mut |x| got.push(x)),
                    "{addr}"
                );
            }
        }
        assert!(got.is_empty());
        assert_eq!(s.held(), 0);
        // An end with no lock kept goes on as it came.
        assert!(!s.take(4, strip(4, 0), Ev::End, Some(6), none, &mut |x| got.push(x)));
    }

    #[test]
    fn a_lock_ends_on_the_strip_it_began_on_when_the_instrument_moves() {
        let mut r = Run::new(&[1, 2, 3, 4, 5]);
        assert_eq!(
            r.ev(4, 0, Ev::Lock(0.25), Some(6)),
            [Sent::Lock(on(6, 0), Some(0.25))]
        );
        // The instrument moved to mixer track 8: the end still goes to 6.
        assert_eq!(r.ev(4, 0, Ev::End, Some(8)), [Sent::Lock(on(6, 0), None)]);
        // A move held across the change: its next value ends it on the old
        // strip first, then goes to the new one.
        assert_eq!(
            r.ev(4, 0, Ev::Lane(0.3), Some(6)),
            [Sent::Lane(on(6, 0), 0.3)]
        );
        assert_eq!(
            r.ev(4, 0, Ev::Lane(0.4), Some(8)),
            [Sent::Lock(on(6, 0), None), Sent::Lane(on(8, 0), 0.4)]
        );
        assert_eq!(r.ev(4, 0, Ev::End, Some(6)), [Sent::Lock(on(8, 0), None)]);
        assert_eq!(r.s.held(), 0);
    }

    #[test]
    fn two_tracks_on_one_strip_take_turns_by_lasting_id() {
        // Track 3 (2) owns the instrument on mixer track 6; tracks 5 (4)
        // and 6 (5) play it. Lasting ids: track 6 was made before track 5.
        let mut r = Run::new(&[1, 2, 7, 4, 9, 3]);
        let vol = on(6, 0);
        // Track 5 first: it owns the control.
        assert_eq!(
            r.ev(4, 0, Ev::Lock(0.2), Some(6)),
            [Sent::Lock(vol, Some(0.2))]
        );
        // Track 6 (id 3) comes before track 5 (id 9): it takes over at once.
        assert_eq!(
            r.ev(5, 0, Ev::Lock(0.6), Some(6)),
            [Sent::Lock(vol, Some(0.6))]
        );
        // Track 3 (id 7), on its own strip, waits behind track 6 but comes
        // before track 5.
        assert!(r.ev(2, 0, Ev::Lock(0.9), Some(6)).is_empty());
        assert_eq!(r.s.waited, 1);
        // Track 5's new value waits: track 6 owns it.
        assert!(r.ev(4, 0, Ev::Lock(0.3), Some(6)).is_empty());
        // Track 6 ends: the next in line, track 3, plays its value.
        assert_eq!(r.ev(5, 0, Ev::End, Some(6)), [Sent::Lock(vol, Some(0.9))]);
        // Track 5 ending changes nothing: it doesn't own it.
        assert!(r.ev(4, 0, Ev::End, Some(6)).is_empty());
        // The last one ends: the control is let go.
        assert_eq!(r.ev(2, 0, Ev::End, Some(6)), [Sent::Lock(vol, None)]);
        assert_eq!(r.s.held(), 0);
        // Another parameter on the same strip is a line of its own.
        assert_eq!(
            r.ev(4, 0, Ev::Lock(0.2), Some(6)),
            [Sent::Lock(vol, Some(0.2))]
        );
        assert_eq!(
            r.ev(5, 1, Ev::Lock(0.5), Some(6)),
            [Sent::Lock(on(6, 1), Some(0.5))]
        );
    }

    #[test]
    fn a_track_without_a_lasting_id_comes_last_then_by_track() {
        let mut r = Run::new(&[0, 0, 5]);
        // Track 1 (no id) first, then track 3 (id 5) takes over.
        assert_eq!(
            r.ev(0, 0, Ev::Lock(0.1), Some(9)),
            [Sent::Lock(on(9, 0), Some(0.1))]
        );
        assert_eq!(
            r.ev(2, 0, Ev::Lock(0.2), Some(9)),
            [Sent::Lock(on(9, 0), Some(0.2))]
        );
        // Track 2 (no id) waits behind both.
        assert!(r.ev(1, 0, Ev::Lock(0.3), Some(9)).is_empty());
        assert_eq!(
            r.ev(2, 0, Ev::End, Some(9)),
            [Sent::Lock(on(9, 0), Some(0.1))]
        );
        assert_eq!(
            r.ev(0, 0, Ev::End, Some(9)),
            [Sent::Lock(on(9, 0), Some(0.3))]
        );
    }

    #[test]
    fn a_full_table_lets_locks_go_on_as_they_came() {
        let mut s = StripLocks::new();
        let none = |_: u8| 0;
        let mut got = Vec::new();
        for i in 0..HOLDS {
            let t = (i % 100) as u8;
            let param = (i / 100) as u16;
            assert!(s.take(
                u16::from(t),
                strip(t, param),
                Ev::Lock(0.5),
                Some(200),
                none,
                &mut |x| got.push(x)
            ));
        }
        assert_eq!(s.held(), HOLDS);
        assert!(!s.take(
            120,
            strip(120, 0),
            Ev::Lock(0.5),
            Some(200),
            none,
            &mut |x| got.push(x)
        ));
        assert_eq!(s.full, 1);
        // Its end isn't kept either, so it goes on as it came.
        assert!(
            !s.take(120, strip(120, 0), Ev::End, Some(200), none, &mut |x| got
                .push(x))
        );
    }
}
