//! Workflow scripts: a musician's session written down as steps, which the
//! app plays by itself (`mpfree play --script FILE`, the device round's
//! scripted runs) and the UI smoke library plays the same way on every pull
//! request (`smoke/scripts.rs`). The device round asked for it (19 Nov 2026):
//! test what merged on the MPC with as few hands as possible, through
//! realistic workflows that use several tools together.
//!
//! On the MPC the steps go in where the hardware's do: a button, a pad, a
//! Q-Link or the wheel as the controller's events (`player_session` takes
//! them beside what the ports read), a tap as a finger on the screen (the
//! screen thread finds the control by its widget, id and label, as the
//! smoke tests do), and MIDI from gear as bytes on the 5-pin MIDI In. So
//! everything after the input is the app's own code on the MPC's own cores:
//! the sound, the screen, the lanes. The run says each step in its log
//! (`script: ...` lines), and the device round's tools read the log.
//!
//! One step per line; `#` starts a comment. Time runs on in the steps: each
//! takes as long as below, then the next starts.
//!
//! ```text
//! title A beat from scratch           the workflow's name in reports
//! covers rec-arm, step-pads           the device checks it stands in for
//! needs demo-songs                    the three demo songs in the library first
//! needs stream-audio                  takes play from their files for this run
//! wait 1.5                            seconds
//! tap header tab Sequencer            widget, id, then the label (may be empty)
//! button PLAY                         a hardware button by the mapper's name
//! button SHIFT + NOTE REPEAT          the first held while the second is pressed
//! button-down ERASE                   one edge (for a chord with the pads)
//! button-up ERASE
//! hold 2 PLAY START                   held for 2 s
//! pad 1 100                           pad 1 (of 16) at velocity 100, 0.1 s
//! pad 1 100 0.5                       held 0.5 s
//! pad-down 1 100 / pad-up 1           one edge
//! qlink 1 +3                          Q-Link 1 (of 4) turned 3 steps
//! wheel -2                            the data wheel
//! wheel SHIFT + 1                     the wheel with SHIFT held
//! gear 90 3C 64                       MIDI bytes from a keyboard on MIDI In
//! mark heavy                          a named point: the run logs its totals
//! expect project: Demo is in          a line holding this text since the last mark
//! expect-not refused                  no line holding this since the last mark
//! expect-on-mpc track 1: plays FM     as expect, on the MPC only: a line from the
//!                                     sound side, which the tests' session leaves out
//! ```
//!
//! A button takes 6 periods, a chord 11, a pad its hold and 2 periods, a
//! Q-Link or wheel step 4 (9 with a button held), a tap a quarter of a
//! second (the screen draws the page it leads to before the next), MIDI from
//! gear 2 periods. The times match the smoke session's own, so a script
//! plays the same on the MPC and in the tests.

use std::fmt;

/// What a script can ask to be set up before its steps.
pub const NEEDS: [&str; 2] = [DEMO_SONGS, STREAM_AUDIO];
/// The three demo songs in the library (demo_songs.rs).
pub const DEMO_SONGS: &str = "demo-songs";
/// Takes play from their files for this run, as Dev's "Audio tracks from
/// their files" has them, without changing the saved setting (the stream
/// test's workflows, D3 in docs/dev/design/disk-streaming.md).
pub const STREAM_AUDIO: &str = "stream-audio";

/// The longest script the app plays (and its `--seconds` for the run).
pub const MAX_SECONDS: f64 = 1800.0;
/// Periods a second, at the app's rate and period.
const PERIODS_A_SECOND: f64 = crate::RATE as f64 / crate::PERIOD as f64;
/// One period, in seconds.
const PERIOD_S: f64 = crate::PERIOD as f64 / crate::RATE as f64;
/// How long a tap takes: the screen thread taps, then draws the page the
/// tap leads to, before the next step can find a control on it.
pub const TAP_S: f64 = 0.25;

