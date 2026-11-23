//! What a hit, a key or a take starts and stops: which pad a note plays,
//! mono, legato and glide, releases, and starting a voice with everything it
//! plays from.

use super::*;

/// What a hit on a pad plays.
#[derive(Clone, Copy)]
pub(super) enum Target {
    /// The pad's own sound.
    Own(u8),
    /// A note, through the keygroups.
    Key(u8),
    /// Pad `pad` at 16 Levels step `k` of `by`.
    Level { pad: u8, by: Levels, k: u8 },
}

impl Target {
    /// Whether voice `v` was started by a hit of this pad.
    pub(super) fn played(&self, v: &Voice) -> bool {
        match *self {
            // The pad's own voices and those of the pads it links to.
            Target::Own(pad) => v.by == pad && v.note == NO_NOTE,
            Target::Key(note) => v.note == note,
            Target::Level { pad, k, .. } => v.pad == pad && v.note == LEVEL_NOTE + k,
        }
    }
}

/// The most keys a mono pad keeps held (sampler R5.5); a key past that
/// forgets the oldest.
pub(super) const MONO_KEYS: usize = 10;

/// A mono pad's keys held, oldest first, each with its velocity, so letting
/// go of the newest goes back to the one before (sampler R5.5).
#[derive(Clone, Copy)]
pub(super) struct HeldKeys {
    pub(super) n: u8,
    pub(super) keys: [(u8, u8); MONO_KEYS],
}

impl HeldKeys {
    pub(super) const NONE: HeldKeys = HeldKeys {
        n: 0,
        keys: [(0, 0); MONO_KEYS],
    };

    pub(super) fn held(&self) -> impl Iterator<Item = (u8, u8)> + '_ {
        self.keys[..usize::from(self.n)].iter().copied()
    }

    /// The newest key held and its velocity.
    pub(super) fn top(&self) -> Option<(u8, u8)> {
        self.held().last()
    }

    /// Key `note` is held: the newest now.
    pub(super) fn press(&mut self, note: u8, velocity: u8) {
        self.lift(note);
        if usize::from(self.n) == MONO_KEYS {
            self.keys.copy_within(1.., 0);
            self.n -= 1;
        }
        self.keys[usize::from(self.n)] = (note, velocity);
        self.n += 1;
    }

    /// Key `note` lets go; true if it was the newest.
    pub(super) fn lift(&mut self, note: u8) -> bool {
        let n = usize::from(self.n);
        let Some(i) = self.keys[..n].iter().position(|k| k.0 == note) else {
            return false;
        };
        self.keys.copy_within(i + 1..n, i);
        self.n -= 1;
        i + 1 == n
    }
}

/// What a hit changes in the voices it starts: a keygroup's semitones, or
/// a 16 Levels step.
#[derive(Clone, Copy, Default)]
pub(super) struct Vary {
    pub(super) semis: i32,
    pub(super) velocity: Option<u8>,
    /// From 0 to 1 of the region (a granular voice's grain position).
    pub(super) start: Option<f32>,
    pub(super) cutoff_hz: Option<f32>,
    pub(super) decay_ms: Option<f32>,
    /// An audition: these source frames once, forward, without fades or a
    /// loop, instead of the layer's region.
    pub(super) slice: Option<(i64, i64)>,
    /// The slice a sliced layer plays (a note's, on keys), in place of the
    /// pad's own and its live or locked [`PadField::Slice`].
    pub(super) cut: Option<usize>,
    /// The pad hit, when it is not the pad playing (a link).
    pub(super) by: Option<u8>,
    /// A mono pad's glide: the cents the voice starts from its pitch, and
    /// slides back from over the pad's glide time (sampler R5.5).
    pub(super) glide_cents: f32,
}

impl Vary {
    pub(super) fn level(by: Levels, k: u8) -> Vary {
        let k = usize::from(k);
        let mut v = Vary::default();
        match by {
            Levels::Velocity => v.velocity = Some(Levels::velocity(k)),
            Levels::Tune => v.semis = Levels::semitones(k),
            Levels::Cutoff => v.cutoff_hz = Some(Levels::cutoff_hz(k)),
            Levels::Decay => v.decay_ms = Some(Levels::decay_ms(k)),
            Levels::Start => v.start = Some(Levels::start(k)),
        }
        v
    }
}

/// A voice's note when its pad was hit, not played as a keygroup.
pub(super) const NO_NOTE: u8 = 255;

/// Cycle's turns count to this and start again: every set size from 1 to
/// 16 layers divides it, so each set keeps its order across the wrap.
pub(super) const TURNS: u32 = 720_720;

/// A 16 Levels voice's note: this plus the pad (0 to 15) that played it.
pub(super) const LEVEL_NOTE: u8 = 128;

/// Points playing voice `v` at the live settings (`live`, its pad's row)
/// and the program's offsets: it glides there from where it is. A voice
/// fading out (choked, stolen) is left alone.
/// Lets a held voice go as the next hit on a toggle or latched pad does:
/// a loop leaves its loop and plays its tail, grains stop, and anything
/// else releases.
pub(super) fn let_go(v: &mut Voice, g: &mut GrainVoice) {
    v.held = false;
    match v.mode {
        Some(PlayMode::Loop) => {
            v.looping = false;
            v.backwards = v.reverse;
        }
        Some(PlayMode::Granular) => {
            g.stop();
            if let Some(a) = &mut v.amp {
                a.release();
            }
        }
        _ => {
            if let Some(a) = &mut v.amp {
                a.release();
            }
        }
    }
}

