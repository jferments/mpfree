//! The Pads tab's Perform view (the Play, Pads and Sequencer redesign's
//! step 6, board 7): the project's **Perform set**, its eight pages of
//! action pads ([`crate::perform_pads`]), shown and edited where the kit's
//! pads are. The Pads of row's **Perform pads** chip opens it (not
//! "Perform": the top bar's MAKE / PERFORM chip is on the same screen); a
//! track's chip goes back to the kit.
//!
//! - The **bar** names the pad picked and what it does, and its **On the
//!   pads** switch puts the set on the pads (Play and the Looper tab play
//!   it, as SHIFT + SAMPLER pressed again on the Looper tab does) or takes
//!   it off.
//! - The **map** shows the page's 16 pads as on the MPC (pad 1 bottom
//!   left), each with what it does; its strip picks the page (A to H, as
//!   PAD BANK does here). A tap picks a pad; a tap on the pad picked plays
//!   its throw while the finger is down. A hit on the hardware picks the
//!   pad too, and plays a throw while held: nothing else a pad does
//!   happens here, so the set can be set up while a song plays.
//! - **What B2 does**: Throw, Loop key, Loop mute, Track mute or Scene;
//!   under it the eight choices of that kind (Play's throws, Loops 1 to 8,
//!   the tracks, the scenes), and the **Pad** list with its settings (a
//!   throw's effect and Hold) and what a hit does.
//! - **Copy**, **Paste** and **Clear** work on the pad picked. Each edit is
//!   one UNDO step; SHIFT + UNDO redoes. The history is this view's own,
//!   kept until the project changes.
//!
//! The screen keeps a copy of the set (the app sends it at the start, when
//! a project opens and after each edit); an edit goes to the app as
//! [`FromUi::PerformPad`], which saves it with the project.

use crate::gui::FromUi;
use crate::perform_pads::{Hold, PadAction, PerformSet, Throw, PADS, PAGES};
use crate::play_page::THROW_LIST;
use mpfree_sampler_ui::widgets::{
    ButtonRow, KitTracks, PadBar, PadCell, PadMap, SectionChip, SectionChips, Setting, SettingList,
};
use mpfree_ui::{Color, EventKind, Ui, UiEvent};

/// The view's page.
pub const PAGE: &str = "Pads · Perform";
/// Every widget of the view starts with this.
pub const PREFIX: &str = "prf-";
pub const BAR: &str = "prf-bar";
pub const MAP: &str = "prf-map";
pub const BUTTONS: &str = "prf-buttons";
/// What the pad does ([`Kind`]); a tap on chip `i` is [`EventKind::Tab`]`(i)`.
pub const TYPE: &str = "prf-type";
/// The eight choices of the pad's kind, four a row.
pub const PICK_A: &str = "prf-pick-a";
pub const PICK_B: &str = "prf-pick-b";
pub const LIST: &str = "prf-list";
/// The Pads of row, the kit view's ([`mpfree_sampler_ui::pads_tab::KITS`]).
pub const KITS: &str = mpfree_sampler_ui::pads_tab::KITS;

/// The view's page: the pads with their buttons as the kit view has them,
/// and what the pad picked does over the rest of the width.
pub const LAYOUT: &str = "col(header:60, prf-bar:60, smp-pads-kits:52, \
                          row(col(prf-map:*, prf-buttons:64):440, \
                          col(prf-type:64, prf-pick-a:64, prf-pick-b:64, prf-list:*):*):*, \
                          hint:26)";

/// The hint line under the view.
pub const HINT: &str = "Perform pads: one set for the project   ·   A-H: its pages (A is the looper's loop keys)   ·   \
                        a type: what the pad does   ·   hit a throw to hear it   ·   no kit changes";

/// The buttons under the map.
const COPY: usize = 0;
const PASTE: usize = 1;
const CLEAR: usize = 2;

/// What a Perform pad can do, in the type chips' order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Throw,
    LoopKey,
    LoopMute,
    TrackMute,
    Scene,
}

impl Kind {
    pub const ALL: [Kind; 5] = [
        Kind::Throw,
        Kind::LoopKey,
        Kind::LoopMute,
        Kind::TrackMute,
        Kind::Scene,
    ];

