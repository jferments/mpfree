//! One edit of the song's time (the Arrange refactor's D6, phase 3):
//! insert bars, delete bars, move a range, copy a range. A [`TimeEdit`] is
//! applied to everything in the song that has a time at once, so nothing
//! is left behind: the blocks (loop blocks too), the markers, the tempo
//! points, the song's lanes (the morph's and the rest) and the morph's
//! rows. It is made on a copy ([`Arrangement::edited`]): refused, nothing
//! changes, and the reason says which part refused ([`Refused`]).
//!
//! Every edit is a list of parts laid end to end: stretches of the song as
//! it was, each put at a new place, and for an insert a gap. Insert at
//! `at`: the song before `at`, a gap, then the rest. Delete `from..to`:
//! the song before `from`, then the rest from `to`. Copy `from..to` to
//! `at`: the song before `at`, the copy, the rest from `at`. Move: the
//! moved range and what lies between it and `at` change places, so the
//! song closes up behind it and makes room for it; nothing is written
//! over.
//!
//! How each kind takes the parts:
//!
//! - **Blocks** are cut where a part starts or ends, each piece playing on
//!   from where its clip had got to (as [`Arrangement::clear`] cuts), and
//!   go with their part. A block never changes lane. Where two parts meet
//!   after the edit (a seam: where a delete closes up, either side of a
//!   moved or copied range), two pieces of one block that meet with its
//!   clip exactly where it had got to are one block again, so a long block
//!   of a one-bar loop stays one block when whole bars move. Blocks placed
//!   apart stay apart, even where they meet in phase: an insert and its
//!   delete leave the song playing as it did, but a block the insert cut
//!   stays two.
//! - **Markers** go with their part, keeping their names (a copied range's
//!   are copied); a deleted range's go.
//! - **Tempo**: each part takes the tempo in effect at its start with it, as
//!   a point at its new place, and the place it left keeps the tempo before
//!   it. A gap holds the tempo just before it. A ramp cut by a part's end
//!   keeps its curve: when what follows starts at the tempo the ramp had
//!   reached, it simply arrives there. Otherwise the next part's point
//!   takes the cut, so the ramp's last 1/16 holds (a point a 1/16 before
//!   the cut; the transport plays a ramp as a staircase of 1/16 steps
//!   anyway), at a tempo worked out so the cut lands when it did; the ramp
//!   arrives at that tempo, a hair off its old line. Points the edit adds that
//!   change nothing are left out, and so is a point on a seam that only
//!   carries on the ramp through it or the tempo before it.
//! - **The song's lanes** are cut and shifted as a clip's lanes are
//!   (`mpfree_moves`' edits): a lane keeps what it does on the ticks the
//!   edit doesn't touch, a ramp running into a cut keeps its line, and a gap
//!   holds the value before it.
//! - **The morph's rows** go with their part, each part starting with the
//!   rows in force at its start, a gap with those before it. Before the
//!   song's first rows there are none, which the rows can't say after
//!   others; a part from there carries on the rows before it (song mode
//!   doesn't load the rows at their ticks yet, arrange.md).
//!
//! A loop block plays its loop in step with the loop's own origin
//! (loop-tracks.md, part 5: a block's own origin is not built), so one
//! moved by other than whole passes of its loop plays another part of it.
//!
//! Plain data on the control thread: an edit allocates.

use crate::arrange::{
    Arrangement, Marker, MorphRows, Placement, TempoPoint, MAX_MARKERS, MAX_MORPHS, MAX_PLACEMENTS,
    MAX_TEMPO_POINTS, SONG_LANE_LEN,
};
use crate::mpfree_moves::{Moves, MovesError};
use crate::{ModelError, Track};
use mpfree_transport::time::{Tick, MAX_BPM, MIN_BPM, MIN_SPAN_TICKS, RAMP_STEP};

/// One edit of the song's time, in song ticks: see the module notes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeEdit {
    /// `len` ticks of room at `at`: everything from `at` on goes on by
    /// `len`, and the room holds what played just before it (no blocks).
    Insert { at: Tick, len: Tick },
    /// Ticks `from..to` taken out: what follows moves back to `from`.
    Delete { from: Tick, to: Tick },
    /// Ticks `from..to` moved to `at` (a tick of the song as it is, not
    /// inside the range): what lies between moves up to make room and close
    /// the gap. At `from` or `to`, nothing moves.
    Move { from: Tick, to: Tick, at: Tick },
    /// A copy of ticks `from..to` put in at `at`, everything from `at` on
    /// going on to make room (`at` may be inside the range).
    Copy { from: Tick, to: Tick, at: Tick },
}

/// Why an edit was refused. Nothing changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// No such range: empty, before the start, past the song's lanes'
    /// end, or a move to inside what it moves.
    BadRange,
    /// More than [`MAX_PLACEMENTS`] blocks.
    Blocks,
    /// More than [`MAX_MARKERS`] markers.
    Markers,
    /// More than [`MAX_TEMPO_POINTS`] tempo points.
    TempoPoints,
    /// A tempo point would come within a 1/16 of another, at this tick of
    /// the edited song: a point near an edge of the edit.
    TempoTooClose(Tick),
    /// A song lane would hold too many points.
    Lanes,
    /// More than [`MAX_MORPHS`] changes of the morph's rows.
    Morphs,
    /// The edited song fails its check (it never should).
    Check(ModelError),
}

/// A part of the edited song: see the module notes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    /// Song ticks `from..end` as they were (to the song's end for None),
    /// from `to` on.
    Piece {
        from: Tick,
        end: Option<Tick>,
        to: Tick,
    },
    /// Room from `to` (to the next piece) holding what played just before
    /// tick `from` as it was.
    Gap { from: Tick, to: Tick },
}

impl TimeEdit {
    /// Whether it leaves the song as it is (a move to where it is).
    pub fn moves_nothing(&self) -> bool {
        matches!(*self, TimeEdit::Move { from, to, at } if at == from || at == to)
    }

