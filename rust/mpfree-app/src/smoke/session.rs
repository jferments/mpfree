//! A whole MPFree session without the MPC: the screen ([`Player`]) and the
//! control side behind it (the app, the mixer, the time engine, the looper
//! and the chain editor) wired as `player_session` wires them, so a test can
//! do what a musician does, step by step, and check both the screen and the
//! engines after each step.
//!
//! What goes through the same code as on the MPC: every touch (through
//! [`Player::touch`]), every screen action and app effect for the control
//! side ([`crate::desk::Desk`], which `player_session` calls too), the
//! buttons, the wheel and the Q-Link cards (through [`App`]'s own entry
//! points), and the time engine's periods (with the per-track arps, as
//! `player_session` runs them).
//!
//! What is mirrored here, because it is the audio loop's and needs the
//! sound card or the kits on disk: the pads (as `player_session` hands them
//! to the app and the time engine), the kit changes, the synth's settings,
//! and the boxes the screen lends the audio thread (the Sample tab's kit
//! views, the preview, the line input, the grain cloud), which come back at
//! once. No sound is rendered: the time engine's notes are counted as hits.

use crate::app::{self, App, Effect, View};
use crate::gui::{self, FromUi, Player, ToUi};
use crate::{
    audiotracks, cores, desk, looping, midi_in, mixing, modpanels, synth, time, undo, Out,
};
use mpfree_ui::{Fonts, PageTarget, Tone};
use std::sync::mpsc::{sync_channel, Receiver};
use std::time::{Duration, Instant};

const RATE: u32 = crate::RATE;
const PERIOD: usize = crate::PERIOD;
/// How long one period is, in seconds.
pub const PERIOD_S: f64 = PERIOD as f64 / RATE as f64;
/// The device's default level: 6 dB under a 0 dB top (`--gain-db`).
const MAX_DB: f32 = 0.0;

/// The time engine's notes, counted (the sampler would play them).
#[derive(Default)]
pub struct Hits {
    /// (pad, velocity) for every note the clips and the arps started.
    pub on: Vec<(usize, u8)>,
    /// Every note started, with the track it came from and when.
    pub played: Vec<Played>,
    /// The session's time at the period's start (seconds).
    base: f64,
    /// This period's events as MIDI Out would read them, for an export's
    /// MIDI file: handed to the export's tap after the period, as
    /// main.rs's pads hand them over (export.rs).
    tapped: Vec<(mpfree_play::Out, crate::midi_out::NoteIs<'static>)>,
    /// This period's hits and releases (pad, velocity, offset), for the
    /// sounding kit when the session has one ([`Session::make_sounds`]).
    queued: Vec<(usize, u8, u32)>,
    released: Vec<(usize, u32)>,
    /// This period's locks on a kit pad's sound (address, value, offset),
    /// for the sounding kit, which takes them before the period's hits.
    lock_queue: Vec<(mpfree_play::mpfree_param::ParamAddr, Option<f32>, u32)>,
    /// Every lock on a pad's sound or an instrument the clips played, in
    /// order (`None`: its end), as main.rs's pads take them.
    pub locked: Vec<(mpfree_play::mpfree_param::ParamAddr, Option<f32>)>,
    /// While an export's replay plays one track alone: the other tracks'
    /// notes are kept quiet, as main.rs's pads keep them (`export::quiets`;
    /// every track plays the pads here).
    pub only: Option<mpfree_export::Only>,
}

/// A note the time engine started, with its track: what a test of a whole
/// song reads to see which tracks played in which section.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Played {
    pub track: usize,
    /// The pad (or note) it played.
    pub note: u8,
    pub velocity: u8,
    /// Seconds into the session.
    pub at: f64,
}

impl Hits {
    /// Kept quiet by an export's replay of another track.
    fn quiet(&self, o: &mpfree_play::Out) -> bool {
        crate::export::quiets(self.only, o.track, true, o.event)
    }

    fn note(&mut self, o: &mpfree_play::Out) {
        if let mpfree_play::Event::NoteOn { note, velocity } = o.event {
            self.played.push(Played {
                track: usize::from(o.track),
                note,
                velocity,
                at: self.base + f64::from(o.offset) / f64::from(RATE),
            });
        }
    }
}

impl time::Pads for Hits {
    fn hit_at(&mut self, _track: u16, pad: usize, velocity: u8, offset: u32) -> bool {
        self.on.push((pad, velocity));
        self.queued.push((pad, velocity, offset));
        true
    }
    fn release_at(&mut self, _track: u16, pad: usize, offset: u32) {
        self.released.push((pad, offset));
    }
    fn stop_all(&mut self) {}
    fn pressure_at(&mut self, _track: u16, _pad: usize, _value: u8, _offset: u32) {}
    // A pad's sound or an instrument takes its locks, as main.rs's pads do.
    fn lock_at(
        &mut self,
        addr: mpfree_play::mpfree_param::ParamAddr,
        value: Option<f32>,
        offset: u32,
    ) -> bool {
        use mpfree_play::mpfree_param::Owner;
        match addr.owner {
            Owner::Pad(..) if crate::sampler::pad_field(addr.param).is_some() => {
                self.lock_queue.push((addr, value, offset));
            }
            Owner::Inst(_) => {}
            _ => return false,
        }
        self.locked.push((addr, value));
        true
    }
    // Every event goes on as usual; the notes are written down first.
    fn out(&mut self, o: &mpfree_play::Out) -> bool {
        use mpfree_play::Event;
        if self.quiet(o) {
            return true;
        }
        self.note(o);
        let pad = match o.event {
            Event::NoteOn { note, .. } | Event::NoteOff { note } | Event::Pressure { note, .. } => {
                usize::from(note)
            }
            _ => 0,
        };
        self.tapped.push((
            *o,
            crate::midi_out::NoteIs::Pad(crate::midi_out::kit_notes(pad)),
        ));
        false
    }
    fn live(&mut self, o: &mpfree_play::Out) -> Option<mpfree_play::Event> {
        use crate::midi_out::{kit_notes, NoteIs};
        use mpfree_play::Event;
        if self.quiet(o) {
            return None;
        }
        self.note(o);
        // As main.rs's pads: the MIDI file gets what MIDI Out would.
        let is = match o.event {
            Event::NoteOn { note, .. } if note < 16 => NoteIs::Pad(kit_notes(usize::from(note))),
            _ => NoteIs::Pad(&[]),
        };
        self.tapped.push((*o, is));
        match o.event {
            Event::NoteOn { note, velocity } => {
                self.on.push((usize::from(note), velocity));
                self.queued.push((usize::from(note), velocity, o.offset));
                Some(o.event)
            }
            Event::NoteOff { note } => {
                self.released.push((usize::from(note), o.offset));
                Some(o.event)
            }
            _ => None,
        }
    }
}

/// The screen and the control side, as on the MPC.
pub struct Session {
    pub screen: Player,
    pub app: App,
    pub mix: mixing::MixHost,
    pub time: time::TimeHost,
    pub panels: modpanels::Panels,
    pub looper: looping::LoopHost,
    /// The tempo a free first loop set, for the app at the next period
    /// (main.rs's `loop_tempo`).
    loop_tempo: Option<f64>,
    /// The audio tracks on the Song view's lanes (a short memory: the
    /// tests record no takes).
    pub tracks: audiotracks::TrackHost,
    pub music: midi_in::MusicIn,
    pub synth: cores::SynthSlot,
    pub level_db: f32,
    /// Every line the control side wrote to the log, in order.
    pub lines: Vec<String>,
    /// Every action the screen sent, in order (short form).
    pub sent: Vec<String>,
    /// The buttons' lights as last sent (by button number).
    pub lights: [u8; 128],
    /// The lights that reach the controller, as main.rs sends them: only
    /// those Akai's app was seen lighting (`akai::app_lights`), so a light
    /// for a made-up button number never shows here.
    pub sent_lights: [u8; 128],
    /// The notes the time engine played.
    pub hits: Hits,
    /// Following another device's clock ([`Session::clock`]), and its
    /// bytes for the next period.
    follow: Option<time::Follow>,
    clock_bytes: Vec<u8>,
    /// A kit that sounds, when a test asks for one ([`Session::make_sounds`]):
    /// the pads then play into the mixer, as main.rs's sampler plays them.
    pub sounds: Option<crate::sampler::Player>,
    /// The sound loop's side of an export (export.rs).
    pub export: crate::export::Host,
    /// Every period's master output, while a test asks for it.
    pub heard: Option<Vec<f32>>,
    /// Controls a walk pressed that did nothing (see `walk.rs`).
    pub dead: Vec<String>,
    /// The kit the pads play: the armed track's (the synth is the last).
    pub kit: usize,
    /// Whether the armed track plays the synth (the pads play it then).
    pub synth_on: bool,
    /// What each track plays, as main.rs keeps it.
    pub track_kits: crate::sampler::TrackKits,
    /// The project's instruments, as main.rs keeps them.
    pub instrument_list: crate::instrument_list::InstrumentList,
    /// The tracks' own instruments and sends the screen was last told of,
    /// as main.rs keeps them.
    sounds_shown: gui::TrackSounds,
    sends_shown: gui::TrackSends,
    /// The time engine's REDOs seen so far, as player_session keeps them.
    redos_seen: u64,
    /// Messages for the screen that found it gone (none expected).
    pub quit: bool,
    kits: usize,
    /// STEP SEQ's pads held as steps (step_keys.rs).
    step_keys: crate::step_keys::StepKeys,
    /// The pads pressed as TRACK MUTE's keys, so their release is theirs
    /// too.
    mute_pressed: [bool; 16],
    /// The synth chosen for the pads, as main.rs keeps it: the pads play
    /// the shared synth on their own while the armed track plays something
    /// else, the pads held on it, and every hit it played (pad, velocity).
    preview_on: bool,
    preview_held: [bool; 16],
    pub previewed: Vec<(usize, u8)>,
    /// Loop pads' hits waiting for the launch line (sampler R5.3b), as
    /// player_session keeps them: the pads pressed but not yet sounded,
    /// how many found the table full, and how many STOP dropped.
    awaiting: [bool; 16],
    waits_full: u64,
    stop_dropped: usize,
    waiting_shown: u16,
    /// The loop keys held (SHIFT + SAMPLER on the Looper tab).
    keys_held: crate::perform_pads::Held,
    /// Controls learned on the looper (its footswitch), as player_session
    /// keeps them.
    remote: crate::remote::Remote,
    /// Keys learned on the Perform pads (Learn a key), as player_session
    /// keeps them.
    perform_keys: crate::remote::PerformKeys,
    out: Out,
    to_screen: Receiver<ToUi>,
    /// The app's effects waiting for the desk (a test may add some).
    pub fx: Vec<Effect>,
    kit_to: Option<usize>,
    /// A kit picked in the list (or turned with SHIFT + the wheel) since
    /// the kit last changed: the pads follow the track (main.rs's own).
    kit_picked: bool,
    /// The boxes the screen lent the audio thread.
    preview: Option<Box<crate::preview::Preview>>,
    rec_feed: Option<Box<crate::sampler::RecFeed>>,
    /// The line input's sound ([`Session::line_in`]): a sine of this many
    /// Hz, and where it is in its cycle.
    line_tone: Option<f32>,
    line_phase: f32,
    /// The recorder's Gain on the line input, linear, and where the last
    /// period's ramp ended (as main.rs).
    rec_gain: (f32, f32),
    cloud: Option<Box<crate::sampler::Cloud>>,
    /// Simulated time: periods run, and the clock the rate limits read.
    periods: u64,
    t0: Instant,
    /// The screen as last drawn (only what changed is drawn again).
    canvas: mpfree_ui::SoftCanvas,
    /// UNDO on the Synth tab and the mixer: their turns, as the audio loop
    /// keeps them, and whether each had something to undo when last told.
    synth_turns: undo::Turns<u16>,
    /// The presets the tracks' own synths play, and the preset picker's
    /// try (presets.rs), as main.rs keeps them.
    presets: crate::presets::TrackPresets,
    mix_turns: crate::mixturns::MixTurns,
    turns_told: (bool, bool),
    /// The library the browser saves projects into (`with_library`).
    pub library: Option<mpfree_library::Library>,
    /// A save asked for (SHIFT + BROWSE, Save in the browser), and the
    /// Looper's loops being read for it, as `player_session` keeps them.
    save_asked: bool,
    loops_saving: bool,
    audio_saving: bool,
    collect_now: bool,
    loops_held: Option<mpfree_looper::save::Saved>,
    audio_held: Option<mpfree_audiotracks::save::Saved>,
}

