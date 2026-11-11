//! WORKSTATION ONLY: CAPTURE heard. A beat plays through the transport, the
//! player and the real sampler engine; the pads play along without
//! recording, and CAPTURE keeps what they played. It never touches any
//! device; it only writes one WAV file.
//!
//! ```text
//! cargo run --release -p mpfree-capture --example capture_demo -- [--period N] [--out FILE]
//! ```
//!
//! 90 BPM. A two-bar drum clip plays from bar 1 on track 1. Track 2 is
//! armed and empty, and nothing records:
//!
//! - Bars 3 and 4: a distorted chord stab is played on the pads, a little
//!   off the grid. It is heard live and not recorded.
//! - Bar 4, beat 4: CAPTURE. The riff becomes a new two-bar clip on track
//!   2, in its place: bar 5 is its first bar, as bar 3 was. Bars 5 to 12
//!   play it from the clip, with the timing it was played with.
//! - Bar 7: a metal clank is played on the off-beats over the clip.
//! - Bar 8: CAPTURE again. Played in time over the clip that is playing,
//!   the clanks go into it, where they were played: bars 9 and 11 have
//!   them.
//! - The transport stops on bar 13 and the last hits ring out.
//!
//! The tool then undoes both captures on a copy of the song (take undo)
//! and checks that the clanks, then the clip, are gone. It prints the bars
//! with what played live and what played from the clip, and checks every
//! one. Every sound is synthesised here; no sample files are used.
//!
//! `events` hashes every note heard (frame, pad, velocity, live or from a
//! clip) and is the same on every computer and for every period size; the
//! tool checks that itself at 1, 128 and 4096 frames. `audio` hashes the
//! WAV bytes; it repeats on one computer.

use std::f32::consts::TAU;

use mpfree_capture::{keep, Capture, Now};
use mpfree_play::{Event, Launch, Out, PlayView, Player, Sink, Takes};
use mpfree_sampler::engine::{Engine, EngineView, Event as SEvent};
use mpfree_sampler::model::{Layer, Pad, PlayMode, Project as SProject, Region, Source};
use mpfree_seq::{Clip, Note, Project, Track};
use mpfree_transport::time::Tick;
use mpfree_transport::{Block, Command, Transport};

const RATE: u32 = 44_100;
const BPM: f64 = 90.0;
const BAR: Tick = 3840;
const BEAT: Tick = 960;
const S: Tick = 240;
const MASTER: f32 = 0.6;
const TAIL: f64 = 2.5;
const KICK: u8 = 0;
const SNARE: u8 = 1;
const HAT: u8 = 2;
const STAB: u8 = 3;
const CLANK: u8 = 4;
const DRUMS: usize = 0;
const TAKE: usize = 1;
/// The stab riff as played over bars 3 and 4 (song ticks): a little late
/// or early here and there.
const RIFF: [(Tick, u8); 7] = [
    (2 * BAR + 12, 120),
    (2 * BAR + 712, 96),
    (2 * BAR + 1460, 110),
    (2 * BAR + 2885, 104),
    (3 * BAR + 30, 118),
    (3 * BAR + 970, 100),
    (3 * BAR + 2415, 112),
];
/// The clanks over bar 7.
const CLANKS: [(Tick, u8); 4] = [
    (6 * BAR + 490, 100),
    (6 * BAR + 1435, 92),
    (6 * BAR + 2408, 104),
    (6 * BAR + 3360, 96),
];
/// When CAPTURE is pressed: after the last note of each phrase, before the
/// clip's next note.
const PRESS: [Tick; 2] = [3 * BAR + 3000, 7 * BAR + 480];
const STOP_AT: Tick = 12 * BAR;

