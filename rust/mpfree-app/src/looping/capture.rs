//! Capture and What you played: the window, its wave and the take.

use super::*;

impl LoopHost {
    /// Where Capture goes now: the picked loop if it is empty (or
    /// capturing: then Capture calls it off), else the first empty loop;
    /// and the bars that loop's Length asks for (Free:
    /// [`CAPTURE_FREE_BARS`]).
    pub(crate) fn capture_into(&self) -> Option<(usize, u32)> {
        let picked = self.looper.loop_status(self.picked)?;
        let n = if picked.capturing.is_some() {
            self.picked
        } else {
            self.free_loop().ok()?
        };
        let bars = self.looper.loop_status(n)?.bars;
        let bars = self
            .span()
            .map_or(bars.unwrap_or(CAPTURE_FREE_BARS), |s| s.1);
        Some((n, bars))
    }

    /// The bars What you played's window grows and shrinks by, for loop
    /// `n`, and whether they are the pattern's: the pattern's length while
    /// Length is Pattern and a pattern plays, else the Length's bars,
    /// else 1.
    fn capture_unit(&self, n: usize) -> (u32, bool) {
        match (self.match_pattern, self.pattern_bars) {
            (true, Some(b)) => (b.max(1), true),
            _ => (
                self.looper
                    .loop_status(n)
                    .and_then(|s| s.bars)
                    .unwrap_or(1)
                    .max(1),
                false,
            ),
        }
    }

    /// What you played's window as it stands: one moved back whose bar
    /// line is no longer kept (the song stopped or jumped, or it fell out
    /// of the 60 s) is let go of.
    fn span(&self) -> Option<(Option<Tick>, u32)> {
        let (end, bars) = self.capture_span?;
        match end {
            Some(t) => self
                .looper
                .capture_take_to(self.clock, t, 1)
                .map(|_| (end, bars)),
            None => Some((None, bars)),
        }
    }

    /// What Capture would take now, `bars` of it: up to the window's bar
    /// line when it was moved back, else up to the newest. On the clock
    /// the loop it goes into takes (Sync as last set): a free loop's are
    /// the free clock's bars while it is set.
    fn capture_take_now(&self, bars: u32) -> Option<Take> {
        match self.span().and_then(|s| s.0) {
            Some(end) => self.looper.capture_take_to(self.clock, end, bars),
            None => self.looper.capture_take(self.clock, bars),
        }
    }

    /// A Capture now would wait for the bar line coming: the window keeps
    /// up with the song (or the free loops), which is in a bar's last
    /// beat.
    fn capture_waits_now(&self) -> bool {
        self.span().and_then(|s| s.0).is_none() && self.looper.capture_waits(self.clock)
    }

    /// The bars a Capture now counts in, for What you played and the log:
    /// for a free loop (Sync: Free) while the free clock is set, the free
    /// clock's, numbered from its bar 1 (where the free loops start);
    /// else the song's. Their tempo, a bar's ticks and bar 1's tick.
    fn capture_grid(&self) -> (f64, Tick, Tick) {
        match (self.clock, self.looper.free_clock()) {
            (Clock::Free, Some(c)) => (c.bpm, c.meter.bar_ticks(), c.top),
            _ => (self.bpm, self.bar_ticks, 0),
        }
    }

    /// The bar a Capture's tick `t` starts, numbered in its bars
    /// ([`LoopHost::capture_grid`]).
    fn capture_bar(&self, t: Tick) -> i64 {
        let (_, bar, top) = self.capture_grid();
        (t - top).div_euclid(bar.max(1)) + 1
    }

