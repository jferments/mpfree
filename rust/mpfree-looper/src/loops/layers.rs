//! Which layers play and which can come back, the layers sent away
//! to an audio track, and the loop's place as the song moves.

use super::*;

impl Loop {
    /// Layers in play order, not counting ones fading away.
    pub(crate) fn layers(&self) -> usize {
        self.order[..self.in_order]
            .iter()
            .filter(|&&k| !self.slots[k as usize].leaving)
            .count()
    }

    /// The slot of the `k`th layer not fading away.
    pub(crate) fn listed(&self, k: usize) -> Option<usize> {
        self.order[..self.in_order]
            .iter()
            .map(|&s| s as usize)
            .filter(|&s| !self.slots[s].leaving)
            .nth(k)
    }

    pub(crate) fn any_solo(&self) -> bool {
        self.order[..self.in_order]
            .iter()
            .any(|&k| self.slots[k as usize].solo && !self.slots[k as usize].leaving)
    }

    /// Whether `slot` is playing, or Undo or Redo can bring it back.
    fn holds(&self, slot: usize) -> bool {
        self.live(slot)
            || self.undone[..self.n_undone]
                .iter()
                .any(|a| self.brings_back(a, slot))
    }

    /// Whether `slot` is playing, or Undo can bring it back.
    pub(crate) fn live(&self, slot: usize) -> bool {
        self.order[..self.in_order]
            .iter()
            .any(|&k| k as usize == slot)
            || self.done[..self.n_done]
                .iter()
                .any(|a| self.brings_back(a, slot))
    }

    /// Whether undoing or redoing `a` can bring back `slot`'s layer.
    pub(crate) fn brings_back(&self, a: &Act, slot: usize) -> bool {
        match a.kind {
            Kind::Edit | Kind::Snap | Kind::Bars => false,
            Kind::Overdub | Kind::Delete | Kind::Added => a.slot as usize == slot,
            Kind::Merge | Kind::Fit => {
                let kept = &self.shapes[a.index as usize];
                a.slot as usize == slot
                    || kept.order[..kept.in_order]
                        .iter()
                        .any(|&k| k as usize == slot)
            }
        }
    }

    /// The first pass is being recorded into slot 0.
    pub(crate) fn first_in_progress(&self) -> bool {
        matches!(self.phase, Phase::Recording) || self.first_open
    }

    /// Slot `k`'s audio is all written and never changes again (audio
    /// tracks phase 4c, rule 1): no first pass, overdub, merge, fit, load
    /// or layer copying in writes into it, and a first pass is done
    /// keeping what it hears after its loop ([`LayerInfo::closed`]).
    pub(crate) fn slot_closed(&self, k: usize) -> bool {
        !(matches!(self.phase, Phase::Recording | Phase::Armed)
            || (k == 0 && (self.first_open || self.tail))
            || (self.od_open && self.od_layer == k)
            || (self.merging && self.mg_slot == k)
            || (self.fitting && self.fit_slot == k)
            || (self.loading && k < self.ld_layers)
            || (self.adding && self.ad_slot == k))
    }

    /// The slot of the layer numbered `born` ([`LayerInfo::born`]), if
    /// the loop lists it.
    pub(crate) fn slot_of(&self, born: u64) -> Option<usize> {
        (0..self.layers())
            .filter_map(|k| self.listed(k))
            .find(|&s| born != 0 && self.slots[s].born == born)
    }

    /// Give back the memory of every slot that can no longer come back.
    pub(crate) fn reclaim(&self, store: &mut Store, me: usize) {
        for k in 0..MAX_LAYERS {
            let in_use = self.holds(k)
                || store.kept_holds(me, k)
                || (k == 0 && self.first_in_progress())
                || (self.od_open && self.od_layer == k)
                || (self.merging && self.mg_slot == k)
                || (self.fitting && self.fit_slot == k)
                || (self.loading && k < self.ld_layers)
                || (self.adding && self.ad_slot == k);
            if !in_use && store.held[me][k] > 0 {
                store.release(me, k);
            }
            // A slot sent away that no step names any more is gone for
            // good (a loop cleared and kept for Undo still names its own).
            let kept = store.has_kept(me) && store.kept[me].holds(k);
            if !in_use && !kept && store.is_away(me, k) {
                store.away[me] &= !(1 << k);
            }
        }
    }