fn main() {
    let mut period = 128u32;
    let mut out: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--period" => {
                period = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .filter(|p| (1..=8192).contains(p))
                    .unwrap_or_else(|| fail("--period takes 1 to 8192"))
            }
            "--out" => {
                out = Some(
                    args.next()
                        .unwrap_or_else(|| fail("--out takes a file name")),
                )
            }
            "-h" | "--help" => {
                println!("capture_demo [--period N] [--out FILE]   (default FILE: ~/mpfree-work/capture-demo.wav)");
                return;
            }
            other => fail(&format!("unknown option {other}")),
        }
    }
    let out = out.unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        format!("{home}/mpfree-work/capture-demo.wav")
    });
    let frames = frame_of(STOP_AT) + (TAIL * f64::from(RATE)) as u64;

    // The events must not depend on the period size.
    let hashes: Vec<(u32, u64)> = [1u32, 128, 4096]
        .iter()
        .map(|&p| {
            let mut s = Session::new();
            s.run(frames, p, None);
            s.check();
            (p, s.hash())
        })
        .collect();
    let same = hashes.windows(2).all(|w| w[0].1 == w[1].1);

    let mut engine = Engine::new(RATE, 32);
    engine.set_view(EngineView::build(&sounds(), RATE));
    let mut s = Session::new();
    let pcm = s.run(frames, period, Some(&mut engine));
    s.check();
    let wav = wav(&pcm.data);
    if let Some(dir) = std::path::Path::new(&out).parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)
                .unwrap_or_else(|e| fail(&format!("cannot make {}: {e}", dir.display())));
        }
    }
    std::fs::write(&out, &wav).unwrap_or_else(|e| fail(&format!("cannot write {out}: {e}")));

    println!(
        "wrote {out}: {:.1} s, 44.1 kHz stereo 16-bit, period {period} frames",
        frames as f64 / f64::from(RATE)
    );
    println!(
        "notes heard: {}  dropped by the sampler: {}  peak: {:.1} dBFS  clipped samples: {}",
        s.heard.len(),
        engine.stats().dropped_events,
        20.0 * pcm.peak.log10(),
        pcm.clipped
    );
    for line in &s.lines {
        println!("{line}");
    }
    s.print_bars();
    println!("undo on a copy: capture 2 takes the clanks out, capture 1 the clip: checked");
    println!("events {:016x}  (the same on every computer)", s.hash());
    println!("audio  {:016x}  (repeats on this computer)", fnv(&wav));
    for (p, h) in &hashes {
        println!("  events at period {p:>4}: {h:016x}");
    }
    let ok = same && s.hash() == hashes[0].1;
    println!(
        "same events for every period size: {}",
        if ok { "yes" } else { "NO" }
    );
    if !ok {
        std::process::exit(1);
    }
    if pcm.clipped > 0 {
        eprintln!(
            "capture_demo: warning: {} samples clipped at the master",
            pcm.clipped
        );
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("capture_demo: {msg}");
    std::process::exit(2);
}

/// The frame of song tick `tick` (the tempo never changes, and nothing
/// loops).
fn frame_of(tick: Tick) -> u64 {
    (tick as f64 * 60.0 * f64::from(RATE) / (BPM * 960.0)).round() as u64
}

/// The bar (from 1) and the tick in it of an absolute frame.
fn bar_of(frame: u64) -> (i64, Tick) {
    let tick = (frame as f64 * BPM * 960.0 / (60.0 * f64::from(RATE))).round() as Tick;
    (tick / BAR + 1, tick % BAR)
}

struct Pcm {
    data: Vec<u8>,
    peak: f32,
    clipped: usize,
}

/// A note heard: (frame, pad, velocity, played live).
type Heard = (u64, u8, u8, bool);

/// The player's notes, to the sampler and the list of notes heard.
struct Route<'a> {
    heard: &'a mut Vec<Heard>,
    sampler: Option<&'a mut Engine>,
}

impl Sink for Route<'_> {
    fn send(&mut self, o: Out) {
        if let Event::NoteOn { note, velocity } = o.event {
            self.heard.push((o.frame, note, velocity, false));
        }
        if let Some(e) = self.sampler.as_deref_mut() {
            to_sampler(e, o.offset, o.event);
        }
    }
}

