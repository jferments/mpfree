//! WORKSTATION ONLY: a beat programmed on the step grid, played through the
//! transport, the player and the real sampler engine, and written to a WAV
//! file. It never touches any device; it only writes one WAV file.
//!
//! ```text
//! cargo run --release -p mpfree-play --example steps_demo -- [--period N] [--out FILE]
//! ```
//!
//! An empty one-page clip (16 steps) is filled only through the step grid
//! (`mpfree_seq::steps`), each tap and change applied as an undoable edit
//! (`Takes::edit`), as the Play page's step mode will do it:
//!
//! - Bar 1: kicks on 1/16 steps. Snares on 2 and 4, nudged 24 ticks late
//!   (about 16 ms at 96 BPM), so they sit back. Hats on every 1/16: the
//!   off-beat 16ths quieter, and every fourth one at 50 %, so the pattern
//!   changes from pass to pass.
//! - A copied page makes bar 2, and two kicks move in it.
//! - The hat on step 3 has its velocity turned from 100 down to 70: one
//!   edit.
//! - A rising ghost-snare roll on 1/16 triplet steps at the end of bar 2,
//!   and a clank on its last 16th at 35 %.
//!
//! The beat plays four times (8 bars). The tool prints the grid, checks
//! where every note was heard, checks on a copy that UNDO takes the edits
//! back newest first, the whole velocity turn in one step and the copied
//! page in another, and prints an
//! events hash that is the same for every period size (it checks 1, 128
//! and 4096 frames). Every sound is synthesised here.

use std::f32::consts::TAU;

use mpfree_play::{Event, Launch, Out, PlayView, Player, Sink, Takes};
use mpfree_sampler::engine::{Engine, EngineView, Event as SEvent};
use mpfree_sampler::model::{Layer, Pad, PlayMode, Project as SProject, Region, Source};
use mpfree_seq::steps::{Change, Grid, PAGE};
use mpfree_seq::{Clip, Project, Track};
use mpfree_transport::time::Tick;
use mpfree_transport::{Block, Command, Transport};

const RATE: u32 = 44_100;
const BPM: f64 = 96.0;
const BAR: Tick = 3840;
const MASTER: f32 = 0.6;
const TAIL: f64 = 2.0;
const BARS: Tick = 8;
const KICK: u8 = 0;
const SNARE: u8 = 1;
const HAT: u8 = 2;
const CLANK: u8 = 3;
const SOUNDS: [(u8, &str); 4] = [
    (KICK, "kick"),
    (SNARE, "snare"),
    (HAT, "hat"),
    (CLANK, "clank"),
];
/// 1/16 steps of the kick, over both bars: bar 1, then bar 2 as its copy
/// with step 23 moved to 29.
const KICKS: [usize; 6] = [0, 7, 10, 16, 26, 29];
const BAR1_KICKS: [usize; 3] = [0, 7, 10];
const SNARES: [usize; 4] = [4, 12, 20, 28];
const LATE: Tick = 24;
/// 1/16 triplet steps of the roll (bar 2, beat 4) and their velocities.
const ROLL: [(usize, u8); 3] = [(45, 50), (46, 66), (47, 82)];

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
                println!("steps_demo [--period N] [--out FILE]   (default FILE: ~/mpfree-work/steps-demo.wav)");
                return;
            }
            other => fail(&format!("unknown option {other}")),
        }
    }
    let out = out.unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        format!("{home}/mpfree-work/steps-demo.wav")
    });

    let (project, takes) = program();
    let clip = project.tracks[0].slots[0].as_ref().expect("the clip");
    print_grid(clip);
    check_undo(&project, &takes);

    let frames = frame_of(BARS * BAR) + (TAIL * f64::from(RATE)) as u64;
    let hashes: Vec<(u32, u64)> = [1u32, 128, 4096]
        .iter()
        .map(|&p| {
            let heard = play(&project, frames, p, None).1;
            check(&heard);
            (p, hash(&heard))
        })
        .collect();
    let mut engine = Engine::new(RATE, 32);
    engine.set_view(EngineView::build(&sounds(), RATE));
    let (pcm, heard) = play(&project, frames, period, Some(&mut engine));
    check(&heard);
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
        heard.len(),
        engine.stats().dropped_events,
        20.0 * pcm.peak.log10(),
        pcm.clipped
    );
    let per_pass: Vec<usize> = (0..BARS / 2)
        .map(|p| {
            heard
                .iter()
                .filter(|h| h.1 == HAT && bar_of(h.0).0 / 2 == p)
                .count()
        })
        .collect();
    println!("hats heard in each pass of the beat: {per_pass:?} (32 steps, 8 of them at 50 %)");
    println!(
        "checked: kicks on the grid, snares {LATE} ticks late, the roll on triplets, every pass"
    );
    println!("undo on a copy: edits come back newest first; one UNDO takes the whole velocity turn back, another the copied page: checked");
    println!("events {:016x}  (the same on every computer)", hash(&heard));
    println!("audio  {:016x}  (repeats on this computer)", fnv(&wav));
    for (p, h) in &hashes {
        println!("  events at period {p:>4}: {h:016x}");
    }
    let ok = hashes.iter().all(|&(_, h)| h == hash(&heard));
    println!(
        "same events for every period size: {}",
        if ok { "yes" } else { "NO" }
    );
    if !ok {
        std::process::exit(1);
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("steps_demo: {msg}");
    std::process::exit(2);
}

