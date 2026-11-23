//! Following another device's MIDI clock: the transport takes the
//! leader's tempo, keeps to its pulses and, as the project's MIDI setup
//! says, starts, stops and moves with it (docs/dev/design/midi-clock.md,
//! "Following").
//!
//! The MIDI input hands [`Follower::bytes`] the leader port's bytes, each
//! with the frame it arrived at on the transport's frame clock, and a
//! [`ClockIn`] reads them. The follower is part of the audio half: it
//! drives the transport with its ordinary commands before the transport
//! runs, and allocates nothing.
//!
//! - **Start and stop** (`Follow::start_stop`): Start plays from the top
//!   at the first pulse after it, Continue from the leader's Song Position,
//!   and Stop stops. Without `Follow::position` both play from where the
//!   song stands. While the leader is stopped, PLAY waits for it
//!   ([`Follower::waits`]: the app says "Waiting for clock"), and a take
//!   the app has ready ([`Follower::take_waits`]) records from the same
//!   pulse: `Play` and `Record` on one frame, a punch-in with no count-in
//!   ([`Follower::took`] tells the host to ready the clip).
//! - **Position** (`Follow::position`): a Song Position while both are
//!   stopped moves the song there. While both play, the leader's position
//!   is the one to keep: more than a sixteenth away, the transport jumps to
//!   it (counted in [`FollowCounts::jumps`]). The leader owns the timeline,
//!   so the transport's loop is off for as long as position is followed:
//!   the follower holds it ([`Follower::held_loop`]), keeps a loop set
//!   meanwhile in its place ([`Follower::keep_loop`]), and gives it back
//!   when following position stops.
//! - **Tempo**: a phase-locked loop. The phase error is measured at every
//!   pulse and averaged over a few pulses ([`SMOOTH`]). The loop's tempo
//!   for the leader starts from the leader's measured tempo and moves with
//!   the error ([`KI`]); the transport plays it nudged by the error
//!   ([`KP`], at most [`MAX_NUDGE`]). The tempo goes to the transport at
//!   every pulse and takes effect on its next sixteenth line. The measured
//!   tempo is the pulses' average over [`BEATS`] beats (over the last beat
//!   until then, from [`KNOWN_PULSES`] pulses on); a change of more than
//!   [`MOVED`] from the loop's tempo starts the loop again from it. The
//!   tempo shown is the measured one.
//! - **Phase**: the leader's song position while following position with
//!   both playing; otherwise the transport's pulse grid on the leader's
//!   pulses, as many whole pulses apart as they started.
//! - **Lost clock**: when pulses stop for [`ClockIn::LOST_SECONDS`], the
//!   follower lets go of the tempo: the transport plays on at the last one
//!   until the app's tempo applies again (the app keeps the followed tempo
//!   as its own, so nothing changes). If the leader was playing, it is
//!   counted ([`FollowCounts::lost`]) and [`Follower::lost`] is true until
//!   a pulse or a Stop comes, or following ends: System status's "The
//!   clock MPFree follows stopped".
//!
//! Bytes the app reads once a period came during the period before, up to
//! a period before it read them. Stamped with the period's start, the
//! transport would play about half a period behind the leader (1.5 ms at
//! 128 frames), so the app stamps them half a period earlier
//! ([`read_stamp`]): on average when they came. Followed that way, the
//! transport keeps within half a period of the leader, centred on it.

use mpfree_midi_clock::{ClockIn, InEvent, PPQN, PULSE_TICKS};
use mpfree_transport::time::{ticks_in, MAX_BPM, MIN_BPM};
use mpfree_transport::{Command, State, Tick, Transport, PPQ};

/// The frame to stamp bytes with that were read at the start of a period
/// of `period` frames beginning at frame `top`: they came during the
/// period before, so half a period earlier is when they came on average.
pub fn read_stamp(top: u64, period: u32) -> u64 {
    top.saturating_sub(u64::from(period / 2))
}

/// The most the tempo is nudged from the loop's to close a phase error.
pub const MAX_NUDGE: f64 = 0.03;
/// The nudge for an error of a beat (so an error of 10 ticks plays 2 %
/// faster or slower).
pub const KP: f64 = 2.0;
/// How far the loop's tempo moves each pulse for an error of a beat.
/// With [`KP`] and [`SMOOTH`] (picked with a simulation, checked by this
/// module's tests): a steady clock is followed within a tick after
/// four beats, and a 1.5 % change in the leader's tempo is at most about
/// 7 ticks off and within a tick again four beats later.
pub const KI: f64 = 0.05;
/// While keeping the leader's position, an error past a sixteenth jumps.
pub const JUMP_TICKS: f64 = PPQ as f64 / 4.0;
/// Beats the leader's tempo is averaged over.
pub const BEATS: usize = 8;
/// Pulses before the leader's tempo counts as known: one interval with a
/// period's jitter is too rough to set the tempo from.
pub const KNOWN_PULSES: u32 = 3;
/// A change in the leader's tempo this large (3 %) goes in at once.
pub const MOVED: f64 = 0.03;
/// Each pulse's phase error goes into the average with this weight.
pub const SMOOTH: f64 = 0.25;
/// The tempo shown moves only when the leader's moves this much (BPM).
const SHOWN_STEP: f64 = 0.05;
const PULSES_PER_BEAT: u32 = PPQN as u32;

