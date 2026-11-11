//! WORKSTATION ONLY: play a demo song through the transport, the player
//! and the real sampler engine, and write it to a WAV file, so playback can
//! be heard without the MPC. It never touches any device; it only writes
//! one WAV file.
//!
//! ```text
//! cargo run --release -p mpfree-play --example render -- [--period N] [--out FILE]
//! ```
//!
//! The song: 92 BPM, 1/16 swing at 56, a four-bar loop played twice.
//! Drums (a one-bar clip), hats at double speed with their own swing, a
//! two-bar bass line on gated pads (note-offs end the notes), a
//! seven-sixteenths pluck clip that drifts against the bar (polymeter),
//! a track of held chords played through the arpeggiator (`mpfree-arp`),
//! whose 1/16s follow the transport's tempo and swing, and two live takes.
//! The first pass is recorded, and a clave played a little off the grid on
//! the pads goes into an empty one-bar clip (the clip recorder). The clip
//! quantises on playback, so from the next bar on each hit plays on the
//! grid and the bar builds up; the transport punches out at the loop's end.
//! In the second pass the transport punches in again for bar 3, with
//! overdub off: the old clave notes are erased as the playhead reaches
//! them (replace recording) while a new pattern is played, and bar 4 plays
//! the new pattern alone.
//! Every sound is synthesised here; no sample files are used.
//!
//! It prints two hashes. `events` covers every note the player and the arp
//! sent (frame, track, note, velocity), every recorded note and every
//! erased note, and is the same on every computer and for every period
//! size; the tool checks that itself at 1, 128 and 4096 frames.
//! `audio` covers the WAV bytes; it repeats on one computer but can differ
//! between computers (the sounds use the system's maths library).

use std::f32::consts::TAU;

use mpfree_arp::{Engine as Arp, EventKind, Mode, Rate, Settings};
use mpfree_play::{ArpHost, Erased, Event, Launch, Out, PlayView, Player, Recorder, Sink, Taken, ALL};
use mpfree_sampler::dsp::Ahdsr;
use mpfree_sampler::engine::{Engine, EngineView, Event as SEvent};
use mpfree_sampler::model::{Layer, Pad, PlayMode, Project as SProject, Region, Source};
use mpfree_seq::{Clip, Note, Project, Quantise, Speed, Track};
use mpfree_transport::time::Swing;
use mpfree_transport::{Block, Command, Transport};

const RATE: u32 = 44_100;
const BPM: f64 = 92.0;
const BAR: i64 = 3840;
/// The arp track's destination; every other track plays the sampler (0).
const ARP: u16 = 1;
/// The track number the arp's own notes are logged under.
const ARP_OUT: u16 = 100;
/// The track the live take is recorded on.
const TAKE: usize = 5;
/// The takes: a clave played live on pad 20, a little early or late:
/// (pass of the loop, bar, 1/16 step, milliseconds off the grid, velocity).
/// The recorder puts each hit in a one-bar clip at the tick it was played
/// on; the clip quantises to 1/16 on playback, so every hit plays on the
/// grid from the next bar on. The first take (pass 0) builds the bar up
/// over four bars; the second (pass 1, bar 2) replaces it.
const TAKE_HITS: [(u64, i64, i64, f64, u8); 14] = [
    (0, 0, 0, 12.0, 110),
    (0, 0, 3, -9.0, 90),
    (0, 0, 6, 7.0, 95),
    (0, 1, 10, -14.0, 100),
    (0, 1, 12, 5.0, 85),
    (0, 2, 14, 10.0, 70),
    (0, 2, 15, -6.0, 60),
    (0, 3, 8, -11.0, 105),
    (0, 3, 9, 4.0, 50),
    (1, 2, 0, 8.0, 115),
    (1, 2, 4, -6.0, 80),
    (1, 2, 8, 5.0, 110),
    (1, 2, 12, -7.0, 85),
    (1, 2, 14, 3.0, 95),
];
/// Notes of the first take that the second one erases: all of them.
const ERASED: usize = 9;

