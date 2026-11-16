//! The device run's app phases, checked here instead of on the MPC (Jay,
//! 15 Nov 22:35Z: "automate as many of these tests as possible using your
//! own UI smoke testing library instead of having me manually test them").
//!
//! Each module follows one phase of the app step
//! (`tools/mpc-one/host_app_mpc.py`, `APP_PHASES`) through the whole
//! session, as a musician would: the buttons, the pads and the screen in,
//! the engines checked behind them. What is left for a device round is
//! what only the MPC can show: that its buttons, pads and Q-Links reach
//! the app at all, the lights, the sound, and how it holds up. The table in
//! `docs/dev/guides/device-phases.md` names, line by line, the test that
//! checks each step of a phase.
//!
//! The modules are named by area, so a round can run one area alone:
//! `cargo test -p mpfree-app --bins smoke::phases::looper`.

#[cfg(test)]
mod looper {
    //! Phase 11, the Looper tab: record, layers, STOP, tools, undo
    //! history, trim, loops, Clear.

    use super::super::{on_big_stack, Session};
    use crate::app::View;
    use mpfree_looper::{Phase, Status};
    use mpfree_transport::time::PPQ;

    fn status(s: &Session) -> Status {
        s.looper.looper.status()
    }

    /// Runs until the loop's phase is `want`, for at most `seconds`.
    fn until_phase(s: &mut Session, want: Phase, seconds: f64) {
        let end = s.t() + seconds;
        while status(s).phase != want && s.t() < end {
            s.run_periods(1);
        }
        assert_eq!(status(s).phase, want, "after {seconds} s");
    }

    /// A loop recorded on the Looper tab with PLAY and REC: two bars from
    /// the next bar line, then it plays.
    fn record_a_loop(s: &mut Session) {
        s.button("REC");
        assert_eq!(status(s).phase, Phase::Armed, "REC waits for the bar line");
        until_phase(s, Phase::Recording, 2.5);
        until_phase(s, Phase::Playing, 4.5);
    }

    /// REC on the Looper tab records 2 bars from the next bar line; OVERDUB
    /// adds a layer from the next pass and REC ends it; STOP stops the loop
    /// and its playhead at once (live test 3: it played on for minutes);
    /// PLAY starts both again.
    #[test]
    fn rec_records_two_bars_and_stop_stops_the_loop_at_once() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("header", "tab", "Looper");
            assert_eq!(s.view(), View::Looper);
            s.button("PLAY").run_for(0.3);
            record_a_loop(&mut s);
            let bar = 4 * PPQ as u64;
            let st = status(&s);
            assert_eq!(st.origin as u64 % bar, 0, "it started on a bar line");
            assert_eq!(st.ticks as u64, 2 * bar, "two bars long");
            assert_eq!(st.layers, 1);

            // OVERDUB: a layer from the next pass; REC ends it.
            s.button("OVERDUB");
            until_phase(&mut s, Phase::Overdubbing, 4.5);
            s.run_for(1.0);
            s.button("REC");
            until_phase(&mut s, Phase::Playing, 4.5);
            assert_eq!(status(&s).layers, 2, "the overdub is a layer");

            // STOP while it plays: the transport and the loop stop now.
            s.run_for(0.5);
            s.button("STOP");
            assert!(!s.time.info().state.rolling(), "the transport stopped");
            let at = status(&s).position;
            s.run_for(1.5);
            assert_eq!(
                status(&s).position,
                at,
                "the loop's playhead stays where STOP left it"
            );
            let view = s.looper.view(120.0);
            assert_eq!(view.status.position, at, "and the screen's with it");

