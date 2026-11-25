//! The looper's side of the free loops: the free clock as the
//! loops share it, its runs, and Make song's plan.

use super::*;

/// Make song worked out ([`Looper::song_plan`]): every free loop with
/// sound becomes a song loop, in its place, and the song takes the free
/// clock's tempo. The host starts the song on `at` at `bpm` from `tick`
/// (or, with the free loops stopped, sets that tempo and that place at
/// once) and gives the plan back ([`Looper::make_song`]), so the loops
/// change clock on the same frame. Kept, it is what Undo needs
/// ([`Looper::make_free`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SongPlan {
    /// The frame the song starts on: the free clock's next bar line. None:
    /// the free loops are stopped, and they change clock at once (PLAY
    /// then starts the song and them together).
    pub at: Option<u64>,
    /// The song's tick on that frame (with the free loops stopped, where
    /// PLAY starts): the start of the bar the song is stopped in.
    pub tick: Tick,
    /// What every free loop's ticks move by: the song's tick less the free
    /// clock's on that frame (its bar 1 with the free loops stopped), whole
    /// bars.
    pub shift: Tick,
    /// The free clock's tempo and meter, which the song takes.
    pub bpm: f64,
    pub meter: Meter,
    /// The loops that change clock: bit `n` for loop `n`.
    pub loops: u32,
    /// The free clock as it was worked out on: its stamp (it lands only
    /// while the clock still has it) and the clock itself, whose bar 1 an
    /// Undo puts back.
    pub stamp: ClockStamp,
    pub clock: FreeClock,
}

impl SongPlan {
    /// How many loops it changes.
    pub fn count(&self) -> u32 {
        self.loops.count_ones()
    }
}

/// What became of a Make song the host gave ([`Looper::take_song`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SongDone {
    pub plan: SongPlan,
    /// None: the loops changed clock on its frame. Else why they didn't
    /// (the song didn't start there, a free loop started recording, the
    /// free clock stopped or started again), and nothing of the loops
    /// moved.
    pub refused: Option<&'static str>,
}

/// The looper's side of the free loops ([`Clock::Free`]).
#[derive(Clone, Copy, Debug)]
pub(crate) struct FreeTime {
    /// The free clock, while a free loop holds audio, and how many have
    /// been set: each free loop knows the one it was made on
    /// (`Loop::clock_gen`). The one that went with the last free loop
    /// cleared waits with its number, so that Undo can bring it back with
    /// that loop.
    pub(crate) clock: Option<FreeClock>,
    pub(crate) gen: u32,
    pub(crate) kept: Option<(FreeClock, u32)>,
    /// Play for the stopped free loops, pressed while the song plays: they
    /// start on the song's next bar line after song tick `.0` and not
    /// before `.1`, a beat on (NaN until the period's first span gives
    /// them: then the next line).
    pub(crate) cue: Option<(f64, f64)>,
    /// The frame the free clock last started on: a free loop's place jumps
    /// there (and crossfades, if it was still sounding).
    pub(crate) jump: u64,
    /// The frame the free clock's timing holds from: where it last
    /// started, or where a snap re-anchored it ([`FreeTime::stamp`]).
    pub(crate) since: u64,
    /// Once it has stopped: the clock as it ran, its number, and the frame
    /// it stopped on, so that Capture can still take the bars it ran
    /// ([`Looper::free_run`]) until it starts again or another is set.
    pub(crate) ran: Option<(FreeClock, u32, u64)>,
    /// Make song waiting for its frame ([`Looper::make_song`]), and what
    /// became of the last one, until the host takes it.
    pub(crate) song: Option<SongPlan>,
    pub(crate) song_done: Option<SongDone>,
}

/// The free loops with sound, as Make song finds them (bit `n` for loop
/// `n`), or why they can't change clock now: one is armed or recording,
/// loading or being copied into, or one with sound is busy
/// ([`Loop::clock_refusal`]); or there are none.
pub(crate) fn song_check(loops: &[Loop], prints: &[Prints]) -> Result<u32, &'static str> {
    let mut m = 0u32;
    for (k, l) in loops.iter().enumerate() {
        if l.clock != Clock::Free {
            continue;
        }
        if matches!(l.phase, Phase::Armed | Phase::Recording) {
            return Err("a free loop is recording");
        }
        if l.phase == Phase::Empty {
            if l.loading || l.adding || l.capturing() {
                return Err("a free loop is busy");
            }
            continue;
        }
        let pr = &prints[k];
        let print = pr.now.is_some() || matches!(pr.next, Some(Some(_)));
        if let Some(why) = l.clock_refusal(print) {
            return Err(why);
        }
        m |= 1 << k;
    }
    if m == 0 {
        return Err("there are no free loops");
    }
    Ok(m)
}