/// One step as written.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Wait(f64),
    Tap {
        widget: String,
        id: String,
        label: String,
    },
    /// A button pressed and let go; with `held`, while that one is held.
    Button {
        held: Option<String>,
        name: String,
    },
    ButtonEdge {
        name: String,
        down: bool,
    },
    Hold {
        seconds: f64,
        name: String,
    },
    /// Pad 0 to 15.
    Pad {
        pad: u8,
        velocity: u8,
        hold: f64,
    },
    PadEdge {
        pad: u8,
        velocity: u8,
        down: bool,
    },
    /// Q-Link 0 to 3.
    QLink {
        knob: u8,
        delta: i8,
    },
    Wheel {
        held: Option<String>,
        delta: i8,
    },
    Gear(Vec<u8>),
    Mark(String),
    Expect {
        text: String,
        present: bool,
        /// Only the MPC can see it (a line from the sound side, which the
        /// smoke session leaves out); the tests pass it by.
        mpc_only: bool,
    },
}

/// A step with where it stands in the script.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Its line in the file, from 1.
    pub number: usize,
    pub step: Step,
    /// The period it starts on, from the script's start.
    pub at: u64,
    /// How many periods it takes.
    pub len: u64,
}

/// What reaches the app at one period on the MPC.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Tap {
        widget: String,
        id: String,
        label: String,
    },
    Button {
        name: String,
        down: bool,
    },
    Pad {
        pad: u8,
        velocity: u8,
        down: bool,
    },
    QLink {
        knob: u8,
        delta: i8,
    },
    Wheel {
        delta: i8,
    },
    Gear(Vec<u8>),
    Mark(String),
    Expect {
        text: String,
        present: bool,
        /// Only the MPC can see it (a line from the sound side, which the
        /// smoke session leaves out); the tests pass it by.
        mpc_only: bool,
    },
}

/// An act at its period, with the script line it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct Timed {
    pub period: u64,
    pub line: usize,
    pub act: Act,
}

/// A whole script, parsed and timed.
#[derive(Clone, Debug, PartialEq)]
pub struct Script {
    pub title: String,
    /// The device checks it stands in for, as named in `app_checks.py`.
    pub covers: Vec<String>,
    /// What the run sets up before the steps: `demo-songs` puts the three
    /// demo songs (demo_songs.rs) in the data folder's library.
    pub needs: Vec<String>,
    pub lines: Vec<Line>,
    /// Its length in periods.
    pub periods: u64,
}

/// A line the parser could not read.
#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

/// Periods in `seconds`, rounded up, as the smoke session's `run_for`.
pub fn periods_in(seconds: f64) -> u64 {
    (seconds / PERIOD_S).ceil().max(0.0) as u64
}

fn number<T: std::str::FromStr>(word: &str, what: &str) -> Result<T, String> {
    word.trim_start_matches('+')
        .parse()
        .map_err(|_| format!("{what}: {word:?} is not a number"))
}

fn seconds(word: &str) -> Result<f64, String> {
    let s: f64 = number(word, "seconds")?;
    if !(0.0..=MAX_SECONDS).contains(&s) || !s.is_finite() {
        return Err(format!("seconds: {word} is not 0 to {MAX_SECONDS}"));
    }
    Ok(s)
}

fn pad_number(word: &str) -> Result<u8, String> {
    let n: u8 = number(word, "pad")?;
    if !(1..=16).contains(&n) {
        return Err(format!("pad {n}: the pads are 1 to 16"));
    }
    Ok(n - 1)
}

fn velocity(word: &str) -> Result<u8, String> {
    let v: u8 = number(word, "velocity")?;
    if !(1..=127).contains(&v) {
        return Err(format!("velocity {v}: 1 to 127"));
    }
    Ok(v)
}

fn delta(word: &str) -> Result<i8, String> {
    let d: i8 = number(word, "turn")?;
    if d == 0 || !(-63..=63).contains(&d) {
        return Err(format!("turn {d}: -63 to 63, not 0"));
    }
    Ok(d)
}

