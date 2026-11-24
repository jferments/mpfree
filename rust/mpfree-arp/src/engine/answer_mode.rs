//! Answer in the engine (composing tools S5, the Phrase tab's When I
//! stop): the pads play as pressed, the ears ([`crate::ears`]) hear the
//! call, and the answer, made elsewhere and handed back, plays on its line.
//!
//! The engine does the parts that need the audio thread's clock: it tells
//! the ears of every note you play and of time passing, keeps the answer's
//! notes once the host has made them ([`Engine::answer_ready`]), plays
//! them from the line the ears give, and ends them at once when you play
//! over them. Making the answer (`mpfree_compose::answer`) takes longer
//! than a period, so the host does it on a worker: it takes each ask with
//! [`Engine::take_ask`], reads the call with [`Engine::answer_call`] and
//! hands the notes back by the ask's id.
//!
//! **Who ends what (contract 9).** While answering the engine plays two
//! hands: yours, as pressed (as Direct does), and the answer's. They never
//! play at once: a note of yours while the answer plays stops it
//! (`Heard::Stop`), and its notes end on the frame your note starts, before
//! it, so the same pitch struck by you is never cut short by the answer's
//! ending. The answer's notes are its own (`owned`): ending them leaves a
//! note of yours alone.
//!
//! Times are the engine's clock ticks (the transport's while it plays, the
//! engine's own while it doesn't), which only go back on a transport
//! start, stop, locate or loop wrap: each is an [`Ears::reset`].

use super::{Engine, EventKind, PPQ};
use crate::ears::{CallNote, Ears, Heard, Light, Turns};

/// Most notes an answer plays: 4 bars of 1/32s.
pub const MAX_ANSWER_NOTES: usize = 128;

/// A note of an answer, in ticks from its first bar line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnswerNote {
    pub at: i64,
    pub len: i64,
    pub pitch: u8,
    pub velocity: u8,
}

/// The answer's side of the engine: fixed size, made once when Answer is
/// turned on.
#[derive(Clone, Debug)]
pub(super) struct Answering {
    ears: Ears,
    /// The newest ask the host has not taken: (id, the tick it plays from).
    ask: Option<(u32, i64)>,
    /// The answer made for ask `id`, its notes in order of start.
    id: u32,
    notes: [AnswerNote; MAX_ANSWER_NOTES],
    count: usize,
    /// Notes already queued.
    next: usize,
    /// Where it plays from and for how long, once the ears say so.
    from: Option<i64>,
    len: i64,
    /// The pitches the answer has queued and not yet ended.
    owned: u128,
}

impl Answering {
    fn new(turns: Turns, bar: i64) -> Self {
        Self {
            ears: Ears::new(turns, bar, i64::from(PPQ)),
            ask: None,
            id: 0,
            notes: [AnswerNote::default(); MAX_ANSWER_NOTES],
            count: 0,
            next: 0,
            from: None,
            len: 0,
            owned: 0,
        }
    }

    fn clear(&mut self) {
        self.count = 0;
        self.next = 0;
        self.from = None;
        self.len = 0;
    }
}

impl Engine {
    /// Answer on (`Some`: when the answer comes) or off. On, the pads play
    /// as pressed and nothing else generates; the first time it is turned
    /// on it makes its fixed-size state (the only allocation). Off, the
    /// answer ends at once and the notes held stop sounding as pressed:
    /// the mode set plays them from here.
    pub fn set_answer(&mut self, turns: Option<Turns>) {
        match (turns, self.answer.as_mut()) {
            (Some(t), Some(a)) => a.ears.set_turns(t),
            (Some(t), None) => {
                // What the mode was playing stops; your notes play as
                // pressed from the next press.
                if self.running {
                    self.drop_pending_ons();
                    if self.settings.mode == super::Mode::Phrase {
                        self.stop_phrase();
                    }
                    self.running = false;
                }
                self.answer = Some(Box::new(Answering::new(t, self.bar_ticks())));
            }
            (None, Some(_)) => {
                let now = self.now as f64;
                self.end_answer(now);
                self.answer = None;
                if self.settings.mode != super::Mode::Direct {
                    let mut list = [crate::notes::Held::default(); crate::notes::MAX_NOTES];
                    let n = self.notes.active().sorted(&mut list);
                    for h in &list[..n] {
                        self.queue_at(now, EventKind::Off { note: h.note });
                    }
                }
                self.update_running();
            }
            (None, None) => {}
        }
    }

