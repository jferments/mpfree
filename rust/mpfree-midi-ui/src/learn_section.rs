//! The Learn section (board 8 of the MIDI mockups, approved by the owner
//! on 16 Nov 2026): what your controllers move, on one page.
//!
//! - **The list:** each learned control (its device, what on it, and the
//!   channel when it isn't 1), what it moves, where that stands now, and
//!   where it is kept: **Project** (this song) or **Rig** (every project,
//!   [`MidiSetup::rig_learn`]). A row that does nothing says why: its
//!   device isn't plugged in, its port only plays tracks, or the song has
//!   its own mapping for the same control.
//! - **The card** under the list, for the picked row: **Learn again**
//!   (the next control moved takes its place, keeping its range, takeover
//!   and where it is kept: [`Asks::learn_again`]), **Remove**, the
//!   **Range** (each end in the parameter's units, from the app:
//!   [`LearnLive`]), **Works as** (a knob or fader, an endless encoder in
//!   one of three encodings, a button), **Takeover** (Jump, Pick up,
//!   Scale; for knobs and faders) and **Kept in** (Rig, Project).
//! - **Learning:** while MIDI Learn waits for a control ([`Live::learning`]),
//!   a banner names what it will take, with **Cancel** ([`Asks::cancel_learn`]).
//! - **Transport and tempo** (the button by the heading, and the card
//!   while nothing is learned yet): PLAY and STOP, PLAY, STOP, PLAY START,
//!   REC, OVERDUB, TAP TEMPO and Tempo. A tap arms Learn on one
//!   ([`Asks::learn_target`]); the control that takes it works in every
//!   project.
//! - **Clips and scenes** (the button beside it; the owner's card of 18
//!   Nov 2026, "Both: grid taps and a Learn card"): what SHIFT + a tap on
//!   Arrange's Clips grid can't learn. A track, picked with its stepper
//!   (or the MPC's - and +), and its **Mute**, **Solo** and **Stop**, then
//!   **Stop all** and **Back to song** (`Owner::Clips`, [`clips`]). The
//!   control that takes one is kept with the song, as the grid's are.
//! - **Looper** (the button beside that; the looper redesign's
//!   footswitch): a **Loop** stepper (the picked loop, or Loop 1 to 8)
//!   with that loop's **Key** and **Mute**, then **Next loop**, **Undo**,
//!   **Redo**, **Capture** and **Clear** (`Owner::Looper`, [`looper`]). A
//!   tap arms Learn on one, as the transport's do; the control that takes
//!   it works in every project. The MPC One has no footswitch jack, so a
//!   footswitch comes in through a MIDI controller or pedal.
//! - **MIDI Follow** (the card beside the picked row's, or in its place
//!   from the heading's MIDI Follow when the section is narrower than the
//!   two): the **Controller**, **Follow** Off or On, its **Knobs** (its
//!   own, learned with **Learn knobs** one at a time, else its profile's,
//!   with **Clear**) and what it does now or why it waits. The rig's
//!   ([`MidiSetup::follow`]); Learn knobs asks the app
//!   ([`Asks::follow_learn`]), which says how many it found
//!   ([`Live::follow_learning`]).
//!
//! The wheel picks a row, - and + step Works as, and Q-Links 1 to 4 turn
//! the range's two ends, Works as and Takeover.
//!
//! [`MidiSetup::rig_learn`]: mpfree_midi::setup::MidiSetup::rig_learn
//! [`Asks::learn_again`]: crate::Asks::learn_again
//! [`Asks::cancel_learn`]: crate::Asks::cancel_learn
//! [`Asks::learn_target`]: crate::Asks::learn_target
//! [`Live::learning`]: crate::Live::learning
//! [`MidiSetup::follow`]: mpfree_midi::setup::MidiSetup::follow
//! [`Asks::follow_learn`]: crate::Asks::follow_learn
//! [`Live::follow_learning`]: crate::Live::follow_learning
//! [`LearnLive`]: crate::LearnLive

use std::cell::Cell;

use mpfree_midi::learn::{Encoding, Mode, Saved, Source, Takeover};
use mpfree_midi::setup::{Kept, PortInput, DIN_NAME, FOLLOW_KNOBS};
use mpfree_mixer_ui::{IoInput, IoKnob, IoReply, IoSection};
use mpfree_param::looper::{self, Which};
use mpfree_param::{clips, transport, Owner, ParamAddr};
use mpfree_ui::{Align, Color, FontId, PaintCx, Rect, Target, Touch};

use crate::{note_name, IoModel, Shared};

const HEAD_H: i32 = 42;
const BANNER_H: i32 = 48;
const GAP: i32 = 10;
const TABLE_HEAD_H: i32 = 28;
const ROW_H: i32 = 40;
const CARD_H: i32 = 244;
const BUTTON_W: i32 = 130;
const BUTTON_H: i32 = 34;
const STEP_W: i32 = 40;
const LABEL_W: i32 = 110;
const LINE_H: i32 = 40;
const SEG_W: i32 = 104;
/// The heading's Transport and tempo button.
const TRANSPORT_W: i32 = 190;
/// The heading's Clips and scenes button.
const CLIPS_W: i32 = 150;
/// The heading's Looper button.
const LOOPER_W: i32 = 96;
/// The Clips and scenes card's track stepper's - and +.
const CLIP_STEP_W: i32 = 34;
/// Where the heading's key may start, after its words ("WHAT YOUR
/// CONTROLLERS MOVE · 12"), and the least room it is drawn in.
const KEY_FROM: i32 = 290;
const KEY_MIN_W: i32 = 250;
/// The transport card's buttons: four a row.
const PICK_H: i32 = 44;
/// A range end's step, a tap or a Q-Link detent (SHIFT: a tenth).
const RANGE_STEP: f32 = 0.01;
/// The MIDI Follow card's width beside the picked row's card.
const FOLLOW_W: i32 = 340;
/// The narrowest picked row's card: its Range line's two steppers.
const ROW_CARD_W: i32 = 14 + LABEL_W + 190 + 36 + 190 + 14;
/// The heading's MIDI Follow button, in a section too narrow for the
/// two cards side by side.
const FOLLOW_BUTTON_W: i32 = 130;
/// The Follow card's labels.
const FOLLOW_LABEL_W: i32 = 100;
/// A line of the Follow card's words.
const SAYS_LINE_H: i32 = 16;

pub(crate) const HINT: &str = "Tap a row to edit it  ·  wheel: pick  ·  - and +: works as  ·  Q-Links: range, works as, takeover";
/// The hint line while the Clips and scenes card shows.
pub(crate) const CLIPS_HINT: &str =
    "Tap one, then press a button on your controller  ·  - and +: the track  ·  wheel: pick a row";

/// The hint line while the looper's card shows.
pub(crate) const LOOPER_HINT: &str =
    "Tap a button, then press your footswitch  ·  - and +: the loop for Key and Mute";

/// The card of buttons to learn that shows in the picked row's place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Buttons {
    /// The transport's eight ([`transport::ALL`]).
    Transport,
    /// A track's Mute, Solo and Stop, Stop all and Back to song.
    Clips,
    /// A loop's Key and Mute and the looper's own presses (the
    /// footswitch).
    Looper,
}

/// The Clips and scenes card's buttons, after the track's stepper: the
/// picked track's three, then the grid's two.
const CLIP_BUTTONS: [&str; 5] = ["Mute", "Solo", "Stop", "Stop all", "Back to song"];

/// The address the Clips and scenes card's button `i` arms, for track
/// `track` (from 0).
fn clip_target(i: usize, track: u8) -> ParamAddr {
    let id = match i {
        0 => clips::mute(track),
        1 => clips::solo(track),
        2 => clips::stop(track),
        3 => clips::STOP_ALL,
        _ => clips::BACK_TO_SONG,
    };
    ParamAddr::new(Owner::Clips, id)
}

/// The looper card's buttons, after the Loop stepper: that loop's Key
/// and Mute, then the looper's own (on the loop picked when they are
/// pressed).
const LOOPER_BUTTONS: [&str; 7] = [
    "Key",
    "Mute",
    "Next loop",
    "Undo",
    "Redo",
    "Capture",
    "Clear",
];

/// What the looper card's Loop stepper steps through: the picked loop,
/// then Loops 1 to 8.
fn loop_choices() -> impl Iterator<Item = Which> {
    std::iter::once(Which::Picked).chain((0..looper::LOOPS as u8).map(Which::Loop))
}

/// The Loop stepper's words: "Picked loop" never looks like a loop's
/// number (the owner's naming rule).
fn loop_words(w: Which) -> String {
    match w {
        Which::Picked => "Picked loop".to_string(),
        Which::Loop(n) => format!("Loop {}", u16::from(n) + 1),
    }
}

/// The address the looper card's button `i` arms, with the Loop stepper
/// on `w`.
fn looper_target(i: usize, w: Which) -> ParamAddr {
    let id = match i {
        0 => looper::key_of(w),
        1 => looper::mute_of(w),
        2 => looper::NEXT_LOOP,
        3 => looper::UNDO,
        4 => looper::REDO,
        5 => looper::CAPTURE,
        _ => looper::CLEAR,
    };
    ParamAddr::new(Owner::Looper, id)
}

/// How a learned control can move its parameter, in the order Works as
/// steps through them.
pub(crate) const MODES: [(Mode, &str); 6] = [
    (Mode::Absolute, "Knob or fader"),
    (
        Mode::Relative(Encoding::TwosComplement),
        "Endless (1 up, 127 down)",
    ),
    (
        Mode::Relative(Encoding::Offset64),
        "Endless (65 up, 63 down)",
    ),
    (Mode::Relative(Encoding::SignBit), "Endless (1 up, 65 down)"),
    (Mode::Toggle, "Button: on, then off"),
    (Mode::Momentary, "Button: while held"),
];

const TAKEOVERS: [(Takeover, &str, &str); 3] = [
    (
        Takeover::Jump,
        "Jump",
        "Jump: the value goes straight to where the knob is.",
    ),
    (
        Takeover::Pickup,
        "Pick up",
        "Pick up: the value waits until the knob passes it, so nothing jumps.",
    ),
    (
        Takeover::Scale,
        "Scale",
        "Scale: the value moves at once, in step with the knob, until they meet.",
    ),
];

const KEPT: [(Kept, &str); 2] = [(Kept::Rig, "Rig"), (Kept::Project, "Project")];

/// What a tap hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Row(usize),
    Cancel,
    Again,
    Remove,
    /// The range's low (0) or high (1) end, down or up.
    Range(usize, i32),
    Mode(i32),
    Takeover(usize),
    Kept(usize),
    /// The heading's Transport and tempo: the card shows them, or the
    /// picked row again.
    Transport,
    /// The transport card's (or the Clips and scenes card's) Back to the
    /// picked row.
    Back,
    /// One of the transport card's buttons ([`transport::ALL`]).
    Pick(usize),
    /// The heading's Clips and scenes: the card shows them, or the picked
    /// row again.
    Clips,
    /// The Clips and scenes card's track, down or up.
    ClipTrack(i32),
    /// One of its buttons ([`CLIP_BUTTONS`]).
    Clip(usize),
    /// The heading's Looper: the card shows the looper's, or the picked
    /// row again.
    Looper,
    /// The looper card's Loop, down or up.
    LoopStep(i32),
    /// One of its buttons ([`LOOPER_BUTTONS`]).
    LooperPick(usize),
    /// The heading's MIDI Follow, in a narrow section: the card shows it,
    /// or the picked row again.
    Follow,
    /// The Follow card's Controller, down or up.
    FollowPort(i32),
    /// Its Follow: Off (false) or On (true).
    FollowOn(bool),
    /// Its Learn knobs, or Done while it listens.
    FollowLearn,
    /// Its Clear: the controller's profile's knobs again.
    FollowClear,
    /// Its Send values back: Off (false) or On (true).
    FollowBack(bool),
}

struct Layout {
    banner: Option<Rect>,
    cancel: Rect,
    head: Rect,
    /// The heading's Transport and tempo button.
    transport: Rect,
    /// The heading's Clips and scenes button, left of it.
    clips: Rect,
    /// The heading's Looper button, left of Clips and scenes.
    looper: Rect,
    /// The heading's key (RIG and PROJECT), where there is room for it.
    key: Option<Rect>,
    /// The heading's MIDI Follow button, in a section too narrow for the
    /// Follow card beside the picked row's.
    follow_button: Option<Rect>,
    table: Rect,
    /// The rows' area, under the table's column heads.
    body: Rect,
    card: Rect,
    again: Rect,
    remove: Rect,
    /// Each line of the card: Range, Works as, Takeover, Kept in.
    line: [Rect; 4],
    /// The Follow card is beside the picked row's (board 8).
    wide: bool,
    /// The MIDI Follow card: beside the picked row's card, or in its place.
    follow: Rect,
}

