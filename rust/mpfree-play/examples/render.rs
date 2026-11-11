//! WORKSTATION ONLY: play a demo beat through the transport, the player
//! and the real sampler engine, and write it to a WAV file, so playback can
//! be heard without the MPC. It never touches any device; it only writes
//! one WAV file.
//!
//! ```text
//! cargo run --release -p mpfree-play --example render -- [--period N] [--out FILE]
//! ```
//!
//! The beat: 84 BPM, heavy 1/16 swing, a four-bar loop played three times,
//! then a final hit that rings out. Saturated, bit-crushed drums (a one-bar
//! clip), crushed hats at double speed with their own swing, a distorted
//! two-bar bass line on gated pads (note-offs end the notes), a
//! seven-sixteenths glitch clip that drifts against the bar (polymeter),
//! dark chords held for a bar each and played through the arpeggiator
//! (`mpfree-arp`), whose 1/16s follow the transport's tempo and swing,
//! vinyl dust, and two live takes on an empty one-bar clip that quantises
//! to 1/16 (the clip recorder):
//!
//! - Pass 1 (bars 1 to 4) records. A distorted chord stab is played on the
//!   pads a little off the grid, one or two hits a bar. From the next bar
//!   on, each hit plays from the clip on the grid, so the stab riff builds
//!   up. The transport punches out at the loop's end.
//! - Pass 2: bars 5 and 6 play the stab riff. Bar 7 punches in with
//!   overdub off (replace recording): the stabs are erased as the playhead
//!   reaches them, so they stop, while a metal clank is played on the
//!   off-beats. Bar 8 and pass 3 play the clanks alone.
//!
//! At the end the tool undoes take 2 on a copy of the song and checks that
//! the stab riff is back (take undo).
//!
//! Every sound is synthesised here; no sample files are used. The tool
//! prints the take track bar by bar, from the notes the player and the pads
//! actually sent, and checks that the stabs stop in bar 7.
//!
//! It prints two hashes. `events` covers every note the player and the arp
//! sent (frame, track, note, velocity), every recorded note and every
//! erased note, and is the same on every computer and for every period
//! size; the tool checks that itself at 1, 128 and 4096 frames.
//! `audio` covers the WAV bytes; it repeats on one computer but can differ
//! between computers (the sounds use the system's maths library).

use std::f32::consts::TAU;

use mpfree_arp::{Engine as Arp, EventKind, Mode, Rate, Settings};
use mpfree_play::{
    ArpHost, Erased, Event, Launch, Out, PlayView, Player, Recorder, Sink, Taken, Takes, ALL,
};
use mpfree_sampler::dsp::Ahdsr;
use mpfree_sampler::engine::{Engine, EngineView, Event as SEvent};
use mpfree_sampler::model::{Layer, Pad, PlayMode, Project as SProject, Region, Source};
use mpfree_seq::{Clip, Note, Project, Quantise, Speed, Track};
use mpfree_transport::time::Swing;
use mpfree_transport::{Block, Command, State, Transport};

