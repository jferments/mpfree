//! The Play tab's keyboard: every keygroup of the kit over the keys it
//! plays, the pad picked drawn with handles to drag its lowest and highest
//! key and its root, and the notes the 16 pads play marked under the keys.
//! Keys pressed on it play the kit as notes.

use mpfree_analysis::note_name;
use mpfree_sampler::model::Keys;
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Touch, Widget};

use crate::play_widgets::fade_cut;

/// One keygroup on the keyboard.
#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    /// The pad, as the kit numbers them from 0.
    pub pad: usize,
    pub keys: Keys,
    /// Its key fade, in notes: the bar is cut down along the fade's curve
    /// inside each edge (none at the ends of the keyboard).
    pub fade: u8,
    /// What the zone is called on the keyboard (`A3`).
    pub label: String,
    pub color: Color,
}

/// What a finger holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grab {
    /// The picked zone's lowest key (0), highest key (1) or root (2).
    Handle(usize),
    /// A key, held down.
    Key(u8),
}

/// The keyboard with the kit's keygroups over it. A tap on a zone gives
/// [`EventKind::Select`] with its pad; a drag on the picked zone's left or
/// right edge or its root gives [`EventKind::Marker`] with `index` 0, 1 or
/// 2 and the key as `frame`; a key pressed and let go gives
/// [`EventKind::Pad`] with the note as `index`.
#[derive(Clone, Debug, Default)]
pub struct KeyZones {
    pub title: String,
    pub right: String,
    pub caption: String,
    /// The keys drawn, lowest to highest.
    pub first: u8,
    pub last: u8,
    pub zones: Vec<Zone>,
    /// The pad picked, if it is one of the zones.
    pub picked: Option<usize>,
    /// The note pad 1 plays, when the pads play notes (the 16 pads' notes
    /// are marked under the keys).
    pub pads_from: Option<u8>,
    /// Keys held down, lit.
    pub held: Vec<u8>,
    grab: Option<(u64, Grab)>,
}

/// The most lanes zones are stacked in; more overlap in the last one.
const MAX_LANES: usize = 4;
/// How near (px) a handle must be touched.
const HANDLE: i32 = 14;

impl KeyZones {
    pub fn new(title: &str) -> KeyZones {
        KeyZones {
            title: title.into(),
            first: 36,
            last: 95,
            ..KeyZones::default()
        }
    }

    /// Draws whole octaves round the notes `lo` to `hi`, at least three
    /// octaves (five keys of room below, seven above).
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

    /// The zones' lanes, the keys, the pads' strip and the caption.
    fn areas(r: Rect) -> (Rect, Rect, Rect, Rect) {
        let body = r.inset(14).take_top(34).1;
        let (caption, rest) = body.take_bottom(22);
        let (pads, rest) = rest.take_bottom(20);
        let keys_h = (rest.h * 11 / 20).clamp(48, 150);
        let (keys, lanes) = rest.take_bottom(keys_h);
        (
            Rect::new(lanes.x, lanes.y, lanes.w, lanes.h - 6),
            keys,
            pads,
            caption,
        )
    }

    fn board(&self, r: Rect) -> Board {
        let (_, keys, _, _) = KeyZones::areas(r);
        Board::new(self.first, self.last.max(self.first), keys)
    }

    /// Each zone's lane, and how many lanes there are: a zone goes in the
    /// first lane where it overlaps no zone before it.
    fn lanes(&self) -> (Vec<usize>, usize) {
        let mut ends: Vec<u8> = Vec::new();
        let mut order: Vec<usize> = (0..self.zones.len()).collect();
        order.sort_by_key(|&i| (self.zones[i].keys.lo, self.zones[i].pad));
        let mut lane = vec![0; self.zones.len()];
        for i in order {
            let k = self.zones[i].keys;
            let l = match ends.iter().position(|&e| e < k.lo) {
                Some(l) => l,
                None if ends.len() < MAX_LANES => {
                    ends.push(0);
                    ends.len() - 1
                }
                None => MAX_LANES - 1,
            };
            ends[l] = ends[l].max(k.hi);
            lane[i] = l;
        }
        (lane, ends.len().max(1))
    }

