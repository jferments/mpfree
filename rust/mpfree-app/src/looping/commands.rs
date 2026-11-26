//! Presses and settings, with their log lines, and the undo history's steps.

use super::*;

impl LoopHost {
    /// A press or a setting, with its `looper:` line for the log.
    pub fn command(&mut self, c: Command) -> String {
        // UNDO or REDO of a snap or of Read as lands on the loop's
        // downbeat with the tempo, as the step did.
        let h = self.looper.history();
        let tempo = |s: Option<&Step>| matches!(s, Some(Step::Snap(_) | Step::Bars(_)));
        match c {
            // Make song, while nothing has changed since: its own UNDO
            // and REDO, with the song's tempo.
            Command::Undo => {
                if let SongStep::Made(p, b, v) = self.song {
                    if v == self.versions() {
                        self.song = SongStep::Unmake(p, b);
                        return "looper: UNDO: Make song, the loops free again".to_string();
                    }
                }
            }
            Command::Redo => {
                if let SongStep::Unmade(v) = self.song {
                    if v == self.versions() {
                        self.song = SongStep::Want;
                        return "looper: REDO: Make song".to_string();
                    }
                }
            }
            _ => {}
        }
        match c {
            Command::Undo if tempo(h.done().last()) => return self.snap(Snap::Undo),
            Command::Redo if tempo(h.redo().first()) => return self.snap(Snap::Redo),
            // One whose layer is away (phase 4d.4) waits for it, as the
            // undo history's presses do.
            Command::Undo | Command::Redo
                if self
                    .looper
                    .away_for(self.looper.selected(), c == Command::Redo)
                    .is_some() =>
            {
                return self.steps(if c == Command::Redo { 1 } else { -1 });
            }
            _ => {}
        }
        // The app's latency is the line input's; the pads have their own.
        // Records from: Loops makes a new loop a resample.
        let empty = self
            .looper
            .loop_status(self.picked)
            .is_some_and(|s| s.phase == Phase::Empty);
        let capturing = self
            .looper
            .loop_status(self.picked)
            .is_some_and(|s| s.capturing.is_some());
        // Length: Pattern. The first pass takes the pattern's length, kept
        // up to date by set_pattern_bars; any other Length lets go of it.
        if let Command::SetBars(b) = c {
            self.match_pattern = b == Some(PATTERN_BARS);
            // A new Length: What you played's window takes it.
            self.capture_span = None;
        }
        // Starts and Sync stick: every empty loop records as they were
        // last set.
        if let Command::SetStarts(w) = c {
            self.starts = w;
        }
        if let Command::SetClock(k) = c {
            if k != self.clock {
                // What you played counts the other clock's bars now.
                self.capture_span = None;
            }
            self.clock = k;
            // Set by hand, it is the picked loop's too (a Loop cell armed
            // for it lets go on Free: `desk::cell_lets_go`).
            self.for_cell = false;
            // A loop with sound, or sound coming in, keeps its own: the
            // looper would refuse it.
            let takes = self.looper.loop_status(self.picked).is_some_and(|s| {
                s.phase == Phase::Empty
                    && s.loading.is_none()
                    && s.copying.is_none()
                    && s.capturing.is_none()
            });
            if !takes {
                let sync = match k {
                    Clock::Song => "the song",
                    Clock::Free => "free",
                };
                return format!(
                    "looper: sync: {sync} from the next empty loop on (loop {} keeps its own)",
                    self.picked + 1
                );
            }
        }
        if c == Command::Record && empty {
            let clock = if self.for_cell {
                Clock::Song
            } else {
                self.clock
            };
            self.sticky_settings(self.picked, clock);
        }
        if self.match_pattern
            && c == Command::Record
            && empty
            && self.looper.status().bars != self.pattern_bars
        {
            self.looper.command(Command::SetBars(self.pattern_bars));
        }
        let c = match c {
            Command::SetBars(Some(PATTERN_BARS)) => Command::SetBars(self.pattern_bars),
            Command::SetLatency(_) if self.source.inside() => {
                Command::SetLatency(self.pads_latency)
            }
            // Record (a ring, REC) on a loop capturing calls it off, as
            // Capture again does.
            Command::Record if capturing => Command::Capture(1),
            Command::Record if self.resample && empty => Command::Resample,
            c => c,
        };
        if let Command::SetLatency(l) = c {
            self.latency_set = l;
        }
        self.steps_left = 0;
        let s = self.looper.status();
        let moved = self.moved_text(c, &s);
        let what = match (c, s.phase) {
            (Command::Undo | Command::Redo, Phase::Playing) if moved.is_some() => {
                moved.unwrap_or_default()
            }
            (Command::Record, Phase::Empty) if s.clock == Clock::Free && !self.resample => {
                self.free_record_words(&s)
            }
            (Command::Record, Phase::Recording) if s.clock == Clock::Free && s.free => format!(
                "Record during the free loop's first pass: it closes (once it is {} at \
                 {LOOP_MAX_BPM:.0} BPM), and its length sets the free loops' tempo",
                shortest_words(s.bars)
            ),
            (Command::Record, Phase::Empty) if s.starts == Starts::Play && !self.resample => {
                format!(
                    "Record: armed, the first pass starts when you play {} ({})",
                    self.first_note(),
                    if s.free {
                        format!(
                            "{}: REC again closes it, and its length sets the tempo",
                            free_words(s.bars)
                        )
                    } else {
                        bars_text(s.bars)
                    }
                )
            }
            (Command::Record, Phase::Empty) if s.free && !self.resample => format!(
                "Record: the first pass records {} from now: REC again closes it, \
                 and its length sets the tempo and starts the song",
                free_words(s.bars)
            ),
            (Command::Record, Phase::Empty) => format!(
                "Record: armed, the first pass starts on the next bar line ({})",
                bars_text(s.bars)
            ),
            (Command::Record, Phase::Armed) => "Record again: disarmed".to_string(),
            (Command::Record, Phase::Recording) if s.free => format!(
                "Record during the free first pass: it closes (once it is {} at \
                 {LOOP_MAX_BPM:.0} BPM), and its length sets the tempo",
                shortest_words(s.bars)
            ),
            (Command::Record, Phase::Recording) => {
                "Record during the first pass: it ends on the next bar line".to_string()
            }
            // Pressed again while it waits for its print or its fit.
            (Command::Record | Command::Overdub, Phase::Playing) if s.overdub_next => {
                "Overdub again: the overdub waiting for the fit is off".to_string()
            }
            (Command::Record | Command::Overdub, Phase::Playing) if s.rate != 0 && !s.printed => {
                format!(
                    "overdub on, once the loop's print at {} is in and the loop is fitted to it \
                     (its print becomes its layer; Undo takes the fit back)",
                    crate::looper_page::speed_text(s.speed)
                )
            }
            (Command::Record | Command::Overdub, Phase::Playing) if s.printed => {
                "overdub on, once the loop is fitted to the tempo (its print becomes its layer; \
                 Undo takes the fit back)"
                    .to_string()
            }
            (Command::Record | Command::Overdub, Phase::Playing) => {
                format!("overdub on (layer {})", s.layers + 1)
            }
            (Command::Record | Command::Overdub, Phase::Overdubbing) => "overdub off".to_string(),
            (Command::Overdub, Phase::Recording) if !s.overdub_next => {
                "Overdub during the first pass: the overdub \
                                                                         starts where it ends"
                    .to_string()
            }
            (Command::Overdub, Phase::Recording) => {
                "Overdub again: no overdub after the first pass".to_string()
            }
            (Command::Overdub, _) => format!("Overdub: nothing to overdub ({:?})", s.phase),
            (Command::Undo, Phase::Armed | Phase::Recording) => {
                "Undo: first pass thrown away".to_string()
            }
            (Command::Undo, Phase::Overdubbing) => {
                "Undo: the overdub in progress fades out".to_string()
            }
            (Command::Undo, Phase::Playing) if s.layers > 1 => {
                format!("Undo: layer {} fades out", s.layers)
            }
            (Command::Undo | Command::Capture(_), Phase::Empty) if s.capturing.is_some() => {
                format!("Capture into Loop {} called off", self.picked + 1)
            }
            (Command::Undo, Phase::Empty) if s.kept => {
                "Undo: the cleared loop comes back".to_string()
            }
            (Command::Undo, _) => {
                "Undo: nothing to undo (Clear removes the first pass)".to_string()
            }
            (Command::Redo, Phase::Playing) if s.redo > 0 => {
                format!("Redo: layer {} fades back in", s.layers + 1)
            }
            (Command::Redo, _) => "Redo: nothing to redo".to_string(),
            (Command::Clear, Phase::Empty) => "Clear: nothing to clear".to_string(),
            (Command::Clear, _) => format!(
                "Clear: the loop fades out ({} layers); UNDO brings it back",
                s.layers
            ),
            (Command::Forget, Phase::Empty) => "Forget: nothing kept".to_string(),
            (Command::Forget, _) => {
                format!("Forget: the loop fades out ({} layers), no undo", s.layers)
            }
            (Command::Solo(o), _) => {
                if o {
                    "soloed: the loops not soloed go quiet".to_string()
                } else {
                    "solo off".to_string()
                }
            }
            (Command::SetOverdubs(o), _) => format!("overdubs: {}", overdubs_text(o)),
            (Command::Mute(m), _) => {
                if m {
                    "muted".to_string()
                } else {
                    "unmuted".to_string()
                }
            }
            (Command::SetBars(b), _) => format!("first pass length {}", bars_text(b)),
            (Command::SetAfter(a), _) => format!("after recording: {a:?}"),
            (Command::SetStarts(Starts::Bar), _) => "starts: on the bar".to_string(),
            (Command::SetStarts(Starts::Play), _) => {
                format!("starts: when you play {}", self.first_note())
            }
            (Command::Played(f), _) => format!("a note played on frame {f}"),
            (Command::StartsByEar(on), _) => format!(
                "a loop that starts when you play listens to {}",
                if on { "the line input" } else { "the pads" }
            ),
            (Command::TempoFromLoop(on), _) => format!(
                "tempo from the first loop: {}",
                if on {
                    "on (a free first pass with the song stopped sets the tempo)"
                } else {
                    "off"
                }
            ),
            (Command::SetClock(Clock::Song), _) => "sync: the song".to_string(),
            (Command::SetClock(Clock::Free), _) => {
                "sync: free (the loop keeps its own time)".to_string()
            }
            (Command::FreeStop, _) => "free loops: stop, at their tops".to_string(),
            (Command::FreePlay, _) => "free loops: play from their tops".to_string(),
            (Command::UseReading, _) => match s.asks {
                Some(r) => format!(
                    "use the reading: it plays as {} from its top",
                    reading_words(&r)
                ),
                None => "use the reading: nothing waits on its Length".to_string(),
            },
            (Command::SetSmartEnd(m), _) => format!("smart loop end: {}", smart_end_text(m)),
            (Command::SmartByEar(on), _) => format!(
                "smart loop end listens to {}",
                if on {
                    "the hits it hears in the take"
                } else {
                    "the pads and MIDI notes"
                }
            ),
            (Command::TempoMeant(on), _) => format!(
                "the song's tempo counts as {} when a free first loop's length is read",
                if on {
                    "set by hand"
                } else {
                    "left from before"
                }
            ),
            (Command::SetFadeOld(k), _) => format!("fade old: {:.0} % kept a pass", 100.0 * k),
            (Command::SetLatency(l), _) => {
                format!("latency {l} frames ({:.1} ms)", l as f64 / 44.1)
            }
            (Command::SetLevel(v), _) => format!("loop level {:.1} dB", 20.0 * v.max(1e-6).log10()),
            (Command::Edit(Edit::Speed(v)), Phase::Playing) if v == s.speed => {
                format!("speed {}: already", crate::looper_page::speed_text(v))
            }
            (Command::Edit(Edit::Speed(v)), Phase::Playing) => format!(
                "speed {}: {} (Undo takes it back)",
                crate::looper_page::speed_text(v),
                match v - s.speed {
                    -2 => "two octaves down, four times the bars",
                    -1 => "an octave down, twice the bars",
                    1 => "an octave up, half the bars",
                    _ => "two octaves up, a quarter of the bars",
                }
            ),
            (Command::Edit(e), Phase::Playing) => format!("edit {e:?}"),
            (Command::Edit(e), _) => format!("edit {e:?}: waits, or is refused while recording"),
            (Command::Merge, Phase::Playing) => {
                format!("merge {} layers into one, in the background", s.layers)
            }
            (Command::Merge, _) => "merge: refused while recording".to_string(),
            (Command::Resample, Phase::Empty) => {
                "Resample: armed, the first pass records the other loops".to_string()
            }
            (Command::Resample, Phase::Armed | Phase::Recording) if s.resampling => {
                "Resample again: as Record".to_string()
            }
            (Command::Resample, _) => "Resample: only on an empty loop".to_string(),
            (Command::Fit, Phase::Playing) if s.printed => format!(
                "fit to the tempo: the print becomes the loop's one layer ({} layers before; Undo takes it back)",
                s.layers
            ),
            (Command::Fit, _) => "fit: refused, no print plays".to_string(),
            (Command::Capture(b), _) => format!("Capture: the last {} heard", bars_words_n(b)),
            (Command::CaptureTo(end, b), _) => format!(
                "Capture: {} up to the bar line on tick {end}",
                bars_words_n(b)
            ),
            (Command::NewInput, _) => "Capture forgets what it heard before".to_string(),
            (Command::ForgetRedo, _) => "every loop forgets what Redo could bring back".into(),
            (Command::RetireThrough(t), _) => {
                format!("every loop lets go of its steps up to transaction {}", t.0)
            }
            (Command::AbSeam(true), _) => "A/B on the seam".to_string(),
            (Command::AbSeam(false), _) => "A/B off".to_string(),
            (Command::AbMove(_), _) => "A/B against your move".to_string(),
        };
        if !self.looper.command(c) {
            return format!("looper: {what}: NOT done, the command queue is full");
        }
        format!("looper: {what}")
    }

