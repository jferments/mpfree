//! The few C library calls this program needs that std does not offer:
//! ioctl (ALSA), fcntl (clear O_NONBLOCK), sched_setscheduler and mlockall.
//!
//! Every ioctl request number and structure size here is for the MPC One's
//! 32-bit ARM (armv7, EABI5 hard-float). They were checked against the
//! kernel's <sound/asound.h> compiled for that target (see the tests in
//! alsa.rs). On any other pointer width the device calls refuse to run, so
//! the host build can still be tested.

#![allow(unsafe_code)]

use std::io;
use std::os::raw::{c_int, c_ulong};
use std::os::unix::io::RawFd;

pub const IS_DEVICE_ABI: bool = cfg!(target_pointer_width = "32");

pub const O_NONBLOCK: i32 = 0o4000;
const F_GETFL: c_int = 3;
const F_SETFL: c_int = 4;
const SCHED_FIFO: c_int = 1;
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
}

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

/// SCHED_FIFO for the calling thread (pid 0 = this thread).
pub fn realtime(priority: i32) -> io::Result<()> {
    device_abi()?;
    let p = SchedParam {
        sched_priority: priority,
    };
    // SAFETY: p lives across the call.
    if unsafe { sched_setscheduler(0, SCHED_FIFO, &p) } < 0 {
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
