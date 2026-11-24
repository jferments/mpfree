//! The Looper tab's deck (the looper redesign, approved by Jay on 17 Nov):
//! every loop as the same channel, side by side, so the eye learns one
//! shape and finds it eight times.
//!
//! A channel is the loop's name and length, its ring, a level bar beside
//! the ring and its speaker under it. The ring is the loop: its wave drawn
//! round it, an arc that fills as it plays, a tick a bar and a hand at
//! now, and in the middle which bar is playing ("2 of 4"). Its colour is
//! its state: the loop's own colour playing, red recording, amber
//! overdubbing, grey muted; an empty loop is a dashed circle with what it
//! records from under it, and one waiting for its bar a red dashed one
//! with the bar it starts on.
//!
//! A tap on a ring does the loop's next step (record, play, overdub,
//! play), on the press, so a punch is where it was played. Held for
//! [`CLEAR_HOLD_S`] the ring clears the loop (its arc drains meanwhile).
//! A tap on a loop's name picks it without starting anything; the speaker
//! mutes it; the level bar beside the ring drags, or turns with the wheel
//! while a finger rests on it. While a layer is dragged out of the picked
//! loop's lanes the channels that take it light up, an empty one too (the
//! layer becomes a loop of its own there).
//!
//! The header has the song's clock (a dot a beat, the bar and beat, the
//! tempo) and the memory every loop shares.

use crate::looper_tab::{
    card_in, landing_fill, level_frac, level_of_frac, level_text, paint_card, paint_spots, DropTo,
    LayerDrag, LoopAct,
};
use mpfree_ui::draw::speaker_on;
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Shape, Target, Touch, Widget};
use std::cell::RefCell;
use std::f32::consts::PI;

/// The deck's widget name.
pub const DECK: &str = "ldeck";

/// How long a ring is held to clear its loop.
pub const CLEAR_HOLD_S: f32 = 1.0;

/// The header's height, the channels' margins and the space between them.
const HEAD_H: i32 = 46;
/// The header's right end kept for the memory gauge, and its buttons'
/// widths.
const MEMORY_W: i32 = 250;
const INPUT_W: i32 = 220;
const PADS_W: i32 = 160;
const SIDE: i32 = 14;
const GAP: i32 = 8;
/// A channel's name row and its ring (a fingertip and a half).
const NAME_H: i32 = 28;
const RING: i32 = 92;
/// The level bar beside the ring answers a finger this wide.
const LEVEL_HIT_W: i32 = 30;
/// The S beside the speaker, at most.
const SOLO_W: i32 = 44;

/// What a loop is doing, as its ring shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RingState {
    /// Nothing recorded yet.
    Empty,
    /// Armed: the first pass starts on this bar of the song (None while
    /// the transport is stopped: on the bar PLAY starts).
    Waiting(Option<i64>),
    Recording,
    Overdubbing,
    Playing,
    Muted,
    /// On a Loop track where another loop plays: silent, in its place.
    Off,
}

impl RingState {
    /// A loop's state from its status; an armed loop starts on
    /// `next_bar` (None while stopped).
    pub fn of(l: &mpfree_looper::Status, next_bar: Option<i64>) -> RingState {
        use mpfree_looper::Phase;
        match l.phase {
            // A Capture waits for its bar line or copies in: as armed.
            Phase::Empty if l.capturing.is_some() => RingState::Waiting(next_bar),
            Phase::Empty => RingState::Empty,
            Phase::Armed => RingState::Waiting(next_bar),
            Phase::Recording => RingState::Recording,
            Phase::Overdubbing => RingState::Overdubbing,
            Phase::Playing if l.muted => RingState::Muted,
            // Silent in its place: stopped on its Loop track, or another
            // loop is soloed.
            Phase::Playing if !l.on || l.solo_out => RingState::Off,
            Phase::Playing => RingState::Playing,
        }
    }

    /// The loop has sound (it can be muted, cleared and levelled).
    pub fn has_sound(self) -> bool {
        !matches!(self, RingState::Empty | RingState::Waiting(_))
    }
}

/// One loop on the deck.
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    /// "Loop 1".
    pub name: String,
    /// "4 bars"; empty while the loop has no length.
    pub length: String,
    pub colour: Color,
    pub state: RingState,
    /// Where it is in its cycle, 0 to 1 (recording: through its bar, or
    /// through its length when that is set).
    pub position: f32,
    /// Its bars, a tick each round the ring.
    pub bars: usize,
    /// The middle of the ring: big ("2", "13", "+") and small ("of 4",
    /// "REC on bar").
    pub centre: (String, String),
    /// Its wave round the ring, 0 to 255 a column; empty: none yet.
    pub wave: Vec<u8>,
    /// Its level, 0 to 2 (1 as recorded).
    pub level: f32,
    /// How loud it plays now, 0 to 1 along the level bar.
    pub meter: f32,
    /// Under an empty ring: what it records from ("Pads", "Line in"), or
    /// that UNDO brings back the loop cleared.
    pub foot: String,
    /// Soloed: the S beside the speaker lit.
    pub solo: bool,
    /// A loop on an audio track (phase 4, board 5): its name under its
    /// place ("Vox loop" under "Vocal · row 2"), both small on the name
    /// line; empty for the one-line name.
    pub sub: String,
    /// Whose M and S these are: "VOCAL'S TRACK" over them, and they show
    /// the track's mute and solo; empty for the loop's own.
    pub track: String,
    pub track_mute: bool,
    pub track_solo: bool,
    /// It holds a layer not yet on the stick (audio tracks phase 4c.3):
    /// a dot at its ring's top right.
    pub unsaved: bool,
}

impl Default for Channel {
    fn default() -> Self {
        Channel {
            name: String::new(),
            length: String::new(),
            colour: Color::rgb(0x6EA8FE),
            state: RingState::Empty,
            position: 0.0,
            bars: 0,
            centre: ("+".to_string(), String::new()),
            wave: Vec::new(),
            level: 1.0,
            meter: 0.0,
            foot: String::new(),
            solo: false,
            sub: String::new(),
            track: String::new(),
            track_mute: false,
            track_solo: false,
            unsaved: false,
        }
    }
}

