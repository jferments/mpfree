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
//! The Play page's effects are inserts from the start ([`PlayFx`]): a DJ
//! filter on the Drums and Loop tracks (the Filter Q-Links), a 1/8 dotted
//! delay on Return A (the Delay send Q-Links send to it), and Perform FX
//! (`mpfree_fx::Perform`) on the master, before Master safety: the throws
//! hold its pads, the XY pad plays its DJ filter and echo, and the FX mix
//! Q-Link is its Mix. Each rests (costs next to nothing) until it is used.
//!
//! Parameter locks: the clips' locks on strip controls and inserts go
//! through the mod engine ([`MixHost::lock`], `modmix.rs`), and once a
//! control has been locked the host's own turns of it go through the
//! engine too ([`MixHost::set`]), so the end of a lock puts back the last
//! turn.
//!
//! The screen: while the mixer panel shows ([`MixHost::show`]), the audio
//! loop sends a [`MixState`] every [`STATE_EVERY`] (it takes the meters),
//! and applies what the panel sends ([`MixHost::action`]).
//!
//! Nothing in [`MixHost::period`] allocates. Its time is in the "mix" stage
//! of CPU by engine; the state for the screen allocates, a few times a
//! second while the panel shows, as the other pages' states do.

use std::time::{Duration, Instant};

use mpfree_fx::ids::mixer::output;
use mpfree_fx::ids::mixer::{self as mx, monitor, source};
use mpfree_fx::ids::{delay, dj_filter};
use mpfree_fx::{Delay, DjFilter, Edit, Insert, Mixer, MixerConfig, StripId, CAPTURES};
use mpfree_mixer_ui::{default_name, MixAction, MixState, MixView};
use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_play::mpfree_param::{Owner, ParamAddr};
use mpfree_sampler::engine::PadOut;

use crate::PERIOD;
use mpfree_fx::ids::perform as perf;

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
/// The Play page's inserts: the DJ filters' slot on the Drums and Loop
/// tracks, the delay's return, and the throws' slot on the master.
const FILTER_SLOT: usize = 0;
/// [`MixHost::poll_built`]'s line when the builder's queue was full and an
/// effect taken out of a chain was freed on the audio thread.
const DROPPED_HERE: &str =
    "mixer: an effect taken out was freed on the audio thread (the builder's queue was full)";
pub const DELAY_RETURN: usize = 0;
const PERFORM_SLOT: usize = 0;
/// A send this low is off.
pub const SEND_OFF_DB: f32 = -60.0;
/// The capture point the sampler resamples from (the Rec tab's Source):
/// the last of the mixer's [`CAPTURES`], so the others stay free for
/// print, stems and the looper.
pub const SAMPLER_CAPTURE: usize = CAPTURES - 1;

/// What the Play page's Q-Links and speakers show, read from the mixer (so
/// the mixer panel's changes show there too).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayLevels {
    /// The Drums and Loop tracks' sends to the delay, dB.
    pub drums_send_db: f32,
    pub loop_send_db: f32,
    /// The Loop track's and the line input's faders, dB.
    pub loop_db: f32,
    pub line_db: f32,
    /// The DJ filters, -100 (low-pass) to 100 (high-pass); 0 is off.
    pub drums_filter: f32,
    pub loop_filter: f32,
    /// The XY pad's FX mix, 0 to 1.
    pub fx_mix: f32,
    /// Pads muted (a bit each), as the speakers show them.
    pub pad_mutes: u16,
}

/// A change from the Play page for the mixer. The page says what it wants
/// (a filter's place, a throw held, where the XY pad is); which engine and
/// which parameter make it is the mixer's business ([`MixHost::play`]), so
/// another performance engine swaps in here without the page changing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlayFx {
    /// A strip's parameter (a fader, a send).
    Strip { strip: StripId, id: u16, value: f32 },
    /// A track's DJ filter: -100 (low-pass) to 100 (high-pass), 0 off.
    Filter { strip: StripId, position: f32 },
    /// A speaker: mute or unmute a pad.
    PadMute { pad: usize, mute: bool },
    /// Drop drums: these pads (a bit each) muted while held, as well.
    Drop(u16),
    /// A throw held or let go.
    Throw { fx: ThrowFx, down: bool },
    /// The XY pad: where the finger is (0 to 1 each way), and whether it
    /// is down.
    Xy { x: f32, y: f32, down: bool },
}