    /// Its chip's name.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Throw => "Throw",
            Kind::LoopKey => "Loop key",
            Kind::LoopMute => "Loop mute",
            Kind::TrackMute => "Track mute",
            Kind::Scene => "Scene",
        }
    }

    /// The line under its name.
    fn sub(self) -> &'static str {
        match self {
            Kind::Throw => "an effect",
            Kind::LoopKey => "a loop's steps",
            Kind::LoopMute => "a loop's mute",
            Kind::TrackMute => "a track's mute",
            Kind::Scene => "a row launched",
        }
    }

    /// What `a` is; None for nothing.
    pub fn of(a: PadAction) -> Option<Kind> {
        Some(match a {
            PadAction::None => return None,
            PadAction::Throw(_) => Kind::Throw,
            PadAction::LoopKey(_) => Kind::LoopKey,
            PadAction::LoopMute(_) => Kind::LoopMute,
            PadAction::Mute(_) => Kind::TrackMute,
            PadAction::Scene(_) => Kind::Scene,
        })
    }

    /// The pad's action made this kind: a loop's key and its mute swap
    /// keeping the loop; anything else starts at the first choice.
    pub fn action(self, from: PadAction) -> PadAction {
        match (self, from) {
            (k, a) if Kind::of(a) == Some(k) => a,
            (Kind::LoopKey, PadAction::LoopMute(n)) => PadAction::LoopKey(n),
            (Kind::LoopMute, PadAction::LoopKey(n)) => PadAction::LoopMute(n),
            (Kind::Throw, _) => PadAction::Throw(Throw {
                index: 0,
                hold: Hold::WhileHeld,
            }),
            (Kind::LoopKey, _) => PadAction::LoopKey(0),
            (Kind::LoopMute, _) => PadAction::LoopMute(0),
            (Kind::TrackMute, _) => PadAction::Mute(0),
            (Kind::Scene, _) => PadAction::Scene(0),
        }
    }
}

/// The project's tracks and scenes as the view names them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Names {
    /// Each track's name and what it plays ("Kit", "FM · Glass Keys").
    pub tracks: Vec<(String, String)>,
    /// Each scene row's name, as far as the project names them.
    pub scenes: Vec<String>,
}

impl Names {
    fn track(&self, t: usize) -> String {
        self.tracks
            .get(t)
            .map_or_else(|| format!("Track {}", t + 1), |(n, _)| n.clone())
    }

    fn scene(&self, n: usize) -> String {
        match self.scenes.get(n).filter(|s| !s.is_empty()) {
            Some(s) => format!("Scene {} · {s}", n + 1),
            None => format!("Scene {}", n + 1),
        }
    }
}

/// Scene rows a pad can launch: the grid's rows.
const SCENES: usize = mpfree_seq::scenes::SCENES;
/// Loops a key can be.
const LOOPS: usize = mpfree_looper::MAX_LOOPS;

/// One edit, for UNDO.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Edit {
    page: usize,
    pad: usize,
    before: PadAction,
    after: PadAction,
}

/// What the view asks of the screen.
#[derive(Debug, PartialEq)]
pub enum Act {
    /// For the app.
    Send(FromUi),
    /// Play's throw `i` (in [`THROW_LIST`]) held or let go: a finger on
    /// the map's pad.
    Hear(usize, bool),
    /// A line for the hint and the log.
    Tell(String),
}

/// The Perform view's state.
#[derive(Debug, Default)]
pub struct PerformTab {
    /// The project's set, as the app last sent it.
    pub set: PerformSet,
    /// The page shown: the app's PAD BANK page for the set.
    pub page: usize,
    /// The pad picked (0 is pad 1).
    pub pad: usize,
    /// The set is on the pads (Play and the Looper tab play it).
    pub on: bool,
    /// The Pads tab shows the set, not the kit.
    pub shown: bool,
    built: bool,
    copied: Option<PadAction>,
    lit: [bool; PADS],
    /// The throw a finger on the map's pad plays.
    hearing: Option<usize>,
    done: Vec<Edit>,
    undone: Vec<Edit>,
}

/// Pad `pad` of page `page` by name ("B2").
pub fn pad_label(page: usize, pad: usize) -> String {
    format!("{}{}", (b'A' + (page % PAGES) as u8) as char, pad + 1)
}

/// What `a` is called on its pad ("Reverb wash", "Loop 2", "Drums").
pub fn action_name(a: PadAction, names: &Names) -> String {
    match a {
        PadAction::None => String::new(),
        PadAction::Throw(t) => t.name().to_string(),
        PadAction::LoopKey(n) | PadAction::LoopMute(n) => format!("Loop {}", n + 1),
        PadAction::Mute(t) => names.track(usize::from(t)),
        PadAction::Scene(n) => names.scene(usize::from(n)),
    }
}

/// A throw's Hold in words.
fn hold_text(h: Hold) -> &'static str {
    match h {
        Hold::WhileHeld => "While held",
        Hold::Latch => "Latch",
    }
}

