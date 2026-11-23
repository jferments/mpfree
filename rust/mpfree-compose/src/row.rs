//! The row as one picture (revision 3, S2): every part of a clip grid
//! row as it plays, the tune's phrase plan read back from its notes, and
//! what that leaves free, for Add a part to play with.
//!
//! - Each part's notes over the row's length, and the drums by kick,
//!   snare, hats and the rest: the loudest hit on each 1/16 step.
//! - The **lead**: the tune, the melody part with the most movement
//!   unless one is picked, as one line.
//! - The **plan**: the tune's landings (where its phrases end), its high
//!   point and its last note, read back from its notes; where it rests;
//!   how much goes on in each bar. With no tune (a beat alone), the plan
//!   is S1's phrase shape over the row's bars.
//!
//! Every part added reads the same plan, so a bass lands where the tune
//! lands and chords answer where it rests; a part kept joins the row, so
//! the next part added reads it too.

use mpfree_context::chord::{Chord, ChordTimeline};
use mpfree_context::groove::{GrooveTemplate, Lane, STEP};
use mpfree_context::key::{Key, KeyReading, Mode};
use mpfree_context::PartRole;
use mpfree_transport::time::{Meter, Tick};

use crate::motif::{boundaries, line};
use crate::realize::PlainNote;
use crate::rhythm::Grid;
use crate::settings::Ending;
use crate::sketch::{gestures, AnchorKind, Gesture};

/// A drum hit as heard, in row ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrumHit {
    pub at: Tick,
    pub lane: Lane,
    pub velocity: u8,
}

/// What a part plays, as heard over the row.
#[derive(Clone, Debug, PartialEq)]
pub enum Heard {
    Notes(Vec<PlainNote>),
    Hits(Vec<DrumHit>),
}

/// One part of the row: what it is in the music, and what it plays.
#[derive(Clone, Debug, PartialEq)]
pub struct RowPart {
    pub role: PartRole,
    pub heard: Heard,
}

impl RowPart {
    /// Its notes (none for drums).
    pub fn notes(&self) -> &[PlainNote] {
        match &self.heard {
            Heard::Notes(n) => n,
            Heard::Hits(_) => &[],
        }
    }

    /// Its hits (none for a pitched part).
    pub fn hits(&self) -> &[DrumHit] {
        match &self.heard {
            Heard::Notes(_) => &[],
            Heard::Hits(h) => h,
        }
    }
}

/// What a row is heard from.
#[derive(Clone, Copy)]
pub struct ToHear<'a> {
    /// Every part, each already looped over the row's length (as
    /// [`mod@crate::read`] hears clips).
    pub parts: &'a [RowPart],
    /// The row's length in ticks; heard as whole bars.
    pub length: Tick,
    pub meter: Meter,
    pub key: &'a KeyReading,
    pub played_pcs: u16,
    /// The row's chords: the one harmony answer over its length.
    pub chords: &'a ChordTimeline,
    pub groove: Option<&'a GrooveTemplate>,
    /// The part to hear as the tune, else the melody with the most
    /// movement.
    pub lead: Option<usize>,
}

/// A moment the plan marks: a landing, the high point or the last note.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark {
    pub kind: AnchorKind,
    /// In 1/16 steps from the row's start.
    pub at: i32,
    pub len: i32,
    /// The tune's note there, when the plan was read from a tune.
    pub pitch: Option<u8>,
}

/// What the row does as a whole: the plan every added part reads.
#[derive(Clone, Debug, PartialEq)]
pub struct RowPlan {
    /// Landings, the high point and the last note, in time order.
    pub marks: Vec<Mark>,
    /// Where the tune rests a beat or more, as `(from, to)` in steps. A
    /// rest across the loop's end is two: to the end, and from the start.
    pub rests: Vec<(i32, i32)>,
    /// How much goes on in each bar, 0 to 1.
    pub energy: Vec<f32>,
    /// Read back from a tune; else S1's phrase shape over a beat.
    pub from_tune: bool,
}

impl RowPlan {
    /// The high point, if the plan has one.
    pub fn peak(&self) -> Option<Mark> {
        self.marks
            .iter()
            .copied()
            .find(|m| m.kind == AnchorKind::Peak)
    }

    /// Whether the tune rests at `step`.
    pub fn resting(&self, step: i32) -> bool {
        self.rests.iter().any(|&(a, b)| a <= step && step < b)
    }

