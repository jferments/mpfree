//! Workflows: what a musician does, step by step, through the whole
//! session (the screen and the control side together), each step checked
//! against the engines, not only the screen. The controls catalogue
//! (docs/dev/guides/controls.md) names the workflow each control is in.

#[cfg(test)]
mod tests {
    use super::super::{on_big_stack, Session};
    use crate::app::View;
    use mpfree_chain_ui::{ChainState, ChainView, CHAIN};
    use mpfree_fx::StripId;
    use mpfree_mixer_ui::{MixerView, MIXER};

    // ---------------------------------------------------------- readers

    /// The chain the editor stands for, as the mixer has it (the truth).
    fn engine(s: &mut Session) -> ChainState {
        s.panels
            .chain_state(&s.mix)
            .expect("the chain editor is open")
    }

    /// The chain as the editor draws it.
    fn screen(s: &Session) -> ChainState {
        s.screen
            .ui
            .get::<ChainView>(CHAIN)
            .expect("the chain editor")
            .state()
            .clone()
    }

    fn slots(c: &ChainState) -> Vec<(String, bool)> {
        c.slots.iter().map(|x| (x.name.clone(), x.on)).collect()
    }

    fn names(c: &ChainState) -> Vec<String> {
        c.slots.iter().map(|x| x.name.clone()).collect()
    }

