//! The Stems page on the Sample page (the stems plan's step 4): the pad
//! bar's Stems button opens it over the Wave page for the pad or take shown,
//! and the sampler's [`Split`] works out the stems a step a job on the job
//! service's sound lane, while the screen goes on. The page's own code is
//! `mpfree_sampler_ui::stems_page`; this is the app's side of it:
//!
//! - **Opening and closing.** Stems opens the page (from Edit pad, the
//!   Sample tab with it); Stems again goes back to Wave. While nothing is
//!   splitting, the page follows the pad picked.
//! - **The model** is read from the data folder's `files/models/`
//!   ([`MODEL_FILE`], where the USB Creator puts it) by the split's first
//!   step, off the screen, and kept while the page is open, so the next
//!   split doesn't read it again; leaving the page lets it go (78.7 MB).
//! - **Memory.** Before a split starts, what it holds at most
//!   ([`mpfree_sampler::stems::holds`]) is promised to the shared memory
//!   check ([`crate::memory::reserve`]), and the promise is held until the
//!   split ends however it ends.
//! - **While it splits** the page shows how far it has got and each stem's
//!   picture as it comes, a few times a second; on other pages it goes on,
//!   and the pad bar's Stems button says how far ("Stems 42 %").
//! - **Landing.** A finished split lands where the page said, as one undo
//!   step: on the kit's pads, the sound's pad's layers or takes (one
//!   [`Change::Stems`]), on a new kit for a new track (as a chop's New kit
//!   does), or in the library under Samples › Stems (the app writes the
//!   files on a thread of its own, [`save_stems`]).
//! - **Once landed** a tap on a stem plays its pad, Play all four plays
//!   them together, and a Q-Link turns a stem's level on its pad or layer
//!   (one undo step a turn).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use mpfree_sampler::model::{Source, StemsTo, PADS};
use mpfree_sampler::stems::{self as split, Model, ModelFrom, Settings, Split};
use mpfree_sampler_ui::command::Recorded;
use mpfree_sampler_ui::send;
use mpfree_sampler_ui::stems_page::{self, Ask, Landing, Phase, Put, Room, Sound, StemsPage};
use mpfree_ui::{EventKind, Ui, UiEvent};

use super::{job_id, Change, Editor, NewKitTrack, SamplePage};
use crate::gui::FromUi;

/// The model's folder in the data folder's `files/`, and its file (Deezer's
/// Spleeter 4-stem model, packed by `mpfree-stems`' `pack`): where the USB
/// Creator puts it.
pub const MODELS: &str = "models";
pub const MODEL_FILE: &str = "spleeter-4stems.mpfstem";

/// The folder in Samples that stems saved to the library go to, each
/// sound's in a folder of its own named after it.
pub const LIBRARY: &str = "Stems";

/// What the page says when the model file isn't there.
pub const NO_MODEL: &str = "No stems model in files/models/ on the stick yet";

/// While it splits, the page is drawn again at most this often, seconds
/// (or when a step ends).
const DRAW_EVERY: f32 = 0.5;

/// "STEM" and the edit count: a level turned after landing, its undo step.
const TURN_ID: u64 = 0x5354_454D_0000_0000;

/// The Sample page's stems: where the model is, the model once read, and
/// the page while it is open or its split runs.
#[derive(Default)]
pub(super) struct Here {
    model_path: Option<PathBuf>,
    model: Option<Arc<Model>>,
    on: Option<On>,
    /// Stems put in the library, for the app to write: the sound's name
    /// (the folder's) and the stems.
    saves: Vec<(String, Vec<Source>)>,
}

/// The page, open on one sound.
struct On {
    page: StemsPage,
    /// Over the Wave page now (else its split runs on behind other pages).
    shown: bool,
    /// The sound: the kit, and its pad or the take kept on the page.
    kit: usize,
    pad: usize,
    take: Option<usize>,
    /// Its channels, for the memory it holds.
    channels: usize,
    run: Option<Run>,
    /// The kit it landed on (a new kit's, for New kit).
    landed_kit: usize,
    /// A level turning since it landed: the stem and the turn's undo id,
    /// until the Q-Link is let go ([`SamplePage::lift`]).
    turn: Option<(usize, u64)>,
}

/// A split under way.
struct Run {
    split: Split,
    started: Instant,
    /// New kit: the empty kit it lands on, fixed when it started.
    new_kit: Option<usize>,
    /// The progress last drawn, and the seconds since.
    drawn: f32,
    since: f32,
}

impl On {
    fn splitting(&self) -> bool {
        self.run.is_some()
    }
}

/// Writes stems put in the library into a new folder in Samples / Stems
/// named after their sound ("Soul loop", or "Soul loop 2" when that is
/// taken), each a WAV file named after the stem ("Soul loop Vocals.wav"),
/// making Stems if it isn't there. Each file is written whole or not at
/// all. Call it off the screen's thread. Returns the folder.
pub fn save_stems(
    lib: &mpfree_library::Library,
    name: &str,
    sounds: &[Source],
) -> std::io::Result<mpfree_library::Loc> {
    use mpfree_library::{Loc, Place};
    let samples = Loc::top(Place::Samples);
    let stems = samples.join(LIBRARY)?;
    if lib.list(&stems).is_err() {
        lib.new_folder(&samples, LIBRARY)?;
    }
    let folder = lib.new_folder(&stems, &mpfree_sampler::export::file_safe(name))?;
    for s in sounds {
        let mut bytes = Vec::new();
        mpfree_sampler::wav::write(s, &mut bytes)?;
        let file = format!("{}.wav", mpfree_sampler::export::file_safe(&s.name));
        lib.write_new(&folder, &file, &bytes)?;
    }
    Ok(folder)
}

