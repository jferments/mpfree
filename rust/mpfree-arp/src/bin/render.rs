//! WORKSTATION ONLY: render arpeggiator and latch-pattern output to WAV files,
//! so patterns can be heard without the MPC.
//!
//! It drives the same `mpfree-arp` engine the MPC uses, in 128-frame periods
//! at 44.1 kHz, and plays its notes on a small built-in synth (or built-in
//! drum sounds for pad slots). It never touches any device; it only writes
//! WAV files and a report.txt into one new folder.
//!
//! ```text
//! mpfree-arp-render --demo                       every order, every starter pattern, drums, lanes, scales
//! mpfree-arp-render --chords "60,63,67,70;65,68,72,75" --mode pattern --builtin Bounce
//! mpfree-arp-render --pads 1,3,5,7 --mode pattern --pattern "x x:2 . x | vel 127 60 90"
//! mpfree-arp-render --patterns my-patterns.txt   every pattern in a library file (see patterns/example.txt)
//! ```
//!
//! Options: `--out DIR` (default ~/mpfree-work/arp-render-<UTC>), `--mode
//! repeat|arp|pattern`, `--order NAME`, `--pattern TEXT` or `--builtin NAME`,
//! `--chords "n,n;n,n"` (one bar each, looped) or `--notes n,n` or `--pads
//! 1,2` (drum sounds), `--bpm`, `--rate 1/16`, `--octaves`, `--gate`,
//! `--swing`, `--strum`, `--scale d_dorian`, `--ramp TARGET:STEPS`, `--retrigger`,
//! `--bars`, `--seed`, `--name`.
//!
//! The report gives two hashes per file. `events` covers the notes the
//! engine played (frame, pitch, velocity) and is the same on every computer.
//! `audio` covers the WAV bytes; it repeats on one computer, but the synth
//! uses the system's maths library (exp, tanh, powf), so it can differ
//! between computers.
#![forbid(unsafe_code)]

use mpfree_arp::scale::SCALES;
use mpfree_arp::{Engine, EventKind, Events, LatchMode, Library, Mode, Order, Pattern, Ramp, Rate, Scale, Settings, BUILT_IN};
use std::f32::consts::TAU;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const RATE: u32 = 44_100;
const PERIOD: u32 = 128;
const VOICES: usize = 24;

// ---------------------------------------------------------------- sounds

#[derive(Clone, Copy, PartialEq)]
enum Sound {
    /// A MIDI note on the synth.
    Synth(u8),
    /// A pad slot (0..=15) on the drum sounds.
    Drum(u8),
}

#[derive(Clone, Copy)]
struct Voice {
    sound: Sound,
    key: u8,
    velocity: f32,
    age: u32,
    phase: f32,
    phase2: f32,
    released_at: Option<u32>,
    lp: f32,
    noise: u32,
    alive: bool,
}

struct Synth {
    voices: [Voice; VOICES],
    drums: bool,
}

impl Synth {
    fn new(drums: bool) -> Self {
        let v = Voice { sound: Sound::Synth(0), key: 0, velocity: 0.0, age: 0, phase: 0.0, phase2: 0.0,
                        released_at: None, lp: 0.0, noise: 1, alive: false };
        Self { voices: [v; VOICES], drums }
    }

    fn on(&mut self, key: u8, velocity: u8) {
        let sound = if self.drums { Sound::Drum(key % 16) } else { Sound::Synth(key) };
        let slot = self.voices.iter().position(|v| !v.alive).unwrap_or_else(|| {
            // Steal the oldest.
            let mut best = 0;
            for (i, v) in self.voices.iter().enumerate() {
                if v.age > self.voices[best].age {
                    best = i;
                }
            }
            best
        });
        self.voices[slot] = Voice { sound, key, velocity: f32::from(velocity) / 127.0, age: 0, phase: 0.0, phase2: 0.0,
                                    released_at: None, lp: 0.0, noise: 0x1234_5678 ^ (u32::from(key) * 7919), alive: true };
    }

    fn off(&mut self, key: u8) {
        for v in self.voices.iter_mut().filter(|v| v.alive && v.key == key && v.released_at.is_none()) {
            v.released_at = Some(v.age);
        }
    }

