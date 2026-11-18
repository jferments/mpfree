//! The CV section: the eight CV/gate outputs, each with its job, its track
//! and its main setting, and the volts it gives now. Each output's tuning
//! against a module is a fact about the rig, so it lives in Prefs.
//!
//! An LFO or an envelope ([`mpfree_midi::cv_mod`]) has more settings than
//! a row holds, so its output gets two more rows under its own, with the
//! same three columns:
//!
//! - **LFO:** the row's Track is the track whose notes restart it ("Runs
//!   free" for none) and its Setting the rate. Then Shape, Low and High;
//!   then Follows (Tempo: the rate in bars and notes, on the song's beat;
//!   Seconds: in hertz), Notes (Smooth, or the notes it snaps to at
//!   1 V/oct: Semitones, a scale, Fifths or Octaves) and Root (the scale's
//!   key, with C at 0 V).
//! - **Envelope:** the row's Track is the track whose keys open it and its
//!   Setting the top voltage. Then Attack, Decay and Sustain; then Release
//!   and Velocity (whether the top follows each note's).

use std::time::Duration;

use mpfree_midi::cv::{CvAssign, Priority, Scale, CV_OUTS};
use mpfree_midi::cv_mod::{Env, Lfo, Notes, Rate, Shape};
use mpfree_midi::output::MAX_TRACKS;
use mpfree_midi::setup::MAX_VOLTS;

use crate::form::{clamp_step, Field, Form, Row, Tone};
use crate::{note_name, IoModel};

pub(crate) struct Cv;

/// The jobs in the order the Job field steps through them.
const JOBS: [&str; 12] = [
    "Off", "Pitch", "Gate", "Trigger", "Velocity", "CC", "LFO", "Envelope", "Clock", "Run",
    "Reset", "Fixed",
];

/// An LFO's rates on the tempo: quarter notes a cycle, and their names.
const BEATS: [(f32, &str); 14] = [
    (64.0, "16 bars"),
    (32.0, "8 bars"),
    (16.0, "4 bars"),
    (8.0, "2 bars"),
    (4.0, "1 bar"),
    (2.0, "1/2"),
    (4.0 / 3.0, "1/2 T"),
    (1.0, "1/4"),
    (2.0 / 3.0, "1/4 T"),
    (0.5, "1/8"),
    (1.0 / 3.0, "1/8 T"),
    (0.25, "1/16"),
    (1.0 / 6.0, "1/16 T"),
    (0.125, "1/32"),
];
/// An LFO's rates in seconds.
const HZ: [f32; 19] = [
    0.02, 0.05, 0.1, 0.2, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0,
    16.0, 20.0,
];
/// What an LFO snaps to: pitch classes from the root.
const NOTES: [(u16, &str); 8] = [
    (0, "Smooth"),
    (0xFFF, "Semitones"),
    (0xAB5, "Major"),
    (0x5AD, "Minor"),
    (0x295, "Major pent."),
    (0x4A9, "Minor pent."),
    (0x081, "Fifths"),
    (0x001, "Octaves"),
];
const KEYS: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
/// An envelope's attack, decay and release times.
const TIMES_MS: [f32; 14] = [
    0.0, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0,
];

const PPQNS: [u16; 8] = [1, 2, 4, 8, 12, 24, 48, 96];
const PULSE_MS: [f32; 6] = [1.0, 2.0, 5.0, 10.0, 20.0, 50.0];

fn job(a: &CvAssign) -> usize {
    match a {
        CvAssign::Off => 0,
        CvAssign::Pitch { .. } => 1,
        CvAssign::Gate { .. } => 2,
        CvAssign::Trigger { .. } => 3,
        CvAssign::Velocity { .. } => 4,
        CvAssign::Cc { .. } => 5,
        CvAssign::Lfo(_) => 6,
        CvAssign::Envelope(_) => 7,
        CvAssign::Clock { .. } => 8,
        CvAssign::Run { .. } => 9,
        CvAssign::Reset { .. } => 10,
        CvAssign::Fixed { .. } => 11,
    }
}

/// The nearest of `list` to `v`.
fn nearest(list: impl Iterator<Item = f32>, v: f32) -> usize {
    list.enumerate()
        .min_by(|a, b| (a.1 - v).abs().total_cmp(&(b.1 - v).abs()))
        .map_or(0, |(i, _)| i)
}

fn rate_text(r: Rate) -> String {
    match r {
        Rate::Beats(b) => BEATS[nearest(BEATS.iter().map(|b| b.0), b)].1.to_string(),
        Rate::Hz(hz) => format!("{hz} Hz"),
    }
}

