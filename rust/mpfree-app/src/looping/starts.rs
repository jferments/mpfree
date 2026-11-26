//! When I play, Tempo from the first loop, and the notes smart loop end
//! hears.

use super::*;

impl LoopHost {
    /// What starts a loop set to start when you play, for the log and
    /// the screen: "(the first pad you play)".
    pub fn first_note(&self) -> &'static str {
        match self.source.line() {
            false => "(the first pad you play)",
            true => "(the line input's first note over -40 dB)",
        }
    }

    /// Starts: When I play, with the pads: the loop listening for a pad
    /// hit, if one is (the pads are what it records, and no hit was told
    /// on `frame` yet).
    pub fn listens_for_pads(&self, frame: u64) -> Option<usize> {
        if !self.source.inside() || self.resample || self.played_at == Some(frame) {
            return None;
        }
        (0..MAX_LOOPS).find(|&n| self.looper.loop_status(n).is_some_and(|s| s.listening))
    }

    /// A pad hit on song frame `frame` (the coming period's first: the
    /// pads are read before the transport runs): a loop listening for one
    /// starts on it ([`LoopHost::listens_for_pads`]).
    pub fn played(&mut self, frame: u64) -> bool {
        self.played_at = Some(frame);
        self.looper.command(Command::Played(frame))
    }

    /// After the looper's period: a loop that heard you play with the song
    /// stopped asks for it to start, on the next period's first frame, so
    /// bar 1 is your note. Some(loop) once a note.
    pub fn take_start(&mut self) -> Option<usize> {
        self.looper.take_start()
    }

    /// The song can't start on loop `n`'s note (`why`): the loop forgets
    /// it and listens again. Counted; the line for the log and the screen.
    pub fn start_refused(&mut self, n: usize, why: &str) -> String {
        self.looper.start_refused();
        self.note_refused(n, why)
    }

    /// A note that would start loop `n` and the song, which can't start
    /// (`why`): counted; the line for the log and the screen. The loop
    /// still listens.
    pub fn note_refused(&mut self, n: usize, why: &str) -> String {
        self.starts_refused += 1;
        format!(
            "looper: Loop {} heard you, but the song can't start: {why}. It listens again",
            n + 1
        )
    }

    /// Tempo from the first loop on or off ([`Command::TempoFromLoop`]):
    /// the MENU setting, while the song can take a tempo from a loop.
    /// Cheap to call every period: the looper is told when it changes.
    pub fn set_tempo_from_loop(&mut self, on: bool) {
        if on != self.free_tempo && self.looper.command(Command::TempoFromLoop(on)) {
            self.free_tempo = on;
        }
    }

    /// The song's tempo was set by hand since the project opened
    /// ([`Command::TempoMeant`]): a free first loop's reading counts it
    /// strongly, a tempo left from before only lightly. Cheap to call
    /// every period: the looper is told when it changes.
    pub fn set_tempo_meant(&mut self, on: bool) {
        if on != self.tempo_meant && self.looper.command(Command::TempoMeant(on)) {
            self.tempo_meant = on;
        }
    }

    /// Loop `n` listening for a pad records free when it hears one
    /// ([`Status::free`]): the note doesn't start the song; closing the
    /// pass does (a song loop's), or sets the free clock (a free loop's).
    pub fn records_free(&self, n: usize) -> bool {
        self.looper.loop_status(n).is_some_and(|s| s.free)
    }

    /// Loop `n` keeps time with the free clock (Sync: Free,
    /// [`Clock::Free`]): a note that starts it never starts the song.
    pub fn on_free_clock(&self, n: usize) -> bool {
        self.looper
            .loop_status(n)
            .is_some_and(|s| s.clock == Clock::Free)
    }

    /// After the looper's period: a free first pass closed, so the song
    /// starts at the tempo its length sets, on the frame it comes round
    /// ([`Looper::take_tempo`]): its loop, how its length read (its bars
    /// and tempo, how sure) and that frame. A Length the reading couldn't
    /// keep is counted ([`LoopHost::readings_overruled`]).
    pub fn take_tempo(&mut self) -> Option<(usize, Reading, u64, Tick)> {
        let (n, r, at, into) = self.looper.take_tempo()?;
        if r.overruled() {
            self.readings_overruled += 1;
        }
        let version = self.looper.loop_status(n).map_or(0, |s| s.version);
        self.close_call = match (r.sure, r.other) {
            (Sure::Ambiguous, Some((bars, _))) => Some((n, version, bars)),
            _ => None,
        };
        Some((n, r, at, into))
    }

    /// Smart loop end (the MENU setting, [`Command::SetSmartEnd`]): it
    /// follows the pads and MIDI notes, and listens for the hits in the
    /// line input and a mix (step 3, [`Command::SmartByEar`], told each
    /// period with the source). Cheap to call every period: the looper is
    /// told when it changes.
    pub fn set_smart_end(&mut self, m: SmartEnd) {
        if m != self.smart && self.looper.command(Command::SetSmartEnd(m)) {
            self.smart = m;
        }
    }

    /// A note started on `track` (a pad hit or a MIDI note-on), on the
    /// song's frame clock: smart loop end weighs it. Every note of a
    /// period, before [`LoopHost::run`].
    pub fn note(&mut self, frame: u64, track: u16, key: u8, velocity: u8) {
        self.looper.note(NoteStart {
            frame,
            source: track,
            key,
            velocity,
        });
    }

    /// Notes started, the newest on `at`, that the time engine had no
    /// room to pass on: counted ([`LoopHost::notes_lost`]), and smart
    /// loop end keeps a tap whose reach they fall in.
    pub fn notes_lost(&mut self, at: u64) {
        self.notes_lost += 1;
        self.looper.notes_lost(at);
    }
}