/// "A + B": A held while B; else just B.
fn held_and(rest: &str) -> (Option<String>, String) {
    match rest.split_once(" + ") {
        Some((held, name)) => (Some(held.trim().to_string()), name.trim().to_string()),
        None => (None, rest.trim().to_string()),
    }
}

/// The button names a script may use: the control mapper's.
fn known_button(name: &str) -> Result<(), String> {
    let known = crate::app::mapper::CONTROLS
        .iter()
        .any(|c| c.name == name && !matches!(c.job, crate::app::mapper::Job::None));
    if known {
        Ok(())
    } else {
        Err(format!(
            "no button called {name:?} (the names are the control mapper's, as \"PAD BANK A/E\")"
        ))
    }
}

fn step(verb: &str, rest: &str) -> Result<Step, String> {
    let words: Vec<&str> = rest.split_whitespace().collect();
    let want = |n: usize| -> Result<(), String> {
        if words.len() == n {
            Ok(())
        } else {
            Err(format!("{verb} takes {n} word(s), not {}", words.len()))
        }
    };
    Ok(match verb {
        "wait" => {
            want(1)?;
            Step::Wait(seconds(words[0])?)
        }
        "tap" => {
            if words.len() < 2 {
                return Err(
                    "tap takes a widget, an id and a label (the label may be empty)".into(),
                );
            }
            let label = rest
                .trim_start()
                .split_once(char::is_whitespace)
                .and_then(|(_, r)| r.trim_start().split_once(char::is_whitespace))
                .map(|(_, l)| l.trim().to_string())
                .unwrap_or_default();
            Step::Tap {
                widget: words[0].to_string(),
                id: words[1].to_string(),
                label,
            }
        }
        "button" => {
            let (held, name) = held_and(rest);
            if let Some(h) = &held {
                known_button(h)?;
            }
            known_button(&name)?;
            Step::Button { held, name }
        }
        "button-down" | "button-up" => {
            let name = rest.trim().to_string();
            known_button(&name)?;
            Step::ButtonEdge {
                name,
                down: verb == "button-down",
            }
        }
        "hold" => {
            let (s, name) = rest
                .trim()
                .split_once(char::is_whitespace)
                .ok_or("hold takes seconds, then a button")?;
            let name = name.trim().to_string();
            known_button(&name)?;
            Step::Hold {
                seconds: seconds(s)?,
                name,
            }
        }
        "pad" => {
            if !(2..=3).contains(&words.len()) {
                return Err("pad takes a pad (1 to 16), a velocity and, if held, seconds".into());
            }
            Step::Pad {
                pad: pad_number(words[0])?,
                velocity: velocity(words[1])?,
                hold: words.get(2).map_or(Ok(0.1), |w| seconds(w))?,
            }
        }
        "pad-down" => {
            want(2)?;
            Step::PadEdge {
                pad: pad_number(words[0])?,
                velocity: velocity(words[1])?,
                down: true,
            }
        }
        "pad-up" => {
            want(1)?;
            Step::PadEdge {
                pad: pad_number(words[0])?,
                velocity: 0,
                down: false,
            }
        }
        "qlink" => {
            want(2)?;
            let k: u8 = number(words[0], "Q-Link")?;
            if !(1..=4).contains(&k) {
                return Err(format!("Q-Link {k}: 1 to 4"));
            }
            Step::QLink {
                knob: k - 1,
                delta: delta(words[1])?,
            }
        }
        "wheel" => {
            let (held, d) = held_and(rest);
            if let Some(h) = &held {
                known_button(h)?;
            }
            Step::Wheel {
                held,
                delta: delta(&d)?,
            }
        }
        "gear" => {
            if words.is_empty() || words.len() > 16 {
                return Err("gear takes 1 to 16 bytes in hex".into());
            }
            let bytes = words
                .iter()
                .map(|w| {
                    u8::from_str_radix(w, 16).map_err(|_| format!("gear: {w:?} is not a hex byte"))
                })
                .collect::<Result<Vec<u8>, String>>()?;
            if bytes[0] < 0x80 {
                return Err("gear: the first byte is a status byte (80 to FF)".into());
            }
            Step::Gear(bytes)
        }
        "mark" => {
            want(1)?;
            Step::Mark(words[0].to_string())
        }
        "expect" | "expect-not" | "expect-on-mpc" => {
            let text = rest.trim().to_string();
            if text.is_empty() {
                return Err(format!("{verb} takes the text to look for"));
            }
            Step::Expect {
                text,
                present: verb != "expect-not",
                mpc_only: verb == "expect-on-mpc",
            }
        }
        _ => return Err(format!("no step called {verb:?}")),
    })
}

