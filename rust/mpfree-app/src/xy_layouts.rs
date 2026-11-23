//! The XY pad's ten ready-made layouts, the choices on its chip
//! (`xy_list.rs`): Filter + echo, and the nine effects of the XY FX. Each
//! is an `mpfree_xy` layout (the XY controller plan, phase 1), and plays
//! exactly what the pad played before the engine took it over
//! (`mixing::tests::the_ten_ready_made_xy_layouts_send_what_they_sent_before`).
//!
//! - **Filter + echo** (no pick): across, the Perform FX's DJ filter on the
//!   master from -100 (low-pass) to 100 (high-pass), off in the middle;
//!   up, its echo from 0 to 100. It rests in the middle at the bottom and
//!   glides back there when the finger lifts.
//! - **The nine XY FX** (pick 0 to 8): the master's XY FX insert, its X
//!   across and Y up, each 0 to 100, its Active the gate (the effect opens
//!   when the finger lands and lets go when it lifts). The dot stays where
//!   the finger lifts; it starts in the middle a quarter of the way up.
//!   Which of the nine plays is the insert's Effect, set when it is picked
//!   (`PlayFx::XyEffect`).
//!
//! The settings are named by the insert's slot id on the master (`uid`),
//! so they follow the insert if it moves.

use mpfree_fx::ids::perform as pf;
use mpfree_fx::xyfx::ids as xy;
use mpfree_play::mpfree_param::{Owner, ParamAddr, Strip};
use mpfree_xy::{Axis, Compiled, Layout, Lift, Pad, Target};

/// How fast Filter + echo glides back: about this many ms to cover 63 % of
/// the way.
pub const SNAP_MS: f32 = 60.0;

/// The insert setting `id` on the master's insert with slot id `uid`.
fn master(uid: u16, id: u16) -> ParamAddr {
    ParamAddr::new(Owner::Insert(Strip::Master, uid), id)
}

/// Whether `pick` is one of the nine XY FX.
fn xy_fx(pick: Option<usize>) -> bool {
    pick.is_some_and(|k| k < mpfree_fx::xyfx::EFFECTS.len())
}

/// The settings across and up for `pick`, on the insert with slot id
/// `uid` (the Perform FX for Filter + echo, the XY FX for the nine).
fn targets(pick: Option<usize>, uid: u16) -> ([Target; 1], [Target; 1]) {
    if xy_fx(pick) {
        (
            [Target::linear(master(uid, xy::X), 0.0, 100.0)],
            [Target::linear(master(uid, xy::Y), 0.0, 100.0)],
        )
    } else {
        (
            [Target::linear(master(uid, pf::DJ_FILTER), -100.0, 100.0)],
            [Target::linear(master(uid, pf::ECHO), 0.0, 100.0)],
        )
    }
}

/// `pick`'s layout as the sound loop plays it, on the insert with slot id
/// `uid`. Allocates nothing.
pub fn compiled(pick: Option<usize>, uid: u16) -> Compiled {
    let (across, up) = targets(pick, uid);
    let gate = xy_fx(pick).then(|| master(uid, xy::ACTIVE));
    Compiled::new(&across, &up, gate).expect("the ready-made layouts check")
}

/// The pad for `pick`: where it rests and what it does when the finger
/// lifts.
pub fn pad(pick: Option<usize>) -> Pad {
    if xy_fx(pick) {
        Pad::new((0.5, 0.25), Lift::Stay)
    } else {
        Pad::new((0.5, 0.0), Lift::GoBack { snap_ms: SNAP_MS })
    }
}

/// `pick` as a layout of yours, on the insert with slot id `uid`, called
/// `name` (Setup on a ready-made layout: a copy becomes yours and the
/// ready-made one stays as it was). Filter + echo goes back over
/// [`COPY_GLIDE_MS`]; an XY FX's copy keeps its gate and stays (its
/// effect is set on the insert when it plays: `xy_song::Mine::fx`).
pub fn layout(pick: Option<usize>, uid: u16, name: &str) -> Layout {
    let (across, up) = targets(pick, uid);
    let (x, y) = match pick.filter(|_| xy_fx(pick)) {
        Some(k) => {
            let (ax, ay) = mpfree_fx::xyfx::axes(k);
            (ax.name, ay.name)
        }
        None => ("Filter", "Echo"),
    };
    let fx = xy_fx(pick);
    Layout {
        across: Axis::new(x, &across),
        up: Axis::new(y, &up),
        gate: fx.then(|| master(uid, xy::ACTIVE)),
        rest: pad(pick).rest(),
        lift: if fx {
            Lift::Stay
        } else {
            Lift::GoBack {
                snap_ms: COPY_GLIDE_MS,
            }
        },
        ..Layout::new(name)
    }
}

/// How long a copy of Filter + echo glides back: its pad lands within a
/// hundredth in about five times [`SNAP_MS`].
pub const COPY_GLIDE_MS: f32 = 5.0 * SNAP_MS;

/// Filter + echo's two values with the pad at `x`, `y`: the DJ filter
/// (-100 to 100) and the echo (0 to 100), for its label.
pub fn filter_echo(x: f32, y: f32) -> (f32, f32) {
    let (across, up) = targets(None, 0);
    (across[0].value(x), up[0].value(y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ten_layouts_play_their_settings_and_read_back_where_they_stand() {
        let fe = compiled(None, 7);
        let mut got = Vec::new();
        fe.values(0.25, 0.5, true, |a, v| got.push((a, v)));
        assert_eq!(
            got,
            [
                (master(7, pf::DJ_FILTER), -50.0),
                (master(7, pf::ECHO), 50.0)
            ]
        );
        for k in 0..9 {
            let c = compiled(Some(k), 3);
            let mut got = Vec::new();
            c.values(0.1, 0.9, false, |a, v| got.push((a, v)));
            assert_eq!(got.len(), 3, "{k}");
            assert_eq!(got[2], (master(3, xy::ACTIVE), 0.0), "the gate, last");
            let at = c.position(|a| got.iter().find(|g| g.0 == a).map(|g| g.1));
            assert!((at.0.unwrap() - 0.1).abs() < 1e-5 && (at.1.unwrap() - 0.9).abs() < 1e-5);
            assert!(pad(Some(k)).stays());
        }
        assert!(!pad(None).stays());
        assert_eq!(pad(None).rest(), (0.5, 0.0));
        assert_eq!(filter_echo(1.0, 0.25), (100.0, 25.0));
        // Past the nine: Filter + echo.
        assert_eq!(compiled(Some(9), 7), fe);
    }

    /// A copy plays what the ready-made one plays, as a layout of yours.
    #[test]
    fn a_copy_of_a_ready_made_layout_plays_the_same_settings() {
        for pick in [None, Some(0), Some(4)] {
            let l = layout(pick, 7, "Mine");
            let c = l.compile().unwrap();
            let r = compiled(pick, 7);
            let (mut a, mut b) = (Vec::new(), Vec::new());
            c.values(0.3, 0.6, true, |x, v| a.push((x, v)));
            r.values(0.3, 0.6, true, |x, v| b.push((x, v)));
            assert_eq!(a, b, "{pick:?}");
            assert_eq!(c.rest(), pad(pick).rest());
        }
        let wash = layout(Some(3), 2, "Wash 2");
        assert_eq!(
            (wash.across.name.as_str(), wash.lift),
            ("Decay", Lift::Stay)
        );
        assert_eq!(layout(None, 2, "F").up.name, "Echo");
    }
}
