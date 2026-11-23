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

/// A touch on a gear lane's mod rows on the Sequencer's Steps (LFO to
/// CC, layout spec 4m): the change made in the mod matrix, then the rows
/// on its control for the screen, and what the matrix moves of the
/// tracks' MIDI out after a change. Only a track's MIDI out control, and
/// only a row aimed at it, is touched.
fn lane_rows(mix: &mut mixing::MixHost, ask: gui::LaneRowsAsk, out: &mut Out) {
    use gui::LaneRowsAsk as Ask;
    use mpfree_mod::{Change, Row, Source};
    use mpfree_play::mpfree_param::Owner;
    let target = match ask {
        Ask::Show(t) | Ask::AddLfo(t) => t,
        Ask::Depth { target, .. } | Ask::Remove { target, .. } => target,
    };
    if !matches!(target.owner, Owner::Midi(_)) {
        return;
    }
    let aimed = |mix: &mixing::MixHost, row: usize| {
        u8::try_from(row)
            .ok()
            .filter(|_| mix.modu.rows_on(target).iter().any(|r| r.0 == row))
    };
    let changed = match ask {
        Ask::Show(_) => false,
        Ask::AddLfo(_) => {
            let row = Row {
                source: Source::Lfo(0),
                via: None,
                target,
                depth: modpanels::ROW_DEPTH,
            };
            let added = mix.modu.change(&mix.mixer, Change::AddRow(row)).is_ok();
            if !added {
                out.show(gui::ToUi::Tell(
                    "mod rows: the mod matrix is full (64 rows)".to_string(),
                ));
            }
            added
        }
        Ask::Depth { row, depth, .. } => aimed(mix, row).is_some_and(|row| {
            let depth = depth.clamp(-1.0, 1.0);
            mix.modu
                .change(&mix.mixer, Change::Depth { row, depth })
                .is_ok()
        }),
        Ask::Remove { row, .. } => aimed(mix, row)
            .is_some_and(|row| mix.modu.change(&mix.mixer, Change::RemoveRow(row)).is_ok()),
    };
    let rows = mix
        .modu
        .rows_on(target)
        .into_iter()
        .map(|(i, s, d)| (i, mpfree_chain_ui::state::source_name(s), d))
        .collect();
    let can_add = mix.modu.engine().rows().count() < mpfree_mod::MAX_ROWS;
    out.show(gui::ToUi::LaneRows {
        target,
        rows,
        can_add,
    });
    if changed {
        out.show(gui::ToUi::GearMoved(mix.modu.midi_moved()));
    }
}

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
            gui::FromUi::ShowState(open, showing) => app.set_show_state(open, showing),
            // The Show view's Click switch: as a learned Click control.
            gui::FromUi::ShowControl(id) => {
                if let Some(e) = app.show_press(id) {
                    fx.extend(e);
                }
            }
            // The Show view's MENU > Open the Set editor: BROWSE, on the
            // Sets place the screen has put up.
            gui::FromUi::OpenSetEditor => fx.extend(app.open_browser()),
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
            // Its "SC from ..." or "Nothing picked": the sidechain list too.
            gui::FromUi::Mixer(mpfree_mixer_ui::MixAction::OpenSidechain { strip, slot }) => {
                fx.extend(panels.open_sidechain(app, mix, strip, slot))
            }
            gui::FromUi::Mixer(mpfree_mixer_ui::MixAction::AddInsert { strip }) => {
                fx.extend(panels.open_insert(app, mix, strip, None))
            }
            // An effect picked on the Effects look: its knobs on the Q-Links.
            gui::FromUi::Mixer(mpfree_mixer_ui::MixAction::PickInsert { strip, slot }) => {
                panels.pick_insert(strip, slot)
            }
            gui::FromUi::Mixer(mpfree_mixer_ui::MixAction::MoveInsert { strip, from, to }) => {
                fx.extend(panels.move_insert(mix, strip, from, to))
            }
            gui::FromUi::Mixer(mpfree_mixer_ui::MixAction::Unpick) => panels.unpick(),
            // The EQ look: + EQ, and a band picked or dragged.
            gui::FromUi::Mixer(mpfree_mixer_ui::MixAction::AddEq { strip }) => {
                fx.extend(panels.add_eq(mix, strip))
            }
            gui::FromUi::Mixer(
                a @ (mpfree_mixer_ui::MixAction::FocusInsert { .. }
                | mpfree_mixer_ui::MixAction::SetInsert { .. }
                | mpfree_mixer_ui::MixAction::ReleaseInsert { .. }),
            ) => {
                let recording = time.info().state == mpfree_transport::State::Recording;
                panels.insert_param(mix, &a, recording)
            }
            gui::FromUi::PadEffects { kit, pad } => {
                fx.extend(panels.open_pad_mix(app, mix, kit, pad))
            }
            gui::FromUi::Chain(a) => {
                let recording = time.info().state == mpfree_transport::State::Recording;
                fx.extend(panels.chain(app, mix, time, &a, recording, &mut |m| {
                    // MIDI Learn arms with the Q-Link learn: a control
                    // moved on a keyboard can take the knob too (M5).
                    if let gui::ToUi::ChainLearn(addr) = &m {
                        music.arm(*addr);
                    }
                    out.show(m)
                }))
            }
            // The chain editor's Capture: the copy starts on the mixer and
            // its box comes back when it ends. One refused at once goes
            // straight back (a full channel, the screen stalled, would free
            // it here, between periods, as edits do).
            gui::FromUi::Capture { slot, held } => {
                if let Err((held, end)) = panels.capture(mix, slot, held) {
                    let _ = out.try_show(gui::ToUi::Captured(held, end));
                }
            }
            gui::FromUi::LaneRows(ask) => lane_rows(mix, ask, out),
            gui::FromUi::ChainLearned => {
                panels.learned();
                music.disarm();
            }
            // SHIFT + a touch on a value (layout spec 4d): MIDI Learn waits
            // for a control to take it, and the same value again cancels.
            // A knob on the Synth tab is the selected track's synth's, an
            // arp slider the armed track's arp's, and a pad's setting on
            // the Sample or Pads tab the armed track's kit's.
            gui::FromUi::LearnValue(v) => {
                use mpfree_play::mpfree_param::{Owner, ParamAddr};
                let addr = match v {
                    gui::LearnValue::Param(a) => a,
                    gui::LearnValue::Synth(id) => {
                        let t = u8::try_from(app.track()).unwrap_or(u8::MAX);
                        ParamAddr::new(Owner::Inst(t), id)
                    }
                    gui::LearnValue::Arp(id) => {
                        let t = u8::try_from(app.track()).unwrap_or(u8::MAX);
                        ParamAddr::new(Owner::Arp(t), id)
                    }
                    gui::LearnValue::Pad { pad, field } => {
                        let t = u8::try_from(app.track()).unwrap_or(u8::MAX);
                        ParamAddr::new(Owner::Pad(t, pad), field)
                    }
                };
                if music.armed() == Some(addr) && music.waits() {
                    music.disarm();
                } else {
                    music.arm_from(addr, midi_in::ArmedBy::Touch);
                }
            }
            gui::FromUi::Macros(a) => {
                let recording = time.info().state == mpfree_transport::State::Recording;
                fx.extend(panels.macros(app, mix, &a, recording))
            }
            gui::FromUi::OpenMacros(bank) => fx.extend(panels.open_macros(app, mix, bank)),
            // The strip menu's Freeze, Thaw, Print and Cancel (takes.rs).
            gui::FromUi::Mixer(
                a @ (mpfree_mixer_ui::MixAction::Freeze(_)
                | mpfree_mixer_ui::MixAction::Thaw(_)
                | mpfree_mixer_ui::MixAction::Print(_)
                | mpfree_mixer_ui::MixAction::CancelTake(_)),
            ) => {
                let tracks = time.project().tracks.len();
                let bpm = time.info().bpm;
                if let Some(line) = mix.take_action(&a, tracks, bpm, &|t| time.now_playing(t)) {
                    let line = format!("mixer: {line}");
                    out.note(&line);
                    out.show(gui::ToUi::Tell(line));
                }
            }
            // An audio track's strip's M and S are the track's mute and
            // solo (layout spec 3a), as TRACK MUTE's and the panel's are.
            gui::FromUi::Mixer(mpfree_mixer_ui::MixAction::Set {
                strip: mpfree_fx::StripId::Track(ch),
                id: id @ (mpfree_fx::ids::mixer::MUTE | mpfree_fx::ids::mixer::SOLO),
                value,
            }) if audio_track_on(time, mix, ch).is_some() =>
            {
                let t = audio_track_on(time, mix, ch).unwrap_or(0);
                let solo = id == mpfree_fx::ids::mixer::SOLO;
                match audio_strip_switch(time, tracks, mix, t, solo, value >= 0.5) {
                    Some(Ok(line)) => out.note(&line),
                    Some(Err(e)) => out.note(&format!("mixer: {e}")),
                    None => {}
                }
            }
            // The mixer panel. Its master fader is the Level, so the Play page follows it.
            gui::FromUi::Mixer(a) => {
                if let Some(db) = mix.action(&a) {
                    **level_db = db;
                    out.show(gui::ToUi::Level { db: **level_db });
                }
            }
            gui::FromUi::FolderIcon => fx.extend(app.toggle_browser()),
            gui::FromUi::OpenIo => fx.extend(app.open_io()),
            // The Play page: filters, sends, faders, speakers, throws, XY.
            gui::FromUi::PlayFx(p) => {
                // What the XY pad plays: Prefs keeps the pick.
                if let crate::mixing::PlayFx::XyEffect(e) = p {
                    let choices = crate::play_page::XY_CHOICES;
                    let name = choices.get(e.map_or(0, |k| k + 1)).unwrap_or(&"?");
                    out.note(&format!("XY pad: plays {name}"));
                    fx.extend(app.set_xy_effect(e));
                }
                mix.play_screen(p);
                let _ = tell_throw_said(mix, out);
            }
            // A lane's speaker: the pad muted on its track (step 10).
            gui::FromUi::PadMute { track, pad, mute } => {
                let line = time.pad_mute_now(track, pad, mute).unwrap_or_else(|e| e);
                out.note(&line);
            }
            gui::FromUi::Steps(a) => {
                // ERASE + a lane's name: said in place of the hint.
                let erase = matches!(a, time::StepAction::ClearPad { .. });
                // The pattern's length: said in place of the hint too.
                let length = matches!(
                    a,
                    time::StepAction::Length { .. } | time::StepAction::LengthBy { .. }
                );
                if !erase && !length {
                    // A tap or a turn: apart (--time-thread) it goes to the
                    // editing side's thread and is said when it answers.
                    let say = time::Say::Note {
                        ok: "",
                        err: "steps: ",
                    };
                    if let Some(r) = time.step_action_then(a, say.clone()) {
                        let _ = crate::time_said(out, say, r);
                    }
                    return None;
                }
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
            gui::FromUi::Recall(on) => app.set_recall(on),
            gui::FromUi::Outline(shown) => app.set_outline(shown),
            gui::FromUi::Held(on) => app.set_held(on),
            gui::FromUi::Tab(k) => fx.extend(app.tab(k)),
            gui::FromUi::MenuSelect(k) => fx.extend(app.select(k)),
            gui::FromUi::MenuStep(k, d) => fx.extend(app.step_item(k, d)),
            gui::FromUi::GuideButton(k) => fx.extend(app.guide_button(k)),
            gui::FromUi::HelpButton(k) => fx.extend(app.help_button(k)),
            gui::FromUi::Arp(a) => fx.extend(app.arp_action(a)),
            gui::FromUi::Workspace(a) => fx.extend(app.workspace_action(*a)),
            gui::FromUi::ArpChip => fx.extend(app.toggle_panel()),
            gui::FromUi::PadLevelOff => fx.extend(app.pad_level_off()),
            gui::FromUi::StepsOff => fx.extend(app.steps_off()),
            gui::FromUi::StepPage(page) => fx.extend(app.set_step_page(page)),
            gui::FromUi::Levels {
                on,
                open,
                transpose,
            } => fx.extend(app.levels_shown(on, open, transpose)),
            gui::FromUi::Time(a) => {
                use crate::rec_arm::Press;
                use mpfree_time_ui::TimeAction::Transport;
                use mpfree_transport::Command;
                let ready = app.rec.ready.is_some();
                match a {
                    // The bar's Rec is REC: it records what is armed.
                    Transport(Command::Record) => {
                        fx.push(app::Effect::Rec(Press::Rec));
                        return None;
                    }
                    // Its Play with the notes ready starts the take, and its
                    // Stop lets go, as the buttons do.
                    Transport(Command::Play) if ready => {
                        fx.push(app::Effect::Rec(Press::Play { from_start: false }));
                        return None;
                    }
                    Transport(Command::Stop) if ready => fx.push(app::Effect::Rec(Press::Stop)),
                    _ => {}
                }
                // An empty Loop cell tapped: the next loop lands there.
                if let mpfree_time_ui::TimeAction::LoopCell(Some(cell)) = a {
                    fx.extend(arm_loop_cell(app, time, tracks, looper, mix, out, cell));
                    return None;
                }
                // SHIFT + a loop's cell dropped on another Loop track: a
                // new loop, a copy of its audio (loop-tracks.md part 6).
                if let mpfree_time_ui::TimeAction::DropClip {
                    from,
                    to,
                    copy: true,
                } = a
                {
                    let loops = |t: usize| time.project().tracks.get(t).is_some_and(|t| t.is_loops());
                    if from.0 != to.0 && loops(from.0) && loops(to.0) {
                        fx.extend(copy_loop_cell(time, looper, out, from, to));
                        return None;
                    }
                }
                let picked = matches!(
                    a,
                    mpfree_time_ui::TimeAction::Arm(Some(_)) | mpfree_time_ui::TimeAction::Pick(_)
                );
                fx.extend(crate::time_action(out, time, tracks, looper, app, mix, a));
                // Picking a track arms its notes, as Bitwig's auto-arm
                // (not while the arm is pinned: `arm` leaves it).
                if picked {
                    fx.extend(arm(
                        app,
                        time,
                        looper,
                        out,
                        crate::rec_arm::Target::Notes,
                        crate::rec_arm::Mover::Track,
                    ));
                }
            }
            gui::FromUi::Arm(t) => {
                fx.extend(arm(app, time, looper, out, t, crate::rec_arm::Mover::Page))
            }
            gui::FromUi::ArmTap(t) => {
                fx.extend(arm(app, time, looper, out, t, crate::rec_arm::Mover::Tap))
            }
            // SHIFT + Arm: REC records both, or lets go of one of the two.
            // A track's row of the target list: that track alone, as
            // picking it does (refused while a take records).
            gui::FromUi::ArmTrack(t) => {
                let now = rec_now(time, looper);
                if now.taking() {
                    fx.extend(rec_effects(vec![app.rec.busy(&now)], time, looper));
                    return None;
                }
                // The musician's pick for REC; what the pads play stays
                // (the instrument redesign's rule 1, app/pads_focus.rs).
                if time.armed() != Some(t) {
                    fx.extend(crate::time_action(
                        out,
                        time,
                        tracks,
                        looper,
                        app,
                        mix,
                        mpfree_time_ui::TimeAction::Arm(Some(t)),
                    ));
                }
                fx.extend(arm(
                    app,
                    time,
                    looper,
                    out,
                    crate::rec_arm::Target::Notes,
                    crate::rec_arm::Mover::Tap,
                ));
            }
            // SHIFT + a track's row: REC records it too, or lets go of it.
            gui::FromUi::ArmTrackAdd(t) => {
                let now = rec_now(time, looper);
                app.rec.also = time.also();
                let name = time
                    .project()
                    .tracks
                    .get(t)
                    .map_or_else(|| format!("Track {}", t + 1), |tr| tr.name.clone());
                if time.project().tracks.get(t).is_none_or(|tr| tr.is_loops()) {
                    fx.push(app::Effect::Tell(format!(
                        "{name} plays loops: the Looper row records them"
                    )));
                    return None;
                }
                if time.project().tracks.get(t).is_some_and(|tr| tr.is_audio()) {
                    fx.push(app::Effect::Tell(format!(
                        "{name} is an audio track: its arm dot on the Song view arms it"
                    )));
                    return None;
                }
                let done = app.rec.add_track(t, &name, time.armed(), &now);
                out.show(gui::ToUi::Rec(app.rec));
                fx.extend(rec_effects(done, time, looper));
            }
            gui::FromUi::ArmAdd(t) => {
                let now = rec_now(time, looper);
                app.rec.also = time.also();
                let was = app.rec;
                let done = app.rec.add(t, &now);
                if app.rec != was && !app.rec.both {
                    out.note(&format!("rec: {} armed", app.rec.name()));
                }
                out.show(gui::ToUi::Rec(app.rec));
                fx.extend(rec_effects(done, time, looper));
            }
            // The Sample tab's recorder: the pad take records, or ended
            // (REC records what it did before).
            gui::FromUi::PadTake(n) => {
                if app.rec.pad_now(n) {
                    out.show(gui::ToUi::Rec(app.rec));
                }
            }
            // The Wave page's REC (the one Sample page's step 2a): a pad
            // take there that goes at once, or the one running ends; while
            // another take counts in or records, the row says why.
            gui::FromUi::RecHere { kit, pad } => {
                let now = rec_now(time, looper);
                match app.rec.rec_here(kit, pad, &now) {
                    Ok(done) => {
                        if let Some(p) = app.rec.pad.filter(|p| p.here) {
                            out.note(&format!(
                                "rec: {} of kit {} for one take, from the Wave page's REC",
                                p.name(),
                                p.kit + 1
                            ));
                        }
                        fx.extend(rec_effects(done, time, looper));
                        out.show(gui::ToUi::Rec(app.rec));
                    }
                    Err(why) => {
                        out.note(&format!("rec: the Wave page's REC refused: {why}"));
                        out.show(gui::ToUi::RecHereNo(why));
                    }
                }
            }
            // The Wave page's Keep card (one-page step 2c): Sample Recall's
            // rule, for its tap and for its hold.
            gui::FromUi::KeepLast => {
                if take_waits(app, time, looper) {
                    let s = KEEP_WAITS.to_string();
                    out.note(&s);
                    out.show(gui::ToUi::Tell(s));
                } else {
                    out.show(gui::ToUi::KeepLast);
                }
            }
            gui::FromUi::RecallOpen { pad } => {
                fx.push(app::Effect::Recall(app::RecallKey::Open { pad }))
            }
            gui::FromUi::RecPin(on) => {
                app.rec.pinned = on;
                out.note(if on {
                    "rec: the arm pinned: only a tap on Arm moves it"
                } else {
                    "rec: the arm unpinned"
                });
                out.show(gui::ToUi::Rec(app.rec));
            }
            gui::FromUi::DebugPage(k) => fx.extend(app.debug_page(k)),
            gui::FromUi::MapRow(k) => fx.extend(app.map_row(k)),
            gui::FromUi::MapButton(k) => fx.extend(app.map_button(k)),
            gui::FromUi::Looper(k) => {
                // The loop's own Record and Overdub (the strip's, the Looper
                // tab's) arm the loop for REC too, letting go of the notes'
                // ready: the tap is the loop's record.
                use crate::looper_page::{B_OVERDUB, B_RECORD};
                if matches!(k, B_RECORD | B_OVERDUB)
                    && !app.rec.armed(crate::rec_arm::Target::Loop)
                {
                    app.rec.ready = None;
                    fx.extend(arm(
                        app,
                        time,
                        looper,
                        out,
                        crate::rec_arm::Target::Loop,
                        crate::rec_arm::Mover::Tap,
                    ));
                }
                fx.extend(app.looper_tap(k));
            }
            gui::FromUi::SamplePhrase(clip) => match time.put_clip(*clip) {
                Ok(line) | Err(line) => out.note(&format!("time: {line}")),
            },
            // Put on Audio track (the one Sample page's step 6): where it
            // went, or why not, said in place of the hint.
            gui::FromUi::SampleToAudio(s) => {
                // No audio track with room: a new one at the end of the
                // track list (phase 1a), as + Track adds it.
                if tracks.bring_track(&time.info()).is_none() && tracks.free_slot().is_some() {
                    match time.act(mpfree_time_ui::view::TimeAction::AddAudioTrack) {
                        Ok(line) => out.note(&format!("time: {line}")),
                        Err(e) => out.note(&format!("time: {e}")),
                    }
                    tracks.sync(time.project(), time.silenced(), true);
                }
                let (Ok(line) | Err(line)) = tracks.bring(*s, &time.info());
                out.note(&format!("audio tracks: {line}"));
                out.show(gui::ToUi::Tell(line));
            }
            gui::FromUi::LoopLayer { id, command } => {
                fx.push(app::Effect::Looper(looping::LoopAction::Layer(id, command)))
            }
            gui::FromUi::LoopPick(n) => {
                fx.push(app::Effect::Looper(looping::LoopAction::Select(n)))
            }
            gui::FromUi::LoopCopy { from, id, to, keep } => fx.push(app::Effect::Looper(
                looping::LoopAction::CopyLayer { from, id, to, keep },
            )),
            gui::FromUi::LoopMute(n) => {
                fx.push(app::Effect::Looper(looping::LoopAction::ToggleMute(n)))
            }
            gui::FromUi::LoopSolo(n) => {
                fx.push(app::Effect::Looper(looping::LoopAction::ToggleSolo(n)))
            }
            gui::FromUi::LoopLevel(n, level) => {
                fx.push(app::Effect::Looper(looping::LoopAction::Level(n, level)))
            }
            gui::FromUi::LoopRecordLoops(on) => {
                fx.push(app::Effect::Looper(looping::LoopAction::RecordLoops(on)))
            }
            gui::FromUi::LoopBars(bars) => fx.extend(app.set_loop_bars(bars)),
            gui::FromUi::LoopKeys => fx.extend(app.toggle_loop_keys()),
            gui::FromUi::PerformOff => fx.extend(app.perform_off()),
            gui::FromUi::PerformOn => fx.extend(app.perform_on()),
            gui::FromUi::PerformShown(on) => {
                if on {
                    out.show(throw_strips(mix, time));
                    out.show(throw_knobs(mix, app.perform_set()));
                }
                fx.extend(app.show_perform(on))
            }
            // The Perform view's Q-Links: a throw's setting on its strip's
            // Perform FX, as the chain editor's knobs set theirs.
            gui::FromUi::ThrowKnob { addr, value } => {
                let recording = time.info().state == mpfree_transport::State::Recording;
                modpanels::set_param(mix, addr, value, recording);
            }
            gui::FromUi::ThrowKnobLift(addr) => mix.modu.release(crate::hand::Hand::Screen, addr),
            gui::FromUi::ThrowEffect { on, fx: throw } => {
                fx.extend(panels.open_throw_mix(app, mix, on, throw))
            }
            gui::FromUi::PerformPage(page) => fx.extend(app.perform_page_to(page)),
            gui::FromUi::PerformPad { page, pad, action } => {
                fx.extend(app.set_perform_pad(page, pad, action))
            }
            gui::FromUi::PerformName { page, name } => {
                fx.extend(app.set_perform_name(page, &name))
            }
            gui::FromUi::LoopCapture => {
                fx.push(app::Effect::Looper(looping::LoopAction::Capture))
            }
            gui::FromUi::LooperSetting(item, d) => fx.extend(app.looper_setting(item, d)),
            gui::FromUi::LoopCommand(c) => {
                fx.push(app::Effect::Looper(looping::LoopAction::Command(c)))
            }
            gui::FromUi::LoopListen(what) => {
                fx.push(app::Effect::Looper(looping::LoopAction::Listen(what)))
            }
            gui::FromUi::LoopMix(strip) => fx.extend(app.loop_mix(strip)),
            gui::FromUi::LoopCaptureSpan(end, bars) => fx.push(app::Effect::Looper(
                looping::LoopAction::CaptureSpan(end, bars),
            )),
            gui::FromUi::LoopMixesWanted => out.show(throw_strips(mix, time)),
            gui::FromUi::LoopSteps(n) => {
                fx.push(app::Effect::Looper(looping::LoopAction::Steps(n)))
            }
            gui::FromUi::LoopSnap(s) => fx.push(app::Effect::Looper(looping::LoopAction::Snap(s))),
            gui::FromUi::LoopSide(side, keep) => fx.push(app::Effect::Looper(
                looping::LoopAction::Side(side, keep),
            )),
            gui::FromUi::LoopAb(on) => fx.push(app::Effect::Looper(looping::LoopAction::Ab(on))),
            gui::FromUi::LoopAnswer(anyway) => fx.push(app::Effect::Looper(
                looping::LoopAction::Answer(anyway),
            )),
            gui::FromUi::LoopPointView(open) => fx.push(app::Effect::Looper(
                looping::LoopAction::PointView(open),
            )),
            // The Pads tab's Browse for: the browser, as BROWSE opens it.
            gui::FromUi::PadsBrowse => fx.extend(app.open_browser()),
            // The Synth tab's preset picker: the browser over the page, and
            // back to it once the picker closes (presets.rs).
            gui::FromUi::PresetPicker(true) => fx.extend(app.open_browser()),
            gui::FromUi::PresetPicker(false) => fx.extend(app.close_browser()),
            // The Synth tab's gear and the options' Back: what MENU does.
            gui::FromUi::OptionsButton => fx.extend(app.options_button()),
            // The Synth tab's list of synths, picking for the pads
            // (app/synth_pick.rs, step 6).
            gui::FromUi::SynthPickOpen => fx.extend(app.open_synth_pick()),
            gui::FromUi::SynthPickTap(i) => fx.extend(app.pick_pads_synth(i)),
            gui::FromUi::SynthPickClose(keep) => fx.extend(app.close_synth_pick(keep)),
            gui::FromUi::SynthPickScope(only) => fx.extend(app.synth_pick_scope(only)),
            gui::FromUi::SynthPickLayer(on) => fx.extend(app.synth_pick_layer(on)),
            // The Synth tab's button: the synth on the pads whichever track
            // is selected, or Follow track (app/pads_focus.rs).
            gui::FromUi::PlayOnPads => fx.extend(app.toggle_pads_synth()),
            // The "Pads: Poly" chip: Follow track.
            gui::FromUi::FollowTrack => fx.extend(app.pads_follow_track()),
            // The audio loop's own: named one by one, so a new action has
            // to be given a home here or there.
            a @ (gui::FromUi::Pad { .. }
            | gui::FromUi::Audition { .. }
            | gui::FromUi::PadBank(_)
            | gui::FromUi::Kit(_)
            | gui::FromUi::SaveTouch(_)
            | gui::FromUi::Synth(_)
            | gui::FromUi::Preset(_)
            | gui::FromUi::NewTrackSound { .. }
            | gui::FromUi::SkinValues
            | gui::FromUi::Slice { .. }
            | gui::FromUi::SampleLiveChop { .. }
            | gui::FromUi::SampleHit { .. }
            | gui::FromUi::SampleTake { .. }
            | gui::FromUi::SampleKey { .. }
            | gui::FromUi::SamplePadLive { .. }
            | gui::FromUi::Preview(_)
            | gui::FromUi::PreviewStop
            | gui::FromUi::SaveBrowserLists(_)
            | gui::FromUi::ScreenPicture(_)
            | gui::FromUi::PluginsChanged
            | gui::FromUi::SampleView(_)
            | gui::FromUi::SampleInput(_)
            | gui::FromUi::SampleSource(_)
            | gui::FromUi::SampleGain(_)
            | gui::FromUi::SampleGrains(_)
            | gui::FromUi::SaveProject
            | gui::FromUi::OpenProject(_)
            // The song's patches and the patch picked: MIDI in's.
            | gui::FromUi::Patches(_)
            | gui::FromUi::PickPatch(_)
            | gui::FromUi::SongEnd(_)
            | gui::FromUi::SongEndReset
            | gui::FromUi::OpenLoops(_)
            | gui::FromUi::OpenAudio(_)
            | gui::FromUi::AudioFiles(_)
            | gui::FromUi::AudioSavedInto(_)
            | gui::FromUi::TrackInstrument(_)
            // A MIDI file's notes go on the pads the instruments play them on.
            | gui::FromUi::MidiFile(_)
            // The synth's and the mixer's turn histories are the loop's.
            | gui::FromUi::UndoHistory(_)
            | gui::FromUi::UndoOn { .. }
            | gui::FromUi::ExportAsk
            | gui::FromUi::Export(_)
            // Controller mode's chip: the I/O model and MIDI out's.
            | gui::FromUi::ControllerOff
            // The Q-Links' values, for MIDI Follow's Send values back.
            | gui::FromUi::QLinkValues(_)) => return Some(a),
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
            panels,
            looper,
            tracks,
            card,
            ..
        } = self;
        match e {
            // - and + on the mixer step the I/O view's row; on a strips view
            // they go to the wheel's outline.
            app::Effect::MixerStep(d) if !crate::io_shown(mix) => {
                out.show(gui::ToUi::Nav(app::Nav::Step(d)))
            }
            app::Effect::Mixer(v) => mix.show(v),
            // The list of synths over the Synth tab (app/synth_pick.rs).
            app::Effect::SynthPick(v) => out.show(gui::ToUi::SynthPick(v.map(Box::new))),
            app::Effect::MixerLast => mix.show(Some(mix.last())),
            app::Effect::MixerFaders => out.show(gui::ToUi::MixerFaders),
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
            app::Effect::QLinkBankBack => out.show(gui::ToUi::QLinkBankBack),
            app::Effect::PadCopy { from, to } => out.show(gui::ToUi::PadCopy { from, to }),
            app::Effect::PadClear { pad } => out.show(gui::ToUi::PadClear(pad)),
            // SHIFT + TC: the armed track's Quantise while recording.
            app::Effect::RecordQuantiseToggle => {
                let line = time
                    .toggle_record_quantise()
                    .unwrap_or_else(|e| format!("SHIFT + TC: {e}"));
                out.note(&format!("time: {line}"));
                out.show(gui::ToUi::Tell(line));
            }
            app::Effect::PlayBack => {
                let line = time
                    .play_back()
                    .unwrap_or_else(|e| format!("SHIFT + PLAY: {e}"));
                out.note(&format!("time: {line}"));
                out.show(gui::ToUi::Tell(line));
            }
            app::Effect::CountInToggle => {
                let line = time
                    .toggle_count_in()
                    .unwrap_or_else(|e| format!("SHIFT + OVERDUB: {e}"));
                out.note(&format!("time: {line}"));
                out.show(gui::ToUi::Tell(line));
            }
            app::Effect::MacrosPanel => later.extend(panels.toggle_macros(app, mix)),
            app::Effect::PadPick { slot } => out.show(gui::ToUi::PadPick(usize::from(slot))),
            app::Effect::TypeTempo => out.show(gui::ToUi::TypeTempo),
            app::Effect::ControllerSettings => {
                out.show(gui::ToUi::OpenIoSection(mpfree_midi_ui::CONTROLLER))
            }
            app::Effect::ScreenPicture => out.show(gui::ToUi::ScreenPicture),
            app::Effect::XyFull => out.show(gui::ToUi::XyFull),
            app::Effect::Reset => out.show(gui::ToUi::Reset),
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
            // UNDO while a take records throws it away (sequencer
            // decision 1); otherwise it is the page's.
            app::Effect::Undo { redo } => {
                let now = rec_now(time, looper);
                match app.rec.throw(&now).filter(|_| !redo) {
                    Some(done) => {
                        later.extend(rec_effects(done, time, looper));
                        out.show(gui::ToUi::Rec(app.rec));
                    }
                    None => out.show(gui::ToUi::Undo(redo)),
                }
            }
            app::Effect::UndoButton(down) => out.show(gui::ToUi::UndoButton(down)),
            app::Effect::Rec(p) => {
                let now = rec_now(time, looper);
                // The synth chosen for the pads: the notes go where it
                // plays, on a track that gets it (app/pads_focus.rs); the
                // take starts next period, once the track is armed.
                if let Some(e) = rec_synth(app, time, tracks, looper, mix, out, p, &now) {
                    later.extend(e);
                    return None;
                }
                let done = app.rec.press(p, &now);
                later.extend(rec_effects(done, time, looper));
                out.show(gui::ToUi::Rec(app.rec));
            }
            // An empty kit pad held + REC (sampler R5.7): one take onto it.
            app::Effect::PadArm { kit, pad } => {
                let now = rec_now(time, looper);
                let done = app.rec.arm_pad(kit, pad, &now);
                if let Some(p) = app.rec.pad {
                    out.note(&format!(
                        "rec: {} of kit {} armed for one take",
                        p.name(),
                        kit + 1
                    ));
                }
                later.extend(rec_effects(done, time, looper));
                out.show(gui::ToUi::Rec(app.rec));
            }
            app::Effect::PadTake(d) => out.show(gui::ToUi::PadTake(d)),
            // Sample Recall opens only while no take is ready, counting in
            // or recording: REC mid-take punches out, and the overlay
            // mustn't take that (the sequencer thread, 18 Nov). A pad take
            // armed or recording holds REC for that take the same way.
            app::Effect::Recall(app::RecallKey::Open { pad }) => {
                if take_waits(app, time, looper) {
                    let s = RECALL_WAITS.to_string();
                    out.note(&s);
                    out.show(gui::ToUi::Tell(s));
                } else {
                    out.show(gui::ToUi::Recall(app::RecallKey::Open { pad }));
                }
            }
            app::Effect::Recall(k) => out.show(gui::ToUi::Recall(k)),
            app::Effect::Nav(n) => out.show(gui::ToUi::Nav(n)),
            app::Effect::Held { delta, fine } => out.show(gui::ToUi::Held { delta, fine }),
            app::Effect::Menu(m) => out.show(gui::ToUi::Menu(m)),
            app::Effect::Guide(g) => out.show(gui::ToUi::Guide(g)),
            app::Effect::Status(s) => out.show(gui::ToUi::Status(s)),
            app::Effect::Settings(s) => out.show(gui::ToUi::Settings(s)),
            app::Effect::ClearLogs => out.show(gui::ToUi::ClearLogs),
            app::Effect::ClearConsole => out.show(gui::ToUi::ClearConsole),
            app::Effect::TouchLab => out.show(gui::ToUi::TouchLab),
            app::Effect::PatchList => out.show(gui::ToUi::PatchList),
            app::Effect::SetMenu(m) => out.show(gui::ToUi::SetMenu(m)),
            app::Effect::ShowKey(k) => out.show(gui::ToUi::ShowKey(k)),
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
            app::Effect::EditTc => out.show(gui::ToUi::EditTc),
            app::Effect::Map(m) => out.show(gui::ToUi::Map(Box::new(m))),
            app::Effect::LevelsWheel(d) => out.show(gui::ToUi::LevelsWheel(d)),
            // ERASE + PAD BANK with STEP SEQ on: said in place of the hint.
            app::Effect::Steps(a) => {
                let line = match time.step_action(a) {
                    Ok(line) if line.is_empty() => {
                        "STEP SEQ: no steps on that page to clear".to_string()
                    }
                    Ok(line) => format!("{line} (UNDO brings them back)"),
                    Err(e) => format!("steps: {e}"),
                };
                out.note(&line);
                out.show(gui::ToUi::Tell(line));
            }
            app::Effect::PlayView(v) => out.show(gui::ToUi::PlayView(v)),
            app::Effect::ArrangeView(v) => out.show(gui::ToUi::ArrangeView(v)),
            app::Effect::SampleEditor(e) => out.show(gui::ToUi::SampleEditor(e)),
            app::Effect::EditPad => out.show(gui::ToUi::EditPad),
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
            // A Perform pad's throw, and its track mute.
            app::Effect::PlayFx(p) => {
                mix.play(p);
                let _ = tell_throw_said(mix, out);
            }
            // The set changed: the strips its throws play on get their
            // Perform FX (picking On is where the mix changes, never a hit).
            app::Effect::PerformSet { set, opened } => {
                mix.perform_strips(&set.throw_strips(), opened);
                let _ = tell_throw_said(mix, out);
                let knobs = throw_knobs(mix, &set);
                out.show(gui::ToUi::PerformSet { set, opened });
                out.show(throw_strips(mix, time));
                out.show(knobs);
            }
            app::Effect::TrackMute { track, wait } => {
                let say = time::Say::Note {
                    ok: "perform: ",
                    err: "perform: track mute: ",
                };
                if let Some(r) = time.mute_or_solo_then(track, wait, false, say.clone()) {
                    let _ = crate::time_said(out, say, r);
                }
            }
            // The audio loop's own, named one by one as above.
            e @ (app::Effect::Light(..)
            | app::Effect::Octave(_)
            | app::Effect::SynthPads(_)
            | app::Effect::StopSounds
            | app::Effect::Audio(_)
            | app::Effect::Quit
            | app::Effect::Show(_)
            | app::Effect::ProgEdit
            | app::Effect::SaveProject
            | app::Effect::TrackSound { .. }
            | app::Effect::TrackSend { .. }
            | app::Effect::TrackCopy { .. }
            | app::Effect::PadsFocus(_)
            | app::Effect::SynthOnTrack(_)
            | app::Effect::SixteenLevels
            | app::Effect::PadRec(_)
            | app::Effect::Bank { .. }
            | app::Effect::BankLights
            | app::Effect::SavePatterns(_)
            | app::Effect::SetupNextStart { .. }) => return Some(e),
        }
        None
    }
}

