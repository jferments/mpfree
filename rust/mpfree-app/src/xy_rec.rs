//! Where the XY pad's moves record (the XY controller plan, phase 3, 3.1:
//! a layout of yours's Set settings record).
//!
//! Each period the sound loop takes the pad's moves
//! ([`crate::xy_play::RecMove`]) and sends each where it records:
//!
//! - **REC records a track:** that track's own sounds (the synth or kit it
//!   plays, shared with another track or not, its mixer strip and
//!   effects, its MIDI effects, its arp, its outside gear) go into the
//!   clip REC records, as smooth moves, through `TimeHost::gesture` (which
//!   the move log keeps for CAPTURE too).
//! - **Rec song records:** everything else (the master, a bus, a return,
//!   another track) goes into the song's own lanes through
//!   `TimeHost::song_gesture`. With Rec song on its own, everything does,
//!   track settings included: a pass of moves over the whole song.
//! - **Neither:** a move of the armed track's own sounds goes to the move
//!   log alone, so CAPTURE can keep it, and the rest isn't recorded.
//!
//! A gesture records where it began: REC punching in or out, or Rec song
//! going on, mid-move ends it there and it goes on where it goes now.
//! What records is the pad's hand, not the setting's value: lanes play at
//! the lock layer, under the mod matrix, so an LFO plays on top of a
//! recorded move as it did on top of the hand. Follow selected track
//! changes what the pad moves, never where a move records: a setting goes
//! where its own track says. XY moves never go through `record_knob`, so a
//! move never records twice, as a lane and as a step's lock.
//!
//! Nothing here allocates.

use crate::xy_play::RecMove;
use mpfree_play::mpfree_param::{Owner, ParamAddr};
use mpfree_seq::mpfree_moves::{Input, Map};
use mpfree_xy::SLOTS;

/// Where a move records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Goes {
    /// Into the clip REC records (and the move log).
    Clip,
    /// Into the song's own lanes.
    Song,
    /// To the move log alone, for CAPTURE.
    Log,
    /// Nowhere.
    Not,
}

/// What records now, as the sound loop sees it each period.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Now {
    /// The track REC records, or would (the armed track).
    pub armed: Option<u8>,
    /// REC records it now.
    pub rec: bool,
    /// Rec song is armed.
    pub song: bool,
    /// The transport rolls.
    pub rolling: bool,
}

impl Now {
    /// Where a move of `addr` records. `plays(t)`: the armed track plays
    /// the synth `Owner::Inst(t)` names (its own, another track's by a
    /// send, or the shared synth, which `Owner::Inst` names on a track
    /// without one of its own).
    pub fn goes(&self, addr: ParamAddr, plays: impl Fn(u8) -> bool) -> Goes {
        if !self.rolling {
            return Goes::Not;
        }
        let own = self.armed.is_some_and(|a| own(addr, a, &plays));
        match (own, self.rec, self.song) {
            (true, true, _) => Goes::Clip,
            (_, _, true) => Goes::Song,
            (true, false, false) => Goes::Log,
            (false, _, false) => Goes::Not,
        }
    }
}

/// Whether `addr` is one of track `track`'s own sounds: see the module
/// notes.
pub fn own(addr: ParamAddr, track: u8, plays: impl Fn(u8) -> bool) -> bool {
    match addr.owner {
        Owner::Inst(t) => plays(t),
        o => o.track() == Some(track),
    }
}

/// Whether the armed track `armed` plays the synth `Owner::Inst(t)` names
/// (the `plays` of [`Now::goes`]). `owner(track)`: the place whose synth a
/// track plays, its own or another's by a send (None: it has none, so
/// `Owner::Inst` names the shared synth on it); `pads_synth(track)`: a
/// track with none plays the shared synth (not a kit).
pub fn plays_synth(
    armed: u8,
    t: u8,
    owner: impl Fn(usize) -> Option<usize>,
    pads_synth: impl Fn(usize) -> bool,
) -> bool {
    match (owner(usize::from(t)), owner(usize::from(armed))) {
        (Some(x), Some(y)) => x == y,
        (None, None) => pads_synth(usize::from(armed)),
        _ => false,
    }
}

/// The XY pad's gestures under way: where each records.
#[derive(Debug)]
pub struct XyRec {
    on: [Option<(ParamAddr, Goes, Map)>; SLOTS * 2],
}