    fn render(&mut self, out: &mut [f32]) {
        for s in out.iter_mut() {
            let mut mix = 0.0;
            for v in self.voices.iter_mut().filter(|v| v.alive) {
                mix += match v.sound {
                    Sound::Synth(n) => synth_sample(v, n),
                    Sound::Drum(p) => drum_sample(v, p),
                };
                v.age += 1;
            }
            *s = (mix * 0.35).tanh();
        }
    }
}

fn noise(v: &mut Voice) -> f32 {
    v.noise ^= v.noise << 13;
    v.noise ^= v.noise >> 17;
    v.noise ^= v.noise << 5;
    (v.noise as f32 / u32::MAX as f32) * 2.0 - 1.0
}

/// Two slightly detuned saws through a one-pole low-pass, with an ADSR.
fn synth_sample(v: &mut Voice, note: u8) -> f32 {
    let t = v.age as f32 / RATE as f32;
    let f = 440.0 * 2f32.powf((f32::from(note) - 69.0) / 12.0);
    v.phase = (v.phase + f / RATE as f32).fract();
    v.phase2 = (v.phase2 + f * 1.004 / RATE as f32).fract();
    let raw = (v.phase * 2.0 - 1.0) + (v.phase2 * 2.0 - 1.0);
    let cutoff = (0.05 + 0.25 * v.velocity * (-t * 6.0).exp()).min(0.9);
    v.lp += cutoff * (raw - v.lp);
    let attack = (t / 0.003).min(1.0);
    let decay = 0.45 + 0.55 * (-t / 0.15).exp();
    let release = match v.released_at {
        Some(at) => {
            let r = (v.age - at) as f32 / RATE as f32;
            if r > 0.4 {
                v.alive = false;
            }
            (-r / 0.08).exp()
        }
        None => 1.0,
    };
    v.lp * attack * decay * release * v.velocity * 0.5
}

/// Sixteen simple drum sounds for pad slots 0..=15 (kick, snare, hats...).
fn drum_sample(v: &mut Voice, pad: u8) -> f32 {
    let t = v.age as f32 / RATE as f32;
    if t > 1.2 {
        v.alive = false;
        return 0.0;
    }
    let vel = v.velocity;
    let n = noise(v);
    let tone = |f0: f32, f1: f32, sweep: f32, decay: f32, phase: &mut f32| {
        let f = f1 + (f0 - f1) * (-t / sweep).exp();
        *phase = (*phase + f / RATE as f32).fract();
        (*phase * TAU).sin() * (-t / decay).exp()
    };
    let s = match pad {
        0 => tone(160.0, 48.0, 0.04, 0.35, &mut v.phase),                 // kick
        1 => 0.5 * tone(220.0, 180.0, 0.02, 0.12, &mut v.phase) + 0.6 * n * (-t / 0.15).exp(), // snare
        2 => 0.4 * n * (-t / 0.03).exp(),                                 // closed hat
        3 => 0.35 * n * (-t / 0.25).exp(),                                // open hat
        4 => 0.6 * n * ((-t / 0.012).exp() + 0.6 * (-(t - 0.02).max(0.0) / 0.12).exp()), // clap
        5 => tone(900.0, 800.0, 0.01, 0.03, &mut v.phase),                // rim
        6 => tone(140.0, 90.0, 0.08, 0.3, &mut v.phase),                  // low tom
        7 => tone(220.0, 150.0, 0.08, 0.25, &mut v.phase),                // high tom
        8 => 0.3 * n * (-t / 0.8).exp(),                                  // crash
        9 => 0.2 * n * (-t / 0.5).exp() + 0.1 * tone(3200.0, 3200.0, 1.0, 0.5, &mut v.phase), // ride
        10 => 0.3 * n * (-t / 0.06).exp(),                                // shaker
        11 => tone(600.0, 400.0, 0.02, 0.1, &mut v.phase),                // perc
        12 => tone(1200.0, 900.0, 0.01, 0.08, &mut v.phase),              // perc 2
        13 => tone(330.0, 300.0, 0.1, 0.4, &mut v.phase),                 // tonal hit
        14 => tone(90.0, 40.0, 0.15, 0.6, &mut v.phase),                  // sub
        _ => tone(500.0, 500.0, 1.0, 0.15, &mut v.phase) * (t * 30.0 * TAU).sin(), // cowbell-ish
    };
    s * vel
}

