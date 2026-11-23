//! The Notes stage on the chain editor (layout spec section 4b, board 7,
//! approved by Jay 2026-11-17 19:19Z): a note track's MIDI effects and its
//! arp, top to bottom, above the track's sound, with a Plays on row
//! between them; and the page of the one selected, in the effect page's
//! frame: its name and what it does, chips for its main choice, what goes
//! in and what comes out on a keyboard, and its settings in cards.
//!
//! The engine is `mpfree-notefx`. The app keeps the settings in the
//! project (`mpfree_seq::Track::note_fx`) and fills [`NotesState`]; what a
//! touch here means comes back as [`NoteAction`]s inside
//! [`crate::ChainAction::Notes`]. Q-Links 1 to 4 turn the effect's four
//! ([`mpfree_notefx::Fx::qlinks`]) as parameters of
//! [`Owner::NoteFx`] (track, the effect's id): [`param_views`] gives them
//! to the chain state, so the Q-Link column works as it does for a sound
//! effect.

use mpfree_fx::mpfree_node::{Curve, ParamInfo, Unit};
use mpfree_notefx::settings::Kind as SetKind;
use mpfree_notefx::{Fx, Item, TrackFx, MAX_FX};
use mpfree_param::{Owner, ParamAddr};
use mpfree_ui::{Align, Color, FontId, PaintCx, Rect, Target, Theme};

use crate::state::ParamView;

/// A row of a list with Notes shown (both groups are compact, so both fit).
pub const ROW_H: i32 = 44;
pub const ROW_GAP: i32 = 4;
/// "+ MIDI effect" and, under the Sound group, "+ Add effect".
pub const ADD_H: i32 = 38;
/// The Plays on row.
pub const PLAYS_H: i32 = 58;
/// The Sound group's caption.
pub const CAPTION_H: i32 = 28;
/// Notes rows shown before the list scrolls.
pub const NOTES_SHOWN: usize = 4;
/// The scroll arrows in a caption.
const ARROW_W: i32 = 30;
/// The ON pill's part of a row.
const POWER_W: i32 = 52;

/// The Notes group: what the app tells the chain editor about the track.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NotesState {
    /// The track, from 0.
    pub track: u8,
    pub fx: TrackFx,
    /// The arp's row: what it does now ("Up · 1/16 · 2 oct") and whether
    /// it plays (a mode other than Direct).
    pub arp: String,
    pub arp_on: bool,
    /// The row whose page shows (an index into [`TrackFx::items`]); None
    /// while a sound effect's page shows.
    pub selected: Option<usize>,
    /// The MIDI effect picker is up ("+ MIDI effect").
    pub picking: bool,
    /// The Plays on row: where the track's notes play ("Its own sound",
    /// "Juno · ch 2") and a line under it.
    pub plays_on: String,
    pub plays_detail: String,
    /// Why the controllers the track sends to MIDI out go nowhere ("sends
    /// nothing: the track plays its own sound"), for the CC effects'
    /// header in place of what they do; None while they go out.
    pub ccs_send_nothing: Option<String>,
}

impl NotesState {
    /// The rows, top to bottom.
    pub fn items(&self) -> Vec<Item> {
        self.fx.items()
    }

    pub fn item(&self, row: usize) -> Option<Item> {
        self.items().get(row).copied()
    }

    /// The selected row's effect: its place, its id and its settings.
    pub fn page_fx(&self) -> Option<(usize, u16, Fx)> {
        match self.item(self.selected?)? {
            Item::Fx(p) => Some((p, self.fx.ids[p], self.fx.slots[p]?.fx)),
            Item::Arp => None,
        }
    }

    /// The arp's row is selected: its page shows.
    pub fn arp_page(&self) -> bool {
        self.selected.and_then(|r| self.item(r)) == Some(Item::Arp)
    }

    /// A page of the Notes group shows (an effect's or the arp's).
    pub fn page_shown(&self) -> bool {
        !self.picking && self.selected.and_then(|r| self.item(r)).is_some()
    }

    /// Row `row`'s name, its line under it and whether it plays.
    pub fn row_view(&self, row: usize) -> Option<(String, String, bool)> {
        Some(match self.item(row)? {
            Item::Fx(p) => {
                let s = self.fx.slots[p]?;
                (s.fx.name().to_string(), s.fx.summary(), s.on)
            }
            Item::Arp => ("Arp".to_string(), self.arp.clone(), self.arp_on),
        })
    }

    /// "Notes 2 of 4".
    pub fn position(&self, row: usize) -> String {
        format!("Notes {} of {}", row + 1, self.items().len())
    }

    /// Where row `row` sits among the others: "after Scale, before Arp",
    /// "first, before Arp", "after Arp, last", "the only one".
    pub fn neighbours(&self, row: usize) -> String {
        let n = self.items().len();
        let name = |r: usize| self.row_view(r).map(|v| v.0).unwrap_or_default();
        match (row > 0, row + 1 < n) {
            (true, true) => format!("after {}, before {}", name(row - 1), name(row + 1)),
            (false, true) => format!("first, before {}", name(row + 1)),
            (true, false) => format!("after {}, last", name(row - 1)),
            (false, false) => "the only one".to_string(),
        }
    }
}

/// What a touch on the Notes group or its page means, for the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteAction {
    /// Row `row` of the Notes list was tapped: show its page.
    Select(usize),
    /// Switch row `row`'s effect on (`on`) or off (the arp's row has no
    /// switch here: its page opens the arp).
    SetOn { row: usize, on: bool },
    /// Move row `from` to row `to` (the arp too: the effects it passes
    /// change side).
    Move { from: usize, to: usize },
    /// Take out row `row`'s effect.
    Remove(usize),
    /// "+ MIDI effect" (true) or the picker's Cancel (false).
    Picker(bool),
    /// Add effect `i` of [`Fx::all`] above the arp.
    Add(usize),
    /// Set setting `k` of the effect with id `id` to `value`.
    Set { id: u16, k: usize, value: i32 },
    /// The Plays on row: where the track plays (the app opens the MIDI
    /// section's Tracks out).
    PlaysOn,
    /// The arp's page: open the Arp panel on this track.
    OpenArp,
}

// ---- Parameters, for the Q-Links -------------------------------------------

/// Setting `k` of the effect with id `id` on `track`.
pub const fn addr(track: u8, id: u16, k: usize) -> ParamAddr {
    ParamAddr::new(Owner::NoteFx(track, id), k as u16)
}

