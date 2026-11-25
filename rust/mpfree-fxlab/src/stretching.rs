//! The stretch corpus (the audio tracks rethink, phase 5c.5): what a
//! region or clip on Stretch sounds like when the song plays at another
//! tempo than the one it was made at, measured and rendered to listen to.
//!
//! - **The sounds** ([`sounds`]), all made in MPFree, no samples: a drum
//!   break (two bars at 100 BPM, the factory kit), a piano (Keys, Soft
//!   Tines), a voice (Choir, Aah Choir, one sung line), a bass (Bass,
//!   Round Mono), a guitar (Pluck, Nylon, picked), a wide pad (detuned
//!   tones, left and right apart) and a full mix of the drums, bass, piano
//!   and pad. The first six are the sampler's listening set's where it has
//!   them ([`crate::listening`]); the guitar and the mix are this set's.
//! - **The speeds** ([`SPEEDS`]): the song's tempo over the one the sound
//!   was made at, 0.8 and 1.25 (a musician's usual range) and 0.5 and 2
//!   for the extremes. Each is stretched as the tracks stretch a copy:
//!   `mpfree_stretch` with the tracks' settings
//!   ([`mpfree_audiotracks::copies::settings`]), evenly to its length at
//!   that tempo, at its own pitch.
//! - **The numbers**, each against the sound as made, where the stretch
//!   should move nothing but time:
//!   - **onsets found** (0 to 1): of the hits the onset finder picks in
//!     the sound, moved to where the bars put them (their frame over the
//!     speed), the share with a hit picked in the stretch within 30 ms;
//!   - **extra onsets**: hits picked in the stretch with none of the
//!     sound's within 30 ms, over the sound's hits: an attack heard twice,
//!     or a smear heard as a new hit;
//!   - **onset timing** (ms): how far the found ones land from where they
//!     should, the median: the rhythm;
//!   - **attack rise** (ms): how much slower the hits rise, from 10 % to
//!     90 % of their peak within 40 ms, than in the sound: sharpness;
//!   - **pre-echo** (dB): how much more the 25 ms before each hit holds,
//!     against the 20 ms after it, than in the sound: smearing ahead of
//!     the attack;
//!   - **stereo change**: how far the left and right channels'
//!     correlation moved;
//!   - **pitch** (cents): every 20 ms where the sound holds one steady
//!     note over all that both windows hear, the median distance between
//!     the stretch's pitch and the sound's at the same place in the music;
//!     its spread, the **pitch wobble** (how far 90 % of the windows stay
//!     from that median); and **pitch off**, the share of those windows a
//!     semitone or more away (the tracker taking another partial, or the
//!     stretch losing the note), counted rather than averaged in;
//!   - **phasiness**: how much less periodic those windows are (the YIN
//!     aperiodicity, 0 for a pure tone, 1 for noise, the stretch's less
//!     the sound's): the phase vocoder's swirl on held notes;
//!   - **length** (ms): how far its length is from the bars'.
//!
//! The ratchet (`tests/stretching.rs`, `quality/stretch.tsv`) keeps each
//! from getting worse, so a change to `mpfree-stretch` can't spoil them
//! unnoticed; the owner's ear decides the defaults (the listening round).
//! `fxlab stretch --out DIR` writes each sound (`<sound>.wav`), its
//! stretches (`<sound>-x0.8.wav`) and the numbers (`report.txt`), for the
//! listening zip.

use mpfree_analysis::onsets::OnsetAnalysis;
use mpfree_analysis::pitch::yin;
use mpfree_stretch::TimeMap;

use crate::listening::{self, played, preset, to_peak};
use crate::quality::{Better, Row};
use crate::synth::Engine as Synth;
use crate::wav::Stereo;

/// The song's tempo over the one the sound was made at.
pub const SPEEDS: [f64; 4] = [0.5, 0.8, 1.25, 2.0];

/// The plan's id for these numbers.
const ID: &str = "S5";

/// A hit is found when one is picked this close to where it should be.
const FOUND_MS: f64 = 30.0;

/// The onset finder's sensitivity, as the tracks' copies use it.
const SENSITIVITY: f32 = 0.6;

/// YIN's window and search range, and the aperiodicity under which a
/// window holds a steady pitch.
const YIN_WINDOW: usize = 2048;
const MIN_HZ: f32 = 50.0;
const MAX_HZ: f32 = 1500.0;
const STEADY: f32 = 0.25;
/// The sound's note is held when its pitch moves less than this, cents.
const HELD: f64 = 15.0;
/// A window this far from the sound's pitch or more is counted off.
const OFF_CENTS: f64 = 100.0;

