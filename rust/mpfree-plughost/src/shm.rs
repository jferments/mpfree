//! The block the host and the plugin process share: one request (context,
//! events, input) and one answer (output, parameter values), handed over
//! with two sequence numbers that double as futex words.
//!
//! The host writes a request, then stores its number in `req` (release)
//! and wakes the child. The child reads `req` (acquire), processes, writes
//! the answer, stores the same number in `done` (release) and wakes the
//! host. Each side writes only its own fields between those two stores, so
//! plain reads and writes through raw pointers are race-free.

use mpfree_node::MAX_BLOCK;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

pub const MAGIC: u32 = 0x4D50_4C48; // "MPLH"
pub const VERSION: u32 = 2;
/// Events one block carries to the plugin process.
pub const MAX_EVENTS: usize = 512;
/// Parameter values the plugin process reports back each block.
pub const MAX_PARAMS: usize = 1024;

pub const EV_NOTE_ON: u32 = 1;
pub const EV_NOTE_OFF: u32 = 2;
pub const EV_PRESSURE: u32 = 3;
pub const EV_BEND: u32 = 4;
pub const EV_ALL_OFF: u32 = 5;
pub const EV_PARAM: u32 = 6;
/// The node was reset (between blocks): the plugin resets before the block.
pub const EV_RESET: u32 = 7;
/// A per-note expression: `a` the note, `b` the kind (0 slide, 1 tone, 2
/// release velocity), `value` its value.
pub const EV_EXPRESSION: u32 = 8;

/// One event, flattened.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShmEvent {
    pub offset: u32,
    pub kind: u32,
    /// Note, or parameter index.
    pub a: u32,
    /// Velocity or pressure value.
    pub b: u32,
    /// Bend or parameter value.
    pub value: f32,
    pub _pad: u32,
}

/// The shared block.
#[repr(C)]
pub struct Shm {
    pub magic: u32,
    pub version: u32,
    /// The request the host posted last (futex word for the child).
    pub req: AtomicU32,
    /// The request the child finished last (futex word for the host).
    pub done: AtomicU32,
    // ---- the request (host writes) ----
    pub frames: u32,
    pub nevents: u32,
    pub rate: u32,
    pub playing: u32,
    pub frame: u64,
    pub bpm: f64,
    pub beats: f64,
    pub events: [ShmEvent; MAX_EVENTS],
    pub in_l: [f32; MAX_BLOCK],
    pub in_r: [f32; MAX_BLOCK],
    // ---- the answer (child writes) ----
    pub out_l: [f32; MAX_BLOCK],
    pub out_r: [f32; MAX_BLOCK],
    pub nvalues: u32,
    /// Blocks the plugin failed (its own error), all-time.
    pub errors: u32,
    pub values: [f32; MAX_PARAMS],
}

pub const SIZE: usize = std::mem::size_of::<Shm>();

/// Writes a fresh header into a zeroed mapping.
///
/// # Safety
/// `p` points at `SIZE` writable, zeroed bytes, aligned for `Shm`.
pub unsafe fn init(p: *mut Shm) {
    (*p).magic = MAGIC;
    (*p).version = VERSION;
}

/// Checks a mapping's header.
///
/// # Safety
/// `p` points at a mapping of at least `SIZE` bytes.
pub unsafe fn valid(p: *const Shm) -> bool {
    (*p).magic == MAGIC && (*p).version == VERSION
}

const FUTEX_WAIT: libc::c_int = 0;
const FUTEX_WAKE: libc::c_int = 1;

/// Sleeps while `word` holds `expected`, at most `timeout` (None: no
/// limit). Returns early on a wake, a change, a signal or the timeout; the
/// caller checks the word again. No allocation, no lock.
pub fn futex_wait(word: &AtomicU32, expected: u32, timeout: Option<Duration>) {
    let ts;
    let tsp = match timeout {
        Some(t) => {
            ts = libc::timespec {
                tv_sec: t.as_secs() as libc::time_t,
                tv_nsec: t.subsec_nanos() as libc::c_long,
            };
            &ts as *const libc::timespec
        }
        None => std::ptr::null(),
    };
    // SAFETY: a valid futex word in shared memory (no FUTEX_PRIVATE_FLAG,
    // so it works across processes) and a valid or null timeout.
    unsafe {
        libc::syscall(
            libc::SYS_futex,
            word.as_ptr(),
            FUTEX_WAIT,
            expected,
            tsp,
            std::ptr::null::<u32>(),
            0,
        );
    }
}

/// Wakes every waiter on `word`.
pub fn futex_wake(word: &AtomicU32) {
    // SAFETY: as futex_wait.
    unsafe {
        libc::syscall(
            libc::SYS_futex,
            word.as_ptr(),
            FUTEX_WAKE,
            i32::MAX,
            std::ptr::null::<libc::timespec>(),
            std::ptr::null::<u32>(),
            0,
        );
    }
}

/// A shared mapping of one `Shm` over a memfd.
pub struct Mapping {
    pub fd: libc::c_int,
    pub ptr: *mut Shm,
}

// SAFETY: the mapping is plain shared memory; access discipline is the
// req/done protocol described above.
unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}

impl Mapping {
    /// Makes a new memfd of `SIZE` bytes and maps it, with a fresh header.
    pub fn create() -> std::io::Result<Mapping> {
        // SAFETY: plain syscalls; every failure path closes what it opened.
        unsafe {
            let fd = libc::syscall(
                libc::SYS_memfd_create,
                c"mpfree-plughost".as_ptr(),
                libc::MFD_CLOEXEC,
            ) as libc::c_int;
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::ftruncate(fd, SIZE as libc::off_t) != 0 {
                let e = std::io::Error::last_os_error();
                libc::close(fd);
                return Err(e);
            }
            let m = Self::map(fd)?;
            init(m.ptr);
            Ok(m)
        }
    }

    /// Maps an existing memfd (the child's end).
    pub fn attach(fd: libc::c_int) -> std::io::Result<Mapping> {
        // SAFETY: mmap of an fd we were handed; checked below.
        unsafe {
            let m = Self::map(fd)?;
            if !valid(m.ptr) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "not an mpfree-plughost block",
                ));
            }
            Ok(m)
        }
    }

    unsafe fn map(fd: libc::c_int) -> std::io::Result<Mapping> {
        let p = libc::mmap(
            std::ptr::null_mut(),
            SIZE,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0,
        );
        if p == libc::MAP_FAILED {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(e);
        }
        Ok(Mapping {
            fd,
            ptr: p as *mut Shm,
        })
    }

    /// The host's sequence word. (Only the two atomics are handed out as
    /// references: the other fields change under the other process, so
    /// they are reached through `ptr`, one field at a time.)
    pub fn req(&self) -> &AtomicU32 {
        // SAFETY: valid for the mapping's life; an atomic field.
        unsafe { &(*self.ptr).req }
    }

    /// The child's sequence word.
    pub fn done(&self) -> &AtomicU32 {
        // SAFETY: as req.
        unsafe { &(*self.ptr).done }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: unmapping and closing what we mapped and opened.
        unsafe {
            libc::munmap(self.ptr as *mut libc::c_void, SIZE);
            libc::close(self.fd);
        }
    }
}

/// Loads a sequence number.
pub fn load(w: &AtomicU32) -> u32 {
    w.load(Ordering::Acquire)
}

/// Stores a sequence number and wakes the other side.
pub fn post(w: &AtomicU32, v: u32) {
    w.store(v, Ordering::Release);
    futex_wake(w);
}
