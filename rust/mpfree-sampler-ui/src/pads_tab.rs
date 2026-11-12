//! The pad editor's Pads tab: the kit's pads at a glance, to pick one and
//! set it up. It sits first in the pad bar, a sub-page of the Sample tab
//! that leaves room for the pad's settings and the kit's sounds.
//!
//! - The **pad map** shows the bank's 16 pads as on the MPC (pad 1 bottom
//!   left): name, layers and play mode, and the clip a pad launches. A tap
//!   picks a pad for every tab (Wave, Chop and Play edit it next); a tap on
//!   the pad picked plays it while the finger is down. Pads held on the
//!   hardware light up.
//! - **Copy**, **Paste** and **Clear** work on the pad picked, each one undo
//!   step. **Map to keys** makes the bank's pads one keygroup instrument:
//!   each pad at its sound's own note (from its keygroup, its name or the
//!   pitch heard, worked out by the app), reaching halfway to its
//!   neighbours, and the pads then play notes; one undo step.
//! - The **Pad** list holds the settings a pad is set up with: its sound,
//!   play mode, voices, mute group, level, pan and tune. Stepping Sound puts
//!   the kit's next or previous sound on the pad.
//! - The **sounds** list holds every sound in the kit, with the pads that
//!   use it; **Put on** puts the sound picked on the pad picked.
//! - **Q-Links 1 to 4** turn level, pan, tune and voices; the **wheel**
//!   steps the row picked.
//!
//! The app makes a [`PadsState`] for the kit on show, calls
//! [`PadsTab::update`] when the project changes, and applies the
//! [`PadsAction`]s it gets back.

use mpfree_sampler::model::{
    ChannelPick, Fade, GrainParams, Layer, ModelError, Pad, PlayMode, Project, Region, SourceId,
};
use mpfree_ui::widgets::{ListItem, ListView};
use mpfree_ui::{EventKind, Ui, UiEvent};

use crate::chop_tab::{bank_of, pad_text};
use crate::play_tab::{self, stepped, value_text};
use crate::rec_tab;
use crate::wave_tab::{self, default_loop, duration_text, mode_label, pad_color, TABS};
use crate::widgets::{ButtonRow, PadBar, PadCell, PadMap, Setting, SettingList};

/// Widget names the tab adds. The app's own are `header` and `hint`.
pub const BAR: &str = "smp-pads-bar";
pub const MAP: &str = "smp-pads-map";
pub const BUTTONS: &str = "smp-pads-buttons";
pub const LIST: &str = "smp-pads-list";
pub const SOUNDS: &str = "smp-pads-sounds";
pub const PUT: &str = "smp-pads-put";

/// The tab's page: the pad map with its buttons, the Pad list, and the
/// kit's sounds with the button that puts one on the pad.
pub const LAYOUT: &str = "col(header:60, smp-pads-bar:60, \
                          row(col(smp-pads-map:*, smp-pads-buttons:64):440, smp-pads-list:*, \
                          col(smp-pads-sounds:*, smp-pads-put:64):340):*, hint:26)";

/// The tab's place in [`TABS`].
pub const TAB: usize = 0;

/// The hint line the app shows under the tab.
pub const HINT: &str =
    "Tap a pad to pick it, again to hear it · Copy, Paste, Clear, Map to keys · pick a sound, then Put it on \
                        the pad · Q-Links: level, pan, tune, voices";

/// Gesture ids the tab hands out start here, clear of other screens'.
const GESTURE_BASE: u64 = 0x5041_4453_0000_0000;

/// The Pad list's rows, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Sound,
    Mode,
    Polyphony,
    MuteGroup,
    Level,
    Pan,
    Tune,
}

