//! Answer's ears (revision 3, S5): when a call has ended, when the answer
//! plays, and when you take your turn back.
//!
//! Answer is set on the Phrase tab with Plays: When I stop. You play a
//! phrase and stop; after a beat of quiet the call is over, and the answer
//! plays from the next bar line, or from the next beat after **Answer
//! now**. Trade takes turns instead: your turn is 1, 2 or 4 bars from the
//! bar line before your first note, then the answer's turn as long, then
//! yours again. Play while the answer plays and it stops making new notes
//! at once (held notes get a quick release); play after it was asked for
//! but before it began and it is dropped, never played over you.
//!
//! The ears only say when. The answer is made on a worker
//! (`mpfree_compose::answer`) from the call kept here, and handed back with
//! [`Ears::ready`]; each one carries an id, so a late answer for a call
//! you have since played over is dropped. Notes on another route (another
//! track or MIDI channel) never join the call; they only make it wait
//! (contract 9). Fixed size, no allocation: the ears sit in the audio
//! thread beside the arp engine.
//!
//! Times are ticks at [`crate::PPQ`], from any start, never going back
//! (a locate or a loop wrap is a [`Ears::reset`]).

/// Most notes a call keeps; past it the oldest go (an answer reads only
/// a call's last 4 bars).
pub const MAX_CALL_NOTES: usize = 256;

/// When the answer comes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turns {
    /// After you stop: a beat of quiet, then the next bar line.
    WhenIStop,
    /// Turns of 1, 2 or 4 bars.
    Trade(u8),
}

impl Turns {
    pub const fn words(self) -> &'static str {
        match self {
            Turns::WhenIStop => "When I stop",
            Turns::Trade(1) => "Trade every bar",
            Turns::Trade(2) => "Trade every 2 bars",
            Turns::Trade(_) => "Trade every 4 bars",
        }
    }
}

/// The turn light along the strip: you played, quiet, answer, your turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Light {
    /// Nothing played yet, or the answer is over: your turn.
    YourTurn,
    /// You are playing.
    You,
    /// You stopped; a beat of quiet ends the call.
    Quiet,
    /// The answer is being made, or waits for its bar line.
    Coming,
    /// The answer plays.
    Answer,
}

/// A note of the call, in ticks from the bar line it started after.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CallNote {
    pub at: i64,
    /// Up to now while the note is held.
    pub len: i64,
    pub pitch: u8,
    pub velocity: u8,
}

/// What the ears say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Heard {
    /// Make answer `id` from [`Ears::call`]; it is to play from `at`.
    Ask { id: u32, at: i64 },
    /// Answer `id` plays from `at` for `len` ticks.
    Play { id: u32, at: i64, len: i64 },
    /// You played over answer `id`: no new notes; held notes get a quick
    /// release.
    Stop { id: u32 },
    /// Answer `id` is not wanted any more: you played before it began.
    Drop { id: u32 },
    /// Answer `id` has played out.
    Done { id: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Waiting for you.
    Idle,
    /// You are playing a call.
    Calling,
    /// Answer `id` asked for, to play from `at`; `len` once ready.
    Asked { id: u32, at: i64, len: Option<i64> },
    /// Answer `id` plays until `until`.
    Answering { id: u32, until: i64 },
}

/// Answer's ears for one track.
#[derive(Clone, Debug)]
pub struct Ears {
    turns: Turns,
    bar: i64,
    beat: i64,
    /// How long before its bar line an answer is asked for in Trade, and
    /// the least time between asking and playing: a 1/16.
    lead: i64,
    state: State,
    call: [CallNote; MAX_CALL_NOTES],
    count: usize,
    /// The call's bar line (absolute ticks).
    from: i64,
    /// Where each held pitch's note is in the call, plus one (0: not held).
    held: [u16; 128],
    holding: u8,
    /// When the last note ended, or a note elsewhere was played.
    last: i64,
    id: u32,
}

