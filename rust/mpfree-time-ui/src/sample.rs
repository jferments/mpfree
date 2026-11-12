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
use mpfree_seq::{Clip, Note, Project, Track};
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
        project: Project,
        launches: Vec<Launch>,
        armed: Option<usize>,
    ) -> Result<Session, String> {
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
    let mut ui = app_ui(fonts, "Clips", CLIPS_HINT);
    let mut page = TimePage::add_to(&mut ui)?;
    ui.show(CLIPS_PAGE)?;
    page.first_row = first_row;
    let mut state = s.state();
    state.chips = vec![arp_chip()];
    page.update(&mut ui, &state);
    Ok(Sample {
        name,
        about,
        ui,
        state,
    })
}

/// The Clips page's Song view: rows 1 to 5 made into a song, the third
/// placement on the drums made unique and selected, song mode on and Rec
/// song armed, playing at 5.3.
fn song_sample(fonts: Fonts) -> Result<Sample, String> {
    let mut s = playing()?;
    let rows: Vec<(usize, Tick)> = (0..5)
        .filter(|&r| s.project.row_filled(r))
        .map(|r| (r, scene_length(&s.project, r, BAR)))
        .collect();
    let mut song = scenes_to_song(&s.project, &rows, 0).map_err(|e| format!("{e:?}"))?;
    song.make_unique(&s.project.tracks, 2)
        .map_err(|e| format!("{e:?}"))?;
    s.project.arrangement = song;
    s.project.scene_mut(1).ok_or("no row 2")?.name = "Verse".into();
    s.project.arrangement.markers[1].name = "Verse".into();
    let mut ui = app_ui(fonts, "Clips", SONG_HINT);
    let page = TimePage::add_to(&mut ui)?;
    ui.show(CLIPS_PAGE)?;
    let mut state = s.state();
    state.chips = vec![arp_chip()];
    state.song_view = true;
    state.song_on = true;
    state.song_rec = true;
    state.selected = Some(2);
    page.update(&mut ui, &state);
    Ok(Sample {
        name: "clips-song",
        about: "Clips page, Song view: rows 1 to 5 as a song, a drum placement made unique \
                and selected, song mode on at 5.3, Rec song armed",
        ui,
        state,
    })
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
    let page = TimePage::add_to(&mut ui)?;
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
        assert_eq!(samples.len(), 10);
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