impl Row {
    pub const ALL: [Row; 7] = [
        Row::Sound,
        Row::Mode,
        Row::Polyphony,
        Row::MuteGroup,
        Row::Level,
        Row::Pan,
        Row::Tune,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Row::Sound => "Sound",
            Row::Mode => "Mode",
            Row::Polyphony => "Polyphony",
            Row::MuteGroup => "Mute group",
            Row::Level => "Level",
            Row::Pan => "Pan",
            Row::Tune => "Tune",
        }
    }

    /// The Play tab's row for the same setting.
    fn play_row(self) -> Option<play_tab::Row> {
        match self {
            Row::Polyphony => Some(play_tab::Row::Polyphony),
            Row::MuteGroup => Some(play_tab::Row::MuteGroup),
            Row::Level => Some(play_tab::Row::Level),
            Row::Pan => Some(play_tab::Row::Pan),
            Row::Tune => Some(play_tab::Row::Tune),
            Row::Sound | Row::Mode => None,
        }
    }

    fn index(self) -> usize {
        Row::ALL.iter().position(|r| *r == self).unwrap_or(0)
    }
}

/// The rows the Q-Links turn, in knob order.
pub const QLINK_ROWS: [Row; 4] = [Row::Level, Row::Pan, Row::Tune, Row::Polyphony];

const MODES: [PlayMode; 5] = [
    PlayMode::OneShot,
    PlayMode::Gate,
    PlayMode::Toggle,
    PlayMode::Loop,
    PlayMode::Granular,
];

/// The buttons under the map, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Copy,
    Paste,
    Clear,
    MapToKeys,
}

impl Button {
    pub const ALL: [Button; 4] = [
        Button::Copy,
        Button::Paste,
        Button::Clear,
        Button::MapToKeys,
    ];
}

/// A sound in the kit.
#[derive(Clone, Debug, PartialEq)]
pub struct Sound {
    pub id: SourceId,
    pub name: String,
    pub frames: u64,
    pub rate: u32,
    /// The pads (of the whole project) that play it.
    pub pads: Vec<usize>,
}

/// Everything the tab shows: the bank of the pad picked, and the kit's
/// sounds.
#[derive(Clone, Debug)]
pub struct PadsState {
    /// The pad picked.
    pub pad: usize,
    /// The bank's first pad.
    pub first: usize,
    /// The bank's pads, None where a pad has no sound.
    pub pads: Vec<Option<Pad>>,
    pub sounds: Vec<Sound>,
    /// The clip each pad of the bank launches, if any (none yet: the time
    /// engine doesn't launch clips from pads).
    pub clips: Vec<Option<String>>,
}

impl PadsState {
    /// The bank of pad `pad` in `p`, and its sounds.
    pub fn from_project(p: &Project, pad: usize) -> PadsState {
        let bank = bank_of(pad);
        let pads = bank
            .clone()
            .map(|i| p.pad(i).filter(|pd| !pd.layers.is_empty()).cloned())
            .collect();
        let sounds = p
            .sources()
            .iter()
            .map(|s| Sound {
                id: s.id(),
                name: s.name.clone(),
                frames: s.frames(),
                rate: s.rate(),
                pads: p.users_of(s.id()),
            })
            .collect();
        PadsState {
            pad,
            first: bank.start,
            pads,
            sounds,
            clips: vec![None; bank.len()],
        }
    }

    /// The pad picked, if it has a sound.
    pub fn picked(&self) -> Option<&Pad> {
        self.pads
            .get(self.pad - self.first)
            .and_then(Option::as_ref)
    }

    /// The sound on the pad picked's first layer, as an index into `sounds`.
    pub fn sound_of_picked(&self) -> Option<usize> {
        let id = self.picked()?.layers.first()?.region.source;
        self.sounds.iter().position(|s| s.id == id)
    }
}

