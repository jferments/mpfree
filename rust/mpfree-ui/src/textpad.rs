//! Typing a name (the sampler review's R1: "No names. There is no way to
//! type one; names come from files and takes").
//!
//! A page asks with one or more [`TextAsk`]s: what is being named, its
//! name now and how long a name may be. The [`TextPad`] shows the name now,
//! ready to change (Delete takes off the letter before the cursor, Clear
//! takes off the lot), over a keyboard: the digits, the letters in QWERTY
//! rows, Shift (once, for the next letter; on by itself at the start of an
//! empty name), the hyphen, apostrophe and point, #+= (the letter keys
//! type symbols until ABC; Jay, 17 Nov: "It would be nice to be able to
//! input more symbols"), Space, Cancel and Enter, every key at least
//! 64 px. With more than one ask (a pad and its sound),
//! a switch at the top picks which name the keys type, and each keeps what
//! is typed in it. Enter with a name left empty keeps the pad open and
//! says so in red; Cancel closes it and changes nothing.
//!
//! Every key gives [`EventKind::Tab`] with its index ([`KEY_ENTER`],
//! [`KEY_CANCEL`] and the rest); the app acts on Enter (reading
//! [`TextPad::entered`]) and Cancel, and the pad keeps what is typed
//! itself. The wheel and – / + move the cursor ([`TextPad::nudge`]).

use crate::geom::Rect;
use crate::text::{Align, FontId};
use crate::ui::{EventKind, PaintCx, Target, Touch, Widget};

/// What a page asks to be named.
#[derive(Clone, Debug, PartialEq)]
pub struct TextAsk {
    /// The page's own name for the field, handed back with the name.
    pub key: String,
    /// The switch's word for it, with more than one ask: "Pad", "Sound".
    pub label: String,
    /// "Name pad A5".
    pub title: String,
    /// The name now.
    pub value: String,
    /// The most characters a name may have.
    pub max_len: usize,
}

/// The character keys, as `Tab` indices 0 to 38: each key's character, and
/// with Shift.
const CHARS: [(char, char); 39] = [
    ('1', '!'),
    ('2', '@'),
    ('3', '#'),
    ('4', '$'),
    ('5', '%'),
    ('6', '&'),
    ('7', '('),
    ('8', ')'),
    ('9', '_'),
    ('0', '+'),
    ('q', 'Q'),
    ('w', 'W'),
    ('e', 'E'),
    ('r', 'R'),
    ('t', 'T'),
    ('y', 'Y'),
    ('u', 'U'),
    ('i', 'I'),
    ('o', 'O'),
    ('p', 'P'),
    ('a', 'A'),
    ('s', 'S'),
    ('d', 'D'),
    ('f', 'F'),
    ('g', 'G'),
    ('h', 'H'),
    ('j', 'J'),
    ('k', 'K'),
    ('l', 'L'),
    ('z', 'Z'),
    ('x', 'X'),
    ('c', 'C'),
    ('v', 'V'),
    ('b', 'B'),
    ('n', 'N'),
    ('m', 'M'),
    ('-', '-'),
    ('\'', '\''),
    ('.', ','),
];

/// What the letter keys (indices 10 to 35) type while #+= is on.
const SYMBOLS: [char; 26] = [
    '!', '@', '#', '$', '%', '^', '&', '*', '(', ')', '~', '`', '=', '+', '[', ']', '{', '}', ';',
    ':', '"', '<', '>', '?', '/', '|',
];
/// The letter keys' indices.
const LETTERS: std::ops::Range<usize> = 10..36;

/// The other keys, as `Tab` indices.
pub const KEY_SHIFT: usize = 40;
pub const KEY_BACK: usize = 41;
pub const KEY_CLEAR: usize = 42;
pub const KEY_SPACE: usize = 43;
pub const KEY_CANCEL: usize = 44;
pub const KEY_ENTER: usize = 45;
/// The letter keys type symbols, or letters again.
pub const KEY_SYMBOLS: usize = 46;
/// The switch's ask `i` is `KEY_FIELD + i`.
pub const KEY_FIELD: usize = 50;

