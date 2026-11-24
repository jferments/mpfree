//! When I stop on the Phrase tab: Answer in the app (composing tools S5,
//! board B; the design is in `docs/dev/design/arpeggiator.md`, "Answer
//! on the Phrase tab").
//!
//! With Plays: When I stop, the armed track's engine answers
//! (`Engine::set_answer`): the pads play as pressed, its ears hear the
//! call, and when the call is over they ask for an answer. The app takes
//! each ask once a period ([`App::answer_poll`]), makes the answer on a
//! composer's thread of its own (`mpfree_compose::answer`, which takes
//! longer than a period), and hands its notes back to the engine by the
//! ask's id; the engine plays them from the bar line the ears give, or the
//! next one if they came late, and stops them when you play again.
//!
//! - **Mode** Echo or Answer is the composer's reply; Trade takes turns
//!   (the ears' `Trade`), each answered as Answer.
//! - **Answers for** Like mine (as long as the call) or 1, 2 or 4 bars;
//!   in Trade, each turn's bars (Like mine: 2).
//! - **Answer now** ends the call at once: the answer plays from the next
//!   beat.
//! - **Capture both** is CAPTURE, which hears the engine's notes: your
//!   call and its answer as one clip.
//!
//! The Q-Links are the reply's Variation, Activity and Range on this
//! panel; Records (contract 8) comes later.

use crate::app::{App, Effect};
use crate::time::compose::Worker;
use mpfree_arp::{AnswerNote, AnswerTurns, CallNote, Light, PhraseKind, PPQ};
use mpfree_arp_ui::view::{AnswerAction, AnswerMode, AnswerNow, HeardNote, PlaysWhen};
use mpfree_compose::answer::{answer, Answered, Call, Reply};
use mpfree_compose::event::Source;
use mpfree_compose::profile::InstrumentKind;
use mpfree_compose::realize::PlainNote;
use mpfree_compose::request::Refused;
use mpfree_compose::rules::RangeSetting;
use mpfree_compose::settings::{Ending, Settings};
use mpfree_compose::GROOVE;
use mpfree_context::PartRole;
use mpfree_time_ui::TimeAction;
use mpfree_transport::time::{Meter, Swing};

/// Trade's turns when Answers for is Like mine, in bars.
pub const TRADE_BARS: u8 = 2;

/// One answer to make: everything the composer reads, copied off the
/// engine's call.
struct AnswerJob {
    call: Vec<(PlainNote, Source)>,
    meter: Meter,
    bpm: f64,
    kind: InstrumentKind,
    role: PartRole,
    settings: Settings,
    reply: Reply,
    bars: Option<u8>,
}

type AnswerDone = Result<Answered, Refused>;

impl AnswerJob {
    fn run(&self) -> AnswerDone {
        answer(&Call {
            notes: &self.call,
            meter: self.meter,
            bpm: self.bpm,
            swing: Swing::default(),
            kind: self.kind,
            role: self.role,
            settings: &self.settings,
            reply: self.reply,
            bars: self.bars,
        })
    }
}

/// Answer's state: how it replies, its composer's thread, and the call
/// and answer last heard, for the screen.
pub(in crate::app) struct Answer {
    pub mode: AnswerMode,
    pub bars: Option<u8>,
    /// The Q-Links on this panel: Steady (0) to Restless (1), Sparse (0)
    /// to Busy (1, 0.5 as busy as the call), and Range.
    pub variation: f32,
    pub activity: f32,
    pub range: RangeSetting,
    worker: Option<Worker<AnswerJob, AnswerDone>>,
    /// The track whose engine has Answer on.
    on: Option<usize>,
    /// The ask being made, and the call it was made from (ticks from its
    /// bar line), for the picture.
    asked: Option<u32>,
    call: Vec<HeardNote>,
    /// The answer last made: its notes, from its bar line, the call's and
    /// its own length, its key and its ending.
    made: Option<Answered>,
    refused: Option<String>,
    light: Option<Light>,
    /// Notes of the call the screen has drawn.
    seen: usize,
    /// Makes an answer a period after it is asked, in the caller's thread:
    /// for tests that count periods.
    #[cfg(test)]
    pub(super) inline: bool,
}