/// What a touch or control on the tab asks for. The app applies it.
#[derive(Clone, Debug, PartialEq)]
pub enum PadsAction {
    /// Pad `pad` picked: every tab of the editor edits it from now on.
    Pick(usize),
    /// Play pad `pad` (down) or let it go, as a hit would.
    Audition { pad: usize, down: bool },
    /// Pad `pad` set to `after` (None clears it). `gesture` merges a run of
    /// changes into one undo step; None is a step of its own.
    Set {
        pad: usize,
        after: Option<Box<Pad>>,
        gesture: Option<u64>,
    },
    /// Map to keys: the app makes `pads` (the bank's pads with a sound) a
    /// keygroup instrument at their sounds' notes
    /// ([`mpfree_sampler::keymap`], `Project::map_to_keys`).
    MapToKeys { pads: Vec<usize> },
    /// Open the editor's tab `i` of [`TABS`].
    Tab(usize),
}

impl PadsAction {
    /// Applies [`PadsAction::Set`] to the project; other actions change
    /// nothing here.
    pub fn apply(&self, p: &mut Project) -> Result<(), ModelError> {
        match self {
            PadsAction::Set {
                pad,
                after,
                gesture,
            } => p.set_pad(*pad, after.as_deref().cloned(), *gesture),
            _ => Ok(()),
        }
    }
}

/// `pad` (or a new pad) playing `sound` from start to end on its first
/// layer. Its other settings, and the old sound's gain, tune, channels and
/// play mode, stay; it takes the sound's name.
pub fn with_sound(pad: Option<&Pad>, sound: &Sound) -> Pad {
    let whole = |old: Option<&Region>| {
        let mut r = match old {
            Some(o) => Region {
                source: sound.id,
                start: 0,
                end: sound.frames,
                sustain_loop: None,
                ..o.clone()
            },
            None => Region {
                source: sound.id,
                start: 0,
                end: sound.frames,
                sustain_loop: None,
                fade_in: Fade::NONE,
                fade_out: Fade::NONE,
                gain_db: 0.0,
                reverse: false,
                channels: ChannelPick::Stereo,
                tune_cents: 0,
                mode: PlayMode::OneShot,
                grain: GrainParams::default(),
                warp: None,
            },
        };
        (r.fade_in, r.fade_out) = (Fade::NONE, Fade::NONE);
        if r.mode == PlayMode::Loop {
            r.sustain_loop = Some(default_loop(&r));
        }
        r
    };
    match pad {
        Some(p) => {
            let mut p = p.clone();
            if let Some(l) = p.layers.first_mut() {
                l.region = whole(Some(&l.region));
            }
            p.name = sound.name.clone();
            p
        }
        None => Pad::new(&sound.name, Layer::new(whole(None))),
    }
}

/// `p` with every layer in play mode `mode` (a loop added where Loop needs
/// one).
fn with_mode(p: &Pad, mode: PlayMode) -> Pad {
    let mut p = p.clone();
    for l in &mut p.layers {
        l.region.mode = mode;
        if mode == PlayMode::Loop && l.region.sustain_loop.is_none() {
            l.region.sustain_loop = Some(default_loop(&l.region));
        }
    }
    p
}

/// Which edit an undo step belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Row(Row),
}

/// The Pads tab's widgets and what their touches mean. It keeps the pad
/// copied, the sound picked and the undo step being made.
pub struct PadsTab {
    /// The page name the tab's layout is added under.
    pub name: String,
    /// The bank letter shown in pad names, as [`wave_tab::WaveTab::bank`].
    pub bank: Option<char>,
    /// A kit name for the map's title, such as "Kit 1 · Breaks".
    pub kit: String,
    copied: Option<(usize, Pad)>,
    sound: usize,
    shown_pad: Option<usize>,
    gesture: Option<(Key, u64)>,
    next_gesture: u64,
}

