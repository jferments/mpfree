//! Synth-voice pads' own synths (the Pads tab's Instrument pads, the
//! redesign's 6b; Jay's card, 18 Nov 2026 20:15Z: "Own pad channel, Synth
//! tab's sound").
//!
//! - **A copy per pad.** Each Instrument pad of a kit plays a copy of its
//!   engine, made as the Synth tab has that engine's sound now (as a
//!   track's own synth starts, `instruments.rs`), into the strip the pad's
//!   sample would play into (`MixHost::pad_dest`: its pad channel, a lent
//!   one, or its kit's track), so its fader, effects and sends are the
//!   pad's, not the shared Synth strip's.
//! - **Sixteen at most.** Up to [`SLOTS`] pads have a copy; a pad past
//!   them plays through the Synth channel as before, counted, logged and
//!   said on the screen ([`PadSynths::plan`]). Pads that have one keep it,
//!   so a new Instrument pad never takes another's.
//! - **Following the Synth tab.** A copy's values follow the Synth tab's
//!   for its engine ([`PadSynths::follow`], a few times a second), and a
//!   new design (a preset with its own effects or macros) makes the copy
//!   again. Its tuning is the Synth tab's.
//! - **Rest.** A copy renders only while it has notes sounding or events
//!   to play (an old one ringing out included), so sixteen quiet pads cost
//!   next to nothing.
//! - **Putting one in** allocates nowhere here: the builder thread makes
//!   the [`Instrument`] (`control::make_instrument`, the job naming the
//!   pad: [`PadFor`]) and the audio loop puts it in with
//!   [`PadSynths::put`]. An old copy rings out into the same strip and
//!   comes back through [`PadSynths::pop_retired`], to be dropped off the
//!   audio thread.
//!
//! - **On the sound cores.** With Prefs "Sound cores" on two or three, the
//!   copies render on lanes on the other cores beside the audio thread,
//!   placed by what each costs ([`PadSynths::set_lanes`], as the tracks'
//!   own synths are); the sound is the same to the bit.
//!
//! Everything after [`PadSynths::new`] and [`PadSynths::set_lanes`] never
//! allocates, frees or locks.

use crate::synth::{Engine, Synth};
use mpfree_fx::Mixer;
use mpfree_node::{Ctx, NodeEvent, Timed, Tuning};
use mpfree_track::{Instrument, Outputs, TrackHost};

/// The most synth-voice pads with a synth of their own at once.
pub const SLOTS: usize = 16;

/// The host's slots: a copy for each of [`SLOTS`] pads, and as many again
/// for old copies ringing out.
pub const HOST_SLOTS: usize = 2 * SLOTS;

/// The tracks whose hits are remembered (every sequencer track).
const TRACKS: usize = mpfree_seq::MAX_TRACKS;
/// A kit's pads.
const PADS: usize = mpfree_sampler::model::PADS;
/// The kits whose pads past the sixteen are told of.
const KITS: usize = mpfree_sampler::engine::MAX_KITS;

/// How often (in calls, once a period) the copies look at the Synth tab's
/// values: about every 23 ms at 128 frames.
const FOLLOW_EVERY: u32 = 8;

/// A route's code: the slot above [`SLOT_SHIFT`], a pad strip's flag and
/// the strip's index below.
const SLOT_SHIFT: u32 = 20;
const PAD_FLAG: usize = 1 << 19;
const INDEX_MASK: usize = PAD_FLAG - 1;

/// Where a synth-voice pad's sound goes: a pad strip (its own channel, or
/// one lent to it) or a track's strip (its kit's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dest {
    Pad(usize),
    Track(usize),
}

impl Dest {
    fn code(self, slot: usize) -> usize {
        let i = match self {
            Dest::Pad(i) => PAD_FLAG | (i & INDEX_MASK),
            Dest::Track(i) => i & INDEX_MASK,
        };
        slot << SLOT_SHIFT | i
    }
}