impl SamplePage {
    // ------------------------------------------------------------ the app

    /// Where the stems model is: the data folder's `files/models/`
    /// ([`MODEL_FILE`]); None without a data folder.
    pub fn set_stems_model(&mut self, data: Option<&std::path::Path>) {
        self.stems.model_path = data.map(|d| d.join("files").join(MODELS).join(MODEL_FILE));
    }

    /// The Stems page's hint line, while it shows.
    pub fn stems_hint(&self) -> String {
        self.stems
            .on
            .as_ref()
            .map(|on| on.page.stems.hint())
            .unwrap_or_default()
    }

    /// Stems put in the library since the last call, for the app to write
    /// ([`save_stems`]): each sound's name and its stems.
    pub fn take_stem_saves(&mut self) -> Vec<(String, Vec<Source>)> {
        std::mem::take(&mut self.stems.saves)
    }

    // ------------------------------------------------- the page's hooks

    /// The Stems page shows over the Wave page.
    pub(super) fn stems_shown(&self) -> bool {
        self.editor == Editor::Wave && self.stems.on.as_ref().is_some_and(|on| on.shown)
    }

    /// The pad bar's Stems pressed: the page opens for the sound shown
    /// (the Sample tab with it, from Edit pad), or goes back to Wave.
    pub(super) fn press_stems(&mut self, ui: &mut Ui) {
        if self.stems_shown() {
            self.close_stems(ui);
            return;
        }
        if self.editor.on_pads_tab() {
            self.lift();
            self.sample_editor = Editor::Wave;
            self.to_sample_tab = true;
        } else if self.editor != Editor::Wave {
            self.open_editor(ui, Editor::Wave);
        }
        // A split running shows again, whatever sound is picked now.
        if let Some(on) = self.stems.on.as_mut().filter(|on| on.splitting()) {
            on.shown = true;
            self.stale = true;
            return;
        }
        if self.tab.is_none() {
            // The Wave page's frame (the pad bar) comes first.
            self.refresh(ui);
        }
        self.open_stems(ui);
    }

    /// The page opened on the sound shown now (or closed, with none).
    fn open_stems(&mut self, ui: &mut Ui) {
        let kit = self.kit;
        let take = self.shown_take();
        let pad = self.pad;
        let Some((sound, channels)) = self.stems_sound(kit, pad, take) else {
            self.stems.on = None;
            self.told
                .push("The pad picked has no sound to split".into());
            return;
        };
        if let Err(e) = StemsPage::add(ui) {
            self.lines.push(format!("sample: no Stems page ({e})"));
            return;
        }
        let room = Room::default();
        let page = StemsPage::open(ui, sound, room);
        self.stems.on = Some(On {
            page,
            shown: true,
            kit,
            pad,
            take,
            channels,
            run: None,
            landed_kit: kit,
            turn: None,
        });
        self.show_stems(ui);
        self.stale = true;
    }

    /// Back to Wave: the page closes, unless its split runs (it goes on,
    /// and the pad bar says how far); the model is let go with it.
    fn close_stems(&mut self, ui: &mut Ui) {
        match self.stems.on.as_mut() {
            Some(on) if on.splitting() => on.shown = false,
            _ => {
                self.stems.on = None;
                self.stems.model = None;
            }
        }
        send::light_stems(ui, false);
        self.stale = true;
    }

    /// The pad bar's Chop to pads on the Stems page: back to Wave with the
    /// Slice tool picked, so the cuts show before anything is chopped (a
    /// split under way goes on).
    pub(super) fn stems_chop(&mut self, ui: &mut Ui) {
        self.close_stems(ui);
        let (Some(tab), Some(st)) = (self.tab.as_mut(), self.state.as_ref()) else {
            return;
        };
        if tab.tool() != super::Tool::Slice {
            tab.set_tool(ui, st, super::Tool::Slice);
            self.keep_chop();
            self.follow_sound(ui);
        }
    }

    /// The kits were replaced (a project opened, New project, a kit file
    /// loaded): a split under way stops, as its sound may have gone, and
    /// the page closes with its model.
    pub(super) fn stems_kits_replaced(&mut self) {
        if let Some(run) = self.stems.on.as_mut().and_then(|on| on.run.take()) {
            run.split.stop(&self.jobs);
            let name = self
                .stems
                .on
                .as_ref()
                .map_or_else(String::new, |on| on.page.stems.sound.name.clone());
            self.lines
                .push(format!("sample: stems of {name} stopped: the kits changed"));
        }
        self.stems.on = None;
        self.stems.model = None;
    }

