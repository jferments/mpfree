//! Who owns the pad.

use crate::input::{Device, Phase, XyInput};

/// What the owner's input does to the pad.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// A new gesture holds the pad at `x`, `y` (another may have lost it).
    Begin { x: f32, y: f32 },
    /// The owner moved to `x`, `y`.
    Move { x: f32, y: f32 },
    /// The owner let go (or was cancelled) at `x`, `y`.
    End { x: f32, y: f32 },
}

const DEVICES: usize = 2;

fn slot(d: Device) -> usize {
    match d {
        Device::Screen => 0,
        Device::Midi => 1,
    }
}

/// The pad's owner: one touch on one device, from its Begin to its End or
/// Cancel.
///
/// - The first touch owns the pad until it ends; another touch on the same
///   device is ignored by its id, never "whichever finger moved last".
/// - A Begin from another device takes the pad: the newest gesture wins,
///   as for any setting held by hand. The gesture that lost it is ignored
///   to its end; a new one from that device takes the pad back.
/// - Only the owner's End or Cancel lets go.
/// - An input older than the last from its device (by `seq`) is dropped.
#[derive(Clone, Debug, Default)]
pub struct Gesture {
    owner: Option<(Device, u64)>,
    /// Per device: the last `seq` seen.
    seq: [Option<u32>; DEVICES],
    /// Inputs ignored: another touch owned the pad, or theirs had lost it.
    pub ignored: u64,
    /// Gestures another device's new gesture took the pad from.
    pub taken: u64,
    /// Inputs that arrived out of order.
    pub stale: u64,
}

impl Gesture {
    /// What `i` does to the pad: a step for the owner's inputs, `None` for
    /// one that is ignored.
    pub fn input(&mut self, i: &XyInput) -> Option<Step> {
        let d = slot(i.device);
        if let Some(last) = self.seq[d] {
            // Wrapping: a seq less than half the range behind is older.
            if i.seq != last && i.seq.wrapping_sub(last) > u32::MAX / 2 {
                self.stale += 1;
                return None;
            }
        }
        self.seq[d] = Some(i.seq);
        let me = (i.device, i.touch);
        let (x, y) = (i.x, i.y);
        match i.phase {
            Phase::Begin => match self.owner {
                Some(o) if o == me => Some(Step::Move { x, y }),
                Some((od, _)) if od == i.device => {
                    self.ignored += 1;
                    None
                }
                Some(_) => {
                    self.taken += 1;
                    self.owner = Some(me);
                    Some(Step::Begin { x, y })
                }
                None => {
                    self.owner = Some(me);
                    Some(Step::Begin { x, y })
                }
            },
            Phase::Move if self.owner == Some(me) => Some(Step::Move { x, y }),
            Phase::Move => {
                self.ignored += 1;
                None
            }
            Phase::End | Phase::Cancel if self.owner == Some(me) => {
                self.owner = None;
                Some(Step::End { x, y })
            }
            Phase::End | Phase::Cancel => None,
        }
    }

    /// Whether a gesture holds the pad.
    pub fn held(&self) -> bool {
        self.owner.is_some()
    }

    /// The device whose gesture holds the pad.
    pub fn owner(&self) -> Option<Device> {
        self.owner.map(|(d, _)| d)
    }

    /// Every gesture let go of (a project opening, the layout switched):
    /// true when one held the pad.
    pub fn clear(&mut self) -> bool {
        self.owner.take().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn i(device: Device, touch: u64, seq: u32, phase: Phase, x: f32) -> XyInput {
        XyInput::new(device, touch, seq, 0, phase, x, 0.5)
    }

    #[test]
    fn the_first_touch_owns_the_pad_until_it_ends() {
        let mut g = Gesture::default();
        let s = Device::Screen;
        assert_eq!(
            g.input(&i(s, 1, 0, Phase::Begin, 0.1)),
            Some(Step::Begin { x: 0.1, y: 0.5 })
        );
        // A second finger lands, moves and lifts: nothing.
        assert_eq!(g.input(&i(s, 2, 1, Phase::Begin, 0.9)), None);
        assert_eq!(g.input(&i(s, 2, 2, Phase::Move, 0.8)), None);
        assert_eq!(g.input(&i(s, 2, 3, Phase::End, 0.8)), None);
        assert!(g.held());
        assert_eq!(
            g.input(&i(s, 1, 4, Phase::Move, 0.2)),
            Some(Step::Move { x: 0.2, y: 0.5 })
        );
        assert_eq!(
            g.input(&i(s, 1, 5, Phase::Cancel, 0.2)),
            Some(Step::End { x: 0.2, y: 0.5 })
        );
        assert!(!g.held());
        assert_eq!(g.ignored, 2);
    }

    #[test]
    fn another_controllers_gesture_takes_the_pad_and_the_loser_waits() {
        let mut g = Gesture::default();
        let (s, m) = (Device::Screen, Device::Midi);
        assert!(g.input(&i(s, 1, 0, Phase::Begin, 0.1)).is_some());
        assert_eq!(
            g.input(&i(m, 0, 0, Phase::Begin, 0.7)),
            Some(Step::Begin { x: 0.7, y: 0.5 })
        );
        assert_eq!(g.owner(), Some(m));
        // The finger moves on and lifts: ignored, the knob still holds it.
        assert_eq!(g.input(&i(s, 1, 1, Phase::Move, 0.3)), None);
        assert_eq!(g.input(&i(s, 1, 2, Phase::End, 0.3)), None);
        assert!(g.held());
        assert_eq!(
            g.input(&i(m, 0, 1, Phase::End, 0.5)),
            Some(Step::End { x: 0.5, y: 0.5 })
        );
        assert_eq!((g.taken, g.ignored), (1, 1));
        // A new touch holds it again.
        assert!(g.input(&i(s, 3, 3, Phase::Begin, 0.4)).is_some());
        assert!(g.clear());
        assert!(!g.clear());
    }

    #[test]
    fn an_input_out_of_order_is_dropped() {
        let mut g = Gesture::default();
        let s = Device::Screen;
        assert!(g.input(&i(s, 1, 10, Phase::Begin, 0.1)).is_some());
        assert_eq!(g.input(&i(s, 1, 9, Phase::Move, 0.2)), None);
        assert_eq!(g.stale, 1);
        assert!(g.input(&i(s, 1, 11, Phase::Move, 0.3)).is_some());
        // The count wraps.
        assert!(g.input(&i(s, 1, u32::MAX, Phase::Move, 0.3)).is_none());
        let mut g = Gesture::default();
        assert!(g.input(&i(s, 1, u32::MAX, Phase::Begin, 0.1)).is_some());
        assert!(g.input(&i(s, 1, 0, Phase::Move, 0.2)).is_some());
    }
}
