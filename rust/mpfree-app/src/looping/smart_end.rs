//! Smart loop end: its snaps, the questions it asks, and what it made of a
//! first pass, every period.

use super::*;

impl LoopHost {
    /// Smart loop end's Snap ([`Snap::Found`]), Use my tap ([`Snap::Tap`]),
    /// Use my press, Read as ([`Snap::Bars`]), or UNDO or REDO of one, on
    /// the picked loop: the loop and the song's tempo change together on
    /// its next downbeat. Worked out and asked of the time engine at the
    /// next period ([`LoopHost::tend_snap`]); what it did, or why not,
    /// comes after. The line for the log now.
    pub fn snap(&mut self, s: Snap) -> String {
        self.snap_on(self.looper.selected(), s)
    }

    /// [`LoopHost::snap`] on loop `n`, picked or not.
    pub(crate) fn snap_on(&mut self, n: usize, s: Snap) -> String {
        self.snap_ask = None;
        self.ab_stop();
        self.side_want = None;
        self.moving = None;
        self.snap_want = Some((n, s, false));
        format!(
            "looper: {}: Loop {} changes on its next downbeat, with the tempo",
            snap_name(s, self.reads_again(s)),
            n + 1
        )
    }

    /// Whether snap `s` is an Undo or a Redo of Read as, before it lands.
    pub(crate) fn reads_again(&self, s: Snap) -> bool {
        let h = self.looper.history();
        let step = match s {
            Snap::Undo => h.done().last(),
            Snap::Redo => h.redo().first(),
            _ => None,
        };
        matches!(step, Some(Step::Bars(_)))
    }

