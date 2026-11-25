//! Edits: the loop's shape, Move, Turn and Resize, A/B and the
//! shapes Undo keeps.

use super::*;

impl Loop {
    /// The loop's shape now.
    pub(crate) fn shape(&self) -> Shape {
        Shape {
            len: self.len,
            len_ticks: self.len_ticks,
            origin: self.origin,
            seam: self.seam,
            region: self.region,
            speed: self.speed,
            rate: self.rate,
            views: self.views,
            order: self.order,
            in_order: self.in_order,
        }
    }

    pub(crate) fn set_shape(&mut self, s: &Shape) {
        self.len = s.len;
        self.len_ticks = s.len_ticks;
        self.origin = s.origin;
        self.seam = s.seam;
        self.region = s.region;
        self.speed = s.speed;
        self.rate = s.rate;
        self.views = s.views;
    }

    /// Make an edit, keeping the shape before it for Undo.
    pub(crate) fn edit(&mut self, stats: &mut Stats, e: Edit) {
        self.make_edit(stats, e, Kind::Edit);
    }

    /// Edit `e`, as a step of `kind` Undo takes back; false (counted) if it
    /// can't be made.
    fn make_edit(&mut self, stats: &mut Stats, e: Edit, kind: Kind) -> bool {
        let new = self.editable().then(|| self.edited(e)).flatten();
        self.take_shape(
            stats,
            new,
            Act {
                kind,
                edit: Some(e),
                ..NO_ACT
            },
        )
    }

    /// A snap's move ([`Kind::Snap`]), as a step Undo takes back; false
    /// (counted) if it can't be made.
    pub(crate) fn make_shift(&mut self, stats: &mut Stats, by: Shift) -> bool {
        let new = self.editable().then(|| self.shifted(by)).flatten();
        self.take_shape(
            stats,
            new,
            Act {
                kind: Kind::Snap,
                shift: by,
                ..NO_ACT
            },
        )
    }

    /// Read as `bars` bars, `ticks` song ticks in all ([`Kind::Bars`]), as
    /// a step Undo takes back: the loop's length, layers and sound stay.
    /// False (counted) if it can't be made.
    pub(crate) fn make_bars(&mut self, stats: &mut Stats, bars: u32, ticks: Tick) -> bool {
        let new = (self.editable() && ticks > 0).then(|| Shape {
            len_ticks: ticks,
            ..self.shape()
        });
        self.take_shape(
            stats,
            new,
            Act {
                kind: Kind::Bars,
                bars,
                ..NO_ACT
            },
        )
    }

    /// Playing, nothing recording: an edit can be made.
    fn editable(&self) -> bool {
        self.phase == Phase::Playing
            && !self.od_open
            && !self.first_open
            && self.len > 0
            && self.len_ticks > 0
    }

    /// Take shape `new` (None: counted, false), crossfading from the one
    /// now, which is kept for the Undo of `act`.
    fn take_shape(&mut self, stats: &mut Stats, new: Option<Shape>, act: Act) -> bool {
        let Some(new) = new else {
            stats.refused += 1;
            return false;
        };
        self.n_undone = 0;
        let Some(i) = self.free_shape() else {
            stats.refused += 1;
            return false;
        };
        self.shapes[i] = self.shape();
        self.set_shape(&new);
        self.push_done(Act {
            slot: i as u8,
            index: 0,
            ..act
        });
        if act.fades() {
            self.crossfade_from(i);
        }
        true
    }

    /// The shape a snap's move makes: the first pass's region in the audio
    /// it kept, its start moved and its length changed by `by` (an
    /// [`Edit::Move`] and an [`Edit::Resize`] at once). None if it can't
    /// be made.
    /// A/B on the seam ([`Command::AbSeam`], [`Command::AbMove`]) starts:
    /// the other side's seam made, played from the next half way (or, for
    /// a move while it runs, from the next half way on). False if it
    /// can't.
    pub(crate) fn ab_start(&mut self, to: Option<Shift>) -> bool {
        let Some(seam) = self.ab_seam(to) else {
            return false;
        };
        self.ab_views = self.views;
        self.ab_views[0] = seam;
        self.ab_on = true;
        self.ab_other = false;
        self.ab_off = false;
        self.ab_next = None;
        self.ab_last = self.pos;
        self.ab_version = self.version;
        true
    }

