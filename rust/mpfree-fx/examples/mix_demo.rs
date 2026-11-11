//! Renders a song through the mixer and writes `~/mpfree-work/mix-demo.wav`
//! (or the path given), plus the drum submix as the resampler would take it
//! (`mix-demo-drum-bus.wav` next to it).
//!
//! The routing, set up the way a user would on the MPC's mixer:
//! - Track 1 kick and Track 3 (a drum kit track, fed by two pad channels:
//!   snare and hats) go to **Sub 1**, the drum bus, with a glue compressor.
//! - Track 2 bass, with a compressor **keyed by the kick** (a per-insert
//!   sidechain), goes to the master.
//! - Track 4 chords send to **Return 1** (Cloud reverb) and **Return 2**
//!   (a dotted-eighth ping-pong delay); the snare pad sends to the reverb.
//! - **Sub 2** has a drive on it.
//!
//! What happens, all as sample-timed mixer events:
//! - bars 1-2: drums alone (bass and chords muted)
//! - bars 3-4: the bass comes in; its compressor is at 0 % mix
//! - bars 5-6: the compressor goes to 100 %: the bass pumps under the kick
//! - bars 7-8: the chords come in, with the reverb and delay sends
//! - bar 9: **solo** the drum bus: solo in place, so the snare's reverb
//!   stays (returns are solo-safe) and everything else goes
//! - bar 10: unsolo; on beat 3 the chords' output moves from the master to
//!   Sub 2 (the drive), crossfaded
//! - bar 11: **mute** the kick: the bass still pumps, because a muted
//!   strip still keys
//! - bar 12: the kick is back and the chords return to the master
//! - bar 13: a last hit rings out through the reverb and delay.
//!
//! It prints the level of every bar, the pumping depth (bass level just
//! after each kick against just before the next), what Master safety
//! caught and what the mixer cost per block (profiling on). The self-check
//! fails if a sample is non-finite, the converter would clip, the bass does
//! not pump while the kick is muted, or the solo bar lets the bass through.

use std::f32::consts::TAU;

use mpfree_fx::ids::{self, mixer as mx};
use mpfree_fx::mpfree_node::{Ctx, NodeEvent, Timed};
use mpfree_fx::{
    gain_to_db, CapturePoint, Cloud, Compressor, Delay, Drive, Mixer, MixerConfig, StripId,
    CLOUD_PRESETS,
};

const RATE: u32 = 48_000;
const BPM: f64 = 96.0;
const BLOCK: usize = 128;
const BARS: usize = 14;

const KICK: StripId = StripId::Track(0);
const BASS: StripId = StripId::Track(1);
const KIT: StripId = StripId::Track(2);
const CHORDS: StripId = StripId::Track(3);
const SNARE: StripId = StripId::Pad(0);
const HATS: StripId = StripId::Pad(1);
const DRUM_BUS: StripId = StripId::Bus(0);
const DRIVE_BUS: StripId = StripId::Bus(1);
const REVERB: StripId = StripId::Return(0);
const DELAY: StripId = StripId::Return(1);

fn step(s: f64) -> usize {
    (s * 60.0 / BPM * f64::from(RATE) / 4.0).round() as usize
}
fn bar(b: usize) -> usize {
    step((b as f64 - 1.0) * 16.0)
}
fn db(x: f32) -> f32 {
    gain_to_db(x)
}
fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| (v * v) as f64).sum::<f64>() / x.len().max(1) as f64).sqrt() as f32
}
fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}
fn hz(midi: i32) -> f32 {
    440.0 * 2f32.powf((midi - 69) as f32 / 12.0)
}

struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 40) as f32 / (1u64 << 23) as f32) - 1.0
    }
}

/// A saw from its harmonics below 10 kHz.
fn saw(ph: f32, hz: f32) -> f32 {
    let n = ((10_000.0 / hz) as usize).clamp(1, 32);
    (1..=n)
        .map(|k| (TAU * ph * k as f32).sin() / k as f32)
        .sum::<f32>()
        * 0.55
}

type Stem = (Vec<f32>, Vec<f32>);

