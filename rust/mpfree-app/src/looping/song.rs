//! Make song: free loops into a song and back, on the one history.

use super::*;

/// The song as Make song found it: its tempo and place, and whether Make
/// song starts it (the free loops played).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SongBefore {
    bpm: f64,
    position: Tick,
    starts: bool,
}

/// Make song as it goes ([`LoopHost::make_song`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum SongStep {
    #[default]
    Idle,
    /// Pressed (or REDO): worked out in the next period, with the time
    /// engine ([`LoopHost::tend_song`]).
    Want,
    /// Given to the looper: it waits for its frame.
    Waits(SongPlan, SongBefore),
    /// Made: what UNDO takes back while every loop is at the version it
    /// was then (nothing else has changed).
    Made(SongPlan, SongBefore, [u64; MAX_LOOPS]),
    /// UNDO asked for: taken back in the next period, with the time
    /// engine.
    Unmake(SongPlan, SongBefore),
    /// Taken back: REDO makes it again while every loop is at the version
    /// it was then.
    Unmade([u64; MAX_LOOPS]),
}

impl LoopHost {
    /// Make song (a free loop's header): every free loop with sound
    /// becomes a song loop where it is, and the song takes their tempo,
    /// its bar lines on theirs ([`Looper::song_plan`]); worked out in the
    /// next period, with the time engine ([`LoopHost::tend_song`]). The
    /// line for the log.
    pub fn make_song(&mut self) -> String {
        if matches!(self.song, SongStep::Waits(..)) {
            return "looper: Make song waits for the free loops' bar line".to_string();
        }
        self.song = SongStep::Want;
        self.song_txn = mpfree_transport::TransactionId::default();
        "looper: Make song".to_string()
    }

    /// Make song on the one history (Dev's loops on audio tracks, phase
    /// 4d.2): its transaction on the done side while made, on the redo
    /// side once taken back; neither while it is being worked out or once
    /// it can't be taken back or made again. Into `done` and `redo`,
    /// cleared first.
    pub fn song_steps(
        &self,
        done: &mut Vec<mpfree_transport::TransactionId>,
        redo: &mut Vec<mpfree_transport::TransactionId>,
    ) {
        done.clear();
        redo.clear();
        if !self.song_txn.is_some() {
            return;
        }
        match self.song {
            SongStep::Made(..) | SongStep::Unmake(..) => done.push(self.song_txn),
            SongStep::Unmade(_) => redo.push(self.song_txn),
            _ => {}
        }
    }

    /// Make song's step as UNDO names it: "Make song · Loops 1 and 2".
    pub fn song_words(&self) -> String {
        match self.song {
            SongStep::Made(p, ..) | SongStep::Unmake(p, _) => {
                format!("Make song · {}", loops_words(p.loops))
            }
            _ => "Make song".to_string(),
        }
    }

    /// The one UNDO (`redo` false) or REDO on Make song's step (phase
    /// 4d.2): taken back, or made again, in the next period with the time
    /// engine ([`LoopHost::tend_song`]), which says if the looper refuses
    /// it. The one history takes steps back newest first, so the loops'
    /// versions aren't asked, as the Looper tab's own UNDO asks them. The
    /// line for the log.
    pub fn song_undo(&mut self, redo: bool) -> String {
        match (self.song, redo) {
            (SongStep::Made(p, b, _), false) => {
                self.song = SongStep::Unmake(p, b);
                "looper: UNDO: Make song, the loops free again".to_string()
            }
            (SongStep::Unmade(_), true) => {
                self.song = SongStep::Want;
                "looper: REDO: Make song".to_string()
            }
            _ => "looper: Make song has nothing to take back or make again".to_string(),
        }
    }

    /// The one history can no longer reach Make song's step (phase 4d.3's
    /// forgetting, for 4d.2): taken back with nothing to redo (`redo`), or
    /// made at or before `through`. Whether it went.
    pub fn forget_song(&mut self, redo: bool, through: mpfree_transport::TransactionId) -> bool {
        let gone = match self.song {
            SongStep::Unmade(_) => redo,
            SongStep::Made(..) => through.is_some() && self.song_txn <= through,
            _ => false,
        };
        if gone {
            self.song = SongStep::Idle;
        }
        gone
    }

    /// Make song's button, as the screen shows it.
    pub(crate) fn song_button(&self) -> SongButton {
        if matches!(self.song, SongStep::Want | SongStep::Waits(..)) {
            SongButton::Waits
        } else if self.looper.song_stopped() {
            SongButton::Ready
        } else {
            SongButton::Faint
        }
    }

    /// Every loop's version: Make song's UNDO holds while none has moved
    /// on.
    pub(crate) fn versions(&self) -> [u64; MAX_LOOPS] {
        std::array::from_fn(|n| self.looper.loop_status(n).map_or(0, |s| s.version))
    }

