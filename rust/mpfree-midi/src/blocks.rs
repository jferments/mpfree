//! Blocks on a route (board 5 of the MIDI powerhouse mockups, approved
//! 16 Nov 2026; plan item 3.2): what a route does to the messages it
//! takes, in order, before they play a track or go thru. A route (a port's
//! notes to the armed track, a port and channel to a track, thru to an
//! output) has up to [`MAX_BLOCKS`]:
//!
//! - **Filter** drops the kinds it names (notes, pressure, bend, controls,
//!   program changes).
//! - **Channel** sends them on one channel (for thru: the gear's).
//! - **Split** passes only the notes from its lowest to its highest (and
//!   their pressure): the rest of the keyboard is free for another route,
//!   or the armed track.
//! - **Transpose** moves notes by semitones; one pushed off the keyboard
//!   is dropped.
//! - **Velocity** shapes a note's velocity with a curve, then fits it
//!   between a lowest and a highest, or plays every note at one velocity.
//!
//! [`Blocks::apply`] runs a message through them: the message as it
//! leaves, or None when a block drops it. A note-off goes through the same
//! blocks as its note-on, so it ends what the note-on started; the input
//! router lets go of a route's held keys before its blocks change
//! ([`crate::input::InputRouter`]). Nothing here allocates.
//!
//! As text (the project's MIDI part, after the route's line):
//! `filter program bend; transpose -12; velocity soft 20 127`.

use crate::msg::{Channel, Msg};

/// The most blocks one route has.
pub const MAX_BLOCKS: usize = 4;

/// A kind of message a Filter can drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Notes,
    Pressure,
    Bend,
    Controls,
    Program,
}

impl Kind {
    pub const ALL: [Kind; 5] = [
        Kind::Notes,
        Kind::Pressure,
        Kind::Bend,
        Kind::Controls,
        Kind::Program,
    ];

    /// Its word on screen and in the text.
    pub fn word(self) -> &'static str {
        match self {
            Kind::Notes => "notes",
            Kind::Pressure => "pressure",
            Kind::Bend => "bend",
            Kind::Controls => "controls",
            Kind::Program => "program",
        }
    }

    fn bit(self) -> u8 {
        1 << (self as u8)
    }

    /// The kind `m` is.
    pub fn of(m: &Msg) -> Kind {
        match m {
            Msg::NoteOn { .. } | Msg::NoteOff { .. } => Kind::Notes,
            Msg::PolyPressure { .. } | Msg::ChanPressure { .. } => Kind::Pressure,
            Msg::PitchBend { .. } => Kind::Bend,
            Msg::Cc { .. } => Kind::Controls,
            Msg::Program { .. } => Kind::Program,
        }
    }
}

/// The kinds a Filter drops.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Kinds(u8);

impl Kinds {
    pub const NONE: Kinds = Kinds(0);

    pub fn has(self, k: Kind) -> bool {
        self.0 & k.bit() != 0
    }

    pub fn with(self, k: Kind, on: bool) -> Kinds {
        if on {
            Kinds(self.0 | k.bit())
        } else {
            Kinds(self.0 & !k.bit())
        }
    }

    /// Its kinds, in [`Kind::ALL`]'s order.
    pub fn iter(self) -> impl Iterator<Item = Kind> {
        Kind::ALL.into_iter().filter(move |k| self.has(*k))
    }
}

/// How Velocity shapes a note's velocity before fitting it in its range.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Curve {
    /// Softer playing comes out louder.
    Soft,
    #[default]
    Linear,
    /// It takes a harder hit to sound loud.
    Hard,
}

impl Curve {
    pub const ALL: [Curve; 3] = [Curve::Soft, Curve::Linear, Curve::Hard];

    pub fn word(self) -> &'static str {
        match self {
            Curve::Soft => "soft",
            Curve::Linear => "linear",
            Curve::Hard => "hard",
        }
    }

    /// `v` (1 to 127) shaped, as a share of full (0 to 1).
    fn shape(self, v: u8) -> f32 {
        let x = f32::from(v.clamp(1, 127)) / 127.0;
        match self {
            Curve::Soft => x.sqrt(),
            Curve::Linear => x,
            Curve::Hard => x * x,
        }
    }
}

