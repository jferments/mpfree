//! M11 synthetic event-stream tests. No device access: every stream below is
//! hand-written in the shape the Linux input core emits for a Type-B device.

use mpfree_touch::gesture::{Gesture, GestureConfig, GestureRecognizer};
use mpfree_touch::mt::*;
use mpfree_touch::pointer::{AxisMap, Orientation, PointerEvent, RawRange};

const SLOT: u16 = 47;
const X: u16 = 53;
const Y: u16 = 54;
const TID: u16 = 57;
const ABS_X: u16 = 0;

/// Tiny stream builder: `S::new().slot(0).id(5).xy(10, 20).syn()`.
#[derive(Default)]
struct S {
    t: u64,
    ev: Vec<InputEvent>,
}

impl S {
    fn new() -> Self {
        Self::default()
    }
    fn at(mut self, t: u64) -> Self {
        self.t = t;
        self
    }
    fn raw(mut self, kind: u16, code: u16, value: i32) -> Self {
        self.ev.push(InputEvent::new(self.t, kind, code, value));
        self
    }
    fn slot(self, s: i32) -> Self {
        self.raw(EV_ABS, SLOT, s)
    }
    fn id(self, id: i32) -> Self {
        self.raw(EV_ABS, TID, id)
    }
    fn x(self, v: i32) -> Self {
        self.raw(EV_ABS, X, v)
    }
    fn y(self, v: i32) -> Self {
        self.raw(EV_ABS, Y, v)
    }
    fn xy(self, x: i32, y: i32) -> Self {
        self.x(x).y(y)
    }
    fn btn(self, down: bool) -> Self {
        self.raw(EV_KEY, BTN_TOUCH, i32::from(down))
    }
    fn syn(self) -> Self {
        self.raw(EV_SYN, SYN_REPORT, 0)
    }
    fn dropped(self) -> Self {
        self.raw(EV_SYN, SYN_DROPPED, 0)
    }
}

/// (phase, kernel tracking id, raw x, raw y)
type Seen = (Phase, i32, i32, i32);

/// Feed a stream; return (step, events seen) per committed frame.
fn feed(d: &mut MtDecoder, s: S) -> Vec<(Step, Vec<Seen>)> {
    let mut frames = Vec::new();
    for ev in s.ev {
        let step = d.push(ev);
        if step != Step::Pending {
            let evs = if step == Step::Frame {
                d.events()
                    .iter()
                    .map(|e| {
                        (
                            e.phase,
                            e.contact.tracking_id,
                            e.contact.raw_x,
                            e.contact.raw_y,
                        )
                    })
                    .collect()
            } else {
                Vec::new()
            };
            frames.push((step, evs));
        }
    }
    frames
}

fn decoder() -> MtDecoder {
    MtDecoder::new(10).unwrap()
}

use Phase::*;

#[test]
fn rejects_bad_slot_counts() {
    assert_eq!(
        MtDecoder::new(0).unwrap_err(),
        ConfigError::InvalidSlotCount
    );
    assert_eq!(
        MtDecoder::new(65).unwrap_err(),
        ConfigError::InvalidSlotCount
    );
    assert_eq!(MtDecoder::new(64).unwrap().slot_count(), 64);
}

#[test]
fn single_contact_down_move_up_with_implicit_slot_zero() {
    let mut d = decoder();
    let f = feed(
        &mut d,
        S::new()
            .id(7)
            .xy(100, 200)
            .btn(true)
            .raw(EV_ABS, ABS_X, 100)
            .syn()
            .x(110)
            .syn()
            .x(110)
            .syn() // duplicate value: no Move
            .id(-1)
            .btn(false)
            .syn(),
    );
    assert_eq!(f[0].1, vec![(Down, 7, 100, 200)]);
    assert_eq!(f[1].1, vec![(Move, 7, 110, 200)]);
    assert_eq!(f[2].1, vec![]);
    assert_eq!(f[3].1, vec![(Up, 7, 110, 200)]);
    assert!(d.contacts().is_empty());
    let st = d.stats();
    assert_eq!(st.frames, 4);
    assert_eq!(st.btn_touch_mismatches, 0);
    assert_eq!(st.ignored_events, 1); // the ABS_X single-touch echo
}