impl Default for XyRec {
    fn default() -> XyRec {
        XyRec {
            on: [None; SLOTS * 2],
        }
    }
}

impl XyRec {
    pub fn new() -> XyRec {
        XyRec::default()
    }

    /// Gestures under way.
    #[cfg(test)]
    pub fn moving(&self) -> usize {
        self.on.iter().flatten().count()
    }

    /// One of the pad's moves, while `now` says what records. `plays` as
    /// [`Now::goes`]; `map(addr)` the setting's map (asked once, as its
    /// gesture starts; None leaves it out); `give(goes, input)` sends it
    /// there (`Goes::Clip` and `Goes::Log` through `TimeHost::gesture`,
    /// `Goes::Song` through `TimeHost::song_gesture`).
    pub fn feed(
        &mut self,
        m: RecMove,
        now: &Now,
        plays: impl Fn(u8) -> bool,
        map: impl FnOnce(ParamAddr) -> Option<Map>,
        mut give: impl FnMut(Goes, Input),
    ) {
        if !now.rolling {
            // The recorder, the move log and the song recorder end what
            // moved at the stop.
            self.on = [None; SLOTS * 2];
            return;
        }
        let addr = match m {
            RecMove::Moved(a, _) | RecMove::Ended(a) => a,
        };
        let k = self.on.iter().position(|o| o.is_some_and(|o| o.0 == addr));
        match m {
            RecMove::Ended(_) => {
                if let Some((_, goes, _)) = k.and_then(|k| self.on[k].take()) {
                    if goes != Goes::Not {
                        give(goes, Input::End { addr });
                    }
                }
            }
            RecMove::Moved(_, value) => {
                let to = now.goes(addr, plays);
                let map = match k.and_then(|k| self.on[k]) {
                    Some((_, was, map)) => {
                        if was != to && was != Goes::Not {
                            // Where it records changed mid-move: it ends
                            // there and goes on here.
                            give(was, Input::End { addr });
                        }
                        map
                    }
                    None => {
                        if to == Goes::Not {
                            return;
                        }
                        let Some(map) = map(addr) else { return };
                        map
                    }
                };
                let slot = k.or_else(|| self.on.iter().position(Option::is_none));
                let Some(slot) = slot else { return };
                self.on[slot] = Some((addr, to, map));
                if to != Goes::Not {
                    give(to, Input::Move { addr, value, map });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_play::mpfree_param::{track_strip, Strip};

    fn inst(t: u8) -> ParamAddr {
        ParamAddr::new(Owner::Inst(t), 3)
    }
    fn strip(t: u8) -> ParamAddr {
        ParamAddr::new(Owner::Strip(track_strip(t)), 1)
    }
    fn master() -> ParamAddr {
        ParamAddr::new(Owner::Strip(Strip::Master), 0)
    }

    /// REC records the armed track's own sounds into its clip, a synth it
    /// shares with another track included; Rec song takes the rest, and
    /// with Rec song alone all of it; the move log keeps the armed track's
    /// own otherwise; nothing records while stopped.
    #[test]
    fn each_setting_records_where_its_track_says() {
        // Track 1 plays track 3's synth, not one of its own.
        let plays = |t: u8| t == 3;
        let mut now = Now {
            armed: Some(1),
            rec: true,
            song: false,
            rolling: true,
        };
        assert_eq!(now.goes(inst(3), plays), Goes::Clip, "shared with track 3");
        assert_eq!(now.goes(strip(1), plays), Goes::Clip);
        assert_eq!(
            now.goes(ParamAddr::new(Owner::Arp(1), 0), plays),
            Goes::Clip
        );
        assert_eq!(now.goes(inst(2), plays), Goes::Not);
        assert_eq!(now.goes(master(), plays), Goes::Not);
        now.song = true;
        assert_eq!(now.goes(inst(3), plays), Goes::Clip);
        assert_eq!(now.goes(master(), plays), Goes::Song);
        assert_eq!(now.goes(strip(2), plays), Goes::Song, "another track's");
        // Rec song alone: everything goes into the song.
        now.rec = false;
        assert_eq!(now.goes(inst(3), plays), Goes::Song);
        // Neither: the armed track's own for CAPTURE.
        now.song = false;
        assert_eq!(now.goes(strip(1), plays), Goes::Log);
        assert_eq!(now.goes(master(), plays), Goes::Not);
        now.rolling = false;
        assert_eq!(now.goes(strip(1), plays), Goes::Not);
    }

    /// Two tracks that play one synth (one sends to the other's, or both
    /// to one in a free place) share it: a move of it records into the
    /// clip of whichever of them REC records. The shared synth is a
    /// track's only while that track plays it, not a kit.
    #[test]
    fn a_synth_two_tracks_play_is_each_ones_own() {
        // Track 1 has its own; tracks 2 and 3 play one in a free place
        // (130); tracks 4 and 5 have none, 5 plays the shared synth.
        let owner = |t: usize| match t {
            0 => Some(0),
            1 | 2 => Some(130),
            _ => None,
        };
        let pads_synth = |t: usize| t == 4;
        let plays = |armed: u8, t: u8| plays_synth(armed, t, owner, pads_synth);
        assert!(plays(2, 1), "track 3 plays track 2's");
        assert!(plays(1, 2) && plays(1, 1));
        assert!(!plays(2, 0), "not track 1's own");
        assert!(plays(0, 0));
        assert!(!plays(0, 1));
        assert!(plays(4, 3), "the shared synth, on track 4 or 5");
        assert!(!plays(3, 3), "track 4 plays a kit");
        assert!(!plays(4, 1) && !plays(1, 3));
        let now = Now {
            armed: Some(2),
            rec: true,
            song: true,
            rolling: true,
        };
        let goes = |t: u8| now.goes(inst(t), |t| plays(2, t));
        assert_eq!(goes(1), Goes::Clip, "track 2's synth, in track 3's clip");
        assert_eq!(goes(0), Goes::Song);
    }

    /// A gesture's moves go where it records, its map asked once; a change
    /// of where it records mid-move ends it there and goes on in the new
    /// place; a stop forgets it.
    #[test]
    fn a_gesture_records_where_it_goes_and_moves_on_when_that_changes() {
        let plays = |t: u8| t == 0;
        let mut now = Now {
            armed: Some(0),
            rec: false,
            song: true,
            rolling: true,
        };
        let mut r = XyRec::new();
        let mut sent = Vec::new();
        let mut asked = 0;
        let map = Map::linear(0.0, 1.0);
        let mut feed = |r: &mut XyRec, m: RecMove, now: &Now, sent: &mut Vec<(Goes, Input)>| {
            r.feed(
                m,
                now,
                plays,
                |_| {
                    asked += 1;
                    Some(map)
                },
                |g, i| sent.push((g, i)),
            );
        };
        let a = inst(0);
        feed(&mut r, RecMove::Moved(a, 0.2), &now, &mut sent);
        feed(&mut r, RecMove::Moved(a, 0.4), &now, &mut sent);
        // REC punches in mid-move: the song's part ends, the clip's starts.
        now.rec = true;
        feed(&mut r, RecMove::Moved(a, 0.6), &now, &mut sent);
        feed(&mut r, RecMove::Ended(a), &now, &mut sent);
        let mv = |v: f32| Input::Move {
            addr: a,
            value: v,
            map,
        };
        assert_eq!(
            sent,
            [
                (Goes::Song, mv(0.2)),
                (Goes::Song, mv(0.4)),
                (Goes::Song, Input::End { addr: a }),
                (Goes::Clip, mv(0.6)),
                (Goes::Clip, Input::End { addr: a }),
            ]
        );
        assert_eq!(r.moving(), 0);
        // The master with only REC on records nowhere, and isn't kept.
        sent.clear();
        now.song = false;
        feed(&mut r, RecMove::Moved(master(), 0.5), &now, &mut sent);
        assert!(sent.is_empty());
        assert_eq!(r.moving(), 0);
        // A stop forgets what moved.
        feed(&mut r, RecMove::Moved(a, 0.5), &now, &mut sent);
        assert_eq!(r.moving(), 1);
        now.rolling = false;
        feed(&mut r, RecMove::Moved(a, 0.5), &now, &mut sent);
        assert_eq!(r.moving(), 0);
        assert_eq!(asked, 2, "once for each gesture that records");
    }
}
