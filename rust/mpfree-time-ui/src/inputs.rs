//! An audio track's input list (the audio tracks rethink's phase 4, board
//! 4; layout spec 4c, The input): one list for the track's loops and its
//! takes, over the Clips grid under the track's column head, or over the
//! Song view under its lane. **Line** (L, R, L+R mono, Stereo); **Pads**
//! (the instrument the pads play, after its effects and fader, with its
//! clips); **Mix › a strip** (after its fader); **Mix › Master** (before
//! the master's effects: all you hear but this track). An input the mixer
//! refuses is dim, and the line under its group says why. The app works
//! out what the mixer offers and refuses ([`InputChoices`]); a tap on an
//! input gives `Select` with its index ([`LINE`] + k, [`PADS`],
//! [`MASTER`], [`STRIP`] + i), dim or not, and the app picks it or says
//! why it can't. A tap anywhere else on the list does nothing.

use std::cell::RefCell;

use mpfree_audiotracks::Input;
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Target, Touch, Widget};

/// The list's widget name: a float over the Clips and Song pages.
pub const INPUT_LIST: &str = "input-list";

/// `Select` indices: the line input's four picks (in [`LINES`]' order),
/// the pads, the master, then the strips.
pub const LINE: usize = 0;
pub const PADS: usize = 4;
pub const MASTER: usize = 5;
pub const STRIP: usize = 8;

/// The line input's picks, as the list names them, in their order.
pub const LINES: [(&str, Input); 4] = [
    ("L", Input::Left),
    ("R", Input::Right),
    ("L+R (mono)", Input::Sum),
    ("Stereo", Input::Stereo),
];

/// One input the mixer offers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Choice {
    /// "Bass", "Bus A", "Pads · Kit 1".
    pub name: String,
    /// The project track whose instrument plays into it (its name takes
    /// that track's colour); None for a submix or the master.
    pub owner: Option<usize>,
    /// The colour its name is drawn in (the app fills it from `owner`);
    /// None: the text colour.
    pub color: Option<Color>,
    /// Why the mixer refuses it, said when it is tapped ("Vocal plays
    /// into Bus A, so it would record itself"); None when it can be
    /// picked.
    pub refused: Option<String>,
}

/// What the list offers track `track`, and what it records from now.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InputChoices {
    /// The audio track: its engine track, as the actions name it.
    pub track: usize,
    /// Its name ("Vocal").
    pub name: String,
    /// The line input it records, while it records the line input.
    pub line: Option<Input>,
    /// What it records inside the mix: [`PADS`], [`MASTER`] or [`STRIP`]
    /// + i; None while it records the line input.
    pub picked: Option<usize>,
    /// The instrument the pads play ("Pads · Kit 1"); None when they play
    /// nothing a track can record (MIDI out).
    pub pads: Option<Choice>,
    /// Under the pads, why they can't be had.
    pub pads_why: String,
    /// Every strip but the track's own, the pads' instrument's and the
    /// master, in the mixer's order.
    pub strips: Vec<Choice>,
    /// Under the strips: why those that are dim are ("Bus A is dim: Vocal
    /// plays into it. Pick a strip Vocal doesn't feed.").
    pub strips_why: String,
    pub master: Choice,
    /// Beside Master, why it is dim.
    pub master_why: String,
}

/// The list, as a widget: see the module notes.
#[derive(Default)]
pub struct InputList {
    pub choices: InputChoices,
    hits: RefCell<Vec<(usize, Rect)>>,
}

const PAD_X: i32 = 18;
const PAD_Y: i32 = 16;
const TITLE_H: i32 = 18;
const GROUP_GAP: i32 = 14;
const GROUP_H: i32 = 15;
const UNDER: i32 = 6;
const BUTTON_H: i32 = 40;
const PADS_H: i32 = 46;
const GAP: i32 = 8;
const WHY_H: i32 = 15;
const MASTER_W: i32 = 132;
/// The list's width, where the screen has room.
pub const LIST_W: i32 = 700;

impl InputList {
    /// The strips' columns and rows: five across, more when they would
    /// take more than three rows.
    fn strip_grid(&self) -> (usize, usize) {
        let n = self.choices.strips.len();
        let cols = n.div_ceil(3).clamp(5, 8);
        (cols, n.div_ceil(cols))
    }

    /// How tall the list is.
    pub fn height(&self) -> i32 {
        let (_, rows) = self.strip_grid();
        let rows = rows as i32;
        let strips = if rows > 0 {
            rows * (BUTTON_H + GAP) - GAP
        } else {
            WHY_H
        };
        let why = |s: &str| if s.is_empty() { 0 } else { UNDER + WHY_H };
        PAD_Y
            + TITLE_H
            + 4 * (GROUP_GAP + GROUP_H + UNDER)
            + BUTTON_H
            + PADS_H
            + why(&self.choices.pads_why)
            + strips
            + why(&self.choices.strips_why)
            + BUTTON_H
            + PAD_Y
    }