    /// Whether slot `k` may be sent away now (audio tracks phase 4d.4): it
    /// holds blocks, its layer is closed and synced to its file, only
    /// history names it (not playing, nothing writing it), and the loop
    /// isn't one cleared and kept for Undo.
    fn may_send_away(&self, store: &Store, me: usize, k: usize) -> bool {
        store.held[me][k] > 0
            && !store.is_away(me, k)
            && !store.has_kept(me)
            && self.slots[k].born != 0
            && store.synced[me][k] == self.slots[k].born
            && self.slot_closed(k)
            && !self.order[..self.in_order].iter().any(|&o| o as usize == k)
            && !(self.od_open && self.od_layer == k)
            && !(self.merging && self.mg_slot == k)
            && !(self.fitting && self.fit_slot == k)
            && !(self.loading && k < self.ld_layers)
            && !(self.adding && self.ad_slot == k)
            && store.back.is_none_or(|b| b.lp != me || b.slot != k)
    }

    /// Send away one synced slot only history holds, the oldest step's
    /// first, then (with `redo_too`) the steps Redo brings back, the
    /// farthest first, never a slot in `skip`: its blocks go back, its
    /// audio stays in its file (audio tracks phase 4d.4). False if none
    /// may go.
    pub(crate) fn send_away(
        &self,
        store: &mut Store,
        me: usize,
        skip: u16,
        redo_too: bool,
    ) -> bool {
        let undone = if redo_too { self.n_undone } else { 0 };
        let mut acts = self.done[..self.n_done]
            .iter()
            .chain(&self.undone[..undone]);
        let k = acts.find_map(|a| {
            (0..MAX_LAYERS).find(|&k| {
                skip & (1 << k) == 0 && self.brings_back(a, k) && self.may_send_away(store, me, k)
            })
        });
        let Some(k) = k else {
            return false;
        };
        store.away_blocks[me][k] = store.held[me][k];
        store.release(me, k);
        store.away[me] |= 1 << k;
        true
    }

    /// The slots the next Undo or Redo that brings back `slot` would
    /// bring back with it (a merge's sources).
    pub(crate) fn needs_with(&self, slot: usize) -> u16 {
        let next = self.done[..self.n_done].last().into_iter();
        let next = next.chain(self.undone[..self.n_undone].last());
        let mut mask = 0;
        for a in next.filter(|a| self.brings_back(a, slot)) {
            for k in 0..MAX_LAYERS {
                if self.brings_back(a, k) {
                    mask |= 1 << k;
                }
            }
        }
        mask
    }

    /// The away slot the next Undo (or Redo, `redo`) would bring back, if
    /// it would bring one back ([`Looper::away_for`]).
    pub(crate) fn top_away(&self, store: &Store, me: usize, redo: bool) -> Option<usize> {
        let free = self.phase == Phase::Playing && !self.od_open;
        let a = if redo {
            self.undone[..self.n_undone].last().filter(|_| free)
        } else {
            let stops = self.merging || self.fitting || self.print_wait;
            self.done[..self.n_done].last().filter(|_| free && !stops)
        }?;
        (0..MAX_LAYERS).find(|&k| store.is_away(me, k) && self.brings_back(a, k))
    }

    pub(crate) fn must_wait(&self, q: Queued) -> bool {
        match q {
            // A capture: again, or Undo, calls it off; the rest waits.
            Queued::Loop(Command::Capture(_) | Command::CaptureTo(..) | Command::Undo)
                if self.capturing() =>
            {
                false
            }
            _ if self.capture.is_some() => true,
            // Everything waits for a bounce (Clear stops it) and a load
            // (Clear calls it off).
            _ if self.bouncing || self.loading => true,
            // A layer copying in: Undo calls it off; the rest waits, as it
            // does while one of the loop's layers is copied out.
            Queued::Loop(Command::Undo) if self.adding => false,
            _ if self.adding || self.lend.is_some() => true,
            Queued::Loop(Command::Undo) => {
                self.leaving() || self.clearing || self.xf > 0 || self.swapping
            }
            // A new overdub, an edit or a merge waits while the last
            // overdub's tail is written, so that tail still fades out.
            Queued::Loop(
                Command::Record | Command::Overdub | Command::Edit(_) | Command::Merge,
            ) if self.od_open && self.phase == Phase::Playing => true,
            // Everything else waits for a merge or a fit (Undo stops
            // them), and for a fit's undo or redo to swap.
            _ if self.merging || self.fitting || self.swapping => true,
            _ => self.fading() || self.clearing || self.xf > 0,
        }
    }