    /// The mark that starts at `step`.
    pub fn mark_at(&self, step: i32) -> Option<Mark> {
        self.marks.iter().copied().find(|m| m.at == step)
    }

    /// The marks starting in `[from, to)`.
    pub fn marks_in(&self, from: i32, to: i32) -> impl Iterator<Item = Mark> + '_ {
        self.marks
            .iter()
            .copied()
            .filter(move |m| from <= m.at && m.at < to)
    }

    /// Bar `bar`'s energy (the last bar's past the end).
    pub fn energy_of(&self, bar: usize) -> f32 {
        self.energy
            .get(bar)
            .or(self.energy.last())
            .copied()
            .unwrap_or(0.5)
    }
}

/// The tune, as one line.
#[derive(Clone, Debug, PartialEq)]
pub struct Lead {
    /// Which of the row's parts it is.
    pub part: usize,
    /// One note at a time: the highest at each start, each cut at the
    /// next start; in time order.
    pub line: Vec<PlainNote>,
}

impl Lead {
    /// The note sounding at `t`.
    pub fn at(&self, t: Tick) -> Option<PlainNote> {
        let i = self.line.partition_point(|n| n.at <= t);
        let n = *self.line.get(i.checked_sub(1)?)?;
        (t < n.at + n.len).then_some(n)
    }

    /// Whether a note starts in `[from, to)`.
    pub fn starts_in(&self, from: Tick, to: Tick) -> bool {
        let i = self.line.partition_point(|n| n.at < from);
        self.line.get(i).is_some_and(|n| n.at < to)
    }

    /// The notes sounding anywhere in `[from, to)`.
    pub fn sounding_in(&self, from: Tick, to: Tick) -> impl Iterator<Item = &PlainNote> {
        self.line
            .iter()
            .filter(move |n| n.at < to && n.at + n.len > from)
    }

    /// The lowest note sounding in `[from, to)`.
    pub fn lowest(&self, from: Tick, to: Tick) -> Option<u8> {
        self.sounding_in(from, to).map(|n| n.pitch).min()
    }
}

/// The row as one picture.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub meter: Meter,
    pub grid: Grid,
    pub bars: usize,
    /// 1/16 steps in the row.
    pub steps: i32,
    pub length: Tick,
    /// The key notes are counted in: the key, or with no clear key the
    /// likeliest.
    pub key: Key,
    pub clear: bool,
    pub played_pcs: u16,
    pub chords: ChordTimeline,
    pub groove: Option<GrooveTemplate>,
    pub parts: Vec<RowPart>,
    /// The loudest hit of each drum on each step (0: none), kick, snare,
    /// hats, then the rest.
    drums: [Vec<u8>; 4],
    pub lead: Option<Lead>,
    pub plan: RowPlan,
}

const fn lane_index(lane: Lane) -> usize {
    match lane {
        Lane::Kick => 0,
        Lane::Snare => 1,
        Lane::Hats => 2,
        Lane::Other => 3,
    }
}

/// Notes as one line: the highest at each start, each cut at the next.
fn one_line(notes: &[PlainNote]) -> Vec<PlainNote> {
    let mut v: Vec<PlainNote> = notes.to_vec();
    v.sort_by_key(|n| (n.at, std::cmp::Reverse(n.pitch)));
    v.dedup_by_key(|n| n.at);
    for i in 0..v.len().saturating_sub(1) {
        let next = v[i + 1].at;
        v[i].len = v[i].len.min(next - v[i].at).max(1);
    }
    v
}

/// How much a line moves: its changes of note and its different notes.
fn movement(notes: &[PlainNote]) -> usize {
    let l = one_line(notes);
    let moves = l.windows(2).filter(|w| w[0].pitch != w[1].pitch).count();
    let pcs = l.iter().fold(0u16, |m, n| m | 1 << (n.pitch % 12));
    moves + pcs.count_ones() as usize
}

fn pick_lead(parts: &[RowPart], wanted: Option<usize>) -> Option<usize> {
    if let Some(i) = wanted {
        if parts.get(i).is_some_and(|p| !p.notes().is_empty()) {
            return Some(i);
        }
    }
    parts
        .iter()
        .enumerate()
        .filter(|(_, p)| p.role == PartRole::Melody && !p.notes().is_empty())
        .max_by_key(|(i, p)| (movement(p.notes()), std::cmp::Reverse(*i)))
        .map(|(i, _)| i)
}