    /// What Record does on an empty free loop `s` ([`Clock::Free`]), for
    /// the log: with nothing playing and no free clock yet, a pass in
    /// your own time whose length sets the free clock; else the free
    /// clock's bar lines, or the song's while the song plays and the free
    /// clock doesn't (the free clock is set, or starts, there). The song
    /// never starts or stops for it.
    fn free_record_words(&self, s: &Status) -> String {
        let starts = |now: &str| match s.starts {
            Starts::Play => format!(
                "Record: armed, the first pass starts when you play {}",
                self.first_note()
            ),
            Starts::Bar => format!("Record: {now}"),
        };
        match self.looper.free_clock() {
            None if s.free => format!(
                "{}, free in your own time ({}): REC again closes it, and its length sets \
                 the free loops' tempo (the song stays stopped)",
                starts("the first pass records from now"),
                free_words(s.bars)
            ),
            None => format!(
                "{} ({}), and the free loops take the song's tempo there",
                starts("armed, the free loop's first pass starts on the song's next bar line"),
                bars_text(s.bars)
            ),
            Some(c) if c.running => format!(
                "{} ({})",
                starts("armed, the free loop's first pass starts on the free loops' next bar line"),
                bars_text(s.bars)
            ),
            Some(_) if self.last_rolling => format!(
                "{} ({}), and the stopped free loops start with it",
                starts("armed, the free loop's first pass starts on the song's next bar line"),
                bars_text(s.bars)
            ),
            Some(_) => format!(
                "{} ({}), and the stopped free loops start with it",
                starts("the free loop's first pass records from now"),
                bars_text(s.bars)
            ),
        }
    }

