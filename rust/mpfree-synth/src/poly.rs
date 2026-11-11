//! Poly: a subtractive synth. Two band-limited oscillators per voice with
//! up to four unison stacks, a sub oscillator and noise, a drive stage into
//! a ladder or state-variable filter, two ADSR envelopes, a tempo-syncable
//! LFO, poly, mono and legato modes with glide.
//!
//! Timing: amp envelopes, oscillators and filters run every frame. Pitch,
//! cutoff, the LFO, the filter envelope and parameter smoothing run at a
//! control rate of one step every [`CTRL`] frames, aligned to the absolute
//! frame, so the output does not depend on how the host cuts blocks.

use mpfree_node::{Ctx, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed};

use crate::dsp::*;
use crate::params::*;

/// Frames per control step.
pub const CTRL: u64 = 16;
/// The most notes sounding at once (the Voices parameter's top).
pub const MAX_POLY: usize = 16;
/// Voice slots: the notes plus room for stolen voices to fade out.
pub const POOL: usize = MAX_POLY + 4;
const UNISON_MAX: usize = 4;
/// A stolen voice fades out over this long.
const KILL_MS: f32 = 3.0;
/// Peak of one full-velocity note at the defaults, about -12 dBFS.
const VOICE_GAIN: f32 = 0.16;

static INFO: NodeInfo = NodeInfo { id: "mpfree.poly", name: "Poly", kind: Kind::Instrument };

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Free,
    /// The key is down (or held by the mono note stack).
    On,
    Release,
    /// Stolen: fading out fast, then free.
    Kill,
}

#[derive(Clone, Copy, Debug)]
struct Voice {
    state: State,
    note: u8,
    vel: f32,
    age: u64,
    /// Current and target pitch in (fractional) notes, for glide.
    pitch: f32,
    target: f32,
    ph1: [f32; UNISON_MAX],
    ph2: [f32; UNISON_MAX],
    ph_sub: f32,
    /// Per-stack phase increments and pan gains, set each control step.
    dt1: [f32; UNISON_MAX],
    dt2: [f32; UNISON_MAX],
    dt_sub: f32,
    pan_l: [f32; UNISON_MAX],
    pan_r: [f32; UNISON_MAX],
    pw1: f32,
    pw2: f32,
    amp: Env,
    fenv: Env,
    coefs: FilterCoefs,
    filt: [Filter; 2],
    /// Gain from velocity, the LFO and pressure, set each control step.
    gain: f32,
    press: Smooth,
    kill_gain: f32,
    kill_step: f32,
}

impl Voice {
    fn new() -> Voice {
        Voice {
            state: State::Free,
            note: 0,
            vel: 0.0,
            age: 0,
            pitch: 60.0,
            target: 60.0,
            ph1: [0.0; UNISON_MAX],
            ph2: [0.0; UNISON_MAX],
            ph_sub: 0.0,
            dt1: [0.0; UNISON_MAX],
            dt2: [0.0; UNISON_MAX],
            dt_sub: 0.0,
            pan_l: [1.0; UNISON_MAX],
            pan_r: [1.0; UNISON_MAX],
            pw1: 0.5,
            pw2: 0.5,
            amp: Env::default(),
            fenv: Env::default(),
            coefs: FilterCoefs::default(),
            filt: [Filter::default(); 2],
            gain: 0.0,
            press: Smooth::new(0.0),
            kill_gain: 1.0,
            kill_step: 0.0,
        }
    }

    fn sounding(&self) -> bool { matches!(self.state, State::On | State::Release) }
}

/// Smoothed parameters, stepped once per control step.
#[derive(Clone, Copy, Debug)]
struct Smoothed {
    lvl1: Smooth,
    lvl2: Smooth,
    sub: Smooth,
    noise: Smooth,
    pw1: Smooth,
    pw2: Smooth,
    /// Cutoff in octaves (log2 Hz).
    cutoff: Smooth,
    res: Smooth,
    drive: Smooth,
    out_l: Smooth,
    out_r: Smooth,
    bend: Smooth,
}

/// Control values shared by every voice for one control step.
#[derive(Clone, Copy, Debug, Default)]
struct Shared {
    lfo: f32,
    mode: u8,
    lvl1: f32,
    lvl2: f32,
    sub: f32,
    noise: f32,
    drive_gain: f32,
    drive_amt: f32,
    out_l: f32,
    out_r: f32,
    wave1: u8,
    wave2: u8,
    unison: usize,
}

