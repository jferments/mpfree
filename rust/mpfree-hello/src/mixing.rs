//! The mixer in the pad player: mpfree-fx's [`Mixer`] is the one place the
//! sounds are summed.
//!
//! - The playing kit's 16 pads each have a pad channel (Pad Mix), and all
//!   16 go to Track 1 "Drums" (the sampler renders each pad into its own
//!   channel: `render_routed`; the clips play the kit too). Track 2
//!   "Synth" is Poly, Track 3 "Loop" is the audio looper's playback. The
//!   tracks go to the master.
//! - Input 1 "Line in" is the line input, as the stereo pair, going to the
//!   master. It is heard while the looper's Monitor is on (the strip's
//!   Monitor follows it), and it is off while the line input is closed. The
//!   looper records the line input from its own copy, as before.
//! - The master fader is the pad player's Level (Q-Link 1 or the slider on
//!   the Play page), never above `--gain-db`. Master safety (the safe
//!   limiter at -0.3 dBFS) takes the place of the clip at full scale.
//! - The metronome goes on after the mixer, as before: never recorded or
//!   limited (it keeps itself within ±1).
//! - What the looper records from the pads: the drums (the pads that
//!   played, summed from their channels' inputs) and the synth as the
//!   engines make them, before their strips, as before. Recording through
//!   the mixer's capture points comes with the recorder, which makes up
//!   the period they arrive late.
//!
//! The screen: while the mixer panel shows ([`MixHost::show`]), the audio
//! loop sends a [`MixState`] every [`STATE_EVERY`] (it takes the meters),
//! and applies what the panel sends ([`MixHost::action`]).
//!
//! Nothing in [`MixHost::period`] allocates. Its time is in the "mix" stage
//! of CPU by engine; the state for the screen allocates, a few times a
//! second while the panel shows, as the other pages' states do.

use std::time::{Duration, Instant};

use mpfree_fx::ids::mixer::{self as mx, monitor, source};
use mpfree_fx::{Mixer, MixerConfig, StripId};
use mpfree_fx::ids::mixer::output;
use mpfree_mixer_ui::{default_name, MixAction, MixState, MixView};
use mpfree_node::Ctx;
use mpfree_sampler::engine::PadOut;

use crate::PERIOD;

/// The tracks: what each engine plays into.
pub const DRUMS: usize = 0;
pub const SYNTH: usize = 1;
pub const LOOP: usize = 2;
pub const TRACKS: usize = 3;
/// Pad channels: the playing kit's pads.
pub const PADS: usize = 16;
/// The line input's strip.
pub const LINE: StripId = StripId::Input(0);
/// Insert slots each strip has room for.
pub const SLOTS: usize = 4;
/// How often the panel gets a new state (meters) while it shows.
pub const STATE_EVERY: Duration = Duration::from_millis(40);

/// A period of stereo sound.
pub type Stereo<'a> = (&'a [f32; PERIOD], &'a [f32; PERIOD]);

/// The strips' names on the panel.
pub fn name(s: StripId) -> String {
    match s {
        StripId::Track(DRUMS) => "Drums".into(),
        StripId::Track(SYNTH) => "Synth".into(),
        StripId::Track(LOOP) => "Loop".into(),
        LINE => "Line in".into(),
        s => default_name(s),
    }
}

pub struct MixHost {
    pub mixer: Mixer,
    /// The highest the master fader goes (`--gain-db`).
    max_db: f32,
    monitor: bool,
    line_open: bool,
    /// Master safety's count of frames over the ceiling, at the last period.
    overs: u64,
    /// The panel's view while it shows, and the strip selected on it.
    shown: Option<MixView>,
    selected: Option<StripId>,
    state_at: Option<Instant>,
}

impl MixHost {
    /// Made before the sound starts (it allocates). `level_db` is the
    /// Level now, `max_db` the most it goes to.
    pub fn new(rate: u32, level_db: f32, max_db: f32) -> MixHost {
        let cfg = MixerConfig { inputs: 1, pads: PADS, tracks: TRACKS, slots: SLOTS, max_latency_ms: 50.0 };
        let mut mixer = Mixer::new(cfg, rate, PERIOD);
        for p in 0..PADS {
            mixer.set_now(StripId::Pad(p), mx::OUTPUT, output::TRACK);
            mixer.set_now(StripId::Pad(p), mx::OUTPUT_TRACK, DRUMS as f32);
        }
        mixer.set_now(StripId::Master, mx::VOLUME, level_db.min(max_db));
        mixer.set_now(LINE, mx::SOURCE, source::OFF);
        mixer.set_now(LINE, mx::MONITOR, monitor::OFF);
        MixHost { mixer, max_db, monitor: false, line_open: false, overs: 0, shown: None, selected: None, state_at: None }
    }