fn to_sampler(engine: &mut Engine, offset: u32, event: Event) {
    let ev = match event {
        Event::NoteOn { note, velocity } => SEvent::NoteOn {
            pad: note,
            velocity,
        },
        Event::NoteOff { note } => SEvent::NoteOff { pad: note },
        Event::AllOff => SEvent::AllOff,
        Event::Lock { .. } | Event::Unlock { .. } => return,
    };
    engine.schedule(offset, ev);
}

/// The audio thread's part (transport, live pads, capture, player) and the
/// control thread's (CAPTURE pressed, a new view compiled).
struct Session {
    project: Project,
    launches: Vec<Launch>,
    view: PlayView,
    player: Box<Player>,
    capture: Box<Capture>,
    takes: Takes,
    live: Vec<(u64, Event)>,
    next_live: usize,
    presses: usize,
    heard: Vec<Heard>,
    lines: Vec<String>,
}

impl Session {
    fn new() -> Session {
        let mut project = Project::new();
        let mut drums = Track::new("Drums", 0);
        let mut beat = Clip::new("Beat", 2 * BAR);
        let hits = [
            (0, KICK, 120),
            (BEAT, SNARE, 110),
            (1680, KICK, 96),
            (3 * BEAT, SNARE, 116),
            (BAR, KICK, 118),
            (BAR + BEAT, SNARE, 108),
            (BAR + 2160, KICK, 90),
            (BAR + 2400, KICK, 104),
            (BAR + 3 * BEAT, SNARE, 116),
        ];
        for (tick, note, velocity) in hits {
            beat.add_note(Note {
                tick,
                len: S,
                note,
                velocity,
                chance: 100,
            })
            .expect("in range");
        }
        for i in 0..16 {
            beat.add_note(Note {
                tick: i * 2 * S,
                len: S / 2,
                note: HAT,
                velocity: if i % 2 == 0 { 80 } else { 58 },
                chance: 100,
            })
            .expect("in range");
        }
        drums.slots[0] = Some(beat);
        project.tracks.push(drums);
        project.tracks.push(Track::new("Stab", 0));
        let launches = vec![Launch {
            track: DRUMS,
            slot: 0,
            origin: 0,
        }];
        let mut live: Vec<(u64, Event)> = Vec::new();
        for &(tick, velocity) in &RIFF {
            live.push((
                frame_of(tick),
                Event::NoteOn {
                    note: STAB,
                    velocity,
                },
            ));
            live.push((frame_of(tick + S), Event::NoteOff { note: STAB }));
        }
        for &(tick, velocity) in &CLANKS {
            live.push((
                frame_of(tick),
                Event::NoteOn {
                    note: CLANK,
                    velocity,
                },
            ));
            live.push((frame_of(tick + S / 2), Event::NoteOff { note: CLANK }));
        }
        live.sort_by_key(|l| l.0);
        let view = compile(&project, &launches);
        Session {
            project,
            launches,
            view,
            player: Box::new(Player::new()),
            capture: Box::new(Capture::new()),
            takes: Takes::default(),
            live,
            next_live: 0,
            presses: 0,
            heard: Vec::new(),
            lines: Vec::new(),
        }
    }

