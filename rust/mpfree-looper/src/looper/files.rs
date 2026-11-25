//! Saving, opening, printing and bouncing loops, and the takes
//! Capture keeps.

use super::*;

impl Looper {
    /// Hand loop `n` a print ([`Print`]) for the tempo it should play at,
    /// or `None` to take its print away. It takes over once the print
    /// playing has faded out (at most [`FADE`] frames), and plays while
    /// the loop is at the print's version and length in ticks. The print
    /// it replaces waits in [`Looper::take_print`], so that it is not
    /// freed here. Does not allocate. Gives `p` back when another print is
    /// still waiting to take over, the loop number is past [`MAX_LOOPS`],
    /// or the print is empty or shorter than its frames.
    pub fn print(&mut self, n: usize, p: Option<Print>) -> Result<(), Option<Print>> {
        let Some(slot) = self.prints.get_mut(n) else {
            return Err(p);
        };
        let ok = p
            .as_ref()
            .is_none_or(|p| p.frames > 0 && p.audio.len() >= 2 * p.frames && p.ticks > 0);
        if !ok || slot.next.is_some() {
            return Err(p);
        }
        slot.next = Some(p);
        Ok(())
    }

    /// Loop `n` as it would be saved, without its layers' audio (read it
    /// with [`Looper::read_layer`]) and with no file names. None unless it
    /// is playing with nothing changing its layers: no overdub, first pass
    /// still being written, merge, fit or load. Allocates (the list of
    /// layers): not for the audio thread.
    pub fn loop_file(&self, n: usize) -> Option<LoopFile> {
        let l = self.loops.get(n)?;
        if !l.savable() {
            return None;
        }
        let layers = (0..l.layers())
            .filter_map(|k| l.listed(k))
            .map(|slot| {
                let s = &l.slots[slot];
                LayerFile {
                    level: s.level,
                    pan: s.pan,
                    muted: s.mute,
                    solo: s.solo,
                    faded: s.age,
                    file: String::new(),
                    audio: Vec::new(),
                    raw: None,
                }
            })
            .collect();
        Some(LoopFile {
            frames: l.len,
            ticks: l.len_ticks,
            origin: l.origin,
            level: l.level,
            muted: l.muted,
            bars: l.bars,
            after: l.after,
            starts: l.starts,
            fade_old: l.fade_old,
            speed: l.speed,
            rate: l.rate,
            id: 0,
            clock: l.clock,
            layers,
            raw: None,
        })
    }

    /// Loop `n` as the looper holds it (format 2, audio tracks phase 4c),
    /// to save it so that it reopens exactly: [`Looper::loop_file`] with
    /// its shape ([`save::RawShape`]) and each layer's view of its audio
    /// ([`save::RawLayer`]), without the audio (read it with
    /// [`Looper::raw_layer`]). None when it couldn't be saved now.
    /// Allocates: not for the audio thread.
    pub fn loop_file_raw(&self, n: usize) -> Option<LoopFile> {
        let mut f = self.loop_file(n)?;
        let l = &self.loops[n];
        // The first pass's region and capture are in slot 0's audio,
        // which opens as layer 0: kept only while that is its first layer.
        let first = l.listed(0) == Some(0);
        f.raw = Some(save::RawShape {
            seam: l.seam,
            region: l.region.filter(|_| first),
            captured: if first { l.captured } else { (0, 0) },
            cut: l.cut,
        });
        for (k, y) in f.layers.iter_mut().enumerate() {
            let slot = l.listed(k)?;
            let (s, v) = (&l.slots[slot], &l.views[slot]);
            y.raw = Some(save::RawLayer {
                size: v.size(),
                valid: s.valid,
                pieces: v
                    .pieces()
                    .iter()
                    .map(|p| (p.start, p.len, p.back))
                    .collect(),
                replace: s.replace,
                rec: 0,
                from: Vec::new(),
            });
        }
        Some(f)
    }