const RATE: u32 = 44_100;
const BPM: f64 = 84.0;
const BAR: i64 = 3840;
/// A 1/16 in ticks.
const S: i64 = 240;
/// Frames in a bar: exactly 126000 at 84 BPM.
const BAR_FRAMES: f64 = BAR as f64 * 60.0 * RATE as f64 / (BPM * 960.0);
/// The master gain: the mix peaks about 1.5 dB below full scale.
const MASTER: f32 = 0.6;
/// Passes of the four-bar loop before the final hit.
const PASSES: u64 = 3;
/// Seconds the final hit rings out.
const TAIL: f64 = 3.5;
/// The arp track's destination; every other track plays the sampler (0).
const ARP: u16 = 1;
/// The track number the arp's own notes are logged under.
const ARP_OUT: u16 = 100;
/// The track the live takes are recorded on.
const TAKE: usize = 5;
/// Pads: the chord stab, the metal clank, the vinyl dust, the crash.
const STAB: u8 = 20;
const CLANK: u8 = 21;
const DUST: u8 = 22;
const CRASH: u8 = 23;
/// The takes, played live on the pads, a little early or late: (pass of
/// the loop, bar, 1/16 step, milliseconds off the grid, velocity, pad).
/// The recorder puts each hit in a one-bar clip at the tick it was played
/// on; the clip quantises to 1/16 on playback, so every hit plays on the
/// grid from the next bar on. Take 1 (pass 0) builds a stab riff over four
/// bars; take 2 (pass 1, bar 2, overdub off) replaces it with clanks.
const TAKE_HITS: [(u64, i64, i64, f64, u8, u8); 9] = [
    (0, 0, 0, 12.0, 120, STAB),
    (0, 0, 3, -9.0, 100, STAB),
    (0, 1, 6, 7.0, 110, STAB),
    (0, 2, 10, -12.0, 105, STAB),
    (0, 3, 13, 5.0, 100, STAB),
    (1, 2, 2, 6.0, 115, CLANK),
    (1, 2, 6, -8.0, 95, CLANK),
    (1, 2, 10, 4.0, 120, CLANK),
    (1, 2, 14, -5.0, 100, CLANK),
];
/// Notes of the first take that the second one erases: all of them.
const ERASED: usize = 5;
/// The final hit, on the pads where the next pass would start: kick, stab
/// and crash. The transport has stopped, so it is heard, not recorded.
const END_HITS: [(u8, u8); 3] = [(0, 127), (STAB, 127), (CRASH, 120)];

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

    let frames = frame_in(PASSES, 0) + (TAIL * f64::from(RATE)) as u64;

    // The events must not depend on the period size.
    let hashes: Vec<(u32, u64, usize)> = [1u32, 128, 4096]
        .iter()
        .map(|&p| {
            let mut session = Session::new();
            play(frames, p, |_, block| session.period(block, None));
            session.check();
            (p, session.log.hash(), session.log.notes)
        })
        .collect();
    let same = hashes.windows(2).all(|w| w[0].1 == w[1].1);

    // The audio, at the chosen period size.
    let mut engine = Engine::new(RATE, 32);
    engine.set_view(EngineView::build(&sounds(), RATE));
    let mut session = Session::new();
    let mut left = vec![0.0f32; 8192];
    let mut right = vec![0.0f32; 8192];
    let mut pcm = Vec::with_capacity(frames as usize * 4);
    let mut peak = 0.0f32;
    let mut clipped = 0usize;
    play(frames, period, |n, block| {
        session.period(block, Some(&mut engine));
        let (l, r) = (&mut left[..n as usize], &mut right[..n as usize]);
        engine.render(l, r);
        for (a, b) in l.iter().zip(r.iter()) {
            for s in [a, b] {
                // A plain master gain with headroom: no saturation here.
                let v = s * MASTER;
                peak = peak.max(v.abs());
                clipped += usize::from(v.abs() > 1.0);
                let v = (v.clamp(-1.0, 1.0) * 32767.0).round() as i16;
                pcm.extend_from_slice(&v.to_le_bytes());
            }
        }
    });
    session.check();
    let log = &session.log;
    let wav = wav(&pcm);
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
        "notes played: {}  recorded: {}  erased: {}  dropped by the sampler: {}  peak: {:.1} dBFS  clipped samples: {clipped}",
        log.notes,
        log.recorded,
        log.erased,
        engine.stats().dropped_events,
        20.0 * peak.log10()
    );
    session.timeline.print();
    println!(
        "takes: {}  (undoing take 2 brings the stab riff back: checked)",
        session.takes.done().len()
    );
    println!("events {:016x}  (the same on every computer)", log.hash());
    println!("audio  {:016x}  (repeats on this computer)", fnv(&wav));
    for (p, h, n) in &hashes {
        println!("  events at period {p:>4}: {h:016x} ({n} notes)");
    }
    println!(
        "same events for every period size: {}",
        if same && log.hash() == hashes[0].1 {
            "yes"
        } else {
            "NO"
        }
    );
    if !same || log.hash() != hashes[0].1 {
        std::process::exit(1);
    }
    // Drive and crush inside a sound are deliberate; clipping here, at the
    // master, would not be, so the tool says so (it does not stop).
    if clipped > 0 {
        eprintln!("render: warning: {clipped} samples clipped at the master");
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

/// The bar (from 0) of an absolute frame, and the 1/16 step in it, with
/// the fraction. The tempo never changes, so frames map straight to bars.
fn bar_of(frame: u64) -> (usize, f64) {
    let b = frame as f64 / BAR_FRAMES;
    (b.floor() as usize, (b - b.floor()) * 16.0)
}

/// Run the transport for `frames` frames in periods of `period`, calling
/// `each` with every block. It records the first pass of the loop and
/// punches out at the loop's end, records bar 3 of the second pass with
/// overdub off (the transport's default), and stops on the last pass's
/// final bar line.
fn play(frames: u64, period: u32, mut each: impl FnMut(u32, &Block)) {
    let mut t = Transport::new(RATE, BPM).expect("valid transport");
    t.command(
        0,
        Command::SetSwing(Swing::new(240, 60).expect("valid swing")),
    );
    t.command(0, Command::SetLoop(Some((0, 4 * BAR))));
    t.command(0, Command::Record);
    t.command(frame_in(1, 0), Command::Record);
    t.command(frame_in(1, 2 * BAR), Command::Record);
    t.command(frame_in(1, 3 * BAR), Command::Record);
    // A command applies before a loop jump on the same frame, so this
    // stops the music right on the bar line, before the next pass.
    t.command(frame_in(PASSES, 0), Command::Stop);
    let mut block = Block::new();
    while t.frame() < frames {
        let n = period.min((frames - t.frame()) as u32);
        t.process(n, &mut block);
        each(n, &block);
    }
}

/// Everything that runs each period: the player, the arp, the live pads
/// and the recorder on the audio thread, then the control thread's part:
/// removing the erased notes from the clip and adding the recorded ones
/// (through `Takes`, which keeps them for undo), and compiling a new view.
struct Session {
    project: Project,
    launches: Vec<Launch>,
    view: PlayView,
    player: Player,
    arp: ArpHost,
    rec: Recorder,
    takes: Takes,
    live: Vec<(u64, Event)>,
    next_live: usize,
    log: EventLog,
    timeline: Timeline,
}

impl Session {
    fn new() -> Session {
        let (project, launches) = song();
        let view = compile(&project, &launches);
        Session {
            project,
            launches,
            view,
            player: Player::new(),
            arp: arp(),
            rec: Recorder::new(),
            takes: Takes::default(),
            live: live_hits(),
            next_live: 0,
            log: EventLog::default(),
            timeline: Timeline::default(),
        }
    }

    fn period(&mut self, block: &Block, mut sampler: Option<&mut Engine>) {
        let Session {
            project,
            launches,
            view,
            player,
            arp,
            rec,
            takes,
            live,
            next_live,
            log,
            timeline,
        } = self;
        timeline.block(block);
        player.process(
            view,
            block,
            &mut Route {
                arp,
                log,
                timeline,
                sampler: sampler.as_deref_mut(),
            },
        );
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
            if let Event::NoteOn { note, .. } = event {
                timeline.hits.push((frame, note, true));
            }
            if let Some(engine) = sampler.as_deref_mut() {
                to_sampler(
                    engine,
                    &Out {
                        offset,
                        frame,
                        track: TAKE as u16,
                        dest: 0,
                        event,
                    },
                );
            }
        }
        rec.process(view, block);
        if player.erased().is_empty() && rec.taken().is_empty() {
            return;
        }
        for e in player.erased() {
            assert!(takes.erase(project, e), "an erased note is in the clip");
            log.erase(e);
            timeline.erase(block.start_frame, e.note.tick);
        }
        for k in rec.taken() {
            takes.add(project, k).expect("a valid note");
            log.take(k);
        }
        *view = compile(project, launches);
    }

    fn check(&self) {
        assert_eq!(self.arp.dropped(), 0, "the arp dropped notes");
        assert_eq!(self.rec.dropped(), 0, "the recorder dropped notes");
        assert_eq!(self.player.stats().dropped, 0, "the player dropped notes");
        assert_eq!(
            self.log.recorded,
            TAKE_HITS.len(),
            "every live hit is recorded"
        );
        assert_eq!(self.log.erased, ERASED, "the second take erases the first");
        let take = &self.project.tracks[TAKE].slots[0]
            .as_ref()
            .expect("the take's clip")
            .notes;
        assert_eq!(
            take.len(),
            TAKE_HITS.len() - ERASED,
            "the clip holds the second take"
        );
        assert!(take.iter().all(|n| n.note == CLANK), "only clanks are left");
        // What was heard from the clip: stabs in bars 2 to 6 only, clanks
        // from bar 8 on, and every erased note in bar 7.
        for &(frame, pad, live) in &self.timeline.hits {
            let bar = bar_of(frame + 100).0 + 1;
            if !live && pad == STAB {
                assert!(
                    (2..=6).contains(&bar),
                    "a stab played from the clip in bar {bar}"
                );
            }
            if !live && pad == CLANK {
                assert!(bar >= 8, "a clank played from the clip in bar {bar}");
            }
        }
        assert!(
            self.timeline.erased.iter().all(|&(bar, _)| bar == 6),
            "erased in bar 7: {:?}",
            self.timeline.erased
        );
        // Undo take 2 on a copy: exactly the stab riff of take 1 is back.
        let shape: Vec<(u32, usize, usize)> = self
            .takes
            .done()
            .iter()
            .map(|t| (t.number, t.added.len(), t.erased.len()))
            .collect();
        assert_eq!(
            shape,
            vec![(1, ERASED, 0), (2, TAKE_HITS.len() - ERASED, ERASED)],
            "two takes"
        );
        let (mut project, mut takes) = (self.project.clone(), self.takes.clone());
        assert_eq!(takes.undo(&mut project), Some(2));
        let mut back = project.tracks[TAKE].slots[0]
            .as_ref()
            .expect("the take's clip")
            .notes
            .clone();
        let mut riff: Vec<Note> = self.takes.done()[0].added.iter().map(|a| a.2).collect();
        back.sort_by_key(|n| n.tick);
        riff.sort_by_key(|n| n.tick);
        assert_eq!(back, riff, "undo brings the stab riff back");
        assert!(back.iter().all(|n| n.note == STAB));
    }
}