/// What `a` does, in a few words (the bar's line, the log).
pub fn action_summary(a: PadAction, names: &Names) -> String {
    match a {
        PadAction::None => "Nothing on it".to_string(),
        PadAction::Throw(t) => match t.hold {
            Hold::WhileHeld => "Throw · while held, on the master".to_string(),
            Hold::Latch => "Throw · latched, on the master".to_string(),
        },
        PadAction::LoopKey(n) => format!("Loop key · the looper's Loop {}", n + 1),
        PadAction::LoopMute(n) => format!("Loop mute · the looper's Loop {}", n + 1),
        PadAction::Mute(t) => format!("Track mute · {}", names.track(usize::from(t))),
        PadAction::Scene(n) => format!("Scene · launches row {}", u16::from(n) + 1),
    }
}

/// What a hit on a pad doing `a` does, for under the Pad list.
pub fn hit_text(label: &str, a: PadAction, names: &Names) -> String {
    match a {
        PadAction::None => format!("Nothing yet: pick what {label} does above."),
        PadAction::Throw(t) => match t.hold {
            Hold::WhileHeld => format!(
                "Held, it plays {} on the master; let go, it rings out. Play's throw cards \
                 play the same effects.",
                t.name()
            ),
            Hold::Latch => format!(
                "A hit starts {} on the master and the next lets it go (it rings out).",
                t.name()
            ),
        },
        PadAction::LoopKey(n) => format!(
            "A hit is Loop {}'s next step, as a tap on its ring: record from the next bar \
             line, end the first pass, overdub, play. Held for a second, it clears the loop.",
            n + 1
        ),
        PadAction::LoopMute(n) => format!(
            "A hit mutes or unmutes Loop {}; with SHIFT held it solos it.",
            n + 1
        ),
        PadAction::Mute(t) => format!(
            "A hit mutes or unmutes {} at the launch line in Perform, at once in Make \
             (SHIFT gives the other timing), as TRACK MUTE's pads do.",
            names.track(usize::from(t))
        ),
        PadAction::Scene(n) => format!(
            "A hit launches {}'s row, as its scene button on the grid does.",
            names.scene(usize::from(n))
        ),
    }
}

/// The colour of a pad doing `a`.
fn action_color(a: PadAction) -> Color {
    match a {
        PadAction::None => Color::rgb(0x8A93A6),
        PadAction::Throw(t) => THROW_LIST
            .get(usize::from(t.index))
            .map_or(mpfree_ui::draw::FX, |d| d.color),
        PadAction::LoopKey(n) | PadAction::LoopMute(n) => {
            let c = crate::looper_tab::LOOP_COLOURS;
            c[usize::from(n) % c.len()]
        }
        PadAction::Mute(_) => mpfree_ui::draw::FX,
        PadAction::Scene(_) => mpfree_ui::draw::KNOB[1],
    }
}

/// A pad's cell on the map.
fn cell(page: usize, pad: usize, a: PadAction, names: &Names) -> PadCell {
    let (detail, badge) = match a {
        PadAction::None => (String::new(), None),
        PadAction::Throw(t) => (
            match t.hold {
                Hold::WhileHeld => "master".to_string(),
                Hold::Latch => "latch · master".to_string(),
            },
            Some("THROW"),
        ),
        PadAction::LoopKey(_) => ("its key".to_string(), Some("LOOP")),
        PadAction::LoopMute(_) => ("its mute".to_string(), Some("LOOP MUTE")),
        PadAction::Mute(_) => ("track mute".to_string(), Some("MUTE")),
        PadAction::Scene(_) => ("launches it".to_string(), Some("SCENE")),
    };
    PadCell {
        label: pad_label(page, pad),
        name: action_name(a, names),
        detail,
        clip: None,
        color: action_color(a),
        badge: badge.map(str::to_string),
    }
}

/// The page's pads with an action, pad 1 the lowest bit.
fn used(set: &PerformSet, page: usize) -> u16 {
    set.page(page)
        .pads
        .iter()
        .enumerate()
        .filter(|(_, a)| **a != PadAction::None)
        .fold(0, |m, (i, _)| m | 1 << i)
}

/// The first of the eight choices shown for `n` of `len` (a window of
/// eight holding it).
fn window(n: usize, len: usize) -> usize {
    let first = (n / 8) * 8;
    first.min(len.saturating_sub(1) / 8 * 8)
}

impl PerformTab {
    /// The page's action pads and their settings in `ui`, once.
    pub fn build(&mut self, ui: &mut Ui) -> Result<(), String> {
        if self.built {
            return Ok(());
        }
        let mut bar = PadBar::new(&[]);
        bar.button = Some("On the pads: off".into());
        bar.switch = Some(false);
        ui.add(BAR, bar);
        // The Pads of row is the kit view's; the app fills it.
        if ui.get::<KitTracks>(KITS).is_none() {
            ui.add(KITS, KitTracks::new());
        }
        ui.add(MAP, PadMap::new("Perform set"));
        ui.add(BUTTONS, ButtonRow::new(&["Copy", "Paste", "Clear"]));
        ui.add(TYPE, SectionChips::new(&Kind::ALL.map(Kind::label)));
        ui.add(PICK_A, SectionChips::new(&[]));
        ui.add(PICK_B, SectionChips::new(&[]));
        ui.add(LIST, SettingList::new("Pad"));
        ui.add_page(PAGE, LAYOUT)?;
        self.built = true;
        Ok(())
    }

