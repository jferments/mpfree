//! A voice's sound: the plain, 8-point, Hermite and band-limited reads, the fades,
//! envelopes and filter, the movement, pressure and live values on their
//! steps, and the ring-out after a voice ends.

use super::*;

/// Filter coefficients, and anything pressure moves, are recomputed every
/// this many frames.
pub(super) const FILTER_STEP: u32 = 16;

/// Pressure is smoothed with this time constant, so steps between pressure
/// messages don't zipper.
pub(super) const PRESSURE_SMOOTH_S: f32 = 0.005;

pub(super) const ONE: i64 = 1 << 32;

/// The step past which a voice takes the band-limited read: two semitones
/// up (and a hair more, so a pad tuned exactly two up stays put whatever
/// the rounding). Up to it, the highest a 44.1 kHz sound holds
/// (22.05 kHz) moves to under 24.75 kHz and folds back no lower than
/// 19.35 kHz, above hearing; the band-limited read costs several times the
/// Hermite read, so a voice detuned a little, bent or swept by vibrato
/// doesn't pay for it.
pub const CLEAN_ABOVE: i64 = 4_820_937_788 + (1 << 16);

/// A voice's [`Voice::bl_above`]: never, unless it is started clean.
#[derive(Clone, Copy, Debug)]
pub(super) struct BlAbove(pub(super) i64);

impl Default for BlAbove {
    fn default() -> BlAbove {
        BlAbove(i64::MAX)
    }
}

/// The frames over which a voice crossfades from one read to another
/// ([`Voice::xf_left`]): about 6 ms at 44.1 kHz.
pub(super) const XF_FRAMES: u16 = 256;

/// The read a voice renders a block with, chosen once a block from where
/// it is ([`read_for`]), so no read's loop chooses inside it. Which frames
/// a read may look at (the region, clamped at its edges) is the voice's
/// business, not the read's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Read {
    /// Its sound's frames as they are, the whole block ([`render_plain`]).
    Plain,
    /// The 8-point read ([`bandlimit::near_kernel`]), and a whole frame as
    /// it is at the sound's own speed.
    Pitched,
    /// A Vintage pad's Hermite read, at every speed.
    Vintage,
    /// The band-limited read, from the frame its step passed
    /// [`Voice::bl_above`] on.
    Band,
}

/// The read for a voice's next block.
#[inline]
pub(super) fn read_for(v: &Voice) -> Read {
    if v.bl || v.step > v.bl_above.0 {
        Read::Band
    } else if !v.follow.moving && plain_now(v) {
        Read::Plain
    } else if v.vintage {
        Read::Vintage
    } else {
        Read::Pitched
    }
}

