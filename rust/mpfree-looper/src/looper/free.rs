//! Free loops: the free clock, Make song and back to free.

use super::*;

impl Looper {
    /// Whether loop `k`, recording a first pass now with the song stopped,
    /// would record free ([`Command::TempoFromLoop`]): on, not resampling,
    /// and every other song loop empty (nothing loading, copying in or
    /// capturing; free loops don't count). Its Length doesn't matter: a bar
    /// count is the bars at the tempo you play. A free loop records free
    /// whenever there is no free clock yet (and it isn't resampling): its
    /// length sets the free clock.
    pub(crate) fn frees(&self, k: usize) -> bool {
        let l = &self.loops[k];
        if l.clock == Clock::Free {
            return !l.resample && self.free_time.clock.is_none();
        }
        self.free_tempo
            && !l.resample
            && self.loops.iter().enumerate().all(|(i, o)| {
                i == k
                    || o.clock == Clock::Free
                    || (o.phase == Phase::Empty && !o.loading && !o.adding && o.capture.is_none())
            })
    }

    /// Loop `k`'s first pass starts free on song frame `rec_from` (the
    /// period's first, or a note before it, copied in from what was
    /// heard).
    pub(crate) fn begin_free(&mut self, k: usize, rec_from: u64) {
        let Looper {
            store,
            loops,
            now,
            bar_ticks,
            ..
        } = self;
        loops[k].begin_free(store, k, rec_from, *now, *bar_ticks);
        self.free_open = true;
        // Smart loop end's notes: from a reach before the pass (a smart
        // start looks there) on.
        let reach = ends::reach(self.rate, f64::NAN);
        self.notes.keep_from(rec_from.saturating_sub(reach));
        self.hits.keep_from(rec_from.saturating_sub(reach));
    }

    /// The free clock, while a free loop holds audio ([`FreeClock`]): its
    /// tempo, meter and bar 1, and whether it runs. None otherwise.
    pub fn free_clock(&self) -> Option<FreeClock> {
        self.free_time.clock
    }

    /// Where the free clock is at the period now (its tick, with the
    /// fraction; its bar 1 while it is stopped), for the screen's bar
    /// counts. None with no free clock.
    pub fn free_now(&self) -> Option<f64> {
        self.free_time.clock.map(|c| c.tick_at(self.now))
    }

    /// A Play for the stopped free loops waits for the song's next bar
    /// line ([`Command::FreePlay`] while the song plays); Stop calls it
    /// off.
    pub fn free_cued(&self) -> bool {
        self.free_time.cue.is_some()
    }

    /// Clock `clock` as it is now ([`ClockStamp`]): what a snap or a
    /// capture worked out on it keeps, to act only while it holds.
    pub fn stamp(&self, clock: Clock) -> ClockStamp {
        match clock {
            Clock::Song => ClockStamp::SONG,
            Clock::Free => self.free_time.stamp(),
        }
    }