/// The letter key typing `c` while #+= is on, for tests.
pub fn symbol_key(c: char) -> Option<usize> {
    SYMBOLS
        .iter()
        .position(|&s| s == c)
        .map(|i| LETTERS.start + i)
}

/// The character key typing `c` (without or with Shift), for tests.
pub fn char_key(c: char) -> Option<(usize, bool)> {
    CHARS.iter().enumerate().find_map(|(i, &(a, b))| {
        if a == c {
            Some((i, false))
        } else if b == c {
            Some((i, true))
        } else {
            None
        }
    })
}

/// The card's width at most, a key's height at least, and the parts above
/// the keys.
const CARD_W: i32 = 1248;
const KEY_H: i32 = 64;
const GAP: i32 = 10;
const HEAD_H: i32 = 64;
const DISPLAY_H: i32 = 96;
const WARN_H: i32 = 30;
/// One switch segment's width.
const FIELD_W: i32 = 168;

/// The keyboard. See the module page.
#[derive(Default)]
pub struct TextPad {
    pub asks: Vec<TextAsk>,
    /// The ask the keys type into.
    pub field: usize,
    /// What is typed in each ask (each starts as its name now).
    pub typed: Vec<String>,
    /// The cursor in the name showing, in characters from its start.
    pub caret: usize,
    /// The next letter is a capital (or the key's other character).
    pub shift: bool,
    /// The letter keys type symbols ([`KEY_SYMBOLS`]), until it is
    /// pressed again.
    pub symbols: bool,
    /// Said under the name: why Enter didn't close the pad.
    pub warning: String,
}

impl TextPad {
    /// Ask for names: each starts as it is now, the first one showing with
    /// the cursor at its end.
    pub fn open(&mut self, asks: Vec<TextAsk>) {
        self.typed = asks.iter().map(|a| a.value.clone()).collect();
        self.asks = asks;
        self.warning.clear();
        self.symbols = false;
        self.pick(0);
    }

    pub fn close(&mut self) {
        self.asks.clear();
        self.typed.clear();
        self.field = 0;
        self.caret = 0;
        self.shift = false;
        self.symbols = false;
        self.warning.clear();
    }

    pub fn is_open(&self) -> bool {
        !self.asks.is_empty()
    }

    /// The ask showing.
    pub fn ask(&self) -> Option<&TextAsk> {
        self.asks.get(self.field)
    }

    /// What is typed in the ask showing.
    pub fn text(&self) -> &str {
        self.typed.get(self.field).map_or("", String::as_str)
    }

    fn len(&self) -> usize {
        self.text().chars().count()
    }

    /// Show ask `i`, with the cursor at the end of its name.
    fn pick(&mut self, i: usize) {
        if i >= self.asks.len() {
            return;
        }
        self.field = i;
        self.caret = self.len();
        self.shift = self.caret == 0;
    }

    /// Enter: each name that changed, by its key, with spaces trimmed off
    /// its ends (none when nothing changed). None, with the warning set and
    /// that name showing, when a name is left empty.
    pub fn entered(&mut self) -> Option<Vec<(String, String)>> {
        if !self.is_open() {
            return None;
        }
        let mut out = Vec::new();
        for i in 0..self.asks.len() {
            let name = self.typed[i].trim();
            if name.is_empty() {
                let ask = &self.asks[i];
                self.warning = format!(
                    "{}: type a name, or Cancel to keep {}",
                    ask.label, ask.value
                );
                self.pick(i);
                return None;
            }
            if name != self.asks[i].value {
                out.push((self.asks[i].key.clone(), name.to_string()));
            }
        }
        self.warning.clear();
        Some(out)
    }

    /// The wheel or – / +: the cursor `steps` characters on (or back).
    pub fn nudge(&mut self, steps: i32) {
        if !self.is_open() {
            return;
        }
        let to = (self.caret as i64 + i64::from(steps)).clamp(0, self.len() as i64);
        self.caret = to as usize;
    }

