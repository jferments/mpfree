//! The time engine in the pad player: the transport, the sequencer
//! (`mpfree-play`'s player and recorder), take undo and a metronome click,
//! run in the audio loop.
//!
//! Each period the loop first runs the transport ([`TimeHost::advance`]),
//! so the arp can follow its clock ([`TimeHost::arp_clock`]), hands
//! [`TimeHost::pad`] the pad notes it plays (after the arp, with their
//! offsets), then calls [`TimeHost::period`]:
//! the transport runs, the clips play their notes on the kit's pads through
//! [`Pads`] (the sampler), the armed track records, and recorded and erased
//! notes go into the clips and the take history. After the sampler renders,
//! [`TimeHost::mix_click`] adds the click while counting in and recording.
//!
//! Edits (launches, takes, arming) compile a new play view in the loop.
//! That allocates, but only on an edit, not every period; moving it to a
//! control thread is a later step. Every track plays the current kit.

use mpfree_play::{Event, Launch, Out, PlayView, Player, Recorder, Sink, Takes};
use mpfree_seq::{Clip, Project, Track};
use mpfree_time_ui::view::{launches_after, TimeAction, TimeState, TransportInfo};
use mpfree_transport::{Block, Command, Mark, State, Transport};
use std::f32::consts::TAU;

/// Tracks in a new project.
pub const TRACKS: usize = 8;
/// Bars in a clip made by tapping an empty slot (without a loop set).
pub const NEW_CLIP_BARS: i64 = 2;
/// Notes the sequencer can start or stop in one period.
const MAX_OUT: usize = 512;

/// Where the sequencer's notes go: the sampler's pads.
pub trait Pads {
    fn hit_at(&mut self, pad: usize, velocity: u8, offset: u32) -> bool;
    fn release_at(&mut self, pad: usize, offset: u32);
    fn stop_all(&mut self);
}

impl Pads for crate::sampler::Player {
    fn hit_at(&mut self, pad: usize, velocity: u8, offset: u32) -> bool {
        crate::sampler::Player::hit_at(self, pad, velocity, offset)
    }
    fn release_at(&mut self, pad: usize, offset: u32) {
        crate::sampler::Player::release_at(self, pad, offset)
    }
    fn stop_all(&mut self) {
        crate::sampler::Player::stop_all(self)
    }
}

/// The player's notes for one period, in a fixed table.
struct Collect {
    out: [Out; MAX_OUT],
    n: usize,
    dropped: u64,
}

impl Sink for Collect {
    fn send(&mut self, out: Out) {
        if self.n < MAX_OUT {
            self.out[self.n] = out;
            self.n += 1;
        } else {
            self.dropped += 1;
        }
    }
}

/// The metronome: a short sine blip on each beat, higher on the bar.
#[derive(Clone, Copy, Debug)]
struct Click {
    rate: f32,
    /// Starts in this period: (offset, accent).
    starts: [(u32, bool); 8],
    n: usize,
    phase: f32,
    freq: f32,
    env: f32,
}

/// The click's level (about -12 dBFS) and decay (a quarter of its level
/// after 15 ms).
const CLICK_GAIN: f32 = 0.25;
const CLICK_MS: f32 = 15.0;

impl Click {
    fn new(rate: u32) -> Click {
        Click { rate: rate as f32, starts: [(0, false); 8], n: 0, phase: 0.0, freq: 0.0, env: 0.0 }
    }

    fn schedule(&mut self, offset: u32, accent: bool) {
        if self.n < self.starts.len() {
            self.starts[self.n] = (offset, accent);
            self.n += 1;
        }
    }