    /// The set as the app has it now: after an edit, or a project's
    /// (`opened`: a project opened or New, whose set has a history of its
    /// own).
    pub fn set_set(&mut self, set: PerformSet, opened: bool) {
        if opened {
            self.done.clear();
            self.undone.clear();
        }
        self.set = set;
    }

    /// UNDO has an edit to take back.
    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    /// The action of the pad picked.
    pub fn picked(&self) -> PadAction {
        self.set.page(self.page).action(self.pad)
    }

    /// The Pad list's rows for `a`.
    fn rows(a: PadAction, names: &Names) -> Vec<Setting> {
        let row = |name: &str, value: String| Setting {
            name: name.to_string(),
            value,
        };
        let mut rows = vec![row(
            "Pad does",
            Kind::of(a).map_or("Nothing", Kind::label).to_string(),
        )];
        match a {
            PadAction::None => {}
            PadAction::Throw(t) => {
                rows.push(row("Effect", t.name().to_string()));
                rows.push(row("Hold", hold_text(t.hold).to_string()));
            }
            PadAction::LoopKey(n) | PadAction::LoopMute(n) => {
                rows.push(row("Loop", format!("Loop {}", n + 1)));
            }
            PadAction::Mute(t) => {
                rows.push(row("Track", names.track(usize::from(t))));
            }
            PadAction::Scene(n) => {
                rows.push(row("Scene", names.scene(usize::from(n))));
            }
        }
        rows
    }

    /// The eight choices of `a`'s kind shown (four a row), the first's
    /// place, and the one `a` is.
    fn choices(a: PadAction, names: &Names) -> (Vec<SectionChip>, usize, Option<usize>) {
        let chip = |name: String, sub: String| SectionChip { name, sub };
        match a {
            PadAction::None => (Vec::new(), 0, None),
            PadAction::Throw(t) => (
                THROW_LIST
                    .iter()
                    .map(|d| chip(d.name.to_string(), d.sub.to_string()))
                    .collect(),
                0,
                Some(usize::from(t.index)),
            ),
            PadAction::LoopKey(n) | PadAction::LoopMute(n) => (
                (0..LOOPS)
                    .map(|k| chip(format!("Loop {}", k + 1), String::new()))
                    .collect(),
                0,
                Some(usize::from(n)),
            ),
            PadAction::Mute(t) => {
                let t = usize::from(t);
                let len = names.tracks.len().max(t + 1);
                let first = window(t, len);
                let chips = (first..len.min(first + 8))
                    .map(|k| {
                        let sound = names.tracks.get(k).map(|(_, s)| s.clone());
                        chip(names.track(k), sound.unwrap_or_default())
                    })
                    .collect();
                (chips, first, Some(t))
            }
            PadAction::Scene(n) => {
                let n = usize::from(n);
                let first = window(n, SCENES);
                let chips = (first..SCENES.min(first + 8))
                    .map(|k| {
                        let name = names.scenes.get(k).cloned().unwrap_or_default();
                        chip(format!("Scene {}", k + 1), name)
                    })
                    .collect();
                (chips, first, Some(n))
            }
        }
    }

