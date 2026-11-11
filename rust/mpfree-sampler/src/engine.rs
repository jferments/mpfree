//! The sampler's real-time engine: a fixed voice pool that plays the pads
//! of an [`EngineView`]. `render` and the event calls never allocate, lock,
//! log or touch files; the view is built off the audio thread with
//! [`EngineView::build`] and swapped in with [`Engine::set_view`].
//!
//! Each voice reads its region with 4-point Hermite interpolation at a
//! 32.32 fixed-point position, loops forward (with an equal-power
//! crossfade) or ping-pong, applies fades, an AHDSR amp envelope, an
//! optional 2- or 4-pole state-variable filter with its own envelope, and
//! equal-power pan. Stolen and choked voices fade out instead of clicking.

use std::sync::Arc;

use crate::dsp::{db_to_gain, equal_power, hermite, pan_gains, Ahdsr, Envelope, Filter, FilterKind, Rng, Svf};
use crate::model::{ChannelPick, Fade, LayerSelect, LoopMode, PlayMode, Project, PADS};

/// Fade length when a voice is choked, muted by its group, or pushed out by
/// the pad's polyphony limit.
pub const KILL_FRAMES: u16 = 64;
/// Fade length of the previous sound when its voice is stolen for a new hit.
pub const STEAL_FRAMES: u16 = 32;
/// Events that can wait for the next render.
pub const MAX_EVENTS: usize = 256;
/// Filter coefficients are recomputed every this many frames.
const FILTER_STEP: u32 = 16;
const ONE: i64 = 1 << 32;

#[derive(Clone, Debug)]
pub struct ViewLoop { start: i64, end: i64, crossfade: i64, ping_pong: bool }

/// A layer resolved into plain numbers and a handle to its audio.
#[derive(Clone, Debug)]
pub struct ViewLayer {
    data: Arc<[f32]>,
    stereo: bool,
    pick: ChannelPick,
    start: i64,
    end: i64,
    sustain_loop: Option<ViewLoop>,
    fade_in: Fade,
    fade_out: Fade,
    reverse: bool,
    mode: PlayMode,
    gain_l: f32,
    gain_r: f32,
    /// Source frames per output frame, 32.32 fixed point.
    step: i64,
    vel_lo: u8,
    vel_hi: u8,
}

#[derive(Clone, Debug)]
pub struct ViewPad {
    layers: Vec<ViewLayer>,
    select: LayerSelect,
    polyphony: u8,
    mute_group: Option<u8>,
    chokes: Vec<usize>,
    amp: Ahdsr,
    filter: Filter,
    filter_env: Ahdsr,
    vel_to_level: f32,
    vel_to_start: f32,
}

/// What the audio thread needs from the project, for every pad.
#[derive(Clone, Debug)]
pub struct EngineView {
    pub revision: u64,
    pads: Vec<Option<ViewPad>>,
}

impl EngineView {
    pub fn empty() -> EngineView { EngineView { revision: 0, pads: vec![None; PADS] } }

