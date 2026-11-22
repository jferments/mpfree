//! Grains: granular voices, the grain budget and its sharing by musical
//! priority ([`crate::grain_share`]), and what the screen draws of a cloud.

use super::*;

/// A layer's cloud as it plays now ([`Engine::cloud_of`]), for drawing it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CloudState {
    /// The spray its grains start in, 0 to 1 of the region: its setting as
    /// movement and pressure have moved it.
    pub spray: f32,
    /// More grains wanted at once than a voice keeps
    /// ([`mpfree_grain::MAX_GRAINS`]): the voice spaces them out to fit.
    pub thinned: bool,
}

/// Grains alive at once across every granular voice. Past it, new grains
/// are skipped. A first guess, to be set by measurement on the MPC: the
/// device bench's `--afford` prints the grains that fit in the sampler's
/// share beside a kit's sample voices ("grain budget: N grains", R7 step
/// A6).
pub const GRAIN_BUDGET: usize = 256;

/// Granular voices render their grains in chunks of this many frames.
pub(super) const GRAIN_CHUNK: usize = 256;

/// Which cloud a grain [`Claim`] is ([`Engine::share_grains`]).
#[derive(Clone, Copy, Debug)]
pub(super) enum Cloud {
    /// A voice slot's grain voice.
    Slot(usize),
    /// The master effect's cloud (0) or its repeat's (1).
    Master(usize),
}

/// The grain voice of `cloud`, if it is still there.
pub(super) fn cloud_mut<'a>(
    pool: &'a mut Voices,
    master: &'a mut Option<GrainFx>,
    cloud: Cloud,
) -> Option<&'a mut GrainVoice> {
    match cloud {
        Cloud::Slot(slot) => pool.grain_mut(slot),
        Cloud::Master(k) => master
            .as_mut()
            .and_then(|fx| fx.clouds_mut().into_iter().nth(k)),
    }
}

/// A granular voice: grains from the voice's region, then the same
/// envelope, gain, filter and fades as every other voice.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_granular(
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
    // Taken out of the voice for the render and put back after (a clone
    // would touch the shared count, which both cores' voices share).
    let Some(data) = v.data.take() else {
        v.active = false;
        return;
    };
    let ch = if v.stereo { 2 } else { 1 };
    let (start, end) = (v.start.max(0) as u64, v.end.max(0) as u64);
    // Clean pitching unless the pad is Vintage, from the sound's octave
    // copies when they were made by the time the voice started.
    let clean = v.grain_clean;
    let cell = v.octaves.take();
    let octaves = cell.as_deref().and_then(OctaveCell::get);
    // A Stretch's hits, kept sharp.
    let hit_cell = v.hits.take();
    let hits = hit_cell.as_deref().and_then(HitCell::get).unwrap_or(&[]);
    match &data {
        Pcm::F32(d) => match GrainClip::new(&d[..], ch, v.src_rate) {
            Some(c) => {
                let c = c.with_range(start, end).with_hits(hits);
                let c = if clean { c.clean(octaves) } else { c };
                grains(v, g, &c, budget, frame, rate, left, right, sl, sr)
            }
            None => v.active = false,
        },
        Pcm::I16(d) => match GrainClip::new(&d[..], ch, v.src_rate) {
            Some(c) => {
                let c = c.with_range(start, end).with_hits(hits);
                let c = if clean { c.clean(octaves) } else { c };
                grains(v, g, &c, budget, frame, rate, left, right, sl, sr)
            }
            None => v.active = false,
        },
    }
    v.octaves = cell;
    v.hits = hit_cell;
    if v.active {
        v.data = Some(data);
    }
}

/// [`render_granular`]'s grains from `clip`.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
pub(super) fn grains<C: GrainSource>(
    v: &mut Voice,
    g: &mut GrainVoice,
    clip: &C,
    budget: &mut Budget,
    frame: u64,
    rate: f32,
    left: &mut [f32],
    right: &mut [f32],
    sl: &mut [f32],
    sr: &mut [f32],
) {
    // A sound too short to read makes no grains, however long the pad is
    // held: the voice ends at once (silent: it never made a sound).
    let (start, end) = clip.span();
    if end - start < MIN_SPAN {
        v.active = false;
        return;
    }
    let mut at = 0;
    while at < left.len() && v.active {
        let n = (left.len() - at).min(sl.len());
        let (bl, br) = (&mut sl[..n], &mut sr[..n]);
        bl.fill(0.0);
        br.fill(0.0);
        let gliding = v.glide_left > 0;
        if gliding {
            // On to the next key, a part at a time.
            let k = (n as u32).min(v.glide_left);
            v.glide_left -= k;
            v.grain_hit = if v.glide_left == 0 {
                v.glide_to
            } else {
                v.grain_hit + v.glide_step * k as f32
            };
            let mut p = pitched_grain(v.layer_grain, v.grain_hit, v.grain_follow);
            p.position = v.grain_base.position;
            v.grain_base = p;
        }
        if gliding || v.route.target != PressureTarget::Off || v.grain_cents != v.follow.cents {
            let cents = v.follow.cents;
            g.set_params(tuned_grain(
                pressed_grain(v.grain_base, v.route, v.press),
                cents,
                v.grain_follow,
            ));
            v.grain_cents = cents;
        }
        g.render(clip, budget, frame + at as u64, bl, br);
        let follow = v.follow.moving;
        for k in 0..n {
            let (l, r) = if follow {
                finish::<true>(v, rate, bl[k], br[k], 1.0)
            } else {
                finish::<false>(v, rate, bl[k], br[k], 1.0)
            };
            left[at + k] += l;
            right[at + k] += r;
            if !v.active {
                break;
            }
        }
        at += n;
    }
}