impl PadsTab {
    /// Adds the widgets and the page `name` to `ui` (the app adds `header`
    /// and `hint` first) and shows `st`.
    pub fn add(ui: &mut Ui, name: &str, st: &PadsState) -> Result<PadsTab, String> {
        let mut bar = PadBar::new(&TABS);
        bar.enabled = TABS.iter().map(|t| wave_tab::BUILT.contains(t)).collect();
        bar.active = TAB;
        ui.add(BAR, bar);
        ui.add(MAP, PadMap::new("Pads"));
        ui.add(
            BUTTONS,
            ButtonRow::new(&["Copy", "Paste", "Clear", "Map to keys"]),
        );
        ui.add(LIST, SettingList::new("Pad"));
        ui.add(SOUNDS, ListView::new("Sounds in the kit"));
        let mut put = ButtonRow::new(&["Put on pad"]);
        put.primary = Some(0);
        ui.add(PUT, put);
        ui.add_page(name, LAYOUT)?;
        let mut tab = PadsTab {
            name: name.to_string(),
            bank: None,
            kit: String::new(),
            copied: None,
            sound: 0,
            shown_pad: None,
            gesture: None,
            next_gesture: GESTURE_BASE,
        };
        tab.update(ui, st);
        Ok(tab)
    }

    /// The row picked in the Pad list.
    pub fn row(&self, ui: &Ui) -> Row {
        ui.get::<SettingList>(LIST)
            .and_then(|l| Row::ALL.get(l.selected).copied())
            .unwrap_or(Row::Sound)
    }

    /// The sound picked in the sounds list.
    pub fn sound(&self) -> usize {
        self.sound
    }

    /// Ends the turn or run of taps being made: the next edit is a new
    /// undo step.
    pub fn lift(&mut self) {
        self.gesture = None;
    }

    /// Pad `pad` (of the project) is held down, or let go.
    pub fn set_lit(&self, ui: &mut Ui, st: &PadsState, pad: usize, down: bool) {
        let Some(i) = pad.checked_sub(st.first).filter(|i| *i < st.pads.len()) else {
            return;
        };
        if let Some(m) = ui.get_mut::<PadMap>(MAP) {
            m.lit.resize(st.pads.len(), false);
            m.lit[i] = down;
        }
    }

    /// Shows `st`.
    pub fn update(&mut self, ui: &mut Ui, st: &PadsState) {
        if self.shown_pad != Some(st.pad) {
            self.shown_pad = Some(st.pad);
            if let Some(s) = st.sound_of_picked() {
                self.sound = s;
            }
        }
        self.sound = self.sound.min(st.sounds.len().saturating_sub(1));
        self.show(ui, st);
    }

