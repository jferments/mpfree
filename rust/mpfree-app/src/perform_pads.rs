//! The Perform pads: pages of pad actions, kept apart from the kits'
//! sounds. A kit's pad is a sound (a sample, a slice, an instrument's
//! voice), sequenced on its track and saved with the kit; a Perform pad is
//! an action on the looper or the mix, which belongs to the project and
//! not to any track (Jay, 17 Nov 23:52Z: "separating throws/FX/loop
//! keys/etc from 'sound' pads").
//!
//! The first page is the looper's **loop keys** (the Looper tab as an
//! instrument, step 2): SHIFT + SAMPLER on the Looper tab puts it on the
//! pads, and they stay loop keys on the Looper tab and Play. More pages
//! and actions (throws, macros, track mutes, scenes) come with the Play,
//! Pads and Sequencer redesign, which takes this page as its page A.

use crate::looper_deck::RingState;
use mpfree_looper::MAX_LOOPS;

/// Pads on a page.
pub const PADS: usize = 16;

/// What a Perform pad does. New kinds of action are new variants; a pad
/// with nothing on it is [`PadAction::None`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PadAction {
    /// Nothing: the pad is dark and a hit does nothing.
    #[default]
    None,
    /// Loop `n`'s key (0 to 7, Loop 1 to 8): a hit is the loop's next
    /// step, as a tap on its ring on the Looper tab (record from the next
    /// bar line, cancel, end the first pass, overdub, play, unmute); held
    /// for a second it clears the loop.
    LoopKey(u8),
    /// Loop `n`'s mute (0 to 7): a hit mutes or unmutes it.
    LoopMute(u8),
}

/// A page of Perform pads: its name, and what each pad does, pad 1 (bottom
/// left) first, as the kits number their pads.
#[derive(Clone, Debug, PartialEq)]
pub struct PerformPage {
    pub name: String,
    pub pads: [PadAction; PADS],
}

impl PerformPage {
    /// The loop keys. Pads 13 to 16 (the top row, where the eye goes
    /// first) are Loops 1 to 4's keys and pads 9 to 12 Loops 5 to 8's;
    /// pads 5 to 8 and 1 to 4 are their mutes, each two rows under its
    /// key, so a column of pads belongs to one loop.
    pub fn loop_keys() -> PerformPage {
        let mut pads = [PadAction::None; PADS];
        for k in 0..4u8 {
            let c = usize::from(k);
            pads[12 + c] = PadAction::LoopKey(k);
            pads[8 + c] = PadAction::LoopKey(4 + k);
            pads[4 + c] = PadAction::LoopMute(k);
            pads[c] = PadAction::LoopMute(4 + k);
        }
        PerformPage {
            name: "Loop keys".to_string(),
            pads,
        }
    }

    /// Pad `pad`'s action (0 is pad 1); None past the page.
    pub fn action(&self, pad: usize) -> PadAction {
        self.pads.get(pad).copied().unwrap_or_default()
    }

    /// The pad that is loop `n`'s key, if the page has one.
    #[cfg(test)]
    pub fn key_of(&self, n: u8) -> Option<usize> {
        self.pads.iter().position(|&a| a == PadAction::LoopKey(n))
    }
}

/// What a loop key's light needs to know about its loop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoopLight {
    pub state: RingState,
    /// The loop's colour, 0 to 255 a channel (the deck's).
    pub colour: [u8; 3],
    /// The loop is in its first beat: a playing loop's key flashes, so
    /// the hands can see where it comes round.
    pub downbeat: bool,
    /// Soloed: its mute pad lights blue.
    pub solo: bool,
}

impl Default for LoopLight {
    fn default() -> LoopLight {
        LoopLight {
            state: RingState::Empty,
            colour: [0; 3],
            downbeat: false,
            solo: false,
        }
    }
}

/// How long a loop key is held to clear its loop, as a ring on the
/// Looper tab.
pub const CLEAR_HOLD_S: f64 = crate::looper_deck::CLEAR_HOLD_S as f64;

