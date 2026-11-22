//! The listener (phase 0C): what was played, read as a key and a chord
//! timeline. The design's "Hearing the key, honestly" and "Chords as a
//! timeline"; the numbers are the second prototype's.
//!
//! - **The key.** Every note is weighted by how long it sounds and where
//!   it falls (more on bar lines and beats) and compared with the six
//!   modes in all 12 keys: 72 readings, turned into odds. The cues a
//!   player uses pick the home note: the last note, the first strong
//!   note, the lowest and longest notes, each bar's first note and a note
//!   held at a bar's end. A few notes can't add up to certainty, so the
//!   evidence is capped, and [`KeyReading`] never says more than "not
//!   sure yet" for three notes.
//! - **The chords.** Read beat by beat with a semi-Markov search (as in
//!   HarmAn and Melisma): every span of 1 beat up to the whole of what was
//!   read is scored against 144 chord shapes by the notes that fit, the
//!   notes that don't, the chord notes that never sounded and whether the
//!   lowest note is the root, plus how long chords usually last. Where no
//!   chord fits, or nothing sounds, the answer is no clear chord.
//! - **One chord** for a handful of notes ([`chord_of`]): D F A says "Dm"
//!   at once while the key stays open.
//!
//! What is read is capped ([`ReadBudget`]), and a reading that was cut
//! short says so.

use mpfree_transport::time::{Meter, Tick};

use crate::chord::{Chord, ChordAnswer, ChordSpan, ChordTimeline, Heard, Quality};
use crate::context::ReadBudget;
use crate::key::{Candidate, Key, KeyReading, Mode};

/// A note as the listener hears it, in ticks from the start of what is
/// read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Played {
    pub at: Tick,
    pub len: Tick,
    pub pitch: u8,
    pub velocity: u8,
}

impl Played {
    pub const fn new(at: Tick, len: Tick, pitch: u8, velocity: u8) -> Played {
        Played {
            at,
            len,
            pitch,
            velocity,
        }
    }
}

/// How likely each scale degree is to sound, given the home note: the
/// home note and fifth most, then the third.
const DEGREE_WEIGHT: [f64; 7] = [5.0, 2.8, 3.6, 2.8, 4.0, 2.6, 2.4];
/// A note outside the mode.
const OUTSIDE_WEIGHT: f64 = 0.55;
/// The most evidence the notes add up to, in weighted notes: past this a
/// longer input is no surer.
const EVIDENCE_CAP: f64 = 24.0;

impl Mode {
    /// How common each mode is in what people bring to a groovebox.
    fn prior(self) -> f64 {
        match self {
            Mode::Major | Mode::Minor => 1.0,
            Mode::Dorian => 0.55,
            Mode::Mixolydian => 0.45,
            Mode::Phrygian => 0.2,
            Mode::Lydian => 0.15,
        }
    }
}

/// A note's weight: how long it sounds (a quarter to two beats) and where
/// it starts (half as much again on a bar line, a fifth more on a beat).
fn weight(n: &Played, meter: Meter) -> f64 {
    let beat = meter.beat_ticks();
    let w = (n.len as f64 / beat as f64).clamp(0.25, 2.0);
    let m = if n.at.rem_euclid(meter.bar_ticks()) == 0 {
        1.5
    } else if n.at.rem_euclid(beat) == 0 {
        1.2
    } else {
        1.0
    };
    w * m
}