    /// What Undo or Redo does when the step it takes back (or brings
    /// back) is a layer dragged between loops: with the other loop's step
    /// too while each is its loop's last ([`Looper::copy_layer`]).
    fn moved_text(&self, c: Command, s: &Status) -> Option<String> {
        let me = self.picked;
        let top = |h: History| match c {
            Command::Undo => h.done().last().copied(),
            _ => h.redo().first().copied(),
        };
        let other = |o: usize| self.looper.loop_history(o).and_then(top);
        let undo = c == Command::Undo;
        match top(self.looper.history())? {
            Step::Added(_, o) => {
                let tied = matches!(other(o), Some(Step::Moved(_, t)) if t == me);
                let o = o + 1;
                Some(match (undo, tied) {
                    (true, true) if s.layers <= 1 => {
                        format!("Undo: the layer goes back to loop {o}, and this loop clears")
                    }
                    (true, true) => format!("Undo: the layer goes back to loop {o}"),
                    (true, false) if s.layers <= 1 => {
                        format!("Undo: the layer from loop {o} goes, and this loop clears")
                    }
                    (true, false) => format!("Undo: the layer from loop {o} fades out"),
                    (false, true) => format!("Redo: the layer moves here from loop {o} again"),
                    (false, false) => format!("Redo: the layer from loop {o} fades back in"),
                })
            }
            Step::Moved(_, o) => {
                let tied = matches!(other(o), Some(Step::Added(_, f)) if f == me);
                let o = o + 1;
                Some(match (undo, tied) {
                    (true, true) => format!("Undo: the layer comes back from loop {o}"),
                    (true, false) => format!("Undo: the layer moved to loop {o} comes back here"),
                    (false, true) => format!("Redo: the layer moves to loop {o} again"),
                    (false, false) => format!("Redo: the layer moved to loop {o} fades out here"),
                })
            }
            _ => None,
        }
    }

