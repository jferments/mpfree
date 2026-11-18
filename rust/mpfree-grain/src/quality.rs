//! Quality modes (the granular roadmap's step C5, the plan's decision 4:
//! grit in the engine): how a voice's cloud is coloured on its way out,
//! after its level is set.
//!
//! - **Clean** (the default) leaves it as it is, bit for bit: the voice
//!   doesn't run this stage at all.
//! - **Tape** plays it off a short tape loop: a delay of about a
//!   millisecond whose length moves with wow (0.6 Hz, with a slower
//!   0.37 Hz under it) and flutter (7 Hz), so the pitch wavers by about
//!   ±5 cents, then a gentle saturation (shaped as `tanh`, so quiet
//!   sounds pass at their level and only loud peaks round off) and a one-pole roll-off
//!   above 12 kHz. On the Grains effect the cloud goes back into the
//!   memory through it, so a feedback trail wobbles more each time round,
//!   as a tape echo's does.
//! - **Lo-fi** is cold and digital (Beads' third mode): each sample held
//!   for about a quarter of 44.1 kHz (11 kHz, with no filter first, so
//!   the highs alias on purpose) and squeezed to 8 bits through a µ-law
//!   curve, so quiet tails step and loud ones crunch, while silence stays
//!   silent.
//!
//! Everything moves with the voice's own clock (the wow's place, the
//! lo-fi's holds), so a voice sounds the same however its audio is cut
//! into periods. A switch from one mode to another crossfades over 10 ms,
//! running both, so it never clicks. The stage keeps its memory in fixed
//! arrays (a 256-frame tape, about 2 KB a voice): nothing allocates.

use crate::hermite;

/// How a voice's cloud is coloured on its way out ([`crate::Params::quality`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Quality {
    /// As it is (the default, bit for bit as before quality modes).
    #[default]
    Clean,
    /// Wow and flutter, a soft saturation and a high roll-off.
    Tape,
    /// Rate and bit reduction: 11 kHz and 8-bit µ-law, aliasing on purpose.
    LoFi,
}

impl Quality {
    /// Every mode, in the order a page steps through them.
    pub const ALL: [Quality; 3] = [Quality::Clean, Quality::Tape, Quality::LoFi];

    /// Its name on a page.
    pub fn name(self) -> &'static str {
        match self {
            Quality::Clean => "Clean",
            Quality::Tape => "Tape",
            Quality::LoFi => "Lo-fi",
        }
    }

    /// Its place in [`Quality::ALL`].
    pub fn index(self) -> usize {
        match self {
            Quality::Clean => 0,
            Quality::Tape => 1,
            Quality::LoFi => 2,
        }
    }

    /// The mode at `i` in [`Quality::ALL`] (Clean past the end).
    pub fn from_index(i: usize) -> Quality {
        Quality::ALL.get(i).copied().unwrap_or_default()
    }
}

/// Frames the tape holds (a power of two): room for the longest delay up
/// to 96 kHz; at higher rates the wow is scaled down to fit.
const TAPE_LEN: usize = 256;
const TAPE_MASK: usize = TAPE_LEN - 1;
/// The tape's speed wanders by these (Hz, and the fraction of the speed at
/// most either way): wow, a slower wow under it so the wobble doesn't
/// repeat exactly every 1.7 s, and flutter.
const WOBBLES: [(f64, f64); 3] = [(0.6, 0.003), (0.37, 0.001), (7.0, 0.0005)];
/// The tape's high roll-off (one pole), Hz.
const ROLLOFF_HZ: f64 = 12_000.0;
/// The delay's length is worked out at the edges of each run of this many
/// frames of the voice's clock and drawn straight between them.
const SEG: u64 = 64;
/// The lo-fi rate, Hz: each sample is held for the rate over this many
/// frames, rounded (4 at 44.1 and 48 kHz).
const LOFI_HZ: f64 = 11_025.0;
/// µ-law's µ, and the steps either side of zero (8 bits).
const MU: f32 = 255.0;
const STEPS: f32 = 127.0;
/// How long a switch from one mode to another crossfades, seconds.
const SWITCH_SECS: f64 = 0.01;

