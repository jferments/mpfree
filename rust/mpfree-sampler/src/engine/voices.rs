//! The voice pool: each voice's state, the pool's two halves (one may play on
//! a second core), stealing, load shedding, the hard stop, and what the
//! screen asks of the voices playing.

use super::*;

/// Fade length when a voice is choked, muted by its group, or pushed out by
/// the pad's polyphony limit.
pub const KILL_FRAMES: u16 = 64;

/// Fade length of the previous sound when its voice is stolen for a new hit.
pub const STEAL_FRAMES: u16 = 32;

/// What a playing voice follows of the live settings: where its gain,
/// filter and tune are going, the steps it takes there, and the pad's own
/// settings from when it started (a voice finishes as it started, whatever
/// view comes in).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Follow {
    /// Still gliding to a target.
    pub(super) moving: bool,
    /// Frames into the current step of [`FILTER_STEP`] frames.
    pub(super) count: u32,
    /// The gain (left, right, before the tape's fade) it is going to, and
    /// the step a frame.
    pub(super) gain_to: [f32; 2],
    pub(super) gain_inc: [f32; 2],
    /// The cutoff (Hz) and resonance its filter is going to.
    pub(super) fc_to: f32,
    pub(super) res_to: f32,
    /// The program's tune in cents: in use, and where it is going.
    pub(super) cents: f32,
    pub(super) cents_to: f32,
    /// The one-pole step per [`FILTER_STEP`] frames.
    pub(super) k: f32,
    /// The pad's own level, pan, cutoff and resonance when it started (the
    /// cutoff of 16 Levels if it set one; an open filter's if none).
    pub(super) own_db: f32,
    pub(super) own_pan: f32,
    pub(super) own_fc: f32,
    pub(super) own_res: f32,
}

/// A voice's movement (sampler R5 part 4): its pad's LFO, its pitch
/// envelope and what its hit's velocity did to its pitch and cutoff.
/// Worked out every [`FILTER_STEP`] frames ([`motion_step`]), as pressure
/// is; the level and pan glide across each step.
#[derive(Clone, Copy, Debug)]
pub(super) struct Motion {
    /// The LFO or the pitch envelope still moves something: [`finish`]
    /// steps it, and the voice is never plain ([`plain_now`]).
    pub(super) on: bool,
    /// Frames into the current step.
    pub(super) count: u32,
    pub(super) shape: LfoShape,
    pub(super) to: LfoTarget,
    /// The depth in use (a live one, else the pad's own) and the pad's own.
    pub(super) depth: f32,
    pub(super) own_depth: f32,
    /// Where it is in its cycle (0 to 1), and how far a step moves it.
    pub(super) phase: f32,
    pub(super) inc: f32,
    /// A synced LFO's cycle in beats (0 when it runs free at `hz`), and
    /// output frames a step, so a tempo change or an edit can set `inc`.
    pub(super) beats: f32,
    pub(super) hz: f32,
    pub(super) step_s: f32,
    /// The random waves' values: the one it comes from and the one it
    /// holds or goes to.
    pub(super) held: [f32; 2],
    pub(super) rng: Rng,
    /// The pitch envelope now, in cents, and what a step leaves of it.
    pub(super) env_cents: f32,
    pub(super) env_k: f32,
    /// The hit's velocity on its pitch (cents) and on its cutoff (a factor).
    pub(super) vel_cents: f32,
    pub(super) vel_fc: f32,
    /// A mono pad's glide (sampler R5.5): cents still to slide, and how
    /// many a step slides (a straight line in cents, so every interval
    /// takes the pad's glide time).
    pub(super) glide_cents: f32,
    pub(super) glide_inc: f32,
    /// What it all comes to: cents on the pitch, a factor on the cutoff,
    /// and a gain on each side, gliding to the next over the step.
    pub(super) cents: f32,
    pub(super) fc: f32,
    pub(super) side: [f32; 2],
    pub(super) side_inc: [f32; 2],
    /// Its pitch may move: not a warped layer, not a grain voice.
    pub(super) pitched: bool,
}

impl Default for Motion {
    fn default() -> Motion {
        Motion {
            on: false,
            count: 0,
            shape: LfoShape::Sine,
            to: LfoTarget::Off,
            depth: 0.0,
            own_depth: 0.0,
            phase: 0.0,
            inc: 0.0,
            beats: 0.0,
            hz: 0.0,
            step_s: 0.0,
            held: [0.0; 2],
            rng: Rng::new(1),
            env_cents: 0.0,
            env_k: 0.0,
            vel_cents: 0.0,
            vel_fc: 1.0,
            glide_cents: 0.0,
            glide_inc: 0.0,
            cents: 0.0,
            fc: 1.0,
            side: [1.0; 2],
            side_inc: [0.0; 2],
            pitched: false,
        }
    }
}

/// The pitch envelope's fall over its decay: e^-9.21 is 1e-4, as the
/// grain envelope (24 semitones come back to under a cent).
pub(super) const ENV_FALL: f32 = 9.21;

impl Motion {
    /// Slides the pitch in from `cents` over `ms` at output rate `rate`
    /// (sampler R5.5); 0 ms jumps.
    pub(super) fn glide(&mut self, cents: f32, ms: f32, rate: f32) {
        let steps = (ms * 0.001 * rate / FILTER_STEP as f32).round();
        if steps < 1.0 || !self.pitched {
            self.glide_cents = 0.0;
            self.glide_inc = 0.0;
            return;
        }
        self.glide_cents = cents;
        self.glide_inc = cents.abs() / steps;
        self.on = true;
    }

    /// Takes `lfo`'s wave, target and rate (and `depth` in place of its
    /// own), keeping where it is in its cycle: a new voice, or an edit of
    /// the pad while it plays. `bpm` is the project tempo.
    pub(super) fn set_lfo(&mut self, lfo: &PadLfo, depth: f32, bpm: f64) {
        self.shape = lfo.shape;
        self.to = lfo.target;
        self.own_depth = lfo.depth;
        self.depth = depth;
        self.beats = lfo.beats.unwrap_or(0.0);
        self.hz = lfo.rate_hz;
        self.retempo(bpm);
        if self.to != LfoTarget::Off {
            self.on = true;
        }
    }