/// The key the notes are in, as a reading in words (with its odds kept
/// inside for ranking).
pub fn key(notes: &[Played], meter: Meter) -> KeyReading {
    if notes.is_empty() {
        return KeyReading::nothing();
    }
    let mut heard = [0.0f64; 12];
    for n in notes {
        heard[usize::from(n.pitch % 12)] += weight(n, meter);
    }
    let total: f64 = heard.iter().sum();
    let temper = if total > EVIDENCE_CAP {
        EVIDENCE_CAP / total
    } else {
        1.0
    };
    let distinct = heard.iter().filter(|&&x| x > 0.0).count() as u8;

    // The home-note cues, in time order.
    let mut by_time: Vec<&Played> = notes.iter().collect();
    by_time.sort_by_key(|n| (n.at, n.pitch));
    let pc = |n: &Played| n.pitch % 12;
    let last = by_time.last().map(|n| pc(n));
    let first_strong = by_time
        .iter()
        .find(|n| n.at.rem_euclid(meter.beat_ticks()) == 0)
        .map(|n| pc(n));
    let lowest = by_time.iter().min_by_key(|n| n.pitch).map(|n| pc(n));
    // The earliest of the longest.
    let longest = by_time.iter().rev().max_by_key(|n| n.len).map(|n| pc(n));
    let bar = meter.bar_ticks();
    let step = meter.beat_ticks() / 4;
    let mut downbeats = Vec::new();
    let mut bar_ends = Vec::new();
    let mut i = 0;
    while i < by_time.len() {
        let b = by_time[i].at.div_euclid(bar);
        let j = i + by_time[i..]
            .iter()
            .take_while(|n| n.at.div_euclid(bar) == b)
            .count();
        let in_bar = &by_time[i..j];
        if let Some(f) = in_bar.iter().find(|n| n.at.rem_euclid(bar) == 0) {
            downbeats.push(pc(f));
        }
        if let Some(l) = in_bar.last() {
            if l.len >= 2 * step {
                bar_ends.push(pc(l));
            }
        }
        i = j;
    }

    let mut scores: Vec<(Key, f64)> = Vec::with_capacity(72);
    for tonic in 0..12u8 {
        for mode in Mode::ALL {
            let steps = mode.steps();
            let mut profile = [OUTSIDE_WEIGHT; 12];
            for (d, s) in steps.iter().enumerate() {
                profile[usize::from((tonic + s) % 12)] = DEGREE_WEIGHT[d];
            }
            let sum: f64 = profile.iter().sum();
            let mut ll = 0.0;
            for p in 0..12 {
                ll += heard[p] * temper * (profile[p] / sum).ln();
            }
            let fifth = (tonic + steps[4]) % 12;
            let cue = |p: Option<u8>, home: f64, fif: f64| match p {
                Some(x) if x == tonic => home,
                Some(x) if x == fifth => fif,
                _ => 0.0,
            };
            ll += cue(last, 0.8, 0.25)
                + cue(first_strong, 0.45, 0.15)
                + cue(lowest, 0.4, 0.1)
                + cue(longest, 0.35, 0.1);
            for &d in &downbeats {
                ll += cue(Some(d), 0.35, 0.15);
            }
            for &e in &bar_ends {
                ll += cue(Some(e), 0.5, 0.15);
            }
            ll += mode.prior().ln();
            scores.push((Key::new(tonic, mode), ll));
        }
    }
    let top = scores.iter().map(|s| s.1).fold(f64::MIN, f64::max);
    let z: f64 = scores.iter().map(|s| (s.1 - top).exp()).sum();
    let cands: Vec<Candidate> = scores
        .iter()
        .map(|&(key, ll)| Candidate {
            key,
            odds: ((ll - top).exp() / z) as f32,
        })
        .collect();
    KeyReading::new(&cands, distinct)
}

/// A small cost for reading a richer chord than the notes need.
fn complexity(q: Quality) -> f64 {
    match q {
        Quality::Major | Quality::Minor => 0.0,
        Quality::Dominant7 | Quality::Minor7 | Quality::Major7 => 0.05,
        Quality::Power => 0.12,
        Quality::HalfDiminished7 | Quality::Minor6 | Quality::Major6 | Quality::Diminished => 0.09,
        Quality::Sus4 => 0.1,
        Quality::Diminished7 => 0.14,
    }
}