/// Adds `len` frames of `f` at `at`, panned, with a 3 ms fade at the end.
fn add(s: &mut Stem, at: usize, len: usize, pan: f32, f: &mut dyn FnMut(usize) -> f32) {
    let fade = (0.003 * RATE as f32) as usize;
    for k in 0..len {
        let i = at + k;
        if i >= s.0.len() {
            break;
        }
        let x = f(k) * ((len - k) as f32 / fade as f32).min(1.0);
        s.0[i] += x * (1.0 - pan);
        s.1[i] += x * (1.0 + pan);
    }
}

/// The dry sources, one per strip that is fed.
struct Stems {
    kick: Stem,
    bass: Stem,
    snare: Stem,
    hats: Stem,
    chords: Stem,
}

const PROGRESSION: [[i32; 3]; 4] = [[57, 60, 64], [53, 57, 60], [55, 60, 64], [55, 59, 62]];
const ROOTS: [i32; 4] = [33, 29, 36, 31];

fn stems(total: usize) -> Stems {
    let z = || (vec![0.0f32; total], vec![0.0f32; total]);
    let mut s = Stems {
        kick: z(),
        bass: z(),
        snare: z(),
        hats: z(),
        chords: z(),
    };
    let mut noise = Noise(5);
    let rate = RATE as f32;
    // Bars 1 to 12 play; bar 13 is one last hit.
    for b in 1..=13usize {
        let s0 = (b as f64 - 1.0) * 16.0;
        let last = b == 13;
        let kicks: &[f64] = if last { &[0.0] } else { &[0.0, 4.0, 8.0, 12.0] };
        for &k in kicks {
            let mut ph = 0.0f32;
            add(
                &mut s.kick,
                step(s0 + k),
                (0.35 * rate) as usize,
                0.0,
                &mut |i| {
                    let t = i as f32 / rate;
                    ph += (48.0 + 120.0 * (-t * 32.0).exp()) / rate;
                    ((ph * TAU).sin() * 2.5).tanh() * (-t * 7.0).exp() * 0.55
                },
            );
        }
        let snares: &[f64] = if last { &[0.0] } else { &[4.0, 12.0] };
        for &k in snares {
            let mut ph = 0.0f32;
            add(
                &mut s.snare,
                step(s0 + k),
                (0.25 * rate) as usize,
                0.05,
                &mut |i| {
                    let t = i as f32 / rate;
                    ph += 190.0 / rate;
                    noise.next() * (-t * 16.0).exp() * 0.38
                        + (ph * TAU).sin() * (-t * 28.0).exp() * 0.28
                },
            );
        }
        if !last {
            for h in 0..8 {
                let vel = if h % 2 == 1 { 0.16 } else { 0.09 };
                let mut prev = 0.0f32;
                add(
                    &mut s.hats,
                    step(s0 + h as f64 * 2.0),
                    (0.05 * rate) as usize,
                    -0.3,
                    &mut |i| {
                        let t = i as f32 / rate;
                        let n = noise.next();
                        let hp = n - prev;
                        prev = n;
                        hp * (-t * 70.0).exp() * vel
                    },
                );
            }
        }
        let chord = (b - 1) % 4;
        // Bass: eighth notes on the root, a one-pole low pass on a saw.
        let notes = if last { 1 } else { 8 };
        for e in 0..notes {
            let f = hz(ROOTS[chord] + if e == 7 { 12 } else { 0 });
            let (mut ph, mut lp) = (0.0f32, 0.0f32);
            let len = if last {
                (1.5 * rate) as usize
            } else {
                step(1.9)
            };
            add(&mut s.bass, step(s0 + e as f64 * 2.0), len, 0.0, &mut |i| {
                ph = (ph + f / rate).fract();
                let t = i as f32 / rate;
                let x = saw(ph, f);
                lp += (x - lp) * 0.12;
                lp * (t / 0.004).min(1.0) * 0.55
            });
        }
        // Chords: stabs on the "and" of 2 and of 4 (the delay answers them).
        let stabs: &[f64] = if last { &[0.0] } else { &[6.0, 14.0] };
        for &k in stabs {
            let len = if last {
                (2.0 * rate) as usize
            } else {
                step(1.5)
            };
            for (v, &n) in PROGRESSION[chord].iter().enumerate() {
                let f = hz(n);
                let mut ph = v as f32 * 0.21;
                let pan = (v as f32 - 1.0) * 0.35;
                add(&mut s.chords, step(s0 + k), len, pan, &mut |i| {
                    ph = (ph + f / rate).fract();
                    let t = i as f32 / rate;
                    (saw(ph, f) * 0.5 + (TAU * ph).sin() * 0.5)
                        * (t / 0.005).min(1.0)
                        * (-t * 3.0).exp()
                        * 0.16
                });
            }
        }
    }
    s
}

