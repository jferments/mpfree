//! The looper's side of smart loop end: the host's asks, the notes
//! it hears, the finds it runs and what it decides.

use super::*;

impl Looper {
    /// A loop waiting for you to play heard a note while the song was
    /// stopped: the host should start the song now (bar 1 from the bar it
    /// stopped in, at the next period's first frame), and the loop's first
    /// pass starts with it, on the note ([`Starts::Play`]). Some(loop)
    /// once per note; [`Looper::start_refused`] if the song can't start.
    pub fn take_start(&mut self) -> Option<usize> {
        let n = self.loops.iter().position(|l| l.want_start)?;
        self.loops[n].want_start = false;
        Some(n)
    }

    /// The song couldn't start on the note ([`Looper::take_start`]): the
    /// loop forgets it and listens again. (The host counts it, and says
    /// why.)
    pub fn start_refused(&mut self) {
        for l in &mut self.loops {
            if l.phase == Phase::Armed && l.pending.is_some_and(|(_, pickup)| !pickup) {
                l.unhear();
            }
        }
    }

    /// A free first pass closed ([`Command::TempoFromLoop`]): its loop, how
    /// its length read (its bars and the tempo they set, how sure, the
    /// other reading), the frame the host starts the song on at that tempo
    /// (the next period's first), and where: this many ticks after the
    /// start of the bar it stopped in (the loop's first bar). 0, where the
    /// loop comes round, unless smart loop end's Auto listened on past
    /// the end it closed on: then the loop is already that far in, and
    /// the song starts there with it. Once per pass. If the song can't
    /// start, the loop plays at the song's tempo when it does (like tape,
    /// until a print is in).
    pub fn take_tempo(&mut self) -> Option<(usize, Reading, u64, Tick)> {
        self.tempo_out.take()
    }

    /// The tempo the song takes from the next period, while
    /// [`Looper::take_tempo`] hasn't been taken: a free first pass closed
    /// in this one.
    pub fn tempo_coming(&self) -> Option<f64> {
        self.tempo_out.as_ref().map(|(_, r, _, _)| r.bpm)
    }

    /// What smart loop end made of a closing tap ([`Command::SetSmartEnd`]),
    /// once per pass: Auto's when the loop closes, Suggest's at the end of
    /// the reach after it.
    pub fn take_smart(&mut self) -> Option<SmartNote> {
        self.smart_out.take()
    }

