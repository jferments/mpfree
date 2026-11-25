//! Work spread over periods: prints and fits, loops opening and
//! captures copying in, and layers copied in from another loop.

use super::*;

impl Loop {
    /// The print in the looper's hands changed: its numbers, for `follow`.
    pub(crate) fn take_print(&mut self, p: Option<&Print>) {
        match p {
            Some(p) => {
                self.p_frames = p.frames;
                self.p_ticks = p.ticks;
                self.p_origin = p.origin;
                self.p_version = p.version;
                self.ppos = 0.0;
                self.pjump_pos = 0.0;
            }
            None => self.p_frames = 0,
        }
        self.print_on = 0.0;
    }

    /// The loop can play a print now: playing, nothing being written or
    /// changed.
    fn print_ok(&self) -> bool {
        self.phase == Phase::Playing
            && !self.od_open
            && !self.first_open
            && !self.merging
            && !self.clearing
            && !self.swapping
            && self.len > 0
    }

    /// Print `p` is for the loop as it is now (and no other print waits to
    /// take over: `waiting`).
    pub(crate) fn print_live(&self, p: &Print, waiting: bool) -> bool {
        !waiting
            && self.p_frames == p.frames
            && p.version == self.version
            && p.ticks == self.len_ticks
            && p.origin == self.origin
            && self.print_ok()
    }

    /// Start fitting the loop to its print (`frames` long) in the period
    /// starting at `now`: the print is copied into a new layer at
    /// [`FIT_SPEED`], which then takes the layers' place.
    pub(crate) fn start_fit(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        frames: usize,
        now: u64,
    ) {
        let ready = self.phase == Phase::Playing
            && !self.od_open
            && !self.first_open
            && !self.merging
            && !self.bouncing
            && !self.fitting
            && !self.swapping
            && !self.clearing
            && !self.fading()
            && self.xf == 0
            && self.len > 0
            && self.in_order > 0
            && frames > 0;
        if !ready {
            stats.refused += 1;
            return;
        }
        let Some(k) = self.free_slot_for(store, me, frames) else {
            stats.refused += 1;
            return;
        };
        let Some(i) = self.free_shape() else {
            stats.refused += 1;
            return;
        };
        self.slots[k] = Slot {
            valid: (0, frames),
            born: store.born(),
            ..Slot::NEW
        };
        self.fitting = true;
        self.fit_slot = k;
        self.fit_shape = i;
        self.fit_done = 0;
        self.fit_frames = frames;
        self.fit_from = now;
        self.fit_at = now + frames.div_ceil(FIT_SPEED) as u64;
    }

    /// Copy the print as far as the period ending before frame `end` is
    /// due: [`FIT_SPEED`] frames for each frame since the fit started.
    pub(crate) fn render_fit(&mut self, store: &mut Store, me: usize, p: &Print, end: u64) {
        let due = (end - self.fit_from).saturating_mul(FIT_SPEED as u64);
        let target = self.fit_frames.min(due.min(usize::MAX as u64) as usize);
        for q in self.fit_done..target {
            store.set(me, self.fit_slot, q, [p.audio[2 * q], p.audio[2 * q + 1]]);
        }
        self.fit_done = self.fit_done.max(target);
    }

    /// Playing with nothing changing its layers, so that it can be saved.
    pub(crate) fn savable(&self) -> bool {
        self.phase == Phase::Playing
            && self.len > 0
            && !self.od_open
            && !self.first_open
            && !self.merging
            && !self.fitting
            && !self.swapping
            && !self.clearing
            && !self.loading
            && !self.adding
    }

    /// Copy a saved loop's layers in as far as the period ending before
    /// frame `end` is due: [`LOAD_SPEED`] frames for each frame since the
    /// load started.
    pub(crate) fn render_load(&mut self, store: &mut Store, me: usize, f: &LoopFile, end: u64) {
        let total = self.ld_total;
        let due = (end - self.ld_from).saturating_mul(LOAD_SPEED as u64);
        let target = total.min(due.min(usize::MAX as u64) as usize);
        // Each layer's frames in turn: the loop's length, or as the
        // looper held it, its audio's own size.
        let size = |k: usize| match &self.ld_raw {
            Some(r) => r.views[k].size(),
            None => self.ld_frames,
        };
        let (mut k, mut base) = (0, 0);
        for i in self.ld_done..target {
            while i - base >= size(k) {
                base += size(k);
                k += 1;
            }
            let q = i - base;
            let a = &f.layers[k].audio;
            store.set(me, k, q, [a[2 * q], a[2 * q + 1]]);
        }
        self.ld_done = self.ld_done.max(target);
    }

