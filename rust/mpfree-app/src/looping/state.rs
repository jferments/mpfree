//! What the looper is doing, for the screen, the lights, MIDI Learn and the
//! status line.

use super::*;

impl LoopHost {
    /// Whether a loop is armed or records its first pass, and whether one
    /// overdubs: the REC and OVERDUB lights ([`crate::app::App::transport_lights`]).
    pub fn recording(&self) -> (bool, bool) {
        let (mut rec, mut dub) = (false, false);
        for s in (0..MAX_LOOPS).filter_map(|n| self.looper.loop_status(n)) {
            match s.phase {
                Phase::Armed | Phase::Recording => rec = true,
                Phase::Overdubbing => dub = true,
                Phase::Empty | Phase::Playing => {}
            }
        }
        (rec, dub)
    }

    /// For the arm ([`crate::rec_arm::Now`]): whether a loop records its
    /// first pass or overdubs, and whether the picked loop is armed (its
    /// first pass waits for PLAY or the bar line).
    pub fn rec_now(&self) -> (bool, bool) {
        let taking = (0..MAX_LOOPS)
            .filter_map(|n| self.looper.loop_status(n))
            .any(|s| matches!(s.phase, Phase::Recording | Phase::Overdubbing));
        (taking, self.looper.status().phase == Phase::Armed)
    }

    /// The loop a new one records into, for a Loop cell armed on the
    /// grid: the picked loop if it is empty, else the first empty one; or
    /// why there is none, as REC says it. Less than a second of memory
    /// left is full: a first pass with no memory for it is thrown away.
    pub fn free_loop(&self) -> Result<usize, String> {
        let copy = self.copy.map(|c| c.to);
        let empty = |n: usize| {
            copy != Some(n)
                && self.looper.loop_status(n).is_some_and(|s| {
                    s.phase == Phase::Empty
                        && s.copying.is_none()
                        && s.loading.is_none()
                        && s.capturing.is_none()
                })
        };
        let n = if empty(self.picked) {
            self.picked
        } else {
            (0..MAX_LOOPS).find(|&n| empty(n)).ok_or_else(|| {
                format!("{MAX_LOOPS} loops in use: clear one on the Looper tab to record another")
            })?
        };
        let m = self.looper.memory();
        if m.capacity.saturating_sub(m.used) < self.rate as usize {
            return Err(
                "The loops' memory is full: clear a loop on the Looper tab to record another"
                    .to_string(),
            );
        }
        Ok(n)
    }

    /// What the picked loop is doing (armed, recording, overdubbing...).
    pub fn phase(&self) -> Phase {
        self.looper.status().phase
    }

    /// Loop `n` is a free loop with sound, stopped with the free clock
    /// (as [`LooperView::free_stopped`]): its ring or key plays the free
    /// loops.
    pub fn free_stopped(&self, n: usize) -> bool {
        self.looper.loop_status(n).is_some_and(|l| {
            l.clock == Clock::Free
                && l.phase == Phase::Playing
                && !l.muted
                && self.looper.free_clock().is_some_and(|c| !c.running)
        })
    }

    /// Loop `n` as the looper has it (the loop keys' lights and steps).
    pub fn loop_status(&self, n: usize) -> Option<Status> {
        self.looper.loop_status(n)
    }

    /// The loop picked (0 is Loop 1): the buttons and a learned control's
    /// picked loop act on it.
    pub fn picked(&self) -> usize {
        self.picked
    }

    /// Sync as last set, for every empty loop ([`Clock::Song`] at first).
    pub fn sync(&self) -> Clock {
        self.clock
    }

    /// The loops muted, bit `n` for loop `n`, as MIDI Learn reads them.
    pub fn mute_bits(&self) -> u8 {
        (0..MAX_LOOPS.min(8)).fold(0, |bits, n| {
            let muted = self
                .looper
                .loop_status(n)
                .is_some_and(|s| s.muted && s.phase != Phase::Empty);
            bits | (u8::from(muted) << n)
        })
    }

