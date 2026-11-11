//! The Sample tab: the sampler's pad editor (`mpfree-sampler-ui`) on the pad
//! player's kits. It has the Pads tab (the kit's pads on a compact map, to
//! pick one, copy, paste or clear it, set it up and put the kit's sounds on
//! it), the Wave tab (Start, End, the loop, the fades and the region's
//! settings), the Chop tab (slices by transients, beats, a
//! grid, silence or by hand, put on the kit's pads) and the Play tab (the
//! layers' velocities, the amp envelope, the filter, voices, mute group,
//! pressure); Grain and FX come later. The pad bar's tabs switch between
//! them.
//!
//! - The screen thread owns every kit's project, handed over by
//!   `sampler::Player::take_projects` when the kits are loaded.
//! - The tab edits the playing kit's pad that was hit last, or tapped on the
//!   Pads tab's map. SHIFT + wheel and - / + pick the previous or next pad
//!   with a sound. A pad without a sound shows the Pads tab, where a sound
//!   can be put on it.
//! - An edit changes the project here (drags and turns merge into one undo
//!   step each), then the kit's engine view is built again here and sent to
//!   the audio thread in a box ([`SamplePage::take_view`]). The audio thread
//!   swaps it in and sends the old view back in the same box, to be freed
//!   here. One view travels at a time, and edits made meanwhile go with the
//!   next one, so the audio thread never builds, allocates or frees a view.
//! - A tap on the waveform plays the pad, as a hit would. In Gate, Loop and
//!   Granular modes the sound keeps going until the next tap.
//! - Each finished drag or turn writes one `sample:` line to the log, with
//!   what changed; so does each chop put on pads.
//! - On the Play tab a hit's velocity shows on the layer map, and the wheel
//!   changes the row picked (a turn, like a Q-Link's, is one undo step).
//! - Chop to pads changes the kit like any edit (one undo step), and the
//!   Play page's pad names follow ([`SamplePage::take_renamed`]).
//! - Edits live in memory until the power goes off: nothing is saved yet.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mpfree_sampler::engine::EngineView;
use mpfree_sampler::model::{
    ChannelPick, Fade, LoopMode, Pad, PlayMode, Project, Region, SourceId,
};
use mpfree_sampler_ui::chop_tab::{self, ChopAction, ChopState, ChopTab};
use mpfree_sampler_ui::pads_tab::{self, PadsAction, PadsState, PadsTab};
use mpfree_sampler_ui::play_tab::{self, PlayAction, PlayState, PlayTab};
use mpfree_sampler_ui::wave_tab::{
    self, duration_text, mode_label, WaveAction, WaveState, WaveTab,
};
use mpfree_sampler_ui::SourceView;
use mpfree_ui::widgets::Card;
use mpfree_ui::{Tone, Ui, UiEvent};

use crate::app::bank_letter;
use crate::gui::FromUi;
use crate::sampler::{Playheads, ViewSwap};

/// The Wave tab's page.
pub const PAGE: &str = "Sample";
/// The Chop tab's page.
pub const CHOP_PAGE: &str = "Sample · Chop";
/// The Play tab's page.
pub const PLAY_PAGE: &str = "Sample · Play";
/// The Pads tab's page.
pub const PADS_PAGE: &str = "Sample · Pads";
/// The page shown while the kits load, or while the synth has the pads.
pub const EMPTY_PAGE: &str = "Sample · no sound";
pub const EMPTY: &str = "sample-empty";
pub const EMPTY_LAYOUT: &str = "col(header:60, sample-empty:*, hint:26)";
/// The pad editor's widgets all start with this (the Chop tab's with
/// `smp-chop-`, the Play tab's with `smp-play-`).
pub const PREFIX: &str = "smp-";
const CHOP_PREFIX: &str = "smp-chop-";
const PLAY_PREFIX: &str = "smp-play-";
const PADS_PREFIX: &str = "smp-pads-";
/// A Q-Link resting this long ends its turn: the next one is a new undo step.
const TURN_REST: Duration = Duration::from_secs(1);

/// The region (Wave tab) or pad (Play tab) a drag or turn started from,
/// for its log line.
struct Gesture {
    id: u64,
    kit: usize,
    pad: usize,
    before: Before,
}

enum Before {
    Region(Region),
    Pad(Box<Pad>),
    /// A pad on the Pads tab, which may have had no sound.
    Slot(Option<Box<Pad>>),
}

/// The pad editor's tab on show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Editor {
    Pads,
    Wave,
    Chop,
    Play,
}

