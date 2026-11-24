//! The list of synths over the Synth tab's page (Jay's device notes, 20
//! Nov 2026; the "Synth page: pick and play" mockups): a tap on the
//! instrument card opens it. A heading ("Pick a synth for the pads") with
//! Cancel and Keep, then a tile for each choice (the selected track's
//! instrument, so the pads follow the track again, then the built-in
//! synths, then the plugin synths), each with its name and what it is good
//! for. No track changes (the instrument redesign's step 6). The one on
//! the pads now is outlined and tagged "ON THE PADS".
//!
//! On a kit's pages a row under the heading offers **All banks** and
//! **Bank B only** (the bank the pads are on, step 6c): where a tap puts
//! its synth.
//!
//! **Layer** (step 11b), a switch in the heading before Cancel: with it
//! on, a tap adds a synth to what the pads play or takes it off, up to
//! four; each one playing is outlined and tagged.
//!
//! **Synths and Kits** (sampler step 15e; the canvas "Pads and kits: step
//! 15e", boards 1 and 2, Jay's "Build as drawn" of 23 Nov 16:11Z): with
//! kits to offer, a switch at the start of the row under the heading picks
//! the side. The Kits side has a tile for each kit (its number and name,
//! how many pads, the banks they fill, keygroups, and the track that plays
//! it), the empty kits last ("9 · Empty"), after a first tile that gives
//! the pads back ("Track 1's kit", or "Follow base" for one bank). With
//! "Bank B only", a kit with more than one bank filled gets a row for the
//! bank of it bank B plays; a kit that holds keygroups gets its Keys play
//! there (Pads, Keygroups or Both).
//!
//! Events: `Select(i)` a tile (choice `i`), `Tab(CANCEL)`, `Tab(KEEP)`,
//! `Tab(ALL_BANKS)`, `Tab(BANK_ONLY)`, `Tab(LAYER)`, `Tab(SYNTHS)` and
//! `Tab(KITS)`; on the Kits side `Cell { row: KIT_TILES, col }` a kit
//! tile, `Cell { row: FROM_BANK, col }` a bank of the source bank row
//! and `Cell { row: KEYS, col }` a Keys play choice.

use mpfree_ui::text::FontId;
use mpfree_ui::{Align, Color, EventKind, PaintCx, Rect, Target, Touch, Widget};

/// The widget's name in the app's page.
pub const PICK: &str = "synth-pick";
/// `Tab(CANCEL)`: the pads play what they played when the list opened.
pub const CANCEL: usize = 0;
/// `Tab(KEEP)`: the pads keep what they play now.
pub const KEEP: usize = 1;
/// `Tab(ALL_BANKS)`: a tap puts its synth on every bank (step 6c).
pub const ALL_BANKS: usize = 2;
/// `Tab(BANK_ONLY)`: a tap puts its synth on the bank the pads are on.
pub const BANK_ONLY: usize = 3;
/// `Tab(LAYER)`: the Layer switch (step 11b).
pub const LAYER: usize = 4;
/// `Tab(SYNTHS)` and `Tab(KITS)`: the side the list shows (sampler step
/// 15e).
pub const SYNTHS: usize = 5;
pub const KITS: usize = 6;
/// `Cell { row: KIT_TILES, col }`: the Kits side's tile `col`.
pub const KIT_TILES: usize = 0;
/// `Cell { row: FROM_BANK, col }`: the outlined kit's bank
/// `FromRow::banks[col]`, for the bank the pads are on.
pub const FROM_BANK: usize = 1;
/// `Cell { row: KEYS, col }`: the outlined kit's Keys play, 0 Pads, 1
/// Keygroups, 2 Both.
pub const KEYS: usize = 2;
/// The Keys play choices, in order.
pub const KEYS_PLAY: [&str; 3] = ["Pads", "Keygroups", "Both"];

/// The heading's height.
const HEAD_H: i32 = 64;
/// The All banks / Bank B only row's height, when it shows.
const SCOPE_H: i32 = 48;
/// Tiles a row, and a tile's height.
const COLS: i32 = 4;
const TILE_H: i32 = 104;
/// A kit's tile is lower: there are 17 of them.
const KIT_TILE_H: i32 = 84;
const GAP: i32 = 10;
/// The Kits side's row for the outlined kit (its bank, its Keys play).
const KIT_ROW_H: i32 = 46;

