//! Capture: the bars it takes, and the take copied into a loop.

use super::*;

impl Looper {
    /// The free clock's run a capture into a free loop takes bars from: as
    /// it runs, or, once it has stopped, as it last ran (up to the frame
    /// it stopped on) while that is the clock there is. None with no free
    /// clock, or one that hasn't run since it was set.
    fn free_run(&self) -> Option<FreeRun> {
        let ft = &self.free_time;
        let c = ft.clock?;
        if c.running {
            return Some(FreeRun {
                clock: c,
                since: ft.since,
                until: None,
            });
        }
        let (clock, _, until) = ft.ran.filter(|r| r.1 == ft.gen)?;
        Some(FreeRun {
            clock,
            since: ft.since,
            until: Some(until),
        })
    }

    /// Whose bar lines a capture into an empty loop on `clock` takes: the
    /// song's for a song loop, and for a free loop while no free clock is
    /// set (the free clock is then set from them); the free clock's run
    /// for a free loop while one is ([`Looper::free_run`]). None while the
    /// free clock there is hasn't run (its free loops would not hear the
    /// bars).
    pub(crate) fn capture_bars(&self, clock: Clock) -> Option<CaptureBars> {
        match (clock, self.free_time.clock) {
            (Clock::Song, _) | (Clock::Free, None) => Some(CaptureBars::Song),
            (Clock::Free, Some(_)) => self.free_run().map(CaptureBars::Free),
        }
    }

    /// The bar line a capture into an empty loop on `clock` ends on
    /// ([`Looper::capture_bars`]), and whether it is still to come: with
    /// `to`, the one heard on that tick, else the last one heard, or the
    /// one coming in a bar's last beat. None with no such line.
    fn capture_end(&self, clock: Clock, to: Option<Tick>) -> Option<(CaptureEnd, bool)> {
        match self.capture_bars(clock)? {
            CaptureBars::Song => match to {
                None => {
                    let wait = self.heard.in_last_beat();
                    match (self.heard.lines(), wait) {
                        (n, true) => Some((CaptureEnd::Song(n), true)),
                        (0, false) => None,
                        (n, false) => Some((CaptureEnd::Song(n - 1), false)),
                    }
                }
                Some(t) => self.heard.line_at(t).map(|i| (CaptureEnd::Song(i), false)),
            },
            CaptureBars::Free(run) => {
                let at = self.heard.at();
                let stamp = self.free_time.stamp();
                let free = |end| CaptureEnd::Free { end, stamp };
                match to {
                    None => match run.coming_line(at) {
                        Some(coming) => Some((free(coming), true)),
                        None => run.last_line(at).map(|(t, _)| (free(t), false)),
                    },
                    Some(t) => run.line_heard(t, at).map(|_| (free(t), false)),
                }
            }
        }
    }

    /// `bars` whole bars of the free clock's `run` ending on its bar line
    /// on tick `end`, read `latency` late: fewer (halving) as
    /// [`Heard::take`] gives them. None unless that line was heard in the
    /// run.
    pub(crate) fn free_take(
        &self,
        run: &FreeRun,
        end: Tick,
        bars: u32,
        latency: u32,
    ) -> Option<Take> {
        let e = run.line_heard(end, self.heard.at())?;
        let c = &run.clock;
        let bar = c.meter.bar_ticks().max(1);
        let mut k = bars.max(1);
        loop {
            let start = end - i64::from(k) * bar;
            if let Some(t) = c
                .frame_of(start)
                .filter(|&f| f >= run.since)
                .and_then(|f| self.heard.take_lines((start, f), (end, e), k, latency))
            {
                return Some(t);
            }
            if k == 1 {
                return None;
            }
            k /= 2;
        }
    }

    /// Capture into loop `s` ([`Command::Capture`]): it waits for its bar
    /// line (the last one heard, or, pressed in a bar's last beat, the one
    /// coming); [`Looper::tend_captures`] takes it from there.
    /// Capture into loop `s`: `bars` ending on the last bar line (or the
    /// one coming, in a bar's last beat), or on the one heard on tick
    /// `to`.
    pub(crate) fn start_capture(&mut self, s: usize, bars: u32, to: Option<Tick>) {
        let l = &self.loops[s];
        let empty = l.phase == Phase::Empty
            && !l.clearing
            && !l.loading
            && !l.adding
            && self.loads[s].is_none()
            && bars > 0;
        let Some((end, wait)) = self.capture_end(l.clock, to).filter(|_| empty) else {
            self.stats.refused += 1;
            return;
        };
        let l = &mut self.loops[s];
        l.capture = Some(Capture {
            bars,
            end,
            wait,
            take: None,
        });
        // A new loop plays once it is in.
        l.on = true;
        l.cue = None;
    }

    /// Each capture waiting: once its bar line is heard, the take; once
    /// the take's last frame is heard (`latency` after the bar line), it
    /// copies in, as a load does, from what was heard.
    pub(crate) fn tend_captures(&mut self) {
        for k in 0..self.loops.len() {
            if let Some(c) = self.loops[k].capture {
                self.loops[k].capture = self.tend_capture(k, c);
            }
        }
    }