    /// Frames `from` on of layer `k` of loop `n`'s audio (in play order)
    /// as it was written, not as it plays: its view's `size` frames, which
    /// [`Looper::loop_file_raw`] gives (audio tracks phase 4c). Frames it
    /// never wrote, and any past its audio, are silence. Into `out`,
    /// interleaved stereo; gives the loop's version ([`Status::version`]),
    /// or None, writing nothing, when the loop couldn't be saved now or
    /// has no such layer. Does not allocate.
    pub fn raw_layer(&self, n: usize, k: usize, from: usize, out: &mut [f32]) -> Option<u64> {
        let l = self.loops.get(n)?;
        if !l.savable() {
            return None;
        }
        let slot = l.listed(k)?;
        self.raw_slot(n, slot, from, out);
        Some(l.version)
    }

    /// Frames `from` on of loop `n`'s layer numbered `born`
    /// ([`LayerInfo::born`]) as it was written, as [`Looper::raw_layer`]
    /// gives them, once the layer is closed ([`LayerInfo::closed`]) and
    /// whatever else the loop is doing: a closed layer's audio never
    /// changes, so phase 4c.3 writes each to its file while the loop
    /// plays, overdubs or is edited. Gives the layer's size, or None,
    /// writing nothing, when the loop doesn't list that layer or it is
    /// still being written. Does not allocate.
    pub fn raw_of(&self, n: usize, born: u64, from: usize, out: &mut [f32]) -> Option<usize> {
        let l = self.loops.get(n)?;
        let slot = l.slot_of(born).filter(|&s| l.slot_closed(s))?;
        self.raw_slot(n, slot, from, out);
        Some(l.views[slot].size())
    }

    /// Slot `slot` of loop `n`'s audio from frame `from`, as written (see
    /// [`Looper::raw_layer`]).
    fn raw_slot(&self, n: usize, slot: usize, from: usize, out: &mut [f32]) {
        let l = &self.loops[n];
        let blocks = self.store.blocks_of(n, slot);
        let size = l.views[slot].size();
        let (start, count) = l.slots[slot].valid;
        for (i, y) in out.chunks_exact_mut(2).enumerate() {
            let a = from + i;
            // Frames from the first one written, as playback counts them.
            let written = a < size && (count >= size || (a + size - start % size) % size < count);
            let x = if written {
                self.store.frame(blocks, a / BLOCK, a % BLOCK)
            } else {
                [0.0; 2]
            };
            y.copy_from_slice(&x);
        }
    }

    /// Frames `from` on of layer `k` of loop `n` (in play order) as it
    /// plays, through the loop's edits and their seam fades, at gain 1
    /// (silent where a Replace layer after it covers it, as heard),
    /// into `out` (interleaved stereo, wrapping round the loop). Gives the
    /// loop's version ([`Status::version`]), so that a save read across
    /// several periods can tell that the layers changed; None, writing
    /// nothing, when the loop could not be saved ([`Looper::loop_file`])
    /// or has no such layer. Does not allocate.
    pub fn read_layer(&self, n: usize, k: usize, from: usize, out: &mut [f32]) -> Option<u64> {
        let l = self.loops.get(n)?;
        if !l.savable() {
            return None;
        }
        let slot = l.listed(k)?;
        let blocks = self.store.blocks_of(n, slot);
        let (valid, view) = (l.slots[slot].valid, &l.views[slot]);
        // Under a Replace layer it is saved as heard: silent where that
        // covers it, so the loop loads sounding the same.
        let replacing = l.replacing();
        let any_solo = l.any_solo();
        let strength = |s: &Slot| if s.heard(any_solo) { 1.0 } else { 0.0 };
        for (i, y) in out.chunks_exact_mut(2).enumerate() {
            let p = (from + i) % l.len;
            let mut x = layer_frame(&self.store, blocks, valid, view, l.seam, p);
            if replacing {
                let d = l.ducks(&l.views, l.len, p, None, strength)[slot];
                x = x.map(|v| v * d);
            }
            y.copy_from_slice(&x);
        }
        Some(l.version)
    }

