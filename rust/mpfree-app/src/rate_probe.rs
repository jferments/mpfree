//! The read-only rate probe (DSP quality step 0.3,
//! docs/dev/design/dsp-quality.md#the-rate-probe): asks an ALSA PCM device
//! which sample rates, sample formats and channel counts its driver would
//! accept, and prints the answers.
//!
//! It only asks. The rules it keeps (the tests below check them):
//!
//! - The one call it makes on a device is HW_REFINE ([`Refine`] is the only
//!   way in). It never sets a format (HW_PARAMS), prepares, starts, reads
//!   or writes audio.
//! - [`probe_path`] opens the device non-blocking: a device another program
//!   holds is reported once as busy and never retried.
//! - Every device it opens is closed again on every path: the [`alsa::Pcm`]
//!   is dropped before [`probe_path`] returns, failures included.
//! - Nothing here returns an error or panics on a device's answer: every
//!   outcome is a line of text, so the caller (inventory, the pad player)
//!   always carries on.
//!
//! A yes from the driver does not prove the converters run at that rate;
//! that needs a tone through the outputs and back, a separate step that
//! changes the stream and has its own go.

use crate::alsa::{self, HwParams, Interval};
use std::io;

/// The rates asked about, in Hz.
pub const RATES: [u32; 6] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];

/// The one question the probe may ask a device. Anything that only
/// implements this cannot be set up, started, read or written through it.
pub trait Refine {
    fn refine(&self, p: &mut HwParams) -> io::Result<()>;
}

impl Refine for alsa::Pcm {
    fn refine(&self, p: &mut HwParams) -> io::Result<()> {
        alsa::Pcm::refine(self, p)
    }
}

/// ALSA's names for the sample formats a mask bit stands for
/// (<sound/asound.h>, SNDRV_PCM_FORMAT_*); others print as their number.
fn format_name(bit: u32) -> Option<&'static str> {
    Some(match bit {
        0 => "S8",
        1 => "U8",
        2 => "S16_LE",
        3 => "S16_BE",
        4 => "U16_LE",
        5 => "U16_BE",
        6 => "S24_LE",
        7 => "S24_BE",
        8 => "U24_LE",
        9 => "U24_BE",
        10 => "S32_LE",
        11 => "S32_BE",
        12 => "U32_LE",
        13 => "U32_BE",
        14 => "FLOAT_LE",
        15 => "FLOAT_BE",
        16 => "FLOAT64_LE",
        17 => "FLOAT64_BE",
        32 => "S24_3LE",
        33 => "S24_3BE",
        _ => return None,
    })
}

/// The formats a refined parameter block still allows, as names.
fn formats(p: &HwParams) -> String {
    let names: Vec<String> = (0..256u32)
        .filter(|&b| p.mask_has(alsa::PARAM_FORMAT, b))
        .map(|b| format_name(b).map_or_else(|| format!("format {b}"), str::to_string))
        .collect();
    if names.is_empty() {
        "none".to_string()
    } else {
        names.join(" ")
    }
}

fn interval(p: &HwParams, param: usize) -> String {
    let i: Interval = p.interval(param);
    i.describe()
}

/// Ask `dev` about every rate in [`RATES`], after one open question about
/// the rate range. One line per answer; never fails.
pub fn probe(dev: &impl Refine) -> Vec<String> {
    let mut lines = Vec::with_capacity(RATES.len() + 1);
    let mut any = HwParams::any();
    lines.push(match dev.refine(&mut any) {
        Ok(()) => format!(
            "any rate: rate {}, formats {}, channels {}",
            interval(&any, alsa::PARAM_RATE),
            formats(&any),
            interval(&any, alsa::PARAM_CHANNELS)
        ),
        Err(e) => format!("any rate: asking failed: {e}"),
    });
    for rate in RATES {
        let mut p = HwParams::any();
        p.set_int(alsa::PARAM_RATE, rate);
        lines.push(match dev.refine(&mut p) {
            Ok(()) => format!(
                "{rate} Hz: yes; formats {}; channels {}",
                formats(&p),
                interval(&p, alsa::PARAM_CHANNELS)
            ),
            Err(e) if e.raw_os_error() == Some(EINVAL) => format!("{rate} Hz: no"),
            Err(e) => format!("{rate} Hz: asking failed: {e}"),
        });
    }
    lines
}

/// EINVAL: what HW_REFINE answers when nothing can satisfy the request.
const EINVAL: i32 = 22;
/// EBUSY: what opening a PCM device answers while another program holds it.
const EBUSY: i32 = 16;