    /// When the answer comes, while Answer is on.
    pub fn answer_turns(&self) -> Option<Turns> {
        self.answer.as_ref().map(|a| a.ears.turns())
    }

    /// The pads play as pressed: Direct, or Answer on.
    pub(super) fn as_pressed(&self) -> bool {
        self.settings.mode == super::Mode::Direct || self.answer.is_some()
    }

    /// The newest answer asked for and not yet taken: its id, and the tick
    /// it is to play from. Read the call with [`Engine::answer_call`]
    /// before the next press.
    pub fn take_ask(&mut self) -> Option<(u32, i64)> {
        self.answer.as_mut().and_then(|a| a.ask.take())
    }

    /// The call as the ears keep it, in ticks from its bar line.
    pub fn answer_call(&self) -> &[CallNote] {
        self.answer.as_ref().map_or(&[], |a| a.ears.call())
    }

    /// The call's bar line, in clock ticks.
    pub fn answer_call_from(&self) -> i64 {
        self.answer.as_ref().map_or(0, |a| a.ears.call_from())
    }

    /// Answer `id` is made: its notes, from its first bar line, and how
    /// long it plays. Kept when the ears still want it; it plays from its
    /// line, or the next bar line if it came late.
    pub fn answer_ready(&mut self, id: u32, notes: &[AnswerNote], len: i64) {
        let now = self.clock_now();
        let Some(a) = self.answer.as_mut() else {
            return;
        };
        let Some(h) = a.ears.ready(id, len, now) else {
            return;
        };
        a.clear();
        a.id = id;
        let n = notes.len().min(MAX_ANSWER_NOTES);
        a.notes[..n].copy_from_slice(&notes[..n]);
        a.notes[..n].sort_by_key(|n| (n.at, n.pitch));
        a.count = n;
        self.answer_heard(Some(h), self.now as f64);
    }

    /// Answer `id` could not be made: your turn again.
    pub fn answer_refused(&mut self, id: u32) {
        if let Some(a) = self.answer.as_mut() {
            a.ears.refused(id);
        }
    }

    /// Answer now: the call is over, the answer plays from the next beat.
    pub fn answer_now(&mut self) {
        let now = self.clock_now();
        if let Some(a) = self.answer.as_mut() {
            let h = a.ears.answer_now(now);
            self.answer_heard(h, self.now as f64);
        }
    }

    /// The turn light, while Answer is on.
    pub fn answer_light(&self) -> Option<Light> {
        let now = self.clock_now();
        self.answer.as_ref().map(|a| a.ears.light(now))
    }

    /// The answer playing or waiting for its line: its id, the tick it
    /// plays from and how long, and its notes.
    pub fn answer_playing(&self) -> Option<(u32, i64, i64, &[AnswerNote])> {
        let a = self.answer.as_ref()?;
        let from = a.from?;
        Some((a.id, from, a.len, &a.notes[..a.count]))
    }

    /// The clock tick now (the start of the coming period), whole.
    pub fn clock_now(&self) -> i64 {
        self.clock_tick(self.now as f64).floor() as i64
    }

    /// Something is coming or playing: the engine must not be skipped.
    pub(super) fn answer_busy(&self) -> bool {
        self.answer
            .as_ref()
            .is_some_and(|a| a.owned != 0 || a.from.is_some() || a.ears.light(0) == Light::Coming)
    }

    /// Ticks in a bar: the transport's, or 4/4 while it is stopped. The
    /// call's ticks count from its bar line in these bars.
    pub fn bar_ticks(&self) -> i64 {
        i64::from(self.sync.map_or(4 * PPQ, |s| s.bar_ticks))
    }

    /// Your notes changed at `frame`: the ears hear them. `added` and
    /// `removed` are the notes that started and ended.
    pub(super) fn answer_hears(&mut self, frame: f64, added: &[(u8, u8)], removed: &[u8]) {
        let now = self.clock_tick(frame).floor() as i64;
        let Some(a) = self.answer.as_mut() else {
            return;
        };
        for &p in removed {
            a.ears.note_off(now, p);
        }
        let mut heard = None;
        for &(p, v) in added {
            heard = heard.or(a.ears.note_on(now, p, v));
        }
        self.answer_heard(heard, frame);
    }

