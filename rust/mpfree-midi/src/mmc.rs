//! MIDI Machine Control coming in: other gear's transport (a DAW, a tape
//! deck, a hard disk recorder) as the MMC SysEx it sends, read only.
//!
//! An MMC command is a universal real-time SysEx, `F0 7F <device> 06
//! <command> … F7`. MPFree reads the ones the MPC's own Sync settings
//! take ("Receive MMC", the MPC Standalone 3.10.1 guide's appendix, MIDI
//! Machine Control): Deferred Play and Play, Stop, Pause, Record Strobe
//! and Locate. Any device id is taken, the all-call 7F too. MPFree never
//! sends MMC (it would be SysEx on the jacks).
//!
//! [`Collect`] keeps one port's SysEx as it arrives, in fixed room, so
//! the bytes can be read once its F7 comes; [`parse`] reads them.

/// The longest SysEx kept to be read: an MMC Locate is 13 bytes.
pub const KEEP: usize = 16;

/// One port's SysEx as it arrives, in fixed room: a SysEx longer than
/// [`KEEP`] is let pass unread. Never allocates.
#[derive(Clone, Copy, Debug, Default)]
pub struct Collect {
    buf: [u8; KEEP],
    len: usize,
    /// Inside a SysEx (after its F0).
    on: bool,
    /// Longer than `KEEP`: not kept.
    over: bool,
}

impl Collect {
    /// One byte as it came in. The whole SysEx, F0 to F7, once its F7
    /// ends one that fit. Real-time bytes (F8 to FF) can come inside a
    /// SysEx and are left out; any other status byte ends it unread.
    pub fn byte(&mut self, b: u8) -> Option<&[u8]> {
        match b {
            0xF0 => {
                self.on = true;
                self.over = false;
                self.buf[0] = b;
                self.len = 1;
                None
            }
            0xF8..=0xFF => None,
            0xF7 if self.on => {
                self.on = false;
                if self.over || self.len >= KEEP {
                    return None;
                }
                self.buf[self.len] = b;
                self.len += 1;
                Some(&self.buf[..self.len])
            }
            0x80..=0xF7 => {
                self.on = false;
                None
            }
            _ if self.on => {
                if self.len < KEEP {
                    self.buf[self.len] = b;
                    self.len += 1;
                } else {
                    self.over = true;
                }
                None
            }
            _ => None,
        }
    }
}

/// An MMC command MPFree reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mmc {
    Stop,
    Play,
    /// Play once the deck is where it was told to go: MPFree plays at
    /// once, as the MPC does.
    DeferredPlay,
    /// Punch in.
    RecordStrobe,
    /// Punch out.
    RecordExit,
    Pause,
    /// Go to a time ([`Smpte`]).
    Locate(Smpte),
    /// Any other command (fast forward, rewind, eject…), by its number.
    Other(u8),
}

impl Mmc {
    /// The command's name, for the log.
    pub fn name(self) -> String {
        match self {
            Mmc::Stop => "MMC Stop".into(),
            Mmc::Play => "MMC Play".into(),
            Mmc::DeferredPlay => "MMC Deferred Play".into(),
            Mmc::RecordStrobe => "MMC Record Strobe".into(),
            Mmc::RecordExit => "MMC Record Exit".into(),
            Mmc::Pause => "MMC Pause".into(),
            Mmc::Locate(at) => format!("MMC Locate {at}"),
            Mmc::Other(c) => format!("MMC command {c:02X}"),
        }
    }
}

/// A time code's frame rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rate {
    Fps24,
    Fps25,
    /// 29.97 frames a second, drop frame.
    Fps30Drop,
    Fps30,
}

/// A time as hours, minutes, seconds, frames and hundredths of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Smpte {
    pub rate: Rate,
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
    pub frames: u8,
    pub subframes: u8,
}

impl Smpte {
    /// The time from 00:00:00:00, in seconds. Drop frame counts the
    /// frames its numbers skip (two each minute but every tenth) and runs
    /// at 30000/1001 frames a second.
    pub fn secs(self) -> f64 {
        let whole =
            u32::from(self.hours) * 3600 + u32::from(self.minutes) * 60 + u32::from(self.seconds);
        let part = f64::from(self.frames) + f64::from(self.subframes) / 100.0;
        match self.rate {
            Rate::Fps24 => f64::from(whole) + part / 24.0,
            Rate::Fps25 => f64::from(whole) + part / 25.0,
            Rate::Fps30 => f64::from(whole) + part / 30.0,
            Rate::Fps30Drop => {
                let minutes = u32::from(self.hours) * 60 + u32::from(self.minutes);
                let dropped = 2 * (minutes - minutes / 10);
                let frames = f64::from(whole * 30) - f64::from(dropped) + part;
                frames * 1001.0 / 30000.0
            }
        }
    }
}

impl std::fmt::Display for Smpte {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sep = if self.rate == Rate::Fps30Drop {
            ';'
        } else {
            ':'
        };
        write!(
            f,
            "{:02}:{:02}:{:02}{sep}{:02}",
            self.hours, self.minutes, self.seconds, self.frames
        )
    }
}

