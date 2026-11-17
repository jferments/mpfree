//! What each port's channels hold now, for the Monitor's Now view: the
//! notes down with their velocity and pressure, the last value of each
//! controller that moved, bend, channel pressure and program ([`Held`]),
//! and how steady a clock's pulses come ([`Pulses`]).
//!
//! [`Held`] is kept by the [`super::Monitor`] from the messages it lists,
//! so a message a [`super::Feed`] had no room for is missed here too (the
//! list says how many). [`Pulses`] is measured by the [`super::Feed`] at
//! every clock pulse, since the list counts the clock rather than listing
//! it. Neither allocates once made.

use crate::msg::Msg;

use super::To;

/// A time not yet seen.
pub const NEVER: f32 = f32::NEG_INFINITY;
/// [`Held::by`] when no track sent the note.
pub const NO_TRACK: u8 = 0xFF;
/// A note held this long with no note-off is called stuck (seconds).
pub const STUCK_S: f32 = 30.0;
/// A gap longer than this many pulses counts as lost or late pulses.
pub const LATE: f32 = 1.5;
/// How many seconds of worst gaps [`Pulses`] keeps.
pub const SECONDS: usize = 60;

/// One channel of one port, one way: what it holds now.
#[derive(Clone, Debug, PartialEq)]
pub struct Held {
    /// Each note's velocity while it is down (0: up).
    pub vel: [u8; 128],
    /// Each note's pressure (polyphonic aftertouch) while it is down.
    pub pressure: [u8; 128],
    /// When each note went down (the app's clock, seconds).
    pub down_at: [f32; 128],
    /// The track that sent each note out ([`NO_TRACK`]: none, or in).
    pub by: [u8; 128],
    /// Each controller's last value, and when it moved ([`NEVER`]: not
    /// yet).
    pub cc: [u8; 128],
    pub cc_at: [f32; 128],
    /// Pitch bend, -8192 to 8191, and when it moved.
    pub bend: i16,
    pub bend_at: f32,
    /// Channel pressure and when it moved.
    pub chan_pressure: u8,
    pub chan_pressure_at: f32,
    /// The last program change (0 to 127) and when it came.
    pub program: u8,
    pub program_at: f32,
    /// Where the newest note on or off went.
    pub note_to: To,
}

impl Default for Held {
    fn default() -> Self {
        Self::new()
    }
}

impl Held {
    pub fn new() -> Held {
        Held {
            vel: [0; 128],
            pressure: [0; 128],
            down_at: [NEVER; 128],
            by: [NO_TRACK; 128],
            cc: [0; 128],
            cc_at: [NEVER; 128],
            bend: 0,
            bend_at: NEVER,
            chan_pressure: 0,
            chan_pressure_at: NEVER,
            program: 0,
            program_at: NEVER,
            note_to: To::Unsaid,
        }
    }

    /// A message on this channel at `t`, which went `to`.
    pub fn take(&mut self, msg: &Msg, t: f64, to: To) {
        let t = t as f32;
        match *msg {
            Msg::NoteOn { note, vel, .. } => {
                let n = usize::from(note & 0x7F);
                self.vel[n] = vel.max(1);
                self.pressure[n] = 0;
                self.down_at[n] = t;
                self.by[n] = match to {
                    To::SentBy(track) => track,
                    _ => NO_TRACK,
                };
                self.note_to = to;
            }
            Msg::NoteOff { note, .. } => {
                let n = usize::from(note & 0x7F);
                self.vel[n] = 0;
                self.pressure[n] = 0;
                self.note_to = to;
            }
            Msg::PolyPressure { note, value, .. } => {
                let n = usize::from(note & 0x7F);
                if self.vel[n] > 0 {
                    self.pressure[n] = value;
                }
            }
            Msg::Cc { cc, value, .. } => {
                let c = usize::from(cc & 0x7F);
                self.cc[c] = value;
                self.cc_at[c] = t;
                match cc {
                    // All sound off, all notes off (and the mode
                    // messages, which end notes too).
                    120 | 123..=127 => self.all_up(),
                    // Reset all controllers.
                    121 => {
                        self.bend = 0;
                        self.chan_pressure = 0;
                        self.pressure = [0; 128];
                    }
                    _ => {}
                }
            }
            Msg::Program { program, .. } => {
                self.program = program;
                self.program_at = t;
            }
            Msg::ChanPressure { value, .. } => {
                self.chan_pressure = value;
                self.chan_pressure_at = t;
            }
            Msg::PitchBend { value, .. } => {
                self.bend = value;
                self.bend_at = t;
            }
        }
    }