/// The unit a setting reads in on a Q-Link card.
fn unit_of(fx: &Fx, k: usize) -> Unit {
    match (fx, k) {
        (Fx::Transpose { .. }, _) | (Fx::Echo(_), 5) | (Fx::Randomize(_), 0) => Unit::Semitones,
        (Fx::Chord(_), 4) | (Fx::Echo(_), 4) | (Fx::Chance { .. }, _) | (Fx::Randomize(_), 1) => {
            Unit::Percent
        }
        (Fx::Chord(_), 5) | (Fx::Length { .. }, 2) | (Fx::Echo(_), 2) => Unit::Ms,
        (Fx::Humanize { .. }, 0) | (Fx::Strum(_), 0) | (Fx::Randomize(_), 3) => Unit::Ms,
        (Fx::CcSmooth(_), 1) => Unit::Ms,
        _ => Unit::None,
    }
}

/// Setting `k` of `fx` as a parameter: its range, how a Q-Link steps it
/// (one value a detent) and its value now as the default.
pub fn info(fx: &Fx, k: usize) -> Option<ParamInfo> {
    let s = fx.settings().get(k)?;
    Some(ParamInfo {
        id: k as u16,
        name: s.name,
        group: s.section,
        min: s.min() as f32,
        max: s.max() as f32,
        default: fx.get(k) as f32,
        unit: unit_of(fx, k),
        curve: match s.kind {
            SetKind::Choice(list) => Curve::Choice(list),
            SetKind::Number { .. } => Curve::Stepped,
        },
        readonly: false,
    })
}

/// The selected effect's settings as parameter views: every one its page
/// shows, and the four the Q-Links turn.
pub fn param_views(track: u8, id: u16, fx: &Fx) -> (Vec<ParamView>, Vec<ParamView>) {
    let view = |k: usize| {
        let info = info(fx, k)?;
        let value = fx.get(k) as f32;
        Some(ParamView {
            addr: addr(track, id, k),
            info,
            label: info.name.to_string(),
            value,
            pos: info.to_unit(value),
            mod_range: None,
            macro_: None,
            locked: false,
            xy: None,
        })
    };
    let all = (0..fx.settings().len())
        .filter(|&k| fx.shows(k))
        .filter_map(view)
        .collect();
    let four = fx.qlinks().iter().filter_map(|&k| view(k)).collect();
    (all, four)
}

// ---- The list --------------------------------------------------------------

/// The Notes group in the chain column, under the list's caption (which
/// reads NOTES), and what it leaves for the Sound group.
#[derive(Clone, Debug, PartialEq)]
pub struct ListLayout {
    /// Each row shown and its row in [`NotesState::items`].
    pub rows: Vec<(Rect, usize)>,
    /// How many rows show at once, and the first shown.
    pub fit: usize,
    pub from: usize,
    /// "+ MIDI effect", while a place is free.
    pub add: Option<Rect>,
    /// Up and down in the NOTES caption, when the rows don't all fit.
    pub arrows: Option<(Rect, Rect)>,
    pub plays: Rect,
    /// The Sound group's caption, and the rest of the column for its rows
    /// and its Add.
    pub sound_caption: Rect,
    pub sound: Rect,
}

/// The scroll arrows at a caption's right.
pub fn caption_arrows(caption: Rect) -> (Rect, Rect) {
    let y = caption.y + (caption.h - 24) / 2 - 3;
    let down = Rect::new(caption.right() - ARROW_W, y, ARROW_W, 24);
    let up = Rect::new(down.x - ARROW_W - 4, y, ARROW_W, 24);
    (up, down)
}

/// Where the Notes group goes in `body` (the list's card under its
/// caption, above its buttons), its rows from row `from`. `caption`: the
/// NOTES caption, for its arrows.
pub fn list_layout(caption: Rect, body: Rect, ns: &NotesState, from: usize) -> ListLayout {
    let n = ns.items().len();
    let fit = n.min(NOTES_SHOWN);
    let from = from.min(n - fit);
    let mut y = body.y;
    let rows = (from..from + fit)
        .map(|i| {
            let r = Rect::new(body.x, y, body.w, ROW_H);
            y += ROW_H + ROW_GAP;
            (r, i)
        })
        .collect();
    let add = (ns.fx.count() < MAX_FX).then(|| {
        let r = Rect::new(body.x, y, body.w, ADD_H);
        y += ADD_H + ROW_GAP;
        r
    });
    let arrows = (n > fit).then(|| caption_arrows(caption));
    let plays = Rect::new(body.x, y + 4, body.w, PLAYS_H);
    y = plays.bottom() + 8;
    let sound_caption = Rect::new(body.x, y, body.w, CAPTION_H);
    let sound = Rect::new(
        body.x,
        sound_caption.bottom(),
        body.w,
        (body.bottom() - sound_caption.bottom()).max(0),
    );
    ListLayout {
        rows,
        fit,
        from,
        add,
        arrows,
        plays,
        sound_caption,
        sound,
    }
}

/// A row of either group: its number, name, line and ON pill.
#[allow(clippy::too_many_arguments)]
pub fn paint_row(
    cx: &mut PaintCx,
    t: &Theme,
    row: Rect,
    number: usize,
    name: &str,
    line: &str,
    on: bool,
    sel: bool,
) {
    cx.canvas.fill_round_rect(
        row,
        t.round(10),
        if sel {
            t.raised
        } else {
            t.surface.mix(t.raised, 0.35)
        },
    );
    if sel {
        cx.canvas
            .fill_round_rect(Rect::new(row.x, row.y + 10, 4, row.h - 20), 2, t.accent);
    }
    let ink = t.button_ink(on);
    cx.text(
        FontId::Sans,
        t.size_small - 2.0,
        Rect::new(row.x + 8, row.y, 20, row.h),
        Align::Center,
        t.text_dim,
        &number.to_string(),
    );
    let (power, text) = row.take_right(POWER_W);
    let text = Rect::new(row.x + 34, row.y + 3, text.right() - row.x - 36, row.h - 6);
    let (top, bottom) = text.take_top(text.h / 2 + 1);
    cx.text(FontId::Bold, t.size_body - 3.0, top, Align::Left, ink, name);
    cx.text(
        FontId::Sans,
        t.size_small - 3.0,
        bottom,
        Align::Left,
        t.text_dim,
        if on { line } else { "off" },
    );
    let pill = Rect::new(power.x + 4, power.y + (power.h - 22) / 2, 40, 22);
    cx.canvas.fill_round_rect(
        pill,
        11,
        if on {
            t.good.mix(t.surface, 0.3)
        } else {
            t.outline
        },
    );
    cx.text(
        FontId::Bold,
        t.size_small - 3.0,
        pill,
        Align::Center,
        if on { t.on_accent } else { t.text_dim },
        if on { "ON" } else { "OFF" },
    );
}