    /// A Q-Link let go (or a rest after a turn): the next level turned is
    /// a new undo step.
    pub(super) fn stems_lift(&mut self) {
        if let Some(on) = self.stems.on.as_mut() {
            on.turn = None;
        }
    }

    /// An undo or redo: stems landed may have gone, so the page can split
    /// again.
    pub(super) fn stems_undone(&mut self) {
        if let Some(on) = self
            .stems
            .on
            .as_mut()
            .filter(|on| on.page.stems.has_stems())
        {
            on.page.stems.stopped();
            on.turn = None;
        }
    }

    /// The page's touches: its picture, its panel and its Q-Link cards.
    pub(super) fn stems_event(&mut self, ui: &mut Ui, ev: &UiEvent, out: &mut Vec<FromUi>) {
        self.answered = true;
        let Some(on) = self.stems.on.as_mut() else {
            return;
        };
        let ask = match ev.widget.as_str() {
            stems_page::VIEW => on.page.view_event(ui, &ev.kind),
            stems_page::LIST => on.page.event(ui, &ev.kind),
            stems_page::QLINKS => match ev.kind {
                EventKind::Step { index, delta } => on.page.qlink(ui, index, delta),
                _ => Ask::Nothing,
            },
            _ => Ask::Nothing,
        };
        self.stems_ask(ui, ask, out);
    }

    /// Q-Link `knob` turned on the Stems page: a stem's level.
    pub(super) fn stems_qlink(&mut self, ui: &mut Ui, knob: usize, ticks: i32) {
        let Some(on) = self.stems.on.as_mut() else {
            return;
        };
        let ask = on.page.qlink(ui, knob, ticks);
        self.stems_ask(ui, ask, &mut Vec::new());
    }

    /// The wheel picking: the panel's next control.
    pub(super) fn stems_pick(&mut self, ui: &mut Ui, delta: i32) {
        if let Some(on) = self.stems.on.as_mut() {
            on.page.wheel(ui, delta, false);
        }
    }

    /// - / +, or the wheel once its push opened the control: steps it.
    pub(super) fn stems_wheel(&mut self, ui: &mut Ui, delta: i32) {
        let Some(on) = self.stems.on.as_mut() else {
            return;
        };
        let ask = on.page.wheel(ui, delta, true);
        self.stems_ask(ui, ask, &mut Vec::new());
    }

    /// After the page was built again: the Stems button lit while it shows
    /// (or saying how far a split behind other pages has got), the page
    /// following the pad picked while nothing splits, and what it has room
    /// for now.
    pub(super) fn refresh_stems(&mut self, ui: &mut Ui) {
        let shown = self.stems_shown();
        send::light_stems(ui, shown);
        self.label_stems(ui);
        if !shown {
            return;
        }
        let now = (self.kit, self.pad, self.shown_take());
        let follows = self.stems.on.as_ref().is_some_and(|on| {
            on.page.stems.phase == Phase::Ready && (on.kit, on.pad, on.take) != now
        });
        if follows {
            self.open_stems(ui);
            return;
        }
        self.show_stems(ui);
    }

    /// Each frame: the split's progress and pictures, and its stems landed
    /// once it has finished.
    pub(super) fn poll_stems(&mut self, ui: &mut Ui, dt: f32, showing: bool) {
        let shown = showing && self.stems_shown();
        let Some(on) = self.stems.on.as_mut() else {
            return;
        };
        let Some(run) = on.run.as_mut() else {
            return;
        };
        if let Some(m) = run.split.model() {
            self.stems.model.get_or_insert(m);
        }
        match run.split.poll() {
            None => {
                let progress = run.split.progress();
                run.since += dt;
                if progress != run.drawn || run.since >= DRAW_EVERY {
                    let moved = progress != run.drawn;
                    run.drawn = progress;
                    run.since = 0.0;
                    let ran = run.started.elapsed().as_secs_f32();
                    if shown {
                        let peaks = run.split.peaks();
                        on.page.splitting(ui, progress, ran, &peaks);
                    } else {
                        on.page.stems.phase = Phase::Splitting {
                            progress,
                            ran_s: ran,
                        };
                    }
                    if moved {
                        self.label_stems(ui);
                    }
                }
            }
            Some(Ok(made)) => {
                let peaks = run.split.peaks();
                let new_kit = run.new_kit;
                on.run = None;
                on.page.set_lanes(ui, &peaks);
                self.land_stems(ui, made, new_kit);
            }
            Some(Err(why)) => {
                let name = on.page.stems.sound.name.clone();
                on.run = None;
                on.page.stems.stopped();
                on.page.show(ui);
                self.told.push(format!("Stems of {name} not made: {why}"));
                self.lines
                    .push(format!("sample: stems of {name} not made ({why})"));
                self.let_go_hidden();
                self.label_stems(ui);
            }
        }
    }

    // ------------------------------------------------------- the work

