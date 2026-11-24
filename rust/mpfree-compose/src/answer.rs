//! Answer: call and response (revision 3, S5). You play a phrase and stop;
//! mpfree answers it.
//!
//! The design's two replies, both made by Continue on the call, so the
//! answer is planned, played and judged as any new bars are:
//!
//! - **Echo** plays your phrase back a little changed: your rhythm and the
//!   shape of your tune kept, Similarity near Like mine, and the ending
//!   yours (home if you landed home, open if you left it open).
//! - **Answer** replies with your rhythm, its own notes and a different
//!   ending: your rhythm kept, Similarity toward New, and the ending turned
//!   round (open if you landed home, home if you left it open), the way a
//!   call that asks is answered by one that settles.
//!
//! Trade (turns every 1, 2 or 4 bars) is when, not what: the live side says
//! when a turn ends, and each turn is answered as Answer.
//!
//! The answer is as long as the call, in bars: a call of 1 bar gets 1, of 2
//! gets 2, of 3 or 4 gets 4, and a longer call is answered from its last 4
//! bars. This runs as a background job, as Continue does: it reads the
//! call, makes the drafts and lets the critic pick, all before the next bar
//! line after a beat of quiet.

use mpfree_context::chord::ChordTimeline;
use mpfree_context::key::Key;
use mpfree_context::listen::{listen, Played};
use mpfree_context::{PartRole, ReadBudget};
use mpfree_seq::Note;
use mpfree_transport::time::{Meter, Swing, Tick};

use crate::event::Source;
use crate::motif::line;
use crate::profile::InstrumentKind;
use crate::realize::PlainNote;
use crate::request::{continue_notes, phrasing, Continue, Refused};
use crate::settings::{Ending, Keeps, Settings};

/// Longest answer, in bars.
pub const MAX_BARS: i64 = 4;

/// How mpfree replies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Reply {
    /// Your phrase back, a little changed.
    Echo,
    /// Your rhythm, its own notes, a different ending.
    #[default]
    Answer,
}

impl Reply {
    pub const ALL: [Reply; 2] = [Reply::Echo, Reply::Answer];

    pub const fn words(self) -> &'static str {
        match self {
            Reply::Echo => "Echo",
            Reply::Answer => "Answer",
        }
    }

    /// What it does, in a line.
    pub const fn says(self) -> &'static str {
        match self {
            Reply::Echo => "Your phrase back, a little changed",
            Reply::Answer => "Your rhythm, its own notes, a different ending",
        }
    }
}

/// A phrase you played, to be answered.
#[derive(Clone, Copy)]
pub struct Call<'a> {
    /// What you played, in ticks from the bar line the call started after
    /// (the live side counts from there).
    pub notes: &'a [(PlainNote, Source)],
    pub meter: Meter,
    pub bpm: f64,
    pub swing: Swing,
    /// The track's instrument.
    pub kind: InstrumentKind,
    pub role: PartRole,
    /// Style, Character, Activity, Range and the seed, as Continue takes
    /// them. Bars, Ending, Similarity and what is kept are the reply's.
    pub settings: &'a Settings,
    pub reply: Reply,
    /// Answers for: None is Like mine (as long as the call, in whole bars:
    /// 1, 2 or 4); Some(1, 2 or 4) is that many bars.
    pub bars: Option<u8>,
}

/// An answer, ready to play.
#[derive(Clone, Debug, PartialEq)]
pub struct Answered {
    /// The answer's notes, in ticks from its first bar line.
    pub notes: Vec<Note>,
    /// How long it plays: whole bars.
    pub length: Tick,
    /// How long the call was taken to be: whole bars, from its first bar line.
    pub call_length: Tick,
    /// The call as it was answered: from its first bar line, a long one's
    /// last 4 bars only.
    pub call: Vec<(PlainNote, Source)>,
    /// The key the call was read in.
    pub key: Key,
    /// How the call ended, and so how the answer ends.
    pub call_ends: Ending,
    pub ends: Ending,
    /// What the critic changed in the draft it picked (S4), in words.
    pub critic: Vec<String>,
}