            // PLAY: both go again.
            s.button("PLAY").run_for(0.5);
            assert!(s.time.info().state.rolling());
            assert_ne!(status(&s).position, at, "the loop plays again");
            assert_eq!(status(&s).layers, 2, "with both layers");
            s.button("STOP");
        });
    }

    /// The steps the picked loop's UNDO would take back.
    fn steps(s: &Session) -> Vec<mpfree_looper::Step> {
        s.looper.looper.history().done().to_vec()
    }

    /// A loop of two layers, playing, on the Looper tab.
    fn two_layers() -> Session {
        let mut s = Session::new();
        s.press("header", "tab", "Looper");
        s.button("PLAY").run_for(0.3);
        record_a_loop(&mut s);
        s.button("OVERDUB");
        until_phase(&mut s, Phase::Overdubbing, 4.5);
        s.run_for(1.0);
        s.button("REC");
        until_phase(&mut s, Phase::Playing, 4.5);
        assert_eq!(status(&s).layers, 2);
        s
    }

    /// With the loop playing, each tool does its edit (½ length, × 2,
    /// Reverse, the nudges, the seam fade, Merge layers), UNDO and SHIFT +
    /// UNDO take the last back and put it back, the undo history goes
    /// back to an older step and forward again, and the trim grip trims.
    #[test]
    fn the_tools_the_undo_history_and_trim_work_while_the_loop_plays() {
        use mpfree_looper::{Edit, Step};
        on_big_stack(|| {
            let mut s = two_layers();
            let bar = 4 * PPQ as u64;
            // A tool by the start of its label (the seam fade's says its
            // length: "Seam fade 6 ms").
            let tool = |s: &mut Session, label: &str| {
                let c = s
                    .controls()
                    .into_iter()
                    .find(|c| {
                        c.widget == "leditor"
                            && c.target.id == "tool"
                            && c.target.label.starts_with(label)
                    })
                    .unwrap_or_else(|| panic!("no tool {label}"));
                assert!(c.target.enabled, "{label} works while the loop plays");
                s.press_target(&c.target).run_for(0.2);
            };

            tool(&mut s, "½ length");
            assert_eq!(status(&s).ticks as u64, bar, "half: one bar");
            tool(&mut s, "× 2");
            assert_eq!(status(&s).ticks as u64, 2 * bar, "doubled: two again");
            tool(&mut s, "Reverse");
            tool(&mut s, "← 1/16");
            tool(&mut s, "1/16 →");
            let seam = status(&s).seam;
            tool(&mut s, "Seam fade");
            assert_ne!(status(&s).seam, seam, "the seam fade changed");
            let edits: Vec<Step> = steps(&s).into_iter().skip(1).collect();
            assert!(
                matches!(
                    edits.as_slice(),
                    [
                        Step::Edit(Edit::Half),
                        Step::Edit(Edit::Double),
                        Step::Edit(Edit::Reverse),
                        Step::Edit(Edit::Nudge(-1)),
                        Step::Edit(Edit::Nudge(1)),
                        Step::Edit(Edit::SeamFade(_)),
                    ]
                ),
                "after the overdub, each tool is a step: {edits:?}"
            );

            // UNDO takes the seam fade back; SHIFT + UNDO puts it back.
            let n = steps(&s).len();
            s.button("UNDO").run_for(0.2);
            assert_eq!(steps(&s).len(), n - 1, "UNDO took a step back");
            assert_eq!(status(&s).seam, seam);
            s.button_with("SHIFT", "UNDO").run_for(0.2);
            assert_eq!(steps(&s).len(), n, "SHIFT + UNDO put it back");

            // Merge layers: the two become one, as a render while it plays.
            tool(&mut s, "Merge layers");
            let end = s.t() + 10.0;
            while status(&s).layers != 1 && s.t() < end {
                s.run_for(0.1);
            }
            assert_eq!(status(&s).layers, 1, "merged into one layer");
            assert!(matches!(steps(&s).last(), Some(Step::Merge(_))));

            // The undo history: an older step tapped goes back to it (row
            // k is k steps done; row 0 is the first pass alone), and the
            // newest tapped again goes forward to it.
            let n = steps(&s).len();
            tool(&mut s, "Undo history");
            let rows = |s: &mut Session| -> Vec<mpfree_ui::Target> {
                let mut v: Vec<_> = s
                    .controls()
                    .into_iter()
                    .filter(|c| c.widget == "leditor" && c.target.id == "history-step")
                    .map(|c| c.target)
                    .collect();
                v.sort_by_key(|t| t.index);
                v
            };
            let older = rows(&mut s).first().cloned().expect("older steps listed");
            assert!(older.index < n);
            s.press_target(&older).run_for(0.5);
            assert_eq!(steps(&s).len(), older.index, "back to {}", older.label);
            assert_eq!(status(&s).layers, 2, "before the merge: two layers again");
            // The list shows the rows around the one that is now, so the
            // newest may take a tap or two on the last row shown.
            for _ in 0..4 {
                if steps(&s).len() == n {
                    break;
                }
                let last = rows(&mut s)
                    .last()
                    .cloned()
                    .expect("steps to go forward to");
                s.press_target(&last).run_for(0.5);
                assert_eq!(steps(&s).len(), last.index, "forward to {}", last.label);
            }
            assert_eq!(steps(&s).len(), n, "forward to the newest again");
            assert_eq!(status(&s).layers, 1);
            tool(&mut s, "Undo history");

            // The trim grip at the end, dragged in and let go: shorter.
            let ticks = status(&s).ticks;
            s.press("leditor", "trim-end", "trim end").run_for(0.3);
            assert!(status(&s).ticks < ticks, "trimmed");
            assert!(matches!(
                steps(&s).last(),
                Some(Step::Edit(Edit::Trim { .. }))
            ));
            assert_eq!(status(&s).phase, Phase::Playing, "and it played on");
            s.button("STOP");
        });
    }

    /// Layer `k` of loop `n`, as the engine has it.
    fn layer(s: &Session, n: usize, k: usize) -> mpfree_looper::LayerInfo {
        s.looper
            .looper
            .loop_layer_info(n, k)
            .unwrap_or_else(|| panic!("loop {} has a layer {}", n + 1, k + 1))
    }

    fn layers_of(s: &Session, n: usize) -> usize {
        s.looper.looper.loop_status(n).map_or(0, |st| st.layers)
    }

    /// The control `id` number `index` on the Looper tab's widget `widget`.
    fn at(s: &mut Session, widget: &str, id: &str, index: usize) -> mpfree_ui::Target {
        s.controls()
            .into_iter()
            .find(|c| c.widget == widget && c.target.id == id && c.target.index == index)
            .unwrap_or_else(|| panic!("no {widget}/{id} {index}"))
            .target
    }

    /// A finger on layer row `k`'s lane, moved off it and onto `to`, then
    /// lifted there, as the musician drags it.
    fn drag_layer(s: &mut Session, k: usize, to: mpfree_ui::Rect) {
        let lane = at(s, "leditor", "layer", k).rect;
        let (x, y) = (lane.x + 30, lane.y + lane.h / 2);
        let (tx, ty) = (to.x + to.w / 2, to.y + to.h / 2);
        let mut acts: Vec<crate::gui::FromUi> = s
            .screen
            .touch(7, mpfree_ui::Touch::Down, x, y)
            .into_iter()
            .collect();
        for i in 1..=6 {
            let (mx, my) = (x + (tx - x) * i / 6, y + (ty - y) * i / 6);
            acts.extend(s.screen.touch(7, mpfree_ui::Touch::Move, mx, my));
        }
        acts.extend(s.screen.touch(7, mpfree_ui::Touch::Up, tx, ty));
        acts.extend(s.screen.take_actions());
        for a in acts {
            s.route(a);
        }
        s.run_for(0.3);
    }

    /// A layer row's speaker, S, level bar and a tap on its lane (the
    /// Q-Links then turn it); a second loop from + New loop; a layer
    /// dragged onto it moves there and UNDO takes the move back; one
    /// dragged onto + New loop becomes a loop of its own; a loop's speaker
    /// keeps it silent through STOP and PLAY; Clear empties the picked loop;
    /// x deletes layers, and x on the last one clears the loop too.
    #[test]
    fn layers_move_between_loops_and_speakers_clear_and_x_work() {
        on_big_stack(|| {
            let mut s = two_layers();
            // Layer 2's row.
            let t = at(&mut s, "leditor", "layer-mute", 1);
            s.press_target(&t).run_for(0.1);
            assert!(layer(&s, 0, 1).muted, "the speaker muted layer 2");
            let t = at(&mut s, "leditor", "layer-mute", 1);
            s.press_target(&t).run_for(0.1);
            assert!(!layer(&s, 0, 1).muted);
            let t = at(&mut s, "leditor", "layer-solo", 1);
            s.press_target(&t).run_for(0.1);
            assert!(layer(&s, 0, 1).solo, "S soloed it");
            assert!(!layer(&s, 0, 0).audible, "and left layer 1 out");
            let t = at(&mut s, "leditor", "layer-solo", 1);
            s.press_target(&t).run_for(0.1);
            let level = layer(&s, 0, 1).level;
            let t = at(&mut s, "leditor", "layer-level", 1);
            s.press_target(&t).run_for(0.1);
            assert_ne!(layer(&s, 0, 1).level, level, "the level bar moved it");
            let t = at(&mut s, "leditor", "layer", 0);
            s.press_target(&t).run_for(0.1);
            let level = layer(&s, 0, 0).level;
            s.qlink(0, -6).run_for(0.1);
            assert!(layer(&s, 0, 0).level < level, "Q-Link 1 turned layer 1");

            // + New loop: loop 2, picked, recorded at the same length.
            s.press("lloops", "new-loop", "+ New loop").run_for(0.1);
            assert_eq!(s.looper.looper.selected(), 1);
            record_a_loop(&mut s);
            assert_eq!(layers_of(&s, 1), 1);

            // Back on loop 1, its layer 2 dragged onto loop 2: it moves.
            let t = at(&mut s, "lloops", "loop", 0);
            s.press_target(&t).run_for(0.1);
            assert_eq!(s.looper.looper.selected(), 0);
            let two = at(&mut s, "lloops", "loop", 1).rect;
            drag_layer(&mut s, 1, two);
            assert_eq!(
                (layers_of(&s, 0), layers_of(&s, 1)),
                (1, 2),
                "the layer moved into loop 2"
            );
            assert_eq!(s.looper.looper.selected(), 1, "which is picked");
            // One UNDO takes the whole move back: out of loop 2 and back
            // into loop 1.
            s.button("UNDO").run_for(0.3);
            assert_eq!(
                (layers_of(&s, 0), layers_of(&s, 1)),
                (2, 1),
                "UNDO took the move back"
            );
            let t = at(&mut s, "lloops", "loop", 0);
            s.press_target(&t).run_for(0.1);

            // Onto + New loop: a loop of its own (loop 3).
            let new = at(&mut s, "lloops", "new-loop", 0).rect;
            drag_layer(&mut s, 1, new);
            assert_eq!(layers_of(&s, 0), 1);
            assert_eq!(layers_of(&s, 2), 1, "loop 3 has it");
            assert_eq!(s.looper.looper.selected(), 2);

            // Loop 1's speaker: silent, and it stays so through STOP and
            // PLAY; tapped again it plays.
            let t = at(&mut s, "lloops", "loop-mute", 0);
            s.press_target(&t).run_for(0.1);
            let muted = |s: &Session| s.looper.looper.loop_status(0).unwrap().muted;
            assert!(muted(&s), "loop 1 is silent");
            s.button("STOP").run_for(0.3);
            s.button("PLAY").run_for(0.3);
            assert!(muted(&s), "and stays out after STOP and PLAY");
            assert_ne!(layers_of(&s, 0), 0, "but it is kept");
            let t = at(&mut s, "lloops", "loop-mute", 0);
            s.press_target(&t).run_for(0.1);
            assert!(!muted(&s), "back in");

            // Loop 2 picked and cleared, first pass and all.
            let t = at(&mut s, "lloops", "loop", 1);
            s.press_target(&t).run_for(0.1);
            s.press("lloops", "clear", "Clear").run_for(0.3);
            assert_eq!(s.looper.looper.loop_status(1).unwrap().phase, Phase::Empty);

            // Loop 1: an overdub, then x on each layer down to the last;
            // x on that one clears the loop.
            let t = at(&mut s, "lloops", "loop", 0);
            s.press_target(&t).run_for(0.1);
            s.button("OVERDUB");
            until_phase(&mut s, Phase::Overdubbing, 4.5);
            s.run_for(1.0);
            s.button("REC");
            until_phase(&mut s, Phase::Playing, 4.5);
            assert_eq!(layers_of(&s, 0), 2);
            let t = at(&mut s, "leditor", "layer-delete", 1);
            s.press_target(&t).run_for(0.3);
            assert_eq!(layers_of(&s, 0), 1, "x deleted layer 2");
            let t = at(&mut s, "leditor", "layer-delete", 0);
            s.press_target(&t).run_for(0.3);
            assert_eq!(
                s.looper.looper.loop_status(0).unwrap().phase,
                Phase::Empty,
                "x on the last layer cleared the loop"
            );
            s.button("STOP");
        });
    }
}