impl Ears {
    /// Ears for a meter of `bar` and `beat` ticks.
    pub fn new(turns: Turns, bar: i64, beat: i64) -> Ears {
        let bar = bar.max(1);
        let beat = beat.clamp(1, bar);
        Ears {
            turns,
            bar,
            beat,
            lead: (crate::PPQ as i64 / 4).min(beat),
            state: State::Idle,
            call: [CallNote::default(); MAX_CALL_NOTES],
            count: 0,
            from: 0,
            held: [0; 128],
            holding: 0,
            last: 0,
            id: 0,
        }
    }

    pub fn turns(&self) -> Turns {
        self.turns
    }

    /// Change when the answer comes; takes effect from your next call.
    pub fn set_turns(&mut self, turns: Turns) {
        self.turns = turns;
    }

    /// The call as it stands, in ticks from its bar line.
    pub fn call(&self) -> &[CallNote] {
        &self.call[..self.count]
    }

    /// The call's bar line, in absolute ticks.
    pub fn call_from(&self) -> i64 {
        self.from
    }

    /// A transport stop, locate or loop wrap: everything forgotten. Says
    /// which answer to stop, if one was playing or coming.
    pub fn reset(&mut self) -> Option<Heard> {
        let out = match self.state {
            State::Answering { id, .. } => Some(Heard::Stop { id }),
            State::Asked { id, .. } => Some(Heard::Drop { id }),
            _ => None,
        };
        self.state = State::Idle;
        self.forget();
        out
    }

    /// A note you played on this track.
    pub fn note_on(&mut self, now: i64, pitch: u8, velocity: u8) -> Option<Heard> {
        let out = match self.state {
            State::Answering { id, .. } => {
                self.state = State::Idle;
                Some(Heard::Stop { id })
            }
            // In Trade, a note just before the answer's turn is the end of
            // yours: heard, not answered.
            State::Asked { at, .. } if matches!(self.turns, Turns::Trade(_)) && now < at => {
                return None;
            }
            State::Asked { id, .. } => {
                self.state = State::Idle;
                Some(Heard::Drop { id })
            }
            _ => None,
        };
        if self.state == State::Idle {
            self.forget();
            self.from = now.div_euclid(self.bar) * self.bar;
            self.state = State::Calling;
        }
        let p = usize::from(pitch & 0x7F);
        if self.held[p] != 0 {
            // The same pitch again: the note before it ends here.
            self.end(p, now);
        }
        if self.count == MAX_CALL_NOTES {
            self.call.copy_within(1.., 0);
            self.count -= 1;
            for h in self.held.iter_mut().filter(|h| **h != 0) {
                *h -= 1;
                // A held note pushed out of the call is let go.
                if *h == 0 {
                    self.holding = self.holding.saturating_sub(1);
                }
            }
        }
        self.call[self.count] = CallNote {
            at: now - self.from,
            len: 0,
            pitch: pitch & 0x7F,
            velocity,
        };
        self.count += 1;
        self.held[p] = self.count as u16;
        self.holding += 1;
        self.last = now;
        out
    }

    /// A note you let go of.
    pub fn note_off(&mut self, now: i64, pitch: u8) {
        let p = usize::from(pitch & 0x7F);
        if self.held[p] != 0 {
            self.end(p, now);
            self.last = self.last.max(now);
        }
    }

    /// A note played on another route: the call waits a beat more.
    pub fn elsewhere(&mut self, now: i64) {
        if self.state == State::Calling {
            self.last = self.last.max(now);
        }
    }

    /// Answer now: ask at once, to play from the next beat.
    pub fn answer_now(&mut self, now: i64) -> Option<Heard> {
        if self.state != State::Calling || self.count == 0 {
            return None;
        }
        let at = Self::next_line(now + self.lead, self.beat);
        self.ask(now, at)
    }