/// A tile of the Kits side (sampler step 15e).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KitTile {
    /// "7 · Big Kit", "Track 1's kit", "9 · Empty".
    pub name: String,
    /// The name Keep says ("Big Kit").
    pub short: String,
    /// "48 pads in A to C, keygroups · Track 5", "Start a kit here".
    pub sub: String,
    /// The kit's colour; None for the first tile and an empty kit.
    pub colour: Option<Color>,
    /// An empty kit: drawn faint, outlined thin.
    pub empty: bool,
}

/// "Bank B plays Big Kit's bank A B C" (sampler step 15e, board 2).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FromRow {
    /// The bank the pads are on.
    pub bank: char,
    /// The outlined kit's name.
    pub kit: String,
    /// The kit's banks that hold sounds.
    pub banks: Vec<char>,
    /// The one bank `bank` plays, of `banks`.
    pub on: usize,
}

/// The Kits side, while the list offers kits (sampler step 15e).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KitsSide {
    /// The list shows this side, not the synths.
    pub shown: bool,
    /// The first tile (Follow track, or Follow base), then each kit.
    pub tiles: Vec<KitTile>,
    /// The tile on the pads (the bank's, with Bank B only) now; None: a
    /// synth plays there.
    pub now: Option<usize>,
    /// The tile on the pads when the list opened (Keep names a new one).
    pub had: Option<usize>,
    /// The kits the pads play as layers, by tile, each outlined.
    pub layers: Vec<usize>,
    /// The outlined kit's source bank, with Bank B only and more than one
    /// bank of it filled.
    pub from: Option<FromRow>,
    /// The outlined kit's name and Keys play (of [`KEYS_PLAY`]), for one
    /// that holds keygroups.
    pub keys: Option<(String, usize)>,
}

impl KitsSide {
    fn offered(&self) -> bool {
        !self.tiles.is_empty()
    }

    fn row(&self) -> bool {
        self.shown && (self.from.is_some() || self.keys.is_some())
    }

    fn playing(&self, i: usize) -> bool {
        Some(i) == self.now || self.layers.contains(&i)
    }
}

