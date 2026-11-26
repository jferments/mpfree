//! Layers copied or moved between loops, and what waits on them.

use super::*;

impl Looper {
    /// Start a layer's copy between loops ([`Looper::copy_layer`]).
    pub(crate) fn start_copy(&mut self, from: usize, id: u8, to: usize, keep: bool) {
        let slot = id as usize;
        let listed = |l: &Loop| {
            slot < MAX_LAYERS && l.order[..l.in_order].contains(&id) && !l.slots[slot].leaving
        };
        if from >= MAX_LOOPS || to >= MAX_LOOPS || !listed(&self.loops[from]) {
            self.stats.no_layer += 1;
            return;
        }
        // Nothing changing the layers, nothing fading.
        let free = |l: &Loop| l.savable() && !l.busy() && !l.fading() && l.xf == 0;
        let (src, dst) = (&self.loops[from], &self.loops[to]);
        let empty = dst.phase == Phase::Empty
            && !dst.clearing
            && !dst.loading
            && !dst.adding
            && self.loads[to].is_none();
        // Onto a loop on the same clock (the same free clock, for free
        // loops) and as long; an empty loop becomes one like the source.
        let onto = free(dst)
            && dst.len_ticks == src.len_ticks
            && dst.clock == src.clock
            && (src.clock == Clock::Song || dst.clock_gen == src.clock_gen)
            && dst.in_order < MAX_LAYERS - 1;
        if from == to || !free(src) || !(empty || onto) {
            self.stats.refused += 1;
            return;
        }
        let s = src.slots[slot];
        let (len, ticks, origin, layers) = (src.len, src.len_ticks, src.origin, src.layers());
        let (speed, rate) = (src.speed, src.rate);
        let (clock, clock_gen) = (src.clock, src.clock_gen);
        let l = &mut self.loops[to];
        let k = if empty {
            // The layer's loop takes the place of one kept for Undo.
            self.store.drop_kept(to);
            l.reclaim(&mut self.store, to);
            let need = blocks_for(len);
            (self.store.available() >= need && self.store.grow(to, 0, need)).then_some(0)
        } else {
            l.free_slot(&mut self.store, to)
        };
        let Some(k) = k else {
            l.reclaim(&mut self.store, to);
            self.stats.refused += 1;
            return;
        };
        let frames = if empty { len } else { l.len };
        if empty {
            // It takes the layer's loop's speed with its length, and its
            // clock.
            l.speed = speed;
            l.rate = rate;
            l.clock = clock;
            l.clock_gen = clock_gen;
        }
        let mut new = Slot {
            valid: (0, frames),
            level: s.level,
            pan: s.pan,
            mute: s.mute,
            age: s.age,
            // Into a loop that plays, it fades in; an empty loop fades in
            // as a whole.
            presence: if empty { 1.0 } else { 0.0 },
            born: self.store.born(),
            ..Slot::NEW
        };
        (new.gl, new.gr) = new.target(!empty && l.any_solo());
        l.slots[k] = new;
        // In time with the song: this loop's frame 0 is where `from` is
        // at this loop's start.
        l.ad_shift = if empty {
            0.0
        } else {
            rem_ticks((l.origin - origin) as f64, ticks as f64) * len as f64 / ticks as f64
        };
        l.ad_rate = len as f64 / frames as f64;
        l.adding = true;
        l.ad_from = from;
        l.ad_src = slot;
        l.ad_slot = k;
        l.ad_frames = frames;
        l.ad_done = 0;
        l.ad_start = self.now;
        l.ad_at = self.now + frames.div_ceil(COPY_SPEED) as u64;
        l.ad_move = !keep && layers > 1;
        l.ad_ticks = ticks;
        l.ad_origin = origin;
        let lend = Lend {
            slot: id,
            to: to as u8,
            to_slot: k as u8,
            at: l.ad_at,
            drop: l.ad_move,
        };
        self.loops[from].lend = Some(lend);
    }

