//! What the looper heard, for Capture ([`crate::Command::Capture`]).
//!
//! The looper keeps the last [`HEARD_SECONDS`] of its input whatever it is
//! doing, and the frame of every bar line the song played over it. Capture
//! takes whole bars from it: the frames between two bar lines, so a tempo
//! change in between can't skew them. Bar lines are only kept together
//! while the song plays on unbroken: a stop, a count-in, a locate or a
//! transport loop's jump starts a new run, and a take never spans two.
//!
//! The input is stamped with the frame it arrived on; a take is read
//! `latency` frames later, as a first pass is written, so what was played
//! on a bar line lands on the loop's first frame.

use mpfree_transport::time::Tick;
use mpfree_transport::{Block, State};

use crate::{FADE, LOAD_SPEED};

/// Seconds of input the looper keeps for Capture: room for three times
/// a 4-bar pattern at 60 BPM, or 30 bars at 120.
pub const HEARD_SECONDS: u32 = 60;

/// Bar lines kept: more than the seconds kept hold at any tempo a
/// capture is played at (63 bars of 4/4 is 60 s at 252 BPM).
const LINES: usize = 64;

/// Frames a take leaves spare at the old end of what is kept, for the
/// periods between its bar line and the copy reading its first frames.
const SLACK: u64 = 8192;

/// A bar line the song played: its frame and tick, and the unbroken run
/// it belongs to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Line {
    frame: u64,
    tick: Tick,
    run: u32,
}

/// Whole bars of what was heard: where they are and what they become.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Take {
    /// Bars taken (fewer than asked when only fewer were heard, halving).
    pub bars: u32,
    /// The frame heard that becomes the loop's first (latency included).
    pub start: u64,
    /// The loop's frames and ticks, and the song position it starts on.
    pub frames: usize,
    pub ticks: Tick,
    pub origin: Tick,
}

/// The last [`HEARD_SECONDS`] of input, and the bar lines in it.
pub(crate) struct Heard {
    /// Interleaved stereo, frame `f` at `f % cap`.
    audio: Vec<f32>,
    cap: usize,
    /// The frame after the last one heard.
    at: u64,
    /// The first frame of the input as it is now (it began, or changed).
    from: u64,
    lines: [Line; LINES],
    /// Bar lines ever kept (the newest is `n_lines - 1`).
    n_lines: u64,
    run: u32,
    /// While the song plays on: the tick the next period starts on.
    next_tick: Option<Tick>,
    /// While the song plays: its position at `at`, and a bar's and a
    /// beat's ticks.
    now: Option<(f64, Tick, Tick)>,
    /// The frame the unbroken run of song playing now began on.
    run_from: u64,
}

/// A bar line: its tick and the frame it fell on.
pub type LineAt = (Tick, u64);

impl Heard {
    /// Room for `frames` frames (none: Capture finds nothing).
    pub fn new(frames: usize) -> Heard {
        Heard {
            audio: vec![0.0; 2 * frames],
            cap: frames,
            at: 0,
            from: 0,
            lines: [Line::default(); LINES],
            n_lines: 0,
            run: 0,
            next_tick: None,
            now: None,
            run_from: 0,
        }
    }

    /// The bytes it holds.
    #[cfg(test)]
    pub fn bytes(&self) -> usize {
        self.audio.len() * std::mem::size_of::<f32>()
    }

    /// One period: `n` frames of interleaved `input`, from `block`'s start
    /// frame, and the bar lines the song played in it.
    pub fn hear(&mut self, block: &Block, input: &[f32], n: usize) {
        if block.start_frame != self.at {
            // A gap (or the first period): what came before is not next
            // to what comes now.
            self.at = block.start_frame;
            self.from = block.start_frame;
            self.run = self.run.wrapping_add(1);
            self.run_from = block.start_frame;
        }
        if self.cap > 0 {
            let mut done = 0;
            while done < n {
                let i = ((self.at + done as u64) % self.cap as u64) as usize;
                let len = (n - done).min(self.cap - i);
                self.audio[2 * i..2 * (i + len)]
                    .copy_from_slice(&input[2 * done..2 * (done + len)]);
                done += len;
            }
        }
        self.at += n as u64;
        for span in block.spans() {
            if !matches!(span.state, State::Playing | State::Recording) {
                self.next_tick = None;
                self.now = None;
                continue;
            }
            let ticks = span.ticks();
            if self.next_tick != Some(ticks.start) {
                self.run = self.run.wrapping_add(1);
                self.run_from = span.frame;
            }
            self.next_tick = Some(ticks.end);
            let bar = span.meter.bar_ticks();
            for (tick, o) in span.lines(bar) {
                self.lines[(self.n_lines % LINES as u64) as usize] = Line {
                    frame: block.start_frame + u64::from(o),
                    tick,
                    run: self.run,
                };
                self.n_lines += 1;
            }
            let end = span.frame + u64::from(span.len);
            self.now = Some((span.tick_at(end), bar, span.meter.beat_ticks()));
        }
    }

    /// The input changed (another source): what came before is forgotten.
    pub fn forget(&mut self) {
        self.from = self.at;
    }