/// The performance effects a throw holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThrowFx {
    Delay,
    Reverb,
    Stutter,
    TapeStop,
    FilterDip,
    BeatRepeat,
    VinylBrake,
}

impl ThrowFx {
    /// The Perform FX pad (0 to 15) that plays it, in the default bank:
    /// Stutter 1/16, Tape stop 1/4, Vinyl brake 1/2, Echo throw 1/8 dotted,
    /// LP sweep 1 bar, Beat repeat 1/4 and Wash throw.
    pub fn pad(self) -> usize {
        match self {
            ThrowFx::Stutter => 0,
            ThrowFx::TapeStop => 2,
            ThrowFx::VinylBrake => 3,
            ThrowFx::Delay => 9,
            ThrowFx::FilterDip => 10,
            ThrowFx::BeatRepeat => 14,
            ThrowFx::Reverb => 15,
        }
    }
}

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
    /// The mod engine between the clips' locks and the mixer.
    pub modu: crate::modmix::ModMix,
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
    /// Pads muted by their speakers or the panel, and by Drop drums.
    pad_mutes: u16,
    dropped: u16,
    /// Builds the chain editor's effects, and drops the ones edits take
    /// out, off the audio thread (control.rs). None: here (the tests).
    builder: Option<crate::control::Builder>,
    /// A built effect the mixer had no room for yet (its edits were full),
    /// put in on a later period.
    landing: Option<crate::control::Built>,
}

impl MixHost {
    /// Made before the sound starts (it allocates). `level_db` is the
    /// Level now, `max_db` the most it goes to.
    pub fn new(rate: u32, level_db: f32, max_db: f32) -> MixHost {
        let cfg = MixerConfig {
            inputs: 1,
            pads: PADS,
            tracks: TRACKS,
            slots: SLOTS,
            max_latency_ms: 50.0,
        };
        let mut mixer = Mixer::new(cfg, rate, PERIOD);
        for p in 0..PADS {
            mixer.set_now(StripId::Pad(p), mx::OUTPUT, output::TRACK);
            mixer.set_now(StripId::Pad(p), mx::OUTPUT_TRACK, DRUMS as f32);
        }
        mixer.set_now(StripId::Master, mx::VOLUME, level_db.min(max_db));
        mixer.set_now(LINE, mx::SOURCE, source::OFF);
        mixer.set_now(LINE, mx::MONITOR, monitor::OFF);
        // The Play page's effects.
        let ret = StripId::Return(DELAY_RETURN);
        let mut echo = Delay::new();
        echo.set_param(delay::MIX, 100.0);
        let inserts: [(StripId, Box<dyn Node>); 4] = [
            (StripId::Track(DRUMS), Box::new(DjFilter::new())),
            (StripId::Track(LOOP), Box::new(DjFilter::new())),
            (ret, Box::new(echo)),
            (StripId::Master, Box::new(mpfree_fx::Perform::new())),
        ];
        for (strip, node) in inserts {
            let insert = Insert::new(node, rate, PERIOD);
            // Room for these is made above (SLOTS), so this never fails.
            let _ = mixer.edit(Edit::Insert {
                strip,
                at: 0,
                insert,
            });
        }
        for t in [DRUMS, LOOP] {
            mixer.set_now(StripId::Track(t), mx::send(DELAY_RETURN), SEND_OFF_DB);
        }
        MixHost {
            mixer,
            modu: crate::modmix::ModMix::new(rate),
            max_db,
            monitor: false,
            line_open: false,
            overs: 0,
            shown: None,
            selected: None,
            state_at: None,
            pad_mutes: 0,
            dropped: 0,
            builder: None,
            landing: None,
        }
    }