/// The pad a copy is made for, with its engine's design count when it was
/// asked for ([`Synth::design_of`]): carried by the builder's job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PadFor {
    pub kit: u8,
    pub pad: u8,
    pub engine: Engine,
    pub design: u32,
}

impl PadFor {
    fn at(&self) -> (usize, usize) {
        (usize::from(self.kit), usize::from(self.pad))
    }
}

/// What [`PadSynths::plan`] found: copies to ask the builder for (at most
/// [`SLOTS`]), and how many synth-voice pads play through the Synth
/// channel, past the sixteen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub asks: [Option<PadFor>; SLOTS],
    /// Pads past the sixteen now.
    pub over: usize,
    /// The first of them not told of before (kit, pad), if any.
    pub newly_over: Option<(usize, usize)>,
    /// The pads playing through the Synth channel
    /// ([`PadSynths::on_synth`]) changed: the Pads tab is told.
    pub changed: bool,
}

/// The synth-voice pads' copies. See the module docs.
pub struct PadSynths {
    host: TrackHost,
    /// What each slot's copy is for, while it holds one.
    held: [Option<PadFor>; HOST_SLOTS],
    /// Asked of the builder, not in yet.
    asked: [Option<PadFor>; SLOTS],
    /// Asked for, but the builder couldn't make it: not asked again
    /// until its pad or its engine's design changes.
    failed: [Option<PadFor>; SLOTS],
    /// The slot and note each track's hit on each pad started (track *
    /// [`PADS`] + pad), so its let-go and pressure find them.
    voices: Box<[Option<(u8, u8)>]>,
    /// Slots sent anything since the last render: awake for it.
    woke: [bool; HOST_SLOTS],
    /// Pads past the sixteen already told of, a bit each, by kit.
    told: [u128; KITS],
    /// The synth-voice pads that play through the Synth channel, a bit
    /// each, by kit: past the sixteen, or their copy couldn't be made.
    on_synth: [u128; KITS],
    /// Hits on a synth-voice pad that had no copy (the Synth channel's).
    fell_back: u64,
    /// Calls to [`PadSynths::follow`], for its pace.
    ticks: u32,
    /// Values sent to copies to follow the Synth tab.
    followed: u64,
}

impl PadSynths {
    /// Made before the sound starts (it allocates).
    pub fn new(rate: u32, max_block: usize) -> PadSynths {
        let mut host = TrackHost::new(HOST_SLOTS, rate, max_block);
        for s in 0..HOST_SLOTS {
            host.route(s, None);
        }
        PadSynths {
            host,
            held: [None; HOST_SLOTS],
            asked: [None; SLOTS],
            failed: [None; SLOTS],
            voices: vec![None; TRACKS * PADS].into_boxed_slice(),
            woke: [false; HOST_SLOTS],
            told: [0; KITS],
            on_synth: [0; KITS],
            fell_back: 0,
            ticks: 0,
            followed: 0,
        }
    }

    /// The slot holding pad `pad` of kit `kit`'s copy.
    fn slot_of(&self, kit: usize, pad: usize) -> Option<usize> {
        self.held
            .iter()
            .position(|h| h.is_some_and(|h| h.at() == (kit, pad)))
    }

    /// The kit and pad slot `slot`'s copy plays, while it holds one.
    pub fn pad_of(&self, slot: usize) -> Option<(usize, usize)> {
        self.held.get(slot).copied().flatten().map(|h| h.at())
    }

    /// Whether pad `pad` of kit `kit` has a copy of `engine` of its own.
    #[cfg(test)]
    pub fn has(&self, kit: usize, pad: usize, engine: Engine) -> bool {
        self.slot_of(kit, pad)
            .is_some_and(|s| self.held[s].is_some_and(|h| h.engine == engine))
    }

    /// How many pads have a copy.
    pub fn count(&self) -> usize {
        self.held.iter().filter(|h| h.is_some()).count()
    }

    /// Hits that played through the Synth channel because their pad had
    /// no copy (past the sixteen, or while its copy was being made).
    #[cfg(test)]
    pub fn fell_back(&self) -> u64 {
        self.fell_back
    }