pub struct SamplePage {
    projects: Vec<Project>,
    names: Vec<String>,
    rate: u32,
    /// Waveform peaks, tempo and root note, per source, made the first time
    /// a pad using the source is shown.
    sources: HashMap<SourceId, Arc<SourceView>>,
    kit: usize,
    pad: usize,
    tab: Option<WaveTab>,
    state: Option<WaveState>,
    editor: Editor,
    /// The Chop tab, added the first time it is opened.
    chop: Option<ChopTab>,
    chop_state: Option<ChopState>,
    /// The Play tab, added the first time it is opened.
    play: Option<PlayTab>,
    play_state: Option<PlayState>,
    /// The shown pad's last hit, for the Play tab's layer map.
    velocity: Option<u8>,
    /// The Pads tab, added the first time it shows.
    pads: Option<PadsTab>,
    pads_state: Option<PadsState>,
    /// A kit whose pads were renamed or replaced, for the Play page.
    renamed: Option<usize>,
    /// The kit or pad changed since the page was last built.
    stale: bool,
    /// Kits whose new view the audio thread doesn't have yet.
    dirty: Vec<bool>,
    /// A view is on its way to the audio thread (or back).
    in_flight: bool,
    /// The pad a tap on the waveform holds down (Gate, Loop, Granular).
    auditioning: Option<usize>,
    last_turn: Option<Instant>,
    gesture: Option<Gesture>,
    shown_heads: Vec<u64>,
    lines: Vec<String>,
    /// Edits applied, and views sent to the audio thread.
    pub edits: u64,
    pub views_sent: u64,
}

impl SamplePage {
    /// Adds the page for a pad without a sound to `ui` (the Wave tab's page
    /// is added with the first sound shown).
    pub fn add(ui: &mut Ui) -> Result<SamplePage, String> {
        ui.add(EMPTY, Card::new("Sample"));
        ui.add_page(EMPTY_PAGE, EMPTY_LAYOUT)?;
        let mut p = SamplePage {
            projects: Vec::new(),
            names: Vec::new(),
            rate: 44_100,
            sources: HashMap::new(),
            kit: 0,
            pad: 0,
            tab: None,
            state: None,
            editor: Editor::Wave,
            chop: None,
            chop_state: None,
            play: None,
            play_state: None,
            velocity: None,
            pads: None,
            pads_state: None,
            renamed: None,
            stale: true,
            dirty: Vec::new(),
            in_flight: false,
            auditioning: None,
            last_turn: None,
            gesture: None,
            shown_heads: Vec::new(),
            lines: Vec::new(),
            edits: 0,
            views_sent: 0,
        };
        p.show_empty(ui);
        Ok(p)
    }

    /// The kits' projects (in kit order; a kit after them, the synth, has
    /// none) and the rate their engine views are built for. Their undo
    /// history starts here, so UNDO never takes a loaded kit apart.
    pub fn set_kits(&mut self, ui: &mut Ui, mut projects: Vec<Project>, rate: u32) {
        self.lift();
        for p in &mut projects {
            p.clear_history();
        }
        self.dirty = vec![false; projects.len()];
        self.in_flight = false;
        self.projects = projects;
        self.rate = rate;
        self.sources.clear();
        self.pad = self.with_sound(self.pad, 1).unwrap_or(self.pad);
        self.refresh(ui);
    }

    /// The kits' names, for the page and the log.
    pub fn set_names(&mut self, names: Vec<String>) {
        self.names = names;
        self.stale = true;
    }

    /// The kit the pads play now. Builds the page again when it is shown.
    pub fn set_kit(&mut self, k: usize) {
        if k == self.kit {
            return;
        }
        self.lift();
        self.kit = k;
        self.pad = self.with_sound(self.pad, 1).unwrap_or(self.pad);
        self.stale = true;
    }

    /// A pad was hit: the tab edits it from now on.
    pub fn hit(&mut self, pad: usize) {
        if self.auditioning == Some(pad) {
            // Its own release lets it go.
            self.auditioning = None;
        }
        if pad == self.pad || pad >= 16 {
            return;
        }
        self.lift();
        self.pad = pad;
        self.velocity = None;
        self.stale = true;
    }

    /// The pad hit last was hit at `velocity` (after [`SamplePage::hit`]):
    /// the Play tab's layer map shows it.
    pub fn hit_velocity(&mut self, ui: &mut Ui, pad: usize, velocity: u8) {
        self.light(ui, pad, true);
        if pad != self.pad {
            return;
        }
        self.velocity = Some(velocity);
        if !self.stale {
            if let Some(tab) = self.play.as_mut() {
                tab.set_velocity(ui, self.velocity);
            }
        }
    }

    /// The previous (`delta` < 0) or next pad with a sound.
    pub fn step_pad(&mut self, delta: i32) {
        let from = (self.pad as i32 + delta.signum()).rem_euclid(16) as usize;
        if let Some(p) = self.with_sound(from, delta.signum()) {
            self.hit(p);
        }
    }

    /// The first pad from `from` on (wrapping round, in the direction of
    /// `dir`) that has a sound in the kit.
    fn with_sound(&self, from: usize, dir: i32) -> Option<usize> {
        let p = self.projects.get(self.kit)?;
        let dir = if dir < 0 { 15 } else { 1 };
        (0..16)
            .map(|i| (from + i * dir) % 16)
            .find(|&pad| p.pad(pad).is_some_and(|pd| !pd.layers.is_empty()))
    }