/// The beat, entered step by step into an empty clip as undoable edits.
fn program() -> (Project, Takes) {
    let mut project = Project::new();
    let mut t = Track::new("Drums", 0);
    t.slots[0] = Some(Clip::new(
        "Steps",
        Grid::new(240).expect("1/16").page_length(),
    ));
    project.tracks.push(t);
    let mut takes = Takes::default();
    let sixteenths = Grid::new(240).expect("1/16");
    let triplets = Grid::new(160).expect("1/16 triplet");
    // Work out each edit from the clip as it is, then apply it.
    let mut edit =
        |grid: Grid, f: &dyn Fn(Grid, &Clip) -> mpfree_seq::steps::Edit, join: Option<u64>| {
            let e = f(grid, project.tracks[0].slots[0].as_ref().expect("the clip"));
            takes
                .edit(&mut project, 0, 0, &e, join)
                .unwrap_or_else(|e| fail(&format!("edit refused: {e:?}")));
        };
    for &i in &BAR1_KICKS {
        edit(sixteenths, &|g, c| g.toggle(c, KICK, i, 120), None);
    }
    for &i in &SNARES[..2] {
        edit(sixteenths, &|g, c| g.toggle(c, SNARE, i, 112), None);
        edit(
            sixteenths,
            &|g, c| g.change(c, SNARE, i, Change::Nudge(LATE)),
            None,
        );
    }
    for i in 0..PAGE {
        edit(sixteenths, &|g, c| g.toggle(c, HAT, i, 100), None);
        edit(
            sixteenths,
            &|g, c| g.change(c, HAT, i, Change::Length(120)),
            None,
        );
        if i % 2 == 1 {
            edit(
                sixteenths,
                &|g, c| g.change(c, HAT, i, Change::Velocity(58)),
                None,
            );
        }
        if i % 4 == 3 {
            edit(
                sixteenths,
                &|g, c| g.change(c, HAT, i, Change::Chance(50)),
                None,
            );
        }
    }
    // Bar 2: a copy of bar 1, with the kick on step 23 moved to 29.
    edit(sixteenths, &|g, c| g.add_page(c, true), None);
    edit(sixteenths, &|g, c| g.toggle(c, KICK, 23, 120), None);
    edit(sixteenths, &|g, c| g.toggle(c, KICK, 29, 120), None);
    // A Q-Link turned from 100 down to 70 on the hat of step 2: one edit.
    for v in (1..=10).map(|k| 100 - 3 * k) {
        edit(
            sixteenths,
            &|g, c| g.change(c, HAT, 2, Change::Velocity(v)),
            Some(Change::Velocity(v).key(HAT, 2)),
        );
    }
    for &(i, v) in &ROLL {
        edit(triplets, &|g, c| g.toggle(c, SNARE, i, v), None);
    }
    edit(sixteenths, &|g, c| g.toggle(c, CLANK, 31, 110), None);
    edit(
        sixteenths,
        &|g, c| g.change(c, CLANK, 31, Change::Chance(35)),
        None,
    );
    (project, takes)
}

