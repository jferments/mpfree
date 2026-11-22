//! A cursor: one voice's place in a source. Several cursors can read one
//! source at once, sharing its cached units.
//!
//! A cursor seeks to any frame, plays at a speed (file frames moved per
//! output frame: the file's rate over the output's, times the musical
//! speed) and goes round a loop. Each period it gathers the file frames it
//! will pass, round the loop, from the [`Player`]'s cache into its own
//! buffer, with the neighbours its read looks at, then reads them
//! ([`Read`]). By default the read is band-limited, as the sampler's own
//! clean read is: a Kaiser-windowed sinc cut at the output's Nyquist
//! frequency, so a sound played faster doesn't fold back down as
//! aliasing. 4-point Hermite stays as the cheaper choice. At speed 1 from
//! a whole frame either gives the file's own samples, bit for bit.
//!
//! A cursor can stop at an end frame (a region that is part of a take):
//! frames from there on are silent, not counted and not read ahead. A
//! start is a seek, and a start partway into a period is a call with the
//! rest of the period's buffer.
//!
//! Output is always stereo, interleaved; a mono source plays in both.
//! Frames whose unit isn't cached play silence and are counted as missed;
//! the neighbours the interpolation looks at are not counted.
//!
//! Nothing here allocates once the cursor is made.

use std::sync::OnceLock;

use crate::msg::Place;
use crate::player::Player;
use crate::resample::Kernel;

/// The fastest a cursor moves: four file frames an output frame.
pub const MAX_SPEED: f64 = 4.0;

/// Output frames handled in one pass; longer calls go in passes.
pub const PASS: usize = 1024;

/// How a cursor reads between frames and at speeds other than 1. The
/// sampler sets it from what the sound thread can afford beside
/// everything else; it never changes by itself under load.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Read {
    /// 4-point Hermite: the cheapest, but a faster read folds what it
    /// moves past the output's Nyquist frequency back down.
    Hermite,
    /// Band-limited, 8 zero crossings a side: the sampler's own clean
    /// read. Its cut follows the speed up to twice ([`MAX_STRETCH`]);
    /// faster than that it filters a little less. 16 to 32 taps a frame.
    #[default]
    Sinc8,
    /// The same with 16 crossings a side: a sharper cut for twice the
    /// work.
    Sinc16,
}

/// The most a band-limited read stretches its sinc: past twice the speed
/// it stays there, so a read never costs more than twice its taps at
/// speed 1.
pub const MAX_STRETCH: f64 = 2.0;

/// Table steps per crossing of the voice reads' kernels.
const STEPS: usize = 128;

fn kernel(read: Read) -> Option<&'static Kernel> {
    static K8: OnceLock<Kernel> = OnceLock::new();
    static K16: OnceLock<Kernel> = OnceLock::new();
    match read {
        Read::Hermite => None,
        Read::Sinc8 => Some(K8.get_or_init(|| Kernel::new(8, 5.0, STEPS))),
        Read::Sinc16 => Some(K16.get_or_init(|| Kernel::new(16, 6.0, STEPS))),
    }
}

/// Frames the widest read looks at on each side.
const MAX_REACH: usize = 16 * MAX_STRETCH as usize;

/// File frames gathered for one pass, at most: the frames passed and the
/// read's neighbours on each side.
const GATHER: usize = (PASS as f64 * MAX_SPEED) as usize + 2 * MAX_REACH + 4;

pub struct Cursor {
    source: u16,
    id: u16,
    /// The file frame it plays next, with its fraction.
    pos: u64,
    frac: f64,
    speed: f64,
    looping: Option<(u64, u64)>,
    end: Option<u64>,
    read: Read,
    /// Gathered frames, interleaved in the source's channels.
    gathered: Box<[f32]>,
}