#[test]
fn two_contacts_publish_coherent_snapshots() {
    let mut d = decoder();
    feed(
        &mut d,
        S::new()
            .slot(0)
            .id(1)
            .xy(10, 10)
            .slot(3)
            .id(2)
            .xy(500, 900)
            .syn(),
    );
    let c = d.contacts();
    assert_eq!(c.len(), 2);
    assert_eq!((c[0].slot, c[0].tracking_id), (0, 1));
    assert_eq!((c[1].slot, c[1].tracking_id), (3, 2));
    assert_ne!(c[0].id, c[1].id);
    // Only slot 3 moves; slot 3 stays selected across frames.
    let f = feed(&mut d, S::new().y(950).syn());
    assert_eq!(f[0].1, vec![(Move, 2, 500, 950)]);
    assert_eq!(d.contacts().len(), 2);
}

#[test]
fn slot_reused_by_new_contact_keeps_unrepeated_coordinates() {
    let mut d = decoder();
    feed(&mut d, S::new().id(1).xy(300, 400).syn().id(-1).syn());
    let first = d.events()[0].contact.id;
    // The kernel drops duplicate per-slot values: a new finger at the same X
    // only sends Y. The X must be inherited from the slot, not lost.
    let f = feed(&mut d, S::new().id(2).y(410).syn());
    assert_eq!(f[0].1, vec![(Down, 2, 300, 410)]);
    assert_ne!(d.contacts()[0].id, first);
}

#[test]
fn direct_tracking_id_switch_is_up_then_down_in_one_frame() {
    let mut d = decoder();
    feed(&mut d, S::new().id(1).xy(1, 1).syn());
    let f = feed(&mut d, S::new().id(2).xy(5, 5).syn());
    assert_eq!(f[0].1, vec![(Up, 1, 1, 1), (Down, 2, 5, 5)]);
}

#[test]
fn contact_born_and_ended_inside_one_frame_is_invisible() {
    let mut d = decoder();
    let f = feed(&mut d, S::new().id(1).xy(1, 1).id(-1).syn());
    assert_eq!(f[0].1, vec![]);
}

#[test]
fn tracking_id_wraparound_still_yields_fresh_contact_ids() {
    let mut d = decoder();
    feed(&mut d, S::new().id(65535).xy(1, 1).syn().id(-1).syn());
    let a = d.events()[0].contact.id;
    feed(&mut d, S::new().id(0).xy(1, 1).syn());
    let b = d.events()[0].contact.id;
    assert!(b > a);
}

#[test]
fn malformed_sequences_are_contained() {
    let mut d = decoder();
    let f = feed(
        &mut d,
        S::new()
            .slot(10)
            .id(1)
            .xy(1, 1) // beyond 10 slots: ignored
            .slot(-1)
            .id(2)
            .xy(2, 2) // negative: ignored
            .raw(EV_ABS, 99, 5) // unknown axis
            .raw(0x05, 0, 1) // EV_SW
            .slot(1)
            .id(-1) // release with no contact
            .id(3)
            .xy(3, 3)
            .syn(),
    );
    assert_eq!(f[0].1, vec![(Down, 3, 3, 3)]);
    let st = d.stats();
    assert_eq!(st.out_of_range_slots, 2);
    assert_eq!(st.ignored_events, 8);
}

#[test]
fn no_frame_is_published_without_syn_report() {
    let mut d = decoder();
    let f = feed(&mut d, S::new().id(1).xy(1, 1));
    assert!(f.is_empty());
    assert!(d.contacts().is_empty());
}

#[test]
fn btn_touch_disagreement_is_counted_not_trusted() {
    let mut d = decoder();
    feed(&mut d, S::new().btn(true).syn());
    assert!(d.contacts().is_empty());
    assert_eq!(d.stats().btn_touch_mismatches, 1);
}

