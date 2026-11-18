//! The Play page's panels, drawn as Jay's approved mockups F to I draw
//! them (/mnt/project-files/hello-mpc/play-mockups/source: the same
//! sizes, colours and spacing), from the app's real state:
//!
//! - [`PlayTransport`]: the transport card (state, position, tempo, where
//!   the bar is) with the view switch at its right end.
//! - [`MiniPads`]: the small pad map with its heading, and the Chromatic |
//!   In key switch while the synth has the pads.
//! - [`StepGrid`]: the pattern, a lane per sound with its speaker (mute).
//! - [`Throws`]: hold-to-play effects.
//! - [`XyPad`]: the two-way effect pad.
//! - [`YourTiming`]: your hits against the grid, and how they spread.
//!
//! The widgets only draw and turn touches into events; the app fills them
//! in and decides what the events do. Each redraws only what changed where
//! that matters on the MPC (the transport's numbers, a pad, a step).

use crate::play_page::{SynthPads, LAYOUT_SWITCH, LOCKED, PLAY_SWITCH};
use mpfree_arp::scale::pads::PadLayout;
use mpfree_seq::Cond;
use mpfree_ui::draw::{self, card, card_head, dot, pill, speaker, KNOB};
use mpfree_ui::widgets::{Pad, PadGrid};
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Target, Tone, Touch, Widget};
use std::cell::RefCell;
use std::collections::VecDeque;

// ------------------------------------------------------------- transport

/// What the transport card shows.
#[derive(Clone, Debug, PartialEq)]
pub struct TransportLook {
    /// "PLAYING", "STOPPED", "RECORDING", "OVERDUB", "COUNT-IN".
    pub state: String,
    pub tone: Tone,
    /// bar.beat.sixteenth: "9.1.02".
    pub pos: String,
    /// "92.0".
    pub bpm: String,
    /// The sixteenth of the bar playing (0 to 15), None while stopped.
    pub step: Option<usize>,
    /// The bar playing within `bars` (the loop's, or 4).
    pub bar: usize,
    pub bars: usize,
    /// The switch's views: All | Loop on Play, Steps | Notes on the
    /// Sequencer.
    pub views: &'static [&'static str],
    /// The view lit in the switch.
    pub view: usize,
    /// The REC chip: what REC records ([`crate::rec_arm`]).
    pub rec: Option<RecChip>,
    /// What the pads play, on Play's card only.
    pub pads: Option<PadsPlay>,
}

/// What the pads play, on Play's transport card: the armed track, its kit
/// and the bank on the pads, "Drums · Boom Bap · A" (the Play, Pads and
/// Sequencer redesign, step 4: it took the top bar's kit chip's place, and
/// names the bank since the PAD BANK lights can't show E to H). A tap
/// gives [`TAP_PADS`]: the app opens the list of kits.
#[derive(Clone, Debug, PartialEq)]
pub struct PadsPlay {
    /// "Drums": the track, left out first when the line is too long.
    pub track: String,
    /// "Boom Bap · A": the kit and the bank, or the synth's line.
    pub what: String,
    /// The track's colour, for the dot before the text.
    pub color: Color,
    /// Between the track and the rest: " · ", or in controller mode the
    /// Controller chip's own " → " or ": ".
    pub join: &'static str,
    /// Controller mode: the dot in the theme's warn colour, as the chip.
    pub warn: bool,
}

impl PadsPlay {
    /// The whole line: "Drums · Boom Bap · A".
    pub fn text(&self) -> String {
        format!("{}{}{}", self.track, self.join, self.what)
    }

    /// In controller mode, the Controller chip's words ("Controller →
    /// Juno-X · ch 3 · A", "Controller: pick its port on I/O"): "Controller"
    /// in the track's place, so a line too long keeps the gear and bank.
    pub fn controller(chip: &str) -> PadsPlay {
        let (join, what) = match (
            chip.strip_prefix("Controller → "),
            chip.strip_prefix("Controller: "),
        ) {
            (Some(w), _) => (" → ", w),
            (None, Some(w)) => (": ", w),
            (None, None) => (" · ", chip),
        };
        PadsPlay {
            track: "Controller".to_string(),
            what: what.to_string(),
            color: Color::rgb(0xFFC145),
            join,
            warn: true,
        }
    }
}

/// Where a take stands, for the REC chip; ordered by how far along it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RecState {
    /// REC would record it: grey, with a red dot.
    Armed,
    /// PLAY starts the take: outlined in red.
    Ready,
    /// Recording (or counting in): solid red.
    Recording,
}

/// The REC chip on the transport bars: what REC records and where its take
/// stands (sequencer decision 1, mockups F and G).
#[derive(Clone, Debug, PartialEq)]
pub struct RecChip {
    pub state: RecState,
    /// "Drums · Clip 1", "Loop 1".
    pub what: String,
    /// The take adds (OVERDUB) rather than replaces.
    pub add: bool,
    /// Only a tap on Arm moves the arm (the target list's Pin the arm).
    pub pinned: bool,
}

impl RecChip {
    /// Its first line: "REC" ("REC · PINNED"), "READY · PLAY starts",
    /// "● REC", "● OVERDUB".
    pub fn head(&self) -> String {
        match (self.state, self.add) {
            (RecState::Armed, _) if self.pinned => "REC · PINNED".into(),
            (RecState::Armed, _) => "REC".into(),
            (RecState::Ready, false) => "READY · PLAY starts".into(),
            (RecState::Ready, true) => "READY TO ADD · PLAY starts".into(),
            (RecState::Recording, false) => "● REC".into(),
            (RecState::Recording, true) => "● OVERDUB".into(),
        }
    }

    /// The chip as one line, for the Arrange bar and the log: "REC → Loop 1".
    pub fn line(&self) -> String {
        format!("{} → {}", self.head(), self.what)
    }
}

/// An Arm pill at the left of a panel's heading (the loop strip, the
/// pattern; mockups F and G): grey "Arm" on what REC doesn't record, red
/// on what it does. A tap gives `Select(0)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmPill {
    /// Not armed: a tap arms it.
    Arm,
    /// Armed: REC records it. The panel has a red edge.
    Armed,
    /// Ready: PLAY starts its take. The panel has a dashed red outline.
    Ready,
    /// Recording. The panel has a solid red outline.
    Rec,
}

impl ArmPill {
    pub fn label(self) -> &'static str {
        match self {
            ArmPill::Arm => "Arm",
            ArmPill::Armed => "Armed",
            ArmPill::Ready => "Ready",
            ArmPill::Rec => "Rec",
        }
    }

    /// The pill of what REC records, as its chip stands.
    pub fn of(state: RecState) -> ArmPill {
        match state {
            RecState::Armed => ArmPill::Armed,
            RecState::Ready => ArmPill::Ready,
            RecState::Recording => ArmPill::Rec,
        }
    }
}

/// The Arm pill's width, and the room the heading's title moves right by.
pub const ARM_PILL_W: i32 = 92;
const ARM_PILL_H: i32 = 28;

/// Where the Arm pill sits in a card `r`.
pub fn arm_pill_rect(r: Rect) -> Rect {
    let (h, _) = card_head(r);
    Rect::new(
        h.x - 2,
        h.y + h.h / 2 - ARM_PILL_H / 2,
        ARM_PILL_W,
        ARM_PILL_H,
    )
}

/// The Arm pill's tap area: the pill and the heading band round it.
pub fn arm_pill_hit(r: Rect) -> Rect {
    let p = arm_pill_rect(r);
    Rect::new(p.x - 6, r.y, p.w + 12, p.bottom() + 6 - r.y)
}

/// Paint card `r`'s Arm pill and its edge or outline (over the card
/// already painted).
pub fn paint_arm(cx: &mut PaintCx, r: Rect, pill: ArmPill) {
    let t = cx.theme.clone();
    paint_pill(cx, arm_pill_rect(r), pill);
    // The panel: a red edge while armed, a dashed outline while ready, a
    // solid one while recording.
    match pill {
        ArmPill::Arm => {}
        ArmPill::Armed => {
            cx.canvas
                .fill_round_rect(Rect::new(r.x, r.y + 12, 3, r.h - 24), 1, t.bad)
        }
        ArmPill::Ready => dashed_outline(cx, r, t.bad),
        ArmPill::Rec => cx.canvas.stroke_round_rect(r, t.radius, 2, t.bad),
    }
}

/// A dashed outline just inside `r`, 2 px wide.
pub fn dashed_outline(cx: &mut PaintCx, r: Rect, c: Color) {
    let o = r.inset(1);
    let (x0, y0, x1, y1) = (
        o.x as f32,
        o.y as f32,
        o.right() as f32 - 1.0,
        o.bottom() as f32 - 1.0,
    );
    let mut sh = mpfree_ui::Shape::new(r);
    sh.dashed(
        &[(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)],
        2.0,
        8.0,
    );
    cx.canvas.blend_shape(&sh, c);
}

/// The Arm pill itself in `p`.
pub fn paint_pill(cx: &mut PaintCx, p: Rect, pill: ArmPill) {
    let t = cx.theme.clone();
    let rad = p.h / 2;
    let (fill, line, dot_c, ink) = match pill {
        ArmPill::Arm => (t.surface, Some((1, t.outline)), None, t.text_dim),
        ArmPill::Armed => (
            t.bad.mix(t.surface, 0.8),
            Some((2, t.bad)),
            Some(t.bad),
            t.text,
        ),
        ArmPill::Ready => (
            t.bad.mix(t.surface, 0.85),
            Some((2, t.bad)),
            Some(t.bad),
            t.bad,
        ),
        ArmPill::Rec => (t.bad, None, Some(t.bg), t.bg),
    };
    cx.canvas.fill_round_rect(p, rad, fill);
    if let Some((w, c)) = line {
        cx.canvas.stroke_round_rect(p, rad, w, c);
    }
    let (dx, dy) = (p.x + 16, p.y + p.h / 2);
    match dot_c {
        Some(c) => dot(cx, dx, dy, 5, c),
        None => cx
            .canvas
            .stroke_round_rect(Rect::new(dx - 5, dy - 5, 10, 10), 5, 2, t.text_dim),
    }
    cx.text(
        FontId::Bold,
        13.0,
        Rect::new(p.x + 28, p.y, p.w - 32, p.h),
        Align::Left,
        ink,
        pill.label(),
    );
}

/// The chip's height on the Play transport card.
const REC_CHIP_H: i32 = 52;
/// A 16th's dot on the transport card, and the room between the dots and
/// the view switch (narrowed for the REC chip).
const DOT_CELL: i32 = 8;
const DOTS_GAP: i32 = 16;

pub fn paint_rec_chip(cx: &mut PaintCx, r: Rect, c: &RecChip) {
    let t = cx.theme.clone();
    let (fill, head_ink, ink) = match c.state {
        RecState::Armed => (t.raised, t.text_dim, t.text),
        RecState::Ready => (t.bad.mix(t.surface, 0.82), t.bad, t.text),
        RecState::Recording => (t.bad, t.bg, t.bg),
    };
    cx.canvas.fill_round_rect(r, 10, fill);
    if c.state == RecState::Ready {
        cx.canvas.stroke_round_rect(r, 10, 2, t.bad);
    }
    let mut x = r.x + 12;
    if c.state == RecState::Armed {
        cx.canvas
            .fill_round_rect(Rect::new(x, r.y + 13, 8, 8), 4, t.bad);
        x += 14;
    }
    let half = r.h / 2;
    cx.text(
        FontId::Bold,
        12.0,
        Rect::new(x, r.y + 4, r.right() - 10 - x, half - 2),
        Align::Left,
        head_ink,
        &c.head(),
    );
    cx.text(
        FontId::Bold,
        14.0,
        Rect::new(r.x + 12, r.y + half - 2, r.w - 20, half),
        Align::Left,
        ink,
        &format!("→ {}", c.what),
    );
}

/// What the pads play ([`PadsPlay`]), raised as a button: "PADS PLAY"
/// over the track's dot and the text.
fn paint_pads_play(cx: &mut PaintCx, r: Rect, p: &PadsPlay) {
    let t = cx.theme.clone();
    cx.canvas.fill_round_rect(r, 10, t.raised);
    let half = r.h / 2;
    cx.text(
        FontId::Bold,
        11.0,
        Rect::new(r.x + 12, r.y + 4, r.w - 22, half - 2),
        Align::Left,
        t.text_dim,
        "PADS PLAY",
    );
    let line = Rect::new(r.x + 12, r.y + half - 2, r.w - 22, half);
    cx.canvas.fill_round_rect(
        Rect::new(line.x, line.y + (line.h - 8) / 2, 8, 8),
        4,
        if p.warn { t.warn } else { p.color },
    );
    let text = Rect::new(line.x + 14, line.y, line.w - 14, line.h);
    // Too long: the track goes first (Play's rows name it), then the
    // kit's name is cut, but the bank at its end stays.
    let mut s = p.text();
    if cx.measure(FontId::Bold, 14.0, &s) > text.w {
        s.clone_from(&p.what);
    }
    if cx.measure(FontId::Bold, 14.0, &s) > text.w {
        if let Some((kit, bank)) = s.rsplit_once(" · ") {
            let tail = format!(" · {bank}");
            let room = text.w - cx.measure(FontId::Bold, 14.0, &tail);
            let mut cut = String::new();
            for c in kit.chars() {
                let next = format!("{cut}{c}…");
                if cx.measure(FontId::Bold, 14.0, &next) > room {
                    break;
                }
                cut.push(c);
            }
            s = format!("{}…{tail}", cut.trim_end());
        }
    }
    cx.text(FontId::Bold, 14.0, text, Align::Left, t.text, &s);
}

impl Default for TransportLook {
    fn default() -> Self {
        TransportLook {
            state: "STOPPED".into(),
            tone: Tone::Dim,
            pos: "1.1.01".into(),
            bpm: "120.0".into(),
            step: None,
            bar: 0,
            bars: 4,
            views: &PLAY_SWITCH,
            view: 0,
            rec: None,
            pads: None,
        }
    }
}

/// Where the parts that change while playing sit, as last drawn.
#[derive(Clone, Copy, Debug, Default)]
struct TransportAreas {
    pos: Rect,
    dots: Rect,
    /// The tempo and its "BPM": a tap there is a tap of the tempo.
    bpm: Rect,
    /// The REC chip, if it was drawn.
    rec: Rect,
    /// What the pads play, if it was drawn.
    pads: Rect,
}

/// [`PlayTransport`]'s event for a tap on the tempo
/// ([`EventKind::Select`] with this index): a tap of the tempo, as TAP
/// TEMPO (live test 3: Jay wants the BPM tappable).
pub const TAP_BPM: usize = 0;
/// [`PlayTransport`]'s event for a tap on the REC chip: what REC records
/// and how to change it is said.
pub const TAP_REC: usize = 1;
/// [`PlayTransport`]'s event for a tap on what the pads play
/// ([`PadsPlay`]): the app opens the list of kits.
pub const TAP_PADS: usize = 2;

/// The transport card. A tap on the switch gives `Tab(view)`.
#[derive(Default)]
pub struct PlayTransport {
    pub look: TransportLook,
    painted: RefCell<Option<(Rect, TransportLook, TransportAreas)>>,
}

const SWITCH_SEG_W: i32 = 70;
/// What the pads play: as wide as its text, within these.
const PADS_MIN_W: i32 = 132;
const PADS_MAX_W: i32 = 224;
const SWITCH_SEG_H: i32 = 44;

impl PlayTransport {
    #[cfg(test)]
    /// Where the tempo was drawn (its tap area), once painted.
    pub fn bpm_area(&self) -> Option<Rect> {
        self.painted.borrow().as_ref().map(|(_, _, a)| a.bpm)
    }

