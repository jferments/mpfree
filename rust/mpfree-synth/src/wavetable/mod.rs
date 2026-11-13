//! Wavetable: two wavetable oscillators per voice that scan through a stack
//! of single-cycle frames, each with a phase warp (bend, sync, mirror, FM
//! from the other oscillator) and up to eight unison stacks; a sub sine and
//! noise; Poly's ladder or state-variable filter, then its drive; an amp
//! envelope and a mod envelope (cutoff, positions, warp); a tempo-syncable
//! LFO (pitch, cutoff, positions, level); poly, mono and legato with glide.
//!
//! - [`table`]: band-limited tables, one mip level per half octave, so a
//!   note only reads harmonics below Nyquist.
//! - [`factory`]: ten tables made from formulas, shared by every instance.
//! - [`import`]: tables from WAV files (single cycles, Serum-style
//!   multi-frame files) and from any pitched sample, such as a pad's.
//!
//! A user table reaches the audio thread as an `Arc<Table>` through
//! [`Wavetable::set_user_table`], which hands the old one back so the host
//! drops it on the control thread: the audio thread never frees a table.
//!
//! Timing follows Poly: oscillators, the amp envelope and the filter run
//! every frame; pitch, positions, warp, cutoff, the LFO, the mod envelope
//! and pressure every [`CTRL`] frames on the absolute frame, so the output
//! does not depend on how the host cuts blocks. Positions and gains ramp
//! across each control step.

pub mod factory;
mod fft;
pub mod import;
pub mod params;
pub mod table;

use std::sync::Arc;

use mpfree_node::{
    Ctx, Expr, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Pressure, PressureChord, PressureCoefs,
    PressureMode, PressureShape, Smooth, Timed, Tuning,
};

use crate::dsp::*;
use crate::expr::{expr_coef, NoteExprs, VoiceExpr};
use crate::params::{Preset, LFO_SYNC_BEATS};
use crate::pressure::{swell_gain, Depths, Slot};
use params::*;
use table::Table;

pub use factory::factory;

/// Frames per control step.
pub const CTRL: u64 = 16;
/// The most notes sounding at once.
pub const MAX_POLY: usize = 16;
/// Voice slots: the notes plus room for stolen voices to fade out.
pub const POOL: usize = MAX_POLY + 4;
/// The most unison stacks per oscillator.
pub const UNISON_MAX: usize = 8;
const KILL_MS: f32 = 3.0;
/// Peak of one full-velocity note at the defaults, about -12 dBFS (as Poly).
const VOICE_GAIN: f32 = 0.2;

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.wavetable",
    name: "Wavetable",
    kind: Kind::Instrument,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Free,
    On,
    Release,
    Kill,
}

/// How an oscillator bends its phase this control step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Warp {
    #[default]
    Off,
    /// Phase distortion: the knee at `k`, slopes `a` before it, `b` after.
    Bend {
        k: f32,
        a: f32,
        b: f32,
    },
    Sync {
        ratio: f32,
    },
    Mirror {
        amt: f32,
    },
    Fm {
        index: f32,
    },
}

impl Warp {
    fn new(mode: u8, amt: f32) -> Warp {
        let amt = amt.clamp(0.0, 1.0);
        match mode {
            1 => {
                let k = 0.5 - 0.48 * amt;
                Warp::Bend {
                    k,
                    a: 0.5 / k,
                    b: 0.5 / (1.0 - k),
                }
            }
            2 => Warp::Sync {
                ratio: 1.0 + 7.0 * amt,
            },
            3 => Warp::Mirror { amt },
            4 => Warp::Fm { index: amt * 0.5 },
            _ => Warp::Off,
        }
    }

    /// How many times faster the table is read, for the mip level.
    fn speed(self) -> f32 {
        match self {
            Warp::Bend { a, .. } => a,
            Warp::Sync { ratio } => ratio,
            Warp::Mirror { amt } if amt > 0.0 => 2.0,
            Warp::Fm { index } => 1.0 + 8.0 * index,
            _ => 1.0,
        }
    }

    /// The phase read from, given the running phase and the other
    /// oscillator's last sample (for FM).
    #[inline]
    fn apply(self, p: f32, other: f32) -> f32 {
        match self {
            Warp::Off => p,
            Warp::Bend { k, a, b } => {
                if p < k {
                    p * a
                } else {
                    0.5 + (p - k) * b
                }
            }
            Warp::Sync { ratio } => (p * ratio).fract(),
            Warp::Mirror { amt } => {
                let m = 1.0 - (1.0 - 2.0 * p).abs();
                (p + (m - p) * amt).clamp(0.0, 0.999_999)
            }
            Warp::Fm { index } => {
                let x = p + index * other;
                x - x.floor()
            }
        }
    }
}

/// Where an oscillator reads during one control step: two neighbouring
/// frames of its level and the crossfade between them, ramped per frame.
/// A fast sweep that crosses a frame inside a step extrapolates past the
/// pair for the rest of that step (16 frames), which is inaudible.
#[derive(Clone, Copy, Debug, Default)]
struct Run {
    off0: usize,
    off1: usize,
    t: f32,
    t_inc: f32,
    len: f32,
}

