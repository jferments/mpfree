//! Every read that plays audio at another speed, measured one way (DSP
//! quality step 1.6, `docs/dev/design/pitched-reads.md`): the sampler's
//! pad reads, the grain reads, the streaming cursor's reads and the
//! stretch's resampler. Each keeps its own code, tuned to its own memory
//! access; measured the same way, any difference between them is chosen,
//! not accidental.
//!
//! - **Q1, images:** a sine whose output lands on one FFT bin (as
//!   [`harmonics`] does), at 3, 9 and 13.5 kHz out, played an octave
//!   down to an octave and a fifth up: the loudest other bin from 20 Hz to
//!   20 kHz against the tone (the sampler's S1 and Y5 measure, for every
//!   reader).
//! - **Q2, passband:** the same tone's level against the tone at pitch.
//! - **Q3, folds:** an 18 kHz sine a fifth up and a 15 kHz sine an octave
//!   up (27 and 30 kHz, above what 44.1 kHz carries, so they would fold
//!   back to 17.1 and 14.1 kHz): what is heard of them, RMS, against the
//!   tone.
//! - **Q4, loop wraps:** a loop of 100.6 periods of 1 kHz (its seam a
//!   step), the sound ending where the loop does, read a semitone up, two
//!   semitones down and a fifth up, against the loop unrolled (the sound
//!   to the loop's end, then the loop over and over) read straight
//!   through: the largest difference, dB against the tone. A read that
//!   takes its neighbours at the seam from round the loop matches; one
//!   that takes the recording past the end, or before the start once
//!   round, does not. For the readers that loop (the pads and the
//!   streaming cursor); grains and the stretch read no loops.
//!
//! A row whose sine would have to be above 21 kHz in the sound is left
//! out (it can't be in a 44.1 kHz file), and so is a speed a reader can't
//! play (the streaming cursor's fastest is four times).
//!
//! The standard's targets are set per row ([`target`]): the sound
//! thread's reads must meet them on the rows the 8-point sampler read
//! meets, and the rows it doesn't (octave drops, 13.5 kHz a fifth down)
//! are tracked for step 3.3. The stretch's resampler runs off the sound
//! thread, so it is held to more. One read is an exception by choice,
//! measured with no target: the Vintage pad (aliasing is its sound).
//! Every number is kept by a ratchet (`tests/readers.rs`,
//! `quality/readers.tsv`).

use mpfree_grain::buffer::Clip;
use mpfree_grain::GrainSource;
use mpfree_sampler::bandlimit::Octaves;
use mpfree_sampler::grain_clip::GrainClip;
use mpfree_sampler::model::{Loop, LoopMode, PlayMode};
use mpfree_stream::coord::Plan;
use mpfree_stream::cursor::{Cursor, Read};
use mpfree_stream::player::Config;
use mpfree_stream::testkit::{float_wav, folder, Rig};

use crate::fft::{bin_db, power_spectrum};
use crate::harmonics;
use crate::quality::{Better, Row};
use crate::sampler;
use crate::signals::{coherent, db_to_amp, sine};

/// The sines' level, dBFS peak.
const LEVEL_DB: f64 = -6.0;

/// Output frames left to settle before the analysis.
const SETTLE: usize = 4096;

/// The ratchet file's header.
const HEADER: &str =
    "# The pitched reads' quality ratchet: every number may stay or get better, never worse.\n\
# Written by `MPFREE_QUALITY_BLESS=1 cargo test --locked -p mpfree-fxlab --test readers`;\n\
# see docs/dev/design/pitched-reads.md. key<TAB>value\n";

/// The speeds measured, in cents, and how the report names them.
pub const STEPS: [(i32, &str); 8] = [
    (-1200, "an octave down"),
    (-700, "a fifth down"),
    (-200, "two semitones down"),
    (-100, "a semitone down"),
    (100, "a semitone up"),
    (200, "two semitones up"),
    (700, "a fifth up"),
    (1900, "an octave and a fifth up"),
];

/// Where the tone comes out, Hz.
pub const OUTS: [f64; 3] = [3_000.0, 9_000.0, 13_500.0];