impl Default for Answer {
    fn default() -> Self {
        Self {
            mode: AnswerMode::default(),
            bars: None,
            variation: 0.3,
            activity: 0.5,
            range: RangeSetting::Narrow,
            worker: None,
            on: None,
            asked: None,
            call: Vec::new(),
            made: None,
            refused: None,
            light: None,
            seen: 0,
            #[cfg(test)]
            inline: false,
        }
    }
}

impl Answer {
    /// When the answer comes, as the ears count it.
    pub fn turns(&self) -> AnswerTurns {
        match self.mode {
            AnswerMode::Trade => AnswerTurns::Trade(self.bars.unwrap_or(TRADE_BARS)),
            _ => AnswerTurns::WhenIStop,
        }
    }

    fn reply(&self) -> Reply {
        match self.mode {
            AnswerMode::Echo => Reply::Echo,
            AnswerMode::Answer | AnswerMode::Trade => Reply::Answer,
        }
    }
}

/// The composer's instrument and part for a phrase of `kind`.
fn instrument(kind: PhraseKind) -> (InstrumentKind, PartRole) {
    match kind {
        PhraseKind::Bass => (InstrumentKind::Bass, PartRole::Bass),
        PhraseKind::Chords => (InstrumentKind::Chords, PartRole::Chords),
        PhraseKind::Riff => (InstrumentKind::OneNote, PartRole::Melody),
        PhraseKind::Line => (InstrumentKind::Chordal, PartRole::Melody),
    }
}

/// The meter of a bar of `bar` ticks: quarter notes when they fit, else
/// eighths.
fn meter_of(bar: i64) -> Meter {
    let q = i64::from(PPQ);
    let m = if bar % q == 0 {
        Meter::new(u8::try_from(bar / q).unwrap_or(4), 4)
    } else {
        Meter::new(u8::try_from(bar / (q / 2)).unwrap_or(8), 8)
    };
    m.unwrap_or_default()
}

/// The call as the composer reads it.
fn call_of(notes: &[CallNote]) -> Vec<(PlainNote, Source)> {
    notes
        .iter()
        .map(|n| {
            (
                PlainNote {
                    at: n.at,
                    len: n.len.max(1),
                    pitch: n.pitch,
                    velocity: n.velocity.max(1),
                },
                Source::You,
            )
        })
        .collect()
}

impl App {
    /// A touch on the When I stop panel.
    pub(super) fn answer_action(&mut self, a: AnswerAction) -> Vec<Effect> {
        self.arp_dirty = true;
        match a {
            AnswerAction::Mode(m) => {
                self.answer.mode = m;
                vec![Effect::Note(format!("answer: {}", m.label()))]
            }
            AnswerAction::For(b) => {
                self.answer.bars = b;
                Vec::new()
            }
            AnswerAction::Now => {
                if let Some(t) = self.answer.on {
                    self.arps.engine_at_mut(t).answer_now();
                }
                vec![Effect::Note("answer: now".to_string())]
            }
            AnswerAction::Capture => vec![Effect::Time(TimeAction::Capture)],
        }
    }

