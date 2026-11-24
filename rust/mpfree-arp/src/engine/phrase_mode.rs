//! Phrase mode: what the engine does with a phrase.
//!
//! The engine has two generators. The step modes (Repeat, Arp, Pattern)
//! play one step after another on the rate's grid, in `engine.rs`. Phrase
//! mode plays a [`Phrase`]'s notes where they were written, and lives
//! here. Both share the engine's held notes and Latch, its settings and
//! Modifier, its clock, and its queue of note-ons and note-offs.
//!
//! Where the phrase is (which note is next, which time round, the line
//! a waiting phrase comes in on) is the [`Playhead`]'s business
//! ([`crate::phrase`]): it counts in ticks and knows nothing of frames or
//! of sound. This file turns what the playhead says is due into notes:
//! the pitch from the newest held note, the velocity, the gate, swing,
//! and who ends what.
//!
//! **Who ends what.** A step mode's note is a few steps long at most, so
//! the engine lets every note it started run out. A phrase's note can be
//! bars long, so here:
//!
//! - the same note struck again ends the one sounding at that moment (its
//!   own ending, still queued, would otherwise cut the new one short);
//! - when the phrase stops (nothing held, or the mode changes), what it
//!   has sounding ends at once;
//! - when it starts again (Retrigger, a transport start or jump), or
//!   another phrase comes in, what still sounds ends there.
//!
//! [`Playhead`]: crate::phrase::Playhead

use super::{Engine, EventKind, Hit, Mode, Settings, SyncStart, PPQ};
use crate::phrase::{self, Feel, Join, Phrase, Turn, MAX_PHRASE_NOTES};
use crate::scale::Scale;

/// The 1/16 a phrase starts on and swings on, in ticks.
const SIXTEENTH: u32 = PPQ / 4;

/// How late a press may be for a start line and still count for it, in
/// seconds (and never more than half a 1/16): about as late as a hand
/// playing on the beat lands. A press meant for the next 1/16 comes
/// later than this at any tempo.
const LATE_PRESS: f64 = 0.060;

/// How fast a phrase plays against its written rhythm (written ticks per
/// clock tick) with the settings `set` as a Modifier pushes them to
/// `live`: the speed, times the push on the rate (two steps faster from
/// 1/16 is 1/32: twice as fast). The engine plays by it, and a screen
/// shows the same number.
pub fn phrase_speed(set: &Settings, live: &Settings) -> f64 {
    live.speed.factor() * f64::from(set.rate.ticks()) / f64::from(live.rate.ticks())
}

impl Engine {
    /// The phrase Phrase mode plays. Picked while one plays, it comes in
    /// from its first note on a line of the playing phrase: its next bar
    /// line while the transport plays, so the bars it keeps with the song
    /// go on unbroken, and its next step while the transport is stopped,
    /// when nothing else counts bars and a list of phrases is quick to
    /// hear through. Otherwise it is there at once. Picking again before
    /// that line replaces the one waiting, and picking the one that plays
    /// leaves it playing on, with nothing waiting (so a track armed again
    /// never starts its phrase over).
    pub fn set_phrase(&mut self, phrase: Phrase) {
        if self.running && self.settings.mode == Mode::Phrase {
            self.next_phrase = (phrase != self.phrase).then_some(phrase);
        } else {
            self.phrase = phrase;
            self.next_phrase = None;
        }
    }

    /// The phrase playing (or that plays next in Phrase mode).
    pub fn phrase(&self) -> &Phrase {
        &self.phrase
    }

    /// The key a phrase follows In key while the arp's own scale
    /// (`Settings::scale`) is off: the scale the track's pads play, so a
    /// riff stays in the key the pads are in. The arp's own scale, when
    /// on, wins. It changes nothing in the other modes, and is not one of
    /// the arp's settings: it belongs to the track's pads.
    pub fn set_key(&mut self, key: Option<Scale>) {
        self.key = key;
    }

    pub fn key(&self) -> Option<Scale> {
        self.key
    }

    /// Where the phrase is now, in written ticks from its start (0 up to
    /// its length), for a cursor on screen; None unless a phrase plays.
    pub fn phrase_at(&self) -> Option<f64> {
        if !self.running || self.settings.mode != Mode::Phrase {
            return None;
        }
        self.playhead
            .position(&self.phrase, self.clock_tick(self.now as f64))
    }

    /// The line of the playing phrase a waiting one comes in on: its next
    /// bar line while the transport plays, its next step while it is
    /// stopped ([`Engine::set_phrase`]).
    fn phrase_join(&self) -> Join {
        match self.sync {
            Some(_) => Join::Bar,
            None => Join::Step,
        }
    }

    /// The line a picked phrase is waiting for while another still plays,
    /// for a screen that shows the pick coming in; None when none waits.
    pub fn phrase_waiting(&self) -> Option<Join> {
        self.next_phrase.is_some().then(|| self.phrase_join())
    }

    /// Ticks to the next place a phrase may start while the transport
    /// plays: the next 1/16, beat or bar ([`SyncStart`]).
    pub(super) fn phrase_start_unit(&self) -> u32 {
        match (self.settings.start, self.sync) {
            (SyncStart::Step, _) => SIXTEENTH,
            (SyncStart::Beat, _) => PPQ,
            (SyncStart::Bar, Some(s)) => s.bar_ticks,
            (SyncStart::Bar, None) => PPQ * 4,
        }
    }

    /// Written ticks per clock tick ([`phrase_speed`]).
    fn phrase_factor(&self) -> f64 {
        phrase_speed(&self.settings, &self.live)
    }

    /// The phrase from its first note: on a multiple of `unit` ticks
    /// while the transport plays ([`Engine::start_line`]), else now. A
    /// phrase that was waiting for its line comes in with it.
    pub(super) fn start_phrase(&mut self, unit: u32) {
        if let Some(p) = self.next_phrase.take() {
            self.phrase = p;
        }
        let origin = match self.sync {
            Some(_) => self.start_line(i64::from(unit)) as f64,
            None => self.clock_tick(self.now as f64),
        };
        self.playhead.start(origin, self.phrase_factor());
    }