    fn run(&mut self, frames: u64, period: u32, mut sampler: Option<&mut Engine>) -> Pcm {
        let mut t = Box::new(Transport::new(RATE, BPM).expect("valid transport"));
        t.command(0, Command::Play);
        t.command(frame_of(STOP_AT), Command::Stop);
        let mut block = Box::new(Block::new());
        let (mut left, mut right) = (vec![0.0f32; 8192], vec![0.0f32; 8192]);
        let mut pcm = Pcm {
            data: Vec::with_capacity(frames as usize * 4),
            peak: 0.0,
            clipped: 0,
        };
        while t.frame() < frames {
            let n = period.min((frames - t.frame()) as u32);
            assert!(t.process(n, &mut block));
            let end = block.start_frame + u64::from(n);
            // The pads: heard at once, and into the capture history.
            while self.next_live < self.live.len() && self.live[self.next_live].0 < end {
                let (frame, event) = self.live[self.next_live];
                self.next_live += 1;
                let offset = (frame - block.start_frame) as u32;
                self.capture.input(offset, event);
                if let Event::NoteOn { note, velocity } = event {
                    self.heard.push((frame, note, velocity, true));
                }
                if let Some(e) = sampler.as_deref_mut() {
                    to_sampler(e, offset, event);
                }
            }
            self.capture.process(&block);
            self.player.process(
                &self.view,
                &block,
                &mut Route {
                    heard: &mut self.heard,
                    sampler: sampler.as_deref_mut(),
                },
            );
            if let Some(e) = sampler.as_deref_mut() {
                let (l, r) = (&mut left[..n as usize], &mut right[..n as usize]);
                e.render(l, r);
                for (a, b) in l.iter().zip(r.iter()) {
                    for x in [a, b] {
                        // A plain master gain with headroom: no saturation here.
                        let v = x * MASTER;
                        pcm.peak = pcm.peak.max(v.abs());
                        pcm.clipped += usize::from(v.abs() > 1.0);
                        pcm.data.extend_from_slice(
                            &((v.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes(),
                        );
                    }
                }
            }
            // CAPTURE, pressed on the control thread after this period.
            if self.presses < PRESS.len() && end > frame_of(PRESS[self.presses]) {
                self.presses += 1;
                self.press(&t);
            }
        }
        pcm
    }

    fn press(&mut self, t: &Transport) {
        let phrase = self.capture.phrase().expect("something was played");
        let now = Now {
            position: t.position(),
            rolling: t.state().rolling(),
            meter: t.meter(),
            loop_range: t.loop_range(),
        };
        let name = format!("Capture {}", self.takes.captures() + 1);
        let k = keep(
            &phrase,
            &mut self.project,
            &self.launches,
            &mut self.takes,
            TAKE,
            now,
            &name,
        )
        .expect("kept");
        if let Some(l) = k.launch {
            self.launches.retain(|x| x.track != l.track);
            self.launches.push(l);
        }
        self.capture.mark_kept(phrase.id);
        self.view = compile(&self.project, &self.launches);
        let (bar, tick) = bar_of(frame_of(t.position()));
        let what = match k.launch {
            Some(l) => format!(
                "a new clip in slot {}, playing from its place (origin tick {})",
                k.slot + 1,
                l.origin
            ),
            None => format!("into the playing clip in slot {}", k.slot + 1),
        };
        let bars = if k.bars == 1 {
            "1 bar".to_string()
        } else {
            format!("{} bars", k.bars)
        };
        self.lines.push(format!(
            "CAPTURE at bar {bar} tick {tick}: {} notes, {bars}, {what}",
            k.notes
        ));
    }

    fn hash(&self) -> u64 {
        // In frame order: a period hands over its live notes before the
        // clip's, so a long period lists them in another order.
        let mut heard = self.heard.clone();
        heard.sort();
        let mut bytes = Vec::new();
        for &(frame, pad, velocity, live) in &heard {
            bytes.extend_from_slice(&frame.to_le_bytes());
            bytes.extend_from_slice(&[pad, velocity, u8::from(live)]);
        }
        fnv(&bytes)
    }

    /// The pads' notes heard in bar `bar`, live or from a clip, as ticks
    /// in the bar.
    fn in_bar(&self, bar: i64, pad: u8, live: bool) -> Vec<Tick> {
        self.heard
            .iter()
            .filter(|h| h.1 == pad && h.3 == live)
            .map(|h| bar_of(h.0))
            .filter(|b| b.0 == bar)
            .map(|b| b.1)
            .collect()
    }

    fn check(&self) {
        assert_eq!(self.capture.dropped(), 0, "the capture dropped notes");
        assert_eq!(self.player.stats().dropped, 0, "the player dropped notes");
        assert_eq!(self.presses, 2);
        let clip = self.project.clip(TAKE, 0).expect("the captured clip");
        assert_eq!((clip.name.as_str(), clip.length), ("Capture 1", 2 * BAR));
        // The riff in the clip, ticks from the bar 3 line; the clanks went
        // in where they were played, in the clip's first bar.
        let mut want: Vec<Tick> = RIFF.iter().map(|r| r.0 - 2 * BAR).collect();
        want.extend(CLANKS.iter().map(|c| c.0 - 6 * BAR));
        want.sort();
        let mut got: Vec<Tick> = clip.notes.iter().map(|n| n.tick).collect();
        got.sort();
        assert_eq!(
            got, want,
            "the clip holds what was played, where it was played"
        );
        // Heard: the riff live in bars 3 and 4, from the clip in bars 5
        // to 12, each time where it was played in the bar.
        let riff_in = |odd: bool| -> Vec<Tick> {
            RIFF.iter()
                .filter(|r| (r.0 / BAR == 2) == odd)
                .map(|r| r.0 % BAR)
                .collect()
        };
        for bar in 1..=13 {
            let clip_stabs = self.in_bar(bar, STAB, false);
            let live_stabs = self.in_bar(bar, STAB, true);
            let want_clip = if (5..=12).contains(&bar) {
                riff_in(bar % 2 == 1)
            } else {
                vec![]
            };
            let want_live = if (3..=4).contains(&bar) {
                riff_in(bar == 3)
            } else {
                vec![]
            };
            assert!(
                close(&clip_stabs, &want_clip),
                "bar {bar}: stabs from the clip {clip_stabs:?}, want {want_clip:?}"
            );
            assert!(
                close(&live_stabs, &want_live),
                "bar {bar}: live stabs {live_stabs:?}"
            );
            let clanks = self.in_bar(bar, CLANK, false);
            let want: Vec<Tick> = if bar == 9 || bar == 11 {
                CLANKS.iter().map(|c| c.0 % BAR).collect()
            } else {
                vec![]
            };
            assert!(
                close(&clanks, &want),
                "bar {bar}: clanks from the clip {clanks:?}"
            );
        }
        // Undo on a copy: the clanks go, then the clip.
        let (mut p, mut takes) = (self.project.clone(), self.takes.clone());
        assert_eq!(takes.undo(&mut p), Some(2));
        assert_eq!(p.clip(TAKE, 0).map(|c| c.notes.len()), Some(RIFF.len()));
        assert_eq!(takes.undo(&mut p), Some(1));
        assert!(p.clip(TAKE, 0).is_none());
    }

    fn print_bars(&self) {
        println!("bar  stab (live / clip)   clank (live / clip)");
        for bar in 1..=12 {
            let n = |pad, live| self.in_bar(bar, pad, live).len();
            println!(
                "{bar:>3}  {:>4} / {:<4}         {:>4} / {:<4}",
                n(STAB, true),
                n(STAB, false),
                n(CLANK, true),
                n(CLANK, false)
            );
        }
    }
}

/// Every tick within a tick of the wanted one (frames round to ticks).
fn close(got: &[Tick], want: &[Tick]) -> bool {
    got.len() == want.len() && got.iter().zip(want).all(|(a, b)| (a - b).abs() <= 1)
}

fn compile(project: &Project, launches: &[Launch]) -> PlayView {
    PlayView::compile(project, launches).expect("the song compiles")
}

/// A repeatable noise source.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / u32::MAX as f32 * 2.0 - 1.0
    }
}