    /// Shows the set's page and the pad picked.
    pub fn show(&self, ui: &mut Ui, names: &Names) {
        let page = self.page.min(PAGES - 1);
        let a = self.picked();
        let label = pad_label(page, self.pad);
        let page_name = self.set.page_name(page);
        if let Some(bar) = ui.get_mut::<PadBar>(BAR) {
            bar.color = action_color(a);
            bar.title = match a {
                PadAction::None => format!("Perform pads {label} · nothing"),
                a => format!("Perform pads {label} · {}", action_name(a, names)),
            };
            bar.subtitle = format!(
                "{} · page {} of the project's Perform set",
                action_summary(a, names),
                (b'A' + page as u8) as char
            );
            bar.button = Some(if self.on {
                "On the pads: on".to_string()
            } else {
                "On the pads: off".to_string()
            });
            bar.switch = Some(self.on);
        }
        if let Some(m) = ui.get_mut::<PadMap>(MAP) {
            m.title = "Perform set".to_string();
            m.right = page_name.clone();
            m.cells = (0..PADS)
                .map(|p| cell(page, p, self.set.page(page).action(p), names))
                .collect();
            m.selected = self.pad;
            m.lit = self.lit.to_vec();
            m.banks = (0..PAGES).map(|k| used(&self.set, k)).collect();
            m.bank = page;
        }
        if let Some(b) = ui.get_mut::<ButtonRow>(BUTTONS) {
            b.labels = vec![
                "Copy".to_string(),
                match self.copied {
                    Some(_) => format!("Paste {label}"),
                    None => "Paste".to_string(),
                },
                "Clear".to_string(),
            ];
            b.enabled = vec![
                a != PadAction::None,
                self.copied.is_some(),
                a != PadAction::None,
            ];
            b.why = vec![
                format!("{label} has nothing on it to copy"),
                "Copy a pad first".to_string(),
                format!("{label} has nothing on it"),
            ];
        }
        if let Some(c) = ui.get_mut::<SectionChips>(TYPE) {
            c.chips = Kind::ALL
                .iter()
                .map(|k| SectionChip {
                    name: k.label().to_string(),
                    sub: k.sub().to_string(),
                })
                .collect();
            c.selected = Kind::of(a)
                .and_then(|k| Kind::ALL.iter().position(|&x| x == k))
                .unwrap_or(usize::MAX);
        }
        let (chips, first, now) = Self::choices(a, names);
        let at = now.and_then(|n| n.checked_sub(first));
        let (top, bottom) = chips.split_at(chips.len().min(4));
        for (id, part, offset) in [(PICK_A, top, 0), (PICK_B, bottom, 4)] {
            if let Some(c) = ui.get_mut::<SectionChips>(id) {
                c.chips = part.to_vec();
                c.selected = at
                    .and_then(|i| i.checked_sub(offset))
                    .filter(|&i| i < part.len())
                    .unwrap_or(usize::MAX);
            }
        }
        if let Some(l) = ui.get_mut::<SettingList>(LIST) {
            l.title = format!("Pad {label}");
            l.right = page_name;
            l.rows = Self::rows(a, names);
            l.selected = l.selected.min(l.rows.len().saturating_sub(1));
            let live = format!(
                "Live: On the pads (or SHIFT + SAMPLER on the Looper tab) puts page {} on the \
                 pads on Play and the Looper tab, and PAD BANK turns its pages.",
                (b'A' + page as u8) as char
            );
            l.footer = Some((format!("{} {live}", hit_text(&label, a, names)), false));
        }
    }

    /// Pad `pad` (0 is pad 1) hit on the hardware: picked.
    pub fn hit(&mut self, pad: usize) {
        if pad < PADS {
            self.pad = pad;
            self.lit[pad] = true;
        }
    }

    /// Pad `pad` let go on the hardware.
    pub fn up(&mut self, pad: usize) {
        if let Some(l) = self.lit.get_mut(pad) {
            *l = false;
        }
    }

    /// Pad `pad` of the page shown does `after` now: an UNDO step, the
    /// app told, and a line for the log.
    fn edit(&mut self, pad: usize, after: PadAction, names: &Names) -> Vec<Act> {
        let page = self.page.min(PAGES - 1);
        let before = self.set.page(page).action(pad);
        if before == after {
            return Vec::new();
        }
        self.done.push(Edit {
            page,
            pad,
            before,
            after,
        });
        self.undone.clear();
        self.apply(page, pad, after);
        let label = pad_label(page, pad);
        let line = match after {
            PadAction::None => format!("Perform pads {label} cleared"),
            a => format!("Perform pads {label}: {}", edit_text(a, names)),
        };
        vec![
            Act::Send(FromUi::PerformPad {
                page,
                pad,
                action: after,
            }),
            Act::Tell(line),
        ]
    }

    /// Puts `a` on the copy kept here, as the app will.
    fn apply(&mut self, page: usize, pad: usize, a: PadAction) {
        if let Some(slot) = self
            .set
            .pages
            .get_mut(page)
            .and_then(|p| p.pads.get_mut(pad))
        {
            *slot = a;
        }
    }

    /// UNDO (or SHIFT + UNDO, `redo`): the last edit taken back, or
    /// made again. The page and pad it was on show.
    pub fn undo(&mut self, redo: bool, names: &Names) -> Vec<Act> {
        let (from, to) = if redo {
            (&mut self.undone, &mut self.done)
        } else {
            (&mut self.done, &mut self.undone)
        };
        let Some(e) = from.pop() else {
            return vec![Act::Tell(if redo {
                "Nothing to redo on the Perform set".to_string()
            } else {
                "Nothing to undo on the Perform set".to_string()
            })];
        };
        to.push(e);
        let a = if redo { e.after } else { e.before };
        self.apply(e.page, e.pad, a);
        let mut out = Vec::new();
        if e.page != self.page {
            self.page = e.page;
            out.push(Act::Send(FromUi::PerformPage(e.page)));
        }
        self.pad = e.pad;
        out.push(Act::Send(FromUi::PerformPad {
            page: e.page,
            pad: e.pad,
            action: a,
        }));
        let label = pad_label(e.page, e.pad);
        let what = match a {
            PadAction::None => "nothing".to_string(),
            a => edit_text(a, names),
        };
        out.push(Act::Tell(format!(
            "{}: Perform pads {label} back to {what}",
            if redo { "Redo" } else { "Undo" }
        )));
        out
    }