impl Engine {
    /// What a hit on pad `pad` of the kit being played plays.
    pub(super) fn target(&self, pad: u8) -> Target {
        match self.views[self.at].pads_play {
            PadsPlay::Pads => Target::Own(pad),
            PadsPlay::Keys { from } => Target::Key(from.saturating_add(pad).min(127)),
            PadsPlay::Levels { pad: p, by } => Target::Level {
                pad: p,
                by,
                k: pad % 16,
            },
        }
    }

    /// Every keygroup holding `note` plays it.
    pub(super) fn key(&mut self, note: u8, velocity: u8) {
        if velocity == 0 {
            self.key_off(note);
            return;
        }
        for p in 0..self.views[self.at].pads.len() {
            let Some((vary, sliced)) = self.key_vary(p, note) else {
                continue;
            };
            if self.views[self.at].pads[p]
                .as_ref()
                .is_some_and(ViewPad::mono)
            {
                self.mono_on(p, note, velocity, vary, sliced);
            } else {
                self.note_on(p as u8, velocity, note, vary);
            }
        }
    }

    /// What key `note` changes in pad `p`'s voices, and whether the pad is
    /// sliced; None if its keys don't hold the note.
    pub(super) fn key_vary(&self, p: usize, note: u8) -> Option<(Vary, bool)> {
        let vp = self.views[self.at].pads.get(p)?.as_ref()?;
        let k = vp.keys.filter(|k| k.contains(note))?;
        // A sliced pad: one note a slice from its lowest, each at the
        // sound's own pitch.
        let sliced = vp.sliced();
        let vary = if sliced {
            Vary {
                cut: Some(usize::from(note - k.lo)),
                ..Vary::default()
            }
        } else {
            Vary {
                semis: i32::from(note) - i32::from(k.root),
                ..Vary::default()
            }
        };
        Some((vary, sliced))
    }

    /// Key `note` lets go. A mono pad that was playing it goes back to its
    /// newest key still held (sampler R5.5); the rest let go of it.
    pub(super) fn key_off(&mut self, note: u8) {
        let at = self.at;
        for p in 0..self.views[at].pads.len() {
            let mono = self.views[at].pads[p].as_ref().is_some_and(ViewPad::mono);
            let i = at * PADS + p;
            if !mono || !self.mono_keys[i].lift(note) {
                continue;
            }
            let Some((back, velocity)) = self.mono_keys[i].top() else {
                continue;
            };
            if let Some((vary, sliced)) = self.key_vary(p, back) {
                self.mono_play(p, back, velocity, vary, sliced, true);
            }
        }
        self.release(|v| v.note == note);
    }

    /// Key `note` on mono pad `p` (sampler R5.5): it is the newest key held,
    /// and the pad plays it.
    pub(super) fn mono_on(&mut self, p: usize, note: u8, velocity: u8, vary: Vary, sliced: bool) {
        let keys = &mut self.mono_keys[self.at * PADS + p];
        let over = keys.held().any(|(k, _)| k != note);
        keys.press(note, velocity);
        self.mono_play(p, note, velocity, vary, sliced, over);
    }

    /// Mono pad `p` plays `note` alone. `over`: another key is held, so
    /// with Legato the sounding note moves there, and with Glide on held
    /// keys only, it glides. Otherwise the pad's voices stop and the note
    /// starts, gliding from where the pad's pitch is with Glide.
    pub(super) fn mono_play(
        &mut self,
        p: usize,
        note: u8,
        velocity: u8,
        mut vary: Vary,
        sliced: bool,
        over: bool,
    ) {
        let at = self.at;
        let Some(m) = self.views[at].pads[p].as_ref().map(|vp| vp.mono) else {
            return;
        };
        let pad = p as u8;
        // A sliced pad plays each note's slice at its own pitch.
        let glide = m.glide_ms > 0.0 && !sliced && (over || !m.glide_held);
        if m.legato && over && !sliced && self.legato_to(p, note, vary, glide) {
            self.mono_at[at * PADS + p] = Some(f32::from(note));
            return;
        }
        let from = self.mono_pitch(pad).or(self.mono_at[at * PADS + p]);
        for (v, _) in self.pool.iter_mut() {
            if v.active && usize::from(v.kit) == at && v.pad == pad {
                Self::kill(v);
            }
        }
        if glide {
            if let Some(from) = from {
                vary.glide_cents = (from - f32::from(note)) * 100.0;
            }
        }
        self.note_on(pad, velocity, note, vary);
        self.mono_at[at * PADS + p] = Some(f32::from(note));
    }

    /// Where mono pad `pad`'s pitch is now, as a note (with what is left
    /// of a glide): a sounding, pitched voice of a key's.
    pub(super) fn mono_pitch(&self, pad: u8) -> Option<f32> {
        let at = self.at;
        self.pool.iter().find_map(|(v, _)| {
            (v.active
                && v.kill_left == 0
                && usize::from(v.kit) == at
                && v.pad == pad
                && v.note != NO_NOTE
                && v.motion.pitched)
                .then(|| f32::from(v.note) + v.motion.glide_cents / 100.0)
        })
    }