    /// Time has moved on to `now`; call once a block.
    pub fn tick(&mut self, now: i64) -> Option<Heard> {
        match self.state {
            State::Idle => None,
            State::Calling => match self.turns {
                Turns::WhenIStop => {
                    if self.holding > 0 || now < self.last + self.beat {
                        return None;
                    }
                    let at = Self::next_line(now + self.lead, self.bar);
                    self.ask(now, at)
                }
                Turns::Trade(n) => {
                    let end = self.from + self.turn_len(n);
                    if now < end - self.lead {
                        return None;
                    }
                    self.ask(now, end)
                }
            },
            State::Asked { id, at, len } => {
                if now < at {
                    return None;
                }
                match len {
                    // Ready, and its bar line has come: it plays.
                    Some(len) => {
                        self.state = State::Answering {
                            id,
                            until: at + len,
                        };
                        None
                    }
                    // Not ready by its bar line: the next one.
                    None => {
                        let at = Self::next_line(now + self.lead, self.bar);
                        self.state = State::Asked { id, at, len };
                        None
                    }
                }
            }
            State::Answering { id, until } => {
                if now < until {
                    return None;
                }
                self.forget();
                match self.turns {
                    // Your turn starts as the answer's ends.
                    Turns::Trade(_) => {
                        self.from = until;
                        self.state = State::Calling;
                    }
                    Turns::WhenIStop => self.state = State::Idle,
                }
                Some(Heard::Done { id })
            }
        }
    }

    /// Answer `id` is made and lasts `len` ticks. Says when it plays, or
    /// nothing when it is no longer wanted.
    pub fn ready(&mut self, id: u32, len: i64, now: i64) -> Option<Heard> {
        let State::Asked { id: asked, at, .. } = self.state else {
            return None;
        };
        if asked != id {
            return None;
        }
        let len = match self.turns {
            Turns::Trade(n) => self.turn_len(n),
            Turns::WhenIStop => len.max(1),
        };
        let at = if now <= at {
            at
        } else {
            Self::next_line(now + self.lead, self.bar)
        };
        self.state = State::Asked {
            id,
            at,
            len: Some(len),
        };
        Some(Heard::Play { id, at, len })
    }

    /// Answer `id` could not be made (nothing to answer): your turn again.
    pub fn refused(&mut self, id: u32) {
        if matches!(self.state, State::Asked { id: a, .. } if a == id) {
            self.forget();
            self.state = State::Idle;
        }
    }

    /// The turn light at `now`.
    pub fn light(&self, now: i64) -> Light {
        match self.state {
            State::Idle => Light::YourTurn,
            State::Calling if self.count == 0 => Light::YourTurn,
            State::Calling if self.holding > 0 || now < self.last + self.beat / 4 => Light::You,
            State::Calling => match self.turns {
                Turns::WhenIStop => Light::Quiet,
                Turns::Trade(_) => Light::You,
            },
            State::Asked { .. } => Light::Coming,
            State::Answering { .. } => Light::Answer,
        }
    }

    fn turn_len(&self, n: u8) -> i64 {
        i64::from(n.clamp(1, 4)) * self.bar
    }

    fn ask(&mut self, now: i64, at: i64) -> Option<Heard> {
        // Held notes count up to now.
        for p in 0..128 {
            if self.held[p] != 0 {
                let i = usize::from(self.held[p] - 1);
                self.call[i].len = (now - self.from - self.call[i].at).max(1);
            }
        }
        if self.count == 0 {
            // A turn with nothing played: wait for you.
            self.state = State::Idle;
            return None;
        }
        self.id = self.id.wrapping_add(1);
        self.state = State::Asked {
            id: self.id,
            at,
            len: None,
        };
        Some(Heard::Ask { id: self.id, at })
    }

    fn end(&mut self, p: usize, now: i64) {
        let i = usize::from(self.held[p] - 1);
        self.call[i].len = (now - self.from - self.call[i].at).max(1);
        self.held[p] = 0;
        self.holding = self.holding.saturating_sub(1);
    }