/// A run of the free clock, for a capture into a free loop
/// ([`Looper::free_run`]): the clock as it runs (or ran), the frame the
/// run began on ([`FreeTime::jump`]), and, once it has stopped, the frame
/// it stopped on.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FreeRun {
    pub(crate) clock: FreeClock,
    pub(crate) since: u64,
    pub(crate) until: Option<u64>,
}

impl FreeRun {
    /// The frame its bar lines are heard up to, at absolute frame `at`.
    fn end(&self, at: u64) -> u64 {
        self.until.map_or(at, |u| at.min(u))
    }

    /// Its newest bar line heard by absolute frame `at`, as its tick and
    /// frame.
    pub(crate) fn last_line(&self, at: u64) -> Option<(Tick, u64)> {
        self.clock.line_before(self.end(at), self.since)
    }

    /// While it runs and absolute frame `at` is in a bar's last beat: the
    /// bar line coming, which a capture then waits for.
    pub(crate) fn coming_line(&self, at: u64) -> Option<Tick> {
        self.until
            .is_none()
            .then(|| self.clock.last_beat_line(at))?
    }

    /// The frame of its bar line on tick `t`, if `t` is one (a multiple of
    /// a bar) heard in this run by absolute frame `at`.
    pub(crate) fn line_heard(&self, t: Tick, at: u64) -> Option<u64> {
        let bar = self.clock.meter.bar_ticks().max(1);
        self.clock
            .frame_of(t)
            .filter(|&f| t.rem_euclid(bar) == 0 && f >= self.since && f < self.end(at))
    }
}

impl FreeTime {
    /// The free clock as it is now ([`ClockStamp`]).
    pub(crate) fn stamp(&self) -> ClockStamp {
        ClockStamp {
            clock: Clock::Free,
            gen: self.gen,
            since: self.since,
        }
    }

    /// Whether the free clock runs on absolute frame `f`.
    pub(crate) fn rolls_at(&self, f: u64) -> bool {
        self.clock.is_some_and(|c| c.rolls_at(f))
    }

    /// The stopped free clock starts on frame `f` from its top, and the
    /// free loops with it.
    pub(crate) fn start(&mut self, f: u64) {
        self.cue = None;
        if let Some(c) = self.clock.as_mut().filter(|c| !c.running) {
            c.start_at(f);
            self.jump = f;
            self.since = f;
        }
    }

    /// A free loop's first pass begins on the song's bar line `line` (its
    /// tick and frame) in `span`: the free clock is set there as a copy of
    /// the song's (with none yet: a new one), or the stopped one starts
    /// there from its top, keeping its tempo (the stopped free loops come
    /// in with it). Gives the clock, and the line on it.
    pub(crate) fn set_on_song(
        &mut self,
        span: &Span,
        line: (Tick, u64),
    ) -> (FreeClock, (Tick, u64)) {
        self.cue = None;
        let c = match self.clock.as_mut() {
            Some(c) => {
                c.start_at(line.1);
                *c
            }
            None => {
                self.gen = self.gen.wrapping_add(1);
                *self.clock.insert(FreeClock::of_song(span, line.0))
            }
        };
        self.jump = line.1;
        self.since = line.1;
        (c, (c.top, line.1))
    }

    /// The free clock stops on frame `f` and goes back to its bar 1 (STOP
    /// with the song stopped, or the free loops' own Stop). Each free loop
    /// on it hears it stop there as a song loop hears STOP: a first pass
    /// keeps what was played (as far as the next bar line), an overdub
    /// ends, and they fade out. A free loop armed to start on a bar line
    /// lets go (nothing plays to start it), one waiting for your note
    /// listens again, and a free first pass in your own time takes it as
    /// its closing tap: the free clock it sets starts stopped.
    pub(crate) fn stop(
        &mut self,
        loops: &mut [Loop],
        store: &mut Store,
        stats: &mut Stats,
        f: u64,
    ) {
        self.cue = None;
        let rolling = self.rolls_at(f);
        let tick = self.clock.map_or(0.0, |c| c.tick_at(f)).floor() as Tick;
        for (k, l) in loops.iter_mut().enumerate() {
            if l.clock != Clock::Free {
                continue;
            }
            match l.phase {
                Phase::Armed if l.starts == Starts::Bar || l.resample => l.disarm(),
                Phase::Armed => l.unhear(),
                Phase::Recording if l.free => {
                    l.ending = true;
                    l.stop_close = true;
                }
                _ if rolling => l.marker(store, stats, k, Mark::Stopped { tick }, f),
                _ => {}
            }
        }
        if let Some(c) = self.clock.as_mut() {
            // Capture can still take the bars it ran.
            self.ran = rolling.then_some((*c, self.gen, f));
            c.stop();
        }
    }