fn layout(r: Rect, learning: bool) -> Layout {
    let mut y = r.y;
    let banner = learning.then(|| {
        let b = Rect::new(r.x, y, r.w, BANNER_H);
        y += BANNER_H + GAP;
        b
    });
    let cancel = banner.map_or(Rect::new(0, 0, 0, 0), |b| {
        Rect::new(b.right() - 10 - 100, b.y + 7, 100, b.h - 14)
    });
    let head = Rect::new(r.x + 4, y, r.w - 8, HEAD_H - 8);
    let wide = r.w >= ROW_CARD_W + GAP + FOLLOW_W;
    let follow_button = (!wide).then(|| {
        Rect::new(
            head.right() - FOLLOW_BUTTON_W,
            head.y,
            FOLLOW_BUTTON_W,
            head.h,
        )
    });
    let right = follow_button.map_or(head.right(), |b| b.x - 8);
    let transport = Rect::new(right - TRANSPORT_W, head.y, TRANSPORT_W, head.h);
    let clips = Rect::new(transport.x - 8 - CLIPS_W, head.y, CLIPS_W, head.h);
    let looper = Rect::new(clips.x - 8 - LOOPER_W, head.y, LOOPER_W, head.h);
    // The key between the heading's words and the buttons, where they
    // leave it room.
    let key_x = head.x + KEY_FROM;
    let key = (wide && looper.x - 12 - key_x >= KEY_MIN_W)
        .then(|| Rect::new(key_x, head.y, looper.x - 12 - key_x, head.h));
    y += HEAD_H;
    let bottom = Rect::new(r.x, r.bottom() - CARD_H, r.w, CARD_H);
    let (card, follow) = if wide {
        let w = r.w - GAP - FOLLOW_W;
        (
            Rect::new(r.x, bottom.y, w, CARD_H),
            Rect::new(r.x + w + GAP, bottom.y, FOLLOW_W, CARD_H),
        )
    } else {
        (bottom, bottom)
    };
    let table = Rect::new(r.x, y, r.w, (card.y - GAP - y).max(0));
    let inner = table.inset(10);
    let body = Rect::new(
        inner.x,
        inner.y + TABLE_HEAD_H,
        inner.w,
        (inner.h - TABLE_HEAD_H).max(0),
    );
    let ci = card.inset(14);
    let remove = Rect::new(ci.right() - BUTTON_W, ci.y, BUTTON_W, BUTTON_H);
    let again = Rect::new(remove.x - 8 - BUTTON_W, ci.y, BUTTON_W, BUTTON_H);
    let line = std::array::from_fn(|i| {
        Rect::new(
            ci.x,
            ci.y + BUTTON_H + 10 + i as i32 * LINE_H,
            ci.w,
            LINE_H - 6,
        )
    });
    Layout {
        banner,
        cancel,
        head,
        transport,
        clips,
        looper,
        key,
        follow_button,
        table,
        body,
        card,
        again,
        remove,
        line,
        wide,
        follow,
    }
}

/// The MIDI Follow card's parts.
struct FollowRects {
    title: Rect,
    /// Learn knobs, or Done.
    learn: Rect,
    /// Each line's label: Controller, Follow, Knobs.
    labels: [Rect; 3],
    /// The Controller's − and +.
    port: [Rect; 2],
    /// Follow's Off and On.
    on: [Rect; 2],
    /// The knobs' CCs, then Clear.
    knobs: Rect,
    clear: Rect,
    /// What Follow does now, or why it waits.
    says: Rect,
    /// Send values back: its label, Off and On. On the Follow line where
    /// the card is wide enough (the MPC's screen swaps it in at the
    /// card's full width), else a line at the foot.
    back_label: Rect,
    back: [Rect; 2],
    wide: bool,
}

/// Send values back's label, where it shares the Follow line.
const BACK_LABEL_W: i32 = 150;

fn follow_rects(card: Rect) -> FollowRects {
    let ci = card.inset(14);
    let learn = Rect::new(ci.right() - BUTTON_W, ci.y, BUTTON_W, BUTTON_H);
    let title = Rect::new(ci.x, ci.y, learn.x - ci.x - 8, BUTTON_H);
    let line = |i: i32| Rect::new(ci.x, ci.y + BUTTON_H + 10 + i * LINE_H, ci.w, LINE_H - 6);
    let lines = [line(0), line(1), line(2)];
    let x = ci.x + FOLLOW_LABEL_W;
    let [l0, l1, l2] = lines;
    let seg = 90;
    let clear = Rect::new(ci.right() - 80, l2.y, 80, l2.h);
    let says_y = l2.bottom() + 8;
    let wide = ci.w >= FOLLOW_LABEL_W + 2 * seg + 6 + 24 + BACK_LABEL_W + 2 * seg + 6;
    let (back_label, bx, by, bh, says_bottom) = if wide {
        let lx = x + 2 * seg + 6 + 24;
        (
            Rect::new(lx, l1.y, BACK_LABEL_W, l1.h),
            lx + BACK_LABEL_W,
            l1.y,
            l1.h,
            ci.bottom(),
        )
    } else {
        // A little lower than the lines above, so the line saying what
        // Follow does keeps one line of its own.
        let h = l2.h - 4;
        let y = ci.bottom() - h;
        (Rect::new(ci.x, y, FOLLOW_LABEL_W, h), x, y, h, y - 4)
    };
    FollowRects {
        title,
        learn,
        labels: lines.map(|l| Rect::new(l.x, l.y, FOLLOW_LABEL_W, l.h)),
        port: [
            Rect::new(x, l0.y, STEP_W, l0.h),
            Rect::new(ci.right() - STEP_W, l0.y, STEP_W, l0.h),
        ],
        on: [
            Rect::new(x, l1.y, seg, l1.h),
            Rect::new(x + seg + 6, l1.y, seg, l1.h),
        ],
        knobs: Rect::new(x, l2.y, clear.x - 8 - x, l2.h),
        clear,
        says: Rect::new(ci.x, says_y, ci.w, (says_bottom - says_y).max(0)),
        back_label,
        back: [
            Rect::new(bx, by, seg, bh),
            Rect::new(bx + seg + 6, by, seg, bh),
        ],
        wide,
    }
}

/// What a tap on the Follow card hit.
fn follow_hit(m: &IoModel, card: Rect, x: i32, y: i32) -> Option<Hit> {
    let r = follow_rects(card);
    if r.learn.contains(x, y) {
        return Some(Hit::FollowLearn);
    }
    if let Some(i) = r.port.iter().position(|p| p.contains(x, y)) {
        return Some(Hit::FollowPort(if i == 0 { -1 } else { 1 }));
    }
    if let Some(i) = r.on.iter().position(|p| p.contains(x, y)) {
        return Some(Hit::FollowOn(i == 1));
    }
    if follow_clearable(m) && r.clear.contains(x, y) {
        return Some(Hit::FollowClear);
    }
    if let Some(i) = r.back.iter().position(|p| p.contains(x, y)) {
        return Some(Hit::FollowBack(i == 1));
    }
    None
}

fn follow_targets(m: &IoModel, card: Rect, out: &mut Vec<Target>) {
    let r = follow_rects(card);
    let learn = if follow_listening(m) {
        "Done"
    } else {
        "Learn knobs"
    };
    out.push(Target::tap("follow-learn", 0, learn, r.learn));
    out.push(Target::tap("follow-port", 0, "Controller −", r.port[0]));
    out.push(Target::tap("follow-port", 1, "Controller +", r.port[1]));
    out.push(Target::tap("follow-on", 0, "Follow off", r.on[0]));
    out.push(Target::tap("follow-on", 1, "Follow on", r.on[1]));
    if follow_clearable(m) {
        out.push(Target::tap("follow-clear", 0, "Clear the knobs", r.clear));
    }
    out.push(Target::tap(
        "follow-back",
        0,
        "Send values back off",
        r.back[0],
    ));
    out.push(Target::tap(
        "follow-back",
        1,
        "Send values back on",
        r.back[1],
    ));
}

/// "MIDI in", or a USB device's name (the one given to it, if any).
fn shown(m: &IoModel, name: &str) -> String {
    if name == DIN_NAME {
        "MIDI in".to_string()
    } else {
        m.setup.shown(name).to_string()
    }
}

/// The controllers MIDI Follow can follow: each port plugged in with an
/// input, then the one picked if it isn't plugged in now.
fn follow_ports(m: &IoModel) -> Vec<String> {
    let mut v: Vec<String> = m
        .live
        .ports
        .iter()
        .filter(|p| p.has_in)
        .map(|p| p.name.clone())
        .collect();
    let f = &m.setup.follow;
    if !f.port.is_empty() && !v.contains(&f.port) {
        v.push(f.port.clone());
    }
    v
}

/// Learn knobs listens, or the app is asked to.
fn follow_listening(m: &IoModel) -> bool {
    m.live.follow_learning.is_some() || m.asks.follow_learn == Some(true)
}

/// The controller's own knobs can be cleared (its profile's take over).
fn follow_clearable(m: &IoModel) -> bool {
    !m.setup.follow.knobs.is_empty() && !follow_listening(m)
}

/// The knobs' line: "CC 74 71 76 77", "CC 74 71 (profile)", "none yet".
fn knobs_text(m: &IoModel) -> String {
    let f = &m.setup.follow;
    if m.live.follow_learning == Some(0) {
        return "listening…".to_string();
    }
    let list = |k: &[u8]| {
        k.iter()
            .take(FOLLOW_KNOBS)
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(" ")
    };
    if !f.knobs.is_empty() {
        return format!("CC {}", list(&f.knobs));
    }
    match m.profile(&f.port).filter(|p| !p.knobs.is_empty()) {
        Some(p) => format!("CC {} (profile)", list(&p.knobs)),
        None => "none yet".to_string(),
    }
}

/// What MIDI Follow does now, or why it waits.
fn follow_says(m: &IoModel) -> String {
    let f = &m.setup.follow;
    if let Some(n) = m.live.follow_learning {
        return format!(
            "Turn the knobs you want, one at a time, in Q-Link order: {n} of {FOLLOW_KNOBS} \
             found. Done keeps them."
        );
    }
    if f.port.is_empty() {
        return "Pick the controller to follow, then turn Follow on. Kept for every project."
            .to_string();
    }
    let name = shown(m, &f.port);
    if m.live.port(&f.port).is_none() {
        return format!("{name} isn't plugged in: Follow waits for it.");
    }
    if !f.on {
        return format!(
            "On, {name}'s knobs turn whatever the Q-Links turn on screen, in every project."
        );
    }
    if f.knobs_of(m.profile(&f.port)).is_empty() {
        return "No knobs yet: tap Learn knobs, then turn up to four of its knobs.".to_string();
    }
    let back = match m.live.port(&f.port) {
        _ if !f.send_back => "",
        Some(p) if !p.has_out => " It has no output, so no values go back.",
        _ if !m.live.built.notes_out => " Values go back once MIDI out is on (Prefs).",
        _ => " Its lights or faders follow the Q-Links' values.",
    };
    format!(
        "Its keys play the armed track and its knobs turn whatever the Q-Links turn on screen. \
         A learned control wins.{back}"
    )
}

/// `text` in `r`, a line of [`SAYS_LINE_H`] at a time, broken between
/// words; what doesn't fit under `r` is left out.
fn para(cx: &mut PaintCx, px: f32, r: Rect, ink: Color, text: &str) {
    let mut y = r.y;
    let mut line = String::new();
    let put = |cx: &mut PaintCx, line: &str, y: &mut i32| {
        if *y + SAYS_LINE_H <= r.bottom() {
            let lr = Rect::new(r.x, *y, r.w, SAYS_LINE_H);
            cx.text(FontId::Sans, px, lr, Align::Left, ink, line);
        }
        *y += SAYS_LINE_H;
    };
    for w in text.split(' ') {
        let longer = if line.is_empty() {
            w.to_string()
        } else {
            format!("{line} {w}")
        };
        if !line.is_empty() && cx.measure(FontId::Sans, px, &longer) > r.w {
            put(cx, &line, &mut y);
            line = w.to_string();
        } else {
            line = longer;
        }
    }
    if !line.is_empty() {
        put(cx, &line, &mut y);
    }
}

/// The transport card's buttons, in [`transport::ALL`]'s order: two rows
/// of four under the card's title and its line of words.
fn pick_rects(l: &Layout) -> [Rect; 8] {
    let ci = l.card.inset(14);
    let w = (ci.w - 3 * 8) / 4;
    let y = ci.y + BUTTON_H + 34;
    std::array::from_fn(|i| {
        let (row, col) = ((i / 4) as i32, (i % 4) as i32);
        Rect::new(ci.x + col * (w + 8), y + row * (PICK_H + 8), w, PICK_H)
    })
}

/// The address a transport card button arms.
fn pick_target(i: usize) -> ParamAddr {
    ParamAddr::new(Owner::Transport, transport::ALL[i].0)
}

/// The Clips and scenes card's parts, on the transport card's grid: the
/// track's stepper (- and +) in the first place, its Mute, Solo and Stop
/// beside it, then Stop all and Back to song on the second row.
fn clip_rects(l: &Layout) -> ([Rect; 2], [Rect; 5]) {
    let p = pick_rects(l);
    let s = p[0];
    let steps = [
        Rect::new(s.x, s.y, CLIP_STEP_W, s.h),
        Rect::new(s.right() - CLIP_STEP_W, s.y, CLIP_STEP_W, s.h),
    ];
    (steps, [p[1], p[2], p[3], p[4], p[5]])
}

/// The looper card's parts, on the same grid: the Loop stepper (- and
/// +) in the first place, that loop's Key and Mute and Next loop beside
/// it, then Undo, Redo, Capture and Clear on the second row.
fn looper_rects(l: &Layout) -> ([Rect; 2], [Rect; 7]) {
    let (steps, _) = clip_rects(l);
    let p = pick_rects(l);
    (steps, [p[1], p[2], p[3], p[4], p[5], p[6], p[7]])
}

/// The table's columns: Control, Moves, Now, Kept in (x and width).
fn columns(body: Rect) -> [(i32, i32); 4] {
    let kept = 96;
    let now = 140;
    let rest = body.w - kept - now;
    let control = rest * 45 / 100;
    let moves = rest - control;
    [
        (body.x, control),
        (body.x + control, moves),
        (body.x + control + moves, now),
        (body.x + control + moves + now, kept),
    ]
}

