//! The Play page's Loop group (audio tracks phase 4e, step 4e.1; board 7
//! of the phase 4 screens, approved as drawn by Jay on 23 Nov 2026): a
//! performance group over the loops on audio tracks that never writes a
//! track's settings (the plan's decision 3, after Jay's review).
//!
//! - **Level and Filter act on each loop's sound** before its track: a
//!   gain and a DJ filter on the loop's own buffer (the looper's slot, 8 at
//!   most), before the track player sums it into the track
//!   ([`LoopGroup::run`], [`LoopGroup::out`]). So they reach loops only,
//!   never a take clip on the same track, and a loop fading out after its
//!   track switched to a take clip keeps them until its fade ends.
//! - **Delay adds to the channel's Send A** while the track plays a loop:
//!   a gain on top of the send's own level (`Mixer::set_send_add`), whose
//!   own value is never written ([`LoopGroup::sends`]).
//! - **At rest** (0 dB, Off, 0 %) nothing runs past a check and every
//!   track's settings, automation and locks are as they were: the group
//!   only ever adds last, on the way through.
//! - **The target switch:** every loop (the default), or only the loops on
//!   the armed track.
//!
//! The group is a performance: it isn't saved, and a new start is at rest.
//! Nothing here allocates after [`LoopGroup::new`].

use mpfree_fx::ids::dj_filter;
use mpfree_fx::mpfree_node::{Ctx, Node};
use mpfree_fx::DjFilter;
use mpfree_looper::MAX_LOOPS;

use crate::looping::PERIOD;

/// Level's range: silence at the bottom, +6 dB at the top.
pub const LEVEL_FLOOR_DB: f32 = -60.0;
pub const LEVEL_TOP_DB: f32 = 6.0;

/// The Delay as the most it adds to a send: 1 (0 dB on top of the send's
/// own level).
pub const DELAY_MAX: f32 = 1.0;

/// The return the Delay adds to: Send A.
pub const SEND_A: usize = 0;

/// How long a loop's filter keeps running after it went back to Off:
/// its knob's glide and its ring (the filter's own tail), 0.1 s.
const TAIL_S: f32 = 0.1;

/// Where the Loop group's knobs are, and whom they reach.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroupSet {
    /// [`LEVEL_FLOOR_DB`] (silent) to [`LEVEL_TOP_DB`]; 0 at rest.
    pub level_db: f32,
    /// The DJ filter's place: -100 (low-pass) to 100 (high-pass), 0 off.
    pub filter: f32,
    /// What the Delay adds to Send A, 0 to [`DELAY_MAX`].
    pub delay: f32,
    /// Only the armed track's loops ("Armed: Vocal"); else every loop.
    pub armed_only: bool,
}

impl Default for GroupSet {
    fn default() -> GroupSet {
        GroupSet {
            level_db: 0.0,
            filter: 0.0,
            delay: 0.0,
            armed_only: false,
        }
    }
}

impl GroupSet {
    /// All three knobs at rest: nothing is changed anywhere.
    pub fn at_rest(&self) -> bool {
        self.level_db == 0.0 && self.filter == 0.0 && self.delay == 0.0
    }

    /// Level as a gain.
    pub fn gain(&self) -> f32 {
        if self.level_db <= LEVEL_FLOOR_DB {
            0.0
        } else {
            10f32.powf(self.level_db.min(LEVEL_TOP_DB) / 20.0)
        }
    }

    /// Kept in range (a value from the screen).
    pub fn clamped(self) -> GroupSet {
        let f = |v: f32, lo: f32, hi: f32| if v.is_finite() { v.clamp(lo, hi) } else { 0.0 };
        GroupSet {
            level_db: f(self.level_db, LEVEL_FLOOR_DB, LEVEL_TOP_DB),
            filter: f(self.filter, -100.0, 100.0),
            delay: f(self.delay, 0.0, DELAY_MAX),
            armed_only: self.armed_only,
        }
    }
}

/// Whether the group reaches a loop on project track `on` (None: on no
/// track), `armed` being the armed track: every loop, or with the switch
/// only the armed track's.
pub fn reaches(set: &GroupSet, armed: Option<usize>, on: Option<usize>) -> bool {
    !set.armed_only || (armed.is_some() && on == armed)
}