/// One block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    /// Drops these kinds.
    Filter(Kinds),
    /// Everything leaves on this channel (0 to 15).
    Channel(Channel),
    /// Only notes from `low` to `high` pass, with their pressure.
    Split { low: u8, high: u8 },
    /// Notes move this many semitones (within [`MAX_TRANSPOSE`]).
    Transpose(i8),
    /// A note-on's velocity: shaped by `curve`, then fitted from `low` to
    /// `high`; or always `fixed`.
    Velocity {
        curve: Curve,
        low: u8,
        high: u8,
        fixed: Option<u8>,
    },
}

/// The furthest Transpose goes either way, in semitones.
pub const MAX_TRANSPOSE: i8 = 48;

/// The kinds of block, in "+ Block"'s order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Filter,
    Channel,
    Split,
    Transpose,
    Velocity,
}

impl BlockKind {
    pub const ALL: [BlockKind; 5] = [
        BlockKind::Filter,
        BlockKind::Channel,
        BlockKind::Split,
        BlockKind::Transpose,
        BlockKind::Velocity,
    ];

    /// Its name on its tile.
    pub fn name(self) -> &'static str {
        match self {
            BlockKind::Filter => "Filter",
            BlockKind::Channel => "Channel",
            BlockKind::Split => "Split",
            BlockKind::Transpose => "Transpose",
            BlockKind::Velocity => "Velocity",
        }
    }

    /// A new block of this kind, doing nothing yet where it can (a Filter
    /// that drops program changes, as the board's example).
    pub fn new_block(self) -> Block {
        match self {
            BlockKind::Filter => Block::Filter(Kinds::NONE.with(Kind::Program, true)),
            BlockKind::Channel => Block::Channel(0),
            BlockKind::Split => Block::Split { low: 0, high: 127 },
            BlockKind::Transpose => Block::Transpose(0),
            BlockKind::Velocity => Block::Velocity {
                curve: Curve::Linear,
                low: 1,
                high: 127,
                fixed: None,
            },
        }
    }
}

impl Block {
    pub fn kind(&self) -> BlockKind {
        match self {
            Block::Filter(_) => BlockKind::Filter,
            Block::Channel(_) => BlockKind::Channel,
            Block::Split { .. } => BlockKind::Split,
            Block::Transpose(_) => BlockKind::Transpose,
            Block::Velocity { .. } => BlockKind::Velocity,
        }
    }

    /// Kept in range: a channel 0 to 15, a split's ends in order, a
    /// transpose within [`MAX_TRANSPOSE`], velocities 1 to 127 with the
    /// lowest at most the highest.
    pub fn tidy(self) -> Block {
        match self {
            Block::Filter(k) => Block::Filter(k),
            Block::Channel(c) => Block::Channel(c.min(15)),
            Block::Split { low, high } => {
                let (low, high) = (low.min(127), high.min(127));
                Block::Split {
                    low: low.min(high),
                    high: high.max(low),
                }
            }
            Block::Transpose(t) => Block::Transpose(t.clamp(-MAX_TRANSPOSE, MAX_TRANSPOSE)),
            Block::Velocity {
                curve,
                low,
                high,
                fixed,
            } => {
                let (low, high) = (low.clamp(1, 127), high.clamp(1, 127));
                Block::Velocity {
                    curve,
                    low: low.min(high),
                    high: high.max(low),
                    fixed: fixed.map(|f| f.clamp(1, 127)),
                }
            }
        }
    }

    /// `m` through this block: as it leaves, or None when dropped.
    pub fn apply(&self, m: Msg) -> Option<Msg> {
        match *self {
            Block::Filter(drop) => (!drop.has(Kind::of(&m))).then_some(m),
            Block::Channel(c) => Some(m.on_channel(c & 0x0F)),
            Block::Split { low, high } => match m.note() {
                Some(n) if n < low || n > high => None,
                _ => Some(m),
            },
            Block::Transpose(t) => {
                if m.note().is_some() {
                    m.transposed(t)
                } else {
                    Some(m)
                }
            }
            Block::Velocity {
                curve,
                low,
                high,
                fixed,
            } => match m {
                Msg::NoteOn { ch, note, vel } if vel > 0 => {
                    let vel = match fixed {
                        Some(f) => f.clamp(1, 127),
                        None => {
                            let (lo, hi) = (f32::from(low.min(high)), f32::from(high.max(low)));
                            (lo + (hi - lo) * curve.shape(vel))
                                .round()
                                .clamp(1.0, 127.0) as u8
                        }
                    };
                    Some(Msg::NoteOn { ch, note, vel })
                }
                other => Some(other),
            },
        }
    }

