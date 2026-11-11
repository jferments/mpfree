//! Sample screens with a real transport, player, recorder and take undo
//! behind them, for the workstation preview and the tests. Track and clip
//! names are made up (no Akai content).
//!
//! The screens sit in the pad player as it would be with the time engine
//! wired in: a Clips tab in its header, the transport bar under the header
//! on the Play and Clips pages, and a sample arp chip on the bar where the
//! arp's chip goes.

use crate::view::{launches_after, with_transport, TimeAction, TimePage, TimeState, TransportInfo, CLIPS_HINT,
                  CLIPS_PAGE, TRANSPORT};
use crate::widgets::TransportBar;
use mpfree_play::{Event, Launch, Out, PlayView, Player, Recorder, Sink, Takes};
use mpfree_seq::{Clip, Note, Project, Track};
use mpfree_transport::time::Tick;
use mpfree_transport::{Block, Command, Transport};
use mpfree_ui::widgets::{Chip, Fader, Header, Label, ListItem, ListView, PadGrid, StatList};
use mpfree_ui::{Align, Color, Fonts, Theme, Tone, Ui};

/// The screen as the pad player sees it (the MPC One's panel, turned).
pub const SCREEN: (i32, i32) = (1280, 800);
/// The pad player's header tabs with Clips added.
pub const APP_TABS: [&str; 5] = ["Play", "Clips", "Debug", "Console", "Prefs"];
/// The pad player's Play page (`mpfree-hello`'s `PLAY_LAYOUT`), before the
/// bar goes in.
pub const APP_PLAY_LAYOUT: &str = "col(header:60, row(pads:*, col(kits:*, level:96, timing:232):460):*, hint:26)";
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

/// A running engine: the transport, the player and the recorder on the
/// "audio thread", and the project, the launches and the takes on the
/// "control thread", as the app will run them.
pub struct Session {
    // Boxed: the audio thread's fixed tables are large for a test thread's
    // stack.
    pub transport: Box<Transport>,
    player: Box<Player>,
    recorder: Box<Recorder>,
    pub project: Project,
    pub launches: Vec<Launch>,
    pub armed: Option<usize>,
    pub takes: Takes,
    view: PlayView,
    block: Box<Block>,
    /// Pad notes still to play into the recorder, at song ticks.
    live: Vec<(Tick, Event)>,
}

impl Session {
    pub fn new(project: Project, launches: Vec<Launch>, armed: Option<usize>) -> Result<Session, String> {
        let transport = Box::new(Transport::new(RATE, BPM).map_err(|e| format!("transport: {e:?}"))?);
        let mut s = Session { transport, player: Box::new(Player::new()), recorder: Box::new(Recorder::new()),
                              project, launches, armed, takes: Takes::default(), view: PlayView::empty(),
                              block: Box::new(Block::new()), live: Vec::new() };
        s.compile()?;
        Ok(s)
    }

    fn compile(&mut self) -> Result<(), String> {
        let mut v = PlayView::compile(&self.project, &self.launches).map_err(|e| format!("compile: {e:?}"))?;
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
        self.transport.command(at, cmd).then_some(()).ok_or_else(|| "command queue full".to_string())
    }

    /// Play `note` on the pads from `tick` for `len` ticks.
    pub fn pad(&mut self, tick: Tick, len: Tick, note: u8, velocity: u8) {
        self.live.push((tick, Event::NoteOn { note, velocity }));
        self.live.push((tick + len, Event::NoteOff { note }));
    }

    /// The control thread's side of a user action on the launches.
    pub fn act(&mut self, a: TimeAction) -> Result<(), String> {
        self.launches = launches_after(&self.launches, &a, &TransportInfo::of(&self.transport));
        self.compile()
    }