    /// Values sent to the copies to follow the Synth tab, so far.
    #[cfg(test)]
    pub fn followed(&self) -> u64 {
        self.followed
    }

    /// The status line's part: how many pads have their own synth, the
    /// hits that played through the Synth channel for want of one, and
    /// the values followed. None while no pad has had one.
    pub fn describe(&self) -> Option<String> {
        let n = self.count();
        (n > 0 || self.fell_back > 0 || self.followed > 0).then(|| {
            format!(
                "{n} of {SLOTS} pads, {} hits through the Synth channel, {} values followed",
                self.fell_back, self.followed
            )
        })
    }

    // ------------------------------------------------------------ notes

    /// Track `track`'s hit on pad `pad` of kit `kit`, a synth-voice pad of
    /// `engine` at `note`: on the pad's copy, `offset` frames into the next
    /// render. A pad still holding a note from this track ends it first.
    /// False if the pad has no copy of that engine (the hit is the shared
    /// synth's then, and counted). Never allocates.
    #[allow(clippy::too_many_arguments)]
    pub fn note_on(
        &mut self,
        track: u16,
        kit: usize,
        pad: usize,
        engine: Engine,
        note: u8,
        velocity: u8,
        offset: u32,
    ) -> bool {
        let Some(s) = self
            .slot_of(kit, pad)
            .filter(|&s| self.held[s].is_some_and(|h| h.engine == engine))
        else {
            self.fell_back += 1;
            return false;
        };
        let Some(k) = voice_key(track, pad) else {
            return false;
        };
        self.end_voice(k, offset);
        if note > 127 || velocity == 0 {
            return true;
        }
        self.host.note_on(s, offset, note, velocity.min(127));
        self.voices[k] = Some((s as u8, note));
        self.woke[s] = true;
        true
    }

    /// That hit let go: its note ends (once nothing else holds it). False
    /// if no copy held it (the shared synth's, then).
    pub fn note_off(&mut self, track: u16, pad: usize, offset: u32) -> bool {
        voice_key(track, pad).is_some_and(|k| self.end_voice(k, offset))
    }

    /// Pressure on that hit's note. False if no copy holds it.
    pub fn pressure(&mut self, track: u16, pad: usize, value: u8, offset: u32) -> bool {
        let Some((s, note)) = voice_key(track, pad).and_then(|k| self.voices[k]) else {
            return false;
        };
        let s = usize::from(s);
        self.host.event(
            s,
            Timed::new(
                offset,
                NodeEvent::Pressure {
                    note,
                    value: value.min(127),
                },
            ),
        );
        self.woke[s] = true;
        true
    }

    fn end_voice(&mut self, k: usize, offset: u32) -> bool {
        let Some((s, note)) = self.voices[k].take() else {
            return false;
        };
        let s = usize::from(s);
        self.host.note_off(s, offset, note);
        self.woke[s] = true;
        true
    }

    /// Track `track` went silent (muted, or another soloed): the notes its
    /// hits hold end.
    pub fn end_track(&mut self, track: u16, offset: u32) {
        for pad in 0..PADS {
            self.note_off(track, pad, offset);
        }
    }

    /// Every note ends with its release.
    pub fn all_off(&mut self) {
        self.voices.fill(None);
        self.host.all_off();
        self.woke = [true; HOST_SLOTS];
    }

    /// STOP: every note ends, faded out.
    pub fn stop(&mut self) {
        self.voices.fill(None);
        self.host.stop();
        self.woke = [true; HOST_SLOTS];
    }

    // ------------------------------------------------------------ copies