    /// Every note up.
    pub fn all_up(&mut self) {
        self.vel = [0; 128];
        self.pressure = [0; 128];
    }

    /// The notes down, lowest first.
    pub fn down(&self) -> impl Iterator<Item = u8> + '_ {
        (0..128u8).filter(|&n| self.vel[usize::from(n)] > 0)
    }

    /// How many notes are down.
    pub fn count(&self) -> usize {
        self.vel.iter().filter(|&&v| v > 0).count()
    }

    /// The sustain pedal (CC 64), once it has moved: down or not.
    pub fn sustain(&self) -> Option<bool> {
        (self.cc_at[64] > NEVER).then_some(self.cc[64] >= 64)
    }

    /// The bank (CC 0 and CC 32), once either has come.
    pub fn bank(&self) -> Option<(u8, u8)> {
        (self.cc_at[0] > NEVER || self.cc_at[32] > NEVER).then_some((self.cc[0], self.cc[32]))
    }

    /// Each controller that moved, with its value and when, newest first.
    /// Not for the audio loop: it allocates.
    pub fn moved(&self) -> Vec<(u8, u8, f32)> {
        let mut v: Vec<(u8, u8, f32)> = (0..128u8)
            .filter(|&c| self.cc_at[usize::from(c)] > NEVER)
            .map(|c| (c, self.cc[usize::from(c)], self.cc_at[usize::from(c)]))
            .collect();
        v.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
        v
    }

    /// Whether anything has come on this channel.
    pub fn is_empty(&self) -> bool {
        self.count() == 0
            && self.cc_at.iter().all(|&t| t == NEVER)
            && self.bend_at == NEVER
            && self.chan_pressure_at == NEVER
            && self.program_at == NEVER
            && self.down_at.iter().all(|&t| t == NEVER)
    }
}

/// How steady a clock's pulses come: measured at each pulse, by the
/// second. Times are when the pulses were read, so on the MIDI jacks each
/// is within a sound period (about 2.9 ms) of when it arrived.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pulses {
    /// The last full second's mean gap between pulses and half the range
    /// of its gaps, in ms; 0 until a second has passed.
    pub gap_ms: f32,
    pub spread_ms: f32,
    /// Pulses lost or late since the clock started: a gap of more than
    /// [`LATE`] pulses counts the pulses that would fit in it, at least
    /// one.
    pub late: u32,
    worst: [f32; SECONDS],
    /// Seconds kept in `worst` (at most [`SECONDS`]) and where the next
    /// goes.
    kept: u8,
    next: u8,
    last: f64,
    from: f64,
    n: u32,
    sum: f64,
    min: f32,
    max: f32,
}

impl Default for Pulses {
    fn default() -> Self {
        Self::new()
    }
}

impl Pulses {
    pub fn new() -> Pulses {
        Pulses {
            gap_ms: 0.0,
            spread_ms: 0.0,
            late: 0,
            worst: [0.0; SECONDS],
            kept: 0,
            next: 0,
            last: f64::NEG_INFINITY,
            from: f64::NEG_INFINITY,
            n: 0,
            sum: 0.0,
            min: f32::INFINITY,
            max: 0.0,
        }
    }

    /// The clock started arriving: its first pulse came at `t`.
    pub fn start(&mut self, t: f64) {
        *self = Pulses::new();
        self.last = t;
        self.from = t;
    }

    /// A pulse at `t` (seconds).
    pub fn tick(&mut self, t: f64) {
        if !self.last.is_finite() {
            self.start(t);
            return;
        }
        let gap = ((t - self.last) * 1000.0) as f32;
        self.last = t;
        if self.gap_ms > 0.0 && gap > LATE * self.gap_ms {
            let fit = (gap / self.gap_ms).round() as u32;
            self.late = self.late.saturating_add(fit.saturating_sub(1).max(1));
        }
        self.n += 1;
        self.sum += f64::from(gap);
        self.min = self.min.min(gap);
        self.max = self.max.max(gap);
        if t - self.from >= 1.0 {
            self.gap_ms = (self.sum / f64::from(self.n)) as f32;
            self.spread_ms = (self.max - self.min) / 2.0;
            self.worst[usize::from(self.next)] = self.max;
            self.next = ((usize::from(self.next) + 1) % SECONDS) as u8;
            self.kept = (usize::from(self.kept) + 1).min(SECONDS) as u8;
            self.from = t;
            self.n = 0;
            self.sum = 0.0;
            self.min = f32::INFINITY;
            self.max = 0.0;
        }
    }

