//! A song time edit on the regions (the Arrange refactor's 3.4).
//!
//! When the song's time is edited (a section moved or copied, bars put in
//! or taken out: `mpfree_seq::TimeEdit`), every region moves with the
//! stretch of song it sits in, so a vocal stays with its verse. The edit
//! comes as up to [`MAX_PIECES`] [`Piece`]s: a stretch of the old song
//! (`from..end`) and the tick it starts on after (`to`), in order of `to`,
//! none overlapping after the edit. What lies in no piece is gone (a
//! delete), and a tick no piece reaches is silent (an insert's room).
//!
//! - A region is cut where a piece starts or ends, and each part moves
//!   with its piece. A part that starts inside the region plays on from
//!   where the region had got to: its `src` moves on by the frames from
//!   the region's start to the cut along the old tempo map (the tempo
//!   moves with its stretch, so those frames are the same after).
//! - A region keeps its fade in on the part that holds its start and its
//!   fade out on the part that holds its end; a cut has neither (as
//!   [`crate::Command::Split`] cuts).
//! - Two parts of one region that meet again with the audio running on
//!   (the second starts where the first ended in the old song: a copy put
//!   straight after what it copies) are one region again, so no crossfade
//!   lies in the middle of a held note.
//! - Nothing allocates: [`Retime`] is a fixed size and [`Retime::apply`]
//!   writes into a slice the caller owns, so the audio thread can make it.

use crate::{frames_from, Region};
use mpfree_transport::time::{TempoMap, Tick};

/// Pieces an edit has at most (a move: what comes before, the moved
/// stretch, what it passes, what comes after).
pub const MAX_PIECES: usize = 4;

/// A stretch of the old song, `from..end` (`end` [`Tick::MAX`] for to the
/// song's end), that starts on tick `to` after the edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    pub from: Tick,
    pub end: Tick,
    pub to: Tick,
}

impl Piece {
    const NONE: Piece = Piece {
        from: 0,
        end: 0,
        to: 0,
    };

    /// Where old tick `t` (inside the piece) is after the edit.
    fn place(&self, t: Tick) -> Tick {
        t - self.from + self.to
    }
}

/// A song time edit for the regions: see the module notes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retime {
    pieces: [Piece; MAX_PIECES],
    n: u8,
}

impl Retime {
    /// The edit with these pieces, or None for more than [`MAX_PIECES`], an
    /// empty or negative one, or pieces out of order or overlapping after
    /// the edit.
    pub fn new(pieces: &[Piece]) -> Option<Retime> {
        if pieces.len() > MAX_PIECES {
            return None;
        }
        let mut after: Tick = 0;
        for p in pieces {
            if p.from < 0 || p.end <= p.from || p.to < after {
                return None;
            }
            after = if p.end == Tick::MAX {
                Tick::MAX
            } else {
                p.to.checked_add(p.end - p.from)?
            };
        }
        let mut out = [Piece::NONE; MAX_PIECES];
        out[..pieces.len()].copy_from_slice(pieces);
        Some(Retime {
            pieces: out,
            n: pieces.len() as u8,
        })
    }

    pub fn pieces(&self) -> &[Piece] {
        &self.pieces[..usize::from(self.n)]
    }

    /// Whether the edit changes any of `regions` (one that moves, is cut
    /// or goes): a track it leaves as it is needs no undo step.
    pub fn changes(&self, regions: &[Region]) -> bool {
        regions.iter().any(|r| {
            !self
                .pieces()
                .iter()
                .any(|p| p.from <= r.start && r.end <= p.end && p.to == p.from)
        })
    }

    /// How many regions `regions` become (parts cut at the edges, joined
    /// where they run on): for a dry run, with no room needed.
    pub fn count(&self, regions: &[Region]) -> usize {
        let mut n = 0;
        let mut last: Option<(usize, Tick, Tick)> = None;
        for p in self.pieces() {
            for (i, r) in regions.iter().enumerate() {
                if r.end <= p.from || r.start >= p.end {
                    continue;
                }
                let (a, b) = (r.start.max(p.from), r.end.min(p.end));
                let joins = last.is_some_and(|(j, old_end, new_end)| {
                    j == i && old_end == a && new_end == p.place(a)
                });
                if !joins {
                    n += 1;
                }
                last = Some((i, b, p.place(b)));
            }
        }
        n
    }