    /// Which copies to ask for, given the synth-voice pads now (`pads`:
    /// kit, pad and engine, kit by kit in pad order). A copy whose pad
    /// isn't a synth-voice pad of its engine any more goes (it rings out);
    /// one made before its engine's design changed is asked for again (it
    /// stays until the new one is in). A new synth-voice pad is asked for
    /// while fewer than [`SLOTS`] pads have one or are waiting for one;
    /// the rest are counted in [`Plan::over`]. Never allocates.
    pub fn plan(
        &mut self,
        pads: impl Iterator<Item = (usize, usize, Engine)> + Clone,
        synth: &Synth,
    ) -> Plan {
        // Copies whose pads changed go.
        for s in 0..HOST_SLOTS {
            if let Some(h) = self.held[s] {
                if !wanted(pads.clone(), h) && self.host.swap(s, None).is_ok() {
                    self.held[s] = None;
                    self.forget_voices(s);
                }
            }
        }
        // Asks whose pads changed are forgotten (what lands is checked
        // again in `put`), and failures whose pad or design changed (it is
        // tried again).
        for a in &mut self.asked {
            if a.is_some_and(|p| !wanted(pads.clone(), p)) {
                *a = None;
            }
        }
        for f in &mut self.failed {
            if f.is_some_and(|p| !wanted(pads.clone(), p) || p.design != synth.design_of(p.engine))
            {
                *f = None;
            }
        }
        let mut plan = Plan::default();
        let mut n = 0;
        let mut ask = |plan: &mut Plan, p: PadFor| {
            if n < SLOTS {
                plan.asks[n] = Some(p);
                n += 1;
            }
        };
        let mut over_now = [0u128; KITS];
        let mut on_synth = [0u128; KITS];
        for (kit, pad, engine) in pads {
            if kit >= KITS || pad >= PADS {
                continue;
            }
            let design = synth.design_of(engine);
            let p = PadFor {
                kit: kit as u8,
                pad: pad as u8,
                engine,
                design,
            };
            let waiting = self.asked.iter().flatten().any(|a| *a == p);
            match self.slot_of(kit, pad).and_then(|s| self.held[s]) {
                // Its copy is up to date, or the new one is on its way.
                Some(h) if h == p || waiting => {}
                // Made before the design changed: again.
                Some(_) => {
                    if let Some(a) = self.asked.iter_mut().find(|a| a.is_none()) {
                        *a = Some(p);
                        ask(&mut plan, p);
                    }
                }
                None if waiting => {}
                // Its copy couldn't be made: the Synth channel's.
                None if self.failed.iter().flatten().any(|f| *f == p) => {
                    on_synth[kit] |= 1 << pad;
                }
                None => {
                    let taken = self.count()
                        + self
                            .asked
                            .iter()
                            .flatten()
                            .filter(|a| {
                                self.slot_of(usize::from(a.kit), usize::from(a.pad))
                                    .is_none()
                            })
                            .count();
                    match self.asked.iter_mut().find(|a| a.is_none()) {
                        Some(a) if taken < SLOTS => {
                            *a = Some(p);
                            ask(&mut plan, p);
                        }
                        _ => {
                            over_now[kit] |= 1 << pad;
                            plan.over += 1;
                            if self.told[kit] & 1 << pad == 0 && plan.newly_over.is_none() {
                                plan.newly_over = Some((kit, pad));
                            }
                        }
                    }
                }
            }
        }
        // A pad told of that has its copy now, or isn't one, can be told
        // of again later; the one told of now is told of once.
        for (told, now) in self.told.iter_mut().zip(over_now) {
            *told &= now;
        }
        if let Some((kit, pad)) = plan.newly_over {
            self.told[kit] |= 1 << pad;
        }
        for (on, over) in on_synth.iter_mut().zip(over_now) {
            *on |= over;
        }
        plan.changed = on_synth != self.on_synth;
        self.on_synth = on_synth;
        plan
    }

    /// The synth-voice pads that play through the Synth channel as the
    /// last [`PadSynths::plan`] found them, a bit each, by kit: those past
    /// the sixteen, and those whose copy couldn't be made.
    pub fn on_synth(&self) -> &[u128; KITS] {
        &self.on_synth
    }