    /// A capture copying in from what was heard, as far as the period
    /// ending before frame `end` is due. Its loop point fades into what
    /// was heard just before its first frame, as a first pass's does. Were
    /// what it reads gone by (the room [`Heard::take`] leaves keeps it),
    /// the capture would be thrown away.
    pub(crate) fn render_heard(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        heard: &Heard,
        end: u64,
    ) {
        let Some(from) = self.ld_heard else {
            return;
        };
        let total = self.ld_frames;
        let due = (end - self.ld_from).saturating_mul(LOAD_SPEED as u64);
        let target = total.min(due.min(usize::MAX as u64) as usize);
        if self.ld_done < target && !heard.holds(from + self.ld_done as u64) {
            stats.aborted += 1;
            self.loading = false;
            self.ld_heard = None;
            self.reclaim(store, me);
            return;
        }
        let fade = FADE.min(total / 2);
        for q in self.ld_done..target {
            let mut x = heard.frame(from + q as u64);
            if q + fade >= total {
                let k = q + fade - total;
                let w = (k as f32 + 0.5) / fade as f32;
                let p = (from + k as u64)
                    .checked_sub(fade as u64)
                    .map_or([0.0; 2], |f| heard.frame(f));
                x = [x[0] * (1.0 - w) + p[0] * w, x[1] * (1.0 - w) + p[1] * w];
            }
            store.set(me, 0, q, x);
        }
        self.ld_done = self.ld_done.max(target);
    }

    /// The saved loop (or a capture) is all in: it plays from now on
    /// (fading in), in time with the song.
    pub(crate) fn finish_load(&mut self) {
        self.loading = false;
        self.ld_heard = None;
        self.len = self.ld_frames;
        self.cut = self.len;
        self.len_ticks = self.ld_ticks;
        self.origin = self.ld_origin;
        self.in_order = self.ld_layers;
        for k in 0..self.ld_layers {
            self.order[k] = k as u8;
            self.views[k] = View::whole(self.len);
        }
        self.seam = FADE;
        // As the looper held it: its views and shape, exactly.
        if let Some(r) = self.ld_raw.take() {
            self.views = r.views;
            self.seam = r.shape.seam;
            self.region = r.shape.region;
            self.captured = r.shape.captured;
            self.cut = r.shape.cut;
        }
        self.n_done = 0;
        self.n_undone = 0;
        self.gain = 0.0;
        self.pos = 0.0;
        self.jump = 0;
        self.xf = 0;
        self.mixing = true;
        self.phase = Phase::Playing;
        self.touch();
    }

    /// The layer copied in joins the loop, fading in, as a step Undo
    /// takes back. Into an empty loop it is the loop: it plays from now on
    /// (fading in), in time with the song.
    pub(crate) fn finish_add(&mut self) {
        self.adding = false;
        if self.phase == Phase::Empty {
            self.len = self.ad_frames;
            self.len_ticks = self.ad_ticks;
            self.origin = self.ad_origin;
            self.in_order = 0;
            self.seam = FADE;
            self.n_done = 0;
            self.n_undone = 0;
            self.gain = 0.0;
            self.pos = 0.0;
            self.jump = 0;
            self.xf = 0;
            self.phase = Phase::Playing;
        }
        let k = self.ad_slot;
        self.views[k] = View::whole(self.len);
        self.order[self.in_order] = k as u8;
        self.push_done(Act {
            kind: Kind::Added,
            slot: k as u8,
            index: self.in_order as u8,
            edit: None,
            ages: [1.0; MAX_LAYERS],
            link: Some(Link {
                lp: self.ad_from as u8,
                slot: self.ad_src as u8,
                tied: self.ad_move,
            }),
            shift: Shift::NONE,
            bars: 0,
            txn: TransactionId::NONE,
        });
        self.in_order += 1;
        self.mixing = true;
        self.touch();
    }