/// The header's clock and memory.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeckHead {
    /// The beat now (from 0) and the beats in a bar; None while stopped.
    pub beat: Option<(usize, usize)>,
    /// "Bar 10 · beat 3", "Stopped".
    pub clock: String,
    /// "92 BPM".
    pub tempo: String,
    /// "37 s of 5 min", and how full, 0 to 1.
    pub memory: (String, f32),
    /// The pads are the loop keys (SHIFT + SAMPLER): the Pads switch lit.
    pub pads: bool,
    /// Records from: what every loop records ("Pads", "Line
    /// in", "Loops") and its level now, 0 to 1 along the meter. While a
    /// take holds another source than the one picked, the take's.
    pub input: (String, f32),
    /// While a take holds another source than the one picked: the pick,
    /// which the next take records (the chip says "This take").
    pub input_next: Option<String>,
    /// Capture, for Play's loop row (the Looper tab has it on What you
    /// played, over the bars it takes).
    pub capture: CaptureButton,
    /// Loops in audio cells (phase 4, board 5): the loops' gauge in the
    /// header's "LOOPS" place ("LOOPS 6 OF 8"); empty for "LOOPS".
    pub gauge: String,
    /// The input pill says where the next loop goes ("Records into Vocal
    /// · row 3 · Line L"), not what every loop records from.
    pub into: bool,
}

/// The header's Capture button: the last bars heard become a loop.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CaptureButton {
    /// "Capture last 2 bars", "Capture on the bar line", "Capturing".
    pub label: String,
    /// It has something to take now (else faint: a tap says why).
    pub works: bool,
    /// A capture waits for its bar line or copies in: lit, and a tap
    /// calls it off.
    pub lit: bool,
}

/// Where a channel's parts are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChannelParts {
    pub card: Rect,
    pub name: Rect,
    pub ring: Rect,
    pub level: Rect,
    pub level_hit: Rect,
    pub mute: Rect,
    /// The S beside the speaker: solo.
    pub solo: Rect,
}

/// A finger on a ring: which loop, how long it has stayed, and whether
/// the hold has cleared the loop.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RingPress {
    finger: u64,
    n: usize,
    from: (i32, i32),
    held: f32,
    cleared: bool,
}

/// What the deck drew last: where, its head, each channel's look, and
/// whether a dragged layer's card was over it.
type Drawn = (Rect, DeckHead, Vec<ChannelLook>, bool);

/// What a channel looked like when last drawn, to draw again only the
/// channels that changed.
#[derive(Clone, Debug, PartialEq)]
struct ChannelLook {
    ch: Channel,
    picked: bool,
    /// How much of a clear hold is left, in tenths of a percent.
    clearing: Option<u32>,
    /// The dragged layer would land here now: whether it can.
    landing: Option<bool>,
    lit: bool,
}

/// The loops side by side, the clock and the memory gauge.
#[derive(Default)]
pub struct LoopDeck {
    pub head: DeckHead,
    pub channels: Vec<Channel>,
    pub picked: usize,
    /// A layer dragged out of the lanes: the channels that take it lit,
    /// and its card while the finger is over the deck.
    pub drag: Option<LayerDrag>,
    press: Option<RingPress>,
    /// A finger on a level bar: its loop.
    levelling: Option<(u64, usize)>,
    drawn: RefCell<Option<Drawn>>,
    acts: Vec<LoopAct>,
}

impl LoopDeck {
    /// Every act the deck has kept since the last call.
    pub fn take(&mut self) -> Vec<LoopAct> {
        std::mem::take(&mut self.acts)
    }

    /// The header's strip.
    pub fn head_rect(r: Rect) -> Rect {
        Rect::new(r.x, r.y, r.w, HEAD_H)
    }

    /// The Pads switch in the header, left of Records from: the pads as
    /// loop keys (SHIFT + SAMPLER does the same).
    pub fn pads_rect(r: Rect) -> Rect {
        let c = LoopDeck::input_rect(r);
        Rect::new(c.x - 12 - PADS_W, c.y, PADS_W, c.h)
    }

    /// Records from in the header, left of the memory gauge: what every
    /// loop records, with its meter; a tap changes it.
    pub fn input_rect(r: Rect) -> Rect {
        let h = LoopDeck::head_rect(r);
        Rect::new(
            h.right() - 16 - MEMORY_W - INPUT_W,
            h.y + 5,
            INPUT_W,
            HEAD_H - 6,
        )
    }

    /// Channel `i`'s parts, of `n`, in the deck at `r`.
    pub fn parts(r: Rect, i: usize, n: usize) -> ChannelParts {
        let n = n.max(1) as i32;
        let area = Rect::new(r.x + SIDE, r.y + HEAD_H, r.w - 2 * SIDE, r.h - HEAD_H - 10);
        let w = (area.w - GAP * (n - 1)) / n;
        let card = Rect::new(area.x + i as i32 * (w + GAP), area.y, w, area.h);
        let ring = RING.min(card.w - 36).min(card.h - NAME_H - 52).max(24);
        let ring = Rect::new(card.x + 8, card.y + NAME_H, ring, ring);
        let level = Rect::new(card.right() - 17, ring.y + 8, 7, ring.h - 16);
        let level_hit = Rect::new(card.right() - LEVEL_HIT_W, ring.y, LEVEL_HIT_W, ring.h);
        let by = ring.bottom() + 8;
        let row = Rect::new(card.x + 8, by, card.w - 16, card.bottom() - 8 - by);
        let solo_w = (row.w / 3).clamp(30, SOLO_W);
        let mute = Rect::new(row.x, row.y, row.w - solo_w - 6, row.h);
        let solo = Rect::new(row.right() - solo_w, row.y, solo_w, row.h);
        ChannelParts {
            card,
            name: Rect::new(card.x, card.y, card.w, NAME_H),
            ring,
            level,
            level_hit,
            mute,
            solo,
        }
    }

