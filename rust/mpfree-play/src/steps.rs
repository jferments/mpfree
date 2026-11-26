//! Where a clip with a play direction (sequencer S6,
//! [`mpfree_seq::Direction`]) plays its steps, for the player and the
//! recorder alike, so a note recorded as heard plays where it was heard.
//!
//! Counted in "play ticks": the clip's own ticks from its origin, as if it
//! played forward. The play ticks are cut into play steps of
//! [`DIRECTION_STEP`], each around its line: play step `S` holds the play
//! ticks within half a step of `S × step`. Pass `S / n` of a clip of `n`
//! steps plays its step `S mod n` as the clip step its direction picks
//! for that pass ([`mpfree_seq::Direction::step`], passes counted from the
//! one its launch started in), and a clip step holds the notes within
//! half a step of its own line (the clip's last half step is its first
//! step's, played before the line). Each note keeps its distance from the
//! line, so a note played a little early or late moves with its step.

use mpfree_seq::{Clip, Direction, Speed, DIRECTION_STEP};
use mpfree_transport::time::Tick;

use crate::view::ClipView;

/// A step, in clip ticks.
pub(crate) const STEP: f64 = DIRECTION_STEP as f64;
/// Half a step: how far from its line a step's notes reach.
pub(crate) const HALF: f64 = STEP / 2.0;

/// A launched clip's play direction, as [`Steps::of`] makes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Steps {
    direction: Direction,
    /// The clip's steps (two or more).
    pub n: u32,
    /// The pass its launch started in, counted from its origin: the
    /// direction counts passes from here.
    first: i64,
    /// Random's orders: the track's and the slot's.
    seed: u64,
}

impl Steps {
    /// `clip`'s play direction on `track`, launched on song tick `start`
    /// from `origin`: None when it plays forward ([`ClipView::steps`] 0).
    pub fn of(clip: &ClipView, track: usize, origin: Tick, start: Tick) -> Option<Steps> {
        (clip.steps >= 2).then(|| Steps {
            direction: clip.direction,
            n: clip.steps,
            first: first_pass(clip, origin, start),
            seed: seed(track, clip.slot),
        })
    }

    /// The play step whose ticks hold play tick `x`.
    pub fn at(x: f64) -> i64 {
        ((x + HALF) / STEP).floor() as i64
    }

    /// The pass play step `big` is in, counted from the origin.
    pub fn pass(&self, big: i64) -> i64 {
        big.div_euclid(i64::from(self.n))
    }

    /// Play step `big`'s line, in its pass's clip ticks.
    pub fn line(&self, big: i64) -> f64 {
        big.rem_euclid(i64::from(self.n)) as f64 * STEP
    }

    /// The clip step play step `big` plays.
    pub fn pick(&self, big: i64) -> u32 {
        let n = i64::from(self.n);
        let s = big.rem_euclid(n) as u32;
        self.direction
            .step(s, big.div_euclid(n) - self.first, self.n, self.seed)
    }

    /// The clip's length, in clip ticks.
    pub fn len(&self) -> f64 {
        f64::from(self.n) * STEP
    }

    /// What play tick `x` hears: the clip tick (0 to the clip's length) of
    /// the step played there, as far from that step's line as `x` is from
    /// the play step's, and the pass the play step is in.
    pub fn heard(&self, x: f64) -> (f64, i64) {
        let big = Steps::at(x);
        let k = self.pick(big);
        let local = f64::from(k) * STEP + (x - big as f64 * STEP);
        (local.rem_euclid(self.len()), self.pass(big))
    }

    /// The clip step that holds clip tick `local` (0 to the clip's
    /// length), and how far from that step's line it is.
    pub fn step_of(&self, local: f64) -> (u32, f64) {
        let k = Steps::at(local).rem_euclid(i64::from(self.n));
        let mut off = local - k as f64 * STEP;
        if off > HALF {
            // The last half step: the first step's, before its line.
            off -= self.len();
        }
        (k as u32, off)
    }
}

/// The pass song tick `start` is in, counted from `origin` (`num` passes
/// are exactly `length × den` song ticks): pass numbers for trig
/// conditions and directions count from it.
pub(crate) fn first_pass(clip: &ClipView, origin: Tick, start: Tick) -> i64 {
    pass_of(clip.speed, clip.length, origin, start)
}

fn pass_of(speed: Speed, length: Tick, origin: Tick, start: Tick) -> i64 {
    ((start - origin) * Tick::from(speed.num.max(1)))
        .div_euclid(length.max(1) * Tick::from(speed.den.max(1)))
}