    /// Resolves every pad of a project for an engine running at `rate`.
    /// Call this off the audio thread; it allocates.
    pub fn build(project: &Project, rate: u32) -> EngineView {
        let mut pads = Vec::with_capacity(PADS);
        for i in 0..PADS {
            pads.push(project.pad(i).map(|p| {
                let pad_gain = db_to_gain(p.level_db);
                let layers = p.layers.iter().filter_map(|l| {
                    let s = project.source(l.region.source)?;
                    let r = &l.region;
                    let (pl, pr) = pan_gains(l.pan + p.pan);
                    let g = db_to_gain(r.gain_db) * db_to_gain(l.level_db) * pad_gain;
                    let cents = f64::from(r.tune_cents + l.tune_cents + p.tune_cents);
                    let ratio = f64::from(s.rate()) / f64::from(rate) * (cents / 1200.0).exp2();
                    Some(ViewLayer {
                        data: s.data().clone(),
                        stereo: s.channels() == 2,
                        pick: r.channels,
                        start: r.start as i64,
                        end: r.end as i64,
                        sustain_loop: r.sustain_loop.map(|lp| ViewLoop {
                            start: lp.start as i64, end: lp.end as i64,
                            crossfade: i64::from(lp.crossfade), ping_pong: lp.mode == LoopMode::PingPong,
                        }),
                        fade_in: r.fade_in,
                        fade_out: r.fade_out,
                        reverse: r.reverse,
                        mode: r.mode,
                        gain_l: g * pl,
                        gain_r: g * pr,
                        step: ((ratio * ONE as f64).round() as i64).max(1),
                        vel_lo: l.vel_lo,
                        vel_hi: l.vel_hi,
                    })
                }).collect();
                ViewPad {
                    layers, select: p.select, polyphony: p.polyphony, mute_group: p.mute_group,
                    chokes: p.chokes.clone(), amp: p.amp, filter: p.filter, filter_env: p.filter_env,
                    vel_to_level: p.vel_to_level.clamp(0.0, 1.0), vel_to_start: p.vel_to_start.clamp(0.0, 1.0),
                }
            }));
        }
        EngineView { revision: project.revision(), pads }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    NoteOn { pad: u8, velocity: u8 },
    NoteOff { pad: u8 },
    /// Releases every voice (gate-style), e.g. on STOP.
    AllOff,
}

#[derive(Clone, Debug, Default)]
struct Voice {
    active: bool,
    pad: u8,
    layer: u8,
    data: Option<Arc<[f32]>>,
    stereo: bool,
    pick_l: u8,
    pick_r: u8,
    pos: i64,
    step: i64,
    backwards: bool,
    reverse: bool,
    start: i64,
    end: i64,
    loop_start: i64,
    loop_end: i64,
    crossfade: i64,
    ping_pong: bool,
    looping: bool,
    fade_in: Option<Fade>,
    fade_out: Option<Fade>,
    gain_l: f32,
    gain_r: f32,
    mode: Option<PlayMode>,
    held: bool,
    age: u64,
    amp: Option<Envelope>,
    fenv: Option<Envelope>,
    filter: Option<Filter>,
    svf: [Svf; 2],
    filter_count: u32,
    kill_left: u16,
    tail_left: u16,
    tail_l: f32,
    tail_r: f32,
    last_l: f32,
    last_r: f32,
}

impl Voice {
    #[inline]
    fn at(&self, data: &[f32], i: i64, ch: u8) -> f32 {
        let i = i.clamp(self.start, self.end - 1) as usize;
        if self.stereo { data[2 * i + usize::from(ch)] } else { data[i] }
    }

    /// One channel at a fixed-point position, Hermite-interpolated.
    #[inline]
    fn read(&self, data: &[f32], pos: i64, pick: u8) -> f32 {
        let i = pos >> 32;
        let t = (pos & (ONE - 1)) as f32 / ONE as f32;
        let ch = |k: i64| -> f32 {
            match pick {
                0 => self.at(data, k, 0),
                1 => self.at(data, k, 1),
                _ => 0.5 * (self.at(data, k, 0) + self.at(data, k, 1)),
            }
        };
        if t == 0.0 { return ch(i); }
        hermite(ch(i - 1), ch(i), ch(i + 1), ch(i + 2), t)
    }
}

fn picks(stereo: bool, pick: ChannelPick) -> (u8, u8) {
    if !stereo { return (0, 0); }
    match pick {
        ChannelPick::Stereo => (0, 1),
        ChannelPick::MonoSum => (2, 2),
        ChannelPick::Left => (0, 0),
        ChannelPick::Right => (1, 1),
    }
}

/// Counters for the debug page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub hits: u64,
    pub steals: u64,
    pub dropped_events: u64,
    pub peak_voices: usize,
}

pub struct Engine {
    rate: f32,
    view: EngineView,
    voices: Vec<Voice>,
    events: Vec<(u32, Event)>,
    cycle: Vec<u8>,
    rng: Rng,
    clock: u64,
    fixed_velocity: Option<u8>,
    stats: Stats,
}

impl Engine {
    /// `voices` is the size of the fixed pool. All memory is reserved here.
    pub fn new(rate: u32, voices: usize) -> Engine {
        Engine {
            rate: rate as f32,
            view: EngineView::empty(),
            voices: vec![Voice::default(); voices.max(1)],
            events: Vec::with_capacity(MAX_EVENTS),
            cycle: vec![0; PADS],
            rng: Rng::new(0x5A4D_5052),
            clock: 0,
            fixed_velocity: None,
            stats: Stats::default(),
        }
    }