    /// The value the effect itself holds for knob `name` of insert `slot`
    /// on strip `sid`: what the sound is made with.
    fn node_param(s: &Session, sid: StripId, slot: usize, name: &str) -> f32 {
        let n = s
            .mix
            .mixer
            .chain(sid)
            .and_then(|c| c.node(slot))
            .unwrap_or_else(|| panic!("{sid:?} has an insert {slot}"));
        let info = n
            .params()
            .iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("{} has a knob {name}", n.info().name));
        n.param(info.id)
    }

    /// Parameter `id` of insert `slot` of strip `sid`, as the mixer has it.
    fn node_param_id(s: &Session, sid: StripId, slot: usize, id: u16) -> f32 {
        s.mix
            .mixer
            .chain(sid)
            .and_then(|c| c.node(slot))
            .unwrap_or_else(|| panic!("{sid:?} has an insert {slot}"))
            .param(id)
    }

    /// The insert rows a strip shows on the mixer.
    fn strip_inserts(s: &Session, strip: &str) -> Vec<String> {
        let v = s.screen.ui.get::<MixerView>(MIXER).expect("the mixer");
        let st = v.state();
        let row = st
            .strips
            .iter()
            .chain(std::iter::once(&st.master))
            .find(|x| x.name == strip)
            .unwrap_or_else(|| panic!("the mixer shows {strip}"));
        row.inserts.iter().map(|i| i.name.clone()).collect()
    }

    /// The editor draws what the mixer has: the inserts, on or bypassed,
    /// the one picked and its knobs.
    fn agree(s: &mut Session, when: &str) {
        let (t, v) = (engine(s), screen(s));
        assert_eq!(slots(&v), slots(&t), "{when}: the inserts");
        assert_eq!(v.selected, t.selected, "{when}: the insert picked");
        if t.selected.is_some() && v.group == t.group {
            let knobs = |c: &ChainState| -> Vec<(String, String)> {
                c.params
                    .iter()
                    .map(|p| (p.info.name.to_string(), p.text()))
                    .collect()
            };
            assert_eq!(knobs(&v), knobs(&t), "{when}: the knobs");
        }
    }

    fn knob(c: &ChainState, name: &str) -> f32 {
        c.params
            .iter()
            .find(|p| p.info.name == name)
            .unwrap_or_else(|| panic!("{} shows a knob {name}", c.title))
            .value
    }

    // -------------------------------------------------------- workflows

    /// Mixer > Drums "+ Insert" > Compressor: the compressor goes in after
    /// the DJ filter, its knobs and its Q-Links set the effect itself, ON
    /// bypasses it, Move up puts it first, Back shows it on the strip, its
    /// row opens it again and Remove takes it out.
    /// The Mix tab's Effects look (Jay's card, 14 Nov 07:15Z): + FX on a
    /// row adds an effect in the strip's first free slot, a tap picks one
    /// for the Q-Links, a drag along the row moves it, a tap again opens
    /// its chain, and TRACK MIX comes back on Faders.
    #[test]
    fn the_effects_look_adds_picks_turns_and_moves_an_effect() {
        on_big_stack(|| {
            use mpfree_mixer_ui::Look;
            use mpfree_ui::Touch;
            let drums = StripId::Track(0);
            let mut s = Session::new();
            s.press("header", "tab", "Mix");
            s.press("mixer", "look", "Effects");
            assert_eq!(mpfree_mixer_ui::look(&s.screen.ui), Look::Effects);
            assert_eq!(s.hint(), mpfree_mixer_ui::MIXER_HINT_EFFECTS);
            assert_eq!(strip_inserts(&s, "Drums"), ["Filter · DJ"]);

            // + FX: the picker, and the effect lands in slot 2.
            s.press("mixer", "fx-add", "Drums + FX");
            assert_eq!(s.view(), View::Chain);
            s.press("chain", "pick", "Compressor").run_for(0.1);
            assert_eq!(names(&engine(&mut s)), ["Filter · DJ", "Compressor"]);
            s.press("chain", "back", "").run_for(0.1);
            assert_eq!(s.view(), View::Mixer);
            assert_eq!(
                mpfree_mixer_ui::look(&s.screen.ui),
                Look::Effects,
                "back on the look it came from"
            );
            assert_eq!(strip_inserts(&s, "Drums"), ["Filter · DJ", "Compressor"]);

            // A tap picks it: Q-Link 2 is its Ratio.
            s.press("mixer", "fx", "Compressor").run_for(0.3);
            assert_eq!(s.view(), View::Mixer, "picked, not opened");
            assert_eq!(mpfree_mixer_ui::pick(&s.screen.ui), Some((drums, 1)));
            let before = node_param(&s, drums, 1, "Ratio");
            let volume = s.mix.mixer.param(drums, mpfree_fx::ids::mixer::VOLUME);
            s.qlink(1, 4);
            assert!(
                node_param(&s, drums, 1, "Ratio") > before,
                "Q-Link 2 raised the picked compressor's Ratio"
            );
            assert_eq!(
                s.mix.mixer.param(drums, mpfree_fx::ids::mixer::VOLUME),
                volume,
                "not the strip's"
            );

            // Dragged onto slot 1: first in the chain, still picked.
            let at = |s: &mut Session, label: &str| {
                s.control("mixer", "fx", label)
                    .unwrap_or_else(|| panic!("the {label} cell"))
                    .target
                    .centre()
            };
            let (x, y) = at(&mut s, "Compressor");
            let (x0, _) = at(&mut s, "Filter · DJ");
            let mut acts: Vec<crate::gui::FromUi> =
                s.screen.touch(9, Touch::Down, x, y).into_iter().collect();
            acts.extend(s.screen.touch(9, Touch::Move, (x + x0) / 2, y));
            acts.extend(s.screen.touch(9, Touch::Move, x0, y));
            acts.extend(s.screen.touch(9, Touch::Up, x0, y));
            acts.extend(s.screen.take_actions());
            for a in acts {
                s.route(a);
            }
            s.run_for(0.3);
            let order: Vec<String> = (0..2)
                .map(|i| {
                    s.mix
                        .mixer
                        .chain(drums)
                        .unwrap()
                        .node(i)
                        .unwrap()
                        .info()
                        .name
                        .to_string()
                })
                .collect();
            assert_eq!(order, ["Compressor", "DJ filter"], "moved in the mixer");
            assert_eq!(strip_inserts(&s, "Drums"), ["Compressor", "Filter · DJ"]);
            assert_eq!(mpfree_mixer_ui::pick(&s.screen.ui), Some((drums, 0)));
            let before = node_param(&s, drums, 0, "Ratio");
            s.qlink(1, 4);
            assert!(
                node_param(&s, drums, 0, "Ratio") > before,
                "the Q-Links went with it"
            );

            // Again: its chain, on it.
            s.press("mixer", "fx", "Compressor");
            assert_eq!(s.view(), View::Chain);
            assert_eq!(engine(&mut s).selected, Some(0));
            s.press("chain", "back", "").run_for(0.1);

            // TRACK MIX: back to Play; again: the Mix tab on Faders, the
            // Q-Links on the strip again.
            s.button("TRACK MIX / PAD MIX");
            assert_eq!(s.view(), View::Play);
            s.button("TRACK MIX / PAD MIX").run_for(0.1);
            assert_eq!(s.view(), View::Mixer);
            assert_eq!(mpfree_mixer_ui::look(&s.screen.ui), Look::Faders);
            assert_eq!(mpfree_mixer_ui::pick(&s.screen.ui), None);
            assert!(s.control("mixer", "fader", "Drums fader").is_some());
        });
    }

    /// The Mix tab's EQ look (Jay's EQ boards, 15 Nov): + EQ adds a
    /// parametric EQ in the strip's first free slot, a tap on its curve
    /// picks it for the Q-Links, a dot dragged moves its band in the
    /// mixer, the Q-Links follow the band, and a tap again opens it.
    #[test]
    fn the_eq_look_adds_picks_and_shapes_an_eq() {
        on_big_stack(|| {
            use mpfree_fx::ids::eq as q;
            use mpfree_mixer_ui::Look;
            use mpfree_ui::Touch;
            let drums = StripId::Track(0);
            let mut s = Session::new();
            s.press("header", "tab", "Mix");
            s.press("mixer", "look", "EQ");
            assert_eq!(mpfree_mixer_ui::look(&s.screen.ui), Look::Eq);
            assert_eq!(s.hint(), mpfree_mixer_ui::MIXER_HINT_EQ);
            assert_eq!(strip_inserts(&s, "Drums"), ["Filter · DJ"]);

            // + EQ: it lands after the filter, and the row shows its curve.
            s.press("mixer", "eq-add", "Drums + EQ").run_for(0.3);
            assert_eq!(s.view(), View::Mixer, "added in place");
            assert_eq!(strip_inserts(&s, "Drums"), ["Filter · DJ", "EQ"]);
            assert!(s.control("mixer", "eq", "Drums EQ").is_some());

            // A tap on the curve away from a dot picks the EQ.
            let curve = s
                .control("mixer", "eq", "Drums EQ")
                .expect("the curve")
                .target
                .rect;
            let (x, y) = (curve.x + 6, curve.y + 4);
            let mut acts: Vec<crate::gui::FromUi> =
                s.screen.touch(9, Touch::Down, x, y).into_iter().collect();
            acts.extend(s.screen.touch(9, Touch::Up, x, y));
            acts.extend(s.screen.take_actions());
            for a in acts {
                s.route(a);
            }
            s.run_for(0.3);
            assert_eq!(mpfree_mixer_ui::pick(&s.screen.ui), Some((drums, 1)));

            // Band 2's dot dragged up and right: its gain and frequency.
            let r = s.screen.ui.rect_of(mpfree_mixer_ui::MIXER).unwrap();
            let (x, y) = s
                .screen
                .ui
                .get::<mpfree_mixer_ui::MixerView>(mpfree_mixer_ui::MIXER)
                .unwrap()
                .eq_dot_at(r, drums, mpfree_mixer_ui::eq_look::Dot::Band(1))
                .expect("band 2's dot");
            let hz = node_param_id(&s, drums, 1, q::freq(1));
            let db = node_param_id(&s, drums, 1, q::gain(1));
            // A fader moved first, so UNDO must take back the drag, not it.
            s.route(crate::gui::FromUi::Mixer(mpfree_mixer_ui::MixAction::Set {
                strip: drums,
                id: mpfree_fx::ids::mixer::VOLUME,
                value: -4.0,
            }));
            s.run_for(2.0);
            let mut acts: Vec<crate::gui::FromUi> =
                s.screen.touch(9, Touch::Down, x, y).into_iter().collect();
            acts.extend(s.screen.touch(9, Touch::Move, x + 20, y - 10));
            acts.extend(s.screen.touch(9, Touch::Move, x + 40, y - 20));
            acts.extend(s.screen.touch(9, Touch::Up, x + 40, y - 20));
            acts.extend(s.screen.take_actions());
            for a in acts {
                s.route(a);
            }
            s.run_for(0.3);
            assert!(
                node_param_id(&s, drums, 1, q::gain(1)) > 3.0,
                "band 2 raised"
            );
            assert!(node_param_id(&s, drums, 1, q::freq(1)) > hz, "and moved up");
            assert_eq!(s.view(), View::Mixer);
            let (hz2, db2) = (
                node_param_id(&s, drums, 1, q::freq(1)),
                node_param_id(&s, drums, 1, q::gain(1)),
            );

            // UNDO takes the whole drag back (frequency and gain), not the
            // fader; SHIFT + UNDO puts it back.
            s.button("UNDO").run_for(0.2);
            assert!(
                s.said().iter().any(|l| l == "Undid Drums EQ Band 2"),
                "{:?}",
                s.said()
            );
            assert_eq!(node_param_id(&s, drums, 1, q::freq(1)), hz);
            assert_eq!(node_param_id(&s, drums, 1, q::gain(1)), db);
            assert_eq!(
                s.mix.mixer.param(drums, mpfree_fx::ids::mixer::VOLUME),
                -4.0,
                "the fader stays"
            );
            s.button_with("SHIFT", "UNDO").run_for(0.2);
            assert_eq!(node_param_id(&s, drums, 1, q::freq(1)), hz2);
            assert_eq!(node_param_id(&s, drums, 1, q::gain(1)), db2);

            // The Q-Links are band 2's: Q-Link 1 its frequency.
            s.run_for(2.0);
            let hz = node_param_id(&s, drums, 1, q::freq(1));
            s.qlink(0, 4);
            s.run_for(0.2);
            assert!(
                node_param_id(&s, drums, 1, q::freq(1)) > hz,
                "Q-Link 1: band 2's Freq"
            );
            // That turn is the mixer's too: UNDO takes it back.
            s.button("UNDO").run_for(0.2);
            assert!(
                s.said().iter().any(|l| l == "Undid Drums EQ Band 2"),
                "{:?}",
                s.said()
            );
            assert_eq!(node_param_id(&s, drums, 1, q::freq(1)), hz);

            // A tap on the picked EQ again: it opens in the chain editor.
            s.press("mixer", "eq", "Drums EQ");
            assert_eq!(s.view(), View::Chain, "a tap on the picked EQ opens it");
            s.press("chain", "back", "").run_for(0.1);
            assert_eq!(mpfree_mixer_ui::look(&s.screen.ui), Look::Eq);
        });
    }

    /// The chain editor's strip button (Jay's card, 14 Nov 07:15Z): the
    /// strip list over the editor, a tap on a strip opens its chain in
    /// place, ‹ still going back to the mixer.
    #[test]
    fn the_strip_button_opens_another_strips_chain() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("header", "tab", "Mix");
            s.press("mixer", "insert", "Filter · DJ").run_for(0.2);
            assert_eq!(s.view(), View::Chain);
            assert_eq!(engine(&mut s).title, "Drums FX");
            s.press("chain", "strip", "Drums").run_for(0.3);
            assert_eq!(s.hint(), mpfree_chain_ui::STRIPS_HINT);
            assert!(
                s.control("chain", "strips-pick", "Synth").is_some(),
                "the tracks, from the mixer"
            );
            // A strip with effects: its chain on the first.
            s.press("chain", "strips-pick", "Loop").run_for(0.3);
            let t = engine(&mut s);
            assert_eq!((t.title.as_str(), t.selected), ("Loop FX", Some(0)));
            assert_eq!(t.back.as_deref(), Some("Mixer"));
            assert!(t.list.is_none(), "the list shut");
            assert_eq!(screen(&s).title, "Loop FX");
            // One with none: the picker up.
            s.press("chain", "strip", "Loop").run_for(0.3);
            s.press("chain", "strips-pick", "Synth").run_for(0.3);
            assert!(engine(&mut s).picking, "the picker up on an empty chain");
            s.press("chain", "cancel", "Cancel");
            // A group, then Back keeps the strip.
            s.press("chain", "strip", "Synth").run_for(0.3);
            s.press("chain", "strips-group", "Returns").run_for(0.3);
            assert!(
                s.control("chain", "strips-pick", "").is_some(),
                "the returns"
            );
            s.press("chain", "strips-back", "Back").run_for(0.2);
            assert_eq!(engine(&mut s).title, "Synth FX");
            s.press("chain", "back", "").run_for(0.1);
            assert_eq!(s.view(), View::Mixer);
        });
    }

    #[test]
    fn the_pads_tab_effects_row_opens_the_pads_chain_in_mix() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("header", "tab", "Pads");
            s.press("smp-pads-bar", "tab", "Pad").run_for(0.3);
            assert_eq!(s.view(), View::Pads);
            // A pad with no effects: + FX, and the picker up in Mix.
            s.press("smp-pads-list", "row-button", "+ FX").run_for(0.2);
            assert_eq!(s.view(), View::Chain);
            let t = engine(&mut s);
            assert!(t.picking, "the picker up on the pad's empty chain");
            assert_eq!(t.back.as_deref(), Some("Pads"));
            let title = t.title.clone();
            assert!(title.ends_with(" FX"), "{title}");
            s.press("chain", "pick", "Compressor").run_for(0.2);
            assert_eq!(names(&engine(&mut s)), ["Compressor"]);
            // ‹ back to the Pads tab, whose row names the effect.
            s.press("chain", "back", "").run_for(0.3);
            assert_eq!(s.view(), View::Pads);
            assert!(
                s.control("smp-pads-list", "row-button", "Compressor")
                    .is_some(),
                "the row names the pad's effect"
            );
            // Again, with the wheel to the row and a push: the chain on its
            // first effect this time.
            s.wheel(8);
            s.button("DATA DIAL PUSH").run_for(0.2);
            assert_eq!(s.view(), View::Chain, "the push presses the Effects button");
            let t = engine(&mut s);
            assert_eq!((t.title.as_str(), t.selected), (title.as_str(), Some(0)));
            assert!(!t.picking);
        });
    }

    #[test]
    fn an_insert_added_on_the_mixer_is_turned_bypassed_moved_and_removed() {
        on_big_stack(|| {
            let drums = StripId::Track(0);
            let mut s = Session::new();
            s.press("header", "tab", "Mix");
            assert_eq!(s.view(), View::Mixer);
            assert_eq!(strip_inserts(&s, "Drums"), ["Filter · DJ"]);

            // "+ Insert" opens the editor in the mixer's place, picker up.
            s.press("mixer", "add-insert", "Drums + Insert");
            assert_eq!(s.view(), View::Chain);
            assert!(s.control("chain", "cancel", "Cancel").is_some());
            s.press("chain", "pick", "Compressor").run_for(0.1);
            let t = engine(&mut s);
            assert_eq!(t.title, "Drums FX");
            assert_eq!(names(&t), ["Filter · DJ", "Compressor"]);
            assert_eq!(t.selected, Some(1), "the new insert is picked");
            assert!(s.control("chain", "cancel", "Cancel").is_none());
            agree(&mut s, "added");

            // A knob dragged sets the compressor itself.
            let before = node_param(&s, drums, 1, "Threshold");
            s.press("chain", "knob", "Threshold");
            let now = node_param(&s, drums, 1, "Threshold");
            assert_ne!(now, before, "the drag moved Threshold");
            assert!((knob(&engine(&mut s), "Threshold") - now).abs() < 1e-3);
            agree(&mut s, "Threshold dragged");

            // Q-Link 2 is Ratio.
            let before = node_param(&s, drums, 1, "Ratio");
            s.qlink(1, 4);
            assert!(
                node_param(&s, drums, 1, "Ratio") > before,
                "Q-Link 2 raised Ratio"
            );
            agree(&mut s, "Q-Link 2 turned");

            // ON bypasses it and lets it in again.
            s.press("chain", "slot-on", "Compressor ON");
            assert!(!s.mix.mixer.chain(drums).unwrap().is_on(1), "bypassed");
            assert_eq!(slots(&engine(&mut s))[1], ("Compressor".into(), false));
            agree(&mut s, "bypassed");
            s.press("chain", "slot-on", "Compressor ON");
            assert!(s.mix.mixer.chain(drums).unwrap().is_on(1), "back in");
            agree(&mut s, "on again");

            // Move up: first in the chain, still picked.
            s.press("chain", "move-up", "Move up").run_for(0.1);
            let t = engine(&mut s);
            assert_eq!(names(&t), ["Compressor", "Filter · DJ"]);
            assert_eq!(t.selected, Some(0));
            assert!(
                (node_param(&s, drums, 0, "Threshold") - now).abs() < 1e-3,
                "the knobs went with it"
            );
            agree(&mut s, "moved up");

            // Back: the mixer, the strip showing the new order.
            s.press("chain", "back", "");
            assert_eq!(s.view(), View::Mixer);
            s.run_for(0.1);
            assert_eq!(strip_inserts(&s, "Drums"), ["Compressor", "Filter · DJ"]);

            // Its row opens the editor on it.
            s.press("mixer", "insert", "Compressor");
            assert_eq!(s.view(), View::Chain);
            assert_eq!(engine(&mut s).selected, Some(0));
            agree(&mut s, "opened from its row");

            // Remove takes it out of the sound and off the strip.
            s.press("chain", "remove", "Remove").run_for(0.1);
            assert_eq!(names(&engine(&mut s)), ["Filter · DJ"]);
            assert_eq!(s.mix.mixer.chain(drums).unwrap().len(), 1);
            agree(&mut s, "removed");
            s.press("chain", "back", "").run_for(0.1);
            assert_eq!(strip_inserts(&s, "Drums"), ["Filter · DJ"]);
        });
    }

    /// The picker's Cancel adds nothing and leaves the editor on the chain;
    /// Back from there is the mixer as it was.
    #[test]
    fn the_effect_pickers_cancel_adds_nothing() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("header", "tab", "Mix");
            s.press("mixer", "add-insert", "Loop + Insert");
            s.press("chain", "cancel", "Cancel").run_for(0.1);
            assert_eq!(s.view(), View::Chain);
            assert!(
                s.control("chain", "pick", "").is_none(),
                "the picker closed"
            );
            assert_eq!(names(&engine(&mut s)), ["Filter · DJ"]);
            agree(&mut s, "cancelled");
            // "+ Add effect" opens the picker again; Back closes the editor.
            s.press("chain", "add", "+ Add effect");
            // The picker opens on its first group; the delays are in theirs.
            s.press("chain", "section", "Delay and reverb");
            assert!(s.control("chain", "pick", "Delay").is_some());
            s.press("chain", "back", "").run_for(0.1);
            assert_eq!(s.view(), View::Mixer);
            assert_eq!(strip_inserts(&s, "Loop"), ["Filter · DJ"]);
        });
    }

    /// A knob touched, then Macro 1: the macro takes it. Q-LINK steps the
    /// column to the macros, and Q-Link 1 then moves the knob through the
    /// macro, in the effect itself. Holding the macro's card opens the
    /// macros panel, where the knob is the macro's target; its Back is the
    /// editor again.
    #[test]
    fn a_knob_given_to_a_macro_moves_with_the_macro() {
        on_big_stack(|| {
            let drums = StripId::Track(0);
            let mut s = Session::new();
            s.press("header", "tab", "Mix");
            s.press("mixer", "insert", "Filter · DJ");
            s.press("chain", "knob", "Position");
            let at = node_param(&s, drums, 0, "Position");
            s.press("chain", "macro", "Macro 1");
            let m = s.panels.macros_state(&s.mix);
            assert_eq!(m.macros[0].targets.len(), 1, "Macro 1 has the knob");
            assert_eq!(m.macros[0].targets[0].label, "DJ filter position (Drums)");

            s.button("Q-LINK");
            assert_eq!(
                s.control("qlinks", "knob-card", "").map(|c| c.target.label),
                Some("Macro 1".to_string())
            );
            s.qlink(0, 10).run_for(0.1);
            assert!(s.panels.macros_state(&s.mix).macros[0].value > 0.0);
            let moved = node_param(&s, drums, 0, "Position");
            assert_ne!(moved, at, "the macro moved Position in the filter");
            agree(&mut s, "macro turned");

            // Hold the macro's card: the macros panel, on its bank.
            let card = s.control("qlinks", "knob-card", "Macro 1").unwrap();
            let hold = mpfree_ui::Target {
                gesture: mpfree_ui::Gesture::Hold,
                ..card.target.clone()
            };
            s.press_target(&hold);
            assert_eq!(s.view(), View::Macros);
            assert!(s.control("macros", "target", "Macro 1 target 1").is_some());
            s.press("macros", "back", "");
            assert_eq!(s.view(), View::Chain);
            agree(&mut s, "back from the macros panel");
        });
    }

    /// Learn: a knob touched, Learn, then a Q-Link turned: that Q-Link
    /// moves the knob from then on.
    #[test]
    fn learn_puts_a_knob_on_the_next_qlink_turned() {
        on_big_stack(|| {
            let drums = StripId::Track(0);
            let mut s = Session::new();
            s.press("header", "tab", "Mix");
            s.press("mixer", "insert", "Filter · DJ");
            s.press("chain", "knob", "Position");
            s.press("chain", "learn", "Learn");
            let n = s.sent.len();
            s.qlink(2, 4);
            assert!(
                s.sent[n..].iter().any(|a| a == "ChainLearned"),
                "{:?}",
                &s.sent[n..]
            );
            let cards: Vec<String> = s
                .controls()
                .into_iter()
                .filter(|c| c.widget == "qlinks")
                .map(|c| c.target.label)
                .collect();
            assert_eq!(cards[2], "Position", "Q-Link 3 has it: {cards:?}");
            let at = node_param(&s, drums, 0, "Position");
            s.qlink(2, -6).run_for(0.1);
            assert!(
                node_param(&s, drums, 0, "Position") < at,
                "Q-Link 3 lowered it"
            );
            agree(&mut s, "learned and turned");
        });
    }

    /// Sample > FX: the pad's chain first, then a tap on the track's scope
    /// shows the drum track's chain (its DJ filter); an effect added there
    /// is on the Drums strip in the mixer. The header's Sample tab goes
    /// back to the sound editor.
    #[test]
    fn the_sample_tabs_fx_shows_the_pad_then_the_track() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("header", "tab", "Sample");
            s.press("smp-bar", "tab", "FX");
            assert_eq!(s.view(), View::Chain);
            let t = engine(&mut s);
            assert_eq!(t.title, "Pad 5 FX", "the pad's chain first");
            assert!(t.slots.is_empty());
            agree(&mut s, "the pad's chain");

            s.press("chain", "scope", "Drums");
            let t = engine(&mut s);
            assert_eq!(t.title, "Drums FX");
            assert_eq!(names(&t), ["Filter · DJ"]);
            agree(&mut s, "the track's chain");

            s.press("chain", "add", "+ Add effect");
            s.press("chain", "section", "Delay and reverb");
            s.press("chain", "pick", "Delay").run_for(0.1);
            assert_eq!(names(&engine(&mut s)), ["Filter · DJ", "Delay · Digital"]);
            agree(&mut s, "a delay on the track");

            // Back to the pad's chain: still empty.
            s.press("chain", "scope", "Pad 5");
            assert!(engine(&mut s).slots.is_empty());
            agree(&mut s, "the pad's chain again");

            s.press("header", "tab", "Sample");
            assert_eq!(s.view(), View::Sample);
            assert!(
                s.control("smp-bar", "tab", "FX").is_some(),
                "the sound editor's bar"
            );

            s.press("header", "tab", "Mix").run_for(0.1);
            assert_eq!(
                strip_inserts(&s, "Drums"),
                ["Filter · DJ", "Delay · Digital"]
            );
        });
    }

    /// The strip as the mixer screen shows it.
    fn strip(s: &Session, name: &str) -> mpfree_mixer_ui::StripView {
        let st = s
            .screen
            .ui
            .get::<MixerView>(MIXER)
            .expect("the mixer")
            .state();
        st.strips
            .iter()
            .chain(std::iter::once(&st.master))
            .find(|x| x.name == name)
            .cloned()
            .unwrap_or_else(|| panic!("the mixer shows {name}"))
    }

    /// Track Mix: the Drums fader, its level tap, M, S, send A and its
    /// pre/post, and the output picker each set the mixer itself, and the
    /// strip shows what the mixer has.
    #[test]
    fn the_mixer_strip_controls_set_the_mixer() {
        use mpfree_fx::ids::mixer as mx;
        on_big_stack(|| {
            let (drums, synth) = (StripId::Track(0), StripId::Track(1));
            let mut s = Session::new();
            s.press("header", "tab", "Mix");
            let p = |s: &Session, id| s.mix.mixer.param(drums, id);
            let shows = |s: &mut Session, when: &str| {
                s.run_for(0.1);
                let v = strip(s, "Drums");
                let m = &s.mix.mixer;
                assert!(
                    (v.volume_db - m.param(drums, mx::VOLUME)).abs() < 0.05,
                    "{when}: volume"
                );
                assert_eq!(v.mute, m.param(drums, mx::MUTE) > 0.5, "{when}: M");
                assert_eq!(v.solo, m.param(drums, mx::SOLO) > 0.5, "{when}: S");
                assert_eq!(
                    f32::from(v.output),
                    m.param(drums, mx::OUTPUT),
                    "{when}: output"
                );
                let a = v.sends.expect("Drums has sends")[0];
                assert!(
                    (a.db - m.param(drums, mx::send(0))).abs() < 0.05 || a.db < -60.0,
                    "{when}: send A"
                );
                assert_eq!(
                    a.pre,
                    m.param(drums, mx::send_pre(0)) > 0.5,
                    "{when}: send A pre"
                );
            };

            let v0 = p(&s, mx::VOLUME);
            s.press("mixer", "fader", "Drums fader");
            assert!(p(&s, mx::VOLUME) > v0, "the fader went up");
            shows(&mut s, "fader");
            s.press("mixer", "value", "Drums level (to 0 dB)");
            assert_eq!(p(&s, mx::VOLUME), 0.0, "the level tap is 0 dB");
            shows(&mut s, "0 dB");

            s.press("mixer", "mute", "Drums M");
            assert_eq!(p(&s, mx::MUTE), 1.0);
            assert!(!s.mix.mixer.is_audible(drums));
            shows(&mut s, "muted");
            s.press("mixer", "mute", "Drums M");
            assert_eq!(p(&s, mx::MUTE), 0.0);

            s.press("mixer", "solo", "Drums S");
            assert_eq!(p(&s, mx::SOLO), 1.0);
            assert!(
                !s.mix.mixer.is_audible(synth),
                "the synth is quiet while Drums is soloed"
            );
            shows(&mut s, "soloed");
            s.press("mixer", "solo", "Drums S");
            assert!(s.mix.mixer.is_audible(synth));

            let a0 = p(&s, mx::send(0));
            s.press("mixer", "send", "Drums send A");
            assert!(p(&s, mx::send(0)) > a0, "send A went up");
            let pre0 = p(&s, mx::send_pre(0));
            s.press("mixer", "send-pre", "Drums send A pre/post");
            assert_ne!(p(&s, mx::send_pre(0)), pre0, "pre/post switched");
            shows(&mut s, "send A");

            s.press("mixer", "output", "Drums output");
            let choices: Vec<String> = s
                .controls()
                .into_iter()
                .filter(|c| c.target.id == "output-choice")
                .map(|c| c.target.label)
                .collect();
            assert!(choices.iter().any(|c| c == "Sub 1"), "{choices:?}");
            s.press("mixer", "output-choice", "Sub 1");
            assert_eq!(p(&s, mx::OUTPUT), mx::output::bus(0));
            shows(&mut s, "to Sub 1");
        });
    }

    /// Mixer > I/O: the section picked brings its own hint and Q-Links;
    /// every setting can be tapped (none sits under the master strip); a
    /// setting at its end says so on the hint line instead of doing
    /// nothing.
    #[test]
    fn the_io_view_follows_its_section_and_says_why_a_setting_stays() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("header", "io-icon", "");
            assert_eq!(s.view(), View::Mixer);
            let audio_hint = s.hint();
            s.press("mixer", "io-section", "Sync");
            assert_ne!(s.hint(), audio_hint, "Sync's own hint");
            assert!(s.hint().starts_with("Tap a setting"), "{}", s.hint());
            let card = s.control("qlinks", "knob-card", "").map(|c| c.target.label);
            assert_eq!(card.as_deref(), Some("Tempo from"), "Q-Link 1 is Sync's");

            s.press("mixer", "io-section", "CV");
            let mixer = s.screen.ui.rect_of(MIXER).unwrap();
            for c in s.controls().into_iter().filter(|c| c.target.id == "field") {
                let r = c.target.rect;
                assert!(
                    r.right() <= mixer.right() - 150,
                    "{} at {r:?} runs under the master strip",
                    c.target.label
                );
            }
            // CV 1: Off, then Pitch, then Gate (High starts at 5.0 V).
            s.press("mixer", "field", "CV 1: Off");
            s.press("mixer", "field", "CV 1: Pitch");
            let high = s
                .control("mixer", "field", "CV 1: 5.0 V high")
                .expect("Gate's High");
            let tells = s.screen.tells();
            s.press_target(&high.target);
            assert_eq!(s.screen.tells(), tells + 1, "the tap said something");
            assert_eq!(
                s.hint(),
                "CV 1 High is at its highest, 5.0 V high: - or Q-Link 3 lowers it"
            );
            // - lowers it, as the hint says.
            s.button("-");
            assert!(s.control("mixer", "field", "CV 1: 4.9 V high").is_some());
        });
    }

    /// A knob touched, then "+ LFO": a row under the knob says how deep the
    /// LFO goes; its depth drags; its × takes it off again.
    #[test]
    fn an_lfo_on_a_knob_gets_a_row_and_comes_off() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("header", "tab", "Mix");
            s.press("mixer", "insert", "Filter · DJ");
            s.press("chain", "knob", "Resonance");
            assert!(s.control("chain", "depth", "").is_none());
            s.press("chain", "add-lfo", "+ LFO").run_for(0.1);
            let row = s
                .control("chain", "depth", "row 1 depth")
                .expect("the LFO's row");
            let t = engine(&mut s);
            let res = t
                .params
                .iter()
                .find(|p| p.info.name == "Resonance")
                .unwrap();
            assert!(res.mod_range.is_some(), "the LFO moves Resonance");
            s.press_target(&row.target).run_for(0.1);
            s.press("chain", "remove-row", "row 1 ×").run_for(0.1);
            assert!(
                s.control("chain", "depth", "").is_none(),
                "the row came off"
            );
            let t = engine(&mut s);
            let res = t
                .params
                .iter()
                .find(|p| p.info.name == "Resonance")
                .unwrap();
            assert!(res.mod_range.is_none(), "nothing moves Resonance now");
        });
    }

    // ------------------------------------------------ musician workflows

    /// A lane of the Play page's step grid: its pad, steps and speaker.
    fn lane(s: &Session, name: &str) -> (usize, Vec<bool>, bool) {
        let g = s
            .screen
            .ui
            .get::<crate::play_widgets::StepGrid>(crate::play_page::STEPS)
            .expect("the step grid");
        let l = g
            .lanes
            .iter()
            .find(|l| l.name == name)
            .unwrap_or_else(|| panic!("the grid has a {name} lane"));
        (l.pad, l.steps.iter().map(|&v| v > 0).collect(), l.muted)
    }

    /// The steps of `pad`'s row in the clip the grid shows, as the time
    /// engine has it.
    fn clip_row(s: &Session, pad: usize) -> Vec<bool> {
        let c = s.time.steps().clip.expect("the track has a clip");
        c.row(pad as u8).iter().map(Option::is_some).collect()
    }

    fn on(steps: &[bool]) -> Vec<usize> {
        steps
            .iter()
            .enumerate()
            .filter(|(_, &v)| v)
            .map(|(i, _)| i + 1)
            .collect()
    }

    fn hits_on(s: &Session, pad: usize, from: usize) -> usize {
        s.hits.on[from..].iter().filter(|h| h.0 == pad).count()
    }

    /// The mixer: the selected strip's name opens its menu; Freeze takes
    /// the playing loop through the strip's inserts and plays it from
    /// then on, and the frozen strip's menu offers Thaw and Print.
    #[test]
    fn a_strips_menu_freezes_the_playing_loop_and_thaws_it() {
        use mpfree_fx::FreezeState;
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("play-transport", "view", "Beats");
            for k in [1, 5, 9, 13] {
                s.press("steps", "step", &format!("Kick step {k}"));
            }
            s.button("PLAY").run_for(0.1);
            s.press("header", "tab", "Mix");
            s.press("mixer", "strip-name", "Drums");
            s.press("mixer", "strip-name", "Drums");
            s.press("mixer", "menu", "Freeze");
            let said = |s: &Session, what: &str| s.lines.iter().any(|l| l.contains(what));
            assert!(
                said(&s, "Drums: freezing its 4-beat loop"),
                "Freeze says what it takes: {:?}",
                s.lines
            );
            // The take waits for its memory (made on the builder thread,
            // which this session outruns), then for the loop's next start,
            // then records a bar.
            let drums = StripId::Track(crate::mixing::DRUMS);
            let frozen = |s: &Session| {
                matches!(
                    s.mix.mixer.freeze_state(drums),
                    FreezeState::FadingIn | FreezeState::On
                )
            };
            for _ in 0..150 {
                if frozen(&s) {
                    break;
                }
                s.run_for(0.1);
            }
            assert!(frozen(&s), "frozen within 15 s: {:?}", s.lines);
            assert!(said(&s, "Drums: frozen"), "and says so");

            s.press("mixer", "strip-name", "Drums");
            assert!(
                s.control("mixer", "menu", "Print to a new sample")
                    .is_some(),
                "Print is offered"
            );
            s.press("mixer", "menu", "Thaw");
            assert!(said(&s, "Drums: thawed"), "Thaw says so");
            assert!(!frozen(&s), "and the strip plays its live effects");
            s.button("STOP");
        });
    }

    /// Play > Beats: four Kick steps tapped make the pattern, PLAY plays
    /// the kick four times a bar, and a step tapped again comes off.
    #[test]
    fn steps_tapped_on_beats_play_in_time() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("play-transport", "view", "Beats");
            for k in [1, 5, 9, 13] {
                s.press("steps", "step", &format!("Kick step {k}"));
            }
            let (kick, shown, _) = lane(&s, "Kick");
            assert_eq!(
                on(&clip_row(&s, kick)),
                [1, 5, 9, 13],
                "the clip has the taps"
            );
            assert_eq!(on(&shown), [1, 5, 9, 13], "the grid shows them");

            let from = s.hits.on.len();
            s.button("PLAY").run_for(2.0);
            let n = hits_on(&s, kick, from);
            assert!(
                (4..=5).contains(&n),
                "one bar at 120 BPM plays the kick 4 times, not {n}"
            );
            s.button("STOP");

            // A tap on a step that is on picks it (the Q-Links edit it); a
            // second tap takes it off.
            s.press("steps", "step", "Kick step 5");
            assert_eq!(
                on(&clip_row(&s, kick)),
                [1, 5, 9, 13],
                "the first tap picks it"
            );
            let card = s.control("qlinks", "knob-card", "").map(|c| c.target.label);
            assert_eq!(
                card.as_deref(),
                Some("Velocity"),
                "Q-Link 1 is its velocity"
            );
            s.press("steps", "step", "Kick step 5");
            assert_eq!(
                on(&clip_row(&s, kick)),
                [1, 9, 13],
                "the second takes it off"
            );
            assert_eq!(on(&lane(&s, "Kick").1), [1, 9, 13]);
        });
    }

    /// A project opened (as at start, Prefs > At start) has no clip
    /// launched. PLAY then starts the pattern Play's Beats shows, as in a
    /// new project; TRACK MUTE's pad 1 silences it and lets it play again.
    /// A step put on the pattern while nothing plays on its track starts
    /// it at the next bar line. (The 16 Nov device round: the grid's line
    /// moved over steps that never played, and TRACK MUTE had nothing to
    /// mute.)
    #[test]
    fn an_opened_projects_pattern_plays_on_play_and_track_mute_silences_it() {
        on_big_stack(|| {
            let dir =
                std::env::temp_dir().join(format!("mpfree-open-plays-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("a scratch folder");
            let mut s = Session::with_library(&dir);
            s.press("play-transport", "view", "Beats");
            for k in [1, 5, 9, 13] {
                s.press("steps", "step", &format!("Kick step {k}"));
            }
            s.button_with("SHIFT", "BROWSE").run_for(0.5);
            let name = s
                .said()
                .into_iter()
                .find_map(|l| l.strip_prefix("project: saved ").map(str::to_string))
                .expect("saved");
            // BROWSE, Projects, the row: the first tap picks it, or opens
            // it when it is picked already (the save and an open leave it
            // picked), and a second tap opens it. Over unsaved work the
            // browser asks first: an edit counts once the open has settled,
            // 1.5 s of the wall clock (OPEN_SETTLE), so a slow machine asks
            // where a fast one doesn't. The library opens on its own
            // thread, in real time.
            let open = |s: &mut Session| {
                if s.app.view() != View::Browser {
                    s.button("BROWSE");
                }
                s.press("browser", "place", "Projects");
                let is_in = format!("project: {name} is in");
                let opening = format!("project: opening {name}");
                let count = |v: &[String], l: &str| v.iter().filter(|x| *x == l).count();
                let ins = count(&s.lines, &is_in);
                let opens = count(&s.said(), &opening);
                let asks = |s: &mut Session| s.control("browser", "confirm-yes", "").is_some();
                s.press("browser", "row", &name);
                if !asks(s) && count(&s.said(), &opening) == opens {
                    s.press("browser", "row", &name);
                }
                if asks(s) {
                    s.press("browser", "confirm-yes", "");
                }
                let wall = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while count(&s.lines, &is_in) == ins && std::time::Instant::now() < wall {
                    s.run_for(0.05);
                }
                assert!(
                    count(&s.lines, &is_in) > ins,
                    "{name} opened: {:?}",
                    &s.lines[s.lines.len().saturating_sub(10)..]
                );
                s.run_for(0.3);
                s.press("header", "tab", "Play");
                s.press("play-transport", "view", "Beats");
            };
            open(&mut s);
            let kick = s
                .time
                .steps()
                .clip
                .map(|_| 0)
                .expect("the pattern is there");
            assert_eq!(on(&clip_row(&s, kick)), [1, 5, 9, 13]);
            assert!(
                s.time.now_playing(0).is_none(),
                "an opened project launches nothing"
            );

            let from = s.hits.on.len();
            s.button("PLAY").run_for(2.0);
            let n = hits_on(&s, kick, from);
            assert!(
                (4..=5).contains(&n),
                "PLAY plays the pattern: {n} kicks in a bar"
            );
            assert!(
                s.lines
                    .iter()
                    .any(|l| l.contains("launched (the Play page's pattern)")),
                "{:?}",
                &s.lines[s.lines.len().saturating_sub(6)..]
            );

            // TRACK MUTE: pad 1 mutes track 1, and again lets it play.
            s.button("TRACK MUTE");
            s.pad(0, 100, 0.05);
            assert_eq!(s.time.muted(0), Some(true), "pad 1: track 1");
            s.run_for(0.6);
            let from = s.hits.on.len();
            s.run_for(2.0);
            assert_eq!(hits_on(&s, kick, from), 0, "muted, the kick is silent");
            s.pad(0, 100, 0.05);
            assert_eq!(s.time.muted(0), Some(false));
            s.run_for(0.6);
            let from = s.hits.on.len();
            s.run_for(2.0);
            assert!(hits_on(&s, kick, from) >= 3, "unmuted, it plays again");
            s.button("TRACK MUTE");
            s.button("STOP");
            s.run_for(0.2);

            // Opened again and playing with nothing launched (the looper's
            // REC + PLAY, say): a step put on the pattern starts it.
            open(&mut s);
            s.route(crate::gui::FromUi::Time(
                mpfree_time_ui::TimeAction::Transport(mpfree_transport::Command::Play),
            ));
            s.run_for(0.1);
            assert!(
                s.time.now_playing(0).is_some(),
                "PLAY on its own launches it too"
            );
            s.button("STOP");
            s.run_for(0.2);
            s.route(crate::gui::FromUi::Time(
                mpfree_time_ui::TimeAction::StopAll,
            ));
            s.run_for(0.2);
            assert!(s.time.now_playing(0).is_none(), "every clip stopped");
            let label = step_label(&mut s, 3);
            s.press("steps", "step", &label);
            s.run_for(0.1);
            assert!(
                s.time.now_playing(0).is_some(),
                "the step started the pattern"
            );
            let from = s.hits.on.len();
            s.button("PLAY").run_for(2.0);
            assert!(hits_on(&s, kick, from) >= 4, "and it plays");
            s.button("STOP");
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// The label of step `k` (from 1) on the first lane of Play's Beats.
    fn step_label(s: &mut Session, k: usize) -> String {
        s.controls()
            .iter()
            .find(|c| {
                c.widget == "steps"
                    && c.target.id == "step"
                    && c.target.label.ends_with(&format!(" step {k}"))
            })
            .map(|c| c.target.label.clone())
            .expect("a step on Beats")
    }

    /// Recording on Beats into the pattern: a pad hit while REC records
    /// shows on the grid while it records (not only after STOP), the clip
    /// has it after STOP, and UNDO takes the take back out, bringing back
    /// the step REC recorded over.
    #[test]
    fn rec_on_beats_shows_hits_as_they_go_in_and_undo_takes_them_out() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("play-transport", "view", "Beats");
            // A step first: the pattern's clip (REC has nothing to record
            // into on an empty track yet; see the app thread).
            s.press("steps", "step", "Snare step 1");
            let (snare, kick) = (lane(&s, "Snare").0, lane(&s, "Kick").0);
            s.button("REC");
            assert_eq!(s.time.info().state, mpfree_transport::State::Recording);
            s.run_for(0.55);
            s.pad(kick, 100, 0.05);
            s.run_for(0.4);
            assert_eq!(s.time.info().state, mpfree_transport::State::Recording);
            let shown = on(&lane(&s, "Kick").1);
            assert_eq!(shown.len(), 1, "the hit shows while recording: {shown:?}");
            assert_eq!(shown, on(&clip_row(&s, kick)), "as the clip has it");

            s.run_for(1.5);
            s.button("STOP").run_for(0.2);
            assert_eq!(on(&lane(&s, "Kick").1), shown, "kept after STOP");
            assert_eq!(
                on(&clip_row(&s, snare)),
                Vec::<usize>::new(),
                "REC recorded over the snare"
            );

            s.button("UNDO").run_for(0.2);
            assert!(on(&clip_row(&s, kick)).is_empty(), "UNDO took the hit out");
            assert!(on(&lane(&s, "Kick").1).is_empty(), "and the grid shows it");
            assert_eq!(on(&clip_row(&s, snare)), [1], "the snare step is back");
            assert_eq!(on(&lane(&s, "Snare").1), [1]);
        });
    }

    /// The Kick's speaker on Play and Pad 1's M in the pad mixer are one
    /// mute: each shows what the other did.
    #[test]
    fn a_speaker_on_play_and_m_in_the_pad_mixer_are_one_mute() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("play-transport", "view", "Beats");
            let kick = lane(&s, "Kick").0;
            s.press("steps", "mute", "Kick speaker");
            assert!(
                s.mix.play_levels().pad_mutes & (1 << kick) != 0,
                "the speaker muted pad 1"
            );
            assert!(lane(&s, "Kick").2, "drawn muted");

            s.press("header", "tab", "Mix");
            s.press("mixer", "view", "Pads").run_for(0.1);
            assert!(strip(&s, "Pad 1").mute, "the pad mixer shows Pad 1 muted");
            s.press("mixer", "mute", "Pad 1 M").run_for(0.1);
            assert!(!strip(&s, "Pad 1").mute);
            assert_eq!(
                s.mix.play_levels().pad_mutes & (1 << kick),
                0,
                "M let it play"
            );

            s.press("header", "tab", "Play");
            assert_eq!(s.view(), View::Play);
            assert!(!lane(&s, "Kick").2, "the speaker shows it playing");
        });
    }

    /// TRACK MUTE: the pads mute tracks (pad 1 is track 1, playing its
    /// pattern); the hint says so; TRACK MUTE again and the pads play.
    #[test]
    fn track_mute_turns_the_pads_into_track_mutes_and_back() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("play-transport", "view", "Beats");
            for k in [1, 5, 9, 13] {
                s.press("steps", "step", &format!("Kick step {k}"));
            }
            let kick = lane(&s, "Kick").0;
            s.button("PLAY").run_for(0.1);
            s.button("TRACK MUTE");
            assert!(
                s.hint()
                    .starts_with("TRACK MUTE · bank A: pads 1-8 mute tracks 1-8"),
                "{}",
                s.hint()
            );
            let from = s.hits.on.len();
            s.pad(0, 100, 0.05);
            assert!(s.time.state().tracks[0].mute, "pad 1 muted track 1");
            assert_eq!(hits_on(&s, kick, from), 0, "pad 1 didn't play the kick");
            s.run_for(2.0);
            assert_eq!(hits_on(&s, kick, from), 0, "track 1 is quiet");
            s.pad(0, 100, 0.05);
            assert!(!s.time.state().tracks[0].mute, "again: playing");
            s.run_for(2.0);
            assert!(hits_on(&s, kick, from) >= 3, "track 1 plays again");

            s.button("TRACK MUTE");
            assert!(!s.hint().starts_with("TRACK MUTE"), "{}", s.hint());
            let from = s.hits.on.len();
            s.button("STOP");
            s.pad(0, 100, 0.05);
            assert_eq!(hits_on(&s, kick, from), 1, "pad 1 plays the kick");
        });
    }

    /// The Synth tab puts the synth on the pads, and it stays there after
    /// leaving (the header says so) until a kit is picked from the chip's
    /// list. The wheel alone moves the outline and keeps the synth.
    #[test]
    fn the_synth_stays_on_the_pads_until_a_kit_is_picked() {
        on_big_stack(|| {
            let mut s = Session::new();
            let chip = |s: &mut Session| s.control("header", "chip", "").unwrap().target.label;
            assert!(!s.synth_on);
            s.press("header", "tab", "Synth");
            assert_eq!(s.view(), View::Synth);
            assert!(s.synth_on, "the pads play the synth on its tab");
            s.press("header", "tab", "Play");
            assert!(s.synth_on, "and still on Play");
            assert!(chip(&mut s).starts_with("Poly"), "{}", chip(&mut s));
            s.wheel(-1).run_for(0.2);
            assert!(s.synth_on, "the wheel alone moves the outline");
            let c = s.control("header", "chip", "").unwrap().target;
            s.press_target(&c);
            let rows: Vec<_> = s
                .controls()
                .into_iter()
                .filter(|c| c.widget == "kits" && c.target.id == "row")
                .collect();
            s.press_target(&rows[s.kits() - 1].target).run_for(0.2);
            assert!(!s.synth_on, "the chip's list picked a kit");
            assert_eq!(s.kit, s.kits() - 1, "the last kit, just before the synth");
            assert!(chip(&mut s).contains("· Bank"), "{}", chip(&mut s));
        });
    }

    /// Every value a button's light took on the controller while the
    /// session ran for `seconds` (`Session::sent_lights`, as main.rs sends).
    fn lights_over(s: &mut Session, note: u8, seconds: f64) -> Vec<u8> {
        let mut seen = Vec::new();
        let steps = (seconds / 0.05).ceil() as usize;
        for _ in 0..steps {
            s.run_for(0.05);
            let v = s.sent_lights[usize::from(note)];
            if !seen.contains(&v) {
                seen.push(v);
            }
        }
        seen
    }

    /// REC, PLAY and OVERDUB on the MPC as it starts from a fresh stick
    /// (no saved map, Map controls never run): the buttons are the real
    /// ones (73, 82, 80), they record and overdub the loop on Play, and
    /// their lights reach the controller: REC and PLAY blink while REC
    /// waits for PLAY, PLAY stays lit while playing, REC blinks while the
    /// first pass records, OVERDUB blinks while it overdubs, and STOP puts
    /// them out. Jay found these dead and dark on the MPC three times while
    /// tests passed, because the tests made up REC's and OVERDUB's numbers.
    #[test]
    fn rec_play_and_overdub_record_the_loop_and_light_on_a_fresh_mpc() {
        use crate::app::{LIGHT_DARK, LIGHT_ON};
        use mpfree_controller::mpc_one::button;
        use mpfree_looper::Phase;
        on_big_stack(|| {
            let mut s = Session::new();
            assert_eq!(s.button_note("REC"), button::REC, "REC's real number");
            assert_eq!(s.button_note("OVERDUB"), button::OVERDUB);
            assert_eq!(s.button_note("PLAY"), button::PLAY);
            assert_eq!(s.view(), View::Play);
            // REC on Play's All view arms the loop; REC and PLAY blink.
            s.button("REC");
            s.run_for(0.1);
            assert_eq!(s.looper.phase(), Phase::Armed, "REC armed the loop");
            let rec = lights_over(&mut s, button::REC, 1.0);
            assert!(
                rec.contains(&LIGHT_ON) && rec.contains(&LIGHT_DARK),
                "REC blinks while armed: {rec:?}"
            );
            let play = lights_over(&mut s, button::PLAY, 1.0);
            assert!(
                play.contains(&LIGHT_ON) && play.contains(&LIGHT_DARK),
                "PLAY blinks while REC waits for it: {play:?}"
            );
            // PLAY: the first pass records; PLAY lit, REC still blinking.
            s.button("PLAY");
            s.run_for(0.3);
            assert!(s.time.info().state.rolling(), "PLAY rolls the transport");
            assert_eq!(s.looper.phase(), Phase::Recording, "the first pass records");
            assert_eq!(lights_over(&mut s, button::PLAY, 1.0), vec![LIGHT_ON]);
            let rec = lights_over(&mut s, button::REC, 1.0);
            assert!(
                rec.contains(&LIGHT_ON) && rec.contains(&LIGHT_DARK),
                "REC blinks while recording: {rec:?}"
            );
            // Two bars at 120 BPM: the loop then plays, REC goes dark.
            for _ in 0..100 {
                if s.looper.phase() == Phase::Playing {
                    break;
                }
                s.run_for(0.1);
            }
            assert_eq!(s.looper.phase(), Phase::Playing, "the loop plays");
            assert_eq!(lights_over(&mut s, button::REC, 0.7), vec![LIGHT_DARK]);
            // OVERDUB: a layer goes on while OVERDUB blinks; again: off.
            s.button("OVERDUB");
            s.run_for(0.2);
            assert_eq!(s.looper.phase(), Phase::Overdubbing, "OVERDUB overdubs");
            let dub = lights_over(&mut s, button::OVERDUB, 1.0);
            assert!(
                dub.contains(&LIGHT_ON) && dub.contains(&LIGHT_DARK),
                "OVERDUB blinks while overdubbing: {dub:?}"
            );
            s.button("OVERDUB");
            s.run_for(0.2);
            assert_eq!(s.looper.phase(), Phase::Playing, "OVERDUB again: off");
            assert_eq!(lights_over(&mut s, button::OVERDUB, 0.7), vec![LIGHT_DARK]);
            // STOP: the transport stops and PLAY goes dark.
            s.button("STOP");
            s.run_for(0.2);
            assert!(!s.time.info().state.rolling());
            assert_eq!(s.sent_lights[usize::from(button::PLAY)], LIGHT_DARK);
            assert_eq!(s.sent_lights[usize::from(button::REC)], LIGHT_DARK);
        });
    }

    /// REC and OVERDUB on the Looper tab, on a fresh MPC: REC arms the
    /// picked loop and lights; REC again disarms it.
    #[test]
    fn rec_on_the_looper_tab_arms_the_loop_on_a_fresh_mpc() {
        use crate::app::LIGHT_ON;
        use mpfree_controller::mpc_one::button;
        use mpfree_looper::Phase;
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("header", "tab", "Looper");
            assert_eq!(s.view(), View::Looper);
            s.button("REC");
            s.run_for(0.1);
            assert_eq!(s.looper.phase(), Phase::Armed);
            assert!(lights_over(&mut s, button::REC, 1.0).contains(&LIGHT_ON));
            s.button("PLAY");
            s.run_for(0.3);
            assert_eq!(s.looper.phase(), Phase::Recording);
        });
    }

    /// The Looper with no loop yet: its faint tools and uses say why on
    /// the hint line instead of doing nothing.
    #[test]
    fn faint_looper_tools_say_why() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("header", "tab", "Looper");
            let faint: Vec<_> = s
                .controls()
                .into_iter()
                .filter(|c| matches!(c.target.id, "tool" | "use") && !c.target.enabled)
                .collect();
            assert!(!faint.is_empty(), "with no loop, the tools are faint");
            let normal = s.hint();
            for c in faint {
                let n = s.screen.tells();
                s.press_target(&c.target);
                assert_eq!(s.screen.tells(), n + 1, "{} said something", c.target.label);
                assert_ne!(s.hint(), normal, "{} said why", c.target.label);
            }
        });
    }

    /// The tempo tapped on Play: four taps 0.4 s apart make 150 BPM, on
    /// the transport and on the screen.
    #[test]
    fn the_tempo_tapped_on_play_sets_the_transport() {
        on_big_stack(|| {
            let mut s = Session::new();
            for _ in 0..4 {
                s.press("play-transport", "tempo", "");
                s.run_for(0.4 - 4.0 * super::super::session::PERIOD_S);
            }
            s.run_for(1.0);
            let bpm = s.time.info().bpm;
            assert!(
                (bpm - 150.0).abs() < 2.0,
                "tapped 150 BPM, the transport has {bpm}"
            );
            let label = s
                .control("play-transport", "tempo", "")
                .unwrap()
                .target
                .label;
            assert_eq!(
                label,
                format!("{bpm:.1} BPM"),
                "the screen shows the transport's"
            );
        });
    }

    /// The level: the master bar on Play and the mixer's Master fader are
    /// one level, whichever moves it.
    #[test]
    fn the_master_bar_and_the_mixer_master_are_one_level() {
        use mpfree_fx::ids::mixer as mx;
        on_big_stack(|| {
            let mut s = Session::new();
            let before = s.level_db;
            s.press("master", "level", "");
            // A tap on the bar sets the level where it lands (which may
            // be under the -6 dB the level starts at since round 4).
            assert!(
                (s.level_db - before).abs() > 0.05,
                "the bar moved the level"
            );
            let master = s.mix.mixer.param(StripId::Master, mx::VOLUME);
            assert!(
                (master - s.level_db).abs() < 0.05,
                "the mixer's master is {master}"
            );

            s.press("header", "tab", "Mix").run_for(0.1);
            assert!((strip(&s, "Master").volume_db - s.level_db).abs() < 0.05);
            s.press("mixer", "value", "Master level (to 0 dB)")
                .run_for(0.1);
            let lv = s.level_db;
            assert!((s.mix.mixer.param(StripId::Master, mx::VOLUME) - lv).abs() < 0.05);
            s.press("header", "tab", "Play");
            let bar = s.control("master", "level", "").unwrap().target.label;
            assert_eq!(bar, format!("{lv:.1}"), "the bar shows the mixer's level");
        });
    }

    /// SHIFT + the wheel on Play changes the kit: the header names it,
    /// and the pads play it. The wheel alone moves the outline and leaves
    /// the kit alone.
    #[test]
    fn shift_and_the_wheel_on_play_change_the_kit() {
        on_big_stack(|| {
            let mut s = Session::new();
            let chip = |s: &mut Session| s.control("header", "chip", "").unwrap().target.label;
            let first = chip(&mut s);
            assert_eq!(s.kit, 0);
            s.wheel(1).run_for(0.2);
            assert_eq!(s.kit, 0, "the wheel alone keeps the kit");
            assert_eq!(chip(&mut s), first);
            s.wheel_with("SHIFT", 1).run_for(0.2);
            assert_eq!(s.kit, 1, "the next kit plays");
            assert_ne!(chip(&mut s), first, "the header names it");
            assert!(chip(&mut s).ends_with("Bank B"), "{}", chip(&mut s));
            s.wheel_with("SHIFT", -1).run_for(0.2);
            assert_eq!(s.kit, 0);
            assert_eq!(chip(&mut s), first);
        });
    }

    /// The header's kit chip, whatever chips sit beside it.
    fn kit_chip(s: &mut Session) -> mpfree_ui::Target {
        s.controls()
            .into_iter()
            .find(|c| {
                c.widget == "header" && c.target.id == "chip" && c.target.label.contains("· Bank")
            })
            .map(|c| c.target)
            .expect("the kit chip")
    }

    /// The kit chip opens the list of kits over any page: a kit tapped
    /// there loads and the page comes back; Back, the chip again or a tab
    /// close it and keep the kit (Jay's card, 13 Nov 14:57Z).
    #[test]
    fn the_kit_chip_opens_a_list_of_kits_over_the_page() {
        on_big_stack(|| {
            let mut s = Session::new();
            let play = s.page();
            let chip = kit_chip(&mut s);
            s.press_target(&chip);
            assert_eq!(s.page(), crate::gui::KITS_PAGE, "the list is open");
            assert_eq!(s.view(), View::Play, "over Play");
            assert!(s.hint().starts_with("Tap a kit"), "{}", s.hint());
            let rows: Vec<_> = s
                .controls()
                .into_iter()
                .filter(|c| c.widget == "kits" && c.target.id == "row")
                .collect();
            assert_eq!(rows.len(), s.kits() + 1, "every kit, then the synth");
            s.press_target(&rows[2].target).run_for(0.2);
            assert_eq!(s.kit, 2, "the kit tapped plays");
            assert_eq!(s.page(), play, "and Play is back");
            assert!(
                kit_chip(&mut s).label.ends_with("Bank C"),
                "{}",
                kit_chip(&mut s).label
            );

            // Over the mixer: Back keeps the kit and the mixer.
            s.press("header", "tab", "Mix").run_for(0.1);
            let mixer = s.page();
            let chip = kit_chip(&mut s);
            s.press_target(&chip);
            assert_eq!(s.page(), crate::gui::KITS_PAGE);
            s.press("kits-card", "button", "Back");
            assert_eq!((s.page(), s.view()), (mixer.clone(), View::Mixer));
            assert_eq!(s.kit, 2, "Back changed no kit");

            // The chip again closes it.
            let chip = kit_chip(&mut s);
            s.press_target(&chip);
            let chip = kit_chip(&mut s);
            s.press_target(&chip);
            assert_eq!(s.page(), mixer, "the chip closed the list");

            // A tab closes it and opens its page.
            let chip = kit_chip(&mut s);
            s.press_target(&chip);
            s.press("header", "tab", "Arrange");
            assert_eq!(s.view(), View::Arrange);
            assert_ne!(s.page(), crate::gui::KITS_PAGE);
            assert!(!s.hint().starts_with("Tap a kit"), "{}", s.hint());
            assert_eq!(s.kit, 2);

            // The list's last row is the synth, as the wheel's last step.
            let chip = kit_chip(&mut s);
            s.press_target(&chip);
            let synth = s
                .controls()
                .into_iter()
                .rfind(|c| c.widget == "kits" && c.target.id == "row");
            s.press_target(&synth.expect("the synth's row").target)
                .run_for(0.2);
            assert!(s.synth_on, "the pads play the synth");
            assert_eq!(s.view(), View::Arrange);
        });
    }

    /// FULL LEVEL shows a Full level chip; a tap on it turns it off, as
    /// FULL LEVEL again would. SHIFT + FULL LEVEL's Half level chip too.
    #[test]
    fn a_tap_on_the_level_chip_turns_that_level_off() {
        on_big_stack(|| {
            let mut s = Session::new();
            let as_played = s.app.velocity(40);
            let light = s.button_note("FULL LEVEL") as usize;
            for (held, chip, velocity) in
                [(None, "Full level", 127), (Some("SHIFT"), "Half level", 64)]
            {
                match held {
                    Some(h) => s.button_with(h, "FULL LEVEL"),
                    None => s.button("FULL LEVEL"),
                };
                assert_eq!(
                    s.app.velocity(40),
                    velocity,
                    "{chip}: every hit at {velocity}"
                );
                assert_eq!(s.lights[light], crate::app::LIGHT_ON);
                s.press("header", "chip", chip);
                assert_eq!(
                    s.app.velocity(40),
                    as_played,
                    "{chip} off: the pads play as hit"
                );
                assert_eq!(
                    s.lights[light],
                    crate::app::LIGHT_OFF,
                    "FULL LEVEL's light is off"
                );
                assert!(
                    s.control("header", "chip", chip).is_none(),
                    "the {chip} chip went"
                );
            }
        });
    }
}