/// What following another device's clock does: the project's MIDI setup.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Follow {
    /// Start, Continue and Stop start and stop the transport.
    pub start_stop: bool,
    /// Song Position moves the song, and while both play the transport
    /// keeps the leader's position.
    pub position: bool,
}

/// What following put right or lost, for the status line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FollowCounts {
    /// Jumps to the leader's position, which was more than a sixteenth away.
    pub jumps: u32,
    /// Times the leader's clock stopped arriving while it played.
    pub lost: u32,
    /// Commands the transport's queue refused.
    pub refused: u32,
}

impl FollowCounts {
    pub fn any(&self) -> bool {
        self.jumps + self.lost + self.refused > 0
    }
}

pub struct Follower {
    rate: u32,
    follow: Option<Follow>,
    clock: ClockIn,
    /// The leader plays: after Start or Continue, until Stop.
    playing: bool,
    /// Start or Continue came: at the next pulse, play (from this tick,
    /// when following position).
    start: Option<Option<i64>>,
    /// Commands went to the transport that it has not run yet, so its
    /// position is not the one to measure.
    moved: bool,
    /// The last pulse's frame.
    last: Option<u64>,
    /// Pulses since the clock started arriving.
    pulses: u32,
    /// A pulse's frame every beat, oldest first, for the tempo.
    beats: [u64; BEATS + 1],
    n_beats: usize,
    since_beat: u32,
    /// The leader's tempo, measured, once known.
    bpm: Option<f64>,
    /// The tempo shown, which moves in steps of [`SHOWN_STEP`] or more.
    shown: Option<f64>,
    /// The loop's tempo for the leader.
    base: Option<f64>,
    /// The phase error, averaged, in ticks (positive: the leader is
    /// ahead), and whether it is to the leader's position.
    err: Option<f64>,
    absolute: bool,
    counts: FollowCounts,
    /// The leader's clock stopped arriving while it played, and no pulse
    /// or Stop has come since.
    lost: bool,
    /// A take is ready: at the leader's start it records, adding (true)
    /// or replacing.
    take: Option<bool>,
    /// A take started at the leader's start, until [`Follower::took`].
    took: bool,
    /// Following position: the transport's loop is off, and this is the
    /// one it gets back.
    hold: bool,
    held: Option<(Tick, Tick)>,
}

impl Follower {
    /// Following nothing, at sample rate `rate`.
    pub fn new(rate: u32) -> Follower {
        Follower {
            rate: rate.max(1),
            follow: None,
            clock: ClockIn::new(rate),
            playing: false,
            start: None,
            moved: false,
            last: None,
            pulses: 0,
            beats: [0; BEATS + 1],
            n_beats: 0,
            since_beat: 0,
            bpm: None,
            shown: None,
            base: None,
            err: None,
            absolute: false,
            counts: FollowCounts::default(),
            lost: false,
            take: None,
            took: false,
            hold: false,
            held: None,
        }
    }

    /// Follow another device's clock with these options, or not (None).
    /// A change starts again from nothing known; the counts, a ready take
    /// and a held loop stay (the next period gives the loop back if
    /// position isn't followed now).
    pub fn set(&mut self, follow: Option<Follow>) {
        if follow != self.follow {
            let fresh = Follower {
                follow,
                ..Follower::new(self.rate)
            };
            let old = std::mem::replace(self, fresh);
            self.counts = old.counts;
            self.take = old.take;
            self.hold = old.hold;
            self.held = old.held;
        }
    }

    /// Following start and stop with the leader stopped: PLAY waits for
    /// its Start.
    pub fn waits(&self) -> bool {
        self.follow.is_some_and(|f| f.start_stop) && !self.playing
    }

    /// A take the app has ready (REC pressed while stopped), adding
    /// (true) or replacing, or none. Told every period; while
    /// [`Follower::waits`], the leader's start records it.
    pub fn take_waits(&mut self, take: Option<bool>) {
        self.take = take;
    }

    /// True once after the leader's start began a take, so the host makes
    /// the clip it records into.
    pub fn took(&mut self) -> bool {
        std::mem::take(&mut self.took)
    }

    /// While following position: the loop the transport gets back when
    /// that stops (None: no loop).
    pub fn held_loop(&self) -> Option<Option<(Tick, Tick)>> {
        self.hold.then_some(self.held)
    }

    /// A loop set while following position is kept for later, not set:
    /// true if it was.
    pub fn keep_loop(&mut self, range: Option<(Tick, Tick)>) -> bool {
        if self.hold {
            self.held = range;
        }
        self.hold
    }

    /// The leader's tempo while following and its clock arrives, to show
    /// (it moves in steps of 0.05 BPM or more).
    pub fn bpm(&self) -> Option<f64> {
        self.follow.and(self.shown)
    }

    pub fn counts(&self) -> FollowCounts {
        self.counts
    }

    /// Following, the leader's clock stopped arriving while it played:
    /// true until a pulse or a Stop comes, or following changes.
    pub fn lost(&self) -> bool {
        self.follow.is_some() && self.lost
    }

