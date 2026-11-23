//! Voices that are sounding follow an edit of their kit, and the pads' live
//! values and the program's offsets, gliding there so nothing clicks.

use super::*;

/// How long the step is smoothed over when an edit moves a loop behind a
/// playing voice and it jumps into the loop ([`Engine::edit_view`]).
pub const JUMP_FRAMES: u16 = 64;

/// How fast playing voices follow a live setting ([`Event::PadLive`],
/// [`Event::ProgramLive`]): a one-pole glide with this time constant,
/// stepped every [`FILTER_STEP`] frames (linear within each step for the
/// gain).
pub(super) const LIVE_SMOOTH_S: f32 = 0.010;

/// The filter a live cutoff gives a pad without one: a 2-pole low-pass,
/// starting open (the filter clamps the cutoff below half the rate).
pub(super) const OPEN_FILTER: Filter = Filter {
    kind: FilterKind::LowPass,
    poles: 2,
    cutoff_hz: 22_000.0,
    resonance: 0.0,
    env_octaves: 0.0,
    drive_db: 0.0,
};

/// A voice's filter with the live settings on it: `cutoff` and `resonance`
/// in place of its own, then the program's offset `octaves` on the cutoff.
/// A voice without a filter gets an open 2-pole low-pass when a cutoff is
/// set or the offset darkens.
pub(super) fn live_filter(
    f: Option<Filter>,
    cutoff: Option<f32>,
    resonance: Option<f32>,
    octaves: f32,
) -> Option<Filter> {
    let mut f = match f {
        Some(f) => f,
        None if cutoff.is_some() || octaves < 0.0 => OPEN_FILTER,
        None => return None,
    };
    if let Some(hz) = cutoff {
        f.cutoff_hz = hz;
    }
    if let Some(r) = resonance {
        f.resonance = r;
    }
    if octaves != 0.0 {
        f.cutoff_hz *= octaves.exp2();
    }
    Some(f)
}

/// The gain (left, right) of a voice whose gain before the pad's level
/// and pan is `unit`, at `db` and pan `pan` (its layer's pan adds to it).
pub(super) fn live_gains(unit: f32, layer_pan: f32, db: f32, pan: f32) -> (f32, f32) {
    let (pl, pr) = pan_gains(layer_pan + pan);
    let g = unit * db_to_gain(db);
    (g * pl, g * pr)
}

pub(super) fn retarget(
    v: &mut Voice,
    live: &[f32; PadField::COUNT],
    program: &[f32; ProgramField::COUNT],
) {
    if !v.active || v.kill_left > 0 {
        return;
    }
    let get = |f: PadField| Some(live[f as usize]).filter(|x| !x.is_nan());
    let f = &mut v.follow;
    let db = get(PadField::Level).unwrap_or(f.own_db) + program[ProgramField::Level as usize];
    let pan = get(PadField::Pan).unwrap_or(f.own_pan);
    let (l, r) = live_gains(v.unit_gain, v.layer_pan, db, pan);
    f.gain_to = [l, r];
    let cutoff = get(PadField::Cutoff);
    let octaves = program[ProgramField::Cutoff as usize];
    if v.filter.is_none() && (cutoff.is_some() || octaves < 0.0) {
        // From open, so it comes in without a click.
        v.filter = Some(OPEN_FILTER);
        v.fenv = Some(Envelope::idle());
        v.filt.clear();
        v.filter_key = None;
        f.own_fc = OPEN_FILTER.cutoff_hz;
    }
    if v.filter.is_some() {
        f.fc_to = cutoff.unwrap_or(f.own_fc) * octaves.exp2();
        f.res_to = get(PadField::Resonance).unwrap_or(f.own_res);
    }
    f.cents_to = if v.warp_bpm > 0.0 {
        0.0
    } else {
        program[ProgramField::Tune as usize]
    };
    if !f.moving {
        // A voice that had landed may have run on through the rest of
        // that render call's loop, or not: its steps start afresh on the
        // event's frame, so blocks of any size play it the same.
        f.count = 0;
    }
    f.moving = true;
    // A live LFO depth, or back to the pad's own.
    let m = &mut v.motion;
    m.depth = get(PadField::LfoDepth).unwrap_or(m.own_depth);
    if m.to != LfoTarget::Off {
        m.on = true;
    }
}