    /// Zone `i`'s rectangle, if it shows.
    pub fn zone_rect(&self, r: Rect, i: usize) -> Option<Rect> {
        let z = self.zones.get(i)?;
        if z.keys.hi < self.first || z.keys.lo > self.last {
            return None;
        }
        let (band, ..) = KeyZones::areas(r);
        let b = self.board(r);
        let (lane, n) = self.lanes();
        let h = (band.h / n as i32).min(34);
        let (a, _) = b.span(z.keys.lo.max(self.first));
        let (_, e) = b.span(z.keys.hi.min(self.last));
        Some(Rect::new(
            a + 1,
            band.bottom() - h * (n - lane[i]) as i32 + 2,
            (e - a - 2).max(6),
            h - 4,
        ))
    }

    fn picked_index(&self) -> Option<usize> {
        let p = self.picked?;
        self.zones.iter().position(|z| z.pad == p)
    }

    /// The middle of key `note`, low on the key (where only it is), if it
    /// is drawn.
    pub fn key_point(&self, r: Rect, note: u8) -> Option<(i32, i32)> {
        if !(self.first..=self.last).contains(&note) {
            return None;
        }
        let b = self.board(r);
        let (a, e) = b.span(note);
        let y = if is_black(note) {
            b.keys.y + b.keys.h / 4
        } else {
            b.keys.bottom() - b.keys.h / 5
        };
        Some(((a + e) / 2, y))
    }

    /// The x of a zone's root dot.
    pub fn root_x(&self, r: Rect, keys: Keys) -> i32 {
        let (a, e) = self.board(r).span(keys.root.clamp(self.first, self.last));
        (a + e) / 2
    }
}

/// Where each key is: white keys share the width, black keys sit on their
/// edges.
struct Board {
    first: u8,
    last: u8,
    whites: Vec<u8>,
    x0: i32,
    kw: i32,
    keys: Rect,
}

pub fn is_black(note: u8) -> bool {
    matches!(note % 12, 1 | 3 | 6 | 8 | 10)
}

impl Board {
    fn new(first: u8, last: u8, keys: Rect) -> Board {
        let whites: Vec<u8> = (first..=last).filter(|n| !is_black(*n)).collect();
        let n = whites.len().max(1) as i32;
        let kw = (keys.w / n).max(1);
        Board {
            first,
            last,
            x0: keys.x + (keys.w - kw * n) / 2,
            kw,
            whites,
            keys,
        }
    }

    fn white_index(&self, note: u8) -> Option<usize> {
        self.whites.iter().position(|w| *w == note)
    }

    /// The left and right edge of a key's column.
    fn span(&self, note: u8) -> (i32, i32) {
        let end = self.x0 + self.kw * self.whites.len() as i32;
        if note < self.first {
            return (self.x0, self.x0);
        }
        if note > self.last {
            return (end, end);
        }
        match self.white_index(note) {
            Some(i) => (
                self.x0 + self.kw * i as i32,
                self.x0 + self.kw * (i as i32 + 1),
            ),
            None => {
                let c = match self.white_index(note + 1) {
                    Some(i) => self.x0 + self.kw * i as i32,
                    None => end,
                };
                (c - self.kw / 3, c + self.kw / 3)
            }
        }
    }

    /// The key at `x`, a black key first where one is (at height `y` on
    /// the keys; above them, any black key's column counts), clamped to
    /// the keys drawn.
    fn note_at(&self, x: i32, y: i32) -> u8 {
        let black_height = y < self.keys.y + self.keys.h * 3 / 5;
        if black_height {
            if let Some(n) = (self.first..=self.last).find(|&n| {
                let (a, b) = self.span(n);
                is_black(n) && x >= a && x < b
            }) {
                return n;
            }
        }
        let i = ((x - self.x0).div_euclid(self.kw)).clamp(0, self.whites.len() as i32 - 1);
        self.whites.get(i as usize).copied().unwrap_or(self.first)
    }
}