    /// The Level (Q-Link 1, the slider): the master fader.
    pub fn set_level(&mut self, db: f32) { self.mixer.set(StripId::Master, mx::VOLUME, db.min(self.max_db)); }

    /// The master fader now, dB.
    #[cfg(test)]
    pub fn level(&self) -> f32 { self.mixer.param(StripId::Master, mx::VOLUME) }

    /// Follows the looper: whether the line input is open, and whether it
    /// is heard (Monitor). Cheap to call every period.
    pub fn follow(&mut self, line_open: bool, monitor_on: bool) {
        if line_open != self.line_open {
            self.line_open = line_open;
            self.mixer.set(LINE, mx::SOURCE, if line_open { source::STEREO } else { source::OFF });
        }
        if monitor_on != self.monitor {
            self.monitor = monitor_on;
            self.mixer.set(LINE, mx::MONITOR, if monitor_on { monitor::ON } else { monitor::OFF });
        }
    }

    /// The pad channels for the sampler's `render_routed` this period.
    pub fn pads(&mut self) -> Pads<'_> { Pads { mixer: &mut self.mixer, played: 0 } }

    /// The drums as the sampler made them: the pads that played (`played`,
    /// from [`Pads::played`]) summed from their channels' inputs into
    /// `left` and `right`, for the looper. Silence when none played.
    pub fn drums(&mut self, played: u32, left: &mut [f32; PERIOD], right: &mut [f32; PERIOD]) {
        left.fill(0.0);
        right.fill(0.0);
        for p in (0..PADS).filter(|p| played & (1 << p) != 0) {
            if let Some((l, r)) = self.mixer.pad_input(p) {
                for f in 0..PERIOD {
                    left[f] += l[f];
                    right[f] += r[f];
                }
            }
        }
    }

    /// One period: the synth and the loop into their tracks, the line input
    /// into the input (the pads are in already: [`MixHost::pads`]), the
    /// master out into `left` and `right`. Returns the output's peak and
    /// whether master safety caught anything.
    pub fn period(&mut self, ctx: &Ctx, synth: Stereo, looped: Stereo, line: Stereo, left: &mut [f32; PERIOD],
                  right: &mut [f32; PERIOD]) -> (f32, bool) {
        for (t, (l, r)) in [(SYNTH, synth), (LOOP, looped)] {
            if let Some((dl, dr)) = self.mixer.track_input(t) {
                dl[..PERIOD].copy_from_slice(l);
                dr[..PERIOD].copy_from_slice(r);
            }
        }
        if self.line_open {
            let (hl, hr) = self.mixer.hw_input();
            hl[..PERIOD].copy_from_slice(line.0);
            hr[..PERIOD].copy_from_slice(line.1);
        }
        self.mixer.process(ctx, left, right);
        let peak = left.iter().chain(right.iter()).fold(0f32, |m, s| m.max(s.abs()));
        let overs = self.mixer.safety().stats().overs;
        let caught = overs != self.overs;
        self.overs = overs;
        (peak, caught)
    }

    /// The mixer panel opened on `view`, or closed (`None`).
    pub fn show(&mut self, view: Option<MixView>) {
        self.shown = view;
        self.state_at = None;
    }

    /// What the panel sent. Returns the master fader's new level when it
    /// moved, so the Play page's Level follows.
    pub fn action(&mut self, a: &MixAction) -> Option<f32> {
        match *a {
            MixAction::Set { strip: StripId::Master, id: mx::VOLUME, value } => {
                let db = value.min(self.max_db);
                self.mixer.set(StripId::Master, mx::VOLUME, db);
                self.state_at = None;
                Some(db)
            }
            MixAction::Set { strip, id, value } => {
                self.mixer.set(strip, id, value);
                None
            }
            MixAction::Select(s) => {
                self.selected = Some(s);
                self.state_at = None;
                None
            }
            MixAction::View(v) => {
                self.show(Some(v));
                None
            }
            // The insert pages come with the FX pages.
            MixAction::OpenInsert { .. } | MixAction::AddInsert { .. } => None,
        }
    }

