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
//!   the four Q-Link cards (168 px) next to the MPC's Q-Link knobs;
//! - **All** (home, F): the looper strip, the pattern beside the pads, and
//!   the throws beside the XY pad;
//! - **Beats** (G): the whole pattern, then the pads and the throws;
//! - **Loop** (H): the loop big with its buttons, then the pads, the
//!   throws and the XY pad;
//! - **Drum** (I): your timing beside the XY pad, then the pads and the
//!   throws.
//!
//! The panels are in `play_widgets`. The arp's panel goes into the same
//! frame ([`framed`]): its column takes the view's left and the pads the
//! right. NOTE REPEAT's slim strip takes the top of the view's first panel
//! ([`PlayView::quick_layout`]), so the pads never move when it comes and
//! goes.

use crate::mixing::{self, PlayFx, PlayLevels, ThrowFx};
use crate::play_widgets::{
    Lane, StepGrid, Throw, Throws, TimedHit, TransportLook, XyPad, YourTiming,
};
use crate::time::{HitInfo, StepAction, StepsData, STEP_TICKS};
use mpfree_arp::scale::pads::PadLayout;
use mpfree_arp_ui::view as arp_view;
use mpfree_fx::ids::mixer as mixer_ids;
use mpfree_fx::StripId;
use mpfree_seq::steps::{Change, Step, PAGE};
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
/// The pads heading on the Synth tab, over its presets: always the synth.
pub const SYNTH_PADS_HEAD: &str = "synth-pads-head";

/// The views, in the switch's order.
pub const VIEWS: [&str; 4] = ["All", "Beats", "Loop", "Drum"];
/// The transport card's height.
pub const TRANSPORT_H: i32 = 84;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlayView {
    #[default]
    All,
    Beats,
    Loop,
    Drum,
}

impl PlayView {
    pub const ALL: [PlayView; 4] = [
        PlayView::All,
        PlayView::Beats,
        PlayView::Loop,
        PlayView::Drum,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> PlayView {
        PlayView::ALL[i.min(3)]
    }

    pub fn name(self) -> &'static str {
        VIEWS[self.index()]
    }

    /// The view's page in the `Ui`. All, the home view, is "Play", as
    /// before.
    pub fn page(self) -> &'static str {
        match self {
            PlayView::All => "Play",
            PlayView::Beats => "Play · Beats",
            PlayView::Loop => "Play · Loop",
            PlayView::Drum => "Play · Drum",
        }
    }

    /// The view under the transport card, as the mockups lay it out. With
    /// `band`, NOTE REPEAT's strip takes the top of the first panel, so
    /// nothing else moves.
    fn body(self, band: bool) -> String {
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
                "{strip}:150, row({}, {PADS}:{HOME_PADS_W}):*, row({THROWS}:*, {XY}:280):120",
                first(STEPS)
            ),
            PlayView::Beats => format!("{}, row({PADS}:300, {THROWS}:*):196", first(STEPS)),
            PlayView::Loop => format!(
                "{}, row({PADS}:300, {THROWS}:*, {XY}:300):196",
                first(strip)
            ),
            PlayView::Drum => format!(
                "row({}, {XY}:330):*, row({PADS}:300, {THROWS}:*):196",
                first(TIMING)
            ),
        }
    }

    pub fn layout(self) -> String {
        frame(&self.body(false))
    }

    /// The view while NOTE REPEAT is held or pads are latched.
    pub fn quick_layout(self) -> String {
        frame(&self.body(true))
    }
}

/// The mini pads' width on the home view (All), which the arp's panel
/// keeps, so the pads stay put when it opens.
const HOME_PADS_W: i32 = 270;

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
/// column (`row(pads:*, col(...):W):*`), the column takes the view's left
/// and the pads the right. A layout already in the frame comes back as it
/// is.
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
        Some((column, width)) if width.parse::<i32>().is_ok() => {
            frame(&format!("row({column}:*, {PADS}:{HOME_PADS_W}):*"))
        }
        _ => frame(body),
    }
}