/// Playing voice `v` (with its grain voice `g`) follows an edit of its pad
/// in `view` ([`Engine::edit_view`]), if its layer there still plays its
/// sound: the loop points at once, a new tune at once, a granular voice's
/// grain settings, and the level, pan, cutoff and resonance gliding there
/// from where they are, under `live` (its pad's live row) and `program`.
/// `was_live`: it followed a live value that the new view cleared, so it
/// glides back to its own even if the edit didn't change it.
pub(super) fn follow_edit(
    v: &mut Voice,
    g: &mut GrainVoice,
    view: &EngineView,
    live: &[f32; PadField::COUNT],
    program: &[f32; ProgramField::COUNT],
    was_live: bool,
    bpm: f64,
) {
    if !v.active || v.kill_left > 0 || v.audition {
        return;
    }
    let Some(vp) = view.pad_or_take(usize::from(v.pad)) else {
        return;
    };
    // The LFO's wave, target, rate and depth, from where it is in its
    // cycle (sampler R5 part 4).
    let depth = Some(live[PadField::LfoDepth as usize]).filter(|x| !x.is_nan());
    v.motion
        .set_lfo(&vp.lfo, depth.unwrap_or(vp.lfo.depth), bpm);
    let Some(l) = vp.layers.get(usize::from(v.layer)) else {
        return;
    };
    let same_sound = v.stereo == l.stereo
        && v.src_rate == l.src_rate
        && v.data.as_ref().is_some_and(|d| d.ptr_eq(&l.data));
    if !same_sound {
        return;
    }
    let f = v.follow;
    let mut moved = was_live;

    // Level and pan: the layer's gain and pan under the hit's own.
    let unit_gain = l.unit_gain * v.hit_gain;
    if unit_gain != v.unit_gain
        || l.pan != v.layer_pan
        || vp.level_db != f.own_db
        || vp.pan != f.own_pan
    {
        (v.unit_gain, v.layer_pan) = (unit_gain, l.pan);
        (v.follow.own_db, v.follow.own_pan) = (vp.level_db, vp.pan);
        moved = true;
    }

    // The filter's cutoff and resonance, where the filter is still the
    // same kind (and its cutoff the pad's, not 16 Levels'). A low-pass
    // turned on comes in from open; one turned off opens.
    let pf = vp.filter;
    let (fc, res) = match v.filter {
        _ if !v.own_cut => (f.own_fc, f.own_res),
        Some(cur) if cur.kind == pf.kind && cur.poles == pf.poles => (pf.cutoff_hz, pf.resonance),
        Some(cur) if cur.kind == FilterKind::LowPass && pf.kind == FilterKind::Off => {
            (OPEN_FILTER.cutoff_hz, 0.0)
        }
        None if pf.kind == FilterKind::LowPass => {
            v.filter = Some(Filter {
                cutoff_hz: OPEN_FILTER.cutoff_hz,
                resonance: 0.0,
                ..pf
            });
            v.fenv = Some(Envelope::idle());
            v.filt.clear();
            v.filter_key = None;
            (pf.cutoff_hz, pf.resonance)
        }
        _ => (f.own_fc, f.own_res),
    };
    if fc != f.own_fc || res != f.own_res {
        (v.follow.own_fc, v.follow.own_res) = (fc, res);
        moved = true;
    }
    // A ladder's Drive at once (it keeps its level as it turns).
    if let Some(cur) = v.filter.as_mut() {
        if cur.kind == FilterKind::Ladder
            && pf.kind == FilterKind::Ladder
            && cur.drive_db != pf.drive_db
        {
            cur.drive_db = pf.drive_db;
            v.filter_key = None;
        }
    }

    // Tune: the layer's speed against the one it played at. A warped
    // layer's speed is the tempo's; a granular voice's pitch is its grains'.
    if !v.granular && v.warp_bpm == 0.0 && l.warp_bpm == 0.0 && l.step != v.layer_step {
        let ratio = l.step as f64 / v.layer_step.max(1) as f64;
        v.plain_step = ((v.plain_step as f64 * ratio).round() as i64).max(1);
        v.base_step = v.plain_step;
        v.layer_step = l.step;
        retime(v);
    }

    // A granular voice: the layer's grain settings, with what its hit
    // added (a keygroup's semitones, 16 Levels' position) kept.
    // Out of Stretch, it stops keeping hits.
    if l.hits.is_none() {
        v.hits = None;
    }
    if v.granular
        && l.mode == PlayMode::Granular
        && (l.grain != v.layer_grain || l.follow != v.grain_follow)
    {
        v.grain_follow = l.follow;
        let mut p = pitched_grain(l.grain, v.grain_hit, v.grain_follow);
        if v.grain_base.position != v.layer_grain.position {
            p.position = v.grain_base.position;
        }
        v.grain_base = p;
        v.layer_grain = l.grain;
        g.set_params(tuned_grain(
            pressed_grain(p, v.route, v.press),
            v.follow.cents,
            v.grain_follow,
        ));
        v.grain_cents = v.follow.cents;
    }

    if v.mode == Some(l.mode) && !v.granular && !v.reverse && !l.reverse {
        follow_loop(v, l);
    }
    if moved {
        retarget(v, live, program);
    }
}