pub struct Poly {
    rate: u32,
    values: [f32; ID_SPACE],
    voices: [Voice; POOL],
    sm: Smoothed,
    sh: Shared,
    rng: Rng,
    lfo_phase: f64,
    lfo_held: f32,
    /// Notes held, oldest first, for mono and legato.
    stack: [u8; MAX_POLY],
    stack_len: usize,
    clock: u64,
    last_pitch: f32,
    /// Absolute frame of the next frame to process, for control steps.
    frame: u64,
    meter_voices: f32,
    /// Notes that did not fit anywhere (should stay 0).
    pub dropped: u64,
}

impl Default for Poly {
    fn default() -> Self { Poly::new(48_000) }
}

impl Poly {
    pub fn new(rate: u32) -> Poly {
        let mut values = [0.0; ID_SPACE];
        for p in PARAMS { values[p.id as usize] = p.default; }
        let v = |id: u16| values[id as usize];
        let sm = Smoothed {
            lvl1: Smooth::new(v(OSC1_LEVEL) / 100.0),
            lvl2: Smooth::new(v(OSC2_LEVEL) / 100.0),
            sub: Smooth::new(v(SUB_LEVEL) / 100.0),
            noise: Smooth::new(v(NOISE_LEVEL) / 100.0),
            pw1: Smooth::new(v(OSC1_PW) / 100.0),
            pw2: Smooth::new(v(OSC2_PW) / 100.0),
            cutoff: Smooth::new(v(CUTOFF).log2()),
            res: Smooth::new(v(RESONANCE) / 100.0),
            drive: Smooth::new(v(DRIVE)),
            out_l: Smooth::new(1.0),
            out_r: Smooth::new(1.0),
            bend: Smooth::new(0.0),
        };
        let mut s = Poly {
            rate,
            values,
            voices: [Voice::new(); POOL],
            sm,
            sh: Shared::default(),
            rng: Rng::new(0x005E_ED0F_901F),
            lfo_phase: 0.0,
            lfo_held: 0.0,
            stack: [0; MAX_POLY],
            stack_len: 0,
            clock: 0,
            last_pitch: 60.0,
            frame: 0,
            meter_voices: 0.0,
            dropped: 0,
        };
        s.prepare(rate, mpfree_node::MAX_BLOCK);
        s
    }

    /// Loads a factory preset: defaults, then the preset's values. Voices
    /// keep sounding; smoothed parameters glide to the new values.
    pub fn load_preset(&mut self, preset: &Preset) {
        for p in PARAMS { if !p.readonly { self.set_param(p.id, p.default); } }
        for &(id, v) in preset.values { self.set_param(id, v); }
    }

    pub fn presets() -> &'static [Preset] { PRESETS }

    /// Voices sounding or fading.
    pub fn active_voices(&self) -> usize { self.voices.iter().filter(|v| v.state != State::Free).count() }

    #[inline]
    fn v(&self, id: u16) -> f32 { self.values[id as usize] }

    fn ctrl_rate(&self) -> f32 { self.rate as f32 / CTRL as f32 }

    /// Pushes parameter targets into the smoothers.
    fn update_targets(&mut self) {
        let pan = self.v(PAN) / 100.0;
        let lvl = 10f32.powf(self.v(LEVEL) / 20.0);
        let lvl = if self.v(LEVEL) <= -60.0 { 0.0 } else { lvl };
        self.sm.lvl1.target = self.v(OSC1_LEVEL) / 100.0;
        self.sm.lvl2.target = self.v(OSC2_LEVEL) / 100.0;
        self.sm.sub.target = self.v(SUB_LEVEL) / 100.0;
        self.sm.noise.target = self.v(NOISE_LEVEL) / 100.0;
        self.sm.pw1.target = self.v(OSC1_PW) / 100.0;
        self.sm.pw2.target = self.v(OSC2_PW) / 100.0;
        self.sm.cutoff.target = self.v(CUTOFF).log2();
        self.sm.res.target = self.v(RESONANCE) / 100.0;
        self.sm.drive.target = self.v(DRIVE);
        self.sm.out_l.target = lvl * (1.0 - pan).sqrt();
        self.sm.out_r.target = lvl * (1.0 + pan).sqrt();
    }

