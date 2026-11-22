//! Phrases (phase 0C): the notes of each new bar. The second prototype's
//! method on mpfree's types, with contract 6's rules.
//!
//! Each new bar gets its rhythm and a target from its job (your bar
//! again, varied, moved up the scale, answered, developed, or something
//! new), then notes from a small beam search over places in the scale.
//! Every candidate is scored on how you move from note to note (a model
//! learned from your notes only), the style's leaps, the chord under it,
//! leaps and repeats, register following the tension, the target's
//! shape, and for the last note the way into the next bar and the
//! ending. The finished line nearest the bar's similarity aim wins.
//!
//! Only the range you picked, the instrument's range, locked bars and the
//! Keeps never give way. A leap over an octave, a chromatic note that
//! doesn't move on by a half step and an ending's last note are
//! preferences weighed by the style ([`Weights`]): rare, never banned,
//! and counted when they give way ([`Line::gave_way`]).
//!
//! Trying a bar again is filling a gap (contract 1): everything around it
//! stays, its first note follows the bar before and its last note leads
//! into the bar after, and a loop's last bar leads into its first.

use mpfree_context::chord::{Chord, ChordTimeline, Quality, Tone};
use mpfree_context::groove::STEP;
use mpfree_context::key::{Key, KeyReading, Mode};
use mpfree_transport::time::{Meter, Tick};

use crate::dice::{Dice, DiceName, Job};
use crate::event::{NoteDecision, Source};
use crate::motif::{bar_of, line, motifs, similarity, transform, GridNote, Transform};
use crate::plan::{BarJob, BarPlan};
use crate::profile::{InstrumentKind, Profile};
use crate::realize::{decisions_of, Fit, PlainNote};
use crate::rhythm::{feel, fresh, rhythm_of, vary, Grid, Onset};
use crate::rules::{RangeSetting, Rule, Suggestion, Unmet, Weights};
use crate::settings::{Ending, Settings};
use crate::style::{MelodyStyle, Style};

/// Steps the model knows: -7 to 7 scale places.
const SPAN: usize = 15;

/// How you move from note to note: odds of each next step in the scale
/// after the last one or two (a variable-order model, orders 0 to 2).
/// Learned only from notes a person played (contract 8).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    rows: Vec<(usize, [i8; 2], [f32; SPAN])>,
    /// Steps learned from.
    pub steps: usize,
}

impl Model {
    pub fn learn(line: &[GridNote]) -> Model {
        let iv: Vec<i8> = line
            .windows(2)
            .map(|w| (w[1].pos - w[0].pos).clamp(-7, 7) as i8)
            .collect();
        let mut rows: Vec<(usize, [i8; 2], [f32; SPAN])> = Vec::new();
        for i in 0..iv.len() {
            for order in 0..=2usize.min(i) {
                let mut context = [0i8; 2];
                for (j, c) in context.iter_mut().enumerate().take(order) {
                    *c = iv[i - 1 - j];
                }
                let next = (iv[i] + 7) as usize;
                match rows.iter_mut().find(|r| r.0 == order && r.1 == context) {
                    Some(r) => r.2[next] += 1.0,
                    None => {
                        let mut counts = [0f32; SPAN];
                        counts[next] = 1.0;
                        rows.push((order, context, counts));
                    }
                }
            }
        }
        Model {
            rows,
            steps: iv.len(),
        }
    }

    /// Odds of each next step (-7 to 7) after `history` (latest last):
    /// the orders mixed, longer contexts trusted more once seen.
    pub fn odds(&self, history: &[i8]) -> [f64; SPAN] {
        let mut mix = [0.0; SPAN];
        let mut total = 0.0;
        for order in 0..=2usize.min(history.len()) {
            let mut context = [0i8; 2];
            for (j, c) in context.iter_mut().enumerate().take(order) {
                *c = history[history.len() - 1 - j];
            }
            if let Some(r) = self.rows.iter().find(|r| r.0 == order && r.1 == context) {
                let seen: f64 = r.2.iter().map(|&x| f64::from(x)).sum();
                let w = seen.min(4.0) * ((order + 1) as f64).powi(2);
                for (m, &x) in mix.iter_mut().zip(&r.2) {
                    *m += w * f64::from(x) / seen;
                }
                total += w;
            }
        }
        if total > 0.0 {
            mix.iter_mut().for_each(|x| *x /= total);
        }
        mix
    }
}

/// The style's odds of each step, by size.
fn style_odds(m: &MelodyStyle) -> [f64; SPAN] {
    let mut o = [0.0; SPAN];
    for (i, x) in o.iter_mut().enumerate() {
        *x = f64::from(m.intervals[(i as i32 - 7).unsigned_abs() as usize]);
    }
    let s: f64 = o.iter().sum();
    if s > 0.0 {
        o.iter_mut().for_each(|x| *x /= s);
    }
    o
}

/// What a pitch is over a chord, for scoring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Over {
    Tone,
    Seventh,
    Colour,
    Scale,
    Outside,
}

fn over(chord: Chord, pitch: u8, key: Key) -> Over {
    match chord.tone_of(i32::from(pitch)) {
        Some(Tone::Seventh) => Over::Seventh,
        Some(_) => Over::Tone,
        None if key.has(i32::from(pitch)) => {
            let iv = (i32::from(pitch) - i32::from(chord.root)).rem_euclid(12);
            let major = matches!(
                chord.quality,
                Quality::Major | Quality::Major7 | Quality::Dominant7
            );
            if iv == 2 || iv == 9 || (iv == 5 && !major) {
                Over::Colour
            } else {
                Over::Scale
            }
        }
        None => Over::Outside,
    }
}

