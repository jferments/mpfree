//! Merge, Fade old, Undo and Redo, and the layers rendered as heard
//! for a merge or a bounce.

use super::*;

impl Loop {
    /// A fit's undo or redo: dip out, then swap (`swap`).
    fn start_swap(&mut self, i: usize) {
        self.swapping = true;
        self.swap_shape = i;
        self.mixing = true;
    }

    /// Swap the loop's shape and layers with the ones kept at
    /// `swap_shape`, while it is silent.
    pub(crate) fn swap(&mut self) {
        let i = self.swap_shape;
        let now = self.shape();
        let kept = self.shapes[i];
        let old = self.len as f64;
        self.set_shape(&kept);
        self.order = kept.order;
        self.in_order = kept.in_order;
        self.shapes[i] = now;
        if self.len > 0 {
            self.pos = (self.pos / old * self.len as f64).min(self.len as f64 - 1e-6);
        }
        self.jump = 0;
        self.xf = 0;
        self.swapping = false;
        self.mixing = true;
        self.touch();
    }

    /// Start a merge ([`Command::Merge`]) in the period starting at `now`.
    pub(crate) fn merge(&mut self, store: &mut Store, stats: &mut Stats, me: usize, now: u64) {
        let ready = self.phase == Phase::Playing
            && !self.od_open
            && !self.first_open
            && !self.merging
            && !self.bouncing
            && self.len > 0
            && self.in_order > 0;
        let g = self.heard_gains();
        let first = self.order[0] as usize;
        // One layer playing its audio as it is: all of it, or a first
        // pass's region from its start.
        let plain = self.views[first].is_whole()
            || (first == 0 && self.bounds_now().is_some_and(|(start, src)| start == src));
        let nothing = self.in_order == 1 && plain && g[first] == [1.0, 1.0];
        if !ready || nothing {
            stats.refused += 1;
            return;
        }
        let Some(k) = self.free_slot(store, me) else {
            stats.refused += 1;
            return;
        };
        let Some(i) = self.free_shape() else {
            stats.refused += 1;
            return;
        };
        self.slots[k] = Slot {
            valid: (0, self.len),
            born: store.born(),
            ..Slot::NEW
        };
        self.views[k] = View::whole(self.len);
        self.mg_g = g;
        self.merging = true;
        self.mg_slot = k;
        self.mg_shape = i;
        self.mg_done = 0;
        self.mg_from = now;
        self.mg_at = now + self.len.div_ceil(MERGE_SPEED) as u64;
    }

    /// Render the merge up to what the period ending before frame `end`
    /// is due: [`MERGE_SPEED`] frames for each frame since it started.
    pub(crate) fn render_merge(&mut self, store: &mut Store, me: usize, end: u64) {
        let due = (end - self.mg_from).saturating_mul(MERGE_SPEED as u64);
        let target = self.len.min(due.min(usize::MAX as u64) as usize);
        while self.mg_done < target {
            let from = self.mg_done;
            let n = RUN.min(target - from);
            let mut acc = [[0.0f32; 2]; RUN];
            self.render_layers(store, me, from, &mut acc[..n]);
            for (j, a) in acc[..n].iter().enumerate() {
                store.set(me, self.mg_slot, from + j, *a);
            }
            self.mg_done += n;
        }
    }

    /// Frames `from..` of every layer, as heard (at the gains in `mg_g`),
    /// added into `acc`.
    fn render_layers(&self, store: &Store, me: usize, from: usize, acc: &mut [[f32; 2]]) {
        self.render_layers_at(store, me, from, acc, &self.mg_g, self.replacing());
    }

    /// [`Loop::render_layers`] at the gains `gains` (each layer slot's,
    /// left and right); `replacing` hears Replace layers over the layers
    /// under them, as they play.
    pub(crate) fn render_layers_at(
        &self,
        store: &Store,
        me: usize,
        from: usize,
        acc: &mut [[f32; 2]],
        gains: &[[f32; 2]; MAX_LAYERS],
        replacing: bool,
    ) {
        if replacing {
            // Frame by frame, each layer at its share under the Replace
            // layers after it, as they are heard.
            let any_solo = self.any_solo();
            let strength = |s: &Slot| if s.heard(any_solo) { 1.0 } else { 0.0 };
            for (j, a) in acc.iter_mut().enumerate() {
                let p = from + j;
                let d = self.ducks(&self.views, self.len, p, None, strength);
                for &k in &self.order[..self.in_order] {
                    let k = k as usize;
                    let g = gains[k];
                    let v = &self.views[k];
                    if g == [0.0, 0.0] || v.size() == 0 || d[k] == 0.0 {
                        continue;
                    }
                    let blocks = store.blocks_of(me, k);
                    let x = layer_frame(store, blocks, self.slots[k].valid, v, self.seam, p);
                    a[0] += g[0] * x[0] * d[k];
                    a[1] += g[1] * x[1] * d[k];
                }
            }
            return;
        }
        for &k in &self.order[..self.in_order] {
            let k = k as usize;
            let g = gains[k];
            let v = &self.views[k];
            if g == [0.0, 0.0] || v.size() == 0 {
                continue;
            }
            let blocks = store.blocks_of(me, k);
            let valid = self.slots[k].valid;
            let mut at = Cursor::new(v);
            for (j, a) in acc.iter_mut().enumerate() {
                let (i, off) = at.locate(from + j);
                let x = piece_frame(store, blocks, valid, v, self.seam, i, off);
                a[0] += g[0] * x[0];
                a[1] += g[1] * x[1];
            }
        }
    }