/// A strip event at a frame of the song.
struct Cue {
    at: usize,
    strip: StripId,
    insert: Option<usize>,
    id: u16,
    value: f32,
}

fn cue(at: usize, strip: StripId, id: u16, value: f32) -> Cue {
    Cue {
        at,
        strip,
        insert: None,
        id,
        value,
    }
}
fn cue_insert(at: usize, strip: StripId, slot: usize, id: u16, value: f32) -> Cue {
    Cue {
        at,
        strip,
        insert: Some(slot),
        id,
        value,
    }
}

fn set_up(m: &mut Mixer) {
    use mx::output;
    // Drums into the drum bus: the kick directly, the pads through the kit
    // track.
    m.set_now(KICK, mx::OUTPUT, output::bus(0));
    m.set_now(KIT, mx::OUTPUT, output::bus(0));
    for pad in [SNARE, HATS] {
        m.set_now(pad, mx::OUTPUT, output::TRACK);
        m.set_now(pad, mx::OUTPUT_TRACK, 2.0);
    }
    m.set_now(HATS, mx::PAN, -25.0);
    m.set_now(SNARE, mx::send(0), -9.0);
    m.set_now(DRUM_BUS, mx::VOLUME, -1.0);
    let glue = m.chain_mut(DRUM_BUS).unwrap();
    glue.push(Box::new(Compressor::new()));
    glue.set_param(0, ids::compressor::THRESHOLD, -16.0);
    glue.set_param(0, ids::compressor::RATIO, 3.0);
    glue.set_param(0, ids::compressor::ATTACK, 15.0);
    glue.set_param(0, ids::compressor::RELEASE, 120.0);
    glue.set_param(0, ids::compressor::MAKEUP, 2.0);
    // The bass, ducked by the kick: a compressor whose detector listens to
    // Track 1.
    m.set_now(BASS, mx::VOLUME, -3.0);
    let duck = m.chain_mut(BASS).unwrap();
    duck.push(Box::new(Compressor::new()));
    duck.set_param(0, ids::compressor::SIDECHAIN, 1.0);
    duck.set_param(0, ids::compressor::THRESHOLD, -30.0);
    duck.set_param(0, ids::compressor::RATIO, 8.0);
    duck.set_param(0, ids::compressor::ATTACK, 2.0);
    duck.set_param(0, ids::compressor::RELEASE, 180.0);
    duck.set_param(0, ids::compressor::KNEE, 3.0);
    duck.set_param(0, ids::compressor::MIX, 0.0);
    assert!(m.set_key(BASS, 0, Some(KICK)));
    // Chords to the reverb and the delay.
    m.set_now(CHORDS, mx::VOLUME, -4.0);
    m.set_now(CHORDS, mx::send(0), -12.0);
    m.set_now(CHORDS, mx::send(1), -8.0);
    let verb = m.chain_mut(REVERB).unwrap();
    verb.push(Box::new(Cloud::new()));
    let hall = CLOUD_PRESETS
        .iter()
        .find(|p| p.name.contains("Hall"))
        .unwrap_or(&CLOUD_PRESETS[0]);
    for &(id, v) in hall.values {
        verb.set_param(0, id, v);
    }
    verb.set_param(0, ids::cloud::MIX, 100.0);
    let echo = m.chain_mut(DELAY).unwrap();
    echo.push(Box::new(Delay::new()));
    echo.set_param(0, ids::delay::SYNC, 7.0);
    echo.set_param(0, ids::delay::FEEDBACK, 45.0);
    echo.set_param(0, ids::delay::PING_PONG, 1.0);
    echo.set_param(0, ids::delay::TONE, 4_000.0);
    echo.set_param(0, ids::delay::MIX, 100.0);
    m.set_now(DELAY, mx::VOLUME, -4.0);
    // Sub 2: a drive the chords move to in bar 10.
    let drive = m.chain_mut(DRIVE_BUS).unwrap();
    drive.push(Box::new(Drive::new()));
    drive.set_param(0, ids::drive::DRIVE, 18.0);
    drive.set_param(0, ids::drive::AUTO_GAIN, 1.0);
    // Muted at the start.
    m.set_now(BASS, mx::MUTE, 1.0);
    m.set_now(CHORDS, mx::MUTE, 1.0);
    // What the resampler would take: the drum bus after its fader. And the
    // bass after its fader, to measure the pumping.
    assert!(m.set_capture(0, Some((DRUM_BUS, CapturePoint::AfterFader))));
    assert!(m.set_capture(1, Some((BASS, CapturePoint::AfterFader))));
}