/// The steppers and segments on a card line.
fn range_rects(line: Rect) -> [Rect; 4] {
    let x = line.x + LABEL_W;
    let w = 190;
    let lo = Rect::new(x, line.y, w, line.h);
    let hi = Rect::new(x + w + 36, line.y, w, line.h);
    [
        Rect::new(lo.x, lo.y, STEP_W, lo.h),
        Rect::new(lo.right() - STEP_W, lo.y, STEP_W, lo.h),
        Rect::new(hi.x, hi.y, STEP_W, hi.h),
        Rect::new(hi.right() - STEP_W, hi.y, STEP_W, hi.h),
    ]
}

fn mode_rects(line: Rect) -> [Rect; 2] {
    let x = line.x + LABEL_W;
    let w = 300;
    [
        Rect::new(x, line.y, STEP_W, line.h),
        Rect::new(x + w - STEP_W, line.y, STEP_W, line.h),
    ]
}

fn segments(line: Rect, n: usize) -> Vec<Rect> {
    (0..n)
        .map(|i| {
            Rect::new(
                line.x + LABEL_W + i as i32 * (SEG_W + 6),
                line.y,
                SEG_W,
                line.h,
            )
        })
        .collect()
}

/// "MIDI in", "Keystep 37" (or the name given to it), "Any port".
fn port_name(m: &IoModel, l: &Saved) -> String {
    match l.port.as_deref() {
        None => "Any port".to_string(),
        Some(DIN_NAME) => "MIDI in".to_string(),
        Some(p) => m.setup.shown(p).to_string(),
    }
}

/// "CC 74", "CC 1 Mod wheel", "note C1 ch 16", "pitch bend ch 2": a CC
/// named by its port's profile, else as most gear names it.
fn control_name(m: &IoModel, l: &Saved) -> String {
    let profile = l.port.as_deref().and_then(|p| m.profile(p));
    control_words(l, true, profile)
}

/// "CC 74", "note C1 ch 16": the control, its CC named (by `profile`, else
/// as most gear names it) when `named`.
fn control_words(
    l: &Saved,
    named: bool,
    profile: Option<&mpfree_midi::profile::Profile>,
) -> String {
    let what = match l.source {
        Source::Cc(n) => match mpfree_midi::profile::cc_name(profile, n).filter(|_| named) {
            Some(name) => format!("CC {n} {name}"),
            None => format!("CC {n}"),
        },
        Source::Note(n) => format!("note {}", note_name(n)),
        Source::PitchBend => "pitch bend".to_string(),
        Source::ChanPressure => "pressure".to_string(),
    };
    if l.channel == 0 {
        what
    } else {
        format!("{what} ch {}", l.channel + 1)
    }
}

/// "Cutoff (Filter, Bass)" as ("Cutoff", "Filter, Bass").
fn split_name(name: &str) -> (&str, &str) {
    match name.split_once(" (") {
        Some((a, b)) => (a, b.strip_suffix(')').unwrap_or(b)),
        None => (name, ""),
    }
}

/// Why a learned control does nothing now, if it doesn't.
fn idle_why(m: &IoModel, kept: Kept, l: &Saved) -> Option<&'static str> {
    if kept == Kept::Rig && m.setup.learn.iter().any(|p| p.same_control(l)) {
        return Some("the song's own mapping wins");
    }
    match l.port.as_deref() {
        Some(p) if m.live.port(p).is_none() => Some("unplugged, waits"),
        Some(p) if m.setup.port(p).input == PortInput::PlaysTracks => {
            Some("its port only plays tracks")
        }
        _ => None,
    }
}

fn name_of(m: &IoModel, k: usize, l: &Saved) -> String {
    m.live
        .learn_names
        .get(k)
        .cloned()
        .unwrap_or_else(|| l.target.to_string())
}

pub(crate) struct Learn {
    model: Shared,
    /// The picked row, in `MidiSetup::all_learned`'s order.
    pick: usize,
    /// The first row shown.
    top: Cell<usize>,
    down: Option<(u64, Option<Hit>)>,
    note: Option<String>,
    /// The row whose Learn again was tapped, lit while Learn waits.
    again: Option<usize>,
    /// The card shows buttons to learn, not the picked row (the
    /// transport's while nothing is learned too: [`Learn::buttons_shown`]).
    buttons: Option<Buttons>,
    /// What the button tapped arms, lit while Learn waits for it.
    armed: Option<ParamAddr>,
    /// The Clips and scenes card's track, from 0.
    clip_track: usize,
    /// The looper card's Loop stepper.
    loop_which: Which,
    /// The card shows MIDI Follow, in a section too narrow for it beside
    /// the picked row's.
    follow: bool,
    /// What was drawn last: the model's generation, whether Learn waited
    /// and the values now (to draw again when they change).
    drawn: (u64, bool, u64),
}

impl Learn {
    pub(crate) fn new(model: Shared) -> Learn {
        Learn {
            model,
            pick: 0,
            top: Cell::new(0),
            down: None,
            note: None,
            again: None,
            buttons: None,
            armed: None,
            clip_track: 0,
            loop_which: Which::Picked,
            follow: false,
            drawn: (u64::MAX, false, 0),
        }
    }

    fn rows_shown(body: Rect) -> usize {
        (body.h / ROW_H).max(1) as usize
    }

    /// The card of buttons that shows in the picked row's place, with `n`
    /// controls learned: the one asked for, else the transport's while
    /// there is no row to show.
    fn buttons_shown(&self, n: usize) -> Option<Buttons> {
        self.buttons.or((n == 0).then_some(Buttons::Transport))
    }

    /// The tracks the Clips and scenes card steps through (the I/O view's,
    /// at least one).
    fn clip_tracks(m: &IoModel) -> usize {
        m.live.tracks.len().clamp(1, usize::from(clips::TRACKS))
    }

