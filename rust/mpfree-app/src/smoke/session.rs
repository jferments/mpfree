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
/// The device's default level: 6 dB under a -12 dB top (`--gain-db`).
const MAX_DB: f32 = -12.0;

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
    fn hit_at(&mut self, pad: usize, velocity: u8, offset: u32) -> bool {
        self.on.push((pad, velocity));
        self.queued.push((pad, velocity, offset));
        true
    }
    fn release_at(&mut self, pad: usize, offset: u32) {
        self.released.push((pad, offset));
    }
    fn stop_all(&mut self) {}
    fn pressure_at(&mut self, _pad: usize, _value: u8, _offset: u32) {}
    // Every event goes on as usual; the notes are written down first.
    fn out(&mut self, o: &mpfree_play::Out) -> bool {
        use mpfree_play::Event;
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
    /// The notes the time engine played.
    pub hits: Hits,
    /// A kit that sounds, when a test asks for one ([`Session::make_sounds`]):
    /// the pads then play into the mixer, as main.rs's sampler plays them.
    pub sounds: Option<crate::sampler::Player>,
    /// The sound loop's side of an export (export.rs).
    pub export: crate::export::Host,
    /// Every period's master output, while a test asks for it.
    pub heard: Option<Vec<f32>>,
    /// Controls a walk pressed that did nothing (see `walk.rs`).
    pub dead: Vec<String>,
    /// The kit the pads play (the synth is the last).
    pub kit: usize,
    pub synth_on: bool,
    /// Messages for the screen that found it gone (none expected).
    pub quit: bool,
    kits: usize,
    out: Out,
    to_screen: Receiver<ToUi>,
    fx: Vec<Effect>,
    kit_to: Option<usize>,
    /// The boxes the screen lent the audio thread.
    preview: Option<Box<crate::preview::Preview>>,
    rec_feed: Option<Box<crate::sampler::RecFeed>>,
    cloud: Option<Box<crate::sampler::Cloud>>,
    /// Simulated time: periods run, and the clock the rate limits read.
    periods: u64,
    t0: Instant,
    /// The screen as last drawn (only what changed is drawn again).
    canvas: mpfree_ui::SoftCanvas,
    /// UNDO on the Synth tab and the mixer: their turns, as the audio loop
    /// keeps them, and whether each had something to undo when last told.
    synth_turns: undo::Turns<u16>,
    mix_turns: undo::Turns<(mpfree_fx::StripId, u16)>,
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
/// ones as measured, the rest made up (on the MPC the control mapper finds
/// them).
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
            lights: [0; 128],
            hits: Hits::default(),
            sounds: None,
            export: crate::export::Host::new(),
            heard: None,
            dead: Vec::new(),
            kit: 0,
            synth_on: false,
            quit: false,
            kits: n,
            out,
            to_screen,
            fx: Vec::new(),
            kit_to: None,
            preview: None,
            rec_feed: None,
            cloud: None,
            periods: 0,
            t0: Instant::now(),
            canvas: mpfree_ui::SoftCanvas::new(gui::SCREEN.0, gui::SCREEN.1),
            synth_turns: undo::Turns::default(),
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
            mixer: crate::project::mixer_text(&self.mix.mixer, self.mix.pad_mutes()),
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

    /// The page the screen shows ("Play", "Sample · Chop", ...).
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

    /// Press `t` as a finger would, and let the session run a few periods.
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
            .filter(|_| !matches!(page, View::Play | View::Arrange))
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

    /// Hit a pad on the controller (down, then up after `hold` seconds).
    pub fn pad(&mut self, pad: usize, velocity: u8, hold: f64) -> &mut Session {
        self.pad_edge(pad, velocity, true);
        self.run_for(hold);
        self.pad_edge(pad, 0, false);
        self.run_periods(2)
    }

    /// A pad pressed or let go on the controller, as `player_session` hands
    /// it over (the mute keys first).
    pub fn pad_edge(&mut self, pad: usize, velocity: u8, down: bool) {
        if self.app.mute_keys() != app::MuteKeys::Off {
            if down {
                let line = crate::mute_key(
                    self.app.mute_keys(),
                    self.app.mute_bank(),
                    pad,
                    &mut self.time,
                    &mut self.mix,
                );
                self.out.note(&line);
            } else {
                self.out.show(ToUi::PadUp(pad));
            }
            return;
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
        let t = self.t();
        if let Some(e) = self.app.tick(t) {
            self.fx.push(e);
        }
        self.effects();
        self.change_kit();
        // The time engine, as player_session runs it.
        self.app.set_rolling(self.time.info().state.rolling());
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
        self.time.follow_tempo(self.app.settings.bpm);
        self.time.advance(PERIOD as u32);
        let armed = self.time.armed().unwrap_or(0);
        if armed != self.app.track() {
            let fx = self.app.set_track(armed);
            self.fx.extend(fx);
        }
        if let Some(Effect::Arp(v)) = self.app.arp_poll() {
            self.out.show(ToUi::Arp(Box::new(v)));
        }
        self.hits.base = t;
        self.time
            .period_arps(PERIOD as u32, &mut self.hits, &mut self.app.arps);
        for (o, is) in std::mem::take(&mut self.hits.tapped) {
            self.export.tap.send(&o, is);
        }
        // The pads' hits into the sounding kit, when there is one.
        let (queued, released) = (
            std::mem::take(&mut self.hits.queued),
            std::mem::take(&mut self.hits.released),
        );
        if let Some(p) = self.sounds.as_mut() {
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
        if let Some(line) = self.time.take_song_note() {
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
        // The looper's period (silence in), as main.rs runs it: it reads
        // the loops for a save and lets an open's clears land.
        let z0 = [0f32; PERIOD];
        self.looper.take_pads(&z0, &z0);
        let (mut ll, mut lr) = ([0f32; PERIOD], [0f32; PERIOD]);
        if let Some(line) = self.looper.run(self.time.block(), &mut ll, &mut lr) {
            self.out.note(&line);
        }
        self.save_period();
        // The mixer's period (silence in): edits to its chains land here,
        // as they do in player_session's.
        if let Some(line) = self.mix.poll_built() {
            self.out.note(line);
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
            (&z, &z),
            (&al, &ar),
            (&z, &z),
            &mut l,
            &mut r,
        );
        if let Some(h) = self.heard.as_mut() {
            h.extend(l.iter().zip(&r).flat_map(|(a, b)| [*a, *b]));
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
            panels, mix, out, ..
        } = self;
        panels.poll(view, mix, bpm, now, &mut |m| out.show(m));
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
            mixer: crate::project::mixer_text(&self.mix.mixer, self.mix.pad_mutes()),
            modu: self.mix.modu.save(),
            synth: crate::project::synth_text(&self.app.synth_settings(), &self.synth.sounds()),
            arp: self.app.arp_part(),
            instruments: crate::project::instruments_text(&self.mix.instruments),
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
        let (notes, fx) = self.app.open_arp(o.arp.as_deref());
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
        let mix_turn = match &a {
            FromUi::Mixer(mpfree_mixer_ui::MixAction::Set { strip, id, value }) => {
                Some((*strip, *id, self.mix.value(*strip, *id), *value))
            }
            _ => None,
        };
        let mut fx = std::mem::take(&mut self.fx);
        let rest = {
            let (mut d, out) = self.desk();
            d.screen(a, t, out, &mut fx)
        };
        self.fx = fx;
        // As the audio loop does: a mixer turn, from its value before the
        // desk set it.
        if let Some((strip, id, from, value)) = mix_turn {
            let master = (mpfree_fx::StripId::Master, mpfree_fx::ids::mixer::VOLUME);
            let to = if (strip, id) == master {
                self.level_db
            } else {
                value
            };
            self.mix_turns.turned((strip, id), from, to, now);
        }
        let Some(a) = rest else { return };
        match a {
            FromUi::Pad { pad, down } if pad < 16 => {
                let v = self.app.settings.screen_velocity;
                self.pad_edge(pad, v, down);
            }
            FromUi::Pad { .. } => {}
            FromUi::Kit(k) => self.kit_to = Some(k.min(self.kits)),
            FromUi::Synth(mpfree_synth_ui::SynthAction::Set { id, value }) => {
                let from = self.synth.value(id);
                self.synth.set(id, value);
                let to = self.synth.value(id);
                self.synth_turns.turned(id, from, to, now);
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
            FromUi::SampleGrains(c) => self.cloud = Some(c),
            // A tap on a track's sound line on Arrange, as main.rs: the
            // track is armed and the Synth tab's MENU opens on its
            // Instrument row, showing that track's instrument.
            FromUi::TrackInstrument(track) if track < self.time.project().tracks.len() => {
                if self.time.armed() != Some(track) {
                    let arm = mpfree_time_ui::TimeAction::Arm(Some(track));
                    self.fx.push(Effect::Time(arm));
                }
                let e = self.app.open_track_instrument(track);
                self.fx.extend(e);
                let tab = crate::synth_tab(&self.synth, &self.mix, track, None);
                self.out.show(ToUi::Synth(tab));
            }
            FromUi::TrackInstrument(track) => {
                self.out.note(&format!("arrange: no track {}", track + 1))
            }
            FromUi::SaveProject => self.save_asked = true,
            FromUi::OpenProject(o) => self.open_project(o),
            FromUi::OpenLoops(l) => self.looper.open(*l),
            FromUi::OpenAudio(a) => self.tracks.open(*a),
            FromUi::ExportAsk => {
                let f = crate::export::Facts::gather(
                    &self.time,
                    &self.mix,
                    mpfree_export::Rig::default(),
                );
                self.out.show(ToUi::ExportFacts(Box::new(f)));
            }
            FromUi::Export(run) => {
                if let Err(run) = self.export.begin(run, &mut self.mix.mixer, &mut self.time) {
                    self.out.show(ToUi::ExportOver(run));
                }
            }
            // The sampler's and the store's: nothing to check here.
            FromUi::SaveTouch(_)
            | FromUi::Slice { .. }
            | FromUi::SampleLiveChop { .. }
            | FromUi::SampleHit { .. }
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
                            crate::mix_turn_row(t.key.0, t.key.1, t.to)
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
                            let (strip, id) = t.key;
                            let value = if redo { t.to } else { t.from };
                            let set = mpfree_mixer_ui::MixAction::Set { strip, id, value };
                            if let Some(db) = self.mix.action(&set) {
                                self.level_db = db;
                                self.out.show(ToUi::Level { db });
                            }
                            crate::mix_turn_said(strip, id, redo)
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
                    Effect::Light(note, value) => self.lights[usize::from(note & 0x7F)] = value,
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
                    Effect::Show(v) => {
                        if v == View::Synth && !self.synth_on {
                            self.kit_to = Some(self.kits);
                        } else if v.sampler() && self.synth_on {
                            self.kit_to = Some(self.kit);
                        }
                        self.out.show(ToUi::Show(v));
                    }
                    Effect::ProgEdit => {
                        let synth_now = self.kit_to.map_or(self.synth_on, |k| k == self.kits);
                        if synth_now {
                            later.extend(self.app.open_tab(View::Synth));
                        } else {
                            self.app.set_pads_browse(false);
                            self.out
                                .show(ToUi::PadsView(mpfree_sampler_ui::pads_tab::PAD_VIEW));
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
                            let group = now / 8 * 8;
                            let target = group + app::bank_target(now % 8, button, shift);
                            if target < self.kits {
                                self.kit_to = Some(target);
                            }
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
                    // 16 LEVEL, as main.rs: the synth's pads have none.
                    Effect::SixteenLevels if self.synth_on || self.kit_to == Some(self.kits) => {
                        self.out.note(
                            "16 LEVEL: 16 Levels plays a kit; pick a kit with the wheel first",
                        )
                    }
                    Effect::SixteenLevels => self.out.show(ToUi::SixteenLevels),
                    // A track's own instrument is built beside the audio
                    // thread, which the session leaves out: the track keeps
                    // what it had, and the screen hears so.
                    Effect::TrackSound { track, .. } => {
                        self.out.note(&format!(
                            "track {}: own instruments are not built in the test session",
                            track + 1
                        ));
                        let held = self.mix.instruments.held(track);
                        self.app.set_track_sound(track, held);
                    }
                    other => panic!("the desk handed back {other:?}"),
                }
            }
            self.fx.extend(later);
            self.change_kit();
        }
    }

    /// A kit (or the synth) picked this period, as player_session takes it.
    fn change_kit(&mut self) {
        let Some(k) = self.kit_to.take() else { return };
        let playing = if self.synth_on { self.kits } else { self.kit };
        if k == playing {
            return;
        }
        if k == self.kits {
            self.synth_on = true;
        } else {
            if self.synth_on {
                self.synth_on = false;
                self.synth.all_off();
            }
            self.kit = k;
        }
        self.app
            .set_pad_layout(self.synth_on.then(|| self.synth.pad_layout()));
        self.out.show(ToUi::Kit(k));
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
