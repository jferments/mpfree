//! The kernel's stick and memory errors for System status (after phase 3,
//! Jay's pick of 23 Nov 2026 11:43Z, "Save a report, then kernel errors").
//! A thread reads the kernel's log (`/dev/kmsg`, read-only) from the start
//! of this boot and turns a short, reviewed list of its messages into
//! problems. Anything not on the list is ignored, so a harmless driver
//! message never colours the chip.
//!
//! The list, in Linux's own words (6.18's `block/blk-core.c`,
//! `fs/buffer.c`, `fs/ext4/super.c`, `fs/fat/misc.c`, `fs/exfat/misc.c`,
//! `mm/oom_kill.c`):
//!
//! | Message | Problem |
//! |---|---|
//! | `I/O error, dev sda, sector …` (any `… error, dev sdX, sector`) | The stick had a read or write error |
//! | `Buffer I/O error on dev sda4, logical block …` | the same |
//! | `EXT4-fs error (device sda4): …` | The stick's files have an error |
//! | `FAT-fs (sda1): error, …`, `exFAT-fs (sda1): error, …` | the same |
//! | `Out of memory: Killed process 1234 (mpfree…) …`, `Memory cgroup out of memory: Killed process …` | The MPC ran out of memory |
//!
//! The stick is any `sd` disk: the MPC One has one USB port, and the stick
//! in it is MPFree's. The eMMC and the SD card (`mmcblk`) are left out:
//! MPFree's boot switches both off. Only MPFree's own programs count for
//! memory (`mpfree`, `mpfree-plughost`, …), as the plan's list says.
//!
//! From the start of this boot, not of this run: when the kernel stopped
//! MPFree itself for memory, the run after it (a safe start) shows why.
//! `Buffer I/O error on dev mmcblk1p1, logical block 2` was seen on the MPC
//! (stick-autoboot.md, 15 Nov), so that format is known; the others are
//! from the kernel's source and wait for a device check.
//!
//! Errors come in bursts, so a second thread counts them and sends each
//! kind at most once a second, with the count (`Report::happened`).
//!
//! Without `/dev/kmsg` (the app's own user can't read it when the kernel
//! restricts it) the thread reads `journalctl -k -b -f` instead, as Dev >
//! Console follows the whole journal; without either it says so once.

use std::fs::File;
use std::io::{BufRead, BufReader, ErrorKind, Read};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use mpfree_status::{kinds, Kind, Report};

/// What a kernel message on the list means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Found {
    /// A read or write error on the stick.
    StickErrors,
    /// The stick's file system found a mistake.
    StickFiles,
    /// The kernel stopped one of MPFree's programs for memory.
    OutOfMemory,
}

impl Found {
    pub fn kind(self) -> &'static Kind {
        match self {
            Found::StickErrors => &kinds::STICK_ERRORS,
            Found::StickFiles => &kinds::STICK_FILES_ERROR,
            Found::OutOfMemory => &kinds::OUT_OF_MEMORY,
        }
    }
}

/// The device a message names after `after`, up to `,`, `)`, `:` or a
/// space.
fn device_after<'a>(line: &'a str, after: &str) -> Option<&'a str> {
    let rest = &line[line.find(after)? + after.len()..];
    let end = rest
        .find(|c: char| c == ',' || c == ')' || c == ':' || c.is_whitespace())
        .unwrap_or(rest.len());
    let dev = &rest[..end];
    (!dev.is_empty()).then_some(dev)
}

/// Whether a device ("sda", "sda4") is on the stick.
fn on_stick(dev: &str) -> bool {
    dev.starts_with("sd")
}

/// The program a kill message names: "Killed process 1234 (mpfree) …"
/// gives "mpfree".
fn killed(line: &str) -> Option<&str> {
    let rest = &line[line.find("Killed process ")? + "Killed process ".len()..];
    let open = rest.find('(')?;
    let close = rest[open..].find(')')? + open;
    Some(&rest[open + 1..close])
}

/// One kernel message (without its record header), if it is on the list.
pub fn classify(line: &str) -> Option<Found> {
    if line.starts_with("Out of memory") || line.contains("out of memory: Killed process") {
        return killed(line)
            .filter(|p| p.starts_with("mpfree"))
            .map(|_| Found::OutOfMemory);
    }
    let (dev, found) = if line.starts_with("Buffer I/O error on dev ") {
        (device_after(line, "on dev "), Found::StickErrors)
    } else if line.contains(" error, dev ") && line.contains(", sector ") {
        (device_after(line, " error, dev "), Found::StickErrors)
    } else if line.starts_with("EXT4-fs error (device ") {
        (device_after(line, "(device "), Found::StickFiles)
    } else if (line.starts_with("FAT-fs (") || line.starts_with("exFAT-fs ("))
        && line.contains("): error")
    {
        (device_after(line, "-fs ("), Found::StickFiles)
    } else {
        return None;
    };
    dev.filter(|d| on_stick(d)).map(|_| found)
}