/// The LFO's wave `shape` at `phase` (0 to 1), -1 to 1, as the Grain
/// page's LFO draws it: the random waves from `held[0]` to `held[1]`.
pub(super) fn lfo_wave(shape: LfoShape, phase: f32, held: [f32; 2]) -> f32 {
    use std::f32::consts::{PI, TAU};
    match shape {
        LfoShape::Sine => (phase * TAU).sin(),
        LfoShape::Triangle => 1.0 - 4.0 * ((phase + 0.25).fract() - 0.5).abs(),
        LfoShape::SawUp => 2.0 * phase - 1.0,
        LfoShape::SawDown => 1.0 - 2.0 * phase,
        LfoShape::Square => {
            if phase < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        LfoShape::Hold => held[1],
        LfoShape::Drift => {
            let t = 0.5 - 0.5 * (phase * PI).cos();
            held[0] + (held[1] - held[0]) * t
        }
    }
}

/// A random value from -1 to 1.
pub(super) fn bipolar(rng: &mut Rng) -> f32 {
    (rng.next_u32() >> 8) as f32 / (1u32 << 23) as f32 - 1.0
}

/// A balance `x` from -1 (left) to 1 (right): the far side's gain falls
/// on a quarter sine and the near side's stays.
pub(super) fn balance(x: f32) -> [f32; 2] {
    let fall = (x.abs().min(1.0) * std::f32::consts::FRAC_PI_2).cos();
    if x > 0.0 {
        [fall, 1.0]
    } else {
        [1.0, fall]
    }
}

/// One voice's next `left.len()` frames, added into `left` and `right`.
/// A granular voice that ends gives its grains back to the budget.
#[allow(clippy::too_many_arguments)]
pub(super) fn play(
    v: &mut Voice,
    g: &mut GrainVoice,
    budget: &mut Budget,
    frame: u64,
    rate: f32,
    left: &mut [f32],
    right: &mut [f32],
    sl: &mut [f32],
    sr: &mut [f32],
) {
    if v.granular {
        render_granular(v, g, budget, frame, rate, left, right, sl, sr);
        if !v.active {
            g.kill(budget);
            v.granular = false;
        }
    } else {
        render_voice(v, rate, left, right);
    }
}

/// One frame of following the live settings: the gain's step, and every
/// [`FILTER_STEP`] frames the next steps ([`live_step`]). Only the loops of
/// a voice that is following call it ([`render_voice`]).
#[inline(always)]
pub(super) fn follow_live(v: &mut Voice) {
    if v.follow.count == 0 {
        live_step(v);
    }
    v.follow.count = (v.follow.count + 1) % FILTER_STEP;
    v.base_gain_l += v.follow.gain_inc[0];
    v.base_gain_r += v.follow.gain_inc[1];
    v.gain_l = v.base_gain_l * v.tape_gain;
    v.gain_r = v.base_gain_r * v.tape_gain;
}

/// [`follow_live`]'s step every [`FILTER_STEP`] frames: each value moves
/// `k` of the way to its target (the cutoff in octaves), the gain linearly
/// across the step; one close enough lands. The voice stops following when
/// all have landed.
#[inline(never)]
pub(super) fn live_step(v: &mut Voice) {
    let f = &mut v.follow;
    let k = f.k;
    let mut moving = false;
    let mut glide = |at: &mut f32, to: f32, inc: &mut f32| {
        let d = to - *at;
        if d.abs() <= 1e-5 * to.abs().max(at.abs()) + 1e-9 {
            *at = to;
            *inc = 0.0;
        } else {
            *inc = d * k / FILTER_STEP as f32;
            moving = true;
        }
    };
    glide(&mut v.base_gain_l, f.gain_to[0], &mut f.gain_inc[0]);
    glide(&mut v.base_gain_r, f.gain_to[1], &mut f.gain_inc[1]);
    if let Some(flt) = v.filter.as_mut().filter(|_| f.fc_to > 0.0) {
        let (fc, res) = (flt.cutoff_hz, flt.resonance);
        let ratio = f.fc_to / fc;
        if (ratio - 1.0).abs() <= 1e-4 {
            flt.cutoff_hz = f.fc_to;
        } else {
            flt.cutoff_hz = fc * ratio.powf(k);
            moving = true;
        }
        let d = f.res_to - res;
        if d.abs() <= 1e-4 {
            flt.resonance = f.res_to;
        } else {
            flt.resonance = res + d * k;
            moving = true;
        }
        if flt.cutoff_hz != fc || flt.resonance != res {
            v.filter_key = None;
        }
    }
    let d = f.cents_to - f.cents;
    if d != 0.0 {
        if d.abs() <= 0.01 {
            f.cents = f.cents_to;
        } else {
            f.cents += d * k;
            moving = true;
        }
        retime(v);
    }
    v.follow.moving = moving || v.jump_left > 0;
}

/// Smooths the voice's pressure and, every [`FILTER_STEP`] frames, updates
/// what it moves on the voice itself: gain, step (pitch) and cutoff.
#[inline]
pub(super) fn follow_pressure(v: &mut Voice) {
    v.press += (v.press_target - v.press) * v.press_coef;
    if v.press_count == 0 {
        pressure_step(v);
    }
    v.press_gain += v.press_gain_inc;
    v.press_count = (v.press_count + 1) % FILTER_STEP;
}

/// A warped layer's step at `tempo`: `tempo / warp_bpm` times its own.
pub(super) fn warped_step(step: i64, tempo: f64, warp_bpm: f64) -> i64 {
    ((step as f64 * tempo / warp_bpm).round() as i64).max(1)
}

/// [`follow_pressure`]'s update every [`FILTER_STEP`] frames, kept out of
/// the per-frame loop.
#[inline(never)]
pub(super) fn pressure_step(v: &mut Voice) {
    // The smoothing lands, so pressure let go doesn't creep toward zero
    // through the denormals.
    if (v.press_target - v.press).abs() < TINY {
        v.press = v.press_target;
    }
    let d = v.route.depth * v.press;
    match v.route.target {
        PressureTarget::Level => {
            v.press_gain_inc = (db_to_gain(12.0 * d) - v.press_gain) / FILTER_STEP as f32
        }
        PressureTarget::Pitch => retime(v),
        PressureTarget::Filter => v.press_fc = (4.0 * d).exp2(),
        _ => {}
    }
}

/// The voice's step from its base step, its pitch pressure, its speed
/// (the tape's, with wow and flutter), the program's tune and its
/// movement's pitch. A granular voice's grains keep their own pitch.
pub(super) fn retime(v: &mut Voice) {
    if v.granular {
        return;
    }
    let mut step = v.base_step as f64;
    if v.route.target == PressureTarget::Pitch {
        step *= f64::from(v.route.depth * v.press).exp2();
    }
    if v.speed != 1.0 {
        step *= f64::from(v.speed);
    }
    let cents = v.follow.cents + v.motion.cents;
    if cents != 0.0 {
        step *= f64::from(cents / 1200.0).exp2();
    }
    v.step = step.round().max(1.0) as i64;
}

/// One frame of the voice's movement: every [`FILTER_STEP`] frames its
/// next step ([`motion_step`]), and each frame the level and pan's glide.
/// Only called while [`Motion::on`].
#[inline(always)]
pub(super) fn follow_motion(v: &mut Voice) {
    if v.motion.count == 0 {
        motion_step(v);
    }
    let m = &mut v.motion;
    m.count = (m.count + 1) % FILTER_STEP;
    m.side[0] += m.side_inc[0];
    m.side[1] += m.side_inc[1];
}

/// [`follow_motion`]'s step every [`FILTER_STEP`] frames, kept out of the
/// per-frame loop: the LFO's value and the pitch envelope's, on to the
/// pitch (a new step), the cutoff ([`retune`] sees it) and the gain on
/// each side. It stops itself when nothing moves any more: no LFO, the
/// envelope back home and the gains landed.
#[inline(never)]
pub(super) fn motion_step(v: &mut Voice) {
    let m = &mut v.motion;
    let mut cents = m.vel_cents + m.env_cents + m.glide_cents;
    if m.env_cents != 0.0 {
        m.env_cents *= m.env_k;
        if m.env_cents.abs() < 0.01 {
            m.env_cents = 0.0;
        }
    }
    if m.glide_cents != 0.0 {
        m.glide_cents = if m.glide_cents.abs() <= m.glide_inc {
            0.0
        } else {
            m.glide_cents - m.glide_inc.copysign(m.glide_cents)
        };
    }
    let mut fc = m.vel_fc;
    let mut side = [1.0f32; 2];
    let lfo = m.to != LfoTarget::Off;
    if lfo {
        let x = m.depth * lfo_wave(m.shape, m.phase, m.held);
        m.phase += m.inc;
        if m.phase >= 1.0 {
            m.phase = m.phase.fract();
            m.held = [m.held[1], bipolar(&mut m.rng)];
        }
        match m.to {
            LfoTarget::Pitch => cents += 1200.0 * x,
            LfoTarget::Cutoff => fc *= (4.0 * x).exp2(),
            LfoTarget::Level => {
                let g = 1.0 - 0.5 * (m.depth.abs() - x);
                side = [g, g];
            }
            LfoTarget::Pan => side = balance(x),
            LfoTarget::Off => {}
        }
    }
    let mut gliding = false;
    for k in 0..2 {
        let d = side[k] - m.side[k];
        if d.abs() < 1e-6 {
            m.side[k] = side[k];
            m.side_inc[k] = 0.0;
        } else {
            m.side_inc[k] = d / FILTER_STEP as f32;
            gliding = true;
        }
    }
    m.fc = fc;
    let cents = if m.pitched { cents } else { 0.0 };
    // The envelope on the pitch keeps it on a step past coming home, so
    // the pitch lands on the velocity's alone.
    let homing = cents != m.vel_cents;
    m.on = lfo || homing || gliding;
    if cents != m.cents {
        m.cents = cents;
        retime(v);
    }
}

/// Sets the voice's filter for envelope level `e`. The coefficients only
/// depend on `e` and the cutoff factors of pressure and movement, so while
/// none moves (a held sustain, no pressure, no LFO on the cutoff) this
/// skips the `exp2` and `tan`. A state-variable type glides to the new
/// cutoff over the [`FILTER_STEP`] frames until the next ([`VoiceFilter`]),
/// so an envelope or LFO moves it every frame.
/// It also flushes the filter's state: ringing out on silence, the filter
/// reaches exact zeros instead of denormals (here, out of the per-frame
/// loop, it costs nothing measurable).
#[inline(never)]
pub(super) fn retune(v: &mut Voice, f: Filter, e: f32, rate: f32) {
    v.filt.flush(f.kind);
    let moved = v.press_fc * v.motion.fc;
    let key = (e.to_bits(), moved.to_bits());
    if v.filter_key == Some(key) {
        return;
    }
    v.filter_key = Some(key);
    let fc = f.cutoff_hz * (f.env_octaves * e).exp2() * moved;
    v.filt.set(&f, fc, f.resonance, rate, FILTER_STEP);
}

/// `start` times pressure's gain, the amp envelope's next value and the kill
/// fade, in that order; then the movement's next step, before the filter
/// so a movement on the cutoff reaches it this frame.
#[inline(always)]
fn amp_gain(v: &mut Voice, start: f32) -> f32 {
    let mut amp = start;
    if v.route.target != PressureTarget::Off {
        follow_pressure(v);
        amp *= v.press_gain;
    }
    if let Some(a) = &mut v.amp {
        amp *= a.next();
    }
    if v.kill_left > 0 {
        amp *= f32::from(v.kill_left) / f32::from(KILL_FRAMES);
    }
    if v.motion.on {
        follow_motion(v);
    }
    amp
}

/// The filter envelope's step and the filter retuned every
/// [`FILTER_STEP`] frames, ahead of a frame through it: the filter's type,
/// or `None` with no filter.
#[inline(always)]
fn filter_tick(v: &mut Voice, rate: f32) -> Option<FilterKind> {
    let (Some(f), Some(fe)) = (v.filter, v.fenv.as_mut()) else {
        return None;
    };
    let e = fe.next();
    if v.filter_count == 0 {
        retune(v, f, e, rate);
    }
    v.filter_count = (v.filter_count + 1) % FILTER_STEP;
    Some(f.kind)
}

/// Everything after the source read, for one frame: the filter, then the
/// amp envelope, pressure's gain, the kill fade, the voice's level and pan
/// and the movement's sides after it (so a Drive or a loud resonance hears
/// the sound as it is, not where its envelope or its velocity put it: the
/// filter gap plan's F2), and the stolen voice's tail. A Comb comes after
/// all the gains instead, so it rings on once the sound has ended
/// ([`ring_out`]); a voice with no filter or a Comb takes its gains in the
/// order it always did, so it renders exactly as before (and as
/// [`render_plain`] does). `g` is the gain from the region's fades, part
/// of the sound. Marks the voice done when its kill fade or envelope ends.
#[inline(always)]
pub(super) fn finish<const FOLLOW: bool>(
    v: &mut Voice,
    rate: f32,
    mut l: f32,
    mut r: f32,
    g: f32,
) -> (f32, f32) {
    if FOLLOW {
        follow_live(v);
    }
    // One test a frame, each order its own path: every type but the Comb
    // comes before the gains (`v.filter` is `None` for Off).
    if v.filter.is_some_and(|f| f.kind != FilterKind::Comb) {
        let amp = amp_gain(v, 1.0);
        l *= g;
        r *= g;
        if let Some(kind) = filter_tick(v, rate) {
            (l, r) = v.filt.process_first(kind, l, r);
        }
        l *= amp * v.gain_l;
        r *= amp * v.gain_r;
        if v.motion.on {
            l *= v.motion.side[0];
            r *= v.motion.side[1];
        }
    } else {
        // The region's fades in the gain, in the order it always had.
        let amp = amp_gain(v, g);
        if v.motion.on {
            l *= v.motion.side[0];
            r *= v.motion.side[1];
        }
        l *= amp * v.gain_l;
        r *= amp * v.gain_r;
        // A Comb, if any.
        if filter_tick(v, rate).is_some() {
            (l, r) = v.filt.comb.process(l, r);
        }
    }

    if v.tail_left > 0 {
        let t = f32::from(v.tail_left) / f32::from(STEAL_FRAMES);
        l += v.tail_l * t;
        r += v.tail_r * t;
        v.tail_left -= 1;
    }
    v.last_l = l;
    v.last_r = r;

    if v.kill_left > 0 {
        v.kill_left -= 1;
        if v.kill_left == 0 {
            v.active = false;
            // Cut, not ended: a Comb doesn't ring on.
            v.filt.comb.reset();
        }
    }
    if v.amp.as_ref().is_some_and(|a| a.is_done()) {
        v.active = false;
    }
    (l, r)
}

/// A sample voice's next `left.len()` frames, added into `left` and
/// `right`. Whether it follows a live setting is settled once a call (only
/// events start that, and they come between calls), so a voice that isn't
/// following runs the loop it always did.
pub(super) fn render_voice(v: &mut Voice, rate: f32, left: &mut [f32], right: &mut [f32]) {
    if v.ringing {
        ring_out(v, left, right);
    } else {
        // A run stops early, the voice still playing, when its read
        // changes: its step passed CLEAN_ABOVE (a glide, pressure), or it
        // came onto or off its sound's own speed on a whole frame. The
        // next run reads the new way, fading in over the old.
        let mut done = 0;
        while done < left.len() {
            let (l, r) = (&mut left[done..], &mut right[done..]);
            let read = read_for(v);
            done += match (read, v.follow.moving) {
                (Read::Band, _) => render_voice_bl(v, rate, l, r),
                (Read::Plain, _) => {
                    render_plain(v, l, r);
                    l.len()
                }
                (Read::Vintage, true) => render_voice_as::<true, false, true>(v, rate, l, r),
                (_, true) => render_voice_as::<true, false, false>(v, rate, l, r),
                (Read::Vintage, false) => render_voice_as::<false, false, true>(v, rate, l, r),
                _ => render_voice_as::<false, false, false>(v, rate, l, r),
            };
            if !v.active {
                break;
            }
        }
        if !v.active && done < left.len() && rings_on(v) {
            v.active = true;
            v.ringing = true;
            ring_out(v, &mut left[done..], &mut right[done..]);
        } else if !v.active && rings_on(v) {
            // Ended on the block's last frame: it rings from the next.
            v.active = true;
            v.ringing = true;
        }
    }
    v.heard = (v.heard * HEARD_FALL).max(v.last_l.abs().max(v.last_r.abs()));
    if v.heard < TINY {
        v.heard = 0.0;
    }
}

/// [`render_voice`] for a voice reading band-limited, from now on. Kept
/// out of line, so the plain and Hermite paths stay as small as they were.
#[inline(never)]
pub(super) fn render_voice_bl(
    v: &mut Voice,
    rate: f32,
    left: &mut [f32],
    right: &mut [f32],
) -> usize {
    v.bl = true;
    if v.follow.moving {
        render_voice_as::<true, true, false>(v, rate, left, right)
    } else {
        render_voice_as::<false, true, false>(v, rate, left, right)
    }
}

/// Whether a voice that just ended goes on as its Comb's ringing: its
/// sound or its envelope ended (a cut resets the comb, [`finish`]), and
/// the comb still holds something to hear.
#[inline]
pub(super) fn rings_on(v: &Voice) -> bool {
    v.filter.is_some_and(|f| f.kind == FilterKind::Comb) && v.filt.comb.rings()
}

/// A Comb's ringing after its voice's sound ended, added into `left` and
/// `right`, until it is too quiet to hear ([`Comb::rings`]) or a cut's
/// fade ends. The delay stays where the sound left it.
#[cold]
#[inline(never)]
pub(super) fn ring_out(v: &mut Voice, left: &mut [f32], right: &mut [f32]) {
    for (ol, or) in left.iter_mut().zip(right.iter_mut()) {
        let (mut l, mut r) = v.filt.comb.process(0.0, 0.0);
        if v.kill_left > 0 {
            let t = f32::from(v.kill_left) / f32::from(KILL_FRAMES);
            (l, r) = (l * t, r * t);
            v.kill_left -= 1;
            if v.kill_left == 0 {
                v.filt.comb.reset();
            }
        }
        *ol += l;
        *or += r;
        v.last_l = l;
        v.last_r = r;
        if !v.filt.comb.rings() {
            v.active = false;
            v.ringing = false;
            break;
        }
    }
}

/// Whether an amp envelope with these settings soon holds a level (its
/// sustain, reached within a couple of frames) until a release, so a voice
/// with it plays plain ([`plain_now`]) after its first frames.
pub(super) fn amp_holds(a: &Ahdsr, rate: f32) -> bool {
    let frames = |ms: f32| ms.max(0.0) * 0.001 * rate;
    frames(a.attack_ms) < 1.0
        && frames(a.hold_ms) < 1.0
        && a.sustain > 0.0
        && (a.sustain >= 1.0 || frames(a.decay_ms) < 1.0)
}

/// Whether the voice's next frames are a plain copy of its sound at a
/// fixed gain, so [`render_plain`] gives exactly what the full loop would:
/// forward at the sound's own speed from a whole frame (no Hermite
/// blend), no loop, fades, filter or pressure, its envelope holding a
/// level, no kill fade or stolen voice's tail under way, and no other
/// read heard last (one that was fades out first, through [`hear`]). A
/// kit's drum hits are like this from their second frame to their last.
#[inline]
pub(super) fn plain_now(v: &Voice) -> bool {
    v.step == ONE
        && !v.backwards
        && !v.looping
        && v.pos & (ONE - 1) == 0
        && v.fade_in.is_none()
        && v.fade_out.is_none()
        && v.filter.is_none()
        && v.route.target == PressureTarget::Off
        && !v.motion.on
        && v.kill_left == 0
        && v.tail_left == 0
        && v.amp.as_ref().is_none_or(|a| a.is_steady())
        && (!v.stereo || (v.pick_l == 0 && v.pick_r == 1))
        && v.xf_left == 0
        && v.last_read.is_none_or(|r| r == Heard::Exact)
}

/// A plain voice's next frames ([`plain_now`]): each frame of its sound
/// times its gain, added in. Bit for bit what [`render_voice_as`] gives
/// for such a voice (`a_plain_voice_renders_as_the_full_loop_does`), at a
/// fraction of the cost: the loop is a multiply and an add a sample.
pub(super) fn render_plain(v: &mut Voice, left: &mut [f32], right: &mut [f32]) {
    v.last_read = Some(Heard::Exact);
    let Some(data) = v.data.take() else {
        v.active = false;
        return;
    };
    match &data {
        Pcm::F32(d) => render_plain_from(v, d, left, right),
        Pcm::I16(d) => render_plain_from(v, d, left, right),
    }
    if v.active {
        v.data = Some(data);
    }
}

/// [`render_plain`] from the samples as the sound holds them.
#[inline(always)]
pub(super) fn render_plain_from<S: Sample>(
    v: &mut Voice,
    data: &[S],
    left: &mut [f32],
    right: &mut [f32],
) {
    let i = v.pos >> 32;
    let n = if i < v.start {
        0
    } else {
        left.len().min((v.end - i).max(0) as usize)
    };
    // The full loop's gain: the envelope's level (its `g`), times the
    // voice's gain on each side.
    let g = v.amp.as_ref().map_or(1.0, |a| a.level());
    let (gl, gr) = (g * v.gain_l, g * v.gain_r);
    // Raw samples times the gain with the samples' scale in it: exactly
    // the scaled samples times the gain (the scale is a power of two).
    let (sl, sr) = (gl * S::SCALE, gr * S::SCALE);
    if n > 0 {
        let k = i as usize;
        let (left, right) = (&mut left[..n], &mut right[..n]);
        if v.stereo {
            let src = &data[2 * k..2 * (k + n)];
            for ((ol, or), f) in left
                .iter_mut()
                .zip(right.iter_mut())
                .zip(src.chunks_exact(2))
            {
                *ol += f[0].raw() * sl;
                *or += f[1].raw() * sr;
            }
            v.last_l = src[2 * n - 2].raw() * sl;
            v.last_r = src[2 * n - 1].raw() * sr;
        } else {
            let src = &data[k..k + n];
            for ((ol, or), x) in left.iter_mut().zip(right.iter_mut()).zip(src) {
                *ol += x.raw() * sl;
                *or += x.raw() * sr;
            }
            v.last_l = src[n - 1].raw() * sl;
            v.last_r = src[n - 1].raw() * sr;
        }
        v.pos += (n as i64) << 32;
    }
    if n < left.len() {
        // It reached its end within the block, as the full loop finds.
        v.active = false;
    }
}

/// How much a voice's [`Voice::heard`] level falls each render it is not
/// held up by its output (about 3 dB in four renders).
pub(super) const HEARD_FALL: f32 = 0.84;

/// Sets which copy a band-limited voice reads and how far its sinc
/// stretches, for its step now: its sound, or the octave copy that brings
/// its speed under [`bandlimit::MAX_STRETCH`] (positions and the region
/// shift right by the copy's octave). Set again whenever its step moves,
/// on that frame, so the sound doesn't depend on how the blocks fall.
#[inline]
pub(super) fn aim_bl(v: &mut Voice, octaves: Option<&bandlimit::Octaves>) {
    let mut r = v.step as f32 / ONE as f32;
    let mut octave = 0u32;
    if let Some(o) = octaves {
        while r >= bandlimit::MAX_STRETCH && o.level(octave as usize + 1).is_some() {
            octave += 1;
            r *= 0.5;
        }
    }
    v.bl_octave = octave;
    v.bl_reach = Reach::new(r);
    v.bl_step = v.step;
}

/// Both output channels of a band-limited voice at `pos` ([`aim_bl`] says
/// from what).
#[inline(always)]
pub(super) fn read_bl<S: Sample>(
    v: &Voice,
    kernel: &bandlimit::Kernel,
    data: &[S],
    octaves: Option<&bandlimit::Octaves>,
    pos: i64,
) -> (f32, f32) {
    let k = v.bl_octave;
    let (lo, hi, at) = (v.start >> k, (v.end - 1) >> k, pos >> k);
    // What reads straight, in the copy's frames, and a frame past a loop's
    // end folded as the sound's own frame there is ([`Voice::fold`]).
    let unit = (1i64 << k) - 1;
    let fast = ((v.fast_lo + unit) >> k, (v.fast_hi - 1) >> k);
    let fold = |j: i64| v.fold(j << k) >> k;
    let picks = (v.pick_l, v.pick_r);
    match octaves.and_then(|o| o.level(k as usize)) {
        // The copies are floats, whatever the sound holds.
        Some(copy) if k > 0 => bandlimit::read2_folded(
            kernel,
            &copy[..],
            v.stereo,
            lo,
            hi,
            fast,
            fold,
            at,
            v.bl_reach,
            picks,
        ),
        _ => bandlimit::read2_folded(
            kernel, data, v.stereo, lo, hi, fast, fold, at, v.bl_reach, picks,
        ),
    }
}

/// A sample voice's next frames, added into `left` and `right`, read with
/// the 8-point read, Hermite (`VINTAGE`) or band-limited (`BL`). Returns the
/// frames it rendered: all of them, unless the voice ended or its read
/// changed on a frame ([`render_voice`] carries on with the new one). Kept
/// out of line: inlined into [`render_voice`], every read's loop got slower.
#[inline(never)]
pub(super) fn render_voice_as<const FOLLOW: bool, const BL: bool, const VINTAGE: bool>(
    v: &mut Voice,
    rate: f32,
    left: &mut [f32],
    right: &mut [f32],
) -> usize {
    // Taken out of the voice for the render and put back after (a clone
    // would touch the shared count, which both cores' voices share).
    let Some(data) = v.data.take() else {
        v.active = false;
        return left.len();
    };
    let done = match &data {
        Pcm::F32(d) => render_voice_runs::<f32, FOLLOW, BL, VINTAGE>(v, d, rate, left, right),
        Pcm::I16(d) => render_voice_runs::<i16, FOLLOW, BL, VINTAGE>(v, d, rate, left, right),
    };
    if v.active {
        v.data = Some(data);
    }
    done
}

/// [`render_voice_as`] in at most two runs: while a change of read fades
/// in, the frames that carry the old read under the new, then the rest
/// with no fade to look after.
#[inline(always)]
fn render_voice_runs<S: Sample, const FOLLOW: bool, const BL: bool, const VINTAGE: bool>(
    v: &mut Voice,
    data: &[S],
    rate: f32,
    left: &mut [f32],
    right: &mut [f32],
) -> usize {
    if !VINTAGE {
        let now = if BL {
            Heard::Band
        } else if v.on_frame(v.pos) {
            Heard::Exact
        } else {
            Heard::Near
        };
        hear(v, now);
    }
    if VINTAGE || v.xf_left == 0 {
        return render_voice_from::<S, FOLLOW, BL, VINTAGE, false>(v, data, rate, left, right);
    }
    let n = usize::from(v.xf_left).min(left.len());
    let (l, r) = (&mut left[..n], &mut right[..n]);
    let done = render_voice_from::<S, FOLLOW, BL, VINTAGE, true>(v, data, rate, l, r);
    if done < n || !v.active || n == left.len() {
        return done;
    }
    let (l, r) = (&mut left[n..], &mut right[n..]);
    n + render_voice_from::<S, FOLLOW, BL, VINTAGE, false>(v, data, rate, l, r)
}

/// One run of [`render_voice_as`] from the samples as the sound holds
/// them, with (`XF`) or without a change of read fading in.
#[inline(always)]
pub(super) fn render_voice_from<
    S: Sample,
    const FOLLOW: bool,
    const BL: bool,
    const VINTAGE: bool,
    const XF: bool,
>(
    v: &mut Voice,
    data: &[S],
    rate: f32,
    left: &mut [f32],
    right: &mut [f32],
) -> usize {
    // The band-limited read's copies, taken only by a voice that reads
    // with them, and its kernel, fetched once a call.
    let octaves = if BL { v.octaves.take() } else { None };
    let copies = octaves.as_deref().and_then(OctaveCell::get);
    // What its reads take straight, for this run's loop and region.
    v.refresh_fast();
    let kernel = bandlimit::kernel();
    let near = bandlimit::near_kernel();
    // Whether the 8-point read takes whole frames as they are: found once,
    // as at one step it can't change ([`Voice::on_frame`]). The run ends
    // when the step moves it.
    let seen = v.step;
    let exact = !BL && !VINTAGE && v.on_frame(v.pos);
    let mut xf = if XF { v.xf_left } else { 0 };
    let mut done = left.len();
    for (k, (ol, or)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
        let i = v.pos >> 32;
        if i < v.start || i >= v.end {
            v.active = false;
            done = k;
            break;
        }
        if BL && v.step != v.bl_step {
            aim_bl(v, copies);
        }

        let (mut l, mut r) =
            read_one::<S, BL, VINTAGE>(v, kernel, near, data, copies, v.pos, exact, xf);

        // Forward-loop crossfade: blend in the audio before the loop start.
        if v.looping && !v.ping_pong && v.crossfade > 0 && i >= v.loop_end - v.crossfade {
            let x = (i - (v.loop_end - v.crossfade)) as f32 / v.crossfade as f32;
            let (out, inn) = equal_power(x);
            let back = v.pos - ((v.loop_end - v.loop_start) << 32);
            let (bl, br) =
                read_one::<S, BL, VINTAGE>(v, kernel, near, data, copies, back, exact, xf);
            l = l * out + bl * inn;
            r = r * out + br * inn;
        }
        // An edit's jump into a moved loop: the step fading out. A voice
        // follows while one is under way ([`live_step`]).
        if FOLLOW && v.jump_left > 0 {
            let t = f32::from(v.jump_left) / f32::from(JUMP_FRAMES);
            l += v.jump[0] * t;
            r += v.jump[1] * t;
            v.jump_left -= 1;
        }

        let mut g = 1.0f32;
        if let Some(f) = v.fade_in {
            let d = if v.reverse {
                v.end - 1 - i
            } else {
                i - v.start
            };
            if d < i64::from(f.frames) {
                g *= f.shape(d as f32 / f.frames as f32);
            }
        }
        if let Some(f) = v.fade_out {
            let d = if v.reverse {
                i - v.start
            } else {
                v.end - 1 - i
            };
            if d < i64::from(f.frames) {
                g *= f.shape(d as f32 / f.frames as f32);
            }
        }
        let (l, r) = finish::<FOLLOW>(v, rate, l, r, g);
        *ol += l;
        *or += r;
        if XF && xf > 0 {
            xf -= 1;
        }

        // Advance.
        if v.backwards {
            v.pos -= v.step;
        } else {
            v.pos += v.step;
        }
        let j = v.pos >> 32;
        if v.looping {
            if v.ping_pong {
                if !v.backwards && j >= v.loop_end {
                    v.backwards = true;
                    v.pos = ((v.loop_end - 1) << 32) - (v.pos - (v.loop_end << 32));
                    went_round(v);
                } else if v.backwards && j < v.loop_start {
                    v.backwards = false;
                    v.pos = (v.loop_start << 32) + ((v.loop_start << 32) - v.pos);
                    went_round(v);
                }
            } else if j >= v.loop_end {
                v.pos -= (v.loop_end - v.loop_start) << 32;
                went_round(v);
            }
        }

        if !v.active {
            done = k + 1;
            break;
        }
        // The step moved (a glide, pressure, the movement's pitch): past
        // two semitones up, or onto or off the sound's own speed on a whole
        // frame, another read takes over from the next frame.
        if !BL
            && v.step != seen
            && (v.step > v.bl_above.0 || !VINTAGE && v.on_frame(v.pos) != exact)
        {
            done = k + 1;
            break;
        }
    }
    if XF {
        v.xf_left = xf;
    }
    if v.active && BL {
        v.octaves = octaves;
    }
    done
}

/// One frame of [`render_voice_from`]'s read at `pos`: the band-limited,
/// Hermite or 8-point read, with the old read under it while a change of
/// read fades in. Always inlined: as a call of its own it costs the held
/// pads a tenth more.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn read_one<S: Sample, const BL: bool, const VINTAGE: bool>(
    v: &Voice,
    kernel: &bandlimit::Kernel,
    near: &bandlimit::NearKernel,
    data: &[S],
    copies: Option<&bandlimit::Octaves>,
    pos: i64,
    exact: bool,
    xf: u16,
) -> (f32, f32) {
    let new = if BL {
        read_bl(v, kernel, data, copies, pos)
    } else if VINTAGE {
        v.read2_hermite(data, pos)
    } else {
        v.read2_near(near, data, pos, exact)
    };
    if VINTAGE || xf == 0 {
        new
    } else {
        faded(v, data, pos, new, xf)
    }
}

