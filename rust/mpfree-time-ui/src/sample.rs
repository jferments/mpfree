//! Sample screens with a real transport, player, recorder, capture and take
//! undo behind them, for the workstation preview and the tests. Track and
//! clip names are made up (no Akai content).
//!
//! The screens sit in the pad player as it would be with the time engine
//! wired in: a Clips tab in its header, the transport bar under the header
//! on the Play and Clips pages, and a sample arp chip on the bar where the
//! arp's chip goes.

use crate::view::{
    bars_label, clips_after, with_transport, TimeAction, TimePage, TimeState, TransportInfo,
    CLIPS_HINT, CLIPS_PAGE, SONG_HINT, TRANSPORT,
};
use crate::widgets::TransportBar;
use mpfree_capture::{keep, Capture, Kept, Now};
use mpfree_play::{Cue, Event, Launch, Out, PlayView, Player, Recorder, Sink, Takes};
use mpfree_seq::arrange::{scene_length, scenes_to_song};
use mpfree_seq::{Clip, Note, Project, TouchPoint, Track};
use mpfree_transport::time::Tick;
use mpfree_transport::{Block, Command, Transport};
use mpfree_ui::widgets::{Chip, Fader, Header, Label, ListItem, ListView, PadGrid, StatList};
use mpfree_ui::{Align, Color, Fonts, Theme, Tone, Ui};

/// The screen as the pad player sees it (the MPC One's panel, turned).
pub const SCREEN: (i32, i32) = (1280, 800);
/// The pad player's Play page (`mpfree-app`'s `PLAY_LAYOUT`), before the
/// bar goes in.
pub const APP_PLAY_LAYOUT: &str =
    "col(header:60, row(pads:*, col(kits:*, level:96, timing:232):460):*, hint:26)";
const PLAY_HINT: &str =
    "Wheel or tap: kit   ·   Q-Link 1 or slider: level   ·   NOTE REPEAT: repeat   ·   MENU: options   ·   STOP: silence";

pub const RATE: u32 = 44_100;
pub const PERIOD: u32 = 128;
pub const BPM: f64 = 92.0;
const BAR: Tick = 3840;
const BEAT: Tick = 960;
const SIXTEENTH: Tick = 240;
/// The tracks of the sample project.
pub const DRUMS: usize = 0;
pub const CHORDS: usize = 4;
pub const STAB: usize = 5;

/// One sample screen: its file name, what it shows, and the UI showing it.
pub struct Sample {
    pub name: &'static str,
    pub about: &'static str,
    pub ui: Ui,
    pub state: TimeState,
}

struct Mute;
impl Sink for Mute {
    fn send(&mut self, _out: Out) {}
}

/// A running engine: the transport, the player, the recorder and the
/// capture on the "audio thread", and the project, the launches and the
/// takes on the "control thread", as the app will run them.
pub struct Session {
    // Boxed: the audio thread's fixed tables are large for a test thread's
    // stack.
    pub transport: Box<Transport>,
    player: Box<Player>,
    recorder: Box<Recorder>,
    pub capture: Box<Capture>,
    pub project: Project,
    pub launches: Vec<Launch>,
    /// Launches and stops waiting for the next bar line.
    pub cues: Vec<Cue>,
    pub armed: Option<usize>,
    pub takes: Takes,
    view: PlayView,
    block: Box<Block>,
    /// Pad notes still to play into the recorder, at song ticks.
    live: Vec<(Tick, Event)>,
}

impl Session {
    pub fn new(
        mut project: Project,
        launches: Vec<Launch>,
        armed: Option<usize>,
    ) -> Result<Session, String> {
        // Every clip has its lasting id, as the app's opened projects do.
        project.give_clip_ids();
        let transport =
            Box::new(Transport::new(RATE, BPM).map_err(|e| format!("transport: {e:?}"))?);
        let mut s = Session {
            transport,
            player: Box::new(Player::new()),
            recorder: Box::new(Recorder::new()),
            capture: Box::new(Capture::new()),
            project,
            launches,
            cues: Vec::new(),
            armed,
            takes: Takes::default(),
            view: PlayView::empty(),
            block: Box::new(Block::new()),
            live: Vec::new(),
        };
        s.compile()?;
        Ok(s)
    }

    fn compile(&mut self) -> Result<(), String> {
        // An undone capture takes its clip, and so its launch, away.
        let project = &self.project;
        self.launches
            .retain(|l| project.clip(l.track, l.slot).is_some());
        self.cues.retain(|c| {
            c.slot
                .is_none_or(|slot| project.clip(c.track, slot).is_some())
        });
        let mut v = PlayView::compile_cued(&self.project, &self.launches, &self.cues)
            .map_err(|e| format!("compile: {e:?}"))?;
        v.set_armed(self.armed);
        self.view = v;
        Ok(())
    }

    /// Queue `cmd` for the frame where song tick `tick` falls (now, if
    /// stopped or already past).
    pub fn command_at(&mut self, tick: Tick, cmd: Command) -> Result<(), String> {
        let now = self.transport.frame();
        let at = match self.block.spans().last() {
            Some(s) if s.state.rolling() => s.frame_of(tick as f64).max(now),
            _ => now,
        };
        self.transport
            .command(at, cmd)
            .then_some(())
            .ok_or_else(|| "command queue full".to_string())
    }

    /// Play `note` on the pads from `tick` for `len` ticks.
    pub fn pad(&mut self, tick: Tick, len: Tick, note: u8, velocity: u8) {
        self.live.push((tick, Event::NoteOn { note, velocity }));
        self.live.push((tick + len, Event::NoteOff { note }));
    }

    /// The control thread's side of a user action on the launches, or
    /// Capture.
    pub fn act(&mut self, a: TimeAction) -> Result<(), String> {
        if a == TimeAction::Capture {
            return self.keep().map(|_| ());
        }
        (self.launches, self.cues) = clips_after(
            &self.project,
            &self.launches,
            &self.cues,
            &a,
            &TransportInfo::of(&self.transport),
        );
        self.compile()
    }

    /// CAPTURE: keep the last phrase in the armed track (track 1 if none),
    /// into its playing clip or as a new clip that plays on in its place.
    pub fn keep(&mut self) -> Result<Kept, String> {
        let phrase = self
            .capture
            .pending()
            .and(self.capture.phrase())
            .ok_or("nothing new to capture")?;
        let t = &self.transport;
        let now = Now {
            position: t.position(),
            rolling: t.state().rolling(),
            meter: t.meter(),
            loop_range: t.loop_range(),
        };
        let name = format!("Capture {}", self.takes.captures() + 1);
        let track = self.armed.unwrap_or(0);
        let k = keep(
            &phrase,
            &mut self.project,
            &self.launches,
            &mut self.takes,
            track,
            now,
            &name,
            // This session's pads are pads alone.
            mpfree_seq::Holds::Hits,
        )
        .map_err(|e| format!("capture: {e:?}"))?;
        if let Some(l) = k.launch {
            self.launches.retain(|x| x.track != l.track);
            self.cues.retain(|x| x.track != l.track);
            self.launches.push(l);
        }
        self.capture.mark_kept(phrase.id);
        self.compile()?;
        Ok(k)
    }

    pub fn undo(&mut self) -> Result<Option<u32>, String> {
        let n = self.takes.undo(&mut self.project);
        // An undone capture can be kept again.
        if n.is_some() && self.takes.undone().last().is_some_and(|t| t.captured) {
            self.capture.forget_kept();
        }
        self.compile()?;
        Ok(n)
    }

    /// Run `n` periods.
    pub fn run_periods(&mut self, n: usize) -> Result<(), String> {
        (0..n).try_for_each(|_| self.period())
    }

    /// Run periods until the song position reaches `tick`.
    pub fn run_to(&mut self, tick: Tick) -> Result<(), String> {
        for _ in 0..1_000_000 {
            if self.transport.position() >= tick {
                return Ok(());
            }
            self.period()?;
        }
        Err(format!("the song never reached tick {tick}"))
    }

    fn period(&mut self) -> Result<(), String> {
        if !self.transport.process(PERIOD, &mut self.block) {
            return Err("bad period".into());
        }
        // Pad notes that fall in this period, as the pads would bring them.
        let block = &self.block;
        let mut i = 0;
        while i < self.live.len() {
            let (tick, ev) = self.live[i];
            match block.spans().iter().find_map(|s| s.offset_of(tick as f64)) {
                Some(off) => {
                    self.recorder.input(off, ev);
                    self.capture.input(off, ev);
                    self.live.remove(i);
                }
                None => i += 1,
            }
        }
        self.capture.process(&self.block);
        self.player.process(&self.view, &self.block, &mut Mute);
        self.recorder.process(&self.view, &self.block);
        let mut changed = false;
        for st in self.player.started().to_vec() {
            st.apply(&mut self.launches, &mut self.cues);
            changed = true;
        }
        for e in self.player.erased().to_vec() {
            changed |= self.takes.erase(&mut self.project, &e);
        }
        for e in self.player.erased_locks().to_vec() {
            changed |= self.takes.erase_lock(&mut self.project, &e);
        }
        for k in self.recorder.taken().to_vec() {
            self.takes
                .add(&mut self.project, &k)
                .map_err(|e| format!("recorded note: {e:?}"))?;
            changed = true;
        }
        for k in self.recorder.taken_locks().to_vec() {
            self.takes
                .add_lock(&mut self.project, &k)
                .map_err(|e| format!("recorded lock: {e:?}"))?;
            changed = true;
        }
        if changed {
            self.compile()?;
        }
        Ok(())
    }

    pub fn state(&self) -> TimeState {
        let mut st = TimeState::from_engine(
            TransportInfo::of(&self.transport),
            &self.project,
            &self.launches,
            self.armed,
            &self.takes,
        );
        st.capture = self.capture.pending().map(|p| bars_label(p.bars));
        st.cues = self.cues.clone();
        st
    }
}

fn clip(name: &str, length: Tick, notes: &[(Tick, u8, u8)]) -> Clip {
    let mut c = Clip::new(name, length);
    for &(tick, note, velocity) in notes {
        let len = SIXTEENTH.min(length - tick);
        c.add_note(Note {
            tick,
            len,
            note,
            velocity,
            chance: 100,
            cond: mpfree_seq::Cond::Always,
            ratchet: 1,
            touch: mpfree_seq::Touch::NONE,
            on: true,
            ramp: 0,
            join: mpfree_seq::Join::Attack,
            group: 0,
        })
        .expect("sample note in range");
    }
    c
}

/// A note every `step` ticks through `length`.
fn every(step: Tick, length: Tick, note: u8, velocity: u8) -> Vec<(Tick, u8, u8)> {
    (0..length / step)
        .map(|i| (i * step, note, velocity))
        .collect()
}

fn beat(bars: Tick) -> Vec<(Tick, u8, u8)> {
    (0..bars)
        .flat_map(|b| {
            [
                (b * BAR, 0, 120),
                (b * BAR + BEAT, 1, 110),
                (b * BAR + 1680, 0, 100),
                (b * BAR + 3 * BEAT, 1, 115),
            ]
        })
        .collect()
}

/// The stab riff: five notes in one bar (what take 1 replaces).
pub const STAB_RIFF: [(Tick, u8, u8); 5] = [
    (0, 6, 120),
    (720, 6, 100),
    (1440, 6, 110),
    (2400, 6, 105),
    (3120, 6, 100),
];

/// The sample project: eight tracks with clips in some of their slots.
pub fn project() -> Project {
    let mut p = Project::new();
    let mut track = |name: &str, clips: Vec<(usize, Clip)>| {
        let mut t = Track::new(name, p.tracks.len() as u16);
        for (slot, c) in clips {
            t.slots[slot] = Some(c);
        }
        p.tracks.push(t);
    };
    track(
        "Drums",
        vec![
            (0, clip("Boom bap", 2 * BAR, &beat(2))),
            (1, clip("Half time", 4 * BAR, &beat(4)[..8])),
            (2, clip("Fill", BAR, &every(SIXTEENTH, BAR, 1, 90))),
            (4, clip("Break", 2 * BAR, &every(480, 2 * BAR, 0, 100))),
        ],
    );
    track(
        "Hats",
        vec![
            (0, clip("Hats 16", BAR, &every(SIXTEENTH, BAR, 2, 80))),
            (1, clip("Shaker", 2 * BAR, &every(480, 2 * BAR, 3, 70))),
        ],
    );
    track(
        "Bass",
        vec![
            (0, clip("Sub line", 2 * BAR, &every(1280, 2 * BAR, 4, 100))),
            (1, clip("Bass B", 4 * BAR, &every(1280, 4 * BAR, 4, 100))),
            (3, clip("Drop", 2 * BAR, &every(BAR, 2 * BAR, 4, 127))),
        ],
    );
    track(
        "Glitch",
        vec![
            (
                0,
                clip(
                    "Glitch",
                    7 * SIXTEENTH,
                    &every(3 * SIXTEENTH, 7 * SIXTEENTH, 5, 90),
                ),
            ),
            (2, clip("Stutter", BAR, &every(SIXTEENTH, BAR, 5, 70))),
        ],
    );
    track(
        "Chords",
        vec![
            (
                0,
                clip("Arp chords", 4 * BAR, &every(2 * BEAT, 4 * BAR, 7, 90)),
            ),
            (2, clip("Pad", 8 * BAR, &every(2 * BAR, 8 * BAR, 8, 80))),
        ],
    );
    track("Stab", vec![(0, clip("Stab riff", BAR, &STAB_RIFF))]);
    track("Dust", vec![(0, clip("Dust", 4 * BAR, &[(0, 9, 60)]))]);
    track(
        "FX",
        vec![
            (0, clip("Riser", 4 * BAR, &[(0, 10, 90)])),
            (3, clip("Wash", 2 * BAR, &[(0, 11, 90)])),
        ],
    );
    p
}