    pub fn undo(&mut self) -> Result<Option<u32>, String> {
        let n = self.takes.undo(&mut self.project);
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
                    self.live.remove(i);
                }
                None => i += 1,
            }
        }
        self.player.process(&self.view, &self.block, &mut Mute);
        self.recorder.process(&self.view, &self.block);
        let mut changed = false;
        for e in self.player.erased().to_vec() {
            changed |= self.takes.erase(&mut self.project, &e);
        }
        for k in self.recorder.taken().to_vec() {
            self.takes.add(&mut self.project, &k).map_err(|e| format!("recorded note: {e:?}"))?;
            changed = true;
        }
        if changed {
            self.compile()?;
        }
        Ok(())
    }

    pub fn state(&self) -> TimeState {
        TimeState::from_engine(TransportInfo::of(&self.transport), &self.project, &self.launches, self.armed,
                               &self.takes)
    }
}

fn clip(name: &str, length: Tick, notes: &[(Tick, u8, u8)]) -> Clip {
    let mut c = Clip::new(name, length);
    for &(tick, note, velocity) in notes {
        let len = SIXTEENTH.min(length - tick);
        c.add_note(Note { tick, len, note, velocity }).expect("sample note in range");
    }
    c
}

/// A note every `step` ticks through `length`.
fn every(step: Tick, length: Tick, note: u8, velocity: u8) -> Vec<(Tick, u8, u8)> {
    (0..length / step).map(|i| (i * step, note, velocity)).collect()
}

fn beat(bars: Tick) -> Vec<(Tick, u8, u8)> {
    (0..bars).flat_map(|b| [(b * BAR, 0, 120), (b * BAR + BEAT, 1, 110), (b * BAR + 1680, 0, 100),
                             (b * BAR + 3 * BEAT, 1, 115)]).collect()
}

/// The stab riff: five notes in one bar (what take 1 replaces).
pub const STAB_RIFF: [(Tick, u8, u8); 5] = [(0, 6, 120), (720, 6, 100), (1440, 6, 110), (2400, 6, 105), (3120, 6, 100)];

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
    track("Drums", vec![(0, clip("Boom bap", 2 * BAR, &beat(2))), (1, clip("Half time", 4 * BAR, &beat(4)[..8])),
                        (2, clip("Fill", BAR, &every(SIXTEENTH, BAR, 1, 90))),
                        (4, clip("Break", 2 * BAR, &every(480, 2 * BAR, 0, 100)))]);
    track("Hats", vec![(0, clip("Hats 16", BAR, &every(SIXTEENTH, BAR, 2, 80))),
                       (1, clip("Shaker", 2 * BAR, &every(480, 2 * BAR, 3, 70)))]);
    track("Bass", vec![(0, clip("Sub line", 2 * BAR, &every(1280, 2 * BAR, 4, 100))),
                       (1, clip("Bass B", 4 * BAR, &every(1280, 4 * BAR, 4, 100))),
                       (3, clip("Drop", 2 * BAR, &every(BAR, 2 * BAR, 4, 127)))]);
    track("Glitch", vec![(0, clip("Glitch", 7 * SIXTEENTH, &every(3 * SIXTEENTH, 7 * SIXTEENTH, 5, 90))),
                         (2, clip("Stutter", BAR, &every(SIXTEENTH, BAR, 5, 70)))]);
    track("Chords", vec![(0, clip("Arp chords", 4 * BAR, &every(2 * BEAT, 4 * BAR, 7, 90))),
                         (2, clip("Pad", 8 * BAR, &every(2 * BAR, 8 * BAR, 8, 80)))]);
    track("Stab", vec![(0, clip("Stab riff", BAR, &STAB_RIFF))]);
    track("Dust", vec![(0, clip("Dust", 4 * BAR, &[(0, 9, 60)]))]);
    track("FX", vec![(0, clip("Riser", 4 * BAR, &[(0, 10, 90)])), (3, clip("Wash", 2 * BAR, &[(0, 11, 90)]))]);
    p
}

/// Every track's first clip from bar 1, the glitch from bar 3; the chords
/// wait to be launched.
fn launches() -> Vec<Launch> {
    [0, 1, 2, 5, 6, 7]
        .iter()
        .map(|&track| Launch { track, slot: 0, origin: 0 })
        .chain(std::iter::once(Launch { track: 3, slot: 0, origin: 2 * BAR }))
        .collect()
}

