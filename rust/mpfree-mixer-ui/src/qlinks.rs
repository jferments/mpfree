//! The Q-Links while the mixer is open. The four knobs work on the selected
//! strip: Q-Link 1 its Volume, 2 its Pan, 3 Send A and 4 Send B. A return
//! and the master have no sends, so Q-Links 3 and 4 have nothing to move
//! there, nor do any of them when the screen does not show the selected
//! strip (another view is open, or nothing is selected).
//!
//! [`show`] gives what each knob's card says. [`turn`] (a turn of some
//! detents) and [`to`] (a knob or slider at a place, 0 to 1) give the
//! [`MixAction::Set`] for the mixer, or `None` when nothing changes.
//! [`crate::qlinks`](fn@crate::qlinks), [`crate::qlink_turn`] and [`crate::qlink_to`] do the
//! same through the mixer widget, which shows the change at once.

use mpfree_fx::ids::mixer as mx;
use mpfree_fx::{StripId, SILENT_DB};

use crate::state::{MixState, StripView};
use crate::view::{db_text, db_to_pos, pan_text, pos_to_db, MixAction};

/// The knobs' names, Q-Link 1 to 4.
pub const LABELS: [&str; 4] = ["Volume", "Pan", "Send A", "Send B"];

/// A level's step per detent, as a share of the fader's travel (about
/// 0.4 dB near 0 dB, more further down, where the fader's law is coarser).
/// To tune on the device, against how many detents a turn of the knob has.
pub const LEVEL_STEP: f32 = 0.01;
/// A level's fine step (SHIFT held), dB per detent.
pub const LEVEL_FINE_DB: f32 = 0.1;
/// Pan's step and fine step per detent (pan runs -100 to 100).
pub const PAN_STEP: f32 = 2.0;
pub const PAN_FINE: f32 = 1.0;

/// The top of a level's range, dB.
const MAX_DB: f32 = 6.0;

/// One Q-Link's card.
#[derive(Clone, Debug, PartialEq)]
pub struct QLinkView {
    /// "Volume", "Pan", "Send A" or "Send B".
    pub label: &'static str,
    /// The value as the strip writes it: "-3.0 dB", "off", "C", "L40".
    /// Empty when the knob has nothing to move.
    pub text: String,
    /// "pre" for a send taken before the fader, else empty.
    pub sub: &'static str,
    /// The parameter's own value: dB for a level, -100 to 100 for pan.
    pub value: f32,
    /// How far round the knob's arc is lit, 0 to 1: a level where the
    /// fader would be, pan from hard left.
    pub pos: f32,
    /// The mixer parameter it moves (`mpfree_fx::ids::mixer`); `None` when
    /// it has nothing to move.
    pub id: Option<u16>,
}

impl QLinkView {
    pub fn enabled(&self) -> bool {
        self.id.is_some()
    }

    fn off(label: &'static str) -> QLinkView {
        QLinkView {
            label,
            text: String::new(),
            sub: "",
            value: 0.0,
            pos: 0.0,
            id: None,
        }
    }
}

/// The four Q-Links and the strip they work on.
#[derive(Clone, Debug, PartialEq)]
pub struct QLinks {
    /// The selected strip, when the screen shows it.
    pub strip: Option<StripId>,
    /// Its name, for a heading over the knobs ("Bass"); empty when none.
    pub name: String,
    pub knobs: [QLinkView; 4],
}

/// The strip the knobs work on: the selected one, if `st` shows it.
pub fn strip(st: &MixState) -> Option<&StripView> {
    st.selected.and_then(|s| st.strip(s))
}

/// What each Q-Link's card says now.
pub fn show(st: &MixState) -> QLinks {
    let Some(s) = strip(st) else {
        return QLinks {
            strip: None,
            name: String::new(),
            knobs: LABELS.map(QLinkView::off),
        };
    };
    let level = |label, id, db: f32, sub| QLinkView {
        label,
        text: db_text(db),
        sub,
        value: db,
        pos: db_to_pos(db),
        id: Some(id),
    };
    let send = |r: usize| match s.sends {
        Some(x) => level(
            LABELS[2 + r],
            mx::send(r),
            x[r].db,
            if x[r].pre { "pre" } else { "" },
        ),
        None => QLinkView::off(LABELS[2 + r]),
    };
    QLinks {
        strip: Some(s.id),
        name: s.name.clone(),
        knobs: [
            level(LABELS[0], mx::VOLUME, s.volume_db, ""),
            QLinkView {
                label: LABELS[1],
                text: pan_text(s.pan),
                sub: "",
                value: s.pan,
                pos: ((s.pan + 100.0) / 200.0).clamp(0.0, 1.0),
                id: Some(mx::PAN),
            },
            send(0),
            send(1),
        ],
    }
}