    /// Legato on mono pad `p`: its sounding voices of a key's move to
    /// `note` without starting again, gliding there with `glide`. A grain
    /// layer's cloud moves with its own glide; a warped layer keeps its
    /// pitch. False if none sounds.
    pub(super) fn legato_to(&mut self, p: usize, note: u8, vary: Vary, glide: bool) -> bool {
        let at = self.at;
        let pad = p as u8;
        let Some((layers, ms)) = self.views[at].pads[p]
            .as_ref()
            .map(|vp| (vp.layers.len(), vp.mono.glide_ms))
        else {
            return false;
        };
        let mut moved = false;
        for li in 0..layers {
            moved |= self.glide_held(pad, li, note, vary);
        }
        let rate = self.rate;
        for (v, _) in self.pool.iter_mut() {
            if !v.active
                || v.granular
                || v.kill_left > 0
                || usize::from(v.kit) != at
                || v.pad != pad
                || v.note == NO_NOTE
            {
                continue;
            }
            if v.motion.pitched {
                let from = (f32::from(v.note) - f32::from(note)) * 100.0 + v.motion.glide_cents;
                let octaves = (f64::from(note) - f64::from(v.note)) / 12.0;
                v.plain_step = ((v.plain_step as f64 * octaves.exp2()).round() as i64).max(1);
                v.base_step = v.plain_step;
                let m = &mut v.motion;
                let was = m.glide_cents;
                if glide {
                    m.glide(from, ms, rate);
                } else {
                    m.glide_cents = 0.0;
                }
                m.cents += m.glide_cents - was;
                retime(v);
            }
            v.note = note;
            moved = true;
        }
        moved
    }

    /// Lets go of the held voices `which` picks, of the kit being played.
    pub(super) fn release(&mut self, which: impl Fn(&Voice) -> bool) {
        let at = self.at;
        for (v, g) in self.pool.iter_mut() {
            // A latched voice waits for its pad's next hit.
            if !v.active || usize::from(v.kit) != at || !which(v) || !v.held || v.latch {
                continue;
            }
            match v.mode {
                Some(PlayMode::Gate) => {
                    v.held = false;
                    if let Some(a) = &mut v.amp {
                        a.release();
                    }
                }
                Some(PlayMode::Granular) => {
                    v.held = false;
                    g.stop();
                    if let Some(a) = &mut v.amp {
                        a.release();
                    }
                }
                Some(PlayMode::Loop) => {
                    v.held = false;
                    v.looping = false;
                    v.backwards = v.reverse;
                }
                _ => {} // one-shot ignores it; toggle waits for the next hit
            }
        }
    }

    /// [`Event::TakeOn`].
    pub(super) fn take_on(&mut self, velocity: u8) {
        let at = self.at;
        let Some(vp) = self.views[at].take.as_ref() else {
            return;
        };
        if velocity == 0 || vp.layers.is_empty() {
            return;
        }
        let velocity = self.fixed_velocity.unwrap_or(velocity.min(127));
        let vel = f32::from(velocity) / 127.0;
        let level = (1.0 - vp.vel_to_level) + vp.vel_to_level * vel * vel;
        let li = vp
            .layers
            .iter()
            .position(|l| (l.vel_lo..=l.vel_hi).contains(&velocity))
            .unwrap_or(0);
        self.stats.hits += 1;
        for (v, _) in self.pool.iter_mut() {
            if v.active && usize::from(v.kit) == at && v.pad == TAKE_PAD {
                Self::kill(v);
            }
        }
        self.start_voice(
            usize::from(TAKE_PAD),
            li,
            level,
            vel,
            NO_NOTE,
            Vary::default(),
        );
    }

    pub(super) fn audition(&mut self, pad: u8, layer: u8, start: u32, end: u32, velocity: u8) {
        // A new audition ends live chop (Player::live_chop turns it on
        // again after its own).
        self.chop_pad = None;
        if velocity == 0 || start >= end {
            return;
        }
        let velocity = self.fixed_velocity.unwrap_or(velocity.min(127));
        let p = usize::from(pad);
        let at = self.at;
        // A slice of the take the Sample page shows ([`TAKE_PAD`]) plays
        // from the take, on its own voice, as a pad's slice does.
        let vp = if pad == TAKE_PAD {
            self.views[at].take.as_ref()
        } else {
            self.views[at].pads.get(p).and_then(Option::as_ref)
        };
        let Some(vp) = vp else {
            return;
        };
        let li = usize::from(layer);
        if li >= vp.layers.len() {
            return;
        }
        self.stats.hits += 1;
        let vel = f32::from(velocity) / 127.0;
        let level = (1.0 - vp.vel_to_level) + vp.vel_to_level * vel * vel;
        for (v, _) in self.pool.iter_mut() {
            if v.active && usize::from(v.kit) == at && v.pad == pad {
                Self::kill(v);
            }
        }
        let vary = Vary {
            slice: Some((i64::from(start), i64::from(end))),
            ..Vary::default()
        };
        self.start_voice(p, li, level, vel, NO_NOTE, vary);
    }

    /// A hit on pad `pad` itself: it plays, and so does every pad it
    /// links to, each as a hit on it would (its layers, chokes and latch),
    /// its voices let go when `pad` is. Velocity 0 is a lift.
    pub(super) fn hit(&mut self, pad: u8, velocity: u8) {
        if velocity == 0 {
            self.release(|v| v.by == pad && v.note == NO_NOTE);
            return;
        }
        let mut links = [0u8; PADS];
        let mut n = 0;
        if let Some(Some(vp)) = self.views[self.at].pads.get(usize::from(pad)) {
            for &l in &vp.links {
                links[n] = l as u8;
                n += 1;
            }
        }
        self.spare_from = self.clock + 1;
        self.note_on(pad, velocity, NO_NOTE, Vary::default());
        for &l in &links[..n] {
            let by = Vary {
                by: Some(pad),
                ..Vary::default()
            };
            self.note_on(l, velocity, NO_NOTE, by);
        }
        self.spare_from = u64::MAX;
    }