    /// Every channel's card: where a dragged layer can land.
    pub fn cards(&self, r: Rect) -> Vec<Rect> {
        let n = self.channels.len();
        (0..n).map(|i| LoopDeck::parts(r, i, n).card).collect()
    }

    /// A finger has stayed on a ring a moment longer: held for
    /// [`CLEAR_HOLD_S`] it clears the loop (once). True when it just did.
    pub fn hold_tick(&mut self, dt: f32) -> bool {
        let Some(p) = self.press.as_mut() else {
            return false;
        };
        if p.cleared {
            return false;
        }
        p.held += dt.max(0.0);
        if p.held < CLEAR_HOLD_S {
            return false;
        }
        p.cleared = true;
        self.acts.push(LoopAct::Clear(p.n));
        true
    }

    /// A finger rests on a ring whose loop can be cleared (time passing
    /// matters to the deck).
    pub fn holding(&self) -> bool {
        self.press.is_some_and(|p| !p.cleared)
    }

    /// How much of the clear hold on loop `n` is left, 1 to 0.
    fn clearing(&self, n: usize) -> Option<f32> {
        self.press
            .filter(|p| p.n == n && !p.cleared && p.held > 0.0)
            .map(|p| (1.0 - p.held / CLEAR_HOLD_S).clamp(0.0, 1.0))
    }

    fn looks(&self) -> Vec<ChannelLook> {
        self.channels
            .iter()
            .enumerate()
            .map(|(i, ch)| ChannelLook {
                ch: ch.clone(),
                picked: i == self.picked,
                clearing: self.clearing(i).map(|f| (f * 1000.0) as u32),
                landing: self
                    .drag
                    .as_ref()
                    .and_then(|d| d.landing)
                    .and_then(|(to, ok)| (to == DropTo::Loop(i)).then_some(ok)),
                lit: self
                    .drag
                    .as_ref()
                    .is_some_and(|d| d.lit.contains(&DropTo::Loop(i))),
            })
            .collect()
    }

    /// The level a finger at height `y` on level bar `p` points to.
    fn level_at(p: &ChannelParts, y: i32) -> f32 {
        let f = (p.level.bottom() - y) as f32 / p.level.h.max(1) as f32;
        level_of_frac(f)
    }
}

/// The ring's wave, arc, ticks and hand, the middle's words, and while a
/// hold clears it the arc draining in red.
/// Capture's mark in `b`: a circle going back on itself, round a dot
/// (what was just played, taken back).
pub fn capture_icon(cx: &mut PaintCx, b: Rect, c: Color) {
    let (x, y) = (b.x as f32 + b.w as f32 / 2.0, b.y as f32 + b.h as f32 / 2.0);
    let r = b.w.min(b.h) as f32 / 2.0 - 1.5;
    let mut s = Shape::new(b.inset(-2));
    s.arc(x, y, r, 0.6 * PI, 2.35 * PI, 2.0);
    // The arrowhead where the circle ends, at its upper left.
    let (ex, ey) = (x + r * (0.6 * PI).cos(), y + r * (0.6 * PI).sin());
    s.segment((ex, ey), (ex - 4.0, ey - 1.0), 2.0);
    s.segment((ex, ey), (ex + 1.0, ey - 4.0), 2.0);
    s.disc(x, y, 2.6);
    s.draw(cx.canvas, c);
}

pub(crate) fn paint_ring(cx: &mut PaintCx, b: Rect, ch: &Channel, clearing: Option<f32>) {
    let t = cx.theme.clone();
    let c = b.w.min(b.h) as f32 / 2.0;
    let (ox, oy) = (b.x as f32 + b.w as f32 / 2.0, b.y as f32 + b.h as f32 / 2.0);
    let rs = c - 3.5;
    let r0 = c * 0.54;
    let r1 = c * 0.82;
    let bbox = b.inset(-6);
    let at = |a: f32, r: f32| (ox + r * a.cos(), oy + r * a.sin());
    let top = -PI / 2.0;
    let pos = ch.position.clamp(0.0, 1.0);
    let now = top + 2.0 * PI * pos;
    let dim = matches!(ch.state, RingState::Muted | RingState::Off);
    match ch.state {
        RingState::Empty | RingState::Waiting(_) => {
            let pts: Vec<(f32, f32)> = (0..=96)
                .map(|k| at(top + 2.0 * PI * k as f32 / 96.0, rs))
                .collect();
            let mut s = Shape::new(bbox);
            s.dashed(&pts, 3.0, 7.0);
            let c = if ch.state == RingState::Empty {
                t.outline.mix(t.text_faint, 0.35)
            } else {
                t.bad
            };
            s.draw(cx.canvas, c);
        }
        _ => {
            let mut s = Shape::new(bbox);
            s.arc(ox, oy, rs, 0.0, 2.0 * PI, 5.0);
            s.draw(cx.canvas, t.bg.mix(t.surface, 0.6));
            // The wave: the part played this time round brighter.
            let ink = if dim { t.text_faint } else { ch.colour };
            let n = ch.wave.len();
            let (mut past, mut ahead) = (Shape::new(bbox), Shape::new(bbox));
            for (k, &v) in ch.wave.iter().enumerate() {
                let f = k as f32 / n as f32;
                if ch.state == RingState::Recording && f > pos {
                    continue;
                }
                let a = top + 2.0 * PI * f;
                let len = 1.2 + v as f32 / 255.0 * (r1 - r0);
                let seg = (at(a, r0), at(a, r0 + len));
                if f <= pos && !dim {
                    past.segment(seg.0, seg.1, 2.0);
                } else {
                    ahead.segment(seg.0, seg.1, 2.0);
                }
            }
            past.draw(cx.canvas, ink);
            ahead.draw(cx.canvas, ink.mix(t.surface, if dim { 0.45 } else { 0.58 }));
            let state = match ch.state {
                RingState::Recording => t.bad,
                RingState::Overdubbing => t.warn,
                _ => ch.colour,
            };
            let mut s = Shape::new(bbox);
            if dim {
                s.arc(ox, oy, rs, 0.0, 2.0 * PI, 3.0);
                s.draw(cx.canvas, t.text_faint);
            } else if pos > 0.001 {
                s.arc(ox, oy, rs, top, now, 5.0);
                s.draw(cx.canvas, state);
            }
            if ch.bars > 1 && ch.state != RingState::Recording {
                let mut s = Shape::new(bbox);
                for k in 0..ch.bars {
                    let a = top + 2.0 * PI * k as f32 / ch.bars as f32;
                    s.segment(at(a, rs - 9.0), at(a, rs - 5.0), 2.0);
                }
                s.draw(cx.canvas, t.text.mix(t.surface, 0.55));
            }
            if !dim {
                let mut s = Shape::new(bbox);
                s.segment(at(now, r0 - 5.0), at(now, rs + 2.0), 2.5);
                s.draw(cx.canvas, t.text);
            }
        }
    }
    if let Some(left) = clearing {
        let mut s = Shape::new(bbox);
        if left > 0.001 {
            s.arc(ox, oy, rs, top, top + 2.0 * PI * left, 5.0);
        }
        s.draw(cx.canvas, t.bad);
    }
    // The middle: which bar, or what a tap does.
    let big_ink = match ch.state {
        RingState::Empty => t.text_faint,
        RingState::Waiting(_) | RingState::Recording => t.bad,
        RingState::Overdubbing => t.warn,
        RingState::Muted | RingState::Off => t.text_faint,
        RingState::Playing => t.text,
    };
    let big_px = (c * 0.5).clamp(14.0, 23.0);
    let (big, small) = &ch.centre;
    let small = if clearing.is_some() {
        "hold: clear"
    } else {
        small
    };
    let mid = b.y + b.h / 2;
    if small.is_empty() {
        let r = Rect::new(b.x, mid - 14, b.w, 28);
        cx.text(FontId::Bold, big_px, r, Align::Center, big_ink, big);
    } else {
        let r = Rect::new(b.x, mid - 19, b.w, 26);
        cx.text(FontId::Bold, big_px, r, Align::Center, big_ink, big);
        let r = Rect::new(b.x + 6, mid + 6, b.w - 12, 15);
        let ink = if clearing.is_some() {
            t.bad
        } else {
            t.text_dim
        };
        cx.text(FontId::Bold, 11.0, r, Align::Center, ink, small);
    }
}

