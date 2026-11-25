//! Layers and their history: what each loop holds, its steps, and
//! the layers sent away to an audio track and brought back.

use super::*;

impl Looper {
    /// The memory gauge.
    pub fn memory(&self) -> Memory {
        Memory {
            used: (self.store.n_blocks - self.store.available()) * BLOCK,
            capacity: self.capacity,
        }
    }

    /// Layer `k` of the selected loop, counting in play order: its audio,
    /// for drawing the waveform. None if there is no such layer.
    pub fn layer(&self, k: usize) -> Option<LayerView<'_>> {
        self.loop_layer(self.selected, k)
    }

    /// Layer `k` of loop `n`, counting in play order.
    pub fn loop_layer(&self, n: usize, k: usize) -> Option<LayerView<'_>> {
        let l = self.loops.get(n)?;
        let slot = l.listed(k)?;
        let valid = l.slots[slot].valid;
        (l.len > 0).then_some(LayerView {
            store: &self.store,
            lp: n,
            slot,
            len: l.len,
            valid,
            view: l.views[slot],
        })
    }

    /// Loop `n`'s first pass as far as it has recorded, while it records
    /// ([`Status::recorded`] frames), for a waveform that grows as it
    /// records. None before the first frame and once the pass has ended.
    pub fn first_pass(&self, n: usize) -> Option<LayerView<'_>> {
        let l = self.loops.get(n)?;
        let kept = KEEP_BEFORE + l.written;
        let view = View::region(kept, KEEP_BEFORE, l.written, KEEP_BEFORE)?;
        (l.phase == Phase::Recording).then_some(LayerView {
            store: &self.store,
            lp: n,
            slot: 0,
            len: l.written,
            valid: (0, kept),
            view,
        })
    }

    /// Loop `n`'s first pass as it was kept, not through its edits:
    /// frames `from..from + out.len()` of the audio it kept (frame 0 its
    /// first, [`KEEP_BEFORE`] before Record), silence outside it; and where
    /// the loop is in it now: its first frame and its length. For the loop
    /// point view, which draws your press, your tap and what smart loop
    /// end found, with the audio round them. None (`out` silent) unless
    /// the first pass plays alone as a stretch of what it kept (an
    /// overdub, a cut or a merge ends that). Allocation-free.
    pub fn read_kept(&self, n: usize, from: i64, out: &mut [[f32; 2]]) -> Option<(usize, usize)> {
        out.fill([0.0; 2]);
        let l = self.loops.get(n)?;
        let (start, _) = l.bounds_now()?;
        let size = l.views[0].size();
        let a = from.clamp(0, size as i64) as usize;
        let b = from.saturating_add(out.len() as i64).clamp(0, size as i64) as usize;
        if b > a {
            let at = (a as i64 - from) as usize;
            LayerView {
                store: &self.store,
                lp: n,
                slot: 0,
                len: b - a,
                valid: l.slots[0].valid,
                view: View::region(size, a, b - a, a)?,
            }
            .read(0, &mut out[at..at + (b - a)]);
        }
        Some((start, l.len))
    }

    /// The selected loop's history, for the Looper tab's undo history.
    pub fn history(&self) -> History {
        self.loop_history(self.selected)
            .expect("the selected loop exists")
    }

    /// Loop `n`'s history. None if there is no such loop.
    pub fn loop_history(&self, n: usize) -> Option<History> {
        let l = self.loops.get(n)?;
        let mut h = History {
            done: [Step::Merge(0); HISTORY],
            n_done: l.n_done,
            redo: [Step::Merge(0); HISTORY],
            n_redo: l.n_undone,
        };
        for (to, a) in h.done.iter_mut().zip(&l.done[..l.n_done]) {
            *to = a.step();
        }
        for (to, a) in h.redo.iter_mut().zip(l.undone[..l.n_undone].iter().rev()) {
            *to = a.step();
        }
        Some(h)
    }

    /// Loop `n`'s steps by their transactions (audio tracks phase 4d):
    /// those Undo takes back into `done`, oldest first, and those Redo
    /// brings back into `redo`, the next first, as of the last period.
    /// Each step keeps its transaction through Undo and Redo. A loop
    /// cleared and kept for Undo lists the steps it was cleared with and
    /// then its Clear (`step` None), which Undo takes back next; and the
    /// steps it had undone (Redo brings them back once it is back). Both
    /// lists are cleared first; they allocate only to grow. Returns the
    /// newest step's transaction the loop's history let go of from its
    /// oldest end (none if it never did).
    pub fn loop_steps(
        &self,
        n: usize,
        done: &mut Vec<StepTxn>,
        redo: &mut Vec<TransactionId>,
    ) -> TransactionId {
        done.clear();
        redo.clear();
        let Some(l) = self.loops.get(n) else {
            return TransactionId::NONE;
        };
        let of = |a: &Act| StepTxn {
            txn: a.txn,
            step: Some(a.step()),
        };
        if l.phase == Phase::Empty && self.store.has_kept(n) {
            let k = &self.store.kept[n];
            done.extend(k.done[..k.n_done].iter().map(of));
            done.push(StepTxn {
                txn: self.store.kept_txn[n],
                step: None,
            });
            redo.extend(k.undone[..k.n_undone].iter().rev().map(|a| a.txn));
            return l.let_go.max(k.let_go);
        }
        done.extend(l.done[..l.n_done].iter().map(of));
        redo.extend(l.undone[..l.n_undone].iter().rev().map(|a| a.txn));
        l.let_go
    }

    /// Layer `k` of the selected loop's id and mix, counting in play order.
    /// None if there is no such layer.
    pub fn layer_info(&self, k: usize) -> Option<LayerInfo> {
        self.loop_layer_info(self.selected, k)
    }

    /// The layer slot `slot` of loop `n` holds ([`LayerInfo::id`] names
    /// the slot, [`LayerInfo::born`] the layer), listed in play order or
    /// not: a loop being loaded puts layer `k` in slot `k`, numbered
    /// before it is listed. 0 for none yet.
    pub fn slot_born(&self, n: usize, slot: usize) -> Option<u64> {
        Some(self.loops.get(n)?.slots.get(slot)?.born)
    }

    /// Whether loop `n` still holds the audio of the layer numbered `born`
    /// ([`LayerInfo::born`]) in slot `slot`: in play order, kept for its
    /// Undo or Redo, or in the loop a Clear kept for Undo. False once its
    /// blocks have gone (the slot made into another layer, the cleared
    /// loop forgotten). Audio tracks phase 4c.6: the host keeps the
    /// layer's recording named while this holds. A layer sent away
    /// (phase 4d.4) is held still: its audio is its file, which UNDO
    /// brings back. Does not allocate.
    pub fn holds_layer(&self, n: usize, slot: usize, born: u64) -> bool {
        let (Some(l), true) = (self.loops.get(n), slot < MAX_LAYERS && born != 0) else {
            return false;
        };
        let st = &self.store;
        let has = st.held[n][slot] > 0 || st.is_away(n, slot);
        let live = l.slots[slot].born == born && has;
        let kept = (st.kept_holds(n, slot) || st.is_away(n, slot))
            && st.has_kept(n)
            && st.kept[n].slots[slot].born == born;
        live || kept
    }

    /// The app says loop `n`'s layer numbered `born` is written whole to
    /// its file and synced (audio tracks phase 4d.4): from now on it may
    /// leave memory while only history holds it, when memory is needed
    /// for a new layer, rather than a step being let go. Nothing if the
    /// loop has no such layer.
    pub fn set_synced(&mut self, n: usize, born: u64) {
        let Some(l) = self.loops.get(n) else {
            return;
        };
        if let Some(k) = (0..MAX_LAYERS).find(|&k| born != 0 && l.slots[k].born == born) {
            self.store.synced[n][k] = born;
        }
    }

    /// Whether loop `n`'s layer numbered `born` is away: its blocks gone
    /// back, its audio in its file (phase 4d.4).
    pub fn is_away(&self, n: usize, born: u64) -> bool {
        let Some(l) = self.loops.get(n) else {
            return false;
        };
        (0..MAX_LAYERS).any(|k| born != 0 && l.slots[k].born == born && self.store.is_away(n, k))
    }

    /// The layer the next Undo (or Redo, `redo`) of loop `n` would bring
    /// back that is away, its own or a tied loop's, as (loop, slot, its
    /// number): that Undo is refused until it is brought back
    /// ([`Looper::begin_back`]). None if it needs none (phase 4d.4).
    pub fn away_for(&self, n: usize, redo: bool) -> Option<(usize, usize, u64)> {
        if n >= MAX_LOOPS {
            return None;
        }
        let c = if redo { Command::Redo } else { Command::Undo };
        let (lp, k) = self.needs_back(n, c)?;
        Some((lp, k, self.loops[lp].slots[k].born))
    }

    /// How many of loop `n`'s next `want` Undos (or Redos, `redo`), from
    /// its history as it is now, need no layer that is away (phase 4d.4):
    /// the Looper tab's history list goes that far, then waits for the
    /// layer to be brought back. Does not allocate.
    pub fn steps_before_away(&self, n: usize, redo: bool, want: usize) -> usize {
        let Some(l) = self.loops.get(n) else {
            return 0;
        };
        let acts = if redo {
            &l.undone[..l.n_undone]
        } else {
            &l.done[..l.n_done]
        };
        acts.iter()
            .rev()
            .take(want)
            .take_while(|a| {
                (0..MAX_LAYERS).all(|k| !(self.store.is_away(n, k) && l.brings_back(a, k)))
            })
            .count()
    }

    /// Whether loop `n`'s Undo (or Redo, `redo`) would wait now: a layer
    /// fading out, a crossfade, a clear (phase 4d.4: a layer is brought
    /// back for it once nothing is fading, so that a layer an Undo just
    /// took out of the play order can make room).
    pub fn step_waits(&self, n: usize, redo: bool) -> bool {
        let c = if redo { Command::Redo } else { Command::Undo };
        self.loops
            .get(n)
            .is_some_and(|l| l.must_wait(Queued::Loop(c)))
    }

    /// Undos and Redos in the command queue, not applied yet.
    pub fn steps_queued(&self) -> usize {
        self.queue[..self.queued]
            .iter()
            .filter(|q| matches!(q, Queued::Loop(Command::Undo | Command::Redo)))
            .count()
    }

    /// Start bringing loop `n`'s away slot `slot` back (phase 4d.4): it
    /// takes the blocks it held again, and [`Looper::put_back`] hands in
    /// its frames from its file, as [`Looper::raw_of`] gave them. To make
    /// room, other synced layers only history holds leave memory first
    /// (Undo's oldest, then Redo's farthest, in any loop), never one the
    /// same step brings back with it. Gives the frames to
    /// bring (the layer's size). Refused while another is being brought
    /// back, for a slot not away, or with no room even so.
    pub fn begin_back(&mut self, n: usize, slot: usize) -> Result<usize, &'static str> {
        if n >= MAX_LOOPS || slot >= MAX_LAYERS || !self.store.is_away(n, slot) {
            return Err("that layer isn't away");
        }
        if self.store.back.is_some() {
            return Err("another layer is being brought back");
        }
        let need = self.store.away_blocks[n][slot] as usize;
        let skip = self.loops[n].needs_with(slot);
        while self.store.available() < need {
            let sent = (0..MAX_LOOPS).any(|m| {
                let skip = if m == n { skip } else { 0 };
                self.loops[m].send_away(&mut self.store, m, skip, true)
            });
            if !sent {
                break;
            }
        }
        let size = self.loops[n].views[slot].size();
        self.store.back = Some(Back {
            lp: n,
            slot,
            done: 0,
            size,
        });
        if self.store.available() < need || !self.store.grow(n, slot, need) {
            self.store.back = None;
            self.store.shrink(n, slot, 0);
            return Err("there is no room in the loops' memory");
        }
        Ok(size)
    }

    /// Whether a layer is being brought back ([`Looper::begin_back`]).
    /// False once it is all in, or once it was called off: its step let
    /// go, its loop forgotten or loaded over.
    pub fn bringing_back(&self) -> bool {
        self.store.back.is_some()
    }

    /// The next frames (interleaved stereo) of the layer being brought
    /// back ([`Looper::begin_back`]), in order from its first. True once
    /// it is all in: it is the loop's again, and Undo can bring it back.
    /// Frames past its size are left out. Does not allocate.
    pub fn put_back(&mut self, frames: &[f32]) -> bool {
        let Some(mut b) = self.store.back else {
            return false;
        };
        for x in frames.chunks_exact(2) {
            if b.done >= b.size {
                break;
            }
            self.store.set(b.lp, b.slot, b.done, [x[0], x[1]]);
            b.done += 1;
        }
        if b.done < b.size {
            self.store.back = Some(b);
            return false;
        }
        self.store.back = None;
        self.store.away[b.lp] &= !(1 << b.slot);
        true
    }

    /// Stop bringing a layer back (its file couldn't be read): its blocks
    /// go back again and it stays away.
    pub fn call_back_off(&mut self) {
        if let Some(b) = self.store.back.take() {
            self.store.release(b.lp, b.slot);
        }
    }

    /// The layers layer `k` of loop `n` (in play order) was made from by
    /// a merge or a fit, by their numbers ([`LayerInfo::born`]), in play
    /// order then, 0 after the last; all 0 for a layer recorded or
    /// loaded (audio tracks phase 4c.5: the host names their recordings
    /// as its sources). Does not allocate.
    pub fn layer_from(&self, n: usize, k: usize) -> Option<[u64; MAX_LAYERS]> {
        let l = self.loops.get(n)?;
        let slot = l.listed(k)?;
        let (born, from) = l.derived[slot];
        Some(match born == l.slots[slot].born && born != 0 {
            true => from,
            false => [0; MAX_LAYERS],
        })
    }

    /// Layer `k` of loop `n`, counting in play order.
    pub fn loop_layer_info(&self, n: usize, k: usize) -> Option<LayerInfo> {
        let l = self.loops.get(n)?;
        let slot = l.listed(k)?;
        let s = &l.slots[slot];
        let any_solo = l.any_solo();
        Some(LayerInfo {
            id: slot as u8,
            level: s.level,
            pan: s.pan,
            muted: s.mute,
            solo: s.solo,
            audible: !s.mute && (!any_solo || s.solo),
            recording: l.od_open && l.od_layer == slot,
            faded: s.age,
            replace: s.replace.is_some(),
            born: s.born,
            closed: l.slot_closed(slot),
        })
    }
}