    /// The multiple of `unit` ticks a press now counts for, while the
    /// transport plays: the next one or, for a press no more than
    /// [`LATE_PRESS`] after one, that one.
    ///
    /// A step mode can start on the next line whatever the hand did: its
    /// steps are all alike. A phrase is written against the bar, so one
    /// pressed on the beat a moment late and started a 1/16 on would play
    /// every note a 1/16 off the beat. Started from the line the hand
    /// meant, it is in time from there, and only what the press was late
    /// for is late ([`Engine::play_phrase_note`]).
    fn start_line(&self, unit: i64) -> i64 {
        let unit = unit.max(1);
        let next = self.grid_after(unit);
        let last = next - unit;
        let late = self.now as f64 - self.frame_of(last as f64);
        let window =
            (LATE_PRESS * self.sample_rate).min(f64::from(SIXTEENTH / 2) * self.frames_per_tick());
        if late <= window {
            last
        } else {
            next
        }
    }

    /// The phrase again from its first note while it plays (see
    /// [`Engine::start_phrase`]): what has not started never does, and
    /// what still sounds ends where the phrase starts again.
    pub(super) fn restart_phrase(&mut self, unit: u32) {
        self.drop_pending_ons();
        self.start_phrase(unit);
        let at = self.clock_frame(self.playhead.origin());
        self.end_notes_by(at.max(self.now as f64));
    }

    /// Retrigger: a new note starts the phrase again, on the next 1/16
    /// while the transport plays, else now.
    pub(super) fn retrigger_phrase(&mut self) {
        self.restart_phrase(SIXTEENTH);
    }

    /// The phrase stops (nothing held, or the mode changed): what it has
    /// sounding ends now.
    pub(super) fn stop_phrase(&mut self) {
        self.end_notes_by(self.now as f64);
    }

    /// A new speed keeps the phrase's place: from now on it plays at the
    /// new speed from where it is.
    pub(super) fn respeed_phrase(&mut self) {
        let factor = self.phrase_factor();
        if (factor - self.playhead.factor()).abs() > 1e-12 {
            self.playhead
                .respeed(self.clock_tick(self.now as f64), factor);
        }
    }

    /// Plays every note of the phrase that starts before frame `end`, and
    /// brings a waiting phrase in on its line: the bar line with the
    /// transport, the next step without ([`Engine::set_phrase`]).
    pub(super) fn play_phrase(&mut self, end: u64) {
        let until = self.clock_tick(end as f64);
        let join = self.phrase_join();
        // A turn is a note or a phrase coming in: every note twice and a
        // swap at the very most in one period, so the loop always ends.
        for _ in 0..MAX_PHRASE_NOTES * 2 + 2 {
            let waiting = self.next_phrase.is_some().then_some(join);
            match self.playhead.turn(&self.phrase, waiting, until) {
                None => break,
                Some(Turn::Swap) => {
                    if let Some(p) = self.next_phrase.take() {
                        self.phrase = p;
                    }
                    let line = self.phrase_swung(self.playhead.origin());
                    self.end_notes_by(self.clock_frame(line));
                }
                Some(Turn::Note { index, at, pass }) => {
                    let repeats = u64::from(self.settings.repeats);
                    if repeats == 0 || pass < repeats {
                        self.play_phrase_note(index, at, pass);
                    }
                }
            }
        }
    }

    /// Note `i` of the phrase, `w` written ticks from where the phrase
    /// started, on its `pass`th time round, from the newest note held:
    /// its pitch In key or Exact; an octave more each time round, up to
    /// the octaves set; its length by the gate ([`Phrase::gated`]); its
    /// velocity as written times the held note's (100 plays it as
    /// written); then the modifier's rolls and accents.
    ///
    /// A note the press was late for ([`Engine::start_line`]) sounds at
    /// the press. It ends where it was written to, as a hand that lands
    /// late and lets go in time would end it, but keeps half its length
    /// at the least, so the first note of a short-gated phrase is still
    /// heard; and it never runs into the next note's start. A note whose
    /// place has wholly passed (the next one's start is behind the press
    /// too) is left out.
    fn play_phrase_note(&mut self, i: usize, w: f64, pass: u64) {
        let Some(held) = self.notes.active().as_slice().last().copied() else {
            return;
        };
        let n = self.phrase.notes()[i];
        let live = self.live;
        let scale = live.scale.or(self.key);
        let Some(pitch) = self
            .phrase
            .pitch(&n, held.note, live.follow, scale, live.snap)
        else {
            return;
        };
        let octave = (pass % u64::from(live.octaves.max(1))) as i16;
        let note = i16::from(pitch) + 12 * octave;
        if !(0..=127).contains(&note) {
            return;
        }
        let note = note as u8;
        // Its start and its end go through the same sums, so a note held
        // to the next one ends on the very frame that one starts.
        let t = self.playhead.tick_of(w);
        let t_off = self.playhead.tick_of(w + self.phrase.gated(i, live.gate));
        let mut at = self.clock_frame(self.phrase_swung(t));
        let mut off = self.clock_frame(self.phrase_swung(t_off));
        let now = self.now as f64;
        if at < now {
            let t_next = self.playhead.tick_of(w + self.phrase.gated(i, 100));
            let next = self.clock_frame(self.phrase_swung(t_next));
            if next <= now {
                self.step += 1;
                return;
            }
            off = off.max((now + (off - at) / 2.0).min(next));
            at = now;
        }
        let off = off.max(at + 1.0);
        let v = f64::from(n.velocity) * f64::from(self.velocity_of(held.velocity)) / 100.0;
        let v = self
            .settings
            .ramp
            .apply(v.round().clamp(1.0, 127.0) as u8, self.step);
        let hit = self.pushed_hit(Hit {
            velocity: Some(v),
            ..Hit::plain(100)
        });
        let velocity = hit.velocity.unwrap_or(v);
        self.end_note_by(note, at);
        if hit.ratchet > 1 {
            self.tied = 0.0;
            self.hit(note, velocity, at, off - at, hit.ratchet, 100);
        } else {
            self.queue_note(at, off, note, velocity);
        }
        self.step += 1;
    }