/// A read that plays audio at another speed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reader {
    /// A sampler pad: the plain copy at pitch, the 8-point read up to two
    /// semitones up, the band-limited read above, from the octave copies
    /// past an octave.
    Pad,
    /// A Vintage pad: Hermite at every speed, no low-pass (by choice).
    PadVintage,
    /// A sampler pad's grains ([`GrainClip`] with its clean read):
    /// band-limited above two semitones up, Hermite at and under.
    PadGrains,
    /// The grain engine's own read ([`Clip`], and the live buffers the
    /// Grains effect reads): Hermite at every speed.
    Grains,
    /// The streaming cursor's band-limited read (8 crossings a side, the
    /// default).
    Stream8,
    /// The same with 16 crossings a side.
    Stream16,
    /// The streaming cursor's cheap read: the pads' 8-point read, its cut
    /// following the speed past the file's own.
    StreamNear,
    /// The stretch's windowed-sinc resampler (`mpfree_stretch::
    /// convert_rate`), off the sound thread: its pitch shift and the
    /// conversion of sounds loaded at another rate.
    Stretch,
}

impl Reader {
    pub const ALL: [Reader; 8] = [
        Reader::Pad,
        Reader::PadVintage,
        Reader::PadGrains,
        Reader::Grains,
        Reader::Stream8,
        Reader::Stream16,
        Reader::StreamNear,
        Reader::Stretch,
    ];

    /// How the report names it.
    pub fn name(self) -> &'static str {
        match self {
            Reader::Pad => "pad",
            Reader::PadVintage => "pad vintage",
            Reader::PadGrains => "pad grains",
            Reader::Grains => "grains",
            Reader::Stream8 => "stream sinc8",
            Reader::Stream16 => "stream sinc16",
            Reader::StreamNear => "stream 8-point",
            Reader::Stretch => "stretch",
        }
    }

    /// Whether it runs on the sound thread.
    pub fn live(self) -> bool {
        self != Reader::Stretch
    }

    /// Whether it is an exception by choice, measured with no target.
    pub fn chosen(self) -> bool {
        self == Reader::PadVintage
    }

    /// The speed it plays at, `cents` up, as it rounds it: file frames an
    /// output frame. None for a speed it can't play.
    fn speed(self, cents: i32, rate: u32) -> Option<f64> {
        let exact = (f64::from(cents) / 1200.0).exp2();
        match self {
            Reader::Pad | Reader::PadVintage => Some(sampler::speed(cents, rate, rate)),
            Reader::Stream8 | Reader::Stream16 | Reader::StreamNear => {
                (exact <= mpfree_stream::cursor::MAX_SPEED).then_some(exact)
            }
            // convert_rate takes whole rates: the sound as if recorded at
            // `from`, played at `rate`.
            Reader::Stretch => Some((f64::from(rate) * exact).round() / f64::from(rate)),
            Reader::PadGrains | Reader::Grains => Some(exact),
        }
    }

    /// Whether it plays loops (Q4).
    pub fn loops(self) -> bool {
        matches!(
            self,
            Reader::Pad
                | Reader::PadVintage
                | Reader::Stream8
                | Reader::Stream16
                | Reader::StreamNear
        )
    }

    /// `frames` output frames of mono `x` (a 44.1 kHz sound at `rate`)
    /// read at `cents` up, from its start; the left channel.
    fn play(self, x: &[f64], cents: i32, rate: u32, frames: usize) -> Vec<f64> {
        self.play_looped(x, cents, rate, frames, None)
    }

    /// [`Reader::play`], round the loop `start..end` (frames) when given,
    /// for a reader that [`Reader::loops`].
    fn play_looped(
        self,
        x: &[f64],
        cents: i32,
        rate: u32,
        frames: usize,
        looped: Option<(u64, u64)>,
    ) -> Vec<f64> {
        let v = self.speed(cents, rate).expect("a speed it plays");
        assert!(looped.is_none() || self.loops(), "{self:?} plays no loops");
        match self {
            Reader::Pad | Reader::PadVintage => {
                let s = sampler::sound(x, rate);
                let vintage = self == Reader::PadVintage;
                let p = sampler::pad(&s, |p| {
                    p.tune_cents = cents;
                    p.vintage = vintage;
                    if let Some((start, end)) = looped {
                        let r = &mut p.layers[0].region;
                        r.mode = PlayMode::Loop;
                        r.sustain_loop = Some(Loop {
                            start,
                            end,
                            crossfade: 0,
                            mode: LoopMode::Forward,
                        });
                    }
                });
                let kit = sampler::kit(vec![(s, p)]);
                sampler::play(&kit, rate, &[(0, sampler::hit(0))], frames).left
            }
            Reader::PadGrains | Reader::Grains => {
                let data: Vec<f32> = x.iter().map(|&s| s as f32).collect();
                let mut out = vec![[0.0f32; 2]; frames];
                // From frame 8, so the read's neighbours are all inside.
                if self == Reader::Grains {
                    let clip = Clip::new(&data, 1, rate).expect("a lab clip");
                    clip.read_run(8.0, v, &mut out);
                } else {
                    let octaves = Octaves::make(&data, 1);
                    let clip = GrainClip::new(&data, 1, rate)
                        .expect("a lab clip")
                        .clean(Some(&octaves));
                    let aim = clip.aim(v);
                    if aim.clean {
                        clip.read_aimed_run(8.0, v, aim, &mut out);
                    } else {
                        clip.read_run(8.0, v, &mut out);
                    }
                }
                out.iter().map(|f| f64::from(f[0])).collect()
            }
            Reader::Stream8 | Reader::Stream16 | Reader::StreamNear => {
                let read = match self {
                    Reader::Stream8 => Read::Sinc8,
                    Reader::Stream16 => Read::Sinc16,
                    _ => Read::Near,
                };
                stream_play(x, read, v, rate, frames, looped)
            }
            Reader::Stretch => {
                let data: Vec<f32> = x.iter().map(|&s| s as f32).collect();
                let from = (f64::from(rate) * v).round() as u32;
                let out = mpfree_stretch::convert_rate(&data, 1, from, rate);
                let mut y: Vec<f64> = out.iter().map(|&s| f64::from(s)).collect();
                y.resize(frames, 0.0);
                y.truncate(frames);
                y
            }
        }
    }
}