/// A voice of a looping mode follows its layer's loop ([`follow_edit`]):
/// the new points, kept inside the region it plays; if it is now past
/// the loop, it jumps in, the step smoothed ([`JUMP_FRAMES`]). A loop
/// taken away lets it play on to its end; one added while it is held
/// starts looping.
pub(super) fn follow_loop(v: &mut Voice, l: &ViewLayer) {
    if !matches!(
        v.mode,
        Some(PlayMode::Loop | PlayMode::Toggle | PlayMode::Gate)
    ) {
        return;
    }
    let Some(lp) = l.sustain_loop.as_ref() else {
        if v.looping {
            v.looping = false;
            v.backwards = false;
        }
        return;
    };
    let (start, end) = (lp.start.max(v.start), lp.end.min(v.end));
    if end - start < 2 {
        return;
    }
    let looping = v.looping || v.held;
    let unchanged = (v.loop_start, v.loop_end, v.ping_pong) == (start, end, lp.ping_pong);
    v.loop_start = start;
    v.loop_end = end;
    v.ping_pong = lp.ping_pong;
    // Short of the new loop's start, it has not been round this one yet:
    // the frames before it are the recording's ([`Voice::fold`]).
    if v.pos >> 32 < start {
        v.wrapped = false;
    }
    // Its blend reads before the loop's start, never before the region's.
    v.crossfade = lp
        .crossfade
        .min((end - start) / 2)
        .min(start - v.start)
        .max(0);
    if !looping || (unchanged && v.looping) {
        return;
    }
    v.looping = true;
    let i = v.pos >> 32;
    let fraction = v.pos & (ONE - 1);
    let (pos, backwards) = if v.ping_pong {
        if !v.backwards && i >= end {
            (((end - 1) << 32) | fraction, true)
        } else if v.backwards && i < start {
            ((start << 32) | fraction, false)
        } else {
            return;
        }
    } else if i >= end {
        let into = (v.pos - (start << 32)) % ((end - start) << 32);
        ((start << 32) + into, false)
    } else {
        return;
    };
    // The read on each side of the jump, so the step fades out instead
    // of clicking (with what is left of an earlier jump's): before it, the
    // recording where the voice was, as it heard it; after it, round the
    // loop it has now gone into ([`Voice::fold`]).
    if let Some(data) = v.data.take() {
        let read = |v: &Voice, at: i64| match &data {
            Pcm::F32(d) => v.read2(d, at),
            Pcm::I16(d) => v.read2(d, at),
        };
        v.looping = false;
        v.refresh_fast();
        let (al, ar) = read(v, v.pos);
        v.looping = true;
        v.wrapped = true;
        v.refresh_fast();
        let (bl, br) = read(v, pos);
        v.data = Some(data);
        let left = f32::from(v.jump_left) / f32::from(JUMP_FRAMES);
        v.jump = [v.jump[0] * left + al - bl, v.jump[1] * left + ar - br];
        v.jump_left = JUMP_FRAMES;
        if !v.follow.moving {
            v.follow.count = 0;
        }
        v.follow.moving = true;
    }
    v.pos = pos;
    v.backwards = backwards;
    v.wrapped = true;
}