// ---------------------------------------------------------------- jobs

#[derive(Clone)]
struct Job {
    name: String,
    about: String,
    settings: Settings,
    pattern: Pattern,
    /// Chords, one per bar, looped; each note list is held for the bar.
    chords: Vec<Vec<u8>>,
    drums: bool,
    bpm: f64,
    bars: u32,
    seed: u32,
}

struct Rendered {
    samples: Vec<f32>,
    /// Every event as (absolute frame, kind) bytes, for a hash that does not
    /// depend on floating-point maths.
    events: Vec<u8>,
    notes: usize,
    peak: f32,
    dropped: u64,
}

fn render(job: &Job) -> Rendered {
    let mut e = Engine::new(RATE, job.bpm);
    e.set_seed(job.seed);
    e.set_settings(job.settings);
    e.set_pattern(job.pattern);
    e.set_latch(LatchMode::Replace);
    let mut synth = Synth::new(job.drums);
    let bar_frames = (RATE as f64 * 60.0 / job.bpm * 4.0).round() as u64;
    let total = bar_frames * u64::from(job.bars) + u64::from(RATE); // plus a second of tail
    let mut samples = Vec::with_capacity(total as usize);
    let mut events = Events::new();
    let mut buf = [0f32; PERIOD as usize];
    let mut frame = 0u64;
    let mut bar = u64::MAX;
    let mut notes = 0;
    let mut log = Vec::new();
    while frame < total {
        let this_bar = frame / bar_frames;
        if this_bar != bar && this_bar < u64::from(job.bars) {
            bar = this_bar;
            let chord = &job.chords[(bar % job.chords.len() as u64) as usize];
            // A fresh press with nothing held replaces the latched chord.
            for &n in chord {
                e.press(n, 100);
            }
            for &n in chord {
                e.release(n);
            }
        }
        if this_bar >= u64::from(job.bars) && !e.active().is_empty() {
            e.clear_latched();
        }
        e.process(PERIOD, &mut events);
        // Split the period at each event's frame, so notes start on time.
        let mut pos = 0usize;
        for ev in events.as_slice() {
            let at = (ev.offset as usize).min(PERIOD as usize);
            synth.render(&mut buf[pos..at]);
            pos = at;
            log.extend_from_slice(&(frame + u64::from(ev.offset)).to_le_bytes());
            match ev.kind {
                EventKind::On { note, velocity } => {
                    synth.on(note, velocity);
                    notes += 1;
                    log.extend_from_slice(&[1, note, velocity]);
                }
                EventKind::Off { note } => {
                    synth.off(note);
                    log.extend_from_slice(&[0, note, 0]);
                }
            }
        }
        synth.render(&mut buf[pos..]);
        samples.extend_from_slice(&buf);
        frame += u64::from(PERIOD);
    }
    let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
    Rendered { samples, events: log, notes, peak, dropped: events.dropped + e.overflow }
}

