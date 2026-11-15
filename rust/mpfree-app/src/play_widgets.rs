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

use crate::play_page::{SynthPads, LAYOUT_SWITCH, VIEWS};
use mpfree_arp::scale::pads::PadLayout;
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
    /// The view lit in the switch.
    pub view: usize,
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
            view: 0,
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
}

/// [`PlayTransport`]'s event for a tap on the tempo
/// ([`EventKind::Select`] with this index): a tap of the tempo, as TAP
/// TEMPO (live test 3: Jay wants the BPM tappable).
pub const TAP_BPM: usize = 0;

/// The transport card. A tap on the switch gives `Tab(view)`.
#[derive(Default)]
pub struct PlayTransport {
    pub look: TransportLook,
    painted: RefCell<Option<(Rect, TransportLook, TransportAreas)>>,
}

const SWITCH_SEG_W: i32 = 70;
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

    /// The view switch's box and its segments.
    pub fn switch(r: Rect) -> (Rect, [Rect; VIEWS.len()]) {
        let inner = PlayTransport::inner(r);
        let w = VIEWS.len() as i32 * SWITCH_SEG_W + 8;
        let b = Rect::new(
            inner.right() - w,
            inner.y + (inner.h - SWITCH_SEG_H - 8) / 2,
            w,
            SWITCH_SEG_H + 8,
        );
        let segs = std::array::from_fn(|i| {
            Rect::new(
                b.x + 4 + i as i32 * SWITCH_SEG_W,
                b.y + 4,
                SWITCH_SEG_W,
                SWITCH_SEG_H,
            )
        });
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
        let cell = 9;
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
        if self.look.step != old.step || self.look.bar != old.bar {
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
        cx.text(
            FontId::Bold,
            12.0,
            Rect::new(x + bw + 6, inner.y + 6, 40, inner.h),
            Align::Left,
            t.text_dim,
            "BPM",
        );
        // The view switch at the right end: one tap each.
        let (b, segs) = PlayTransport::switch(r);
        cx.canvas.fill_round_rect(b, 12, t.bg.mix(t.surface, 0.3));
        for (i, (sr, name)) in segs.iter().zip(VIEWS).enumerate() {
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
        let gw = 16 * 9 + 3 * 6;
        let dots = Rect::new(b.x - 22 - gw, inner.y + 6, gw, 18 + 26);
        self.paint_dots(cx, dots);
        // The tempo's tap area: its number and "BPM", the card's full height
        // (at least 64 px wide for a finger).
        let bpm = Rect::new(x - 8, r.y, (bw + 52).max(64), r.h);
        *self.painted.borrow_mut() =
            Some((r, self.look.clone(), TransportAreas { pos, dots, bpm }));
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let (_, segs) = PlayTransport::switch(r);
        let mut out: Vec<Target> = segs
            .iter()
            .enumerate()
            .map(|(i, s)| Target::tap("view", i, VIEWS[i], *s).disabled_if(i == self.look.view))
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
        }
        out
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let (_, segs) = PlayTransport::switch(r);
        if let Some(i) = segs.iter().position(|s| s.contains(x, y)) {
            return Some(EventKind::Tab(i));
        }
        let on_bpm = self
            .painted
            .borrow()
            .as_ref()
            .is_some_and(|(pr, _, a)| *pr == r && a.bpm.contains(x, y));
        on_bpm.then_some(EventKind::Select(TAP_BPM))
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
}

/// The pattern: a lane per sound (as many as fit, the ones that play
/// first, in pad order), 16 steps a page in groups of four, the playhead's
/// column lit, the step picked outlined. A tap on a step gives
/// `Cell { row: pad, col: step }`, on a lane's speaker `Toggle(pad)`. The
/// head's controls (Jay after live test 3: 32, 64 and 128 steps) give
/// `Tab(HEAD_PREV)` and `Tab(HEAD_NEXT)` for the page, `Tab(HEAD_SHORTER)`
/// and `Tab(HEAD_LONGER)` for the length.
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
    painted: RefCell<Option<(Rect, StepLook)>>,
}

/// The head's controls, as `Tab` indices.
pub const HEAD_PREV: usize = 0;
pub const HEAD_NEXT: usize = 1;
pub const HEAD_SHORTER: usize = 2;
pub const HEAD_LONGER: usize = 3;
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
}

/// The pattern head's controls, right to left: + and - round the length,
/// then › and ‹ round the page number. Each answers over the head's whole
/// height.
struct Head {
    title: Rect,
    prev: Rect,
    page: Rect,
    next: Rect,
    shorter: Rect,
    length: Rect,
    longer: Rect,
}

/// A head button's width.
const HEAD_BTN: i32 = 56;

impl Head {
    fn of(r: Rect) -> Head {
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
        Head {
            title: Rect::new(h.x, h.y, (prev.x - 12 - h.x).max(0), h.h),
            prev,
            page,
            next,
            shorter,
            length,
            longer,
        }
    }