/// The MMC command in a whole SysEx (F0 to F7), if it is one: `F0 7F
/// <device> 06 <command> … F7`. A Locate needs its target, `44 06 01 hr
/// mn sc fr ff` (hr's bits 5 and 6 the rate); one without is `None`.
pub fn parse(sysex: &[u8]) -> Option<Mmc> {
    let [0xF0, 0x7F, _device, 0x06, command, rest @ ..] = sysex else {
        return None;
    };
    if rest.last() != Some(&0xF7) {
        return None;
    }
    Some(match *command {
        0x01 => Mmc::Stop,
        0x02 => Mmc::Play,
        0x03 => Mmc::DeferredPlay,
        0x06 => Mmc::RecordStrobe,
        0x07 => Mmc::RecordExit,
        0x09 => Mmc::Pause,
        0x44 => {
            let [0x06, 0x01, hr, mn, sc, fr, ff, 0xF7] = *rest else {
                return None;
            };
            let rate = match (hr >> 5) & 0x03 {
                0 => Rate::Fps24,
                1 => Rate::Fps25,
                2 => Rate::Fps30Drop,
                _ => Rate::Fps30,
            };
            let at = Smpte {
                rate,
                hours: hr & 0x1F,
                minutes: mn.min(59),
                seconds: sc.min(59),
                frames: fr & 0x1F,
                subframes: ff.min(99),
            };
            Mmc::Locate(at)
        }
        c => Mmc::Other(c),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut c = Collect::default();
        bytes
            .iter()
            .filter_map(|&b| c.byte(b).map(<[u8]>::to_vec))
            .collect()
    }

    /// The commands the MPC takes, from any device id.
    #[test]
    fn reads_the_transports_commands() {
        for (cmd, want) in [
            (0x01, Mmc::Stop),
            (0x02, Mmc::Play),
            (0x03, Mmc::DeferredPlay),
            (0x06, Mmc::RecordStrobe),
            (0x07, Mmc::RecordExit),
            (0x09, Mmc::Pause),
            (0x04, Mmc::Other(0x04)),
        ] {
            for dev in [0x7F, 0x00, 0x10] {
                assert_eq!(parse(&[0xF0, 0x7F, dev, 0x06, cmd, 0xF7]), Some(want));
            }
        }
        // Not MMC: a non-real-time universal SysEx, a response (07), a
        // maker's own, a cut-off one.
        assert_eq!(parse(&[0xF0, 0x7E, 0x7F, 0x06, 0x02, 0xF7]), None);
        assert_eq!(parse(&[0xF0, 0x7F, 0x7F, 0x07, 0x02, 0xF7]), None);
        assert_eq!(parse(&[0xF0, 0x47, 0x7F, 0x06, 0x02, 0xF7]), None);
        assert_eq!(parse(&[0xF0, 0x7F, 0x7F, 0x06, 0x02]), None);
    }

    /// Locate reads its time at each rate; Locate Zero is 0 s.
    #[test]
    fn locate_reads_its_time() {
        let locate = |hr: u8, mn, sc, fr, ff| {
            parse(&[
                0xF0, 0x7F, 0x7F, 0x06, 0x44, 0x06, 0x01, hr, mn, sc, fr, ff, 0xF7,
            ])
        };
        let Some(Mmc::Locate(zero)) = locate(0x20, 0, 0, 0, 0) else {
            panic!("locate zero");
        };
        assert_eq!((zero.rate, zero.secs()), (Rate::Fps25, 0.0));
        let Some(Mmc::Locate(at)) = locate(0x21, 2, 3, 5, 50) else {
            panic!("25 fps");
        };
        assert_eq!(at.hours, 1);
        assert!((at.secs() - (3600.0 + 123.0 + 5.5 / 25.0)).abs() < 1e-9);
        assert_eq!(at.to_string(), "01:02:03:05");
        let Some(Mmc::Locate(at)) = locate(0x60, 0, 10, 15, 0) else {
            panic!("30 fps");
        };
        assert!((at.secs() - 10.5).abs() < 1e-9);
        let Some(Mmc::Locate(at)) = locate(0x00, 0, 1, 12, 0) else {
            panic!("24 fps");
        };
        assert!((at.secs() - 1.5).abs() < 1e-9);
        // Drop frame: 00:01:00;02 is the first frame of minute 1, frame
        // 1800 at 29.97 (1800 × 1.001 / 30 s).
        let Some(Mmc::Locate(at)) = locate(0x40, 1, 0, 2, 0) else {
            panic!("drop frame");
        };
        assert_eq!(at.rate, Rate::Fps30Drop);
        assert!((at.secs() - 1800.0 * 1001.0 / 30000.0).abs() < 1e-9);
        assert_eq!(at.to_string(), "00:01:00;02");
        // A Locate without its target is nothing.
        assert_eq!(
            parse(&[0xF0, 0x7F, 0x7F, 0x06, 0x44, 0x06, 0x01, 0x20, 0xF7]),
            None
        );
    }

    /// The SysEx is kept as it arrives: clock bytes inside it are left out,
    /// another status byte ends it unread, and one too long passes.
    #[test]
    fn collects_a_sysex_in_fixed_room() {
        let play = [0xF0, 0x7F, 0x7F, 0x06, 0x02, 0xF7];
        assert_eq!(collect(&play), [play.to_vec()]);
        assert_eq!(
            collect(&[0xF0, 0x7F, 0xF8, 0x7F, 0x06, 0xFE, 0x02, 0xF7]),
            [play.to_vec()]
        );
        assert!(collect(&[0xF0, 0x7F, 0x90, 0x7F, 0x06, 0x02, 0xF7]).is_empty());
        let mut long = vec![0xF0];
        long.extend([0x01; KEEP]);
        long.push(0xF7);
        assert!(collect(&long).is_empty());
        // The next one after it is read.
        long.extend(play);
        assert_eq!(collect(&long), [play.to_vec()]);
        // A stray F7 is nothing.
        assert!(collect(&[0xF7, 0x02]).is_empty());
    }
}