/// Grain settings `cents` higher (the program's tune).
pub(super) fn tuned_grain(g: GrainParams, cents: f32, follow: KeyFollow) -> GrainParams {
    if cents != 0.0 {
        pitched_grain(g, cents / 100.0, follow)
    } else {
        g
    }
}

/// Grain settings `semis` higher: the grains' pitch (and with key
/// tracking their size, shorter, and the scan, faster, as much), or for a
/// Tone region the density, with the size shortened as much so each grain
/// keeps as many cycles of the note.
pub(super) fn pitched_grain(mut g: GrainParams, semis: f32, follow: KeyFollow) -> GrainParams {
    if !follow.tone {
        g.pitch += semis;
    }
    // Every voice's hit comes here, granular or not: the ratio only when
    // something follows it (k is exactly 1 at 0, so nothing changes then).
    if semis == 0.0 || !(follow.tone || follow.size || follow.scan) {
        return g;
    }
    let k = (semis / 12.0).exp2();
    if follow.tone {
        g.density *= k;
        g.size_ms /= k;
    } else if follow.size {
        g.size_ms /= k;
    }
    if follow.scan {
        g.scan *= k;
    }
    g
}

/// What a key moves on a granular voice ([`pitched_grain`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct KeyFollow {
    /// A Tone region: its density (the note), with its size shortened as
    /// much so each grain keeps its cycles; else the grains' pitch.
    pub(super) tone: bool,
    /// The grains' size too (key tracking, [`GrainKeys::size`]).
    pub(super) size: bool,
    /// The scan too ([`GrainKeys::scan`]).
    pub(super) scan: bool,
}

/// Grain settings with pressure `p` (0 to 1) applied along `route`.
pub(super) fn pressed_grain(mut g: GrainParams, route: PressureRoute, p: f32) -> GrainParams {
    let d = route.depth * p;
    match route.target {
        PressureTarget::Pitch => g.pitch += 12.0 * d,
        PressureTarget::GrainDensity => g.density *= (4.0 * d).exp2(),
        PressureTarget::GrainSize => g.size_ms *= (3.0 * d).exp2(),
        PressureTarget::GrainPosition => g.position = (g.position + d).rem_euclid(1.0),
        PressureTarget::GrainSpray => g.spray = (g.spray + d).clamp(0.0, 1.0),
        PressureTarget::Off | PressureTarget::Level | PressureTarget::Filter => {}
    }
    g
}

impl Engine {
    /// Grains playing now, across every granular voice.
    pub fn live_grains(&self) -> usize {
        self.pool.a.budget.live() + self.pool.b.as_ref().map_or(0, |b| b.budget.live())
    }

    /// The grains layer `layer` of pad `pad` plays now, across every voice
    /// playing it: each one's reading position (0 to 1 across the region),
    /// window level and direction, for drawing the cloud. Fills `out` from
    /// the start and returns how many it holds (grains past its end are
    /// left out). Never allocates. Kit 0's.
    pub fn grains_of(&self, pad: u8, layer: u8, out: &mut [GrainView]) -> usize {
        self.kit_grains_of(0, pad, layer, out)
    }

    /// [`Engine::grains_of`] for kit `kit`'s pad.
    pub fn kit_grains_of(&self, kit: usize, pad: u8, layer: u8, out: &mut [GrainView]) -> usize {
        let mut n = 0;
        for (v, g) in self.pool.iter() {
            if v.active
                && usize::from(v.kit) == kit
                && v.granular
                && v.pad == pad
                && v.layer == layer
                && n < out.len()
            {
                n += g.grains(&mut out[n..]);
            }
        }
        n
    }