/// A one-pole low-pass filter.
struct Lp {
    a: f32,
    y: f32,
}

impl Lp {
    fn new(hz: f32) -> Lp {
        Lp {
            a: 1.0 - (-TAU * hz / RATE as f32).exp(),
            y: 0.0,
        }
    }

    fn run(&mut self, x: f32) -> f32 {
        self.y += self.a * (x - self.y);
        self.y
    }
}

/// Saturation: soft clipping that keeps full scale at full scale.
fn drive(x: f32, k: f32) -> f32 {
    (k * x).tanh() / k.tanh()
}

/// Bit crushing to `bits` bits.
fn crush(x: f32, bits: u32) -> f32 {
    let q = (1u32 << (bits - 1)) as f32;
    (x * q).round() / q
}

/// `secs` seconds of `f(t)`, as stereo frames. The last 20 ms fade out,
/// so no sound ends with a click.
fn make(secs: f32, mut f: impl FnMut(f32) -> f32) -> Vec<f32> {
    let n = (secs * RATE as f32) as usize;
    let fade = (RATE as usize / 50).min(n);
    (0..n)
        .flat_map(|i| {
            let left = n - i;
            let gain = if left < fade {
                0.5 - 0.5 * (std::f32::consts::PI * left as f32 / fade as f32).cos()
            } else {
                1.0
            };
            let s = gain * f(i as f32 / RATE as f32);
            [s, s]
        })
        .collect()
}

