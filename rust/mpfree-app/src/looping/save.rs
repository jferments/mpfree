//! Saving the loops with a project, and opening them again.

use super::*;

/// A save under way ([`LoopHost::save`]).
pub(crate) enum Saving {
    /// Until no loop is recording a first pass, overdubbing, merging,
    /// fitting or loading.
    Waiting,
    /// The worker is making room for the layers' audio.
    Sizing {
        /// Each loop's (version, layers, frames) when the save began.
        at: [(u64, usize, usize); MAX_LOOPS],
        /// A snapshot (phase 4c.4, recordings on): each layer's number,
        /// by loop and play order, read by it whatever the loop does.
        snap: Option<Borns>,
    },
    /// Reading layer `k` of `saved.loops[i]` from `frame`.
    Reading {
        saved: Saved,
        at: [(u64, usize, usize); MAX_LOOPS],
        snap: Option<Borns>,
        i: usize,
        k: usize,
        frame: usize,
    },
}

/// Each loop's layers' numbers ([`LayerInfo::born`]), by loop and play
/// order, as a snapshot save took them (phase 4c.4).
type Borns = [[u64; MAX_LAYERS]; MAX_LOOPS];

/// Frames of the loops' layers a save reads for each frame that plays.
pub const SAVE_SPEED: usize = 64;

/// Frames a save reads of layer `k` of `f`: the loop's length as it
/// plays, or as the looper holds it, its audio's size (phase 4c.2).
pub(crate) fn layer_frames(f: &LoopFile, k: usize) -> usize {
    match copied(f, k) {
        true => 0,
        false => f.layers[k].raw.as_ref().map_or(f.frames, |r| r.size),
    }
}

/// Layer `k` of `f` is in its recording's synced file, which a snapshot
/// save named (phase 4c.4): the project copies the file, and nothing of
/// it is read or held for the save.
pub fn copied(f: &LoopFile, k: usize) -> bool {
    let y = &f.layers[k];
    !y.file.is_empty() && y.raw.as_ref().is_some_and(|r| r.rec != 0)
}

impl LoopHost {
    /// Save every loop for a project: once no loop is recording a first
    /// pass, overdubbing, merging, fitting or loading, the worker makes
    /// room, and each loop's layers are read as they play ([`SAVE_SPEED`]
    /// frames for each frame), with no glitch. A loop that changes while
    /// it is read starts the save again. [`LoopHost::take_saved`] gives
    /// it when done, with each layer's audio: the project keeps the text
    /// (`mpfree_looper::save::to_text`, each layer naming its file) and a
    /// WAV a layer (`save::wav`). Allocates only the list of loops, here
    /// and when it starts again.
    pub fn save(&mut self) -> String {
        if self.printer.tx.is_none() {
            return "looper: cannot save, the worker did not start".into();
        }
        if let Some(s) = self.saved.take() {
            self.printer.send(ToPrinter::DropSaved(s));
        }
        if let Some(Saving::Reading { saved, .. }) = self.saving.take() {
            self.printer.send(ToPrinter::DropSaved(saved));
        }
        self.saving = Some(Saving::Waiting);
        "looper: saving the loops".into()
    }

    /// How far the save is, 0 to 1 (0 while it waits for a loop).
    pub fn saving(&self) -> Option<f32> {
        match self.saving.as_ref()? {
            Saving::Reading {
                saved, i, k, frame, ..
            } => {
                // Frames of a loop's first `upto` layers.
                let frames = |f: &LoopFile, upto: usize| -> usize {
                    (0..upto.min(f.layers.len()))
                        .map(|j| layer_frames(f, j))
                        .sum()
                };
                let all: usize = saved
                    .loops
                    .iter()
                    .map(|(_, f)| frames(f, f.layers.len()))
                    .sum();
                let done: usize = saved.loops[..*i]
                    .iter()
                    .map(|(_, f)| frames(f, f.layers.len()))
                    .sum::<usize>()
                    + saved.loops.get(*i).map_or(0, |(_, f)| frames(f, *k))
                    + frame;
                Some(done as f32 / all.max(1) as f32)
            }
            _ => Some(0.0),
        }
    }