    fn inner(r: Rect) -> Rect {
        r.inset_xy(16, 12)
    }

    /// The view switch's box and its `n` segments.
    pub fn switch(r: Rect, n: usize) -> (Rect, Vec<Rect>) {
        let inner = PlayTransport::inner(r);
        let w = n as i32 * SWITCH_SEG_W + 8;
        let b = Rect::new(
            inner.right() - w,
            inner.y + (inner.h - SWITCH_SEG_H - 8) / 2,
            w,
            SWITCH_SEG_H + 8,
        );
        let segs = (0..n)
            .map(|i| {
                Rect::new(
                    b.x + 4 + i as i32 * SWITCH_SEG_W,
                    b.y + 4,
                    SWITCH_SEG_W,
                    SWITCH_SEG_H,
                )
            })
            .collect();
        (b, segs)
    }

    fn tone(&self, cx: &PaintCx) -> Color {
        let t = cx.theme;
        match self.look.tone {
            Tone::Dim | Tone::Normal => t.text_dim,
            k => t.tone(k),
        }
    }

    fn paint_dots(&self, cx: &mut PaintCx, a: Rect) {
        let t = cx.theme.clone();
        let tone = self.tone(cx);
        cx.canvas.fill_rect(a, t.surface);
        let l = &self.look;
        let bars = l.bars.max(1);
        let barw = (a.w - (bars as i32 - 1) * 4) / bars as i32;
        for b in 0..bars {
            let br = Rect::new(a.x + b as i32 * (barw + 4), a.y, barw, 8);
            let c = if l.step.is_none() {
                t.raised
            } else if b < l.bar {
                t.text_dim
            } else if b == l.bar {
                tone
            } else {
                t.raised
            };
            cx.canvas.fill_round_rect(br, 3, c);
        }
        let cell = DOT_CELL;
        let sy = a.y + 18;
        for s in 0..16 {
            let sx = a.x + s as i32 * cell + (s as i32 / 4) * 6;
            let c = match l.step {
                Some(p) if s == p => tone,
                Some(p) if s < p => t.outline,
                _ => t.raised,
            };
            let h = if s % 4 == 0 { 26 } else { 20 };
            cx.canvas
                .fill_round_rect(Rect::new(sx, sy + 26 - h, cell - 3, h), 2, c);
        }
    }

    fn paint_pos(&self, cx: &mut PaintCx, a: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(a, t.surface);
        cx.text(FontId::Mono, 34.0, a, Align::Left, t.text, &self.look.pos);
    }
}

impl Widget for PlayTransport {
    fn changed_parts(&self, r: Rect) -> Option<Vec<Rect>> {
        let painted = self.painted.borrow();
        let (at, old, areas) = painted.as_ref()?;
        if *at != r {
            return None;
        }
        if self.look.pos.len() != old.pos.len() {
            // The tempo moves over for the bar's new digit.
            return None;
        }
        let mut now = self.look.clone();
        now.pos.clone_from(&old.pos);
        now.step = old.step;
        now.bar = old.bar;
        if now != *old {
            return None;
        }
        let mut parts = Vec::new();
        if self.look.pos != old.pos {
            parts.push(areas.pos);
        }
        if (self.look.step != old.step || self.look.bar != old.bar) && !areas.dots.is_empty() {
            parts.push(areas.dots);
        }
        Some(parts)
    }

    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        cx.panel(r);
        let inner = PlayTransport::inner(r);
        let tone = self.tone(cx);
        let ph = 34;
        let py = inner.y + (inner.h - ph) / 2;
        let pw = if matches!(self.look.tone, Tone::Dim | Tone::Normal) {
            let w = draw::pill_width(cx, &self.look.state, true);
            let pr = Rect::new(inner.x, py, w, ph);
            cx.canvas.fill_round_rect(pr, ph / 2, t.raised);
            cx.text(
                FontId::Bold,
                14.0,
                pr,
                Align::Center,
                t.text_dim,
                &self.look.state,
            );
            w
        } else {
            pill(cx, inner.x, py, ph, &self.look.state, tone, true)
        };
        let mut x = inner.x + pw + 18;
        // A box as wide as the position's digits (all the same width), so
        // the tempo moves only when the bar gets a digit.
        let digits: String = self
            .look
            .pos
            .chars()
            .map(|c| if c.is_ascii_digit() { '8' } else { c })
            .collect();
        let posw = cx.measure(FontId::Mono, 34.0, &digits);
        let pos = Rect::new(x, inner.y, posw + 4, inner.h);
        self.paint_pos(cx, pos);
        x += posw + 26;
        let bw = cx.text(
            FontId::Bold,
            30.0,
            Rect::new(x, inner.y - 2, 110, inner.h),
            Align::Left,
            t.text,
            &self.look.bpm,
        );
        let bpm_word = cx.text(
            FontId::Bold,
            12.0,
            Rect::new(x + bw + 6, inner.y + 6, 40, inner.h),
            Align::Left,
            t.text_dim,
            "BPM",
        );
        // The view switch at the right end: one tap each.
        let (b, segs) = PlayTransport::switch(r, self.look.views.len());
        // What the pads play, before the switch (Play's card only).
        let mut pads = Rect::default();
        let mut end = b.x - DOTS_GAP;
        if let Some(p) = &self.look.pads {
            let tw = cx.measure(FontId::Bold, 14.0, &p.text()) + 12 + 14 + 12;
            let w = tw.clamp(PADS_MIN_W, PADS_MAX_W);
            let h = REC_CHIP_H.min(inner.h);
            pads = Rect::new(b.x - 12 - w, inner.y + (inner.h - h) / 2, w, h);
            paint_pads_play(cx, pads, p);
            end = pads.x - 12;
        }
        // The REC chip between the tempo and where the bar is. On Play,
        // what the pads play takes the dots' room (the mockup had no view
        // switch): the position and Play's rows say where the bar is.
        let left = x + bw + 6 + bpm_word + 14;
        let gw = if self.look.pads.is_some() {
            0
        } else {
            16 * DOT_CELL + 3 * 6
        };
        let mut rec = Rect::default();
        if let Some(c) = &self.look.rec {
            let right = if gw == 0 { end } else { end - gw - 14 };
            if right - left >= 96 {
                let h = REC_CHIP_H.min(inner.h);
                let cr = Rect::new(left, inner.y + (inner.h - h) / 2, right - left, h);
                paint_rec_chip(cx, cr, c);
                rec = cr;
            }
        }
        cx.canvas.fill_round_rect(b, 12, t.bg.mix(t.surface, 0.3));
        for (i, (sr, name)) in segs.iter().zip(self.look.views).enumerate() {
            let on = i == self.look.view;
            if on {
                cx.canvas.fill_round_rect(*sr, 10, t.accent);
            }
            cx.text(
                FontId::Bold,
                15.0,
                *sr,
                Align::Center,
                if on { t.bg } else { t.text_dim },
                name,
            );
        }
        // Where the bar is: the loop's bars above, the 16ths of this bar below.
        let dots = if gw == 0 {
            Rect::default()
        } else {
            let dots = Rect::new(end - gw, inner.y + 6, gw, 18 + 26);
            self.paint_dots(cx, dots);
            dots
        };
        // The tempo's tap area: its number and "BPM", the card's full height
        // (at least 64 px wide for a finger).
        let bpm = Rect::new(x - 8, r.y, (bw + 52).max(64), r.h);
        *self.painted.borrow_mut() = Some((
            r,
            self.look.clone(),
            TransportAreas {
                pos,
                dots,
                bpm,
                rec,
                pads,
            },
        ));
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let (_, segs) = PlayTransport::switch(r, self.look.views.len());
        let mut out: Vec<Target> = segs
            .iter()
            .zip(self.look.views)
            .enumerate()
            .map(|(i, (s, name))| {
                Target::tap("view", i, *name, *s).disabled_if(i == self.look.view)
            })
            .collect();
        if let Some((pr, _, a)) = self.painted.borrow().as_ref() {
            if *pr == r && !a.bpm.is_empty() {
                out.push(Target::tap(
                    "tempo",
                    0,
                    format!("{} BPM", self.look.bpm),
                    a.bpm,
                ));
            }
            if let (true, false, Some(c)) = (*pr == r, a.rec.is_empty(), &self.look.rec) {
                out.push(Target::tap("rec", 0, c.line(), a.rec));
            }
            if let (true, false, Some(p)) = (*pr == r, a.pads.is_empty(), &self.look.pads) {
                out.push(Target::tap("pads", 0, p.text(), a.pads));
            }
        }
        out
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let (_, segs) = PlayTransport::switch(r, self.look.views.len());
        if let Some(i) = segs.iter().position(|s| s.contains(x, y)) {
            return Some(EventKind::Tab(i));
        }
        let painted = self.painted.borrow();
        let (pr, _, a) = painted.as_ref()?;
        if *pr != r {
            return None;
        }
        if a.bpm.contains(x, y) {
            Some(EventKind::Select(TAP_BPM))
        } else if !a.rec.is_empty() && a.rec.contains(x, y) {
            Some(EventKind::Select(TAP_REC))
        } else if !a.pads.is_empty() && a.pads.contains(x, y) {
            Some(EventKind::Select(TAP_PADS))
        } else {
            None
        }
    }
}

// ------------------------------------------------------------- pads

/// The pad map: 4 x 4 small pads, pad 1 bottom left, each in its sound's
/// colour with its number and name, lit when hit. The heading says what
/// the pads play; while the synth has them it carries the Chromatic | In
/// key switch (Jay, 12:22Z). A tap on a pad gives `Pad { index, down }`
/// (and its lift `down: false`), on the switch `Tab(0)` or `Tab(1)`.
pub struct MiniPads {
    /// The pads' names, colours and light (the app's [`PadGrid`] state).
    pub grid: PadGrid,
    /// "Pads · Bank A".
    pub title: String,
    /// The kit's name, on the right.
    pub right: String,
    pub synth: Option<SynthPads>,
    fingers: Vec<(u64, usize)>,
    painted: RefCell<Option<PaintedPads>>,
}

/// What [`MiniPads`] last drew: where, the pads, its heading and switch.
type PaintedPads = (Rect, Vec<Pad>, String, String, Option<SynthPads>);

impl Default for MiniPads {
    fn default() -> Self {
        MiniPads {
            grid: PadGrid::new(16, 4),
            title: "Pads".into(),
            right: String::new(),
            synth: None,
            fingers: Vec::new(),
            painted: RefCell::new(None),
        }
    }
}

const SWITCH_W: i32 = 150;

impl MiniPads {
    /// The heading line, the switch's halves (while the synth has the
    /// pads) and the pads' area.
    pub fn parts(&self, r: Rect) -> (Rect, Option<[Rect; 2]>, Rect) {
        if self.synth.is_none() {
            let (h, body) = card_head(r);
            return (h, None, body);
        }
        let inner = r.inset_xy(12, 10);
        let (h, body) = inner.take_top(28);
        let sw = Rect::new(h.right() - SWITCH_W, h.y, SWITCH_W.min(h.w), h.h);
        let half = sw.w / 2;
        let halves = [
            Rect::new(sw.x, sw.y, half, sw.h),
            Rect::new(sw.x + half, sw.y, sw.w - half, sw.h),
        ];
        (h, Some(halves), body.take_top(6).1)
    }

    /// Pad `i`'s square in the pads' area `body`.
    pub fn cell(body: Rect, i: usize) -> Rect {
        let gap = 6;
        let w = (body.w - 3 * gap) / 4;
        let h = (body.h - 3 * gap) / 4;
        let col = (i % 4) as i32;
        let row = 3 - (i / 4) as i32;
        Rect::new(body.x + col * (w + gap), body.y + row * (h + gap), w, h)
    }

    fn pad_at(&self, r: Rect, x: i32, y: i32) -> Option<usize> {
        let (_, _, body) = self.parts(r);
        (0..16).find(|&i| MiniPads::cell(body, i).contains(x, y))
    }

    fn paint_pad(&self, cx: &mut PaintCx, body: Rect, i: usize) {
        let t = cx.theme.clone();
        let cr = MiniPads::cell(body, i);
        cx.canvas.fill_rect(cr.inset(-4), t.surface);
        let Some(pad) = self.grid.pads.get(i) else {
            return;
        };
        let h = cr.h;
        if pad.label.is_empty() || pad.color.is_none() {
            cx.canvas
                .fill_round_rect(cr, 7, t.raised.mix(t.surface, 0.5));
            if h >= 34 {
                let ir = cr.inset_xy(7, 0);
                cx.text(
                    FontId::Bold,
                    10.5,
                    ir.take_top(h / 2).0.take_top(4).1,
                    Align::Left,
                    t.text_faint,
                    &format!("{}", i + 1),
                );
            }
            return;
        }
        let c = draw::sound_color(&pad.label)
            .or(pad.color)
            .unwrap_or(t.accent);
        let lit = if pad.held {
            1.0
        } else {
            pad.level.clamp(0.0, 1.0)
        };
        let fill = t.raised.mix(c, 0.28 + 0.72 * lit);
        if lit > 0.0 {
            cx.canvas
                .stroke_round_rect(cr.inset(-3), 10, 2, c.with_alpha((180.0 * lit) as u8));
        }
        cx.canvas.fill_round_rect(cr, 7, fill);
        let ink = if lit > 0.45 { t.bg } else { t.text };
        let ir = cr.inset_xy(7, 0);
        if h >= 34 {
            let (a, b) = ir.take_top(h / 2);
            cx.text(
                FontId::Bold,
                10.5,
                a.take_top(4).1,
                Align::Left,
                ink.with_alpha(150),
                &format!("{}", i + 1),
            );
            cx.text(
                FontId::Sans,
                12.0,
                b.take_bottom(h / 2 - 2).0,
                Align::Left,
                ink,
                &pad.label,
            );
        } else {
            cx.text(FontId::Sans, 12.0, ir, Align::Left, ink, &pad.label);
        }
    }
}

impl Widget for MiniPads {
    fn changed_parts(&self, r: Rect) -> Option<Vec<Rect>> {
        let painted = self.painted.borrow();
        let (at, old, title, right, synth) = painted.as_ref()?;
        if *at != r
            || *title != self.title
            || *right != self.right
            || *synth != self.synth
            || old.len() != self.grid.pads.len()
        {
            return None;
        }
        let (_, _, body) = self.parts(r);
        Some(
            (0..self.grid.pads.len())
                .filter(|&i| old[i] != self.grid.pads[i])
                .map(|i| MiniPads::cell(body, i).inset(-4))
                .collect(),
        )
    }

    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        cx.panel(r);
        let (h, halves, body) = self.parts(r);
        let title_w = match halves {
            Some(hv) => hv[0].x - 8 - h.x,
            None => h.w,
        };
        cx.text(
            FontId::Bold,
            draw::HEAD_PX,
            Rect::new(h.x, h.y, title_w.max(0), h.h),
            Align::Left,
            t.text_dim,
            &self.title.to_uppercase(),
        );
        match (halves, self.synth) {
            (Some(hv), Some(s)) => {
                let pillr = Rect::new(hv[0].x, hv[0].y, hv[0].w + hv[1].w, hv[0].h);
                cx.canvas.fill_round_rect(pillr, pillr.h / 2, t.raised);
                let on = match s.layout {
                    PadLayout::Chromatic => Some(0),
                    PadLayout::InKey => Some(1),
                    PadLayout::Chords => None,
                };
                for (i, (half, name)) in hv.iter().zip(LAYOUT_SWITCH).enumerate() {
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
                        12.5,
                        *half,
                        Align::Center,
                        ink,
                        name,
                    );
                }
            }
            _ => {
                if !self.right.is_empty() {
                    cx.text(
                        FontId::Sans,
                        draw::HEAD_PX,
                        h,
                        Align::Right,
                        t.text_faint,
                        &self.right,
                    );
                }
            }
        }
        for i in 0..self.grid.pads.len().min(16) {
            self.paint_pad(cx, body, i);
        }
        *self.painted.borrow_mut() = Some((
            r,
            self.grid.pads.clone(),
            self.title.clone(),
            self.right.clone(),
            self.synth,
        ));
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let (_, switch, body) = self.parts(r);
        let mut out = Vec::new();
        if let Some(hv) = switch {
            for (i, h) in hv.iter().enumerate() {
                out.push(Target::tap("layout", i, ["Chromatic", "In key"][i], *h));
            }
        }
        for i in 0..16 {
            let label = self
                .grid
                .pads
                .get(i)
                .map_or(String::new(), |p| p.label.clone());
            out.push(Target::tap("pad", i, label, MiniPads::cell(body, i)));
        }
        out
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down => {
                if let Some(hv) = self.parts(r).1 {
                    if let Some(i) = hv.iter().position(|h| h.contains(x, y)) {
                        return Some(EventKind::Tab(i));
                    }
                }
                let i = self.pad_at(r, x, y)?;
                self.fingers.push((finger, i));
                Some(EventKind::Pad {
                    index: i,
                    down: true,
                })
            }
            Touch::Up | Touch::Cancel => {
                let k = self.fingers.iter().position(|(f, _)| *f == finger)?;
                let (_, i) = self.fingers.remove(k);
                Some(EventKind::Pad {
                    index: i,
                    down: false,
                })
            }
            Touch::Move => None,
        }
    }
}