    #[cfg(test)]
    pub fn state(&self) -> Option<&WaveState> {
        self.state.as_ref()
    }

    #[cfg(test)]
    pub fn project(&self, kit: usize) -> Option<&Project> {
        self.projects.get(kit)
    }

    #[cfg(test)]
    pub fn chop(&self) -> Option<(&ChopTab, &ChopState)> {
        self.chop.as_ref().zip(self.chop_state.as_ref())
    }

    /// A pad let go: the Pads tab's map stops lighting it.
    pub fn pad_up(&mut self, ui: &mut Ui, pad: usize) {
        self.light(ui, pad, false);
    }

    fn light(&mut self, ui: &mut Ui, pad: usize, down: bool) {
        if let (Some(tab), Some(st)) = (self.pads.as_ref(), self.pads_state.as_ref()) {
            tab.set_lit(ui, st, pad, down);
        }
    }

    #[cfg(test)]
    pub fn pads(&self) -> Option<(&PadsTab, &PadsState)> {
        self.pads.as_ref().zip(self.pads_state.as_ref())
    }

    #[cfg(test)]
    pub fn play(&self) -> Option<(&PlayTab, &PlayState)> {
        self.play.as_ref().zip(self.play_state.as_ref())
    }

    #[cfg(test)]
    pub fn editor(&self) -> Editor {
        self.editor
    }

