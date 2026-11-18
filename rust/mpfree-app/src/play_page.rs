//! The Play page (layout spec, "A new Play page"; Jay 09:34Z to 10:16Z): the
//! home for finger drumming, live looping and making beats. The big pads
//! are gone, because the MPC has them; the screen shows what they can't.
//!
//! Each view is laid out exactly as Jay's approved mockups F to I
//! (/mnt/project-files/hello-mpc/play-mockups/source/src/main.rs, `views`):
//!
//! - the frame, the same in every view: the transport card (84 px) with
//!   the view switch at its right end, the view under it, and beside them,
//!   from under the top bar to the hint line, the master meter (66 px) and
//!   the four Q-Link cards (128 px) next to the MPC's Q-Link knobs;
//! - **All** (home, F): the looper strip, the pattern, and the throws
//!   beside the XY pad;
//! - **Beats** (G): the whole pattern, then (unless Play's options turn
//!   it off) the Your timing strip, then the throws;
//! - **Loop** (H): the loop big with its buttons, then the throws and the
//!   XY pad.
//!
//! The Drum view (I) is gone since 13 Nov: Jay folded its timing into
//! Beats as a strip after live test 3 ("Fold timing into Beats, drop
//! Drum").
//!
//! No view has a pads panel since 12 Nov (Jay's pick A, 15:45Z): the
//! pattern, the loop and your timing take its room, and the pads have
//! their own tab. The panels are in `play_widgets`. The arp's panel goes
//! into the same frame ([`framed`]) and takes the view. NOTE REPEAT's slim
//! strip takes the top of the view's first panel
//! ([`PlayView::quick_layout`]), so nothing else moves when it comes and
//! goes.

use crate::mixing::{self, PlayFx, PlayLevels, ThrowFx};
use crate::play_widgets::{
    Lane, Marks, RecChip, RecState, StepGrid, Throw, Throws, TimedHit, TransportLook, XyPad,
    YourTiming,
};
use crate::time::{HitInfo, StepAction, StepsData, LINE, STEP_TICKS};
use mpfree_arp::scale::pads::PadLayout;
use mpfree_arp_ui::view as arp_view;
use mpfree_fx::ids::mixer as mixer_ids;
use mpfree_fx::StripId;
use mpfree_node::{Curve, ParamInfo};
use mpfree_play::mpfree_param::RelAddr;
use mpfree_seq::steps::{Change, Step, PAGE};
use mpfree_seq::Cond;
use mpfree_time_ui::view::TransportInfo;
use mpfree_transport::time::{Tick, PPQ};
use mpfree_transport::State;
use mpfree_ui::draw;
use mpfree_ui::qlink::{QLinkCard, COLUMN_WIDTH, MASTER_WIDTH};
use mpfree_ui::text::{Align, FontId};
use mpfree_ui::{Color, EventKind, PaintCx, Rect, Tone, Touch, Widget};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Widget names on the page. The loop strip is the Looper tab's.
pub const TRANSPORT: &str = "play-transport";
pub const MASTER: &str = "master";
pub const QLINKS: &str = "qlinks";
pub const PADS: &str = "pads";
pub const STEPS: &str = "steps";
pub const THROWS: &str = "throws";
pub const XY: &str = "xy";
pub const TIMING: &str = "play-timing";
/// The step inspector under the pattern on Beats, in the throws' place
/// while a step is held or picked (sequencer mockup A).
pub const INSPECTOR: &str = "step-inspector";
/// The pads heading on the Synth tab, over its presets: always the synth.
pub const SYNTH_PADS_HEAD: &str = "synth-pads-head";

/// Play's views, in its switch's order (MAIN steps through them). Beats
/// is the Sequencer tab's Steps view, so it is not one of them.
pub const PLAY_VIEWS: [PlayView; 2] = [PlayView::All, PlayView::Loop];
/// Play's switch.
pub const PLAY_SWITCH: [&str; 2] = ["All", "Loop"];
/// The Sequencer's switch: Steps (Beats' pages) and Notes (the note
/// editor).
pub const SEQ_SWITCH: [&str; 2] = ["Steps", "Notes"];
/// The Sequencer switch's Notes.
pub const SEQ_NOTES: usize = 1;
/// Beats with the Your timing strip (Play's options, Timing strip On).
pub const BEATS_TIMING_PAGE: &str = "Play · Beats · timing";
/// Beats with a step held or picked: the step inspector in the throws'
/// place, without and with the timing strip.
pub const BEATS_STEP_PAGE: &str = "Play · Beats · step";
pub const BEATS_TIMING_STEP_PAGE: &str = "Play · Beats · timing · step";
/// The Your timing strip's height in Beats.
pub const TIMING_STRIP_H: i32 = 72;
/// The transport card's height.
pub const TRANSPORT_H: i32 = 84;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlayView {
    #[default]
    All,
    Beats,
    Loop,
}

impl PlayView {
    pub const ALL: [PlayView; 3] = [PlayView::All, PlayView::Beats, PlayView::Loop];

    pub fn name(self) -> &'static str {
        match self {
            PlayView::All => "All",
            PlayView::Beats => "Steps",
            PlayView::Loop => "Loop",
        }
    }

    /// Play's view after this one (MAIN on Play): All and Loop in turn.
    pub fn next_on_play(self) -> PlayView {
        match self {
            PlayView::All => PlayView::Loop,
            PlayView::Beats | PlayView::Loop => PlayView::All,
        }
    }

    /// The view's page in the `Ui`. All, the home view, is "Play", as
    /// before.
    pub fn page(self) -> &'static str {
        match self {
            PlayView::All => "Play",
            PlayView::Beats => "Play · Beats",
            PlayView::Loop => "Play · Loop",
        }
    }

    /// The page showing for this view: Beats has a second page with the
    /// Your timing strip, for when `timing` (Play's options) is on, and
    /// with `step` (a step held or picked) a page with the step inspector
    /// in the throws' place, so nothing else moves.
    pub fn page_with(self, timing: bool, step: bool) -> &'static str {
        match (self, timing, step) {
            (PlayView::Beats, true, true) => BEATS_TIMING_STEP_PAGE,
            (PlayView::Beats, false, true) => BEATS_STEP_PAGE,
            (PlayView::Beats, true, false) => BEATS_TIMING_PAGE,
            _ => self.page(),
        }
    }

    /// The view under the transport card, as the mockups lay it out. With
    /// `band`, NOTE REPEAT's strip takes the top of the first panel, so
    /// nothing else moves.
    fn body(self, band: bool, timing: bool) -> String {
        let first = |slot: &str| {
            if band {
                format!("col({}:{}, {slot}:*):*", arp_view::MINI, arp_view::MINI_H)
            } else {
                format!("{slot}:*")
            }
        };
        let strip = crate::looper_page::LOOP_STRIP;
        match self {
            PlayView::All => format!(
                "{strip}:150, {}, row({THROWS}:*, {XY}:280):120",
                first(STEPS)
            ),
            PlayView::Beats if timing => {
                format!("{}, {TIMING}:{TIMING_STRIP_H}, {THROWS}:196", first(STEPS))
            }
            PlayView::Beats => format!("{}, {THROWS}:196", first(STEPS)),
            PlayView::Loop => format!("{}, row({THROWS}:*, {XY}:300):196", first(strip)),
        }
    }

    pub fn layout(self) -> String {
        self.layout_for(false)
    }

    /// The layout of [`PlayView::page_with`]`(timing, false)`.
    pub fn layout_for(self, timing: bool) -> String {
        frame(&self.body(false, timing))
    }

    /// Beats' layout with the step inspector in the throws' place (the
    /// same height), with or without the timing strip.
    pub fn step_layout(timing: bool) -> String {
        PlayView::Beats
            .layout_for(timing)
            .replace(&format!("{THROWS}:196"), &format!("{INSPECTOR}:196"))
    }

    /// The view while NOTE REPEAT is held or pads are latched.
    pub fn quick_layout(self) -> String {
        self.quick_layout_for(false)
    }

    /// [`PlayView::quick_layout`] with or without the timing strip.
    pub fn quick_layout_for(self, timing: bool) -> String {
        frame(&self.body(true, timing))
    }
}

/// The Play frame around `body` (one or more panels, top to bottom): the
/// transport card over it, the master meter and the Q-Link cards beside.
pub fn frame(body: &str) -> String {
    format!(
        "col(header:60, row(col({TRANSPORT}:{TRANSPORT_H}, {body}):*, \
             {MASTER}:{MASTER_WIDTH}, {QLINKS}:{COLUMN_WIDTH}):*, hint:26)"
    )
}

/// A layout written for the old Play page, `col(header:60, BODY, hint:26)`,
/// in the frame: the arp's panel and band. When BODY is the pads beside a
/// column (`row(pads:*, col(...):W):*`), the column takes the whole view:
/// Play has no pads panel (Jay, 12 Nov 15:45Z). A layout already in the
/// frame comes back as it is.
pub fn framed(layout: &str) -> String {
    if layout.contains(TRANSPORT) {
        return layout.to_string();
    }
    let Some(body) = layout
        .strip_prefix("col(header:60, ")
        .and_then(|r| r.strip_suffix(", hint:26)"))
    else {
        return layout.to_string();
    };
    let side = body
        .strip_prefix("row(pads:*, ")
        .and_then(|r| r.strip_suffix("):*"))
        .and_then(|r| r.rsplit_once(':'));
    match side {
        Some((column, width)) if width.parse::<i32>().is_ok() => frame(&format!("{column}:*")),
        _ => frame(body),
    }
}

/// The Synth tab's layout with the pads heading, and its switch, over the
/// presets (Jay, 12:22Z: the switch on Play and on the Synth tab).
pub fn synth_with_pads(layout: &str) -> String {
    let (slot, presets) = (
        mpfree_synth_ui::view::PRESETS_SLOT,
        mpfree_synth_ui::view::PRESETS,
    );
    layout.replacen(
        slot,
        &format!("col({SYNTH_PADS_HEAD}:30, {presets}:*):*"),
        1,
    )
}

// ---------------------------------------------------------------- pads bar

/// While the synth has the pads: the bank playing's layout, whether a
/// scale is picked (In key needs one), and the bank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SynthPads {
    pub layout: PadLayout,
    pub scale: bool,
    pub bank: usize,
}

/// The pads panel's heading line: what the pads play, and while the synth
/// has them, the Chromatic | In key switch. A tap on a half gives `Tab(0)`
/// (Chromatic) or `Tab(1)` (In key).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PadsBar {
    /// "PADS · BANK A · BOOM BAP", "SYNTH · BANK B · C3 TO D#4".
    pub title: String,
    pub synth: Option<SynthPads>,
}

/// The switch's halves, in tap order.
pub const LAYOUT_SWITCH: [&str; 2] = ["Chromatic", "In key"];
const PILL_W: i32 = 184;

impl PadsBar {
    /// The switch's two halves in the bar's rectangle `r`.
    pub fn halves(r: Rect) -> [Rect; 2] {
        let pill = Rect::new(
            r.right() - PILL_W,
            r.y + 1,
            PILL_W.min(r.w),
            (r.h - 2).max(0),
        );
        let w = pill.w / 2;
        [
            Rect::new(pill.x, pill.y, w, pill.h),
            Rect::new(pill.x + w, pill.y, pill.w - w, pill.h),
        ]
    }
}