    /// The Clips and scenes card's track, named as the project names it.
    fn clip_track_name(&self, m: &IoModel) -> String {
        let t = self.clip_track.min(Self::clip_tracks(m) - 1);
        m.live
            .tracks
            .get(t)
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| format!("Track {}", t + 1))
    }

    /// The Clips and scenes card's track, a step down or up.
    fn clip_step(&mut self, d: i32) -> IoReply {
        let n = {
            let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
            Self::clip_tracks(&m)
        };
        let at = self.clip_track.min(n - 1) as i32;
        let to = (at + d.signum()).clamp(0, n as i32 - 1);
        if to == at {
            self.note = Some(if d < 0 {
                "The first track is picked".to_string()
            } else {
                "The last track is picked".to_string()
            });
            return IoReply::Handled;
        }
        self.clip_track = to as usize;
        IoReply::Redraw
    }

    /// The heading's Transport and tempo or Clips and scenes: its card
    /// shows, or the picked row's again when it showed already.
    fn show_buttons(&mut self, b: Buttons) -> IoReply {
        let n = {
            let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
            m.setup.all_learned().count()
        };
        self.buttons = (self.buttons_shown(n) != Some(b)).then_some(b);
        self.follow = false;
        IoReply::Redraw
    }

    /// Learn waits for a control to take `target`, named `what` on the
    /// hint line: a button on a card of buttons.
    fn arm(&mut self, target: ParamAddr, what: &str) -> IoReply {
        self.arm_as(target, what, "press a button or move a knob")
    }

    /// [`Learn::arm`], saying `how` the control takes it.
    fn arm_as(&mut self, target: ParamAddr, what: &str, how: &str) -> IoReply {
        let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        m.asks.learn_target = Some(target);
        m.asks.learn_again = None;
        m.asks.cancel_learn = false;
        self.again = None;
        self.armed = Some(target);
        self.note = Some(format!("{what}: {how} on your controller"));
        IoReply::Redraw
    }

    /// The looper card's Loop, a step down or up.
    fn loop_step(&mut self, d: i32) -> IoReply {
        let all: Vec<Which> = loop_choices().collect();
        let at = all.iter().position(|w| *w == self.loop_which).unwrap_or(0);
        let to = (at as i32 + d.signum()).clamp(0, all.len() as i32 - 1) as usize;
        if to == at {
            self.note = Some(if d < 0 {
                "The picked loop is the first".to_string()
            } else {
                "Loop 8 is the last".to_string()
            });
            return IoReply::Handled;
        }
        self.loop_which = all[to];
        IoReply::Redraw
    }

    /// The looper card's button `i`: Learn waits for a control to take
    /// it, for the Loop stepper's loop.
    fn arm_looper(&mut self, i: usize) -> IoReply {
        let target = looper_target(i, self.loop_which);
        let mut name = looper::name(target.param).unwrap_or_default();
        if let Some(c) = name.get(..1) {
            name = c.to_uppercase() + &name[1..];
        }
        let how = if looper::is_value(target.param) {
            "press a key or a switch, or move a fader,"
        } else {
            "press a footswitch, a button or a key"
        };
        self.arm_as(target, &name, how)
    }

    fn top_for(&self, rows: usize, shown: usize) -> usize {
        let mut top = self.top.get();
        if self.pick < top {
            top = self.pick;
        } else if self.pick >= top + shown {
            top = self.pick + 1 - shown;
        }
        top = top.min(rows.saturating_sub(shown));
        self.top.set(top);
        top
    }

    fn row_rect(l: &Layout, top: usize, k: usize) -> Option<Rect> {
        let shown = Self::rows_shown(l.body);
        (k >= top && k < top + shown).then(|| {
            Rect::new(
                l.body.x,
                l.body.y + (k - top) as i32 * ROW_H,
                l.body.w,
                ROW_H - 4,
            )
        })
    }

    fn hit(&self, x: i32, y: i32, r: Rect) -> Option<Hit> {
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let l = layout(r, m.live.learning.is_some());
        if l.banner.is_some() && l.cancel.contains(x, y) {
            return Some(Hit::Cancel);
        }
        let n = m.setup.all_learned().count();
        if l.transport.contains(x, y) {
            return Some(Hit::Transport);
        }
        if l.clips.contains(x, y) {
            return Some(Hit::Clips);
        }
        if l.looper.contains(x, y) {
            return Some(Hit::Looper);
        }
        if l.follow_button.is_some_and(|b| b.contains(x, y)) {
            return Some(Hit::Follow);
        }
        let top = self.top_for(n, Self::rows_shown(l.body));
        for k in 0..n {
            if Self::row_rect(&l, top, k).is_some_and(|rr| rr.contains(x, y)) {
                return Some(Hit::Row(k));
            }
        }
        if self.follow_shown(&l) && l.follow.contains(x, y) {
            return follow_hit(&m, l.follow, x, y);
        }
        if !l.card.contains(x, y) {
            return None;
        }
        if let Some(b) = self.buttons_shown(n) {
            if n > 0 && l.remove.contains(x, y) {
                return Some(Hit::Back);
            }
            return match b {
                Buttons::Transport => pick_rects(&l)
                    .iter()
                    .position(|p| p.contains(x, y))
                    .map(Hit::Pick),
                Buttons::Clips => {
                    let (steps, buttons) = clip_rects(&l);
                    match steps.iter().position(|p| p.contains(x, y)) {
                        Some(i) => Some(Hit::ClipTrack(if i == 0 { -1 } else { 1 })),
                        None => buttons.iter().position(|p| p.contains(x, y)).map(Hit::Clip),
                    }
                }
                Buttons::Looper => {
                    let (steps, buttons) = looper_rects(&l);
                    match steps.iter().position(|p| p.contains(x, y)) {
                        Some(i) => Some(Hit::LoopStep(if i == 0 { -1 } else { 1 })),
                        None => buttons
                            .iter()
                            .position(|p| p.contains(x, y))
                            .map(Hit::LooperPick),
                    }
                }
            };
        }
        if l.again.contains(x, y) {
            return Some(Hit::Again);
        }
        if l.remove.contains(x, y) {
            return Some(Hit::Remove);
        }
        let rr = range_rects(l.line[0]);
        for (i, r) in rr.iter().enumerate() {
            if r.contains(x, y) {
                return Some(Hit::Range(i / 2, if i % 2 == 0 { -1 } else { 1 }));
            }
        }
        let [down, up] = mode_rects(l.line[1]);
        if down.contains(x, y) {
            return Some(Hit::Mode(-1));
        }
        if up.contains(x, y) {
            return Some(Hit::Mode(1));
        }
        for (i, s) in segments(l.line[2], TAKEOVERS.len()).iter().enumerate() {
            if s.contains(x, y) {
                return Some(Hit::Takeover(i));
            }
        }
        for (i, s) in segments(l.line[3], KEPT.len()).iter().enumerate() {
            if s.contains(x, y) {
                return Some(Hit::Kept(i));
            }
        }
        None
    }

    /// The Follow card shows: beside the picked row's, or in its place.
    fn follow_shown(&self, l: &Layout) -> bool {
        l.wide || self.follow
    }

    /// The picked row's card (or the transport's) shows.
    fn card_shown(&self, l: &Layout) -> bool {
        l.wide || !self.follow
    }

    /// Follow's Controller, a step down or up the ports with an input.
    /// Another controller's knobs aren't this one's: its own are cleared.
    fn follow_port(&mut self, d: i32) -> IoReply {
        let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let ports = follow_ports(&m);
        let to = match ports.iter().position(|p| *p == m.setup.follow.port) {
            None if ports.is_empty() => {
                self.note = Some("No controller is plugged in".to_string());
                return IoReply::Handled;
            }
            None => 0,
            Some(i) => {
                let j = i as i32 + d;
                if j < 0 || j >= ports.len() as i32 {
                    self.note = Some(if d < 0 {
                        "The first controller is picked".to_string()
                    } else {
                        "The last controller is picked".to_string()
                    });
                    return IoReply::Handled;
                }
                j as usize
            }
        };
        if follow_listening(&m) {
            m.asks.follow_learn = Some(false);
        }
        let f = &mut m.setup.follow;
        f.port = ports[to].clone();
        f.knobs.clear();
        m.gen += 1;
        IoReply::Redraw
    }

    /// Follow Off or On; On picks the first controller if none is.
    fn follow_on(&mut self, on: bool) -> IoReply {
        let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        if m.setup.follow.on == on {
            return IoReply::Handled;
        }
        if on && m.setup.follow.port.is_empty() {
            match follow_ports(&m).into_iter().next() {
                Some(p) => m.setup.follow.port = p,
                None => {
                    self.note = Some("Plug in a controller first".to_string());
                    return IoReply::Handled;
                }
            }
        }
        if !on && follow_listening(&m) {
            m.asks.follow_learn = Some(false);
        }
        m.setup.follow.on = on;
        m.gen += 1;
        IoReply::Redraw
    }

    /// Learn knobs (turning Follow on), or Done while it listens.
    fn follow_learn(&mut self) -> IoReply {
        let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        if follow_listening(&m) {
            m.asks.follow_learn = Some(false);
            return IoReply::Redraw;
        }
        let port = m.setup.follow.port.clone();
        if port.is_empty() {
            self.note = Some("Pick the controller first".to_string());
            return IoReply::Handled;
        }
        let name = shown(&m, &port);
        if m.live.port(&port).is_none_or(|p| !p.has_in) {
            self.note = Some(format!("{name} isn't plugged in"));
            return IoReply::Handled;
        }
        if !m.setup.follow.on {
            m.setup.follow.on = true;
            m.gen += 1;
        }
        m.asks.follow_learn = Some(true);
        self.note = Some(format!("Turn the knobs you want on {name}, one at a time"));
        IoReply::Redraw
    }

    fn follow_clear(&mut self) -> IoReply {
        let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        if !follow_clearable(&m) {
            return IoReply::Handled;
        }
        m.setup.follow.knobs.clear();
        m.gen += 1;
        IoReply::Redraw
    }

    /// Changes the picked learned control; the app applies it at its next
    /// look ([`IoModel::gen`]).
    fn edit(&mut self, f: impl FnOnce(&mut Saved) -> Result<bool, String>) -> IoReply {
        let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let Some(l) = m.setup.learned_at_mut(self.pick) else {
            return IoReply::Handled;
        };
        match f(l) {
            Ok(true) => {
                m.gen += 1;
                IoReply::Redraw
            }
            Ok(false) => IoReply::Handled,
            Err(e) => {
                self.note = Some(e);
                IoReply::Handled
            }
        }
    }

    fn range(&mut self, end: usize, d: i32, step: f32) -> IoReply {
        self.edit(|l| {
            let v = if end == 0 { &mut l.lo } else { &mut l.hi };
            let to = ((*v + d as f32 * step) / step).round() * step;
            let to = to.clamp(0.0, 1.0);
            if (to - *v).abs() < f32::EPSILON {
                return Err(if d < 0 {
                    "The range's end is at the bottom".to_string()
                } else {
                    "The range's end is at the top".to_string()
                });
            }
            *v = to;
            Ok(true)
        })
    }

    fn mode(&mut self, d: i32) -> IoReply {
        self.edit(|l| {
            let at = MODES.iter().position(|x| x.0 == l.mode).unwrap_or(0) as i32;
            let to = (at + d).clamp(0, MODES.len() as i32 - 1);
            if to == at {
                return Err("No more ways that way".to_string());
            }
            l.mode = MODES[to as usize].0;
            Ok(true)
        })
    }

    fn takeover(&mut self, i: usize) -> IoReply {
        self.edit(|l| {
            if l.mode != Mode::Absolute {
                return Err("Takeover is for knobs and faders".to_string());
            }
            let to = TAKEOVERS[i].0;
            let changed = l.takeover != to;
            l.takeover = to;
            Ok(changed)
        })
    }

    fn kept(&mut self, i: usize) -> IoReply {
        let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let to = KEPT[i].0;
        if m.setup.learned_at(self.pick).is_none_or(|(k, _)| k == to) {
            return IoReply::Handled;
        }
        match m.setup.keep_at(self.pick, to) {
            Some(at) => {
                self.pick = at;
                m.gen += 1;
                IoReply::Redraw
            }
            None => {
                self.note = Some("No room: 256 learned controls are kept there".to_string());
                IoReply::Handled
            }
        }
    }

    fn tap(&mut self, h: Hit) -> IoReply {
        match h {
            Hit::Row(k) => {
                self.pick = k;
                self.buttons = None;
                self.follow = false;
                IoReply::Redraw
            }
            Hit::Cancel => {
                let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
                m.asks.cancel_learn = true;
                m.asks.learn_again = None;
                m.asks.learn_target = None;
                self.again = None;
                self.armed = None;
                IoReply::Redraw
            }
            Hit::Transport => self.show_buttons(Buttons::Transport),
            Hit::Clips => self.show_buttons(Buttons::Clips),
            Hit::ClipTrack(d) => self.clip_step(d),
            Hit::Clip(i) => {
                let name = {
                    let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
                    self.clip_track_name(&m)
                };
                let track = self.clip_track as u8;
                let what = match i {
                    0..=2 => format!("{} {name}", CLIP_BUTTONS[i]),
                    _ => CLIP_BUTTONS[i].to_string(),
                };
                self.arm(clip_target(i, track), &what)
            }
            Hit::Looper => self.show_buttons(Buttons::Looper),
            Hit::LoopStep(d) => self.loop_step(d),
            Hit::LooperPick(i) => self.arm_looper(i),
            Hit::Follow => {
                self.follow = !self.follow;
                IoReply::Redraw
            }
            Hit::FollowPort(d) => self.follow_port(d),
            Hit::FollowOn(on) => self.follow_on(on),
            Hit::FollowLearn => self.follow_learn(),
            Hit::FollowClear => self.follow_clear(),
            Hit::FollowBack(on) => {
                let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
                if m.setup.follow.send_back == on {
                    return IoReply::Handled;
                }
                m.setup.follow.send_back = on;
                m.gen += 1;
                IoReply::Redraw
            }
            Hit::Back => {
                self.buttons = None;
                IoReply::Redraw
            }
            Hit::Pick(i) => self.arm(pick_target(i), transport::ALL[i].1),
            Hit::Again => {
                let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
                if m.setup.learned_at(self.pick).is_some() {
                    m.asks.learn_again = Some(self.pick);
                    m.asks.learn_target = None;
                    m.asks.cancel_learn = false;
                    self.again = Some(self.pick);
                    self.armed = None;
                    self.note =
                        Some("Move a knob, a key or a pedal on your controller".to_string());
                }
                IoReply::Redraw
            }
            Hit::Remove => {
                let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
                if m.setup.forget_at(self.pick).is_none() {
                    return IoReply::Handled;
                }
                // Keep the app's words in step until it names them again.
                if self.pick < m.live.learn_names.len() {
                    m.live.learn_names.remove(self.pick);
                }
                if self.pick < m.live.learn_live.len() {
                    m.live.learn_live.remove(self.pick);
                }
                let n = m.setup.all_learned().count();
                self.pick = self.pick.min(n.saturating_sub(1));
                m.gen += 1;
                IoReply::Redraw
            }
            Hit::Range(end, d) => self.range(end, d, RANGE_STEP),
            Hit::Mode(d) => self.mode(d),
            Hit::Takeover(i) => self.takeover(i),
            Hit::Kept(i) => self.kept(i),
        }
    }

    /// The values the list shows, as a number, to draw again when one
    /// moves.
    fn now_hash(m: &IoModel) -> u64 {
        let found = m.live.follow_learning.map_or(u64::MAX, |n| n as u64);
        m.live.learn_live.iter().fold(found, |h, v| {
            let x = v.now.map_or(u64::MAX, |x| (x * 1000.0) as u64);
            h.wrapping_mul(31).wrapping_add(x)
        })
    }

    /// The MIDI Follow card (board 8's right card).
    fn paint_follow(&self, cx: &mut PaintCx, m: &IoModel, card: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_round_rect(card, 12, t.surface);
        let r = follow_rects(card);
        cx.text(
            FontId::Bold,
            t.size_small,
            r.title,
            Align::Left,
            t.text_dim,
            "MIDI FOLLOW",
        );
        let listening = follow_listening(m);
        button(
            cx,
            r.learn,
            if listening { "Done" } else { "Learn knobs" },
            listening,
        );
        for (lr, s) in r.labels.iter().zip(["Controller", "Follow", "Knobs"]) {
            cx.text(FontId::Sans, t.size_body, *lr, Align::Left, t.text, s);
        }
        let f = &m.setup.follow;
        let name = if f.port.is_empty() {
            "none picked".to_string()
        } else {
            shown(m, &f.port)
        };
        stepper(cx, r.port[0], r.port[1], &name);
        segment(cx, r.on[0], "Off", !f.on, true);
        segment(cx, r.on[1], "On", f.on, true);
        cx.text(
            FontId::Sans,
            t.size_small,
            r.knobs,
            Align::Left,
            if f.knobs.is_empty() {
                t.text_dim
            } else {
                t.text
            },
            &knobs_text(m),
        );
        if follow_clearable(m) {
            button(cx, r.clear, "Clear", false);
        }
        cx.text(
            FontId::Sans,
            t.size_body,
            r.back_label,
            Align::Left,
            t.text,
            if r.wide {
                "Send values back"
            } else {
                "Send back"
            },
        );
        segment(cx, r.back[0], "Off", !f.send_back, true);
        segment(cx, r.back[1], "On", f.send_back, true);
        para(cx, t.size_small * 0.9, r.says, t.text_dim, &follow_says(m));
    }

    /// The transport card: a button for each of the transport's controls,
    /// the one Learn waits for lit.
    fn paint_transport(&self, cx: &mut PaintCx, m: &IoModel, l: &Layout, back: bool) {
        let t = cx.theme.clone();
        cx.canvas.fill_round_rect(l.card, 12, t.surface);
        let ci = l.card.inset(14);
        cx.text(
            FontId::Bold,
            t.size_small,
            Rect::new(ci.x, ci.y, l.again.x - ci.x - 8, BUTTON_H),
            Align::Left,
            t.text_dim,
            "TRANSPORT AND TEMPO",
        );
        if back {
            button(cx, l.remove, "Back", false);
        }
        para(
            cx,
            t.size_small * 0.9,
            Rect::new(ci.x, ci.y + BUTTON_H, ci.w, 2 * SAYS_LINE_H),
            t.text_dim,
            "Tap one, then press a button or move a knob on your controller. It works in every project.",
        );
        for (i, r) in pick_rects(l).into_iter().enumerate() {
            button(
                cx,
                r,
                transport::ALL[i].1,
                self.waits_for(m, pick_target(i)),
            );
        }
        let last = pick_rects(l)[7];
        para(
            cx,
            t.size_small * 0.9,
            Rect::new(
                ci.x,
                last.bottom() + 6,
                ci.w,
                ci.bottom() - last.bottom() - 6,
            ),
            t.text_faint,
            "A key on PLAY and STOP plays, then stops. Tempo wants a knob or a fader. \
             Mutes: hold SHIFT and touch a strip's M.",
        );
    }

    /// Whether Learn waits for a control to take `target`, armed from one
    /// of this section's buttons: that button is lit.
    fn waits_for(&self, m: &IoModel, target: ParamAddr) -> bool {
        let waiting = m.asks.learn_target.is_some() || m.live.learning.is_some();
        waiting && self.armed == Some(target)
    }

    /// The Clips and scenes card: the picked track's Mute, Solo and Stop,
    /// then Stop all and Back to song, the one Learn waits for lit.
    fn paint_clips(&self, cx: &mut PaintCx, m: &IoModel, l: &Layout, back: bool) {
        let t = cx.theme.clone();
        cx.canvas.fill_round_rect(l.card, 12, t.surface);
        let ci = l.card.inset(14);
        cx.text(
            FontId::Bold,
            t.size_small,
            Rect::new(ci.x, ci.y, l.again.x - ci.x - 8, BUTTON_H),
            Align::Left,
            t.text_dim,
            "CLIPS AND SCENES",
        );
        if back {
            button(cx, l.remove, "Back", false);
        }
        para(
            cx,
            t.size_small * 0.9,
            Rect::new(ci.x, ci.y + BUTTON_H, ci.w, 2 * SAYS_LINE_H),
            t.text_dim,
            "Pick a track, tap one, then press a button on your controller. It is kept with this song.",
        );
        let (steps, buttons) = clip_rects(l);
        stepper(cx, steps[0], steps[1], &self.clip_track_name(m));
        let track = self.clip_track.min(Self::clip_tracks(m) - 1) as u8;
        for (i, r) in buttons.into_iter().enumerate() {
            let lit = self.waits_for(m, clip_target(i, track));
            button(cx, r, CLIP_BUTTONS[i], lit);
        }
        let last = pick_rects(l)[7];
        para(
            cx,
            t.size_small * 0.9,
            Rect::new(
                ci.x,
                last.bottom() + 6,
                ci.w,
                ci.bottom() - last.bottom() - 6,
            ),
            t.text_faint,
            "A key turns Mute and Solo on, then off; in Perform they wait for the line. \
             Clips and scenes: hold SHIFT and tap them on Arrange's Clips grid.",
        );
    }

    /// The looper card (the footswitch): the Loop stepper, that loop's
    /// Key and Mute and the looper's own presses, the one Learn waits for
    /// lit.
    fn paint_looper(&self, cx: &mut PaintCx, m: &IoModel, l: &Layout, back: bool) {
        let t = cx.theme.clone();
        cx.canvas.fill_round_rect(l.card, 12, t.surface);
        let ci = l.card.inset(14);
        cx.text(
            FontId::Bold,
            t.size_small,
            Rect::new(ci.x, ci.y, l.again.x - ci.x - 8, BUTTON_H),
            Align::Left,
            t.text_dim,
            "LOOPER",
        );
        if back {
            button(cx, l.remove, "Back", false);
        }
        para(
            cx,
            t.size_small * 0.9,
            Rect::new(ci.x, ci.y + BUTTON_H, ci.w, 2 * SAYS_LINE_H),
            t.text_dim,
            "Tap one, then press a footswitch or a button on your controller. It works in every project.",
        );
        let (steps, buttons) = looper_rects(l);
        stepper(cx, steps[0], steps[1], &loop_words(self.loop_which));
        for (i, r) in buttons.into_iter().enumerate() {
            let lit = self.waits_for(m, looper_target(i, self.loop_which));
            button(cx, r, LOOPER_BUTTONS[i], lit);
        }
        let last = pick_rects(l)[7];
        para(
            cx,
            t.size_small * 0.9,
            Rect::new(
                ci.x,
                last.bottom() + 6,
                ci.w,
                ci.bottom() - last.bottom() - 6,
            ),
            t.text_faint,
            "A loop's key does what its pad does; held for a second it clears the loop. \
             The MPC One has no footswitch jack: plug the footswitch into a MIDI controller or pedal.",
        );
    }

    /// The cards under the list: the picked row's (or a card of buttons)
    /// and MIDI Follow's, side by side or the one asked for.
    fn paint_cards(&self, cx: &mut PaintCx, m: &IoModel, l: &Layout, n: usize) {
        if self.follow_shown(l) {
            self.paint_follow(cx, m, l.follow);
        }
        if !self.card_shown(l) {
            return;
        }
        match self.buttons_shown(n) {
            Some(Buttons::Transport) => self.paint_transport(cx, m, l, n > 0),
            Some(Buttons::Clips) => self.paint_clips(cx, m, l, n > 0),
            Some(Buttons::Looper) => self.paint_looper(cx, m, l, n > 0),
            None => self.paint_card(cx, m, l, self.pick.min(n - 1)),
        }
    }

    fn paint_card(&self, cx: &mut PaintCx, m: &IoModel, l: &Layout, k: usize) {
        let t = cx.theme.clone();
        cx.canvas.fill_round_rect(l.card, 12, t.surface);
        let Some((kept, s)) = m.setup.learned_at(k) else {
            return;
        };
        let name = name_of(m, k, s);
        let (main, _) = split_name(&name);
        let ci = l.card.inset(14);
        // Its CC unnamed: the title says what it moves.
        let title = format!("{} → {}", control_words(s, false, None), main).to_uppercase();
        cx.text(
            FontId::Bold,
            t.size_small,
            Rect::new(ci.x, ci.y, l.again.x - ci.x - 8, BUTTON_H),
            Align::Left,
            t.text_dim,
            &title,
        );
        let waiting =
            self.again == Some(k) && (m.asks.learn_again == Some(k) || m.live.learning.is_some());
        button(cx, l.again, "Learn again", waiting);
        button(cx, l.remove, "Remove", false);
        let live = m.live.learn_live.get(k);
        let label = |cx: &mut PaintCx, r: Rect, s: &str| {
            cx.text(
                FontId::Sans,
                t.size_body,
                Rect::new(r.x, r.y, LABEL_W, r.h),
                Align::Left,
                t.text,
                s,
            );
        };
        // Range.
        label(cx, l.line[0], "Range");
        let end = |v: f32, text: Option<&String>| match text {
            Some(s) if !s.is_empty() => s.clone(),
            _ => format!("{:.0} %", v * 100.0),
        };
        let rr = range_rects(l.line[0]);
        stepper(cx, rr[0], rr[1], &end(s.lo, live.map(|v| &v.lo_text)));
        stepper(cx, rr[2], rr[3], &end(s.hi, live.map(|v| &v.hi_text)));
        cx.text(
            FontId::Sans,
            t.size_small,
            Rect::new(rr[1].right(), rr[1].y, rr[2].x - rr[1].right(), rr[1].h),
            Align::Center,
            t.text_dim,
            "to",
        );
        // Works as.
        label(cx, l.line[1], "Works as");
        let [down, up] = mode_rects(l.line[1]);
        let mode = MODES.iter().find(|x| x.0 == s.mode).map_or("?", |x| x.1);
        stepper(cx, down, up, mode);
        // Takeover.
        label(cx, l.line[2], "Takeover");
        let knob = s.mode == Mode::Absolute;
        for (i, sr) in segments(l.line[2], TAKEOVERS.len()).into_iter().enumerate() {
            segment(
                cx,
                sr,
                TAKEOVERS[i].1,
                knob && s.takeover == TAKEOVERS[i].0,
                knob,
            );
        }
        // Kept in.
        label(cx, l.line[3], "Kept in");
        let segs = segments(l.line[3], KEPT.len());
        for (i, sr) in segs.iter().enumerate() {
            segment(cx, *sr, KEPT[i].1, kept == KEPT[i].0, true);
        }
        let last = segs[KEPT.len() - 1];
        cx.text(
            FontId::Sans,
            t.size_small,
            Rect::new(
                last.right() + 12,
                last.y,
                l.line[3].right() - last.right() - 12,
                last.h,
            ),
            Align::Left,
            t.text_dim,
            match kept {
                Kept::Rig => "every project",
                Kept::Project => "this song",
            },
        );
        // What the takeover does, or why there is none.
        let why = if knob {
            TAKEOVERS
                .iter()
                .find(|x| x.0 == s.takeover)
                .map_or("", |x| x.2)
        } else {
            "Takeover is for knobs and faders: a button or an encoder never jumps."
        };
        let lr = Rect::new(ci.x, l.line[3].bottom() + 8, ci.w, 20);
        cx.text(FontId::Sans, t.size_small, lr, Align::Left, t.text_dim, why);
    }
}