    /// Pad `pad` plays: hit (`note` is [`NO_NOTE`]), as the keygroup for
    /// `note`, or at a 16 Levels step, with what `vary` changes.
    pub(super) fn note_on(&mut self, pad: u8, velocity: u8, note: u8, vary: Vary) {
        if velocity == 0 {
            self.release(|v| v.pad == pad && v.note == note);
            return;
        }
        // 16 Levels of velocity are the pads' own, whatever Full Level says.
        let velocity = vary
            .velocity
            .unwrap_or(self.fixed_velocity.unwrap_or(velocity.min(127)));
        let p = usize::from(pad);
        let at = self.at;
        let Some(Some(vp)) = self.views[at].pads.get(p) else {
            return;
        };
        self.stats.hits += 1;

        // A second hit on a held toggle or latched pad lets it go instead
        // of starting it again.
        let mut toggled_off = false;
        for (v, g) in self.pool.iter_mut() {
            if v.active
                && usize::from(v.kit) == at
                && v.pad == pad
                && v.note == note
                && v.held
                && (v.latch || v.mode == Some(PlayMode::Toggle))
            {
                let_go(v, g);
                toggled_off = true;
            }
        }
        if toggled_off {
            return;
        }

        // Mute groups and one-way chokes (not the voices this hit started
        // on the pads it links to).
        let group = vp.mute_group;
        for (v, _) in self.pool.iter_mut() {
            if !v.active || usize::from(v.kit) != at || v.pad == pad || v.age >= self.spare_from {
                continue;
            }
            let other = usize::from(v.pad);
            let same_group = group.is_some()
                && self.views[at]
                    .pads
                    .get(other)
                    .and_then(Option::as_ref)
                    .is_some_and(|o| o.mute_group == group);
            if same_group || vp.chokes.contains(&other) {
                Self::kill(v);
            }
        }

        // Which layers play.
        let count = vp.layers.len();
        let mut chosen = [false; crate::model::MAX_LAYERS];
        let matching =
            |i: usize| vp.layers[i].vel_lo <= velocity && velocity <= vp.layers[i].vel_hi;
        let n_match = (0..count).filter(|&i| matching(i)).count();
        if n_match == 0 {
            return;
        }
        // With a velocity fade, layers with the same velocities are a set:
        // Cycle and Random take turns within each set, and overlapping sets
        // play together, crossfaded.
        let sets = vp.vel_fade > 0 && vp.select != LayerSelect::All;
        match vp.select {
            LayerSelect::All => {
                for (i, c) in chosen.iter_mut().enumerate().take(count) {
                    *c = matching(i);
                }
            }
            LayerSelect::Cycle | LayerSelect::Random if sets => {
                let k = if vp.select == LayerSelect::Cycle {
                    let turn = &mut self.cycle[at * PADS + p];
                    let k = *turn % TURNS;
                    *turn = (k + 1) % TURNS;
                    k as usize
                } else {
                    self.rng.below(TURNS) as usize
                };
                let same = |i: usize, j: usize| {
                    let (a, b) = (&vp.layers[i], &vp.layers[j]);
                    matching(j) && a.vel_lo == b.vel_lo && a.vel_hi == b.vel_hi
                };
                for i in (0..count).filter(|&i| matching(i)) {
                    let before = (0..i).filter(|&j| same(i, j)).count();
                    let size = (0..count).filter(|&j| same(i, j)).count();
                    chosen[i] = before == k % size;
                }
            }
            LayerSelect::Cycle | LayerSelect::Random => {
                let k = if vp.select == LayerSelect::Cycle {
                    let turn = &mut self.cycle[at * PADS + p];
                    let k = *turn as usize % n_match;
                    *turn = ((k + 1) % n_match) as u32;
                    k
                } else {
                    self.rng.below(n_match as u32) as usize
                };
                if let Some(i) = (0..count).filter(|&i| matching(i)).nth(k) {
                    chosen[i] = true;
                }
            }
        }

        let vel = f32::from(velocity) / 127.0;
        let level = (1.0 - vp.vel_to_level) + vp.vel_to_level * vel * vel;
        // Key and velocity fades: a level for each layer, set at the start.
        let key_gain = match vp.keys {
            Some(k) if note != NO_NOTE && k.contains(note) => {
                edge_gain(note, k.lo, k.hi, vp.key_fade, 0, 127)
            }
            _ => 1.0,
        };
        let mut gains = [key_gain; crate::model::MAX_LAYERS];
        for (li, g) in gains.iter_mut().enumerate().take(count) {
            let l = &vp.layers[li];
            *g *= edge_gain(velocity, l.vel_lo, l.vel_hi, vp.vel_fade, 1, 127);
        }
        let polyphony = usize::from(vp.polyphony);
        // A mono pad's gliding granular layers: a key played while one
        // holds moves that cloud there, without a new start (legato).
        let mut legato = [false; crate::model::MAX_LAYERS];
        if polyphony == 1 && !vp.mono() {
            for (li, l) in vp.layers.iter().enumerate().take(count) {
                legato[li] = l.glide > 0 && l.mode == PlayMode::Granular;
            }
        }
        for li in 0..count {
            if !chosen[li] {
                continue;
            }
            if legato[li] && self.glide_held(pad, li, note, vary) {
                continue;
            }
            // Polyphony: push out this pad's oldest voices past the limit.
            loop {
                let mut n = 0;
                let mut oldest: Option<(usize, u64)> = None;
                for (i, (v, _)) in self.pool.iter().enumerate() {
                    if v.active && usize::from(v.kit) == at && v.pad == pad && v.kill_left == 0 {
                        n += 1;
                        if oldest.is_none_or(|(_, age)| age > v.age) {
                            oldest = Some((i, v.age));
                        }
                    }
                }
                if n < polyphony {
                    break;
                }
                match oldest.and_then(|(o, _)| self.pool.voice_mut(o)) {
                    Some(v) => Self::kill(v),
                    None => break,
                }
            }
            self.start_voice(p, li, level * gains[li], vel, note, vary);
        }
    }