    /// The builder couldn't make the copy asked for `p`: its pad plays
    /// through the Synth channel, and it isn't asked for again until the
    /// pad or its engine's design changes.
    pub fn failed(&mut self, p: PadFor) {
        self.unask(p);
        if let Some(f) = self.failed.iter_mut().find(|f| f.is_none()) {
            *f = Some(p);
        }
    }

    /// The builder couldn't take the ask for `p`: asked again at the next
    /// [`PadSynths::plan`].
    pub fn unask(&mut self, p: PadFor) {
        for a in &mut self.asked {
            if *a == Some(p) {
                *a = None;
            }
        }
    }

    /// Puts in a copy the builder made for `p`, in `tuning`: into the
    /// slot its pad's copy had (which rings out), else a free one. False,
    /// and the copy comes back, if its pad wasn't asked for any more or no
    /// slot is free. Never allocates.
    pub fn put(&mut self, p: PadFor, inst: Instrument, tuning: &Tuning) -> Result<(), Instrument> {
        let Some(a) = self.asked.iter_mut().find(|a| **a == Some(p)) else {
            return Err(inst);
        };
        *a = None;
        let (kit, pad) = p.at();
        let s = match self.slot_of(kit, pad) {
            Some(s) => s,
            None if self.count() >= SLOTS => return Err(inst),
            None => {
                let free = |s: &usize| self.held[*s].is_none();
                match (0..HOST_SLOTS)
                    .filter(free)
                    .find(|&s| !self.host.ringing(s))
                    .or_else(|| (0..HOST_SLOTS).find(free))
                {
                    Some(s) => s,
                    None => return Err(inst),
                }
            }
        };
        self.host.set_tuning(s, tuning);
        if let Err((_, back)) = self.host.swap(s, Some(inst)) {
            return match back {
                Some(i) => Err(i),
                None => Ok(()),
            };
        }
        // The hits its old copy held ended with it.
        self.forget_voices(s);
        self.held[s] = Some(p);
        self.woke[s] = true;
        Ok(())
    }

    /// The hits slot `s`'s copy held, which ended with it (it went or was
    /// made again).
    fn forget_voices(&mut self, s: usize) {
        for v in self.voices.iter_mut() {
            if v.is_some_and(|(vs, _)| usize::from(vs) == s) {
                *v = None;
            }
        }
    }

    /// A copy taken out (after its ring-out), to drop off the audio thread.
    pub fn pop_retired(&mut self) -> Option<Instrument> {
        self.host.pop_retired()
    }

    // ------------------------------------------------------------ lanes

    /// The copies split over the audio thread and a lane per config, by
    /// what each costs (`TrackHost::set_lanes`, as the tracks' own synths
    /// are; [`crate::cores::Cores::start_pads`]). The sound into each
    /// strip is the same to the bit. Allocates and starts the lanes'
    /// threads: call it before the sound starts.
    pub fn set_lanes(&mut self, configs: Vec<mpfree_lanes::Config>) -> Vec<mpfree_lanes::Setup> {
        self.host.set_lanes(configs)
    }

    /// When the lanes' joins stop waiting this period (the period's one
    /// deadline, [`crate::cores::lane_deadline`]).
    pub fn set_lane_deadline(&mut self, at: Option<std::time::Instant>) {
        self.host.set_deadline(at);
    }

    /// After a late period: the lanes' copies back, waiting until
    /// `deadline` at most and past it as long as it takes (counted). Call
    /// it right after each write. Never allocates.
    pub fn settle(&mut self, deadline: std::time::Instant) {
        self.host.settle(deadline);
    }

    /// Lanes beside the audio thread (0: every copy plays on it).
    #[cfg(test)]
    pub fn lanes(&self) -> usize {
        self.host.lanes()
    }

    /// Every lane's counts added up.
    pub fn lane_counts(&self) -> mpfree_track::LaneCounts {
        let mut all = mpfree_track::LaneCounts::default();
        for k in 0..self.host.lanes() {
            if let Some(c) = self.host.lane_counts(k) {
                all.periods += c.periods;
                all.late += c.late;
                all.waits += c.waits;
            }
        }
        all
    }