fn button(cx: &mut PaintCx, r: Rect, text: &str, lit: bool) {
    let t = cx.theme.clone();
    let (fill, ink) = if lit {
        (t.accent2, t.on_accent)
    } else {
        (t.raised, t.text)
    };
    cx.canvas.fill_round_rect(r, 10, fill);
    cx.text(FontId::Sans, t.size_small, r, Align::Center, ink, text);
}

fn segment(cx: &mut PaintCx, r: Rect, text: &str, on: bool, enabled: bool) {
    let t = cx.theme.clone();
    let (fill, ink) = match (on, enabled) {
        (true, _) => (t.accent, t.on_accent),
        (false, true) => (t.raised, t.text),
        (false, false) => (t.raised, t.text_faint),
    };
    cx.canvas.fill_round_rect(r, 10, fill);
    cx.text(FontId::Bold, t.size_small, r, Align::Center, ink, text);
}

fn stepper(cx: &mut PaintCx, minus: Rect, plus: Rect, value: &str) {
    let t = cx.theme.clone();
    for (r, s) in [(minus, "−"), (plus, "+")] {
        cx.canvas.fill_round_rect(r, 8, t.raised);
        cx.text(FontId::Bold, t.size_body, r, Align::Center, t.text, s);
    }
    let mid = Rect::new(
        minus.right() + 4,
        minus.y,
        plus.x - minus.right() - 8,
        minus.h,
    );
    cx.canvas.fill_round_rect(mid, 8, t.bg);
    cx.text(
        FontId::Sans,
        t.size_small,
        mid,
        Align::Center,
        t.text,
        value,
    );
}

impl IoSection for Learn {
    fn label(&self) -> &str {
        crate::LEARN
    }