    /// Each full second's worst gap in ms, oldest first (the last minute).
    pub fn seconds(&self) -> impl Iterator<Item = f32> + '_ {
        let kept = usize::from(self.kept);
        let first = (usize::from(self.next) + SECONDS - kept) % SECONDS;
        (0..kept).map(move |i| self.worst[(first + i) % SECONDS])
    }

    /// The worst gap of the last minute, in ms, once a second has passed.
    pub fn worst(&self) -> Option<f32> {
        self.seconds().reduce(f32::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_controls_bend_and_program_are_held_as_they_come() {
        let mut h = Held::new();
        assert!(h.is_empty());
        let to = To::Track {
            track: 1,
            pad: None,
        };
        h.take(
            &Msg::NoteOn {
                ch: 0,
                note: 60,
                vel: 96,
            },
            1.0,
            to,
        );
        h.take(
            &Msg::NoteOn {
                ch: 0,
                note: 64,
                vel: 88,
            },
            1.1,
            to,
        );
        h.take(
            &Msg::PolyPressure {
                ch: 0,
                note: 64,
                value: 34,
            },
            1.2,
            to,
        );
        h.take(
            &Msg::Cc {
                ch: 0,
                cc: 74,
                value: 88,
            },
            1.3,
            To::Nothing,
        );
        h.take(
            &Msg::Cc {
                ch: 0,
                cc: 1,
                value: 47,
            },
            1.4,
            To::Nothing,
        );
        h.take(&Msg::PitchBend { ch: 0, value: 2048 }, 1.5, To::Nothing);
        h.take(&Msg::Program { ch: 0, program: 11 }, 1.6, To::Nothing);
        assert_eq!(h.down().collect::<Vec<_>>(), [60, 64]);
        assert_eq!((h.vel[64], h.pressure[64]), (88, 34));
        assert_eq!(h.note_to, to);
        // Newest first.
        assert_eq!(h.moved(), [(1, 47, 1.4), (74, 88, 1.3)]);
        assert_eq!((h.bend, h.program), (2048, 11));
        assert_eq!(h.sustain(), None);
        assert_eq!(h.bank(), None);
        h.take(
            &Msg::NoteOff {
                ch: 0,
                note: 60,
                vel: 64,
            },
            2.0,
            to,
        );
        assert_eq!(h.count(), 1);
        h.take(
            &Msg::Cc {
                ch: 0,
                cc: 64,
                value: 127,
            },
            2.1,
            To::Nothing,
        );
        assert_eq!(h.sustain(), Some(true));
        // All notes off ends the rest.
        h.take(
            &Msg::Cc {
                ch: 0,
                cc: 123,
                value: 0,
            },
            2.2,
            To::Nothing,
        );
        assert_eq!(h.count(), 0);
        assert!(!h.is_empty());
    }

    #[test]
    fn a_note_sent_out_keeps_its_track() {
        let mut h = Held::new();
        let on = Msg::NoteOn {
            ch: 1,
            note: 36,
            vel: 110,
        };
        h.take(&on, 3.0, To::SentBy(1));
        assert_eq!((h.by[36], h.down_at[36]), (1, 3.0));
        h.take(&on, 3.5, To::Clock);
        assert_eq!(h.by[36], NO_TRACK);
    }

    #[test]
    fn pulses_say_their_gap_spread_worst_second_and_lost_ones() {
        let mut p = Pulses::new();
        // 120 BPM: a pulse every 1/48 s, read on 2.9 ms periods.
        let period: f64 = 128.0 / 44_100.0;
        let mut t = 0.0;
        let mut n = 0;
        while t < 2.0 {
            let read = (t / period).ceil() * period;
            p.tick(read);
            t += 1.0 / 48.0;
            n += 1;
        }
        assert!(n > 90);
        assert!((p.gap_ms - 20.83).abs() < 0.2, "{}", p.gap_ms);
        assert!(p.spread_ms > 0.0 && p.spread_ms <= 1.5, "{}", p.spread_ms);
        assert_eq!(p.late, 0);
        assert!((1..=2).contains(&p.seconds().count()));
        // Two pulses lost: a gap of three.
        p.tick(t + 2.0 / 48.0);
        assert_eq!(p.late, 2);
        let worst = p.worst().unwrap();
        assert!(worst < 25.0, "{worst}");
        // A minute and more keeps the last 60 seconds.
        for s in 0..70 {
            for k in 0..48 {
                p.tick(t + 1.0 + f64::from(s) + f64::from(k) / 48.0);
            }
        }
        assert_eq!(p.seconds().count(), SECONDS);
        // Started again, it forgets.
        p.start(100.0);
        assert_eq!((p.late, p.seconds().count(), p.gap_ms), (0, 0, 0.0));
    }
}
