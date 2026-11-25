//! The looper: its loops, its memory and the commands it takes, and its
//! controls (the rest of its methods, by job, in the files under
//! `looper/`).

mod apply;
mod capture;
mod copy;
mod files;
mod free;
mod layers;
mod period;
mod smart_end;

use super::*;

/// The looper: its loops, their shared memory and the command queue.
pub struct Looper {
    pub(crate) store: Store,
    capacity: usize,
    /// Listen: a loop (or one of its layers) played alone while the song
    /// is stopped ([`Looper::listen`]).
    listen: Option<Listen>,
    pub(crate) loops: Vec<Loop>,
    selected: usize,
    queue: [Queued; QUEUE],
    queued: usize,
    stats: Stats,
    /// The period's first frame.
    now: u64,
    bounce: Option<Bouncing>,
    prints: Vec<Prints>,
    /// Each loop's saved loop being loaded, or loaded and waiting for
    /// [`Looper::take_loaded`].
    loads: Vec<Option<LoopFile>>,
    /// The last [`HEARD_SECONDS`] of input and its bar lines, for Capture.
    pub(crate) heard: Heard,
    rate: u32,
    /// Tempo from the first loop ([`Command::TempoFromLoop`]).
    free_tempo: bool,
    /// The song's tempo was set by hand ([`Command::TempoMeant`]).
    tempo_meant: bool,
    /// The song was stopped at the period's start, and its bar's ticks
    /// then.
    stopped: bool,
    bar_ticks: Tick,
    /// A free first pass closed: its loop, its reading, the frame the
    /// song starts on and the ticks into the loop it starts at
    /// ([`Looper::take_tempo`]).
    tempo_out: Option<(usize, Reading, u64, Tick)>,
    /// A free first pass may be recording: [`Looper::close_free`] looks
    /// at the loops only then, not every period.
    free_open: bool,
    /// Smart loop end ([`Command::SetSmartEnd`]), the notes it weighs and
    /// what it made of the last tap, for the host.
    smart: SmartEnd,
    notes: NoteLog,
    smart_out: Option<SmartNote>,
    /// Smart loop end listens ([`Command::SmartByEar`]): the hit track,
    /// the hits it heard, and the finder's ask under way.
    by_ear: bool,
    track: HitTrack,
    hits: HitLog,
    find: Option<Finding>,
    find_gen: u64,
    /// The last period's last span: where the song goes from here, for a
    /// snap's downbeat ([`Looper::snap_plan`]).
    last_span: Option<Span>,
    /// The free clock and what waits on it ([`FreeTime`]).
    free_time: FreeTime,
    /// The free clock's ticks for a run (as `process` works out the
    /// song's), kept here so that a period without free loops doesn't
    /// clear them.
    free_ticks: [f64; RUN],
    /// What each loop and Listen played in the last period, apart
    /// ([`Looper::process_each`]).
    outs: LoopOuts,
    /// The loops on audio tracks ([`Looper::play`]): where the track
    /// player says each sounds next. None: the loop launches itself.
    on_track: [Option<OnTrack>; MAX_LOOPS],
    /// The free loops whose cells are on audio tracks, a bit a loop
    /// ([`Looper::free_on_track`]): each launches itself on the free clock
    /// as ever, but its buffer is for its track.
    free_tracked: u32,
}