#[test]
fn syn_dropped_discards_until_report_then_requests_resync() {
    let mut d = decoder();
    feed(
        &mut d,
        S::new()
            .slot(0)
            .id(1)
            .xy(10, 10)
            .slot(1)
            .id(2)
            .xy(20, 20)
            .syn(),
    );
    let ids: Vec<_> = d.contacts().iter().map(|c| c.id).collect();
    let f = feed(
        &mut d,
        S::new()
            .slot(0)
            .x(11) // partial frame, rolled back by the drop
            .dropped()
            .x(999)
            .id(-1) // lost events: ignored
            .syn(),
    );
    assert_eq!(f, vec![(Step::ResyncRequired, vec![])]);
    assert_eq!(d.stats().sync_drops, 1);
    // Snapshot: slot 0 kept id 1 but moved, slot 1 lifted, slot 4 is new.
    let mut slots = vec![
        SlotSnapshot {
            tracking_id: -1,
            raw_x: 0,
            raw_y: 0
        };
        10
    ];
    slots[0] = SlotSnapshot {
        tracking_id: 1,
        raw_x: 15,
        raw_y: 10,
    };
    slots[1] = SlotSnapshot {
        tracking_id: -1,
        raw_x: 20,
        raw_y: 20,
    };
    slots[4] = SlotSnapshot {
        tracking_id: 9,
        raw_x: 40,
        raw_y: 40,
    };
    assert_eq!(
        d.resync(
            &MtSnapshot {
                current_slot: 4,
                slots
            },
            50
        ),
        Step::Frame
    );
    let got: Vec<_> = d
        .events()
        .iter()
        .map(|e| (e.phase, e.contact.tracking_id))
        .collect();
    assert_eq!(got, vec![(Move, 1), (Up, 2), (Down, 9)]);
    assert_eq!(
        d.contacts()[0].id,
        ids[0],
        "surviving contact keeps its identity"
    );
    // Current slot restored from the snapshot: slot 4 moves without ABS_MT_SLOT.
    let f = feed(&mut d, S::new().x(41).syn());
    assert_eq!(f[0].1, vec![(Move, 9, 41, 40)]);
}

#[test]
fn snapshot_with_replaced_tracking_id_is_up_then_down() {
    let mut d = decoder();
    feed(&mut d, S::new().id(1).xy(1, 1).syn().dropped().syn());
    let mut slots = vec![
        SlotSnapshot {
            tracking_id: -1,
            raw_x: 0,
            raw_y: 0
        };
        10
    ];
    slots[0] = SlotSnapshot {
        tracking_id: 2,
        raw_x: 1,
        raw_y: 1,
    };
    d.resync(
        &MtSnapshot {
            current_slot: 0,
            slots,
        },
        9,
    );
    let got: Vec<_> = d
        .events()
        .iter()
        .map(|e| (e.phase, e.contact.tracking_id))
        .collect();
    assert_eq!(got, vec![(Up, 1), (Down, 2)]);
}

#[test]
fn resync_without_snapshot_cancels_and_waits_for_fresh_contacts() {
    let mut d = decoder();
    feed(
        &mut d,
        S::new().slot(2).id(1).xy(10, 10).syn().dropped().syn(),
    );
    assert_eq!(d.resync_without_snapshot(5), Step::Frame);
    assert_eq!(d.events()[0].phase, Cancel);
    assert!(d.contacts().is_empty());
    // The held finger keeps moving: its slot is stale and current slot is
    // unknown, so nothing is published until a slot and fresh id arrive.
    let f = feed(&mut d, S::new().x(12).syn().slot(2).y(13).syn());
    assert_eq!(f.iter().map(|x| x.1.len()).sum::<usize>(), 0);
    // A fresh contact with only Y reported: X unknown, so it is held back...
    let f = feed(&mut d, S::new().slot(3).id(4).y(30).syn());
    assert_eq!(f[0].1, vec![]);
    // ...until X arrives.
    let f = feed(&mut d, S::new().x(31).syn());
    assert_eq!(f[0].1, vec![(Down, 4, 31, 30)]);
}

#[test]
fn ignoring_resync_request_falls_back_to_cancel_on_next_event() {
    let mut d = decoder();
    feed(&mut d, S::new().id(1).xy(10, 10).syn().dropped().syn());
    let f = feed(&mut d, S::new().slot(1).id(2).xy(5, 5).syn());
    assert_eq!(f[0].1, vec![(Cancel, 1, 10, 10), (Down, 2, 5, 5)]);
}