/// The sample project grown to 24 tracks (Jay, 14 Nov: up to 128, like
/// Akai's MPC): the eight above, then sixteen more with a clip or two.
pub fn big_project() -> Project {
    let mut p = project();
    // (track name, [(slot, clip name, length)])
    type Clips = &'static [(usize, &'static str, Tick)];
    let more: [(&str, Clips); 16] = [
        ("Keys", &[(0, "Rhodes", 4 * BAR), (1, "Rhodes B", 4 * BAR)]),
        ("Lead", &[(1, "Hook", 2 * BAR)]),
        ("Vox chop", &[(0, "Chop A", BAR), (2, "Chop B", BAR)]),
        ("Perc", &[(0, "Congas", 2 * BAR)]),
        ("Toms", &[(2, "Tom fill", BAR)]),
        ("Clap", &[(0, "Clap 2+4", BAR), (1, "Clap roll", BAR)]),
        ("Ride", &[(1, "Ride", 2 * BAR)]),
        ("Strings", &[(0, "Strings", 8 * BAR)]),
        ("Brass", &[(3, "Stabs", 2 * BAR)]),
        (
            "Pluck",
            &[(0, "Pluck arp", 2 * BAR), (2, "Pluck 2", 2 * BAR)],
        ),
        ("Sub 2", &[(1, "Sub B", 4 * BAR)]),
        ("Choir", &[(2, "Aahs", 8 * BAR)]),
        ("Noise", &[(0, "Sweep", 4 * BAR)]),
        ("Reverse", &[(3, "Rev cymbal", BAR)]),
        ("Bell", &[(0, "Bells", 2 * BAR)]),
        ("Organ", &[(1, "Organ", 4 * BAR)]),
    ];
    for (i, (name, clips)) in more.iter().enumerate() {
        let mut t = Track::new(name, p.tracks.len() as u16);
        for &(slot, clip_name, length) in *clips {
            let note = (i % 16) as u8;
            t.slots[slot] = Some(clip(clip_name, length, &every(BEAT, length, note, 90)));
        }
        p.tracks.push(t);
    }
    p
}

/// The 24-track project playing from bar 1 at 3.1, every track's slot 1
/// that has a clip launched, and `armed` armed.
pub fn big_playing(armed: usize) -> Result<Session, String> {
    let p = big_project();
    let launches = (0..p.tracks.len())
        .filter(|&t| p.tracks[t].slots[0].is_some())
        .map(|track| Launch {
            track,
            slot: 0,
            origin: 0,
            start: 0,
        })
        .collect();
    let mut s = Session::new(p, launches, Some(armed))?;
    s.command_at(0, Command::Play)?;
    s.run_to(2 * BAR + SIXTEENTH)?;
    Ok(s)
}

