//! Overdubs: their slots, their writer, punching out and the song
//! jumping under them.

use super::*;

impl Loop {
    /// A slot for a new overdub, with the memory for a whole pass: one no
    /// layer plays and Undo cannot bring back (what Redo could bring back
    /// goes, as a new overdub ends redo). With no room, the history up to
    /// its oldest delete or merge is forgotten, so that memory can be used.
    /// None if there is still no room.
    pub(crate) fn free_slot(&mut self, store: &mut Store, me: usize) -> Option<usize> {
        self.free_slot_for(store, me, self.len)
    }

    /// A free slot with memory for `frames` frames (see `free_slot`).
    pub(crate) fn free_slot_for(
        &mut self,
        store: &mut Store,
        me: usize,
        frames: usize,
    ) -> Option<usize> {
        let need = blocks_for(frames);
        loop {
            // Blocks free now, plus what dropping redo would give back.
            let redo_blocks: usize = (0..MAX_LAYERS)
                .filter(|&k| !self.live(k))
                .map(|k| store.held[me][k] as usize)
                .sum();
            let slot = (0..MAX_LAYERS).find(|&k| !self.live(k));
            if let Some(k) = slot.filter(|_| store.available() + redo_blocks >= need) {
                self.n_undone = 0;
                self.reclaim(store, me);
                if k == 0 {
                    // The first pass is gone for good: its writer stops.
                    self.tail = false;
                }
                return store.grow(me, k, need).then_some(k);
            }
            // A synced layer only history holds leaves memory before any
            // step is let go (audio tracks phase 4d.4).
            if self.send_away(store, me, 0, false) {
                continue;
            }
            let i = self.done[..self.n_done]
                .iter()
                .position(|a| matches!(a.kind, Kind::Delete | Kind::Merge | Kind::Fit))?;
            self.let_go = self.done[i].txn;
            self.done.copy_within(i + 1..self.n_done, 0);
            self.n_done -= i + 1;
        }
    }

    /// A new layer on top, written from now on, fading in over `fade`
    /// frames.
    pub(crate) fn open_overdub(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        fade: usize,
    ) {
        self.mixing = true;
        // One slot is kept back, so that a merge always has one.
        if self.in_order >= MAX_LAYERS - 1 {
            stats.refused += 1;
            return;
        }
        let Some(k) = self.free_slot(store, me) else {
            stats.refused += 1;
            return;
        };
        self.slots[k] = Slot {
            born: store.born(),
            ..Slot::NEW
        };
        self.order[self.in_order] = k as u8;
        self.views[k] = View::whole(self.len);
        let mut ages = [1.0; MAX_LAYERS];
        for (a, s) in ages.iter_mut().zip(&self.slots) {
            *a = s.age;
        }
        self.push_done(Act {
            kind: Kind::Overdub,
            slot: k as u8,
            index: self.in_order as u8,
            edit: None,
            ages,
            link: None,
            shift: Shift::NONE,
            bars: 0,
            txn: TransactionId::NONE,
        });

        self.in_order += 1;
        self.od_open = true;
        self.od_handoff = false;
        self.od_layer = k;
        self.od_in = 0;
        self.od_q = 0;
        self.od_y = [0.0; 2];
        self.od_rise = FADE;
        self.od_fresh_from = 0;
        self.od_fade = fade;
        self.od_stop_in = None;
        self.od_once = self.overdubs == Overdubs::Once;
        // One pass ends with a fade as long as its start's: the two
        // crossfade where they meet.
        self.od_out_len = if self.od_once { fade.max(1) } else { FADE };
        if self.overdubs == Overdubs::Replace {
            self.slots[k].replace = Some((fade.max(1) as u32, self.od_out_len as u32));
        }
        self.od_out = self.od_out_len;
        self.od_peak = 0.0;
        self.od_faded = false;
        self.phase = Phase::Overdubbing;
    }

    /// End the overdub: the writer goes on for `latency` frames (that input
    /// belongs before the press), then fades out.
    pub(crate) fn punch_out(&mut self) {
        if self.od_stop_in.is_none() {
            self.od_stop_in = Some(self.latency);
        }
        self.phase = Phase::Playing;
    }