/// The tape: its two channels, where the next frame goes, the roll-off's
/// state, and the delay's shape in frames.
#[derive(Clone, Debug)]
struct Tape {
    l: [f32; TAPE_LEN],
    r: [f32; TAPE_LEN],
    w: usize,
    /// Starting again: the first frame fills the tape, as if it had been
    /// there all along, so the delayed sound comes in without a step.
    fresh: bool,
    lp: [f32; 2],
    k_lp: f32,
    /// The delay at rest, and each wobble's depth (frames) and rate
    /// (cycles a frame).
    base: f64,
    wobbles: [(f64, f64); 3],
    /// The run of [`SEG`] frames the delay was last worked out for: its
    /// first frame, and the delay at its start and end.
    seg: Option<u64>,
    d0: f32,
    d1: f32,
}

impl Tape {
    fn new(rate: u32) -> Tape {
        let rate = f64::from(rate.max(1));
        // A sine delay of depth A seconds at f Hz moves the speed by
        // 2 pi f A at most.
        let mut depth: f64 = WOBBLES
            .iter()
            .map(|&(hz, by)| by / (std::f64::consts::TAU * hz) * rate)
            .sum();
        // Room for the deepest dip and the four frames read around it,
        // never nearer than two frames to the newest.
        let fit = ((TAPE_LEN - 4) as f64 / 2.0 / (depth + 2.0)).min(1.0);
        depth *= fit;
        let mut wobbles = [(0.0, 0.0); 3];
        for (w, &(hz, by)) in wobbles.iter_mut().zip(&WOBBLES) {
            *w = (by / (std::f64::consts::TAU * hz) * rate * fit, hz / rate);
        }
        let k_lp = 1.0 - (-std::f64::consts::TAU * ROLLOFF_HZ.min(0.45 * rate) / rate).exp();
        Tape {
            l: [0.0; TAPE_LEN],
            r: [0.0; TAPE_LEN],
            w: 0,
            fresh: true,
            lp: [0.0; 2],
            k_lp: k_lp as f32,
            base: depth + 2.0,
            wobbles,
            seg: None,
            d0: 0.0,
            d1: 0.0,
        }
    }

    fn restart(&mut self) {
        self.fresh = true;
    }

    /// The delay, in frames, at frame `c` of the voice's clock.
    fn delay_at(&self, c: u64) -> f32 {
        let mut d = self.base;
        for &(depth, cycles) in &self.wobbles {
            // The phase from the clock, kept small so it stays exact.
            let phase = (c as f64 * cycles).fract();
            d += depth * (std::f64::consts::TAU * phase).sin();
        }
        d as f32
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32], clock: u64) {
        let n = l.len().min(r.len());
        if self.fresh && n > 0 {
            self.fresh = false;
            self.l = [l[0]; TAPE_LEN];
            self.r = [r[0]; TAPE_LEN];
            self.lp = [soft(l[0]), soft(r[0])];
        }
        for i in 0..n {
            let c = clock + i as u64;
            let seg = c - c % SEG;
            if self.seg != Some(seg) {
                self.seg = Some(seg);
                self.d0 = self.delay_at(seg);
                self.d1 = self.delay_at(seg + SEG);
            }
            let d = self.d0 + (self.d1 - self.d0) * ((c - seg) as f32 / SEG as f32);
            self.l[self.w] = l[i];
            self.r[self.w] = r[i];
            // Four-point Hermite between the frames `d` back, so the
            // moving read keeps the highs (a straight line between two
            // would dull them by up to 3 dB at 10 kHz as it moves).
            let k = d as usize;
            let t = 1.0 - (d - k as f32);
            let x2 = (self.w + TAPE_LEN - k) & TAPE_MASK;
            let (x0, x1, x3) = (
                (x2 + TAPE_LEN - 2) & TAPE_MASK,
                (x2 + TAPE_MASK) & TAPE_MASK,
                (x2 + 1) & TAPE_MASK,
            );
            let yl = hermite(self.l[x0], self.l[x1], self.l[x2], self.l[x3], t);
            let yr = hermite(self.r[x0], self.r[x1], self.r[x2], self.r[x3], t);
            self.w = (self.w + 1) & TAPE_MASK;
            self.lp[0] += self.k_lp * (soft(yl) - self.lp[0]);
            self.lp[1] += self.k_lp * (soft(yr) - self.lp[1]);
            l[i] = self.lp[0];
            r[i] = self.lp[1];
        }
    }
}