    /// A synced LFO keeps its cycle in beats at tempo `bpm`.
    pub(super) fn retempo(&mut self, bpm: f64) {
        let hz = if self.beats > 0.0 {
            (bpm / 60.0) as f32 / self.beats
        } else {
            self.hz
        };
        self.inc = hz * self.step_s;
    }

    /// The cycles a second it runs at, at tempo `bpm`.
    pub(super) fn cycle_hz(&self, bpm: f64) -> f64 {
        if self.beats > 0.0 {
            bpm / 60.0 / f64::from(self.beats)
        } else {
            f64::from(self.hz)
        }
    }
}

/// What a voice's frame was read with, as far as the crossfade between
/// reads goes ([`Voice::last_read`]). A Vintage pad's Hermite read never
/// changes, so it has none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Heard {
    /// The frame as it is: its sound's own speed on a whole frame.
    #[default]
    Exact,
    /// The 8-point read ([`bandlimit::near_kernel`]).
    Near,
    /// The band-limited read ([`bandlimit::read2`]).
    Band,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Voice {
    pub(super) active: bool,
    /// The kit whose pad started it ([`Engine::with_kits`]).
    pub(super) kit: u8,
    pub(super) pad: u8,
    pub(super) layer: u8,
    pub(super) data: Option<Pcm>,
    pub(super) stereo: bool,
    pub(super) pick_l: u8,
    pub(super) pick_r: u8,
    pub(super) pos: i64,
    pub(super) step: i64,
    pub(super) backwards: bool,
    pub(super) reverse: bool,
    pub(super) start: i64,
    pub(super) end: i64,
    pub(super) loop_start: i64,
    pub(super) loop_end: i64,
    pub(super) crossfade: i64,
    pub(super) ping_pong: bool,
    pub(super) looping: bool,
    /// It has gone round its loop, or turned at an end of a ping-pong
    /// loop, since it started: from then on the frames before the loop's
    /// start that its read looks at are the ones the loop plays there
    /// ([`Voice::fold`]), not the recording's.
    pub(super) wrapped: bool,
    /// Frames `fast_lo..fast_hi` can be read straight, with nothing to
    /// clamp or fold ([`Voice::refresh_fast`], at the start of each render
    /// run). Left at 0 and 0, every read takes the careful path.
    pub(super) fast_lo: i64,
    pub(super) fast_hi: i64,
    pub(super) fade_in: Option<Fade>,
    pub(super) fade_out: Option<Fade>,
    pub(super) gain_l: f32,
    pub(super) gain_r: f32,
    pub(super) mode: Option<PlayMode>,
    pub(super) held: bool,
    pub(super) age: u64,
    pub(super) amp: Option<Envelope>,
    pub(super) fenv: Option<Envelope>,
    pub(super) filter: Option<Filter>,
    pub(super) svf: [Svf; 2],
    /// The ladder's state, for a Ladder filter, and a Vowel's gain after
    /// its formants (sampler R5.6).
    pub(super) ladder: Ladder,
    pub(super) vowel_gain: f32,
    /// The Comb filter's line (sampler R5.6), made with the pool and
    /// kept from note to note ([`Comb::reset`]), so nothing allocates.
    pub(super) comb: Comb,
    /// Its sound has ended (or its envelope), and its Comb rings on
    /// ([`ring_out`]).
    pub(super) ringing: bool,
    pub(super) filter_count: u32,
    /// The envelope level and pressure factor (as bits) the filter was
    /// last set for.
    pub(super) filter_key: Option<(u32, u32)>,
    pub(super) kill_left: u16,
    pub(super) tail_left: u16,
    pub(super) tail_l: f32,
    pub(super) tail_r: f32,
    pub(super) last_l: f32,
    pub(super) last_r: f32,
    /// About how loud it is now, for load shedding: the louder of its last
    /// two output samples at the end of each render, held and falling a
    /// little each render (so a peak, not a zero crossing). Starts at its
    /// gain.
    pub(super) heard: f32,
    /// Plays through the engine's grain voice in the same slot.
    pub(super) granular: bool,
    pub(super) src_rate: u32,
    /// Grain settings before pressure moves them.
    pub(super) grain_base: GrainParams,
    /// What a key moves ([`ViewLayer::follow`]), and the semitones its hit
    /// added (a keygroup's, a live tune's), kept for a live edit and a
    /// glide; of those, the hit's key (a keygroup's, 16 Levels').
    pub(super) grain_follow: KeyFollow,
    pub(super) grain_hit: f32,
    pub(super) grain_key: i32,
    /// Gliding to another key ([`ViewLayer::glide`]): where `grain_hit`
    /// goes, how far it moves a frame, and the frames left.
    pub(super) glide_to: f32,
    pub(super) glide_step: f32,
    pub(super) glide_left: u32,
    pub(super) route: PressureRoute,
    /// Pressure asked for and the smoothed value in use, 0 to 1.
    pub(super) press_target: f32,
    pub(super) press: f32,
    pub(super) press_coef: f32,
    /// What pressure has made of the step, the gain and the cutoff. The
    /// gain glides to each new value over [`FILTER_STEP`] frames.
    pub(super) base_step: i64,
    pub(super) press_gain: f32,
    pub(super) press_gain_inc: f32,
    pub(super) press_fc: f32,
    pub(super) press_count: u32,
    /// The layer's tempo when warped (0 if not), and its step before the
    /// tempo moves it.
    pub(super) warp_bpm: f64,
    pub(super) plain_step: i64,
    /// The note a keygroup voice plays ([`NO_NOTE`] for a pad hit).
    pub(super) note: u8,
    /// A keygroup note that follows its kit's tuning: the cents from its
    /// note's 12-TET pitch its step took in at the start (and since, on a
    /// legato or glide move). None for a pad hit, a slice, the take and a
    /// 16 Levels step, which don't follow a change of tuning.
    pub(super) tuned: Option<f32>,
    /// How far its key sounds from its sound's root, in cents, in its
    /// kit's tuning: a keygroup's semitones, its tuning's offset and a 16
    /// Levels Tune step's interval (semitones times 100 in 12-TET), not
    /// the pad's or program's Tune or a bend. For what follows the key
    /// played, such as a filter's key tracking.
    pub(super) key_cents: f32,
    /// The pad whose hit started it: its own pad, or the pad linking to it.
    pub(super) by: u8,
    /// Its pad latches: lifting a pad or a key lets it play on.
    pub(super) latch: bool,
    /// An audition (a slice, or the region for live chop).
    pub(super) audition: bool,
    /// How fast it plays against its step: the tape's speed with wow and
    /// flutter (1 when neither moves).
    pub(super) speed: f32,
    /// Its gain before the tape fades it.
    pub(super) base_gain_l: f32,
    pub(super) base_gain_r: f32,
    /// Wow and flutter, 0 to 1, and where its three wobbles are (cycles).
    pub(super) wow: f32,
    pub(super) wow_phase: [f32; 3],
    /// Its gain before the pad's level and pan (region, layer and
    /// velocity), its layer's own pan, and the tape's fade on its gain:
    /// live Level and Pan work from these.
    pub(super) unit_gain: f32,
    pub(super) layer_pan: f32,
    pub(super) tape_gain: f32,
    /// The live settings it follows.
    pub(super) follow: Follow,
    /// The program tune (cents) its grains were last set for.
    pub(super) grain_cents: f32,
    /// Started as a plain one-shot ([`Engine::set_full_voices`]): it may
    /// take a voice past the full-voice limit.
    pub(super) light: bool,
    /// The step past which it reads band-limited ([`Voice::bl`]):
    /// [`CLEAN_ABOVE`], or never for a Vintage pad or a granular voice.
    pub(super) bl_above: BlAbove,
    /// Reads band-limited ([`bandlimit::read2`]), from the first frame its
    /// step passed [`Voice::bl_above`] on: once set, for the rest of its
    /// life, so its sound doesn't change as its pitch moves.
    pub(super) bl: bool,
    /// Its sound's octave copies, when made by the time it started.
    pub(super) octaves: Option<Arc<OctaveCell>>,
    /// A granular Stretch's hits, when found by the time it started.
    pub(super) hits: Option<Arc<HitCell>>,
    /// A granular voice's grains read band-limited when pitched up
    /// ([`GrainClip::clean`]): not on a Vintage pad.
    pub(super) grain_clean: bool,
    /// A Vintage pad's voice keeps the Hermite read at every speed below
    /// [`CLEAN_ABOVE`] ([`Voice::read2`]); every other voice reads with
    /// [`bandlimit::near_kernel`] there.
    pub(super) vintage: bool,
    /// The step the band-limited read was last aimed for ([`aim_bl`]),
    /// the octave copy it reads (0: the sound) and the sinc's stretch.
    pub(super) bl_step: i64,
    pub(super) bl_octave: u32,
    pub(super) bl_reach: Reach,
    /// What it follows of an edit ([`Engine::edit_view`]): the hit's own
    /// gain (velocity, 16 Levels) under the layer's, whether its filter's
    /// cutoff is the pad's (not one 16 Levels set), and the layer's step
    /// and grain settings it plays from now.
    pub(super) hit_gain: f32,
    pub(super) own_cut: bool,
    pub(super) layer_step: i64,
    pub(super) layer_grain: GrainParams,
    /// A jump into a moved loop: what the read was minus what it is now,
    /// fading out over [`JUMP_FRAMES`] frames so the sound doesn't step.
    pub(super) jump: [f32; 2],
    pub(super) jump_left: u16,
    /// What its last frame was read with (none before its first), and a
    /// change of read fading in: from `xf_from` over [`XF_FRAMES`] frames,
    /// `xf_left` of them to go. The reads differ near the top (the 8-point
    /// read passes 18 kHz 2.5 dB down on a whole frame, where the plain
    /// copy is exact), so a voice that changes read crossfades rather than
    /// stepping (DSP quality step 1.4, third review).
    pub(super) last_read: Option<Heard>,
    pub(super) xf_from: Heard,
    pub(super) xf_left: u16,
    /// Its pad's LFO and pitch envelope, and its hit's velocity on its
    /// pitch and cutoff (sampler R5 part 4).
    pub(super) motion: Motion,
}