    /// The byte where the cursor is in the name showing.
    fn caret_byte(&self) -> usize {
        let t = self.text();
        t.char_indices().nth(self.caret).map_or(t.len(), |(b, _)| b)
    }

    /// Puts `c` in at the cursor.
    fn put(&mut self, c: char) {
        let at = self.caret_byte();
        if let Some(t) = self.typed.get_mut(self.field) {
            t.insert(at, c);
            self.caret += 1;
        }
    }

    /// A key: what it does to the typing. Enter and Cancel are the app's.
    fn press(&mut self, key: usize) {
        if !self.usable(key) {
            return;
        }
        self.warning.clear();
        match key {
            k if k < CHARS.len() => {
                self.put(self.char_of(k));
                self.shift = false;
            }
            KEY_SPACE => self.put(' '),
            KEY_SYMBOLS => self.symbols = !self.symbols,
            KEY_SHIFT => self.shift = !self.shift,
            KEY_BACK => {
                let end = self.caret_byte();
                let field = self.field;
                let t = &mut self.typed[field];
                if let Some((from, _)) = t[..end].char_indices().last() {
                    t.replace_range(from..end, "");
                    self.caret -= 1;
                }
                self.shift = self.typed[field].is_empty();
            }
            KEY_CLEAR => {
                self.typed[self.field].clear();
                self.caret = 0;
                self.shift = true;
            }
            k if k >= KEY_FIELD => self.pick(k - KEY_FIELD),
            _ => {}
        }
    }

    /// Whether key `k` does anything now.
    fn usable(&self, k: usize) -> bool {
        let Some(ask) = self.ask() else {
            return false;
        };
        let room = self.len() < ask.max_len;
        match k {
            k if k < CHARS.len() => room,
            KEY_SPACE => room && self.caret > 0,
            KEY_BACK => self.caret > 0,
            KEY_CLEAR => !self.text().is_empty(),
            KEY_SHIFT | KEY_CANCEL | KEY_ENTER | KEY_SYMBOLS => true,
            k if k >= KEY_FIELD => k - KEY_FIELD < self.asks.len() && k - KEY_FIELD != self.field,
            _ => false,
        }
    }

    /// The card in `r`: centred, as wide as fits up to [`CARD_W`].
    fn card(r: Rect) -> Rect {
        let w = r.w.min(CARD_W);
        Rect::new(r.x + (r.w - w) / 2, r.y, w, r.h)
    }

    /// The name's box in `r`.
    fn display(r: Rect) -> Rect {
        let c = TextPad::card(r).inset(16);
        Rect::new(c.x, c.y + HEAD_H + GAP, c.w, DISPLAY_H)
    }

    /// The switch's segments (one per ask, when there are two or more),
    /// at the head's right.
    pub fn fields(&self, r: Rect) -> Vec<(usize, Rect)> {
        if self.asks.len() < 2 {
            return Vec::new();
        }
        let c = TextPad::card(r).inset(16);
        let n = self.asks.len() as i32;
        let x0 = c.right() - n * FIELD_W - (n - 1) * GAP;
        (0..self.asks.len())
            .map(|i| {
                let x = x0 + i as i32 * (FIELD_W + GAP);
                (KEY_FIELD + i, Rect::new(x, c.y, FIELD_W, HEAD_H))
            })
            .collect()
    }