    /// The save, once done: drop it away from the audio loop.
    pub fn take_saved(&mut self) -> Option<Saved> {
        self.saved.take()
    }

    /// A save under way stops, and one done but not taken is dropped (a
    /// project opening while the save of another goes on). Does not
    /// allocate.
    pub fn cancel_save(&mut self) {
        if let Some(s) = self.saved.take() {
            self.printer.send(ToPrinter::DropSaved(s));
        }
        if let Some(Saving::Reading { saved, .. }) = self.saving.take() {
            self.printer.send(ToPrinter::DropSaved(saved));
        }
    }

    /// Loops taken from a save that is not going ahead, dropped on the
    /// worker rather than here.
    pub fn drop_saved(&self, s: Saved) {
        self.printer.send(ToPrinter::DropSaved(s));
    }

    /// An open or New project: a save under way stops, and every loop
    /// fades out and is forgotten (as Clear does), from the next periods.
    /// The project's loops, if it has any, come with [`LoopHost::open`]
    /// and go in once every loop is empty ([`LoopHost::tend_open`]).
    pub fn clear_all(&mut self) -> String {
        self.cancel_save();
        if let Some(s) = self.incoming.take() {
            self.printer.send(ToPrinter::DropSaved(s));
        }
        self.steps_left = 0;
        self.clear_next = true;
        self.emptying = true;
        self.on_tracks.forget();
        self.tend_open();
        "looper: every loop fades out for the project".into()
    }

    /// An opened project's loops: they go in once every loop is empty.
    pub fn open(&mut self, saved: Saved) {
        if let Some(s) = self.incoming.replace(saved) {
            self.printer.send(ToPrinter::DropSaved(s));
        }
    }

    /// Whether an open's loops are still going out or coming in.
    pub fn opening(&self) -> bool {
        self.clear_next || self.emptying || self.incoming.is_some()
    }

    /// Between periods while [`LoopHost::opening`]: the clears once the
    /// queue has room, then the project's loops once every loop is empty.
    /// Returns a line for the log when they go in.
    pub fn tend_open(&mut self) -> Option<String> {
        if self.clear_next {
            if self.looper.room() < 2 * MAX_LOOPS + 1 {
                return None;
            }
            for n in 0..MAX_LOOPS {
                self.looper.select_loop(n);
                self.looper.command(Command::Forget);
            }
            self.looper.select_loop(0);
            self.picked = 0;
            self.for_cell = false;
            self.clear_next = false;
            // The clears apply with the next period: a load before then
            // would be cleared with them.
            self.settle = 2;
            return None;
        }
        if self.settle > 0 {
            return None;
        }
        let empty = (0..MAX_LOOPS).all(|n| {
            self.looper
                .loop_status(n)
                .is_some_and(|s| s.phase == Phase::Empty && s.loading.is_none())
        });
        if !empty {
            return None;
        }
        self.emptying = false;
        let saved = self.incoming.take()?;
        Some(self.load(saved))
    }

