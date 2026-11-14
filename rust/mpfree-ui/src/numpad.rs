//! Typing a number (Jay after live test 3: "Warp tempo (and other numeric
//! fields with large range) should have ability to enter a number, instead
//! of only being able to use dial").
//!
//! A page asks with a [`NumberAsk`]: the field's name, its unit, its value
//! now, its range and how many decimals it takes. The [`NumPad`] shows the
//! keys (0 to 9, the point and the minus where the field takes them,
//! Delete),
//! with Cancel and Enter, each at least 64 px. What is typed shows large,
//! with the value now faint under it until the first key. Enter with a
//! number out of range keeps the pad open and says the range in red; Enter
//! with nothing typed, or Cancel, closes it and changes nothing.
//!
//! Every key gives [`EventKind::Tab`] with its index ([`KEY_ENTER`],
//! [`KEY_CANCEL`] and the rest); the app acts on Enter (reading
//! [`NumPad::entered`]) and Cancel, and the pad keeps what is typed itself.
//! The wheel and – / + step the number ([`NumPad::nudge`]).

use crate::geom::Rect;
use crate::text::{Align, FontId};
use crate::ui::{EventKind, PaintCx, Touch, Widget};

/// What a page asks to be typed.
#[derive(Clone, Debug, PartialEq)]
pub struct NumberAsk {
    /// The page's own name for the field, handed back with the number.
    pub key: String,
    /// "Warp tempo".
    pub title: String,
    /// "BPM", or empty.
    pub unit: String,
    /// The value now.
    pub value: f64,
    pub min: f64,
    pub max: f64,
    /// Places after the point: 0 takes whole numbers only.
    pub decimals: u8,
}

impl NumberAsk {
    /// `x` as the field shows it: its decimals, trailing zeros dropped.
    pub fn text(&self, x: f64) -> String {
        let s = format!("{x:.*}", self.decimals as usize);
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        }
    }

    /// "20 to 999 BPM".
    pub fn range(&self) -> String {
        let unit = if self.unit.is_empty() {
            String::new()
        } else {
            format!(" {}", self.unit)
        };
        format!("{} to {}{unit}", self.text(self.min), self.text(self.max))
    }
}

/// The keys, as `Tab` indices: 0 to 9 are the digits.
pub const KEY_POINT: usize = 10;
pub const KEY_MINUS: usize = 11;
pub const KEY_BACK: usize = 12;
pub const KEY_CANCEL: usize = 13;
pub const KEY_ENTER: usize = 14;

/// The most characters typed (a minus, digits and a point).
const MAX_TYPED: usize = 9;
/// The card's width at most, and a key row's height at least.
const CARD_W: i32 = 760;
const KEY_H: i32 = 64;
const GAP: i32 = 10;
const DISPLAY_H: i32 = 112;

/// The keypad. See the module page.
#[derive(Default)]
pub struct NumPad {
    pub ask: Option<NumberAsk>,
    /// What is typed so far ("12", "-3.5").
    pub typed: String,
    /// Said under the number: the range after an Enter out of it.
    pub warning: String,
}

impl NumPad {
    /// Ask for a number: the pad starts empty.
    pub fn open(&mut self, ask: NumberAsk) {
        self.ask = Some(ask);
        self.typed.clear();
        self.warning.clear();
    }

    pub fn close(&mut self) {
        self.ask = None;
        self.typed.clear();
        self.warning.clear();
    }

    /// The number typed, if it is one and in range; otherwise None, with
    /// the warning set (empty when nothing was typed).
    pub fn entered(&mut self) -> Option<f64> {
        let ask = self.ask.as_ref()?;
        if self.typed.is_empty() || self.typed == "-" || self.typed == "." {
            self.warning.clear();
            return None;
        }
        match self.typed.parse::<f64>() {
            Ok(x) if x.is_finite() && (ask.min..=ask.max).contains(&x) => {
                self.warning.clear();
                Some(x)
            }
            _ => {
                self.warning = format!("{}: type {}", ask.title, ask.range());
                None
            }
        }
    }

    /// The wheel or – / +: `steps` whole steps (1, or 0.1 for a field
    /// with decimals) from what is typed, or from the value now.
    pub fn nudge(&mut self, steps: i32) {
        let Some(ask) = self.ask.as_ref() else {
            return;
        };
        let from = self.typed.parse::<f64>().unwrap_or(ask.value);
        let step = if ask.decimals == 0 { 1.0 } else { 0.1 };
        let to = (from + f64::from(steps) * step).clamp(ask.min, ask.max);
        self.typed = ask.text(to);
        self.warning.clear();
    }