/// Answer `call`, or refuse when nothing was played.
pub fn answer(call: &Call) -> Result<Answered, Refused> {
    let bar = call.meter.bar_ticks().max(1);
    let mut notes: Vec<(PlainNote, Source)> = call
        .notes
        .iter()
        .copied()
        .filter(|(n, _)| n.len > 0 && n.at >= 0)
        .collect();
    if notes.is_empty() {
        return Err(Refused::NoClip);
    }
    notes.sort_by_key(|(n, _)| (n.at, n.pitch));
    // From the call's first bar line; a long call's last 4 bars.
    let first_bar = notes[0].0.at / bar;
    let end = notes.iter().map(|(n, _)| n.at + n.len).max().unwrap_or(0);
    let last_bar = ((end + bar - 1) / bar).max(first_bar + 1);
    let from_bar = first_bar.max(last_bar - MAX_BARS);
    let shift = from_bar * bar;
    notes.retain(|(n, _)| n.at >= shift);
    for (n, _) in &mut notes {
        n.at -= shift;
    }
    let call_bars = match last_bar - from_bar {
        1 => 1,
        2 => 2,
        _ => 4,
    };
    let length = call_bars * bar;
    // Answers for: Like mine, or 1, 2 or 4 bars.
    let bars = match call.bars {
        Some(b @ (1 | 2 | 4)) => i64::from(b),
        Some(_) => 4,
        None => call_bars,
    };
    for (n, _) in &mut notes {
        n.len = n.len.min(length - n.at).max(1);
    }

    let played: Vec<Played> = notes
        .iter()
        .map(|(n, _)| Played::new(n.at, n.len, n.pitch, n.velocity))
        .collect();
    let heard = listen(&played, length, ReadBudget::default(), call.meter);
    let context = mpfree_context::MusicalContext {
        key: heard.key,
        chords: heard.chords,
        groove: None,
        played_pcs: heard.played_pcs,
        motifs: Vec::new(),
        read: Vec::new(),
        cut_short: heard.cut_short,
    };

    let plain: Vec<PlainNote> = notes.iter().map(|(n, _)| *n).collect();
    let mut req = Continue {
        notes: &notes,
        length,
        context: &context,
        settings: call.settings,
        role: call.role,
        kind: call.kind,
        meter: call.meter,
        bpm: call.bpm,
        swing: call.swing,
        loops: false,
        row_length: 0,
    };
    // The key Continue will read the call in.
    let empty = ChordTimeline::default();
    let key = phrasing(&req, &empty).key;
    let call_ends = match line(&plain, key).last() {
        Some(g) if g.alt == 0 && g.pos.rem_euclid(7) == 0 => Ending::Home,
        _ => Ending::Open,
    };
    let (similarity, keeps, ends) = match call.reply {
        Reply::Echo => (
            0.9,
            Keeps {
                rhythm: true,
                tune_shape: true,
                ..Keeps::default()
            },
            call_ends,
        ),
        Reply::Answer => (
            0.35,
            Keeps {
                rhythm: true,
                ..Keeps::default()
            },
            match call_ends {
                Ending::Home => Ending::Open,
                _ => Ending::Home,
            },
        ),
    };
    let settings = Settings {
        bars: bars as u8,
        ending: ends,
        similarity,
        keeps,
        ..*call.settings
    };
    req.settings = &settings;
    let made = continue_notes(&req)?;
    let answer_length = bars * bar;
    let answer = made
        .notes
        .iter()
        .filter(|n| n.tick >= length && n.tick < length + answer_length)
        .map(|n| Note {
            tick: n.tick - length,
            len: n.len.min(length + answer_length - n.tick).max(1),
            ..*n
        })
        .collect();
    Ok(Answered {
        notes: answer,
        length: answer_length,
        call_length: length,
        call: notes,
        key,
        call_ends,
        ends,
        critic: made.critic,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motif::{bar_of, similarity, GridNote};
    use crate::style::{GROOVE, JAZZISH, MINIMAL};
    use mpfree_context::groove::STEP;
    use mpfree_transport::time::PPQ;

    const BAR: Tick = 4 * PPQ as Tick;

    fn clip(notes: &[(i64, i64, u8)]) -> Vec<(PlainNote, Source)> {
        notes
            .iter()
            .map(|&(on, len, pitch)| {
                (
                    PlainNote {
                        at: on * STEP,
                        len: len * STEP,
                        pitch,
                        velocity: 90,
                    },
                    Source::You,
                )
            })
            .collect()
    }

    /// Two bars that ask: C minor, climbing and left on the 5th.
    fn asks() -> Vec<(PlainNote, Source)> {
        clip(&[
            (0, 2, 60),
            (2, 1, 63),
            (3, 1, 65),
            (4, 2, 67),
            (6, 2, 65),
            (8, 3, 67),
            (11, 1, 68),
            (12, 4, 67),
            (16, 2, 63),
            (18, 2, 65),
            (20, 2, 67),
            (22, 2, 70),
            (24, 4, 68),
            (28, 4, 67),
        ])
    }

    /// One bar that settles: down to the home note.
    fn settles() -> Vec<(PlainNote, Source)> {
        clip(&[(0, 2, 67), (2, 2, 65), (4, 2, 63), (6, 2, 62), (8, 8, 60)])
    }

    fn call<'a>(notes: &'a [(PlainNote, Source)], s: &'a Settings, reply: Reply) -> Call<'a> {
        Call {
            notes,
            meter: Meter::default(),
            bpm: 120.0,
            swing: Swing::default(),
            kind: InstrumentKind::OneNote,
            role: PartRole::Melody,
            settings: s,
            reply,
            bars: None,
        }
    }

    fn grid(a: &Answered) -> (Vec<GridNote>, Vec<GridNote>, Key) {
        let plain: Vec<PlainNote> = a.call.iter().map(|(n, _)| *n).collect();
        let key = a.key;
        let ans: Vec<PlainNote> = a
            .notes
            .iter()
            .map(|n| PlainNote {
                at: n.tick,
                len: n.len,
                pitch: n.note,
                velocity: n.velocity,
            })
            .collect();
        (line(&plain, key), line(&ans, key), key)
    }

    /// Onsets in steps, rounded (the playing moves notes by a few ms).
    fn onsets(a: &[GridNote]) -> Vec<i32> {
        let mut v: Vec<i32> = a.iter().map(|g| g.on).collect();
        v.dedup();
        v
    }

    #[test]
    fn echo_is_your_phrase_a_little_changed_and_answer_your_rhythm_with_its_own_notes() {
        let mut echo_like = 0.0;
        let mut answer_like = 0.0;
        let mut n = 0.0;
        for (name, notes) in [("asks", asks()), ("settles", settles())] {
            for style in [MINIMAL, GROOVE, JAZZISH] {
                for seed in 1..=4 {
                    let s = Settings {
                        style,
                        seed,
                        ..Settings::default()
                    };
                    let what = format!("{name} {} seed {seed}", style.name);
                    let echo = answer(&call(&notes, &s, Reply::Echo)).unwrap();
                    let ans = answer(&call(&notes, &s, Reply::Answer)).unwrap();
                    assert_eq!(echo.length, ans.length, "{what}");
                    assert_eq!(echo.length, if name == "asks" { 2 } else { 1 } * BAR);
                    let (yours, e, key) = grid(&echo);
                    let (_, a, _) = grid(&ans);
                    let gb = (BAR / STEP) as i32;
                    let bars = (echo.length / BAR) as i32;
                    let like = |x: &[GridNote]| {
                        (0..bars)
                            .map(|k| {
                                similarity(key, &bar_of(x, k, gb), &bar_of(&yours, k, gb)).total
                            })
                            .sum::<f64>()
                            / f64::from(bars)
                    };
                    // Echo: not your phrase exactly.
                    let same = e.len() == yours.len()
                        && e.iter()
                            .zip(&yours)
                            .all(|(x, y)| x.on == y.on && x.pos == y.pos);
                    assert!(!same, "{what}: the echo is the call itself");
                    // Answer: your rhythm, note for note.
                    assert_eq!(onsets(&a), onsets(&yours), "{what}: the answer's rhythm");
                    // A different ending: the call's turned round.
                    assert_ne!(ans.ends, ans.call_ends, "{what}");
                    assert_eq!(echo.ends, echo.call_ends, "{what}");
                    let home = |x: &[GridNote]| {
                        x.last()
                            .is_some_and(|g| g.alt == 0 && g.pos.rem_euclid(7) == 0)
                    };
                    assert_eq!(home(&yours), echo.call_ends == Ending::Home, "{what}");
                    if ans.ends == Ending::Open {
                        assert!(!home(&a), "{what}: the answer should leave it open");
                    }
                    echo_like += like(&e);
                    answer_like += like(&a);
                    n += 1.0;
                }
            }
        }
        let (echo_like, answer_like) = (echo_like / n, answer_like / n);
        println!(
            "answer: echo like the call {echo_like:.2}, answer like the call {answer_like:.2}"
        );
        assert!(echo_like >= 0.7, "the echo: like the call {echo_like:.2}");
        assert!(
            echo_like > answer_like + 0.05,
            "echo {echo_like:.2}, answer {answer_like:.2}"
        );
    }

    #[test]
    fn a_long_call_is_answered_from_its_last_four_bars_and_nothing_is_refused() {
        let s = Settings::default();
        // Six bars: asks three times over.
        let mut long = asks();
        for k in 1..3 {
            long.extend(asks().into_iter().map(|(mut n, src)| {
                n.at += k * 2 * BAR;
                (n, src)
            }));
        }
        let a = answer(&call(&long, &s, Reply::Answer)).unwrap();
        assert_eq!(a.length, 4 * BAR);
        assert!(a.call.iter().all(|(n, _)| n.at < 4 * BAR));
        assert_eq!(a.call.len(), 2 * asks().len());
        // A call that starts late in its bar counts from that bar line.
        let late: Vec<_> = settles()
            .into_iter()
            .map(|(mut n, src)| {
                n.at += 3 * BAR + 8 * STEP;
                (n, src)
            })
            .collect();
        let a = answer(&call(&late, &s, Reply::Echo)).unwrap();
        assert_eq!(a.length, 2 * BAR);
        assert_eq!(a.call[0].0.at, 8 * STEP);
        assert!(a.notes.iter().all(|n| n.tick < a.length));
        assert!(matches!(
            answer(&call(&[], &s, Reply::Answer)),
            Err(Refused::NoClip)
        ));
    }

    #[test]
    fn answers_for_sets_the_answer_s_length_and_like_mine_follows_the_call() {
        let s = Settings::default();
        let notes = settles();
        for (bars, want) in [(None, 1), (Some(1), 1), (Some(2), 2), (Some(4), 4)] {
            let c = Call {
                bars,
                ..call(&notes, &s, Reply::Answer)
            };
            let a = answer(&c).unwrap();
            assert_eq!(a.call_length, BAR, "{bars:?}");
            assert_eq!(a.length, want * BAR, "{bars:?}");
            assert!(
                a.notes.iter().all(|n| n.tick + n.len <= a.length),
                "{bars:?}"
            );
            // Something to hear in its last bar.
            assert!(
                a.notes.iter().any(|n| n.tick >= a.length - BAR),
                "{bars:?}: the answer's last bar is empty"
            );
        }
    }
}