/// Every hardware button with a job, on a number of its own: the measured
/// ones as measured (REC, OVERDUB, UNDO and PLAY START among them, as on a
/// fresh MPC), the rest made up (on the MPC the control mapper finds them).
fn map_every_button(app: &mut App) {
    let mut next = 90u8;
    for c in app::mapper::CONTROLS.iter() {
        if matches!(c.job, app::mapper::Job::None | app::mapper::Job::Fixed(_)) {
            continue;
        }
        if app.map.of(c.job).is_none() {
            app.map.learn(c.job, Some(next));
            next += 1;
        }
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// A session as `player_session` starts one: six kits and the synth, the
    /// app with every button known, and the screen on the Play page.
    pub fn new() -> Session {
        let (tx, to_screen) = sync_channel::<ToUi>(1 << 16);
        let out = Out {
            console: None,
            sink: Some(tx),
            dropped: 0,
            report: None,
            queued: None,
            last_draw: Instant::now(),
        };
        let mut app = App::new(RATE, false);
        map_every_button(&mut app);
        let level_db = MAX_DB - 6.0;
        let synth = cores::SynthSlot::new(synth::Synth::new(RATE));
        let mut kits = gui::sample_kits();
        let n = kits.len();
        kits.push(synth.kit_view());
        let time = time::TimeHost::new(RATE, app.settings.bpm).expect("the time engine starts");
        let mix = mixing::MixHost::new(RATE, level_db, MAX_DB);
        let panels = modpanels::Panels::new();
        let music = midi_in::MusicIn::new();
        let mut s = Session {
            screen: Player::new(Fonts::embedded(), gui::SCREEN, false, None),
            app,
            mix,
            time,
            panels,
            looper: looping::LoopHost::new(RATE, 20),
            tracks: audiotracks::TrackHost::new(RATE, 2),
            music,
            synth,
            level_db,
            lines: Vec::new(),
            sent: Vec::new(),
            follow: None,
            clock_bytes: Vec::new(),
            lights: [0; 128],
            sent_lights: [0; 128],
            hits: Hits::default(),
            sounds: None,
            export: crate::export::Host::new(),
            heard: None,
            dead: Vec::new(),
            kit: 0,
            synth_on: false,
            track_kits: crate::sampler::TrackKits::new(n),
            instrument_list: crate::instrument_list::InstrumentList::new(),
            sounds_shown: [None; crate::instruments::TRACKS],
            sends_shown: [None; crate::instruments::TRACKS],
            redos_seen: 0,
            quit: false,
            kits: n,
            step_keys: crate::step_keys::StepKeys::default(),
            mute_pressed: [false; 16],
            preview_on: false,
            preview_held: [false; 16],
            previewed: Vec::new(),
            awaiting: [false; 16],
            waits_full: 0,
            stop_dropped: 0,
            waiting_shown: 0,
            keys_held: crate::perform_pads::Held::default(),
            perform_keys: crate::remote::PerformKeys::default(),
            remote: crate::remote::Remote::new(),
            loop_tempo: None,
            out,
            to_screen,
            fx: Vec::new(),
            kit_to: None,
            kit_picked: false,
            preview: None,
            rec_feed: None,
            line_tone: None,
            line_phase: 0.0,
            rec_gain: (1.0, 1.0),
            cloud: None,
            periods: 0,
            t0: Instant::now(),
            canvas: mpfree_ui::SoftCanvas::new(gui::SCREEN.0, gui::SCREEN.1),
            synth_turns: undo::Turns::default(),
            presets: crate::presets::TrackPresets::default(),
            mix_turns: undo::Turns::default(),
            turns_told: (false, false),
            library: None,
            save_asked: false,
            loops_saving: false,
            audio_saving: false,
            collect_now: false,
            loops_held: None,
            audio_held: None,
        };
        // What player_session shows before the sound starts.
        s.out.show(ToUi::Kits(kits));
        s.out.show(ToUi::Synth(s.synth.state()));
        let mut projects = vec![mpfree_sampler_ui::sample::project().0];
        projects.resize_with(n, mpfree_sampler::model::Project::new);
        s.out.show(ToUi::SampleKits {
            projects,
            rate: mpfree_sampler_ui::sample::RATE,
        });
        s.out.show(ToUi::Session("Akai session".into(), Tone::Good));
        s.out.show(ToUi::Settings(s.app.settings.clone()));
        s.out
            .show(ToUi::Time(Box::new(crate::time_state(&s.time, &s.tracks))));
        s.out.show(ToUi::Level { db: s.level_db });
        s.out
            .show(ToUi::Macros(Box::new(s.panels.macros_state(&s.mix))));
        s.out.show(ToUi::PlayLevels(s.mix.play_levels()));
        s.out.show(ToUi::IoModel(midi_in::io_model()));
        s.fx = s.app.start();
        s.settle();
        // The first fifth of a second: the built-in inserts (the DJ
        // filters, Perform FX) land, as they do before anyone can reach
        // the screen.
        s.run_for(0.2);
        s
    }

    /// A session with a library in `dir`, as `--data` gives the app one:
    /// BROWSE, Save, Open and New project work, as on the MPC. The
    /// browser's jobs run at once (no worker thread), so a test needs no
    /// waiting.
    pub fn with_library(dir: &std::path::Path) -> Session {
        let mut s = Session::new();
        let lib = mpfree_library::Library::at(dir).expect("a library in the scratch folder");
        s.screen
            .set_browser(mpfree_browser_ui::Browser::inline(lib.clone()), "");
        s.library = Some(lib);
        // What New project goes back to, as player_session sends it once
        // the mixer's own inserts are in.
        let d = s.defaults();
        s.out.show(ToUi::ProjectDefaults(Box::new(d)));
        s.settle();
        s
    }

    /// The engines' parts as they are now, as `player_session` makes New
    /// project's (call it on a fresh session for those).
    pub fn defaults(&self) -> crate::project::Defaults {
        crate::project::Defaults {
            tempo: self.app.settings.bpm,
            mixer: crate::project::mixer_text(
                &self.mix.mixer,
                self.mix.pad_mutes(),
                self.mix.pad_solos(),
            ),
            modu: self.mix.modu.save(),
            synth: crate::project::synth_text(&self.app.synth_settings(), &self.synth.sounds()),
        }
    }

    /// How many drum kits there are (the synth comes after them).
    pub fn kits(&self) -> usize {
        self.kits
    }

    /// Seconds into the session.
    pub fn t(&self) -> f64 {
        self.periods as f64 * PERIOD_S
    }

    fn now(&self) -> Instant {
        self.t0 + Duration::from_secs_f64(self.t())
    }

    fn desk(&mut self) -> (desk::Desk<'_>, &mut Out) {
        (
            desk::Desk {
                app: &mut self.app,
                mix: &mut self.mix,
                time: &mut self.time,
                panels: &mut self.panels,
                looper: &mut self.looper,
                tracks: &mut self.tracks,
                music: &mut self.music,
                level_db: &mut self.level_db,
                max_db: MAX_DB,
                card: 0,
            },
            &mut self.out,
        )
    }

    // ------------------------------------------------------------ the screen

    /// The page the screen shows ("Play", "Sample · Play", ...).
    pub fn page(&self) -> String {
        self.screen.page().to_string()
    }

    pub fn view(&self) -> View {
        self.screen.view()
    }

    /// The controls on the page now, as drawn (some widgets measure their
    /// controls as they paint, so the page is drawn first).
    pub fn controls(&mut self) -> Vec<PageTarget> {
        self.screen.ui.paint(&mut self.canvas);
        self.screen.ui.targets()
    }

    /// The control `widget/id` on the page whose label is `label` (or the
    /// first one, for an empty label).
    pub fn control(&mut self, widget: &str, id: &str, label: &str) -> Option<PageTarget> {
        self.controls().into_iter().find(|o| {
            o.widget == widget && o.target.id == id && (label.is_empty() || o.target.label == label)
        })
    }

    /// Show the steps (`"Beats"`: the Sequencer tab's Steps, which were
    /// Play's Beats view) or Play (`"All"`, its one view since the looper
    /// redesign's step 5), as a musician would: the tab.
    pub fn play_view(&mut self, name: &str) -> &mut Session {
        if name == "Beats" {
            return self.press("header", "tab", "Sequencer");
        }
        if self.view() != View::Play {
            self.press("header", "tab", "Play");
        }
        self
    }

    /// Arm the loop for REC as a musician would on Play: the loop row's
    /// Arm pill (Play's Loop view did it before the looper redesign's step
    /// 5). Already armed, it leaves it.
    pub fn arm_loop(&mut self) -> &mut Session {
        self.play_view("All");
        if self
            .control(crate::loop_row::LOOP_ROW, "arm", "Arm")
            .is_some()
        {
            self.press(crate::loop_row::LOOP_ROW, "arm", "Arm");
        }
        self
    }

    /// Press the control `widget/id` labelled `label` the way its gesture
    /// says (a tap, a drag or a long press), then let the session run a few
    /// periods. Panics, naming what the page has, when it is not there.
    pub fn press(&mut self, widget: &str, id: &str, label: &str) -> &mut Session {
        let Some(c) = self.control(widget, id, label) else {
            let have: Vec<String> = self
                .controls()
                .iter()
                .map(|o| format!("{}/{} \"{}\"", o.widget, o.target.id, o.target.label))
                .collect();
            panic!(
                "no {widget}/{id} \"{label}\" on {}; it has:\n{}",
                self.page(),
                have.join("\n")
            );
        };
        self.press_target(&c.target)
    }

    /// Rest a finger on the control `widget/id` labelled `label` for a
    /// second before lifting it, whatever its gesture (a step held on the
    /// Sequencer's Steps picks it), then let the session run a few
    /// periods. Panics when it is not there.
    pub fn hold_press(&mut self, widget: &str, id: &str, label: &str) -> &mut Session {
        let mut t = self
            .control(widget, id, label)
            .unwrap_or_else(|| panic!("no {widget}/{id} \"{label}\" on {}", self.page()))
            .target;
        t.gesture = mpfree_ui::Gesture::Hold;
        self.press_target(&t)
    }

    /// Press `t` as a finger would, and let the session run a few periods.
    /// Opens the list of kits as a musician would: what the pads play on
    /// Play's transport, or from any other page the Pads tab's Kit (the
    /// top bar's kit chip went in the Play, Pads and Sequencer redesign).
    pub fn open_kits(&mut self) -> &mut Session {
        if let Some(c) = self.control(crate::play_page::TRANSPORT, "pads", "") {
            return self.press_target(&c.target);
        }
        self.press("header", "tab", "Pads");
        let kit = self
            .control(mpfree_sampler_ui::pads_tab::BAR, "kit", "")
            .expect("the Pads tab's Kit")
            .target;
        self.press_target(&kit)
    }

    /// What the pads play, as Play's transport says it ("Track 1 ·
    /// Classic House · A"); empty off Play.
    pub fn pads_play(&mut self) -> String {
        self.control(crate::play_page::TRANSPORT, "pads", "")
            .map(|c| c.target.label)
            .unwrap_or_default()
    }

    /// Track `track` gets its own instrument, a copy of the synth's sound
    /// for `engine`, as the builder thread would put it in (the session
    /// builds none itself), and the app hears of it.
    pub fn give_own(&mut self, track: usize, engine: crate::synth::Engine) -> &mut Session {
        let (id, extras, values) = self.synth.sound_of(engine);
        let plugins = crate::plugins::Plugins::none();
        let made = crate::instruments::make(&plugins, id, &extras, &values, RATE, PERIOD)
            .expect("an engine's copy");
        let pads = self.synth.pads_for(engine);
        let tuning = self.synth.tuning_now();
        assert!(self.mix.instruments.put(track, made, pads, &tuning).is_ok());
        let held = self.mix.instruments.held(track);
        self.app.set_track_sound(track, held);
        self
    }

    /// Picks the synth from the list of kits (its last row) for the armed
    /// track, as taps would ([`Session::open_kits`]).
    pub fn pick_synth(&mut self) -> &mut Session {
        self.open_kits();
        let row = self
            .controls()
            .into_iter()
            .rfind(|c| c.widget == "kits" && c.target.id == "row")
            .expect("the synth's row");
        self.press_target(&row.target).run_for(0.2)
    }

    pub fn press_target(&mut self, t: &mpfree_ui::Target) -> &mut Session {
        let acts = super::press(&mut self.screen, t);
        for a in acts {
            self.route(a);
        }
        self.run_periods(4)
    }

    /// The screen as drawn now, as RGB bytes.
    pub fn pixels(&mut self) -> Vec<u8> {
        self.screen.ui.paint(&mut self.canvas);
        self.canvas.to_rgb()
    }

    // ------------------------------------------------------- the hardware

    /// The number of the hardware button the control mapper calls `name`
    /// ("REC", "PLAY", "PAD BANK A/E", ...).
    pub fn button_note(&self, name: &str) -> u8 {
        let c = app::mapper::CONTROLS
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no hardware control called {name}"));
        self.app.map.of(c.job).unwrap_or_else(|| {
            panic!("{name} has no job yet (the mapper writes its number down only)")
        })
    }

    /// Press and let go of a hardware button.
    pub fn button(&mut self, name: &str) -> &mut Session {
        let note = self.button_note(name);
        let t = self.t();
        let held =
            crate::erase_held_steps(&self.app, &mut self.step_keys, &mut self.time, note, true);
        self.fx.extend(held);
        let fx = self.app.button(note, true, t);
        self.fx.extend(fx);
        self.run_periods(2);
        let t = self.t();
        let fx = self.app.button(note, false, t);
        self.fx.extend(fx);
        self.run_periods(4)
    }

    /// A kit that sounds on the pads: pad `p` a short decaying tone of
    /// its own pitch (220 Hz times 2^(p/12)), so the mixer, the mixdown
    /// and the stems carry what the clips play.
    pub fn make_sounds(&mut self) -> &mut Session {
        let notes = mpfree_controller::mpc_one::PAD_NOTES;
        let tones: Vec<Vec<i16>> = (0..16)
            .map(|p| {
                let hz = 220.0 * 2f64.powf(f64::from(p) / 12.0);
                let n = (RATE / 10) as usize;
                (0..n)
                    .flat_map(|i| {
                        let t = i as f64 / f64::from(RATE);
                        let x = (t * hz * std::f64::consts::TAU).sin() * (-t * 30.0).exp();
                        let v = (x * 12000.0) as i16;
                        [v, v]
                    })
                    .collect()
            })
            .collect();
        let names: Vec<String> = (0..16).map(|p| format!("Tone {}", p + 1)).collect();
        let sounds: Vec<(u8, &str, &[i16])> = (0..16)
            .map(|p| (notes[p], names[p].as_str(), &tones[p][..]))
            .collect();
        let file = crate::kit::build_kits(&[("Tones", crate::kit::build(&sounds))]);
        let mut kits = crate::kit::parse_kits(&file).expect("the tones kit");
        self.sounds = Some(crate::sampler::Player::new(&mut kits, &notes, RATE).expect("a player"));
        self
    }

    /// Follows another device's clock with these options (None: not), as
    /// the project's Tempo from does, and hands the time engine `bytes`
    /// from it at the next period's top.
    pub fn clock(&mut self, follow: Option<time::Follow>, bytes: &[u8]) -> &mut Session {
        self.follow = follow;
        self.clock_bytes.extend_from_slice(bytes);
        self.run_periods(1)
    }

    /// Runs the session until the screen says a line starting with
    /// `prefix` (after now), for at most `seconds` of the session's time
    /// and of the wall clock both; the line if it did.
    pub fn until_said_like(&mut self, prefix: &str, seconds: f64) -> Option<String> {
        let from = self.said().len();
        let end = self.t() + seconds;
        let wall = std::time::Instant::now() + Duration::from_secs_f64(seconds);
        loop {
            if let Some(l) = self
                .said()
                .into_iter()
                .skip(from)
                .find(|l| l.starts_with(prefix))
            {
                return Some(l);
            }
            if self.t() >= end && std::time::Instant::now() >= wall {
                return None;
            }
            self.run_periods(8);
        }
    }

    /// What the screen said in its events log (the project's saves and
    /// opens, as a musician reads them), oldest first.
    pub fn said(&self) -> Vec<String> {
        self.screen
            .ui
            .get::<mpfree_ui::widgets::LogPanel>("events")
            .map(|e| e.lines().map(|(l, _)| l.clone()).collect())
            .unwrap_or_default()
    }

    /// Runs the session until the screen says `line`, for at most
    /// `seconds` of the session's time and of the wall clock both (the
    /// library writes on its own thread, in real time); true if it did.
    pub fn until_said(&mut self, line: &str, seconds: f64) -> bool {
        let end = self.t() + seconds;
        let wall = std::time::Instant::now() + Duration::from_secs_f64(seconds);
        while self.t() < end || std::time::Instant::now() < wall {
            if self.said().iter().any(|l| l == line) {
                return true;
            }
            self.run_periods(8);
            std::thread::sleep(Duration::from_millis(2));
        }
        self.said().iter().any(|l| l == line)
    }

    /// Hold `held` (SHIFT, say) while pressing `name`.
    pub fn button_with(&mut self, held: &str, name: &str) -> &mut Session {
        let h = self.button_note(held);
        let t = self.t();
        let fx = self.app.button(h, true, t);
        self.fx.extend(fx);
        self.run_periods(1);
        self.button(name);
        let t = self.t();
        let fx = self.app.button(h, false, t);
        self.fx.extend(fx);
        self.run_periods(4)
    }

    /// Hold a hardware button down for `seconds` (FILL through a bar, say),
    /// the session running meanwhile.
    pub fn hold(&mut self, name: &str, seconds: f64) -> &mut Session {
        let note = self.button_note(name);
        let t = self.t();
        let fx = self.app.button(note, true, t);
        self.fx.extend(fx);
        self.run_for(seconds);
        let t = self.t();
        let fx = self.app.button(note, false, t);
        self.fx.extend(fx);
        self.run_periods(4)
    }

    /// Turn Q-Link `knob` (0 to 3) by `delta` on the controller, routed by
    /// page as player_session routes it (the relative port).
    pub fn qlink(&mut self, knob: usize, delta: i32) -> &mut Session {
        let page = self.app.view();
        if let Some(m) = crate::desk::qlink_to_screen(page, knob, delta)
            .filter(|_| !matches!(page, View::Play | View::Sequencer | View::Arrange))
        {
            self.out.show(m);
        } else if let Some(e) = self.app.qlink(knob, delta, None) {
            self.fx.extend(e);
        } else if let Some(m) = crate::desk::qlink_to_screen(page, knob, delta) {
            self.out.show(m);
        } else if knob == 0 {
            self.level_db = (self.level_db + crate::LEVEL_STEP_DB * delta as f32)
                .clamp(crate::LEVEL_MIN_DB, MAX_DB);
            self.mix.set_level(self.level_db);
            self.out.show(ToUi::Level { db: self.level_db });
        }
        self.run_periods(4)
    }

    /// Turn the data wheel.
    pub fn wheel(&mut self, delta: i32) -> &mut Session {
        let fx = self.app.wheel(delta);
        self.fx.extend(fx);
        self.run_periods(4)
    }

    /// Hold `held` (SHIFT, say) while turning the data wheel.
    pub fn wheel_with(&mut self, held: &str, delta: i32) -> &mut Session {
        let h = self.button_note(held);
        let t = self.t();
        let fx = self.app.button(h, true, t);
        self.fx.extend(fx);
        self.run_periods(1);
        self.wheel(delta);
        let t = self.t();
        let fx = self.app.button(h, false, t);
        self.fx.extend(fx);
        self.run_periods(4)
    }

    /// A hardware button pressed (`down`) or let go, for chords with the
    /// pads (ERASE + a pad).
    pub fn button_edge(&mut self, name: &str, down: bool) -> &mut Session {
        let note = self.button_note(name);
        let t = self.t();
        let held =
            crate::erase_held_steps(&self.app, &mut self.step_keys, &mut self.time, note, down);
        self.fx.extend(held);
        let fx = self.app.button(note, down, t);
        self.fx.extend(fx);
        crate::live_erase_let_go(&self.app, &mut self.time);
        self.run_periods(2)
    }

    /// A USB keyboard's key pressed (`velocity` Some) or let go, as
    /// `player_session` hands it over while the synth has the pads: it
    /// plays the synth by note, and with step pads of the line held it
    /// goes on them.
    pub fn key_edge(&mut self, note: u8, velocity: Option<u8>) -> &mut Session {
        match velocity {
            Some(v) => self.app.key_down(note, v),
            None => self.app.key_up(note),
        }
        if let Some(p) = crate::step_key(
            &self.app,
            &mut self.step_keys,
            &mut self.time,
            note,
            velocity,
        ) {
            if !p.log.is_empty() {
                self.out.note(&p.log);
            }
            if let Some(t) = p.tell {
                self.fx.push(Effect::Tell(t));
            }
        }
        self.run_periods(2)
    }

    /// Hit a pad on the controller (down, then up after `hold` seconds).
    /// A control learned on looper parameter `id` (a footswitch) moved to
    /// `value` (0 to 1), as player_session takes MIDI Learn's: on any page.
    pub fn footswitch(&mut self, id: u16, value: f32) -> &mut Session {
        let now = crate::desk::looper_now(&self.looper);
        let t = self.t();
        let act = self.remote.looper_write(id, value, now, t);
        let fx =
            crate::desk::looper_remote(&mut self.app, &self.time, &self.looper, &mut self.out, act);
        self.fx.extend(fx);
        self.run_periods(2)
    }

    /// A control learned on Play's own control `id` (the Learn section's
    /// Play card) moved to `value` (0 to 1), as player_session takes MIDI
    /// Learn's: on any page.
    pub fn play_knob(&mut self, id: u16, value: f32) -> &mut Session {
        let mut moves = crate::remote::PlayMoves::default();
        moves.set(id, value);
        let fx = crate::desk::play_remote(&self.time, &mut self.mix, &mut self.out, &mut moves);
        self.fx.extend(fx);
        self.run_periods(2)
    }

    /// A key learned on Perform parameter `id` (Learn a key) moved to
    /// `value` (0 to 1), as player_session takes MIDI Learn's: on any page.
    pub fn perform_key(&mut self, id: u16, value: f32) -> &mut Session {
        let t = self.t();
        let set = self.app.perform_set();
        let act = self
            .perform_keys
            .write(id, value, t, |page, pad| set.page(page).action(pad));
        let fx = crate::desk::perform_remote(
            &mut self.app,
            &self.time,
            &self.looper,
            &mut self.out,
            act,
            0,
        );
        self.fx.extend(fx);
        self.run_periods(2)
    }

    /// The song's Sync settings for MIDI in: other gear's MMC and what a
    /// program change does (the I/O view's Sync section).
    pub fn midi_in_setup(
        &mut self,
        mmc_in: bool,
        program_in: mpfree_midi::setup::ProgramIn,
    ) -> &mut Session {
        let io = std::sync::Arc::new(std::sync::Mutex::new(mpfree_midi_ui::IoModel::default()));
        {
            let mut m = io.lock().unwrap();
            m.setup.mmc_in = mmc_in;
            m.setup.program_in = program_in;
            m.gen += 1;
        }
        let t = self.t();
        self.music.sync(&io, t);
        self
    }

    /// Bytes from other gear on the 5-pin MIDI In, as the sound loop takes
    /// them: what MMC and program changes do (the Sync section's).
    pub fn midi_from_gear(&mut self, bytes: &[u8]) -> &mut Session {
        let t = self.t();
        let mut keys = Vec::new();
        self.music
            .bytes(crate::midi_in::DIN, bytes, t, &mut keys, &mut |_| {});
        let fx = crate::desk::remote_in(&mut self.music, &self.time, &mut self.app, &mut self.out);
        self.fx.extend(fx);
        self.run_periods(2)
    }

    /// The controller the Perform pads' learned keys came from went away.
    pub fn unplug_perform_keys(&mut self) -> &mut Session {
        for action in self.perform_keys.let_go() {
            let fx = crate::desk::perform_pad_up(&mut self.app, action);
            self.fx.extend(fx);
        }
        self.run_periods(2)
    }

    pub fn pad(&mut self, pad: usize, velocity: u8, hold: f64) -> &mut Session {
        self.pad_edge(pad, velocity, true);
        self.run_for(hold);
        self.pad_edge(pad, 0, false);
        self.run_periods(2)
    }

    /// A pad pressed or let go on the controller, as `player_session` hands
    /// it over (the step pads, then the mute keys and 16 Levels' Transpose
    /// first).
    pub fn pad_edge(&mut self, pad: usize, velocity: u8, down: bool) {
        if down && self.app.pads_are_steps() {
            let v = self.app.velocity(velocity);
            let rolling = self.time.info().state.rolling();
            let p = crate::step_press(
                &self.app,
                &mut self.step_keys,
                &mut self.time,
                pad,
                v,
                rolling,
            );
            if !p.log.is_empty() {
                self.out.note(&p.log);
            }
            if let Some(t) = p.tell {
                self.fx.push(Effect::Tell(t));
            }
            return;
        }
        if !down && self.step_keys.held(pad) {
            let r = self.step_keys.release(pad, &mut self.time);
            if !r.log.is_empty() {
                self.out.note(&r.log);
            }
            self.out.show(ToUi::PadUp(pad));
            return;
        }
        if down && pad < 16 && self.app.pads_are_loop_keys() {
            let action = self.app.perform_page().action(pad);
            self.keys_held.press_action(pad, self.t(), action);
            let (fx, line) = crate::desk::perform_pad(
                &mut self.app,
                &self.time,
                &self.looper,
                &mut self.out,
                action,
            );
            self.fx.extend(fx);
            if !line.is_empty() {
                self.out.note(&line);
            }
            return;
        }
        if let Some(action) = (!down)
            .then(|| self.keys_held.release_action(pad))
            .flatten()
        {
            // A held throw lets go with its pad.
            let fx = crate::desk::perform_pad_up(&mut self.app, action);
            self.fx.extend(fx);
            self.out.show(ToUi::PadUp(pad));
            return;
        }
        if down && pad < 16 && self.app.pads_mute() {
            self.mute_pressed[pad] = true;
            let (tap, more) = self.app.mute_tap();
            self.fx.extend(more);
            let why = crate::no_mutes(&self.track_kits, self.app.track(), self.app.pad_page());
            let line = crate::mute_key(tap, pad, why, &mut self.time, &mut self.mix);
            self.out.note(&line);
            return;
        }
        if down && pad < 16 && self.app.pads_transpose() {
            // 16 Levels' Transpose (sequencer S4): the pad moves the armed
            // track; its release plays nothing.
            self.mute_pressed[pad] = true;
            let own = &self.mix.instruments;
            let kits = &self.track_kits;
            let line = crate::transpose_pad(pad, &mut self.time, &self.synth, kits, own);
            self.out.note(&line);
            return;
        }
        // The synth chosen for the pads, as main.rs: it plays the hit,
        // before the arp and the recorder; the armed track keeps its kit.
        if down && pad < 16 && self.preview_on {
            let v = self.app.velocity(velocity);
            self.synth.note_on(pad, v, 0);
            self.preview_held[pad] = true;
            self.previewed.push((pad, v));
            self.out.show(ToUi::PadDown { pad, velocity: v });
            return;
        }
        if !down && pad < 16 && std::mem::take(&mut self.preview_held[pad]) {
            self.synth.note_off(pad, 0);
            self.out.show(ToUi::PadUp(pad));
            return;
        }
        if !down && pad < 16 && std::mem::take(&mut self.mute_pressed[pad]) {
            self.out.show(ToUi::PadUp(pad));
            return;
        }
        // Live erase: ERASE held while recording.
        if down && crate::live_erase_starts(&self.app, &self.time) {
            let tell = crate::live_erase(&mut self.time, self.app.kit_pad(pad));
            self.fx.extend(tell);
            return;
        }
        let kit_pad = self.app.kit_pad(pad) as u8;
        if !down && self.time.erases(kit_pad) {
            self.time.erase_sound(kit_pad, false);
            self.out.show(ToUi::PadUp(pad));
            return;
        }
        let take = self
            .app
            .rec
            .ready
            .map(|take| take == crate::rec_arm::Take::Add);
        if down {
            let exporting = self.export.running();
            let said = crate::pad_starts_loop(&mut self.looper, &mut self.time, take, exporting);
            self.out.loop_start(said);
        }
        // A Loop pad's hit waits for the launch line (sampler R5.3b), as
        // player_session: the sounding kit says which pads start on it.
        if let Some(p) = self.sounds.as_ref() {
            let w = time::PadWait {
                from: time::PadFrom::Pad(pad as u8),
                down,
                velocity,
                kit_pad: self.app.kit_pad(pad) as u8,
            };
            let (so, full) = (self.synth_on, &mut self.waits_full);
            if crate::waits_for_line(&mut self.time, p, &self.app, so, w, full, &mut self.out) {
                return;
            }
        }
        if let Some(ev) = self.app.pressed_event(pad, velocity, down) {
            self.time.pad_pressed(0, ev);
        }
        if down {
            let fx = self.app.pad_down(pad, velocity);
            self.fx.extend(fx);
        } else {
            let fx = self.app.pad_up(pad);
            self.fx.extend(fx);
            self.out.show(ToUi::PadUp(pad));
        }
    }

    // ------------------------------------------------------------ running

    /// Run for `seconds` of simulated time.
    pub fn run_for(&mut self, seconds: f64) -> &mut Session {
        let n = (seconds / PERIOD_S).ceil() as u64;
        self.run_periods(n)
    }

    /// Run `n` periods: what the screen sent, the app's effects, the time
    /// engine, and the screen taking what came back.
    /// The line input on, with a sine of `hz` on it at half scale (the
    /// session has no audio in), or off with None. The sampler's input
    /// takes it every period, with the bar lines, as main.rs feeds it.
    pub fn line_in(&mut self, hz: Option<f32>) -> &mut Session {
        self.line_tone = hz;
        self.app.settings.line_input = hz.is_some();
        self.out.show(ToUi::Settings(self.app.settings.clone()));
        self.settle()
    }

    pub fn run_periods(&mut self, n: u64) -> &mut Session {
        for _ in 0..n {
            self.period();
        }
        self
    }

    /// The screen's actions and the app's effects until both are done
    /// (no time passes).
    pub fn settle(&mut self) -> &mut Session {
        for _ in 0..16 {
            let acts = self.screen.take_actions();
            let busy = !acts.is_empty() || !self.fx.is_empty();
            for a in acts {
                self.route(a);
            }
            self.effects();
            self.tell_turns();
            self.feed_screen();
            if !busy {
                break;
            }
        }
        self
    }

    fn period(&mut self) {
        self.settle();
        // The list of instruments follows the tracks' own, as main.rs's
        // period does.
        self.instrument_list
            .follow(self.mix.instruments.held_tracks());
        let t = self.t();
        // A loop key held for a second clears its loop, as player_session.
        for pad in self.keys_held.holds(t) {
            if self.app.pads_are_loop_keys() {
                let action = self.app.perform_page().action(pad);
                let (fx, line) = crate::desk::perform_pad_held(action);
                self.fx.extend(fx);
                if !line.is_empty() {
                    self.out.note(&line);
                }
            }
        }
        // And a learned loop key (a footswitch), on any page.
        for n in self.remote.loop_holds(t, crate::perform_pads::CLEAR_HOLD_S) {
            let key = crate::perform_pads::PadAction::LoopKey(n as u8);
            let (fx, line) = crate::desk::perform_pad_held(key);
            self.fx.extend(fx);
            self.out.note(&format!("{line}, from a learned control"));
        }
        // And a key learned on a Perform pad that is a loop key.
        for key in self
            .perform_keys
            .holds(t, crate::perform_pads::CLEAR_HOLD_S)
        {
            let (fx, line) = crate::desk::perform_pad_held(key);
            self.fx.extend(fx);
            if !line.is_empty() {
                self.out.note(&format!("{line}, from a learned key"));
            }
        }
        if let Some(e) = self.app.tick(t) {
            self.fx.push(e);
        }
        if let Some(bpm) = self.loop_tempo.take() {
            self.fx.extend(self.app.set_tempo(bpm));
        }
        self.effects();
        self.change_kit();
        // The time engine, as player_session runs it.
        self.app.set_rolling(self.time.info().state.rolling());
        if self.app.set_perform(self.time.perform()) {
            self.out.show(ToUi::Rec(self.app.rec));
        }
        let taking = crate::desk::rec_now(&self.time, &self.looper).taking();
        if let Some(line) = self.time.take_follows(taking) {
            self.out.note(&format!("time: {line}"));
        }
        self.app.set_track_count(self.time.track_count());
        // Knob owners, as main.rs.
        let ranks = crate::track_ranks(self.time.project());
        self.mix.instruments.set_ranks(&ranks);
        self.synth.set_ranks(&ranks);
        // A track UNDO took away sets its kit aside, and REDO gives it back
        // (as player_session).
        let (redos, can_redo) = self.time.redos();
        let redone = redos != self.redos_seen;
        self.redos_seen = redos;
        if self
            .track_kits
            .follow(self.time.track_count(), redone, can_redo)
        {
            self.out
                .show(ToUi::TrackKits(Box::new(self.track_kits.clone())));
        }
        // What each track plays of the instruments, as main.rs tells the
        // screen (it does so a few times a second; here each period).
        let sounds: gui::TrackSounds = std::array::from_fn(|t| self.mix.instruments.held(t));
        if sounds != self.sounds_shown {
            self.sounds_shown = sounds;
            self.out.show(ToUi::TrackSounds(Box::new(sounds)));
        }
        let sends = crate::sends_now(&self.mix.instruments);
        if sends != self.sends_shown {
            self.sends_shown = sends;
            self.out.show(ToUi::TrackSends(Box::new(sends)));
        }
        let (loop_rec, loop_dub) = self.looper.recording();
        let info = self.time.info();
        let lights = self.app.transport_lights(
            app::TransportLights {
                rolling: info.state.rolling(),
                recording: loop_rec
                    || matches!(
                        info.state,
                        mpfree_transport::State::Recording | mpfree_transport::State::CountIn
                    ),
                overdub: loop_dub || info.overdub,
            },
            t,
        );
        self.fx.extend(lights);
        // Another device's clock, as main.rs gives it to the time engine.
        self.time.set_leader(self.music.leader_name());
        self.time.take_waits(
            self.app
                .rec
                .ready
                .map(|take| take == crate::rec_arm::Take::Add),
        );
        self.time.follow(self.follow);
        if !self.clock_bytes.is_empty() {
            let bytes = std::mem::take(&mut self.clock_bytes);
            let frame = self.time.next_frame();
            if let Some(line) = self.time.clock_in(frame, &bytes) {
                self.out.note(&line);
            }
        }
        self.time.follow_tempo(self.app.settings.bpm);
        self.time.advance(PERIOD as u32);
        if self.app.rec.ready.is_some()
            && self.time.info().state == mpfree_transport::State::Recording
        {
            self.app.rec.ready = None;
            self.out.show(ToUi::Rec(self.app.rec));
        }
        let rolled =
            crate::desk::pad_rolled(&mut self.app, &self.time, &self.looper, &mut self.out);
        self.fx.extend(rolled);
        // As main.rs: the pads follow the selected track.
        let selected = self.time.selected().unwrap_or(0);
        if selected != self.app.track() {
            let fx = self.app.set_track(selected);
            self.fx.extend(fx);
            // The pads play what the newly selected track plays.
            self.kit_to = Some(self.track_kits.entry(selected));
        }
        if let Some(Effect::Arp(v)) = self.app.arp_poll() {
            self.out.show(ToUi::Arp(Box::new(v)));
        }
        // The Loop pads' hits that waited play on their line (STOP drops
        // those still waiting), and the screen outlines those waiting.
        let mut due = time::DuePads::default();
        let dropped = self.time.due_pads(&mut due);
        crate::note_stop_dropped(dropped, &mut self.stop_dropped, &mut self.out);
        let waiting = self.time.hits_waiting(self.app.pad_page());
        if self.waiting_shown != waiting {
            self.waiting_shown = waiting;
            self.out.show(ToUi::PadsWaiting(waiting));
        }
        for &(w, offset) in due.as_slice() {
            let fx = crate::play_waited(
                &mut self.app,
                &mut self.time,
                &mut self.awaiting,
                w,
                offset,
                &mut self.out,
            );
            self.fx.extend(fx);
        }
        self.hits.base = t;
        self.hits.only = self.export.only();
        self.time
            .period_arps(PERIOD as u32, &mut self.hits, &mut self.app.arps);
        for (o, is) in std::mem::take(&mut self.hits.tapped) {
            self.export.tap.send(&o, is);
        }
        // The pads' hits into the sounding kit, when there is one.
        let (queued, released, locks) = (
            std::mem::take(&mut self.hits.queued),
            std::mem::take(&mut self.hits.released),
            std::mem::take(&mut self.hits.lock_queue),
        );
        if let Some(p) = self.sounds.as_mut() {
            // Locks first, so a note on a lock's frame starts with it.
            for (addr, value, offset) in locks {
                time::Pads::lock_at(p, addr, value, offset);
            }
            for (pad, velocity, offset) in queued {
                p.hit_at(pad, velocity, offset);
            }
            for (pad, offset) in released {
                p.release_at(pad, offset);
            }
        }
        for &(offset, addr, value) in self.time.locks() {
            self.mix.lock(offset, addr, value);
        }
        for &(offset, addr, value) in self.time.lanes() {
            self.mix.lane(offset, addr, value);
        }
        self.time.clear_locks();
        for h in self.time.hits() {
            self.out.show(ToUi::Hit(*h));
        }
        self.time.clear_hits();
        if self.time.take_steps_changed() {
            self.out.show(ToUi::Steps(Box::new(self.time.steps())));
        }
        if !self.app.copy_held() {
            self.step_keys.forget_copy();
        }
        if let Some(held) = self.step_keys.held_news() {
            self.out.show(ToUi::StepsHeld(held));
        }
        if let Some(line) = self.time.take_later_note() {
            self.out.note(&format!("time: {line}"));
        }
        // `|`, not `||`: both flags are taken.
        if self.time.take_changed() | self.tracks.take_changed() {
            self.out.show(ToUi::Time(Box::new(crate::time_state(
                &self.time,
                &self.tracks,
            ))));
        } else {
            self.out.show(ToUi::Transport(self.time.info()));
        }
        if let Some(c) = self.cloud.take() {
            self.out.show(ToUi::SampleGrains(c));
        }
        // The sampler's input: the line input's sine, when it is on.
        if let (Some(feed), Some(hz)) = (self.rec_feed.as_mut(), self.line_tone) {
            let step = hz / RATE as f32;
            let mut l = [0f32; PERIOD];
            for x in l.iter_mut() {
                *x = 0.5 * (self.line_phase * std::f32::consts::TAU).sin();
                self.line_phase = (self.line_phase + step).fract();
            }
            feed.period_gain(self.time.block(), &l, &l, 0, self.rec_gain);
        }
        self.rec_gain.0 = self.rec_gain.1;
        // The looper's period, as main.rs runs it (`crate::loop_input`):
        // what the pads play (no MIDI out here), from the last period's
        // mix, a take's source held to its end; and the loop out to the
        // mixer's Loop track. It reads the loops for a save and lets an
        // open's clears land.
        let (mut pl, mut pr) = ([0f32; PERIOD], [0f32; PERIOD]);
        let homes = self.track_kits.homes(|t| !self.mix.instruments.plays(t));
        let pads = crate::pads_instrument(&self.app, &self.mix, &self.track_kits, false);
        let said = crate::loop_input(
            &mut self.looper,
            &mut self.mix,
            pads,
            &homes,
            &mut pl,
            &mut pr,
        );
        for (line, tell) in said.into_iter().flatten() {
            self.out.note(&line);
            if tell {
                self.out.show(ToUi::Tell(crate::looper_says(&line)));
            }
        }
        let (mut ll, mut lr) = ([0f32; PERIOD], [0f32; PERIOD]);
        for line in self.looper.follow_plan(&self.time.loop_plan()) {
            self.out.note(&line);
        }
        self.looper
            .set_tempo_from_loop(crate::tempo_from_loop_allowed(
                self.app.settings.loop_tempo,
                &self.time,
                self.export.running(),
            ));
        if let Some(line) = self.looper.run(self.time.block(), &mut ll, &mut lr) {
            self.out.note(&line);
        }
        let take = self
            .app
            .rec
            .ready
            .map(|take| take == crate::rec_arm::Take::Add);
        let exporting = self.export.running();
        let said = crate::loop_starts_song(&mut self.looper, &mut self.time, take, exporting);
        self.out.loop_start(said);
        let (said, tempo) =
            crate::loop_sets_tempo(&mut self.looper, &mut self.time, take, exporting);
        self.out.loop_start(said);
        self.loop_tempo = tempo.or(self.loop_tempo);
        // The Looper tab and the Play page's strip follow the loop, as
        // main.rs shows them: its wave a slice at a time, its changes in
        // the log, and the loop's view about as often as the transport card
        // (every 11 periods, 32 ms).
        if let Some(wave) = self
            .looper
            .wave_step(self.periods, crate::LOOP_WAVE_PERIODS)
        {
            self.out.show(ToUi::LoopWave(wave));
        }
        for line in self.looper.changes(|| self.time.position_text()) {
            self.out.note(&line);
        }
        if let Some(now) = self.looper.grid_now() {
            for line in self.time.loops_are(&now) {
                self.out.note(&format!("time: {line}"));
            }
        }
        if let Some(end) = self.looper.copy_ended() {
            let fx = crate::desk::copy_ended(&mut self.time, end);
            self.fx.extend(fx);
        }
        if self.periods.is_multiple_of(11) {
            let view = self.looper.view(self.app.settings.bpm);
            self.out.show(ToUi::Looper(Box::new(view)));
        }
        self.save_period();
        // The mixer's period (silence in): edits to its chains land here,
        // as they do in player_session's.
        if let Some(line) = self.mix.poll_built() {
            self.out.note(line);
        }
        // A Perform FX the Perform set put on came in, or one came off:
        // said, and the Perform view's Q-Links follow, as in main.rs.
        if crate::desk::tell_throw_said(&mut self.mix, &mut self.out) {
            self.out
                .show(crate::desk::throw_knobs(&self.mix, self.app.perform_set()));
        }
        // A switch of mode that came in: Q-Links and MIDI controls learned
        // on the insert's controls follow them, as in main.rs.
        for landed in self.mix.modes.take_landed() {
            self.music.retarget(&landed);
            self.out.show(ToUi::ParamsMoved(Box::new(landed)));
        }
        let ctx = mixing::ctx(
            RATE,
            self.periods * PERIOD as u64,
            self.app.settings.bpm,
            self.time.block(),
        );
        let z = [0f32; PERIOD];
        let (mut l, mut r) = ([0f32; PERIOD], [0f32; PERIOD]);
        let (mut al, mut ar) = ([0f32; PERIOD], [0f32; PERIOD]);
        // An export's replay of one audio track: only it plays (export.rs).
        self.tracks.host.tracks.set_only(match self.export.only() {
            Some(mpfree_export::Only::AudioTrack(t)) => Some(usize::from(t)),
            _ => None,
        });
        self.tracks.period(
            self.time.block(),
            self.time.tempo_map(),
            self.time.song_on(),
            (&z, &z),
            (&mut al, &mut ar),
        );
        let mut played = 0;
        if let Some(p) = self.sounds.as_mut() {
            p.set_tempo(self.time.info().bpm);
            let mut pads = self.mix.pads();
            p.render_routed(PERIOD, &mut pads);
            played = pads.played();
        }
        self.mix.drums(played, &mut l, &mut r);
        self.mix.period(
            &ctx,
            (&z, &z),
            (&ll, &lr),
            (&al, &ar),
            (&z, &z),
            &mut l,
            &mut r,
        );
        // The kit's pads whose mute or solo came due, as main.rs lands them.
        crate::land_pads(&mut self.time, &mut self.mix);
        if let Some(h) = self.heard.as_mut() {
            h.extend(l.iter().zip(&r).flat_map(|(a, b)| [*a, *b]));
        }
        // A capture of an effect's memory that has ended, as in main.rs.
        if let Some((held, end)) = self.mix.take_captured() {
            if let Err(ToUi::Captured(held, end)) = self.out.try_show(ToUi::Captured(held, end)) {
                self.mix.keep_captured(held, end);
            }
        }
        // An export takes this period's capture points and notes, as in
        // main.rs; a fast one waits for its files' room in place of the
        // card's write.
        if self.export.running() {
            self.export.notes(self.time.block());
            if let Some(run) = self.export.period(&mut self.mix.mixer, &mut self.time) {
                self.out.show(ToUi::ExportOver(run));
            }
        }
        if self.export.fast() {
            self.export.wait_for_room(self.periods);
        }
        // Freeze and Print, as player_session runs them.
        let n_tracks = self.time.project().tracks.len();
        let time = &self.time;
        match self
            .mix
            .takes_period(&ctx, n_tracks, &|t| time.now_playing(t))
        {
            Some(crate::takes::Said::Note(l)) => {
                let l = format!("mixer: {l}");
                self.out.note(&l);
                self.out.show(ToUi::Tell(l));
            }
            Some(crate::takes::Said::Printed(name, p)) => self.out.show(ToUi::Printed(name, p)),
            None => {}
        }
        let now = self.now();
        if let Some(st) = self.mix.poll_state(now) {
            self.out.show(ToUi::Mixer(Box::new(st)));
        }
        let (view, bpm) = (self.app.view(), self.time.info().bpm);
        let Session {
            panels,
            mix,
            out,
            app,
            time,
            ..
        } = self;
        let notes = |t| crate::modpanels::NotesSource::of(app, time, t);
        panels.poll(view, mix, &notes, bpm, now, &mut |m| out.show(m));
        self.out.show(ToUi::PlayLevels(self.mix.play_levels()));
        self.periods += 1;
        self.feed_screen();
        self.screen.tick(PERIOD_S as f32);
        // The library here runs inline (no worker thread), so the browser
        // is never busy and the screen's tick doesn't step it: a save's
        // answer (ProjectSaved) waits for it, as the worker's would.
        self.screen.step_browser();
        self.settle();
    }

    /// A save, as `player_session` makes one: the Looper's loops are read
    /// first (as they play), then what every engine holds is copied for
    /// the screen, which writes the parts into the library.
    fn save_period(&mut self) {
        // An open going in: the mixer's swap and the loops, a few edits a
        // period, as player_session tends them.
        if self.mix.opening() {
            if let Some(line) = self.mix.tend_open() {
                self.out.note(&line);
            }
        }
        if self.looper.opening() {
            if let Some(line) = self.looper.tend_open() {
                self.out.note(&line);
            }
        }
        if let Some(line) = self.tracks.take_note() {
            self.out.note(&line);
        }
        if std::mem::take(&mut self.save_asked) {
            if self.mix.opening() || self.looper.opening() || self.tracks.opening() {
                self.out
                    .note("project: not saved, a project is still opening (try again in a moment)");
            } else if self.collect_now {
                self.out.note("project: already saving");
            } else {
                let line = self.looper.save();
                self.loops_saving = self.looper.saving().is_some();
                if !self.loops_saving {
                    self.out
                        .note(&format!("project: saving without the loops ({line})"));
                }
                self.audio_saving = self.tracks.host.save();
                if !self.audio_saving {
                    self.out.note(
                        "project: saving without the audio tracks (their worker did not start)",
                    );
                }
                self.collect_now = true;
            }
        }
        if self.loops_saving {
            self.loops_held = self.looper.take_saved();
            self.loops_saving = self.loops_held.is_none();
        }
        if self.audio_saving {
            self.audio_held = self.tracks.host.take_saved();
            self.audio_saving = self.audio_held.is_none();
        }
        if self.collect_now && !self.loops_saving && !self.audio_saving {
            self.collect_now = false;
            let (loops, audio) = (self.loops_held.take(), self.audio_held.take());
            self.collect(loops, audio);
        }
    }

    fn collect(
        &mut self,
        loops: Option<mpfree_looper::save::Saved>,
        audio: Option<mpfree_audiotracks::save::Saved>,
    ) {
        let c = crate::project::Collected {
            tempo: self.app.settings.bpm,
            swing: self.time.info().swing,
            clips: self.time.project().clone(),
            mixer: crate::project::mixer_text(
                &self.mix.mixer,
                self.mix.pad_mutes(),
                self.mix.pad_solos(),
            ),
            modu: self.mix.modu.save(),
            synth: crate::project::synth_text(&self.app.synth_settings(), &self.synth.sounds()),
            arp: self.app.arp_part(),
            perform: self.app.perform_set().text(),
            instruments: crate::project::instruments_text(&self.mix.instruments),
            track_kits: crate::project::track_kits_text(
                &self.track_kits,
                &self.mix.instruments,
                &self.instrument_list,
            ),
            instrument_list: self.instrument_list.text(),
            pads: self.app.pads_part(),
            loops,
            audio,
        };
        self.out.show(ToUi::ProjectParts(Box::new(c)));
    }

    /// An opened project (or New) swapped in between two periods, as
    /// `player_session` swaps it.
    fn open_project(&mut self, mut o: Box<crate::project::Opening>) {
        use mpfree_time_ui::TimeAction;
        use mpfree_transport::Command;
        self.fx
            .push(Effect::Time(TimeAction::Transport(Command::Stop)));
        self.fx.push(Effect::StopSounds);
        // A new or opened project arms the notes (rec_arm.rs).
        self.app.rec = crate::rec_arm::RecArm::default();
        self.out.show(ToUi::Rec(self.app.rec));
        // A save under way stops: the project changes under it.
        if self.collect_now {
            self.looper.cancel_save();
            self.tracks.host.cancel_save();
            if let Some(l) = self.loops_held.take() {
                self.looper.drop_saved(l);
            }
            if let Some(a) = self.audio_held.take() {
                self.tracks.host.drop_saved(a);
            }
            (self.loops_saving, self.audio_saving, self.collect_now) = (false, false, false);
            self.out
                .note("project: the save stopped, a project is opening");
        }
        if let Some(bpm) = o.tempo {
            let fx = self.app.set_tempo(bpm);
            self.fx.extend(fx);
        }
        if let Some(mut t) = o.time.take() {
            (t.metronome, t.click_gain) = (self.time.metronome, self.time.click_gain);
            std::mem::swap(&mut self.time, &mut *t);
            o.time = Some(t);
            self.out.show(ToUi::Time(Box::new(self.time.state())));
        }
        if let Some(sw) = o.swing {
            self.fx
                .push(Effect::Time(TimeAction::Transport(Command::SetSwing(sw))));
        }
        if let Some(m) = o.mix.take() {
            self.mix.open(m);
        }
        if let Some((options, sounds)) = o.synth.take() {
            for line in self.synth.set_sounds(&sounds) {
                self.out.note(&line);
            }
            let (notes, fx) = self.app.open_synth(&options);
            for line in notes {
                self.out.note(&format!("project: {line}"));
            }
            self.fx.extend(fx);
            self.out.show(ToUi::Synth(self.synth.state()));
            o.synth = Some((options, sounds));
        }
        // The sends, as main.rs sets them (the session builds no own
        // instruments, so only those to instruments already here play).
        self.mix.instruments.clear_sends();
        for line in crate::open_sends(&mut self.mix.instruments, &o.sends) {
            self.out.note(&line);
        }
        self.app
            .set_track_sends(&crate::sends_now(&self.mix.instruments));
        // What each track plays, as main.rs takes it.
        self.track_kits = crate::sampler::TrackKits::new(self.kits);
        for &(track, kit) in &o.track_kits {
            if kit.is_some_and(|k| k >= self.kits)
                || !self.track_kits.set(track, kit.unwrap_or(self.kits))
            {
                self.out
                    .note(&format!("project: track {} plays kit 1", track + 1));
            }
        }
        self.kit_to = Some(self.track_kits.entry(self.app.track()));
        std::mem::swap(&mut self.instrument_list, &mut o.instrument_list);
        let (notes, fx) = self.app.open_arp(o.arp.as_deref());
        for line in notes {
            self.out.note(&line);
        }
        self.fx.extend(fx);
        let (notes, fx) = self.app.open_perform(o.perform.as_deref());
        for line in notes {
            self.out.note(&line);
        }
        self.fx.extend(fx);
        self.app.forget_copies();
        let (notes, fx) = self.app.open_pads(o.pads.as_deref());
        for line in notes {
            self.out.note(&line);
        }
        self.fx.extend(fx);
        if o.clear_loops {
            let line = self.looper.clear_all();
            self.out.note(&line);
        }
        if o.clear_audio {
            // Recordings end; the tracks empty once they have.
            self.tracks.clear();
        }
        for line in o.notes.drain(..) {
            self.out.note(&line);
        }
        self.out.note(&format!("project: {} is in", o.what));
        self.out.show(ToUi::ProjectBack(o));
    }

    /// An action from the screen: the control side's through the desk, the
    /// rest as the audio loop takes it.
    pub fn route(&mut self, a: FromUi) {
        self.sent.push(super::short(&a));
        let t = self.t();
        let now = self.now();
        let mix_turn = crate::mixturns::before_ui(&a, &self.mix, self.app.view() == View::Mixer);
        if crate::step_turn(&a) {
            self.step_keys.turned();
        }
        crate::record_knob(&a, &self.app, &self.mix, &mut self.time);
        let mut fx = std::mem::take(&mut self.fx);
        let rest = {
            let (mut d, out) = self.desk();
            d.screen(a, t, out, &mut fx)
        };
        self.fx = fx;
        // As the audio loop does: a mixer turn, from its value before the
        // desk set it.
        if let Some(turn) = mix_turn {
            crate::mixturns::record(&mut self.mix_turns, turn, self.level_db, now);
        }
        let Some(a) = rest else { return };
        match a {
            FromUi::Pad { pad, down } if pad < 16 => {
                let v = self.app.settings.screen_velocity;
                self.pad_edge(pad, v, down);
            }
            FromUi::Pad { .. } => {}
            // A scale row's note: no sound is rendered here.
            FromUi::Audition { .. } => {}
            // The Pads tab's strip, as main.rs turns the page.
            FromUi::PadBank(b) if b < app::KIT_PAGES && !self.synth_on => {
                self.app.set_pad_page(b);
                self.out.note(&format!(
                    "pad bank {}: pads {}-{}",
                    app::bank_letter(b),
                    app::kit_pad_name(b * 16),
                    app::kit_pad_name(b * 16 + 15)
                ));
                self.out.show(ToUi::PadPage(b));
            }
            FromUi::PadBank(_) => {}
            FromUi::Kit(k) => {
                self.kit_to = Some(k.min(self.kits));
                self.kit_picked = true;
            }
            FromUi::Synth(mpfree_synth_ui::SynthAction::Set { id, value }) => {
                let from = self.synth.value(id);
                self.synth.set(id, value);
                let to = self.synth.value(id);
                self.synth_turns.turned(id, from, to, now);
            }
            // The Synth tab's preset picker, as main.rs.
            FromUi::Preset(ask) => {
                let track = self.app.track();
                if !self.mix.instruments.plays(track) && ask.changes_sound() {
                    self.synth_turns.clear();
                }
                let track = crate::instrument_track(&self.mix.instruments, track);
                self.presets
                    .act(ask, track, &mut self.synth, &mut self.mix, &mut self.out);
            }
            FromUi::Synth(mpfree_synth_ui::SynthAction::Preset(k)) => {
                self.synth_turns.clear();
                if let Some(name) = self.synth.load_preset(k).map(str::to_string) {
                    self.out.note(&format!(
                        "synth: preset {} of {}, {name}",
                        k + 1,
                        self.synth.preset_count()
                    ));
                    self.out.show(ToUi::SynthValues(self.synth.values()));
                }
            }
            // The kit's view goes back at once (the sampler would swap it in).
            FromUi::SampleView(swap) => self.out.show(ToUi::SampleViewBack(swap)),
            FromUi::Preview(p) => {
                if let Some(old) = self.preview.replace(p) {
                    self.out.show(ToUi::PreviewBack(old));
                }
            }
            FromUi::PreviewStop => {
                if let Some(p) = self.preview.as_mut() {
                    p.stop();
                }
            }
            FromUi::SampleInput(feed) => {
                let old = match feed {
                    Some(f) => self.rec_feed.replace(f),
                    None => self.rec_feed.take(),
                };
                if let Some(old) = old {
                    self.out.show(ToUi::SampleInputBack(old));
                }
            }
            FromUi::SampleSource(src) => {
                self.mix.set_rec_source(src);
            }
            FromUi::SampleGain(db) => self.rec_gain.1 = 10f32.powf(db / 20.0),
            FromUi::SampleGrains(c) => self.cloud = Some(c),
            // A tap on a track's sound line on Arrange, as main.rs: the
            // track is picked and the Synth tab's MENU opens on its
            // Instrument row, showing that track's instrument.
            FromUi::TrackInstrument(track) if track < self.time.project().tracks.len() => {
                if self.time.selected() != Some(track) {
                    let pick = mpfree_time_ui::TimeAction::Pick(track);
                    self.fx.push(Effect::Time(pick));
                }
                let e = self.app.open_track_instrument(track);
                self.fx.extend(e);
                let tab = crate::synth_tab(&self.synth, &self.mix, track, &self.presets);
                self.out.show(ToUi::Synth(tab));
            }
            FromUi::TrackInstrument(track) => {
                self.out.note(&format!("arrange: no track {}", track + 1))
            }
            // A MIDI file from BROWSE, as main.rs.
            FromUi::MidiFile(m) => {
                let (synth, kits) = (&*self.synth, &self.track_kits);
                let own = &self.mix.instruments;
                let r = crate::midi_file::put(&mut self.time, &m, |t, n| {
                    let synth_has = kits.plays_synth(t).then_some(synth);
                    crate::midi_file::pad_on_track(own, synth_has, t, n)
                });
                let (Ok(line) | Err(line)) = r;
                self.out.note(&line);
                self.out.show(ToUi::Tell(line));
            }
            FromUi::SaveProject => self.save_asked = true,
            FromUi::OpenProject(o) => self.open_project(o),
            FromUi::OpenLoops(l) => self.looper.open(*l),
            FromUi::OpenAudio(a) => self.tracks.open(*a),
            FromUi::ExportAsk => {
                // Nothing plays outside here, and every track plays the
                // pads: the drum kit's strip or the synth's.
                let pads = mpfree_fx::StripId::Track(if self.synth_on {
                    mixing::SYNTH
                } else {
                    mixing::DRUMS
                });
                let f = crate::export::Facts::gather(
                    &self.time,
                    &self.mix,
                    mpfree_export::Rig::default(),
                    &|_| false,
                    &|_| pads,
                    &crate::export::audio_heard(&self.tracks.host.tracks),
                );
                self.out.show(ToUi::ExportFacts(Box::new(f)));
            }
            FromUi::Export(run) => {
                match self.export.begin(run, &mut self.mix.mixer, &mut self.time) {
                    // With replays, the arps' random notes start the same
                    // in every pass, as in main.rs.
                    Ok(()) if self.export.replays() => self.app.arps.reseed(),
                    Ok(()) => {}
                    Err(run) => self.out.show(ToUi::ExportOver(run)),
                }
            }
            // The sampler's and the store's: nothing to check here.
            FromUi::SaveTouch(_)
            | FromUi::Slice { .. }
            | FromUi::SampleLiveChop { .. }
            | FromUi::SampleHit { .. }
            | FromUi::SampleTake { .. }
            | FromUi::SampleKey { .. }
            | FromUi::SamplePadLive { .. }
            | FromUi::SaveBrowserLists(_) => {}
            FromUi::UndoHistory(on) => {
                let (rows, now) = match on {
                    gui::UndoOn::Synth => self
                        .synth_turns
                        .rows("Start: the sound before these turns", |t| {
                            crate::synth_turn_row(&self.synth, t.key, t.to)
                        }),
                    gui::UndoOn::Mixer => self
                        .mix_turns
                        .rows("Start: the mix before these turns", |t| {
                            crate::mixturns::row(t)
                        }),
                };
                self.out.show(ToUi::UndoHistory { on, rows, now });
            }
            FromUi::UndoOn { on, redo } => {
                let line = match on {
                    gui::UndoOn::Synth => {
                        let turn = if redo {
                            self.synth_turns.redo()
                        } else {
                            self.synth_turns.undo()
                        };
                        turn.map(|t| {
                            self.synth.set(t.key, if redo { t.to } else { t.from });
                            self.out.show(ToUi::SynthValues(self.synth.values()));
                            crate::synth_turn_said(&self.synth, t.key, redo)
                        })
                    }
                    gui::UndoOn::Mixer => {
                        let turn = if redo {
                            self.mix_turns.redo()
                        } else {
                            self.mix_turns.undo()
                        };
                        turn.map(|t| {
                            let (line, level) = crate::mixturns::apply(&mut self.mix, &t, redo);
                            if let Some(db) = level {
                                self.level_db = db;
                                self.out.show(ToUi::Level { db });
                            }
                            line
                        })
                    }
                };
                let line = line.unwrap_or_else(|| {
                    format!(
                        "Nothing to {} on this page",
                        if redo { "redo" } else { "undo" }
                    )
                });
                self.fx.push(Effect::Tell(line));
            }
            other => panic!("the desk handed back {}", super::short(&other)),
        }
    }

    /// Whether the Synth tab's and the mixer's turns have something to
    /// undo, when that changes (as the audio loop tells the screen).
    fn tell_turns(&mut self) {
        let ready = (
            !self.synth_turns.done().is_empty(),
            !self.mix_turns.done().is_empty(),
        );
        if ready != self.turns_told {
            self.turns_told = ready;
            self.out.show(ToUi::TurnsReady {
                synth: ready.0,
                mixer: ready.1,
            });
        }
    }

    /// The app's effects: the control side's through the desk, the rest as
    /// the audio loop takes them.
    fn effects(&mut self) {
        // What REC leaves alone with the synth chosen for the pads, as
        // main.rs: kits picked (the session has no MIDI out), and where it
        // records the pads as they are.
        self.app.set_configured(self.track_kits.picked());
        self.app.set_synth_tracks(self.track_kits.on_synth());
        for _ in 0..8 {
            if self.fx.is_empty() {
                return;
            }
            let mut later = Vec::new();
            for e in std::mem::take(&mut self.fx) {
                let rest = {
                    let (mut d, out) = self.desk();
                    d.effect(e, out, &mut later)
                };
                let Some(e) = rest else { continue };
                match e {
                    Effect::Light(note, value) => {
                        self.lights[usize::from(note & 0x7F)] = value;
                        if crate::akai::app_lights(note) {
                            self.sent_lights[usize::from(note & 0x7F)] = value;
                        }
                    }
                    Effect::Octave(d) if self.synth_on => {
                        if self.synth.shift_octave(d) {
                            self.app.set_pad_layout(Some(self.synth.pad_layout()));
                            self.out.show(ToUi::KitChanged {
                                kit: self.kits,
                                view: self.synth.kit_view(),
                            });
                        }
                    }
                    Effect::Octave(d) => {
                        let now = self.kit_to.unwrap_or(self.kit) as i64;
                        self.kit_to =
                            Some((now + d as i64).rem_euclid(self.kits as i64 + 1) as usize);
                        self.kit_picked = true;
                    }
                    Effect::SynthPads(p) => {
                        if self.synth.set_settings(p) {
                            self.out.show(ToUi::Synth(self.synth.state()));
                        }
                        self.out.show(ToUi::KitChanged {
                            kit: self.kits,
                            view: self.synth.kit_view(),
                        });
                        if self.synth_on {
                            self.app.set_pad_layout(Some(self.synth.pad_layout()));
                        }
                    }
                    Effect::StopSounds => self.synth.stop(),
                    // What the pads play, as main.rs: the screen names it;
                    // the pads follow once the kit has (change_kit).
                    Effect::PadsFocus(f) => self.out.show(ToUi::PadsFocus(f)),
                    // REC with the synth chosen for the pads, as main.rs:
                    // the track plays the same synth from now on, as a pick
                    // of Synth.
                    Effect::SynthOnTrack(t) => {
                        if self.track_kits.set(t, self.track_kits.synth()) {
                            self.out
                                .show(ToUi::TrackKits(Box::new(self.track_kits.clone())));
                        }
                        if t == self.app.track() {
                            self.kit_to = Some(self.track_kits.synth());
                        }
                    }
                    // As main.rs: opening a page changes what no track plays.
                    Effect::Show(v) => {
                        if v == View::Synth
                            && !self.kit_to.map_or(self.synth_on, |k| k == self.kits)
                            && !self.mix.instruments.plays(self.app.track())
                        {
                            self.out
                                .show(ToUi::Tell(crate::synth_tab_hint(self.app.track())));
                        }
                        self.out.show(ToUi::Show(v));
                    }
                    Effect::ProgEdit => {
                        let synth_now = self.kit_to.map_or(self.synth_on, |k| k == self.kits);
                        if synth_now {
                            later.extend(self.app.open_tab(View::Synth));
                        } else {
                            later.extend(self.app.open_tab(View::Pads));
                        }
                    }
                    Effect::Bank { button, shift } if button < 4 => {
                        let now = self.kit_to.unwrap_or(self.kit);
                        if now == self.kits {
                            let target =
                                app::bank_target(self.synth.settings().bank % 8, button, shift);
                            later.extend(self.app.synth_bank(target));
                        } else {
                            // The kit's page, as main.rs turns it.
                            let page = app::bank_target(self.app.pad_page(), button, shift);
                            self.app.set_pad_page(page);
                            self.out.note(&format!(
                                "pad bank {}: pads {}-{}",
                                app::bank_letter(page),
                                app::kit_pad_name(page * 16),
                                app::kit_pad_name(page * 16 + 15)
                            ));
                            self.out.show(ToUi::PadPage(page));
                        }
                    }
                    Effect::Bank { .. } => {}
                    // The bank buttons' lights (TRACK MUTE's bank): the
                    // session keeps no bank lights of its own.
                    Effect::BankLights => {}
                    Effect::Audio(p) => self.out.note(&format!("audio buffer: {p} periods")),
                    Effect::Quit => self.quit = true,
                    Effect::SavePatterns(_) => {}
                    Effect::SaveProject => self.save_asked = true,
                    // 16 LEVEL, as main.rs: the synth's pads open Transpose;
                    // the Drums engine's have neither.
                    Effect::SixteenLevels
                        if (self.synth_on || self.kit_to == Some(self.kits))
                            && !self.synth.plays_notes() =>
                    {
                        self.out.note(crate::DRUMS_NO_LEVELS)
                    }
                    Effect::SixteenLevels => self.out.show(ToUi::SixteenLevels),
                    // As main.rs: an empty kit pad held + REC arms a pad
                    // take; with no sampler in the session, REC as alone.
                    Effect::PadRec(held) => later.push(crate::desk::pad_rec(
                        &held,
                        self.track_kits.kit_of(self.app.track()),
                        |k, p| self.sounds.as_ref().is_none_or(|s| s.has_sound(k, p)),
                    )),
                    // A track's own instrument is built beside the audio
                    // thread, which the session leaves out: the track keeps
                    // what it had, and the screen hears so.
                    // As main.rs: Kit takes the track's own instrument out
                    // (or ends its send), and the tracks that played it
                    // play their kits again.
                    Effect::TrackSound { track, id: None } => {
                        let senders = self.mix.instruments.senders(track);
                        if self.mix.instruments.take_out(track) {
                            self.out
                                .note(&format!("track {}: plays the kit", track + 1));
                            if let Some(s) = crate::sends_ended(senders, track) {
                                self.out.note(&s);
                                self.out.show(ToUi::Tell(s));
                            }
                        }
                        let held = self.mix.instruments.held(track);
                        self.app.set_track_sound(track, held);
                        self.app
                            .set_track_sends(&crate::sends_now(&self.mix.instruments));
                    }
                    Effect::TrackSound { track, .. } => {
                        self.out.note(&format!(
                            "track {}: own instruments are not built in the test session",
                            track + 1
                        ));
                        let held = self.mix.instruments.held(track);
                        self.app.set_track_sound(track, held);
                    }
                    // As main.rs (step 4).
                    Effect::TrackSend { track, to } => {
                        let (lines, tell) = crate::send_track(&mut self.mix.instruments, track, to);
                        for line in &lines {
                            self.out.note(line);
                        }
                        if let Some(s) = tell {
                            self.out.show(ToUi::Tell(s));
                        }
                        let held = self.mix.instruments.held(track);
                        self.app.set_track_sound(track, held);
                        self.app
                            .set_track_sends(&crate::sends_now(&self.mix.instruments));
                    }
                    // As main.rs (step 4c), the copy made at once, as the
                    // builder thread would.
                    Effect::TrackCopy { track, from } => {
                        let name = self
                            .mix
                            .instruments
                            .held(from)
                            .map_or("instrument", |h| h.name);
                        if let Some((id, (extras, values))) =
                            crate::sound_of_track(&self.mix.instruments, from)
                        {
                            let plugins = crate::plugins::Plugins::none();
                            let made = crate::instruments::make(
                                &plugins, id, &extras, &values, RATE, PERIOD,
                            )
                            .expect("a copy of an engine");
                            let engine = crate::synth::Engine::from_node_id(id)
                                .unwrap_or(crate::synth::Engine::Poly);
                            let pads = self.synth.pads_for(engine);
                            let tuning = self.synth.tuning_now();
                            if self.mix.instruments.put(track, made, pads, &tuning).is_ok() {
                                let s = crate::copied_note(track, from, name);
                                self.out.note(&s);
                                self.out.show(ToUi::Tell(s));
                            }
                        }
                        let held = self.mix.instruments.held(track);
                        self.app.set_track_sound(track, held);
                        self.app
                            .set_track_sends(&crate::sends_now(&self.mix.instruments));
                    }
                    other => panic!("the desk handed back {other:?}"),
                }
            }
            self.fx.extend(later);
            self.change_kit();
        }
    }

    /// A kit (or the synth) picked this period for the armed track, or
    /// the pads following the armed track, as main.rs takes it; then what
    /// the pads play (app/pads_focus.rs).
    fn change_kit(&mut self) {
        // As main.rs: a kit picked while the synth was chosen for the pads
        // makes them follow the track; so does the arp over a kit.
        if std::mem::take(&mut self.kit_picked) {
            let fx = self.app.kit_pick_follows_track();
            self.fx.extend(fx);
        }
        let fx = self.app.arp_ends_pads_synth();
        self.fx.extend(fx);
        let playing = if self.synth_on { self.kits } else { self.kit };
        if let Some(k) = self.kit_to.take().filter(|&k| k != playing) {
            if self.track_kits.set(self.app.track(), k) {
                self.out
                    .show(ToUi::TrackKits(Box::new(self.track_kits.clone())));
            }
            if k == self.kits {
                self.synth_on = true;
            } else {
                self.synth_on = false;
                self.kit = k;
            }
            self.app
                .set_pad_layout(self.synth_on.then(|| self.synth.pad_layout()));
            self.out.show(ToUi::Kit(k));
        }
        // As main.rs: the synth chosen for the pads plays on its own while
        // the track plays something else, through the track while it plays
        // the synth; a pad still held on it ends its note as the pads leave
        // it, and its let-go is taken quietly.
        let preview_now = self.app.pads_chose_synth() && !self.synth_on;
        if preview_now != self.preview_on {
            if !preview_now {
                for pad in (0..16).filter(|&p| self.preview_held[p]) {
                    self.synth.note_off(pad, 0);
                }
            }
            self.preview_on = preview_now;
        }
    }

    /// Everything the control side sent the screen, taken as the screen
    /// thread takes it.
    fn feed_screen(&mut self) {
        while let Ok(m) = self.to_screen.try_recv() {
            match &m {
                ToUi::Line(l) | ToUi::Midi(l) => self.lines.push(l.clone()),
                ToUi::Quit => self.quit = true,
                _ => {}
            }
            self.screen.apply(m);
        }
        assert_eq!(self.out.dropped, 0, "the screen's queue overflowed");
    }

    // ------------------------------------------------------------- checks

    /// The hint line at the foot of the screen (where a control says in
    /// place why it can't work now).
    pub fn hint(&self) -> String {
        self.screen
            .ui
            .get::<mpfree_ui::widgets::Label>("hint")
            .map_or(String::new(), |l| l.text.clone())
    }
}
