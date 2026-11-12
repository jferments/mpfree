//! Auto-sample's picture: the keyboard with the notes it samples and the
//! keys each one will play (its zone), the velocity layers on a 1 to 127
//! axis, and, while it runs, the note playing and how far it has got.

use mpfree_analysis::note_name;
use mpfree_sampler::model::Keys;
use mpfree_ui::{Align, Color, FontId, PaintCx, Rect, Widget};

use crate::rec_widgets::REC_COLOR;

/// One velocity layer: the velocities it answers and the one it is
/// played at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerBand {
    pub lo: u8,
    pub hi: u8,
    pub played: u8,
}

/// The keyboard, the zones, the layers and the progress.
#[derive(Clone, Debug, Default)]
pub struct KeyMapView {
    pub title: String,
    pub right: String,
    /// The keys drawn, lowest to highest (whole octaves round the notes).
    pub first: u8,
    pub last: u8,
    /// Each sampled note's zone (its root is the note played).
    pub zones: Vec<Keys>,
    /// Zones whose takes are all done.
    pub done: Vec<bool>,
    /// The note playing now.
    pub current: Option<u8>,
    pub layers: Vec<LayerBand>,
    pub round_robins: u8,
    /// Takes done, of all.
    pub progress: Option<(usize, usize)>,
    /// A line under the velocity axis, such as why it can't start.
    pub note: String,
    pub caption: String,
}

impl KeyMapView {
    pub fn new(title: &str) -> KeyMapView {
        KeyMapView {
            title: title.into(),
            first: 36,
            last: 95,
            round_robins: 1,
            ..KeyMapView::default()
        }
    }

    /// Draws whole octaves from the C at or below `lo` (less 5 keys) to the
    /// B at or above `hi` (plus 7), at least three octaves.
    pub fn show_range(&mut self, lo: u8, hi: u8) {
        let mut first = lo.saturating_sub(5);
        first -= first % 12;
        let mut last = hi.saturating_add(7).min(127);
        last = (last - last % 12 + 11).min(127);
        while last - first < 35 {
            if last < 127 {
                last = (last + 12).min(127);
            } else if first >= 12 {
                first -= 12;
            } else {
                break;
            }
        }
        self.first = first;
        self.last = last;
    }

    /// The zones' band, the keyboard under it, the velocity axis under
    /// that and the caption, inside the panel's body.
    pub fn areas(r: Rect) -> (Rect, Rect, Rect, Rect) {
        let body = r.inset(14).take_top(34).1;
        let (caption, rest) = body.take_bottom(24);
        let (band, rest) = rest.take_top(46);
        let keys_h = (rest.h - 130).clamp(80, 180);
        let (keys, vel) = rest.take_top(keys_h);
        (
            band,
            keys,
            Rect::new(vel.x, vel.y + 16, vel.w, vel.h - 16),
            caption,
        )
    }
}

fn is_black(note: u8) -> bool {
    matches!(note % 12, 1 | 3 | 6 | 8 | 10)
}