    /// From now on effects added in the chain editor are built on the
    /// `mpfree-build` thread, and the effects edits take out are dropped
    /// there (control.rs). Allocates and starts the thread: call it before
    /// the loop.
    pub fn build_off_thread(&mut self) -> std::io::Result<()> {
        self.builder = Some(crate::control::Builder::start()?);
        Ok(())
    }

    /// Adds effect `id` at the end of `strip`'s chain. With a builder it
    /// is built there and goes in a few periods later
    /// ([`MixHost::poll_built`]); without one, here. Err: the mixer is busy
    /// (too many edits or builds under way), or no effect has that id.
    pub fn add_insert(&mut self, strip: StripId, id: &'static str) -> Result<(), &'static str> {
        let rate = self.mixer.rate();
        if let Some(b) = self.builder.as_mut() {
            return if b.build(strip, id, rate, PERIOD) {
                Ok(())
            } else {
                Err("The mixer is busy")
            };
        }
        let node = mpfree_fx_ui::make(id).ok_or("No such effect")?;
        let at = self.mixer.chain(strip).map_or(0, |c| c.len());
        self.mixer
            .edit(Edit::Insert {
                strip,
                at,
                insert: Insert::new(node, rate, PERIOD),
            })
            .map_err(|_| "The mixer is busy")
    }

    /// Effects asked for and not in the mixer's edits yet.
    pub fn inserts_building(&self) -> usize {
        self.builder.as_ref().map_or(0, |b| b.pending()) + usize::from(self.landing.is_some())
    }

    /// Once a period: puts the effects the builder made into their chains
    /// (at the end), and hands the ones edits took out back to be dropped
    /// off the audio thread. Returns a line for the log when an effect
    /// could not be made. No allocation; never waits.
    pub fn poll_built(&mut self) -> Option<&'static str> {
        let mut said = None;
        loop {
            let built = match self.landing.take() {
                Some(b) => b,
                None => match self.builder.as_mut().and_then(|b| b.take()) {
                    Some(b) => b,
                    None => break,
                },
            };
            let Some(insert) = built.insert else {
                said = Some("mixer: the chain editor asked for an effect that has no maker");
                continue;
            };
            let Some(at) = self.mixer.chain(built.strip).map(|c| c.len()) else {
                said = Some("mixer: an effect was built for a strip the mixer does not have");
                if !self.drop_off_thread(insert) {
                    said = Some(DROPPED_HERE);
                }
                continue;
            };
            if let Err(Edit::Insert { insert, .. }) = self.mixer.edit(Edit::Insert {
                strip: built.strip,
                at,
                insert,
            }) {
                // The mixer's edits are full: it goes in on a later period.
                self.landing = Some(crate::control::Built {
                    strip: built.strip,
                    id: built.id,
                    insert: Some(insert),
                });
                break;
            }
        }
        while let Some(old) = self.mixer.pop_retired() {
            if !self.drop_off_thread(old) {
                said = Some(DROPPED_HERE);
            }
        }
        said
    }

    /// Drops `insert` on the builder's thread, or here without one (the
    /// tests). False if a builder's queue was full and it was dropped here.
    fn drop_off_thread(&mut self, insert: Insert) -> bool {
        match self.builder.as_mut() {
            Some(b) => b.drop_later(insert),
            None => {
                drop(insert);
                true
            }
        }
    }

    /// A change from the Play page.
    pub fn play(&mut self, fx: PlayFx) {
        match fx {
            PlayFx::Strip { strip, id, value } => {
                self.set(strip, id, value);
            }
            PlayFx::Filter { strip, position } => {
                self.insert(strip, FILTER_SLOT, dj_filter::POSITION, position);
            }
            PlayFx::Throw { fx, down } => {
                // The throw holds its Perform FX pad (the default bank's,
                // as the FX thread laid it out for the throws).
                self.insert(
                    StripId::Master,
                    PERFORM_SLOT,
                    perf::hold(fx.pad()),
                    if down { 1.0 } else { 0.0 },
                );
            }
            PlayFx::Xy { x, y, .. } => {
                let pad = mpfree_fx::XyPad::perform();
                for (axis, pos) in [(pad.x, x), (pad.y, y)] {
                    self.insert(StripId::Master, PERFORM_SLOT, axis.param, axis.value(pos));
                }
            }
            PlayFx::PadMute { pad, mute } if pad < PADS => {
                if mute {
                    self.pad_mutes |= 1 << pad;
                } else {
                    self.pad_mutes &= !(1 << pad);
                }
                self.apply_mutes();
            }
            PlayFx::PadMute { .. } => {}
            PlayFx::Drop(pads) => {
                self.dropped = pads;
                self.apply_mutes();
            }
        }
    }