/// What the arm finds ([`crate::rec_arm::Now`]): the transport, the loops
/// and the armed track's name.
/// Whether a take is ready, counting in or recording (the notes, a loop,
/// or a pad take): Sample Recall waits for it (the sequencer thread, 18
/// Nov; REC arm rule 6), as the Wave page's Keep card does (one-page step
/// 2c), since REC mid-take punches out and neither may take that.
/// The audio track whose own mixer strip is channel `ch` (the audio
/// tracks rethink, phase 1b), if one is.
fn audio_track_on(time: &time::TimeHost, mix: &mixing::MixHost, ch: usize) -> Option<usize> {
    mixing::channel_track(ch).filter(|&t| {
        mix.track_channel(t) == ch && time.project().tracks.get(t).is_some_and(|tr| tr.is_audio())
    })
}

/// Audio track `t`'s strip's M (or S, `solo`) set `on` on the mixer panel:
/// the track's mute or solo, as TRACK MUTE's and the track panel's are
/// (layout spec 3a), so a solo there is project-wide. The engine track
/// follows at once and the panel reads again. What the time engine said,
/// or None when it was so already.
fn audio_strip_switch(
    time: &mut time::TimeHost,
    tracks: &mut crate::audiotracks::TrackHost,
    mix: &mut mixing::MixHost,
    t: usize,
    solo: bool,
    on: bool,
) -> Option<Result<String, String>> {
    let tr = time.project().tracks.get(t)?;
    let now = if solo { tr.solo } else { tr.mute };
    let said = (now != on).then(|| time.mute_or_solo(t, false, solo));
    tracks.sync(time.project(), time.silenced(), true);
    mix.state_now();
    said
}