/// What each Keys play choice does, for the hint beside them.
fn keys_hint(i: usize) -> &'static str {
    match i {
        0 => "Each key plays its pad",
        1 => "Each key plays the keygroups that hold it",
        _ => "Keygroups where one holds the note, the pad elsewhere",
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SynthList {
    /// "Pick a synth for the pads".
    pub title: String,
    /// Each choice's name and what it is good for.
    pub choices: Vec<(String, String)>,
    /// The choice on the pads now (None: one the list hasn't got).
    pub now: Option<usize>,
    /// The choice the pads had when the list opened (Keep names the one
    /// on the pads; with nothing changed it reads "Done").
    pub had: Option<usize>,
    /// On a kit's pages (step 6c): the bank the pads are on ('B') and
    /// whether a tap gives it alone the synth. None: no row.
    pub bank: Option<(char, bool)>,
    /// Layer is on (step 11b): a tap adds a synth or takes it off.
    pub layer: bool,
    /// The choices the pads play as layers beyond `now`, each outlined.
    pub layers: Vec<usize>,
    /// The Kits side (sampler step 15e); no tiles: no Synths and Kits
    /// switch.
    pub kits: KitsSide,
}

impl SynthList {
    fn head(r: Rect) -> Rect {
        Rect::new(r.x + 16, r.y, r.w - 32, HEAD_H)
    }

    /// Cancel and Keep, at the heading's right.
    pub fn buttons(&self, r: Rect) -> [Rect; 2] {
        let h = Self::head(r);
        let keep = Rect::new(h.right() - 150, h.y + 10, 150, h.h - 20);
        let cancel = Rect::new(keep.x - 12 - 120, keep.y, 120, keep.h);
        [cancel, keep]
    }

    /// The Layer switch, before Cancel (step 11b).
    pub fn layer_button(&self, r: Rect) -> Rect {
        let [cancel, _] = self.buttons(r);
        Rect::new(cancel.x - 12 - 110, cancel.y, 110, cancel.h)
    }

    /// Whether choice `i` plays on the pads: the first or a layer.
    fn playing(&self, i: usize) -> bool {
        Some(i) == self.now || self.layers.contains(&i)
    }

    /// Whether the row under the heading shows: the Synths and Kits
    /// switch, and All banks and Bank B only.
    fn side_row(&self) -> bool {
        self.bank.is_some() || self.kits.offered()
    }

    /// Synths and Kits, at the start of the row under the heading (sampler
    /// step 15e), while there are kits to offer.
    pub fn sides(&self, r: Rect) -> Option<[Rect; 2]> {
        if !self.kits.offered() {
            return None;
        }
        let y = r.y + HEAD_H;
        let synths = Rect::new(r.x + 16, y, 110, SCOPE_H - 12);
        let kits = Rect::new(synths.right() + 6, y, 110, SCOPE_H - 12);
        Some([synths, kits])
    }

    /// All banks and Bank B only, under the heading (step 6c), while the
    /// row shows: at its start, or at its end after Synths and Kits.
    pub fn scopes(&self, r: Rect) -> Option<[Rect; 2]> {
        self.bank?;
        let y = r.y + HEAD_H;
        if self.kits.offered() {
            let one = Rect::new(r.right() - 16 - 170, y, 170, SCOPE_H - 12);
            let all = Rect::new(one.x - 8 - 150, y, 150, SCOPE_H - 12);
            return Some([all, one]);
        }
        let all = Rect::new(r.x + 16, y, 150, SCOPE_H - 12);
        let one = Rect::new(all.right() + 8, y, 170, SCOPE_H - 12);
        Some([all, one])
    }

    /// The Kits side's row for the outlined kit: its words' place, then
    /// the source bank's letters (with Bank B only), then "Keys play" and
    /// its three choices, and the hint's place after them.
    fn kit_row(&self, r: Rect) -> Option<KitRow> {
        if !self.kits.row() {
            return None;
        }
        let y = r.y + HEAD_H + SCOPE_H;
        let h = KIT_ROW_H - 12;
        let mut x = r.x + 16;
        let mut row = KitRow::default();
        if let Some(f) = &self.kits.from {
            row.from_words = Rect::new(x, y, 250, h);
            x += 250 + 8;
            for _ in &f.banks {
                row.letters.push(Rect::new(x, y, 44, h));
                x += 44 + 6;
            }
            x += 18;
        }
        if self.kits.keys.is_some() {
            let words = if self.kits.from.is_some() { 86 } else { 220 };
            row.keys_words = Rect::new(x, y, words, h);
            x += words + 8;
            for (i, w) in [76, 112, 70].into_iter().enumerate() {
                row.keys[i] = Rect::new(x, y, w, h);
                x += w + 6;
            }
            x += 10;
        }
        row.hint = Rect::new(x, y, (r.right() - 16 - x).max(0), h);
        Some(row)
    }

    /// The tiles' top: under the heading, the row under it, and the Kits
    /// side's row for its kit.
    fn body_top(&self, r: Rect) -> i32 {
        r.y + HEAD_H
            + if self.side_row() { SCOPE_H } else { 0 }
            + if self.kits.row() { KIT_ROW_H } else { 0 }
    }

    /// Tile `i` of `n`, four a row under the heading, `tall` at most.
    fn grid(&self, r: Rect, i: usize, n: usize, tall: i32) -> Rect {
        let top = self.body_top(r);
        let body = Rect::new(r.x + 16, top, r.w - 32, r.bottom() - top - 16);
        let w = (body.w - GAP * (COLS - 1)) / COLS;
        // The rows shrink when there are many.
        let rows = (n as i32 + COLS - 1) / COLS;
        let h = tall.min((body.h - GAP * (rows - 1).max(0)) / rows.max(1));
        let (col, row) = (i as i32 % COLS, i as i32 / COLS);
        Rect::new(body.x + col * (w + GAP), body.y + row * (h + GAP), w, h)
    }

    /// Tile `i`: four a row under the heading.
    pub fn tile(&self, r: Rect, i: usize) -> Rect {
        self.grid(r, i, self.choices.len(), TILE_H)
    }

    /// The Kits side's tile `i`.
    pub fn kit_tile(&self, r: Rect, i: usize) -> Rect {
        self.grid(r, i, self.kits.tiles.len(), KIT_TILE_H)
    }

    /// The Kits side's tiles: each kit's colour, name, and what it holds;
    /// an empty kit faint; the ones on the pads outlined and tagged.
    fn paint_kits(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        for (i, k) in self.kits.tiles.iter().enumerate() {
            let b = self.kit_tile(r, i);
            let on = self.kits.playing(i);
            if k.empty && !on {
                cx.canvas.stroke_round_rect(b, 12, 1, t.text_faint);
            } else {
                cx.canvas
                    .fill_round_rect(b, 12, if on { t.raised } else { t.bg });
                if on {
                    cx.canvas.stroke_round_rect(b, 12, 2, t.accent);
                } else {
                    cx.canvas.stroke_round_rect(b, 12, 1, t.outline);
                }
            }
            let inner = b.inset_xy(14, 8);
            let line = (inner.h / 3).max(16);
            let sw = 14;
            let name_x = match k.colour {
                Some(c) => {
                    cx.canvas.fill_round_rect(
                        Rect::new(inner.x, inner.y + (line - sw) / 2, sw, sw),
                        4,
                        t.content(c),
                    );
                    inner.x + sw + 10
                }
                None if i == 0 => {
                    cx.canvas.fill_round_rect(
                        Rect::new(inner.x, inner.y + (line - sw) / 2, sw, sw),
                        4,
                        t.text_faint,
                    );
                    inner.x + sw + 10
                }
                None => inner.x,
            };
            cx.text(
                FontId::Bold,
                t.size_body - 1.0,
                Rect::new(name_x, inner.y, inner.right() - name_x, line),
                Align::Left,
                if k.empty { t.text_dim } else { t.text },
                &k.name,
            );
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                Rect::new(inner.x, inner.y + line, inner.w, line),
                Align::Left,
                t.text_dim,
                &k.sub,
            );
            if on {
                cx.text(
                    FontId::Bold,
                    t.size_small - 2.0,
                    Rect::new(inner.x, inner.y + 2 * line, inner.w, inner.h - 2 * line),
                    Align::Left,
                    t.accent,
                    "ON THE PADS",
                );
            }
        }
    }

    fn keep_label(&self) -> String {
        if !self.layers.is_empty() || !self.kits.layers.is_empty() {
            return "Keep layers".to_string();
        }
        if let Some(t) = self.kits.now.and_then(|n| self.kits.tiles.get(n)) {
            if self.kits.now != self.kits.had {
                return format!("Keep {}", t.short);
            }
        }
        match self.now.and_then(|n| self.choices.get(n)) {
            Some((name, _)) if self.now != self.had => format!("Keep {name}"),
            _ => "Done".to_string(),
        }
    }
}