    /// Clock tick `t` moved by swing: the transport's while it plays with
    /// some, else the arp's own on 1/16s, counted from the transport's
    /// tick 0 while it plays and from where the phrase started while it
    /// doesn't.
    ///
    /// A phrase written on a triplet grid is left where it is. Swing
    /// moves the second of every two, so it would push the threes of a
    /// triplet apart unevenly (at 58 % on 1/16s, the second 1/8 triplet
    /// 13 ms late at 120 bpm and the third as much), and a triplet has
    /// its lilt already: played straight it sits with a swung beat, whose
    /// off-beats are near its third note.
    fn phrase_swung(&self, t: f64) -> f64 {
        if self.phrase.feel() == Feel::Triplet {
            return t;
        }
        let own = f64::from(SIXTEENTH);
        let (grid, amount, base) = match self.sync {
            Some(s) => match s.swing {
                Some((grid, amount)) => (f64::from(grid), amount, 0.0),
                None => (own, self.live.swing, 0.0),
            },
            None => (own, self.live.swing, self.playhead.started()),
        };
        phrase::swing(t, grid, amount, base)
    }

    /// Nothing the phrase has queued starts at or after `frame`, and
    /// nothing it has sounding goes on past it.
    fn end_notes_by(&mut self, frame: f64) {
        let mut i = 0;
        while i < self.queued {
            let p = &mut self.queue[i];
            match p.kind {
                EventKind::On { .. } if p.at >= frame => {
                    self.queued -= 1;
                    self.queue[i] = self.queue[self.queued];
                    continue;
                }
                EventKind::Off { .. } if p.at > frame => p.at = frame,
                _ => {}
            }
            i += 1;
        }
    }