/// Below this a span's best chord isn't heard: no clear chord.
pub const NO_CLEAR_CHORD: f64 = 0.1;
/// What each silent beat inside a chord costs it, and earns a stretch of
/// no chord: a beat's rest stays inside its chord, a bar of silence is no
/// clear chord.
const SILENT_BEAT: f64 = 0.15;
/// What each sounding beat costs a stretch of no chord.
const SOUNDING_REST: f64 = 1.0;

/// How well one chord explains a span: the notes that fit, less the notes
/// that don't, less the chord notes that never sounded, plus the lowest
/// note on the root, all per note heard. 0 when nothing sounds.
fn span_score(sum: &[f64; 12], bass: Option<u8>, c: Chord, key: Option<Key>) -> f64 {
    let mask = c.pcs();
    let (mut total, mut fit, mut missing) = (0.0, 0.0, 0.0);
    for (pc, &x) in sum.iter().enumerate() {
        total += x;
        if mask & (1 << pc) != 0 {
            fit += x;
            if x <= 0.0 {
                missing += 1.0;
            }
        }
    }
    if total <= 0.0 {
        return 0.0;
    }
    let out = total - fit;
    let mut s = (fit - 1.1 * out) / total - 0.22 * missing - complexity(c.quality);
    if let Some(b) = bass {
        let b = b % 12;
        if b == c.root {
            s += 0.3;
        } else if mask & (1 << b) != 0 {
            s += 0.02;
        } else {
            s -= 0.1;
        }
    }
    if key.is_some_and(|k| k.pcs() & mask == mask) {
        s += 0.04;
    }
    s
}

/// How likely a span of `d` beats starting on beat `start` is: 2, 4, 8
/// and 16 beats from a bar line or half bar likeliest; a one-beat chord
/// must earn its place. `bar` is beats in a bar.
fn length_prior(start: usize, d: usize, bar: usize) -> f64 {
    let base = if d == bar {
        0.0
    } else if d * 2 == bar {
        -0.12
    } else if d == 2 * bar {
        -0.02
    } else if d == 4 * bar {
        -0.04
    } else if d.is_multiple_of(bar) {
        // Longer still: a drone or slow harmony.
        -0.06
    } else if d == 1 {
        -0.45
    } else if d == 3 * bar {
        -0.2
    } else {
        -0.35
    };
    let align = if start.is_multiple_of(bar) {
        0.0
    } else if bar.is_multiple_of(2) && start.is_multiple_of(bar / 2) {
        -0.1
    } else {
        -0.4
    };
    base + align
}

fn all_chords() -> Vec<Chord> {
    let mut v = Vec::with_capacity(144);
    for root in 0..12 {
        for q in Quality::ALL {
            v.push(Chord::new(root, q));
        }
    }
    v
}

/// Pitch-class weights and the lowest note, beat by beat. A note counts
/// fully on the beat it starts and 0.6 on beats it is held into.
fn beats_of(notes: &[Played], beats: usize, beat: Tick) -> (Vec<[f64; 12]>, Vec<Option<u8>>) {
    let mut prof = vec![[0.0f64; 12]; beats];
    let mut low: Vec<Option<u8>> = vec![None; beats];
    for n in notes {
        let a = n.at.max(0);
        let b = n.at + n.len.max(1);
        let first = (a / beat) as usize;
        let last = ((b - 1).max(a) / beat) as usize;
        for (bi, (p, l)) in prof
            .iter_mut()
            .zip(low.iter_mut())
            .enumerate()
            .take(last.min(beats.saturating_sub(1)) + 1)
            .skip(first)
        {
            let s = (bi as Tick * beat).max(a);
            let e = ((bi as Tick + 1) * beat).min(b);
            if e <= s {
                continue;
            }
            let w = (e - s) as f64 / beat as f64 * if bi == first { 1.0 } else { 0.6 };
            p[usize::from(n.pitch % 12)] += w;
            *l = Some(l.map_or(n.pitch, |x| x.min(n.pitch)));
        }
    }
    (prof, low)
}

