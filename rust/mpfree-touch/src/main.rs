//! M9: narrowly scoped Linux evdev metadata-only ioctl utility.
//!
//! Pure-Rust DSP and display modules stay unsafe-free. This tiny, separately
//! reviewed FFI boundary is the ONLY unsafe code, required by Linux ioctl.
//! Never reads events, never calls EVIOCGRAB, never opens DRM/I2C/ALSA,
//! never changes factory systemd, scheduling, calibration or firmware.
//!
//! --self-test has NO device I/O. --inspect is not in the SSH wrapper and
//! requires explicit opt-in; it opens only verified ili2117 event0 O_RDONLY.
#![deny(unsafe_op_in_unsafe_fn)]

use mpfree_touch::{
    ABS_MT_POSITION_X, ABS_MT_POSITION_Y, ABS_MT_SLOT, ABS_MT_TRACKING_ID,
    ABS_X, ABS_Y, AxisInfo, validate_fixture,
};
use std::fs::{self, OpenOptions};
use std::os::fd::AsRawFd;
#[cfg(target_os = "linux")]
use std::os::unix::fs::OpenOptionsExt;
use std::ffi::{c_int, c_ulong};

#[cfg(target_os = "linux")]
#[repr(C)]
#[derive(Default)]
struct LinuxInputAbsInfo {
    value: i32,      // current value; never exposed or printed
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    // The UAPI expects (fd, EVIOCGABS request, struct input_absinfo*).
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
}

#[cfg(target_os = "linux")]
const fn eviocgabs(code: u8) -> c_ulong {
    // Linux asm-generic ioctl encoding: _IOR('E', 0x40 + code, 24).
    // On this platform input_absinfo has six signed 32-bit members.
    let request: u64 = (2_u64 << 30)
        | (24_u64 << 16)
        | ((b'E' as u64) << 8)
        | (0x40_u64 + code as u64);
    request as c_ulong
}

