//! The Monitor's lines as text: the file Save log has the app's store keep
//! in the data folder's `files/midi-logs/` ([`saved`], [`file_name`]), and
//! the lines the app writes
//! to its log in RAM for the workstation console ([`console_line`],
//! `tools/mpc-one/midi_console.py`).
//!
//! Both say what the screen says: when, the port and way, the channel, the
//! message in words, where it went, and its bytes. A SysEx is its length,
//! its maker and its first bytes, as listed; clock, active sensing and
//! time code are counted, not listed (a line when each starts and stops).

use mpfree_midi::monitor::{Dir, Kind, Seen};

use crate::monitor::{bytes_text, clock_text, line_words, port_text, way, went_to};
use crate::IoModel;

/// One line's fields, as the screen words them.
#[derive(Clone, Debug, PartialEq)]
pub struct LogLine {
    /// The message's number since the start (a gap means lines were not
    /// kept).
    pub n: u64,
    /// When, on the app's clock (seconds).
    pub t: f64,
    /// The port's name ("MIDI" for the 5-pin jacks, "MPC", "System").
    pub port: String,
    pub dir: Dir,
    /// 1 to 16, or none.
    pub channel: Option<u8>,
    /// The console's name for its kind ([`kind_name`]).
    pub kind: &'static str,
    pub words: String,
    pub went: String,
    pub bytes: String,
}

/// The console's name for a kind of line (its `--notes`, `--cc` and so on).
pub fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Notes => "notes",
        Kind::Controls => "cc",
        Kind::Programs => "program",
        Kind::Bends => "bend",
        Kind::Clock => "clock",
        Kind::System => "sysex",
        Kind::MpcControls => "mpc",
        Kind::Lights => "lights",
        Kind::KeepAlive => "keepalive",
        Kind::Cv => "cv",
        Kind::Events => "system",
    }
}

/// Line `n` of `m`'s monitor in words.
pub fn line(m: &IoModel, n: u64, s: &Seen) -> LogLine {
    let mon = &m.monitor;
    let port = port_text(mon, s.port, s.dir);
    // The port's name alone: the way has a field of its own.
    let suffix = format!(" {}", way(s.dir));
    let port = port.strip_suffix(&suffix).unwrap_or(&port).to_string();
    LogLine {
        n,
        t: s.t,
        port,
        dir: s.dir,
        channel: s.what.channel().map(|c| c + 1),
        kind: kind_name(s.what.kind()),
        words: line_words(mon, s),
        went: went_to(m, s.to).0,
        bytes: bytes_text(&s.what),
    }
}

/// Tabs and line ends in a field become spaces, so each line keeps its
/// fields.
fn field(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

/// A line for the app's log in RAM, which the workstation console reads:
/// tab-separated `n`, seconds, port, `in` or `out`, channel (or `-`),
/// kind, words, where it went (or `-`), bytes.
pub fn console_line(l: &LogLine) -> String {
    let dash = |s: &str| {
        if s.is_empty() {
            "-".to_string()
        } else {
            field(s)
        }
    };
    format!(
        "{}\t{:.3}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        l.n,
        l.t,
        field(&l.port),
        way(l.dir),
        l.channel.map_or("-".to_string(), |c| c.to_string()),
        l.kind,
        dash(&l.words),
        dash(&l.went),
        dash(&l.bytes),
    )
}

/// The lines of `m`'s monitor numbered `from` and on, oldest first, for
/// the console, with the number to start from next time. Lines no longer
/// kept are skipped (the numbers say how many).
pub fn console_lines(m: &IoModel, from: u64) -> (Vec<String>, u64) {
    let mut v: Vec<String> = m
        .monitor
        .newest()
        .take_while(|&(n, _)| n >= from)
        .map(|(n, s)| console_line(&line(m, n, s)))
        .collect();
    v.reverse();
    (v, m.monitor.total().max(from))
}

/// The text Save log writes: a heading, then `lines` oldest first in
/// columns. `when` is the date and time it was saved ("2026-11-16 19:58
/// UTC"), `shown` what the list showed ("Everything").
pub fn saved(m: &IoModel, lines: &[(u64, Seen)], when: &str, shown: &str) -> String {
    let mut rows: Vec<[String; 6]> = vec![[
        "Time".into(),
        "Port".into(),
        "Ch".into(),
        "Message".into(),
        "Went to".into(),
        "Bytes".into(),
    ]];
    for (n, s) in lines.iter().rev() {
        let l = line(m, *n, s);
        rows.push([
            clock_text(l.t),
            format!("{} {}", l.port, way(l.dir)),
            l.channel.map_or("-".to_string(), |c| c.to_string()),
            l.words,
            l.went,
            l.bytes,
        ]);
    }
    let mut w = [0usize; 6];
    for r in &rows {
        for (k, f) in r.iter().enumerate() {
            w[k] = w[k].max(f.chars().count());
        }
    }
    let mut out = format!(
        "MPFree MIDI log, saved {when}\n\
         Showing: {shown}. {} lines, oldest first. Times are the app's clock since it started.\n",
        lines.len()
    );
    if m.monitor.lost > 0 {
        out.push_str(&format!(
            "{} messages came too many at once and were not listed.\n",
            m.monitor.lost
        ));
    }
    out.push('\n');
    for r in &rows {
        let mut row = String::new();
        for (k, f) in r.iter().enumerate() {
            if k + 1 < r.len() {
                row.push_str(f);
                row.push_str(&" ".repeat(w[k] - f.chars().count() + 2));
            } else {
                row.push_str(f);
            }
        }
        out.push_str(row.trim_end());
        out.push('\n');
    }
    out
}

/// `secs` since 1970 as the date and time a log is named and headed with:
/// ("2026-11-16-1958", "2026-11-16 19:58 UTC").
pub fn stamp(secs: u64) -> (String, String) {
    let (y, mo, d, h, mi) = mpfree_library::project::civil(secs);
    (
        format!("{y:04}-{mo:02}-{d:02}-{h:02}{mi:02}"),
        format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02} UTC"),
    )
}