    /// The other side's seam for A/B ([`Loop::ab_start`]): its first
    /// layer's view, the loop's own but for its last beat (and its first,
    /// when the start moves). None if it can't be made.
    fn ab_seam(&self, to: Option<Shift>) -> Option<View> {
        let (found, moved) = self.smart_now()?;
        if self.od_open || self.busy() || self.speed != 0 || self.rate != 0 || self.print_on > 0.0 {
            return None;
        }
        // The other side: a move of your own; else the snap from your tap
        // (what Snap would make); from the snap, your tap, or your press
        // if only the start moved (what Use my tap or Use my press would).
        let by = match (to, found) {
            (Some(t), _) if t != moved => t - moved,
            (Some(_), _) => return None,
            (None, Some(f)) if f != moved => f - moved,
            _ if moved.end != 0 => Shift {
                start: 0,
                end: -moved.end,
            },
            _ if moved.start != 0 => Shift {
                start: -moved.start,
                end: 0,
            },
            _ => return None,
        };
        let other = self.shifted(by)?;
        // A beat (a quarter) of the loop, in frames.
        let ticks = i128::from(self.len_ticks.max(1));
        let beat = ((i128::from(PPQ) * self.len as i128 + ticks / 2) / ticks) as usize;
        let (v, w) = (self.views[0], other.views[0]);
        let (len, wl) = (v.len(), w.len());
        if beat == 0 || 3 * beat > len || 3 * beat > wl {
            return None;
        }
        let seam = || {
            let end = w.cut(wl - beat, wl)?;
            if by.start == 0 {
                v.cut(0, len - beat)?.then(&end)
            } else {
                w.cut(0, beat)?.then(&v.cut(beat, len - beat)?)?.then(&end)
            }
        };
        seam().filter(|s| s.len() == len)
    }

    /// A/B on the seam, each frame while it runs: from half way round
    /// (where both sides play the same), the other side's seam the next
    /// time round, then the loop's own; off there when asked. It ends at
    /// once when the loop changes in any other way: a snap as it lands,
    /// an edit, an overdub, a merge or a fit, a print coming in.
    #[inline(never)]
    pub(crate) fn ab_turn(&mut self) {
        if !self.ab_on {
            return;
        }
        let half = self.len as f64 * 0.5;
        let crossed = self.ab_last < half && self.pos >= half;
        self.ab_last = self.pos;
        let changed = self.version != self.ab_version
            || self.phase != Phase::Playing
            || self.od_open
            || self.print_on > 0.0
            || (self.busy() && self.snap.is_none());
        if changed || (crossed && self.ab_off) {
            self.ab_end();
        } else if let Some(to) = self.ab_next.filter(|_| crossed) {
            // A move's new other side, from half way, where both play the
            // same: it plays next.
            self.ab_next = None;
            match self.ab_seam(Some(to)) {
                Some(seam) => {
                    self.ab_views[0] = seam;
                    self.ab_other = true;
                }
                None => self.ab_end(),
            }
        } else if crossed {
            self.ab_other = !self.ab_other;
        }
    }

    pub(crate) fn ab_end(&mut self) {
        self.ab_on = false;
        self.ab_other = false;
        self.ab_off = false;
        self.ab_next = None;
    }

    pub(crate) fn shifted(&self, by: Shift) -> Option<Shape> {
        let (start, src) = self.bounds_now()?;
        let mut s = self.shape();
        let shortest = self.shortest();
        let to = (
            (start as i64).checked_add(by.start)?,
            (src as i64).checked_add(by.start)?,
            (s.len as i64).checked_add(by.longer())?,
        );
        self.rebound(&mut s, (start, src), to, shortest)?;
        Some(s)
    }