impl Widget for KeyMapView {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.titled_panel(r, &self.title, &self.right);
        let (band, keys, vel, caption) = KeyMapView::areas(r);
        let small = t.size_small - 1.0;
        let rec = Color::rgb(REC_COLOR);
        // The white keys share the width; black keys sit on their edges.
        let whites: Vec<u8> = (self.first..=self.last).filter(|n| !is_black(*n)).collect();
        let n_white = whites.len().max(1) as i32;
        let kw = keys.w / n_white;
        let x0 = keys.x + (keys.w - kw * n_white) / 2;
        let white_index = |note: u8| whites.iter().position(|w| *w == note);
        // Left and right edge of a key's column, in the keyboard's pixels.
        let key_span = |note: u8| -> (i32, i32) {
            if note < self.first {
                return (x0, x0);
            }
            if note > self.last {
                return (x0 + kw * n_white, x0 + kw * n_white);
            }
            match white_index(note) {
                Some(i) => (x0 + kw * i as i32, x0 + kw * (i as i32 + 1)),
                None => {
                    let i = white_index(note + 1).unwrap_or(0) as i32;
                    let c = x0 + kw * i;
                    (c - kw / 3, c + kw / 3)
                }
            }
        };
        // Zones: a band per sampled note over the keys it plays.
        for (k, z) in self.zones.iter().enumerate() {
            if z.hi < self.first || z.lo > self.last {
                continue;
            }
            let (a, _) = key_span(z.lo.max(self.first));
            let (_, b) = key_span(z.hi.min(self.last));
            let done = self.done.get(k).copied().unwrap_or(false);
            let playing = self.current.is_some_and(|c| c == z.root);
            let fill = if playing {
                t.bg.mix(rec, 0.45)
            } else if done {
                t.bg.mix(t.good, 0.35)
            } else if k % 2 == 0 {
                t.bg.mix(t.accent, 0.28)
            } else {
                t.bg.mix(t.accent, 0.16)
            };
            let zr = Rect::new(a + 1, band.y + 4, (b - a - 2).max(2), band.h - 8);
            cx.canvas.fill_round_rect(zr, 6, fill);
            // The root: a dot over its key.
            let (ra, rb) = key_span(z.root);
            let dot = Rect::new((ra + rb) / 2 - 4, zr.bottom() - 12, 8, 8);
            cx.canvas
                .fill_round_rect(dot, 4, if playing { rec } else { t.text });
            if z.lo < self.first && k == 0 {
                let lr = Rect::new(zr.x + 4, zr.y, 60, 18);
                cx.text(FontId::Bold, small, lr, Align::Left, t.text_dim, "← 0");
            }
            if z.hi > self.last && k + 1 == self.zones.len() {
                let lr = Rect::new(zr.right() - 64, zr.y, 60, 18);
                cx.text(FontId::Bold, small, lr, Align::Right, t.text_dim, "127 →");
            }
        }
        // The keys.
        let roots: Vec<u8> = self.zones.iter().map(|z| z.root).collect();
        for (i, &n) in whites.iter().enumerate() {
            let kr = Rect::new(x0 + kw * i as i32 + 1, keys.y, kw - 2, keys.h);
            let color = if self.current == Some(n) {
                rec
            } else if roots.contains(&n) {
                t.text.mix(t.accent, 0.35)
            } else {
                t.text.mix(t.bg, 0.25)
            };
            cx.canvas.fill_round_rect(kr, 3, color);
            if n % 12 == 0 {
                let lr = Rect::new(kr.x, kr.bottom() - 20, kr.w, 18);
                cx.text(FontId::Sans, small, lr, Align::Center, t.bg, &note_name(n));
            }
        }
        for n in self.first..=self.last {
            if !is_black(n) {
                continue;
            }
            let (a, b) = key_span(n);
            let kr = Rect::new(a, keys.y, b - a, keys.h * 3 / 5);
            let color = if self.current == Some(n) {
                rec
            } else if roots.contains(&n) {
                t.bg.mix(t.accent, 0.7)
            } else {
                t.bg
            };
            cx.canvas.fill_round_rect(kr, 3, color);
        }
        // Velocity: the axis from 1 to 127, a band per layer, a tick where
        // each is played.
        let label_w = 84;
        let axis = Rect::new(vel.x + label_w, vel.y + 22, vel.w - label_w, 36);
        let lr = Rect::new(vel.x, axis.y, label_w - 8, axis.h);
        cx.text(FontId::Bold, small, lr, Align::Left, t.text_dim, "Velocity");
        cx.canvas.fill_round_rect(axis, 6, t.bg);
        let x_of = |v: u8| axis.x + ((f32::from(v) - 1.0) / 126.0 * axis.w as f32) as i32;
        for (k, l) in self.layers.iter().enumerate() {
            let (a, b) = (x_of(l.lo), x_of(l.hi));
            let fill = if k % 2 == 0 {
                t.bg.mix(t.accent, 0.32)
            } else {
                t.bg.mix(t.accent, 0.18)
            };
            cx.canvas
                .fill_rect(Rect::new(a, axis.y + 2, (b - a).max(2), axis.h - 4), fill);
            let x = x_of(l.played);
            cx.canvas
                .fill_rect(Rect::new(x - 1, axis.y - 6, 3, axis.h + 12), t.text);
            let tr = Rect::new(x - 30, axis.y - 24, 60, 18);
            cx.text(
                FontId::Mono,
                small,
                tr,
                Align::Center,
                t.text,
                &l.played.to_string(),
            );
        }
        if self.round_robins > 1 {
            let rr = Rect::new(axis.x, axis.bottom() + 6, axis.w, 18);
            cx.text(
                FontId::Sans,
                small,
                rr,
                Align::Left,
                t.text_dim,
                &format!("{} takes of each, played in turn", self.round_robins),
            );
        }
        // Progress, under the axis.
        if let Some((done, total)) = self.progress {
            let pr = Rect::new(axis.x, axis.bottom() + 28, axis.w, 10);
            cx.canvas.fill_round_rect(pr, 5, t.bg);
            let w = (pr.w as f32 * done as f32 / total.max(1) as f32) as i32;
            cx.canvas
                .fill_round_rect(Rect::new(pr.x, pr.y, w.max(2), pr.h), 5, rec);
        }
        // A line under the axis, such as why it can't start.
        if !self.note.is_empty() {
            let nr = Rect::new(vel.x, axis.bottom() + 48, vel.w, 40);
            cx.canvas.fill_round_rect(nr, 8, t.bg.mix(t.warn, 0.18));
            cx.text(
                FontId::Bold,
                t.size_body,
                nr.inset(8),
                Align::Center,
                t.text,
                &self.note,
            );
        }
        if !self.caption.is_empty() {
            cx.text(
                FontId::Sans,
                small,
                caption,
                Align::Right,
                t.text_dim,
                &self.caption,
            );
        }
    }
}