/// Each loop's light, from the looper: its state, its colour on the deck,
/// and whether it plays its first beat.
pub fn lights(looper: &crate::looping::LoopHost) -> [LoopLight; MAX_LOOPS] {
    std::array::from_fn(|n| {
        let Some(l) = looper.loop_status(n) else {
            return LoopLight::default();
        };
        let c = crate::looper_tab::LOOP_COLOURS[n % crate::looper_tab::LOOP_COLOURS.len()];
        let beats = l.ticks as f64 / mpfree_transport::time::PPQ as f64;
        LoopLight {
            state: RingState::of(&l, None),
            colour: [c.r, c.g, c.b],
            downbeat: beats > 0.0 && f64::from(l.position) * beats < 1.0,
            solo: l.solo && l.phase != mpfree_looper::Phase::Empty,
        }
    })
}

/// The Perform pads held, as the pad player and the smoke tests keep them:
/// when each went down and whether its hold has cleared its loop.
#[derive(Clone, Debug, Default)]
pub struct Held {
    down: [Option<(f64, bool)>; PADS],
}

impl Held {
    /// Pad `pad` went down at `t` seconds.
    pub fn press(&mut self, pad: usize, t: f64) {
        if let Some(d) = self.down.get_mut(pad) {
            *d = Some((t, false));
        }
    }

    /// Pad `pad` let go: true when it was down as a Perform pad (its
    /// release is the Perform pads', not the kit's).
    pub fn release(&mut self, pad: usize) -> bool {
        self.down.get_mut(pad).and_then(Option::take).is_some()
    }

    /// The pads whose hold reached [`CLEAR_HOLD_S`] by `t`, once each.
    pub fn holds(&mut self, t: f64) -> Vec<usize> {
        let mut out = Vec::new();
        for (pad, d) in self.down.iter_mut().enumerate() {
            if let Some((at, false)) = *d {
                if t - at >= CLEAR_HOLD_S {
                    *d = Some((at, true));
                    out.push(pad);
                }
            }
        }
        out
    }
}

/// Red: recording, waiting for the bar (blinking), or muted on a mute pad.
pub const RED: [u8; 3] = [0x7F, 0x00, 0x00];
/// Amber: overdubbing.
pub const AMBER: [u8; 3] = [0x7F, 0x44, 0x00];
/// Blue: a soloed loop's mute pad (SHIFT + the pad).
pub const SOLO: [u8; 3] = [0x00, 0x38, 0x7F];
const DARK: [u8; 3] = [0; 3];

/// A colour at `f` of its brightness, in the pads' 0 to 127.
fn pad_rgb(c: [u8; 3], f: f32) -> [u8; 3] {
    c.map(|v| ((f32::from(v >> 1) * f.clamp(0.0, 1.0)).round() as u8).min(0x7F))
}

