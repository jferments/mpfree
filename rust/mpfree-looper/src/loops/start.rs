//! Clearing a loop and bringing it back, smart starts and snaps, and
//! the bar lines a first pass starts on.

use super::*;

impl Loop {
    /// Back to empty, keeping the settings. The memory goes back when the
    /// command is done (`reclaim`).
    pub(crate) fn reset(&mut self) {
        self.mixing = true;
        self.touch();
        self.print_on = 0.0;
        self.fitting = false;
        self.fit_overdub = false;
        self.print_wait = false;
        self.swapping = false;
        self.phase = Phase::Empty;
        self.len = 0;
        self.len_ticks = 0;
        self.speed = 0;
        self.rate = 0;
        self.in_order = 0;
        self.n_done = 0;
        self.n_undone = 0;
        self.end_tick = None;
        self.ending = false;
        self.cut = 0;
        self.then_overdub = false;
        self.first_open = false;
        self.tail = false;
        self.short = false;
        self.ab_end();
        self.before = None;
        self.region = None;
        self.captured = (0, 0);
        self.written = 0;
        self.od_open = false;
        self.clearing = false;
        self.keep_clear = false;
        // A cleared loop lets go of its solo, so the others play.
        self.solo = false;
        self.gain = 0.0;
        self.pos = 0.0;
        self.jump = 0;
        self.xf = 0;
        self.seam = FADE;
        self.merging = false;
        self.bouncing = false;
        self.resample = false;
        self.loading = false;
        self.ld_heard = None;
        self.capture = None;
        self.adding = false;
        self.lend = None;
        self.unhear();
        self.lead = 0;
        self.fill = None;
        self.free = false;
        self.stop_close = false;
        self.asks = None;
        self.asks_yes = false;
        self.forget_smart();
    }

    /// A smart start, as the first pass closes: the loop starts `lead`
    /// frames later in what the pass kept (on your first hit), its length
    /// kept. False (nothing changes) if that can't be made.
    pub(crate) fn start_on(&mut self, lead: usize) -> bool {
        let Some((start, src)) = self.region else {
            return false;
        };
        let size = self.views[0].size();
        let Some(v) = View::region(size, start + lead, self.len, src + lead) else {
            return false;
        };
        self.views[0] = v;
        self.region = Some((start + lead, src + lead));
        true
    }

    /// Smart loop end lets go of this loop's tap, wait, offer and snap.
    pub(crate) fn forget_smart(&mut self) {
        self.tap = None;
        self.closing = false;
        self.smart_wait = None;
        self.found = None;
        self.moved = Shift::NONE;
        self.snap = None;
        self.snap_frame = u64::MAX;
    }

    /// What was found and where the loop is from the press and the tap,
    /// while the loop is as smart loop end left it.
    pub(crate) fn smart_now(&self) -> Option<(Option<Shift>, Shift)> {
        (self.smart_version == self.version && self.snap.is_none() && self.phase == Phase::Playing)
            .then_some((self.found, self.moved))
    }