// ------------------------------------------------------------- steps

/// One lane of the pattern: a sound and its steps on the page shown.
#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    /// The pad (or note) the lane plays.
    pub pad: usize,
    pub name: String,
    pub color: Color,
    pub muted: bool,
    /// Each step's velocity on the page (0: empty).
    pub steps: Vec<u8>,
    /// What each step's cell shows besides its velocity, step for step
    /// (missing: nothing).
    pub marks: Vec<Marks>,
    /// On a synth track's scale rows: the pad's note (a chord's root) and
    /// whether it is the key's root, shaded.
    pub note: Option<u8>,
    pub root: bool,
    /// On Play's rows of a bank: the pad's name on the MPC ("A1"), at the
    /// right of the lane's name.
    pub label: Option<String>,
    /// A pad with no sound: its row stays, dashed, with no speaker.
    pub empty: bool,
}

/// A synth track's pattern as scale rows (the sequencer design's mockup
/// B): the lanes are the pads in pitch order, high on top, as many as
/// fit from `low` up; Fold keeps only the notes used.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pitched {
    /// Only the rows with notes.
    pub fold: bool,
    /// The lowest note row shown, counted up from the lowest there is.
    pub low: usize,
}

/// What a step's cell shows besides its velocity, which is the height of
/// its bar (the full step cell of the sequencer design's mockup A).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Marks {
    /// Its locks: a purple dot each, three at most (on an empty step too:
    /// a lock with no note).
    pub locks: u8,
    /// Its trig condition, as a small label at the top left.
    pub cond: Cond,
    /// The chance it plays, in percent: under 100 the cell has a dashed
    /// outline, with the percent when it has no condition (0 is 100).
    pub chance: u8,
    /// Its hits: two or more draw a tick each.
    pub ratchet: u8,
    /// Nudged early (below 0) or late (above 0): a small triangle at the
    /// bottom left or right.
    pub nudge: i8,
    /// Its note lasts into the next step: a bar into it.
    pub tie: bool,
    /// An empty step a note before it lasts over: a thin bar through it.
    pub held_over: bool,
    /// Switched off but kept (the redesign's step 3b; `Note::on`): an
    /// outline in the lane's colour with "off" in it, its lock dots still
    /// there, since a tap brings it all back.
    pub off: bool,
}

/// The pattern: a lane per sound (as many as fit, the ones that play
/// first, in pad order), 16 steps a page in groups of four, the playhead's
/// column lit, the step picked outlined. A tap on a step gives
/// `Cell { row: pad, col: step }`, on a lane's speaker `Toggle(pad)`. The
/// head's controls (Jay after live test 3: 32, 64 and 128 steps) give
/// `Tab(HEAD_PREV)` and `Tab(HEAD_NEXT)` for the page, `Tab(HEAD_SHORTER)`
/// and `Tab(HEAD_LONGER)` for the length.
///
/// On a synth track ([`Pitched`]) the lanes are scale rows, high on top,
/// each named by its note with the roots shaded; a finger on a row's name
/// plays it (`Pad { index: pad, down }`, down and up), and the head adds
/// Fold (`Tab(HEAD_FOLD)`) and the octave's ‹ and › (`Tab(HEAD_LOWER)`,
/// `Tab(HEAD_HIGHER)`).
#[derive(Default)]
pub struct StepGrid {
    /// "Pattern 1 · Drums".
    pub title: String,
    /// "1/16 · 1 bar".
    pub right: String,
    /// Every lane there could be, most wanted first (sounds with notes,
    /// the pad hit last, then the kit's other sounds).
    pub lanes: Vec<Lane>,
    /// Steps on the page (16, fewer at the end of a short clip).
    pub steps: usize,
    /// The playhead's step on the page.
    pub play: Option<usize>,
    /// The step picked: (pad, step on the page).
    pub selected: Option<(usize, usize)>,
    /// The page shown (from 0) and how many there are.
    pub page: usize,
    pub pages: usize,
    /// The clip's length in steps; 0 with no clip yet.
    pub length: usize,
    /// STEP SEQ: the lane whose steps the pads are (a pad), outlined and
    /// tagged "On the pads".
    pub on_pads: Option<usize>,
    /// STEP SEQ: the step pads held, (pad, step on the page), outlined as
    /// the step picked is.
    pub held: Vec<(usize, usize)>,
    /// The Arm pill at the left of the heading: the notes as a REC target.
    pub arm: Option<ArmPill>,
    /// A synth track's scale rows; None for a kit's sounds.
    pub pitched: Option<Pitched>,
    /// Play's head over the armed track's pads, its lanes a bank's 16 in
    /// pad order; None on the Sequencer's Steps.
    pub bank: Option<BankHead>,
    /// The finger on a scale row's name, and its pad, until it lifts.
    sounding: Option<(u64, usize)>,
    /// A finger on Play's rows: a tap until it moves, then a swipe.
    swipe: Option<Swipe>,
    painted: RefCell<Option<(Rect, StepLook)>>,
}

/// Play's head over the armed track's pads (the Play, Pads and Sequencer
/// redesign's board 1, Jay 18 Nov): the Arm pill, then the track with its
/// switch, the kit's banks A to H, the pages with Follow, and the way to
/// the Sequencer tab, in place of the title and the length.
#[derive(Clone, Debug, PartialEq)]
pub struct BankHead {
    /// The track's name, and "Track 1" under it, by a dot in its colour.
    pub track: String,
    pub sub: String,
    pub color: Color,
    /// Another track the switch can go to.
    pub switch: bool,
    /// The kit's bank the rows are (0 is A); None on the synth's pads,
    /// which have no banks.
    pub bank: Option<usize>,
    /// The page follows the playhead.
    pub follow: bool,
    /// The first of the bank's rows shown (a swipe moves it).
    pub top: usize,
}

/// A finger on Play's rows: where it went down, the top row then, the
/// rows it has moved them, and the tap it gives if it lifts unmoved.
#[derive(Clone, Debug, PartialEq)]
struct Swipe {
    finger: u64,
    y: i32,
    top: usize,
    moved: i32,
    swiping: bool,
    tap: Option<EventKind>,
}

/// Play's head: the track switch's ‹ and ›, and Follow, as `Tab`
/// indices; bank A's button is `HEAD_BANK`, B to H after it. (Its
/// "Sequencer tab ›" button went on 18 Nov 2026: the Sequencer tab is in
/// the top bar, Jay 07:28Z.)
pub const HEAD_TRACK_PREV: usize = 7;
pub const HEAD_TRACK_NEXT: usize = 8;
pub const HEAD_FOLLOW: usize = 9;
pub const HEAD_BANK: usize = 16;
/// A swipe on Play's rows: `Step { index: ROWS, delta }`, the rows to
/// move the first one shown by.
pub const ROWS: usize = 0;
/// A finger moves this far before it swipes.
const SWIPE_PX: i32 = 12;
/// Play's names are wider: the pad's name on the MPC beside the sound's.
const BANK_NAMES_W: i32 = 232;
const BANK_BTN_W: i32 = 34;
const RAIL_W: i32 = 6;

/// The head's controls, as `Tab` indices.
pub const HEAD_PREV: usize = 0;
pub const HEAD_NEXT: usize = 1;
pub const HEAD_SHORTER: usize = 2;
pub const HEAD_LONGER: usize = 3;
/// Scale rows: Fold, and the octave down and up.
pub const HEAD_FOLD: usize = 4;
pub const HEAD_LOWER: usize = 5;
pub const HEAD_HIGHER: usize = 6;
/// The most steps the pattern's + makes, and the fewest its - leaves.
pub const MAX_STEPS: usize = 128;
pub const MIN_STEPS: usize = 16;

/// What a paint drew, to find what changed.
#[derive(Clone, Debug, PartialEq)]
struct StepLook {
    title: String,
    right: String,
    lanes: Vec<Lane>,
    steps: usize,
    play: Option<usize>,
    selected: Option<(usize, usize)>,
    page: (usize, usize, usize),
    on_pads: Option<usize>,
    held: Vec<(usize, usize)>,
    arm: Option<ArmPill>,
    pitched: Option<Pitched>,
    bank: Option<BankHead>,
}

/// The pattern head's controls, right to left: + and - round the length,
/// then › and ‹ round the page number; on scale rows, the octave's ‹ and
/// › round its name, then Fold. Each answers over the head's whole
/// height.
struct Head {
    title: Rect,
    prev: Rect,
    page: Rect,
    next: Rect,
    shorter: Rect,
    length: Rect,
    longer: Rect,
    /// Scale rows only (empty otherwise).
    fold: Rect,
    lower: Rect,
    octave: Rect,
    higher: Rect,
}

/// A head button's width.
const HEAD_BTN: i32 = 56;

impl Head {
    fn of(r: Rect, pitched: bool) -> Head {
        let (h, body) = card_head(r);
        let band = Rect::new(h.x, r.y, h.w, body.y - 2 - r.y);
        let mut x = h.right();
        let mut take = |w: i32| {
            x -= w;
            Rect::new(x, band.y, w, band.h)
        };
        let longer = take(HEAD_BTN);
        let length = take(78);
        let shorter = take(HEAD_BTN);
        take(14);
        let next = take(HEAD_BTN);
        let page = take(52);
        let prev = take(HEAD_BTN);
        let none = Rect::new(prev.x, band.y, 0, 0);
        let (fold, lower, octave, higher) = if pitched {
            take(14);
            let higher = take(HEAD_BTN);
            let octave = take(64);
            let lower = take(HEAD_BTN);
            take(6);
            let fold = take(68);
            (fold, lower, octave, higher)
        } else {
            (none, none, none, none)
        };
        let left = if pitched { fold.x } else { prev.x };
        Head {
            title: Rect::new(h.x, h.y, (left - 12 - h.x).max(0), h.h),
            prev,
            page,
            next,
            shorter,
            length,
            longer,
            fold,
            lower,
            octave,
            higher,
        }
    }

    /// The control under (`x`, `y`), as its `Tab` index.
    fn at(&self, x: i32, y: i32) -> Option<usize> {
        [
            (self.prev, HEAD_PREV),
            (self.next, HEAD_NEXT),
            (self.shorter, HEAD_SHORTER),
            (self.longer, HEAD_LONGER),
            (self.fold, HEAD_FOLD),
            (self.lower, HEAD_LOWER),
            (self.higher, HEAD_HIGHER),
        ]
        .into_iter()
        .find(|(b, _)| b.contains(x, y))
        .map(|(_, i)| i)
    }
}

/// Play's head over the armed track's pads ([`BankHead`]), left to
/// right after the Arm pill: the track with ‹ and › at its ends (each half
/// of it goes that way), the banks A to H, the page with ‹ and ›, then
/// Follow. The room the Sequencer tab's button had (18 Nov 2026) goes to
/// the page's ‹ and › (60 px, a fingertip), Follow, then the banks.
struct BankLayout {
    track: Rect,
    banks: [Rect; 8],
    prev: Rect,
    page: Rect,
    next: Rect,
    follow: Rect,
}

impl BankLayout {
    fn of(r: Rect) -> BankLayout {
        let (h, body) = card_head(r);
        let band = Rect::new(h.x, r.y, h.w, body.y - 2 - r.y);
        let mut x = h.x + ARM_PILL_W + 8;
        // The narrowest each can be, and the room left beside them.
        let least = 196 + 10 + 8 * BANK_BTN_W + 7 * 4 + 10 + 40 + 92 + 40 + 8 + 76;
        let mut slack = (h.right() - x - least).max(0);
        let mut grow = |want: i32| {
            let g = want.min(slack);
            slack -= g;
            g
        };
        let arrow = 40 + grow(40) / 2;
        let follow_w = 76 + grow(20);
        let bank_w = BANK_BTN_W + grow(8 * 14) / 8;
        let mut take = |w: i32, gap: i32| {
            let b = Rect::new(x, band.y, w, band.h);
            x += w + gap;
            b
        };
        let track = take(196, 10);
        let mut banks = [Rect::new(0, 0, 0, 0); 8];
        for (i, b) in banks.iter_mut().enumerate() {
            *b = take(bank_w, if i == 7 { 10 } else { 4 });
        }
        let prev = take(arrow, 0);
        let page = take(92, 0);
        let next = take(arrow, 8);
        let follow = take(follow_w, 8);
        BankLayout {
            track,
            banks,
            prev,
            page,
            next,
            follow,
        }
    }

    /// The control under (`x`, `y`), as its `Tab` index.
    fn at(&self, x: i32, y: i32) -> Option<usize> {
        if self.track.contains(x, y) {
            let left = x < self.track.x + self.track.w / 2;
            return Some(if left {
                HEAD_TRACK_PREV
            } else {
                HEAD_TRACK_NEXT
            });
        }
        if let Some(i) = self.banks.iter().position(|b| b.contains(x, y)) {
            return Some(HEAD_BANK + i);
        }
        [
            (self.prev, HEAD_PREV),
            (self.next, HEAD_NEXT),
            (self.follow, HEAD_FOLLOW),
        ]
        .into_iter()
        .find(|(b, _)| b.contains(x, y))
        .map(|(_, i)| i)
    }
}

/// Lanes are about this tall at least.
const LANE_MIN_H: i32 = 34;
const LANE_MAX_H: i32 = 44;
const LANE_GAP: i32 = 5;
const NAMES_W: i32 = 132;

/// Where a pattern's parts sit in its rectangle.
struct GridGeom {
    names: Rect,
    grid: Rect,
    rows: usize,
    rh: i32,
    cw: i32,
}

impl GridGeom {
    fn col_x(&self, s: usize) -> i32 {
        let (group, cg) = (10, 4);
        self.grid.x + s as i32 * (self.cw + cg) + (s as i32 / 4) * (group - cg)
    }

    fn row_y(&self, k: usize) -> i32 {
        self.grid.y + k as i32 * (self.rh + LANE_GAP)
    }

    fn cell(&self, k: usize, s: usize) -> Rect {
        Rect::new(self.col_x(s), self.row_y(k), self.cw, self.rh)
    }