pub fn take_waits(app: &app::App, time: &time::TimeHost, looper: &looping::LoopHost) -> bool {
    let now = rec_now(time, looper);
    now.taking() || now.loop_armed || app.rec.ready.is_some() || app.rec.pad.is_some()
}

/// What Sample Recall and the Keep card say while a take waits
/// ([`take_waits`]).
pub const RECALL_WAITS: &str = "Finish the take first: Sample Recall opens once it stops";
pub const KEEP_WAITS: &str = "Finish the take first: Keep keeps what just played once it stops";

/// The recording plan ([`app::RecPlan`], the redesign's step 5b): where
/// REC or OVERDUB on the notes would record the pads' synth (the shared
/// one, or from step 6 a synth picked in the list of synths) and, from step
/// 11c, each of their layers, then (step 11e) each synth a MIDI route
/// names, over the armed track as it is while the pads play its kit; or
/// None when they'd record the armed track as it is (the pads follow the
/// track and no route names a synth, the armed track plays the synth
/// itself and every layer and route synth has an armed track that plays
/// it, or REC records only the loop). REC uses it ([`rec_synth`]) and the REC chip
/// names it before you press (`main.rs` and the smoke session send it
/// when it changes), so the two can't disagree. Changes nothing, and
/// allocates nothing.
pub fn rec_plan(
    app: &app::App,
    time: &time::TimeHost,
    mix: &mixing::MixHost,
) -> Option<app::RecPlan> {
    use crate::rec_arm::Target;
    use app::SynthTake;
    let armed = time.armed().unwrap_or(0);
    // The pads' synth; or, while they play the armed track's kit (or its
    // MIDI out), that track as it is when a MIDI route names a synth to
    // record (step 11e): `synth` empty.
    let (holder, synth, kit) = if app.pads_chose_synth() {
        (None, app.pads_synth_name(), false)
    } else if let Some(h) = app.pads_holder() {
        (Some(h), app.pads_held()?.name, false)
    } else if app.routes_record() {
        (None, "", true)
    } else {
        return None;
    };
    let notes = app.rec.target == Target::Notes || app.rec.both;
    if !notes {
        return None;
    }
    let project = time.project();
    let app_side = |t: usize| track_set_up(app, mix, t);
    let ready = kit || app.track_plays_synth(armed, holder);
    let take = if ready {
        SynthTake::On(armed)
    } else {
        app::synth_track(project, armed, app.rec.pinned, app_side)
    };
    let mut plan = app::RecPlan {
        take,
        synth,
        holder,
        ready,
        layers: [None; app::REC_MORE],
    };
    let first = match take {
        SynthTake::On(t) => Some(t),
        SynthTake::New => None,
        // Nowhere for the first: nothing records, the layers neither.
        SynthTake::Pinned { .. } | SynthTake::Full => return Some(plan),
    };
    let bit = |t: usize| 1u128.checked_shl(t as u32).unwrap_or(0);
    let also = time.also();
    // The armed track can record a layer it plays when the first moves
    // the arm from it.
    let candidates = if first == Some(armed) {
        also
    } else {
        also | bit(armed)
    };
    plan.layers = app::layer_takes(
        app.rec_more_synths(),
        project.tracks.len(),
        first,
        candidates,
        |t, h| app.track_plays_synth(t, h),
        |t| app::unfit(project, t, app_side).is_none(),
    );
    // Every layer armed where it plays already: REC as it is.
    let set =
        |l: &app::LayerTake| matches!(l.take, SynthTake::On(t) if l.plays && also & bit(t) != 0);
    if ready && plan.layers.iter().flatten().all(set) {
        return None;
    }
    Some(plan)
}