    /// The snap waiting on the question "... anyway?" (it would change
    /// the tempo under recorded audio), if one is: its loop and kind, for
    /// the tests (the screen reads [`SmartView::asking`]).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn snap_asking(&self) -> Option<(usize, Snap)> {
        self.snap_ask
    }

    /// A first pass that closed waiting on its Length ([`Status::asks`]):
    /// the line saying so ([`length_ask_words`]), once a wait, on the
    /// screen too.
    pub fn take_asks(&mut self) -> Option<(String, bool)> {
        let mut line = None;
        for n in 0..MAX_LOOPS {
            let bit = 1u32 << n;
            match self.looper.loop_status(n).and_then(|s| s.asks) {
                Some(r) if self.asks_told & bit == 0 => {
                    self.asks_told |= bit;
                    line = line.or(Some((format!("looper: {}", length_ask_words(n, &r)), true)));
                }
                None => self.asks_told &= !bit,
                Some(_) => {}
            }
        }
        line
    }

    /// The picked loop's first pass waiting on its Length, if it is:
    /// its number and how it read.
    pub fn length_asking(&self) -> Option<(usize, Reading)> {
        let n = self.looper.selected();
        self.looper.loop_status(n)?.asks.map(|r| (n, r))
    }

    /// Read as (the hint line's yes while a first pass waits on its
    /// Length): it plays as it read ([`Command::UseReading`]). The line
    /// for the log, or None if nothing waits.
    pub fn use_reading(&mut self) -> Option<String> {
        let (n, r) = self.length_asking()?;
        self.looper.command(Command::UseReading);
        Some(format!(
            "looper: Loop {} plays as {}, from its top",
            n + 1,
            reading_words(&r)
        ))
    }

    /// Discard (the hint line's no while a first pass waits on its
    /// Length): the pass is cleared, as Clear does (UNDO brings it back).
    pub fn discard_waiting(&mut self) -> Option<String> {
        let (n, _) = self.length_asking()?;
        self.looper.command(Command::Clear);
        Some(format!(
            "looper: Loop {}'s pass thrown away (UNDO brings it back)",
            n + 1
        ))
    }

    /// Snap anyway: the snap that asked goes ahead, as [`LoopHost::snap`]
    /// does, the audio tracks' recordings left at their old tempo. The
    /// line for the log, or None if none asked.
    pub fn snap_anyway(&mut self) -> Option<String> {
        let (n, s) = self.snap_ask.take()?;
        self.snap_want = Some((n, s, true));
        Some(format!(
            "looper: {} anyway: Loop {} changes on its next downbeat, with the tempo",
            snap_name(s, self.reads_again(s)),
            n + 1
        ))
    }

    /// Not now: the snap that asked is dropped and nothing changes. The
    /// line for the log, or None if none asked.
    pub fn snap_not_now(&mut self) -> Option<String> {
        let (n, _) = self.snap_ask.take()?;
        Some(format!(
            "looper: Loop {}: not now: it stays as it is",
            n + 1
        ))
    }

    /// Every period, after the time engine's and before
    /// [`LoopHost::run`]: a snap asked for is worked out
    /// ([`Looper::snap_plan`]) and its tempo asked of the time engine on
    /// the loop's next downbeat ([`TimeHost::timing_change`]): refused
    /// while another song loop has sound (its tempo would change under it;
    /// a free loop keeps the free clock's), or whatever the time engine
    /// refuses. While `audio` (recorded audio, the audio tracks' regions,
    /// would play out of step) it asks first ([`LoopHost::snap_asking`])
    /// and goes ahead on Snap anyway. Once the time engine commits it, the
    /// loop is told to change on the same frame ([`Looper::snap_at`]); if
    /// the loop changes first, the tempo is taken back. A free loop's
    /// changes the free clock's tempo instead: no other loop but a free
    /// one is in its way, nothing is asked of the time engine, and the
    /// looper gets the plan in the period its frame falls in. A refusal:
    /// the line for the log and the screen, counted.
    pub fn tend_snap(&mut self, time: &mut TimeHost, audio: bool) -> Option<(String, bool)> {
        if let Some(p) = self.free_snap {
            if self
                .looper
                .loop_status(p.n)
                .is_none_or(|s| s.version != p.version)
            {
                self.free_snap = None;
                return Some(self.snap_refused(
                    p.n,
                    p.snap,
                    "the loop changed before its downbeat",
                ));
            }
            if p.at < time.next_frame() + PERIOD as u64 {
                // Said once it has landed (take_snap).
                self.free_snap = None;
                self.looper.snap_at(p);
            }
            return None;
        }
        if let Some((p, ticket)) = self.snap_wait {
            if self
                .looper
                .loop_status(p.n)
                .is_none_or(|s| s.version != p.version)
            {
                self.snap_wait = None;
                time.cancel_timing(ticket, "the loop changed");
                return Some(self.snap_refused(
                    p.n,
                    p.snap,
                    "the loop changed before its downbeat",
                ));
            }
            match time.timing_answer(ticket) {
                Some(Answer::Waiting) => return None,
                Some(Answer::Committed(at)) => {
                    self.snap_wait = None;
                    // Said once it has landed (take_snap).
                    self.looper.snap_at(SnapPlan { at, ..p });
                    return None;
                }
                Some(Answer::Cancelled(why) | Answer::Refused(why)) => {
                    self.snap_wait = None;
                    return Some(self.snap_refused(p.n, p.snap, why));
                }
                None => {
                    self.snap_wait = None;
                    return Some(self.snap_refused(p.n, p.snap, "the time engine lost it"));
                }
            }
        }
        if self.snap_want.is_none() {
            let (n, side, keep) = self.side_want?;
            match self.snap_to(n, side) {
                Some(s) => self.snap_want = Some((n, s, false)),
                None => {
                    // There already (or neither side any more).
                    self.side_want = None;
                    if keep {
                        let v = self.looper.loop_status(n).map_or(0, |s| s.version);
                        self.kept = Some((n, v));
                    }
                    return None;
                }
            }
        }
        let (n, s, anyway) = self.snap_want.take()?;
        // A free loop's snap changes the free clock's tempo, not the
        // song's: song loops and the audio tracks aren't in its way, the
        // looper says if another free loop is, and there is nothing to ask
        // the time engine.
        if self
            .looper
            .loop_status(n)
            .is_some_and(|st| st.clock == Clock::Free)
        {
            return match self.looper.snap_plan(n, s, time.next_frame()) {
                Ok(p) => {
                    self.free_snap = Some(p);
                    None
                }
                Err(why) => Some(self.snap_refused(n, s, why)),
            };
        }
        // Another song loop with sound would change tempo under it; a
        // free loop keeps the free clock's.
        let others = (0..MAX_LOOPS)
            .filter(|&k| k != n)
            .filter_map(|k| self.looper.loop_status(k))
            .any(|st| st.phase != Phase::Empty && st.clock == Clock::Song);
        if others {
            return Some(self.snap_refused(n, s, "another song loop has sound"));
        }
        let p = match self.looper.snap_plan(n, s, time.next_frame()) {
            Ok(p) => p,
            Err(why) => return Some(self.snap_refused(n, s, why)),
        };
        let c = TimingChange {
            at: p.at,
            tempo: Some(p.bpm),
            position: None,
            play: false,
            take: None,
            made_against: time.timeline(),
        };
        match time.allowed(&c, audio) {
            Allowed::Safe => {}
            Allowed::Ask(_) if anyway => {}
            Allowed::Ask(_) => {
                self.snap_ask = Some((n, s));
                self.side_want = None;
                self.ab_stop();
                return Some((
                    format!("looper: {}", ask_words(n, s, self.reads_again(s))),
                    true,
                ));
            }
            Allowed::Refused(why) => return Some(self.snap_refused(n, s, why)),
        }
        match time.timing_change(c) {
            Ok(ticket) => {
                self.snap_wait = Some((p, ticket));
                None
            }
            Err(why) => Some(self.snap_refused(n, s, why)),
        }
    }

    /// Snap `s` on loop `n` can't be made (`why`): counted; the line for
    /// the log and the screen.
    fn snap_refused(&mut self, n: usize, s: Snap, why: &str) -> (String, bool) {
        self.snaps_refused += 1;
        // A side asked for, and A/B, stop there.
        self.side_want = None;
        self.ab_stop();
        (
            format!(
                "looper: Loop {}: can't {}: {why}",
                n + 1,
                snap_verb(s, self.reads_again(s))
            ),
            true,
        )
    }

    /// After the looper's period: what became of a snap given it
    /// ([`Looper::take_snap`]): the line for the log, said on the screen
    /// too ("snapped the end 48 ms earlier: 2 bars at 120.00 BPM"), and the tempo
    /// the song now plays at, for the app's (it changed with the loop, or
    /// already had when a loop changed first).
    pub fn take_snap(&mut self) -> (Option<(String, bool)>, Option<f64>) {
        let Some(SnapDone { plan: p, landed }) = self.looper.take_snap() else {
            return (None, None);
        };
        let n = p.n + 1;
        // A free loop's snap changes the free clock's tempo, never the
        // song's.
        let free = self
            .looper
            .loop_status(p.n)
            .is_some_and(|s| s.clock == Clock::Free);
        let song_bpm = (!free).then_some(p.bpm);
        if !landed {
            self.snaps_refused += 1;
            let why = if free {
                "the loop changed, or the free loops stopped first"
            } else {
                "the loop changed (the tempo did)"
            };
            return (
                Some((
                    format!(
                        "looper: Loop {n}: {} didn't land: {why}",
                        snap_name(p.snap, self.reads_again(p.snap))
                    ),
                    true,
                )),
                song_bpm,
            );
        }
        let Some(s) = self.looper.loop_status(p.n) else {
            return (None, song_bpm);
        };
        let bar_ticks = match self.looper.free_clock().filter(|_| free) {
            Some(c) => c.meter.bar_ticks(),
            None => self.bar_ticks,
        };
        let bars = ticks_text(s.ticks, bar_ticks);
        let moves = shift_text(p.by, self.rate);
        // Landed: an Undo's step is the next Redo's now, and the other way.
        let h = self.looper.history();
        let read = |s: Option<&Step>| matches!(s, Some(Step::Bars(_)));
        let what = match p.snap {
            Snap::Found => format!("snapped {moves}"),
            Snap::To(_) => format!("moved by hand {moves}"),
            Snap::Tap => "back to your tap".to_string(),
            Snap::Press => "back to your press".to_string(),
            Snap::Bars(_) => "read again".to_string(),
            Snap::Undo if read(h.redo().first()) => "Undo: read as before".to_string(),
            Snap::Redo if read(h.done().last()) => "Redo: read again".to_string(),
            Snap::Undo => format!("Undo: {moves}"),
            Snap::Redo => format!("Redo: {moves}"),
        };
        let at = if free { ", free at" } else { " at" };
        (
            Some((
                format!(
                    "looper: Loop {n}: {what}: {bars}{at} {:.2} BPM, from its downbeat",
                    p.bpm
                ),
                true,
            )),
            song_bpm,
        )
    }

    /// After the looper's period: what smart loop end made of a closing
    /// tap ([`Looper::take_smart`]), once a pass. The line for the log
    /// (with the best two ends, scored) and, when your tap stood, the
    /// words for the screen.
    pub fn take_smart(&mut self) -> Option<(String, Option<String>)> {
        let note = self.looper.take_smart()?;
        Some(smart_words(&note, self.smart, self.rate))
    }

    /// The song can't start at the tempo loop `n` set (`why`): counted;
    /// the line for the log and the screen. The loop plays at the song's
    /// tempo when it starts.
    pub fn tempo_refused(&mut self, n: usize, bars: u32, bpm: f64, why: &str) -> String {
        self.tempos_refused += 1;
        format!(
            "looper: Loop {} is {} at {bpm:.2} BPM, but the song can't start at that tempo: \
             {why}. The loop plays at the song's tempo when it starts",
            n + 1,
            bars_words_n(bars)
        )
    }
}