    /// An overdub that never wrote a frame leaves no layer behind, and
    /// nothing to undo. True if it was dropped.
    /// Throw the overdub away (recording, or still writing its tail): it
    /// stops and fades out, and Redo can bring back what it recorded.
    pub(crate) fn undo_open_overdub(&mut self) {
        self.od_open = false;
        self.phase = Phase::Playing;
        let slot = self.od_layer;
        if !self.drop_empty_overdub() {
            if let Some(i) = self.done[..self.n_done]
                .iter()
                .rposition(|a| a.kind == Kind::Overdub && a.slot as usize == slot)
            {
                let mut a = self.take_done(i);
                self.swap_ages(&mut a);
                self.push_undone(a);
            }
            self.slots[slot].leaving = true;
        }
    }

    fn drop_empty_overdub(&mut self) -> bool {
        if self.slots[self.od_layer].valid.1 != 0 {
            return false;
        }
        self.drop_overdub();
        true
    }

    /// Take the overdub's layer out of the play order and its step out of
    /// the history.
    fn drop_overdub(&mut self) {
        let k = self.od_layer;
        if let Some(at) = self.order[..self.in_order]
            .iter()
            .position(|&s| s as usize == k)
        {
            self.remove_at(at);
        }
        if let Some(i) = self.done[..self.n_done]
            .iter()
            .rposition(|a| a.kind == Kind::Overdub && a.slot as usize == k)
        {
            self.take_done(i);
        }
    }

    fn close_overdub(&mut self, store: &mut Store, me: usize) {
        self.mixing = true;
        self.touch();
        self.od_open = false;
        // An overdub that took only silence (below [`SILENT`]) and faded
        // nothing leaves no layer and no step to undo. It goes at once:
        // what it holds is too quiet to click.
        // A Replace of silence stays: it silences what was there.
        let replace = self.slots[self.od_layer].replace.is_some();
        if !self.drop_empty_overdub() && self.od_peak < SILENT && !self.od_faded && !replace {
            self.drop_overdub();
        }
        if self.phase == Phase::Overdubbing {
            self.phase = Phase::Playing;
        }
        self.reclaim(store, me);
    }

    /// One frame of the overdub writer. It waits for the first pass's
    /// writer to finish (it runs `latency` frames behind, so this input is
    /// still the first pass), and while the transport is stopped (Overdub
    /// pressed before Play); it fades out when the transport stops.
    pub(crate) fn overdub_frame(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        x: [f32; 2],
        playing: bool,
    ) {
        if self.first_open {
            return;
        }
        if !playing {
            if self.od_in == 0 {
                if self.od_stop_in.is_some() {
                    // Punched out before it began.
                    self.close_overdub(store, me);
                }
                return;
            }
            self.od_stop_in = Some(0);
        }
        self.write_overdub(store, stats, me, x);
    }

    /// Smart loop end's Auto with Then: Overdub, at the overdub's first
    /// frame (`q` in the loop, input `x`, `y` as the overdub's fade-in
    /// would write it): what the first pass's writer heard after the
    /// loop's end before this frame (you played on while Auto listened)
    /// becomes the overdub's first moments, written once just before `q`,
    /// where the overdub would have put them, fading in as the overdub
    /// would have; this frame follows at the gain after them. The pass's
    /// writer, still keeping what it hears after the end (`tail`), has
    /// just written this frame's input as its own last frame, so that
    /// frame says where it is. Nothing is handed on if the pass didn't
    /// keep it (its memory was short, [`Stats::kept_short`], or it heard
    /// too long past the end). Out of line, in the writer's fresh-layer
    /// branch: no frame but this one pays for it.
    #[cold]
    #[inline(never)]
    fn hand_off(&mut self, store: &mut Store, me: usize, q: usize, x: [f32; 2], y: [f32; 2]) {
        self.od_handoff = false;
        let (len, k) = (self.len, self.od_layer);
        let (v0, v1) = self.slots[0].valid;
        // The pass's frame for this input, and the frames after the
        // loop's end before it.
        let now = v1.saturating_sub(KEEP_BEFORE + 1);
        let first = self.region.map_or(0, |r| r.0.saturating_sub(KEEP_BEFORE));
        let h = now.saturating_sub(first + len).min(len.saturating_sub(1));
        if !self.tail || v0 != 0 || h == 0 {
            self.slots[k].valid = (q, 1);
            store.set(me, k, q, y);
            return;
        }
        let fade = self.od_fade;
        let gain = |i: usize| {
            if i < fade {
                (i as f32 + 0.5) / fade as f32
            } else {
                1.0
            }
        };
        let at = (q + len - h) % len;
        for i in 0..h {
            let a = store.get(me, 0, KEEP_BEFORE + now - h + i);
            let g = gain(i);
            self.od_peak = self.od_peak.max(a[0].abs()).max(a[1].abs());
            store.set(me, k, (at + i) % len, [a[0] * g, a[1] * g]);
        }
        let g = gain(h);
        store.set(me, k, q, [x[0] * g, x[1] * g]);
        self.slots[k].valid = (at, h + 1);
        self.od_in = h + 1;
        self.handoffs += 1;
    }