/// The chords of `notes` over `len` ticks, as a timeline from tick 0:
/// each span a chord with how sure it is and a runner-up, or no clear
/// chord where none fits or nothing sounds. `key`, when known, favours
/// chords in it a little.
pub fn chords(notes: &[Played], len: Tick, key: Option<Key>, meter: Meter) -> ChordTimeline {
    let beat = meter.beat_ticks().max(1);
    let len = len.max(1);
    let beats = ((len + beat - 1) / beat) as usize;
    let bar = usize::from(meter.beats.max(1));
    let (prof, low) = beats_of(notes, beats, beat);
    let cands = all_chords();
    // The candidates are the 144 chords and, last, no chord.
    let rest = cands.len();
    let k = rest + 1;
    // dp[e][c]: the best score of beats 0..e ending with candidate c over
    // a span that ends at e; back[e][c]: that span's length and the
    // candidate before it.
    let mut dp = vec![vec![f64::MIN; k]; beats + 1];
    let mut back = vec![vec![(0usize, usize::MAX); k]; beats + 1];
    dp[0].iter_mut().for_each(|x| *x = 0.0);
    for e in 1..=beats {
        let mut sum = [0.0f64; 12];
        let mut bass: Option<u8> = None;
        let mut sounding = 0.0;
        for d in 1..=e {
            let s = e - d;
            for (acc, x) in sum.iter_mut().zip(&prof[s]) {
                *acc += x;
            }
            if let Some(l) = low[s] {
                bass = Some(bass.map_or(l, |x| x.min(l)));
            }
            if prof[s].iter().any(|&x| x > 0.0) {
                sounding += 1.0;
            }
            let silent = d as f64 - sounding;
            // The best two chords to come from, so a change of chord is
            // one pass over the chords.
            let (mut b1, mut b1c, mut b2, mut b2c) = (f64::MIN, 0usize, f64::MIN, 0usize);
            for (c, &v) in dp[s].iter().enumerate() {
                if v > b1 {
                    (b2, b2c) = (b1, b1c);
                    (b1, b1c) = (v, c);
                } else if v > b2 {
                    (b2, b2c) = (v, c);
                }
            }
            let prior = length_prior(s, d, bar);
            for c in 0..k {
                let (from, prev) = if s == 0 {
                    (0.0, usize::MAX)
                } else if b1c != c {
                    (b1, b1c)
                } else {
                    (b2, b2c)
                };
                if from == f64::MIN {
                    continue;
                }
                // Evidence grows with the beats that sound.
                let evidence = if c == rest {
                    SILENT_BEAT * silent - SOUNDING_REST * sounding
                } else {
                    sounding * span_score(&sum, bass, cands[c], key) - SILENT_BEAT * silent
                };
                let v = from + evidence + prior;
                if v > dp[e][c] {
                    dp[e][c] = v;
                    back[e][c] = (d, prev);
                }
            }
        }
    }
    // Trace back, best chord at the end first.
    let mut c = (0..k)
        .max_by(|&a, &b| dp[beats][a].total_cmp(&dp[beats][b]))
        .unwrap_or(0);
    let mut e = beats;
    let mut found: Vec<(usize, usize, usize)> = Vec::new();
    while e > 0 {
        let (d, prev) = back[e][c];
        let d = d.max(1);
        found.push((e - d, e, c));
        e -= d;
        c = if prev == usize::MAX { 0 } else { prev };
    }
    found.reverse();
    let mut spans: Vec<ChordSpan> = Vec::new();
    for (s, e, c) in found {
        let mut sum = [0.0f64; 12];
        let mut bass = None;
        for b in s..e {
            for (acc, x) in sum.iter_mut().zip(&prof[b]) {
                *acc += x;
            }
            if let Some(l) = low[b] {
                bass = Some(bass.map_or(l, |x: u8| x.min(l)));
            }
        }
        let start = s as Tick * beat;
        let end = (e as Tick * beat).min(len);
        let silent = sum.iter().all(|&x| x <= 0.0);
        let mine = cands
            .get(c)
            .map_or(f64::MIN, |&ch| span_score(&sum, bass, ch, key));
        let span = if c == rest || silent || mine < NO_CLEAR_CHORD {
            ChordSpan::new(start, end - start, ChordAnswer::NoClearChord, Heard::Heard)
        } else {
            let mut scored: Vec<(f64, Chord)> = cands
                .iter()
                .map(|&ch| (span_score(&sum, bass, ch, key), ch))
                .collect();
            scored.sort_by(|a, b| b.0.total_cmp(&a.0));
            let runner = scored.iter().find(|x| x.1 != cands[c]);
            ChordSpan {
                sureness: (mine - runner.map_or(0.0, |r| r.0)).clamp(0.0, 1.0) as f32,
                runner_up: runner.map(|r| r.1),
                ..ChordSpan::new(
                    start,
                    end - start,
                    ChordAnswer::Chord(cands[c]),
                    Heard::Heard,
                )
            }
        };
        // Two stretches of no clear chord side by side are one.
        match spans.last_mut() {
            Some(prev)
                if prev.answer == ChordAnswer::NoClearChord
                    && span.answer == ChordAnswer::NoClearChord =>
            {
                prev.len += span.len;
            }
            _ => spans.push(span),
        }
    }
    ChordTimeline::new(spans).unwrap_or_default()
}

