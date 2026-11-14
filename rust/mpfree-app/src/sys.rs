//! The few C library calls this program needs that std does not offer:
//! ioctl (ALSA), fcntl (clear O_NONBLOCK), sched_setscheduler, mlockall,
//! and a stop handler (signal, unlink, raise) for the safe start
//! ([`remove_on_stop`], crash.rs).
//!
//! Every ioctl request number and structure size here is for the MPC One's
//! 32-bit ARM (armv7, EABI5 hard-float). They were checked against the
//! kernel's <sound/asound.h> compiled for that target (see the tests in
//! alsa.rs). On any other pointer width the device calls refuse to run, so
//! the host build can still be tested.

#![allow(unsafe_code)]

use std::ffi::CString;
use std::io;
use std::os::raw::{c_char, c_int, c_ulong};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::RawFd;
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};

pub const IS_DEVICE_ABI: bool = cfg!(target_pointer_width = "32");

pub const O_NONBLOCK: i32 = 0o4000;
const F_GETFL: c_int = 3;
const F_SETFL: c_int = 4;
const SCHED_FIFO: c_int = 1;
/// OR'd into the policy: threads this thread starts later begin at the
/// normal policy and priority instead of inheriting SCHED_FIFO
/// (`sched_setscheduler(2)`, Linux 2.6.32 and later).
const SCHED_RESET_ON_FORK: c_int = 0x4000_0000;
const MCL_CURRENT: c_int = 1;
const MCL_FUTURE: c_int = 2;

#[repr(C)]
struct SchedParam {
    sched_priority: c_int,
}

extern "C" {
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    fn sched_setscheduler(pid: c_int, policy: c_int, param: *const SchedParam) -> c_int;
    fn mlockall(flags: c_int) -> c_int;
    // A handler is a pointer-sized value: SIG_DFL (0), SIG_ERR (-1) or a
    // function's address.
    fn signal(sig: c_int, handler: usize) -> usize;
    fn raise(sig: c_int) -> c_int;
    fn unlink(path: *const c_char) -> c_int;
}

// The same numbers on 32-bit ARM and x86-64 Linux (signal(7)).
const SIGHUP: c_int = 1;
const SIGINT: c_int = 2;
const SIGTERM: c_int = 15;
const SIG_DFL: usize = 0;
const SIG_ERR: usize = usize::MAX;

fn device_abi() -> io::Result<()> {
    if IS_DEVICE_ABI {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "device calls are built for the MPC's 32-bit ARM only",
        ))
    }
}

/// ioctl with a pointer to a buffer the caller sized for this request.
/// `len` must equal the size encoded in `request` (checked here).
pub fn ioctl_buf(fd: RawFd, request: u32, buf: &mut [u8]) -> io::Result<c_int> {
    device_abi()?;
    let size = ((request >> 16) & 0x3FFF) as usize;
    if size != buf.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "ioctl {request:#x} wants {size} bytes, buffer has {}",
                buf.len()
            ),
        ));
    }
    // SAFETY: the kernel reads and writes at most `size` bytes at buf, which is that long.
    let rc = unsafe { ioctl(fd, request as c_ulong, buf.as_mut_ptr()) };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(rc)
    }
}

/// ioctl with no argument (such as PREPARE and DROP).
pub fn ioctl_none(fd: RawFd, request: u32) -> io::Result<c_int> {
    device_abi()?;
    if (request >> 16) & 0x3FFF != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "this request takes an argument",
        ));
    }
    // SAFETY: a request without an argument; the kernel reads nothing from us.
    let rc = unsafe { ioctl(fd, request as c_ulong, 0usize) };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(rc)
    }
}

/// ioctl that passes an int by pointer (_IOW(..., int)).
pub fn ioctl_int(fd: RawFd, request: u32, value: c_int) -> io::Result<c_int> {
    let mut bytes = value.to_ne_bytes();
    ioctl_buf(fd, request, &mut bytes)
}

