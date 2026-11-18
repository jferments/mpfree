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
    use mpfree_looper::{Overdubs, Phase, Status};
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

    /// A loop recorded on the Looper tab with PLAY and REC, from the next
    /// bar line, two bars long, then it plays: free length (the default),
    /// closed by REC again a bar and a half in, on the bar line; or a set
    /// length (+ New loop takes the length of the loop picked before).
    fn record_a_loop(s: &mut Session) {
        s.button("REC");
        assert_eq!(status(s).phase, Phase::Armed, "REC waits for the bar line");
        until_phase(s, Phase::Recording, 2.5);
        if status(s).bars.is_none() {
            s.run_for(3.0);
            s.button("REC");
            assert!(status(s).ending, "REC again: it ends on the bar line");
            until_phase(s, Phase::Playing, 1.5);
        } else {
            until_phase(s, Phase::Playing, 4.5);
        }
    }

    /// REC on the Looper tab records from the next bar line until REC
    /// again, which closes it on the bar line; OVERDUB adds a layer from
    /// the next pass and REC ends it; STOP stops the loop and its playhead
    /// at once (live test 3: it played on for minutes); PLAY starts both
    /// again.
    #[test]
    fn rec_closes_on_the_bar_line_and_stop_stops_the_loop_at_once() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.make_sounds();
            s.press("header", "tab", "Looper");
            assert_eq!(s.view(), View::Looper);
            s.button("PLAY").run_for(0.3);
            record_a_loop(&mut s);
            let bar = 4 * PPQ as u64;
            let st = status(&s);
            assert_eq!(st.origin as u64 % bar, 0, "it started on a bar line");
            assert_eq!(st.ticks as u64, 2 * bar, "two bars long");
            assert_eq!(st.bars, None, "free length by default (Jay, 16 Nov)");
            assert_eq!(st.layers, 1);

            // OVERDUB: a layer from the next pass (a pad played into it:
            // an overdub of silence leaves none); REC ends it.
            s.button("OVERDUB");
            until_phase(&mut s, Phase::Overdubbing, 4.5);
            s.pad(0, 120, 0.3).run_for(0.7);
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

    /// A loop's first layer as it plays, frame by frame.
    fn first_layer(s: &Session) -> Vec<[f32; 2]> {
        let layer = s.looper.looper.layer(0).expect("a layer");
        let mut x = vec![[0f32; 2]; status(s).frames];
        layer.read(0, &mut x);
        x
    }

    /// REC + PLAY records until STOP (free length, the default), and STOP
    /// stops at once and keeps what was played: the loop runs to the next
    /// bar line, silent after STOP, and PLAY plays it (Jay, 16 Nov).
    #[test]
    fn rec_and_play_record_until_stop_which_keeps_the_take() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.make_sounds();
            s.press("header", "tab", "Looper");
            s.button("REC");
            assert_eq!(status(&s).phase, Phase::Armed, "REC waits for PLAY");
            s.button("PLAY");
            until_phase(&mut s, Phase::Recording, 0.5);
            s.pad(0, 100, 0.05);
            // A bar and a quarter at 120 BPM: still recording.
            s.run_for(2.4);
            assert_eq!(status(&s).phase, Phase::Recording, "no set length");
            s.button("STOP");
            assert!(!s.time.info().state.rolling(), "STOP stops at once");
            s.run_for(0.1);
            let st = status(&s);
            assert_eq!(st.phase, Phase::Playing, "the take is kept");
            assert_eq!(
                st.ticks,
                2 * 4 * PPQ as i64,
                "two bars: to the next bar line"
            );
            let cut = st.cut.expect("cut at STOP");
            let seconds = cut as f64 / 44_100.0;
            assert!((2.4..2.6).contains(&seconds), "cut after {seconds} s");
            assert!(
                s.said()
                    .iter()
                    .any(|l| l.starts_with("looper: loop of 2 bars") && l.contains("cut at STOP")),
                "{:?}",
                s.said()
            );
            // The pad is in it, and nothing after STOP.
            let x = first_layer(&s);
            let peak = |x: &[[f32; 2]]| x.iter().fold(0f32, |m, v| m.max(v[0].abs()));
            assert!(peak(&x[..cut]) > 0.05, "the pad was recorded");
            let quiet_to = st.frames - mpfree_looper::FADE - 1;
            assert_eq!(peak(&x[cut..quiet_to]), 0.0, "nothing after STOP");
            // SHIFT + STOP: back to bar 1. PLAY: the loop plays.
            s.button_with("SHIFT", "STOP").run_for(0.1);
            s.button("PLAY").run_for(0.5);
            assert!(s.time.info().state.rolling());
            assert!(status(&s).position > 0.0, "the loop plays from bar 1");
            s.button("STOP");
        });
    }

    /// The loop records the pads as they are heard, with their effects:
    /// the echo of the Play page's delay on the Drums is in the loop, and
    /// the pad lands where it was played, though the mix comes a period
    /// late (the final round: the echo was heard but not recorded).
    #[test]
    fn the_loop_records_the_pads_with_their_effects() {
        use crate::mixing::{PlayFx, DELAY_RETURN, DRUMS};
        use mpfree_fx::ids::mixer as mx;
        use mpfree_fx::StripId;
        on_big_stack(|| {
            let mut s = Session::new();
            s.make_sounds();
            s.mix.play(PlayFx::Strip {
                strip: StripId::Track(DRUMS),
                id: mx::send(DELAY_RETURN),
                value: 0.0,
            });
            s.press("header", "tab", "Looper");
            s.button("REC").button("PLAY");
            until_phase(&mut s, Phase::Recording, 0.5);
            s.run_for(0.25);
            s.pad(0, 127, 0.05);
            s.run_for(1.5);
            s.button("STOP").run_for(0.1);
            assert_eq!(status(&s).phase, Phase::Playing);
            assert!(
                !s.said().iter().any(|l| l.contains("records the pads dry")),
                "{:?}",
                s.said()
            );
            let x = first_layer(&s);
            let rms = |a: usize, b: usize| {
                (x[a..b].iter().map(|v| v[0] * v[0]).sum::<f32>() / (b - a) as f32).sqrt()
            };
            let hit = x.iter().position(|v| v[0].abs() > 0.01).expect("the pad");
            // Played about 0.25 s into the bar (a period or two either
            // way for where the session hands the press over), not a
            // period later.
            let at = hit as f64 / 44_100.0;
            assert!((0.24..0.27).contains(&at), "the pad at {at} s");
            // The pad's sound is gone in 0.15 s; the 1/8 dotted echo comes
            // 0.375 s after it.
            let ms = |m: usize| hit + m * 441 / 10;
            let gap = rms(ms(250), ms(350));
            let echo = rms(ms(375), ms(475));
            assert!(
                echo > 0.01 && echo > 5.0 * gap,
                "echo {echo}, before it {gap}"
            );
        });
    }

    /// The steps the picked loop's UNDO would take back.
    fn steps(s: &Session) -> Vec<mpfree_looper::Step> {
        s.looper.looper.history().done().to_vec()
    }

    /// A loop of two layers, playing, on the Looper tab.
    fn two_layers() -> Session {
        let mut s = Session::new();
        s.make_sounds();
        s.press("header", "tab", "Looper");
        s.button("PLAY").run_for(0.3);
        record_a_loop(&mut s);
        s.button("OVERDUB");
        until_phase(&mut s, Phase::Overdubbing, 4.5);
        // A pad into it: an overdub of silence leaves no layer.
        s.pad(0, 120, 0.3).run_for(0.7);
        s.button("REC");
        until_phase(&mut s, Phase::Playing, 4.5);
        assert_eq!(status(&s).layers, 2);
        s
    }

    /// With the loop playing, each control does its edit (½, × 2,
    /// Reverse, a nudge dragged on the ruler, the seam fade on the picked
    /// loop's Q-Links, Merge), UNDO and SHIFT + UNDO take the last back and
    /// put it back, the undo history (the chips) goes back to an older
    /// step and forward again, and the trim grip trims.
    #[test]
    fn the_tools_the_undo_history_and_trim_work_while_the_loop_plays() {
        use mpfree_looper::{Edit, Step};
        on_big_stack(|| {
            let mut s = two_layers();
            let bar = 4 * PPQ as u64;
            // A control under the lanes by its id.
            let tool = |s: &mut Session, id: &str| {
                let c = s
                    .controls()
                    .into_iter()
                    .find(|c| c.widget == "leditor" && c.target.id == id)
                    .unwrap_or_else(|| panic!("no control {id}"));
                assert!(c.target.enabled, "{id} works while the loop plays");
                s.press_target(&c.target).run_for(0.2);
            };

            tool(&mut s, "half");
            assert_eq!(status(&s).ticks as u64, bar, "half: one bar");
            tool(&mut s, "double");
            assert_eq!(status(&s).ticks as u64, 2 * bar, "doubled: two again");
            tool(&mut s, "reverse");
            // The ruler dragged right a 1/16 note.
            tool(&mut s, "nudge");
            // Q-LINK twice: the picked loop's knobs; Q4 the seam fade.
            s.button("Q-LINK");
            s.button("Q-LINK");
            let seam = status(&s).seam;
            s.qlink(3, 10).run_for(0.2);
            assert_ne!(status(&s).seam, seam, "the seam fade changed");
            let edits: Vec<Step> = steps(&s).into_iter().skip(1).collect();
            assert!(
                matches!(
                    edits.as_slice(),
                    [
                        Step::Edit(Edit::Half),
                        Step::Edit(Edit::Double),
                        Step::Edit(Edit::Reverse),
                        Step::Edit(Edit::Nudge(1)),
                        Step::Edit(Edit::SeamFade(_)),
                        ..
                    ]
                ),
                "after the overdub, each edit is a step: {edits:?}"
            );
            // The knob's detents may land as more than one step: undo
            // back to before the first.
            while matches!(steps(&s).last(), Some(Step::Edit(Edit::SeamFade(_)))) {
                s.button("UNDO").run_for(0.2);
            }
            assert_eq!(status(&s).seam, seam);
            s.button_with("SHIFT", "UNDO").run_for(0.2);
            assert_ne!(status(&s).seam, seam, "SHIFT + UNDO put one back");

            // UNDO takes the last step back; SHIFT + UNDO puts it back.
            let n = steps(&s).len();
            s.button("UNDO").run_for(0.2);
            assert_eq!(steps(&s).len(), n - 1, "UNDO took a step back");
            s.button_with("SHIFT", "UNDO").run_for(0.2);
            assert_eq!(steps(&s).len(), n, "SHIFT + UNDO put it back");

            // Merge layers: the two become one, as a render while it plays.
            tool(&mut s, "merge");
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
            tool(&mut s, "history");
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
            tool(&mut s, "history");

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

    /// A finger on layer `k`'s lane, held, moved off it and onto `to`,
    /// then lifted there, as the musician drags it.
    fn drag_layer(s: &mut Session, k: usize, to: mpfree_ui::Rect) {
        let lane = at(s, "leditor", "layer", k).rect;
        let (x, y) = (lane.x + 30, lane.y + lane.h / 2);
        let (tx, ty) = (to.x + to.w / 2, to.y + to.h / 2);
        let mut acts: Vec<crate::gui::FromUi> = s
            .screen
            .touch(7, mpfree_ui::Touch::Down, x, y)
            .into_iter()
            .collect();
        // Held still until it can lift (a move sooner swipes the rows).
        s.run_for(f64::from(crate::looper_tab::LIFT_HOLD_S) + 0.05);
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

    /// A lane's head (its speaker) and S; Q1 turns loop 1's level, and on
    /// the picked loop's bank Q2 the picked layer's; a second loop from a
    /// tap on an empty ring (and its ring again to close it); a layer
    /// dragged onto it moves there and UNDO takes the move back; one
    /// dragged onto an empty loop becomes a loop of its own; a loop's
    /// speaker keeps it silent through STOP and PLAY; Clear empties the
    /// picked loop, and so does a ring held a second; x deletes layers, and
    /// x on the last one clears the loop too.
    #[test]
    fn layers_move_between_loops_and_speakers_clear_and_x_work() {
        on_big_stack(|| {
            let mut s = two_layers();
            // Layer 2's lane.
            let t = at(&mut s, "leditor", "layer-mute", 1);
            s.press_target(&t).run_for(0.1);
            assert!(layer(&s, 0, 1).muted, "the head muted layer 2");
            let t = at(&mut s, "leditor", "layer-mute", 1);
            s.press_target(&t).run_for(0.1);
            assert!(!layer(&s, 0, 1).muted);
            let t = at(&mut s, "leditor", "layer-solo", 1);
            s.press_target(&t).run_for(0.1);
            assert!(layer(&s, 0, 1).solo, "S soloed it");
            assert!(!layer(&s, 0, 0).audible, "and left layer 1 out");
            let t = at(&mut s, "leditor", "layer-solo", 1);
            s.press_target(&t).run_for(0.1);
            // Q1: loop 1's level.
            let level = s.looper.looper.loop_status(0).unwrap().level;
            s.qlink(0, -6).run_for(0.1);
            assert!(
                s.looper.looper.loop_status(0).unwrap().level < level,
                "Q-Link 1 turned loop 1"
            );
            // The picked loop's bank: Q2 turns the layer tapped.
            s.button("Q-LINK");
            s.button("Q-LINK");
            let t = at(&mut s, "leditor", "layer", 0);
            s.press_target(&t).run_for(0.1);
            let level = layer(&s, 0, 0).level;
            s.qlink(1, -6).run_for(0.1);
            assert!(layer(&s, 0, 0).level < level, "Q-Link 2 turned layer 1");

            // Loop 2's ring: picked and armed; its ring again closes it.
            let t = at(&mut s, "ldeck", "ring", 1);
            s.press_target(&t).run_for(0.1);
            assert_eq!(s.looper.looper.selected(), 1);
            assert_eq!(status(&s).phase, Phase::Armed, "the ring armed it");
            until_phase(&mut s, Phase::Recording, 2.5);
            s.run_for(3.0);
            let t = at(&mut s, "ldeck", "ring", 1);
            s.press_target(&t);
            assert!(status(&s).ending, "the ring again: it ends on the bar line");
            until_phase(&mut s, Phase::Playing, 1.5);
            assert_eq!(layers_of(&s, 1), 1);

            // Back on loop 1, its layer 2 dragged onto loop 2: it moves.
            let t = at(&mut s, "ldeck", "loop", 0);
            s.press_target(&t).run_for(0.1);
            assert_eq!(s.looper.looper.selected(), 0);
            let two = at(&mut s, "ldeck", "loop", 1).rect;
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
            let t = at(&mut s, "ldeck", "loop", 0);
            s.press_target(&t).run_for(0.1);

            // Onto loop 3, empty: a loop of its own.
            let three = at(&mut s, "ldeck", "ring", 2).rect;
            drag_layer(&mut s, 1, three);
            assert_eq!(layers_of(&s, 0), 1);
            assert_eq!(layers_of(&s, 2), 1, "loop 3 has it");
            assert_eq!(s.looper.looper.selected(), 2);

            // Loop 1's speaker: silent, and it stays so through STOP and
            // PLAY; tapped again it plays.
            let t = at(&mut s, "ldeck", "loop-mute", 0);
            s.press_target(&t).run_for(0.1);
            let muted = |s: &Session| s.looper.looper.loop_status(0).unwrap().muted;
            assert!(muted(&s), "loop 1 is silent");
            s.button("STOP").run_for(0.3);
            s.button("PLAY").run_for(0.3);
            assert!(muted(&s), "and stays out after STOP and PLAY");
            assert_ne!(layers_of(&s, 0), 0, "but it is kept");
            let t = at(&mut s, "ldeck", "loop-mute", 0);
            s.press_target(&t).run_for(0.1);
            assert!(!muted(&s), "back in");

            // Loop 2 picked and cleared, first pass and all.
            let t = at(&mut s, "ldeck", "loop", 1);
            s.press_target(&t).run_for(0.1);
            s.press("leditor", "clear", "Clear").run_for(0.3);
            assert_eq!(s.looper.looper.loop_status(1).unwrap().phase, Phase::Empty);
            // Loop 3's ring held a second: cleared.
            let ring = at(&mut s, "ldeck", "ring", 2).rect;
            let (x, y) = (ring.x + ring.w / 2, ring.y + ring.h / 2);
            let mut acts: Vec<crate::gui::FromUi> = s
                .screen
                .touch(7, mpfree_ui::Touch::Down, x, y)
                .into_iter()
                .collect();
            acts.extend(s.screen.take_actions());
            for a in acts {
                s.route(a);
            }
            s.run_for(f64::from(crate::looper_deck::CLEAR_HOLD_S) + 0.2);
            let up = s.screen.touch(7, mpfree_ui::Touch::Up, x, y);
            for a in up.into_iter().chain(s.screen.take_actions()) {
                s.route(a);
            }
            s.run_for(0.3);
            assert_eq!(
                s.looper.looper.loop_status(2).unwrap().phase,
                Phase::Empty,
                "the held ring cleared loop 3"
            );

            // Loop 1: an overdub, then x on each layer down to the last;
            // x on that one clears the loop.
            let t = at(&mut s, "ldeck", "loop", 0);
            s.press_target(&t).run_for(0.1);
            s.button("OVERDUB");
            until_phase(&mut s, Phase::Overdubbing, 4.5);
            s.pad(0, 120, 0.3).run_for(0.7);
            s.button("REC");
            until_phase(&mut s, Phase::Playing, 4.5);
            assert_eq!(layers_of(&s, 0), 2);
            // The x is on the picked layer's lane: a tap picks it.
            let t = at(&mut s, "leditor", "layer", 1);
            s.press_target(&t).run_for(0.1);
            let t = at(&mut s, "leditor", "layer-delete", 1);
            s.press_target(&t).run_for(0.3);
            assert_eq!(layers_of(&s, 0), 1, "x deleted layer 2");
            let t = at(&mut s, "leditor", "layer", 0);
            s.press_target(&t).run_for(0.1);
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

    /// The pads as loop keys (the Looper as an instrument, step 2):
    /// SHIFT + SAMPLER again on the Looper tab. Pad 13 is Loop 1's key: a hit
    /// records from the next bar line, a hit ends the first pass on the
    /// bar line, a hit overdubs, a hit plays on (nothing was played into
    /// the overdub, so it leaves no layer). Pad 5, two rows under it,
    /// mutes it and lets it play. Pad 14 is Loop 2's key, and a hit picks
    /// it. Held a second, a key clears its loop. The deck's Pads switch
    /// gives the pads back, and on the Sample tab they play the kit.
    #[test]
    fn the_pads_as_loop_keys_record_overdub_mute_and_clear() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("header", "tab", "Looper");
            s.button_with("SHIFT", "SAMPLER");
            assert!(s.app.pads_are_loop_keys());
            assert!(s.hint().starts_with("Pads: loop keys"), "{}", s.hint());
            s.button("PLAY").run_for(0.3);
            let loop1 = |s: &Session| s.looper.looper.loop_status(0).unwrap();
            // Pad 13: Loop 1 waits for the bar line, records, and a hit
            // ends it there.
            s.pad(12, 100, 0.05);
            assert_eq!(loop1(&s).phase, Phase::Armed);
            until_phase(&mut s, Phase::Recording, 2.5);
            s.run_for(2.0);
            s.pad(12, 100, 0.05);
            assert!(
                loop1(&s).ending,
                "a hit ends the first pass on the bar line"
            );
            until_phase(&mut s, Phase::Playing, 2.5);
            // A hit overdubs, and another plays on. Nothing was played into
            // it (the pads are keys), so it leaves no layer.
            s.pad(12, 100, 0.05);
            let end = s.t() + 2.5;
            while loop1(&s).phase != Phase::Overdubbing && s.t() < end {
                s.run_periods(1);
            }
            assert_eq!(loop1(&s).phase, Phase::Overdubbing);
            s.run_for(0.5);
            s.pad(12, 100, 0.05);
            s.run_for(0.2);
            assert_eq!(loop1(&s).phase, Phase::Playing);
            assert_eq!(loop1(&s).layers, 1, "an overdub of silence leaves none");
            // Pad 5 mutes Loop 1 and lets it play again.
            s.pad(4, 100, 0.05);
            assert!(loop1(&s).muted, "pad 5 muted Loop 1");
            s.pad(4, 100, 0.05);
            assert!(!loop1(&s).muted);
            // Pad 14 is Loop 2's key: it picks Loop 2 and arms it; a hit
            // again lets it go.
            s.pad(13, 100, 0.05);
            assert_eq!(s.looper.looper.selected(), 1);
            assert_eq!(s.looper.looper.loop_status(1).unwrap().phase, Phase::Armed);
            s.pad(13, 100, 0.05);
            assert_eq!(s.looper.looper.loop_status(1).unwrap().phase, Phase::Empty);
            // Pad 13 held a second clears Loop 1; a shorter hold doesn't.
            s.pad(12, 100, 1.3);
            s.run_periods(4);
            assert_eq!(loop1(&s).phase, Phase::Empty, "the held key cleared it");
            assert!(s
                .said()
                .iter()
                .any(|l| l.contains("Loop 1 cleared (its key held)")));
            // The deck's Pads switch gives the pads back.
            s.press("ldeck", "pads-keys", "Pads as loop keys (on)");
            assert!(!s.app.pads_are_loop_keys());
            s.press("ldeck", "pads-keys", "Pads as loop keys");
            assert!(s.app.pads_are_loop_keys());
            // On the Sample tab the pads play the kit again.
            s.press("header", "tab", "Sample");
            assert!(!s.app.pads_are_loop_keys());
            s.button("STOP");
        });
    }

    /// Overdubs: Replace (step 3b): an overdub with a pad in it takes the
    /// place of what was there for as long as it records; its lane says
    /// "replace", and UNDO takes it away.
    #[test]
    fn a_replace_overdub_takes_the_place_of_what_was_there() {
        on_big_stack(|| {
            let mut s = two_layers();
            s.press("leditor", "overdubs", "Replace").run_for(0.1);
            assert_eq!(status(&s).overdubs, Overdubs::Replace);
            s.button("OVERDUB");
            until_phase(&mut s, Phase::Overdubbing, 4.5);
            s.pad(3, 120, 0.3).run_for(0.5);
            s.button("REC");
            until_phase(&mut s, Phase::Playing, 4.5);
            s.run_for(0.2);
            assert_eq!(status(&s).layers, 3);
            assert!(layer(&s, 0, 2).replace, "the new layer replaces");
            assert!(!layer(&s, 0, 1).replace);
            let e = s
                .screen
                .ui
                .get::<crate::looper_tab::LoopEditor>(crate::looper_tab::EDITOR)
                .expect("the editor");
            assert!(
                e.layers.iter().any(|l| l.tag.ends_with("· replace")),
                "a lane says replace"
            );
            s.button("UNDO").run_for(0.3);
            assert_eq!(status(&s).layers, 2, "UNDO took it away");
            s.button("STOP");
        });
    }

    /// A finger on loop `n`'s ring for `seconds`, then lifted.
    fn hold_ring(s: &mut Session, n: usize, seconds: f64) {
        let ring = at(s, "ldeck", "ring", n).rect;
        let (x, y) = (ring.x + ring.w / 2, ring.y + ring.h / 2);
        let down = s.screen.touch(7, mpfree_ui::Touch::Down, x, y);
        for a in down.into_iter().chain(s.screen.take_actions()) {
            s.route(a);
        }
        s.run_for(seconds);
        let up = s.screen.touch(7, mpfree_ui::Touch::Up, x, y);
        for a in up.into_iter().chain(s.screen.take_actions()) {
            s.route(a);
        }
        s.run_for(0.3);
    }

    /// The forgiving looper (the Looper as an instrument, step 3a):
    /// Overdubs: Once records one pass and plays on; a loop's S solos it
    /// and the other loops go quiet in their places, as SHIFT + its mute
    /// pad does with the pads as loop keys; a loop cleared (its ring held,
    /// ERASE + its ring, its key held) comes back with UNDO, as it was
    /// before the press that started an overdub.
    #[test]
    fn once_solo_and_a_clear_that_undo_brings_back() {
        on_big_stack(|| {
            let mut s = two_layers();
            let l = |s: &Session, n: usize| s.looper.looper.loop_status(n).unwrap();
            // Overdubs: Once. A tap on the ring records one pass with a
            // pad in it, then the loop plays on by itself.
            s.press("leditor", "overdubs", "Once").run_for(0.1);
            assert_eq!(status(&s).overdubs, Overdubs::Once);
            let t = at(&mut s, "ldeck", "ring", 0);
            s.press_target(&t);
            until_phase(&mut s, Phase::Overdubbing, 4.5);
            s.pad(1, 120, 0.3);
            until_phase(&mut s, Phase::Playing, 5.0);
            assert_eq!(status(&s).layers, 3, "one pass, one layer");

            // Loop 2 from its ring, with a pad in it.
            let t = at(&mut s, "ldeck", "ring", 1);
            s.press_target(&t).run_for(0.1);
            until_phase(&mut s, Phase::Recording, 2.5);
            s.pad(2, 120, 0.3).run_for(2.7);
            let t = at(&mut s, "ldeck", "ring", 1);
            s.press_target(&t);
            until_phase(&mut s, Phase::Playing, 2.5);

            // Loop 2's S: Loop 1 goes quiet in its place; S again lets go.
            let t = at(&mut s, "ldeck", "loop-solo", 1);
            s.press_target(&t).run_for(0.1);
            assert!(l(&s, 1).solo && l(&s, 0).solo_out, "loop 2 soloed");
            assert_eq!(l(&s, 0).phase, Phase::Playing, "loop 1 keeps its place");
            let t = at(&mut s, "ldeck", "loop-solo", 1);
            s.press_target(&t).run_for(0.1);
            assert!(!l(&s, 1).solo && !l(&s, 0).solo_out);

            // Loop 1's ring held a second: its press started an overdub,
            // the hold clears it. UNDO brings it back as it was.
            hold_ring(&mut s, 0, f64::from(crate::looper_deck::CLEAR_HOLD_S) + 0.2);
            assert_eq!((l(&s, 0).phase, l(&s, 0).kept), (Phase::Empty, true));
            assert!(s.said().iter().any(|l| l.contains("UNDO brings it back")));
            s.button("UNDO").run_for(0.3);
            assert_eq!(
                (l(&s, 0).phase, l(&s, 0).layers, l(&s, 0).kept),
                (Phase::Playing, 3, false),
                "UNDO brought loop 1 back, all three layers"
            );

            // ERASE + Loop 2's ring: cleared at once, and UNDO again.
            s.button_edge("ERASE", true);
            let t = at(&mut s, "ldeck", "ring", 1);
            s.press_target(&t).run_for(0.3);
            s.button_edge("ERASE", false);
            assert_eq!(l(&s, 1).phase, Phase::Empty, "ERASE + the ring cleared it");
            assert_eq!(s.looper.looper.selected(), 1);
            s.button("UNDO").run_for(0.3);
            assert_eq!(l(&s, 1).phase, Phase::Playing, "and UNDO brought it back");

            // The pads as loop keys: SHIFT + pad 6 (Loop 2's mute) solos
            // Loop 2, and again lets go.
            s.button_with("SHIFT", "SAMPLER");
            assert!(s.app.pads_are_loop_keys());
            s.button_edge("SHIFT", true);
            s.pad(5, 100, 0.05);
            s.button_edge("SHIFT", false);
            assert!(l(&s, 1).solo, "SHIFT + its mute pad soloed loop 2");
            assert!(!l(&s, 1).muted);
            s.button_edge("SHIFT", true);
            s.pad(5, 100, 0.05);
            s.button_edge("SHIFT", false);
            assert!(!l(&s, 1).solo);
            // Pad 13 held: Loop 1 cleared; UNDO brings it back.
            s.pad(12, 100, 1.3);
            s.run_periods(4);
            assert_eq!(l(&s, 0).phase, Phase::Empty, "the held key cleared it");
            s.button("UNDO").run_for(0.3);
            assert_eq!((l(&s, 0).phase, l(&s, 0).layers), (Phase::Playing, 3));
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
            .chain_state(&s.mix, &|t| {
                crate::modpanels::NotesSource::of(&s.app, &s.time, t)
            })
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
mod step_seq {
    //! STEP SEQ: the pads as steps (sequencer decision 2, Jay 16 Nov).

    use super::super::{on_big_stack, Session};
    use crate::app::{MuteKeys, View, LIGHT_OFF, LIGHT_ON};
    use mpfree_controller::mpc_one::button;

    /// The pad played last is the sound; STEP SEQ makes the pads its
    /// steps on Beats and on any tab; a tap adds a note (making the clip),
    /// a lit step comes off on the release, PAD BANK B is page 2, ERASE +
    /// a step clears it, TRACK MUTE takes the pads and STEP SEQ gives
    /// them back.
    #[test]
    fn step_seq_makes_the_pads_the_steps_of_the_sound_played_last() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.run_for(0.1);
            s.pad(2, 100, 0.05);
            s.press("header", "tab", "Looper");
            s.button("STEP SEQ");
            assert!(s.app.pads_are_steps());
            assert_eq!(s.view(), View::Play, "STEP SEQ shows Beats");
            assert_eq!(s.sent_lights[button::STEP_SEQ as usize], LIGHT_ON);
            assert!(s
                .lines
                .iter()
                .any(|l| l == "STEP SEQ: the pads are pad A3's steps, page 1"));
            let row = |s: &Session| -> Vec<bool> {
                let d = s.time.steps();
                d.clip
                    .map_or(vec![], |c| c.row(2).iter().map(Option::is_some).collect())
            };
            // Pad 1 is step 1: a one-bar clip with pad 3's note there.
            s.pad(0, 100, 0.05);
            assert_eq!(row(&s).len(), 16);
            assert!(row(&s)[0]);
            // On another tab too; a lit step comes off on the release.
            s.press("header", "tab", "Looper");
            s.pad(4, 90, 0.05);
            assert!(row(&s)[4]);
            s.pad(4, 90, 0.05);
            assert!(!row(&s)[4]);
            assert_eq!(
                s.time.steps().clip.unwrap().row(0).iter().flatten().count(),
                0,
                "pad 1's own row is untouched"
            );
            // PAD BANK B: page 2, past this clip's end; nothing changes.
            s.button("PAD BANK B/F");
            assert_eq!(s.app.step_pads().unwrap().page, 1);
            s.pad(0, 100, 0.05);
            assert_eq!(row(&s).len(), 16);
            s.button("PAD BANK A/E");
            // ERASE + a step: cleared.
            s.button_edge("ERASE", true);
            s.pad(0, 100, 0.05);
            assert!(s.hint().contains("UNDO brings it back"), "{}", s.hint());
            s.button_edge("ERASE", false);
            assert!(!row(&s)[0]);
            // TRACK MUTE takes the pads; STEP SEQ takes them back.
            s.button("TRACK MUTE");
            assert_eq!(s.app.mute_keys(), MuteKeys::Tracks);
            assert!(!s.app.pads_are_steps());
            s.button("STEP SEQ");
            assert_eq!(s.app.mute_keys(), MuteKeys::Off);
            assert!(s.app.pads_are_steps());
            s.button("STEP SEQ");
            assert!(s.app.step_pads().is_none(), "the pads play again");
            assert_eq!(s.sent_lights[button::STEP_SEQ as usize], LIGHT_OFF);
        });
    }

    /// STEP SEQ on the synth (sequencer S1, sixth part): the pads are the
    /// whole line's steps. STEP SEQ held, pads 1 and 5 played together
    /// are the pen; a step tapped gets that chord and a lit one comes off
    /// whole; Beats' lane on the pads is the line, and the chip names the
    /// pen; played, the clip plays the chord.
    #[test]
    fn step_seq_on_the_synth_puts_the_pen_on_the_line() {
        use crate::time::LINE;
        on_big_stack(|| {
            let mut s = Session::new();
            s.run_for(0.1);
            s.pick_synth();
            assert!(s.synth_on, "the pads play the synth");
            s.button("STEP SEQ");
            assert!(s.app.step_pads().is_some_and(|p| p.line()));
            assert_eq!(s.view(), View::Play, "STEP SEQ shows Beats");
            // The pen: STEP SEQ held, pads 1 and 5 together.
            s.button_edge("STEP SEQ", true);
            assert!(s.hint().contains("hold a chord"), "{}", s.hint());
            s.pad_edge(0, 100, true);
            s.pad_edge(4, 100, true);
            s.run_periods(2);
            s.pad_edge(0, 0, false);
            s.pad_edge(4, 0, false);
            s.button_edge("STEP SEQ", false);
            assert_eq!(s.app.step_pads().unwrap().pen.pads(), [0, 4]);
            assert!(s.app.pads_are_steps());
            let chip = s
                .controls()
                .into_iter()
                .find(|c| c.widget == "header" && c.target.label.starts_with("Pads: steps"))
                .map(|c| c.target.label);
            assert!(
                chip.as_deref()
                    .is_some_and(|c| c.matches(" · ").count() == 2 && c.contains(" + ")),
                "{chip:?}"
            );
            // Steps 1 and 9 get the chord.
            s.pad(0, 100, 0.05);
            s.pad(8, 100, 0.05);
            let rows = |s: &Session| -> Vec<Vec<usize>> {
                let c = s.time.steps().clip.expect("a clip");
                [0u8, 4, LINE]
                    .iter()
                    .map(|&p| {
                        c.row(p)
                            .iter()
                            .enumerate()
                            .filter_map(|(i, st)| st.map(|_| i + 1))
                            .collect()
                    })
                    .collect()
            };
            assert_eq!(rows(&s), [vec![1, 9], vec![1, 9], vec![1, 9]]);
            let g = s
                .screen
                .ui
                .get::<crate::play_widgets::StepGrid>(crate::play_page::STEPS)
                .expect("the step grid");
            let lane = &g.lanes[0];
            assert_eq!((lane.pad, lane.name.as_str()), (usize::from(LINE), "Line"));
            assert!(lane.steps[0] > 0 && lane.steps[8] > 0);
            // Played: both pads of the chord sound on step 1.
            s.hits.on.clear();
            s.button("PLAY");
            s.run_for(0.3);
            s.button("STOP");
            for pad in [0, 4] {
                assert!(
                    s.hits.on.iter().any(|h| h.0 == pad),
                    "pad {pad}: {:?}",
                    s.hits.on
                );
            }
            // Step 9 pressed alone: off whole; UNDO brings it back.
            s.pad(8, 100, 0.05);
            assert_eq!(rows(&s), [vec![1], vec![1], vec![1]]);
            s.button("UNDO");
            assert_eq!(rows(&s), [vec![1, 9], vec![1, 9], vec![1, 9]]);
            // A tap on the line's lane on Beats puts the pen there too.
            s.press("steps", "step", "Line step 13");
            assert_eq!(rows(&s)[2], [1, 9, 13]);
            // Held, Q-LINK twice: the synth's Sound bank; a turn locks the
            // whole step.
            s.pad_edge(0, 100, true);
            s.run_periods(4);
            s.button("Q-LINK");
            s.button("Q-LINK");
            let title = s
                .screen
                .ui
                .get::<mpfree_ui::qlink::QLinkColumn>(crate::play_page::QLINKS)
                .map(|q| q.title.clone())
                .unwrap_or_default();
            assert!(title.ends_with("Sound"), "{title}");
            s.qlink(0, 3);
            s.pad_edge(0, 0, false);
            s.run_periods(4);
            let c = s.time.steps().clip.unwrap();
            assert!(
                c.lock_values.iter().any(|l| (l.0, l.1) == (LINE, 0)),
                "{:?}",
                c.lock_values
            );
            assert_eq!(c.locks(LINE)[0], 1);
            assert_eq!(rows(&s)[2], [1, 9, 13], "the step stays");
        });
    }

    /// A synth step's notes (sequencer S1, seventh part): a line step held,
    /// Q-Link 1 is its Note and moves it up the pads; a keyboard's keys
    /// played while it is held go on it as a chord, in place of what it
    /// held, one UNDO for the chord; the inspector names the step's notes;
    /// a key no pad plays says so.
    #[test]
    fn on_the_synth_the_note_knob_and_the_keys_set_a_steps_notes() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.run_for(0.1);
            s.pick_synth();
            s.button("STEP SEQ");
            // The pen: pad 1 (C3), then steps 1 and 5.
            s.button_edge("STEP SEQ", true);
            s.pad(0, 100, 0.05);
            s.button_edge("STEP SEQ", false);
            s.pad(0, 100, 0.05);
            s.pad(4, 100, 0.05);
            let pads = |s: &Session, step: usize| -> Vec<u8> {
                let c = s.time.steps().clip.expect("a clip");
                (0..16u8)
                    .filter(|&p| c.row(p).get(step).copied().flatten().is_some())
                    .collect()
            };
            assert_eq!(pads(&s, 0), [0]);
            let cards = |s: &Session| -> Vec<(String, String)> {
                s.screen
                    .ui
                    .get::<mpfree_ui::qlink::QLinkColumn>(crate::play_page::QLINKS)
                    .map(|q| {
                        q.cards
                            .iter()
                            .map(|c| (c.name.clone(), c.value.clone()))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            // Step 1 held: Q-Link 1 is its note; a detent is a pad up.
            s.pad_edge(0, 100, true);
            s.run_periods(4);
            assert!(s.hint().contains("Q-Links: note, velocity"), "{}", s.hint());
            let c = cards(&s);
            assert_eq!(c[0].0, "Note", "{c:?}");
            assert_eq!(c[1].0, "Velocity", "{c:?}");
            let before = c[0].1.clone();
            s.qlink(0, 1);
            assert_eq!(pads(&s, 0), [1]);
            assert_ne!(cards(&s)[0].1, before, "the card follows");
            // A keyboard's keys, E3 and G3 together: the step's chord.
            let (e3, g3) = (52, 55);
            assert_eq!(s.app.pad_of_key(e3), Some(4));
            s.key_edge(e3, Some(90));
            s.key_edge(g3, Some(110));
            s.key_edge(e3, None);
            s.key_edge(g3, None);
            assert_eq!(pads(&s, 0), [4, 7]);
            assert_eq!(cards(&s)[0].0, "Chord");
            // A key no pad plays: said, and nothing changes.
            s.key_edge(20, Some(100));
            s.key_edge(20, None);
            assert!(
                s.hint().contains("isn't a note of the pads"),
                "{}",
                s.hint()
            );
            s.pad_edge(0, 0, false);
            s.run_periods(4);
            assert_eq!(pads(&s, 0), [4, 7], "the keys kept the step");
            let c = s.time.steps().clip.unwrap();
            assert_eq!(c.row(4)[0].map(|st| st.velocity), Some(90));
            assert_eq!(c.row(7)[0].map(|st| st.velocity), Some(110));
            // Picked on Beats, the inspector names its notes.
            s.press("steps", "step", "Line step 1");
            let title = s
                .screen
                .ui
                .get::<crate::play_widgets::StepInspector>(crate::play_page::INSPECTOR)
                .map(|i| i.inspect.title.clone())
                .unwrap_or_default();
            assert!(
                title.starts_with("Step 1 · ") && title.contains(" + "),
                "{title}"
            );
            // UNDO: the chord goes (one take for both keys), then the turn.
            s.button("UNDO");
            assert_eq!(pads(&s, 0), [1]);
            s.button("UNDO");
            assert_eq!(pads(&s, 0), [0]);
            assert_eq!(pads(&s, 4), [0], "step 5 as it was");
            // Keys with no step held only play.
            s.key_edge(e3, Some(100));
            s.key_edge(e3, None);
            assert_eq!(pads(&s, 8), Vec::<u8>::new());
        });
    }

    /// Step pads held (sequencer S1): Q-Link 1 turned on the controller
    /// sets the step held, which stays on its release; a later step tapped
    /// while one is held is a tie; COPY + a step, then others, copies it,
    /// and UNDO takes a copy back.
    #[test]
    fn held_step_pads_set_tie_and_copy_steps() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.run_for(0.1);
            s.pad(2, 100, 0.05);
            s.button("STEP SEQ");
            // Pad 3's steps: (step, velocity, length).
            let row = |s: &Session| -> Vec<(usize, u8, i64)> {
                let d = s.time.steps();
                d.clip.map_or(vec![], |c| {
                    c.row(2)
                        .iter()
                        .enumerate()
                        .filter_map(|(i, st)| st.map(|st| (i, st.velocity, st.len)))
                        .collect()
                })
            };
            s.pad(0, 100, 0.05);
            s.pad(4, 100, 0.05);
            let v = row(&s)[0].1;
            // Hold step 1 and turn Q-Link 1: louder, and it stays.
            s.pad_edge(0, 100, true);
            s.run_periods(4);
            assert!(s.hint().starts_with("Step 1 held"), "{}", s.hint());
            s.qlink(0, 3);
            s.pad_edge(0, 0, false);
            s.run_periods(4);
            assert_eq!(row(&s)[0], (0, v + 6, 240), "{:?}", row(&s));
            // Hold step 1 and tap step 3: it lasts to the end of step 3.
            s.pad_edge(0, 100, true);
            s.run_periods(4);
            s.pad(2, 100, 0.05);
            assert!(
                s.hint().contains("lasts to the end of step 3"),
                "{}",
                s.hint()
            );
            s.pad_edge(0, 0, false);
            s.run_periods(4);
            assert_eq!(row(&s)[0], (0, v + 6, 720));
            assert_eq!(row(&s).len(), 2, "the tie's end stays empty");
            // COPY + step 5, then steps 9 and 13.
            s.button_edge("COPY", true);
            assert!(s.app.step_pads().is_some_and(|p| p.copying));
            for pad in [4, 8, 12] {
                s.pad(pad, 100, 0.05);
            }
            s.button_edge("COPY", false);
            let steps: Vec<usize> = row(&s).iter().map(|r| r.0).collect();
            assert_eq!(steps, [0, 4, 8, 12]);
            s.button("UNDO");
            let steps: Vec<usize> = row(&s).iter().map(|r| r.0).collect();
            assert_eq!(steps, [0, 4, 8], "UNDO: the last copy goes");
        });
    }

    /// Locks on held steps (sequencer S1 locks): Q-LINK steps to the
    /// Sound bank, a turn there locks the kit pad's tune on the step
    /// held, and played, the lock reaches the kit before the step's note;
    /// SHIFT + an empty step takes a lock with no note; ERASE pressed while
    /// a step is held takes its locks off, and UNDO brings them back.
    #[test]
    fn held_step_pads_lock_the_kit_and_erase_takes_the_locks_off() {
        use mpfree_play::mpfree_param::{OnTrack, Owner, RelAddr};
        on_big_stack(|| {
            let mut s = Session::new();
            s.run_for(0.1);
            // Pad 5: the kit pad with a sample on the Sample tab.
            s.pad(4, 100, 0.05);
            s.button("STEP SEQ");
            s.pad(0, 100, 0.05);
            s.pad(4, 100, 0.05);
            let tune = RelAddr::new(OnTrack::Pad(4), 2);
            let lock = |s: &Session, step: usize| {
                s.time
                    .steps()
                    .clip
                    .and_then(|c| c.lock_value(4, step, tune))
            };
            // Hold step 1; Q-LINK twice: the Sound bank. Turn its tune.
            s.pad_edge(0, 100, true);
            s.run_periods(4);
            s.button("Q-LINK");
            s.button("Q-LINK");
            assert!(s.hint().contains("Q-Links: lock the sound"), "{}", s.hint());
            s.qlink(0, 3);
            s.pad_edge(0, 0, false);
            s.run_periods(4);
            let locked = lock(&s, 0).expect("a lock on step 1");
            assert!(locked > 0.0, "tuned up: {locked}");
            assert_eq!(
                s.time.steps().clip.unwrap().row(4).iter().flatten().count(),
                2,
                "the step stays"
            );
            // SHIFT + step 9 (empty): a lock with no note.
            s.button_edge("SHIFT", true);
            s.pad_edge(8, 100, true);
            s.run_periods(4);
            s.button_edge("SHIFT", false);
            assert!(s.hint().contains("Q-Links: lock the sound"), "{}", s.hint());
            s.qlink(0, -2);
            s.pad_edge(8, 0, false);
            s.run_periods(4);
            assert!(lock(&s, 8).is_some_and(|v| v < 0.0), "{:?}", lock(&s, 8));
            assert_eq!(
                s.time.steps().clip.unwrap().row(4).iter().flatten().count(),
                2,
                "no note on step 9"
            );
            // Played: the kit takes the lock, before step 1's note.
            s.hits.locked.clear();
            s.button("PLAY");
            s.run_for(2.2);
            s.button("STOP");
            s.run_periods(4);
            let at = |owner| matches!(owner, Owner::Pad(_, 4));
            assert!(
                s.hits
                    .locked
                    .iter()
                    .any(|(a, v)| at(a.owner) && a.param == 2 && *v == Some(locked)),
                "{:?}",
                s.hits.locked
            );
            // ERASE while step 1 is held: its locks off, the note stays.
            s.pad_edge(0, 100, true);
            s.run_periods(4);
            s.button("ERASE");
            assert!(
                s.hint().contains("the locks of step 1 are off"),
                "{}",
                s.hint()
            );
            s.pad_edge(0, 0, false);
            s.run_periods(4);
            assert_eq!(lock(&s, 0), None);
            assert!(lock(&s, 8).is_some(), "step 9 was not held");
            assert_eq!(
                s.time.steps().clip.unwrap().row(4).iter().flatten().count(),
                2,
                "its note stays"
            );
            s.button("UNDO");
            assert_eq!(lock(&s, 0), Some(locked), "UNDO brings it back");
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

    /// STEP SEQ's locks (`^steps: step \d+ of pad \d+: lock `,
    /// `^ERASE: the locks of step \d+ are off`, `^steps: step \d+ of pad
    /// \d+ held for a lock with no note`): one lock line a turn, however
    /// many detents.
    #[test]
    fn step_locks_write_their_lines() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.run_for(0.1);
            s.pad(4, 100, 0.05);
            s.button("STEP SEQ");
            s.pad(0, 100, 0.05);
            s.pad_edge(0, 100, true);
            s.run_periods(4);
            s.button("Q-LINK");
            s.button("Q-LINK");
            let n = s.lines.len();
            s.qlink(0, 2);
            s.qlink(0, 2);
            let l = wrote(&s, n, "steps: step 1 of pad 5: lock ", "");
            assert!(l.starts_with("steps: step 1 of pad 5: lock pad4.2 "), "{l}");
            let locks = since(&s, n)
                .iter()
                .filter(|l| l.contains(": lock "))
                .count();
            assert_eq!(locks, 1, "one line a turn");
            let n = s.lines.len();
            s.button("ERASE");
            wrote(&s, n, "ERASE: the locks of step 1 are off", "");
            s.pad_edge(0, 0, false);
            s.run_periods(4);
            let n = s.lines.len();
            s.button_edge("SHIFT", true);
            s.pad(6, 100, 0.05);
            s.button_edge("SHIFT", false);
            wrote(
                &s,
                n,
                "steps: step 7 of pad 5 held for a lock with no note",
                "",
            );
        });
    }

    /// STEP SEQ on the synth (`^STEP SEQ: the pads are the line's steps`,
    /// `step \d+ of the line: pads 1, 5 on$`, `^steps: step 5 of the line
    /// erased$`), as the `step-synth` check has them do it.
    #[test]
    fn step_synth_writes_its_lines() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.run_for(0.1);
            s.pick_synth();
            let n = s.lines.len();
            s.button("STEP SEQ");
            wrote(&s, n, "STEP SEQ: the pads are the line's steps", "");
            s.button_edge("STEP SEQ", true);
            s.pad_edge(0, 100, true);
            s.pad_edge(4, 100, true);
            s.run_periods(2);
            s.pad_edge(0, 0, false);
            s.pad_edge(4, 0, false);
            s.button_edge("STEP SEQ", false);
            let n = s.lines.len();
            for pad in [0, 4, 8, 12] {
                s.pad(pad, 100, 0.05);
            }
            let l = wrote(&s, n, "steps: ", "of the line: pads 1, 5 on");
            assert!(l.ends_with("step 1 of the line: pads 1, 5 on"), "{l}");
            wrote(&s, n, "steps: step 13 of the line: pads 1, 5 on", "");
            let n = s.lines.len();
            s.pad(4, 100, 0.05);
            let l = wrote(&s, n, "steps: step 5 of the line erased", "");
            assert_eq!(l, "steps: step 5 of the line erased");
        });
    }

    /// A synth step's notes (`^steps: step 1 of the line: lowest note on
    /// pad \d+$`, `^steps: step 1 of the line: pads [\d, ]+ from the keys
    /// on$`), as the `step-notes` and `step-keys` checks have them done:
    /// a line a detent of Q-Link 1, and a line a key.
    #[test]
    fn step_notes_and_keys_write_their_lines() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.run_for(0.1);
            s.pick_synth();
            s.button("STEP SEQ");
            s.button_edge("STEP SEQ", true);
            s.pad(0, 100, 0.05);
            s.button_edge("STEP SEQ", false);
            s.pad(0, 100, 0.05);
            s.pad(4, 100, 0.05);
            // Q-Link 1 two clicks right on step 1 held: C3 to D3.
            let n = s.lines.len();
            s.pad_edge(0, 100, true);
            s.run_periods(4);
            s.qlink(0, 1);
            s.qlink(0, 1);
            s.pad_edge(0, 0, false);
            s.run_periods(4);
            let l = wrote(&s, n, "steps: step 1 of the line: lowest note on pad 3", "");
            assert_eq!(l, "steps: step 1 of the line: lowest note on pad 3");
            // E3 and G3 on the keys, step 1 held.
            let n = s.lines.len();
            s.pad_edge(0, 100, true);
            s.run_periods(4);
            s.key_edge(52, Some(100));
            s.key_edge(55, Some(100));
            s.key_edge(52, None);
            s.key_edge(55, None);
            s.pad_edge(0, 0, false);
            s.run_periods(4);
            // A line a key: the first key alone, then both.
            wrote(
                &s,
                n,
                "steps: step 1 of the line: pads 5 from the keys on",
                "",
            );
            let l = wrote(
                &s,
                n,
                "steps: step 1 of the line: pads ",
                "5, 8 from the keys on",
            );
            assert_eq!(l, "steps: step 1 of the line: pads 5, 8 from the keys on");
        });
    }

    /// REC records what is armed (`^rec: ready, PLAY records `, `^time:
    /// transport: Record, `, `^Undid take `, `^rec: the loop armed$`,
    /// `^looper: Record: armed`), as the `rec-arm` check has them done: on
    /// the home view REC readies the notes and PLAY records them, played
    /// back on PLAY, UNDO takes the take, and the Loop view arms the loop
    /// for REC.
    /// The loop-tracks check (`app_checks.py`): a loop recorded on Play's
    /// Loop view lands on Loops A, row 2's number stops it on its bar line,
    /// and its cell brings it back on one.
    #[test]
    fn loop_tracks_write_their_lines() {
        use mpfree_looper::Phase;
        on_big_stack(|| {
            let mut s = Session::new();
            s.run_for(0.1);
            s.press("play-transport", "view", "Loop").settle();
            let n = s.lines.len();
            s.button("REC").button("PLAY");
            s.run_for(0.5);
            s.pad(0, 100, 0.05);
            s.run_for(2.0);
            s.button("REC");
            s.run_for(2.5);
            assert_eq!(s.looper.phase(), Phase::Playing);
            wrote(&s, n, "time: Loop 1 on Loops A, row 1, playing", "");
            s.press("header", "tab", "Arrange").settle();
            s.press("clips", "pager", "Tracks after").settle();
            s.press("clips", "scene", "Scene 2");
            s.run_for(2.5);
            let off = wrote(&s, n, "looper: loop 1 off from bar ", "");
            s.press("clips", "slot", "Loops A slot 1");
            s.run_for(2.5);
            let on = wrote(&s, n, "looper: loop 1 on from bar ", "");
            let bar = |l: &str| l.rsplit(' ').next().and_then(|b| b.parse::<u32>().ok());
            assert!(bar(&on) > bar(&off), "{off} / {on}");
            s.button("STOP");
        });
    }

    /// `loop-cell`'s lines: SHIFT + "+ Track" adds a Loop track, a tap on
    /// its row 2 cell arms it, and REC, PLAY and REC record a loop there.
    #[test]
    fn loop_cell_writes_its_lines() {
        use mpfree_looper::Phase;
        on_big_stack(|| {
            let mut s = Session::new();
            s.run_for(0.1);
            s.press("header", "tab", "Arrange").settle();
            let n = s.lines.len();
            s.button_edge("SHIFT", true);
            s.press("clips", "add-track", "+ Track").settle();
            s.button_edge("SHIFT", false);
            wrote(&s, n, "time: Loops A added, a Loop track", "");
            s.press("clips", "pager", "Tracks after").settle();
            s.press("clips", "slot", "Loops A slot 2").settle();
            wrote(&s, n, "time: loop cell: Loops A row 2 armed", "");
            s.button("REC").button("PLAY");
            s.run_for(0.5);
            s.pad(0, 100, 0.05);
            s.run_for(2.0);
            s.button("REC");
            s.run_for(2.5);
            assert_eq!(s.looper.phase(), Phase::Playing);
            wrote(&s, n, "time: Loop 1 on Loops A, row 2, playing", "");
            s.button("STOP");
        });
    }

    /// `loop-copy`'s lines: SHIFT + Loops A's cell dropped on Loops B's
    /// row 3 copies the loop there, its layers all go in, and its cell's
    /// launch brings the copy in on a bar line.
    #[test]
    fn loop_copy_writes_its_lines() {
        use mpfree_looper::Phase;
        on_big_stack(|| {
            let mut s = Session::new();
            s.run_for(0.1);
            s.press("play-transport", "view", "Loop").settle();
            s.button("REC").button("PLAY");
            s.run_for(0.5);
            s.pad(0, 100, 0.05);
            s.run_for(2.0);
            s.button("REC");
            s.run_for(2.5);
            assert_eq!(s.looper.phase(), Phase::Playing);
            s.press("header", "tab", "Arrange").settle();
            s.button_edge("SHIFT", true);
            s.press("clips", "add-track", "+ Track").settle();
            s.button_edge("SHIFT", false);
            let n = s.lines.len();
            s.route(FromUi::Time(TimeAction::DropClip {
                from: (8, 0),
                to: (9, 2),
                copy: true,
            }));
            s.run_for(1.0);
            wrote(&s, n, "clips: Loop 1 copies into Loop 2 on Loops B / 3", "");
            wrote(&s, n, "looper: loop 2 is a copy of loop 1, 1 layer", "");
            s.press("clips", "pager", "Tracks after").settle();
            s.press("clips", "slot", "Loops B slot 3");
            s.run_for(2.5);
            wrote(&s, n, "looper: loop 2 on from bar ", "");
            s.button("STOP");
        });
    }

    #[test]
    fn rec_arm_writes_its_lines() {
        use mpfree_looper::Phase;
        use mpfree_transport::State;
        on_big_stack(|| {
            let mut s = Session::new();
            s.run_for(0.1);
            s.press("play-transport", "view", "Beats").settle();
            s.press("play-transport", "view", "All").settle();
            let n = s.lines.len();
            s.button("REC");
            s.run_for(0.1);
            wrote(&s, n, "rec: ready, PLAY records ", "");
            assert_eq!(s.time.info().state, State::Stopped, "nothing starts");
            s.button("PLAY");
            s.run_for(2.0);
            assert_eq!(s.time.info().state, State::Recording);
            s.pad(0, 100, 0.05);
            s.run_for(2.0);
            s.button("STOP");
            s.run_for(0.2);
            wrote(&s, n, "time: transport: Record, ", "");
            // PLAY plays the take back.
            let from = s.hits.on.len();
            s.button("PLAY");
            s.run_for(4.5);
            assert!(
                s.hits.on[from..].iter().any(|h| h.0 == 0),
                "the take played back"
            );
            s.button("STOP");
            s.run_for(0.2);
            s.button("UNDO");
            s.run_for(0.2);
            wrote(&s, n, "Undid take ", "");
            // UNDO while it records throws the take away; the music plays on.
            s.button("REC").button("PLAY");
            s.run_for(2.0);
            assert_eq!(s.time.info().state, State::Recording);
            s.pad(0, 100, 0.05);
            s.run_for(0.3);
            s.button("UNDO");
            s.run_for(0.3);
            assert_eq!(s.time.info().state, State::Playing, "the music plays on");
            let l = wrote(&s, n, "time: undo: take ", " thrown away");
            assert!(l.starts_with("time: undo: take "), "{l}");
            s.button("STOP");
            s.run_for(0.2);
            let n = s.lines.len();
            s.press("play-transport", "view", "Loop").settle();
            s.button("REC");
            s.run_for(0.2);
            assert_eq!(
                wrote(&s, n, "rec: the loop armed", ""),
                "rec: the loop armed"
            );
            wrote(&s, n, "looper: Record: armed", "");
            assert_eq!(s.looper.phase(), Phase::Armed);
            s.button("STOP");
            s.run_for(0.2);
            assert_eq!(s.looper.phase(), Phase::Empty, "STOP disarms");
        });
    }

    /// Live erase (`^ERASE: pad [A-H]?\d+'s notes come out`): ERASE held and a
    /// pad pressed while recording erases that pad's notes as the playhead
    /// passes them, and the pad neither sounds nor records; letting go of
    /// the pad, or of ERASE, stops it; while only playing, ERASE + a pad
    /// plays the pad as before.
    #[test]
    fn live_erase_writes_its_lines() {
        use mpfree_transport::State;
        on_big_stack(|| {
            let mut s = Session::new();
            s.run_for(0.1);
            s.button("REC");
            s.button("PLAY");
            s.run_for(2.0);
            assert_eq!(s.time.info().state, State::Recording);
            s.pad(0, 100, 0.05);
            s.pad(2, 100, 0.05);
            s.run_for(2.0);
            s.button("STOP");
            s.run_for(0.2);
            let pads_in = |s: &Session| {
                let mut n: Vec<u8> = s.time.project().tracks[0]
                    .slots
                    .iter()
                    .flatten()
                    .flat_map(|c| c.notes.iter().map(|n| n.note))
                    .collect();
                n.sort();
                n.dedup();
                n
            };
            assert_eq!(pads_in(&s), [0, 2]);
            // Overdub the take, with ERASE + pad 1 held for a while.
            let n = s.lines.len();
            s.button("OVERDUB");
            s.button("PLAY");
            s.run_for(2.0);
            assert_eq!(s.time.info().state, State::Recording);
            wrote(&s, n, "rec: ready, PLAY adds to ", "");
            let n = s.lines.len();
            let hits = s.hits.on.len();
            s.button_edge("ERASE", true);
            s.pad_edge(0, 100, true);
            s.run_periods(4);
            wrote(&s, n, "ERASE: pad A1's notes come out", "");
            assert!(s.time.erases(0));
            s.run_for(4.5);
            assert!(
                !s.hits.on[hits..].iter().any(|h| h.0 == 0),
                "pad 1 neither sounds nor plays back"
            );
            assert_eq!(pads_in(&s), [2], "pad 1's notes came out");
            s.pad_edge(0, 0, false);
            assert!(!s.time.erasing(), "let go of the pad: it stops");
            // ERASE let go first stops it too.
            s.pad_edge(2, 100, true);
            assert!(s.time.erases(2));
            s.button_edge("ERASE", false);
            assert!(!s.time.erasing(), "let go of ERASE: it stops");
            s.pad_edge(2, 0, false);
            s.button("STOP");
            assert_eq!(pads_in(&s), [2], "pad 3's notes stay");
            // The check ends with UNDO twice on Play's All view: both
            // takes go, so the checks after it find Track 1 as it was.
            s.press("play-transport", "view", "All");
            s.run_for(0.2);
            let n = s.lines.len();
            s.button("UNDO").run_for(0.2);
            s.button("UNDO").run_for(0.2);
            let undid = since(&s, n)
                .iter()
                .filter(|l| l.starts_with("Undid take "))
                .count();
            assert_eq!(undid, 2, "{:#?}", since(&s, n));
            assert!(pads_in(&s).is_empty(), "Track 1 has no notes left");
        });
    }

    /// Knob recording (sequencer S1): while recording, a knob turned on
    /// the screen for the armed track (a mixer strip's pan here, an
    /// instrument's knob) becomes a lock on the note being played; while
    /// only playing, nothing is recorded.
    #[test]
    fn knob_turns_while_recording_become_locks() {
        use mpfree_fx::ids::mixer;
        use mpfree_fx::StripId;
        use mpfree_mixer_ui::MixAction;
        use mpfree_play::mpfree_param::{OnTrack, RelAddr};
        use mpfree_synth_ui::view::SynthAction;
        use mpfree_transport::State;
        on_big_stack(|| {
            let mut s = Session::new();
            s.run_for(0.1);
            s.button("REC");
            s.button("PLAY");
            s.run_for(2.0);
            assert_eq!(s.time.info().state, State::Recording);
            let locks = |s: &Session| -> Vec<(RelAddr, f32)> {
                s.time.project().tracks[0]
                    .slots
                    .iter()
                    .flatten()
                    .flat_map(|c| c.locks.iter().map(|l| (l.addr, l.value)))
                    .collect()
            };
            assert!(locks(&s).is_empty());
            // Pad 1 held, the Track 1 strip's pan turned, and a knob of
            // its instrument.
            s.pad_edge(0, 100, true);
            s.run_periods(2);
            s.route(FromUi::Mixer(MixAction::Set {
                strip: StripId::Track(0),
                id: mixer::PAN,
                value: -40.0,
            }));
            s.route(FromUi::Synth(SynthAction::Set { id: 3, value: 0.25 }));
            s.run_periods(2);
            s.pad_edge(0, 0, false);
            s.run_for(2.5);
            let got = locks(&s);
            assert!(
                got.contains(&(RelAddr::new(OnTrack::Strip, mixer::PAN), -40.0)),
                "{got:?}"
            );
            assert!(
                got.contains(&(RelAddr::new(OnTrack::Inst, 3), 0.25)),
                "{got:?}"
            );
            s.button("STOP");
            s.run_for(0.2);
            // Playing, not recording: a turn is only a turn.
            let before = locks(&s).len();
            s.button("PLAY");
            s.run_for(0.5);
            s.route(FromUi::Mixer(MixAction::Set {
                strip: StripId::Track(0),
                id: mixer::PAN,
                value: 20.0,
            }));
            s.run_for(2.5);
            assert_eq!(locks(&s).len(), before);
            s.button("STOP");
        });
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

    /// The mute queue (`^track mute: .* muted on the next `, `^time:
    /// track mute: at \d+\.1, .* muted`, `^track mute: .* solo on the
    /// next `, `^time: track mute: at \d+\.1, .* solo`): in Perform a
    /// pad waits for the bar line and lands on it; TRACK MUTE held solos.
    #[test]
    fn mute_queue_writes_its_lines() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("play-transport", "view", "Beats");
            for k in [1, 5, 9, 13] {
                s.press("steps", "step", &format!("Kick step {k}"));
            }
            s.button("PLAY").run_for(0.2);
            s.button_with("SHIFT", "MAIN").run_for(0.2);
            s.button("TRACK MUTE");
            let n = s.lines.len();
            s.pad(0, 100, 0.05);
            wrote(&s, n, "track mute: ", " muted on the next ");
            s.run_for(2.2);
            let l = wrote(&s, n, "time: track mute: at ", " muted");
            assert!(l.starts_with("time: track mute: at 2.1, "), "{l}");
            let n = s.lines.len();
            s.button_edge("TRACK MUTE", true);
            s.pad(1, 100, 0.05);
            s.button_edge("TRACK MUTE", false);
            wrote(&s, n, "track mute: ", " solo on the next ");
            s.run_for(2.2);
            wrote(&s, n, "time: track mute: at ", " solo");
            s.button("TRACK MUTE");
            s.button("STOP");
        });
    }

    /// Reload clip on the bar (`^time: save state: .* kept`, `^time:
    /// reload: .* reloads at `, `^time: (.*; )?reload: .* back as saved on
    /// bar \d+`):
    /// Save state keeps the clip, Reload clip waits for the bar line and
    /// the saved pattern comes back on it.
    #[test]
    fn reload_on_the_bar_writes_its_lines() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("play-transport", "view", "Beats");
            for k in [1, 5, 9, 13] {
                s.press("steps", "step", &format!("Kick step {k}"));
            }
            s.press("header", "tab", "Arrange").settle();
            let tab = s
                .controls()
                .into_iter()
                .find(|o| o.widget == "record" && o.target.id == "tab" && o.target.index == 1)
                .expect("the Record panel's Clip tab")
                .target;
            s.press_target(&tab);
            s.settle();
            let n = s.lines.len();
            s.press("clip-card", "button", "Save state").settle();
            wrote(&s, n, "time: save state: ", " kept");
            s.press("header", "tab", "Play").settle();
            s.press("steps", "step", "Kick step 3");
            s.press("steps", "step", "Kick step 7");
            s.press("header", "tab", "Arrange").settle();
            s.button("PLAY").run_for(0.3);
            let n = s.lines.len();
            s.press("morph", "button", "Reload clip").settle();
            wrote(&s, n, "time: reload: ", " reloads at ");
            s.run_for(2.0);
            let l = wrote(&s, n, "time: ", " back as saved on bar ");
            assert!(l.contains("reload: "), "{l}");
            s.button("STOP");
        });
    }

    /// Live transpose (`^transpose: .* \+5 st$`, `^transpose: .* as
    /// written$`), as the `live-transpose` check has it done: with the
    /// synth on the pads, 16 LEVEL opens Transpose, pad 14 moves the armed
    /// track up a fourth and pad 9 brings it back.
    #[test]
    fn live_transpose_writes_its_lines() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.run_for(0.1);
            s.pick_synth();
            s.button("16 LEVEL");
            assert!(s.app.pads_transpose(), "16 LEVEL on the synth: Transpose");
            let n = s.lines.len();
            s.pad(13, 100, 0.05);
            let l = wrote(&s, n, "transpose: ", " +5 st");
            assert_eq!(l, "transpose: Track 1 +5 st");
            let n = s.lines.len();
            s.pad(8, 100, 0.05);
            wrote(&s, n, "transpose: Track 1 as written", "");
            s.button("16 LEVEL");
            let n = s.lines.len();
            s.pad(13, 100, 0.05);
            assert!(
                !since(&s, n).iter().any(|l| l.starts_with("transpose: ")),
                "16 LEVEL gave the pads back"
            );
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
            // The looper check's `^looper: loop of .* cut at STOP`, and
            // never `^looper: records the pads dry`.
            s.run_for(3.0);
            let n = s.lines.len();
            s.button("STOP").run_for(0.2);
            wrote(&s, n, "looper: loop of ", "cut at STOP");
            assert!(!s
                .lines
                .iter()
                .any(|l| l.starts_with("looper: records the pads dry")));
        });
    }
}