    /// Add the click to `left` and `right` (keeping them within ±1);
    /// returns the peak afterwards.
    fn mix(&mut self, left: &mut [f32], right: &mut [f32]) -> f32 {
        let decay = 0.25f32.powf(1.0 / (CLICK_MS * 1e-3 * self.rate));
        let mut peak = 0f32;
        let mut next = 0;
        for f in 0..left.len().min(right.len()) {
            while next < self.n && self.starts[next].0 as usize == f {
                self.freq = if self.starts[next].1 { 1760.0 } else { 1320.0 };
                self.phase = 0.0;
                self.env = 1.0;
                next += 1;
            }
            if self.env > 1e-4 {
                let s = CLICK_GAIN * self.env * (self.phase * TAU).sin();
                self.phase = (self.phase + self.freq / self.rate).fract();
                self.env *= decay;
                left[f] = (left[f] + s).clamp(-1.0, 1.0);
                right[f] = (right[f] + s).clamp(-1.0, 1.0);
            }
            peak = peak.max(left[f].abs()).max(right[f].abs());
        }
        self.n = 0;
        peak
    }
}

pub struct TimeHost {
    // Boxed: the fixed tables are large.
    transport: Box<Transport>,
    player: Box<Player>,
    recorder: Box<Recorder>,
    block: Box<Block>,
    sink: Box<Collect>,
    view: PlayView,
    project: Project,
    launches: Vec<Launch>,
    armed: Option<usize>,
    takes: Takes,
    click: Click,
    /// Click while counting in and recording.
    pub metronome: bool,
    /// The clips, launches or takes changed since [`TimeHost::take_changed`].
    changed: bool,
    /// [`TimeHost::advance`] ran the transport for the coming period (and
    /// whether it could).
    advanced: Option<bool>,
    /// A start or jump after the first span of the last period: the next
    /// arp clock says it jumped.
    jump_pending: bool,
}

impl TimeHost {
    /// Stopped at bar 1 with an empty project of [`TRACKS`] tracks, track 1
    /// armed.
    pub fn new(rate: u32, bpm: f64) -> Result<TimeHost, String> {
        let transport = Box::new(Transport::new(rate, bpm).map_err(|e| format!("transport: {e:?}"))?);
        let mut project = Project::new();
        for i in 0..TRACKS {
            project.tracks.push(Track::new(&format!("Track {}", i + 1), i as u16));
        }
        let blank = Out { offset: 0, frame: 0, track: 0, dest: 0, event: Event::AllOff };
        let mut host = TimeHost { transport, player: Box::new(Player::new()), recorder: Box::new(Recorder::new()),
                                  block: Box::new(Block::new()),
                                  sink: Box::new(Collect { out: [blank; MAX_OUT], n: 0, dropped: 0 }),
                                  view: PlayView::empty(), project, launches: Vec::new(), armed: Some(0),
                                  takes: Takes::default(), click: Click::new(rate), metronome: true, changed: true,
                                  advanced: None, jump_pending: false };
        host.compile()?;
        Ok(host)
    }

    fn compile(&mut self) -> Result<(), String> {
        let mut v = PlayView::compile(&self.project, &self.launches).map_err(|e| format!("clips: {e:?}"))?;
        v.set_armed(self.armed);
        self.view = v;
        self.changed = true;
        Ok(())
    }

    /// A transport command, from the next period on.
    pub fn command(&mut self, cmd: Command) -> bool {
        let at = self.transport.frame();
        self.transport.command(at, cmd)
    }

    /// Follow the app's tempo (TAP TEMPO, the menu).
    pub fn follow_tempo(&mut self, bpm: f64) {
        if (self.transport.bpm() - bpm).abs() > 1e-9 {
            self.command(Command::SetTempo(bpm));
        }
    }