    /// A transport start, stop, locate or wrap, or a new meter: the ears
    /// forget, and what was playing or coming ends now.
    pub(super) fn answer_reset(&mut self) {
        let bar = self.bar_ticks();
        let Some(a) = self.answer.as_mut() else {
            return;
        };
        let h = a.ears.reset();
        if a.ears.bar() != bar {
            let turns = a.ears.turns();
            a.ears = Ears::new(turns, bar, i64::from(PPQ));
        }
        self.answer_heard(h, self.now as f64);
    }

    /// STOP or a panic: everything sounding has ended already; the ears
    /// forget and nothing more of the answer plays.
    pub(super) fn answer_hush(&mut self) {
        if let Some(a) = self.answer.as_mut() {
            a.ears.reset();
            a.ask = None;
            a.clear();
            a.owned = 0;
        }
    }

    /// Once a period, before its notes are drained: the ears hear time
    /// pass, and the answer's notes that start before `end` are queued.
    pub(super) fn answer_period(&mut self, end: u64) {
        let now = self.clock_now();
        let Some(a) = self.answer.as_mut() else {
            return;
        };
        let h = a.ears.tick(now);
        self.answer_heard(h, self.now as f64);
        let until = self.clock_tick(end as f64);
        loop {
            let Some(a) = self.answer.as_mut() else {
                return;
            };
            let Some(from) = a.from else {
                return;
            };
            if a.next >= a.count {
                return;
            }
            let n = a.notes[a.next];
            let t = from + n.at;
            if t as f64 >= until {
                return;
            }
            a.next += 1;
            if n.at >= a.len {
                continue;
            }
            let stop = from + a.len;
            a.owned |= 1 << (n.pitch & 127);
            let at = self.clock_frame(t as f64).max(self.now as f64);
            let off = self.clock_frame((t + n.len.max(1)).min(stop) as f64);
            self.end_note_by_answer(n.pitch, at);
            if self.queued + 2 <= super::MAX_PENDING {
                self.queue_at(
                    at,
                    EventKind::On {
                        note: n.pitch,
                        velocity: n.velocity.max(1),
                    },
                );
                self.queue_at(off.max(at + 1.0), EventKind::Off { note: n.pitch });
            } else {
                self.overflow += 2;
            }
        }
    }

    /// The drain sent `note`'s ending: the answer no longer owns it.
    pub(super) fn answer_ended(&mut self, note: u8) {
        if let Some(a) = self.answer.as_mut() {
            a.owned &= !(1 << (note & 127));
        }
    }

    /// What the ears said, at `frame`.
    fn answer_heard(&mut self, heard: Option<Heard>, frame: f64) {
        let Some(h) = heard else {
            return;
        };
        let Some(a) = self.answer.as_mut() else {
            return;
        };
        match h {
            Heard::Ask { id, at } => {
                a.ask = Some((id, at));
                a.clear();
            }
            Heard::Play { id, at, len } if id == a.id => {
                a.from = Some(at);
                a.len = len;
                a.next = 0;
            }
            Heard::Play { .. } => {}
            Heard::Stop { .. } => {
                a.ask = None;
                self.end_answer(frame);
            }
            Heard::Drop { .. } => {
                a.ask = None;
                a.clear();
            }
            Heard::Done { .. } => a.clear(),
        }
    }

    /// The answer stops at `frame`: nothing of it starts there or later,
    /// and what it has sounding ends there.
    fn end_answer(&mut self, frame: f64) {
        let Some(a) = self.answer.as_mut() else {
            return;
        };
        let owned = a.owned;
        a.clear();
        let mut i = 0;
        while i < self.queued {
            let p = &mut self.queue[i];
            match p.kind {
                EventKind::On { note, .. } if owned & (1 << (note & 127)) != 0 && p.at >= frame => {
                    self.queued -= 1;
                    self.queue[i] = self.queue[self.queued];
                    continue;
                }
                EventKind::Off { note } if owned & (1 << (note & 127)) != 0 && p.at > frame => {
                    p.at = frame;
                }
                _ => {}
            }
            i += 1;
        }
    }