    /// Put a saved loop back into loop `n`, which must be empty: its
    /// settings now, and its layers once their audio has copied in (at
    /// [`LOAD_SPEED`]); then it fades in, in time with the song, from
    /// `origin` (a free loop: in time with the free clock, which must be
    /// open, [`Looper::open_free_clock`]). Takes its memory now. Its commands wait while it copies,
    /// except Clear (which calls it off), Mute and the settings. The
    /// [`LoopFile`] waits in [`Looper::take_loaded`] once done, so that
    /// its audio is not freed here. Does not allocate. Gives `f` back if
    /// the loop is not empty, the last load has not been collected, the
    /// loop number is past [`MAX_LOOPS`], there are no layers or more
    /// than `MAX_LAYERS - 1`, a layer is shorter than `frames`, the
    /// memory is short, or it is a free loop and there is no free clock.
    /// A loop kept as the looper held it ([`Looper::loop_file_raw`]) is
    /// checked whole first, each layer's audio being its view's size; its
    /// memory goes up to each layer's last frame written, and its views
    /// and shape come back exactly.
    // The file comes back whole, not boxed: this runs on the audio thread,
    // which must not allocate or free.
    #[allow(clippy::result_large_err)]
    pub fn load(&mut self, n: usize, f: LoopFile) -> Result<(), LoopFile> {
        let Some(l) = self.loops.get_mut(n) else {
            return Err(f);
        };
        let ok = l.phase == Phase::Empty
            && !l.clearing
            && !l.loading
            && !l.adding
            && self.loads[n].is_none()
            && f.frames > 0
            && f.ticks > 0
            && !f.layers.is_empty()
            && f.layers.len() < MAX_LAYERS
            && (f.clock == Clock::Song || self.free_time.clock.is_some());
        // Each layer's audio and how it plays it: as it plays (the loop's
        // length, a whole view), or as the looper held it (format 2).
        let raw = match LoadRaw::of(&f) {
            Ok(r) => r,
            Err(()) => return Err(f),
        };
        let size = |k: usize| raw.as_ref().map_or(f.frames, |r| r.views[k].size());
        let ok = ok && (0..f.layers.len()).all(|k| f.layers[k].audio.len() >= 2 * size(k));
        if !ok {
            return Err(f);
        }
        // Memory for the frames written: a first pass's view reaches
        // MAX_SEAM frames past what it kept, which it never writes.
        let held = |k: usize| match &f.layers[k].raw {
            Some(r) => written_to(size(k), r.valid),
            None => size(k),
        };
        let need: usize = (0..f.layers.len()).map(|k| blocks_for(held(k))).sum();
        if self.store.available() < need {
            return Err(f);
        }
        // A loaded loop takes the place of one kept for Undo.
        self.store.drop_kept(n);
        l.reclaim(&mut self.store, n);
        for (k, y) in f.layers.iter().enumerate() {
            self.store.grow(n, k, blocks_for(held(k)));
            let r = y.raw.as_ref();
            let mut s = Slot {
                valid: r.map_or((0, f.frames), |r| r.valid),
                replace: r.and_then(|r| r.replace),
                level: y.level.clamp(0.0, 2.0),
                pan: y.pan.clamp(-1.0, 1.0),
                mute: y.muted,
                solo: y.solo,
                age: y.faded.clamp(0.0, 1.0),
                born: self.store.born(),
                ..Slot::NEW
            };
            let any_solo = f.layers.iter().any(|y| y.solo);
            (s.gl, s.gr) = s.target(any_solo);
            // A muted Replace covers nothing from its first frame.
            s.duck = if s.heard(any_solo) { 1.0 } else { 0.0 };
            l.slots[k] = s;
        }
        l.level = f.level.clamp(0.0, 2.0);
        l.muted = f.muted;
        l.bars = f.bars.filter(|&b| b > 0);
        l.after = f.after;
        l.starts = f.starts;
        l.fade_old = f.fade_old.clamp(0.0, 1.0);
        l.speed = f.speed.clamp(-1, 1);
        l.rate = f.rate.clamp(-2, 2);
        l.clock = f.clock;
        l.clock_gen = self.free_time.gen;
        l.loading = true;
        l.ld_layers = f.layers.len();
        l.ld_frames = f.frames;
        l.ld_total = (0..f.layers.len()).map(size).sum();
        l.ld_raw = raw;
        l.ld_ticks = f.ticks;
        l.ld_origin = f.origin;
        l.ld_done = 0;
        l.ld_from = self.now;
        l.ld_at = self.now + l.ld_total.div_ceil(LOAD_SPEED) as u64;
        self.loads[n] = Some(f);
        Ok(())
    }