#[cfg(test)]
mod mixer {
    //! Phase 12, the Mix tab: TRACK MIX, strip UNDO, an effect's modes
    //! with learned Q-Links.

    use super::super::{on_big_stack, Session};
    use crate::app::View;
    use mpfree_chain_ui::ChainState;
    use mpfree_fx::ids::mixer as mx;
    use mpfree_fx::StripId;
    use mpfree_mixer_ui::{MixView, MixerView, MIXER};

    const DRUMS: StripId = StripId::Track(0);

    fn mix_view(s: &Session) -> MixView {
        s.screen
            .ui
            .get::<MixerView>(MIXER)
            .expect("the mixer")
            .state()
            .view
    }

    /// TRACK MIX opens the Mix tab and goes back to the page it came from;
    /// SHIFT + TRACK MIX opens it on the pads. On the strips, UNDO takes
    /// back the last strip change (a mute after a fader) and SHIFT + UNDO
    /// puts it back; UNDO again takes back the fader.
    #[test]
    fn track_mix_comes_and_goes_and_undo_takes_back_strip_changes() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("header", "tab", "Arrange");
            s.button("TRACK MIX / PAD MIX").run_for(0.1);
            assert_eq!(s.view(), View::Mixer);
            assert_eq!(mix_view(&s), MixView::Tracks);
            s.button("TRACK MIX / PAD MIX").run_for(0.1);
            assert_eq!(s.view(), View::Arrange, "back to the page it came from");
            s.button_with("SHIFT", "TRACK MIX / PAD MIX").run_for(0.1);
            assert_eq!(s.view(), View::Mixer);
            assert_eq!(mix_view(&s), MixView::Pads, "SHIFT: on the pads");
            s.button("TRACK MIX / PAD MIX").run_for(0.1);
            s.button("TRACK MIX / PAD MIX").run_for(0.1);
            assert_eq!(mix_view(&s), MixView::Tracks);