// ---- pointer + gesture layers ----------------------------------------------

struct Rig {
    d: MtDecoder,
    map: AxisMap,
    g: GestureRecognizer,
    out: Vec<Gesture>,
    buf: Vec<PointerEvent>,
}

impl Rig {
    fn new(cfg: GestureConfig) -> Self {
        let r = RawRange { min: 0, max: 2048 };
        Self {
            d: decoder(),
            // Identity mapping into a 2049x2049 space keeps the arithmetic obvious.
            map: AxisMap::new(r, r, Orientation::default(), 2049, 2049).unwrap(),
            g: GestureRecognizer::new(cfg),
            out: Vec::new(),
            buf: Vec::new(),
        }
    }
    fn run(&mut self, s: S) -> Vec<Gesture> {
        self.out.clear();
        for ev in s.ev {
            if self.d.push(ev) == Step::Frame {
                self.buf.clear();
                self.buf
                    .extend(self.d.events().iter().map(|ce| self.map.pointer(ce)));
                self.g.frame(&self.buf, &mut self.out);
            }
        }
        std::mem::take(&mut self.out)
    }
    fn tick(&mut self, t: u64) -> Vec<Gesture> {
        let mut out = Vec::new();
        self.g.tick(t, &mut out);
        out
    }
}

fn kinds(g: &[Gesture]) -> Vec<&'static str> {
    g.iter()
        .map(|g| match g {
            Gesture::Tap { .. } => "tap",
            Gesture::LongPress { .. } => "long",
            Gesture::DragStart { .. } => "drag-start",
            Gesture::Drag { .. } => "drag",
            Gesture::DragEnd {
                cancelled: false, ..
            } => "drag-end",
            Gesture::DragEnd {
                cancelled: true, ..
            } => "drag-cancel",
            Gesture::Swipe { .. } => "swipe",
            Gesture::TwoFingerStart { .. } => "2f-start",
            Gesture::TwoFinger { .. } => "2f",
            Gesture::TwoFingerEnd { cancelled: false } => "2f-end",
            Gesture::TwoFingerEnd { cancelled: true } => "2f-cancel",
        })
        .collect()
}

#[test]
fn tap_within_slop_and_time() {
    let mut r = Rig::new(GestureConfig::default());
    let g = r.run(
        S::new()
            .at(0)
            .id(1)
            .xy(100, 100)
            .syn()
            .at(50_000)
            .x(105)
            .syn()
            .at(120_000)
            .id(-1)
            .syn(),
    );
    assert_eq!(kinds(&g), ["tap"]);
}

#[test]
fn slow_release_is_neither_tap_nor_long_press_without_hold() {
    let cfg = GestureConfig {
        long_press_us: 10_000_000,
        ..GestureConfig::default()
    };
    let mut r = Rig::new(cfg);
    let g = r.run(
        S::new()
            .at(0)
            .id(1)
            .xy(100, 100)
            .syn()
            .at(400_000)
            .id(-1)
            .syn(),
    );
    assert!(g.is_empty());
}

#[test]
fn long_press_fires_from_tick_and_suppresses_tap() {
    let mut r = Rig::new(GestureConfig::default());
    r.run(S::new().at(0).id(1).xy(100, 100).syn());
    assert!(r.tick(499_999).is_empty());
    assert_eq!(kinds(&r.tick(500_000)), ["long"]);
    assert!(r.tick(900_000).is_empty());
    assert!(r.run(S::new().at(950_000).id(-1).syn()).is_empty());
}

#[test]
fn drag_reports_start_at_origin_and_incremental_deltas() {
    let mut r = Rig::new(GestureConfig::default());
    let g = r.run(
        S::new()
            .at(0)
            .id(1)
            .xy(100, 100)
            .syn()
            .at(10_000)
            .x(130)
            .syn()
            .at(20_000)
            .x(140)
            .syn()
            .at(30_000)
            .id(-1)
            .syn(),
    );
    assert_eq!(kinds(&g), ["drag-start", "drag", "drag", "drag-end"]);
    assert_eq!(
        g[0],
        Gesture::DragStart {
            id: ContactId(1),
            x: 100.0,
            y: 100.0
        }
    );
    let Gesture::Drag { dx, .. } = g[2] else {
        panic!()
    };
    assert_eq!(dx, 10.0);
}