    /// What a Capture of `bars` into an empty loop on `clock` would take
    /// now ([`Command::Capture`]), on the bars it would take
    /// ([`Looper::capture_bars`]: the song's, or for a free loop while
    /// the free clock is set, its own): the bars ending on the last bar
    /// line heard (pressed in a bar's last beat, it waits for the next one
    /// instead), read with the loops' latency. None when not even one
    /// whole bar was heard.
    pub fn capture_take(&self, clock: Clock, bars: u32) -> Option<Take> {
        let latency = self.loops[self.selected].latency;
        match self.capture_bars(clock)? {
            CaptureBars::Song => {
                let end = self.heard.lines().checked_sub(1)?;
                self.heard.take(end, bars, latency)
            }
            CaptureBars::Free(run) => {
                let (end, _) = run.last_line(self.heard.at())?;
                self.free_take(&run, end, bars, latency)
            }
        }
    }

    /// What [`Command::CaptureTo`] would take now, into an empty loop on
    /// `clock`: `bars` ending on the bar line heard on tick `end` (of the
    /// song, or of the free clock, as for [`Looper::capture_take`]).
    pub fn capture_take_to(&self, clock: Clock, end: Tick, bars: u32) -> Option<Take> {
        let latency = self.loops[self.selected].latency;
        match self.capture_bars(clock)? {
            CaptureBars::Song => {
                let end = self.heard.line_at(end)?;
                self.heard.take(end, bars, latency)
            }
            CaptureBars::Free(run) => self.free_take(&run, end, bars, latency),
        }
    }

    /// A Capture into an empty loop on `clock` now would wait for the bar
    /// line coming: the song plays (for a free loop while the free clock
    /// is set, the free clock runs) and is in a bar's last beat.
    pub fn capture_waits(&self, clock: Clock) -> bool {
        match self.capture_bars(clock) {
            Some(CaptureBars::Song) => self.heard.in_last_beat(),
            Some(CaptureBars::Free(run)) => run.coming_line(self.heard.at()).is_some(),
            None => false,
        }
    }

    /// The bar lines a capture into an empty loop on `clock` counts, the
    /// newest up to `out.len()`, oldest first: each one's tick and the
    /// frame it fell on. Returns how many. The song's as it last played
    /// on unbroken (since it started, jumped or looped); for a free loop
    /// while the free clock is set, the free clock's since it last
    /// started. With [`Looper::heard_at`], where What you played puts its
    /// bars.
    pub fn heard_lines(&self, clock: Clock, out: &mut [(Tick, u64)]) -> usize {
        match self.capture_bars(clock) {
            Some(CaptureBars::Song) => self.heard.recent(out),
            Some(CaptureBars::Free(run)) => {
                let at = self.heard.at();
                let bar = run.clock.meter.bar_ticks().max(1);
                let mut n = 0;
                let mut line = run.last_line(at);
                while let Some((t, f)) = line.filter(|_| n < out.len()) {
                    out[n] = (t, f);
                    n += 1;
                    line = run.line_heard(t - bar, at).map(|f| (t - bar, f));
                }
                out[..n].reverse();
                n
            }
            None => 0,
        }
    }