    /// The overdub writer's frame: input `x` on the loop frame `latency`
    /// before the playhead. A layer plays only the frames its overdub
    /// recorded (`Slot::valid`, one stretch from where it began), never
    /// what a reused block held before, so the writer goes from frame to
    /// frame of the loop, whatever the song's tempo: the song slower than
    /// the loop was recorded brings it to a frame again (the frame keeps
    /// the input it took first), faster takes it past frames (they take
    /// what lies between the inputs either side), and a jump goes to
    /// [`Loop::od_jump`]. At the loop's own tempo it moves a frame a frame.
    fn write_overdub(&mut self, store: &mut Store, stats: &mut Stats, me: usize, x: [f32; 2]) {
        let len = self.len;
        let q = ((self.pos - f64::from(self.latency))
            .rem_euclid(len as f64)
            .round() as usize)
            % len;
        // The loop frames on from the one it wrote last.
        let was = self.od_q;
        let mut step = if q >= was { q - was } else { q + len - was };
        if self.slots[self.od_layer].valid.1 > 0 && step > MAX_STEP {
            if !self.od_jump(store, stats, me, q) {
                return;
            }
            step = 1;
        }
        let k = self.od_layer;
        // One pass: it punches out `latency` frames early, so that its
        // fade-out starts on the frame it started on (counted in the loop
        // frames it has recorded).
        if self.od_once
            && self.od_stop_in.is_none()
            && self.slots[k].valid.1 >= len.saturating_sub(self.latency as usize).max(1)
        {
            self.punch_out();
        }
        let mut g = if self.od_in < self.od_fade {
            (self.od_in as f32 + 0.5) / self.od_fade as f32
        } else {
            1.0
        };
        // Fading in where the song jumped it to.
        if self.od_rise < FADE {
            g = g.min((self.od_rise as f32 + 0.5) * STEP);
            self.od_rise += 1;
        }
        self.od_in += 1;
        self.od_peak = self.od_peak.max(x[0].abs()).max(x[1].abs());
        let mut last = false;
        match self.od_stop_in {
            Some(0) => {
                g *= (self.od_out as f32 - 0.5) / self.od_out_len as f32;
                self.od_out -= 1;
                last = self.od_out == 0;
            }
            Some(left) => self.od_stop_in = Some(left - 1),
            None => {}
        }
        let y = [x[0] * g, x[1] * g];
        let first = self.slots[k].valid.1 == 0;
        if first {
            if self.od_handoff {
                self.hand_off(store, me, q, x, y);
            } else {
                self.slots[k].valid = (q, 1);
                store.set(me, k, q, y);
            }
        } else if step == 1 {
            self.od_put(store, me, q, y, g);
        } else if step > 1 {
            let y0 = self.od_y;
            for i in 1..step {
                let w = i as f32 / step as f32;
                let t = if was + i >= len {
                    was + i - len
                } else {
                    was + i
                };
                let v = [y0[0] + (y[0] - y0[0]) * w, y0[1] + (y[1] - y0[1]) * w];
                self.od_put(store, me, t, v, g);
            }
            self.od_put(store, me, q, y, g);
        }
        if first || step != 0 {
            self.od_q = q;
            self.od_y = y;
        }
        if last {
            self.close_overdub(store, me);
        }
    }

