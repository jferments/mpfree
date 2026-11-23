//! The disk's priority for one thread (P5's first knob, in D4's steps 9
//! and 10): Linux's `ioprio_set` on that thread alone.
//!
//! D2b found that reads wait behind writes on the stick (1.65 s beside a
//! recording). The MPC's stick queue uses the `bfq` scheduler, which
//! serves a thread's disk requests by its priority: the real-time class
//! before the best-effort one, and within best effort by level, 0 first, 7
//! last (a weight of 80 against 10). Whether that helps on a stick that
//! takes one request at a time is what the steps measure.
//!
//! A priority belongs to a thread and goes when the thread ends; the steps
//! set it only on threads they start and end themselves, so nothing
//! outside the program changes and there is nothing to put back. The
//! kernel's own writeback threads keep theirs.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

/// `IOPRIO_WHO_PROCESS`: one thread, by its id (0: the caller).
const WHO_THREAD: libc::c_int = 1;
/// The class sits above 13 bits of level (`IOPRIO_CLASS_SHIFT`).
const CLASS_SHIFT: u32 = 13;

/// A disk priority: a class and a level within it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IoPrio {
    /// None set: the kernel derives best effort from the CPU nice level.
    None,
    RealTime(u8),
    BestEffort(u8),
    Idle,
}

impl IoPrio {
    pub(super) fn value(self) -> libc::c_int {
        let (class, level) = match self {
            IoPrio::None => (0, 0),
            IoPrio::RealTime(l) => (1, l),
            IoPrio::BestEffort(l) => (2, l),
            IoPrio::Idle => (3, 0),
        };
        (class << CLASS_SHIFT) | libc::c_int::from(level)
    }

    pub(super) fn from_value(v: libc::c_int) -> Option<IoPrio> {
        let level = u8::try_from(v & ((1 << CLASS_SHIFT) - 1)).ok()?;
        match v >> CLASS_SHIFT {
            0 => Some(IoPrio::None),
            1 if level < 8 => Some(IoPrio::RealTime(level)),
            2 if level < 8 => Some(IoPrio::BestEffort(level)),
            3 => Some(IoPrio::Idle),
            _ => None,
        }
    }

    pub(super) fn name(self) -> String {
        match self {
            IoPrio::None => "none set (best effort from the nice level)".into(),
            IoPrio::RealTime(l) => format!("real-time {l}"),
            IoPrio::BestEffort(l) => format!("best effort {l}"),
            IoPrio::Idle => "idle".into(),
        }
    }
}

/// Set thread `tid`'s disk priority (0: the calling thread) and read it
/// back.
pub(super) fn set(tid: libc::pid_t, p: IoPrio) -> Result<(), String> {
    // SAFETY: ioprio_set takes three integers and changes only the disk
    // priority of the named thread, which is one of this program's.
    let r = unsafe {
        libc::syscall(
            libc::SYS_ioprio_set,
            WHO_THREAD,
            libc::c_int::from(tid),
            p.value(),
        )
    };
    if r != 0 {
        return Err(format!(
            "the disk priority {} could not be set: {}",
            p.name(),
            std::io::Error::last_os_error()
        ));
    }
    match get(tid)? {
        got if got == p => Ok(()),
        got => Err(format!(
            "the disk priority was set to {} but reads {}",
            p.name(),
            got.name()
        )),
    }
}

/// Thread `tid`'s disk priority (0: the calling thread).
pub(super) fn get(tid: libc::pid_t) -> Result<IoPrio, String> {
    // SAFETY: ioprio_get takes two integers and only reads.
    let r = unsafe { libc::syscall(libc::SYS_ioprio_get, WHO_THREAD, libc::c_int::from(tid)) };
    if r < 0 {
        return Err(format!(
            "the disk priority could not be read: {}",
            std::io::Error::last_os_error()
        ));
    }
    let v = libc::c_int::try_from(r).map_err(|_| format!("a disk priority of {r}"))?;
    IoPrio::from_value(v).ok_or_else(|| format!("a disk priority of {v}"))
}

/// Whether these priorities can be set here, tried on a thread of its own
/// that ends straight after, so nothing keeps them.
pub(super) fn can_set(wanted: &[IoPrio]) -> Result<(), String> {
    let wanted = wanted.to_vec();
    std::thread::Builder::new()
        .name("disktest-prio".into())
        .spawn(move || wanted.into_iter().try_for_each(|p| set(0, p)))
        .map_err(|e| format!("a thread could not start: {e}"))?
        .join()
        .unwrap_or_else(|_| Err("the trial thread failed".into()))
}

