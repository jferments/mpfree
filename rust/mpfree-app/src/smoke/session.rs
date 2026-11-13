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
use crate::{audiotracks, cores, desk, looping, midi_in, mixing, modpanels, synth, time, Out};
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
}

impl time::Pads for Hits {
    fn hit_at(&mut self, pad: usize, velocity: u8, _offset: u32) -> bool {
        self.on.push((pad, velocity));
        true
    }
    fn release_at(&mut self, _pad: usize, _offset: u32) {}
    fn stop_all(&mut self) {}
    fn pressure_at(&mut self, _pad: usize, _value: u8, _offset: u32) {}
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
                let line =
                    crate::mute_key(self.app.mute_keys(), pad, &mut self.time, &mut self.mix);
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
        self.time
            .period_arps(PERIOD as u32, &mut self.hits, &mut self.app.arps);
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
        self.mix.drums(0, &mut l, &mut r);
        self.mix.period(
            &ctx,
            (&z, &z),
            (&z, &z),
            (&al, &ar),
            (&z, &z),
            &mut l,
            &mut r,
        );
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
        self.settle();
    }

    /// An action from the screen: the control side's through the desk, the
    /// rest as the audio loop takes it.
    pub fn route(&mut self, a: FromUi) {
        self.sent.push(super::short(&a));
        let t = self.t();
        let mut fx = std::mem::take(&mut self.fx);
        let rest = {
            let (mut d, out) = self.desk();
            d.screen(a, t, out, &mut fx)
        };
        self.fx = fx;
        let Some(a) = rest else { return };
        match a {
            FromUi::Pad { pad, down } if pad < 16 => {
                let v = self.app.settings.screen_velocity;
                self.pad_edge(pad, v, down);
            }
            FromUi::Pad { .. } => {}
            FromUi::Kit(k) => self.kit_to = Some(k.min(self.kits)),
            FromUi::Synth(mpfree_synth_ui::SynthAction::Set { id, value }) => {
                self.synth.set(id, value)
            }
            FromUi::Synth(mpfree_synth_ui::SynthAction::Preset(k)) => {
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
            // The sampler's and the store's: nothing to check here.
            FromUi::SaveTouch(_)
            | FromUi::Slice { .. }
            | FromUi::SampleLiveChop { .. }
            | FromUi::SampleHit { .. }
            | FromUi::SampleKey { .. }
            | FromUi::SamplePadLive { .. }
            | FromUi::SaveBrowserLists(_) => {}
            other => panic!("the desk handed back {}", super::short(&other)),
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
                    Effect::Kit(d) => {
                        // From the synth, when it has the pads (as main.rs).
                        let playing = if self.synth_on { self.kits } else { self.kit };
                        let now = self.kit_to.unwrap_or(playing) as i64;
                        self.kit_to =
                            Some((now + d as i64).rem_euclid(self.kits as i64 + 1) as usize);
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
                    Effect::Audio(p) => self.out.note(&format!("audio buffer: {p} periods")),
                    Effect::Quit => self.quit = true,
                    Effect::SavePatterns(_) => {}
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