    /// The gains each layer is heard at, for a merge or a bounce.
    /// The gains Listen plays at: the loop as heard, or one layer alone
    /// (`id`, a layer slot) at its level and pan, even when muted.
    pub(crate) fn listen_gains(&self, layer: Option<u8>) -> [[f32; 2]; MAX_LAYERS] {
        let Some(id) = layer else {
            return self.heard_gains();
        };
        let mut g = [[0.0f32; 2]; MAX_LAYERS];
        let k = id as usize;
        if self.order[..self.in_order].contains(&id) {
            let s = &self.slots[k];
            let level = s.level * s.age;
            g[k] = [
                level * (1.0 - s.pan).min(1.0),
                level * (1.0 + s.pan).min(1.0),
            ];
        }
        g
    }

    pub(crate) fn heard_gains(&self) -> [[f32; 2]; MAX_LAYERS] {
        let any_solo = self.any_solo();
        let mut g = [[0.0f32; 2]; MAX_LAYERS];
        for &k in &self.order[..self.in_order] {
            let (l, r) = self.slots[k as usize].target(any_solo);
            g[k as usize] = [l, r];
        }
        g
    }

    /// A bounce's frames `from..to`, at `level`, into `out` (interleaved).
    pub(crate) fn render_out(
        &self,
        store: &Store,
        me: usize,
        from: usize,
        to: usize,
        level: f32,
        out: &mut [f32],
    ) {
        let mut p = from;
        while p < to {
            let n = RUN.min(to - p);
            let mut acc = [[0.0f32; 2]; RUN];
            self.render_layers(store, me, p, &mut acc[..n]);
            for (j, a) in acc[..n].iter().enumerate() {
                out[2 * (p + j)] = a[0] * level;
                out[2 * (p + j) + 1] = a[1] * level;
            }
            p += n;
        }
    }

    /// Slot `k`, made by a merge or a fit, is made from the layers in
    /// the play order now (4c.5).
    pub(crate) fn note_derived(&mut self, k: usize) {
        let mut from = [0; MAX_LAYERS];
        for (f, &s) in from.iter_mut().zip(&self.order[..self.in_order]) {
            *f = self.slots[s as usize].born;
        }
        self.derived[k] = (self.slots[k].born, from);
    }

    /// The merged layer takes the layers' place at once (it sounds the
    /// same); Undo brings them back.
    pub(crate) fn finish_merge(&mut self, store: &mut Store, me: usize) {
        self.render_merge(store, me, u64::MAX / 4);
        self.merging = false;
        let (k, i) = (self.mg_slot, self.mg_shape);
        self.shapes[i] = self.shape();
        self.note_derived(k);
        self.order[0] = k as u8;
        self.in_order = 1;
        self.mixing = true;
        self.n_undone = 0;
        self.push_done(Act {
            kind: Kind::Merge,
            slot: k as u8,
            index: i as u8,
            edit: None,
            ages: [1.0; MAX_LAYERS],
            link: None,
            shift: Shift::NONE,
            bars: 0,
            txn: TransactionId::NONE,
        });
    }

    /// Undo a merge: the merged layer fades out, and the layers it took
    /// the place of (kept at `a.index`) fade back in, in their order.
    fn unmerge(&mut self, a: Act) {
        self.mixing = true;
        let i = a.index as usize;
        self.slots[a.slot as usize].leaving = true;
        let kept = self.shapes[i];
        for &k in &kept.order[..kept.in_order] {
            self.order[self.in_order] = k;
            self.in_order += 1;
            let s = &mut self.slots[k as usize];
            s.presence = 0.0;
            s.leaving = false;
        }
        self.shapes[i].order[0] = a.slot;
        self.shapes[i].in_order = 1;
    }