impl Widget for PadsBar {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        let line = r.inset_xy(4, 0);
        let Some(s) = self.synth else {
            cx.text(
                FontId::Bold,
                t.size_small,
                line,
                Align::Left,
                t.text_dim,
                &self.title.to_uppercase(),
            );
            return;
        };
        let halves = PadsBar::halves(r);
        let title = Rect::new(line.x, line.y, (halves[0].x - 8 - line.x).max(0), line.h);
        // Over the Synth tab's narrow presets column the switch alone says it.
        if title.w >= 56 {
            cx.text(
                FontId::Bold,
                t.size_small,
                title,
                Align::Left,
                t.text_dim,
                &self.title.to_uppercase(),
            );
        }
        let pill = Rect::new(
            halves[0].x,
            halves[0].y,
            halves[0].w + halves[1].w,
            halves[0].h,
        );
        cx.canvas.fill_round_rect(pill, pill.h / 2, t.raised);
        let on = match s.layout {
            PadLayout::Chromatic => Some(0),
            PadLayout::InKey => Some(1),
            PadLayout::Chords => None,
        };
        for (i, (half, name)) in halves.iter().zip(LAYOUT_SWITCH).enumerate() {
            let lit = on == Some(i);
            if lit {
                cx.canvas
                    .fill_round_rect(half.inset(2), (half.h - 4) / 2, t.accent);
            }
            let ink = if lit {
                t.bg
            } else if i == 1 && !s.scale {
                t.text_faint
            } else {
                t.text_dim
            };
            cx.text(
                if lit { FontId::Bold } else { FontId::Sans },
                t.size_small,
                *half,
                Align::Center,
                ink,
                name,
            );
        }
    }

    fn targets(&self, r: Rect) -> Vec<mpfree_ui::Target> {
        if self.synth.is_none() {
            return Vec::new();
        }
        PadsBar::halves(r)
            .iter()
            .enumerate()
            .map(|(i, h)| mpfree_ui::Target::tap("layout", i, ["Chromatic", "In key"][i], *h))
            .collect()
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down || self.synth.is_none() {
            return None;
        }
        PadsBar::halves(r)
            .iter()
            .position(|h| h.contains(x, y))
            .map(EventKind::Tab)
    }
}

// ---------------------------------------------------------------- the page

/// What the Play page's Q-Links turn. The set depends on the view (and in
/// Beats, on a step being picked); [`PlayPage::knobs`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Knob {
    Swing,
    /// The pad hit last: its tuning (the Sample tab's).
    Tune,
    DrumsFilter,
    DrumsSend,
    LoopLevel,
    LoopSend,
    LineLevel,
    LoopFilter,
    /// The step picked in Beats, or the step pads held (their Step bank).
    Velocity,
    Length,
    Nudge,
    Chance,
    /// The step pads held, after Q-LINK (their Trig bank): the trig
    /// condition, the ratchet and the nudge in ticks.
    Cond,
    Ratchet,
    NudgeTicks,
    /// A step of the synth's line (STEP SEQ on the synth): its notes move
    /// along the pads, the chord keeping its shape (`Change::Note`).
    Note,
    /// No knob: an empty card (the Trig bank's fourth, for the ratchet's
    /// ramp to come).
    Empty,
    /// The step pads held, Q-LINK twice (their Sound bank): knob `k` of
    /// [`PlayPage::sound_bank`], a lock on the held steps.
    Sound(u8),
    /// 16 Levels' Transpose (sequencer S4): the armed track's live
    /// transpose, two octaves (or 24 notes of the key) either way, in
    /// Tune's place.
    Transpose,
}

/// Which bank the Q-Links show while step pads are held: Q-LINK steps
/// Step, Trig, Sound and round. Held steps with no note have only Sound.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HeldBank {
    #[default]
    Step,
    Trig,
    Sound,
}

impl HeldBank {
    pub fn name(self) -> &'static str {
        match self {
            HeldBank::Step => "Step",
            HeldBank::Trig => "Trig",
            HeldBank::Sound => "Sound",
        }
    }

    /// The bank Q-LINK steps to.
    pub fn next(self) -> HeldBank {
        match self {
            HeldBank::Step => HeldBank::Trig,
            HeldBank::Trig => HeldBank::Sound,
            HeldBank::Sound => HeldBank::Step,
        }
    }
}

/// How a Sound bank knob reads and turns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SoundKind {
    /// A kit pad's tune, in cents.
    Tune,
    /// A kit pad's decay, in ms: the sound decays to silence.
    Decay,
    /// A kit pad's filter cutoff, in Hz (a pad without a filter gets a
    /// low-pass).
    Cutoff,
    /// A kit pad's level, in dB.
    Level,
    /// An instrument's parameter, as its node describes it.
    Param(ParamInfo),
}

/// One of the Sound bank's knobs: a setting of the held steps' sound, a
/// lock on them when turned.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundKnob {
    pub name: String,
    /// What it locks, on the clip's track.
    pub addr: RelAddr,
    pub kind: SoundKind,
    /// The sound's own value: what a step without a lock plays.
    pub own: f32,
}

/// The lock colour on a Sound card whose step has a lock, and a lock's
/// dot on a step's cell (the step pads' purple).
pub const LOCKED: Color = Color::rgb(0xA0_40_FF);

impl SoundKind {
    /// The value as the card writes it, where its ring is, and whether it
    /// is lit both ways.
    fn read(self, v: f32) -> (String, f32, bool) {
        match self {
            SoundKind::Tune => {
                let st = v / 100.0;
                let text = if (st - st.round()).abs() < 0.05 {
                    format!("{:+.0} st", st.round())
                } else {
                    format!("{st:+.1} st")
                };
                (
                    text.replace("+0 st", "0 st"),
                    ((st + 24.0) / 48.0).clamp(0.0, 1.0),
                    true,
                )
            }
            SoundKind::Decay if v >= DECAY_MAX_MS => ("open".to_string(), 1.0, false),
            SoundKind::Decay => (
                if v >= 1000.0 {
                    format!("{:.2} s", v / 1000.0)
                } else {
                    format!("{v:.0} ms")
                },
                ((v / 10.0).max(1.0).log2() / (DECAY_MAX_MS / 10.0).log2()).clamp(0.0, 1.0),
                false,
            ),
            SoundKind::Cutoff if v >= CUTOFF_OPEN_HZ => ("open".to_string(), 1.0, false),
            SoundKind::Cutoff => (
                if v >= 1000.0 {
                    format!("{:.1} kHz", v / 1000.0)
                } else {
                    format!("{v:.0} Hz")
                },
                ((v / 20.0).max(1.0).log2() / (CUTOFF_OPEN_HZ / 20.0).log2()).clamp(0.0, 1.0),
                false,
            ),
            SoundKind::Level => (db_text(v), ((v + 60.0) / 72.0).clamp(0.0, 1.0), false),
            SoundKind::Param(p) => (
                mpfree_synth_ui::format::value_line(&p, v),
                p.to_unit(v),
                p.min < 0.0 && p.max > 0.0,
            ),
        }
    }

    /// `v` turned by `ticks` detents (`fine`: SHIFT held).
    fn turn(self, v: f32, ticks: i32, fine: bool) -> f32 {
        let d = ticks as f32;
        match self {
            SoundKind::Tune => (v + d * if fine { 10.0 } else { 100.0 }).clamp(-2400.0, 2400.0),
            SoundKind::Decay => {
                let from = v.clamp(10.0, DECAY_MAX_MS);
                (from * (d / if fine { 32.0 } else { 8.0 }).exp2())
                    .round()
                    .clamp(10.0, DECAY_MAX_MS)
            }
            SoundKind::Cutoff => {
                let from = v.clamp(20.0, CUTOFF_OPEN_HZ);
                (from * (d / if fine { 48.0 } else { 12.0 }).exp2()).clamp(20.0, CUTOFF_OPEN_HZ)
            }
            SoundKind::Level => (v + d * if fine { 0.1 } else { 0.5 }).clamp(-60.0, 12.0),
            SoundKind::Param(p) => match p.curve {
                Curve::Stepped | Curve::Choice(_) | Curve::Toggle => p.clamp(v + d),
                _ => {
                    let per = mpfree_synth_ui::view::DETENTS * if fine { 10.0 } else { 1.0 };
                    p.from_unit(p.to_unit(v) + d / per)
                }
            },
        }
    }
}

/// A kit pad's cutoff that reads as an open filter.
pub const CUTOFF_OPEN_HZ: f32 = 20_000.0;

/// A Q-Link turn, for the engines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Turn {
    Fx(PlayFx),
    Step(StepAction),
    /// The transport's swing, as a percentage (50 is straight).
    Swing(u8),
    /// The pad's tuning or decay, by this many detents (the Sample page
    /// edits the kit).
    Pad(Knob, i32),
    /// The armed track's live transpose, to this (sequencer S4).
    Transpose(i8),
}

/// What the Transpose Q-Link turns (16 Levels' Transpose on an armed
/// track that plays notes): the track, and where it is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransposeKnob {
    pub track: String,
    pub by: i8,
    pub in_key: bool,
}

/// One throw: its name, what it does, its colour and the performance
/// effect it holds (None: Drop drums, which mutes the drum pads).
pub struct ThrowDef {
    pub name: &'static str,
    pub sub: &'static str,
    pub color: Color,
    pub fx: Option<ThrowFx>,
}

/// The throws, in the mockups' order (6 show on the home view, 4 on Loop).
pub const THROW_LIST: [ThrowDef; 8] = [
    ThrowDef {
        name: "Delay throw",
        sub: "1/8 dotted",
        color: draw::KNOB[0],
        fx: Some(ThrowFx::Delay),
    },
    ThrowDef {
        name: "Reverb wash",
        sub: "long, dark",
        color: draw::KNOB[3],
        fx: Some(ThrowFx::Reverb),
    },
    ThrowDef {
        name: "Stutter",
        sub: "1/16",
        color: draw::KNOB[2],
        fx: Some(ThrowFx::Stutter),
    },
    ThrowDef {
        name: "Tape stop",
        sub: "1 beat",
        color: draw::KICK,
        fx: Some(ThrowFx::TapeStop),
    },
    ThrowDef {
        name: "Filter dip",
        sub: "sweep down",
        color: draw::KNOB[1],
        fx: Some(ThrowFx::FilterDip),
    },
    ThrowDef {
        name: "Beat repeat",
        sub: "last 1/4",
        color: draw::PERC,
        fx: Some(ThrowFx::BeatRepeat),
    },
    ThrowDef {
        name: "Drop drums",
        sub: "Kick, Snare, Hat",
        color: draw::SNARE,
        fx: None,
    },
    ThrowDef {
        name: "Vinyl brake",
        sub: "2 beats",
        color: draw::FX,
        fx: Some(ThrowFx::VinylBrake),
    },
];

/// A pad of the kit as the Play page shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PadInfo {
    pub label: Option<String>,
    pub color: Option<Color>,
    /// The synth's pads: where the pad's note sits, for Beats' scale rows
    /// (None for a kit's sounds and the Drums engine's).
    pub pitch: Option<PadPitch>,
}

/// A synth pad's note (a chord pad's root) and whether it is the key's
/// root (with no scale, a C), as its light shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PadPitch {
    pub note: u8,
    pub root: bool,
}

impl PadInfo {
    /// The pad's colour on the page: its kind of sound's, else its own.
    pub fn color(&self) -> Option<Color> {
        self.label
            .as_deref()
            .and_then(draw::sound_color)
            .or(self.color)
    }
}

/// Engine values shown on the Q-Links come back from the audio thread; a
/// knob turned this recently keeps the screen's own value.
const HOLD_TURNED: Duration = Duration::from_millis(400);
/// Step lengths a Q-Link steps through, in ticks (1/128 to 2 bars).
const LENGTHS: [Tick; 12] = [30, 60, 80, 120, 160, 240, 320, 480, 720, 960, 1920, 3840];
/// The timing window: two bars.
const TIMING_BARS: i64 = 2;
/// Velocity a tapped step gets.
pub const TAP_VELOCITY: u8 = 100;