/// The Loop group as the audio loop runs it.
pub struct LoopGroup {
    set: GroupSet,
    filters: Vec<DjFilter>,
    /// Each loop's gain at the end of the last period it was run.
    gain: [f32; MAX_LOOPS],
    /// Frames each loop's filter still runs after going back to Off.
    tail: [u32; MAX_LOOPS],
    /// Each loop's sound as the group made it this period, interleaved
    /// stereo, [`PERIOD`] frames a loop.
    bufs: Box<[f32]>,
    /// The loops whose sound the group made this period ([`LoopGroup::out`]).
    made: u32,
    left: Box<[f32]>,
    right: Box<[f32]>,
    tail_frames: u32,
    /// Each audio track's Send A offset as last set, by mixer channel
    /// (`usize::MAX`: none).
    sent: [(usize, f32); mpfree_audiotracks::MAX_TRACKS],
    ctx: Ctx<'static>,
}

impl LoopGroup {
    /// At rest. Allocates its buffers and filters, never after.
    pub fn new(rate: u32) -> LoopGroup {
        let filters = (0..MAX_LOOPS)
            .map(|_| {
                let mut f = DjFilter::new();
                f.prepare(rate, PERIOD);
                f
            })
            .collect();
        LoopGroup {
            set: GroupSet::default(),
            filters,
            gain: [1.0; MAX_LOOPS],
            tail: [0; MAX_LOOPS],
            bufs: vec![0.0; MAX_LOOPS * 2 * PERIOD].into_boxed_slice(),
            made: 0,
            left: vec![0.0; PERIOD].into_boxed_slice(),
            right: vec![0.0; PERIOD].into_boxed_slice(),
            tail_frames: (rate as f32 * TAIL_S) as u32,
            sent: [(usize::MAX, 0.0); mpfree_audiotracks::MAX_TRACKS],
            ctx: Ctx::new(rate, 0),
        }
    }

    pub fn set(&self) -> GroupSet {
        self.set
    }

    /// The screen moved a knob or the switch.
    pub fn set_to(&mut self, set: GroupSet) {
        self.set = set.clamped();
    }