    fn hint(&self) -> Option<&str> {
        if self.buttons == Some(Buttons::Clips) && !self.follow {
            return Some(CLIPS_HINT);
        }
        if self.buttons == Some(Buttons::Looper) && !self.follow {
            return Some(LOOPER_HINT);
        }
        Some(HINT)
    }

    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let l = layout(r, m.live.learning.is_some());
        // Learning: what the next control moved takes.
        if let (Some(b), Some(name)) = (l.banner, &m.live.learning) {
            cx.canvas.fill_round_rect(b, 12, t.surface);
            cx.canvas
                .fill_round_rect(Rect::new(b.x, b.y + 6, 4, b.h - 12), 2, t.accent2);
            let chip = Rect::new(b.x + 16, b.y + 12, 96, b.h - 24);
            cx.canvas.fill_round_rect(chip, 6, t.accent2);
            cx.text(
                FontId::Bold,
                t.size_small * 0.85,
                chip,
                Align::Center,
                t.on_accent,
                "LEARNING",
            );
            let text = Rect::new(chip.right() + 12, b.y, l.cancel.x - chip.right() - 20, b.h);
            cx.text(
                FontId::Sans,
                t.size_small,
                text,
                Align::Left,
                t.text,
                &format!("{name}: now move a knob, a key or a pedal on any controller"),
            );
            button(cx, l.cancel, "Cancel", false);
        }
        // The heading and the key.
        let all: Vec<(Kept, &Saved)> = m.setup.all_learned().collect();
        // The words end before the buttons: "LEARNED · N" where the
        // long words don't fit (cut short rather than under them).
        let words = Rect::new(l.head.x, l.head.y, l.looper.x - 8 - l.head.x, l.head.h);
        let mut heading = format!("WHAT YOUR CONTROLLERS MOVE · {}", all.len());
        if cx.measure(FontId::Bold, t.size_small, &heading) > words.w {
            heading = format!("LEARNED · {}", all.len());
        }
        cx.text(
            FontId::Bold,
            t.size_small,
            words,
            Align::Left,
            t.text_dim,
            &heading,
        );
        let card = self.buttons_shown(all.len());
        button(
            cx,
            l.transport,
            "Transport and tempo",
            card == Some(Buttons::Transport),
        );
        button(
            cx,
            l.clips,
            "Clips and scenes",
            card == Some(Buttons::Clips),
        );
        button(cx, l.looper, "Looper", card == Some(Buttons::Looper));
        if let Some(b) = l.follow_button {
            button(cx, b, "MIDI Follow", self.follow);
        }
        if let Some(key) = l.key {
            cx.text(
                FontId::Sans,
                t.size_small * 0.85,
                key,
                Align::Right,
                t.text_faint,
                "RIG: every project  ·  PROJECT: this song",
            );
        }
        cx.canvas.fill_round_rect(l.table, 12, t.surface);
        let cols = columns(l.body);
        let inner = l.table.inset(10);
        for (i, h) in ["Control", "Moves", "Now", "Kept in"].iter().enumerate() {
            cx.text(
                FontId::Sans,
                t.size_small * 0.85,
                Rect::new(cols[i].0 + 10, inner.y, cols[i].1 - 10, TABLE_HEAD_H),
                Align::Left,
                t.text_faint,
                h,
            );
        }
        if all.is_empty() {
            let lr = Rect::new(l.body.x + 10, l.body.y + 8, l.body.w - 20, 24);
            cx.text(
                FontId::Sans,
                t.size_small,
                lr,
                Align::Left,
                t.text_dim,
                "No controls learned yet.",
            );
            let lr = Rect::new(lr.x, lr.bottom(), lr.w, 24);
            cx.text(
                FontId::Sans,
                t.size_small,
                lr,
                Align::Left,
                t.text_faint,
                "Hold SHIFT and touch a value on any page, then move a knob on your controller.",
            );
            self.paint_cards(cx, &m, &l, 0);
            return;
        }
        let shown = Self::rows_shown(l.body);
        let top = self.top_for(all.len(), shown);
        for (k, (kept, s)) in all.iter().enumerate() {
            let Some(rr) = Self::row_rect(&l, top, k) else {
                continue;
            };
            let idle = idle_why(&m, *kept, s);
            if k == self.pick {
                cx.canvas.stroke_round_rect(rr, 8, 2, t.accent2);
            } else if k % 2 == 1 {
                cx.canvas.fill_round_rect(rr, 8, t.raised);
            }
            let ink = if idle.is_some() { t.text_faint } else { t.text };
            let dim = if idle.is_some() {
                t.text_faint
            } else {
                t.text_dim
            };
            // Control: the device, then what on it.
            let (x, w) = cols[0];
            let c = Rect::new(x + 10, rr.y, w - 14, rr.h);
            let port = port_name(&m, s);
            let pw = (port.chars().count() as i32 * 9 + 10).min(c.w / 2);
            cx.text(
                FontId::Sans,
                t.size_small,
                Rect::new(c.x, c.y, pw, c.h),
                Align::Left,
                ink,
                &port,
            );
            cx.text(
                FontId::Sans,
                t.size_small * 0.9,
                Rect::new(c.x + pw, c.y, c.w - pw, c.h),
                Align::Left,
                dim,
                &control_name(&m, s),
            );
            // Moves: the parameter, then where it is (or why it waits).
            let (x, w) = cols[1];
            let c = Rect::new(x + 10, rr.y, w - 14, rr.h);
            let name = name_of(&m, k, s);
            let (main, rest) = split_name(&name);
            let mw = (main.chars().count() as i32 * 9 + 12).min(c.w * 3 / 5);
            cx.text(
                FontId::Sans,
                t.size_small,
                Rect::new(c.x, c.y, mw, c.h),
                Align::Left,
                ink,
                main,
            );
            let sub = idle.unwrap_or(rest);
            let sub_ink = match idle {
                Some("its port only plays tracks") => t.warn,
                Some(_) => t.text_faint,
                None => t.text_dim,
            };
            cx.text(
                FontId::Sans,
                t.size_small * 0.9,
                Rect::new(c.x + mw, c.y, c.w - mw, c.h),
                Align::Left,
                sub_ink,
                sub,
            );
            // Now: where its parameter stands.
            let (x, w) = cols[2];
            let bar = Rect::new(x + 10, rr.y + rr.h / 2 - 3, w - 30, 6);
            match m.live.learn_live.get(k).and_then(|v| v.now) {
                Some(v) => {
                    cx.canvas.fill_round_rect(bar, 3, t.bg);
                    let fill = (bar.w as f32 * v.clamp(0.0, 1.0)) as i32;
                    if fill > 0 {
                        cx.canvas.fill_round_rect(
                            Rect::new(bar.x, bar.y, fill, bar.h),
                            3,
                            t.accent2,
                        );
                    }
                }
                None => {
                    cx.text(
                        FontId::Sans,
                        t.size_small,
                        Rect::new(x + 10, rr.y, w - 14, rr.h),
                        Align::Left,
                        t.text_faint,
                        "—",
                    );
                }
            }
            // Kept in.
            let (x, _) = cols[3];
            let chip = Rect::new(x + 6, rr.y + 7, 80, rr.h - 14);
            let (fill, word) = match kept {
                Kept::Rig => (t.accent, "RIG"),
                Kept::Project => (t.accent2, "PROJECT"),
            };
            cx.canvas.stroke_round_rect(chip, 6, 1, fill);
            cx.text(
                FontId::Bold,
                t.size_small * 0.8,
                chip,
                Align::Center,
                fill,
                word,
            );
        }
        if all.len() > shown {
            let lr = Rect::new(l.body.x + 10, l.table.bottom() - 22, l.body.w - 20, 18);
            cx.text(
                FontId::Sans,
                t.size_small * 0.85,
                lr,
                Align::Left,
                t.text_faint,
                &format!(
                    "rows {}–{} of {}",
                    top + 1,
                    (top + shown).min(all.len()),
                    all.len()
                ),
            );
        }
        self.paint_cards(cx, &m, &l, all.len());
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let l = layout(r, m.live.learning.is_some());
        let mut out = Vec::new();
        if l.banner.is_some() {
            out.push(Target::tap("learn-cancel", 0, "Cancel learning", l.cancel));
        }
        let all: Vec<(Kept, &Saved)> = m.setup.all_learned().collect();
        out.push(Target::tap(
            "learn-transport",
            0,
            "Transport and tempo",
            l.transport,
        ));
        out.push(Target::tap("learn-clips", 0, "Clips and scenes", l.clips));
        out.push(Target::tap("learn-looper", 0, "Looper controls", l.looper));
        if let Some(b) = l.follow_button {
            out.push(Target::tap("learn-follow", 0, "MIDI Follow", b));
        }
        let top = self.top_for(all.len(), Self::rows_shown(l.body));
        for (k, (_, s)) in all.iter().enumerate() {
            if let Some(rr) = Self::row_rect(&l, top, k) {
                out.push(Target::tap(
                    "learn-row",
                    k,
                    format!(
                        "{} {}: {}",
                        port_name(&m, s),
                        control_name(&m, s),
                        name_of(&m, k, s)
                    ),
                    rr,
                ));
            }
        }
        if self.follow_shown(&l) {
            follow_targets(&m, l.follow, &mut out);
        }
        if !self.card_shown(&l) {
            return out;
        }
        if let Some(b) = self.buttons_shown(all.len()) {
            if !all.is_empty() {
                out.push(Target::tap("learn-back", 0, "Back", l.remove));
            }
            match b {
                Buttons::Transport => {
                    for (i, r) in pick_rects(&l).into_iter().enumerate() {
                        out.push(Target::tap("learn-pick", i, transport::ALL[i].1, r));
                    }
                }
                Buttons::Clips => {
                    let (steps, buttons) = clip_rects(&l);
                    let name = self.clip_track_name(&m);
                    out.push(Target::tap(
                        "learn-clip-track",
                        0,
                        format!("Track before {name}"),
                        steps[0],
                    ));
                    out.push(Target::tap(
                        "learn-clip-track",
                        1,
                        format!("Track after {name}"),
                        steps[1],
                    ));
                    for (i, r) in buttons.into_iter().enumerate() {
                        let label = match i {
                            0..=2 => format!("{} {name}", CLIP_BUTTONS[i]),
                            _ => CLIP_BUTTONS[i].to_string(),
                        };
                        out.push(Target::tap("learn-clip", i, label, r));
                    }
                }
                Buttons::Looper => {
                    let (steps, buttons) = looper_rects(&l);
                    out.push(Target::tap("learn-loop", 0, "Loop −", steps[0]));
                    out.push(Target::tap("learn-loop", 1, "Loop +", steps[1]));
                    for (i, r) in buttons.into_iter().enumerate() {
                        out.push(Target::tap("learn-looper-pick", i, LOOPER_BUTTONS[i], r));
                    }
                }
            }
            return out;
        }
        let Some((_, s)) = all.get(self.pick.min(all.len().saturating_sub(1))) else {
            return out;
        };
        out.push(Target::tap("learn-again", 0, "Learn again", l.again));
        out.push(Target::tap("learn-remove", 0, "Remove", l.remove));
        let rr = range_rects(l.line[0]);
        for (i, label) in ["Low −", "Low +", "High −", "High +"].iter().enumerate() {
            out.push(Target::tap("learn-range", i, *label, rr[i]));
        }
        let [down, up] = mode_rects(l.line[1]);
        out.push(Target::tap("learn-mode", 0, "Works as −", down));
        out.push(Target::tap("learn-mode", 1, "Works as +", up));
        for (i, sr) in segments(l.line[2], TAKEOVERS.len()).into_iter().enumerate() {
            out.push(
                Target::tap("learn-takeover", i, TAKEOVERS[i].1, sr)
                    .disabled_if(s.mode != Mode::Absolute),
            );
        }
        for (i, sr) in segments(l.line[3], KEPT.len()).into_iter().enumerate() {
            out.push(Target::tap("learn-kept", i, KEPT[i].1, sr));
        }
        out
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> IoReply {
        match phase {
            Touch::Down => {
                let h = self.hit(x, y, r);
                self.down = Some((finger, h));
                IoReply::Handled
            }
            Touch::Move => IoReply::Handled,
            Touch::Cancel => {
                self.down = None;
                IoReply::Handled
            }
            Touch::Up => {
                let Some((f, Some(h))) = self.down.take() else {
                    return IoReply::Handled;
                };
                if f != finger || self.hit(x, y, r) != Some(h) {
                    return IoReply::Handled;
                }
                self.tap(h)
            }
        }
    }

    fn input(&mut self, ev: IoInput) -> IoReply {
        let n = {
            let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
            m.setup.all_learned().count()
        };
        match ev {
            IoInput::Wheel(d) | IoInput::Page(d) => {
                let d = if matches!(ev, IoInput::Page(_)) {
                    d * 4
                } else {
                    d
                };
                let to = (self.pick as i32 + d).clamp(0, n.saturating_sub(1) as i32) as usize;
                if to == self.pick {
                    self.note = Some(if n == 0 {
                        "No controls learned yet".to_string()
                    } else if d > 0 {
                        "The last one is picked".to_string()
                    } else {
                        "The first one is picked".to_string()
                    });
                    return IoReply::Handled;
                }
                self.pick = to;
                IoReply::Redraw
            }
            IoInput::Step(d)
                if d != 0 && !self.follow && self.buttons_shown(n) == Some(Buttons::Clips) =>
            {
                self.clip_step(d)
            }
            // The looper's card: - and + step its Loop.
            IoInput::Step(d)
                if d != 0 && !self.follow && self.buttons_shown(n) == Some(Buttons::Looper) =>
            {
                self.loop_step(d)
            }
            IoInput::Step(_) if self.buttons_shown(n).is_some() || self.follow => IoReply::Handled,
            IoInput::Step(d) if d != 0 => self.mode(d.signum()),
            IoInput::Step(_) => IoReply::Handled,
        }
    }

    fn tick(&mut self, _now: std::time::Duration) -> IoReply {
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let now = (m.gen, m.live.learning.is_some(), Self::now_hash(&m));
        if now != self.drawn {
            self.drawn = now;
            return IoReply::Redraw;
        }
        IoReply::Ignored
    }

    fn take_note(&mut self) -> Option<String> {
        self.note.take()
    }

    fn qlinks(&self) -> Option<[IoKnob; 4]> {
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let off = || IoKnob {
            label: String::new(),
            text: String::new(),
            pos: 0.0,
            enabled: false,
        };
        let Some((_, s)) = m
            .setup
            .learned_at(self.pick)
            .filter(|_| self.buttons.is_none() && !self.follow)
        else {
            return Some([off(), off(), off(), off()]);
        };
        let live = m.live.learn_live.get(self.pick);
        let end = |v: f32, text: Option<&String>| match text {
            Some(x) if !x.is_empty() => x.clone(),
            _ => format!("{:.0} %", v * 100.0),
        };
        let mode = MODES.iter().position(|x| x.0 == s.mode).unwrap_or(0);
        let take = TAKEOVERS
            .iter()
            .position(|x| x.0 == s.takeover)
            .unwrap_or(0);
        Some([
            IoKnob {
                label: "Low".to_string(),
                text: end(s.lo, live.map(|v| &v.lo_text)),
                pos: s.lo,
                enabled: true,
            },
            IoKnob {
                label: "High".to_string(),
                text: end(s.hi, live.map(|v| &v.hi_text)),
                pos: s.hi,
                enabled: true,
            },
            IoKnob {
                label: "Works as".to_string(),
                text: MODES[mode].1.to_string(),
                pos: mode as f32 / (MODES.len() - 1) as f32,
                enabled: true,
            },
            IoKnob {
                label: "Takeover".to_string(),
                text: TAKEOVERS[take].1.to_string(),
                pos: take as f32 / (TAKEOVERS.len() - 1) as f32,
                enabled: s.mode == Mode::Absolute,
            },
        ])
    }