/// `frames` output frames of mono `x` read by a streaming cursor, `read`
/// at `speed`, from frame 8 (left channel). The sound goes through a file,
/// as a stem's does.
fn stream_play(
    x: &[f64],
    read: Read,
    speed: f64,
    rate: u32,
    frames: usize,
    looped: Option<(u64, u64)>,
) -> Vec<f64> {
    let dir = folder("fxlab-readers");
    let path = dir.join("tone.wav");
    let data: Vec<f32> = x.iter().map(|&s| s as f32).collect();
    float_wav(&path, rate, 1, &data);
    let mut rig = Rig::new(
        Config {
            slots: 256,
            ..Config::default()
        },
        Plan::default(),
    );
    rig.c.new_project();
    rig.c.open(0, &path).expect("the lab's file opens");
    let mut k = Cursor::new(0, 0);
    k.set_read(read);
    k.set_speed(speed);
    k.seek(8);
    k.set_loop(looped);
    rig.warm(&mut [&mut k], 400);
    let mut got = Vec::with_capacity(frames + 256);
    let mut missed = 0;
    while got.len() < frames {
        let mut out = vec![vec![0.0f32; 512]];
        missed += rig.period(&mut [&mut k], &mut out);
        got.extend(out[0].iter().step_by(2).map(|&s| f64::from(s)));
    }
    assert_eq!(missed, 0, "the lab's stream played every frame");
    got.truncate(frames);
    let _ = std::fs::remove_dir_all(&dir);
    got
}

/// What a reader made of a pitched sine.
struct Pitched {
    level_db: f64,
    spur_db: f64,
    spur_hz: f64,
}

/// A sine that comes out on the bin nearest `out_hz`, read `cents` up.
fn pitched_sine(r: Reader, cents: i32, out_hz: f64, rate: u32) -> Pitched {
    let n = harmonics::N;
    let frames = SETTLE + n;
    let v = r.speed(cents, rate).expect("a speed it plays");
    let (bin, f_out) = coherent(out_hz, n, rate);
    let x = sine(
        f_out / v,
        db_to_amp(LEVEL_DB),
        (frames as f64 * v) as usize + 256,
        rate,
    );
    let out = r.play(&x, cents, rate, frames);
    let ps = power_spectrum(&out[SETTLE..SETTLE + n]);
    let hz = |k: usize| k as f64 * f64::from(rate) / n as f64;
    let fund = ps[bin].max(1e-300);
    let (spur_k, spur) = (1..=n / 2)
        .filter(|&k| k != bin && hz(k) >= 20.0 && hz(k) <= 20_000.0)
        .map(|k| (k, ps[k]))
        .fold((0, 0.0), |a, b| if b.1 > a.1 { b } else { a });
    Pitched {
        level_db: bin_db(fund),
        spur_db: floor(10.0 * (spur / fund).max(1e-30).log10()),
        spur_hz: hz(spur_k),
    }
}