    /// Each period, after the looper's and before the tracks': loop `n`'s
    /// sound this period is `src(n)` (interleaved stereo, `frames` frames;
    /// None: it isn't on a track), and `aimed(n)` says whether the group
    /// reaches it (the switch). Makes the sound of each loop the group
    /// changes; [`LoopGroup::out`] gives it. At rest, with every loop's
    /// gain back at 1 and its filter rung out, only a check.
    pub fn run<'a>(
        &mut self,
        frames: usize,
        src: impl Fn(usize) -> Option<&'a [f32]>,
        aimed: impl Fn(usize) -> bool,
    ) {
        self.made = 0;
        let frames = frames.min(PERIOD);
        let quiet = self.set.level_db == 0.0 && self.set.filter == 0.0;
        if quiet && self.gain.iter().all(|&g| g == 1.0) && self.tail.iter().all(|&t| t == 0) {
            return;
        }
        let gain = self.set.gain();
        for n in 0..MAX_LOOPS {
            let Some(s) = src(n).filter(|s| s.len() >= 2 * frames) else {
                // Off its track: it starts again from rest.
                if self.gain[n] != 1.0 || self.tail[n] != 0 {
                    self.gain[n] = 1.0;
                    self.tail[n] = 0;
                    self.filters[n].set_param(dj_filter::POSITION, 0.0);
                    self.filters[n].reset();
                }
                continue;
            };
            let on = aimed(n);
            let (to, pos) = if on {
                (gain, self.set.filter)
            } else {
                (1.0, 0.0)
            };
            if pos != 0.0 {
                self.tail[n] = self.tail_frames;
            }
            let filtering = pos != 0.0 || self.tail[n] > 0;
            if to == 1.0 && self.gain[n] == 1.0 && !filtering {
                continue;
            }
            let (l, r) = (&mut self.left[..frames], &mut self.right[..frames]);
            for j in 0..frames {
                l[j] = s[2 * j];
                r[j] = s[2 * j + 1];
            }
            if filtering {
                let f = &mut self.filters[n];
                f.set_param(dj_filter::POSITION, pos);
                f.process(&self.ctx, &[], l, r);
                if pos == 0.0 {
                    self.tail[n] = self.tail[n].saturating_sub(frames as u32);
                    if self.tail[n] == 0 {
                        f.reset();
                    }
                }
            }
            // The gain glides across the period, so a turn never clicks.
            let from = self.gain[n];
            let out = &mut self.bufs[n * 2 * PERIOD..][..2 * frames];
            let step = (to - from) / frames as f32;
            for j in 0..frames {
                let g = from + step * (j + 1) as f32;
                out[2 * j] = l[j] * g;
                out[2 * j + 1] = r[j] * g;
            }
            self.gain[n] = to;
            self.made |= 1 << n;
        }
    }

    /// Loop `n`'s sound as the group made it this period, if it changed it
    /// (else the looper's own is the one).
    pub fn out(&self, n: usize) -> Option<&[f32]> {
        (n < MAX_LOOPS && self.made & (1 << n) != 0)
            .then(|| &self.bufs[n * 2 * PERIOD..(n + 1) * 2 * PERIOD])
    }

    /// Each period, the Delay: each audio track playing a loop the group
    /// reaches (`plays(t)`: engine track `t`'s mixer channel, and whether
    /// the group reaches the loop it plays; None: no loop, or no track)
    /// gets the Delay on its Send A; every other gets nothing added. Only a
    /// change reaches the mixer.
    pub fn sends(
        &mut self,
        mixer: &mut mpfree_fx::Mixer,
        plays: impl Fn(usize) -> Option<(usize, bool)>,
    ) {
        for t in 0..self.sent.len() {
            let (ch, add) = match plays(t) {
                Some((ch, true)) => (ch, self.set.delay),
                Some((ch, false)) => (ch, 0.0),
                None => (usize::MAX, 0.0),
            };
            let (was_ch, was) = self.sent[t];
            if was_ch != ch && was_ch != usize::MAX && was != 0.0 {
                mixer.set_send_add(mpfree_fx::StripId::Track(was_ch), SEND_A, 0.0);
            }
            if ch != usize::MAX && (was_ch != ch || was != add) {
                mixer.set_send_add(mpfree_fx::StripId::Track(ch), SEND_A, add);
            }
            self.sent[t] = (ch, add);
        }
    }
}

/// The Loop group's knobs as the screen turns them and writes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupKnob {
    Level,
    Filter,
    Delay,
}

impl GroupKnob {
    pub const ALL: [GroupKnob; 3] = [GroupKnob::Level, GroupKnob::Filter, GroupKnob::Delay];