impl Step {
    /// How many periods the step takes (the smoke session's own times).
    pub fn len(&self) -> u64 {
        match self {
            Step::Wait(s) => periods_in(*s),
            Step::Tap { .. } => periods_in(TAP_S),
            Step::Button { held: None, .. } => 6,
            Step::Button { held: Some(_), .. } => 11,
            Step::ButtonEdge { .. } => 2,
            Step::Hold { seconds, .. } => periods_in(*seconds) + 4,
            Step::Pad { hold, .. } => periods_in(*hold) + 2,
            Step::PadEdge { .. } => 2,
            Step::QLink { .. } => 4,
            Step::Wheel { held: None, .. } => 4,
            Step::Wheel { held: Some(_), .. } => 9,
            Step::Gear(_) => 2,
            Step::Mark(_) | Step::Expect { .. } => 0,
        }
    }

    /// What reaches the app, by periods after the step's start.
    pub fn acts(&self) -> Vec<(u64, Act)> {
        let button = |name: &str, down| Act::Button {
            name: name.to_string(),
            down,
        };
        match self {
            Step::Wait(_) => Vec::new(),
            Step::Tap { widget, id, label } => vec![(
                0,
                Act::Tap {
                    widget: widget.clone(),
                    id: id.clone(),
                    label: label.clone(),
                },
            )],
            Step::Button { held: None, name } => {
                vec![(0, button(name, true)), (2, button(name, false))]
            }
            Step::Button {
                held: Some(h),
                name,
            } => vec![
                (0, button(h, true)),
                (1, button(name, true)),
                (3, button(name, false)),
                (7, button(h, false)),
            ],
            Step::ButtonEdge { name, down } => vec![(0, button(name, *down))],
            Step::Hold { seconds, name } => {
                vec![
                    (0, button(name, true)),
                    (periods_in(*seconds), button(name, false)),
                ]
            }
            Step::Pad {
                pad,
                velocity,
                hold,
            } => vec![
                (
                    0,
                    Act::Pad {
                        pad: *pad,
                        velocity: *velocity,
                        down: true,
                    },
                ),
                (
                    periods_in(*hold),
                    Act::Pad {
                        pad: *pad,
                        velocity: 0,
                        down: false,
                    },
                ),
            ],
            Step::PadEdge {
                pad,
                velocity,
                down,
            } => vec![(
                0,
                Act::Pad {
                    pad: *pad,
                    velocity: *velocity,
                    down: *down,
                },
            )],
            Step::QLink { knob, delta } => vec![(
                0,
                Act::QLink {
                    knob: *knob,
                    delta: *delta,
                },
            )],
            Step::Wheel { held: None, delta } => vec![(0, Act::Wheel { delta: *delta })],
            Step::Wheel {
                held: Some(h),
                delta,
            } => vec![
                (0, button(h, true)),
                (1, Act::Wheel { delta: *delta }),
                (5, button(h, false)),
            ],
            Step::Gear(b) => vec![(0, Act::Gear(b.clone()))],
            Step::Mark(m) => vec![(0, Act::Mark(m.clone()))],
            Step::Expect {
                text,
                present,
                mpc_only,
            } => vec![(
                0,
                Act::Expect {
                    text: text.clone(),
                    present: *present,
                    mpc_only: *mpc_only,
                },
            )],
        }
    }
}

