//! The loops' controls: picking a loop, level, solo and mute, launches and
//! the Loop tracks, and layers dragged between loops.

use super::*;

impl LoopHost {
    /// Pick loop `n` for the presses and controls after this, with its
    /// `looper:` line for the log ("loop 2 picked (empty)").
    pub fn select(&mut self, n: usize) -> String {
        self.for_cell = false;
        self.select_on(n, self.clock)
    }

    /// Pick empty loop `n` to record into a Loop cell armed on the grid:
    /// it takes Sync: Song whatever Sync was last set to, as a free loop
    /// has no cell ([`LoopHost::grid_now`]), and Sync stays as set for
    /// the loops picked after it.
    pub fn select_for_cell(&mut self, n: usize) -> String {
        let line = self.select_on(n, Clock::Song);
        self.for_cell = self.picked == n.min(MAX_LOOPS - 1);
        line
    }

    /// [`LoopHost::select`], an empty loop taking `clock` for its Sync.
    fn select_on(&mut self, n: usize, clock: Clock) -> String {
        self.steps_left = 0;
        let n = n.min(MAX_LOOPS - 1);
        let what = match self.looper.loop_status(n) {
            Some(s) if s.phase == Phase::Empty => "empty".to_string(),
            Some(s) => format!(
                "{} layer{}, {:?}",
                s.layers,
                if s.layers == 1 { "" } else { "s" },
                s.phase
            ),
            None => "not there".to_string(),
        };
        if !self.looper.select_loop(n) {
            return format!(
                "looper: loop {} ({what}): NOT picked, the command queue is full",
                n + 1
            );
        }
        self.picked = n;
        // An empty loop takes Starts and Sync as they were last set (it
        // shows them).
        if self
            .looper
            .loop_status(n)
            .is_some_and(|s| s.phase == Phase::Empty)
        {
            self.sticky_settings(n, clock);
        }
        format!("looper: loop {} picked ({what})", n + 1)
    }

    /// Empty loop `n`, picked, takes Starts as last set and `clock` for
    /// its Sync (as last set, but for a Loop cell's), where it differs (a
    /// loop that is copying in or capturing keeps its Sync: the looper
    /// refuses it).
    pub(crate) fn sticky_settings(&mut self, n: usize, clock: Clock) {
        let Some(s) = self.looper.loop_status(n) else {
            return;
        };
        if s.starts != self.starts {
            self.looper.command(Command::SetStarts(self.starts));
        }
        let busy = s.loading.is_some() || s.copying.is_some() || s.capturing.is_some();
        if s.clock != clock && !busy {
            self.looper.command(Command::SetClock(clock));
        }
    }

    /// Loop `n`'s mute turned over (the picked loop's with None), leaving
    /// the pick as it was: the loop cards' speakers and the Mute button.
    /// STOP with the transport stopped (the round of 16 Nov: REC on the
    /// Looper tab's playing loop started an overdub that waits for PLAY,
    /// OVERDUB and PLAY blinked, and STOP twice left them blinking): an
    /// overdub waiting for the transport is let go and an armed loop
    /// disarmed, on every loop. None when nothing waited.
    pub fn stop_waiting(&mut self) -> Option<String> {
        let picked = self.picked;
        let mut said = Vec::new();
        // Free loops on the free clock while it runs wait for nothing:
        // STOP stops them first (as it stops the song first), and their
        // overdubs and armed passes then wait for PLAY as the song's do.
        let free_runs = self.looper.free_clock().is_some_and(|c| c.running);
        for n in 0..MAX_LOOPS {
            let Some(s) = self.looper.loop_status(n) else {
                continue;
            };
            if free_runs && s.clock == Clock::Free {
                continue;
            }
            let (c, what) = match s.phase {
                Phase::Overdubbing => (Command::Overdub, "the overdub waiting for PLAY is off"),
                Phase::Armed => (Command::Record, "disarmed"),
                _ => continue,
            };
            let need = if n == picked { 1 } else { 3 };
            if self.looper.room() < need {
                said.push(format!(
                    "loop {}: NOT done, the command queue is full",
                    n + 1
                ));
                continue;
            }
            if n != picked {
                self.looper.select_loop(n);
            }
            self.looper.command(c);
            if n != picked {
                self.looper.select_loop(picked);
            }
            said.push(format!("loop {}: {what}", n + 1));
        }
        (!said.is_empty()).then(|| format!("looper: STOP: {}", said.join("; ")))
    }

    /// Tempo follows: Free loops (the looper refactor's R2, tempo
    /// context): the free loops' clock as a span the time engine copies
    /// over each period ([`crate::time::TimeHost::set_lead`]), while it
    /// runs; None while it is stopped or there is none, when the song's
    /// clock leads.
    pub fn tempo_lead(&self) -> Option<Span> {
        self.looper
            .free_clock()
            .filter(|c| c.running)
            .map(|c| lead_span(&c))
    }

