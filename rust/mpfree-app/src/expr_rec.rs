//! Expression recorded into a clip (the MIDI plan's 3.7, agreed with the
//! sequencer thread on 18 Nov 2026): while the armed track plays outside
//! gear on MIDI out, the keyboard's pitch wheel and channel pressure as the
//! gear is sent them (`MidiOut::expression_sent`), and its mod wheel,
//! breath and foot controller, become moves on the track's `Owner::Midi`
//! lanes (`midi::BEND`, `midi::PRESSURE`, CC 1, 2 and 11) through
//! `TimeHost::gesture`. While the transport records the recorder writes
//! them into the clip; while it rolls the move log keeps them for CAPTURE.
//!
//! A 14-bit wheel can send hundreds of values a second, and every value
//! given becomes a lane point, so they are thinned first, as pad pressure
//! is (`mpfree_play::record`): per control, at most one value a period
//! (its last), given when it is [`STEP`] or more 7-bit steps from the last
//! one given, or one step away once it has lasted [`SETTLE_S`]. A
//! gesture's first value is always given, and so is its end: the wheel
//! back in the middle, the pressure, mod wheel or breath back at 0. A foot
//! controller rests anywhere, so its gesture ends only where the recorder
//! ends it (a punch out or a stop). Another track armed, or the track's
//! gear going, ends what is moving; the transport stopping forgets it (the
//! recorder and the move log end it there).
//!
//! MPE gear and synth tracks are left out: an MPE note bends on its own,
//! and a synth's bend has no lane. Nothing allocates.

use mpfree_play::mpfree_param::{midi, Owner, ParamAddr};
use mpfree_seq::mpfree_moves::{Input, Map, Steps};

/// A value this many 7-bit steps from the last one given is given at once
/// (as pad pressure's `TOUCH_STEP`).
pub const STEP: i32 = mpfree_play::record::TOUCH_STEP as i32;
/// A value one step from the last one given is given once it has lasted
/// this long, in seconds (as pad pressure's `TOUCH_SETTLE_MS`).
pub const SETTLE_S: f64 = mpfree_play::record::TOUCH_SETTLE_MS as f64 / 1000.0;

/// The controls followed: the parameter, its units in one 7-bit step, and
/// whether it rests at 0 (its gesture ends there).
const CONTROLS: [(u16, i32, bool); 5] = [
    (midi::BEND, 64, true),
    (midi::PRESSURE, 1, true),
    (1, 1, true),   // the mod wheel
    (2, 1, true),   // breath
    (11, 1, false), // a foot controller
];
/// The first of [`CONTROLS`] that is a controller (after the bend and the
/// pressure).
const FIRST_CC: usize = 2;

/// One control's state.
#[derive(Clone, Copy, Debug, Default)]
struct Follow {
    /// Its value now (this period's last), in the parameter's own units;
    /// None until the gear is sent one.
    now: Option<i32>,
    /// A gesture is under way: the last value given.
    given: Option<i32>,
    /// A value one step from `given`, waiting to last [`SETTLE_S`]: the
    /// value and when it came.
    wait: Option<(i32, f64)>,
}

/// What the keys' expression gives the recorder; see the module notes.
#[derive(Debug, Default)]
pub struct ExprRec {
    /// The track the controls are followed on.
    track: Option<u8>,
    follow: [Follow; CONTROLS.len()],
}

impl ExprRec {
    pub fn new() -> ExprRec {
        ExprRec::default()
    }

    /// The bend (-8192 to 8191) and channel pressure (0 to 127, None for
    /// gear that takes it note by note) the gear was sent, once a period.
    pub fn sent(&mut self, bend: i16, pressure: Option<u8>) {
        self.follow[0].now = Some(i32::from(bend));
        self.follow[1].now = pressure.map(i32::from);
    }