    /// Load saved loops (`save::from_text`, each layer's audio from its
    /// WAV with `save::from_wav`) into the loops they came
    /// from, which must be empty: each copies in while silent, then fades
    /// in, in time with the song. Free loops load onto the saved free
    /// clock, opened first, stopped: they open stopped at their tops, as
    /// a project opens with nothing launched. A loop that is not empty
    /// keeps what it has (its saved one is dropped on the worker), and
    /// free loops can't load while free loops still hold the free clock.
    pub fn load(&mut self, saved: Saved) -> String {
        let clock = saved
            .free_clock
            .filter(|c| self.looper.open_free_clock(c.bpm, c.meter, c.top));
        let (mut done, mut kept, mut free, mut unclocked) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for (n, f) in saved.loops {
            let id = f.id;
            let on_free = f.clock == Clock::Free;
            let mut recs = [0u128; mpfree_looper::MAX_LAYERS];
            let mut from = [[0u128; mpfree_looper::MAX_LAYERS]; mpfree_looper::MAX_LAYERS];
            for ((r, m), y) in recs.iter_mut().zip(from.iter_mut()).zip(&f.layers) {
                *r = y.raw.as_ref().map_or(0, |r| r.rec);
                for (a, &b) in m
                    .iter_mut()
                    .zip(y.raw.as_ref().map_or(&[][..], |r| &r.from))
                {
                    *a = b;
                }
            }
            match self.looper.load(n, f) {
                Ok(()) => {
                    self.on_tracks.set(n, id);
                    self.layer_recs.opened(&self.looper, n, &recs, &from);
                    done.push((n + 1).to_string());
                    if on_free {
                        free.push((n + 1).to_string());
                    }
                }
                Err(f) => {
                    if on_free && clock.is_none() {
                        unclocked.push((n + 1).to_string());
                    } else {
                        kept.push((n + 1).to_string());
                    }
                    self.printer.send(ToPrinter::DropLoop(f));
                }
            }
        }
        self.looper.select_loop(saved.selected);
        self.picked = saved.selected;
        self.for_cell = false;
        let mut note = format!("looper: loading loops {}", done.join(", "));
        if done.is_empty() {
            note = "looper: no loops to load".into();
        }
        if let Some(c) = clock.filter(|_| !free.is_empty()) {
            note += &format!(
                "; loops {} are free loops, stopped at their tops: PLAY starts them with \
                 the song ({:.2} BPM in {}/{})",
                free.join(", "),
                c.bpm,
                c.meter.beats,
                c.meter.unit
            );
        }
        if !kept.is_empty() {
            note += &format!(
                "; loops {} were not empty and kept what they had",
                kept.join(", ")
            );
        }
        if !unclocked.is_empty() {
            note += &format!(
                "; free loops {} weren't loaded: other free loops still hold the free clock",
                unclocked.join(", ")
            );
        }
        if saved.rate != self.rate {
            note += &format!(
                " (saved at {} Hz: they play like tape until their prints are in)",
                saved.rate
            );
        }
        note
    }

    /// Each loop's (version, layers, frames) now.
    fn shapes(&self) -> [(u64, usize, usize); MAX_LOOPS] {
        let mut at = [(0, 0, 0); MAX_LOOPS];
        for (n, a) in at.iter_mut().enumerate() {
            if let Some(s) = self.looper.loop_status(n) {
                *a = (s.version, s.layers, s.frames);
            }
        }
        at
    }

