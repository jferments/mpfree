//! The operating system's part: sleeping and waking on a word, pinning a
//! thread to a core, and SCHED_FIFO. Linux first (futex, sched_setaffinity,
//! sched_setscheduler); on other systems a waiter polls with short sleeps,
//! and pinning and real-time priority report that they are not supported
//! yet, so a lane still works everywhere.
//!
//! [`background`] is for a program's helper threads: the normal policy,
//! and the cores [`set_background`] named once the sound has cores of its
//! own.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

#[cfg(target_os = "linux")]
mod imp {
    #![allow(unsafe_code)]

    use super::*;
    use std::io;

    /// Sleeps while `word` holds `expected`, for at most `timeout` (None:
    /// until woken). Returns at once if the word already differs, and may
    /// return early (a signal, a spurious wake): callers check the word
    /// again.
    pub fn wait(word: &AtomicU32, expected: u32, timeout: Option<Duration>) {
        let ts;
        let tsp = match timeout {
            Some(t) => {
                ts = libc::timespec {
                    tv_sec: t.as_secs().min(i32::MAX as u64) as libc::time_t,
                    tv_nsec: t.subsec_nanos() as libc::c_long,
                };
                &ts as *const libc::timespec
            }
            None => std::ptr::null(),
        };
        // SAFETY: a valid futex word that outlives the call (it lives in an
        // Arc both sides hold), a valid or null relative timeout. PRIVATE:
        // both threads are in this process.
        unsafe {
            libc::syscall(
                libc::SYS_futex,
                word.as_ptr(),
                libc::FUTEX_WAIT | libc::FUTEX_PRIVATE_FLAG,
                expected,
                tsp,
                std::ptr::null::<u32>(),
                0,
            );
        }
    }

    /// Wakes every thread sleeping in [`wait`] on `word`.
    pub fn wake(word: &AtomicU32) {
        // SAFETY: as `wait`.
        unsafe {
            libc::syscall(
                libc::SYS_futex,
                word.as_ptr(),
                libc::FUTEX_WAKE | libc::FUTEX_PRIVATE_FLAG,
                i32::MAX,
                std::ptr::null::<libc::timespec>(),
                std::ptr::null::<u32>(),
                0,
            );
        }
    }