/// Each bar's energy: the tune's notes and the drums' hits there, against
/// the busiest bar, from 0.2 up.
fn energies(lead: Option<&Lead>, drums: &[Vec<u8>; 4], grid: Grid, bars: usize) -> Vec<f32> {
    let mut e = vec![0.0f32; bars];
    if let Some(l) = lead {
        for n in &l.line {
            let b = (n.at / STEP) as i32 / grid.bar;
            if let Some(x) = e.get_mut(b as usize) {
                *x += 1.0;
            }
        }
    }
    for lane in drums {
        for (s, &v) in lane.iter().enumerate() {
            if v > 0 {
                let b = s as i32 / grid.bar;
                if let Some(x) = e.get_mut(b as usize) {
                    *x += 0.25 * f32::from(v) / 127.0;
                }
            }
        }
    }
    let top = e.iter().copied().fold(0.0f32, f32::max);
    e.iter()
        .map(|&x| if top > 0.0 { 0.2 + 0.8 * x / top } else { 0.5 })
        .collect()
}

/// A stretch of steps, `[from, to)`.
type Span = (i32, i32);

/// The plan read back from the tune: a landing where each phrase ends (a
/// rest of a beat or more after a note, or a strong break after a longer
/// note), the high point, and the last note.
fn tune_plan(lead: &Lead, key: Key, grid: Grid, steps: i32) -> Option<(Vec<Mark>, Vec<Span>)> {
    let gl = line(&lead.line, key);
    let n = gl.len();
    if n < 3 {
        return None;
    }
    let br = boundaries(&gl);
    let mean = br.iter().sum::<f64>() / br.len().max(1) as f64;
    let strong = (mean * 1.5).max(0.5);
    let gap = |i: usize| gl[i + 1].on - (gl[i].on + gl[i].len);
    let mut ends: Vec<usize> = Vec::new();
    for i in 0..n - 1 {
        let cand = gap(i) >= grid.beat || (br[i] >= strong && gl[i].len >= 2);
        if !cand {
            continue;
        }
        // Phrases of at least half a bar: of two ends closer than that,
        // the longer note lands.
        match ends.last().copied() {
            Some(j) if gl[i].on - gl[j].on < grid.half() => {
                if gl[i].len > gl[j].len {
                    *ends.last_mut().expect("an end") = i;
                }
            }
            _ => ends.push(i),
        }
    }
    let last = n - 1;
    if ends
        .last()
        .is_some_and(|&j| gl[last].on - gl[j].on < grid.half())
    {
        ends.pop();
    }
    let pitch = |i: usize| gl[i].pitch(key);
    let peak = (0..n)
        .max_by_key(|&i| (pitch(i), gl[i].len, i))
        .expect("three notes");
    let mut marks: Vec<Mark> = ends
        .iter()
        .map(|&i| Mark {
            kind: if i == peak {
                AnchorKind::Peak
            } else {
                AnchorKind::Landing
            },
            at: gl[i].on,
            len: gl[i].len,
            pitch: Some(pitch(i)),
        })
        .collect();
    if peak != last && !ends.contains(&peak) {
        marks.push(Mark {
            kind: AnchorKind::Peak,
            at: gl[peak].on,
            len: gl[peak].len,
            pitch: Some(pitch(peak)),
        });
    }
    marks.push(Mark {
        kind: AnchorKind::Close,
        at: gl[last].on,
        len: gl[last].len,
        pitch: Some(pitch(last)),
    });
    marks.sort_by_key(|m| m.at);

    let mut rests = Vec::new();
    for i in 0..n - 1 {
        if gap(i) >= grid.beat {
            rests.push((gl[i].on + gl[i].len, gl[i + 1].on));
        }
    }
    let tail_from = (gl[last].on + gl[last].len).min(steps);
    let head = gl[0].on;
    if steps - tail_from + head >= grid.beat {
        if tail_from < steps {
            rests.push((tail_from, steps));
        }
        if head > 0 {
            rests.insert(0, (0, head));
        }
    }
    Some((marks, rests))
}