impl LoopDeck {
    fn paint_head(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let h = LoopDeck::head_rect(r);
        cx.canvas
            .fill_rect(Rect::new(h.x + 8, h.y + 6, h.w - 16, h.h - 8), t.surface);
        let line = Rect::new(h.x + 16, h.y + 10, h.w - 32, 24);
        let label = if self.head.gauge.is_empty() {
            "LOOPS"
        } else {
            self.head.gauge.as_str()
        };
        cx.text(FontId::Bold, 12.0, line, Align::Left, t.text_dim, label);
        let mut x = line.x + 68.max(cx.measure(FontId::Bold, 12.0, label) + 14);
        if let Some((beat, beats)) = self.head.beat {
            for b in 0..beats.min(8) {
                let c = if b == beat {
                    t.text
                } else if b == 0 {
                    t.outline.mix(t.text_faint, 0.5)
                } else {
                    t.outline.mix(t.text_faint, 0.25)
                };
                let mut s = Shape::new(Rect::new(x, line.y + 4, 14, 16));
                s.disc(x as f32 + 6.0, line.y as f32 + 12.0, 6.0);
                s.draw(cx.canvas, c);
                x += 18;
            }
            x += 6;
        }
        let w = cx.measure(FontId::Bold, 15.0, &self.head.clock);
        cx.text(
            FontId::Bold,
            15.0,
            Rect::new(x, line.y, w + 4, line.h),
            Align::Left,
            t.text,
            &self.head.clock,
        );
        cx.text(
            FontId::Sans,
            15.0,
            Rect::new(x + w + 14, line.y, 120, line.h),
            Align::Left,
            t.text_dim,
            &self.head.tempo,
        );
        // The Pads switch: lit while the pads are the loop keys.
        let pads = LoopDeck::pads_rect(r);
        let pill = pads.inset_xy(0, 3);
        if self.head.pads {
            cx.canvas.fill_round_rect(pill, pill.h / 2, t.picked());
        } else {
            cx.canvas.fill_round_rect(pill, pill.h / 2, t.raised);
        }
        let dot = Rect::new(pill.x + 14, pill.y + pill.h / 2 - 5, 10, 10);
        let mut s = Shape::new(dot.inset(-2));
        s.disc(dot.x as f32 + 5.0, dot.y as f32 + 5.0, 5.0);
        s.draw(
            cx.canvas,
            if self.head.pads {
                t.on_picked()
            } else {
                t.text_faint
            },
        );
        cx.text(
            FontId::Bold,
            13.0,
            Rect::new(
                dot.right() + 8,
                pill.y,
                pill.right() - dot.right() - 14,
                pill.h,
            ),
            Align::Left,
            if self.head.pads {
                t.on_picked()
            } else {
                t.text_dim
            },
            "Pads as loop keys",
        );
        // Records from: what every loop records, on one line when it fits
        // ("Records from Line in"), else its caption over it; its meter lit
        // to the level (five bars). While a take holds another source than
        // the one picked, the take's under "This take", and the pick as the
        // next take's when that fits.
        let pill = LoopDeck::input_rect(r).inset_xy(0, 3);
        cx.canvas.fill_round_rect(pill, t.round(8), t.raised);
        let (word, level) = &self.head.input;
        let meter_w = 5 * 7;
        let ww = cx.measure(FontId::Bold, 14.0, word);
        let (cap, held) = match &self.head.input_next {
            Some(next) => {
                let both = format!("THIS TAKE · NEXT: {}", next.to_uppercase());
                let fits = cx.measure(FontId::Bold, 10.5, &both) <= pill.w - 24;
                (if fits { both } else { "THIS TAKE".to_string() }, true)
            }
            None if self.head.into => ("Records into".to_string(), false),
            None => ("Records from".to_string(), false),
        };
        let cw = cx.measure(FontId::Sans, 12.0, &cap);
        let one_line = !held && cw + 8 + ww + 10 + meter_w <= pill.w - 16;
        let (mx, word_rect) = if one_line {
            let total = cw + 8 + ww + 10 + meter_w;
            let x0 = pill.x + ((pill.w - total) / 2).max(8);
            cx.text(
                FontId::Sans,
                12.0,
                Rect::new(x0, pill.y, cw + 2, pill.h),
                Align::Left,
                t.text_dim,
                &cap,
            );
            (
                x0 + cw + 8 + ww + 10,
                Rect::new(x0 + cw + 8, pill.y, ww + 2, pill.h),
            )
        } else {
            let x0 = pill.x + 12;
            let caps = if held {
                cap.clone()
            } else {
                cap.to_uppercase()
            };
            cx.text(
                FontId::Bold,
                10.5,
                Rect::new(x0, pill.y + 3, pill.w - 24, 13),
                Align::Left,
                if held { t.accent } else { t.text_dim },
                &caps,
            );
            let mx = pill.right() - 12 - meter_w;
            (mx, Rect::new(x0, pill.y + 15, mx - x0 - 6, pill.h - 17))
        };
        cx.text(FontId::Bold, 14.0, word_rect, Align::Left, t.text, word);
        let lit = (level.clamp(0.0, 1.0) * 5.0).round() as i32;
        for k in 0..5 {
            let bh = 6 + k * 3;
            let b = Rect::new(mx + k * 7, pill.y + pill.h / 2 + 9 - bh, 5, bh);
            let c = if k < lit {
                if k == 4 {
                    t.bad
                } else {
                    t.good
                }
            } else {
                t.outline
            };
            cx.canvas.fill_round_rect(b, 2, c);
        }
        // The memory every loop shares, on the right.
        let (text, full) = &self.head.memory;
        let tw = cx.measure(FontId::Sans, 12.0, text);
        let right = line.right();
        cx.text(
            FontId::Sans,
            12.0,
            Rect::new(right - tw - 2, line.y, tw + 2, line.h),
            Align::Right,
            t.text_dim,
            text,
        );
        let bar = Rect::new(right - tw - 12 - 70, line.y + 9, 70, 6);
        cx.canvas.fill_round_rect(bar, 3, t.raised);
        let full = full.clamp(0.0, 1.0);
        if full > 0.0 {
            let w = ((bar.w as f32 * full) as i32).max(6);
            let c = if full > 0.9 { t.bad } else { t.accent };
            cx.canvas
                .fill_round_rect(Rect::new(bar.x, bar.y, w, bar.h), 3, c);
        }
        cx.text(
            FontId::Bold,
            12.0,
            Rect::new(bar.x - 64, line.y, 58, line.h),
            Align::Right,
            t.text_dim,
            "Memory",
        );
    }