            let p = |s: &Session, id| s.mix.mixer.param(DRUMS, id);
            let v0 = p(&s, mx::VOLUME);
            s.press("mixer", "fader", "Drums fader").run_for(2.0);
            let v1 = p(&s, mx::VOLUME);
            assert!(v1 > v0, "the fader went up");
            s.press("mixer", "mute", "Drums M").run_for(0.2);
            assert_eq!(p(&s, mx::MUTE), 1.0);

            s.button("UNDO").run_for(0.2);
            assert_eq!(p(&s, mx::MUTE), 0.0, "UNDO took the mute back");
            assert_eq!(p(&s, mx::VOLUME), v1, "not the fader");
            s.button_with("SHIFT", "UNDO").run_for(0.2);
            assert_eq!(p(&s, mx::MUTE), 1.0, "SHIFT + UNDO put it back");
            s.button("UNDO").run_for(0.2);
            s.button("UNDO").run_for(0.2);
            assert_eq!(p(&s, mx::MUTE), 0.0);
            assert_eq!(p(&s, mx::VOLUME), v0, "and then the fader");
            s.run_for(0.2);
            let strip = s
                .screen
                .ui
                .get::<MixerView>(MIXER)
                .unwrap()
                .state()
                .strips
                .iter()
                .find(|v| v.name == "Drums")
                .cloned()
                .expect("the Drums strip");
            assert!((strip.volume_db - v0).abs() < 0.05, "the strip shows it");
            assert!(!strip.mute);
        });
    }

    /// The chain the editor stands for, as the mixer has it.
    fn engine(s: &mut Session) -> ChainState {
        s.panels
            .chain_state(&s.mix)
            .expect("the chain editor is open")
    }

    /// The value insert `slot` of the Drums strip holds for its knob
    /// `name` (None when it has no such knob).
    fn knob(s: &Session, slot: usize, name: &str) -> Option<f32> {
        let n = s.mix.mixer.chain(DRUMS)?.node(slot)?;
        let id = n.params().iter().find(|p| p.name == name)?.id;
        Some(n.param(id))
    }

    /// Insert `slot`'s knob `name` on the Drums strip, as the effect
    /// describes it (its range).
    fn knob_info(s: &Session, slot: usize, name: &str) -> Option<mpfree_node::ParamInfo> {
        let n = s.mix.mixer.chain(DRUMS)?.node(slot)?;
        n.params().iter().find(|p| p.name == name).copied()
    }

    /// The Q-Link cards' labels on the page now.
    fn cards(s: &mut Session) -> Vec<String> {
        s.controls()
            .into_iter()
            .filter(|c| c.widget == "qlinks")
            .map(|c| c.target.label)
            .collect()
    }

    /// The knob `name` on the chain editor's page, going through its
    /// pages until it is there.
    fn touch_knob(s: &mut Session, name: &str) {
        for g in 0..8 {
            if s.control("chain", "knob", name).is_some() {
                s.press("chain", "knob", name);
                return;
            }
            let Some(tab) = s
                .controls()
                .into_iter()
                .find(|c| c.widget == "chain" && c.target.id == "group" && c.target.index == g)
            else {
                continue;
            };
            s.press_target(&tab.target);
        }
        panic!("no {name} on any page of {:?}", engine(s).title);
    }

    /// Picks the mode labelled `label` on the editor's Mode chooser (a
    /// button per mode, or the list behind the chip).
    fn pick_mode(s: &mut Session, label: &str) {
        if s.control("chain", "mode", label).is_some() {
            s.press("chain", "mode", label);
        } else {
            s.press("chain", "mode-list", "");
            s.press("chain", "mode-pick", label);
        }
        s.run_for(0.5);
    }

    /// The Drums row's + FX, Reverb: a Room. Damping learned on Q-Link 2
    /// and Early on Q-Link 3. Plate on the Mode chooser keeps Decay, Size
    /// and Mix (as far as the Plate's ranges go), Q-Link 2 still turns
    /// Damping and Early's Q-Link comes off (a Plate has no Early). Room
    /// again is as it was left.
    #[test]
    fn room_to_plate_keeps_the_shared_knobs_and_learned_q_links_follow() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("header", "tab", "Mix");
            s.press("mixer", "look", "Effects");
            s.press("mixer", "fx-add", "Drums + FX");
            assert_eq!(s.view(), View::Chain);
            s.press("chain", "section", "Delay and reverb");
            s.press("chain", "pick", "Reverb").run_for(0.3);
            let names: Vec<String> = engine(&mut s)
                .slots
                .iter()
                .map(|x| x.name.clone())
                .collect();
            assert_eq!(names, ["Filter · DJ", "Reverb · Room"]);
            let slot = 1;

            // Learn: Damping on Q-Link 2, Early on Q-Link 3.
            for (name, q) in [("Damping", 1), ("Early", 2)] {
                touch_knob(&mut s, name);
                s.press("chain", "learn", "Learn");
                let n = s.sent.len();
                s.qlink(q, 2).run_for(0.1);
                assert!(
                    s.sent[n..].iter().any(|a| a == "ChainLearned"),
                    "{name}: {:?}",
                    &s.sent[n..]
                );
                assert_eq!(cards(&mut s)[q], name, "Q-Link {} has it", q + 1);
            }
            let damping = knob(&s, slot, "Damping").unwrap();
            s.qlink(1, 6).run_for(0.2);
            assert!(
                knob(&s, slot, "Damping").unwrap() > damping,
                "Q-Link 2: Damping"
            );
            let room: Vec<Option<f32>> = ["Decay", "Size", "Mix", "Damping", "Early"]
                .iter()
                .map(|k| knob(&s, slot, k))
                .collect();

            // Plate: the shared knobs stay, each as far as the Plate's
            // range goes (its Size runs 50-150 %, so the Room's 45 % is
            // 50 %); Damping keeps Q-Link 2; Early's Q-Link is free again.
            pick_mode(&mut s, "Reverb mode Plate");
            assert_eq!(engine(&mut s).slots[slot].name, "Reverb · Plate");
            for (k, name) in ["Decay", "Size", "Mix"].iter().enumerate() {
                let v = knob(&s, slot, name).unwrap_or_else(|| panic!("a Plate has {name}"));
                let want = knob_info(&s, slot, name).unwrap().clamp(room[k].unwrap());
                assert!(
                    (v - want).abs() < 1e-3,
                    "{name} stayed: {v} (the Room had {:?})",
                    room[k]
                );
            }
            assert_eq!(knob(&s, slot, "Early"), None, "a Plate has no Early");
            let c = cards(&mut s);
            assert_eq!(c[1], "Damping", "Q-Link 2 still Damping: {c:?}");
            assert_ne!(c[2], "Early", "Early's Q-Link came off: {c:?}");
            let damping = knob(&s, slot, "Damping").unwrap();
            s.qlink(1, -6).run_for(0.2);
            assert!(
                knob(&s, slot, "Damping").unwrap() < damping,
                "Q-Link 2 turns the Plate's Damping"
            );

            // Room again: as it was left.
            pick_mode(&mut s, "Reverb mode Room");
            assert_eq!(engine(&mut s).slots[slot].name, "Reverb · Room");
            for (k, name) in ["Decay", "Size", "Mix", "Early"].iter().enumerate() {
                let i = if k == 3 { 4 } else { k };
                let v = knob(&s, slot, name).unwrap();
                assert!(
                    (v - room[i].unwrap()).abs() < 1e-3,
                    "the Room's {name} is back: {v}, was {:?}",
                    room[i]
                );
            }
        });
    }
}