impl Voice {
    /// Whether its reads fold their neighbours round the loop: a forward
    /// loop with no crossfade, played forward, or a ping-pong loop. A
    /// crossfaded loop's blend reads the frames on either side of its ends
    /// as they are, which is what it plays.
    #[inline]
    fn folds(&self) -> bool {
        self.looping
            && self.loop_end > self.loop_start
            && (self.ping_pong || self.crossfade == 0 && !self.backwards)
    }

    /// Frame `k` as the loop unrolled holds it, for a read near one of the
    /// loop's ends ([`Voice::folds`]): past a forward loop's end, the
    /// frames from its start; before its start once it has wrapped, its
    /// last frames. A ping-pong loop mirrors them, about its last frame's
    /// far edge at the end and about its first frame at the start, as the
    /// voice turns. Any other frame is itself; the caller clamps to the
    /// region.
    #[inline]
    pub(super) fn fold(&self, k: i64) -> i64 {
        if !self.folds() {
            return k;
        }
        let (ls, le) = (self.loop_start, self.loop_end);
        let len = le - ls;
        if self.ping_pong {
            // A loop shorter than the read mirrors more than once.
            let mut k = k;
            for _ in 0..8 {
                if k >= le {
                    k = 2 * le - 1 - k;
                } else if k < ls && self.wrapped {
                    k = 2 * ls - k;
                } else {
                    break;
                }
            }
            k
        } else if k >= le {
            ls + (k - le) % len
        } else if k < ls && self.wrapped {
            le - 1 - (ls - 1 - k) % len
        } else {
            k
        }
    }