/// The rate's place in its list, and the list's length.
fn rate_at(r: Rate) -> (usize, usize) {
    match r {
        Rate::Beats(b) => (nearest(BEATS.iter().map(|b| b.0), b), BEATS.len()),
        Rate::Hz(hz) => (nearest(HZ.iter().copied(), hz), HZ.len()),
    }
}

fn ms_text(ms: f32) -> String {
    if ms >= 1000.0 {
        format!("{} s", ms / 1000.0)
    } else {
        format!("{ms} ms")
    }
}

fn notes_at(n: Notes) -> usize {
    if n.is_off() {
        0
    } else {
        NOTES.iter().position(|x| x.0 == n.mask).unwrap_or(1)
    }
}

fn track(a: &CvAssign) -> Option<u8> {
    match *a {
        CvAssign::Pitch { track, .. }
        | CvAssign::Gate { track, .. }
        | CvAssign::Trigger { track, .. }
        | CvAssign::Velocity { track, .. }
        | CvAssign::Cc { track, .. }
        | CvAssign::Envelope(Env { track, .. }) => Some(track),
        CvAssign::Lfo(l) => l.restart,
        _ => None,
    }
}

/// A job with its usual settings, on `track`.
fn fresh(j: usize, track: u8) -> CvAssign {
    match j {
        1 => CvAssign::Pitch {
            track,
            scale: Scale::VoltsPerOctave(1.0),
            zero_note: 36,
            bend_semis: 2,
            priority: Priority::Last,
        },
        2 => CvAssign::Gate {
            track,
            volts: MAX_VOLTS,
            retrigger_ms: 0.0,
        },
        3 => CvAssign::Trigger {
            track,
            note: None,
            volts: MAX_VOLTS,
            ms: 10.0,
        },
        4 => CvAssign::Velocity {
            track,
            max_volts: MAX_VOLTS,
        },
        5 => CvAssign::Cc {
            track,
            cc: 1,
            min_volts: 0.0,
            max_volts: MAX_VOLTS,
        },
        6 => CvAssign::Lfo(Lfo {
            high_volts: MAX_VOLTS,
            ..Lfo::default()
        }),
        7 => CvAssign::Envelope(Env {
            volts: MAX_VOLTS,
            ..Env::new(track)
        }),
        8 => CvAssign::Clock {
            ppqn: 4,
            volts: MAX_VOLTS,
            ms: 5.0,
        },
        9 => CvAssign::Run { volts: MAX_VOLTS },
        10 => CvAssign::Reset {
            volts: MAX_VOLTS,
            ms: 10.0,
        },
        11 => CvAssign::Fixed { volts: 0.0 },
        _ => CvAssign::Off,
    }
}

/// The main setting's text, its knob name and (index, count).
fn setting(a: &CvAssign) -> Option<(String, &'static str, usize, usize)> {
    let volts = |v: f32| ((v * 10.0).round() as usize, (MAX_VOLTS * 10.0) as usize + 1);
    Some(match *a {
        CvAssign::Off => return None,
        CvAssign::Pitch { zero_note, .. } => (
            format!("0 V at {}", note_name(zero_note)),
            "0 V at",
            zero_note as usize,
            128,
        ),
        CvAssign::Gate { volts: v, .. } => {
            let (i, n) = volts(v);
            (format!("{v:.1} V high"), "High", i, n)
        }
        CvAssign::Trigger { note, .. } => (
            note.map_or("Any note".to_string(), note_name),
            "Note",
            note.map_or(0, |n| n as usize + 1),
            129,
        ),
        CvAssign::Velocity { max_volts, .. } => {
            let (i, n) = volts(max_volts);
            (format!("up to {max_volts:.1} V"), "Top", i, n)
        }
        CvAssign::Cc { cc, .. } => (format!("CC {cc}"), "CC", cc as usize, 128),
        CvAssign::Lfo(l) => {
            let (at, n) = rate_at(l.rate);
            (rate_text(l.rate), "Rate", at, n)
        }
        CvAssign::Envelope(e) => {
            let (i, n) = volts(e.volts);
            (format!("up to {:.1} V", e.volts), "Top", i, n)
        }
        CvAssign::Clock { ppqn, .. } => (
            format!("{ppqn} PPQN"),
            "PPQN",
            PPQNS.iter().position(|p| *p == ppqn).unwrap_or(0),
            PPQNS.len(),
        ),
        CvAssign::Run { volts: v } => {
            let (i, n) = volts(v);
            (format!("{v:.1} V high"), "High", i, n)
        }
        CvAssign::Reset { ms, .. } => (
            format!("{ms:.0} ms pulse"),
            "Pulse",
            PULSE_MS.iter().position(|p| *p == ms).unwrap_or(0),
            PULSE_MS.len(),
        ),
        CvAssign::Fixed { volts: v } => {
            let (i, n) = volts(v);
            (format!("{v:.1} V"), "Volts", i, n)
        }
    })
}