/// The ratchet file's header.
const HEADER: &str =
    "# The stretch corpus ratchet (the audio tracks rethink, 5c.5): every number may stay or get better, never worse.\n\
# Written by `MPFREE_QUALITY_BLESS=1 cargo test --locked -p mpfree-fxlab --test stretching`;\n\
# see docs/dev/design/tempo-follow.md (\"The stretch corpus\"). key<TAB>value\n";

/// A sound of the corpus.
pub struct Sound {
    /// Its file name and the ratchet's word for it.
    pub name: &'static str,
    /// What it is, in a few words.
    pub about: &'static str,
    /// It has hits to time.
    pub hits: bool,
    /// It holds pitched notes.
    pub pitched: bool,
    pub audio: Stereo,
}

/// The listening set's sound called `name`.
fn listening_sound(set: &mut Vec<listening::Sound>, name: &str) -> Stereo {
    let i = set
        .iter()
        .position(|s| s.name == name)
        .unwrap_or_else(|| panic!("the listening set has no {name}"));
    set.remove(i).audio
}

fn mix(parts: &[(&Stereo, f64)]) -> Stereo {
    let n = parts.iter().map(|(s, _)| s.left.len()).min().unwrap_or(0);
    let rate = parts.first().map_or(44_100, |(s, _)| s.rate);
    let side = |pick: fn(&Stereo) -> &Vec<f64>| -> Vec<f64> {
        (0..n)
            .map(|i| parts.iter().map(|(s, g)| pick(s)[i] * g).sum())
            .collect()
    };
    Stereo::new(side(|s| &s.left), side(|s| &s.right), rate)
}

/// Every sound of the corpus, in order.
pub fn sounds(rate: u32) -> Vec<Sound> {
    let mut set = listening::sounds(rate);
    let drums = listening_sound(&mut set, "drum-loop");
    let piano = listening_sound(&mut set, "piano");
    let voice = listening_sound(&mut set, "vocal");
    let bass = listening_sound(&mut set, "bass");
    let pad = listening_sound(&mut set, "wide-pad");
    // A picked nylon guitar: an arpeggio, then a strummed chord.
    let nylon = Synth::Pluck.make(preset(Synth::Pluck, "Nylon"), rate);
    let picked = [
        (52, 100, 0.0, 0.5),
        (59, 90, 0.25, 0.5),
        (64, 92, 0.5, 0.5),
        (67, 95, 0.75, 0.5),
        (64, 90, 1.0, 0.5),
        (59, 88, 1.25, 0.5),
        (52, 105, 2.0, 1.8),
        (59, 100, 2.02, 1.8),
        (64, 98, 2.04, 1.8),
        (67, 96, 2.06, 1.8),
        (71, 94, 2.08, 1.8),
    ];
    let guitar = to_peak(played(nylon, &picked, rate));
    let full = to_peak(mix(&[
        (&drums, 0.5),
        (&bass, 0.4),
        (&piano, 0.35),
        (&pad, 0.25),
    ]));
    let sound = |name, about, hits, pitched, audio| Sound {
        name,
        about,
        hits,
        pitched,
        audio,
    };
    vec![
        sound(
            "drums",
            "a drum break: two bars at 100 BPM, the factory kit",
            true,
            false,
            drums,
        ),
        sound(
            "piano",
            "an electric piano (Keys, Soft Tines): a phrase, then a chord",
            true,
            true,
            piano,
        ),
        sound(
            "voice",
            "a voice (Choir, Aah Choir): one sung line",
            false,
            true,
            voice,
        ),
        sound("bass", "a bass line (Bass, Round Mono)", true, true, bass),
        sound(
            "guitar",
            "a nylon guitar (Pluck, Nylon): picked, then strummed",
            true,
            true,
            guitar,
        ),
        sound(
            "pad",
            "a wide pad: detuned tones, left and right apart",
            false,
            true,
            pad,
        ),
        sound(
            "mix",
            "a full mix: the drums, bass, piano and pad together",
            true,
            false,
            full,
        ),
    ]
}

/// `s` stretched as a region on Stretch is when the song plays at
/// `speed` times the tempo it was made at.
pub fn stretch(s: &Stereo, speed: f64) -> Stereo {
    let n = s.left.len();
    let data: Vec<f32> = (0..n)
        .flat_map(|i| [s.left[i] as f32, s.right[i] as f32])
        .collect();
    let map = TimeMap::ratio(n as u64, 1.0 / speed).expect("a sound");
    let settings = mpfree_audiotracks::copies::settings(s.rate);
    let out = mpfree_stretch::stretch(&data, 2, s.rate, map, settings).expect("stretched");
    let (l, r) = out
        .chunks_exact(2)
        .map(|f| (f64::from(f[0]), f64::from(f[1])))
        .unzip();
    Stereo::new(l, r, s.rate)
}

