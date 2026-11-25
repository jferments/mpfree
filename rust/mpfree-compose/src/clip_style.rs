//! A style made from a clip (composing step 6): the clip's feel, how busy
//! it is, where its accents fall, its loudness and how far it leans, read
//! from its notes ([`read_clip`]); the phrase plan and the chords' habits
//! lent by a built-in style, the nearest first ([`nearest`]). The made
//! style offers itself in every Style row after the built-in three, kept
//! with the song as one line of text ([`ClipStyle::to_text`]).
//!
//! What is read:
//!
//! - **Feel**: the grid (1/8s, 1/16s or triplets) and the swing, from where
//!   the off-beat notes fall. Notes written straight in a clip that plays
//!   swung read as the clip's own swing.
//! - **Busy**: notes a bar (a chord counts once).
//! - **Accents**: the 1/16 steps played louder than the clip's average
//!   (6 or more), in at least half its bars; every step's difference
//!   becomes the style's accent pattern.
//! - **Loudness**: the softest and the loudest note, the average, and the
//!   share of ghost notes (25 or more under the average).
//! - **Leans**: how far the notes sit behind (or ahead of) the grid, in
//!   ms: the middle of the notes near a grid line. A written clip sits on
//!   it.

use mpfree_context::groove::{unswing, STEP};
use mpfree_transport::time::{Meter, Swing, Tick, PPQ};

use crate::realize::PlainNote;
use crate::style::{Feel, PlayingStyle, RhythmStyle, Style, STYLES};

/// The grid a clip plays on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Subdivision {
    Eighths,
    Sixteenths,
    Triplets,
}

impl Subdivision {
    pub const fn words(self) -> &'static str {
        match self {
            Subdivision::Eighths => "8ths",
            Subdivision::Sixteenths => "16ths",
            Subdivision::Triplets => "triplets",
        }
    }

    fn token(self) -> &'static str {
        match self {
            Subdivision::Eighths => "8",
            Subdivision::Sixteenths => "16",
            Subdivision::Triplets => "3",
        }
    }

    fn of_token(t: &str) -> Option<Subdivision> {
        match t {
            "8" => Some(Subdivision::Eighths),
            "16" => Some(Subdivision::Sixteenths),
            "3" => Some(Subdivision::Triplets),
            _ => None,
        }
    }
}

/// What a clip's notes say about how it is played.
#[derive(Clone, Debug, PartialEq)]
pub struct Read {
    pub grid: Subdivision,
    /// 50 straight to 75; triplets read as 66 on 1/8s.
    pub swing: u8,
    /// Notes a bar.
    pub busy: f32,
    /// Share of notes off the beat.
    pub syncopation: f32,
    /// Share of beats with no note starting in them.
    pub rest: f32,
    /// Each 1/16 step's loudness against the clip's average.
    pub accent: [i8; 16],
    /// The accented steps, from 0.
    pub accents: Vec<usize>,
    /// 1/16 steps in a bar.
    pub bar_steps: usize,
    pub steps_per_beat: usize,
    /// The softest and the loudest note.
    pub loudness: (u8, u8),
    pub velocity: u8,
    /// Share of ghost notes.
    pub ghosts: f32,
    /// Behind the grid in ms (ahead below 0).
    pub lean_ms: f32,
}

/// Off-beat notes are what the feel is read from; fewer than this and the
/// clip's own playback swing stands.
const LEAST_OFFBEATS: usize = 3;
/// A step this much louder than the average, in half the bars, is an
/// accent.
const ACCENT: f64 = 6.0;
/// A note this much softer than the average is a ghost.
const GHOST: f64 = 25.0;

