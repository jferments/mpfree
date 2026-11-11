//! The Sample tab: the sampler's pad editor (`mpfree-sampler-ui`) on the pad
//! player's kits. It has the Pads tab (the kit's pads on a compact map, to
//! pick one, copy, paste or clear it, set it up and put the kit's sounds on
//! it), the Wave tab (Start, End, the loop, the fades and the region's
//! settings), the Chop tab (slices by transients, beats, a
//! grid, silence or by hand, put on the kit's pads) and the Play tab (the
//! layers' velocities, the amp envelope, the filter, voices, mute group,
//! pressure) and the Grain tab (the first layer's grain settings, the
//! engine's grains over the sound, a touch pad for position and size) and
//! the Rec tab (the line input onto a pad); FX comes later. The pad bar's
//! tabs switch between them.
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
//! - On the Grain tab the cloud shows the grains the audio thread plays for
//!   the pad: one boxed [`Cloud`] goes there, is filled and comes back
//!   about 25 times a second, only while the tab shows
//!   ([`SamplePage::take_cloud`], [`SamplePage::cloud_back`]).
//! - The Rec tab records the line input onto a pad. While the line input
//!   is on, the page keeps the sampler's input (the last 75 s, about
//!   26 MB) and sends its writing end to the audio thread
//!   ([`SamplePage::take_input`]); turned off, the writer comes back and
//!   the buffer is freed here ([`SamplePage::input_back`]). A take keeps
//!   recording while other pages show ([`SamplePage::tick`]), and lands on
//!   its pad as one undo step.
//! - Edits live in memory until the power goes off: nothing is saved yet.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mpfree_sampler::engine::EngineView;
use mpfree_sampler::model::{
    ChannelPick, Fade, LoopMode, Pad, PadsPlay, PlayMode, Project, Region, Source, SourceId,
};
use mpfree_sampler::record::{self, Input, Progress, Recording};
use mpfree_sampler::warp::{CopyKey, Done, Warps, Worker};
use mpfree_sampler_ui::chop_tab::{self, ChopAction, ChopState, ChopTab};
use mpfree_sampler_ui::grain_tab::{self, GrainAction, GrainState, GrainTab};
use mpfree_sampler_ui::pads_tab::{self, PadsAction, PadsState, PadsTab};
use mpfree_sampler_ui::play_tab::{self, PlayAction, PlayState, PlayTab};
use mpfree_sampler_ui::rec_tab::{self, RecAction, RecState, RecTab, Skip};
use mpfree_sampler_ui::wave_tab::{
    self, duration_text, mode_label, WaveAction, WaveState, WaveTab,
};
use mpfree_sampler_ui::SourceView;
use mpfree_ui::widgets::Card;
use mpfree_ui::{Tone, Ui, UiEvent};

use crate::app::bank_letter;
use crate::gui::FromUi;
use crate::sampler::{Cloud, Playheads, RecFeed, ViewSwap};

/// The Wave tab's page.
pub const PAGE: &str = "Sample";
/// The Chop tab's page.
pub const CHOP_PAGE: &str = "Sample · Chop";
/// The Play tab's page.
pub const PLAY_PAGE: &str = "Sample · Play";
/// The Pads tab's page.
pub const PADS_PAGE: &str = "Sample · Pads";
/// The Grain tab's page.
pub const GRAIN_PAGE: &str = "Sample · Grain";
/// The Rec tab's page.
pub const REC_PAGE: &str = "Sample · Rec";
/// The line input the sampler keeps while it is on: enough for the
/// longest Skip back (a minute, or 8 bars down to 60 BPM) and more.
pub const INPUT_SECONDS: u32 = 75;
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
const GRAIN_PREFIX: &str = "smp-grain-";
const REC_PREFIX: &str = "smp-rec-";
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
    /// A layer's region on the Grain tab.
    Grain {
        layer: usize,
        region: Box<Region>,
    },
    Pad(Box<Pad>),
    /// A pad on the Pads tab, which may have had no sound.
    Slot(Option<Box<Pad>>),
}

/// What a Play page Q-Link turns on the pad hit last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadKnob {
    /// Semitones (SHIFT: 10 cents).
    Tune,
    /// The amp envelope's decay to silence; at the top the sound plays out.
    Decay,
}

/// The pad editor's tab on show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Editor {
    Pads,
    Wave,
    Chop,
    Play,
    Grain,
    Rec,
}