    /// As text: `filter program bend`, `channel 10`, `split C2 B3`,
    /// `transpose -12`, `velocity soft 20 127` or `velocity fixed 100`.
    pub fn to_text(&self) -> String {
        match *self {
            Block::Filter(drop) => {
                let mut s = String::from("filter");
                for k in drop.iter() {
                    s.push(' ');
                    s.push_str(k.word());
                }
                s
            }
            Block::Channel(c) => format!("channel {}", c + 1),
            Block::Split { low, high } => format!("split {low} {high}"),
            Block::Transpose(t) => format!("transpose {t}"),
            Block::Velocity {
                curve,
                low,
                high,
                fixed,
            } => match fixed {
                Some(f) => format!("velocity fixed {f}"),
                None => format!("velocity {} {low} {high}", curve.word()),
            },
        }
    }

    /// Reads [`Block::to_text`]'s words back.
    pub fn parse(text: &str) -> Result<Block, String> {
        let mut w = text.split_whitespace();
        let kind = w.next().ok_or("no block")?;
        let rest: Vec<&str> = w.collect();
        let num = |s: &str, lo: i32, hi: i32| -> Result<i32, String> {
            match s.parse::<i32>() {
                Ok(n) if (lo..=hi).contains(&n) => Ok(n),
                _ => Err(format!("\"{s}\" is not a number from {lo} to {hi}")),
            }
        };
        let b = match (kind, rest.as_slice()) {
            ("filter", kinds) => {
                let mut drop = Kinds::NONE;
                for k in kinds {
                    let kind = Kind::ALL
                        .into_iter()
                        .find(|x| x.word() == *k)
                        .ok_or_else(|| format!("no kind \"{k}\""))?;
                    drop = drop.with(kind, true);
                }
                Block::Filter(drop)
            }
            ("channel", [c]) => Block::Channel(num(c, 1, 16)? as u8 - 1),
            ("split", [lo, hi]) => Block::Split {
                low: num(lo, 0, 127)? as u8,
                high: num(hi, 0, 127)? as u8,
            },
            ("transpose", [t]) => {
                let m = i32::from(MAX_TRANSPOSE);
                Block::Transpose(num(t, -m, m)? as i8)
            }
            ("velocity", ["fixed", v]) => Block::Velocity {
                curve: Curve::Linear,
                low: 1,
                high: 127,
                fixed: Some(num(v, 1, 127)? as u8),
            },
            ("velocity", [c, lo, hi]) => Block::Velocity {
                curve: Curve::ALL
                    .into_iter()
                    .find(|x| x.word() == *c)
                    .ok_or_else(|| format!("no curve \"{c}\""))?,
                low: num(lo, 1, 127)? as u8,
                high: num(hi, 1, 127)? as u8,
                fixed: None,
            },
            _ => return Err(format!("\"{text}\" is not a block")),
        };
        Ok(b.tidy())
    }
}

/// Where [`Blocks::run`] left a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Through {
    /// Through every block: this leaves.
    Out(Msg),
    /// A block took it and dropped it (a Filter, or a note moved off the
    /// keyboard).
    Dropped,
    /// A note outside a Split: the route leaves it for others.
    Free,
}

/// A route's blocks, in order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Blocks {
    list: [Option<Block>; MAX_BLOCKS],
}

impl Blocks {
    pub const NONE: Blocks = Blocks {
        list: [None; MAX_BLOCKS],
    };

    pub fn len(&self) -> usize {
        self.list.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.list[0].is_none()
    }

