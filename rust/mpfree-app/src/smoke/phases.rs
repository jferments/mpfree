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

    /// Runs until the loop's speed and ticks are `want`, for up to 20 s
    /// of wall time: a speed change goes through the looper's thread and
    /// can take longer than a fixed stretch of the music's time on a busy
    /// runner.
    fn until_speed(s: &mut Session, want: (i8, mpfree_transport::time::Tick)) {
        let start = std::time::Instant::now();
        loop {
            let st = status(s);
            if (st.speed, st.ticks) == want {
                return;
            }
            assert!(
                start.elapsed().as_secs() < 20,
                "speed and ticks {:?}, not {want:?}",
                (st.speed, st.ticks)
            );
            s.run_periods(1);
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
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

    /// The loop records what it records from as heard, with its effects,
    /// and the pad lands where it was played though the mix comes a
    /// period late. Pads is the Drums channel (its inserts
    /// and fader): the echo of the Play page's delay on the Drums sounds
    /// on Return A, outside it, so it stays out (20 Nov, the design
    /// review: the pads, never the whole mix). Records from:
    /// Master records the echo too (the final round of 16 Nov: the echo
    /// was heard but not recorded).
    #[test]
    fn the_loop_records_the_pads_with_their_effects() {
        use crate::looping::LoopSource;
        use crate::mixing::{PlayFx, DELAY_RETURN, DRUMS};
        use mpfree_fx::ids::mixer as mx;
        use mpfree_fx::StripId;
        for master in [false, true] {
            on_big_stack(move || {
                let mut s = Session::new();
                s.make_sounds();
                s.mix.play(PlayFx::Strip {
                    strip: StripId::Track(DRUMS),
                    id: mx::send(DELAY_RETURN),
                    value: 0.0,
                });
                if master {
                    let _ = s.looper.set_source(LoopSource::Mix(StripId::Master));
                }
                s.press("header", "tab", "Looper");
                s.button("REC").button("PLAY");
                until_phase(&mut s, Phase::Recording, 0.5);
                s.run_for(0.25);
                s.pad(0, 127, 0.05);
                s.run_for(1.5);
                s.button("STOP").run_for(0.1);
                assert_eq!(status(&s).phase, Phase::Playing);
                assert!(
                    !s.said().iter().any(|l| l.contains("records silence")),
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
                // The pad's sound is gone in 0.15 s; the 1/8 dotted echo
                // comes 0.375 s after it.
                let ms = |m: usize| hit + m * 441 / 10;
                let gap = rms(ms(250), ms(350));
                let echo = rms(ms(375), ms(475));
                if master {
                    assert!(
                        echo > 0.01 && echo > 5.0 * gap,
                        "echo {echo}, before it {gap}"
                    );
                } else {
                    assert!(echo < 1e-3, "no echo from Return A: {echo}");
                }
            });
        }
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

    /// The Perform set's other pages (step 6, board 7): PAD BANK B is
    /// Play's throws. A pad held plays its throw on the master's Perform
    /// FX and its lift lets it go; a latched throw plays until its next
    /// hit, and lets go when the set goes off. On page C a track mute pad
    /// mutes its track (at once in Make) and a scene pad launches its
    /// row. PAD BANK A is the loop keys again.
    #[test]
    fn the_perform_sets_pages_throw_mute_and_launch() {
        use crate::mixing::ThrowFx;
        use crate::perform_pads::{Hold, PadAction, PerformSet, Throw};
        on_big_stack(|| {
            let mut s = Session::new();
            let mut set = PerformSet::default();
            set.pages[1].pads[7] = PadAction::Throw(Throw::new(7, Hold::Latch));
            set.pages[2].pads[0] = PadAction::Mute(0);
            set.pages[2].pads[4] = PadAction::Scene(0);
            // Only the screen's copy: nothing played to let go.
            let fx = s.app.set_perform_set(set);
            assert!(fx
                .iter()
                .all(|e| matches!(e, crate::app::Effect::PerformSet { .. })));
            s.press("header", "tab", "Looper");
            s.button_with("SHIFT", "SAMPLER");
            s.run_for(0.1);
            s.button("PAD BANK B/F");
            assert_eq!(s.app.perform_bank(), 1);
            assert!(s.said().iter().any(|l| l == "Perform: Page B · Throws"));
            assert!(
                s.hint().starts_with("Pads: Perform set, Page B · Throws"),
                "{}",
                s.hint()
            );
            // Pad 2 held: Reverb wash, while it is held.
            s.pad_edge(1, 100, true);
            s.run_periods(4);
            assert!(s.mix.throw_held(ThrowFx::Reverb), "held");
            assert_eq!(s.app.throws_on(), [(1, mpfree_fx::StripId::Master)]);
            s.pad_edge(1, 0, false);
            s.run_periods(4);
            assert!(!s.mix.throw_held(ThrowFx::Reverb), "let go with the pad");
            // Pad 8, latched: Vinyl brake plays on after the lift, and the
            // next hit lets it go.
            s.pad(7, 100, 0.05).run_periods(2);
            assert!(s.mix.throw_held(ThrowFx::VinylBrake), "latched");
            assert!(s.said().iter().any(|l| l == "perform: Vinyl brake latched"));
            s.pad(7, 100, 0.05).run_periods(2);
            assert!(!s.mix.throw_held(ThrowFx::VinylBrake));
            assert!(s.said().iter().any(|l| l == "perform: Vinyl brake let go"));
            // Latched again, the set off: it lets go.
            s.pad(7, 100, 0.05).run_periods(2);
            assert!(s.mix.throw_held(ThrowFx::VinylBrake));
            s.button_with("SHIFT", "SAMPLER").run_periods(2);
            assert!(!s.app.pads_are_loop_keys());
            assert!(!s.mix.throw_held(ThrowFx::VinylBrake), "the set off");
            // Back on, at page B; PAD BANK C: Track 1's mute and Scene 1.
            s.button_with("SHIFT", "SAMPLER");
            assert_eq!(s.app.perform_bank(), 1);
            s.button("PAD BANK C/G");
            assert_eq!(s.app.perform_bank(), 2);
            assert_eq!(s.time.muted(0), Some(false));
            s.pad(0, 100, 0.05);
            assert_eq!(s.time.muted(0), Some(true), "pad 1 muted Track 1");
            s.pad(0, 100, 0.05);
            assert_eq!(s.time.muted(0), Some(false));
            s.pad(4, 100, 0.05);
            assert!(s.said().iter().any(|l| l == "perform: Scene 1"));
            // PAD BANK A: the loop keys.
            s.button("PAD BANK A/E");
            assert_eq!(s.app.perform_bank(), 0);
            assert!(s.hint().starts_with("Pads: loop keys"), "{}", s.hint());
        });
    }

    /// A Perform pad's throw on the Drums track (board 7's On): picking
    /// On puts a Perform FX on Drums and says so, a hit plays the throw
    /// there and not on the master, and On back to the master takes it off
    /// again.
    #[test]
    fn a_throw_on_drums_plays_on_a_perform_fx_its_on_put_there() {
        use crate::gui::FromUi;
        use crate::perform_pads::{Hold, PadAction, Throw};
        use mpfree_fx::StripId;
        on_big_stack(|| {
            let mut s = Session::new();
            let drums = StripId::Track(crate::mixing::DRUMS);
            let perform_on = |s: &Session| {
                let c = s.mix.mixer.chain(drums)?;
                (0..c.len()).find(|&i| {
                    c.node(i)
                        .is_some_and(|n| n.info().id == crate::mixing::PERFORM_FX)
                })
            };
            let delay = Throw::new(0, Hold::WhileHeld);
            let on_drums = Throw { on: drums, ..delay };
            assert_eq!(perform_on(&s), None);
            s.route(FromUi::PerformPad {
                page: 1,
                pad: 0,
                action: PadAction::Throw(on_drums),
            });
            s.run_for(0.1);
            assert!(s.said().iter().any(|l| l
                == "perform: Perform FX put on Drums for the Perform pads' throws (the Mix tab \
                    shows it)"));
            let slot = perform_on(&s).expect("a Perform FX on Drums");
            // The set on the pads at page B: pad 1 throws on Drums.
            s.press("header", "tab", "Looper");
            s.button_with("SHIFT", "SAMPLER");
            s.button("PAD BANK B/F");
            s.pad_edge(0, 100, true);
            s.run_periods(4);
            let held = |s: &Session, strip: StripId, slot: usize| {
                s.mix
                    .mixer
                    .chain(strip)
                    .and_then(|c| c.node(slot))
                    .is_some_and(|n| n.param(mpfree_fx::ids::perform::hold(9)) > 0.5)
            };
            assert!(held(&s, drums, slot), "on Drums");
            assert!(
                !s.mix.throw_held(crate::mixing::ThrowFx::Delay),
                "not the master"
            );
            s.pad_edge(0, 0, false);
            s.run_periods(4);
            assert!(!held(&s, drums, slot));
            // On back to the master: the Perform FX comes off Drums.
            s.route(FromUi::PerformPad {
                page: 1,
                pad: 0,
                action: PadAction::Throw(delay),
            });
            s.run_for(0.1);
            assert!(s
                .said()
                .iter()
                .any(|l| l
                    == "perform: Perform FX taken off Drums (no Perform pad throws there now)"));
            assert_eq!(perform_on(&s), None);
        });
    }

    /// Learn a key (board 7): a key learned on a Perform pad plays it on
    /// any page, whatever the MPC's pads show. On Play, with the pads
    /// playing the kit, B2's key plays Reverb wash while held and lets it
    /// go; an empty pad's key says so; and a controller unplugged with the
    /// key held lets the throw go.
    #[test]
    fn a_key_learned_on_a_perform_pad_plays_it_from_any_page() {
        use mpfree_play::mpfree_param::perform;
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("header", "tab", "Play");
            assert!(!s.app.pads_are_perform());
            let b2 = perform::pad(1, 1);
            s.perform_key(b2, 1.0);
            assert_eq!(s.app.throws_on().len(), 1, "Reverb wash plays");
            let said = s.said();
            assert!(said
                .iter()
                .any(|l| l == "perform: Perform pads B2, from a learned key"));
            assert!(said.iter().any(|l| l == "perform: Reverb wash"));
            s.perform_key(b2, 0.0);
            assert!(s.app.throws_on().is_empty(), "let go");
            // An empty pad: nothing plays, and the log says why.
            s.perform_key(perform::pad(1, 15), 1.0);
            assert!(s.app.throws_on().is_empty());
            assert!(s
                .said()
                .iter()
                .any(|l| l == "perform: Perform pads B16 has nothing on it, from a learned key"));
            s.perform_key(perform::pad(1, 15), 0.0);
            // Unplugged while held: the throw lets go.
            s.perform_key(b2, 1.0);
            assert_eq!(s.app.throws_on().len(), 1);
            s.unplug_perform_keys();
            assert!(s.app.throws_on().is_empty(), "nothing stays held");
            assert!(!s.app.pads_are_perform(), "the pads never moved");
        });
    }

    /// The Perform view's Q-Links on a throw that plays on Drums turn its
    /// Feedback on the Drums track's Perform FX, through the desk to the
    /// mixer; a run of turns is one UNDO step there; Edit the effect opens
    /// that Perform FX in the chain editor on the Delay throw's page.
    #[test]
    fn the_perform_views_q_links_turn_a_throw_on_its_strips_perform_fx() {
        use crate::gui::FromUi;
        use crate::perform_pads::{Hold, PadAction, Throw};
        use mpfree_fx::ids::perform as pf;
        use mpfree_fx::StripId;
        on_big_stack(|| {
            let mut s = Session::new();
            let drums = StripId::Track(crate::mixing::DRUMS);
            let on_drums = Throw {
                on: drums,
                ..Throw::new(0, Hold::WhileHeld)
            };
            s.route(FromUi::PerformPad {
                page: 1,
                pad: 0,
                action: PadAction::Throw(on_drums),
            });
            s.run_for(0.1);
            let feedback = |s: &Session| {
                let c = s.mix.mixer.chain(drums).expect("Drums' chain");
                (0..c.len())
                    .filter_map(|i| c.node(i))
                    .find(|n| n.info().id == crate::mixing::PERFORM_FX)
                    .map(|n| n.param(pf::slot(crate::mixing::ThrowFx::Delay.pad()) + pf::AMOUNT))
            };
            assert_eq!(feedback(&s), Some(55.0));
            // The Pads tab's Perform view, page B, pad 1.
            s.press("header", "tab", "Pads");
            s.press(mpfree_sampler_ui::pads_tab::KITS, "perform", "Perform pads");
            s.button("PAD BANK B/F");
            s.pad_edge(0, 100, true);
            s.run_periods(2);
            s.pad_edge(0, 0, false);
            s.run_periods(4);
            let cards = |s: &Session| {
                s.screen
                    .ui
                    .get::<mpfree_ui::qlink::QLinkColumn>(crate::play_page::QLINKS)
                    .map(|q| {
                        q.cards
                            .each_ref()
                            .map(|c| (c.name.clone(), c.value.clone()))
                    })
            };
            assert_eq!(
                cards(&s).map(|c| [c[0].clone(), c[1].clone()]),
                Some([
                    ("Time".to_string(), "1/8D".to_string()),
                    ("Feedback".to_string(), "55 %".to_string())
                ])
            );
            // Q2 turned up 5, then 3 more: on the mixer, one UNDO step.
            s.qlink(1, 5);
            s.qlink(1, 3);
            s.run_periods(4);
            assert_eq!(feedback(&s), Some(63.0));
            s.screen.apply(crate::gui::ToUi::QLinkTouch(1, false));
            s.run_periods(4);
            assert!(s
                .said()
                .iter()
                .any(|l| l == "perform: Perform pads: Delay throw on Drums: Feedback 63 %"));
            s.button("UNDO");
            s.run_periods(10);
            assert_eq!(feedback(&s), Some(55.0));
            assert!(s
                .said()
                .iter()
                .any(|l| l == "perform: Undo: Delay throw's Feedback on Drums back to 55 %"));
            // Q3 turns nothing, and says why.
            s.qlink(2, 1);
            assert!(s.said().iter().any(|l| l.ends_with(
                "A throw has two settings, on Q1 and Q2: Delay throw has no others here"
            )));
            // Edit the effect: the Drums Perform FX on its Slots 9-12 page.
            let slot = s.mix.perform_slot(drums);
            s.press(crate::perform_tab::ACTS, "button", "Edit the effect ›");
            s.run_periods(4);
            assert_eq!(s.app.view(), crate::app::View::Chain);
            let page = s
                .screen
                .ui
                .get::<mpfree_chain_ui::ChainView>(mpfree_chain_ui::CHAIN)
                .map(|v| {
                    let st = v.state();
                    (st.groups.get(st.group).cloned(), st.selected)
                });
            assert_eq!(page, Some((Some("Slots 9-12".to_string()), slot)));
        });
    }

    /// The Perform set's pages past A are actions, not notes (the
    /// sequencer thread's rec-arm rule): with the notes armed and
    /// recording, throw, track mute and scene pads put nothing in the
    /// clip, and a throw records only what Play's throw tiles do (nothing).
    #[test]
    fn perform_pages_record_nothing_while_the_notes_record() {
        use crate::perform_pads::{PadAction, PerformSet};
        use crate::rec_arm::Target;
        use mpfree_transport::State;
        fn notes(s: &Session) -> usize {
            s.time.project().tracks[0]
                .slots
                .clips()
                .map(|c| c.notes.len())
                .sum()
        }
        on_big_stack(|| {
            let mut s = Session::new();
            s.make_sounds();
            let mut set = PerformSet::default();
            set.pages[2].pads[0] = PadAction::Mute(1);
            set.pages[2].pads[4] = PadAction::Scene(0);
            s.app.set_perform_set(set);
            s.press("header", "tab", "Looper");
            s.button_with("SHIFT", "SAMPLER");
            s.button("PAD BANK B/F");
            s.press("header", "tab", "Play");
            assert!(s.app.pads_are_loop_keys(), "the set stays on Play");
            // The notes armed (an Arm pill's tap), as the Looper tab had
            // armed the loop.
            s.route(crate::gui::FromUi::ArmTap(Target::Notes));
            s.settle();
            assert_eq!(s.app.rec.target, Target::Notes);
            let before = notes(&s);
            s.button("REC").button("PLAY");
            let end = s.t() + 5.0;
            while s.time.info().state != State::Recording && s.t() < end {
                s.run_periods(1);
            }
            assert_eq!(s.time.info().state, State::Recording);
            // Page B: every throw, held a moment.
            for pad in 0..8 {
                s.pad(pad, 110, 0.1);
            }
            // Page C: a track mute (twice, so it plays again) and a scene.
            s.button("PAD BANK C/G");
            s.pad(0, 110, 0.05).pad(0, 110, 0.05).pad(4, 110, 0.05);
            s.run_for(0.5);
            s.button("STOP").settle();
            assert_eq!(notes(&s), before, "no pad of the set is a note");
            assert!(s.said().iter().any(|l| l == "perform: Reverb wash"));
        });
    }

    /// A footswitch (the looper redesign, step 5): controls learned on the
    /// looper play it from any page. On Play, with the pads playing the
    /// kit, Loop 1's key records from the bar line and ends the pass,
    /// a fader on its mute mutes it from the middle up, Next loop picks
    /// Loop 2, the picked loop's key arms Loop 2 and lets it go, Undo
    /// and Clear are the Looper tab's on the picked loop, and Loop 1's
    /// key held a second clears it.
    #[test]
    fn a_footswitch_plays_the_looper_from_any_page() {
        use mpfree_play::mpfree_param::looper as fs;
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("header", "tab", "Play");
            assert!(!s.app.pads_are_loop_keys());
            s.button("PLAY").run_for(0.3);
            let status = |s: &Session, n| s.looper.looper.loop_status(n).unwrap();
            // Loop 1's key: armed for the bar line, recording, and pressed
            // again it ends on the bar line.
            s.footswitch(fs::key(0), 1.0).footswitch(fs::key(0), 0.0);
            assert_eq!(status(&s, 0).phase, Phase::Armed);
            assert!(s.app.rec.armed(crate::rec_arm::Target::Loop));
            until_phase(&mut s, Phase::Recording, 2.5);
            s.run_for(2.0);
            s.footswitch(fs::key(0), 1.0).footswitch(fs::key(0), 0.0);
            assert!(status(&s, 0).ending);
            until_phase(&mut s, Phase::Playing, 2.5);
            assert!(s
                .said()
                .iter()
                .any(|l| l.contains("Loop 1's key, from a learned control")));
            // A fader on Loop 1's mute: muted from the middle up, and
            // playing again under it.
            s.footswitch(fs::mute(0), 0.8);
            assert!(status(&s, 0).muted);
            s.footswitch(fs::mute(0), 0.9);
            assert!(status(&s, 0).muted, "still up: nothing more");
            s.footswitch(fs::mute(0), 0.1);
            assert!(!status(&s, 0).muted);
            // Next loop: Loop 2 picked; its key (the picked loop's) arms
            // it, and again lets it go.
            s.footswitch(fs::NEXT_LOOP, 1.0)
                .footswitch(fs::NEXT_LOOP, 0.0);
            assert_eq!(s.looper.looper.selected(), 1);
            s.footswitch(fs::PICKED_KEY, 1.0)
                .footswitch(fs::PICKED_KEY, 0.0);
            assert_eq!(status(&s, 1).phase, Phase::Armed);
            s.footswitch(fs::PICKED_KEY, 1.0)
                .footswitch(fs::PICKED_KEY, 0.0);
            assert_eq!(status(&s, 1).phase, Phase::Empty);
            // Clear on the picked loop (Loop 1 again), and Undo brings it
            // back.
            s.footswitch(fs::NEXT_LOOP, 1.0)
                .footswitch(fs::NEXT_LOOP, 0.0);
            for _ in 0..6 {
                s.footswitch(fs::NEXT_LOOP, 1.0)
                    .footswitch(fs::NEXT_LOOP, 0.0);
            }
            assert_eq!(s.looper.looper.selected(), 0, "round after Loop 8");
            s.footswitch(fs::CLEAR, 1.0).footswitch(fs::CLEAR, 0.0);
            s.run_for(0.3);
            assert_eq!(status(&s, 0).phase, Phase::Empty);
            s.footswitch(fs::UNDO, 1.0).footswitch(fs::UNDO, 0.0);
            s.run_for(0.3);
            assert_eq!(status(&s, 0).phase, Phase::Playing, "Undo brought it back");
            // Loop 1's key held a second clears it; the pads never moved.
            s.footswitch(fs::key(0), 1.0);
            s.run_for(1.3);
            s.footswitch(fs::key(0), 0.0);
            s.run_periods(4);
            assert_eq!(status(&s, 0).phase, Phase::Empty, "the held key cleared it");
            assert!(s
                .said()
                .iter()
                .any(|l| l.contains("Loop 1 cleared (its key held), from a learned control")));
            assert!(!s.app.pads_are_loop_keys());
            s.button("STOP");
        });
    }

    /// Play's Swing, Drums filter and XY pad learned on a controller's
    /// knobs (the Learn section's Play card): each turns as Play's own
    /// Q-Link or finger does, from any page, and the XY pad on the screen
    /// follows; the pad lets go once it is back in the middle and at the
    /// bottom.
    #[test]
    fn learned_knobs_turn_plays_swing_filter_and_xy_pad_from_any_page() {
        use mpfree_play::mpfree_param::play;
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("header", "tab", "Mix");
            s.play_knob(play::SWING, 1.0);
            s.run_periods(2);
            assert_eq!(s.time.info().swing.amount, 75);
            s.play_knob(play::FILTER, 0.25).run_periods(2);
            assert_eq!(s.mix.play_levels().drums_filter, -50.0);
            // A knob's middle is the filter off.
            s.play_knob(play::FILTER, 64.0 / 127.0).run_periods(2);
            assert_eq!(s.mix.play_levels().drums_filter, 0.0);
            // XY across, then up: each leaves the other where it is.
            s.play_knob(play::XY_ACROSS, 1.0).run_periods(2);
            s.play_knob(play::XY_UP, 0.5).run_periods(2);
            assert_eq!(s.mix.xy_at(), (1.0, 0.5));
            assert_eq!(s.screen.play.xy.position(), (1.0, 0.5), "the pad follows");
            // Back in the middle and at the bottom: let go, and nothing
            // glides anywhere else.
            s.play_knob(play::XY_ACROSS, 64.0 / 127.0).run_periods(2);
            s.play_knob(play::XY_UP, 0.0).run_for(0.5);
            assert_eq!(s.mix.xy_at(), (0.5, 0.0));
            assert_eq!(s.screen.play.xy.position(), (0.5, 0.0));
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

    /// Speed (step 3c): ½× plays the loop an octave down over twice the
    /// bars, like tape, and UNDO and SHIFT + UNDO take it back and put it
    /// back; OVERDUB waits for the print at that speed, fits the loop to
    /// it and records; 1× after the fit plays the fitted audio an octave
    /// up, the loop back to its own length.
    #[test]
    fn half_speed_plays_the_loop_an_octave_down_and_an_overdub_fits_to_it() {
        on_big_stack(|| {
            let mut s = two_layers();
            let (ticks, frames) = (status(&s).ticks, status(&s).frames);
            s.press("leditor", "speed", "Speed ½×");
            until_speed(&mut s, (-1, 2 * ticks));
            let st = status(&s);
            assert_eq!((st.speed, st.rate, st.ticks), (-1, -1, 2 * ticks));
            assert!(s.until_said(
                "looper: speed ½×: an octave down, twice the bars (Undo takes it back)",
                2.0
            ));
            // The strip's ½× is the one lit (so it takes no tap), once the
            // screen has the looper's status.
            let start = std::time::Instant::now();
            while at(&mut s, "leditor", "speed", 0).enabled {
                assert!(start.elapsed().as_secs() < 20, "½× is the speed now");
                s.run_periods(1);
            }
            s.button("UNDO");
            until_speed(&mut s, (0, ticks));
            s.button_with("SHIFT", "UNDO");
            until_speed(&mut s, (-1, 2 * ticks));
            // OVERDUB waits for the print, which the worker makes, then
            // the loop is fitted to it and the overdub records.
            s.button("OVERDUB");
            let start = std::time::Instant::now();
            while status(&s).phase != Phase::Overdubbing {
                assert!(start.elapsed().as_secs() < 60, "no fit");
                s.run_periods(1);
                std::thread::sleep(std::time::Duration::from_micros(100));
            }
            let st = status(&s);
            assert_eq!((st.speed, st.rate, st.frames), (-1, 0, 2 * frames));
            s.pad(3, 120, 0.3).run_for(0.3);
            s.button("OVERDUB");
            until_phase(&mut s, Phase::Playing, 4.5);
            assert_eq!(status(&s).layers, 2, "the fitted layer and the overdub");
            s.button("UNDO").run_for(0.4);
            assert_eq!(status(&s).layers, 1);
            // 1× again: the fitted audio an octave up, the loop's own bars.
            s.press("leditor", "speed", "Speed 1×");
            until_speed(&mut s, (0, ticks));
            assert_eq!(status(&s).rate, 1);
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

    /// Capture (step 4a): the looper keeps what it heard; with the song
    /// playing, SHIFT + REC on the Looper tab turns the last whole bars
    /// played into Loop 1, in time with the song, and the header's
    /// Capture puts the next ones into Loop 2. What you played shows on an
    /// empty loop's page, its bars numbered and the take marked.
    #[test]
    fn capture_turns_what_you_played_into_a_loop() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.make_sounds();
            s.press("header", "tab", "Looper");
            s.button("PLAY").run_for(0.3);
            // A pad on every beat, into bar 4's second beat (120 BPM: a
            // bar is 2 s).
            for _ in 0..13 {
                s.pad(0, 120, 0.1).run_for(0.4);
            }
            // Free length asks 4 bars; 3 whole ones were played, so 2.
            let t = at(&mut s, "leditor", "capture", 0);
            assert_eq!(t.label, "Capture bars 2 and 3");
            s.button_with("SHIFT", "REC");
            assert!(
                s.said()
                    .iter()
                    .any(|l| l.starts_with("looper: Capture: bars 2 and 3 become Loop 1")),
                "{:?}",
                s.said()
            );
            until_phase(&mut s, Phase::Playing, 1.0);
            let bar = 4 * PPQ as i64;
            let st = status(&s);
            assert_eq!(
                (st.ticks, st.origin, st.layers),
                (2 * bar, bar, 1),
                "bars 2 and 3"
            );
            let peak = {
                let v = s.looper.looper.loop_layer(0, 0).expect("a layer");
                let mut f = vec![[0.0f32; 2]; v.len() / 2];
                v.read(0, &mut f);
                f.iter().fold(0.0f32, |m, x| m.max(x[0].abs()))
            };
            assert!(peak > 0.05, "the pads are in it: {peak}");

            // Two bars more, then SHIFT + REC with Loop 1 picked: into
            // Loop 2, the first empty one.
            for _ in 0..8 {
                s.pad(1, 120, 0.1).run_for(0.4);
            }
            s.button_with("SHIFT", "REC");
            s.run_for(0.1);
            assert!(
                s.said().iter().any(|l| l.contains("become Loop 2")),
                "{:?}",
                s.said()
            );
            let l2 = |s: &Session| s.looper.looper.loop_status(1).unwrap();
            let end = s.t() + 1.0;
            while l2(&s).phase != Phase::Playing && s.t() < end {
                s.run_periods(1);
            }
            assert_eq!(l2(&s).phase, Phase::Playing, "Loop 2 plays");
            assert_eq!(status(&s).phase, Phase::Playing, "Loop 1 plays on");

            // In a bar's last beat it waits for the bar line, Loop 3's
            // ring waiting as an armed one; a tap on it calls it off.
            let end = s.t() + 2.5;
            while !s.looper.looper.capture_waits(mpfree_looper::Clock::Song) && s.t() < end {
                s.run_periods(1);
            }
            assert!(
                s.looper.looper.capture_waits(mpfree_looper::Clock::Song),
                "a bar's last beat"
            );
            s.button_with("SHIFT", "REC");
            let l3 = |s: &Session| s.looper.looper.loop_status(2).unwrap();
            assert_eq!(l3(&s).capturing, Some(0.0), "waits for the bar line");
            let ring = at(&mut s, "ldeck", "ring", 2);
            assert_eq!(ring.label, "Loop 3 ring: cancel");
            s.press_target(&ring).run_for(1.0);
            assert!(
                s.said()
                    .iter()
                    .any(|l| l == "looper: Capture into Loop 3 called off"),
                "{:?}",
                s.said()
            );
            assert_eq!((l3(&s).phase, l3(&s).capturing), (Phase::Empty, None));
            s.button("STOP");
        });
    }

    /// Starts: When I play (design step 4b). Armed with the song stopped,
    /// the loop waits for you: nothing starts until the first pad, which
    /// starts the song and the loop together, bar 1 on the hit. With the
    /// song playing, a pad early in a bar starts the next loop on that
    /// bar's line, the pad's sound in it.
    /// Looper redesign step 4c: with Tempo from the first loop on and the
    /// song stopped, a first loop on Free length records free from REC;
    /// REC again closes it, its length sets the tempo (1 bar at about 96
    /// BPM here), and the song starts at that tempo as the loop comes
    /// round. The app takes the tempo exactly, so it isn't asked back.
    #[test]
    fn tempo_from_the_first_loop_sets_the_tempo_and_starts_the_song() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.make_sounds();
            s.press("header", "tab", "Looper");
            // The Looper tab's options: Tempo from the first loop (the
            // setting the MENU row steps).
            s.app.settings.loop_tempo = true;
            let free = at(&mut s, "leditor", "bars", 1);
            assert_eq!(free.label, "Free");
            s.press_target(&free).run_for(0.1);
            assert_eq!(s.time.info().state, mpfree_transport::State::Stopped);
            let before = s.app.settings.bpm;
            assert!(status(&s).free, "an empty loop that would record free");
            s.button("REC").run_for(0.1);
            assert!(
                s.said().iter().any(|l| l
                    == "looper: Record: the first pass records free from now: REC again \
                        closes it, and its length sets the tempo and starts the song"),
                "{:?}",
                s.said()
            );
            let st = status(&s);
            assert_eq!((st.phase, st.free), (Phase::Recording, true));
            // In your own time: a pad, and 2.5 s in all.
            s.pad(0, 120, 0.1);
            s.run_for(2.3);
            assert_eq!(s.time.info().state, mpfree_transport::State::Stopped);
            s.button("REC");
            until_phase(&mut s, Phase::Playing, 1.0);
            let st = status(&s);
            let bar = 4 * PPQ as i64;
            let reading =
                mpfree_looper::read_bars(st.frames, crate::RATE, bar, None, None).unwrap();
            let (bars, bpm) = (reading.bars, reading.bpm);
            assert_eq!((bars, st.ticks, st.origin), (1, bar, 0), "bar 1, one bar");
            assert!((90.0..100.0).contains(&bpm), "{bpm}");
            let said =
                format!("looper: Loop 1 is 1 bar at {bpm:.2} BPM: the song starts at that tempo");
            assert!(s.said().contains(&said), "{:?}", s.said());
            s.run_for(0.5);
            assert!(s.time.info().state.rolling(), "the song plays");
            assert_eq!(s.time.info().bpm, bpm);
            assert_eq!(s.app.settings.bpm, bpm, "the app's tempo, as set");
            assert_ne!(bpm, before);
            assert_eq!(s.time.tempo_map().segments().len(), 1);
            assert_eq!(s.looper.tempos_refused, 0);
            // Smart loop end (Suggest, the default) found no note near
            // the tap: it stood, counted and said.
            assert!(
                s.said()
                    .iter()
                    .any(|l| l == "looper: Loop 1: smart loop end kept your tap: no note near it"),
                "{:?}",
                s.said()
            );
            assert_eq!(s.looper.looper.stats().taps_kept, 1);
        });
    }

    /// Smart loop end step 1, the whole way through: on Auto, a pad hit
    /// just after REC, the same pad again 2 s later and REC tapped 40 ms
    /// after that. The pads' notes reach the looper, the loop is the
    /// length between the two hits, and the song starts as far into bar
    /// 1 as the loop already is.
    #[test]
    fn auto_smart_loop_end_closes_on_the_hit_you_played_again() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.make_sounds();
            s.press("header", "tab", "Looper");
            s.app.settings.loop_tempo = true;
            s.app.settings.smart_end = mpfree_looper::SmartEnd::Auto;
            let free = at(&mut s, "leditor", "bars", 1);
            s.press_target(&free).run_for(0.1);
            s.button("REC");
            // The opening hit (within the reach of REC, so the loop's
            // start is that far before it), and the same pad again.
            let first = s.time.next_frame();
            s.pad(0, 120, 0.1);
            s.run_for(2.0 - 0.1);
            let again = s.time.next_frame();
            s.pad_edge(0, 120, true);
            s.run_for(0.04);
            s.button("REC");
            s.pad_edge(0, 0, false);
            until_phase(&mut s, Phase::Playing, 1.0);
            let st = status(&s);
            let between = (again - first) as usize;
            assert!(
                st.frames.abs_diff(between) <= crate::PERIOD,
                "{} frames, {between} between the hits: {:?}",
                st.frames,
                s.said()
            );
            let line = s
                .said()
                .into_iter()
                .find(|l| l.starts_with("looper: Loop 1: your tap was "))
                .unwrap_or_else(|| panic!("{:?}", s.said()));
            assert!(
                line.contains(" ms late; smart loop end closed it on the end you played")
                    && line.contains(" (best: "),
                "{line}"
            );
            assert_eq!(s.looper.looper.stats().ends_moved, 1);
            // The song started part-way into bar 1, where the loop is.
            s.run_periods(1);
            let info = s.time.info();
            assert!(info.state.rolling());
            let st = status(&s);
            let loop_at = st.origin + (f64::from(st.position) * st.ticks as f64) as i64;
            assert!(info.position > 0, "not from the bar line");
            assert!(
                (info.position - loop_at).abs() < st.ticks / 100,
                "song at {}, loop at {loop_at}",
                info.position
            );
        });
    }

    /// Smart loop end step 1c, a smart start: on Auto, REC pressed 80 ms
    /// before the first pad hit. The loop starts on the hit, not on the
    /// press, is the length between the two hits, and the log says so.
    #[test]
    fn auto_starts_a_loop_on_the_first_hit_after_rec() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.make_sounds();
            s.press("header", "tab", "Looper");
            s.app.settings.loop_tempo = true;
            s.app.settings.smart_end = mpfree_looper::SmartEnd::Auto;
            let free = at(&mut s, "leditor", "bars", 1);
            s.press_target(&free).run_for(0.1);
            s.button("REC");
            s.run_for(0.08);
            let first = s.time.next_frame();
            s.pad(0, 120, 0.1);
            s.run_for(2.0 - 0.1);
            let again = s.time.next_frame();
            s.pad_edge(0, 120, true);
            s.run_for(0.04);
            s.button("REC");
            s.pad_edge(0, 0, false);
            until_phase(&mut s, Phase::Playing, 1.0);
            let st = status(&s);
            let between = (again - first) as usize;
            assert!(
                st.frames.abs_diff(between) <= crate::PERIOD,
                "{} frames, {between} between the hits",
                st.frames
            );
            let b = st.bounds.expect("bounds");
            let lead = b.start - b.captured.0;
            // 80 ms, and the moment the session takes the pad in (its
            // frame comes with the next screen frame, about 20 ms).
            let want = (0.08 * f64::from(crate::RATE)) as i64;
            assert!(
                (want..want + (0.025 * f64::from(crate::RATE)) as i64).contains(&lead),
                "started {lead} frames after REC"
            );
            assert_eq!(st.moved.map(|m| m.start), Some(lead));
            let line = s
                .said()
                .into_iter()
                .find(|l| l.starts_with("looper: Loop 1: your tap was "))
                .unwrap_or_else(|| panic!("{:?}", s.said()));
            assert!(
                line.contains(", and started it on your first hit, ")
                    && line.contains(" ms after REC (best: "),
                "{line}"
            );
            assert_eq!(s.looper.looper.stats().starts_found, 1);
        });
    }

    /// A free first loop on the pads closed 40 ms late with Suggest: the
    /// end offered, the loop's frames and tempo at the tap, and the frames
    /// between the two hits.
    fn closed_late(s: &mut Session) -> (mpfree_looper::Shift, usize, f64, usize) {
        s.make_sounds();
        s.press("header", "tab", "Looper");
        s.app.settings.loop_tempo = true;
        let free = at(s, "leditor", "bars", 1);
        s.press_target(&free).run_for(0.1);
        s.button("REC");
        let first = s.time.next_frame();
        s.pad(0, 120, 0.1);
        s.run_for(2.0 - 0.1);
        let again = s.time.next_frame();
        s.pad_edge(0, 120, true);
        s.run_for(0.04);
        s.button("REC");
        s.pad_edge(0, 0, false);
        until_phase(s, Phase::Playing, 1.0);
        let tap = status(s).frames;
        let tap_bpm = s.time.info().bpm;
        s.run_for(0.2);
        let by = status(s).offer.expect("an offer");
        assert!(by.end < 0, "the tap was late");
        (by, tap, tap_bpm, (again - first) as usize)
    }

    /// Smart loop end step 1b, the whole way through: Suggest closes on
    /// a tap 40 ms after the hit played again, then offers the end it
    /// heard. Snap (what the screens will offer) changes the loop's end
    /// and the song's tempo together on the loop's next downbeat, the
    /// app's tempo follows, and UNDO takes it back the same way.
    #[test]
    fn a_snap_changes_the_loop_and_the_tempo_on_its_downbeat_and_undo_takes_it_back() {
        on_big_stack(|| {
            let mut s = Session::new();
            let (by, tap, tap_bpm, between) = closed_late(&mut s);
            assert_eq!(
                s.looper.snap(mpfree_looper::Snap::Found),
                "looper: Snap: Loop 1 changes on its next downbeat, with the tempo"
            );
            let until_moved = |s: &mut Session, moved: Option<mpfree_looper::Shift>| {
                let end = s.t() + 3.0;
                while status(s).moved != moved && s.t() < end {
                    s.run_periods(1);
                }
                assert_eq!(status(s).moved, moved, "{:?}", s.said());
            };
            until_moved(&mut s, Some(by));
            let st = status(&s);
            assert_eq!(st.frames as i64, tap as i64 + by.longer());
            assert!(
                st.frames.abs_diff(between) <= crate::PERIOD,
                "{}",
                st.frames
            );
            let said = s.said();
            let line = said
                .iter()
                .find(|l| l.starts_with("looper: Loop 1: snapped "))
                .unwrap_or_else(|| panic!("{said:?}"));
            assert!(line.contains(" ms earlier: 1 bar at "), "{line}");
            s.run_for(0.1);
            let bpm = s.time.info().bpm;
            assert!(bpm > tap_bpm, "{bpm} after {tap_bpm}");
            assert_eq!(s.app.settings.bpm, bpm, "the app's tempo follows");
            assert_eq!(s.time.tempo_map().segments().len(), 1);
            // UNDO goes back on a downbeat too, with the tap's tempo.
            s.button("UNDO");
            until_moved(&mut s, None);
            s.run_for(0.1);
            assert_eq!(status(&s).frames, tap);
            assert_eq!(status(&s).offer, Some(by), "on offer again");
            assert!((s.time.info().bpm - tap_bpm).abs() < 1e-9);
            assert_eq!(s.app.settings.bpm, s.time.info().bpm);
            assert_eq!(s.looper.snaps_refused, 0);
        });
    }

    /// Smart loop end plan, step 2: Read as 2 bars keeps the loop's
    /// length and doubles the song's tempo on its next downbeat, the app's
    /// tempo following; the history says so, and UNDO reads it as before
    /// the same way.
    #[test]
    fn read_as_two_bars_doubles_the_tempo_on_the_downbeat_and_undo_reads_it_as_before() {
        on_big_stack(|| {
            let mut s = Session::new();
            let (_, tap, tap_bpm, _) = closed_late(&mut s);
            let bar = 4 * i64::from(mpfree_transport::time::PPQ);
            assert_eq!(status(&s).ticks, bar);
            assert_eq!(
                s.looper.snap(mpfree_looper::Snap::Bars(2)),
                "looper: Read as 2 bars: Loop 1 changes on its next downbeat, with the tempo"
            );
            let until_ticks = |s: &mut Session, ticks: i64| {
                let end = s.t() + 3.0;
                while status(s).ticks != ticks && s.t() < end {
                    s.run_periods(1);
                }
                assert_eq!(status(s).ticks, ticks, "{:?}", s.said());
                s.run_for(0.1);
            };
            until_ticks(&mut s, 2 * bar);
            assert_eq!(status(&s).frames, tap, "the length stays");
            let said = s.said();
            assert!(
                said.iter()
                    .any(|l| l.starts_with("looper: Loop 1: read again: 2 bars at ")),
                "{said:?}"
            );
            assert!((s.time.info().bpm - 2.0 * tap_bpm).abs() < 1e-9);
            assert_eq!(
                s.app.settings.bpm,
                s.time.info().bpm,
                "the app's tempo follows"
            );
            assert_eq!(
                s.looper.looper.history().done().last(),
                Some(&mpfree_looper::Step::Bars(2))
            );
            // UNDO reads it as before, on a downbeat, with the tap's tempo.
            s.button("UNDO");
            until_ticks(&mut s, bar);
            let said = s.said();
            assert!(
                said.iter()
                    .any(|l| l.starts_with("looper: Loop 1: Undo: read as before: 1 bar at ")),
                "{said:?}"
            );
            assert!((s.time.info().bpm - tap_bpm).abs() < 1e-9);
            assert_eq!(s.looper.snaps_refused, 0);
        });
    }

    /// Free loops' step F3: a free first loop (Sync: Free) closed 40 ms
    /// late with Suggest gets its offer too. Snap changes its end and the
    /// free clock's tempo together on its next downbeat, with nothing
    /// asked of the time engine: the song's tempo, and the app's, never
    /// move, and recordings on the audio tracks (which follow the song)
    /// aren't asked about. UNDO takes it back the same way.
    #[test]
    fn a_free_loops_snap_changes_the_free_clocks_tempo_and_not_the_songs() {
        on_big_stack(|| {
            let mut s = Session::new();
            let said = s
                .looper
                .command(mpfree_looper::Command::SetClock(mpfree_looper::Clock::Free));
            assert!(said.contains("sync: free"), "{said}");
            let song_bpm = s.time.info().bpm;
            let (by, tap, _, between) = closed_late(&mut s);
            assert_eq!(status(&s).clock, mpfree_looper::Clock::Free);
            let free_bpm = s.looper.looper.free_clock().expect("its clock").bpm;
            assert_eq!(s.time.info().bpm, song_bpm, "a free loop sets no tempo");
            s.recorded_audio = Some(true);
            s.looper.snap(mpfree_looper::Snap::Found);
            let until_moved = |s: &mut Session, moved: Option<mpfree_looper::Shift>| {
                let end = s.t() + 3.0;
                while status(s).moved != moved && s.t() < end {
                    assert_eq!(s.time.info().bpm, song_bpm, "the song's tempo never moves");
                    s.run_periods(1);
                }
                assert_eq!(status(s).moved, moved, "{:?}", s.said());
                assert_eq!(s.time.info().bpm, song_bpm, "the song's tempo never moves");
            };
            until_moved(&mut s, Some(by));
            assert_eq!(s.looper.snap_asking(), None, "nothing asked");
            let st = status(&s);
            assert_eq!(st.frames as i64, tap as i64 + by.longer());
            assert!(
                st.frames.abs_diff(between) <= crate::PERIOD,
                "{}",
                st.frames
            );
            let said = s.said();
            let line = said
                .iter()
                .find(|l| l.starts_with("looper: Loop 1: snapped "))
                .unwrap_or_else(|| panic!("{said:?}"));
            assert!(line.contains(" ms earlier: 1 bar, free at "), "{line}");
            s.run_for(0.1);
            let c = s.looper.looper.free_clock().unwrap();
            assert!(c.running && c.bpm > free_bpm, "{} after {free_bpm}", c.bpm);
            assert_eq!(s.time.info().bpm, song_bpm, "the song's tempo stays");
            assert_eq!(s.app.settings.bpm, song_bpm, "and the app's");
            assert_eq!(s.time.tempo_map().segments().len(), 1);
            // UNDO goes back on a downbeat too, with the tap's tempo.
            s.button("UNDO");
            until_moved(&mut s, None);
            s.run_for(0.1);
            assert_eq!(status(&s).frames, tap);
            assert_eq!(status(&s).offer, Some(by), "on offer again");
            let c = s.looper.looper.free_clock().unwrap();
            assert!((c.bpm - free_bpm).abs() < 1e-9, "{} {free_bpm}", c.bpm);
            assert_eq!(s.time.info().bpm, song_bpm);
            assert_eq!(s.looper.snaps_refused, 0);
        });
    }

    /// Free loops' step F3, both clocks at once: a free Loop 1 and a song
    /// Loop 2, each closed 40 ms late with Suggest, each snapped with the
    /// other playing. Neither is in the other's way: Loop 2's snap changes
    /// the song's tempo and leaves the free clock, and Loop 1, as they
    /// were; Loop 1's changes the free clock's tempo and leaves the
    /// song's.
    #[test]
    fn a_free_loop_and_a_song_loop_snap_without_holding_each_other_back() {
        on_big_stack(|| {
            use mpfree_looper::{Clock, Command, Snap};
            let mut s = Session::new();
            s.looper.command(Command::SetClock(Clock::Free));
            let (by1, tap1, _, _) = closed_late(&mut s);
            let free_bpm = s.looper.looper.free_clock().expect("its clock").bpm;
            // Loop 2 on the song's clock, closed late the same way: its
            // length sets the song's tempo, and the song starts.
            assert_eq!(s.looper.select(1), "looper: loop 2 picked (empty)");
            assert_eq!(
                s.looper.command(Command::SetClock(Clock::Song)),
                "looper: sync: the song"
            );
            let (by2, tap2, tap_bpm, _) = closed_late(&mut s);
            assert_eq!(status(&s).clock, Clock::Song);
            // Loop 2's snap moves the song's tempo, with Loop 1 playing.
            s.looper.snap(Snap::Found);
            until_moved(&mut s, Some(by2));
            assert_eq!(status(&s).frames as i64, tap2 as i64 + by2.longer());
            let song_bpm = s.time.info().bpm;
            assert!(song_bpm > tap_bpm, "{song_bpm} after {tap_bpm}");
            let c = s.looper.looper.free_clock().unwrap();
            assert!(c.running && c.bpm == free_bpm, "as it was: {c:?}");
            let l1 = s.looper.looper.loop_status(0).unwrap();
            assert_eq!(
                (l1.phase, l1.frames, l1.moved),
                (Phase::Playing, tap1, None)
            );
            // Loop 1's moves the free clock's, with Loop 2 playing (picked
            // with a period to land in, as a press comes a frame later).
            s.looper.select(0);
            s.run_periods(1);
            assert_eq!(s.looper.looper.selected(), 0);
            s.looper.snap(Snap::Found);
            until_moved(&mut s, Some(by1));
            assert_eq!(status(&s).frames as i64, tap1 as i64 + by1.longer());
            let c = s.looper.looper.free_clock().unwrap();
            assert!(c.running && c.bpm > free_bpm, "{} after {free_bpm}", c.bpm);
            assert_eq!(s.time.info().bpm, song_bpm, "the song's tempo stays");
            let l2 = s.looper.looper.loop_status(1).unwrap();
            assert_eq!((l2.phase, l2.moved), (Phase::Playing, Some(by2)));
            assert_eq!(s.looper.snaps_refused, 0, "{:?}", s.said());
        });
    }

    /// Smart loop end plan, step 2: a snap that would change the tempo
    /// under the audio tracks' recordings asks first, and nothing moves
    /// until Snap anyway; Not now drops it.
    #[test]
    fn a_snap_over_recorded_audio_asks_first() {
        on_big_stack(|| {
            let mut s = Session::new();
            let (by, tap, _, _) = closed_late(&mut s);
            s.recorded_audio = Some(true);
            // Not now: nothing changes.
            s.looper.snap(mpfree_looper::Snap::Found);
            s.run_for(0.1);
            let said = s.said();
            assert!(
                said.iter().any(|l| l
                    == "looper: Loop 1: that changes the tempo under the audio tracks' \
                        recordings. Snap anyway?"),
                "{said:?}"
            );
            assert_eq!(
                s.looper.snap_asking(),
                Some((0, mpfree_looper::Snap::Found))
            );
            assert_eq!(
                s.looper.snap_not_now().as_deref(),
                Some("looper: Loop 1: not now: it stays as it is")
            );
            assert_eq!(s.looper.snap_asking(), None);
            s.run_for(2.5);
            assert_eq!(status(&s).frames, tap);
            assert_eq!(status(&s).offer, Some(by), "still on offer");
            // Snap anyway: it lands on the downbeat, as any snap.
            s.looper.snap(mpfree_looper::Snap::Found);
            s.run_for(0.1);
            assert!(s.looper.snap_anyway().is_some());
            let end = s.t() + 3.0;
            while status(&s).moved != Some(by) && s.t() < end {
                s.run_periods(1);
            }
            assert_eq!(status(&s).moved, Some(by), "{:?}", s.said());
            assert_eq!(status(&s).frames as i64, tap as i64 + by.longer());
            assert_eq!(s.looper.snaps_refused, 0);
        });
    }

    /// The Looper tab's editor, as drawn.
    fn smart_editor(s: &Session) -> &crate::looper_tab::LoopEditor {
        s.screen
            .ui
            .get::<crate::looper_tab::LoopEditor>(crate::looper_tab::EDITOR)
            .expect("the editor")
    }

    /// Run until loop 1's moved is `moved` (a snap landed), and a view
    /// after it.
    fn until_moved(s: &mut Session, moved: Option<mpfree_looper::Shift>) {
        let end = s.t() + 3.0;
        while status(s).moved != moved && s.t() < end {
            s.run_periods(1);
        }
        assert_eq!(status(s).moved, moved, "{:?}", s.said());
        s.run_for(0.1);
    }

    /// Smart loop end step 2's screens, the whole way through (Jay's OK
    /// on the drawings, 22 Nov): the chip says how late the tap was, and
    /// its Snap snaps on the downbeat; then it says what the snap did, and
    /// its words open the loop point view with the audio round the press
    /// and the tap. A/B swaps the two sides on the downbeats until Keep my
    /// tap, which puts the chip and the view away.
    #[test]
    fn the_chip_snaps_and_the_loop_point_view_swaps_and_keeps_a_side() {
        on_big_stack(|| {
            let mut s = Session::new();
            let (by, _, _, _) = closed_late(&mut s);
            s.run_for(0.1);
            let ms = |f: i64| (f.abs() as f64 * 1000.0 / 44_100.0).round();
            let chip = smart_editor(&s).chip.clone().expect("the chip");
            // (The first hit came a period's worth after REC.)
            let late = format!("{} ms late", ms(by.end));
            let words = match by.start {
                0 => late,
                f => format!("{late} · first hit {} ms after REC", ms(f)),
            };
            assert_eq!(chip.words, words);
            s.press("leditor", "chip-act", "Snap");
            until_moved(&mut s, Some(by));
            let chip = smart_editor(&s).chip.clone().expect("the chip");
            assert_eq!(chip.words, format!("Snapped {} ms earlier", ms(by.end)));
            assert_eq!(chip.action.0, "Use my tap");
            // The words open the view; the looper reads the audio for it.
            s.press("leditor", "chip", &chip.words);
            s.run_for(0.1);
            let look = smart_editor(&s).point.clone().expect("the view's look");
            assert!(
                look.line.starts_with("Use my tap puts it back: 1 bar at "),
                "{}",
                look.line
            );
            assert!(s.control("leditor", "point-close", "").is_some());
            assert!(
                look.end.peaks.iter().any(|&p| p > 50),
                "the hit played again is in the end's window"
            );
            // A/B: back to the tap on a downbeat, then the snap again.
            s.press("leditor", "point-ab", "A/B");
            until_moved(&mut s, None);
            assert_eq!(status(&s).offer, Some(by));
            until_moved(&mut s, Some(by));
            assert!(smart_editor(&s).point.as_ref().unwrap().ab == Some(true));
            // Keep my tap: A/B stops, the tap on the next downbeat, and
            // the chip and the view go.
            s.press("leditor", "point-pick", "Keep my tap");
            until_moved(&mut s, None);
            s.run_for(2.5);
            assert_eq!(status(&s).moved, None, "A/B stopped");
            assert_eq!(status(&s).offer, Some(by), "still found");
            let e = smart_editor(&s);
            assert!(e.chip.is_none(), "put away");
            assert!(s.control("leditor", "point-close", "").is_none());
            assert_eq!(s.looper.snaps_refused, 0);
        });
    }

    /// Smart loop end step 2's screens: a snap over recorded audio asks on
    /// the hint line, with Snap anyway and Not now at its end; Not now
    /// takes them away and nothing changes.
    #[test]
    fn the_hint_line_asks_with_snap_anyway_and_not_now() {
        on_big_stack(|| {
            let mut s = Session::new();
            let (by, tap, _, _) = closed_late(&mut s);
            s.recorded_audio = Some(true);
            s.run_for(0.1);
            s.press("leditor", "chip-act", "Snap");
            s.run_for(0.1);
            assert_eq!(
                s.hint(),
                "Loop 1: that changes the tempo under the audio tracks' recordings. Snap anyway?"
            );
            assert!(s.control("lask", "ask-yes", "Snap anyway").is_some());
            s.press("lask", "ask-no", "Not now");
            s.run_for(0.1);
            assert_eq!(s.looper.snap_asking(), None);
            assert!(s.control("lask", "ask-yes", "").is_none(), "gone");
            assert_ne!(
                s.hint(),
                "Loop 1: that changes the tempo under the audio tracks' recordings. Snap anyway?"
            );
            s.run_for(2.5);
            assert_eq!(status(&s).frames, tap);
            assert_eq!(status(&s).offer, Some(by));
        });
    }

    /// Length's bars are final (the looper refactor's Finding C): Length
    /// at 1 bar, played for 10 s with the song stopped, would be 24 BPM,
    /// so the loop waits, silent, the song stopped, and the hint line asks
    /// with Read as and Discard at its end. Read as starts the song at the
    /// tempo it read; Discard (a second take) clears the loop.
    #[test]
    fn a_length_too_slow_asks_on_the_hint_line_with_read_as_and_discard() {
        use mpfree_transport::State;
        on_big_stack(|| {
            for yes in [true, false] {
                let mut s = Session::new();
                s.make_sounds();
                s.press("header", "tab", "Looper");
                s.app.settings.loop_tempo = true;
                let one = at(&mut s, "leditor", "bars", 2);
                s.press_target(&one).run_for(0.1);
                s.button("REC");
                s.pad(0, 120, 0.1);
                s.run_for(10.0);
                s.button("REC");
                until_phase(&mut s, Phase::Playing, 1.0);
                // (What smart loop end said shows first, for its 4 s.)
                s.run_for(f64::from(crate::gui::TELL_SECONDS) + 0.5);
                let r = status(&s).asks.expect("it waits on its Length");
                assert!(
                    s.hint()
                        .starts_with("Loop 1: Length's 1 bar would be 24 BPM, slower than 30."),
                    "{}",
                    s.hint()
                );
                assert!(
                    s.said()
                        .iter()
                        .any(|l| l.starts_with("looper: Loop 1: Length's 1 bar would be")),
                    "{:?}",
                    s.said()
                );
                assert_eq!(s.time.info().state, State::Stopped, "nothing starts");
                let read_as = format!("Read as {} bars", r.bars);
                assert!(s.control("lask", "ask-yes", &read_as).is_some());
                if yes {
                    s.press("lask", "ask-yes", &read_as);
                    s.run_for(0.3);
                    assert_eq!(s.time.info().state, State::Playing);
                    assert!(
                        (s.time.info().bpm - r.bpm).abs() < 0.01,
                        "{}",
                        s.time.info().bpm
                    );
                    assert_eq!(status(&s).phase, Phase::Playing);
                } else {
                    s.press("lask", "ask-no", "Discard");
                    s.run_for(0.3);
                    assert_eq!(status(&s).phase, Phase::Empty);
                    assert_eq!(s.time.info().state, State::Stopped);
                }
                assert_eq!(status(&s).asks, None);
                assert!(s.control("lask", "ask-yes", "").is_none(), "gone");
            }
        });
    }

    /// Smart loop end step 2d, hands-free: a footswitch learned on Snap
    /// takes the snap on the downbeat and one learned on Use my tap puts
    /// the loop back on your tap, both hands on the instrument; pressed
    /// again where the loop already is, it stays. While the hint line
    /// asks, Use my tap is Not now and Snap is Snap anyway. With nothing
    /// to choose, it says so.
    #[test]
    fn a_footswitch_snaps_and_puts_the_loop_back_on_your_tap() {
        use mpfree_play::mpfree_param::looper;
        on_big_stack(|| {
            let mut s = Session::new();
            let (by, tap, _, _) = closed_late(&mut s);
            let press = |s: &mut Session, id: u16| {
                s.footswitch(id, 1.0).footswitch(id, 0.0);
            };
            press(&mut s, looper::SNAP);
            assert!(
                s.said().iter().any(|l| l
                    == "looper: Snap: Loop 1 goes there on its next downbeat, if it isn't there, \
                        from a footswitch"),
                "{:?}",
                s.said()
            );
            until_moved(&mut s, Some(by));
            assert_eq!(status(&s).frames as i64, tap as i64 + by.longer());
            press(&mut s, looper::SNAP);
            s.run_for(2.5);
            assert_eq!(status(&s).moved, Some(by), "already snapped, it stays");
            press(&mut s, looper::USE_MY_TAP);
            until_moved(&mut s, None);
            assert_eq!(status(&s).frames, tap);
            assert_eq!(status(&s).offer, Some(by));
            // Over the audio tracks' recordings the hint line asks: Use my
            // tap is Not now, Snap is Snap anyway.
            s.recorded_audio = Some(true);
            s.run_for(0.1);
            press(&mut s, looper::SNAP);
            s.run_for(0.1);
            assert!(s.looper.snap_asking().is_some(), "{:?}", s.said());
            press(&mut s, looper::USE_MY_TAP);
            assert_eq!(s.looper.snap_asking(), None);
            assert!(
                s.said()
                    .iter()
                    .any(|l| l == "looper: Loop 1: not now: it stays as it is, from a footswitch"),
                "{:?}",
                s.said()
            );
            s.run_for(2.5);
            assert_eq!(status(&s).frames, tap);
            press(&mut s, looper::SNAP);
            s.run_for(0.1);
            assert!(s.looper.snap_asking().is_some());
            press(&mut s, looper::SNAP);
            assert_eq!(s.looper.snap_asking(), None);
            until_moved(&mut s, Some(by));
            // Loop 2 has nothing to choose.
            s.footswitch(looper::NEXT_LOOP, 1.0)
                .footswitch(looper::NEXT_LOOP, 0.0);
            press(&mut s, looper::USE_MY_TAP);
            assert!(
                s.said()
                    .iter()
                    .any(|l| l
                        == "looper: Use my tap from a footswitch: Loop 2 has no snap to choose"),
                "{:?}",
                s.said()
            );
        });
    }

    /// A free first loop recorded from the whole mix (Records from:
    /// Master), smart loop end on `mode`: REC 80 ms before the first pad,
    /// the same pad again 2 s later, and REC 40 ms after it. The frames
    /// from the first hit to the second, and the frame REC started on.
    fn by_ear_take(s: &mut Session, mode: mpfree_looper::SmartEnd) -> (usize, u64) {
        use crate::looping::LoopSource;
        use mpfree_fx::StripId;
        s.make_sounds();
        let _ = s.looper.set_source(LoopSource::Mix(StripId::Master));
        s.press("header", "tab", "Looper");
        s.app.settings.loop_tempo = true;
        s.app.settings.smart_end = mode;
        let free = at(s, "leditor", "bars", 1);
        s.press_target(&free).run_for(0.1);
        s.button("REC");
        let rec = s.time.next_frame();
        s.run_for(0.08);
        let first = s.time.next_frame();
        s.pad(0, 120, 0.1);
        s.run_for(2.0 - 0.1);
        let again = s.time.next_frame();
        s.pad_edge(0, 120, true);
        s.run_for(0.04);
        s.button("REC");
        s.pad_edge(0, 0, false);
        until_phase(s, Phase::Playing, 1.0);
        ((again - first) as usize, rec)
    }

    /// Smart loop end step 3, by ear: recording the mix, smart loop end
    /// listens for the hits in it, not the pads' notes. On Auto the loop
    /// starts on the first hit's sound, 80 ms after REC, and closes where
    /// the second hit's sound lines up with it: the frames between the two
    /// hits, to the frame. The log says so, and the status line counts the
    /// hits heard and the finder's answer.
    #[test]
    fn by_ear_auto_closes_a_loop_from_the_mix_on_the_hits_it_hears() {
        on_big_stack(|| {
            let mut s = Session::new();
            let (between, _) = by_ear_take(&mut s, mpfree_looper::SmartEnd::Auto);
            let st = status(&s);
            assert!(
                st.frames.abs_diff(between) <= 1,
                "{} frames, {between} between the hits",
                st.frames
            );
            let lead = st.moved.expect("moved").start;
            let want = (0.08 * f64::from(crate::RATE)) as i64;
            assert!(
                (want..want + (0.025 * f64::from(crate::RATE)) as i64).contains(&lead),
                "started {lead} frames after REC"
            );
            let line = s
                .said()
                .into_iter()
                .find(|l| l.starts_with("looper: Loop 1: your tap was "))
                .unwrap_or_else(|| panic!("{:?}", s.said()));
            assert!(
                line.contains(
                    " late; smart loop end closed it on the end you played, and \
                               started it on your first hit, "
                ),
                "{line}"
            );
            let lst = s.looper.looper.stats();
            assert_eq!(
                (lst.finds_asked, lst.finds_answered, lst.finds_late),
                (1, 1, 0)
            );
            assert!(lst.hits_heard >= 2, "{lst:?}");
            let d = s.looper.describe();
            assert!(
                d.contains("; by ear: ")
                    && d.contains(" hits heard, 1 decisions asked of the finder, 1 answered in time, 0 too late"),
                "{d}"
            );
        });
    }

    /// The same by ear on Suggest: the loop closes on the tap, then offers
    /// the first hit and the end it heard as one snap, and Snap lands it
    /// on the downbeat: the frames between the hits.
    #[test]
    fn by_ear_suggest_offers_the_hits_it_hears_in_the_mix() {
        on_big_stack(|| {
            let mut s = Session::new();
            let (between, _) = by_ear_take(&mut s, mpfree_looper::SmartEnd::Suggest);
            let tap = status(&s).frames;
            s.run_for(0.4);
            let by = status(&s).offer.expect("an offer");
            assert!(by.end < 0 && by.start > 0, "{by:?}");
            assert!(
                (tap as i64 + by.longer()).abs_diff(between as i64) <= 1,
                "{tap} frames and {by:?}, {between} between the hits"
            );
            s.looper.snap(mpfree_looper::Snap::Found);
            until_moved(&mut s, Some(by));
            assert!(status(&s).frames.abs_diff(between) <= 1);
        });
    }

    /// Smart loop end plan, step 0B: with Tempo from the first loop on,
    /// Length 2 bars means 2 bars at the tempo you play (it records free
    /// from REC, and nothing is read). On Free, a loop near the old 80 BPM
    /// edge reads as a close call and says so on the screen; the same
    /// loop with the tempo set by hand to 80 first is 1 bar at 79.
    #[test]
    fn with_length_set_the_first_loop_is_its_bars_at_your_tempo() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.make_sounds();
            s.press("header", "tab", "Looper");
            s.app.settings.loop_tempo = true;
            let two = at(&mut s, "leditor", "bars", 3);
            assert_eq!(two.label, "2");
            s.press_target(&two).run_for(0.1);
            assert!(status(&s).free, "Length 2 bars records free too");
            s.button("REC").run_for(0.1);
            assert!(
                s.said().iter().any(|l| l
                    == "looper: Record: the first pass records 2 bars at the tempo you play \
                        from now: REC again closes it, and its length sets the tempo and \
                        starts the song"),
                "{:?}",
                s.said()
            );
            s.run_for(3.5);
            s.button("REC");
            until_phase(&mut s, Phase::Playing, 1.0);
            let st = status(&s);
            let bar = 4 * PPQ as i64;
            assert_eq!(st.ticks, 2 * bar, "2 bars, as Length set");
            s.run_for(0.2);
            let bpm = s.time.info().bpm;
            let want = 2.0 * 4.0 * 60.0 * f64::from(crate::RATE) / st.frames as f64;
            assert!((bpm - want).abs() < 1e-9, "{bpm}, {want}");
            assert!((125.0..140.0).contains(&bpm), "{bpm}");
            let said = format!(
                "looper: Loop 1 is 2 bars at {bpm:.2} BPM, as Length set: the song starts at \
                 that tempo"
            );
            assert!(s.said().contains(&said), "{:?}", s.said());
        });
        // Free, about 3.04 s: 1 bar at 79 or 2 at 158.
        for by_hand in [false, true] {
            on_big_stack(move || {
                let mut s = Session::new();
                s.make_sounds();
                s.press("header", "tab", "Looper");
                s.app.settings.loop_tempo = true;
                if by_hand {
                    s.fx.extend(s.app.set_tempo_by_hand(80.0));
                }
                s.run_for(0.1);
                let free = at(&mut s, "leditor", "bars", 1);
                s.press_target(&free).run_for(0.1);
                s.button("REC");
                s.run_for(3.04);
                s.button("REC");
                until_phase(&mut s, Phase::Playing, 1.0);
                let st = status(&s);
                let bar = 4 * PPQ as i64;
                let r = s
                    .said()
                    .into_iter()
                    .find(|l| l.starts_with("looper: Loop 1 is "))
                    .expect("the reading said");
                if by_hand {
                    assert_eq!(st.ticks, bar, "{r}");
                    assert!(r.contains("1 bar at 7"), "{r}");
                    assert!(!r.contains("close call"), "{r}");
                } else {
                    assert_eq!(st.ticks, 2 * bar, "{r}");
                    assert!(r.contains("2 bars at 15"), "{r}");
                    assert!(r.contains("(a close call: it could be 1 bar at 7"), "{r}");
                    assert!(
                        s.hint().contains("close call"),
                        "on the screen: {}",
                        s.hint()
                    );
                }
            });
        }
    }

    #[test]
    fn when_i_play_starts_the_loop_and_the_song_on_the_first_pad() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.make_sounds();
            s.press("header", "tab", "Looper");
            let starts = at(&mut s, "leditor", "starts", 1);
            assert_eq!(starts.label, "When I play");
            s.press_target(&starts).run_for(0.1);
            let one = at(&mut s, "leditor", "bars", 2);
            s.press_target(&one).run_for(0.1);
            s.button("REC").run_for(0.1);
            assert!(
                s.said().iter().any(|l| l
                    == "looper: Record: armed, the first pass starts when you play \
                        (the first pad you play) (1 bar)"),
                "{:?}",
                s.said()
            );
            let ring = at(&mut s, "ldeck", "ring", 0);
            assert_eq!(ring.label, "Loop 1 ring: cancel");
            // It waits for you, the song stopped, as long as you like.
            s.run_for(3.0);
            let st = status(&s);
            assert_eq!((st.phase, st.listening), (Phase::Armed, true));
            assert_eq!(s.time.info().state, mpfree_transport::State::Stopped);
            // The first pad starts the song and the loop on it.
            s.pad(0, 120, 0.1);
            assert!(
                s.said()
                    .iter()
                    .any(|l| l == "looper: Loop 1 starts on your note, and the song with it"),
                "{:?}",
                s.said()
            );
            assert!(s.time.info().state.rolling(), "the song plays");
            until_phase(&mut s, Phase::Playing, 2.5);
            let bar = 4 * PPQ as i64;
            let st = status(&s);
            assert_eq!(
                (st.origin, st.ticks, st.lead),
                (0, bar, 0),
                "bar 1, one bar"
            );
            let start = {
                let v = s.looper.looper.loop_layer(0, 0).expect("a layer");
                let mut f = vec![[0.0f32; 2]; 4_410];
                v.read(0, &mut f);
                f.iter().fold(0.0f32, |m, x| m.max(x[0].abs()))
            };
            assert!(start > 0.05, "the pad at the loop's start: {start}");

            // The song playing: Loop 2, When I play, and a pad a quarter
            // of a second into a bar (a bar is 2 s at 120 BPM).
            let ring = at(&mut s, "ldeck", "ring", 1);
            s.press_target(&ring).run_for(0.1);
            let starts = at(&mut s, "leditor", "starts", 1);
            s.press_target(&starts).run_for(0.1);
            let one = at(&mut s, "leditor", "bars", 2);
            s.press_target(&one).run_for(0.1);
            let l2 = |s: &Session| s.looper.looper.loop_status(1).unwrap();
            assert!(l2(&s).listening, "{:?}", l2(&s));
            let pos = |s: &Session| s.time.info().position;
            let end = s.t() + 2.5;
            while pos(&s).rem_euclid(bar) < bar / 8 && s.t() < end {
                s.run_periods(1);
            }
            while pos(&s).rem_euclid(bar) >= bar / 8 && s.t() < end {
                s.run_periods(1);
            }
            s.run_for(0.25);
            let line = pos(&s) - pos(&s).rem_euclid(bar);
            s.pad(1, 120, 0.1);
            let end = s.t() + 2.5;
            while l2(&s).phase != Phase::Playing && s.t() < end {
                s.run_periods(1);
            }
            let st = l2(&s);
            assert_eq!(st.phase, Phase::Playing, "Loop 2 plays");
            assert_eq!(
                (st.origin, st.lead),
                (line, 0),
                "on the bar line before the pad"
            );
            assert_eq!(status(&s).phase, Phase::Playing, "Loop 1 plays on");
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
    /// another tab. SHIFT + TRACK MUTE: each pad mutes itself on the
    /// selected track (step 10: its clips' hits; played live it still
    /// sounds); SHIFT + TRACK MUTE again: the pads play.
    #[test]
    fn track_mute_banks_reach_past_16_tracks_and_shift_mutes_the_tracks_pads() {
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

            // SHIFT + TRACK MUTE: the selected track's pads.
            s.make_sounds();
            s.button_with("SHIFT", "TRACK MUTE");
            assert_eq!(s.app.mute_keys(), MuteKeys::Pads);
            let t = s.app.track();
            s.pad(4, 100, 0.05);
            assert_eq!(s.time.pad_mutes(t), Some((1 << 4, 0)), "A5 muted on it");
            assert_eq!(s.mix.pad_mutes(), 0, "no channel muted");
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
            assert!(loud(&mut s, 4) > 0.01, "pad 5 played live still sounds");
            s.button_with("SHIFT", "TRACK MUTE");
            s.pad(4, 100, 0.05);
            s.button_with("SHIFT", "TRACK MUTE");
            assert_eq!(s.time.pad_mutes(t), Some((0, 0)), "and plays again");
        });
    }
}