/// The grid as a step sequencer shows it, a page per bar.
fn print_grid(clip: &Clip) {
    let g = Grid::new(240).expect("1/16");
    println!(
        "the clip, on 1/16 steps (X loud, x soft, ? by chance, > late; the roll is on triplets):"
    );
    println!(
        "         {}",
        (0..2)
            .map(|_| "1 . . . 2 . . . 3 . . . 4 . . . ")
            .collect::<Vec<_>>()
            .join("| ")
    );
    for (pitch, name) in SOUNDS {
        let row = g.row(clip, pitch);
        let cells: Vec<String> = row
            .chunks(PAGE)
            .map(|page| {
                page.iter()
                    .map(|s| match s {
                        None => ". ",
                        Some(s) if s.chance < 100 => "? ",
                        Some(s) if s.nudge > 0 => "> ",
                        Some(s) if s.velocity >= 90 => "X ",
                        Some(_) => "x ",
                    })
                    .collect::<String>()
            })
            .collect();
        println!("  {name:<6} {}", cells.join("| "));
    }
}

/// UNDO on a copy, newest edit first: the clank's chance, the clank, the
/// roll's three steps, the whole velocity turn in one UNDO, the two kicks
/// and then the copied page.
fn check_undo(project: &Project, takes: &Takes) {
    let mut p = project.clone();
    let mut t = takes.clone();
    let g = Grid::new(240).expect("1/16");
    let hat2 = |p: &Project| {
        g.row(p.tracks[0].slots[0].as_ref().expect("clip"), HAT)[2].map(|s| s.velocity)
    };
    let roll = |p: &Project| {
        Grid::new(160)
            .expect("1/16 triplet")
            .row(p.tracks[0].slots[0].as_ref().expect("clip"), SNARE)[47]
    };
    assert_eq!(hat2(&p), Some(70));
    // The clank's chance, the clank, then the roll's three steps, then the turn.
    for _ in 0..2 {
        t.undo(&mut p).unwrap_or_else(|| fail("nothing to undo"));
    }
    t.undo(&mut p);
    if roll(&p).is_some() {
        fail("undo did not take the roll's last step out");
    }
    t.undo(&mut p);
    t.undo(&mut p);
    if hat2(&p) != Some(70) {
        fail("the turn went back too soon");
    }
    t.undo(&mut p);
    if hat2(&p) != Some(100) {
        fail("one undo did not take the whole velocity turn back");
    }
    t.undo(&mut p);
    t.undo(&mut p);
    let clip = |p: &Project| p.tracks[0].slots[0].clone().expect("clip");
    if clip(&p).length != 2 * BAR {
        fail("the page went too soon");
    }
    t.undo(&mut p);
    if clip(&p).length != BAR || clip(&p).notes.len() != 3 + 2 + 16 {
        fail("one undo did not take the copied page back");
    }
}

/// A note heard: (frame, pad, velocity).
type Heard = (u64, u8, u8);

/// Each pass of the two-bar beat must play the same steps, but for the
/// notes left to chance.
fn check(heard: &[Heard]) {
    let ticks = |pitch: u8| -> Vec<Tick> {
        heard
            .iter()
            .filter(|h| h.1 == pitch)
            .map(|h| bar_of(h.0))
            .map(|(bar, t)| bar * BAR + t)
            .collect()
    };
    let close = |got: &[Tick], want: &[Tick]| {
        got.len() == want.len() && got.iter().zip(want).all(|(a, b)| (a - b).abs() <= 1)
    };
    let every_pass = |steps: &[Tick]| -> Vec<Tick> {
        (0..BARS / 2)
            .flat_map(|p| steps.iter().map(move |s| p * 2 * BAR + s))
            .collect()
    };
    let kicks: Vec<Tick> = KICKS.iter().map(|&i| i as Tick * 240).collect();
    if !close(&ticks(KICK), &every_pass(&kicks)) {
        fail(&format!("the kicks are off the grid: {:?}", ticks(KICK)));
    }
    let mut snares: Vec<Tick> = SNARES.iter().map(|&i| i as Tick * 240 + LATE).collect();
    snares.extend(ROLL.iter().map(|&(i, _)| i as Tick * 160));
    snares.sort();
    if !close(&ticks(SNARE), &every_pass(&snares)) {
        fail(&format!(
            "the snares are not where they were nudged: {:?}",
            ticks(SNARE)
        ));
    }
    let hats = ticks(HAT);
    let sure: Vec<Tick> = every_pass(
        &(0..32)
            .filter(|i| i % 4 != 3)
            .map(|i| i * 240)
            .collect::<Vec<_>>(),
    );
    if !sure.iter().all(|s| hats.iter().any(|h| (h - s).abs() <= 1)) {
        fail("a hat that always plays is missing");
    }
    let by_chance = hats.len() - sure.len();
    if !(4..=28).contains(&by_chance) {
        fail(&format!("{by_chance} of 32 hats at 50 % played"));
    }
}