#[cfg(target_os = "linux")]
fn get_axis(fd: c_int, code: u8) -> Result<AxisInfo, String> {
    let mut out = LinuxInputAbsInfo::default();
    // SAFETY: fd references an O_RDONLY verified Linux evdev device;
    // eviocgabs is _IOR, with no driver state mutation or event consumption.
    // out is repr(C), 24 bytes, correctly aligned and writable by the kernel.
    // The pointer lives until the synchronous ioctl returns. No EVIOCGRAB.
    let code_result = unsafe { ioctl(fd, eviocgabs(code), &mut out as *mut LinuxInputAbsInfo) };
    if code_result != 0 {
        return Err(format!(
            "EVIOCGABS({code}) failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    AxisInfo {
        minimum: out.minimum, maximum: out.maximum,
        fuzz: out.fuzz, flat: out.flat, resolution: out.resolution,
    }
    .check()
    .map_err(|e| format!("invalid axis {code}: {e:?}"))
}

fn strict_sysfs_identity() -> Result<(), String> {
    fn expected(path: &str, target: &str) -> Result<(), String> {
        let content = fs::read_to_string(path)
            .map_err(|e| format!("cannot read exact touch metadata {path}: {e}"))?;
        if content.trim() != target {
            return Err(format!("touch identity mismatch at {path}; expected {target}"));
        }
        Ok(())
    }
    expected("/sys/class/input/event0/device/name", "ILI2117 Touchscreen")?;
    expected("/sys/class/input/event0/device/id/bustype", "0018")?;
    expected("/sys/bus/i2c/devices/4-0026/name", "ili2117")?;
    // Device identity must follow the actual event0 -> input0 -> I2C4-0026
    // sysfs ancestry. Checking only the name is insufficient: an unrelated
    // event device could reuse the same user-visible label.
    let event_device = fs::canonicalize("/sys/class/input/event0/device")
        .map_err(|e| format!("cannot resolve event0 sysfs parent: {e}"))?;
    if !event_device.components().any(|part| part.as_os_str() == "4-0026") {
        return Err("event0 does not belong to physical I2C 4-0026".into());
    }
    // Inspect the actual physical I2C device's bound driver, not a fragile
    // assumed number of 'device/' symlinks between evdev and its bus parent.
    let driver = fs::canonicalize("/sys/bus/i2c/devices/4-0026/driver")
        .map_err(|e| format!("cannot resolve ILI2117 kernel driver: {e}"))?;
    if !driver.ends_with("bus/i2c/drivers/ili2117") {
        return Err("unrecognized kernel driver; aborting before evdev open".into());
    }
    let uevent = fs::read_to_string("/sys/bus/i2c/devices/4-0026/uevent")
        .map_err(|e| format!("missing physical ILI2117 driver identity: {e}"))?;
    if !uevent.lines().any(|line| line == "OF_COMPATIBLE_0=ilitek,ili2117")
        || !uevent.lines().any(|line| line == "DRIVER=ili2117")
    {
        return Err("I2C compatible or driver mismatch; aborting".into());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn arm_inspection_watchdog() -> Result<(), String> {
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;
    let (sender, receiver) = sync_channel::<()>(0);
    std::thread::Builder::new()
        .name("mpfree-touch-timer".into())
        .spawn(move || {
            // The parent only continues after the independently scheduled
            // watchdog thread has entered this routine.
            let _ = sender.send(());
            std::thread::sleep(Duration::from_secs(5));
            // Terminal fallback: abnormal termination releases evdev fd.
            std::process::abort();
        })
        .map_err(|e| format!("unable to start 5s child watchdog: {e}"))?;
    receiver.recv_timeout(Duration::from_secs(1))
        .map_err(|_| "5s watchdog did not arm before touch inspection".to_string())?;
    eprintln!("MPFREE_TOUCH_IOCTL_WATCHDOG_5S_ARMED");
    Ok(())
}

fn metadata_mode() -> Result<(), String> {
    if std::env::var("MPFREE_ACCEPT_TOUCH_IOCTL").as_deref() != Ok("YES") {
        return Err("metadata mode needs MPFREE_ACCEPT_TOUCH_IOCTL=YES".into());
    }
    #[cfg(not(target_os = "linux"))]
    return Err("Linux evdev required".into());

    #[cfg(target_os = "linux")]
    {
        strict_sysfs_identity()?;
        arm_inspection_watchdog()?;
        // Only kernel O_RDONLY/O_NONBLOCK open. Never call read(), write(),
        // EVIOCGRAB, EVIOCSABS, or any nonmetadata ioctl request.
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(0o4000) // O_NONBLOCK on Linux ARM/asm-generic
            .open("/dev/input/event0")
            .map_err(|e| format!("failed to open bound ILI2117 event0: {e}"))?;
        let fd = file.as_raw_fd();
        let codes = [
            ("abs_x", ABS_X),
            ("abs_y", ABS_Y),
            ("abs_mt_slot", ABS_MT_SLOT),
            ("abs_mt_x", ABS_MT_POSITION_X),
            ("abs_mt_y", ABS_MT_POSITION_Y),
            ("abs_mt_tracking_id", ABS_MT_TRACKING_ID),
        ];
        let mut readings = Vec::with_capacity(codes.len());
        for (name, code) in codes {
            readings.push((name, get_axis(fd, code)?));
        }
        let slots = readings[2].1.slot_count()
            .map_err(|e| format!("unsupported reported ABS_MT_SLOT bounds: {e:?}"))?;
        // After the query, the descriptor is already safe to drop; the
        // remaining code only prints metadata. No current touch values.
        drop(file);
        println!("MPFREE_TOUCH_ABS_METADATA_V1");
        println!("driver=ili2117");
        println!("device=event0");
        println!("bus=i2c-4-0026");
        println!("access=evdev_RDONLY");
        println!("events_consumed=0");
        println!("exclusive_grab=no");
        for (name, item) in readings {
            println!("{name}_min={}", item.minimum);
            println!("{name}_max={}", item.maximum);
            println!("{name}_fuzz={}", item.fuzz);
            println!("{name}_flat={}", item.flat);
            println!("{name}_resolution={}", item.resolution);
        }
        println!("multitouch_slots={slots}");
        println!("status=PASS");
        Ok(())
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args();
    let _program = args.next();
    let mode = args.next();
    if args.next().is_some() {
        return Err("no additional args or arbitrary device paths allowed".into());
    }
    match mode.as_deref() {
        Some("--self-test") => {
            #[cfg(target_os = "linux")]
            {
                if eviocgabs(ABS_X) != 0x80184540
                    || eviocgabs(ABS_MT_TRACKING_ID) != 0x80184579 {
                    return Err("Linux EVIOCGABS ABI encoding mismatch".into());
                }
            }
            if validate_fixture() != Ok(10) {
                return Err("synthetic ABS_MT_SLOT model failed".into());
            }
            println!("MPFREE_TOUCH_METADATA_SELFTEST_V1");
            println!("backend=synthetic");
            println!("device_opened=no");
            println!("multitouch_slots_fixture=10");
            println!("status=PASS");
            Ok(())
        }
        Some("--inspect") => metadata_mode(),
        _ => Err("usage: mpfree-touch --self-test | --inspect (requires explicit opt-in)".into()),
    }
}

fn main() {
    if let Err(err) = run() {
        eprintln!("MPFREE TOUCH METADATA FAIL CLOSED: {err}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_ioctl_read_request_matches_real_uapi() {
        use super::*;
        assert_eq!(std::mem::size_of::<LinuxInputAbsInfo>(), 24);
        assert_eq!(eviocgabs(ABS_X), 0x80184540);
        assert_eq!(eviocgabs(ABS_MT_POSITION_X), 0x80184575);
        assert_eq!(eviocgabs(ABS_MT_TRACKING_ID), 0x80184579);
    }
}