impl Looper {
    /// A looper with room for `seconds` of stereo audio at `rate`, shared
    /// by every loop, its first passes and its overdubs, and a little more:
    /// memory comes in blocks of [`BLOCK`] frames, and one block per layer
    /// of a loop is added so that whole layers fill what the seconds
    /// promise. [`Looper::memory`] gives the exact size. Allocates; call
    /// it before the sound starts.
    pub fn new(rate: u32, seconds: u32) -> Looper {
        let n_blocks = n_blocks(rate, seconds);
        Looper {
            store: Store::new(n_blocks),
            capacity: n_blocks * BLOCK,
            listen: None,
            loops: vec![Loop::NEW; MAX_LOOPS],
            selected: 0,
            queue: [Queued::Loop(Command::Clear); QUEUE],
            queued: 0,
            stats: Stats::default(),
            now: 0,
            bounce: None,
            prints: (0..MAX_LOOPS).map(|_| Prints::default()).collect(),
            loads: (0..MAX_LOOPS).map(|_| None).collect(),
            heard: Heard::new(heard_frames(rate, seconds)),
            rate,
            free_tempo: false,
            tempo_meant: false,
            stopped: true,
            bar_ticks: 4 * PPQ as Tick,
            tempo_out: None,
            free_open: false,
            smart: SmartEnd::Off,
            notes: NoteLog::new(),
            by_ear: false,
            track: HitTrack::new(rate),
            hits: HitLog::new(),
            find: None,
            find_gen: 0,
            smart_out: None,
            last_span: None,
            free_time: FreeTime {
                clock: None,
                gen: 0,
                kept: None,
                cue: None,
                jump: u64::MAX,
                since: u64::MAX,
                ran: None,
                song: None,
                song_done: None,
            },
            free_ticks: [0.0; RUN],
            outs: LoopOuts::new(MAX_PERIOD as usize),
            on_track: [None; MAX_LOOPS],
            free_tracked: 0,
        }
    }

    /// Queue a command for the selected loop for the next period. False
    /// (and counted) if the queue is full.
    pub fn command(&mut self, c: Command) -> bool {
        self.push(Queued::Loop(c))
    }

    /// Queue a command for layer `id` ([`LayerInfo::id`]) of the selected
    /// loop for the next period. False (and counted) if the queue is full.
    pub fn layer_command(&mut self, id: u8, c: LayerCommand) -> bool {
        self.push(Queued::Layer(id, c))
    }

    /// Listen: play loop `n` alone from its start, round and round, while
    /// the song is stopped: the whole loop as heard (its layers' levels,
    /// mutes and solos), or with `layer` ([`LayerInfo::id`]) that layer on
    /// its own, even when muted. It plays at the speed it was recorded,
    /// through the loop's level. It stops with [`Looper::stop_listening`],
    /// or by itself when the song starts or the loop records or empties
    /// (a free loop's: when the free clock starts). Refused (false) while
    /// the song plays, for a free loop while the free clock runs (it plays
    /// anyway), or for a loop with nothing to play.
    pub fn listen(&mut self, n: usize, layer: Option<u8>) -> bool {
        let Some(l) = self.loops.get(n) else {
            return false;
        };
        let playable = matches!(l.phase, Phase::Playing)
            && l.len > 0
            && !l.merging
            && layer.is_none_or(|id| l.order[..l.in_order].contains(&id));
        // (A free loop plays by itself while the free clock runs.)
        let free_plays = l.clock == Clock::Free && self.free_time.clock.is_some_and(|c| c.running);
        if !self.stopped || !playable || free_plays {
            return false;
        }
        self.listen = Some(Listen {
            n,
            layer,
            pos: 0,
            gain: 0.0,
            stopping: false,
        });
        true
    }

    /// Listen fades out and stops.
    pub fn stop_listening(&mut self) {
        if let Some(l) = &mut self.listen {
            l.stopping = true;
        }
    }

    /// What Listen plays: the loop, the layer (None: all of it as heard)
    /// and where it has got to, 0 to 1.
    pub fn listening(&self) -> Option<(usize, Option<u8>, f32)> {
        let l = self.listen.filter(|l| !l.stopping)?;
        let len = self.loops[l.n].len.max(1);
        Some((l.n, l.layer, l.pos as f32 / len as f32))
    }