    /// Where song tick `t` is after the edit: None for a tick it deletes.
    /// A copy's ticks are new; the ticks it copied stay theirs. For what
    /// follows the edit about (the playhead, the loop range, a selection).
    pub fn place_of(&self, t: Tick) -> Option<Tick> {
        Some(match *self {
            TimeEdit::Insert { at, len } => {
                if t < at {
                    t
                } else {
                    t + len
                }
            }
            TimeEdit::Delete { from, to } => {
                if t < from {
                    t
                } else if t >= to {
                    t - (to - from)
                } else {
                    return None;
                }
            }
            TimeEdit::Copy { from, to, at } => {
                if t < at {
                    t
                } else {
                    t + (to - from)
                }
            }
            TimeEdit::Move { from, to, at } => {
                let w = to - from;
                if at < from {
                    if t < at || t >= to {
                        t
                    } else if t >= from {
                        t - from + at
                    } else {
                        t + w
                    }
                } else if at > to {
                    if t < from || t >= at {
                        t
                    } else if t >= to {
                        t - w
                    } else {
                        t - from + at - w
                    }
                } else {
                    t
                }
            }
        })
    }

    /// The stretches of the old song the edited one is made of, in order:
    /// each `(from, end, to)`, song ticks `from..end` (to the song's end
    /// for None) starting on tick `to` after the edit. An insert's room is
    /// in none. For what has a time outside the song (the audio tracks'
    /// regions, the Arrange refactor's 3.4). `BadRange` for no such range.
    pub fn pieces(&self) -> Result<Vec<(Tick, Option<Tick>, Tick)>, Refused> {
        Ok(self
            .parts()?
            .into_iter()
            .filter_map(|p| match p {
                Part::Piece { from, end, to } => Some((from, end, to)),
                Part::Gap { .. } => None,
            })
            .collect())
    }

    /// The parts of the edited song, in order, none empty: see the module
    /// notes. `BadRange` for no such range.
    fn parts(&self) -> Result<Vec<Part>, Refused> {
        let end = Tick::from(SONG_LANE_LEN);
        let piece = |from, end, to| Part::Piece { from, end, to };
        let range = |from: Tick, to: Tick| from >= 0 && from < to && to <= end;
        let mut parts = match *self {
            TimeEdit::Insert { at, len } => {
                if !(at >= 0 && len > 0 && at <= end && len <= end - at) {
                    return Err(Refused::BadRange);
                }
                vec![
                    piece(0, Some(at), 0),
                    Part::Gap { from: at, to: at },
                    piece(at, None, at + len),
                ]
            }
            TimeEdit::Delete { from, to } => {
                if !range(from, to) {
                    return Err(Refused::BadRange);
                }
                vec![piece(0, Some(from), 0), piece(to, None, from)]
            }
            TimeEdit::Copy { from, to, at } => {
                if !range(from, to) || !(0..=end - (to - from)).contains(&at) {
                    return Err(Refused::BadRange);
                }
                let w = to - from;
                vec![
                    piece(0, Some(at), 0),
                    piece(from, Some(to), at),
                    piece(at, None, at + w),
                ]
            }
            TimeEdit::Move { from, to, at } => {
                if !range(from, to) || !(0..=end).contains(&at) || (from < at && at < to) {
                    return Err(Refused::BadRange);
                }
                let w = to - from;
                if at < from {
                    vec![
                        piece(0, Some(at), 0),
                        piece(from, Some(to), at),
                        piece(at, Some(from), at + w),
                        piece(to, None, to),
                    ]
                } else if at > to {
                    vec![
                        piece(0, Some(from), 0),
                        piece(to, Some(at), from),
                        piece(from, Some(to), at - w),
                        piece(at, None, at),
                    ]
                } else {
                    vec![piece(0, None, 0)]
                }
            }
        };
        parts.retain(|p| !matches!(*p, Part::Piece { from, end: Some(e), .. } if e <= from));
        Ok(parts)
    }
}

/// The ticks where two pieces meet after the edit (not a gap's edges).
fn seams(parts: &[Part]) -> Vec<Tick> {
    parts
        .windows(2)
        .filter_map(|w| match (w[0], w[1]) {
            (Part::Piece { .. }, Part::Piece { to, .. }) => Some(to),
            _ => None,
        })
        .collect()
}

impl Arrangement {
    /// The song after `edit`, made on a copy (see the module notes), with
    /// the project's tempo `bpm` before the first tempo point. Refused,
    /// with which part refused, the song is as it was. What isn't timed
    /// stays: the song's clips (but those whose last block a delete took)
    /// and the song list (it names rows).
    pub fn edited(
        &self,
        tracks: &[Track],
        edit: TimeEdit,
        bpm: f64,
    ) -> Result<Arrangement, Refused> {
        let parts = edit.parts()?;
        if edit.moves_nothing() {
            return Ok(self.clone());
        }
        let seams = seams(&parts);
        let mut song = Arrangement {
            placements: self.edit_blocks(tracks, &parts, &seams)?,
            clips: self.clips.clone(),
            markers: self.edit_markers(&parts)?,
            lanes: edit_lanes(&self.lanes, edit).map_err(|_| Refused::Lanes)?,
            morphs: self.edit_morphs(&parts)?,
            list: self.list.clone(),
            tempo: self.edit_tempo(&parts, &seams, bpm)?,
        };
        song.drop_unplayed();
        song.check(tracks).map_err(Refused::Check)?;
        Ok(song)
    }