    /// The `status: looper: ...` line.
    pub fn describe(&self) -> String {
        let s = self.looper.status();
        let st = self.looper.stats();
        format!("looper: {:?}, {} layers ({} to redo), loop {}, first pass length {}, latency {} frames, level {:.1} \
                 dB{}; records from {}; line input {}{}, monitor {}; {} overdubs refused, {} first passes thrown \
                 away, {} commands dropped, {} periods kept within ±1; {}prints {} made, {} too late, {} failed",
                s.phase, s.layers, s.redo,
                if s.frames > 0 { format!("{} frames ({:.2} s)", s.frames, s.frames as f64 / 44_100.0) } else { "none".to_string() },
                bars_text(s.bars), s.latency, 20.0 * s.level.max(1e-6).log10(), if s.muted { ", muted" } else { "" },
                self.source.name().to_lowercase(), if self.line.is_open() { "on" } else { "off" },
                if self.line.is_open() || self.line.stats.periods > 0 { format!(" ({})", self.line.describe()) } else { String::new() },
                if self.monitor { "on" } else { "off" }, st.refused, st.aborted, st.dropped_commands, self.clipped,
                if s.printed { "a print plays (pitch kept at this tempo); " } else { "" },
                self.printer.made, self.printer.stale, self.printer.failed)
            + &if self.waves.waits > 0 {
                format!("; {} waveforms waited a period for the screen", self.waves.waits)
            } else {
                String::new()
            }
            + &if self.starts_refused > 0 {
                format!("; {} starts on a note refused", self.starts_refused)
            } else {
                String::new()
            }
            + &if self.tempos_refused > 0 {
                format!("; {} tempos from a loop refused", self.tempos_refused)
            } else {
                String::new()
            }
            + &if st.taps_kept + st.ends_moved + st.ends_offered > 0 {
                format!(
                    "; smart loop end: {} ends moved, {} offered, {} taps kept, {} starts on \
                     the first hit, {} overdubs begun with what was played after the end",
                    st.ends_moved, st.ends_offered, st.taps_kept, st.starts_found, st.handoffs
                )
            } else {
                String::new()
            }
            + &if st.hits_heard + st.finds_asked > 0 || self.finder.gone {
                format!(
                    "; by ear: {} hits heard, {} decisions asked of the finder, {} answered in \
                     time, {} too late (the tap kept), {} answers dropped (late, or for a take \
                     gone), {} periods an ask waited for the finder{}",
                    st.hits_heard,
                    st.finds_asked,
                    st.finds_answered,
                    st.finds_late,
                    st.finds_stale,
                    self.finder.waited,
                    if self.finder.gone {
                        ", its thread is gone (taps are kept)"
                    } else {
                        ""
                    }
                )
            } else {
                String::new()
            }
            + &if self.snaps_refused > 0 {
                format!("; {} snaps refused or not landed", self.snaps_refused)
            } else {
                String::new()
            }
            + &if self.songs_refused > 0 {
                format!("; Make song refused {} times", self.songs_refused)
            } else {
                String::new()
            }
            + &{
                // Layers as recordings (phase 4c.3): layers only in memory
                // so far, and files that couldn't be written.
                let only = (0..MAX_LOOPS).map(|n| self.unsaved_layers(n)).sum::<usize>();
                let failed = self.layer_writer.failed;
                match (only, failed) {
                    (0, 0) => String::new(),
                    (only, 0) => format!("; {only} layers only in memory"),
                    (only, failed) => format!(
                        "; {only} layers only in memory, {failed} layers' files couldn't be written"
                    ),
                }
            }
            + &if self.notes_lost > 0 {
                format!(
                    "; {} periods with more notes than smart loop end could follow",
                    self.notes_lost
                )
            } else {
                String::new()
            }
            + &if self.readings_overruled > 0 {
                format!(
                    "; {} loops too long for their Length's bars (read instead)",
                    self.readings_overruled
                )
            } else {
                String::new()
            }
            + &if self.dropouts > 0 {
                format!("; {} times a take's source was lost while it recorded (silence)", self.dropouts)
            } else {
                String::new()
            }
            + &if self.chosen != self.source {
                format!("; next take from {}", self.chosen.name().to_lowercase())
            } else {
                String::new()
            }
    }

    /// What the Looper tab shows; takes the meter's peaks.
    pub fn view(&mut self, bpm: f64) -> LooperView {
        let m = std::mem::take(&mut self.meter);
        let db = |v: f32| {
            if v > 0.0 {
                20.0 * v.log10()
            } else {
                f32::NEG_INFINITY
            }
        };
        let s = self.looper.status();
        // A beat of the meter (a quarter in 4/4, an eighth in 6/8) at the
        // tempo the loop was recorded at, from its length; the tempo counts
        // quarters.
        let beat = self.bar_ticks.max(1) as f64 / f64::from(self.beats_per_bar.max(1));
        let frames_per_beat = if s.ticks > 0 {
            s.frames as f64 * beat / s.ticks as f64
        } else {
            f64::from(self.rate) * 60.0 / bpm.max(1.0) * beat / f64::from(PPQ)
        };
        let mut layers = [None; MAX_LAYERS];
        for (k, l) in layers.iter_mut().enumerate() {
            *l = self.looper.layer_info(k);
        }
        let mut loops = [s; MAX_LOOPS];
        for (n, l) in loops.iter_mut().enumerate() {
            if let Some(st) = self.looper.loop_status(n) {
                *l = st;
            }
        }
        let smart = self.smart_view(&s);
        LooperView {
            smart,
            status: s,
            stats: self.looper.stats(),
            source: self.chosen,
            taking_from: (self.source != self.chosen).then_some(self.source),
            resample: self.resample,
            line_on: self.line.is_open(),
            line: self.line.stats,
            monitor: self.monitor,
            meter_db: [db(m[0]), db(m[1])],
            beats_per_bar: self.beats_per_bar,
            frames_per_beat,
            bar_ticks: self.bar_ticks,
            layers,
            loops,
            selected: self.looper.selected(),
            memory: self.looper.memory(),
            rate: self.rate,
            history: self.looper.history(),
            cell_waves: self.cells.done,
            ring_waves: self.cells.rings,
            copy_into: self.copy.map(|c| c.to),
            heard: self.heard_wave(),
            capture_into: self.capture_into(),
            free_tempo: self.free_tempo,
            match_pattern: self.match_pattern,
            pattern_bars: self.pattern_bars,
            listening: self.looper.listening(),
            clock: self.clock,
            free: self.looper.free_clock(),
            free_now: self.looper.free_now().unwrap_or(0.0),
            free_cued: self.looper.free_cued(),
            song: self.song_button(),
            ids: self.on_tracks.ids(),
            unsaved: std::array::from_fn(|n| self.unsaved_layers(n) > 0),
        }
    }
}