/// The Synth tab's layout with the pads heading, and its switch, over the
/// presets (Jay, 12:22Z: the switch on Play and on the Synth tab).
pub fn synth_with_pads(layout: &str) -> String {
    let presets = mpfree_synth_ui::view::PRESETS;
    layout.replacen(
        &format!("{presets}:300"),
        &format!("col({SYNTH_PADS_HEAD}:30, {presets}:*):300"),
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
        cx.text(
            FontId::Bold,
            t.size_small,
            title,
            Align::Left,
            t.text_dim,
            &self.title.to_uppercase(),
        );
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
    /// The pad hit last: its tuning and its decay (the Sample tab's).
    Tune,
    Decay,
    DrumsFilter,
    DrumsSend,
    LoopLevel,
    LoopSend,
    LineLevel,
    LoopFilter,
    FxMix,
    /// The step picked in Beats.
    Velocity,
    Length,
    Nudge,
    Chance,
}

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
        sub: "hall, 4 s",
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
    pub steps: StepsData,
    /// The page of steps shown (16 a page).
    pub page: usize,
    /// The step picked in Beats: (pad, step in the clip).
    pub selected: Option<(usize, usize)>,
    /// The pad hit last (Tune and Decay turn it).
    pub last_pad: usize,
    pub pads: Vec<PadInfo>,
    pub levels: PlayLevels,
    turned_at: Option<Instant>,
    pub transport: TransportInfo,
    hits: VecDeque<HitInfo>,
    pub throws: [bool; 8],
    pub xy: (f32, f32, bool),
}