/// The store's folder for saved logs: `files/midi-logs/` in the data
/// folder.
pub const OWNER: &str = "midi-logs";

/// The name for a log saved at `stamp` that no file in `taken` has:
/// `midi-log-<stamp>.txt`, or `-2`, `-3` and on, so a save never replaces
/// a file. None past 99 in one minute.
pub fn file_name(taken: &[String], stamp: &str) -> Option<String> {
    (1..100)
        .map(|k| {
            if k == 1 {
                format!("midi-log-{stamp}.txt")
            } else {
                format!("midi-log-{stamp}-{k}.txt")
            }
        })
        .find(|n| !taken.contains(n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample;

    #[test]
    fn the_console_gets_each_line_once_by_its_fields() {
        let m = sample::monitored(sample::rig());
        let total = m.monitor.total();
        let (all, next) = console_lines(&m, 0);
        assert_eq!(next, total);
        assert_eq!(all.len(), m.monitor.len());
        // Each has nine fields.
        assert!(all.iter().all(|l| l.split('\t').count() == 9), "{all:?}");
        let pad = all
            .iter()
            .find(|l| l.contains("Drums, pad 1"))
            .expect("the drum machine's note");
        let f: Vec<&str> = pad.split('\t').collect();
        assert_eq!(&f[2..6], ["MIDI", "in", "10", "notes"]);
        assert_eq!(f[8], "99 25 64");
        // From the last number on: nothing new.
        assert!(console_lines(&m, next).0.is_empty());
        // The newest two only.
        let (two, _) = console_lines(&m, total - 2);
        assert_eq!(two.len(), 2);
        assert_eq!(two[1], *all.last().unwrap());
    }

    #[test]
    fn a_saved_log_is_the_list_oldest_first_in_columns() {
        let m = sample::monitored(sample::rig());
        let lines: Vec<(u64, Seen)> = m.monitor.newest().map(|(n, s)| (n, *s)).collect();
        let text = saved(&m, &lines, "2026-11-16 19:58 UTC", "Everything");
        let rows: Vec<&str> = text.lines().collect();
        assert_eq!(rows[0], "MPFree MIDI log, saved 2026-11-16 19:58 UTC");
        let head = rows.iter().position(|r| r.starts_with("Time")).unwrap();
        assert_eq!(rows.len() - head - 1, lines.len());
        // Oldest first: the clock arriving before the keyboard's note-off.
        let first = rows[head + 1];
        assert!(first.contains("MIDI in"), "{first}");
        let last = rows.last().unwrap();
        assert!(
            last.contains("Keystep 37 in") && last.contains("Note off"),
            "{last}"
        );
        // Columns line up: Went to starts at the same place on each row
        // (counted in characters: "·" is more than a byte).
        let col = |r: &str, what: &str| r.find(what).map(|i| r[..i].chars().count());
        let at = col(rows[head], "Went to");
        let learn = rows.iter().find(|r| r.contains("Learn: Cutoff")).unwrap();
        assert_eq!(col(learn, "Learn: Cutoff"), at);
    }

    #[test]
    fn a_log_gets_a_name_no_file_has() {
        let (stamp, when) = stamp(1_794_859_080);
        assert_eq!(when, "2026-11-16 19:58 UTC");
        let mut taken = Vec::new();
        let a = file_name(&taken, &stamp).unwrap();
        assert_eq!(a, "midi-log-2026-11-16-1958.txt");
        taken.push(a);
        let b = file_name(&taken, &stamp).unwrap();
        assert_eq!(b, "midi-log-2026-11-16-1958-2.txt");
        assert!(mpfree_store_name_ok(&b));
        taken.extend((2..100).map(|k| format!("midi-log-{stamp}-{k}.txt")));
        assert_eq!(file_name(&taken, &stamp), None);
    }

    /// The store's rule for a file's name (mpfree-store's `valid_name`):
    /// letters, digits, `-`, `_` and `.`, not starting with a dot.
    fn mpfree_store_name_ok(n: &str) -> bool {
        !n.starts_with('.')
            && n.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    }
}