/// The plan over a beat alone: S1's phrase shape over the row's bars,
/// leading back into the first. Landings and the high point on the half
/// bar, the last landing in the last bar, a breath where the shape holds
/// back and after the high point.
fn beat_plan(grid: Grid, bars: usize) -> (Vec<Mark>, Vec<Span>) {
    let shape = if bars >= 2 {
        gestures(bars, Ending::LoopsBack, false)
    } else {
        vec![Gesture::Close(Ending::LoopsBack)]
    };
    let mid = grid.half();
    let mut marks = Vec::new();
    let mut rests = Vec::new();
    for (b, g) in shape.into_iter().enumerate() {
        let start = b as i32 * grid.bar;
        let end = start + grid.bar;
        match g {
            Gesture::Reach => marks.push(Mark {
                kind: AnchorKind::Landing,
                at: start + mid,
                len: grid.beat,
                pitch: None,
            }),
            Gesture::Peak => {
                marks.push(Mark {
                    kind: AnchorKind::Peak,
                    at: start + mid,
                    len: grid.beat,
                    pitch: None,
                });
                rests.push((
                    start + mid + grid.beat,
                    (start + mid + 2 * grid.beat).min(end),
                ));
            }
            Gesture::HoldBack => rests.push((end - grid.beat, end)),
            Gesture::Close(_) => marks.push(Mark {
                kind: AnchorKind::Close,
                at: start + mid,
                len: grid.bar - mid,
                pitch: None,
            }),
            _ => {}
        }
    }
    (marks, rests)
}

impl Row {
    /// Hear a row.
    pub fn hear(h: &ToHear) -> Row {
        let grid = Grid::of(h.meter);
        let bar = h.meter.bar_ticks().max(1);
        let bars = ((h.length + bar - 1) / bar).max(1) as usize;
        let length = bars as Tick * bar;
        let steps = bars as i32 * grid.bar;
        let clear = h.key.key().is_some();
        let key = h.key.best().unwrap_or(Key::new(0, Mode::Minor));

        let mut drums: [Vec<u8>; 4] = std::array::from_fn(|_| vec![0u8; steps as usize]);
        for p in h.parts {
            for hit in p.hits() {
                let s = ((hit.at as f64 / STEP as f64).round() as i32).rem_euclid(steps);
                let cell = &mut drums[lane_index(hit.lane)][s as usize];
                *cell = (*cell).max(hit.velocity);
            }
        }

        let lead = pick_lead(h.parts, h.lead).map(|i| Lead {
            part: i,
            line: one_line(h.parts[i].notes())
                .into_iter()
                .filter(|n| n.at < length)
                .collect(),
        });
        let energy = energies(lead.as_ref(), &drums, grid, bars);
        let read = lead.as_ref().and_then(|l| tune_plan(l, key, grid, steps));
        let from_tune = read.is_some();
        let (marks, rests) = read.unwrap_or_else(|| beat_plan(grid, bars));
        Row {
            meter: h.meter,
            grid,
            bars,
            steps,
            length,
            key,
            clear,
            played_pcs: h.played_pcs,
            chords: h.chords.clone(),
            groove: h.groove.cloned(),
            parts: h.parts.to_vec(),
            drums,
            lead,
            plan: RowPlan {
                marks,
                rests,
                energy,
                from_tune,
            },
        }
    }

    /// The loudest hit of `lane` at `step` (0: none). Steps wrap round
    /// the row, as it loops.
    pub fn drum(&self, lane: Lane, step: i32) -> u8 {
        self.drums[lane_index(lane)][step.rem_euclid(self.steps) as usize]
    }

    /// Whether the row has drums.
    pub fn has_drums(&self) -> bool {
        self.drums.iter().any(|l| l.iter().any(|&v| v > 0))
    }

    /// A step's start in ticks.
    pub fn tick(&self, step: i32) -> Tick {
        Tick::from(step) * STEP
    }

    /// The chord at `step`.
    pub fn chord_at(&self, step: i32) -> Option<Chord> {
        self.chords
            .span_at(self.tick(step.rem_euclid(self.steps)))
            .and_then(|s| s.answer.chord())
    }

    /// Whether a chord starts at `step`: a new chord, or the row's first.
    pub fn chord_starts(&self, step: i32) -> bool {
        let t = self.tick(step.rem_euclid(self.steps));
        self.chords.span_at(t).is_some_and(|s| s.start == t)
    }