    /// A free loop's snap due by frame `f` (a run's first: runs are cut on
    /// it) lands there, and the free clock takes its tempo there, anchored
    /// on the loop's downbeat, which is its bar 1 from now on (the loop is
    /// the only free loop with sound, [`Looper::snap_plan`]): so the loop
    /// plays on in its place, as a song loop does when the song's tempo
    /// changes with it. With the free clock not running there, it doesn't
    /// land.
    pub(crate) fn land_snaps(&mut self, loops: &mut [Loop], stats: &mut Stats, f: u64) {
        for l in loops.iter_mut() {
            if l.clock != Clock::Free || l.snap_frame > f {
                continue;
            }
            // Worked out on the clock as it runs now, or no downbeat.
            if !self.rolls_at(f) || l.snap.is_none_or(|p| p.stamp != self.stamp()) {
                l.drop_snap(stats);
                continue;
            }
            l.land_snap(stats);
            let Some(p) = l.snap_done.filter(|d| d.landed).map(|d| d.plan) else {
                continue;
            };
            // (Anchored there, it places no bar line before it: Capture
            // takes no bars at the old tempo.)
            if let Some(c) = self.clock.as_mut() {
                c.anchor_frame = f;
                c.anchor_tick = p.tick;
                c.bpm = p.bpm;
                c.top = p.tick;
                // Its timing holds from here: bars heard before it were
                // at the old tempo.
                self.since = f;
            }
        }
    }

    /// Make song waiting for frame `f` or before (a run's first: runs are
    /// cut on it) lands there: each free loop it names becomes a song loop
    /// in its place, as long as the song started on that frame from the
    /// plan's tick (with the free loops stopped: the song is still
    /// stopped), the free clock is still the one it was worked out on, and
    /// the free loops are still as they were. Said either way
    /// ([`Looper::take_song`]). The free clock goes once no free loop
    /// holds it ([`Looper::tend_free`]).
    #[cold]
    #[inline(never)]
    pub(crate) fn land_song(
        &mut self,
        loops: &mut [Loop],
        prints: &[Prints],
        stats: &mut Stats,
        span: &Span,
        f: u64,
    ) {
        let Some(p) = self.song.take() else {
            return;
        };
        let started = match p.at {
            Some(_) => {
                span.state.rolling()
                    && span.tick_at(f) == p.tick as f64
                    && span.bpm == p.bpm
                    && self.rolls_at(f)
            }
            None => !span.state.rolling() && !self.rolls_at(f),
        };
        let refused = if p.stamp != self.stamp() {
            Some("the free clock started again")
        } else if p.at.is_some() != self.rolls_at(f) {
            Some(if p.at.is_some() {
                "the free loops stopped"
            } else {
                "the free loops started"
            })
        } else if !started {
            Some(if p.at.is_some() {
                "the song didn't start on the free loops' bar line at their tempo"
            } else {
                "the song started"
            })
        } else if span.meter != p.meter {
            Some("the song's meter changed")
        } else {
            match song_check(loops, prints) {
                Ok(m) if m == p.loops => None,
                Ok(_) => Some("the free loops changed"),
                Err(why) => Some(why),
            }
        };
        if refused.is_none() {
            for (k, l) in loops.iter_mut().enumerate() {
                if p.loops & (1 << k) != 0 {
                    l.change_clock(Clock::Song, p.shift);
                }
            }
            self.cue = None;
        } else {
            stats.refused += 1;
        }
        self.song_done = Some(SongDone { plan: p, refused });
    }

    /// What the song's marks do to the free loops, which hear only the
    /// free clock: STOP with the song stopped stops it
    /// ([`FreeTime::stop`]); PLAY starts it, stopped, from its top; the
    /// song stopping lets go of a free loop armed to start on the song's
    /// bar line (one waiting for your note listens on) and starts the free
    /// loops a Play waits to start; a locate or the song's own loop lets
    /// that Play start on the next bar line.
    pub(crate) fn mark(
        &mut self,
        loops: &mut [Loop],
        store: &mut Store,
        stats: &mut Stats,
        mark: Mark,
        f: u64,
    ) {
        match mark {
            Mark::Panic => self.stop(loops, store, stats, f),
            Mark::Started { .. } => self.start(f),
            Mark::Stopped { .. } => {
                let rolling = self.rolls_at(f);
                for l in loops.iter_mut() {
                    if l.clock == Clock::Free
                        && l.phase == Phase::Armed
                        && !rolling
                        && (l.starts == Starts::Bar || l.resample)
                    {
                        l.disarm();
                    }
                }
                if self.cue.is_some() {
                    self.start(f);
                }
            }
            Mark::Located { .. } | Mark::Looped { .. } => {
                if let Some(cue) = self.cue.as_mut() {
                    *cue = (f64::MIN, f64::MIN);
                }
            }
            _ => {}
        }
    }
}
