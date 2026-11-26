//! Playing: a loop's part of a run of frames, still or moving, and
//! reading its layers and its print.

use super::*;

impl Loop {
    /// A loop on a track's part of a run ([`Looper::play`]): only the
    /// frames it was told, each cut of the run at its place counted from
    /// its run's origin (its ticks moved by its own origin less the
    /// run's), into its own buffer. Frames it wasn't told stay silent.
    ///
    /// Where the track stops playing it while it overdubs (a launch, a
    /// block's end, STOP), the overdub punches out on that frame, its
    /// layer kept, and the writer's tail (the input that belongs before
    /// the frame, then its fade) runs on silently at the place it would
    /// have played, so nothing waits for its next launch to finish
    /// writing. Runs and the gaps between them go in time order.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run_told(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        t: &mut OnTrack,
        told: &Told,
        r: &Run,
        off: usize,
        print: Option<&Print>,
        waiting: bool,
        outs: &mut LoopOuts,
    ) {
        // The transport stopped with nothing told and something that
        // finishes only on frames (the first pass's tail when STOP closed
        // it, an overdub waiting for PLAY, a Clear's fade, a merge, a fit,
        // a swap, a mix change): the period runs silently, as it does off a
        // track, so the first pass finishes writing, an overdub punched out
        // before it began goes, leaving no layer, and a cleared loop
        // empties. Stopped, its place doesn't move; the looper never fades a
        // loop on a track (its track does), so the gain it had comes back.
        let moving = self.first_open
            || self.od_open
            || self.clearing
            || self.swapping
            || self.mixing
            || self.merging
            || self.fitting;
        if !r.playing && told.runs().is_empty() && moving {
            let gain = self.gain;
            let mut gone = [[0f32; 2]; RUN];
            self.run(
                store,
                stats,
                me,
                r,
                print,
                waiting,
                &mut gone[..r.ticks.len()],
            );
            if matches!(self.phase, Phase::Playing | Phase::Overdubbing)
                && !self.clearing
                && !self.swapping
            {
                self.gain = gain;
            }
            return;
        }
        let frames = r.ticks.len();
        let end = off + frames;
        let mut ticks = [0.0f64; RUN];
        // After the last run, the rest of the period is a gap.
        let rest = PlayRun {
            from: end as u32,
            to: end as u32,
            origin: 0.0,
        };
        let mut at = 0usize;
        for p in told.runs().iter().chain([&rest]) {
            let (from, to) = (p.from as usize, p.to as usize);
            // The gap before this run: only a writer still going is run.
            let (a, b) = (at.max(off), from.min(end));
            at = at.max(to);
            // A load or a copy still coming into it (an opened project's
            // loop that a cell names, phase 4e.3): it finishes on its
            // frame in a gap too, as off the track, nothing of it heard.
            if a < b && self.phase == Phase::Empty && (self.loading || self.adding) {
                let (j, m) = (a - off, b - a);
                let cut = Run {
                    from: r.from + j,
                    ticks: &r.ticks[j..j + m],
                    input: &r.input[2 * j..2 * (j + m)],
                    f: r.f + j as u64,
                    playing: r.playing,
                    first: None,
                };
                let mut gone = [[0f32; 2]; RUN];
                self.run(store, stats, me, &cut, print, waiting, &mut gone[..m]);
                // In, it is at full gain at once, as a loop on a track
                // always is: the track fades it, not the looper.
                if self.phase != Phase::Empty {
                    self.gain = 1.0;
                }
            }
            // A layer still copying into a loop that has sound (a whole
            // loop's second layer and on, phase 4e.5): it joins on its
            // frame in a gap too, nothing of it heard, so a copy onto a
            // track that isn't playing it yet finishes.
            if a < b
                && self.phase != Phase::Empty
                && self.adding
                && self.ad_done == self.ad_frames
                && self.ad_at < r.f + (b - off) as u64
            {
                self.finish_add();
            }
            let tail = self.phase == Phase::Overdubbing || self.od_open;
            let gap = t
                .last
                .filter(|l| a < b && tail && l.0 == r.f + (a - off) as u64);
            if let Some((e, origin)) = gap {
                if self.phase == Phase::Overdubbing {
                    self.punch_out();
                    stats.track_punch_outs += 1;
                }
                let (j, m) = (a - off, b - a);
                let shift = self.origin as f64 - origin;
                for (t, &s) in ticks[..m].iter_mut().zip(&r.ticks[j..j + m]) {
                    *t = s + shift;
                }
                let f = r.f + j as u64;
                let cut = Run {
                    from: r.from + j,
                    ticks: &ticks[..m],
                    input: &r.input[2 * j..2 * (j + m)],
                    f,
                    playing: r.playing,
                    first: on_track::first_of(Some((e, origin)), f, origin),
                };
                let mut gone = [[0f32; 2]; RUN];
                self.run(store, stats, me, &cut, print, waiting, &mut gone[..m]);
                t.last = Some((f + m as u64, origin));
            } else if a < b
                && self.phase != Phase::Empty
                && (self.clearing || self.swapping || self.mixing || self.merging || self.fitting)
            {
                // A Clear's fade, a swap, a mix change, a merge or a fit
                // with the song playing and its track not playing it (a
                // loop cell stopped, phase 4e.5): nothing of it is heard
                // in a gap, so it runs on silently and finishes, as when
                // stopped. Its place in the gap isn't kept: the next run
                // is a jump. The gain it had comes back, as there.
                let (j, m) = (a - off, b - a);
                let cut = Run {
                    from: r.from + j,
                    ticks: &r.ticks[j..j + m],
                    input: &r.input[2 * j..2 * (j + m)],
                    f: r.f + j as u64,
                    playing: r.playing,
                    first: None,
                };
                let gain = self.gain;
                let mut gone = [[0f32; 2]; RUN];
                self.run(store, stats, me, &cut, print, waiting, &mut gone[..m]);
                if matches!(self.phase, Phase::Playing | Phase::Overdubbing)
                    && !self.clearing
                    && !self.swapping
                {
                    self.gain = gain;
                }
            }
            // The run itself.
            let (a, b) = (from.max(off), to.min(end));
            if a >= b {
                continue;
            }
            let (j, m) = (a - off, b - a);
            let shift = self.origin as f64 - p.origin;
            for (t, &s) in ticks[..m].iter_mut().zip(&r.ticks[j..j + m]) {
                *t = s + shift;
            }
            let f = r.f + j as u64;
            let cut = Run {
                from: r.from + j,
                ticks: &ticks[..m],
                input: &r.input[2 * j..2 * (j + m)],
                f,
                playing: r.playing,
                first: on_track::first_of(t.last, f, p.origin),
            };
            self.run(
                store,
                stats,
                me,
                &cut,
                print,
                waiting,
                outs.run_mut(me, a, m),
            );
            t.last = Some((f + m as u64, p.origin));
        }
    }

    /// Nothing to play and nothing coming in: empty, with no load and no
    /// layer copying in. Its run would do nothing ([`Loop::run`]), so it
    /// isn't run.
    pub(crate) fn quiet(&self) -> bool {
        self.phase == Phase::Empty && !self.loading && !self.adding
    }

    /// The loop's part of a run: each frame's output is added to `ys`,
    /// once a frame (so a buffer of its own holds exactly what it played:
    /// [`LoopOuts`]). Frame by frame while anything moves (writers, fades,
    /// a jump); once the loop is still, the rest of the run in one go.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        r: &Run,
        print: Option<&Print>,
        waiting: bool,
        ys: &mut [[f32; 2]],
    ) {
        // A layer copying in or out between loops: only then are its
        // frames looked for, so an overdub, which goes a frame at a time,
        // doesn't pay for them.
        let copying = self.adding || self.lend.is_some();
        // A/B on the seam the same way: it starts only between periods
        // (it may end in the run: `ab_turn` says).
        let ab = self.ab_on;
        // The run's frames, as long as `ys`, so that indexing them by a
        // frame of it needs no check.
        let n = ys.len();
        let (ticks, input) = (&r.ticks[..n], &r.input.as_chunks::<2>().0[..n]);
        let mut j = 0;
        while j < ys.len() {
            if self.phase == Phase::Empty {
                // A load, or a layer copied in, starts playing on its frame
                // (whatever the period size), once all of it is in.
                let (at, ready) = if self.loading {
                    (self.ld_at, self.ld_done == self.ld_total)
                } else if self.adding {
                    (self.ad_at, self.ad_done == self.ad_frames)
                } else {
                    return;
                };
                let at = at.saturating_sub(r.f) as usize;
                if !ready || at >= ys.len() {
                    return;
                }
                j = j.max(at);
                if self.loading {
                    self.finish_load();
                } else {
                    self.finish_add();
                }
                continue;
            }
            let f = r.f + j as u64;
            if f >= self.snap_frame {
                self.land_snap(stats);
            }
            if self.merging && f >= self.mg_at {
                self.finish_merge(store, me);
            }
            if copying {
                if self.adding && f >= self.ad_at && self.ad_done == self.ad_frames {
                    self.finish_add();
                }
                if self.lend.is_some_and(|l| f >= l.at) {
                    self.end_lend();
                }
            }
            let live = print.is_some_and(|p| self.print_live(p, waiting));
            if self.fitting && f >= self.fit_at && self.jump == 0 && self.xf == 0 {
                self.finish_fit(store, stats, me, print);
            }
            let first = j == 0 && r.first.unwrap_or(r.from == 0);
            if !first && self.still(r.playing, live) {
                // Still to the run's end, or to where a merge or a fit
                // takes over.
                let mut end = ys.len();
                if self.snap_frame != u64::MAX {
                    end = end
                        .min(self.snap_frame.saturating_sub(r.f) as usize)
                        .max(j + 1);
                }
                if self.merging {
                    end = end.min((self.mg_at - r.f) as usize);
                }
                if self.fitting {
                    end = end.min(self.fit_at.saturating_sub(r.f) as usize).max(j + 1);
                }
                if copying {
                    if self.adding {
                        end = end.min(self.ad_at.saturating_sub(r.f) as usize).max(j + 1);
                    }
                    if let Some(l) = self.lend {
                        end = end.min(l.at.saturating_sub(r.f) as usize).max(j + 1);
                    }
                }
                self.still_run(store, me, print, &ticks[j..end], &mut ys[j..end]);
                j = end;
                continue;
            }
            let x = input[j];
            self.write_first(store, stats, me, f, x);
            self.follow(ticks[j], r.playing, first);
            if ab {
                self.ab_turn();
            }
            let v = self.read(store, me, r.playing, print, live);
            ys[j][0] += v[0];
            ys[j][1] += v[1];
            if self.od_open {
                self.overdub_frame(store, stats, me, x, r.playing);
            }
            j += 1;
        }
    }

    /// Playing at full gain with nothing moving: no writer, fade, mix
    /// change or jump. Only the position changes from frame to frame
    /// (a jump can only come at a span's first frame).
    fn still(&self, playing: bool, live: bool) -> bool {
        // The print's share settled: all of it while live, none if not.
        let settled = if live { 1.0 } else { 0.0 };
        self.phase == Phase::Playing
            && playing
            && !self.od_open
            && !self.first_open
            && !self.tail
            && !self.mixing
            && self.gain == 1.0
            && self.on
            && !self.muted
            && !self.solo_out
            && !self.clearing
            && self.jump == 0
            && self.xf == 0
            && self.len > 0
            && self.print_on == settled
            && !self.swapping
            && !self.ab_on
            && !self.replacing()
    }

    /// A Replace layer plays: the layers under it are read frame by frame
    /// ([`Loop::sample_at`]), with how far it covers each.
    pub(crate) fn replacing(&self) -> bool {
        self.order[..self.in_order]
            .iter()
            .any(|&k| self.slots[k as usize].replace.is_some())
    }

    /// How much of each layer is heard at loop frame `p` under the Replace
    /// layers after it in the play order (1: all of it), through a shape's
    /// views; `strength` is how much each Replace layer silences.
    pub(crate) fn ducks(
        &self,
        views: &[View; MAX_LAYERS],
        len: usize,
        p: usize,
        had: Option<&[u8]>,
        strength: impl Fn(&Slot) -> f32,
    ) -> [f32; MAX_LAYERS] {
        let mut out = [1.0f32; MAX_LAYERS];
        let mut d = 1.0f32;
        for &k in self.order[..self.in_order].iter().rev() {
            if had.is_some_and(|h| !h.contains(&k)) {
                continue;
            }
            let k = k as usize;
            out[k] = d;
            let s = &self.slots[k];
            if s.replace.is_none() {
                continue;
            }
            let v = &views[k];
            let (a, size) = if v.is_whole() {
                (p, len)
            } else if v.size() == 0 {
                continue;
            } else {
                (v.at(p), v.size())
            };
            d *= 1.0 - strength(s) * s.cover(a, size);
        }
        out
    }

    /// Frames of a still loop ([`Loop::still`]): the same sums as `read`
    /// and `sample_at` frame by frame, in the same order, so the same bits,
    /// but layer by layer. Out of line: called once a run, it keeps its
    /// own registers, whatever the frame-by-frame path around it needs.
    #[inline(never)]
    fn still_run(
        &mut self,
        store: &Store,
        me: usize,
        print: Option<&Print>,
        ticks: &[f64],
        ys: &mut [[f32; 2]],
    ) {
        if let Some(p) = print.filter(|_| self.print_on == 1.0) {
            // The print plays alone: as `follow` and `read` frame by frame.
            let (origin, len_ticks, lenf) =
                (self.origin as f64, self.len_ticks as f64, self.len as f64);
            let (p_origin, p_ticks, pf) = (
                self.p_origin as f64,
                self.p_ticks as f64,
                self.p_frames as f64,
            );
            let g = self.gain * self.level;
            for (y, &t) in ys.iter_mut().zip(ticks) {
                let rel = rem_ticks(t - origin, len_ticks);
                self.pos = (rel / len_ticks * lenf).min(lenf - 1e-6);
                let rel = rem_ticks(t - p_origin, p_ticks);
                self.ppos = (rel / p_ticks * pf).min(pf - 1e-6);
                let v = p.frame_at(self.ppos);
                y[0] += v[0] * g;
                y[1] += v[1] * g;
            }
            return;
        }
        let len = self.len;
        let n = ys.len();
        let mut i0s = [0usize; RUN];
        let mut i1s = [0usize; RUN];
        let mut fracs = [0.0f32; RUN];
        // `follow` for a still loop (playing, no jump or crossfade): the
        // song position's place in the loop.
        let (origin, len_ticks, lenf) = (self.origin as f64, self.len_ticks as f64, len as f64);
        for j in 0..n {
            let rel = rem_ticks(ticks[j] - origin, len_ticks);
            self.pos = (rel / len_ticks * lenf).min(lenf - 1e-6);
            let i0 = (self.pos as usize).min(len - 1);
            fracs[j] = (self.pos - i0 as f64) as f32;
            i0s[j] = i0;
            i1s[j] = if i0 + 1 == len { 0 } else { i0 + 1 };
        }
        let mut acc = [[0.0f32; 2]; RUN];
        for &k in &self.order[..self.in_order] {
            let k = k as usize;
            let s = &self.slots[k];
            let g = [s.gl * s.presence, s.gr * s.presence];
            if g == [0.0, 0.0] {
                continue;
            }
            let blocks = store.blocks_of(me, k);
            let view = &self.views[k];
            let whole = view.is_whole();
            if let Some(o) = if whole { None } else { view.forward() } {
                // One piece from audio frame `o` (a first pass's region):
                // as `layer_frame`, its frames straight from the audio
                // where they are all written (nearly always), but for the
                // seam's before the loop point.
                let seam = self.seam;
                let f = seam.min(len);
                let edge = if f > 0 && view.cut_after(0) {
                    len - f
                } else {
                    len
                };
                let (start, count) = s.valid;
                let plain = (start == 0 && o + len <= count) || count >= view.size();
                // The whole run before the seam (as nearly every run is):
                // no frame needs a check. Positions only go forward in a
                // run, so in one shorter than half the loop the last frame
                // is the furthest unless the run went round.
                let short = n > 0 && ticks[n - 1] - ticks[0] < len_ticks * 0.5;
                let top = if short && i0s[0] <= i0s[n - 1] {
                    i0s[n - 1]
                } else {
                    len - 1
                };
                if plain && top + 1 < edge {
                    for j in 0..n {
                        let (q0, q1) = (o + i0s[j], o + i1s[j]);
                        let a = store.frame(blocks, q0 / BLOCK, q0 % BLOCK);
                        let b = store.frame(blocks, q1 / BLOCK, q1 % BLOCK);
                        for c in 0..2 {
                            acc[j][c] += g[c] * (a[c] + (b[c] - a[c]) * fracs[j]);
                        }
                    }
                    continue;
                }
                for j in 0..n {
                    let (i0, i1) = (i0s[j], i1s[j]);
                    let (a, b) = if plain && i0 < edge && i1 < edge {
                        let (q0, q1) = (o + i0, o + i1);
                        (
                            store.frame(blocks, q0 / BLOCK, q0 % BLOCK),
                            store.frame(blocks, q1 / BLOCK, q1 % BLOCK),
                        )
                    } else {
                        (
                            piece_frame(store, blocks, s.valid, view, seam, 0, i0),
                            piece_frame(store, blocks, s.valid, view, seam, 0, i1),
                        )
                    };
                    for c in 0..2 {
                        acc[j][c] += g[c] * (a[c] + (b[c] - a[c]) * fracs[j]);
                    }
                }
                continue;
            }
            if !whole {
                // As `layer_frame`, finding the pieces with a cursor, and
                // reading a frame once when it is one frame's second and
                // the next one's first.
                let empty = view.size() == 0;
                let mut at = Cursor::new(view);
                let seam = self.seam;
                let mut last = (usize::MAX, [0.0f32; 2]);
                for j in 0..n {
                    let (a, b) = if empty {
                        ([0.0; 2], [0.0; 2])
                    } else {
                        let a = if i0s[j] == last.0 {
                            last.1
                        } else {
                            let (i, off) = at.locate(i0s[j]);
                            piece_frame(store, blocks, s.valid, view, seam, i, off)
                        };
                        let (i, off) = at.locate(i1s[j]);
                        let b = piece_frame(store, blocks, s.valid, view, seam, i, off);
                        last = (i1s[j], b);
                        (a, b)
                    };
                    for c in 0..2 {
                        acc[j][c] += g[c] * (a[c] + (b[c] - a[c]) * fracs[j]);
                    }
                }
                continue;
            }
            let (start, count) = s.valid;
            let full = count >= len;
            for j in 0..n {
                let (i0, i1) = (i0s[j], i1s[j]);
                let in0 = full || (i0 + len - start) % len < count;
                let in1 = full || (i1 + len - start) % len < count;
                if !in0 && !in1 {
                    continue;
                }
                let a = if in0 {
                    store.frame(blocks, i0 / BLOCK, i0 % BLOCK)
                } else {
                    [0.0; 2]
                };
                let b = if in1 {
                    store.frame(blocks, i1 / BLOCK, i1 % BLOCK)
                } else {
                    [0.0; 2]
                };
                for c in 0..2 {
                    acc[j][c] += g[c] * (a[c] + (b[c] - a[c]) * fracs[j]);
                }
            }
        }
        // `read`: the gain stays at 1 (it only rises to 1 while audible).
        let g = self.gain * self.level;
        for (y, a) in ys.iter_mut().zip(&acc) {
            y[0] += a[0] * g;
            y[1] += a[1] * g;
        }
    }

    fn read(
        &mut self,
        store: &mut Store,
        me: usize,
        playing: bool,
        print: Option<&Print>,
        live: bool,
    ) -> [f32; 2] {
        let audible = matches!(self.phase, Phase::Playing | Phase::Overdubbing)
            && playing
            && self.on
            && !self.muted
            && !self.solo_out
            && !self.clearing
            && !self.swapping;
        if audible {
            self.gain = (self.gain + STEP).min(1.0);
        } else {
            self.gain = (self.gain - STEP).max(0.0);
        }
        if self.clearing && self.gain == 0.0 {
            if self.keep_clear {
                store.keep(me, self);
            }
            self.reset();
            self.reclaim(store, me);
            return [0.0; 2];
        }
        if self.swapping && self.gain == 0.0 {
            self.swap();
        }
        // The print's share moves only while a print is live or fading.
        let printing = live || self.print_on > 0.0;
        if printing {
            self.step_print(print, live);
        }
        self.mix_layers();
        if self.gain == 0.0 || self.len == 0 {
            return [0.0; 2];
        }
        let y = if printing && self.print_on > 0.0 {
            self.print_at(store, me, print)
        } else {
            self.layers_at(store, me)
        };
        let g = self.gain * self.level;
        [y[0] * g, y[1] * g]
    }

    /// The print's share, a step toward 1 while it is live and toward 0
    /// when not.
    #[cold]
    fn step_print(&mut self, print: Option<&Print>, live: bool) {
        let on = live && print.is_some() && self.p_frames > 0;
        self.print_on = if on {
            (self.print_on + STEP).min(1.0)
        } else {
            (self.print_on - STEP).max(0.0)
        };
    }

    /// The print at its place (through a jump's crossfade), crossfaded
    /// with the layers while it fades in or out.
    #[cold]
    fn print_at(&self, store: &Store, me: usize, print: Option<&Print>) -> [f32; 2] {
        let Some(p) = print.filter(|_| self.p_frames > 0) else {
            return self.layers_at(store, me);
        };
        let mut py = p.frame_at(self.ppos);
        if self.jump > 0 {
            let w = self.jump as f32 * STEP;
            let old = p.frame_at(self.pjump_pos);
            for (v, o) in py.iter_mut().zip(old) {
                *v = *v * (1.0 - w) + o * w;
            }
        }
        if self.print_on < 1.0 {
            let w = self.print_on;
            let ly = self.layers_at(store, me);
            [ly[0] * (1.0 - w) + py[0] * w, ly[1] * (1.0 - w) + py[1] * w]
        } else {
            py
        }
    }

    /// The layers at the loop's place, through a jump's or an edit's
    /// crossfade.
    fn layers_at(&self, store: &Store, me: usize) -> [f32; 2] {
        // (A/B on the seam: the other side's rounds through its views.)
        let views = if self.ab_other {
            &self.ab_views
        } else {
            &self.views
        };
        let now = (self.len, self.seam, views, None);
        let mut y = self.sample_at(store, me, self.pos, now);
        if self.jump > 0 {
            let w = self.jump as f32 * STEP;
            let old = self.sample_at(store, me, self.jump_pos, now);
            for (v, o) in y.iter_mut().zip(old) {
                *v = *v * (1.0 - w) + o * w;
            }
        }
        if self.xf > 0 {
            let w = self.xf as f32 * STEP;
            let sh = &self.shapes[self.xf_shape];
            // The loop before the edit: the layers it had, through their
            // views then. A layer that came in since (an overdub opened in
            // the crossfade, in a slot another layer had before) has no
            // view in it: read through the slot's old view, it would play
            // wrong frames, or (a valid stretch past the old view's end)
            // panic on the arithmetic. The layers it had are all still the
            // loop's: Undo, Delete and Merge wait for the crossfade
            // ([`Loop::must_wait`]).
            let had = Some(&sh.order[..sh.in_order]);
            let old = self.sample_at(store, me, self.xf_pos, (sh.len, sh.seam, &sh.views, had));
            for (v, o) in y.iter_mut().zip(old) {
                *v = *v * (1.0 - w) + o * w;
            }
        }
        y
    }

    /// One frame of every layer's fades: coming and going, and its gains
    /// moving to its level, pan, mute and solo. A layer that has faded
    /// away leaves the play order.
    fn mix_layers(&mut self) {
        if !self.mixing {
            return;
        }
        let any_solo = self.any_solo();
        let mut settled = true;
        let mut p = 0;
        while p < self.in_order {
            let s = &mut self.slots[self.order[p] as usize];
            if s.leaving {
                s.presence = (s.presence - STEP).max(0.0);
                if s.presence == 0.0 {
                    s.leaving = false;
                    self.remove_at(p);
                    settled = false;
                    continue;
                }
            } else if s.presence < 1.0 {
                s.presence = (s.presence + STEP).min(1.0);
            }
            let (l, r) = s.target(any_solo);
            s.gl += (l - s.gl).clamp(-STEP, STEP);
            s.gr += (r - s.gr).clamp(-STEP, STEP);
            let d = if s.heard(any_solo) { 1.0 } else { 0.0 };
            s.duck += (d - s.duck).clamp(-STEP, STEP);
            settled &= !s.leaving && s.presence == 1.0 && s.gl == l && s.gr == r && s.duck == d;
            p += 1;
        }
        self.mixing = !settled;
    }

    /// Every layer at `pos`, read between frames, at its gains, through
    /// a shape's views (the loop's `len`, `seam` and `views`, now or before
    /// an edit, with the layers it had then). Only the frames a layer has
    /// written count.
    fn sample_at(
        &self,
        store: &Store,
        me: usize,
        pos: f64,
        shape: (usize, usize, &[View; MAX_LAYERS], Option<&[u8]>),
    ) -> [f32; 2] {
        let (len, seam, views, had) = shape;
        let i0 = (pos as usize).min(len - 1);
        let frac = (pos - i0 as f64) as f32;
        let i1 = if i0 + 1 == len { 0 } else { i0 + 1 };
        let (b0, o0, b1, o1) = (i0 / BLOCK, i0 % BLOCK, i1 / BLOCK, i1 % BLOCK);
        let mut y = [0.0f32; 2];
        // Under a Replace layer, each layer at its share of i0 and i1.
        let (d0, d1) = if self.replacing() {
            let strength = |s: &Slot| s.duck * s.presence;
            (
                self.ducks(views, len, i0, had, strength),
                self.ducks(views, len, i1, had, strength),
            )
        } else {
            ([1.0; MAX_LAYERS], [1.0; MAX_LAYERS])
        };
        for &k in &self.order[..self.in_order] {
            if had.is_some_and(|h| !h.contains(&k)) {
                continue;
            }
            let k = k as usize;
            let s = &self.slots[k];
            let g = [s.gl * s.presence, s.gr * s.presence];
            if g == [0.0, 0.0] || (d0[k] == 0.0 && d1[k] == 0.0) {
                continue;
            }
            if !views[k].is_whole() {
                let view = &views[k];
                let blocks = store.blocks_of(me, k);
                // A first pass plays its audio straight through from one
                // frame (`still_run` does the same): away from the seam,
                // with every frame written, a frame is just `o + i`.
                let fast = view.forward().filter(|&o| {
                    let f = seam.min(len);
                    let edge = if f > 0 && view.cut_after(0) {
                        len - f
                    } else {
                        len
                    };
                    let (start, count) = s.valid;
                    let plain = (start == 0 && o + len <= count) || count >= view.size();
                    plain && i0 < edge && i1 < edge
                });
                let (a, b) = match fast {
                    Some(o) => {
                        let (q0, q1) = (o + i0, o + i1);
                        (
                            store.frame(blocks, q0 / BLOCK, q0 % BLOCK),
                            store.frame(blocks, q1 / BLOCK, q1 % BLOCK),
                        )
                    }
                    None => (
                        layer_frame(store, blocks, s.valid, view, seam, i0),
                        layer_frame(store, blocks, s.valid, view, seam, i1),
                    ),
                };
                let (a, b) = (a.map(|v| v * d0[k]), b.map(|v| v * d1[k]));
                for (c, v) in y.iter_mut().enumerate() {
                    *v += g[c] * (a[c] + (b[c] - a[c]) * frac);
                }
                continue;
            }
            let (start, count) = s.valid;
            let full = count >= len;
            let in0 = full || (i0 + len - start) % len < count;
            let in1 = full || (i1 + len - start) % len < count;
            if !in0 && !in1 {
                continue;
            }
            let blocks = store.blocks_of(me, k);
            let a = if in0 {
                store.frame(blocks, b0, o0)
            } else {
                [0.0; 2]
            };
            let b = if in1 {
                store.frame(blocks, b1, o1)
            } else {
                [0.0; 2]
            };
            let (a, b) = if d0[k] == 1.0 && d1[k] == 1.0 {
                (a, b)
            } else {
                (a.map(|v| v * d0[k]), b.map(|v| v * d1[k]))
            };
            for (c, v) in y.iter_mut().enumerate() {
                *v += g[c] * (a[c] + (b[c] - a[c]) * frac);
            }
        }
        y
    }
}