    fn qlink_turn(&mut self, knob: usize, detents: i32, fine: bool) -> IoReply {
        if detents == 0 || self.buttons.is_some() || self.follow {
            return IoReply::Ignored;
        }
        let step = if fine { RANGE_STEP / 10.0 } else { RANGE_STEP };
        match knob {
            0 | 1 => self.range(knob, detents, step),
            2 => self.mode(detents.signum()),
            3 => {
                let at = {
                    let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
                    m.setup
                        .learned_at(self.pick)
                        .and_then(|(_, s)| TAKEOVERS.iter().position(|x| x.0 == s.takeover))
                        .unwrap_or(0)
                };
                let to = (at as i32 + detents.signum()).clamp(0, TAKEOVERS.len() as i32 - 1);
                self.takeover(to as usize)
            }
            _ => IoReply::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::form::testing::{paint, paint_in, AREA};
    use crate::LearnLive;
    use std::sync::{Arc, Mutex};

    fn learned(port: Option<&str>, cc: u8, param: u16) -> Saved {
        use mpfree_param::{OnTrack, ParamAddr};
        Saved {
            port: port.map(String::from),
            channel: 0,
            source: Source::Cc(cc),
            target: ParamAddr::new(OnTrack::Inst.on(1), param),
            lo: 0.0,
            hi: 1.0,
            mode: Mode::Absolute,
            takeover: Takeover::Jump,
        }
    }

    /// The sample rig with two controls learned for the song and one, the
    /// tempo fader on the 5-pin jacks, for every project.
    fn model() -> Shared {
        let mut m = crate::sample::rig();
        m.setup.learned(learned(Some("Keystep 37"), 74, 1));
        m.setup.learned(learned(Some("Digitakt"), 1, 2));
        m.setup
            .learned_in(Kept::Rig, learned(Some(DIN_NAME), 20, 3));
        m.live.learn_names = vec![
            "Cutoff (Filter, Bass)".into(),
            "Send A (Lead)".into(),
            "Volume (Drums)".into(),
        ];
        m.live.learn_live = vec![
            LearnLive {
                now: Some(0.5),
                lo_text: "20 Hz".into(),
                hi_text: "20.0 kHz".into(),
            },
            LearnLive::default(),
            LearnLive {
                now: Some(0.8),
                ..LearnLive::default()
            },
        ];
        Arc::new(Mutex::new(m))
    }

    /// The section as the mixer's I/O view gives it on the MPC's 1280 x
    /// 800 screen: wide enough for the Follow card beside the picked
    /// row's.
    const WIDE: Rect = Rect::new(194, 136, 912, 614);

    fn tap(s: &mut Learn, id: &str, index: usize) -> IoReply {
        tap_in(s, AREA, id, index)
    }

    fn tap_in(s: &mut Learn, area: Rect, id: &str, index: usize) -> IoReply {
        let r = s
            .targets(area)
            .into_iter()
            .find(|t| t.id == id && t.index == index)
            .unwrap_or_else(|| panic!("no {id} {index}"))
            .rect;
        let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
        s.touch(1, Touch::Down, x, y, area);
        s.touch(1, Touch::Up, x, y, area)
    }

    fn ids(s: &Learn, area: Rect) -> Vec<&'static str> {
        s.targets(area).iter().map(|t| t.id).collect()
    }

    fn fits(s: &Learn, area: Rect) {
        let _ = paint_in(s, area);
        for t in s.targets(area) {
            let r = t.rect;
            assert!(
                r.x >= area.x
                    && r.y >= area.y
                    && r.right() <= area.right()
                    && r.bottom() <= area.bottom(),
                "{} \"{}\" at {r:?} is outside {area:?}",
                t.id,
                t.label
            );
        }
    }

    #[test]
    fn every_control_fits_the_section() {
        for learning in [false, true] {
            let m = model();
            m.lock().unwrap().live.learning = learning.then(|| "Drive (Filter, Bass)".to_string());
            let s = Learn::new(m);
            let _ = paint(&s);
            let ts = s.targets(AREA);
            assert!(ts.len() > 10, "{} targets", ts.len());
            for t in ts {
                let r = t.rect;
                assert!(
                    r.x >= AREA.x
                        && r.y >= AREA.y
                        && r.right() <= AREA.right()
                        && r.bottom() <= AREA.bottom(),
                    "{} \"{}\" at {r:?} is outside {AREA:?}",
                    t.id,
                    t.label
                );
            }
        }
        // With nothing learned it says how to learn one, and the card
        // offers the transport.
        let mut s = Learn::new(Arc::new(Mutex::new(crate::sample::rig())));
        let _ = paint(&s);
        let empty = ids(&s, AREA);
        assert_eq!(
            empty[..4],
            [
                "learn-transport",
                "learn-clips",
                "learn-looper",
                "learn-follow"
            ]
        );
        assert_eq!(empty[4..], ["learn-pick"; 8]);
        // Clips and scenes in its place, and back.
        tap(&mut s, "learn-clips", 0);
        fits(&s, AREA);
        assert!(ids(&s, AREA).contains(&"learn-clip"));
        assert!(!ids(&s, AREA).contains(&"learn-back"));
        tap(&mut s, "learn-clips", 0);
        assert!(ids(&s, AREA).contains(&"learn-pick"));
        // The transport card fits too, and Clips and scenes' and the
        // looper's.
        for card in ["learn-transport", "learn-clips", "learn-looper"] {
            let mut s = Learn::new(model());
            assert_eq!(tap(&mut s, card, 0), IoReply::Redraw);
            fits(&s, AREA);
            fits(&s, WIDE);
        }
        // On the MPC's screen the Follow card is beside the others.
        for transport in [false, true] {
            let m = model();
            m.lock().unwrap().setup.follow.knobs = vec![74, 71];
            let mut s = Learn::new(m);
            if transport {
                tap_in(&mut s, WIDE, "learn-transport", 0);
            }
            fits(&s, WIDE);
            let ids = ids(&s, WIDE);
            assert!(ids.contains(&"follow-clear"), "{ids:?}");
            assert!(ids.contains(&"learn-pick") == transport, "{ids:?}");
            assert!(!ids.contains(&"learn-follow"), "{ids:?}");
        }
        // Narrower, the heading's MIDI Follow shows it in their place.
        let mut s = Learn::new(model());
        assert_eq!(tap(&mut s, "learn-follow", 0), IoReply::Redraw);
        fits(&s, AREA);
        let ids = ids(&s, AREA);
        assert!(ids.contains(&"follow-on") && !ids.contains(&"learn-again"));
    }

    #[test]
    fn midi_follow_picks_a_controller_learns_its_knobs_and_says_why_it_waits() {
        let m = model();
        let mut s = Learn::new(m.clone());
        assert!(!ids(&s, AREA).contains(&"follow-on"));
        assert_eq!(tap(&mut s, "learn-follow", 0), IoReply::Redraw);
        let says = |m: &Shared| follow_says(&m.lock().unwrap());
        let knobs = |m: &Shared| knobs_text(&m.lock().unwrap());
        assert!(says(&m).starts_with("Pick the controller"), "{}", says(&m));
        // Its Q-Links and - and + leave the picked row alone meanwhile.
        assert!(s.qlinks().unwrap().iter().all(|k| !k.enabled));
        assert_eq!(s.qlink_turn(0, 1, false), IoReply::Ignored);
        // On picks the first controller with an input: the 5-pin jacks.
        assert_eq!(tap(&mut s, "follow-on", 1), IoReply::Redraw);
        {
            let g = m.lock().unwrap();
            assert!(g.setup.follow.on);
            assert_eq!(g.setup.follow.port, DIN_NAME);
        }
        // + steps to the keyboard, and the app is told.
        let gen = m.lock().unwrap().gen;
        assert_eq!(tap(&mut s, "follow-port", 1), IoReply::Redraw);
        assert_eq!(m.lock().unwrap().setup.follow.port, "Keystep 37");
        assert!(m.lock().unwrap().gen > gen);
        assert_eq!(tap(&mut s, "follow-port", 1), IoReply::Handled);
        assert_eq!(
            s.take_note().as_deref(),
            Some("The last controller is picked")
        );
        assert!(says(&m).starts_with("No knobs yet"), "{}", says(&m));
        assert_eq!(knobs(&m), "none yet");
        // Learn knobs asks the app, which says how many it has found.
        assert_eq!(tap(&mut s, "follow-learn", 0), IoReply::Redraw);
        assert_eq!(m.lock().unwrap().asks.follow_learn, Some(true));
        let label = |s: &Learn| {
            s.targets(AREA)
                .into_iter()
                .find(|t| t.id == "follow-learn")
                .map(|t| t.label)
        };
        assert_eq!(label(&s).as_deref(), Some("Done"));
        {
            let mut g = m.lock().unwrap();
            g.asks.follow_learn = None;
            g.live.follow_learning = Some(0);
        }
        assert_eq!(knobs(&m), "listening…");
        {
            let mut g = m.lock().unwrap();
            g.live.follow_learning = Some(2);
            g.setup.follow.knobs = vec![74, 71];
        }
        assert!(says(&m).contains("2 of 4 found"), "{}", says(&m));
        assert_eq!(knobs(&m), "CC 74 71");
        assert!(!ids(&s, AREA).contains(&"follow-clear"));
        let _ = paint(&s);
        // Done stops it, keeping them.
        assert_eq!(tap(&mut s, "follow-learn", 0), IoReply::Redraw);
        assert_eq!(m.lock().unwrap().asks.follow_learn, Some(false));
        m.lock().unwrap().live.follow_learning = None;
        assert!(says(&m).starts_with("Its keys play"), "{}", says(&m));
        assert_eq!(label(&s).as_deref(), Some("Learn knobs"));
        // Send values back: off until asked for; on, the line says what
        // goes back (MIDI out is off in the sample rig).
        assert!(!m.lock().unwrap().setup.follow.send_back);
        let gen = m.lock().unwrap().gen;
        assert_eq!(tap(&mut s, "follow-back", 1), IoReply::Redraw);
        assert!(m.lock().unwrap().setup.follow.send_back);
        assert!(m.lock().unwrap().gen > gen);
        assert_eq!(tap(&mut s, "follow-back", 1), IoReply::Handled);
        assert!(
            says(&m).ends_with("Values go back once MIDI out is on (Prefs)."),
            "{}",
            says(&m)
        );
        m.lock().unwrap().live.built.notes_out = true;
        assert!(says(&m).ends_with("Its lights or faders follow the Q-Links' values."));
        let _ = paint(&s);
        assert_eq!(tap(&mut s, "follow-back", 0), IoReply::Redraw);
        assert!(!m.lock().unwrap().setup.follow.send_back);
        // Clear: the keyboard's profile's knobs again (the Q-Links' four).
        let (p, _) = mpfree_midi::profile::Profile::parse(
            "mpfree-profile 1\nname = Keystep\nmatch = Keystep 37\nknobs = 74 71 76 77 93\n",
        )
        .unwrap();
        m.lock().unwrap().profiles.add(p).unwrap();
        assert_eq!(tap(&mut s, "follow-clear", 0), IoReply::Redraw);
        assert!(m.lock().unwrap().setup.follow.knobs.is_empty());
        assert_eq!(knobs(&m), "CC 74 71 76 77 (profile)");
        // Unplugged, it waits, and can't learn.
        m.lock()
            .unwrap()
            .live
            .ports
            .retain(|p| p.name != "Keystep 37");
        assert_eq!(
            says(&m),
            "Keystep 37 isn't plugged in: Follow waits for it."
        );
        assert_eq!(tap(&mut s, "follow-learn", 0), IoReply::Handled);
        assert_eq!(
            s.take_note().as_deref(),
            Some("Keystep 37 isn't plugged in")
        );
        // It stays picked; - steps back to the jacks, clearing the knobs
        // a controller learned.
        m.lock().unwrap().setup.follow.knobs = vec![1];
        assert_eq!(tap(&mut s, "follow-port", 0), IoReply::Redraw);
        {
            let g = m.lock().unwrap();
            assert_eq!(g.setup.follow.port, DIN_NAME);
            assert!(g.setup.follow.knobs.is_empty());
        }
        // Off.
        assert_eq!(tap(&mut s, "follow-on", 0), IoReply::Redraw);
        assert!(!m.lock().unwrap().setup.follow.on);
        assert!(says(&m).starts_with("On, MIDI in's knobs"), "{}", says(&m));
        // A row goes back to its card.
        assert_eq!(tap(&mut s, "learn-row", 1), IoReply::Redraw);
        assert!(ids(&s, AREA).contains(&"learn-again"));
    }

    #[test]
    fn the_transport_card_arms_learn_on_play_and_stop_the_tempo_and_the_rest() {
        let m = model();
        let mut s = Learn::new(m.clone());
        let _ = paint(&s);
        // The row's card until the heading's button swaps it.
        assert!(!s.targets(AREA).iter().any(|t| t.id == "learn-pick"));
        tap(&mut s, "learn-transport", 0);
        let labels: Vec<String> = s
            .targets(AREA)
            .into_iter()
            .filter(|t| t.id == "learn-pick")
            .map(|t| t.label)
            .collect();
        assert_eq!(
            labels,
            [
                "PLAY and STOP",
                "PLAY",
                "STOP",
                "PLAY START",
                "REC",
                "OVERDUB",
                "TAP TEMPO",
                "Tempo"
            ]
        );
        // The Q-Links have nothing to turn while it shows.
        assert!(s.qlinks().unwrap().iter().all(|k| !k.enabled));
        assert_eq!(s.qlink_turn(0, 1, false), IoReply::Ignored);
        // PLAY and STOP: Learn waits for a control to take it.
        assert_eq!(tap(&mut s, "learn-pick", 0), IoReply::Redraw);
        assert_eq!(
            m.lock().unwrap().asks.learn_target,
            Some(ParamAddr::new(Owner::Transport, transport::PLAYING))
        );
        assert_eq!(
            s.take_note().unwrap(),
            "PLAY and STOP: press a button or move a knob on your controller"
        );
        // Tempo in its place; Cancel takes it back.
        tap(&mut s, "learn-pick", 7);
        assert_eq!(
            m.lock().unwrap().asks.learn_target,
            Some(ParamAddr::new(Owner::Transport, transport::TEMPO))
        );
        m.lock().unwrap().live.learning = Some("Tempo".into());
        let _ = paint(&s);
        tap(&mut s, "learn-cancel", 0);
        {
            let v = m.lock().unwrap();
            assert_eq!(v.asks.learn_target, None);
            assert!(v.asks.cancel_learn);
        }
        m.lock().unwrap().live.learning = None;
        // Back, or a tap on a row, shows the row's card again.
        tap(&mut s, "learn-back", 0);
        assert!(s.targets(AREA).iter().any(|t| t.id == "learn-again"));
        tap(&mut s, "learn-transport", 0);
        tap(&mut s, "learn-row", 1);
        assert!(s.targets(AREA).iter().any(|t| t.id == "learn-again"));
        assert_eq!(s.pick, 1);
    }

    #[test]
    fn the_clips_card_arms_learn_on_a_tracks_mute_solo_and_stop_and_the_grids_two() {
        let m = model();
        let mut s = Learn::new(m.clone());
        let _ = paint(&s);
        assert!(!ids(&s, AREA).contains(&"learn-clip"));
        assert_eq!(tap(&mut s, "learn-clips", 0), IoReply::Redraw);
        assert_eq!(s.hint(), Some(CLIPS_HINT));
        let labels = |s: &Learn| -> Vec<String> {
            s.targets(AREA)
                .into_iter()
                .filter(|t| t.id == "learn-clip")
                .map(|t| t.label)
                .collect()
        };
        // The sample rig's first track.
        assert_eq!(
            labels(&s),
            [
                "Mute Drums",
                "Solo Drums",
                "Stop Drums",
                "Stop all",
                "Back to song"
            ]
        );
        // The Q-Links have nothing to turn while it shows.
        assert!(s.qlinks().unwrap().iter().all(|k| !k.enabled));
        assert_eq!(s.qlink_turn(0, 1, false), IoReply::Ignored);
        // - is at the first track; + and the stepper go on to the next.
        assert_eq!(s.input(IoInput::Step(-1)), IoReply::Handled);
        assert_eq!(s.take_note().as_deref(), Some("The first track is picked"));
        assert_eq!(s.input(IoInput::Step(1)), IoReply::Redraw);
        assert_eq!(tap(&mut s, "learn-clip-track", 1), IoReply::Redraw);
        assert_eq!(labels(&s)[0], "Mute Pads");
        tap(&mut s, "learn-clip-track", 0);
        assert_eq!(labels(&s)[1], "Solo Bass");
        // Solo Bass: Learn waits for a control to take track 2's solo.
        assert_eq!(tap(&mut s, "learn-clip", 1), IoReply::Redraw);
        assert_eq!(
            m.lock().unwrap().asks.learn_target,
            Some(ParamAddr::new(Owner::Clips, clips::solo(1)))
        );
        assert_eq!(
            s.take_note().unwrap(),
            "Solo Bass: press a button or move a knob on your controller"
        );
        // Lit while the app says it waits.
        m.lock().unwrap().asks.learn_target = None;
        m.lock().unwrap().live.learning = Some("Solo Bass".into());
        assert!(s.waits_for(
            &m.lock().unwrap(),
            ParamAddr::new(Owner::Clips, clips::solo(1))
        ));
        let _ = paint(&s);
        // The grid's two in its place.
        tap(&mut s, "learn-clip", 3);
        assert_eq!(
            m.lock().unwrap().asks.learn_target,
            Some(ParamAddr::new(Owner::Clips, clips::STOP_ALL))
        );
        tap(&mut s, "learn-clip", 4);
        assert_eq!(
            m.lock().unwrap().asks.learn_target,
            Some(ParamAddr::new(Owner::Clips, clips::BACK_TO_SONG))
        );
        tap(&mut s, "learn-clip", 2);
        assert_eq!(
            m.lock().unwrap().asks.learn_target,
            Some(ParamAddr::new(Owner::Clips, clips::stop(1)))
        );
        // The last of the six tracks: + says so.
        for _ in 0..8 {
            s.input(IoInput::Step(1));
        }
        assert_eq!(s.take_note().as_deref(), Some("The last track is picked"));
        assert_eq!(labels(&s)[0], "Mute Perc");
        tap(&mut s, "learn-clip", 0);
        assert_eq!(
            m.lock().unwrap().asks.learn_target,
            Some(ParamAddr::new(Owner::Clips, clips::mute(5)))
        );
        // The transport's in its place, then Back to the picked row.
        tap(&mut s, "learn-transport", 0);
        assert!(ids(&s, AREA).contains(&"learn-pick"));
        assert_eq!(s.hint(), Some(HINT));
        tap(&mut s, "learn-clips", 0);
        tap(&mut s, "learn-back", 0);
        assert!(ids(&s, AREA).contains(&"learn-again"));
        // A track the I/O view has no name for is named by its number.
        m.lock().unwrap().live.tracks[0].clear();
        tap(&mut s, "learn-clips", 0);
        for _ in 0..8 {
            s.input(IoInput::Step(-1));
        }
        assert_eq!(labels(&s)[0], "Mute Track 1");
    }

    /// The looper card (the footswitch): the Loop stepper picks the
    /// picked loop or a loop by number, and each button arms Learn on what
    /// it says, for that loop.
    #[test]
    fn the_looper_card_arms_learn_on_a_loops_key_and_mute_and_the_presses() {
        let m = model();
        let mut s = Learn::new(m.clone());
        let _ = paint(&s);
        assert!(!ids(&s, AREA).contains(&"learn-looper-pick"));
        assert_eq!(tap(&mut s, "learn-looper", 0), IoReply::Redraw);
        let _ = paint(&s);
        let labels: Vec<String> = s
            .targets(AREA)
            .into_iter()
            .filter(|t| t.id == "learn-looper-pick")
            .map(|t| t.label)
            .collect();
        assert_eq!(
            labels,
            [
                "Key",
                "Mute",
                "Next loop",
                "Undo",
                "Redo",
                "Capture",
                "Clear"
            ]
        );
        // Nothing for the Q-Links while it shows.
        assert!(s.qlinks().unwrap().iter().all(|k| !k.enabled));
        assert_eq!(s.qlink_turn(0, 1, false), IoReply::Ignored);
        assert_eq!(s.hint(), Some(LOOPER_HINT));
        // - and + step the Loop: down from the picked loop says so, up
        // is Loop 1, and back.
        assert_eq!(s.input(IoInput::Step(-1)), IoReply::Handled);
        assert_eq!(s.take_note().unwrap(), "The picked loop is the first");
        assert_eq!(s.input(IoInput::Step(1)), IoReply::Redraw);
        assert_eq!(s.loop_which, Which::Loop(0));
        assert_eq!(s.input(IoInput::Step(-1)), IoReply::Redraw);
        let armed = |m: &Shared| m.lock().unwrap().asks.learn_target;
        let at = |id| Some(ParamAddr::new(Owner::Looper, id));
        // The picked loop's to start with.
        assert_eq!(s.loop_which, Which::Picked);
        tap(&mut s, "learn-looper-pick", 0);
        assert_eq!(armed(&m), at(looper::PICKED_KEY));
        assert_eq!(
            s.take_note().unwrap(),
            "Picked loop's key: press a footswitch, a button or a key on your controller"
        );
        tap(&mut s, "learn-looper-pick", 1);
        assert_eq!(armed(&m), at(looper::PICKED_MUTE));
        assert_eq!(
            s.take_note().unwrap(),
            "Picked loop's mute: press a key or a switch, or move a fader, on your controller"
        );
        // Lit while the app says it waits.
        assert!(s.waits_for(
            &m.lock().unwrap(),
            ParamAddr::new(Owner::Looper, looper::PICKED_MUTE)
        ));
        let _ = paint(&s);
        // Down from the picked loop: it says so.
        assert_eq!(tap(&mut s, "learn-loop", 0), IoReply::Handled);
        assert_eq!(s.take_note().unwrap(), "The picked loop is the first");
        // Loop 3.
        for _ in 0..3 {
            assert_eq!(tap(&mut s, "learn-loop", 1), IoReply::Redraw);
        }
        assert_eq!(s.loop_which, Which::Loop(2));
        assert_eq!(loop_words(s.loop_which), "Loop 3");
        assert_ne!(loop_words(Which::Picked), loop_words(Which::Loop(0)));
        tap(&mut s, "learn-looper-pick", 0);
        assert_eq!(armed(&m), at(looper::key(2)));
        assert_eq!(
            s.take_note().unwrap(),
            "Loop 3 key: press a footswitch, a button or a key on your controller"
        );
        tap(&mut s, "learn-looper-pick", 1);
        assert_eq!(armed(&m), at(looper::mute(2)));
        for (i, id) in [
            (2, looper::NEXT_LOOP),
            (3, looper::UNDO),
            (4, looper::REDO),
            (5, looper::CAPTURE),
            (6, looper::CLEAR),
        ] {
            tap(&mut s, "learn-looper-pick", i);
            assert_eq!(armed(&m), at(id));
            assert_eq!(s.armed, at(id));
        }
        // Up to Loop 8 and no further.
        for _ in 0..5 {
            tap(&mut s, "learn-loop", 1);
        }
        assert_eq!(s.loop_which, Which::Loop(7));
        assert_eq!(tap(&mut s, "learn-loop", 1), IoReply::Handled);
        assert_eq!(s.take_note().unwrap(), "Loop 8 is the last");
        // Transport and tempo from here shows the transport's card; the
        // Looper button again, or Back, shows the picked row's.
        tap(&mut s, "learn-transport", 0);
        assert!(ids(&s, AREA).contains(&"learn-pick"));
        tap(&mut s, "learn-looper", 0);
        tap(&mut s, "learn-looper", 0);
        assert!(ids(&s, AREA).contains(&"learn-again"));
        tap(&mut s, "learn-looper", 0);
        tap(&mut s, "learn-back", 0);
        assert!(ids(&s, AREA).contains(&"learn-again"));
        // With nothing learned the looper's card shows in the transport's
        // place, with no Back; Looper again shows the transport's.
        let mut s = Learn::new(Arc::new(Mutex::new(crate::sample::rig())));
        tap(&mut s, "learn-looper", 0);
        let shown = ids(&s, AREA);
        assert!(shown.contains(&"learn-looper-pick") && !shown.contains(&"learn-back"));
        tap(&mut s, "learn-looper", 0);
        assert!(ids(&s, AREA).contains(&"learn-pick"));
    }

    /// The heading's words keep their room before the buttons: the long
    /// ones where they fit, else the short.
    #[test]
    fn the_heading_says_what_is_learned_in_the_room_it_has() {
        let s = Learn::new(model());
        for area in [AREA, WIDE] {
            let l = layout(area, false);
            assert!(l.looper.x - 8 > l.head.x + 100, "{area:?}");
            assert!(l.looper.right() + 8 <= l.clips.x);
            assert!(l.clips.right() + 8 <= l.transport.x);
            fits(&s, area);
        }
    }

    #[test]
    fn the_list_says_where_each_is_kept_and_why_one_does_nothing() {
        let m = model();
        let s = Learn::new(m.clone());
        let labels: Vec<String> = s
            .targets(AREA)
            .into_iter()
            .filter(|t| t.id == "learn-row")
            .map(|t| t.label)
            .collect();
        assert_eq!(
            labels,
            [
                "Keystep 37 CC 74 Cutoff: Cutoff (Filter, Bass)",
                "Digitakt CC 1 Mod wheel: Send A (Lead)",
                "MIDI in CC 20: Volume (Drums)",
            ]
        );
        let v = m.lock().unwrap();
        let all: Vec<_> = v.setup.all_learned().collect();
        assert_eq!(all[2].0, Kept::Rig);
        // The keyboard is Learn only in the sample rig: it learns.
        assert_eq!(idle_why(&v, all[0].0, all[0].1), None);
        assert_eq!(idle_why(&v, all[1].0, all[1].1), Some("unplugged, waits"));
        assert_eq!(idle_why(&v, all[2].0, all[2].1), None);
    }

    #[test]
    fn the_card_sets_the_range_how_it_works_and_takes_over_and_where_it_is_kept() {
        let m = model();
        let mut s = Learn::new(m.clone());
        let _ = paint(&s);
        let gen = m.lock().unwrap().gen;
        // Low end up two steps, high end down one.
        assert_eq!(tap(&mut s, "learn-range", 1), IoReply::Redraw);
        tap(&mut s, "learn-range", 1);
        tap(&mut s, "learn-range", 2);
        {
            let v = m.lock().unwrap();
            assert!(v.gen > gen);
            assert!((v.setup.learn[0].lo - 0.02).abs() < 1e-6);
            assert!((v.setup.learn[0].hi - 0.99).abs() < 1e-6);
        }
        // At the bottom, the low end says so.
        tap(&mut s, "learn-range", 0);
        tap(&mut s, "learn-range", 0);
        assert_eq!(tap(&mut s, "learn-range", 0), IoReply::Handled);
        assert_eq!(s.take_note().unwrap(), "The range's end is at the bottom");
        // Scale, then an endless encoder: takeover is for knobs.
        tap(&mut s, "learn-takeover", 2);
        assert_eq!(m.lock().unwrap().setup.learn[0].takeover, Takeover::Scale);
        assert_eq!(s.input(IoInput::Step(1)), IoReply::Redraw);
        assert_eq!(
            m.lock().unwrap().setup.learn[0].mode,
            Mode::Relative(Encoding::TwosComplement)
        );
        assert!(s
            .targets(AREA)
            .iter()
            .any(|t| t.id == "learn-takeover" && !t.enabled));
        assert_eq!(tap(&mut s, "learn-takeover", 0), IoReply::Handled);
        assert_eq!(s.take_note().unwrap(), "Takeover is for knobs and faders");
        // Kept for every project: it moves to the rig's list.
        assert_eq!(tap(&mut s, "learn-kept", 0), IoReply::Redraw);
        {
            let v = m.lock().unwrap();
            assert_eq!(v.setup.learn.len(), 1);
            assert_eq!(v.setup.rig_learn.len(), 2);
            assert_eq!(v.setup.learned_at(s.pick).unwrap().0, Kept::Rig);
            assert_eq!(v.setup.learned_at(s.pick).unwrap().1.source, Source::Cc(74));
        }
        // The Q-Links turn the range and the way it works.
        let q = s.qlinks().unwrap();
        assert_eq!(q[0].label, "Low");
        assert!(!q[3].enabled);
        assert_eq!(s.qlink_turn(1, -1, true), IoReply::Redraw);
    }

    #[test]
    fn learn_again_asks_the_app_and_cancel_stops_it() {
        let m = model();
        let mut s = Learn::new(m.clone());
        let _ = paint(&s);
        s.input(IoInput::Wheel(1));
        tap(&mut s, "learn-again", 0);
        assert_eq!(m.lock().unwrap().asks.learn_again, Some(1));
        assert!(s.take_note().unwrap().starts_with("Move a knob"));
        // The app arms Learn and names what it waits for: the banner
        // shows, with Cancel.
        m.lock().unwrap().live.learning = Some("Send A (Lead)".to_string());
        assert_eq!(s.tick(std::time::Duration::ZERO), IoReply::Redraw);
        let _ = paint(&s);
        tap(&mut s, "learn-cancel", 0);
        let v = m.lock().unwrap();
        assert!(v.asks.cancel_learn);
        assert_eq!(v.asks.learn_again, None);
    }

    #[test]
    fn remove_forgets_the_picked_one() {
        let m = model();
        let mut s = Learn::new(m.clone());
        let _ = paint(&s);
        s.input(IoInput::Wheel(2));
        assert_eq!(tap(&mut s, "learn-remove", 0), IoReply::Redraw);
        let v = m.lock().unwrap();
        assert!(v.setup.rig_learn.is_empty());
        assert_eq!(v.setup.learn.len(), 2);
        assert_eq!(v.live.learn_names.len(), 2);
        assert_eq!(s.pick, 1);
    }
}
