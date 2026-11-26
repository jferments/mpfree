//! What a take records from: the pads, the line input or a mix.

use super::*;

impl LoopHost {
    /// The pad player's sound for this period (the drums and the synth as
    /// the engines make them, before the mixer): what the looper records
    /// from [`LoopSource::Pads`]. Call it before [`LoopHost::run`] each
    /// period.
    pub fn take_pads(&mut self, left: &[f32; PERIOD], right: &[f32; PERIOD]) {
        self.pads_l = *left;
        self.pads_r = *right;
        self.pads_latency = 0;
    }

    /// What the pads play now (`main.rs`, `pads_instrument`), told each
    /// period: Records from: Pads records it, from the next
    /// take if one records now ([`LoopHost::settle`]).
    pub fn follow_pads(&mut self, now: Instrument) -> Option<(String, bool)> {
        self.pads_now = Some(now);
        self.settle()
    }

    /// Where what the pads play sounds in the mixer
    /// ([`crate::mixing::MixHost::instrument_strip`]; None on MIDI out),
    /// told each period with [`LoopHost::follow_pads`].
    pub fn set_pads_strip(&mut self, s: Option<mpfree_fx::StripId>) {
        self.pads_strip = s;
    }

    /// What the pads play now and where it sounds in the mixer: an audio
    /// track's input list's Pads (phase 4, board 4). None before the pads
    /// were first told.
    pub fn pads_play(&self) -> Option<(Instrument, Option<mpfree_fx::StripId>)> {
        Some((self.pads_now?, self.pads_strip))
    }

    /// The Records from picked last ([`LoopHost::set_source`]), whether a
    /// take holds another one until its end or not.
    pub fn chosen(&self) -> LoopSource {
        self.chosen
    }

    /// Records from picked (the Looper tab, Prefs): this take keeps its
    /// source, so it records from the next one, or at once if none
    /// records now ([`LoopHost::settle`]).
    pub fn set_source(&mut self, s: LoopSource) -> Option<(String, bool)> {
        self.chosen = s;
        self.settle()
    }

    /// A take records one source from its start to its end: the source
    /// picked and, for the pads, the instrument they played when it
    /// started (the design review of 20 Nov, point 6). A source picked,
    /// or the pads moved to another instrument, while a take records
    /// waits for the take's end: said once, on the screen too, and the
    /// Looper tab shows both ([`LooperView::taking_from`]). Then it takes
    /// over, and What you played starts again with it
    /// ([`Command::NewInput`]), so a Capture never spans two sources. A
    /// line for the log when it changes or waits (and whether the screen
    /// says it: a wait).
    fn settle(&mut self) -> Option<(String, bool)> {
        let pads = |s: LoopSource| {
            if s == LoopSource::Pads {
                self.pads_now
            } else {
                None
            }
        };
        let next = (self.chosen, pads(self.chosen));
        let now = (self.source, self.pads);
        if next == now {
            self.held_said = None;
            return None;
        }
        if self.taking() {
            if self.held_said == Some(next) {
                return None;
            }
            self.held_said = Some(next);
            let line = format!(
                "looper: this take keeps recording {}; {} from the next take",
                source_words(now.0, now.1),
                source_words(next.0, next.1)
            );
            return Some((line, true));
        }
        self.held_said = None;
        // Before the app first said what the pads play nothing was heard.
        let heard = now != (LoopSource::Pads, None);
        (self.source, self.pads) = next;
        let restarted = match self.looper.command(Command::NewInput) {
            true if heard => ", What you played starts again",
            true => "",
            false => ", What you played NOT started again (the command queue is full)",
        };
        let line = format!(
            "looper: records {}{restarted}",
            source_words(next.0, next.1)
        );
        Some((line, false))
    }

    /// What this period records from, as a take holds it: the source and,
    /// for the pads, their instrument (`main.rs` takes its sound from the
    /// mixer, `loop_input`).
    pub fn takes_from(&self) -> (LoopSource, Option<Instrument>) {
        (self.source, self.pads)
    }

    /// Whether any loop takes sound now: a first pass, an overdub, or a
    /// Capture waiting for its bar line.
    fn taking(&self) -> bool {
        (0..MAX_LOOPS)
            .filter_map(|n| self.looper.loop_status(n))
            .any(|s| {
                matches!(s.phase, Phase::Recording | Phase::Overdubbing) || s.capturing.is_some()
            })
    }