/// One sound's numbers at one speed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Measures {
    pub onsets: usize,
    pub found: f64,
    pub extra: f64,
    pub onset_ms: f64,
    pub rise_ms: f64,
    pub pre_echo_db: f64,
    pub stereo_change: f64,
    pub pitch_cents: f64,
    pub pitch_wobble: f64,
    pub pitch_off: f64,
    pub phasiness: f64,
    pub length_ms: f64,
}

fn mono(s: &Stereo) -> Vec<f32> {
    s.left
        .iter()
        .zip(&s.right)
        .map(|(l, r)| (0.5 * (l + r)) as f32)
        .collect()
}

fn onsets(m: &[f32], rate: u32) -> Vec<u64> {
    let mut o = OnsetAnalysis::analyse(m, 1, rate)
        .map(|a| a.pick(SENSITIVITY))
        .unwrap_or_default();
    o.sort_unstable();
    o
}

/// 10 % to 90 % of the peak within 40 ms of `at`, in ms; None if it never
/// rises.
fn rise_ms(m: &[f32], at: usize, rate: u32) -> Option<f64> {
    let ms = |x: f64| (x * f64::from(rate) / 1000.0) as usize;
    // A 1 ms envelope (the largest magnitude in each millisecond).
    let step = ms(1.0).max(1);
    let end = (at + ms(40.0)).min(m.len());
    let start = at.saturating_sub(ms(5.0));
    let env: Vec<f32> = (start..end)
        .step_by(step)
        .map(|i| {
            m[i..(i + step).min(m.len())]
                .iter()
                .fold(0.0f32, |a, x| a.max(x.abs()))
        })
        .collect();
    let peak = env.iter().copied().fold(0.0f32, f32::max);
    if peak <= 1e-6 {
        return None;
    }
    let lo = env.iter().position(|&e| e >= 0.1 * peak)?;
    let hi = env.iter().position(|&e| e >= 0.9 * peak)?;
    Some((hi.saturating_sub(lo)) as f64 * step as f64 * 1000.0 / f64::from(rate))
}

/// The 25 ms before `at` against the 20 ms after it, dB.
fn pre_echo_db(m: &[f32], at: usize, rate: u32) -> f64 {
    let n = |ms: f64| (ms * f64::from(rate) / 1000.0) as usize;
    let energy = |a: usize, b: usize| -> f64 {
        m[a.min(m.len())..b.min(m.len())]
            .iter()
            .map(|x| f64::from(*x) * f64::from(*x))
            .sum::<f64>()
            + 1e-12
    };
    let before = energy(at.saturating_sub(n(25.0)), at);
    let after = energy(at, at + n(20.0));
    10.0 * (before / after).log10()
}

fn correlation(s: &Stereo) -> f64 {
    let (mut lr, mut ll, mut rr) = (0.0, 0.0, 0.0);
    for (l, r) in s.left.iter().zip(&s.right) {
        lr += l * r;
        ll += l * l;
        rr += r * r;
    }
    if ll <= 1e-12 || rr <= 1e-12 {
        return 1.0;
    }
    lr / (ll * rr).sqrt()
}

/// The pitch and aperiodicity of the window at `at`, if it holds one.
fn pitch_at(m: &[f32], at: usize, rate: u32) -> Option<(f32, f32)> {
    let need = YIN_WINDOW + (rate as f32 / MIN_HZ) as usize + 2;
    let w = m.get(at..at + need)?;
    yin(w, rate, YIN_WINDOW, MIN_HZ, MAX_HZ, 0.15)
}

fn median(v: &mut [f64]) -> f64 {
    percentile(v, 0.5)
}

/// The value `p` of the way up `v` sorted; 0 for none.
fn percentile(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * p).round() as usize]
}