fn wav_bytes(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 4) as u32; // 16-bit stereo
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&RATE.to_le_bytes());
    b.extend_from_slice(&(RATE * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        b.extend_from_slice(&v.to_le_bytes());
        b.extend_from_slice(&v.to_le_bytes());
    }
    b
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

// ---------------------------------------------------------------- arguments

fn order_by_name(s: &str) -> Option<Order> {
    let key = s.to_ascii_lowercase().replace([' ', '-', '_', '.', ','], "");
    Order::ALL.into_iter().find(|o| o.label().to_ascii_lowercase().replace([' ', '-', '_', '.', ','], "") == key
        || format!("{o:?}").to_ascii_lowercase() == key)
}

fn rate_by_name(s: &str) -> Option<Rate> {
    Rate::ALL.into_iter().find(|r| r.label().eq_ignore_ascii_case(s))
}

fn scale_by_name(s: &str) -> Option<Scale> {
    const ROOTS: [&str; 12] = ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"];
    // "D dorian", "d_dorian" and "c#-minor" all work, so no shell quoting is needed.
    let s = s.trim().to_ascii_lowercase().replace(['_', '-'], " ");
    let (root, rest) = s.split_once(' ')?;
    let root = ROOTS.iter().position(|r| *r == root)?;
    let (_, mask) = SCALES.iter().find(|(name, _)| name.to_ascii_lowercase() == rest.trim())?;
    Some(Scale::new(root as u8, *mask))
}

fn notes(s: &str, pads: bool) -> Result<Vec<u8>, String> {
    s.split(',')
        .map(|n| {
            let v: u8 = n.trim().parse().map_err(|_| format!("{n:?} is not a number"))?;
            if pads {
                if !(1..=16).contains(&v) {
                    return Err(format!("pad {v}: pads are 1 to 16"));
                }
                Ok(v - 1)
            } else if v > 127 {
                Err(format!("note {v}: notes are 0 to 127"))
            } else {
                Ok(v)
            }
        })
        .collect()
}

fn base(name: &str, about: &str, mode: Mode) -> Job {
    Job {
        name: name.into(),
        about: about.into(),
        settings: Settings { mode, ..Settings::default() },
        pattern: Pattern::default(),
        chords: vec![vec![60, 63, 67, 70]],
        drums: false,
        bpm: 110.0,
        bars: 4,
        seed: 1,
    }
}

/// A minor, F major, G7, E minor: the loop the demos and `--patterns` play
/// over unless notes are given.
fn four_chord_loop() -> Vec<Vec<u8>> {
    vec![vec![57, 60, 64, 67], vec![53, 57, 60, 64], vec![55, 59, 62, 65], vec![52, 55, 59, 64]]
}

/// One job per pattern in a library, with the other options as given.
fn library_jobs(lib: &Library, job: &Job, chords: Option<Vec<Vec<u8>>>) -> Vec<Job> {
    let chords = chords.unwrap_or_else(four_chord_loop);
    lib.iter()
        .enumerate()
        .map(|(i, (name, pattern))| {
            let mut j = job.clone();
            j.name = format!("{:02}-{}", i + 1, slug(name));
            j.about = format!("Pattern \"{name}\" ({})", pattern.to_text());
            j.pattern = *pattern;
            j.settings.mode = Mode::Pattern;
            j.settings = j.settings.clamped();
            j.chords = chords.clone();
            j
        })
        .collect()
}

fn demo_jobs() -> Vec<Job> {
    let mut jobs = Vec::new();
    let prog = four_chord_loop();
    for (i, order) in Order::ALL.into_iter().enumerate() {
        let mut j = base(&format!("order-{:02}-{}", i + 1, slug(order.label())),
                         &format!("Arp, order {}, 2 octaves, 1/16, over a four-chord loop", order.label()), Mode::Arp);
        j.settings.order = order;
        j.settings.octaves = 2;
        j.chords = prog.clone();
        if order == Order::Played {
            // Pressed in a mixed-up order, so As played does not sound like Up.
            j.chords = prog.iter().map(|c| vec![c[1], c[3], c[0], c[2]]).collect();
            j.about.push_str(", each chord pressed 2nd, 4th, 1st, 3rd note");
        }
        jobs.push(j);
    }
    for (i, (name, text)) in BUILT_IN.iter().enumerate() {
        let mut j = base(&format!("pattern-{:02}-{}", i + 1, slug(name)),
                         &format!("Pattern \"{name}\" ({text}) over the same four-chord loop"), Mode::Pattern);
        j.pattern = Pattern::parse(text).expect("built-in patterns parse");
        j.chords = prog.clone();
        jobs.push(j);
    }
    let mut j = base("drums-01-note-repeat", "Note Repeat on kick and closed hat (pads 1 and 3), 1/16, swing 58", Mode::Repeat);
    j.drums = true;
    j.chords = vec![vec![0, 2]];
    j.settings.swing = 58;
    j.bpm = 92.0;
    jobs.push(j);
    let mut j = base("drums-02-pattern", "Latch pattern over pads 1, 2, 3, 4: \"1 3 2 3 1 3 . 4 | vel 120 60 90 60\"", Mode::Pattern);
    j.drums = true;
    j.chords = vec![vec![0, 1, 2, 3]];
    j.pattern = Pattern::parse("1 3 2 3 1 3 . 4 | vel 120 60 90 60").unwrap();
    j.bpm = 96.0;
    jobs.push(j);
    let mut j = base("drums-03-rolls", "Pattern with ratchets and chance over pads 1, 3, 4, 5: \"1 x:2 2 x:3 1 ?%60 3 x:4\"", Mode::Pattern);
    j.drums = true;
    j.chords = vec![vec![0, 2, 3, 4]];
    j.pattern = Pattern::parse("1 x:2 2 x:3 1 ?%60 3 x:4").unwrap();
    j.bpm = 100.0;
    jobs.push(j);
    let mut j = base("lanes-01-polymeter", "Eight steps against a 5-step octave lane, a 3-step velocity lane and a 4-step gate lane: \"x x x x x x x x | oct 0 1 0 -1 0 | vel 120 60 90 | gate 30 90 60 15\"", Mode::Pattern);
    j.pattern = Pattern::parse("x x x x x x x x | oct 0 1 0 -1 0 | vel 120 60 90 | gate 30 90 60 15").unwrap();
    j.chords = prog.clone();
    jobs.push(j);
    let mut j = base("scale-01-d-dorian", "One held note plays a line through scale degrees in D dorian", Mode::Pattern);
    j.pattern = Pattern::parse("1 1 1 1 1 1 1 1 | deg 0 2 4 5 7 4 2 | gate 40 40 80").unwrap();
    j.settings.scale = scale_by_name("d dorian");
    j.chords = vec![vec![62], vec![62], vec![57], vec![60]];
    jobs.push(j);
    let mut j = base("feel-01-strum", "Chord order with a 40% strum, 1/8", Mode::Arp);
    j.settings.order = Order::Chord;
    j.settings.strum = 40;
    j.settings.rate = Rate::Eighth;
    j.settings.gate = 90;
    j.chords = prog.clone();
    jobs.push(j);
    let mut j = base("feel-02-before-mutation", "Pattern \"Climb\" as written, to compare with feel-03", Mode::Pattern);
    j.pattern = Pattern::parse(BUILT_IN[3].1).unwrap();
    j.chords = prog.clone();
    j.bars = 2;
    jobs.push(j.clone());
    let mut m = j;
    m.name = "feel-03-mutated".into();
    m.about = "The 40% mutation of \"Climb\" (seed 7) on its own".into();
    m.pattern = m.pattern.mutated(7, 40);
    jobs.push(m);
    let groove = "1 2 2 2 1 2 2 1 | nudge 0 16 -6 10 0 18 -4 8 | vel 120 70 90 60";
    let mut j = base("feel-04-nudge", &format!("Kick and hat (pads 1 and 3) with single steps pushed early and late: \"{groove}\""), Mode::Pattern);
    j.drums = true;
    j.chords = vec![vec![0, 2]];
    j.pattern = Pattern::parse(groove).unwrap();
    j.bpm = 94.0;
    jobs.push(j);
    let mut j = base("feel-05-ramp", "Arp Up, 2 octaves, velocity 120 falling to 25 over each bar (ramp 25:16, retrigger on each chord)", Mode::Arp);
    j.settings.octaves = 2;
    j.settings.velocity = mpfree_arp::Velocity::Fixed(120);
    j.settings.ramp = Ramp { target: 25, steps: 16 };
    j.settings.retrigger = true;
    j.chords = prog.clone();
    jobs.push(j);
    let loose = "x x x x x x x x | vel~30 | gate~35 50 | nudge~8";
    let mut j = base("lanes-02-random", &format!("The same eight steps varied every time they play: \"{loose}\""), Mode::Pattern);
    j.pattern = Pattern::parse(loose).unwrap();
    j.chords = prog.clone();
    jobs.push(j);
    jobs
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn utc_stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z", rem / 3600, rem / 60 % 60, rem % 60)
}

fn parse_args(args: &[String]) -> Result<(Vec<Job>, Option<PathBuf>), String> {
    let mut demo = false;
    let mut out = None;
    let mut job = base("custom", "", Mode::Arp);
    let mut chords: Option<Vec<Vec<u8>>> = None;
    let mut library: Option<Library> = None;
    let mut i = 0;
    let value = |i: &mut usize| -> Result<String, String> {
        *i += 1;
        args.get(*i).cloned().ok_or_else(|| format!("{} needs a value", args[*i - 1]))
    };
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--demo" => demo = true,
            "--out" => out = Some(PathBuf::from(value(&mut i)?)),
            "--name" => job.name = slug(&value(&mut i)?),
            "--mode" => {
                job.settings.mode = match value(&mut i)?.as_str() {
                    "repeat" => Mode::Repeat,
                    "arp" => Mode::Arp,
                    "pattern" => Mode::Pattern,
                    m => return Err(format!("--mode {m}: use repeat, arp or pattern")),
                }
            }
            "--order" => {
                let v = value(&mut i)?;
                job.settings.order = order_by_name(&v).ok_or(format!("--order {v}: unknown order"))?;
            }
            "--pattern" => {
                let v = value(&mut i)?;
                job.pattern = Pattern::parse(&v).map_err(|e| format!("--pattern: {e}"))?;
                job.settings.mode = Mode::Pattern;
            }
            "--builtin" => {
                let v = value(&mut i)?;
                let (_, text) = BUILT_IN.iter().find(|(n, _)| n.eq_ignore_ascii_case(&v))
                    .ok_or(format!("--builtin {v}: unknown pattern"))?;
                job.pattern = Pattern::parse(text).unwrap();
                job.settings.mode = Mode::Pattern;
            }
            "--patterns" => {
                let path = value(&mut i)?;
                let text = std::fs::read_to_string(&path).map_err(|e| format!("--patterns {path}: {e}"))?;
                let lib = Library::parse(&text).map_err(|errors| {
                    let lines: Vec<String> = errors.iter().map(|e| format!("  {path}: {e}")).collect();
                    format!("--patterns: fix these lines:\n{}", lines.join("\n"))
                })?;
                if lib.is_empty() {
                    return Err(format!("--patterns {path}: no patterns in the file"));
                }
                library = Some(lib);
            }
            "--chords" => {
                let v = value(&mut i)?;
                chords = Some(v.split(';').map(|c| notes(c, false)).collect::<Result<_, _>>()?);
            }
            "--notes" => chords = Some(vec![notes(&value(&mut i)?, false)?]),
            "--pads" => {
                chords = Some(vec![notes(&value(&mut i)?, true)?]);
                job.drums = true;
            }
            "--bpm" => job.bpm = value(&mut i)?.parse::<f64>().map_err(|_| "--bpm needs a number")?.clamp(20.0, 300.0),
            "--rate" => {
                let v = value(&mut i)?;
                job.settings.rate = rate_by_name(&v).ok_or(format!("--rate {v}: use 1/4, 1/8, 1/8T, 1/16, 1/16T, 1/32, 1/32T"))?;
            }
            "--octaves" | "--gate" | "--swing" | "--strum" => {
                let v: u8 = value(&mut i)?.parse().map_err(|_| format!("{a} needs a whole number"))?;
                match a {
                    "--octaves" => job.settings.octaves = v,
                    "--gate" => job.settings.gate = v,
                    "--swing" => job.settings.swing = v,
                    _ => job.settings.strum = v,
                }
            }
            "--scale" => {
                let v = value(&mut i)?;
                job.settings.scale = Some(scale_by_name(&v).ok_or(format!("--scale {v:?}: use a root and a name, e.g. \"d dorian\""))?);
            }
            "--bars" => job.bars = value(&mut i)?.parse::<u32>().map_err(|_| "--bars needs a number")?.clamp(1, 64),
            "--seed" => job.seed = value(&mut i)?.parse().map_err(|_| "--seed needs a number")?,
            "--ramp" => {
                let v = value(&mut i)?;
                let bad = || format!("--ramp {v}: use TARGET:STEPS, e.g. 30:16 (velocity 30 after 16 steps)");
                let (t, n) = v.split_once(':').ok_or_else(bad)?;
                let target: u8 = t.parse().map_err(|_| bad())?;
                let steps: u16 = n.parse().map_err(|_| bad())?;
                if !(1..=127).contains(&target) {
                    return Err(bad());
                }
                job.settings.ramp = Ramp { target, steps };
            }
            "--retrigger" => job.settings.retrigger = true,
            "-h" | "--help" => return Err(String::new()),
            _ => return Err(format!("unknown option {a}")),
        }
        i += 1;
    }
    if demo {
        return Ok((demo_jobs(), out));
    }
    if let Some(lib) = library {
        if chords.as_ref().is_some_and(|c| c.iter().any(|c| c.is_empty())) {
            return Err("a chord is empty".into());
        }
        return Ok((library_jobs(&lib, &job, chords), out));
    }
    let chords = chords.ok_or("give --demo, or notes with --chords, --notes or --pads")?;
    if chords.iter().any(|c| c.is_empty()) {
        return Err("a chord is empty".into());
    }
    job.settings = job.settings.clamped();
    job.about = format!("{:?} mode, order {}, rate {}, pattern \"{}\"", job.settings.mode, job.settings.order.label(),
                        job.settings.rate.label(), job.pattern.to_text());
    job.chords = chords;
    Ok((vec![job], out))
}