fn cues() -> Vec<Cue> {
    use mx::output;
    let mut c = vec![
        cue(bar(3), BASS, mx::MUTE, 0.0),
        cue_insert(bar(5), BASS, 0, ids::compressor::MIX, 100.0),
        cue(bar(7), CHORDS, mx::MUTE, 0.0),
        cue(bar(9), DRUM_BUS, mx::SOLO, 1.0),
        cue(bar(10), DRUM_BUS, mx::SOLO, 0.0),
        cue(bar(10) + step(8.0), CHORDS, mx::OUTPUT, output::bus(1)),
        cue(bar(11), KICK, mx::MUTE, 1.0),
        cue(bar(12), KICK, mx::MUTE, 0.0),
        cue(bar(12), CHORDS, mx::OUTPUT, output::MASTER),
    ];
    c.sort_by_key(|c| c.at);
    c
}

/// The bass level in the 60 ms after each kick of bar `b`, against the
/// 60 ms before the next, in dB (more negative pumps harder).
fn pump_db(bass: &Stem, b: usize) -> f32 {
    let w = (0.06 * RATE as f64) as usize;
    let mut sum = 0.0;
    for k in 0..4 {
        let hit = bar(b) + step(k as f64 * 4.0) + (0.01 * RATE as f64) as usize;
        let next = bar(b) + step((k + 1) as f64 * 4.0);
        let after = rms(&bass.0[hit..hit + w]);
        let before = rms(&bass.0[next - w..next]);
        sum += db(after.max(1e-9)) - db(before.max(1e-9));
    }
    sum / 4.0
}