    /// The sound of kit `kit`'s pad `pad` (or take `take`) for the page,
    /// and its channels.
    fn stems_sound(
        &mut self,
        kit: usize,
        pad: usize,
        take: Option<usize>,
    ) -> Option<(Sound, usize)> {
        let layer = if take.is_some() { 0 } else { self.layer() };
        let p = self.projects.get(kit)?;
        let (sound, on_pad) = match take {
            Some(i) => (p.take(i)?, None),
            None => (p.pad(pad)?, Some(pad)),
        };
        let region = &sound.layers.get(layer)?.region;
        let source = p.source(region.source)?.clone();
        let (name, span) = (sound.name.clone(), (region.start, region.end));
        let view = self.sources.get(&source);
        let peaks = view.pyramid.is_some().then(|| view.peaks.clone());
        let s = Sound::of(&name, on_pad, &source, span, peaks.as_deref());
        Some((s, usize::from(source.channels())))
    }

    /// The sound the page splits now, and the frames it reads.
    fn stems_source(&self, on: &On) -> Option<(Source, (u64, u64))> {
        let p = self.projects.get(on.kit)?;
        let sound = match on.take {
            Some(i) => p.take(i)?,
            None => p.pad(on.pad)?,
        };
        let layer = match on.take {
            Some(_) => 0,
            None => self.layer().min(sound.layers.len().saturating_sub(1)),
        };
        let region = &sound.layers.get(layer)?.region;
        let source = p.source(region.source)?.clone();
        Some((source, on.page.stems.sound.span(on.page.stems.reads)))
    }

    /// What the split holds at most, as the page is set.
    fn stems_need(&self, on: &On) -> u64 {
        let s = &on.page.stems;
        split::holds(
            s.sound.samples(s.reads),
            on.channels,
            s.kept(),
            self.stems.model.is_some(),
            s.sound.rate != split::RATE,
            &Settings::default(),
        )
    }

    /// The room a split has now: memory, the kit's pads and takes, New
    /// kit and Library as the app offers them, and why none can start.
    fn stems_room(&self, on: &On) -> Room {
        let p = self.projects.get(on.kit);
        let holds = self.stems_need(on);
        let free = match (self.mem_free)() {
            Some(f) => f
                .saturating_sub(crate::memory::FLOOR)
                .saturating_sub(crate::memory::promised()),
            None if crate::memory::room_in(None, holds).is_ok() => holds,
            None => 0,
        };
        let blocked = match &self.stems.model_path {
            _ if on.splitting() => None,
            _ if self.stems.model.is_some() => None,
            Some(path) if path.is_file() => None,
            _ => Some(NO_MODEL.to_string()),
        };
        Room {
            holds,
            free,
            filled: (0..PADS)
                .map(|i| {
                    p.and_then(|p| p.pad(i))
                        .is_some_and(|pd| !pd.layers.is_empty())
                })
                .collect(),
            takes: p.map_or(0, |p| p.takes().len()),
            max_takes: mpfree_sampler::model::MAX_TAKES,
            new_kit_off: match &self.new_kit {
                Some(Ok(u)) if *u != on.kit && *u < self.projects.len() => None,
                Some(Err(why)) => Some(why.clone()),
                _ => Some("No empty kit for a new track now".into()),
            },
            library: self.files,
            blocked,
        }
    }

    /// The page as it is now, with the room it has.
    fn show_stems(&mut self, ui: &mut Ui) {
        let Some(on) = self.stems.on.as_ref() else {
            return;
        };
        let room = self.stems_room(on);
        if let Some(on) = self.stems.on.as_mut() {
            if on.page.stems.room != room {
                on.page.stems.room = room;
            }
            on.page.show(ui);
        }
    }

    /// The pad bar's Stems button: how far a split has got while the page
    /// doesn't show ("Stems 42 %"), else "Stems".
    fn label_stems(&self, ui: &mut Ui) {
        let label = match self.stems.on.as_ref() {
            Some(on) if on.splitting() && !self.stems_shown() => {
                let progress = on.run.as_ref().map_or(0.0, |r| r.split.progress());
                format!("Stems {:.0} %", progress * 100.0)
            }
            _ => send::BUTTONS[send::STEMS].to_string(),
        };
        send::label_stems(ui, &label);
    }

    /// What a press on the page asked for.
    fn stems_ask(&mut self, ui: &mut Ui, ask: Ask, out: &mut Vec<FromUi>) {
        match ask {
            Ask::Nothing => self.show_stems(ui),
            Ask::Say(why) => self.told.push(why),
            Ask::Split => self.start_stems(ui),
            Ask::Stop => self.stop_stems(ui),
            Ask::Play(k) => self.play_stems(Some(k), out),
            Ask::PlayAll => self.play_stems(None, out),
            Ask::Level(k) => self.level_stem(ui, k),
        }
    }

