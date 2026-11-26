//! A loop's and a layer's commands.

use super::*;

impl Loop {
    pub(crate) fn apply_layer(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        id: u8,
        c: LayerCommand,
    ) {
        self.mixing = true;
        self.touch();
        let slot = id as usize;
        let at = self.order[..self.in_order].iter().position(|&k| k == id);
        let Some(at) = at.filter(|_| !self.slots[slot].leaving) else {
            stats.no_layer += 1;
            return;
        };
        let s = &mut self.slots[slot];
        match c {
            LayerCommand::Level(v) if v.is_finite() => s.level = v.clamp(0.0, 2.0),
            LayerCommand::Pan(v) if v.is_finite() => s.pan = v.clamp(-1.0, 1.0),
            LayerCommand::Level(_) | LayerCommand::Pan(_) => {}
            LayerCommand::Mute(m) => s.mute = m,
            LayerCommand::Solo(o) => s.solo = o,
            // The overdub being recorded is thrown away, as Undo does.
            LayerCommand::Delete if self.od_open && self.od_layer == slot => {
                self.apply(store, stats, me, Command::Undo)
            }
            LayerCommand::Delete if self.layers() > 1 => {
                self.n_undone = 0;
                self.push_done(Act {
                    kind: Kind::Delete,
                    slot: id,
                    index: at as u8,
                    edit: None,
                    ages: [1.0; MAX_LAYERS],
                    link: None,
                    shift: Shift::NONE,
                    bars: 0,
                    txn: TransactionId::NONE,
                });
                self.slots[slot].leaving = true;
            }
            LayerCommand::Delete => {}
        }
    }