    /// Each key and its rectangle: five rows of ten places, Space four
    /// places wide and Cancel and Enter two.
    pub fn keys(r: Rect) -> Vec<(usize, Rect)> {
        let c = TextPad::card(r).inset(16);
        let top = TextPad::display(r).bottom() + WARN_H + GAP;
        let h = ((c.bottom() - top - 4 * GAP) / 5).max(KEY_H);
        let w = (c.w - 9 * GAP) / 10;
        let at = |col: i32, row: i32, cols: i32| {
            Rect::new(
                c.x + col * (w + GAP),
                top + row * (h + GAP),
                w * cols + GAP * (cols - 1),
                h,
            )
        };
        let mut out = Vec::with_capacity(46);
        // Digits, then Q to P.
        for i in 0..20 {
            out.push((i, at(i as i32 % 10, i as i32 / 10, 1)));
        }
        // A to L, and Delete.
        for i in 20..29 {
            out.push((i, at(i as i32 - 20, 2, 1)));
        }
        out.push((KEY_BACK, at(9, 2, 1)));
        // Shift, Z to M, the hyphen and the apostrophe.
        out.push((KEY_SHIFT, at(0, 3, 1)));
        for i in 29..38 {
            out.push((i, at(i as i32 - 28, 3, 1)));
        }
        // Cancel, Clear, the symbols, Space, the point and Enter.
        out.push((KEY_CANCEL, at(0, 4, 2)));
        out.push((KEY_CLEAR, at(2, 4, 1)));
        out.push((KEY_SYMBOLS, at(3, 4, 1)));
        out.push((KEY_SPACE, at(4, 4, 3)));
        out.push((38, at(7, 4, 1)));
        out.push((KEY_ENTER, at(8, 4, 2)));
        out
    }

    /// What character key `k` types now.
    fn char_of(&self, k: usize) -> char {
        if self.symbols && LETTERS.contains(&k) {
            return SYMBOLS[k - LETTERS.start];
        }
        let (a, b) = CHARS[k];
        if self.shift {
            b
        } else {
            a
        }
    }

    fn label(&self, k: usize) -> String {
        match k {
            k if k < CHARS.len() => self.char_of(k).to_string(),
            KEY_SYMBOLS if self.symbols => "ABC".into(),
            KEY_SYMBOLS => "#+=".into(),
            KEY_SHIFT => "Shift".into(),
            KEY_BACK => "Delete".into(),
            KEY_CLEAR => "Clear".into(),
            KEY_SPACE => "Space".into(),
            KEY_CANCEL => "Cancel".into(),
            KEY_ENTER => "Enter".into(),
            k => self
                .asks
                .get(k.saturating_sub(KEY_FIELD))
                .map_or_else(String::new, |a| a.label.clone()),
        }
    }

    /// The middle of key `k` (for the app's tests).
    pub fn key_at(r: Rect, k: usize) -> (i32, i32) {
        let (_, kr) = TextPad::keys(r)
            .into_iter()
            .find(|(i, _)| *i == k)
            .expect("a key");
        (kr.x + kr.w / 2, kr.y + kr.h / 2)
    }

    /// The key or switch segment at (x, y).
    fn hit(&self, r: Rect, x: i32, y: i32) -> Option<usize> {
        TextPad::keys(r)
            .into_iter()
            .chain(self.fields(r))
            .find(|(_, kr)| kr.contains(x, y))
            .map(|(k, _)| k)
    }
}