/// Why track `t` can't take the pads' synth for REC, from what the app and
/// the mixer have ([`app::unfit`]'s `app_side`): an instrument of its own,
/// a send, a kit or MIDI output picked, the Drums track's channel, mixer
/// settings. None for one nobody set up.
fn track_set_up(app: &app::App, mix: &mixing::MixHost, t: usize) -> Option<&'static str> {
    let channel = mix.track_channel(t);
    if app.track_sound(t).is_some() {
        Some("has an instrument of its own")
    } else if app.track_send(t).is_some() {
        Some("plays another instrument")
    } else if app.configured(t) {
        Some("has a kit or a MIDI output picked")
    } else if channel < mixing::TRACKS {
        Some("is the Drums track")
    } else if !crate::project::untouched_channel(&mix.mixer, mpfree_fx::StripId::Track(channel)) {
        Some("has mixer settings")
    } else {
        None
    }
}

/// REC or OVERDUB on the notes while the synth is chosen for the pads and
/// the armed track plays something else: the take must play what was
/// heard and change nothing set up (the design's rule 3,
/// app/pads_focus.rs). A track nobody set up gets the synth and is armed
/// (the selected one, the first one, or a new one added at the end): the
/// shared synth as its Synth kit, or a synth picked in the list of synths
/// (step 6) by a send to it. Each of the pads' layers (step 11c) gets a
/// track the same way, armed with it as a track SHIFT + a tap in REC's
/// list arms (#639), so the takes share one number and one UNDO; a layer
/// with an armed track that plays it already keeps it. The press comes
/// again once they play them. None when the press is REC's own: no synth
/// chosen for the pads, the armed track playing it already and every
/// layer armed where it plays (a layer with no place is said then), the
/// loop armed (it records what is heard), or a take already going or
/// ready. `Some(empty)` when nowhere fits (a pinned arm on a track set
/// up, or a full project): the line says why and nothing changes. The arp
/// playing the pads now gives them back to the track first, and REC is
/// REC's own.
#[allow(clippy::too_many_arguments)]
fn rec_synth(
    app: &mut app::App,
    time: &mut time::TimeHost,
    tracks: &mut audiotracks::TrackHost,
    looper: &looping::LoopHost,
    mix: &mut mixing::MixHost,
    out: &mut Out,
    p: crate::rec_arm::Press,
    now: &crate::rec_arm::Now,
) -> Option<Vec<app::Effect>> {
    use crate::rec_arm::Press;
    use app::SynthTake;
    if !matches!(p, Press::Rec | Press::Overdub) || now.taking() || app.rec.ready.is_some() {
        return None;
    }
    let plan = rec_plan(app, time, mix)?;
    let armed = time.armed().unwrap_or(0);
    // The arp came on this period: the pads play the track again, through
    // it, before the take starts (`App::arp_ends_pads_synth`).
    let ended = app.arp_ends_pads_synth();
    if !ended.is_empty() {
        let mut fx = ended;
        fx.push(app::Effect::Rec(p));
        return Some(fx);
    }
    let name = plan.synth;
    let tell = |out: &mut Out, s: String| {
        out.note(&s);
        out.show(gui::ToUi::Tell(s));
    };
    let bit = |t: usize| 1u128.checked_shl(t as u32).unwrap_or(0);
    // The layers (step 11c): which want a track set up or armed, within
    // the tracks that can record at once; which can't record, and why.
    let first_on = match plan.take {
        SynthTake::On(t) => bit(t),
        _ => 0,
    };
    let also_was = time.also() & !first_on;
    let mut room = crate::time::MAX_ALSO.saturating_sub(also_was.count_ones() as usize);
    let mut work = [None; app::REC_MORE];
    let mut left_out = Vec::new();
    for (slot, l) in work.iter_mut().zip(plan.layers.iter().flatten()) {
        let arm = !matches!(l.take, SynthTake::On(t) if also_was & bit(t) != 0);
        match l.take {
            SynthTake::Full => left_out.push(format!(
                "{} isn't recorded: no free track, and the project holds no more",
                l.synth
            )),
            _ if l.plays && !arm => {}
            _ if arm && app.rec.pad.is_some() => left_out.push(format!(
                "{} isn't recorded: a pad take records alone",
                l.synth
            )),
            _ if arm && room == 0 => left_out.push(format!(
                "{} isn't recorded: {} tracks at most record at once, SHIFT + a tap in REC's list lets go of one",
                l.synth,
                crate::time::MAX_ALSO + 1
            )),
            _ => {
                room -= usize::from(arm);
                *slot = Some(*l);
            }
        }
    }
    if plan.ready && work.iter().all(Option::is_none) {
        // Nothing to set up: REC records what it can, and says what not.
        if !left_out.is_empty() {
            tell(out, left_out.join("; "));
        }
        return None;
    }
    let mut fx = Vec::new();
    // A new track, armed as the Song view's + adds one (UNDO takes it
    // away, after the take): `None` when none could be added.
    let mut add = |fx: &mut Vec<app::Effect>,
                   out: &mut Out,
                   time: &mut time::TimeHost,
                   app: &mut app::App,
                   mix: &mut mixing::MixHost|
     -> Option<usize> {
        let count = time.track_count();
        fx.extend(crate::time_action(
            out,
            time,
            tracks,
            looper,
            app,
            mix,
            mpfree_time_ui::TimeAction::AddTrack,
        ));
        (time.track_count() > count).then_some(count)
    };
    let (t, added) = match plan.take {
        SynthTake::On(t) => (t, false),
        SynthTake::New => match add(&mut fx, out, time, app, mix) {
            Some(t) => (t, true),
            None => {
                tell(
                    out,
                    format!("No free track to record {name} on, and no new one could be added"),
                );
                return Some(fx);
            }
        },
        SynthTake::Pinned { track, why } => {
            tell(
                out,
                format!(
                    "REC is pinned to Track {}, which {why}: unpin it in REC's list to record {name} on a free track, or tap Follow track",
                    track + 1
                ),
            );
            return Some(fx);
        }
        SynthTake::Full => {
            tell(
                out,
                format!(
                    "No free track to record {name} on, and the project holds no more: tap Follow track to record Track {}'s kit",
                    armed + 1
                ),
            );
            return Some(fx);
        }
    };
    // The layers' new tracks, added before anything is armed (adding a
    // track lets the other armed tracks go, and arms the new one).
    let mut layer_at = [None; app::REC_MORE];
    for (at, l) in layer_at.iter_mut().zip(work.iter()) {
        let Some(l) = l else { continue };
        *at = match l.take {
            SynthTake::On(lt) => Some((lt, false)),
            _ => match add(&mut fx, out, time, app, mix) {
                Some(lt) => Some((lt, true)),
                None => {
                    tell(
                        out,
                        format!(
                            "No free track to record {} on, and no new one could be added",
                            l.synth
                        ),
                    );
                    return Some(fx);
                }
            },
        };
    }
    if time.armed() != Some(t) {
        fx.extend(crate::time_action(
            out,
            time,
            tracks,
            looper,
            app,
            mix,
            mpfree_time_ui::TimeAction::Arm(Some(t)),
        ));
    }
    if !plan.ready {
        fx.extend(match plan.holder {
            None => app.rec_synth_on(t, added),
            Some(h) => app.rec_send_on(t, h, added),
        });
    }
    // Each layer's track plays its synth and is armed too, with the other
    // tracks armed before.
    app.rec.also = also_was;
    for (l, at) in work.iter().zip(layer_at) {
        let (Some(l), Some((lt, added))) = (l, at) else {
            continue;
        };
        if !l.plays {
            fx.extend(match l.holder {
                None => app.rec_synth_on(lt, added),
                Some(h) => app.rec_send_on(lt, h, added),
            });
        }
        if app.rec.also & bit(lt) == 0 {
            let track = time
                .project()
                .tracks
                .get(lt)
                .map_or_else(|| format!("Track {}", lt + 1), |k| k.name.clone());
            let done = app.rec.add_track(lt, &track, Some(t), now);
            let done = done
                .into_iter()
                .filter(|d| !matches!(d, crate::rec_arm::Do::Tracks(_)));
            fx.extend(rec_effects(done.collect(), time, looper));
        }
    }
    if time.also() != app.rec.also {
        fx.extend(crate::time_action(
            out,
            time,
            tracks,
            looper,
            app,
            mix,
            mpfree_time_ui::TimeAction::ArmAlso(app.rec.also),
        ));
    }
    out.show(gui::ToUi::Rec(app.rec));
    fx.push(app::Effect::Rec(p));
    Some(fx)
}