    /// SHIFT + STOP and a set's Panic stop everything, the free loops too
    /// (Jay's review of 23 Nov, 5.4): as STOP with the song stopped stops
    /// them ([`Command::FreeStop`]), at once and back to their bar 1. A
    /// Play waiting for the song's bar line is called off, an overdub
    /// ends, and a free first pass in your own time closes there, as a
    /// tap would. None when no free loop plays, waits to or records in
    /// your own time (STOP's [`LoopHost::stop_waiting`] lets go of the
    /// rest).
    pub fn stop_free(&mut self) -> Option<String> {
        let runs = self.looper.free_clock().is_some_and(|c| c.running);
        let own_time = (0..MAX_LOOPS).any(|n| {
            self.looper
                .loop_status(n)
                .is_some_and(|s| s.clock == Clock::Free && s.phase == Phase::Recording && s.free)
        });
        if !(runs || own_time || self.looper.free_cued()) {
            return None;
        }
        if self.looper.room() < 1 {
            return Some(
                "looper: the free loops NOT stopped, the command queue is full".to_string(),
            );
        }
        self.looper.command(Command::FreeStop);
        Some("looper: the free loops stopped, back to their bar 1".to_string())
    }

    /// Loop `n`'s level (0 to 2), leaving the picked loop picked.
    pub fn set_level(&mut self, n: usize, level: f32) -> String {
        let picked = self.picked;
        let n = n.min(MAX_LOOPS - 1);
        let level = level.clamp(0.0, 2.0);
        // Pick it, set it, pick the old one back: all three or none.
        let need = if n == picked { 1 } else { 3 };
        if self.looper.room() < need {
            return format!(
                "looper: loop {} level: NOT set, the command queue is full",
                n + 1
            );
        }
        if n != picked {
            self.looper.select_loop(n);
        }
        self.looper.command(Command::SetLevel(level));
        if n != picked {
            self.looper.select_loop(picked);
        }
        format!(
            "looper: loop {} level {}",
            n + 1,
            crate::looper_tab::level_text(level)
        )
    }

    /// Loop `n`'s solo turned over, leaving the picked loop picked. An
    /// empty loop has nothing to solo.
    pub fn toggle_solo(&mut self, n: usize) -> String {
        let picked = self.picked;
        let n = n.min(MAX_LOOPS - 1);
        let Some(s) = self.looper.loop_status(n) else {
            return format!("looper: loop {}: not there", n + 1);
        };
        if s.phase == Phase::Empty {
            return format!("looper: loop {}: empty, nothing to solo", n + 1);
        }
        let on = !s.solo;
        // Pick it, solo, pick the old one back: all three or none.
        let need = if n == picked { 1 } else { 3 };
        if self.looper.room() < need {
            return format!(
                "looper: loop {} solo: NOT done, the command queue is full",
                n + 1
            );
        }
        if n != picked {
            self.looper.select_loop(n);
        }
        self.looper.command(Command::Solo(on));
        if n != picked {
            self.looper.select_loop(picked);
        }
        format!(
            "looper: loop {} {}",
            n + 1,
            if on {
                "soloed: the loops not soloed go quiet"
            } else {
                "solo off"
            }
        )
    }

    pub fn toggle_mute(&mut self, n: Option<usize>) -> String {
        let n = n.unwrap_or(self.picked).min(MAX_LOOPS - 1);
        let on = !self.looper.loop_status(n).is_some_and(|s| s.muted);
        self.set_mute(n, on)
    }

    /// Loop `n` muted (`on`) or let play, leaving the pick as it was: a
    /// learned control's mute (MIDI Learn), which says which way, so two
    /// moves before the looper takes the first can't undo each other.
    pub fn set_mute(&mut self, n: usize, on: bool) -> String {
        let picked = self.picked;
        let n = n.min(MAX_LOOPS - 1);
        let Some(s) = self.looper.loop_status(n) else {
            return format!("looper: loop {}: not there", n + 1);
        };
        if s.phase == Phase::Empty {
            return format!("looper: loop {}: empty, nothing to mute", n + 1);
        }
        // Pick it, mute, pick the old one back: all three or none.
        let need = if n == picked { 1 } else { 3 };
        if self.looper.room() < need {
            return format!(
                "looper: loop {} {}: NOT done, the command queue is full",
                n + 1,
                if on { "mute" } else { "unmute" }
            );
        }
        if n != picked {
            self.looper.select_loop(n);
        }
        self.looper.command(Command::Mute(on));
        if n != picked {
            self.looper.select_loop(picked);
        }
        format!(
            "looper: loop {} {}",
            n + 1,
            if on {
                "muted: silent, PLAY doesn't start it"
            } else {
                "unmuted: it plays with the others"
            }
        )
    }