/// The one chord a handful of notes make, or no clear chord.
pub fn chord_of(notes: &[Played]) -> ChordAnswer {
    let mut sum = [0.0f64; 12];
    for n in notes {
        sum[usize::from(n.pitch % 12)] += 1.0;
    }
    let bass = notes.iter().map(|n| n.pitch).min();
    let best = all_chords()
        .into_iter()
        .map(|c| (span_score(&sum, bass, c, None), c))
        .max_by(|a, b| a.0.total_cmp(&b.0));
    match best {
        Some((s, c)) if s >= NO_CLEAR_CHORD && !notes.is_empty() => ChordAnswer::Chord(c),
        _ => ChordAnswer::NoClearChord,
    }
}

/// What a reading heard, inside its budget.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub key: KeyReading,
    pub chords: ChordTimeline,
    /// Every pitch class played (bit 0 = C).
    pub played_pcs: u16,
    /// The budget cut it short.
    pub cut_short: bool,
}

/// The notes a reading may read: the first `budget.max_bars` bars and,
/// of those, the first `budget.max_notes` notes in time order. Whether
/// anything was left out.
pub fn within(
    notes: &[Played],
    len: Tick,
    budget: ReadBudget,
    meter: Meter,
) -> (Vec<Played>, Tick, bool) {
    let max_len = meter.bar_ticks() * Tick::from(budget.max_bars);
    let len_read = len.min(max_len);
    let mut kept: Vec<Played> = notes
        .iter()
        .copied()
        .filter(|n| (0..len_read).contains(&n.at))
        .collect();
    kept.sort_by_key(|n| (n.at, n.pitch));
    let mut cut = len_read < len || kept.len() < notes.len();
    if kept.len() > budget.max_notes as usize {
        kept.truncate(budget.max_notes as usize);
        cut = true;
    }
    (kept, len_read, cut)
}