    /// Swaps in a new view at once and hands back the old one, so the caller
    /// frees it off the audio thread. Playing voices keep their own handle to
    /// their audio and finish normally.
    pub fn set_view(&mut self, view: EngineView) -> EngineView { std::mem::replace(&mut self.view, view) }
    pub fn view_revision(&self) -> u64 { self.view.revision }

    /// Full Level (Some(127)), Half Level (Some(64)), or off (None).
    pub fn set_fixed_velocity(&mut self, v: Option<u8>) { self.fixed_velocity = v.map(|v| v.clamp(1, 127)); }

    pub fn stats(&self) -> Stats { self.stats }
    pub fn active_voices(&self) -> usize { self.voices.iter().filter(|v| v.active).count() }
    /// (pad, layer, source frame) of every playing voice, for playheads on screen.
    pub fn playheads(&self) -> impl Iterator<Item = (u8, u8, u64)> + '_ {
        self.voices.iter().filter(|v| v.active).map(|v| (v.pad, v.layer, (v.pos >> 32).max(0) as u64))
    }
    pub fn pad_voices(&self, pad: u8) -> usize { self.voices.iter().filter(|v| v.active && v.pad == pad && v.kill_left == 0).count() }

    /// Queues an event `offset` frames into the next render (later renders if
    /// the offset is past its end). Events at the same offset keep their order.
    /// Returns false, and counts it, if the queue is full.
    pub fn schedule(&mut self, offset: u32, ev: Event) -> bool {
        if self.events.len() >= MAX_EVENTS { self.stats.dropped_events += 1; return false; }
        let i = self.events.partition_point(|(o, _)| *o <= offset);
        self.events.insert(i, (offset, ev));
        true
    }

    /// Applies an event now (before the next rendered frame).
    pub fn apply(&mut self, ev: Event) {
        match ev {
            Event::NoteOn { pad, velocity } => self.note_on(pad, velocity),
            Event::NoteOff { pad } => self.note_off(pad),
            Event::AllOff => for v in &mut self.voices {
                if v.active { v.held = false; v.looping = false; if let Some(a) = &mut v.amp { a.release(); } }
            },
        }
    }

    fn kill(v: &mut Voice) { if v.active && v.kill_left == 0 { v.kill_left = KILL_FRAMES; } }

    fn note_off(&mut self, pad: u8) {
        for v in &mut self.voices {
            if !v.active || v.pad != pad || !v.held { continue; }
            match v.mode {
                Some(PlayMode::Gate) => { v.held = false; if let Some(a) = &mut v.amp { a.release(); } }
                Some(PlayMode::Loop) => { v.held = false; v.looping = false; v.backwards = v.reverse; }
                _ => {} // one-shot ignores it; toggle waits for the next hit
            }
        }
    }

    fn note_on(&mut self, pad: u8, velocity: u8) {
        if velocity == 0 { self.note_off(pad); return; }
        let velocity = self.fixed_velocity.unwrap_or(velocity.min(127));
        let p = usize::from(pad);
        let Some(Some(vp)) = self.view.pads.get(p) else { return };
        self.stats.hits += 1;

        // A second hit on a held toggle pad stops it instead of starting it.
        let mut toggled_off = false;
        for v in &mut self.voices {
            if v.active && v.pad == pad && v.held && v.mode == Some(PlayMode::Toggle) {
                v.held = false;
                if let Some(a) = &mut v.amp { a.release(); }
                toggled_off = true;
            }
        }
        if toggled_off { return; }

        // Mute groups and one-way chokes.
        let group = vp.mute_group;
        for v in &mut self.voices {
            if !v.active || v.pad == pad { continue; }
            let other = usize::from(v.pad);
            let same_group = group.is_some() && self.view.pads[other].as_ref().is_some_and(|o| o.mute_group == group);
            if same_group || vp.chokes.contains(&other) { Self::kill(v); }
        }

        // Which layers play.
        let count = vp.layers.len();
        let mut chosen = [false; crate::model::MAX_LAYERS];
        let matching = |i: usize| vp.layers[i].vel_lo <= velocity && velocity <= vp.layers[i].vel_hi;
        let n_match = (0..count).filter(|&i| matching(i)).count();
        if n_match == 0 { return; }
        match vp.select {
            LayerSelect::All => for (i, c) in chosen.iter_mut().enumerate().take(count) { *c = matching(i); },
            LayerSelect::Cycle | LayerSelect::Random => {
                let k = if vp.select == LayerSelect::Cycle {
                    let k = usize::from(self.cycle[p]) % n_match;
                    self.cycle[p] = ((k + 1) % n_match) as u8;
                    k
                } else { self.rng.below(n_match as u32) as usize };
                if let Some(i) = (0..count).filter(|&i| matching(i)).nth(k) { chosen[i] = true; }
            }
        }

        let vel = f32::from(velocity) / 127.0;
        let level = (1.0 - vp.vel_to_level) + vp.vel_to_level * vel * vel;
        let polyphony = usize::from(vp.polyphony);
        for li in 0..count {
            if !chosen[li] { continue; }
            // Polyphony: push out this pad's oldest voices past the limit.
            loop {
                let mut n = 0;
                let mut oldest: Option<usize> = None;
                for (i, v) in self.voices.iter().enumerate() {
                    if v.active && v.pad == pad && v.kill_left == 0 {
                        n += 1;
                        if oldest.is_none_or(|o| self.voices[o].age > v.age) { oldest = Some(i); }
                    }
                }
                if n < polyphony { break; }
                match oldest { Some(o) => Self::kill(&mut self.voices[o]), None => break }
            }
            self.start_voice(p, li, level, vel);
        }
    }

    fn start_voice(&mut self, p: usize, li: usize, level: f32, vel: f32) {
        let Some(vp) = self.view.pads[p].as_ref() else { return };
        let l = &vp.layers[li];
        // A free voice, else the oldest releasing one, else the oldest.
        let slot = self.voices.iter().position(|v| !v.active).unwrap_or_else(|| {
            let mut best = 0;
            for (i, v) in self.voices.iter().enumerate() {
                let b = &self.voices[best];
                let rel = |x: &Voice| x.kill_left > 0 || x.amp.as_ref().is_some_and(|a| a.is_releasing());
                if (rel(v), std::cmp::Reverse(v.age)) > (rel(b), std::cmp::Reverse(b.age)) { best = i; }
            }
            best
        });
        let stolen = self.voices[slot].active;
        let (tail_l, tail_r) = if stolen { self.stats.steals += 1; (self.voices[slot].last_l, self.voices[slot].last_r) } else { (0.0, 0.0) };
        let frames = l.end - l.start;
        let skip = if l.reverse { 0 } else { ((vp.vel_to_start * (1.0 - vel)) as f64 * frames as f64) as i64 };
        let start_frame = if l.reverse { l.end - 1 } else { (l.start + skip).min(l.end - 1) };
        let (pick_l, pick_r) = picks(l.stereo, l.pick);
        let lp = l.sustain_loop.as_ref();
        self.clock += 1;
        let v = &mut self.voices[slot];
        *v = Voice {
            active: true,
            pad: p as u8,
            layer: li as u8,
            data: Some(l.data.clone()),
            stereo: l.stereo,
            pick_l, pick_r,
            pos: start_frame << 32,
            step: l.step,
            backwards: l.reverse,
            reverse: l.reverse,
            start: l.start,
            end: l.end,
            loop_start: lp.map_or(0, |x| x.start),
            loop_end: lp.map_or(0, |x| x.end),
            crossfade: lp.map_or(0, |x| x.crossfade),
            ping_pong: lp.is_some_and(|x| x.ping_pong),
            looping: lp.is_some() && !l.reverse && matches!(l.mode, PlayMode::Loop | PlayMode::Toggle | PlayMode::Gate),
            fade_in: (l.fade_in.frames > 0).then_some(l.fade_in),
            fade_out: (l.fade_out.frames > 0).then_some(l.fade_out),
            gain_l: l.gain_l * level,
            gain_r: l.gain_r * level,
            mode: Some(l.mode),
            held: true,
            age: self.clock,
            amp: Some(Envelope::start(&vp.amp, self.rate)),
            fenv: (vp.filter.kind != FilterKind::Off).then(|| Envelope::start(&vp.filter_env, self.rate)),
            filter: (vp.filter.kind != FilterKind::Off).then_some(vp.filter),
            svf: [Svf::default(); 2],
            filter_count: 0,
            kill_left: 0,
            tail_left: if stolen { STEAL_FRAMES } else { 0 },
            tail_l, tail_r,
            last_l: 0.0, last_r: 0.0,
        };
        let n = self.active_voices();
        if n > self.stats.peak_voices { self.stats.peak_voices = n; }
    }

    /// Renders `left.len()` frames, replacing what is in the buffers.
    /// Panics if the two buffers differ in length (a programming error).
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        assert_eq!(left.len(), right.len(), "left and right must be the same length");
        let n = left.len() as u32;
        left.fill(0.0);
        right.fill(0.0);
        let mut at = 0u32;
        while at < n {
            // Apply every event due now.
            while let Some(&(o, ev)) = self.events.first() {
                if o > at { break; }
                self.events.remove(0);
                self.apply(ev);
            }
            let next = self.events.first().map_or(n, |&(o, _)| o.min(n));
            let (a, b) = (at as usize, next as usize);
            for v in self.voices.iter_mut() {
                if v.active { render_voice(v, self.rate, &mut left[a..b], &mut right[a..b]); }
            }
            at = next;
        }
        for e in &mut self.events { e.0 = e.0.saturating_sub(n); }
    }
}

