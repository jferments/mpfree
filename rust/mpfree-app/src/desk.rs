//! The control desk: where the screen's actions and the app's effects go
//! when they are for the app, the mixer, the time engine, the looper or the
//! chain editor rather than for a sound engine.
//!
//! The audio loop (`player_session` in main.rs) hands each action from the
//! screen and each effect from the app to [`Desk::screen`] and
//! [`Desk::effect`] first, and handles only what they hand back (the pads,
//! the kits, the sampler and the synth, the store, the controller's lights).
//! The smoke tests' session (`smoke/session.rs`) drives the same two
//! functions with the same engines, so a workflow tested there goes the way
//! it goes on the MPC.

use crate::{app, audiotracks, gui, looping, midi_in, mixing, modpanels, time, Out};
use crate::{LEVEL_MIN_DB, LEVEL_STEP_DB};

/// The engines the control side reaches, borrowed for one action or effect.
pub struct Desk<'a> {
    pub app: &'a mut app::App,
    pub mix: &'a mut mixing::MixHost,
    pub time: &'a mut time::TimeHost,
    pub panels: &'a mut modpanels::Panels,
    pub looper: &'a mut looping::LoopHost,
    /// The audio tracks on the Song view's lanes.
    pub tracks: &'a mut audiotracks::TrackHost,
    pub music: &'a mut midi_in::MusicIn,
    /// The Level (the mixer's master fader), in dB, and its top.
    pub level_db: &'a mut f32,
    pub max_db: f32,
    /// The sound card the looper's line input opens.
    pub card: u32,
}