    /// The undo history as the list shows it: its rows ("Start", the
    /// steps UNDO takes back, oldest first, then REDO's) and the row that
    /// is now.
    pub fn history_rows(&self, names: &Names) -> (Vec<String>, usize) {
        let row = |e: &Edit| {
            let label = pad_label(e.page, e.pad);
            match e.after {
                PadAction::None => format!("{label} cleared"),
                a => format!("{label}: {}", edit_text(a, names)),
            }
        };
        let mut rows = vec!["Start: the set as it was".to_string()];
        rows.extend(self.done.iter().map(row));
        let now = rows.len() - 1;
        rows.extend(self.undone.iter().rev().map(row));
        (rows, now)
    }

    /// The wheel moved the Pad list's highlight `d` rows.
    pub fn pick_row(&self, ui: &mut Ui, d: i32) {
        if let Some(l) = ui.get_mut::<SettingList>(LIST) {
            let last = l.rows.len().saturating_sub(1) as i64;
            l.selected = (l.selected as i64 + i64::from(d)).clamp(0, last) as usize;
        }
    }

    /// The highlighted row of the Pad list stepped `d` (- / +, the wheel
    /// with the row open).
    pub fn step_row(&mut self, ui: &Ui, d: i32, names: &Names) -> Vec<Act> {
        let row = ui.get::<SettingList>(LIST).map_or(0, |l| l.selected);
        self.step(row, d, names)
    }

    /// Row `row` of the Pad list stepped `d`.
    fn step(&mut self, row: usize, d: i32, names: &Names) -> Vec<Act> {
        let a = self.picked();
        let d = d.signum();
        if d == 0 {
            return Vec::new();
        }
        let after = match (row, a) {
            (0, a) => {
                // Nothing, then each kind in the chips' order.
                let k = Kind::of(a).and_then(|k| Kind::ALL.iter().position(|&x| x == k));
                let at = k.map_or(0, |k| k as i32 + 1);
                let to = (at + d).clamp(0, Kind::ALL.len() as i32);
                match to {
                    0 => PadAction::None,
                    k => Kind::ALL[k as usize - 1].action(a),
                }
            }
            (1, PadAction::Throw(t)) => PadAction::Throw(Throw {
                index: stepped(usize::from(t.index), d, THROW_LIST.len()) as u8,
                ..t
            }),
            (2, PadAction::Throw(t)) => PadAction::Throw(Throw {
                hold: if d > 0 { Hold::Latch } else { Hold::WhileHeld },
                ..t
            }),
            (1, PadAction::LoopKey(n)) => {
                PadAction::LoopKey(stepped(usize::from(n), d, LOOPS) as u8)
            }
            (1, PadAction::LoopMute(n)) => {
                PadAction::LoopMute(stepped(usize::from(n), d, LOOPS) as u8)
            }
            (1, PadAction::Mute(t)) => {
                let len = names.tracks.len().clamp(1, 256);
                PadAction::Mute(stepped(usize::from(t), d, len) as u8)
            }
            (1, PadAction::Scene(n)) => PadAction::Scene(stepped(usize::from(n), d, SCENES) as u8),
            _ => return Vec::new(),
        };
        self.edit(self.pad, after, names)
    }

    /// One of the eight choices tapped: `i` from the first shown.
    fn choose(&mut self, i: usize, names: &Names) -> Vec<Act> {
        let a = self.picked();
        let (chips, first, _) = Self::choices(a, names);
        if i >= chips.len() {
            return Vec::new();
        }
        let n = first + i;
        let after = match a {
            PadAction::None => return Vec::new(),
            PadAction::Throw(t) => PadAction::Throw(Throw {
                index: n as u8,
                ..t
            }),
            PadAction::LoopKey(_) => PadAction::LoopKey(n as u8),
            PadAction::LoopMute(_) => PadAction::LoopMute(n as u8),
            PadAction::Mute(_) => PadAction::Mute(n.min(255) as u8),
            PadAction::Scene(_) => PadAction::Scene(n.min(255) as u8),
        };
        self.edit(self.pad, after, names)
    }