/// A full Arrange page for the announcement screens (Jay, 19 Nov
/// 02:45Z: more clips, in more states, with some slots left empty): seven
/// note tracks and a Loop track, most rows filled, a few holes and the
/// last rows free.
pub fn busy_project() -> Project {
    use mpfree_seq::{LoopInput, TrackKind};
    let mut p = Project::new();
    type Clips<'a> = &'a [(usize, &'a str, Tick, u8, Tick)];
    // (track, [(slot, name, length, pad, a note every)])
    let tracks: [(&str, Clips); 7] = [
        (
            "Drums",
            &[
                (0, "Intro beat", 2 * BAR, 0, BEAT),
                (2, "Half time", 4 * BAR, 0, 2 * BEAT),
                (3, "Fill", BAR, 1, SIXTEENTH),
                (4, "Break", 2 * BAR, 0, 480),
                (6, "Outro", 2 * BAR, 0, BEAT),
            ],
        ),
        (
            "Hats",
            &[
                (0, "Hats 16", BAR, 2, SIXTEENTH),
                (1, "Shaker", 2 * BAR, 3, 480),
                (2, "Open hats", BAR, 2, 480),
                (4, "Ride", 2 * BAR, 3, BEAT),
            ],
        ),
        (
            "Bass",
            &[
                (0, "Sub line", 2 * BAR, 4, 1280),
                (1, "Bass B", 4 * BAR, 4, 1280),
                (2, "Walk", 2 * BAR, 4, BEAT),
                (3, "Drop", 2 * BAR, 4, BAR),
                (5, "Slide", BAR, 4, 480),
            ],
        ),
        (
            "Keys",
            &[
                (0, "Rhodes", 4 * BAR, 6, 2 * BEAT),
                (1, "Rhodes B", 4 * BAR, 6, 2 * BEAT),
                (3, "Stabs", BAR, 6, 720),
            ],
        ),
        (
            "Lead",
            &[
                (1, "Hook", 2 * BAR, 7, 480),
                (2, "Hook B", 2 * BAR, 7, 480),
                (4, "Solo", 8 * BAR, 7, 480),
            ],
        ),
        (
            "Chords",
            &[
                (0, "Arp chords", 4 * BAR, 8, 2 * BEAT),
                (2, "Pad", 8 * BAR, 8, 2 * BAR),
                (3, "Swell", 4 * BAR, 8, BAR),
            ],
        ),
        (
            "Vox chop",
            &[
                (0, "Chop A", BAR, 9, BEAT),
                (1, "Chop B", BAR, 9, 720),
                (5, "Vox FX", 2 * BAR, 9, BAR),
            ],
        ),
    ];
    for (name, clips) in tracks {
        let mut t = Track::new(name, p.tracks.len() as u16);
        for &(slot, clip_name, length, pad, step) in clips {
            let notes = if pad == 0 && step == BEAT {
                beat(length / BAR)
            } else {
                every(step, length, pad, 96)
            };
            t.slots[slot] = Some(clip(clip_name, length, &notes));
        }
        p.tracks.push(t);
    }
    let mut t = Track::new_loops("Loops A", p.tracks.len() as u16);
    t.kind = TrackKind::Loops(LoopInput::Pads);
    for (slot, n, ticks) in [
        (0usize, 0u8, 4 * BAR),
        (1, 1, 2 * BAR),
        (2, 3, BAR),
        (4, 4, 2 * BAR),
    ] {
        t.slots[slot] = Some(Clip::new_loop(n, ticks));
    }
    p.tracks.push(t);
    p
}

/// [`busy_project`] in the middle of a jam, at 3.4: Drums, Hats, Bass,
/// Keys, Chords and Loops A play from bar 1; Vox chop records over Chop B
/// from bar 3 (red); Lead's Hook and the Drums' Fill wait for bar 4
/// (amber); Vox chop is armed.
pub fn busy_playing() -> Result<Session, String> {
    const VOX: usize = 6;
    let launches = [(0, 0), (1, 0), (2, 1), (3, 0), (5, 0), (VOX, 1), (7, 0)]
        .iter()
        .map(|&(track, slot)| Launch {
            track,
            slot,
            origin: 0,
            start: 0,
        })
        .collect();
    let mut s = Session::new(busy_project(), launches, Some(VOX))?;
    s.command_at(0, Command::Play)?;
    s.run_to(2 * BAR - BEAT)?;
    s.command_at(2 * BAR, Command::Record)?;
    for (i, h) in [0, 720, 1440, 2400, 3120].iter().enumerate() {
        s.pad(2 * BAR + h, SIXTEENTH, 9, 100 + 5 * (i as u8 % 4));
    }
    s.run_to(3 * BAR - BEAT)?;
    s.act(TimeAction::Launch { track: 4, slot: 1 })?;
    s.act(TimeAction::Launch { track: 0, slot: 3 })?;
    s.run_to(3 * BAR - BEAT + SIXTEENTH)?;
    Ok(s)
}

/// Loop tracks (loop-tracks.md): Drums, Hats and Bass, then Loops A
/// (Pads + synth: Loop 1 in rows 1 and 2, Loop 4 in row 4), Loops B (Line
/// in: Loop 2 in row 2, Loop 3 in row 3) and an empty Loops C (Resample),
/// playing row 2 at 3.1.
pub fn loops_playing() -> Result<Session, String> {
    use mpfree_seq::{LoopInput, TrackKind};
    let mut p = project();
    p.tracks.truncate(3);
    let mut loops = |name: &str, input: LoopInput, cells: &[(usize, u8, Tick)]| {
        let mut t = Track::new_loops(name, p.tracks.len() as u16);
        t.kind = TrackKind::Loops(input);
        for &(slot, n, ticks) in cells {
            t.slots[slot] = Some(Clip::new_loop(n, ticks));
        }
        p.tracks.push(t);
    };
    loops(
        "Loops A",
        LoopInput::Pads,
        &[(0, 0, 2 * BAR), (1, 0, 2 * BAR), (3, 3, 4 * BAR)],
    );
    loops(
        "Loops B",
        LoopInput::Line,
        &[(1, 1, 2 * BAR), (2, 2, 4 * BAR)],
    );
    loops("Loops C", LoopInput::Resample, &[]);
    let launches = (0..5)
        .filter(|&t| p.tracks[t].slots[1].is_some())
        .map(|track| Launch {
            track,
            slot: 1,
            origin: 0,
            start: 0,
        })
        .collect();
    let mut s = Session::new(p, launches, Some(0))?;
    s.command_at(0, Command::Play)?;
    s.run_to(2 * BAR + SIXTEENTH)?;
    Ok(s)
}

/// The Song view with Loop tracks (loop-tracks.md part 5): `loops_playing`'s
/// rows 1 to 4 made into a song and song mode on, so the Loop tracks'
/// lanes hold their loops' blocks under the note tracks'.
pub fn song_loops_state() -> Result<TimeState, String> {
    let mut s = loops_playing()?;
    let rows: Vec<(usize, Tick)> = (0..4)
        .filter(|&r| s.project.row_filled(r))
        .map(|r| (r, scene_length(&s.project, r, BAR)))
        .collect();
    s.project.arrangement = scenes_to_song(&s.project, &rows, 0)
        .map(|t| t.song)
        .map_err(|e| format!("{e:?}"))?;
    let mut state = s.state();
    state.song_view = true;
    state.song_on = true;
    Ok(state)
}

/// The Song view of the 24-track project: rows 1 to 4 made into a song,
/// song mode on, scrolled down to the last tracks and "+ Track".
pub fn big_song_state() -> Result<TimeState, String> {
    let mut s = big_playing(19)?;
    let rows: Vec<(usize, Tick)> = (0..4)
        .filter(|&r| s.project.row_filled(r))
        .map(|r| (r, scene_length(&s.project, r, BAR)))
        .collect();
    s.project.arrangement = scenes_to_song(&s.project, &rows, 0)
        .map(|t| t.song)
        .map_err(|e| format!("{e:?}"))?;
    let mut state = s.state();
    state.song_view = true;
    state.song_on = true;
    Ok(state)
}

/// Every track's first clip from bar 1, the glitch from bar 3; the chords
/// wait to be launched.
fn launches() -> Vec<Launch> {
    [0, 1, 2, 5, 6, 7]
        .iter()
        .map(|&track| Launch {
            track,
            slot: 0,
            origin: 0,
            start: 0,
        })
        .chain(std::iter::once(Launch {
            track: 3,
            slot: 0,
            origin: 2 * BAR,
            start: 2 * BAR,
        }))
        .collect()
}

/// The clank pattern the samples record over the stab, in bar `bar` (from 0).
fn clank(s: &mut Session, bar: Tick, hits: &[Tick]) {
    for (i, &h) in hits.iter().enumerate() {
        s.pad(bar * BAR + h, SIXTEENTH, 12, 100 + 5 * (i as u8 % 4));
    }
}

/// Playing from bar 1 to 5.3.240; at 5.3 the chords were tapped, so they
/// start on the next bar, and the glitch was tapped, so it stops there.
pub fn playing() -> Result<Session, String> {
    let mut s = Session::new(project(), launches(), Some(STAB))?;
    s.command_at(0, Command::Play)?;
    s.run_to(4 * BAR + 2 * BEAT)?;
    s.act(TimeAction::Launch {
        track: CHORDS,
        slot: 0,
    })?;
    s.act(TimeAction::StopClip(3))?;
    s.run_to(4 * BAR + 2 * BEAT + SIXTEENTH)?;
    Ok(s)
}

/// Replace recording on the stab from bar 3: the pads play a clank riff,
/// the stab riff is erased for one pass; at 4.2.480, still recording.
pub fn recording() -> Result<Session, String> {
    let mut s = Session::new(project(), launches(), Some(STAB))?;
    s.command_at(0, Command::Play)?;
    s.run_to(2 * BAR - BEAT)?;
    s.command_at(2 * BAR, Command::Record)?;
    clank(&mut s, 2, &[0, 960, 1680, 2880]);
    clank(&mut s, 3, &[480]);
    s.run_to(3 * BAR + BEAT + 480)?;
    Ok(s)
}

/// Take 1 replaced the stab riff (bars 3 and 4), take 2 overdubbed onto it
/// (bar 6), and take 2 was undone: Undo would take back take 1, Redo would
/// bring take 2 back.
pub fn undone() -> Result<Session, String> {
    let mut s = recording()?;
    s.command_at(4 * BAR, Command::Record)?;
    s.run_to(4 * BAR + BEAT)?;
    s.command_at(5 * BAR - BEAT, Command::ToggleOverdub)?;
    s.command_at(5 * BAR, Command::Record)?;
    clank(&mut s, 5, &[240, 1200, 2160, 3120]);
    s.run_to(6 * BAR - BEAT)?;
    s.command_at(6 * BAR, Command::Record)?;
    s.run_to(6 * BAR + 2 * BEAT)?;
    s.undo()?;
    Ok(s)
}

/// Counting in one bar before recording from bar 1, at 0.3.000.
pub fn counting_in() -> Result<Session, String> {
    let mut s = Session::new(project(), launches(), Some(STAB))?;
    s.command_at(0, Command::SetCountIn(1))?;
    s.command_at(0, Command::SetLoop(Some((0, 4 * BAR))))?;
    s.command_at(0, Command::Record)?;
    // The position is 1.1 until the commands apply; then it counts in from 0.1.
    s.run_periods(1)?;
    s.run_to(-BAR + 2 * BEAT)?;
    Ok(s)
}

/// A chord riff the pads play over bars 3 and 4 (6 notes), for CAPTURE.
pub const CHORD_RIFF: [(Tick, u8); 6] = [
    (0, 7),
    (720, 8),
    (1440, 7),
    (BAR, 8),
    (BAR + BEAT, 7),
    (BAR + 2400, 8),
];

/// Playing from bar 1 with the chords track armed but nothing recording:
/// the pads played a chord riff over bars 3 and 4, and at 5.1.480 Capture
/// would keep those 2 bars.
pub fn capture_ready() -> Result<Session, String> {
    let mut s = Session::new(project(), launches(), Some(CHORDS))?;
    s.command_at(0, Command::Play)?;
    for (i, &(tick, note)) in CHORD_RIFF.iter().enumerate() {
        s.pad(2 * BAR + tick, SIXTEENTH * 2, note, 96 + 4 * (i as u8 % 3));
    }
    s.run_to(4 * BAR + 480)?;
    Ok(s)
}

/// Capture pressed at 5.1.480: "Capture 1" in the chords' slot 2 plays on
/// in its place (the riff's bars are 3 and 4, so bar 5 is its first again);
/// at 5.2.
pub fn captured() -> Result<Session, String> {
    let mut s = capture_ready()?;
    s.act(TimeAction::Capture)?;
    s.run_to(4 * BAR + BEAT)?;
    Ok(s)
}

/// Stopped at bar 1 with nothing launched and nothing armed.
pub fn stopped() -> Result<Session, String> {
    Session::new(project(), Vec::new(), None)
}

fn arp_chip() -> Chip {
    // Sample text: the arp thread's chip goes here when it lands.
    Chip::new("Arp · Up · 1/16", Tone::Accent)
}

fn app_ui(fonts: Fonts, tab: &str, hint: &str) -> Ui {
    let mut ui = Ui::new(Theme::dark(), fonts, SCREEN.0, SCREEN.1);
    let mut header = Header::app(tab);
    header.chips = vec![
        Chip::new("Akai session", Tone::Good),
        Chip::new("Kit 2 of 6", Tone::Accent),
    ];
    ui.add("header", header);
    ui.add("hint", Label::new(hint, Tone::Dim, Align::Center));
    ui
}

fn clips_sample(
    name: &'static str,
    about: &'static str,
    fonts: Fonts,
    s: &Session,
    first_row: usize,
) -> Result<Sample, String> {
    let mut ui = app_ui(fonts, "Arrange", CLIPS_HINT);
    let mut page = TimePage::add_to(&mut ui)?;
    ui.show(CLIPS_PAGE)?;
    page.first_row = first_row;
    // No chips: on Arrange the arp's chip is in the header (option E).
    let state = s.state();
    page.update(&mut ui, &state);
    Ok(Sample {
        name,
        about,
        ui,
        state,
    })
}

/// The Song view's state: rows 1 to 5 made into a song, the third placement
/// on the drums made unique and selected, song mode on and Rec song armed,
/// playing at 5.3. The app's `--preview` draws it too, for the manual.
pub fn song_state() -> Result<TimeState, String> {
    let mut s = playing()?;
    let rows: Vec<(usize, Tick)> = (0..5)
        .filter(|&r| s.project.row_filled(r))
        .map(|r| (r, scene_length(&s.project, r, BAR)))
        .collect();
    let song = scenes_to_song(&s.project, &rows, 0)
        .map(|t| t.song)
        .map_err(|e| format!("{e:?}"))?;
    s.project.arrangement = song;
    s.project.make_unique(2).map_err(|e| format!("{e:?}"))?;
    s.project.scene_mut(1).ok_or("no row 2")?.name = "Verse".into();
    s.project.arrangement.markers[1].name = "Verse".into();
    let mut state = s.state();
    state.song_view = true;
    state.song_on = true;
    state.song_rec = true;
    state.selected = Some(2);
    Ok(state)
}

/// The Song view of [`busy_project`]: its seven filled rows made into a
/// song, named as a song's parts, song mode on and Rec song armed, the
/// Keys' second block picked; playing at 5.3.
pub fn busy_song_state() -> Result<TimeState, String> {
    busy_song(busy_project()).map(|(_, state)| state)
}

/// [`busy_song_state`] from `project`, with the session it was played in.
fn busy_song(project: Project) -> Result<(Session, TimeState), String> {
    let mut s = Session::new(project, Vec::new(), Some(0))?;
    let rows: Vec<(usize, Tick)> = (0..7)
        .filter(|&r| s.project.row_filled(r))
        .map(|r| (r, scene_length(&s.project, r, BAR)))
        .collect();
    let names = [
        "Intro", "Verse", "Build", "Drop", "Break", "Bridge", "Outro",
    ];
    for (i, &(r, _)) in rows.iter().enumerate() {
        if let Some(sc) = s.project.scene_mut(r) {
            sc.name = names[i % names.len()].into();
        }
    }
    s.project.arrangement = scenes_to_song(&s.project, &rows, 0)
        .map(|t| t.song)
        .map_err(|e| format!("{e:?}"))?;
    s.command_at(0, Command::Play)?;
    s.run_to(4 * BAR + 2 * BEAT)?;
    let mut state = s.state();
    state.song_view = true;
    state.song_on = true;
    state.song_rec = true;
    state.selected = Some(4);
    Ok((s, state))
}

/// The Song view's Chords lane (composing step 6, board C): the busy song
/// with Row 5's tune on the Lead in its first three rows (the Verse's a
/// fourth up, the Build's a tone down), each scene the song plays heard as
/// the time engine hears it (one `AddPartJob::hear` a row), and the
/// Verse's second chord set to Ab7, yours. Nothing picked.
pub fn song_chords_state() -> Result<TimeState, String> {
    use mpfree_compose::{AddAsk, AddPartJob, InstrumentKind, Relation, Settings};
    let mut p = busy_project();
    for (row, up) in [(0usize, 0i16), (1, 5), (2, -2)] {
        let mut tune = solo_tune();
        for n in &mut tune.notes {
            n.note = (i16::from(n.note) + up).clamp(0, 127) as u8;
        }
        tune.name = ["Tune", "Tune up", "Tune down"][row].into();
        p.tracks[4].slots[row] = Some(tune);
    }
    let (s, mut state) = busy_song(p)?;
    state.selected = None;
    let t = state.transport;
    let mut rows: Vec<usize> = state.scene_sections.iter().map(|x| x.2).collect();
    rows.sort_unstable();
    rows.dedup();
    let mut heard = Vec::new();
    for row in rows {
        let ask = AddAsk {
            row,
            parts: row_reads(&s.project, row, usize::MAX),
            lanes: Default::default(),
            part: mpfree_context::PartRole::Melody,
            relation: Relation::Support,
            kind: InstrumentKind::OneNote,
            pads: [None; 4],
            swing: t.swing,
            transport_swing: t.swing,
            meter: t.meter,
            bpm: t.bpm,
            budget: mpfree_context::ReadBudget::default(),
            settings: Settings::default(),
            bars: None,
            attempt: 0,
            phrase: None,
        };
        heard.push((
            row,
            std::sync::Arc::new(AddPartJob::new(&s.project, ask).hear()),
        ));
    }
    // The Verse (the song's second section) with its second chord yours.
    let verse = state.scene_sections.get(1).map(|x| x.2).ok_or("no Verse")?;
    if let Some((_, c)) = heard.iter_mut().find(|(r, _)| *r == verse) {
        let ab7 = mpfree_context::chord::Chord::new(8, crate::add_view::KIND_QUALITIES[2]);
        let chip = 1.min(c.line.spans().len().saturating_sub(1));
        let line = crate::add_view::use_chord(&c.line, chip, ab7).ok_or("no chord to set")?;
        *c = std::sync::Arc::new(mpfree_compose::RowChords {
            line,
            heard: c.heard.clone(),
        });
    }
    state.song_chords = heard.into();
    Ok(state)
}

/// The Song view with tempo points (arrange.md, the song's tempo): 100 BPM
/// from bar 3 ramping up to 128 at bar 6, that first point picked so the
/// lane's buttons show with Ramp lit; the project's 92 BPM plays before
/// bar 3. The transport bar reads the tempo the ramp plays at 5.3, as the
/// app's would.
pub fn song_tempo_state() -> Result<TimeState, String> {
    let mut s = playing()?;
    let rows: Vec<(usize, Tick)> = (0..5)
        .filter(|&r| s.project.row_filled(r))
        .map(|r| (r, scene_length(&s.project, r, BAR)))
        .collect();
    let mut song = scenes_to_song(&s.project, &rows, 0)
        .map(|t| t.song)
        .map_err(|e| format!("{e:?}"))?;
    song.set_tempo(2 * BAR, 100.0, Some(true))
        .map_err(|e| format!("{e:?}"))?;
    song.set_tempo(5 * BAR, 128.0, Some(false))
        .map_err(|e| format!("{e:?}"))?;
    let map = song.tempo_map(BPM).map_err(|e| format!("{e:?}"))?;
    s.project.arrangement = song;
    let mut state = s.state();
    state.song_view = true;
    state.song_on = true;
    state.tempo_pick = Some(0);
    state.tempo_here = Some(100.0);
    state.transport.bpm = (map.bpm_at(state.transport.position) * 10.0).round() / 10.0;
    Ok(state)
}

/// Clips played over the song (the Arrange refactor's 5.1, D3), in
/// Perform with song mode on: rows 1 to 5 made into the song; Drums, Bass
/// and Stab launched over it, Glitch launched from bar 3 and going back
/// to the song at 6.1, the chords cued for 6.1; Hats, Dust and FX play
/// the song. `song_view` shows it on the Song view in place of the grid.
pub fn over_song_state(song_view: bool) -> Result<TimeState, String> {
    let launches = launches()
        .into_iter()
        .filter(|l| [0, 2, 3, 5].contains(&l.track))
        .collect();
    let mut s = Session::new(project(), launches, Some(STAB))?;
    let rows: Vec<(usize, Tick)> = (0..5)
        .filter(|&r| s.project.row_filled(r))
        .map(|r| (r, scene_length(&s.project, r, BAR)))
        .collect();
    s.project.arrangement = scenes_to_song(&s.project, &rows, 0)
        .map(|t| t.song)
        .map_err(|e| format!("{e:?}"))?;
    s.command_at(0, Command::Play)?;
    s.run_to(4 * BAR + 2 * BEAT)?;
    s.act(TimeAction::Launch {
        track: CHORDS,
        slot: 0,
    })?;
    s.act(TimeAction::StopClip(3))?;
    s.run_to(4 * BAR + 2 * BEAT + SIXTEENTH)?;
    let mut state = s.state();
    state.perform = true;
    state.song_on = true;
    state.song_view = song_view;
    Ok(state)
}

/// Recordings into a real audio tracks engine for the Song view's audio
/// lanes: "Guitar" (audio track 2, stereo) plays bars 1 to 4; "Vocal"
/// (audio track 1, the left jack) was recorded over bars 1 to 8 and then
/// punched in again over bars 5 to 8, with fades on that region. The
/// signals are made up: a strummed tone and a sung-like one, in phrases.
pub struct AudioSession {
    pub tracks: mpfree_audiotracks::AudioTracks,
    pub map: mpfree_transport::time::TempoMap,
}

/// The rate the sample's audio is made at: low, so it is quick to make.
const AUDIO_RATE: u32 = 22_050;

impl AudioSession {
    pub fn new() -> Result<AudioSession, String> {
        use mpfree_audiotracks::{AudioTracks, Command as A, Input};
        let mut t = Transport::new(AUDIO_RATE, BPM).map_err(|e| format!("{e:?}"))?;
        let mut a = AudioTracks::new(AUDIO_RATE, 60);
        let mut b = Block::new();
        let bar_frames = 4.0 * 60.0 / BPM * f64::from(AUDIO_RATE);
        let at = |bars: f64| (bars * bar_frames).round() as u64;
        // A phrase every two beats; the vocal's own pitch and a softer
        // strum for the guitar.
        let sound = |g: u64, hz: f64, phrase: f64| -> f32 {
            let s = g as f64 / f64::from(AUDIO_RATE);
            let beat = 60.0 / BPM;
            let p = (s / (phrase * beat)).fract();
            let env = (p * 6.0).min(1.0) * (1.0 - p).powf(1.5);
            let tone = (s * hz * std::f64::consts::TAU).sin() * 0.6
                + (s * hz * 2.01 * std::f64::consts::TAU).sin() * 0.25;
            let grain = ((g.wrapping_mul(2_654_435_761) >> 7) % 1000) as f64 / 1000.0 - 0.5;
            ((tone + grain * 0.3) * env * 0.9) as f32
        };
        let mut frame = 0u64;
        /// Periods of 1024 frames until frame `to`, the input from `input`.
        fn run(
            t: &mut Transport,
            a: &mut AudioTracks,
            b: &mut Block,
            frame: &mut u64,
            to: u64,
            input: &dyn Fn(u64) -> (f32, f32),
        ) -> Result<(), String> {
            const N: usize = 1024;
            let (mut inp, mut out) = (vec![0.0f32; 2 * N], vec![0.0f32; 2 * N]);
            while *frame < to {
                let n = (N as u64).min(to - *frame) as usize;
                for j in 0..n {
                    let (l, r) = input(*frame + j as u64);
                    inp[2 * j] = l;
                    inp[2 * j + 1] = r;
                }
                if !t.process(n as u32, b) {
                    return Err("audio sample: the transport refused a period".into());
                }
                let map = *t.tempo_map();
                a.process(b, &map, &inp[..2 * n], &mut out[..2 * n]);
                *frame += n as u64;
            }
            Ok(())
        }
        let cmd = |a: &mut AudioTracks, c: A| -> Result<(), String> {
            a.command(c)
                .then_some(())
                .ok_or_else(|| format!("audio sample: {c:?} refused"))
        };
        let guitar = move |g: u64| {
            let v = sound(g, 196.0, 1.0) * 0.8;
            (v, v * 0.9)
        };
        let vocal = move |g: u64| (sound(g, 330.0, 2.0), 0.0);
        let vocal_again = move |g: u64| (sound(g + 777, 294.0, 2.0) * 0.85, 0.0);
        // Guitar: bars 1 to 4, stereo.
        cmd(&mut a, A::SetInput(1, Input::Stereo))?;
        cmd(&mut a, A::Arm(1, true))?;
        t.command(at(0.0), Command::Record);
        t.command(at(4.0), Command::Stop);
        run(&mut t, &mut a, &mut b, &mut frame, at(4.0) + 1, &guitar)?;
        cmd(&mut a, A::Arm(1, false))?;
        // Vocal: bars 1 to 8, the left jack.
        cmd(&mut a, A::SetInput(0, Input::Left))?;
        cmd(&mut a, A::Arm(0, true))?;
        let from = frame + 1024;
        t.command(from, Command::Locate(0));
        t.command(from + 1, Command::Record);
        t.command(from + 1 + at(8.0), Command::Stop);
        run(
            &mut t,
            &mut a,
            &mut b,
            &mut frame,
            from + at(8.0) + 2048,
            &vocal,
        )?;
        // Again over bars 5 to 8.
        let from = frame + 1024;
        t.command(from, Command::Locate(4 * BAR));
        t.command(from + 1, Command::Record);
        t.command(from + 1 + at(4.0), Command::Stop);
        run(
            &mut t,
            &mut a,
            &mut b,
            &mut frame,
            from + at(4.0) + 2048,
            &vocal_again,
        )?;
        cmd(&mut a, A::Arm(0, false))?;
        // Fades on the new part, and the guitar armed again.
        cmd(
            &mut a,
            A::SetFades {
                track: 0,
                at: 4 * BAR + BAR / 2,
                fade_in: AUDIO_RATE / 2,
                fade_out: 2 * AUDIO_RATE,
            },
        )?;
        cmd(&mut a, A::Arm(1, true))?;
        let to = frame + 1024;
        run(&mut t, &mut a, &mut b, &mut frame, to, &|_| (0.0, 0.0))?;
        Ok(AudioSession {
            map: *t.tempo_map(),
            tracks: a,
        })
    }

    /// A take's waveform as `AudioHost::wave` reads it, `columns` wide.
    pub fn wave(&self, id: u16, columns: usize) -> Option<mpfree_audiotracks::host::TakeWave> {
        let take = self.tracks.take(id)?;
        let per = take.frames.div_ceil(columns.max(1)).max(1);
        let columns = (0..take.frames.div_ceil(per))
            .map(|c| {
                let (from, to) = (c * per, ((c + 1) * per).min(take.frames));
                self.tracks.peaks(id, from, to).unwrap_or((0.0, 0.0))
            })
            .collect();
        Some(mpfree_audiotracks::host::TakeWave {
            id,
            frames: take.frames,
            channels: take.channels,
            frames_per_column: per,
            columns,
        })
    }

    /// The lanes for the Song view: Vocal (L, audio track 1) and Guitar
    /// (ST, armed, audio track 2), with their waveforms.
    pub fn lanes(
        &self,
    ) -> (
        Vec<crate::song::AudioLane>,
        Vec<mpfree_audiotracks::host::TakeWave>,
    ) {
        let takes: Vec<_> = self.tracks.takes().collect();
        let lanes = [
            (0, "Vocal", mpfree_audiotracks::Input::Left, 0.42),
            (1, "Guitar", mpfree_audiotracks::Input::Stereo, 0.63),
        ]
        .into_iter()
        .map(|(track, name, input, level)| crate::song::AudioLane {
            track,
            // In the track list: song_audio_state puts them there.
            row: usize::MAX,
            name: name.into(),
            color: crate::view::audio_color(track),
            armed: self.tracks.armed(track),
            input,
            mix: None,
            level,
            volume: 1.0,
            pan: 0.0,
            monitor: mpfree_audiotracks::Monitor::Auto,
            mute: false,
            solo: false,
            blocks: crate::song::audio_blocks(
                track,
                self.tracks.regions(track),
                &takes,
                &self.map,
                AUDIO_RATE,
            ),
            takes: crate::song::take_lanes(
                track,
                self.tracks.regions(track),
                &takes,
                &self.map,
                AUDIO_RATE,
            ),
            recording: None,
            clips: Vec::new(),
        })
        .collect();
        let waves = takes.iter().filter_map(|t| self.wave(t.id, 600)).collect();
        (lanes, waves)
    }

    /// Seconds free and in all, as the Song view's legend shows them
    /// (seconds of stereo: two samples a frame).
    pub fn free(&self) -> (f64, f64) {
        let m = self.tracks.memory();
        let per_s = 2.0 * f64::from(AUDIO_RATE);
        (
            (m.capacity - m.used) as f64 / per_s,
            m.capacity as f64 / per_s,
        )
    }
}

/// The Song view with audio lanes under the song of [`song_state`]: the
/// Vocal's newer take picked (so the bottom row is the region's), the
/// Guitar armed. The app's `--preview` draws it too, for the manual.
pub fn song_audio_state() -> Result<TimeState, String> {
    let mut state = song_state()?;
    let audio = AudioSession::new()?;
    let (mut lanes, waves) = audio.lanes();
    // The two audio tracks at the end of the track list (the audio tracks
    // rethink's phase 1), each lane in its track's place.
    for l in &mut lanes {
        let mut t = state.tracks[0].clone();
        t.name = l.name.clone();
        t.sound = format!("In {}", crate::song::input_chip(l.input));
        t.audio = true;
        t.loops = None;
        t.melodic = false;
        (t.mute, t.solo, t.silent) = (false, false, false);
        t.slots = vec![None; t.slots.len()];
        l.row = state.tracks.len();
        l.color = crate::view::track_color(l.row);
        state.tracks.push(t);
    }
    let picked = lanes[0]
        .blocks
        .iter()
        .position(|b| b.start >= 4 * BAR)
        .ok_or("audio sample: no region from bar 5")?;
    state.audio = lanes;
    state.waves = waves;
    state.selected = None;
    state.region = Some(crate::view::RegionId {
        track: 0,
        index: picked,
    });
    state.audio_free = Some(audio.free());
    Ok(state)
}

/// The Song view with blocks of audio clips (phase 2, step 2d, layout
/// spec 4c, board E), from [`song_audio_state`]: on Guitar's lane after
/// its recording, Riff (its first two bars as a clip, played twice from
/// bar 9, picked) and Outro (a once clip of all four, from bar 13).
pub fn song_audio_blocks_state() -> Result<TimeState, String> {
    use crate::song::{ClipBlock, ClipPiece};
    use crate::view::PlacementInfo;
    let mut state = song_audio_state()?;
    state.region = None;
    let g = 1;
    let row = state.audio[g].row;
    let r = state.audio[g]
        .blocks
        .first()
        .cloned()
        .ok_or("audio blocks sample: no guitar recording")?;
    // The clip's bars from the recording's start, frames in step.
    let per_bar = r.frames * BAR as f64 / (r.end - r.start).max(1) as f64;
    let piece = |bars: Tick| ClipPiece {
        start: 0,
        end: bars * BAR,
        take: r.take,
        src: r.src,
        frames: per_bar * bars as f64,
    };
    let mut placements = state.placements.to_vec();
    let riff = placements.len();
    for (name, start, pass, once) in [
        ("Riff", 8 * BAR, 2 * BAR, false),
        ("Outro", 12 * BAR, 4 * BAR, true),
    ] {
        state.audio[g].clips.push(ClipBlock {
            index: placements.len(),
            origin: start,
            pass,
            once,
            pieces: vec![piece(pass / BAR)],
        });
        placements.push(PlacementInfo {
            track: row,
            start,
            length: 4 * BAR,
            name: name.into(),
            unique: false,
        });
    }
    state.placements = placements.into();
    state.selected = Some(riff);
    Ok(state)
}

/// The Clips grid with audio clips in the audio tracks' cells (layout
/// spec 4f), as on the approved boards, at 92 BPM a little before bar 17:
/// Vocal's Verse vox stopped, Hook vox playing, Ad libs from 17.1;
/// Guitar's Riff made at 120, Outro waiting for its audio until 18.1,
/// Solo missing; and a third audio track, Break, with Intro still
/// ringing past its stop, Amen not ready and Pad wash. Guitar is armed,
/// so its Riff is on the Clip tab.
pub fn clips_audio_state() -> Result<TimeState, String> {
    use crate::view::{AudioCellInfo, ClipInfo};
    let mut state = song_audio_state()?;
    state.song_view = false;
    state.region = None;
    let (vocal, guitar) = (state.audio[0].row, state.audio[1].row);
    let brk_row = state.tracks.len();
    let mut lane = state.audio[1].clone();
    lane.track = 2;
    lane.row = brk_row;
    lane.name = "Break".into();
    lane.input = mpfree_audiotracks::Input::Sum;
    lane.color = crate::view::track_color(brk_row);
    lane.armed = false;
    lane.blocks.clear();
    lane.takes.clear();
    lane.recording = None;
    let mut brk = state.tracks[guitar].clone();
    brk.name = lane.name.clone();
    brk.sound = format!("In {}", crate::song::input_chip(lane.input));
    state.tracks.push(brk);
    state.audio.push(lane);
    state.transport.bpm = 92.0;
    state.transport.position = 15 * BAR + 2 * BEAT;
    let clip = |name: &str, bars: Tick, seed: u32, shape: Shape| {
        let mut settings = Clip::new(name, bars * BAR).settings();
        settings.once = shape == Shape::Swell;
        ClipInfo {
            name: name.to_string(),
            length: bars * BAR,
            speed: mpfree_seq::Speed::NORMAL,
            notes: 0,
            looped: None,
            settings,
            variation: None,
            holds_notes: false,
            holds: mpfree_seq::Holds::Hits,
            audio: Some(AudioCellInfo {
                bpm: if name == "Riff" { 120.0 } else { 92.0 },
                gain: 1.0,
                secs: 197.0,
                wave: sample_wave(seed, shape),
                missing: None,
                ring: None,
            }),
            loop_id: None,
        }
    };
    // A cell: its name, bars, wave seed and shape.
    type Cell = (&'static str, Tick, u32, Shape);
    let cells: [(usize, [Cell; 3]); 3] = [
        (
            vocal,
            [
                ("Verse vox", 4, 3, Shape::Vocal),
                ("Hook vox", 2, 11, Shape::Vocal),
                ("Ad libs", 2, 7, Shape::Vocal),
            ],
        ),
        (
            guitar,
            [
                ("Riff", 2, 5, Shape::Riff),
                ("Outro", 4, 9, Shape::Riff),
                ("Solo", 4, 13, Shape::Riff),
            ],
        ),
        (
            brk_row,
            [
                ("Intro", 4, 17, Shape::Swell),
                ("Amen", 1, 19, Shape::Hits),
                ("Pad wash", 4, 23, Shape::Pad),
            ],
        ),
    ];
    for (track, clips) in cells {
        for (slot, (name, bars, seed, shape)) in clips.into_iter().enumerate() {
            state.tracks[track].slots[slot] = Some(clip(name, bars, seed, shape));
        }
    }
    audio_of(&mut state, guitar, 2)?.missing =
        Some("take 4's file wasn't found when the project opened".into());
    audio_of(&mut state, brk_row, 0)?.ring = Some(0.78);
    state
        .launches
        .retain(|l| l.track != vocal && l.track != guitar && l.track != brk_row);
    // Hook vox 0.45 through its two bars; Ad libs from bar 17.
    state.launches.push(Launch {
        track: vocal,
        slot: 1,
        origin: 15 * BAR + 2 * BEAT - (BAR * 9 / 10),
        start: 0,
    });
    state.cues.retain(|c| c.track != vocal && c.track != guitar);
    state.cues.push(Cue {
        track: vocal,
        slot: Some(2),
    });
    state.cues.push(Cue {
        track: guitar,
        slot: Some(1),
    });
    state.audio_late = vec![(guitar, 17 * BAR)];
    state.not_ready = vec![(brk_row, 1, 2 * BAR)];
    state.armed = None;
    state.audio[1].armed = true;
    state.audio_focus = Some(guitar);
    Ok(state)
}

/// The audio facts of the clip in cell `slot` of `track`.
fn audio_of(
    st: &mut TimeState,
    track: usize,
    slot: usize,
) -> Result<&mut crate::view::AudioCellInfo, String> {
    st.tracks[track].slots[slot]
        .as_mut()
        .and_then(|c| c.audio.as_mut())
        .ok_or_else(|| "audio cells sample: no clip".to_string())
}

/// The shapes of the sample's small waves.
#[derive(Clone, Copy, PartialEq)]
enum Shape {
    Vocal,
    Riff,
    Swell,
    Hits,
    Pad,
}

/// A small wave for a sample cell: `shape`'s envelope over noise from
/// `seed`, each column's peak 0 to 255.
fn sample_wave(seed: u32, shape: Shape) -> [u8; crate::widgets::CELL_WAVE] {
    let mut w = [0u8; crate::widgets::CELL_WAVE];
    let mut x = seed.wrapping_mul(2_654_435_761).max(1);
    let n = w.len() as f32;
    for (c, v) in w.iter_mut().enumerate() {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        let noise = (x % 1000) as f32 / 1000.0;
        let t = c as f32 / n;
        let env = match shape {
            Shape::Vocal => 0.35 + 0.55 * (t * 9.0).sin().abs(),
            Shape::Riff => 0.45 + 0.45 * ((t * 16.0).fract() < 0.5) as u8 as f32,
            Shape::Swell => 0.15 + 0.8 * t,
            Shape::Hits => 0.2 + 0.75 * ((t * 8.0).fract() < 0.25) as u8 as f32,
            Shape::Pad => 0.55 + 0.1 * (t * 3.0).sin(),
        };
        *v = ((env * (0.75 + 0.25 * noise)).clamp(0.0, 1.0) * 255.0) as u8;
    }
    w
}

/// The Clips page's Song view, from [`song_state`].
fn song_sample(fonts: Fonts) -> Result<Sample, String> {
    let state = song_state()?;
    let mut ui = app_ui(fonts, "Arrange", SONG_HINT);
    let mut page = TimePage::add_to(&mut ui)?;
    ui.show(CLIPS_PAGE)?;
    page.update(&mut ui, &state);
    Ok(Sample {
        name: "clips-song",
        about: "Clips page, Song view: rows 1 to 5 as a song, a drum placement made unique \
                and selected, song mode on at 5.3, Rec song armed",
        ui,
        state,
    })
}

/// The Song view with audio lanes, from [`song_audio_state`].
fn song_audio_sample(fonts: Fonts) -> Result<Sample, String> {
    let state = song_audio_state()?;
    let mut ui = app_ui(fonts, "Arrange", SONG_HINT);
    let mut page = TimePage::add_to(&mut ui)?;
    ui.show(CLIPS_PAGE)?;
    page.update(&mut ui, &state);
    Ok(Sample {
        name: "clips-song-audio",
        about: "Arrange page, Song view with two audio tracks after the song's tracks: \
                Vocal recorded over bars 1-8 and punched in over 5-8 (that region picked, \
                with fades), Guitar armed",
        ui,
        state,
    })
}

/// The Song view with blocks of audio clips, from
/// [`song_audio_blocks_state`], and the refusal of one dropped over a
/// region on the hint line.
fn song_audio_blocks_sample(fonts: Fonts) -> Result<Sample, String> {
    let state = song_audio_blocks_state()?;
    let hint = "song: Riff can't go over the region at bar 1: a block and a region never overlap on a lane";
    let mut ui = app_ui(fonts, "Arrange", hint);
    let mut page = TimePage::add_to(&mut ui)?;
    ui.show(CLIPS_PAGE)?;
    page.update(&mut ui, &state);
    Ok(Sample {
        name: "clips-song-audio-blocks",
        about: "Arrange page, Song view with blocks of audio clips on Guitar's lane: Riff \
                (two bars, twice, picked) and Outro (a once clip); a block dropped over a \
                region refused",
        ui,
        state,
    })
}

/// "Keys": two bars of chords in C minor under a short melody, played in
/// by hand, so some notes sit a little off the 1/16 grid.
pub fn keys_clip() -> Clip {
    let mut c = Clip::new("Keys", 2 * BAR);
    // Played on a synth: notes, not pad hits (its scale and plan 4.2's
    // rows go by it).
    c.holds = mpfree_seq::Holds::Notes;
    // (tick, length, note, velocity)
    let chords: [(Tick, Tick, [u8; 3]); 4] = [
        (0, 2 * BEAT - 40, [60, 63, 67]),
        (2 * BEAT + 12, 2 * BEAT - 60, [56, 60, 63]),
        (BAR - 8, 2 * BEAT, [58, 62, 65]),
        (BAR + 2 * BEAT + 20, 2 * BEAT - 80, [55, 58, 62]),
    ];
    for (tick, len, notes) in chords {
        for (k, n) in notes.into_iter().enumerate() {
            c.add_note(Note::new(tick + 6 * k as Tick, len, n, 84 - 6 * k as u8))
                .ok();
        }
    }
    let melody: [(Tick, Tick, u8, u8); 6] = [
        (BEAT + 14, 220, 75, 104),
        (BEAT + 3 * SIXTEENTH - 10, 240, 74, 92),
        (3 * BEAT + 5, 460, 72, 110),
        (BAR + BEAT - 18, 240, 70, 96),
        (BAR + 2 * BEAT + 2 * SIXTEENTH + 9, 230, 72, 100),
        (BAR + 3 * BEAT + 16, 700, 67, 118),
    ];
    for (tick, len, n, v) in melody {
        c.add_note(Note::new(tick, len, n, v)).ok();
    }
    // The two long melody notes were leaned into: their pressure swells
    // (the roll draws it along their bottoms).
    let swells: [(Tick, &[(u32, u8)]); 2] = [
        (
            3 * BEAT + 5,
            &[(0, 30), (60, 70), (140, 112), (260, 90), (380, 40)],
        ),
        (
            BAR + 3 * BEAT + 16,
            &[
                (0, 20),
                (120, 50),
                (240, 85),
                (360, 120),
                (520, 100),
                (640, 60),
            ],
        ),
    ];
    for (tick, run) in swells {
        let points: Vec<TouchPoint> = run
            .iter()
            .map(|&(at, value)| TouchPoint { at, value })
            .collect();
        let touch = c.add_touch(&points).expect("sample pressure fits");
        if let Some(n) = c.notes.iter_mut().find(|n| n.tick == tick) {
            n.touch = touch;
        }
    }
    c.check().expect("sample clip in range");
    c
}

/// The note editor on "Keys" (Chords, slot 2) over the playing session,
/// the melody selected, through `lens`. The app's `--preview` draws it for
/// the manual.
pub fn notes_state(lens: crate::editor::Lens) -> Result<TimeState, String> {
    let mut s = playing()?;
    let keys = keys_clip();
    s.project.tracks[CHORDS].slots[1] = Some(keys.clone());
    let mut state = s.state();
    let mut editor = crate::editor::Editor::new(CHORDS, 1, &keys);
    editor.sel = keys
        .notes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.note >= 67 && n.len < BEAT)
        .map(|(i, _)| i)
        .collect();
    editor.lens = lens;
    state.edit = Some(Box::new(crate::view::EditInfo {
        editor,
        clip: keys.into(),
        rev: 0,
        track_name: "Chords".into(),
        bar: BAR,
        answer: Default::default(),
        locks: Default::default(),
        ghosts: Default::default(),
    }));
    Ok(state)
}

/// The piano roll on the same clip with its scale on (piano roll plan
/// 2B.1): C Dorian, so the G#2 of the second chord is off it and marked;
/// the pads play C minor on the Synth tab, which the Scale menu offers
/// first.
pub fn notes_scale_state() -> Result<TimeState, String> {
    use mpfree_seq::mpfree_scale::{DORIAN, MINOR};
    let mut state = notes_state(crate::editor::Lens::Grid)?;
    let e = state.edit.as_mut().ok_or("no editor")?;
    let mut keys = (*e.clip).clone();
    keys.scale = Some(mpfree_seq::Scale::new(0, DORIAN));
    e.clip = keys.into();
    e.editor.offers = std::sync::Arc::new(crate::editor::ScaleOffers {
        pads: Some(mpfree_seq::Scale::new(0, MINOR)),
        own: vec![("Hijaz".into(), 0b0101_1011_0011, 0b0101_1011_0011)],
    });
    Ok(state)
}

/// The keys clip with the Chord chip on Major seventh (piano roll plan
/// 2B.2) and the chord with its G#2 picked, so the inspector names it.
pub fn notes_chord_state() -> Result<TimeState, String> {
    let mut state = notes_state(crate::editor::Lens::Grid)?;
    let e = state.edit.as_mut().ok_or("no editor")?;
    e.editor.chord = crate::editor::ChordShape::Seventh;
    let all: Vec<usize> = (0..e.clip.notes.len()).collect();
    let g_sharp = e
        .clip
        .notes
        .iter()
        .position(|n| n.note == 56)
        .ok_or("no G#2")?;
    e.editor.sel = mpfree_seq::notes::chords_of(&e.clip, &all)
        .into_iter()
        .find(|chord| chord.contains(&g_sharp))
        .unwrap_or_default();
    Ok(state)
}

/// A two-bar riff in C minor (composing tools, phase 1): what Continue
/// carries on in the samples.
pub fn riff_clip() -> Clip {
    let mut c = Clip::new("Riff", 2 * BAR);
    c.holds = mpfree_seq::Holds::Notes;
    // (tick, length, note, velocity)
    let notes: [(Tick, Tick, u8, u8); 13] = [
        (0, 440, 60, 100),
        (2 * SIXTEENTH, 220, 63, 84),
        (3 * SIXTEENTH, 220, 65, 90),
        (BEAT, 440, 67, 108),
        (2 * BEAT, 440, 70, 96),
        (2 * BEAT + 2 * SIXTEENTH, 440, 67, 88),
        (3 * BEAT, 900, 65, 104),
        (BAR, 440, 63, 100),
        (BAR + 2 * SIXTEENTH, 220, 65, 86),
        (BAR + 3 * SIXTEENTH, 220, 63, 84),
        (BAR + BEAT, 900, 60, 106),
        (BAR + 2 * BEAT, 440, 58, 92),
        (BAR + 3 * BEAT, 900, 60, 110),
    ];
    for (tick, len, n, v) in notes {
        c.add_note(Note::new(tick, len, n, v)).ok();
    }
    c
}

/// Continue open on the riff (composing tools, phase 1 step 2): its second
/// try, four more bars in Minimal, plays in place of the clip; the roll
/// shows the new bars faintly after yours, each bar's job on the ruler and
/// the chords under it, the drawer Continue's settings and the tries.
pub fn notes_continue_state() -> Result<TimeState, String> {
    continue_sample(false, false)
}

/// [`notes_continue_state`] with a Character (S5 board A): Yearning
/// picked in the drawer, the try made with it, the tool bar's title
/// naming it and its line saying what it does.
pub fn notes_continue_character_state() -> Result<TimeState, String> {
    continue_sample(false, true)
}

/// [`notes_continue_state`] a step on (phase 1 step 3): bar 4 locked, bar
/// 6 picked and tried again, so the third try ("bar 6 again") plays; the
/// ruler marks both, the drawer's button unlocks or locks bar 6 and the
/// tool bar's button tries it again.
pub fn notes_continue_picks_state() -> Result<TimeState, String> {
    continue_sample(true, false)
}

fn continue_sample(picks: bool, yearning: bool) -> Result<TimeState, String> {
    use crate::continuing::{Again, Continuing, Try};
    use crate::editor::compose::{self, Picks};
    use crate::editor::{NoteTool, ToolKind};
    use mpfree_compose::request::{continue_clip, continue_clip_how, Making, OnClip};
    use mpfree_compose::{InstrumentKind, State};
    use mpfree_context::{PartRole, ReadBudget};
    let mut s = playing()?;
    let riff = riff_clip();
    s.project.tracks[CHORDS].slots[1] = Some(riff.clone());
    let mut set = compose::ContinueSet::default();
    if yearning {
        set.character = mpfree_compose::Character::Yearning;
        set.row = compose::CHARACTER_ROW;
    }
    let settings = mpfree_compose::Settings {
        seed: 2,
        ..set.settings(set.q, &[])
    };
    let on = OnClip {
        project: &s.project,
        track: CHORDS,
        slot: 1,
        role: PartRole::Melody,
        others: &[],
        lane_of: &|_, _| None,
        transport_swing: Default::default(),
        meter: Default::default(),
        bpm: BPM,
        budget: ReadBudget::default(),
        row: None,
    };
    let (mut proposal, mut made) =
        continue_clip(&on, &settings, InstrumentKind::OneNote).map_err(|r| r.words())?;
    let mut tries = vec![
        Try {
            style: settings.style.name,
            bars: settings.bars,
            again: None,
        };
        2
    ];
    if picks {
        let how = Making::Bars {
            before: std::sync::Arc::new(made),
            bars: vec![5],
            attempt: 1,
        };
        (proposal, made) = continue_clip_how(&on, &settings, InstrumentKind::OneNote, &how)
            .0
            .map_err(|r| r.words())?;
        tries.push(Try {
            again: Some(Again::Bar(5)),
            ..tries[0]
        });
    }
    let position = Some((tries.len(), tries.len()));
    let mut heard = riff.clone();
    heard.apply(&proposal.edit).map_err(|e| format!("{e:?}"))?;
    let mut state = s.state();
    let new_from = made.to - Tick::from(settings.bars) * BAR;
    state.continuing = Some(std::sync::Arc::new(Continuing {
        track: CHORDS,
        slot: 1,
        state: State::Auditioning,
        made: Some(std::sync::Arc::new(made)),
        clip: Some(std::sync::Arc::new(heard)),
        new_from,
        position,
        heard: true,
        line: None,
        words: None,
        settings,
        tries,
    }));
    let mut editor = crate::editor::Editor::new(CHORDS, 1, &riff);
    let mut tool = NoteTool::new(ToolKind::Continue);
    tool.q = set.q;
    editor.open_tool = Some(tool);
    editor.row = crate::editor::CONTINUE_ROW;
    editor.continuing = set;
    if picks {
        editor.continue_picks = Picks {
            bars: vec![5],
            notes: Vec::new(),
            locked: vec![3],
        };
    }
    state.edit = Some(Box::new(crate::view::EditInfo {
        editor,
        clip: riff.into(),
        rev: 0,
        track_name: "Chords".into(),
        bar: BAR,
        answer: Default::default(),
        locks: Default::default(),
        ghosts: Default::default(),
    }));
    Ok(state)
}

/// Chop open on the two long melody notes (piano roll plan 2B.3), in four
/// pieces: the roll previews the pieces, each with its share of the
/// pressure, and the tool's bar takes the buttons' place.
pub fn notes_tool_state() -> Result<TimeState, String> {
    use crate::editor::{NoteTool, ToolKind};
    let mut state = notes_state(crate::editor::Lens::Grid)?;
    let e = state.edit.as_mut().ok_or("no editor")?;
    e.editor.sel = (0..e.clip.notes.len())
        .filter(|&i| !e.clip.notes[i].touch.is_none())
        .collect();
    let mut chop = NoteTool::new(ToolKind::Chop);
    chop.q[0] = 4;
    e.editor.open_tool = Some(chop);
    e.editor.row = crate::editor::CHOP_ROW;
    Ok(state)
}

/// The keys clip with Pads write on (piano roll plan 1C.2): the cursor
/// on the second bar's last beat, where a pad pressed writes, nothing
/// picked.
pub fn notes_pads_state() -> Result<TimeState, String> {
    let mut state = notes_state(crate::editor::Lens::Grid)?;
    let e = state.edit.as_mut().ok_or("no editor")?;
    e.editor.pads_write = true;
    e.editor.cursor = BAR + 3 * BEAT;
    e.editor.sel.clear();
    Ok(state)
}

/// The note editor on the drums (piano roll plan 4.2): a bar of kick,
/// snare, closed and open hats, a clap and a rim shot on the kit's first
/// pads, a row for each pad, which the app names after the kit's sounds.
pub fn notes_drums_state() -> Result<TimeState, String> {
    let mut s = playing()?;
    let mut beat = Clip::new("Boom bap", BAR);
    // (tick, pad, velocity): kick, snare, closed hat, open hat, clap, rim.
    let mut hits: Vec<(Tick, u8, u8)> = vec![
        (0, 0, 120),
        (720, 0, 96),
        (2160, 0, 110),
        (960, 1, 118),
        (2880, 1, 122),
        (2880, 4, 90),
        (3360, 3, 100),
        (3600, 5, 64),
    ];
    hits.extend(
        (0..7)
            .map(|k| k * 480)
            .map(|t| (t, 2, if t % 960 == 0 { 100 } else { 72 })),
    );
    for (tick, pad, velocity) in hits {
        beat.add_note(Note::new(tick, SIXTEENTH, pad, velocity))
            .map_err(|e| format!("beat: {e:?}"))?;
    }
    s.project.tracks[DRUMS].slots[0] = Some(beat.clone());
    let mut state = s.state();
    state.edit = Some(Box::new(crate::view::EditInfo {
        editor: crate::editor::Editor::new(DRUMS, 0, &beat),
        clip: beat.into(),
        rev: 0,
        track_name: "Drums".into(),
        bar: BAR,
        answer: Default::default(),
        locks: Default::default(),
        ghosts: Default::default(),
    }));
    Ok(state)
}

/// The note editor on "Keys" with two other clips faintly behind it
/// (piano roll plan 4.1): "Pluck", the stab track's line on the same row,
/// which plays with it, and "Keys take 1", an earlier take of the chords
/// on another row, shown for reference. The clips hold notes, so the Show
/// menu offers those, the bass track's two lines (Sub line plays now) and
/// none of the pad hits.
pub fn notes_ghosts_state() -> Result<TimeState, String> {
    const BASS: usize = 2;
    let notes = |name: &str, length: Tick, notes: &[(Tick, Tick, u8)]| -> Result<Clip, String> {
        let mut c = Clip::new(name, length);
        for &(tick, len, note) in notes {
            c.add_note(Note::new(tick, len, note, 96))
                .map_err(|e| format!("{name}: {e:?}"))?;
        }
        c.holds = mpfree_seq::Holds::Notes;
        Ok(c)
    };
    let mut s = playing()?;
    let keys = keys_clip();
    // (tick, length, note): an answer between the chords and the tune.
    let pluck = notes(
        "Pluck",
        2 * BAR,
        &[
            (2 * SIXTEENTH, SIXTEENTH, 63),
            (5 * SIXTEENTH, SIXTEENTH, 65),
            (10 * SIXTEENTH, SIXTEENTH, 68),
            (14 * SIXTEENTH, SIXTEENTH, 67),
            (BAR + 2 * SIXTEENTH, SIXTEENTH, 65),
            (BAR + 5 * SIXTEENTH, SIXTEENTH, 67),
            (BAR + 10 * SIXTEENTH, SIXTEENTH, 70),
            (BAR + 13 * SIXTEENTH, SIXTEENTH, 68),
        ],
    )?;
    // The chords as first played: a bar each, voiced lower.
    let take = notes(
        "Keys take 1",
        2 * BAR,
        &[
            (0, BAR - 40, 55),
            (0, BAR - 40, 60),
            (0, BAR - 40, 63),
            (BAR, BAR - 40, 58),
            (BAR, BAR - 40, 62),
            (BAR, BAR - 40, 65),
        ],
    )?;
    let bass = notes(
        "Bass line",
        2 * BAR,
        &[
            (0, BEAT, 36),
            (2 * BEAT, BEAT, 32),
            (BAR, BEAT, 34),
            (BAR + 2 * BEAT, BEAT, 31),
        ],
    )?;
    let p = &mut s.project;
    p.tracks[CHORDS].slots[1] = Some(keys.clone());
    p.tracks[CHORDS].slots[3] = Some(take.clone());
    p.tracks[STAB].slots[1] = Some(pluck.clone());
    p.tracks[BASS].slots[1] = Some(bass);
    if let Some(sub) = p.tracks[BASS].slots[0].as_mut() {
        sub.holds = mpfree_seq::Holds::Notes;
    }
    let mut state = s.state();
    let mut editor = crate::editor::Editor::new(CHORDS, 1, &keys);
    editor.show = vec![(STAB, 1), (CHORDS, 3)];
    let ghost = |track: usize, slot: usize, clip: Clip, name: &str| crate::view::Ghost {
        track,
        slot,
        clip: clip.into(),
        track_name: name.into(),
    };
    state.edit = Some(Box::new(crate::view::EditInfo {
        editor,
        clip: keys.into(),
        rev: 0,
        track_name: "Chords".into(),
        bar: BAR,
        answer: Default::default(),
        locks: Default::default(),
        ghosts: vec![
            ghost(STAB, 1, pluck, "Stab"),
            ghost(CHORDS, 3, take, "Chords"),
        ],
    }));
    Ok(state)
}

/// The note editor on "Keys" with the two melody notes that carry
/// pressure picked (piano roll plan 3A.1): with the Pressure lane on,
/// it draws their swells as lines under them.
pub fn notes_pressure_state() -> Result<TimeState, String> {
    let mut state = notes_state(crate::editor::Lens::Grid)?;
    let e = state.edit.as_mut().ok_or("no editor")?;
    e.editor.sel = (0..e.clip.notes.len())
        .filter(|&i| !e.clip.notes[i].touch.is_none())
        .collect();
    Ok(state)
}

/// The setting the locks sample's lane shows: a filter cutoff, as a
/// synth's knob turns it, a semitone a detent from 20 Hz to open.
pub fn cutoff_setting(addr: mpfree_seq::mpfree_param::RelAddr) -> crate::editor::LockSetting {
    let words = |v: f32| match v {
        v if v >= 20_000.0 => "open".to_string(),
        v if v >= 1000.0 => format!("{:.1} kHz", v / 1000.0),
        v => format!("{v:.0} Hz"),
    };
    let mut values: Vec<(f32, String)> = (0..)
        .map(|k| 20.0 * (k as f32 / 12.0).exp2())
        .take_while(|&v| v < 20_000.0)
        .map(|v| (v, words(v)))
        .collect();
    values.push((20_000.0, words(20_000.0)));
    crate::editor::LockSetting {
        addr,
        name: "Cutoff".into(),
        own: 20_000.0,
        values,
        centred: false,
    }
}

/// The note editor on "Keys" with Cutoff locks on three steps (piano
/// roll plan 3A.2), its lock lane showing: the second chord's three
/// notes share their step's lock and are picked, so the lane says so; the
/// notes the locks reach are marked, and a melody note with a 1:2
/// condition is too. The steps with no lock play the filter open.
pub fn notes_locks_state() -> Result<TimeState, String> {
    use mpfree_seq::mpfree_param::{OnTrack, RelAddr};
    let mut state = notes_state(crate::editor::Lens::Grid)?;
    let e = state.edit.as_mut().ok_or("no editor")?;
    let cutoff = RelAddr::new(OnTrack::Inst, 3);
    let mut clip = (*e.clip).clone();
    let at = |c: &Clip, tick: Tick, note: u8| {
        c.notes
            .iter()
            .position(|n| (n.tick, n.note) == (tick, note))
            .ok_or(format!("no note {note} at {tick}"))
    };
    let set = [
        (at(&clip, 0, 60)?, 640.0),
        (at(&clip, 2 * BEAT + 12, 56)?, 2560.0),
        (at(&clip, 3 * BEAT + 5, 72)?, 1280.0),
    ];
    clip.apply(&mpfree_seq::notes::lock_notes(&clip, cutoff, &set))
        .map_err(|e| format!("{e:?}"))?;
    let odd = at(&clip, BEAT + 14, 75)?;
    clip.notes[odd].cond = mpfree_seq::Cond::Every { a: 1, b: 2 };
    e.editor.sel = (0..clip.notes.len())
        .filter(|&i| (2 * BEAT..3 * BEAT).contains(&clip.notes[i].tick) && clip.notes[i].len > BEAT)
        .collect();
    e.clip = clip.into();
    e.locks = std::sync::Arc::new(vec![cutoff_setting(cutoff)]);
    Ok(state)
}

/// A bar of drums with a step's every setting in use: a nudged hat, a
/// ghost snare at 50 %, a 1:2 hat, a ratchet, a FILL open hat and a lock
/// on the second snare (what the tracker shows).
pub fn tracker_clip() -> Clip {
    use mpfree_seq::mpfree_param::{OnTrack, RelAddr};
    use mpfree_seq::Cond;
    let mut c = clip(
        "Boom bap",
        BAR,
        &[
            (0, 0, 120),
            (1680, 0, 100),
            (960, 1, 110),
            (2880, 1, 115),
            (3600, 1, 40),
            (2160, 3, 90),
        ],
    );
    for k in 0..8 {
        let mut n = Note::new(k * 480, SIXTEENTH, 2, if k % 2 == 0 { 84 } else { 62 });
        match k {
            3 => n.tick += 18,
            5 => n.cond = Cond::Every { a: 1, b: 2 },
            7 => n.ratchet = 3,
            _ => {}
        }
        c.add_note(n).expect("sample hat in range");
    }
    for n in c.notes.iter_mut() {
        match (n.note, n.tick) {
            (1, 3600) => n.chance = 50,
            (3, _) => n.cond = Cond::Fill,
            _ => {}
        }
    }
    let lock = mpfree_seq::Lock::new(2880, Some(1), RelAddr::new(OnTrack::Pad(1), 3), 0.4);
    c.locks.push(lock);
    c.check().expect("sample clip in range");
    c
}

/// The note editor's tracker on [`tracker_clip`] in place of Drums' first
/// clip while it plays, the second snare picked and the Euclidean rows on
/// show. The app's `--preview` draws it for the manual.
pub fn tracker_state() -> Result<TimeState, String> {
    let mut s = playing()?;
    let drums = tracker_clip();
    s.project.tracks[DRUMS].slots[0] = Some(drums.clone());
    let mut state = s.state();
    let mut editor = crate::editor::Editor::new(DRUMS, 0, &drums);
    editor.sel = drums
        .notes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.note == 1 && n.tick == 2880)
        .map(|(i, _)| i)
        .collect();
    editor.sound = 1;
    editor.lens = crate::editor::Lens::Tracker;
    editor.row = crate::editor::EUCLID_ROW + 1;
    state.edit = Some(Box::new(crate::view::EditInfo {
        editor,
        clip: drums.into(),
        rev: 0,
        track_name: "Drums".into(),
        bar: BAR,
        answer: Default::default(),
        locks: Default::default(),
        ghosts: Default::default(),
    }));
    Ok(state)
}

/// The pad player's Play page with the bar under the header.
fn play_sample(fonts: Fonts, s: &Session) -> Result<Sample, String> {
    let mut ui = app_ui(fonts, "Play", PLAY_HINT);
    let mut pads = PadGrid::new(16, 4);
    let names = [
        "Kick",
        "Snare",
        "Closed Hat",
        "Open Hat",
        "Sub",
        "Glitch",
        "Stab",
        "Chord",
        "Chord 2",
        "Dust",
        "Riser",
        "Wash",
        "Clank",
        "",
        "",
        "",
    ];
    let color = Color::from_pad([0x00, 0x2A, 0x3F]);
    for (i, n) in names.iter().enumerate() {
        pads.pads[i].label = n.to_string();
        pads.pads[i].color = (!n.is_empty()).then_some(color);
    }
    pads.hit(0, 118);
    pads.hit(6, 96);
    pads.release(6);
    pads.pads[6].level = 0.5;
    ui.add("pads", pads);
    let mut kits = ListView::new("Kits");
    for (k, d) in [
        ("Classic House", "16 pads"),
        ("Boom Bap", "13 pads"),
        ("Trap", "16 pads"),
        ("Lo-Fi", "9 pads"),
        ("Techno", "16 pads"),
        ("Afrobeat", "14 pads"),
    ] {
        kits.items.push(ListItem {
            text: k.into(),
            detail: d.into(),
            color: Some(color),
        });
    }
    kits.selected = 1;
    ui.add("kits", kits);
    let mut level = Fader::new("Level · Q-Link 1");
    level.value = 0.79;
    level.text = "-18.0 dB".into();
    ui.add("level", level);
    let mut timing = StatList::new("Timing");
    timing.set(vec![
        ("Period".into(), "128 frames · 2.9 ms".into(), Tone::Normal),
        (
            "Audio thread".into(),
            "0.41 ms mean · 0.88 ms max".into(),
            Tone::Good,
        ),
        ("Underruns".into(), "0".into(), Tone::Good),
        ("Voices".into(), "6".into(), Tone::Normal),
    ]);
    ui.add("timing", timing);
    let layout = with_transport(APP_PLAY_LAYOUT).ok_or("the Play layout has no header")?;
    let mut page = TimePage::add_to(&mut ui)?;
    ui.add_page("Play", &layout)?;
    ui.show("Play")?;
    let mut state = s.state();
    state.chips = vec![arp_chip()];
    page.update(&mut ui, &state);
    Ok(Sample {
        name: "play",
        about: "the Play page with the transport bar under the header, playing at 5.3",
        ui,
        state,
    })
}

/// Every sample screen, with `fonts` called once per screen.
pub fn samples(fonts: impl Fn() -> Fonts) -> Result<Vec<Sample>, String> {
    let playing = playing()?;
    let mut out = vec![
        play_sample(fonts(), &playing)?,
        clips_sample(
            "clips-playing",
            "Clips page: playing at 5.3, the chords tapped and waiting for bar 6",
            fonts(),
            &playing,
            0,
        )?,
        clips_sample(
            "clips-recording",
            "Clips page: replace recording into the stab at 4.2, take 1 in progress",
            fonts(),
            &recording()?,
            0,
        )?,
        clips_sample(
            "clips-undo",
            "Clips page: overdub take 2 undone, Redo brings it back",
            fonts(),
            &undone()?,
            0,
        )?,
        clips_sample(
            "clips-count-in",
            "Clips page: counting in a bar before recording from bar 1 (0.3)",
            fonts(),
            &counting_in()?,
            0,
        )?,
        clips_sample(
            "clips-stopped",
            "Clips page: stopped at 1.1, nothing launched or armed",
            fonts(),
            &stopped()?,
            0,
        )?,
        clips_sample(
            "clips-capture-ready",
            "Clips page: a chord riff played over bars 3 and 4 without recording; \
                      Capture would keep 2 bars",
            fonts(),
            &capture_ready()?,
            0,
        )?,
        clips_sample(
            "clips-captured",
            "Clips page: Capture kept the riff as a new clip in Chords / 2, playing on in \
                      its place",
            fonts(),
            &captured()?,
            0,
        )?,
        song_sample(fonts())?,
        song_audio_sample(fonts())?,
        song_audio_blocks_sample(fonts())?,
    ];
    // A narrow look at the bar alone, while recording.
    let rec = recording()?;
    let mut ui = Ui::new(Theme::dark(), fonts(), SCREEN.0, 68);
    ui.add(TRANSPORT, TransportBar::new());
    ui.add_page("bar", "transport")?;
    let mut state = rec.state();
    state.chips = vec![arp_chip()];
    TimePage::default().update(&mut ui, &state);
    out.push(Sample {
        name: "bar-recording",
        about: "the transport bar alone, recording",
        ui,
        state,
    });
    Ok(out)
}

/// The Lead's eight-bar Solo for Add a part's screens: a tune in C minor
/// on the chord tones of Cm, Cm, A♭, B♭, Cm and A♭, resting in bars 7 and
/// 8 (where the row's chords are only suggested).
fn solo_tune() -> Clip {
    let bars: [[u8; 4]; 6] = [
        [72, 75, 79, 75],
        [79, 75, 72, 75],
        [68, 72, 75, 72],
        [70, 74, 77, 74],
        [72, 79, 75, 72],
        [75, 72, 68, 72],
    ];
    let notes: Vec<(Tick, u8, u8)> = bars
        .iter()
        .enumerate()
        .flat_map(|(b, pitches)| {
            pitches
                .iter()
                .enumerate()
                .map(move |(i, &n)| (b as Tick * BAR + i as Tick * BEAT, n, 100))
        })
        .collect();
    let mut c = clip("Solo", 8 * BAR, &notes);
    c.holds = mpfree_seq::Holds::Notes;
    for n in &mut c.notes {
        n.len = BEAT * 3 / 4;
    }
    c
}

/// The busy jam ([`busy_project`]) with row 5 playing from bar 1 (Break,
/// Ride and Solo: Drums, Hats and Lead, the Solo a tune in C minor), at
/// 2.3: Add a part's screens (composing tools S2.6).
fn row5_playing() -> Result<Session, String> {
    let mut p = busy_project();
    p.tracks[4].slots[ADD_ROW] = Some(solo_tune());
    let launches = [(DRUMS, ADD_ROW), (1, ADD_ROW), (4, ADD_ROW)]
        .iter()
        .map(|&(track, slot)| Launch {
            track,
            slot,
            origin: 0,
            start: 0,
        })
        .collect();
    let mut s = Session::new(p, launches, None)?;
    s.command_at(0, Command::Play)?;
    s.run_to(BAR + 2 * BEAT)?;
    Ok(s)
}

/// The row Add a part's screens are on: row 5.
pub const ADD_ROW: usize = 4;
/// Where on the row the chord picker opens and the Chords tab's chord was
/// changed: bar 4, the B♭ the Solo plays.
const PICK_BAR: Tick = 3 * BAR;

/// The chip of `line` that holds [`PICK_BAR`] (the last chip on a shorter
/// line).
pub fn picked_chip(line: &mpfree_context::chord::ChordTimeline) -> usize {
    let spans = line.spans();
    spans
        .iter()
        .position(|s| s.start <= PICK_BAR && PICK_BAR < s.end())
        .unwrap_or(spans.len().saturating_sub(1))
}
/// The busy jam's Bass track.
pub const ADD_BASS: usize = 2;

/// Row `row`'s clips but track `but`'s, as the time engine has Add a part
/// read them: drums as drums, the rest as one harmony.
fn row_reads(p: &Project, row: usize, but: usize) -> Vec<mpfree_compose::ToRead> {
    use mpfree_context::PartRole;
    p.tracks
        .iter()
        .enumerate()
        .filter(|&(t, tr)| t != but && tr.kind == mpfree_seq::TrackKind::Notes)
        .filter_map(|(t, tr)| {
            let c = tr.slots.get(row)?.as_ref()?;
            if c.looped.is_some() || c.notes.is_empty() {
                return None;
            }
            let role = match c.holds {
                mpfree_seq::Holds::Hits => PartRole::Drums,
                mpfree_seq::Holds::Notes => PartRole::Chords,
            };
            Some(mpfree_compose::ToRead {
                track: t,
                slot: row,
                role,
            })
        })
        .collect()
}

/// A part for the Bass track's cell in row 5 with `pick`'s choices, made
/// as the time engine's composer thread makes it (its second ask), and
/// the row's chords as it hears them.
fn row5_part(
    s: &Session,
    pick: &crate::add_view::AddPick,
) -> Result<(mpfree_compose::PartMade, mpfree_compose::RowChords), String> {
    use mpfree_compose::{AddAsk, AddPartJob, InstrumentKind};
    let t = s.state().transport;
    let ask = AddAsk {
        row: ADD_ROW,
        parts: row_reads(&s.project, ADD_ROW, ADD_BASS),
        lanes: Default::default(),
        part: pick.part,
        relation: pick.relation,
        kind: InstrumentKind::Bass,
        pads: [None; 4],
        swing: t.swing,
        transport_swing: t.swing,
        meter: t.meter,
        bpm: t.bpm,
        budget: mpfree_context::ReadBudget::default(),
        settings: pick.settings(),
        bars: pick.bars,
        attempt: 1,
        phrase: pick.phrase_of().map(|(_, p)| p),
    };
    let job = AddPartJob::new(&s.project, ask);
    let made = job.run().result.map_err(|e| format!("add a part: {e:?}"))?;
    Ok((made, job.hear()))
}

/// Add a part open on the busy jam's row 5 (composing tools S2.6, board
/// A): Break, Ride and Solo heard, Bass on Support made for the Bass
/// track's empty cell, its second try playing there, and the row's chords
/// heard under the row. Made here as the time engine makes it, with a
/// fixed try, so the screen is the same every time. With it, the page's
/// choices, as [`crate::add_view::AddPick::open`] makes them.
pub fn add_part_state() -> Result<(TimeState, crate::add_view::AddPick), String> {
    use crate::adding::{AddState, Adding};
    let s = row5_playing()?;
    let mut st = s.state();
    st.scene_focus = ADD_ROW;
    let pick = crate::add_view::AddPick::open(&st, ADD_ROW, Some(ADD_BASS))?;
    let (made, heard) = row5_part(&s, &pick)?;
    st.adding = Some(std::sync::Arc::new(Adding {
        row: ADD_ROW,
        track: ADD_BASS,
        state: AddState::Playing,
        made: Some(std::sync::Arc::new(made)),
        position: Some((2, 2)),
        line: None,
        picked: None,
        words: None,
        part: pick.part,
        relation: pick.relation,
    }));
    st.row_chords = Some((ADD_ROW, std::sync::Arc::new(heard)));
    Ok((st, pick))
}

/// Add a part on New track (composing tools S2.6b): the panel on row 5
/// with its Track row stepped to New track, on the Bass synth, a Bass try
/// playing with the row on the track Keep would add.
pub fn add_part_new_track_state() -> Result<(TimeState, crate::add_view::AddPick), String> {
    use crate::adding::{AddState, Adding};
    let s = row5_playing()?;
    let mut st = s.state();
    st.scene_focus = ADD_ROW;
    let mut pick = crate::add_view::AddPick::open(&st, ADD_ROW, Some(ADD_BASS))?;
    for _ in 0..st.tracks.len() {
        if pick.new_track {
            break;
        }
        pick.step(&st, crate::add_view::ROW_TRACK, 1)?;
    }
    if !pick.new_track {
        return Err("New track isn't on the Track row".into());
    }
    let (made, heard) = row5_part(&s, &pick)?;
    st.adding = Some(std::sync::Arc::new(Adding {
        row: ADD_ROW,
        track: pick.track,
        state: AddState::Playing,
        made: Some(std::sync::Arc::new(made)),
        position: Some((1, 1)),
        line: None,
        picked: None,
        words: None,
        part: pick.part,
        relation: pick.relation,
    }));
    st.row_chords = Some((ADD_ROW, std::sync::Arc::new(heard)));
    Ok((st, pick))
}

/// Add a part on a phrase (P2, board H): the panel on row 5 with its
/// Style row run on past Jazz-ish to MPFree's Night drive, the bass line
/// played over the row's chords, its second try playing in the Bass
/// track's cell.
pub fn add_part_phrase_state() -> Result<(TimeState, crate::add_view::AddPick), String> {
    use crate::adding::{AddState, Adding};
    let s = row5_playing()?;
    let mut st = s.state();
    st.scene_focus = ADD_ROW;
    let mut pick = crate::add_view::AddPick::open(&st, ADD_ROW, Some(ADD_BASS))?;
    pick.phrases = mpfree_arp_ui::PhraseList::built_in().phrases.into();
    let night = pick
        .phrases
        .iter()
        .position(|(n, _)| n == "Night drive")
        .ok_or("no Night drive")?;
    pick.phrase = Some(night);
    let (made, heard) = row5_part(&s, &pick)?;
    st.adding = Some(std::sync::Arc::new(Adding {
        row: ADD_ROW,
        track: ADD_BASS,
        state: AddState::Playing,
        made: Some(std::sync::Arc::new(made)),
        position: Some((2, 2)),
        line: None,
        picked: None,
        words: None,
        part: pick.part,
        relation: pick.relation,
    }));
    st.row_chords = Some((ADD_ROW, std::sync::Arc::new(heard)));
    Ok((st, pick))
}

/// The Chords tab later (composing tools S2.6, board C): the Bass part
/// kept in row 5, then bar 4's B♭ set to A♭7 from the tab,
/// which leaves the Bass and the Solo off the new chords (ticked, with
/// how many notes would move).
pub fn chords_tab_state() -> Result<TimeState, String> {
    use mpfree_context::chord::Chord;
    let mut s = row5_playing()?;
    let st = s.state();
    let pick = crate::add_view::AddPick::open(&st, ADD_ROW, Some(ADD_BASS))?;
    let (made, heard) = row5_part(&s, &pick)?;
    s.project.tracks[ADD_BASS].slots[ADD_ROW] = Some(made.clip);
    let mut st = s.state();
    st.scene_focus = ADD_ROW;
    let chip = picked_chip(&heard.line);
    let ab7 = Chord::new(8, crate::add_view::KIND_QUALITIES[2]);
    let line = crate::add_view::use_chord(&heard.line, chip, ab7).ok_or("no chord to change")?;
    st.unfit = vec![crate::adding::Unfit {
        row: ADD_ROW,
        before: heard.line.clone(),
        clips: vec![(ADD_BASS, 3), (4, 2)],
    }]
    .into();
    st.row_chords = Some((
        ADD_ROW,
        std::sync::Arc::new(mpfree_compose::RowChords {
            line,
            heard: heard.heard,
        }),
    ));
    Ok(st)
}

/// The Sequencer's note editor on an empty two-bar clip in the Bass
/// track's row 5 (composing tools S2.6, board D): its card offers Add a
/// part with the row's Break, Ride and Solo.
pub fn empty_clip_state() -> Result<TimeState, String> {
    let mut s = row5_playing()?;
    let mut empty = Clip::new("Clip 5", 2 * BAR);
    empty.holds = mpfree_seq::Holds::Notes;
    s.project.tracks[ADD_BASS].slots[ADD_ROW] = Some(empty.clone());
    let mut state = s.state();
    state.armed = Some(ADD_BASS);
    let editor = crate::editor::Editor::new(ADD_BASS, ADD_ROW, &empty);
    state.edit = Some(Box::new(crate::view::EditInfo {
        editor,
        clip: empty.into(),
        rev: 0,
        track_name: "Bass".into(),
        bar: BAR,
        answer: Default::default(),
        locks: Default::default(),
        ghosts: Default::default(),
    }));
    Ok(state)
}

/// Make a style open (composing step 6, board D): a made-up two-bar drum
/// groove on Clip 1 of a Drums track, read as Keep would keep it: kick on
/// 1 and the "and" of 3, the snare loud on 2 and 4, 1/16 hats with their
/// 8ths accented and soft ghost notes between, swung 56 %.
pub fn style_sheet_sample() -> crate::style_sheet::StyleSheet {
    use mpfree_compose::{read_clip, realize::PlainNote, ClipStyle};
    use mpfree_transport::time::{Meter, Swing, PPQ};
    let meter = Meter::default();
    let bar = meter.bar_ticks();
    let step = Tick::from(PPQ) / 4;
    let mut notes: Vec<PlainNote> = Vec::new();
    let mut hit = |at: Tick, pitch: u8, velocity: u8| {
        notes.push(PlainNote {
            at,
            len: step / 2,
            pitch,
            velocity,
        })
    };
    for b in 0..2 {
        let start = b * bar;
        for s in 0..16 {
            let at = start + Tick::from(s) * step;
            match s {
                0 | 10 => hit(at, 36, 110),
                4 | 12 => hit(at, 38, 118),
                _ => {}
            }
            let hat = match s % 4 {
                0 => 96,
                2 => 84,
                _ => 52,
            };
            hit(at, 42, hat);
        }
    }
    notes.sort_by_key(|n| n.at);
    let swing = Swing::new(PPQ / 4, 56).unwrap_or_default();
    let made = read_clip(&notes, 2 * bar, meter, 96.0, swing)
        .map(|read| ClipStyle::new("Clip 1", read, meter))
        .expect("a groove with notes reads");
    crate::style_sheet::StyleSheet {
        track: 0,
        slot: 0,
        made,
        from: "From Clip 1 · Drums · 2 bars".to_string(),
        hits: notes.iter().map(|n| (n.at, n.velocity)).collect(),
        length: 2 * bar,
        bar,
        row: crate::style_sheet::NAME_ROW,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{cell_of, take_items};
    use crate::widgets::CellState;
    use mpfree_play::anchored;
    use mpfree_transport::State;
    use mpfree_ui::SoftCanvas;

    fn notes(s: &Session, track: usize) -> Vec<(Tick, u8)> {
        let c = s.project.clip(track, 0).unwrap();
        let mut v: Vec<(Tick, u8)> = c.notes.iter().map(|n| (n.tick, n.note)).collect();
        v.sort();
        v
    }

    #[test]
    fn every_sample_draws() {
        let samples = samples(Fonts::embedded).unwrap();
        assert_eq!(samples.len(), 12);
        for mut s in samples {
            let (w, h) = s.ui.size();
            let mut c = SoftCanvas::new(w, h);
            assert!(!s.ui.paint(&mut c).is_empty(), "{}", s.name);
            // Not a blank screen: the bar's surface colour is in it.
            let surface = Theme::dark().surface;
            assert!((0..h).any(|y| c.pixel(w / 2, y) == surface), "{}", s.name);
        }
    }

    #[test]
    fn the_song_sample_shows_rows_as_a_song() {
        let s = samples(Fonts::embedded).unwrap();
        let song = s.iter().find(|s| s.name == "clips-song").unwrap();
        let st = &song.state;
        assert!(st.song_view && st.song_on);
        let names: Vec<&str> = st.markers.iter().map(|m| m.1.as_str()).collect();
        assert_eq!(names[..2], ["Scene 1", "Verse"]);
        assert!(st.placements[2].unique);
        assert_eq!(
            song.ui.layout_of(CLIPS_PAGE),
            Some(crate::view::SONG_LAYOUT)
        );
    }

    #[test]
    fn the_playing_sample_queues_the_chords_for_the_next_bar() {
        let s = playing().unwrap();
        let st = s.state();
        assert_eq!(st.transport.state, State::Playing);
        // A period is about 4 ticks, so the run stops just past 5.3.240.
        let at = st.transport.meter.position(st.transport.position);
        assert_eq!((at.bar, at.beat), (5, 3));
        assert!((240..245).contains(&at.tick), "{at}");
        let chords = cell_of(&st, CHORDS, 0);
        assert_eq!(
            (chords.state, chords.foot.as_str()),
            (CellState::Queued, "from 6.1")
        );
        assert_eq!(cell_of(&st, DRUMS, 0).state, CellState::Playing);
        let glitch = cell_of(&st, 3, 0);
        assert_eq!(
            (glitch.state, glitch.detail.as_str()),
            (CellState::Playing, "stops 6.1"),
            "the glitch started at bar 3"
        );
        assert_eq!(
            st.cues,
            vec![
                Cue {
                    track: CHORDS,
                    slot: Some(0)
                },
                Cue {
                    track: 3,
                    slot: None
                }
            ]
        );
    }

    #[test]
    fn cues_start_on_the_bar_line_and_become_launches() {
        let mut s = playing().unwrap();
        // A stop for the drums, cued in the same bar.
        s.act(TimeAction::StopClip(DRUMS)).unwrap();
        assert!(
            s.launches.iter().any(|l| l.track == DRUMS),
            "the drums play to the bar line"
        );
        assert_eq!(cell_of(&s.state(), DRUMS, 0).detail, "stops 6.1");
        s.run_to(5 * BAR - 8).unwrap();
        assert_eq!(s.cues.len(), 3);
        s.run_to(5 * BAR + 8).unwrap();
        assert!(s.cues.is_empty());
        assert!(!s.launches.iter().any(|l| l.track == DRUMS || l.track == 3));
        let chords = s
            .launches
            .iter()
            .find(|l| l.track == CHORDS)
            .copied()
            .unwrap();
        let clip = s.project.clip(CHORDS, 0).unwrap();
        assert_eq!(chords.origin, anchored(5 * BAR, clip.length, clip.speed));
        assert_eq!(cell_of(&s.state(), CHORDS, 0).state, CellState::Playing);
        // Taken back before its bar line, a cue does nothing.
        s.act(TimeAction::StopClip(CHORDS)).unwrap();
        assert_eq!(
            s.cues,
            vec![Cue {
                track: CHORDS,
                slot: None
            }]
        );
        s.act(TimeAction::Uncue(CHORDS)).unwrap();
        s.run_to(6 * BAR + 8).unwrap();
        assert_eq!(s.launches.iter().find(|l| l.track == CHORDS), Some(&chords));
    }

    #[test]
    fn the_recording_sample_really_replaced_the_stab() {
        let s = recording().unwrap();
        let st = s.state();
        assert_eq!(st.transport.state, State::Recording);
        assert_eq!(cell_of(&st, STAB, 0).state, CellState::Recording);
        // The riff's five notes were erased in bar 3; four clanks of bar 3
        // and the one of bar 4 were recorded.
        assert_eq!(st.done.len(), 1);
        assert_eq!((st.done[0].added, st.done[0].erased), (5, 5));
        assert_eq!(
            notes(&s, STAB),
            vec![(0, 12), (480, 12), (960, 12), (1680, 12), (2880, 12)]
        );
    }

    #[test]
    fn the_undo_sample_took_back_take_two_only() {
        let s = undone().unwrap();
        let st = s.state();
        assert_eq!(st.transport.state, State::Playing);
        assert!(st.transport.overdub);
        let rows: Vec<(u32, usize, usize)> = st
            .done
            .iter()
            .map(|r| (r.number, r.added, r.erased))
            .collect();
        assert_eq!(rows, vec![(1, 5, 5)]);
        let rows: Vec<(u32, usize, usize)> = st
            .undone
            .iter()
            .map(|r| (r.number, r.added, r.erased))
            .collect();
        assert_eq!(rows, vec![(2, 4, 0)]);
        // The clip is as take 1 left it.
        assert_eq!(
            notes(&s, STAB),
            vec![(0, 12), (480, 12), (960, 12), (1680, 12), (2880, 12)]
        );
        let (items, current) = take_items(&st);
        assert_eq!(
            items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(),
            vec!["Take 2 · Stab / 1", "Take 1 · Stab / 1"]
        );
        assert_eq!(current, Some(1));
    }

    #[test]
    fn the_capture_samples_keep_the_riff_as_a_clip_in_its_place() {
        let ready = capture_ready().unwrap();
        let st = ready.state();
        assert_eq!(st.capture.as_deref(), Some("2 bars"));
        assert!(st.done.is_empty(), "nothing was recorded");
        let s = captured().unwrap();
        let st = s.state();
        assert_eq!(st.capture, None, "kept: nothing new");
        let c = s.project.clip(CHORDS, 1).unwrap();
        assert_eq!((c.name.as_str(), c.length), ("Capture 1", 2 * BAR));
        let ticks: Vec<(Tick, u8)> = c.notes.iter().map(|n| (n.tick, n.note)).collect();
        assert_eq!(ticks, CHORD_RIFF.to_vec());
        // Bar 5 is the riff's first bar again: it is playing, a pass in.
        let cell = cell_of(&st, CHORDS, 1);
        assert_eq!(cell.state, CellState::Playing);
        assert!(cell.progress < 0.2, "{}", cell.progress);
        let (items, _) = take_items(&st);
        assert_eq!(
            (items[0].title.as_str(), items[0].detail.as_str()),
            ("Capture 1 · Chords / 2", "new +6")
        );
    }

    #[test]
    fn capture_over_a_playing_clip_adds_to_it_and_undo_takes_it_back() {
        // The stab's one-bar riff plays from bar 1; the pads play two
        // clanks in bar 3 without recording.
        let mut s = Session::new(project(), launches(), Some(STAB)).unwrap();
        s.command_at(0, Command::Play).unwrap();
        s.pad(2 * BAR + 480, SIXTEENTH, 12, 100);
        s.pad(2 * BAR + 2880, SIXTEENTH, 12, 100);
        s.run_to(3 * BAR + 100).unwrap();
        let k = s.keep().unwrap();
        assert_eq!((k.slot, k.notes, k.launch), (0, 2, None));
        let mut want: Vec<(Tick, u8)> = STAB_RIFF.iter().map(|&(t, n, _)| (t, n)).collect();
        want.extend([(480, 12), (2880, 12)]);
        want.sort();
        assert_eq!(notes(&s, STAB), want);
        assert_eq!(s.keep(), Err("nothing new to capture".to_string()));
        s.undo().unwrap();
        assert_eq!(notes(&s, STAB).len(), STAB_RIFF.len());
        assert_eq!(
            s.state().capture.as_deref(),
            Some("1 bar"),
            "an undone capture can be kept again"
        );
        // A new clip undone takes its launch with it.
        s.armed = Some(CHORDS);
        let k = s.keep().unwrap();
        assert_eq!(k.slot, 1);
        assert!(s.launches.iter().any(|l| l.track == CHORDS));
        s.undo().unwrap();
        assert!(s.project.clip(CHORDS, 1).is_none());
        assert!(!s.launches.iter().any(|l| l.track == CHORDS));
    }

    #[test]
    fn the_count_in_sample_counts_in() {
        let st = counting_in().unwrap().state();
        assert_eq!(st.transport.state, State::CountIn);
        let at = st.transport.meter.position(st.transport.position);
        assert_eq!((at.bar, at.beat), (0, 3));
        assert!(at.tick < 5, "{at}");
        assert_eq!(
            cell_of(&st, STAB, 0).state,
            CellState::Queued,
            "nothing plays before bar 1"
        );
    }
}
