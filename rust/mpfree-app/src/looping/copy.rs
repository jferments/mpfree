//! A whole loop copied into an empty one.

use super::*;

/// A whole loop copying into an empty one, a layer at a time
/// ([`LoopHost::copy_loop`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LoopCopy {
    from: usize,
    pub(crate) to: usize,
    /// The source's layers by id, in play order, and how many.
    ids: [u8; MAX_LAYERS],
    layers: usize,
    /// Layers asked for so far.
    asked: usize,
    /// Periods since the last layer was asked for with nothing to show
    /// for it (refused, or not started yet), and tries in all.
    waited: u32,
    tries: u32,
}

/// How a whole loop's copy ended ([`LoopHost::copy_ended`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CopyEnd {
    pub from: usize,
    pub to: usize,
    /// Layers that went in, of how many.
    pub layers: usize,
    pub of: usize,
}

impl LoopHost {
    /// Copy loop `from` into `to`, an empty loop: every layer, with its
    /// mix, and the loop's level (a loop cell copied onto another audio
    /// track). The copy is launched off, so it
    /// stays silent until its cell is launched. The layers copy in one at
    /// a time in the background while the loops play on
    /// ([`Looper::copy_layer`], each a step the looper's UNDO takes
    /// back); [`LoopHost::copy_ended`] says when it is all in or stopped.
    /// Refused, saying why, while another copy runs, when `from` isn't
    /// just playing, `to` isn't empty or the memory can't hold the
    /// layers.
    pub fn copy_loop(&mut self, from: usize, to: usize) -> Result<String, String> {
        if let Some(c) = self.copy {
            return Err(format!(
                "Loop {} is still copying into Loop {}: wait for it to finish",
                c.from + 1,
                c.to + 1
            ));
        }
        let (Some(a), Some(b)) = (self.looper.loop_status(from), self.looper.loop_status(to))
        else {
            return Err("No such loop".to_string());
        };
        let still = a.merging.is_none()
            && a.fitting.is_none()
            && a.loading.is_none()
            && a.copying.is_none();
        if from == to || a.phase != Phase::Playing || a.layers == 0 || !still {
            return Err(format!(
                "Loop {} can't be copied while it records or changes: try again when it just plays",
                from + 1
            ));
        }
        if b.phase != Phase::Empty || b.loading.is_some() || b.copying.is_some() {
            return Err(format!("Loop {} isn't empty", to + 1));
        }
        let m = self.looper.memory();
        let need = a.layers * a.frames + self.rate as usize;
        if m.capacity.saturating_sub(m.used) < need {
            return Err(format!(
                "The loops' memory can't hold a copy of Loop {} ({} layer{}): clear a loop on the Looper tab first",
                from + 1,
                a.layers,
                if a.layers == 1 { "" } else { "s" }
            ));
        }
        let mut ids = [0u8; MAX_LAYERS];
        let mut layers = 0;
        for (k, id) in ids.iter_mut().enumerate().take(a.layers) {
            if let Some(l) = self.looper.loop_layer_info(from, k) {
                *id = l.id;
                layers = k + 1;
            }
        }
        if layers == 0 || self.looper.room() < 4 {
            return Err(format!("Loop {} can't be copied now: try again", from + 1));
        }
        // Off until its cell is launched, at the source's level.
        self.looper.launch(to, false, None);
        self.looper.select_loop(to);
        self.looper.command(Command::SetLevel(a.level));
        self.looper.select_loop(self.picked);
        self.copy = Some(LoopCopy {
            from,
            to,
            ids,
            layers,
            asked: 0,
            waited: 0,
            tries: 0,
        });
        self.copy_end = None;
        Ok(format!(
            "looper: loop {} copies into loop {}, {} layer{}",
            from + 1,
            to + 1,
            layers,
            if layers == 1 { "" } else { "s" }
        ))
    }

    /// The end of the last whole loop's copy, once: all its layers in, or
    /// as many as went in before it stopped (none: the copy's loop is
    /// empty, so its cells should go).
    pub fn copy_ended(&mut self) -> Option<CopyEnd> {
        self.copy_end.take()
    }

    /// One period of a whole loop's copy: once the last layer asked for
    /// is in, the next is asked for. A layer that doesn't start (the
    /// loops were busy or fading) is asked for again, for up to two
    /// seconds; the copy stops then, or when its loop is cleared or
    /// undone, or its source no longer plays. Allocates nothing.
    pub(crate) fn tend_copy(&mut self) {
        let Some(mut c) = self.copy else {
            return;
        };
        let (Some(a), Some(b)) = (
            self.looper.loop_status(c.from),
            self.looper.loop_status(c.to),
        ) else {
            self.copy = None;
            return;
        };
        let have = if b.phase == Phase::Empty { 0 } else { b.layers };
        let ended = CopyEnd {
            from: c.from,
            to: c.to,
            layers: have,
            of: c.layers,
        };
        if b.copying.is_some() {
            c.waited = 0;
            self.copy = Some(c);
            return;
        }
        // All in; or cleared or undone under it: it stops with what it has.
        if have == c.layers || have + 1 < c.asked {
            self.copy = None;
            self.copy_end = Some(ended);
            return;
        }
        if c.asked > have {
            // Asked, but not started: it starts with the next period, or
            // it was refused.
            c.waited += 1;
            if c.waited <= 2 {
                self.copy = Some(c);
                return;
            }
            c.asked = have;
            c.tries += 1;
        }
        let limit = 2 * self.rate / PERIOD as u32;
        let gone = a.phase != Phase::Playing || (b.phase != Phase::Empty && b.ticks != a.ticks);
        if gone || c.tries > limit {
            self.copy = None;
            self.copy_end = Some(ended);
            return;
        }
        if self.looper.room() > 0 && self.looper.copy_layer(c.from, c.ids[have], c.to, true) {
            c.asked = have + 1;
            c.waited = 0;
        }
        self.copy = Some(c);
    }
}