    /// A held granular voice of layer `li` of pad `pad`, if one is
    /// playing, glides to `vary`'s key (a mono pad's next key: see
    /// [`Region::grain_glide_ms`]) and plays `note` from now on. False if
    /// none holds.
    ///
    /// [`Region::grain_glide_ms`]: crate::model::Region::grain_glide_ms
    pub(super) fn glide_held(&mut self, pad: u8, li: usize, note: u8, vary: Vary) -> bool {
        let p = usize::from(pad);
        let Some(glide) = self.views[self.at].pads[p]
            .as_ref()
            .and_then(|vp| vp.layers.get(li))
            .map(|l| l.glide)
        else {
            return false;
        };
        let at = self.at;
        for (v, _) in self.pool.iter_mut() {
            if v.active
                && v.granular
                && v.held
                && v.kill_left == 0
                && usize::from(v.kit) == at
                && v.pad == pad
                && usize::from(v.layer) == li
            {
                let now = if v.glide_left > 0 {
                    v.glide_to
                } else {
                    v.grain_hit
                };
                let to = now + (vary.semis - v.grain_key) as f32;
                v.glide_to = to;
                v.glide_step = (to - v.grain_hit) / glide.max(1) as f32;
                v.glide_left = glide;
                v.grain_key = vary.semis;
                v.note = note;
                v.by = vary.by.unwrap_or(pad);
                self.last_key[at * PADS + p] = Some(to);
                return true;
            }
        }
        false
    }

    /// A new voice's movement on pad `vp` (sampler R5 part 4): its LFO
    /// (from the start of its cycle, or where the pad's cycle is with
    /// Retrig off), its pitch envelope, and velocity `vel` (0 to 1) on its
    /// pitch and cutoff, under the pad's `live` row. `pitched`: its pitch
    /// may move. `glide`: cents it slides in from, over the pad's glide
    /// time (a mono pad's next note, sampler R5.5).
    pub(super) fn motion_for(
        &self,
        vp: &ViewPad,
        live: &[f32; PadField::COUNT],
        vel: f32,
        pitched: bool,
        glide: f32,
    ) -> Motion {
        let set = |f: PadField| Some(live[f as usize]).filter(|x| !x.is_nan());
        let rate = self.rate;
        let soft = 1.0 - vel.clamp(0.0, 1.0);
        let semis = set(PadField::PitchEnv).unwrap_or(vp.pitch_env.semitones);
        let decay_ms = set(PadField::PitchDecay).unwrap_or(vp.pitch_env.decay_ms);
        let decay = (decay_ms * 0.001 * rate).max(1.0);
        let mut m = Motion {
            step_s: FILTER_STEP as f32 / rate,
            env_cents: if pitched { 100.0 * semis } else { 0.0 },
            env_k: (-ENV_FALL * FILTER_STEP as f32 / decay).exp(),
            vel_cents: if pitched {
                -100.0 * vp.vel_to_pitch * soft
            } else {
                0.0
            },
            vel_fc: (-4.0 * vp.vel_to_cutoff * soft).exp2(),
            pitched,
            // Seeded from the hit count, so the same performance renders
            // the same.
            rng: Rng::new(0x4C46_4F00 ^ (self.clock as u32).wrapping_mul(0x9E37_79B9)),
            ..Motion::default()
        };
        m.held = [bipolar(&mut m.rng), bipolar(&mut m.rng)];
        m.set_lfo(
            &vp.lfo,
            set(PadField::LfoDepth).unwrap_or(vp.lfo.depth),
            self.tempo,
        );
        if !vp.lfo.retrig && m.to != LfoTarget::Off {
            m.phase = self.free_phase(&m);
        }
        if pitched && glide != 0.0 {
            m.glide(glide, vp.mono.glide_ms, rate);
        }
        m.cents = m.vel_cents + m.env_cents + m.glide_cents;
        m.fc = m.vel_fc;
        m.on |= m.env_cents != 0.0 || m.glide_cents != 0.0;
        m
    }

    /// Where an LFO that runs on between hits (Retrig off) is in its cycle
    /// now: on the transport's beats when synced and it plays, else on the
    /// engine's own clock, so every voice of the pad joins one cycle.
    pub(super) fn free_phase(&self, m: &Motion) -> f32 {
        let rate = f64::from(self.rate);
        let at = self.event_frame as f64;
        let cycles = match self.beat {
            Some((frame, beat)) if m.beats > 0.0 => {
                (beat + (at - frame as f64) * self.tempo / 60.0 / rate) / f64::from(m.beats)
            }
            _ => at * m.cycle_hz(self.tempo) / rate,
        };
        cycles.rem_euclid(1.0) as f32
    }