impl Run {
    /// A run on `level` of `tab` from frame position `from` to `to`.
    fn new(tab: &Table, level: usize, from: f32, to: f32) -> Run {
        let lv = &tab.levels()[level];
        let len = lv.len as f32;
        let last = tab.frames() - 1;
        if last == 0 {
            return Run {
                len,
                ..Run::default()
            };
        }
        let (from, to) = (from.clamp(0.0, last as f32), to.clamp(0.0, last as f32));
        let f0 = (from.min(to) as usize).min(last - 1);
        let st = lv.stride();
        Run {
            off0: f0 * st,
            off1: (f0 + 1) * st,
            t: from - f0 as f32,
            t_inc: (to - from) / CTRL as f32,
            len,
        }
    }

    /// One sample at phase `ph` (0 to 1): linear in phase, then between
    /// the two frames.
    #[inline]
    fn read(&self, d: &[f32], ph: f32) -> f32 {
        let x = ph * self.len;
        let i = x as usize;
        let fr = x - i as f32;
        let (a0, a1) = (d[self.off0 + i], d[self.off0 + i + 1]);
        let (b0, b1) = (d[self.off1 + i], d[self.off1 + i + 1]);
        let s0 = a0 + (a1 - a0) * fr;
        let s1 = b0 + (b1 - b0) * fr;
        s0 + (s1 - s0) * self.t
    }
}

#[derive(Clone, Copy, Debug)]
struct Voice {
    state: State,
    note: u8,
    vel: f32,
    age: u64,
    pitch: f32,
    target: f32,
    /// Per oscillator, per unison stack: phase, increment, last output.
    ph: [[f32; UNISON_MAX]; 2],
    dt: [[f32; UNISON_MAX]; 2],
    last: [[f32; UNISON_MAX]; 2],
    ph_sub: f32,
    dt_sub: f32,
    pan_l: [f32; UNISON_MAX],
    pan_r: [f32; UNISON_MAX],
    /// Per oscillator: mip level, the fastest stack's increment (times the
    /// warp's speed-up) it was picked for, frame position at the end of
    /// this control step, the read run, warp.
    level: [usize; 2],
    top: [f32; 2],
    pos: [f32; 2],
    run: [Run; 2],
    warp: [Warp; 2],
    amp: Env,
    menv: Env,
    coefs: FilterCoefs,
    filt: [Filter; 2],
    gain: f32,
    gain_inc: f32,
    drive_gain: f32,
    drive_amt: f32,
    /// Keeps the driven voice at its undriven level (dsp::DriveLevel).
    drive_level: DriveLevel,
    press: Pressure,
    pv: f32,
    /// Its note's own slide and tone.
    ex: VoiceExpr,
    kill_gain: f32,
    kill_step: f32,
    /// Whether the next control step snaps positions instead of ramping.
    snap: bool,
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
            ph: [[0.0; UNISON_MAX]; 2],
            dt: [[0.0; UNISON_MAX]; 2],
            last: [[0.0; UNISON_MAX]; 2],
            ph_sub: 0.0,
            dt_sub: 0.0,
            pan_l: [1.0; UNISON_MAX],
            pan_r: [1.0; UNISON_MAX],
            level: [0; 2],
            top: [0.0; 2],
            pos: [0.0; 2],
            run: [Run::default(); 2],
            warp: [Warp::Off; 2],
            amp: Env::default(),
            menv: Env::default(),
            coefs: FilterCoefs::default(),
            filt: [Filter::default(); 2],
            gain: 0.0,
            gain_inc: 0.0,
            drive_gain: 1.0,
            drive_amt: 0.0,
            drive_level: DriveLevel::new(),
            press: Pressure::new(),
            pv: 0.0,
            ex: VoiceExpr::default(),
            kill_gain: 1.0,
            kill_step: 0.0,
            snap: true,
        }
    }

    fn sounding(&self) -> bool {
        matches!(self.state, State::On | State::Release)
    }
}

#[derive(Clone, Copy, Debug)]
struct Smoothed {
    lvl: [Smooth; 2],
    pos: [Smooth; 2],
    warp: [Smooth; 2],
    sub: Smooth,
    noise: Smooth,
    cutoff: Smooth,
    res: Smooth,
    drive: Smooth,
    out_l: Smooth,
    out_r: Smooth,
    bend: Smooth,
}

#[derive(Clone, Copy, Debug, Default)]
struct Shared {
    lfo: f32,
    mode: u8,
    lvl: [f32; 2],
    sub: f32,
    noise: f32,
    drive_gain: f32,
    drive_amt: f32,
    out_l: f32,
    out_r: f32,
    unison: usize,
}

pub struct Wavetable {
    rate: u32,
    values: [f32; ID_SPACE],
    voices: [Voice; POOL],
    /// The table each oscillator reads, and the user tables loaded.
    tables: [Arc<Table>; 2],
    user: [Option<Arc<Table>>; 2],
    sm: Smoothed,
    sh: Shared,
    rng: Rng,
    lfo_phase: f64,
    lfo_held: f32,
    stack: [u8; MAX_POLY],
    stack_len: usize,
    clock: u64,
    last_pitch: f32,
    meter_voices: f32,
    press_c: PressureCoefs,
    depths: Depths,
    chord: PressureChord,
    vib_phase: f32,
    vib: f32,
    /// How notes become frequencies.
    tuning: Tuning,
    /// Each note's slide and tone for its next note-on, and the voices'
    /// smoothing coefficient.
    exprs: NoteExprs,
    expr_c: f32,
    /// Notes that did not fit anywhere (should stay 0).
    pub dropped: u64,
}

impl Default for Wavetable {
    fn default() -> Self {
        Wavetable::new(48_000)
    }
}