    /// The page to show: the editor's tab; the Pads tab for a pad without
    /// a sound; or the page that says the kits are loading or the synth
    /// has the pads.
    pub fn page(&self) -> &'static str {
        match self.editor {
            Editor::Chop if self.chop_state.is_some() && self.chop.is_some() => CHOP_PAGE,
            Editor::Play if self.play_state.is_some() && self.play.is_some() => PLAY_PAGE,
            Editor::Wave if self.state.is_some() && self.tab.is_some() => PAGE,
            _ if self.pads_state.is_some() && self.pads.is_some() => PADS_PAGE,
            _ => EMPTY_PAGE,
        }
    }

    /// Builds the page again if the kit or pad changed since. Call it
    /// before showing the page.
    pub fn refresh_if_stale(&mut self, ui: &mut Ui) {
        if self.stale {
            self.refresh(ui);
        }
    }

    fn refresh(&mut self, ui: &mut Ui) {
        self.stale = false;
        self.state = self.build_state();
        let bank = Some(bank_letter(self.kit));
        match (self.state.as_ref(), self.tab.as_mut()) {
            (Some(st), Some(tab)) => {
                tab.bank = bank;
                tab.update(ui, st);
            }
            (Some(st), None) => match WaveTab::add(ui, PAGE, st) {
                Ok(mut tab) => {
                    tab.bank = bank;
                    tab.update(ui, st);
                    self.tab = Some(tab);
                }
                Err(e) => {
                    self.lines.push(format!("sample: no Wave page ({e})"));
                    self.state = None;
                }
            },
            (None, _) => {}
        }
        self.chop_state = None;
        if self.editor == Editor::Chop {
            self.refresh_chop(ui, bank);
        }
        self.play_state = None;
        if self.editor == Editor::Play {
            self.refresh_play(ui, bank);
        }
        self.pads_state = None;
        if self.editor == Editor::Pads || self.state.is_none() {
            self.refresh_pads(ui, bank);
        }
        if self.page() == EMPTY_PAGE {
            self.show_empty(ui);
        }
        self.shown_heads.clear();
        if let Some(tab) = self.tab.as_ref() {
            tab.set_playheads(ui, &[]);
        }
        if let Some(tab) = self.chop.as_ref() {
            tab.set_playheads(ui, &[]);
        }
    }

    fn refresh_chop(&mut self, ui: &mut Ui, bank: Option<char>) {
        let Some(st) = self.state.as_ref() else {
            return;
        };
        let Some(project) = self.projects.get(self.kit) else {
            return;
        };
        self.chop_state = ChopState::from_project(project, st.pad, st.layer, st.source.clone());
        let Some(cs) = self.chop_state.as_ref() else {
            return;
        };
        match self.chop.as_mut() {
            Some(tab) => {
                tab.bank = bank;
                tab.update(ui, cs);
            }
            None => match ChopTab::add(ui, CHOP_PAGE, cs) {
                Ok(mut tab) => {
                    tab.bank = bank;
                    tab.update(ui, cs);
                    self.chop = Some(tab);
                }
                Err(e) => {
                    self.lines.push(format!("sample: no Chop page ({e})"));
                    self.chop_state = None;
                }
            },
        }
    }

    fn refresh_pads(&mut self, ui: &mut Ui, bank: Option<char>) {
        let Some(project) = self.projects.get(self.kit) else {
            return;
        };
        let st = PadsState::from_project(project, self.pad);
        let kit = match self.names.get(self.kit) {
            Some(n) => format!("Kit {} · {n}", self.kit + 1),
            None => format!("Kit {}", self.kit + 1),
        };
        match self.pads.as_mut() {
            Some(tab) => {
                (tab.bank, tab.kit) = (bank, kit);
                tab.update(ui, &st);
            }
            None => match PadsTab::add(ui, PADS_PAGE, &st) {
                Ok(mut tab) => {
                    (tab.bank, tab.kit) = (bank, kit);
                    tab.update(ui, &st);
                    self.pads = Some(tab);
                }
                Err(e) => {
                    self.lines.push(format!("sample: no Pads page ({e})"));
                    return;
                }
            },
        }
        self.pads_state = Some(st);
    }

    fn refresh_play(&mut self, ui: &mut Ui, bank: Option<char>) {
        if self.state.is_none() {
            return;
        }
        let Some(project) = self.projects.get(self.kit) else {
            return;
        };
        let sources = &mut self.sources;
        self.play_state = PlayState::from_project(project, self.pad, |s| {
            sources
                .entry(s.id())
                .or_insert_with(|| Arc::new(SourceView::analyse(s)))
                .clone()
        });
        let Some(ps) = self.play_state.as_ref() else {
            return;
        };
        match self.play.as_mut() {
            Some(tab) => {
                tab.bank = bank;
                tab.update(ui, ps);
                tab.set_velocity(ui, self.velocity);
            }
            None => match PlayTab::add(ui, PLAY_PAGE, ps) {
                Ok(mut tab) => {
                    tab.bank = bank;
                    tab.update(ui, ps);
                    tab.set_velocity(ui, self.velocity);
                    self.play = Some(tab);
                }
                Err(e) => {
                    self.lines.push(format!("sample: no Play page ({e})"));
                    self.play_state = None;
                }
            },
        }
    }

    fn build_state(&mut self) -> Option<WaveState> {
        let p = self.projects.get(self.kit)?;
        let region = &p.pad(self.pad)?.layers.first()?.region;
        let source = p.source(region.source)?;
        let view = self
            .sources
            .entry(source.id())
            .or_insert_with(|| Arc::new(SourceView::analyse(source)))
            .clone();
        WaveState::from_project(p, self.pad, 0, view)
    }

    fn kit_text(&self) -> String {
        self.kit_text_of(self.kit)
    }

    fn kit_text_of(&self, kit: usize) -> String {
        match self.names.get(kit) {
            Some(n) => format!("kit {} ({n})", kit + 1),
            None => format!("kit {}", kit + 1),
        }
    }

    fn pad_text(&self, kit: usize, pad: usize) -> String {
        format!("{}{}", bank_letter(kit), pad % 16 + 1)
    }

    fn show_empty(&mut self, ui: &mut Ui) {
        let (heading, lines): (String, Vec<&str>) = if self.projects.is_empty() {
            (
                "The kits are still loading".into(),
                vec!["Their sounds show here once they are in."],
            )
        } else if self.kit >= self.projects.len() {
            (
                "The pads play the synth".into(),
                vec!["Pick a drum kit on Play (the wheel or a tap) to edit its sounds here."],
            )
        } else {
            (
                format!(
                    "Pad {} has no sound in {}",
                    self.pad_text(self.kit, self.pad),
                    self.kit_text()
                ),
                vec![
                    "Hit a pad with a sound to edit it,",
                    "or turn SHIFT + wheel or press - / + for the next one.",
                ],
            )
        };
        if let Some(c) = ui.get_mut::<Card>(EMPTY) {
            c.heading = heading;
            c.lines = lines
                .into_iter()
                .map(|l| (l.to_string(), Tone::Dim))
                .collect();
        }
    }

    // ------------------------------------------------------------ edits

    /// A touch on one of the editor's widgets. Taps that play the pad go
    /// in `out`, for the audio thread.
    pub fn event(&mut self, ui: &mut Ui, ev: &UiEvent, out: &mut Vec<FromUi>) {
        if ev.widget.starts_with(CHOP_PREFIX) {
            let (Some(tab), Some(st)) = (self.chop.as_mut(), self.chop_state.as_ref()) else {
                return;
            };
            if let Some(a) = tab.action(ui, ev, st) {
                self.act_chop(ui, a);
            }
            return;
        }
        if ev.widget.starts_with(PLAY_PREFIX) {
            let (Some(tab), Some(st)) = (self.play.as_mut(), self.play_state.as_ref()) else {
                return;
            };
            if let Some(a) = tab.action(ui, ev, st) {
                self.act_play(ui, a);
            }
            return;
        }
        if ev.widget.starts_with(PADS_PREFIX) {
            let (Some(tab), Some(st)) = (self.pads.as_mut(), self.pads_state.as_ref()) else {
                return;
            };
            if let Some(a) = tab.action(ui, ev, st) {
                self.act_pads(ui, a, out);
            }
            return;
        }
        let (Some(tab), Some(st)) = (self.tab.as_mut(), self.state.as_ref()) else {
            return;
        };
        if let Some(a) = tab.action(ui, ev, st) {
            self.act(ui, a, out);
        }
    }

    /// Opens the editor's tab `i` (Pads, Wave, Chop or Play; the others
    /// aren't built).
    fn open(&mut self, ui: &mut Ui, i: usize) {
        let editor = match i {
            pads_tab::TAB => Editor::Pads,
            wave_tab::TAB => Editor::Wave,
            chop_tab::TAB => Editor::Chop,
            play_tab::TAB => Editor::Play,
            _ => return,
        };
        if editor != self.editor {
            self.lift();
            self.editor = editor;
            self.refresh(ui);
        }
    }

    fn act_chop(&mut self, ui: &mut Ui, a: ChopAction) {
        match &a {
            ChopAction::Tab(i) => self.open(ui, *i),
            ChopAction::ToPads {
                pad, bounds, mode, ..
            } => {
                let kit = self.kit;
                let Some(project) = self.projects.get_mut(kit) else {
                    return;
                };
                let name = project.pad(*pad).map_or(String::new(), |p| p.name.clone());
                match a.apply(project) {
                    Ok(pads) => {
                        self.edits += 1;
                        if let Some(d) = self.dirty.get_mut(kit) {
                            *d = true;
                        }
                        self.renamed = Some(kit);
                        let n = bounds.len() - 1;
                        let to = if n == 1 {
                            self.pad_text(kit, pads.start)
                        } else {
                            format!(
                                "{} to {}",
                                self.pad_text(kit, pads.start),
                                self.pad_text(kit, pads.end - 1)
                            )
                        };
                        let s = |f: u64| format!("{:.3} s", f as f64 / f64::from(self.rate.max(1)));
                        let span = format!("{} to {}", s(bounds[0]), s(bounds[n]));
                        self.lines.push(format!("sample: {} pad {} {name}: chopped {span} into {n} slice{} ({mode:?}) on \
                                                 pads {to}", self.kit_text_of(kit), self.pad_text(kit, *pad),
                                                if n == 1 { "" } else { "s" }));
                    }
                    Err(e) => self.lines.push(format!(
                        "sample: chop of pad {} not made ({e:?})",
                        self.pad_text(kit, *pad)
                    )),
                }
                self.refresh(ui);
            }
        }
    }

    fn act_pads(&mut self, ui: &mut Ui, a: PadsAction, out: &mut Vec<FromUi>) {
        if !matches!(a, PadsAction::Tab(_)) {
            // Working on the Pads tab (shown for a pad without a sound, too)
            // keeps it open.
            self.editor = Editor::Pads;
        }
        match &a {
            PadsAction::Tab(i) => self.open(ui, *i),
            PadsAction::Pick(pad) => {
                self.hit(*pad);
                self.refresh(ui);
            }
            PadsAction::Audition { pad, down } => out.push(FromUi::Pad {
                pad: *pad,
                down: *down,
            }),
            PadsAction::Set { pad, gesture, .. } => {
                let (kit, pad, gesture) = (self.kit, *pad, *gesture);
                let before = self
                    .projects
                    .get(kit)
                    .and_then(|p| p.pad(pad))
                    .cloned()
                    .map(Box::new);
                if self.gesture.as_ref().is_some_and(|g| Some(g.id) != gesture) {
                    self.lift();
                }
                let one_step = gesture.is_none();
                if self.gesture.is_none() {
                    self.gesture = Some(Gesture {
                        id: gesture.unwrap_or(u64::MAX),
                        kit,
                        pad,
                        before: Before::Slot(before),
                    });
                }
                let Some(project) = self.projects.get_mut(kit) else {
                    return;
                };
                match a.apply(project) {
                    Ok(()) => {
                        self.edits += 1;
                        if let Some(d) = self.dirty.get_mut(kit) {
                            *d = true;
                        }
                        self.renamed = Some(kit);
                    }
                    Err(e) => self.lines.push(format!(
                        "sample: edit of pad {} not made ({e:?})",
                        self.pad_text(kit, pad)
                    )),
                }
                if one_step {
                    // Copy, paste, clear and put are a step each: log it now.
                    self.lift();
                }
                self.refresh(ui);
            }
        }
    }

    fn act_play(&mut self, ui: &mut Ui, a: PlayAction) {
        match &a {
            PlayAction::Tab(i) => self.open(ui, *i),
            PlayAction::Pad { pad, gesture, .. } => {
                let (kit, pad, gesture) = (self.kit, *pad, *gesture);
                if self.gesture.as_ref().is_some_and(|g| g.id != gesture) {
                    self.lift();
                }
                if self.gesture.is_none() {
                    if let Some(st) = self.play_state.as_ref() {
                        self.gesture = Some(Gesture {
                            id: gesture,
                            kit,
                            pad,
                            before: Before::Pad(Box::new(st.settings.clone())),
                        });
                    }
                }
                let Some(project) = self.projects.get_mut(kit) else {
                    return;
                };
                match a.apply(project) {
                    Ok(()) => {
                        self.edits += 1;
                        if let Some(d) = self.dirty.get_mut(kit) {
                            *d = true;
                        }
                    }
                    Err(e) => self.lines.push(format!(
                        "sample: edit of pad {} not made ({e:?})",
                        self.pad_text(kit, pad)
                    )),
                }
                self.refresh(ui);
            }
        }
    }

    /// The kit whose pads were renamed or replaced since the last call, with
    /// its pads' names, for the Play page.
    pub fn take_renamed(&mut self) -> Option<(usize, Vec<Option<String>>)> {
        let kit = self.renamed.take()?;
        let p = self.projects.get(kit)?;
        Some((
            kit,
            (0..16)
                .map(|i| {
                    p.pad(i)
                        .filter(|pd| !pd.layers.is_empty())
                        .map(|pd| pd.name.clone())
                })
                .collect(),
        ))
    }

    /// Q-Link `knob` (0-3) turned by `ticks` detents; `fine` (SHIFT held)
    /// turns finer. On Wave they move Start, End, Loop start and Loop end
    /// (one frame a detent with SHIFT); on Chop the slice, its start, the
    /// setting and the first pad; on Play Attack, Decay, Cutoff and
    /// Resonance.
    pub fn qlink(&mut self, ui: &mut Ui, knob: usize, ticks: i32, fine: bool) {
        self.turned();
        if self.page() == PADS_PAGE {
            let (Some(tab), Some(st)) = (self.pads.as_mut(), self.pads_state.as_ref()) else {
                return;
            };
            if let Some(a) = tab.qlink(ui, st, knob, ticks) {
                self.act_pads(ui, a, &mut Vec::new());
            }
            return;
        }
        if self.editor == Editor::Play {
            let (Some(tab), Some(st)) = (self.play.as_mut(), self.play_state.as_ref()) else {
                return;
            };
            if let Some(a) = tab.qlink(ui, st, knob, ticks, fine) {
                self.act_play(ui, a);
            }
            return;
        }
        if self.editor == Editor::Chop {
            if let (Some(tab), Some(st)) = (self.chop.as_mut(), self.chop_state.as_ref()) {
                tab.qlink(ui, st, knob, ticks, fine);
            }
            return;
        }
        let (Some(tab), Some(st)) = (self.tab.as_mut(), self.state.as_ref()) else {
            return;
        };
        if let Some(a) = tab.qlink(ui, st, knob, ticks, fine) {
            self.act(ui, a, &mut Vec::new());
        }
    }

    /// A Q-Link or the wheel turned: a turn after a rest is a new undo step.
    fn turned(&mut self) {
        let now = Instant::now();
        if self
            .last_turn
            .is_some_and(|t| now.duration_since(t) >= TURN_REST)
        {
            self.lift();
        }
        self.last_turn = Some(now);
    }

    /// The data wheel: zoom round the marker last moved or the slice picked
    /// (Wave, Chop), or change the row picked (Play).
    pub fn wheel(&mut self, ui: &mut Ui, delta: i32) {
        if self.page() == PADS_PAGE {
            self.turned();
            let (Some(tab), Some(st)) = (self.pads.as_mut(), self.pads_state.as_ref()) else {
                return;
            };
            if let Some(a) = tab.wheel(ui, st, delta) {
                self.act_pads(ui, a, &mut Vec::new());
            }
            return;
        }
        match self.editor {
            Editor::Pads => {}
            Editor::Play => {
                self.turned();
                let (Some(tab), Some(st)) = (self.play.as_mut(), self.play_state.as_ref()) else {
                    return;
                };
                if let Some(a) = tab.wheel(ui, st, delta) {
                    self.act_play(ui, a);
                }
            }
            Editor::Chop => {
                if let (Some(tab), Some(st)) = (self.chop.as_mut(), self.chop_state.as_ref()) {
                    tab.wheel(ui, st, delta);
                }
            }
            Editor::Wave => {
                if let (Some(tab), Some(st)) = (self.tab.as_mut(), self.state.as_ref()) {
                    tab.wheel(ui, st, delta);
                }
            }
        }
    }

    fn act(&mut self, ui: &mut Ui, a: WaveAction, out: &mut Vec<FromUi>) {
        match a {
            WaveAction::Region { pad, gesture, .. } => {
                let kit = self.kit;
                if self.gesture.as_ref().is_some_and(|g| g.id != gesture) {
                    self.lift();
                }
                if self.gesture.is_none() {
                    if let Some(st) = self.state.as_ref() {
                        self.gesture = Some(Gesture {
                            id: gesture,
                            kit,
                            pad,
                            before: Before::Region(st.region.clone()),
                        });
                    }
                }
                let Some(project) = self.projects.get_mut(kit) else {
                    return;
                };
                match a.apply(project) {
                    Ok(()) => {
                        self.edits += 1;
                        if let Some(d) = self.dirty.get_mut(kit) {
                            *d = true;
                        }
                    }
                    Err(e) => self.lines.push(format!(
                        "sample: edit of pad {} not made ({e:?})",
                        self.pad_text(kit, pad)
                    )),
                }
                self.refresh(ui);
            }
            WaveAction::Audition { pad } => self.audition(pad, out),
            WaveAction::Tab(i) => self.open(ui, i),
        }
    }

    fn audition(&mut self, pad: usize, out: &mut Vec<FromUi>) {
        if let Some(held) = self.auditioning.take() {
            out.push(FromUi::Pad {
                pad: held,
                down: false,
            });
            return;
        }
        let held = self.state.as_ref().is_some_and(|st| {
            matches!(
                st.region.mode,
                PlayMode::Gate | PlayMode::Loop | PlayMode::Granular
            )
        });
        out.push(FromUi::Pad { pad, down: true });
        if held {
            self.auditioning = Some(pad);
        } else {
            out.push(FromUi::Pad { pad, down: false });
        }
    }

    /// A finger lifted or a Q-Link was let go: the drag or turn is over,
    /// so the next edit is a new undo step. Writes its log line.
    pub fn lift(&mut self) {
        if let Some(tab) = self.tab.as_mut() {
            tab.lift();
        }
        if let Some(tab) = self.play.as_mut() {
            tab.lift();
        }
        if let Some(tab) = self.pads.as_mut() {
            tab.lift();
        }
        self.last_turn = None;
        let Some(g) = self.gesture.take() else { return };
        if let Before::Slot(before) = &g.before {
            let after = self.projects.get(g.kit).and_then(|p| p.pad(g.pad));
            let what = describe_slot(before.as_deref(), after);
            if !what.is_empty() {
                let kit = self.kit_text_of(g.kit);
                self.lines.push(format!(
                    "sample: {kit} pad {}: {what}",
                    self.pad_text(g.kit, g.pad)
                ));
            }
            return;
        }
        let Some(after) = self.projects.get(g.kit).and_then(|p| p.pad(g.pad)) else {
            return;
        };
        let what = match &g.before {
            Before::Region(before) => {
                let Some(l) = after.layers.first() else {
                    return;
                };
                describe(before, &l.region, self.rate)
            }
            Before::Pad(before) => play_tab::describe(before, after),
            Before::Slot(_) => String::new(),
        };
        if !what.is_empty() {
            let name = self.projects[g.kit]
                .pad(g.pad)
                .map_or(String::new(), |p| p.name.clone());
            let kit = self.kit_text_of(g.kit);
            self.lines.push(format!(
                "sample: {kit} pad {} {name}: {what}",
                self.pad_text(g.kit, g.pad)
            ));
        }
    }

    /// Leaving the page: let go of a pad a tap holds, end the gesture.
    pub fn leave(&mut self, out: &mut Vec<FromUi>) {
        if let Some(pad) = self.auditioning.take() {
            out.push(FromUi::Pad { pad, down: false });
        }
        self.lift();
    }

    /// UNDO (or redo) in the playing kit.
    pub fn undo(&mut self, ui: &mut Ui, redo: bool) {
        self.lift();
        let kit = self.kit;
        let what = if redo { "redo" } else { "undo" };
        let Some(p) = self.projects.get_mut(kit) else {
            self.lines.push(format!(
                "sample: nothing to {what} (the synth has the pads)"
            ));
            return;
        };
        if if redo { p.redo() } else { p.undo() } {
            if let Some(d) = self.dirty.get_mut(kit) {
                *d = true;
            }
            self.renamed = Some(kit);
            self.lines
                .push(format!("sample: {what} in {}", self.kit_text()));
            self.refresh(ui);
        } else {
            self.lines
                .push(format!("sample: nothing to {what} in {}", self.kit_text()));
        }
    }

    // ------------------------------------------------------------ the audio thread

    /// The next kit view for the audio thread, if a kit was edited and no
    /// view is on its way already. Built here, off the audio thread.
    pub fn take_view(&mut self) -> Option<FromUi> {
        if self.in_flight {
            return None;
        }
        let kit = self.dirty.iter().position(|d| *d)?;
        self.dirty[kit] = false;
        self.in_flight = true;
        self.views_sent += 1;
        let view = EngineView::build(&self.projects[kit], self.rate);
        Some(FromUi::SampleView(Box::new(ViewSwap {
            kit,
            view: Some(view),
        })))
    }

    /// The audio thread is done with a view: the box holds the one it
    /// replaced, freed here.
    pub fn view_back(&mut self, swap: Box<ViewSwap>) {
        self.in_flight = false;
        drop(swap);
    }

    /// Kit `kit`'s view found no room on its way: build it again next time.
    pub fn unsent(&mut self, kit: usize) {
        self.in_flight = false;
        self.views_sent -= 1;
        if let Some(d) = self.dirty.get_mut(kit) {
            *d = true;
        }
    }

    /// Where the playing kit's voices are; the shown pad's are drawn.
    pub fn set_playheads(&mut self, ui: &mut Ui, heads: &Playheads) {
        let frames: Vec<u64> = if usize::from(heads.kit) == self.kit && self.state.is_some() {
            heads.of(self.pad).collect()
        } else {
            Vec::new()
        };
        if frames == self.shown_heads {
            return;
        }
        match self.editor {
            Editor::Wave => {
                if let Some(tab) = self.tab.as_ref() {
                    tab.set_playheads(ui, &frames);
                }
            }
            Editor::Chop => {
                if let Some(tab) = self.chop.as_ref() {
                    tab.set_playheads(ui, &frames);
                }
            }
            // The Play and Pads tabs draw no playheads.
            Editor::Play | Editor::Pads => {}
        }
        self.shown_heads = frames;
    }

    /// Lines for the log since the last call.
    pub fn take_lines(&mut self) -> Vec<String> {
        std::mem::take(&mut self.lines)
    }
}