    /// Lane `k`'s mute button as drawn: a box with the speaker in it at
    /// the right of the names.
    fn mute_button(&self, k: usize) -> Rect {
        Rect::new(self.names.right() - 50, self.row_y(k), 40, self.rh)
    }

    /// Where lane `k` answers a tap to mute: its whole name and button,
    /// the lane's full height with half the gap above and below (Jay
    /// after live test 3: bigger mutes).
    fn mute_hit(&self, k: usize) -> Rect {
        let half = LANE_GAP / 2;
        Rect::new(
            self.names.x,
            self.row_y(k) - half,
            self.names.w - 4,
            self.rh + LANE_GAP,
        )
    }
}

impl StepGrid {
    fn geom(&self, r: Rect) -> GridGeom {
        let (_, body) = card_head(r);
        // Play's rows: wider names, and the rail at the right.
        let (names, grid) = match self.bank {
            Some(_) => {
                let (names, grid) = body.take_left(BANK_NAMES_W);
                (names, grid.take_right(RAIL_W + 8).1)
            }
            None => body.take_left(NAMES_W),
        };
        let fit = ((grid.h + LANE_GAP) / (LANE_MIN_H + LANE_GAP)).clamp(1, 8) as usize;
        let rows = fit.min(self.lanes.len()).max(1);
        let rh = ((grid.h - (rows as i32 - 1) * LANE_GAP) / rows as i32).min(LANE_MAX_H);
        let cw = (grid.w - 3 * 10 - 15 * 4) / 16;
        GridGeom {
            names,
            grid,
            rows,
            rh,
            cw,
        }
    }

    /// The lanes shown in `rows` rows: the most wanted, in pad order. On
    /// scale rows, the line (if it is there) on top, then the notes from
    /// the window's lowest up, high on top.
    fn shown(&self, rows: usize) -> Vec<&Lane> {
        if let (Some(b), None) = (&self.bank, self.pitched) {
            // Play's rows: the bank's pads in order, from the swipe's top.
            let top = b.top.min(self.lanes.len().saturating_sub(rows));
            return self.lanes.iter().skip(top).take(rows).collect();
        }
        let Some(p) = self.pitched else {
            let mut v: Vec<&Lane> = self.lanes.iter().take(rows).collect();
            v.sort_by_key(|l| l.pad);
            return v;
        };
        let (mut top, notes): (Vec<&Lane>, Vec<&Lane>) =
            self.lanes.iter().partition(|l| l.pad >= 16);
        let n = rows.saturating_sub(top.len());
        let low = p.low.min(notes.len().saturating_sub(n));
        top.extend(notes.iter().skip(low).take(n).rev());
        top
    }

    /// How many note rows the scale rows show in `r` (the line's taken
    /// off), for the octave's ‹ and ›.
    pub fn note_rows(&self, r: Rect) -> usize {
        let top = self.lanes.iter().filter(|l| l.pad >= 16).count();
        self.geom(r).rows.saturating_sub(top).max(1)
    }

    /// The octave's name on scale rows: the lowest row's ("Oct 3", for
    /// C3 up).
    fn octave(&self, rows: usize) -> String {
        self.shown(rows)
            .iter()
            .filter_map(|l| l.note)
            .min()
            .map_or_else(String::new, |n| format!("Oct {}", i32::from(n) / 12 - 1))
    }

    /// Scale rows: whether the octave's ‹ and › can move the window.
    fn octave_usable(&self, rows: usize) -> (bool, bool) {
        let Some(p) = self.pitched else {
            return (false, false);
        };
        let top = self.lanes.iter().filter(|l| l.pad >= 16).count();
        let notes = self.lanes.len() - top;
        let n = rows.saturating_sub(top);
        let low = p.low.min(notes.saturating_sub(n));
        (low > 0, low + n < notes)
    }

    fn look(&self) -> StepLook {
        StepLook {
            title: self.title.clone(),
            right: self.right.clone(),
            lanes: self.lanes.clone(),
            steps: self.steps,
            play: self.play,
            selected: self.selected,
            page: (self.page, self.pages, self.length),
            on_pads: self.on_pads,
            held: self.held.clone(),
            arm: self.arm,
            pitched: self.pitched,
            bank: self.bank.clone(),
        }
    }

    /// The middle of head control `k` (`HEAD_PREV` ...) with scale rows
    /// (`pitched`) or without, for the app's tests.
    #[cfg(test)]
    pub fn head_at(r: Rect, k: usize, pitched: bool) -> (i32, i32) {
        let h = Head::of(r, pitched);
        let b = [
            h.prev, h.next, h.shorter, h.longer, h.fold, h.lower, h.higher,
        ][k];
        (b.x + b.w / 2, b.y + b.h / 2)
    }

    /// Which head controls can be used now: (‹, ›, -, +).
    fn usable(&self) -> (bool, bool, bool, bool) {
        let clip = self.length > 0;
        (
            self.page > 0,
            self.page + 1 < self.pages,
            clip && self.length > MIN_STEPS,
            clip && self.length < MAX_STEPS,
        )
    }

    /// The head: the title on the left, then the details, the page with
    /// ‹ and ›, and the length with - and +; on scale rows, Fold and the
    /// octave with its ‹ and › before the page. The details go where they
    /// would crowd the title.
    fn paint_head(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let hd = Head::of(r, self.pitched.is_some());
        let (h, _) = card_head(r);
        let title_w = cx.measure(FontId::Bold, draw::HEAD_PX, &self.title.to_uppercase())
            + if self.arm.is_some() {
                ARM_PILL_W + 10
            } else {
                0
            };
        let right_w = cx.measure(FontId::Sans, draw::HEAD_PX, &self.right);
        if !self.right.is_empty() && title_w + 16 + right_w <= hd.title.w {
            cx.text(
                FontId::Sans,
                draw::HEAD_PX,
                hd.title,
                Align::Right,
                t.text_faint,
                &self.right,
            );
        }
        let (prev, next, shorter, longer) = self.usable();
        let mid = h.y + h.h / 2;
        let button = |cx: &mut PaintCx, b: Rect, label: &str, usable: bool| {
            let face = Rect::new(b.x + 4, mid - 13, b.w - 8, 26);
            cx.canvas.fill_round_rect(face, 8, t.raised);
            cx.text(
                FontId::Bold,
                16.0,
                face,
                Align::Center,
                t.button_ink(usable),
                label,
            );
        };
        button(cx, hd.prev, "‹", prev);
        button(cx, hd.next, "›", next);
        button(cx, hd.shorter, "−", shorter);
        button(cx, hd.longer, "+", longer);
        let row = |b: Rect| Rect::new(b.x, h.y, b.w, h.h);
        if let Some(p) = self.pitched {
            let rows = self.geom(r).rows;
            let (lower, higher) = self.octave_usable(rows);
            button(cx, hd.lower, "‹", lower);
            button(cx, hd.higher, "›", higher);
            cx.text(
                FontId::Bold,
                draw::HEAD_PX,
                row(hd.octave),
                Align::Center,
                t.text_dim,
                &self.octave(rows),
            );
            // Fold: lit while on.
            let face = Rect::new(hd.fold.x + 4, mid - 13, hd.fold.w - 8, 26);
            let (fill, ink) = if p.fold {
                (t.accent, t.bg)
            } else {
                (t.raised, t.text)
            };
            cx.canvas.fill_round_rect(face, 8, fill);
            cx.text(FontId::Bold, 14.0, face, Align::Center, ink, "Fold");
        }
        let page = format!("{}/{}", self.page + 1, self.pages.max(1));
        cx.text(
            FontId::Bold,
            draw::HEAD_PX,
            row(hd.page),
            Align::Center,
            t.text_dim,
            &page,
        );
        let length = format!("{} steps", self.length.max(MIN_STEPS));
        cx.text(
            FontId::Bold,
            draw::HEAD_PX,
            row(hd.length),
            Align::Center,
            t.text_dim,
            &length,
        );
    }

    /// Play's head ([`BankHead`]): the track with ‹ and ›, the banks (the
    /// one shown lit), the steps shown of how many with ‹ and ›, and
    /// Follow (lit while on).
    fn paint_bank_head(&self, cx: &mut PaintCx, r: Rect, b: &BankHead) {
        let t = cx.theme.clone();
        let l = BankLayout::of(r);
        let (h, _) = card_head(r);
        let mid = h.y + h.h / 2;
        let face = |b: Rect| Rect::new(b.x, mid - 14, b.w, 28);
        // The track: a dot in its colour, its name over "Track 1".
        let tr = face(l.track);
        cx.canvas.fill_round_rect(tr, 8, t.raised);
        let ink = t.button_ink(b.switch);
        cx.text(
            FontId::Bold,
            16.0,
            tr.take_left(22).0,
            Align::Center,
            ink,
            "‹",
        );
        cx.text(
            FontId::Bold,
            16.0,
            tr.take_right(22).0,
            Align::Center,
            ink,
            "›",
        );
        cx.canvas
            .fill_round_rect(Rect::new(tr.x + 26, mid - 4, 8, 8), 4, b.color);
        let text_r = Rect::new(tr.x + 40, tr.y, tr.w - 64, tr.h);
        if b.sub.is_empty() {
            cx.text(FontId::Bold, 14.0, text_r, Align::Left, t.text, &b.track);
        } else {
            let (name_r, sub_r) = text_r.take_top(16);
            cx.text(FontId::Bold, 14.0, name_r, Align::Left, t.text, &b.track);
            cx.text(FontId::Sans, 10.0, sub_r, Align::Left, t.text_dim, &b.sub);
        }
        // The banks: A to H, the one shown lit.
        for (i, br) in l.banks.iter().enumerate() {
            let f = face(*br);
            let lit = b.bank == Some(i);
            let (fill, ink) = if lit {
                (t.accent, t.bg)
            } else {
                (t.raised, t.button_ink(b.bank.is_some()))
            };
            cx.canvas.fill_round_rect(f, 8, fill);
            let letter = ((b'A' + i as u8) as char).to_string();
            cx.text(FontId::Bold, 14.0, f, Align::Center, ink, &letter);
        }
        // The page: the steps shown, of how many.
        let (prev, next, _, _) = self.usable();
        for (br, label, usable) in [(l.prev, "‹", prev), (l.next, "›", next)] {
            let f = face(br).inset_xy(3, 0);
            cx.canvas.fill_round_rect(f, 8, t.raised);
            cx.text(
                FontId::Bold,
                16.0,
                f,
                Align::Center,
                t.button_ink(usable),
                label,
            );
        }
        let total = self.length.max(MIN_STEPS);
        let from = (self.page * 16 + 1).min(total);
        let to = (self.page * 16 + self.steps.max(1)).min(total);
        cx.text(
            FontId::Bold,
            draw::HEAD_PX,
            Rect::new(l.page.x, h.y, l.page.w, h.h),
            Align::Center,
            t.text_dim,
            &format!("{from}–{to} of {total}"),
        );
        // Follow, lit while on.
        let f = face(l.follow);
        let (fill, ink) = if b.follow {
            (t.accent, t.bg)
        } else {
            (t.raised, t.text)
        };
        cx.canvas.fill_round_rect(f, 8, fill);
        cx.text(FontId::Bold, 13.0, f, Align::Center, ink, "Follow");
    }

    /// The lanes shown on a page this tall (for the app's tests).
    #[cfg(test)]
    pub fn lanes_shown(&self, r: Rect) -> Vec<usize> {
        let g = self.geom(r);
        self.shown(g.rows).iter().map(|l| l.pad).collect()
    }

    /// The middle of row `k`'s name, left of its mute (for the app's tests).
    #[cfg(test)]
    pub fn name_at(&self, r: Rect, k: usize) -> (i32, i32) {
        let g = self.geom(r);
        (g.names.x + 20, g.row_y(k) + g.rh / 2)
    }

    /// The middle of row `k`'s step `s` (for the app's tests).
    #[cfg(test)]
    pub fn step_at(&self, r: Rect, k: usize, s: usize) -> (i32, i32) {
        let c = self.geom(r).cell(k, s);
        (c.x + c.w / 2, c.y + c.h / 2)
    }

    /// The middle of Play's head control `k` (`HEAD_TRACK_PREV` ...,
    /// `HEAD_BANK + i`), for the app's tests.
    #[cfg(test)]
    pub fn bank_head_at(r: Rect, k: usize) -> (i32, i32) {
        let l = BankLayout::of(r);
        let b = match k {
            HEAD_TRACK_PREV => l.track.take_left(l.track.w / 2).0,
            HEAD_TRACK_NEXT => l.track.take_right(l.track.w / 2).0,
            HEAD_PREV => l.prev,
            HEAD_NEXT => l.next,
            HEAD_FOLLOW => l.follow,
            k => l.banks[k - HEAD_BANK],
        };
        (b.x + b.w / 2, b.y + b.h / 2)
    }

    /// How many rows show in `r` (for the app's tests).
    #[cfg(test)]
    pub fn rows_shown(&self, r: Rect) -> usize {
        self.geom(r).rows
    }
}

/// One step's cell: empty (the first of each beat a little lighter), or
/// its note as a bar as tall as its velocity over a dim cell in the lane's
/// colour, with its marks: the condition or the chance at the top left,
/// the ratchet's ticks after it, a dot per lock at the top right, the
/// nudge's triangle, a dashed outline for a chance under 100 %, a bar into
/// the next step for a note that lasts over it.
#[allow(clippy::too_many_arguments)]
fn paint_cell(
    cx: &mut PaintCx,
    sr: Rect,
    v: u8,
    m: &Marks,
    c: Color,
    muted: bool,
    lit: bool,
    beat: bool,
) {
    let t = cx.theme.clone();
    let tint = |a: f32| {
        let f = t.raised.mix(c, if muted { a * 0.4 } else { a });
        if lit {
            f.mix(Color::rgb(0xFFFFFF), 0.35)
        } else {
            f
        }
    };
    if v > 0 && m.off {
        // Off but kept (board 1's "off" step): the empty step's fill, an
        // outline and the word in the lane's colour, and its locks' dots.
        let base = if beat {
            t.raised
        } else {
            t.raised.mix(t.surface, 0.45)
        };
        cx.canvas.fill_round_rect(sr, 5, base);
        let ink = if muted {
            t.raised.mix(c, 0.5)
        } else {
            c.mix(t.text, 0.15)
        };
        cx.canvas.stroke_round_rect(sr, 5, 2, ink);
        cx.text(FontId::Bold, 12.0, sr, Align::Center, ink, "off");
        for k in 0..i32::from(m.locks.min(3)) {
            let dx = sr.right() - 8 - k * 9;
            dot(cx, dx, sr.y + 8, 4, LOCKED);
        }
        return;
    }
    if v == 0 {
        let base = if beat {
            t.raised
        } else {
            t.raised.mix(t.surface, 0.45)
        };
        cx.canvas.fill_round_rect(sr, 5, base);
        if m.held_over {
            let bar = Rect::new(sr.x - 4, sr.y + sr.h / 2 - 3, sr.w * 2 / 3 + 4, 6);
            cx.canvas.fill_rect(bar, tint(0.55));
        }
    } else {
        // The cell dim in the lane's colour, the velocity's bar bright
        // over its bottom.
        cx.canvas.fill_round_rect(sr, 5, tint(0.35));
        let bh = ((sr.h as f32 * f32::from(v) / 127.0).round() as i32).clamp(4, sr.h);
        cx.canvas
            .fill_round_rect(Rect::new(sr.x, sr.bottom() - bh, sr.w, bh), 5, tint(0.9));
        if m.tie {
            // Across the gap into the next step.
            let bar = Rect::new(sr.right() - 2, sr.y + sr.h / 2 - 3, 16, 6);
            cx.canvas.fill_rect(bar, tint(0.9));
        }
    }
    if v > 0 && m.chance > 0 && m.chance < 100 {
        let o = sr.inset(1);
        let (x0, y0, x1, y1) = (
            o.x as f32,
            o.y as f32,
            o.right() as f32 - 1.0,
            o.bottom() as f32 - 1.0,
        );
        let mut sh = mpfree_ui::Shape::new(sr);
        sh.dashed(
            &[(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)],
            2.0,
            5.0,
        );
        cx.canvas.blend_shape(&sh, c.mix(Color::rgb(0xFFFFFF), 0.3));
    }
    let ink = t.bg;
    // The top left: the condition, else the chance, then the ticks.
    let label = if m.cond != Cond::Always {
        m.cond.to_string()
    } else if v > 0 && m.chance > 0 && m.chance < 100 {
        format!("{}%", m.chance)
    } else {
        String::new()
    };
    let mut x = sr.x + 4;
    if !label.is_empty() {
        let w = (label.len() as i32 * 7 + 6).min(sr.w - 8);
        let chip = Rect::new(x, sr.y + 3, w, 14);
        cx.canvas
            .fill_round_rect(chip, 3, Color::rgb(0xFFFFFF).with_alpha(220));
        cx.text(FontId::Bold, 10.0, chip, Align::Center, ink, &label);
        x += w + 3;
    }
    if v > 0 && m.ratchet > 1 {
        for k in 0..i32::from(m.ratchet.min(4)) {
            cx.canvas
                .fill_rect(Rect::new(x + k * 4, sr.y + 4, 2, 9), ink.with_alpha(200));
        }
    }
    // The top right: a dot per lock.
    for k in 0..i32::from(m.locks.min(3)) {
        let dx = sr.right() - 8 - k * 9;
        dot(cx, dx, sr.y + 8, 4, LOCKED);
    }
    // The nudge: a triangle pointing the way it goes.
    if v > 0 && m.nudge != 0 {
        // Nine rows, widest (the tip) in the middle: ◀ at the bottom left
        // for early, ▶ at the bottom right for late.
        let y = sr.bottom() - 13;
        for row in 0..9 {
            let w = 5 - (row - 4i32).abs();
            let x = if m.nudge < 0 {
                sr.x + 9 - w
            } else {
                sr.right() - 9
            };
            cx.canvas.fill_rect(Rect::new(x, y + row, w, 1), ink);
        }
    }
}