    fn insert(&mut self, strip: StripId, slot: usize, id: u16, value: f32) {
        let addr = crate::modmix::strip_of(strip).and_then(|s| {
            let uid = self.mixer.chain(strip)?.uid(slot)?;
            Some(ParamAddr::new(Owner::Insert(s, uid), id))
        });
        if addr.is_some_and(|a| self.modu.set(a, value)) {
            return;
        }
        self.mixer
            .queue_insert(strip, slot, Timed::new(0, NodeEvent::Param { id, value }));
    }

    /// A strip control as the mixer panel set it, for UNDO: a pad's mute
    /// is its own, not Drop drums' held over it.
    pub fn value(&self, strip: StripId, id: u16) -> f32 {
        match strip {
            StripId::Pad(p) if id == mx::MUTE && p < PADS => {
                f32::from(u8::from(self.pad_mutes & (1 << p) != 0))
            }
            _ => self.mixer.param(strip, id),
        }
    }

    /// Sets a strip control from the start of the next period: through the
    /// mod engine once a lock has made it a target, else straight to the
    /// mixer.
    pub fn set(&mut self, strip: StripId, id: u16, value: f32) {
        let addr = crate::modmix::strip_of(strip).map(|s| ParamAddr::new(Owner::Strip(s), id));
        if !addr.is_some_and(|a| self.modu.set(a, value)) {
            self.mixer.set(strip, id, value);
        }
    }

    /// A clip's parameter lock (`value`) or its end (`None`), `offset`
    /// frames into the coming period.
    pub fn lock(&mut self, offset: u32, addr: ParamAddr, value: Option<f32>) {
        self.modu.lock(&self.mixer, offset, addr, value);
    }

    /// A clip's move lane value (an FX move), `offset` frames into the
    /// coming period.
    pub fn lane(&mut self, offset: u32, addr: ParamAddr, value: f32) {
        self.modu.lane(&self.mixer, offset, addr, value);
    }

    fn apply_mutes(&mut self) {
        for p in 0..PADS {
            let on = (self.pad_mutes | self.dropped) & (1 << p) != 0;
            let want = if on { 1.0 } else { 0.0 };
            if self.mixer.param(StripId::Pad(p), mx::MUTE) != want {
                self.set(StripId::Pad(p), mx::MUTE, want);
            }
        }
    }

    /// Master safety's limiting now, dB (0, or a positive number of dB
    /// taken off), for the master meter.
    pub fn gain_reduction_db(&self) -> f32 {
        use mpfree_node::Node as _;
        -self
            .mixer
            .safety()
            .param(mpfree_fx::ids::master::REDUCTION)
            .min(0.0)
    }

    /// The pads muted by their speakers or SHIFT + TRACK MUTE (a bit
    /// each; Drop drums' are not counted).
    pub fn pad_mutes(&self) -> u16 {
        self.pad_mutes
    }

    /// What the Play page shows. Cheap: a few reads.
    pub fn play_levels(&self) -> PlayLevels {
        let m = &self.mixer;
        let insert = |s: StripId, slot: usize, id: u16| {
            m.chain(s)
                .and_then(|c| c.node(slot))
                .map_or(0.0, |n| n.param(id))
        };
        PlayLevels {
            drums_send_db: m.param(StripId::Track(DRUMS), mx::send(DELAY_RETURN)),
            loop_send_db: m.param(StripId::Track(LOOP), mx::send(DELAY_RETURN)),
            loop_db: m.param(StripId::Track(LOOP), mx::VOLUME),
            line_db: m.param(LINE, mx::VOLUME),
            drums_filter: insert(StripId::Track(DRUMS), FILTER_SLOT, dj_filter::POSITION),
            loop_filter: insert(StripId::Track(LOOP), FILTER_SLOT, dj_filter::POSITION),
            fx_mix: insert(StripId::Master, PERFORM_SLOT, perf::MIX) / 100.0,
            pad_mutes: self.pad_mutes,
        }
    }