    /// The same for one note, as it is struck again at `frame`: the one
    /// sounding ends there, so its ending can't cut the new one short.
    fn end_note_by(&mut self, note: u8, frame: f64) {
        let mut i = 0;
        while i < self.queued {
            let p = &mut self.queue[i];
            match p.kind {
                EventKind::On { note: n, .. } if n == note && p.at >= frame => {
                    self.queued -= 1;
                    self.queue[i] = self.queue[self.queued];
                    continue;
                }
                EventKind::Off { note: n } if n == note && p.at > frame => p.at = frame,
                _ => {}
            }
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{
        engine, notes_of, on_velocities, ons, run, run_synced, span, wheel, Span, SR, STEP,
    };
    use super::super::*;
    use crate::modifier::{ModSource, Modifier};
    use crate::notes::LatchMode;
    use crate::phrase::{Follow, Join, Phrase, Speed};
    use crate::scale::Scale;

    fn phrase_engine(text: &str, settings: Settings) -> Engine {
        let mut e = Engine::new(SR, 120.0);
        e.set_settings(Settings {
            mode: Mode::Phrase,
            ..settings
        });
        e.set_phrase(Phrase::parse(text).unwrap());
        e
    }

    fn on(note: u8, velocity: u8) -> EventKind {
        EventKind::On { note, velocity }
    }

    fn off(note: u8) -> EventKind {
        EventKind::Off { note }
    }

    const BAR: u64 = 16 * STEP;
    const EVERY_SIXTEENTH: &str = "bass either : 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1";

    /// A held note plays the whole phrase from it, in the phrase's own
    /// rhythm and lengths (gate 50 plays them as written: 80 % of a 1/16),
    /// from the press, and again from the next bar.
    #[test]
    fn a_held_note_plays_the_phrase_in_its_own_rhythm_from_that_note() {
        let mut e = phrase_engine("riff either : 1 . 5 1+ . . b7 .", Settings::default());
        e.press(60, 100);
        let got = run(&mut e, BAR + STEP, 128);
        let len = STEP * 8 / 10;
        assert_eq!(
            got,
            [
                (0, on(60, 100)),
                (len, off(60)),
                (2 * STEP, on(67, 100)),
                (2 * STEP + len, off(67)),
                (3 * STEP, on(72, 100)),
                (3 * STEP + len, off(72)),
                (6 * STEP, on(70, 100)),
                (6 * STEP + len, off(70)),
                (BAR, on(60, 100)),
                (BAR + len, off(60)),
            ]
        );
    }

    /// In key, a phrase moves by steps of the track's scale from the held
    /// note; Exact, by the semitones it was written with.
    #[test]
    fn in_key_moves_by_the_scale_and_exact_by_semitones() {
        let line = "line major : 1 2 3 4 5 6 7 1+";
        let c_minor = Settings {
            scale: Some(Scale::new(0, crate::scale::MINOR)),
            ..Settings::default()
        };
        let mut e = phrase_engine(line, c_minor);
        e.press(60, 100);
        assert_eq!(notes_of(&mut e, 8), [60, 62, 63, 65, 67, 68, 70, 72]);
        let mut e = phrase_engine(line, c_minor);
        e.press(62, 100);
        assert_eq!(notes_of(&mut e, 8), [62, 63, 65, 67, 68, 70, 72, 74]);
        let mut e = phrase_engine(
            line,
            Settings {
                follow: Follow::Exact,
                ..c_minor
            },
        );
        e.press(62, 100);
        assert_eq!(notes_of(&mut e, 8), [62, 64, 66, 67, 69, 71, 73, 74]);
        // No scale: by semitones too.
        let mut e = phrase_engine(line, Settings::default());
        e.press(60, 100);
        assert_eq!(notes_of(&mut e, 8), [60, 62, 64, 65, 67, 69, 71, 72]);
    }

    /// With the arp's own scale off, In key follows the key the track's
    /// pads play; the arp's scale, when on, wins; Exact takes no notice.
    #[test]
    fn in_key_follows_the_pads_key_when_the_arp_has_no_scale() {
        let line = "line major : 1 2 3 4 5 6 7 1+";
        let c_minor = Some(Scale::new(0, crate::scale::MINOR));
        let c_dorian = Some(Scale::new(0, crate::scale::DORIAN));
        let mut e = phrase_engine(line, Settings::default());
        e.set_key(c_minor);
        assert_eq!(e.key(), c_minor);
        e.press(60, 100);
        assert_eq!(notes_of(&mut e, 8), [60, 62, 63, 65, 67, 68, 70, 72]);
        let mut e = phrase_engine(
            line,
            Settings {
                scale: c_dorian,
                ..Settings::default()
            },
        );
        e.set_key(c_minor);
        e.press(60, 100);
        assert_eq!(notes_of(&mut e, 8), [60, 62, 63, 65, 67, 69, 70, 72]);
        let mut e = phrase_engine(
            line,
            Settings {
                follow: Follow::Exact,
                ..Settings::default()
            },
        );
        e.set_key(c_minor);
        e.press(60, 100);
        assert_eq!(notes_of(&mut e, 8), [60, 62, 64, 65, 67, 69, 71, 72]);
    }

    /// The newest note held leads; letting it go goes back to the one
    /// before. The phrase keeps its place throughout.
    #[test]
    fn the_newest_held_note_leads_and_the_phrase_keeps_its_place() {
        let mut e = phrase_engine(EVERY_SIXTEENTH, Settings::default());
        e.press(60, 100);
        let mut got = ons(&run(&mut e, 2 * STEP, 128));
        e.press(64, 100);
        got.extend(ons(&run(&mut e, 2 * STEP, 128)));
        e.release(64);
        got.extend(ons(&run(&mut e, 2 * STEP, 128)));
        assert_eq!(
            got,
            [
                (0, 60),
                (STEP, 60),
                (2 * STEP, 64),
                (3 * STEP, 64),
                (4 * STEP, 60),
                (5 * STEP, 60)
            ]
        );
    }

    /// Letting go stops the phrase and ends what it plays; Latch keeps it
    /// going after the pad is let go.
    #[test]
    fn letting_go_stops_the_phrase_and_latch_keeps_it_going() {
        let mut e = phrase_engine(EVERY_SIXTEENTH, Settings::default());
        e.press(60, 100);
        assert_eq!(ons(&run(&mut e, 2 * STEP, 128)).len(), 2);
        e.release(60);
        assert!(ons(&run(&mut e, 2 * STEP, 128)).is_empty());
        assert_eq!(e.sounding(), 0, "nothing left sounding");
        assert!(e.idle());
        e.set_latch(LatchMode::Replace);
        e.press(60, 100);
        e.release(60);
        assert_eq!(ons(&run(&mut e, 4 * STEP, 128)).len(), 4);
    }

    /// With Retrigger a new note starts the phrase again from its first
    /// note; without, the phrase carries on from where it is.
    #[test]
    fn retrigger_starts_the_phrase_again_from_its_first_note() {
        let scale = "line either : 1 2 3 4 5 6 7 1+";
        for (retrigger, want) in [(true, [67, 69, 71]), (false, [72, 74, 76])] {
            let mut e = phrase_engine(
                scale,
                Settings {
                    retrigger,
                    ..Settings::default()
                },
            );
            e.press(60, 100);
            assert_eq!(notes_of(&mut e, 3), [60, 62, 64]);
            e.press(67, 100);
            let got = ons(&run(&mut e, 3 * STEP, 128));
            assert_eq!(
                got,
                [
                    (3 * STEP, want[0]),
                    (4 * STEP, want[1]),
                    (5 * STEP, want[2])
                ],
                "retrigger {retrigger}"
            );
        }
    }

    /// With the transport, a phrase picked while another plays comes in
    /// on the next bar line, from its first note.
    #[test]
    fn with_the_transport_a_picked_phrase_comes_in_on_the_next_bar_line() {
        let spans = [span(0, 0, 120.0)];
        let mut e = phrase_engine(
            "bass either : 1 . . . 1 . . . 1 . . . 1 . . .",
            Settings::default(),
        );
        let first = ons(&run_synced(&mut e, &spans, &[(0, 60)], BAR + BAR / 2, 128));
        assert_eq!(first.len(), 6);
        assert_eq!(e.phrase_waiting(), None);
        e.set_phrase(Phrase::parse("riff either : 5 5 5 5").unwrap());
        assert_eq!(e.phrase_waiting(), Some(Join::Bar), "a screen can say so");
        let got = ons(&run_synced(&mut e, &spans, &[], 2 * BAR + BAR / 2, 128));
        assert_eq!(
            got,
            [
                (24 * STEP, 60),
                (28 * STEP, 60),
                (2 * BAR, 67),
                (2 * BAR + STEP, 67),
                (2 * BAR + 2 * STEP, 67),
                (2 * BAR + 3 * STEP, 67)
            ]
        );
        assert_eq!(e.phrase().len(), 4);
        assert_eq!(e.phrase_waiting(), None, "it is in");
    }

    /// With the transport stopped nothing else counts bars, so it comes
    /// in on the playing phrase's next step: a list of phrases is quick
    /// to hear through, and the pulse never stumbles.
    #[test]
    fn without_the_transport_a_picked_phrase_comes_in_on_the_next_step() {
        let mut e = phrase_engine(
            "bass either : 1 . . . 1 . . . 1 . . . 1 . . .",
            Settings::default(),
        );
        e.press(60, 100);
        let first = ons(&run(&mut e, 4 * STEP + STEP / 2, 128));
        assert_eq!(first, [(0, 60), (4 * STEP, 60)]);
        e.set_phrase(Phrase::parse("riff either : 5 5 5 5").unwrap());
        assert_eq!(e.phrase_waiting(), Some(Join::Step));
        let got = ons(&run(&mut e, BAR, 128));
        assert_eq!(
            got,
            [
                (5 * STEP, 67),
                (6 * STEP, 67),
                (7 * STEP, 67),
                (8 * STEP, 67)
            ]
        );
        // A phrase on 1/8s gives way on its own grid: the next 1/8.
        let mut e = phrase_engine("bass either 1/8 : 1 . . . 1 . . .", Settings::default());
        e.press(60, 100);
        run(&mut e, STEP / 2, 128);
        e.set_phrase(Phrase::parse("riff either : 5 . . .").unwrap());
        assert_eq!(ons(&run(&mut e, 4 * STEP, 128)), [(2 * STEP, 67)]);
    }

    /// The phrase that plays, picked again, plays on from where it is; and
    /// picked while another waits for the bar line, it stays and the other
    /// never comes in.
    #[test]
    fn picking_the_phrase_that_plays_changes_nothing() {
        let text = "bass either : 1 . . . 5 . . . 1 . . . 5 . . .";
        let playing = Phrase::parse(text).unwrap();
        let mut e = phrase_engine(text, Settings::default());
        e.press(60, 100);
        let mut same = phrase_engine(text, Settings::default());
        same.press(60, 100);
        run(&mut e, BAR / 2, 128);
        run(&mut same, BAR / 2, 128);
        e.set_phrase(playing);
        assert_eq!(run(&mut e, 2 * BAR, 128), run(&mut same, 2 * BAR, 128));
        e.set_phrase(Phrase::parse("riff either : 5 5 5 5").unwrap());
        e.set_phrase(playing);
        assert_eq!(run(&mut e, 2 * BAR, 128), run(&mut same, 2 * BAR, 128));
    }

    /// Half and Double play the phrase at half or twice its written speed;
    /// a modifier's push on the rate moves it by the same ratio, from where
    /// it is.
    #[test]
    fn speed_halves_or_doubles_the_rhythm_and_the_modifier_pushes_it() {
        for (speed, gap) in [
            (Speed::Half, 2 * STEP),
            (Speed::AsWritten, STEP),
            (Speed::Double, STEP / 2),
        ] {
            let mut e = phrase_engine(
                EVERY_SIXTEENTH,
                Settings {
                    speed,
                    ..Settings::default()
                },
            );
            e.press(60, 100);
            let t: Vec<u64> = ons(&run(&mut e, 4 * gap, 128))
                .iter()
                .map(|(t, _)| *t)
                .collect();
            assert_eq!(t, [0, gap, 2 * gap, 3 * gap], "{speed:?}");
        }
        let mut e = phrase_engine(
            EVERY_SIXTEENTH,
            Settings {
                modifier: wheel(Modifier::default()),
                ..Settings::default()
            },
        );
        e.press(60, 100);
        assert_eq!(ons(&run(&mut e, 2 * STEP, 128)).len(), 2);
        e.control(ModSource::ModWheel, 1.0);
        let t: Vec<u64> = ons(&run(&mut e, 2 * STEP, 128))
            .iter()
            .map(|(t, _)| *t)
            .collect();
        assert_eq!(
            t,
            [2 * STEP, 2 * STEP + STEP / 2, 3 * STEP, 3 * STEP + STEP / 2]
        );
        e.control(ModSource::ModWheel, 0.0);
        let t: Vec<u64> = ons(&run(&mut e, 2 * STEP, 128))
            .iter()
            .map(|(t, _)| *t)
            .collect();
        assert_eq!(t, [4 * STEP, 5 * STEP]);
    }

    /// The XY pad's hand holds the rate over the one as set, as a
    /// Modifier's push does, so while it holds a phrase plays faster or
    /// slower by the same ratio, from where it is, and as written again
    /// when the pad lets go. The settings as set stay as they are.
    #[test]
    fn the_xy_pads_hand_on_the_rate_moves_a_phrases_speed_until_it_lets_go() {
        let place = |r: Rate| Rate::ALL.iter().position(|x| *x == r).unwrap() as f32;
        let starts = |e: &mut Engine, frames: u64| -> Vec<u64> {
            ons(&run(e, frames, 128)).iter().map(|(t, _)| *t).collect()
        };
        let mut e = phrase_engine(EVERY_SIXTEENTH, Settings::default());
        let set = e.settings();
        assert_eq!(set.rate, Rate::Sixteenth);
        e.press(60, 100);
        assert_eq!(starts(&mut e, 2 * STEP), [0, STEP]);
        // Held at 1/32 over the 1/16 as set: twice as fast.
        e.hold(HandOn::Rate, Some(place(Rate::ThirtySecond)));
        assert_eq!(phrase_speed(&e.settings(), &e.live()), 2.0);
        assert_eq!(e.settings(), set, "the settings as set stay");
        assert_eq!(
            starts(&mut e, 2 * STEP),
            [2 * STEP, 2 * STEP + STEP / 2, 3 * STEP, 3 * STEP + STEP / 2]
        );
        // Held at 1/8: half speed, from where the phrase is.
        e.hold(HandOn::Rate, Some(place(Rate::Eighth)));
        assert_eq!(phrase_speed(&e.settings(), &e.live()), 0.5);
        assert_eq!(starts(&mut e, 4 * STEP), [4 * STEP, 6 * STEP]);
        // Let go: as written again.
        e.hold(HandOn::Rate, None);
        assert_eq!(e.live(), set);
        assert_eq!(starts(&mut e, 2 * STEP), [8 * STEP, 9 * STEP]);
    }

    /// The pad's hand on the phrase's own speed (its place in
    /// [`Speed::ALL`]): double while it holds, from where the phrase is,
    /// and the speed as set again when it lets go.
    #[test]
    fn the_xy_pads_hand_on_the_speed_holds_it_until_it_lets_go() {
        let starts = |e: &mut Engine, frames: u64| -> Vec<u64> {
            ons(&run(e, frames, 128)).iter().map(|(t, _)| *t).collect()
        };
        let mut e = phrase_engine(EVERY_SIXTEENTH, Settings::default());
        let set = e.settings();
        e.press(60, 100);
        assert_eq!(starts(&mut e, 2 * STEP), [0, STEP]);
        e.hold(HandOn::Speed, Some(2.0));
        assert_eq!(e.live().speed, Speed::Double);
        assert_eq!(e.settings(), set, "the settings as set stay");
        assert_eq!(starts(&mut e, STEP), [2 * STEP, 2 * STEP + STEP / 2]);
        e.hold(HandOn::Speed, None);
        assert_eq!(e.live(), set);
        assert_eq!(starts(&mut e, 2 * STEP), [3 * STEP, 4 * STEP]);
    }

    /// With Octaves above 1, each time through the phrase climbs an octave,
    /// then starts again from the bottom.
    #[test]
    fn octaves_climb_an_octave_each_time_through() {
        let mut e = phrase_engine(
            "bass either : 1",
            Settings {
                octaves: 2,
                ..Settings::default()
            },
        );
        e.press(60, 100);
        let got = ons(&run(&mut e, 3 * BAR, 128));
        assert_eq!(got, [(0, 60), (BAR, 72), (2 * BAR, 60)]);
    }

    /// Gate 50 plays the lengths as written, less shortens them, more
    /// holds them towards the next note; at 100 a note ends on the frame
    /// the next one starts, before it, so the same note played again is
    /// never cut.
    #[test]
    fn the_gate_shortens_notes_or_holds_them_to_the_next() {
        let phrase = "bass either : 1 . 1 .";
        let written = STEP * 8 / 10;
        for (gate, len) in [
            (25, written / 2),
            (50, written),
            (75, (written + 2 * STEP) / 2),
        ] {
            let mut e = phrase_engine(
                phrase,
                Settings {
                    gate,
                    ..Settings::default()
                },
            );
            e.press(60, 100);
            let got = run(&mut e, 2 * STEP, 128);
            assert_eq!(got, [(0, on(60, 100)), (len, off(60))], "gate {gate}");
        }
        let mut e = phrase_engine(
            phrase,
            Settings {
                gate: 100,
                ..Settings::default()
            },
        );
        e.press(60, 100);
        let got = run(&mut e, BAR + 1, 128);
        assert_eq!(
            got,
            [
                (0, on(60, 100)),
                (2 * STEP, off(60)),
                (2 * STEP, on(60, 100)),
                (BAR, off(60)),
                (BAR, on(60, 100)),
            ]
        );
    }

    /// The held note's velocity scales the phrase's own: held at 100 it
    /// plays as written, accents and all.
    #[test]
    fn the_held_velocity_scales_the_written_velocities() {
        let phrase = "riff either : 1! 1 1 | vel 0 50 0";
        let mut e = phrase_engine(phrase, Settings::default());
        e.press(60, 100);
        assert_eq!(on_velocities(&run(&mut e, 3 * STEP, 128)), [127, 50, 100]);
        let mut e = phrase_engine(phrase, Settings::default());
        e.press(60, 50);
        assert_eq!(on_velocities(&run(&mut e, 3 * STEP, 128)), [64, 25, 50]);
        let mut e = phrase_engine(
            phrase,
            Settings {
                velocity: Velocity::Fixed(100),
                ..Settings::default()
            },
        );
        e.press(60, 20);
        assert_eq!(on_velocities(&run(&mut e, 3 * STEP, 128)), [127, 50, 100]);
    }

    /// With the transport playing, a phrase starts on its next 1/16, beat
    /// or bar (the Start setting) and stays on the transport's grid.
    #[test]
    fn with_the_transport_a_phrase_starts_on_the_next_sixteenth_beat_or_bar() {
        let spans = [span(0, 0, 120.0)];
        for (start, first) in [
            (SyncStart::Step, STEP),
            (SyncStart::Beat, 4 * STEP),
            (SyncStart::Bar, BAR),
        ] {
            let mut e = phrase_engine(
                EVERY_SIXTEENTH,
                Settings {
                    start,
                    ..Settings::default()
                },
            );
            // Pressed 4000 frames (83 ms) after the bar line.
            let got = run_synced(&mut e, &spans, &[(4000, 60)], first + 2 * STEP + 1, 128);
            let t: Vec<u64> = ons(&got).iter().map(|(t, _)| *t).collect();
            assert_eq!(t, [first, first + STEP, first + 2 * STEP], "{start:?}");
        }
    }

    /// A press a moment after a start line counts for that line, as the
    /// hand meant it: the note it was late for sounds at the press and
    /// ends where it was written to, and the rest is in time from the
    /// line. (Started a 1/16 on, every note would be a 1/16 off the beat.)
    #[test]
    fn a_press_a_moment_late_still_starts_on_its_line() {
        let spans = [span(0, 0, 120.0)];
        for start in SyncStart::ALL {
            let mut e = phrase_engine(
                EVERY_SIXTEENTH,
                Settings {
                    start,
                    ..Settings::default()
                },
            );
            // 1000 frames (21 ms) after the bar line.
            let got = run_synced(&mut e, &spans, &[(1000, 60)], 2 * STEP + 1, 128);
            assert_eq!(
                got,
                [
                    (1000, on(60, 100)),
                    (STEP * 8 / 10, off(60)),
                    (STEP, on(60, 100)),
                    (STEP + STEP * 8 / 10, off(60)),
                    (2 * STEP, on(60, 100)),
                ],
                "{start:?}"
            );
        }

        // 60 ms is the most (2880 frames here): a frame later waits for
        // the next 1/16.
        for (press, first) in [(2880, 2880), (2881, STEP)] {
            let mut e = phrase_engine(EVERY_SIXTEENTH, Settings::default());
            let got = run_synced(&mut e, &spans, &[(press, 60)], STEP + 1, 128);
            assert_eq!(ons(&got)[0], (first, 60), "pressed at {press}");
        }

        // A short note whose written end is behind the press keeps half
        // its length (gate 10: a fifth of the written 4800 frames).
        let mut e = phrase_engine(
            EVERY_SIXTEENTH,
            Settings {
                gate: 10,
                ..Settings::default()
            },
        );
        let got = run_synced(&mut e, &spans, &[(1000, 60)], STEP, 128);
        assert_eq!(got, [(1000, on(60, 100)), (1480, off(60))]);

        // A note whose place has wholly passed is left out: on 1/32s at
        // double speed a step is 1500 frames, and the press comes in the
        // second one.
        let mut e = phrase_engine(
            "line either 1/32 : 1 2 3 4 5 6 7 1+",
            Settings {
                speed: Speed::Double,
                ..Settings::default()
            },
        );
        let got = run_synced(&mut e, &spans, &[(2000, 60)], 4501, 128);
        assert_eq!(ons(&got), [(2000, 62), (3000, 64), (4500, 65)]);
    }

    /// A jump of the transport (a loop going round, a locate) starts the
    /// phrase again from its first note on the transport's grid.
    #[test]
    fn a_transport_jump_starts_the_phrase_again() {
        let spans = [
            span(0, 0, 120.0),
            Span {
                jumped: true,
                ..span(3 * STEP + 3000, 0, 120.0)
            },
        ];
        let mut e = phrase_engine("line either : 1 2 3 4 5 6 7 1+", Settings::default());
        let got = run_synced(&mut e, &spans, &[(0, 60)], 6 * STEP + 3001, 128);
        assert_eq!(
            ons(&got),
            [
                (0, 60),
                (STEP, 62),
                (2 * STEP, 64),
                (3 * STEP, 65),
                // The jump lands on tick 0 at 3 1/2 steps: the phrase
                // starts there, its first note on that 1/16.
                (3 * STEP + 3000, 60),
                (4 * STEP + 3000, 62),
                (5 * STEP + 3000, 64),
                (6 * STEP + 3000, 65),
            ]
        );
    }

    /// Free-running, the arp's swing moves every second 1/16 of the phrase
    /// later, counted from the press.
    #[test]
    fn swing_moves_every_second_sixteenth_of_the_phrase() {
        let mut e = phrase_engine(
            EVERY_SIXTEENTH,
            Settings {
                swing: 66,
                ..Settings::default()
            },
        );
        e.press(60, 100);
        let t: Vec<u64> = ons(&run(&mut e, 4 * STEP, 128))
            .iter()
            .map(|(t, _)| *t)
            .collect();
        assert_eq!(t, [0, 7920, 2 * STEP, 2 * STEP + 7920]);
    }

    /// A phrase on a triplet grid plays straight whatever swings: the
    /// arp's own swing, or the transport's.
    #[test]
    fn swing_leaves_a_triplet_phrase_alone() {
        let triplets = "line either 1/8T : 1 1 1 1 1 1";
        // A triplet 1/8 at 120 bpm is 8000 frames.
        let even = [0, 8_000, 16_000, 24_000, 32_000, 40_000];
        let mut e = phrase_engine(
            triplets,
            Settings {
                swing: 66,
                ..Settings::default()
            },
        );
        e.press(60, 100);
        let t: Vec<u64> = ons(&run(&mut e, 8 * STEP, 128))
            .iter()
            .map(|(t, _)| *t)
            .collect();
        assert_eq!(t, even, "the arp's swing");
        let mut e = phrase_engine(triplets, Settings::default());
        let swung = Span {
            swing: Some((PPQ / 4, 62)),
            ..span(0, 0, 120.0)
        };
        let t: Vec<u64> = ons(&run_synced(&mut e, &[swung], &[(0, 60)], 8 * STEP, 128))
            .iter()
            .map(|(t, _)| *t)
            .collect();
        assert_eq!(t, even, "the transport's swing");
        // The same swing does move a phrase written on 1/16s.
        let mut e = phrase_engine(EVERY_SIXTEENTH, Settings::default());
        let t: Vec<u64> = ons(&run_synced(&mut e, &[swung], &[(0, 60)], 2 * STEP, 128))
            .iter()
            .map(|(t, _)| *t)
            .collect();
        assert_eq!(t, [0, 7_440]);
    }

    /// Repeats sets how many times the phrase plays; then it stays quiet
    /// until the notes are let go and played again.
    #[test]
    fn repeats_play_the_phrase_that_many_times() {
        let mut e = phrase_engine(
            "bass either : 1 . 1 .",
            Settings {
                repeats: 2,
                ..Settings::default()
            },
        );
        e.press(60, 100);
        let got = ons(&run(&mut e, 3 * BAR, 128));
        assert_eq!(
            got,
            [(0, 60), (2 * STEP, 60), (BAR, 60), (BAR + 2 * STEP, 60)]
        );
        e.release(60);
        run(&mut e, STEP, 128);
        e.press(62, 100);
        assert_eq!(ons(&run(&mut e, STEP, 128)), [(3 * BAR + STEP, 62)]);
    }

    /// Changing to Phrase mode while notes are held starts the phrase at
    /// once; every note any of it played has ended once it stops.
    #[test]
    fn changing_to_phrase_mode_starts_it_and_nothing_is_left_sounding() {
        let mut e = engine(Mode::Arp);
        e.set_phrase(Phrase::parse("riff either : 1 5 1+ 5").unwrap());
        e.press(60, 100);
        e.press(64, 100);
        run(&mut e, STEP + STEP / 2, 128);
        e.set_settings(Settings {
            mode: Mode::Phrase,
            gate: 100,
            ..e.settings()
        });
        let got = ons(&run(&mut e, 2 * STEP, 128));
        assert_eq!(got, [(STEP + STEP / 2, 64), (2 * STEP + STEP / 2, 71)]);
        e.set_phrase(Phrase::parse("chords either : 1,3,5 . 1,3,5 .").unwrap());
        run(&mut e, BAR, 128);
        e.release(60);
        e.release(64);
        run(&mut e, BAR, 128);
        assert_eq!(e.sounding(), 0);
        assert!(e.idle());
    }
    /// A host that syncs a stopped transport every period, at an odd
    /// period size, gets the phrase on the same frames as one that never
    /// syncs; when the transport stops, the phrase carries on from where
    /// it was, in time.
    #[test]
    fn the_phrase_keeps_time_free_running_and_when_the_transport_stops() {
        let mut e = phrase_engine(EVERY_SIXTEENTH, Settings::default());
        e.press(60, 100);
        let mut got = Vec::new();
        let mut out = Events::new();
        while e.now < 4 * STEP {
            let base = e.now;
            e.sync(&Clock {
                playing: false,
                ..Clock::playing(0, 0, 120.0)
            });
            e.set_tempo(120.0);
            e.process(77, &mut out);
            got.extend(ons(&out
                .as_slice()
                .iter()
                .map(|ev| (base + u64::from(ev.offset), ev.kind))
                .collect::<Vec<_>>()));
        }
        let t: Vec<u64> = got
            .iter()
            .map(|(t, _)| *t)
            .filter(|t| *t < 4 * STEP)
            .collect();
        assert_eq!(t, [0, STEP, 2 * STEP, 3 * STEP]);
        // Playing from tick 0 on frame 0, then stopped at 2 1/2 steps.
        let spans = [
            span(0, 0, 120.0),
            Span {
                playing: false,
                ..span(2 * STEP + STEP / 2, 0, 120.0)
            },
        ];
        let mut e = phrase_engine(EVERY_SIXTEENTH, Settings::default());
        let got = run_synced(&mut e, &spans, &[(0, 60)], 5 * STEP + 1, 128);
        let t: Vec<u64> = ons(&got).iter().map(|(t, _)| *t).collect();
        assert_eq!(t, [0, STEP, 2 * STEP, 3 * STEP, 4 * STEP, 5 * STEP]);
    }
    /// Letting go ends a long note at once: the phrase stops with the
    /// hand, not when its notes run out.
    #[test]
    fn letting_go_ends_a_long_note_at_once() {
        let mut e = phrase_engine("chords either : 1,5~ _ _ _ _ _ _ _", Settings::default());
        e.press(60, 100);
        assert_eq!(
            run(&mut e, 2 * STEP, 128),
            [(0, on(60, 100)), (0, on(67, 100))]
        );
        e.release(60);
        assert_eq!(
            run(&mut e, STEP, 128),
            [(2 * STEP, off(60)), (2 * STEP, off(67))]
        );
        assert_eq!(e.sounding(), 0);
        assert!(e.idle());
    }

    /// The same note struck again ends the one sounding there, so the
    /// first one's ending can't cut the second short.
    #[test]
    fn the_same_note_struck_again_ends_the_one_sounding() {
        // A fifth held for five 1/16s, and the home note four 1/16s in.
        let unit = Rate::Sixteenth.ticks();
        let p = Phrase::from_notes(
            &[(0, 5 * unit, 67, 100), (4 * unit, 3 * unit, 60, 100)],
            60,
            crate::phrase::Written::Either,
            Rate::Sixteenth,
            4,
        );
        let mut e = Engine::new(SR, 120.0);
        e.set_settings(Settings {
            mode: Mode::Phrase,
            ..Settings::default()
        });
        e.set_phrase(p);
        e.press(60, 100);
        let mut got = run(&mut e, 2 * STEP, 128);
        // Hold the fifth: now the home note is the note still sounding.
        e.press(67, 100);
        got.extend(run(&mut e, 6 * STEP, 128));
        assert_eq!(
            got,
            [
                (0, on(67, 100)),
                (4 * STEP, off(67)),
                (4 * STEP, on(67, 100)),
                (7 * STEP, off(67)),
            ]
        );
    }

    /// Starting again (Retrigger here) ends what still sounds where the
    /// phrase starts again: at once free-running; with the transport on
    /// the next 1/16, or at once for a note pressed a moment after one.
    #[test]
    fn starting_again_ends_what_still_sounds() {
        let phrase = "chords either : 1~ _ _ _ _ _ _ _";
        let retrigger = Settings {
            retrigger: true,
            ..Settings::default()
        };
        let mut e = phrase_engine(phrase, retrigger);
        e.press(60, 100);
        let mut got = run(&mut e, 2 * STEP, 128);
        e.press(64, 100);
        got.extend(run(&mut e, 2 * STEP, 128));
        assert_eq!(
            got,
            [
                (0, on(60, 100)),
                (2 * STEP, off(60)),
                (2 * STEP, on(64, 100))
            ]
        );
        // The new note comes 4000 frames after a 1/16, then 144 after.
        for (press, again) in [(STEP + 4000, 2 * STEP), (STEP + 144, STEP + 144)] {
            let mut e = phrase_engine(phrase, retrigger);
            let got = run_synced(
                &mut e,
                &[span(0, 0, 120.0)],
                &[(0, 60), (press, 64)],
                3 * STEP,
                128,
            );
            assert_eq!(
                got,
                [(0, on(60, 100)), (again, off(60)), (again, on(64, 100))],
                "pressed at {press}"
            );
        }
    }

    /// A phrase coming in on a bar line ends the notes of the one before
    /// there.
    #[test]
    fn a_phrase_coming_in_ends_the_old_ones_notes_on_its_bar_line() {
        // Two bars: a note from the last quarter of bar 1, held into bar 2.
        let mut e = phrase_engine(
            "chords either 2 bars : . . . . . . . . . . . . 1~ _ _ _ _ _ _ _",
            Settings::default(),
        );
        let spans = [span(0, 0, 120.0)];
        let mut got = run_synced(&mut e, &spans, &[(0, 60)], 13 * STEP, 128);
        e.set_phrase(Phrase::parse("riff either : 5 . . .").unwrap());
        got.extend(run_synced(&mut e, &spans, &[], 17 * STEP, 128));
        assert_eq!(
            got,
            [
                (12 * STEP, on(60, 100)),
                (BAR, off(60)),
                (BAR, on(67, 100)),
                (BAR + STEP * 8 / 10, off(67)),
            ]
        );
    }
}