impl Widget for StepGrid {
    fn changed_parts(&self, r: Rect) -> Option<Vec<Rect>> {
        let painted = self.painted.borrow();
        let (at, old) = painted.as_ref()?;
        if *at != r {
            return None;
        }
        let now = self.look();
        // Only the playhead moved: its old and new columns.
        let mut moved = now.clone();
        moved.play = old.play;
        if moved != *old {
            return None;
        }
        let g = self.geom(r);
        let col = |s: usize| {
            Rect::new(
                g.col_x(s) - 4,
                g.grid.y - 4,
                g.cw + 8,
                g.row_y(g.rows - 1) + g.rh + 8 - g.grid.y,
            )
        };
        Some(old.play.into_iter().chain(now.play).map(col).collect())
    }

    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        match self.arm {
            // Play's head has no title: the track says it.
            Some(pill) if self.bank.is_some() => {
                card(cx, r, "", "");
                paint_arm(cx, r, pill);
            }
            None if self.bank.is_some() => {
                card(cx, r, "", "");
            }
            // The Arm pill takes the heading's left, the title after it.
            Some(pill) => {
                card(cx, r, "", "");
                let (h, _) = card_head(r);
                let x = h.x + ARM_PILL_W + 10;
                cx.text(
                    FontId::Bold,
                    draw::HEAD_PX,
                    Rect::new(
                        x,
                        h.y,
                        Head::of(r, self.pitched.is_some()).title.right() - x,
                        h.h,
                    ),
                    Align::Left,
                    t.text_dim,
                    &self.title.to_uppercase(),
                );
                paint_arm(cx, r, pill);
            }
            None => {
                card(cx, r, &self.title, "");
            }
        }
        match &self.bank {
            Some(b) => self.paint_bank_head(cx, r, b),
            None => self.paint_head(cx, r),
        }
        let g = self.geom(r);
        let lanes = self.shown(g.rows);
        if let Some(p) = self.play.filter(|&p| p < self.steps) {
            let px = g.col_x(p);
            let h = g.row_y(lanes.len().max(1) - 1) + g.rh - g.grid.y;
            cx.canvas.fill_round_rect(
                Rect::new(px - 3, g.grid.y - 3, g.cw + 6, h + 6),
                6,
                t.accent.with_alpha(40),
            );
        }
        for (k, lane) in lanes.iter().enumerate() {
            let y = g.row_y(k);
            let mb = g.mute_button(k);
            let nr = Rect::new(g.names.x, y, mb.x - 4 - g.names.x, g.rh);
            let c = lane.color;
            let on_pads = self.on_pads == Some(lane.pad);
            if self.pitched.is_some() && lane.pad < 16 {
                // A scale row: its note on a key, the roots shaded and
                // tagged; a finger on it plays it.
                let key = Rect::new(g.names.x, y, g.names.w - 10, g.rh);
                let fill = if lane.root {
                    t.raised.mix(c, 0.45)
                } else {
                    t.raised
                };
                cx.canvas.fill_round_rect(key, 6, fill);
                // The tag only on a root's key, so a chord's name has the
                // whole key elsewhere.
                let inner = key.inset_xy(12, 0);
                let (tag, name_r) = if lane.root {
                    inner.take_right(36)
                } else {
                    (Rect::new(inner.right(), inner.y, 0, 0), inner)
                };
                cx.text(FontId::Sans, 15.0, name_r, Align::Left, t.text, &lane.name);
                if lane.root {
                    cx.text(FontId::Sans, 11.0, tag, Align::Right, t.text_dim, "root");
                }
            } else if lane.empty {
                // A pad with no sound keeps its row: dashed, "empty".
                let box_r = Rect::new(g.names.x, y, g.names.w - 10, g.rh);
                dashed_outline(cx, box_r, t.outline);
                cx.text(
                    FontId::Sans,
                    14.0,
                    box_r.inset_xy(12, 0),
                    Align::Left,
                    t.text_faint,
                    "empty",
                );
                if let Some(label) = &lane.label {
                    cx.text(
                        FontId::Bold,
                        12.0,
                        box_r.inset_xy(12, 0),
                        Align::Right,
                        t.text_faint,
                        label,
                    );
                }
            } else {
                cx.canvas
                    .fill_round_rect(Rect::new(nr.x, y + g.rh / 2 - 5, 10, 10), 3, c);
                let ink = if lane.muted { t.text_faint } else { t.text };
                // Play's rows: the pad's name on the MPC before the speaker.
                let nr = match &lane.label {
                    Some(label) => {
                        let (tag, rest) = nr.take_right(40);
                        cx.text(FontId::Bold, 12.0, tag, Align::Right, t.text_dim, label);
                        rest
                    }
                    None => nr,
                };
                // STEP SEQ's lane: its name over a small "ON THE PADS".
                let name_r = if on_pads {
                    let (top, bottom) = nr.take_left(18).1.take_top(g.rh * 3 / 5);
                    cx.text(
                        FontId::Bold,
                        10.0,
                        bottom,
                        Align::Left,
                        t.accent,
                        "ON THE PADS",
                    );
                    top
                } else {
                    nr.take_left(18).1
                };
                cx.text(FontId::Sans, 14.0, name_r, Align::Left, ink, &lane.name);
                // The mute button: a box round the speaker, red-tinted while
                // muted.
                if lane.muted {
                    cx.canvas.fill_round_rect(mb, 6, t.raised.mix(t.bad, 0.3));
                } else {
                    cx.canvas.fill_round_rect(mb, 6, t.raised);
                }
                cx.canvas.stroke_round_rect(mb, 6, 1, t.outline);
                speaker(cx, mb.x + mb.w / 2 + 1, mb.y + mb.h / 2, t.text, lane.muted);
            }
            for s in 0..16 {
                let sr = g.cell(k, s);
                if s >= self.steps {
                    continue;
                }
                let v = lane.steps.get(s).copied().unwrap_or(0);
                let m = lane.marks.get(s).copied().unwrap_or_default();
                let lit = self.play == Some(s) && !lane.muted;
                paint_cell(cx, sr, v, &m, c, lane.muted, lit, s % 4 == 0);
                if self.selected == Some((lane.pad, s)) || self.held.contains(&(lane.pad, s)) {
                    cx.canvas.stroke_round_rect(sr.inset(-3), 7, 2, t.text);
                }
            }
            if on_pads {
                // The whole lane, names to its last step, inside the gap.
                let right = g.col_x(15) + g.cw;
                let lane_r = Rect::new(g.names.x - 2, y - 2, right + 2 - (g.names.x - 2), g.rh + 4);
                cx.canvas.stroke_round_rect(lane_r, 8, 2, t.accent);
            }
        }
        // Play's rows: a rail at the right, where the rows shown sit
        // among the bank's.
        if self.bank.is_some() && self.pitched.is_none() && self.lanes.len() > g.rows {
            let n = self.lanes.len();
            let top = self.bank.as_ref().map_or(0, |b| b.top.min(n - g.rows));
            let rail_h = g.row_y(g.rows - 1) + g.rh - g.grid.y;
            let rail = Rect::new(g.grid.right() + 8, g.grid.y, RAIL_W, rail_h);
            cx.canvas.fill_round_rect(rail, 3, t.raised);
            let y0 = rail.y + rail_h * top as i32 / n as i32;
            let y1 = rail.y + rail_h * (top + g.rows) as i32 / n as i32;
            cx.canvas
                .fill_round_rect(Rect::new(rail.x, y0, RAIL_W, y1 - y0), 3, t.text_dim);
        }
        *self.painted.borrow_mut() = Some((r, self.look()));
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let g = self.geom(r);
        let mut out = Vec::new();
        let h = Head::of(r, self.pitched.is_some());
        let (prev, next, shorter, longer) = self.usable();
        if let Some(b) = &self.bank {
            // Play's head: the track's ‹ and ›, the banks, the pages and
            // Follow.
            let l = BankLayout::of(r);
            let (left, right) = l.track.take_left(l.track.w / 2);
            out.push(
                Target::tap("head", HEAD_TRACK_PREV, "Previous track", left).disabled_if(!b.switch),
            );
            out.push(
                Target::tap("head", HEAD_TRACK_NEXT, "Next track", right).disabled_if(!b.switch),
            );
            for (i, br) in l.banks.iter().enumerate() {
                let name = format!("Bank {}", (b'A' + i as u8) as char);
                out.push(
                    Target::tap("head", HEAD_BANK + i, name, *br)
                        .disabled_if(b.bank.is_none() || b.bank == Some(i)),
                );
            }
            out.push(Target::tap("head", HEAD_PREV, "Previous page", l.prev).disabled_if(!prev));
            out.push(Target::tap("head", HEAD_NEXT, "Next page", l.next).disabled_if(!next));
            out.push(Target::tap("head", HEAD_FOLLOW, "Follow", l.follow));
        }
        // The head: ‹ › turn the pages of 16, - + halve or double the
        // steps (faint when they can't; a tap then says why).
        for (k, label, rect, can) in [
            (HEAD_PREV, "Previous page", h.prev, prev),
            (HEAD_NEXT, "Next page", h.next, next),
            (HEAD_SHORTER, "Fewer steps", h.shorter, shorter),
            (HEAD_LONGER, "More steps", h.longer, longer),
        ]
        .into_iter()
        .filter(|_| self.bank.is_none())
        {
            out.push(Target::tap("head", k, label, rect).disabled_if(!can));
        }
        if self.pitched.is_some() && self.bank.is_none() {
            let (lower, higher) = self.octave_usable(g.rows);
            for (k, label, rect, can) in [
                (HEAD_FOLD, "Fold", h.fold, true),
                (HEAD_LOWER, "Octave down", h.lower, lower),
                (HEAD_HIGHER, "Octave up", h.higher, higher),
            ] {
                out.push(Target::tap("head", k, label, rect).disabled_if(!can));
            }
        }
        if let Some(pill) = self.arm {
            out.push(Target::tap("arm", 0, pill.label(), arm_pill_hit(r)));
        }
        for (k, lane) in self.shown(g.rows).iter().enumerate() {
            if self.pitched.is_some() && lane.pad < 16 {
                // A scale row's name plays its note while held.
                out.push(Target::hold(
                    "note",
                    lane.pad,
                    format!("{}: play", lane.name),
                    g.mute_hit(k),
                ));
            } else if !lane.empty {
                // The lane's whole name is its mute button (bigger mutes).
                out.push(Target::tap(
                    "mute",
                    lane.pad,
                    format!("{} speaker", lane.name),
                    g.mute_hit(k),
                ));
            }
            for st in 0..self.steps.min(16) {
                out.push(Target::tap(
                    "step",
                    lane.pad * 16 + st,
                    format!("{} step {}", lane.name, st + 1),
                    g.cell(k, st),
                ));
            }
        }
        out
    }

    fn touch(&mut self, f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        // A scale row's note sounds until its finger lifts.
        if phase == Touch::Up {
            if let Some((_, pad)) = self.sounding.filter(|(finger, _)| *finger == f) {
                self.sounding = None;
                return Some(EventKind::Pad {
                    index: pad,
                    down: false,
                });
            }
        }
        if self.bank.is_some() && phase != Touch::Down {
            return self.swipe_on(f, phase, y, r);
        }
        if phase != Touch::Down {
            return None;
        }
        if self.bank.is_some() {
            if let Some(i) = BankLayout::of(r).at(x, y) {
                return Some(EventKind::Tab(i));
            }
        } else if let Some(i) = Head::of(r, self.pitched.is_some()).at(x, y) {
            return Some(EventKind::Tab(i));
        }
        if self.arm.is_some() && arm_pill_hit(r).contains(x, y) {
            return Some(EventKind::Select(0));
        }
        let tap = self.tap_at(f, x, y, r);
        if self.bank.is_none() || matches!(tap, Some(EventKind::Pad { .. })) {
            return tap;
        }
        // Play's rows: the tap waits for the finger to lift, so a swipe
        // can move the rows instead.
        let (_, body) = card_head(r);
        if body.contains(x, y) {
            let top = self.bank.as_ref().map_or(0, |b| b.top);
            self.swipe = Some(Swipe {
                finger: f,
                y,
                top,
                moved: 0,
                swiping: false,
                tap,
            });
        }
        None
    }
}

impl StepGrid {
    /// A finger moving on Play's rows, or lifting: past [`SWIPE_PX`] it
    /// swipes, the rows following it (`Step { index: ROWS, delta }`); one
    /// that lifts before that gives its tap.
    fn swipe_on(&mut self, f: u64, phase: Touch, y: i32, r: Rect) -> Option<EventKind> {
        let mut s = self.swipe.clone().filter(|s| s.finger == f)?;
        match phase {
            Touch::Up => {
                self.swipe = None;
                if s.swiping {
                    None
                } else {
                    s.tap
                }
            }
            Touch::Cancel => {
                self.swipe = None;
                None
            }
            _ => {
                let dy = y - s.y;
                s.swiping |= dy.abs() > SWIPE_PX;
                let g = self.geom(r);
                let pitch = g.rh + LANE_GAP;
                // A finger moving up shows the rows below.
                let want = -dy / pitch.max(1);
                let last = self.lanes.len().saturating_sub(g.rows) as i32;
                let to = (s.top as i32 + want).clamp(0, last);
                let delta = to - (s.top as i32 + s.moved);
                s.moved += delta;
                let swiping = s.swiping;
                self.swipe = Some(s);
                (swiping && delta != 0).then_some(EventKind::Step { index: ROWS, delta })
            }
        }
    }

