//! The period: the line input read and the looper run, and the log's lines
//! for what changed.

use super::*;

impl LoopHost {
    /// One period for the mixer: read the line input, run the looper on the
    /// transport's `block` with the source's sound, and write the loop's
    /// sound into `left` and `right` (the Loop track). The line input of
    /// the period is [`LoopHost::input`]: the mixer's input strip hears it,
    /// following [`LoopHost::monitor`]. A note comes back if the input had
    /// a problem.
    pub fn run(
        &mut self,
        block: &Block,
        left: &mut [f32; PERIOD],
        right: &mut [f32; PERIOD],
    ) -> Option<String> {
        let note = self.record(block);
        for f in 0..PERIOD {
            left[f] = self.out[2 * f];
            right[f] = self.out[2 * f + 1];
        }
        note
    }

    /// The line input of the last period: this period's only after
    /// [`LoopHost::run`] (or [`LoopHost::period`]) has run; zeros while the
    /// line input is closed; and a period with too little input arrived is
    /// padded with silence at its end ([`LineStats`]' short count says
    /// when), so a gap can reach whatever records it (the sampler's input
    /// buffer, too).
    pub fn input(&self) -> (&[f32; PERIOD], &[f32; PERIOD]) {
        (&self.input_l, &self.input_r)
    }

    /// One period without the mixer (the screen's samples): as
    /// [`LoopHost::run`], then the loop (and the line input, with Monitor
    /// on) added to the output, kept within ±1. Returns the output's peak,
    /// and a note for the log if the input had a problem.
    pub fn period(
        &mut self,
        block: &Block,
        left: &mut [f32; PERIOD],
        right: &mut [f32; PERIOD],
    ) -> (f32, Option<String>) {
        self.last_rolling = block
            .spans()
            .iter()
            .any(|s| s.state != mpfree_transport::State::Stopped);
        let note = self.record(block);
        let mut peak = 0f32;
        let mut over = false;
        let mon = if self.monitor { 1.0 } else { 0.0 };
        for f in 0..PERIOD {
            let l = left[f] + self.out[2 * f] + mon * self.input_l[f];
            let r = right[f] + self.out[2 * f + 1] + mon * self.input_r[f];
            over |= l.abs() > 1.0 || r.abs() > 1.0;
            left[f] = l.clamp(-1.0, 1.0);
            right[f] = r.clamp(-1.0, 1.0);
            peak = peak.max(left[f].abs()).max(right[f].abs());
        }
        if over {
            self.clipped += 1;
        }
        (peak, note)
    }