    /// The blocks, each part's cut at its edges and moved with it, then
    /// joined again at the seams where two pieces of one block meet in
    /// phase.
    fn edit_blocks(
        &self,
        tracks: &[Track],
        parts: &[Part],
        seams: &[Tick],
    ) -> Result<Vec<Placement>, Refused> {
        // Each piece with the block it came from.
        let mut out: Vec<(usize, Placement)> = Vec::with_capacity(self.placements.len() + 64);
        for part in parts {
            let Part::Piece {
                from: a,
                end,
                to: d,
            } = *part
            else {
                continue;
            };
            let b = end.unwrap_or(Tick::MAX);
            for (i, p) in self.placements.iter().enumerate() {
                if p.end() <= a || p.start >= b {
                    continue;
                }
                let mut q = if p.start < a {
                    p.from(p.clip(tracks, &self.clips), a)
                } else {
                    p.clone()
                };
                if q.end() > b {
                    q.length = b - q.start;
                }
                q.start += d - a;
                if out.len() >= MAX_PLACEMENTS {
                    return Err(Refused::Blocks);
                }
                out.push((i, q));
            }
        }
        out.sort_by_key(|(_, p)| (p.track, p.start));
        // Joined at the seams: a piece of the same block on, from where
        // its clip had got to. Blocks placed apart stay apart.
        let mut joined: Vec<(usize, Placement)> = Vec::with_capacity(out.len());
        for (i, q) in out {
            if let Some((j, p)) = joined.last_mut() {
                let meets = *j == i && p.end() == q.start && seams.contains(&q.start);
                if meets {
                    if let Some(clip) = p.clip(tracks, &self.clips) {
                        if p.from(Some(clip), p.end()).offset == q.offset {
                            p.length += q.length;
                            continue;
                        }
                    }
                }
            }
            joined.push((i, q));
        }
        Ok(joined.into_iter().map(|(_, p)| p).collect())
    }

    /// The markers, moved (or copied) with their parts.
    fn edit_markers(&self, parts: &[Part]) -> Result<Vec<Marker>, Refused> {
        let mut out: Vec<Marker> = Vec::with_capacity(self.markers.len());
        for part in parts {
            let Part::Piece {
                from: a,
                end,
                to: d,
            } = *part
            else {
                continue;
            };
            let b = end.unwrap_or(Tick::MAX);
            for m in self.markers.iter().filter(|m| (a..b).contains(&m.at)) {
                if out.len() >= MAX_MARKERS {
                    return Err(Refused::Markers);
                }
                out.push(Marker {
                    at: m.at - a + d,
                    ..m.clone()
                });
            }
        }
        out.sort_by_key(|m| m.at);
        Ok(out)
    }

    /// The morph's rows: each part starts with those in force at its start,
    /// a gap with those before it; only real changes stay.
    fn edit_morphs(&self, parts: &[Part]) -> Result<Vec<MorphRows>, Refused> {
        let before = |t: Tick| {
            let k = self.morphs.partition_point(|m| m.at < t);
            k.checked_sub(1).map(|j| self.morphs[j])
        };
        let mut out: Vec<MorphRows> = Vec::with_capacity(self.morphs.len() + parts.len());
        let mut push = |m: MorphRows| match out.last_mut() {
            Some(last) if last.at >= m.at => *last = m,
            _ => out.push(m),
        };
        for part in parts {
            match *part {
                Part::Gap { from, to, .. } => {
                    if let Some(r) = before(from) {
                        push(MorphRows { at: to, ..r });
                    }
                }
                Part::Piece {
                    from: a,
                    end,
                    to: d,
                } => {
                    if let Some(r) = self.morph_rows_at(a) {
                        push(MorphRows { at: d, ..r });
                    }
                    let b = end.unwrap_or(Tick::MAX);
                    for m in self.morphs.iter().filter(|m| a < m.at && m.at < b) {
                        push(MorphRows {
                            at: m.at - a + d,
                            ..*m
                        });
                    }
                }
            }
        }
        out.dedup_by(|b, a| (a.a, a.b) == (b.a, b.b));
        if out.len() > MAX_MORPHS {
            return Err(Refused::Morphs);
        }
        Ok(out)
    }

    /// The tempo points: see the module notes.
    fn edit_tempo(
        &self,
        parts: &[Part],
        seams: &[Tick],
        bpm: f64,
    ) -> Result<Vec<TempoPoint>, Refused> {
        let line = Line {
            points: &self.tempo,
            bpm,
        };
        let mut out: Vec<New> = Vec::with_capacity(self.tempo.len() + 2 * parts.len());
        for (k, part) in parts.iter().enumerate() {
            match *part {
                Part::Gap { from, to, .. } => out.push(New {
                    p: TempoPoint {
                        at: to,
                        bpm: line.before(from),
                        ramp: false,
                    },
                    added: true,
                    src: from,
                }),
                Part::Piece {
                    from: a,
                    end,
                    to: d,
                } => {
                    let (start, ramp) = line.from(a);
                    let own = self.tempo.iter().any(|p| p.at == a);
                    out.push(New {
                        p: TempoPoint {
                            at: d,
                            bpm: start,
                            ramp,
                        },
                        added: !own,
                        src: a,
                    });
                    let b = end.unwrap_or(Tick::MAX);
                    out.extend(
                        self.tempo
                            .iter()
                            .filter(|p| a < p.at && p.at < b)
                            .map(|p| New {
                                p: TempoPoint {
                                    at: p.at - a + d,
                                    ..*p
                                },
                                added: false,
                                src: p.at,
                            }),
                    );
                    // A ramp running into the part's end.
                    let Some(b) = end else {
                        continue;
                    };
                    if !line.glides_into(b) {
                        continue;
                    }
                    let next = match parts.get(k + 1) {
                        Some(Part::Piece { from, .. }) => line.from(*from).0,
                        Some(Part::Gap { from, .. }) => line.before(*from),
                        None => continue,
                    };
                    if next == line.before(b) {
                        // It arrives at what follows, as it did.
                        continue;
                    }
                    // Its last 1/16 holds, since the next part's point takes
                    // the cut, and the ramp arrives at the hold's tempo: one
                    // worked out so the cut lands when it did.
                    let cut = d + (b - a);
                    let hold = cut - RAMP_STEP;
                    let Some(last) = out.last_mut() else {
                        continue;
                    };
                    let took = ramp_time((b - last.src) as f64, last.p.bpm, line.before(b));
                    if last.p.at + RAMP_STEP <= hold {
                        let len = (b - RAMP_STEP - last.src) as f64;
                        let bpm = arriving(len, last.p.bpm, took);
                        out.push(New {
                            p: TempoPoint {
                                at: hold,
                                bpm,
                                ramp: false,
                            },
                            added: true,
                            src: b - RAMP_STEP,
                        });
                    } else {
                        // Less than two 1/16s of it: all of it holds.
                        last.p.bpm = (b - last.src) as f64 / took;
                        last.p.ramp = false;
                    }
                }
            }
        }
        // Left out: a point the edit added that changes nothing, and one on
        // a seam that only carries on the tempo or the ramp through it.
        let mut kept: Vec<TempoPoint> = Vec::with_capacity(out.len());
        for (i, n) in out.iter().enumerate() {
            let p = n.p;
            let (was, glides) = kept.last().map_or((bpm, false), |k| (k.bpm, k.ramp));
            let seam = seams.contains(&p.at);
            if (n.added || seam) && !p.ramp && !glides && was == p.bpm {
                continue;
            }
            if seam && p.ramp && glides {
                let prev = kept[kept.len() - 1];
                if let Some(next) = out.get(i + 1).map(|n| n.p) {
                    if glide(prev, next, p.at) == p.bpm {
                        continue;
                    }
                }
            }
            kept.push(p);
        }
        if kept.len() > MAX_TEMPO_POINTS {
            return Err(Refused::TempoPoints);
        }
        if let Some(w) = kept.windows(2).find(|w| w[1].at - w[0].at < MIN_SPAN_TICKS) {
            return Err(Refused::TempoTooClose(w[1].at));
        }
        Ok(kept)
    }
}