impl Default for PlayPage {
    fn default() -> Self {
        PlayPage {
            view: PlayView::All,
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
                fx_mix: 0.35,
                pad_mutes: 0,
            },
            turned_at: None,
            transport: TransportInfo::default(),
            hits: VecDeque::new(),
            throws: [false; 8],
            xy: (0.5, 0.3, false),
        }
    }
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
    /// The Q-Links' targets on the view shown.
    pub fn knobs(&self) -> [Knob; 4] {
        match self.view {
            PlayView::Beats if self.selected_step().is_some() => {
                [Knob::Velocity, Knob::Length, Knob::Nudge, Knob::Chance]
            }
            PlayView::All | PlayView::Beats => {
                [Knob::Swing, Knob::Tune, Knob::DrumsFilter, Knob::DrumsSend]
            }
            PlayView::Loop => [
                Knob::LoopLevel,
                Knob::LoopSend,
                Knob::LineLevel,
                Knob::LoopFilter,
            ],
            PlayView::Drum => [Knob::Tune, Knob::Decay, Knob::Swing, Knob::FxMix],
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
                let a = self.transport.swing.amount;
                QLinkCard::new(
                    "Swing",
                    "Transport",
                    &format!("{a} %"),
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
            Knob::Decay => match pad {
                Some((_, Some(ms))) => QLinkCard::new(
                    "Decay",
                    &self.pad_scope(self.last_pad),
                    &decay_text(ms),
                    (ms / DECAY_MAX_MS).sqrt().clamp(0.0, 1.0),
                ),
                Some((_, None)) => {
                    QLinkCard::new("Decay", &self.pad_scope(self.last_pad), "Full", 1.0)
                }
                None => QLinkCard::new("Decay", &self.pad_scope(self.last_pad), "-", 0.0),
            },
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
            Knob::FxMix => QLinkCard::new("FX mix", "XY", &pct(l.fx_mix), l.fx_mix),
            Knob::Velocity | Knob::Length | Knob::Nudge | Knob::Chance => {
                let Some((pad, i, s)) = self.selected_step() else {
                    return QLinkCard::default();
                };
                let scope = format!("Step {} · {}", i + 1, self.pad_name(pad));
                match knob {
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
            Knob::Tune | Knob::Decay => Turn::Pad(knob, ticks),
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
            Knob::FxMix => {
                l.fx_mix = (l.fx_mix + d * if fine { 0.005 } else { 0.02 }).clamp(0.0, 1.0);
                Turn::Fx(PlayFx::FxMix(l.fx_mix))
            }
            Knob::Velocity | Knob::Length | Knob::Nudge | Knob::Chance => {
                let (pad, i, s) = self.selected_step()?;
                let change = match knob {
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
                let a = StepAction::Change {
                    pad: pad as u8,
                    step: i,
                    change,
                };
                self.apply_locally(a);
                Turn::Step(a)
            }
        })
    }

    /// A step change shows at once; the engine's steps follow.
    fn apply_locally(&mut self, a: StepAction) {
        let StepAction::Change { pad, step, change } = a else {
            return;
        };
        let Some(clip) = self.steps.clip.as_mut() else {
            return;
        };
        let Some((_, row)) = clip.rows.iter_mut().find(|(p, _)| *p == pad) else {
            return;
        };
        if let Some(Some(s)) = row.get_mut(step) {
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
        self.page = self.page.min(pages - 1);
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

    /// The pattern's lanes, most wanted first: the sounds with notes, the
    /// pad hit last, then the kit's other sounds, then the rest.
    fn lanes(&self) -> Vec<Lane> {
        let clip = self.steps.clip.as_ref();
        let has_notes = |p: usize| clip.is_some_and(|c| c.row(p as u8).iter().any(Option::is_some));
        let has_sound = |p: usize| self.pads.get(p).is_some_and(|i| i.label.is_some());
        let mut order: Vec<usize> = (0..16).filter(|&p| has_notes(p)).collect();
        if !order.contains(&self.last_pad) && has_sound(self.last_pad) {
            order.push(self.last_pad);
        }
        order.extend((0..16).filter(|&p| has_sound(p) && !has_notes(p) && p != self.last_pad));
        order.extend((0..16).filter(|&p| !has_sound(p) && !has_notes(p) && p != self.last_pad));
        let from = self.page * PAGE;
        order
            .into_iter()
            .map(|p| {
                let row = clip.map_or(&[][..], |c| c.row(p as u8));
                Lane {
                    pad: p,
                    name: self.pad_name(p),
                    color: self.pads[p].color().unwrap_or(draw::FX),
                    muted: self.levels.pad_mutes & (1 << p) != 0,
                    steps: (from..from + PAGE)
                        .map(|i| row.get(i).copied().flatten().map_or(0, |s| s.velocity))
                        .collect(),
                }
            })
            .collect()
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
        if pages > 1 {
            right.push_str(&format!(" · page {} of {pages}", self.page + 1));
        }
        if clip.is_none() {
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
    }

    /// A tap on step `col` of the page shown, on `pad`'s lane. In Beats a
    /// tap on a step with notes picks it (for the Q-Links), a second tap
    /// clears it; elsewhere a tap turns it on or off.
    pub fn tap_step(&mut self, pad: usize, col: usize) -> Option<StepAction> {
        let i = self.page * PAGE + col;
        let on = self
            .steps
            .clip
            .as_ref()
            .and_then(|c| c.row(pad as u8).get(i).copied().flatten())
            .is_some();
        if self.view == PlayView::Beats && on && self.selected != Some((pad, i)) {
            self.selected = Some((pad, i));
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

    /// The XY pad moved, or let go.
    pub fn xy(&mut self, x: f32, y: f32, down: bool) -> PlayFx {
        self.xy = (x, y, down);
        PlayFx::Xy { x, y, down }
    }

    pub fn fill_xy(&self, p: &mut XyPad) {
        let (x, y, _) = self.xy;
        let (hz, feedback) = mixing::xy_meaning(x, y);
        let label = format!("{} · {:.0} %", hz_text(hz), feedback * 100.0);
        if p.label != label {
            p.label = label;
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
        let (start, bars) = match t.loop_range {
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
            view: self.view.index(),
        }
    }
}

/// The decay's longest time on the card: past it the sound plays out.
pub const DECAY_MAX_MS: f32 = 4000.0;

fn decay_text(ms: f32) -> String {
    if ms >= 1000.0 {
        format!("{:.1} s", ms / 1000.0)
    } else {
        format!("{ms:.0} ms")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_ui::layout;

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
            for spec in [v.layout(), v.quick_layout()] {
                assert_eq!(
                    rect(&spec, TRANSPORT),
                    Rect::new(12, 84, 998, TRANSPORT_H),
                    "{spec}"
                );
                assert_eq!(
                    rect(&spec, MASTER),
                    Rect::new(1022, 84, MASTER_WIDTH, 666),
                    "{spec}"
                );
                assert_eq!(
                    rect(&spec, QLINKS),
                    Rect::new(1100, 84, COLUMN_WIDTH, 666),
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
        assert_eq!(rect(&all, strip), Rect::new(12, 180, 998, 150));
        assert_eq!(rect(&all, STEPS), Rect::new(12, 342, 716, 276));
        assert_eq!(rect(&all, PADS), Rect::new(740, 342, 270, 276));
        assert_eq!(rect(&all, THROWS), Rect::new(12, 630, 706, 120));
        assert_eq!(rect(&all, XY), Rect::new(730, 630, 280, 120));
        let beats = PlayView::Beats.layout();
        assert_eq!(rect(&beats, STEPS), Rect::new(12, 180, 998, 362));
        assert_eq!(rect(&beats, PADS), Rect::new(12, 554, 300, 196));
        assert_eq!(rect(&beats, THROWS), Rect::new(324, 554, 686, 196));
        let lp = PlayView::Loop.layout();
        assert_eq!(rect(&lp, strip), Rect::new(12, 180, 998, 362));
        assert_eq!(rect(&lp, PADS), Rect::new(12, 554, 300, 196));
        assert_eq!(rect(&lp, THROWS), Rect::new(324, 554, 374, 196));
        assert_eq!(rect(&lp, XY), Rect::new(710, 554, 300, 196));
        let drum = PlayView::Drum.layout();
        assert_eq!(rect(&drum, TIMING), Rect::new(12, 180, 656, 362));
        assert_eq!(rect(&drum, XY), Rect::new(680, 180, 330, 362));
        assert_eq!(rect(&drum, PADS), Rect::new(12, 554, 300, 196));
        assert_eq!(rect(&drum, THROWS), Rect::new(324, 554, 686, 196));
    }

    #[test]
    fn the_band_never_moves_the_pads() {
        for v in PlayView::ALL {
            let (plain, band) = (v.layout(), v.quick_layout());
            assert_eq!(rect(&plain, PADS), rect(&band, PADS), "{v:?}");
            assert_eq!(rect(&band, arp_view::MINI).h, arp_view::MINI_H, "{v:?}");
        }
    }

    #[test]
    fn the_arp_panel_takes_the_left_and_the_pads_stay_right() {
        let spec = framed(arp_view::ARP_LAYOUT);
        let pads = rect(&spec, PADS);
        assert_eq!((pads.right(), pads.w), (1010, HOME_PADS_W));
        let list = rect(&spec, arp_view::LIST);
        assert!(list.right() < pads.x && list.w >= 460, "{list:?}");
        assert_eq!(rect(&spec, QLINKS), Rect::new(1100, 84, COLUMN_WIDTH, 666));
        assert_eq!(framed(&spec), spec, "framing twice changes nothing");
        // Any other page body still gets the frame.
        let other = framed("col(header:60, log:*, hint:26)");
        assert!(rect(&other, "log").w > 900);
    }

    #[test]
    fn the_synth_tab_gets_the_pads_heading_over_its_presets() {
        use mpfree_synth_ui::view::{LAYOUT, PLAIN_LAYOUT, PRESETS};
        for l in [LAYOUT, PLAIN_LAYOUT] {
            let spec = synth_with_pads(l);
            let (head, presets) = (rect(&spec, SYNTH_PADS_HEAD), rect(&spec, PRESETS));
            assert_eq!((head.x, head.w, head.h), (presets.x, 300, 30), "{spec}");
            assert_eq!(presets.y, head.bottom() + 12);
        }
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