    /// A 1/16 note of the loop, in frames (at least 1).
    fn shortest(&self) -> i64 {
        let (len, ticks) = (self.len as i128, i128::from(self.len_ticks.max(1)));
        ((i128::from(SIXTEENTH) * len + ticks / 2) / ticks).max(1) as i64
    }

    /// Shape `s`'s first pass from region `from` (its first frame and the
    /// one on bar 1) to `to` (first frame, bar 1, length): within what was
    /// kept, at least `shortest` frames, and moved. None otherwise.
    fn rebound(
        &self,
        s: &mut Shape,
        from: (usize, usize),
        to: (i64, i64, i64),
        shortest: i64,
    ) -> Option<()> {
        let (a, b, n) = to;
        let kept = self.slots[0].valid.1 as i64;
        if a < 0
            || n < shortest
            || a + n > kept
            || to == (from.0 as i64, from.1 as i64, s.len as i64)
        {
            return None;
        }
        let size = s.views[0].size();
        s.views[0] = View::region(size, a as usize, n as usize, b as usize)?;
        s.region = Some((a as usize, b as usize));
        s.len = n as usize;
        Some(())
    }

    /// The shape an edit makes, or None if it cannot be made.
    fn edited(&self, e: Edit) -> Option<Shape> {
        let mut s = self.shape();
        let (len, ticks) = (s.len, s.len_ticks);
        // Ticks of the loop to frames, rounded.
        let frames = |t: Tick| {
            ((i128::from(t) * len as i128 + i128::from(ticks) / 2) / i128::from(ticks)) as usize
        };
        let live: [bool; MAX_LAYERS] = std::array::from_fn(|k| self.live(k));
        let mut each = |f: &dyn Fn(&View) -> Option<View>| -> Option<()> {
            for k in (0..MAX_LAYERS).filter(|&k| live[k]) {
                s.views[k] = f(&s.views[k])?;
            }
            Some(())
        };
        if matches!(
            e,
            Edit::Half | Edit::Double | Edit::Reverse | Edit::Nudge(_) | Edit::Trim { .. }
        ) {
            // The first pass's layer plays more than its region now.
            s.region = None;
        }
        match e {
            Edit::Half => {
                if ticks % 2 != 0 || ticks / 2 < SIXTEENTH || len < 2 {
                    return None;
                }
                each(&|v| v.cut(0, len / 2))?;
                s.len = len / 2;
                s.len_ticks = ticks / 2;
            }
            Edit::Double => {
                if len > u32::MAX as usize / 2 || ticks > Tick::MAX / 4 {
                    return None;
                }
                each(&|v| v.then(v))?;
                s.len = 2 * len;
                s.len_ticks = 2 * ticks;
            }
            Edit::Reverse => {
                each(&|v| Some(v.reverse()))?;
            }
            Edit::Nudge(n) => {
                let by =
                    (f64::from(n) * SIXTEENTH as f64 * len as f64 / ticks as f64).round() as i64;
                if by.rem_euclid(len as i64) == 0 {
                    return None;
                }
                each(&|v| v.rotate(by))?;
            }
            Edit::Trim { from, to } => {
                if from < 0 || to > ticks || to - from < SIXTEENTH {
                    return None;
                }
                let (a, b) = (frames(from), frames(to));
                if b <= a {
                    return None;
                }
                each(&|v| v.cut(a, b))?;
                s.len = b - a;
                s.len_ticks = to - from;
                s.origin += from;
            }
            Edit::Move(d) | Edit::Resize(d) | Edit::Turn(d) => {
                let (start, src) = self.bounds_now()?;
                let (a, b, n) = (start as i64, src as i64, len as i64);
                let to = match e {
                    Edit::Move(_) => (a.checked_add(d)?, b.checked_add(d)?, n),
                    Edit::Resize(_) => (a, b, n.checked_add(d)?),
                    _ => (a, a + (b - a + d.rem_euclid(n)) % n, n),
                };
                // Within what was kept, at least a 1/16 note, and moved.
                let shortest = frames(SIXTEENTH).max(1) as i64;
                self.rebound(&mut s, (start, src), to, shortest)?;
            }
            Edit::SeamFade(f) => s.seam = (f as usize).min(MAX_SEAM),
            Edit::Speed(to) => {
                let to = to.clamp(-1, 1);
                let by = to - s.speed;
                if by == 0 || (s.rate + by).abs() > 2 {
                    return None;
                }
                // Faster is fewer ticks for the same audio, slower more.
                let m: Tick = 1 << by.unsigned_abs();
                if by > 0 {
                    if ticks % m != 0 || ticks / m < SIXTEENTH {
                        return None;
                    }
                    s.len_ticks = ticks / m;
                } else {
                    s.len_ticks = ticks.checked_mul(m).filter(|t| *t <= Tick::MAX / 4)?;
                }
                s.speed = to;
                s.rate += by;
            }
        }
        // Every layer plays the loop's length (a check that costs nothing).
        if (0..MAX_LAYERS).any(|k| live[k] && s.views[k].len() != s.len) {
            return None;
        }
        Some(s)
    }