    /// How layer `layer` of pad `pad` plays as grains now, beyond its
    /// grains ([`Engine::grains_of`]): the spray its first voice's grains
    /// start in (as its movement and pressure move it), and whether any of
    /// its voices is thinning its grains to fit (a cloud wanting more at
    /// once than a voice keeps). None while no voice plays it. Never
    /// allocates.
    pub fn cloud_of(&self, pad: u8, layer: u8) -> Option<CloudState> {
        self.kit_cloud_of(0, pad, layer)
    }

    /// [`Engine::cloud_of`] for kit `kit`'s pad.
    pub fn kit_cloud_of(&self, kit: usize, pad: u8, layer: u8) -> Option<CloudState> {
        let mut out: Option<CloudState> = None;
        for (v, g) in self.pool.iter() {
            if v.active
                && usize::from(v.kit) == kit
                && v.granular
                && v.pad == pad
                && v.layer == layer
            {
                let c = out.get_or_insert(CloudState {
                    spray: g.moved_params().spray,
                    thinned: false,
                });
                c.thinned |= g.stats().thinned;
            }
        }
        out
    }

    /// Shares the grain budget out by musical priority before a part of
    /// `frames` frames ([`crate::grain_share`]): among every granular voice
    /// here (the lane's share only while it is home; away, it keeps the
    /// limit it left with) and, with `master`, the master effect's clouds.
    /// Then, crowded, fades the oldest grains of the lowest-ranked clouds
    /// over their shares to keep [`grain_share::RESERVE`] grains free for
    /// new hits. While every cloud fits with room to spare, every share is
    /// none and the clouds play exactly as before. No allocation.
    pub(super) fn share_grains(&mut self, frames: usize, master: bool) {
        let rate = self.rate as u32;
        let new_hit = (grain_share::NEW_HIT_SECS * f64::from(rate)) as u64;
        let Engine {
            pool,
            master: fx,
            claims,
            clouds,
            stats,
            ..
        } = self;
        // No grain playing anywhere here: nothing to share, and a cloud
        // starting now starts unshared (`GrainVoice::start`). So a kit with
        // no grain pads pays only this.
        if pool.a.budget.live() == 0 && pool.b.as_ref().is_none_or(|b| b.budget.live() == 0) {
            return;
        }
        claims.clear();
        clouds.clear();
        for (slot, (v, g)) in pool.iter().enumerate() {
            if v.active && v.granular {
                let played = g.played();
                let rank = grain_share::rank_of(played, v.held, rate);
                // A new hit claims what it will have by the end of its
                // first moments, not all it will ever want.
                let wants = match rank {
                    Rank::NewHit => g.wants_within(new_hit.saturating_sub(played)),
                    _ => g.wants(),
                };
                claims.push(Claim::new(rank, played, wants, g.alive()));
                clouds.push(Cloud::Slot(slot));
            }
        }
        if let Some(fx) = fx.as_mut().filter(|_| master) {
            for (k, g) in fx.clouds_mut().into_iter().enumerate() {
                if g.wants() > 0 || g.alive() > 0 {
                    // It plays everything: it counts as the youngest held
                    // cloud, protected first and thinned last of them.
                    claims.push(Claim::new(Rank::Held, 0, g.wants(), g.alive()));
                    clouds.push(Cloud::Master(k));
                }
            }
        }
        if claims.is_empty() {
            return;
        }
        let (total, live) = match pool.b.as_ref() {
            Some(b) => (GRAIN_BUDGET, pool.a.budget.live() + b.budget.live()),
            None if pool.split => (
                GRAIN_BUDGET.saturating_sub(pool.b_limit),
                pool.a.budget.live(),
            ),
            None => (GRAIN_BUDGET, pool.a.budget.live()),
        };
        let shared = grain_share::share(claims, total);
        for (c, &cloud) in claims.iter().zip(clouds.iter()) {
            if let Some(g) = cloud_mut(pool, fx, cloud) {
                if c.share == 0 && c.wants > 0 && g.share() > 0 {
                    stats.held_back_clouds += 1;
                }
                g.set_share(c.share);
            }
        }
        if !shared.crowded {
            return;
        }
        stats.crowded_frames += frames as u64;
        // Grains ending within the fade (or this part, if longer) count as
        // room already, so a grain fading out is not counted twice.
        let fade = (grain_share::FADE_SECS * f64::from(rate)).max(2.0) as u32;
        let soon = fade.max(frames as u32);
        let ending: usize = clouds
            .iter()
            .filter_map(|&c| cloud_mut(pool, fx, c).map(|g| g.ending(soon)))
            .sum();
        let free = total.saturating_sub(live);
        let faded = grain_share::make_room(claims, free, ending, |i, n| {
            cloud_mut(pool, fx, clouds[i]).map_or(0, |g| g.fade_oldest(n, fade))
        });
        stats.faded_grains += faded as u64;
    }
}