    /// What you played's window moved or resized on the Looper tab:
    /// Capture takes `bars` ending on the bar line that starts bar `end`;
    /// None, or the newest bar line, keeps up with the song. Until a
    /// Capture, a new Length, or its bar line is no longer kept. A line
    /// for the log.
    pub fn set_capture_span(&mut self, end: Option<i64>, bars: u32) -> String {
        let bars = bars.clamp(1, HEARD_MAX_BARS);
        let mut lines = [(0 as Tick, 0u64); HEARD_MAX_BARS as usize + 1];
        let n = self.looper.heard_lines(self.clock, &mut lines);
        let number = |t: Tick| self.capture_bar(t);
        let newest = n.checked_sub(1).map(|i| number(lines[i].0));
        let pinned = match end {
            Some(e) if Some(e) != newest => match lines[..n].iter().find(|l| number(l.0) == e) {
                Some(l) => Some((e, l.0)),
                None => {
                    return format!(
                        "looper: Capture window: bar {e}'s line isn't kept: it stays as it was"
                    )
                }
            },
            _ => None,
        };
        self.capture_span = Some((pinned.map(|p| p.1), bars));
        match pinned {
            Some((e, _)) => format!(
                "looper: Capture window: {}",
                bars_words(e - i64::from(bars), bars)
            ),
            None => format!("looper: Capture window: the last {}", bars_words_n(bars)),
        }
    }

    /// What the looper heard over the last [`HEARD_BARS`] bars at the
    /// tempo now, with its bar lines and what Capture would take.
    pub(crate) fn heard_wave(&self) -> HeardWave {
        let mut w = HeardWave::default();
        let chunks = self.heard_peaks.len() as u64;
        if chunks == 0 {
            return w;
        }
        let now = self.looper.heard_at();
        let (bpm, bar_ticks, _) = self.capture_grid();
        let bar =
            bar_ticks.max(1) as f64 / f64::from(PPQ) * 60.0 / bpm.max(1.0) * f64::from(self.rate);
        // Room for the window to grow to four times its unit (the
        // pattern), and a bar more than it takes.
        let into = self.capture_into();
        if let Some((n, _)) = into {
            (w.unit, w.pattern) = self.capture_unit(n);
        }
        let shown = HEARD_BARS
            .max(4 * w.unit)
            .max(into.map_or(0, |i| i.1 + 1))
            .min(HEARD_MAX_BARS);
        let span = (bar * f64::from(shown)).min((chunks - 1) as f64 * HEARD_CHUNK as f64);
        let from = now as f64 - span;
        let x = |f: u64| ((f as f64 - from) / span) as f32;
        for (c, col) in w.cols.iter_mut().enumerate() {
            let a = from + span * c as f64 / HEARD_COLS as f64;
            let b = from + span * (c + 1) as f64 / HEARD_COLS as f64;
            if b <= 0.0 {
                continue;
            }
            let (a, b) = (
                a.max(0.0) as u64 / HEARD_CHUNK,
                (b as u64).div_ceil(HEARD_CHUNK),
            );
            let p = (a..b.max(a + 1))
                .map(|k| self.heard_peaks[(k % chunks) as usize])
                .fold(0.0f32, f32::max);
            *col = if p > 0.0 {
                20.0 * p.log10()
            } else {
                f32::NEG_INFINITY
            };
        }
        let mut lines = [(0 as Tick, 0u64); HEARD_MAX_BARS as usize + 1];
        let n = self.looper.heard_lines(self.clock, &mut lines);
        let number = |t: Tick| self.capture_bar(t);
        for &(t, f) in &lines[..n] {
            if f as f64 >= from && w.n_lines < w.lines.len() {
                w.lines[w.n_lines] = (number(t), x(f));
                w.n_lines += 1;
            }
        }
        w.waits = self.capture_waits_now();
        if let Some((_, bars)) = into {
            let latency = u64::from(self.looper.status().latency);
            w.take = self.capture_take_now(bars).map(|t| {
                let a = t.start.saturating_sub(latency);
                let (a, b) = (x(a), x(a + t.frames as u64));
                if w.waits {
                    // It takes the bars up to the bar line coming: one on.
                    let shift = b - a;
                    let shift = shift / t.bars as f32;
                    (a + shift, 1.0, number(t.origin) + 1, t.bars)
                } else {
                    (a, b, number(t.origin), t.bars)
                }
            });
        }
        w
    }