/// Your tune, read for writing more of it.
#[derive(Clone, Debug)]
pub struct Phrasing<'a> {
    /// The key places are counted in: the key, or with no clear key the
    /// likeliest one, with new notes kept to the notes you played.
    pub key: Key,
    pub clear: bool,
    pub played_pcs: u16,
    /// The chords under every bar, yours and new.
    pub chords: &'a ChordTimeline,
    pub style: &'a Style,
    /// Your notes as one line, over your bars.
    pub yours: Vec<GridNote>,
    pub your_bars: usize,
    pub model: Model,
    /// The middle of what you played, as a scale place.
    pub centre: i32,
    /// The pitch classes you played, longest first.
    pub pitch_set: Vec<u8>,
    /// Your notes per beat.
    pub density: f64,
    pub grid: Grid,
    pub profile: Profile,
    /// The part's number, for its dice.
    pub part: u16,
    /// Each bar's last note looks at the bar after when it is already
    /// there (off only to measure what that is worth).
    pub two_sided: bool,
    /// The clip loops: its last bar leads into its first.
    pub loops: bool,
}

impl<'a> Phrasing<'a> {
    /// Read `notes` (with who made each) over `your_bars` bars. Only
    /// notes a person played are learned from; the rest are left out.
    /// The melody plays on a one-note lead unless `profile` is changed.
    pub fn new(
        key: &KeyReading,
        played_pcs: u16,
        chords: &'a ChordTimeline,
        style: &'a Style,
        notes: &[(PlainNote, Source)],
        your_bars: usize,
        meter: Meter,
    ) -> Phrasing<'a> {
        let clear = key.key().is_some();
        let key = key.best().unwrap_or(Key::new(0, Mode::Major));
        let grid = Grid::of(meter);
        let your_bars = your_bars.max(1);
        let mine: Vec<PlainNote> = notes
            .iter()
            .filter(|(_, s)| s.learnable())
            .map(|(n, _)| *n)
            .filter(|n| n.at < your_bars as Tick * Tick::from(grid.bar) * STEP)
            .collect();
        let yours = line(&mine, key);
        let model = Model::learn(&yours);
        let profile = InstrumentKind::OneNote.profile();
        let centre = if yours.is_empty() {
            key.position((profile.low / 2 + profile.high / 2).min(127))
                .0
        } else {
            (yours.iter().map(|m| f64::from(m.pos)).sum::<f64>() / yours.len() as f64).round()
                as i32
        };
        let mut weight = [0.0f64; 12];
        for m in &yours {
            weight[usize::from(m.pitch(key) % 12)] += f64::from(m.len);
        }
        let mut pitch_set: Vec<u8> = (0..12u8)
            .filter(|&p| weight[usize::from(p)] > 0.0)
            .collect();
        pitch_set.sort_by(|a, b| weight[usize::from(*b)].total_cmp(&weight[usize::from(*a)]));
        let beats = f64::from((grid.bar / grid.beat).max(1));
        let density = yours.len() as f64 / (your_bars as f64 * beats);
        Phrasing {
            key,
            clear,
            played_pcs,
            chords,
            style,
            yours,
            your_bars,
            model,
            centre,
            pitch_set,
            density,
            grid,
            profile,
            part: 0,
            two_sided: true,
            loops: false,
        }
    }

    fn dice(&self, settings: &Settings, job: Job, bar: usize, attempt: u16) -> Dice {
        DiceName {
            seed: settings.seed,
            part: self.part,
            phrase: 0,
            job,
            bar: bar as u16,
            attempt,
        }
        .dice()
    }

    /// The lowest and highest pitch new notes may use with `range`: inside
    /// the range you picked around the middle of your tune, and inside
    /// the instrument's.
    fn pitches(&self, range: RangeSetting) -> Option<(u8, u8)> {
        let lo = self
            .key
            .pitch_at(self.centre - range.steps(), 0)
            .max(self.profile.low);
        let hi = self
            .key
            .pitch_at(self.centre + range.steps(), 0)
            .min(self.profile.high);
        (lo <= hi).then_some((lo, hi))
    }

    /// Refused: the setting in the way and the one change that fixes it.
    fn unmet(&self, rule: Rule, bar: usize, range: RangeSetting) -> Unmet {
        let mut wider = range.wider();
        while let Some(w) = wider {
            if rule != Rule::RangeSetting || self.pitches(w).is_some() {
                break;
            }
            wider = w.wider();
        }
        let rule = if rule == Rule::RangeSetting && wider.is_none() {
            Rule::InstrumentRange
        } else {
            rule
        };
        Unmet {
            rule,
            bar: Some(bar as u16 + 1),
            range: (rule == Rule::RangeSetting).then_some(range),
            suggestion: wider.map(Suggestion::Range),
        }
    }
}

/// One planned note of a bar: where it starts, how long, and the note of
/// your bar (as the job reshapes it) it aims at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Planned {
    pub on: i32,
    pub len: i32,
    pub target: Option<GridNote>,
}

fn targets(rhythm: &[Onset], tune: &[GridNote]) -> Vec<Planned> {
    rhythm
        .iter()
        .map(|r| Planned {
            on: r.on,
            len: r.len,
            target: tune
                .iter()
                .rev()
                .find(|m| m.on <= r.on)
                .or(tune.first())
                .copied(),
        })
        .collect()
}