/// The take track bar by bar, from the notes the player and the pads sent
/// and the notes replace recording erased, for the printout.
#[derive(Default)]
struct Timeline {
    /// Note-ons: (frame, pad, played live on the pads).
    hits: Vec<(u64, u8, bool)>,
    /// Erased notes: (bar, step), bars from 0.
    erased: Vec<(usize, usize)>,
    /// Bars in which the transport recorded.
    recording: Vec<bool>,
}

impl Timeline {
    fn block(&mut self, block: &Block) {
        for s in block.spans() {
            if s.state != State::Recording || s.len == 0 {
                continue;
            }
            for f in [s.frame, s.frame + u64::from(s.len) - 1] {
                let bar = bar_of(f).0;
                if self.recording.len() <= bar {
                    self.recording.resize(bar + 1, false);
                }
                self.recording[bar] = true;
            }
        }
    }

    /// An erased note, named by its clip tick, in a period starting at
    /// `start`: it plays on its 1/16 step at or after the period's start.
    fn erase(&mut self, start: u64, tick: i64) {
        let step = (tick as f64 / S as f64).round() as usize % 16;
        let (mut bar, at) = bar_of(start);
        if step as f64 + 0.5 < at {
            bar += 1;
        }
        self.erased.push((bar, step));
    }

