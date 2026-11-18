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
            .chain_state(&s.mix, &|t| {
                crate::modpanels::NotesSource::of(&s.app, &s.time, t)
            })
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

    /// Removing an effect shuts its page (Jay, 16 Nov: "If we delete the
    /// effect, its configuration page should not remain open", for any
    /// effect): the chain editor goes back to its list with nothing
    /// picked, rather than opening the next effect's page.
    #[test]
    fn removing_an_effect_shuts_its_page() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("header", "tab", "Mix");
            s.press("mixer", "look", "EQ");
            s.press("mixer", "eq-add", "Drums + EQ").run_for(0.3);
            s.press("mixer", "look", "Effects");
            // The EQ's page open (a first tap picks, a second opens).
            s.press("mixer", "fx", "EQ").run_for(0.2);
            if s.view() != View::Chain {
                s.press("mixer", "fx", "EQ").run_for(0.2);
            }
            assert_eq!(s.view(), View::Chain);
            let e = engine(&mut s);
            assert_eq!((e.selected, e.eq), (Some(1), true), "on the EQ's page");

            // Remove: the EQ's page shuts, and no other effect's opens.
            s.press("chain", "remove", "Remove");
            assert_eq!(screen(&s).selected, None, "shut at once");
            s.run_for(0.3);
            let (e, v) = (engine(&mut s), screen(&s));
            assert_eq!(names(&e), ["Filter · DJ"], "the EQ is gone");
            assert_eq!((e.selected, e.eq), (None, false), "no page in the app");
            assert_eq!((v.selected, v.eq), (None, false), "no page on screen");
            assert_eq!(s.view(), View::Chain, "the chain's list stays");
            assert_eq!(s.hint(), mpfree_chain_ui::CHAIN_HINT, "not the EQ's hint");
            assert!(
                s.control("chain", "remove", "Remove").is_none(),
                "nothing picked to remove"
            );

            // Any effect alike: the filter's page, then Remove.
            s.press("chain", "slot", "Filter · DJ").run_for(0.2);
            assert_eq!(engine(&mut s).selected, Some(0), "on the filter's page");
            s.press("chain", "remove", "Remove").run_for(0.3);
            let (e, v) = (engine(&mut s), screen(&s));
            assert!(names(&e).is_empty(), "the filter is gone");
            assert_eq!(e.selected, None);
            assert_eq!(v.selected, None);
            s.press("chain", "back", "").run_for(0.2);
            assert_eq!(s.view(), View::Mixer);
            assert_eq!(strip_inserts(&s, "Drums"), Vec::<String>::new());
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

    /// What the step inspector on Beats shows now.
    fn inspect(s: &Session) -> crate::play_page::Inspect {
        s.screen
            .ui
            .get::<crate::play_widgets::StepInspector>(crate::play_page::INSPECTOR)
            .expect("the step inspector")
            .inspect
            .clone()
    }

    /// The Q-Link cards' names and values.
    fn cards(s: &Session) -> Vec<String> {
        s.screen
            .ui
            .get::<mpfree_ui::qlink::QLinkColumn>(crate::play_page::QLINKS)
            .expect("the Q-Links")
            .cards
            .iter()
            .map(|c| format!("{} {}", c.name, c.value))
            .collect()
    }

    /// Play > Beats, the step inspector (sequencer mockup A): a step
    /// picked puts it in the throws' place with the step's settings;
    /// its Trig bank (or Q-LINK) gives the Q-Links the condition, ratchet
    /// and nudge, and a turn there reaches the clip; Sound needs the
    /// step's pad held. Its × picks it off, the step kept: the throws
    /// come back and Q-LINK reaches the macros again.
    #[test]
    fn beats_inspects_the_step_picked_and_its_banks_pick_the_q_links() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("play-transport", "view", "Beats");
            for k in [1, 5] {
                s.press("steps", "step", &format!("Kick step {k}"));
            }
            assert!(!s.page().ends_with("· step"), "{}", s.page());
            assert!(s.control("throws", "throw", "").is_some());
            s.press("steps", "step", "Kick step 5");
            assert!(s.page().ends_with("· step"), "{}", s.page());
            assert!(
                s.control("throws", "throw", "").is_none(),
                "the inspector has the throws' place"
            );
            let i = inspect(&s);
            assert_eq!(i.title, "Step 5 · Kick");
            let names: Vec<&str> = i.chips.iter().map(|c| c.0.as_str()).collect();
            assert_eq!(
                names,
                [
                    "Velocity",
                    "Length",
                    "Nudge",
                    "Chance",
                    "Condition",
                    "Ratchet"
                ]
            );
            assert_eq!(i.turned, [true, true, true, true, false, false]);
            assert!(i.locks.is_empty() && i.one && !i.sound);
            let sound = s.control("step-inspector", "bank", "Sound bank").unwrap();
            assert!(!sound.target.enabled, "Sound locks the steps held");

            // Trig: the Q-Links turn the condition, ratchet and nudge.
            s.press("step-inspector", "bank", "Trig bank");
            assert_eq!(inspect(&s).turned, [false, false, true, false, true, true]);
            assert_eq!(
                cards(&s)[..3],
                ["Condition always", "Ratchet off", "Nudge 0 ticks"]
            );
            s.qlink(1, 1);
            let kick = lane(&s, "Kick").0;
            let ratchet = |s: &Session| {
                let c = s.time.steps().clip.expect("the clip");
                c.row(kick as u8)[4].map(|st| st.ratchet)
            };
            assert_eq!(ratchet(&s), Some(2), "the turn reached the clip");
            assert_eq!(inspect(&s).chips[5].1, "2×");
            // Q-LINK steps back to Step.
            s.button("Q-LINK");
            assert_eq!(inspect(&s).bank, crate::play_page::HeldBank::Step);
            assert_eq!(cards(&s)[0], "Velocity 100");
            // × picks it off and keeps it: the throws come back, and
            // Q-LINK reaches the macro banks again.
            s.press("step-inspector", "close", "Close the inspector");
            assert_eq!(on(&clip_row(&s, kick)), [1, 5], "the step stays");
            assert!(!s.page().ends_with("· step"), "{}", s.page());
            assert!(s.control("throws", "throw", "").is_some());
            s.button("Q-LINK");
            let title = s
                .screen
                .ui
                .get::<mpfree_ui::qlink::QLinkColumn>(crate::play_page::QLINKS)
                .map(|q| q.title.clone());
            assert_eq!(title.as_deref(), Some("Q-Links · Macros 1-4"));
        });
    }

    /// The step inspector's locks, Copy and Clear: a lock put on a step
    /// held (STEP SEQ, the Sound bank) shows on the step picked with its
    /// ×, which takes it off (UNDO brings it back); Copy and a step tapped
    /// on the lane copies the step with its lock; Clear takes a step out
    /// with its lock, one take for UNDO.
    #[test]
    fn the_step_inspector_takes_locks_off_copies_and_clears_a_step() {
        use mpfree_play::mpfree_param::{OnTrack, RelAddr};
        on_big_stack(|| {
            let mut s = Session::new();
            s.run_for(0.1);
            // Pad 5, the kit pad with a sample, on the pads as steps.
            s.pad(4, 100, 0.05);
            s.press("play-transport", "view", "Beats");
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
            let name = s
                .screen
                .ui
                .get::<crate::play_widgets::StepGrid>(crate::play_page::STEPS)
                .and_then(|g| g.lanes.iter().find(|l| l.pad == 4).map(|l| l.name.clone()))
                .expect("pad 5's lane");
            // Held, the inspector shows the step held and offers Sound.
            s.pad_edge(4, 100, true);
            s.run_periods(4);
            let i = inspect(&s);
            assert_eq!(i.title, format!("Step 5 · {name} · held"));
            assert!(i.sound);
            s.press("step-inspector", "bank", "Sound bank");
            assert_eq!(inspect(&s).bank, crate::play_page::HeldBank::Sound);
            s.qlink(0, 3);
            s.pad_edge(4, 0, false);
            s.run_periods(4);
            let locked = lock(&s, 4).expect("a lock on step 5");
            assert!(!s.page().ends_with("· step"), "let go: {}", s.page());

            // Picked on the screen: its lock, with the ×.
            s.press("steps", "step", &format!("{name} step 5"));
            let i = inspect(&s);
            assert_eq!(i.locks.len(), 1, "{i:?}");
            assert_eq!(i.locks[0].1, "Tune");
            s.press("step-inspector", "unlock", "Take off lock 1");
            assert_eq!(lock(&s, 4), None, "the × took it off");
            assert!(inspect(&s).locks.is_empty());
            assert_eq!(on(&clip_row(&s, 4)), [1, 5], "the note stays");
            s.button("UNDO");
            assert_eq!(lock(&s, 4), Some(locked), "UNDO brings it back");

            // Copy, then step 9: the note and its lock.
            s.press("step-inspector", "copy", "Copy the step");
            assert!(inspect(&s).copying);
            assert!(s.hint().contains("takes step 5's copy"), "{}", s.hint());
            s.press("steps", "step", &format!("{name} step 9"));
            assert_eq!(on(&clip_row(&s, 4)), [1, 5, 9]);
            assert_eq!(lock(&s, 8), Some(locked), "the copy has the lock");

            // Clear step 9: out, its lock too; UNDO brings both back.
            s.press("steps", "step", &format!("{name} step 9"));
            assert_eq!(inspect(&s).title, format!("Step 9 · {name}"));
            s.press("step-inspector", "clear", "Clear the step");
            assert_eq!(on(&clip_row(&s, 4)), [1, 5]);
            assert_eq!(lock(&s, 8), None);
            assert!(!s.page().ends_with("· step"), "{}", s.page());
            s.button("UNDO");
            assert_eq!(on(&clip_row(&s, 4)), [1, 5, 9]);
            assert_eq!(lock(&s, 8), Some(locked));
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
            // REC readies the notes; PLAY starts the take.
            s.button("REC");
            assert_eq!(s.time.info().state, mpfree_transport::State::Stopped);
            s.button("PLAY");
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

    /// PAD BANK B pages the kit: pad 5 is then the kit's pad B5 (pad 20),
    /// REC records note 20 and the clip plays pad 20 back, while pad 5
    /// (A5) stays as it was.
    #[test]
    fn pad_bank_b_records_and_plays_the_kits_next_pads() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("play-transport", "view", "Beats");
            s.press("steps", "step", "Snare step 1");
            s.button("PAD BANK B/F");
            assert_eq!(s.app.pad_page(), 1, "bank B");
            s.button("REC");
            s.button("PLAY");
            s.run_for(0.55);
            s.pad(4, 100, 0.05);
            s.run_for(0.4);
            s.button("STOP").run_for(0.2);
            assert_eq!(on(&clip_row(&s, 20)).len(), 1, "the take has pad B5");
            assert!(on(&clip_row(&s, 4)).is_empty(), "and not A5");

            let from = s.hits.on.len();
            s.button("PLAY");
            s.run_for(2.2);
            s.button("STOP").run_for(0.1);
            assert!(hits_on(&s, 20, from) > 0, "the clip plays pad B5");
            assert_eq!(hits_on(&s, 4, from), 0, "not A5");
        });
    }

    /// The Pads tab's strip of banks turns the kit's bank as PAD BANK
    /// does: the pads, the log, the kit chip and the tab all go to B, and
    /// a hit on pad 5 is then B5.
    #[test]
    fn the_pads_tabs_strip_puts_bank_b_on_the_pads() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("header", "tab", "Pads");
            let n = s.lines.len();
            s.press(mpfree_sampler_ui::pads_tab::MAP, "bank", "Bank B")
                .run_for(0.1);
            assert_eq!(s.app.pad_page(), 1);
            assert!(
                s.lines[n..].iter().any(|l| l == "pad bank B: pads B1-B16"),
                "{:?}",
                &s.lines[n..]
            );
            assert!(kit_chip(&mut s).label.ends_with("Bank B"));
            assert_eq!(s.screen.sample.pad() / 16, 1, "the tab is on bank B");
            s.pad(4, 100, 0.05);
            assert_eq!(s.screen.sample.pad(), 20, "pad 5 is B5");
            s.press(mpfree_sampler_ui::pads_tab::MAP, "bank", "Bank A")
                .run_for(0.1);
            assert_eq!(s.app.pad_page(), 0);
            assert_eq!(s.screen.sample.pad(), 4, "and back to A5");
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

    /// A kit for each track (the sampler review's decision 1): a kit
    /// picked in the chip's list is the armed track's, arming another
    /// track moves the pads to what that one plays, and Arrange says what
    /// each plays. Track 1 keeps kit 1 throughout.
    #[test]
    fn each_track_keeps_the_kit_picked_for_it() {
        use crate::gui::FromUi;
        use mpfree_time_ui::TimeAction;
        on_big_stack(|| {
            let mut s = Session::new();
            assert!(s.kits() >= 2, "{} kits", s.kits());
            s.route(FromUi::Time(TimeAction::Arm(Some(1))));
            // The arm reaches the app with the next periods.
            s.run_for(0.05);
            assert_eq!(s.app.track(), 1);
            let chip = s.control("header", "chip", "").unwrap().target;
            s.press_target(&chip);
            let rows: Vec<_> = s
                .controls()
                .into_iter()
                .filter(|c| c.widget == "kits" && c.target.id == "row")
                .collect();
            s.press_target(&rows[1].target).run_for(0.2);
            assert_eq!(
                (s.kit, s.track_kits.entry(1)),
                (1, 1),
                "track 2 plays kit 2"
            );
            assert_eq!(s.track_kits.entry(0), 0, "track 1 keeps kit 1");
            s.route(FromUi::Time(TimeAction::Arm(Some(0))));
            // The arm reaches the app with the next periods.
            s.run_for(0.05);
            assert_eq!(s.kit, 0, "the pads follow track 1 to kit 1");
            s.route(FromUi::Time(TimeAction::Arm(Some(1))));
            // The arm reaches the app with the next periods.
            s.run_for(0.05);
            assert_eq!(s.kit, 1, "and track 2 back to kit 2");
            s.press("header", "tab", "Arrange").settle();
            let sounds: Vec<&str> = s.screen.time.tracks[..2]
                .iter()
                .map(|t| t.sound.as_str())
                .collect();
            assert_eq!(sounds, ["Kit", "Kit 2"]);
        });
    }

    /// Opening the Synth tab changes what no track plays (the sequencer
    /// thread's point, 17 Nov 17:43Z): the armed track keeps its kit, and
    /// the hint says how to play the synth on it. Picked from the kit
    /// chip's list, the synth is the track's, and stays after leaving the
    /// tab (the header says so) until a kit is picked there. The wheel
    /// alone moves the outline and keeps the synth.
    #[test]
    fn the_synth_plays_on_a_track_once_picked_until_a_kit_is() {
        on_big_stack(|| {
            let mut s = Session::new();
            let chip = |s: &mut Session| s.control("header", "chip", "").unwrap().target.label;
            assert!(!s.synth_on);
            s.press("header", "tab", "Synth");
            assert_eq!(s.view(), View::Synth);
            assert!(!s.synth_on, "opening the tab leaves the track on its kit");
            assert!(s.hint().contains("pick Synth"), "{}", s.hint());
            s.pick_synth();
            assert!(s.synth_on, "picked, the pads play the synth");
            assert!(s.track_kits.plays_synth(s.app.track()), "the armed track's");
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
    /// ones (73, 82, 80), they record and overdub the loop armed by Play's
    /// Loop view, and
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
            // The Loop view arms the loop; REC arms it to record, and REC
            // and PLAY blink.
            s.press("play-transport", "view", "Loop").settle();
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
            // Free length: REC again closes it on the next bar line (two
            // bars at 120 BPM), the loop then plays and REC goes dark.
            s.run_for(2.0);
            assert_eq!(
                s.looper.phase(),
                Phase::Recording,
                "it records until REC or STOP"
            );
            s.button("REC");
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

    /// STOP with the transport stopped lets go of a loop waiting for PLAY
    /// (the final round's transport check, step 8: on the Looper tab REC
    /// on the playing loop started an overdub that waits for PLAY, so
    /// OVERDUB and PLAY blinked, and STOP twice left them blinking). Also
    /// an armed first pass: STOP disarms it, REC and PLAY go dark.
    #[test]
    fn stop_lets_go_of_a_loop_waiting_for_play() {
        use crate::app::LIGHT_DARK;
        use mpfree_controller::mpc_one::button;
        use mpfree_looper::Phase;
        on_big_stack(|| {
            let mut s = Session::new();
            // Armed on Play's Loop view, then STOP: disarmed, the lights
            // dark.
            s.press("play-transport", "view", "Loop").settle();
            s.button("REC").run_for(0.1);
            assert_eq!(s.looper.phase(), Phase::Armed);
            s.button("STOP").run_for(0.3);
            assert_eq!(s.looper.phase(), Phase::Empty, "STOP disarms");
            assert_eq!(lights_over(&mut s, button::REC, 0.7), vec![LIGHT_DARK]);
            assert_eq!(lights_over(&mut s, button::PLAY, 0.7), vec![LIGHT_DARK]);

            // A loop recorded, then stopped: REC + PLAY, and STOP keeps
            // what was played.
            s.button("REC").run_for(0.1);
            s.button("PLAY").run_for(3.0);
            assert_eq!(s.looper.phase(), Phase::Recording);
            s.button("STOP").run_for(0.3);
            assert!(!s.time.info().state.rolling());
            assert_eq!(s.looper.phase(), Phase::Playing, "STOP kept the take");

            // Step 8: the Looper tab, REC: an overdub waits for PLAY.
            s.button_with("SHIFT", "SAMPLER").run_for(0.2);
            assert_eq!(s.view(), View::Looper);
            s.button("REC").run_for(0.2);
            assert_eq!(s.looper.phase(), Phase::Overdubbing, "REC overdubs a loop");
            let dub = lights_over(&mut s, button::OVERDUB, 1.0);
            assert!(dub.len() > 1, "OVERDUB blinks while it waits: {dub:?}");

            // STOP: the waiting overdub goes, and every light with it.
            s.button("STOP").run_for(0.3);
            assert_eq!(s.looper.phase(), Phase::Playing, "the overdub let go");
            assert!(
                s.said()
                    .iter()
                    .any(|l| l == "looper: STOP: loop 1: the overdub waiting for PLAY is off"),
                "{:?}",
                s.said()
            );
            assert_eq!(lights_over(&mut s, button::OVERDUB, 0.7), vec![LIGHT_DARK]);
            assert_eq!(lights_over(&mut s, button::REC, 0.7), vec![LIGHT_DARK]);
            assert_eq!(lights_over(&mut s, button::PLAY, 0.7), vec![LIGHT_DARK]);
            assert_eq!(s.looper.looper.status().layers, 1, "nothing was added");
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

    /// The Looper with no loop yet: its faint controls (Send, Clear) say
    /// why on the hint line instead of doing nothing.
    #[test]
    fn faint_looper_tools_say_why() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("header", "tab", "Looper");
            let faint: Vec<_> = s
                .controls()
                .into_iter()
                .filter(|c| c.widget == "leditor" && !c.target.enabled)
                .filter(|c| matches!(c.target.id, "send" | "clear"))
                .collect();
            assert_eq!(faint.len(), 2, "with no loop, Send and Clear are faint");
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
            // The pad bank is the app's, not the kit's: still A.
            assert!(chip(&mut s).ends_with("Bank A"), "{}", chip(&mut s));
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
                kit_chip(&mut s).label.ends_with("Bank A"),
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

    /// REC records what is armed (sequencer decision 1): on the home view
    /// the notes are armed, REC makes them ready and PLAY records them; a
    /// take that records keeps the arm where it is; the Loop view arms the
    /// loop, and the ready goes with the arm when it moves.
    #[test]
    fn rec_records_what_is_armed_and_play_starts_a_ready_take() {
        use crate::play_widgets::{PlayTransport, RecChip, RecState};
        use crate::rec_arm::{RecArm, Take, Target};
        use mpfree_looper::Phase;
        use mpfree_transport::State;
        fn chip(s: &Session) -> RecChip {
            s.screen
                .ui
                .get::<PlayTransport>(crate::play_page::TRANSPORT)
                .and_then(|t| t.look.rec.clone())
                .expect("the REC chip")
        }
        fn notes(s: &Session) -> usize {
            s.time.project().tracks[0]
                .slots
                .iter()
                .flatten()
                .map(|c| c.notes.len())
                .sum()
        }
        on_big_stack(|| {
            let mut s = Session::new();
            s.make_sounds();
            assert_eq!(s.view(), View::Play);
            // The home view: the notes are armed.
            assert_eq!(s.app.rec.target, Target::Notes);
            assert_eq!(chip(&s).state, RecState::Armed);
            // REC, stopped: ready, and nothing starts.
            s.button("REC").settle();
            assert_eq!(chip(&s).state, RecState::Ready);
            assert!(s.hint().starts_with("PLAY: record"), "{}", s.hint());
            assert_eq!(s.time.info().state, State::Stopped);
            // STOP lets go.
            s.button("STOP").settle();
            assert_eq!(chip(&s).state, RecState::Armed);
            assert_eq!(s.app.rec.ready, None);
            // REC, then PLAY: the take records (after the count-in, if
            // any).
            s.button("REC").button("PLAY");
            let end = s.t() + 5.0;
            while s.time.info().state != State::Recording && s.t() < end {
                s.run_periods(1);
            }
            assert_eq!(s.time.info().state, State::Recording);
            s.settle();
            assert_eq!(chip(&s).state, RecState::Recording);
            // The Loop view opened while it records doesn't take the arm.
            s.press("play-transport", "view", "Loop").settle();
            assert_eq!(s.app.rec.target, Target::Notes);
            s.pad(0, 100, 0.05);
            s.run_for(0.2);
            s.button("STOP").settle();
            assert!(notes(&s) > 0, "the pad went into the clip");
            assert_eq!(s.looper.phase(), Phase::Empty, "the loop recorded nothing");
            // Opened again with nothing recording, the Loop view arms the
            // loop, and REC arms it.
            s.press("play-transport", "view", "Beats").settle();
            s.press("play-transport", "view", "Loop").settle();
            assert_eq!(s.app.rec.target, Target::Loop);
            assert_eq!(chip(&s).what, "Loop 1");
            s.button("REC").settle();
            assert_eq!(s.looper.phase(), Phase::Armed);
            // (The loop's view reaches the screen every 11 periods.)
            s.run_periods(12).settle();
            assert_eq!(chip(&s).state, RecState::Ready);
            // Beats takes the arm back with its ready: the loop lets go and
            // the notes wait for PLAY.
            s.press("play-transport", "view", "Beats").settle();
            s.run_periods(12).settle();
            assert_eq!(
                s.app.rec,
                RecArm {
                    target: Target::Notes,
                    ready: Some(Take::Replace),
                    ..RecArm::default()
                }
            );
            assert_eq!(s.looper.phase(), Phase::Empty);
            assert_eq!(chip(&s).state, RecState::Ready);
        });
    }

    /// The loop strip on the home view says what REC does: with the notes
    /// armed, that REC records them and the Loop view arms the loop; with
    /// the loop armed, that REC records the pads into it. Once a notes take
    /// is done, the strip's own Record arms the loop for REC.
    #[test]
    fn the_loop_strip_says_what_rec_records_and_its_record_arms_the_loop() {
        use crate::looper_page::{LoopStrip, LOOP_STRIP};
        use crate::rec_arm::Target;
        use mpfree_looper::Phase;
        use mpfree_transport::State;
        fn note(s: &Session) -> String {
            s.screen
                .ui
                .get::<LoopStrip>(LOOP_STRIP)
                .unwrap()
                .note
                .clone()
        }
        on_big_stack(|| {
            let mut s = Session::new();
            let notes =
                "Empty: REC records the notes now. Open the Loop view to record a loop here";
            assert_eq!(note(&s), notes);
            s.press("play-transport", "view", "Loop").settle();
            assert_eq!(note(&s), "Empty: REC records the pads and the synth");
            s.press("play-transport", "view", "Beats").settle();
            s.press("play-transport", "view", "All").settle();
            assert_eq!(note(&s), notes);
            // A notes take keeps the arm when the Loop view opens.
            s.button("REC").button("PLAY");
            let end = s.t() + 5.0;
            while s.time.info().state != State::Recording && s.t() < end {
                s.run_periods(1);
            }
            s.press("play-transport", "view", "Loop").settle();
            assert_eq!(s.app.rec.target, Target::Notes);
            assert_eq!(note(&s), notes);
            s.button("STOP").settle();
            // Then the strip's Record arms the loop, for REC too.
            s.press("play-loop", "button", "Record").settle();
            assert_eq!(s.app.rec.target, Target::Loop);
            assert_eq!(s.looper.phase(), Phase::Armed);
            assert!(s.app.rec.ready.is_none());
        });
    }

    /// UNDO while the notes record throws the take so far away: recording
    /// stops, the music plays on, the clip is as it was, and REDO doesn't
    /// bring it back.
    #[test]
    fn undo_while_recording_throws_the_take_away_and_the_music_plays_on() {
        use mpfree_transport::State;
        fn notes(s: &Session) -> usize {
            s.time.project().tracks[0]
                .slots
                .iter()
                .flatten()
                .map(|c| c.notes.len())
                .sum()
        }
        on_big_stack(|| {
            let mut s = Session::new();
            s.button("REC").button("PLAY");
            let end = s.t() + 5.0;
            while s.time.info().state != State::Recording && s.t() < end {
                s.run_periods(1);
            }
            s.pad(0, 100, 0.05);
            s.run_for(0.3);
            assert!(notes(&s) > 0, "the hit went in as it recorded");
            s.button("UNDO");
            s.run_for(0.3);
            assert_eq!(s.time.info().state, State::Playing, "the music plays on");
            assert_eq!(notes(&s), 0, "the take is gone");
            assert!(
                s.said()
                    .iter()
                    .any(|l| l.starts_with("time: undo: take ") && l.ends_with(" thrown away")),
                "{:?}",
                s.said()
            );
            assert_eq!(
                s.hint(),
                "UNDO: the take is thrown away; the music plays on"
            );
            s.button_with("SHIFT", "UNDO").run_for(0.2);
            assert_eq!(notes(&s), 0, "REDO doesn't bring it back");
            // Not recording, UNDO is the page's again.
            s.button("STOP").run_for(0.2);
            assert_eq!(s.app.rec.ready, None);
        });
    }

    /// The Arm pills and the REC chip's target list (mockups F to H): a
    /// pill tapped arms what it heads, red on what REC records; the chip
    /// opens the list, whose rows arm, whose Pin keeps pages from moving
    /// the arm, and whose Rec song is the grid's; REC ready on a page
    /// without a transport bar shows in the header.
    #[test]
    fn arm_pills_and_the_target_list_move_the_arm_and_pin_it() {
        use crate::looper_page::{LoopStrip, LOOP_STRIP};
        use crate::play_widgets::{ArmPill, StepGrid};
        use crate::rec_arm::Target;
        use mpfree_looper::Phase;
        fn pills(s: &Session) -> (Option<ArmPill>, Option<ArmPill>) {
            (
                s.screen
                    .ui
                    .get::<StepGrid>(crate::play_page::STEPS)
                    .unwrap()
                    .arm,
                s.screen.ui.get::<LoopStrip>(LOOP_STRIP).unwrap().arm,
            )
        }
        on_big_stack(|| {
            let mut s = Session::new();
            assert_eq!(pills(&s), (Some(ArmPill::Armed), Some(ArmPill::Arm)));
            // The armed pill says what REC records.
            s.press("steps", "arm", "Armed").settle();
            assert!(s.hint().starts_with("REC records Track 1"), "{}", s.hint());
            // The strip's Arm arms the loop; REC then readies it.
            s.press("play-loop", "arm", "Arm").settle();
            assert_eq!(s.app.rec.target, Target::Loop);
            assert_eq!(pills(&s), (Some(ArmPill::Arm), Some(ArmPill::Armed)));
            s.button("REC");
            s.run_periods(12).settle();
            assert_eq!(s.looper.phase(), Phase::Armed);
            assert_eq!(pills(&s).1, Some(ArmPill::Ready));
            // The pattern's Arm takes the arm back, the ready with it.
            s.press("steps", "arm", "Arm").settle();
            s.run_periods(12).settle();
            assert_eq!(s.app.rec.target, Target::Notes);
            assert_eq!(pills(&s), (Some(ArmPill::Ready), Some(ArmPill::Arm)));
            // REC ready shows in the header on a page without the bar.
            s.press("header", "tab", "Mix").settle();
            assert!(
                s.controls().iter().any(|c| c.widget == "header"
                    && c.target.id == "chip"
                    && c.target.label.starts_with("READY")),
                "the header's REC chip"
            );
            s.press("header", "tab", "Play").settle();
            s.button("STOP").settle();
            // The chip opens the list.
            let chip = s.control("play-transport", "rec", "").unwrap().target;
            s.press_target(&chip);
            s.settle();
            assert_eq!(s.screen.ui.page(), crate::rec_list::REC_PAGE);
            // Pin: Beats and the Loop view leave the arm where it is.
            s.press("rec-list", "switch", "Pin the arm").settle();
            assert!(s.app.rec.pinned);
            s.press("rec-list", "row", "Looper · Loop 1").settle();
            assert_eq!(s.app.rec.target, Target::Loop, "a row arms, pinned or not");
            s.press("rec-list", "close", "Close").settle();
            assert_ne!(s.screen.ui.page(), crate::rec_list::REC_PAGE);
            s.press("play-transport", "view", "Beats").settle();
            assert_eq!(s.app.rec.target, Target::Loop, "pinned: Beats leaves it");
            // Rec song is the grid's.
            let chip = s.control("play-transport", "rec", "").unwrap().target;
            s.press_target(&chip);
            s.settle();
            assert_eq!(s.screen.ui.page(), crate::rec_list::REC_PAGE);
            s.press("rec-list", "switch", "Rec song").settle();
            assert!(s.time.state().song_rec, "Rec song on");
            s.press("rec-list", "switch", "Pin the arm").settle();
            assert!(!s.app.rec.pinned);
            // A view picked closes it.
            s.press("play-transport", "view", "All").settle();
            assert_eq!(s.screen.ui.page(), "Play");
        });
    }

    /// SHIFT + Arm: REC records the pads into a clip and a loop in one
    /// pass. The chip names both; REC readies the notes and arms the loop,
    /// PLAY starts both after the count-in, REC closes both, and a plain
    /// tap on one pill keeps only that one.
    #[test]
    fn shift_arm_records_the_notes_and_a_loop_in_one_pass() {
        use crate::looper_page::{LoopStrip, LOOP_STRIP};
        use crate::play_widgets::{ArmPill, PlayTransport, RecState, StepGrid};
        use crate::rec_arm::Target;
        use mpfree_looper::Phase;
        use mpfree_transport::State;
        fn pills(s: &Session) -> (Option<ArmPill>, Option<ArmPill>) {
            (
                s.screen
                    .ui
                    .get::<StepGrid>(crate::play_page::STEPS)
                    .unwrap()
                    .arm,
                s.screen.ui.get::<LoopStrip>(LOOP_STRIP).unwrap().arm,
            )
        }
        fn chip(s: &Session) -> crate::play_widgets::RecChip {
            s.screen
                .ui
                .get::<PlayTransport>(crate::play_page::TRANSPORT)
                .and_then(|t| t.look.rec.clone())
                .expect("the REC chip")
        }
        fn notes(s: &Session) -> usize {
            s.time.project().tracks[0]
                .slots
                .iter()
                .flatten()
                .map(|c| c.notes.len())
                .sum()
        }
        on_big_stack(|| {
            let mut s = Session::new();
            // SHIFT + the strip's Arm: both are armed.
            s.button_edge("SHIFT", true);
            s.press("play-loop", "arm", "Arm").settle();
            s.button_edge("SHIFT", false);
            assert!(s.app.rec.both, "{:?}", s.app.rec);
            assert_eq!(pills(&s), (Some(ArmPill::Armed), Some(ArmPill::Armed)));
            assert!(chip(&s).what.ends_with(" + Loop 1"), "{:?}", chip(&s));
            // Beats finds the notes armed already: both stay.
            s.press("play-transport", "view", "Beats").settle();
            s.press("play-transport", "view", "All").settle();
            assert!(s.app.rec.both);
            // A bar's count-in: the loop waits for it too.
            s.route(crate::gui::FromUi::Time(
                mpfree_time_ui::TimeAction::Transport(mpfree_transport::Command::SetCountIn(1)),
            ));
            s.run_periods(2);
            // REC: the notes ready, the loop armed to wait for PLAY.
            s.button("REC");
            s.run_periods(12).settle();
            assert!(s.app.rec.ready.is_some());
            assert_eq!(s.looper.phase(), Phase::Armed);
            assert_eq!(chip(&s).state, RecState::Ready);
            assert_eq!(pills(&s), (Some(ArmPill::Ready), Some(ArmPill::Ready)));
            // PLAY: both record once the count-in is over.
            s.button("PLAY");
            let end = s.t() + 6.0;
            let mut counted = false;
            while (s.time.info().state != State::Recording || s.looper.phase() != Phase::Recording)
                && s.t() < end
            {
                s.run_periods(1);
                if s.time.info().state == State::CountIn {
                    counted = true;
                    assert_ne!(
                        s.looper.phase(),
                        Phase::Recording,
                        "the loop doesn't record the count-in"
                    );
                }
            }
            assert!(counted, "a bar's count-in");
            assert_eq!(s.time.info().state, State::Recording);
            assert_eq!(s.looper.phase(), Phase::Recording, "the loop's first pass");
            assert_eq!(chip(&s).state, RecState::Recording);
            s.pad(0, 100, 0.05);
            s.run_for(1.0);
            // A tap while both record says to finish first.
            s.press("steps", "arm", "Rec").settle();
            assert!(s.app.rec.both);
            assert!(
                s.hint().starts_with("Finish the take first"),
                "{}",
                s.hint()
            );
            // REC: the notes punch out, the loop's pass closes on the bar.
            s.button("REC");
            let end = s.t() + 6.0;
            while s.looper.phase() != Phase::Playing && s.t() < end {
                s.run_periods(1);
            }
            assert_eq!(s.time.info().state, State::Playing, "the music plays on");
            assert_eq!(s.looper.phase(), Phase::Playing, "the loop plays");
            assert!(notes(&s) > 0, "the pad went into the clip");
            s.button("STOP").settle();
            // A plain tap on the pattern's pill keeps only the notes.
            s.press("steps", "arm", "Armed").settle();
            assert!(!s.app.rec.both);
            assert_eq!(s.app.rec.target, Target::Notes);
            assert_eq!(pills(&s), (Some(ArmPill::Armed), Some(ArmPill::Arm)));
        });
    }

    /// Arrange's bar: its Rec is REC, so it readies the notes and its Play
    /// starts the take (its Stop lets go); its chip says what REC records,
    /// and names the loop once the Looper tab has armed it.
    #[test]
    fn arranges_bar_rec_and_play_record_what_is_armed() {
        use crate::rec_arm::{Take, Target};
        use mpfree_looper::Phase;
        use mpfree_transport::State;
        fn chip(s: &mut Session) -> String {
            s.control("transport", "chip", "")
                .expect("the REC chip on Arrange's bar")
                .target
                .label
        }
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("header", "tab", "Arrange").settle();
            assert!(chip(&mut s).starts_with("REC → "), "{}", chip(&mut s));
            // Rec readies the notes; nothing starts.
            s.press("transport", "rec", "Rec").settle();
            assert_eq!(s.app.rec.ready, Some(Take::Replace));
            assert_eq!(s.time.info().state, State::Stopped);
            assert!(chip(&mut s).starts_with("READY"), "{}", chip(&mut s));
            // Stop lets go.
            s.press("transport", "stop", "Stop").settle();
            assert_eq!(s.app.rec.ready, None);
            assert_eq!(s.time.info().state, State::Stopped);
            // Rec, then Play: the take records.
            s.press("transport", "rec", "Rec").settle();
            s.press("transport", "play", "Play");
            let end = s.t() + 5.0;
            while s.time.info().state != State::Recording && s.t() < end {
                s.run_periods(1);
            }
            assert_eq!(s.time.info().state, State::Recording);
            assert_eq!(s.app.rec.ready, None);
            s.press("transport", "stop", "Stop").settle();
            assert_eq!(s.time.info().state, State::Stopped);
            // The Looper tab arms the loop: Arrange's chip names it, and
            // Rec arms the loop to record.
            s.press("header", "tab", "Looper").settle();
            s.press("header", "tab", "Arrange").settle();
            assert_eq!(s.app.rec.target, Target::Loop);
            assert_eq!(chip(&mut s), "REC → Loop 1");
            s.press("transport", "rec", "Rec").settle();
            assert_eq!(s.looper.phase(), Phase::Armed);
        });
    }

    /// Loop tracks (loop-tracks.md, part 2): a loop recorded on Play's
    /// Loop view lands on Loops A, row 1, playing. Row 2's scene stops it
    /// on the bar line (Loops A has nothing in row 2), and the next loop
    /// lands there, in the scene's row. Launching row 1's cell switches the
    /// two on one bar line; Stop all turns both off; a loop cleared takes
    /// its cell with it.
    #[test]
    fn a_recorded_loop_lands_on_a_loop_track_and_launches_like_a_clip() {
        use crate::gui::FromUi;
        use mpfree_looper::{Command, Phase};
        use mpfree_time_ui::TimeAction;
        /// The first line from `from` on starting with `prefix`, running
        /// the session up to `seconds` for it.
        fn line(s: &mut Session, from: usize, prefix: &str, seconds: f64) -> String {
            let end = s.t() + seconds;
            loop {
                if let Some(l) = s.lines[from..].iter().find(|l| l.starts_with(prefix)) {
                    return l.clone();
                }
                assert!(
                    s.t() < end,
                    "no line like {prefix:?}: {:?}",
                    &s.lines[s.lines.len().saturating_sub(12)..]
                );
                s.run_periods(4);
            }
        }
        fn until(s: &mut Session, n: usize, phase: Phase) {
            let end = s.t() + 6.0;
            while s.looper.loop_status(n).map(|l| l.phase) != Some(phase) && s.t() < end {
                s.run_periods(4);
            }
            assert_eq!(s.looper.loop_status(n).map(|l| l.phase), Some(phase));
        }
        fn on(s: &Session, n: usize) -> bool {
            s.looper
                .loop_status(n)
                .is_some_and(|l| l.on && l.cue.is_none())
        }
        /// The bar of a "looper: loop 1 on from bar 5" line.
        fn bar(l: &str) -> u32 {
            l.rsplit(' ').next().and_then(|b| b.parse().ok()).expect(l)
        }
        on_big_stack(|| {
            let mut s = Session::new();
            s.make_sounds();
            s.press("play-transport", "view", "Loop").settle();
            let from = s.lines.len();
            s.button("REC").button("PLAY");
            until(&mut s, 0, Phase::Recording);
            // A pad's tone in the first pass, for its wave.
            s.pad(0, 120, 0.3);
            s.run_for(1.7);
            s.button("REC");
            until(&mut s, 0, Phase::Playing);
            line(&mut s, from, "time: Loop 1 on Loops A, row 1, playing", 1.0);
            let p = s.time.project();
            assert_eq!((p.tracks.len(), p.tracks[8].name.as_str()), (9, "Loops A"));
            let cell = p.clip(8, 0).expect("its cell");
            assert_eq!((cell.name.as_str(), cell.looped), ("Loop 1", Some(0)));
            assert_eq!(cell.length, s.looper.loop_status(0).unwrap().ticks);
            assert_eq!(s.time.state().launch(8).map(|l| l.slot), Some(0));
            assert!(on(&s, 0));
            s.run_periods(2);
            assert_eq!(s.screen.time.tracks[8].sound, "Pads");
            // Part 3: the cell draws the loop's wave once the looper has
            // made it, and Play's strip names the loop by its place.
            s.run_for(1.5);
            assert!(
                s.screen.time.loops.loops[0].wave.iter().any(|&p| p > 0),
                "{:?}",
                s.screen.time.loops.loops[0]
            );
            let cell = mpfree_time_ui::view::cell_of(&s.screen.time, 8, 0);
            assert!(cell.wave.is_some_and(|w| w.iter().any(|&p| p > 0)));
            let strip = s
                .screen
                .ui
                .get::<crate::looper_page::LoopStrip>(crate::looper_page::LOOP_STRIP)
                .unwrap();
            assert!(
                strip.right_wide.starts_with("Loops A · Loop 1 · row 1 · "),
                "{}",
                strip.right_wide
            );
            // Show in Clips on the Looper tab: the Arrange page, with Loops
            // A's page of tracks in view.
            s.press("header", "tab", "Looper").settle();
            s.press("leditor", "send", "Send").settle();
            s.press("leditor", "send-to", "Show in Clips").settle();
            assert_eq!(s.view(), View::Arrange);
            assert_eq!(s.screen.time_page.first_track, 8);
            s.press("header", "tab", "Play").settle();

            // Row 2's scene: Loops A has nothing there, so loop 1 stops on
            // the bar line.
            let from = s.lines.len();
            s.route(FromUi::Time(TimeAction::LaunchScene(1)));
            line(&mut s, from, "looper: loop 1 off from bar ", 3.0);
            s.run_periods(2);
            assert!(!on(&s, 0) && s.time.state().launch(8).is_none());

            // The next loop records into the scene's row on Loops A.
            s.looper.select(1);
            let from = s.lines.len();
            s.button("REC");
            until(&mut s, 1, Phase::Recording);
            s.run_for(2.0);
            s.button("REC");
            until(&mut s, 1, Phase::Playing);
            line(&mut s, from, "time: Loop 2 on Loops A, row 2, playing", 1.0);
            assert!(on(&s, 1) && !on(&s, 0));

            // Row 1's cell: loop 1 comes in and loop 2 goes, on one line.
            let from = s.lines.len();
            s.route(FromUi::Time(TimeAction::Launch { track: 8, slot: 0 }));
            let one = line(&mut s, from, "looper: loop 1 on from bar ", 3.0);
            let two = line(&mut s, from, "looper: loop 2 off from bar ", 0.1);
            assert_eq!(bar(&one), bar(&two), "{one} / {two}");
            s.run_periods(2);
            assert!(on(&s, 0) && !on(&s, 1));
            assert_eq!(s.time.state().launch(8).map(|l| l.slot), Some(0));

            // Stop all: off on the next line.
            let from = s.lines.len();
            s.route(FromUi::Time(TimeAction::StopAll));
            line(&mut s, from, "looper: loop 1 off from bar ", 3.0);
            s.run_periods(2);
            assert!(!on(&s, 0) && !on(&s, 1));

            // A loop cleared: its cell empties.
            s.looper.select(0);
            let from = s.lines.len();
            s.looper.command(Command::Clear);
            line(
                &mut s,
                from,
                "time: Loop 1 cleared: its cells are empty",
                2.0,
            );
            assert!(s.time.project().clip(8, 0).is_none());
            assert_eq!(s.time.project().clip(8, 1).and_then(|c| c.looped), Some(1));
            assert_eq!(s.time.project().check(), Ok(()));
        });
    }

    /// A MIDI file in BROWSE's Patterns place (an MPC drum export, on its
    /// pad notes): the row tapped twice loads it, and its track lands on the
    /// armed track's first empty slot with each note on its pad; UNDO on the
    /// Clips page takes it away.
    #[test]
    fn a_midi_file_from_browse_lands_on_its_pads_and_undo_takes_it() {
        use mpfree_midi::msg::Msg;
        use mpfree_midi::smf::{Smf, SmfTrack};
        on_big_stack(|| {
            let dir = std::env::temp_dir().join(format!("mpfree-midi-file-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("a scratch folder");
            let mut s = Session::with_library(&dir);
            let on = |note: u8| Msg::NoteOn {
                ch: 9,
                note,
                vel: 100,
            };
            let off = |note: u8| Msg::NoteOff {
                ch: 9,
                note,
                vel: 64,
            };
            let file = Smf {
                name: "Beat".into(),
                tempos: vec![(0, 90.0)],
                meters: vec![(0, 4, 4)],
                tracks: vec![SmfTrack {
                    name: "MPC drums".into(),
                    // C#2 is pad 1, C2 pad 2, and E3 no pad.
                    events: vec![
                        (0, on(37)),
                        (240, off(37)),
                        (960, on(36)),
                        (1200, off(36)),
                        (1920, on(52)),
                        (2000, off(52)),
                    ],
                }],
                end: 3840,
            };
            let patterns = s
                .library
                .as_ref()
                .expect("a library")
                .root()
                .join("Patterns");
            std::fs::create_dir_all(&patterns).unwrap();
            std::fs::write(patterns.join("Beat.mid"), file.bytes()).unwrap();
            s.button("BROWSE");
            s.press("browser", "place", "Patterns");
            // It is read on its own thread, in real time.
            let said = |s: &Session| {
                s.said()
                    .into_iter()
                    .filter(|l| l.starts_with("MIDI file Beat.mid"))
                    .collect::<Vec<_>>()
            };
            let wait = |s: &mut Session, secs: u64| {
                let wall = std::time::Instant::now() + std::time::Duration::from_secs(secs);
                while said(s).is_empty() && std::time::Instant::now() < wall {
                    s.run_for(0.05);
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            };
            // The first tap picks the row, or loads it when it is picked
            // already (the only row is); then a second tap loads it.
            s.press("browser", "row", "Beat.mid");
            wait(&mut s, 1);
            if said(&s).is_empty() {
                s.press("browser", "row", "Beat.mid");
                wait(&mut s, 5);
            }
            let lines = said(&s);
            assert_eq!(lines.len(), 1, "loaded once: {lines:?}");
            let line = lines[0].clone();
            assert_eq!(
                line,
                "MIDI file Beat.mid: 1 clip with 2 notes, from slot 1 of Track 1 \
                 (UNDO takes them away); its tempo 90 BPM; left out: 1 note no pad plays"
            );
            let clip = s.time.project().tracks[0].slots[0].clone().expect("a clip");
            assert_eq!(clip.name, "MPC drums");
            let pads: Vec<(i64, u8)> = clip.notes.iter().map(|n| (n.tick, n.note)).collect();
            assert_eq!(pads, [(0, 0), (960, 1)]);
            assert_eq!(clip.length, 3840);
            s.button("BROWSE");
            s.press("header", "tab", "Arrange");
            s.button("UNDO");
            s.run_for(0.1);
            assert!(
                s.time.project().tracks[0].slots[0].is_none(),
                "UNDO took it away: {:?} / {:?}",
                &s.lines[s.lines.len().saturating_sub(8)..],
                s.said().iter().rev().take(6).collect::<Vec<_>>()
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// Loop tracks part 4 (loop-tracks.md): SHIFT + "+ Track" adds Loops A,
    /// and a tap on its row 2 cell arms it: the loop armed for REC, an
    /// empty loop picked, the chip naming the cell. REC and PLAY record the
    /// first pass, REC closes it, and it plays in that cell. With row 1's
    /// cell armed, OVERDUB layers the loop the track plays. The notes armed
    /// let go of the cell. Resample, the track's sound line stepped twice,
    /// records the loops as heard into the next cell armed.
    #[test]
    fn a_loop_records_into_the_cell_tapped_on_the_grid() {
        use crate::gui::FromUi;
        use crate::rec_arm::Target;
        use mpfree_looper::Phase;
        use mpfree_time_ui::TimeAction;
        fn until(s: &mut Session, n: usize, phase: Phase) {
            let end = s.t() + 6.0;
            while s.looper.loop_status(n).map(|l| l.phase) != Some(phase) && s.t() < end {
                s.run_periods(4);
            }
            assert_eq!(s.looper.loop_status(n).map(|l| l.phase), Some(phase));
        }
        fn chip(s: &mut Session) -> String {
            s.control("transport", "chip", "")
                .expect("the REC chip on Arrange's bar")
                .target
                .label
        }
        /// Whether a line from `from` on is `want`, running the session up
        /// to a second for it.
        fn said(s: &mut Session, from: usize, want: &str) -> bool {
            let end = s.t() + 1.0;
            while !s.lines[from..].iter().any(|l| l == want) && s.t() < end {
                s.run_periods(4);
            }
            s.lines[from..].iter().any(|l| l == want)
        }
        fn cell(s: &Session, slot: usize) -> String {
            let c = mpfree_time_ui::view::cell_of(&s.screen.time, 8, slot);
            format!("{} / {} / {}", c.name, c.detail, c.foot)
        }
        on_big_stack(|| {
            let mut s = Session::new();
            s.make_sounds();
            s.press("header", "tab", "Arrange").settle();
            s.button_edge("SHIFT", true);
            s.press("clips", "add-track", "+ Track").settle();
            s.button_edge("SHIFT", false);
            let p = s.time.project();
            assert_eq!(p.tracks.len(), 9);
            assert_eq!(p.tracks[8].loops(), Some(mpfree_seq::LoopInput::Pads));
            assert_eq!(s.time.armed(), Some(0), "the pads stay on Track 1");
            s.press("clips", "pager", "Tracks after").settle();
            assert_eq!(s.screen.time.tracks[8].sound, "Pads");

            // Its row 2 cell, tapped: armed.
            s.press("clips", "slot", "Loops A slot 2").settle();
            assert_eq!(s.time.loop_cell(), Some((8, 1)));
            assert_eq!(s.app.rec.target, Target::Loop);
            assert_eq!(chip(&mut s), "REC → Loops A · row 2");
            assert_eq!(
                s.hint(),
                "REC → Loops A · row 2: REC records a loop there, from pads + synth"
            );
            assert_eq!(cell(&s, 1), "Armed / REC here / ");
            // REC readies it, PLAY starts the first pass.
            s.button("REC");
            s.run_periods(12).settle();
            assert_eq!(s.looper.phase(), Phase::Armed);
            assert_eq!(cell(&s, 1), "Ready / bar line / or PLAY");
            s.button("PLAY");
            until(&mut s, 0, Phase::Recording);
            s.pad(0, 120, 0.3);
            s.run_periods(12).settle();
            assert_eq!(cell(&s, 1), "REC / REC ends / the pass");
            s.run_for(1.4);
            let from = s.lines.len();
            s.button("REC");
            until(&mut s, 0, Phase::Playing);
            let want = "time: Loop 1 on Loops A, row 2, playing";
            assert!(said(&mut s, from, want), "{:?}", &s.lines[from..]);
            assert_eq!(s.time.loop_cell(), None);
            assert_eq!(s.time.loop_on(8), Some(0));
            s.run_periods(12).settle();
            assert_eq!(chip(&mut s), "REC → Loops A · Loop 1 · row 2");

            // Row 1 armed: OVERDUB layers the loop Loops A plays.
            s.press("clips", "slot", "Loops A slot 1").settle();
            assert_eq!(s.time.loop_cell(), Some((8, 0)));
            s.button("OVERDUB");
            until(&mut s, 0, Phase::Overdubbing);
            s.run_for(1.0);
            s.button("OVERDUB");
            until(&mut s, 0, Phase::Playing);
            assert_eq!(s.looper.loop_status(0).map(|l| l.layers), Some(2));
            assert_eq!(s.looper.loop_status(1).map(|l| l.phase), Some(Phase::Empty));
            // The notes armed: the cell lets go.
            s.route(FromUi::ArmTap(Target::Notes));
            s.run_periods(4);
            assert_eq!(s.time.loop_cell(), None);

            // Resample: its first pass records the loops as heard.
            s.route(FromUi::Time(TimeAction::LoopInput(8)));
            s.route(FromUi::Time(TimeAction::LoopInput(8)));
            s.run_periods(4).settle();
            assert_eq!(s.screen.time.tracks[8].sound, "Resample");
            s.press("clips", "slot", "Loops A slot 3").settle();
            assert_eq!(s.time.loop_cell(), Some((8, 2)));
            s.button("REC");
            until(&mut s, 1, Phase::Recording);
            assert!(s.looper.loop_status(1).is_some_and(|l| l.resampling));
            s.run_for(1.5);
            let from = s.lines.len();
            s.button("REC");
            until(&mut s, 1, Phase::Playing);
            let want = "time: Loop 2 on Loops A, row 3, playing";
            assert!(said(&mut s, from, want), "{:?}", &s.lines[from..]);
            assert_eq!(s.time.loop_on(8), Some(1), "one loop plays on Loops A");
            assert_eq!(s.time.project().check(), Ok(()));
        });
    }

    /// Loop tracks part 5 (loop-tracks.md): Rec song keeps a loop's launch
    /// and stop on Loops A as a block, and song mode plays it: the loop on
    /// from the block's start and off on its end's bar line.
    #[test]
    fn rec_song_keeps_a_loops_launches_and_song_mode_plays_them() {
        use crate::gui::FromUi;
        use mpfree_looper::Phase;
        use mpfree_time_ui::TimeAction;
        fn until(s: &mut Session, n: usize, phase: Phase) {
            let end = s.t() + 6.0;
            while s.looper.loop_status(n).map(|l| l.phase) != Some(phase) && s.t() < end {
                s.run_periods(4);
            }
            assert_eq!(s.looper.loop_status(n).map(|l| l.phase), Some(phase));
        }
        /// The first line from `from` on starting with `prefix`, running
        /// the session up to `seconds` for it.
        fn line(s: &mut Session, from: usize, prefix: &str, seconds: f64) -> String {
            let end = s.t() + seconds;
            loop {
                if let Some(l) = s.lines[from..].iter().find(|l| l.starts_with(prefix)) {
                    return l.clone();
                }
                assert!(
                    s.t() < end,
                    "no line like {prefix:?}: {:?}",
                    &s.lines[s.lines.len().saturating_sub(12)..]
                );
                s.run_periods(4);
            }
        }
        /// The bar of a "looper: loop 1 on from bar 5" line.
        fn bar(l: &str) -> u32 {
            l.rsplit(' ').next().and_then(|b| b.parse().ok()).expect(l)
        }
        on_big_stack(|| {
            let mut s = Session::new();
            s.make_sounds();
            s.press("play-transport", "view", "Loop").settle();
            s.button("REC").button("PLAY");
            until(&mut s, 0, Phase::Recording);
            s.pad(0, 120, 0.3);
            s.run_for(1.7);
            s.button("REC");
            until(&mut s, 0, Phase::Playing);
            s.button("STOP").run_for(0.2);

            // Rec song, from bar 1: the loop plays, Stop all stops it on a
            // bar line, and STOP keeps it as a block on Loops A.
            s.route(FromUi::Time(TimeAction::Transport(
                mpfree_transport::Command::Locate(0),
            )));
            s.route(FromUi::Time(TimeAction::RecSong(true)));
            s.button("PLAY");
            s.run_for(3.0);
            let from = s.lines.len();
            s.route(FromUi::Time(TimeAction::StopAll));
            let off = line(&mut s, from, "looper: loop 1 off from bar ", 3.0);
            s.run_for(0.5);
            let from = s.lines.len();
            s.button("STOP");
            line(&mut s, from, "time: rec song", 1.0);
            let p = s.time.project();
            let blocks: Vec<_> = p
                .arrangement
                .placements
                .iter()
                .filter(|b| b.track == 8)
                .map(|b| (b.start, b.end()))
                .collect();
            let bar_ticks = 3840;
            assert_eq!(
                blocks,
                [(0, i64::from(bar(&off) - 1) * bar_ticks)],
                "{:?}",
                p.arrangement.placements
            );

            // Song mode: the block plays the loop from bar 1 and stops it
            // on its end's bar line.
            s.route(FromUi::Time(TimeAction::SongMode(true)));
            s.route(FromUi::Time(TimeAction::Transport(
                mpfree_transport::Command::Locate(0),
            )));
            s.run_periods(4);
            let from = s.lines.len();
            s.button("PLAY");
            let again = line(&mut s, from, "looper: loop 1 off from bar ", 8.0);
            assert_eq!(bar(&again), bar(&off), "{again}");
            assert!(
                s.lines[from..]
                    .iter()
                    .all(|l| !l.starts_with("looper: loop 1 on from")),
                "on from the start, at once: {:?}",
                &s.lines[from..]
            );
            s.button("STOP");
        });
    }

    /// Loop tracks part 6 (loop-tracks.md): SHIFT + a loop's cell dropped
    /// on another Loop track makes a new loop there, a copy of its layers,
    /// silent until launched, then playing with it; a copy into another
    /// row of its own Loop track is the same loop, so a scene change
    /// between the two rows keeps it playing without a launch.
    #[test]
    fn a_loop_copied_onto_another_loop_track_is_a_new_loop_with_its_layers() {
        use crate::gui::FromUi;
        use mpfree_looper::Phase;
        use mpfree_time_ui::TimeAction;
        fn until(s: &mut Session, what: &str, done: &dyn Fn(&Session) -> bool) {
            let end = s.t() + 8.0;
            while !done(s) && s.t() < end {
                s.run_periods(4);
            }
            assert!(
                done(s),
                "never {what}: {:?}",
                &s.lines[s.lines.len().saturating_sub(12)..]
            );
        }
        fn phase(n: usize, p: Phase) -> impl Fn(&Session) -> bool {
            move |s: &Session| s.looper.loop_status(n).map(|l| l.phase) == Some(p)
        }
        on_big_stack(|| {
            let mut s = Session::new();
            s.make_sounds();
            // Loop 1: a first pass and an overdub, on Loops A row 1.
            s.press("play-transport", "view", "Loop").settle();
            s.button("REC").button("PLAY");
            until(&mut s, "recording", &phase(0, Phase::Recording));
            s.pad(0, 120, 0.3);
            s.run_for(1.7);
            s.button("REC");
            until(&mut s, "playing", &phase(0, Phase::Playing));
            s.button("OVERDUB");
            until(&mut s, "overdubbing", &phase(0, Phase::Overdubbing));
            s.pad(1, 110, 0.3);
            s.run_for(2.1);
            s.button("OVERDUB");
            until(&mut s, "two layers", &|s: &Session| {
                s.looper
                    .loop_status(0)
                    .is_some_and(|l| l.phase == Phase::Playing && l.layers == 2)
            });
            assert_eq!(s.time.project().clip(8, 0).and_then(|c| c.looped), Some(0));

            // Loops B, and SHIFT + Loops A's cell dropped on its row 2.
            s.route(FromUi::Time(TimeAction::AddLoopTrack));
            s.run_periods(2);
            assert!(s.time.project().tracks[9].is_loops());
            let from = s.lines.len();
            s.route(FromUi::Time(TimeAction::DropClip {
                from: (8, 0),
                to: (9, 1),
                copy: true,
            }));
            s.run_periods(2).settle();
            assert!(
                s.hint()
                    .starts_with("Loop 1 copies into Loop 2 on Loops B / 2: launch it to hear it"),
                "{}",
                s.hint()
            );
            assert_eq!(s.time.project().clip(9, 1).and_then(|c| c.looped), Some(1));
            s.run_periods(11).settle();
            let c = mpfree_time_ui::view::cell_of(&s.screen.time, 9, 1);
            assert_eq!((c.name.as_str(), c.foot.as_str()), ("Loop 2", "copying in"));
            until(&mut s, "copied", &|s: &Session| {
                s.lines[from..]
                    .iter()
                    .any(|l| l == "looper: loop 2 is a copy of loop 1, 2 layers")
            });
            let (a, b) = (
                s.looper.loop_status(0).unwrap(),
                s.looper.loop_status(1).unwrap(),
            );
            assert_eq!((b.layers, b.ticks, b.frames), (2, a.ticks, a.frames));
            assert!(!b.on, "silent until launched");
            s.run_periods(12).settle();
            let c = mpfree_time_ui::view::cell_of(&s.screen.time, 9, 1);
            assert_eq!(c.foot, "", "copied");
            // Launched, it plays with loop 1, on the bar line.
            s.route(FromUi::Time(TimeAction::Launch { track: 9, slot: 1 }));
            until(&mut s, "on", &|s: &Session| {
                s.looper.loop_status(1).is_some_and(|l| l.on)
            });
            assert!(s.looper.loop_status(0).unwrap().on);

            // The same loop in row 3 of Loops A: row 3's scene keeps it
            // playing (no launch), and Loops B, empty there, stops.
            s.route(FromUi::Time(TimeAction::DropClip {
                from: (8, 0),
                to: (8, 2),
                copy: true,
            }));
            s.run_periods(2);
            assert_eq!(s.time.project().clip(8, 2).and_then(|c| c.looped), Some(0));
            let from = s.lines.len();
            s.route(FromUi::Time(TimeAction::LaunchScene(2)));
            until(&mut s, "loop 2 off", &|s: &Session| {
                s.looper.loop_status(1).is_some_and(|l| !l.on)
            });
            s.run_for(0.5);
            assert!(s.looper.loop_status(0).unwrap().on);
            assert!(
                s.lines[from..]
                    .iter()
                    .all(|l| !l.starts_with("looper: loop 1 ")),
                "loop 1 went on playing: {:?}",
                &s.lines[from..]
            );
            assert_eq!(s.time.project().check(), Ok(()));
            s.button("STOP");
        });
    }

    /// Make and Perform (sequencer S4, Jay's decision 5): the top bar's
    /// MAKE chip switches to Perform and says what changed; MAIN then goes
    /// to Arrange's Clips grid and steps to the Song view and back, the arm
    /// is pinned, and SHIFT + MAIN comes back to Make, with nothing
    /// playing stopped.
    #[test]
    fn make_and_perform_switch_from_the_chip_and_shift_main() {
        use mpfree_transport::State;
        on_big_stack(|| {
            let mut s = Session::new();
            s.run_for(0.1);
            assert!(!s.time.project().perform, "a new project is in Make");
            assert!(s.control("header", "chip", "MAKE").is_some());
            s.button("PLAY");
            s.run_for(0.3);
            s.press("header", "chip", "MAKE");
            s.run_for(0.2);
            assert!(s.time.project().perform);
            assert!(s.control("header", "chip", "PERFORM").is_some());
            assert!(s.hint().starts_with("Perform: "), "{}", s.hint());
            assert!(s.app.rec.pinned, "Perform pins the arm");
            assert_eq!(s.time.info().state, State::Playing, "nothing stops");
            // MAIN: the Clips grid, then the Song view, then the grid.
            s.button("MAIN").run_for(0.2);
            assert_eq!(s.view(), View::Arrange);
            assert!(!s.screen.time.song_view);
            s.button("MAIN").run_for(0.2);
            assert!(s.screen.time.song_view);
            s.button("MAIN").run_for(0.2);
            assert!(!s.screen.time.song_view);
            // A page doesn't move the pinned arm.
            s.press("header", "tab", "Looper").run_for(0.2);
            assert_eq!(s.app.rec.target, crate::rec_arm::Target::Notes);
            // SHIFT + MAIN: Make again; MAIN is Play's.
            s.button_with("SHIFT", "MAIN").run_for(0.2);
            assert!(!s.time.project().perform);
            assert!(s.hint().starts_with("Make: "), "{}", s.hint());
            assert!(!s.app.rec.pinned);
            assert!(s.control("header", "chip", "MAKE").is_some());
            s.button("MAIN").run_for(0.2);
            assert_eq!(s.view(), View::Play);
            assert_eq!(s.time.info().state, State::Playing);
            s.button("STOP");
        });
    }

    /// The mute queue and solo (sequencer S4): in Perform TRACK MUTE's
    /// pad waits for the next bar line (the transport bar says WAITING),
    /// and the kick goes quiet there; TRACK MUTE held with a pad solos
    /// (SHIFT: at once), and letting go leaves the mute keys on.
    #[test]
    fn in_perform_a_track_mute_waits_for_the_bar_and_held_it_solos() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("play-transport", "view", "Beats");
            for k in [1, 5, 9, 13] {
                s.press("steps", "step", &format!("Kick step {k}"));
            }
            s.button("PLAY").run_for(0.3);
            s.button_with("SHIFT", "MAIN").run_for(0.2);
            assert!(s.time.project().perform);
            s.button("TRACK MUTE");
            s.pad(0, 100, 0.05);
            assert!(s.time.mute_light(0).unwrap().waiting, "it waits");
            assert_eq!(s.time.muted(0), Some(false));
            assert!(
                s.said()
                    .iter()
                    .any(|l| l.starts_with("track mute: ") && l.ends_with(" muted on the next bar")),
                "{:?}",
                s.said()
            );
            s.run_for(0.1);
            let reading = s.screen.time.waiting_reading();
            assert!(
                reading.as_deref().is_some_and(|r| r.ends_with(" at 2.1")),
                "{reading:?}"
            );
            // The bar line: muted, and the kick is silent after it.
            s.run_for(2.0);
            assert_eq!(s.time.muted(0), Some(true), "landed on bar 2");
            assert!(s.time.mute_wait().is_none());
            let from = s.hits.on.len();
            s.run_for(2.0);
            assert_eq!(hits_on(&s, 0, from), 0, "muted, the kick is silent");
            // Held with SHIFT + a pad: a solo at once.
            s.button_edge("TRACK MUTE", true);
            s.button_edge("SHIFT", true);
            s.pad(0, 100, 0.05);
            s.button_edge("SHIFT", false);
            assert!(s.time.mute_light(0).unwrap().solo, "soloed at once");
            s.button_edge("TRACK MUTE", false);
            assert_eq!(s.app.mute_keys(), crate::app::MuteKeys::Tracks, "still on");
            // STOP lands a change that waits.
            s.pad(0, 100, 0.05);
            assert!(s.time.mute_light(0).unwrap().waiting);
            s.button("STOP").run_for(0.1);
            assert_eq!(s.time.muted(0), Some(false), "STOP landed it");
            assert!(s.time.mute_wait().is_none());
            s.button("TRACK MUTE");
        });
    }

    /// Save state and Reload clip (sequencer S4): the clip card keeps the
    /// pattern, two more steps change it, and Reload clip on the morph
    /// bar brings it back on the next bar line; UNDO takes the reload
    /// back.
    #[test]
    fn reload_clip_brings_a_saved_pattern_back_on_the_bar() {
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
            assert!(
                s.control("morph", "button", "Reload clip")
                    .is_some_and(|c| !c.target.enabled),
                "nothing saved yet"
            );
            s.press("clip-card", "button", "Save state").settle();
            let (t, slot) = s.time.state().focus().unwrap();
            let notes = |s: &Session| s.time.project().clip(t, slot).unwrap().notes.len();
            let saved = notes(&s);
            assert_eq!(saved, 4);
            assert!(s
                .said()
                .iter()
                .any(|l| l.contains("save state: ") && l.contains(" kept")));
            // Two more steps.
            s.press("header", "tab", "Play").settle();
            s.press("steps", "step", "Kick step 3");
            s.press("steps", "step", "Kick step 7");
            assert_eq!(notes(&s), 6);
            s.press("header", "tab", "Arrange").settle();
            s.button("PLAY").run_for(0.3);
            let n = s.lines.len();
            s.press("morph", "button", "Reload clip").settle();
            assert_eq!(s.time.state().reloads.len(), 1, "it waits for the bar");
            assert_eq!(notes(&s), 6);
            s.run_for(2.0);
            assert_eq!(notes(&s), saved, "back on the line");
            let lines = s.lines[n..].to_vec();
            assert!(
                lines
                    .iter()
                    .any(|l| l.contains("reload: ") && l.contains(" back as saved on bar 2")),
                "{lines:#?}"
            );
            s.button("UNDO").settle();
            assert_eq!(notes(&s), 6, "UNDO takes the reload back");
            s.button("STOP");
        });
    }

    /// The song list (sequencer S4, mockup D): in Perform MAIN goes
    /// Clips, Song, List; scenes added to the list, a step's times, and
    /// Play from here plays it from the next bar line, the step lit with
    /// its pass.
    #[test]
    fn the_song_list_plays_from_a_step_in_perform() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("play-transport", "view", "Beats");
            for k in [1, 5, 9, 13] {
                s.press("steps", "step", &format!("Kick step {k}"));
            }
            s.button_with("SHIFT", "MAIN").settle();
            assert!(s.time.project().perform);
            s.button("MAIN").settle();
            s.button("MAIN").settle();
            assert!(s.time.state().song_view);
            s.button("MAIN").settle();
            let st = s.time.state();
            assert!(st.list_view && !st.song_view);
            assert!(s.control("song-list", "add", "+ Add a scene").is_some());
            s.press("song-list", "add", "+ Add a scene").settle();
            s.press("song-list", "add", "+ Add a scene").settle();
            s.press("song-list", "times-more", "Step 1 +").settle();
            let list = &s.time.project().arrangement.list;
            assert_eq!(list.steps.len(), 2);
            assert_eq!(list.steps[0].times, 2);
            s.press("song-list", "step", "Step 1").settle();
            assert_eq!(s.time.state().list_pick, Some(0));
            s.button("PLAY").run_for(0.3);
            s.press("song-list", "button", "Play from here").settle();
            assert!(
                s.time.state().list[0].waiting,
                "it waits for the next bar line"
            );
            s.run_for(2.2);
            let st = s.time.state();
            assert_eq!(st.list[0].playing.map(|p| p.0), Some(1), "pass 1");
            assert!(s.said().iter().any(|l| l.contains("song list from step 1")));
            s.button("STOP");
        });
    }

    /// Rec song with REC (sequencer S4): in Perform the take REC records
    /// brings Rec song with it, and the REC chip says "+ Rec song"; what
    /// the grid played goes into the song when the take ends. The target
    /// list's switch takes it off for the next take.
    #[test]
    fn in_perform_rec_brings_rec_song_with_the_take() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("play-transport", "view", "Beats");
            for k in [1, 5, 9, 13] {
                s.press("steps", "step", &format!("Kick step {k}"));
            }
            s.button_with("SHIFT", "MAIN").settle();
            assert!(s.time.project().perform);
            let chip = |s: &mut Session| {
                s.control("play-transport", "rec", "")
                    .map(|c| c.target.label)
                    .unwrap_or_default()
            };
            assert!(chip(&mut s).ends_with(" + Rec song"), "{}", chip(&mut s));
            s.button("REC").button("PLAY");
            s.run_for(2.5);
            assert!(s.time.state().song_rec, "the take turned Rec song on");
            assert!(
                s.said()
                    .iter()
                    .any(|l| l == "time: rec song: on with REC (Perform)"),
                "{:?}",
                s.said()
            );
            s.pad(2, 100, 0.05);
            s.run_for(2.0);
            s.button("STOP").run_for(0.3);
            assert!(!s.time.state().song_rec, "off with the take");
            assert!(
                !s.time.state().placements.is_empty(),
                "what the grid played is in the song"
            );
            // The list's switch: off for the next take.
            let c = s.control("play-transport", "rec", "").unwrap().target;
            s.press_target(&c);
            s.settle();
            s.press("rec-list", "switch", "Rec song").settle();
            assert!(!s.time.state().song_with_take);
            assert!(!chip(&mut s).ends_with(" + Rec song"), "{}", chip(&mut s));
            s.press("rec-list", "switch", "Rec song").settle();
            assert!(s.time.state().song_with_take, "and back");
            s.press("rec-list", "close", "Close").settle();
        });
    }
}