fn wav(left: &[f32], right: &[f32]) -> Vec<u8> {
    let mut pcm = Vec::with_capacity(left.len() * 4);
    for (&l, &r) in left.iter().zip(right) {
        for v in [l, r] {
            pcm.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32_767.0).round() as i16).to_le_bytes());
        }
    }
    let data = pcm.len() as u32;
    let mut b = Vec::with_capacity(44 + pcm.len());
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&RATE.to_le_bytes());
    b.extend_from_slice(&(RATE * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    b.extend_from_slice(&pcm);
    b
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            std::path::Path::new(&home)
                .join("mpfree-work")
                .join("mix-demo.wav")
        });
    let bus_out = out.with_file_name(format!(
        "{}-drum-bus.wav",
        out.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("mix-demo")
    ));
    let total = bar(BARS + 1);
    let st = stems(total);
    let mut m = Mixer::new(MixerConfig::default(), RATE, BLOCK);
    set_up(&mut m);
    m.set_profiling(true);
    let cues = cues();
    let mut next = 0;
    let feeds: [(StripId, &Stem); 5] = [
        (KICK, &st.kick),
        (BASS, &st.bass),
        (CHORDS, &st.chords),
        (SNARE, &st.snare),
        (HATS, &st.hats),
    ];
    let z = || (vec![0.0f32; total], vec![0.0f32; total]);
    let (mut mix, mut bus, mut bass) = (z(), z(), z());
    let mut at = 0;
    while at < total {
        let len = BLOCK.min(total - at);
        while next < cues.len() && cues[next].at < at + len {
            let c = &cues[next];
            let t = Timed::new(
                (c.at - at) as u32,
                NodeEvent::Param {
                    id: c.id,
                    value: c.value,
                },
            );
            let ok = match c.insert {
                Some(slot) => m.queue_insert(c.strip, slot, t),
                None => m.queue(c.strip, t),
            };
            assert!(ok, "cue {} refused", next);
            next += 1;
        }
        for (s, stem) in &feeds {
            let (l, r) = m.strip_input(*s).unwrap();
            l[..len].copy_from_slice(&stem.0[at..at + len]);
            r[..len].copy_from_slice(&stem.1[at..at + len]);
        }
        let beats = at as f64 / f64::from(RATE) * BPM / 60.0;
        let ctx = Ctx {
            bpm: BPM,
            beats,
            playing: true,
            ..Ctx::new(RATE, at as u64)
        };
        m.process(&ctx, &mut mix.0[at..at + len], &mut mix.1[at..at + len]);
        for (slot, dst) in [(0, &mut bus), (1, &mut bass)] {
            let (l, r) = m.capture(slot).unwrap();
            dst.0[at..at + len].copy_from_slice(&l[..len]);
            dst.1[at..at + len].copy_from_slice(&r[..len]);
        }
        at += len;
    }

    println!(
        "Mixer demo: {BARS} bars at {BPM} BPM, {RATE} Hz, {BLOCK}-frame blocks, latency {} frames",
        m.latency()
    );
    let what = [
        "drums alone",
        "drums alone",
        "bass in, compressor at 0 %",
        "bass in, compressor at 0 %",
        "bass pumps under the kick",
        "bass pumps under the kick",
        "chords in, reverb and delay sends",
        "chords in, reverb and delay sends",
        "drum bus soloed (reverb stays)",
        "chords move to Sub 2 (drive) on beat 3",
        "kick muted, bass still pumps",
        "kick back, chords to master",
        "last hit",
        "tail",
    ];
    for b in 1..=BARS {
        let (a, e) = (bar(b), bar(b + 1).min(total));
        let level = rms(&mix.0[a..e]).max(rms(&mix.1[a..e]));
        let pk = peak(&mix.0[a..e]).max(peak(&mix.1[a..e]));
        println!(
            "  bar {b:>2}: rms {:>6.1} dB, peak {:>6.1} dB  {}",
            db(level),
            db(pk),
            what[b - 1]
        );
    }
    let pumps: Vec<(usize, f32)> = [3usize, 5, 11]
        .iter()
        .map(|&b| (b, pump_db(&bass, b)))
        .collect();
    for (b, p) in &pumps {
        println!("  bass after each kick vs before the next, bar {b}: {p:+.1} dB");
    }
    let solo_bass = rms(&bass.0[bar(9) + step(1.0)..bar(10)]);
    println!(
        "  bass during the drum bus solo: {:.1} dB",
        db(solo_bass.max(1e-9))
    );
    let s = m.safety().stats();
    println!("  Master safety: {} frames over the ceiling caught, peak out {:.1} dB, most reduction {:.1} dB", s.overs, s.peak_out_db, s.max_reduction_db);
    let whole = m.probe().lifetime();
    println!(
        "  mixer cost on this machine: mean {:.1} us, p99 {:.1} us per block ({:.2} % of a period)",
        whole.mean_ns / 1e3,
        whole.p99_ns as f64 / 1e3,
        100.0 * whole.mean_ns / (BLOCK as f64 * 1e9 / f64::from(RATE))
    );
    let mut rows: Vec<(StripId, f64)> = m
        .strips()
        .map(|s| (s, m.strip_probe(s).unwrap().lifetime().mean_ns / 1e3))
        .collect();
    rows.sort_by(|a, b| b.1.total_cmp(&a.1));
    let top: Vec<String> = rows
        .iter()
        .take(5)
        .map(|(s, us)| format!("{s:?} {us:.1} us"))
        .collect();
    println!("  slowest strips (with their inserts): {}", top.join(", "));

    let finite = mix.0.iter().chain(&mix.1).all(|v| v.is_finite());
    let clip = peak(&mix.0).max(peak(&mix.1)) > 1.0;
    let ghost = pumps[2].1 < -3.0;
    let soloed = solo_bass < 1e-4;
    println!("Self-check: finite {finite}, under full scale {}, bass pumps with the kick muted {ghost}, solo silences the bass {soloed}", !clip);

    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).expect("output folder");
    }
    std::fs::write(&out, wav(&mix.0, &mix.1)).expect("write the mix");
    std::fs::write(&bus_out, wav(&bus.0, &bus.1)).expect("write the drum bus");
    println!("Wrote {} and {}", out.display(), bus_out.display());
    if !(finite && !clip && ghost && soloed) {
        std::process::exit(1);
    }
}