/// A sawtooth from `harmonics` sine partials.
fn saw(hz: f32, t: f32, harmonics: usize) -> f32 {
    (1..=harmonics)
        .map(|k| (TAU * hz * k as f32 * t).sin() / k as f32)
        .sum()
}

/// Synthesised, dirty sounds on pads 0 to 4: kick, snare, hat, stab, clank.
fn sounds() -> SProject {
    let mut n = Noise(0x2468_ace1);
    let mut phase = 0.0f32;
    let kick = make(0.8, |t| {
        let f = 42.0 + 110.0 * (-t * 24.0).exp();
        phase += TAU * f / RATE as f32;
        let body = phase.sin() * (-t * 5.0).exp();
        let click = n.next() * (-t * 400.0).exp() * 0.5;
        0.75 * drive(1.3 * body + click, 2.5)
    });
    let mut lp = Lp::new(1500.0);
    let snare = make(0.35, |t| {
        let body = (TAU * 185.0 * t).sin() * (-t * 22.0).exp();
        let x = n.next();
        0.65 * crush(
            drive(0.5 * body + 0.8 * (x - lp.run(x)) * (-t * 11.0).exp(), 4.0),
            8,
        )
    });
    let mut lp = Lp::new(5000.0);
    let hat = make(0.07, |t| {
        let x = n.next();
        0.35 * crush((x - lp.run(x)) * (-t * 60.0).exp(), 6)
    });
    // A D minor chord (D3 F3 A3 D4) of sawtooths through a closing filter.
    let mut y = 0.0f32;
    let stab = make(0.7, |t| {
        let s: f32 = [146.83, 174.61, 220.0, 293.66]
            .iter()
            .map(|&hz| saw(hz, t, 8))
            .sum();
        let cutoff = 500.0 + 4000.0 * (-t * 8.0).exp();
        y += (1.0 - (-TAU * cutoff / RATE as f32).exp()) * (0.25 * s - y);
        let env = (t / 0.003).min(1.0) * (-t * 5.0).exp();
        0.9 * crush(drive(1.5 * y, 2.5) * env, 8)
    });
    // Metal: inharmonic partials and a click.
    let clank = make(0.35, |t| {
        let partials = [
            (540.0, 1.0, 9.0),
            (1213.0, 0.7, 12.0),
            (1771.0, 0.5, 16.0),
            (2390.0, 0.4, 20.0),
        ];
        let s: f32 = partials
            .iter()
            .map(|&(hz, a, d)| a * (TAU * hz * t).sin() * (-t * d).exp())
            .sum();
        let click = n.next() * (-t * 250.0).exp();
        0.7 * crush(drive(0.6 * s + 0.5 * click, 3.0), 6)
    });
    let mut p = SProject::new();
    for (pad, name, data) in [
        (KICK, "kick", kick),
        (SNARE, "snare", snare),
        (HAT, "hat", hat),
        (STAB, "stab", stab),
        (CLANK, "clank", clank),
    ] {
        let s = Source::new(name, RATE, 2, data).expect("valid source");
        p.add_source(s.clone());
        let mut r = Region::whole(&s);
        r.mode = PlayMode::OneShot;
        p.set_pad(usize::from(pad), Some(Pad::new(name, Layer::new(r))), None)
            .expect("valid pad");
    }
    p
}

fn wav(pcm: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(44 + pcm.len());
    let data = pcm.len() as u32;
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&2u16.to_le_bytes()); // stereo
    b.extend_from_slice(&RATE.to_le_bytes());
    b.extend_from_slice(&(RATE * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    b.extend_from_slice(pcm);
    b
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}