    /// Loop `n` on or off, as a clip launches: from the bar
    /// line at song tick `at`, on the frame it falls on, or with `None` at
    /// once ([`Looper::launch`]). The line for the log.
    pub fn launch(&mut self, n: usize, on: bool, at: Option<mpfree_transport::Tick>) -> String {
        let what = if on { "on" } else { "off" };
        if n >= MAX_LOOPS {
            return format!("looper: loop {}: not there", n + 1);
        }
        if !self.looper.launch(n, on, at) {
            return format!(
                "looper: loop {} {what}: NOT done, the command queue is full",
                n + 1
            );
        }
        match at {
            Some(t) => {
                format!(
                    "looper: loop {} {what} from bar {}",
                    n + 1,
                    t.div_euclid(self.bar_ticks.max(1)) + 1
                )
            }
            None => format!("looper: loop {} {what}", n + 1),
        }
    }

    /// The loops as the grid's loop cells need them
    /// ([`crate::time::TimeHost::loops_are_on`]): which have a first pass,
    /// how long and from where. None while an open's loops are going out
    /// or coming in, so a project's cells aren't emptied by the old loops
    /// fading out. A free loop ([`Clock::Free`]) is told with `free`, as
    /// its bars aren't the song's: it lands in a free cell (phase 4e.4),
    /// which the looper still launches on its clock.
    pub fn grid_now(&self) -> Option<[LoopNow; MAX_LOOPS]> {
        if self.opening() {
            return None;
        }
        Some(std::array::from_fn(|n| match self.looper.loop_status(n) {
            // A loop still asking how to read its Length (Read as or
            // Discard) is no pass yet: it lands once answered.
            Some(s)
                if matches!(s.phase, Phase::Playing | Phase::Overdubbing)
                    && s.ticks > 0
                    && s.asks.is_none() =>
            {
                LoopNow {
                    pass: true,
                    ticks: s.ticks,
                    origin: s.origin,
                    free: s.clock == Clock::Free,
                }
            }
            _ => LoopNow::default(),
        }))
    }

    /// The free cells (audio tracks phase 4e.4), before the period: each
    /// free loop a cell names goes on or off as `plan` has it (the time
    /// engine's `free_plan`): on, on the free clock's next bar line (at
    /// once with no free clock running); off, at once. An empty loop
    /// waits; a full queue tries again next period. Lines for the log;
    /// allocates only then.
    pub fn follow_free(&mut self, plan: &[Option<bool>; MAX_LOOPS]) -> Vec<String> {
        let mut lines = Vec::new();
        let running = self.looper.free_clock().is_some_and(|c| c.running);
        for (n, want) in plan.iter().enumerate() {
            let Some(on) = *want else {
                continue;
            };
            let Some(s) = self.looper.loop_status(n) else {
                continue;
            };
            let there = match s.cue {
                None => s.on == on,
                Some(cue) => cue.0 == on,
            };
            if there || s.phase == Phase::Empty || s.clock != Clock::Free || self.looper.room() == 0
            {
                continue;
            }
            let at = (on && running).then_some(mpfree_transport::Tick::MIN);
            if self.looper.launch(n, on, at) {
                lines.push(format!(
                    "looper: free loop {} {}",
                    n + 1,
                    match (on, at) {
                        (true, Some(_)) => "on, on the free clock's next bar line",
                        (true, None) => "on",
                        (false, _) => "off",
                    }
                ));
            }
        }
        lines
    }

    /// A loop in no cell plays, as loops did before the grid had them,
    /// before the period: each one that is off, or waits on a cue, goes on
    /// at once. A loop on an audio track goes where its track says, and a
    /// free cell's loop where its cell does (`follow_free`). An empty loop
    /// waits. Lines for the log; allocates only then. A full queue tries
    /// again next period.
    pub fn follow_untracked(&mut self) -> Vec<String> {
        let mut lines = Vec::new();
        for n in 0..MAX_LOOPS {
            let Some(s) = self.looper.loop_status(n) else {
                continue;
            };
            let there = s.cue.is_none() && s.on;
            let on_track = self.looper.on_track(n) || self.looper.free_tracked(n);
            if there || on_track || s.phase == Phase::Empty || self.looper.room() == 0 {
                continue;
            }
            lines.push(self.launch(n, true, None));
        }
        lines
    }