    /// The pitches the row's other pitched parts hold at `t`.
    pub fn sounding(&self, t: Tick) -> impl Iterator<Item = u8> + '_ {
        self.parts
            .iter()
            .flat_map(|p| p.notes())
            .filter(move |n| n.at <= t && t < n.at + n.len)
            .map(|n| n.pitch)
    }

    /// The lowest and highest notes the `role` parts sound in
    /// `[from, to)`.
    pub fn span_of(&self, role: PartRole, from: Tick, to: Tick) -> Option<(u8, u8)> {
        let mut it = self
            .parts
            .iter()
            .filter(|p| p.role == role)
            .flat_map(|p| p.notes())
            .filter(|n| n.at < to && n.at + n.len > from)
            .map(|n| n.pitch);
        let first = it.next()?;
        Some(it.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p))))
    }

    /// Whether the tune starts a note on `step`.
    pub fn lead_starts(&self, step: i32) -> bool {
        let t = self.tick(step.rem_euclid(self.steps));
        self.lead
            .as_ref()
            .is_some_and(|l| l.starts_in(t - STEP / 2, t + STEP / 2))
    }

    /// The tune's note sounding at `step`.
    pub fn lead_at(&self, step: i32) -> Option<PlainNote> {
        let t = self.tick(step.rem_euclid(self.steps));
        self.lead.as_ref().and_then(|l| l.at(t))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::motif::tests::riff;
    use mpfree_context::chord::{Heard as ChordHeard, Quality};
    use mpfree_context::key::Candidate;

    pub(crate) fn c_minor() -> KeyReading {
        KeyReading::new(
            &[Candidate {
                key: Key::new(0, Mode::Minor),
                odds: 0.6,
            }],
            7,
        )
    }

    pub(crate) fn chords_cm_fm(bars: i64) -> ChordTimeline {
        let bar = Meter::default().bar_ticks();
        let cm = Chord::new(0, Quality::Minor7);
        let fm = Chord::new(5, Quality::Minor7);
        let v: Vec<(Chord, Tick)> = (0..bars)
            .map(|b| (if b % 2 == 0 { cm } else { fm }, bar))
            .collect();
        ChordTimeline::of(&v, ChordHeard::Heard)
    }

    /// A boom-bap beat over `bars` bars: kick on 1, the "and" of 2 and 3,
    /// snare on 2 and 4, eighth-note hats.
    pub(crate) fn beat(bars: i32) -> RowPart {
        let mut hits = Vec::new();
        for b in 0..bars {
            let o = b * 16;
            for (s, lane, v) in [
                (0, Lane::Kick, 110),
                (6, Lane::Kick, 90),
                (10, Lane::Kick, 100),
                (4, Lane::Snare, 110),
                (12, Lane::Snare, 108),
            ] {
                hits.push(DrumHit {
                    at: Tick::from(o + s) * STEP,
                    lane,
                    velocity: v,
                });
            }
            for s in (0..16).step_by(2) {
                hits.push(DrumHit {
                    at: Tick::from(o + s) * STEP,
                    lane: Lane::Hats,
                    velocity: 70,
                });
            }
        }
        RowPart {
            role: PartRole::Drums,
            heard: Heard::Hits(hits),
        }
    }

    pub(crate) fn riff_row(chords: &ChordTimeline, key: &KeyReading, with_beat: bool) -> Row {
        let mut parts = vec![RowPart {
            role: PartRole::Melody,
            heard: Heard::Notes(riff()),
        }];
        if with_beat {
            parts.push(beat(2));
        }
        Row::hear(&ToHear {
            parts: &parts,
            length: 2 * Meter::default().bar_ticks(),
            meter: Meter::default(),
            key,
            played_pcs: 0,
            chords,
            groove: None,
            lead: None,
        })
    }

    #[test]
    fn the_riff_lands_climbs_to_its_high_point_and_closes() {
        let (key, chords) = (c_minor(), chords_cm_fm(2));
        let row = riff_row(&chords, &key, true);
        assert_eq!(row.lead.as_ref().map(|l| l.part), Some(0));
        assert!(row.plan.from_tune);
        let p = &row.plan;
        // The first phrase ends on the held C of bar 1; the high point is
        // the A flat in bar 2; the riff closes on its last F.
        assert!(
            p.marks
                .iter()
                .any(|m| m.kind == AnchorKind::Landing && m.at == 12 && m.pitch == Some(60)),
            "{:?}",
            p.marks
        );
        assert_eq!(p.peak().map(|m| (m.at, m.pitch)), Some((18, Some(68))));
        let last = p.marks.last().expect("a close");
        assert_eq!(
            (last.kind, last.at, last.pitch),
            (AnchorKind::Close, 28, Some(65))
        );
        // The riff plays on through the loop: nowhere to rest a beat.
        assert!(p.rests.is_empty(), "{:?}", p.rests);
    }

    #[test]
    fn the_drums_are_read_by_kick_snare_and_hats() {
        let (key, chords) = (c_minor(), chords_cm_fm(2));
        let row = riff_row(&chords, &key, true);
        assert!(row.has_drums());
        assert_eq!(row.drum(Lane::Kick, 6), 90);
        assert_eq!(row.drum(Lane::Snare, 12), 108);
        assert_eq!(row.drum(Lane::Snare, 13), 0);
        assert_eq!(row.drum(Lane::Kick, 32), 110, "steps wrap round the loop");
        assert!(row.chord_starts(16) && !row.chord_starts(17));
        assert_eq!(row.chord_at(20).map(|c| c.root), Some(5));
    }

    #[test]
    fn a_beat_alone_gets_s1s_shape_with_one_high_point() {
        let (key, chords) = (c_minor(), chords_cm_fm(4));
        let parts = [beat(4)];
        let row = Row::hear(&ToHear {
            parts: &parts,
            length: 4 * Meter::default().bar_ticks(),
            meter: Meter::default(),
            key: &key,
            played_pcs: 0,
            chords: &chords,
            groove: None,
            lead: None,
        });
        assert!(row.lead.is_none() && !row.plan.from_tune);
        let peaks = row
            .plan
            .marks
            .iter()
            .filter(|m| m.kind == AnchorKind::Peak)
            .count();
        assert_eq!(peaks, 1);
        let last = row.plan.marks.last().expect("a close");
        assert_eq!(last.kind, AnchorKind::Close);
        assert_eq!(last.at / 16, 3, "the close is in the last bar");
        assert_eq!(row.plan.energy.len(), 4);
    }

    #[test]
    fn the_tune_is_the_melody_unless_another_part_is_picked() {
        let (key, chords) = (c_minor(), chords_cm_fm(2));
        let bass: Vec<PlainNote> = (0..8)
            .map(|i| PlainNote {
                at: i * 4 * STEP,
                len: 3 * STEP,
                pitch: if i % 2 == 0 { 36 } else { 43 },
                velocity: 100,
            })
            .collect();
        let parts = [
            RowPart {
                role: PartRole::Bass,
                heard: Heard::Notes(bass),
            },
            RowPart {
                role: PartRole::Melody,
                heard: Heard::Notes(riff()),
            },
        ];
        let mut h = ToHear {
            parts: &parts,
            length: 2 * Meter::default().bar_ticks(),
            meter: Meter::default(),
            key: &key,
            played_pcs: 0,
            chords: &chords,
            groove: None,
            lead: None,
        };
        assert_eq!(Row::hear(&h).lead.map(|l| l.part), Some(1));
        h.lead = Some(0);
        assert_eq!(Row::hear(&h).lead.map(|l| l.part), Some(0));
        let row = Row::hear(&ToHear { lead: None, ..h });
        assert_eq!(row.span_of(PartRole::Bass, 0, 8 * STEP), Some((36, 43)));
        assert!(row.lead_starts(16) && !row.lead_starts(17));
    }

    #[test]
    fn a_rest_across_the_loops_end_is_split_in_two() {
        let (key, chords) = (c_minor(), chords_cm_fm(2));
        // Notes from beat 2 of bar 1 to beat 3 of bar 2, then silence.
        let tune: Vec<PlainNote> = [
            (4, 4, 67),
            (8, 4, 65),
            (12, 4, 63),
            (16, 4, 62),
            (20, 4, 60),
        ]
        .iter()
        .map(|&(s, l, p)| PlainNote {
            at: s * STEP,
            len: l * STEP,
            pitch: p,
            velocity: 90,
        })
        .collect();
        let parts = [RowPart {
            role: PartRole::Melody,
            heard: Heard::Notes(tune),
        }];
        let row = Row::hear(&ToHear {
            parts: &parts,
            length: 2 * Meter::default().bar_ticks(),
            meter: Meter::default(),
            key: &key,
            played_pcs: 0,
            chords: &chords,
            groove: None,
            lead: None,
        });
        assert_eq!(row.plan.rests, vec![(0, 4), (24, 32)]);
        assert!(row.plan.resting(30) && row.plan.resting(2) && !row.plan.resting(10));
    }
}