    fn print(&self) {
        println!("take track, bar by bar, 1/16 steps (S stab, K clank: played by the clip; s, k: played on the pads; * final hit; x: erased, not played):");
        let bars = self
            .hits
            .iter()
            .map(|h| bar_of(h.0).0 + 1)
            .max()
            .unwrap_or(0);
        for b in 0..bars {
            let mut grid = ['.'; 16];
            for &(bar, step) in &self.erased {
                if bar == b {
                    grid[step] = 'x';
                }
            }
            // The clip's notes first, so the pads' show where both play.
            for live in [false, true] {
                for &(frame, pad, l) in &self.hits {
                    let (mut bar, at) = bar_of(frame);
                    let mut step = at.round() as usize;
                    if step == 16 {
                        bar += 1;
                        step = 0;
                    }
                    if l != live || bar != b {
                        continue;
                    }
                    let c = match pad {
                        STAB => 's',
                        CLANK => 'k',
                        _ => '*',
                    };
                    grid[step] = if live { c } else { c.to_ascii_uppercase() };
                }
            }
            let cells: Vec<String> = grid.chunks(4).map(|c| c.iter().collect()).collect();
            let rec = if self.recording.get(b) == Some(&true) {
                "recording"
            } else {
                ""
            };
            let erased = self.erased.iter().filter(|e| e.0 == b).count();
            let note = if erased > 0 {
                format!("{erased} old notes erased")
            } else {
                String::new()
            };
            println!(
                "  bar {:>2}  {:>4.1} s  {rec:<9}  {}  {note}",
                b + 1,
                b as f64 * BAR_FRAMES / f64::from(RATE),
                cells.join(" ")
            );
        }
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
        fnv(&[
            &self.player[..],
            &self.arp[..],
            &self.take[..],
            &self.erase[..],
        ]
        .concat())
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
            // The demo has no parameter locks (and its hash stays).
            Event::Lock { .. } | Event::Unlock { .. } => return,
        };
        if kind == 1 {
            self.notes += 1;
        }
        let bytes = if o.track == ARP_OUT {
            &mut self.arp
        } else {
            &mut self.player
        };
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
    timeline: &'a mut Timeline,
    sampler: Option<&'a mut Engine>,
}