    /// Frame `t` of the overdub's layer takes `y`, its input at gain `g`:
    /// the frame after what it has recorded, or, once it has gone round
    /// the loop or the song took it back, a frame it recorded before, which
    /// it adds to (over a Replace's earlier passes it crossfades, and Fade
    /// old keeps their share).
    #[inline]
    fn od_put(&mut self, store: &mut Store, me: usize, t: usize, y: [f32; 2], g: f32) {
        let (len, k) = (self.len, self.od_layer);
        let (start, count) = self.slots[k].valid;
        if count < len {
            let end = if start + count >= len {
                start + count - len
            } else {
                start + count
            };
            if t == end {
                self.slots[k].valid = (start, count + 1);
                store.set(me, k, t, y);
                return;
            }
            // Back over what it recorded: the song jumped back.
            self.od_fresh_from = count;
        }
        if self.slots[k].replace.is_some() {
            // Replace: a later pass takes the place of the earlier ones,
            // crossfading into them as it fades out.
            let was = store.get(me, k, t);
            store.set(
                me,
                k,
                t,
                [was[0] * (1.0 - g) + y[0], was[1] * (1.0 - g) + y[1]],
            );
        } else if self.fade_old < 1.0 {
            // Fade old: this layer's earlier passes keep their share.
            let was = store.get(me, k, t);
            let f = self.fade_old;
            store.set(me, k, t, [was[0] * f + y[0], was[1] * f + y[1]]);
        } else {
            store.add(me, k, t, y);
        }
    }

    /// The song jumped under the overdub (a locate, or the song's loop
    /// going round where this loop doesn't fit it), to loop frame `q`.
    /// Into what it has recorded, or on from its end, it records on there,
    /// fading in and adding to what it recorded, and what it recorded last
    /// fades out where it was. Past what it has recorded, before it has
    /// gone round the loop, a layer would hold frames it never recorded:
    /// it ends where it was (faded out), and a new overdub records from
    /// the new place, as Overdub pressed twice would (Once, or punching
    /// out already, it just ends). Hardly begun (under a fade), it starts
    /// again at the new place. False if it ended and records nothing more.
    #[cold]
    #[inline(never)]
    fn od_jump(&mut self, store: &mut Store, stats: &mut Stats, me: usize, q: usize) -> bool {
        let (len, k) = (self.len, self.od_layer);
        let (start, count) = self.slots[k].valid;
        self.od_rise = 0;
        if count >= len {
            return true;
        }
        let d = if q >= start {
            q - start
        } else {
            q + len - start
        };
        let ending = self.od_stop_in.is_some();
        if d <= count && !ending {
            self.fade_back(store, me);
            return true;
        }
        if count < FADE && !ending {
            self.slots[k].valid = (0, 0);
            self.od_fresh_from = 0;
            return true;
        }
        self.fade_back(store, me);
        let again = !ending && !self.od_once;
        self.close_overdub(store, me);
        if again {
            self.open_overdub(store, stats, me, FADE);
        }
        self.od_open
    }

    /// What the overdub recorded last, up to where it is, fades out there,
    /// as a punch-out fades it: the song jumped it elsewhere. Only frames
    /// it recorded once (where it went back over what it recorded, what
    /// it added to is left as it is).
    #[cold]
    fn fade_back(&mut self, store: &mut Store, me: usize) {
        let (len, k) = (self.len, self.od_layer);
        let (start, count) = self.slots[k].valid;
        if count == 0 || count >= len || self.od_q != (start + count - 1) % len {
            return;
        }
        let n = self.od_out_len.min(count - self.od_fresh_from.min(count));
        let last = self.od_q;
        for i in 0..n {
            let t = if last >= i { last - i } else { last + len - i };
            let w = (i as f32 + 0.5) / n as f32;
            let v = store.get(me, k, t);
            store.set(me, k, t, [v[0] * w, v[1] * w]);
        }
    }
}