/// Q-Link `knob` (0 to 3) turned by `detents` (up is positive); `fine`
/// (SHIFT held) takes small steps. `None` when nothing changes: the knob
/// has nothing to move, the value is at its end, or `detents` is 0.
pub fn turn(st: &MixState, knob: usize, detents: i32, fine: bool) -> Option<MixAction> {
    let (s, k) = target(st, knob)?;
    if detents == 0 {
        return None;
    }
    let d = detents as f32;
    let value = if k.id == Some(mx::PAN) {
        let step = if fine { PAN_FINE } else { PAN_STEP };
        (k.value + d * step).clamp(-100.0, 100.0).round()
    } else if fine {
        let db = ((k.value.max(SILENT_DB) + d * LEVEL_FINE_DB) * 10.0).round() / 10.0;
        if db <= SILENT_DB {
            SILENT_DB
        } else {
            db.min(MAX_DB)
        }
    } else {
        // The detent at 0 dB holds a step that lands near it; a turn always
        // moves, so take more of the travel until the level changes.
        let p = db_to_pos(k.value);
        (1..=4)
            .map(|n| pos_to_db(p + d * n as f32 * LEVEL_STEP))
            .find(|v| *v != k.value)
            .unwrap_or(k.value)
    };
    set(s, k, value)
}

/// Q-Link `knob` (0 to 3) put at `pos` (0 to 1, as a knob read as a
/// position, or a slider): a level by the fader's law, pan from hard left.
pub fn to(st: &MixState, knob: usize, pos: f32) -> Option<MixAction> {
    let (s, k) = target(st, knob)?;
    let pos = pos.clamp(0.0, 1.0);
    let value = if k.id == Some(mx::PAN) {
        (pos * 200.0 - 100.0).round()
    } else {
        pos_to_db(pos)
    };
    set(s, k, value)
}

fn target(st: &MixState, knob: usize) -> Option<(StripId, QLinkView)> {
    let q = show(st);
    let k = q.knobs.get(knob)?.clone();
    k.enabled().then_some((q.strip?, k))
}