impl Desk<'_> {
    /// An action from the screen, `t` seconds into the session. The app's
    /// effects go on `fx`. Returns the action when it is for the audio loop
    /// itself (the pads, the kits, the sampler, the synth, the store).
    pub fn screen(
        &mut self,
        a: gui::FromUi,
        t: f64,
        out: &mut Out,
        fx: &mut Vec<app::Effect>,
    ) -> Option<gui::FromUi> {
        let Desk {
            app,
            mix,
            time,
            panels,
            looper,
            tracks,
            music,
            level_db,
            max_db,
            ..
        } = self;
        let max_db = *max_db;
        match a {
            gui::FromUi::LevelDb(db) => {
                **level_db = db.clamp(LEVEL_MIN_DB, max_db);
                mix.set_level(**level_db);
                out.show(gui::ToUi::Level { db: **level_db });
            }
            // A drag on a Q-Link card on the Play page: as the knob turned.
            gui::FromUi::QLink { knob, delta } => {
                if let Some(e) = app.qlink(knob, delta, None) {
                    fx.extend(e);
                } else if knob == 0 {
                    **level_db =
                        (**level_db + LEVEL_STEP_DB * delta as f32).clamp(LEVEL_MIN_DB, max_db);
                    mix.set_level(**level_db);
                    out.show(gui::ToUi::Level { db: **level_db });
                }
            }
            gui::FromUi::PadsLayout { in_key } => fx.extend(app.synth_pick_layout(in_key)),
            gui::FromUi::TapTempo => fx.extend(app.tap_on_screen(t)),
            // A strip's insert row or "+": the chain editor in the mixer's place.
            gui::FromUi::Mixer(mpfree_mixer_ui::MixAction::OpenInsert { strip, slot }) => {
                fx.extend(panels.open_insert(app, mix, strip, Some(slot)))
            }
            gui::FromUi::Mixer(mpfree_mixer_ui::MixAction::AddInsert { strip }) => {
                fx.extend(panels.open_insert(app, mix, strip, None))
            }
            // The Sample tab's FX: the chain editor on the pad's channel.
            gui::FromUi::SampleFx { pad } => fx.extend(panels.open_pad(app, pad, mixing::DRUMS)),
            gui::FromUi::Chain(a) => {
                let recording = time.info().state == mpfree_transport::State::Recording;
                fx.extend(panels.chain(app, mix, &a, recording, &mut |m| {
                    // MIDI Learn arms with the Q-Link learn: a control
                    // moved on a keyboard can take the knob too (M5).
                    if let gui::ToUi::ChainLearn(addr) = &m {
                        music.arm(*addr);
                    }
                    out.show(m)
                }))
            }
            gui::FromUi::ChainLearned => {
                panels.learned();
                music.disarm();
            }
            gui::FromUi::Macros(a) => {
                let recording = time.info().state == mpfree_transport::State::Recording;
                fx.extend(panels.macros(app, mix, &a, recording))
            }
            gui::FromUi::OpenMacros(bank) => fx.extend(panels.open_macros(app, mix, bank)),
            // The mixer panel. Its master fader is the Level, so the Play page follows it.
            gui::FromUi::Mixer(a) => {
                if let Some(db) = mix.action(&a) {
                    **level_db = db;
                    out.show(gui::ToUi::Level { db: **level_db });
                }
            }
            gui::FromUi::MixerIcon => fx.extend(app.toggle_mixer(false)),
            gui::FromUi::IoIcon => fx.extend(app.toggle_io(crate::io_shown(mix))),
            gui::FromUi::OpenIo => fx.extend(app.open_io()),
            // The Play page: filters, sends, faders, speakers, throws, XY.
            gui::FromUi::PlayFx(p) => mix.play(p),
            gui::FromUi::Steps(a) => {
                // ERASE + a lane's name: said in place of the hint.
                let erase = matches!(a, time::StepAction::ClearPad { .. });
                // The pattern's length: said in place of the hint too.
                let length = matches!(a, time::StepAction::Length { .. });
                match time.step_action(a) {
                    Ok(line) if length && !line.is_empty() => fx.push(app::Effect::Tell(line)),
                    Err(e) if length => fx.push(app::Effect::Tell(format!("steps: {e}"))),
                    Ok(line) if erase => fx.push(app::Effect::Tell(if line.is_empty() {
                        "steps: that lane has no steps to erase".to_string()
                    } else {
                        format!("{line} (UNDO brings them back)")
                    })),
                    Ok(line) if !line.is_empty() => out.note(&line),
                    Ok(_) => {}
                    Err(e) if erase => fx.push(app::Effect::Tell(format!("steps: {e}"))),
                    Err(e) => out.note(&format!("steps: {e}")),
                }
            }
            gui::FromUi::UndoReady(ready) => app.set_undo_ready(ready),
            gui::FromUi::Overlay(on) => app.set_overlay(on),
            gui::FromUi::Tab(k) => fx.extend(app.tab(k)),
            gui::FromUi::MenuSelect(k) => fx.extend(app.select(k)),
            gui::FromUi::MenuStep(k, d) => fx.extend(app.step_item(k, d)),
            gui::FromUi::GuideButton(k) => fx.extend(app.guide_button(k)),
            gui::FromUi::Arp(a) => fx.extend(app.arp_action(a)),
            gui::FromUi::Workspace(a) => fx.extend(app.workspace_action(*a)),
            gui::FromUi::ArpChip => fx.extend(app.toggle_panel()),
            gui::FromUi::PadLevelOff => fx.extend(app.pad_level_off()),
            gui::FromUi::Levels { on, open } => fx.extend(app.levels_shown(on, open)),
            gui::FromUi::Time(a) => {
                fx.extend(crate::time_action(out, time, tracks, looper, app, mix, a))
            }
            gui::FromUi::DebugPage(k) => fx.extend(app.debug_page(k)),
            gui::FromUi::MapRow(k) => fx.extend(app.map_row(k)),
            gui::FromUi::MapButton(k) => fx.extend(app.map_button(k)),
            gui::FromUi::Looper(k) => fx.extend(app.looper_tap(k)),
            gui::FromUi::SamplePhrase(clip) => match time.put_clip(*clip) {
                Ok(line) | Err(line) => out.note(&format!("time: {line}")),
            },
            gui::FromUi::LoopLayer { id, command } => {
                fx.push(app::Effect::Looper(looping::LoopAction::Layer(id, command)))
            }
            gui::FromUi::LoopPick(n) => {
                fx.push(app::Effect::Looper(looping::LoopAction::Select(n)))
            }
            gui::FromUi::LoopMute(n) => {
                fx.push(app::Effect::Looper(looping::LoopAction::ToggleMute(n)))
            }
            gui::FromUi::LooperSetting(item, d) => fx.extend(app.looper_setting(item, d)),
            gui::FromUi::LoopCommand(c) => {
                fx.push(app::Effect::Looper(looping::LoopAction::Command(c)))
            }
            gui::FromUi::LoopResample(n) => {
                fx.push(app::Effect::Looper(looping::LoopAction::Resample(n)))
            }
            gui::FromUi::LoopSteps(n) => {
                fx.push(app::Effect::Looper(looping::LoopAction::Steps(n)))
            }
            // The Pads tab's switch: the browser or the pad's settings.
            gui::FromUi::PadsView(v) => {
                app.set_pads_browse(v == mpfree_sampler_ui::pads_tab::BROWSE)
            }
            // The audio loop's own: named one by one, so a new action has
            // to be given a home here or there.
            a @ (gui::FromUi::Pad { .. }
            | gui::FromUi::Kit(_)
            | gui::FromUi::SaveTouch(_)
            | gui::FromUi::Synth(_)
            | gui::FromUi::Slice { .. }
            | gui::FromUi::SampleLiveChop { .. }
            | gui::FromUi::SampleHit { .. }
            | gui::FromUi::SampleKey { .. }
            | gui::FromUi::SamplePadLive { .. }
            | gui::FromUi::Preview(_)
            | gui::FromUi::PreviewStop
            | gui::FromUi::SaveBrowserLists(_)
            | gui::FromUi::SampleView(_)
            | gui::FromUi::SampleInput(_)
            | gui::FromUi::SampleSource(_)
            | gui::FromUi::SampleGrains(_)
            | gui::FromUi::SaveProject
            | gui::FromUi::OpenProject(_)
            | gui::FromUi::OpenLoops(_)
            | gui::FromUi::OpenAudio(_)
            // The synth's and the mixer's turn histories are the loop's.
            | gui::FromUi::UndoHistory(_)
            | gui::FromUi::UndoOn { .. }) => return Some(a),
        }
        None
    }

    /// An effect from the app. What it gives back for the next period (the
    /// time engine's and the looper's effects) goes on `later`. Returns the
    /// effect when it is for the audio loop itself (lights, kits, the synth,
    /// the sound card, pages that hand the pads over, the store).
    pub fn effect(
        &mut self,
        e: app::Effect,
        out: &mut Out,
        later: &mut Vec<app::Effect>,
    ) -> Option<app::Effect> {
        let Desk {
            app,
            mix,
            time,
            looper,
            tracks,
            card,
            ..
        } = self;
        match e {
            // - and + on the mixer step the I/O view's row; on a strips view
            // they step the kit, as elsewhere.
            app::Effect::MixerStep(d) if !crate::io_shown(mix) => return Some(app::Effect::Kit(d)),
            app::Effect::Mixer(v) => mix.show(v),
            // The wheel on the mixer: Q-Link 1 on a strips view, the
            // section's selection on the I/O view.
            app::Effect::MixerWheel(d) => out.show(if crate::io_shown(mix) {
                gui::ToUi::IoInput(mpfree_mixer_ui::IoInput::Wheel(d))
            } else {
                gui::ToUi::MixerQLink(0, d)
            }),
            app::Effect::MixerStep(d) => {
                out.show(gui::ToUi::IoInput(mpfree_mixer_ui::IoInput::Step(d)))
            }
            app::Effect::ChainTurn { knob, delta } => out.show(gui::ToUi::ChainQLink(knob, delta)),
            app::Effect::ChainWheel(d) => out.show(gui::ToUi::ChainWheel(d)),
            app::Effect::ChainPush => out.show(gui::ToUi::ChainPush),
            app::Effect::QLinkBank => out.show(gui::ToUi::QLinkBank),
            app::Effect::SynthPage(d) => out.show(gui::ToUi::SynthPage(d)),
            app::Effect::SampleWheel(d) => out.show(gui::ToUi::SampleWheel(d)),
            app::Effect::Rows(k) => out.show(gui::ToUi::Rows(k)),
            app::Effect::SamplePad(d) => out.show(gui::ToUi::SamplePad(d)),
            app::Effect::SampleUndo { redo } => out.show(gui::ToUi::SampleUndo(redo)),
            app::Effect::Note(s) => out.note(&s),
            app::Effect::Tell(s) => {
                out.note(&s);
                out.show(gui::ToUi::Tell(s));
            }
            app::Effect::Undo { redo } => out.show(gui::ToUi::Undo(redo)),
            app::Effect::UndoButton(down) => out.show(gui::ToUi::UndoButton(down)),
            app::Effect::PlayRec { overdub } => out.show(gui::ToUi::PlayRec(overdub)),
            app::Effect::Menu(m) => out.show(gui::ToUi::Menu(m)),
            app::Effect::Guide(g) => out.show(gui::ToUi::Guide(g)),
            app::Effect::Status(s) => out.show(gui::ToUi::Status(s)),
            app::Effect::Settings(s) => out.show(gui::ToUi::Settings(s)),
            app::Effect::ClearLogs => out.show(gui::ToUi::ClearLogs),
            app::Effect::ClearConsole => out.show(gui::ToUi::ClearConsole),
            app::Effect::TouchLab => out.show(gui::ToUi::TouchLab),
            app::Effect::CalibrateTouch => out.show(gui::ToUi::CalibrateTouch),
            app::Effect::TouchWheel(d) => out.show(gui::ToUi::TouchWheel(d)),
            app::Effect::TouchStep(d) => out.show(gui::ToUi::TouchStep(d)),
            app::Effect::Arp(v) => out.show(gui::ToUi::Arp(Box::new(v))),
            app::Effect::Patterns(list) => out.show(gui::ToUi::Patterns(list)),
            app::Effect::Workspace(w) => out.show(gui::ToUi::Workspace(Box::new(w))),
            app::Effect::Recorded(p, next) => out.show(gui::ToUi::Recorded(Box::new(p), next)),
            app::Effect::WorkspaceWheel(d) => out.show(gui::ToUi::WorkspaceWheel(d)),
            app::Effect::ClipsWheel { delta, scroll } => {
                out.show(gui::ToUi::ClipsWheel { delta, scroll })
            }
            app::Effect::Map(m) => out.show(gui::ToUi::Map(Box::new(m))),
            app::Effect::LevelsWheel(d) => out.show(gui::ToUi::LevelsWheel(d)),
            app::Effect::PlayView(v) => out.show(gui::ToUi::PlayView(v)),
            app::Effect::SampleEditor(e) => out.show(gui::ToUi::SampleEditor(e)),
            app::Effect::Browse(k) => out.show(gui::ToUi::Browse(k)),
            app::Effect::Fill(on) => time.set_fill(on, 0),
            app::Effect::Metronome { on, db } => {
                time.metronome = on;
                time.click_gain = crate::time::click_gain(f32::from(db));
            }
            // What it gives back (the tempo's status line and note) is
            // handled with the next period's effects.
            app::Effect::Time(a) => {
                later.extend(crate::time_action(out, time, tracks, looper, app, mix, a))
            }
            app::Effect::Looper(a) => {
                later.extend(crate::looper_action(out, looper, app, *card, a))
            }
            // The audio loop's own, named one by one as above.
            e @ (app::Effect::Light(..)
            | app::Effect::Kit(_)
            | app::Effect::Octave(_)
            | app::Effect::SynthPads(_)
            | app::Effect::StopSounds
            | app::Effect::Audio(_)
            | app::Effect::Quit
            | app::Effect::Show(_)
            | app::Effect::ProgEdit
            | app::Effect::SaveProject
            | app::Effect::TrackSound { .. }
            | app::Effect::SixteenLevels
            | app::Effect::Bank { .. }
            | app::Effect::SavePatterns(_)) => return Some(e),
        }
        None
    }
}

/// What a Q-Link turned on the controller (`ticks` steps of knob `k`)
/// means to the screen on `page`, for the pages whose screen decides what
/// each knob turns; None elsewhere (the app or the Level takes it).
/// player_session asks the app first on Play and Arrange (NOTE REPEAT held,
/// the arp panel), and only then hands the turn to their page.
pub fn qlink_to_screen(page: app::View, k: usize, ticks: i32) -> Option<gui::ToUi> {
    use app::View;
    if k >= 4 || ticks == 0 {
        return None;
    }
    Some(match page {
        View::Synth => gui::ToUi::SynthQLink(k, ticks),
        View::Mixer => gui::ToUi::MixerQLink(k, ticks),
        View::Looper => gui::ToUi::LooperQLink(k, ticks),
        View::Chain | View::Macros => gui::ToUi::ChainQLink(k, ticks),
        View::Sample | View::Pads => gui::ToUi::SampleQLink(k, ticks),
        View::Play | View::Arrange => gui::ToUi::PlayQLink(k, ticks),
        _ => return None,
    })
}