    /// A tap on one of the view's widgets: what it asks.
    pub fn event(&mut self, ui: &mut Ui, ev: &UiEvent, names: &Names) -> Vec<Act> {
        let label = pad_label(self.page, self.pad);
        match (ev.widget.as_str(), &ev.kind) {
            (BAR, EventKind::Tab(0)) => vec![Act::Send(if self.on {
                FromUi::PerformOff
            } else {
                FromUi::PerformOn
            })],
            (MAP, EventKind::Tab(k)) if *k < PAGES => {
                if *k == self.page {
                    return Vec::new();
                }
                self.page = *k;
                vec![Act::Send(FromUi::PerformPage(*k))]
            }
            (MAP, EventKind::Select(p)) if *p < PADS => {
                self.pad = *p;
                Vec::new()
            }
            (MAP, EventKind::Pad { index, down: true }) => {
                match self.set.page(self.page).action(*index) {
                    PadAction::Throw(t) => {
                        let i = usize::from(t.index);
                        self.hearing = Some(i);
                        vec![Act::Hear(i, true)]
                    }
                    PadAction::None => Vec::new(),
                    a => vec![Act::Tell(format!(
                        "{} is a {}: it plays on Play and the Looper tab with the set on the pads",
                        pad_label(self.page, *index),
                        Kind::of(a).map_or("pad", Kind::label).to_lowercase()
                    ))],
                }
            }
            (MAP, EventKind::Pad { down: false, .. }) => match self.hearing.take() {
                Some(i) => vec![Act::Hear(i, false)],
                None => Vec::new(),
            },
            (BUTTONS, EventKind::Tab(COPY)) => {
                let a = self.picked();
                self.copied = (a != PadAction::None).then_some(a);
                vec![Act::Tell(format!(
                    "Copied {label}: {}",
                    edit_text(a, names)
                ))]
            }
            (BUTTONS, EventKind::Tab(PASTE)) => match self.copied {
                Some(a) => self.edit(self.pad, a, names),
                None => Vec::new(),
            },
            (BUTTONS, EventKind::Tab(CLEAR)) => self.edit(self.pad, PadAction::None, names),
            (BUTTONS, EventKind::Select(i)) => ui
                .get::<ButtonRow>(BUTTONS)
                .and_then(|b| b.why_faint(*i))
                .map(|w| vec![Act::Tell(w.to_string())])
                .unwrap_or_default(),
            (TYPE, EventKind::Tab(i)) => match Kind::ALL.get(*i) {
                Some(k) => {
                    let after = k.action(self.picked());
                    self.edit(self.pad, after, names)
                }
                None => Vec::new(),
            },
            (PICK_A, EventKind::Tab(i)) => self.choose(*i, names),
            (PICK_B, EventKind::Tab(i)) => self.choose(4 + *i, names),
            (LIST, EventKind::Step { index, delta }) => self.step(*index, *delta, names),
            _ => Vec::new(),
        }
    }
}

/// `a` for the log ("Reverb wash, latched", "Loop 2's key").
fn edit_text(a: PadAction, names: &Names) -> String {
    match a {
        PadAction::None => "nothing".to_string(),
        PadAction::Throw(t) => match t.hold {
            Hold::WhileHeld => format!("{}, while held", t.name()),
            Hold::Latch => format!("{}, latched", t.name()),
        },
        PadAction::LoopKey(n) => format!("Loop {}'s key", n + 1),
        PadAction::LoopMute(n) => format!("Loop {}'s mute", n + 1),
        PadAction::Mute(t) => format!("{}'s mute", names.track(usize::from(t))),
        PadAction::Scene(n) => format!("launches {}", names.scene(usize::from(n))),
    }
}