/// A tempo point of the edited song: whether the edit added it, and the
/// tick of the song as it was that it stands for.
#[derive(Clone, Copy, Debug)]
struct New {
    p: TempoPoint,
    added: bool,
    src: Tick,
}

/// The song's tempo as a line over its ticks: the project's tempo `bpm`
/// before the first point.
struct Line<'a> {
    points: &'a [TempoPoint],
    bpm: f64,
}

/// The tempo on the glide from point `a` to point `b` at tick `t`.
fn glide(a: TempoPoint, b: TempoPoint, t: Tick) -> f64 {
    a.bpm + (b.bpm - a.bpm) * (t - a.at) as f64 / (b.at - a.at) as f64
}

impl Line<'_> {
    /// The tempo from tick `t` on (a point on `t` counts), and whether it
    /// glides on from there to a later point.
    fn from(&self, t: Tick) -> (f64, bool) {
        let i = self.points.partition_point(|p| p.at <= t);
        let Some(a) = i.checked_sub(1).map(|j| self.points[j]) else {
            return (self.bpm, false);
        };
        match self.points.get(i) {
            Some(&b) if a.ramp => (glide(a, b, t), true),
            _ => (a.bpm, false),
        }
    }

    /// The tempo just before tick `t`: what a ramp has reached there.
    fn before(&self, t: Tick) -> f64 {
        let i = self.points.partition_point(|p| p.at < t);
        let Some(a) = i.checked_sub(1).map(|j| self.points[j]) else {
            return self.bpm;
        };
        match self.points.get(i) {
            Some(&b) if a.ramp => glide(a, b, t),
            _ => a.bpm,
        }
    }

    /// Whether a ramp glides into tick `t` from before it, to a point on or
    /// after it.
    fn glides_into(&self, t: Tick) -> bool {
        let i = self.points.partition_point(|p| p.at < t);
        i > 0 && self.points[i - 1].ramp && i < self.points.len()
    }
}

/// How long a glide from tempo `from` to tempo `to` over `len` ticks
/// takes, in ticks at 1 BPM (the ticks over the logarithmic mean of the
/// two tempos).
fn ramp_time(len: f64, from: f64, to: f64) -> f64 {
    let r = to / from;
    if (r - 1.0).abs() < 1e-12 {
        2.0 * len / (from + to)
    } else {
        len * r.ln() / (to - from)
    }
}

