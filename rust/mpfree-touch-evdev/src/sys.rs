//! The only unsafe code in this crate: two read-only evdev ioctls.
//!
//! - `EVIOCGABS(code)`: copies one axis's `input_absinfo` (value, range,
//!   noise) out of the kernel.
//! - `EVIOCGMTSLOTS(len)`: copies one multitouch value for every slot.
//!
//! Both are `_IOR` requests: they read device state, change nothing, consume
//! no events and do not affect other readers such as Akai's app. There is no
//! `EVIOCGRAB`, `EVIOCSABS` or any other request here, and CI checks that.
#![allow(unsafe_code)]

use std::io;
use std::os::raw::{c_int, c_ulong};

/// `O_NONBLOCK` from the Linux UAPI; the same value on ARM and x86.
pub const O_NONBLOCK: i32 = 0o4000;

/// `struct input_absinfo`: six signed 32-bit members on every Linux ABI.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AbsInfo {
    pub value: i32,
    pub minimum: i32,
    pub maximum: i32,
    pub fuzz: i32,
    pub flat: i32,
    pub resolution: i32,
}

unsafe extern "C" {
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
}

/// asm-generic `_IOC(_IOC_READ, 'E', nr, size)`, used by ARM and x86 alike.
const fn ior(nr: u64, size: u64) -> c_ulong {
    ((2_u64 << 30) | (size << 16) | ((b'E' as u64) << 8) | nr) as c_ulong
}

pub const fn eviocgabs(code: u8) -> c_ulong {
    ior(0x40 + code as u64, std::mem::size_of::<AbsInfo>() as u64)
}

pub const fn eviocgmtslots(len_bytes: usize) -> c_ulong {
    ior(0x0a, len_bytes as u64)
}

/// Linux caps multitouch slots at 64; the request buffer is the code plus
/// one value per slot.
pub const MAX_SLOTS: usize = 64;

pub fn get_abs(fd: c_int, code: u8) -> io::Result<AbsInfo> {
    let mut out = AbsInfo::default();
    // SAFETY: EVIOCGABS is _IOR with size_of::<AbsInfo>() == 24 encoded in
    // the request; `out` is a writable, aligned repr(C) value of exactly that
    // size that outlives this synchronous call. It reads state only.
    let rc = unsafe { ioctl(fd, eviocgabs(code), &mut out as *mut AbsInfo) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(out)
}

/// One value (`code` = TRACKING_ID, POSITION_X ...) for each of `slots` slots.
pub fn get_mt_slots(fd: c_int, code: u8, slots: usize) -> io::Result<Vec<i32>> {
    if slots == 0 || slots > MAX_SLOTS {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "slot count out of range"));
    }
    // struct input_mt_request_layout { __u32 code; __s32 values[slots]; }
    let mut buf = [0i32; MAX_SLOTS + 1];
    buf[0] = i32::from(code);
    let len = std::mem::size_of::<i32>() * (slots + 1);
    // SAFETY: the request encodes `len` bytes, which is at most the size of
    // `buf` (an aligned, writable i32 array that outlives the call). The
    // kernel writes at most `len` bytes and reads only the leading code.
    let rc = unsafe { ioctl(fd, eviocgmtslots(len), buf.as_mut_ptr()) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(buf[1..=slots].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_match_the_linux_uapi() {
        // Computed by hand from the <linux/input.h> definitions:
        // EVIOCGABS(ABS_MT_SLOT=0x2f) = _IOR('E', 0x6f, 24).
        assert_eq!(eviocgabs(0x2f) as u64, 0x8018_456f);
        assert_eq!(eviocgabs(0) as u64, 0x8018_4540);
        // EVIOCGMTSLOTS(44) for 10 slots: _IOC(READ, 'E', 0x0a, 44).
        assert_eq!(eviocgmtslots(44) as u64, 0x802c_450a);
        assert_eq!(std::mem::size_of::<AbsInfo>(), 24);
    }

    #[test]
    fn mt_slots_rejects_bad_counts_before_any_syscall() {
        assert!(get_mt_slots(-1, 57, 0).is_err());
        assert!(get_mt_slots(-1, 57, 65).is_err());
    }

    #[test]
    fn a_bad_fd_is_an_error_not_a_crash() {
        assert!(get_abs(-1, 0x2f).is_err());
        assert!(get_mt_slots(-1, 57, 10).is_err());
    }
}
