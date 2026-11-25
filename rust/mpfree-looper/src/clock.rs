//! The clocks a loop keeps time with: the song's and the looper's
//! free clock, and a loop's place on either.

use super::*;

/// The clock a loop keeps time with: the Looper tab's Sync setting
/// ([`Command::SetClock`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Clock {
    /// The song's, the transport: the loop starts and ends on the song's
    /// bar lines, follows its position through stops, locates and its own
    /// loop, and plays at its tempo.
    #[default]
    Song,
    /// The looper's free clock ([`FreeClock`]): the loop plays on when the
    /// song stops, keeps its place when the song locates and its tempo
    /// when the song's changes, and records with the song stopped, at once
    /// or in your own time. Every free loop keeps time with the one free
    /// clock, so they stay together.
    Free,
}

/// A clock domain as it is at one moment ([`Looper::stamp`]): which clock
/// and, for the free clock, the number it was set as and the frame its
/// timing holds from (the frame it last started on, or a snap re-anchored
/// it on). Something scheduled on a clock (a snap, a capture's bar line)
/// keeps the stamp it was worked out on and acts only while its clock
/// still has it: never on a free clock set again, started again from its
/// top on another frame, or given another tempo since. The song's clock
/// is the time engine's, which commits its own changes, so its stamp
/// never changes ([`ClockStamp::SONG`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockStamp {
    pub clock: Clock,
    /// How many free clocks had been set (`FreeTime::gen`); 0 for the song.
    pub gen: u32,
    /// The absolute frame the free clock's timing holds from; 0 for the
    /// song.
    pub since: u64,
}

impl ClockStamp {
    /// The song's clock.
    pub const SONG: ClockStamp = ClockStamp {
        clock: Clock::Song,
        gen: 0,
        since: 0,
    };
}

/// The looper's free clock: the time the free loops keep ([`Clock::Free`],
/// [`Looper::free_clock`]). The first free loop sets it: with the song
/// playing, a copy of the song's clock from the bar line the loop starts
/// on, so the loop plays exactly as a song loop does until the song stops,
/// locates or changes tempo; with nothing playing, the tempo its length
/// reads as ([`read_bars`]), its bar 1 on the loop's start. It lasts while
/// a free loop holds audio. It never locates and never changes tempo:
/// only the free loops' own Stop and Play ([`Command::FreeStop`],
/// [`Command::FreePlay`]), STOP with the song stopped ([`Mark::Panic`])
/// and PLAY ([`Mark::Started`]) stop and start it. A stop takes it back to
/// its bar 1 (`top`), so the free loops start again where they started,
/// together, each in its place against the others.
///
/// While it runs, tick `t` falls on frame `anchor_frame + round(frames
/// from anchor_tick to t)` at `bpm`, as on the transport's spans
/// ([`Span`]), from `anchor_frame` on (a clock set to start on a frame to
/// come is still at `anchor_tick` before it). Stopped, it rests at `top`.
/// A plain value: copy it to ask it where it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FreeClock {
    pub anchor_frame: u64,
    pub anchor_tick: Tick,
    pub bpm: f64,
    pub rate: u32,
    pub meter: Meter,
    pub running: bool,
    /// Its bar 1: where a stop takes it back to, and a start runs from.
    pub top: Tick,
}

impl FreeClock {
    /// A clock at rest on its bar 1, `top`.
    pub fn stopped(bpm: f64, rate: u32, meter: Meter, top: Tick) -> FreeClock {
        FreeClock {
            anchor_frame: 0,
            anchor_tick: top,
            bpm,
            rate,
            meter,
            running: false,
            top,
        }
    }

    /// The song's clock in `span` (which must be rolling), copied exactly,
    /// its bar 1 on `top`: it gives the same ticks on the same frames as
    /// the song for as long as the song keeps that anchor and tempo.
    pub(crate) fn of_song(span: &Span, top: Tick) -> FreeClock {
        FreeClock {
            anchor_frame: span.anchor_frame,
            anchor_tick: span.anchor_tick,
            bpm: span.bpm,
            rate: span.rate,
            meter: span.meter,
            running: true,
            top,
        }
    }

