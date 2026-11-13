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
    #[test]
    fn an_insert_added_on_the_mixer_is_turned_bypassed_moved_and_removed() {
        on_big_stack(|| {
            let drums = StripId::Track(0);
            let mut s = Session::new();
            s.press("header", "mixer-icon", "");
            assert_eq!(s.view(), View::Mixer);
            assert_eq!(strip_inserts(&s, "Drums"), ["DJ filter"]);

            // "+ Insert" opens the editor in the mixer's place, picker up.
            s.press("mixer", "add-insert", "Drums + Insert");
            assert_eq!(s.view(), View::Chain);
            assert!(s.control("chain", "cancel", "Cancel").is_some());
            s.press("chain", "pick", "Compressor").run_for(0.1);
            let t = engine(&mut s);
            assert_eq!(t.title, "Drums FX");
            assert_eq!(names(&t), ["DJ filter", "Compressor"]);
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
            assert!(node_param(&s, drums, 1, "Ratio") > before, "Q-Link 2 raised Ratio");
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
            assert_eq!(names(&t), ["Compressor", "DJ filter"]);
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
            assert_eq!(strip_inserts(&s, "Drums"), ["Compressor", "DJ filter"]);

            // Its row opens the editor on it.
            s.press("mixer", "insert", "Compressor");
            assert_eq!(s.view(), View::Chain);
            assert_eq!(engine(&mut s).selected, Some(0));
            agree(&mut s, "opened from its row");

            // Remove takes it out of the sound and off the strip.
            s.press("chain", "remove", "Remove").run_for(0.1);
            assert_eq!(names(&engine(&mut s)), ["DJ filter"]);
            assert_eq!(s.mix.mixer.chain(drums).unwrap().len(), 1);
            agree(&mut s, "removed");
            s.press("chain", "back", "").run_for(0.1);
            assert_eq!(strip_inserts(&s, "Drums"), ["DJ filter"]);
        });
    }

    /// The picker's Cancel adds nothing and leaves the editor on the chain;
    /// Back from there is the mixer as it was.
    #[test]
    fn the_effect_pickers_cancel_adds_nothing() {
        on_big_stack(|| {
            let mut s = Session::new();
            s.press("header", "mixer-icon", "");
            s.press("mixer", "add-insert", "Loop + Insert");
            s.press("chain", "cancel", "Cancel").run_for(0.1);
            assert_eq!(s.view(), View::Chain);
            assert!(s.control("chain", "pick", "").is_none(), "the picker closed");
            assert_eq!(names(&engine(&mut s)), ["DJ filter"]);
            agree(&mut s, "cancelled");
            // "+ Add effect" opens the picker again; Back closes the editor.
            s.press("chain", "add", "+ Add effect");
            assert!(s.control("chain", "pick", "Delay").is_some());
            s.press("chain", "back", "").run_for(0.1);
            assert_eq!(s.view(), View::Mixer);
            assert_eq!(strip_inserts(&s, "Loop"), ["DJ filter"]);
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
            s.press("header", "mixer-icon", "");
            s.press("mixer", "insert", "DJ filter");
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
            s.press("header", "mixer-icon", "");
            s.press("mixer", "insert", "DJ filter");
            s.press("chain", "knob", "Position");
            s.press("chain", "learn", "Learn");
            let n = s.sent.len();
            s.qlink(2, 4);
            assert!(s.sent[n..].iter().any(|a| a == "ChainLearned"), "{:?}", &s.sent[n..]);
            let cards: Vec<String> = s
                .controls()
                .into_iter()
                .filter(|c| c.widget == "qlinks")
                .map(|c| c.target.label)
                .collect();
            assert_eq!(cards[2], "Position", "Q-Link 3 has it: {cards:?}");
            let at = node_param(&s, drums, 0, "Position");
            s.qlink(2, -6).run_for(0.1);
            assert!(node_param(&s, drums, 0, "Position") < at, "Q-Link 3 lowered it");
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
            assert_eq!(names(&t), ["DJ filter"]);
            agree(&mut s, "the track's chain");

            s.press("chain", "add", "+ Add effect");
            s.press("chain", "pick", "Delay").run_for(0.1);
            assert_eq!(names(&engine(&mut s)), ["DJ filter", "Delay"]);
            agree(&mut s, "a delay on the track");

            // Back to the pad's chain: still empty.
            s.press("chain", "scope", "Pad 5");
            assert!(engine(&mut s).slots.is_empty());
            agree(&mut s, "the pad's chain again");

            s.press("header", "tab", "Sample");
            assert_eq!(s.view(), View::Sample);
            assert!(s.control("smp-bar", "tab", "FX").is_some(), "the sound editor's bar");

            s.press("header", "mixer-icon", "").run_for(0.1);
            assert_eq!(strip_inserts(&s, "Drums"), ["DJ filter", "Delay"]);
        });
    }

    /// The strip as the mixer screen shows it.
    fn strip(s: &Session, name: &str) -> mpfree_mixer_ui::StripView {
        let st = s.screen.ui.get::<MixerView>(MIXER).expect("the mixer").state();
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
            s.press("header", "mixer-icon", "");
            let p = |s: &Session, id| s.mix.mixer.param(drums, id);
            let shows = |s: &mut Session, when: &str| {
                s.run_for(0.1);
                let v = strip(s, "Drums");
                let m = &s.mix.mixer;
                assert!((v.volume_db - m.param(drums, mx::VOLUME)).abs() < 0.05, "{when}: volume");
                assert_eq!(v.mute, m.param(drums, mx::MUTE) > 0.5, "{when}: M");
                assert_eq!(v.solo, m.param(drums, mx::SOLO) > 0.5, "{when}: S");
                assert_eq!(f32::from(v.output), m.param(drums, mx::OUTPUT), "{when}: output");
                let a = v.sends.expect("Drums has sends")[0];
                assert!((a.db - m.param(drums, mx::send(0))).abs() < 0.05 || a.db < -60.0, "{when}: send A");
                assert_eq!(a.pre, m.param(drums, mx::send_pre(0)) > 0.5, "{when}: send A pre");
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
            assert!(!s.mix.mixer.is_audible(synth), "the synth is quiet while Drums is soloed");
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
                assert!(r.right() <= mixer.right() - 150, "{} at {r:?} runs under the master strip", c.target.label);
            }
            // CV 1: Off, then Pitch, then Gate (High starts at 5.0 V).
            s.press("mixer", "field", "CV 1: Off");
            s.press("mixer", "field", "CV 1: Pitch");
            let high = s.control("mixer", "field", "CV 1: 5.0 V high").expect("Gate's High");
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
            s.press("header", "mixer-icon", "");
            s.press("mixer", "insert", "DJ filter");
            s.press("chain", "knob", "Resonance");
            assert!(s.control("chain", "depth", "").is_none());
            s.press("chain", "add-lfo", "+ LFO").run_for(0.1);
            let row = s.control("chain", "depth", "row 1 depth").expect("the LFO's row");
            let t = engine(&mut s);
            let res = t.params.iter().find(|p| p.info.name == "Resonance").unwrap();
            assert!(res.mod_range.is_some(), "the LFO moves Resonance");
            s.press_target(&row.target).run_for(0.1);
            s.press("chain", "remove-row", "row 1 ×").run_for(0.1);
            assert!(s.control("chain", "depth", "").is_none(), "the row came off");
            let t = engine(&mut s);
            let res = t.params.iter().find(|p| p.info.name == "Resonance").unwrap();
            assert!(res.mod_range.is_none(), "nothing moves Resonance now");
        });
    }
}