impl Script {
    /// Reads a script; the first line it can't read is the error.
    pub fn parse(text: &str) -> Result<Script, Error> {
        let mut s = Script {
            title: String::new(),
            covers: Vec::new(),
            needs: Vec::new(),
            lines: Vec::new(),
            periods: 0,
        };
        let mut at = 0u64;
        for (i, raw) in text.lines().enumerate() {
            let number = i + 1;
            let line = raw.split_once('#').map_or(raw, |(l, _)| l).trim();
            if line.is_empty() {
                continue;
            }
            let (verb, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
            // The app's own labels and lines have their dots, arrows and
            // halves; the rest is typed, and a look-alike letter would
            // name nothing.
            if !line.is_ascii()
                && !matches!(
                    verb,
                    "title" | "mark" | "tap" | "expect" | "expect-not" | "expect-on-mpc"
                )
            {
                return Err(Error {
                    line: number,
                    message: "keep scripts ASCII (a tap, an expect, a mark or the title may quote the app's own characters)".into(),
                });
            }
            match verb {
                "title" => s.title = rest.trim().to_string(),
                "covers" => s.covers.extend(
                    rest.split(',')
                        .map(|c| c.trim().to_string())
                        .filter(|c| !c.is_empty()),
                ),
                "needs" => {
                    for need in rest.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                        if !NEEDS.contains(&need) {
                            return Err(Error {
                                line: number,
                                message: format!(
                                    "needs {need:?}: a script can need {}",
                                    NEEDS.join(", ")
                                ),
                            });
                        }
                        s.needs.push(need.to_string());
                    }
                }
                _ => {
                    let step = step(verb, rest).map_err(|message| Error {
                        line: number,
                        message,
                    })?;
                    let len = step.len();
                    s.lines.push(Line {
                        number,
                        step,
                        at,
                        len,
                    });
                    at += len;
                }
            }
        }
        if s.title.is_empty() {
            return Err(Error {
                line: 1,
                message: "a script starts with its title (title ...)".into(),
            });
        }
        if at as f64 / PERIODS_A_SECOND > MAX_SECONDS {
            return Err(Error {
                line: s.lines.last().map_or(1, |l| l.number),
                message: format!("the script runs past {MAX_SECONDS} s"),
            });
        }
        s.periods = at;
        Ok(s)
    }

    /// Its length in seconds.
    pub fn seconds(&self) -> f64 {
        self.periods as f64 / PERIODS_A_SECOND
    }

    /// Every act, in the order they reach the app.
    pub fn timeline(&self) -> Vec<Timed> {
        let mut out: Vec<Timed> = self
            .lines
            .iter()
            .flat_map(|l| {
                l.step.acts().into_iter().map(move |(d, act)| Timed {
                    period: l.at + d,
                    line: l.number,
                    act,
                })
            })
            .collect();
        // Stable: acts at one period keep the script's order.
        out.sort_by_key(|t| t.period);
        out
    }
}

/// Seconds a scripted run goes on after its last step: the status lines
/// at its end (`script: done`, a second after the last step) and the
/// sounds' tails.
pub const TAIL_S: u64 = 3;

/// The number the app reads for button `name` (the control mapper's
/// name), as Map controls or the measured map gave it.
pub fn button_note(app: &crate::app::App, name: &str) -> Option<u8> {
    let c = crate::app::mapper::CONTROLS
        .iter()
        .find(|c| c.name == name)?;
    app.map.of(c.job)
}