/// Read a clip's notes (in its ticks, as written; `swing` is the swing it
/// plays with). None for a clip with no notes.
pub fn read_clip(
    notes: &[PlainNote],
    length: Tick,
    meter: Meter,
    bpm: f64,
    swing: Swing,
) -> Option<Read> {
    if notes.is_empty() {
        return None;
    }
    let beat = meter.beat_ticks().max(1);
    let bar = meter.bar_ticks().max(1);
    let bars = ((length.max(1) + bar - 1) / bar).max(1);
    let bar_steps = ((bar / STEP) as usize).clamp(1, 16);
    let steps_per_beat = ((beat / STEP) as usize).max(1);
    // One onset a time: a chord counts once, at its loudest.
    let mut onsets: Vec<(Tick, u8)> = Vec::new();
    let mut sorted: Vec<&PlainNote> = notes.iter().collect();
    sorted.sort_by_key(|n| n.at);
    for n in sorted {
        match onsets.last_mut() {
            Some(last) if (n.at - last.0).abs() < STEP / 4 => last.1 = last.1.max(n.velocity),
            _ => onsets.push((n.at, n.velocity)),
        }
    }
    // How far the notes near a beat line sit off it: swing never moves
    // those, so it is the lean, taken off before the feel is read.
    let on_beat: Vec<f64> = onsets
        .iter()
        .map(|&(at, _)| {
            let r = at.rem_euclid(beat);
            if r > beat / 2 {
                (r - beat) as f64
            } else {
                r as f64
            }
        })
        .filter(|off| off.abs() <= STEP as f64 / 3.0)
        .collect();
    let lean_ticks = {
        let mut v = on_beat;
        v.sort_by(f64::total_cmp);
        v.get(v.len() / 2).copied().unwrap_or(0.0)
    };
    let frac = |at: Tick| (at as f64 - lean_ticks).rem_euclid(beat as f64) / beat as f64;
    let near = |f: f64, x: f64, w: f64| (f - x).abs() <= w;
    let fracs: Vec<f64> = onsets.iter().map(|&(at, _)| frac(at)).collect();
    let sixteenth: Vec<f64> = fracs
        .iter()
        .copied()
        .filter(|&f| (0.18..=0.42).contains(&f) || (0.68..=0.92).contains(&f))
        .collect();
    let thirds = fracs.iter().filter(|&&f| near(f, 1.0 / 3.0, 0.03)).count();
    let two_thirds = fracs.iter().filter(|&&f| near(f, 2.0 / 3.0, 0.03)).count();
    let quarters = fracs
        .iter()
        .filter(|&&f| near(f, 0.25, 0.03) || near(f, 0.75, 0.03))
        .count();
    let n = onsets.len() as f64;
    let median = |mut v: Vec<f64>| -> Option<f64> {
        if v.is_empty() {
            return None;
        }
        v.sort_by(f64::total_cmp);
        Some(v[v.len() / 2])
    };
    let (grid, read_swing, offbeats) =
        if thirds as f64 >= 0.1 * n && two_thirds as f64 >= 0.1 * n && quarters < thirds {
            (Subdivision::Triplets, 66.0, thirds + two_thirds)
        } else if sixteenth.len() as f64 >= 0.15 * n {
            // Within its 1/8: how far into the pair the off 1/16 falls.
            let s = sixteenth.iter().map(|&f| (f % 0.5) / 0.5 * 100.0).collect();
            (
                Subdivision::Sixteenths,
                median(s).unwrap_or(50.0),
                sixteenth.len(),
            )
        } else {
            let off: Vec<f64> = fracs
                .iter()
                .copied()
                .filter(|&f| (0.44..=0.76).contains(&f))
                .collect();
            let k = off.len();
            (
                Subdivision::Eighths,
                median(off.into_iter().map(|f| f * 100.0).collect()).unwrap_or(50.0),
                k,
            )
        };
    let mut swing_amount = read_swing.round().clamp(50.0, 75.0) as u8;
    let mut grid = grid;
    // Written straight in a clip that plays swung: the clip's swing.
    if (swing_amount <= 52 || offbeats < LEAST_OFFBEATS) && swing.amount > 52 {
        swing_amount = swing.amount;
        grid = if swing.grid >= PPQ / 2 {
            Subdivision::Eighths
        } else {
            Subdivision::Sixteenths
        };
    }
    let unit = match grid {
        Subdivision::Sixteenths => PPQ / 4,
        _ => PPQ / 2,
    };
    let felt = Swing::new(unit, swing_amount).unwrap_or_default();

    // Where each onset sits on the grid, and how far off it.
    let mut steps: Vec<usize> = Vec::with_capacity(onsets.len());
    let mut lean: Vec<f64> = Vec::new();
    let ms_per_tick = 60_000.0 / (bpm.max(1.0) * f64::from(PPQ));
    for &(at, _) in &onsets {
        let straight = unswing(felt, at as f64);
        let step = (straight / STEP as f64).round();
        steps.push((step as i64).rem_euclid(bar_steps as i64) as usize);
        let line = felt.apply(step * STEP as f64);
        let off = at as f64 - line;
        if off.abs() <= STEP as f64 / 3.0 {
            lean.push(off * ms_per_tick);
        }
    }
    let lean_ms = median(lean).unwrap_or(0.0) as f32;

    // Loudness and accents.
    let mean = onsets.iter().map(|&(_, v)| f64::from(v)).sum::<f64>() / n;
    let mut sum = [0.0f64; 16];
    let mut hits = [0u32; 16];
    for (&s, &(_, v)) in steps.iter().zip(&onsets) {
        sum[s] += f64::from(v);
        hits[s] += 1;
    }
    let mut accent = [0i8; 16];
    let mut accents = Vec::new();
    for s in 0..bar_steps {
        if hits[s] > 0 {
            let d = sum[s] / f64::from(hits[s]) - mean;
            accent[s] = d.round().clamp(-16.0, 16.0) as i8;
            if d >= ACCENT && u64::from(hits[s]) * 2 >= bars as u64 {
                accents.push(s);
            }
        }
    }
    let lo = notes.iter().map(|n| n.velocity).min().unwrap_or(0);
    let hi = notes.iter().map(|n| n.velocity).max().unwrap_or(0);
    let ghosts = onsets
        .iter()
        .filter(|&&(_, v)| f64::from(v) < mean - GHOST)
        .count() as f64
        / n;
    let off_beat = steps.iter().filter(|&&s| s % steps_per_beat != 0).count() as f64 / n;
    let beats_total = (bars * i64::from(meter.beats)).max(1);
    let struck: std::collections::BTreeSet<i64> =
        onsets.iter().map(|&(at, _)| at.div_euclid(beat)).collect();
    let rest = 1.0 - struck.len().min(beats_total as usize) as f64 / beats_total as f64;
    Some(Read {
        grid,
        swing: swing_amount,
        busy: (n / bars as f64) as f32,
        syncopation: off_beat as f32,
        rest: rest as f32,
        accent,
        accents,
        bar_steps,
        steps_per_beat,
        loudness: (lo, hi),
        velocity: mean.round().clamp(1.0, 127.0) as u8,
        ghosts: ghosts as f32,
        lean_ms,
    })
}