    /// The copy of one of the loop's layers has joined the other loop: a
    /// move takes the layer out of this one, as a delete Undo takes back
    /// (with the other loop's step while each is its loop's last).
    pub(crate) fn end_lend(&mut self) {
        let Some(l) = self.lend.take() else {
            return;
        };
        let at = self.order[..self.in_order]
            .iter()
            .position(|&k| k == l.slot)
            .filter(|_| !self.slots[l.slot as usize].leaving);
        let Some(at) = at.filter(|_| l.drop && self.layers() > 1) else {
            return;
        };
        self.n_undone = 0;
        self.push_done(Act {
            kind: Kind::Delete,
            slot: l.slot,
            index: at as u8,
            edit: None,
            ages: [1.0; MAX_LAYERS],
            link: Some(Link {
                lp: l.to,
                slot: l.to_slot,
                tied: true,
            }),
            shift: Shift::NONE,
            bars: 0,
            txn: TransactionId::NONE,
        });
        self.slots[l.slot as usize].leaving = true;
        self.mixing = true;
        self.touch();
    }

    /// An overdub waiting for the print at another speed
    /// ([`Loop::print_wait`]): once the print plays, the fit starts and
    /// the overdub with it; back at the speed it was recorded at (no print
    /// needed), the overdub starts at once.
    pub(crate) fn tend_print_wait(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        pr: &Prints,
        now: u64,
    ) {
        if self.phase != Phase::Playing || self.od_open || self.fitting {
            self.print_wait = false;
            return;
        }
        if self.adding || self.lend.is_some() || self.busy() {
            return;
        }
        let live = pr
            .now
            .as_ref()
            .filter(|p| self.print_on > 0.0 && self.print_live(p, pr.next.is_some()));
        if let Some(frames) = live.map(|p| p.frames) {
            self.print_wait = false;
            self.start_fit(store, stats, me, frames, now);
            if self.fitting {
                self.fit_overdub = true;
            }
        } else if self.rate == 0 && self.print_on == 0.0 {
            self.print_wait = false;
            self.open_overdub(store, stats, me, FADE);
        }
    }

    pub(crate) fn cancel_fit(&mut self, stats: &mut Stats) {
        self.fitting = false;
        self.fit_overdub = false;
        self.print_wait = false;
        stats.refused += 1;
    }

    /// The fitted layer takes the layers' place, at the print's length:
    /// it sounds as the print did, which fades out as the loop is no
    /// longer at the print's version. Undo brings the layers back.
    pub(crate) fn finish_fit(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        p: Option<&Print>,
    ) {
        let Some(p) = p.filter(|p| self.print_live(p, false)) else {
            self.cancel_fit(stats);
            return;
        };
        self.fit_from = 0;
        self.render_fit(store, me, p, u64::MAX / (2 * FIT_SPEED as u64));
        self.fitting = false;
        let (k, i) = (self.fit_slot, self.fit_shape);
        self.shapes[i] = self.shape();
        self.note_derived(k);
        let old = self.len as f64;
        self.len = self.fit_frames;
        // The print was made at the loop's speed: its audio plays as it is.
        self.rate = 0;
        self.views[k] = View::whole(self.len);
        self.pos = (self.pos / old * self.len as f64).min(self.len as f64 - 1e-6);
        self.order[0] = k as u8;
        self.in_order = 1;
        self.mixing = true;
        self.n_undone = 0;
        self.touch();
        self.push_done(Act {
            kind: Kind::Fit,
            slot: k as u8,
            index: i as u8,
            edit: None,
            ages: [1.0; MAX_LAYERS],
            link: None,
            shift: Shift::NONE,
            bars: 0,
            txn: TransactionId::NONE,
        });
        if std::mem::take(&mut self.fit_overdub) {
            self.open_overdub(store, stats, me, FADE);
        }
    }
}