    /// One period of a save: see [`LoopHost::save`].
    pub(crate) fn tend_save(&mut self) {
        let Some(state) = self.saving.take() else {
            return;
        };
        self.saving = match state {
            Saving::Waiting => {
                // Every loop empty (or armed) or still enough to read; with
                // recordings on, every layer closed too (a first pass's
                // tail in), so a snapshot can read each by its number.
                let snap = self.recordings;
                let ready = (0..MAX_LOOPS).all(|n| {
                    let s = self.looper.loop_status(n).expect("a loop");
                    matches!(s.phase, Phase::Empty | Phase::Armed) && s.loading.is_none()
                        || self.looper.read_layer(n, 0, 0, &mut []).is_some()
                            && (!snap
                                || (0..MAX_LAYERS)
                                    .map_while(|k| self.looper.loop_layer_info(n, k))
                                    .all(|y| y.closed))
                });
                if ready {
                    let mut saved = Saved {
                        rate: self.rate,
                        selected: self.looper.selected(),
                        loops: (0..MAX_LOOPS)
                            .filter_map(|n| {
                                let mut f = match self.recordings {
                                    true => self.looper.loop_file_raw(n)?,
                                    false => self.looper.loop_file(n)?,
                                };
                                // Its lasting id, if it has one (phase 4b).
                                f.id = self.on_tracks.id(n).map_or(0, |i| i.0);
                                // Each layer's recording (4c.2); one whose
                                // file is synced is copied from it (4c.4).
                                for (k, y) in f.layers.iter_mut().enumerate() {
                                    if let (Some(r), Some(info)) =
                                        (y.raw.as_mut(), self.looper.loop_layer_info(n, k))
                                    {
                                        r.rec = self.layer_recs.of(n, &info);
                                        // What a merge or a fit made it
                                        // from (4c.5).
                                        r.from = self.layer_recs.sources_of(n, &info).to_vec();
                                        if snap
                                            && r.rec != 0
                                            && self.layer_recs.file(n, &info)
                                                == crate::loops_on_tracks::FileState::Synced
                                        {
                                            y.file = format!("loop_{:032x}.wav", r.rec);
                                        }
                                    }
                                }
                                Some((n, f))
                            })
                            .collect(),
                        free_clock: self
                            .looper
                            .free_clock()
                            .map(|c| mpfree_looper::save::ClockFile::of(&c)),
                        kept: Vec::new(),
                    };
                    // Recordings held for an Undo that no layer names
                    // (4c.6): named, so the project keeps their files.
                    if snap {
                        saved.kept = self.layer_recs.held_files(&self.looper);
                        saved.kept.retain(|f| {
                            !saved
                                .loops
                                .iter()
                                .any(|(_, l)| l.layers.iter().any(|y| y.file == *f))
                        });
                    }
                    // The snapshot: each layer by its number.
                    let borns = snap.then(|| {
                        let mut b = [[0; MAX_LAYERS]; MAX_LOOPS];
                        for (n, row) in b.iter_mut().enumerate() {
                            for (k, x) in row.iter_mut().enumerate() {
                                match self.looper.loop_layer_info(n, k) {
                                    Some(y) => *x = y.born,
                                    None => break,
                                }
                            }
                        }
                        b
                    });
                    self.printer.send(ToPrinter::Size(saved));
                    Some(Saving::Sizing {
                        at: self.shapes(),
                        snap: borns,
                    })
                } else {
                    Some(Saving::Waiting)
                }
            }
            Saving::Sizing { at, snap } => match self.printer.sized.take() {
                Some(saved) => Some(Saving::Reading {
                    saved,
                    at,
                    snap,
                    i: 0,
                    k: 0,
                    frame: 0,
                }),
                None => Some(Saving::Sizing { at, snap }),
            },
            Saving::Reading {
                mut saved,
                at,
                snap,
                mut i,
                mut k,
                mut frame,
            } => {
                // A snapshot reads closed layers by their numbers, which
                // never change, so the loops may go on; without one, a
                // loop that changes starts the save again.
                if snap.is_none() && self.shapes() != at {
                    // A loop changed: start again.
                    self.save_restarts += 1;
                    self.printer.send(ToPrinter::DropSaved(saved));
                    self.saving = Some(Saving::Waiting);
                    return;
                }
                let mut budget = SAVE_SPEED * PERIOD;
                while budget > 0 && i < saved.loops.len() {
                    let (n, f) = &mut saved.loops[i];
                    let size = layer_frames(f, k);
                    let take = budget.min(size - frame);
                    let raw = f.layers[k].raw.is_some();
                    let out = &mut f.layers[k].audio[2 * frame..2 * (frame + take)];
                    // By its number (a snapshot, 4c.4), as written (4c.2)
                    // or as it plays.
                    let read = match (snap.as_ref(), raw) {
                        _ if take == 0 => Some(0),
                        (Some(b), _) => self.looper.raw_of(*n, b[*n][k], frame, out).map(|_| 0),
                        (None, true) => self.looper.raw_layer(*n, k, frame, out),
                        (None, false) => self.looper.read_layer(*n, k, frame, out),
                    };
                    if read.is_none() && snap.is_some() {
                        // The layer went from memory before it was read
                        // (cleared, undone): start again.
                        self.save_restarts += 1;
                        self.printer.send(ToPrinter::DropSaved(saved));
                        self.saving = Some(Saving::Waiting);
                        return;
                    }
                    if read.is_none() {
                        // Busy for now (a merge or a fit starting): wait.
                        break;
                    }
                    budget -= take;
                    frame += take;
                    if frame == size {
                        frame = 0;
                        k += 1;
                        if k == f.layers.len() {
                            k = 0;
                            i += 1;
                        }
                    }
                }
                if i == saved.loops.len() {
                    self.saved = Some(saved);
                    None
                } else {
                    Some(Saving::Reading {
                        saved,
                        at,
                        snap,
                        i,
                        k,
                        frame,
                    })
                }
            }
        };
    }
}