/// Steps a value in tenths of a volt.
fn step_volts(v: &mut f32, d: i32) {
    let i = clamp_step(
        (*v * 10.0).round() as usize,
        d,
        (MAX_VOLTS * 10.0) as usize + 1,
    );
    *v = i as f32 / 10.0;
}

fn step_setting(a: &mut CvAssign, d: i32) {
    match a {
        CvAssign::Off => {}
        CvAssign::Pitch { zero_note, .. } => {
            *zero_note = clamp_step(*zero_note as usize, d, 128) as u8
        }
        CvAssign::Gate { volts, .. } | CvAssign::Run { volts } | CvAssign::Fixed { volts } => {
            step_volts(volts, d)
        }
        CvAssign::Velocity { max_volts, .. } => step_volts(max_volts, d),
        CvAssign::Trigger { note, .. } => {
            let at = note.map_or(0, |n| n as usize + 1);
            let to = clamp_step(at, d, 129);
            *note = (to > 0).then(|| (to - 1) as u8);
        }
        CvAssign::Cc { cc, .. } => *cc = clamp_step(*cc as usize, d, 128) as u8,
        CvAssign::Lfo(l) => {
            let (at, n) = rate_at(l.rate);
            let to = clamp_step(at, d, n);
            l.rate = match l.rate {
                Rate::Beats(_) => Rate::Beats(BEATS[to].0),
                Rate::Hz(_) => Rate::Hz(HZ[to]),
            };
        }
        CvAssign::Envelope(e) => step_volts(&mut e.volts, d),
        CvAssign::Clock { ppqn, .. } => {
            let at = PPQNS.iter().position(|p| p == ppqn).unwrap_or(0);
            *ppqn = PPQNS[clamp_step(at, d, PPQNS.len())];
        }
        CvAssign::Reset { ms, .. } => {
            let at = PULSE_MS.iter().position(|p| p == ms).unwrap_or(0);
            *ms = PULSE_MS[clamp_step(at, d, PULSE_MS.len())];
        }
    }
}

fn track_name(m: &IoModel, t: u8) -> String {
    m.live
        .tracks
        .get(t as usize)
        .cloned()
        .unwrap_or_else(|| format!("Track {}", t + 1))
}

fn set_track(a: &mut CvAssign, t: u8) {
    match a {
        CvAssign::Pitch { track, .. }
        | CvAssign::Gate { track, .. }
        | CvAssign::Trigger { track, .. }
        | CvAssign::Velocity { track, .. }
        | CvAssign::Cc { track, .. }
        | CvAssign::Envelope(Env { track, .. }) => *track = t,
        _ => {}
    }
}