    /// The control under (`x`, `y`), as its `Tab` index.
    fn at(&self, x: i32, y: i32) -> Option<usize> {
        [
            (self.prev, HEAD_PREV),
            (self.next, HEAD_NEXT),
            (self.shorter, HEAD_SHORTER),
            (self.longer, HEAD_LONGER),
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
        let (names, grid) = body.take_left(NAMES_W);
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

    /// The lanes shown in `rows` rows: the most wanted, in pad order.
    fn shown(&self, rows: usize) -> Vec<&Lane> {
        let mut v: Vec<&Lane> = self.lanes.iter().take(rows).collect();
        v.sort_by_key(|l| l.pad);
        v
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
        }
    }

    /// The middle of head control `k` (`HEAD_PREV` ...), for the app's
    /// tests.
    #[cfg(test)]
    pub fn head_at(r: Rect, k: usize) -> (i32, i32) {
        let h = Head::of(r);
        let b = [h.prev, h.next, h.shorter, h.longer][k];
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
    /// ‹ and ›, and the length with - and +.
    fn paint_head(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let hd = Head::of(r);
        let (h, _) = card_head(r);
        if !self.right.is_empty() {
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
        card(cx, r, &self.title, "");
        self.paint_head(cx, r);
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
            cx.canvas
                .fill_round_rect(Rect::new(nr.x, y + g.rh / 2 - 5, 10, 10), 3, c);
            let ink = if lane.muted { t.text_faint } else { t.text };
            cx.text(
                FontId::Sans,
                14.0,
                nr.take_left(18).1,
                Align::Left,
                ink,
                &lane.name,
            );
            // The mute button: a box round the speaker, red-tinted while
            // muted.
            if lane.muted {
                cx.canvas.fill_round_rect(mb, 6, t.raised.mix(t.bad, 0.3));
            } else {
                cx.canvas.fill_round_rect(mb, 6, t.raised);
            }
            cx.canvas.stroke_round_rect(mb, 6, 1, t.outline);
            speaker(cx, mb.x + mb.w / 2 + 1, mb.y + mb.h / 2, t.text, lane.muted);
            for s in 0..16 {
                let sr = g.cell(k, s);
                if s >= self.steps {
                    continue;
                }
                let v = lane.steps.get(s).copied().unwrap_or(0);
                if v == 0 {
                    let base = if s % 4 == 0 {
                        t.raised
                    } else {
                        t.raised.mix(t.surface, 0.45)
                    };
                    cx.canvas.fill_round_rect(sr, 5, base);
                } else {
                    let a = 0.35 + 0.65 * v as f32 / 127.0;
                    let mut fill = t.raised.mix(c, a);
                    if lane.muted {
                        fill = t.raised.mix(c, 0.25);
                    }
                    if self.play == Some(s) && !lane.muted {
                        fill = fill.mix(Color::rgb(0xFFFFFF), 0.35);
                    }
                    cx.canvas.fill_round_rect(sr, 5, fill);
                    // Velocity as a short line along the bottom.
                    let vw = ((sr.w - 8) as f32 * v as f32 / 127.0) as i32;
                    cx.canvas.fill_rect(
                        Rect::new(sr.x + 4, sr.bottom() - 5, vw.max(2), 2),
                        t.bg.with_alpha(120),
                    );
                }
                if self.selected == Some((lane.pad, s)) {
                    cx.canvas.stroke_round_rect(sr.inset(-3), 7, 2, t.text);
                }
            }
        }
        *self.painted.borrow_mut() = Some((r, self.look()));
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let g = self.geom(r);
        let mut out = Vec::new();
        // The head: ‹ › turn the pages of 16, - + halve or double the
        // steps (faint when they can't; a tap then says why).
        let h = Head::of(r);
        let (prev, next, shorter, longer) = self.usable();
        for (k, label, rect, can) in [
            (HEAD_PREV, "Previous page", h.prev, prev),
            (HEAD_NEXT, "Next page", h.next, next),
            (HEAD_SHORTER, "Fewer steps", h.shorter, shorter),
            (HEAD_LONGER, "More steps", h.longer, longer),
        ] {
            out.push(Target::tap("head", k, label, rect).disabled_if(!can));
        }
        for (k, lane) in self.shown(g.rows).iter().enumerate() {
            // The lane's whole name is its mute button (bigger mutes).
            out.push(Target::tap(
                "mute",
                lane.pad,
                format!("{} speaker", lane.name),
                g.mute_hit(k),
            ));
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

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        if let Some(i) = Head::of(r).at(x, y) {
            return Some(EventKind::Tab(i));
        }
        let g = self.geom(r);
        let lanes = self.shown(g.rows);
        for (k, lane) in lanes.iter().enumerate() {
            if g.mute_hit(k).contains(x, y) {
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
/// where the finger was.
pub struct XyPad {
    pub title: String,
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
            title: "XY · DJ filter + echo".into(),
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

    /// Where the finger has been, oldest first (the previews draw one).
    pub fn set_trail(&mut self, points: &[(f32, f32)]) {
        self.trail = points.iter().rev().take(TRAIL).rev().copied().collect();
    }
}

impl Widget for XyPad {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        let body = card(cx, r, &self.title, &self.right);
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
        vec![Target::drag("xy", 0, self.title.clone(), pad)]
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let pad = XyPad::pad(r);
        if pad.w <= 0 || pad.h <= 0 {
            return None;
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
        let (_, segs) = PlayTransport::switch(r);
        for (i, s) in segs.iter().enumerate() {
            assert_eq!(
                tr.touch(1, Touch::Down, s.x + s.w / 2, s.y + s.h / 2, r),
                Some(EventKind::Tab(i))
            );
        }
        assert_eq!(segs.len(), 3, "All, Beats, Loop");
        assert!(segs[2].right() <= r.right() - 16);
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
        }
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