    /// Carry out a touch on the time engine's screens. Returns a line for
    /// the log.
    pub fn act(&mut self, a: TimeAction) -> Result<String, String> {
        let info = self.info();
        match a {
            TimeAction::Transport(cmd) => {
                if !self.command(cmd) {
                    return Err("transport: command queue full".into());
                }
                Ok(format!("transport: {cmd:?}"))
            }
            TimeAction::Undo => match self.takes.undo(&mut self.project) {
                Some(n) => self.compile().map(|_| format!("undo: take {n}")),
                None => Ok("undo: nothing to undo".into()),
            },
            TimeAction::Redo => match self.takes.redo(&mut self.project) {
                Some(n) => self.compile().map(|_| format!("redo: take {n}")),
                None => Ok("redo: nothing to redo".into()),
            },
            TimeAction::Arm(track) => {
                self.armed = track.filter(|&t| t < self.project.tracks.len());
                self.compile()?;
                Ok(match self.armed {
                    Some(t) => format!("armed: {}", self.project.tracks[t].name),
                    None => "armed: none".into(),
                })
            }
            TimeAction::Launch { .. } | TimeAction::StopClip(_) => {
                let after = launches_after(&self.launches, &a, &info);
                let before = std::mem::replace(&mut self.launches, after);
                if let Err(e) = self.compile() {
                    self.launches = before;
                    return Err(e);
                }
                Ok(format!("clips: {a:?}"))
            }
            TimeAction::NewClip { track, slot } => {
                let bar = info.meter.bar_ticks();
                let length = info.loop_range.map_or(NEW_CLIP_BARS * bar, |(a, b)| b - a);
                let t = self.project.tracks.get_mut(track).ok_or("no such track")?;
                let s = t.slots.get_mut(slot).ok_or("no such slot")?;
                if s.is_some() {
                    return Err(format!("slot {} of {} is not empty", slot + 1, t.name));
                }
                *s = Some(Clip::new(&format!("Clip {}", slot + 1), length));
                self.armed = Some(track);
                self.launches = launches_after(&self.launches, &a, &info);
                self.compile()?;
                // Record into it, unless already recording or counting in.
                if !matches!(info.state, State::Recording | State::CountIn) {
                    self.command(Command::Record);
                }
                Ok(format!("new clip in slot {} of track {}, recording", slot + 1, track + 1))
            }
            TimeAction::Chip(_) => Ok(String::new()),
        }
    }

    /// A pad note played in the coming period, at `offset`: recorded into
    /// the armed track while the transport records.
    pub fn pad(&mut self, offset: u32, event: Event) {
        self.recorder.input(offset, event);
    }

    /// Run the transport for the coming period, before the app's process,
    /// so the arp can follow it ([`TimeHost::arp_clock`]).
    /// [`TimeHost::period`] then plays and records on the same block.
    pub fn advance(&mut self, frames: u32) -> bool {
        let ok = self.transport.process(frames, &mut self.block);
        self.advanced = Some(ok);
        ok
    }

    /// The arp's clock for the coming period, from the first span of the
    /// block [`TimeHost::advance`] made: None while stopped, so the arp
    /// runs on its own tempo. The count-in counts as playing. `jumped` is
    /// set by a start or jump before the first span, or by one later in the
    /// last period (the arp follows the first span only, so it catches up
    /// one period later).
    // The arp thread's App::set_clock takes it: `app.set_clock(time.arp_clock())`
    // after `time.advance` in the audio loop. Until that lands only tests call it.
    #[allow(dead_code)]
    pub fn arp_clock(&mut self) -> Option<mpfree_arp::Clock> {
        let jump = |m: &Mark| matches!(m, Mark::Started { .. } | Mark::CountIn { .. } | Mark::Located { .. }
                                            | Mark::Looped { .. });
        let first = self.block.spans().first().copied();
        let first_offset = first.map_or(0, |s| s.offset);
        let mut jumped = std::mem::take(&mut self.jump_pending);
        for m in self.block.markers() {
            if jump(&m.mark) {
                if m.offset <= first_offset {
                    jumped = true;
                } else {
                    self.jump_pending = true;
                }
            }
        }
        let span = first?;
        if !span.state.rolling() {
            return None;
        }
        Some(mpfree_arp::Clock {
            anchor_tick: span.anchor_tick,
            frames_since_anchor: span.frame.saturating_sub(span.anchor_frame),
            bpm: span.bpm,
            playing: true,
            jumped,
            swing: (span.swing.amount > 50).then_some((span.swing.grid, span.swing.amount)),
            bar_ticks: u32::try_from(span.meter.bar_ticks()).unwrap_or(4 * mpfree_arp::PPQ),
        })
    }