/// The name of 1/16 step `s` of a bar ("1", "the e of 2", "the and of 3",
/// "the a of 3").
pub fn step_words(s: usize, steps_per_beat: usize) -> String {
    let k = steps_per_beat.max(1);
    let beat = s / k + 1;
    match (s % k, k) {
        (0, _) => beat.to_string(),
        (2, 4) => format!("the and of {beat}"),
        (1, _) => format!("the e of {beat}"),
        (_, _) => format!("the a of {beat}"),
    }
}

impl Read {
    /// The feel: the grid, and the swing when there is one.
    pub fn feel(&self) -> Feel {
        match self.grid {
            Subdivision::Triplets => Feel::Swing8,
            Subdivision::Eighths if self.swing >= 55 => Feel::Swing8,
            _ if self.grid == Subdivision::Sixteenths || self.syncopation >= 0.45 => Feel::Push,
            _ => Feel::Simplify,
        }
    }

    /// What was read, in words, as the Make a style sheet lists it.
    pub fn words(&self) -> Vec<(&'static str, String)> {
        let feel = if self.grid == Subdivision::Triplets {
            "triplets".to_string()
        } else if self.swing > 52 {
            format!("{}, swung {} %", self.grid.words(), self.swing)
        } else {
            format!("{}, straight", self.grid.words())
        };
        let accents = match self.accents.as_slice() {
            [] => "even".to_string(),
            [one] => format!("on {}", step_words(*one, self.steps_per_beat)),
            more => {
                let names: Vec<String> = more
                    .iter()
                    .map(|&s| step_words(s, self.steps_per_beat))
                    .collect();
                let (last, rest) = names.split_last().unwrap_or((&names[0], &[]));
                format!("on {} and {last}", rest.join(", "))
            }
        };
        let lean = self.lean_ms.round() as i32;
        let leans = match lean {
            0 => "on the grid".to_string(),
            l if l > 0 => format!("{l} ms late"),
            l => format!("{} ms early", -l),
        };
        vec![
            ("Feel", feel),
            (
                "Busy",
                format!("about {} notes a bar", self.busy.round() as i32),
            ),
            ("Accents", accents),
            (
                "Loudness",
                format!("{} to {}", self.loudness.0, self.loudness.1),
            ),
            ("Leans", leans),
        ]
    }
}