    /// Make song's turn, before the period: Make song pressed is worked
    /// out, and the song started on the free loops' next bar line at their
    /// tempo (`time.start_at`, from the start of the bar it is stopped
    /// in, as Tempo from the first loop starts it), or, with the free
    /// loops stopped, given that tempo and place at once; the looper gets
    /// the plan, to change the loops' clock on the same frame
    /// ([`Looper::make_song`]). Or UNDO's: the loops free again on a copy
    /// of the song's clock ([`Looper::make_free`]), then the song back at
    /// its tempo and place, stopped if Make song started it. Refused while
    /// an export runs (`exporting`), another device's clock leads or the
    /// song has tempo points. With loops on audio tracks (phase 4d.2) the
    /// time engine follows the loops' clocks: a free cell whose loop
    /// became a song loop plays on as a song cell, and back. The line for
    /// the log (and the screen, when true), and the tempo the app takes
    /// from the next period on.
    pub fn tend_song(
        &mut self,
        time: &mut TimeHost,
        exporting: bool,
    ) -> (Option<(String, bool)>, Option<f64>) {
        if self.opening() {
            // Another project's loops: nothing to take back.
            self.song = SongStep::Idle;
            return (None, None);
        }
        match self.song {
            SongStep::Want => {
                self.song = SongStep::Idle;
                match self.start_song(time, exporting) {
                    Ok((line, bpm)) => (Some((line, false)), Some(bpm)),
                    Err(why) => {
                        self.songs_refused += 1;
                        (Some((format!("looper: Make song: {why}"), true)), None)
                    }
                }
            }
            SongStep::Unmake(p, b) => {
                self.song = SongStep::Idle;
                match self.unmake_song(time, p, b) {
                    Ok(line) => (Some((line, false)), Some(b.bpm)),
                    Err(why) => {
                        self.songs_refused += 1;
                        let line = format!("looper: UNDO can't take Make song back: {why}");
                        (Some((line, true)), None)
                    }
                }
            }
            _ => (None, None),
        }
    }

    fn start_song(
        &mut self,
        time: &mut TimeHost,
        exporting: bool,
    ) -> Result<(String, f64), String> {
        if exporting {
            return Err("an export is running".to_string());
        }
        if time.followed_bpm().is_some() || time.clock_waits() {
            return Err("the song follows another device's clock".to_string());
        }
        if time.tempo_map().segments().len() > 1 {
            return Err("the song has tempo points".to_string());
        }
        let p = self.looper.song_plan(time.next_frame())?;
        let info = time.info();
        let before = SongBefore {
            bpm: info.bpm,
            position: info.position,
            starts: p.at.is_some(),
        };
        let when = match p.at {
            Some(at) => {
                time.start_at(at, Some(p.bpm), None, 0)?;
                "on their next bar line, where the song starts"
            }
            None => {
                use mpfree_transport::Command as T;
                let at = time.next_frame();
                let moved = info.position == p.tick || time.command_at(at, T::Locate(p.tick));
                if !(moved && time.command_at(at, T::SetTempo(p.bpm))) {
                    return Err("the song's queue is full".to_string());
                }
                "now: PLAY starts them with the song, from their tops"
            }
        };
        // Worked out just now on the same clock: it goes in (if not, the
        // looper says why, take_song).
        self.looper.make_song(p);
        self.song = SongStep::Waits(p, before);
        let turns = if p.count() == 1 {
            "becomes a song loop"
        } else {
            "become song loops"
        };
        Ok((
            format!(
                "looper: Make song: {} {turns} {when}, at {:.2} BPM",
                loops_words(p.loops),
                p.bpm
            ),
            p.bpm,
        ))
    }

    fn unmake_song(
        &mut self,
        time: &mut TimeHost,
        p: SongPlan,
        b: SongBefore,
    ) -> Result<String, String> {
        use mpfree_transport::Command as T;
        self.looper.make_free(p)?;
        let at = time.next_frame();
        let info = time.info();
        let rolls = info.state.rolling();
        let mut sent = true;
        let stops = rolls && b.starts;
        if stops {
            sent &= time.command_at(at, T::Stop);
        }
        if rolls && !stops {
            // PLAY started it: it plays on, at its tempo.
            sent &= time.command_at(at, T::TempoNow(b.bpm));
        } else {
            if info.position != b.position {
                sent &= time.command_at(at, T::Locate(b.position));
            }
            sent &= time.command_at(at, T::SetTempo(b.bpm));
        }
        self.song = SongStep::Unmade(self.versions());
        Ok(format!(
            "looper: Make song taken back: {} free again, the song at {:.2} BPM{}{}",
            loops_words(p.loops) + if p.count() == 1 { " is" } else { " are" },
            b.bpm,
            if stops { ", stopped" } else { "" },
            if sent {
                ""
            } else {
                " (the song's queue was full)"
            }
        ))
    }

    /// What became of Make song in the period ([`Looper::take_song`]):
    /// made (UNDO takes it back), or why not. The line for the log and
    /// the screen.
    pub fn take_song(&mut self) -> Option<(String, bool)> {
        let d = self.looper.take_song()?;
        let SongStep::Waits(p, b) = self.song else {
            return d
                .refused
                .map(|why| (format!("looper: Make song didn't happen: {why}"), true));
        };
        match d.refused {
            None => {
                self.song = SongStep::Made(p, b, self.versions());
                // A new step on the one history, or the same one made
                // again by REDO.
                if !self.song_txn.is_some() {
                    self.song_txn = mpfree_transport::TransactionId::next();
                }
                let n = p.count();
                Some((
                    format!(
                        "looper: Made a song from {n} free loop{}: {:.2} BPM, {}/{}; UNDO takes it back",
                        if n == 1 { "" } else { "s" },
                        p.bpm,
                        p.meter.beats,
                        p.meter.unit
                    ),
                    true,
                ))
            }
            Some(why) => {
                self.song = SongStep::Idle;
                self.songs_refused += 1;
                Some((format!("looper: Make song didn't happen: {why}"), true))
            }
        }
    }
}