/// What is heard of a sine at `hz` read `cents` up, against the sine
/// itself: dB RMS.
fn heard_of(r: Reader, hz: f64, cents: i32, rate: u32) -> f64 {
    let frames = SETTLE + harmonics::N;
    let v = r.speed(cents, rate).expect("a speed it plays");
    let amp = db_to_amp(LEVEL_DB);
    let x = sine(hz, amp, (frames as f64 * v) as usize + 256, rate);
    let out = r.play(&x, cents, rate, frames);
    let seg = &out[SETTLE..];
    let rms = (seg.iter().map(|x| x * x).sum::<f64>() / seg.len() as f64).sqrt();
    floor(20.0 * (rms / (amp / 2f64.sqrt())).max(1e-30).log10())
}

/// The loop Q4 reads: 100.6 periods of 1 kHz at 44.1 kHz, so its seam is
/// a step, and a read that takes its neighbours from the wrong side of it
/// (the recording past the end, or before the start once round) shows.
const LOOP: (u64, u64) = (10_000, 14_437);

/// Q4: the largest difference between reading round [`LOOP`] (the sound
/// ends where the loop does) and reading the loop unrolled (the sound up
/// to the loop's end, then the loop over and over) straight through,
/// `cents` up: dB against the tone's peak.
fn loop_wrap_db(r: Reader, cents: i32, rate: u32) -> f64 {
    let frames = 24_000;
    let v = r.speed(cents, rate).expect("a speed it plays");
    let amp = db_to_amp(LEVEL_DB);
    let (ls, le) = (LOOP.0 as usize, LOOP.1 as usize);
    let x = sine(1_000.0, amp, le, rate);
    let n = (frames as f64 * v) as usize + 256;
    let unrolled: Vec<f64> = (0..n.max(le))
        .map(|k| {
            if k < le {
                x[k]
            } else {
                x[ls + (k - le) % (le - ls)]
            }
        })
        .collect();
    let looped = r.play_looped(&x, cents, rate, frames, Some(LOOP));
    let unrolled = r.play(&unrolled, cents, rate, frames);
    let d = looped
        .iter()
        .zip(&unrolled)
        .fold(0.0f64, |m, (a, b)| m.max((a - b).abs()));
    floor(20.0 * (d / amp).max(1e-30).log10())
}

/// Whether a Q1 or Q2 row is in the standard's required set: every row
/// but the octave drops and 13.5 kHz a fifth down (step 3.3's).
pub fn required(cents: i32, out_hz: f64) -> bool {
    cents > -1200 && !(out_hz > 10_000.0 && cents <= -700)
}

/// The standard's target for reader `r`'s `metric` on a row, if any.
pub fn target(r: Reader, metric: &str, cents: i32, out_hz: f64) -> Option<f64> {
    if r.chosen() {
        return None;
    }
    let live = r.live();
    match metric {
        "spur_worst_db" if !live => Some(-70.0),
        "spur_worst_db" => required(cents, out_hz).then_some(-50.0),
        // Off the sound thread: flat wherever the tone in the sound is a
        // tone anyone hears (at or under 20 kHz).
        "level_db" if !live => {
            (out_hz / (f64::from(cents) / 1200.0).exp2() <= 20_000.0).then_some(0.5)
        }
        // The passband to 9 kHz out.
        "level_db" => (required(cents, out_hz) && out_hz < 10_000.0).then_some(1.0),
        "heard_db" if !live => Some(-70.0),
        "heard_db" => Some(-50.0),
        "wrap_db" => Some(-100.0),
        _ => None,
    }
}