    /// Sets [`Voice::fast_lo`] and [`Voice::fast_hi`]: the region, less
    /// what lies past the loop's end and, once it has wrapped, before its
    /// start, when its reads fold.
    #[inline]
    pub(super) fn refresh_fast(&mut self) {
        let (mut lo, mut hi) = (self.start, self.end);
        if self.folds() {
            hi = hi.min(self.loop_end);
            if self.wrapped {
                lo = lo.max(self.loop_start);
            }
        }
        self.fast_lo = lo;
        self.fast_hi = hi;
    }

    /// The sound at frame `i` (folded round the loop, [`Voice::fold`], and
    /// clamped to the region), channel `ch` (0 for a mono sound).
    #[inline]
    pub(super) fn at<S: Sample>(&self, data: &[S], i: i64, ch: u8) -> f32 {
        let i = self.fold(i).clamp(self.start, self.end - 1) as usize;
        if self.stereo {
            data[2 * i + usize::from(ch)].f()
        } else {
            data[i].f()
        }
    }

    /// The sound's raw value (see [`Sample::raw`]) at frame `i` folded
    /// round the loop and clamped to the region, as [`Voice::at`].
    #[inline]
    fn at_raw<S: Sample>(&self, data: &[S], i: i64, ch: u8) -> f32 {
        let i = self.fold(i).clamp(self.start, self.end - 1) as usize;
        if self.stereo {
            data[2 * i + usize::from(ch)].raw()
        } else {
            data[i].raw()
        }
    }

    /// One channel at a fixed-point position, with every neighbour clamped
    /// to the region: what [`Voice::read2`] gives for `pick`, bit for bit
    /// (the tests' reference; the reads' own edge cases are
    /// [`Voice::read_near`] and [`Voice::read_hermite`]).
    #[cfg(test)]
    pub(super) fn read<S: Sample>(&self, data: &[S], pos: i64, pick: u8) -> f32 {
        if self.vintage {
            self.read_hermite(data, pos, pick)
        } else {
            self.read_near(data, pos, pick, self.on_frame(pos))
        }
    }

    /// Whether the voice reads its sound at its own speed with `pos` on a
    /// whole frame, so the frame is read as it is. It holds for every
    /// frame of a run at one step: a step of exactly one frame keeps the
    /// fraction, through loops and ping-pong turns alike.
    #[inline(always)]
    pub(super) fn on_frame(&self, pos: i64) -> bool {
        pos as u32 == 0 && self.step == ONE
    }

    /// [`Voice::read`] through [`bandlimit::near_kernel`], neighbours
    /// folded round the loop and clamped to the region. `exact` ([`Voice::on_frame`]) takes the frame
    /// as it is; otherwise the kernel reads every position, a whole frame
    /// too (the old side of a crossfade).
    #[cold]
    #[inline(never)]
    fn read_near<S: Sample>(&self, data: &[S], pos: i64, pick: u8, exact: bool) -> f32 {
        let i = pos >> 32;
        let f = pos as u32;
        if exact {
            return pick_of(self.at(data, i, 0), self.at(data, i, 1), pick);
        }
        let c = bandlimit::near_kernel().weights(f);
        let k0 = i - (NEAR_HALF as i64 - 1);
        let (l, r) = if self.stereo {
            let mut x = [0.0f32; 2 * NEAR_TAPS];
            for (j, w) in x.chunks_exact_mut(2).enumerate() {
                w[0] = self.at_raw(data, k0 + j as i64, 0);
                w[1] = self.at_raw(data, k0 + j as i64, 1);
            }
            bandlimit::near_sum2(&c, &x)
        } else {
            let mut x = [0.0f32; NEAR_TAPS];
            for (j, w) in x.iter_mut().enumerate() {
                *w = self.at_raw(data, k0 + j as i64, 0);
            }
            let m = bandlimit::near_sum(&c, &x);
            (m, m)
        };
        pick_of(l * S::SCALE, r * S::SCALE, pick)
    }

    /// Both output channels at a fixed-point position: what
    /// `Voice::read` gives for `pick_l` and `pick_r`, bit for bit: a
    /// Vintage pad's Hermite read ([`Voice::read2_hermite`]), any other
    /// voice's 8-point read ([`Voice::read2_near`]). The render loops call
    /// those directly, chosen once a block ([`Read`]).
    #[inline(always)]
    pub(super) fn read2<S: Sample>(&self, data: &[S], pos: i64) -> (f32, f32) {
        if self.vintage {
            self.read2_hermite(data, pos)
        } else {
            self.read2_near(bandlimit::near_kernel(), data, pos, self.on_frame(pos))
        }
    }