    /// Whether it runs on absolute frame `frame`.
    pub fn rolls_at(&self, frame: u64) -> bool {
        self.running && frame >= self.anchor_frame
    }

    /// Where it is on absolute frame `frame`, with the fraction, worked out
    /// as [`Span::tick_at`] works out the song's: `anchor_tick` while it
    /// doesn't run.
    pub fn tick_at(&self, frame: u64) -> f64 {
        if !self.rolls_at(frame) {
            return self.anchor_tick as f64;
        }
        self.anchor_tick as f64
            + ticks_in(frame as f64 - self.anchor_frame as f64, self.bpm, self.rate)
    }

    /// The clock over `len` frames from absolute frame `frame`, which is
    /// `offset` into its period, as one of the transport's spans: playing
    /// while it runs, stopped where it rests when not. Free loops get their
    /// bar lines and their place from it as song loops get theirs from the
    /// transport's.
    pub fn span(&self, offset: u32, len: u32, frame: u64) -> Span {
        let rolling = self.rolls_at(frame);
        Span {
            offset,
            len,
            frame,
            state: if rolling {
                State::Playing
            } else {
                State::Stopped
            },
            overdub: false,
            anchor_frame: if rolling { self.anchor_frame } else { 0 },
            anchor_tick: self.anchor_tick,
            bpm: self.bpm,
            rate: self.rate,
            swing: Swing::default(),
            meter: self.meter,
        }
    }

    /// Its next bar line on or after absolute frame `frame`, while it
    /// runs: the bar's tick and the frame it falls on, as the transport
    /// places the song's ([`Span::lines`]).
    pub fn next_bar(&self, frame: u64) -> Option<(Tick, u64)> {
        if !self.running {
            return None;
        }
        let from = frame.max(self.anchor_frame);
        let bar = self.meter.bar_ticks().max(1);
        // A span a bar and a little long holds the next line.
        let frames = bar as f64 * 60.0 * f64::from(self.rate) / (self.bpm * f64::from(PPQ));
        let len = (frames.ceil() as u64 + 2).min(u64::from(u32::MAX)) as u32;
        let span = self.span(0, len, from);
        span.lines(bar)
            .next()
            .map(|(t, o)| (t, from + u64::from(o)))
    }

    /// While it runs: the frame its tick `t` falls on (at or after its
    /// anchor), as its spans place it, bit for bit.
    pub fn frame_of(&self, t: Tick) -> Option<u64> {
        (self.running && t >= self.anchor_tick)
            .then(|| self.span(0, 1, self.anchor_frame).frame_of(t as f64))
    }

    /// While it runs: its newest bar line on a frame before `at` and at
    /// or after `since` (once heard by `at`, in its run since then), as
    /// its tick and frame.
    pub(crate) fn line_before(&self, at: u64, since: u64) -> Option<(Tick, u64)> {
        if !self.running || at <= self.anchor_frame.max(since) {
            return None;
        }
        let bar = self.meter.bar_ticks().max(1);
        let mut t = (self.tick_at(at - 1).floor() as Tick).div_euclid(bar) * bar;
        while let Some(f) = self.frame_of(t).filter(|&f| f >= since) {
            if f < at {
                return Some((t, f));
            }
            t -= bar;
        }
        None
    }

    /// While it runs: the bar line coming after absolute frame `at`, when
    /// `at` is in a bar's last beat (a Capture then waits for it).
    pub(crate) fn last_beat_line(&self, at: u64) -> Option<Tick> {
        if !self.rolls_at(at) {
            return None;
        }
        let (bar, beat) = (self.meter.bar_ticks().max(1), self.meter.beat_ticks());
        let now = self.tick_at(at);
        let line = (now / bar as f64).floor() as Tick * bar;
        (now - line as f64 >= (bar - beat) as f64).then_some(line + bar)
    }

    /// Started on absolute frame `f`, from its bar 1.
    pub(crate) fn start_at(&mut self, f: u64) {
        self.anchor_frame = f;
        self.anchor_tick = self.top;
        self.running = true;
    }