impl Cursor {
    /// A cursor on `source` at frame 0, at speed 1, with no loop. `id`
    /// must be unique among the cursors playing at once and below
    /// [`crate::msg::MAX_CURSORS`]; it is how the coordinator tells two
    /// cursors on one source apart. Allocates its buffer: make cursors off
    /// the sound thread.
    pub fn new(source: u16, id: u16) -> Cursor {
        // Make the reads' tables here, never on the sound thread.
        kernel(Read::Sinc8);
        kernel(Read::Sinc16);
        Cursor {
            source,
            id,
            pos: 0,
            frac: 0.0,
            speed: 1.0,
            looping: None,
            end: None,
            read: Read::default(),
            gathered: vec![0.0; GATHER * 2].into_boxed_slice(),
        }
    }

    pub fn source(&self) -> u16 {
        self.source
    }

    pub fn id(&self) -> u16 {
        self.id
    }

    /// Point it at another source (or the same one again) at frame 0.
    pub fn set_source(&mut self, source: u16) {
        self.source = source;
        self.seek(0);
    }

    /// The whole file frame it plays next.
    pub fn frame(&self) -> u64 {
        self.pos
    }

    /// Where it plays next, with the fraction.
    pub fn position(&self) -> f64 {
        self.pos as f64 + self.frac
    }

    /// Play from `frame` next, exactly.
    pub fn seek(&mut self, frame: u64) {
        self.pos = frame;
        self.frac = 0.0;
    }

    pub fn speed(&self) -> f64 {
        self.speed
    }

    /// File frames moved per output frame, above 0 and at most
    /// [`MAX_SPEED`]; anything else (0, negative, not a number) is 1.
    pub fn set_speed(&mut self, speed: f64) {
        self.speed = if speed.is_finite() && speed > 0.0 {
            speed.min(MAX_SPEED)
        } else {
            1.0
        };
    }

    pub fn read(&self) -> Read {
        self.read
    }

    /// How it reads between frames ([`Read`]).
    pub fn set_read(&mut self, read: Read) {
        self.read = read;
    }

    pub fn looping(&self) -> Option<(u64, u64)> {
        self.looping
    }

    /// Go round frames `start..end` once the cursor is inside them (a
    /// cursor before the loop plays into it; one past its end plays on).
    /// An empty loop is no loop.
    pub fn set_loop(&mut self, looping: Option<(u64, u64)>) {
        self.looping = looping.filter(|&(s, e)| e > s);
    }

    pub fn end(&self) -> Option<u64> {
        self.end
    }

    /// Stop at file frame `end`: from there on it plays silence, not
    /// counted as missed, and nothing past it is read ahead.
    pub fn set_end(&mut self, end: Option<u64>) {
        self.end = end;
    }

    /// Where it plays and how it moves, as posted to the coordinator.
    pub fn place(&self) -> Place {
        Place {
            frame: self.pos,
            speed: self.speed,
            looping: self.looping,
            end: self.end,
        }
    }

    /// The file frame that `frame` (counted on from the cursor without
    /// wrapping) lands on, round the loop if the cursor is inside it.
    fn wrap(&self, frame: u64) -> u64 {
        match self.looping {
            Some((s, e)) if self.pos < e && frame >= e => s + (frame - e) % (e - s),
            _ => frame,
        }
    }

    /// Gather file frames `first..first + n` (round the loop) into the
    /// buffer from `at` (frames), counting what's missed only if `count`.
    fn gather(&mut self, p: &mut Player, first: u64, n: usize, at: usize, count: bool) -> u64 {
        let ch = match p.shape(self.source) {
            Some((c, _)) => usize::from(c),
            None => return 0,
        };
        let mut missed = 0;
        let mut k = 0;
        while k < n {
            let f = self.wrap(first + k as u64);
            // A run of frames that follow each other in the file: up to the
            // loop's end when the cursor goes round it.
            let run_end = match self.looping {
                Some((_, e)) if self.pos < e && f < e => e - f,
                _ => u64::MAX,
            };
            let mut run = (n - k).min(usize::try_from(run_end).unwrap_or(usize::MAX));
            let stop = self.end.unwrap_or(u64::MAX);
            if f >= stop {
                // Past the end: silence, the rest of this run too.
                self.gathered[(at + k) * ch..(at + k + run) * ch].fill(0.0);
                k += run;
                continue;
            }
            run = run.min(usize::try_from(stop - f).unwrap_or(usize::MAX));
            let out = &mut self.gathered[(at + k) * ch..(at + k + run) * ch];
            missed += if count {
                p.read(self.source, f, out)
            } else {
                p.peek(self.source, f, out)
            };
            k += run;
        }
        missed
    }