    /// Once a period, from [`App::arp_poll`]: Answer on the armed track's
    /// engine while the Phrase tab plays When I stop (off elsewhere), each
    /// ask sent to the composer and each answer made handed back.
    pub(super) fn answer_poll(&mut self) {
        let want = (self.arp_mode == mpfree_arp::Mode::Phrase
            && self.improvise.when == PlaysWhen::WhenIStop)
            .then(|| self.answer.turns());
        if self
            .answer
            .on
            .is_some_and(|t| want.is_none() || t != self.track)
        {
            if let Some(t) = self.answer.on.take() {
                self.arps.engine_at_mut(t).set_answer(None);
            }
            self.arp_dirty = true;
        }
        let Some(turns) = want else {
            self.answer.light = None;
            return;
        };
        let track = self.track;
        if self.arps.engine_at(track).answer_turns() != Some(turns) {
            self.arps.engine_at_mut(track).set_answer(Some(turns));
            self.arp_dirty = true;
        }
        self.answer.on = Some(track);
        if let Some((id, _)) = self.arps.engine_at_mut(track).take_ask() {
            self.answer_ask(id);
        }
        self.answer_done();
        let light = self.arps.engine_at(track).answer_light();
        let seen = self.arps.engine_at(track).answer_call().len();
        if light != self.answer.light || seen != self.answer.seen {
            self.answer.light = light;
            self.answer.seen = seen;
            self.arp_dirty = true;
        }
    }

    /// The ears asked for answer `id`: the call goes to the composer.
    fn answer_ask(&mut self, id: u32) {
        let e = self.arps.engine_at(self.track);
        let notes = e.answer_call();
        let bar = e.bar_ticks();
        self.answer.call = notes
            .iter()
            .map(|n| HeardNote {
                at: n.at,
                len: n.len.max(1),
                pitch: n.pitch,
            })
            .collect();
        let kind = self
            .phrases
            .get(self.phrase_index)
            .map_or(PhraseKind::Riff, |(_, p)| p.kind);
        let (kind, role) = instrument(kind);
        let a = &self.answer;
        let job = AnswerJob {
            call: call_of(notes),
            meter: meter_of(bar),
            bpm: self.settings.bpm,
            kind,
            role,
            settings: Settings {
                style: GROOVE,
                variation: a.variation,
                activity: a.activity,
                range: a.range,
                seed: 0xa5_5e ^ u64::from(id),
                ..Settings::default()
            },
            reply: a.reply(),
            bars: match a.mode {
                // A turn's answer is as long as the turn.
                AnswerMode::Trade => Some(a.bars.unwrap_or(TRADE_BARS)),
                _ => a.bars,
            },
        };
        self.answer.asked = Some(id);
        self.answer.made = None;
        self.answer.refused = None;
        self.arp_dirty = true;
        #[cfg(test)]
        if self.answer.inline {
            let done = job.run();
            self.answer_hand_back(id, done);
            return;
        }
        if self.answer.worker.is_none() {
            match Worker::start("answer", AnswerJob::run) {
                Ok(w) => self.answer.worker = Some(w),
                Err(e) => {
                    self.answer.refused = Some(e);
                    self.arps.engine_at_mut(self.track).answer_refused(id);
                    return;
                }
            }
        }
        if let Some(w) = &self.answer.worker {
            if w.jobs.send((id, Box::new(job))).is_err() {
                self.answer.worker = None;
                self.arps.engine_at_mut(self.track).answer_refused(id);
            }
        }
    }

    /// Answers the composer has made since the last period.
    fn answer_done(&mut self) {
        loop {
            let Some(w) = &self.answer.worker else {
                return;
            };
            let Ok((id, done)) = w.done.try_recv() else {
                return;
            };
            // A newer ask took its place: the ears no longer want it.
            let Some(done) = done else {
                continue;
            };
            self.answer_hand_back(id, *done);
        }
    }

    /// Answer `id` made, or refused: to the engine, and kept for the
    /// screen.
    fn answer_hand_back(&mut self, id: u32, done: AnswerDone) {
        let Some(t) = self.answer.on else {
            return;
        };
        if self.answer.asked != Some(id) {
            return;
        }
        self.arp_dirty = true;
        match done {
            Ok(a) => {
                let notes: Vec<AnswerNote> = a
                    .notes
                    .iter()
                    .map(|n| AnswerNote {
                        at: n.tick,
                        len: n.len,
                        pitch: n.note,
                        velocity: n.velocity,
                    })
                    .collect();
                self.arps
                    .engine_at_mut(t)
                    .answer_ready(id, &notes, a.length);
                self.answer.made = Some(a);
            }
            Err(r) => {
                self.arps.engine_at_mut(t).answer_refused(id);
                self.answer.refused = Some(format!("{r:?}"));
            }
        }
    }

