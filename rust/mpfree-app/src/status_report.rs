//! System status's report (Save a report): one plain text file about this
//! session, to attach to a bug report. The screen thread writes the words
//! ([`text`]) from its status board and readings; the sound loop gives them
//! to the store like a screen picture ([`save`]: `files/reports/`, named by
//! the time, UTC by the MPC's clock).
//!
//! What it never holds: serial numbers, network addresses (Prefs > System
//! info already leaves both out), the host name, or the names of projects,
//! samples, tracks and takes. A problem about a track or a take says which
//! by its number ("track 3"); a MIDI port keeps its device's name, since a
//! missing "Juno-X" is often the point.

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use mpfree_status::{Board, Level, ProblemState, Subject};
use mpfree_store::Store;

use crate::status_screen::Readings;
use crate::status_watch::Watch;
use crate::sysinfo::SystemInfo;

/// The store's folder for reports: `files/reports/`.
pub const OWNER: &str = "reports";

/// How many of the last `status:` log lines a report keeps.
pub const LINES: usize = 200;

/// Rows of Prefs > System info a report leaves out.
const LEFT_OUT: [&str; 1] = ["Host name"];

/// What the report says besides the board.
pub struct Facts<'a> {
    /// When it was saved, seconds after 1970 (UTC, the MPC's clock).
    pub saved: u64,
    /// When this run's status board started.
    pub start: Instant,
    pub now: Instant,
    pub system: &'a SystemInfo,
    /// This start: None for a normal one; a safe start after a crash, with
    /// the crash's panic line when there was one.
    pub safe_start: Option<Option<String>>,
    pub readings: &'a Readings,
    /// The last `status:` log lines, oldest first, as [`log_line`] made
    /// them.
    pub lines: &'a [String],
}

/// A time since the start: "0:12", "14:03", "1:02:03".
pub fn since(d: Duration) -> String {
    let s = d.as_secs();
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// What a problem is about, in a report's words: a port by its device's
/// name, anything else by its number, never a name someone typed.
pub fn about(subject: Subject, names: &Watch) -> String {
    if subject == Subject::NONE {
        return String::new();
    }
    if subject.midi_port_index().is_some() {
        if let Some(n) = names.subject_name(subject) {
            return format!(" ({n})");
        }
    }
    format!(" ({subject})")
}

/// One log line for the report's list: the time since the start, then the
/// line ("[0:12] status: + sound/dropout (1) The sound dropped out").
pub fn log_line(at: Duration, line: &str) -> String {
    format!("[{}] {line}", since(at))
}

fn level(l: Level) -> &'static str {
    match l {
        Level::Fail => "red",
        Level::Warn => "yellow",
        Level::Note => "note",
    }
}

/// `secs` after 1970 as "2026-11-23 11:40:00 UTC".
fn utc(secs: u64) -> String {
    let s = crate::screen_picture::stamp(secs);
    // "20261123-114000"
    format!(
        "{}-{}-{} {}:{}:{} UTC",
        &s[0..4],
        &s[4..6],
        &s[6..8],
        &s[9..11],
        &s[11..13],
        &s[13..15]
    )
}