/// The step a launched clip with a play direction plays, for a step
/// playhead (the Play page's pattern, the step pads): the same order the
/// player plays, from the project's clip, so the light is on the step
/// heard rather than on the step under the clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepOrder(Steps);

impl StepOrder {
    /// `clip` in `slot` on `track`, launched on song tick `start` from
    /// `origin` (a [`crate::Launch`]): None when it plays its steps
    /// forward, as the player does ([`Clip::direction_steps`]).
    pub fn of(
        clip: &Clip,
        track: usize,
        slot: usize,
        origin: Tick,
        start: Tick,
    ) -> Option<StepOrder> {
        let n = clip.direction_steps()?;
        Some(StepOrder(Steps {
            direction: clip.direction,
            n,
            first: pass_of(clip.speed, clip.length, origin, start),
            seed: seed(track, slot),
        }))
    }

    /// The clip step played while play step `big` sounds: the steps
    /// counted forward from the origin, from line to line (step `big`
    /// from its line until the next).
    pub fn step(&self, big: i64) -> usize {
        self.0.pick(big) as usize
    }
}

/// Random's seed for the clip in `slot` on `track`: the same every time
/// the clip plays there, so a pass plays the same order each time (an
/// export too); a copy in another slot or on another track has its own.
fn seed(track: usize, slot: usize) -> u64 {
    let mix = |mut z: u64| {
        z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    mix(mix(track as u64) ^ slot as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps(direction: Direction, n: u32) -> Steps {
        Steps {
            direction,
            n,
            first: 0,
            seed: 5,
        }
    }

    /// What a play tick hears is the step the player plays there, at the
    /// same distance from its line; a step's half before its line is the
    /// one before's.
    #[test]
    fn a_play_tick_hears_the_step_played_there() {
        let st = steps(Direction::Reverse, 4);
        // Play step 0 plays clip step 3.
        assert_eq!(st.heard(0.0), (720.0, 0));
        assert_eq!(st.heard(10.0), (730.0, 0));
        assert_eq!(st.heard(119.0), (839.0, 0));
        // Just before play step 1's line: clip step 2, a little early.
        assert_eq!(st.heard(230.0), (470.0, 0));
        // Just before the next pass: its first play step, clip step 3.
        assert_eq!(st.heard(950.0), (710.0, 1));
        // Clip step 0 just early: the clip's last half step.
        assert_eq!(st.heard(710.0), (950.0, 0));
        assert_eq!(st.step_of(950.0), (0, -10.0));
        assert_eq!(st.step_of(710.0), (3, -10.0));
        assert_eq!(st.step_of(845.0), (0, -115.0));
        assert_eq!(st.step_of(835.0), (3, 115.0));
        // Forward hears what it plays.
        let f = steps(Direction::PingPong, 2);
        for x in [0.0, 100.0, 300.0, 470.0] {
            assert_eq!(f.heard(x).0, x);
        }
    }

    /// A step playhead's order from the project's clip is the player's
    /// own, launched mid-clip at half speed too (passes count from the
    /// launch); forward and part steps have none.
    #[test]
    fn a_step_playhead_follows_the_order_the_player_plays() {
        use crate::view::view_of;
        let mut clip = Clip::new("Hats", 8 * DIRECTION_STEP);
        clip.speed = Speed { num: 1, den: 2 };
        for direction in [Direction::Reverse, Direction::PingPong, Direction::Random] {
            clip.direction = direction;
            // Launched in its third pass from an origin of bar 2.
            let (origin, start) = (3840, 3840 + 2 * 2 * 8 * DIRECTION_STEP + 480);
            let order = StepOrder::of(&clip, 3, 5, origin, start).expect("a direction");
            let view = view_of(
                &clip,
                None,
                None,
                &mpfree_seq::inst::Knobs::default(),
                5,
                origin,
                start,
            );
            let played = Steps::of(&view, 3, origin, start).expect("the player's");
            for big in 0..64 {
                assert_eq!(
                    order.step(big),
                    played.pick(big) as usize,
                    "{direction:?} {big}"
                );
            }
        }
        clip.direction = Direction::Forward;
        assert_eq!(StepOrder::of(&clip, 3, 5, 0, 0), None);
        clip.direction = Direction::Reverse;
        clip.length = 8 * DIRECTION_STEP + 60;
        assert_eq!(StepOrder::of(&clip, 3, 5, 0, 0), None);
    }
}