/// A small chevron, its top left at `(x, y)`, pointing up or down.
fn chevron(cx: &mut PaintCx, x: i32, y: i32, up: bool, c: Color) {
    for i in 0..5 {
        let y = if up { y + 4 - i } else { y + i };
        cx.canvas.fill_rect(Rect::new(x + i, y, 2, 2), c);
        cx.canvas.fill_rect(Rect::new(x + 9 - i, y, 2, 2), c);
    }
}

/// The ON pill's part of a row.
pub fn pill_of(row: Rect) -> Rect {
    row.take_right(POWER_W).0
}

/// A group's caption: its name at the left, its count and (when the rows
/// don't fit) its arrows at the right.
#[allow(clippy::too_many_arguments)]
pub fn paint_caption(
    cx: &mut PaintCx,
    t: &Theme,
    caption: Rect,
    name: &str,
    count: &str,
    arrows: Option<(Rect, Rect)>,
    up: bool,
    down: bool,
) {
    let cap = caption.inset_xy(6, 0).take_top(CAPTION_H - 6).0;
    cx.text(
        FontId::Bold,
        t.size_small - 1.0,
        cap,
        Align::Left,
        t.text_dim,
        name,
    );
    let right = match arrows {
        Some((u, d)) => {
            for (r, ok, pointing_up) in [(u, up, true), (d, down, false)] {
                cx.canvas.fill_round_rect(
                    r,
                    t.round(6),
                    if ok {
                        t.raised
                    } else {
                        t.surface.mix(t.raised, 0.3)
                    },
                );
                let ink = t.button_ink(ok);
                chevron(cx, r.x + r.w / 2 - 5, r.y + r.h / 2 - 3, pointing_up, ink);
            }
            u.x - 8
        }
        None => cap.right(),
    };
    cx.text(
        FontId::Sans,
        t.size_small - 1.0,
        Rect::new(cap.x, cap.y, (right - cap.x).max(0), cap.h),
        Align::Right,
        t.text_dim,
        count,
    );
}

/// The Notes group: its rows, "+ MIDI effect" and the Plays on row (the
/// caption is the list's own).
pub fn paint_list(cx: &mut PaintCx, t: &Theme, l: &ListLayout, ns: &NotesState) {
    for &(r, i) in &l.rows {
        if let Some((name, line, on)) = ns.row_view(i) {
            paint_row(cx, t, r, i + 1, &name, &line, on, ns.selected == Some(i));
        }
    }
    if let Some(a) = l.add {
        cx.canvas.stroke_round_rect(a, t.round(10), 1, t.outline);
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            a,
            Align::Center,
            t.text,
            "+  MIDI effect",
        );
    }
    let p = l.plays;
    cx.canvas
        .fill_round_rect(p, t.round(10), t.surface.mix(t.raised, 0.35));
    let inner = p.inset_xy(12, 4);
    let (top, rest) = inner.take_top(16);
    cx.text(
        FontId::Bold,
        t.size_small - 4.0,
        top,
        Align::Left,
        t.text_dim,
        "PLAYS ON  ↓",
    );
    let (mid, low) = rest.take_top(rest.h / 2 + 2);
    cx.text(
        FontId::Bold,
        t.size_small - 1.0,
        mid,
        Align::Left,
        t.text,
        &ns.plays_on,
    );
    let w = cx.measure(FontId::Bold, t.size_small - 1.0, &ns.plays_on);
    chevron(cx, mid.x + w + 8, mid.y + mid.h / 2 - 2, false, t.text_dim);
    cx.text(
        FontId::Sans,
        t.size_small - 4.0,
        low,
        Align::Left,
        t.text_dim,
        &ns.plays_detail,
    );
}

// ---- The page --------------------------------------------------------------

/// The settings each effect shows as chips over its page (the main
/// choice: the chord's shape, the scale's root).
fn chip_setting(fx: &Fx) -> Option<usize> {
    match fx {
        Fx::Chord(_) | Fx::Scale { .. } => Some(0),
        _ => None,
    }
}

/// The note played into the page's keyboard: one a scale moves.
fn played_note(fx: &Fx) -> u8 {
    match fx {
        Fx::Scale { .. } => 49,
        _ => 48,
    }
}

/// Whether the effect changes pitches (its page draws the keyboard).
fn draws_keys(fx: &Fx) -> bool {
    matches!(fx, Fx::Transpose { .. } | Fx::Scale { .. } | Fx::Chord(_))
}

const NAMES: [&str; 12] = [
    "C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
];

/// "C3" for note 48 (middle C, 60, is C4).
pub fn note_name(n: u8) -> String {
    format!("{}{}", NAMES[usize::from(n % 12)], i32::from(n / 12) - 1)
}

fn is_black(n: u8) -> bool {
    matches!(n % 12, 1 | 3 | 6 | 8 | 10)
}

/// One setting's row in a card.
#[derive(Clone, Debug, PartialEq)]
pub struct RowLayout {
    pub k: usize,
    pub label: Rect,
    /// A few choices: a button each and the value it sets.
    pub seg: Vec<(Rect, i32)>,
    /// More choices, or a number: - and + round the value.
    pub minus: Option<Rect>,
    pub value: Rect,
    pub plus: Option<Rect>,
}

/// A card of settings: its section's title and its rows.
#[derive(Clone, Debug, PartialEq)]
pub struct CardLayout {
    pub rect: Rect,
    pub title: &'static str,
    pub rows: Vec<RowLayout>,
}

/// The page of the selected row, or the MIDI effect picker.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageLayout {
    /// The chips (setting `chip_k`): each one shown and the value it sets,
    /// and the arrows at the ends when they don't all fit.
    pub chip_k: Option<usize>,
    pub chips: Vec<(Rect, i32)>,
    pub chip_arrows: Option<(Rect, Rect)>,
    /// What goes in and what comes out, for an effect that moves pitches.
    pub keys: Option<Rect>,
    pub cards: Vec<CardLayout>,
    /// The arp's page: the button to its panel.
    pub open_arp: Option<Rect>,
    /// The picker: its Cancel and a tile per effect of [`Fx::all`].
    pub cancel: Option<Rect>,
    pub tiles: Vec<Rect>,
}

/// A chip's width for `name`.
fn chip_w(name: &str) -> i32 {
    30 + 10 * name.chars().count() as i32
}

/// Chips (each with its value) and, when they don't all fit, the arrows
/// at the row's ends.
type ChipRow = (Vec<(Rect, i32)>, Option<(Rect, Rect)>);