    /// The gear was sent controller `cc` at `value` (0 to 127) from the
    /// keyboard. Controllers other than the mod wheel, breath and foot
    /// are not followed.
    pub fn control(&mut self, cc: u8, value: u8) {
        if let Some(k) = CONTROLS
            .iter()
            .position(|&(id, _, _)| id < midi::CONTROLLERS && id == u16::from(cc))
        {
            self.follow[k].now = Some(i32::from(value.min(127)));
        }
    }

    /// Once a period, after the keys: `track` is the armed track while
    /// its gear takes the keys' expression (None otherwise), `rolling`
    /// whether the transport rolls, `t` seconds. Gives `give` what to
    /// record, for `TimeHost::gesture`.
    pub fn period(
        &mut self,
        track: Option<u8>,
        rolling: bool,
        t: f64,
        mut give: impl FnMut(Input),
    ) {
        if track != self.track {
            if let Some(old) = self.track {
                for (k, f) in self.follow.iter_mut().enumerate() {
                    if f.given.is_some() && rolling {
                        give(Input::End { addr: addr(old, k) });
                    }
                    // The new track's gear hasn't been sent the old one's
                    // controllers; its bend and pressure come with `sent`.
                    if k >= FIRST_CC {
                        f.now = None;
                    }
                }
            }
            self.track = track;
            for f in &mut self.follow {
                (f.given, f.wait) = (None, None);
            }
        }
        if !rolling {
            // The recorder and the move log end what moved at the stop.
            for f in &mut self.follow {
                (f.given, f.wait) = (None, None);
            }
            return;
        }
        let Some(track) = track else { return };
        for (k, f) in self.follow.iter_mut().enumerate() {
            let Some(v) = f.now else { continue };
            let (_, unit, rests) = CONTROLS[k];
            let at_rest = rests && v == 0;
            let Some(g) = f.given else {
                if !at_rest {
                    give(moved(track, k, v));
                    (f.given, f.wait) = (Some(v), None);
                }
                continue;
            };
            if at_rest {
                give(Input::End {
                    addr: addr(track, k),
                });
                (f.given, f.wait) = (None, None);
                continue;
            }
            let steps = (v - g).abs() / unit;
            if steps >= STEP {
                give(moved(track, k, v));
                (f.given, f.wait) = (Some(v), None);
            } else if steps == 0 {
                f.wait = None;
            } else {
                match f.wait {
                    Some((w, since)) if (w - v).abs() / unit == 0 => {
                        if t - since >= SETTLE_S {
                            give(moved(track, k, v));
                            (f.given, f.wait) = (Some(v), None);
                        }
                    }
                    _ => f.wait = Some((v, t)),
                }
            }
        }
    }
}

/// Control `k`'s parameter on track `track`.
fn addr(track: u8, k: usize) -> ParamAddr {
    ParamAddr::new(Owner::Midi(track), CONTROLS[k].0)
}