fn main() {
    let mut period = 128u32;
    let mut out: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--period" => period = args.next().and_then(|v| v.parse().ok()).filter(|p| (1..=8192).contains(p)).unwrap_or_else(|| fail("--period takes 1 to 8192")),
            "--out" => out = Some(args.next().unwrap_or_else(|| fail("--out takes a file name"))),
            "-h" | "--help" => {
                println!("render [--period N] [--out FILE]   (default FILE: ~/mpfree-work/play-demo.wav)");
                return;
            }
            other => fail(&format!("unknown option {other}")),
        }
    }
    let out = out.unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        format!("{home}/mpfree-work/play-demo.wav")
    });

    let frames = (8.0 * 4.0 * 60.0 / BPM * f64::from(RATE)).ceil() as u64 + RATE as u64;

    // The events must not depend on the period size.
    let hashes: Vec<(u32, u64, usize)> = [1u32, 128, 4096].iter().map(|&p| {
        let mut session = Session::new();
        play(frames, p, |_, block| session.period(block, None));
        session.check();
        (p, session.log.hash(), session.log.notes)
    }).collect();
    let same = hashes.windows(2).all(|w| w[0].1 == w[1].1);

    // The audio, at the chosen period size.
    let mut engine = Engine::new(RATE, 32);
    engine.set_view(EngineView::build(&sounds(), RATE));
    let mut session = Session::new();
    let mut left = vec![0.0f32; 8192];
    let mut right = vec![0.0f32; 8192];
    let mut pcm = Vec::with_capacity(frames as usize * 4);
    play(frames, period, |n, block| {
        session.period(block, Some(&mut engine));
        let (l, r) = (&mut left[..n as usize], &mut right[..n as usize]);
        engine.render(l, r);
        for (a, b) in l.iter().zip(r.iter()) {
            for s in [a, b] {
                let v = ((s * 0.7).clamp(-1.0, 1.0) * 32767.0).round() as i16;
                pcm.extend_from_slice(&v.to_le_bytes());
            }
        }
    });
    session.check();
    let log = &session.log;
    let wav = wav(&pcm);
    if let Some(dir) = std::path::Path::new(&out).parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).unwrap_or_else(|e| fail(&format!("cannot make {}: {e}", dir.display())));
        }
    }
    std::fs::write(&out, &wav).unwrap_or_else(|e| fail(&format!("cannot write {out}: {e}")));

    println!("wrote {out}: {:.1} s, 44.1 kHz stereo 16-bit, period {period} frames", frames as f64 / f64::from(RATE));
    println!("notes played: {}  recorded: {}  erased: {}  dropped by the sampler: {}", log.notes, log.recorded, log.erased, engine.stats().dropped_events);
    println!("events {:016x}  (the same on every computer)", log.hash());
    println!("audio  {:016x}  (repeats on this computer)", fnv(&wav));
    for (p, h, n) in &hashes {
        println!("  events at period {p:>4}: {h:016x} ({n} notes)");
    }
    println!("same events for every period size: {}", if same && log.hash() == hashes[0].1 { "yes" } else { "NO" });
    if !same || log.hash() != hashes[0].1 {
        std::process::exit(1);
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("render: {msg}");
    std::process::exit(2);
}

/// The frame of song tick `tick` before the loop first wraps.
fn frame_of(tick: f64) -> u64 {
    (tick * 60.0 * f64::from(RATE) / (BPM * 960.0)).round() as u64
}

/// The frame of song tick `tick` in pass `pass` of the four-bar loop. A
/// pass starts on the frame where the last one's end tick falls, and a
/// tick falls on its rounded frame, so this is the frame it plays on.
fn frame_in(pass: u64, tick: i64) -> u64 {
    pass * frame_of((4 * BAR) as f64) + frame_of(tick as f64)
}