    /// What the When I stop panel shows.
    pub(super) fn answer_view(&self) -> AnswerNow {
        let a = &self.answer;
        let e = self.arps.engine_at(self.track);
        let bar = e.bar_ticks();
        // While you play, the call as it comes; after, the call answered
        // and its answer.
        let live = matches!(e.answer_light(), Some(Light::You | Light::Quiet));
        let made = a.made.as_ref().filter(|_| !live);
        // The answer plays after the call in the picture: the call's whole
        // bars, then the answer's.
        let heard: Vec<HeardNote> = if live {
            e.answer_call()
                .iter()
                .map(|n| HeardNote {
                    at: n.at,
                    len: n.len.max(1),
                    pitch: n.pitch,
                })
                .collect()
        } else {
            a.call.clone()
        };
        let call_bars = made.map_or_else(
            || {
                let end = heard.iter().map(|n| n.at + n.len).max().unwrap_or(0);
                u32::try_from((end + bar - 1) / bar.max(1))
                    .unwrap_or(1)
                    .max(1)
            },
            |m| {
                u32::try_from(m.call_length / bar.max(1))
                    .unwrap_or(1)
                    .max(1)
            },
        );
        let shift = i64::from(call_bars) * bar;
        let answer = made.map_or_else(Vec::new, |m| {
            m.notes
                .iter()
                .map(|n| HeardNote {
                    at: shift + n.tick,
                    len: n.len,
                    pitch: n.note,
                })
                .collect()
        });
        let playhead = e
            .answer_playing()
            .filter(|(id, ..)| a.asked == Some(*id))
            .map(|(_, from, _, _)| shift + (e.clock_now() - from))
            .filter(|t| *t >= shift);
        AnswerNow {
            mode: a.mode,
            bars: a.bars,
            light: a.light,
            call: made.map_or(heard, |m| {
                m.call
                    .iter()
                    .map(|(n, _)| HeardNote {
                        at: n.at,
                        len: n.len,
                        pitch: n.pitch,
                    })
                    .collect()
            }),
            answer,
            bar_ticks: bar,
            call_bars,
            answer_bars: made.map_or(0, |m| u32::try_from(m.length / bar.max(1)).unwrap_or(1)),
            playhead,
            key: made.map(|m| m.key.name()),
            ends: made.map(|m| match m.ends {
                Ending::Home => "home",
                _ => "open",
            }),
            refused: a.refused.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::View;
    use mpfree_arp::scale::pads::{layout, PadLayout, PadSetup};
    use mpfree_arp::{EventKind, Events, Mode};
    use mpfree_arp_ui::view::{ArpAction, ImproviseAction};

    const SR: u32 = 44_100;

    /// The Arp panel on its Phrase tab with When I stop picked, the pads on
    /// a synth (pad 1 is C2).
    fn answering() -> App {
        let mut app = App::new(SR, false);
        app.start();
        app.set_pad_layout(Some(layout(&PadSetup {
            layout: PadLayout::Chromatic,
            ..PadSetup::default()
        })));
        app.toggle_panel();
        assert_eq!(app.view(), View::Arp);
        app.arp_action(ArpAction::Mode(Mode::Phrase));
        app.arp_action(ArpAction::Improvise(ImproviseAction::Plays(
            PlaysWhen::WhenIStop,
        )));
        app.answer.inline = true;
        app.arp_poll();
        app
    }

    /// Runs `periods` periods of 128 frames, polling after each; returns
    /// the notes that started.
    fn run(app: &mut App, periods: usize) -> Vec<u8> {
        let mut ev = Events::new();
        let mut on = Vec::new();
        for _ in 0..periods {
            app.process(128, &mut ev);
            for e in ev.as_slice() {
                if let EventKind::On { note, .. } = e.kind {
                    on.push(note);
                }
            }
            app.arp_poll();
        }
        on
    }

    /// Periods of 128 frames in `beats` beats at the app's tempo.
    fn periods(app: &App, beats: f64) -> usize {
        (beats * 60.0 / app.settings.bpm * f64::from(SR) / 128.0).ceil() as usize
    }

    #[test]
    fn when_i_stop_answers_the_call_on_the_next_bar_line() {
        let mut app = answering();
        assert!(app.arps.engine_at(app.track).answer_turns().is_some());
        // Pads 1, 3, 5, 8: C D E G, a quarter note each.
        let mut heard = Vec::new();
        for pad in [0, 2, 4, 7] {
            app.pad_down(pad, 100);
            let b = periods(&app, 0.9);
            heard.extend(run(&mut app, b));
            app.pad_up(pad);
            let b = periods(&app, 0.1);
            heard.extend(run(&mut app, b));
        }
        assert_eq!(heard.len(), 4, "the pads play as pressed: {heard:?}");
        let v = app.answer_view();
        assert_eq!(v.call.len(), 4);
        // A beat of quiet ends the call; the answer is made and plays
        // through bar 2.
        let b = periods(&app, 5.0);
        let answered = run(&mut app, b);
        assert!(!answered.is_empty(), "it answered");
        let v = app.answer_view();
        assert!(!v.answer.is_empty());
        assert_eq!(v.call_bars, 1);
        assert_eq!(v.answer_bars, 1, "Like mine: as long as the call");
        assert!(v.key.is_some() && v.ends.is_some());
        // Back to While I hold: Answer is off and the phrase plays again.
        app.arp_action(ArpAction::Improvise(ImproviseAction::Plays(
            PlaysWhen::WhileIHold,
        )));
        app.arp_poll();
        assert!(app.arps.engine_at(app.track).answer_turns().is_none());
    }

    #[test]
    fn mode_and_answers_for_set_the_turns_and_capture_is_captures() {
        let mut app = answering();
        app.arp_action(ArpAction::Answer(AnswerAction::Mode(AnswerMode::Trade)));
        app.arp_action(ArpAction::Answer(AnswerAction::For(Some(4))));
        app.arp_poll();
        assert_eq!(
            app.arps.engine_at(app.track).answer_turns(),
            Some(AnswerTurns::Trade(4))
        );
        assert_eq!(app.answer_view().mode, AnswerMode::Trade);
        let fx = app.arp_action(ArpAction::Answer(AnswerAction::Capture));
        assert!(fx
            .iter()
            .any(|f| matches!(f, Effect::Time(TimeAction::Capture))));
    }

    #[test]
    fn answer_now_plays_from_the_next_beat() {
        let mut app = answering();
        app.pad_down(0, 100);
        let b = periods(&app, 0.5);
        run(&mut app, b);
        app.pad_up(0);
        app.arp_action(ArpAction::Answer(AnswerAction::Now));
        let b = periods(&app, 3.0);
        let answered = run(&mut app, b);
        assert!(!answered.is_empty(), "it answered within the bar");
    }

    #[test]
    fn the_worker_makes_the_answer_off_the_audio_thread() {
        let mut app = answering();
        app.answer.inline = false;
        app.pad_down(0, 100);
        let b = periods(&app, 1.0);
        run(&mut app, b);
        app.pad_up(0);
        // Wait out the call's beat of quiet and the composer's thread.
        let mut answered = Vec::new();
        for _ in 0..400 {
            answered.extend(run(&mut app, 8));
            if !answered.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!answered.is_empty(), "the worker's answer played");
    }
}