    /// One period of Listen, into its own buffer of `outs` (n frames,
    /// interleaved).
    fn play_listen(&mut self, block: &Block, n: usize, outs: &mut LoopOuts) {
        let Some(mut ls) = self.listen else {
            return;
        };
        let l = &self.loops[ls.n];
        let stopped = block.spans().iter().all(|s| s.state == State::Stopped)
            && !(l.clock == Clock::Free && self.free_time.clock.is_some_and(|c| c.running));
        let still = l.phase == Phase::Playing
            && l.len > 0
            && !l.merging
            && ls
                .layer
                .is_none_or(|id| l.order[..l.in_order].contains(&id));
        if !still {
            // The loop changed under it: stop at once (it is silent or
            // recording anyway).
            self.listen = None;
            return;
        }
        if !stopped {
            ls.stopping = true;
        }
        let gains = l.listen_gains(ls.layer);
        let replacing = ls.layer.is_none() && l.replacing();
        let level = l.level;
        let len = l.len;
        let step = 1.0 / FADE as f32;
        let out = outs.listen_mut(ls.n);
        let mut f = 0;
        while f < n {
            let k = RUN.min(n - f).min(len - ls.pos);
            let mut acc = [[0.0f32; 2]; RUN];
            l.render_layers_at(&self.store, ls.n, ls.pos, &mut acc[..k], &gains, replacing);
            for (j, a) in acc[..k].iter().enumerate() {
                ls.gain = if ls.stopping {
                    (ls.gain - step).max(0.0)
                } else {
                    (ls.gain + step).min(1.0)
                };
                let g = ls.gain * level;
                out[2 * (f + j)] += a[0] * g;
                out[2 * (f + j) + 1] += a[1] * g;
            }
            f += k;
            ls.pos = (ls.pos + k) % len;
            if ls.stopping && ls.gain == 0.0 {
                self.listen = None;
                return;
            }
        }
        self.listen = Some(ls);
    }

    /// Select loop `n` (0 to [`MAX_LOOPS`] - 1) for the commands queued
    /// after this. False (and counted) if the queue is full.
    pub fn select_loop(&mut self, n: usize) -> bool {
        self.push(Queued::Select(n))
    }

    /// Copy layer `id` ([`LayerInfo::id`]) of loop `from` onto loop `to`
    /// for the next period; `keep` false moves it: once the copy plays it
    /// leaves `from` as a delete, unless it is that loop's last layer.
    /// The layer goes as it plays, through `from`'s edits, at its level,
    /// pan, mute and Fade old turn-down (not its solo). Loop `to` must be
    /// empty, and then takes `from`'s length and start, or play a loop
    /// of the same length in ticks: the layer goes in time with the song,
    /// made to that loop's length in frames. Both loops must be playing
    /// with nothing changing their layers, and `to` needs a layer slot
    /// and the memory. It copies at [`COPY_SPEED`] while both play on;
    /// meanwhile the commands that would change either loop's layers
    /// wait, except Undo on `to` and Clear on either, which call it off.
    /// Refused (counted) otherwise. False (and counted) if the queue is
    /// full.
    pub fn copy_layer(&mut self, from: usize, id: u8, to: usize, keep: bool) -> bool {
        self.push(Queued::Copy { from, id, to, keep })
    }

    /// Launch loop `n` (`on`) or stop it, as a clip launches: from the
    /// bar line at song tick `at` (the frame it falls on), or with `None`
    /// at once. Either way it fades in or out as a mute does, and an off
    /// loop keeps its place in the song, so it comes back in step. A new
    /// launch replaces one waiting; `on` with nothing waiting and the loop
    /// already on changes nothing. Recording a first pass turns a loop on.
    /// It doesn't wait for the selected loop's fades. False (and counted)
    /// if the queue is full.
    pub fn launch(&mut self, n: usize, on: bool, at: Option<Tick>) -> bool {
        self.push(Queued::Launch { n, on, at })
    }