impl Widget for KeyZones {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.titled_panel(r, &self.title, &self.right);
        let (band, keys, pads, caption) = KeyZones::areas(r);
        let b = self.board(r);
        let small = t.size_small - 1.0;
        let picked = self.picked_index();
        // The other zones first, then the one picked over them.
        let mut order: Vec<usize> = (0..self.zones.len())
            .filter(|&i| Some(i) != picked)
            .collect();
        order.extend(picked);
        for i in order {
            let (Some(zr), z) = (self.zone_rect(r, i), &self.zones[i]) else {
                continue;
            };
            let on = Some(i) == picked;
            let fill = z.color.mix(t.surface, if on { 0.35 } else { 0.7 });
            cx.canvas.fill_round_rect(zr, 6, fill);
            if z.fade > 0 {
                let (k, f) = (z.keys, z.fade);
                if k.lo > 0 {
                    let to = b.span(k.lo.saturating_add(f).min(127)).0;
                    fade_cut(cx, zr, b.span(k.lo).0, to, true, t.surface);
                }
                if k.hi < 127 {
                    let from = b.span(k.hi.saturating_sub(f)).1;
                    fade_cut(cx, zr, from, b.span(k.hi).1, false, t.surface);
                }
            }
            if on {
                cx.canvas.stroke_round_rect(zr, 6, 2, z.color);
                // Handles on both edges.
                for x in [zr.x, zr.right() - 4] {
                    cx.canvas.fill_round_rect(
                        Rect::new(x, zr.y + 3, 4, (zr.h - 6).max(2)),
                        2,
                        t.text,
                    );
                }
            }
            let rx = self.root_x(r, z.keys);
            if (zr.x..=zr.right()).contains(&rx) || on {
                let d = if on { 10 } else { 6 };
                let dot = Rect::new(rx - d / 2, zr.y + (zr.h - d) / 2, d, d);
                cx.canvas
                    .fill_round_rect(dot, d / 2, if on { t.text } else { t.text_dim });
            }
            let label = if z.keys.lo < self.first && z.keys.hi > self.last {
                format!("← {} →", z.label)
            } else if z.keys.lo < self.first {
                format!("← {}", z.label)
            } else if z.keys.hi > self.last {
                format!("{} →", z.label)
            } else {
                z.label.clone()
            };
            let need = cx.measure(FontId::Bold, small, &label) + 12;
            if zr.w >= need {
                let lr = Rect::new(zr.x + 8, zr.y, zr.w - 16, zr.h);
                let align = if rx - zr.x > zr.w / 2 {
                    Align::Left
                } else {
                    Align::Right
                };
                cx.text(FontId::Bold, small, lr, align, t.text, &label);
            }
        }
        if self.zones.is_empty() {
            cx.text(
                FontId::Sans,
                t.size_small,
                band,
                Align::Center,
                t.text_dim,
                "No keygroups yet",
            );
        }
        // The keys: roots tinted, held keys lit.
        let roots: Vec<u8> = self.zones.iter().map(|z| z.keys.root).collect();
        let lit = t.accent;
        for (i, &n) in b.whites.iter().enumerate() {
            let kr = Rect::new(b.x0 + b.kw * i as i32 + 1, keys.y, b.kw - 2, keys.h);
            let color = if self.held.contains(&n) {
                lit
            } else if roots.contains(&n) {
                t.text.mix(t.accent, 0.35)
            } else {
                t.text.mix(t.bg, 0.25)
            };
            cx.canvas.fill_round_rect(kr, 3, color);
            if n % 12 == 0 && b.kw >= 14 {
                let lr = Rect::new(kr.x - 6, kr.bottom() - 20, kr.w + 12, 18);
                cx.text(
                    FontId::Sans,
                    small - 1.0,
                    lr,
                    Align::Center,
                    t.bg,
                    &note_name(n),
                );
            }
        }
        for n in self.first..=self.last {
            if !is_black(n) {
                continue;
            }
            let (a, e) = b.span(n);
            let kr = Rect::new(a, keys.y, e - a, keys.h * 3 / 5);
            let color = if self.held.contains(&n) {
                lit
            } else if roots.contains(&n) {
                t.bg.mix(t.accent, 0.7)
            } else {
                t.bg
            };
            cx.canvas.fill_round_rect(kr, 3, color);
        }
        // The 16 pads' notes, under the keys.
        if let Some(from) = self.pads_from {
            let to = from.saturating_add(15).min(127);
            if to >= self.first && from <= self.last {
                let (a, _) = b.span(from.max(self.first));
                let (_, e) = b.span(to.min(self.last));
                let pr = Rect::new(a, pads.y + 6, e - a, 4);
                cx.canvas.fill_round_rect(pr, 2, t.accent);
                let lr = Rect::new(a, pads.y + 4, (e - a).max(0), pads.h);
                let text = format!("pads 1–16: {}–{}", note_name(from), note_name(to));
                let need = cx.measure(FontId::Sans, small - 1.0, &text) + 8;
                let lr = if lr.w >= need {
                    Rect::new(lr.x, lr.y + 4, lr.w, lr.h - 4)
                } else {
                    Rect::new(e + 6, pads.y, (pads.right() - e - 6).max(0), pads.h)
                };
                cx.text(
                    FontId::Sans,
                    small - 1.0,
                    lr,
                    Align::Left,
                    t.text_dim,
                    &text,
                );
            }
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

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let (band, keys, ..) = KeyZones::areas(r);
        match phase {
            Touch::Down => {
                if keys.inset_xy(0, -4).contains(x, y) {
                    let note = self.board(r).note_at(x, y);
                    self.grab = Some((finger, Grab::Key(note)));
                    if !self.held.contains(&note) {
                        self.held.push(note);
                    }
                    return Some(EventKind::Pad {
                        index: usize::from(note),
                        down: true,
                    });
                }
                if !band.inset_xy(-HANDLE, -6).contains(x, y) {
                    return None;
                }
                // The picked zone's handles first, then any zone (the
                // picked one on top).
                if let Some(i) = self.picked_index() {
                    if let Some(zr) = self.zone_rect(r, i) {
                        let k = self.zones[i].keys;
                        if zr.inset_xy(-HANDLE, -6).contains(x, y) {
                            let rx = self.root_x(r, k);
                            let handle = if (x - rx).abs() <= HANDLE / 2 + 2 {
                                Some(2)
                            } else if (x - zr.x).abs() <= HANDLE && k.lo >= self.first {
                                Some(0)
                            } else if (x - zr.right()).abs() <= HANDLE && k.hi <= self.last {
                                Some(1)
                            } else if (x - rx).abs() <= HANDLE {
                                Some(2)
                            } else {
                                None
                            };
                            if let Some(h) = handle {
                                self.grab = Some((finger, Grab::Handle(h)));
                            }
                            return None;
                        }
                    }
                }
                let hit = (0..self.zones.len())
                    .rev()
                    .find(|&i| self.zone_rect(r, i).is_some_and(|zr| zr.contains(x, y)))?;
                let pad = self.zones[hit].pad;
                self.picked = Some(pad);
                Some(EventKind::Select(pad))
            }
            Touch::Move => {
                let (g, grab) = self.grab?;
                if g != finger {
                    return None;
                }
                let Grab::Handle(h) = grab else {
                    return None;
                };
                let i = self.picked_index()?;
                let note = self.board(r).note_at(x, keys.y);
                let k = &mut self.zones[i].keys;
                let note = match h {
                    0 => {
                        k.lo = note.min(k.hi);
                        k.lo
                    }
                    1 => {
                        k.hi = note.max(k.lo);
                        k.hi
                    }
                    _ => {
                        k.root = note;
                        k.root
                    }
                };
                Some(EventKind::Marker {
                    index: h,
                    frame: u64::from(note),
                })
            }
            Touch::Up | Touch::Cancel => {
                let (g, grab) = self.grab?;
                if g != finger {
                    return None;
                }
                self.grab = None;
                match grab {
                    Grab::Key(note) => {
                        self.held.retain(|&n| n != note);
                        Some(EventKind::Pad {
                            index: usize::from(note),
                            down: false,
                        })
                    }
                    Grab::Handle(_) => None,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_are_found_under_the_finger_black_keys_first() {
        let keys = Rect::new(0, 100, 700, 100);
        // C4 to B4 and C5 to B5: 14 white keys, 50 px each.
        let b = Board::new(60, 83, keys);
        assert_eq!(b.kw, 50);
        assert_eq!(b.note_at(10, 190), 60);
        assert_eq!(b.note_at(60, 190), 62);
        // On the edge between C and D, high up: C#.
        assert_eq!(b.note_at(50, 110), 61);
        // Low down there is only the white key.
        assert_eq!(b.note_at(50, 190), 62);
        assert_eq!(b.note_at(699, 190), 83);
        assert_eq!(b.note_at(-30, 190), 60);
        assert_eq!(b.span(61), (34, 66));
    }

    #[test]
    fn overlapping_zones_take_lanes_of_their_own() {
        let mut z = KeyZones::new("Keys");
        let zone = |pad, lo, hi| Zone {
            pad,
            keys: Keys { lo, hi, root: lo },
            fade: 0,
            label: String::new(),
            color: Color::rgb(0),
        };
        z.zones = vec![
            zone(0, 0, 59),
            zone(1, 60, 127),
            zone(2, 48, 72),
            zone(3, 60, 64),
        ];
        assert_eq!(z.lanes(), (vec![0, 0, 1, 2], 3));
    }
}
