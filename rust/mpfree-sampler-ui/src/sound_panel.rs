//! The Sound section beside the wave: how the pad plays its sound, four
//! controls at a time (the one Sample page, Jay 18 Nov 2026: "a single
//! page for recording inputs, chopping them up, and placing the chopped
//! samples somewhere else"; mockup https://claude.ai/artifact/NSGLzNnVy5zTA3ykWmpGzU).
//!
//! - **Its parts** are buttons along its top: **Amp** (the amp envelope),
//!   **Filter** (its type, cutoff, resonance and envelope depth),
//!   **Pitch** (tune, wow and flutter, velocity to pitch, Vintage) and
//!   **Movement** (what the LFO moves, its rate and depth, the pitch
//!   envelope). A tap opens one; what it shows is the Play page's own
//!   rows ([`crate::play_tab`]), so the two never disagree.
//! - **Its four controls** turn, step and switch as the Play page's do,
//!   and each change is a [`PlayAction::Pad`]: the app applies it as the
//!   Play page's, a drag or a run of steps one undo step.
//! - A control that does nothing now (the filter's while it is off) is
//!   faint and says why when tapped.
//!
//! The section is a [`SettingList`] drawn as a one-section panel, so the
//! hold-and-turn of the wheel works on it as on every panel.

use mpfree_ui::{EventKind, Ui, UiEvent};

use crate::panel::{Control, Item, Look, Panel, Section};
use crate::play_tab::{self, PlayAction, PlayState, Row};
use crate::widgets::{Setting, SettingList};

/// The section's widget.
pub const SOUND: &str = "smp-sound";
/// Its width on the page, as the mockup has it.
pub const WIDTH: i32 = 340;

/// Gesture ids the section hands out start here, clear of other screens'.
const GESTURE_BASE: u64 = 0x534E_4400_0000_0000;

/// The section's parts, as buttons along its top.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Amp,
    Filter,
    Pitch,
    Movement,
}

impl Part {
    pub const ALL: [Part; 4] = [Part::Amp, Part::Filter, Part::Pitch, Part::Movement];

    pub fn label(self) -> &'static str {
        match self {
            Part::Amp => "Amp",
            Part::Filter => "Filter",
            Part::Pitch => "Pitch",
            Part::Movement => "Movement",
        }
    }

    /// Its four controls, left to right.
    pub fn rows(self) -> [Row; 4] {
        match self {
            Part::Amp => [Row::Attack, Row::Decay, Row::Sustain, Row::Release],
            Part::Filter => [Row::Filter, Row::Cutoff, Row::Resonance, Row::FilterEnv],
            Part::Pitch => [Row::Tune, Row::Wow, Row::VelToPitch, Row::Vintage],
            Part::Movement => [Row::LfoTo, Row::LfoRate, Row::LfoDepth, Row::PitchEnv],
        }
    }

    fn index(self) -> usize {
        Part::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }
}

/// The list's row of the part buttons; the part's controls follow it.
const PARTS_ROW: usize = 0;

pub struct SoundPanel {
    part: Part,
    /// The pad shown, as the app last gave it.
    shown: Option<PlayState>,
    gesture: Option<(Row, u64)>,
    next_gesture: u64,
}

impl SoundPanel {
    /// Adds the section's widget to `ui`; the page puts [`SOUND`] in its
    /// layout.
    pub fn add(ui: &mut Ui) -> SoundPanel {
        ui.add(SOUND, SettingList::new("Sound"));
        let s = SoundPanel {
            part: Part::Amp,
            shown: None,
            gesture: None,
            next_gesture: GESTURE_BASE,
        };
        s.show(ui);
        s
    }

    /// The part open.
    pub fn part(&self) -> Part {
        self.part
    }

    /// The pad shown.
    pub fn state(&self) -> Option<&PlayState> {
        self.shown.as_ref()
    }

    /// Shows pad `st` (None: no pad with a sound, every control faint).
    pub fn update(&mut self, ui: &mut Ui, st: Option<PlayState>) {
        let which = |s: &PlayState| (s.pad, s.take.clone());
        if self.shown.as_ref().map(which) != st.as_ref().map(which) {
            self.lift();
        }
        self.shown = st;
        self.show(ui);
    }