/// What a Pads tab edit did to a pad, for the log: "now plays Hat closed",
/// "cleared (was Break 92)", "Break 92 -> Kick 58", or the settings that
/// changed. Empty if nothing did.
pub fn describe_slot(before: Option<&Pad>, after: Option<&Pad>) -> String {
    match (before, after) {
        (None, None) => String::new(),
        (None, Some(a)) => format!("now plays {}", a.name),
        (Some(b), None) => format!("cleared (was {})", b.name),
        (Some(b), Some(a))
            if b.layers.first().map(|l| l.region.source)
                != a.layers.first().map(|l| l.region.source) =>
        {
            format!("{} -> {}", b.name, a.name)
        }
        (Some(b), Some(a)) => {
            let mut what = Vec::new();
            if b.layers.first().map(|l| l.region.mode) != a.layers.first().map(|l| l.region.mode) {
                let m = |p: &Pad| p.layers.first().map_or("", |l| mode_label(l.region.mode));
                what.push(format!("mode {} -> {}", m(b), m(a)));
            }
            let settings = play_tab::describe(b, a);
            if !settings.is_empty() {
                what.push(settings);
            }
            if what.is_empty() && b != a {
                what.push(format!("now a copy of {}", a.name));
            }
            what.join(", ")
        }
    }
}