    /// How far a snap moves the start and the end ([`Snap`]), if it can
    /// be made now.
    pub(crate) fn snap_by(&self, s: Snap) -> Result<Shift, &'static str> {
        let top = |acts: &[Act]| match acts.last() {
            Some(a) if a.kind == Kind::Snap => Some(a.shift),
            Some(a) if a.kind == Kind::Bars => Some(Shift::NONE),
            _ => None,
        };
        match s {
            Snap::Bars(_) => Ok(Shift::NONE),
            Snap::Found | Snap::Tap | Snap::Press | Snap::To(_) => {
                let (found, moved) = self.smart_now().ok_or("the loop has changed since")?;
                let to = match s {
                    Snap::Found => found.ok_or("smart loop end found nothing to change")?,
                    Snap::To(to) => to,
                    Snap::Tap => Shift {
                        start: moved.start,
                        end: 0,
                    },
                    _ => Shift {
                        start: 0,
                        end: moved.end,
                    },
                };
                let by = to - moved;
                if by.is_none() {
                    return Err(match s {
                        Snap::Found | Snap::To(_) => "the loop is already there",
                        Snap::Tap => "the loop already ends on your tap",
                        _ => "the loop already starts on your press",
                    });
                }
                Ok(by)
            }
            Snap::Undo => top(&self.done[..self.n_done])
                .map(|by| -by)
                .ok_or("the last step isn't a snap"),
            Snap::Redo => top(&self.undone[..self.n_undone]).ok_or("the next step isn't a snap"),
        }
    }

    /// The loop's length in song ticks once snap `s` lands: Read as's
    /// bars of `bar` ticks each, the other side of the step for Undo and
    /// Redo (the shape it keeps), or as now. Or why Read as can't be made.
    pub(crate) fn snap_ticks(&self, s: Snap, bar: Tick) -> Result<Tick, &'static str> {
        let kept = |acts: &[Act]| {
            acts.last()
                .and_then(Act::shape)
                .map(|i| self.shapes[i].len_ticks)
        };
        match s {
            Snap::Bars(0) => Err("a loop is at least 1 bar"),
            Snap::Bars(b) => match Tick::from(b) * bar {
                t if t == self.len_ticks => Err("the loop is already read as that"),
                t => Ok(t),
            },
            Snap::Undo => kept(&self.done[..self.n_done]).ok_or("the last step isn't a snap"),
            Snap::Redo => kept(&self.undone[..self.n_undone]).ok_or("the next step isn't a snap"),
            Snap::Found | Snap::Tap | Snap::Press | Snap::To(_) => Ok(self.len_ticks),
        }
    }

    /// Land the snap waiting, on its frame: the end moves or the loop is
    /// read as other bars (a step Undo takes back, [`Kind::Snap`],
    /// [`Kind::Bars`]), or the step is undone or redone. The tempo changed
    /// on the same frame, and bar 1 is on it from now on. Not if the loop
    /// changed since it was worked out (said, [`SnapDone`]).
    #[cold]
    #[inline(never)]
    pub(crate) fn land_snap(&mut self, stats: &mut Stats) {
        self.snap_frame = u64::MAX;
        let Some(p) = self.snap.take() else {
            return;
        };
        let ready = self.version == p.version && self.phase == Phase::Playing && !self.busy();
        let landed = ready
            && self.snap_by(p.snap) == Ok(p.by)
            && match p.snap {
                Snap::Found | Snap::Tap | Snap::Press | Snap::To(_) => self.make_shift(stats, p.by),
                Snap::Bars(b) => self.make_bars(stats, b, p.ticks),
                Snap::Undo => {
                    self.undo();
                    true
                }
                Snap::Redo => {
                    self.redo();
                    true
                }
            };
        if landed {
            // The loop's downbeat: its new bars count from here (after a
            // reading of other bars, or its undo, the old bar 1 may not
            // fall on it).
            self.origin = p.tick;
            self.moved = self.moved + p.by;
            self.touch();
            self.smart_version = self.version;
        } else {
            stats.refused += 1;
        }
        self.snap_done = Some(SnapDone { plan: p, landed });
    }

    /// The snap waiting can't land (a free loop's, the free loops stopped
    /// before its frame): said ([`SnapDone`]) and counted.
    #[cold]
    #[inline(never)]
    pub(crate) fn drop_snap(&mut self, stats: &mut Stats) {
        self.snap_frame = u64::MAX;
        if let Some(plan) = self.snap.take() {
            stats.refused += 1;
            self.snap_done = Some(SnapDone {
                plan,
                landed: false,
            });
        }
    }

    pub(crate) fn abort(&mut self, stats: &mut Stats) {
        if self.phase == Phase::Recording {
            stats.aborted += 1;
        }
        self.reset();
    }

    /// Undo on a cleared loop: it comes back as it was kept, fading in,
    /// at the song's place, with today's latency and launch. Whatever was
    /// under way when it was cleared (an overdub, a merge) stays stopped;
    /// a print is made again.
    pub(crate) fn unclear(&mut self, store: &mut Store, me: usize) {
        let (latency, on, cue) = (self.latency, self.on, self.cue);
        self.clone_from(&store.kept[me]);
        // The blocks are the loop's again.
        store.kept_mask[me] = 0;
        self.latency = latency;
        self.on = on;
        self.cue = cue;
        self.clearing = false;
        self.keep_clear = false;
        self.gain = 0.0;
        self.od_open = false;
        if self.phase == Phase::Overdubbing {
            self.phase = Phase::Playing;
        }
        self.jump = 0;
        self.xf = 0;
        self.merging = false;
        self.fitting = false;
        self.fit_overdub = false;
        self.print_wait = false;
        self.swapping = false;
        self.bouncing = false;
        self.loading = false;
        self.adding = false;
        self.lend = None;
        self.print_on = 0.0;
        self.p_frames = 0;
        self.mixing = true;
        self.touch();
        self.reclaim(store, me);
    }

    pub(crate) fn marker(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        mark: Mark,
        frame: u64,
    ) {
        // A pickup waiting for its bar line is no pickup once the song
        // stops or jumps: the loop listens again.
        if self.phase == Phase::Armed
            && self.pending.is_some_and(|(_, pickup)| pickup)
            && matches!(
                mark,
                Mark::Stopped { .. } | Mark::Located { .. } | Mark::Looped { .. } | Mark::Panic
            )
        {
            self.unhear();
        }
        // The lead counts from where the tap was in the song: once the
        // song stops or jumps, the loop starts on the next bar line it
        // reaches (on the bar PLAY starts on).
        if self.phase == Phase::Armed
            && matches!(
                mark,
                Mark::Stopped { .. } | Mark::Located { .. } | Mark::Looped { .. } | Mark::Panic
            )
        {
            self.tap_lead = None;
        }
        if self.phase != Phase::Recording {
            return;
        }
        // A free pass has no bars: the song starting under it (not from
        // its close) throws it away. Nothing else moves it while stopped.
        if self.free {
            if matches!(
                mark,
                Mark::Started { .. } | Mark::CountIn { .. } | Mark::Panic
            ) {
                stats.aborted += 1;
                self.reset();
                self.reclaim(store, me);
            }
            return;
        }
        match mark {
            // The transport looped back at `end`: if that is where the pass
            // ends (or Record asked for the next bar), it ends here.
            Mark::Looped { end, .. }
                if self.end_tick == Some(end) || (self.ending && self.end_tick.is_none()) =>
            {
                self.end_tick = Some(end);
                self.finish_first(store, stats, me, end, frame);
            }
            // STOP: what was played is kept, cut where STOP came.
            Mark::Stopped { tick } => self.stop_first(store, stats, me, tick, frame),
            Mark::Looped { .. } | Mark::Located { .. } | Mark::Panic => {
                stats.aborted += 1;
                self.reset();
                self.reclaim(store, me);
            }
            _ => {}
        }
    }

    pub(crate) fn bar_line(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        span: &Span,
        t: Tick,
        f: u64,
    ) {
        // A launch waiting for this line (or one it passed: the song
        // jumped over it).
        if let Some((on, at)) = self.cue {
            if t >= at && matches!(span.state, State::Playing | State::Recording) {
                self.on = on;
                self.cue = None;
            }
        }
        match self.phase {
            // A free pass has no bar lines (the song is stopped under it).
            Phase::Recording if self.free => {}
            Phase::Recording if t > self.origin && (self.end_tick == Some(t) || self.ending) => {
                self.finish_first(store, stats, me, t, f);
            }
            Phase::Recording => {
                self.last_bar = (t, f);
                self.next_bar = span.frame_of((t + self.bar) as f64);
            }
            Phase::Armed if t >= 0 && matches!(span.state, State::Playing | State::Recording) => {
                if let Some((rec_from, lead)) = self.starts_on(span, t, f) {
                    self.begin_first(store, me, span, (t, f), rec_from, lead, f);
                }
            }
            _ => {}
        }
    }

    /// An armed loop at bar line `t` (frame `f`): whether its first pass
    /// starts here, and if so the song frame its first frame was played
    /// on and the frames of a pickup before the line. On the bar: here,
    /// on the line. Waiting for you to play: once a note has come (a
    /// pickup no more than half a bar before this line; a start from a
    /// note the song started on, unless the song started a quarter of a
    /// bar or more after it). A pickup from longer ago is forgotten and
    /// the loop listens again.
    pub(crate) fn starts_on(&mut self, span: &Span, t: Tick, f: u64) -> Option<(u64, usize)> {
        if self.starts == Starts::Bar || self.resample {
            // Too soon after the tap: the next bar line.
            if self
                .tap_lead
                .is_some_and(|(tap, until)| (t as f64) > tap && (t as f64) < until)
            {
                return None;
            }
            return Some((f, 0));
        }
        let (p, pickup) = self.pending?;
        let bar = span
            .frame_of((t + span.meter.bar_ticks()) as f64)
            .saturating_sub(f);
        let early = f.saturating_sub(p);
        if pickup && early > bar / 2 {
            self.unhear();
            return None;
        }
        Some(match pickup {
            true => (f - early, early as usize),
            false if early >= bar / 4 => (f, 0),
            false => (f - early, 0),
        })
    }
}