    fn paint_channel(&self, cx: &mut PaintCx, r: Rect, i: usize, look: &ChannelLook) {
        let t = cx.theme.clone();
        let p = LoopDeck::parts(r, i, self.channels.len());
        let ch = &look.ch;
        cx.canvas
            .fill_rect(p.card.inset(-GAP / 2).intersect(r_body(r)), t.surface);
        let fill = landing_fill(&t, self.drag.as_ref(), DropTo::Loop(i))
            .unwrap_or_else(|| t.surface.mix(t.raised, 0.4));
        cx.canvas.fill_round_rect(p.card, t.round(12), fill);
        if look.picked {
            cx.canvas
                .stroke_round_rect(p.card, t.round(12), 2, t.accent);
        } else {
            cx.canvas
                .stroke_round_rect(p.card, t.round(12), 1, t.outline);
        }
        if look.lit && look.landing.is_none() {
            cx.canvas
                .stroke_round_rect(p.card, t.round(12), 2, t.accent);
        }
        // The name and the length.
        let mut s = Shape::new(Rect::new(p.name.x + 6, p.name.y + 8, 14, 14));
        s.disc(p.name.x as f32 + 14.0, p.name.y as f32 + 15.0, 4.0);
        s.draw(cx.canvas, ch.colour);
        let ink = if ch.state == RingState::Empty {
            t.text_dim
        } else {
            t.text
        };
        if !ch.sub.is_empty() {
            // A loop on an audio track: its place, and its name under it.
            let top = Rect::new(p.name.x + 23, p.name.y + 2, p.name.w - 30, 14);
            cx.text(FontId::Bold, 12.5, top, Align::Left, ink, &ch.name);
            let under = Rect::new(p.name.x + 23, p.name.y + 15, p.name.w - 30, 13);
            cx.text(FontId::Sans, 11.0, under, Align::Left, t.text_dim, &ch.sub);
        } else {
            let line = Rect::new(p.name.x + 23, p.name.y + 5, p.name.w - 30, 20);
            cx.text(FontId::Bold, 15.0, line, Align::Left, ink, &ch.name);
        }
        if p.card.w >= 110 && ch.sub.is_empty() {
            let line = Rect::new(p.name.x, p.name.y + 6, p.name.w - 10, 20);
            cx.text(
                FontId::Sans,
                12.0,
                line,
                Align::Right,
                t.text_dim,
                &ch.length,
            );
        }
        paint_ring(cx, p.ring, ch, look.clearing.map(|c| c as f32 / 1000.0));
        if ch.unsaved && ch.state.has_sound() {
            // A layer not yet on the stick (phase 4c.3): the warning's
            // amber dot, between the ring and the level bar.
            let (x, y) = (p.ring.right() + 4, p.ring.y + 4);
            let mut s = Shape::new(Rect::new(x - 5, y - 5, 10, 10));
            s.disc(x as f32, y as f32, 4.0);
            s.draw(cx.canvas, t.warn);
        }
        if ch.state.has_sound() {
            // The level bar: how loud it plays, and its level (the knob).
            cx.canvas.fill_round_rect(p.level, 4, t.raised);
            let m = (p.level.h as f32 * ch.meter.clamp(0.0, 1.0)) as i32;
            let c = if matches!(ch.state, RingState::Muted | RingState::Off) {
                t.text_faint
            } else {
                ch.colour
            };
            if m > 0 {
                cx.canvas.fill_round_rect(
                    Rect::new(p.level.x, p.level.bottom() - m, p.level.w, m),
                    4,
                    c.mix(t.surface, 0.15),
                );
            }
            let y = p.level.bottom() - (p.level.h as f32 * level_frac(ch.level)) as i32;
            cx.canvas
                .fill_round_rect(Rect::new(p.level.x - 3, y - 2, 13, 4), 2, t.text);
            // The speaker: the loop's, or its track's (phase 4, board 5),
            // with whose over it.
            let (muted, soloed, by) = if ch.track.is_empty() {
                (ch.state == RingState::Muted, ch.solo, 0)
            } else {
                (ch.track_mute, ch.track_solo, 5)
            };
            let b = if muted {
                t.bad.mix(t.bg, 0.72)
            } else {
                t.raised
            };
            cx.canvas.fill_round_rect(p.mute, t.round(10), b);
            let (sx, sy) = (p.mute.x + p.mute.w / 2, p.mute.y + p.mute.h / 2 + by);
            speaker_on(cx, sx, sy, if muted { t.bad } else { t.text }, muted, b);
            // The solo: lit while this loop (its track) is soloed.
            let (fill, ink) = if soloed {
                (t.picked(), t.on_picked())
            } else {
                (t.raised, t.text_dim)
            };
            cx.canvas.fill_round_rect(p.solo, t.round(10), fill);
            cx.text(
                FontId::Bold,
                15.0,
                Rect::new(p.solo.x, p.solo.y + by, p.solo.w, p.solo.h),
                Align::Center,
                ink,
                "S",
            );
            if !ch.track.is_empty() {
                let cap = Rect::new(
                    p.mute.x + 6,
                    p.mute.y + 2,
                    p.solo.right() - p.mute.x - 12,
                    12,
                );
                cx.text(
                    FontId::Bold,
                    9.5,
                    cap,
                    Align::Left,
                    t.readable(t.text_dim, b),
                    &ch.track,
                );
            }
        } else if cx.measure(FontId::Sans, 12.0, &ch.foot) > 2 * (p.mute.w - 8) {
            // A long foot ("REC into an empty audio cell fills it"):
            // wrapped, in as many lines as it takes.
            let lines = wrap_words(cx, &ch.foot, 12.0, p.card.w - 16);
            let lh = 15;
            let top = p.mute.y + (p.mute.h - lh * lines.len() as i32) / 2;
            for (i, line) in lines.iter().enumerate() {
                cx.text(
                    FontId::Sans,
                    12.0,
                    Rect::new(p.card.x + 8, top + i as i32 * lh, p.card.w - 16, lh),
                    Align::Center,
                    t.text_dim,
                    line,
                );
            }
        } else {
            // What it records from, on two lines when it doesn't fit on
            // one ("Pads' / instrument").
            let fits = cx.measure(FontId::Sans, 13.0, &ch.foot) <= p.mute.w - 8;
            match ch.foot.split_once(' ').filter(|_| !fits) {
                Some((a, b)) => {
                    let m = p.mute;
                    let half = m.h / 2;
                    for (i, line) in [a, b].into_iter().enumerate() {
                        let y = m.y + half - 17 + i as i32 * 17;
                        cx.text(
                            FontId::Sans,
                            13.0,
                            Rect::new(m.x, y, m.w, 17),
                            Align::Center,
                            t.text_dim,
                            line,
                        );
                    }
                }
                None => {
                    cx.text(
                        FontId::Sans,
                        13.0,
                        p.mute,
                        Align::Center,
                        t.text_dim,
                        &ch.foot,
                    );
                }
            }
        }
    }
}