pub fn rec_now(time: &time::TimeHost, looper: &looping::LoopHost) -> crate::rec_arm::Now {
    let info = time.info();
    let (loop_taking, loop_armed) = looper.rec_now();
    let notes = time
        .armed()
        .and_then(|t| time.project().tracks.get(t))
        .map_or_else(|| "the notes".to_string(), |t| t.name.clone());
    crate::rec_arm::Now {
        state: info.state,
        overdub: info.overdub,
        loop_taking,
        loop_armed,
        notes,
        clock: time.waiting_for().map(str::to_string),
    }
}

/// REC with kit pads held ([`app::Effect::PadRec`]): the first of them
/// with no sound on the armed track's kit `kit` (None: it plays the
/// synth) arms a pad take; with none, REC as alone.
pub fn pad_rec(
    held: &[usize],
    kit: Option<usize>,
    has_sound: impl Fn(usize, usize) -> bool,
) -> app::Effect {
    match kit.and_then(|k| held.iter().find(|&&p| !has_sound(k, p)).map(|&p| (k, p))) {
        Some((kit, pad)) => app::Effect::PadArm { kit, pad },
        None => app::Effect::Rec(crate::rec_arm::Press::Rec),
    }
}

/// Every period, after the transport ran: a pad take waiting for the
/// music gets its go once the music plays ([`crate::rec_arm::RecArm::rolled`]).
pub fn pad_rolled(
    app: &mut app::App,
    time: &time::TimeHost,
    looper: &looping::LoopHost,
    out: &mut Out,
) -> Vec<app::Effect> {
    if app.rec.pad.is_none_or(|p| p.started) {
        return Vec::new();
    }
    let done = app.rec.rolled(&rec_now(time, looper));
    if done.is_empty() {
        return Vec::new();
    }
    out.show(gui::ToUi::Rec(app.rec));
    rec_effects(done, time, looper)
}

/// Arm `t` (a page that arms what it shows, a track picked), and tell the
/// screen.
fn arm(
    app: &mut app::App,
    time: &time::TimeHost,
    looper: &looping::LoopHost,
    out: &mut Out,
    t: crate::rec_arm::Target,
    by: crate::rec_arm::Mover,
) -> Vec<app::Effect> {
    let now = rec_now(time, looper);
    app.rec.also = time.also();
    let was = app.rec;
    let done = app.rec.move_to(t, by, &now);
    if app.rec != was {
        out.note(&format!("rec: {} armed", app.rec.name()));
    }
    out.show(gui::ToUi::Rec(app.rec));
    let mut fx = rec_effects(done, time, looper);
    // The notes armed alone: a Loop cell armed on the grid lets go.
    if !app.rec.armed(crate::rec_arm::Target::Loop) && time.loop_cell().is_some() {
        fx.push(app::Effect::Time(mpfree_time_ui::TimeAction::LoopCell(
            None,
        )));
    }
    fx
}

/// A Perform pad pressed (the pads as loop keys, SHIFT + SAMPLER on the
/// Looper tab): what a tap on the loop's ring on the Looper tab does. A
/// key picks its loop and takes its next step (a muted loop plays again;
/// otherwise Record: record from the next bar line, cancel, end the first
/// pass, overdub, play), arming the loop for REC as the ring does; a mute
/// pad mutes or unmutes its loop without picking it, and with SHIFT held
/// solos it or lets go. The line for the log.
#[cfg(test)]
pub fn perform_pad(
    app: &mut app::App,
    time: &time::TimeHost,
    looper: &looping::LoopHost,
    out: &mut Out,
    action: crate::perform_pads::PadAction,
) -> (Vec<app::Effect>, String) {
    perform_pad_on(app, time, looper, out, action, 0)
}

/// A Perform pad pressed, as [`perform_pad`], with the drum pads Drop
/// drums mutes (`drums`, [`crate::perform_pads::drum_pads`] of the kit
/// playing). A throw pad plays its throw (held, or latched until the next
/// hit); a track mute pad mutes or unmutes its track, at the launch line
/// in Perform; a scene pad launches its row.
/// The strips a Perform pad's throw can play on now, for the screen.
/// The audio tracks play into their own channels (phase 1b), named after
/// them; the old shared Audio strip is listed only while it holds an
/// older project's settings ([`mixing::MixHost::old_audio`]).
pub fn throw_strips(mix: &mixing::MixHost, time: &time::TimeHost) -> gui::ToUi {
    let mut strips = mixing::throw_strips(
        &mix.mixer,
        mix.own_channels_in_use(),
        &crate::export::track_names(time),
        &|s| mix.own_instrument_on(s),
    );
    if !mix.old_audio() {
        strips.retain(|(_, s)| *s != mpfree_fx::StripId::Track(mixing::AUDIO));
    }
    gui::ToUi::ThrowStrips(strips)
}

/// What the Perform set's throws on tracks and submixes said (a Perform
/// FX put on or taken off, or why a throw couldn't play), in the log and
/// on the screen. Whether there was anything (a Perform FX may have come
/// or gone: [`throw_knobs`] again).
pub fn tell_throw_said(mix: &mut mixing::MixHost, out: &mut Out) -> bool {
    let said = mix.take_throw_said();
    let any = !said.is_empty();
    for line in said {
        out.note(&line);
        out.show(gui::ToUi::Tell(line));
    }
    any
}

/// Each throw of `set` (one per effect and strip) as its strip's Perform
/// FX has it: what the Perform view's Q-Links show and turn.
pub fn throw_knobs(mix: &mixing::MixHost, set: &crate::perform_pads::PerformSet) -> gui::ToUi {
    use crate::perform_pads::PadAction;
    let mut knobs: Vec<mixing::ThrowKnobs> = Vec::new();
    for page in &set.pages {
        for a in &page.pads {
            let PadAction::Throw(t) = *a else {
                continue;
            };
            let Some(fx) = t.fx() else {
                continue;
            };
            if !knobs.iter().any(|k| k.on == t.on && k.fx == fx) {
                knobs.push(mix.throw_knobs(t.on, fx));
            }
        }
    }
    gui::ToUi::ThrowKnobs(knobs)
}