    /// Split as the page is set: the memory promised, then the first step
    /// on the sound lane.
    fn start_stems(&mut self, ui: &mut Ui) {
        let Some(on) = self.stems.on.as_ref() else {
            return;
        };
        let Some((source, span)) = self.stems_source(on) else {
            self.told
                .push("The sound has gone from its pad: pick it again".into());
            return;
        };
        let s = &on.page.stems;
        let new_kit = match (s.put, &self.new_kit) {
            (Put::NewKit, Some(Ok(u))) if *u != on.kit && *u < self.projects.len() => Some(*u),
            (Put::NewKit, _) => {
                self.told.push("No empty kit for a new track now".into());
                return;
            }
            _ => None,
        };
        let model = match (&self.stems.model, &self.stems.model_path) {
            (Some(m), _) => ModelFrom::Loaded(m.clone()),
            (None, Some(path)) => ModelFrom::File(path.clone()),
            (None, None) => {
                self.told.push(NO_MODEL.into());
                return;
            }
        };
        let need = self.stems_need(on);
        let hold = match crate::memory::reserve(self.mem_free, need) {
            Ok(t) => t,
            Err(why) => {
                self.told.push(format!("Stems not started: {why}"));
                return;
            }
        };
        let ask = split::Ask {
            id: job_id(("stems", on.kit, source.id())),
            model,
            input: split::Input::Sound { source, span },
            settings: Settings::default(),
            keep: s.keep,
            sixteen: s.sound.sixteen,
            columns: stems_page::COLUMNS,
        };
        let (name, secs) = (s.sound.name.clone(), s.sound.seconds(s.reads));
        match Split::start(&self.jobs, ask, hold) {
            Ok(split) => {
                if let Some(on) = self.stems.on.as_mut() {
                    on.page.stems.started();
                    on.turn = None;
                    on.run = Some(Run {
                        split,
                        started: Instant::now(),
                        new_kit,
                        drawn: 0.0,
                        since: 0.0,
                    });
                    let to = on.page.stems.landed_text();
                    self.lines.push(format!(
                        "sample: stems of {name}: splitting {secs:.1} s ({} MB promised); {}",
                        need / 1_000_000,
                        to.to_lowercase()
                    ));
                }
                self.show_stems(ui);
            }
            Err(why) => self.told.push(format!("Stems not started: {why}")),
        }
    }

    /// Stop: the split stops before its next step, and nothing lands.
    fn stop_stems(&mut self, ui: &mut Ui) {
        let Some(on) = self.stems.on.as_mut() else {
            return;
        };
        if let Some(run) = on.run.take() {
            if let Some(m) = run.split.model() {
                self.stems.model.get_or_insert(m);
            }
            run.split.stop(&self.jobs);
            self.lines.push(format!(
                "sample: stems of {} stopped after {:.0} s; nothing kept",
                on.page.stems.sound.name,
                run.started.elapsed().as_secs_f32()
            ));
        }
        on.page.stems.stopped();
        self.show_stems(ui);
        self.label_stems(ui);
    }

    /// The stems landed where the page said, as one undo step.
    fn land_stems(&mut self, ui: &mut Ui, made: split::Made, new_kit: Option<usize>) {
        let Some(on) = self.stems.on.as_ref() else {
            return;
        };
        let (kit, name) = (on.kit, on.page.stems.sound.name.clone());
        let took = made.took.as_secs_f32();
        let pad = on
            .take
            .is_none()
            .then(|| self.projects.get(kit).and_then(|p| p.pad(on.pad)))
            .flatten();
        let landed = match on.page.stems.land(made, pad) {
            Ok(Landing::Kit(change)) => self
                .change_kit(kit, change, None)
                .map(|_| kit)
                .map_err(|e| e.to_string()),
            Ok(Landing::NewKit { pads, sounds }) => match new_kit {
                Some(to) => {
                    let change = Change::Stems {
                        sounds: sounds.into_iter().map(Recorded).collect(),
                        to: StemsTo::Pads(pads.into_iter().enumerate().collect()),
                    };
                    self.change_kit(to, change, None)
                        .map(|_| to)
                        .map_err(|e| e.to_string())
                }
                None => Err("no empty kit".to_string()),
            },
            Ok(Landing::Library(sounds)) => {
                self.stems.saves.push((name.clone(), sounds));
                Ok(kit)
            }
            Err(why) => Err(why),
        };
        let Some(on) = self.stems.on.as_mut() else {
            return;
        };
        match landed {
            Ok(to) => {
                on.landed_kit = to;
                on.page.stems.landed(took);
                let at = on.page.stems.landed_text();
                self.lines.push(format!(
                    "sample: stems of {name} made in {took:.1} s: {}",
                    at.to_lowercase()
                ));
                if to != kit {
                    // A new kit, for a new track, named after the sound.
                    if let Some(n) = self.names.get_mut(to) {
                        *n = format!("{name} stems");
                    }
                    self.new_kit_pad = Some((to, 0));
                    self.new_kit_track = Some(NewKitTrack {
                        kit: to,
                        phrase: None,
                    });
                }
                self.stale = true;
            }
            Err(why) => {
                on.page.stems.stopped();
                self.told.push(format!("Stems of {name} not landed: {why}"));
                self.lines
                    .push(format!("sample: stems of {name} not landed ({why})"));
            }
        }
        self.let_go_hidden();
        self.show_stems(ui);
        self.label_stems(ui);
    }

    /// A split that ended behind other pages: the page closes, and the
    /// model goes with it.
    fn let_go_hidden(&mut self) {
        if self.stems.on.as_ref().is_some_and(|on| !on.shown) {
            self.stems.on = None;
            self.stems.model = None;
        }
    }