    /// A key: what it does to the typing. Enter and Cancel are the app's.
    fn press(&mut self, key: usize) {
        let Some(ask) = self.ask.as_ref() else {
            return;
        };
        self.warning.clear();
        match key {
            0..=9 if self.typed.len() < MAX_TYPED => {
                let decimals = self.typed.split('.').nth(1).map_or(0, str::len);
                if !self.typed.contains('.') || decimals < ask.decimals as usize {
                    if self.typed == "0" {
                        self.typed.clear();
                    }
                    self.typed.push(char::from(b'0' + key as u8));
                }
            }
            KEY_POINT if ask.decimals > 0 && !self.typed.contains('.') => {
                if self.typed.is_empty() || self.typed == "-" {
                    self.typed.push('0');
                }
                self.typed.push('.');
            }
            KEY_MINUS if ask.min < 0.0 => {
                if self.typed.starts_with('-') {
                    self.typed.remove(0);
                } else {
                    self.typed.insert(0, '-');
                }
            }
            KEY_BACK => {
                self.typed.pop();
            }
            _ => {}
        }
    }

    /// Whether key `k` does anything for this field.
    fn usable(&self, k: usize) -> bool {
        let Some(ask) = self.ask.as_ref() else {
            return false;
        };
        match k {
            KEY_POINT => ask.decimals > 0,
            KEY_MINUS => ask.min < 0.0,
            _ => true,
        }
    }

    /// The card in `r`: centred, as wide as fits up to [`CARD_W`].
    fn card(r: Rect) -> Rect {
        let w = r.w.min(CARD_W);
        Rect::new(r.x + (r.w - w) / 2, r.y, w, r.h)
    }

    /// The number's box in `r`.
    fn display(r: Rect) -> Rect {
        let c = NumPad::card(r).inset(16);
        Rect::new(c.x, c.y + 40, c.w, DISPLAY_H)
    }

    /// Each key and its rectangle, Enter two rows tall.
    pub fn keys(r: Rect) -> Vec<(usize, Rect)> {
        let c = NumPad::card(r).inset(16);
        let top = NumPad::display(r).bottom() + 40;
        let h = ((c.bottom() - top - 3 * GAP) / 4).max(KEY_H);
        let w = (c.w - 3 * GAP) / 4;
        let at = |col: i32, row: i32, rows: i32| {
            Rect::new(
                c.x + col * (w + GAP),
                top + row * (h + GAP),
                w,
                h * rows + GAP * (rows - 1),
            )
        };
        vec![
            (7, at(0, 0, 1)),
            (8, at(1, 0, 1)),
            (9, at(2, 0, 1)),
            (KEY_BACK, at(3, 0, 1)),
            (4, at(0, 1, 1)),
            (5, at(1, 1, 1)),
            (6, at(2, 1, 1)),
            (KEY_CANCEL, at(3, 1, 1)),
            (1, at(0, 2, 1)),
            (2, at(1, 2, 1)),
            (3, at(2, 2, 1)),
            (KEY_ENTER, at(3, 2, 2)),
            (KEY_MINUS, at(0, 3, 1)),
            (0, at(1, 3, 1)),
            (KEY_POINT, at(2, 3, 1)),
        ]
    }

    fn label(k: usize) -> String {
        match k {
            0..=9 => k.to_string(),
            KEY_POINT => ".".into(),
            KEY_MINUS => "−".into(),
            KEY_BACK => "Delete".into(),
            KEY_CANCEL => "Cancel".into(),
            _ => "Enter".into(),
        }
    }

    /// The middle of key `k` (for the app's tests).
    pub fn key_at(r: Rect, k: usize) -> (i32, i32) {
        let (_, kr) = NumPad::keys(r)
            .into_iter()
            .find(|(i, _)| *i == k)
            .expect("a key");
        (kr.x + kr.w / 2, kr.y + kr.h / 2)
    }
}