/// The Play page's state: the view, the pattern, the pad hit last, the
/// throws and the XY pad, the timing, and the mixer's values the Q-Links
/// show. It fills the page's widgets ([`PlayPage::paint_into`]) and turns
/// their events and the Q-Links into actions for the engines. Nothing here
/// draws: the widgets in `play_widgets` do.
pub struct PlayPage {
    pub view: PlayView,
    /// The page shows on the Sequencer tab (its Steps view, Beats' pages):
    /// the transport's switch is Steps | Notes there.
    pub sequencer: bool,
    pub steps: StepsData,
    /// The page of steps shown (16 a page).
    pub page: usize,
    /// The step picked in Beats: (pad, step in the clip).
    pub selected: Option<(usize, usize)>,
    /// The pad hit last (Tune and Decay turn it).
    pub last_pad: usize,
    pub pads: Vec<PadInfo>,
    pub levels: PlayLevels,
    /// The armed track plays kit 1, whose pads have channels of their own
    /// (the mixer's pad strips): its lanes show and change their mutes.
    /// Another kit plays through its track's channel, so its pads have no
    /// mutes of their own, as a synth's haven't.
    pub pad_strips: bool,
    /// The kit's page on the pads (PAD BANK A = 0): its empty pads fill
    /// the lanes left after the sounds.
    pub pad_page: usize,
    turned_at: Option<Instant>,
    pub transport: TransportInfo,
    hits: VecDeque<HitInfo>,
    pub throws: [bool; 8],
    /// The XY pad's logic: the Perform FX pad (the DJ filter across, the
    /// echo up), gliding home when the finger lifts.
    pub xy: mpfree_fx::XyPad,
    /// STEP SEQ: the sound whose steps the pads are, and their page.
    pub on_pads: Option<crate::app::StepPads>,
    /// STEP SEQ: the step pads held (their sound, and the steps, the first
    /// held first). The Q-Links set them.
    pub held: Option<(u8, Vec<usize>)>,
    /// Q-LINK while steps are held: the bank shown.
    pub bank: HeldBank,
    /// The held steps' sound's Sound bank (a kit pad's tune, decay,
    /// filter and level, or an instrument's first Q-Link set), as the
    /// screen fills it in while steps are held.
    pub sound_bank: [Option<SoundKnob>; 4],
    /// The inspector's Copy: the step (pad, step in the clip) whose copy
    /// the next step tapped on its lane takes.
    pub copy_from: Option<(usize, usize)>,
    /// A synth track's scale rows: Fold (only the notes used), and the
    /// window's lowest row as the octave's ‹ and › left it (None: from
    /// the root under the lowest note used).
    pub fold: bool,
    pub low: Option<usize>,
    /// 16 Levels' Transpose with an armed track that plays notes: the
    /// second Q-Link turns its transpose (the screen fills it in).
    pub transpose: Option<TransposeKnob>,
}

/// What the step inspector shows (Beats, a step held or picked; the
/// sequencer design's mockup A).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inspect {
    /// "Step 7 · Kick · held".
    pub title: String,
    /// The bank the Q-Links show, and whether Sound can be picked (only
    /// while steps are held: a turn there locks them).
    pub bank: HeldBank,
    pub sound: bool,
    /// The step's settings, (name, value), in [`INSPECT_CHIPS`]' order;
    /// none for steps held with no note.
    pub chips: Vec<(String, String)>,
    /// Which chips the bank's Q-Links turn.
    pub turned: [bool; 6],
    /// The step's locks: (parameter, name, value).
    pub locks: Vec<(RelAddr, String, String)>,
    /// One step: Copy and Clear can be used.
    pub one: bool,
    /// Copy waits for the step that takes the copy.
    pub copying: bool,
}

/// The inspector's chips, in order.
pub const INSPECT_CHIPS: [Knob; 6] = [
    Knob::Velocity,
    Knob::Length,
    Knob::Nudge,
    Knob::Chance,
    Knob::Cond,
    Knob::Ratchet,
];

/// A tap on the step inspector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InspectTap {
    /// One of Q-LINK's banks.
    Bank(HeldBank),
    /// The × on lock `k` of the list: it comes off.
    Unlock(usize),
    /// Copy: the next step tapped on the lane takes a copy.
    Copy,
    /// Clear: the step comes out, locks too.
    Clear,
    /// × on a step picked: picked off, the step kept (the throws come
    /// back).
    Close,
}

impl Default for PlayPage {
    fn default() -> Self {
        PlayPage {
            view: PlayView::All,
            sequencer: false,
            steps: StepsData::default(),
            page: 0,
            selected: None,
            last_pad: 0,
            pads: vec![PadInfo::default(); 16],
            levels: PlayLevels {
                drums_send_db: mixing::SEND_OFF_DB,
                loop_send_db: mixing::SEND_OFF_DB,
                loop_db: 0.0,
                line_db: 0.0,
                drums_filter: 0.0,
                loop_filter: 0.0,
                fx_mix: 1.0,
                pad_mutes: 0,
            },
            pad_strips: true,
            pad_page: 0,
            turned_at: None,
            transport: TransportInfo::default(),
            hits: VecDeque::new(),
            throws: [false; 8],
            xy: mpfree_fx::XyPad::perform(),
            on_pads: None,
            held: None,
            bank: HeldBank::Step,
            sound_bank: Default::default(),
            copy_from: None,
            fold: false,
            low: None,
            transpose: None,
        }
    }
}

/// The marks of the page of `row` from step `from` (its locks counted in
/// `locks`): each step's condition, chance, ratchet, nudge and locks, a
/// note that lasts into the next step (a tie), and the empty steps a note
/// lasts over.
fn marks(row: &[Option<Step>], locks: &[usize], from: usize) -> Vec<Marks> {
    let size = STEP_TICKS;
    // Where the last note so far ends, from the row's start.
    let mut reach: Tick = 0;
    let mut held = vec![false; row.len()];
    for (i, s) in row.iter().enumerate() {
        let line = i as Tick * size;
        if s.is_none() && line < reach {
            held[i] = true;
        }
        if let Some(s) = s {
            reach = reach.max(line + s.nudge + s.len);
        }
    }
    (from..from + PAGE)
        .map(|i| {
            let n = locks.get(i).copied().unwrap_or(0).min(255) as u8;
            match row.get(i).copied().flatten() {
                Some(s) => Marks {
                    locks: n,
                    cond: s.cond,
                    chance: s.chance,
                    ratchet: s.ratchet,
                    nudge: s.nudge.signum() as i8,
                    tie: s.nudge + s.len > size,
                    held_over: false,
                },
                None => Marks {
                    locks: n,
                    held_over: held.get(i).copied().unwrap_or(false),
                    ..Marks::default()
                },
            }
        })
        .collect()
}

fn pct(v: f32) -> String {
    format!("{:.0} %", v * 100.0)
}

/// A send's level as the card shows it: off, or a share of full (0 dB).
fn send_pos(db: f32) -> f32 {
    if db <= mixing::SEND_OFF_DB {
        0.0
    } else {
        10f32.powf(db / 20.0).clamp(0.0, 1.0)
    }
}

fn send_db(pos: f32) -> f32 {
    if pos <= 0.005 {
        mixing::SEND_OFF_DB
    } else {
        (20.0 * pos.log10()).max(mixing::SEND_OFF_DB + 1.0)
    }
}

/// A DJ filter's position as the card writes it: "Off", "LP 8.2 kHz",
/// "HP 400 Hz" (the filter's own mapping, `mpfree_fx::DjFilter`).
pub fn filter_text(pos: f32) -> String {
    let a = (pos.abs() / 100.0).clamp(0.0, 1.0);
    if a < 0.01 {
        return "Off".into();
    }
    let (kind, hz) = if pos < 0.0 {
        ("LP", 20_000.0 * (80.0f32 / 20_000.0).powf(a))
    } else {
        ("HP", 20.0 * (8_000.0f32 / 20.0).powf(a))
    };
    format!("{kind} {}", hz_text(hz))
}

pub fn hz_text(hz: f32) -> String {
    if hz >= 10_000.0 {
        format!("{:.0} kHz", hz / 1000.0)
    } else if hz >= 1000.0 {
        format!("{:.1} kHz", hz / 1000.0)
    } else {
        format!("{hz:.0} Hz")
    }
}

fn db_text(db: f32) -> String {
    if db <= -59.5 {
        "-inf dB".into()
    } else {
        format!("{db:+.1} dB").replace("+0.0", "0.0")
    }
}

/// A step's length as a note value where it is one ("1/32"), else ticks.
pub fn length_text(t: Tick) -> String {
    match t {
        30 => "1/128".into(),
        60 => "1/64".into(),
        80 => "1/32T".into(),
        120 => "1/32".into(),
        160 => "1/16T".into(),
        240 => "1/16".into(),
        320 => "1/8T".into(),
        480 => "1/8".into(),
        720 => "1/8D".into(),
        960 => "1/4".into(),
        1920 => "1/2".into(),
        3840 => "1 bar".into(),
        t => format!("{t} ticks"),
    }
}

/// bar.beat.sixteenth, as the transport card shows it ("9.1.02").
pub fn position_text(t: &TransportInfo) -> String {
    let bar = t.meter.bar_ticks().max(1);
    let beat = t.meter.beat_ticks().max(1);
    let pos = t.position.max(0);
    let in_bar = pos % bar;
    format!(
        "{}.{}.{:02}",
        pos / bar + 1,
        in_bar / beat + 1,
        (in_bar % beat) / STEP_TICKS + 1
    )
}

impl PlayPage {
    /// The Q-Links' targets on the view shown: a held step pad's, on any
    /// view.
    pub fn knobs(&self) -> [Knob; 4] {
        if self.held.is_some() {
            return match self.held_bank() {
                HeldBank::Trig => [Knob::Cond, Knob::Ratchet, Knob::NudgeTicks, Knob::Empty],
                HeldBank::Sound => [
                    Knob::Sound(0),
                    Knob::Sound(1),
                    Knob::Sound(2),
                    Knob::Sound(3),
                ],
                HeldBank::Step => self.step_bank(),
            };
        }
        match self.view {
            PlayView::Beats if self.selected_step().is_some() => match self.bank {
                // The inspector's Trig bank, or Q-LINK on a picked step.
                HeldBank::Trig => [Knob::Cond, Knob::Ratchet, Knob::NudgeTicks, Knob::Empty],
                _ => self.step_bank(),
            },
            PlayView::All | PlayView::Beats => {
                // 16 Levels' Transpose takes Tune's place.
                let two = if self.transpose.is_some() {
                    Knob::Transpose
                } else {
                    Knob::Tune
                };
                [Knob::Swing, two, Knob::DrumsFilter, Knob::DrumsSend]
            }
            PlayView::Loop => [
                Knob::LoopLevel,
                Knob::LoopSend,
                Knob::LineLevel,
                Knob::LoopFilter,
            ],
        }
    }

    /// The Step bank: a sound's velocity, length, nudge and chance; on the
    /// synth's line the Note knob comes first and the nudge is on Trig (in
    /// ticks), as the design's synth tracks have it.
    fn step_bank(&self) -> [Knob; 4] {
        if self.on_line() {
            [Knob::Note, Knob::Velocity, Knob::Length, Knob::Chance]
        } else {
            [Knob::Velocity, Knob::Length, Knob::Nudge, Knob::Chance]
        }
    }

    /// The steps the Q-Links set are the synth's line's: held on the pads,
    /// or picked on its lane.
    pub fn on_line(&self) -> bool {
        match &self.held {
            Some((sound, _)) => *sound == LINE,
            None => self.selected.is_some_and(|(p, _)| p == usize::from(LINE)),
        }
    }

    /// The pads (lowest first) that step `i` of the line sounds.
    fn line_pads(&self, i: usize) -> Vec<u8> {
        let Some(clip) = self.steps.clip.as_ref() else {
            return Vec::new();
        };
        clip.rows
            .iter()
            .filter(|(p, row)| *p < 16 && row.get(i).copied().flatten().is_some())
            .map(|(p, _)| *p)
            .collect::<std::collections::BTreeSet<u8>>()
            .into_iter()
            .collect()
    }

    /// Step `i` of the line by its notes, as the pads name them: "C3", or
    /// a chord's "C3 + Eb3 + G3".
    pub fn line_notes(&self, i: usize) -> String {
        let names: Vec<String> = self
            .line_pads(i)
            .into_iter()
            .map(|p| self.pad_name(usize::from(p)))
            .collect();
        names.join(" + ")
    }

    /// How far the lowest note of the line's steps being set can go up the
    /// pads with every note of each staying on them: 15 less the widest
    /// chord's span.
    fn note_top(&self) -> u8 {
        let span = |i: usize| {
            self.steps
                .clip
                .as_ref()
                .and_then(|c| c.row(LINE).get(i).copied().flatten())
                .map_or(0, |s| s.high.saturating_sub(s.low))
        };
        let steps: Vec<usize> = match &self.held {
            Some((_, steps)) => steps.clone(),
            None => self.selected.map(|(_, i)| i).into_iter().collect(),
        };
        15u8.saturating_sub(steps.into_iter().map(span).max().unwrap_or(0))
    }

    /// The step the Q-Links set: the step pad held first that has a note
    /// (pad, step, what it is), else the step picked in Beats.
    pub fn target_step(&self) -> Option<(usize, usize, Step)> {
        if let Some((sound, steps)) = &self.held {
            let clip = self.steps.clip.as_ref()?;
            let row = clip.row(*sound);
            return steps
                .iter()
                .find_map(|&i| Some((usize::from(*sound), i, (*row.get(i)?)?)));
        }
        self.selected_step()
    }