    /// Both channels through [`bandlimit::near_kernel`]'s 8 frames. With
    /// `exact` ([`Voice::on_frame`], found once a run by the render loop),
    /// the frame as it is. Away from the region's edges and the loop's ends
    /// ([`Voice::fast_lo`], nearly always) the
    /// frames are read straight from the data, once for both channels,
    /// with no clamps.
    #[inline(always)]
    pub(super) fn read2_near<S: Sample>(
        &self,
        near: &bandlimit::NearKernel,
        data: &[S],
        pos: i64,
        exact: bool,
    ) -> (f32, f32) {
        let i = pos >> 32;
        let f = pos as u32;
        let k0 = i - (NEAR_HALF as i64 - 1);
        if k0 < self.fast_lo || i + NEAR_HALF as i64 >= self.fast_hi {
            return (
                self.read_near(data, pos, self.pick_l, exact),
                self.read_near(data, pos, self.pick_r, exact),
            );
        }
        let k0 = k0 as usize;
        if !self.stereo {
            // Mono plays the same on both sides.
            if exact {
                let x = data[i as usize].f();
                return (x, x);
            }
            let c = near.weights(f);
            let w: &[S; NEAR_TAPS] = data[k0..k0 + NEAR_TAPS].try_into().expect("taps");
            let x = bandlimit::near_sum(&c, &w.map(S::raw)) * S::SCALE;
            return (x, x);
        }
        let (l, r) = if exact {
            let i = i as usize;
            (data[2 * i].f(), data[2 * i + 1].f())
        } else {
            let c = near.weights(f);
            let w: &[S; 2 * NEAR_TAPS] = data[2 * k0..2 * k0 + 2 * NEAR_TAPS]
                .try_into()
                .expect("taps");
            let (l, r) = bandlimit::near_sum2(&c, &w.map(S::raw));
            (l * S::SCALE, r * S::SCALE)
        };
        (pick_of(l, r, self.pick_l), pick_of(l, r, self.pick_r))
    }

    /// [`Voice::read`] on a Vintage pad: Hermite-interpolated, which equals
    /// the plain copy on a whole frame.
    fn read_hermite<S: Sample>(&self, data: &[S], pos: i64, pick: u8) -> f32 {
        let i = pos >> 32;
        let t = frac(pos);
        let ch = |k: i64| -> f32 {
            match pick {
                0 => self.at(data, k, 0),
                1 => self.at(data, k, 1),
                _ => 0.5 * (self.at(data, k, 0) + self.at(data, k, 1)),
            }
        };
        if t == 0.0 {
            return ch(i);
        }
        hermite(ch(i - 1), ch(i), ch(i + 1), ch(i + 2), t)
    }

    /// [`Voice::read2`] on a Vintage pad: Hermite-interpolated, the four
    /// neighbours read straight from the data away from the region's edges.
    #[inline(always)]
    pub(super) fn read2_hermite<S: Sample>(&self, data: &[S], pos: i64) -> (f32, f32) {
        let i = pos >> 32;
        if i - 1 < self.fast_lo || i + 2 >= self.fast_hi {
            return (
                self.read_hermite(data, pos, self.pick_l),
                self.read_hermite(data, pos, self.pick_r),
            );
        }
        let t = frac(pos);
        let k = (i - 1) as usize;
        if !self.stereo {
            // Mono plays the same on both sides.
            let w: &[S; 4] = data[k..k + 4].try_into().expect("four frames");
            let w = w.map(S::f);
            let x = if t == 0.0 {
                w[1]
            } else {
                hermite(w[0], w[1], w[2], w[3], t)
            };
            return (x, x);
        }
        let w: &[S; 8] = data[2 * k..2 * k + 8].try_into().expect("four frames");
        let w = w.map(S::f);
        let side = |pick: u8| -> [f32; 4] {
            match pick {
                0 => [w[0], w[2], w[4], w[6]],
                1 => [w[1], w[3], w[5], w[7]],
                _ => [
                    0.5 * (w[0] + w[1]),
                    0.5 * (w[2] + w[3]),
                    0.5 * (w[4] + w[5]),
                    0.5 * (w[6] + w[7]),
                ],
            }
        };
        let (a, b) = (side(self.pick_l), side(self.pick_r));
        if t == 0.0 {
            return (a[1], b[1]);
        }
        (
            hermite(a[0], a[1], a[2], a[3], t),
            hermite(b[0], b[1], b[2], b[3], t),
        )
    }
}

/// An output channel from a read's left and right: the left (0), the
/// right (1) or their average.
#[inline(always)]
fn pick_of(l: f32, r: f32, pick: u8) -> f32 {
    match pick {
        0 => l,
        1 => r,
        _ => 0.5 * (l + r),
    }
}

/// The fraction of a 32.32 fixed-point position, 0 to 1. Converted from
/// its low 32 bits as a `u32`, which ARMv7 does in one instruction (a
/// 64-bit integer needs a library call); exact either way.
#[inline(always)]
pub(super) fn frac(pos: i64) -> f32 {
    (pos as u32) as f32 * (1.0 / ONE as f32)
}

pub(super) fn picks(stereo: bool, pick: ChannelPick) -> (u8, u8) {
    if !stereo {
        return (0, 0);
    }
    match pick {
        ChannelPick::Stereo => (0, 1),
        ChannelPick::MonoSum => (2, 2),
        ChannelPick::Left => (0, 0),
        ChannelPick::Right => (1, 1),
    }
}

/// A share of the voice pool, with their grain voices, grain budget and
/// buffers. The engine has one; split ([`Engine::split`]), a second one
/// renders on a lane.
pub(super) struct Pool {
    pub(super) voices: Vec<Voice>,
    /// One grain voice per voice slot, used while that slot is granular.
    pub(super) grains: Vec<GrainVoice>,
    pub(super) budget: Budget,
    pub(super) scratch_l: Vec<f32>,
    pub(super) scratch_r: Vec<f32>,
    /// Where the voices of a pad with no output play, so they end on time.
    pub(super) unheard_l: Vec<f32>,
    pub(super) unheard_r: Vec<f32>,
    /// The lane's share only: each voice's sound for the part of the block
    /// it renders ([`LANE_BLOCK`] frames a voice), and the voices that
    /// played in it.
    pub(super) out_l: Vec<f32>,
    pub(super) out_r: Vec<f32>,
    pub(super) played: Vec<bool>,
}

/// What a playing voice costs to render, in grains (see [`Pool::cost`]).
/// A rough figure from the workstation bench, where a sample voice (read,
/// envelope, filter) costs about as much as 8 to 16 live grains; the
/// device bench's `--afford` says what it is on the MPC (R7 step A6).
pub const VOICE_COST: usize = 8;