fn run(jobs: &[Job], dir: &Path) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut report = String::new();
    let _ = writeln!(report, "mpfree-arp-render report ({} file{}, {} Hz, 16-bit stereo, {} frames per period)\nfolder: {}\n\
                              events = hash of the notes played, the same on every computer; \
                              audio = hash of the WAV, which can differ between computers\n",
                     jobs.len(), if jobs.len() == 1 { "" } else { "s" }, RATE, PERIOD, dir.display());
    for job in jobs {
        let r = render(job);
        let bytes = wav_bytes(&r.samples);
        let file = dir.join(format!("{}.wav", job.name));
        std::fs::write(&file, &bytes).map_err(|e| format!("{}: {e}", file.display()))?;
        let line = format!(
            "{}.wav  {:.1} s  {} notes  peak {:.1} dBFS  dropped {}  events {:016x}  audio {:016x}\n    {} ({} bpm, {} bars)\n",
            job.name, r.samples.len() as f64 / f64::from(RATE), r.notes, 20.0 * r.peak.max(1e-9).log10(), r.dropped,
            fnv(&r.events), fnv(&bytes), job.about, job.bpm, job.bars);
        print!("{line}");
        report.push_str(&line);
    }
    std::fs::write(dir.join("report.txt"), &report).map_err(|e| format!("report.txt: {e}"))?;
    Ok(report)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (jobs, out) = match parse_args(&args) {
        Ok(v) => v,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("mpfree-arp-render: {e}");
            }
            eprintln!("usage: mpfree-arp-render --demo | (--chords \"60,63,67;65,68,72\" | --notes 60,64 | --pads 1,3) \
                       [--mode repeat|arp|pattern] [--order NAME] [--pattern TEXT | --builtin NAME] [--bpm N] [--rate 1/16] \
                       [--octaves N] [--gate N] [--swing N] [--strum N] [--scale d_dorian] [--ramp 30:16] [--retrigger] [--bars N] [--seed N] \
                       [--name NAME] [--out DIR]\n       mpfree-arp-render --patterns FILE [the same options; --chords etc. replace the four-chord loop]");
            std::process::exit(if e.is_empty() { 0 } else { 2 });
        }
    };
    let dir = out.unwrap_or_else(|| {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        let work = std::env::var_os("MPFREE_WORK_DIR").map(PathBuf::from).unwrap_or_else(|| home.join("mpfree-work"));
        work.join(format!("arp-render-{}", utc_stamp()))
    });
    match run(&jobs, &dir) {
        Ok(_) => println!("\nWAV files and report.txt are in {}", dir.display()),
        Err(e) => {
            eprintln!("mpfree-arp-render: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_are_deterministic_and_valid_wav() {
        let jobs = demo_jobs();
        assert!(jobs.len() >= 25);
        let mut names: Vec<&str> = jobs.iter().map(|j| j.name.as_str()).collect();
        names.dedup();
        assert_eq!(names.len(), jobs.len());
        for job in [&jobs[0], &jobs[12], jobs.iter().find(|j| j.drums).unwrap()] {
            let a = wav_bytes(&render(job).samples);
            let b = wav_bytes(&render(job).samples);
            assert_eq!(fnv(&a), fnv(&b), "{}", job.name);
            assert!(!render(job).events.is_empty());
            assert_eq!(&a[..4], b"RIFF");
            assert_eq!(u32::from_le_bytes(a[40..44].try_into().unwrap()) as usize, a.len() - 44);
        }
    }

    #[test]
    fn every_demo_makes_sound_without_clipping_or_drops() {
        for job in demo_jobs() {
            let r = render(&job);
            assert!(r.notes > 0, "{}", job.name);
            assert!(r.peak > 0.05 && r.peak < 1.0, "{}: peak {}", job.name, r.peak);
            assert_eq!(r.dropped, 0, "{}", job.name);
        }
    }

    #[test]
    fn arguments() {
        let a = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
        let (jobs, _) = parse_args(&a("--notes 60,64 --order updown --rate 1/8T --scale c#_minor")).unwrap();
        assert_eq!(jobs[0].settings.scale, Some(Scale::new(1, mpfree_arp::scale::MINOR)));
        assert_eq!(jobs[0].settings.order, Order::UpDown);
        assert_eq!(jobs[0].settings.rate, Rate::EighthTriplet);
        let (jobs, _) = parse_args(&a("--notes 60 --mode arp --ramp 30:16 --retrigger")).unwrap();
        assert_eq!(jobs[0].settings.ramp, Ramp { target: 30, steps: 16 });
        assert!(jobs[0].settings.retrigger);
        assert!(parse_args(&a("--notes 60 --ramp 0:16")).is_err());
        assert!(parse_args(&a("--notes 60 --ramp 16")).is_err());
        let (jobs, _) = parse_args(&a("--pads 1,16 --mode repeat")).unwrap();
        assert!(jobs[0].drums);
        assert_eq!(jobs[0].chords, vec![vec![0, 15]]);
        assert!(parse_args(&a("--pads 17")).is_err());
        let example = concat!(env!("CARGO_MANIFEST_DIR"), "/patterns/example.txt");
        let (jobs, _) = parse_args(&a(&format!("--patterns {example} --bpm 96"))).unwrap();
        assert_eq!(jobs.len(), 8);
        assert_eq!(jobs[0].name, "01-skip");
        assert_eq!(jobs[6].name, "07-loose-eight");
        assert!(jobs.iter().all(|j| j.bpm == 96.0 && j.settings.mode == Mode::Pattern && j.chords.len() == 4));
        let (jobs, _) = parse_args(&a(&format!("--pads 1,3 --patterns {example}"))).unwrap();
        assert!(jobs.iter().all(|j| j.drums && j.chords == vec![vec![0, 2]]));
        for job in &jobs {
            let r = render(job);
            assert!(r.notes > 0 && r.dropped == 0, "{}", job.name);
        }
        let bad = std::env::temp_dir().join(format!("mpfree-arp-render-bad-{}.txt", std::process::id()));
        std::fs::write(&bad, "Ok = x\nBad = x y\n").unwrap();
        let e = parse_args(&a(&format!("--patterns {}", bad.display()))).err().unwrap();
        let _ = std::fs::remove_file(&bad);
        assert!(e.contains("line 2: \"Bad\", token 2 \"y\""), "{e}");
        assert!(parse_args(&a("--notes 60 --order sideways")).is_err());
        assert!(parse_args(&a("--mode arp")).is_err());
        assert_eq!(order_by_name("Random once"), Some(Order::RandomOnce));
        assert_eq!(order_by_name("randomother"), Some(Order::RandomOther));
        assert_eq!(scale_by_name("D dorian"), Some(Scale::new(2, mpfree_arp::scale::DORIAN)));
        assert_eq!(scale_by_name("harmonic_minor a"), None);
        assert_eq!(scale_by_name("a harmonic_minor"), Some(Scale::new(9, mpfree_arp::scale::HARMONIC_MINOR)));
        assert_eq!(scale_by_name("h major"), None);
        assert_eq!(slug("Up-down incl."), "up-down-incl");
        assert_eq!(utc_stamp().len(), 16);
    }
}