    /// Run one period: the clips play into `pads`, the armed track records.
    /// Runs the transport first unless [`TimeHost::advance`] already did.
    pub fn period(&mut self, frames: u32, pads: &mut impl Pads) {
        let ok = match self.advanced.take() {
            Some(ok) => ok,
            None => self.transport.process(frames, &mut self.block),
        };
        if !ok {
            return;
        }
        self.sink.n = 0;
        self.player.process(&self.view, &self.block, &mut *self.sink);
        for o in &self.sink.out[..self.sink.n] {
            match o.event {
                Event::NoteOn { note, velocity } => {
                    pads.hit_at(usize::from(note), velocity, o.offset);
                }
                Event::NoteOff { note } => pads.release_at(usize::from(note), o.offset),
                Event::AllOff => pads.stop_all(),
            }
        }
        self.recorder.process(&self.view, &self.block);
        let mut edited = false;
        for i in 0..self.player.erased().len() {
            let e = self.player.erased()[i];
            edited |= self.takes.erase(&mut self.project, &e);
        }
        for i in 0..self.recorder.taken().len() {
            let k = self.recorder.taken()[i];
            edited |= self.takes.add(&mut self.project, &k).is_ok();
        }
        if edited {
            // The clips only gained or lost notes, so this compiles.
            let _ = self.compile();
        }
        if self.metronome {
            for span in self.block.spans() {
                if matches!(span.state, State::CountIn | State::Recording) {
                    let bar = span.meter.bar_ticks();
                    for (tick, offset) in span.lines(span.meter.beat_ticks()) {
                        self.click.schedule(offset, tick.rem_euclid(bar) == 0);
                    }
                }
            }
        }
    }

    /// Add the click to the rendered period; returns the peak afterwards.
    pub fn mix_click(&mut self, left: &mut [f32], right: &mut [f32]) -> f32 {
        self.click.mix(left, right)
    }

    /// The transport's description of the last period (for the looper).
    pub fn block(&self) -> &Block {
        &self.block
    }

    /// The song position as bar.beat.tick, for log lines.
    pub fn position_text(&self) -> String {
        let t = &self.transport;
        t.meter().position(t.position()).to_string()
    }

    pub fn info(&self) -> TransportInfo {
        TransportInfo::of(&self.transport)
    }

    /// True once after the clips, launches or takes changed.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// Everything the screens show.
    pub fn state(&self) -> TimeState {
        TimeState::from_engine(self.info(), &self.project, &self.launches, self.armed, &self.takes)
    }

    /// Notes lost because a table was full (should stay 0).
    pub fn dropped(&self) -> u64 {
        self.sink.dropped + self.recorder.dropped() + self.player.stats().dropped
    }

