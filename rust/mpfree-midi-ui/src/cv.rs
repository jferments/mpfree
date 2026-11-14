//! The CV section: the eight CV/gate outputs, each with its job, its track
//! and its main setting, and the volts it gives now. Each output's tuning
//! against a module is a fact about the rig, so it lives in Prefs.

use std::time::Duration;

use mpfree_midi::cv::{CvAssign, Priority, Scale, CV_OUTS};
use mpfree_midi::output::MAX_TRACKS;
use mpfree_midi::setup::MAX_VOLTS;

use crate::form::{clamp_step, Field, Form, Row, Tone};
use crate::{note_name, IoModel};

pub(crate) struct Cv;

/// The jobs in the order the Job field steps through them.
const JOBS: [&str; 10] = [
    "Off", "Pitch", "Gate", "Trigger", "Velocity", "CC", "Clock", "Run", "Reset", "Fixed",
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
        CvAssign::Clock { .. } => 6,
        CvAssign::Run { .. } => 7,
        CvAssign::Reset { .. } => 8,
        CvAssign::Fixed { .. } => 9,
    }
}

fn track(a: &CvAssign) -> Option<u8> {
    match *a {
        CvAssign::Pitch { track, .. }
        | CvAssign::Gate { track, .. }
        | CvAssign::Trigger { track, .. }
        | CvAssign::Velocity { track, .. }
        | CvAssign::Cc { track, .. } => Some(track),
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
        6 => CvAssign::Clock {
            ppqn: 4,
            volts: MAX_VOLTS,
            ms: 5.0,
        },
        7 => CvAssign::Run { volts: MAX_VOLTS },
        8 => CvAssign::Reset {
            volts: MAX_VOLTS,
            ms: 10.0,
        },
        9 => CvAssign::Fixed { volts: 0.0 },
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
        | CvAssign::Cc { track, .. } => *track = t,
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
            row = row.field(match track(a) {
                Some(t) => Field::choice("Track", track_name(m, t), t as usize, MAX_TRACKS),
                None => Field::choice("Track", "—", 0, MAX_TRACKS).off(false),
            });
            row = row.field(match setting(a) {
                Some((text, knob, at, n)) => Field::choice(knob, text, at, n),
                None => Field::choice("Setting", "—", 0, 1).off(false),
            });
            rows.push(row);
        }
        rows
    }

    fn step(&self, m: &mut IoModel, key: &str, col: usize, delta: i32) -> bool {
        let Some(i) = key
            .strip_prefix("cv:")
            .and_then(|i| i.parse::<usize>().ok())
            .filter(|i| *i < CV_OUTS)
        else {
            return false;
        };
        let before = m.setup.cv[i];
        let a = &mut m.setup.cv[i];
        match col {
            0 => {
                let j = (job(a) as i32 + delta).rem_euclid(JOBS.len() as i32) as usize;
                *a = fresh(j, track(a).unwrap_or(0));
            }
            1 => {
                if let Some(t) = track(a) {
                    set_track(a, clamp_step(t as usize, delta, MAX_TRACKS) as u8);
                }
            }
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
        m.setup.cv[3] = fresh(9, 0);
        Cv.step(&mut m, "cv:3", 2, 100);
        assert_eq!(m.setup.cv[3], CvAssign::Fixed { volts: MAX_VOLTS });
        assert!(!Cv.step(&mut m, "cv:3", 2, 1));
        // The job wraps round from Fixed to Off.
        Cv.step(&mut m, "cv:3", 0, 1);
        assert_eq!(m.setup.cv[3], CvAssign::Off);
        // What the section sets reads back from the project's text.
        m.setup.cv[5] = fresh(6, 0);
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

    /// Outputs past the ones this run drives say so on their row, and the
    /// banner names which are sent.
    #[test]
    fn unchecked_outputs_say_they_send_nothing() {
        let s = model();
        let mut m = s.lock().unwrap();
        m.setup.cv[0] = fresh(2, 0);
        m.setup.cv[4] = fresh(6, 0);
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