/// A take being recorded, and the pad it goes onto.
struct Take {
    rec: Recording,
    kit: usize,
    pad: usize,
    pick: record::Pick,
}

/// The sampler's input buffer's size: stereo `f32`.
pub fn input_bytes(rate: u32) -> u64 {
    u64::from(INPUT_SECONDS) * u64::from(rate) * 2 * 4
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
    /// The Grain tab, added the first time it is opened.
    grain: Option<GrainTab>,
    grain_state: Option<GrainState>,
    /// The cloud's box, while it is here and not on the audio thread.
    cloud: Option<Box<Cloud>>,
    /// The last cloud shown had grains, a playhead or grains on other pads.
    cloud_shown: bool,
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
    /// A Play page Q-Link turning a pad: the pad, the knob and the turn's
    /// undo id (one undo step a turn).
    play_turn: Option<(usize, PadKnob, u64)>,
    shown_heads: Vec<u64>,
    lines: Vec<String>,
    /// Edits applied, and views sent to the audio thread.
    pub edits: u64,
    pub views_sent: u64,
    /// The project tempo and the stretched copies warped pads play.
    warps: Warps,
    /// The thread that makes them, started the first time one is wanted.
    stretcher: Option<Worker>,
    /// Copies asked of it and not back yet.
    stretching: Vec<CopyKey>,
    /// The Rec tab, added the first time it is opened.
    rec: Option<RecTab>,
    rec_state: Option<RecState>,
    /// The line input is on (its row on the Looper tab's MENU).
    line_on: bool,
    /// The sampler's input while the line input is on, and its writing end
    /// until it goes to the audio thread.
    input: Option<Input>,
    feed: Option<Box<RecFeed>>,
    /// The audio thread has the writer, and should hand it back.
    feeding: bool,
    want_back: bool,
    take: Option<Take>,
    /// Takes made, for their names.
    takes: u32,
    /// The last take, said in a line on the tab.
    last_take: String,
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
            grain: None,
            grain_state: None,
            cloud: Some(Box::default()),
            cloud_shown: false,
            renamed: None,
            stale: true,
            dirty: Vec::new(),
            in_flight: false,
            auditioning: None,
            last_turn: None,
            gesture: None,
            play_turn: None,
            shown_heads: Vec::new(),
            lines: Vec::new(),
            edits: 0,
            views_sent: 0,
            warps: Warps::default(),
            stretcher: None,
            stretching: Vec::new(),
            rec: None,
            rec_state: None,
            line_on: false,
            input: None,
            feed: None,
            feeding: false,
            want_back: false,
            take: None,
            takes: 0,
            last_take: String::new(),
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
        self.warps.clear();
        self.cancel_stretching();
        if self.take.take().is_some() {
            self.lines
                .push("sample: the take was dropped (the kits were loaded again)".to_string());
        }
        // Kits with warped pads get their views at the project tempo.
        for k in 0..self.projects.len() {
            self.dirty[k] = self.warped(k);
        }
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

    /// A pad was hit: the tab edits it (or, with the pads playing notes,
    /// the keygroup that plays its note) from now on.
    pub fn hit(&mut self, pad: usize) {
        if self.auditioning == Some(pad) {
            // Its own release lets it go.
            self.auditioning = None;
        }
        let pad = self.played(pad);
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
        let pad = self.played(pad);
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
        let pad = self.played(pad);
        self.light(ui, pad, false);
    }

    /// The pad that sounds when pad `pad` is hit: itself; with the kit's
    /// pads playing notes, the first keygroup holding its note; with 16
    /// Levels, the pad they play.
    fn played(&self, pad: usize) -> usize {
        let Some(p) = self.projects.get(self.kit) else {
            return pad;
        };
        match p.pads_play() {
            PadsPlay::Keys { from } if pad < 16 => p
                .keygroups_for(from.saturating_add(pad as u8))
                .first()
                .copied()
                .unwrap_or(pad),
            PadsPlay::Levels { pad: of, .. } if pad < 16 => usize::from(of),
            _ => pad,
        }
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
    pub fn grain(&self) -> Option<(&GrainTab, &GrainState)> {
        self.grain.as_ref().zip(self.grain_state.as_ref())
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
            Editor::Grain if self.grain_state.is_some() && self.grain.is_some() => GRAIN_PAGE,
            Editor::Rec if self.rec_state.is_some() && self.rec.is_some() => REC_PAGE,
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
        self.grain_state = None;
        if self.editor == Editor::Grain {
            self.refresh_grain(ui, bank);
        }
        self.rec_state = None;
        if self.editor == Editor::Rec {
            self.refresh_rec(ui, bank);
        }
        self.pads_state = None;
        if self.editor == Editor::Pads || self.state.is_none() && self.editor != Editor::Rec {
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

    fn refresh_grain(&mut self, ui: &mut Ui, bank: Option<char>) {
        if self.state.is_none() {
            return;
        }
        let Some(project) = self.projects.get(self.kit) else {
            return;
        };
        let sources = &mut self.sources;
        self.grain_state = GrainState::from_project(project, self.pad, 0, |s| {
            sources
                .entry(s.id())
                .or_insert_with(|| Arc::new(SourceView::analyse(s)))
                .clone()
        });
        let Some(gs) = self.grain_state.as_ref() else {
            return;
        };
        match self.grain.as_mut() {
            Some(tab) => {
                tab.bank = bank;
                tab.update(ui, gs);
            }
            None => match GrainTab::add(ui, GRAIN_PAGE, gs) {
                Ok(mut tab) => {
                    tab.bank = bank;
                    tab.update(ui, gs);
                    self.grain = Some(tab);
                }
                Err(e) => {
                    self.lines.push(format!("sample: no Grain page ({e})"));
                    self.grain_state = None;
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
                self.act_chop(ui, a, out);
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
        if ev.widget.starts_with(GRAIN_PREFIX) {
            let (Some(tab), Some(st)) = (self.grain.as_mut(), self.grain_state.as_ref()) else {
                return;
            };
            if let Some(a) = tab.action(ui, ev, st) {
                self.act_grain(ui, a);
            }
            return;
        }
        if ev.widget.starts_with(REC_PREFIX) {
            let (Some(tab), Some(st)) = (self.rec.as_mut(), self.rec_state.as_ref()) else {
                return;
            };
            if let Some(a) = tab.action(ui, ev, st) {
                self.act_rec(ui, a);
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

    /// Opens the editor's tab `i` (Pads, Wave, Chop, Play, Grain or Rec;
    /// FX isn't built).
    fn open(&mut self, ui: &mut Ui, i: usize) {
        let editor = match i {
            pads_tab::TAB => Editor::Pads,
            wave_tab::TAB => Editor::Wave,
            chop_tab::TAB => Editor::Chop,
            play_tab::TAB => Editor::Play,
            grain_tab::TAB => Editor::Grain,
            rec_tab::TAB => Editor::Rec,
            _ => return,
        };
        if editor != self.editor {
            self.lift();
            self.editor = editor;
            self.refresh(ui);
        }
    }

    fn act_chop(&mut self, ui: &mut Ui, a: ChopAction, out: &mut Vec<FromUi>) {
        match &a {
            ChopAction::Tab(i) => self.open(ui, *i),
            &ChopAction::Audition {
                pad,
                layer,
                start,
                end,
            } => {
                out.push(FromUi::Slice {
                    kit: self.kit,
                    pad,
                    layer,
                    start,
                    end,
                });
            }
            ChopAction::ToPads {
                pad,
                bounds,
                mode,
                keys_from,
                ..
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
                        let to = match keys_from {
                            Some(k) => format!(
                                "keys {} to {} (pads {to}; the pads now play notes)",
                                mpfree_sampler_ui::note_name(*k),
                                mpfree_sampler_ui::note_name(k.saturating_add(n as u8 - 1))
                            ),
                            None => format!("pads {to}"),
                        };
                        self.lines.push(format!("sample: {} pad {} {name}: chopped {span} into {n} slice{} ({mode:?}) on \
                                                 {to}", self.kit_text_of(kit), self.pad_text(kit, *pad),
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
            PlayAction::PadsPlay(play) => {
                self.lift();
                let kit = self.kit;
                let Some(project) = self.projects.get_mut(kit) else {
                    return;
                };
                let what = match a.apply(project) {
                    Ok(()) => {
                        self.edits += 1;
                        if let Some(d) = self.dirty.get_mut(kit) {
                            *d = true;
                        }
                        match play {
                            PadsPlay::Pads => "the pads play their own sounds".to_string(),
                            PadsPlay::Keys { from } => format!(
                                "the pads play notes, pad 1 {}",
                                mpfree_sampler_ui::note_name(*from)
                            ),
                            PadsPlay::Levels { pad, by } => format!(
                                "the pads play 16 Levels of pad {} by {}",
                                self.pad_text(kit, usize::from(*pad)),
                                by.label().to_lowercase()
                            ),
                        }
                    }
                    Err(e) => format!("pads play not changed ({e:?})"),
                };
                let kit = self.kit_text_of(kit);
                self.lines.push(format!("sample: {kit}: {what}"));
                self.refresh(ui);
            }
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

    fn act_grain(&mut self, ui: &mut Ui, a: GrainAction) {
        match &a {
            GrainAction::Tab(i) => self.open(ui, *i),
            GrainAction::Region {
                pad,
                layer,
                gesture,
                ..
            } => {
                let (kit, pad, layer, gesture) = (self.kit, *pad, *layer, *gesture);
                if self.gesture.as_ref().is_some_and(|g| g.id != gesture) {
                    self.lift();
                }
                if self.gesture.is_none() {
                    if let Some(st) = self.grain_state.as_ref() {
                        self.gesture = Some(Gesture {
                            id: gesture,
                            kit,
                            pad,
                            before: Before::Grain {
                                layer,
                                region: Box::new(st.region.clone()),
                            },
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
    /// Resonance; on Grain position, size, density and pitch (with SHIFT
    /// the sprays, and size and density finer).
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
        if self.editor == Editor::Grain {
            let (Some(tab), Some(st)) = (self.grain.as_mut(), self.grain_state.as_ref()) else {
                return;
            };
            if let Some(a) = tab.qlink(ui, st, knob, ticks, fine) {
                self.act_grain(ui, a);
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

    /// The pad's tuning (semitones) and decay (None: it plays out) in the
    /// playing kit, for the Play page's Q-Links. None when the kit has no
    /// sound on the pad, or the synth has the pads.
    pub fn pad_knobs(&self, pad: usize) -> Option<(f32, Option<f32>)> {
        let p = self.projects.get(self.kit)?.pad(pad)?;
        let decay = (p.amp.sustain < 0.999).then_some(p.amp.decay_ms);
        Some((p.tune_cents as f32 / 100.0, decay))
    }

    /// A Play page Q-Link on `pad` of the playing kit: its tuning or its
    /// decay, by `d` detents (`fine`: SHIFT held). One undo step a turn,
    /// as on the Play tab; the kit's new view goes to the audio thread.
    pub fn turn_pad(&mut self, pad: usize, which: PadKnob, d: i32, fine: bool) {
        self.turned();
        let kit = self.kit;
        let Some(before) = self.projects.get(kit).and_then(|p| p.pad(pad)).cloned() else {
            return;
        };
        if self
            .play_turn
            .is_some_and(|(p, w, _)| (p, w) != (pad, which))
        {
            self.lift();
        }
        let id = match self.play_turn {
            Some((_, _, id)) => id,
            None => {
                // "PLAY" and the edit count: a new undo step.
                let id = 0x504C_4159_0000_0000 | (self.edits & 0xFFFF_FFFF);
                self.play_turn = Some((pad, which, id));
                self.gesture = Some(Gesture {
                    id,
                    kit,
                    pad,
                    before: Before::Pad(Box::new(before)),
                });
                id
            }
        };
        self.last_turn = Some(Instant::now());
        let edit = |p: &mut Pad| match which {
            PadKnob::Tune => {
                let step = if fine { 10 } else { 100 };
                p.tune_cents = (p.tune_cents + d * step).clamp(-2400, 2400);
            }
            PadKnob::Decay => {
                let max = crate::play_page::DECAY_MAX_MS;
                let open = p.amp.sustain >= 0.999;
                let ms = if open { max } else { p.amp.decay_ms.max(10.0) };
                let per = if fine { 32.0 } else { 8.0 };
                let to = (ms * (d as f32 / per).exp2()).round().clamp(10.0, max);
                if to >= max && d > 0 {
                    p.amp.sustain = 1.0;
                    p.amp.decay_ms = 0.0;
                } else {
                    p.amp.sustain = 0.0;
                    p.amp.decay_ms = to;
                }
            }
        };
        match self.projects[kit].edit_pad(pad, Some(id), edit) {
            Ok(()) => {
                self.edits += 1;
                if let Some(dirty) = self.dirty.get_mut(kit) {
                    *dirty = true;
                }
                self.stale = true;
            }
            Err(e) => self.lines.push(format!(
                "sample: pad {} not changed ({e:?})",
                self.pad_text(kit, pad)
            )),
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
    /// (Wave, Chop), or change the row picked (Play, Grain).
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
            Editor::Grain => {
                self.turned();
                let (Some(tab), Some(st)) = (self.grain.as_mut(), self.grain_state.as_ref()) else {
                    return;
                };
                if let Some(a) = tab.wheel(ui, st, delta) {
                    self.act_grain(ui, a);
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
            Editor::Rec => {
                if let (Some(tab), Some(st)) = (self.rec.as_mut(), self.rec_state.as_ref()) {
                    tab.wheel(ui, st, delta);
                }
                // The target pad follows the Onto row.
                self.refresh_rec_state(ui);
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
        if let Some(tab) = self.grain.as_mut() {
            tab.lift();
        }
        self.last_turn = None;
        self.play_turn = None;
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
            Before::Grain { layer, region } => {
                let Some(l) = after.layers.get(*layer) else {
                    return;
                };
                grain_tab::describe(region, &l.region)
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

    // ------------------------------------------------------------ rec

    /// What the Rec tab shows besides the input: the pad picked and where
    /// a take would go.
    fn rec_state_now(&self) -> RecState {
        let project = self.projects.get(self.kit);
        let sound = |p: usize| {
            project
                .and_then(|pr| pr.pad(p))
                .filter(|pd| !pd.layers.is_empty())
        };
        let onto = self
            .rec
            .as_ref()
            .map_or(rec_tab::Onto::NextEmpty, |t| t.settings.onto);
        RecState {
            pad: self.pad,
            pad_name: sound(self.pad).map(|pd| pd.name.clone()),
            target: project.and_then(|_| rec_tab::target(self.pad, onto, |p| sound(p).is_some())),
            line_on: self.line_on,
            last: self.last_take.clone(),
        }
    }

    fn refresh_rec(&mut self, ui: &mut Ui, bank: Option<char>) {
        let st = self.rec_state_now();
        match self.rec.as_mut() {
            Some(tab) => {
                tab.bank = bank;
                tab.update(ui, &st);
            }
            None => match RecTab::add(ui, REC_PAGE, &st, self.rate) {
                Ok(mut tab) => {
                    tab.bank = bank;
                    tab.update(ui, &st);
                    self.rec = Some(tab);
                }
                Err(e) => {
                    self.lines.push(format!("sample: no Rec page ({e})"));
                    return;
                }
            },
        }
        self.rec_state = Some(st);
        self.feed_rec(ui, 0.0);
    }

    /// The Rec tab's state again: the target follows the Onto row and the
    /// pads.
    fn refresh_rec_state(&mut self, ui: &mut Ui) {
        if self.rec_state.is_none() {
            return;
        }
        let st = self.rec_state_now();
        if let Some(tab) = self.rec.as_mut() {
            tab.update(ui, &st);
        }
        self.rec_state = Some(st);
    }

    /// The Rec tab's picture: what came in over the last `dt` seconds.
    fn feed_rec(&mut self, ui: &mut Ui, dt: f32) {
        let Some(tab) = self.rec.as_mut() else {
            return;
        };
        match self.input.as_ref() {
            Some(input) => tab.feed(
                ui,
                input,
                self.take.as_ref().map(|t| &t.rec),
                self.line_on,
                dt,
            ),
            None => tab.idle(ui, self.line_on),
        }
    }

    fn act_rec(&mut self, ui: &mut Ui, a: RecAction) {
        match a {
            RecAction::Tab(i) => self.open(ui, i),
            RecAction::Record => self.start_take(ui),
            RecAction::Stop => {
                if let (Some(t), Some(input)) = (self.take.as_mut(), self.input.as_ref()) {
                    t.rec.stop(input);
                }
                // A take that was recording ends here and lands at once.
                self.tick(ui, 0.0, true);
            }
            RecAction::SkipBack => self.skip_back(ui),
        }
    }

    /// A bar's length now: the transport's while it plays, else 4/4 at the
    /// project tempo.
    fn bar_frames(&self, input: &Input) -> f64 {
        input
            .bars
            .bar_frames()
            .unwrap_or_else(|| 4.0 * 60.0 / self.tempo().max(1.0) * f64::from(self.rate))
    }

    fn start_take(&mut self, ui: &mut Ui) {
        if self.take.is_some() {
            return;
        }
        let st = self.rec_state_now();
        let (Some(input), Some(tab)) = (self.input.as_ref(), self.rec.as_ref()) else {
            self.lines
                .push("sample: no take: the line input is off".to_string());
            return;
        };
        let Some(pad) = st.target else {
            self.lines
                .push("sample: no take: no empty pad in this bank".to_string());
            return;
        };
        let settings = tab.settings;
        let rec = settings.recording(input, self.bar_frames(input));
        self.lines.push(format!(
            "sample: take armed for pad {}: start {}, end {}, {}",
            self.pad_text(self.kit, pad),
            rec_tab::value_text(&settings, rec_tab::Row::Start).to_lowercase(),
            match settings.end {
                rec_tab::EndAt::Length => rec_tab::value_text(&settings, rec_tab::Row::Length),
                _ => rec_tab::value_text(&settings, rec_tab::Row::End).to_lowercase(),
            },
            settings.pick.label().to_lowercase(),
        ));
        self.take = Some(Take {
            rec,
            kit: self.kit,
            pad,
            pick: settings.pick,
        });
        self.feed_rec(ui, 0.0);
    }

    fn skip_back(&mut self, ui: &mut Ui) {
        if self.take.is_some() {
            return;
        }
        let st = self.rec_state_now();
        let (Some(input), Some(tab)) = (self.input.as_ref(), self.rec.as_ref()) else {
            return;
        };
        let Some(pad) = st.target else {
            self.lines
                .push("sample: no skip back: no empty pad in this bank".to_string());
            return;
        };
        let settings = tab.settings;
        let data = match settings.skip {
            Skip::Seconds(n) => input.last_frames(u64::from(n) * u64::from(input.rate)),
            Skip::Bars(n) => input.last_bars(n as usize),
        };
        let what = format!("skip back {}", settings.skip.label());
        let rate = input.rate;
        let name = self.take_name();
        match data.and_then(|d| settings.pick.source(&name, rate, d)) {
            Ok(src) => self.keep(ui, self.kit, pad, src, &what),
            Err(e) => self.lines.push(format!("sample: {what} not kept ({e:?})")),
        }
    }

    fn take_name(&mut self) -> String {
        self.takes += 1;
        format!("Take {}", self.takes)
    }

    /// Puts a recorded sound on pad `pad` of kit `kit` (one undo step) and
    /// picks that pad.
    fn keep(&mut self, ui: &mut Ui, kit: usize, pad: usize, src: Source, what: &str) {
        let (name, frames, rate, ch) = (src.name.clone(), src.frames(), src.rate(), src.channels());
        let Some(project) = self.projects.get_mut(kit) else {
            self.lines
                .push(format!("sample: {what} not kept (the kit is gone)"));
            return;
        };
        match project.put_source(pad, src) {
            Ok(_) => {
                self.edits += 1;
                if let Some(d) = self.dirty.get_mut(kit) {
                    *d = true;
                }
                self.renamed = Some(kit);
                let text = format!(
                    "{name}, {} {} on pad {}",
                    duration_text(frames, rate),
                    if ch == 2 { "stereo" } else { "mono" },
                    self.pad_text(kit, pad)
                );
                self.lines.push(format!("sample: {what}: {text}"));
                self.last_take = text;
                if kit == self.kit {
                    self.lift();
                    self.pad = pad;
                    self.velocity = None;
                }
                self.refresh(ui);
            }
            Err(e) => self.lines.push(format!("sample: {what} not kept ({e:?})")),
        }
    }

    /// About 30 times a second: polls the take (when it ends it lands on
    /// its pad) and, while the Rec tab `showing`, moves its picture `dt`
    /// seconds on.
    pub fn tick(&mut self, ui: &mut Ui, dt: f32, showing: bool) {
        if let (Some(t), Some(input)) = (self.take.as_mut(), self.input.as_ref()) {
            match t.rec.poll(input) {
                Ok(Progress::Done { .. }) => {
                    if let Some(t) = self.take.take() {
                        let name = self.take_name();
                        match t.rec.finish(&name, t.pick) {
                            Some(Ok(src)) => self.keep(ui, t.kit, t.pad, src, "recorded"),
                            Some(Err(e)) => {
                                self.lines.push(format!("sample: take not kept ({e:?})"))
                            }
                            None => {}
                        }
                    }
                }
                Ok(Progress::Nothing) => {
                    self.take = None;
                    self.lines
                        .push("sample: take cancelled before it started".to_string());
                }
                Ok(_) => {}
                Err(e) => {
                    self.take = None;
                    self.lines.push(format!("sample: take lost ({e:?})"));
                }
            }
        }
        if showing && self.editor == Editor::Rec && self.rec_state.is_some() {
            self.feed_rec(ui, dt);
        }
    }

    /// The line input turned on or off (its row on the Looper tab's MENU).
    /// On, the sampler's input is made (once) for the audio thread; off, a
    /// take still recording ends and is kept, and the input is freed.
    pub fn set_line_input(&mut self, ui: &mut Ui, on: bool) {
        if on == self.line_on {
            return;
        }
        self.line_on = on;
        if on {
            if self.input.is_none() {
                let (feed, input) = record::input(INPUT_SECONDS, self.rate);
                self.input = Some(input);
                self.feed = Some(Box::new(RecFeed(feed)));
                self.lines.push(format!(
                    "sample: the line input feeds the sampler ({INPUT_SECONDS} s kept, {:.1} MB)",
                    input_bytes(self.rate) as f64 / 1e6
                ));
            }
        } else {
            if let (Some(t), Some(input)) = (self.take.as_mut(), self.input.as_ref()) {
                t.rec.stop(input);
            }
            self.tick(ui, 0.0, false);
            self.take = None;
            self.input = None;
            // Not sent yet: freed here and now. Else the audio thread hands
            // it back first.
            if self.feed.take().is_none() && self.feeding {
                self.want_back = true;
            }
        }
        self.refresh_rec_state(ui);
        self.feed_rec(ui, 0.0);
    }

    /// The input's writing end, for the audio thread, once; or the ask to
    /// hand it back, once the line input is off.
    pub fn take_input(&mut self) -> Option<FromUi> {
        if let Some(f) = self.feed.take() {
            self.feeding = true;
            return Some(FromUi::SampleInput(Some(f)));
        }
        if std::mem::take(&mut self.want_back) {
            return Some(FromUi::SampleInput(None));
        }
        None
    }

    /// [`SamplePage::take_input`]'s message found no room: again later.
    pub fn input_unsent(&mut self, f: Option<Box<RecFeed>>) {
        match f {
            // Still wanted if the input it writes is still here.
            Some(f) if self.input.is_some() => {
                self.feeding = false;
                self.feed = Some(f);
            }
            Some(_) => self.feeding = false,
            None => self.want_back = true,
        }
    }

    /// The audio thread handed the writing end back: the buffer is freed
    /// here (with the input, already dropped, it was the last owner).
    pub fn input_back(&mut self, f: Box<RecFeed>) {
        let frames = f.written();
        drop(f);
        if self.feed.is_none() && self.input.is_none() {
            self.feeding = false;
        }
        self.lines.push(format!(
            "sample: the line input's buffer was freed ({:.1} s had come in)",
            frames as f64 / f64::from(self.rate.max(1))
        ));
    }

    // ------------------------------------------------------------ warp

    /// The project tempo (the transport's). Kits with warped pads get new
    /// views, and their copies for the old tempo stop being made.
    pub fn set_tempo(&mut self, bpm: f64) {
        if !self.warps.set_tempo(bpm) {
            return;
        }
        self.cancel_stretching();
        for k in 0..self.projects.len() {
            if self.warped(k) {
                self.dirty[k] = true;
            }
        }
    }

    /// The project tempo (the transport's).
    pub fn tempo(&self) -> f64 {
        self.warps.tempo()
    }

    /// Whether kit `kit` has a warped pad.
    fn warped(&self, kit: usize) -> bool {
        self.projects.get(kit).is_some_and(|p| {
            (0..mpfree_sampler::model::PADS).any(|i| {
                p.pad(i)
                    .is_some_and(|pad| pad.layers.iter().any(|l| l.region.warp.is_some()))
            })
        })
    }

    fn cancel_stretching(&mut self) {
        if let Some(w) = &self.stretcher {
            w.cancel();
        }
        self.stretching.clear();
    }

    /// Copies kit `kit` needs at the tempo go to the stretch thread.
    fn ask_for_copies(&mut self, kit: usize) {
        let Some(project) = self.projects.get(kit) else {
            return;
        };
        let missing: Vec<CopyKey> = self
            .warps
            .missing(project)
            .into_iter()
            .filter(|k| !self.stretching.contains(k))
            .collect();
        if missing.is_empty() {
            return;
        }
        if self.stretcher.is_none() {
            match Worker::spawn() {
                Ok(w) => self.stretcher = Some(w),
                Err(e) => {
                    self.lines.push(format!("sample: no stretch thread: {e}"));
                    return;
                }
            }
        }
        let Some(w) = &self.stretcher else { return };
        for k in missing {
            match Warps::spec(project, &k) {
                Ok(spec) => {
                    if w.send(k, spec) {
                        self.stretching.push(k);
                    }
                }
                Err(e) => self.lines.push(format!("sample: can't stretch: {e}")),
            }
        }
    }

    /// Takes in the copies the stretch thread has finished; the kits that
    /// play them get new views.
    fn take_copies(&mut self) {
        let Some(w) = &self.stretcher else { return };
        let mut done = Vec::new();
        while let Some(d) = w.try_recv() {
            done.push(d);
        }
        for d in done {
            match d {
                Done::Copy(key, data, took) => {
                    self.stretching.retain(|k| *k != key);
                    self.warps.insert_for(key, data, &self.projects);
                    for k in 0..self.projects.len() {
                        if self.warps.wanted(&self.projects[k]).contains(&key) {
                            self.dirty[k] = true;
                        }
                    }
                    self.lines.push(format!(
                        "sample: stretched {:.2} s to {:.0} BPM in {:.0} ms ({:.1} MB of copies)",
                        key.frames as f64 / f64::from(self.rate.max(1)),
                        self.warps.tempo(),
                        took.as_secs_f64() * 1000.0,
                        self.warps.bytes() as f64 / 1e6
                    ));
                }
                Done::Failed(key, e) => {
                    self.stretching.retain(|k| *k != key);
                    self.lines.push(format!("sample: stretch failed: {e}"));
                }
                Done::Cancelled(key) => self.stretching.retain(|k| *k != key),
            }
        }
    }

    // ------------------------------------------------------------ the audio thread

    /// The next kit view for the audio thread, if a kit was edited (or a
    /// stretched copy came in) and no view is on its way already. Built
    /// here, off the audio thread, with warped pads at the project tempo.
    pub fn take_view(&mut self) -> Option<FromUi> {
        self.take_copies();
        if self.in_flight {
            return None;
        }
        let kit = self.dirty.iter().position(|d| *d)?;
        self.dirty[kit] = false;
        self.in_flight = true;
        self.views_sent += 1;
        let view = EngineView::build_warped(&self.projects[kit], self.rate, &mut self.warps);
        self.ask_for_copies(kit);
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
            // The Play, Pads and Rec tabs draw no playheads; the Grain
            // tab's come with its cloud.
            Editor::Play | Editor::Pads | Editor::Grain | Editor::Rec => {}
        }
        self.shown_heads = frames;
    }

    /// The cloud's box, naming the Grain tab's pad and layer, for the audio
    /// thread to fill; None unless the Grain tab shows and the box is here.
    pub fn take_cloud(&mut self) -> Option<FromUi> {
        if self.page() != GRAIN_PAGE {
            return None;
        }
        let st = self.grain_state.as_ref()?;
        let mut c = self.cloud.take()?;
        c.kit = self.kit;
        c.pad = st.pad.min(usize::from(u8::MAX)) as u8;
        c.layer = st.layer.min(usize::from(u8::MAX)) as u8;
        Some(FromUi::SampleGrains(c))
    }

    /// The box found no room on its way: it waits here for the next time.
    pub fn cloud_unsent(&mut self, c: Box<Cloud>) {
        self.cloud = Some(c);
    }

    /// The audio thread filled the box: the Grain tab draws its grains if
    /// they are still the ones it shows. The box stays here until the next
    /// [`SamplePage::take_cloud`].
    pub fn cloud_back(&mut self, ui: &mut Ui, c: Box<Cloud>) {
        let shows = self.page() == GRAIN_PAGE && c.kit == self.kit;
        if let (true, Some(tab), Some(st)) = (shows, self.grain.as_ref(), self.grain_state.as_ref())
        {
            if usize::from(c.pad) == st.pad && usize::from(c.layer) == st.layer {
                let busy = c.len > 0 || c.head.is_some() || c.live > 0;
                // An idle cloud is drawn once, not 25 times a second.
                if busy || self.cloud_shown {
                    let r = &st.region;
                    let head = c
                        .head
                        .map(|h| ((h as f64 - r.start as f64) / r.frames().max(1) as f64) as f32);
                    tab.set_cloud(ui, st, c.grains(), head, c.live);
                }
                self.cloud_shown = busy;
            }
        }
        self.cloud = Some(c);
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