/// Run the transport for `frames` frames in periods of `period`, calling
/// `each` with every block. It records the first pass of the loop and
/// punches out at the loop's end, records bar 3 of the second pass with
/// overdub off (the transport's default), then plays on.
fn play(frames: u64, period: u32, mut each: impl FnMut(u32, &Block)) {
    let mut t = Transport::new(RATE, BPM).expect("valid transport");
    t.command(0, Command::SetSwing(Swing::new(240, 56).expect("valid swing")));
    t.command(0, Command::SetLoop(Some((0, 4 * BAR))));
    t.command(0, Command::Record);
    t.command(frame_in(1, 0), Command::Record);
    t.command(frame_in(1, 2 * BAR), Command::Record);
    t.command(frame_in(1, 3 * BAR), Command::Record);
    // Stop just before the end so every note ends.
    t.command(frames - RATE as u64 / 2, Command::Stop);
    let mut block = Block::new();
    while t.frame() < frames {
        let n = period.min((frames - t.frame()) as u32);
        t.process(n, &mut block);
        each(n, &block);
    }
}

/// Everything that runs each period: the player, the arp, the live pads
/// and the recorder on the audio thread, then the control thread's part:
/// removing the erased notes from the clip, adding the recorded ones and
/// compiling a new view.
struct Session {
    project: Project,
    launches: Vec<Launch>,
    view: PlayView,
    player: Player,
    arp: ArpHost,
    rec: Recorder,
    live: Vec<(u64, Event)>,
    next_live: usize,
    log: EventLog,
}

impl Session {
    fn new() -> Session {
        let (project, launches) = song();
        let view = compile(&project, &launches);
        Session { project, launches, view, player: Player::new(), arp: arp(), rec: Recorder::new(), live: live_hits(), next_live: 0, log: EventLog::default() }
    }

    fn period(&mut self, block: &Block, mut sampler: Option<&mut Engine>) {
        let Session { project, launches, view, player, arp, rec, live, next_live, log } = self;
        player.process(view, block, &mut Route { arp, log, sampler: sampler.as_deref_mut() });
        arp.process(block, &mut |offset, kind| {
            let o = arp_out(block, offset, kind);
            log.add(&o);
            if let Some(engine) = sampler.as_deref_mut() {
                to_sampler(engine, &o);
            }
        });
        // The live pads: heard at once, and recorded.
        let end = block.start_frame + u64::from(block.frames);
        while *next_live < live.len() && live[*next_live].0 < end {
            let (frame, event) = live[*next_live];
            *next_live += 1;
            let offset = (frame - block.start_frame) as u32;
            rec.input(offset, event);
            if let Some(engine) = sampler.as_deref_mut() {
                to_sampler(engine, &Out { offset, frame, track: TAKE as u16, dest: 0, event });
            }
        }
        rec.process(view, block);
        if player.erased().is_empty() && rec.taken().is_empty() {
            return;
        }
        for e in player.erased() {
            let clip = project.tracks[e.track].slots[e.slot].as_mut().expect("the take's clip");
            assert!(clip.remove_note(&e.note), "an erased note is in the clip");
            log.erase(e);
        }
        for k in rec.taken() {
            project.tracks[k.track].slots[k.slot].as_mut().expect("the take's clip").add_note(k.note).expect("a valid note");
            log.take(k);
        }
        *view = compile(project, launches);
    }

    fn check(&self) {
        assert_eq!(self.arp.dropped(), 0, "the arp dropped notes");
        assert_eq!(self.rec.dropped(), 0, "the recorder dropped notes");
        assert_eq!(self.player.stats().dropped, 0, "the player dropped notes");
        assert_eq!(self.log.recorded, TAKE_HITS.len(), "every live hit is recorded");
        assert_eq!(self.log.erased, ERASED, "the second take erases the first");
        let take = self.project.tracks[TAKE].slots[0].as_ref().expect("the take's clip");
        assert_eq!(take.notes.len(), TAKE_HITS.len() - ERASED, "the clip holds the second take");
    }
}