    /// Loop `n` plays on an audio track (audio tracks phase 4, step 4b):
    /// `runs` are the frames of the next period it sounds on, in order,
    /// each at its place counted from its origin (`on_track`'s module
    /// says how). The first call puts the loop on the track: from then
    /// the track player says where it sounds, a period at a time (no call,
    /// or no runs, is silence, and a loop told nothing isn't run), and the
    /// loop's own launch, mute and solo are refused. Call it before
    /// [`Looper::process_each`] each period; its own buffer
    /// ([`LoopOuts::loop_out`]) then holds it, at full gain, for the
    /// track to fade and mix. False (counted as refused, and the runs told
    /// before kept) if `n` is past the loops or the runs are more than
    /// [`MAX_PLAY_RUNS`], empty, out of order or overlapping.
    ///
    /// A track places only a song loop: its runs are the song's frames
    /// and ticks. A free loop ([`Clock::Free`]) keeps its own clock, so it
    /// is refused too (counted, and not put on the track), and a loop on a
    /// track refuses Sync: Free ([`Command::SetClock`]).
    pub fn play(&mut self, n: usize, runs: &[PlayRun]) -> bool {
        let Some(l) = self.loops.get_mut(n).filter(|l| l.clock == Clock::Song) else {
            self.stats.refused += 1;
            return false;
        };
        // A free cell's loop made a song loop (Make song) played itself
        // until now: the track takes it over here, with no gap. It played
        // on to the next period's first frame from its own origin, so runs
        // from that origin go straight on, and one a hair off crossfades.
        let next = self.now + self.outs.frames() as u64;
        let seed = (self.free_tracked & (1 << n) != 0).then_some((next, l.origin as f64));
        self.free_tracked &= !(1 << n);
        let t = self.on_track[n].get_or_insert_with(|| {
            // The track fades it: it is on, unmuted and not soloed.
            l.on = true;
            l.cue = None;
            l.muted = false;
            l.solo = false;
            let mut t = OnTrack::default();
            t.last = seed;
            t
        });
        if !t.tell(runs) {
            self.stats.refused += 1;
            return false;
        }
        true
    }

    /// Loop `n` launches itself again, as it did before it went on a
    /// track ([`Looper::play`]) or while its cell was on one
    /// ([`Looper::free_on_track`]): from the next period, in the looper's
    /// mix again.
    pub fn leave_track(&mut self, n: usize) {
        if let Some(t) = self.on_track.get_mut(n) {
            *t = None;
        }
        if n < MAX_LOOPS {
            self.free_tracked &= !(1 << n);
        }
    }

    /// Free loop `n`'s cell is on an audio track (`on`), or no longer is
    /// (audio tracks phase 4e.4, free cells). On one, it launches itself on
    /// the free clock as ever ([`Looper::launch`]; `Some(Tick::MIN)` lands
    /// on the free clock's next bar line) and fades in and out with its
    /// launches, but its buffer ([`LoopOuts::loop_out`]) is for its track:
    /// the looper's mix leaves it out, it doesn't resample, it is out of
    /// the loops' solo, and its own Mute, Solo and Sync are refused (its
    /// track mutes and solos it). Putting it on unmutes and unsolos it.
    /// It stays on through a Clear until it is taken off here, with
    /// [`Looper::leave_track`], or by the track player's first runs for it
    /// ([`Looper::play`]): when Make song makes it a song loop it plays
    /// itself on, still out of the mix, until its track places it, with no
    /// gap. Make song undone, the loop is free again and is put back on
    /// here. False, and counted as refused, for a song loop (`on`) or a
    /// loop past the loops.
    pub fn free_on_track(&mut self, n: usize, on: bool) -> bool {
        let Some(l) = self.loops.get_mut(n) else {
            self.stats.refused += 1;
            return false;
        };
        if !on {
            self.free_tracked &= !(1 << n);
            return true;
        }
        if l.clock != Clock::Free {
            self.stats.refused += 1;
            return false;
        }
        if self.free_tracked & (1 << n) == 0 {
            l.muted = false;
            l.solo = false;
        }
        self.on_track[n] = None;
        self.free_tracked |= 1 << n;
        true
    }

    /// Free loop `n`'s cell is on an audio track ([`Looper::free_on_track`]).
    pub fn free_tracked(&self, n: usize) -> bool {
        n < MAX_LOOPS && self.free_tracked & (1 << n) != 0
    }

    /// Loop `n` is on an audio track ([`Looper::play`]).
    pub fn on_track(&self, n: usize) -> bool {
        self.on_track.get(n).is_some_and(|t| t.is_some())
    }

    /// The loop the commands act on (as of the last period).
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Room left in the command queue this period.
    pub fn room(&self) -> usize {
        QUEUE - self.queued
    }

    fn push(&mut self, q: Queued) -> bool {
        if self.queued == QUEUE {
            self.stats.dropped_commands += 1;
            return false;
        }
        self.queue[self.queued] = q;
        self.queued += 1;
        true
    }

