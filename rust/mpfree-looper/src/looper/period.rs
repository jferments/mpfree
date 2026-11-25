//! The audio period: the commands, every loop's part of the
//! period, and the outputs.

use super::*;

impl Looper {
    /// Run one period: `input` is the line input and `out` the mix, both
    /// interleaved stereo with `block.frames` frames. The loops are added
    /// to `out`; nothing else is changed. The same as
    /// [`Looper::process_each`] then [`LoopOuts::mix_into`] on `out`.
    pub fn process(&mut self, block: &Block, input: &[f32], out: &mut [f32]) {
        let mut outs = std::mem::take(&mut self.outs);
        let n = (block.frames as usize)
            .min(input.len() / 2)
            .min(out.len() / 2)
            .min(outs.capacity());
        self.period(block, input, n, &mut outs);
        outs.mix_into(out);
        self.outs = outs;
    }

    /// Run one period as [`Looper::process`] does, but keep each loop's
    /// sound apart: [`Looper::outs`] then holds what each loop played and
    /// what Listen played, and their [`LoopOuts::mix_into`] is the mix
    /// `process` would have added, bit for bit. `input` is the line input,
    /// interleaved stereo with `block.frames` frames ([`MAX_PERIOD`] at
    /// most).
    pub fn process_each(&mut self, block: &Block, input: &[f32]) {
        let mut outs = std::mem::take(&mut self.outs);
        let n = (block.frames as usize)
            .min(input.len() / 2)
            .min(outs.capacity());
        self.period(block, input, n, &mut outs);
        self.outs = outs;
    }

    /// What each loop and Listen played in the last period
    /// ([`Looper::process_each`]).
    pub fn outs(&self) -> &LoopOuts {
        &self.outs
    }