    /// The bank the Q-Links show while steps are held: the one Q-LINK
    /// picked, or Sound when no held step has a note (SHIFT + an empty
    /// step: a lock with no note).
    pub fn held_bank(&self) -> HeldBank {
        if self.held.is_some() && self.held_bits().1 == 0 {
            HeldBank::Sound
        } else {
            self.bank
        }
    }

    /// Every step held, with a note or not, as a step action's bits.
    fn all_held_bits(&self) -> u128 {
        self.held.as_ref().map_or(0, |(_, steps)| {
            steps
                .iter()
                .filter(|&&i| i < 128)
                .fold(0u128, |b, &i| b | (1u128 << i))
        })
    }

    /// Sound knob `k` and its value on the step held first: its lock
    /// there, else the sound's own (and whether it is a lock).
    fn sound_value(&self, k: usize) -> Option<(&SoundKnob, f32, bool)> {
        let sk = self.sound_bank.get(k)?.as_ref()?;
        let (sound, steps) = self.held.as_ref()?;
        let lock = self
            .steps
            .clip
            .as_ref()
            .and_then(|c| c.lock_value(*sound, *steps.first()?, sk.addr));
        Some((sk, lock.unwrap_or(sk.own), lock.is_some()))
    }

    /// The steps held with a note, as a step action's bits (step 0 the
    /// lowest), and how many there are.
    fn held_bits(&self) -> (u128, u32) {
        let (Some((sound, steps)), Some(clip)) = (&self.held, self.steps.clip.as_ref()) else {
            return (0, 0);
        };
        let row = clip.row(*sound);
        let bits = steps
            .iter()
            .filter(|&&i| i < 128 && row.get(i).copied().flatten().is_some())
            .fold(0u128, |b, &i| b | (1u128 << i));
        (bits, bits.count_ones())
    }

    /// "Step 5", or "Steps 5, 9" when several are held.
    pub fn held_name(&self) -> Option<String> {
        let (_, steps) = self.held.as_ref()?;
        let list: Vec<String> = steps.iter().map(|i| (i + 1).to_string()).collect();
        Some(if list.len() == 1 {
            format!("Step {}", list[0])
        } else {
            format!("Steps {}", list.join(", "))
        })
    }

    /// Whether the step inspector shows: Beats, with steps held or a
    /// step picked.
    pub fn inspecting(&self) -> bool {
        self.view == PlayView::Beats && (self.held.is_some() || self.selected_step().is_some())
    }

    /// The step the inspector shows: the step held first, or the step
    /// picked (pad, step in the clip), and whether it is held.
    fn inspected(&self) -> Option<(usize, usize, bool)> {
        if self.view != PlayView::Beats {
            return None;
        }
        if let Some((sound, steps)) = &self.held {
            return Some((usize::from(*sound), *steps.first()?, true));
        }
        self.selected_step().map(|(pad, i, _)| (pad, i, false))
    }