    /// The undo history: go back `n` steps (below 0) or forward (above 0),
    /// as if Undo or Redo were pressed that many times, as far as the
    /// picked loop's history goes; with one `looper:` line for the log.
    /// What the command queue can't take now goes in over the next
    /// periods, unless another press comes first.
    pub fn steps(&mut self, n: i32) -> String {
        let h = self.looper.history();
        let (way, room) = if n < 0 {
            ("back", h.done().len())
        } else {
            ("forward", h.redo().len())
        };
        let want = (n.unsigned_abs() as usize).min(room);
        if want == 0 {
            self.steps_left = 0;
            return format!("looper: undo history: nothing to go {way} to");
        }
        self.steps_left = want as i32 * n.signum();
        self.queue_steps();
        format!(
            "looper: undo history: {way} {want} step{}",
            if want == 1 { "" } else { "s" }
        )
    }

    /// UNDO, or REDO (`redo`), of loop `n`'s newest step, from the one
    /// history across the engines (audio tracks phase 4d): as the picked
    /// loop's would be, but the pick stays where it is (the loop is
    /// selected for the command and the pick again after it). A snap's or
    /// Read as's lands on the loop's downbeat with the tempo, as the step
    /// did. With its `looper:` line for the log.
    pub fn undo_loop(&mut self, n: usize, redo: bool) -> String {
        let c = if redo { Command::Redo } else { Command::Undo };
        if n == self.picked && self.looper.selected() == n {
            return self.command(c);
        }
        let Some(h) = self.looper.loop_history(n) else {
            return format!("looper: loop {}: there is no such loop", n + 1);
        };
        let top = if redo {
            h.redo().first()
        } else {
            h.done().last()
        };
        if matches!(top, Some(Step::Snap(_) | Step::Bars(_))) {
            let s = if redo { Snap::Redo } else { Snap::Undo };
            return self.snap_on(n, s);
        }
        if self.looper.room() < 3 {
            return format!(
                "looper: loop {}: {c:?} NOT done, the command queue is full",
                n + 1
            );
        }
        self.steps_left = 0;
        self.looper.select_loop(n);
        self.looper.command(c);
        self.looper.select_loop(self.picked);
        format!("looper: loop {}: {c:?}, from the one history", n + 1)
    }