/// Listen to what was played over `len` ticks, inside the budget: the
/// key, then the chords (favouring the key when there is one).
pub fn listen(notes: &[Played], len: Tick, budget: ReadBudget, meter: Meter) -> Reading {
    let (notes, len, cut_short) = within(notes, len, budget, meter);
    let key = key(&notes, meter);
    let chords = chords(&notes, len, key.key(), meter);
    let played_pcs = notes.iter().fold(0u16, |m, n| m | 1 << (n.pitch % 12));
    Reading {
        key,
        chords,
        played_pcs,
        cut_short,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::Sureness;
    use mpfree_transport::time::PPQ;

    const STEP: Tick = PPQ as Tick / 4;
    const BEAT: Tick = PPQ as Tick;
    const BAR: Tick = 4 * BEAT;

    /// A note in 1/16 steps, as the prototype wrote its examples.
    fn n(at: f64, len: f64, pitch: u8) -> Played {
        Played::new(
            (at * STEP as f64).round() as Tick,
            (len * STEP as f64).round() as Tick,
            pitch,
            90,
        )
    }

    fn meter() -> Meter {
        Meter::default()
    }

    fn riff() -> Vec<Played> {
        vec![
            n(0.0, 2.0, 60),
            n(2.0, 1.0, 63),
            n(3.0, 1.0, 67),
            n(4.0, 2.0, 65),
            n(6.0, 2.0, 63),
            n(8.0, 3.0, 62),
            n(11.0, 1.0, 63),
            n(12.0, 4.0, 60),
            n(16.0, 2.0, 67),
            n(18.0, 1.0, 68),
            n(19.0, 1.0, 67),
            n(20.0, 2.0, 65),
            n(22.0, 2.0, 63),
            n(24.0, 2.0, 62),
            n(26.0, 2.0, 63),
            n(28.0, 4.0, 65),
        ]
    }

    #[test]
    fn the_key_in_words_as_the_prototype_heard_it() {
        let dfa = [n(0.0, 2.0, 62), n(2.0, 2.0, 65), n(4.0, 4.0, 69)];
        let r = key(&dfa, meter());
        assert_eq!(
            r.words(),
            "Likely D minor, also possible A minor, not sure yet"
        );
        assert_eq!(chord_of(&dfa).name(), "Dm");

        let gabd = [
            n(0.0, 2.0, 67),
            n(2.0, 2.0, 69),
            n(4.0, 2.0, 71),
            n(6.0, 2.0, 74),
        ];
        let r = key(&gabd, meter());
        assert_eq!(r.sureness(), Sureness::NotSureYet);
        assert_eq!(r.best(), Some(Key::new(7, Mode::Major)));
        assert_eq!(chord_of(&gabd).name(), "G");

        assert_eq!(key(&riff(), meter()).words(), "C minor, fairly sure");

        let dorian = [
            n(0.0, 2.0, 62),
            n(2.0, 2.0, 65),
            n(4.0, 2.0, 69),
            n(6.0, 2.0, 71),
            n(8.0, 2.0, 69),
            n(10.0, 2.0, 67),
            n(12.0, 4.0, 65),
            n(16.0, 2.0, 64),
            n(18.0, 2.0, 62),
            n(20.0, 2.0, 71),
            n(22.0, 2.0, 69),
            n(24.0, 4.0, 67),
            n(28.0, 4.0, 62),
        ];
        assert_eq!(key(&dorian, meter()).words(), "D Dorian, fairly sure");
    }

    #[test]
    fn three_notes_never_read_as_sure_and_nothing_is_no_clear_key() {
        let c_major_triad = [n(0.0, 8.0, 60), n(0.0, 8.0, 64), n(0.0, 8.0, 67)];
        let r = key(&c_major_triad, meter());
        assert!(r.sureness() <= Sureness::NotSureYet);
        assert_eq!(key(&[], meter()).words(), "No clear key");
        // All twelve notes evenly: nothing stands out.
        let chromatic: Vec<Played> = (0..12)
            .map(|i| n(f64::from(i) * 4.0, 4.0, 60 + i as u8))
            .collect();
        assert_eq!(key(&chromatic, meter()).sureness(), Sureness::NoClearKey);
    }

    /// The prototype's comping: a left-hand root and a right-hand voicing
    /// per chord, two chords a bar.
    fn comping() -> Vec<Played> {
        let prog = [
            (0u8, Quality::Minor7, 8.0),
            (5, Quality::Dominant7, 8.0),
            (10, Quality::Major7, 8.0),
            (3, Quality::Major7, 8.0),
            (9, Quality::HalfDiminished7, 8.0),
            (2, Quality::Dominant7, 8.0),
            (7, Quality::Minor6, 16.0),
        ];
        let mut out = Vec::new();
        let mut t = 0.0;
        for (root, q, len) in prog {
            out.push(n(t, len, 36 + root + if root > 7 { 0 } else { 12 }));
            for &iv in &q.intervals()[1..] {
                out.push(n(t, len - 1.0, 60 + (root + iv) % 12));
            }
            t += len;
        }
        out
    }

    #[test]
    fn two_chords_a_bar_are_heard_exactly() {
        let tl = chords(&comping(), 4 * BAR, None, meter());
        assert_eq!(tl.names(), "Cm7 F7 Bbmaj7 Ebmaj7 Am7b5 D7 Gm6");
        let starts: Vec<Tick> = tl.spans().iter().map(|s| s.start / BEAT).collect();
        assert_eq!(starts, vec![0, 2, 4, 6, 8, 10, 12]);
        assert!(tl.spans().iter().all(|s| s.runner_up.is_some()));
        assert_eq!(tl.len(), 4 * BAR);
    }

    #[test]
    fn a_drone_is_one_chord_and_silence_is_no_clear_chord() {
        // C and G held for eight bars, a little melody of chord notes on
        // top: one chord over the whole length.
        let mut drone = vec![n(0.0, 128.0, 36), n(0.0, 128.0, 43)];
        for b in 0..8 {
            drone.push(n(f64::from(b) * 16.0, 4.0, 60));
            drone.push(n(f64::from(b) * 16.0 + 8.0, 4.0, 67));
        }
        let tl = chords(&drone, 8 * BAR, None, meter());
        assert_eq!(tl.spans().len(), 1, "{}", tl.names());
        assert_eq!(tl.spans()[0].len, 8 * BAR);
        // A bar of chord, a bar of nothing, a bar of chord.
        let gap = [
            n(0.0, 16.0, 48),
            n(0.0, 16.0, 52),
            n(0.0, 16.0, 55),
            n(32.0, 16.0, 53),
            n(32.0, 16.0, 57),
            n(32.0, 16.0, 60),
        ];
        let tl = chords(&gap, 3 * BAR, None, meter());
        assert_eq!(tl.names(), "C no clear chord F");
        assert_eq!(tl.spans()[1].start, BAR);
        // A cluster fits no chord.
        assert_eq!(
            chord_of(&[
                n(0.0, 4.0, 60),
                n(0.0, 4.0, 61),
                n(0.0, 4.0, 62),
                n(0.0, 4.0, 63)
            ]),
            ChordAnswer::NoClearChord
        );
    }

    #[test]
    fn the_budget_caps_what_is_read_and_says_so() {
        let budget = ReadBudget {
            max_bars: 2,
            max_notes: 2048,
        };
        let mut long = riff();
        long.extend(riff().iter().map(|p| Played {
            at: p.at + 2 * BAR,
            ..*p
        }));
        let h = listen(&long, 4 * BAR, budget, meter());
        assert!(h.cut_short);
        assert_eq!(h.chords.len(), 2 * BAR);
        assert_eq!(h.key.words(), "C minor, fairly sure");
        let whole = listen(&riff(), 2 * BAR, ReadBudget::default(), meter());
        assert!(!whole.cut_short);
        assert_eq!(whole.played_pcs, 0b0001_1010_1101);
        let few = ReadBudget {
            max_bars: 16,
            max_notes: 4,
        };
        assert!(listen(&riff(), 2 * BAR, few, meter()).cut_short);
    }
}