/// The tempo a glide from `from` over `len` ticks arrives at, then holds
/// for a 1/16, so the two take `took` ([`ramp_time`]'s measure) between
/// them. The longer it takes the lower the tempo, so a search between the
/// slowest and fastest tempos finds it.
fn arriving(len: f64, from: f64, took: f64) -> f64 {
    let step = RAMP_STEP as f64;
    let (mut lo, mut hi) = (MIN_BPM, MAX_BPM);
    for _ in 0..100 {
        let mid = (lo + hi) / 2.0;
        if ramp_time(len, from, mid) + step / mid > took {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) / 2.0
}

/// The song's lanes edited as a clip's lanes are, by `mpfree_moves`' own
/// cut, insert and copy, over the lanes' whole length.
fn edit_lanes(lanes: &Moves, edit: TimeEdit) -> Result<Moves, MovesError> {
    let mut m = lanes.clone();
    if m.is_empty() {
        return Ok(m);
    }
    let len = SONG_LANE_LEN;
    // Every tick is inside the lanes' length: `TimeEdit::parts` checked.
    let t = |x: Tick| u32::try_from(x).map_err(|_| MovesError::Bad);
    match edit {
        TimeEdit::Insert { at, len: n } => m.insert(t(at)?, t(n)?)?,
        TimeEdit::Delete { from, to } => m.cut(t(from)?, t(to)?, len)?,
        TimeEdit::Copy { from, to, at } => copy_lanes(&mut m, t(from)?, t(to)?, t(at)?)?,
        TimeEdit::Move { from, to, at } => {
            let (from, to, at) = (t(from)?, t(to)?, t(at)?);
            let w = to - from;
            if at < from {
                copy_lanes(&mut m, from, to, at)?;
                m.cut(from + w, to + w, len)?;
            } else if at > to {
                copy_lanes(&mut m, from, to, at)?;
                m.cut(from, to, len)?;
            }
        }
    }
    m.check(len)?;
    Ok(m)
}

/// Room at `at` and a copy of `from..to` (as it was) into it.
fn copy_lanes(m: &mut Moves, from: u32, to: u32, at: u32) -> Result<(), MovesError> {
    let (w, len) = (to - from, SONG_LANE_LEN);
    m.insert(at, w)?;
    if at <= from {
        m.copy_range(from + w, to + w, at, len)
    } else if at >= to {
        m.copy_range(from, to, at, len)
    } else {
        // The room opened inside the range: its two halves either side.
        m.copy_range(from, at, at, len)?;
        m.copy_range(at + w, to + w, at + (at - from), len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrange::MORPH_LANE;
    use crate::mpfree_moves::{Kind, Map, Target};
    use crate::mpfree_param::ParamAddr;
    use crate::{Clip, ClipId, Project};
    use mpfree_transport::time::TempoMap;

    const BEAT: Tick = 960;
    const BAR: Tick = 3840;
    const BPM: f64 = 100.0;
    const RATE: u32 = 48_000;
    /// Where the samples stop: past the song's end, so the last part is
    /// heard on into silence too.
    const SONG: Tick = 20 * BAR;
    /// The song's second lane, besides the morph's.
    const OTHER: Target = Target::Fixed(ParamAddr::morph_pos(1));

    fn b(bars: Tick) -> Tick {
        bars * BAR
    }

    /// The song every edit is tried on. Drums: a one-bar beat for 8 bars,
    /// a three-beat fill for 5 (so its blocks cut partway into it), the
    /// beat again for 3. Keys: a two-bar pad for 6 bars, then from bar 8
    /// to the end. Loops A (an audio track): a loop cell's block in bars 3
    /// to 10. Markers A to D
    /// every 4 bars. Tempo: 100 (the project's), 120 at bar 3 gliding to
    /// 140 at bar 7, 90 from bar 11, then from bar 12 gliding to 160 at
    /// bar 15. The morph lane glides from bar 2 to 6 and ends at 10; the
    /// other lane glides from bar 4 to bar 10 and ends at bar 16. The
    /// morph's rows change at bars 1, 6 and 10.
    fn project() -> Project {
        let mut p = Project::new();
        let mut drums = Track::new("drums", 0);
        drums.slots[0] = Some(Clip::new("beat", BAR));
        drums.slots[1] = Some(Clip::new("fill", 3 * BEAT));
        let mut keys = Track::new("keys", 1);
        keys.slots[0] = Some(Clip::new("pad", 2 * BAR));
        let mut loops = Track::new_audio("Loops A", 2);
        let id = crate::LoopId(1);
        loops.slots[0] = Some(Clip::new_audio_loop(id, "Loop 1", 2 * BAR));
        p.tracks = vec![drums, keys, loops];
        p.give_clip_ids();
        let s = &mut p.arrangement;
        for (track, row, from, to) in [
            (0, 0, 0, 8),
            (0, 1, 8, 13),
            (0, 0, 13, 16),
            (1, 0, 0, 6),
            (1, 0, 7, 16),
            (2, 0, 2, 10),
        ] {
            let block = Placement::of(&p.tracks, track, row, b(from), b(to - from));
            s.place(block).unwrap();
        }
        for (at, name) in [(0, "A"), (4, "B"), (8, "C"), (12, "D")] {
            s.mark(b(at), name).unwrap();
        }
        s.set_tempo(b(2), 120.0, Some(true)).unwrap();
        s.set_tempo(b(6), 140.0, None).unwrap();
        s.set_tempo(b(10), 90.0, None).unwrap();
        s.set_tempo(b(11), 90.0, Some(true)).unwrap();
        s.set_tempo(b(14), 160.0, None).unwrap();
        let morph = [
            (b(1) as f64, 0.1, Kind::Ramp),
            (b(5) as f64, 0.9, Kind::Step),
            (b(9) as f64, 0.0, Kind::End),
        ];
        let rows = [
            MorphRows { at: 0, a: 0, b: 1 },
            MorphRows {
                at: b(5),
                a: 2,
                b: 3,
            },
            MorphRows {
                at: b(9),
                a: 1,
                b: 4,
            },
        ];
        s.put_morph(&morph, &rows, 0, b(10)).unwrap();
        let other = [
            (b(3) as f64, 0.2, Kind::Ramp),
            (b(9) as f64, 0.8, Kind::Step),
            (b(15) as f64, 0.0, Kind::End),
        ];
        s.lanes
            .put(OTHER, Map::linear(0.0, 1.0), &other, SONG_LANE_LEN)
            .unwrap();
        s.check(&p.tracks).unwrap();
        p
    }

    /// What the song plays at tick `t`: on each track the clip and the
    /// tick of it, the morph's rows, and each lane's knob position.
    #[derive(Debug, PartialEq)]
    struct Heard {
        blocks: Vec<Option<(ClipId, Tick)>>,
        rows: Option<(usize, usize)>,
        lanes: [Option<f32>; 2],
    }

    fn heard(s: &Arrangement, tracks: &[Track], t: Tick) -> Heard {
        let blocks = (0..tracks.len())
            .map(|track| {
                let p = &s.placements[s.at(track, t)?];
                let clip = p.clip(tracks, &s.clips)?;
                Some((p.clip, (p.offset + t - p.start).rem_euclid(clip.length)))
            })
            .collect();
        let lane = |target| s.lanes.lane(target).and_then(|l| l.state_at(t as f64));
        Heard {
            blocks,
            rows: s.morph_rows_at(t).map(|m| (m.a, m.b)),
            lanes: [lane(MORPH_LANE), lane(OTHER)],
        }
    }

    /// Two hearings the same, the lanes to a hair (a ramp cut by the edit
    /// keeps its line, worked out again).
    fn same(a: &Heard, b: &Heard) -> bool {
        let near = |x: Option<f32>, y: Option<f32>| match (x, y) {
            (Some(x), Some(y)) => (x - y).abs() < 1e-4,
            (x, y) => x == y,
        };
        a.blocks == b.blocks
            && a.rows == b.rows
            && a.lanes.iter().zip(&b.lanes).all(|(&x, &y)| near(x, y))
    }

    fn map(s: &Arrangement) -> TempoMap {
        s.tempo_map(BPM).unwrap()
    }

    /// Every part of the edited song `new` plays what its stretch of `old`
    /// did: every 1/16 the blocks, rows and lanes, and in time: the frames
    /// from the part's start to each 1/16 within 8 (a ramp cut by the part's
    /// end runs a little ahead of its old line before its held 1/16), and
    /// to the part's end within one, so it lasts as long as it did. A gap
    /// plays no block and holds the rows, lanes and tempo before it. The
    /// largest gap in time, in frames.
    fn plays_its_parts(
        old: &Arrangement,
        new: &Arrangement,
        tracks: &[Track],
        edit: TimeEdit,
    ) -> f64 {
        let (om, nm) = (map(old), map(new));
        let parts = edit.parts().unwrap();
        let mut worst: f64 = 0.0;
        for (k, part) in parts.iter().enumerate() {
            match *part {
                Part::Piece {
                    from: a,
                    end,
                    to: d,
                } => {
                    let b = end.unwrap_or(SONG).min(SONG.max(a));
                    let frames = |m: &TempoMap, at: Tick, x: Tick| {
                        m.frames_between(at as f64, (at + x) as f64, RATE)
                    };
                    let mut t = a;
                    while t < b {
                        let (was, now) = (heard(old, tracks, t), heard(new, tracks, d + t - a));
                        assert!(
                            same(&was, &now),
                            "{edit:?}: {t} → {}: {was:?} {now:?}",
                            d + t - a
                        );
                        let (f0, f1) = (frames(&om, a, t - a), frames(&nm, d, t - a));
                        worst = worst.max((f0 - f1).abs());
                        assert!(
                            (f0 - f1).abs() <= 48.0,
                            "{edit:?}: frames {a}+{}: {f0} {f1}",
                            t - a
                        );
                        t += BEAT / 4;
                    }
                    if end.is_some() {
                        let (f0, f1) = (frames(&om, a, b - a), frames(&nm, d, b - a));
                        assert!(
                            (f0 - f1).abs() <= 1.0,
                            "{edit:?}: the part {a}..{b} lasts {f0} frames, now {f1}"
                        );
                    }
                    assert!(
                        (om.bpm_at(a) - nm.bpm_at(d)).abs() < 1e-9,
                        "{edit:?}: the tempo at {a} goes with it"
                    );
                }
                Part::Gap { from, to } => {
                    let Some(Part::Piece { to: next, .. }) = parts.get(k + 1) else {
                        panic!("a gap is followed");
                    };
                    let line = Line {
                        points: &old.tempo,
                        bpm: BPM,
                    };
                    let was = heard(old, tracks, from);
                    for t in (to..*next).step_by((BEAT / 4) as usize) {
                        let now = heard(new, tracks, t);
                        assert!(now.blocks.iter().all(Option::is_none), "{edit:?}: {t}");
                        assert_eq!(now.rows, old.morph_rows_at(from - 1).map(|m| (m.a, m.b)));
                        assert!(
                            same(
                                &Heard {
                                    blocks: now.blocks.clone(),
                                    ..was
                                },
                                &now
                            ) || from == 0
                        );
                        assert!(
                            (nm.bpm_at(t) - line.before(from)).abs() < 1e-9,
                            "{edit:?}: {t}"
                        );
                    }
                }
            }
        }
        worst
    }

    /// Edits that cut through everything: blocks, ramps, lanes, markers.
    fn edits() -> Vec<TimeEdit> {
        vec![
            TimeEdit::Insert {
                at: b(4) + BEAT,
                len: b(2),
            },
            TimeEdit::Insert { at: 0, len: b(1) },
            TimeEdit::Insert {
                at: b(12) + 2 * BEAT,
                len: 3 * BEAT,
            },
            TimeEdit::Delete {
                from: b(3),
                to: b(5),
            },
            TimeEdit::Delete {
                from: b(9),
                to: b(12),
            },
            TimeEdit::Delete {
                from: b(8) + BEAT,
                to: b(13) + 3 * BEAT,
            },
            TimeEdit::Move {
                from: b(4),
                to: b(8),
                at: b(12),
            },
            TimeEdit::Move {
                from: b(8),
                to: b(12),
                at: 0,
            },
            TimeEdit::Move {
                from: b(12),
                to: b(13) + 2 * BEAT,
                at: b(3) + BEAT,
            },
            TimeEdit::Move {
                from: b(2) + BEAT,
                to: b(5),
                at: b(17),
            },
            TimeEdit::Copy {
                from: b(4),
                to: b(8),
                at: b(16),
            },
            TimeEdit::Copy {
                from: b(2),
                to: b(6),
                at: b(4),
            },
            TimeEdit::Copy {
                from: b(11),
                to: b(13),
                at: b(3),
            },
        ]
    }

    /// The pieces the audio tracks' regions follow (Arrange 3.4): the old
    /// song's stretches in the edited song's order, an insert's room in
    /// none, and empty ones left out.
    #[test]
    fn the_pieces_say_where_each_stretch_of_the_old_song_goes() {
        let pieces = |e: TimeEdit| e.pieces().unwrap();
        assert_eq!(
            pieces(TimeEdit::Insert {
                at: b(2),
                len: b(1)
            }),
            [(0, Some(b(2)), 0), (b(2), None, b(3))]
        );
        assert_eq!(
            pieces(TimeEdit::Insert { at: 0, len: b(1) }),
            [(0, None, b(1))]
        );
        assert_eq!(
            pieces(TimeEdit::Delete {
                from: b(2),
                to: b(4)
            }),
            [(0, Some(b(2)), 0), (b(4), None, b(2))]
        );
        assert_eq!(
            pieces(TimeEdit::Move {
                from: b(4),
                to: b(6),
                at: b(1)
            }),
            [
                (0, Some(b(1)), 0),
                (b(4), Some(b(6)), b(1)),
                (b(1), Some(b(4)), b(3)),
                (b(6), None, b(6))
            ]
        );
        assert_eq!(
            pieces(TimeEdit::Copy {
                from: b(1),
                to: b(2),
                at: b(2)
            }),
            [
                (0, Some(b(2)), 0),
                (b(1), Some(b(2)), b(2)),
                (b(2), None, b(3))
            ]
        );
        assert_eq!(
            TimeEdit::Delete {
                from: b(2),
                to: b(2)
            }
            .pieces(),
            Err(Refused::BadRange)
        );
    }

    #[test]
    fn every_kind_of_edit_moves_everything_with_its_part() {
        let p = project();
        for edit in edits() {
            let new = p.arrangement.edited(&p.tracks, edit, BPM).unwrap();
            new.check(&p.tracks).unwrap();
            let worst = plays_its_parts(&p.arrangement, &new, &p.tracks, edit);
            println!("{edit:?}: at most {worst:.1} frames off");
            // A block stays on its own lane, a loop cell's on its audio
            // track's.
            for q in &new.placements {
                let was = p
                    .arrangement
                    .placements
                    .iter()
                    .find(|o| o.clip == q.clip)
                    .unwrap();
                assert_eq!(q.track, was.track, "{edit:?}");
            }
        }
    }

    /// Markers go with their part and keep their names; a deleted
    /// range's go and a copied range's are copied.
    #[test]
    fn markers_go_with_their_part_and_keep_their_names() {
        let p = project();
        let names = |e: TimeEdit| {
            let s = p.arrangement.edited(&p.tracks, e, BPM).unwrap();
            s.markers
                .iter()
                .map(|m| (m.at / BAR, m.name.clone()))
                .collect::<Vec<_>>()
        };
        let n = |v: &[(Tick, &str)]| {
            v.iter()
                .map(|&(a, s)| (a, s.to_string()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(TimeEdit::Insert {
                at: b(4),
                len: b(2)
            }),
            n(&[(0, "A"), (6, "B"), (10, "C"), (14, "D")])
        );
        assert_eq!(
            names(TimeEdit::Delete {
                from: b(4),
                to: b(8)
            }),
            n(&[(0, "A"), (4, "C"), (8, "D")])
        );
        assert_eq!(
            names(TimeEdit::Move {
                from: b(4),
                to: b(8),
                at: b(12)
            }),
            n(&[(0, "A"), (4, "C"), (8, "B"), (12, "D")])
        );
        assert_eq!(
            names(TimeEdit::Copy {
                from: b(4),
                to: b(8),
                at: b(8)
            }),
            n(&[(0, "A"), (4, "B"), (8, "B"), (12, "C"), (16, "D")])
        );
    }

    /// A delete inside a ramp: the ramp keeps its curve up to the cut,
    /// its last 1/16 held so the cut lands on the frame it did, and the
    /// rest takes the tempo it had reached at the far edge with it.
    #[test]
    fn a_ramp_cut_by_a_delete_keeps_its_curve() {
        let p = project();
        let mut s = Arrangement::default();
        s.set_tempo(b(2), 120.0, Some(true)).unwrap();
        s.set_tempo(b(6), 140.0, None).unwrap();
        let new = s
            .edited(
                &p.tracks,
                TimeEdit::Delete {
                    from: b(3),
                    to: b(5),
                },
                BPM,
            )
            .unwrap();
        let at: Vec<(Tick, bool)> = new.tempo.iter().map(|p| (p.at, p.ramp)).collect();
        assert_eq!(
            at,
            [
                (b(2), true),
                (b(3) - RAMP_STEP, false),
                (b(3), true),
                (b(4), false)
            ]
        );
        // 124.6875 a 1/16 before bar 4, 125 at it: the hold is about there.
        let hold = new.tempo[1].bpm;
        assert!(hold > 124.6 && hold < 125.0, "{hold}");
        assert_eq!((new.tempo[2].bpm, new.tempo[3].bpm), (135.0, 140.0));
        let (om, nm) = (map(&s), map(&new));
        let frames = |m: &TempoMap, a: Tick, b: Tick| m.frames_between(a as f64, b as f64, RATE);
        // The cut lands on its frame, and what follows plays as it did.
        let (was, now) = (frames(&om, 0, b(3)), frames(&nm, 0, b(3)));
        assert!((was - now).abs() < 1.0, "{was} {now}");
        let (was, now) = (frames(&om, b(5), b(7)), frames(&nm, b(3), b(5)));
        assert!((was - now).abs() < 1.0, "{was} {now}");
        // Up to the hold the ramp keeps its line, to a tenth of a BPM.
        for t in (b(2)..b(3) - RAMP_STEP).step_by(RAMP_STEP as usize) {
            assert!((om.bpm_at(t) - nm.bpm_at(t)).abs() < 0.1, "{t}");
        }
    }

    /// An insert inside a ramp: the ramp arrives at the room, the room
    /// holds the tempo it had reached, and the ramp goes on after it.
    #[test]
    fn an_insert_in_a_ramp_holds_its_tempo_through_the_room() {
        let p = project();
        let mut s = Arrangement::default();
        s.set_tempo(b(2), 120.0, Some(true)).unwrap();
        s.set_tempo(b(6), 140.0, None).unwrap();
        let new = s
            .edited(
                &p.tracks,
                TimeEdit::Insert {
                    at: b(4),
                    len: b(2),
                },
                BPM,
            )
            .unwrap();
        let points: Vec<(Tick, f64, bool)> =
            new.tempo.iter().map(|p| (p.at, p.bpm, p.ramp)).collect();
        assert_eq!(
            points,
            [
                (b(2), 120.0, true),
                (b(4), 130.0, false),
                (b(6), 130.0, true),
                (b(8), 140.0, false)
            ]
        );
    }

    /// An insert and the delete of what it inserted leave the song playing
    /// as it did: the markers, tempo points and rows as they were, the
    /// lanes heard the same, and the blocks too, but those the insert cut
    /// are two (blocks placed apart stay apart).
    #[test]
    fn an_insert_and_its_delete_leave_the_song_playing_as_it_did() {
        let p = project();
        let old = &p.arrangement;
        for (at, len) in [
            (b(4) + BEAT, b(2)),
            (0, b(1)),
            (b(8) + 5 * 240, 7 * 240),
            (b(12) + 2 * BEAT, 3 * BEAT),
            (b(3), b(1)),
            (b(30), b(1)),
        ] {
            let ins = old
                .edited(&p.tracks, TimeEdit::Insert { at, len }, BPM)
                .unwrap();
            let back = ins
                .edited(
                    &p.tracks,
                    TimeEdit::Delete {
                        from: at,
                        to: at + len,
                    },
                    BPM,
                )
                .unwrap();
            // The blocks as they were, but those the insert cut: two,
            // the second playing on from where the first got to.
            let cut: Vec<Placement> = old
                .placements
                .iter()
                .flat_map(|q| {
                    if q.start < at && at < q.end() {
                        let rest = q.from(q.clip(&p.tracks, &old.clips), at);
                        vec![
                            Placement {
                                length: at - q.start,
                                ..q.clone()
                            },
                            rest,
                        ]
                    } else {
                        vec![q.clone()]
                    }
                })
                .collect();
            assert_eq!(back.placements, cut, "{at} {len}");
            assert_eq!(back.markers, old.markers, "{at} {len}");
            assert_eq!(back.tempo, old.tempo, "{at} {len}");
            assert_eq!(back.morphs, old.morphs, "{at} {len}");
            for t in (0..SONG).step_by(60) {
                let (was, now) = (heard(old, &p.tracks, t), heard(&back, &p.tracks, t));
                assert!(same(&was, &now), "{at} {len}: {t}: {was:?} {now:?}");
            }
        }
    }

    /// A long block of a one-bar loop stays one block when whole bars move
    /// or go; cut partway into a bar, its parts play on in phase.
    #[test]
    fn a_long_block_stays_one_where_whole_bars_move() {
        let p = project();
        let drums = |s: &Arrangement| -> Vec<(Tick, Tick, Tick)> {
            s.on_track(0)
                .iter()
                .map(|q| (q.start, q.length, q.offset))
                .collect()
        };
        let mut s = p.arrangement.clone();
        s.placements.retain(|q| q.track != 0);
        s.place(Placement::of(&p.tracks, 0, 0, 0, b(16))).unwrap();
        for edit in [
            TimeEdit::Move {
                from: b(4),
                to: b(8),
                at: b(12),
            },
            TimeEdit::Delete {
                from: b(4),
                to: b(6),
            },
            TimeEdit::Copy {
                from: b(2),
                to: b(3),
                at: b(5),
            },
        ] {
            let new = s.edited(&p.tracks, edit, BPM).unwrap();
            let len = match edit {
                TimeEdit::Delete { .. } => b(14),
                TimeEdit::Copy { .. } => b(17),
                _ => b(16),
            };
            assert_eq!(drums(&new), [(0, len, 0)], "{edit:?}");
        }
        // Half a bar moved: cut where it left and where it went.
        let new = s
            .edited(
                &p.tracks,
                TimeEdit::Move {
                    from: b(4),
                    to: b(4) + 2 * BEAT,
                    at: b(8),
                },
                BPM,
            )
            .unwrap();
        // The half bar moved on meets the bars that moved up exactly where
        // they had got to (half a bar out from the bar lines), so it joins
        // them; it doesn't meet the bars after it.
        assert_eq!(
            drums(&new),
            [(0, b(4), 0), (b(4), b(4), 2 * BEAT), (b(8), b(8), 0)]
        );
    }

    /// Where each tick of the song goes, for what follows the edit about.
    #[test]
    fn each_tick_has_its_place_after_the_edit() {
        let p = project();
        for edit in edits() {
            let parts = edit.parts().unwrap();
            for t in (0..SONG).step_by(BEAT as usize) {
                let place = edit.place_of(t);
                // The piece that is the tick's own (not a copy) says so.
                let own = parts.iter().rev().find_map(|part| match *part {
                    Part::Piece { from, end, to } if from <= t && end.is_none_or(|e| t < e) => {
                        let copied = matches!(edit, TimeEdit::Copy { from: f, to: e, at }
                            if from == f && end == Some(e) && to == at);
                        (!copied).then_some(t - from + to)
                    }
                    _ => None,
                });
                assert_eq!(place, own, "{edit:?} {t}");
            }
            let _ = &p;
        }
        assert_eq!(
            TimeEdit::Delete {
                from: b(1),
                to: b(2)
            }
            .place_of(b(1)),
            None
        );
    }

    /// Refused, with what refused; the song is as it was.
    #[test]
    fn an_edit_that_cant_be_made_changes_nothing_and_says_why() {
        let p = project();
        let s = &p.arrangement;
        let before = s.clone();
        let refused = |s: &Arrangement, e| s.edited(&p.tracks, e, BPM).unwrap_err();
        for e in [
            TimeEdit::Insert { at: -1, len: BAR },
            TimeEdit::Insert { at: 0, len: 0 },
            TimeEdit::Delete { from: BAR, to: BAR },
            TimeEdit::Delete {
                from: 0,
                to: Tick::from(SONG_LANE_LEN) + 1,
            },
            TimeEdit::Move {
                from: 0,
                to: b(4),
                at: b(2),
            },
            TimeEdit::Copy {
                from: 0,
                to: b(4),
                at: Tick::from(SONG_LANE_LEN),
            },
        ] {
            assert_eq!(refused(s, e), Refused::BadRange, "{e:?}");
        }
        // A tempo point a 1/32 before an edge, and another tempo after it.
        let mut t = Arrangement::default();
        t.set_tempo(b(3) - 120, 100.0, None).unwrap();
        t.set_tempo(b(4), 140.0, None).unwrap();
        assert_eq!(
            refused(
                &t,
                TimeEdit::Delete {
                    from: b(3),
                    to: b(4) + BEAT
                }
            ),
            Refused::TempoTooClose(b(3))
        );
        // Too many of anything.
        let mut many = Arrangement::default();
        for k in 0..3000 {
            many.place(Placement::of(&p.tracks, 0, 0, k, 1)).unwrap();
        }
        assert_eq!(
            refused(
                &many,
                TimeEdit::Copy {
                    from: 0,
                    to: 3000,
                    at: 3000
                }
            ),
            Refused::Blocks
        );
        let mut many = Arrangement::default();
        for k in 0..200 {
            many.mark(k * BAR, "x").unwrap();
        }
        assert_eq!(
            refused(
                &many,
                TimeEdit::Copy {
                    from: 0,
                    to: b(200),
                    at: 0
                }
            ),
            Refused::Markers
        );
        let mut many = Arrangement::default();
        for k in 0..20 {
            many.set_tempo(k * BAR, 100.0 + k as f64, None).unwrap();
        }
        assert_eq!(
            refused(
                &many,
                TimeEdit::Copy {
                    from: 0,
                    to: b(15),
                    at: b(20)
                }
            ),
            Refused::TempoPoints
        );
        assert_eq!(s, &before);
        // A move to where it is changes nothing.
        let e = TimeEdit::Move {
            from: b(4),
            to: b(8),
            at: b(8),
        };
        assert!(e.moves_nothing());
        assert_eq!(&s.edited(&p.tracks, e, BPM).unwrap(), s);
    }
}