/// What a plain one-shot costs in the same units: the drum kit bench
/// cases put a plain voice at about a fifth of a full one.
pub(super) const LIGHT_COST: usize = 2;

impl Pool {
    /// About what this share costs to render now: its playing voices and
    /// their live grains. A new voice goes to the share that costs less, so
    /// two granular clouds don't pile up on one core.
    pub(super) fn cost(&self) -> usize {
        let cost = |v: &Voice| if v.light { LIGHT_COST } else { VOICE_COST };
        self.voices
            .iter()
            .filter(|v| v.active)
            .map(cost)
            .sum::<usize>()
            + self.budget.live()
    }

    /// `n` voices, the first of them slot `first` of the whole pool (it
    /// seeds their grain voices). `lane`: with the buffers the lane's share
    /// renders into.
    pub(super) fn new(rate: u32, first: usize, n: usize, lane: bool) -> Pool {
        let out = if lane { n * LANE_BLOCK } else { 0 };
        let mut voices = vec![Voice::default(); n];
        for v in &mut voices {
            v.comb = Comb::new(rate as f32);
        }
        Pool {
            voices,
            grains: (first..first + n)
                .map(|i| GrainVoice::new(rate, i as u64))
                .collect(),
            budget: Budget::new(GRAIN_BUDGET),
            scratch_l: vec![0.0; GRAIN_CHUNK],
            scratch_r: vec![0.0; GRAIN_CHUNK],
            unheard_l: vec![0.0; GRAIN_CHUNK],
            unheard_r: vec![0.0; GRAIN_CHUNK],
            out_l: vec![0.0; out],
            out_r: vec![0.0; out],
            played: vec![false; if lane { n } else { 0 }],
        }
    }

    /// Frames `a..b` of the block (`frame` its first) of every voice, each
    /// into its pad's pair from `out`, or played unheard.
    pub(super) fn render_into(
        &mut self,
        a: usize,
        b: usize,
        frame: u64,
        rate: f32,
        out: &mut impl PadOut,
    ) {
        let Pool {
            voices,
            grains,
            budget,
            scratch_l,
            scratch_r,
            unheard_l,
            unheard_r,
            ..
        } = self;
        for (v, g) in voices.iter_mut().zip(grains.iter_mut()) {
            if !v.active {
                continue;
            }
            match out.pad(usize::from(v.kit), usize::from(v.pad)) {
                Some((l, r)) => play(
                    v,
                    g,
                    budget,
                    frame,
                    rate,
                    &mut l[a..b],
                    &mut r[a..b],
                    scratch_l,
                    scratch_r,
                ),
                None => {
                    let mut k = a;
                    while k < b && v.active {
                        let m = (b - k).min(unheard_l.len());
                        let (ul, ur) = (&mut unheard_l[..m], &mut unheard_r[..m]);
                        ul.fill(0.0);
                        ur.fill(0.0);
                        play(
                            v,
                            g,
                            budget,
                            frame + (k - a) as u64,
                            rate,
                            ul,
                            ur,
                            scratch_l,
                            scratch_r,
                        );
                        k += m;
                    }
                }
            }
        }
    }

    /// The lane's share: `frames` frames from `frame` of every voice, each
    /// into its own pair in `out_l` and `out_r`.
    pub(super) fn render_own(&mut self, frames: usize, frame: u64, rate: f32) {
        let Pool {
            voices,
            grains,
            budget,
            scratch_l,
            scratch_r,
            out_l,
            out_r,
            played,
            ..
        } = self;
        for (k, (v, g)) in voices.iter_mut().zip(grains.iter_mut()).enumerate() {
            played[k] = v.active;
            if !v.active {
                continue;
            }
            let at = k * LANE_BLOCK;
            let (l, r) = (&mut out_l[at..at + frames], &mut out_r[at..at + frames]);
            l.fill(0.0);
            r.fill(0.0);
            play(v, g, budget, frame, rate, l, r, scratch_l, scratch_r);
        }
    }

    /// Adds what [`Pool::render_own`] rendered into frames `a..a + frames`
    /// of each voice's pad pair, in slot order.
    pub(super) fn add_own(&self, a: usize, frames: usize, out: &mut impl PadOut) {
        for (k, v) in self.voices.iter().enumerate() {
            if !self.played[k] {
                continue;
            }
            if let Some((l, r)) = out.pad(usize::from(v.kit), usize::from(v.pad)) {
                let at = k * LANE_BLOCK;
                for (o, x) in l[a..a + frames]
                    .iter_mut()
                    .zip(&self.out_l[at..at + frames])
                {
                    *o += *x;
                }
                for (o, x) in r[a..a + frames]
                    .iter_mut()
                    .zip(&self.out_r[at..at + frames])
                {
                    *o += *x;
                }
            }
        }
    }
}

/// The whole voice pool: slots `0..a.voices.len()` in `a`, the rest in `b`
/// once split. `b` is away (None) while the lane renders it.
pub(super) struct Voices {
    pub(super) a: Pool,
    pub(super) b: Option<Box<Pool>>,
    /// The lane share's grain limit when it left.
    pub(super) b_limit: usize,
    /// The lane share's skipped grains when it left, so the count does not
    /// go back while it is away ([`Engine::stats`]).
    pub(super) b_skipped: u64,
    pub(super) split: bool,
}