/// Every button a script can press gets a number: the measured map has
/// them all on the reference MPC, and any missing (a map that lost one)
/// gets a free number for this run only, as the smoke session's do.
/// Nothing is saved: a scripted run's data folder is its own. Says what
/// it did.
pub fn map_unmapped(app: &mut crate::app::App) -> String {
    use crate::app::mapper::{Job, CONTROLS};
    let used: Vec<u8> = CONTROLS.iter().filter_map(|c| app.map.of(c.job)).collect();
    let mut free = (90u8..=127).filter(|n| !used.contains(n));
    let mut given = Vec::new();
    for c in CONTROLS.iter() {
        if matches!(c.job, Job::None | Job::Fixed(_)) || app.map.of(c.job).is_some() {
            continue;
        }
        let Some(n) = free.next() else {
            break;
        };
        app.map.learn(c.job, Some(n));
        given.push(format!("{} = {n}", c.name));
    }
    if given.is_empty() {
        "script: every button has its number (the measured map)".to_string()
    } else {
        format!(
            "script: {} buttons had no number and got one for this run: {}",
            given.len(),
            given.join(", ")
        )
    }
}

/// `needs demo-songs`: the three demo songs put in the library in `dir`
/// (the run's `--data`, its own folder) as saved projects with the parts
/// `defaults` gives a new one, on a thread of its own at idle priority.
/// Says what it did through `sink` (the screen thread prints it).
pub fn put_demo_songs(
    dir: std::path::PathBuf,
    defaults: crate::project::Defaults,
    sink: Option<std::sync::mpsc::SyncSender<crate::gui::ToUi>>,
) {
    let say = move |line: String| match &sink {
        Some(s) => {
            let _ = s.try_send(crate::gui::ToUi::Line(line));
        }
        None => println!("{line}"),
    };
    let spawned = std::thread::Builder::new()
        .name("mpfree-demo-songs".into())
        .spawn(move || {
            mpfree_lanes::os::idle_job();
            let lib = match mpfree_library::Library::at(&dir) {
                Ok(l) => l,
                Err(e) => return say(format!("script: demo songs not put in the library ({e:?})")),
            };
            let mut names = Vec::new();
            for song in crate::demo_songs::all() {
                match crate::demo_songs::put(&lib, &song, &defaults) {
                    Ok(_) => names.push(song.name),
                    Err(e) => say(format!("script: demo song not put in the library: {e}")),
                }
            }
            say(format!(
                "script: demo songs in the library: {}",
                names.join(", ")
            ));
        });
    if let Err(e) = spawned {
        println!("script: demo songs not put in the library (no thread: {e})");
    }
}

/// The acts of a running script, handed out as their periods come.
pub struct Runner {
    acts: Vec<Timed>,
    next: usize,
}

impl Runner {
    pub fn new(script: &Script) -> Runner {
        Runner {
            acts: script.timeline(),
            next: 0,
        }
    }

    /// The acts due by `period` (from the script's start) not handed out yet.
    pub fn due(&mut self, period: u64) -> &[Timed] {
        let from = self.next;
        while self.next < self.acts.len() && self.acts[self.next].period <= period {
            self.next += 1;
        }
        &self.acts[from..self.next]
    }

    /// Every act handed out.
    #[cfg(test)]
    pub fn done(&self) -> bool {
        self.next >= self.acts.len()
    }
}