#[test]
fn two_fingers_pan_and_pinch() {
    let mut r = Rig::new(GestureConfig::default());
    let g = r.run(
        S::new()
            .slot(0)
            .id(1)
            .xy(100, 100)
            .slot(1)
            .id(2)
            .xy(300, 100)
            .syn(),
    );
    assert_eq!(
        g,
        vec![Gesture::TwoFingerStart {
            cx: 200.0,
            cy: 100.0
        }]
    );
    // Spread both fingers outward to double the distance.
    let g = r.run(S::new().slot(0).x(0).slot(1).x(400).syn());
    assert_eq!(
        g,
        vec![Gesture::TwoFinger {
            cx: 200.0,
            cy: 100.0,
            dx: 0.0,
            dy: 0.0,
            scale: 2.0
        }]
    );
    // Move both down by 50: pan.
    let g = r.run(S::new().slot(0).y(150).slot(1).y(150).syn());
    assert_eq!(
        g,
        vec![Gesture::TwoFinger {
            cx: 200.0,
            cy: 150.0,
            dx: 0.0,
            dy: 50.0,
            scale: 2.0
        }]
    );
    let g = r.run(S::new().slot(0).id(-1).slot(1).id(-1).syn());
    assert_eq!(kinds(&g), ["2f-end"]);
}

#[test]
fn second_finger_never_hijacks_an_active_drag() {
    let mut r = Rig::new(GestureConfig::default());
    r.run(S::new().slot(0).id(1).xy(100, 100).syn().x(200).syn());
    // A fader is being dragged; a second finger taps a pad.
    let g = r.run(S::new().slot(1).id(2).xy(900, 900).syn().id(-1).syn());
    assert_eq!(kinds(&g), ["tap"]);
    let g = r.run(S::new().slot(0).x(210).syn());
    assert_eq!(kinds(&g), ["drag"]);
}

#[test]
fn two_finger_can_be_disabled_for_independent_touches() {
    let cfg = GestureConfig {
        two_finger: false,
        ..GestureConfig::default()
    };
    let mut r = Rig::new(cfg);
    let g = r.run(
        S::new()
            .slot(0)
            .id(1)
            .xy(100, 100)
            .slot(1)
            .id(2)
            .xy(900, 900)
            .syn()
            .slot(0)
            .id(-1)
            .slot(1)
            .id(-1)
            .syn(),
    );
    assert_eq!(kinds(&g), ["tap", "tap"]);
}

#[test]
fn sync_loss_cancels_gestures_instead_of_completing_them() {
    let mut r = Rig::new(GestureConfig::default());
    r.run(S::new().slot(0).id(1).xy(100, 100).syn().x(300).syn());
    r.run(S::new().slot(1).id(2).xy(900, 900).syn()); // pending tap candidate
    r.d.push(InputEvent::new(0, EV_SYN, SYN_DROPPED, 0));
    assert_eq!(
        r.d.push(InputEvent::new(0, EV_SYN, SYN_REPORT, 0)),
        Step::ResyncRequired
    );
    r.d.resync_without_snapshot(1);
    let mut out = Vec::new();
    let frame: Vec<_> = r.d.events().iter().map(|ce| r.map.pointer(ce)).collect();
    r.g.frame(&frame, &mut out);
    assert_eq!(
        kinds(&out),
        ["drag-cancel"],
        "the pending contact must not tap"
    );
}

// ---- M13: seeding a decoder that opens mid-stream ---------------------------

fn snapshot(current_slot: i32, live: &[(usize, i32, i32, i32)]) -> MtSnapshot {
    let mut slots = vec![
        SlotSnapshot {
            tracking_id: -1,
            raw_x: 0,
            raw_y: 0
        };
        10
    ];
    for &(s, tracking_id, raw_x, raw_y) in live {
        slots[s] = SlotSnapshot {
            tracking_id,
            raw_x,
            raw_y,
        };
    }
    MtSnapshot {
        current_slot,
        slots,
    }
}