    /// Copies moved between cores since the lanes started.
    pub fn lane_moves(&self) -> u64 {
        self.host.moves()
    }

    /// The copies take the Synth tab's values for their engines, every
    /// [`FOLLOW_EVERY`] calls (once a period): each value that differs is
    /// sent. Never allocates.
    pub fn follow(&mut self, synth: &Synth) {
        self.ticks = self.ticks.wrapping_add(1);
        if !self.ticks.is_multiple_of(FOLLOW_EVERY) {
            return;
        }
        for s in 0..HOST_SLOTS {
            let Some(h) = self.held[s] else {
                continue;
            };
            for p in self.host.params(s) {
                if p.readonly {
                    continue;
                }
                let want = synth.value_of(h.engine, p.id);
                let has = self.host.param_value(s, p.id);
                if has.is_some_and(|v| v != want) && want.is_finite() {
                    self.host.param(s, 0, p.id, want);
                    self.woke[s] = true;
                    self.followed += 1;
                }
            }
        }
    }

    /// Every copy plays in `tuning` (the Synth tab's).
    pub fn set_tuning(&mut self, tuning: &Tuning) {
        for s in 0..HOST_SLOTS {
            self.host.set_tuning(s, tuning);
        }
    }

    // ------------------------------------------------------------ sound

    /// Adds each copy's next `frames` frames into its pad's strip in
    /// `mixer` (`dest`, by slot: where that pad's sound goes now). A copy
    /// with nothing sounding and nothing to play rests. Never allocates.
    pub fn render(
        &mut self,
        ctx: &Ctx,
        frames: usize,
        mixer: &mut Mixer,
        dest: &[Option<Dest>; HOST_SLOTS],
    ) {
        let mut awake = [false; HOST_SLOTS];
        for s in 0..HOST_SLOTS {
            self.host.route(s, dest[s].map(|d| d.code(s)));
            awake[s] = self.woke[s]
                || self.host.ringing(s)
                || self
                    .host
                    .instrument(s)
                    .is_some_and(|i| i.node().sounding_voices() > 0);
        }
        self.woke = [false; HOST_SLOTS];
        let mut outs = Outs {
            mixer,
            awake: &awake,
        };
        self.host.render(ctx, frames, &mut outs);
    }

    /// Whether any copy sounds or has something to play.
    pub fn busy(&self) -> bool {
        (0..HOST_SLOTS).any(|s| {
            self.woke[s]
                || self.host.ringing(s)
                || self
                    .host
                    .instrument(s)
                    .is_some_and(|i| i.node().sounding_voices() > 0)
        })
    }
}

/// The mixer as the copies' outputs: a route's code names its slot (for
/// whether it is awake) and its strip.
struct Outs<'a> {
    mixer: &'a mut Mixer,
    awake: &'a [bool; HOST_SLOTS],
}

impl Outputs for Outs<'_> {
    fn track(&mut self, code: usize) -> Option<(&mut [f32], &mut [f32])> {
        let i = code & INDEX_MASK;
        if code & PAD_FLAG != 0 {
            self.mixer.pad_input(i)
        } else {
            self.mixer.track_input(i)
        }
    }

    fn wants(&self, code: usize) -> bool {
        self.awake.get(code >> SLOT_SHIFT).copied().unwrap_or(false)
    }
}

/// Whether `pads` (kit, pad and engine) has `p`'s pad as a synth-voice pad
/// of its engine.
fn wanted(mut pads: impl Iterator<Item = (usize, usize, Engine)>, p: PadFor) -> bool {
    pads.any(|x| x == (usize::from(p.kit), usize::from(p.pad), p.engine))
}