fn render_voice(v: &mut Voice, rate: f32, left: &mut [f32], right: &mut [f32]) {
    let Some(data) = v.data.clone() else { v.active = false; return };
    for (ol, or) in left.iter_mut().zip(right.iter_mut()) {
        let i = v.pos >> 32;
        if i < v.start || i >= v.end { v.active = false; break; }

        let mut l = v.read(&data, v.pos, v.pick_l);
        let mut r = v.read(&data, v.pos, v.pick_r);

        // Forward-loop crossfade: blend in the audio before the loop start.
        if v.looping && !v.ping_pong && v.crossfade > 0 && i >= v.loop_end - v.crossfade {
            let x = (i - (v.loop_end - v.crossfade)) as f32 / v.crossfade as f32;
            let (out, inn) = equal_power(x);
            let back = v.pos - ((v.loop_end - v.loop_start) << 32);
            l = l * out + v.read(&data, back, v.pick_l) * inn;
            r = r * out + v.read(&data, back, v.pick_r) * inn;
        }

        let mut g = 1.0f32;
        if let Some(f) = v.fade_in {
            let d = if v.reverse { v.end - 1 - i } else { i - v.start };
            if d < i64::from(f.frames) { g *= f.shape(d as f32 / f.frames as f32); }
        }
        if let Some(f) = v.fade_out {
            let d = if v.reverse { i - v.start } else { v.end - 1 - i };
            if d < i64::from(f.frames) { g *= f.shape(d as f32 / f.frames as f32); }
        }
        if let Some(a) = &mut v.amp { g *= a.next(); }
        if v.kill_left > 0 { g *= f32::from(v.kill_left) / f32::from(KILL_FRAMES); }

        l *= g * v.gain_l;
        r *= g * v.gain_r;

        if let (Some(f), Some(fe)) = (v.filter, v.fenv.as_mut()) {
            let e = fe.next();
            if v.filter_count == 0 {
                let fc = f.cutoff_hz * (f.env_octaves * e).exp2();
                v.svf[0].set(fc, f.resonance, rate);
                v.svf[1].set(fc, f.resonance * 0.5, rate);
            }
            v.filter_count = (v.filter_count + 1) % FILTER_STEP;
            l = v.svf[0].process(0, l, f.kind);
            r = v.svf[0].process(1, r, f.kind);
            if f.poles >= 4 {
                l = v.svf[1].process(0, l, f.kind);
                r = v.svf[1].process(1, r, f.kind);
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
        *ol += l;
        *or += r;

        // Advance.
        if v.backwards { v.pos -= v.step; } else { v.pos += v.step; }
        let j = v.pos >> 32;
        if v.looping {
            if v.ping_pong {
                if !v.backwards && j >= v.loop_end { v.backwards = true; v.pos = ((v.loop_end - 1) << 32) - (v.pos - (v.loop_end << 32)); }
                else if v.backwards && j < v.loop_start { v.backwards = false; v.pos = (v.loop_start << 32) + ((v.loop_start << 32) - v.pos); }
            } else if j >= v.loop_end {
                v.pos -= (v.loop_end - v.loop_start) << 32;
            }
        }

        if v.kill_left > 0 { v.kill_left -= 1; if v.kill_left == 0 { v.active = false; } }
        if v.amp.as_ref().is_some_and(|a| a.is_done()) { v.active = false; }
        if !v.active { break; }
    }
    if !v.active { v.data = None; }
}