/// The player's events, the arp's, the recorded notes and the erased
/// notes, each in order. They are kept apart because the order between
/// them within one period depends on the period size.
#[derive(Default)]
struct EventLog {
    player: Vec<u8>,
    arp: Vec<u8>,
    take: Vec<u8>,
    erase: Vec<u8>,
    notes: usize,
    recorded: usize,
    erased: usize,
}

impl EventLog {
    fn hash(&self) -> u64 {
        fnv(&[&self.player[..], &self.arp[..], &self.take[..], &self.erase[..]].concat())
    }

    fn take(&mut self, k: &Taken) {
        self.recorded += 1;
        note_bytes(&mut self.take, k.track, k.slot, &k.note);
    }

    fn erase(&mut self, e: &Erased) {
        self.erased += 1;
        note_bytes(&mut self.erase, e.track, e.slot, &e.note);
    }

    fn add(&mut self, o: &Out) {
        let (kind, note, vel) = match o.event {
            Event::NoteOn { note, velocity } => (1u8, note, velocity),
            Event::NoteOff { note } => (0, note, 0),
            Event::AllOff => (2, 0, 0),
        };
        if kind == 1 {
            self.notes += 1;
        }
        let bytes = if o.track == ARP_OUT { &mut self.arp } else { &mut self.player };
        bytes.extend_from_slice(&o.frame.to_le_bytes());
        bytes.extend_from_slice(&o.track.to_le_bytes());
        bytes.extend_from_slice(&[kind, note, vel]);
    }
}

/// Logs every event the player sends. The arp track's notes (and a
/// panic's `AllOff`) go to the arp; the rest go to the sampler, if there
/// is one, where a note number is a pad.
struct Route<'a> {
    arp: &'a mut ArpHost,
    log: &'a mut EventLog,
    sampler: Option<&'a mut Engine>,
}

impl Sink for Route<'_> {
    fn send(&mut self, o: Out) {
        self.log.add(&o);
        if o.dest == ARP || o.dest == ALL {
            self.arp.input(o.offset, o.event);
        }
        if o.dest != ARP {
            if let Some(engine) = self.sampler.as_deref_mut() {
                to_sampler(engine, &o);
            }
        }
    }
}