    /// Play `out.len() / 2` frames into `out` (stereo, interleaved), and
    /// note where the cursor plays for the coordinator. A source not open
    /// plays silence. Gives the file frames missed.
    pub fn play(&mut self, p: &mut Player, out: &mut [f32]) -> u64 {
        let Some((channels, _)) = p.shape(self.source) else {
            out.fill(0.0);
            return 0;
        };
        p.playing(self.source, self.id, self.place());
        if out.len() % 2 == 1 {
            out[out.len() - 1] = 0.0;
        }
        let ch = usize::from(channels);
        let mut missed = 0;
        for part in out.chunks_mut(PASS * 2) {
            let n = part.len() / 2;
            // Frames passed this pass: from pos to the last one read.
            let last = self.frac + (n.saturating_sub(1)) as f64 * self.speed;
            let span = last.floor() as usize + 1;
            // Neighbours the read looks at before pos and after the last.
            let (lead, tail) = match kernel(self.read) {
                None => (1, 2),
                Some(k) => {
                    let r = reach(k, self.speed);
                    (r, r + 1)
                }
            };
            self.gather_before(p, lead);
            missed += self.gather(p, self.pos, span, lead, true);
            self.gather(p, self.pos + span as u64, tail, lead + span, false);
            match kernel(self.read) {
                _ if self.speed == 1.0 && self.frac == 0.0 => self.copy(ch, lead, part),
                None => self.interpolate(ch, part),
                Some(k) => self.sinc(k, ch, lead, part),
            }
            // Move on.
            let moved = self.frac + n as f64 * self.speed;
            let whole = moved.floor();
            self.frac = moved - whole;
            self.pos = self.wrap(self.pos + whole as u64);
        }
        missed
    }

    fn gather_one(&mut self, p: &mut Player, frame: u64, at: usize) {
        let ch = match p.shape(self.source) {
            Some((c, _)) => usize::from(c),
            None => return,
        };
        let out = &mut self.gathered[at * ch..(at + 1) * ch];
        p.peek(self.source, frame, out);
    }

    /// The `lead` frames played just before pos, into the buffer's first
    /// `lead` places: silence before the file's start and past its end.
    /// Inside a loop, frames before its start come from its end, as if
    /// the cursor had come round (the first time in, that's a few
    /// neighbours' worth of the loop's end instead of what led into it).
    fn gather_before(&mut self, p: &mut Player, lead: usize) {
        let ch = match p.shape(self.source) {
            Some((c, _)) => usize::from(c),
            None => return,
        };
        let inside = self.looping.filter(|&(s, e)| self.pos >= s && self.pos < e);
        let plain = match inside {
            Some((s, _)) => self.pos - s >= lead as u64,
            None => self.pos >= lead as u64,
        };
        if plain && self.end.is_none_or(|e| self.pos <= e) {
            // Frames pos - lead..pos, all before pos: no wrapping.
            let first = self.pos - lead as u64;
            for k in 0..lead {
                self.gather_one(p, first + k as u64, k);
            }
            return;
        }
        for k in 0..lead {
            let back = (lead - k) as u64;
            let frame = match inside {
                Some((s, e)) if back > self.pos - s => {
                    Some(e - 1 - (back - (self.pos - s) - 1) % (e - s))
                }
                _ => self.pos.checked_sub(back),
            };
            match frame {
                Some(f) if self.end.is_none_or(|e| f < e) => self.gather_one(p, f, k),
                _ => self.gathered[k * ch..(k + 1) * ch].fill(0.0),
            }
        }
    }