    /// Where the list goes in `area` (the grid's or the lanes' rectangle)
    /// for the column head or lane header `head`: under it, centred in
    /// the area, as wide as it has room for, and moved up when it would
    /// reach past the area's foot.
    pub fn place(&self, area: Rect, head: Rect) -> Rect {
        let w = LIST_W.min(area.w - 24).max(0);
        let h = self.height().min(area.h - 16).max(0);
        let x = area.x + (area.w - w) / 2;
        let y = (head.bottom() + 12)
            .min(area.bottom() - 8 - h)
            .max(area.y + 8);
        Rect::new(x, y, w, h)
    }

    /// Every input's rectangle in `r`, by its `Select` index, and where
    /// each group's words go: (rects, groups' label lines, why lines).
    fn layout(&self, r: Rect) -> Layout {
        let c = &self.choices;
        let inner = r.inset_xy(PAD_X, 0);
        let mut y = r.y + PAD_Y;
        let title = Rect::new(inner.x, y, inner.w, TITLE_H);
        y += TITLE_H;
        let mut groups = Vec::new();
        let mut whys = Vec::new();
        let mut hits = Vec::new();
        let group = |y: &mut i32, groups: &mut Vec<Rect>| {
            *y += GROUP_GAP;
            groups.push(Rect::new(inner.x, *y, inner.w, GROUP_H));
            *y += GROUP_H + UNDER;
        };
        // Line: four across.
        group(&mut y, &mut groups);
        let w4 = (inner.w - 3 * GAP) / 4;
        for k in 0..LINES.len() {
            let x = inner.x + k as i32 * (w4 + GAP);
            hits.push((LINE + k, Rect::new(x, y, w4, BUTTON_H)));
        }
        y += BUTTON_H;
        // Pads: one wide button.
        group(&mut y, &mut groups);
        hits.push((PADS, Rect::new(inner.x, y, inner.w, PADS_H)));
        y += PADS_H;
        if !c.pads_why.is_empty() {
            y += UNDER;
            whys.push(Rect::new(inner.x, y, inner.w, WHY_H));
            y += WHY_H;
        }
        // The strips, five across (more when there are many).
        group(&mut y, &mut groups);
        let (cols, rows) = self.strip_grid();
        let cw = (inner.w - (cols as i32 - 1) * GAP) / cols as i32;
        for i in 0..c.strips.len() {
            let (col, row) = ((i % cols) as i32, (i / cols) as i32);
            let b = Rect::new(
                inner.x + col * (cw + GAP),
                y + row * (BUTTON_H + GAP),
                cw,
                BUTTON_H,
            );
            hits.push((STRIP + i, b));
        }
        y += if rows > 0 {
            rows as i32 * (BUTTON_H + GAP) - GAP
        } else {
            WHY_H
        };
        if !c.strips_why.is_empty() {
            y += UNDER;
            whys.push(Rect::new(inner.x, y, inner.w, WHY_H));
            y += WHY_H;
        }
        // The master, its words beside it.
        group(&mut y, &mut groups);
        let master = Rect::new(inner.x, y, MASTER_W, BUTTON_H);
        hits.push((MASTER, master));
        whys.push(Rect::new(
            master.right() + 12,
            y - 2,
            inner.right() - master.right() - 12,
            BUTTON_H + 4,
        ));
        Layout {
            title,
            groups,
            whys,
            hits,
        }
    }

    /// The choice behind `Select` index `k`, and whether it is picked now:
    /// None for a line pick or an index past the list.
    pub fn choice(&self, k: usize) -> Option<&Choice> {
        let c = &self.choices;
        match k {
            PADS => c.pads.as_ref(),
            MASTER => Some(&c.master),
            k if k >= STRIP => c.strips.get(k - STRIP),
            _ => None,
        }
    }

    /// Why input `k` can't be picked, as the screen says it; None when it
    /// can.
    pub fn refused(&self, k: usize) -> Option<String> {
        if k == PADS && self.choices.pads.is_none() {
            return Some(if self.choices.pads_why.is_empty() {
                "the pads play nothing a track can record".to_string()
            } else {
                self.choices.pads_why.clone()
            });
        }
        self.choice(k).and_then(|c| c.refused.clone())
    }