/// The message of one `/dev/kmsg` line ("6,1234,5678901,-;text"); a
/// continuation line (it starts with a space) has none.
pub fn record_text(record: &str) -> Option<&str> {
    if record.starts_with(' ') {
        return None;
    }
    let (_, text) = record.split_once(';')?;
    Some(text.trim_end())
}

/// Counts what was found and says it at most once a second.
#[derive(Debug, Default)]
pub struct Tally {
    counts: Vec<(Found, u32)>,
    said: Option<Instant>,
}

impl Tally {
    pub fn add(&mut self, f: Found) {
        match self.counts.iter_mut().find(|(k, _)| *k == f) {
            Some((_, n)) => *n = n.saturating_add(1),
            None => self.counts.push((f, 1)),
        }
    }

    /// The reports due at `now`: everything counted, once a second has
    /// passed since the last.
    pub fn due(&mut self, now: Instant) -> Vec<Report> {
        if self.counts.is_empty()
            || self
                .said
                .is_some_and(|t| now.saturating_duration_since(t) < Duration::from_secs(1))
        {
            return Vec::new();
        }
        self.said = Some(now);
        self.counts
            .drain(..)
            .map(|(f, n)| Report::happened(f.kind(), n))
            .collect()
    }
}

/// Reads lines from the kernel's log until it ends, giving each found one
/// to `found`; `kmsg` says the lines have `/dev/kmsg`'s record header.
/// Lines are read as bytes (the kernel escapes most, a driver's name may
/// not be UTF-8). A `/dev/kmsg` read that lost lines to the ring's wrap
/// (EPIPE) goes on with the next.
pub fn read_lines(
    from: impl Read,
    kmsg: bool,
    mut found: impl FnMut(Found) -> bool,
) -> std::io::Result<()> {
    let mut r = BufReader::new(from);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match r.read_until(b'\n', &mut buf) {
            Ok(0) => return Ok(()),
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::BrokenPipe || e.kind() == ErrorKind::Interrupted => {
                continue
            }
            Err(e) => return Err(e),
        }
        let line = String::from_utf8_lossy(&buf);
        let text = if kmsg {
            record_text(&line)
        } else {
            Some(line.trim_end())
        };
        if let Some(f) = text.and_then(classify) {
            if !found(f) {
                return Ok(());
            }
        }
    }
}