impl Voices {
    /// Every voice here with its grain voice, in slot order.
    pub(super) fn iter(&self) -> impl Iterator<Item = (&Voice, &GrainVoice)> + '_ {
        let b = self.b.iter().flat_map(|p| p.voices.iter().zip(&p.grains));
        self.a.voices.iter().zip(&self.a.grains).chain(b)
    }

    pub(super) fn iter_mut(&mut self) -> impl Iterator<Item = (&mut Voice, &mut GrainVoice)> + '_ {
        let b = self
            .b
            .iter_mut()
            .flat_map(|p| p.voices.iter_mut().zip(p.grains.iter_mut()));
        self.a
            .voices
            .iter_mut()
            .zip(self.a.grains.iter_mut())
            .chain(b)
    }

    pub(super) fn voice(&self, slot: usize) -> Option<&Voice> {
        let n = self.a.voices.len();
        match slot.checked_sub(n) {
            None => self.a.voices.get(slot),
            Some(k) => self.b.as_ref()?.voices.get(k),
        }
    }

    pub(super) fn voice_mut(&mut self, slot: usize) -> Option<&mut Voice> {
        let n = self.a.voices.len();
        match slot.checked_sub(n) {
            None => self.a.voices.get_mut(slot),
            Some(k) => self.b.as_mut()?.voices.get_mut(k),
        }
    }

    pub(super) fn grain_mut(&mut self, slot: usize) -> Option<&mut GrainVoice> {
        let n = self.a.voices.len();
        match slot.checked_sub(n) {
            None => self.a.grains.get_mut(slot),
            Some(k) => self.b.as_mut()?.grains.get_mut(k),
        }
    }

    /// Silences slot `slot`'s grains, giving them back to its budget.
    pub(super) fn kill_grain(&mut self, slot: usize) {
        let n = self.a.voices.len();
        match slot.checked_sub(n) {
            None => {
                if let Some(g) = self.a.grains.get_mut(slot) {
                    g.kill(&mut self.a.budget);
                }
            }
            Some(k) => {
                if let Some(p) = self.b.as_mut() {
                    let Pool { grains, budget, .. } = &mut **p;
                    if let Some(g) = grains.get_mut(k) {
                        g.kill(budget);
                    }
                }
            }
        }
    }

    /// A free slot for a new voice. Split, from the share playing fewer
    /// voices, so the two cores get about the same work.
    pub(super) fn free_slot(&self) -> Option<usize> {
        let n = self.a.voices.len();
        let free_a = self.a.voices.iter().position(|v| !v.active);
        let Some(b) = self.b.as_ref() else {
            return free_a;
        };
        let free_b = b.voices.iter().position(|v| !v.active).map(|k| n + k);
        if b.cost() < self.a.cost() {
            free_b.or(free_a)
        } else {
            free_a.or(free_b)
        }
    }

    /// Shares the grain budget out before a part of a block: each share
    /// keeps its grains and gets half of what is left, so together they
    /// never pass [`GRAIN_BUDGET`].
    pub(super) fn share_budget(&mut self) {
        if !self.split {
            return;
        }
        match self.b.as_mut() {
            Some(b) => {
                let (la, lb) = (self.a.budget.live(), b.budget.live());
                let free = GRAIN_BUDGET.saturating_sub(la + lb);
                self.a.budget.limit = la + free.div_ceil(2);
                b.budget.limit = lb + free / 2;
            }
            None => self.a.budget.limit = GRAIN_BUDGET.saturating_sub(self.b_limit),
        }
    }
}

/// Every voice of `pool` idle at once, its grains handed back.
pub(super) fn silence(pool: &mut Pool) {
    let Pool {
        voices,
        grains,
        budget,
        ..
    } = pool;
    for (v, g) in voices.iter_mut().zip(grains.iter_mut()) {
        if v.granular {
            g.kill(budget);
        }
        v.active = false;
        v.ringing = false;
        v.granular = false;
        v.held = false;
        v.kill_left = 0;
        v.tail_left = 0;
        v.follow.moving = false;
        v.data = None;
    }
}

impl Engine {
    /// At most `n` of the pool's voices play as full voices (any setting:
    /// filters, envelopes, loops, grains, pitch); the rest of the pool is
    /// for plain one-shots, which cost a fraction as much ([`plain_now`]).
    /// A full voice past the limit takes the place of another full voice,
    /// as a voice past a full pool does. The pad player keeps its full
    /// voices at the 24 it has always had and gives the drum hits more.
    pub fn set_full_voices(&mut self, n: usize) {
        self.full_limit = n.max(1);
    }

    /// The voice slot a new voice takes when there is no free one (or no
    /// free full voice, with `full_only`): a releasing voice first, then
    /// the oldest voice of the pad holding the most voices, whatever its
    /// kit. `held` is [`Engine::held`], all 0, and left so. Never
    /// allocates.
    pub(super) fn victim(pool: &Voices, held: &mut [u16], full_only: bool) -> usize {
        let rel = |x: &Voice| {
            x.kill_left > 0 || x.ringing || x.amp.as_ref().is_some_and(|a| a.is_releasing())
        };
        // One kit: a table on the stack, zeroed as it is made, as an
        // engine of one kit always had; more kits count in `held`, which
        // is zeroed again after.
        let one = held.len() <= PADS;
        let mut own = [0u16; PADS];
        let table: &mut [u16] = if one { &mut own } else { held };
        let at = |x: &Voice| usize::from(x.kit) * PADS + usize::from(x.pad);
        for (v, _) in pool.iter() {
            if v.active && !rel(v) && usize::from(v.pad) < PADS {
                if let Some(n) = table.get_mut(at(v)) {
                    *n += 1;
                }
            }
        }
        type Key = (bool, u16, std::cmp::Reverse<u64>);
        let mut best: Option<(usize, Key)> = None;
        for (i, (v, _)) in pool.iter().enumerate() {
            if full_only && (!v.active || v.light) {
                continue;
            }
            let count = if usize::from(v.pad) < PADS {
                table.get(at(v)).copied().unwrap_or(0)
            } else {
                0
            };
            let key = (rel(v), count, std::cmp::Reverse(v.age));
            if best.is_none_or(|(_, b)| key > b) {
                best = Some((i, key));
            }
        }
        if !one {
            for (v, _) in pool.iter() {
                if usize::from(v.pad) < PADS {
                    if let Some(n) = table.get_mut(at(v)) {
                        *n = 0;
                    }
                }
            }
        }
        best.map_or(0, |(i, _)| i)
    }