    /// At speed 1 from a whole frame: the gathered frames themselves.
    fn copy(&self, ch: usize, lead: usize, out: &mut [f32]) {
        let g = &self.gathered;
        for (j, o) in out.chunks_exact_mut(2).enumerate() {
            for (c, slot) in o.iter_mut().enumerate() {
                *slot = g[(lead + j) * ch + c.min(ch - 1)];
            }
        }
    }

    /// Read the gathered frames (the first is the frame before pos) into
    /// `out`, stereo.
    fn interpolate(&self, ch: usize, out: &mut [f32]) {
        let g = &self.gathered;
        for (j, o) in out.chunks_exact_mut(2).enumerate() {
            // The same sum as `last` in play, so it never reads past it.
            let x = self.frac + j as f64 * self.speed;
            let i = x as usize;
            let t = (x - i as f64) as f32;
            for (c, slot) in o.iter_mut().enumerate() {
                let c = c.min(ch - 1);
                let y = |k: usize| g[(i + k) * ch + c];
                *slot = hermite(y(0), y(1), y(2), y(3), t);
            }
        }
    }

    /// The band-limited read of the gathered frames (pos is at `lead`)
    /// into `out`, stereo: each frame a sum of its neighbours weighted by
    /// the sinc, stretched by the speed (up to [`MAX_STRETCH`]) so it cuts
    /// at the output's Nyquist frequency, and weights summed to 1. The
    /// taps walk the table in fixed point, half a side at a time.
    fn sinc(&self, k: &Kernel, ch: usize, lead: usize, out: &mut [f32]) {
        const ONE: f64 = (1u64 << 16) as f64;
        const INV: f32 = 1.0 / (1u32 << 16) as f32;
        let g = &self.gathered;
        let tab = k.table();
        let mask = tab.len() - 1;
        let stretch = self.speed.clamp(1.0, MAX_STRETCH);
        let r = reach(k, self.speed);
        // Table steps between neighbouring frames, in fixed point.
        let per = k.steps() as f64 / stretch;
        let step = (per * ONE) as u32;
        let last = ch - 1;
        let tap = |pos: u32| {
            let i = (pos >> 16) as usize;
            let a = tab[i & mask];
            a + (pos & 0xFFFF) as f32 * INV * (tab[(i + 1) & mask] - a)
        };
        for (j, o) in out.chunks_exact_mut(2).enumerate() {
            let x = self.frac + j as f64 * self.speed;
            let i = x as usize;
            let t = x - i as f64;
            // The frames round the place: r before it and up to it, then r
            // after.
            let at = (lead + i + 1 - r) * ch;
            let win = &g[at..at + 2 * r * ch];
            let (before, after) = win.split_at(r * ch);
            let (mut a, mut b, mut total) = (0.0f32, 0.0f32, 0.0f32);
            // Frame i, i - 1, ...: t, t + 1, ... frames from the place.
            let mut pos = (t * per * ONE) as u32;
            for f in before.chunks_exact(ch).rev() {
                let w = tap(pos);
                a += w * f[0];
                b += w * f[last];
                total += w;
                pos += step;
            }
            // Frame i + 1, i + 2, ...: 1 - t, 2 - t, ... frames from it.
            let mut pos = ((1.0 - t) * per * ONE) as u32;
            for f in after.chunks_exact(ch) {
                let w = tap(pos);
                a += w * f[0];
                b += w * f[last];
                total += w;
                pos += step;
            }
            let norm = if total == 0.0 { 0.0 } else { 1.0 / total };
            o[0] = a * norm;
            o[1] = b * norm;
        }
    }
}

/// Frames a band-limited read looks at on each side at `speed`.
fn reach(k: &Kernel, speed: f64) -> usize {
    (k.zeros() as f64 * speed.clamp(1.0, MAX_STRETCH)).ceil() as usize
}

/// 4-point Hermite between `y1` (t = 0) and `y2` (t = 1); `y1` exactly at
/// t = 0.
fn hermite(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * t + c2) * t + c1) * t + y1
}