impl Sink for Route<'_> {
    fn send(&mut self, o: Out) {
        self.log.add(&o);
        if let (true, Event::NoteOn { note, .. }) = (o.track == TAKE as u16, o.event) {
            self.timeline.hits.push((o.frame, note, false));
        }
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
        Event::NoteOn { note, velocity } => SEvent::NoteOn {
            pad: note,
            velocity,
        },
        Event::NoteOff { note } => SEvent::NoteOff { pad: note },
        Event::AllOff => SEvent::AllOff,
        Event::Lock { .. } | Event::Unlock { .. } => return,
    };
    engine.schedule(o.offset, ev);
}

/// The arpeggiator: up through the held chord in 1/16s, short notes.
fn arp() -> ArpHost {
    let mut engine = Arp::new(RATE, BPM);
    engine.set_settings(Settings {
        mode: Mode::Arp,
        rate: Rate::Sixteenth,
        gate: 45,
        ..Settings::default()
    });
    ArpHost::new(engine)
}

/// One of the arp's notes as an event for the sampler (and the log).
fn arp_out(block: &Block, offset: u32, kind: EventKind) -> Out {
    let event = match kind {
        EventKind::On { note, velocity } => Event::NoteOn { note, velocity },
        EventKind::Off { note } => Event::NoteOff { note },
    };
    Out {
        offset,
        frame: block.start_frame + u64::from(offset),
        track: ARP_OUT,
        dest: 0,
        event,
    }
}

fn notes(list: &[(i64, i64, u8, u8)]) -> Vec<Note> {
    list.iter()
        .map(|&(tick, len, note, velocity)| Note {
            tick,
            len,
            note,
            velocity,
            chance: 100,
            cond: mpfree_seq::Cond::Always,
            ratchet: 1,
        })
        .collect()
}