    pub fn get(&self, i: usize) -> Option<Block> {
        self.list.get(i).copied().flatten()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Block> {
        self.list.iter().flatten()
    }

    /// Adds `b` at the end; false when there are [`MAX_BLOCKS`] already.
    pub fn push(&mut self, b: Block) -> bool {
        let n = self.len();
        match self.list.get_mut(n) {
            Some(slot) => {
                *slot = Some(b.tidy());
                true
            }
            None => false,
        }
    }

    /// Block `i` becomes `b`.
    pub fn set(&mut self, i: usize, b: Block) {
        if let Some(Some(slot)) = self.list.get_mut(i) {
            *slot = b.tidy();
        }
    }

    /// Takes block `i` away; the ones after it move up.
    pub fn remove(&mut self, i: usize) {
        let n = self.len();
        if i >= n {
            return;
        }
        self.list.copy_within(i + 1..n, i);
        self.list[n - 1] = None;
    }

    /// Block `i` swaps places with the next one.
    pub fn move_later(&mut self, i: usize) {
        if i + 1 < self.len() {
            self.list.swap(i, i + 1);
        }
    }

    /// `m` through every block in order: as it leaves, or None.
    pub fn apply(&self, m: Msg) -> Option<Msg> {
        match self.run(m) {
            Through::Out(m) => Some(m),
            Through::Dropped | Through::Free => None,
        }
    }

    /// `m` through every block in order, and why it stopped if it did: a
    /// note outside a Split is [`Through::Free`] (the route doesn't take
    /// it, so another route or the armed track may), anything else a block
    /// stops is [`Through::Dropped`] (the route took it, and it ends there).
    pub fn run(&self, m: Msg) -> Through {
        let mut m = m;
        for b in self.iter() {
            m = match b.apply(m) {
                Some(next) => next,
                None if matches!(b, Block::Split { .. }) => return Through::Free,
                None => return Through::Dropped,
            };
        }
        Through::Out(m)
    }

    /// Whether `other` sends every note to the same note and channel as
    /// these do (they differ in velocity at most), so a key held through
    /// them ends the same way after the change.
    pub fn same_notes(&self, other: &Blocks) -> bool {
        let plain = |b: &Blocks| {
            b.list.map(|x| match x {
                Some(Block::Velocity { .. }) => Some(BlockKind::Velocity.new_block()),
                other => other,
            })
        };
        plain(self) == plain(other)
    }

    /// As text, the blocks parted by `;` (empty for none).
    pub fn to_text(&self) -> String {
        let words: Vec<String> = self.iter().map(Block::to_text).collect();
        words.join("; ")
    }

    /// Reads [`Blocks::to_text`] back; refused whole when one can't be
    /// read or there are more than [`MAX_BLOCKS`].
    pub fn parse(text: &str) -> Result<Blocks, String> {
        let mut b = Blocks::NONE;
        for part in text.split(';').map(str::trim).filter(|p| !p.is_empty()) {
            if !b.push(Block::parse(part)?) {
                return Err(format!("more than {MAX_BLOCKS} blocks"));
            }
        }
        Ok(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(note: u8, vel: u8) -> Msg {
        Msg::NoteOn { ch: 0, note, vel }
    }

    /// Each block does what its tile says, in order, and drops what it
    /// should; a note-off goes the way of its note-on.
    #[test]
    fn blocks_shape_what_a_route_takes_in_order() {
        let mut b = Blocks::NONE;
        assert_eq!(b.apply(on(60, 100)), Some(on(60, 100)), "none: as it came");
        assert!(b.push(Block::Split { low: 48, high: 72 }));
        assert!(b.push(Block::Transpose(-12)));
        assert!(b.push(Block::Velocity {
            curve: Curve::Soft,
            low: 20,
            high: 127,
            fixed: None
        }));
        assert!(b.push(Block::Channel(9)));
        assert!(!b.push(Block::Transpose(1)), "four at most");
        // C4 at 32: an octave down, louder (soft), on channel 10.
        let out = b.apply(on(60, 32)).unwrap();
        assert_eq!(
            out,
            Msg::NoteOn {
                ch: 9,
                note: 48,
                vel: 74
            }
        );
        let off = b
            .apply(Msg::NoteOff {
                ch: 0,
                note: 60,
                vel: 64,
            })
            .unwrap();
        assert_eq!(
            off,
            Msg::NoteOff {
                ch: 9,
                note: 48,
                vel: 64
            }
        );
        // Outside the split: dropped, its note-off too; a control goes on.
        assert_eq!(b.apply(on(80, 100)), None);
        assert_eq!(
            b.apply(Msg::NoteOff {
                ch: 0,
                note: 80,
                vel: 0
            }),
            None
        );
        let cc = Msg::Cc {
            ch: 0,
            cc: 1,
            value: 5,
        };
        assert_eq!(
            b.apply(cc),
            Some(Msg::Cc {
                ch: 9,
                cc: 1,
                value: 5
            })
        );
        // A filter drops its kinds.
        b.remove(0);
        b.set(0, Block::Filter(Kinds::NONE.with(Kind::Controls, true)));
        assert_eq!(b.len(), 3);
        assert_eq!(b.apply(cc), None);
        assert!(b.apply(on(30, 100)).is_some(), "the split went");
        // Velocity: fixed, and the hard curve with a range.
        let fixed = Block::Velocity {
            curve: Curve::Linear,
            low: 1,
            high: 127,
            fixed: Some(100),
        };
        assert_eq!(fixed.apply(on(60, 3)), Some(on(60, 100)));
        let hard = Block::Velocity {
            curve: Curve::Hard,
            low: 1,
            high: 127,
            fixed: None,
        };
        assert_eq!(hard.apply(on(60, 64)), Some(on(60, 33)));
        assert_eq!(hard.apply(on(60, 127)), Some(on(60, 127)));
        // Off the keyboard: dropped.
        assert_eq!(Block::Transpose(48).apply(on(100, 1)), None);
        // Moved, removed: the rest close up.
        b.move_later(0);
        assert_eq!(b.get(0).map(|x| x.kind()), Some(BlockKind::Velocity));
        assert_eq!(b.get(1).map(|x| x.kind()), Some(BlockKind::Filter));
        b.remove(1);
        assert_eq!(b.len(), 2);
    }

    /// A Split frees what it leaves out; a Filter keeps what it drops; a
    /// velocity change alone keeps held notes as they are.
    #[test]
    fn a_split_frees_what_a_filter_drops() {
        let mut b = Blocks::NONE;
        b.push(Block::Transpose(12));
        b.push(Block::Split { low: 60, high: 127 });
        assert_eq!(b.run(on(40, 90)), Through::Free, "52 is under the split");
        assert_eq!(b.run(on(50, 90)), Through::Out(on(62, 90)));
        let mut f = Blocks::NONE;
        f.push(Block::Filter(Kinds::NONE.with(Kind::Notes, true)));
        assert_eq!(f.run(on(50, 90)), Through::Dropped);
        assert_eq!(Block::Transpose(48).apply(on(100, 1)), None);
        let mut off = Blocks::NONE;
        off.push(Block::Transpose(48));
        assert_eq!(off.run(on(100, 1)), Through::Dropped, "off the keyboard");
        let mut louder = b;
        assert!(louder.push(BlockKind::Velocity.new_block()));
        let mut softer = b;
        softer.push(Block::Velocity {
            curve: Curve::Soft,
            low: 40,
            high: 90,
            fixed: None,
        });
        assert!(louder.same_notes(&softer));
        assert!(!louder.same_notes(&b));
        let mut moved = softer;
        moved.set(0, Block::Transpose(11));
        assert!(!moved.same_notes(&softer));
    }

    /// Written as text and read back as they were; what can't be read is
    /// refused with why.
    #[test]
    fn blocks_read_back_as_written() {
        let mut b = Blocks::NONE;
        b.push(Block::Filter(
            Kinds::NONE.with(Kind::Program, true).with(Kind::Bend, true),
        ));
        b.push(Block::Transpose(-12));
        b.push(Block::Velocity {
            curve: Curve::Soft,
            low: 20,
            high: 127,
            fixed: None,
        });
        b.push(Block::Split { low: 36, high: 59 });
        let text = b.to_text();
        assert_eq!(
            text,
            "filter bend program; transpose -12; velocity soft 20 127; split 36 59"
        );
        assert_eq!(Blocks::parse(&text), Ok(b));
        assert_eq!(Blocks::parse(""), Ok(Blocks::NONE));
        let fixed = Blocks::parse("velocity fixed 90; channel 10").unwrap();
        assert_eq!(fixed.get(1), Some(Block::Channel(9)));
        for bad in [
            "transpose 49",
            "channel 0",
            "velocity loud 1 2",
            "filter clocks",
            "split 1",
            "wobble",
            "transpose 1; transpose 1; transpose 1; transpose 1; transpose 1",
        ] {
            assert!(Blocks::parse(bad).is_err(), "{bad}");
        }
        // Out of order ends are put in order.
        assert_eq!(
            Block::parse("split 60 40"),
            Ok(Block::Split { low: 40, high: 60 })
        );
    }
}
