//! Commands, applied at the start of a period.

use super::*;

impl Looper {
    pub(crate) fn apply_commands(&mut self) {
        // Commands that would cut a fade short wait for it, in order. Once
        // a selection waits, everything after it waits too, so no command
        // reaches the wrong loop.
        let mut kept = 0;
        let mut waiting = false;
        let mut hold_all = false;
        for k in 0..self.queued {
            let q = self.queue[k];
            // Overdub (or Record into one) over a print: fit the loop to
            // the print's tempo first; the overdub waits for it.
            // It starts on the frame the fit takes over (a second press
            // while it copies calls the overdub off).
            if !waiting && matches!(q, Queued::Loop(Command::Overdub | Command::Record)) {
                let s = self.selected;
                // On a stopped free loop: the free loops play from their
                // tops, and the overdub starts with them.
                let l = &self.loops[s];
                if l.clock == Clock::Free
                    && l.phase == Phase::Playing
                    && !l.od_open
                    && !self.free_time.rolls_at(self.now)
                {
                    self.free_play();
                }
                let l = &self.loops[s];
                if l.fitting && l.fit_overdub {
                    self.loops[s].fit_overdub = false;
                    continue;
                }
                // Waiting for the print at another speed: a second press
                // calls it off.
                if l.print_wait {
                    self.loops[s].print_wait = false;
                    continue;
                }
                let p = self.prints[s].now.as_ref();
                let live = p.filter(|p| l.print_live(p, self.prints[s].next.is_some()));
                let lent = l.adding || l.lend.is_some();
                if let Some(frames) = live.filter(|_| l.print_on > 0.0 && !lent).map(|p| p.frames) {
                    let l = &mut self.loops[s];
                    l.start_fit(&mut self.store, &mut self.stats, s, frames, self.now);
                    if l.fitting {
                        l.fit_overdub = true;
                        continue;
                    }
                }
                // At another speed the audio plays faster or slower than
                // it would record: the overdub waits for the print made at
                // that speed, and fits the loop to it first.
                let l = &self.loops[s];
                if l.rate != 0 && l.phase == Phase::Playing && !l.od_open {
                    if lent {
                        self.stats.refused += 1;
                    } else {
                        self.loops[s].print_wait = true;
                    }
                    continue;
                }
            }
            let deferrable = matches!(
                q,
                Queued::Loop(
                    Command::Record
                        | Command::Overdub
                        | Command::Undo
                        | Command::Redo
                        | Command::Edit(_)
                        | Command::Merge
                        | Command::Resample
                        | Command::Capture(_)
                        | Command::CaptureTo(..)
                ) | Queued::Layer(_, LayerCommand::Delete)
                    | Queued::Copy { .. }
            ) || (self.loops[self.selected].busy()
                && matches!(q, Queued::Layer(..)));
            let held = match q {
                // A copy waits for both of its loops.
                Queued::Copy { from, to, .. } => [from, to]
                    .iter()
                    .any(|&k| self.loops.get(k).is_some_and(|l| l.must_wait(q))),
                _ => self.loops[self.selected].must_wait(q),
            };
            let wait = hold_all
                || (deferrable && (waiting || held))
                || (waiting && matches!(q, Queued::Select(_)));
            if wait {
                self.queue[kept] = q;
                kept += 1;
                waiting = true;
                hold_all |= matches!(q, Queued::Select(_));
            } else {
                self.apply(q);
            }
        }
        self.queued = kept;
    }