#[cfg(test)]
mod track_kits {
    //! A kit per track as tracks come and go (the sampler review's
    //! decision 1, and BROWSE's New track with this kit).

    use super::super::{on_big_stack, Session};
    use crate::gui::FromUi;
    use mpfree_time_ui::TimeAction;

    /// A new track given a kit of its own (as + Kit track and BROWSE's New
    /// track with this kit give it one), UNDO, + Track: the new track
    /// plays kit 1, not the kit the track UNDO took away had.
    #[test]
    fn a_track_undo_takes_away_gives_its_kit_back() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.run_for(0.1);
            assert!(s.kits() > 1, "a second kit to give");
            let n = s.time.track_count();
            s.route(FromUi::Time(TimeAction::AddTrack));
            s.run_periods(4);
            assert_eq!(s.time.track_count(), n + 1);
            assert_eq!(s.time.armed(), Some(n));
            // Once it is armed, the screen gives it its kit.
            s.route(FromUi::Kit(1));
            s.run_periods(4);
            assert_eq!(s.track_kits.entry(n), 1);
            assert_eq!(s.kit, 1, "the pads play it");
            // UNDO takes the track away, and its place plays kit 1 again.
            s.route(FromUi::Time(TimeAction::Undo));
            s.run_periods(4);
            assert_eq!(s.time.track_count(), n);
            assert_eq!(s.track_kits.entry(n), 0);
            // + Track: the new track plays kit 1, and the pads with it.
            s.route(FromUi::Time(TimeAction::AddTrack));
            s.run_periods(4);
            assert_eq!(s.time.track_count(), n + 1);
            assert_eq!(s.time.armed(), Some(n));
            assert_eq!(s.track_kits.entry(n), 0);
            assert_eq!(s.kit, 0, "the pads play kit 1");
            assert!(!s.track_kits.set_aside(1), "the kit is the project's again");
        });
    }

    /// A new track given a kit of its own, UNDO, REDO: one UNDO takes the
    /// track and its kit (set aside, out of the list of kits), and one
    /// REDO brings both back.
    #[test]
    fn redo_brings_a_track_back_with_its_kit() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.run_for(0.1);
            assert!(s.kits() > 1, "a second kit to give");
            let n = s.time.track_count();
            s.route(FromUi::Time(TimeAction::AddTrack));
            s.run_periods(4);
            s.route(FromUi::Kit(1));
            s.run_periods(4);
            assert_eq!(s.track_kits.entry(n), 1);
            s.route(FromUi::Time(TimeAction::Undo));
            s.run_periods(4);
            assert_eq!(s.time.track_count(), n);
            assert_eq!(s.track_kits.entry(n), 0);
            assert!(s.track_kits.set_aside(1), "kit 2 waits for REDO");
            s.route(FromUi::Time(TimeAction::Redo));
            s.run_periods(4);
            assert_eq!(s.time.track_count(), n + 1);
            assert_eq!(s.track_kits.entry(n), 1, "back on its kit");
            assert!(!s.track_kits.set_aside(1));
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
            assert_eq!(s.view(), View::Sequencer, "STEP SEQ shows the Steps");
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
            assert_eq!(s.view(), View::Sequencer, "STEP SEQ shows the Steps");
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
            // Held on Beats, the inspector names its notes.
            s.hold_press("steps", "step", "Line step 1");
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
    /// back on PLAY, UNDO takes the take, and the loop row's Arm arms the
    /// loop for REC.
    /// The loop-tracks check (`app_checks.py`): a loop recorded from Play's
    /// loop row lands on Loops A, row 2's number stops it on its bar line,
    /// and its cell brings it back on one.
    #[test]
    fn loop_tracks_write_their_lines() {
        use mpfree_looper::Phase;
        on_big_stack(|| {
            let mut s = Session::new();
            s.run_for(0.1);
            s.arm_loop().settle();
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
            s.arm_loop().settle();
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
            s.play_view("Beats").settle();
            s.play_view("All").settle();
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
            s.arm_loop().settle();
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
                    .clips()
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
            s.play_view("All");
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
                    .clips()
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
            s.play_view("Beats");
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
            s.play_view("Beats");
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
            s.play_view("Beats");
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
            // never `^looper: (the take )?records silence`.
            s.run_for(3.0);
            let n = s.lines.len();
            s.button("STOP").run_for(0.2);
            wrote(&s, n, "looper: loop of ", "cut at STOP");
            assert!(!s
                .lines
                .iter()
                .any(|l| l.starts_with("looper: records silence")
                    || l.starts_with("looper: the take records silence")));
        });
    }
}