/// Control `k` on track `track` at `v`, as a move.
fn moved(track: u8, k: usize, v: i32) -> Input {
    let id = CONTROLS[k].0;
    let (min, max) = midi::range(id).unwrap_or((0, 127));
    Input::Move {
        addr: ParamAddr::new(Owner::Midi(track), id),
        value: v as f32,
        map: Map {
            min: f32::from(min),
            max: f32::from(max),
            log: false,
            steps: Steps::Whole,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One period's gifts, as (parameter, value or None for an end).
    fn run(r: &mut ExprRec, track: Option<u8>, t: f64) -> Vec<(u16, Option<f32>)> {
        let mut out = Vec::new();
        r.period(track, true, t, |i| {
            out.push(match i {
                Input::Move { addr, value, .. } => (addr.param, Some(value)),
                Input::End { addr } => (addr.param, None),
            })
        });
        out
    }

    /// The wheel: its first value at once, then only moves of two 7-bit
    /// steps or more, a one-step move once it has lasted 10 ms, and its
    /// end back in the middle.
    #[test]
    fn the_wheel_is_thinned_and_ends_in_the_middle() {
        let mut r = ExprRec::new();
        let p = Some(0);
        r.sent(0, Some(0));
        assert!(run(&mut r, p, 0.0).is_empty(), "at rest: nothing");
        r.sent(1000, Some(0));
        assert_eq!(run(&mut r, p, 0.001), [(midi::BEND, Some(1000.0))]);
        // Less than a step: nothing; one step: waits; two: at once.
        r.sent(1050, Some(0));
        assert!(run(&mut r, p, 0.002).is_empty());
        r.sent(1070, Some(0));
        assert!(run(&mut r, p, 0.003).is_empty(), "one step waits");
        r.sent(1075, Some(0));
        assert!(run(&mut r, p, 0.008).is_empty(), "not 10 ms yet");
        assert_eq!(
            run(&mut r, p, 0.0131),
            [(midi::BEND, Some(1075.0))],
            "it lasted"
        );
        r.sent(1075 + 128, Some(0));
        assert_eq!(run(&mut r, p, 0.014), [(midi::BEND, Some(1203.0))]);
        // A one-step jitter back and forth is never given.
        for (k, v) in [1203 + 64, 1203, 1203 + 64, 1203].into_iter().enumerate() {
            r.sent(v, Some(0));
            assert!(run(&mut r, p, 0.015 + 0.004 * k as f64).is_empty());
        }
        r.sent(0, Some(0));
        assert_eq!(run(&mut r, p, 0.05), [(midi::BEND, None)]);
        assert!(run(&mut r, p, 0.06).is_empty());
    }

    /// Pressure and the mod wheel end at 0; a foot controller only where
    /// the recorder ends it; another track armed ends what moves; the
    /// stop forgets it, so the next roll starts again.
    #[test]
    fn controls_end_at_rest_on_another_track_and_start_again_after_a_stop() {
        let mut r = ExprRec::new();
        let p = Some(3);
        r.sent(0, Some(40));
        r.control(1, 64);
        r.control(11, 100);
        r.control(64, 127); // a pedal: not followed
        assert_eq!(
            run(&mut r, p, 0.0),
            [
                (midi::PRESSURE, Some(40.0)),
                (1, Some(64.0)),
                (11, Some(100.0))
            ]
        );
        r.sent(0, Some(0));
        r.control(1, 0);
        r.control(11, 0);
        assert_eq!(
            run(&mut r, p, 0.01),
            [(midi::PRESSURE, None), (1, None), (11, Some(0.0))]
        );
        // Track 4 armed: the foot's gesture on track 3 ends there.
        let mut ended = Vec::new();
        r.sent(0, Some(0));
        r.period(Some(4), true, 0.02, |i| ended.push(i));
        assert_eq!(
            ended,
            [Input::End {
                addr: ParamAddr::new(Owner::Midi(3), 11)
            }]
        );
        // The new track's gear has had no mod wheel yet.
        r.sent(500, None);
        assert_eq!(run(&mut r, Some(4), 0.03), [(midi::BEND, Some(500.0))]);
        // Stopped: nothing given, nothing ended; rolling again with the
        // wheel still held starts a new gesture.
        let mut none = 0;
        r.period(Some(4), false, 0.04, |_| none += 1);
        assert_eq!(none, 0);
        assert_eq!(run(&mut r, Some(4), 0.05), [(midi::BEND, Some(500.0))]);
        // The gear gone (None): its moving wheel ends.
        assert_eq!(run(&mut r, None, 0.06), [(midi::BEND, None)]);
    }

    /// A move names the track's MIDI lane with the parameter's own range.
    #[test]
    fn a_move_maps_the_bends_whole_range() {
        let Input::Move { addr, map, .. } = moved(2, 0, -8192) else {
            unreachable!()
        };
        assert_eq!(addr, ParamAddr::new(Owner::Midi(2), midi::BEND));
        assert_eq!((map.min, map.max), (-8192.0, 8191.0));
        assert_eq!(map, Map::of(&crate::modmix::midi_param(addr).unwrap().0));
    }
}