    /// The voices a let-go would release now: a Gate, Loop or Grains sound
    /// whose pad, key or take is held (not latched, not fading out). Every
    /// kit's; the lane's share counts only while it is here.
    pub fn held_voices(&self) -> usize {
        self.pool
            .iter()
            .filter(|(v, _)| {
                v.active
                    && v.held
                    && !v.latch
                    && v.kill_left == 0
                    && matches!(
                        v.mode,
                        Some(PlayMode::Gate | PlayMode::Loop | PlayMode::Granular)
                    )
            })
            .count()
    }

    /// Load shedding: the voice here this engine would give up first, the
    /// least audible one ([`Cull::audibility`]) by how loud it has been
    /// lately. Voices already fading and auditions are left be, and so are
    /// the lane's while it has them (they are not here). No allocation.
    pub fn cull_candidate(&self) -> Option<Cull> {
        self.cull_pick().map(|(_, c)| c)
    }

    /// Gives up the voice [`Engine::cull_candidate`] names, with the
    /// steal's short fade. True if it did. No allocation.
    pub fn cull(&mut self) -> bool {
        let Some((slot, _)) = self.cull_pick() else {
            return false;
        };
        let Some(v) = self.pool.voice_mut(slot) else {
            return false;
        };
        Self::kill(v);
        self.stats.culls += 1;
        true
    }

    pub(super) fn cull_pick(&self) -> Option<(usize, Cull)> {
        let mut best = None;
        for (i, (v, _)) in self.pool.iter().enumerate() {
            if !v.active || v.kill_left > 0 || v.audition {
                continue;
            }
            let c = Cull {
                level: v.heard,
                releasing: v.amp.as_ref().is_some_and(|a| a.is_releasing()),
            };
            best = Cull::first(best, Some((i, c)));
        }
        best
    }

    /// (pad, layer, source frame) of every playing voice, for playheads on screen.
    /// A granular voice reports its reading position (before spray); a
    /// voice playing a stretched copy, the source frame that part of the
    /// copy was made from.
    /// Kit 0's.
    pub fn playheads(&self) -> impl Iterator<Item = (u8, u8, u64)> + '_ {
        self.kit_playheads(0)
    }

    /// [`Engine::playheads`] of kit `kit`'s voices.
    pub fn kit_playheads(&self, kit: usize) -> impl Iterator<Item = (u8, u8, u64)> + '_ {
        self.pool
            .iter()
            .filter(move |(v, _)| v.active && usize::from(v.kit) == kit)
            .map(|(v, g)| (v.pad, v.layer, self.source_frame(v, g)))
    }

    /// Where voice `v` is in its source: see [`Engine::playheads`].
    pub(super) fn source_frame(&self, v: &Voice, g: &GrainVoice) -> u64 {
        let at = if v.granular {
            v.start + (f64::from(g.playhead()) * (v.end - v.start) as f64) as i64
        } else {
            let pos = v.pos >> 32;
            match self.copy_map(v) {
                Some(map) => map.source(pos as f64).round() as i64,
                None => pos,
            }
        };
        at.max(0) as u64
    }

    /// The map of the stretched copy voice `v` plays, if it plays one.
    pub(super) fn copy_map(&self, v: &Voice) -> Option<&CopyMap> {
        let l = self
            .views
            .get(usize::from(v.kit))?
            .pad_or_take(usize::from(v.pad))?
            .layers
            .get(usize::from(v.layer))?;
        let same = v.data.as_ref().is_some_and(|d| d.ptr_eq(&l.data));
        l.source_at.as_ref().filter(|_| same)
    }

    /// The voices kit 0's pad `pad` plays now, not counting those fading out.
    pub fn pad_voices(&self, pad: u8) -> usize {
        self.kit_pad_voices(0, pad)
    }

    /// [`Engine::pad_voices`] of kit `kit`'s pad.
    pub fn kit_pad_voices(&self, kit: usize, pad: u8) -> usize {
        self.pool
            .iter()
            .filter(|(v, _)| {
                v.active && usize::from(v.kit) == kit && v.pad == pad && v.kill_left == 0
            })
            .count()
    }

    /// The pads whose hit keeps a latched voice playing, as bits (pad `k`
    /// is bit `k`): a pad lights while it is latched. Hits only: a latched
    /// keygroup note or 16 Levels step isn't a pad of its own. Kit 0's.
    pub fn latched_pads(&self) -> u128 {
        self.kit_latched_pads(0)
    }

    /// [`Engine::latched_pads`] of kit `kit`.
    pub fn kit_latched_pads(&self, kit: usize) -> u128 {
        let mut bits = 0u128;
        for (v, _) in self.pool.iter() {
            if v.active
                && usize::from(v.kit) == kit
                && v.latch
                && v.held
                && v.note == NO_NOTE
                && usize::from(v.by) < PADS
            {
                bits |= 1 << v.by;
            }
        }
        bits
    }

    /// The hard stop: every voice and its grains go idle at once, without a
    /// fade, the master effect's grains stop and forget what they hold, and
    /// the events waiting are dropped (let-gos waiting aside too).
    /// [`Event::AllOff`] releases instead; this is for a reset or a panic,
    /// where a click is fine. The live settings, the tape and the view
    /// stay. Never allocates. Split, a share still away on the lane (late)
    /// stops as it comes back.
    pub fn stop_all(&mut self) {
        self.events.clear();
        for &i in &self.late {
            self.late_due[i as usize] = NOT_DUE;
        }
        self.late.clear();
        silence(&mut self.pool.a);
        match self.pool.b.as_deref_mut() {
            Some(b) => silence(b),
            None => self.stop_lane = self.pool.split,
        }
        if let Some(fx) = &mut self.master {
            fx.reset(&mut self.pool.a.budget);
        }
    }

    pub(super) fn kill(v: &mut Voice) {
        if v.active && v.kill_left == 0 {
            v.kill_left = KILL_FRAMES;
        }
    }
}