    fn picked(&self, k: usize) -> bool {
        let c = &self.choices;
        match k {
            k if k < PADS => c.line == Some(LINES[k].1),
            k => c.picked == Some(k),
        }
    }
}

struct Layout {
    title: Rect,
    groups: Vec<Rect>,
    whys: Vec<Rect>,
    hits: Vec<(usize, Rect)>,
}

/// `text` in lines no wider than `w` at `px`, broken between words.
fn wrap(cx: &mut PaintCx, text: &str, px: f32, w: i32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split(' ') {
        match lines.last_mut() {
            Some(l) if cx.measure(FontId::Sans, px, &format!("{l} {word}")) <= w => {
                l.push(' ');
                l.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    lines
}

impl Widget for InputList {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let c = &self.choices;
        cx.canvas.fill_rect(r, t.bg);
        cx.canvas.fill_round_rect(r, t.round(12), t.panel());
        cx.canvas.stroke_round_rect(r, t.round(12), 2, t.good);
        let l = self.layout(r);
        let upper = c.name.to_uppercase();
        let note = "latency made up per input";
        let note_w = cx.measure(FontId::Sans, 12.0, note);
        cx.text(
            FontId::Sans,
            12.0,
            l.title,
            Align::Right,
            t.text_faint,
            note,
        );
        let title = Rect::new(l.title.x, l.title.y, l.title.w - note_w - 12, l.title.h);
        cx.text(
            FontId::Bold,
            13.0,
            title,
            Align::Left,
            t.text_dim,
            &format!("{upper}'S INPUT \u{b7} FOR ITS LOOPS AND ITS TAKES"),
        );
        let labels = [
            "LINE \u{b7} THE INPUTS ON THE BACK".to_string(),
            "PADS \u{b7} THE INSTRUMENT THE PADS PLAY, AFTER ITS EFFECTS AND FADER".to_string(),
            "MIX \u{203a} A STRIP, AFTER ITS FADER".to_string(),
            format!("MIX \u{203a} MASTER, BEFORE ITS EFFECTS: ALL YOU HEAR BUT {upper}"),
        ];
        for (g, label) in l.groups.iter().zip(&labels) {
            cx.text(FontId::Bold, 12.0, *g, Align::Left, t.text_dim, label);
        }
        for &(k, b) in &l.hits {
            let picked = self.picked(k);
            let dim = self.refused(k).is_some();
            let fill = if picked { t.accent } else { t.raised };
            let fill = if dim { fill.mix(t.panel(), 0.55) } else { fill };
            cx.canvas.fill_round_rect(b, t.round(8), fill);
            let ink = if picked {
                t.ink_on(t.accent)
            } else {
                self.choice(k)
                    .and_then(|ch| ch.color)
                    .map_or(t.text, |col| t.readable(col, t.raised))
            };
            let ink = if dim { ink.mix(t.panel(), 0.6) } else { ink };
            if k < PADS {
                cx.text(FontId::Bold, 14.0, b, Align::Center, ink, LINES[k].0);
                continue;
            }
            if k == PADS {
                let name = c.pads.as_ref().map_or("Pads", |p| p.name.as_str());
                let inner = b.inset_xy(14, 0);
                let name_w = cx.measure(FontId::Bold, 14.0, name);
                cx.text(FontId::Bold, 14.0, inner, Align::Left, ink, name);
                if let Some(p) = &c.pads {
                    let of = p.name.split(" \u{b7} ").nth(1).unwrap_or("it");
                    let sub =
                        format!("\u{b7} with its clips: a clip playing on {of} is recorded too");
                    let rest = Rect::new(inner.x + name_w + 6, b.y, inner.w - name_w - 6, b.h);
                    let sub_ink = if picked { ink } else { t.text_dim };
                    cx.text(FontId::Sans, 13.0, rest, Align::Left, sub_ink, &sub);
                }
                continue;
            }
            let name = self.choice(k).map_or("", |ch| ch.name.as_str());
            cx.text(
                FontId::Bold,
                14.0,
                b.inset_xy(6, 0),
                Align::Center,
                ink,
                name,
            );
        }
        let mut whys: Vec<&str> = Vec::new();
        if !c.pads_why.is_empty() {
            whys.push(&c.pads_why);
        }
        if !c.strips_why.is_empty() {
            whys.push(&c.strips_why);
        }
        for (w, text) in l.whys.iter().zip(&whys) {
            cx.text(FontId::Sans, 12.0, *w, Align::Left, t.warn, text);
        }
        // Beside Master: why it is dim, in two lines at most.
        if let Some(w) = l.whys.last().filter(|_| !c.master_why.is_empty()) {
            let lines = wrap(cx, &c.master_why, 12.0, w.w);
            let n = lines.len().min(2) as i32;
            let top = w.y + (w.h - n * 16) / 2;
            for (i, line) in lines.iter().take(2).enumerate() {
                let at = Rect::new(w.x, top + i as i32 * 16, w.w, 16);
                cx.text(FontId::Sans, 12.0, at, Align::Left, t.warn, line);
            }
        }
        *self.hits.borrow_mut() = l.hits;
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        self.layout(r)
            .hits
            .into_iter()
            .map(|(k, b)| {
                let label = match k {
                    k if k < PADS => format!("Line {}", LINES[k].0),
                    k => self
                        .choice(k)
                        .map_or("Pads", |c| c.name.as_str())
                        .to_string(),
                };
                Target::tap("input", k, label, b)
            })
            .collect()
    }

    fn touch(&mut self, _finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Up {
            return None;
        }
        let l = self.layout(r);
        l.hits
            .iter()
            .find(|(_, b)| b.contains(x, y))
            .map(|&(k, _)| EventKind::Select(k))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> InputList {
        let strip = |name: &str, owner: Option<usize>, refused: Option<&str>| Choice {
            name: name.to_string(),
            owner,
            color: None,
            refused: refused.map(str::to_string),
        };
        InputList {
            choices: InputChoices {
                track: 2,
                name: "Vocal".to_string(),
                line: Some(Input::Left),
                picked: None,
                pads: Some(strip("Pads \u{b7} Kit 1", Some(0), None)),
                pads_why: String::new(),
                strips: vec![
                    strip("Bass", Some(1), None),
                    strip("Keys", Some(3), None),
                    strip("Bus A", None, Some("Vocal plays into Bus A")),
                ],
                strips_why: "Bus A is dim: Vocal plays into it. Pick a strip Vocal doesn't feed."
                    .to_string(),
                master: strip(
                    "Master",
                    None,
                    Some("Vocal plays into Master through Bus A"),
                ),
                master_why: "Vocal plays into Master through Bus A: pick another input, or \
                             send Vocal straight to Master."
                    .to_string(),
            },
            hits: RefCell::default(),
        }
    }

    #[test]
    fn each_input_is_a_target_and_a_tap_names_it() {
        let mut l = list();
        let area = Rect::new(20, 110, 880, 600);
        let head = Rect::new(400, 150, 100, 44);
        let r = l.place(area, head);
        // Under the head, inside the area, as wide as the board draws it.
        assert!(r.y >= head.bottom() && r.bottom() <= area.bottom(), "{r:?}");
        assert_eq!(r.w, LIST_W);
        let targets = l.targets(r);
        let keys: Vec<usize> = targets.iter().map(|t| t.index).collect();
        assert_eq!(
            keys,
            vec![0, 1, 2, 3, PADS, STRIP, STRIP + 1, STRIP + 2, MASTER]
        );
        for t in &targets {
            let (x, y) = t.centre();
            assert_eq!(l.touch(0, Touch::Down, x, y, r), None);
            assert_eq!(
                l.touch(0, Touch::Up, x, y, r),
                Some(EventKind::Select(t.index)),
                "{}",
                t.label
            );
        }
        // Between the groups: nothing.
        assert_eq!(l.touch(0, Touch::Up, r.x + 4, r.y + 4, r), None);
        // A dim one says why; a line pick and an open strip don't.
        assert_eq!(
            l.refused(STRIP + 2).as_deref(),
            Some("Vocal plays into Bus A")
        );
        assert!(l.refused(MASTER).is_some());
        assert_eq!(l.refused(STRIP), None);
        assert_eq!(l.refused(1), None);
        assert!(l.picked(0) && !l.picked(1) && !l.picked(PADS));
        // The pads on MIDI out: Pads is dim, and says so.
        l.choices.pads = None;
        l.choices.pads_why = "the pads play MIDI out".to_string();
        assert_eq!(l.refused(PADS).as_deref(), Some("the pads play MIDI out"));
    }

    #[test]
    fn many_strips_go_more_across_and_the_list_fits_its_area() {
        let mut l = list();
        let one = l.choices.strips[0].clone();
        l.choices.strips = vec![one; 20];
        assert_eq!(l.strip_grid(), (7, 3));
        let area = Rect::new(20, 110, 880, 600);
        let r = l.place(area, Rect::new(30, 150, 100, 44));
        assert!(r.bottom() <= area.bottom() && r.y >= area.y, "{r:?}");
        let last = l
            .layout(r)
            .hits
            .iter()
            .map(|h| h.1.bottom())
            .max()
            .unwrap_or(0);
        assert!(last <= r.bottom(), "{last} past {r:?}");
    }
}