/// Chips for `names` across `row`, all of them if they fit, else a run
/// round `current` with an arrow at each end.
fn chip_row(row: Rect, names: &[&str], current: usize) -> ChipRow {
    const GAP: i32 = 8;
    let widths: Vec<i32> = names.iter().map(|n| chip_w(n)).collect();
    let need = |a: usize, b: usize| widths[a..b].iter().sum::<i32>() + GAP * (b - a) as i32 - GAP;
    let place = |a: usize, b: usize, x0: i32| {
        let mut x = x0;
        (a..b)
            .map(|i| {
                let r = Rect::new(x, row.y, widths[i], row.h);
                x += widths[i] + GAP;
                (r, i as i32)
            })
            .collect::<Vec<_>>()
    };
    if names.is_empty() || need(0, names.len()) <= row.w {
        return (place(0, names.len(), row.x), None);
    }
    let room = row.w - 2 * (44 + GAP);
    let (mut a, mut b) = (current, current + 1);
    loop {
        if b < names.len() && need(a, b + 1) <= room {
            b += 1;
        } else if a > 0 && need(a - 1, b) <= room {
            a -= 1;
        } else {
            break;
        }
    }
    let prev = Rect::new(row.x, row.y, 44, row.h);
    let next = Rect::new(row.right() - 44, row.y, 44, row.h);
    (place(a, b, prev.right() + GAP), Some((prev, next)))
}

/// A card's rows: inline (the name left, the control right) when the
/// card is wide, else the name over the control.
fn card_rows(inner: Rect, fx: &Fx, ks: &[usize]) -> Vec<RowLayout> {
    let wide = inner.w >= 380;
    let row_h = if wide { 58 } else { 74 };
    let mut y = inner.y;
    ks.iter()
        .map(|&k| {
            let row = Rect::new(inner.x, y, inner.w, row_h);
            y += row_h;
            let (label, ctl) = if wide {
                let (l, c) = row.take_left(inner.w - 250);
                (l, Rect::new(c.x, c.y + (c.h - 44) / 2, c.w, 44))
            } else {
                let (l, c) = row.take_top(24);
                (l, Rect::new(c.x, c.y + 2, c.w, 44))
            };
            let s = fx.settings()[k];
            match s.kind {
                SetKind::Choice(list) if list.len() <= 4 => {
                    let n = list.len() as i32;
                    let w = (ctl.w - 4) / n;
                    let seg = (0..n)
                        .map(|i| (Rect::new(ctl.x + 2 + i * w, ctl.y + 2, w, ctl.h - 4), i))
                        .collect();
                    RowLayout {
                        k,
                        label,
                        seg,
                        minus: None,
                        value: ctl,
                        plus: None,
                    }
                }
                _ => {
                    let minus = Rect::new(ctl.x, ctl.y, 52, ctl.h);
                    let plus = Rect::new(ctl.right() - 52, ctl.y, 52, ctl.h);
                    RowLayout {
                        k,
                        label,
                        seg: Vec::new(),
                        minus: Some(minus),
                        value: Rect::new(minus.right(), ctl.y, plus.x - minus.right(), ctl.h),
                        plus: Some(plus),
                    }
                }
            }
        })
        .collect()
}