    fn update_envs(&mut self) {
        let fr = self.rate as f32;
        let cr = self.ctrl_rate();
        let (aa, ad, as_, ar) = (self.v(AMP_A), self.v(AMP_D), self.v(AMP_S) / 100.0, self.v(AMP_R));
        let (fa, fd, fs, frl) = (self.v(FENV_A), self.v(FENV_D), self.v(FENV_S) / 100.0, self.v(FENV_R));
        for v in &mut self.voices {
            v.amp.set(aa, ad, as_, ar, fr);
            v.fenv.set(fa, fd, fs, frl, cr);
        }
    }

    fn reset_smoothers(&mut self) {
        self.update_targets();
        let cr = self.rate / CTRL as u32;
        let sm = &mut self.sm;
        for s in [&mut sm.lvl1, &mut sm.lvl2, &mut sm.sub, &mut sm.noise, &mut sm.pw1, &mut sm.pw2, &mut sm.res, &mut sm.drive, &mut sm.out_l, &mut sm.out_r, &mut sm.cutoff] {
            s.set_time(cr, 15.0);
            s.snap(s.target);
        }
        sm.bend.set_time(cr, 4.0);
    }

    // ---- notes ----

    fn note_on(&mut self, note: u8, velocity: u8) {
        if velocity == 0 { self.note_off(note); return; }
        let vel = f32::from(velocity.min(127)) / 127.0;
        self.clock += 1;
        if self.stack_len == 0 && self.v(LFO_RETRIG) >= 0.5 { self.lfo_phase = 0.0; }
        let mode = self.v(VOICE_MODE) as u8;
        // The note stack (also kept in poly mode, for retrigger on AllOff).
        self.stack_remove(note);
        if self.stack_len == MAX_POLY { self.stack.copy_within(1.., 0); self.stack_len -= 1; }
        let was_held = self.stack_len > 0;
        self.stack[self.stack_len] = note;
        self.stack_len += 1;

        if mode != 0 {
            // Mono and legato use slot 0 only.
            let glide = self.v(GLIDE) > 0.0;
            let legato = mode == 2 && was_held && self.voices[0].state == State::On;
            let from = if self.voices[0].sounding() { self.voices[0].pitch } else { self.last_pitch };
            let v = &mut self.voices[0];
            if v.state == State::Free || v.state == State::Kill { Self::fresh(v); }
            v.note = note;
            v.target = f32::from(note);
            v.pitch = if glide && (mode == 1 || legato) { from } else { v.target };
            if !legato {
                v.vel = vel;
                v.amp.gate_on();
                v.fenv.gate_on();
            }
            v.state = State::On;
            v.age = self.clock;
            self.last_pitch = f32::from(note);
            self.voice_control(0);
            return;
        }

        let glide = self.v(GLIDE) > 0.0;
        let from = self.last_pitch;
        self.last_pitch = f32::from(note);
        // The same note sounding again reuses its voice.
        let slot = match self.voices.iter().position(|v| v.sounding() && v.note == note) {
            Some(i) => i,
            None => {
                let poly = (self.v(POLYPHONY) as usize).clamp(1, MAX_POLY);
                let sounding = self.voices.iter().filter(|v| v.sounding()).count();
                if sounding >= poly { self.steal(); }
                match self.voices.iter().position(|v| v.state == State::Free) {
                    Some(i) => i,
                    None => {
                        // Every slot is fading: reuse the quietest.
                        let Some(i) = (0..POOL).min_by(|&a, &b| self.voices[a].kill_gain.total_cmp(&self.voices[b].kill_gain)) else { self.dropped += 1; return };
                        i
                    }
                }
            }
        };
        let unison = (self.v(UNISON) as usize).clamp(1, UNISON_MAX);
        let v = &mut self.voices[slot];
        if !v.sounding() { Self::fresh(v); }
        v.note = note;
        v.vel = vel;
        v.target = f32::from(note);
        v.pitch = if glide { from } else { v.target };
        v.state = State::On;
        v.age = self.clock;
        v.amp.gate_on();
        v.fenv.gate_on();
        if unison > 1 {
            for u in 0..unison {
                v.ph1[u] = self.rng.unit();
                v.ph2[u] = self.rng.unit();
            }
        }
        self.voice_control(slot);
    }

    /// Clears a slot's state for a new note.
    fn fresh(v: &mut Voice) {
        v.ph1 = [0.0; UNISON_MAX];
        v.ph2 = [0.0; UNISON_MAX];
        v.ph_sub = 0.0;
        v.amp.clear();
        v.fenv.clear();
        v.filt = [Filter::default(); 2];
        v.press.snap(0.0);
        v.kill_gain = 1.0;
    }