/// The demo beat: five tracks on the sampler's pads, one on the arp, an
/// empty clip for the live takes, and the vinyl dust.
fn song() -> (Project, Vec<Launch>) {
    let mut p = Project::new();
    let mut add = |name: &str, c: Clip| {
        let mut t = Track::new(name, if name == "arp" { ARP } else { 0 });
        t.slots[0] = Some(c);
        p.tracks.push(t);
    };

    // Pad 0 kick, pad 1 snare (ghost notes at low velocity).
    let mut drums = Clip::new("drums", BAR);
    drums.notes = notes(&[
        (0, S, 0, 127),
        (7 * S, S, 0, 80),
        (10 * S, S, 0, 118),
        (4 * S, S, 1, 127),
        (9 * S, S, 1, 28),
        (12 * S, S, 1, 124),
        (15 * S, S, 1, 40),
    ]);
    add("drums", drums);

    // Pads 2 closed hat, 3 open hat: a bar of clip at double speed, so
    // half a bar of 1/16s with a 1/32 roll at the end, and its own swing.
    let mut hats = Clip::new("hats", BAR);
    hats.speed = Speed::new(2, 1).expect("valid speed");
    hats.swing = Some(Swing::new(240, 62).expect("valid swing"));
    hats.notes = notes(&[
        (0, S, 2, 100),
        (2 * S, S, 2, 50),
        (4 * S, S, 2, 85),
        (6 * S, S, 3, 75),
        (8 * S, S, 2, 100),
        (10 * S, S, 2, 50),
        (12 * S, S, 2, 85),
        (13 * S, S / 2, 2, 40),
        (14 * S, S / 2, 2, 60),
        (15 * S, S / 2, 2, 45),
    ]);
    add("hats", hats);

    // Pads 4 to 8: a distorted bass on gated pads, so note lengths matter.
    let mut bass = Clip::new("bass", 2 * BAR);
    bass.notes = notes(&[
        (0, 5 * S, 4, 120),
        (6 * S, S, 4, 85),
        (8 * S, 3 * S, 5, 110),
        (12 * S, 2 * S, 6, 100),
        (14 * S, 2 * S, 7, 95),
        (16 * S, 5 * S, 4, 120),
        (22 * S, 2 * S, 8, 100),
        (24 * S, 4 * S, 7, 110),
        (28 * S, 2 * S, 6, 95),
        (30 * S, S, 5, 80),
    ]);
    add("bass", bass);

    // Pads 9 to 12: a 7/16 glitch that drifts against the bar, from bar 3.
    let mut glitch = Clip::new("glitch", 7 * S);
    glitch.notes = notes(&[
        (0, S, 9, 80),
        (2 * S, S, 11, 60),
        (3 * S, S, 10, 70),
        (5 * S, 2 * S, 12, 75),
    ]);
    add("glitch", glitch);

    // Pads 13 to 19, through the arp: D minor, B flat, G minor, A minor 7, a bar each.
    let mut chords = Clip::new("arp", 4 * BAR);
    let held = [[13u8, 14, 16], [14, 17, 19], [15, 17, 19], [16, 18, 19]];
    chords.notes = held
        .iter()
        .enumerate()
        .flat_map(|(bar, c)| {
            c.iter().map(move |&n| Note {
                tick: bar as i64 * BAR,
                len: BAR,
                note: n,
                velocity: 85,
                chance: 100,
                cond: mpfree_seq::Cond::Always,
                ratchet: 1,
            })
        })
        .collect();
    add("arp", chords);

    // The takes, recorded live into this empty bar.
    let mut take = Clip::new("take", BAR);
    take.quantise = Quantise::new(240, 100);
    add("take", take);

    // Vinyl dust: one long sound, started at each pass.
    let mut dust = Clip::new("dust", 4 * BAR);
    dust.notes = notes(&[(0, 4 * BAR, DUST, 100)]);
    add("dust", dust);

    let origin = [0, 0, 0, 2 * BAR, 0, 0, 0];
    let launches: Vec<Launch> = (0..origin.len())
        .map(|track| Launch {
            track,
            slot: 0,
            origin: origin[track],
            start: origin[track],
        })
        .collect();
    (p, launches)
}

/// The control thread's view of the song, with the take's track armed.
fn compile(project: &Project, launches: &[Launch]) -> PlayView {
    let mut view = PlayView::compile(project, launches).expect("valid song");
    view.set_armed(Some(TAKE));
    view
}