/// Where the Notes page goes in `page` (the effect column between the
/// header and the Modulate row).
pub fn page_layout(page: Rect, ns: &NotesState) -> PageLayout {
    let mut out = PageLayout::default();
    if ns.picking {
        let (head, body) = page.inset(14).take_top(52);
        out.cancel = Some(Rect::new(head.right() - 120, head.y + 6, 120, 40));
        let all = Fx::all();
        let cols = 4;
        let gap = 10;
        let w = (body.w - gap * (cols - 1)) / cols;
        // As tall as the rows of them fit, up to 120.
        let rows = (all.len() as i32 + cols - 1) / cols;
        let h = ((body.h - 8 - gap * (rows - 1)) / rows).min(120);
        out.tiles = (0..all.len() as i32)
            .map(|i| {
                Rect::new(
                    body.x + (i % cols) * (w + gap),
                    body.y + 8 + (i / cols) * (h + gap),
                    w,
                    h,
                )
            })
            .collect();
        return out;
    }
    if ns.arp_page() {
        let card = page.take_top(220).0;
        out.open_arp = Some(Rect::new(card.x + 20, card.bottom() - 70, 260, 50));
        return out;
    }
    let Some((_, _, fx)) = ns.page_fx() else {
        return out;
    };
    let mut rest = page;
    if let Some(k) = chip_setting(&fx) {
        if let SetKind::Choice(list) = fx.settings()[k].kind {
            let (row, r) = rest.take_top(44);
            let (chips, arrows) = chip_row(row, list, fx.get(k).max(0) as usize);
            out.chip_k = Some(k);
            out.chips = chips;
            out.chip_arrows = arrows;
            rest = r.take_top(10).1;
        }
    }
    if draws_keys(&fx) {
        let (keys, r) = rest.take_top(176);
        out.keys = Some(keys);
        rest = r.take_top(12).1;
    }
    // The settings by section, in order, the chips' one left out.
    let mut sections: Vec<(&'static str, Vec<usize>)> = Vec::new();
    for (k, s) in fx.settings().iter().enumerate() {
        if Some(k) == out.chip_k || !fx.shows(k) {
            continue;
        }
        match sections.iter_mut().find(|(name, _)| *name == s.section) {
            Some((_, ks)) => ks.push(k),
            None => sections.push((s.section, vec![k])),
        }
    }
    let n = sections.len().clamp(1, 3) as i32;
    let gap = 12;
    let w = (rest.w - gap * (n - 1)) / n;
    out.cards = sections
        .into_iter()
        .take(3)
        .enumerate()
        .map(|(i, (title, ks))| {
            let rect = Rect::new(rest.x + i as i32 * (w + gap), rest.y, w, rest.h);
            let inner = rect.inset_xy(18, 12).take_top(34).1;
            CardLayout {
                rect,
                title,
                rows: card_rows(inner, &fx, &ks),
            }
        })
        .collect();
    out
}

/// What a tap on the page asks: a chip, an arrow, a button of a card, the
/// arp's button, or the picker's.
pub fn tap_page(l: &PageLayout, ns: &NotesState, x: i32, y: i32) -> Option<NoteAction> {
    if l.cancel.is_some_and(|r| r.contains(x, y)) {
        return Some(NoteAction::Picker(false));
    }
    if let Some(i) = l.tiles.iter().position(|r| r.contains(x, y)) {
        return Some(NoteAction::Add(i));
    }
    if l.open_arp.is_some_and(|r| r.contains(x, y)) {
        return Some(NoteAction::OpenArp);
    }
    let (_, id, fx) = ns.page_fx()?;
    let set =
        |k: usize, value: i32| {
            let s = fx.settings()[k];
            (value >= s.min() && value <= s.max() && value != fx.get(k))
                .then_some(NoteAction::Set { id, k, value })
        };
    if let Some(k) = l.chip_k {
        if let Some(&(_, v)) = l.chips.iter().find(|(r, _)| r.contains(x, y)) {
            return set(k, v);
        }
        if let Some((prev, next)) = l.chip_arrows {
            if prev.contains(x, y) {
                return set(k, fx.get(k) - 1);
            }
            if next.contains(x, y) {
                return set(k, fx.get(k) + 1);
            }
        }
    }
    for row in l.cards.iter().flat_map(|c| &c.rows) {
        if let Some(&(_, v)) = row.seg.iter().find(|(r, _)| r.contains(x, y)) {
            return set(row.k, v);
        }
        if row.minus.is_some_and(|r| r.contains(x, y)) {
            return set(row.k, fx.get(row.k) - 1);
        }
        if row.plus.is_some_and(|r| r.contains(x, y)) {
            return set(row.k, fx.get(row.k) + 1);
        }
    }
    None
}

/// The setting a finger rests on at (`x`, `y`) on a MIDI effect's page,
/// for the data wheel to turn while it stays down (Jay, 17 Nov: hold a
/// control and turn the wheel): the chips' row with its arrows, or a
/// card's row (its name, buttons, value and – / +), with that row's
/// rectangle.
pub fn setting_at(l: &PageLayout, x: i32, y: i32) -> Option<(usize, Rect)> {
    let span = |rects: &mut dyn Iterator<Item = Rect>| rects.reduce(|a, b| a.union(b));
    if let Some(k) = l.chip_k {
        let arrows = l.chip_arrows.into_iter().flat_map(|(a, b)| [a, b]);
        let row = span(&mut l.chips.iter().map(|c| c.0).chain(arrows));
        if let Some(r) = row.filter(|r| r.contains(x, y)) {
            return Some((k, r));
        }
    }
    l.cards.iter().flat_map(|c| &c.rows).find_map(|row| {
        let parts = [Some(row.label), row.minus, Some(row.value), row.plus];
        let r = span(
            &mut parts
                .into_iter()
                .flatten()
                .chain(row.seg.iter().map(|s| s.0)),
        )?;
        r.contains(x, y).then_some((row.k, r))
    })
}

/// Setting `k` of the MIDI effect whose page shows, `delta` steps on (a
/// choice's next or earlier one), held at its ends; None when it can't
/// move further.
pub fn step_setting(ns: &NotesState, k: usize, delta: i32) -> Option<NoteAction> {
    let (_, id, fx) = ns.page_fx()?;
    let s = fx.settings().get(k)?;
    let now = fx.get(k);
    let value = (now + delta).clamp(s.min(), s.max());
    (value != now).then_some(NoteAction::Set { id, k, value })
}

/// The page's tap targets, for the smoke tests and the outline.
pub fn page_targets(l: &PageLayout, ns: &NotesState, out: &mut Vec<Target>) {
    if let Some(c) = l.cancel {
        out.push(Target::tap("notes-cancel", 0, "Cancel", c));
    }
    let all = Fx::all();
    for (i, r) in l.tiles.iter().enumerate() {
        out.push(Target::tap("notes-pick", i, all[i].name(), *r));
    }
    if let Some(r) = l.open_arp {
        out.push(Target::tap("notes-open-arp", 0, "Open the arp", r));
    }
    let Some((_, _, fx)) = ns.page_fx() else {
        return;
    };
    if let Some(k) = l.chip_k {
        let names = match fx.settings()[k].kind {
            SetKind::Choice(list) => list,
            SetKind::Number { .. } => &[],
        };
        let now = fx.get(k);
        for &(r, v) in &l.chips {
            let name = names.get(v as usize).copied().unwrap_or("");
            out.push(Target::tap("notes-chip", v as usize, name, r).disabled_if(v == now));
        }
        if let Some((prev, next)) = l.chip_arrows {
            let s = fx.settings()[k];
            out.push(Target::tap("notes-chip-prev", 0, "‹", prev).disabled_if(now <= s.min()));
            out.push(Target::tap("notes-chip-next", 0, "›", next).disabled_if(now >= s.max()));
        }
    }
    for row in l.cards.iter().flat_map(|c| &c.rows) {
        let s = fx.settings()[row.k];
        let now = fx.get(row.k);
        let names = match s.kind {
            SetKind::Choice(list) => list,
            SetKind::Number { .. } => &[],
        };
        for &(r, v) in &row.seg {
            let label = format!(
                "{} {}",
                s.name,
                names.get(v as usize).copied().unwrap_or("")
            );
            out.push(Target::tap("notes-seg", row.k, label, r).disabled_if(v == now));
        }
        if let Some(r) = row.minus {
            out.push(
                Target::tap("notes-minus", row.k, format!("{} -", s.name), r)
                    .disabled_if(now <= s.min()),
            );
        }
        if let Some(r) = row.plus {
            out.push(
                Target::tap("notes-plus", row.k, format!("{} +", s.name), r)
                    .disabled_if(now >= s.max()),
            );
        }
    }
}

/// The header's title and line for a Notes page: the name, then where it
/// sits and what it does ("Notes 2 of 4 · adds notes to each one
/// played"); a CC effect on a track that plays its own sound says it
/// sends nothing instead. None with no row picked.
pub fn header_text(ns: &NotesState) -> Option<(String, String)> {
    if ns.picking {
        return Some((
            "Add a MIDI effect".to_string(),
            "It goes above the arp; move it with Up and Down".to_string(),
        ));
    }
    let row = ns.selected?;
    let does = match ns.page_fx() {
        Some((_, _, fx)) if fx.is_cc() => {
            ns.ccs_send_nothing.as_deref().unwrap_or_else(|| fx.does())
        }
        Some((_, _, fx)) => fx.does(),
        None => "plays the held notes in turn",
    };
    let name = ns.row_view(row).map(|v| v.0).unwrap_or_default();
    Some((name, format!("{} · {does}", ns.position(row))))
}

/// The header for a Notes page ([`header_text`]).
pub fn paint_header(cx: &mut PaintCx, t: &Theme, header: Rect, ns: &NotesState) {
    cx.canvas.fill_round_rect(header, t.radius, t.surface);
    let Some((title, line)) = header_text(ns) else {
        return;
    };
    let inner = header.inset_xy(16, 0);
    let w = cx.measure(FontId::Bold, t.size_title, &title);
    cx.text(
        FontId::Bold,
        t.size_title,
        inner,
        Align::Left,
        t.text,
        &title,
    );
    let x = inner.x + w + 18;
    cx.text(
        FontId::Sans,
        t.size_small,
        Rect::new(x, inner.y, (inner.right() - x).max(0), inner.h),
        Align::Left,
        t.text_dim,
        &line,
    );
}

/// The Modulate row's place for a MIDI effect: what the Q-Links turn.
pub fn paint_modulate(cx: &mut PaintCx, t: &Theme, row: Rect, ns: &NotesState) {
    cx.canvas.fill_round_rect(row, t.radius, t.surface);
    let text = match ns.page_fx() {
        Some((_, _, fx)) => {
            let names: Vec<&str> = fx.qlinks().iter().map(|&k| fx.settings()[k].name).collect();
            format!("Q-Links turn {}", join_and(&names))
        }
        None if ns.arp_page() => "The arp's settings are on its panel".to_string(),
        None => String::new(),
    };
    cx.text(
        FontId::Sans,
        t.size_small,
        row.inset_xy(18, 0),
        Align::Left,
        t.text_dim,
        &text,
    );
}

/// `text` in lines no wider than `w`, broken between words.
fn wrap(cx: &mut PaintCx, size: f32, text: &str, w: i32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split(' ') {
        match lines.last_mut() {
            Some(line) if cx.measure(FontId::Sans, size, &format!("{line} {word}")) <= w => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    lines
}

/// "a, b and c".
fn join_and(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn paint_button(cx: &mut PaintCx, t: &Theme, r: Rect, label: &str, on: bool, usable: bool) {
    cx.canvas.fill_round_rect(
        r,
        t.round(10),
        if on {
            t.picked()
        } else if usable {
            t.raised
        } else {
            t.surface.mix(t.raised, 0.3)
        },
    );
    cx.text(
        if on { FontId::Bold } else { FontId::Sans },
        t.size_small,
        r,
        Align::Center,
        if on {
            t.on_picked()
        } else {
            t.button_ink(usable)
        },
        label,
    );
}

/// The page: the picker, the arp's, or the selected effect's.
pub fn paint_page(cx: &mut PaintCx, t: &Theme, page: Rect, l: &PageLayout, ns: &NotesState) {
    if ns.picking {
        cx.canvas.fill_round_rect(page, t.radius, t.surface);
        if let Some(c) = l.cancel {
            paint_button(cx, t, c, "Cancel", false, true);
        }
        let all = Fx::all();
        for (fx, r) in all.iter().zip(&l.tiles) {
            cx.canvas.fill_round_rect(*r, t.round(12), t.raised);
            let inner = r.inset(16);
            let (top, rest) = inner.take_top(30);
            cx.text(
                FontId::Bold,
                t.size_body,
                top,
                Align::Left,
                t.text,
                fx.name(),
            );
            let size = t.size_small - 2.0;
            let mut y = rest.y;
            for line in wrap(cx, size, fx.does(), rest.w) {
                let r = Rect::new(rest.x, y, rest.w, 22);
                cx.text(FontId::Sans, size, r, Align::Left, t.text_dim, &line);
                y += 22;
            }
        }
        return;
    }
    if ns.arp_page() {
        let card = page.take_top(220).0;
        cx.canvas.fill_round_rect(card, t.radius, t.surface);
        let inner = card.inset_xy(20, 14);
        let (cap, rest) = inner.take_top(26);
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            cap,
            Align::Left,
            t.text_dim,
            "THE ARP",
        );
        let (line, rest) = rest.take_top(36);
        let what = if ns.arp_on {
            ns.arp.clone()
        } else {
            "Off: the notes play as they come".to_string()
        };
        cx.text(FontId::Bold, t.size_body, line, Align::Left, t.text, &what);
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            rest.take_top(28).0,
            Align::Left,
            t.text_dim,
            "Effects above it change the notes it gets; effects below it change what it plays.",
        );
        if let Some(b) = l.open_arp {
            paint_button(cx, t, b, "Open the arp  ›", false, true);
        }
        return;
    }
    let Some((_, _, fx)) = ns.page_fx() else {
        return;
    };
    if let Some(k) = l.chip_k {
        let now = fx.get(k);
        let names = match fx.settings()[k].kind {
            SetKind::Choice(list) => list,
            SetKind::Number { .. } => &[],
        };
        for &(r, v) in &l.chips {
            let name = names.get(v as usize).copied().unwrap_or("");
            paint_button(cx, t, r, name, v == now, true);
        }
        if let Some((prev, next)) = l.chip_arrows {
            let s = fx.settings()[k];
            paint_button(cx, t, prev, "‹", false, now > s.min());
            paint_button(cx, t, next, "›", false, now < s.max());
        }
    }
    if let Some(keys) = l.keys {
        paint_keys(cx, t, keys, ns, &fx);
    }
    for card in &l.cards {
        cx.canvas.fill_round_rect(card.rect, t.radius, t.surface);
        let cap = Rect::new(card.rect.x + 18, card.rect.y + 12, card.rect.w - 36, 30);
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            cap,
            Align::Left,
            t.text_dim,
            &card.title.to_uppercase(),
        );
        for row in &card.rows {
            let s = fx.settings()[row.k];
            let now = fx.get(row.k);
            cx.text(
                FontId::Sans,
                t.size_body - 2.0,
                row.label,
                Align::Left,
                t.text,
                s.name,
            );
            if !row.seg.is_empty() {
                cx.canvas
                    .fill_round_rect(row.value, t.round(10), t.bg.mix(t.surface, 0.4));
                let names = match s.kind {
                    SetKind::Choice(list) => list,
                    SetKind::Number { .. } => &[],
                };
                for &(r, v) in &row.seg {
                    let name = names.get(v as usize).copied().unwrap_or("");
                    if v == now {
                        cx.canvas.fill_round_rect(r, t.round(8), t.picked());
                    }
                    cx.text(
                        if v == now { FontId::Bold } else { FontId::Sans },
                        t.size_small,
                        r,
                        Align::Center,
                        if v == now { t.on_picked() } else { t.text_dim },
                        name,
                    );
                }
                continue;
            }
            if let (Some(m), Some(p)) = (row.minus, row.plus) {
                paint_button(cx, t, m, "–", false, now > s.min());
                paint_button(cx, t, p, "+", false, now < s.max());
            }
            cx.text(
                FontId::Mono,
                t.size_body - 2.0,
                row.value,
                Align::Center,
                t.text,
                &fx.value_text(row.k),
            );
        }
    }
}

/// What goes in and what comes out: two octaves of keys from the C under
/// the lowest note, the note played outlined and the notes out filled.
fn paint_keys(cx: &mut PaintCx, t: &Theme, card: Rect, ns: &NotesState, fx: &Fx) {
    cx.canvas.fill_round_rect(card, t.radius, t.surface);
    let inner = card.inset_xy(18, 10);
    let (cap, rest) = inner.take_top(28);
    cx.text(
        FontId::Bold,
        t.size_small - 1.0,
        cap,
        Align::Left,
        t.text_dim,
        "WHAT GOES IN, WHAT COMES OUT",
    );
    if let Some(row) = ns.selected {
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            cap,
            Align::Right,
            t.text_dim,
            &ns.neighbours(row),
        );
    }
    let (legend, keys) = rest.take_bottom(30);
    let keys = keys.take_bottom(keys.h - 6).0;
    let played = played_note(fx);
    let out = fx.pitches(played);
    let low = out.iter().copied().chain([played]).min().unwrap_or(played);
    let first = low - low % 12;
    let whites: Vec<u8> = (first..first.saturating_add(24))
        .filter(|&n| n <= 127 && !is_black(n))
        .collect();
    let ww = keys.w / whites.len().max(1) as i32;
    // White keys nearly as bright as the text on a dark theme; a light
    // theme's keyboard colours otherwise (the keys the right way round).
    let piano = t.keys();
    let white_ink = if t.is_light() {
        piano.white
    } else {
        t.text.mix(t.surface, 0.06)
    };
    let x_of = |n: u8| -> i32 {
        let i = whites.iter().position(|&w| w > n).unwrap_or(whites.len());
        keys.x + i as i32 * ww
    };
    for (i, &n) in whites.iter().enumerate() {
        let r = Rect::new(keys.x + i as i32 * ww + 1, keys.y, ww - 2, keys.h);
        let fill = if out.contains(&n) && n != played {
            t.accent
        } else {
            white_ink
        };
        cx.canvas.fill_round_rect(r, 4, fill);
        if n == played {
            cx.canvas.stroke_round_rect(r, 4, 3, t.good);
        }
        if n % 12 == 0 || n == played {
            cx.text(
                FontId::Bold,
                t.size_small - 3.0,
                r.take_bottom(24).0,
                Align::Center,
                t.ink_on(fill),
                &note_name(n),
            );
        }
    }
    for n in (first..first.saturating_add(24)).filter(|&n| n <= 127 && is_black(n)) {
        let x = x_of(n) - ww * 3 / 10;
        let r = Rect::new(x, keys.y, ww * 6 / 10, keys.h * 6 / 10);
        let fill = if out.contains(&n) && n != played {
            t.accent
        } else {
            piano.black
        };
        cx.canvas.fill_round_rect(r, 3, fill);
        if n == played {
            cx.canvas.stroke_round_rect(r, 3, 3, t.good);
        }
    }
    // The legend, and the notes as names.
    let sw = |cx: &mut PaintCx, x: i32, c: Color, outline: bool| {
        let r = Rect::new(x, legend.y + 9, 14, 14);
        if outline {
            cx.canvas.stroke_round_rect(r, 3, 2, c);
        } else {
            cx.canvas.fill_round_rect(r, 3, c);
        }
    };
    sw(cx, legend.x, t.good, true);
    cx.text(
        FontId::Sans,
        t.size_small - 2.0,
        Rect::new(legend.x + 20, legend.y, 70, legend.h),
        Align::Left,
        t.text,
        "played",
    );
    sw(cx, legend.x + 96, t.accent, false);
    cx.text(
        FontId::Sans,
        t.size_small - 2.0,
        Rect::new(legend.x + 116, legend.y, 70, legend.h),
        Align::Left,
        t.text,
        "added",
    );
    let names: Vec<String> = out.iter().map(|&n| note_name(n)).collect();
    let flow = if names.is_empty() {
        format!("{} in → nothing out", note_name(played))
    } else {
        format!("{} in → {} out", note_name(played), names.join(" "))
    };
    let x = legend.x + 200;
    cx.text(
        FontId::Mono,
        t.size_small - 2.0,
        Rect::new(x, legend.y, legend.right() - x, legend.h),
        Align::Left,
        t.text_dim,
        &flow,
    );
}