    pub(crate) fn apply(&mut self, store: &mut Store, stats: &mut Stats, me: usize, c: Command) {
        self.mixing = true;
        // Commands that leave the layers' sound as it is keep the version
        // (and so the print); a merge sounds the same as its layers.
        if !matches!(
            c,
            Command::Mute(_)
                | Command::Solo(_)
                | Command::SetOverdubs(_)
                | Command::SetBars(_)
                | Command::SetAfter(_)
                | Command::SetStarts(_)
                | Command::SetClock(_)
                | Command::FreeStop
                | Command::FreePlay
                | Command::UseReading
                | Command::AbSeam(_)
                | Command::AbMove(_)
                | Command::Played(_)
                | Command::StartsByEar(_)
                | Command::TempoFromLoop(_)
                | Command::TempoMeant(_)
                | Command::SetSmartEnd(_)
                | Command::SmartByEar(_)
                | Command::SetFadeOld(_)
                | Command::SetLatency(_)
                | Command::SetLevel(_)
                | Command::Merge
                | Command::Fit
                | Command::NewInput
                | Command::ForgetRedo
                | Command::RetireThrough(_)
        ) {
            self.touch();
        }
        match c {
            Command::Record => match self.phase {
                Phase::Empty => {
                    self.phase = Phase::Armed;
                    self.ring = [0.0; 2 * FADE];
                    self.unhear();
                    self.just_armed = true;
                    self.tap_lead = None;
                    // A new loop plays once recorded.
                    self.on = true;
                    self.cue = None;
                }
                Phase::Armed => {
                    self.phase = Phase::Empty;
                    self.resample = false;
                    self.unhear();
                }
                Phase::Recording => self.ending = true,
                Phase::Playing | Phase::Overdubbing => {
                    self.apply(store, stats, me, Command::Overdub)
                }
            },
            Command::Overdub => match self.phase {
                Phase::Playing => self.open_overdub(store, stats, me, FADE),
                Phase::Overdubbing => self.punch_out(),
                // (Not after a resample: the overdub would take the input.)
                Phase::Recording if !self.resample => self.then_overdub = !self.then_overdub,
                _ => {}
            },
            Command::Undo => match self.phase {
                Phase::Armed | Phase::Recording => self.abort(stats),
                Phase::Overdubbing | Phase::Playing if self.od_open => self.undo_open_overdub(),
                Phase::Playing if self.merging => self.merging = false,
                Phase::Playing if self.fitting => {
                    self.fitting = false;
                    self.fit_overdub = false;
                    self.print_wait = false;
                }
                // An overdub waiting for its print is called off.
                Phase::Playing if self.print_wait => self.print_wait = false,
                Phase::Playing => self.undo(),
                Phase::Empty if store.has_kept(me) && !self.loading && !self.adding => {
                    self.unclear(store, me)
                }
                _ => {}
            },
            Command::Redo => {
                if self.phase == Phase::Playing && self.n_undone > 0 && !self.od_open {
                    self.redo();
                }
            }
            Command::Clear | Command::Forget => {
                let keep = c == Command::Clear;
                if !keep {
                    store.drop_kept(me);
                }
                match self.phase {
                    Phase::Empty => {}
                    Phase::Armed | Phase::Recording => self.abort(stats),
                    Phase::Playing | Phase::Overdubbing => {
                        // An overdub under way is undone first, so UNDO
                        // brings the loop back as it was before it (a key
                        // held to clear started one on its press), and
                        // Redo then brings what it recorded.
                        if self.od_open {
                            self.undo_open_overdub();
                        }
                        self.clearing = true;
                        self.keep_clear = keep;
                    }
                }
            }
            Command::Mute(m) => self.muted = m,
            Command::Solo(o) => self.solo = o,
            Command::SetOverdubs(o) => self.overdubs = o,
            Command::SetBars(b) => self.bars = b.filter(|&b| b > 0),
            Command::SetAfter(a) => self.after = a,
            Command::SetStarts(w) => {
                self.starts = w;
                if w == Starts::Bar {
                    self.unhear();
                }
            }
            Command::SetClock(c) => {
                if self.phase == Phase::Empty && !self.loading && !self.adding && !self.capturing()
                {
                    self.clock = c;
                } else if c != self.clock {
                    stats.refused += 1;
                }
            }
            // The looper hands these to every loop.
            Command::FreeStop
            | Command::FreePlay
            | Command::Played(_)
            | Command::StartsByEar(_)
            | Command::TempoFromLoop(_)
            | Command::TempoMeant(_)
            | Command::SetSmartEnd(_)
            | Command::SmartByEar(_) => {}
            // (Started in the next `close_free`, on its frame.)
            Command::UseReading if self.asks.is_some() => self.asks_yes = true,
            Command::UseReading => stats.refused += 1,
            Command::AbSeam(true) if self.ab_on => self.ab_off = false,
            Command::AbSeam(true) => {
                if !self.ab_start(None) {
                    stats.refused += 1;
                }
            }
            Command::AbMove(to) if self.ab_on => {
                self.ab_next = Some(to);
                self.ab_off = false;
            }
            Command::AbMove(to) => {
                if !self.ab_start(Some(to)) {
                    stats.refused += 1;
                }
            }
            // On its own side it ends now; on the other, at half way.
            Command::AbSeam(false) if self.ab_other => self.ab_off = true,
            Command::AbSeam(false) => self.ab_end(),
            Command::SetFadeOld(k) if k.is_finite() => self.fade_old = k.clamp(0.0, 1.0),
            Command::SetFadeOld(_) => {}
            Command::SetLatency(l) => self.latency = l,
            Command::SetLevel(v) => {
                self.level = if v.is_finite() {
                    v.clamp(0.0, 2.0)
                } else {
                    self.level
                }
            }
            Command::Edit(e) => self.edit(stats, e),
            // The looper starts these, with the time, the print and what
            // it heard.
            Command::Merge
            | Command::Fit
            | Command::Capture(_)
            | Command::CaptureTo(..)
            | Command::NewInput
            | Command::ForgetRedo
            | Command::RetireThrough(_) => {}
            Command::Resample => match self.phase {
                Phase::Empty => {
                    self.apply(store, stats, me, Command::Record);
                    self.resample = true;
                }
                Phase::Armed | Phase::Recording if self.resample => {
                    self.apply(store, stats, me, Command::Record)
                }
                _ => stats.refused += 1,
            },
        }
    }
}