    /// Reads the line input and runs the looper into `self.out`.
    fn record(&mut self, block: &Block) -> Option<String> {
        self.queue_steps();
        let note = self.line.period(&mut self.input_l, &mut self.input_r);
        // An audio track's line pick is the line input as its take has
        // it, in the pads' buffers (they are not heard from then).
        if let LoopSource::Track(TrackInput {
            from: TrackFrom::Line(pick),
            ..
        }) = self.source
        {
            let line = (&self.input_l, &self.input_r);
            pick_line(pick, line, &mut self.pads_l, &mut self.pads_r);
        }
        let (src_l, src_r) = match self.source {
            LoopSource::Line => (&self.input_l, &self.input_r),
            _ => (&self.pads_l, &self.pads_r),
        };
        for f in 0..PERIOD {
            self.input[2 * f] = src_l[f];
            self.input[2 * f + 1] = src_r[f];
        }
        for (m, x) in self.meter.iter_mut().zip([src_l, src_r]) {
            *m = x.iter().fold(*m, |a, v| a.max(v.abs()));
        }
        let chunks = self.heard_peaks.len() as u64;
        if chunks > 0 {
            for f in 0..PERIOD {
                let at = block.start_frame + f as u64;
                let p = &mut self.heard_peaks[((at / HEARD_CHUNK) % chunks) as usize];
                if at.is_multiple_of(HEARD_CHUNK) {
                    *p = 0.0;
                }
                *p = p.max(src_l[f].abs()).max(src_r[f].abs());
            }
        }
        // A period nobody handed the pads' sound to records silence.
        self.pads_l = [0.0; PERIOD];
        self.pads_r = [0.0; PERIOD];
        // The pads' latency follows the mix (a plugin's latency, or dry).
        if self.source.inside()
            && self.latency_set != self.pads_latency
            && self.looper.command(Command::SetLatency(self.pads_latency))
        {
            self.latency_set = self.pads_latency;
        }
        // A loop that starts when you play listens to the line input; the
        // pads' sound has the song in it, so a pad hit is told instead.
        let by_ear = self.source.line();
        if by_ear != self.by_ear_set && self.looper.command(Command::StartsByEar(by_ear)) {
            self.by_ear_set = by_ear;
        }
        // Smart loop end listens for the hits in the line input and a mix
        // (the notes are the pads', and a mix may hold more than them).
        let by_ear = self.source != LoopSource::Pads;
        if by_ear != self.by_ear_smart && self.looper.command(Command::SmartByEar(by_ear)) {
            self.by_ear_smart = by_ear;
        }
        if let Some(span) = block.spans().first() {
            self.beats_per_bar = u32::from(span.meter.beats);
            self.bar_ticks = span.meter.bar_ticks();
            self.bpm = span.bpm;
        }
        // Each loop's sound apart (audio tracks phase 4a), added up where
        // the Loop strip takes the mix, so nothing sounds different: the
        // sum is what `process` would have added, bit for bit. Phase 4b
        // gives each loop its own track's channel.
        self.out.fill(0.0);
        self.looper.process_each(block, &self.input);
        self.looper.outs().mix_into(&mut self.out);
        self.settle = self.settle.saturating_sub(1);
        if let Some(span) = block.spans().last() {
            // A free first pass that closed in this period sets the song's
            // tempo from the next one, so the printer sees the loops at
            // that tempo and asks for no print they won't need.
            let bpm = self.looper.tempo_coming().unwrap_or(span.bpm);
            self.printer.tend(&mut self.looper, bpm);
        }
        self.finder.tend(&mut self.looper);
        self.tend_save();
        self.tend_copy();
        note
    }

    /// Lines for the log when the looper's phase or layers changed since
    /// the last call; `at` gives the song position as text. Nothing is
    /// allocated when nothing changed.
    pub fn changes(&mut self, at: impl FnOnce() -> String) -> Vec<String> {
        let s = self.looper.status();
        let was = self.last;
        self.last = s;
        let mut lines = Vec::new();
        if s.phase == was.phase && s.layers == was.layers && s.redo == was.redo {
            return lines;
        }
        let at = at();
        if s.phase != was.phase {
            lines.push(match s.phase {
                Phase::Empty if was.phase == Phase::Recording => format!("looper: first pass thrown away at {at} (STOP \
                                                                           before a 1/16 note, or the transport jumped)"),
                Phase::Empty => format!("looper: empty at {at}"),
                Phase::Armed => format!("looper: armed at {at}"),
                Phase::Recording => format!("looper: recording the first pass from {at} ({})", bars_text(s.bars)),
                Phase::Playing if was.phase == Phase::Recording => {
                    let kept = match s.cut {
                        Some(c) => format!(", cut at STOP after {:.2} s (silent to the bar line)", c as f64 / 44_100.0),
                        None => String::new(),
                    };
                    let how = if s.asks.is_some() { "waiting on its Length" } else { "playing" };
                    format!("looper: loop of {} at {at}: {} frames ({:.2} s), {how}{kept}", ticks_text(s.ticks, self.bar_ticks),
                            s.frames, s.frames as f64 / 44_100.0)
                }
                Phase::Playing => format!("looper: playing at {at}, {} layers", s.layers),
                Phase::Overdubbing => format!("looper: overdubbing layer {} from {at}", s.layers),
            });
        } else if s.layers != was.layers || s.redo != was.redo {
            lines.push(format!(
                "looper: {} layers, {} to redo, at {at}",
                s.layers, s.redo
            ));
        }
        lines
    }
}