#[cfg(test)]
mod explore {
    use super::super::{on_big_stack, Session};
    fn dump(s: &mut Session, what: &str) {
        println!("== {what}: {} view {:?} hint: {}", s.page(), s.view(), s.hint());
        let mut last = String::new();
        let mut n = 0;
        for c in s.controls() {
            let key = format!("{}/{}", c.widget, c.target.id);
            if c.widget == "header" || c.widget == "qlinks" { continue; }
            if key == last { n += 1; if n > 3 { continue; } } else { n = 0; last = key.clone(); }
            println!("  {key} #{} \"{}\" {:?}{}", c.target.index, c.target.label, c.target.gesture, if c.target.enabled { "" } else { " (off)" });
        }
    }
    #[test]
    fn explore() {
        on_big_stack(|| {
            let mut s = Session::new();
            let info = |s: &Session| { let i = s.time.info(); format!("{:?} od {} bpm {}", i.state, i.overdub, i.bpm) };
            s.press("play-transport", "view", "Beats");
            let g = s.screen.ui.get::<crate::play_widgets::StepGrid>(crate::play_page::STEPS).unwrap();
            println!("lanes {:?}", g.lanes.iter().map(|l| (l.name.clone(), l.pad)).collect::<Vec<_>>());
            let m0 = s.mark();
            s.button("REC");
            println!("after REC: {}", info(&s));
            s.button("PLAY");
            println!("after PLAY: {}", info(&s));
            s.run_for(0.3);
            println!("0.3 s: {}", info(&s));
            s.run_for(2.0);
            println!("2.3 s: {}", info(&s));
            let h = s.hits.on.len();
            println!("pos {} armed? steps track {}", s.time.position_text(), s.time.steps().track);
            s.pad(0, 100, 0.05);
            println!("hits {} -> {} : {:?}", h, s.hits.on.len(), &s.hits.on[h..]);
            s.run_for(0.4);
            println!("pos {}", s.time.position_text());
            let g = s.screen.ui.get::<crate::play_widgets::StepGrid>(crate::play_page::STEPS).unwrap();
            println!("grid kick {:?} clip {:?}", g.lanes[0].steps, s.time.steps().clip.as_ref().map(|c| c.name.clone()));
            let m = s.mark();
            s.button("STOP");
            s.run_for(0.2);
            let g = s.screen.ui.get::<crate::play_widgets::StepGrid>(crate::play_page::STEPS).unwrap();
            println!("after STOP: {} grid kick {:?} clip {:?} lines {:?}", info(&s), g.lanes[0].steps, s.time.steps().clip.as_ref().map(|c| c.name.clone()), s.lines_since(m));
            println!("all lines since REC: {:#?}", s.lines_since(m0));
            let st = s.time.state();
            println!("tracks: {:?}", st.tracks.iter().take(3).map(|t| (t.name.clone(), format!("{:?}", t.slots))).collect::<Vec<_>>());
            s.button("UNDO");
            s.run_for(0.2);
            let g = s.screen.ui.get::<crate::play_widgets::StepGrid>(crate::play_page::STEPS).unwrap();
            println!("after UNDO grid kick {:?} hint {}", g.lanes[0].steps, s.hint());
            s.press("header", "mixer-icon", "");
            s.press("mixer", "view", "Pads");
            dump(&mut s, "mixer pads");
            s.press("header", "tab", "Synth");
            println!("synth tab: synth_on {}", s.synth_on);
            s.press("header", "tab", "Looper");
            dump(&mut s, "looper");
            s.press("header", "tab", "Play");
            let k = s.kit;
            s.wheel(1);
            let chip = s.control("header", "chip", "").map(|c| c.target.label); println!("wheel: kit {} -> {} chip {:?}", k, s.kit, chip);
            s.button("TRACK MUTE");
            println!("track mute: hint {}", s.hint());
        });
    }
}