    /// What a finger down at (`x`, `y`) on the rows taps: a lane's name
    /// (its mute, or a scale row's note), or a step.
    fn tap_at(&mut self, f: u64, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let g = self.geom(r);
        let lanes = self.shown(g.rows);
        for (k, lane) in lanes.iter().enumerate() {
            if g.mute_hit(k).contains(x, y) {
                if self.pitched.is_some() && lane.pad < 16 {
                    let pad = lane.pad;
                    self.sounding = Some((f, pad));
                    return Some(EventKind::Pad {
                        index: pad,
                        down: true,
                    });
                }
                if lane.empty {
                    return None;
                }
                return Some(EventKind::Toggle(lane.pad));
            }
            for s in 0..self.steps.min(16) {
                // A little slack round each step, half the gaps.
                if g.cell(k, s).inset(-2).contains(x, y) {
                    return Some(EventKind::Cell {
                        row: lane.pad,
                        col: s,
                    });
                }
            }
        }
        None
    }
}

// ---------------------------------------------------------- inspector

/// The inspector's taps, as `Tab` indices after the banks' 0 to 2.
pub const INSPECT_COPY: usize = 10;
pub const INSPECT_CLEAR: usize = 11;
pub const INSPECT_CLOSE: usize = 12;
/// On and Off (the step plays, or is switched off and kept) and the Tie /
/// Slide chip (a tap steps it).
pub const INSPECT_ON: usize = 13;
pub const INSPECT_OFF: usize = 14;
pub const INSPECT_JOIN: usize = 15;

/// The step held or picked on Beats, under the pattern in the throws'
/// place (the sequencer design's mockup A): its settings as chips, the
/// ones the Q-Links turn outlined; Q-LINK's banks, a tap picks one
/// (`Tab(0..=2)`: Step, Trig, Sound); its locks, each with a × that takes
/// it off (`Toggle(k)`); On and Off (`Tab(INSPECT_ON)`, `Tab(INSPECT_OFF)`:
/// the step plays, or is switched off and keeps all it holds), Copy
/// (`Tab(INSPECT_COPY)`) and Clear (`Tab(INSPECT_CLEAR)`); the Tie / Slide
/// chip steps with a tap (`Tab(INSPECT_JOIN)`); for a step picked (not
/// held), × at the head's right (`Tab(INSPECT_CLOSE)`) picks it off, the
/// step kept.
#[derive(Default)]
pub struct StepInspector {
    pub inspect: crate::play_page::Inspect,
}

/// Where the inspector's parts sit.
struct InspectGeom {
    head: Rect,
    banks: [Rect; 3],
    close: Rect,
    chips: [Rect; crate::play_page::INSPECT_N],
    label: Rect,
    locks: Vec<Rect>,
    hint: Rect,
    /// On and Off, side by side.
    on: [Rect; 2],
    copy: Rect,
    clear: Rect,
}

const BANK_W: i32 = 84;
const CLOSE_W: i32 = 52;
const LOCK_W: i32 = 196;
const BUTTON_W: i32 = 100;
const ON_W: i32 = 76;

impl StepInspector {
    fn geom(&self, r: Rect) -> InspectGeom {
        let (h, body) = card_head(r);
        let band = Rect::new(h.x, r.y + 4, h.w, h.h + 12);
        let close = Rect::new(band.right() - CLOSE_W + 4, band.y, CLOSE_W - 4, band.h);
        let banks = std::array::from_fn(|k| {
            Rect::new(
                band.right() - CLOSE_W - (3 - k as i32) * BANK_W,
                band.y,
                BANK_W - 4,
                band.h,
            )
        });
        let gap = 10;
        let n = crate::play_page::INSPECT_N as i32;
        let cw = (body.w - (n - 1) * gap) / n;
        let chips =
            std::array::from_fn(|k| Rect::new(body.x + k as i32 * (cw + gap), body.y, cw, 58));
        let label = Rect::new(body.x, body.y + 58 + 12, body.w, 14);
        let row = Rect::new(body.x, label.bottom() + 4, body.w, 44);
        let clear = Rect::new(row.right() - BUTTON_W, row.y, BUTTON_W, row.h);
        let copy = Rect::new(clear.x - gap - BUTTON_W, row.y, BUTTON_W, row.h);
        let off = Rect::new(copy.x - gap - ON_W, row.y, ON_W, row.h);
        let on = [Rect::new(off.x - 4 - ON_W, row.y, ON_W, row.h), off];
        let room = on[0].x - gap - row.x;
        let fit = ((room + gap) / (LOCK_W + gap)).max(0) as usize;
        let n = self.inspect.locks.len().min(fit);
        let locks: Vec<Rect> = (0..n)
            .map(|k| Rect::new(row.x + k as i32 * (LOCK_W + gap), row.y, LOCK_W, row.h))
            .collect();
        let hx = locks.last().map_or(row.x, |l| l.right() + gap);
        let hint = Rect::new(hx, row.y, (on[0].x - gap - hx).max(0), row.h);
        InspectGeom {
            head: h,
            banks,
            close,
            chips,
            label,
            locks,
            hint,
            on,
            copy,
            clear,
        }
    }

    fn banks_usable(&self) -> [bool; 3] {
        let note = !self.inspect.chips.is_empty();
        [note, note, self.inspect.sound]
    }
}

impl Widget for StepInspector {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let i = &self.inspect;
        cx.canvas.fill_rect(r, t.bg);
        card(cx, r, &i.title, "");
        let g = self.geom(r);
        // Q-LINK's banks, the one showing filled.
        let hint_r = Rect::new(
            g.head.x,
            g.head.y,
            (g.banks[0].x - 12 - g.head.x).max(0),
            g.head.h,
        );
        cx.text(
            FontId::Sans,
            13.0,
            hint_r,
            Align::Right,
            t.text_faint,
            "Q-LINK steps the bank",
        );
        let usable = self.banks_usable();
        for (k, b) in g.banks.iter().enumerate() {
            let on = i.bank as usize == k;
            let fill = if on {
                if k == 2 {
                    LOCKED
                } else {
                    t.accent
                }
            } else {
                t.raised
            };
            cx.canvas.fill_round_rect(*b, 8, fill);
            let ink = if on { t.bg } else { t.button_ink(usable[k]) };
            let name = ["Step", "Trig", "Sound"][k];
            cx.text(FontId::Bold, 15.0, *b, Align::Center, ink, name);
        }
        // × for a step picked: picks it off (a step held goes when let go).
        if !i.sound {
            cx.canvas.fill_round_rect(g.close, 8, t.raised);
            cx.text(FontId::Bold, 18.0, g.close, Align::Center, t.text_dim, "×");
        }
        // The settings: name over value, the ones the Q-Links turn outlined.
        if i.chips.is_empty() {
            let last = g.chips[g.chips.len() - 1];
            let r0 = Rect::new(
                g.chips[0].x,
                g.chips[0].y,
                last.right() - g.chips[0].x,
                g.chips[0].h,
            );
            cx.canvas
                .fill_round_rect(r0, 8, t.raised.mix(t.surface, 0.5));
            cx.text(
                FontId::Sans,
                15.0,
                r0.inset_xy(16, 0),
                Align::Left,
                t.text_dim,
                "No note on this step: its locks play with no note",
            );
        }
        for (k, (name, value)) in i.chips.iter().enumerate().take(g.chips.len()) {
            let c = g.chips[k];
            cx.canvas.fill_round_rect(c, 8, t.raised);
            if i.turned[k] {
                cx.canvas.stroke_round_rect(c, 8, 2, t.accent);
            }
            let (top, bottom) = c.inset_xy(12, 6).take_top(20);
            cx.text(FontId::Sans, 13.0, top, Align::Left, t.text_dim, name);
            cx.text(FontId::Bold, 20.0, bottom, Align::Left, t.text, value);
            if k == crate::play_page::INSPECT_JOIN_CHIP {
                // A tap steps it: a small › at its top right says so.
                let mark = Rect::new(c.right() - 26, c.y + 4, 20, 18);
                cx.text(FontId::Bold, 13.0, mark, Align::Center, t.text_dim, "›");
            }
        }
        // On and Off: the one the step is lit (Off in the warn colour, as
        // the step's "off" outline says it).
        if let Some(on) = i.on {
            for (k, b) in g.on.iter().enumerate() {
                let lit = (k == 0) == on;
                let fill = match (lit, k) {
                    (true, 0) => t.accent,
                    (true, _) => t.warn,
                    _ => t.raised,
                };
                cx.canvas.fill_round_rect(*b, 8, fill);
                let ink = if lit { t.bg } else { t.button_ink(true) };
                cx.text(FontId::Bold, 15.0, *b, Align::Center, ink, ["On", "Off"][k]);
            }
        }
        // The locks: a pill each with its ×, then how to add one.
        cx.text(
            FontId::Bold,
            11.0,
            g.label,
            Align::Left,
            t.text_dim,
            "LOCKS ON THIS STEP",
        );
        for (k, lr) in g.locks.iter().enumerate() {
            let (_, name, value) = &i.locks[k];
            cx.canvas
                .fill_round_rect(*lr, lr.h / 2, LOCKED.with_alpha(70));
            cx.canvas.stroke_round_rect(*lr, lr.h / 2, 2, LOCKED);
            dot(
                cx,
                lr.x + 18,
                lr.y + lr.h / 2,
                5,
                LOCKED.mix(Color::rgb(0xFFFFFF), 0.4),
            );
            let text_r = Rect::new(lr.x + 30, lr.y, lr.w - 30 - 34, lr.h);
            let nw = cx.text(FontId::Bold, 15.0, text_r, Align::Left, t.text, name);
            let vr = Rect::new(text_r.x + nw + 8, lr.y, (text_r.w - nw - 8).max(0), lr.h);
            cx.text(FontId::Sans, 14.0, vr, Align::Left, t.text_dim, value);
            let x = Rect::new(lr.right() - 34, lr.y, 26, lr.h);
            cx.text(FontId::Bold, 16.0, x, Align::Center, t.text_dim, "×");
        }
        if g.hint.w > 120 {
            let x0 = g.hint.x as f32;
            let (y0, x1, y1) = (
                g.hint.y as f32,
                g.hint.right() as f32 - 1.0,
                g.hint.bottom() as f32 - 1.0,
            );
            let mut sh = mpfree_ui::Shape::new(g.hint);
            sh.dashed(
                &[(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)],
                1.5,
                6.0,
            );
            cx.canvas.blend_shape(&sh, t.outline);
            let words = if i.sound {
                "+ Lock: turn a Sound knob while the step is held"
            } else {
                "+ Lock: hold the step's pad and turn a knob"
            };
            cx.text(
                FontId::Sans,
                14.0,
                g.hint.inset_xy(14, 0),
                Align::Left,
                t.text_faint,
                words,
            );
        }
        for (b, label, on) in [
            (
                g.copy,
                if i.copying { "Copy to…" } else { "Copy" },
                i.copying,
            ),
            (g.clear, "Clear", false),
        ] {
            cx.canvas
                .fill_round_rect(b, 8, if on { t.accent } else { t.raised });
            let ink = if on { t.bg } else { t.button_ink(i.one) };
            cx.text(FontId::Bold, 15.0, b, Align::Center, ink, label);
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let g = self.geom(r);
        let usable = self.banks_usable();
        let mut out: Vec<Target> = g
            .banks
            .iter()
            .enumerate()
            .map(|(k, b)| {
                Target::tap(
                    "bank",
                    k,
                    format!("{} bank", ["Step", "Trig", "Sound"][k]),
                    *b,
                )
                .disabled_if(!usable[k])
            })
            .collect();
        for (k, lr) in g.locks.iter().enumerate() {
            out.push(Target::tap(
                "unlock",
                k,
                format!("Take off lock {}", k + 1),
                *lr,
            ));
        }
        if !self.inspect.sound {
            out.push(Target::tap(
                "close",
                INSPECT_CLOSE,
                "Close the inspector",
                g.close,
            ));
        }
        if self.inspect.on.is_some() {
            out.push(Target::tap("on", INSPECT_ON, "Step on", g.on[0]));
            out.push(Target::tap("off", INSPECT_OFF, "Step off, kept", g.on[1]));
            out.push(Target::tap(
                "join",
                INSPECT_JOIN,
                "Tie or slide",
                g.chips[crate::play_page::INSPECT_JOIN_CHIP],
            ));
        }
        let one = self.inspect.one;
        out.push(Target::tap("copy", INSPECT_COPY, "Copy the step", g.copy).disabled_if(!one));
        out.push(Target::tap("clear", INSPECT_CLEAR, "Clear the step", g.clear).disabled_if(!one));
        out
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let g = self.geom(r);
        if let Some(k) = g.banks.iter().position(|b| b.contains(x, y)) {
            return Some(EventKind::Tab(k));
        }
        if !self.inspect.sound && g.close.contains(x, y) {
            return Some(EventKind::Tab(INSPECT_CLOSE));
        }
        if let Some(k) = g.locks.iter().position(|l| l.contains(x, y)) {
            return Some(EventKind::Toggle(k));
        }
        if self.inspect.on.is_some() {
            if g.on[0].contains(x, y) {
                return Some(EventKind::Tab(INSPECT_ON));
            }
            if g.on[1].contains(x, y) {
                return Some(EventKind::Tab(INSPECT_OFF));
            }
            if g.chips[crate::play_page::INSPECT_JOIN_CHIP].contains(x, y) {
                return Some(EventKind::Tab(INSPECT_JOIN));
            }
        }
        if g.copy.contains(x, y) {
            return Some(EventKind::Tab(INSPECT_COPY));
        }
        g.clear
            .contains(x, y)
            .then_some(EventKind::Tab(INSPECT_CLEAR))
    }
}

// ------------------------------------------------------------- throws

/// One throw: hold to play.
#[derive(Clone, Debug, PartialEq)]
pub struct Throw {
    pub name: String,
    /// "1/8 dotted", "hall, 4 s".
    pub sub: String,
    pub color: Color,
    pub on: bool,
}

/// Hold-to-play effects: as many as fit (6 in a row on the home view, 8
/// in two rows, 4 in a narrow panel). A finger down on one gives
/// `Pad { index, down: true }`, its lift `down: false`.
#[derive(Default)]
pub struct Throws {
    pub items: Vec<Throw>,
    pub right: String,
    fingers: Vec<(u64, usize)>,
}

impl Throws {
    /// How many show in `r`, and in how many columns.
    fn grid(r: Rect) -> (usize, usize) {
        let (_, body) = card_head(r);
        if body.h < 80 {
            (6, 6)
        } else if r.w < 420 {
            (4, 2)
        } else {
            (8, 4)
        }
    }

    pub fn cells(&self, r: Rect) -> Vec<Rect> {
        let (_, body) = card_head(r);
        let (n, cols) = Throws::grid(r);
        let n = n.min(self.items.len());
        let cols = cols as i32;
        let rows = (n as i32 + cols - 1) / cols.max(1);
        let gap = 8;
        let w = (body.w - (cols - 1) * gap) / cols;
        let h = (body.h - (rows - 1) * gap) / rows.max(1);
        (0..n)
            .map(|i| {
                Rect::new(
                    body.x + (i as i32 % cols) * (w + gap),
                    body.y + (i as i32 / cols) * (h + gap),
                    w,
                    h,
                )
            })
            .collect()
    }
}