    fn show(&self, ui: &mut Ui, st: &PadsState) {
        let here = pad_text(self.bank, st.pad);
        let picked = st.picked();
        if let Some(bar) = ui.get_mut::<PadBar>(BAR) {
            bar.color = pad_color(st.pad);
            bar.active = TAB;
            // A pad without a sound has nothing for the other tabs to edit,
            // but one can be recorded onto it.
            bar.enabled = TABS
                .iter()
                .map(|t| {
                    wave_tab::BUILT.contains(t)
                        && (picked.is_some() || *t == TABS[TAB] || *t == TABS[rec_tab::TAB])
                })
                .collect();
            bar.title = match picked {
                Some(p) => format!("Pad {here} · {}", p.name),
                None => format!("Pad {here} · no sound"),
            };
            let used = st.pads.iter().filter(|p| p.is_some()).count();
            let n = st.sounds.len();
            bar.subtitle = format!(
                "{used} of {} pads with a sound · {n} {} in the kit",
                st.pads.len(),
                if n == 1 { "sound" } else { "sounds" }
            );
        }
        let cells: Vec<PadCell> = st
            .pads
            .iter()
            .enumerate()
            .map(|(i, p)| PadCell {
                label: pad_text(self.bank, st.first + i),
                name: p.as_ref().map_or_else(String::new, |p| p.name.clone()),
                detail: p.as_ref().map_or_else(String::new, pad_detail),
                clip: st.clips.get(i).cloned().flatten(),
                color: pad_color(st.first + i),
            })
            .collect();
        let kit = self.kit.clone();
        if let Some(m) = ui.get_mut::<PadMap>(MAP) {
            m.title = if kit.is_empty() { "Pads".into() } else { kit };
            m.right = self.bank.map_or_else(String::new, |b| format!("bank {b}"));
            m.lit.resize(cells.len(), false);
            m.cells = cells;
            m.selected = st.pad - st.first;
        }
        let paste = self
            .copied
            .as_ref()
            .map(|(from, _)| format!("Paste {}", pad_text(self.bank, *from)));
        if let Some(b) = ui.get_mut::<ButtonRow>(BUTTONS) {
            b.labels[1] = paste.unwrap_or_else(|| "Paste".into());
            let sounds = st.pads.iter().any(Option::is_some);
            b.enabled = vec![
                picked.is_some(),
                self.copied.is_some(),
                picked.is_some(),
                sounds,
            ];
        }
        let rows = Row::ALL
            .iter()
            .map(|r| Setting {
                name: r.label().into(),
                value: row_text(picked, st, *r),
            })
            .collect();
        let clip = st.clips.get(st.pad - st.first).cloned().flatten();
        if let Some(list) = ui.get_mut::<SettingList>(LIST) {
            list.title = format!("Pad {here}");
            list.right = format!("clip: {}", clip.as_deref().unwrap_or("none"));
            list.rows = rows;
        }
        let items = st
            .sounds
            .iter()
            .map(|s| ListItem {
                text: s.name.clone(),
                detail: match s.pads.len() {
                    0 => format!("{} · unused", duration_text(s.frames, s.rate)),
                    1 => format!(
                        "{} · {}",
                        duration_text(s.frames, s.rate),
                        pad_text(self.bank, s.pads[0])
                    ),
                    n => format!("{} · {n} pads", duration_text(s.frames, s.rate)),
                },
                color: None,
            })
            .collect();
        if let Some(l) = ui.get_mut::<ListView>(SOUNDS) {
            l.items = items;
            l.selected = self.sound;
        }
        if let Some(b) = ui.get_mut::<ButtonRow>(PUT) {
            b.labels[0] = format!("Put on {here}");
            b.enabled = vec![!st.sounds.is_empty()];
        }
    }

    /// What a touch on one of the tab's widgets asks for; None for touches
    /// that only change what the tab shows.
    pub fn action(&mut self, ui: &mut Ui, ev: &UiEvent, st: &PadsState) -> Option<PadsAction> {
        match (ev.widget.as_str(), &ev.kind) {
            (BAR, EventKind::Tab(i)) => Some(PadsAction::Tab(*i)),
            (MAP, EventKind::Select(i)) => Some(PadsAction::Pick(st.first + *i)),
            (MAP, EventKind::Pad { index, down }) => Some(PadsAction::Audition {
                pad: st.first + *index,
                down: *down,
            }),
            (BUTTONS, EventKind::Tab(i)) => self.press(ui, st, *Button::ALL.get(*i)?),
            (LIST, EventKind::Step { index, delta }) => {
                self.step(st, *Row::ALL.get(*index)?, *delta)
            }
            (SOUNDS, EventKind::Select(k)) => {
                self.sound = *k;
                None
            }
            (PUT, EventKind::Tab(0)) => {
                let sound = st.sounds.get(self.sound)?;
                let after = with_sound(st.picked(), sound);
                self.lift();
                Some(PadsAction::Set {
                    pad: st.pad,
                    after: Some(Box::new(after)),
                    gesture: None,
                })
            }
            _ => None,
        }
    }