#[cfg(test)]
mod track_mute {
    //! Phase 7, TRACK MUTE and SHIFT + TRACK MUTE.

    use super::super::{on_big_stack, Session};
    use crate::app::{MuteKeys, View};
    use crate::gui::FromUi;
    use mpfree_time_ui::TimeAction;

    /// With 24 tracks: TRACK MUTE's pads are tracks 1-16, PAD BANK B makes
    /// them tracks 17-32 (17-24 here), and the pads stay mute keys on
    /// another tab. SHIFT + TRACK MUTE: each pad mutes itself in the kit,
    /// and a muted pad is silent; SHIFT + TRACK MUTE again: the pads play.
    #[test]
    fn track_mute_banks_reach_past_16_tracks_and_shift_mutes_the_kits_pads() {
        on_big_stack(|| {
            let mut s = Session::new();
            while s.time.track_count() < 24 {
                s.route(FromUi::Time(TimeAction::AddTrack));
                s.run_periods(2);
            }
            s.run_for(0.1);
            s.button("TRACK MUTE");
            assert_eq!(s.app.mute_keys(), MuteKeys::Tracks);
            assert!(
                s.lines
                    .iter()
                    .any(|l| l.contains("TRACK MUTE · bank A: pads 1-16 mute tracks 1-16")),
                "{:?}",
                &s.lines[s.lines.len().saturating_sub(4)..]
            );
            s.pad(1, 100, 0.05);
            assert_eq!(s.time.muted(1), Some(true), "pad 2: track 2");
            s.pad(1, 100, 0.05);
            assert_eq!(s.time.muted(1), Some(false), "again: playing");

            s.button("PAD BANK B/F");
            assert_eq!(s.app.mute_bank(), 1);
            assert_eq!(s.kit, 0, "the kit stays");
            s.pad(0, 100, 0.05);
            assert_eq!(s.time.muted(16), Some(true), "bank B, pad 1: track 17");
            assert_eq!(s.time.muted(0), Some(false), "not track 1");

            // On another tab the pads are still mute keys.
            s.press("header", "tab", "Looper");
            assert_eq!(s.view(), View::Looper);
            s.pad(7, 100, 0.05);
            assert_eq!(s.time.muted(23), Some(true), "pad 8: track 24");
            s.pad(8, 100, 0.05);
            assert_eq!(s.time.track_count(), 24, "pad 9 has no track: nothing");
            s.button("PAD BANK A/E");
            s.press("header", "tab", "Play");

            // SHIFT + TRACK MUTE: the kit's pads.
            s.make_sounds();
            s.button_with("SHIFT", "TRACK MUTE");
            assert_eq!(s.app.mute_keys(), MuteKeys::Pads);
            s.pad(4, 100, 0.05);
            assert_eq!(s.mix.pad_mutes(), 1 << 4, "pad 5 muted in the kit");
            assert_eq!(s.time.muted(4), Some(false), "not track 5");
            s.button_with("SHIFT", "TRACK MUTE");
            assert_eq!(s.app.mute_keys(), MuteKeys::Off, "the pads play again");
            let loud = |s: &mut Session, pad: usize| -> f32 {
                s.heard = Some(Vec::new());
                s.pad(pad, 110, 0.05).run_for(0.1);
                let h = s.heard.take().unwrap();
                h.iter().fold(0f32, |m, x| m.max(x.abs()))
            };
            assert!(loud(&mut s, 5) > 0.01, "pad 6 sounds");
            assert!(loud(&mut s, 4) < 1e-4, "pad 5 is muted in the kit");
            s.button_with("SHIFT", "TRACK MUTE");
            s.pad(4, 100, 0.05);
            s.button_with("SHIFT", "TRACK MUTE");
            assert_eq!(s.mix.pad_mutes(), 0);
            assert!(loud(&mut s, 4) > 0.01, "pad 5 sounds again");
        });
    }
}