    /// Why `c` won't start a take on the picked loop: the source it would
    /// record (the pads or a mix) can't be had now, so a take would be
    /// silence. Said on the screen; never another sound in its place.
    /// None lets it through (and every press that ends or calls off a
    /// take, and Loops, which records the loops).
    pub fn refuse(&self, c: Command) -> Option<String> {
        let why = match self.source.line() {
            true => None,
            false => self.pads_dry,
        }?;
        let s = self.looper.loop_status(self.picked)?;
        let starts = match (c, s.phase) {
            (Command::Record | Command::Overdub, Phase::Empty) => {
                s.capturing.is_none() && !self.resample
            }
            (Command::Record | Command::Overdub, Phase::Playing) => true,
            _ => false,
        };
        starts.then(|| format!("Loop {} doesn't record: {why}", self.picked + 1))
    }

    /// The sound of what it records from for this period, as the mixer
    /// had it: the pads or a mix
    /// ([`crate::mixing::MixHost::loop_take`]), `latency` frames late after
    /// the period it lags; or silence, with why it can't be had (never
    /// another sound in its place). A line for the log when that changes
    /// (rarely: a strip routed to a submix, or a plugin's latency), and
    /// whether the screen says it too: while a take records, silence is a
    /// gap in it (a dropout, counted), and its sound back is said too.
    pub fn take_pads_as(
        &mut self,
        left: &[f32; PERIOD],
        right: &[f32; PERIOD],
        how: Result<u32, &'static str>,
    ) -> Option<(String, bool)> {
        self.pads_l = *left;
        self.pads_r = *right;
        self.pads_latency = how.map_or(0, |l| PERIOD as u32 + l);
        let dry = how.err();
        if dry == self.pads_dry {
            return None;
        }
        self.pads_dry = dry;
        // The pads on MIDI out: silence, said by `settle`.
        if self.source == LoopSource::Pads && self.pads == Some(Instrument::MidiOut) {
            return None;
        }
        let taking = self.taking();
        let what = source_words(self.source, self.pads);
        let line = match (dry, taking) {
            (Some(why), true) => {
                self.dropouts += 1;
                format!(
                    "looper: the take records silence from here, {what} can't be recorded: {why}"
                )
            }
            (None, true) => format!("looper: the take hears {what} again"),
            (Some(why), false) => {
                format!("looper: records silence, {what} can't be recorded: {why}")
            }
            (None, false) => format!("looper: hears {what} again"),
        };
        Some((line, taking))
    }

    /// Record from `s` (the pads or the line input): Capture forgets what
    /// it heard from the other.
    /// Listen: `what` Some(None) plays the picked loop alone, round and
    /// round from its start, while the song is stopped; Some(Some(id))
    /// one of its layers, even a muted one. The same again, or None,
    /// stops it; PLAY stops it too. The line says what it did, or why
    /// not.
    pub fn listen(&mut self, what: Option<Option<u8>>) -> String {
        let n = self.picked;
        let now = self.looper.listening();
        let Some(layer) = what else {
            self.looper.stop_listening();
            return "Listen: stopped".to_string();
        };
        if now.is_some_and(|(m, l, _)| m == n && l == layer) {
            self.looper.stop_listening();
            return "Listen: stopped".to_string();
        }
        let s = self.looper.status();
        let name = match layer {
            None => format!("Loop {}", n + 1),
            Some(_) => format!("Loop {}'s layer", n + 1),
        };
        if self.looper.listen(n, layer) {
            return format!("Listen: {name} plays on its own (again or PLAY: stop)");
        }
        match s.phase {
            Phase::Empty | Phase::Armed => {
                format!("Listen: Loop {} has nothing to play yet", n + 1)
            }
            Phase::Recording | Phase::Overdubbing => {
                format!("Listen: Loop {} is recording", n + 1)
            }
            Phase::Playing if self.last_rolling => {
                "Listen plays while the music is stopped: you hear the loops with it now"
                    .to_string()
            }
            Phase::Playing => format!("Listen: {name} is busy (merging): try again in a moment"),
        }
    }
}