pub fn perform_pad_on(
    app: &mut app::App,
    time: &time::TimeHost,
    looper: &looping::LoopHost,
    out: &mut Out,
    action: crate::perform_pads::PadAction,
    drums: u16,
) -> (Vec<app::Effect>, String) {
    use crate::perform_pads::{Hold, PadAction};
    use looping::LoopAction;
    let mut fx = Vec::new();
    let line = match action {
        PadAction::None => return (fx, String::new()),
        PadAction::Throw(t) => match app.perform_throw(t, true) {
            Some(on) => {
                fx.extend(crate::perform_pads::pad_throw_fx(t, on, drums).map(app::Effect::PlayFx));
                match (t.hold, on) {
                    (Hold::WhileHeld, _) => format!("perform: {}", t.name()),
                    (Hold::Bar, _) => format!("perform: {} for a bar", t.name()),
                    (Hold::Latch, true) => format!("perform: {} latched", t.name()),
                    (Hold::Latch, false) => format!("perform: {} let go", t.name()),
                }
            }
            None => String::new(),
        },
        PadAction::Macro { card, value } => {
            app.perform_macro(card, true);
            fx.push(app::Effect::PlayFx(crate::perform_pads::macro_fx(
                card, value, true,
            )));
            format!("perform: Macro {} at {value} %", u16::from(card) + 1)
        }
        PadAction::Mute(t) => {
            fx.push(app::Effect::TrackMute {
                track: usize::from(t),
                wait: app.mute_waits(),
            });
            String::new()
        }
        PadAction::Scene(n) => {
            fx.push(app::Effect::Time(mpfree_time_ui::TimeAction::LaunchScene(
                usize::from(n),
            )));
            format!("perform: Scene {}", u16::from(n) + 1)
        }
        PadAction::LoopKey(n) => {
            let n = usize::from(n);
            fx.push(app::Effect::Looper(LoopAction::Select(n)));
            let s = looper.loop_status(n);
            if s.is_some_and(|s| s.phase == mpfree_looper::Phase::Playing && s.muted) {
                fx.push(app::Effect::Looper(LoopAction::ToggleMute(Some(n))));
                format!("loop keys: Loop {} plays again", n + 1)
            } else if s.is_some_and(|s| s.capturing.is_some()) {
                // Capturing: its key calls it off.
                fx.push(app::Effect::Looper(LoopAction::Capture));
                format!("loop keys: Loop {}'s key calls its Capture off", n + 1)
            } else {
                if !app.rec.armed(crate::rec_arm::Target::Loop) {
                    app.rec.ready = None;
                    fx.extend(arm(
                        app,
                        time,
                        looper,
                        out,
                        crate::rec_arm::Target::Loop,
                        crate::rec_arm::Mover::Tap,
                    ));
                }
                fx.extend(app.looper_tap(crate::looper_page::B_RECORD));
                format!("loop keys: Loop {}'s key", n + 1)
            }
        }
        PadAction::LoopMute(n) if app.shift_held() => {
            let n = usize::from(n);
            let s = looper.loop_status(n);
            if s.is_none_or(|s| s.phase == mpfree_looper::Phase::Empty) {
                format!("loop keys: Loop {} is empty, nothing to solo", n + 1)
            } else {
                fx.push(app::Effect::Looper(LoopAction::ToggleSolo(n)));
                format!(
                    "loop keys: Loop {} {}",
                    n + 1,
                    if s.is_some_and(|s| s.solo) {
                        "solo off"
                    } else {
                        "soloed"
                    }
                )
            }
        }
        PadAction::LoopMute(n) => {
            let n = usize::from(n);
            let s = looper.loop_status(n);
            if s.is_none_or(|s| s.phase == mpfree_looper::Phase::Empty) {
                format!("loop keys: Loop {} is empty, nothing to mute", n + 1)
            } else {
                fx.push(app::Effect::Looper(LoopAction::ToggleMute(Some(n))));
                format!(
                    "loop keys: Loop {} {}",
                    n + 1,
                    if s.is_some_and(|s| s.muted) {
                        "plays again"
                    } else {
                        "muted"
                    }
                )
            }
        }
    };
    (fx, line)
}

/// A Perform pad let go (`action`, what it pressed): a held throw lets
/// go (a bar's plays its bar out), a macro goes back to where it was.
/// Latched throws, keys, mutes and scenes do nothing on the lift.
pub fn perform_pad_up(
    app: &mut app::App,
    action: crate::perform_pads::PadAction,
) -> Vec<app::Effect> {
    use crate::perform_pads::PadAction;
    match action {
        PadAction::Throw(t) => app
            .perform_throw(t, false)
            .and_then(|on| crate::perform_pads::pad_throw_fx(t, on, 0))
            .map(app::Effect::PlayFx)
            .into_iter()
            .collect(),
        PadAction::Macro { card, value } => {
            app.perform_macro(card, false);
            vec![app::Effect::PlayFx(crate::perform_pads::macro_fx(
                card, value, false,
            ))]
        }
        _ => Vec::new(),
    }
}

/// A key learned on a Perform pad (Learn a key; `remote::PerformAct`), on
/// any page: pressed, the pad's action plays as a hit on it does
/// ([`perform_pad_on`], with the drum pads Drop drums mutes, `drums`), and
/// the log says which pad; let go, its let-go ([`perform_pad_up`]). A
/// loop key's hold clears its loop through [`perform_pad_held`].
pub fn perform_remote(
    app: &mut app::App,
    time: &time::TimeHost,
    looper: &looping::LoopHost,
    out: &mut Out,
    act: crate::remote::PerformAct,
    drums: u16,
) -> Vec<app::Effect> {
    use crate::perform_pads::PadAction;
    use crate::remote::PerformAct;
    match act {
        PerformAct::Nothing => Vec::new(),
        PerformAct::Down { page, pad, action } => {
            let label = crate::perform_tab::pad_label(usize::from(page), usize::from(pad));
            if action == PadAction::None {
                out.note(&format!(
                    "perform: Perform pads {label} has nothing on it, from a learned key"
                ));
                return Vec::new();
            }
            out.note(&format!(
                "perform: Perform pads {label}, from a learned key"
            ));
            let (fx, line) = perform_pad_on(app, time, looper, out, action, drums);
            if !line.is_empty() {
                out.note(&line);
            }
            fx
        }
        PerformAct::Up(action) => perform_pad_up(app, action),
    }
}

/// A learned control on the looper (its footswitch; `remote::LooperAct`),
/// on any page: a loop's key does what its pad does ([`perform_pad_on`]),
/// a mute mutes the loop or lets it play, Next loop picks the next one,
/// Capture is the Looper tab's, and Undo, Redo and Clear are the Looper
/// tab's buttons, on the picked loop. Its hold clears the loop through
/// [`perform_pad_held`].
pub fn looper_remote(
    app: &mut app::App,
    time: &time::TimeHost,
    looper: &looping::LoopHost,
    out: &mut Out,
    act: crate::remote::LooperAct,
) -> Vec<app::Effect> {
    use crate::looper_page::{B_CLEAR, B_REDO, B_UNDO};
    use crate::remote::LooperAct;
    use looping::LoopAction;
    use mpfree_play::mpfree_param::looper::Looper;
    match act {
        LooperAct::Nothing => Vec::new(),
        LooperAct::Key(n) => {
            let key = crate::perform_pads::PadAction::LoopKey(n as u8);
            let (fx, line) = perform_pad_on(app, time, looper, out, key, 0);
            out.note(&format!("{line}, from a learned control"));
            fx
        }
        LooperAct::Mute { n, on } => vec![app::Effect::Looper(LoopAction::Mute(n, on))],
        LooperAct::Press(Looper::NextLoop) => {
            let n = (looper.picked() + 1) % mpfree_looper::MAX_LOOPS;
            vec![app::Effect::Looper(LoopAction::Select(n))]
        }
        LooperAct::Press(Looper::Capture) => vec![app::Effect::Looper(LoopAction::Capture)],
        LooperAct::Press(Looper::Undo) => app.looper_tap(B_UNDO),
        LooperAct::Press(Looper::Redo) => app.looper_tap(B_REDO),
        LooperAct::Press(Looper::Clear) => app.looper_tap(B_CLEAR),
        // Keys and mutes come as Key and Mute.
        LooperAct::Press(Looper::Key(_) | Looper::Mute(_)) => Vec::new(),
    }
}

/// What learned controls did to Play this pass (the Learn section's Play
/// card; `remote::PlayMoves`), on any page: its Swing and Drums filter
/// as its Q-Links turn them (the project's, so the work is unsaved), and
/// its XY pad as a finger moves it, the pad on the screen following.
/// Nothing when nothing moved.
pub fn play_remote(
    time: &time::TimeHost,
    mix: &mut mixing::MixHost,
    out: &mut Out,
    moves: &mut crate::remote::PlayMoves,
) -> Vec<app::Effect> {
    use mpfree_transport::{time::Swing, Command};
    let mut fx = Vec::new();
    if *moves == crate::remote::PlayMoves::default() {
        return fx;
    }
    let swing = time.info().swing;
    let acts = moves.take(play_now(time, mix));
    if let Some(amount) = acts.swing {
        fx.push(app::Effect::Time(mpfree_time_ui::TimeAction::Transport(
            Command::SetSwing(Swing { amount, ..swing }),
        )));
    }
    if let Some(position) = acts.filter {
        mix.play(mixing::PlayFx::Filter {
            strip: mpfree_fx::StripId::Track(mixing::DRUMS),
            position,
        });
    }
    if acts.swing.is_some() || acts.filter.is_some() {
        out.show(gui::ToUi::Edited);
    }
    if let Some((x, y, down)) = acts.xy {
        mix.play(mixing::PlayFx::Xy { x, y, down });
        out.show(gui::ToUi::PlayXy { x, y, down });
    }
    fx
}

/// What Play's learned controls read (`remote::Now`): the swing, the
/// Drums filter and where the XY pad stands.
pub fn play_now(time: &time::TimeHost, mix: &mixing::MixHost) -> crate::remote::Now {
    crate::remote::Now {
        swing: time.info().swing.amount,
        drums_filter: mix.play_levels().drums_filter,
        xy: mix.xy_at(),
        ..crate::remote::Now::default()
    }
}

/// What the looper's learned controls read (`remote::Now`): the loop
/// picked and the loops muted.
pub fn looper_now(looper: &looping::LoopHost) -> crate::remote::Now {
    crate::remote::Now {
        picked: looper.picked(),
        loops_muted: looper.mute_bits(),
        ..crate::remote::Now::default()
    }
}

/// A loop key held for [`crate::perform_pads::CLEAR_HOLD_S`]: its loop
/// cleared, as a ring held on the Looper tab clears it.
pub fn perform_pad_held(action: crate::perform_pads::PadAction) -> (Vec<app::Effect>, String) {
    use crate::perform_pads::PadAction;
    use looping::LoopAction;
    match action {
        PadAction::LoopKey(n) => (
            vec![
                app::Effect::Looper(LoopAction::Select(usize::from(n))),
                app::Effect::Looper(LoopAction::Command(mpfree_looper::Command::Clear)),
            ],
            format!("loop keys: Loop {} cleared (its key held)", n + 1),
        ),
        _ => (Vec::new(), String::new()),
    }
}

/// An empty Loop cell tapped on the grid (loop-tracks.md part 4): the
/// next loop recorded lands there. The loop is armed for REC, an empty
/// loop picked for it and the looper set to record the track's input.
/// With no loop free, the memory full or a take recording, it says why
/// and arms nothing.
fn arm_loop_cell(
    app: &mut app::App,
    time: &mut time::TimeHost,
    tracks: &mut audiotracks::TrackHost,
    looper: &mut looping::LoopHost,
    mix: &mut mixing::MixHost,
    out: &mut Out,
    (t, s): (usize, usize),
) -> Vec<app::Effect> {
    use crate::rec_arm::{Mover, Target};
    let n = match looper.free_loop() {
        Ok(n) => n,
        Err(why) => return vec![app::Effect::Tell(why)],
    };
    let now = rec_now(time, looper);
    let was = app.rec;
    let done = app.rec.move_to(Target::Loop, Mover::Tap, &now);
    if !app.rec.armed(Target::Loop) {
        return rec_effects(done, time, looper);
    }
    if app.rec != was {
        out.note(&format!("rec: {} armed", app.rec.name()));
    }
    let mut fx = crate::time_action(
        out,
        time,
        tracks,
        looper,
        app,
        mix,
        mpfree_time_ui::TimeAction::LoopCell(Some((t, s))),
    );
    out.show(gui::ToUi::Rec(app.rec));
    let input = time
        .project()
        .tracks
        .get(t)
        .and_then(|tr| tr.loops())
        .unwrap_or_default();
    fx.push(app::Effect::Looper(looping::LoopAction::Select(n)));
    fx.extend(app.loop_input(input));
    fx.extend(rec_effects(done, time, looper));
    if let Some(name) = mpfree_time_ui::view::loop_cell_name(&time.state()) {
        fx.push(app::Effect::Tell(format!(
            "REC \u{2192} {name}: REC records a loop there, from {}",
            input.name().to_lowercase()
        )));
    }
    fx
}