    /// The panel's next state, while it shows and [`STATE_EVERY`] has
    /// passed (at once after a change).
    pub fn poll_state(&mut self, now: Instant) -> Option<MixState> {
        let view = self.shown?;
        if self.state_at.is_some_and(|t| now.duration_since(t) < STATE_EVERY) {
            return None;
        }
        self.state_at = Some(now);
        let mut st = MixState::from_mixer(&mut self.mixer, view, &name);
        st.selected = self.selected;
        Some(st)
    }
}

/// The pad channels as the sampler sees them (`PadOut`): pad `p` of the
/// playing kit adds into pad channel `p`. It notes which pads played.
pub struct Pads<'a> {
    mixer: &'a mut Mixer,
    played: u32,
}

impl Pads<'_> {
    /// The pads that played this period, a bit each.
    pub fn played(&self) -> u32 { self.played }
}

impl PadOut for Pads<'_> {
    fn pad(&mut self, pad: usize) -> Option<(&mut [f32], &mut [f32])> {
        if pad >= PADS {
            return None;
        }
        self.played |= 1 << pad;
        self.mixer.pad_input(pad)
    }
}

/// The mixer's context for a period: the transport's tempo, position
/// (beats) and whether it rolls, from the period's first span.
pub fn ctx(rate: u32, frame: u64, bpm: f64, block: &mpfree_transport::Block) -> Ctx<'static> {
    let mut c = Ctx::new(rate, frame);
    c.bpm = bpm;
    if let Some(s) = block.spans().first() {
        c.bpm = s.bpm;
        c.playing = s.state.rolling();
        c.beats = s.tick_at(s.frame) / f64::from(mpfree_transport::PPQ);
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 44_100;

    fn tone(f: f32, amp: f32, at: usize) -> [f32; PERIOD] {
        std::array::from_fn(|i| amp * (std::f32::consts::TAU * f * (at + i) as f32 / RATE as f32).sin())
    }

    /// Pad 1 plays `drums` (as the sampler would, into its channel), the
    /// line input `line`.
    fn run(h: &mut MixHost, periods: usize, drums: f32, line: f32) -> (f32, bool) {
        let z = [0f32; PERIOD];
        let (mut l, mut r) = ([0f32; PERIOD], [0f32; PERIOD]);
        let (mut peak, mut caught) = (0f32, false);
        for p in 0..periods {
            let d = tone(110.0, drums, p * PERIOD);
            let i = tone(220.0, line, p * PERIOD);
            if drums > 0.0 {
                let mut pads = h.pads();
                let (pl, pr) = pads.pad(0).unwrap();
                pl[..PERIOD].iter_mut().zip(&d).for_each(|(o, x)| *o += x);
                pr[..PERIOD].iter_mut().zip(&d).for_each(|(o, x)| *o += x);
            }
            let ctx = Ctx::new(RATE, (p * PERIOD) as u64);
            let (pk, c) = h.period(&ctx, (&z, &z), (&z, &z), (&i, &i), &mut l, &mut r);
            // The loudest of the last half: after the glides.
            if p >= periods / 2 {
                peak = peak.max(pk);
            }
            caught |= c;
        }
        (peak, caught)
    }

    #[test]
    fn the_engines_reach_the_master_at_the_level() {
        let mut h = MixHost::new(RATE, -6.0, -6.0);
        let (peak, caught) = run(&mut h, 200, 0.5, 0.0);
        // 0.5 at -6 dB (about 0.25): pad 1 and the drums track at 0 dB, the master at the Level.
        assert!((0.24..0.26).contains(&peak), "{peak}");
        assert!(!caught);
        // The Level never goes above --gain-db.
        h.set_level(0.0);
        run(&mut h, 2, 0.5, 0.0);
        assert_eq!(h.level(), -6.0);
        h.set_level(-12.0);
        let (peak, _) = run(&mut h, 200, 0.5, 0.0);
        assert!((0.12..0.13).contains(&peak), "{peak}");
    }

    #[test]
    fn the_line_input_is_heard_only_while_open_and_monitored() {
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        assert_eq!(run(&mut h, 100, 0.0, 0.5).0, 0.0, "closed");
        h.follow(true, false);
        assert_eq!(run(&mut h, 100, 0.0, 0.5).0, 0.0, "open, not monitored");
        h.follow(true, true);
        let (peak, _) = run(&mut h, 200, 0.0, 0.5);
        assert!((0.49..0.51).contains(&peak), "{peak}");
        h.follow(false, true);
        assert!(run(&mut h, 200, 0.0, 0.5).0 < 1e-6, "closed again");
    }

    #[test]
    fn master_safety_catches_what_would_clip() {
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        let (peak, caught) = run(&mut h, 400, 1.6, 0.0);
        assert!(caught);
        assert!(peak <= 1.0, "{peak}");
    }

    #[test]
    fn the_panel_gets_states_while_it_shows_and_drives_the_mixer() {
        let mut h = MixHost::new(RATE, -6.0, -6.0);
        let t0 = Instant::now();
        assert!(h.poll_state(t0).is_none(), "closed");
        h.show(Some(MixView::Tracks));
        run(&mut h, 20, 0.5, 0.0);
        let st = h.poll_state(t0).expect("at once when opened");
        assert_eq!(st.strips.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Drums", "Synth", "Loop"]);
        assert!(st.strips[0].peak[0] > 0.1);
        assert_eq!(st.master.volume_db, -6.0);
        assert!(h.poll_state(t0 + STATE_EVERY / 2).is_none(), "not again so soon");
        assert!(h.poll_state(t0 + STATE_EVERY).is_some());
        // A fader on the panel; the master fader is the Level.
        assert_eq!(h.action(&MixAction::Set { strip: StripId::Track(DRUMS), id: mx::MUTE, value: 1.0 }), None);
        assert_eq!(h.action(&MixAction::Set { strip: StripId::Master, id: mx::VOLUME, value: 3.0 }), Some(-6.0));
        assert_eq!(h.action(&MixAction::Select(StripId::Track(SYNTH))), None);
        run(&mut h, 200, 0.5, 0.0);
        let st = h.poll_state(t0 + STATE_EVERY * 3).unwrap();
        assert!(st.strips[0].mute);
        assert_eq!(st.selected, Some(StripId::Track(SYNTH)));
        // The meter held the mute's fade until that read; now it is silent.
        run(&mut h, 20, 0.5, 0.0);
        let st = h.poll_state(t0 + STATE_EVERY * 4).unwrap();
        assert_eq!(st.strip(StripId::Track(DRUMS)).unwrap().peak, [0.0; 2], "muted");
        h.action(&MixAction::View(MixView::Inputs));
        let st = h.poll_state(t0 + STATE_EVERY * 4).unwrap();
        assert_eq!((st.view, st.strips[0].name.as_str()), (MixView::Inputs, "Line in"));
        h.show(None);
        assert!(h.poll_state(t0 + STATE_EVERY * 10).is_none());
        assert_eq!(h.level(), -6.0);
    }

    #[test]
    fn the_pads_go_through_their_channels_into_the_drums_track() {
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        // The sampler's pads 1 and 3; pad 17 is not there.
        let mut pads = h.pads();
        pads.pad(0).unwrap().0[..PERIOD].fill(0.25);
        pads.pad(2).unwrap().0[..PERIOD].fill(0.5);
        assert!(pads.pad(PADS).is_none());
        let played = pads.played();
        assert_eq!(played, 0b101);
        let (mut l, mut r) = ([0f32; PERIOD], [0f32; PERIOD]);
        h.drums(played, &mut l, &mut r);
        assert_eq!((l[0], l[PERIOD - 1], r[0]), (0.75, 0.75, 0.0), "what the looper records");
        // Pad 1 muted on Pad Mix: the rest still reach the master through Drums.
        h.action(&MixAction::Set { strip: StripId::Pad(0), id: mx::MUTE, value: 1.0 });
        h.show(Some(MixView::Pads));
        let st = h.poll_state(Instant::now()).unwrap();
        assert_eq!(st.strips.len(), PADS);
        assert_eq!((st.strips[0].name.as_str(), st.strips[0].output_label()), ("Pad 1", "Track 1".to_string()));
        // (Master safety's DC blocker is still forgetting the first block's
        // DC: about -100 dBFS.)
        let muted = run(&mut h, 200, 0.5, 0.0).0;
        assert!(muted < 1e-4, "pad 1 muted: {muted}");
        h.action(&MixAction::Set { strip: StripId::Pad(0), id: mx::MUTE, value: 0.0 });
        let (peak, _) = run(&mut h, 200, 0.5, 0.0);
        assert!((0.49..0.51).contains(&peak), "{peak}");
    }

    #[test]
    fn the_context_follows_the_transport() {
        let block = mpfree_transport::Block::default();
        let c = ctx(RATE, 128, 97.0, &block);
        assert_eq!((c.rate, c.frame, c.bpm, c.playing), (RATE, 128, 97.0, false));
    }
}