    pub fn stats(&self) -> Stats {
        Stats {
            handoffs: self.loops.iter().map(|l| l.handoffs).sum(),
            hits_heard: self.track.found,
            ..self.stats
        }
    }

    /// The selected loop.
    pub fn status(&self) -> Status {
        self.loop_status(self.selected)
            .expect("the selected loop is there")
    }

    /// Loop `n`, or None past [`MAX_LOOPS`].
    pub fn loop_status(&self, n: usize) -> Option<Status> {
        let l = self.loops.get(n)?;
        Some(Status {
            phase: l.phase,
            layers: l.layers(),
            redo: l.n_undone,
            bars: l.bars,
            frames: l.len,
            ticks: l.len_ticks,
            origin: l.origin,
            position: if l.len > 0 {
                (l.pos / l.len as f64) as f32
            } else {
                0.0
            },
            ending: l.ending,
            overdub_next: l.then_overdub || l.fit_overdub || l.print_wait,
            after: l.after,
            fade_old: l.fade_old,
            latency: l.latency,
            level: l.level,
            muted: l.muted,
            on: l.on,
            cue: l.cue,
            seam: l.seam,
            merging: l.merging.then(|| l.mg_done as f32 / l.len.max(1) as f32),
            resampling: l.resample && (l.first_in_progress() || l.phase == Phase::Armed),
            capacity: self.capacity,
            used: (self.store.blocks(n) - self.store.kept_blocks_of(n)) * BLOCK,
            version: l.version,
            printed: l.print_on > 0.0,
            fitting: l
                .fitting
                .then(|| l.fit_done as f32 / l.fit_frames.max(1) as f32),
            loading: (l.loading && l.ld_heard.is_none())
                .then(|| l.ld_done as f32 / l.ld_total.max(1) as f32),
            copying: l
                .adding
                .then(|| l.ad_done as f32 / l.ad_frames.max(1) as f32),
            recorded: if l.phase == Phase::Recording {
                l.written
            } else {
                0
            },
            // (Not once an edit has made it longer than the pass was.)
            cut: (l.len > 0 && l.cut < l.len && l.cut < l.captured.1 - l.captured.0)
                .then_some(l.cut),
            starts_on: (l.phase == Phase::Armed)
                .then_some(l.tap_lead)
                .flatten()
                .map(|(tap, until)| {
                    // (A free loop on the free clock counts its bars.)
                    let bar = match self.free_time.clock {
                        Some(c) if l.on_free(c.rolls_at(self.now)) => c.meter.bar_ticks(),
                        _ => self.bar_ticks,
                    };
                    let bar = bar.max(1) as f64;
                    let line = (tap / bar).ceil() * bar;
                    (if line > tap && line < until {
                        (until / bar).ceil() * bar
                    } else {
                        line
                    }) as Tick
                }),
            kept: self.store.has_kept(n),
            solo: l.solo,
            solo_out: l.solo_out,
            overdubs: l.overdubs,
            speed: l.speed,
            rate: l.rate,
            capturing: if l.capture.is_some() {
                Some(0.0)
            } else {
                (l.loading && l.ld_heard.is_some())
                    .then(|| l.ld_done as f32 / l.ld_frames.max(1) as f32)
            },
            starts: l.starts,
            clock: l.clock,
            listening: l.listening(),
            lead: l.bounds_now().map_or(l.lead, |(start, src)| src - start),
            free: l.free
                || ((l.phase == Phase::Empty || l.listening()) && self.stopped && self.frees(n)),
            bounds: l.bounds_now().map(|(start, src)| Bounds {
                src: src as i64,
                start: start as i64,
                captured: (l.captured.0 as i64, l.captured.1 as i64),
                kept: l.slots[0].valid.1 as i64,
                short: l.short,
            }),
            closing: l.closing,
            offer: l
                .smart_now()
                .and_then(|(f, m)| f.map(|f| f - m))
                .filter(|o| !o.is_none()),
            moved: l.smart_now().map(|(_, m)| m).filter(|m| !m.is_none()),
            found: l.smart_now().and_then(|(f, _)| f),
            snapping: l.snap.map(|p| p.at),
            asks: l.asks,
            ab: l.ab_on.then_some(l.ab_other),
        })
    }
}
