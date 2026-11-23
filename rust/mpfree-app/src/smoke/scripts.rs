//! The device round's workflow scripts (`tools/mpc-one/devtest/workflows/`,
//! script.rs) played in the smoke session on every pull request: each
//! step through the same entry points as the MPC's buttons, pads, wheel and
//! touchscreen, at the times the app plays them on the MPC, and each
//! `expect` checked against the lines the session wrote (the control
//! side's and what the screen said: both reach the MPC's log). A script that
//! passes here does what it says; the device round then plays it on the
//! MPC's own cores (`mpfree play --script`) to see what it costs there.

use super::Session;
use crate::script::{Script, Step, DEMO_SONGS, STREAM_AUDIO};

/// One period, in seconds (the session's clock).
const PERIOD_S: f64 = crate::PERIOD as f64 / crate::RATE as f64;

/// The session's periods so far.
fn periods(s: &Session) -> u64 {
    (s.t() / PERIOD_S).round() as u64
}

/// Plays `w` in `s` from now, step by step, each to its end on the script's
/// clock. Returns what went wrong: a control not on the page, an `expect`
/// not met, a step that took longer in the session than the script allows
/// (then the MPC and the tests would drift apart).
///
/// With `long_s`, every wait longer than that plays as `long_s` and the
/// time it leaves out is taken off the script's clock: the stream test's
/// workflows (D3) record and play for minutes on the MPC, which here would
/// write hundreds of MB. (What streaming did is read from the status
/// lines the MPC's run writes at each mark, not checked here: this
/// session runs faster than the drive could keep up.)
pub fn play(s: &mut Session, w: &Script, long_s: Option<f64>) -> Vec<String> {
    let mut wrong = Vec::new();
    if w.needs.iter().any(|n| n == STREAM_AUDIO) && !s.tracks.streams() {
        wrong
            .push("needs stream-audio, and the session's takes don't play from their files".into());
    }
    if w.needs.iter().any(|n| n == DEMO_SONGS) {
        match s.library.clone() {
            Some(lib) => {
                let d = s.defaults();
                for song in crate::demo_songs::all() {
                    if let Err(e) = crate::demo_songs::put(&lib, &song, &d) {
                        wrong.push(format!("demo song not put: {e}"));
                    }
                }
            }
            None => wrong.push("needs demo-songs, and the session has no library".into()),
        }
    }
    // The script's clock: the session's periods, plus those the long
    // waits left out.
    let base = periods(s);
    let mut left_out = 0u64;
    // Where the last mark was, in the control side's lines and in what
    // the screen said (both reach the MPC's log).
    let mut mark_at = (s.lines.len(), s.screen.said_all.len());
    for l in &w.lines {
        let at = |s: &Session, left_out: u64| periods(s) - base + left_out;
        match &l.step {
            Step::Wait(secs) => {
                if let Some(long) = long_s.filter(|long| secs > long) {
                    let short = (long / PERIOD_S).ceil() as u64;
                    left_out += l.len.saturating_sub(short);
                }
            }
            Step::Tap { widget, id, label } => {
                if s.control(widget, id, label).is_some() {
                    s.press(widget, id, label);
                } else {
                    let all = s.controls();
                    let name = |o: &mpfree_ui::PageTarget| {
                        format!("{}/{} \"{}\"", o.widget, o.target.id, o.target.label)
                    };
                    let mut have: Vec<String> = all
                        .iter()
                        .filter(|o| o.widget == *widget || o.target.id == *id)
                        .map(name)
                        .collect();
                    // Nothing like it: the whole page, to write the step from.
                    if have.is_empty() {
                        have = all.iter().map(name).collect();
                    }
                    wrong.push(format!(
                        "line {}: no control {widget}/{id} \"{label}\" on {}; it has {}",
                        l.number,
                        s.page(),
                        have.join(", ")
                    ));
                }
            }
            Step::Button { held: None, name } => {
                s.button(name);
            }
            Step::Button {
                held: Some(h),
                name,
            } => {
                s.button_with(h, name);
            }
            Step::ButtonEdge { name, down } => {
                s.button_edge(name, *down);
            }
            Step::Hold { seconds, name } => {
                s.hold(name, *seconds);
            }
            Step::Pad {
                pad,
                velocity,
                hold,
            } => {
                s.pad(usize::from(*pad), *velocity, *hold);
            }
            Step::PadEdge {
                pad,
                velocity,
                down,
            } => s.pad_edge(usize::from(*pad), *velocity, *down),
            Step::QLink { knob, delta } => {
                s.qlink(usize::from(*knob), i32::from(*delta));
            }
            Step::Wheel { held: None, delta } => {
                s.wheel(i32::from(*delta));
            }
            Step::Wheel {
                held: Some(h),
                delta,
            } => {
                s.wheel_with(h, i32::from(*delta));
            }
            Step::Gear(bytes) => {
                s.midi_from_gear(bytes);
            }
            Step::Mark(_) => mark_at = (s.lines.len(), s.screen.said_all.len()),
            // A line only the MPC writes (an instrument built beside the
            // sound): the device round's report checks it.
            Step::Expect { mpc_only: true, .. } => {}
            Step::Expect { text, present, .. } => {
                let since: Vec<&String> = s.lines[mark_at.0.min(s.lines.len())..]
                    .iter()
                    .chain(&s.screen.said_all[mark_at.1.min(s.screen.said_all.len())..])
                    .collect();
                let seen = since.iter().any(|x| x.contains(text.as_str()));
                if seen != *present {
                    let recent: Vec<&&String> = since.iter().rev().take(12).collect();
                    wrong.push(format!(
                        "line {}: {} {text:?} since the last mark; the last lines: {recent:?}",
                        l.number,
                        if *present {
                            "no line with"
                        } else {
                            "a line with"
                        }
                    ));
                }
            }
        }
        let end = l.at + l.len;
        let now = at(s, left_out);
        if now > end {
            wrong.push(format!(
                "line {}: took {} periods in the session, the script allows {}",
                l.number,
                now - l.at,
                l.len
            ));
        } else {
            s.run_periods(end - now);
        }
    }
    wrong
}

