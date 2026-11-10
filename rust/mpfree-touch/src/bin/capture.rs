//! M12: bounded, non-grabbing, read-only ILI2117 event capture.
//!
//! Prints every evdev event from `/dev/input/event0` as one text line for at
//! most CAPTURE_SECONDS, then exits. The workstation runs it once per touch
//! step, after the owner presses Enter. All decoding and calibration happens
//! on the workstation (`mpfree-touch-analyze`).
//!
//! What this program does on the MPC, and nothing else:
//! - reads sysfs to confirm event0 is the ILI2117 on I2C 4-0026;
//! - opens the fixed path /dev/input/event0 read-only and non-blocking;
//! - read()s events into its own evdev client buffer and prints them.
//!
//! It never grabs the device (the factory app keeps receiving every touch),
//! never issues an ioctl, never writes to any file or device, and accepts no
//! path arguments. Linux evdev gives every reader its own copy of each event,
//! so reading here does not take events away from the factory app.
#![forbid(unsafe_code)]

use std::fs::{self, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
#[cfg(target_os = "linux")]
use std::os::unix::fs::OpenOptionsExt;
use std::time::{Duration, Instant};

const DEVICE: &str = "/dev/input/event0";
/// One touch step: about 3 s of countdown on the workstation, then 6 s to
/// perform it. Must match STEP_SECONDS in tools/mpc-one/host_touch_capture.py.
const CAPTURE_SECONDS: u64 = 9;
const WATCHDOG_SECONDS: u64 = 20;
const MAX_EVENTS: u64 = 100_000;
const POLL_MS: u64 = 10;
/// struct input_event on 32-bit ARM Linux: u32 sec, u32 usec, u16 type,
/// u16 code, i32 value. A 64-bit kernel would use 24 bytes; any read that is
/// not a whole number of 16-byte records stops the capture.
const EVENT_BYTES: usize = 16;
const EV_SYN: u16 = 0;
/// Highest event type Linux defines (EV_MAX). A larger value means the
/// record layout is not what we assumed.
const EV_MAX: u16 = 0x1f;
const SYN_DROPPED: u16 = 3;

const HEADER: &str = "MPFREE_TOUCH_CAPTURE_V2";
const ARMED: &str = "MPFREE_TOUCH_CAPTURE_ARMED";
const END: &str = "MPFREE_TOUCH_CAPTURE_END";
const WATCHDOG_MARKER: &str = "MPFREE_TOUCH_CAPTURE_WATCHDOG_20S_ARMED";
const OPT_IN: &str = "MPFREE_ACCEPT_TOUCH_CAPTURE";

/// Same identity rules as the M10 metadata probe (src/main.rs), duplicated
/// on purpose so the M10 binary and its CI checks stay untouched.
fn strict_sysfs_identity() -> Result<(), String> {
    fn expected(path: &str, target: &str) -> Result<(), String> {
        let content = fs::read_to_string(path)
            .map_err(|e| format!("cannot read touch identity {path}: {e}"))?;
        if content.trim() != target {
            return Err(format!("touch identity mismatch at {path}; expected {target}"));
        }
        Ok(())
    }
    expected("/sys/class/input/event0/device/name", "ILI2117 Touchscreen")?;
    expected("/sys/class/input/event0/device/id/bustype", "0018")?;
    expected("/sys/bus/i2c/devices/4-0026/name", "ili2117")?;
    let event_device = fs::canonicalize("/sys/class/input/event0/device")
        .map_err(|e| format!("cannot resolve event0 sysfs parent: {e}"))?;
    if !event_device.components().any(|part| part.as_os_str() == "4-0026") {
        return Err("event0 does not belong to physical I2C 4-0026".into());
    }
    let driver = fs::canonicalize("/sys/bus/i2c/devices/4-0026/driver")
        .map_err(|e| format!("cannot resolve ILI2117 kernel driver: {e}"))?;
    if !driver.ends_with("bus/i2c/drivers/ili2117") {
        return Err("unrecognized kernel driver; refusing to open event0".into());
    }
    let compatible = fs::read("/sys/bus/i2c/devices/4-0026/of_node/compatible")
        .map_err(|e| format!("cannot read touchscreen compatible: {e}"))?;
    if !compatible.split(|b| *b == 0).any(|part| part == b"ilitek,ili2117") {
        return Err("I2C device-tree compatible mismatch; refusing to open event0".into());
    }
    Ok(())
}

fn arm_watchdog() -> Result<(), String> {
    use std::sync::mpsc::sync_channel;
    let (sender, receiver) = sync_channel::<()>(0);
    std::thread::Builder::new()
        .name("mpfree-capture-watchdog".into())
        .spawn(move || {
            let _ = sender.send(());
            std::thread::sleep(Duration::from_secs(WATCHDOG_SECONDS));
            // Last resort: abnormal exit closes the evdev descriptor.
            std::process::abort();
        })
        .map_err(|e| format!("unable to start watchdog: {e}"))?;
    receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| "watchdog did not arm".to_string())?;
    eprintln!("{WATCHDOG_MARKER}");
    Ok(())
}

/// Parse whole 16-byte records. Pure, so it is unit-tested on the host.
/// (sec, usec, type, code, value)
type Record = (u32, u32, u16, u16, i32);