    /// Stopped, back on its bar 1.
    pub(crate) fn stop(&mut self) {
        *self = FreeClock::stopped(self.bpm, self.rate, self.meter, self.top);
    }
}

/// Each frame's place on a clock, from absolute frame `f` on: a clock
/// anchored at `anchor_tick` on `anchor_frame` at `bpm` and `rate`, as
/// [`Span::tick_at`] and [`FreeClock::tick_at`] work it out, bit for bit
/// (`anchor_tick` throughout while it doesn't run). Out of line, so that
/// its loop stays one the compiler vectorises: inline in
/// [`Looper::process`] it can lose that when the code round it changes
/// (it did with the free clock, at a cost of 3 000 instructions a period).
#[inline(never)]
pub(crate) fn fill_ticks(out: &mut [f64], rolling: bool, clock: (u64, Tick, f64, u32), f: u64) {
    let (anchor_frame, anchor_tick, bpm, rate) = clock;
    if !rolling {
        out.fill(anchor_tick as f64);
        return;
    }
    for (j, t) in out.iter_mut().enumerate() {
        *t = anchor_tick as f64 + ticks_in((f + j as u64) as f64 - anchor_frame as f64, bpm, rate);
    }
}

/// Where a loop `len_ticks` long that starts on tick `origin` is when its
/// clock is on tick `tick`: 0 at its start, to 1 at its end. Every loop
/// works out its place so, from the song's clock or the free one
/// ([`Clock`]).
pub fn place(tick: f64, origin: Tick, len_ticks: Tick) -> f64 {
    if len_ticks <= 0 {
        return 0.0;
    }
    rem_ticks(tick - origin as f64, len_ticks as f64) / len_ticks as f64
}

/// `x.rem_euclid(y)` bit for bit, for `y` a whole number of ticks above 0,
/// without the cost of `fmod` (a loop of integer steps in software, which
/// was an eighth of the looper's time with 8 loops playing).
///
/// For |x| below 2^31, `n * y` is a whole number and so a multiple of x's
/// last place, which makes `x - n * y` exact; `n` can be one too many where
/// `x / y` rounds up to a whole number, and adding `y` back is exact too,
/// as the result (fmod's) is representable.
pub(crate) fn rem_ticks(x: f64, y: f64) -> f64 {
    // Below 2^31 the quotient fits an i32, whose conversion is one
    // instruction (`trunc` is a call on the MPC's ARMv7 and on base x86-64).
    if x.abs() >= 2.0e9 || x.is_nan() {
        return x.rem_euclid(y);
    }
    let n = f64::from((x / y) as i32);
    let mut r = x - n * y;
    if x >= 0.0 {
        if r < 0.0 {
            r += y;
        } else if r >= y {
            r -= y;
        }
    } else if r > 0.0 {
        r -= y;
    } else if r <= -y {
        r += y;
    }
    // fmod: a zero result has x's sign.
    if r == 0.0 {
        r = 0.0f64.copysign(x);
    }
    if r < 0.0 {
        r + y
    } else {
        r
    }
}

/// The bar lines around a note: the last one at or before it and the
/// next, each as its tick and frame ([`Heard::around`] for the song's,
/// [`free_around`] for the free clock's).
pub(crate) type Around = (Option<(Tick, u64)>, Option<(Tick, u64)>);

/// The free clock's bar lines around frame `p` (a note), as
/// [`Heard::around`] gives the song's: the last one at or before `p`
/// (None when `p` came before the clock started, or before its first
/// line), and the next.
pub(crate) fn free_around(c: &FreeClock, p: u64) -> Around {
    let after = c.next_bar(p.saturating_add(1));
    let before = after.filter(|_| c.rolls_at(p)).and_then(|(t, _)| {
        let t = t - c.meter.bar_ticks().max(1);
        (t >= c.anchor_tick).then(|| (t, c.span(0, 0, p).frame_of(t as f64)))
    });
    (before, after)
}