/// A gentle saturation shaped as `tanh` (its 3/2 Padé fit, within 2 % of
/// it, levelling off smoothly at 1 at 3), so it passes small signals at
/// their level.
#[inline]
fn soft(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

/// Lo-fi: what is held, and for how many frames.
#[derive(Clone, Debug)]
struct LoFi {
    hold: u32,
    held: [f32; 2],
}

impl LoFi {
    fn new(rate: u32) -> LoFi {
        LoFi {
            hold: (f64::from(rate) / LOFI_HZ).round().max(1.0) as u32,
            held: [0.0; 2],
        }
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32], clock: u64) {
        let n = l.len().min(r.len());
        // A new sample on every frame of the voice's clock that the hold
        // divides.
        let mut k = (clock % u64::from(self.hold)) as u32;
        for i in 0..n {
            if k == 0 {
                self.held = [crush(l[i]), crush(r[i])];
            }
            k += 1;
            if k == self.hold {
                k = 0;
            }
            l[i] = self.held[0];
            r[i] = self.held[1];
        }
    }
}

/// 8-bit µ-law: compressed, rounded to one of 255 steps and expanded
/// again. Full scale clips; zero stays zero.
#[inline]
fn crush(x: f32) -> f32 {
    let ln_mu = (1.0 + MU).ln();
    let a = x.abs().min(1.0);
    let q = ((1.0 + MU * a).ln() / ln_mu * STEPS).round() / STEPS;
    (((q * ln_mu).exp() - 1.0) / MU).copysign(x)
}

/// A voice's quality stage: the mode it plays, the one it is fading from
/// after a switch, and a switch asked for while one fades (it follows).
#[derive(Clone, Debug)]
pub(crate) struct Stage {
    to: Quality,
    from: Quality,
    next: Option<Quality>,
    /// Frames left of the switch's crossfade, and its length.
    left: u32,
    len: u32,
    tape: Tape,
    lofi: LoFi,
}

/// The longest run [`Stage::process`] takes.
pub(crate) const RUN: usize = 64;

impl Stage {
    pub(crate) fn new(rate: u32) -> Stage {
        Stage {
            to: Quality::Clean,
            from: Quality::Clean,
            next: None,
            left: 0,
            len: ((SWITCH_SECS * f64::from(rate.max(1))).round() as u32).max(1),
            tape: Tape::new(rate),
            lofi: LoFi::new(rate),
        }
    }

    /// Clean, with no switch under way: the voice can skip the stage.
    pub(crate) fn is_clean(&self) -> bool {
        self.to == Quality::Clean && self.left == 0 && self.next.is_none()
    }

    /// Plays `q` from now, crossfading from what it played (after the
    /// crossfade under way, if there is one).
    pub(crate) fn set(&mut self, q: Quality) {
        if self.left > 0 {
            self.next = (q != self.to).then_some(q);
        } else {
            self.begin(q);
        }
    }

    fn begin(&mut self, q: Quality) {
        if q == self.to {
            return;
        }
        // A mode coming in starts afresh.
        match q {
            Quality::Tape => self.tape.restart(),
            Quality::LoFi => self.lofi.held = [0.0; 2],
            Quality::Clean => {}
        }
        self.from = self.to;
        self.to = q;
        self.left = self.len;
    }

    /// Plays `q` from now with no crossfade (the voice is silent), and
    /// nothing held from before.
    pub(crate) fn jump(&mut self, q: Quality) {
        self.tape.restart();
        self.lofi.held = [0.0; 2];
        self.left = 0;
        self.next = None;
        self.to = q;
        self.from = q;
    }

    /// Colours `l` and `r` in place (at most [`RUN`] frames); `clock` is
    /// the voice's clock at their first frame.
    pub(crate) fn process(&mut self, l: &mut [f32], r: &mut [f32], clock: u64) {
        let n = l.len().min(r.len()).min(RUN);
        let mut at = 0;
        while at < n {
            if self.left == 0 {
                if let Some(q) = self.next.take() {
                    self.begin(q);
                    continue;
                }
                let c = clock + at as u64;
                self.run(self.to, &mut l[at..n], &mut r[at..n], c);
                return;
            }
            // The crossfade's frames: both modes, the old fading out.
            let k = (n - at).min(self.left as usize);
            let c = clock + at as u64;
            let (mut tl, mut tr) = ([0.0f32; RUN], [0.0f32; RUN]);
            tl[..k].copy_from_slice(&l[at..at + k]);
            tr[..k].copy_from_slice(&r[at..at + k]);
            self.run(self.from, &mut tl[..k], &mut tr[..k], c);
            self.run(self.to, &mut l[at..at + k], &mut r[at..at + k], c);
            let len = self.len as f32;
            for i in 0..k {
                self.left -= 1;
                let t = 1.0 - self.left as f32 / len;
                l[at + i] = tl[i] + t * (l[at + i] - tl[i]);
                r[at + i] = tr[i] + t * (r[at + i] - tr[i]);
            }
            at += k;
        }
    }