fn set(strip: StripId, k: QLinkView, value: f32) -> Option<MixAction> {
    (value != k.value).then_some(MixAction::Set {
        strip,
        id: k.id?,
        value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{SendView, StripView};
    use mpfree_fx::RETURNS;

    fn state() -> MixState {
        let mut st = MixState {
            strips: (0..4)
                .map(|t| StripView::new(StripId::Track(t), format!("Track {}", t + 1)))
                .collect(),
            ..MixState::default()
        };
        st.strips[3].name = "Bass".into();
        st.strips[3].volume_db = -3.0;
        st.strips[3].pan = -40.0;
        let mut sends = [SendView::default(); RETURNS];
        sends[0] = SendView {
            db: -12.0,
            pre: true,
        };
        st.strips[3].sends = Some(sends);
        st.strips.push(StripView::new(StripId::Return(0), "Reverb"));
        st.selected = Some(StripId::Track(3));
        st
    }

    fn value(a: Option<MixAction>) -> Option<f32> {
        match a? {
            MixAction::Set { value, .. } => Some(value),
            _ => None,
        }
    }

    #[test]
    fn the_cards_show_the_selected_strip() {
        let mut st = state();
        let q = show(&st);
        assert_eq!(q.strip, Some(StripId::Track(3)));
        assert_eq!(q.name, "Bass");
        let texts: Vec<(&str, &str, &str)> = q
            .knobs
            .iter()
            .map(|k| (k.label, k.text.as_str(), k.sub))
            .collect();
        assert_eq!(
            texts,
            [
                ("Volume", "-3.0 dB", ""),
                ("Pan", "L40", ""),
                ("Send A", "-12.0 dB", "pre"),
                ("Send B", "off", "")
            ]
        );
        assert_eq!(
            q.knobs.each_ref().map(|k| k.id),
            [
                Some(mx::VOLUME),
                Some(mx::PAN),
                Some(mx::send(0)),
                Some(mx::send(1))
            ]
        );
        assert!((q.knobs[1].pos - 0.3).abs() < 1e-6);
        assert_eq!(q.knobs[3].pos, 0.0);
        // A return has no sends: Q-Links 3 and 4 are off.
        st.selected = Some(StripId::Return(0));
        let q = show(&st);
        assert_eq!(q.name, "Reverb");
        assert_eq!(
            q.knobs.iter().map(|k| k.enabled()).collect::<Vec<_>>(),
            [true, true, false, false]
        );
        assert_eq!(q.knobs[2].text, "");
        // The master is always on the screen.
        st.selected = Some(StripId::Master);
        assert_eq!(show(&st).strip, Some(StripId::Master));
        // A strip this view does not show, or none: nothing to move.
        for sel in [Some(StripId::Pad(0)), None] {
            st.selected = sel;
            let q = show(&st);
            assert_eq!((q.strip, q.name.as_str()), (None, ""));
            assert!(q.knobs.iter().all(|k| !k.enabled() && k.text.is_empty()));
            assert_eq!(q.knobs.each_ref().map(|k| k.label), LABELS);
            assert_eq!(turn(&st, 0, 1, false), None);
        }
    }

    #[test]
    fn turns_step_levels_by_the_fader_law_and_pan_by_two() {
        let mut st = state();
        assert_eq!(
            turn(&st, 0, 1, false),
            Some(MixAction::Set {
                strip: StripId::Track(3),
                id: mx::VOLUME,
                value: -2.6
            })
        );
        assert_eq!(value(turn(&st, 0, -1, false)), Some(-3.4));
        assert_eq!(value(turn(&st, 0, 1, true)), Some(-2.9));
        assert_eq!(value(turn(&st, 0, -5, true)), Some(-3.5));
        assert_eq!(value(turn(&st, 1, 3, false)), Some(-34.0));
        assert_eq!(value(turn(&st, 1, 3, true)), Some(-37.0));
        assert_eq!(turn(&st, 1, 0, false), None);
        assert_eq!(
            value(turn(&st, 3, 1, false)),
            Some(-78.0),
            "a send off comes up from the bottom"
        );
        assert_eq!(value(turn(&st, 3, 1, true)), Some(-79.9));
        assert_eq!(turn(&st, 3, -1, false), None, "already off");
        assert_eq!(turn(&st, 4, 1, false), None);
        // Coarse turns stop at 0 dB on the way through, and every detent
        // changes the level until the end.
        st.strips[3].volume_db = -0.2;
        assert_eq!(value(turn(&st, 0, 1, false)), Some(0.0));
        let mut db = -80.0;
        for _ in 0..200 {
            st.strips[3].volume_db = db;
            match value(turn(&st, 0, 1, false)) {
                Some(v) => {
                    assert!(v > db, "{db} -> {v}");
                    db = v;
                }
                None => break,
            }
        }
        assert_eq!(db, 6.0);
        st.strips[3].volume_db = 5.95;
        assert_eq!(value(turn(&st, 0, 1, true)), Some(6.0));
        st.strips[3].pan = 99.0;
        assert_eq!(value(turn(&st, 1, 1, false)), Some(100.0));
        st.strips[3].pan = 100.0;
        assert_eq!(turn(&st, 1, 1, false), None);
        // A knob or slider at a place.
        assert_eq!(value(to(&st, 0, 0.8)), Some(0.0));
        assert_eq!(value(to(&st, 0, 0.0)), Some(SILENT_DB));
        assert_eq!(value(to(&st, 1, 0.5)), Some(0.0));
        assert_eq!(to(&st, 1, 1.0), None, "pan is already hard right");
        assert_eq!(value(to(&st, 2, 1.0)), Some(6.0));
        st.selected = Some(StripId::Return(0));
        assert_eq!(to(&st, 2, 1.0), None);
    }
}