pub fn clear_nonblock(fd: RawFd) -> io::Result<()> {
    device_abi()?;
    // SAFETY: plain fcntl on a descriptor we own.
    let flags = unsafe { fcntl(fd, F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: as above.
    if unsafe { fcntl(fd, F_SETFL, flags & !O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// SCHED_FIFO for the calling thread (pid 0 = this thread), with
/// SCHED_RESET_ON_FORK: a thread it starts afterwards (a writer, a loader, a
/// scanner) runs at the normal policy rather than at this priority, where
/// it would hold the audio thread's core for as long as it runs (two
/// SCHED_FIFO threads at one priority take turns only when one blocks). A
/// thread that wants real time asks for it itself, as the lanes do.
pub fn realtime(priority: i32) -> io::Result<()> {
    device_abi()?;
    let p = SchedParam {
        sched_priority: priority,
    };
    // SAFETY: p lives across the call.
    if unsafe { sched_setscheduler(0, SCHED_FIFO | SCHED_RESET_ON_FORK, &p) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn lock_memory() -> io::Result<()> {
    device_abi()?;
    // SAFETY: no pointers.
    if unsafe { mlockall(MCL_CURRENT | MCL_FUTURE) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The file the stop handler removes: null, or a NUL-terminated C string
/// that is never freed (a handler may be reading it when it is replaced).
static ON_STOP: AtomicPtr<c_char> = AtomicPtr::new(ptr::null_mut());

/// The stop handler: only async-signal-safe calls (unlink, signal, raise).
extern "C" fn on_stop(sig: c_int) {
    let path = ON_STOP.load(Ordering::Acquire);
    // SAFETY: `path` is null or a C string that lives as long as the
    // program; the signal number is the one this handler was called for.
    unsafe {
        if !path.is_null() {
            unlink(path);
        }
        signal(sig, SIG_DFL);
        raise(sig);
    }
}

/// From now on a stop from outside (SIGTERM as `systemctl stop` sends it,
/// SIGINT from Ctrl-C, SIGHUP when the terminal goes) first removes `path`,
/// then ends the program as the signal always did: its default action,
/// raised again, so the exit status is the same as without the handler.
pub fn remove_on_stop(path: &Path) -> io::Result<()> {
    let c = CString::new(path.as_os_str().as_bytes())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    // The old string (if any) is leaked on purpose: see ON_STOP.
    ON_STOP.store(c.into_raw(), Ordering::Release);
    for sig in [SIGTERM, SIGINT, SIGHUP] {
        let handler = on_stop as extern "C" fn(c_int) as usize;
        // SAFETY: the handler only makes async-signal-safe calls.
        if unsafe { signal(sig, handler) } == SIG_ERR {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// A stop no longer removes anything (the run ended and removed the file
/// itself). The handler stays and still ends the program as the signal
/// would.
pub fn forget_on_stop() {
    ON_STOP.store(ptr::null_mut(), Ordering::Release);
}

/// Tests only: a pass-through global allocator that counts the calls made
/// by threads that asked to be watched, so a test can show that a piece of
/// the audio loop never allocates or frees. Other test threads are not
/// counted. Not part of the program itself.
#[cfg(test)]
pub mod alloc_count {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! {
        static WATCH: Cell<bool> = const { Cell::new(false) };
        static CALLS: Cell<usize> = const { Cell::new(0) };
    }

    fn count() {
        if WATCH.with(Cell::get) {
            CALLS.with(|c| c.set(c.get() + 1));
        }
    }

    struct Counting;

    // SAFETY: every call goes straight to the system allocator.
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            count();
            unsafe { System.alloc(l) }
        }
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
            count();
            unsafe { System.dealloc(p, l) }
        }
        unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
            count();
            unsafe { System.realloc(p, l, n) }
        }
    }

    #[global_allocator]
    static GLOBAL: Counting = Counting;

    /// Allocations and frees made by this thread while running `f`.
    pub fn calls<R>(f: impl FnOnce() -> R) -> (R, usize) {
        CALLS.with(|c| c.set(0));
        WATCH.with(|w| w.set(true));
        let r = f();
        WATCH.with(|w| w.set(false));
        (r, CALLS.with(Cell::get))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_counting_allocator_sees_this_threads_allocations() {
        let (v, calls) = super::alloc_count::calls(|| vec![1u8; 64]);
        assert_eq!(v.len(), 64);
        assert!(calls >= 1);
        let ((), none) = super::alloc_count::calls(|| ());
        assert_eq!(none, 0);
    }
}