    /// Fades out one sounding voice: the quietest releasing one, else the
    /// oldest held one.
    fn steal(&mut self) {
        let rel = (0..POOL).filter(|&i| self.voices[i].state == State::Release).min_by(|&a, &b| self.voices[a].amp.level.total_cmp(&self.voices[b].amp.level));
        let victim = rel.or_else(|| (0..POOL).filter(|&i| self.voices[i].state == State::On).min_by_key(|&i| self.voices[i].age));
        if let Some(i) = victim {
            let n = (KILL_MS * 0.001 * self.rate as f32).max(1.0);
            let v = &mut self.voices[i];
            v.state = State::Kill;
            v.kill_gain = 1.0;
            v.kill_step = 1.0 / n;
        }
    }

    fn stack_remove(&mut self, note: u8) {
        if let Some(i) = self.stack[..self.stack_len].iter().position(|&n| n == note) {
            self.stack.copy_within(i + 1..self.stack_len, i);
            self.stack_len -= 1;
        }
    }

    fn note_off(&mut self, note: u8) {
        self.stack_remove(note);
        let mode = self.v(VOICE_MODE) as u8;
        let glide = self.v(GLIDE) > 0.0;
        if mode != 0 {
            let v = &mut self.voices[0];
            if v.state != State::On || v.note != note { return; }
            if self.stack_len > 0 {
                // Back to the newest note still held, without a retrigger.
                let back = self.stack[self.stack_len - 1];
                v.note = back;
                v.target = f32::from(back);
                if !glide { v.pitch = v.target; }
                self.last_pitch = v.target;
            } else {
                v.state = State::Release;
                v.amp.gate_off();
                v.fenv.gate_off();
            }
            return;
        }
        for v in &mut self.voices {
            if v.state == State::On && v.note == note {
                v.state = State::Release;
                v.amp.gate_off();
                v.fenv.gate_off();
            }
        }
    }

    fn all_off(&mut self) {
        self.stack_len = 0;
        for v in &mut self.voices {
            if v.state == State::On {
                v.state = State::Release;
                v.amp.gate_off();
                v.fenv.gate_off();
            }
        }
    }

    fn pressure(&mut self, note: u8, value: u8) {
        for v in &mut self.voices {
            if v.sounding() && v.note == note { v.press.target = f32::from(value.min(127)) / 127.0; }
        }
    }

    fn apply(&mut self, ev: &NodeEvent) {
        match *ev {
            NodeEvent::NoteOn { note, velocity } => self.note_on(note, velocity),
            NodeEvent::NoteOff { note } => self.note_off(note),
            NodeEvent::Pressure { note, value } => self.pressure(note, value),
            NodeEvent::Bend { value } => self.sm.bend.target = if value.is_finite() { value.clamp(-1.0, 1.0) } else { 0.0 },
            NodeEvent::AllOff => self.all_off(),
            NodeEvent::Param { id, value } => self.set_param(id, value),
        }
    }

    // ---- control rate ----

    /// One control step: smoothers, the LFO, then every voice.
    fn control(&mut self, ctx: &Ctx, offset: usize) {
        let sm = &mut self.sm;
        let s = &mut self.sh;
        s.lvl1 = sm.lvl1.next();
        s.lvl2 = sm.lvl2.next();
        s.sub = sm.sub.next();
        s.noise = sm.noise.next();
        sm.pw1.next();
        sm.pw2.next();
        sm.cutoff.next();
        sm.res.next();
        let drive = sm.drive.next();
        s.out_l = sm.out_l.next();
        s.out_r = sm.out_r.next();
        sm.bend.next();
        s.drive_gain = 10f32.powf(drive / 20.0);
        s.drive_amt = (drive / 3.0).min(1.0);
        s.mode = self.values[FILTER_MODE as usize] as u8;
        s.wave1 = self.values[OSC1_WAVE as usize] as u8;
        s.wave2 = self.values[OSC2_WAVE as usize] as u8;
        s.unison = (self.values[UNISON as usize] as usize).clamp(1, UNISON_MAX);

        // LFO: free-running, or locked to the transport's beat.
        let shape = LfoShape::from_index(self.v(LFO_SHAPE));
        let sync = (self.v(LFO_SYNC) as usize).min(LFO_SYNC_BEATS.len() - 1);
        let prev = self.lfo_phase;
        if sync > 0 && ctx.playing {
            let beats = ctx.beats + offset as f64 * ctx.bpm / 60.0 / f64::from(ctx.rate);
            self.lfo_phase = (beats / LFO_SYNC_BEATS[sync]).rem_euclid(1.0);
        } else {
            let hz = if sync > 0 { ctx.bpm / 60.0 / LFO_SYNC_BEATS[sync] } else { f64::from(self.v(LFO_RATE)) };
            self.lfo_phase = (self.lfo_phase + hz * CTRL as f64 / f64::from(self.rate)).rem_euclid(1.0);
        }
        if self.lfo_phase < prev { self.lfo_held = self.rng.bipolar(); }
        self.sh.lfo = lfo_value(shape, self.lfo_phase as f32, self.lfo_held);

        for i in 0..POOL {
            if self.voices[i].state != State::Free { self.voice_control(i); }
        }
    }