/// An LFO's or envelope's two more rows: (title, note, fields).
fn more_rows(i: usize, a: &CvAssign, m: &IoModel) -> Vec<Row> {
    let volts = |v: f32| ((v * 10.0).round() as usize, (MAX_VOLTS * 10.0) as usize + 1);
    let time = |knob, ms: f32| {
        Field::choice(
            knob,
            format!("{knob} {}", ms_text(ms)),
            nearest(TIMES_MS.iter().copied(), ms),
            TIMES_MS.len(),
        )
    };
    let n = i + 1;
    match *a {
        CvAssign::Lfo(l) => {
            let shape = Shape::ALL.iter().position(|s| *s == l.shape).unwrap_or(0);
            let (lo, of) = volts(l.low_volts);
            let (hi, _) = volts(l.high_volts);
            // What it follows, or the notes it snaps to (the title column is
            // narrow beside the Q-Links).
            let snap = match (l.rate, l.restart) {
                _ if !l.notes.is_off() => "snaps to notes, 1 V/oct".to_string(),
                (_, Some(t)) => format!("restarts on {}'s notes", track_name(m, t)),
                (Rate::Beats(_), None) => "on the song's beat".to_string(),
                (Rate::Hz(_), None) => "runs free".to_string(),
            };
            vec![
                Row::item(format!("cv:{i}:1"), format!("CV {n} wave"))
                    .note("shape and range", Tone::Faint)
                    .field(Field::choice(
                        "Shape",
                        l.shape.name(),
                        shape,
                        Shape::ALL.len(),
                    ))
                    .field(Field::choice(
                        "Low",
                        format!("Low {:.1} V", l.low_volts),
                        lo,
                        of,
                    ))
                    .field(Field::choice(
                        "High",
                        format!("High {:.1} V", l.high_volts),
                        hi,
                        of,
                    )),
                Row::item(format!("cv:{i}:2"), format!("CV {n} timing"))
                    .note(snap, Tone::Faint)
                    .field(Field::choice(
                        "Follows",
                        match l.rate {
                            Rate::Beats(_) => "Tempo",
                            Rate::Hz(_) => "Seconds",
                        },
                        usize::from(matches!(l.rate, Rate::Hz(_))),
                        2,
                    ))
                    .field(Field::choice(
                        "Notes",
                        NOTES[notes_at(l.notes)].1,
                        notes_at(l.notes),
                        NOTES.len(),
                    ))
                    .field(
                        Field::choice(
                            "Root",
                            format!("Root {}", KEYS[usize::from(l.notes.root % 12)]),
                            usize::from(l.notes.root % 12),
                            12,
                        )
                        .off(!l.notes.is_off()),
                    ),
            ]
        }
        CvAssign::Envelope(e) => {
            let sus = (e.sustain * 20.0).round() as usize;
            vec![
                Row::item(format!("cv:{i}:1"), format!("CV {n} shape"))
                    .note(
                        format!("opened by {}'s keys", track_name(m, e.track)),
                        Tone::Faint,
                    )
                    .field(time("Attack", e.attack_ms))
                    .field(time("Decay", e.decay_ms))
                    .field(Field::choice(
                        "Sustain",
                        format!("Sustain {}%", sus * 5),
                        sus,
                        21,
                    )),
                Row::item(format!("cv:{i}:2"), format!("CV {n} release"))
                    .note("after the last key lets go", Tone::Faint)
                    // The row's title names it: the time alone fits.
                    .field(Field::choice(
                        "Release",
                        ms_text(e.release_ms),
                        nearest(TIMES_MS.iter().copied(), e.release_ms),
                        TIMES_MS.len(),
                    ))
                    .field(Field::choice(
                        "Velocity",
                        if e.velocity {
                            "By velocity"
                        } else {
                            "Fixed top"
                        },
                        usize::from(e.velocity),
                        2,
                    )),
            ]
        }
        _ => Vec::new(),
    }
}

/// Steps field `col` of an LFO's or envelope's row `r` (1 or 2).
fn step_more(a: &mut CvAssign, r: usize, col: usize, d: i32) {
    let time = |ms: &mut f32| {
        let at = nearest(TIMES_MS.iter().copied(), *ms);
        *ms = TIMES_MS[clamp_step(at, d, TIMES_MS.len())];
    };
    match (a, r, col) {
        (CvAssign::Lfo(l), 1, 0) => {
            let at = Shape::ALL.iter().position(|s| *s == l.shape).unwrap_or(0);
            l.shape = Shape::ALL[clamp_step(at, d, Shape::ALL.len())];
        }
        (CvAssign::Lfo(l), 1, 1) => step_volts(&mut l.low_volts, d),
        (CvAssign::Lfo(l), 1, 2) => step_volts(&mut l.high_volts, d),
        (CvAssign::Lfo(l), 2, 0) => {
            // At 120 BPM a beat is half a second: the nearest the other way.
            l.rate = match (l.rate, d > 0) {
                (Rate::Beats(b), true) => Rate::Hz(HZ[nearest(HZ.iter().copied(), 2.0 / b)]),
                (Rate::Hz(hz), false) => {
                    Rate::Beats(BEATS[nearest(BEATS.iter().map(|b| b.0), 2.0 / hz)].0)
                }
                (r, _) => r,
            };
        }
        (CvAssign::Lfo(l), 2, 1) => {
            let to = clamp_step(notes_at(l.notes), d, NOTES.len());
            l.notes.mask = NOTES[to].0;
        }
        (CvAssign::Lfo(l), 2, 2) if !l.notes.is_off() => {
            l.notes.root = clamp_step(usize::from(l.notes.root % 12), d, 12) as u8
        }
        (CvAssign::Envelope(e), 1, 0) => time(&mut e.attack_ms),
        (CvAssign::Envelope(e), 1, 1) => time(&mut e.decay_ms),
        (CvAssign::Envelope(e), 1, 2) => {
            let at = (e.sustain * 20.0).round() as usize;
            e.sustain = clamp_step(at, d, 21) as f32 / 20.0;
        }
        (CvAssign::Envelope(e), 2, 0) => time(&mut e.release_ms),
        (CvAssign::Envelope(e), 2, 1) => e.velocity = d > 0,
        _ => {}
    }
}