    /// A layer dragged onto another loop ([`Looper::copy_layer`]): what
    /// it does, as the screen says it, and why not when it can't. A loop's
    /// only layer goes as a copy (the loop keeps it).
    pub fn copy_layer(&mut self, from: usize, id: u8, to: usize, keep: bool) -> String {
        self.steps_left = 0;
        let (Some(a), Some(b)) = (self.looper.loop_status(from), self.looper.loop_status(to))
        else {
            return "looper: no such loop".to_string();
        };
        let k = (0..MAX_LAYERS).find(|&k| {
            self.looper
                .loop_layer_info(from, k)
                .is_some_and(|l| l.id == id)
        });
        let Some(k) = k else {
            return format!("looper: loop {} has no such layer now", from + 1);
        };
        let bars = |s: &Status| {
            let b = (s.ticks as f64 / self.bar_ticks.max(1) as f64).round();
            format!("{b} bar{}", if b == 1.0 { "" } else { "s" })
        };
        let (layer, there, here) = (k + 1, to + 1, from + 1);
        if from == to {
            return format!("looper: Layer {layer} is in loop {there} already");
        }
        if b.phase != Phase::Empty && b.ticks != a.ticks {
            return format!(
                "looper: loop {there} is {} and Layer {layer} {}: a layer goes only onto a loop of its length",
                bars(&b),
                bars(&a)
            );
        }
        if !matches!(b.phase, Phase::Empty | Phase::Playing) || a.phase != Phase::Playing {
            return format!("looper: Layer {layer} can't go now: a loop is recording");
        }
        let copy = keep || a.layers <= 1;
        if !self.looper.copy_layer(from, id, to, keep) {
            return "looper: the drop is NOT done, the command queue is full".to_string();
        }
        let undo = "UNDO takes it back";
        match (b.phase == Phase::Empty, copy) {
            (true, false) => format!("looper: Layer {layer} moves into loop {there}, a loop of its own ({undo})"),
            (true, true) if a.layers <= 1 => format!(
                "looper: a copy of loop {here} goes into loop {there} (it has one layer, so loop {here} keeps it)"
            ),
            (true, true) => format!("looper: a copy of Layer {layer} goes into loop {there}, a loop of its own"),
            (false, false) => format!("looper: Layer {layer} moves to loop {there} ({undo})"),
            (false, true) if a.layers <= 1 => format!(
                "looper: a copy of Layer {layer} goes onto loop {there} (loop {here} keeps its only layer)"
            ),
            (false, true) => format!("looper: a copy of Layer {layer} goes onto loop {there} ({undo})"),
        }
    }

    /// A layer's control, by the layer's id, with its `looper:` line for
    /// the log ("layer 3 muted").
    pub fn layer_command(&mut self, id: u8, c: LayerCommand) -> String {
        if c == LayerCommand::Delete {
            self.steps_left = 0;
            // The last layer can't be deleted on its own (the looper keeps
            // it): its × clears the loop, as Clear does (13 Nov live test,
            // where the first pass's × said "deleted" and stayed).
            let s = self.looper.status();
            let last = s.layers <= 1
                && (0..MAX_LAYERS).any(|k| self.looper.layer_info(k).is_some_and(|l| l.id == id));
            if last && s.phase != Phase::Empty {
                return self.command(Command::Clear);
            }
        }
        let k = (0..MAX_LAYERS)
            .find(|&k| self.looper.layer_info(k).is_some_and(|l| l.id == id))
            .map_or_else(|| format!("layer id {id}"), |k| format!("layer {}", k + 1));
        let what = match c {
            LayerCommand::Mute(true) => "muted".to_string(),
            LayerCommand::Mute(false) => "unmuted".to_string(),
            LayerCommand::Solo(true) => "soloed".to_string(),
            LayerCommand::Solo(false) => "solo off".to_string(),
            LayerCommand::Level(v) => format!("level {:.1} dB", 20.0 * v.max(1e-6).log10()),
            LayerCommand::Pan(p) => format!("pan {p:+.2}"),
            LayerCommand::Delete => "deleted (Undo brings it back)".to_string(),
        };
        if !self.looper.layer_command(id, c) {
            return format!("looper: {k} {what}: NOT done, the command queue is full");
        }
        format!("looper: {k} {what}")
    }

    /// The length of the pattern playing, in bars (the longest clip that
    /// plays, rounded up to whole bars; None when nothing plays, or past
    /// [`PATTERN_MAX_BARS`]). With Length: Pattern the picked loop, while
    /// empty, is kept at it, so a tap records exactly one pattern's worth.
    pub fn set_pattern_bars(&mut self, bars: Option<u32>) {
        let bars = bars.filter(|&b| (1..=PATTERN_MAX_BARS).contains(&b));
        self.pattern_bars = bars;
        if !self.match_pattern {
            return;
        }
        let s = self.looper.status();
        if s.phase == Phase::Empty && s.capturing.is_none() && s.bars != bars {
            self.looper.command(Command::SetBars(bars));
        }
    }
}