/// The clank pattern the samples record over the stab, in bar `bar` (from 0).
fn clank(s: &mut Session, bar: Tick, hits: &[Tick]) {
    for (i, &h) in hits.iter().enumerate() {
        s.pad(bar * BAR + h, SIXTEENTH, 12, 100 + 5 * (i as u8 % 4));
    }
}

/// Playing from bar 1 to 5.3.240; at 5.3 the chords were tapped, so they
/// start on the next bar.
pub fn playing() -> Result<Session, String> {
    let mut s = Session::new(project(), launches(), Some(STAB))?;
    s.command_at(0, Command::Play)?;
    s.run_to(4 * BAR + 2 * BEAT)?;
    s.act(TimeAction::Launch { track: CHORDS, slot: 0 })?;
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

/// Stopped at bar 1 with nothing launched and nothing armed.
pub fn stopped() -> Result<Session, String> {
    Session::new(project(), Vec::new(), None)
}

fn arp_chip() -> Chip {
    // Sample text: the arp thread's chip goes here when it lands.
    Chip::new("Arp · Up · 1/16", Tone::Accent)
}

fn app_ui(fonts: Fonts, tab: usize, hint: &str) -> Ui {
    let mut ui = Ui::new(Theme::dark(), fonts, SCREEN.0, SCREEN.1);
    let mut header = Header::new(&APP_TABS);
    header.active = tab;
    header.chips = vec![Chip::new("Akai session", Tone::Good), Chip::new("Kit 2 of 6", Tone::Accent)];
    ui.add("header", header);
    ui.add("hint", Label::new(hint, Tone::Dim, Align::Center));
    ui
}

fn clips_sample(name: &'static str, about: &'static str, fonts: Fonts, s: &Session, first_row: usize)
    -> Result<Sample, String> {
    let mut ui = app_ui(fonts, 1, CLIPS_HINT);
    let mut page = TimePage::add_to(&mut ui)?;
    ui.show(CLIPS_PAGE)?;
    page.first_row = first_row;
    let mut state = s.state();
    state.chips = vec![arp_chip()];
    page.update(&mut ui, &state);
    Ok(Sample { name, about, ui, state })
}

/// The pad player's Play page with the bar under the header.
fn play_sample(fonts: Fonts, s: &Session) -> Result<Sample, String> {
    let mut ui = app_ui(fonts, 0, PLAY_HINT);
    let mut pads = PadGrid::new(16, 4);
    let names = ["Kick", "Snare", "Closed Hat", "Open Hat", "Sub", "Glitch", "Stab", "Chord", "Chord 2", "Dust",
                 "Riser", "Wash", "Clank", "", "", ""];
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
    for (k, d) in [("Classic House", "16 pads"), ("Boom Bap", "13 pads"), ("Trap", "16 pads"), ("Lo-Fi", "9 pads"),
                   ("Techno", "16 pads"), ("Afrobeat", "14 pads")] {
        kits.items.push(ListItem { text: k.into(), detail: d.into(), color: Some(color) });
    }
    kits.selected = 1;
    ui.add("kits", kits);
    let mut level = Fader::new("Level · Q-Link 1");
    level.value = 0.79;
    level.text = "-18.0 dB".into();
    ui.add("level", level);
    let mut timing = StatList::new("Timing");
    timing.set(vec![("Period".into(), "128 frames · 2.9 ms".into(), Tone::Normal),
                    ("Audio thread".into(), "0.41 ms mean · 0.88 ms max".into(), Tone::Good),
                    ("Underruns".into(), "0".into(), Tone::Good),
                    ("Voices".into(), "6".into(), Tone::Normal)]);
    ui.add("timing", timing);
    let layout = with_transport(APP_PLAY_LAYOUT).ok_or("the Play layout has no header")?;
    let page = TimePage::add_to(&mut ui)?;
    ui.add_page("Play", &layout)?;
    ui.show("Play")?;
    let mut state = s.state();
    state.chips = vec![arp_chip()];
    page.update(&mut ui, &state);
    Ok(Sample { name: "play", about: "the Play page with the transport bar under the header, playing at 5.3",
                ui, state })
}

/// Every sample screen, with `fonts` called once per screen.
pub fn samples(fonts: impl Fn() -> Fonts) -> Result<Vec<Sample>, String> {
    let playing = playing()?;
    let mut out = vec![
        play_sample(fonts(), &playing)?,
        clips_sample("clips-playing", "Clips page: playing at 5.3, the chords tapped and waiting for bar 6",
                     fonts(), &playing, 0)?,
        clips_sample("clips-recording", "Clips page: replace recording into the stab at 4.2, take 1 in progress",
                     fonts(), &recording()?, 0)?,
        clips_sample("clips-undo", "Clips page: overdub take 2 undone, Redo brings it back", fonts(), &undone()?,
                     0)?,
        clips_sample("clips-count-in", "Clips page: counting in a bar before recording from bar 1 (0.3)", fonts(),
                     &counting_in()?, 0)?,
        clips_sample("clips-stopped", "Clips page: stopped at 1.1, nothing launched or armed", fonts(), &stopped()?,
                     0)?,
    ];
    // A narrow look at the bar alone, while recording.
    let rec = recording()?;
    let mut ui = Ui::new(Theme::dark(), fonts(), SCREEN.0, 68);
    ui.add(TRANSPORT, TransportBar::new());
    ui.add_page("bar", "transport")?;
    let mut state = rec.state();
    state.chips = vec![arp_chip()];
    TimePage::default().update(&mut ui, &state);
    out.push(Sample { name: "bar-recording", about: "the transport bar alone, recording", ui, state });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{cell_of, take_items};
    use crate::widgets::CellState;
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
        assert_eq!(samples.len(), 7);
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
    fn the_playing_sample_queues_the_chords_for_the_next_bar() {
        let s = playing().unwrap();
        let st = s.state();
        assert_eq!(st.transport.state, State::Playing);
        // A period is about 4 ticks, so the run stops just past 5.3.240.
        let at = st.transport.meter.position(st.transport.position);
        assert_eq!((at.bar, at.beat), (5, 3));
        assert!((240..245).contains(&at.tick), "{at}");
        let chords = cell_of(&st, CHORDS, 0);
        assert_eq!((chords.state, chords.foot.as_str()), (CellState::Queued, "from 6.1"));
        assert_eq!(cell_of(&st, DRUMS, 0).state, CellState::Playing);
        assert_eq!(cell_of(&st, 3, 0).state, CellState::Playing, "the glitch started at bar 3");
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
        assert_eq!(notes(&s, STAB), vec![(0, 12), (480, 12), (960, 12), (1680, 12), (2880, 12)]);
    }

    #[test]
    fn the_undo_sample_took_back_take_two_only() {
        let s = undone().unwrap();
        let st = s.state();
        assert_eq!(st.transport.state, State::Playing);
        assert!(st.transport.overdub);
        let rows: Vec<(u32, usize, usize)> = st.done.iter().map(|r| (r.number, r.added, r.erased)).collect();
        assert_eq!(rows, vec![(1, 5, 5)]);
        let rows: Vec<(u32, usize, usize)> = st.undone.iter().map(|r| (r.number, r.added, r.erased)).collect();
        assert_eq!(rows, vec![(2, 4, 0)]);
        // The clip is as take 1 left it.
        assert_eq!(notes(&s, STAB), vec![(0, 12), (480, 12), (960, 12), (1680, 12), (2880, 12)]);
        let (items, current) = take_items(&st);
        assert_eq!(items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(),
                   vec!["Take 2 · Stab / 1", "Take 1 · Stab / 1"]);
        assert_eq!(current, Some(1));
    }

    #[test]
    fn the_count_in_sample_counts_in() {
        let st = counting_in().unwrap().state();
        assert_eq!(st.transport.state, State::CountIn);
        let at = st.transport.meter.position(st.transport.position);
        assert_eq!((at.bar, at.beat), (0, 3));
        assert!(at.tick < 5, "{at}");
        assert_eq!(cell_of(&st, STAB, 0).state, CellState::Queued, "nothing plays before bar 1");
    }
}