fn hash(heard: &[Heard]) -> u64 {
    let bytes: Vec<u8> = heard
        .iter()
        .flat_map(|&(f, p, v)| f.to_le_bytes().into_iter().chain([p, v]))
        .collect();
    fnv(&bytes)
}

fn frame_of(tick: Tick) -> u64 {
    (tick as f64 * 60.0 * f64::from(RATE) / (BPM * 960.0)).round() as u64
}

/// The bar (from 0) and the tick in it of an absolute frame.
fn bar_of(frame: u64) -> (Tick, Tick) {
    let tick = (frame as f64 * BPM * 960.0 / (60.0 * f64::from(RATE))).round() as Tick;
    (tick / BAR, tick % BAR)
}

struct Pcm {
    data: Vec<u8>,
    peak: f32,
    clipped: usize,
}

struct Route<'a> {
    heard: &'a mut Vec<Heard>,
    sampler: Option<&'a mut Engine>,
}

impl Sink for Route<'_> {
    fn send(&mut self, o: Out) {
        if let Event::NoteOn { note, velocity } = o.event {
            self.heard.push((o.frame, note, velocity));
        }
        if let Some(e) = self.sampler.as_deref_mut() {
            let ev = match o.event {
                Event::NoteOn { note, velocity } => SEvent::NoteOn {
                    pad: note,
                    velocity,
                },
                Event::NoteOff { note } => SEvent::NoteOff { pad: note },
                Event::AllOff => SEvent::AllOff,
                // The demo's beat has no parameter locks.
                Event::Lock { .. } | Event::Unlock { .. } => return,
            };
            e.schedule(o.offset, ev);
        }
    }
}

/// Play the beat for `frames` frames (it stops after 8 bars).
fn play(
    project: &Project,
    frames: u64,
    period: u32,
    mut sampler: Option<&mut Engine>,
) -> (Pcm, Vec<Heard>) {
    let view = PlayView::compile(
        project,
        &[Launch {
            track: 0,
            slot: 0,
            origin: 0,
            start: 0,
        }],
    )
    .expect("the beat compiles");
    let mut t = Box::new(Transport::new(RATE, BPM).expect("valid transport"));
    t.command(0, Command::Play);
    t.command(frame_of(BARS * BAR), Command::Stop);
    let mut player = Box::new(Player::new());
    let mut block = Box::new(Block::new());
    let (mut left, mut right) = (vec![0.0f32; 8192], vec![0.0f32; 8192]);
    let mut pcm = Pcm {
        data: Vec::with_capacity(frames as usize * 4),
        peak: 0.0,
        clipped: 0,
    };
    let mut heard = Vec::new();
    while t.frame() < frames {
        let n = period.min((frames - t.frame()) as u32);
        assert!(t.process(n, &mut block));
        player.process(
            &view,
            &block,
            &mut Route {
                heard: &mut heard,
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
    }
    (pcm, heard)
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

/// `secs` seconds of `f(t)`, as stereo frames, fading out over the last
/// 20 ms so no sound ends with a click.
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

/// Synthesised, dirty sounds on pads 0 to 3: kick, snare, hat, clank.
fn sounds() -> SProject {
    let mut n = Noise(0x1357_9bdf);
    let mut phase = 0.0f32;
    let kick = make(0.7, |t| {
        let f = 45.0 + 120.0 * (-t * 26.0).exp();
        phase += TAU * f / RATE as f32;
        let body = phase.sin() * (-t * 6.0).exp();
        let click = n.next() * (-t * 400.0).exp() * 0.5;
        0.75 * drive(1.3 * body + click, 2.5)
    });
    let mut lp = Lp::new(1500.0);
    let snare = make(0.3, |t| {
        let body = (TAU * 190.0 * t).sin() * (-t * 22.0).exp();
        let x = n.next();
        0.65 * crush(
            drive(0.5 * body + 0.8 * (x - lp.run(x)) * (-t * 12.0).exp(), 4.0),
            8,
        )
    });
    let mut lp = Lp::new(6000.0);
    let hat = make(0.06, |t| {
        let x = n.next();
        0.35 * crush((x - lp.run(x)) * (-t * 70.0).exp(), 7)
    });
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
        0.6 * crush(drive(0.6 * s + 0.5 * click, 3.0), 6)
    });
    let mut p = SProject::new();
    for (pad, name, data) in [
        (KICK, "kick", kick),
        (SNARE, "snare", snare),
        (HAT, "hat", hat),
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