/// `new` with the read the voice heard before it faded under it, `xf`
/// frames of [`XF_FRAMES`] left. The plain copy's side is the band-limited
/// read unstretched: exact on a whole frame, as the copy is, flat to about
/// 19 kHz and with no phase error between frames, so the fade carries only
/// the new read's own difference. Out of the render loop: a voice spends
/// 6 ms here per change of read.
#[cold]
#[inline(never)]
fn faded<S: Sample>(v: &Voice, data: &[S], pos: i64, new: (f32, f32), xf: u16) -> (f32, f32) {
    let old = match v.xf_from {
        Heard::Exact => {
            let picks = (v.pick_l, v.pick_r);
            let k = bandlimit::kernel();
            bandlimit::read2_folded(
                k,
                data,
                v.stereo,
                v.start,
                v.end - 1,
                (v.fast_lo, v.fast_hi - 1),
                |j| v.fold(j),
                pos,
                Reach::default(),
                picks,
            )
        }
        Heard::Near | Heard::Band => v.read2_near(bandlimit::near_kernel(), data, pos, false),
    };
    let g = f32::from(xf) * (1.0 / f32::from(XF_FRAMES));
    (new.0 + (old.0 - new.0) * g, new.1 + (old.1 - new.1) * g)
}

/// The voice went round its loop (or turned at an end of a ping-pong
/// one): from now on its reads fold the frames before the loop's start
/// too ([`Voice::fold`]).
#[inline]
fn went_round(v: &mut Voice) {
    if !v.wrapped {
        v.wrapped = true;
        v.refresh_fast();
    }
}

/// The voice now hears `now`: if it heard another read before, the
/// crossfade from that one starts ([`Voice::xf_left`]).
fn hear(v: &mut Voice, now: Heard) {
    if v.last_read != Some(now) {
        if let Some(was) = v.last_read {
            v.xf_from = was;
            v.xf_left = XF_FRAMES;
        }
        v.last_read = Some(now);
    }
}