/// Open `path` without waiting, ask [`probe`]'s questions, close it again.
/// The first line names the device; a device that won't open is one more
/// line saying why.
pub fn probe_path(path: &str) -> Vec<String> {
    let mut lines = vec![format!("rate probe {path} (read-only: HW_REFINE only)")];
    match alsa::Pcm::open_nonblocking(path) {
        Ok(pcm) => {
            lines.extend(probe(&pcm).into_iter().map(|l| format!("  {l}")));
            drop(pcm);
            lines.push("  closed".to_string());
        }
        Err(e) if e.raw_os_error() == Some(EBUSY) => {
            lines.push("  busy: another program holds it (not retried)".to_string())
        }
        Err(e) => lines.push(format!("  not opened: {e}")),
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// A driver that allows `rates` and `fmts` with 2 channels, counting
    /// the questions it is asked.
    struct FakeDriver {
        rates: &'static [u32],
        fmts: &'static [u32],
        asked: Cell<usize>,
    }

    impl Refine for FakeDriver {
        fn refine(&self, p: &mut HwParams) -> io::Result<()> {
            self.asked.set(self.asked.get() + 1);
            match p.interval(alsa::PARAM_RATE).single() {
                Some(want) if !self.rates.contains(&want) => {
                    return Err(io::Error::from_raw_os_error(EINVAL));
                }
                Some(_) => {}
                None => {
                    let lo = *self.rates.iter().min().unwrap();
                    let hi = *self.rates.iter().max().unwrap();
                    p.set_range(alsa::PARAM_RATE, lo, hi);
                }
            }
            p.set_mask(alsa::PARAM_FORMAT, self.fmts[0]);
            for &f in &self.fmts[1..] {
                p.allow_mask(alsa::PARAM_FORMAT, f);
            }
            p.set_int(alsa::PARAM_CHANNELS, 2);
            Ok(())
        }
    }

    #[test]
    fn asks_once_about_the_range_then_once_per_rate() {
        let d = FakeDriver {
            rates: &[44_100, 48_000],
            fmts: &[2, 10],
            asked: Cell::new(0),
        };
        let lines = probe(&d);
        assert_eq!(d.asked.get(), 1 + RATES.len());
        assert_eq!(lines.len(), 1 + RATES.len());
        assert_eq!(
            lines[0],
            "any rate: rate [44100..48000], formats S16_LE S32_LE, channels 2"
        );
        assert_eq!(lines[1], "44100 Hz: yes; formats S16_LE S32_LE; channels 2");
        assert_eq!(lines[2], "48000 Hz: yes; formats S16_LE S32_LE; channels 2");
        for (line, rate) in lines[3..].iter().zip(&RATES[2..]) {
            assert_eq!(line, &format!("{rate} Hz: no"));
        }
    }

    struct Refuses(i32);

    impl Refine for Refuses {
        fn refine(&self, _: &mut HwParams) -> io::Result<()> {
            Err(io::Error::from_raw_os_error(self.0))
        }
    }

    #[test]
    fn a_driver_that_refuses_everything_still_gives_every_line() {
        let lines = probe(&Refuses(EINVAL));
        assert_eq!(lines.len(), 1 + RATES.len());
        assert!(lines[0].starts_with("any rate: asking failed"));
        assert!(lines[1..].iter().all(|l| l.ends_with("Hz: no")));
        // Any other error is reported as a failed question, not as a no.
        let lines = probe(&Refuses(5));
        assert!(lines[1..].iter().all(|l| l.contains("asking failed")));
    }

    #[test]
    fn a_missing_device_is_one_line_and_no_panic() {
        let lines = probe_path("/dev/snd/pcmC99D0p-does-not-exist");
        assert_eq!(lines.len(), 2);
        assert!(lines[1].starts_with("  not opened:"), "{lines:?}");
    }

    #[test]
    fn a_device_that_is_not_a_sound_card_is_asked_closed_and_reported() {
        // /dev/null opens; the questions fail (not a PCM device, or not the
        // device ABI on the workstation), and it is closed again.
        let lines = probe_path("/dev/null");
        assert_eq!(lines.len(), 1 + 1 + RATES.len() + 1, "{lines:?}");
        assert_eq!(lines.last().unwrap(), "  closed");
        assert!(!lines.iter().any(|l| l.contains("yes")), "{lines:?}");
    }

    /// The safety rule as a check on the source: outside the tests, this
    /// file names no call that sets up, starts, reads or writes a stream.
    #[test]
    fn the_probe_names_no_call_that_changes_a_stream() {
        let src = include_str!("rate_probe.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        for call in [
            ".hw_params(",
            ".sw_params(",
            ".prepare(",
            ".start(",
            ".drain(",
            ".drop_stream(",
            ".write_frames(",
            ".read_frames(",
            "PCM_HW_PARAMS",
            "PCM_PREPARE",
            "PCM_START",
            "open_playback",
            "open_capture",
        ] {
            assert!(!code.contains(call), "the probe must not call {call}");
        }
    }
}