/// SHIFT + a loop's cell dropped on an empty cell of another Loop track
/// (loop-tracks.md part 6): the cell there takes an empty loop, and the
/// looper copies the loop's layers into it, silent until the cell is
/// launched. Says why not: no loop free, the memory, the loop busy.
fn copy_loop_cell(
    time: &mut time::TimeHost,
    looper: &mut looping::LoopHost,
    out: &mut Out,
    from: (usize, usize),
    to: (usize, usize),
) -> Vec<app::Effect> {
    use mpfree_time_ui::TimeAction::{LoopCopy, LoopCopyOff};
    let tell = |s: String| vec![app::Effect::Tell(s)];
    let Some(n) = time
        .project()
        .clip(from.0, from.1)
        .and_then(|c| c.looped)
        .map(usize::from)
    else {
        return tell("clips: nothing to copy there".to_string());
    };
    let m = match looper.free_loop() {
        Ok(m) => m,
        Err(why) => return tell(why),
    };
    // The cell first: the time engine checks both cells.
    let said = match time.act(LoopCopy {
        from,
        to,
        into: m as u8,
    }) {
        Ok(line) => line,
        Err(why) => return tell(why),
    };
    match looper.copy_loop(n, m) {
        Ok(line) => out.note(&line),
        Err(why) => {
            let _ = time.act(LoopCopyOff(m as u8));
            return tell(why);
        }
    }
    tell(said)
}

/// A loop's copy onto another Loop track ended
/// ([`looping::LoopHost::copy_ended`]): all in, or stopped with some of
/// its layers, or with none, so its cell goes.
pub fn copy_ended(time: &mut time::TimeHost, e: looping::CopyEnd) -> Vec<app::Effect> {
    let (from, to) = (e.from + 1, e.to + 1);
    if e.layers == 0 {
        let mut fx = Vec::new();
        if let Ok(line) = time.act(mpfree_time_ui::TimeAction::LoopCopyOff(e.to as u8)) {
            if !line.is_empty() {
                fx.push(app::Effect::Note(format!("time: {line}")));
            }
        }
        fx.push(app::Effect::Tell(format!(
            "Loop {from} didn't copy into Loop {to}: the loops were busy, so try again"
        )));
        return fx;
    }
    let s = if e.of == 1 { "" } else { "s" };
    if e.layers < e.of {
        return vec![app::Effect::Tell(format!(
            "Loop {to} has {} of Loop {from}'s {} layers: the copy stopped",
            e.layers, e.of
        ))];
    }
    vec![app::Effect::Note(format!(
        "looper: loop {to} is a copy of loop {from}, {} layer{s}",
        e.of
    ))]
}

/// What the arm does, as the app's effects (in order).
fn rec_effects(
    done: Vec<crate::rec_arm::Do>,
    time: &time::TimeHost,
    looper: &looping::LoopHost,
) -> Vec<app::Effect> {
    use crate::rec_arm::Do;
    let mut fx = Vec::new();
    for d in done {
        match d {
            Do::Transport(c) => {
                fx.push(app::Effect::Time(mpfree_time_ui::TimeAction::Transport(c)))
            }
            Do::Looper(c) => fx.extend(loop_take(c, time, looper)),
            Do::StopWaiting => fx.push(app::Effect::Looper(looping::LoopAction::StopWaiting)),
            Do::ThrowTake => fx.push(app::Effect::Time(mpfree_time_ui::TimeAction::ThrowTake)),
            Do::Pad(d) => fx.push(app::Effect::PadTake(d)),
            Do::Tracks(b) => fx.push(app::Effect::Time(mpfree_time_ui::TimeAction::ArmAlso(b))),
            Do::Tell(s) => fx.push(app::Effect::Tell(s)),
            Do::Note(s) => fx.push(app::Effect::Note(s)),
        }
    }
    fx
}

/// The loop's Record or Overdub from REC with a Loop cell armed on the
/// grid (loop-tracks.md part 4): Record makes a new loop in an empty one,
/// from the track's input (Resample: the other loops as heard), or says
/// why it can't; Overdub layers the loop playing on that Loop track. A
/// first pass or an overdub under way takes them as the looper does.
fn loop_take(
    c: mpfree_looper::Command,
    time: &time::TimeHost,
    looper: &looping::LoopHost,
) -> Vec<app::Effect> {
    use looping::LoopAction;
    use mpfree_looper::{Command, Phase};
    let plain = vec![app::Effect::Looper(LoopAction::Command(c))];
    let Some((t, _)) = time.loop_cell() else {
        return plain;
    };
    if matches!(
        looper.phase(),
        Phase::Armed | Phase::Recording | Phase::Overdubbing
    ) {
        return plain;
    }
    let track = time.project().tracks.get(t);
    match c {
        Command::Record => match looper.free_loop() {
            Ok(n) => {
                let resample =
                    track.and_then(|tr| tr.loops()) == Some(mpfree_seq::LoopInput::Resample);
                vec![
                    app::Effect::Looper(LoopAction::Select(n)),
                    app::Effect::Looper(LoopAction::Command(if resample {
                        Command::Resample
                    } else {
                        Command::Record
                    })),
                ]
            }
            Err(why) => vec![app::Effect::Tell(why)],
        },
        Command::Overdub => match time.loop_on(t) {
            Some(n) => vec![
                app::Effect::Looper(LoopAction::Select(n)),
                app::Effect::Looper(LoopAction::Command(Command::Overdub)),
            ],
            None => vec![app::Effect::Tell(format!(
                "Nothing plays on {} to overdub: REC records a loop into its armed cell",
                track.map_or("the Loop track", |tr| tr.name.as_str())
            ))],
        },
        _ => plain,
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
        View::Play | View::Sequencer | View::Arrange => gui::ToUi::PlayQLink(k, ticks),
        _ => return None,
    })
}

/// Other gear's MMC commands and program changes since the last pass
/// (the Sync section's MMC in and Program change in), as effects: MMC as
/// the MPC's PLAY, STOP and REC through the arm ([`app::App::mmc`]), a
/// Locate along the tempo map ([`mmc_locate`]), a scene launched as its
/// button launches it. A program past the Clips grid's last scene is
/// noted and does nothing.
pub fn remote_in(
    music: &mut crate::midi_in::MusicIn,
    time: &time::TimeHost,
    app: &mut app::App,
    out: &mut Out,
) -> Vec<app::Effect> {
    use crate::midi_in::RemoteIn;
    use mpfree_midi::mmc::Mmc;
    let mut fx = Vec::new();
    while let Some(r) = music.next_remote_in() {
        let a = match r {
            RemoteIn::Mmc(Mmc::Locate(at)) => Some(mmc_locate(at, time.tempo_map())),
            RemoteIn::Mmc(c) => {
                fx.extend(app.mmc(c));
                None
            }
            RemoteIn::Scene(k) => {
                let c = mpfree_play::mpfree_param::clips::Clips::Scene(k);
                let a = mpfree_time_ui::view::clips_press(&time.state(), c);
                if a.is_none() {
                    let n = u16::from(k) + 1;
                    out.note(&format!(
                        "Program change {n}: the Clips grid has no scene {n}"
                    ));
                }
                a
            }
        };
        fx.extend(a.map(app::Effect::Time));
    }
    fx
}