    /// A stem (or all four) played on its pads, once landed.
    fn play_stems(&mut self, k: Option<usize>, out: &mut Vec<FromUi>) {
        let Some(on) = self.stems.on.as_ref() else {
            return;
        };
        match on.page.stems.plays(k) {
            Ok(pads) => {
                let kit = on.landed_kit;
                for down in [true, false] {
                    for &pad in &pads {
                        out.push(FromUi::SampleHit {
                            kit,
                            pad,
                            velocity: if down {
                                crate::play_page::TAP_VELOCITY
                            } else {
                                0
                            },
                            down,
                        });
                    }
                }
            }
            Err(why) => self.told.push(why),
        }
    }

    /// Stem `k`'s level turned: once landed, on its pad or layer too, one
    /// undo step a turn (until the Q-Link is let go, or after a rest; a
    /// turn of another stem is a new one).
    fn level_stem(&mut self, ui: &mut Ui, k: usize) {
        let edits = self.edits;
        let Some(on) = self.stems.on.as_mut() else {
            return;
        };
        let db = on.page.stems.levels.get(k).copied().unwrap_or(0.0);
        let Some((pad, layer)) = on.page.stems.level_on(k) else {
            return;
        };
        let kit = on.landed_kit;
        let id = match on.turn {
            Some((s, id)) if s == k => id,
            _ => TURN_ID | (edits & 0xFFFF_FFFF),
        };
        on.turn = Some((k, id));
        let Some(mut to) = self.projects.get(kit).and_then(|p| p.pad(pad)).cloned() else {
            return;
        };
        let Some(l) = to.layers.get_mut(layer) else {
            return;
        };
        l.level_db = db;
        let change = Change::Settings {
            pad,
            to: Box::new(to),
        };
        match self.change_kit(kit, change, Some(id)) {
            Ok(_) => self.stale = true,
            Err(e) => self.lines.push(format!(
                "sample: stem level on pad {} not changed ({e})",
                self.pad_text(kit, pad)
            )),
        }
        self.show_stems(ui);
    }
}