    fn forget(&mut self) {
        self.count = 0;
        self.held = [0; 128];
        self.holding = 0;
    }

    /// The first multiple of `unit` at or after `t`.
    fn next_line(t: i64, unit: i64) -> i64 {
        let unit = unit.max(1);
        (t + unit - 1).div_euclid(unit) * unit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BEAT: i64 = crate::PPQ as i64;
    const BAR: i64 = 4 * BEAT;
    const S16: i64 = BEAT / 4;

    /// Plays `(start, len, pitch)` notes from `from`, ticking every 1/64,
    /// and returns everything heard, with the time it was heard.
    fn play(ears: &mut Ears, notes: &[(i64, i64, u8)], until: i64) -> Vec<(i64, Heard)> {
        let mut out = Vec::new();
        let step = BEAT / 16;
        let mut t = 0;
        while t <= until {
            for &(on, len, pitch) in notes {
                if on + len == t {
                    ears.note_off(t, pitch);
                }
            }
            for &(on, _, pitch) in notes {
                if on == t {
                    if let Some(h) = ears.note_on(t, pitch, 90) {
                        out.push((t, h));
                    }
                }
            }
            if let Some(h) = ears.tick(t) {
                out.push((t, h));
            }
            t += step;
        }
        out
    }

    #[test]
    fn a_beat_of_quiet_ends_the_call_and_the_answer_plays_from_the_next_bar_line() {
        let mut e = Ears::new(Turns::WhenIStop, BAR, BEAT);
        assert_eq!(e.light(0), Light::YourTurn);
        // A bar and a half from beat 2 of bar 1, the last note ending at
        // bar 3's beat 1 and a half.
        let notes = [
            (BEAT, BEAT, 60),
            (2 * BEAT, BEAT / 2, 63),
            (3 * BEAT, 2 * BEAT, 67),
            (5 * BEAT + BEAT / 2, BEAT, 65),
        ];
        let heard = play(&mut e, &notes, 8 * BEAT - 1);
        let end = 5 * BEAT + BEAT / 2 + BEAT;
        assert_eq!(heard, [(end + BEAT, Heard::Ask { id: 1, at: 2 * BAR })]);
        assert_eq!(e.call_from(), 0);
        let call = e.call();
        assert_eq!(call.len(), 4);
        assert_eq!(
            call[0],
            CallNote {
                at: BEAT,
                len: BEAT,
                pitch: 60,
                velocity: 90
            }
        );
        assert_eq!(call[3].len, BEAT);
        assert_eq!(e.light(8 * BEAT - 1), Light::Coming);
        // Ready in time: it plays from bar 3 for its two bars, then it is
        // your turn.
        assert_eq!(
            e.ready(1, 2 * BAR, 8 * BEAT - 1),
            Some(Heard::Play {
                id: 1,
                at: 2 * BAR,
                len: 2 * BAR
            })
        );
        assert_eq!(e.tick(2 * BAR), None);
        assert_eq!(e.light(2 * BAR), Light::Answer);
        assert_eq!(e.tick(4 * BAR - 1), None);
        assert_eq!(e.tick(4 * BAR), Some(Heard::Done { id: 1 }));
        assert_eq!(e.light(4 * BAR), Light::YourTurn);
        assert!(e.call().is_empty());
    }

    #[test]
    fn a_held_note_or_a_note_elsewhere_keeps_the_call_going() {
        let mut e = Ears::new(Turns::WhenIStop, BAR, BEAT);
        e.note_on(0, 60, 90);
        assert_eq!(e.tick(3 * BEAT), None, "still held");
        assert_eq!(e.light(3 * BEAT), Light::You);
        e.note_off(3 * BEAT, 60);
        assert_eq!(e.light(3 * BEAT + BEAT / 2), Light::Quiet);
        e.elsewhere(3 * BEAT + BEAT / 2);
        assert_eq!(e.tick(4 * BEAT), None, "a note on another track waits");
        assert_eq!(e.call().len(), 1, "and never joins the call");
        assert_eq!(
            e.tick(4 * BEAT + BEAT / 2),
            Some(Heard::Ask { id: 1, at: 2 * BAR }),
            "quiet a beat after the note elsewhere; bar 2's line is too near"
        );
    }

    #[test]
    fn answer_now_asks_at_once_for_the_next_beat() {
        let mut e = Ears::new(Turns::WhenIStop, BAR, BEAT);
        assert_eq!(e.answer_now(0), None, "nothing played");
        e.note_on(0, 60, 90);
        e.note_off(BEAT / 2, 60);
        e.note_on(BEAT, 62, 90);
        // Pressed with the note still down: it counts to now.
        assert_eq!(
            e.answer_now(BEAT + BEAT / 2),
            Some(Heard::Ask {
                id: 1,
                at: 2 * BEAT
            })
        );
        assert_eq!(e.call()[1].len, BEAT / 2);
        // Pressed just before a beat: the one after.
        let mut e = Ears::new(Turns::WhenIStop, BAR, BEAT);
        e.note_on(0, 60, 90);
        assert_eq!(
            e.answer_now(2 * BEAT - 10),
            Some(Heard::Ask {
                id: 1,
                at: 3 * BEAT
            })
        );
    }

    #[test]
    fn playing_over_the_answer_stops_it_and_playing_before_it_drops_it() {
        let mut e = Ears::new(Turns::WhenIStop, BAR, BEAT);
        e.note_on(0, 60, 90);
        e.note_off(BEAT, 60);
        assert_eq!(e.tick(2 * BEAT), Some(Heard::Ask { id: 1, at: BAR }));
        // You play again before it began: dropped, and a new call starts.
        assert_eq!(e.note_on(3 * BEAT, 62, 90), Some(Heard::Drop { id: 1 }));
        assert_eq!(e.ready(1, BAR, 3 * BEAT + 5), None, "a late answer for it");
        assert_eq!(e.call().len(), 1);
        assert_eq!(e.call()[0].at, 3 * BEAT);
        e.note_off(4 * BEAT, 62);
        assert_eq!(e.tick(5 * BEAT), Some(Heard::Ask { id: 2, at: 2 * BAR }));
        assert!(e.ready(2, BAR, 5 * BEAT + 5).is_some());
        assert_eq!(e.tick(2 * BAR), None);
        // You play over it: it stops, and your note starts the next call.
        assert_eq!(
            e.note_on(2 * BAR + BEAT, 64, 90),
            Some(Heard::Stop { id: 2 })
        );
        assert_eq!(e.light(2 * BAR + BEAT), Light::You);
        assert_eq!(e.call_from(), 2 * BAR);
        assert_eq!(e.call()[0].at, BEAT);
    }

    #[test]
    fn an_answer_ready_late_waits_for_the_next_bar_line() {
        let mut e = Ears::new(Turns::WhenIStop, BAR, BEAT);
        e.note_on(0, 60, 90);
        e.note_off(BEAT, 60);
        assert_eq!(e.tick(2 * BEAT), Some(Heard::Ask { id: 1, at: BAR }));
        assert_eq!(e.tick(BAR), None);
        assert_eq!(e.light(BAR), Light::Coming);
        assert_eq!(
            e.ready(1, BAR, BAR + 10),
            Some(Heard::Play {
                id: 1,
                at: 2 * BAR,
                len: BAR
            })
        );
        assert_eq!(e.tick(2 * BAR), None);
        assert_eq!(e.light(2 * BAR), Light::Answer);
        assert_eq!(e.tick(3 * BAR), Some(Heard::Done { id: 1 }));
    }

    #[test]
    fn trade_takes_turns_of_its_bars() {
        let mut e = Ears::new(Turns::Trade(2), BAR, BEAT);
        // Your turn: bars 1 and 2, from the first note's bar line; a note
        // in the turn's last 1/16 is heard, not answered.
        let notes = [
            (BEAT, BEAT, 60),
            (BAR, 2 * BEAT, 63),
            (BAR + 3 * BEAT, BEAT - S16, 65),
            (2 * BAR - S16 / 2, S16 / 2, 67),
        ];
        let heard = play(&mut e, &notes, 2 * BAR - 1);
        assert_eq!(heard, [(2 * BAR - S16, Heard::Ask { id: 1, at: 2 * BAR })]);
        assert_eq!(e.call().len(), 3);
        // Ready a few ms later, though shorter than the turn: the answer's
        // turn is the whole two bars.
        assert_eq!(
            e.ready(1, BAR, 2 * BAR - S16 + 20),
            Some(Heard::Play {
                id: 1,
                at: 2 * BAR,
                len: 2 * BAR
            })
        );
        assert_eq!(e.tick(2 * BAR), None);
        assert_eq!(e.light(3 * BAR), Light::Answer);
        assert_eq!(e.tick(4 * BAR), Some(Heard::Done { id: 1 }));
        // Your turn again from bar 5, whether you play on its first beat
        // or later.
        assert_eq!(e.call_from(), 4 * BAR);
        assert_eq!(e.light(4 * BAR), Light::YourTurn);
        e.note_on(4 * BAR + BEAT, 62, 90);
        e.note_off(4 * BAR + 2 * BEAT, 62);
        assert_eq!(e.call()[0].at, BEAT);
        assert_eq!(
            e.tick(6 * BAR - S16),
            Some(Heard::Ask { id: 2, at: 6 * BAR })
        );
    }

    #[test]
    fn a_trade_turn_with_nothing_played_waits_for_you() {
        let mut e = Ears::new(Turns::Trade(1), BAR, BEAT);
        e.note_on(0, 60, 90);
        e.note_off(BEAT, 60);
        assert_eq!(e.tick(BAR - S16), Some(Heard::Ask { id: 1, at: BAR }));
        e.ready(1, BAR, BAR - 10);
        e.tick(BAR);
        assert_eq!(e.tick(2 * BAR), Some(Heard::Done { id: 1 }));
        // Your turn passes with nothing played: no answer, and the next
        // note starts a turn of its own.
        assert_eq!(e.tick(3 * BAR - S16), None);
        assert_eq!(e.light(3 * BAR), Light::YourTurn);
        assert_eq!(e.tick(4 * BAR), None);
        e.note_on(5 * BAR + BEAT, 64, 90);
        assert_eq!(e.call_from(), 5 * BAR);
    }

    #[test]
    fn a_long_call_keeps_its_newest_notes_and_a_reset_forgets_it() {
        let mut e = Ears::new(Turns::WhenIStop, BAR, BEAT);
        for k in 0..300 {
            e.note_on(k * S16, 60 + (k % 12) as u8, 90);
            e.note_off(k * S16 + S16 / 2, 60 + (k % 12) as u8);
        }
        assert_eq!(e.call().len(), MAX_CALL_NOTES);
        assert_eq!(e.call()[0].at, 44 * S16);
        assert_eq!(e.call()[MAX_CALL_NOTES - 1].at, 299 * S16);
        // Still held when pushed out: let go, nothing stuck.
        let mut e = Ears::new(Turns::WhenIStop, BAR, BEAT);
        e.note_on(0, 1, 90);
        for k in 1..=MAX_CALL_NOTES as i64 {
            e.note_on(k, 2 + (k % 100) as u8, 90);
            e.note_off(k + 1, 2 + (k % 100) as u8);
        }
        assert_eq!(e.light(10 * BAR), Light::Quiet);
        assert!(e.tick(10 * BAR).is_some());
        assert_eq!(e.reset(), Some(Heard::Drop { id: 1 }));
        assert!(e.call().is_empty());
        assert_eq!(e.light(10 * BAR), Light::YourTurn);
    }
}