    /// Opens part `part`.
    pub fn open(&mut self, ui: &mut Ui, part: Part) {
        if part != self.part {
            self.part = part;
            self.lift();
        }
        self.show(ui);
    }

    /// Ends the drag or run of steps being made: the next change is a new
    /// undo step. Call it when a finger lifts and when the wheel rests.
    pub fn lift(&mut self) {
        self.gesture = None;
    }

    /// What a touch on the section asks for; None for touches elsewhere,
    /// or that only change what it shows.
    pub fn action(&mut self, ui: &mut Ui, ev: &UiEvent) -> Option<PlayAction> {
        if ev.widget != SOUND {
            return None;
        }
        match &ev.kind {
            EventKind::Step { index, delta } if *index == PARTS_ROW => {
                let n = Part::ALL.len() as i32;
                let to = (self.part.index() as i32 + delta).rem_euclid(n) as usize;
                self.open(ui, Part::ALL[to]);
                None
            }
            EventKind::Step { index, delta } => {
                let row = *self.part.rows().get(index.checked_sub(1)?)?;
                let a = self.step(row, *delta);
                self.show(ui);
                a
            }
            _ => None,
        }
    }

    /// Row `row` stepped `delta` steps, as the Play page steps it.
    fn step(&mut self, row: Row, delta: i32) -> Option<PlayAction> {
        let st = self.shown.as_ref()?;
        if delta == 0 || play_tab::control_of(st, 0, 0, row).faint {
            return None;
        }
        let after = play_tab::stepped(&st.settings, row, delta, st.granular());
        if after == st.settings {
            return None;
        }
        let pad = st.pad;
        let gesture = match self.gesture {
            Some((r, g)) if r == row => g,
            _ => {
                let g = self.next_gesture;
                self.next_gesture += 1;
                self.gesture = Some((row, g));
                g
            }
        };
        // Shown at once; the app's next update brings the same.
        if let Some(s) = self.shown.as_mut() {
            s.settings = after.clone();
        }
        Some(PlayAction::Pad {
            pad,
            after: Box::new(after),
            gesture,
        })
    }

    fn show(&self, ui: &mut Ui) {
        let rows = self.part.rows();
        let parts = Control::new(Look::Choice {
            options: Part::ALL.iter().map(|p| p.label().to_string()).collect(),
            on: self.part.index(),
            off: Vec::new(),
        });
        let (settings, controls): (Vec<Setting>, Vec<Control>) = std::iter::once((
            Setting {
                name: "Sound".into(),
                value: self.part.label().into(),
            },
            parts,
        ))
        .chain(rows.iter().map(|&row| {
            let setting = Setting {
                name: row.label().into(),
                value: self
                    .shown
                    .as_ref()
                    .map_or_else(|| "—".into(), |st| play_tab::text_of(st, 0, 0, row)),
            };
            let control = match self.shown.as_ref() {
                Some(st) => compact(play_tab::control_of(st, 0, 0, row)),
                None => Control::new(Look::Chip).faint_because(true, "No sound on the pad"),
            };
            (setting, control)
        }))
        .unzip();
        let items = rows
            .iter()
            .enumerate()
            .map(|(k, row)| {
                let it = Item::new(k + 1);
                // A selector's name goes over it; a knob and a switch
                // have theirs already.
                if matches!(row, Row::Filter | Row::LfoTo) {
                    it.labelled()
                } else {
                    it
                }
            })
            .collect();
        let note = self
            .shown
            .as_ref()
            .map_or(String::new(), |st| match &st.take {
                Some(name) => name.clone(),
                None => format!("pad {}", crate::wave_tab::pad_label(st.pad)),
            });
        let panel = Panel {
            sections: vec![
                Section::new("Sound", WIDTH, vec![vec![Item::new(PARTS_ROW)], items]).note(&note),
            ],
        };
        if let Some(list) = ui.get_mut::<SettingList>(SOUND) {
            list.right = self.part.label().into();
            list.rows = settings;
            list.controls = controls;
            list.panel = Some(panel);
        }
    }
}

/// A control as the Sound section draws it: a choice of several options
/// (the LFO's targets) as a selector, so four fit side by side.
fn compact(c: Control) -> Control {
    match c.look {
        Look::Choice { .. } => Control {
            look: Look::Chip,
            ..c
        },
        _ => c,
    }
}