/// The Stems page's screens, for the app's previews and its smoke sweep:
/// "Break 92" on A5 with no model on the stick, and ready to split onto
/// A6–A9 with the model read (a small network in its place, which a split
/// tapped in the sweep runs quickly).
pub fn screens(fonts: &impl Fn() -> mpfree_ui::Fonts) -> Vec<(&'static str, crate::gui::Player)> {
    let press = |p: &mut crate::gui::Player| {
        p.on_event(UiEvent {
            widget: send::SEND.into(),
            kind: EventKind::Tab(send::STEMS),
        });
    };
    // A fixed free memory, so each preview is the same picture every time.
    let free: fn() -> Option<u64> = || Some(1_500_000_000);
    let mut none = crate::gui::sample_sampler(fonts());
    none.sample.mem_free = free;
    press(&mut none);
    let mut ready = crate::gui::sample_sampler(fonts());
    ready.sample.mem_free = free;
    ready.sample.stems.model = Some(Arc::new(Model::small(3)));
    press(&mut ready);
    vec![("sample-stems-no-model", none), ("sample-stems", ready)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::{sample_sampler, Player, ToUi};
    use mpfree_sampler_ui::stems_page::{Row, LIST, PAGE as STEMS, QLINKS, VIEW};
    use mpfree_sampler_ui::widgets::ButtonRow;
    use mpfree_ui::widgets::Label;
    use mpfree_ui::Fonts;
    use std::time::Duration;

    /// The sample kit's "Break 92" on A5, the Sample page showing.
    fn player() -> Player {
        sample_sampler(Fonts::embedded())
    }

    fn ev(p: &mut Player, widget: &str, kind: EventKind) {
        p.on_event(UiEvent {
            widget: widget.into(),
            kind,
        });
    }

    fn press_stems(p: &mut Player) {
        ev(p, send::SEND, EventKind::Tab(send::STEMS));
    }

    fn row(p: &mut Player, r: Row) {
        ev(p, LIST, EventKind::Tab(r.index()));
    }

    fn hint(p: &Player) -> String {
        p.ui.get::<Label>("hint").unwrap().text.clone()
    }

    fn stems_button(p: &Player) -> (String, bool) {
        let b = p.ui.get::<ButtonRow>(send::SEND).unwrap();
        (
            b.labels[send::STEMS].clone(),
            b.primary == Some(send::STEMS),
        )
    }

    /// The small network in the model's place: quick, its stems mean
    /// nothing.
    fn with_model(p: &mut Player) {
        p.sample.stems.model = Some(Arc::new(Model::small(3)));
    }

    fn on(p: &Player) -> &On {
        p.sample.stems.on.as_ref().expect("the Stems page")
    }

    /// Frames until the split has ended.
    fn until_done(p: &mut Player) {
        let end = Instant::now() + Duration::from_secs(120);
        while p.sample.stems.on.as_ref().is_some_and(On::splitting) {
            assert!(Instant::now() < end, "the split never ended");
            std::thread::sleep(Duration::from_millis(5));
            p.tick(0.02);
        }
    }

    fn pad_name(p: &Player, pad: usize) -> Option<String> {
        p.sample
            .project(0)
            .and_then(|k| k.pad(pad))
            .map(|pd| pd.name.clone())
    }

    #[test]
    fn stems_opens_the_page_over_the_wave_page_and_again_goes_back() {
        let mut p = player();
        assert_eq!(stems_button(&p), ("Stems".into(), false));
        press_stems(&mut p);
        assert_eq!(p.page(), STEMS);
        assert_eq!(stems_button(&p), ("Stems".into(), true), "lit");
        let s = &on(&p).page.stems;
        assert_eq!(s.sound.name, "Break 92");
        assert_eq!(s.sound.pad, Some(4));
        assert_eq!(s.sound.peaks.len(), stems_page::COLUMNS);
        assert!(
            s.sound.peaks.iter().any(|(lo, hi)| hi - lo > 0.1),
            "its picture"
        );
        // On the stick-less test player there's no model: it says so.
        assert_eq!(s.room.blocked.as_deref(), Some(NO_MODEL));
        assert_eq!(hint(&p), NO_MODEL);
        row(&mut p, Row::Go);
        assert_eq!(hint(&p), NO_MODEL, "a tap on the faint button says why");
        // The wheel walks the controls; Q-Links turn the levels.
        p.apply(ToUi::SampleQLink(1, 4));
        assert_eq!(on(&p).page.stems.levels[1], 2.0);
        press_stems(&mut p);
        assert_eq!(p.page(), crate::sample_page::PAGE);
        assert_eq!(stems_button(&p), ("Stems".into(), false));
        assert!(p.sample.stems.on.is_none() && p.sample.stems.model.is_none());
        // Chop to pads on the page: back to Wave on the Slice tool, nothing
        // chopped yet.
        press_stems(&mut p);
        let filled = |p: &Player| {
            let kit = p.sample.project(0).unwrap();
            (0..PADS)
                .filter(|&i| kit.pad(i).is_some_and(|pd| !pd.layers.is_empty()))
                .count()
        };
        let before = filled(&p);
        ev(&mut p, send::SEND, EventKind::Tab(send::CHOP_TO_PADS));
        assert_eq!(p.page(), crate::sample_page::PAGE);
        assert_eq!(p.sample.tool(), crate::sample_page::Tool::Slice);
        assert!(p.sample.stems.on.is_none());
        assert_eq!(filled(&p), before);
    }

    #[test]
    fn the_model_is_looked_for_in_the_data_folders_files() {
        let mut p = player();
        let dir = std::env::temp_dir().join(format!("mpfree-stems-model-{}", std::process::id()));
        let models = dir.join("files").join(MODELS);
        std::fs::create_dir_all(&models).unwrap();
        p.sample.set_stems_model(Some(&dir));
        press_stems(&mut p);
        assert_eq!(on(&p).page.stems.room.blocked.as_deref(), Some(NO_MODEL));
        press_stems(&mut p);
        std::fs::write(models.join(MODEL_FILE), b"MPFSTEM2").unwrap();
        press_stems(&mut p);
        assert_eq!(on(&p).page.stems.room.blocked, None);
        // A file that isn't a model: the first step says so, and nothing
        // is promised any more.
        row(&mut p, Row::Go);
        until_done(&mut p);
        assert_eq!(on(&p).page.stems.phase, Phase::Ready);
        assert!(
            hint(&p).starts_with("Stems of Break 92 not made: stem model"),
            "{}",
            hint(&p)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_split_lands_on_the_next_empty_pads_as_one_undo_step() {
        let mut p = player();
        with_model(&mut p);
        press_stems(&mut p);
        assert_eq!(on(&p).page.stems.go_text(), "Split to A6–A9");
        row(&mut p, Row::Go);
        assert!(on(&p).splitting());
        // Its memory is promised (other tests' promises come and go).
        assert!(crate::memory::promised() >= p.sample.stems_need(on(&p)));
        assert!(hint(&p).starts_with("Stop: stop the split"), "{}", hint(&p));
        // Settings wait while it splits.
        row(&mut p, Row::Put);
        assert!(hint(&p).contains("Stop it to change this"), "{}", hint(&p));
        until_done(&mut p);
        assert_eq!(on(&p).page.stems.phase, Phase::Landed("On A6–A9".into()));
        for (pad, stem) in [(5, "Vocals"), (6, "Drums"), (7, "Bass"), (8, "Other")] {
            assert_eq!(pad_name(&p, pad), Some(format!("Break 92 {stem}")));
        }
        assert!(p.sample.stems.model.is_some(), "kept for the next split");
        // A tap on a lane plays its pad; Play all four plays them together.
        p.take_actions();
        ev(&mut p, VIEW, EventKind::Select(2));
        let hits: Vec<(usize, bool)> = p
            .take_actions()
            .into_iter()
            .filter_map(|a| match a {
                FromUi::SampleHit {
                    kit: 0, pad, down, ..
                } => Some((pad, down)),
                _ => None,
            })
            .collect();
        assert_eq!(hits, [(7, true), (7, false)]);
        row(&mut p, Row::PlayAll);
        let downs = p
            .take_actions()
            .into_iter()
            .filter(|a| matches!(a, FromUi::SampleHit { down: true, .. }))
            .count();
        assert_eq!(downs, 4);
        // One UNDO takes all four back, and it can split again.
        p.apply(ToUi::SampleUndo(false));
        assert!((5..9).all(|pad| pad_name(&p, pad).is_none()));
        assert_eq!(on(&p).page.stems.phase, Phase::Ready);
        assert_eq!(on(&p).page.stems.go_text(), "Split to A6–A9");
    }

    #[test]
    fn a_landed_stems_level_turns_on_its_pad_one_undo_step_a_turn() {
        let mut p = player();
        with_model(&mut p);
        press_stems(&mut p);
        row(&mut p, Row::Go);
        until_done(&mut p);
        let level = |p: &Player| p.sample.project(0).unwrap().pad(6).unwrap().layers[0].level_db;
        assert_eq!(level(&p), 0.0);
        ev(
            &mut p,
            QLINKS,
            EventKind::Step {
                index: 1,
                delta: -4,
            },
        );
        p.apply(ToUi::SampleQLink(1, -2));
        assert_eq!(level(&p), -3.0, "drums: Q-Link 2");
        p.apply(ToUi::SampleUndo(false));
        assert_eq!(level(&p), 0.0, "the turn was one step");
        assert_eq!(pad_name(&p, 6).as_deref(), Some("Break 92 Drums"));
    }

    #[test]
    fn behind_other_pages_the_split_goes_on_and_the_button_says_how_far() {
        let mut p = player();
        with_model(&mut p);
        press_stems(&mut p);
        row(&mut p, Row::Go);
        press_stems(&mut p);
        assert_eq!(p.page(), crate::sample_page::PAGE, "back to Wave");
        assert!(stems_button(&p).0.starts_with("Stems ") && stems_button(&p).0.ends_with(" %"));
        // Stems again shows the split.
        press_stems(&mut p);
        assert_eq!(p.page(), STEMS);
        assert!(on(&p).splitting());
        press_stems(&mut p);
        until_done(&mut p);
        assert_eq!(pad_name(&p, 8).as_deref(), Some("Break 92 Other"), "landed");
        assert_eq!(stems_button(&p), ("Stems".into(), false));
        assert!(
            p.sample.stems.on.is_none() && p.sample.stems.model.is_none(),
            "let go"
        );
    }

    #[test]
    fn stop_ends_the_split_and_nothing_lands() {
        let mut p = player();
        with_model(&mut p);
        press_stems(&mut p);
        row(&mut p, Row::Go);
        assert_eq!(on(&p).page.stems.go_text(), "Stop");
        row(&mut p, Row::Go);
        assert_eq!(on(&p).page.stems.phase, Phase::Ready);
        assert!(!on(&p).splitting());
        let end = Instant::now() + Duration::from_secs(60);
        while p.sample.jobs.busy() {
            assert!(Instant::now() < end, "its step never ended");
            std::thread::sleep(Duration::from_millis(5));
            p.tick(0.02);
        }
        assert!((5..9).all(|pad| pad_name(&p, pad).is_none()));
    }

    #[test]
    fn new_kits_stop_a_split_and_close_the_page() {
        let mut p = player();
        with_model(&mut p);
        press_stems(&mut p);
        row(&mut p, Row::Go);
        assert!(on(&p).splitting());
        // New project: the factory kits again.
        assert!(p.sample.reset_kits(&mut p.ui));
        assert!(p.sample.stems.on.is_none() && p.sample.stems.model.is_none());
        assert!(p
            .sample
            .take_lines()
            .iter()
            .any(|l| l == "sample: stems of Break 92 stopped: the kits changed"));
        let end = Instant::now() + Duration::from_secs(60);
        while p.sample.jobs.busy() {
            assert!(Instant::now() < end, "its step never ended");
            std::thread::sleep(Duration::from_millis(5));
            p.tick(0.02);
        }
        assert!(
            (5..9).all(|pad| pad_name(&p, pad).is_none()),
            "nothing landed"
        );
    }

    #[test]
    fn put_on_library_hands_the_app_the_stems_to_save() {
        let mut p = player();
        with_model(&mut p);
        p.sample.files = true;
        press_stems(&mut p);
        // Put on: Pads, Layers, Takes, New kit, Library.
        for _ in 0..4 {
            ev(
                &mut p,
                LIST,
                EventKind::Step {
                    index: Row::Put.index(),
                    delta: 1,
                },
            );
        }
        assert_eq!(on(&p).page.stems.put, Put::Library);
        ev(&mut p, VIEW, EventKind::Toggle(0));
        row(&mut p, Row::Go);
        // The app takes them before the next frame ends; here they're
        // taken straight from the page.
        let end = Instant::now() + Duration::from_secs(120);
        let saves = loop {
            let s = p.sample.take_stem_saves();
            if !s.is_empty() {
                break s;
            }
            assert!(Instant::now() < end, "never landed");
            std::thread::sleep(Duration::from_millis(5));
            p.sample.tick(&mut p.ui, 0.02, true);
        };
        assert_eq!(saves.len(), 1);
        let (name, sounds) = &saves[0];
        assert_eq!(name, "Break 92");
        let names: Vec<&str> = sounds.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Break 92 Drums", "Break 92 Bass", "Break 92 Other"]);
        assert!(
            (5..9).all(|pad| pad_name(&p, pad).is_none()),
            "the kit stays"
        );
    }
}