    /// Pitch, filter and gains for one voice from the current control values.
    fn voice_control(&mut self, i: usize) {
        let rate = self.rate as f32;
        let lfo = self.sh.lfo;
        let unison = (self.values[UNISON as usize] as usize).clamp(1, UNISON_MAX);
        let detune = self.v(UNISON_DETUNE) / 100.0;
        let spread = self.v(UNISON_SPREAD) / 100.0;
        let bend = self.sm.bend.value * self.v(BEND_RANGE);
        let glide_ms = self.v(GLIDE);
        let glide_c = if glide_ms > 0.0 { seg_coef(glide_ms, self.ctrl_rate()) } else { 1.0 };
        let o1 = self.v(OSC1_OCT) * 12.0;
        let o2 = self.v(OSC2_OCT) * 12.0 + self.v(OSC2_SEMI) + self.v(OSC2_FINE) / 100.0;
        let vib = lfo * self.v(LFO_PITCH);
        let pwm = lfo * self.v(LFO_PW) / 100.0;
        let pw1 = (self.sm.pw1.value + pwm).clamp(0.05, 0.95);
        let pw2 = (self.sm.pw2.value + pwm).clamp(0.05, 0.95);
        let fenv_amt = self.v(FENV_AMOUNT);
        let key = self.v(KEY_TRACK) / 100.0;
        let vel_cut = self.v(VEL_CUTOFF);
        let lfo_cut = lfo * self.v(LFO_CUTOFF);
        let press_cut = self.v(PRESSURE_CUTOFF);
        let vel_amp = self.v(VEL_AMP) / 100.0;
        let trem = 1.0 - self.v(LFO_AMP) / 100.0 * (0.5 - 0.5 * lfo);
        let cutoff = self.sm.cutoff.value;
        let res = self.sm.res.value;
        let norm = VOICE_GAIN / (unison as f32).sqrt();

        let v = &mut self.voices[i];
        v.pitch += (v.target - v.pitch) * glide_c;
        if (v.target - v.pitch).abs() < 1e-4 { v.pitch = v.target; }
        let base = v.pitch + bend + vib;
        for u in 0..unison {
            let pos = if unison > 1 { u as f32 / (unison - 1) as f32 * 2.0 - 1.0 } else { 0.0 };
            let cents = pos * detune;
            v.dt1[u] = (note_hz(base + o1 + cents) / rate).min(0.45);
            v.dt2[u] = (note_hz(base + o2 - cents * 0.7) / rate).min(0.45);
            let p = pos * spread;
            v.pan_l[u] = (1.0 - p).sqrt();
            v.pan_r[u] = (1.0 + p).sqrt();
        }
        v.dt_sub = (note_hz(base + o1 - 12.0) / rate).min(0.45);
        v.pw1 = pw1;
        v.pw2 = pw2;
        let press = v.press.next();
        let fe = v.fenv.next();
        let oct = cutoff + fe * fenv_amt + (f32::from(v.note) - 60.0) / 12.0 * key + v.vel * vel_cut + lfo_cut + press * press_cut;
        v.coefs = FilterCoefs::new(exp2(oct), res, rate);
        let vg = (1.0 - vel_amp) + vel_amp * v.vel * v.vel;
        v.gain = norm * vg * trem;
    }

    // ---- audio rate ----