    /// Redo a merge: the layers fade out (kept at `a.index` again), and
    /// the merged layer fades in.
    fn remerge(&mut self, a: Act) {
        let i = a.index as usize;
        let n = self.in_order;
        self.shapes[i].order = self.order;
        self.shapes[i].in_order = n;
        for &k in &self.order[..n] {
            self.slots[k as usize].leaving = true;
        }
        self.restore(Act {
            kind: Kind::Overdub,
            slot: a.slot,
            index: n as u8,
            ..NO_ACT
        });
    }

    /// Undo the newest overdub, delete, edit or merge.
    pub(crate) fn undo(&mut self) {
        if self.n_done == 0 {
            return;
        }
        let mut a = self.take_done(self.n_done - 1);
        self.swap_ages(&mut a);
        self.push_undone(a);
        match a.kind {
            Kind::Delete => self.restore(a),
            Kind::Overdub => self.slots[a.slot as usize].leaving = true,
            // The last layer: it made the loop, which goes with it.
            Kind::Added if self.layers() <= 1 => {
                self.od_open = false;
                self.clearing = true;
                self.keep_clear = false;
            }
            Kind::Added => self.slots[a.slot as usize].leaving = true,
            Kind::Edit | Kind::Snap | Kind::Bars => self.swap_shape(a.slot as usize, a.fades()),
            Kind::Merge => self.unmerge(a),
            Kind::Fit => self.start_swap(a.index as usize),
        }
    }

    /// Redo what was undone last.
    pub(crate) fn redo(&mut self) {
        self.n_undone -= 1;
        let mut a = self.undone[self.n_undone];
        self.swap_ages(&mut a);
        self.push_done(a);
        match a.kind {
            Kind::Delete => self.slots[a.slot as usize].leaving = true,
            Kind::Overdub | Kind::Added => self.restore(a),
            Kind::Edit | Kind::Snap | Kind::Bars => self.swap_shape(a.slot as usize, a.fades()),
            Kind::Merge => self.remerge(a),
            Kind::Fit => self.start_swap(a.index as usize),
        }
    }

    /// Fade old, once: every layer but the one being recorded turns down
    /// (gliding over a few ms, as a level change does). Below -80 dB it is
    /// silent.
    pub(crate) fn fade_older(&mut self) {
        self.touch();
        self.od_faded = true;
        let k = self.od_layer;
        for &s in &self.order[..self.in_order] {
            if s as usize != k {
                let a = &mut self.slots[s as usize].age;
                *a *= self.fade_old;
                if *a < 1e-4 {
                    *a = 0.0;
                }
            }
        }
        self.mixing = true;
    }

    /// An overdub's undo or redo: the layers' Fade old turn-down goes back
    /// to the other side of it, and the act keeps this side.
    pub(crate) fn swap_ages(&mut self, a: &mut Act) {
        if a.kind != Kind::Overdub {
            return;
        }
        for (x, s) in a.ages.iter_mut().zip(self.slots.iter_mut()) {
            std::mem::swap(x, &mut s.age);
        }
        self.mixing = true;
    }

    /// Put an act's layer back in its place, fading in.
    fn restore(&mut self, a: Act) {
        self.mixing = true;
        let at = (a.index as usize).min(self.in_order);
        self.order.copy_within(at..self.in_order, at + 1);
        self.order[at] = a.slot;
        self.in_order += 1;
        let s = &mut self.slots[a.slot as usize];
        s.presence = 0.0;
        s.leaving = false;
    }

    /// Take the layer at place `at` out of the play order.
    pub(crate) fn remove_at(&mut self, at: usize) {
        self.mixing = true;
        self.order.copy_within(at + 1..self.in_order, at);
        self.in_order -= 1;
    }

    /// Remember an act for Undo; the oldest is forgotten when the history
    /// is full.
    pub(crate) fn push_done(&mut self, mut a: Act) {
        // A new step takes a transaction; one undone and redone keeps its.
        if !a.txn.is_some() {
            a.txn = TransactionId::next();
        }
        if self.n_done == HISTORY {
            self.let_go = self.take_done(0).txn;
        }
        self.done[self.n_done] = a;
        self.n_done += 1;
    }

    pub(crate) fn push_undone(&mut self, a: Act) {
        if self.n_undone == HISTORY {
            self.undone.copy_within(1..HISTORY, 0);
            self.n_undone -= 1;
        }
        self.undone[self.n_undone] = a;
        self.n_undone += 1;
    }

    pub(crate) fn take_done(&mut self, i: usize) -> Act {
        let a = self.done[i];
        self.done.copy_within(i + 1..self.n_done, i);
        self.n_done -= 1;
        a
    }
}