impl Widget for Throws {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        card(cx, r, "Throws", &self.right);
        for (cr, th) in self.cells(r).iter().zip(&self.items) {
            let (w, h) = (cr.w, cr.h);
            if th.on {
                cx.canvas
                    .stroke_round_rect(cr.inset(-3), 13, 2, th.color.with_alpha(170));
                cx.canvas.fill_round_rect(*cr, 10, th.color);
            } else {
                cx.canvas.fill_round_rect(*cr, 10, t.raised);
                cx.canvas
                    .fill_round_rect(Rect::new(cr.x + 10, cr.y + 9, 18, 4), 2, th.color);
            }
            let ink = if th.on { t.bg } else { t.text };
            let sub_ink = if th.on {
                t.bg.with_alpha(170)
            } else {
                t.text_dim
            };
            let inner = cr.inset_xy(10, 0);
            if h >= 50 {
                let (a, b) = inner.take_top(h / 2 + 4);
                cx.text(
                    FontId::Bold,
                    if w < 130 { 14.0 } else { 15.0 },
                    a.take_top(10).1,
                    Align::Left,
                    ink,
                    &th.name,
                );
                cx.text(
                    FontId::Sans,
                    12.0,
                    b.take_bottom(h / 2 - 4).0.take_top(h / 2 - 12).0,
                    Align::Left,
                    sub_ink,
                    &th.sub,
                );
            } else {
                let (a, b) = inner.take_top(h / 2 + 3);
                cx.text(
                    FontId::Bold,
                    14.0,
                    a.take_top(6).1,
                    Align::Left,
                    ink,
                    &th.name,
                );
                cx.text(
                    FontId::Sans,
                    11.5,
                    b.take_top(h / 2 - 9).0,
                    Align::Left,
                    sub_ink,
                    &th.sub,
                );
            }
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        self.cells(r)
            .into_iter()
            .enumerate()
            .map(|(i, c)| Target::hold("throw", i, self.items[i].name.clone(), c))
            .collect()
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down => {
                let i = self.cells(r).iter().position(|c| c.contains(x, y))?;
                self.fingers.push((finger, i));
                Some(EventKind::Pad {
                    index: i,
                    down: true,
                })
            }
            Touch::Up | Touch::Cancel => {
                let k = self.fingers.iter().position(|(f, _)| *f == finger)?;
                let (_, i) = self.fingers.remove(k);
                Some(EventKind::Pad {
                    index: i,
                    down: false,
                })
            }
            Touch::Move => None,
        }
    }
}

// ------------------------------------------------------------- XY

/// The two-way effect pad: hold and move. Gives `Xy { x, y, down }` (0 to
/// 1, left to right and bottom to top) while the finger is on it, and
/// once more with `down: false` when it lifts. It keeps a short trail of
/// where the finger was. Its head is a chip naming what it plays ("XY ·
/// Filter + echo ▾"): a tap anywhere on the head gives `Select(0)`, and
/// the screen opens the list to pick another (Jay, 18 Nov 2026 07:34Z).
pub struct XyPad {
    pub title: String,
    /// What it plays, on its chip: "Filter + echo", "Wash".
    pub chip: String,
    pub right: String,
    pub x_axis: String,
    pub y_axis: String,
    /// What the place means: "3.9 kHz · 62 %".
    pub label: String,
    pub x: f32,
    pub y: f32,
    pub on: bool,
    trail: VecDeque<(f32, f32)>,
    finger: Option<u64>,
}

impl Default for XyPad {
    fn default() -> Self {
        XyPad {
            title: "XY pad".into(),
            chip: "Filter + echo".into(),
            right: "hold to play".into(),
            x_axis: "LP  ←  off  →  HP".into(),
            y_axis: "↑ echo".into(),
            label: String::new(),
            x: 0.5,
            y: 0.0,
            on: false,
            trail: VecDeque::new(),
            finger: None,
        }
    }
}

const TRAIL: usize = 10;

impl XyPad {
    fn pad(r: Rect) -> Rect {
        let (_, body) = card_head(r);
        body.take_bottom(18).1
    }

    /// The head over the pad, all of it the chip's target (a fingertip
    /// across the card's width).
    fn head(r: Rect) -> Rect {
        let pad = XyPad::pad(r);
        Rect::new(r.x, r.y, r.w, (pad.y - 4 - r.y).max(0))
    }

    /// The chip as drawn: "XY · Filter + echo" and its ▾, in the head.
    fn chip_rect(&self, cx: &mut PaintCx, r: Rect) -> Rect {
        let (h, _) = card_head(r);
        let w = cx.measure(FontId::Bold, draw::HEAD_PX, &self.chip_text()) + 42;
        Rect::new(h.x - 4, h.y - 5, w.min(h.w + 4), h.h + 10)
    }

    fn chip_text(&self) -> String {
        format!("XY · {}", self.chip)
    }

    /// Where the finger has been, oldest first (the previews draw one).
    pub fn set_trail(&mut self, points: &[(f32, f32)]) {
        self.trail = points.iter().rev().take(TRAIL).rev().copied().collect();
    }
}

impl Widget for XyPad {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        let body = card(cx, r, "", &self.right);
        // The chip: what the pad plays, and a ▾ for the list.
        let chip = self.chip_rect(cx, r);
        cx.canvas.fill_round_rect(chip, chip.h / 2, t.raised);
        cx.text(
            FontId::Bold,
            draw::HEAD_PX,
            Rect::new(chip.x + 12, chip.y, chip.w - 30, chip.h),
            Align::Left,
            t.text,
            &self.chip_text(),
        );
        let (cx0, cy0) = (chip.right() - 16, chip.y + chip.h / 2 - 2);
        for k in 0..5 {
            cx.canvas
                .fill_rect(Rect::new(cx0 - 4 + k, cy0 + k, 9 - 2 * k, 1), t.text_dim);
        }
        let (axis, pad) = body.take_bottom(18);
        cx.canvas.fill_round_rect(pad, 10, t.bg.mix(t.surface, 0.4));
        for k in 1..4 {
            let x = pad.x + pad.w * k / 4;
            let y = pad.y + pad.h * k / 4;
            cx.canvas
                .fill_rect(Rect::new(x, pad.y + 6, 1, pad.h - 12), t.raised);
            cx.canvas
                .fill_rect(Rect::new(pad.x + 6, y, pad.w - 12, 1), t.raised);
        }
        let at = |fx: f32, fy: f32| {
            (
                pad.x + (fx.clamp(0.0, 1.0) * pad.w as f32) as i32,
                pad.y + ((1.0 - fy.clamp(0.0, 1.0)) * pad.h as f32) as i32,
            )
        };
        let c = KNOB[0];
        let n = self.trail.len();
        for (k, &(tx, ty)) in self.trail.iter().enumerate() {
            let f = (k + 1) as f32 / (n + 1) as f32;
            let (px, py) = at(tx, ty);
            dot(
                cx,
                px,
                py,
                2 + (k as i32) / 3,
                c.with_alpha((40.0 + 120.0 * f) as u8),
            );
        }
        let (px, py) = at(self.x, self.y);
        let (line, ring_a, body_c) = if self.on {
            (c.with_alpha(110), c.with_alpha(60), c)
        } else {
            (c.with_alpha(40), c.with_alpha(25), c.mix(t.surface, 0.55))
        };
        cx.canvas
            .fill_rect(Rect::new(px, pad.y + 4, 1, pad.h - 8), line);
        cx.canvas
            .fill_rect(Rect::new(pad.x + 4, py, pad.w - 8, 1), line);
        dot(cx, px, py, 20, ring_a);
        dot(cx, px, py, 12, body_c);
        cx.text(
            FontId::Sans,
            11.5,
            axis,
            Align::Left,
            t.text_faint,
            &self.x_axis,
        );
        cx.text(
            FontId::Sans,
            11.5,
            axis,
            Align::Right,
            t.text_faint,
            &self.y_axis,
        );
        if !self.label.is_empty() {
            let lw = cx.measure(FontId::Bold, 12.0, &self.label) + 4;
            // Beside the knob on the side with more room (the mockups: on
            // its left once past the middle).
            let lx = if px < pad.x + pad.w / 2 || px - 24 - lw < pad.x + 6 {
                px + 24
            } else {
                px - 24 - lw
            };
            let ly = (py - 9).clamp(pad.y + 4, pad.bottom() - 22);
            cx.text(
                FontId::Bold,
                12.0,
                Rect::new(lx, ly, lw, 18),
                Align::Left,
                if self.on { t.text } else { t.text_dim },
                &self.label,
            );
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let pad = XyPad::pad(r);
        if pad.w <= 0 || pad.h <= 0 {
            return Vec::new();
        }
        vec![
            Target::tap("xy-chip", 0, self.chip_text(), XyPad::head(r)),
            Target::drag("xy", 0, self.title.clone(), pad),
        ]
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let pad = XyPad::pad(r);
        if pad.w <= 0 || pad.h <= 0 {
            return None;
        }
        if phase == Touch::Down && self.finger.is_none() && XyPad::head(r).contains(x, y) {
            return Some(EventKind::Select(0));
        }
        match phase {
            Touch::Down | Touch::Move => {
                if phase == Touch::Down {
                    if !pad.contains(x, y) {
                        return None;
                    }
                    self.finger = Some(finger);
                    self.trail.clear();
                } else if self.finger != Some(finger) {
                    return None;
                }
                let fx = ((x - pad.x) as f32 / pad.w as f32).clamp(0.0, 1.0);
                let fy = (1.0 - (y - pad.y) as f32 / pad.h as f32).clamp(0.0, 1.0);
                if self.on {
                    self.trail.push_back((self.x, self.y));
                    while self.trail.len() > TRAIL {
                        self.trail.pop_front();
                    }
                }
                self.x = fx;
                self.y = fy;
                self.on = true;
                Some(EventKind::Xy {
                    x: fx,
                    y: fy,
                    down: true,
                })
            }
            Touch::Up | Touch::Cancel => {
                if self.finger != Some(finger) {
                    return None;
                }
                self.finger = None;
                self.on = false;
                self.trail.clear();
                Some(EventKind::Xy {
                    x: self.x,
                    y: self.y,
                    down: false,
                })
            }
        }
    }
}

// ------------------------------------------------------------- timing

/// One hit in the window: where it landed in sixteenths from the window's
/// start, how far from the nearest sixteenth (in sixteenths, negative is
/// early) and in ms, and its velocity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimedHit {
    pub pad: usize,
    pub at: f32,
    pub off: f32,
    pub off_ms: f32,
    pub velocity: u8,
    /// From the window's last time round (ahead of the playhead): drawn
    /// faint, until this time round plays over it.
    pub old: bool,
}

/// Your timing: the last two bars of your hits against the 1/16 grid, a
/// lane for each of the four sounds you hit most, then the average
/// offset, the spread, the velocity and how the offsets spread.
#[derive(Clone, Debug, PartialEq)]
pub struct YourTiming {
    pub right: String,
    /// (pad, name, colour, hits in the window).
    pub lanes: Vec<(usize, String, Color, Vec<TimedHit>)>,
    /// The playhead, in sixteenths from the window's start.
    pub now: Option<f32>,
    /// Sixteenths across the window.
    pub steps: usize,
    pub average_ms: Option<f32>,
    pub spread_ms: Option<f32>,
    pub velocity: Option<f32>,
    /// Offsets from 30 ms early to 30 ms late, 4 ms a bar.
    pub histogram: [u32; 15],
    /// How much larger than life the offsets are drawn.
    pub magnify: f32,
    /// Shown when there is nothing yet.
    pub note: String,
}

impl Default for YourTiming {
    fn default() -> Self {
        YourTiming {
            right: "last 2 bars · grid 1/16".into(),
            lanes: Vec::new(),
            now: None,
            steps: 32,
            average_ms: None,
            spread_ms: None,
            velocity: None,
            histogram: [0; 15],
            magnify: 4.0,
            note: String::new(),
        }
    }
}

impl Widget for YourTiming {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        if r.h < YourTiming::FULL_H {
            self.paint_strip(cx, r);
            return;
        }
        let body = card(cx, r, "Your timing", &self.right);
        let (stats, lanes) = body.take_bottom(92);
        let lanes = lanes.take_bottom(10).1;
        let (nm, grid) = lanes.take_left(64);
        let steps = self.steps.max(1);
        let sx = |s: f32| grid.x + (s / steps as f32 * grid.w as f32) as i32;
        for s in 0..=steps {
            let c = if s % 16 == 0 {
                t.text_faint
            } else if s % 4 == 0 {
                t.outline
            } else {
                t.raised
            };
            cx.canvas
                .fill_rect(Rect::new(sx(s as f32), grid.y, 1, grid.h), c);
        }
        let n = self.lanes.len().max(4) as i32;
        let lh = grid.h / n;
        if self.lanes.is_empty() && !self.note.is_empty() {
            cx.text(
                FontId::Sans,
                14.0,
                grid,
                Align::Center,
                t.text_dim,
                &self.note,
            );
        }
        for (li, (_, name, c, hits)) in self.lanes.iter().enumerate() {
            let y = grid.y + li as i32 * lh;
            cx.text(
                FontId::Sans,
                13.0,
                Rect::new(nm.x, y, nm.w, lh),
                Align::Left,
                t.text_dim,
                name,
            );
            for h in hits {
                let hx = sx(h.at + h.off * (self.magnify - 1.0));
                let gx = sx(h.at - h.off);
                let cy = y + lh / 2;
                let (a, b) = if hx < gx { (hx, gx) } else { (gx, hx) };
                cx.canvas
                    .fill_rect(Rect::new(a, cy - 1, (b - a).max(1), 2), c.with_alpha(110));
                let vel = h.velocity as f32 / 127.0;
                let c = if h.old { c.mix(t.surface, 0.7) } else { *c };
                dot(cx, hx, cy, (4.0 + 4.0 * vel) as i32, c);
            }
        }
        if let Some(now) = self.now {
            cx.canvas
                .fill_rect(Rect::new(sx(now), grid.y - 2, 2, grid.h + 4), t.text);
        }
        let (nums, hist) = stats.take_left(360);
        self.paint_numbers(cx, nums.take_top(14).1, false);
        self.paint_spread(cx, hist.inset_xy(0, 10), false);
    }
}

impl YourTiming {
    /// Shorter than this, it draws as the Beats view's strip: no lanes,
    /// only the numbers and how the offsets spread.
    pub const FULL_H: i32 = 150;

    /// The strip in Beats (Jay 13 Nov: fold the Drum view's timing into
    /// Beats): the title, the three numbers, then the spread of offsets.
    fn paint_strip(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.panel(r);
        let inner = r.inset_xy(12, 8);
        let (title, rest) = inner.take_left(180);
        let (a, b) = title.take_top(title.h / 2);
        cx.text(
            FontId::Bold,
            draw::HEAD_PX,
            a,
            Align::Left,
            t.text_dim,
            "YOUR TIMING",
        );
        // The strip has no lanes, so the offsets' magnifying is moot.
        let caption = self.right.split(" · offsets").next().unwrap_or("");
        cx.text(FontId::Sans, 12.0, b, Align::Left, t.text_faint, caption);
        let (nums, hist) = rest.take_left(390);
        self.paint_numbers(cx, nums, true);
        self.paint_spread(cx, hist, true);
        if self.lanes.is_empty() && !self.note.is_empty() {
            let note = "Play the pads to see your timing";
            cx.text(FontId::Sans, 13.0, hist, Align::Center, t.text_dim, note);
        }
    }