#[test]
fn unseeded_reader_files_a_contact_under_the_wrong_slot() {
    // The gap M13 fixes. The kernel last selected slot 3 before we opened, so
    // a new contact there arrives without ABS_MT_SLOT. An unseeded decoder
    // assumes slot 0.
    let mut d = decoder();
    let f = feed(&mut d, S::new().id(7).xy(100, 200).syn());
    assert_eq!(d.contacts()[0].slot, 0);
    assert_eq!(f[0].1, vec![(Down, 7, 100, 200)]);
}

#[test]
fn seeded_reader_uses_the_kernels_current_slot() {
    let mut d = decoder();
    assert_eq!(d.seed(&snapshot(3, &[]), 0), Step::Pending);
    assert!(d.events().is_empty());
    let f = feed(&mut d, S::new().id(7).xy(100, 200).syn());
    assert_eq!(d.contacts()[0].slot, 3);
    assert_eq!(f[0].1, vec![(Down, 7, 100, 200)]);
    // A second finger in another slot, then the first lifts: both land right.
    feed(&mut d, S::new().slot(4).id(8).xy(5, 6).syn());
    let f = feed(&mut d, S::new().slot(3).id(-1).syn());
    assert_eq!(f[0].1, vec![(Up, 7, 100, 200)]);
    assert_eq!(d.contacts()[0].slot, 4);
}

#[test]
fn seeded_positions_cover_values_the_kernel_will_not_resend() {
    // The kernel filters repeats per slot: a new contact at the same X as the
    // slot's last contact sends only Y. Without the seeded X the decoder would
    // hold the contact back.
    let mut d = decoder();
    d.seed(
        &MtSnapshot {
            current_slot: 0,
            slots: vec![
                SlotSnapshot {
                    tracking_id: -1,
                    raw_x: 640,
                    raw_y: 10
                };
                10
            ],
        },
        0,
    );
    let f = feed(&mut d, S::new().id(9).y(400).syn());
    assert_eq!(f[0].1, vec![(Down, 9, 640, 400)]);
}

#[test]
fn fingers_already_down_at_open_are_ignored_until_they_lift() {
    let mut d = decoder();
    assert_eq!(d.seed(&snapshot(2, &[(2, 40, 300, 300)]), 0), Step::Pending);
    assert!(d.contacts().is_empty());
    // The resting finger moves: nothing is published.
    let f = feed(&mut d, S::new().x(310).syn().y(320).syn());
    assert!(f.iter().all(|x| x.1.is_empty()));
    assert!(d.contacts().is_empty());
    // It lifts, silently.
    let f = feed(&mut d, S::new().id(-1).syn());
    assert_eq!(f[0].1, vec![]);
    // A fresh touch in the same slot at the same Y: the kernel sends only X,
    // and the Y the stale finger last reported fills in.
    let f = feed(&mut d, S::new().id(41).x(500).syn());
    assert_eq!(f[0].1, vec![(Down, 41, 500, 320)]);
}

#[test]
fn seeding_a_running_decoder_cancels_what_it_had_published() {
    let mut d = decoder();
    feed(&mut d, S::new().slot(1).id(5).xy(1, 2).syn());
    assert_eq!(d.seed(&snapshot(1, &[(1, 5, 1, 2)]), 77), Step::Frame);
    let got: Vec<_> = d
        .events()
        .iter()
        .map(|e| (e.phase, e.contact.tracking_id, e.time_us))
        .collect();
    assert_eq!(got, vec![(Cancel, 5, 77)]);
    assert!(d.contacts().is_empty());
    // The next frame does not repeat the cancel.
    let f = feed(&mut d, S::new().slot(0).id(6).xy(3, 4).syn());
    assert_eq!(f[0].1, vec![(Down, 6, 3, 4)]);
}

#[test]
fn seed_with_a_bad_current_slot_ignores_updates_until_a_slot_arrives() {
    let mut d = decoder();
    d.seed(&snapshot(-1, &[]), 0);
    let f = feed(
        &mut d,
        S::new().id(1).xy(1, 1).syn().slot(0).id(2).xy(2, 2).syn(),
    );
    assert_eq!(f[0].1, vec![]);
    assert_eq!(f[1].1, vec![(Down, 2, 2, 2)]);
}