/// The report's words.
pub fn text(f: &Facts, board: &Board, names: &Watch) -> String {
    let mut out = String::new();
    let at = |t: Instant| since(t.saturating_duration_since(f.start));
    let _ = writeln!(out, "MPFree report");
    let _ = writeln!(
        out,
        "Saved {} (by the MPC's clock), {} after MPFree started.",
        utc(f.saved),
        since(f.now.saturating_duration_since(f.start))
    );
    let _ = writeln!(
        out,
        "It holds no serial numbers, network addresses or names of projects, samples, tracks or takes."
    );

    let _ = writeln!(out, "\n== MPFree and the MPC");
    for (k, v) in f.system.software.iter().chain(&f.system.hardware) {
        if LEFT_OUT.contains(&k.as_str()) {
            continue;
        }
        let _ = writeln!(out, "{k}: {v}");
    }

    let _ = writeln!(out, "\n== This start");
    match &f.safe_start {
        None => {
            let _ = writeln!(out, "A normal start.");
        }
        Some(why) => {
            let _ = writeln!(
                out,
                "A safe start: the last run crashed ({}).",
                why.as_deref()
                    .unwrap_or("no panic line: a crash outside Rust, or out of memory")
            );
        }
    }

    let _ = writeln!(out, "\n== Now");
    let r = f.readings;
    let mut now = Vec::new();
    if let Some(p) = r.cpu_pct {
        now.push(format!("the sound's share of the CPU {p:.0} %"));
    }
    if let Some(p) = r.ram_pct {
        now.push(format!("RAM {p:.0} % used"));
    }
    if let Some(b) = r.free {
        now.push(format!("{} MB free", b / 1_000_000));
    }
    if let Some(s) = r.stick {
        now.push(format!(
            "the stick {} MB free of {} MB{}",
            s.free / 1_000_000,
            s.total / 1_000_000,
            if s.writable { "" } else { ", read only" }
        ));
    }
    if let Some(h) = r.heat {
        if let Some(c) = h.temp_c {
            now.push(match h.critical_c {
                Some(off) => format!("{c:.1} °C (it turns itself off at {off:.0} °C)"),
                None => format!("{c:.1} °C"),
            });
        }
        if let (Some(m), Some(top), Some(full)) = (h.clock_mhz, h.top_mhz, h.full_mhz) {
            now.push(format!(
                "processor {m} MHz, top {top} MHz of {full} MHz at the start"
            ));
        }
    }
    if now.is_empty() {
        now.push("nothing read yet".to_string());
    }
    for l in now {
        let _ = writeln!(out, "{l}");
    }

    let problems = board.raw();
    let _ = writeln!(out, "\n== Problems this session ({})", problems.len());
    if problems.is_empty() {
        let _ = writeln!(out, "None.");
    }
    for p in &problems {
        let state = match p.state {
            ProblemState::Now => match p.since {
                Some(s) => format!("going now, since {}", at(s)),
                None => "now".to_string(),
            },
            ProblemState::Ended => match p.ended {
                Some(e) => format!("ended at {}", at(e)),
                None => "ended".to_string(),
            },
            _ => "settling".to_string(),
        };
        let mut notes = Vec::new();
        if p.hidden {
            notes.push("its cause was going");
        }
        if !p.shown {
            notes.push("not shown yet");
        }
        let notes = if notes.is_empty() {
            String::new()
        } else {
            format!(" ({})", notes.join(", "))
        };
        let _ = writeln!(
            out,
            "{} {}{}: {}. {} time{} · first at {} · last at {} · {state}{notes}",
            level(p.kind.level),
            p.kind.id,
            about(p.subject, names),
            p.kind.title,
            p.times,
            if p.times == 1 { "" } else { "s" },
            at(p.first),
            at(p.last),
        );
    }

    let _ = writeln!(out, "\n== The last status lines ({})", f.lines.len());
    for l in f.lines {
        let _ = writeln!(out, "{l}");
    }
    out
}

/// The reports this run has named, to keep two in one second apart.
pub type Names = crate::screen_picture::Names;