#[cfg(test)]
mod device_log {
    //! The log lines the device checks are judged on
    //! (`tools/mpc-one/app_checks.py`): each step done here as on the MPC,
    //! and the line it must write, in the shape the check matches. A line
    //! that changes shape fails here first, so the check can follow it.

    use super::super::{on_big_stack, Session};
    use crate::gui::FromUi;
    use mpfree_time_ui::TimeAction;
    use mpfree_transport::time::PPQ;

    /// The lines written since `from` (the log as the device keeps it).
    fn since(s: &Session, from: usize) -> Vec<String> {
        s.lines[from..].to_vec()
    }

    /// Asserts a line since `from` starts with `start` and holds `has`.
    fn wrote(s: &Session, from: usize, start: &str, has: &str) -> String {
        let lines = since(s, from);
        lines
            .iter()
            .find(|l| l.starts_with(start) && l.contains(has))
            .cloned()
            .unwrap_or_else(|| panic!("no line starting {start:?} with {has:?} in {lines:#?}"))
    }

    /// TRACK MUTE (`^track mute: .* muted`, `^TRACK MUTE \S+ bank [B-H]: `).
    #[test]
    fn track_mute_writes_its_mutes_and_its_bank() {
        on_big_stack(|| {
            let mut s = Session::new();
            while s.time.track_count() < 24 {
                s.route(FromUi::Time(TimeAction::AddTrack));
                s.run_periods(2);
            }
            s.button("TRACK MUTE");
            let n = s.lines.len();
            s.pad(2, 100, 0.05);
            wrote(&s, n, "track mute: ", " muted");
            let n = s.lines.len();
            s.button("PAD BANK B/F");
            let l = wrote(&s, n, "TRACK MUTE · bank B: ", "");
            assert_eq!(l, "TRACK MUTE · bank B: pads 1-8 mute tracks 17-24");
        });
    }