/// The 16 pads' colours for `page`, from each loop's light: lit by state
/// so the pads can be played without looking at the screen. A key: dark
/// when its loop is empty, red blinking while it waits for its bar, red
/// recording, its colour playing (full on the loop's first beat, softer
/// after), amber overdubbing, its colour dim when muted, off or quiet
/// for another loop's solo. A mute pad: dark for an empty loop, red while
/// muted, blue while soloed, its colour dim while it plays. `blink`: the
/// half of the blink that is lit.
pub fn colours(page: &PerformPage, loops: &[LoopLight], blink: bool) -> [[u8; 3]; PADS] {
    std::array::from_fn(|pad| {
        let light = |n: u8| loops.get(usize::from(n)).copied().unwrap_or_default();
        match page.action(pad) {
            PadAction::None => DARK,
            PadAction::LoopKey(n) => {
                let l = light(n);
                match l.state {
                    RingState::Empty => DARK,
                    RingState::Waiting(_) if blink => RED,
                    RingState::Waiting(_) => DARK,
                    RingState::Recording => RED,
                    RingState::Overdubbing => AMBER,
                    RingState::Playing if l.downbeat => pad_rgb(l.colour, 1.0),
                    RingState::Playing => pad_rgb(l.colour, 0.55),
                    RingState::Muted | RingState::Off => pad_rgb(l.colour, 0.12),
                }
            }
            PadAction::LoopMute(n) => {
                let l = light(n);
                match l.state {
                    RingState::Empty | RingState::Waiting(_) => DARK,
                    RingState::Muted => RED,
                    _ if l.solo => SOLO,
                    _ => pad_rgb(l.colour, 0.2),
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_loop_keys_put_each_loop_in_a_column_key_over_mute() {
        let p = PerformPage::loop_keys();
        assert_eq!(p.name, "Loop keys");
        // Pad 13 (top left) is Loop 1's key, pad 5 its mute; pad 9 is
        // Loop 5's key and pad 1 (bottom left) its mute.
        assert_eq!(p.action(12), PadAction::LoopKey(0));
        assert_eq!(p.action(4), PadAction::LoopMute(0));
        assert_eq!(p.action(15), PadAction::LoopKey(3));
        assert_eq!(p.action(8), PadAction::LoopKey(4));
        assert_eq!(p.action(0), PadAction::LoopMute(4));
        assert_eq!(p.action(11), PadAction::LoopKey(7));
        assert_eq!(p.action(3), PadAction::LoopMute(7));
        assert_eq!(p.action(16), PadAction::None);
        for n in 0..8u8 {
            let key = p.key_of(n).unwrap();
            let mute = p.pads.iter().position(|&a| a == PadAction::LoopMute(n));
            assert_eq!(
                mute,
                Some(key - 8),
                "loop {n}'s mute is two rows under its key"
            );
        }
    }

    #[test]
    fn a_key_held_a_second_clears_once() {
        let mut h = Held::default();
        h.press(12, 1.0);
        assert!(h.holds(1.5).is_empty());
        assert_eq!(h.holds(2.0), vec![12]);
        assert!(h.holds(3.0).is_empty(), "once");
        assert!(h.release(12));
        assert!(!h.release(12), "let go once");
        // A hold the pad let go of first never comes.
        h.press(13, 4.0);
        assert!(h.release(13));
        assert!(h.holds(6.0).is_empty());
    }

    #[test]
    fn the_keys_light_by_state() {
        let p = PerformPage::loop_keys();
        let blue = [0x6E, 0xA8, 0xFE];
        let mut loops = [LoopLight::default(); 8];
        loops[0] = LoopLight {
            state: RingState::Playing,
            colour: blue,
            downbeat: false,
            solo: false,
        };
        loops[1].state = RingState::Recording;
        loops[2].state = RingState::Waiting(Some(5));
        loops[3].state = RingState::Overdubbing;
        loops[4] = LoopLight {
            state: RingState::Muted,
            colour: blue,
            downbeat: false,
            solo: false,
        };
        let on = colours(&p, &loops, true);
        let off = colours(&p, &loops, false);
        // Loop 1 plays in its colour, its mute dim; on its first beat the
        // key is full.
        assert_eq!(on[12], [0x1E, 0x2E, 0x46]);
        assert_eq!(on[4], [0x0B, 0x11, 0x19]);
        loops[0].downbeat = true;
        assert_eq!(colours(&p, &loops, true)[12], [0x37, 0x54, 0x7F]);
        // Loop 2 records: red. Loop 3 waits: red, blinking.
        assert_eq!(on[13], RED);
        assert_eq!((on[14], off[14]), (RED, [0; 3]));
        // Loop 4 overdubs: amber.
        assert_eq!(on[15], AMBER);
        // Loop 5 is muted: its key dim, its mute red.
        assert_eq!(on[8], [0x07, 0x0A, 0x0F]);
        assert_eq!(on[0], RED);
        // Empty loops: dark keys and mutes.
        assert_eq!(on[11], [0; 3]);
        assert_eq!(on[3], [0; 3]);
        // Loop 1 soloed: its mute blue; a loop quiet for it is off, dim.
        loops[0].solo = true;
        loops[5] = LoopLight {
            state: RingState::Off,
            colour: blue,
            downbeat: false,
            solo: false,
        };
        let solo = colours(&p, &loops, true);
        assert_eq!(solo[4], SOLO);
        assert_eq!(solo[9], [0x07, 0x0A, 0x0F]);
        // Every value is in the pads' 0 to 127.
        assert!(on.iter().flatten().all(|&v| v <= 0x7F));
    }
}