    /// Snap `s` on loop `n` ([`Snap`]), worked out against where the song
    /// was going at the end of the last period: how far the start and the
    /// end move, the loop's length in song ticks after it (Read as's bars
    /// of the song's meter), the tempo at which those ticks are its new
    /// length exactly, and the loop's next downbeat on or after frame
    /// `after` (the transport's next frame, which may be a period on from
    /// the looper's), its tick and frame. The host asks the time engine
    /// for that tempo on that frame and, once committed, gives the plan to
    /// [`Looper::snap_at`]. Or why it can't: the song isn't playing, the
    /// loop isn't playing as it was left, it plays at another speed, it is
    /// already read as those bars, or the new length is outside what was
    /// kept or the tempo range. Changes nothing.
    ///
    /// A free loop's ([`Clock::Free`]) is worked out against the free
    /// clock, whose tempo it changes instead of the song's: the host hands
    /// it straight to [`Looper::snap_at`], with nothing to ask of the time
    /// engine. Or why not: the free loops aren't playing, or another free
    /// loop has sound (its tempo would change under it).
    pub fn snap_plan(&self, n: usize, s: Snap, after: u64) -> Result<SnapPlan, &'static str> {
        let l = self.loops.get(n).ok_or("there is no such loop")?;
        let span = if l.clock == Clock::Free {
            if (0..self.loops.len()).any(|k| k != n && self.loops[k].holds_free()) {
                return Err("another free loop has sound");
            }
            let at = self.heard.at();
            self.free_time
                .clock
                .filter(|c| c.rolls_at(at))
                .map(|c| c.span(0, 0, at))
                .ok_or("the free loops aren't playing")?
        } else {
            self.last_span
                .filter(|s| matches!(s.state, State::Playing | State::Recording))
                .ok_or("the song isn't playing")?
        };
        if l.phase != Phase::Playing || l.od_open || l.busy() {
            return Err("the loop is busy");
        }
        if l.speed != 0 || l.rate != 0 {
            return Err("the loop plays at another speed");
        }
        let by = l.snap_by(s)?;
        let ticks = l.snap_ticks(s, span.meter.bar_ticks())?;
        let len = l.len as i64 + by.longer();
        let fits = match s {
            Snap::Found | Snap::Tap | Snap::Press | Snap::To(_) => l.shifted(by).is_some(),
            Snap::Bars(_) | Snap::Undo | Snap::Redo => true,
        };
        if !fits && l.short && by.end > 0 {
            return Err(MEMORY_FULL);
        }
        if !fits || len <= 0 || ticks <= 0 || l.len_ticks <= 0 {
            return Err("that end is outside what the loop kept");
        }
        let bpm = 60.0 * f64::from(self.rate) * ticks as f64 / (f64::from(PPQ) * len as f64);
        if !(LOOP_MIN_BPM..=LOOP_MAX_BPM).contains(&bpm) {
            return Err("its tempo would be out of range");
        }
        // The next downbeat from the coming period's first frame on, by
        // the span's clock (as `Span::tick_at` and `Span::frame_of` work it
        // out, the frame rounded the same way, so the tempo's anchor falls
        // on that tick).
        let next = after.max(span.frame + u64::from(span.len));
        let (rate, ppq) = (f64::from(span.rate), f64::from(PPQ));
        let t = span.anchor_tick as f64
            + (next as f64 - span.anchor_frame as f64) * span.bpm * ppq / (60.0 * rate);
        let k = ((t - l.origin as f64) / l.len_ticks as f64).floor() as i64 + 1;
        let tick = l.origin + k * l.len_ticks;
        let f = (tick as f64 - span.anchor_tick as f64) * 60.0 * rate / (span.bpm * ppq);
        let at = (span.anchor_frame as i64 + f.round() as i64).max(0) as u64;
        Ok(SnapPlan {
            n,
            snap: s,
            by,
            ticks,
            bpm,
            tick,
            at: at.max(next),
            version: l.version,
            stamp: self.stamp(l.clock),
        })
    }

    /// Land snap `p` ([`Looper::snap_plan`]) on its frame, now that the
    /// time engine has committed its tempo there: from then the loop
    /// waits for nothing else (commands that change it wait until it has
    /// landed). Call it before [`Looper::process`] runs the period its
    /// frame falls in. False if the loop has changed since (counted, and
    /// said by [`Looper::take_snap`]). Allocation-free. A free loop's
    /// needs no time engine: the free clock takes its tempo on that frame
    /// as the snap lands, its bar 1 on the loop's downbeat (if the free
    /// loops have stopped by then, it doesn't land).
    pub fn snap_at(&mut self, p: SnapPlan) -> bool {
        let Some(l) = self.loops.get_mut(p.n) else {
            return false;
        };
        if l.version != p.version || l.phase != Phase::Playing || l.snap.is_some() {
            self.stats.refused += 1;
            l.snap_done = Some(SnapDone {
                plan: p,
                landed: false,
            });
            return false;
        }
        l.snap = Some(p);
        l.snap_frame = p.at;
        true
    }

    /// What became of a snap given to [`Looper::snap_at`], once.
    pub fn take_snap(&mut self) -> Option<SnapDone> {
        self.loops.iter_mut().find_map(|l| l.snap_done.take())
    }

    /// A note started (a pad hit or a MIDI note-on, [`NoteStart`]), on the
    /// song's frame clock: smart loop end weighs them ([`ends`]). Give
    /// every note of a period before [`Looper::process`] runs it. Kept
    /// only while it could matter (a free first pass armed, recording or
    /// waiting for its decision); allocation-free.
    pub fn note(&mut self, n: NoteStart) {
        if self.takes_notes() {
            self.notes.push(n);
        }
    }

    /// A note started on `at` that was lost before it got here (the
    /// host had no room for it): a tap whose reach it falls in stands,
    /// as for notes the log itself had to drop.
    pub fn notes_lost(&mut self, at: u64) {
        if self.takes_notes() {
            self.notes.lost(at);
        }
    }

    /// Whether a note given now is kept: tempo from the first loop is on
    /// (the reading weighs the beat, and smart loop end the end), or a
    /// decision waits. (A note in the period
    /// that arms the loop counts: the period's end forgets what nothing
    /// needs, [`Looper::wants_notes`].)
    fn takes_notes(&self) -> bool {
        self.free_tempo
            || self
                .loops
                .iter()
                .any(|l| l.smart_wait.is_some() || l.clock == Clock::Free)
    }

    /// Whether the notes are wanted: a free first pass may be armed or
    /// recording (its reading weighs the beat inside it, and smart loop
    /// end, when on, its end), or one waits for smart loop end's decision.
    pub(crate) fn wants_notes(&self) -> bool {
        self.loops.iter().any(|l| {
            l.smart_wait.is_some()
                || ((self.free_tempo || l.clock == Clock::Free)
                    && (l.phase == Phase::Armed || (l.free && l.phase == Phase::Recording)))
        })
    }

    /// The first passes waiting on a Length they couldn't keep
    /// ([`Status::asks`]): [`Command::UseReading`] starts one from its top
    /// on `end`, the next period's first frame, a free loop on the free
    /// clock (as [`Command::FreePlay`]), a song loop with the song at the
    /// tempo it read ([`Looper::take_tempo`]). Its clock started by other
    /// means (PLAY, or Play for the free loops) answers for you: it plays
    /// as read, a song loop at the song's tempo.
    pub(crate) fn tend_asks(&mut self, end: u64) {
        for k in 0..self.loops.len() {
            let l = &mut self.loops[k];
            let Some(r) = l.asks else {
                continue;
            };
            let ft = &mut self.free_time;
            let started = match l.clock {
                Clock::Free => ft.gen == l.clock_gen && ft.clock.is_some_and(|c| c.running),
                Clock::Song => !self.stopped,
            };
            if !started && !l.asks_yes {
                continue;
            }
            l.asks = None;
            l.asks_yes = false;
            if started {
                continue;
            }
            match l.clock {
                Clock::Free if self.stopped => ft.start(end),
                Clock::Free => ft.cue = ft.cue.or(Some((f64::NAN, f64::NAN))),
                Clock::Song => self.tempo_out = Some((k, r, end, 0)),
            }
        }
    }

    /// Suggest: a closed free pass whose reach has been heard to its end
    /// gets its decision. With an overdub under way (the loop's Then, or
    /// Overdub pressed) the tap stands: Suggest can't wait for you there.
    pub(crate) fn tend_smart(&mut self, end: u64) {
        for k in 0..self.loops.len() {
            let Some(w) = self.loops[k].smart_wait else {
                continue;
            };
            if self.loops[k].od_open {
                self.loops[k].smart_wait = None;
                self.smart_said(k, w.tap, SmartOutcome::Kept(OVERDUB_AT_ONCE), 0, None);
                continue;
            }
            // (And for the writer, a latency behind, to have it.)
            if end < w.tap + w.reach + u64::from(self.loops[k].rec_latency) {
                continue;
            }
            let (rate, bar) = (self.rate, w.bar);
            let ask = ends::Ask {
                rate,
                start: w.start,
                tap: w.tap,
                reach: w.reach,
                // (By ear, what the writer has: a latency behind.)
                heard_to: if self.by_ear {
                    end.saturating_sub(u64::from(self.loops[k].rec_latency))
                } else {
                    end
                },
                bar,
                beat: w.beat,
            };
            let bars = w.reading.bars;
            let d = if self.by_ear {
                // By ear: the finder's answer, once it comes.
                let wait = ms_frames(rate, FIND_WAIT_SUGGEST_MS);
                match self.find_for(k, ask, bars, end, wait) {
                    Some(d) => d,
                    None => continue,
                }
            } else {
                let other = w.reading.other.map(|(b, _)| b);
                ends::decide_agreed(&self.notes, &ask, bars, other)
            };
            self.loops[k].smart_wait = None;
            let by = d.moved_by(w.tap);
            let audible = |f: i64| f.unsigned_abs() as f64 * 1000.0 / f64::from(rate) >= OFFER_MS;
            let first = w.start + d.lead;
            let reads = reread((d.end - first) as usize, rate, bar, &w.reading).is_some();
            let moved = d.verdict == Verdict::Moved && reads;
            // What Snap would make: the end found, and a smart start (your
            // first hit came just after your press), or else the end as far
            // from the press, when the loop kept the audio for them.
            let end = if moved && audible(by) { by } else { 0 };
            let lead = d.lead as i64;
            let start = if audible(lead) { lead } else { 0 };
            let l = &mut self.loops[k];
            let fits = |f: Shift| f.is_none() || l.shifted(f).is_some();
            let without = Shift {
                start: 0,
                end: if end != 0 { end - lead } else { 0 },
            };
            let found = [Shift { start, end }, without]
                .into_iter()
                .find(|&f| fits(f))
                .unwrap_or(Shift::NONE);
            let outcome = match d.verdict {
                Verdict::Moved if !reads => SmartOutcome::Kept("too short a loop"),
                Verdict::Moved if end == 0 => SmartOutcome::Close(by),
                // (Later than the loop kept: memory was full, or else.)
                Verdict::Moved if found.end == 0 => SmartOutcome::Kept(if l.short && end > 0 {
                    MEMORY_FULL
                } else {
                    "that end is outside what the loop kept"
                }),
                Verdict::Moved => SmartOutcome::Offered(found.end),
                v => SmartOutcome::Kept(v.why_by(self.by_ear)),
            };
            if !found.is_none() {
                l.found = Some(found);
                l.moved = Shift::NONE;
                l.smart_version = l.version;
            }
            self.smart_said(k, w.tap, outcome, found.start, Some((&d, &w.reading)));
        }
    }

    /// Smart loop end by ear: loop `k`'s decision on `ask` from the
    /// finder once it has answered, asking it first (the host hands the
    /// ask over, [`Looper::find_job`]). None while it waits, until `wait`
    /// frames past the period that asked (`now`); then your tap stands
    /// ([`find::late`]). Counted either way.
    pub(crate) fn find_for(
        &mut self,
        k: usize,
        ask: ends::Ask,
        bars: u32,
        now: u64,
        wait: u64,
    ) -> Option<ends::Decision> {
        match self.find {
            Some(f) if f.k == k => {
                if let Some(d) = f.answer {
                    self.find = None;
                    self.stats.finds_answered += 1;
                    Some(d)
                } else if now >= f.due {
                    self.find = None;
                    self.stats.finds_late += 1;
                    Some(find::late(&f.ask))
                } else {
                    None
                }
            }
            _ => {
                self.find_gen += 1;
                self.find = Some(Finding {
                    k,
                    gen: self.find_gen,
                    ask,
                    bars,
                    due: now + wait,
                    sent: false,
                    answer: None,
                });
                self.stats.finds_asked += 1;
                None
            }
        }
    }

    /// The finder's ask is dropped once its take no longer waits for it
    /// (cleared, thrown away, its overdub started): a late answer to it
    /// is stale.
    pub(crate) fn forget_stale_find(&mut self) {
        let Some(f) = self.find else {
            return;
        };
        let l = &self.loops[f.k];
        let waits = l.smart_wait.is_some() || (l.free && l.phase == Phase::Recording);
        if !waits {
            self.find = None;
        }
    }

    /// Smart loop end by ear wants the finder: `job` gets the take's
    /// generation, the ask, the hits heard, and the first pass's audio
    /// round the opening hit and round the reach of the tap, and true,
    /// once per ask. The host hands it to the finder's thread
    /// ([`find::find`]) and its answer back ([`Looper::found`]).
    /// Allocation-free.
    pub fn find_job(&mut self, job: &mut FindJob) -> bool {
        let Some(f) = self.find.as_mut().filter(|f| !f.sent) else {
            return false;
        };
        f.sent = true;
        let f = *f;
        job.gen = f.gen;
        job.ask = f.ask;
        job.bars = f.bars;
        job.hits.clone_from(&self.hits);
        job.answer = None;
        job.line_up = 0.0;
        let opening = {
            let mut buf = [hits::Hit::NONE; ends::HEAD + ends::TAIL];
            let n = self.hits.sorted(&mut buf);
            buf[..n]
                .iter()
                .map(|h| h.frame)
                .find(|&x| x >= f.ask.start && x <= f.ask.start + f.ask.reach)
                .unwrap_or(f.ask.start)
        };
        job.open_from = FindJob::open_from_for(opening);
        job.open_len = self.kept_mono(f.k, job.open_from, f.ask.heard_to, &mut job.open);
        job.near_from = FindJob::near_from_for(f.ask.tap, f.ask.reach);
        job.near_len = self.kept_mono(f.k, job.near_from, f.ask.heard_to, &mut job.near);
        true
    }

    /// Whether the looper has asked the finder something the host hasn't
    /// taken yet ([`Looper::find_job`]): an ask that waits for the job to
    /// come back from the finder's thread.
    pub fn find_wanted(&self) -> bool {
        self.find.is_some_and(|f| !f.sent)
    }

    /// The finder's answer to `job`: kept for its take if the looper still
    /// waits for it (the next period uses it), else dropped as stale.
    pub fn found(&mut self, job: &FindJob) {
        match self.find.as_mut() {
            Some(f) if f.gen == job.gen && f.answer.is_none() => f.answer = job.answer,
            _ => self.stats.finds_stale += 1,
        }
    }

    /// Loop `k`'s first pass as recorded, from song frame `from`, in
    /// `out` as mono (the channels' mean), up to song frame `to`: how many
    /// frames from the first are heard (0 before what it kept; silence
    /// after). While it records or after. Allocation-free.
    fn kept_mono(&self, k: usize, from: u64, to: u64, out: &mut [f32]) -> usize {
        out.fill(0.0);
        let Some(l) = self.loops.get(k) else {
            return 0;
        };
        // Frame 0 of what it kept is KEEP_BEFORE before its first.
        let base = l.rec_from as i64 - KEEP_BEFORE as i64;
        let (v0, vn) = if l.phase == Phase::Recording {
            (0, KEEP_BEFORE + l.written)
        } else {
            l.slots[0].valid
        };
        let blocks = self.store.blocks_of(k, 0);
        let mut heard = 0;
        for (i, y) in out.iter_mut().enumerate() {
            let f = from + i as u64;
            let q = f as i64 - base;
            if f >= to || q >= (v0 + vn) as i64 {
                break;
            }
            if q < v0 as i64 {
                continue;
            }
            let q = q as usize;
            let x = self.store.frame(blocks, q / BLOCK, q % BLOCK);
            *y = 0.5 * (x[0] + x[1]);
            heard = i + 1;
        }
        heard
    }

    /// Smart loop end's word on loop `k`'s tap, for the host, counted.
    pub(crate) fn smart_said(
        &mut self,
        k: usize,
        tap: u64,
        outcome: SmartOutcome,
        start: i64,
        decided: Option<(&ends::Decision, &Reading)>,
    ) {
        match outcome {
            SmartOutcome::Moved(_) => self.stats.ends_moved += 1,
            SmartOutcome::Offered(_) => self.stats.ends_offered += 1,
            SmartOutcome::Kept(_) => self.stats.taps_kept += 1,
            SmartOutcome::Close(_) => {}
        }
        if start != 0 {
            self.stats.starts_found += 1;
        }
        self.smart_out = Some(SmartNote {
            n: k,
            outcome,
            start,
            tap,
            best: decided.and_then(|(d, _)| d.best),
            second: decided.and_then(|(d, _)| d.second),
            sure: decided.map(|(d, r)| Confidence::of(d, r)),
        });
    }

    /// Each loop waiting for you to play that a note came to: where its
    /// first pass starts ([`Starts::Play`]). Song stopped: it asks for the
    /// song to start ([`Looper::take_start`]) and starts with it. Counting
    /// in: the note leads into bar 1. Playing: on the bar line nearest the
    /// note: in the first half of a bar, that bar's line (passed: the
    /// frames since are copied in from what was heard), else the next
    /// line, with the note and what followed it as a pickup.
    pub(crate) fn place_hits(&mut self, block: &Block) {
        for k in 0..self.loops.len() {
            let Some(p) = self.loops[k].hit.take() else {
                continue;
            };
            if self.loops[k].phase != Phase::Armed {
                continue;
            }
            let Some(span) = block.spans().first() else {
                continue;
            };
            let free = self.loops[k].clock == Clock::Free;
            // A free loop while the free clock runs: on its bar lines.
            let now = block.start_frame;
            if let Some(c) = self.free_time.clock.filter(|c| free && c.rolls_at(now)) {
                let fs = c.span(0, block.frames, now);
                self.place_on(k, p, &fs, free_around(&c, p), now);
                continue;
            }
            match span.state {
                State::Playing | State::Recording => self.place_hit(k, p, block),
                State::CountIn => self.loops[k].pending = Some((p, true)),
                // A free loop with nothing playing: on the stopped free
                // clock, or in your own time.
                State::Stopped if free => self.begin_free_loop(k, p),
                // Tempo from the loop: it records free from the note.
                State::Stopped if self.frees(k) => self.begin_free(k, p),
                State::Stopped => {
                    self.loops[k].pending = Some((p, false));
                    self.loops[k].want_start = true;
                }
            }
        }
    }

    /// [`Looper::place_hits`] for loop `k`, a note played on song frame
    /// `p`, the song playing.
    fn place_hit(&mut self, k: usize, p: u64, block: &Block) {
        let Some(span) = block
            .spans()
            .iter()
            .find(|s| matches!(s.state, State::Playing | State::Recording))
        else {
            return;
        };
        let around = self.heard.around(p).unwrap_or((None, None));
        self.place_on(k, p, span, around, block.start_frame);
    }

    /// Loop `k`, a note played on song frame `p`, on the bar lines of
    /// `span` (the song's, or the free clock's), `around` being the last
    /// one at or before the note and the next: it starts on the nearest,
    /// `now` being the period's first frame.
    fn place_on(&mut self, k: usize, p: u64, span: &Span, around: Around, now: u64) {
        let bar = span.meter.bar_ticks();
        let (before, after) = around;
        // The line to start on, and whether the note is a pickup before it.
        let (line, pickup) = match before {
            Some((t, f)) => {
                let next = after.unwrap_or_else(|| (t + bar, span.frame_of((t + bar) as f64)));
                if p - f >= next.1.saturating_sub(f) / 2 {
                    (next, true)
                } else if next.1 <= now {
                    // Its bar is over already (a note heard that late is
                    // not a start): listen again.
                    return;
                } else {
                    ((t, f), false)
                }
            }
            None => match after {
                Some(next) => (next, true),
                // No line heard yet in this run: the next one.
                None => {
                    self.loops[k].pending = Some((p, true));
                    return;
                }
            },
        };
        if line.1 > now {
            // Still to come: the bar line starts it.
            self.loops[k].pending = Some((p, pickup));
            return;
        }
        // Passed: it starts there now, from what was heard.
        let half = span.frame_of((line.0 + bar) as f64).saturating_sub(line.1) / 2;
        let (rec_from, lead) = if pickup {
            let early = line.1.saturating_sub(p);
            if early > half {
                // Too long before the line to be its pickup: listen again.
                return;
            }
            (line.1 - early, early as usize)
        } else {
            (line.1, 0)
        };
        self.begin_on(k, span, line, rec_from, lead, now);
    }

    /// Loop `k`'s first pass begins on bar line `line` of `span`, the clock
    /// it goes by ([`Loop::begin_first`]). A free loop that waited for the
    /// song's bar line sets the free clock there first
    /// ([`FreeTime::set_on_song`]), and begins on that.
    fn begin_on(
        &mut self,
        k: usize,
        span: &Span,
        line: (Tick, u64),
        rec_from: u64,
        lead: usize,
        write_at: u64,
    ) {
        let ft = &mut self.free_time;
        let l = &mut self.loops[k];
        if l.clock == Clock::Free {
            if !ft.rolls_at(self.now) {
                let (c, at) = ft.set_on_song(span, line);
                l.clock_gen = ft.gen;
                let fs = c.span(0, 0, line.1);
                l.begin_first(&mut self.store, k, &fs, at, rec_from, lead, write_at);
                return;
            }
            l.clock_gen = ft.gen;
        }
        l.begin_first(&mut self.store, k, span, line, rec_from, lead, write_at);
    }
}