/// The rhythm and targets of one new bar, from its job.
pub fn bar_material(
    ph: &Phrasing,
    bp: &BarPlan,
    your_bars: &[Vec<GridNote>],
    settings: &Settings,
    dice: &mut Dice,
) -> Vec<Planned> {
    let style = ph.style;
    let grid = ph.grid;
    let half = grid.half();
    let n = your_bars.len();
    if n == 0 {
        return Vec::new();
    }
    let src = &your_bars[bp.source.min(n - 1)];
    let other = &your_bars[(bp.source + 1) % n];
    let aim = f64::from(bp.similarity);
    let act = f64::from(bp.activity);
    let amount = (f64::from(style.development.variation) + f64::from(settings.variation) * 0.5)
        * (1.0 - aim).max(0.08)
        * 2.0;
    let keep = settings.keeps.rhythm;
    // Your bar's rhythm, varied by `share` of the amount (when `vary_it`)
    // and given the style's feel, unless Keep Rhythm holds it as yours.
    let reshaped = |tune: &[GridNote], share: f64, vary_it: bool, dice: &mut Dice| {
        let mut r = rhythm_of(tune, grid);
        if !keep {
            if vary_it {
                vary(&mut r, amount * share, act, grid, dice);
            }
            feel(&style.rhythm, &mut r, act, aim, grid, dice);
        }
        r
    };
    match bp.job {
        BarJob::Yours => Vec::new(),
        BarJob::Repeat | BarJob::Loop | BarJob::Return => {
            let vary_it = bp.job != BarJob::Loop || dice.chance(0.5);
            let r = reshaped(src, 0.6, vary_it, dice);
            targets(&r, src)
        }
        BarJob::Sequence => {
            let moved = transform(src, Transform::Move(bp.shift));
            let r = reshaped(src, 0.6, true, dice);
            targets(&r, &moved)
        }
        BarJob::Vary => {
            let mut r = rhythm_of(src, grid);
            if !keep {
                vary(&mut r, amount, act, grid, dice);
            }
            let tune = if dice.chance(0.3) {
                transform(src, Transform::Widen(1.5))
            } else {
                src.clone()
            };
            if !keep {
                feel(&style.rhythm, &mut r, act, aim, grid, dice);
            }
            targets(&r, &tune)
        }
        BarJob::Answer => {
            // The first half of the call, the second half of your other
            // bar, ending on another note than the call did.
            let mut tune: Vec<GridNote> = src.iter().filter(|m| m.on < half).copied().collect();
            tune.extend(other.iter().filter(|m| m.on >= half));
            let home = ph.centre - ph.centre.rem_euclid(7);
            let call_end = src.last().map_or(0, |m| m.pos.rem_euclid(7));
            let end = if call_end == 0 { home + 4 } else { home };
            let tune = transform(&tune, Transform::EndOn(end));
            let r = reshaped(&tune, 0.5, true, dice);
            targets(&r, &tune)
        }
        BarJob::Develop => {
            // The opening motif twice, the second time a step higher.
            let mut head = motifs(src).first().cloned().unwrap_or_else(|| src.clone());
            head.retain(|m| m.on < half);
            if head.len() < 2 {
                head = src.iter().filter(|m| m.on < half).copied().collect();
            }
            if head.last().is_some_and(|m| m.on + m.len > half) {
                head = transform(&head, Transform::Diminish);
            }
            let o = head.first().map_or(0, |m| m.on);
            let a = transform(&head, Transform::Shift(-o));
            let b = transform(
                &transform(&a, Transform::Move(bp.shift.max(1))),
                Transform::Shift(half),
            );
            let mut tune = a;
            tune.extend(b);
            tune.retain(|m| m.on < grid.bar);
            let r = reshaped(&tune, 0.4, true, dice);
            targets(&r, &tune)
        }
        BarJob::Contrast => {
            let r = if keep {
                rhythm_of(src, grid)
            } else {
                let avoid: Vec<i32> = src.iter().map(|m| m.on).collect();
                fresh(&style.rhythm, act, &avoid, grid, dice)
            };
            let tune = transform(&transform(src, Transform::Invert), Transform::Move(2));
            targets(&r, &tune)
        }
        BarJob::Land(_) => {
            if keep {
                return targets(&rhythm_of(src, grid), src);
            }
            // The first half of your bar, then a long last note.
            let mut r: Vec<Onset> = rhythm_of(src, grid)
                .into_iter()
                .filter(|x| x.on < half)
                .collect();
            if let Some(l) = r.last_mut() {
                l.len = l.len.min(half - l.on).max(1);
            }
            let land = if style.playing.legato || dice.chance(0.5) {
                half
            } else {
                (half + 2).min(grid.bar - 1)
            };
            if r.last().is_none_or(|l| l.on + l.len <= land) {
                r.push(Onset {
                    on: land,
                    len: grid.bar - land,
                });
            }
            feel(&style.rhythm, &mut r, act, aim, grid, dice);
            targets(&r, src)
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Beam {
    pos: Vec<i32>,
    alt: Vec<i8>,
    cost: f64,
    broke: u8,
}

const LEAP: u8 = 1;
const CHROMATIC: u8 = 1 << 1;
const ENDING: u8 = 1 << 2;

fn broken(bits: u8) -> Vec<Rule> {
    [
        (LEAP, Rule::LeapSize),
        (CHROMATIC, Rule::ChromaticMotion),
        (ENDING, Rule::EndingTune),
    ]
    .iter()
    .filter(|(b, _)| bits & b != 0)
    .map(|&(_, r)| r)
    .collect()
}

/// One new bar's notes.
#[derive(Clone, Debug, PartialEq)]
pub struct BarOut {
    pub notes: Vec<GridNote>,
    /// How alike its tune is to what it aimed at.
    pub similarity: f64,
    /// Preferences it gave way on.
    pub gave_way: Vec<Rule>,
    /// Candidates scored: the fixed budget a bar costs.
    pub tried: u64,
}

/// Lines kept at each note.
pub const BEAM: usize = 12;

/// Notes for one bar's rhythm: a beam search over pitches in range.
/// `before` is the line's last notes before the bar, `next_first` the
/// place of the next bar's first note when it is already there.
#[allow(clippy::too_many_arguments)]
pub fn fit(
    ph: &Phrasing,
    bp: &BarPlan,
    bar: usize,
    planned: &[Planned],
    before: &[(i32, i8)],
    next_first: Option<i32>,
    src_bar: &[GridNote],
    settings: &Settings,
    dice: &mut Dice,
) -> Result<BarOut, Unmet> {
    let style = ph.style;
    let key = ph.key;
    let w: Weights = style.weights;
    let grid = ph.grid;
    let Some((plo, phi)) = ph.pitches(settings.range) else {
        return Err(ph.unmet(Rule::RangeSetting, bar, settings.range));
    };
    let similarity_set = f64::from(settings.similarity);
    let aim = f64::from(bp.similarity);
    let sty = style_odds(&style.melody);
    let learned = ph.model.steps as f64;
    let alpha = learned / (learned + 12.0) * (0.4 + 0.6 * similarity_set);
    let temp = 0.1 + 0.9 * f64::from(settings.variation);
    let w_target = 0.4 + 2.0 * aim;
    let register = ph.centre + (f64::from(bp.tension) * 4.0 - 1.5).round() as i32;
    let home = ph.centre - ph.centre.rem_euclid(7);
    let first_yours = ph.yours.first().map_or(ph.centre, |m| m.pos);
    let miss = 3.0 * f64::from(w.ending_tune);
    let all_pcs = ph.played_pcs & 0x0FFF;
    let mut beams = vec![Beam::default()];
    let mut tried = 0u64;
    for (i, r) in planned.iter().enumerate() {
        let tick = Tick::from(bar as i32 * grid.bar + r.on) * STEP;
        let chord = ph.chords.span_at(tick).and_then(|s| s.answer.chord());
        let strong = grid.on_beat(r.on);
        let last = i + 1 == planned.len();
        let prev_strong = i > 0 && grid.on_beat(planned[i - 1].on);
        let target_step = match (r.target, i.checked_sub(1).and_then(|j| planned[j].target)) {
            (Some(t), Some(q)) => Some(t.pos - q.pos),
            _ => None,
        };
        let mut next: Vec<(usize, i32, i8, f64, u8)> = Vec::with_capacity(beams.len() * 32);
        for (bi, st) in beams.iter().enumerate() {
            let mut h: Vec<(i32, i8)> = before.to_vec();
            h.extend(st.pos.iter().copied().zip(st.alt.iter().copied()));
            let prev = h.last().copied();
            let steps: Vec<i8> = h
                .windows(2)
                .rev()
                .take(2)
                .map(|x| (x[1].0 - x[0].0).clamp(-7, 7) as i8)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let um = ph.model.odds(&steps);
            for p in plo..=phi {
                let (pos, alt) = key.position(p);
                let pc = p % 12;
                if !ph.clear {
                    // No clear key: only the notes you played.
                    if all_pcs != 0 && all_pcs & (1 << pc) == 0 {
                        continue;
                    }
                } else if alt != 0 && (style.melody.chromatic <= 0.0 || strong || last) {
                    continue;
                }
                // Keep Tune shape: every step goes the way yours did.
                if settings.keeps.tune_shape {
                    if let (Some(ts), Some((pp, _))) = (target_step, prev) {
                        if (pos - pp).signum() != ts.signum() {
                            continue;
                        }
                    }
                }
                tried += 1;
                let mut c = 0.0;
                let mut broke = st.broke;
                let iv = prev.map_or(0, |(pp, _)| (pos - pp).clamp(-7, 7));
                if let Some((pp, pa)) = prev {
                    let semis = (i32::from(p) - i32::from(key.pitch_at(pp, pa))).abs();
                    if semis > 12 {
                        c += f64::from(w.leap) * (8.0 + f64::from(semis - 12));
                        broke |= LEAP;
                    }
                    if pa != 0 && semis != 1 {
                        c += 2.0 * f64::from(w.chromatic);
                        broke |= CHROMATIC;
                    }
                }
                // 1. Your steps, blended with the style's.
                let ix = (iv + 7) as usize;
                let pr = alpha * um[ix] + (1.0 - alpha) * sty[ix];
                c -= pr.max(1e-4).ln();
                // 2. The chord under it.
                if let Some(ch) = chord {
                    let m = &style.melody;
                    c += match over(ch, p, key) {
                        Over::Tone => 0.0,
                        Over::Seventh => {
                            if style.harmony.sevenths {
                                0.1
                            } else {
                                0.6
                            }
                        }
                        Over::Colour => f64::from(m.colour) * if strong { 1.3 } else { 0.7 },
                        Over::Scale if strong => {
                            f64::from(m.off_chord[0]) * (1.0 - 0.4 * f64::from(bp.tension))
                        }
                        Over::Scale => f64::from(m.off_chord[1]),
                        Over::Outside => 1.3 - f64::from(m.chromatic),
                    };
                    // A note off the chord on a beat moves on by step.
                    if let Some((pp, pa)) = prev {
                        let q = key.pitch_at(pp, pa);
                        if prev_strong
                            && matches!(over(ch, q, key), Over::Scale | Over::Colour)
                            && (i32::from(p) - i32::from(q)).abs() > 2
                        {
                            c += 1.0;
                        }
                    }
                }
                // 3. Leaps cost, and a leap turns back.
                if iv.abs() > 3 {
                    c += f64::from(iv.abs() - 3) * f64::from(w.leap);
                }
                if h.len() >= 2 {
                    let piv = h[h.len() - 1].0 - h[h.len() - 2].0;
                    if piv.abs() > 3 && iv.signum() == piv.signum() && iv != 0 {
                        c += 0.8;
                    }
                }
                // 4. Repeats, and the notes you used.
                if h.len() >= 2 && h[h.len() - 1].0 == pos && h[h.len() - 2].0 == pos {
                    c += f64::from(w.repeats) * 2.0;
                }
                let set = usize::from(style.melody.pitch_set);
                if set > 0 && !ph.pitch_set.iter().take(set).any(|&x| x == pc) {
                    c += 2.5;
                }
                // 5. Register follows tension.
                c += 0.04 * f64::from((pos - register).pow(2));
                // 6. The target's place and steps.
                if let Some(t) = r.target {
                    let mut mc = 0.4 * f64::from((pos - t.pos).abs()) / 3.0;
                    if let (Some(ts), Some(_)) = (target_step, prev) {
                        mc += 0.6 * f64::from((iv - ts).abs()) / 2.0;
                    }
                    c += w_target * mc.min(3.0);
                }
                // 7. The way out, and the ending.
                if last {
                    if let Some(nf) = next_first {
                        let d = (nf - pos).abs();
                        if d > 2 {
                            c += 0.6 * f64::from(d - 2);
                        }
                    }
                    if let BarJob::Land(e) = bp.job {
                        let d = pos.rem_euclid(7);
                        let cost = match e {
                            Ending::Home if d == 0 => 0.0,
                            Ending::Home if d == 2 || d == 4 => 0.25 * miss,
                            Ending::Open if matches!(d, 1 | 4 | 6) => 0.0,
                            Ending::LoopsBack if (pos - first_yours).abs() <= 2 || d == 4 => 0.0,
                            _ => miss,
                        };
                        if cost >= miss && miss > 0.0 {
                            broke |= ENDING;
                        }
                        c += cost;
                        if e == Ending::Home {
                            c += 0.1 * f64::from((pos - home).abs());
                        }
                    }
                }
                next.push((bi, pos, alt, st.cost + c - temp * dice.gumbel(), broke));
            }
        }
        if next.is_empty() {
            let rule = if settings.keeps.tune_shape {
                Rule::KeepTuneShape
            } else {
                Rule::RangeSetting
            };
            let mut u = ph.unmet(rule, bar, settings.range);
            if rule == Rule::KeepTuneShape {
                u.suggestion = settings.range.wider().map(Suggestion::Range);
            }
            return Err(u);
        }
        let keep = BEAM.min(next.len());
        next.select_nth_unstable_by(keep - 1, |a, b| a.3.total_cmp(&b.3));
        next.truncate(keep);
        next.sort_by(|a, b| a.3.total_cmp(&b.3));
        beams = next
            .iter()
            .map(|&(bi, pos, alt, cost, broke)| {
                let mut st = beams[bi].clone();
                st.pos.push(pos);
                st.alt.push(alt);
                st.cost = cost;
                st.broke = broke;
                st
            })
            .collect();
    }
    // The finished line nearest the aim, measured against the planned
    // material (your bar as the job reshapes it), or against your bar
    // itself for a contrast.
    let velocity = |r: &Planned| r.target.map_or(style.playing.velocity, |t| t.velocity);
    let planned_tune: Option<Vec<GridNote>> = if bp.job == BarJob::Contrast {
        None
    } else {
        planned
            .iter()
            .map(|r| {
                r.target.map(|t| GridNote {
                    on: r.on,
                    len: r.len,
                    pos: t.pos,
                    alt: t.alt,
                    velocity: t.velocity,
                })
            })
            .collect()
    };
    let reference: &[GridNote] = planned_tune.as_deref().unwrap_or(src_bar);
    let min_cost = beams.iter().map(|b| b.cost).fold(f64::MAX, f64::min);
    let mut best: Option<(f64, Vec<GridNote>, f64, u8)> = None;
    for st in &beams {
        let notes: Vec<GridNote> = planned
            .iter()
            .zip(st.pos.iter().zip(&st.alt))
            .map(|(r, (&pos, &alt))| GridNote {
                on: r.on,
                len: r.len,
                pos,
                alt,
                velocity: velocity(r),
            })
            .collect();
        let sim = similarity(key, &notes, reference).tune();
        let score =
            (st.cost - min_cost) / planned.len().max(1) as f64 * 0.5 + 4.0 * (sim - aim).abs();
        if best.as_ref().is_none_or(|b| score < b.0) {
            best = Some((score, notes, sim, st.broke));
        }
    }
    let (_, notes, sim, broke) = best.unwrap_or((0.0, Vec::new(), 1.0, 0));
    Ok(BarOut {
        notes,
        similarity: sim,
        gave_way: broken(broke),
        tried,
    })
}

/// The whole line: your bars, then the new ones.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Every note, in steps from the start of your first bar.
    pub notes: Vec<GridNote>,
    /// Each new bar's similarity to what it aimed at (`None` for yours
    /// and locked bars).
    pub similarity: Vec<Option<f64>>,
    /// Preferences that gave way, by bar (from 1): counted and shown,
    /// never hidden.
    pub gave_way: Vec<(u16, Rule)>,
    pub tried: u64,
}

impl Line {
    /// Bar `bar`'s notes, from its start.
    pub fn bar(&self, bar: usize, grid: Grid) -> Vec<GridNote> {
        bar_of(&self.notes, bar as i32, grid.bar)
    }
}

/// Carry the tune on over the plan. `locks[b]` keeps bar `b` as given
/// (its notes from the bar's start); `attempts[b]` is that bar's Try
/// again count. Refused, with one suggested change, only when the range
/// you picked, the instrument or the Keeps leave no way.
pub fn continue_line(
    ph: &Phrasing,
    plan: &[BarPlan],
    settings: &Settings,
    locks: &[Option<Vec<GridNote>>],
    attempts: &[u16],
) -> Result<Line, Unmet> {
    let gb = ph.grid.bar;
    let your_bars: Vec<Vec<GridNote>> = (0..ph.your_bars)
        .map(|b| bar_of(&ph.yours, b as i32, gb))
        .collect();
    let first_new = ph.your_bars.min(plan.len());
    let mut notes: Vec<GridNote> = ph
        .yours
        .iter()
        .filter(|m| m.on < first_new as i32 * gb)
        .copied()
        .collect();
    let mut sims = vec![None; first_new];
    let mut gave_way = Vec::new();
    let mut tried = 0;
    for (b, bp) in plan.iter().enumerate().skip(first_new) {
        let off = b as i32 * gb;
        if let Some(Some(l)) = locks.get(b) {
            notes.extend(l.iter().map(|m| GridNote {
                on: m.on + off,
                ..*m
            }));
            sims.push(None);
            continue;
        }
        let attempt = attempts.get(b).copied().unwrap_or(0);
        let mut rd = ph.dice(settings, Job::Rhythm, b, attempt);
        let mut pd = ph.dice(settings, Job::Pitch, b, attempt);
        let planned = bar_material(ph, bp, &your_bars, settings, &mut rd);
        let before: Vec<(i32, i8)> = notes
            .iter()
            .rev()
            .take(3)
            .rev()
            .map(|m| (m.pos, m.alt))
            .collect();
        let next_first = if !ph.two_sided {
            None
        } else if b + 1 < plan.len() {
            locks
                .get(b + 1)
                .and_then(|l| l.as_ref())
                .and_then(|l| l.first())
                .map(|m| m.pos)
        } else if ph.loops {
            notes
                .iter()
                .filter(|m| m.on < gb)
                .min_by_key(|m| m.on)
                .map(|m| m.pos)
        } else {
            None
        };
        let src_bar: &[GridNote] = your_bars
            .get(bp.source.min(your_bars.len().saturating_sub(1)))
            .map_or(&[], |v| v.as_slice());
        let out = fit(
            ph, bp, b, &planned, &before, next_first, src_bar, settings, &mut pd,
        )?;
        tried += out.tried;
        gave_way.extend(out.gave_way.iter().map(|&r| (b as u16 + 1, r)));
        sims.push(Some(out.similarity));
        notes.extend(out.notes.iter().map(|m| GridNote {
            on: m.on + off,
            ..*m
        }));
    }
    Ok(Line {
        notes,
        similarity: sims,
        gave_way,
        tried,
    })
}

/// Try bar `bar` of `line` again: every other bar stays exactly as it is,
/// and the new bar is scored on its way in and its way out (and a loop's
/// last bar on the way back to bar 1).
pub fn try_again(
    ph: &Phrasing,
    plan: &[BarPlan],
    settings: &Settings,
    line: &Line,
    bar: usize,
    attempt: u16,
) -> Result<Line, Unmet> {
    let mut locks: Vec<Option<Vec<GridNote>>> = (0..plan.len())
        .map(|b| Some(line.bar(b, ph.grid)))
        .collect();
    let mut attempts = vec![0u16; plan.len()];
    if let Some(l) = locks.get_mut(bar) {
        *l = None;
        attempts[bar] = attempt;
    }
    let mut out = continue_line(ph, plan, settings, &locks, &attempts)?;
    // The other bars' measures are the line's as it was.
    for (b, s) in out.similarity.iter_mut().enumerate() {
        if b != bar {
            *s = line.similarity.get(b).copied().flatten();
        }
    }
    let mut gave_way: Vec<(u16, Rule)> = line
        .gave_way
        .iter()
        .filter(|(b, _)| usize::from(*b) != bar + 1)
        .copied()
        .collect();
    gave_way.extend(
        out.gave_way
            .iter()
            .filter(|(b, _)| usize::from(*b) == bar + 1),
    );
    gave_way.sort_by_key(|g| g.0);
    out.gave_way = gave_way;
    Ok(out)
}

/// The new bars' decisions, read back from the line under the chords, so
/// a later chord change re-fits them (contract 2). The register is the
/// first new note.
pub fn decisions(ph: &Phrasing, line: &Line, from_bar: usize) -> (u8, Vec<NoteDecision>) {
    let from = from_bar as i32 * ph.grid.bar;
    let notes: Vec<PlainNote> = line
        .notes
        .iter()
        .filter(|m| m.on >= from)
        .map(|m| m.plain(ph.key))
        .collect();
    let fit = Fit {
        harmony: ph.chords,
        key: ph.clear.then_some(ph.key),
        played_pcs: ph.played_pcs,
        origin: 0,
    };
    decisions_of(&notes, &fit, Source::Mpfree)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Part, PartId};
    use crate::plan::plan;
    use crate::progression::progression;
    use crate::realize::realize;
    use crate::style::{GROOVE, JAZZISH, MINIMAL};
    use mpfree_context::listen::{listen, Played};
    use mpfree_context::{PartRole, ReadBudget};

    fn riff() -> Vec<PlainNote> {
        [
            (0, 2, 60, 100),
            (2, 1, 63, 88),
            (3, 1, 67, 92),
            (4, 2, 65, 96),
            (6, 2, 63, 86),
            (8, 3, 62, 94),
            (11, 1, 63, 80),
            (12, 4, 60, 90),
            (16, 2, 67, 100),
            (18, 1, 68, 86),
            (19, 1, 67, 84),
            (20, 2, 65, 94),
            (22, 2, 63, 84),
            (24, 2, 62, 92),
            (26, 2, 63, 84),
            (28, 4, 65, 88),
        ]
        .iter()
        .map(|&(on, len, pitch, velocity)| PlainNote {
            at: on * STEP,
            len: len * STEP,
            pitch,
            velocity,
        })
        .collect()
    }

    /// The riff read, a plan of `bars`, its chords: what a request has
    /// before the phrase.
    struct Request {
        key: KeyReading,
        played: u16,
        chords: ChordTimeline,
        plan: Vec<BarPlan>,
        settings: Settings,
        meter: Meter,
    }

    fn request(style: Style, bars: usize, seed: u64, ending: Ending) -> Request {
        request_in(style, bars, seed, ending, Meter::default())
    }

    fn request_in(style: Style, bars: usize, seed: u64, ending: Ending, meter: Meter) -> Request {
        let notes = riff();
        let played: Vec<Played> = notes
            .iter()
            .map(|n| Played::new(n.at, n.len, n.pitch, n.velocity))
            .collect();
        let heard = listen(&played, 2 * meter.bar_ticks(), ReadBudget::default(), meter);
        let settings = Settings {
            style,
            ending,
            seed,
            ..Settings::default()
        };
        let density = notes.len() as f32 / 8.0;
        let plan = plan(&settings, 2, bars, density);
        let chords = progression(
            &heard.chords,
            heard.key.key(),
            &style.harmony,
            &plan,
            settings.variation,
            seed,
            0,
            meter,
        );
        Request {
            key: heard.key,
            played: heard.played_pcs,
            chords,
            plan,
            settings,
            meter,
        }
    }

    fn phrasing(r: &Request) -> Phrasing<'_> {
        let notes: Vec<(PlainNote, Source)> =
            riff().into_iter().map(|n| (n, Source::You)).collect();
        Phrasing::new(
            &r.key,
            r.played,
            &r.chords,
            &r.settings.style,
            &notes,
            2,
            r.meter,
        )
    }

    #[test]
    fn your_steps_are_learned_from_your_notes_only() {
        let r = request(JAZZISH, 4, 1, Ending::Home);
        let mut notes: Vec<(PlainNote, Source)> =
            riff().into_iter().map(|n| (n, Source::You)).collect();
        let ph = Phrasing::new(
            &r.key,
            r.played,
            &r.chords,
            &JAZZISH,
            &notes,
            2,
            Meter::default(),
        );
        assert_eq!(ph.model.steps, 15);
        assert_eq!(ph.key.name(), "C minor");
        assert!(ph.clear);
        // Notes the composer or an arp made are never learned from.
        for (i, n) in riff().into_iter().enumerate() {
            let source = if i % 2 == 0 {
                Source::Mpfree
            } else {
                Source::Pattern
            };
            notes.push((
                PlainNote {
                    pitch: n.pitch + 7,
                    at: n.at + STEP,
                    ..n
                },
                source,
            ));
        }
        let mixed = Phrasing::new(
            &r.key,
            r.played,
            &r.chords,
            &JAZZISH,
            &notes,
            2,
            Meter::default(),
        );
        assert_eq!(mixed.model, ph.model);
        assert_eq!(mixed.yours, ph.yours);
        // The riff mostly moves by step.
        let odds = ph.model.odds(&[]);
        assert!(odds[6] + odds[8] > 0.5, "{odds:?}");
    }

    #[test]
    fn sixteen_bars_land_near_each_aim_in_range_and_home() {
        let r = request(JAZZISH, 16, 16, Ending::Home);
        let ph = phrasing(&r);
        let line = continue_line(&ph, &r.plan, &r.settings, &[], &[]).unwrap();
        let (lo, hi) = ph.pitches(r.settings.range).unwrap();
        assert!(line
            .notes
            .iter()
            .skip(16)
            .all(|m| (lo..=hi).contains(&m.pitch(ph.key))));
        // Every new bar has notes, and its tune lands near its aim.
        let mut off = 0.0;
        for b in 2..16 {
            assert!(!line.bar(b, ph.grid).is_empty(), "bar {}", b + 1);
            off += (line.similarity[b].unwrap() - f64::from(r.plan[b].similarity)).abs();
        }
        let mean = off / 14.0;
        assert!(mean < 0.15, "mean distance from the aims {mean:.3}");
        // The last note is the home note.
        let last = line.notes.last().unwrap();
        assert_eq!(last.pos.rem_euclid(7), 0);
        // Jazz-ish's chromatic notes may skip their half step (an
        // enclosure); nothing else gave way.
        assert!(
            line.gave_way.iter().all(|g| g.1 == Rule::ChromaticMotion),
            "{:?}",
            line.gave_way
        );
        // Your bars are yours, untouched.
        assert_eq!(line.notes[..16], ph.yours[..]);
        assert!(line.tried > 0);
    }

    #[test]
    fn the_same_request_makes_the_same_line_and_a_try_changes_one_bar() {
        let r = request(GROOVE, 8, 3, Ending::Home);
        let ph = phrasing(&r);
        let a = continue_line(&ph, &r.plan, &r.settings, &[], &[]).unwrap();
        let b = continue_line(&ph, &r.plan, &r.settings, &[], &[]).unwrap();
        assert_eq!(a, b);
        let mut changed = 0;
        for attempt in 1..=4 {
            let t = try_again(&ph, &r.plan, &r.settings, &a, 5, attempt).unwrap();
            for bar in (0..8).filter(|&x| x != 5) {
                assert_eq!(t.bar(bar, ph.grid), a.bar(bar, ph.grid), "bar {}", bar + 1);
            }
            if t.bar(5, ph.grid) != a.bar(5, ph.grid) {
                changed += 1;
            }
        }
        assert!(changed >= 3, "Try again gives another bar ({changed} of 4)");
    }

    /// Big steps (more than a major third) into the next bar, after trying
    /// bars again: the prototype's measurement. Returns the share two-
    /// sided and one-sided, and the share in the line as first written.
    fn big_steps_into_the_next_bar() -> (f64, f64, f64) {
        let mut counts = [(0usize, 0usize); 3];
        for seed in 0..40u64 {
            let r = request(JAZZISH, 16, 500 + seed, Ending::Home);
            let mut ph = phrasing(&r);
            let base = continue_line(&ph, &r.plan, &r.settings, &[], &[]).unwrap();
            let semis = |a: &GridNote, b: &GridNote| {
                (i32::from(a.pitch(ph.key)) - i32::from(b.pitch(ph.key))).abs()
            };
            for &b in &[2usize, 5, 9, 12] {
                if let (Some(l), Some(f)) = (
                    base.bar(b, ph.grid).last(),
                    base.bar(b + 1, ph.grid).first(),
                ) {
                    counts[0].0 += usize::from(semis(l, f) > 4);
                    counts[0].1 += 1;
                }
                for attempt in 1..=3 {
                    for (k, two) in [(1usize, true), (2, false)] {
                        ph.two_sided = two;
                        let t = try_again(&ph, &r.plan, &r.settings, &base, b, attempt).unwrap();
                        if let (Some(l), Some(f)) =
                            (t.bar(b, ph.grid).last(), t.bar(b + 1, ph.grid).first())
                        {
                            counts[k].0 += usize::from(semis(l, f) > 4);
                            counts[k].1 += 1;
                        }
                    }
                }
            }
        }
        let share = |k: usize| counts[k].0 as f64 / counts[k].1.max(1) as f64;
        (share(1), share(2), share(0))
    }

    #[test]
    fn try_again_leads_into_the_next_bar() {
        let (two, one, written) = big_steps_into_the_next_bar();
        // The prototype: 17 % two-sided, 28 % one-sided, 16 % as written.
        assert!(two < one, "two-sided {two:.2} one-sided {one:.2}");
        assert!(
            two <= 0.22,
            "two-sided {two:.2} one-sided {one:.2} written {written:.2}"
        );
    }

    #[test]
    fn a_loops_last_bar_leads_back_into_bar_one() {
        let mut big = [(0usize, 0usize); 2];
        for seed in 0..40u64 {
            let r = request(JAZZISH, 4, 900 + seed, Ending::LoopsBack);
            let mut ph = phrasing(&r);
            let base = continue_line(&ph, &r.plan, &r.settings, &[], &[]).unwrap();
            for attempt in 1..=3 {
                for (k, loops) in [(0usize, true), (1, false)] {
                    ph.loops = loops;
                    let t = try_again(&ph, &r.plan, &r.settings, &base, 3, attempt).unwrap();
                    let (l, f) = (t.bar(3, ph.grid), t.bar(0, ph.grid));
                    if let (Some(l), Some(f)) = (l.last(), f.first()) {
                        let d = (i32::from(l.pitch(ph.key)) - i32::from(f.pitch(ph.key))).abs();
                        big[k].0 += usize::from(d > 4);
                        big[k].1 += 1;
                    }
                }
            }
        }
        let share = |k: usize| big[k].0 as f64 / big[k].1 as f64;
        // The prototype: 17 % with the wrap, 53 % without.
        assert!(share(0) < share(1), "{:.2} {:.2}", share(0), share(1));
        assert!(share(0) <= 0.25, "{:.2} {:.2}", share(0), share(1));
    }

    #[test]
    fn keeps_hold_your_rhythm_and_tune_shape() {
        let mut r = request(GROOVE, 8, 5, Ending::Home);
        r.settings.keeps.rhythm = true;
        r.settings.keeps.tune_shape = true;
        let ph = phrasing(&r);
        let line = continue_line(&ph, &r.plan, &r.settings, &[], &[]).unwrap();
        for b in 2..8 {
            let bp = r.plan[b];
            let src = line.bar(bp.source, ph.grid);
            let new = line.bar(b, ph.grid);
            let on = |v: &[GridNote]| v.iter().map(|m| m.on).collect::<Vec<_>>();
            if bp.job != BarJob::Develop && bp.job != BarJob::Answer {
                assert_eq!(on(&new), on(&src), "bar {} ({})", b + 1, bp.job.words());
            }
            // Every step goes the way your bar's did.
            if bp.job == BarJob::Repeat || bp.job == BarJob::Return || bp.job == BarJob::Loop {
                for (x, y) in new.windows(2).zip(src.windows(2)) {
                    assert_eq!(
                        (x[1].pos - x[0].pos).signum(),
                        (y[1].pos - y[0].pos).signum(),
                        "bar {}",
                        b + 1
                    );
                }
            }
        }
    }

    #[test]
    fn a_range_that_cant_fit_says_which_setting_and_one_change() {
        let mut r = request(MINIMAL, 4, 1, Ending::Home);
        r.settings.range = RangeSetting::Narrow;
        let mut ph = phrasing(&r);
        // A bass under a tune around middle C: Narrow is above its range.
        ph.profile = InstrumentKind::Bass.profile();
        ph.profile.high = 52;
        let u = continue_line(&ph, &r.plan, &r.settings, &[], &[]).unwrap_err();
        assert_eq!(u.rule, Rule::RangeSetting);
        assert_eq!(
            u.words(),
            "Can't fit a new bar 3 between your bars with Range Narrow. Try Range Medium?"
        );
        // Nothing wider fits either: it's the instrument.
        ph.profile.high = 40;
        let u = continue_line(&ph, &r.plan, &r.settings, &[], &[]).unwrap_err();
        assert_eq!(u.rule, Rule::InstrumentRange);
        assert_eq!(u.suggestion, None);
    }

    #[test]
    fn no_clear_key_keeps_to_the_notes_played() {
        let mut r = request(GROOVE, 4, 2, Ending::Home);
        r.key = KeyReading::nothing();
        r.played = (1 << 0) | (1 << 3) | (1 << 7) | (1 << 10);
        let ph = phrasing(&r);
        assert!(!ph.clear);
        let line = continue_line(&ph, &r.plan, &r.settings, &[], &[]).unwrap();
        for m in line.notes.iter().filter(|m| m.on >= 32) {
            assert!(r.played & (1 << (m.pitch(ph.key) % 12)) != 0);
        }
    }

    #[test]
    fn the_new_bars_decisions_make_the_same_notes() {
        for style in [MINIMAL, GROOVE, JAZZISH] {
            let r = request(style, 8, 7, Ending::Home);
            let ph = phrasing(&r);
            let line = continue_line(&ph, &r.plan, &r.settings, &[], &[]).unwrap();
            let (register, ds) = decisions(&ph, &line, 2);
            assert!(ds.iter().all(|d| d.source == Source::Mpfree));
            let part = Part {
                id: PartId(0),
                role: PartRole::Melody,
                kind: InstrumentKind::OneNote,
                register,
                decisions: ds,
            };
            let fit = Fit {
                harmony: &r.chords,
                key: Some(ph.key),
                played_pcs: ph.played_pcs,
                origin: 0,
            };
            let again: Vec<u8> = realize(&part, &fit).iter().map(|e| e.pitch).collect();
            let written: Vec<u8> = line
                .notes
                .iter()
                .skip(16)
                .map(|m| m.pitch(ph.key))
                .collect();
            assert_eq!(again, written, "{}", style.name);
        }
    }

    #[test]
    fn bars_of_three_beats_stay_inside_their_bars() {
        let three = Meter::new(3, 4).unwrap();
        for style in [MINIMAL, GROOVE, JAZZISH] {
            let r = request_in(style, 8, 4, Ending::Home, three);
            let ph = phrasing(&r);
            assert_eq!(ph.grid.bar, 12);
            let line = continue_line(&ph, &r.plan, &r.settings, &[], &[]).unwrap();
            for b in 2..8 {
                let bar = line.bar(b, ph.grid);
                assert!(!bar.is_empty(), "{} bar {}", style.name, b + 1);
                assert!(bar.iter().all(|m| (0..12).contains(&m.on)));
            }
            assert!(line.notes.iter().all(|m| m.on < 8 * 12));
        }
    }
}