/// `n` stepped `d` within `0..len`, stopping at the ends.
fn stepped(n: usize, d: i32, len: usize) -> usize {
    let last = len.saturating_sub(1) as i64;
    (n as i64 + i64::from(d)).clamp(0, last) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Names {
        Names {
            tracks: vec![
                ("Drums".into(), "Kit".into()),
                ("Bass".into(), "FM · Sub".into()),
                ("Keys".into(), String::new()),
            ],
            scenes: vec![String::new(), "Drop".into()],
        }
    }

    fn sends(acts: &[Act]) -> Vec<&FromUi> {
        acts.iter()
            .filter_map(|a| match a {
                Act::Send(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_kind_keeps_the_loop_and_starts_the_rest_at_the_first() {
        assert_eq!(
            Kind::LoopMute.action(PadAction::LoopKey(3)),
            PadAction::LoopMute(3)
        );
        assert_eq!(
            Kind::LoopKey.action(PadAction::LoopMute(5)),
            PadAction::LoopKey(5)
        );
        assert_eq!(
            Kind::Scene.action(PadAction::LoopKey(5)),
            PadAction::Scene(0)
        );
        let t = PadAction::Throw(Throw {
            index: 3,
            hold: Hold::Latch,
        });
        assert_eq!(Kind::Throw.action(t), t);
        assert_eq!(Kind::of(PadAction::None), None);
    }

    #[test]
    fn edits_go_to_the_app_and_undo_takes_them_back() {
        let mut tab = PerformTab {
            page: 1,
            pad: 1,
            ..PerformTab::default()
        };
        let n = names();
        // B2 is Reverb wash, held: made a scene.
        let acts = tab.edit(1, Kind::Scene.action(tab.picked()), &n);
        assert_eq!(
            sends(&acts),
            [&FromUi::PerformPad {
                page: 1,
                pad: 1,
                action: PadAction::Scene(0),
            }]
        );
        assert!(acts.contains(&Act::Tell("Perform pads B2: launches Scene 1".into())));
        assert_eq!(tab.picked(), PadAction::Scene(0));
        // Row 2, Drop.
        tab.step(1, 1, &n);
        assert_eq!(tab.picked(), PadAction::Scene(1));
        let (rows, now) = tab.history_rows(&n);
        assert_eq!(
            rows,
            [
                "Start: the set as it was",
                "B2: launches Scene 1",
                "B2: launches Scene 2 · Drop"
            ]
        );
        assert_eq!(now, 2);
        // UNDO twice: Reverb wash again, on page B even from page C.
        tab.page = 2;
        let acts = tab.undo(false, &n);
        assert_eq!(sends(&acts)[0], &FromUi::PerformPage(1));
        tab.undo(false, &n);
        assert_eq!(
            tab.picked(),
            PadAction::Throw(Throw {
                index: 1,
                hold: Hold::WhileHeld
            })
        );
        let acts = tab.undo(false, &n);
        assert_eq!(
            acts,
            [Act::Tell("Nothing to undo on the Perform set".into())]
        );
        // SHIFT + UNDO: the scene again.
        tab.undo(true, &n);
        assert_eq!(tab.picked(), PadAction::Scene(0));
        assert_eq!(tab.history_rows(&n).1, 1);
    }

    #[test]
    fn a_throw_steps_its_effect_and_hold() {
        let mut tab = PerformTab {
            page: 1,
            ..PerformTab::default()
        };
        let n = names();
        tab.step(1, 1, &n);
        tab.step(2, 1, &n);
        assert_eq!(
            tab.picked(),
            PadAction::Throw(Throw {
                index: 1,
                hold: Hold::Latch
            })
        );
        // The ends hold.
        assert!(tab.step(2, 1, &n).is_empty());
        let a = tab.picked();
        assert_eq!(
            hit_text("B1", a, &n),
            "A hit starts Reverb wash on the master and the next lets it go (it rings out)."
        );
        assert_eq!(action_summary(a, &n), "Throw · latched, on the master");
    }

    #[test]
    fn the_track_and_scene_choices_show_eight_round_the_one_picked() {
        let n = names();
        let (chips, first, now) = PerformTab::choices(PadAction::Mute(1), &n);
        assert_eq!(first, 0);
        assert_eq!(now, Some(1));
        assert_eq!(
            chips.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["Drums", "Bass", "Keys"]
        );
        assert_eq!(chips[1].sub, "FM · Sub");
        let (chips, first, _) = PerformTab::choices(PadAction::Scene(10), &n);
        assert_eq!(first, 8);
        assert_eq!(chips.len(), 8);
        assert_eq!(chips[0].name, "Scene 9");
        let (chips, _, now) = PerformTab::choices(PadAction::Scene(1), &n);
        assert_eq!(chips[1].sub, "Drop");
        assert_eq!(now, Some(1));
        // A mute of a track past the song's: its window still shows.
        let (chips, first, _) = PerformTab::choices(PadAction::Mute(9), &n);
        assert_eq!(first, 8);
        assert_eq!(chips[1].name, "Track 10");
    }

    #[test]
    fn a_new_project_starts_the_history_again() {
        let n = names();
        let mut tab = PerformTab {
            page: 1,
            ..PerformTab::default()
        };
        tab.step(0, -1, &n);
        assert_eq!(tab.picked(), PadAction::None);
        // The app's echo of the edit keeps it.
        let echo = tab.set.clone();
        tab.set_set(echo, false);
        assert_eq!(tab.history_rows(&n).0.len(), 2);
        // Another project's set: a history of its own.
        let mut other = PerformSet::default();
        other.pages[2].pads[0] = PadAction::Scene(3);
        tab.set_set(other, true);
        assert_eq!(tab.history_rows(&n).0.len(), 1);
        assert_eq!(tab.set.page(2).action(0), PadAction::Scene(3));
    }
}