    fn render(&mut self, from: usize, to: usize, left: &mut [f32], right: &mut [f32]) {
        let s = self.sh;
        let w1 = Wave::from_index(f32::from(s.wave1));
        let w2 = Wave::from_index(f32::from(s.wave2));
        let mode = FilterMode::from_index(f32::from(s.mode));
        // Silent sources are skipped (their phases wait).
        let (use1, use2, use_sub, use_noise) = (s.lvl1 != 0.0, s.lvl2 != 0.0, s.sub != 0.0, s.noise != 0.0);
        for v in &mut self.voices {
            if v.state == State::Free { continue; }
            for k in from..to {
                let mut l = 0.0;
                let mut r = 0.0;
                for u in 0..s.unison {
                    let mut a = 0.0;
                    if use1 { a += osc(w1, &mut v.ph1[u], v.dt1[u], v.pw1) * s.lvl1; }
                    if use2 { a += osc(w2, &mut v.ph2[u], v.dt2[u], v.pw2) * s.lvl2; }
                    l += a * v.pan_l[u];
                    r += a * v.pan_r[u];
                }
                let mut c = 0.0;
                if use_sub { c += osc(Wave::Pulse, &mut v.ph_sub, v.dt_sub, 0.5) * s.sub; }
                if use_noise { c += self.rng.bipolar() * s.noise; }
                let l = drive(l + c, s.drive_gain, s.drive_amt);
                let r = drive(r + c, s.drive_gain, s.drive_amt);
                let l = v.filt[0].process(mode, &v.coefs, l);
                let r = v.filt[1].process(mode, &v.coefs, r);
                let mut g = v.amp.next() * v.gain;
                if v.state == State::Kill {
                    v.kill_gain -= v.kill_step;
                    if v.kill_gain <= 0.0 { v.kill_gain = 0.0; v.state = State::Free; }
                    g *= v.kill_gain;
                }
                left[k] += l * g * s.out_l;
                right[k] += r * g * s.out_r;
                if v.state == State::Free { break; }
                if v.amp.idle() && v.state != State::On {
                    v.state = State::Free;
                    break;
                }
            }
        }
    }
}

impl Node for Poly {
    fn info(&self) -> &'static NodeInfo { &INFO }
    fn params(&self) -> &'static [ParamInfo] { PARAMS }

    fn set_param(&mut self, id: u16, value: f32) {
        let Some(p) = info(id) else { return };
        if p.readonly { return; }
        self.values[id as usize] = p.clamp(value);
        self.update_targets();
        if matches!(id, AMP_A | AMP_D | AMP_S | AMP_R | FENV_A | FENV_D | FENV_S | FENV_R) { self.update_envs(); }
        if id == VOICE_MODE { self.all_off(); }
    }

    fn param(&self, id: u16) -> f32 {
        match id {
            METER_VOICES => self.meter_voices,
            _ if info(id).is_some() => self.values[id as usize],
            _ => f32::NAN,
        }
    }

    fn prepare(&mut self, rate: u32, _max_block: usize) {
        self.rate = rate.max(8_000);
        self.update_envs();
        self.reset_smoothers();
        self.control(&Ctx::new(self.rate, 0), 0);
    }

    fn reset(&mut self) {
        for v in &mut self.voices { *v = Voice::new(); }
        self.update_envs();
        self.reset_smoothers();
        self.stack_len = 0;
        self.lfo_phase = 0.0;
        self.meter_voices = 0.0;
        self.control(&Ctx::new(self.rate, 0), 0);
    }

    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        assert_eq!(left.len(), right.len());
        if ctx.rate != self.rate { self.prepare(ctx.rate, left.len()); }
        let len = left.len();
        // Events land on their frames; a control step runs on every
        // CTRL-aligned absolute frame.
        let mut at = 0usize;
        let mut i = 0usize;
        while at < len {
            while i < events.len() && events[i].offset as usize <= at {
                self.apply(&events[i].ev);
                i += 1;
            }
            let next = events.get(i).map_or(len, |e| (e.offset as usize).min(len));
            while at < next {
                let abs = ctx.frame + at as u64;
                if abs.is_multiple_of(CTRL) { self.control(ctx, at); }
                let end = next.min(at + (CTRL - abs % CTRL) as usize);
                self.render(at, end, left, right);
                at = end;
            }
        }
        // Events at or past the end (a host error) still apply, so no note hangs.
        for e in &events[i..] { self.apply(&e.ev); }
        self.frame = ctx.frame + len as u64;
        self.meter_voices = self.voices.iter().filter(|v| v.sounding()).count() as f32;
    }

    fn tail_frames(&self) -> u32 {
        let ms = self.v(AMP_R).max(KILL_MS);
        (ms * 0.001 * self.rate as f32 * 1.5) as u32
    }
}