    /// Merging, fitting, being bounced out or copying a layer in or out:
    /// the layers must not change.
    /// Nothing under way that its steps or their memory are part of (a
    /// merge, a fit or its swap, a bounce, a load, a copy, a Clear, a
    /// capture): its history can be cut down now ([`Command::ForgetRedo`],
    /// [`Command::RetireThrough`]).
    pub(crate) fn settled(&self) -> bool {
        !self.busy() && !self.loading && !self.clearing && self.capture.is_none()
    }

    pub(crate) fn busy(&self) -> bool {
        self.snap.is_some()
            || self.merging
            || self.bouncing
            || self.fitting
            || self.swapping
            || self.adding
            || self.lend.is_some()
    }

    /// A layer is fading out of the play order.
    fn leaving(&self) -> bool {
        self.order[..self.in_order]
            .iter()
            .any(|&k| self.slots[k as usize].leaving)
    }

    /// A layer is fading in or out.
    pub(crate) fn fading(&self) -> bool {
        self.order[..self.in_order]
            .iter()
            .any(|&k| self.slots[k as usize].leaving || self.slots[k as usize].presence < 1.0)
    }

    /// Move the loop's position on a frame; `tick` is that frame's song
    /// position. While playing it is the song position's place in the
    /// loop; while not, it runs on so the sound fades out where it was
    /// going, then, once silent, waits at the song position's place (where
    /// the transport will start again), so the playhead stops with the
    /// transport. A jump starts a crossfade.
    pub(crate) fn follow(&mut self, tick: f64, playing: bool, first: bool) {
        if self.len == 0 {
            return;
        }
        let len = self.len as f64;
        let was = self.pos;
        let was_p = self.ppos;
        if self.p_frames > 0 {
            self.follow_print(tick, playing);
        }
        if self.jump > 0 {
            self.jump -= 1;
            self.jump_pos = (self.jump_pos + 1.0) % len;
        }
        if self.xf > 0 {
            self.xf -= 1;
            let old = &self.shapes[self.xf_shape];
            let old_len = old.len as f64;
            self.xf_pos = if playing {
                let rel = rem_ticks(tick - old.origin as f64, old.len_ticks as f64);
                (rel / old.len_ticks as f64 * old_len).min(old_len - 1e-6)
            } else {
                (self.xf_pos + 1.0) % old_len
            };
        }
        if !playing && self.gain > 0.0 {
            self.pos = (was + 1.0) % len;
            return;
        }
        let rel = rem_ticks(tick - self.origin as f64, self.len_ticks as f64);
        self.pos = (rel / self.len_ticks as f64 * len).min(len - 1e-6);
        if !playing {
            return;
        }
        // Fade old: the loop came round (forwards, a frame or so on, not a
        // jump) while overdubbing.
        if self.fade_old < 1.0
            && self.phase == Phase::Overdubbing
            && self.pos < was
            && len - (was - self.pos) <= 4.0
        {
            self.fade_older();
        }
        // Within a span the song position is continuous; a jump can only
        // come at a span's first frame. (An edit's crossfade covers the
        // change of place an edit makes.)
        if first && self.gain > 0.0 && self.xf == 0 {
            let mut d = (self.pos - was).rem_euclid(len);
            if d > len / 2.0 {
                d -= len;
            }
            if d.abs() > 4.0 {
                self.jump = FADE;
                self.jump_pos = (was + 1.0) % len;
                if self.p_frames > 0 {
                    self.pjump_pos = (was_p + 1.0) % self.p_frames as f64;
                }
            }
        }
    }

    /// The print's place, as `follow` moves the loop's (and its jump's).
    #[cold]
    fn follow_print(&mut self, tick: f64, playing: bool) {
        let pf = self.p_frames as f64;
        if self.jump > 0 {
            self.pjump_pos = (self.pjump_pos + 1.0) % pf;
        }
        // As the loop's: on while it fades out, then at the song position.
        self.ppos = if playing || self.gain == 0.0 {
            let rel = rem_ticks(tick - self.p_origin as f64, self.p_ticks as f64);
            (rel / self.p_ticks as f64 * pf).min(pf - 1e-6)
        } else {
            (self.ppos + 1.0) % pf
        };
    }
}