/// The places of the Kits side's row ([`SynthList::kit_row`]).
#[derive(Default)]
struct KitRow {
    from_words: Rect,
    letters: Vec<Rect>,
    keys_words: Rect,
    keys: [Rect; 3],
    hint: Rect,
}

/// A segment of a switch: picked in the picked colour, else raised and
/// outlined.
fn segment(cx: &mut PaintCx, rect: Rect, on: bool, label: &str) {
    let t = cx.theme.clone();
    if on {
        cx.canvas.fill_round_rect(rect, 8, t.picked());
    } else {
        cx.canvas.fill_round_rect(rect, 8, t.raised);
        cx.canvas.stroke_round_rect(rect, 8, 1, t.outline);
    }
    cx.text(
        FontId::Bold,
        t.size_small,
        rect,
        Align::Center,
        if on { t.on_picked() } else { t.text },
        label,
    );
}

impl Widget for SynthList {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.panel(r);
        let h = Self::head(r);
        let [cancel, keep] = self.buttons(r);
        let layer = self.layer_button(r);
        let words = Rect::new(h.x, h.y + 10, layer.x - h.x - 12, h.h - 20);
        let (top, bottom) = words.take_top(words.h * 3 / 5);
        cx.text(
            FontId::Bold,
            t.size_title + 1.0,
            top,
            Align::Left,
            t.text,
            &self.title,
        );
        cx.text(
            FontId::Sans,
            t.size_small,
            bottom,
            Align::Left,
            t.text_dim,
            &match self.bank {
                _ if self.layer => "A tap adds it to the pads or takes it off, up to 4".to_string(),
                Some((b, true)) => {
                    format!("A tap puts it on bank {b} at once: play its pads to hear it")
                }
                _ => "A tap puts it on the pads at once: play them to hear it".to_string(),
            },
        );
        // Layer: on in the accent, off raised and outlined.
        if self.layer {
            cx.canvas.fill_round_rect(layer, 10, t.accent);
        } else {
            cx.canvas.fill_round_rect(layer, 10, t.raised);
            cx.canvas.stroke_round_rect(layer, 10, 1, t.outline);
        }
        cx.text(
            FontId::Bold,
            t.size_body - 1.0,
            layer,
            Align::Center,
            if self.layer { t.on_accent } else { t.text },
            "Layer",
        );
        if let (Some([all, one]), Some((b, only))) = (self.scopes(r), self.bank) {
            segment(cx, all, !only, "All banks");
            segment(cx, one, only, &format!("Bank {b} only"));
        }
        if let Some([synths, kits]) = self.sides(r) {
            segment(cx, synths, !self.kits.shown, "Synths");
            segment(cx, kits, self.kits.shown, "Kits");
        }
        if let Some(row) = self.kit_row(r) {
            if let Some(f) = &self.kits.from {
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    row.from_words,
                    Align::Left,
                    t.text_dim,
                    &format!("Bank {} plays {}'s bank", f.bank, f.kit),
                );
                for (i, (rect, letter)) in row.letters.iter().zip(&f.banks).enumerate() {
                    segment(cx, *rect, i == f.on, &letter.to_string());
                }
            }
            if let Some((kit, on)) = &self.kits.keys {
                let words = if self.kits.from.is_some() {
                    "Keys play".to_string()
                } else {
                    format!("{kit}'s keys play")
                };
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    row.keys_words,
                    Align::Left,
                    t.text_dim,
                    &words,
                );
                for (i, label) in KEYS_PLAY.iter().enumerate() {
                    segment(cx, row.keys[i], i == *on, label);
                }
                if self.kits.from.is_none() {
                    cx.text(
                        FontId::Sans,
                        t.size_small - 1.0,
                        row.hint,
                        Align::Left,
                        t.text_dim,
                        keys_hint(*on),
                    );
                }
            }
        }
        cx.canvas.fill_round_rect(cancel, 10, t.raised);
        cx.canvas.stroke_round_rect(cancel, 10, 1, t.outline);
        cx.text(
            FontId::Sans,
            t.size_body - 1.0,
            cancel,
            Align::Center,
            t.text,
            "Cancel",
        );
        cx.canvas.fill_round_rect(keep, 10, t.accent);
        cx.text(
            FontId::Bold,
            t.size_body - 1.0,
            keep,
            Align::Center,
            t.on_accent,
            &self.keep_label(),
        );
        if self.kits.shown {
            self.paint_kits(cx, r);
            return;
        }
        for (i, (name, good)) in self.choices.iter().enumerate() {
            let b = self.tile(r, i);
            let on = self.playing(i);
            cx.canvas
                .fill_round_rect(b, 12, if on { t.raised } else { t.bg });
            if on {
                cx.canvas.stroke_round_rect(b, 12, 2, t.accent);
            } else {
                cx.canvas.stroke_round_rect(b, 12, 1, t.outline);
            }
            let inner = b.inset_xy(14, 10);
            let line = (inner.h / 3).max(18);
            let sw = 16;
            let swatch = if i == 0 {
                t.text_faint
            } else {
                t.content(t.track[(i - 1) % t.track.len()])
            };
            cx.canvas.fill_round_rect(
                Rect::new(inner.x, inner.y + (line - sw) / 2, sw, sw),
                4,
                swatch,
            );
            cx.text(
                FontId::Bold,
                t.size_body,
                Rect::new(inner.x + sw + 10, inner.y, inner.w - sw - 10, line),
                Align::Left,
                t.text,
                name,
            );
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                Rect::new(inner.x, inner.y + line, inner.w, line),
                Align::Left,
                t.text_dim,
                good,
            );
            if on {
                cx.text(
                    FontId::Bold,
                    t.size_small - 2.0,
                    Rect::new(inner.x, inner.y + 2 * line, inner.w, inner.h - 2 * line),
                    Align::Left,
                    t.accent,
                    "ON THE PADS",
                );
            }
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let [cancel, keep] = self.buttons(r);
        let mut out = vec![
            Target::tap("cancel", CANCEL, "Cancel", cancel),
            Target::tap("keep", KEEP, self.keep_label(), keep),
            Target::tap("layer", LAYER, "Layer", self.layer_button(r)),
        ];
        if let (Some([all, one]), Some((b, _))) = (self.scopes(r), self.bank) {
            out.push(Target::tap("all-banks", ALL_BANKS, "All banks", all));
            out.push(Target::tap(
                "bank-only",
                BANK_ONLY,
                format!("Bank {b} only"),
                one,
            ));
        }
        if let Some([synths, kits]) = self.sides(r) {
            out.push(Target::tap("side", SYNTHS, "Synths", synths));
            out.push(Target::tap("side", KITS, "Kits", kits));
        }
        if self.kits.shown {
            if let Some(row) = self.kit_row(r) {
                if let Some(f) = &self.kits.from {
                    for (i, (rect, l)) in row.letters.iter().zip(&f.banks).enumerate() {
                        out.push(Target::tap("from-bank", i, format!("Bank {l}"), *rect));
                    }
                }
                if self.kits.keys.is_some() {
                    for (i, label) in KEYS_PLAY.iter().enumerate() {
                        out.push(Target::tap("keys-play", i, *label, row.keys[i]));
                    }
                }
            }
            out.extend(
                self.kits
                    .tiles
                    .iter()
                    .enumerate()
                    .map(|(i, k)| Target::tap("kit", i, k.name.clone(), self.kit_tile(r, i))),
            );
            return out;
        }
        out.extend(
            self.choices
                .iter()
                .enumerate()
                .map(|(i, (name, _))| Target::tap("synth", i, name.clone(), self.tile(r, i))),
        );
        out
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        if let Some(k) = self.buttons(r).iter().position(|b| b.contains(x, y)) {
            return Some(EventKind::Tab(k));
        }
        if self.layer_button(r).contains(x, y) {
            return Some(EventKind::Tab(LAYER));
        }
        if let Some(k) = self
            .scopes(r)
            .and_then(|s| s.iter().position(|b| b.contains(x, y)))
        {
            return Some(EventKind::Tab(ALL_BANKS + k));
        }
        if let Some(k) = self
            .sides(r)
            .and_then(|s| s.iter().position(|b| b.contains(x, y)))
        {
            return Some(EventKind::Tab(SYNTHS + k));
        }
        if self.kits.shown {
            if let Some(row) = self.kit_row(r) {
                if let Some(col) = row.letters.iter().position(|b| b.contains(x, y)) {
                    return Some(EventKind::Cell {
                        row: FROM_BANK,
                        col,
                    });
                }
                if self.kits.keys.is_some() {
                    if let Some(col) = row.keys.iter().position(|b| b.contains(x, y)) {
                        return Some(EventKind::Cell { row: KEYS, col });
                    }
                }
            }
            return (0..self.kits.tiles.len())
                .find(|&i| self.kit_tile(r, i).contains(x, y))
                .map(|col| EventKind::Cell {
                    row: KIT_TILES,
                    col,
                });
        }
        (0..self.choices.len())
            .find(|&i| self.tile(r, i).contains(x, y))
            .map(EventKind::Select)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(n: usize) -> SynthList {
        SynthList {
            title: "Pick a synth for Track 1".into(),
            choices: (0..n).map(|i| (format!("S{i}"), String::new())).collect(),
            now: Some(2),
            had: Some(0),
            bank: None,
            layer: false,
            layers: Vec::new(),
            kits: KitsSide::default(),
        }
    }

    #[test]
    fn a_tap_on_a_tile_picks_it_and_the_buttons_cancel_or_keep() {
        let mut l = list(13);
        let r = Rect::new(244, 84, 884, 666);
        for i in 0..13 {
            let b = l.tile(r, i);
            assert!(
                b.right() <= r.right() && b.bottom() <= r.bottom(),
                "tile {i} {b:?}"
            );
            let (x, y) = (b.x + b.w / 2, b.y + b.h / 2);
            assert_eq!(l.touch(0, Touch::Down, x, y, r), Some(EventKind::Select(i)));
        }
        let [c, k] = l.buttons(r);
        assert_eq!(
            l.touch(0, Touch::Down, c.x + 5, c.y + 5, r),
            Some(EventKind::Tab(CANCEL))
        );
        assert_eq!(
            l.touch(0, Touch::Down, k.x + 5, k.y + 5, r),
            Some(EventKind::Tab(KEEP))
        );
        assert_eq!(l.keep_label(), "Keep S2");
        l.now = Some(0);
        assert_eq!(l.keep_label(), "Done", "nothing changed");
    }

    #[test]
    fn many_plugin_synths_still_fit() {
        let mut l = list(40);
        let r = Rect::new(244, 84, 884, 666);
        let last = l.tile(r, 39);
        assert!(last.bottom() <= r.bottom(), "{last:?}");
        l.bank = Some(('B', false));
        let last = l.tile(r, 39);
        assert!(last.bottom() <= r.bottom(), "with the banks' row: {last:?}");
    }

    /// Step 6c: on a kit's pages the row under the heading picks where a
    /// tap puts its synth, every bank or the one the pads are on; the
    /// tiles move down for it and still take their taps.
    #[test]
    fn all_banks_or_bank_b_only_sit_under_the_heading() {
        let mut l = list(13);
        let r = Rect::new(244, 84, 884, 666);
        assert!(l.scopes(r).is_none(), "no row on the synth's layouts");
        let top = l.tile(r, 0).y;
        l.bank = Some(('B', true));
        let [all, one] = l.scopes(r).expect("the row");
        assert!(l.tile(r, 0).y > all.bottom() && l.tile(r, 0).y > top);
        assert_eq!(
            l.touch(0, Touch::Down, all.x + 5, all.y + 5, r),
            Some(EventKind::Tab(ALL_BANKS))
        );
        assert_eq!(
            l.touch(0, Touch::Down, one.x + 5, one.y + 5, r),
            Some(EventKind::Tab(BANK_ONLY))
        );
        let b = l.tile(r, 12);
        assert!(b.bottom() <= r.bottom(), "{b:?}");
        assert_eq!(
            l.touch(0, Touch::Down, b.x + b.w / 2, b.y + b.h / 2, r),
            Some(EventKind::Select(12))
        );
        let names: Vec<String> = l.targets(r).into_iter().map(|t| t.label).collect();
        assert!(names.contains(&"Bank B only".to_string()), "{names:?}");
    }

    /// Step 11b: the Layer switch sits in the heading before Cancel, clear
    /// of the title's words, and takes its tap; the layers are outlined
    /// with the first, and Keep keeps them.
    #[test]
    fn the_layer_switch_sits_before_cancel_and_layers_are_outlined() {
        let mut l = list(13);
        let r = Rect::new(244, 84, 884, 666);
        let [cancel, _] = l.buttons(r);
        let layer = l.layer_button(r);
        assert!(layer.right() < cancel.x && layer.x > r.x + 300, "{layer:?}");
        assert_eq!(
            l.touch(0, Touch::Down, layer.x + 5, layer.y + 5, r),
            Some(EventKind::Tab(LAYER))
        );
        l.layer = true;
        l.layers = vec![5, 7];
        assert!(l.playing(2) && l.playing(5) && l.playing(7) && !l.playing(3));
        assert_eq!(l.keep_label(), "Keep layers");
        let names: Vec<String> = l.targets(r).into_iter().map(|t| t.label).collect();
        assert!(names.contains(&"Layer".to_string()), "{names:?}");
    }

    fn kits(n: usize) -> KitsSide {
        KitsSide {
            shown: true,
            tiles: (0..n)
                .map(|i| KitTile {
                    name: format!("{i} · Kit"),
                    short: format!("Kit {i}"),
                    sub: "14 pads · no track".into(),
                    colour: (i > 0 && i < 9).then_some(Color::rgb(0xC86432)),
                    empty: i >= 9,
                })
                .collect(),
            now: Some(7),
            had: Some(0),
            ..Default::default()
        }
    }

    /// Sampler step 15e, boards 1 and 2: with kits to offer, Synths and
    /// Kits start the row under the heading and the bank switch moves to
    /// its end; the Kits side's 17 tiles fit and each takes its tap as a
    /// kit's; the outlined kit's row (its bank, its Keys play) pushes them
    /// down and takes its own taps; Keep names the kit put on the pads.
    #[test]
    fn the_kits_side_lists_the_kits_with_the_outlined_kits_row() {
        let mut l = list(13);
        l.kits = kits(17);
        let r = Rect::new(244, 84, 884, 666);
        let [synths, kits_tab] = l.sides(r).expect("the switch");
        assert_eq!(
            l.touch(0, Touch::Down, synths.x + 5, synths.y + 5, r),
            Some(EventKind::Tab(SYNTHS))
        );
        assert_eq!(
            l.touch(0, Touch::Down, kits_tab.x + 5, kits_tab.y + 5, r),
            Some(EventKind::Tab(KITS))
        );
        l.bank = Some(('B', true));
        let [all, one] = l.scopes(r).unwrap();
        assert!(all.x > kits_tab.right() && one.right() <= r.right() - 16);
        let top = l.kit_tile(r, 0).y;
        for i in 0..17 {
            let b = l.kit_tile(r, i);
            assert!(b.bottom() <= r.bottom(), "tile {i} {b:?}");
            assert_eq!(
                l.touch(0, Touch::Down, b.x + b.w / 2, b.y + b.h / 2, r),
                Some(EventKind::Cell {
                    row: KIT_TILES,
                    col: i
                })
            );
        }
        assert_eq!(l.keep_label(), "Keep Kit 7");
        // Bank B only on a kit of three banks that holds keygroups.
        l.kits.from = Some(FromRow {
            bank: 'B',
            kit: "Big Kit".into(),
            banks: vec!['A', 'B', 'C'],
            on: 2,
        });
        l.kits.keys = Some(("Big Kit".into(), 2));
        assert!(l.kit_tile(r, 0).y > top, "the row pushes the tiles down");
        let row = l.kit_row(r).unwrap();
        assert_eq!(row.letters.len(), 3);
        assert!(row.keys[2].right() <= r.right() - 16, "{:?}", row.keys[2]);
        let c = row.letters[2];
        assert_eq!(
            l.touch(0, Touch::Down, c.x + 5, c.y + 5, r),
            Some(EventKind::Cell {
                row: FROM_BANK,
                col: 2
            })
        );
        let k = row.keys[1];
        assert_eq!(
            l.touch(0, Touch::Down, k.x + 5, k.y + 5, r),
            Some(EventKind::Cell { row: KEYS, col: 1 })
        );
        let last = l.kit_tile(r, 16);
        assert!(last.bottom() <= r.bottom(), "{last:?}");
        let names: Vec<String> = l.targets(r).into_iter().map(|t| t.label).collect();
        for n in ["Kits", "Bank C", "Keygroups", "16 · Kit"] {
            assert!(names.contains(&n.to_string()), "{n}: {names:?}");
        }
        // The Synths side: the synths' tiles again, by Select.
        l.kits.shown = false;
        let b = l.tile(r, 3);
        assert_eq!(
            l.touch(0, Touch::Down, b.x + 5, b.y + 5, r),
            Some(EventKind::Select(3))
        );
        assert!(l.kit_row(r).is_none(), "the kit's row is the Kits side's");
    }
}