/// The takes' hits as pad presses and releases (60 ms later), and the
/// final hit, in frame order.
fn live_hits() -> Vec<(u64, Event)> {
    let mut hits = vec![];
    let press = |hits: &mut Vec<(u64, Event)>, on: u64, note: u8, velocity: u8| {
        hits.push((on, Event::NoteOn { note, velocity }));
        hits.push((on + u64::from(RATE) * 60 / 1000, Event::NoteOff { note }));
    };
    for &(pass, bar, step, ms, velocity, pad) in &TAKE_HITS {
        let on = frame_in(pass, bar * BAR + step * S) as i64
            + (ms * f64::from(RATE) / 1000.0).round() as i64;
        press(&mut hits, on.max(0) as u64, pad, velocity);
    }
    for &(pad, velocity) in &END_HITS {
        press(&mut hits, frame_in(PASSES, 0), pad, velocity);
    }
    hits.sort_by_key(|h| h.0);
    hits
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

/// Lower the sample rate by holding every `n`th frame (old-sampler grit).
fn hold(mut v: Vec<f32>, n: usize) -> Vec<f32> {
    for i in 0..v.len() / 2 {
        let src = i / n * n;
        v[2 * i] = v[2 * src];
        v[2 * i + 1] = v[2 * src + 1];
    }
    v
}

/// A sawtooth from `harmonics` sine partials.
fn saw(hz: f32, t: f32, harmonics: usize) -> f32 {
    (1..=harmonics)
        .map(|k| (TAU * hz * k as f32 * t).sin() / k as f32)
        .sum()
}

/// Synthesised sounds on pads 0 to 23: saturated, bit-crushed and dirty.
fn sounds() -> SProject {
    let mut n = Noise(0x1234_5678);
    let mut phase = 0.0f32;
    let kick = make(0.9, |t| {
        let f = 40.0 + 120.0 * (-t * 25.0).exp();
        phase += TAU * f / RATE as f32;
        let body = phase.sin() * (-t * 4.5).exp();
        let click = n.next() * (-t * 400.0).exp() * 0.5;
        0.75 * drive(1.3 * body + click, 2.5)
    });
    let mut lp = Lp::new(1500.0);
    let snare = hold(
        make(0.4, |t| {
            let body = (TAU * 180.0 * t).sin() * (-t * 22.0).exp()
                + 0.4 * (TAU * 320.0 * t).sin() * (-t * 35.0).exp();
            let x = n.next();
            let bright = x - lp.run(x);
            0.7 * crush(
                drive(0.55 * body + 0.8 * bright * (-t * 10.0).exp(), 4.0),
                8,
            )
        }),
        2,
    );
    let mut hat = |secs: f32, decay: f32, level: f32| {
        let mut lp = Lp::new(5000.0);
        hold(
            make(secs, |t| {
                let x = n.next();
                level * crush((x - lp.run(x)) * (-t * decay).exp(), 6)
            }),
            3,
        )
    };
    let closed = hat(0.08, 55.0, 0.4);
    let open = hat(0.45, 6.0, 0.3);
    let bass = |hz: f32| {
        let (mut lp, mut lp2) = (Lp::new(500.0), Lp::new(900.0));
        make(2.0, move |t| {
            let x = lp.run(0.5 * (saw(hz, t, 10) + saw(hz * 1.007, t, 10)));
            let sub = 0.5 * (TAU * hz * 0.5 * t).sin();
            0.38 * lp2.run(drive(1.8 * x + sub, 3.5)) * (-t * 0.6).exp()
        })
    };
    let glitch = |hz: f32| {
        make(0.22, |t| {
            0.22 * crush(
                (TAU * hz * t).sin() * (TAU * hz * 1.414 * t).sin() * (-t * 20.0).exp(),
                5,
            )
        })
    };
    let arp_tone = |hz: f32| {
        let mut lp = Lp::new(1400.0);
        make(1.0, move |t| {
            let square: f32 = (1..=9)
                .step_by(2)
                .map(|k| {
                    (TAU * hz * k as f32 * t).sin() / k as f32
                        + (TAU * hz * 1.004 * k as f32 * t).sin() / k as f32
                })
                .sum();
            0.16 * drive(lp.run(0.6 * square), 2.0) * (-t * 1.2).exp()
        })
    };
    // A D minor chord (D3 F3 A3 D4) of sawtooths through a closing filter,
    // driven, with the envelope after the drive so the tail dies away.
    let mut y = 0.0f32;
    let stab = make(0.8, |t| {
        let s: f32 = [146.83, 174.61, 220.0, 293.66]
            .iter()
            .map(|&hz| saw(hz, t, 8))
            .sum();
        let cutoff = 500.0 + 4000.0 * (-t * 8.0).exp();
        y += (1.0 - (-TAU * cutoff / RATE as f32).exp()) * (0.25 * s - y);
        let env = (t / 0.003).min(1.0) * (-t * 4.5).exp();
        0.95 * crush(drive(1.5 * y, 2.5) * env, 8)
    });
    // Metal: inharmonic partials and a click.
    let clank = make(0.4, |t| {
        let partials = [
            (540.0, 1.0, 9.0),
            (1213.0, 0.7, 12.0),
            (1771.0, 0.5, 16.0),
            (2390.0, 0.4, 20.0),
            (3311.0, 0.3, 26.0),
        ];
        let s: f32 = partials
            .iter()
            .map(|&(hz, a, d)| a * (TAU * hz * t).sin() * (-t * d).exp())
            .sum();
        let click = n.next() * (-t * 250.0).exp();
        0.8 * crush(drive(0.6 * s + 0.5 * click, 3.0), 6)
    });
    // Vinyl: sparse pops over a soft hiss, a little longer than four bars.
    let mut hiss = Lp::new(3000.0);
    let mut pop = 0.0f32;
    let dust = make(11.5, |_| {
        if n.next().abs() > 0.9996 {
            pop = 0.5 * n.next();
        }
        let out = pop + 0.04 * hiss.run(n.next());
        pop *= 0.55;
        0.35 * out
    });
    let mut lp = Lp::new(6000.0);
    let crash = make(3.2, |t| {
        let x = n.next();
        let ring = 0.3 * (TAU * 3150.0 * t).sin() * (TAU * 4470.0 * t).sin();
        0.32 * crush(
            (x - lp.run(x) + ring) * (-t * 1.5).exp() * (t / 0.002).min(1.0),
            7,
        )
    });

    let mut p = SProject::new();
    let mut put = |pad: usize, name: &str, data: Vec<f32>, gate: bool| {
        let s = Source::new(name, RATE, 2, data).expect("valid source");
        p.add_source(s.clone());
        let mut r = Region::whole(&s);
        r.mode = if gate {
            PlayMode::Gate
        } else {
            PlayMode::OneShot
        };
        let mut pad_def = Pad::new(name, Layer::new(r));
        if gate {
            pad_def.amp = Ahdsr {
                attack_ms: 2.0,
                hold_ms: 0.0,
                decay_ms: 120.0,
                sustain: 0.7,
                release_ms: 60.0,
            };
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
    // D minor pentatonic bass: D2, F2, G2, A2, C3.
    for (i, hz) in [73.42, 87.31, 98.0, 110.0, 130.81].iter().enumerate() {
        put(4 + i, "bass", bass(*hz), true);
    }
    // Glitch: D5, F5, A5, C6.
    for (i, hz) in [587.33, 698.46, 880.0, 1046.5].iter().enumerate() {
        put(9 + i, "glitch", glitch(*hz), false);
    }
    // Arp: D3, F3, G3, A3, B flat 3, C4, D4 on gated pads, so the arp's gate is heard.
    for (i, hz) in [146.83, 174.61, 196.0, 220.0, 233.08, 261.63, 293.66]
        .iter()
        .enumerate()
    {
        put(13 + i, "arp", arp_tone(*hz), true);
    }
    put(usize::from(STAB), "stab", stab, false);
    put(usize::from(CLANK), "clank", clank, false);
    put(usize::from(DUST), "dust", dust, false);
    put(usize::from(CRASH), "crash", crash, false);
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