impl Widget for TextPad {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        let Some(ask) = self.ask() else {
            return;
        };
        let card = TextPad::card(r);
        cx.panel(card);
        let inner = card.inset(16);
        // The head: what is named, its name now and how long it may be;
        // the switch at the right.
        let fields = self.fields(r);
        let head_w = fields.first().map_or(inner.w, |(_, f)| f.x - inner.x - GAP);
        let (title, sub) = Rect::new(inner.x, inner.y, head_w, HEAD_H).take_top(36);
        cx.text(
            FontId::Bold,
            t.size_title,
            title,
            Align::Left,
            t.text,
            &ask.title,
        );
        cx.text(
            FontId::Sans,
            t.size_body,
            sub,
            Align::Left,
            t.text_dim,
            &format!("now {} · up to {} characters", ask.value, ask.max_len),
        );
        for (k, fr) in &fields {
            let on = *k - KEY_FIELD == self.field;
            cx.canvas.fill_round_rect(*fr, 12, t.raised);
            if on {
                cx.canvas.stroke_round_rect(*fr, 12, 2, t.accent);
            }
            cx.text(
                if on { FontId::Bold } else { FontId::Sans },
                t.size_title,
                *fr,
                Align::Center,
                if on { t.text } else { t.text_dim },
                &self.label(*k),
            );
        }
        // The name, with the cursor; smaller when it is long.
        let d = TextPad::display(r);
        cx.canvas.fill_round_rect(d, 12, t.raised);
        cx.canvas.stroke_round_rect(d, 12, 2, t.accent);
        let line = d.inset_xy(20, 0);
        let text = self.text();
        let size = [40.0, 32.0, 24.0]
            .into_iter()
            .find(|&s| cx.measure(FontId::Bold, s, text) + 8 <= line.w)
            .unwrap_or(24.0);
        cx.text(FontId::Bold, size, line, Align::Left, t.text, text);
        let before = &text[..self.caret_byte()];
        let x = (line.x + cx.measure(FontId::Bold, size, before) + 2).min(line.right() - 3);
        let h = size as i32 + 8;
        cx.canvas
            .fill_rect(Rect::new(x, d.y + (d.h - h) / 2, 3, h), t.accent);
        if !self.warning.is_empty() {
            let below = Rect::new(inner.x, d.bottom() + 4, inner.w, WARN_H - 4);
            cx.text(
                FontId::Bold,
                t.size_body,
                below,
                Align::Left,
                t.bad,
                &self.warning,
            );
        }
        for (k, kr) in TextPad::keys(r) {
            let usable = self.usable(k);
            let enter = k == KEY_ENTER;
            let shift_on = (k == KEY_SHIFT && self.shift) || (k == KEY_SYMBOLS && self.symbols);
            cx.canvas
                .fill_round_rect(kr, 12, if enter { t.accent } else { t.raised });
            if shift_on {
                cx.canvas.stroke_round_rect(kr, 12, 2, t.accent);
            }
            let ink = if enter {
                t.on_accent
            } else {
                t.button_ink(usable)
            };
            let size = if k < CHARS.len() { 30.0 } else { t.size_title };
            cx.text(FontId::Bold, size, kr, Align::Center, ink, &self.label(k));
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        if !self.is_open() {
            return Vec::new();
        }
        TextPad::keys(r)
            .into_iter()
            .map(|(k, kr)| Target::tap("key", k, self.label(k), kr).disabled_if(!self.usable(k)))
            .chain(self.fields(r).into_iter().map(|(k, fr)| {
                Target::tap("field", k - KEY_FIELD, self.label(k), fr).disabled_if(!self.usable(k))
            }))
            .collect()
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down || !self.is_open() {
            return None;
        }
        let k = self.hit(r, x, y)?;
        if !self.usable(k) {
            return None;
        }
        self.press(k);
        Some(EventKind::Tab(k))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad_name(value: &str) -> TextAsk {
        TextAsk {
            key: "pad".into(),
            label: "Pad".into(),
            title: "Name pad A5".into(),
            value: value.into(),
            max_len: 12,
        }
    }

    fn sound_name(value: &str) -> TextAsk {
        TextAsk {
            key: "sound".into(),
            label: "Sound".into(),
            title: "Name pad A5's sound".into(),
            value: value.into(),
            max_len: 12,
        }
    }

    const R: Rect = Rect {
        x: 0,
        y: 0,
        w: 1280,
        h: 714,
    };

    fn tap(p: &mut TextPad, k: usize) -> Option<EventKind> {
        let (x, y) = TextPad::key_at(R, k);
        p.touch(1, Touch::Down, x, y, R)
    }

    /// Types `s` as a finger would, with Shift where a character needs it.
    fn type_in(p: &mut TextPad, s: &str) {
        for c in s.chars() {
            if c == ' ' {
                tap(p, KEY_SPACE);
                continue;
            }
            let (k, shifted) = char_key(c).expect("a key for it");
            if shifted != p.shift {
                tap(p, KEY_SHIFT);
            }
            assert_eq!(tap(p, k), Some(EventKind::Tab(k)), "{c}");
        }
    }

    #[test]
    fn the_name_now_is_ready_to_change_and_enter_gives_it_trimmed() {
        let mut p = TextPad::default();
        p.open(vec![pad_name("Break 92")]);
        assert_eq!((p.text(), p.caret, p.shift), ("Break 92", 8, false));
        tap(&mut p, KEY_BACK);
        tap(&mut p, KEY_BACK);
        type_in(&mut p, "Kick ");
        assert_eq!(p.text(), "Break Kick ");
        assert_eq!(
            p.entered(),
            Some(vec![("pad".to_string(), "Break Kick".to_string())])
        );
        // Unchanged: nothing to hand back, and the pad closes.
        p.open(vec![pad_name("Break 92")]);
        assert_eq!(p.entered(), Some(vec![]));
        assert_eq!(tap(&mut p, KEY_ENTER), Some(EventKind::Tab(KEY_ENTER)));
        assert_eq!(tap(&mut p, KEY_CANCEL), Some(EventKind::Tab(KEY_CANCEL)));
    }

    #[test]
    fn shift_is_for_one_letter_and_on_by_itself_in_an_empty_name() {
        let mut p = TextPad::default();
        p.open(vec![pad_name("x")]);
        tap(&mut p, KEY_CLEAR);
        assert!(p.shift, "an empty name starts with a capital");
        let (s, _) = char_key('s').unwrap();
        let (n, _) = char_key('n').unwrap();
        let (one, _) = char_key('1').unwrap();
        for k in [s, n, KEY_SHIFT, one, KEY_SHIFT, n] {
            tap(&mut p, k);
        }
        assert_eq!(p.text(), "Sn!N");
        assert_eq!(p.label(n), "n", "Shift is off again");
        tap(&mut p, KEY_SHIFT);
        assert_eq!(
            (p.label(n), p.label(one), p.label(38)),
            ("N".into(), "!".into(), ",".into())
        );
    }

    #[test]
    fn the_symbols_key_turns_the_letter_keys_into_symbols_until_abc() {
        let mut p = TextPad::default();
        p.open(vec![pad_name("Kick")]);
        let (q, _) = char_key('q').unwrap();
        assert_eq!(p.label(KEY_SYMBOLS), "#+=");
        assert_eq!(tap(&mut p, KEY_SYMBOLS), Some(EventKind::Tab(KEY_SYMBOLS)));
        assert_eq!(
            (p.label(q), p.label(KEY_SYMBOLS)),
            ("!".into(), "ABC".into())
        );
        for c in ['#', '[', '<', '|'] {
            tap(&mut p, symbol_key(c).unwrap());
        }
        // The digits, hyphen and point type as before; the symbols stay on.
        tap(&mut p, char_key('2').unwrap().0);
        tap(&mut p, char_key('-').unwrap().0);
        assert_eq!(p.text(), "Kick#[<|2-");
        assert!(p.symbols);
        tap(&mut p, KEY_SYMBOLS);
        tap(&mut p, q);
        assert_eq!(p.text(), "Kick#[<|2-q");
        // Every symbol is on a key, and a new name starts with letters.
        assert!(SYMBOLS.iter().all(|&c| symbol_key(c).is_some()));
        tap(&mut p, KEY_SYMBOLS);
        p.open(vec![pad_name("Kick")]);
        assert!(!p.symbols);
    }

    #[test]
    fn the_wheel_moves_the_cursor_and_keys_type_at_it() {
        let mut p = TextPad::default();
        p.open(vec![pad_name("Bak")]);
        p.nudge(-1);
        assert_eq!(p.caret, 2);
        type_in(&mut p, "c");
        assert_eq!((p.text(), p.caret), ("Back", 3));
        p.nudge(-10);
        assert_eq!(p.caret, 0);
        assert!(
            p.targets(R)
                .iter()
                .any(|t| t.index == KEY_BACK && !t.enabled),
            "nothing before the cursor to delete"
        );
        assert!(
            p.targets(R)
                .iter()
                .any(|t| t.index == KEY_SPACE && !t.enabled),
            "no space at the start"
        );
        p.nudge(2);
        tap(&mut p, KEY_BACK);
        assert_eq!((p.text(), p.caret), ("Bck", 1));
        p.nudge(99);
        assert_eq!(p.caret, 3);
    }

    #[test]
    fn a_full_name_takes_no_more_letters() {
        let mut p = TextPad::default();
        p.open(vec![pad_name("Twelve chars")]);
        assert_eq!(p.len(), 12);
        let (a, _) = char_key('a').unwrap();
        assert_eq!(tap(&mut p, a), None);
        assert_eq!(tap(&mut p, KEY_SPACE), None);
        assert_eq!(p.text(), "Twelve chars");
        tap(&mut p, KEY_BACK);
        assert_eq!(tap(&mut p, a), Some(EventKind::Tab(a)));
        assert_eq!(p.text(), "Twelve chara");
    }

    #[test]
    fn an_empty_name_keeps_the_pad_open_and_says_so() {
        let mut p = TextPad::default();
        p.open(vec![pad_name("Kick")]);
        tap(&mut p, KEY_CLEAR);
        tap(&mut p, KEY_SPACE);
        assert_eq!(p.text(), "", "no space at the start");
        assert_eq!(p.entered(), None);
        assert_eq!(p.warning, "Pad: type a name, or Cancel to keep Kick");
        type_in(&mut p, "K");
        assert_eq!(p.warning, "", "a key clears it");
    }

    #[test]
    fn the_switch_picks_which_name_the_keys_type_and_each_keeps_its_own() {
        let mut p = TextPad::default();
        p.open(vec![pad_name("Break 92 1"), sound_name("Break 92")]);
        assert_eq!(p.fields(R).len(), 2);
        assert!(
            p.targets(R)
                .iter()
                .any(|t| t.id == "field" && t.index == 0 && !t.enabled),
            "the field showing"
        );
        type_in(&mut p, "a");
        let (x, y) = {
            let (_, fr) = p.fields(R)[1];
            (fr.x + fr.w / 2, fr.y + fr.h / 2)
        };
        assert_eq!(
            p.touch(1, Touch::Down, x, y, R),
            Some(EventKind::Tab(KEY_FIELD + 1))
        );
        assert_eq!((p.text(), p.caret), ("Break 92", 8));
        tap(&mut p, KEY_CLEAR);
        type_in(&mut p, "Amen");
        assert_eq!(
            p.entered(),
            Some(vec![
                ("pad".to_string(), "Break 92 1a".to_string()),
                ("sound".to_string(), "Amen".to_string()),
            ])
        );
        // One name empty: that one shows, with the warning.
        p.open(vec![pad_name("Kick"), sound_name("Hit")]);
        p.pick(1);
        tap(&mut p, KEY_CLEAR);
        p.pick(0);
        assert_eq!(p.entered(), None);
        assert_eq!(
            (p.field, p.warning.as_str()),
            (1, "Sound: type a name, or Cancel to keep Hit")
        );
        // One ask: no switch.
        p.open(vec![pad_name("Kick")]);
        assert!(p.fields(R).is_empty());
    }

    #[test]
    fn every_key_is_big_enough_to_hit_and_none_overlap() {
        let mut p = TextPad::default();
        p.open(vec![pad_name("Kick"), sound_name("Hit")]);
        let keys: Vec<(usize, Rect)> = TextPad::keys(R).into_iter().chain(p.fields(R)).collect();
        assert_eq!(keys.len(), 39 + 7 + 2);
        let card = TextPad::card(R);
        for (i, (k, kr)) in keys.iter().enumerate() {
            assert!(kr.w >= 64 && kr.h >= 64, "key {k}: {kr:?}");
            assert!(
                kr.x >= card.x && kr.right() <= card.right() && kr.bottom() <= card.bottom(),
                "key {k} in the card"
            );
            for (j, kr2) in keys.iter().skip(i + 1) {
                assert!(kr.intersect(*kr2).is_empty(), "keys {k} and {j} overlap");
            }
        }
    }
}
