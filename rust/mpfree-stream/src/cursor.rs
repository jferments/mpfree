//! A cursor: one voice's place in a source. Several cursors can read one
//! source at once, sharing its cached units.
//!
//! A cursor seeks to any frame, plays at a speed (file frames moved per
//! output frame: the file's rate over the output's, times the musical
//! speed) and goes round a loop. Each period it gathers the file frames it
//! will pass, round the loop, from the [`Player`]'s cache into its own
//! buffer, then reads them with 4-point Hermite interpolation; at speed 1
//! from a whole frame that gives the file's own samples, bit for bit.
//! (A band-limited read for rate changes comes with stems, phase P3.)
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

use crate::msg::Place;
use crate::player::Player;

/// The fastest a cursor moves: four file frames an output frame.
pub const MAX_SPEED: f64 = 4.0;

/// Output frames handled in one pass; longer calls go in passes.
pub const PASS: usize = 1024;

/// File frames gathered for one pass, at most: the frames passed and the
/// interpolation's neighbours (one before, two after).
const GATHER: usize = (PASS as f64 * MAX_SPEED) as usize + 4;

pub struct Cursor {
    source: u16,
    id: u16,
    /// The file frame it plays next, with its fraction.
    pos: u64,
    frac: f64,
    speed: f64,
    looping: Option<(u64, u64)>,
    end: Option<u64>,
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
        Cursor {
            source,
            id,
            pos: 0,
            frac: 0.0,
            speed: 1.0,
            looping: None,
            end: None,
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
            // Neighbours: one before (frame pos - 1, or silence at the
            // file's start), two after.
            if self.pos == 0 {
                self.gathered[..ch].fill(0.0);
            } else {
                let before = match self.looping {
                    Some((s, e)) if self.pos == s && self.pos < e => e - 1,
                    _ => self.pos - 1,
                };
                if self.end.is_some_and(|e| before >= e) {
                    self.gathered[..ch].fill(0.0);
                } else {
                    self.gather_one(p, before, 0);
                }
            }
            missed += self.gather(p, self.pos, span, 1, true);
            self.gather(p, self.pos + span as u64, 2, 1 + span, false);
            self.interpolate(ch, part);
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
}

/// 4-point Hermite between `y1` (t = 0) and `y2` (t = 1); `y1` exactly at
/// t = 0.
fn hermite(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * t + c2) * t + c1) * t + y1
}