impl Wavetable {
    /// A Wavetable synth at the defaults. Builds the factory tables on the
    /// first call in the process: call it on the control thread.
    pub fn new(rate: u32) -> Wavetable {
        let mut values = [0.0; ID_SPACE];
        for p in PARAMS {
            values[p.id as usize] = p.default;
        }
        let v = |id: u16| values[id as usize];
        let f = factory();
        let sm = Smoothed {
            lvl: [
                Smooth::new(v(osc_id(0, OSC_LEVEL)) / 100.0),
                Smooth::new(v(osc_id(1, OSC_LEVEL)) / 100.0),
            ],
            pos: [
                Smooth::new(v(osc_id(0, POSITION)) / 100.0),
                Smooth::new(v(osc_id(1, POSITION)) / 100.0),
            ],
            warp: [
                Smooth::new(v(osc_id(0, WARP)) / 100.0),
                Smooth::new(v(osc_id(1, WARP)) / 100.0),
            ],
            sub: Smooth::new(v(SUB_LEVEL) / 100.0),
            noise: Smooth::new(v(NOISE_LEVEL) / 100.0),
            cutoff: Smooth::new(v(CUTOFF).log2()),
            res: Smooth::new(v(RESONANCE) / 100.0),
            drive: Smooth::new(v(DRIVE)),
            out_l: Smooth::new(1.0),
            out_r: Smooth::new(1.0),
            bend: Smooth::new(0.0),
        };
        let mut s = Wavetable {
            rate,
            values,
            voices: [Voice::new(); POOL],
            tables: [f[0].clone(), f[0].clone()],
            user: [None, None],
            sm,
            sh: Shared::default(),
            rng: Rng::new(0x0057_AB1E_5EED),
            lfo_phase: 0.0,
            lfo_held: 0.0,
            stack: [0; MAX_POLY],
            stack_len: 0,
            clock: 0,
            last_pitch: 60.0,
            meter_voices: 0.0,
            press_c: PressureCoefs::default(),
            depths: Depths::default(),
            chord: PressureChord::default(),
            vib_phase: 0.0,
            vib: 0.0,
            tuning: Tuning::standard(),
            exprs: NoteExprs::new(),
            expr_c: 1.0,
            dropped: 0,
        };
        s.pick_tables();
        s.prepare(rate, mpfree_node::MAX_BLOCK);
        s
    }

    pub fn load_preset(&mut self, preset: &Preset) {
        for p in PARAMS {
            if !p.readonly {
                self.set_param(p.id, p.default);
            }
        }
        for &(id, v) in preset.values {
            self.set_param(id, v);
        }
    }