    /// [`Looper::tend_captures`] for loop `k`: the capture as it is now,
    /// or None once it copies in or is refused.
    fn tend_capture(&mut self, k: usize, mut c: Capture) -> Option<Capture> {
        let h = &self.heard;
        let latency = self.loops[k].latency;
        let take = match c.end {
            CaptureEnd::Song(mut end) => {
                if c.wait {
                    if h.lines() > end {
                        c.wait = false;
                    } else if h.playing() {
                        return Some(c);
                    } else if end > 0 {
                        // The song stopped before that bar line: the bars
                        // before.
                        c.wait = false;
                        end -= 1;
                        c.end = CaptureEnd::Song(end);
                    } else {
                        self.stats.refused += 1;
                        return None;
                    }
                }
                c.take.or_else(|| h.take(end, c.bars, latency))
            }
            CaptureEnd::Free { mut end, stamp } => {
                // Its bars are the free clock's run they were heard in: not
                // once it has started again, been re-anchored or another
                // clock is set.
                let Some(run) = self.free_run().filter(|_| self.free_time.stamp() == stamp) else {
                    self.stats.refused += 1;
                    return None;
                };
                if c.wait {
                    match run.line_heard(end, h.at()) {
                        Some(_) => c.wait = false,
                        None if run.until.is_none() => return Some(c),
                        None => {
                            // The free loops stopped before that bar line:
                            // the bars before.
                            c.wait = false;
                            end -= run.clock.meter.bar_ticks().max(1);
                            c.end = CaptureEnd::Free { end, stamp };
                        }
                    }
                }
                c.take
                    .or_else(|| self.free_take(&run, end, c.bars, latency))
            }
        };
        let Some(t) = take else {
            self.stats.refused += 1;
            return None;
        };
        c.take = Some(t);
        if self.heard.at() < t.start + t.frames as u64 {
            // Its last frames are still to be heard.
            return Some(c);
        }
        let need = blocks_for(t.frames);
        if self.store.available() < need {
            self.stats.refused += 1;
            return None;
        }
        // A free loop takes the free clock its bars were on; with none, the
        // song's bars set it, as a free loop recorded with the song does.
        if self.loops[k].clock == Clock::Free {
            match c.end {
                CaptureEnd::Free { stamp, .. } => self.loops[k].clock_gen = stamp.gen,
                CaptureEnd::Song(_) if self.free_time.clock.is_none() => {
                    self.set_free_clock_from(&t, latency);
                    self.loops[k].clock_gen = self.free_time.gen;
                }
                // Another free loop set it meanwhile, maybe at another
                // tempo: the song's bars can't go on it.
                CaptureEnd::Song(_) => {
                    self.stats.refused += 1;
                    return None;
                }
            }
        }
        // It takes the place of the loop kept for Undo.
        self.store.drop_kept(k);
        let l = &mut self.loops[k];
        l.reclaim(&mut self.store, k);
        if !self.store.grow(k, 0, need) {
            self.stats.refused += 1;
            l.reclaim(&mut self.store, k);
            return None;
        }
        l.slots[0] = Slot {
            valid: (0, t.frames),
            born: self.store.born(),
            ..Slot::NEW
        };
        l.loading = true;
        l.ld_heard = Some(t.start);
        l.ld_layers = 1;
        l.ld_frames = t.frames;
        l.ld_total = t.frames;
        l.ld_raw = None;
        l.ld_ticks = t.ticks;
        l.ld_origin = t.origin;
        l.ld_done = 0;
        // From the frame its last frame was heard on, when that is in
        // this period, so the period size changes nothing.
        l.ld_from = self.now.max(t.start + t.frames as u64);
        l.ld_at = l.ld_from + t.frames.div_ceil(LOAD_SPEED) as u64;
        None
    }

    /// The free clock set from the song's bars `t` that a free loop
    /// captured (read `latency` late), bar 1 on their start: a copy of the
    /// song's clock while the song still plays on the one they were on,
    /// so the loop plays sample for sample as a song loop would until the
    /// song stops or changes; else at the tempo their frames make, running
    /// on from their end while the song plays, or with the song stopped,
    /// stopped on its bar 1 (the loop waits for Play, as a song loop
    /// captured then waits for PLAY).
    fn set_free_clock_from(&mut self, t: &Take, latency: u32) {
        let first = t.start - u64::from(latency);
        let end = (first + t.frames as u64, t.origin + t.ticks);
        let song = self.last_span.filter(|sp| {
            sp.state.rolling()
                && sp.frame_of(t.origin as f64) == first
                && sp.frame_of(end.1 as f64) == end.0
        });
        let (c, from) = match song {
            Some(sp) => (FreeClock::of_song(&sp, t.origin), first),
            None => {
                let mut c = FreeClock {
                    anchor_frame: end.0,
                    anchor_tick: end.1,
                    bpm: t.ticks as f64 * 60.0 * f64::from(self.rate)
                        / (f64::from(PPQ) * t.frames.max(1) as f64),
                    rate: self.rate,
                    meter: self.last_span.map(|sp| sp.meter).unwrap_or_default(),
                    running: true,
                    top: t.origin,
                };
                if !self.last_span.is_some_and(|sp| sp.state.rolling()) {
                    c.stop();
                }
                (c, end.0)
            }
        };
        let ft = &mut self.free_time;
        ft.gen = ft.gen.wrapping_add(1);
        ft.clock = Some(c);
        ft.kept = None;
        ft.cue = None;
        ft.jump = from;
        ft.since = from;
    }
}