    /// The time engine's part of the status line.
    pub fn describe(&self) -> String {
        let t = &self.transport;
        format!("time: {:?} at {}, {:.1} BPM, {} takes, {} notes lost to full tables",
                t.state(), t.meter().position(t.position()), t.bpm(), self.takes.done().len(), self.dropped())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 44_100;
    const PERIOD: u32 = 128;

    #[derive(Default)]
    struct Fake {
        hits: Vec<(u64, usize, u8)>,
        frame: u64,
        stops: usize,
    }

    impl Pads for Fake {
        fn hit_at(&mut self, pad: usize, velocity: u8, offset: u32) -> bool {
            self.hits.push((self.frame + u64::from(offset), pad, velocity));
            true
        }
        fn release_at(&mut self, _pad: usize, _offset: u32) {}
        fn stop_all(&mut self) {
            self.stops += 1;
        }
    }

    /// Run until frame `to`, pressing `presses` (absolute frame, pad, velocity, frames held) on the way.
    fn run(h: &mut TimeHost, pads: &mut Fake, to: u64, presses: &[(u64, u8, u8, u64)]) {
        while pads.frame < to {
            let end = pads.frame + u64::from(PERIOD);
            for &(at, pad, v, held) in presses {
                if (pads.frame..end).contains(&at) {
                    h.pad((at - pads.frame) as u32, Event::NoteOn { note: pad, velocity: v });
                }
                if (pads.frame..end).contains(&(at + held)) {
                    h.pad((at + held - pads.frame) as u32, Event::NoteOff { note: pad });
                }
            }
            h.period(PERIOD, pads);
            pads.frame = end;
        }
    }

    /// Frames per bar at 120 BPM.
    const BAR: u64 = 88_200;

    /// The arp's clock follows the transport: None while stopped, the
    /// transport's anchor while playing (a whole tick on an exact frame),
    /// `jumped` once at the start and after a locate, and the swing.
    #[test]
    fn the_arp_clock_follows_the_transport() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        assert!(h.advance(PERIOD));
        assert_eq!(h.arp_clock(), None, "stopped: the arp runs on its own");
        h.period(PERIOD, &mut pads);
        h.command(Command::Play);
        let mut clocks = Vec::new();
        for _ in 0..40 {
            h.advance(PERIOD);
            clocks.push(h.arp_clock().expect("playing"));
            h.period(PERIOD, &mut pads);
        }
        assert!(clocks[0].jumped && clocks[1..].iter().all(|c| !c.jumped), "{clocks:?}");
        for c in &clocks {
            assert!(c.playing && c.bpm == 120.0 && c.bar_ticks == 3840 && c.swing.is_none());
            // The anchor's tick and frame agree with the song position at the period's start.
            let tick = c.anchor_tick as f64 + c.frames_since_anchor as f64 * 120.0 * 960.0 / (60.0 * f64::from(RATE));
            assert!(tick >= 0.0);
        }
        let k = clocks.len() - 1;
        let at = |c: &mpfree_arp::Clock| c.anchor_tick as f64
            + c.frames_since_anchor as f64 * 120.0 * 960.0 / (60.0 * f64::from(RATE));
        assert!((at(&clocks[k]) - at(&clocks[0]) - k as f64 * f64::from(PERIOD) * 960.0 * 2.0 / f64::from(RATE)).abs()
                < 1e-6);
        // A locate while playing: jumped once, from the new place.
        h.command(Command::Locate(3840 * 4));
        h.advance(PERIOD);
        let c = h.arp_clock().unwrap();
        assert!(c.jumped && (at(&c) - 15_360.0).abs() < 1e-6, "{c:?}");
        h.period(PERIOD, &mut pads);
        h.advance(PERIOD);
        assert!(!h.arp_clock().unwrap().jumped);
        h.period(PERIOD, &mut pads);
        // Swing on the transport goes to the arp.
        h.command(Command::SetSwing(mpfree_transport::Swing { grid: 240, amount: 62 }));
        h.period(PERIOD, &mut pads);
        h.advance(PERIOD);
        assert_eq!(h.arp_clock().unwrap().swing, Some((240, 62)));
        h.period(PERIOD, &mut pads);
        // Stopped again: None.
        h.command(Command::Stop);
        h.period(PERIOD, &mut pads);
        h.advance(PERIOD);
        assert_eq!(h.arp_clock(), None);
    }