    pub fn presets() -> &'static [Preset] {
        PRESETS
    }

    /// Loads a user table into oscillator `o` (0 = A, 1 = B); the Table
    /// choice's "User" plays it. Returns the table it replaces, which the
    /// caller must drop off the audio thread. Never allocates or frees.
    pub fn set_user_table(&mut self, o: usize, table: Arc<Table>) -> Option<Arc<Table>> {
        let o = o.min(1);
        let old = self.user[o].replace(table);
        self.pick_tables();
        old
    }

    /// The table oscillator `o` reads now.
    pub fn table(&self, o: usize) -> &Arc<Table> {
        &self.tables[o.min(1)]
    }

    /// The user table loaded into oscillator `o`, if any.
    pub fn user_table(&self, o: usize) -> Option<&Arc<Table>> {
        self.user[o.min(1)].as_ref()
    }

    pub fn active_voices(&self) -> usize {
        self.voices
            .iter()
            .filter(|v| v.state != State::Free)
            .count()
    }

    /// The pressure each sounding voice uses (0 to 1), for the live bars.
    pub fn pressures(&self, out: &mut [f32]) -> usize {
        let mut n = 0;
        for v in self.voices.iter().filter(|v| v.sounding()) {
            if n == out.len() {
                break;
            }
            out[n] = v.pv;
            n += 1;
        }
        n
    }

    /// Where each oscillator reads now, 0 to 1 through its table: the
    /// newest sounding voice's, else the knob's. For the table visual.
    pub fn live_positions(&self) -> [f32; 2] {
        let newest = self
            .voices
            .iter()
            .filter(|v| v.sounding())
            .max_by_key(|v| v.age);
        std::array::from_fn(|o| {
            let frames = self.tables[o].frames();
            match newest {
                Some(v) if frames > 1 => v.pos[o] / (frames - 1) as f32,
                _ => self.v(osc_id(o, POSITION)) / 100.0,
            }
        })
    }

    #[inline]
    fn v(&self, id: u16) -> f32 {
        self.values[id as usize]
    }

    /// Points each oscillator at its chosen table. Clones an `Arc` (no
    /// allocation); the old one is still held by the factory or `user`.
    fn pick_tables(&mut self) {
        let f = factory();
        for o in 0..2 {
            let i = self.v(osc_id(o, TABLE));
            let t = match &self.user[o] {
                Some(u) if i >= USER_TABLE => u,
                // "User" with nothing loaded plays Basic.
                _ if i >= USER_TABLE => &f[0],
                _ => &f[(i as usize).min(f.len() - 1)],
            };
            if Arc::ptr_eq(t, &self.tables[o]) {
                continue;
            }
            self.tables[o] = t.clone();
            // Sounding voices move to the new table where they are.
            let tab = &self.tables[o];
            for v in self.voices.iter_mut().filter(|v| v.state != State::Free) {
                v.level[o] = tab.level_for(v.top[o]);
                v.pos[o] = v.pos[o].min((tab.frames() - 1) as f32);
                v.run[o] = Run::new(tab, v.level[o], v.pos[o], v.pos[o]);
            }
        }
    }

    fn update_pressure(&mut self) {
        let shape = PressureShape::from_params(PRESS_BASE, |id| self.values[id as usize]);
        self.press_c = shape.coefs(self.ctrl_rate());
        let slots: [Slot; 4] = std::array::from_fn(|n| Slot {
            target: self.v(press_target(n)) as u8,
            depth: self.v(press_depth(n)) / 100.0,
        });
        self.depths = if shape.mode == PressureMode::Off {
            Depths::default()
        } else {
            Depths::from_slots(&slots)
        };
    }

    fn ctrl_rate(&self) -> f32 {
        self.rate as f32 / CTRL as f32
    }

    fn update_targets(&mut self) {
        let pan = self.v(PAN) / 100.0;
        let lvl = if self.v(LEVEL) <= -60.0 {
            0.0
        } else {
            10f32.powf(self.v(LEVEL) / 20.0)
        };
        for o in 0..2 {
            self.sm.lvl[o].target = self.v(osc_id(o, OSC_LEVEL)) / 100.0;
            self.sm.pos[o].target = self.v(osc_id(o, POSITION)) / 100.0;
            self.sm.warp[o].target = self.v(osc_id(o, WARP)) / 100.0;
        }
        self.sm.sub.target = self.v(SUB_LEVEL) / 100.0;
        self.sm.noise.target = self.v(NOISE_LEVEL) / 100.0;
        self.sm.cutoff.target = self.v(CUTOFF).log2();
        self.sm.res.target = self.v(RESONANCE) / 100.0;
        self.sm.drive.target = self.v(DRIVE);
        self.sm.out_l.target = lvl * (1.0 - pan).sqrt();
        self.sm.out_r.target = lvl * (1.0 + pan).sqrt();
    }

    fn update_envs(&mut self) {
        let fr = self.rate as f32;
        let cr = self.ctrl_rate();
        let (aa, ad, as_, ar) = (
            self.v(AMP_A),
            self.v(AMP_D),
            self.v(AMP_S) / 100.0,
            self.v(AMP_R),
        );
        let (ma, md, ms, mr) = (
            self.v(MENV_A),
            self.v(MENV_D),
            self.v(MENV_S) / 100.0,
            self.v(MENV_R),
        );
        for v in &mut self.voices {
            v.amp.set(aa, ad, as_, ar, fr);
            v.menv.set(ma, md, ms, mr, cr);
        }
    }

    fn reset_smoothers(&mut self) {
        self.update_targets();
        let cr = self.rate / CTRL as u32;
        let sm = &mut self.sm;
        let [l0, l1] = &mut sm.lvl;
        let [p0, p1] = &mut sm.pos;
        let [w0, w1] = &mut sm.warp;
        for s in [
            l0,
            l1,
            p0,
            p1,
            w0,
            w1,
            &mut sm.sub,
            &mut sm.noise,
            &mut sm.res,
            &mut sm.drive,
            &mut sm.out_l,
            &mut sm.out_r,
            &mut sm.cutoff,
        ] {
            s.set_time(cr, 15.0);
            s.snap(s.target);
        }
        sm.bend.set_time(cr, 4.0);
        self.expr_c = expr_coef(cr as f32);
    }

    // ---- notes ----

    fn note_on(&mut self, note: u8, velocity: u8) {
        if velocity == 0 {
            self.note_off(note);
            return;
        }
        let vel = f32::from(velocity.min(127)) / 127.0;
        self.clock += 1;
        if self.stack_len == 0 && self.v(LFO_RETRIG) >= 0.5 {
            self.lfo_phase = 0.0;
        }
        let mode = self.v(VOICE_MODE) as u8;
        self.stack_remove(note);
        if self.stack_len == MAX_POLY {
            self.stack.copy_within(1.., 0);
            self.stack_len -= 1;
        }
        let was_held = self.stack_len > 0;
        self.stack[self.stack_len] = note;
        self.stack_len += 1;
        let glide = self.v(GLIDE) > 0.0;

        if mode != 0 {
            let legato = mode == 2 && was_held && self.voices[0].state == State::On;
            let from = if self.voices[0].sounding() {
                self.voices[0].pitch
            } else {
                self.last_pitch
            };
            let ex = self.exprs.on(note);
            let v = &mut self.voices[0];
            let fresh = v.state == State::Free || v.state == State::Kill;
            if fresh {
                Self::fresh(v);
            }
            v.press.start();
            if legato {
                v.ex.retarget(ex);
            } else {
                v.ex.start(ex);
            }
            v.note = note;
            v.target = f32::from(note);
            v.pitch = if glide && (mode == 1 || legato) {
                from
            } else {
                v.target
            };
            if !legato {
                v.vel = vel;
                v.amp.gate_on();
                v.menv.gate_on();
            }
            v.state = State::On;
            v.age = self.clock;
            self.last_pitch = f32::from(note);
            self.voice_control(0);
            if fresh {
                Self::snap_gain(&mut self.voices[0]);
            }
            return;
        }

        let from = self.last_pitch;
        self.last_pitch = f32::from(note);
        let slot = match self
            .voices
            .iter()
            .position(|v| v.sounding() && v.note == note)
        {
            Some(i) => i,
            None => {
                let poly = (self.v(POLYPHONY) as usize).clamp(1, MAX_POLY);
                if self.voices.iter().filter(|v| v.sounding()).count() >= poly {
                    self.steal();
                }
                match self.voices.iter().position(|v| v.state == State::Free) {
                    Some(i) => i,
                    None => {
                        let Some(i) = (0..POOL).min_by(|&a, &b| {
                            self.voices[a]
                                .kill_gain
                                .total_cmp(&self.voices[b].kill_gain)
                        }) else {
                            self.dropped += 1;
                            return;
                        };
                        i
                    }
                }
            }
        };
        let unison = (self.v(UNISON) as usize).clamp(1, UNISON_MAX);
        let ex = self.exprs.on(note);
        let v = &mut self.voices[slot];
        let fresh = !v.sounding();
        if fresh {
            Self::fresh(v);
        }
        v.press.start();
        v.ex.start(ex);
        v.note = note;
        v.vel = vel;
        v.target = f32::from(note);
        v.pitch = if glide { from } else { v.target };
        v.state = State::On;
        v.age = self.clock;
        v.amp.gate_on();
        v.menv.gate_on();
        if unison > 1 {
            for u in 0..unison {
                v.ph[0][u] = self.rng.unit();
                v.ph[1][u] = self.rng.unit();
            }
        }
        self.voice_control(slot);
        if fresh {
            Self::snap_gain(&mut self.voices[slot]);
        }
    }

    fn snap_gain(v: &mut Voice) {
        v.gain += v.gain_inc * CTRL as f32;
        v.gain_inc = 0.0;
    }

    fn fresh(v: &mut Voice) {
        v.ph = [[0.0; UNISON_MAX]; 2];
        v.last = [[0.0; UNISON_MAX]; 2];
        v.ph_sub = 0.0;
        v.amp.clear();
        v.menv.clear();
        v.filt = [Filter::default(); 2];
        v.press.clear();
        v.pv = 0.0;
        v.kill_gain = 1.0;
        v.snap = true;
        v.drive_level.clear();
    }

    fn steal(&mut self) {
        let rel = (0..POOL)
            .filter(|&i| self.voices[i].state == State::Release)
            .min_by(|&a, &b| {
                self.voices[a]
                    .amp
                    .level
                    .total_cmp(&self.voices[b].amp.level)
            });
        let victim = rel.or_else(|| {
            (0..POOL)
                .filter(|&i| self.voices[i].state == State::On)
                .min_by_key(|&i| self.voices[i].age)
        });
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
        self.exprs.off(note);
        self.stack_remove(note);
        let mode = self.v(VOICE_MODE) as u8;
        let glide = self.v(GLIDE) > 0.0;
        if mode != 0 {
            let v = &mut self.voices[0];
            if v.state != State::On || v.note != note {
                return;
            }
            if self.stack_len > 0 {
                let back = self.stack[self.stack_len - 1];
                v.ex.retarget(self.exprs.on(back));
                v.note = back;
                v.target = f32::from(back);
                if !glide {
                    v.pitch = v.target;
                }
                self.last_pitch = v.target;
            } else {
                Self::release(v);
            }
            return;
        }
        for v in &mut self.voices {
            if v.state == State::On && v.note == note {
                Self::release(v);
            }
        }
    }

    fn release(v: &mut Voice) {
        v.state = State::Release;
        v.amp.gate_off();
        v.menv.gate_off();
        v.press.release();
    }

    fn all_off(&mut self) {
        self.stack_len = 0;
        self.exprs.clear();
        for v in &mut self.voices {
            if v.state == State::On {
                Self::release(v);
            }
        }
    }

    fn pressure(&mut self, note: u8, value: u8) {
        for v in &mut self.voices {
            if v.sounding() && v.note == note {
                v.press.set(value);
            }
        }
    }

    /// One note's own slide or tone: its held voices glide to it.
    fn expression(&mut self, note: u8, kind: Expr, value: f32) {
        let Some(value) = self.exprs.set(note, kind, value) else {
            return;
        };
        for v in &mut self.voices {
            if v.state == State::On && v.note == note {
                v.ex.aim(kind, value);
            }
        }
    }

    fn apply(&mut self, ev: &NodeEvent) {
        match *ev {
            NodeEvent::NoteOn { note, velocity } => self.note_on(note, velocity),
            NodeEvent::NoteOff { note } => self.note_off(note),
            NodeEvent::Pressure { note, value } => self.pressure(note, value),
            NodeEvent::Bend { value } => {
                self.sm.bend.target = if value.is_finite() {
                    value.clamp(-1.0, 1.0)
                } else {
                    0.0
                }
            }
            NodeEvent::AllOff => self.all_off(),
            NodeEvent::Param { id, value } => self.set_param(id, value),
            NodeEvent::Expression { note, kind, value } => self.expression(note, kind, value),
        }
    }

    // ---- control rate ----

    fn control(&mut self, ctx: &Ctx, offset: usize) {
        let sm = &mut self.sm;
        let s = &mut self.sh;
        for o in 0..2 {
            s.lvl[o] = sm.lvl[o].next();
            sm.pos[o].next();
            sm.warp[o].next();
        }
        s.sub = sm.sub.next();
        s.noise = sm.noise.next();
        sm.cutoff.next();
        sm.res.next();
        let drive = sm.drive.next();
        s.out_l = sm.out_l.next();
        s.out_r = sm.out_r.next();
        sm.bend.next();
        s.drive_gain = 10f32.powf(drive / 20.0);
        s.drive_amt = (drive / 3.0).min(1.0);
        s.mode = self.values[FILTER_MODE as usize] as u8;
        s.unison = (self.values[UNISON as usize] as usize).clamp(1, UNISON_MAX);

        let c = &self.press_c;
        let chord = if c.mode == PressureMode::Highest {
            self.chord.step(
                c,
                self.voices
                    .iter()
                    .filter(|v| v.sounding())
                    .map(|v| (v.state == State::On, v.press.aim(c))),
            )
        } else {
            0.0
        };
        let mut top = 0.0f32;
        for v in self.voices.iter_mut().filter(|v| v.state != State::Free) {
            let own = v.press.step(c);
            v.pv = match c.mode {
                PressureMode::PerNote => own,
                PressureMode::Highest => chord,
                PressureMode::Off => 0.0,
            };
            if v.sounding() {
                top = top.max(v.pv);
            }
        }
        let vib_hz = self.values[PRESS_VIB_RATE as usize];
        self.vib_phase = (self.vib_phase + vib_hz * CTRL as f32 / self.rate as f32).fract();
        self.vib = (std::f32::consts::TAU * self.vib_phase).sin();

        let shape = LfoShape::from_index(self.v(LFO_SHAPE));
        let sync = (self.v(LFO_SYNC) as usize).min(LFO_SYNC_BEATS.len() - 1);
        let prev = self.lfo_phase;
        if sync > 0 && ctx.playing {
            let beats = ctx.beats + offset as f64 * ctx.bpm / 60.0 / f64::from(ctx.rate);
            self.lfo_phase = (beats / LFO_SYNC_BEATS[sync]).rem_euclid(1.0);
        } else {
            let hz = if sync > 0 {
                ctx.bpm / 60.0 / LFO_SYNC_BEATS[sync]
            } else {
                f64::from(self.v(LFO_RATE))
            };
            let hz = hz * f64::from(exp2(self.depths.get(target::LFO_RATE) * 2.0 * top));
            self.lfo_phase =
                (self.lfo_phase + hz * CTRL as f64 / f64::from(self.rate)).rem_euclid(1.0);
        }
        if self.lfo_phase < prev {
            self.lfo_held = self.rng.bipolar();
        }
        self.sh.lfo = lfo_value(shape, self.lfo_phase as f32, self.lfo_held);

        for i in 0..POOL {
            if self.voices[i].state != State::Free {
                self.voice_control(i);
            }
        }
    }

    /// Pitch, positions, warp, filter and gain for one voice.
    fn voice_control(&mut self, i: usize) {
        let rate = self.rate as f32;
        let lfo = self.sh.lfo;
        let unison = (self.values[UNISON as usize] as usize).clamp(1, UNISON_MAX);
        let detune = self.v(UNISON_DETUNE) / 100.0;
        let spread = self.v(UNISON_SPREAD) / 100.0;
        let bend = self.sm.bend.value * self.v(BEND_RANGE);
        let glide_ms = self.v(GLIDE);
        let glide_c = if glide_ms > 0.0 {
            seg_coef(glide_ms, self.ctrl_rate())
        } else {
            1.0
        };
        let tune: [f32; 2] = std::array::from_fn(|o| {
            self.v(osc_id(o, OCT)) * 12.0
                + self.v(osc_id(o, SEMI))
                + self.v(osc_id(o, FINE)) / 100.0
        });
        let vib = lfo * self.v(LFO_PITCH);
        let d = self.depths;
        let vib_p = self.vib;
        let menv_cut = self.v(MENV_CUTOFF);
        let menv_pos = [self.v(MENV_POS_A) / 100.0, self.v(MENV_POS_B) / 100.0];
        let menv_warp = self.v(MENV_WARP) / 100.0;
        let lfo_pos = [self.v(LFO_POS_A) / 100.0, self.v(LFO_POS_B) / 100.0];
        let vel_pos = self.v(VEL_POS) / 100.0;
        let key = self.v(KEY_TRACK) / 100.0;
        let vel_cut = self.v(VEL_CUTOFF);
        let lfo_cut = lfo * self.v(LFO_CUTOFF);
        let vel_amp = self.v(VEL_AMP) / 100.0;
        let trem = 1.0 - self.v(LFO_AMP) / 100.0 * (0.5 - 0.5 * lfo);
        let cutoff = self.sm.cutoff.value;
        let res = self.sm.res.value;
        let base_pos = [self.sm.pos[0].value, self.sm.pos[1].value];
        let base_warp = [self.sm.warp[0].value, self.sm.warp[1].value];
        let modes = [
            self.v(osc_id(0, WARP_MODE)) as u8,
            self.v(osc_id(1, WARP_MODE)) as u8,
        ];
        let (drive_gain, drive_amt) = (self.sh.drive_gain, self.sh.drive_amt);
        let norm = VOICE_GAIN / (unison as f32).sqrt();
        let tabs = [&self.tables[0], &self.tables[1]];
        let expr_c = self.expr_c;
        let tuning = &self.tuning;

        let v = &mut self.voices[i];
        let p = v.pv;
        v.pitch += (v.target - v.pitch) * glide_c;
        if (v.target - v.pitch).abs() < 1e-4 {
            v.pitch = v.target;
        }
        v.ex.step(expr_c);
        let mut base = v.pitch + bend + vib;
        if v.ex.slide != 0.0 {
            base += v.ex.slide;
        }
        if d.any {
            base += p * (d.get(target::PITCH) * 12.0 + d.get(target::VIBRATO) * vib_p);
        }
        let me = v.menv.next();
        for o in 0..2 {
            // Position: knob, mod env, LFO (unipolar sweep up), velocity, pressure.
            let pos = base_pos[o]
                + me * menv_pos[o]
                + (0.5 + 0.5 * lfo) * lfo_pos[o]
                + (v.vel - 0.5) * vel_pos
                + p * d.get(target::POSITION);
            let at = pos.clamp(0.0, 1.0) * (tabs[o].frames() - 1) as f32;
            let from = if v.snap { at } else { v.pos[o] };
            v.pos[o] = at;
            let w = base_warp[o] + me * menv_warp + p * d.get(target::WARP);
            v.warp[o] = Warp::new(modes[o], w);
            let mut top = 0.0f32;
            for u in 0..unison {
                let pos = if unison > 1 {
                    u as f32 / (unison - 1) as f32 * 2.0 - 1.0
                } else {
                    0.0
                };
                let cents = pos * detune * if o == 0 { 1.0 } else { -0.7 };
                v.dt[o][u] = (tuning.hz(base + tune[o] + cents) / rate).min(0.45);
                top = top.max(v.dt[o][u]);
            }
            v.top[o] = top * v.warp[o].speed();
            v.level[o] = tabs[o].level_for(v.top[o]);
            v.run[o] = Run::new(tabs[o], v.level[o], from, at);
        }
        v.snap = false;
        for u in 0..unison {
            let pos = if unison > 1 {
                u as f32 / (unison - 1) as f32 * 2.0 - 1.0
            } else {
                0.0
            };
            let q = pos * spread;
            v.pan_l[u] = (1.0 - q).sqrt();
            v.pan_r[u] = (1.0 + q).sqrt();
        }
        v.dt_sub = (tuning.hz(base + tune[0] - 12.0) / rate).min(0.45);
        let oct = cutoff
            + me * menv_cut
            + (f32::from(v.note) - 60.0) / 12.0 * key
            + v.vel * vel_cut
            + lfo_cut
            + p * d.get(target::CUTOFF) * 4.0;
        let res = (res + p * d.get(target::RESONANCE)).clamp(0.0, 1.0);
        v.coefs = FilterCoefs::new(exp2(oct), res, rate);
        v.drive_gain = drive_gain;
        v.drive_amt = drive_amt;
        let vg = (1.0 - vel_amp) + vel_amp * v.vel * v.vel;
        let swell = swell_gain(d.get(target::LEVEL).clamp(-1.0, 1.0), p);
        // The drive's level correction, from the step just rendered.
        let level = if v.drive_amt > 0.0 {
            v.drive_level.step(CTRL, self.rate)
        } else {
            v.drive_level.clear();
            1.0
        };
        let gain = norm * vg * trem * swell * level;
        v.gain_inc = (gain - v.gain) / CTRL as f32;
    }

    // ---- audio rate ----

    fn render(&mut self, from: usize, to: usize, left: &mut [f32], right: &mut [f32]) {
        let s = self.sh;
        let mode = FilterMode::from_index(f32::from(s.mode));
        let tabs = (&*self.tables[0], &*self.tables[1]);
        for v in &mut self.voices {
            if v.state == State::Free {
                continue;
            }
            // Mono: every unison voice panned the same both ways and both
            // filters in the same state, so the right side would come out
            // equal to the left bit for bit; work it out once. Each case
            // has its own copy of the loop, so stereo runs as before.
            let mono = (0..s.unison).all(|u| v.pan_l[u].to_bits() == v.pan_r[u].to_bits())
                && v.filt[0].same_as(&v.filt[1]);
            let out = (&mut *left, &mut *right);
            if mono {
                run_voice::<true>(v, &s, mode, tabs, &mut self.rng, from..to, out);
                v.filt[1] = v.filt[0];
            } else {
                run_voice::<false>(v, &s, mode, tabs, &mut self.rng, from..to, out);
            }
        }
    }
}