    /// Lets the calling thread run only on `core`.
    pub fn pin(core: usize) -> Result<(), String> {
        if core >= libc::CPU_SETSIZE as usize {
            return Err(format!("core {core} is past the CPU set's size"));
        }
        // SAFETY: a zeroed cpu_set_t is the empty set; CPU_SET writes inside
        // it (core is checked above); pid 0 is the calling thread.
        unsafe {
            let mut set: libc::cpu_set_t = std::mem::zeroed();
            libc::CPU_SET(core, &mut set);
            if libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set) != 0 {
                return Err(io::Error::last_os_error().to_string());
            }
        }
        Ok(())
    }

    /// Lets the calling thread run only on `cores` (at least one).
    pub fn pin_set(cores: &[usize]) -> Result<(), String> {
        if cores.is_empty() {
            return Err("no core given".into());
        }
        if let Some(c) = cores.iter().find(|&&c| c >= libc::CPU_SETSIZE as usize) {
            return Err(format!("core {c} is past the CPU set's size"));
        }
        // SAFETY: as in `pin`; every core is checked above.
        unsafe {
            let mut set: libc::cpu_set_t = std::mem::zeroed();
            for &c in cores {
                libc::CPU_SET(c, &mut set);
            }
            if libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set) != 0 {
                return Err(io::Error::last_os_error().to_string());
            }
        }
        Ok(())
    }

    /// The normal policy (SCHED_OTHER) for the calling thread: what a
    /// helper thread asks for if it may have inherited SCHED_FIFO. Lowering
    /// the policy needs no privilege.
    pub fn normal() -> Result<(), String> {
        let p = libc::sched_param { sched_priority: 0 };
        // SAFETY: p lives across the call; pid 0 is the calling thread.
        if unsafe { libc::sched_setscheduler(0, libc::SCHED_OTHER, &p) } != 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        Ok(())
    }

    /// SCHED_FIFO at `priority` for the calling thread.
    pub fn fifo(priority: i32) -> Result<(), String> {
        let p = libc::sched_param {
            sched_priority: priority,
        };
        // SAFETY: p lives across the call; pid 0 is the calling thread.
        if unsafe { libc::sched_setscheduler(0, libc::SCHED_FIFO, &p) } != 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        Ok(())
    }

    /// The cores the calling thread may run on.
    pub fn allowed_cores() -> Vec<usize> {
        // SAFETY: a zeroed set filled by the kernel; CPU_ISSET reads inside it.
        unsafe {
            let mut set: libc::cpu_set_t = std::mem::zeroed();
            if libc::sched_getaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &mut set) != 0 {
                return Vec::new();
            }
            (0..libc::CPU_SETSIZE as usize)
                .filter(|&c| libc::CPU_ISSET(c, &set))
                .collect()
        }
    }

    /// The core the calling thread is on right now.
    pub fn current_core() -> Option<usize> {
        // SAFETY: no arguments.
        let c = unsafe { libc::sched_getcpu() };
        (c >= 0).then_some(c as usize)
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::*;
    use std::sync::atomic::Ordering;

    /// Polls: sleeps a little while the word holds `expected`. Callers check
    /// the word again, so a late wake only costs time.
    pub fn wait(word: &AtomicU32, expected: u32, timeout: Option<Duration>) {
        if word.load(Ordering::Acquire) == expected {
            let step = Duration::from_micros(100);
            std::thread::sleep(timeout.map_or(step, |t| t.min(step)));
        }
    }

    /// Nothing to do: waiters poll.
    pub fn wake(_word: &AtomicU32) {}

    pub fn pin_set(_cores: &[usize]) -> Result<(), String> {
        Err("pinning is not supported on this system yet".into())
    }

    pub fn normal() -> Result<(), String> {
        Ok(())
    }

    pub fn pin(_core: usize) -> Result<(), String> {
        Err("pinning to a core is not supported on this system yet".into())
    }

    pub fn fifo(_priority: i32) -> Result<(), String> {
        Err("SCHED_FIFO is not supported on this system yet".into())
    }

    pub fn allowed_cores() -> Vec<usize> {
        (0..std::thread::available_parallelism().map_or(1, |n| n.get())).collect()
    }

    pub fn current_core() -> Option<usize> {
        None
    }
}

pub use imp::*;

/// [`set_background`]'s cores as a bit mask; 0: none named.
static BACKGROUND: AtomicU32 = AtomicU32::new(0);

/// Names the cores [`background`] puts helper threads on (cores 0 to 31;
/// others are left out). Empty: none, helper threads stay where they start.
pub fn set_background(cores: &[usize]) {
    let mask = cores
        .iter()
        .filter(|&&c| c < 32)
        .fold(0u32, |m, &c| m | 1 << c);
    BACKGROUND.store(mask, Ordering::Relaxed);
}

/// The cores [`set_background`] named (empty if none).
pub fn background_cores() -> Vec<usize> {
    let mask = BACKGROUND.load(Ordering::Relaxed);
    (0..32).filter(|c| mask & (1 << c) != 0).collect()
}

/// Puts the calling helper thread at the normal policy (SCHED_OTHER) and,
/// if [`set_background`] named cores, on those. A thread a program starts
/// for other work (a writer, a loader, a scanner) calls it first: a thread
/// begins with the cores of the thread that started it, so one started by
/// the audio thread would run on the audio thread's core and keep it from
/// idling. Changes only the calling thread, in RAM. Allocates nothing.
pub fn background() -> Result<(), String> {
    normal()?;
    let mask = BACKGROUND.load(Ordering::Relaxed);
    if mask == 0 {
        return Ok(());
    }
    let mut cores = [0usize; 32];
    let mut n = 0;
    for c in 0..32 {
        if mask & (1 << c) != 0 {
            cores[n] = c;
            n += 1;
        }
    }
    pin_set(&cores[..n])
}