impl Form for Cv {
    fn label(&self) -> &'static str {
        crate::CV
    }

    fn widths(&self) -> &'static [i32] {
        &[300, 170, 170, 230]
    }

    fn banner(&self, m: &IoModel) -> Option<String> {
        match usize::from(m.live.built.cv_outs) {
            0 => Some(
                "CV out is off in this run: MPFree sends nothing to the jacks, and what \
                 you set here is kept with the project."
                    .to_string(),
            ),
            n if n < CV_OUTS => Some(format!(
                "CV 1 to {n} are sent; CV {} to {CV_OUTS} wait for a check on the MPC. \
                 The volts are not yet measured at the jacks.",
                n + 1
            )),
            _ => None,
        }
    }

    fn rows(&self, m: &IoModel, _now: Duration) -> Vec<Row> {
        let mut rows = vec![Row::heading("Outputs", vec!["Job", "Track", "Setting"])];
        for (i, a) in m.setup.cv.iter().enumerate() {
            let v = m.live.cv_volts[i];
            let mut row = Row::item(format!("cv:{i}"), format!("CV {}", i + 1));
            row = if *a == CvAssign::Off {
                row.note(format!("{v:.2} V · off"), Tone::Faint)
            } else if m.live.built.cv_outs == 0 {
                row.note("sends nothing: CV out is off", Tone::Warn)
            } else if i >= usize::from(m.live.built.cv_outs) {
                row.note("sends nothing yet: not checked on the MPC", Tone::Warn)
            } else {
                row.note(format!("{v:.2} V now"), Tone::Normal)
            };
            row = row.field(Field::choice("Job", JOBS[job(a)], job(a), JOBS.len()));
            row = row.field(match (a, track(a)) {
                (CvAssign::Lfo(_), t) => Field::choice(
                    "Restart",
                    t.map_or("Runs free".to_string(), |t| track_name(m, t)),
                    t.map_or(0, |t| t as usize + 1),
                    MAX_TRACKS + 1,
                ),
                (_, Some(t)) => Field::choice("Track", track_name(m, t), t as usize, MAX_TRACKS),
                (_, None) => Field::choice("Track", "—", 0, MAX_TRACKS).off(false),
            });
            row = row.field(match setting(a) {
                Some((text, knob, at, n)) => Field::choice(knob, text, at, n),
                None => Field::choice("Setting", "—", 0, 1).off(false),
            });
            rows.push(row);
            rows.extend(more_rows(i, a, m));
        }
        rows
    }

    fn step(&self, m: &mut IoModel, key: &str, col: usize, delta: i32) -> bool {
        let Some((i, more)) = key.strip_prefix("cv:").and_then(|k| {
            let (i, r) = k.split_once(':').unwrap_or((k, "0"));
            let i = i.parse::<usize>().ok().filter(|i| *i < CV_OUTS)?;
            Some((i, r.parse::<usize>().ok()?))
        }) else {
            return false;
        };
        let before = m.setup.cv[i];
        let a = &mut m.setup.cv[i];
        if more > 0 {
            step_more(a, more, col, delta);
            return m.setup.cv[i] != before;
        }
        match col {
            0 => {
                let j = (job(a) as i32 + delta).rem_euclid(JOBS.len() as i32) as usize;
                *a = fresh(j, track(a).unwrap_or(0));
            }
            1 => match a {
                CvAssign::Lfo(l) => {
                    let to = clamp_step(
                        l.restart.map_or(0, |t| t as usize + 1),
                        delta,
                        MAX_TRACKS + 1,
                    );
                    l.restart = (to > 0).then(|| (to - 1) as u8);
                }
                _ => {
                    if let Some(t) = track(a) {
                        set_track(a, clamp_step(t as usize, delta, MAX_TRACKS) as u8);
                    }
                }
            },
            _ => step_setting(a, delta),
        }
        m.setup.cv[i] != before
    }

    fn live(&self, m: &IoModel, _now: Duration) -> u64 {
        m.live.cv_volts.iter().fold(0u64, |h, v| {
            h.wrapping_mul(1_000_003)
                .wrapping_add((v * 100.0).round() as i64 as u64)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::form::testing::{paint, tap, AREA};
    use crate::form::FormSection;
    use crate::{Built, Live};
    use mpfree_midi::setup::MidiSetup;
    use mpfree_mixer_ui::{IoInput, IoReply, IoSection};
    use std::sync::{Arc, Mutex};

    fn model() -> crate::Shared {
        Arc::new(Mutex::new(IoModel {
            setup: MidiSetup::default(),
            live: Live::default(),
            ..IoModel::default()
        }))
    }

    #[test]
    fn eight_outputs_off_at_first() {
        let s = model();
        let rows = Cv.rows(&s.lock().unwrap(), Duration::ZERO);
        assert_eq!(rows.len(), 1 + CV_OUTS);
        assert_eq!(rows[1].title, "CV 1");
        assert_eq!(rows[1].fields[0].text, "Off");
        assert!(!rows[1].fields[1].enabled);
        assert_eq!(rows[1].note, "0.00 V · off");
    }

    #[test]
    fn a_tap_on_job_makes_it_pitch_with_the_usual_settings() {
        let s = model();
        let mut sec = FormSection::new(Cv, s.clone());
        let _ = paint(&sec);
        let f = sec.field_rect("cv:0", 0, AREA).unwrap();
        assert_eq!(tap(&mut sec, f.x + 10, f.y + 10), IoReply::Redraw);
        let a = s.lock().unwrap().setup.cv[0];
        assert_eq!(a, fresh(1, 0));
        let rows = Cv.rows(&s.lock().unwrap(), Duration::ZERO);
        assert_eq!(rows[1].fields[2].text, "0 V at C2");
        assert_eq!(rows[1].note, "sends nothing: CV out is off");
        // The Q-Links: Job, Track, the setting.
        let k = sec.qlinks().unwrap();
        assert_eq!(
            [
                k[0].label.as_str(),
                k[1].label.as_str(),
                k[2].label.as_str()
            ],
            ["Job", "Track", "0 V at"]
        );
        sec.qlink_turn(1, 3, false);
        sec.qlink_turn(2, 12, false);
        let rows = Cv.rows(&s.lock().unwrap(), Duration::ZERO);
        assert_eq!(rows[1].fields[1].text, "Track 4");
        assert_eq!(rows[1].fields[2].text, "0 V at C3");
    }

    #[test]
    fn a_new_job_keeps_the_track_and_settings_stay_in_range() {
        let s = model();
        let mut m = s.lock().unwrap();
        m.setup.cv[3] = fresh(2, 6);
        assert!(Cv.step(&mut m, "cv:3", 0, 1));
        assert_eq!(m.setup.cv[3], fresh(3, 6));
        // Volts stop at the jack's top.
        m.setup.cv[3] = fresh(11, 0);
        Cv.step(&mut m, "cv:3", 2, 100);
        assert_eq!(m.setup.cv[3], CvAssign::Fixed { volts: MAX_VOLTS });
        assert!(!Cv.step(&mut m, "cv:3", 2, 1));
        // The job wraps round from Fixed to Off.
        Cv.step(&mut m, "cv:3", 0, 1);
        assert_eq!(m.setup.cv[3], CvAssign::Off);
        // What the section sets reads back from the project's text.
        m.setup.cv[5] = fresh(8, 0);
        Cv.step(&mut m, "cv:5", 2, 1);
        let (back, warn) = MidiSetup::parse(&m.setup.to_text());
        assert!(warn.is_empty(), "{warn:?}");
        assert_eq!(back.cv, m.setup.cv);
    }

    #[test]
    fn a_tap_or_knob_that_cannot_move_a_setting_says_why() {
        let s = model();
        s.lock().unwrap().setup.cv[4] = CvAssign::Run { volts: MAX_VOLTS };
        let mut sec = FormSection::new(Cv, s.clone());
        let _ = paint(&sec);
        // High is at 5.0 V already: the tap selects it and says why.
        let f = sec.field_rect("cv:4", 2, AREA).unwrap();
        tap(&mut sec, f.x + 10, f.y + 10);
        assert_eq!(
            s.lock().unwrap().setup.cv[4],
            CvAssign::Run { volts: MAX_VOLTS }
        );
        assert_eq!(
            sec.take_note().as_deref(),
            Some("CV 5 High is at its highest, 5.0 V high: - or Q-Link 3 lowers it")
        );
        assert_eq!(sec.take_note(), None, "said once");
        // Its Q-Link turned further up says the same; down moves it.
        assert_eq!(sec.qlink_turn(2, 1, false), IoReply::Handled);
        assert!(sec.take_note().is_some());
        assert_eq!(sec.qlink_turn(2, -1, false), IoReply::Redraw);
        assert_eq!(sec.take_note(), None);
        // Run has no track: its Track field is faint and says it can't change.
        let f = sec.field_rect("cv:4", 1, AREA).unwrap();
        tap(&mut sec, f.x + 10, f.y + 10);
        let n = sec.take_note().unwrap();
        assert!(n.starts_with("CV 5 Track can't change"), "{n}");
        // The page arrows at the last row say so.
        sec.input(IoInput::Wheel(20));
        assert_eq!(sec.input(IoInput::Page(1)), IoReply::Handled);
        assert_eq!(sec.take_note().as_deref(), Some("The last row is selected"));
    }

    #[test]
    fn volts_now_show_once_built() {
        let s = model();
        {
            let mut m = s.lock().unwrap();
            m.live.built = Built {
                cv_outs: 4,
                ..Built::default()
            };
            m.setup.cv[0] = fresh(2, 0);
            m.live.cv_volts[0] = 5.0;
        }
        let rows = Cv.rows(&s.lock().unwrap(), Duration::ZERO);
        assert_eq!(rows[1].note, "5.00 V now");
        let mut sec = FormSection::new(Cv, s.clone());
        assert_eq!(sec.tick(Duration::ZERO), IoReply::Redraw);
        assert_eq!(sec.tick(Duration::ZERO), IoReply::Ignored);
        s.lock().unwrap().live.cv_volts[0] = 0.0;
        assert_eq!(sec.tick(Duration::ZERO), IoReply::Redraw);
    }

    /// An LFO's two more rows: its wave, then its timing; the restart
    /// track, the rate and every setting step and read back from the
    /// project's text.
    #[test]
    fn an_lfo_gets_two_more_rows_and_its_settings_step() {
        let s = model();
        let mut m = s.lock().unwrap();
        m.setup.cv[1] = fresh(6, 0);
        let rows = Cv.rows(&m, Duration::ZERO);
        assert_eq!(rows.len(), 1 + CV_OUTS + 2);
        let texts = |r: &Row| r.fields.iter().map(|f| f.text.clone()).collect::<Vec<_>>();
        assert_eq!(texts(&rows[2]), ["LFO", "Runs free", "1 bar"]);
        assert_eq!(rows[3].title, "CV 2 wave");
        assert_eq!(texts(&rows[3]), ["Sine", "Low 0.0 V", "High 5.0 V"]);
        assert_eq!(rows[4].title, "CV 2 timing");
        assert_eq!(texts(&rows[4]), ["Tempo", "Smooth", "Root C"]);
        assert_eq!(rows[4].note, "on the song's beat");
        assert!(!rows[4].fields[2].enabled, "no root while smooth");
        assert_eq!(rows[5].title, "CV 3");
        // The rate, faster; a restart track; a ramp from 1 V.
        assert!(Cv.step(&mut m, "cv:1", 2, 2));
        assert!(Cv.step(&mut m, "cv:1", 1, 2));
        assert!(Cv.step(&mut m, "cv:1:1", 0, 2));
        assert!(Cv.step(&mut m, "cv:1:1", 1, 10));
        // Snapped to a minor scale from A; then on seconds.
        assert!(Cv.step(&mut m, "cv:1:2", 1, 3));
        assert!(Cv.step(&mut m, "cv:1:2", 2, 9));
        let rows = Cv.rows(&m, Duration::ZERO);
        assert_eq!(texts(&rows[2]), ["LFO", "Track 2", "1/2 T"]);
        assert_eq!(texts(&rows[3]), ["Ramp up", "Low 1.0 V", "High 5.0 V"]);
        assert_eq!(texts(&rows[4]), ["Tempo", "Minor", "Root A"]);
        assert_eq!(rows[4].note, "snaps to notes, 1 V/oct");
        assert!(Cv.step(&mut m, "cv:1:2", 0, 1));
        assert!(!Cv.step(&mut m, "cv:1:2", 0, 1), "already on seconds");
        let rows = Cv.rows(&m, Duration::ZERO);
        // 1/2 T is 4/3 of a beat: 1.5 Hz at 120 BPM.
        assert_eq!(texts(&rows[2]), ["LFO", "Track 2", "1.5 Hz"]);
        assert_eq!(texts(&rows[4])[0], "Seconds");
        // Runs free again, past the first track.
        assert!(Cv.step(&mut m, "cv:1", 1, -5));
        assert_eq!(texts(&Cv.rows(&m, Duration::ZERO)[2])[1], "Runs free");
        let (back, warn) = MidiSetup::parse(&m.setup.to_text());
        assert!(warn.is_empty(), "{warn:?}");
        assert_eq!(back.cv, m.setup.cv);
        // A job past the LFO drops its rows.
        assert!(Cv.step(&mut m, "cv:1", 0, 2));
        assert_eq!(Cv.rows(&m, Duration::ZERO).len(), 1 + CV_OUTS);
    }

    /// An envelope's rows: attack, decay and sustain, then release and
    /// velocity, on the track its keys come from.
    #[test]
    fn an_envelope_gets_two_more_rows_on_its_track() {
        let s = model();
        let mut m = s.lock().unwrap();
        m.setup.cv[0] = fresh(7, 2);
        let rows = Cv.rows(&m, Duration::ZERO);
        let texts = |r: &Row| r.fields.iter().map(|f| f.text.clone()).collect::<Vec<_>>();
        assert_eq!(texts(&rows[1]), ["Envelope", "Track 3", "up to 5.0 V"]);
        assert_eq!(
            texts(&rows[2]),
            ["Attack 5 ms", "Decay 200 ms", "Sustain 60%"]
        );
        assert_eq!(rows[2].note, "opened by Track 3's keys");
        assert_eq!(texts(&rows[3]), ["300 ms", "Fixed top"]);
        assert!(Cv.step(&mut m, "cv:0:1", 0, -3));
        assert!(Cv.step(&mut m, "cv:0:1", 1, 3));
        assert!(Cv.step(&mut m, "cv:0:1", 2, -12));
        assert!(Cv.step(&mut m, "cv:0:2", 0, 100));
        assert!(Cv.step(&mut m, "cv:0:2", 1, 1));
        assert!(Cv.step(&mut m, "cv:0", 2, -10));
        let rows = Cv.rows(&m, Duration::ZERO);
        assert_eq!(texts(&rows[1]), ["Envelope", "Track 3", "up to 4.0 V"]);
        assert_eq!(texts(&rows[2]), ["Attack 0 ms", "Decay 2 s", "Sustain 0%"]);
        assert_eq!(texts(&rows[3]), ["10 s", "By velocity"]);
        // The Q-Links take a row's fields: on the shape row, its three.
        drop(m);
        let mut sec = FormSection::new(Cv, s.clone());
        let _ = paint(&sec);
        let f = sec.field_rect("cv:0:1", 1, AREA).unwrap();
        tap(&mut sec, f.x + 10, f.y + 10);
        let k = sec.qlinks().unwrap();
        assert_eq!(
            [
                k[0].label.as_str(),
                k[1].label.as_str(),
                k[2].label.as_str()
            ],
            ["Attack", "Decay", "Sustain"]
        );
        let m = s.lock().unwrap();
        let (back, warn) = MidiSetup::parse(&m.setup.to_text());
        assert!(warn.is_empty(), "{warn:?}");
        assert_eq!(back.cv, m.setup.cv);
    }

    /// Outputs past the ones this run drives say so on their row, and the
    /// banner names which are sent.
    #[test]
    fn unchecked_outputs_say_they_send_nothing() {
        let s = model();
        let mut m = s.lock().unwrap();
        m.setup.cv[0] = fresh(2, 0);
        m.setup.cv[4] = fresh(8, 0);
        let rows = Cv.rows(&m, Duration::ZERO);
        assert_eq!(rows[1].note, "sends nothing: CV out is off");
        assert!(Cv
            .banner(&m)
            .unwrap()
            .starts_with("CV out is off in this run"));
        m.live.built.cv_outs = 4;
        let rows = Cv.rows(&m, Duration::ZERO);
        assert_eq!(rows[1].note, "0.00 V now");
        assert_eq!(rows[5].note, "sends nothing yet: not checked on the MPC");
        assert_eq!(rows[6].note, "0.00 V · off");
        assert!(Cv
            .banner(&m)
            .unwrap()
            .starts_with("CV 1 to 4 are sent; CV 5 to 8 wait for a check on the MPC."));
        m.live.built.cv_outs = 8;
        assert_eq!(Cv.banner(&m), None);
    }
}