/// One voice over frames `span` into `out` (left, right). With `MONO` the
/// right side is the left one (see `render`) and its filter isn't run.
/// Kept out of line, so each copy is optimised on its own.
#[inline(never)]
fn run_voice<const MONO: bool>(
    v: &mut Voice,
    s: &Shared,
    mode: FilterMode,
    (ta, tb): (&Table, &Table),
    rng: &mut Rng,
    span: std::ops::Range<usize>,
    (left, right): (&mut [f32], &mut [f32]),
) {
    let (wa, wb) = (v.warp[0], v.warp[1]);
    // B is read when heard or when it frequency-modulates A; A when
    // heard or when it modulates B.
    let use_b = s.lvl[1] != 0.0 || matches!(wa, Warp::Fm { .. });
    let use_a = s.lvl[0] != 0.0 || matches!(wb, Warp::Fm { .. });
    let da = ta.levels()[v.level[0]].data();
    let db = tb.levels()[v.level[1]].data();
    let (use_sub, use_noise) = (s.sub != 0.0, s.noise != 0.0);
    // The drive's gain and level scale, once per run rather than per
    // sample: it sees the mix as one full oscillator, however many stacks
    // and sources feed it (dsp::drive_scales).
    let (dg, dn, damt) = if v.drive_amt > 0.0 {
        let lv = mix_level(s.unison, s.lvl, s.sub, s.noise);
        let (k, n) = drive_scales(v.drive_gain, lv);
        (k, n, v.drive_amt)
    } else {
        (1.0, 1.0, 0.0)
    };
    for k in span {
        let (ra, rb) = (v.run[0], v.run[1]);
        let mut l = 0.0;
        let mut r = 0.0;
        for u in 0..s.unison {
            let mut b = 0.0;
            if use_b {
                let ph = v.ph[1][u];
                b = rb.read(db, wb.apply(ph, v.last[0][u]));
                v.last[1][u] = b;
                let n = ph + v.dt[1][u];
                v.ph[1][u] = if n >= 1.0 { n - 1.0 } else { n };
            }
            let mut a = 0.0;
            if use_a {
                let ph = v.ph[0][u];
                a = ra.read(da, wa.apply(ph, b));
                v.last[0][u] = a;
                let n = ph + v.dt[0][u];
                v.ph[0][u] = if n >= 1.0 { n - 1.0 } else { n };
            }
            let x = a * s.lvl[0] + b * s.lvl[1];
            l += x * v.pan_l[u];
            if !MONO {
                r += x * v.pan_r[u];
            }
        }
        v.run[0].t += ra.t_inc;
        v.run[1].t += rb.t_inc;
        let mut c = 0.0;
        if use_sub {
            c += osc(Wave::Sine, &mut v.ph_sub, v.dt_sub, 0.5) * s.sub;
        }
        if use_noise {
            c += rng.bipolar() * s.noise;
        }
        // The filter, then the drive (as Poly's: after the filter, so a
        // low cutoff doesn't take out the grit).
        let x = v.filt[0].process(mode, &v.coefs, l + c);
        let l = drive_n(x, dg, dn, damt);
        if damt > 0.0 {
            v.drive_level.add(x, l);
        }
        let r = if MONO {
            l
        } else {
            let r = v.filt[1].process(mode, &v.coefs, r + c);
            drive_n(r, dg, dn, damt)
        };
        let mut g = v.amp.next() * v.gain;
        v.gain += v.gain_inc;
        if v.state == State::Kill {
            v.kill_gain -= v.kill_step;
            if v.kill_gain <= 0.0 {
                v.kill_gain = 0.0;
                v.state = State::Free;
            }
            g *= v.kill_gain;
        }
        left[k] += l * g * s.out_l;
        right[k] += r * g * s.out_r;
        if v.state == State::Free {
            break;
        }
        if v.amp.idle() && v.state != State::On {
            v.state = State::Free;
            break;
        }
    }
}