/// The log line for an act, as the run writes it (`script: ...`); the
/// device round's tools read these to know where each step was.
pub fn said(t: &Timed) -> String {
    let what = match &t.act {
        Act::Tap { widget, id, label } => format!("tap {widget} {id} {label}"),
        Act::Button { name, down } => {
            format!("button {name} {}", if *down { "down" } else { "up" })
        }
        Act::Pad {
            pad,
            velocity,
            down: true,
        } => format!("pad {} down {velocity}", pad + 1),
        Act::Pad { pad, .. } => format!("pad {} up", pad + 1),
        Act::QLink { knob, delta } => format!("qlink {} {delta:+}", knob + 1),
        Act::Wheel { delta } => format!("wheel {delta:+}"),
        Act::Gear(b) => format!(
            "gear {}",
            b.iter()
                .map(|x| format!("{x:02X}"))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        Act::Mark(m) => format!("mark {m}"),
        Act::Expect {
            text,
            mpc_only: true,
            ..
        } => format!("expect-on-mpc {text}"),
        Act::Expect {
            text,
            present: true,
            ..
        } => format!("expect {text}"),
        Act::Expect { text, .. } => format!("expect-not {text}"),
    };
    format!(
        "script: line {} at {:.3} s: {what}",
        t.line,
        t.period as f64 / PERIODS_A_SECOND
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEMO: &str = "\
title A beat
covers step-pads, track-mute
needs demo-songs
# a comment
wait 1
tap header tab Sequencer
button PLAY
button SHIFT + NOTE REPEAT
pad 1 100 0.5
qlink 2 -3
wheel SHIFT + 1
gear 90 3C 64
mark heavy
expect playing
expect-not refused
";

    #[test]
    fn a_script_can_need_its_takes_played_from_their_files() {
        let s = Script::parse("title t\nneeds demo-songs, stream-audio\nwait 1\n").expect("reads");
        assert_eq!(s.needs, [DEMO_SONGS, STREAM_AUDIO]);
    }

    #[test]
    fn a_script_reads_and_times_its_steps() {
        let s = Script::parse(DEMO).expect("reads");
        assert_eq!(s.title, "A beat");
        assert_eq!(s.covers, ["step-pads", "track-mute"]);
        assert_eq!(s.needs, ["demo-songs"]);
        assert_eq!(s.lines.len(), 11);
        let lens: Vec<u64> = s.lines.iter().map(|l| l.len).collect();
        // wait 1 s = 345 periods (344.5 up); tap 87; button 6; chord 11;
        // pad 0.5 s + 2; qlink 4; wheel with SHIFT 9; gear 2; marks 0.
        assert_eq!(lens, [345, 87, 6, 11, 175, 4, 9, 2, 0, 0, 0]);
        assert_eq!(s.lines[2].at, 345 + 87);
        assert_eq!(s.periods, lens.iter().sum::<u64>());
        assert!((s.seconds() - s.periods as f64 * 128.0 / 44100.0).abs() < 1e-9);
    }

    #[test]
    fn a_tap_keeps_its_label_with_spaces_and_may_have_none() {
        let s = Script::parse("title t\ntap browser row My Song 2\ntap transport pads\n")
            .expect("reads");
        assert_eq!(
            s.lines[0].step,
            Step::Tap {
                widget: "browser".into(),
                id: "row".into(),
                label: "My Song 2".into()
            }
        );
        assert_eq!(
            s.lines[1].step,
            Step::Tap {
                widget: "transport".into(),
                id: "pads".into(),
                label: String::new()
            }
        );
    }

    #[test]
    fn the_plus_and_minus_buttons_and_chords_with_them_read() {
        let s = Script::parse("title t\nbutton +\nbutton SHIFT + -\nwheel -1\n").expect("reads");
        assert_eq!(
            s.lines[0].step,
            Step::Button {
                held: None,
                name: "+".into()
            }
        );
        assert_eq!(
            s.lines[1].step,
            Step::Button {
                held: Some("SHIFT".into()),
                name: "-".into()
            }
        );
        assert_eq!(
            s.lines[2].step,
            Step::Wheel {
                held: None,
                delta: -1
            }
        );
    }

    #[test]
    fn what_a_script_cannot_say_is_refused_with_its_line() {
        for (text, line, part) in [
            ("wait 1\n", 1, "title"),
            ("title t\nbutton FILL\n", 2, "no button called"),
            ("title t\nbutton Other 1\n", 2, "no button called"),
            ("title t\npad 17 100\n", 2, "1 to 16"),
            ("title t\npad 1 0\n", 2, "1 to 127"),
            ("title t\nqlink 5 1\n", 2, "1 to 4"),
            ("title t\nqlink 1 0\n", 2, "not 0"),
            ("title t\ngear 3C 64\n", 2, "status byte"),
            ("title t\ngear 9Z\n", 2, "hex"),
            ("title t\nwait 2000\n", 2, "0 to 1800"),
            ("title t\njump 3\n", 2, "no step"),
            ("title t\ntap header\n", 2, "widget"),
            ("title t\nexpect\n", 2, "text"),
            ("title t\nwait 900\nwait 901\n", 3, "runs past"),
            ("title t\nbutton Pl\u{e4}y\n", 2, "ASCII"),
            ("title t\nneeds songs\n", 2, "demo-songs, stream-audio"),
        ] {
            let e = Script::parse(text).expect_err(text);
            assert_eq!(e.line, line, "{text}: {e}");
            assert!(e.message.contains(part), "{text}: {e}");
        }
    }

    #[test]
    fn the_runner_hands_out_each_act_once_as_its_period_comes() {
        let s = Script::parse("title t\nbutton PLAY\npad 3 90 0.01\nmark end\n").expect("reads");
        let mut r = Runner::new(&s);
        let first: Vec<Act> = r.due(0).iter().map(|t| t.act.clone()).collect();
        assert_eq!(
            first,
            [Act::Button {
                name: "PLAY".into(),
                down: true
            }]
        );
        assert!(r.due(1).is_empty());
        assert_eq!(r.due(2).len(), 1, "PLAY let go two periods on");
        // The pad at 6, its release 4 periods later (0.01 s is 3.4
        // periods, rounded up), the mark when the pad's step ends (6 + 4 + 2).
        let rest: Vec<(u64, String)> = r.due(100).iter().map(|t| (t.period, said(t))).collect();
        assert_eq!(rest.len(), 3);
        assert_eq!(rest[0].0, 6);
        assert!(rest[0].1.ends_with("pad 3 down 90"), "{rest:?}");
        assert_eq!(rest[1].0, 10);
        assert_eq!(rest[2].0, 12);
        assert!(
            rest[2].1.contains("script: line 4 at 0.035 s: mark end"),
            "{rest:?}"
        );
        assert!(r.done());
        assert!(r.due(1000).is_empty());
    }

    #[test]
    fn every_button_a_script_names_has_a_number_on_a_fresh_app() {
        let mut app = crate::app::App::new(44_100, false);
        let said = map_unmapped(&mut app);
        assert!(said.contains("every button has its number"), "{said}");
        for c in crate::app::mapper::CONTROLS.iter() {
            if !matches!(c.job, crate::app::mapper::Job::None) {
                assert!(button_note(&app, c.name).is_some(), "{}", c.name);
            }
        }
        assert_eq!(button_note(&app, "Other 1"), None);
    }

    #[test]
    fn a_button_without_a_number_gets_a_free_one_for_the_run() {
        use crate::app::mapper::Job;
        let mut app = crate::app::App::new(44_100, false);
        app.map.learn(Job::TapTempo, None);
        assert_eq!(button_note(&app, "TAP TEMPO"), None);
        let said = map_unmapped(&mut app);
        let n = button_note(&app, "TAP TEMPO").expect("a number now");
        assert!(said.contains(&format!("TAP TEMPO = {n}")), "{said}");
        let others: Vec<u8> = crate::app::mapper::CONTROLS
            .iter()
            .filter(|c| c.name != "TAP TEMPO")
            .filter_map(|c| button_note(&app, c.name))
            .collect();
        assert!(!others.contains(&n), "{n} is free");
    }

    #[test]
    fn every_workflow_in_the_device_round_reads() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/mpc-one/devtest/workflows");
        let mut n = 0;
        for e in std::fs::read_dir(&dir).expect("the workflows folder") {
            let p = e.expect("an entry").path();
            if p.extension().and_then(|x| x.to_str()) != Some("mpw") {
                continue;
            }
            let text = std::fs::read_to_string(&p).expect("readable");
            let s = Script::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            assert!(
                !s.covers.is_empty(),
                "{}: says which checks it covers",
                p.display()
            );
            assert!(s.seconds() <= 600.0, "{}: at most 10 minutes", p.display());
            n += 1;
        }
        assert!(n >= 6, "the device round's workflows are there ({n})");
    }
}
