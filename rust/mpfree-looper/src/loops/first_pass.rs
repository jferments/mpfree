//! The first pass: its start, its writer and its end.

use super::*;

impl Loop {
    /// The first pass starts on bar line `t` at frame `f` (`line`), its
    /// first frame played on song frame `rec_from` (`f`, or before it
    /// with a pickup of `lead` frames or a start from a note). The writer
    /// takes the input from frame `write_at` on; what was heard before
    /// that copies in ([`Fill`]).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn begin_first(
        &mut self,
        store: &mut Store,
        me: usize,
        span: &Span,
        line: (Tick, u64),
        rec_from: u64,
        lead: usize,
        write_at: u64,
    ) {
        let (t, f) = line;
        // The new loop takes the place of one kept for Undo.
        store.drop_kept(me);
        self.phase = Phase::Recording;
        self.origin = t;
        self.start_frame = f;
        self.bar = span.meter.bar_ticks();
        self.last_bar = (t, f);
        self.next_bar = span.frame_of((t + self.bar) as f64);
        self.end_tick = self.bars.map(|b| t + i64::from(b) * self.bar);
        self.ending = false;
        self.then_overdub = self.after == After::Overdub && !self.resample;
        self.first_open = true;
        self.rec_latency = if self.resample { 0 } else { self.latency };
        self.len = 0;
        self.written = 0;
        self.rec_from = rec_from;
        self.lead = lead;
        self.unhear();
        let from = rec_from + u64::from(self.rec_latency);
        self.keep_before(from);
        self.fill = (from < write_at).then(|| Fill {
            from,
            done: 0,
            to: (write_at - from) as usize,
            at: write_at,
        });
    }

    /// The first pass's first frame is the one heard at `from`: what was
    /// heard before it copies in as its first [`KEEP_BEFORE`] frames
    /// ([`Loop::render_before`]). Not for a resample, which records what
    /// the other loops play, not what was heard: it keeps the input
    /// before it for the loop point, as the writer saw it
    /// ([`Loop::keep_pre`]).
    fn keep_before(&mut self, from: u64) {
        self.tail = false;
        self.before = (!self.resample).then_some((from, 0));
    }

    /// The first pass starts free ([`Command::TempoFromLoop`]) on song
    /// frame `rec_from`, `now` being the period's first: no bars or bar
    /// lines until it closes ([`Looper::close_free`]). A start before
    /// `now` (a note) copies in from what was heard, as
    /// [`Loop::begin_first`]'s does.
    pub(crate) fn begin_free(
        &mut self,
        store: &mut Store,
        me: usize,
        rec_from: u64,
        now: u64,
        bar: Tick,
    ) {
        store.drop_kept(me);
        self.phase = Phase::Recording;
        self.free = true;
        self.forget_smart();
        self.origin = 0;
        self.start_frame = rec_from;
        self.bar = bar;
        self.last_bar = (0, rec_from);
        self.next_bar = u64::MAX;
        self.end_tick = None;
        self.ending = false;
        self.then_overdub = self.after == After::Overdub;
        self.first_open = true;
        self.rec_latency = self.latency;
        self.len = 0;
        self.written = 0;
        self.rec_from = rec_from;
        self.lead = 0;
        self.unhear();
        let from = rec_from + u64::from(self.rec_latency);
        self.keep_before(from);
        self.fill = (from < now).then(|| Fill {
            from,
            done: 0,
            to: (now - from) as usize,
            at: now,
        });
    }

    /// The first pass's frames heard before it began, copied in at
    /// [`LOAD_SPEED`] from what was heard. Thrown away (counted) if what
    /// was heard has moved past them or there is no memory.
    pub(crate) fn render_fill(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        heard: &Heard,
        end: u64,
    ) {
        let Some(mut fl) = self.fill else {
            return;
        };
        let due = end.saturating_sub(fl.at).saturating_mul(LOAD_SPEED as u64);
        let target = fl.to.min(due.min(usize::MAX as u64) as usize);
        if fl.done < target {
            if !heard.holds(fl.from + fl.done as u64)
                || !store.grow(me, 0, (KEEP_BEFORE + target - 1) / BLOCK + 1)
            {
                stats.aborted += 1;
                self.reset();
                self.reclaim(store, me);
                return;
            }
            for q in fl.done..target {
                store.set(me, 0, KEEP_BEFORE + q, heard.frame(fl.from + q as u64));
            }
            fl.done = target;
            self.written = self.written.max(target);
        }
        self.fill = (fl.done < fl.to).then_some(fl);
    }

    /// What was heard before the first pass, copied in as its first
    /// [`KEEP_BEFORE`] frames as it comes ([`Loop::keep_before`]):
    /// silence where nothing was heard (the input had just started). The
    /// pass is thrown away (counted) if there is no memory.
    pub(crate) fn render_before(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        heard: &Heard,
    ) {
        let Some((from, done)) = self.before else {
            return;
        };
        // Frame `q` kept is the one heard at `from - KEEP_BEFORE + q`.
        let first = from as i64 - KEEP_BEFORE as i64;
        let target = (heard.at() as i64 - first).clamp(0, KEEP_BEFORE as i64) as usize;
        if done >= target {
            return;
        }
        if !store.grow(me, 0, (target - 1) / BLOCK + 1) {
            stats.aborted += 1;
            self.reset();
            self.reclaim(store, me);
            return;
        }
        for q in done..target {
            let g = first + q as i64;
            let x = if g >= 0 {
                heard.frame(g as u64)
            } else {
                [0.0; 2]
            };
            store.set(me, 0, q, x);
        }
        self.before = (target < KEEP_BEFORE).then_some((from, target));
    }

    pub(crate) fn finish_first(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        t: Tick,
        f: u64,
    ) {
        let len = (f - self.start_frame) as usize;
        self.finish_len(store, stats, me, t, len, true);
    }

    /// STOP during the first pass (the transport stopped at `tick`, frame
    /// `f`): what was played is kept, and nothing after STOP. The loop is
    /// whole bars, so it stays on the song's bar lines: up to the next bar
    /// line, silent after the cut, or back to the bar line just passed when
    /// STOP came within a 1/16 note after it (pressed a moment late). With
    /// no whole bar left that way, nothing is kept.
    pub(crate) fn stop_first(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        tick: Tick,
        f: u64,
    ) {
        let (bar_t, bar_f) = self.last_bar;
        // The bar line just passed, or the next one (as the transport would
        // have reached it at the tempo then).
        let (end_t, end_f) = if tick - bar_t <= SIXTEENTH {
            (bar_t, bar_f)
        } else {
            (bar_t + self.bar, self.next_bar)
        };
        let len = end_f.saturating_sub(self.start_frame) as usize;
        if end_t <= self.origin || len == 0 {
            stats.aborted += 1;
            self.reset();
            self.reclaim(store, me);
            return;
        }
        // Stopped: no overdub waits for PLAY.
        self.then_overdub = false;
        self.end_tick = Some(end_t);
        // Nothing after STOP is kept.
        if self.finish_len(store, stats, me, end_t, len, false) {
            self.cut = len.min(f.saturating_sub(self.rec_from) as usize);
            // What it has written of that: the rest comes in over the
            // writer's `latency` frames ([`Loop::write_first`]).
            self.slots[0].valid = (0, KEEP_BEFORE + self.cut.min(self.written));
        }
    }

    /// The first pass ends at tick `t`, `len` frames long, keeping what
    /// follows it if `after` ([`KEEP_AFTER`]: its memory is taken now,
    /// or counted short). False if there is no memory for the pass (it is
    /// thrown away).
    pub(crate) fn finish_len(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        t: Tick,
        len: usize,
        after: bool,
    ) -> bool {
        self.mixing = true;
        // The loop is a region of what the pass keeps: what was heard
        // before it, the loop, and what the writer hears after it while
        // the loop plays. Past those, MAX_SEAM frames are never written,
        // so a seam that reaches past what was kept reads silence instead
        // of going round to the other end.
        let kept = KEEP_BEFORE + len;
        let size = kept + KEEP_AFTER + MAX_SEAM;
        // A pickup plays at the loop's end, leading into its first bar.
        if self.lead >= len {
            self.lead = 0;
        }
        let view = View::region(size, KEEP_BEFORE, len, KEEP_BEFORE + self.lead);
        let tail = after && view.is_some() && store.grow(me, 0, blocks_for(kept + KEEP_AFTER));
        let Some(view) = view.filter(|_| tail || store.grow(me, 0, blocks_for(kept))) else {
            stats.aborted += 1;
            self.reset();
            self.reclaim(store, me);
            return false;
        };
        if after && !tail {
            stats.kept_short += 1;
        }
        self.short = after && !tail;
        // A pass cut back to a bar line it had recorded past gives back
        // the blocks after it.
        store.shrink(
            me,
            0,
            blocks_for(if tail { kept + KEEP_AFTER } else { kept }),
        );
        self.len = len;
        self.cut = len;
        self.len_ticks = t - self.origin;
        self.order[0] = 0;
        self.in_order = 1;
        // What the writer has written: it runs `latency` frames behind,
        // so a pass closed on its end has that much of it still to come,
        // which is the loop's only once written ([`Loop::write_first`]):
        // till then a reused block's old audio would play there. A pass
        // that closed after its end (smart loop end's Auto listens on past
        // it) has written what it heard after the end already: that is
        // the start of what it keeps after it.
        let heard = (KEEP_BEFORE + self.written).min(if tail { kept + KEEP_AFTER } else { kept });
        self.slots[0] = Slot {
            valid: (0, heard),
            born: store.born(),
            ..Slot::NEW
        };
        self.views[0] = view;
        self.region = Some((KEEP_BEFORE, KEEP_BEFORE + self.lead));
        self.captured = (KEEP_BEFORE, kept);
        self.tail = tail;
        self.n_done = 0;
        self.n_undone = 0;
        self.ending = false;
        self.pos = 0.0;
        self.jump = 0;
        self.phase = Phase::Playing;
        self.touch();
        if self.then_overdub {
            self.then_overdub = false;
            self.open_overdub(store, stats, me, HANDOFF_FADE);
        }
        true
    }

    /// The first pass's writer: frame `f`'s input belongs `latency` frames
    /// earlier, so the loop's frame `j` is written at `start + latency + j`.
    pub(crate) fn write_first(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        f: u64,
        x: [f32; 2],
    ) {
        let before = match self.phase {
            Phase::Armed => true,
            _ if self.first_open => f < self.rec_from + u64::from(self.rec_latency),
            _ if self.tail => false,
            _ => return,
        };
        if before {
            // Waiting for you to play, by ear: the first note over
            // PLAY_LEVEL, as played (the latency before it came in).
            if self.by_ear && self.listening() && x[0].abs().max(x[1].abs()) >= PLAY_LEVEL {
                self.hit = Some(f.saturating_sub(u64::from(self.latency)));
            }
            self.ring[2 * self.ring_at] = x[0];
            self.ring[2 * self.ring_at + 1] = x[1];
            self.ring_at = (self.ring_at + 1) % FADE;
            return;
        }
        let j = (f - self.rec_from - u64::from(self.rec_latency)) as usize;
        if j == 0 && self.resample && !self.keep_pre(store, stats, me) {
            return;
        }
        // The pass's frame `j` is the audio's `KEEP_BEFORE + j`.
        let q = KEEP_BEFORE + j;
        if self.len > 0 && j >= self.cut {
            // The pass is written once a start heard before it is in.
            if self.first_open && self.fill.is_none() && self.before.is_none() {
                self.close_first(store, me);
            }
            if self.tail && q >= self.captured.1 {
                self.keep_after(store, me, q, x);
            }
            return;
        }
        if !store.grow(me, 0, q / BLOCK + 1) {
            stats.aborted += 1;
            self.reset();
            self.reclaim(store, me);
            return;
        }
        store.set(me, 0, q, x);
        self.written = self.written.max(j + 1);
        // Closed (or cut by STOP) with its last frames still coming in:
        // each is the loop's once it is written.
        if self.len > 0 && self.slots[0].valid == (0, q) {
            self.slots[0].valid.1 = q + 1;
        }
    }

    /// What the writer hears after the loop's end, as frame `q` of the
    /// audio, up to [`KEEP_AFTER`] frames (its memory was taken when the
    /// pass ended): what the layer holds grows by a frame. It stops at a
    /// gap (the slot went to another layer).
    fn keep_after(&mut self, store: &mut Store, me: usize, q: usize, x: [f32; 2]) {
        let end = self.captured.1 + KEEP_AFTER;
        if q >= end || self.slots[0].valid != (0, q) {
            self.tail = false;
            return;
        }
        store.set(me, 0, q, x);
        self.slots[0].valid.1 = q + 1;
        self.tail = q + 1 < end;
    }

    /// A resample's first frame: it keeps what the writer took just
    /// before it (what the other loops played, not what was heard) as the
    /// last [`FADE`] frames before the loop, for the loop point, and
    /// silence before those. False if there is no memory (the pass is
    /// thrown away). Out of line: it runs once a pass, and the writer
    /// around it runs every frame.
    #[inline(never)]
    fn keep_pre(&mut self, store: &mut Store, stats: &mut Stats, me: usize) -> bool {
        if !store.grow(me, 0, blocks_for(KEEP_BEFORE)) {
            stats.aborted += 1;
            self.reset();
            self.reclaim(store, me);
            return false;
        }
        for q in 0..KEEP_BEFORE - FADE {
            store.set(me, 0, q, [0.0; 2]);
        }
        for k in 0..FADE {
            let r = (self.ring_at + k) % FADE;
            let x = [self.ring[2 * r], self.ring[2 * r + 1]];
            store.set(me, 0, KEEP_BEFORE - FADE + k, x);
        }
        true
    }

    /// The first pass is written. Its loop point is a seam the loop plays
    /// ([`piece_frame`]: the end fades into what was heard just before
    /// the start), so nothing is written for it; a take STOP cut short
    /// fades out where STOP came.
    fn close_first(&mut self, store: &mut Store, me: usize) {
        self.first_open = false;
        // A resampled first pass is written: overdubs take the input.
        self.resample = false;
        let cut = self.cut.min(self.len);
        if cut < self.len {
            let out = FADE.min(cut / 2);
            for k in 0..out {
                let w = 1.0 - (k as f32 + 0.5) / out as f32;
                let q = KEEP_BEFORE + cut - out + k;
                let a = store.get(me, 0, q);
                store.set(me, 0, q, [a[0] * w, a[1] * w]);
            }
        }
    }
}