/// Board 7's Notes, for the sample screens and the tests: Scale, Chord
/// (m7, its page shown), the arp, Humanize (off), on track 3.
pub fn example() -> NotesState {
    let mut fx = TrackFx::default();
    let all = Fx::all();
    fx.add(all[0]);
    fx.add(all[1]);
    fx.add(all[6]);
    // Humanize below the arp.
    fx.move_row(2, 3);
    if let Some(Some(s)) = fx.slots.get_mut(1) {
        s.fx.set(0, 5); // m7
        s.fx.set(4, 90); // the added notes at 90 %
    }
    if let Some(Some(s)) = fx.slots.get_mut(2) {
        s.on = false;
    }
    NotesState {
        track: 2,
        fx,
        arp: "Up · 1/16 · 2 oct".to_string(),
        arp_on: true,
        selected: Some(1),
        picking: false,
        plays_on: "Its own sound".to_string(),
        plays_detail: "Change it in MIDI, Tracks out".to_string(),
        ccs_send_nothing: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board7() -> NotesState {
        example()
    }

    #[test]
    fn rows_say_where_they_sit_and_what_they_do() {
        let ns = board7();
        assert_eq!(ns.items().len(), 4);
        assert_eq!(ns.position(1), "Notes 2 of 4");
        assert_eq!(ns.neighbours(1), "after Scale, before Arp");
        assert_eq!(ns.neighbours(0), "first, before Chord");
        assert_eq!(ns.neighbours(3), "after Arp, last");
        let (name, line, on) = ns.row_view(1).unwrap();
        assert_eq!((name.as_str(), on), ("Chord", true));
        assert!(line.contains("m7"), "{line}");
        assert_eq!(ns.row_view(2).unwrap().0, "Arp");
        assert!(!ns.row_view(3).unwrap().2, "Humanize is off");
        let (place, id, fx) = ns.page_fx().unwrap();
        assert_eq!((place, id, fx.name()), (1, 2, "Chord"));
        assert!(!ns.arp_page());
    }

    #[test]
    fn the_q_links_are_the_effects_four_on_its_own_addresses() {
        let ns = board7();
        let (_, id, fx) = ns.page_fx().unwrap();
        let (all, four) = param_views(ns.track, id, &fx);
        let names: Vec<&str> = four.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(names, ["Chord", "Voicing", "Inversion", "Velocity"]);
        assert_eq!(four[0].addr, ParamAddr::new(Owner::NoteFx(2, 2), 0));
        assert_eq!(four[0].text(), "m7");
        assert_eq!(four[3].text(), "90 %");
        // In key's settings show only for an In key chord.
        assert!(all.iter().all(|p| p.info.group != "In key"));
        for p in all.iter().chain(&four) {
            p.info.check().unwrap();
        }
    }

    #[test]
    fn the_list_scrolls_past_four_and_leaves_the_rest_for_the_sound() {
        let mut ns = board7();
        let body = Rect::new(10, 100, 200, 494);
        let cap = Rect::new(10, 72, 200, 28);
        let l = list_layout(cap, body, &ns, 0);
        assert_eq!((l.rows.len(), l.arrows), (4, None));
        assert!(l.add.is_some());
        assert!(l.sound.h >= 2 * (ROW_H + ROW_GAP) + ADD_H, "{:?}", l.sound);
        for fx in Fx::all().into_iter().take(5) {
            ns.fx.add(fx);
        }
        assert_eq!(ns.fx.count(), MAX_FX);
        let l = list_layout(cap, body, &ns, 99);
        assert_eq!(l.rows.len(), 4);
        assert_eq!(l.from, 5, "the last four of nine");
        assert!(l.arrows.is_some() && l.add.is_none());
    }

    #[test]
    fn the_chord_page_has_its_chips_keys_and_two_cards() {
        let ns = board7();
        let page = Rect::new(240, 72, 900, 560);
        let l = page_layout(page, &ns);
        assert_eq!(l.chip_k, Some(0));
        assert!(l.keys.is_some());
        let titles: Vec<&str> = l.cards.iter().map(|c| c.title).collect();
        assert_eq!(titles, ["Voicing", "Added notes"]);
        // The chips that fit hold m7 (with arrows: they don't all fit); a
        // tap on maj7 sets it, the arrow before it sets 7th, m7 nothing.
        let at = |v: i32| *l.chips.iter().find(|c| c.1 == v).unwrap();
        let set = |value| Some(NoteAction::Set { id: 2, k: 0, value });
        let (r, _) = at(6);
        assert_eq!(tap_page(&l, &ns, r.x + 2, r.y + 2), set(6));
        let (r, _) = at(5);
        assert_eq!(tap_page(&l, &ns, r.x + 2, r.y + 2), None);
        let (prev, _) = l.chip_arrows.unwrap();
        assert_eq!(tap_page(&l, &ns, prev.x + 2, prev.y + 2), set(4));
        // Voicing is three buttons; Velocity is - and +.
        let voicing = &l.cards[0].rows[0];
        assert_eq!(voicing.seg.len(), 3);
        let vel = l.cards[1].rows.iter().find(|r| r.k == 4).unwrap();
        let plus = vel.plus.unwrap();
        assert_eq!(
            tap_page(&l, &ns, plus.x + 1, plus.y + 1),
            Some(NoteAction::Set {
                id: 2,
                k: 4,
                value: 91
            })
        );
        // Every place to tap is on the page.
        let mut targets = Vec::new();
        page_targets(&l, &ns, &mut targets);
        for t in &targets {
            assert!(
                t.rect.x >= page.x
                    && t.rect.right() <= page.right()
                    && t.rect.bottom() <= page.bottom(),
                "{} {:?}",
                t.label,
                t.rect
            );
        }
    }

    #[test]
    fn every_effects_page_fits() {
        let page = Rect::new(240, 72, 900, 560);
        for fx in Fx::all() {
            let mut ns = NotesState::default();
            ns.fx.add(fx);
            ns.selected = Some(0);
            let l = page_layout(page, &ns);
            for c in &l.cards {
                for r in &c.rows {
                    assert!(r.value.bottom() <= c.rect.bottom(), "{}", fx.name());
                    assert!(r.value.right() <= c.rect.right(), "{}", fx.name());
                }
            }
        }
        // A chord in key has a third card.
        let mut ns = NotesState::default();
        let mut chord = Fx::all()[1];
        chord.set(0, 12);
        ns.fx.add(chord);
        ns.selected = Some(0);
        let l = page_layout(page, &ns);
        assert_eq!(l.cards.len(), 3);
        for c in &l.cards {
            for r in &c.rows {
                assert!(r.value.bottom() <= c.rect.bottom(), "{}", c.title);
            }
        }
    }

    /// A CC effect's header says what it does while the track's
    /// controllers go out, and that it sends nothing while the track
    /// plays its own sound; other effects say what they do either way.
    #[test]
    fn a_cc_effect_says_in_place_when_it_sends_nothing() {
        let mut ns = board7();
        let row = ns.fx.add(Fx::CcSmooth(mpfree_notefx::CcSmooth::default()));
        ns.selected = row;
        let (title, line) = header_text(&ns).unwrap();
        assert_eq!(title, "CC smooth");
        assert_eq!(line, "Notes 3 of 5 · smooths a MIDI out controller's steps");
        ns.ccs_send_nothing = Some("sends nothing: the track plays its own sound".to_string());
        assert_eq!(
            header_text(&ns).unwrap().1,
            "Notes 3 of 5 · sends nothing: the track plays its own sound"
        );
        ns.selected = Some(1);
        assert_eq!(
            header_text(&ns).unwrap().1,
            "Notes 2 of 5 · adds notes to each one played"
        );
        // Its glide reads in ms on a Q-Link card.
        let smooth = Fx::CcSmooth(mpfree_notefx::CcSmooth::default());
        assert_eq!(unit_of(&smooth, 1), Unit::Ms);
        assert_eq!(smooth.value_text(0), "All");
        assert_eq!(smooth.value_text(2), "Jumps");
    }

    #[test]
    fn the_picker_offers_every_effect_and_the_arp_page_its_panel() {
        let mut ns = board7();
        ns.picking = true;
        let page = Rect::new(240, 72, 900, 560);
        let l = page_layout(page, &ns);
        assert_eq!(l.tiles.len(), Fx::all().len());
        for t in &l.tiles {
            assert!(t.bottom() <= page.bottom() && t.right() <= page.right());
        }
        let r = l.tiles[3];
        assert_eq!(
            tap_page(&l, &ns, r.x + 1, r.y + 1),
            Some(NoteAction::Add(3))
        );
        ns.picking = false;
        ns.selected = Some(2);
        assert!(ns.arp_page());
        let l = page_layout(page, &ns);
        let b = l.open_arp.unwrap();
        assert_eq!(
            tap_page(&l, &ns, b.x + 1, b.y + 1),
            Some(NoteAction::OpenArp)
        );
    }

    #[test]
    fn note_names_count_middle_c_as_c4() {
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(48), "C3");
        assert_eq!(note_name(51), "Eb3");
        assert_eq!(note_name(0), "C-1");
    }
}