    /// The answer strikes `note` again at `frame`: its note sounding ends
    /// there, so its ending can't cut the new one short.
    fn end_note_by_answer(&mut self, note: u8, frame: f64) {
        for p in &mut self.queue[..self.queued] {
            if p.kind == (EventKind::Off { note }) && p.at > frame {
                p.at = frame;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{run, SR};
    use super::*;
    use crate::engine::{Clock, Events, Mode, Settings};

    const BEAT: i64 = PPQ as i64;
    const BAR: i64 = 4 * BEAT;
    /// Frames in a tick at 120 bpm and 48 kHz.
    const FPT: f64 = 48_000.0 * 60.0 / (120.0 * PPQ as f64);
    const BLOCK: u32 = 128;

    fn answering() -> Engine {
        let mut e = Engine::new(SR, 120.0);
        e.set_settings(Settings {
            mode: Mode::Phrase,
            ..Settings::default()
        });
        e.set_answer(Some(Turns::WhenIStop));
        e
    }

    fn frame(tick: i64) -> u64 {
        (tick as f64 * FPT).round() as u64
    }

    /// Runs blocks until frame `to`, collecting (frame, event).
    fn until(e: &mut Engine, from: &mut u64, to: u64, log: &mut Vec<(u64, EventKind)>) {
        let mut out = Events::new();
        while *from < to {
            e.process(BLOCK, &mut out);
            for ev in out.as_slice() {
                log.push((*from + u64::from(ev.offset), ev.kind));
            }
            *from += u64::from(BLOCK);
        }
    }

    fn ons(log: &[(u64, EventKind)]) -> Vec<(u64, u8)> {
        log.iter()
            .filter_map(|(f, k)| match k {
                EventKind::On { note, .. } => Some((*f, *note)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn your_notes_play_as_pressed_and_the_answer_plays_on_its_bar_line() {
        let mut e = answering();
        let (mut t, mut log) = (0u64, Vec::new());
        // A call of two quarter notes in bar 1.
        e.press(60, 100);
        until(&mut e, &mut t, frame(BEAT), &mut log);
        e.release(60);
        e.press(62, 100);
        let second = t;
        until(&mut e, &mut t, frame(2 * BEAT), &mut log);
        e.release(62);
        assert_eq!(ons(&log), vec![(0, 60), (second, 62)], "as pressed");
        // A beat of quiet: asked, to play from bar 2.
        until(&mut e, &mut t, frame(3 * BEAT) + 256, &mut log);
        let (id, at) = e.take_ask().expect("asked");
        assert_eq!(at, BAR);
        let call = e.answer_call();
        assert_eq!(call.len(), 2);
        assert_eq!((call[0].pitch, call[1].pitch), (60, 62));
        assert!(matches!(e.answer_light(), Some(Light::Coming)));
        let answer = [
            AnswerNote {
                at: 0,
                len: BEAT,
                pitch: 64,
                velocity: 90,
            },
            AnswerNote {
                at: 2 * BEAT,
                len: 4 * BEAT,
                pitch: 67,
                velocity: 90,
            },
        ];
        e.answer_ready(id, &answer, BAR);
        let mut log = Vec::new();
        until(&mut e, &mut t, frame(2 * BAR) + 1024, &mut log);
        assert_eq!(
            ons(&log),
            vec![(frame(BAR), 64), (frame(BAR + 2 * BEAT), 67)]
        );
        // The long note is cut at the answer's end.
        let off67 = log
            .iter()
            .find(|(_, k)| *k == EventKind::Off { note: 67 })
            .map(|(f, _)| *f);
        assert_eq!(off67, Some(frame(2 * BAR)));
        assert!(matches!(e.answer_light(), Some(Light::YourTurn)));
        assert_eq!(e.sounding(), 0);
    }

    #[test]
    fn playing_over_the_answer_ends_its_notes_before_yours() {
        let mut e = answering();
        let (mut t, mut log) = (0u64, Vec::new());
        e.press(60, 100);
        until(&mut e, &mut t, frame(BEAT), &mut log);
        e.release(60);
        until(&mut e, &mut t, frame(2 * BEAT) + 256, &mut log);
        let (id, _) = e.take_ask().expect("asked");
        e.answer_ready(
            id,
            &[
                AnswerNote {
                    at: 0,
                    len: 2 * BEAT,
                    pitch: 64,
                    velocity: 90,
                },
                AnswerNote {
                    at: 2 * BEAT,
                    len: BEAT,
                    pitch: 65,
                    velocity: 90,
                },
            ],
            BAR,
        );
        until(&mut e, &mut t, frame(BAR + BEAT), &mut log);
        assert!(matches!(e.answer_light(), Some(Light::Answer)));
        // You play the answer's own pitch over it.
        let mut log = Vec::new();
        e.press(64, 100);
        let pressed = t;
        until(&mut e, &mut t, frame(BAR + 3 * BEAT), &mut log);
        assert_eq!(
            &log[..2],
            &[
                (pressed, EventKind::Off { note: 64 }),
                (
                    pressed,
                    EventKind::On {
                        note: 64,
                        velocity: 100
                    }
                )
            ],
            "its note ends first, on the frame yours starts"
        );
        assert!(
            ons(&log).iter().all(|&(_, n)| n != 65),
            "nothing more of it"
        );
        assert!(matches!(e.answer_light(), Some(Light::You)));
        // Your note still sounds.
        assert_eq!(e.sounding(), 1 << 64);
    }

    #[test]
    fn a_transport_jump_drops_the_answer_coming_and_off_plays_the_mode_again() {
        let mut e = answering();
        let (mut t, mut log) = (0u64, Vec::new());
        e.sync(&Clock::playing(0, 0, 120.0));
        e.press(60, 100);
        until(&mut e, &mut t, frame(BEAT), &mut log);
        e.release(60);
        until(&mut e, &mut t, frame(2 * BEAT) + 256, &mut log);
        assert!(e.take_ask().is_some());
        let mut jump = Clock::playing(0, 0, 120.0);
        jump.jumped = true;
        e.sync(&jump);
        assert!(matches!(e.answer_light(), Some(Light::YourTurn)));
        assert!(!e.answer_busy());
        // Off: a held note plays the phrase again.
        e.press(60, 100);
        e.set_answer(None);
        assert!(e.answer_turns().is_none());
        assert!(e.running, "the phrase plays from what is held");
    }

    #[test]
    fn an_answer_for_a_call_played_over_is_never_played() {
        let mut e = answering();
        let (mut t, mut log) = (0u64, Vec::new());
        e.press(60, 100);
        until(&mut e, &mut t, frame(BEAT), &mut log);
        e.release(60);
        until(&mut e, &mut t, frame(2 * BEAT) + 256, &mut log);
        let (id, _) = e.take_ask().expect("asked");
        // You play again before it is ready: dropped.
        e.press(62, 100);
        e.answer_ready(
            id,
            &[AnswerNote {
                at: 0,
                len: BEAT,
                pitch: 70,
                velocity: 90,
            }],
            BAR,
        );
        let mut log = Vec::new();
        e.release(62);
        until(&mut e, &mut t, frame(BAR + BEAT), &mut log);
        assert!(ons(&log).iter().all(|&(_, n)| n != 70));
    }

    #[test]
    fn stop_hushes_the_answer_and_an_engine_waiting_on_one_is_not_idle() {
        let mut e = answering();
        let (mut t, mut log) = (0u64, Vec::new());
        e.press(60, 100);
        until(&mut e, &mut t, frame(BEAT), &mut log);
        e.release(60);
        until(&mut e, &mut t, frame(2 * BEAT) + 256, &mut log);
        let (id, _) = e.take_ask().expect("asked");
        assert!(!e.idle(), "an answer is coming");
        e.answer_ready(
            id,
            &[AnswerNote {
                at: 0,
                len: BAR,
                pitch: 64,
                velocity: 90,
            }],
            BAR,
        );
        until(&mut e, &mut t, frame(BAR + BEAT), &mut log);
        assert_eq!(e.sounding(), 1 << 64);
        e.hush();
        let mut log = Vec::new();
        until(&mut e, &mut t, frame(2 * BAR), &mut log);
        assert_eq!(e.sounding(), 0);
        assert!(ons(&log).is_empty());
        assert!(e.idle());
        let _ = run;
    }
}