    /// AVERAGE, SPREAD and VELOCITY side by side in `nums`.
    fn paint_numbers(&self, cx: &mut PaintCx, nums: Rect, strip: bool) {
        let t = cx.theme.clone();
        let avg = match self.average_ms {
            Some(a) if a < -0.5 => format!("{:.0} ms early", -a),
            Some(a) if a > 0.5 => format!("{a:.0} ms late"),
            Some(_) => "on the grid".to_string(),
            None => "-".to_string(),
        };
        let spread = self
            .spread_ms
            .map_or("-".to_string(), |s| format!("± {s:.0} ms"));
        let vel = self
            .velocity
            .map_or("-".to_string(), |v| format!("avg {v:.0}"));
        let items = [
            ("AVERAGE", avg, t.accent),
            ("SPREAD", spread, t.text),
            ("VELOCITY", vel, t.text),
        ];
        let iw = nums.w / 3;
        let (label_h, value_px) = if strip { (20, 18.0) } else { (22, 20.0) };
        for (i, (k, v, c)) in items.iter().enumerate() {
            let ir = Rect::new(nums.x + i as i32 * iw, nums.y, iw, nums.h);
            let (a, b) = ir.take_top(label_h);
            cx.text(FontId::Sans, 12.0, a, Align::Left, t.text_faint, k);
            cx.text(FontId::Bold, value_px, b.take_top(34).0, Align::Left, *c, v);
        }
    }

    /// How the offsets spread, 30 ms early to 30 ms late, in `hist`.
    fn paint_spread(&self, cx: &mut PaintCx, hist: Rect, strip: bool) {
        let t = cx.theme.clone();
        cx.canvas.fill_round_rect(hist, 8, t.bg.mix(t.surface, 0.4));
        let hb = if strip {
            hist.inset_xy(10, 5)
        } else {
            hist.inset_xy(12, 10)
        };
        let (axis, bars_r) = hb.take_bottom(if strip { 14 } else { 16 });
        let n = self.histogram.len() as i32;
        let top = self.histogram.iter().copied().max().unwrap_or(0).max(1);
        let bw = (bars_r.w - (n - 1) * 3) / n;
        for (i, &cnt) in self.histogram.iter().enumerate() {
            let h = (cnt as f32 / top as f32 * bars_r.h as f32) as i32;
            let c = if i == 7 {
                t.good
            } else {
                t.accent.with_alpha(170)
            };
            cx.canvas.fill_round_rect(
                Rect::new(
                    bars_r.x + i as i32 * (bw + 3),
                    bars_r.bottom() - h.max(2),
                    bw,
                    h.max(2),
                ),
                2,
                c,
            );
        }
        cx.text(
            FontId::Sans,
            11.0,
            axis,
            Align::Left,
            t.text_faint,
            "30 ms early",
        );
        cx.text(
            FontId::Sans,
            11.0,
            axis,
            Align::Center,
            t.text_faint,
            "on the grid",
        );
        cx.text(
            FontId::Sans,
            11.0,
            axis,
            Align::Right,
            t.text_faint,
            "30 ms late",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switch_taps_give_the_views() {
        let mut tr = PlayTransport::default();
        let r = Rect::new(12, 84, 998, 84);
        let (_, segs) = PlayTransport::switch(r, tr.look.views.len());
        for (i, s) in segs.iter().enumerate() {
            assert_eq!(
                tr.touch(1, Touch::Down, s.x + s.w / 2, s.y + s.h / 2, r),
                Some(EventKind::Tab(i))
            );
        }
        assert_eq!(segs.len(), 2, "All, Loop");
        assert!(segs[1].right() <= r.right() - 16);
        // The Sequencer's switch: Steps and Notes, named as the targets.
        tr.look.views = &crate::play_page::SEQ_SWITCH;
        let names: Vec<String> = tr
            .targets(r)
            .into_iter()
            .filter(|t| t.id == "view")
            .map(|t| t.label)
            .collect();
        assert_eq!(names, ["Steps", "Notes"]);
    }

    #[test]
    fn mini_pads_tap_and_lift() {
        let mut m = MiniPads::default();
        let r = Rect::new(0, 0, 270, 275);
        let (_, _, body) = m.parts(r);
        let c = MiniPads::cell(body, 0);
        assert!(c.y > MiniPads::cell(body, 15).y, "pad 1 is at the bottom");
        assert_eq!(
            m.touch(7, Touch::Down, c.x + 5, c.y + 5, r),
            Some(EventKind::Pad {
                index: 0,
                down: true
            })
        );
        assert_eq!(
            m.touch(7, Touch::Up, 0, 0, r),
            Some(EventKind::Pad {
                index: 0,
                down: false
            })
        );
    }

    fn lane(pad: usize, steps: &[u8]) -> Lane {
        Lane {
            pad,
            name: format!("Pad {}", pad + 1),
            color: KNOB[0],
            muted: false,
            steps: steps.to_vec(),
            marks: Vec::new(),
            note: None,
            root: false,
            label: None,
            empty: false,
        }
    }

    /// Play's rows (the Play, Pads and Sequencer redesign's board 1): the
    /// bank's pads in order from the swipe's top, a tap that waits for the
    /// finger to lift, a swipe that moves the rows instead, an empty pad
    /// with no mute, and the head's track switch, banks and Follow.
    #[test]
    fn play_rows_show_the_bank_in_order_and_a_swipe_moves_them() {
        let mut g = StepGrid {
            steps: 16,
            length: 16,
            pages: 1,
            bank: Some(BankHead {
                track: "Drums".into(),
                sub: "Track 1 · armed".into(),
                color: KNOB[0],
                switch: true,
                bank: Some(0),
                follow: false,
                top: 0,
            }),
            ..Default::default()
        };
        g.lanes = (0..16)
            .map(|p| Lane {
                label: Some(format!("A{}", p + 1)),
                empty: p == 15,
                ..lane(p, &[0; 16])
            })
            .collect();
        let r = Rect::new(0, 0, 1038, 330);
        let rows = g.rows_shown(r);
        assert!((6..=8).contains(&rows), "{rows}");
        assert_eq!(g.lanes_shown(r), (0..rows).collect::<Vec<_>>());
        // A tap on a step waits for the lift.
        let (x, y) = g.step_at(r, 1, 4);
        assert_eq!(g.touch(1, Touch::Down, x, y, r), None);
        assert_eq!(
            g.touch(1, Touch::Up, x, y, r),
            Some(EventKind::Cell { row: 1, col: 4 })
        );
        // A name: its mute, on the lift.
        let (x, y) = g.name_at(r, 2);
        assert_eq!(g.touch(1, Touch::Down, x, y, r), None);
        assert_eq!(g.touch(1, Touch::Up, x, y, r), Some(EventKind::Toggle(2)));
        // A swipe up: the rows below, a row at a time; no tap at the end.
        let (x, y) = g.step_at(r, 3, 4);
        assert_eq!(g.touch(1, Touch::Down, x, y, r), None);
        let pitch = g.step_at(r, 1, 0).1 - g.step_at(r, 0, 0).1;
        assert_eq!(
            g.touch(1, Touch::Move, x, y - 6, r),
            None,
            "not a swipe yet"
        );
        assert_eq!(
            g.touch(1, Touch::Move, x, y - 2 * pitch - 4, r),
            Some(EventKind::Step {
                index: ROWS,
                delta: 2
            })
        );
        // Past the last row it stops.
        assert_eq!(
            g.touch(1, Touch::Move, x, y - 20 * pitch, r),
            Some(EventKind::Step {
                index: ROWS,
                delta: (16 - rows) as i32 - 2
            })
        );
        assert_eq!(g.touch(1, Touch::Up, x, y - 20 * pitch, r), None);
        // The screen moves the rows: the bank's last ones show, the empty
        // pad's row dashed, with no mute.
        if let Some(b) = g.bank.as_mut() {
            b.top = 16 - rows;
        }
        assert_eq!(g.lanes_shown(r), (16 - rows..16).collect::<Vec<_>>());
        let (x, y) = g.name_at(r, rows - 1);
        g.touch(1, Touch::Down, x, y, r);
        assert_eq!(
            g.touch(1, Touch::Up, x, y, r),
            None,
            "an empty pad has no mute"
        );
        let ts = g.targets(r);
        assert!(!ts.iter().any(|t| t.id == "mute" && t.index == 15));
        // The head: on the press.
        for k in [HEAD_TRACK_PREV, HEAD_TRACK_NEXT, HEAD_BANK + 2, HEAD_FOLLOW] {
            let (x, y) = StepGrid::bank_head_at(r, k);
            assert_eq!(
                g.touch(1, Touch::Down, x, y, r),
                Some(EventKind::Tab(k)),
                "{k}"
            );
            g.touch(1, Touch::Up, x, y, r);
        }
        // No length's - and + on Play; the bank shown can't be picked again.
        assert!(!ts.iter().any(|t| t.id == "head" && t.index == HEAD_LONGER));
        let bank_a = ts
            .iter()
            .find(|t| t.id == "head" && t.index == HEAD_BANK)
            .unwrap();
        assert!(!bank_a.enabled);
    }

    /// A synth track's scale rows (the sequencer design's mockup B): the
    /// window of notes high on top, the line above it, a name that plays
    /// its note while held, and the head's Fold and octave.
    #[test]
    fn scale_rows_show_the_notes_high_on_top_and_their_names_play() {
        let mut g = StepGrid {
            steps: 16,
            pitched: Some(Pitched {
                fold: false,
                low: 0,
            }),
            ..Default::default()
        };
        // Sixteen pads, lowest first, a root every seventh.
        g.lanes = (0..16)
            .map(|p| Lane {
                note: Some(48 + p as u8),
                root: p % 7 == 0,
                ..lane(p, &[0; 16])
            })
            .collect();
        let r = Rect::new(0, 0, 998, 360);
        assert_eq!(g.lanes_shown(r), vec![7, 6, 5, 4, 3, 2, 1, 0]);
        assert_eq!(g.note_rows(r), 8);
        assert_eq!(g.octave(8), "Oct 3");
        assert_eq!(g.octave_usable(8), (false, true));
        g.pitched = Some(Pitched {
            fold: false,
            low: 3,
        });
        assert_eq!(g.lanes_shown(r), vec![10, 9, 8, 7, 6, 5, 4, 3]);
        // Past the top: the highest eight.
        g.pitched = Some(Pitched {
            fold: false,
            low: 20,
        });
        assert_eq!(g.lanes_shown(r), vec![15, 14, 13, 12, 11, 10, 9, 8]);
        assert_eq!(g.octave_usable(8), (true, false));
        // STEP SEQ's line stays on top, over seven notes.
        g.pitched = Some(Pitched {
            fold: false,
            low: 0,
        });
        g.lanes.insert(0, lane(255, &[0; 16]));
        assert_eq!(g.lanes_shown(r), vec![255, 6, 5, 4, 3, 2, 1, 0]);
        assert_eq!(g.note_rows(r), 7);
        // A finger on a row's name plays it until that finger lifts.
        let (x, y) = g.name_at(r, 1);
        assert_eq!(
            g.touch(4, Touch::Down, x, y, r),
            Some(EventKind::Pad {
                index: 6,
                down: true
            })
        );
        assert_eq!(g.touch(5, Touch::Up, x, y, r), None, "another finger");
        assert_eq!(
            g.touch(4, Touch::Up, x, y, r),
            Some(EventKind::Pad {
                index: 6,
                down: false
            })
        );
        // The line's name is no key.
        let (x, y) = g.name_at(r, 0);
        assert_eq!(
            g.touch(6, Touch::Down, x, y, r),
            Some(EventKind::Toggle(255))
        );
        // The head: Fold, then the octave's ‹ and ›, before the page's.
        for k in [HEAD_FOLD, HEAD_LOWER, HEAD_HIGHER, HEAD_PREV, HEAD_LONGER] {
            let (x, y) = StepGrid::head_at(r, k, true);
            assert_eq!(g.touch(7, Touch::Down, x, y, r), Some(EventKind::Tab(k)));
        }
        let t = g.targets(r);
        assert!(t.iter().any(|t| t.id == "note" && t.label == "Pad 7: play"));
        assert!(!t.iter().any(|t| t.id == "mute" && t.index < 16));
        assert!(t.iter().any(|t| t.id == "head" && t.label == "Fold"));
        // A kit's lanes: no Fold, no octave.
        g.pitched = None;
        let (x, y) = StepGrid::head_at(r, HEAD_FOLD, true);
        assert_eq!(g.touch(8, Touch::Down, x, y, r), None);
    }

    #[test]
    fn the_grid_shows_the_lanes_that_fit_in_pad_order() {
        let mut g = StepGrid {
            steps: 16,
            ..Default::default()
        };
        g.lanes = (0..16).rev().map(|p| lane(p, &[0; 16])).collect();
        // Mockup G's pattern (8 lanes) and F's (6).
        assert_eq!(g.lanes_shown(Rect::new(0, 0, 998, 360)).len(), 8);
        assert_eq!(
            g.lanes_shown(Rect::new(0, 0, 716, 275)),
            vec![10, 11, 12, 13, 14, 15]
        );
        let r = Rect::new(0, 0, 998, 360);
        let geom = g.geom(r);
        let c = geom.cell(2, 5);
        assert_eq!(
            g.touch(1, Touch::Down, c.x + 3, c.y + 3, r),
            Some(EventKind::Cell { row: 10, col: 5 })
        );
        // The mute answers anywhere on the lane's name, its full height
        // and gaps: far more than the old 32 px square.
        let m = geom.mute_hit(0);
        assert!(m.w >= 110 && m.h >= geom.rh + LANE_GAP, "{m:?}");
        for (x, y) in [(m.x + 2, m.y + 1), (m.right() - 1, m.bottom() - 1)] {
            assert_eq!(
                g.touch(1, Touch::Down, x, y, r),
                Some(EventKind::Toggle(8)),
                "({x}, {y})"
            );
        }
        // The lanes' mute areas meet with no gap and don't overlap.
        assert_eq!(geom.mute_hit(0).bottom(), geom.mute_hit(1).y);
        let b = geom.mute_button(0);
        assert!(m.contains(b.x, b.y) && m.contains(b.right() - 1, b.bottom() - 1));
    }

    #[test]
    fn throws_fit_their_panel() {
        let mut th = Throws {
            items: (0..8)
                .map(|i| Throw {
                    name: format!("T{i}"),
                    sub: String::new(),
                    color: KNOB[0],
                    on: false,
                })
                .collect(),
            ..Throws::default()
        };
        assert_eq!(th.cells(Rect::new(0, 0, 704, 120)).len(), 6);
        assert_eq!(th.cells(Rect::new(0, 0, 686, 196)).len(), 8);
        assert_eq!(th.cells(Rect::new(0, 0, 374, 196)).len(), 4);
        let r = Rect::new(0, 0, 686, 196);
        let c = th.cells(r)[5];
        assert_eq!(
            th.touch(3, Touch::Down, c.x + 2, c.y + 2, r),
            Some(EventKind::Pad {
                index: 5,
                down: true
            })
        );
        assert_eq!(
            th.touch(3, Touch::Up, 0, 0, r),
            Some(EventKind::Pad {
                index: 5,
                down: false
            })
        );
    }

    #[test]
    fn the_xy_pad_reports_where_the_finger_is() {
        let mut xy = XyPad::default();
        let r = Rect::new(0, 0, 300, 196);
        let p = XyPad::pad(r);
        let Some(EventKind::Xy { x, y, down }) = xy.touch(1, Touch::Down, p.x, p.bottom() - 1, r)
        else {
            panic!("xy");
        };
        assert!(down && x == 0.0 && y < 0.02, "{x} {y}");
        let Some(EventKind::Xy { x, y, .. }) = xy.touch(1, Touch::Move, p.right(), p.y, r) else {
            panic!("xy");
        };
        assert!(x == 1.0 && y == 1.0 && xy.on);
        assert!(matches!(
            xy.touch(1, Touch::Up, 0, 0, r),
            Some(EventKind::Xy { down: false, .. })
        ));
        assert!(!xy.on);
    }
}