    /// One period of `n` frames, each loop into its own buffer of `outs`.
    fn period(&mut self, block: &Block, input: &[f32], n: usize, outs: &mut LoopOuts) {
        outs.begin(n);
        self.now = block.start_frame;
        self.last_span = block.spans().last().copied();
        if let Some(s) = block.spans().first() {
            self.stopped = s.state == State::Stopped;
            self.bar_ticks = s.meter.bar_ticks();
        }
        self.heard.hear(block, input, n);
        self.apply_commands();
        if self.free_time.clock.is_some() || self.loops.iter().any(|l| l.clock == Clock::Free) {
            self.tend_free();
        }
        // Smart loop end by ear: the hit track listens while a free first
        // pass may need it.
        if self.by_ear && self.wants_notes() {
            let latency = self.loops.first().map_or(0, |l| l.latency);
            self.track
                .hear(block.start_frame, &input[..2 * n], latency, &mut self.hits);
        }
        // A loop armed with the song playing starts no sooner than
        // LEAD_BEATS after the tap; armed while stopped or counting in,
        // on the bar PLAY starts on (with the pads' pattern, SHIFT + Arm).
        // A free loop armed while the free clock runs counts its beat on
        // that clock. Play for the free loops pressed while the song plays
        // waits as long.
        if let Some(s) = block.spans().first() {
            let rolling = matches!(s.state, State::Playing | State::Recording);
            let tap = s.tick_at(s.frame);
            let until = tap + (LEAD_BEATS * s.meter.beat_ticks()) as f64;
            let free = self.free_time.clock.filter(|c| c.rolls_at(s.frame));
            for l in &mut self.loops {
                if std::mem::take(&mut l.just_armed) && l.phase == Phase::Armed {
                    l.tap_lead = match free.filter(|_| l.clock == Clock::Free) {
                        Some(c) => {
                            let tap = c.tick_at(s.frame);
                            Some((tap, tap + (LEAD_BEATS * c.meter.beat_ticks()) as f64))
                        }
                        None => rolling.then_some((tap, until)),
                    };
                }
            }
            if let Some(cue) = self.free_time.cue.as_mut().filter(|c| c.0.is_nan()) {
                *cue = (tap, until);
            }
        }
        self.place_hits(block);
        self.close_free(block);
        self.tend_captures();
        self.play_listen(block, n, outs);
        // A loop with sound soloed: the others fall silent. A loop on a
        // track is its track's to solo, and plays at full gain while it
        // plays: the track fades it (a clear or a swap still fades it out).
        // One that was heard as it went on the track (its first pass just
        // ended, its own fade in under way) rises on to full gain as it
        // would have off the track, so the track takes it on unfaded with
        // no step (`LoopPlayer::moving`).
        // A free loop whose cell is on a track is its track's to solo too.
        let free_tracked = self.free_tracked;
        let tracked = |k: usize| self.on_track[k].is_some() || free_tracked & (1 << k) != 0;
        let any_solo = (self.loops.iter().enumerate())
            .any(|(k, l)| l.solo && l.phase != Phase::Empty && !tracked(k));
        let mut told = [None; MAX_LOOPS];
        for (k, l) in self.loops.iter_mut().enumerate() {
            l.solo_out = any_solo && !l.solo && free_tracked & (1 << k) == 0;
            if let Some(t) = self.on_track[k].as_mut() {
                told[k] = Some(t.take());
                l.solo_out = false;
                if matches!(l.phase, Phase::Playing | Phase::Overdubbing)
                    && !l.clearing
                    && !l.swapping
                    && l.gain == 0.0
                {
                    l.gain = 1.0;
                }
            }
        }
        let end = block.start_frame + n as u64;
        for k in 0..self.loops.len() {
            if self.loops[k].adding {
                self.render_copy(k, end);
            }
        }
        let Looper {
            store,
            loops,
            stats,
            bounce,
            prints,
            loads,
            heard,
            on_track,
            free_time,
            free_ticks,
            ..
        } = self;
        let end = block.start_frame + n as u64;
        for (k, l) in loops.iter_mut().enumerate() {
            let pr = &mut prints[k];
            // A print waiting takes over once the one playing has faded
            // out (and the last one taken out has been collected).
            if pr.next.is_some() && (l.print_on == 0.0 || pr.now.is_none()) && pr.old.is_none() {
                pr.old = pr.now.take();
                pr.now = pr.next.take().flatten();
                l.take_print(pr.now.as_ref());
            }
            if l.print_wait {
                l.tend_print_wait(store, stats, k, pr, block.start_frame);
            }
            if l.merging {
                l.render_merge(store, k, end);
            }
            if l.fitting {
                match pr.now.as_ref().filter(|p| l.print_live(p, false)) {
                    Some(p) => l.render_fit(store, k, p, end),
                    // The print went: no fit.
                    None => l.cancel_fit(stats),
                }
            }
            if l.loading {
                if let Some(f) = loads[k].as_ref() {
                    l.render_load(store, k, f, end);
                } else if l.ld_heard.is_some() {
                    l.render_heard(store, stats, k, heard, end);
                }
            }
            if l.before.is_some() {
                l.render_before(store, stats, k, heard);
            }
            if l.fill.is_some() {
                l.render_fill(store, stats, k, heard, end);
            }
        }
        if let Some(b) = bounce.as_mut().filter(|b| b.complete.is_none()) {
            let l = &mut loops[b.n];
            if !l.bouncing {
                // Cleared: what was rendered goes back as it is.
                b.complete = Some(false);
            } else {
                let len = b.buf.len() / 2;
                let speed = if b.dry { PRINT_SPEED } else { MERGE_SPEED };
                let to = len.min(b.done + speed * n);
                let level = if b.dry { 1.0 } else { l.level };
                l.render_out(store, b.n, b.done, to, level, &mut b.buf);
                b.done = to;
                if to == len {
                    l.bouncing = false;
                    b.complete = Some(true);
                }
            }
        }
        // Loops resampling run last in each run, on what the others played
        // in it.
        // (Not a loop on a track: it hears its track's input.)
        // (Nor a free loop whose cell is on a track.)
        let rs = loops.iter().enumerate().fold(0u32, |m, (k, l)| {
            m | (u32::from(l.resample && on_track[k].is_none()) << k)
        }) & !free_tracked;
        outs.set_resampling(rs);
        outs.set_on_track(
            (on_track.iter().enumerate()).fold(0u32, |m, (k, t)| m | (u32::from(t.is_some()) << k))
                | free_tracked,
        );
        // Free loops keep time with the free clock: each run gets its ticks
        // too, its bar lines cut the runs, and of the song's marks they
        // hear only what [`FreeTime::mark`] makes of them. With no free
        // loop holding, starting or loading a loop, none of that runs, and
        // the song's loops run as they always have.
        let any_free = loops
            .iter()
            .any(|l| l.clock == Clock::Free && (l.phase != Phase::Empty || l.loading || l.adding));
        // Free bar lines before this frame have been handed out.
        let mut free_done = block.start_frame;
        // A Make song waiting for the song's start, read once (only a
        // call between periods sets it): on its frame, or with the free
        // loops stopped on the period's first.
        let mut song_at = free_time
            .song
            .as_ref()
            .map_or(u64::MAX, |p| p.at.unwrap_or(0));
        let markers = block.markers();
        let mut m = 0;
        for span in block.spans() {
            let bar = span.meter.bar_ticks();
            let watch_bars = free_time.cue.is_some()
                || loops
                    .iter()
                    .any(|l| matches!(l.phase, Phase::Armed | Phase::Recording) || l.cue.is_some());
            let mut lines = span.lines(bar);
            let mut next = if watch_bars { lines.next() } else { None };
            // Playing or recording: the loops sound. Counting in: the clock
            // runs, but nothing plays back.
            let playing = matches!(span.state, State::Playing | State::Recording);
            let span_len = span.len as usize;
            let mut i = 0;
            while i < span_len {
                let off = span.offset as usize + i;
                if off >= n {
                    return;
                }
                let f = span.frame + i as u64;
                while m < markers.len() && markers[m].frame <= f {
                    let (mark, at) = (markers[m].mark, markers[m].frame);
                    let rolling = free_time.rolls_at(at);
                    for (k, l) in loops.iter_mut().enumerate() {
                        if !(any_free && l.on_free(rolling)) {
                            l.marker(store, stats, k, mark, at);
                        }
                    }
                    if any_free {
                        free_time.mark(loops, store, stats, mark, at);
                    }
                    m += 1;
                }
                while let Some((t, o)) = next {
                    if o as usize > off {
                        break;
                    }
                    if o as usize == off {
                        let rolling = free_time.rolls_at(f);
                        for (k, l) in loops.iter_mut().enumerate() {
                            if !any_free || l.clock == Clock::Song {
                                l.bar_line(store, stats, k, span, t, f);
                            } else if !l.on_free(rolling) && t >= 0 && playing {
                                // A free loop waiting for the song's bar
                                // line: it starts here, setting the free
                                // clock from the song's (or starting it).
                                if let Some((rec_from, lead)) = l.starts_on(span, t, f) {
                                    let (c, line) = free_time.set_on_song(span, (t, f));
                                    let fs = c.span(off as u32, (span_len - i) as u32, f);
                                    l.clock_gen = free_time.gen;
                                    l.begin_first(store, k, &fs, line, rec_from, lead, f);
                                    free_done = free_done.min(f);
                                }
                            }
                        }
                        // Play pressed for the stopped free loops: they
                        // start here, a beat or more after it.
                        if let Some((tap, until)) = free_time.cue {
                            let t = t as f64;
                            if playing && t >= 0.0 && !(t > tap && t < until) {
                                free_time.start(f);
                                free_done = free_done.min(f);
                            }
                        }
                    }
                    next = lines.next();
                }
                // The free clock's bar lines, for the free loops on it.
                let mut free_next = None;
                if any_free {
                    if let Some(c) = free_time.clock.filter(|c| c.running) {
                        let watch = loops.iter().any(|l| {
                            l.on_free(true)
                                && (matches!(l.phase, Phase::Armed | Phase::Recording)
                                    || l.cue.is_some())
                        });
                        if watch {
                            free_next = c.next_bar(f.max(free_done));
                            if let Some((t, _)) = free_next.filter(|&(_, at)| at == f) {
                                let fs = c.span(off as u32, (span_len - i) as u32, f);
                                for (k, l) in loops.iter_mut().enumerate() {
                                    if l.on_free(true) {
                                        l.bar_line(store, stats, k, &fs, t, f);
                                    }
                                }
                                free_done = f + 1;
                                free_next = c.next_bar(f + 1);
                            }
                        }
                    }
                }
                // A run of frames up to the next bar line or marker: the
                // loops are independent in it, so each runs it in turn.
                let mut end = span_len.min(i + RUN).min(n - span.offset as usize);
                if let Some((_, o)) = next {
                    end = end.min(o as usize - span.offset as usize);
                }
                if m < markers.len() {
                    end = end.min((markers[m].frame - span.frame) as usize);
                }
                if any_free {
                    if let Some((_, at)) = free_next {
                        end = end.min((at - span.frame) as usize);
                    }
                    // A free clock set to start on a frame to come.
                    if let Some(c) = free_time.clock.filter(|c| c.running && c.anchor_frame > f) {
                        end = end.min((c.anchor_frame - span.frame) as usize);
                    }
                    // A free loop's snap, which changes the free clock's tempo
                    // on its frame ([`FreeTime::land_snaps`]).
                    for l in loops.iter().filter(|l| l.clock == Clock::Free) {
                        if l.snap_frame > f && l.snap_frame != u64::MAX {
                            end = end.min((l.snap_frame - span.frame) as usize);
                        }
                    }
                }
                // Make song, which the song's start lands on its frame
                // ([`FreeTime::land_song`]).
                if song_at <= f {
                    free_time.land_song(loops, prints, stats, span, f);
                    song_at = u64::MAX;
                }
                end = end.min(usize::try_from(song_at - span.frame).unwrap_or(usize::MAX));
                let frames = end - i;
                let mut ticks = [0.0f64; RUN];
                let song = (span.anchor_frame, span.anchor_tick, span.bpm, span.rate);
                fill_ticks(&mut ticks[..frames], span.state.rolling(), song, f);
                if any_free {
                    free_time.land_snaps(loops, stats, f);
                }
                let free_rolling = any_free && free_time.rolls_at(f);
                if any_free {
                    let ft = &mut free_ticks[..frames];
                    match free_time.clock {
                        Some(c) => {
                            let free = (c.anchor_frame, c.anchor_tick, c.bpm, c.rate);
                            fill_ticks(ft, free_rolling, free, f);
                        }
                        None => ft.fill(0.0),
                    }
                }
                let run = Run {
                    from: i,
                    ticks: &ticks[..frames],
                    input: &input[2 * off..2 * (off + frames)],
                    f,
                    playing,
                    first: None,
                };
                // The free loops' run: the same frames on the free clock. Its
                // place can jump only where the clock starts.
                let free_run = Run {
                    from: usize::from(f != free_time.jump),
                    ticks: &free_ticks[..frames],
                    playing: free_rolling,
                    ..run
                };
                // Each loop adds its frames to its own buffer; a quiet one
                // has none to add.
                for (k, l) in loops.iter_mut().enumerate() {
                    if rs & (1 << k) == 0 && !l.quiet() {
                        let (p, waiting) = (prints[k].now.as_ref(), prints[k].next.is_some());
                        let r = l.run_from(any_free, free_rolling, &run, &free_run);
                        match (on_track[k].as_mut(), &told[k]) {
                            (Some(t), Some(told)) => {
                                l.run_told(store, stats, k, t, told, r, off, p, waiting, outs)
                            }
                            _ => {
                                l.run(store, stats, k, r, p, waiting, outs.run_mut(k, off, frames))
                            }
                        }
                    }
                }
                if rs != 0 {
                    // What the others played in the run, summed as the mix
                    // sums them.
                    let mut heard = [0.0f32; 2 * RUN];
                    outs.sum_into(!rs, off, &mut heard[..2 * frames]);
                    let run = Run {
                        input: &heard[..2 * frames],
                        ..run
                    };
                    let free_run = Run {
                        input: &heard[..2 * frames],
                        ..free_run
                    };
                    for (k, l) in loops.iter_mut().enumerate() {
                        if rs & (1 << k) != 0 && !l.quiet() {
                            let (p, waiting) = (prints[k].now.as_ref(), prints[k].next.is_some());
                            let r = l.run_from(any_free, free_rolling, &run, &free_run);
                            l.run(store, stats, k, r, p, waiting, outs.run_mut(k, off, frames));
                        }
                    }
                }
                i = end;
            }
        }
        while m < markers.len() {
            let (mark, at) = (markers[m].mark, markers[m].frame);
            let rolling = free_time.rolls_at(at);
            for (k, l) in loops.iter_mut().enumerate() {
                if !(any_free && l.on_free(rolling)) {
                    l.marker(store, stats, k, mark, at);
                }
            }
            if any_free {
                free_time.mark(loops, store, stats, mark, at);
            }
            m += 1;
        }
    }
}