/// What changed between two versions of a region, for the log: "Start
/// 0.000 s -> 0.023 s, gain 0.0 -> +2.0 dB". Empty if nothing did.
pub fn describe(a: &Region, b: &Region, rate: u32) -> String {
    let s = |f: u64| format!("{:.3} s", f as f64 / f64::from(rate.max(1)));
    let fade = |f: &Fade| duration_text(u64::from(f.frames), rate);
    let channels = |c: ChannelPick| match c {
        ChannelPick::Stereo => "stereo",
        ChannelPick::MonoSum => "mono sum",
        ChannelPick::Left => "left",
        ChannelPick::Right => "right",
    };
    let mut out = Vec::new();
    if a.mode != b.mode {
        out.push(format!(
            "mode {} -> {}",
            mode_label(a.mode),
            mode_label(b.mode)
        ));
    }
    if a.start != b.start {
        out.push(format!("Start {} -> {}", s(a.start), s(b.start)));
    }
    if a.end != b.end {
        out.push(format!("End {} -> {}", s(a.end), s(b.end)));
    }
    match (a.sustain_loop, b.sustain_loop) {
        (None, Some(l)) => out.push(format!("loop on, {} to {}", s(l.start), s(l.end))),
        (Some(_), None) => out.push("loop off".to_string()),
        (Some(x), Some(y)) => {
            if x.start != y.start {
                out.push(format!("Loop start {} -> {}", s(x.start), s(y.start)));
            }
            if x.end != y.end {
                out.push(format!("Loop end {} -> {}", s(x.end), s(y.end)));
            }
            if x.mode != y.mode {
                let m = |l: LoopMode| {
                    if l == LoopMode::PingPong {
                        "ping-pong"
                    } else {
                        "forward"
                    }
                };
                out.push(format!("loop {} -> {}", m(x.mode), m(y.mode)));
            }
            if x.crossfade != y.crossfade {
                out.push(format!(
                    "crossfade {} -> {}",
                    duration_text(u64::from(x.crossfade), rate),
                    duration_text(u64::from(y.crossfade), rate)
                ));
            }
        }
        (None, None) => {}
    }
    if a.fade_in != b.fade_in {
        out.push(format!(
            "fade in {} -> {}",
            fade(&a.fade_in),
            fade(&b.fade_in)
        ));
    }
    if a.fade_out != b.fade_out {
        out.push(format!(
            "fade out {} -> {}",
            fade(&a.fade_out),
            fade(&b.fade_out)
        ));
    }
    if a.gain_db != b.gain_db {
        out.push(format!("gain {:+.1} -> {:+.1} dB", a.gain_db, b.gain_db));
    }
    if a.tune_cents != b.tune_cents {
        out.push(format!(
            "tune {:+} -> {:+} cents",
            a.tune_cents, b.tune_cents
        ));
    }
    if a.reverse != b.reverse {
        out.push(
            if b.reverse {
                "reverse on"
            } else {
                "reverse off"
            }
            .to_string(),
        );
    }
    if a.channels != b.channels {
        out.push(format!(
            "channels {} -> {}",
            channels(a.channels),
            channels(b.channels)
        ));
    }
    out.join(", ")
}