    /// Capture ([`Command::Capture`]): the last bars heard become a loop,
    /// the picked one if it is empty, else the first empty one; its
    /// Length gives the bars (Free: [`CAPTURE_FREE_BARS`]). Again while it
    /// waits for its bar line or copies in, it is called off. A line for
    /// the log, saying which bars, or why not.
    pub fn capture(&mut self) -> String {
        let picked = self.looper.loop_status(self.picked);
        if picked.is_some_and(|s| s.capturing.is_some()) {
            if !self.looper.command(Command::Capture(1)) {
                return "looper: Capture: NOT called off, the command queue is full".to_string();
            }
            return format!("looper: Capture into Loop {} called off", self.picked + 1);
        }
        let n = match self.free_loop() {
            Ok(n) => n,
            Err(why) => return format!("looper: Capture: {why}"),
        };
        let span = self.span();
        let asked = span.map(|s| s.1).unwrap_or_else(|| {
            self.looper
                .loop_status(n)
                .and_then(|s| s.bars)
                .unwrap_or(CAPTURE_FREE_BARS)
        });
        let pinned = span.and_then(|s| s.0);
        let waits = self.capture_waits_now();
        let take = self.capture_take_now(asked);
        // A free loop takes the free clock's bars while it is set.
        let free = self.clock == Clock::Free;
        let on_free = free && self.looper.free_clock().is_some();
        let Some(t) = take.or(waits.then_some(Take {
            bars: asked,
            start: 0,
            frames: 0,
            ticks: 0,
            origin: 0,
        })) else {
            if on_free {
                return "looper: Capture: no whole bar of the free loops heard yet: it takes the \
                        bars they have played since they last started (and since Records from \
                        changed)"
                    .to_string();
            }
            return "looper: Capture: no whole bar heard yet: it takes the bars the song has \
                    played since PLAY (and since Records from changed)"
                .to_string();
        };
        let m = self.looper.memory();
        if t.frames > m.capacity.saturating_sub(m.used) {
            return format!(
                "looper: Capture: the loops' memory can't hold {} ({:.1} s): clear a loop, or \
                 set a bigger Loop memory in Prefs",
                bars_words_n(t.bars),
                t.frames as f64 / f64::from(self.rate)
            );
        }
        if n != self.picked {
            if !self.looper.select_loop(n) {
                return "looper: Capture: NOT started, the command queue is full".to_string();
            }
            self.picked = n;
            self.for_cell = false;
        }
        // It records as a loop recorded now would: Sync as last set (the
        // bars above were counted on that clock), and Starts.
        self.sticky_settings(n, self.clock);
        let c = match pinned {
            Some(end) => Command::CaptureTo(end, asked),
            None => Command::Capture(asked),
        };
        if !self.looper.command(c) {
            return "looper: Capture: NOT started, the command queue is full".to_string();
        }
        // The window goes back to the Length's bars, keeping up.
        self.capture_span = None;
        let sync = if free { ", free" } else { "" };
        if take.is_none() || waits {
            return format!(
                "looper: Capture: on the bar line coming, the last {} become Loop {}{sync}",
                bars_words_n(asked),
                n + 1
            );
        }
        let since = if on_free {
            "since the free loops started"
        } else {
            "since PLAY"
        };
        let fewer = match (t.bars < asked, span.is_some()) {
            (false, _) => String::new(),
            (true, false) => format!(" (its Length asks {asked}: fewer were played {since})"),
            (true, true) => format!(" (the window asks {asked}: fewer were played or kept)"),
        };
        format!(
            "looper: Capture: {} become Loop {}{sync}{fewer}",
            bars_words(self.capture_bar(t.origin), t.bars),
            n + 1
        )
    }
}