    /// Points the sampler's capture point at `src`, or frees it for the
    /// line input. Takes effect from the next block; false if the mixer
    /// has no such strip (the capture point is then free).
    pub fn set_rec_source(&mut self, src: crate::sampler::RecSource) -> bool {
        match src {
            crate::sampler::RecSource::Line => {
                self.mixer.set_capture(SAMPLER_CAPTURE, None);
                true
            }
            crate::sampler::RecSource::Mix(strip, point) => {
                let ok = self
                    .mixer
                    .set_capture(SAMPLER_CAPTURE, Some((strip, point)));
                if !ok {
                    self.mixer.set_capture(SAMPLER_CAPTURE, None);
                }
                ok
            }
        }
    }

    /// What the sampler's capture point took in the last period, and how
    /// many frames late it hears the transport's block; None while it is
    /// free.
    pub fn rec_capture(&self) -> Option<(&[f32], &[f32], u32)> {
        let (l, r) = self.mixer.capture(SAMPLER_CAPTURE)?;
        let latency = self.mixer.capture_latency(SAMPLER_CAPTURE)?;
        Some((l, r, latency))
    }

    /// The Level (Q-Link 1, the slider): the master fader.
    pub fn set_level(&mut self, db: f32) {
        self.set(StripId::Master, mx::VOLUME, db.min(self.max_db));
    }

    /// The master fader now, dB.
    #[cfg(test)]
    pub fn level(&self) -> f32 {
        self.mixer.param(StripId::Master, mx::VOLUME)
    }

    /// Follows the looper: whether the line input is open, and whether it
    /// is heard (Monitor). Cheap to call every period.
    pub fn follow(&mut self, line_open: bool, monitor_on: bool) {
        if line_open != self.line_open {
            self.line_open = line_open;
            self.set(
                LINE,
                mx::SOURCE,
                if line_open {
                    source::STEREO
                } else {
                    source::OFF
                },
            );
        }
        if monitor_on != self.monitor {
            self.monitor = monitor_on;
            self.set(
                LINE,
                mx::MONITOR,
                if monitor_on {
                    monitor::ON
                } else {
                    monitor::OFF
                },
            );
        }
    }