    /// The regions after the edit, written into `out` in order, and how
    /// many; None (and `out` part written) when they need more room than
    /// `out` has. `regions` are in order and don't overlap, on the old
    /// tempo map `map`, at `rate` frames a second.
    pub fn apply(
        &self,
        regions: &[Region],
        map: &TempoMap,
        rate: u32,
        out: &mut [Region],
    ) -> Option<usize> {
        let mut n: usize = 0;
        // Where the last region put came from: its index and the old tick
        // its end was, to join a part of the same region running on.
        let mut last: Option<(usize, Tick)> = None;
        for p in self.pieces() {
            for (i, r) in regions.iter().enumerate() {
                if r.end <= p.from || r.start >= p.end {
                    continue;
                }
                let (a, b) = (r.start.max(p.from), r.end.min(p.end));
                let mut q = *r;
                if a > r.start {
                    q.src = r.src + frames_from(map, r.start as f64, a as f64, rate).round() as i64;
                    q.fade_in = 0;
                }
                if b < r.end {
                    q.fade_out = 0;
                }
                q.start = p.place(a);
                q.end = p.place(b);
                if let (Some((j, old_end)), Some(prev)) = (last, n.checked_sub(1)) {
                    if j == i && old_end == a && out[prev].end == q.start {
                        out[prev].end = q.end;
                        out[prev].fade_out = q.fade_out;
                        last = Some((i, b));
                        continue;
                    }
                }
                *out.get_mut(n)? = q;
                n += 1;
                last = Some((i, b));
            }
        }
        Some(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_REGIONS;

    const RATE: u32 = 48_000;
    /// A bar of 4/4 in ticks (960 a beat).
    const BAR: Tick = 3840;

    fn map() -> TempoMap {
        TempoMap::constant(120.0).unwrap()
    }

    fn region(start: Tick, end: Tick, take: u16, src: i64) -> Region {
        Region {
            start,
            end,
            take,
            src,
            gain: 1.0,
            fade_in: 100,
            fade_out: 200,
        }
    }

    fn piece(from: Tick, end: Option<Tick>, to: Tick) -> Piece {
        Piece {
            from,
            end: end.unwrap_or(Tick::MAX),
            to,
        }
    }

    fn apply(r: &Retime, regions: &[Region]) -> Vec<Region> {
        let mut out = [Region::EMPTY; MAX_REGIONS];
        let n = r.apply(regions, &map(), RATE, &mut out).expect("room");
        out[..n].to_vec()
    }

    /// The take and frame that plays at tick `t` (each region from its
    /// start at the recorded rate, as the tracks play it), at 120 BPM.
    fn heard(regions: &[Region], t: Tick) -> Option<(u16, i64)> {
        let r = regions.iter().find(|r| r.contains(t))?;
        let f = frames_from(&map(), r.start as f64, t as f64, RATE).round() as i64;
        Some((r.take, r.src + f))
    }

    /// A bar is 2 s at 120 BPM: 96,000 frames.
    const BAR_FRAMES: i64 = 96_000;

    /// Verse (bars 5-8) moved before the Intro (bars 1-4): a vocal across
    /// the Intro's end is cut there, each part goes with its section and
    /// plays what it played, and the fades stay on the outer ends.
    #[test]
    fn a_moved_section_takes_its_audio_with_it() {
        let (from, to) = (4 * BAR, 8 * BAR);
        let r = Retime::new(&[
            piece(from, Some(to), 0),
            piece(0, Some(from), to - from),
            piece(to, None, to),
        ])
        .unwrap();
        let vocal = region(2 * BAR, 6 * BAR, 1, 5_000);
        let guitar = region(9 * BAR, 10 * BAR, 2, 0);
        let after = apply(&r, &[vocal, guitar]);
        assert_eq!(
            after,
            [
                Region {
                    start: 0,
                    end: 2 * BAR,
                    src: 5_000 + 2 * BAR_FRAMES,
                    fade_in: 0,
                    ..vocal
                },
                Region {
                    start: 6 * BAR,
                    end: 8 * BAR,
                    fade_out: 0,
                    ..vocal
                },
                guitar,
            ]
        );
        // Every tick plays in its new place what it played in its old one.
        for t in (0..12 * BAR).step_by(97) {
            let new = if t < from {
                t + (to - from)
            } else if t < to {
                t - from
            } else {
                t
            };
            assert_eq!(heard(&[vocal, guitar], t), heard(&after, new), "tick {t}");
        }
    }

    /// Two empty bars at bar 5: everything from there two bars on, a
    /// region across bar 5 cut there with the room silent.
    #[test]
    fn bars_put_in_leave_room_and_move_the_rest_on() {
        let at = 4 * BAR;
        let r = Retime::new(&[piece(0, Some(at), 0), piece(at, None, at + 2 * BAR)]).unwrap();
        let held = region(3 * BAR, 5 * BAR, 3, 0);
        let after = apply(&r, &[held]);
        assert_eq!(after.len(), 2);
        assert_eq!(
            (after[0].start, after[0].end, after[0].fade_out),
            (3 * BAR, at, 0)
        );
        assert_eq!(
            (after[1].start, after[1].end, after[1].src, after[1].fade_in),
            (6 * BAR, 7 * BAR, BAR_FRAMES, 0)
        );
        assert_eq!(heard(&after, at + BAR), None, "the room is silent");
    }

    /// Bars 3-4 taken out: what was in them goes, what crossed their edges
    /// is cut there, what came after moves back; the two parts either
    /// side of the gap stay two (the audio between them is gone).
    #[test]
    fn bars_taken_out_take_their_audio_with_them() {
        let (from, to) = (2 * BAR, 4 * BAR);
        let r = Retime::new(&[piece(0, Some(from), 0), piece(to, None, from)]).unwrap();
        let inside = region(2 * BAR + 100, 3 * BAR, 1, 0);
        let across = region(BAR, 5 * BAR, 2, 0);
        // One track at a time (the two overlap).
        assert_eq!(apply(&r, &[inside]), []);
        let after = apply(&r, &[across]);
        assert_eq!(
            after
                .iter()
                .map(|q| (q.start, q.end, q.src))
                .collect::<Vec<_>>(),
            [(BAR, 2 * BAR, 0), (2 * BAR, 3 * BAR, 3 * BAR_FRAMES)]
        );
        assert_eq!(heard(&after, 2 * BAR), heard(&[across], 4 * BAR));
    }

    /// A copy put straight after what it copies: the copy's part and the
    /// part after it run on in the old song, so they are one region.
    #[test]
    fn a_copy_right_after_its_stretch_runs_on_into_the_rest() {
        let (from, to) = (2 * BAR, 4 * BAR);
        let w = to - from;
        let r = Retime::new(&[
            piece(0, Some(to), 0),
            piece(from, Some(to), to),
            piece(to, None, to + w),
        ])
        .unwrap();
        let held = region(3 * BAR, 6 * BAR, 1, 0);
        let after = apply(&r, &[held]);
        assert_eq!(
            after
                .iter()
                .map(|q| (q.start, q.end, q.src, q.fade_in, q.fade_out))
                .collect::<Vec<_>>(),
            [
                (3 * BAR, 4 * BAR, 0, 100, 0),
                (5 * BAR, 8 * BAR, 0, 100, 200),
            ],
            "the copy's part and the rest joined"
        );
    }

    /// Too many parts for the room: None, and a track nothing moves needs
    /// no step.
    #[test]
    fn no_room_is_refused_and_a_track_left_alone_is_unchanged() {
        let r = Retime::new(&[
            piece(0, Some(4 * BAR), 0),
            piece(2 * BAR, Some(4 * BAR), 4 * BAR),
            piece(4 * BAR, None, 6 * BAR),
        ])
        .unwrap();
        let regions = [region(0, 2 * BAR, 1, 0), region(3 * BAR, 5 * BAR, 1, 0)];
        // Bars 1-2, bar 4 of the second, its copy running on into bar 5.
        let mut out = [Region::EMPTY; 2];
        assert_eq!(r.apply(&regions, &map(), RATE, &mut out), None);
        let mut out = [Region::EMPTY; 3];
        assert_eq!(r.apply(&regions, &map(), RATE, &mut out), Some(3));
        assert_eq!(r.count(&regions), 3);
        assert!(r.changes(&regions));
        assert!(!r.changes(&regions[..1]), "bars 1-2 stay where they are");
        assert!(Retime::new(&[piece(0, Some(0), 0)]).is_none());
        assert!(Retime::new(&[piece(BAR, None, BAR), piece(0, Some(BAR), 0)]).is_none());
    }
}