    /// Starts layer `li` of pad `p` with what `vary` changes. A slice, for
    /// an audition, plays those source frames instead of the layer's
    /// region: once, forward, without fades or a loop.
    pub(super) fn start_voice(
        &mut self,
        p: usize,
        li: usize,
        level: f32,
        vel: f32,
        note: u8,
        vary: Vary,
    ) {
        let semis = vary.semis;
        let at = self.at;
        // A pad's, or the take's on TAKE_PAD: none of the tables below
        // indexed by pad reaches it (they read it with `get`, or not at
        // all).
        let Some(vp) = self.views[at].pad_or_take(p) else {
            return;
        };
        let l = &vp.layers[li];
        // A slice is in source frames; a stretched copy has its own.
        let slice = vary.slice.map(|(a, b)| match &l.source_at {
            Some(map) => {
                let at = |x: i64| map.at(x as f64).round() as i64;
                (at(a), at(b))
            }
            None => (a, b),
        });
        let slice = match slice {
            Some((a, b)) => {
                let frames = (l.data.len() / if l.stereo { 2 } else { 1 }) as i64;
                let (a, b) = (a.max(0), b.min(frames));
                if a >= b {
                    return;
                }
                Some((a, b))
            }
            None => None,
        };
        // A sliced layer plays one slice (R4): the note's, else the pad's
        // live or locked Slice, else its own. Past its last, nothing.
        let cut = if slice.is_none() && !l.cuts.is_empty() {
            let set = self.lives[at].row(p)[PadField::Slice as usize];
            let k = vary.cut.unwrap_or(if set.is_nan() {
                usize::from(vp.slice)
            } else {
                set.round().max(0.0) as usize
            });
            let mut edges = std::iter::once(l.start)
                .chain(l.cuts.iter().copied())
                .chain(std::iter::once(l.end));
            match (edges.nth(k), edges.next()) {
                (Some(a), Some(b)) if a < b => Some((a, b, k)),
                _ => return,
            }
        } else {
            None
        };
        // The slice's own settings (R4 part 5).
        let own = cut
            .and_then(|(_, _, k)| l.slice_sets.get(k))
            .copied()
            .unwrap_or(ViewSlice::PLAIN);
        let level = level * own.gain;
        // What the voice reads: the layer's region, the slice of it (on to
        // the region's end, if the slice says so), or the audition's
        // frames once. A slice doesn't loop.
        let (start, end, reverse, mode) = match (slice, cut) {
            (Some((a, b)), _) => (a, b, false, PlayMode::OneShot),
            (None, Some((a, b, _))) => {
                let mode = match l.mode {
                    PlayMode::Loop => PlayMode::Gate,
                    m => m,
                };
                let b = if own.to_end { l.end } else { b };
                (a, b, l.reverse != own.reverse, mode)
            }
            (None, None) => (l.start, l.end, l.reverse, l.mode),
        };
        let granular = mode == PlayMode::Granular;
        let frames = end - start;
        let skip = if reverse || slice.is_some() {
            0
        } else {
            let from = vary.start.unwrap_or(vp.vel_to_start * (1.0 - vel));
            (from as f64 * frames as f64) as i64
        };
        let start_frame = if reverse {
            end - 1
        } else {
            (start + skip).min(end - 1)
        };
        let (pick_l, pick_r) = picks(l.stereo, l.pick);
        let lp = if slice.is_some() || cut.is_some() {
            None
        } else {
            l.sustain_loop.as_ref()
        };
        // A slice's fades are the region's, each at most half the slice.
        let half = if cut.is_some() {
            u32::try_from(frames / 2).unwrap_or(u32::MAX)
        } else {
            u32::MAX
        };
        let fit = |f: Fade| {
            Some(Fade {
                frames: f.frames.min(half),
                ..f
            })
            .filter(|f| f.frames > 0)
        };
        let (fade_in, fade_out) = if slice.is_some() {
            (None, None)
        } else {
            let out = match own.fade_out {
                Some(frames) => Fade {
                    frames,
                    ..l.fade_out
                },
                None => l.fade_out,
            };
            (fit(l.fade_in), fit(out))
        };
        // The pad's live settings and locks, where set, in place of its own.
        let kit_live = &self.lives[at];
        let live = kit_live.row(p);
        let program = kit_live.program;
        let set = |f: PadField| Some(live[f as usize]).filter(|x| !x.is_nan());
        let mut amp = vp.amp;
        if let Some(ms) = set(PadField::Attack) {
            amp.attack_ms = ms;
        }
        let decay = |rows: &[[f32; PadField::COUNT]; PADS]| {
            rows.get(p)
                .map_or(f32::NAN, |r| r[PadField::Decay as usize])
        };
        if decay(&kit_live.pads).is_nan() {
            // A locked decay decays to silence, as the Play page's Decay
            // knob shortens a sound.
            if let Some(ms) = Some(decay(&kit_live.locks)).filter(|x| !x.is_nan()) {
                (amp.decay_ms, amp.sustain) = (ms, 0.0);
            }
        } else if let Some(ms) = set(PadField::Decay) {
            amp.decay_ms = ms;
        }
        if let Some(ms) = set(PadField::Release) {
            amp.release_ms = ms;
        }
        if let Some(ms) = vary.decay_ms {
            (amp.decay_ms, amp.sustain) = (ms, 0.0);
        }
        // A slice's own cutoff in place of the pad's, unless the hit has
        // one (16 Levels by Cutoff).
        let cutoff = vary
            .cutoff_hz
            .or((own.cutoff_hz > 0.0).then_some(own.cutoff_hz));
        let filter = match cutoff {
            Some(hz) if vp.filter.kind == FilterKind::Off => Some(Filter {
                kind: FilterKind::LowPass,
                poles: 2,
                cutoff_hz: hz,
                resonance: 0.0,
                env_octaves: 0.0,
                drive_db: 0.0,
            }),
            Some(hz) => Some(Filter {
                cutoff_hz: hz,
                ..vp.filter
            }),
            None => (vp.filter.kind != FilterKind::Off).then_some(vp.filter),
        };
        let own_fc = filter.map_or(OPEN_FILTER.cutoff_hz, |f| f.cutoff_hz);
        let own_res = filter.map_or(0.0, |f| f.resonance);
        let filter = live_filter(
            filter,
            set(PadField::Cutoff),
            set(PadField::Resonance),
            program[ProgramField::Cutoff as usize],
        );
        // A live tune in place of the pad's own, and the program's tune
        // (which playing voices follow). A warped layer keeps its pitch.
        let warped = l.warp_bpm > 0.0;
        let (tune, cents) = if warped {
            (0.0, 0.0)
        } else {
            (
                set(PadField::Tune).map_or(0.0, |c| c - vp.tune_cents as f32),
                program[ProgramField::Tune as usize],
            )
        };
        // A slice's own pitch repitches it, warped or not.
        let tune = tune + own.cents;
        // A gliding layer's voice starts from the pad's last key.
        let hit = semis as f32 + tune / 100.0;
        let from = if granular && l.glide > 0 && p < PADS {
            self.last_key[at * PADS + p].filter(|k| *k != hit)
        } else {
            None
        };
        if granular && p < PADS {
            self.last_key[at * PADS + p] = Some(hit);
        }
        let mut grain = pitched_grain(l.grain, from.unwrap_or(hit), l.follow);
        if let Some(at) = vary.start {
            grain.position = (at * 16.0 / 15.0).min(1.0);
        }
        // A keygroup a few semitones from its root reads that much faster.
        let plain = if semis == 0 && tune == 0.0 {
            l.step
        } else {
            let octaves = f64::from(semis) / 12.0 + f64::from(tune) / 1200.0;
            ((l.step as f64 * octaves.exp2()).round() as i64).max(1)
        };
        // The gain: the view's, unless a live level or pan or the
        // program's level moves it.
        let unit_gain = l.unit_gain * level;
        let (live_db, live_pan) = (set(PadField::Level), set(PadField::Pan));
        let program_db = program[ProgramField::Level as usize];
        let (gain_l, gain_r) = if live_db.is_none() && live_pan.is_none() && program_db == 0.0 {
            (l.gain_l * level, l.gain_r * level)
        } else {
            live_gains(
                unit_gain,
                l.pan,
                live_db.unwrap_or(vp.level_db) + program_db,
                live_pan.unwrap_or(vp.pan),
            )
        };
        let follow = Follow {
            gain_to: [gain_l, gain_r],
            fc_to: filter.map_or(0.0, |f| f.cutoff_hz),
            res_to: filter.map_or(0.0, |f| f.resonance),
            cents,
            cents_to: cents,
            k: self.live_k,
            own_db: vp.level_db,
            own_pan: vp.pan,
            own_fc,
            own_res,
            ..Follow::default()
        };
        let step = if l.warp_bpm > 0.0 {
            warped_step(plain, self.tempo, l.warp_bpm)
        } else {
            plain
        };
        // The pad's LFO, pitch envelope and velocity on pitch and cutoff
        // (R5 part 4), with a live or locked depth and envelope. A warped
        // layer and a grain voice keep their pitch.
        let motion = self.motion_for(vp, &live, vel, !granular && !warped, vary.glide_cents);
        // A plain one-shot (as a kit's drum hits are) can take one of the
        // voices past the full-voice limit: it costs a fraction of a full
        // voice once it plays (`plain_now`, `render_plain`).
        let light = !motion.on
            && motion.cents == 0.0
            && !granular
            && mode == PlayMode::OneShot
            && step == ONE
            && !reverse
            && fade_in.is_none()
            && fade_out.is_none()
            && filter.is_none()
            && vp.pressure.target == PressureTarget::Off
            && vp.wow == 0.0
            && cents == 0.0
            && self.tape == 1.0
            && amp_holds(&amp, self.rate)
            && (!l.stereo || (pick_l, pick_r) == (0, 1));
        // A free voice, else the oldest releasing one, else the oldest of
        // the pad holding the most voices: a pad hit over and over (Note
        // Repeat on a long sound) gives up its own voices before another
        // pad's sound is cut short. A full voice past the full-voice limit
        // takes one of the full voices the same way.
        let full_now = if light {
            0
        } else {
            self.pool
                .iter()
                .filter(|(v, _)| v.active && !v.light)
                .count()
        };
        let slot = if !light && full_now >= self.full_limit {
            Self::victim(&self.pool, &mut self.held, true)
        } else {
            match self.pool.free_slot() {
                Some(slot) => slot,
                None => Self::victim(&self.pool, &mut self.held, false),
            }
        };
        let Some(old) = self.pool.voice(slot) else {
            return;
        };
        let (stolen, was_granular, last) = (old.active, old.granular, (old.last_l, old.last_r));
        if was_granular {
            self.pool.kill_grain(slot);
        }
        let (tail_l, tail_r) = if stolen {
            self.stats.steals += 1;
            last
        } else {
            (0.0, 0.0)
        };
        self.clock += 1;
        let Some(v) = self.pool.voice_mut(slot) else {
            return;
        };
        // The comb's line stays with the slot.
        let mut comb = std::mem::take(&mut v.filt.comb);
        comb.reset();
        *v = Voice {
            active: true,
            kit: at as u8,
            pad: p as u8,
            layer: li as u8,
            data: Some(l.data.clone()),
            stereo: l.stereo,
            pick_l,
            pick_r,
            pos: start_frame << 32,
            step,
            backwards: reverse,
            reverse,
            start,
            end,
            loop_start: lp.map_or(0, |x| x.start),
            loop_end: lp.map_or(0, |x| x.end),
            crossfade: lp.map_or(0, |x| x.crossfade),
            ping_pong: lp.is_some_and(|x| x.ping_pong),
            looping: !granular
                && lp.is_some()
                && !reverse
                && matches!(mode, PlayMode::Loop | PlayMode::Toggle | PlayMode::Gate),
            // Not round its loop yet; the render works out what reads
            // straight ([`Voice::refresh_fast`]).
            wrapped: false,
            fast_lo: 0,
            fast_hi: 0,
            fade_in,
            fade_out,
            gain_l,
            gain_r,
            mode: Some(mode),
            held: true,
            age: self.clock,
            amp: Some(Envelope::start(&amp, self.rate)),
            fenv: filter.map(|_| Envelope::start(&vp.filter_env, self.rate)),
            filter,
            filt: VoiceFilter {
                comb,
                ..VoiceFilter::default()
            },
            ringing: false,
            filter_count: 0,
            filter_key: None,
            kill_left: 0,
            tail_left: if stolen { STEAL_FRAMES } else { 0 },
            tail_l,
            tail_r,
            last_l: 0.0,
            last_r: 0.0,
            heard: gain_l.abs().max(gain_r.abs()),
            granular,
            src_rate: l.src_rate,
            grain_base: grain,
            grain_follow: l.follow,
            grain_hit: from.unwrap_or(hit),
            grain_key: semis,
            glide_to: hit,
            glide_step: from.map_or(0.0, |k| (hit - k) / l.glide as f32),
            glide_left: if from.is_some() { l.glide } else { 0 },
            route: vp.pressure,
            press_target: 0.0,
            press: 0.0,
            press_coef: 1.0 - (-1.0 / (PRESSURE_SMOOTH_S * self.rate)).exp(),
            base_step: step,
            press_gain: 1.0,
            press_gain_inc: 0.0,
            press_fc: 1.0,
            press_count: 0,
            warp_bpm: l.warp_bpm,
            plain_step: plain,
            note,
            by: vary.by.unwrap_or(p as u8),
            latch: vp.latch,
            audition: vary.slice.is_some(),
            speed: 1.0,
            base_gain_l: gain_l,
            base_gain_r: gain_r,
            wow: vp.wow,
            // Each voice wobbles from its own place, so a chord doesn't
            // wander in step.
            wow_phase: [
                (self.clock as f32 * 0.618_034).fract(),
                (self.clock as f32 * 0.414_214).fract(),
                (self.clock as f32 * 0.732_051).fract(),
            ],
            unit_gain,
            layer_pan: l.pan,
            tape_gain: 1.0,
            follow,
            grain_cents: cents,
            light,
            bl_above: BlAbove(if vp.vintage || granular {
                i64::MAX
            } else {
                CLEAN_ABOVE
            }),
            bl: false,
            // The copies if they are made by now, else none for its life:
            // what it reads doesn't hang on a thread's timing.
            octaves: if vp.vintage {
                None
            } else {
                l.octaves.as_ref().filter(|c| c.get().is_some()).cloned()
            },
            hits: l.hits.as_ref().filter(|c| c.get().is_some()).cloned(),
            grain_clean: !vp.vintage,
            vintage: vp.vintage,
            bl_step: 0,
            bl_octave: 0,
            bl_reach: Reach::default(),
            hit_gain: level,
            own_cut: vary.cutoff_hz.is_none(),
            layer_step: l.step,
            layer_grain: l.grain,
            jump: [0.0; 2],
            jump_left: 0,
            last_read: None,
            xf_from: Heard::Exact,
            xf_left: 0,
            motion,
        };
        if self.tape != 1.0 {
            // A hit on a stopped (or slowing) tape plays at its speed.
            v.speed = self.tape;
            let gain = (self.tape / TAPE_FADE).min(1.0);
            v.tape_gain = gain;
            (v.gain_l, v.gain_r) = (v.base_gain_l * gain, v.base_gain_r * gain);
            retime(v);
        } else if cents != 0.0 || v.motion.cents != 0.0 {
            retime(v);
        }
        let beat = self.frames_per_beat();
        if let Some(g) = self.pool.grain_mut(slot).filter(|_| granular) {
            g.set_tempo(beat);
            g.set_params(tuned_grain(grain, cents, l.follow));
            // Seeded from the hit count, so the same performance renders the same.
            g.reseed(0x6772_6169_6e00 ^ self.clock);
            g.start();
        }
        let n = self.active_voices();
        if n > self.stats.peak_voices {
            self.stats.peak_voices = n;
        }
    }
}