    pub fn name(self) -> &'static str {
        match self {
            GroupKnob::Level => "Level",
            GroupKnob::Filter => "Filter",
            GroupKnob::Delay => "Delay",
        }
    }

    /// The knob's value in `s`.
    pub fn get(self, s: &GroupSet) -> f32 {
        match self {
            GroupKnob::Level => s.level_db,
            GroupKnob::Filter => s.filter,
            GroupKnob::Delay => s.delay,
        }
    }

    /// `s` with this knob turned `ticks` detents (`fine`: SHIFT held).
    pub fn turn(self, s: GroupSet, ticks: i32, fine: bool) -> GroupSet {
        let d = ticks as f32;
        let mut s = s;
        match self {
            GroupKnob::Level => {
                let from = s.level_db.max(LEVEL_FLOOR_DB);
                s.level_db =
                    (from + d * if fine { 0.1 } else { 0.5 }).clamp(LEVEL_FLOOR_DB, LEVEL_TOP_DB);
                // Back across 0: at rest exactly.
                if s.level_db.abs() < 0.05 {
                    s.level_db = 0.0;
                }
            }
            GroupKnob::Filter => {
                s.filter = (s.filter + d * if fine { 0.5 } else { 2.0 }).clamp(-100.0, 100.0);
                if s.filter.abs() < 0.25 {
                    s.filter = 0.0;
                }
            }
            GroupKnob::Delay => {
                s.delay = (s.delay + d * if fine { 0.005 } else { 0.02 }).clamp(0.0, DELAY_MAX);
                if s.delay < 0.0025 {
                    s.delay = 0.0;
                }
            }
        }
        s
    }

    /// How far round its arc the knob is lit, 0 to 1, and whether it is lit
    /// both ways from the middle.
    pub fn arc(self, s: &GroupSet) -> (f32, bool) {
        match self {
            GroupKnob::Level => (
                ((s.level_db - LEVEL_FLOOR_DB) / (LEVEL_TOP_DB - LEVEL_FLOOR_DB)).clamp(0.0, 1.0),
                false,
            ),
            GroupKnob::Filter => (((s.filter + 100.0) / 200.0).clamp(0.0, 1.0), true),
            GroupKnob::Delay => ((s.delay / DELAY_MAX).clamp(0.0, 1.0), false),
        }
    }

    /// The value as the knob writes it: "0.0 dB", "-inf dB", "Off",
    /// "LP 2.1 kHz", "0 %", "+18 %".
    pub fn text(self, s: &GroupSet) -> String {
        match self {
            GroupKnob::Level if s.level_db <= LEVEL_FLOOR_DB => "-inf dB".to_string(),
            GroupKnob::Level => format!("{:+.1} dB", s.level_db).replace("+0.0", "0.0"),
            GroupKnob::Filter => crate::play_page::filter_text(s.filter),
            GroupKnob::Delay if s.delay == 0.0 => "0 %".to_string(),
            GroupKnob::Delay => format!("+{:.0} %", s.delay * 100.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn tone(frames: usize, k: f32) -> Vec<f32> {
        (0..frames)
            .flat_map(|j| {
                let x = (j as f32 * k).sin() * 0.5;
                [x, -x]
            })
            .collect()
    }

    /// At rest the group makes nothing: every loop plays its own sound,
    /// whatever the switch, so every track is as it was.
    #[test]
    fn at_rest_the_group_leaves_every_loop_as_it_is() {
        let mut g = LoopGroup::new(RATE);
        let s = tone(PERIOD, 0.05);
        for armed_only in [false, true] {
            g.set_to(GroupSet {
                armed_only,
                ..GroupSet::default()
            });
            g.run(PERIOD, |_| Some(&s[..]), |_| true);
            assert!((0..MAX_LOOPS).all(|n| g.out(n).is_none()));
        }
        // Turned and back: once the glide and the filter's ring are over,
        // the group is out of the way again, sample for sample.
        g.set_to(GroupSet {
            level_db: -6.0,
            filter: -40.0,
            ..GroupSet::default()
        });
        g.run(PERIOD, |_| Some(&s[..]), |_| true);
        assert!(g.out(0).is_some());
        g.set_to(GroupSet::default());
        let mut periods = 0;
        while (0..MAX_LOOPS).any(|n| g.out(n).is_some()) || periods == 0 {
            g.run(PERIOD, |_| Some(&s[..]), |_| true);
            periods += 1;
            assert!(periods < 100, "the group never came back to rest");
        }
        assert!(periods as u32 * PERIOD as u32 <= RATE / 10 + 2 * PERIOD as u32);
    }

    /// Level is a gain on the loops the group reaches, gliding across the
    /// first period; a loop it doesn't reach (the switch on another track)
    /// is left alone, and a loop off its track is never made.
    #[test]
    fn level_reaches_only_the_loops_aimed_at() {
        let mut g = LoopGroup::new(RATE);
        let s = tone(PERIOD, 0.05);
        g.set_to(GroupSet {
            level_db: -6.0,
            armed_only: true,
            ..GroupSet::default()
        });
        let gain = g.set().gain();
        for _ in 0..2 {
            g.run(PERIOD, |n| (n != 3).then_some(&s[..]), |n| n == 1);
        }
        let out = g.out(1).unwrap();
        for j in 0..2 * PERIOD {
            assert!((out[j] - s[j] * gain).abs() < 1e-6);
        }
        assert!(g.out(0).is_none(), "not aimed at");
        assert!(g.out(3).is_none(), "not on a track");
        // Silence at the bottom.
        g.set_to(GroupSet {
            level_db: LEVEL_FLOOR_DB,
            ..GroupSet::default()
        });
        for _ in 0..2 {
            g.run(PERIOD, |_| Some(&s[..]), |_| true);
        }
        assert!(g.out(0).unwrap().iter().all(|&x| x == 0.0));
    }

    /// The filter's low-pass takes the top off a loop's sound.
    #[test]
    fn the_filter_takes_the_top_off() {
        let mut g = LoopGroup::new(RATE);
        // Near 7 kHz.
        let s = tone(PERIOD, 0.9);
        g.set_to(GroupSet {
            filter: -90.0,
            ..GroupSet::default()
        });
        let mut peak = 0.0f32;
        for k in 0..40 {
            g.run(PERIOD, |_| Some(&s[..]), |_| true);
            if k > 30 {
                peak = g.out(0).unwrap().iter().fold(peak, |p, x| p.max(x.abs()));
            }
        }
        assert!(peak < 0.05, "{peak}");
    }

    /// The Delay goes on each track's Send A while it plays a loop the
    /// group reaches, and comes off (to exactly nothing added) when it
    /// stops, when the switch leaves it out, or when the group is at rest.
    #[test]
    fn the_delay_goes_on_send_a_only_while_a_track_plays_a_loop_it_reaches() {
        let cfg = mpfree_fx::MixerConfig {
            inputs: 2,
            pads: 4,
            tracks: 4,
            slots: 4,
            max_latency_ms: 50.0,
        };
        let mut m = mpfree_fx::Mixer::new(cfg, RATE, 512);
        let mut g = LoopGroup::new(RATE);
        let add =
            |m: &mpfree_fx::Mixer, ch: usize| m.send_add(mpfree_fx::StripId::Track(ch), SEND_A);
        g.set_to(GroupSet {
            delay: 0.4,
            ..GroupSet::default()
        });
        // Track 0 plays a loop on channel 2, track 1 a take on channel 3.
        g.sends(&mut m, |t| match t {
            0 => Some((2, true)),
            1 => Some((3, false)),
            _ => None,
        });
        assert_eq!((add(&m, 2), add(&m, 3)), (0.4, 0.0));
        // Its loop stops: nothing added.
        g.sends(&mut m, |t| (t == 1).then_some((3, false)));
        assert_eq!(add(&m, 2), 0.0);
        // The track moves to another channel while it plays: the old one
        // is put back.
        g.sends(&mut m, |t| (t == 0).then_some((2, true)));
        g.sends(&mut m, |t| (t == 0).then_some((1, true)));
        assert_eq!((add(&m, 2), add(&m, 1)), (0.0, 0.4));
        // At rest: nothing anywhere, and the send's own value untouched.
        g.set_to(GroupSet::default());
        g.sends(&mut m, |t| (t == 0).then_some((1, true)));
        assert_eq!(add(&m, 1), 0.0);
        assert_eq!(
            m.param(
                mpfree_fx::StripId::Track(1),
                mpfree_fx::ids::mixer::send(SEND_A)
            ),
            mpfree_fx::SILENT_DB
        );
    }

    #[test]
    fn the_knobs_turn_write_and_rest_at_zero() {
        let s = GroupSet::default();
        assert!(s.at_rest());
        let s = GroupKnob::Level.turn(s, -6, false);
        assert_eq!(GroupKnob::Level.text(&s), "-3.0 dB");
        let s = GroupKnob::Level.turn(s, 6, false);
        assert_eq!(s.level_db, 0.0);
        assert_eq!(GroupKnob::Level.text(&s), "0.0 dB");
        let s = GroupKnob::Delay.turn(s, 9, false);
        assert_eq!(GroupKnob::Delay.text(&s), "+18 %");
        let s = GroupKnob::Delay.turn(s, -9, false);
        assert_eq!(GroupKnob::Delay.text(&s), "0 %");
        assert_eq!(GroupKnob::Filter.text(&s), "Off");
        let s = GroupKnob::Filter.turn(s, -20, false);
        assert!(
            GroupKnob::Filter.text(&s).starts_with("LP"),
            "{}",
            GroupKnob::Filter.text(&s)
        );
        let s = GroupKnob::Filter.turn(s, 20, false);
        assert!(s.at_rest());
        let s = GroupKnob::Level.turn(GroupSet::default(), -200, false);
        assert_eq!(GroupKnob::Level.text(&s), "-inf dB");
        assert_eq!(s.gain(), 0.0);
    }
}