/// `out`, the sound stretched to `speed`, against `src`.
pub fn measure(snd: &Sound, speed: f64, out: &Stereo) -> Measures {
    let src = &snd.audio;
    let rate = src.rate;
    let (ms, mo) = (mono(src), mono(out));
    let mut m = Measures {
        stereo_change: (correlation(out) - correlation(src)).abs(),
        length_ms: (out.left.len() as f64 - src.left.len() as f64 / speed).abs() * 1000.0
            / f64::from(rate),
        ..Measures::default()
    };
    if snd.hits {
        let (want, got) = (onsets(&ms, rate), onsets(&mo, rate));
        let tol = FOUND_MS * f64::from(rate) / 1000.0;
        let (mut errs, mut rises, mut echoes) = (Vec::new(), Vec::new(), Vec::new());
        // Hits whose place in the stretch is past its end don't count.
        let want: Vec<u64> = want
            .into_iter()
            .filter(|&w| (w as f64 / speed) < mo.len() as f64 - tol)
            .collect();
        for &w in &want {
            let at = w as f64 / speed;
            let Some(&g) = got
                .iter()
                .min_by(|a, b| (**a as f64 - at).abs().total_cmp(&(**b as f64 - at).abs()))
            else {
                continue;
            };
            if (g as f64 - at).abs() > tol {
                continue;
            }
            errs.push((g as f64 - at).abs() * 1000.0 / f64::from(rate));
            if let (Some(a), Some(b)) = (
                rise_ms(&ms, w as usize, rate),
                rise_ms(&mo, g as usize, rate),
            ) {
                rises.push(b - a);
            }
            echoes.push(pre_echo_db(&mo, g as usize, rate) - pre_echo_db(&ms, w as usize, rate));
        }
        let mean = |v: &[f64]| {
            if v.is_empty() {
                0.0
            } else {
                v.iter().sum::<f64>() / v.len() as f64
            }
        };
        m.onsets = want.len();
        m.found = if want.is_empty() {
            1.0
        } else {
            errs.len() as f64 / want.len() as f64
        };
        m.onset_ms = median(&mut errs);
        // Hits in the stretch with none of the sound's near: an attack
        // heard twice, or a smear heard as a new hit.
        let extra = got
            .iter()
            .filter(|&&g| {
                !want
                    .iter()
                    .any(|&w| (g as f64 - w as f64 / speed).abs() <= tol)
            })
            .count();
        m.extra = if want.is_empty() {
            0.0
        } else {
            extra as f64 / want.len() as f64
        };
        m.rise_ms = mean(&rises);
        m.pre_echo_db = mean(&echoes);
    }
    if snd.pitched {
        // Every 20 ms of the stretch, against the sound at the same place
        // in the music, where the note is held over all both windows
        // hear: the sound's pitch at the window's start and at the end of
        // the music the stretch's window spans agree within HELD cents.
        let hop = (0.02 * f64::from(rate)) as usize;
        let span = (YIN_WINDOW as f64 * speed.max(1.0)) as usize;
        let (mut cents, mut aper) = (Vec::new(), Vec::new());
        let mut at = 0;
        while at < mo.len() {
            let from = (at as f64 * speed) as usize;
            if let (Some((fs, a_s)), Some((fe, _)), Some((fo, a_o))) = (
                pitch_at(&ms, from, rate),
                pitch_at(&ms, from + span, rate),
                pitch_at(&mo, at, rate),
            ) {
                let held =
                    fs > 0.0 && fe > 0.0 && (1200.0 * f64::from(fe / fs).log2()).abs() < HELD;
                if held && a_s < STEADY && a_o < STEADY + 0.25 && fo > 0.0 {
                    cents.push(1200.0 * (f64::from(fo) / f64::from(fs)).log2());
                    aper.push(f64::from(a_o - a_s));
                }
            }
            at += hop;
        }
        // A window a semitone or more away is the tracker taking another
        // partial (an octave, a fifth) or the stretch losing the note:
        // counted, not averaged in.
        let (near, off): (Vec<f64>, Vec<f64>) = cents.iter().partition(|c| c.abs() < OFF_CENTS);
        m.pitch_off = off.len() as f64 / cents.len().max(1) as f64;
        let mut abs: Vec<f64> = near.iter().map(|c| c.abs()).collect();
        m.pitch_cents = median(&mut abs);
        let mut mid = near.clone();
        let centre = median(&mut mid);
        let mut spread: Vec<f64> = near.iter().map(|c| (c - centre).abs()).collect();
        m.pitch_wobble = percentile(&mut spread, 0.9);
        m.phasiness = aper.iter().sum::<f64>() / aper.len().max(1) as f64;
    }
    m
}

/// The file and ratchet word for a speed: "x0.8".
pub fn speed_word(speed: f64) -> String {
    format!("x{speed}")
}