#[cfg(test)]
mod tests {
    use super::super::{on_big_stack, Session};
    use super::*;
    use std::path::{Path, PathBuf};

    fn folder() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/mpc-one/devtest/workflows")
    }

    /// The stream test's workflows (D3, `host_streamtest_mpc.py`).
    fn stream_folder() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/mpc-one/streamtest/workflows")
    }

    /// The longest wait a workflow that needs stream-audio plays here.
    const LONG_WAIT_S: f64 = 1.0;
    /// The audio track memory a session that streams has: the MPC's
    /// default (Prefs > Audio track memory, 1:00).
    const STREAM_MEMORY_S: u32 = 60;

    /// Plays one workflow file in a session with a library of its own.
    fn check(name: &str) {
        check_file(folder().join(format!("{name}.mpw")));
    }

    /// Plays one of the stream test's workflows.
    fn check_stream(name: &str) {
        check_file(stream_folder().join(format!("{name}.mpw")));
    }

    /// A session whose takes play from their files and record to them, as
    /// `needs stream-audio` has the MPC's: the stream layer started first,
    /// then the library's work folder looked at and files on.
    fn streaming(s: &mut Session) {
        s.tracks = crate::audiotracks::TrackHost::new(crate::RATE, STREAM_MEMORY_S);
        assert!(
            s.tracks.stream(STREAM_MEMORY_S).is_some(),
            "streaming starts"
        );
        s.screen.recover_audio_files();
        let wall = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !s.tracks.host.tracks.records_to_files() || s.tracks.host.tracks.player().is_none() {
            assert!(
                std::time::Instant::now() < wall,
                "timed out: files and the player"
            );
            s.run_periods(4);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    fn check_file(path: PathBuf) {
        on_big_stack(move || {
            let name = path
                .file_stem()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let w = Script::parse(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
            let dir =
                std::env::temp_dir().join(format!("mpfree-devtest-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("a scratch folder");
            let mut s = Session::with_library(&dir);
            let streams = w.needs.iter().any(|n| n == STREAM_AUDIO);
            if streams {
                streaming(&mut s);
            }
            let wrong = play(&mut s, &w, streams.then_some(LONG_WAIT_S));
            let _ = std::fs::remove_dir_all(&dir);
            // MPFREE_SCRIPT_LINES=1: every line the session wrote, to write
            // a script's expects from.
            if std::env::var_os("MPFREE_SCRIPT_LINES").is_some() {
                for l in &s.lines {
                    eprintln!("{name}: {l}");
                }
                for l in &s.screen.said_all {
                    eprintln!("{name} (screen): {l}");
                }
            }
            assert!(wrong.is_empty(), "{name}.mpw:\n  {}", wrong.join("\n  "));
        });
    }

    /// A Q-Link's rest is timed on the session's clock, not the wall
    /// clock's: Grain pitch turned up, a second of the session, turned back
    /// is two undo steps; turned up, a second of the wall clock with no
    /// periods, turned back is one turn that changed nothing. The Sample
    /// page workflow's Grain pitch check (`sample-page.mpw`) failed in CI,
    /// whose session played its `wait 1` faster than a second.
    #[test]
    fn a_qlink_turns_rest_is_timed_on_the_sessions_clock() {
        const GRAIN: &str = "title Grain pitch turned twice\nwait 1\npad 1 100 0.5\ntap header tab Sample\ntap smp-tools tool Grain\ntap smp-region choice Grain: Grains\n";
        let run = |name: &str, first: &str, between: Option<std::time::Duration>, last: &str| {
            let dir = std::env::temp_dir()
                .join(format!("mpfree-devtest-rest-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("a scratch folder");
            let mut s = Session::with_library(&dir);
            let mut wrong = play(&mut s, &Script::parse(first).expect("the first half"), None);
            if let Some(d) = between {
                std::thread::sleep(d);
            }
            wrong.extend(play(
                &mut s,
                &Script::parse(last).expect("the second half"),
                None,
            ));
            let _ = std::fs::remove_dir_all(&dir);
            assert!(wrong.is_empty(), "{name}:\n  {}", wrong.join("\n  "));
        };
        on_big_stack(move || {
            run(
                "session",
                &format!("{GRAIN}qlink 4 +4\nwait 1\n"),
                None,
                "title Turned back\nqlink 4 -4\nwait 0.5\ntap smp-region choice Grain: Grain LFO\nexpect : pitch +0 st -> +4 st\n",
            );
            run(
                "wall",
                &format!("{GRAIN}qlink 4 +4\n"),
                Some(std::time::Duration::from_millis(1_200)),
                "title Turned back\nqlink 4 -4\nwait 0.5\ntap smp-region choice Grain: Grain LFO\nexpect-not : pitch +0 st\n",
            );
        });
    }

    /// The workflows below, one test each; a file in the folder that isn't
    /// listed here fails [`every_workflow_file_has_its_test`].
    macro_rules! workflows {
        ($($test:ident => $file:literal,)*) => {
            const LISTED: &[&str] = &[$($file),*];
            $(
                #[test]
                fn $test() {
                    check($file);
                }
            )*
        };
    }

    workflows! {
        workflow_beat_from_scratch => "beat-from-scratch",
        workflow_loops => "loops",
        workflow_every_synth => "every-synth",
        workflow_effects => "effects",
        workflow_songs => "songs",
        workflow_sample_page => "sample-page",
        workflow_heavy_load => "heavy-load",
        workflow_smart_loop_end => "smart-loop-end",
    }

    /// MPFREE_SCRIPT=FILE plays that file as the tests above play the
    /// folder's (a workflow being written, with MPFREE_SCRIPT_LINES=1 to
    /// see what the session wrote); without it, nothing.
    #[test]
    fn the_script_mpfree_script_names() {
        if let Some(p) = std::env::var_os("MPFREE_SCRIPT") {
            check_file(PathBuf::from(p));
        }
    }

    /// The stream test's workflows, one test each, played with their
    /// takes on files ([`streaming`]) and the long waits short.
    macro_rules! stream_workflows {
        ($($test:ident => $file:literal,)*) => {
            const STREAM_LISTED: &[&str] = &[$($file),*];
            $(
                #[test]
                fn $test() {
                    check_stream($file);
                }
            )*
        };
    }

    stream_workflows! {
        stream_eight_tracks => "eight-tracks",
    }

    /// The workflows' names in `dir`, sorted.
    fn files_in(dir: PathBuf) -> Vec<String> {
        let mut files: Vec<String> = std::fs::read_dir(dir)
            .expect("the workflows folder")
            .filter_map(|e| {
                let p = e.ok()?.path();
                (p.extension()? == "mpw").then(|| p.file_stem()?.to_str().map(str::to_string))?
            })
            .collect();
        files.sort();
        files
    }

    #[test]
    fn every_workflow_file_has_its_test() {
        for (dir, listed) in [(folder(), LISTED), (stream_folder(), STREAM_LISTED)] {
            let mut listed: Vec<String> = listed.iter().map(|s| s.to_string()).collect();
            listed.sort();
            assert_eq!(
                files_in(dir),
                listed,
                "each workflow in the folder has its test in smoke/scripts.rs"
            );
        }
    }

    /// The device round's workflows play from memory, as its runs do
    /// (their data folders are in RAM); the stream test's all need their
    /// takes on files.
    #[test]
    fn only_the_stream_tests_workflows_need_streaming() {
        for (dir, want) in [(folder(), false), (stream_folder(), true)] {
            for name in files_in(dir.clone()) {
                let text = std::fs::read_to_string(dir.join(format!("{name}.mpw"))).unwrap();
                let w = Script::parse(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
                assert_eq!(
                    w.needs.iter().any(|n| n == STREAM_AUDIO),
                    want,
                    "{name}.mpw"
                );
            }
        }
    }
}