    /// The frame after the last one heard.
    pub fn at(&self) -> u64 {
        self.at
    }

    /// Whether frame `f` is still kept.
    pub fn holds(&self, f: u64) -> bool {
        f >= self.from && f < self.at && f + self.cap as u64 >= self.at
    }

    /// Frame `f` as heard; silence where it isn't kept.
    pub fn frame(&self, f: u64) -> [f32; 2] {
        if !self.holds(f) {
            return [0.0; 2];
        }
        let i = (f % self.cap as u64) as usize;
        [self.audio[2 * i], self.audio[2 * i + 1]]
    }

    /// Bar lines kept so far, ever (the newest is one less).
    pub fn lines(&self) -> u64 {
        self.n_lines
    }

    /// Bar line `i` (counted from the first ever kept), while kept.
    fn line(&self, i: u64) -> Option<Line> {
        (i < self.n_lines && self.n_lines - i <= LINES as u64)
            .then(|| self.lines[(i % LINES as u64) as usize])
    }

    /// The bar line on song tick `tick` in the newest run (the song as it
    /// last played on unbroken), counted as [`Heard::lines`] counts.
    pub fn line_at(&self, tick: Tick) -> Option<u64> {
        let newest = self.line(self.n_lines.checked_sub(1)?)?;
        (0..self.n_lines.min(LINES as u64))
            .map(|k| self.n_lines - 1 - k)
            .map_while(|i| self.line(i).filter(|l| l.run == newest.run).map(|l| (i, l)))
            .find(|(_, l)| l.tick == tick)
            .map(|(i, _)| i)
    }

    /// The song plays and is in a bar's last beat: a Capture waits for
    /// the bar line coming.
    pub fn in_last_beat(&self) -> bool {
        self.now.is_some_and(|(t, bar, beat)| {
            let bar = bar.max(1) as f64;
            t.rem_euclid(bar) >= bar - beat as f64
        })
    }

    /// Whether the song plays now.
    pub fn playing(&self) -> bool {
        self.now.is_some()
    }

    /// While the song plays: the bar lines of this unbroken run of it
    /// around frame `p` (a note played then), each as its tick and frame:
    /// the last one at or before `p` (None when `p` came before the run
    /// began, or before its first line), and the next one, once heard.
    /// None when the song isn't playing.
    pub fn around(&self, p: u64) -> Option<(Option<LineAt>, Option<LineAt>)> {
        self.now?;
        let newest = self.n_lines.checked_sub(1).and_then(|i| self.line(i));
        let Some(newest) = newest.filter(|l| l.frame >= self.run_from) else {
            return Some((None, None));
        };
        let (mut before, mut after) = (None, None);
        let mut i = self.n_lines;
        while let Some(l) = i.checked_sub(1).and_then(|k| self.line(k)) {
            if l.run != newest.run {
                break;
            }
            if l.frame > p {
                after = Some((l.tick, l.frame));
            } else {
                if p >= self.run_from {
                    before = Some((l.tick, l.frame));
                }
                break;
            }
            i -= 1;
        }
        Some((before, after))
    }

    /// The newest bar lines of the newest run, up to `out.len()`, oldest
    /// first: each one's tick and frame. Returns how many.
    pub fn recent(&self, out: &mut [(Tick, u64)]) -> usize {
        let Some(newest) = self.n_lines.checked_sub(1).and_then(|i| self.line(i)) else {
            return 0;
        };
        let mut n = 0;
        while n < out.len() && (n as u64) < self.n_lines {
            match self.line(self.n_lines - 1 - n as u64) {
                Some(l) if l.run == newest.run => n += 1,
                _ => break,
            }
        }
        for (k, o) in out[..n].iter_mut().enumerate() {
            let l = self.lines[((self.n_lines - n as u64 + k as u64) % LINES as u64) as usize];
            *o = (l.tick, l.frame);
        }
        n
    }

    /// `bars` whole bars ending on bar line `end`, read `latency` frames
    /// late: fewer (halving) when fewer were played since the song last
    /// started or jumped, were heard since the input changed, or fit in
    /// what is kept. None when not even one bar does.
    pub fn take(&self, end: u64, bars: u32, latency: u32) -> Option<Take> {
        let e = self.line(end)?;
        let mut k = bars.max(1);
        loop {
            if let Some(s) = end
                .checked_sub(u64::from(k))
                .and_then(|i| self.line(i))
                .filter(|s| s.run == e.run && e.frame > s.frame)
            {
                let start = s.frame + u64::from(latency);
                let frames = (e.frame - s.frame) as usize;
                let first = start.saturating_sub(FADE as u64);
                // Kept until the copy has read it: from when all of it is
                // heard, for as long as the copy takes.
                let copy = (frames / LOAD_SPEED) as u64 + SLACK;
                let last = self.at.max(start + frames as u64);
                if start >= self.from && last - first + copy <= self.cap as u64 {
                    return Some(Take {
                        bars: k,
                        start,
                        frames,
                        ticks: e.tick - s.tick,
                        origin: s.tick,
                    });
                }
            }
            if k == 1 {
                return None;
            }
            k /= 2;
        }
    }
}