/// The built-in style nearest a clip's feel: the same feel first, then the
/// nearest busyness.
pub fn nearest(read: &Read, meter: Meter) -> &'static Style {
    let beats = f32::from(meter.beats.max(1));
    let per_beat = read.busy / beats;
    let feel = read.feel();
    STYLES
        .iter()
        .min_by(|a, b| {
            let cost = |s: &Style| {
                (if s.rhythm.feel == feel { 0.0 } else { 10.0 })
                    + (s.rhythm.density - per_beat).abs()
            };
            cost(a).total_cmp(&cost(b))
        })
        .unwrap_or(&STYLES[0])
}

/// How many styles a Style row steps through: the built-in ones, then the
/// project's made ones.
pub fn style_count(made: &[ClipStyle]) -> usize {
    STYLES.len() + made.len()
}

/// The Style rows' style `k`: a built-in one, else a made one (the last
/// there is when `k` is past the end, as after an UNDO took one away).
pub fn style_at(k: usize, made: &[ClipStyle]) -> Style {
    match k.checked_sub(STYLES.len()) {
        None => STYLES[k],
        Some(m) => made
            .get(m)
            .or(made.last())
            .map_or(STYLES[STYLES.len() - 1], ClipStyle::style),
    }
}

/// Style `k`'s name, as a Style row shows it.
pub fn style_name(k: usize, made: &[ClipStyle]) -> &str {
    match k.checked_sub(STYLES.len()) {
        None => STYLES[k].name,
        Some(m) => made
            .get(m)
            .or(made.last())
            .map_or(STYLES[STYLES.len() - 1].name, |c| c.name.as_str()),
    }
}

/// The project's made styles from its lines (`Project::styles`); a line
/// this build can't read is left out.
pub fn clip_styles(lines: &[String]) -> Vec<ClipStyle> {
    lines
        .iter()
        .filter_map(|l| ClipStyle::from_text(l))
        .collect()
}

/// A made style's name as the composer's styles carry one
/// (`Style::name`, which is `&'static str`): each name is kept once for
/// the life of the program, so naming a style again, or opening its
/// project again, costs nothing more. Names are typed, a few a project.
fn named(name: &str) -> &'static str {
    use std::sync::Mutex;
    static NAMES: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(n) = names.iter().find(|n| **n == name) {
        return n;
    }
    let n: &'static str = Box::leak(name.to_string().into_boxed_str());
    names.push(n);
    n
}

/// A style made from a clip: its name, the built-in style lending the plan
/// and the chords' habits, and what the clip's notes said.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipStyle {
    pub name: String,
    /// The built-in style's name ([`Style::name`]).
    pub plan: &'static str,
    pub read: Read,
}