    /// The leader port's bytes, arrived at `frame` (on the transport's
    /// frame clock: [`Transport::frame`] for bytes read at the top of the
    /// period). Ignored while not following.
    pub fn bytes(&mut self, t: &mut Transport, frame: u64, bytes: &[u8]) {
        let Some(f) = self.follow else {
            return;
        };
        for &b in bytes {
            if let Some(e) = self.clock.byte(frame, b) {
                self.event(t, f, frame, e);
            }
        }
    }

    /// Once a period, just before the transport runs (it runs what the
    /// follower sent): the loop held or given back, and a clock that
    /// stopped arriving.
    pub fn period(&mut self, t: &mut Transport) {
        self.moved = false;
        let hold = self.follow.is_some_and(|f| f.position);
        if hold != self.hold {
            self.hold = hold;
            if hold {
                self.held = t.loop_range();
                if self.held.is_some() {
                    self.command(t, Command::SetLoop(None));
                }
            } else if let Some(range) = self.held.take() {
                self.command(t, Command::SetLoop(Some(range)));
            }
        }
        let Some(last) = self.last else {
            return;
        };
        let gap = t.frame().saturating_sub(last) as f64;
        if gap > ClockIn::LOST_SECONDS * f64::from(self.rate) {
            if self.playing {
                self.counts.lost += 1;
                self.lost = true;
            }
            // The tempo is unknown again. The `ClockIn` keeps the leader's
            // play state and position, and averages afresh after a gap.
            self.last = None;
            self.pulses = 0;
            self.n_beats = 0;
            self.since_beat = 0;
            self.bpm = None;
            self.shown = None;
            self.base = None;
            self.err = None;
        }
    }

    fn command(&mut self, t: &mut Transport, cmd: Command) {
        if !t.command(t.frame(), cmd) {
            self.counts.refused += 1;
        }
    }

    fn event(&mut self, t: &mut Transport, f: Follow, frame: u64, e: InEvent) {
        match e {
            InEvent::Started => {
                self.playing = true;
                if f.start_stop {
                    self.start = Some(f.position.then_some(0));
                }
            }
            InEvent::Continued { tick } => {
                self.playing = true;
                if f.start_stop {
                    self.start = Some(f.position.then_some(tick));
                }
            }
            InEvent::Stopped => {
                self.playing = false;
                self.lost = false;
                self.start = None;
                if f.start_stop && t.state().rolling() {
                    self.command(t, Command::Stop);
                    self.moved = true;
                }
            }
            InEvent::Located { tick } => {
                // A Song Position moves the song while both are stopped
                // (or a Stop just went to the transport).
                let stopped = t.state() == State::Stopped || self.moved;
                if f.position && !self.playing && stopped {
                    self.command(t, Command::Locate(tick));
                    self.moved = true;
                }
            }
            InEvent::Pulse { tick } => self.pulse(t, f, frame, tick),
        }
    }

    fn pulse(&mut self, t: &mut Transport, f: Follow, frame: u64, tick: i64) {
        self.tempo_pulse(frame);
        if !self.moved {
            self.phase(t, f, frame, tick);
        }
        // The tempo before a start, so it starts at the leader's.
        self.set_tempo(t);
        if let Some(from) = self.start.take() {
            // Play from the pulse's own tick: the transport starts at the
            // next period, a little late, and closes that like any phase
            // error, rather than skip the first notes.
            if let Some(tick) = from {
                self.command(t, Command::Locate(tick));
            }
            // A ready take records from here: the mode first, then a
            // punch-in on the frame it starts (no count-in: the leader
            // counts).
            let take = self.take.take();
            if take.is_some_and(|add| add != t.overdub()) {
                self.command(t, Command::ToggleOverdub);
            }
            self.command(t, Command::Play);
            if take.is_some() {
                self.command(t, Command::Record);
                self.took = true;
            }
            self.moved = true;
            self.err = None;
        }
    }

    /// The leader's tempo, from the pulses' frames.
    fn tempo_pulse(&mut self, frame: u64) {
        self.last = Some(frame);
        self.lost = false;
        self.pulses = self.pulses.saturating_add(1);
        self.since_beat += 1;
        if self.n_beats == 0 || self.since_beat >= PULSES_PER_BEAT {
            if self.n_beats == self.beats.len() {
                self.beats.copy_within(1.., 0);
                self.n_beats -= 1;
            }
            self.beats[self.n_beats] = frame;
            self.n_beats += 1;
            self.since_beat = 0;
        }
        let short = self.clock.state(frame).bpm;
        let mut long = (self.n_beats >= 2).then(|| {
            let span = (self.beats[self.n_beats - 1] - self.beats[0]) as f64;
            60.0 * f64::from(self.rate) * (self.n_beats - 1) as f64 / span.max(1.0)
        });
        // The tempo changed: average from here again.
        if let (Some(s), Some(l)) = (short, long) {
            if (s - l).abs() > MOVED * l {
                self.beats[0] = frame;
                self.n_beats = 1;
                self.since_beat = 0;
                long = None;
            }
        }
        self.bpm = if self.pulses >= KNOWN_PULSES {
            long.or(short)
        } else {
            None
        };
        if let Some(b) = self.bpm {
            if self.shown.is_none_or(|s| (b - s).abs() >= SHOWN_STEP) {
                self.shown = Some(b);
            }
        }
    }