    /// The pad channels for the sampler's `render_routed` this period.
    pub fn pads(&mut self) -> Pads<'_> {
        Pads {
            mixer: &mut self.mixer,
            played: 0,
        }
    }

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
    pub fn period(
        &mut self,
        ctx: &Ctx,
        synth: Stereo,
        looped: Stereo,
        line: Stereo,
        left: &mut [f32; PERIOD],
        right: &mut [f32; PERIOD],
    ) -> (f32, bool) {
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
        self.modu.process(ctx, PERIOD, &mut self.mixer);
        self.mixer.process(ctx, left, right);
        let peak = left
            .iter()
            .chain(right.iter())
            .fold(0f32, |m, s| m.max(s.abs()));
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

    /// The mixer view the panel shows, if it is open.
    pub fn shown(&self) -> Option<MixView> {
        self.shown
    }

    /// What the panel sent. Returns the master fader's new level when it
    /// moved, so the Play page's Level follows.
    pub fn action(&mut self, a: &MixAction) -> Option<f32> {
        match *a {
            MixAction::Set {
                strip: StripId::Master,
                id: mx::VOLUME,
                value,
            } => {
                let db = value.min(self.max_db);
                self.set(StripId::Master, mx::VOLUME, db);
                self.state_at = None;
                Some(db)
            }
            MixAction::Set {
                strip: StripId::Pad(p),
                id: mx::MUTE,
                value,
            } if p < PADS => {
                // The speakers on the Play page follow.
                self.play(PlayFx::PadMute {
                    pad: p,
                    mute: value >= 0.5,
                });
                None
            }
            MixAction::Set { strip, id, value } => {
                self.set(strip, id, value);
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
            // Freeze, Thaw and Print: the strip menu stays shut until the
            // app sets `MixState::takes` and acts on them here.
            MixAction::Freeze(_)
            | MixAction::Thaw(_)
            | MixAction::Print(_)
            | MixAction::CancelTake(_) => None,
        }
    }

    /// The panel's next state, while it shows and [`STATE_EVERY`] has
    /// passed (at once after a change).
    pub fn poll_state(&mut self, now: Instant) -> Option<MixState> {
        let view = self.shown?;
        if self
            .state_at
            .is_some_and(|t| now.duration_since(t) < STATE_EVERY)
        {
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
    pub fn played(&self) -> u32 {
        self.played
    }
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
        std::array::from_fn(|i| {
            amp * (std::f32::consts::TAU * f * (at + i) as f32 / RATE as f32).sin()
        })
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

    /// UNDO on the mixer sets a pad's mute back to the pad's own, not to
    /// Drop drums' held over it.
    #[test]
    fn a_pads_mute_reads_as_its_own_under_drop_drums() {
        let mut h = MixHost::new(RATE, -6.0, -6.0);
        h.play(PlayFx::PadMute { pad: 2, mute: true });
        h.play(PlayFx::Drop(0b11));
        assert_eq!(h.value(StripId::Pad(2), mx::MUTE), 1.0);
        assert_eq!(
            h.value(StripId::Pad(0), mx::MUTE),
            0.0,
            "dropped, not muted"
        );
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
    fn the_samplers_capture_point_hears_the_strip_picked() {
        use crate::sampler::RecSource;
        use mpfree_fx::CapturePoint;
        let mut h = MixHost::new(RATE, -6.0, -6.0);
        assert!(h.rec_capture().is_none(), "free until a source is picked");
        let master = RecSource::Mix(StripId::Master, CapturePoint::AfterSafety);
        let pad1 = RecSource::Mix(StripId::Pad(0), CapturePoint::AfterFader);
        // One period of pad 1 playing 0.5; what came out, and what the
        // capture point took.
        let z = [0f32; PERIOD];
        let mut at = 0;
        let mut once = |h: &mut MixHost| {
            let d = tone(110.0, 0.5, at);
            {
                let mut pads = h.pads();
                let (pl, pr) = pads.pad(0).unwrap();
                pl[..PERIOD].copy_from_slice(&d);
                pr[..PERIOD].copy_from_slice(&d);
            }
            let ctx = Ctx::new(RATE, at as u64);
            let (mut l, mut r) = ([0f32; PERIOD], [0f32; PERIOD]);
            h.period(&ctx, (&z, &z), (&z, &z), (&z, &z), &mut l, &mut r);
            at += PERIOD;
            let peak = |x: &[f32]| x.iter().fold(0f32, |m, v| m.max(v.abs()));
            let cap = h.rec_capture().map(|(cl, cr, lat)| {
                assert_eq!((cl.len(), cr.len()), (PERIOD, PERIOD));
                (cl == l.as_slice() && cr == r.as_slice(), peak(cl), lat)
            });
            (peak(&l), cap)
        };
        assert!(h.set_rec_source(master));
        for _ in 0..200 {
            once(&mut h);
        }
        let (out, cap) = once(&mut h);
        let (same, _, lat) = cap.unwrap();
        assert!(same, "the master as heard is what comes out");
        assert!((0.24..0.26).contains(&out), "{out}");
        assert!(lat < RATE / 20, "{lat} frames late");
        // Pad 1's channel after its fader: before the Level.
        assert!(h.set_rec_source(pad1));
        for _ in 0..50 {
            once(&mut h);
        }
        let (_, cap) = once(&mut h);
        let (same, peak, _) = cap.unwrap();
        assert!(!same && (0.49..0.51).contains(&peak), "{peak}");
        // Back to the line input: the capture point is free again.
        assert!(h.set_rec_source(RecSource::Line));
        once(&mut h);
        assert!(h.rec_capture().is_none());
        // A strip the mixer doesn't have.
        let none = RecSource::Mix(StripId::Pad(40), CapturePoint::AfterFader);
        assert!(!h.set_rec_source(none));
        assert!(h.rec_capture().is_none());
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
        assert_eq!(
            st.strips
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            ["Drums", "Synth", "Loop"]
        );
        assert!(st.strips[0].peak[0] > 0.1);
        assert_eq!(st.master.volume_db, -6.0);
        assert!(
            h.poll_state(t0 + STATE_EVERY / 2).is_none(),
            "not again so soon"
        );
        assert!(h.poll_state(t0 + STATE_EVERY).is_some());
        // A fader on the panel; the master fader is the Level.
        assert_eq!(
            h.action(&MixAction::Set {
                strip: StripId::Track(DRUMS),
                id: mx::MUTE,
                value: 1.0
            }),
            None
        );
        assert_eq!(
            h.action(&MixAction::Set {
                strip: StripId::Master,
                id: mx::VOLUME,
                value: 3.0
            }),
            Some(-6.0)
        );
        assert_eq!(h.action(&MixAction::Select(StripId::Track(SYNTH))), None);
        run(&mut h, 200, 0.5, 0.0);
        let st = h.poll_state(t0 + STATE_EVERY * 3).unwrap();
        assert!(st.strips[0].mute);
        assert_eq!(st.selected, Some(StripId::Track(SYNTH)));
        // The meter held the mute's fade until that read; now it is silent.
        run(&mut h, 20, 0.5, 0.0);
        let st = h.poll_state(t0 + STATE_EVERY * 4).unwrap();
        assert_eq!(
            st.strip(StripId::Track(DRUMS)).unwrap().peak,
            [0.0; 2],
            "muted"
        );
        h.action(&MixAction::View(MixView::Inputs));
        let st = h.poll_state(t0 + STATE_EVERY * 4).unwrap();
        assert_eq!(
            (st.view, st.strips[0].name.as_str()),
            (MixView::Inputs, "Line in")
        );
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
        assert_eq!(
            (l[0], l[PERIOD - 1], r[0]),
            (0.75, 0.75, 0.0),
            "what the looper records"
        );
        // Pad 1 muted on Pad Mix: the rest still reach the master through Drums.
        h.action(&MixAction::Set {
            strip: StripId::Pad(0),
            id: mx::MUTE,
            value: 1.0,
        });
        h.show(Some(MixView::Pads));
        let st = h.poll_state(Instant::now()).unwrap();
        assert_eq!(st.strips.len(), PADS);
        assert_eq!(
            (st.strips[0].name.as_str(), st.strips[0].output_label()),
            ("Pad 1", "Track 1".to_string())
        );
        // (Master safety's DC blocker is still forgetting the first block's
        // DC: about -100 dBFS.)
        let muted = run(&mut h, 200, 0.5, 0.0).0;
        assert!(muted < 1e-4, "pad 1 muted: {muted}");
        h.action(&MixAction::Set {
            strip: StripId::Pad(0),
            id: mx::MUTE,
            value: 0.0,
        });
        let (peak, _) = run(&mut h, 200, 0.5, 0.0);
        assert!((0.49..0.51).contains(&peak), "{peak}");
    }

    #[test]
    fn the_throws_and_the_xy_pad_play_perform_fx() {
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        let perform = |h: &MixHost, id: u16| {
            h.mixer
                .chain(StripId::Master)
                .and_then(|c| c.node(PERFORM_SLOT))
                .map(|n| n.param(id))
        };
        // The inserts go in once each strip has ducked out (about 10 ms).
        run(&mut h, 10, 0.0, 0.0);
        assert_eq!(h.mixer.edits_pending(), 0);
        assert_eq!(perform(&h, perf::MIX), Some(100.0));
        assert_eq!(h.play_levels().fx_mix, 1.0);
        h.play(PlayFx::Throw {
            fx: ThrowFx::Delay,
            down: true,
        });
        h.play(PlayFx::Xy {
            x: 0.0,
            y: 1.0,
            down: true,
        });
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(
            perform(&h, perf::hold(9)),
            Some(1.0),
            "the echo throw's pad"
        );
        assert_eq!(perform(&h, perf::DJ_FILTER), Some(-100.0), "low-pass");
        assert_eq!(perform(&h, perf::ECHO), Some(100.0));
        h.play(PlayFx::Throw {
            fx: ThrowFx::Delay,
            down: false,
        });
        h.play(PlayFx::Xy {
            x: 0.5,
            y: 0.0,
            down: false,
        });
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(perform(&h, perf::hold(9)), Some(0.0));
        assert_eq!(perform(&h, perf::DJ_FILTER), Some(0.0), "off in the middle");
        // Each throw holds a pad of its own.
        let all = [
            ThrowFx::Delay,
            ThrowFx::Reverb,
            ThrowFx::Stutter,
            ThrowFx::TapeStop,
            ThrowFx::FilterDip,
            ThrowFx::BeatRepeat,
            ThrowFx::VinylBrake,
        ];
        let mut pads: Vec<usize> = all.iter().map(|t| t.pad()).collect();
        pads.sort_unstable();
        pads.dedup();
        assert_eq!(pads.len(), all.len());
    }

    /// Runs periods (polling as the app does) until track 2's chain has
    /// `len` inserts, for at most 5 s.
    fn until_chain(h: &mut MixHost, len: usize) -> usize {
        let strip = StripId::Track(SYNTH);
        let give_up = Instant::now() + Duration::from_secs(5);
        while h.mixer.chain(strip).unwrap().len() != len && Instant::now() < give_up {
            assert_eq!(h.poll_built(), None);
            run(h, 1, 0.0, 0.0);
            // Built on the builder thread when there is one.
            std::thread::sleep(Duration::from_micros(200));
        }
        assert_eq!(h.poll_built(), None);
        h.mixer.chain(strip).unwrap().len()
    }

    /// Adds then removes a Drive on track 2 `times` times, checking the
    /// chain each time. Removes glide out over a few periods.
    fn add_and_remove(h: &mut MixHost, times: usize) {
        let strip = StripId::Track(SYNTH);
        let base = h.mixer.chain(strip).unwrap().len();
        for round in 0..times {
            h.add_insert(strip, "mpfree.fx.drive").unwrap();
            assert_eq!(until_chain(h, base + 1), base + 1, "add {round}");
            assert_eq!(h.inserts_building(), 0);
            h.mixer.edit(Edit::Remove { strip, at: base }).ok().unwrap();
            assert_eq!(until_chain(h, base), base, "remove {round}");
        }
    }

    #[test]
    fn chain_edits_keep_landing_after_many_removes() {
        // The app never took retired inserts back from the mixer, which
        // stops applying edits once 32 are waiting: the chain editor froze
        // after 32 removes. poll_built takes them back every period.
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        add_and_remove(&mut h, 40);
    }

    #[test]
    fn effects_built_off_the_audio_thread_land_in_their_chain() {
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        h.build_off_thread().unwrap();
        add_and_remove(&mut h, 40);
        // An effect with no maker is said, and nothing lands.
        let strip = StripId::Track(SYNTH);
        let before = h.mixer.chain(strip).unwrap().len();
        h.add_insert(strip, "no.such.effect").unwrap();
        let give_up = Instant::now() + Duration::from_secs(5);
        let mut said = None;
        while said.is_none() && Instant::now() < give_up {
            said = h.poll_built();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(said.unwrap().contains("no maker"), "{said:?}");
        assert_eq!(h.mixer.chain(strip).unwrap().len(), before);
        assert_eq!(h.inserts_building(), 0);
    }

    #[test]
    fn the_context_follows_the_transport() {
        let block = mpfree_transport::Block::default();
        let c = ctx(RATE, 128, 97.0, &block);
        assert_eq!(
            (c.rate, c.frame, c.bpm, c.playing),
            (RATE, 128, 97.0, false)
        );
    }
}