fn note_bytes(bytes: &mut Vec<u8>, track: usize, slot: usize, n: &Note) {
    for v in [track as i64, slot as i64, n.tick, n.len] {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes.extend_from_slice(&[n.note, n.velocity]);
}

fn to_sampler(engine: &mut Engine, o: &Out) {
    let ev = match o.event {
        Event::NoteOn { note, velocity } => SEvent::NoteOn { pad: note, velocity },
        Event::NoteOff { note } => SEvent::NoteOff { pad: note },
        Event::AllOff => SEvent::AllOff,
    };
    engine.schedule(o.offset, ev);
}

/// The arpeggiator: up through the held chord in 1/16s, short notes.
fn arp() -> ArpHost {
    let mut engine = Arp::new(RATE, BPM);
    engine.set_settings(Settings { mode: Mode::Arp, rate: Rate::Sixteenth, gate: 45, ..Settings::default() });
    ArpHost::new(engine)
}

/// One of the arp's notes as an event for the sampler (and the log).
fn arp_out(block: &Block, offset: u32, kind: EventKind) -> Out {
    let event = match kind {
        EventKind::On { note, velocity } => Event::NoteOn { note, velocity },
        EventKind::Off { note } => Event::NoteOff { note },
    };
    Out { offset, frame: block.start_frame + u64::from(offset), track: ARP_OUT, dest: 0, event }
}

fn notes(list: &[(i64, i64, u8, u8)]) -> Vec<Note> {
    list.iter().map(|&(tick, len, note, velocity)| Note { tick, len, note, velocity }).collect()
}

/// The demo song: four tracks on the sampler's pads, one on the arp, and
/// an empty clip for the live take.
fn song() -> (Project, Vec<Launch>) {
    const S: i64 = 240; // a 1/16
    let mut p = Project::new();
    let mut add = |name: &str, c: Clip| {
        let mut t = Track::new(name, if name == "arp" { ARP } else { 0 });
        t.slots[0] = Some(c);
        p.tracks.push(t);
    };

    // Pads 0 kick, 1 snare.
    let mut drums = Clip::new("drums", BAR);
    drums.notes = notes(&[(0, S, 0, 120), (6 * S, S, 0, 90), (10 * S, S, 0, 110), (4 * S, S, 1, 115), (12 * S, S, 1, 118), (15 * S, S, 1, 50)]);
    add("drums", drums);

    // Pads 2 closed hat, 3 open hat: half a bar of 1/16s at double speed, so 1/32s.
    let mut hats = Clip::new("hats", BAR / 2);
    hats.speed = Speed::new(2, 1).expect("valid speed");
    hats.swing = Some(Swing::new(240, 62).expect("valid swing"));
    hats.notes = (0..8).map(|i| Note { tick: i * S, len: S / 2, note: if i == 6 { 3 } else { 2 }, velocity: [100, 50, 75, 45][i as usize % 4] }).collect();
    add("hats", hats);

    // Pads 4 to 8: bass on gated pads, so note lengths matter.
    let mut bass = Clip::new("bass", 2 * BAR);
    bass.notes = notes(&[(0, 3 * S, 4, 110), (3 * S, S, 4, 80), (6 * S, 2 * S, 6, 100), (10 * S, 4 * S, 5, 105), (16 * S, 3 * S, 4, 110), (20 * S, 2 * S, 7, 95), (24 * S, 6 * S, 8, 100), (30 * S, S, 7, 70)]);
    add("bass", bass);

    // Pads 9 to 12: a 7/16 pluck that drifts against the bar.
    let mut pluck = Clip::new("pluck", 7 * S);
    pluck.notes = notes(&[(0, S, 9, 90), (2 * S, S, 11, 70), (3 * S, S, 10, 80), (5 * S, 2 * S, 12, 85)]);
    add("pluck", pluck);

    // Pads 13 to 19, through the arp: a chord held for each bar.
    let mut chords = Clip::new("arp", 4 * BAR);
    let held = [[13u8, 14, 16], [14, 16, 17], [15, 17, 18], [16, 17, 19]];
    chords.notes = held.iter().enumerate().flat_map(|(bar, c)| c.iter().map(move |&n| Note { tick: bar as i64 * BAR, len: BAR, note: n, velocity: 85 })).collect();
    add("arp", chords);

    // Pad 20: the take, recorded live into this empty bar.
    let mut take = Clip::new("take", BAR);
    take.quantise = Quantise::new(240, 100);
    add("take", take);

    let origin = [0, 0, 0, 2 * BAR, 0, 0];
    let launches: Vec<Launch> = (0..6).map(|track| Launch { track, slot: 0, origin: origin[track] }).collect();
    (p, launches)
}

/// The control thread's view of the song, with the take's track armed.
fn compile(project: &Project, launches: &[Launch]) -> PlayView {
    let mut view = PlayView::compile(project, launches).expect("valid song");
    view.set_armed(Some(TAKE));
    view
}

/// The take's hits as pad presses and releases (60 ms later), in frame order.
fn live_hits() -> Vec<(u64, Event)> {
    let mut hits = vec![];
    for &(pass, bar, step, ms, velocity) in &TAKE_HITS {
        let on = frame_in(pass, bar * BAR + step * 240) as i64 + (ms * f64::from(RATE) / 1000.0).round() as i64;
        let on = on.max(0) as u64;
        hits.push((on, Event::NoteOn { note: 20, velocity }));
        hits.push((on + u64::from(RATE) * 60 / 1000, Event::NoteOff { note: 20 }));
    }
    hits.sort_by_key(|h| h.0);
    hits
}

/// Synthesised sounds on pads 0 to 20.
fn sounds() -> SProject {
    let rate = RATE as f32;
    let mut noise_state = 0x1234_5678u32;
    let mut noise = move || {
        noise_state ^= noise_state << 13;
        noise_state ^= noise_state >> 17;
        noise_state ^= noise_state << 5;
        noise_state as f32 / u32::MAX as f32 * 2.0 - 1.0
    };
    let make = |secs: f32, f: &mut dyn FnMut(f32) -> f32| -> Vec<f32> {
        let n = (secs * rate) as usize;
        (0..n).flat_map(|i| {
            let s = f(i as f32 / rate);
            [s, s]
        }).collect()
    };
    let mut phase = 0.0f32;
    let kick = make(0.4, &mut |t| {
        let f = 45.0 + 90.0 * (-t * 30.0).exp();
        phase += TAU * f / rate;
        phase.sin() * (-t * 7.0).exp()
    });
    let snare = make(0.25, &mut |t| (0.5 * noise() + 0.5 * (TAU * 190.0 * t).sin()) * (-t * 18.0).exp());
    let mut last = 0.0f32;
    let mut hat = |len: f32, decay: f32| make(len, &mut |t| {
        let n = noise();
        let hp = n - last;
        last = n;
        0.5 * hp * (-t * decay).exp()
    });
    let closed = hat(0.06, 70.0);
    let open = hat(0.35, 9.0);
    let tone = |hz: f32, secs: f32, bright: f32| -> Vec<f32> {
        let n = (secs * rate) as usize;
        (0..n).flat_map(|i| {
            let t = i as f32 / rate;
            let s = (TAU * hz * t).sin() + bright * (TAU * 2.0 * hz * t).sin() * 0.5 + bright * (TAU * 3.0 * hz * t).sin() * 0.25;
            let s = s * 0.45 * (-t * 1.5).exp();
            [s, s]
        }).collect()
    };
    let mut p = SProject::new();
    let mut put = |pad: usize, name: &str, data: Vec<f32>, gate: bool| {
        let s = Source::new(name, RATE, 2, data).expect("valid source");
        p.add_source(s.clone());
        let mut r = Region::whole(&s);
        r.mode = if gate { PlayMode::Gate } else { PlayMode::OneShot };
        let mut pad_def = Pad::new(name, Layer::new(r));
        if gate {
            pad_def.amp = Ahdsr { attack_ms: 2.0, hold_ms: 0.0, decay_ms: 120.0, sustain: 0.7, release_ms: 60.0 };
        }
        if pad == 2 || pad == 3 {
            pad_def.mute_group = Some(1); // the closed hat stops the open one
        }
        p.set_pad(pad, Some(pad_def), None).expect("valid pad");
    };
    put(0, "kick", kick, false);
    put(1, "snare", snare, false);
    put(2, "closed hat", closed, false);
    put(3, "open hat", open, false);
    // A minor pentatonic bass: A1, C2, D2, E2, G2.
    for (i, hz) in [55.0, 65.41, 73.42, 82.41, 98.0].iter().enumerate() {
        put(4 + i, "bass", tone(*hz, 2.0, 0.8), true);
    }
    // Pluck: A4, C5, D5, E5.
    for (i, hz) in [440.0, 523.25, 587.33, 659.26].iter().enumerate() {
        put(9 + i, "pluck", tone(*hz, 0.5, 0.3).iter().enumerate().map(|(k, s)| s * 0.5 * (-(k as f32) / rate * 6.0).exp()).collect(), false);
    }
    // Arp: A3, C4, D4, E4, G4, A4, C5 on gated pads, so the arp's gate is heard.
    for (i, hz) in [220.0, 261.63, 293.66, 329.63, 392.0, 440.0, 523.25].iter().enumerate() {
        put(13 + i, "arp", tone(*hz, 1.0, 1.0).iter().map(|s| s * 0.45).collect(), true);
    }
    // Clave: a short, bright click.
    put(20, "clave", make(0.12, &mut |t| 0.3 * (TAU * 2500.0 * t).sin() * (-t * 45.0).exp()), false);
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
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}