    /// The frame after the last one the looper heard (the end of the
    /// period it last ran).
    pub fn heard_at(&self) -> u64 {
        self.heard.at()
    }

    /// The saved loop loop `n` has loaded (or called off), to drop away
    /// from the audio thread. None while it is still copying in.
    pub fn take_loaded(&mut self, n: usize) -> Option<LoopFile> {
        if self.loops.get(n)?.loading {
            return None;
        }
        self.loads.get_mut(n)?.take()
    }

    /// The print loop `n` no longer holds, if any, to drop away from the
    /// audio thread.
    pub fn take_print(&mut self, n: usize) -> Option<Print> {
        self.prints.get_mut(n)?.old.take()
    }

    /// The print loop `n` holds now (frames, ticks, version), if any.
    pub fn print_info(&self, n: usize) -> Option<(usize, Tick, u64)> {
        let p = self.prints.get(n)?.now.as_ref()?;
        Some((p.frames, p.ticks, p.version))
    }

    /// As [`Looper::bounce`], for a print: the layers as heard without the
    /// loop's level, at [`PRINT_SPEED`]; the [`Bounce`] carries the loop's
    /// version. The commands of the loop wait while it renders.
    pub fn bounce_print(&mut self, n: usize, buf: Vec<f32>) -> Result<(), Vec<f32>> {
        self.start_bounce(n, buf, true)
    }

    /// Render loop `n` out into `buf`, for a pad or a WAV file: one pass,
    /// as heard ([`Bounce`]). `buf` needs room for the loop (twice its
    /// frames) and is never grown, so this does not allocate. It renders
    /// in the next periods at [`MERGE_SPEED`]; the loop plays on, and its
    /// commands wait until it is done (Clear stops it). One at a time.
    /// `buf` comes back as the error when the loop is not playing, is
    /// recording or merging, `buf` is too small, or a bounce is not yet
    /// taken.
    pub fn bounce(&mut self, n: usize, buf: Vec<f32>) -> Result<(), Vec<f32>> {
        self.start_bounce(n, buf, false)
    }

    fn start_bounce(&mut self, n: usize, mut buf: Vec<f32>, dry: bool) -> Result<(), Vec<f32>> {
        let Some(l) = self.loops.get_mut(n) else {
            return Err(buf);
        };
        let ready = self.bounce.is_none()
            && l.phase == Phase::Playing
            && !l.od_open
            && !l.first_open
            && !l.busy()
            && !l.clearing
            && !l.fitting
            && !l.swapping
            && l.len > 0
            && buf.capacity() >= 2 * l.len;
        if !ready {
            return Err(buf);
        }
        buf.clear();
        buf.resize(2 * l.len, 0.0);
        l.mg_g = l.heard_gains();
        l.bouncing = true;
        self.bounce = Some(Bouncing {
            n,
            buf,
            done: 0,
            complete: None,
            dry,
            version: l.version,
        });
        Ok(())
    }

    /// How far the bounce is, 0 to 1 (None if there is none).
    pub fn bounce_progress(&self) -> Option<f32> {
        self.bounce
            .as_ref()
            .map(|b| b.done as f32 / (b.buf.len() / 2).max(1) as f32)
    }

    /// The finished bounce, with its buffer; None while it renders or if
    /// there is none. Moves the buffer out without allocating.
    pub fn take_bounce(&mut self) -> Option<Bounce> {
        let complete = self.bounce.as_ref()?.complete?;
        let b = self.bounce.take()?;
        let l = &self.loops[b.n];
        Some(Bounce {
            from_loop: b.n,
            frames: b.buf.len() / 2,
            audio: b.buf,
            ticks: l.len_ticks,
            origin: l.origin,
            complete,
            version: b.version,
            dry: b.dry,
            rate: l.rate,
        })
    }
}