    /// Copy loop `k`'s layer in as far as the period ending before frame
    /// `end` is due: [`COPY_SPEED`] frames for each frame since it
    /// started. It reads the other loop's layer as it plays, between its
    /// frames when the lengths differ.
    pub(crate) fn render_copy(&mut self, k: usize, end: u64) {
        let l = &self.loops[k];
        let (from, src_slot, slot) = (l.ad_from, l.ad_src, l.ad_slot);
        let (shift, rate, frames) = (l.ad_shift, l.ad_rate, l.ad_frames);
        let due = (end - l.ad_start).saturating_mul(COPY_SPEED as u64);
        let target = frames.min(due.min(usize::MAX as u64) as usize);
        let mut done = l.ad_done;
        let src = &self.loops[from];
        let (valid, view, seam, len) = (
            src.slots[src_slot].valid,
            src.views[src_slot],
            src.seam,
            src.len,
        );
        if len == 0 {
            return;
        }
        while done < target {
            let n = RUN.min(target - done);
            let mut buf = [[0.0f32; 2]; RUN];
            let blocks = self.store.blocks_of(from, src_slot);
            for (j, y) in buf[..n].iter_mut().enumerate() {
                let x = ((done + j) as f64 * rate + shift).rem_euclid(len as f64);
                let i = (x as usize).min(len - 1);
                let frac = (x - i as f64) as f32;
                let a = layer_frame(&self.store, blocks, valid, &view, seam, i);
                *y = if frac == 0.0 {
                    a
                } else {
                    let b = layer_frame(&self.store, blocks, valid, &view, seam, (i + 1) % len);
                    [a[0] + (b[0] - a[0]) * frac, a[1] + (b[1] - a[1]) * frac]
                };
            }
            for (j, y) in buf[..n].iter().enumerate() {
                self.store.set(k, slot, done + j, *y);
            }
            done += n;
        }
        self.loops[k].ad_done = done;
    }

    /// Call off the layer copying into loop `t`, if any: its memory goes
    /// back, and the layer stays in its own loop.
    pub(crate) fn call_off_copy(&mut self, t: usize) {
        let l = &mut self.loops[t];
        if !l.adding {
            return;
        }
        l.adding = false;
        let from = l.ad_from;
        if self.loops[from].lend.is_some_and(|x| x.to as usize == t) {
            self.loops[from].lend = None;
        }
        self.loops[t].reclaim(&mut self.store, t);
    }

    /// The loop whose step Undo (or Redo) takes back (or brings back)
    /// with loop `s`'s: the two sides of a layer moved between them, each
    /// the last step of its loop, with both loops playing and free.
    pub(crate) fn tied(&self, s: usize, c: Command) -> Option<usize> {
        let top = |l: &Loop| -> Option<Act> {
            match c {
                Command::Undo => l.done[..l.n_done].last().copied(),
                _ => l.undone[..l.n_undone].last().copied(),
            }
        };
        let ready = |l: &Loop| {
            l.phase == Phase::Playing
                && !l.od_open
                && !l.busy()
                && !l.must_wait(Queued::Loop(Command::Undo))
        };
        let a = top(&self.loops[s])?;
        let link = a.link.filter(|k| k.tied)?;
        let o = link.lp as usize;
        let b = top(self.loops.get(o)?)?;
        let back = b
            .link
            .filter(|k| k.tied && k.lp as usize == s && k.slot == a.slot);
        (o != s
            && back.is_some()
            && b.slot == link.slot
            && ready(&self.loops[s])
            && ready(&self.loops[o]))
        .then_some(o)
    }

    /// The away slot Undo (or Redo) of loop `s` now would bring back, its
    /// own or a tied loop's: (loop, slot) ([`Looper::away_for`]).
    pub(crate) fn needs_back(&self, s: usize, c: Command) -> Option<(usize, usize)> {
        let redo = c == Command::Redo;
        let away = |n: usize| self.loops[n].top_away(&self.store, n, redo).map(|k| (n, k));
        away(s).or_else(|| self.tied(s, c).and_then(away))
    }

    /// One input, one recording: loop `s` starts recording, so the others
    /// stop. An overdub punches out, a first pass ends on the next bar line
    /// (on its bar line if free length), an armed loop disarms.
    pub(crate) fn others_stop_recording(&mut self, s: usize) {
        for (k, l) in self.loops.iter_mut().enumerate() {
            if k == s {
                continue;
            }
            match l.phase {
                Phase::Armed => {
                    l.phase = Phase::Empty;
                    l.unhear();
                }
                Phase::Recording => {
                    l.ending = true;
                    l.end_tick = None;
                    l.then_overdub = false;
                }
                _ if l.od_open => l.punch_out(),
                _ => {}
            }
            l.reclaim(&mut self.store, k);
        }
    }
}