    fn run(&mut self, q: Quality, l: &mut [f32], r: &mut [f32], clock: u64) {
        match q {
            Quality::Clean => {}
            Quality::Tape => self.tape.run(l, r, clock),
            Quality::LoFi => self.lofi.run(l, r, clock),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_passes_quiet_sounds_and_rounds_loud_ones() {
        assert!((soft(0.01) - 0.01).abs() < 1e-6);
        assert!((soft(0.25) - 0.25f32.tanh()).abs() < 1e-3);
        assert!((soft(1.0) - 1.0f32.tanh()).abs() < 0.02);
        // It levels off smoothly: no corner at 3.
        assert!(soft(2.9) < 1.0 && soft(2.9) > 0.9999);
        assert_eq!(soft(5.0), 1.0);
        assert_eq!(soft(-5.0), -1.0);
    }

    #[test]
    fn crush_keeps_silence_and_steps_in_255() {
        assert_eq!(crush(0.0), 0.0);
        assert_eq!(crush(1.0), 1.0);
        assert_eq!(crush(2.0), 1.0);
        assert_eq!(crush(-0.5), -crush(0.5));
        // Near the step it rounds to.
        assert!((crush(0.5) - 0.5).abs() < 0.03);
        let mut seen = Vec::new();
        for i in -1000..=1000 {
            let y = crush(i as f32 / 1000.0);
            if !seen.contains(&y.to_bits()) {
                seen.push(y.to_bits());
            }
        }
        assert!(seen.len() <= 255, "{} steps", seen.len());
    }

    #[test]
    fn the_tape_fits_at_every_rate() {
        for rate in [8_000, 22_050, 44_100, 48_000, 96_000, 192_000] {
            let t = Tape::new(rate);
            let mut most: f32 = 0.0;
            let mut least: f32 = f32::MAX;
            for c in (0..rate as u64 * 4).step_by(7) {
                let d = t.delay_at(c);
                most = most.max(d);
                least = least.min(d);
            }
            assert!(least >= 1.0, "{rate}: {least}");
            assert!(most <= (TAPE_LEN - 2) as f32, "{rate}: {most}");
        }
    }

    #[test]
    fn a_switch_crossfades_and_clean_is_untouched_after() {
        let mut s = Stage::new(44_100);
        assert!(s.is_clean());
        s.set(Quality::LoFi);
        assert!(!s.is_clean());
        let mut clock = 0;
        while s.left > 0 {
            let (mut l, mut r) = ([0.3f32; 64], [0.3f32; 64]);
            s.process(&mut l, &mut r, clock);
            clock += 64;
        }
        s.set(Quality::Clean);
        let (mut l, mut r) = ([0.3f32; 64], [0.3f32; 64]);
        s.process(&mut l, &mut r, clock);
        // Fading back: between the lo-fi's step and the input.
        assert!(l.iter().all(|x| (x - 0.3).abs() < 0.01));
        while s.left > 0 {
            let (mut l, mut r) = ([0.3f32; 64], [0.3f32; 64]);
            s.process(&mut l, &mut r, clock);
            clock += 64;
        }
        assert!(s.is_clean());
    }

    #[test]
    fn lofi_holds_on_the_clock_whatever_the_runs() {
        let input: Vec<f32> = (0..256).map(|i| (i as f32 * 0.37).sin() * 0.5).collect();
        let run = |cut: usize| {
            let mut s = LoFi::new(44_100);
            let mut l = input.clone();
            let mut r = input.clone();
            let mut at = 0;
            while at < l.len() {
                let k = cut.min(l.len() - at);
                let (a, b) = (&mut l[at..at + k], &mut r[at..at + k]);
                s.run(a, b, 1000 + at as u64);
                at += k;
            }
            l
        };
        assert_eq!(run(64), run(7));
        assert_eq!(run(64), run(1));
    }
}