impl ClipStyle {
    /// From a clip read, its plan the nearest built-in style; the name
    /// starts as the clip's ("Clip 1 groove").
    pub fn new(clip_name: &str, read: Read, meter: Meter) -> ClipStyle {
        ClipStyle {
            name: format!("{clip_name} groove"),
            plan: nearest(&read, meter).name,
            read,
        }
    }

    /// The style a Continue, Add a part or New idea plays: the plan's
    /// harmony, bass, melody, development and weights, with the clip's
    /// rhythm and playing, under the made style's name.
    pub fn style(&self) -> Style {
        let base = STYLES
            .iter()
            .find(|s| s.name == self.plan)
            .copied()
            .unwrap_or(STYLES[0]);
        let r = &self.read;
        let beats = (r.bar_steps / r.steps_per_beat.max(1)).max(1) as f32;
        let unit = match r.grid {
            Subdivision::Sixteenths => PPQ / 4,
            _ => PPQ / 2,
        };
        let (lo, hi) = r.loudness;
        Style {
            rhythm: RhythmStyle {
                feel: r.feel(),
                swing: Swing::new(unit, r.swing).unwrap_or(base.rhythm.swing),
                syncopation: r.syncopation.clamp(0.0, 1.0),
                density: (r.busy / beats).clamp(0.25, 4.0),
                rest: r.rest.clamp(0.0, 0.8),
                ..base.rhythm
            },
            playing: PlayingStyle {
                accent: r.accent,
                velocity: r.velocity.clamp(30, 120),
                velocity_range: (f32::from(hi.saturating_sub(lo)) / 4.0).clamp(4.0, 20.0),
                ghosts: r.ghosts.clamp(0.0, 0.5),
                lean_ms: r.lean_ms.clamp(-25.0, 25.0),
                ..base.playing
            },
            name: named(&self.name),
            ..base
        }
    }

    /// One line of text, as the song keeps it.
    pub fn to_text(&self) -> String {
        let r = &self.read;
        let accent: Vec<String> = r.accent.iter().map(i8::to_string).collect();
        let accents: Vec<String> = r.accents.iter().map(usize::to_string).collect();
        format!(
            "plan={} grid={} swing={} busy={} sync={} rest={} vel={} lo={} hi={} ghosts={} lean={} steps={} beat={} accent={} accents={} name={}",
            self.plan.replace(' ', "_"),
            r.grid.token(),
            r.swing,
            r.busy,
            r.syncopation,
            r.rest,
            r.velocity,
            r.loudness.0,
            r.loudness.1,
            r.ghosts,
            r.lean_ms,
            r.bar_steps,
            r.steps_per_beat,
            accent.join(","),
            if accents.is_empty() { "-".to_string() } else { accents.join(",") },
            self.name
        )
    }

