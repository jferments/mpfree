//! MIDI 1.0 byte-stream parser for a single RawMIDI substream.
//!
//! Real-time-safe: fixed-size state, no allocation, O(1) per byte.

/// Largest SysEx (including F0 and F7) kept intact. The MPC One messages
/// seen so far are at most 36 bytes; longer ones are reported truncated.
pub const SYSEX_CAPACITY: usize = 64;

/// A complete message. SysEx borrows the parser's buffer until the next feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Message<'a> {
    /// Channel voice message. `data2` is 0 for 2-byte messages
    /// (program change, channel pressure).
    Channel { status: u8, data1: u8, data2: u8 },
    /// System exclusive including the leading F0 and trailing F7.
    /// `truncated` is true if the message exceeded [`SYSEX_CAPACITY`]; then
    /// `bytes` holds the first `SYSEX_CAPACITY - 1` bytes plus F7.
    SysEx { bytes: &'a [u8], truncated: bool },
    /// System real-time byte (F8..=FF), delivered immediately.
    RealTime(u8),
    /// System common message (F1..=F6), data bytes zero-filled.
    Common { status: u8, data1: u8, data2: u8 },
}

#[derive(Debug, Clone)]
pub struct Parser {
    running: u8,
    data: [u8; 2],
    have: u8,
    need: u8,
    in_sysex: bool,
    sysex: [u8; SYSEX_CAPACITY],
    sysex_len: usize,
    sysex_truncated: bool,
    /// Data bytes dropped because no status byte preceded them.
    pub stray_bytes: u32,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

fn channel_data_len(status: u8) -> u8 {
    match status & 0xF0 {
        0xC0 | 0xD0 => 1,
        _ => 2,
    }
}

fn common_data_len(status: u8) -> u8 {
    match status {
        0xF1 | 0xF3 => 1,
        0xF2 => 2,
        _ => 0, // F4, F5 undefined; F6 tune request
    }
}

impl Parser {
    pub const fn new() -> Self {
        Self {
            running: 0,
            data: [0; 2],
            have: 0,
            need: 0,
            in_sysex: false,
            sysex: [0; SYSEX_CAPACITY],
            sysex_len: 0,
            sysex_truncated: false,
            stray_bytes: 0,
        }
    }

    /// Feed one byte; returns a message when one completes.
    pub fn feed(&mut self, byte: u8) -> Option<Message<'_>> {
        if byte >= 0xF8 {
            return Some(Message::RealTime(byte));
        }
        if byte == 0xF0 {
            self.in_sysex = true;
            self.sysex[0] = 0xF0;
            self.sysex_len = 1;
            self.sysex_truncated = false;
            self.running = 0;
            return None;
        }
        if byte == 0xF7 {
            if !self.in_sysex {
                self.stray_bytes = self.stray_bytes.saturating_add(1);
                return None;
            }
            self.in_sysex = false;
            let end = if self.sysex_len < SYSEX_CAPACITY {
                self.sysex_len
            } else {
                SYSEX_CAPACITY - 1
            };
            self.sysex[end] = 0xF7;
            return Some(Message::SysEx {
                bytes: &self.sysex[..=end],
                truncated: self.sysex_truncated,
            });
        }
        if byte & 0x80 != 0 {
            // Any other status byte ends an unterminated SysEx (discarded).
            self.in_sysex = false;
            self.have = 0;
            if byte >= 0xF1 {
                self.running = 0;
                self.need = common_data_len(byte);
                if self.need == 0 {
                    return Some(Message::Common {
                        status: byte,
                        data1: 0,
                        data2: 0,
                    });
                }
                self.data = [0; 2];
                self.running = byte; // held only until this message completes
            } else {
                self.running = byte;
                self.need = channel_data_len(byte);
            }
            return None;
        }
        // Data byte.
        if self.in_sysex {
            if self.sysex_len < SYSEX_CAPACITY - 1 {
                self.sysex[self.sysex_len] = byte;
                self.sysex_len += 1;
            } else {
                self.sysex_truncated = true;
            }
            return None;
        }
        if self.running == 0 {
            self.stray_bytes = self.stray_bytes.saturating_add(1);
            return None;
        }
        self.data[self.have as usize] = byte;
        self.have += 1;
        if self.have < self.need {
            return None;
        }
        self.have = 0;
        let status = self.running;
        let (d1, d2) = (self.data[0], if self.need == 2 { self.data[1] } else { 0 });
        if status >= 0xF1 {
            self.running = 0; // system common has no running status
            return Some(Message::Common {
                status,
                data1: d1,
                data2: d2,
            });
        }
        Some(Message::Channel {
            status,
            data1: d1,
            data2: d2,
        })
    }
}