/// The id of this program's thread called `name`, waiting up to `wait` for
/// it to appear (a new thread names itself as it starts). The name is as
/// `/proc/self/task/ID/comm` has it, at most 15 bytes.
pub(super) fn thread_named(name: &str, wait: Duration) -> Option<libc::pid_t> {
    let t = Instant::now();
    loop {
        if let Ok(tasks) = fs::read_dir("/proc/self/task") {
            for e in tasks.flatten() {
                let comm = fs::read_to_string(e.path().join("comm")).unwrap_or_default();
                if comm.trim_end_matches('\n') == name {
                    if let Some(id) = e.file_name().to_str().and_then(|s| s.parse().ok()) {
                        return Some(id);
                    }
                }
            }
        }
        if t.elapsed() >= wait {
            return None;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// The scheduler a disk's queue uses (the one in brackets in
/// `DISK/queue/scheduler`), read-only.
pub(super) fn scheduler(disk: &Path) -> Result<String, String> {
    let p = disk.join("queue/scheduler");
    let text = fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    parse_scheduler(&text).ok_or_else(|| format!("{} reads {:?}", p.display(), text.trim()))
}

pub(super) fn parse_scheduler(text: &str) -> Option<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if let Some(w) = words
        .iter()
        .find(|w| w.starts_with('[') && w.ends_with(']'))
    {
        return Some(w[1..w.len() - 1].to_string());
    }
    // A queue with only one choice may print it bare.
    match words.as_slice() {
        [one] => Some(one.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priorities_are_coded_as_the_kernel_codes_them() {
        // include/uapi/linux/ioprio.h: IOPRIO_PRIO_VALUE(class, level).
        assert_eq!(IoPrio::None.value(), 0);
        assert_eq!(IoPrio::RealTime(4).value(), (1 << 13) | 4);
        assert_eq!(IoPrio::BestEffort(0).value(), 2 << 13);
        assert_eq!(IoPrio::BestEffort(7).value(), (2 << 13) | 7);
        assert_eq!(IoPrio::Idle.value(), 3 << 13);
        for p in [
            IoPrio::None,
            IoPrio::RealTime(0),
            IoPrio::RealTime(7),
            IoPrio::BestEffort(4),
            IoPrio::Idle,
        ] {
            assert_eq!(IoPrio::from_value(p.value()), Some(p));
        }
        assert_eq!(IoPrio::from_value((2 << 13) | 8), None);
        assert_eq!(IoPrio::from_value(4 << 13), None);
    }

    #[test]
    fn the_scheduler_is_the_one_in_brackets() {
        assert_eq!(
            parse_scheduler("mq-deadline kyber [bfq] none\n").as_deref(),
            Some("bfq")
        );
        assert_eq!(
            parse_scheduler("[none] mq-deadline\n").as_deref(),
            Some("none")
        );
        assert_eq!(parse_scheduler("none\n").as_deref(), Some("none"));
        assert_eq!(parse_scheduler("mq-deadline none\n"), None);
        assert_eq!(parse_scheduler(""), None);
    }

    #[test]
    fn a_thread_is_found_by_its_name_and_its_priority_goes_with_it() {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let h = std::thread::Builder::new()
            .name("prio-test-a".into())
            .spawn(move || {
                let _ = rx.recv();
            })
            .unwrap();
        let id = thread_named("prio-test-a", Duration::from_secs(2)).expect("found");
        assert!(id > 0);
        // A lower priority needs no privilege; read back as set.
        set(id, IoPrio::BestEffort(7)).unwrap();
        assert_eq!(get(id).unwrap(), IoPrio::BestEffort(7));
        tx.send(()).unwrap();
        h.join().unwrap();
        assert_eq!(thread_named("prio-test-a", Duration::ZERO), None);
        assert_eq!(
            thread_named("no-such-thread", Duration::from_millis(5)),
            None
        );
    }

    #[test]
    fn a_trial_leaves_the_calling_thread_as_it_was() {
        let before = get(0).unwrap();
        assert!(can_set(&[IoPrio::BestEffort(7)]).is_ok());
        assert_eq!(get(0).unwrap(), before);
    }
}