impl Widget for NumPad {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        let Some(ask) = self.ask.as_ref() else {
            return;
        };
        let card = NumPad::card(r);
        cx.panel(card);
        let inner = card.inset(16);
        let head = Rect::new(inner.x, inner.y, inner.w, 32);
        cx.text(
            FontId::Bold,
            t.size_title,
            head,
            Align::Left,
            t.text,
            &format!("Type the {}", ask.title.to_lowercase()),
        );
        cx.text(
            FontId::Sans,
            t.size_body,
            head,
            Align::Right,
            t.text_dim,
            &ask.range(),
        );
        let d = NumPad::display(r);
        cx.canvas.fill_round_rect(d, 12, t.raised);
        cx.canvas.stroke_round_rect(d, 12, 2, t.accent);
        let unit = if ask.unit.is_empty() {
            String::new()
        } else {
            format!(" {}", ask.unit)
        };
        let (big, small) = d.inset_xy(20, 8).take_top(64);
        if self.typed.is_empty() {
            cx.text(FontId::Bold, 44.0, big, Align::Right, t.text_faint, &unit);
        } else {
            cx.text(
                FontId::Bold,
                44.0,
                big,
                Align::Right,
                t.text,
                &format!("{}{unit}", self.typed),
            );
        }
        cx.text(
            FontId::Sans,
            t.size_body,
            small,
            Align::Right,
            t.text_dim,
            &format!("now {}{unit}", ask.text(ask.value)),
        );
        let below = Rect::new(inner.x, d.bottom() + 6, inner.w, 28);
        if !self.warning.is_empty() {
            cx.text(
                FontId::Bold,
                t.size_body,
                below,
                Align::Left,
                t.bad,
                &self.warning,
            );
        }
        for (k, kr) in NumPad::keys(r) {
            let usable = self.usable(k);
            let enter = k == KEY_ENTER;
            cx.canvas
                .fill_round_rect(kr, 12, if enter { t.accent } else { t.raised });
            let ink = if enter {
                t.on_accent
            } else {
                t.button_ink(usable)
            };
            let size = if k <= 9 || k == KEY_POINT || k == KEY_MINUS {
                32.0
            } else {
                t.size_title
            };
            cx.text(
                FontId::Bold,
                size,
                kr,
                Align::Center,
                ink,
                &NumPad::label(k),
            );
        }
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down || self.ask.is_none() {
            return None;
        }
        let (k, _) = NumPad::keys(r)
            .into_iter()
            .find(|(_, kr)| kr.contains(x, y))?;
        self.press(k);
        Some(EventKind::Tab(k))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempo() -> NumberAsk {
        NumberAsk {
            key: "tempo".into(),
            title: "Warp tempo".into(),
            unit: "BPM".into(),
            value: 92.0,
            min: 20.0,
            max: 999.0,
            decimals: 2,
        }
    }

    const R: Rect = Rect {
        x: 0,
        y: 0,
        w: 1256,
        h: 690,
    };

    fn tap(p: &mut NumPad, k: usize) -> Option<EventKind> {
        let (x, y) = NumPad::key_at(R, k);
        p.touch(1, Touch::Down, x, y, R)
    }

    #[test]
    fn keys_type_a_number_and_enter_checks_its_range() {
        let mut p = NumPad::default();
        p.open(tempo());
        for k in [1, 2, 7, KEY_POINT, 5] {
            assert_eq!(tap(&mut p, k), Some(EventKind::Tab(k)));
        }
        assert_eq!(p.typed, "127.5");
        assert_eq!(p.entered(), Some(127.5));
        // Two decimals at most; Delete takes the last one off.
        tap(&mut p, 0);
        tap(&mut p, 9);
        assert_eq!(p.typed, "127.50");
        tap(&mut p, KEY_BACK);
        assert_eq!(p.typed, "127.5");
        // Out of range: no number, and the range said.
        p.typed = "1500".into();
        assert_eq!(p.entered(), None);
        assert_eq!(p.warning, "Warp tempo: type 20 to 999 BPM");
        // Nothing typed: no number and no warning (Enter closes).
        p.typed.clear();
        assert_eq!(p.entered(), None);
        assert_eq!(p.warning, "");
        assert_eq!(tap(&mut p, KEY_ENTER), Some(EventKind::Tab(KEY_ENTER)));
        assert_eq!(tap(&mut p, KEY_CANCEL), Some(EventKind::Tab(KEY_CANCEL)));
    }

    #[test]
    fn a_whole_number_field_takes_no_point_and_minus_only_below_zero() {
        let mut p = NumPad::default();
        p.open(NumberAsk {
            decimals: 0,
            ..tempo()
        });
        for k in [KEY_MINUS, 8, KEY_POINT, 5] {
            tap(&mut p, k);
        }
        assert_eq!(p.typed, "85");
        p.open(NumberAsk {
            min: -24.0,
            max: 24.0,
            ..tempo()
        });
        for k in [KEY_MINUS, KEY_POINT, 5] {
            tap(&mut p, k);
        }
        assert_eq!(p.typed, "-0.5");
        assert_eq!(p.entered(), Some(-0.5));
    }

    #[test]
    fn the_wheel_steps_from_the_value_now_and_stays_in_range() {
        let mut p = NumPad::default();
        p.open(NumberAsk {
            decimals: 0,
            ..tempo()
        });
        p.nudge(3);
        assert_eq!(p.typed, "95");
        p.nudge(-2000);
        assert_eq!(p.typed, "20");
    }

    #[test]
    fn every_key_is_big_enough_to_hit() {
        for (k, kr) in NumPad::keys(R) {
            assert!(kr.w >= 64 && kr.h >= 64, "key {k}: {kr:?}");
        }
    }
}
