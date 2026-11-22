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
//! Events: `Select(i)` a tile (choice `i`), `Tab(CANCEL)`, `Tab(KEEP)`,
//! `Tab(ALL_BANKS)` and `Tab(BANK_ONLY)`.

use mpfree_ui::text::FontId;
use mpfree_ui::{Align, EventKind, PaintCx, Rect, Target, Touch, Widget};

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

/// The heading's height.
const HEAD_H: i32 = 64;
/// The All banks / Bank B only row's height, when it shows.
const SCOPE_H: i32 = 48;
/// Tiles a row, and a tile's height.
const COLS: i32 = 4;
const TILE_H: i32 = 104;
const GAP: i32 = 10;

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

    /// All banks and Bank B only, under the heading (step 6c), while the
    /// row shows.
    pub fn scopes(&self, r: Rect) -> Option<[Rect; 2]> {
        self.bank?;
        let y = r.y + HEAD_H;
        let all = Rect::new(r.x + 16, y, 150, SCOPE_H - 12);
        let one = Rect::new(all.right() + 8, y, 170, SCOPE_H - 12);
        Some([all, one])
    }

    /// The tiles' top: under the heading, and the banks' row.
    fn body_top(&self, r: Rect) -> i32 {
        r.y + HEAD_H + if self.bank.is_some() { SCOPE_H } else { 0 }
    }

    /// Tile `i`: four a row under the heading.
    pub fn tile(&self, r: Rect, i: usize) -> Rect {
        let top = self.body_top(r);
        let body = Rect::new(r.x + 16, top, r.w - 32, r.bottom() - top - 16);
        let w = (body.w - GAP * (COLS - 1)) / COLS;
        // The rows shrink when there are many plugin synths.
        let rows = (self.choices.len() as i32 + COLS - 1) / COLS;
        let h = TILE_H.min((body.h - GAP * (rows - 1).max(0)) / rows.max(1));
        let (col, row) = (i as i32 % COLS, i as i32 / COLS);
        Rect::new(body.x + col * (w + GAP), body.y + row * (h + GAP), w, h)
    }

    fn keep_label(&self) -> String {
        match self.now.and_then(|n| self.choices.get(n)) {
            Some((name, _)) if self.now != self.had => format!("Keep {name}"),
            _ => "Done".to_string(),
        }
    }
}

impl Widget for SynthList {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.panel(r);
        let h = Self::head(r);
        let [cancel, keep] = self.buttons(r);
        let words = Rect::new(h.x, h.y + 10, cancel.x - h.x - 12, h.h - 20);
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
                Some((b, true)) => {
                    format!("A tap puts it on bank {b} at once: play its pads to hear it")
                }
                _ => "A tap puts it on the pads at once: play them to hear it".to_string(),
            },
        );
        if let (Some([all, one]), Some((b, only))) = (self.scopes(r), self.bank) {
            for (rect, on, label) in [
                (all, !only, "All banks".to_string()),
                (one, only, format!("Bank {b} only")),
            ] {
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
                    &label,
                );
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
        for (i, (name, good)) in self.choices.iter().enumerate() {
            let b = self.tile(r, i);
            let on = Some(i) == self.now;
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
        if let Some(k) = self
            .scopes(r)
            .and_then(|s| s.iter().position(|b| b.contains(x, y)))
        {
            return Some(EventKind::Tab(ALL_BANKS + k));
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
}