/// Where other gear's MMC Locate goes (the Sync section's MMC in): the
/// tick its time reaches along the tempo map from 0, so 00:00:00:00 is
/// the start and Locate zero then Play is PLAY START.
pub fn mmc_locate(
    at: mpfree_midi::mmc::Smpte,
    map: &mpfree_transport::TempoMap,
) -> mpfree_time_ui::TimeAction {
    let frames = at.secs() * f64::from(crate::RATE);
    let tick = map.tick_after(0.0, frames, crate::RATE);
    mpfree_time_ui::TimeAction::Transport(mpfree_transport::Command::Locate(tick.round() as i64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_play::mpfree_param::{Owner, ParamAddr};

    /// LFO to CC: a gear lane's mod rows ask the mod matrix; each answer
    /// is the rows on its control, and a change says what the matrix moves
    /// of the tracks' MIDI out. A row aimed elsewhere, or a control that
    /// isn't a track's MIDI out, is left alone.
    #[test]
    fn a_gear_lanes_mod_rows_add_set_and_take_off_an_lfo() {
        use gui::LaneRowsAsk as Ask;
        use gui::ToUi;
        let (tx, rx) = std::sync::mpsc::sync_channel::<ToUi>(64);
        let mut out = Out {
            console: None,
            sink: Some(tx),
            dropped: 0,
            report: None,
            queued: None,
            last_draw: std::time::Instant::now(),
        };
        let mut mix = mixing::MixHost::new(44_100, 0.0, 6.0);
        let cc = ParamAddr::new(Owner::Midi(1), 43);
        let other = ParamAddr::new(Owner::Midi(1), 44);
        let mut ask = |mix: &mut mixing::MixHost, a: Ask| {
            lane_rows(mix, a, &mut out);
            rx.try_iter()
                .filter_map(|m| match m {
                    ToUi::LaneRows {
                        target,
                        rows,
                        can_add,
                    } => Some(format!("{target:?} {rows:?} {can_add}")),
                    ToUi::GearMoved(v) => Some(format!("moved {v:?}")),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(ask(&mut mix, Ask::Show(cc)), [format!("{cc:?} [] true")]);
        assert_eq!(
            ask(&mut mix, Ask::AddLfo(cc)),
            [
                format!("{cc:?} [(0, \"LFO 1\", 0.25)] true"),
                format!("moved [{cc:?}]"),
            ]
        );
        assert!(mix
            .modu
            .change(
                &mix.mixer,
                mpfree_mod::Change::AddRow(mpfree_mod::Row {
                    source: mpfree_mod::Source::Lfo(1),
                    via: None,
                    target: other,
                    depth: 0.5,
                })
            )
            .is_ok());
        // Row 1 is CC 44's: CC 43's lane can't touch it.
        assert_eq!(
            ask(
                &mut mix,
                Ask::Depth {
                    target: cc,
                    row: 1,
                    depth: 0.0
                }
            ),
            [format!("{cc:?} [(0, \"LFO 1\", 0.25)] true")]
        );
        assert_eq!(mix.modu.rows_on(other)[0].2, 0.5);
        let set = ask(
            &mut mix,
            Ask::Depth {
                target: cc,
                row: 0,
                depth: -2.0,
            },
        );
        assert_eq!(set[0], format!("{cc:?} [(0, \"LFO 1\", -1.0)] true"));
        let off = ask(&mut mix, Ask::Remove { target: cc, row: 0 });
        assert_eq!(
            off,
            [format!("{cc:?} [] true"), format!("moved [{other:?}]")]
        );
        // Not a track's MIDI out: nothing asked, nothing said.
        let strip = ParamAddr::new(Owner::Strip(mpfree_play::mpfree_param::Strip::Track(0)), 0);
        assert!(ask(&mut mix, Ask::AddLfo(strip)).is_empty());
        assert!(mix.modu.rows_on(strip).is_empty());
    }

    /// Other gear's MMC Locate goes along the tempo map: 0 s is the
    /// start, 2 s at 120 BPM is beat 4.
    #[test]
    fn mmc_locate_goes_along_the_tempo_map() {
        use mpfree_midi::mmc::{Rate, Smpte};
        use mpfree_time_ui::TimeAction;
        use mpfree_transport::Command;
        let time = time::TimeHost::new(44_100, 120.0).unwrap();
        let at = |seconds| Smpte {
            rate: Rate::Fps25,
            hours: 0,
            minutes: 0,
            seconds,
            frames: 0,
            subframes: 0,
        };
        let go = |s| mmc_locate(at(s), time.tempo_map());
        assert_eq!(go(0), TimeAction::Transport(Command::Locate(0)));
        let ticks = 4 * i64::from(mpfree_transport::PPQ);
        assert_eq!(go(2), TimeAction::Transport(Command::Locate(ticks)));
    }

    /// Sample Recall and the Wave page's Keep card wait while a take is
    /// ready, counting in or recording: the notes' take ready, or a pad
    /// take armed (one-page step 2c, the sequencer thread's rule).
    #[test]
    fn recall_and_keep_wait_for_a_take_that_is_ready_or_recording() {
        let mut app = app::App::new(44_100, false);
        let time = time::TimeHost::new(44_100, 92.0).unwrap();
        let looper = looping::LoopHost::new(44_100, 8);
        assert!(!take_waits(&app, &time, &looper), "nothing taking");
        app.rec.ready = Some(crate::rec_arm::Take::Replace);
        assert!(take_waits(&app, &time, &looper), "the notes' take ready");
        app.rec.ready = None;
        app.rec.pad = Some(crate::rec_arm::PadTake {
            kit: 0,
            pad: 5,
            id: 1,
            started: false,
            recording: false,
            here: true,
        });
        assert!(take_waits(&app, &time, &looper), "a pad take armed");
        assert!(KEEP_WAITS.starts_with("Finish the take first"));
        assert!(RECALL_WAITS.starts_with("Finish the take first"));
    }

    /// REC while the synth is chosen for the pads: a track nobody set up
    /// gets the synth and is armed (Track 1, the Drums track, never), no
    /// clip changes, the pads keep the synth, and the next press records
    /// there; with the loop armed REC is the loop's as ever. A track with a kit or MIDI output picked or mixer
    /// settings is passed over; with none free a new track is added; a
    /// pinned arm stays, and REC goes nowhere, saying why. The arp come on
    /// gives the pads back to the track, and REC is REC's own.
    #[test]
    fn rec_while_the_pads_play_the_synth_records_it_on_a_track_nobody_set_up() {
        use crate::rec_arm::{Press, Target};
        let (tx, rx) = std::sync::mpsc::sync_channel::<gui::ToUi>(64);
        let mut out = Out {
            console: None,
            sink: Some(tx),
            dropped: 0,
            report: None,
            queued: None,
            last_draw: std::time::Instant::now(),
        };
        let mut app = app::App::new(44_100, false);
        let mut time = time::TimeHost::new(44_100, 92.0).unwrap();
        let looper = looping::LoopHost::new(44_100, 8);
        let mut tracks = audiotracks::TrackHost::new(44_100, 10);
        let mut mix = mixing::MixHost::new(44_100, 0.0, 6.0);
        let now = rec_now(&time, &looper);
        let mut rec = |app: &mut app::App,
                       time: &mut time::TimeHost,
                       mix: &mut mixing::MixHost,
                       out: &mut Out| {
            rec_synth(app, time, &mut tracks, &looper, mix, out, Press::Rec, &now)
        };
        let to = |fx: &[app::Effect]| {
            fx.iter().find_map(|e| match e {
                app::Effect::SynthOnTrack(t) => Some(*t),
                _ => None,
            })
        };
        // The recording plan the REC chip names before each press: what
        // REC then does (step 5b).
        let plan = |app: &app::App, time: &time::TimeHost, mix: &mixing::MixHost| {
            rec_plan(app, time, mix).map(|p| p.take)
        };
        use app::SynthTake;
        // The drums on Track 1.
        assert_eq!(time.armed(), Some(0));
        time.put_clip(mpfree_seq::Clip::new("Beat", 384)).unwrap();
        let drums = time.project().tracks[0].clone();
        // Without the synth on the pads REC is REC's own.
        assert_eq!(plan(&app, &time, &mix), None);
        assert_eq!(rec(&mut app, &mut time, &mut mix, &mut out), None);
        // With it: Track 2, the first nobody set up, armed for the take.
        app.toggle_pads_synth();
        assert_eq!(plan(&app, &time, &mix), Some(SynthTake::On(1)));
        assert_eq!(
            rec_plan(&app, &time, &mix).map(|p| p.synth),
            Some(app.pads_synth_name())
        );
        let fx = rec(&mut app, &mut time, &mut mix, &mut out).expect("the synth's take");
        assert_eq!(to(&fx), Some(1), "{fx:?}");
        assert_eq!(time.armed(), Some(1), "armed for the take");
        assert_eq!(fx.last(), Some(&app::Effect::Rec(Press::Rec)));
        assert!(app.pads_chose_synth(), "REC never moves the pads");
        assert!(app.track_sound(1).is_none(), "the same synth, no copy");
        assert_eq!(time.project().tracks[0], drums, "the drums unchanged");
        // The press again: Track 2 plays the synth, so REC records there.
        assert_eq!(plan(&app, &time, &mix), None);
        assert_eq!(rec(&mut app, &mut time, &mut mix, &mut out), None);
        // With the loop armed, the loop records what is heard.
        time.act(mpfree_time_ui::TimeAction::Arm(Some(0))).unwrap();
        app.rec.target = Target::Loop;
        assert_eq!(plan(&app, &time, &mix), None);
        assert_eq!(rec(&mut app, &mut time, &mut mix, &mut out), None);
        assert!(app.pads_chose_synth());
        app.rec.target = Target::Notes;
        // A kit picked on Track 2, mixer settings on Track 3: Track 4.
        app.set_configured(1 << 1);
        let p = mpfree_fx::strip_params()
            .iter()
            .find(|p| {
                crate::project::saved_control(mpfree_fx::StripId::Track(mix.track_channel(2)), p.id)
            })
            .expect("a strip control");
        mix.mixer.set_now(
            mpfree_fx::StripId::Track(mix.track_channel(2)),
            p.id,
            if p.default == 0.0 { 0.5 } else { 0.0 },
        );
        assert_eq!(plan(&app, &time, &mix), Some(SynthTake::On(3)));
        let fx = rec(&mut app, &mut time, &mut mix, &mut out).expect("the synth's take");
        assert_eq!(to(&fx), Some(3), "{fx:?}");
        // Pinned on Track 1: REC stays, records nothing and says why.
        time.act(mpfree_time_ui::TimeAction::Arm(Some(0))).unwrap();
        app.rec.pinned = true;
        assert_eq!(
            plan(&app, &time, &mix),
            Some(SynthTake::Pinned {
                track: 0,
                why: "has clips"
            })
        );
        assert_eq!(
            rec(&mut app, &mut time, &mut mix, &mut out),
            Some(Vec::new())
        );
        assert_eq!(time.armed(), Some(0));
        assert!(app.pads_chose_synth(), "nothing changes");
        let said: Vec<String> = rx
            .try_iter()
            .filter_map(|m| match m {
                gui::ToUi::Tell(t) => Some(t),
                _ => None,
            })
            .collect();
        assert!(
            said.iter()
                .any(|t| t.starts_with("REC is pinned to Track 1, which has clips")),
            "{said:?}"
        );
        app.rec.pinned = false;
        // Every track set up: a new one at the end, armed.
        app.set_configured(u128::MAX);
        assert_eq!(plan(&app, &time, &mix), Some(SynthTake::New));
        let count = time.track_count();
        let fx = rec(&mut app, &mut time, &mut mix, &mut out).expect("the synth's take");
        assert_eq!(to(&fx), Some(count), "{fx:?}");
        assert_eq!(time.track_count(), count + 1);
        assert_eq!(time.armed(), Some(count));
        assert!(fx
            .iter()
            .any(|e| matches!(e, app::Effect::Note(n) if n.contains("on a new track, Track"))));
        assert_eq!(time.project().tracks[0], drums, "the drums unchanged");
        // The arp on: the try-out ends and REC is REC's own next period.
        time.act(mpfree_time_ui::TimeAction::Arm(Some(0))).unwrap();
        assert!(app.pads_chose_synth());
        app.toggle_panel();
        assert!(app.generating());
        let fx = rec(&mut app, &mut time, &mut mix, &mut out).expect("the arp's end");
        assert_eq!(
            fx[0],
            app::Effect::PadsFocus(app::PadFocus::Track),
            "{fx:?}"
        );
        assert_eq!(to(&fx), None);
        assert_eq!(fx.last(), Some(&app::Effect::Rec(Press::Rec)));
        assert!(!app.pads_chose_synth());
        assert_eq!(rec(&mut app, &mut time, &mut mix, &mut out), None);
    }

    /// The audio tracks rethink, phase 1b: an audio track's strip's M and
    /// S are the track's mute and solo (a solo project-wide), set only
    /// when they change; a note track's channel is left to the mixer.
    #[test]
    fn an_audio_track_s_strip_m_and_s_are_the_track_s() {
        let mut p = mpfree_seq::Project::new();
        p.tracks.push(mpfree_seq::Track::new("Track 1", 0));
        p.tracks.push(mpfree_seq::Track::new_audio("Vocal", 0));
        let mut time = time::TimeHost::with_project(44_100, 120.0, p).unwrap();
        let mut mix = mixing::MixHost::new(44_100, 0.0, 6.0);
        let mut tracks = crate::audiotracks::TrackHost::new(44_100, 1);
        tracks.sync(time.project(), time.silenced(), true);
        let ch = mix.track_channel(1);
        assert_eq!(audio_track_on(&time, &mix, ch), Some(1));
        assert_eq!(audio_track_on(&time, &mix, mix.track_channel(0)), None);
        assert_eq!(audio_track_on(&time, &mix, mixing::AUDIO), None);
        let said = audio_strip_switch(&mut time, &mut tracks, &mut mix, 1, true, true);
        assert!(matches!(said, Some(Ok(_))), "{said:?}");
        assert!(time.project().tracks[1].solo);
        assert_eq!(time.silenced() & 0b11, 0b01, "the note track is left out");
        let again = audio_strip_switch(&mut time, &mut tracks, &mut mix, 1, true, true);
        assert_eq!(again, None, "soloed already");
        audio_strip_switch(&mut time, &mut tracks, &mut mix, 1, false, true);
        assert!(time.project().tracks[1].mute);
        audio_strip_switch(&mut time, &mut tracks, &mut mix, 1, true, false);
        assert!(!time.project().tracks[1].solo);
        assert_eq!(time.silenced() & 0b11, 0b10, "muted");
    }

    /// Refactor C1: a track picked while REC's arm is pinned is selected
    /// alone (the Sequencer edits it and the pads follow it) and REC stays
    /// on the pinned track; unpinned, a pick arms it too; a row tapped in
    /// REC's list (an arm, not a pick) moves even a pinned arm, and the
    /// selection with it.
    #[test]
    fn a_track_picked_with_the_arm_pinned_is_selected_and_rec_stays() {
        use mpfree_time_ui::TimeAction;
        let (tx, _rx) = std::sync::mpsc::sync_channel::<gui::ToUi>(64);
        let mut out = Out {
            console: None,
            sink: Some(tx),
            dropped: 0,
            report: None,
            queued: None,
            last_draw: std::time::Instant::now(),
        };
        let mut app = app::App::new(44_100, false);
        let mut time = time::TimeHost::new(44_100, 92.0).unwrap();
        let looper = looping::LoopHost::new(44_100, 8);
        let mut tracks = audiotracks::TrackHost::new(44_100, 10);
        let mut mix = mixing::MixHost::new(44_100, 0.0, 6.0);
        let mut act = |app: &mut app::App, time: &mut time::TimeHost, a| {
            crate::time_action(&mut out, time, &mut tracks, &looper, app, &mut mix, a)
        };
        assert_eq!((time.armed(), time.selected()), (Some(0), Some(0)));
        app.rec.pinned = true;
        act(&mut app, &mut time, TimeAction::Pick(2));
        assert_eq!((time.armed(), time.selected()), (Some(0), Some(2)));
        assert_eq!(
            rec_now(&time, &looper).notes,
            "Track 1",
            "the REC chip still names it"
        );
        assert_eq!(time.steps().track, 2, "the Sequencer edits the pick");
        // A row of REC's list is an arm: it moves even a pinned arm.
        act(&mut app, &mut time, TimeAction::Arm(Some(3)));
        assert_eq!((time.armed(), time.selected()), (Some(3), Some(3)));
        // Unpinned, a pick arms and selects.
        app.rec.pinned = false;
        act(&mut app, &mut time, TimeAction::Pick(5));
        assert_eq!((time.armed(), time.selected()), (Some(5), Some(5)));
    }
}