    /// A button pressed (as a tap on it would).
    pub fn press(&mut self, ui: &mut Ui, st: &PadsState, b: Button) -> Option<PadsAction> {
        self.lift();
        match b {
            Button::Copy => {
                self.copied = Some((st.pad, st.picked()?.clone()));
                self.show(ui, st);
                None
            }
            Button::Paste => {
                let (_, p) = self.copied.as_ref()?;
                (st.picked() != Some(p)).then(|| PadsAction::Set {
                    pad: st.pad,
                    after: Some(Box::new(p.clone())),
                    gesture: None,
                })
            }
            Button::Clear => {
                st.picked()?;
                Some(PadsAction::Set {
                    pad: st.pad,
                    after: None,
                    gesture: None,
                })
            }
            Button::MapToKeys => {
                let pads: Vec<usize> = (0..st.pads.len())
                    .filter(|&k| st.pads[k].is_some())
                    .map(|k| st.first + k)
                    .collect();
                (!pads.is_empty()).then_some(PadsAction::MapToKeys { pads })
            }
        }
    }

    /// Q-Link `knob` (0 to 3: level, pan, tune, voices) turned by `delta`
    /// detents.
    pub fn qlink(
        &mut self,
        ui: &mut Ui,
        st: &PadsState,
        knob: usize,
        delta: i32,
    ) -> Option<PadsAction> {
        let row = *QLINK_ROWS.get(knob)?;
        if let Some(list) = ui.get_mut::<SettingList>(LIST) {
            list.selected = row.index();
        }
        self.step(st, row, delta)
    }

    /// The data wheel: steps the row picked.
    pub fn wheel(&mut self, ui: &mut Ui, st: &PadsState, delta: i32) -> Option<PadsAction> {
        let row = self.row(ui);
        self.step(st, row, delta)
    }

    fn step(&mut self, st: &PadsState, row: Row, delta: i32) -> Option<PadsAction> {
        if delta == 0 {
            return None;
        }
        let picked = st.picked();
        let after = match row {
            Row::Sound => {
                if st.sounds.is_empty() {
                    return None;
                }
                let n = st.sounds.len() as i32;
                let k = match st.sound_of_picked() {
                    Some(k) => (k as i32 + delta).rem_euclid(n),
                    None if delta > 0 => 0,
                    None => n - 1,
                };
                self.sound = k as usize;
                with_sound(picked, &st.sounds[k as usize])
            }
            Row::Mode => {
                let p = picked?;
                let now = p.layers.first()?.region.mode;
                let i = MODES.iter().position(|m| *m == now).unwrap_or(0) as i32;
                with_mode(
                    p,
                    MODES[(i + delta).rem_euclid(MODES.len() as i32) as usize],
                )
            }
            r => stepped(picked?, r.play_row()?, delta, false),
        };
        if Some(&after) == picked {
            return None;
        }
        let gesture = match self.gesture {
            Some((k, g)) if k == Key::Row(row) => g,
            _ => {
                let g = self.next_gesture;
                self.next_gesture += 1;
                self.gesture = Some((Key::Row(row), g));
                g
            }
        };
        Some(PadsAction::Set {
            pad: st.pad,
            after: Some(Box::new(after)),
            gesture: Some(gesture),
        })
    }
}

/// "3 layers · Loop", for the map.
fn pad_detail(p: &Pad) -> String {
    let mode = p.layers.first().map_or("", |l| mode_label(l.region.mode));
    match p.layers.len() {
        1 => mode.to_string(),
        n => format!("{n} layers · {mode}"),
    }
}

fn row_text(p: Option<&Pad>, st: &PadsState, row: Row) -> String {
    let Some(p) = p else {
        return if row == Row::Sound {
            "none".into()
        } else {
            "—".into()
        };
    };
    match row {
        Row::Sound => {
            let name = st
                .sound_of_picked()
                .map_or("?", |k| st.sounds[k].name.as_str());
            if p.layers.len() > 1 {
                format!("{name} +{}", p.layers.len() - 1)
            } else {
                name.to_string()
            }
        }
        Row::Mode => p
            .layers
            .first()
            .map_or("—".into(), |l| mode_label(l.region.mode).to_string()),
        r => r
            .play_row()
            .map_or_else(String::new, |pr| value_text(p, pr)),
    }
}