/// The key of track `track`'s hit on pad `pad` in [`PadSynths::voices`].
fn voice_key(track: u16, pad: usize) -> Option<usize> {
    let t = usize::from(track);
    (t < TRACKS && pad < PADS).then_some(t * PADS + pad)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 44_100;
    const BLOCK: usize = 128;

    /// A copy of `engine` as `synth` has it, made as the builder would.
    fn copy_of(synth: &Synth, engine: Engine) -> Instrument {
        let (id, extras, values) = synth.sound_of(engine);
        crate::instruments::make(
            &crate::plugins::Plugins::none(),
            id,
            &extras,
            &values,
            RATE,
            BLOCK,
        )
        .expect("the engine's copy")
        .0
    }

    /// Asks for and puts in every copy `pads` wants.
    fn fill(ps: &mut PadSynths, synth: &Synth, pads: &[(usize, usize, Engine)]) -> Plan {
        let plan = ps.plan(pads.iter().copied(), synth);
        for p in plan.asks.iter().flatten() {
            assert!(ps
                .put(*p, copy_of(synth, p.engine), &Tuning::standard())
                .is_ok());
        }
        plan
    }

    fn mixer() -> Mixer {
        let cfg = mpfree_fx::MixerConfig {
            inputs: 1,
            pads: crate::mixing::PAD_STRIPS,
            tracks: 8,
            slots: 4,
            max_latency_ms: 50.0,
        };
        Mixer::new(cfg, RATE, BLOCK)
    }

    fn peak(b: Option<(&mut [f32], &mut [f32])>) -> f32 {
        b.map_or(0.0, |(l, r)| {
            l.iter().chain(r.iter()).fold(0.0f32, |m, x| m.max(x.abs()))
        })
    }

    /// Each synth-voice pad is asked a copy of its engine once, up to the
    /// sixteen; the seventeenth plays through the Synth channel, told of
    /// once; a pad that stops being one gives its copy up, and the next
    /// one past the sixteen gets it at the next look.
    #[test]
    fn sixteen_pads_have_their_own_synth_and_the_rest_are_told_of_once() {
        let synth = Synth::new(RATE);
        let mut ps = PadSynths::new(RATE, BLOCK);
        let mut pads: Vec<_> = (0..17).map(|p| (0, p, Engine::Analog)).collect();
        let plan = fill(&mut ps, &synth, &pads);
        assert_eq!(plan.asks.iter().flatten().count(), SLOTS);
        assert_eq!((plan.over, plan.newly_over), (1, Some((0, 16))));
        assert_eq!(ps.count(), SLOTS);
        assert!(ps.has(0, 15, Engine::Analog) && !ps.has(0, 16, Engine::Analog));
        // Asked nothing more, and told of once.
        assert!(plan.changed, "the Pads tab is told");
        assert_eq!(ps.on_synth()[0], 1 << 16);
        let again = ps.plan(pads.iter().copied(), &synth);
        assert_eq!(again.asks, [None; SLOTS]);
        assert_eq!((again.over, again.newly_over), (1, None));
        assert!(!again.changed);
        // Pad 1 plays a sample now: its copy goes, and pad 17 gets one.
        pads.remove(0);
        let plan = fill(&mut ps, &synth, &pads);
        let asked: Vec<_> = plan.asks.iter().flatten().map(|p| (p.kit, p.pad)).collect();
        assert_eq!(asked, [(0, 16)]);
        assert_eq!(plan.over, 0);
        assert!(!ps.has(0, 0, Engine::Analog) && ps.has(0, 16, Engine::Analog));
        assert_eq!(ps.on_synth()[0], 0);
        // A pad turned to another engine is asked for again.
        pads[0].2 = Engine::Poly;
        let plan = ps.plan(pads.iter().copied(), &synth);
        assert_eq!(
            plan.asks[0].map(|p| (p.pad, p.engine)),
            Some((1, Engine::Poly))
        );
    }

    /// A copy the builder couldn't make: its pad plays through the Synth
    /// channel (said), and isn't asked for again until its engine's design
    /// changes.
    #[test]
    fn a_copy_that_couldnt_be_made_is_not_asked_for_again() {
        let mut synth = Synth::new(RATE);
        let mut ps = PadSynths::new(RATE, BLOCK);
        let pads = [(0, 5, Engine::Fm)];
        let plan = ps.plan(pads.iter().copied(), &synth);
        let p = plan.asks[0].unwrap();
        assert!(!plan.changed);
        ps.failed(p);
        let plan = ps.plan(pads.iter().copied(), &synth);
        assert_eq!(plan.asks, [None; SLOTS]);
        assert!(plan.changed);
        assert_eq!(ps.on_synth()[0], 1 << 5);
        assert_eq!(ps.plan(pads.iter().copied(), &synth).asks, [None; SLOTS]);
        // A new design: tried again.
        synth.redesigned(Engine::Fm);
        let plan = ps.plan(pads.iter().copied(), &synth);
        assert_eq!(plan.asks[0].map(|p| p.pad), Some(5));
        assert_eq!(ps.on_synth()[0], 0);
    }

    /// A hit on a pad with its own copy plays it into the pad's strip, and
    /// nothing else; its let-go ends it there. A pad without one is the
    /// shared synth's (false, counted).
    #[test]
    fn a_hit_plays_the_pads_own_copy_into_its_strip() {
        let synth = Synth::new(RATE);
        let mut ps = PadSynths::new(RATE, BLOCK);
        fill(&mut ps, &synth, &[(0, 3, Engine::Poly)]);
        assert!(!ps.note_on(0, 0, 4, Engine::Poly, 60, 100, 0));
        assert_eq!(ps.fell_back(), 1);
        assert!(ps.note_on(0, 0, 3, Engine::Poly, 60, 100, 0));
        let mut m = mixer();
        let mut dest = [None; HOST_SLOTS];
        let slot = (0..HOST_SLOTS)
            .find(|&s| ps.pad_of(s) == Some((0, 3)))
            .unwrap();
        dest[slot] = Some(Dest::Pad(3));
        let ctx = Ctx::new(RATE, 0);
        let mut heard = 0.0f32;
        for _ in 0..8 {
            ps.render(&ctx, BLOCK, &mut m, &dest);
            heard = heard.max(peak(m.pad_input(3)));
            assert_eq!(peak(m.pad_input(2)), 0.0);
            assert_eq!(peak(m.track_input(0)), 0.0);
            for p in [2, 3] {
                if let Some((l, r)) = m.pad_input(p) {
                    l.fill(0.0);
                    r.fill(0.0);
                }
            }
        }
        assert!(heard > 0.0, "pad 4's strip heard its copy");
        assert!(ps.busy());
        assert!(ps.note_off(0, 3, 0));
        assert!(!ps.note_off(0, 3, 0), "ended already");
    }

    /// A copy follows the Synth tab's values for its engine a few times a
    /// second.
    #[test]
    fn a_copy_follows_the_synth_tabs_values() {
        let mut synth = Synth::new(RATE);
        let mut ps = PadSynths::new(RATE, BLOCK);
        fill(&mut ps, &synth, &[(0, 0, Engine::Poly)]);
        let p = synth
            .param_info_list()
            .into_iter()
            .find(|p| !p.readonly && p.max > p.min)
            .unwrap();
        let to = if synth.value(p.id) == p.max {
            p.min
        } else {
            p.max
        };
        synth.set(p.id, to);
        for _ in 0..FOLLOW_EVERY {
            ps.follow(&synth);
        }
        assert_eq!(ps.followed(), 1);
        let mut m = mixer();
        ps.render(&Ctx::new(RATE, 0), BLOCK, &mut m, &[None; HOST_SLOTS]);
        let slot = (0..HOST_SLOTS).find(|&s| ps.pad_of(s).is_some()).unwrap();
        assert_eq!(ps.host.param_value(slot, p.id), Some(synth.value(p.id)));
        for _ in 0..FOLLOW_EVERY {
            ps.follow(&synth);
        }
        assert_eq!(ps.followed(), 1, "nothing more to send");
    }
}