    /// The phase error at this pulse, averaged; or a jump to the leader's
    /// position when it is too far.
    fn phase(&mut self, t: &mut Transport, f: Follow, frame: u64, tick: i64) {
        let state = t.state();
        if !state.rolling() {
            self.err = None;
            return;
        }
        let at = t.frame();
        let since = at as f64 - frame as f64;
        let ours = t.position_exact() - ticks_in(since, t.bpm(), self.rate);
        let absolute =
            f.position && self.playing && matches!(state, State::Playing | State::Recording);
        if absolute != self.absolute {
            self.absolute = absolute;
            self.err = None;
        }
        let e = if absolute {
            tick as f64 - ours
        } else {
            // The transport's pulse grid on the leader's pulses: the
            // nearest line to the error so far.
            let p = PULSE_TICKS as f64;
            let near = self.err.unwrap_or(0.0);
            let d = -ours - near;
            near + d - p * (d / p).round()
        };
        if absolute && e.abs() > JUMP_TICKS {
            let lead = self.bpm.unwrap_or(t.bpm());
            let to = tick + ticks_in(since, lead, self.rate).round() as i64;
            self.command(t, Command::Locate(to));
            self.counts.jumps += 1;
            self.moved = true;
            self.err = None;
            return;
        }
        let mut avg = self.err.map_or(e, |a| a + (e - a) * SMOOTH);
        if !absolute {
            let p = PULSE_TICKS as f64;
            avg -= p * (avg / p).round();
        }
        self.err = Some(avg);
    }