fn rows_of(rows: &mut Vec<Row>, snd: &Sound, speed: f64, m: &Measures) {
    let subject = format!("stretch/{}[{}]", snd.name, speed_word(speed));
    let mut row = |metric: &str, value: f64, better: Better, tol: f64| {
        rows.push(Row {
            target_id: ID,
            subject: subject.clone(),
            metric: metric.to_string(),
            value,
            better,
            target: None,
            tol,
        });
    };
    row("length_ms", m.length_ms, Better::Lower, 1.0);
    row("stereo_change", m.stereo_change, Better::Lower, 0.02);
    if snd.hits {
        row("onsets", m.onsets as f64, Better::Info, 0.0);
        row("onsets_found", m.found, Better::Higher, 0.05);
        row("extra_onsets", m.extra, Better::Lower, 0.05);
        row("onset_ms", m.onset_ms, Better::Lower, 1.0);
        row("attack_rise_ms", m.rise_ms, Better::Lower, 1.0);
        row("pre_echo_db", m.pre_echo_db, Better::Lower, 1.0);
    }
    if snd.pitched {
        row("pitch_cents", m.pitch_cents, Better::Lower, 2.0);
        row("pitch_wobble_cents", m.pitch_wobble, Better::Lower, 3.0);
        row("pitch_off", m.pitch_off, Better::Lower, 0.03);
        row("phasiness", m.phasiness, Better::Lower, 0.02);
    }
}

/// Every sound at every speed: the numbers.
pub fn report(rate: u32) -> Vec<Row> {
    let mut rows = Vec::new();
    for s in sounds(rate) {
        for &speed in &SPEEDS {
            let out = stretch(&s.audio, speed);
            rows_of(&mut rows, &s, speed, &measure(&s, speed, &out));
        }
    }
    rows
}

/// The ratchet file for `rows`.
pub fn ratchet_text(rows: &[Row]) -> String {
    let body = crate::sampler::ratchet_text(rows);
    let start = body.lines().take_while(|l| l.starts_with('#')).count();
    let mut s = String::from(HEADER);
    for l in body.lines().skip(start) {
        s.push_str(l);
        s.push('\n');
    }
    s
}

/// Writes every sound (`<sound>.wav`), its stretches (`<sound>-x0.8.wav`)
/// and the numbers with what each file is (`report.txt`) into `dir`;
/// gives the report.
pub fn write_set(dir: &std::path::Path, rate: u32) -> std::io::Result<String> {
    std::fs::create_dir_all(dir)?;
    let mut rows = Vec::new();
    let mut about = String::from(
        "The stretch corpus (the audio tracks rethink, 5c.5): each sound as made, then stretched\n\
         as a region on Stretch is when the song plays at 0.5, 0.8, 1.25 and 2 times the tempo\n\
         it was made at (x0.8 is slower and longer, x1.25 faster and shorter), at its own pitch.\n\n",
    );
    for s in sounds(rate) {
        crate::wav::write(&dir.join(format!("{}.wav", s.name)), &s.audio)?;
        about.push_str(&format!("{}.wav: {}\n", s.name, s.about));
        for &speed in &SPEEDS {
            let out = stretch(&s.audio, speed);
            crate::wav::write(
                &dir.join(format!("{}-{}.wav", s.name, speed_word(speed))),
                &out,
            )?;
            rows_of(&mut rows, &s, speed, &measure(&s, speed, &out));
        }
    }
    let text = format!("{about}\n{}", crate::sampler::to_text(&rows));
    std::fs::write(dir.join("report.txt"), &text)?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 44_100;

    /// The measures see what they should: the sound against itself moves
    /// nothing, and a copy shifted 10 ms later is 10 ms late.
    #[test]
    fn the_measures_see_a_hit_moved_and_nothing_where_nothing_moved() {
        let snd = sounds(RATE).into_iter().find(|s| s.name == "bass").unwrap();
        let same = measure(&snd, 1.0, &snd.audio.clone());
        assert_eq!(same.found, 1.0);
        assert!(same.onset_ms < 0.5, "{same:?}");
        assert!(
            same.pitch_cents < 1.0 && same.phasiness.abs() < 1e-6,
            "{same:?}"
        );
        assert!(same.stereo_change < 1e-9 && same.length_ms < 1e-9);
        let lag = (0.010 * f64::from(RATE)) as usize;
        let shift = |x: &Vec<f64>| -> Vec<f64> {
            let mut v = vec![0.0; lag];
            v.extend_from_slice(&x[..x.len() - lag]);
            v
        };
        let late = Stereo::new(shift(&snd.audio.left), shift(&snd.audio.right), RATE);
        let m = measure(&snd, 1.0, &late);
        assert!((m.onset_ms - 10.0).abs() < 3.0, "{m:?}");
    }
}