/// Gives the report's words to the store; what to say on the hint line.
pub fn save(store: &Store, names: &mut Names, text: String) -> String {
    if !store.saves() {
        return "Report: no data folder to keep it in this run".into();
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = names.next_named("report", "txt", secs);
    match store.write(OWNER, &name, text.into_bytes()) {
        Ok(()) => format!("Report saved: files/{OWNER}/{name}"),
        Err(e) => format!("Report: not saved ({e})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status_watch::Look;
    use mpfree_status::{kinds, Report};

    fn facts<'a>(
        system: &'a SystemInfo,
        readings: &'a Readings,
        lines: &'a [String],
        start: Instant,
    ) -> Facts<'a> {
        Facts {
            // 2026-11-23 11:40:00 UTC.
            saved: 1_795_434_000,
            start,
            now: start + Duration::from_secs(754),
            system,
            safe_start: None,
            readings,
            lines,
        }
    }

    /// The report says what the page says, in plain text, and leaves out
    /// the host name and every name someone typed: a track by its number,
    /// a port by its device's name.
    #[test]
    fn a_report_holds_the_session_and_no_names_typed() {
        let start = Instant::now();
        let mut board = Board::new();
        board.apply(
            Report::happened(&kinds::SOUND_DROPOUT, 3),
            start + Duration::from_secs(12),
        );
        board.apply(
            Report::began(&kinds::RECORDING_AT_RISK).about(Subject::audio_track(3)),
            start + Duration::from_secs(60),
        );
        board.tick(start + Duration::from_secs(754));
        let mut names = Watch::default();
        names.set_audio_tracks([(3, "Dana's vocal")]);
        // Juno-X missing at two looks: port 1.
        let ports = ["Juno-X".to_string()];
        for _ in 0..2 {
            names.look(
                &Look {
                    missing_ports: Some(&ports),
                    ..Look::default()
                },
                &mut |r| board.apply(r, start + Duration::from_secs(70)),
            );
        }
        board.tick(start + Duration::from_secs(754));
        let system = SystemInfo {
            software: vec![
                ("MPFree".into(), "pad player 0.1.0, commit abc1234".into()),
                ("Host name".into(), "danas-mpc".into()),
            ],
            hardware: vec![("Board".into(), "Akai MPC One".into())],
        };
        let readings = Readings {
            cpu_pct: Some(24.0),
            ram_pct: Some(41.0),
            ..Readings::default()
        };
        let lines = vec![log_line(
            Duration::from_secs(12),
            "status: + sound/dropout (1) The sound dropped out",
        )];
        let t = text(&facts(&system, &readings, &lines, start), &board, &names);
        assert!(
            t.starts_with("MPFree report\nSaved 2026-11-23 11:40:00 UTC"),
            "{t}"
        );
        assert!(t.contains("12:34 after MPFree started"), "{t}");
        assert!(
            t.contains("MPFree: pad player 0.1.0, commit abc1234"),
            "{t}"
        );
        assert!(t.contains("Board: Akai MPC One"), "{t}");
        assert!(!t.contains("danas-mpc") && !t.contains("Host name"), "{t}");
        assert!(t.contains("A normal start."), "{t}");
        assert!(t.contains("the sound's share of the CPU 24 %"), "{t}");
        assert!(
            t.contains("red sound/dropout: The sound dropped out. 3 times · first at 0:12"),
            "{t}"
        );
        assert!(t.contains("(track 3)"), "{t}");
        assert!(!t.contains("Dana"), "a name typed: {t}");
        assert!(t.contains("midi/port-missing (Juno-X)"), "{t}");
        assert!(t.contains("[0:12] status: + sound/dropout"), "{t}");
        assert!(t.contains("== The last status lines (1)"), "{t}");
    }

    #[test]
    fn a_safe_start_says_the_crash_and_an_empty_session_says_so() {
        let start = Instant::now();
        let system = SystemInfo::default();
        let readings = Readings::default();
        let mut f = facts(&system, &readings, &[], start);
        f.safe_start = Some(Some("thread 'main' at src/gui.rs:1:2: boom".into()));
        let t = text(&f, &Board::new(), &Watch::default());
        assert!(
            t.contains(
                "A safe start: the last run crashed (thread 'main' at src/gui.rs:1:2: boom)."
            ),
            "{t}"
        );
        assert!(t.contains("== Problems this session (0)\nNone."), "{t}");
        assert!(t.contains("nothing read yet"), "{t}");
        f.safe_start = Some(None);
        let t = text(&f, &Board::new(), &Watch::default());
        assert!(t.contains("no panic line"), "{t}");
    }

    #[test]
    fn a_report_is_named_by_the_time_and_needs_a_data_folder() {
        let mut n = Names::default();
        assert_eq!(
            n.next_named("report", "txt", 1_795_434_000),
            "report-20261123-114000.txt"
        );
        assert_eq!(
            n.next_named("report", "txt", 1_795_434_000),
            "report-20261123-114000-2.txt"
        );
        assert_eq!(since(Duration::from_secs(3723)), "1:02:03");
        assert_eq!(
            save(&Store::none(), &mut n, "x".into()),
            "Report: no data folder to keep it in this run"
        );
        let dir = std::env::temp_dir().join(format!("mpfree-report-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::at(&dir, |_| {});
        let said = save(&store, &mut n, "MPFree report\n".into());
        assert!(
            said.starts_with("Report saved: files/reports/report-"),
            "{said}"
        );
        let name = said.rsplit('/').next().unwrap();
        store.flush();
        assert_eq!(
            store.read(OWNER, name).as_deref(),
            Some(&b"MPFree report\n"[..])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