    fn apply(&mut self, q: Queued) {
        let s = self.selected;
        match q {
            Queued::Select(n) if n < MAX_LOOPS => self.selected = n,
            Queued::Select(_) => self.stats.no_layer += 1,
            // A loop on a track launches only where the track says.
            Queued::Launch { n, .. } if self.on_track(n) => self.stats.refused += 1,
            Queued::Launch { n, on, at } => match self.loops.get_mut(n) {
                Some(l) => match at {
                    Some(t) => l.cue = Some((on, t)),
                    None => {
                        l.on = on;
                        l.cue = None;
                    }
                },
                None => self.stats.no_layer += 1,
            },
            // Its track mutes and solos it, on the song's clock.
            Queued::Loop(Command::Mute(_) | Command::Solo(_) | Command::SetClock(Clock::Free))
                if self.on_track(s) =>
            {
                self.stats.refused += 1
            }
            // A free loop whose cell is on a track: its track mutes and
            // solos it, and it keeps its clock.
            Queued::Loop(Command::Mute(_) | Command::Solo(_) | Command::SetClock(_))
                if self.free_tracked(s) =>
            {
                self.stats.refused += 1
            }
            Queued::Loop(Command::SetLatency(l)) => {
                for lp in &mut self.loops {
                    lp.latency = l;
                }
            }
            Queued::Loop(Command::NewInput) => self.heard.forget(),
            Queued::Loop(Command::ForgetRedo) => {
                for k in 0..MAX_LOOPS {
                    if !self.loops[k].settled() {
                        continue;
                    }
                    self.loops[k].n_undone = 0;
                    self.store.kept[k].n_undone = 0;
                    self.loops[k].reclaim(&mut self.store, k);
                }
            }
            Queued::Loop(Command::RetireThrough(t)) => {
                for k in 0..MAX_LOOPS {
                    let l = &mut self.loops[k];
                    if !l.settled() {
                        continue;
                    }
                    while l.n_done > 0 && l.done[0].txn <= t {
                        l.let_go = l.let_go.max(l.take_done(0).txn);
                    }
                    if self.store.has_kept(k) {
                        if self.store.kept_txn[k] <= t {
                            self.store.drop_kept(k);
                        } else {
                            let kept = &mut self.store.kept[k];
                            while kept.n_done > 0 && kept.done[0].txn <= t {
                                kept.let_go = kept.let_go.max(kept.take_done(0).txn);
                            }
                        }
                    }
                    self.loops[k].reclaim(&mut self.store, k);
                }
            }
            Queued::Loop(Command::Played(f)) => {
                for l in &mut self.loops {
                    if l.listening() && !l.by_ear {
                        l.hit = Some(f);
                    }
                }
            }
            Queued::Loop(Command::StartsByEar(on)) => {
                for l in &mut self.loops {
                    l.by_ear = on;
                }
            }
            Queued::Loop(Command::TempoFromLoop(on)) => self.free_tempo = on,
            Queued::Loop(Command::FreeStop) => {
                let now = self.now;
                self.free_time
                    .stop(&mut self.loops, &mut self.store, &mut self.stats, now);
            }
            Queued::Loop(Command::FreePlay) => self.free_play(),
            // A cleared free loop comes back with the free clock it was
            // made on: the one there is, or the one that went with it. Not
            // once another has been set.
            Queued::Loop(Command::Undo)
                if self.loops[s].phase == Phase::Empty
                    && !self.loops[s].capturing()
                    && !self.loops[s].loading
                    && !self.loops[s].adding
                    && self.store.has_kept(s)
                    && self.store.kept[s].clock == Clock::Free =>
            {
                let gen = self.store.kept[s].clock_gen;
                let ft = &mut self.free_time;
                let back = match (ft.clock, ft.kept) {
                    (Some(_), _) => ft.gen == gen,
                    (None, Some((c, g))) if g == gen => {
                        ft.clock = Some(c);
                        ft.kept = None;
                        true
                    }
                    (None, _) => false,
                };
                if back {
                    self.loops[s].apply(&mut self.store, &mut self.stats, s, Command::Undo);
                } else {
                    self.stats.refused += 1;
                }
            }
            Queued::Loop(Command::TempoMeant(on)) => self.tempo_meant = on,
            Queued::Loop(Command::SetSmartEnd(m)) => self.smart = m,
            Queued::Loop(Command::SmartByEar(on)) => {
                if on != self.by_ear {
                    self.by_ear = on;
                    self.hits.clear();
                    self.find = None;
                }
            }
            // A capture under way: again, Undo or Clear call it off.
            Queued::Loop(
                Command::Capture(_)
                | Command::CaptureTo(..)
                | Command::Undo
                | Command::Clear
                | Command::Forget,
            ) if self.loops[s].capturing() => {
                self.loops[s].call_off_capture();
            }
            Queued::Loop(Command::Capture(bars)) => self.start_capture(s, bars, None),
            Queued::Loop(Command::CaptureTo(end, bars)) => self.start_capture(s, bars, Some(end)),
            Queued::Copy { from, id, to, keep } => self.start_copy(from, id, to, keep),
            Queued::Loop(Command::Clear | Command::Forget) if self.loops[s].loading => {
                // Called off: the memory goes back (below).
                self.loops[s].loading = false;
            }
            // A copy in or out of the loop is called off, then it clears.
            Queued::Loop(Command::Clear | Command::Forget)
                if self.loops[s].adding || self.loops[s].lend.is_some() =>
            {
                self.call_off_copy(s);
                if let Some(t) = self.loops[s].lend.map(|l| l.to as usize) {
                    self.call_off_copy(t);
                }
                self.apply(q);
                return;
            }
            // Undo calls a copy in off.
            Queued::Loop(Command::Undo) if self.loops[s].adding => self.call_off_copy(s),
            // A step whose layer is away waits until it is brought back.
            Queued::Loop(c @ (Command::Undo | Command::Redo))
                if self.needs_back(s, c).is_some() =>
            {
                self.stats.refused += 1
            }
            Queued::Loop(c @ (Command::Undo | Command::Redo)) if self.tied(s, c).is_some() => {
                let o = self.tied(s, c).expect("tied above");
                self.loops[s].apply(&mut self.store, &mut self.stats, s, c);
                self.loops[o].apply(&mut self.store, &mut self.stats, o, c);
                // A layer that made a loop of its own took that loop with
                // it: the other loop's step can't come back either.
                if c == Command::Undo && self.loops[s].clearing {
                    self.loops[o].n_undone -= 1;
                } else if c == Command::Undo && self.loops[o].clearing {
                    self.loops[s].n_undone -= 1;
                }
                self.loops[o].reclaim(&mut self.store, o);
            }
            Queued::Loop(Command::Merge) => {
                self.loops[s].merge(&mut self.store, &mut self.stats, s, self.now)
            }
            Queued::Loop(Command::Fit) => {
                let l = &self.loops[s];
                let p = self.prints[s].now.as_ref();
                match p.filter(|p| l.print_live(p, self.prints[s].next.is_some())) {
                    Some(p) if l.print_on > 0.0 => {
                        let frames = p.frames;
                        self.loops[s].start_fit(
                            &mut self.store,
                            &mut self.stats,
                            s,
                            frames,
                            self.now,
                        )
                    }
                    _ => self.stats.refused += 1,
                }
            }
            Queued::Loop(c) => {
                let was = self.loops[s].recording();
                let empty = self.loops[s].phase == Phase::Empty;
                self.loops[s].apply(&mut self.store, &mut self.stats, s, c);
                if !was && self.loops[s].recording() {
                    self.others_stop_recording(s);
                }
                // Tempo from the loop, on the bar: armed with the song
                // stopped, it records free from now (there is no bar line
                // to wait for).
                let l = &self.loops[s];
                if c == Command::Record
                    && empty
                    && l.phase == Phase::Armed
                    && l.starts == Starts::Bar
                    && self.stopped
                    && self.frees(s)
                {
                    self.begin_free(s, self.now);
                }
            }
            Queued::Layer(id, c) => {
                let was = self.loops[s].recording();
                self.loops[s].apply_layer(&mut self.store, &mut self.stats, s, id, c);
                if !was && self.loops[s].recording() {
                    self.others_stop_recording(s);
                }
            }
        }
        self.loops[s].reclaim(&mut self.store, s);
    }
}