    #[test]
    fn a_new_clip_records_the_pads_and_plays_them_back() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        h.command(Command::SetLoop(Some((0, 2 * 3840))));
        assert!(h.act(TimeAction::NewClip { track: 2, slot: 0 }).unwrap().contains("recording"));
        let st = h.state();
        assert_eq!(st.armed, Some(2));
        assert_eq!(st.clip(2, 0).map(|c| c.length), Some(2 * 3840), "as long as the loop");
        // Record two hits in the first pass; the second pass plays them.
        run(&mut h, &mut pads, 2 * BAR, &[(1000, 4, 100, 2000), (BAR + 500, 9, 80, 2000)]);
        assert_eq!(h.info().state, State::Recording);
        assert!(h.take_changed());
        let st = h.state();
        assert_eq!(st.done.len(), 1);
        assert_eq!(st.done[0].added, 2);
        assert!(pads.hits.is_empty(), "nothing plays the notes back while they are being played");
        run(&mut h, &mut pads, 4 * BAR, &[]);
        // Notes are kept in whole ticks (960 to a beat: 23 frames at 120 BPM).
        let back: Vec<(u64, usize)> = pads.hits.iter().map(|&(f, p, _)| (f, p)).collect();
        assert_eq!(back.len(), 2, "{back:?}");
        for ((f, p), (want, pad)) in back.iter().zip([(2 * BAR + 1000, 4), (3 * BAR + 500, 9)]) {
            assert!(f.abs_diff(want) <= 23 && *p == pad, "{back:?}");
        }
        // Undo takes the take out; the next pass is silent.
        assert_eq!(h.act(TimeAction::Undo).unwrap(), "undo: take 1");
        pads.hits.clear();
        run(&mut h, &mut pads, 6 * BAR, &[]);
        assert!(pads.hits.is_empty());
        assert_eq!(h.act(TimeAction::Redo).unwrap(), "redo: take 1");
        h.act(TimeAction::Transport(Command::Record)).unwrap();
        run(&mut h, &mut pads, 8 * BAR, &[]);
        assert_eq!(pads.hits.len(), 2, "redo brings both back");
        assert_eq!(h.dropped(), 0);
    }

    #[test]
    fn a_slot_with_a_clip_is_not_made_again() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        h.act(TimeAction::NewClip { track: 0, slot: 3 }).unwrap();
        assert!(h.act(TimeAction::NewClip { track: 0, slot: 3 }).is_err());
        assert!(h.act(TimeAction::NewClip { track: 99, slot: 0 }).is_err());
    }

    #[test]
    fn the_click_counts_in_and_follows_recording_only() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        let (mut l, mut r) = ([0f32; PERIOD as usize], [0f32; PERIOD as usize]);
        let mut onsets = Vec::new();
        let mut frame = 0u64;
        let mut prev = 0f32;
        let mut step = |h: &mut TimeHost, pads: &mut Fake, frame: &mut u64, onsets: &mut Vec<u64>| {
            h.period(PERIOD, pads);
            l.fill(0.0);
            r.fill(0.0);
            h.mix_click(&mut l, &mut r);
            // An onset: sound after silence (a blip dies away within 0.1 s).
            for (f, &s) in l.iter().enumerate() {
                if s != 0.0 && prev == 0.0 {
                    onsets.push(*frame + f as u64);
                }
                prev = s;
            }
            pads.frame += u64::from(PERIOD);
            *frame += u64::from(PERIOD);
        };
        // Playing: no click.
        h.command(Command::Play);
        while frame < BAR {
            step(&mut h, &mut pads, &mut frame, &mut onsets);
        }
        assert!(onsets.is_empty(), "{onsets:?}");
        h.command(Command::Stop);
        h.command(Command::SetCountIn(1));
        h.command(Command::Record);
        let start = frame;
        while frame < start + 2 * BAR {
            step(&mut h, &mut pads, &mut frame, &mut onsets);
        }
        // One bar of count-in and one of recording: eight beats, half a second apart.
        assert_eq!(onsets.len(), 8, "{onsets:?}");
        for w in onsets.windows(2) {
            assert!((w[1] - w[0]).abs_diff(22_050) <= 1, "{onsets:?}");
        }
        assert!(l.iter().chain(&r).all(|s| s.abs() <= 1.0));
    }

    #[test]
    fn the_transport_follows_the_apps_tempo() {
        let mut h = TimeHost::new(RATE, 120.0).unwrap();
        let mut pads = Fake::default();
        h.follow_tempo(96.0);
        h.period(PERIOD, &mut pads);
        assert_eq!(h.info().bpm, 96.0);
        h.follow_tempo(96.0);
        h.period(PERIOD, &mut pads);
        assert_eq!(h.info().bpm, 96.0);
    }
}