    /// The clip grid and the Song view: a clip moved, a copy, a block
    /// moved (`^clips: .* moved to `, `^clips: a copy of `,
    /// `^song: .* moved to `).
    #[test]
    fn clip_and_block_moves_write_their_lines() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("play-transport", "view", "Beats");
            s.press("steps", "step", "Kick step 1");
            s.run_for(0.1);
            assert!(
                s.time.state().tracks[0].slots[0].is_some(),
                "a clip in row 1"
            );
            let n = s.lines.len();
            s.route(FromUi::Time(TimeAction::DropClip {
                from: (0, 0),
                to: (0, 1),
                copy: false,
            }));
            s.run_periods(2);
            wrote(&s, n, "clips: ", " moved to ");
            let n = s.lines.len();
            s.route(FromUi::Time(TimeAction::DropClip {
                from: (0, 1),
                to: (0, 2),
                copy: true,
            }));
            s.run_periods(2);
            wrote(&s, n, "clips: a copy of ", "");

            s.route(FromUi::Time(TimeAction::ScenesToSong));
            s.run_periods(2);
            let n = s.lines.len();
            s.route(FromUi::Time(TimeAction::MovePlacement {
                index: 0,
                track: 0,
                start: 16 * PPQ as mpfree_transport::time::Tick,
                copy: false,
            }));
            s.run_periods(2);
            wrote(&s, n, "song: ", " moved to ");
        });
    }

    /// NOTE REPEAT: - / + while it is held name the mode (`^arp: `);
    /// SHIFT + NOTE REPEAT latches (`^Latch on`); STOP with the latch on
    /// (`^STOP: (all sounds stopped|arp silenced)`). The Looper's REC
    /// (`^looper: Record: armed`).
    #[test]
    fn note_repeat_latch_stop_and_the_looper_write_their_lines() {
        on_big_stack(|| {
            let mut s = Session::new();
            let n = s.lines.len();
            s.button_with("NOTE REPEAT", "+");
            wrote(&s, n, "arp: ", "");
            let n = s.lines.len();
            s.button_with("SHIFT", "NOTE REPEAT");
            wrote(&s, n, "Latch on", "");
            let n = s.lines.len();
            s.button("STOP");
            let lines = since(&s, n);
            assert!(
                lines
                    .iter()
                    .any(|l| l.starts_with("STOP: all sounds stopped")
                        || l.starts_with("STOP: arp silenced")),
                "{lines:#?}"
            );
            s.button("STOP");

            s.press("header", "tab", "Looper");
            s.button("PLAY").run_for(0.2);
            let n = s.lines.len();
            s.button("REC");
            wrote(&s, n, "looper: Record: armed", "");
            s.button("STOP");
        });
    }
}