/// `text` in lines no wider than `w` at `size`, broken between words.
fn wrap_words(cx: &mut PaintCx, text: &str, size: f32, w: i32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split(' ') {
        match lines.last_mut() {
            Some(l) if cx.measure(FontId::Sans, size, &format!("{l} {word}")) <= w => {
                l.push(' ');
                l.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    lines
}

/// The deck's area below its header (the channels' backdrop).
fn r_body(r: Rect) -> Rect {
    Rect::new(r.x, r.y + HEAD_H, r.w, r.h - HEAD_H)
}

impl Widget for LoopDeck {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        cx.panel(r);
        self.paint_head(cx, r);
        let looks = self.looks();
        for (i, look) in looks.iter().enumerate() {
            self.paint_channel(cx, r, i, look);
        }
        let over = self.drag.as_ref().filter(|d| r.contains(d.at.0, d.at.1));
        if let Some(d) = &self.drag {
            let spots: Vec<(DropTo, Rect)> = self
                .cards(r)
                .into_iter()
                .enumerate()
                .map(|(i, c)| (DropTo::Loop(i), c))
                .collect();
            paint_spots(cx, d, &spots);
        }
        if let Some(d) = over {
            let d = LayerDrag {
                card: card_in(r, d.at),
                ..d.clone()
            };
            paint_card(cx, &d);
        }
        *self.drawn.borrow_mut() = Some((r, self.head.clone(), looks, over.is_some()));
    }

    fn changed_parts(&self, r: Rect) -> Option<Vec<Rect>> {
        let drawn = self.drawn.borrow();
        let (at, head, looks, card) = drawn.as_ref()?;
        // A dragged layer's card can cross channels: all of it.
        let over = self
            .drag
            .as_ref()
            .is_some_and(|d| r.contains(d.at.0, d.at.1));
        if *at != r || *card || over || looks.len() != self.channels.len() {
            return None;
        }
        let now = self.looks();
        let mut parts = Vec::new();
        if *head != self.head {
            parts.push(LoopDeck::head_rect(r));
        }
        for (i, (a, b)) in looks.iter().zip(&now).enumerate() {
            if a != b {
                let card = LoopDeck::parts(r, i, now.len()).card;
                parts.push(card.inset(-GAP / 2).intersect(r_body(r)));
            }
        }
        Some(parts)
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let n = self.channels.len();
        let mut out = vec![
            Target::tap(
                "pads-keys",
                0,
                if self.head.pads {
                    "Pads as loop keys (on)"
                } else {
                    "Pads as loop keys"
                },
                LoopDeck::pads_rect(r),
            ),
            Target::tap(
                "records-from",
                0,
                match &self.head.input_next {
                    Some(next) => format!(
                        "Records from: this take {}, the next take {next}",
                        self.head.input.0
                    ),
                    None => format!("Records from: {}", self.head.input.0),
                },
                LoopDeck::input_rect(r),
            ),
        ];
        for (i, ch) in self.channels.iter().enumerate() {
            let p = LoopDeck::parts(r, i, n);
            out.push(Target::tap("loop", i, ch.name.clone(), p.name));
            let does = match ch.state {
                RingState::Empty => "record",
                RingState::Waiting(_) => "cancel",
                RingState::Recording => "close and play",
                RingState::Overdubbing => "play",
                RingState::Playing => "overdub",
                RingState::Muted => "unmute",
                RingState::Off => "overdub",
            };
            out.push(Target::tap(
                "ring",
                i,
                format!("{} ring: {does}", ch.name),
                p.ring,
            ));
            if ch.state.has_sound() {
                out.push(Target::tap(
                    "loop-mute",
                    i,
                    format!("{} speaker", ch.name),
                    p.mute,
                ));
                out.push(Target::tap(
                    "loop-solo",
                    i,
                    format!("{} solo{}", ch.name, if ch.solo { " (on)" } else { "" }),
                    p.solo,
                ));
                out.push(Target::drag(
                    "loop-level",
                    i,
                    format!("{} level {}", ch.name, level_text(ch.level)),
                    p.level_hit,
                ));
            }
        }
        out
    }

    fn steps(&self, t: &Target) -> bool {
        t.id == "loop-level"
    }

    fn step(&mut self, t: &Target, delta: i32, r: Rect) -> Option<EventKind> {
        self.held_step(t, delta, false, r)
    }

    /// SHIFT + the wheel's push: the loop's level back to 0 dB.
    fn reset(&mut self, t: &Target, _r: Rect) -> Option<EventKind> {
        if t.id != "loop-level" {
            return None;
        }
        let ch = self.channels.get_mut(t.index)?;
        if (ch.level - 1.0).abs() < 1e-6 {
            return None;
        }
        ch.level = 1.0;
        self.acts.push(LoopAct::Level(t.index, 1.0));
        Some(EventKind::Value(1.0))
    }

    fn held_step(&mut self, t: &Target, delta: i32, fine: bool, _r: Rect) -> Option<EventKind> {
        if t.id != "loop-level" {
            return None;
        }
        let ch = self.channels.get_mut(t.index)?;
        let db = (20.0 * ch.level.max(1e-6).log10()).max(-60.0);
        let db = (db + delta as f32 * if fine { 0.1 } else { 0.5 }).clamp(-60.0, 6.0);
        let level = if db <= -60.0 {
            0.0
        } else {
            10f32.powf(db / 20.0)
        };
        if (level - ch.level).abs() < 1e-6 {
            return None;
        }
        ch.level = level;
        self.acts.push(LoopAct::Level(t.index, level));
        Some(EventKind::Value(level))
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let n = self.channels.len();
        match phase {
            Touch::Down => {
                if LoopDeck::pads_rect(r).contains(x, y) {
                    self.acts.push(LoopAct::PadKeys);
                    return Some(EventKind::Toggle(0));
                }
                if LoopDeck::input_rect(r).contains(x, y) {
                    self.acts
                        .push(LoopAct::Strip(crate::looper_tab::Ctl::Input));
                    return Some(EventKind::Tab(0));
                }
                let i = (0..n).find(|&i| LoopDeck::parts(r, i, n).card.contains(x, y))?;
                let p = LoopDeck::parts(r, i, n);
                let state = self.channels[i].state;
                if state.has_sound() && p.mute.contains(x, y) {
                    self.acts.push(LoopAct::Mute(i));
                    return Some(EventKind::Toggle(i));
                }
                if state.has_sound() && p.solo.contains(x, y) {
                    self.acts.push(LoopAct::Solo(i));
                    return Some(EventKind::Toggle(i));
                }
                if state.has_sound() && p.level_hit.contains(x, y) {
                    self.levelling = Some((finger, i));
                    let level = LoopDeck::level_at(&p, y);
                    self.channels[i].level = level;
                    self.acts.push(LoopAct::Level(i, level));
                    return Some(EventKind::Value(level));
                }
                if p.ring.inset(-4).contains(x, y) {
                    // On the press: a punch is where it was played.
                    self.acts.push(LoopAct::Tap(i));
                    if state.has_sound() {
                        self.press = Some(RingPress {
                            finger,
                            n: i,
                            from: (x, y),
                            held: 0.0,
                            cleared: false,
                        });
                    }
                    return Some(EventKind::Select(i));
                }
                if p.name.contains(x, y) {
                    self.acts.push(LoopAct::Pick(i));
                    return Some(EventKind::Select(i));
                }
                None
            }
            Touch::Move => {
                if let Some((f, i)) = self.levelling.filter(|l| l.0 == finger) {
                    let p = LoopDeck::parts(r, i, n);
                    let level = LoopDeck::level_at(&p, y);
                    let _ = f;
                    if (level - self.channels.get(i)?.level).abs() < 1e-4 {
                        return None;
                    }
                    self.channels[i].level = level;
                    self.acts.push(LoopAct::Level(i, level));
                    return Some(EventKind::Value(level));
                }
                // A finger that slides off is no hold.
                if let Some(p) = self.press.filter(|p| p.finger == finger) {
                    let (dx, dy) = (x - p.from.0, y - p.from.1);
                    if dx.abs().max(dy.abs()) > 24 {
                        self.press = None;
                    }
                }
                None
            }
            Touch::Up | Touch::Cancel => {
                if self.levelling.is_some_and(|l| l.0 == finger) {
                    self.levelling = None;
                }
                if self.press.is_some_and(|p| p.finger == finger) {
                    self.press = None;
                }
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deck() -> (LoopDeck, Rect) {
        let d = LoopDeck {
            channels: (0..8)
                .map(|i| Channel {
                    name: format!("Loop {}", i + 1),
                    state: if i < 2 {
                        RingState::Playing
                    } else {
                        RingState::Empty
                    },
                    ..Channel::default()
                })
                .collect(),
            ..LoopDeck::default()
        };
        (d, Rect::new(12, 84, 1116, 240))
    }

    #[test]
    fn eight_channels_fit_with_rings_a_fingertip_and_a_half_wide() {
        let (_, r) = deck();
        let a = LoopDeck::parts(r, 0, 8);
        let h = LoopDeck::parts(r, 7, 8);
        assert!(a.card.w >= 125, "{:?}", a.card);
        assert_eq!(a.ring.w, RING);
        assert!(a.mute.h >= 48, "a speaker a fingertip tall: {:?}", a.mute);
        assert!(h.card.right() <= r.right() - SIDE + 1);
        assert!(
            a.level_hit.x > a.ring.right() - 8,
            "the level bar is beside the ring"
        );
    }

    /// Records from sits left of the memory, the Pads switch left of it,
    /// and a tap on it changes what every loop records. (Capture is on
    /// What you played, over the bars it takes, not up here.)
    #[test]
    fn records_from_in_the_header_changes_the_input() {
        let (mut d, r) = deck();
        let c = LoopDeck::input_rect(r);
        let p = LoopDeck::pads_rect(r);
        assert!(p.right() < c.x && c.right() < r.right() - MEMORY_W);
        assert!(c.h >= 40, "a fingertip tall: {c:?}");
        d.head.input = ("Line in".into(), 0.4);
        let ts = d.targets(r);
        let t = ts
            .iter()
            .find(|t| t.id == "records-from")
            .expect("Records from");
        assert!(t.enabled && t.label == "Records from: Line in");
        assert!(!ts.iter().any(|t| t.id == "capture"));
        let (x, y) = (c.x + c.w / 2, c.y + c.h / 2);
        assert!(d.touch(1, Touch::Down, x, y, r).is_some());
        assert_eq!(
            d.take(),
            vec![LoopAct::Strip(crate::looper_tab::Ctl::Input)]
        );
        d.touch(1, Touch::Up, x, y, r);
    }

    #[test]
    fn a_ring_acts_on_the_press_and_a_hold_clears_the_loop() {
        let (mut d, r) = deck();
        let p = LoopDeck::parts(r, 1, 8);
        let (x, y) = (p.ring.x + p.ring.w / 2, p.ring.y + p.ring.h / 2);
        assert_eq!(d.touch(1, Touch::Down, x, y, r), Some(EventKind::Select(1)));
        assert_eq!(d.take(), vec![LoopAct::Tap(1)], "on the press");
        assert!(d.holding());
        assert!(!d.hold_tick(CLEAR_HOLD_S / 2.0));
        assert!(d.hold_tick(CLEAR_HOLD_S / 2.0 + 0.01));
        assert_eq!(d.take(), vec![LoopAct::Clear(1)]);
        assert!(!d.hold_tick(1.0), "once");
        d.touch(1, Touch::Up, x, y, r);
        // Lifted early: nothing cleared.
        d.touch(2, Touch::Down, x, y, r);
        d.hold_tick(0.5);
        d.touch(2, Touch::Up, x, y, r);
        assert_eq!(d.take(), vec![LoopAct::Tap(1)]);
        // An empty ring has nothing to clear.
        let p = LoopDeck::parts(r, 4, 8);
        d.touch(3, Touch::Down, p.ring.x + 40, p.ring.y + 40, r);
        assert!(!d.holding());
        assert_eq!(d.take(), vec![LoopAct::Tap(4)]);
    }

    #[test]
    fn the_name_picks_the_speaker_mutes_and_the_level_bar_drags() {
        let (mut d, r) = deck();
        let p = LoopDeck::parts(r, 0, 8);
        d.touch(1, Touch::Down, p.name.x + 30, p.name.y + 10, r);
        d.touch(2, Touch::Down, p.mute.x + 20, p.mute.y + 10, r);
        assert_eq!(d.take(), vec![LoopAct::Pick(0), LoopAct::Mute(0)]);
        let x = p.level_hit.x + 10;
        d.touch(3, Touch::Down, x, p.level.y, r);
        d.touch(3, Touch::Move, x, p.level.bottom(), r);
        d.touch(3, Touch::Up, x, p.level.bottom(), r);
        let acts = d.take();
        assert!(
            matches!(acts.first(), Some(LoopAct::Level(0, l)) if *l > 1.9),
            "{acts:?}"
        );
        assert!(
            matches!(acts.last(), Some(LoopAct::Level(0, l)) if *l == 0.0),
            "{acts:?}"
        );
        // An empty loop has no speaker or level: its name picks it.
        let p = LoopDeck::parts(r, 5, 8);
        d.touch(4, Touch::Down, p.mute.x + 20, p.mute.y + 10, r);
        assert!(d.take().is_empty());
    }

    #[test]
    fn the_wheel_turns_a_level_bar_in_half_decibels() {
        let (mut d, r) = deck();
        let t = d
            .targets(r)
            .into_iter()
            .find(|t| t.id == "loop-level" && t.index == 1)
            .unwrap();
        assert!(d.steps(&t));
        d.held_step(&t, -2, false, r);
        d.held_step(&t, 1, true, r);
        let acts = d.take();
        let db = |a: &LoopAct| match a {
            LoopAct::Level(1, l) => 20.0 * l.log10(),
            _ => f32::NAN,
        };
        assert!((db(&acts[0]) + 1.0).abs() < 1e-3, "{acts:?}");
        assert!((db(&acts[1]) + 0.9).abs() < 1e-3, "{acts:?}");
        // SHIFT + the wheel's push: back to 0 dB, then nothing to do.
        assert_eq!(d.reset(&t, r), Some(EventKind::Value(1.0)));
        assert_eq!(d.take(), [LoopAct::Level(1, 1.0)]);
        assert_eq!(d.reset(&t, r), None);
    }
}