    /// What the step inspector shows, if it shows.
    pub fn inspect(&self) -> Option<Inspect> {
        let (pad, i, held) = self.inspected()?;
        let name = self.pad_name(pad);
        // A step of the line by its notes ("Step 5 · C3 + G3").
        let notes = if pad == usize::from(LINE) {
            self.line_notes(i)
        } else {
            String::new()
        };
        let name = if notes.is_empty() { name } else { notes };
        let title = match (held, self.held_name()) {
            (true, Some(steps)) => format!("{steps} · {name} · held"),
            _ => format!("Step {} · {name}", i + 1),
        };
        let has_note = self.target_step().is_some();
        let chips = if has_note {
            INSPECT_CHIPS
                .iter()
                .map(|&k| {
                    let c = self.card(k, None);
                    (c.name, c.value)
                })
                .collect()
        } else {
            Vec::new()
        };
        let bank = if held { self.held_bank() } else { self.bank };
        let turned = match bank {
            // On the line, Note (in the title) takes the nudge's Q-Link.
            HeldBank::Step if pad == usize::from(LINE) => [true, true, false, true, false, false],
            HeldBank::Step => [true, true, true, true, false, false],
            HeldBank::Trig => [false, false, true, false, true, true],
            HeldBank::Sound => [false; 6],
        };
        let locks = self
            .steps
            .clip
            .as_ref()
            .map(|c| {
                c.lock_values
                    .iter()
                    .filter(|l| (usize::from(l.0), l.1) == (pad, i))
                    .map(|&(_, _, addr, v)| {
                        let (n, text) = self.lock_text(addr, v);
                        (addr, n, text)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let one = self.held.as_ref().is_none_or(|(_, steps)| steps.len() == 1);
        Some(Inspect {
            title,
            bank,
            sound: held,
            chips,
            turned,
            locks,
            one: one && has_note,
            copying: self.copy_from == Some((pad, i)),
        })
    }

    /// A lock's parameter and value as the inspector names them: a kit
    /// pad's by its field, an instrument's by the Sound bank's name when
    /// it is there.
    fn lock_text(&self, addr: RelAddr, v: f32) -> (String, String) {
        use mpfree_play::mpfree_param::OnTrack;
        use mpfree_sampler::engine::PadField;
        if let Some(sk) = self.sound_bank.iter().flatten().find(|k| k.addr == addr) {
            return (sk.name.clone(), sk.kind.read(v).0);
        }
        let named = |n: &str, kind: SoundKind| (n.to_string(), kind.read(v).0);
        match addr.on {
            OnTrack::Pad(_) => match addr.param {
                p if p == PadField::Tune as u16 => named("Tune", SoundKind::Tune),
                p if p == PadField::Decay as u16 => named("Decay", SoundKind::Decay),
                p if p == PadField::Cutoff as u16 => named("Filter", SoundKind::Cutoff),
                p if p == PadField::Level as u16 => named("Level", SoundKind::Level),
                p if p == PadField::Pan as u16 => ("Pan".to_string(), format!("{v:+.2}")),
                p if p == PadField::Resonance as u16 => {
                    ("Resonance".to_string(), format!("{:.0} %", v * 100.0))
                }
                p if p == PadField::Attack as u16 => ("Attack".to_string(), format!("{v:.0} ms")),
                p if p == PadField::Release as u16 => ("Release".to_string(), format!("{v:.0} ms")),
                // Slices count from 1 on the screen, from 0 in the engine.
                p if p == PadField::Slice as u16 => (
                    "Slice".to_string(),
                    format!("{}", v.round().max(0.0) as i64 + 1),
                ),
                p => (format!("Pad setting {p}"), format!("{v:.2}")),
            },
            OnTrack::Inst => (format!("Synth {}", addr.param), format!("{v:.2}")),
            _ => (format!("{addr}"), format!("{v:.2}")),
        }
    }

    /// A tap on the step inspector: a bank picked (Q-LINK's), a lock
    /// taken off the steps (every step held), Copy (the next step tapped
    /// on the lane takes a copy) or Clear. The step action, if any.
    pub fn inspector_tap(&mut self, tap: InspectTap) -> Option<StepAction> {
        let (pad, i, held) = self.inspected()?;
        let steps = if held {
            self.all_held_bits()
        } else {
            1u128.checked_shl(i as u32)?
        };
        match tap {
            InspectTap::Bank(HeldBank::Sound) if !held => None,
            InspectTap::Bank(b) => {
                self.bank = b;
                None
            }
            InspectTap::Unlock(k) => {
                let (addr, _, _) = self.inspect()?.locks.get(k).cloned()?;
                let a = StepAction::Lock {
                    pad: pad as u8,
                    steps,
                    addr,
                    value: None,
                };
                if let Some(c) = self.steps.clip.as_mut() {
                    c.lock_values.retain(|l| {
                        !(usize::from(l.0) == pad
                            && steps & (1u128 << l.1.min(127)) != 0
                            && l.2 == addr)
                    });
                }
                Some(a)
            }
            InspectTap::Copy if self.inspect()?.one => {
                self.copy_from = Some((pad, i));
                None
            }
            InspectTap::Clear if self.inspect()?.one => {
                self.copy_from = None;
                if !held {
                    self.selected = None;
                }
                Some(StepAction::ClearSteps {
                    pad: pad as u8,
                    from: i,
                    to: i + 1,
                })
            }
            InspectTap::Close if !held => {
                self.selected = None;
                self.copy_from = None;
                self.bank = HeldBank::Step;
                None
            }
            InspectTap::Copy | InspectTap::Clear | InspectTap::Close => None,
        }
    }

    /// The step picked, if it still holds notes.
    pub fn selected_step(&self) -> Option<(usize, usize, Step)> {
        let (pad, i) = self.selected?;
        let clip = self.steps.clip.as_ref()?;
        let s = (*clip.row(pad as u8).get(i)?)?;
        Some((pad, i, s))
    }

    fn pad_name(&self, pad: usize) -> String {
        if pad == usize::from(LINE) {
            // The synth's steps on the pads: the whole line.
            return "Line".to_string();
        }
        self.pads
            .get(pad)
            .and_then(|p| p.label.clone())
            .unwrap_or_else(|| format!("Pad {}", pad + 1))
    }

    /// "PAD 5 · HAT".
    fn pad_scope(&self, pad: usize) -> String {
        format!("Pad {} · {}", pad + 1, self.pad_name(pad))
    }

    fn ms_of_ticks(&self, ticks: f64) -> f32 {
        (ticks * 60_000.0 / (self.transport.bpm.max(1.0) * f64::from(PPQ))) as f32
    }

    /// The card for `knob`. `pad` is the pad's tuning (semitones) and
    /// decay (None: the sound plays out) from the Sample page, when the
    /// kit has the pad.
    pub fn card(&self, knob: Knob, pad: Option<(f32, Option<f32>)>) -> QLinkCard {
        let l = &self.levels;
        let mut c = match knob {
            Knob::Swing => {
                // As the transport bar says it: 50 % is straight, so "off".
                let a = self.transport.swing.amount;
                let text = if a == 50 {
                    "off".to_string()
                } else {
                    format!("{a} %")
                };
                QLinkCard::new(
                    "Swing",
                    "Transport",
                    &text,
                    f32::from(a.saturating_sub(50)) / 25.0,
                )
            }
            Knob::Tune => match pad {
                Some((st, _)) => {
                    let mut c = QLinkCard::new(
                        "Tune",
                        &self.pad_scope(self.last_pad),
                        &format!("{st:+.0} st").replace("+0 st", "0 st"),
                        ((st + 24.0) / 48.0).clamp(0.0, 1.0),
                    );
                    c.bipolar = true;
                    c
                }
                None => {
                    // No sound on the pad to tune: an empty ring.
                    let mut c = QLinkCard::new("Tune", &self.pad_scope(self.last_pad), "-", 0.5);
                    c.bipolar = true;
                    c
                }
            },
            Knob::Transpose => {
                let (scope, text, by) = match &self.transpose {
                    Some(t) => {
                        let text = mpfree_time_ui::view::transpose_text(mpfree_seq::Transpose {
                            by: t.by,
                            in_key: t.in_key,
                        });
                        (t.track.clone(), text, t.by)
                    }
                    None => (String::new(), "-".to_string(), 0),
                };
                let max = f32::from(mpfree_seq::MAX_TRANSPOSE);
                let mut c = QLinkCard::new(
                    "Transpose",
                    &scope,
                    &text,
                    ((f32::from(by) + max) / (2.0 * max)).clamp(0.0, 1.0),
                );
                c.bipolar = true;
                c
            }
            Knob::DrumsFilter | Knob::LoopFilter => {
                let (v, scope) = if knob == Knob::DrumsFilter {
                    (l.drums_filter, "Drums")
                } else {
                    (l.loop_filter, "Looper")
                };
                let mut c = QLinkCard::new("Filter", scope, &filter_text(v), (v + 100.0) / 200.0);
                c.bipolar = true;
                c
            }
            Knob::DrumsSend | Knob::LoopSend => {
                let (db, scope) = if knob == Knob::DrumsSend {
                    (l.drums_send_db, "Drums")
                } else {
                    (l.loop_send_db, "Looper")
                };
                let p = send_pos(db);
                QLinkCard::new("Delay send", scope, &pct(p), p)
            }
            Knob::LoopLevel => QLinkCard::new(
                "Loop level",
                "Looper",
                &db_text(l.loop_db),
                ((l.loop_db + 60.0) / 66.0).clamp(0.0, 1.0),
            ),
            Knob::LineLevel => QLinkCard::new(
                "Line level",
                "Line in",
                &db_text(l.line_db),
                ((l.line_db + 60.0) / 66.0).clamp(0.0, 1.0),
            ),
            Knob::Empty => return QLinkCard::default(),
            Knob::Sound(k) => {
                let Some((sk, v, locked)) = self.sound_value(usize::from(k)) else {
                    return QLinkCard::default();
                };
                let (text, pos, bipolar) = sk.kind.read(v);
                let sound = self.held.as_ref().map_or(0, |h| usize::from(h.0));
                let scope = format!(
                    "{} · {}",
                    self.held_name().unwrap_or_default(),
                    if locked {
                        "lock".to_string()
                    } else {
                        self.pad_name(sound)
                    }
                );
                let mut c = QLinkCard::new(&sk.name, &scope, &text, pos);
                c.bipolar = bipolar;
                if locked {
                    c.color = Some(LOCKED);
                }
                c
            }
            Knob::Velocity
            | Knob::Length
            | Knob::Nudge
            | Knob::Chance
            | Knob::Cond
            | Knob::Ratchet
            | Knob::NudgeTicks
            | Knob::Note => {
                let Some((pad, i, s)) = self.target_step() else {
                    return QLinkCard::default();
                };
                let scope = match self.held_name() {
                    Some(name) => format!("{name} · {}", self.pad_name(pad)),
                    None => format!("Step {} · {}", i + 1, self.pad_name(pad)),
                };
                match knob {
                    Knob::Note => {
                        // The lowest note; a chord says so in its name.
                        let name = if s.notes > 1 { "Chord" } else { "Note" };
                        QLinkCard::new(
                            name,
                            &scope,
                            &self.pad_name(usize::from(s.low)),
                            f32::from(s.low) / f32::from(self.note_top().max(1)),
                        )
                    }
                    Knob::Cond => {
                        let all = Cond::all();
                        let at = all.iter().position(|&c| c == s.cond).unwrap_or(0);
                        let text = if s.cond == Cond::Always {
                            "always".to_string()
                        } else {
                            s.cond.to_string()
                        };
                        QLinkCard::new(
                            "Condition",
                            &scope,
                            &text,
                            at as f32 / (all.len() - 1) as f32,
                        )
                    }
                    Knob::Ratchet => QLinkCard::new(
                        "Ratchet",
                        &scope,
                        &if s.ratchet <= 1 {
                            "off".to_string()
                        } else {
                            format!("{}×", s.ratchet)
                        },
                        f32::from(s.ratchet.saturating_sub(1))
                            / f32::from(mpfree_seq::MAX_RATCHET - 1),
                    ),
                    Knob::NudgeTicks => {
                        let mut c = QLinkCard::new(
                            "Nudge",
                            &scope,
                            &format!("{:+} ticks", s.nudge).replace("+0 ticks", "0 ticks"),
                            (0.5 + s.nudge as f32 / STEP_TICKS as f32).clamp(0.0, 1.0),
                        );
                        c.bipolar = true;
                        c
                    }
                    Knob::Velocity => QLinkCard::new(
                        "Velocity",
                        &scope,
                        &s.velocity.to_string(),
                        f32::from(s.velocity) / 127.0,
                    ),
                    Knob::Length => {
                        let k = LENGTHS
                            .iter()
                            .position(|&x| x >= s.len)
                            .unwrap_or(LENGTHS.len() - 1);
                        QLinkCard::new(
                            "Length",
                            &scope,
                            &length_text(s.len),
                            k as f32 / (LENGTHS.len() - 1) as f32,
                        )
                    }
                    Knob::Nudge => {
                        let ms = self.ms_of_ticks(s.nudge as f64);
                        let mut c = QLinkCard::new(
                            "Nudge",
                            &scope,
                            &format!("{ms:+.0} ms").replace("+0 ms", "0 ms"),
                            (0.5 + s.nudge as f32 / STEP_TICKS as f32).clamp(0.0, 1.0),
                        );
                        c.bipolar = true;
                        c
                    }
                    _ => QLinkCard::new(
                        "Chance",
                        &scope,
                        &format!("{} %", s.chance),
                        f32::from(s.chance) / 100.0,
                    ),
                }
            }
        };
        c.scope = c.scope.to_uppercase();
        c
    }

    /// Q-Link `k` turned by `ticks` detents (`fine`: SHIFT held). The
    /// screen's values move at once; the engines hear the returned turn.
    pub fn turn(&mut self, k: usize, ticks: i32, fine: bool) -> Option<Turn> {
        let knob = *self.knobs().get(k)?;
        self.turn_knob(knob, ticks, fine)
    }

    /// Turn `knob` by `ticks` detents, whichever view shows (the Looper
    /// tab turns the line level with it).
    pub fn turn_knob(&mut self, knob: Knob, ticks: i32, fine: bool) -> Option<Turn> {
        if ticks == 0 {
            return None;
        }
        self.turned_at = Some(Instant::now());
        let d = ticks as f32;
        let l = &mut self.levels;
        Some(match knob {
            Knob::Swing => {
                let a = (i32::from(self.transport.swing.amount) + ticks).clamp(50, 75) as u8;
                self.transport.swing.amount = a;
                Turn::Swing(a)
            }
            Knob::Tune => Turn::Pad(knob, ticks),
            Knob::Transpose => {
                let t = self.transpose.as_mut()?;
                let max = i32::from(mpfree_seq::MAX_TRANSPOSE);
                let by = (i32::from(t.by) + ticks).clamp(-max, max) as i8;
                if by == t.by {
                    return None;
                }
                t.by = by;
                Turn::Transpose(by)
            }
            Knob::DrumsFilter | Knob::LoopFilter => {
                let (v, strip) = if knob == Knob::DrumsFilter {
                    (&mut l.drums_filter, StripId::Track(mixing::DRUMS))
                } else {
                    (&mut l.loop_filter, StripId::Track(mixing::LOOP))
                };
                *v = (*v + d * if fine { 0.5 } else { 2.0 }).clamp(-100.0, 100.0);
                Turn::Fx(PlayFx::Filter {
                    strip,
                    position: *v,
                })
            }
            Knob::DrumsSend | Knob::LoopSend => {
                let (db, strip) = if knob == Knob::DrumsSend {
                    (&mut l.drums_send_db, StripId::Track(mixing::DRUMS))
                } else {
                    (&mut l.loop_send_db, StripId::Track(mixing::LOOP))
                };
                let p = (send_pos(*db) + d * if fine { 0.005 } else { 0.02 }).clamp(0.0, 1.0);
                *db = send_db(p);
                Turn::Fx(PlayFx::Strip {
                    strip,
                    id: mixer_ids::send(mixing::DELAY_RETURN),
                    value: *db,
                })
            }
            Knob::LoopLevel | Knob::LineLevel => {
                let (db, strip) = if knob == Knob::LoopLevel {
                    (&mut l.loop_db, StripId::Track(mixing::LOOP))
                } else {
                    (&mut l.line_db, mixing::LINE)
                };
                *db = (*db + d * if fine { 0.1 } else { 0.5 }).clamp(-60.0, 6.0);
                Turn::Fx(PlayFx::Strip {
                    strip,
                    id: mixer_ids::VOLUME,
                    value: *db,
                })
            }
            Knob::Empty => return None,
            Knob::Sound(k) => {
                // Every step held takes the lock, with a note or not.
                let (sk, v, _) = self.sound_value(usize::from(k))?;
                let (addr, value) = (sk.addr, sk.kind.turn(v, ticks, fine));
                let pad = self.held.as_ref()?.0;
                let a = StepAction::Lock {
                    pad,
                    steps: self.all_held_bits(),
                    addr,
                    value: Some(value),
                };
                self.apply_locally(a);
                Turn::Step(a)
            }
            Knob::Velocity
            | Knob::Length
            | Knob::Nudge
            | Knob::Chance
            | Knob::Cond
            | Knob::Ratchet
            | Knob::NudgeTicks
            | Knob::Note => {
                let (pad, i, s) = self.target_step()?;
                let change = match knob {
                    // A pad a detent, every note of every step set staying
                    // on the pads.
                    Knob::Note => {
                        let to = (i32::from(s.low) + ticks.signum())
                            .clamp(0, i32::from(self.note_top()))
                            as u8;
                        if to == s.low {
                            return None;
                        }
                        Change::Note(to)
                    }
                    Knob::Cond => Change::Cond(s.cond.step(ticks.signum())),
                    Knob::Ratchet => Change::Ratchet(
                        (i32::from(s.ratchet) + ticks.signum())
                            .clamp(1, i32::from(mpfree_seq::MAX_RATCHET))
                            as u8,
                    ),
                    Knob::NudgeTicks => {
                        Change::Nudge(s.nudge + Tick::from(ticks) * if fine { 1 } else { 2 })
                    }
                    Knob::Velocity => Change::Velocity(
                        (i32::from(s.velocity) + ticks * if fine { 1 } else { 2 }).clamp(1, 127)
                            as u8,
                    ),
                    Knob::Length => {
                        let k = LENGTHS
                            .iter()
                            .position(|&x| x >= s.len)
                            .unwrap_or(LENGTHS.len() - 1) as i32;
                        Change::Length(
                            LENGTHS
                                [(k + ticks.signum()).clamp(0, LENGTHS.len() as i32 - 1) as usize],
                        )
                    }
                    Knob::Nudge => {
                        Change::Nudge(s.nudge + Tick::from(ticks) * if fine { 2 } else { 10 })
                    }
                    _ => Change::Chance(
                        (i32::from(s.chance) + ticks * if fine { 1 } else { 5 }).clamp(1, 100)
                            as u8,
                    ),
                };
                // Several steps held: they all take it, as one undo.
                let (bits, n) = self.held_bits();
                let a = if n > 1 {
                    StepAction::ChangeSteps {
                        pad: pad as u8,
                        steps: bits,
                        change,
                    }
                } else {
                    StepAction::Change {
                        pad: pad as u8,
                        step: i,
                        change,
                    }
                };
                self.apply_locally(a);
                Turn::Step(a)
            }
        })
    }

    /// A step change shows at once; the engine's steps follow.
    fn apply_locally(&mut self, a: StepAction) {
        if let StepAction::Lock {
            pad,
            steps,
            addr,
            value: Some(v),
        } = a
        {
            if let Some(clip) = self.steps.clip.as_mut() {
                for step in (0..128).filter(|i| steps & (1u128 << i) != 0) {
                    match clip
                        .lock_values
                        .iter_mut()
                        .find(|l| (l.0, l.1, l.2) == (pad, step, addr))
                    {
                        Some(l) => l.3 = v,
                        None => clip.lock_values.push((pad, step, addr, v)),
                    }
                }
            }
            return;
        }
        let (pad, steps, change) = match a {
            StepAction::Change { pad, step, change } if step < 128 => (pad, 1u128 << step, change),
            StepAction::ChangeSteps { pad, steps, change } => (pad, steps, change),
            _ => return,
        };
        let Some(clip) = self.steps.clip.as_mut() else {
            return;
        };
        let Some((_, row)) = clip.rows.iter_mut().find(|(p, _)| *p == pad) else {
            return;
        };
        for step in (0..128).filter(|i| steps & (1u128 << i) != 0) {
            let Some(Some(s)) = row.get_mut(step) else {
                continue;
            };
            match change {
                Change::Velocity(v) => s.velocity = v,
                Change::Length(t) => s.len = t,
                Change::Nudge(n) => {
                    let half = STEP_TICKS / 2;
                    s.nudge = n.clamp(-half, half - 1);
                }
                Change::Chance(c) => s.chance = c,
                Change::Cond(c) if c.ok() => s.cond = c,
                Change::Cond(_) => {}
                Change::Ratchet(r) => s.ratchet = r.clamp(1, mpfree_seq::MAX_RATCHET),
                Change::Note(to) => {
                    s.high = to.saturating_add(s.high - s.low);
                    s.low = to;
                }
            }
        }
    }

    /// The engine's values for the Q-Links and the speakers.
    pub fn set_levels(&mut self, l: PlayLevels) {
        let fresh = self.turned_at.is_none_or(|t| t.elapsed() >= HOLD_TURNED);
        if fresh {
            self.levels = l;
        } else {
            self.levels.pad_mutes = l.pad_mutes;
        }
    }

    /// The transport: its swing too, unless a knob is turning it.
    pub fn set_transport(&mut self, t: TransportInfo) {
        let keep = self.turned_at.is_some_and(|at| at.elapsed() < HOLD_TURNED);
        let swing = self.transport.swing;
        self.transport = t;
        if keep {
            self.transport.swing = swing;
        }
        // Hits older than the window go.
        let window = (TIMING_BARS * t.meter.bar_ticks()) as f64;
        let now = t.position as f64;
        while self
            .hits
            .front()
            .is_some_and(|h| h.tick < now - window || h.tick > now + window)
        {
            self.hits.pop_front();
        }
    }

    pub fn set_steps(&mut self, s: StepsData) {
        self.steps = s;
        let n = self.steps.clip.as_ref().map_or(16, |c| c.steps);
        let pages = n.div_ceil(PAGE).max(1);
        self.page = match self.on_pads {
            // A clip grown to reach the pads' page shows it.
            Some(p) => p.page.min(pages - 1),
            None => self.page.min(pages - 1),
        };
    }

    pub fn hit(&mut self, h: HitInfo) {
        if usize::from(h.pad) < 16 {
            // The pad hit last is the pad's own news (ToUi::PadDown).
            self.hits.push_back(h);
            while self.hits.len() > 256 {
                self.hits.pop_front();
            }
        }
    }

    /// The step playing on the page shown, and the page it is on.
    fn playhead(&self) -> Option<(usize, usize)> {
        let clip = self.steps.clip.as_ref()?;
        if !self.transport.state.rolling() {
            return None;
        }
        let s = clip.playing(self.transport.position)?;
        Some((s / PAGE, s % PAGE))
    }

    /// The synth's pads hold notes (not the Drums engine's sounds): Beats
    /// shows its pattern as scale rows.
    pub fn pitched(&self) -> bool {
        self.pads.iter().any(|p| p.pitch.is_some())
    }

    fn has_notes(&self, p: usize) -> bool {
        self.steps
            .clip
            .as_ref()
            .is_some_and(|c| c.row(p as u8).iter().any(Option::is_some))
    }

    fn note_of(&self, p: usize) -> Option<u8> {
        self.pads.get(p).and_then(|i| i.pitch).map(|x| x.note)
    }

    /// Scale rows: the pads in pitch order, lowest first (any with notes
    /// but no note of their own below them); with Fold, only those the
    /// clip uses, unless it uses none.
    pub fn note_rows(&self) -> Vec<usize> {
        let mut v: Vec<usize> = (0..16)
            .filter(|&p| self.note_of(p).is_some() || self.has_notes(p))
            .collect();
        v.sort_by_key(|&p| (self.note_of(p).map_or(-1, i16::from), p));
        if self.fold && v.iter().any(|&p| self.has_notes(p)) {
            v.retain(|&p| self.has_notes(p));
        }
        v
    }

    /// Scale rows: the window's lowest row, as the octave's ‹ and › left
    /// it, else from the root at or under the lowest note used (within an
    /// octave), else the lowest root.
    pub fn low_row(&self, rows: &[usize]) -> usize {
        if let Some(low) = self.low {
            return low;
        }
        let note = |k: usize| self.note_of(rows[k]).map_or(0, i32::from);
        let root = |k: usize| {
            self.pads
                .get(rows[k])
                .and_then(|i| i.pitch)
                .is_some_and(|x| x.root)
        };
        match rows.iter().position(|&p| self.has_notes(p)) {
            Some(k) => (0..=k)
                .rev()
                .find(|&j| root(j) && note(j) + 12 > note(k))
                .unwrap_or(k),
            None => (0..rows.len()).find(|&j| root(j)).unwrap_or(0),
        }
    }

    /// The octave's ‹ (`up` false) or ›, with `n` note rows showing: the
    /// window moves to the row an octave down or up, stopping at the ends.
    /// False when it did not move.
    pub fn turn_octave(&mut self, up: bool, n: usize) -> bool {
        let rows = self.note_rows();
        let top = rows.len().saturating_sub(n.max(1));
        let low = self.low_row(&rows).min(top);
        let note = |k: usize| self.note_of(rows[k]).map_or(0, i32::from);
        let to = if up {
            (low + 1..=top)
                .find(|&j| note(j) >= note(low) + 12)
                .unwrap_or(top)
        } else {
            (0..low)
                .rev()
                .find(|&j| note(j) + 12 <= note(low))
                .unwrap_or(0)
        };
        self.low = Some(to);
        to != low
    }

    /// Fold on or off: the window starts again from the notes.
    pub fn toggle_fold(&mut self) {
        self.fold = !self.fold;
        self.low = None;
    }

    /// The pattern's lanes, most wanted first: the sounds with notes, the
    /// The pads of the pattern's lanes, most wanted first ([`Self::lanes`]).
    #[cfg(test)]
    pub fn lane_pads(&self) -> Vec<usize> {
        self.lanes().iter().map(|l| l.pad).collect()
    }

    /// pad hit last, then the kit's other sounds, then the rest. On a
    /// synth track, the line (STEP SEQ's) and then the scale rows, lowest
    /// first (the grid shows them high on top).
    fn lanes(&self) -> Vec<Lane> {
        let has_notes = |p: usize| self.has_notes(p);
        let has_sound = |p: usize| self.pads.get(p).is_some_and(|i| i.label.is_some());
        // The lane on the pads comes first, so it always shows.
        let on_pads = self.on_pads.map(|s| usize::from(s.sound));
        let mut wanted: Vec<usize> = on_pads.into_iter().collect();
        if self.pitched() {
            wanted.extend(self.note_rows());
            return wanted.into_iter().map(|p| self.lane(p)).collect();
        }
        // Any of a kit's 128 pads (the synth's 16); the empty ones of the
        // page on the pads last.
        let all = 0..self.pads.len().max(16);
        let page =
            (self.pad_page * 16..self.pad_page * 16 + 16).filter(|&p| p < self.pads.len().max(16));
        wanted.extend(all.clone().filter(|&p| has_notes(p)));
        if has_sound(self.last_pad) {
            wanted.push(self.last_pad);
        }
        wanted.extend(all.filter(|&p| has_sound(p) && !has_notes(p) && p != self.last_pad));
        wanted.extend(page.filter(|&p| !has_sound(p) && !has_notes(p) && p != self.last_pad));
        let mut order: Vec<usize> = Vec::with_capacity(16);
        for p in wanted {
            if !order.contains(&p) {
                order.push(p);
            }
        }
        order.into_iter().map(|p| self.lane(p)).collect()
    }

    /// Pad `p`'s lane on the page shown.
    fn lane(&self, p: usize) -> Lane {
        let clip = self.steps.clip.as_ref();
        let from = self.page * PAGE;
        let row = clip.map_or(&[][..], |c| c.row(p as u8));
        let locks = clip.map_or(&[][..], |c| c.locks(p as u8));
        let pitch = self.pads.get(p).and_then(|i| i.pitch);
        Lane {
            pad: p,
            name: self.pad_name(p),
            // The line (the synth's steps) takes its pen's colour.
            color: self
                .pads
                .get(p)
                .or_else(|| {
                    let pen = self.on_pads.filter(|_| p == usize::from(LINE))?.pen;
                    self.pads.get(usize::from(*pen.pads().first()?))
                })
                .and_then(PadInfo::color)
                .unwrap_or(draw::FX),
            // A synth's pads have no mutes of their own (the kit's are
            // the drums'), nor have another kit's (they share their
            // track's channel).
            muted: p < 16
                && !self.pitched()
                && self.pad_strips
                && self.levels.pad_mutes & (1 << p) != 0,
            steps: (from..from + PAGE)
                .map(|i| row.get(i).copied().flatten().map_or(0, |s| s.velocity))
                .collect(),
            marks: marks(row, locks, from),
            note: pitch.map(|x| x.note),
            root: pitch.is_some_and(|x| x.root),
        }
    }

    /// Fill the pattern panel.
    pub fn fill_steps(&self, g: &mut StepGrid) {
        let clip = self.steps.clip.as_ref();
        let first = clip.map_or("Pattern", |c| c.name.as_str());
        let title = if self.steps.track_name.is_empty() {
            first.to_string()
        } else {
            format!("{first} · {}", self.steps.track_name)
        };
        let bars = clip.map_or(1, |c| c.steps.div_ceil(16).max(1));
        let pages = clip.map_or(1, |c| c.steps.div_ceil(PAGE).max(1));
        let mut right = format!("1/16 · {bars} bar{}", if bars == 1 { "" } else { "s" });
        if let Some(s) = self.on_pads {
            // STEP SEQ: which page the pads are, and its PAD BANK.
            let letter = (b'A' + (s.page % 4) as u8) as char;
            right.push_str(&format!(" · pads: page {} · BANK {letter}", s.page + 1));
        } else if clip.is_none() {
            right.push_str(" · tap a step to start one");
        } else if self.view == PlayView::Beats {
            right.push_str(" · tap a step to edit it");
        }
        let steps = clip.map_or(PAGE, |c| c.steps.saturating_sub(self.page * PAGE).min(PAGE));
        let play = self
            .playhead()
            .filter(|(p, _)| *p == self.page)
            .map(|(_, s)| s);
        let selected = match self.view {
            PlayView::Beats => self
                .selected
                .filter(|(_, i)| i / PAGE == self.page)
                .map(|(p, i)| (p, i % PAGE)),
            _ => None,
        };
        let lanes = self.lanes();
        if g.title != title {
            g.title = title;
        }
        if g.right != right {
            g.right = right;
        }
        if g.lanes != lanes {
            g.lanes = lanes;
        }
        if g.steps != steps {
            g.steps = steps;
        }
        if g.play != play {
            g.play = play;
        }
        if g.selected != selected {
            g.selected = selected;
        }
        // The step pads held, on the page shown: outlined too.
        let held: Vec<(usize, usize)> = self
            .held
            .as_ref()
            .map(|(sound, steps)| {
                steps
                    .iter()
                    .filter(|i| *i / PAGE == self.page)
                    .map(|i| (usize::from(*sound), i % PAGE))
                    .collect()
            })
            .unwrap_or_default();
        if g.held != held {
            g.held = held;
        }
        let length = clip.map_or(0, |c| c.steps);
        if (g.page, g.pages, g.length) != (self.page, pages, length) {
            (g.page, g.pages, g.length) = (self.page, pages, length);
        }
        let on_pads = self.on_pads.map(|s| usize::from(s.sound));
        if g.on_pads != on_pads {
            g.on_pads = on_pads;
        }
        let pitched = self.pitched().then(|| crate::play_widgets::Pitched {
            fold: self.fold,
            low: self.low_row(&self.note_rows()),
        });
        if g.pitched != pitched {
            g.pitched = pitched;
        }
    }

    /// STEP SEQ turned on or off, or its sound or page changed: Beats
    /// shows the page the pads are, as far as the clip goes. True when
    /// the page shown moved.
    pub fn set_on_pads(&mut self, s: Option<crate::app::StepPads>) -> bool {
        self.on_pads = s;
        let Some(s) = s else { return false };
        let n = self.steps.clip.as_ref().map_or(PAGE, |c| c.steps);
        let to = s.page.min(n.div_ceil(PAGE).max(1) - 1);
        let moved = to != self.page;
        if moved {
            self.page = to;
            self.selected = None;
        }
        moved
    }

    /// The pattern head's ‹ or › (`delta` -1 or 1): the page before or
    /// after, stopping at the ends. False when it did not move.
    pub fn turn_page(&mut self, delta: i32) -> bool {
        let n = self.steps.clip.as_ref().map_or(PAGE, |c| c.steps);
        let last = n.div_ceil(PAGE).max(1) - 1;
        let to = (self.page as i64 + delta as i64).clamp(0, last as i64) as usize;
        let moved = to != self.page;
        self.page = to;
        if moved {
            // The step picked was on the page left.
            self.selected = None;
        }
        moved
    }

    /// The pattern head's - or + (`longer`): half or twice the steps, from
    /// 16 to 128 (Jay after live test 3). Twice repeats the steps there are
    /// over the new ones, so the beat plays on as it was; half takes the
    /// notes on the steps that go (UNDO brings them back). The error says
    /// why nothing changes.
    pub fn length_action(&self, longer: bool) -> Result<StepAction, String> {
        use crate::play_widgets::{MAX_STEPS, MIN_STEPS};
        let Some(c) = self.steps.clip.as_ref() else {
            return Err("steps: tap a step to start a pattern first".into());
        };
        let n = c.steps;
        if longer {
            if n >= MAX_STEPS {
                return Err(format!("steps: {MAX_STEPS} steps is the most"));
            }
            Ok(StepAction::Length {
                steps: (n * 2).min(MAX_STEPS),
                copy: true,
            })
        } else {
            if n <= MIN_STEPS {
                return Err(format!("steps: {MIN_STEPS} steps is the fewest here"));
            }
            Ok(StepAction::Length {
                steps: (n / 2).max(MIN_STEPS),
                copy: false,
            })
        }
    }

    /// A tap on step `col` of the page shown, on `pad`'s lane. In Beats a
    /// tap on a step with notes picks it (for the Q-Links), a second tap
    /// clears it; elsewhere a tap turns it on or off.
    pub fn tap_step(&mut self, pad: usize, col: usize) -> Option<StepAction> {
        let i = self.page * PAGE + col;
        if pad == usize::from(LINE) {
            return self.tap_line(i);
        }
        // The inspector's Copy: a step on the lane takes the copy; any
        // other tap lets it go.
        if let Some((from_pad, from)) = self.copy_from.take() {
            if from_pad == pad && from != i {
                return Some(StepAction::CopySteps {
                    pad: pad as u8,
                    from,
                    to: i,
                    count: 1,
                });
            }
            return None;
        }
        let on = self
            .steps
            .clip
            .as_ref()
            .and_then(|c| c.row(pad as u8).get(i).copied().flatten())
            .is_some();
        if self.view == PlayView::Beats && on && self.selected != Some((pad, i)) {
            self.selected = Some((pad, i));
            if self.held.is_none() {
                self.bank = HeldBank::Step;
            }
            return None;
        }
        if self.selected == Some((pad, i)) {
            self.selected = None;
        }
        Some(StepAction::Tap {
            pad: pad as u8,
            step: i,
            velocity: TAP_VELOCITY,
        })
    }

    /// A tap on step `i` of the line's lane (STEP SEQ on the synth): on
    /// an empty step the pen goes on; on a lit one, in Beats, the first
    /// tap picks it and a second takes it off, as on a sound's lane.
    fn tap_line(&mut self, i: usize) -> Option<StepAction> {
        let line = usize::from(LINE);
        if let Some((from_pad, from)) = self.copy_from.take() {
            return (from_pad == line && from != i).then_some(StepAction::CopySteps {
                pad: LINE,
                from,
                to: i,
                count: 1,
            });
        }
        let on = self
            .steps
            .clip
            .as_ref()
            .and_then(|c| c.row(LINE).get(i).copied().flatten())
            .is_some();
        if on {
            if self.view == PlayView::Beats && self.selected != Some((line, i)) {
                self.selected = Some((line, i));
                if self.held.is_none() {
                    self.bank = HeldBank::Step;
                }
                return None;
            }
            if self.selected == Some((line, i)) {
                self.selected = None;
            }
            return Some(StepAction::ClearSteps {
                pad: LINE,
                from: i,
                to: i + 1,
            });
        }
        let pen = self.on_pads.filter(|s| s.line())?.pen;
        let (keys, n) = pen.keys(TAP_VELOCITY);
        (n > 0).then_some(StepAction::Enter { step: i, keys, n })
    }

    /// The throws' panel: the list, lit while held.
    pub fn fill_throws(&self, t: &mut Throws) {
        let items: Vec<Throw> = THROW_LIST
            .iter()
            .zip(self.throws)
            .map(|(d, on)| Throw {
                name: d.name.into(),
                sub: d.sub.into(),
                color: d.color,
                on,
            })
            .collect();
        if t.items != items {
            t.items = items;
        }
        if t.right != "hold to play" {
            t.right = "hold to play".into();
        }
    }

    /// The drum pads Drop drums mutes: the kicks, snares and hats.
    pub fn drum_pads(&self) -> u16 {
        let mut mask = 0u16;
        for (p, info) in self.pads.iter().enumerate().take(16) {
            let c = info.label.as_deref().and_then(draw::sound_color);
            if matches!(c, Some(c) if c == draw::KICK || c == draw::SNARE || c == draw::HAT) {
                mask |= 1 << p;
            }
        }
        mask
    }

    /// Throw `i` held or let go.
    pub fn throw(&mut self, i: usize, down: bool) -> Option<PlayFx> {
        let def = THROW_LIST.get(i)?;
        if self.throws[i] == down {
            return None;
        }
        self.throws[i] = down;
        Some(match def.fx {
            Some(fx) => PlayFx::Throw { fx, down },
            None => PlayFx::Drop(if down { self.drum_pads() } else { 0 }),
        })
    }

    /// The XY pad moved, or let go (it then glides home: [`PlayPage::tick_xy`]).
    pub fn xy(&mut self, x: f32, y: f32, down: bool) -> PlayFx {
        if down {
            self.xy.touch(x, y);
        } else {
            self.xy.release();
        }
        let (x, y) = self.xy.position();
        PlayFx::Xy { x, y, down }
    }

    /// The XY pad's glide home, `ms` on: where it is now, while it moves.
    pub fn tick_xy(&mut self, ms: f32) -> Option<PlayFx> {
        self.xy.tick(ms).then(|| {
            let (x, y) = self.xy.position();
            PlayFx::Xy { x, y, down: false }
        })
    }

    /// The XY pad's label ("LP 1.2 kHz · echo 40 %") and, with no finger
    /// on it, where its dot is.
    pub fn fill_xy(&self, p: &mut XyPad) {
        let [(_, dj), (_, echo)] = self.xy.values();
        let label = format!("{} · echo {echo:.0} %", filter_text(dj));
        if p.label != label {
            p.label = label;
        }
        if !self.xy.touched() {
            (p.x, p.y) = self.xy.position();
        }
    }

    /// Your timing: the last two bars of hits against the 1/16 grid.
    pub fn fill_timing(&self, t: &mut YourTiming) {
        let tr = &self.transport;
        let bar = tr.meter.bar_ticks().max(1);
        let window = (TIMING_BARS * bar) as f64;
        let step = STEP_TICKS as f64;
        let now = tr.position as f64;
        let mut by_pad: Vec<(usize, Vec<TimedHit>)> = Vec::new();
        let (mut offs, mut vels) = (Vec::new(), Vec::new());
        let mut histogram = [0u32; 15];
        let round_start = (now / window).floor() * window;
        for h in &self.hits {
            if h.tick < now - window || h.tick > now + step {
                continue;
            }
            let grid = (h.tick / step).round() * step;
            let off_ticks = h.tick - grid;
            let off_ms = (off_ticks * 60_000.0 / (h.bpm.max(1.0) * f64::from(PPQ))) as f32;
            // From the nearest sixteenth's place, so an early hit on the
            // window's first step sits just left of it (not at the end).
            let at = ((grid.rem_euclid(window) + off_ticks) / step) as f32;
            let hit = TimedHit {
                pad: usize::from(h.pad),
                at,
                off: (off_ticks / step) as f32,
                off_ms,
                velocity: h.velocity,
                old: h.tick < round_start - step / 2.0,
            };
            match by_pad.iter_mut().find(|(p, _)| *p == hit.pad) {
                Some((_, v)) => v.push(hit),
                None => by_pad.push((hit.pad, vec![hit])),
            }
            offs.push(off_ms);
            vels.push(f32::from(h.velocity));
            let b = ((off_ms + 30.0) / 4.0).floor().clamp(0.0, 14.0) as usize;
            histogram[b] += 1;
        }
        // The four sounds hit most, in pad order.
        by_pad.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
        by_pad.truncate(4);
        by_pad.sort_by_key(|(p, _)| *p);
        let lanes: Vec<(usize, String, Color, Vec<TimedHit>)> = by_pad
            .into_iter()
            .map(|(p, v)| {
                let c = self
                    .pads
                    .get(p)
                    .and_then(PadInfo::color)
                    .unwrap_or(draw::FX);
                (p, self.pad_name(p), c, v)
            })
            .collect();
        let n = offs.len() as f32;
        let mean = (n > 0.0).then(|| offs.iter().sum::<f32>() / n);
        let spread = mean.map(|m| (offs.iter().map(|o| (o - m) * (o - m)).sum::<f32>() / n).sqrt());
        let velocity = (n > 0.0).then(|| vels.iter().sum::<f32>() / n);
        let now_at = tr
            .state
            .rolling()
            .then(|| (now.rem_euclid(window) / step) as f32);
        t.lanes = lanes;
        t.now = now_at;
        t.steps = (TIMING_BARS * bar / STEP_TICKS) as usize;
        t.average_ms = mean;
        t.spread_ms = spread;
        t.velocity = velocity;
        t.histogram = histogram;
        t.right = format!(
            "last {TIMING_BARS} bars · grid 1/16 · offsets ×{:.0}",
            t.magnify
        );
        t.note = if tr.state.rolling() {
            "Play the pads: your hits show here against the grid".into()
        } else {
            "Press PLAY and play the pads: your hits show here against the grid".into()
        };
    }

    /// The transport card.
    /// The REC chip with the notes armed: the clip they record into (the
    /// one the pattern shows, or a new one when the track has none), and
    /// whether the take is ready or records.
    pub fn notes_rec_chip(&self, ready: Option<crate::rec_arm::Take>) -> RecChip {
        let t = &self.transport;
        let recording = matches!(t.state, State::Recording | State::CountIn);
        let track = if self.steps.track_name.is_empty() {
            "Track 1"
        } else {
            self.steps.track_name.as_str()
        };
        let clip = self
            .steps
            .clip
            .as_ref()
            .map_or("new clip", |c| c.name.as_str());
        RecChip {
            state: if recording {
                RecState::Recording
            } else if ready.is_some() {
                RecState::Ready
            } else {
                RecState::Armed
            },
            what: format!("{track} · {clip}"),
            add: if recording {
                t.overdub
            } else {
                ready == Some(crate::rec_arm::Take::Add)
            },
            pinned: false,
        }
    }

    pub fn transport_look(&self) -> TransportLook {
        let t = &self.transport;
        let (state, tone) = match t.state {
            State::Stopped => ("STOPPED", Tone::Dim),
            State::CountIn => ("COUNT-IN", Tone::Warn),
            State::Recording if t.overdub => ("OVERDUB", Tone::Warn),
            State::Recording => ("RECORDING", Tone::Bad),
            State::Playing => ("PLAYING", Tone::Good),
        };
        let bar_ticks = t.meter.bar_ticks().max(1);
        let (start, bars) = match t.playing_loop() {
            Some((a, b)) if b > a => (a, ((b - a) / bar_ticks).clamp(1, 8) as usize),
            _ => (0, 4),
        };
        let pos = t.position.max(0);
        let rolling = t.state.rolling();
        TransportLook {
            state: state.into(),
            tone,
            pos: position_text(t),
            bpm: format!("{:.1}", t.bpm),
            step: rolling.then(|| ((pos % bar_ticks) / STEP_TICKS).clamp(0, 15) as usize),
            bar: (((pos - start).max(0) / bar_ticks) as usize) % bars,
            bars,
            views: if self.sequencer {
                &SEQ_SWITCH
            } else {
                &PLAY_SWITCH
            },
            view: if self.sequencer {
                0
            } else {
                PLAY_VIEWS.iter().position(|&v| v == self.view).unwrap_or(0)
            },
            rec: None,
        }
    }
}

/// The decay's longest time on the card: past it the sound plays out.
pub const DECAY_MAX_MS: f32 = 4000.0;

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_ui::layout;

    #[test]
    fn the_xy_pad_plays_the_dj_filter_and_echo_and_glides_home() {
        let mut p = PlayPage::default();
        let mut w = XyPad::default();
        p.fill_xy(&mut w);
        assert_eq!((w.label.as_str(), w.x, w.y), ("Off · echo 0 %", 0.5, 0.0));
        assert_eq!(
            p.xy(0.0, 1.0, true),
            PlayFx::Xy {
                x: 0.0,
                y: 1.0,
                down: true
            }
        );
        p.fill_xy(&mut w);
        assert_eq!(w.label, "LP 80 Hz · echo 100 %");
        assert_eq!(p.tick_xy(16.0), None, "a finger holds it");
        p.xy(0.0, 1.0, false);
        let mut moves = 0;
        while let Some(fx) = p.tick_xy(16.0) {
            assert!(matches!(fx, PlayFx::Xy { down: false, .. }), "{fx:?}");
            moves += 1;
            assert!(moves < 100, "it lands");
        }
        assert!(moves > 3, "it glides");
        assert_eq!(p.xy.position(), (0.5, 0.0));
        p.fill_xy(&mut w);
        assert_eq!((w.label.as_str(), w.x, w.y), ("Off · echo 0 %", 0.5, 0.0));
    }

    /// The synth's pads in A minor, In key from A3 (the roots on pads 1,
    /// 8 and 15), and a clip with notes on `notes` (pad, step).
    fn synth_page(notes: &[(u8, usize)]) -> PlayPage {
        let mut p = PlayPage::default();
        let scale = [0u8, 2, 3, 5, 7, 8, 10];
        p.pads = (0..16)
            .map(|k| {
                let note = 57 + 12 * (k / 7) as u8 + scale[k % 7];
                PadInfo {
                    label: Some(crate::synth::note_name(note)),
                    color: None,
                    pitch: Some(PadPitch {
                        note,
                        root: k % 7 == 0,
                    }),
                }
            })
            .collect();
        let mut rows: Vec<(u8, Vec<Option<Step>>)> = (0..16).map(|p| (p, vec![None; 16])).collect();
        for &(pad, i) in notes {
            rows[usize::from(pad)].1[i] = Some(Step {
                velocity: 100,
                len: STEP_TICKS,
                nudge: 0,
                chance: 100,
                cond: Default::default(),
                ratchet: 1,
                notes: 1,
                low: pad,
                high: pad,
            });
        }
        p.steps.clip = Some(crate::time::StepClip {
            slot: 0,
            name: "Pattern 1".into(),
            steps: 16,
            song_length: 3840.0,
            song_step: 240.0,
            origin: None,
            rows,
            locks: Vec::new(),
            lock_values: Vec::new(),
        });
        p
    }

    /// Beats on a synth track (the sequencer design's mockup B): its
    /// rows are the pads' notes in pitch order from the root under the
    /// lowest note used; Fold keeps the notes used; the octave's ‹ and ›
    /// move the window an octave; a kit's lanes are as they were.
    #[test]
    fn a_synth_tracks_rows_follow_the_scale_fold_and_the_octave() {
        // C4, E4 and C5 used.
        let mut p = synth_page(&[(2, 0), (4, 4), (9, 8)]);
        assert!(p.pitched());
        assert_eq!(p.note_rows(), (0..16).collect::<Vec<_>>());
        // From A3, the root under C4.
        assert_eq!(p.low_row(&p.note_rows()), 0);
        let mut g = StepGrid::default();
        p.fill_steps(&mut g);
        assert_eq!(
            g.pitched,
            Some(crate::play_widgets::Pitched {
                fold: false,
                low: 0
            })
        );
        let first: Vec<(usize, Option<u8>, bool)> = g
            .lanes
            .iter()
            .take(3)
            .map(|l| (l.pad, l.note, l.root))
            .collect();
        assert_eq!(
            first,
            [
                (0, Some(57), true),
                (1, Some(59), false),
                (2, Some(60), false)
            ]
        );
        // The synth's pads have no mutes: the kit's are the drums'.
        p.levels.pad_mutes = 0b11;
        p.fill_steps(&mut g);
        assert!(g.lanes.iter().all(|l| !l.muted));
        // An octave up: A4 (pad 8) at the bottom; then as high as it
        // goes with eight rows; then no further.
        assert!(p.turn_octave(true, 8));
        assert_eq!(p.low, Some(7));
        assert!(p.turn_octave(true, 8));
        assert_eq!(p.low, Some(8));
        assert!(!p.turn_octave(true, 8));
        // Down: an octave under B4 is B3, then the bottom.
        assert!(p.turn_octave(false, 8));
        assert_eq!(p.low, Some(1));
        assert!(p.turn_octave(false, 8));
        assert!(!p.turn_octave(false, 8));
        assert_eq!(p.low, Some(0));
        // Fold: only C4, E4 and C5, from the lowest.
        p.toggle_fold();
        assert_eq!((p.fold, p.low), (true, None));
        assert_eq!(p.note_rows(), vec![2, 4, 9]);
        assert_eq!(p.low_row(&p.note_rows()), 0);
        p.fill_steps(&mut g);
        assert_eq!(g.lanes.iter().map(|l| l.pad).collect::<Vec<_>>(), [2, 4, 9]);
        // Fold with no notes shows them all.
        let mut empty = synth_page(&[]);
        empty.toggle_fold();
        assert_eq!(empty.note_rows().len(), 16);
        // With no notes, from the lowest root; a note far above it, from
        // the note itself.
        assert_eq!(empty.low_row(&empty.note_rows()), 0);
        let high = synth_page(&[(10, 0)]);
        assert_eq!(high.low_row(&high.note_rows()), 7, "A4, the root under D5");
        let higher = synth_page(&[(13, 0)]);
        assert_eq!(higher.low_row(&higher.note_rows()), 7, "A4 under G5");
        // A kit: no scale rows.
        let mut kit = synth_page(&[(2, 0)]);
        for pad in &mut kit.pads {
            pad.pitch = None;
        }
        kit.fill_steps(&mut g);
        assert_eq!(g.pitched, None);
    }

    #[test]
    fn the_swing_card_says_off_when_straight_as_the_transport_bar_does() {
        let mut p = PlayPage::default();
        p.transport.swing.amount = 50;
        assert_eq!(p.card(Knob::Swing, None).value, "off");
        p.turn_knob(Knob::Swing, 8, false);
        assert_eq!(p.card(Knob::Swing, None).value, "58 %");
    }

    fn slots(spec: &str) -> Vec<(String, Rect)> {
        layout::solve(
            &layout::parse(spec).unwrap(),
            Rect::new(12, 12, 1256, 776),
            12,
        )
    }

    fn rect(spec: &str, name: &str) -> Rect {
        slots(spec)
            .into_iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("{name} in {spec}"))
            .1
    }

    #[test]
    fn every_view_keeps_the_frame() {
        for v in PlayView::ALL {
            let timing = [v.layout_for(true), v.quick_layout_for(true)];
            for spec in [v.layout(), v.quick_layout()].into_iter().chain(timing) {
                assert_eq!(
                    rect(&spec, TRANSPORT),
                    Rect::new(12, 84, 1038, TRANSPORT_H),
                    "{spec}"
                );
                assert_eq!(
                    rect(&spec, MASTER),
                    Rect::new(1062, 84, MASTER_WIDTH, 666),
                    "{spec}"
                );
                assert_eq!(
                    rect(&spec, QLINKS),
                    Rect::new(1140, 84, COLUMN_WIDTH, 666),
                    "{spec}"
                );
                assert_eq!(rect(&spec, "hint").y, 762, "{spec}");
            }
        }
    }

    /// The panels sit where mockups F to I put them.
    #[test]
    fn the_views_are_laid_out_as_the_mockups() {
        let all = PlayView::All.layout();
        let strip = crate::looper_page::LOOP_STRIP;
        assert_eq!(rect(&all, strip), Rect::new(12, 180, 1038, 150));
        // The pattern fills the room the mini pads had (Jay, 12 Nov 15:45Z).
        assert_eq!(rect(&all, STEPS), Rect::new(12, 342, 1038, 276));
        assert_eq!(rect(&all, THROWS), Rect::new(12, 630, 746, 120));
        assert_eq!(rect(&all, XY), Rect::new(770, 630, 280, 120));
        let beats = PlayView::Beats.layout();
        assert_eq!(rect(&beats, STEPS), Rect::new(12, 180, 1038, 362));
        assert_eq!(rect(&beats, THROWS), Rect::new(12, 554, 1038, 196));
        let lp = PlayView::Loop.layout();
        assert_eq!(rect(&lp, strip), Rect::new(12, 180, 1038, 362));
        assert_eq!(rect(&lp, THROWS), Rect::new(12, 554, 726, 196));
        assert_eq!(rect(&lp, XY), Rect::new(750, 554, 300, 196));
        // Beats with the timing strip (Jay 13 Nov: the Drum view's
        // timing folded into Beats): the pattern keeps six lanes, as All's.
        let timed = PlayView::Beats.layout_for(true);
        assert_eq!(rect(&timed, STEPS), Rect::new(12, 180, 1038, 278));
        assert_eq!(
            rect(&timed, TIMING),
            Rect::new(12, 470, 1038, TIMING_STRIP_H)
        );
        assert_eq!(rect(&timed, THROWS), rect(&beats, THROWS));
        const { assert!(TIMING_STRIP_H < crate::play_widgets::YourTiming::FULL_H) };
        assert_eq!(PlayView::Beats.page_with(true, false), BEATS_TIMING_PAGE);
        assert_eq!(PlayView::Loop.page_with(true, true), PlayView::Loop.page());
        // No view has a pads panel.
        for v in PlayView::ALL {
            for spec in [v.layout(), v.quick_layout()] {
                assert!(!slots(&spec).iter().any(|(n, _)| n == PADS), "{spec}");
            }
        }
    }

    #[test]
    fn the_band_never_moves_the_panels_below_it() {
        for v in PlayView::ALL {
            for timing in [false, true] {
                let (plain, band) = (v.layout_for(timing), v.quick_layout_for(timing));
                assert_eq!(rect(&plain, THROWS), rect(&band, THROWS), "{v:?}");
                assert_eq!(rect(&band, arp_view::MINI).h, arp_view::MINI_H, "{v:?}");
            }
        }
    }

    #[test]
    fn the_arp_panel_takes_the_view() {
        let spec = framed(arp_view::ARP_LAYOUT);
        assert!(!slots(&spec).iter().any(|(n, _)| n == PADS), "{spec}");
        let list = rect(&spec, arp_view::LIST);
        assert_eq!((list.x, list.right()), (12, 1050), "{list:?}");
        assert_eq!(rect(&spec, QLINKS), Rect::new(1140, 84, COLUMN_WIDTH, 666));
        assert_eq!(framed(&spec), spec, "framing twice changes nothing");
        // Any other page body still gets the frame.
        let other = framed("col(header:60, log:*, hint:26)");
        assert!(rect(&other, "log").w > 900);
    }

    #[test]
    fn the_synth_tab_gets_the_pads_heading_over_its_presets() {
        use mpfree_synth_ui::view::{INST, LAYOUT, PRESETS};
        let spec = synth_with_pads(LAYOUT);
        let (head, presets) = (rect(&spec, SYNTH_PADS_HEAD), rect(&spec, PRESETS));
        assert_eq!((head.x, head.w, head.h), (presets.x, 220, 30), "{spec}");
        assert_eq!(
            head.y,
            rect(&spec, INST).bottom() + 12,
            "under the instrument"
        );
        assert_eq!(presets.y, head.bottom() + 12);
    }

    #[test]
    fn the_switch_halves_tap_chromatic_or_in_key() {
        let r = Rect::new(570, 140, 440, 30);
        let mut bar = PadsBar {
            title: "Synth · Bank A".into(),
            synth: None,
        };
        let [chromatic, in_key] = PadsBar::halves(r);
        assert_eq!(
            bar.touch(1, Touch::Down, in_key.x + 5, in_key.y + 5, r),
            None,
            "no switch for a kit"
        );
        bar.synth = Some(SynthPads {
            layout: PadLayout::Chromatic,
            scale: true,
            bank: 0,
        });
        assert_eq!(
            bar.touch(1, Touch::Down, in_key.x + 5, in_key.y + 5, r),
            Some(EventKind::Tab(1))
        );
        assert_eq!(
            bar.touch(1, Touch::Down, chromatic.x + 5, chromatic.y + 5, r),
            Some(EventKind::Tab(0))
        );
        assert_eq!(
            bar.touch(1, Touch::Down, r.x + 5, r.y + 5, r),
            None,
            "the title is not the switch"
        );
        assert_eq!(in_key.right(), r.right());
    }
}