    /// The recording of loop `n`'s layer numbered `born` (0: none yet), to
    /// bring it back from its file (phase 4d.4).
    pub fn layer_rec(&self, n: usize, born: u64) -> u128 {
        self.layer_recs.rec_of(n, born)
    }

    /// Loop `n`'s step as UNDO names it ("Overdub · Layer 2"), its layers
    /// by their place in the loop now; None is its Clear, kept for Undo
    /// ([`mpfree_looper::Looper::loop_steps`]).
    pub fn step_words(&self, n: usize, step: Option<Step>) -> String {
        let Some(s) = step else {
            return "Clear".to_string();
        };
        let layers: Vec<Option<LayerInfo>> = (0..MAX_LAYERS)
            .map(|k| self.looper.loop_layer_info(n, k))
            .collect();
        crate::looper_page::step_text(s, &layers, self.rate)
    }

    /// As many of the undo history's presses as the queue takes, up to
    /// the first whose layer is away (audio tracks phase 4d.4): that one
    /// and those after it wait until it is brought back
    /// ([`LoopHost::waits_for_layer`]).
    pub(crate) fn queue_steps(&mut self) {
        if self.steps_left == 0 {
            return;
        }
        let redo = self.steps_left > 0;
        let c = if redo { Command::Redo } else { Command::Undo };
        let left = self.steps_left.unsigned_abs() as usize;
        let queued = self.looper.steps_queued();
        let n = self.looper.selected();
        let free = self
            .looper
            .steps_before_away(n, redo, queued + left)
            .saturating_sub(queued);
        for _ in 0..free.min(left) {
            if !self.looper.command(c) {
                break;
            }
            self.steps_left -= self.steps_left.signum();
        }
    }

    /// The undo history's presses still to go are called off.
    pub fn stop_steps(&mut self) {
        self.steps_left = 0;
    }

    /// The loop and way (Redo: true) the undo history's presses wait on
    /// for a layer that is away to be brought back from its file (audio
    /// tracks phase 4d.4: the one UNDO's [`crate::one_undo::OneUndo`]
    /// brings it back). None if they don't wait.
    pub fn waits_for_layer(&self) -> Option<(usize, bool)> {
        let redo = self.steps_left > 0;
        let n = self.looper.selected();
        let waits = self.steps_left != 0
            && self.looper.steps_queued() == 0
            && !self.looper.step_waits(n, redo)
            && self.looper.away_for(n, redo).is_some();
        waits.then_some((n, redo))
    }
}