    /// Back from [`ClipStyle::to_text`]; None when a field is missing or
    /// the plan is not a built-in style.
    pub fn from_text(line: &str) -> Option<ClipStyle> {
        let (fields, name) = line.split_once(" name=")?;
        let get = |k: &str| {
            fields
                .split(' ')
                .find_map(|f| f.strip_prefix(k).and_then(|v| v.strip_prefix('=')))
        };
        let num = |k: &str| get(k).and_then(|v| v.parse::<f32>().ok());
        let plan_word = get("plan")?.replace('_', " ");
        let plan = STYLES.iter().find(|s| s.name == plan_word)?.name;
        let mut accent = [0i8; 16];
        for (a, v) in accent.iter_mut().zip(get("accent")?.split(',')) {
            *a = v.parse().ok()?;
        }
        let accents = match get("accents")? {
            "-" => Vec::new(),
            v => v
                .split(',')
                .map(|x| x.parse().ok())
                .collect::<Option<Vec<usize>>>()?,
        };
        Some(ClipStyle {
            name: name.to_string(),
            plan,
            read: Read {
                grid: Subdivision::of_token(get("grid")?)?,
                swing: get("swing")?.parse().ok()?,
                busy: num("busy")?,
                syncopation: num("sync")?,
                rest: num("rest")?,
                accent,
                accents,
                bar_steps: get("steps")?.parse().ok()?,
                steps_per_beat: get("beat")?.parse().ok()?,
                loudness: (get("lo")?.parse().ok()?, get("hi")?.parse().ok()?),
                velocity: get("vel")?.parse().ok()?,
                ghosts: num("ghosts")?,
                lean_ms: num("lean")?,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{GROOVE, JAZZISH, MINIMAL};

    const BAR: Tick = 4 * PPQ as Tick;

    /// The boards' clip: two bars of 1/16s, twelve a bar, accents on 1, 2,
    /// 3 and the a of 3, swung `swing` % and `late_ms` behind at 120 BPM
    /// when played, on the grid when written.
    fn groove(swing: u8, late_ms: f64) -> Vec<PlainNote> {
        let felt = Swing::new(PPQ / 4, swing).unwrap();
        let late = late_ms * 120.0 * f64::from(PPQ) / 60_000.0;
        let steps = [0, 2, 3, 4, 6, 7, 8, 10, 11, 12, 14, 15];
        let mut out = Vec::new();
        for bar in 0..2 {
            for (i, &s) in steps.iter().enumerate() {
                let at = bar * BAR + s * STEP;
                let velocity = match s {
                    0 | 4 | 8 | 11 => 112,
                    _ => 62 + (i as u8 % 3) * 9,
                };
                out.push(PlainNote {
                    at: (felt.apply(at as f64) + late).round() as Tick,
                    len: STEP / 2,
                    pitch: 36 + (s % 5) as u8,
                    velocity,
                });
            }
        }
        out
    }

    fn words(r: &Read) -> Vec<(&'static str, String)> {
        r.words()
    }

    #[test]
    fn the_boards_clip_reads_as_drawn() {
        let notes = groove(56, 4.0);
        let r = read_clip(&notes, 2 * BAR, Meter::default(), 120.0, Swing::default()).unwrap();
        let w = words(&r);
        let want = [
            ("Feel", "16ths, swung 56 %"),
            ("Busy", "about 12 notes a bar"),
            ("Accents", "on 1, 2, 3 and the a of 3"),
            ("Loudness", "62 to 112"),
            ("Leans", "4 ms late"),
        ];
        for ((k, v), (wk, wv)) in w.iter().zip(want) {
            assert_eq!((*k, v.as_str()), (wk, wv));
        }
        assert_eq!(r.feel(), Feel::Push);
        assert_eq!(nearest(&r, Meter::default()).name, GROOVE.name);
    }

    #[test]
    fn written_straight_in_a_swung_clip_reads_the_clips_swing() {
        let notes = groove(50, 0.0);
        let plays = Swing::new(PPQ / 4, 58).unwrap();
        let r = read_clip(&notes, 2 * BAR, Meter::default(), 120.0, plays).unwrap();
        assert_eq!((r.grid, r.swing), (Subdivision::Sixteenths, 58));
        assert_eq!(words(&r)[4].1, "on the grid");
        let straight =
            read_clip(&notes, 2 * BAR, Meter::default(), 120.0, Swing::default()).unwrap();
        assert_eq!(words(&straight)[0].1, "16ths, straight");
    }

    #[test]
    fn swung_eighths_read_as_the_jazz_feel_and_plain_quarters_as_minimal() {
        let felt = Swing::new(PPQ / 2, 64).unwrap();
        let eighths: Vec<PlainNote> = (0..16)
            .map(|i| PlainNote {
                at: felt.apply(f64::from(i) * f64::from(PPQ / 2)).round() as Tick,
                len: STEP,
                pitch: 60,
                velocity: if i % 2 == 0 { 96 } else { 80 },
            })
            .collect();
        let r = read_clip(&eighths, 2 * BAR, Meter::default(), 100.0, Swing::default()).unwrap();
        assert_eq!(
            (r.grid, r.swing, r.feel()),
            (Subdivision::Eighths, 64, Feel::Swing8)
        );
        assert_eq!(nearest(&r, Meter::default()).name, JAZZISH.name);
        let quarters: Vec<PlainNote> = (0..8)
            .map(|i| PlainNote {
                at: i * Tick::from(PPQ),
                len: Tick::from(PPQ),
                pitch: 60,
                velocity: 90,
            })
            .collect();
        let q = read_clip(
            &quarters,
            2 * BAR,
            Meter::default(),
            100.0,
            Swing::default(),
        )
        .unwrap();
        assert_eq!((q.feel(), q.busy.round() as i32), (Feel::Simplify, 4));
        assert_eq!(nearest(&q, Meter::default()).name, MINIMAL.name);
        assert_eq!(words(&q)[2].1, "even");
    }

    #[test]
    fn a_clip_style_keeps_its_plan_and_plays_the_clips_feel() {
        let r = read_clip(
            &groove(56, 4.0),
            2 * BAR,
            Meter::default(),
            120.0,
            Swing::default(),
        )
        .unwrap();
        let cs = ClipStyle::new("Clip 1", r, Meter::default());
        assert_eq!(cs.name, "Clip 1 groove");
        let st = cs.style();
        assert_eq!(st.harmony, GROOVE.harmony);
        assert_eq!(st.development, GROOVE.development);
        assert_eq!(st.rhythm.swing, Swing::new(PPQ / 4, 56).unwrap());
        assert_eq!((st.rhythm.density * 4.0).round() as i32, 12);
        assert_eq!(st.playing.accent[0], cs.read.accent[0]);
        assert!(st.playing.accent[0] > 10 && st.playing.accent[2] < 0);
        assert!((st.playing.lean_ms - 4.0).abs() < 0.6);
        let back = ClipStyle::from_text(&cs.to_text()).unwrap();
        assert_eq!(back.name, cs.name);
        assert_eq!(back.plan, cs.plan);
        assert_eq!(back.read.accent, cs.read.accent);
        assert_eq!(back.read.accents, cs.read.accents);
        assert_eq!(back.style().rhythm, st.rhythm);
        assert_eq!(back.style().playing.accent, st.playing.accent);
        assert!(ClipStyle::from_text("plan=Polka grid=8 name=x").is_none());
        // Under its own name, kept once however often it is made.
        assert_eq!(st.name, "Clip 1 groove");
        assert!(std::ptr::eq(st.name, back.style().name));
    }

    /// A Style row's list: the built-in styles, then the made ones; past
    /// the end (an UNDO took one away), the last there is; a line that
    /// doesn't read is left out.
    #[test]
    fn style_rows_run_on_into_the_made_styles() {
        let r = read_clip(
            &groove(56, 4.0),
            2 * BAR,
            Meter::default(),
            120.0,
            Swing::default(),
        )
        .unwrap();
        let cs = ClipStyle::new("Riff", r, Meter::default());
        let made = clip_styles(&[cs.to_text(), "plan=Polka name=bad".to_string()]);
        assert_eq!(made.len(), 1);
        assert_eq!(style_count(&made), STYLES.len() + 1);
        assert_eq!(style_count(&[]), STYLES.len());
        assert_eq!(style_name(0, &made), STYLES[0].name);
        assert_eq!(style_name(STYLES.len(), &made), "Riff groove");
        assert_eq!(style_at(STYLES.len(), &made).name, "Riff groove");
        assert_eq!(style_at(STYLES.len() + 4, &made).name, "Riff groove");
        assert_eq!(
            style_at(STYLES.len() + 4, &[]).name,
            STYLES[STYLES.len() - 1].name
        );
        assert_eq!(style_name(STYLES.len(), &[]), STYLES[STYLES.len() - 1].name);
    }

    #[test]
    fn steps_are_named_as_a_drummer_counts() {
        assert_eq!(step_words(0, 4), "1");
        assert_eq!(step_words(5, 4), "the e of 2");
        assert_eq!(step_words(10, 4), "the and of 3");
        assert_eq!(step_words(11, 4), "the a of 3");
        assert_eq!(step_words(1, 2), "the e of 1");
    }
}