fn parse_records(bytes: &[u8]) -> Result<Vec<Record>, String> {
    if !bytes.len().is_multiple_of(EVENT_BYTES) {
        return Err(format!(
            "read of {} bytes is not a whole number of {EVENT_BYTES}-byte input_events",
            bytes.len()
        ));
    }
    let records: Vec<_> = bytes
        .chunks_exact(EVENT_BYTES)
        .map(|r| {
            let u32_at = |i: usize| u32::from_le_bytes([r[i], r[i + 1], r[i + 2], r[i + 3]]);
            let u16_at = |i: usize| u16::from_le_bytes([r[i], r[i + 1]]);
            (u32_at(0), u32_at(4), u16_at(8), u16_at(10), u32_at(12) as i32)
        })
        .collect();
    if let Some(bad) = records.iter().find(|r| r.2 > EV_MAX || r.1 >= 1_000_000) {
        return Err(format!("implausible input_event {bad:?}; record layout mismatch"));
    }
    Ok(records)
}

fn capture() -> Result<(), String> {
    if std::env::var(OPT_IN).as_deref() != Ok("YES") {
        return Err(format!("capture needs {OPT_IN}=YES"));
    }
    #[cfg(not(target_os = "linux"))]
    return Err("Linux evdev required".into());

    #[cfg(target_os = "linux")]
    {
        strict_sysfs_identity()?;
        arm_watchdog()?;
        // Read-only, non-blocking. No ioctl of any kind follows, so no grab.
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(0o4000) // O_NONBLOCK on Linux ARM/asm-generic
            .open(DEVICE)
            .map_err(|e| format!("failed to open {DEVICE}: {e}"))?;
        let stdout = io::stdout();
        let mut out = io::BufWriter::new(stdout.lock());
        let w = |out: &mut io::BufWriter<io::StdoutLock>, s: &str| {
            writeln!(out, "{s}").map_err(|e| format!("stdout: {e}"))
        };
        w(&mut out, HEADER)?;
        w(&mut out, "device=event0")?;
        w(&mut out, "access=evdev_RDONLY_NONBLOCK")?;
        w(&mut out, "exclusive_grab=no")?;
        w(&mut out, &format!("capture_seconds={CAPTURE_SECONDS}"))?;
        w(&mut out, &format!("max_events={MAX_EVENTS}"))?;
        w(&mut out, ARMED)?;
        out.flush().map_err(|e| format!("stdout: {e}"))?;

        let started = Instant::now();
        let deadline = Duration::from_secs(CAPTURE_SECONDS);
        let mut buf = [0u8; EVENT_BYTES * 64];
        let (mut events, mut drops, mut reads) = (0u64, 0u64, 0u64);
        let mut status = "PASS";
        while started.elapsed() < deadline {
            match file.read(&mut buf) {
                Ok(0) => return Err("event0 returned end of file".into()),
                Ok(n) => {
                    reads += 1;
                    for (sec, usec, kind, code, value) in parse_records(&buf[..n])? {
                        if kind == EV_SYN && code == SYN_DROPPED {
                            drops += 1;
                        }
                        writeln!(out, "E {sec} {usec} {kind} {code} {value}")
                            .map_err(|e| format!("stdout: {e}"))?;
                        events += 1;
                    }
                    if events >= MAX_EVENTS {
                        status = "EVENT_CAP_REACHED";
                        break;
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    out.flush().map_err(|e| format!("stdout: {e}"))?;
                    std::thread::sleep(Duration::from_millis(POLL_MS));
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(format!("read {DEVICE}: {e}")),
            }
        }
        drop(file);
        writeln!(
            out,
            "{END} events={events} drops={drops} reads={reads} elapsed_ms={} status={status}",
            started.elapsed().as_millis()
        )
        .map_err(|e| format!("stdout: {e}"))?;
        out.flush().map_err(|e| format!("stdout: {e}"))?;
        if status != "PASS" {
            return Err(status.to_lowercase());
        }
        Ok(())
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let result = match (args.next().as_deref(), args.next()) {
        (Some("--capture"), None) => capture(),
        (Some("--self-test"), None) => self_test(),
        _ => Err("usage: mpfree-touch-capture --self-test | --capture (requires opt-in)".into()),
    };
    if let Err(err) = result {
        eprintln!("MPFREE TOUCH CAPTURE FAIL CLOSED: {err}");
        std::process::exit(1);
    }
}

/// Host-only check of the record parser; opens nothing.
fn self_test() -> Result<(), String> {
    let mut rec = Vec::new();
    rec.extend_from_slice(&7u32.to_le_bytes());
    rec.extend_from_slice(&123u32.to_le_bytes());
    rec.extend_from_slice(&3u16.to_le_bytes());
    rec.extend_from_slice(&57u16.to_le_bytes());
    rec.extend_from_slice(&(-1i32).to_le_bytes());
    if parse_records(&rec)? != vec![(7, 123, 3, 57, -1)] {
        return Err("record parser mismatch".into());
    }
    if parse_records(&rec[..15]).is_ok() {
        return Err("short record accepted".into());
    }
    println!("MPFREE_TOUCH_CAPTURE_SELFTEST_V1");
    println!("device_opened=no");
    println!("status=PASS");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_whole_records_and_rejects_partial_ones() {
        assert!(self_test().is_ok());
        assert!(parse_records(&[0u8; 24]).is_err(), "64-bit layout must not parse");
        // Two 24-byte records are 48 bytes, a multiple of 16: the field
        // checks must still catch the misaligned layout.
        let mut wide = Vec::new();
        for _ in 0..2 {
            wide.extend_from_slice(&1_700_000_000u64.to_le_bytes());
            wide.extend_from_slice(&500_000u64.to_le_bytes());
            wide.extend_from_slice(&3u16.to_le_bytes());
            wide.extend_from_slice(&53u16.to_le_bytes());
            wide.extend_from_slice(&1024i32.to_le_bytes());
        }
        assert!(parse_records(&wide).is_err());
        assert_eq!(parse_records(&[]).unwrap(), vec![]);
    }
}