    /// The first pass's loop in the audio it kept (its first frame, and
    /// the one on bar 1), while it is the loop's only layer and plays that
    /// region alone, unedited (but for its seam fade and speed): what
    /// [`Edit::Move`], [`Edit::Resize`] and [`Edit::Turn`] change.
    pub(crate) fn bounds_now(&self) -> Option<(usize, usize)> {
        let (start, src) = self.region?;
        let v = &self.views[0];
        let alone = self.in_order == 1
            && self.order[0] == 0
            && self.slots[0].valid.0 == 0
            && (1..MAX_LAYERS).all(|k| !self.live(k));
        (alone && View::region(v.size(), start, self.len, src).as_ref() == Some(v))
            .then_some((start, src))
    }

    /// A place to keep a shape for Undo. When all are taken, the history
    /// up to its oldest edit or merge is forgotten.
    pub(crate) fn free_shape(&mut self) -> Option<usize> {
        loop {
            let taken = |i: usize| {
                self.done[..self.n_done]
                    .iter()
                    .chain(&self.undone[..self.n_undone])
                    .any(|a| a.shape() == Some(i))
                    || (self.xf > 0 && self.xf_shape == i)
                    || (self.merging && self.mg_shape == i)
                    || (self.fitting && self.fit_shape == i)
                    || (self.swapping && self.swap_shape == i)
            };
            if let Some(i) = (0..EDITS).find(|&i| !taken(i)) {
                return Some(i);
            }
            let i = self.done[..self.n_done]
                .iter()
                .position(|a| a.shape().is_some())?;
            self.let_go = self.done[i].txn;
            self.done.copy_within(i + 1..self.n_done, 0);
            self.n_done -= i + 1;
        }
    }

    /// Swap the shape now with the one kept at `i` (Undo and Redo of an
    /// edit), crossfading from it if `fade`.
    pub(crate) fn swap_shape(&mut self, i: usize, fade: bool) {
        let now = self.shape();
        let kept = self.shapes[i];
        self.set_shape(&kept);
        self.shapes[i] = now;
        if fade {
            self.crossfade_from(i);
        }
    }

    /// Crossfade from the shape kept at `i` (the loop's position is still
    /// that shape's).
    fn crossfade_from(&mut self, i: usize) {
        self.xf = FADE;
        self.xf_shape = i;
        self.xf_pos = self.pos;
    }

    /// The layers' sound changed: a print made before plays no more.
    pub(crate) fn touch(&mut self) {
        self.version = self.version.wrapping_add(1);
    }
}