/// Every number, in the order measured.
pub fn report(rate: u32) -> Vec<Row> {
    let mut rows = Vec::new();
    for r in Reader::ALL {
        for out_hz in OUTS {
            let at_pitch = pitched_sine(r, 0, out_hz, rate);
            for (cents, how) in STEPS {
                if out_hz / (f64::from(cents) / 1200.0).exp2() > 21_000.0
                    || r.speed(cents, rate).is_none()
                {
                    continue;
                }
                let p = pitched_sine(r, cents, out_hz, rate);
                let subject = format!("{}[{how}, {} kHz out]", r.name(), out_hz / 1000.0);
                put(
                    &mut rows,
                    "Q1",
                    subject.clone(),
                    "spur_worst_db",
                    p.spur_db,
                    Better::Lower,
                    target(r, "spur_worst_db", cents, out_hz),
                    0.5,
                );
                put(
                    &mut rows,
                    "Q1",
                    subject.clone(),
                    "spur_hz",
                    p.spur_hz.round(),
                    Better::Info,
                    None,
                    0.0,
                );
                put(
                    &mut rows,
                    "Q2",
                    subject,
                    "level_db",
                    p.level_db - at_pitch.level_db,
                    Better::Near(0.0),
                    target(r, "level_db", cents, out_hz),
                    0.05,
                );
            }
        }
        for (hz, cents, how) in [
            (18_000.0, 700, "18 kHz a fifth up"),
            (15_000.0, 1200, "15 kHz an octave up"),
        ] {
            if r.speed(cents, rate).is_none() {
                continue;
            }
            put(
                &mut rows,
                "Q3",
                format!("{}[{how}]", r.name()),
                "heard_db",
                heard_of(r, hz, cents, rate),
                Better::Lower,
                target(r, "heard_db", cents, hz),
                0.5,
            );
        }
        if r.loops() {
            for (cents, how) in [
                (100, "a semitone up"),
                (-200, "two semitones down"),
                (700, "a fifth up"),
            ] {
                put(
                    &mut rows,
                    "Q4",
                    format!("{}[loop, {how}]", r.name()),
                    "wrap_db",
                    loop_wrap_db(r, cents, rate),
                    Better::Lower,
                    target(r, "wrap_db", cents, 1_000.0),
                    1.0,
                );
            }
        }
    }
    rows
}

/// The rows as text, one a line.
pub fn to_text(rows: &[Row]) -> String {
    sampler::to_text(rows)
}

/// The ratchet file for `rows`.
pub fn ratchet_text(rows: &[Row]) -> String {
    sampler::ratchet_text(rows).replacen(sampler::HEADER, HEADER, 1)
}

/// What fails the ratchet: a row worse than recorded, a row not recorded
/// yet, a recorded row no longer measured.
pub fn ratchet_check(rows: &[Row], recorded: &[(String, f64)]) -> Vec<String> {
    sampler::ratchet_check(rows, recorded)
}

/// Levels far below hearing read as −140 dB.
fn floor(db: f64) -> f64 {
    db.max(-140.0)
}

#[allow(clippy::too_many_arguments)]
fn put(
    rows: &mut Vec<Row>,
    id: &'static str,
    subject: String,
    metric: &str,
    value: f64,
    better: Better,
    target: Option<f64>,
    tol: f64,
) {
    rows.push(Row {
        target_id: id,
        subject: format!("reads/{subject}"),
        metric: metric.to_string(),
        value,
        better,
        target,
        tol,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reader_plays_a_sine_at_its_pitch() {
        // 1 kHz a fifth up comes out at 1.5 kHz on every reader: the
        // loudest bin is the tone's.
        for r in Reader::ALL {
            let p = pitched_sine(r, 700, 3_000.0, 44_100);
            assert!(p.spur_db < -20.0, "{r:?}: {} dB", p.spur_db);
        }
    }

    #[test]
    fn the_required_set_leaves_out_step_3_3s_rows() {
        assert!(!required(-1200, 3_000.0));
        assert!(!required(-700, 13_500.0));
        assert!(required(-700, 9_000.0));
        assert!(required(200, 13_500.0));
        assert_eq!(
            target(Reader::PadVintage, "spur_worst_db", 100, 3_000.0),
            None
        );
        assert_eq!(
            target(Reader::Stretch, "level_db", -1200, 9_000.0),
            Some(0.5)
        );
        // 13.5 kHz a fifth down is 20.2 kHz in the sound.
        assert_eq!(target(Reader::Stretch, "level_db", -700, 13_500.0), None);
        assert_eq!(target(Reader::Pad, "level_db", 200, 13_500.0), None);
    }
}