/// Follows the kernel's log on its own threads, from the start of this
/// boot, sending each problem found with `send` (false: the screen is
/// gone). `log` takes the `status:` lines about the watch itself.
pub fn follow(
    log: impl Fn(String) + Send + 'static,
    send: impl Fn(Report) -> bool + Send + 'static,
) {
    let (tx, rx) = mpsc::channel::<Found>();
    let reader = std::thread::Builder::new()
        .name("mpfree-kernel".into())
        .spawn(move || {
            let mut said = |f| tx.send(f).is_ok();
            match File::open("/dev/kmsg") {
                Ok(f) => {
                    log("status: watching the kernel's log for stick and memory errors".into());
                    if let Err(e) = read_lines(f, true, &mut said) {
                        log(format!("status: the kernel's log stopped ({e})"));
                    }
                }
                Err(kmsg) => {
                    let child = std::process::Command::new("journalctl")
                        .args(["-k", "-b", "-f", "-n", "all", "--no-pager", "-o", "cat"])
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::null())
                        .spawn();
                    match child {
                        Ok(mut c) => {
                            log(format!(
                                "status: watching the kernel's log through journalctl \
                                 (/dev/kmsg: {kmsg})"
                            ));
                            if let Some(out) = c.stdout.take() {
                                let _ = read_lines(out, false, &mut said);
                            }
                            let _ = c.kill();
                            let _ = c.wait();
                        }
                        Err(e) => log(format!(
                            "status: the kernel's log can't be read (/dev/kmsg: {kmsg}; \
                             journalctl: {e}), so its stick and memory errors aren't watched"
                        )),
                    }
                }
            }
        });
    if let Err(e) = reader {
        eprintln!("status: the kernel's log watcher didn't start: {e}");
        return;
    }
    // The counting side: a burst is sent at once, then once a second while
    // it goes on, and what is left a second after it ends.
    let counter = std::thread::Builder::new()
        .name("mpfree-kernel-count".into())
        .spawn(move || {
            let mut tally = Tally::default();
            loop {
                let gone = match rx.recv_timeout(Duration::from_secs(1)) {
                    Ok(f) => {
                        tally.add(f);
                        false
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => false,
                    Err(mpsc::RecvTimeoutError::Disconnected) => true,
                };
                for r in tally.due(Instant::now()) {
                    if !send(r) {
                        return;
                    }
                }
                if gone {
                    return;
                }
            }
        });
    if let Err(e) = counter {
        eprintln!("status: the kernel's log counter didn't start: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_finds_stick_and_memory_errors_and_nothing_else() {
        for (line, want) in [
            // The block layer (6.18's blk_print_req_error).
            (
                "I/O error, dev sda, sector 2048 op 0x1:(WRITE) flags 0x800 phys_seg 1 prio class 2",
                Some(Found::StickErrors),
            ),
            (
                "critical medium error, dev sda, sector 77 op 0x0:(READ) flags 0x0 phys_seg 1 prio class 0",
                Some(Found::StickErrors),
            ),
            (
                "Buffer I/O error on dev sda4, logical block 1234, lost async page write",
                Some(Found::StickErrors),
            ),
            (
                "EXT4-fs error (device sda4): ext4_find_entry:1623: inode #2: comm mpfree: reading directory lblock 0",
                Some(Found::StickFiles),
            ),
            (
                "FAT-fs (sda1): error, fat_get_cluster: invalid cluster chain (i_pos 0)",
                Some(Found::StickFiles),
            ),
            (
                "exFAT-fs (sdb1): error, invalid access to FAT (entry 0x00000000)",
                Some(Found::StickFiles),
            ),
            (
                "Out of memory: Killed process 1234 (mpfree-plughost) total-vm:812345kB, anon-rss:400000kB, file-rss:0kB, shmem-rss:0kB, UID:960 pgtables:400kB oom_score_adj:0",
                Some(Found::OutOfMemory),
            ),
            (
                "Memory cgroup out of memory: Killed process 77 (mpfree) total-vm:1kB, anon-rss:1kB",
                Some(Found::OutOfMemory),
            ),
            // Another program stopped for memory: not MPFree's.
            ("Out of memory: Killed process 88 (acvs) total-vm:1kB, anon-rss:1kB", None),
            // Not on the list.
            ("oom-kill:constraint=CONSTRAINT_NONE,nodemask=(null),task=mpfree,pid=77", None),
            ("FAT-fs (sda1): Volume was not properly unmounted. Some data may be corrupt.", None),
            ("EXT4-fs (sda4): mounted filesystem with ordered data mode.", None),
            ("usb 1-1: reset high-speed USB device number 2 using ehci-platform", None),
            // The eMMC and the SD card: left out.
            ("Buffer I/O error on dev mmcblk1p1, logical block 2, async page read", None),
            ("I/O error, dev mmcblk0, sector 0 op 0x0:(READ) flags 0x0 phys_seg 1 prio class 0", None),
            ("EXT4-fs error (device mmcblk0p7): ext4_lookup:1: comm x: deleted inode", None),
        ] {
            assert_eq!(classify(line), want, "{line}");
        }
    }

    #[test]
    fn a_record_has_its_text_after_the_header() {
        assert_eq!(
            record_text("3,1234,5678901,-;Buffer I/O error on dev sda4, logical block 9\n"),
            Some("Buffer I/O error on dev sda4, logical block 9")
        );
        assert_eq!(record_text(" SUBSYSTEM=block"), None);
        assert_eq!(record_text("no header"), None);
        assert_eq!(
            killed("Out of memory: Killed process 1 (a b) x"),
            Some("a b")
        );
    }

    #[test]
    fn a_log_is_read_line_by_line_with_or_without_headers() {
        let kmsg: &[u8] = b"6,1,10,-;usb 1-1: new device\n\
                    3,2,20,-;Buffer I/O error on dev sda4, logical block 9, async page read\n \
                    SUBSYSTEM=block\n\
                    3,3,30,-;EXT4-fs error (device sda4): ext4_lookup: \xff\n";
        let mut got = Vec::new();
        read_lines(kmsg, true, |f| {
            got.push(f);
            true
        })
        .unwrap();
        assert_eq!(got, [Found::StickErrors, Found::StickFiles]);
        let journal = "Out of memory: Killed process 9 (mpfree) total-vm:1kB\nnothing\n";
        let mut got = Vec::new();
        read_lines(journal.as_bytes(), false, |f| {
            got.push(f);
            true
        })
        .unwrap();
        assert_eq!(got, [Found::OutOfMemory]);
        // A gone screen ends the read.
        let mut n = 0;
        read_lines(kmsg, true, |_| {
            n += 1;
            false
        })
        .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn a_burst_is_said_once_a_second_with_its_count() {
        let t = Instant::now();
        let mut tally = Tally::default();
        for _ in 0..40 {
            tally.add(Found::StickErrors);
        }
        tally.add(Found::OutOfMemory);
        let first = tally.due(t);
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].kind, &kinds::STICK_ERRORS);
        assert!(matches!(
            first[0].op,
            mpfree_status::Op::Happened { times: 40 }
        ));
        tally.add(Found::StickErrors);
        assert!(
            tally.due(t + Duration::from_millis(500)).is_empty(),
            "within the second"
        );
        let later = tally.due(t + Duration::from_millis(1000));
        assert!(matches!(
            later[0].op,
            mpfree_status::Op::Happened { times: 1 }
        ));
        assert!(
            tally.due(t + Duration::from_secs(5)).is_empty(),
            "nothing new"
        );
    }
}