impl Node for Wavetable {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }
    fn params(&self) -> &'static [ParamInfo] {
        PARAMS
    }

    fn set_param(&mut self, id: u16, value: f32) {
        let Some(p) = info(id) else { return };
        if p.readonly {
            return;
        }
        self.values[id as usize] = p.clamp(value);
        self.update_targets();
        if matches!(
            id,
            AMP_A | AMP_D | AMP_S | AMP_R | MENV_A | MENV_D | MENV_S | MENV_R
        ) {
            self.update_envs();
        }
        if id == TABLE || id == OSC_B + TABLE {
            self.pick_tables();
        }
        if id >= PRESS_BASE {
            self.update_pressure();
        }
        if id == VOICE_MODE {
            self.all_off();
        }
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
        self.update_pressure();
        self.reset_smoothers();
        self.control(&Ctx::new(self.rate, 0), 0);
    }

    fn reset(&mut self) {
        for v in &mut self.voices {
            *v = Voice::new();
        }
        self.update_envs();
        self.reset_smoothers();
        self.stack_len = 0;
        self.exprs.clear();
        self.lfo_phase = 0.0;
        self.vib_phase = 0.0;
        self.chord.clear();
        self.meter_voices = 0.0;
        self.control(&Ctx::new(self.rate, 0), 0);
    }

    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        assert_eq!(left.len(), right.len());
        if ctx.rate != self.rate {
            self.prepare(ctx.rate, left.len());
        }
        let len = left.len();
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
                if abs.is_multiple_of(CTRL) {
                    self.control(ctx, at);
                }
                let end = next.min(at + (CTRL - abs % CTRL) as usize);
                self.render(at, end, left, right);
                at = end;
            }
        }
        for e in &events[i..] {
            self.apply(&e.ev);
        }
        self.meter_voices = self.voices.iter().filter(|v| v.sounding()).count() as f32;
    }

    fn tail_frames(&self) -> u32 {
        let ms = self.v(AMP_R).max(KILL_MS);
        (ms * 0.001 * self.rate as f32 * 1.5) as u32
    }

    fn set_tuning(&mut self, tuning: &Tuning) {
        // Voices work their pitch out every control step, so the new tuning
        // reaches them on the next one.
        if self.tuning != *tuning {
            self.tuning.clone_from(tuning);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_are_sound() {
        mpfree_node::check_params(PARAMS).unwrap();
        for p in PARAMS {
            assert!((p.id as usize) < ID_SPACE);
        }
        for preset in PRESETS {
            for &(id, _) in preset.values {
                assert!(info(id).is_some(), "{}: id {id}", preset.name);
            }
        }
    }

    #[test]
    fn warps_stay_in_the_cycle() {
        for mode in 0..5u8 {
            for amt in [0.0, 0.3, 1.0] {
                let w = Warp::new(mode, amt);
                for k in 0..1000 {
                    let p = k as f32 / 1000.0;
                    let q = w.apply(p, (k as f32 * 0.37).sin());
                    assert!((0.0..1.0).contains(&q), "mode {mode} amt {amt} p {p}: {q}");
                }
            }
        }
        assert_eq!(Warp::new(1, 0.0).apply(0.3, 0.0), 0.3);
    }
}