    /// The loop: its tempo for the leader moves with the error, and the
    /// transport plays it nudged by the error.
    fn set_tempo(&mut self, t: &mut Transport) {
        let Some(measured) = self.bpm else {
            return;
        };
        // Without an error to go by (stopped, or just started or jumped),
        // the measured tempo; and again after the leader's changed.
        let mut base = match (self.base, self.err) {
            (Some(b), Some(_)) if (b - measured).abs() <= MOVED * measured => b,
            _ => measured,
        };
        let e = self.err.map_or(0.0, |e| e / f64::from(PPQ));
        base = (base * (1.0 + KI * e)).clamp(MIN_BPM, MAX_BPM);
        self.base = Some(base);
        let bpm = (base * (1.0 + (KP * e).clamp(-MAX_NUDGE, MAX_NUDGE))).clamp(MIN_BPM, MAX_BPM);
        if (bpm - t.bpm()).abs() > 1e-6 * bpm {
            self.command(t, Command::SetTempo(bpm));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_midi_clock::{CLOCK, CONTINUE, SONG_POSITION, START, STOP};
    use mpfree_transport::{Block, Mark, Span};

    const RATE: u32 = 44_100;
    const ALL: Follow = Follow {
        start_stop: true,
        position: true,
    };

    /// What another device does, in order.
    #[derive(Clone, Copy)]
    enum Lead {
        /// Pulses for this many beats at this tempo.
        Clock(f64, f64),
        Start,
        Stop,
        Continue,
        /// Song Position, in sixteenths.
        Position(u16),
        /// No bytes for this many seconds.
        Quiet(f64),
    }

    /// The leader's bytes at their exact frames, and each pulse's frame
    /// with its song position while it played.
    struct Script {
        bytes: Vec<(u64, Vec<u8>)>,
        pulses: Vec<(u64, Option<i64>)>,
    }

    fn script(steps: &[Lead]) -> Script {
        let mut s = Script {
            bytes: Vec::new(),
            pulses: Vec::new(),
        };
        // The next pulse's frame, the gap before it, and the leader.
        let (mut f, mut gap) = (5000.0f64, 1000.0f64);
        let (mut playing, mut tick) = (false, 0i64);
        for &step in steps {
            // Messages between pulses go halfway to the next one.
            let between = (f - gap / 2.0).round() as u64;
            match step {
                Lead::Clock(beats, bpm) => {
                    gap = 60.0 * f64::from(RATE) / (bpm * 24.0);
                    for _ in 0..(beats * 24.0).round() as usize {
                        let at = f.round() as u64;
                        s.bytes.push((at, vec![CLOCK]));
                        s.pulses.push((at, playing.then_some(tick)));
                        if playing {
                            tick += PULSE_TICKS;
                        }
                        f += gap;
                    }
                }
                Lead::Start => {
                    s.bytes.push((between, vec![START]));
                    (playing, tick) = (true, 0);
                }
                Lead::Stop => {
                    s.bytes.push((between, vec![STOP]));
                    playing = false;
                }
                Lead::Continue => {
                    s.bytes.push((between, vec![CONTINUE]));
                    playing = true;
                }
                Lead::Position(n) => {
                    s.bytes.push((
                        between,
                        vec![SONG_POSITION, (n & 0x7F) as u8, (n >> 7) as u8],
                    ));
                    tick = i64::from(n) * 240;
                }
                Lead::Quiet(seconds) => f += seconds * f64::from(RATE),
            }
        }
        s
    }

    /// How the follower did.
    struct Run {
        spans: Vec<Span>,
        marks: Vec<(u64, Mark)>,
        counts: FollowCounts,
        shown: Option<f64>,
        transport: Transport,
    }

    impl Run {
        /// The transport's state and song position at frame `g`.
        fn at(&self, g: u64) -> Option<(State, f64)> {
            self.spans
                .iter()
                .find(|s| (s.frame..s.frame + u64::from(s.len)).contains(&g))
                .map(|s| (s.state, s.tick_at(g)))
        }

        /// The largest distance from the leader's position at its pulses
        /// from the `skip`th of them on (while it played).
        fn worst(&self, s: &Script, skip: usize) -> f64 {
            s.pulses
                .iter()
                .filter_map(|&(g, t)| t.map(|t| (g, t)))
                .skip(skip)
                .map(|(g, t)| {
                    let (state, ours) = self.at(g).expect("ran that far");
                    assert!(state.rolling(), "stopped at frame {g}");
                    (ours - t as f64).abs()
                })
                .fold(0.0, f64::max)
        }

        /// The mean of our position less the leader's at its pulses after
        /// the first `skip`, in ticks: below 0 when we play behind it.
        fn mean(&self, s: &Script, skip: usize) -> f64 {
            let errs: Vec<f64> = s
                .pulses
                .iter()
                .filter_map(|&(g, t)| t.map(|t| (g, t)))
                .skip(skip)
                .map(|(g, t)| self.at(g).expect("ran that far").1 - t as f64)
                .collect();
            errs.iter().sum::<f64>() / errs.len().max(1) as f64
        }
    }

    /// Run a transport at `bpm` following `s` in periods of `period`
    /// frames. Bytes reach the follower at the top of the period after
    /// they came: at their own frames, or (`read_time`) all stamped as the
    /// app reads MIDI, half a period before that period's first frame
    /// ([`read_stamp`]). `play_at` presses PLAY on the transport itself at
    /// that frame.
    fn follow(
        s: &Script,
        f: Follow,
        period: u32,
        read_time: bool,
        bpm: f64,
        play_at: Option<u64>,
        hook: &mut dyn FnMut(&mut Transport),
    ) -> Run {
        let mut t = Transport::new(RATE, bpm).unwrap();
        let mut fl = Follower::new(RATE);
        fl.set(Some(f));
        let mut b = Block::new();
        let end = s.bytes.last().unwrap().0 + 2 * u64::from(period);
        let (mut spans, mut marks, mut next) = (Vec::new(), Vec::new(), 0);
        while t.frame() < end {
            let p = t.frame();
            while next < s.bytes.len() && s.bytes[next].0 < p {
                let (at, ref bytes) = s.bytes[next];
                let frame = if read_time { read_stamp(p, period) } else { at };
                fl.bytes(&mut t, frame, bytes);
                next += 1;
            }
            if play_at.is_some_and(|a| (p..p + u64::from(period)).contains(&a)) {
                t.command(p, Command::Play);
            }
            hook(&mut t);
            fl.period(&mut t);
            assert!(t.process(period, &mut b));
            spans.extend_from_slice(b.spans());
            marks.extend(b.markers().iter().map(|m| (m.frame, m.mark)));
        }
        Run {
            spans,
            marks,
            counts: fl.counts(),
            shown: fl.bpm(),
            transport: t,
        }
    }

    fn plain(s: &Script, f: Follow, period: u32, read_time: bool, bpm: f64) -> Run {
        follow(s, f, period, read_time, bpm, None, &mut |_| {})
    }

    /// The first period start after frame `g`, where bytes that came at `g`
    /// take effect.
    fn after(g: u64, period: u32) -> u64 {
        (g / u64::from(period) + 1) * u64::from(period)
    }

    fn first_pulse_after(s: &Script, byte: u8) -> u64 {
        let at = s.bytes.iter().find(|(_, b)| b[0] == byte).unwrap().0;
        s.pulses.iter().find(|&&(g, _)| g > at).unwrap().0
    }

    /// A transport at 120 BPM follows a leader at 128 that sends clock
    /// for two beats, then Start and 16 beats: it starts from the top in
    /// the period after the first pulse after Start (up to 11 ticks late
    /// at 512 frames), and after four beats keeps within a tick of
    /// the leader's position, at every period size, and shows the leader's
    /// tempo. Nothing had to jump.
    #[test]
    fn a_steady_clock_is_followed_to_a_tick_from_start() {
        let s = script(&[
            Lead::Clock(2.0, 128.0),
            Lead::Start,
            Lead::Clock(16.0, 128.0),
        ]);
        let first = first_pulse_after(&s, START);
        for period in [16, 64, 128, 512, 1024] {
            let r = plain(&s, ALL, period, false, 120.0);
            let started: Vec<_> = r
                .marks
                .iter()
                .filter(|m| matches!(m.1, Mark::Started { .. }))
                .collect();
            assert_eq!(
                started,
                [&(after(first, period), Mark::Started { tick: 0 })],
                "period {period}"
            );
            let worst = r.worst(&s, 4 * 24);
            assert!(worst <= 1.0, "period {period}: {worst:.2} ticks off");
            let shown = r.shown.unwrap();
            assert!((shown - 128.0).abs() < 0.05, "period {period}: {shown}");
            assert_eq!(r.counts, FollowCounts::default());
        }
        // A leader that sends clock only while it plays: the transport
        // starts at its own 120 BPM, takes the leader's 100 from the third
        // pulse, and is within a tick after six beats.
        let s = script(&[Lead::Start, Lead::Clock(16.0, 100.0)]);
        let r = plain(&s, ALL, 128, false, 120.0);
        let worst = r.worst(&s, 6 * 24);
        assert!(worst <= 1.0, "cold: {worst:.2} ticks off");
        assert_eq!(r.counts, FollowCounts::default());
    }

    /// The app reads MIDI once a period, so every byte comes up to a
    /// period before it is read, and is stamped half a period before the
    /// period it was read at ([`read_stamp`]). Followed that way, the
    /// transport is centred on the leader (within a tick on average) and
    /// wanders by no more than half a period, and the tempo shown stays
    /// within 0.1 BPM.
    #[test]
    fn a_clock_read_once_a_period_is_followed_within_half_a_period() {
        let s = script(&[
            Lead::Clock(2.0, 128.0),
            Lead::Start,
            Lead::Clock(24.0, 128.0),
        ]);
        for period in [64, 128, 256] {
            let r = plain(&s, ALL, period, true, 120.0);
            let period_ticks = ticks_in(f64::from(period), 128.0, RATE);
            let worst = r.worst(&s, 4 * 24);
            let mean = r.mean(&s, 4 * 24);
            assert!(
                worst <= period_ticks / 2.0 + 1.0,
                "period {period}: {worst:.2} ticks off, a period is {period_ticks:.2}"
            );
            assert!(
                mean.abs() <= 1.0,
                "period {period}: {mean:.2} ticks on average"
            );
            let shown = r.shown.unwrap();
            assert!((shown - 128.0).abs() < 0.1, "period {period}: {shown}");
            assert_eq!(r.counts, FollowCounts::default());
        }
    }

    /// The leader's tempo steps while it plays, from 120 BPM: by 1.5 %
    /// (which the loop absorbs, at most about 7 ticks off), by 10 % and by
    /// -25 % (which start the loop again from the measured tempo, at most
    /// about 50 and 170 ticks off). Each is back within a tick after 4, 5
    /// and 8 beats, without a jump, and the tempo shown is the new one.
    #[test]
    fn a_tempo_change_is_followed_without_a_jump() {
        for (to, beats, peak) in [(121.8, 4, 10.0), (132.0, 5, 60.0), (90.0, 8, 200.0)] {
            let s = script(&[
                Lead::Clock(1.0, 120.0),
                Lead::Start,
                Lead::Clock(8.0, 120.0),
                Lead::Clock(12.0, to),
            ]);
            let r = plain(&s, ALL, 128, false, 120.0);
            let most = r.worst(&s, 8 * 24);
            assert!(most <= peak, "to {to}: {most:.2} ticks off at most");
            let worst = r.worst(&s, (8 + beats) * 24);
            assert!(
                worst <= 1.0,
                "to {to}: {worst:.2} ticks off after {beats} beats"
            );
            assert!((r.shown.unwrap() - to).abs() < 0.05, "to {to}");
            assert_eq!(r.counts, FollowCounts::default(), "to {to}");
        }
    }

    /// Stop stops the transport in the period after it; a Song Position
    /// while both are stopped moves the song there; Continue plays from
    /// it at the next pulse, and the transport keeps the leader's position
    /// again.
    #[test]
    fn stop_song_position_and_continue_are_followed() {
        let s = script(&[
            Lead::Clock(1.0, 120.0),
            Lead::Start,
            Lead::Clock(4.0, 120.0),
            Lead::Stop,
            Lead::Clock(1.0, 120.0),
            // Bar 3.
            Lead::Position(32),
            Lead::Clock(1.0, 120.0),
            Lead::Continue,
            Lead::Clock(8.0, 120.0),
        ]);
        let stop = s.bytes.iter().find(|(_, b)| b[0] == STOP).unwrap().0;
        let r = plain(&s, ALL, 128, false, 120.0);
        let stopped: Vec<u64> = r
            .marks
            .iter()
            .filter(|m| matches!(m.1, Mark::Stopped { .. }))
            .map(|m| m.0)
            .collect();
        assert_eq!(stopped, [after(stop, 128)]);
        let cont = first_pulse_after(&s, CONTINUE);
        assert!(
            r.marks
                .contains(&(after(cont, 128), Mark::Started { tick: 7680 })),
            "{:?}",
            r.marks
        );
        // Every pulse played after Continue, but the first few.
        let from = s.pulses.iter().position(|&(g, _)| g == cont).unwrap();
        let played = s.pulses[from..].iter().filter(|p| p.1.is_some()).count();
        let worst = r.worst(
            &s,
            s.pulses.iter().filter(|p| p.1.is_some()).count() - played + 24,
        );
        assert!(worst <= 1.0, "{worst:.2} ticks off after Continue");
        assert_eq!(r.counts, FollowCounts::default());
    }

    /// Following start and stop, PLAY waits while the leader is stopped
    /// (and again after its Stop). A ready take records from the leader's
    /// first pulse after Start: Play and Record on one frame, so no
    /// count-in although one is set, adding as the take asked; the host
    /// hears of it once.
    #[test]
    fn a_ready_take_records_from_the_leaders_first_pulse() {
        let s = script(&[
            Lead::Clock(2.0, 120.0),
            Lead::Start,
            Lead::Clock(4.0, 120.0),
            Lead::Stop,
            Lead::Clock(1.0, 120.0),
        ]);
        let mut t = Transport::new(RATE, 120.0).unwrap();
        assert!(t.command(0, Command::SetCountIn(1)));
        let mut fl = Follower::new(RATE);
        fl.set(Some(ALL));
        fl.take_waits(Some(true));
        let mut b = Block::new();
        let (mut marks, mut waited, mut next, mut took) = (Vec::new(), Vec::new(), 0, 0);
        let end = s.bytes.last().unwrap().0 + 256;
        while t.frame() < end {
            let p = t.frame();
            while next < s.bytes.len() && s.bytes[next].0 < p {
                let (at, ref bytes) = s.bytes[next];
                fl.bytes(&mut t, at, bytes);
                next += 1;
            }
            waited.push(fl.waits());
            took += usize::from(fl.took());
            fl.period(&mut t);
            assert!(t.process(128, &mut b));
            marks.extend(b.markers().iter().map(|m| (m.frame, m.mark)));
        }
        let at = after(first_pulse_after(&s, START), 128);
        for m in [
            Mark::Overdub(true),
            Mark::Started { tick: 0 },
            Mark::RecordOn { tick: 0 },
        ] {
            assert!(marks.contains(&(at, m)), "{m:?} at {at}: {marks:?}");
        }
        assert!(!marks.iter().any(|m| matches!(m.1, Mark::CountIn { .. })));
        assert_eq!(took, 1);
        assert_eq!(
            (waited.first(), waited.contains(&false), waited.last()),
            (Some(&true), true, Some(&true))
        );
        // Without start and stop, PLAY never waits.
        fl.set(Some(Follow {
            start_stop: false,
            position: true,
        }));
        assert!(!fl.waits());
    }

    /// Following position, the leader owns the timeline: the transport's
    /// loop is off while it is followed, a loop set meanwhile is kept in
    /// its place, and the transport gets it back when following position
    /// stops.
    #[test]
    fn following_position_holds_the_loop_off_and_gives_it_back() {
        let mut t = Transport::new(RATE, 120.0).unwrap();
        let mut b = Block::new();
        let mut fl = Follower::new(RATE);
        let mut run = |fl: &mut Follower, t: &mut Transport| {
            fl.period(t);
            assert!(t.process(128, &mut b));
            (t.loop_range(), fl.held_loop())
        };
        assert!(t.command(0, Command::SetLoop(Some((0, 3840)))));
        assert_eq!(run(&mut fl, &mut t), (Some((0, 3840)), None));
        fl.set(Some(Follow {
            start_stop: true,
            position: false,
        }));
        assert_eq!(run(&mut fl, &mut t), (Some((0, 3840)), None));
        fl.set(Some(ALL));
        assert_eq!(run(&mut fl, &mut t), (None, Some(Some((0, 3840)))));
        assert!(fl.keep_loop(Some((0, 7680))));
        assert_eq!(run(&mut fl, &mut t), (None, Some(Some((0, 7680)))));
        fl.set(None);
        assert_eq!(run(&mut fl, &mut t), (Some((0, 7680)), None));
        assert!(!fl.keep_loop(None));
    }

    /// Without "follow position", Song Position is ignored and Continue
    /// plays on from where the transport stopped; without "follow
    /// start/stop", Start and Stop are ignored and only the tempo follows.
    #[test]
    fn the_follow_options_say_what_follows() {
        let s = script(&[
            Lead::Clock(1.0, 120.0),
            Lead::Start,
            Lead::Clock(4.0, 120.0),
            Lead::Stop,
            Lead::Position(32),
            Lead::Clock(1.0, 120.0),
            Lead::Continue,
            Lead::Clock(4.0, 120.0),
        ]);
        let no_position = Follow {
            start_stop: true,
            position: false,
        };
        let r = plain(&s, no_position, 128, false, 120.0);
        let starts: Vec<i64> = r
            .marks
            .iter()
            .filter_map(|m| match m.1 {
                Mark::Started { tick } => Some(tick),
                _ => None,
            })
            .collect();
        // From the top (Start), then from where it stopped, about bar 2.
        assert_eq!(starts.len(), 2, "{:?}", r.marks);
        assert_eq!(starts[0], 0);
        assert!((3800..3900).contains(&starts[1]), "{starts:?}");
        let no_start_stop = Follow {
            start_stop: false,
            position: true,
        };
        let r = plain(&s, no_start_stop, 128, false, 96.0);
        assert!(r.marks.iter().all(|m| !matches!(m.1, Mark::Started { .. })));
        assert_eq!(r.transport.state(), State::Stopped);
        assert_eq!(r.transport.position(), 7680, "Song Position still moves it");
        assert!((r.transport.bpm() - 120.0).abs() < 0.05);
    }

    /// Following only the tempo, a transport started on its own lines its
    /// pulse grid up with the leader's pulses within two beats, however
    /// many pulses apart it started.
    #[test]
    fn started_on_its_own_it_keeps_its_grid_on_the_leaders_pulses() {
        let s = script(&[Lead::Clock(12.0, 110.0)]);
        let tempo_only = Follow::default();
        for play_at in [20_000, 20_333, 21_011] {
            let r = follow(
                &s,
                tempo_only,
                128,
                false,
                120.0,
                Some(play_at),
                &mut |_| {},
            );
            let worst = s
                .pulses
                .iter()
                .filter(|&&(g, _)| g > play_at + 4 * 24_054)
                .map(|&(g, _)| {
                    let ours = r.at(g).unwrap().1;
                    (ours - (ours / 40.0).round() * 40.0).abs()
                })
                .fold(0.0, f64::max);
            assert!(
                worst <= 1.0,
                "PLAY at {play_at}: {worst:.2} ticks off the pulses"
            );
            assert_eq!(r.counts, FollowCounts::default());
        }
    }

    /// Following position, a loop of the transport's own jumps back to
    /// the leader's position when it wraps (and so, past its end, plays
    /// on), and the jump is counted.
    #[test]
    fn a_jump_away_from_the_leaders_position_is_put_right_and_counted() {
        let s = script(&[
            Lead::Clock(1.0, 120.0),
            Lead::Start,
            Lead::Clock(12.0, 120.0),
        ]);
        let mut looped = false;
        let r = follow(&s, ALL, 128, false, 120.0, None, &mut |t| {
            if !looped && t.state().rolling() {
                looped = t.command(t.frame(), Command::SetLoop(Some((0, 3840))));
            }
        });
        assert_eq!(r.counts.jumps, 1);
        let worst = r.worst(&s, 6 * 24);
        assert!(worst <= 1.0, "{worst:.2} ticks off after the jump");
    }

    /// The leader's clock stops arriving while it plays (a cable pulled):
    /// the transport plays on at the last tempo, the follower shows no
    /// tempo, so the app's applies again, and the loss is counted once.
    #[test]
    fn a_clock_that_stops_arriving_is_counted_and_the_tempo_kept() {
        let s = script(&[
            Lead::Clock(1.0, 130.0),
            Lead::Start,
            Lead::Clock(4.0, 130.0),
            Lead::Quiet(2.0),
            Lead::Clock(0.04, 130.0),
        ]);
        let r = plain(&s, ALL, 128, false, 120.0);
        assert_eq!(r.counts.lost, 1);
        assert_eq!(r.transport.state(), State::Playing);
        assert!((r.transport.bpm() - 130.0).abs() < 1.0);
        assert_eq!(r.shown, None);
    }

    /// System status's "The clock MPFree follows stopped": `lost` is true
    /// from when a playing leader's pulses stop until a pulse comes back,
    /// or a Stop comes, and never for a leader that was stopped or once
    /// following ends.
    #[test]
    fn a_lost_clock_lasts_until_a_pulse_or_a_stop() {
        let s = script(&[
            Lead::Clock(1.0, 130.0),
            Lead::Quiet(2.0),
            Lead::Clock(1.0, 130.0),
            Lead::Start,
            Lead::Clock(4.0, 130.0),
            Lead::Quiet(2.0),
            Lead::Clock(1.0, 130.0),
            Lead::Quiet(2.0),
            Lead::Stop,
            Lead::Clock(1.0, 130.0),
        ]);
        let period = 128u32;
        let mut t = Transport::new(RATE, 120.0).unwrap();
        let mut fl = Follower::new(RATE);
        fl.set(Some(ALL));
        let mut b = Block::new();
        let end = s.bytes.last().unwrap().0 + 2 * u64::from(period);
        // Each change of `lost`, with the frame it was seen at.
        let (mut changes, mut was, mut next) = (Vec::new(), false, 0);
        while t.frame() < end {
            let p = t.frame();
            while next < s.bytes.len() && s.bytes[next].0 < p {
                let (at, ref bytes) = s.bytes[next];
                fl.bytes(&mut t, at, bytes);
                next += 1;
            }
            fl.period(&mut t);
            if fl.lost() != was {
                was = fl.lost();
                changes.push((p, was));
            }
            assert!(t.process(period, &mut b));
        }
        // Lost twice while it played (the quiet before Start says nothing):
        // the first ends at the next pulse, the second at the Stop.
        assert_eq!(fl.counts().lost, 2);
        let ons: Vec<bool> = changes.iter().map(|c| c.1).collect();
        assert_eq!(ons, [true, false, true, false], "{changes:?}");
        let start = s.bytes.iter().find(|(_, b)| b[0] == START).unwrap().0;
        let stop = s.bytes.iter().find(|(_, b)| b[0] == STOP).unwrap().0;
        assert!(changes[0].0 > start);
        // A pulse ends the first, the Stop the second.
        let back = s.pulses.iter().find(|&&(g, _)| g > changes[0].0).unwrap().0;
        assert_eq!(changes[1].0, after(back, period));
        assert_eq!(changes[3].0, after(stop, period));
        // Following something else (or nothing) ends it too.
        fl.lost = true;
        assert!(fl.lost());
        fl.set(None);
        assert!(!fl.lost());
    }

    #[test]
    fn a_follower_set_to_nothing_ignores_the_clock() {
        let s = script(&[
            Lead::Clock(1.0, 130.0),
            Lead::Start,
            Lead::Clock(4.0, 130.0),
        ]);
        let mut t = Transport::new(RATE, 120.0).unwrap();
        let mut fl = Follower::new(RATE);
        for (at, b) in &s.bytes {
            fl.bytes(&mut t, *at, b);
        }
        let mut block = Block::new();
        fl.period(&mut t);
        assert!(t.process(128, &mut block));
        assert!(block.markers().is_empty());
        assert_eq!(
            (t.state(), t.bpm(), fl.bpm()),
            (State::Stopped, 120.0, None)
        );
    }
}