    /// Make song: what it would do now ([`SongPlan`]), for the host to
    /// start the song with, or why it can't. Every free loop with sound
    /// becomes a song loop in its place, and the song takes the free
    /// clock's tempo. While the free loops play, on the free clock's next
    /// bar line on or after frame `after` (the transport's next frame),
    /// where the song starts from the start of the bar it is stopped in;
    /// while they are stopped, at once, their bar 1 on that bar. Not while
    /// the song plays or counts in, while Make song already waits, while a
    /// free loop records or one with sound is busy ([`song_check`]), or
    /// when the free clock's meter isn't the song's (the song keeps its
    /// own). Changes nothing.
    pub fn song_plan(&self, after: u64) -> Result<SongPlan, &'static str> {
        let span = self.last_span.ok_or("the song hasn't run yet")?;
        if span.state.rolling() {
            return Err("the song is playing");
        }
        if self.free_time.song.is_some() {
            return Err("Make song waits for its bar line");
        }
        let clock = self.free_time.clock.ok_or("there are no free loops")?;
        let loops = song_check(&self.loops, &self.prints)?;
        if clock.meter != span.meter {
            return Err("the free loops' meter isn't the song's");
        }
        let bar = span.meter.bar_ticks().max(1);
        let pos = span.ticks().start;
        let tick = pos - pos.rem_euclid(bar);
        let (at, from) = if clock.running {
            let (t, f) = clock
                .next_bar(after)
                .ok_or("the free clock has no bar line")?;
            (Some(f), t)
        } else {
            (None, clock.top)
        };
        Ok(SongPlan {
            at,
            tick,
            shift: tick - from,
            bpm: clock.bpm,
            meter: clock.meter,
            loops,
            stamp: self.free_time.stamp(),
            clock,
        })
    }

    /// Make song `p` ([`Looper::song_plan`]), once the time engine has the
    /// song's start on its frame (with the free loops stopped: its tempo
    /// and place): the loops change clock on that frame. Call it before
    /// [`Looper::process`] runs the period the frame falls in (with the
    /// free loops stopped, the next period). False if Make song already
    /// waits or the free clock has changed since (counted, and said by
    /// [`Looper::take_song`]). Allocation-free.
    pub fn make_song(&mut self, p: SongPlan) -> bool {
        let refused = if self.free_time.song.is_some() {
            Some("Make song waits for its bar line")
        } else if p.stamp != self.free_time.stamp() {
            Some("the free clock started again")
        } else {
            None
        };
        if refused.is_some() {
            self.stats.refused += 1;
            self.free_time.song_done = Some(SongDone { plan: p, refused });
            return false;
        }
        self.free_time.song = Some(p);
        true
    }

    /// The song was stopped at the last period's start: Make song works
    /// out against it ([`Looper::song_plan`]).
    pub fn song_stopped(&self) -> bool {
        self.stopped
    }

    /// Make song waits for its frame ([`Looper::make_song`]).
    pub fn song_waits(&self) -> bool {
        self.free_time.song.is_some()
    }

    /// What became of a Make song given to [`Looper::make_song`], once.
    pub fn take_song(&mut self) -> Option<SongDone> {
        self.free_time.song_done.take()
    }

    /// Make song's Undo: the loops `p` made song loops
    /// ([`Looper::make_song`]) become free loops again, in their places, on
    /// a free clock at the song's tempo with `p`'s bar 1: a copy of the
    /// song's clock while the song plays (they play on, and the host can
    /// then stop the song), else at rest on that bar 1, where they wait.
    /// Now, for the next period. Refused (why, counted) while the song
    /// counts in, while Make song waits, while a free loop holds sound or
    /// records (there is one free clock), when one of the loops has gone
    /// or is busy ([`Loop::clock_refusal`]), or when the song's tempo or
    /// meter isn't the one `p` gave it (the loops' bars are at that
    /// tempo).
    pub fn make_free(&mut self, p: SongPlan) -> Result<(), &'static str> {
        let r = self.free_back(&p);
        if r.is_err() {
            self.stats.refused += 1;
        }
        r
    }

    fn free_back(&mut self, p: &SongPlan) -> Result<(), &'static str> {
        let span = self.last_span.ok_or("the song hasn't run yet")?;
        if span.state == State::CountIn {
            return Err("the song is counting in");
        }
        if self.free_time.song.is_some() {
            return Err("Make song waits for its bar line");
        }
        let free = |l: &Loop| l.clock == Clock::Free && l.phase != Phase::Empty;
        if self.loops.iter().any(|l| free(l) || l.holds_free()) {
            return Err("another free loop has sound");
        }
        if (span.bpm - p.bpm).abs() > 1e-9 || span.meter != p.meter {
            return Err("the song's tempo has changed since");
        }
        for (k, l) in self.loops.iter().enumerate() {
            if p.loops & (1 << k) == 0 {
                continue;
            }
            if l.clock != Clock::Song || l.phase == Phase::Empty {
                return Err("a loop it made has gone");
            }
            let pr = &self.prints[k];
            let print = pr.now.is_some() || matches!(pr.next, Some(Some(_)));
            if let Some(why) = l.clock_refusal(print) {
                return Err(why);
            }
        }
        let clock = if span.state.rolling() {
            FreeClock {
                anchor_frame: span.anchor_frame,
                anchor_tick: span.anchor_tick - p.shift,
                bpm: span.bpm,
                rate: span.rate,
                meter: span.meter,
                running: true,
                top: p.clock.top,
            }
        } else {
            FreeClock::stopped(p.bpm, p.clock.rate, p.meter, p.clock.top)
        };
        let since = span.frame + u64::from(span.len);
        let ft = &mut self.free_time;
        ft.gen = ft.gen.wrapping_add(1);
        ft.clock = Some(clock);
        ft.kept = None;
        ft.ran = None;
        ft.cue = None;
        ft.jump = since;
        ft.since = since;
        ft.song_done = None;
        for (k, l) in self.loops.iter_mut().enumerate() {
            if p.loops & (1 << k) != 0 {
                l.change_clock(Clock::Free, -p.shift);
                l.clock_gen = ft.gen;
                // On a track as a song loop: its cell is a free cell again
                // from this period, so it launches itself from here, with
                // no period run as a free loop on the song's runs told.
                if self.on_track[k].take().is_some() {
                    self.free_tracked |= 1 << k;
                }
            }
        }
        Ok(())
    }

    /// A project's free clock back ([`save::Saved::free_clock`]): at `bpm`
    /// in `meter`, stopped on its bar 1, `top`, for the project's free
    /// loops to load onto ([`Looper::load`]) before the next period, so
    /// that they open stopped at their tops. Refused (false) while a free
    /// loop holds audio or loads, or for a tempo that isn't one. A free
    /// clock no loop holds any more (the last free loop just faded out:
    /// it goes with the next period) gives way to it, and Undo no longer
    /// brings back the loops made on that one.
    pub fn open_free_clock(&mut self, bpm: f64, meter: Meter, top: Tick) -> bool {
        if self.loops.iter().any(Loop::holds_free) || !(bpm.is_finite() && bpm > 0.0) {
            return false;
        }
        let ft = &mut self.free_time;
        ft.gen = ft.gen.wrapping_add(1);
        ft.clock = Some(FreeClock::stopped(bpm, self.rate, meter, top));
        ft.kept = None;
        ft.cue = None;
        true
    }

    /// Once a period, before anything plays: the free clock goes once no
    /// free loop holds audio (it waits for Undo, [`FreeTime::kept`]), and
    /// a free loop armed on the bar starts at once while nothing plays: on
    /// the stopped free clock, which starts from its top with it (the
    /// stopped free loops come in), or with no free clock yet, in your own
    /// time ([`Looper::begin_free`]), its length then setting the free
    /// clock. (Waiting for your note, it starts on the note,
    /// [`Looper::place_hits`]. Recording what the loops play, it waits for
    /// the song or the free loops to play.)
    pub(crate) fn tend_free(&mut self) {
        let ft = &mut self.free_time;
        if ft.clock.is_some() && !self.loops.iter().any(Loop::holds_free) {
            ft.kept = ft.clock.take().map(|c| (c, ft.gen));
            ft.cue = None;
        }
        // Every free loop with audio is on the clock there is (it went only
        // once none held any): a loop cleared now is kept as made on it.
        for l in &mut self.loops {
            if l.clock == Clock::Free {
                l.clock_gen = ft.gen;
            }
        }
        if !self.stopped || self.free_time.rolls_at(self.now) {
            return;
        }
        for k in 0..self.loops.len() {
            let l = &self.loops[k];
            if l.clock == Clock::Free
                && l.phase == Phase::Armed
                && l.starts == Starts::Bar
                && !l.resample
            {
                self.begin_free_loop(k, self.now);
            }
        }
    }

    /// Free loop `k`, armed, starts with nothing playing, its first frame
    /// played on song frame `rec_from` (the period's first, or your
    /// note): on the free clock, which starts from its top on the later
    /// of this period's first frame and that one (the stopped free loops
    /// start with it), or, with no free clock, in your own time
    /// ([`Looper::begin_free`]).
    pub(crate) fn begin_free_loop(&mut self, k: usize, rec_from: u64) {
        let now = self.now;
        let ft = &mut self.free_time;
        let Some(c) = ft.clock.as_mut() else {
            self.begin_free(k, rec_from);
            return;
        };
        let at = rec_from.max(now);
        c.start_at(at);
        ft.jump = at;
        ft.since = at;
        ft.cue = None;
        let c = *c;
        let l = &mut self.loops[k];
        l.clock_gen = ft.gen;
        l.begin_first(
            &mut self.store,
            k,
            &c.span(0, 0, at),
            (c.top, at),
            rec_from,
            0,
            now,
        );
    }

    /// Play for the stopped free loops ([`Command::FreePlay`]): from their
    /// tops on this period's first frame with the song stopped, or, while
    /// it plays, on its next bar line a beat or more on (the period's first
    /// span says where that is).
    pub(crate) fn free_play(&mut self) {
        let ft = &mut self.free_time;
        if !ft.clock.is_some_and(|c| !c.running) {
            return;
        }
        if self.stopped {
            ft.start(self.now);
        } else if ft.cue.is_none() {
            ft.cue = Some((f64::NAN, f64::NAN));
        }
    }

    /// Each free first pass asked to close ([`Loop::ending`]) closes on
    /// the next period's first frame, once it is long enough
    /// ([`read_bars`]): its bars from the start of the bar the song is
    /// stopped in, and its reading for the host ([`Looper::take_tempo`]).
    pub(crate) fn close_free(&mut self, block: &Block) {
        let end = block.start_frame + u64::from(block.frames);
        self.forget_stale_find();
        self.tend_smart(end);
        self.tend_asks(end);
        if !self.wants_notes() {
            if !self.notes.is_empty() || self.notes.dropped() > 0 {
                self.notes.clear();
            }
            if !self.hits.is_empty() || self.hits.dropped() > 0 {
                self.hits.clear();
            }
        }
        if !self.free_open {
            return;
        }
        let Some(span) = block.spans().first() else {
            return;
        };
        let bar = span.meter.bar_ticks().max(1);
        let beat = span.meter.beat_ticks().max(1);
        let hint = TempoHint {
            bpm: span.bpm,
            kind: if self.tempo_meant {
                TempoKind::Set
            } else {
                TempoKind::Earlier
            },
        };
        let rate = self.rate;
        let mut open = false;
        for k in 0..self.loops.len() {
            let l = &self.loops[k];
            if !(l.free && l.phase == Phase::Recording) {
                continue;
            }
            let said = l.bars;
            let read = |len: usize| read_bars(len, rate, bar, said, Some(hint));
            let rec_from = l.rec_from;
            let len = end.saturating_sub(rec_from) as usize;
            let closing = if l.ending { read(len) } else { None };
            let Some(reading) = closing else {
                open = true;
                continue;
            };
            // The tap: the frame it closes on without smart loop end.
            let tap = *self.loops[k].tap.get_or_insert(end);
            let tap_len = (tap - rec_from) as usize;
            let tap_reading = read(tap_len).unwrap_or(reading);
            let reach = ends::reach(rate, tap_reading.bpm);
            // The reading with the beat inside the loop as evidence: the
            // notes played into it.
            let heard = |len: usize| {
                if self.by_ear {
                    let inner = InnerBeat::new(&self.hits, rate, rec_from, len, bar, beat);
                    read_bars_with(len, rate, bar, said, Some(hint), &|b| inner.fit(b))
                } else {
                    let inner = InnerBeat::new(&self.notes, rate, rec_from, len, bar, beat);
                    read_bars_with(len, rate, bar, said, Some(hint), &|b| inner.fit(b))
                }
            };
            // Auto listens on to the end of the reach first.
            let mut auto = None;
            let (len, reading, lead) = if self.smart == SmartEnd::Auto {
                // By ear, what is heard reaches the writer a latency later.
                let behind = if self.by_ear {
                    u64::from(self.loops[k].latency)
                } else {
                    0
                };
                if end < tap + reach + behind {
                    self.loops[k].closing = true;
                    open = true;
                    continue;
                }
                let ask = ends::Ask {
                    rate,
                    start: rec_from,
                    tap,
                    reach,
                    heard_to: end - behind,
                    bar,
                    beat,
                };
                let tap_reading = heard(tap_len).unwrap_or(tap_reading);
                let bars = tap_reading.bars;
                let other = tap_reading.other.map(|(b, _)| b);
                let d = if self.by_ear {
                    // By ear: the finder's answer, waited for a little.
                    let wait = ms_frames(rate, FIND_WAIT_AUTO_MS);
                    match self.find_for(k, ask, bars, end, wait) {
                        Some(d) => d,
                        None => {
                            self.loops[k].closing = true;
                            open = true;
                            continue;
                        }
                    }
                } else {
                    // A bar count in doubt must agree on the end.
                    ends::decide_agreed(&self.notes, &ask, bars, other)
                };
                // A smart start (your first hit came just after REC) needs
                // what was heard after the end kept: room for it.
                let room = d.lead > 0
                    && self.store.grow(
                        k,
                        0,
                        blocks_for(KEEP_BEFORE + (end - rec_from) as usize + KEEP_AFTER),
                    );
                let first = rec_from + d.lead;
                let from_hit = |to: u64| reread((to - first) as usize, rate, bar, &tap_reading);
                // Moved, the beat's fit is the loop's as it now ends (the
                // reading weighed it at the tap's length).
                let moved = (d.verdict == Verdict::Moved)
                    .then(|| from_hit(d.end))
                    .flatten()
                    .map(|r| Reading {
                        beat: r.beat.and(d.best.map(|b| b.rhythm)),
                        ..r
                    });
                // The loop's length and its start in the pass. Ends are
                // measured from the first hit: with no smart start the loop
                // is as long from the press.
                let (len, lead, reading, outcome) = match moved {
                    Some(r) => {
                        let back = if room { 0 } else { d.lead };
                        let len = (d.end - first) as usize;
                        let by = (d.end - back) as i64 - tap as i64;
                        (len, d.lead - back, r, SmartOutcome::Moved(by))
                    }
                    None => {
                        let why = SmartOutcome::Kept(if d.verdict == Verdict::Moved {
                            "too short a loop"
                        } else {
                            d.verdict.why_by(self.by_ear)
                        });
                        match from_hit(tap).filter(|_| room) {
                            // On your first hit, ending on your tap.
                            Some(r) => ((tap - first) as usize, d.lead, r, why),
                            None => (tap_len, 0, tap_reading, why),
                        }
                    }
                };
                auto = Some((outcome, d));
                (len, reading, lead as usize)
            } else {
                (len, heard(len).unwrap_or(reading), 0)
            };
            // A song loop's bars count from the start of the bar the song
            // stopped in; a free loop's from the free clock's bar 1.
            let free_loop = self.loops[k].clock == Clock::Free;
            let pos = span.ticks().start;
            let origin = if free_loop {
                0
            } else {
                pos - pos.rem_euclid(bar)
            };
            let ticks = i64::from(reading.bars) * bar;
            let t = origin + ticks;
            // Length's bars are final: a reading that couldn't keep them
            // closes the pass, then waits for you (Status::asks), silent
            // and with no tempo set; Then: Overdub waits with it.
            let asks = reading.overruled();
            let l = &mut self.loops[k];
            if asks {
                l.then_overdub = false;
            }
            l.free = false;
            l.closing = false;
            l.origin = origin;
            l.bar = bar;
            l.end_tick = Some(t);
            // (Suggest's offer moves the song's tempo, or a free loop's the
            // free clock's.)
            if self.smart == SmartEnd::Suggest {
                l.smart_wait = Some(SmartWait {
                    tap,
                    reach,
                    start: rec_from,
                    bar,
                    beat,
                    reading,
                });
            }
            if !l.finish_len(&mut self.store, &mut self.stats, k, t, len, true) {
                l.forget_smart();
                continue;
            }
            // A smart start (Auto): the loop starts on your first hit, as
            // long, ending on `stop`.
            let start = if lead > 0 && l.tail && l.start_on(lead) {
                lead
            } else {
                0
            };
            // Auto listened on past the end: the song starts as far into
            // the loop as it came round since, and with Then: Overdub,
            // what you played since begins the overdub.
            let into = end.saturating_sub(rec_from + (start + len) as u64);
            let into = (into as f64 * ticks as f64 / len.max(1) as f64).round() as Tick;
            if let Some((outcome, d)) = auto {
                l.od_handoff = l.od_open;
                let found = Shift {
                    start: start as i64,
                    end: (rec_from + (start + len) as u64) as i64 - tap as i64,
                };
                l.found = Some(found).filter(|f| !f.is_none());
                l.moved = found;
                l.smart_version = l.version;
                // (Where the start couldn't move, the end is as far from
                // the press.)
                let outcome = match outcome {
                    SmartOutcome::Moved(_) => SmartOutcome::Moved(found.end),
                    o => o,
                };
                self.smart_said(k, tap, outcome, start as i64, Some((&d, &reading)));
            }
            let into = into.clamp(0, ticks - 1);
            if asks {
                self.loops[k].asks = Some(reading);
            }
            if !free_loop {
                if !asks {
                    self.tempo_out = Some((k, reading, end, into));
                }
                continue;
            }
            // A free loop sets the free clock instead, and the song never
            // hears of it: the tempo the loop's length read as, bar 1 on
            // its start, running from the frame it comes round on (as far
            // in as Auto listened on past its end), or, closed by STOP,
            // stopped on its bar 1.
            let mut c = FreeClock {
                anchor_frame: end,
                anchor_tick: origin + if asks { 0 } else { into },
                bpm: reading.bpm,
                rate,
                meter: span.meter,
                running: true,
                top: origin,
            };
            let l = &mut self.loops[k];
            if std::mem::take(&mut l.stop_close) || asks {
                c.stop();
            }
            let ft = &mut self.free_time;
            ft.gen = ft.gen.wrapping_add(1);
            ft.clock = Some(c);
            ft.jump = end;
            ft.since = end;
            l.clock_gen = ft.gen;
        }
        self.free_open = open;
    }
}
