//! Phrases in the pad player: the list a track picks its phrase from, and
//! how the track's arp follows the pick (the design is in
//! `docs/dev/design/arpeggiator.md`, "Phrases").
//!
//! A track keeps a pick, never a copy: its place in `App::phrases`, the
//! one list every track picks from (MPFree's own phrases, then the ones
//! only the open project has). The phrase itself goes to the track's
//! engine when it is picked. `TrackUi` stays small that way: all 128 are
//! copied whenever the project's arps are looked over, and a phrase is
//! 3 KB.
//!
//! A phrase moves notes, so it plays where the pads press notes into the
//! arp (a synth); on a kit the pads play as pressed.
//!
//! Until the Phrase tab is built (its boards come first) nothing on screen
//! picks Phrase mode. The wheel, the project's arp part and the engine
//! already follow it.

use crate::app::{App, Effect};
use mpfree_arp::{Library, Mode, Phrase};

/// MPFree's starter phrases, by name, in the library's order.
pub fn built_in_phrases() -> Vec<(String, Phrase)> {
    Library::built_in()
        .phrases()
        .map(|(n, p)| (n.to_string(), *p))
        .collect()
}

impl App {
    /// The phrases to pick from: MPFree's own, then the open project's.
    /// (The Phrase tab's list reads this and the pick; until the tab is
    /// built only the tests do.)
    #[cfg(test)]
    pub fn phrases(&self) -> &[(String, Phrase)] {
        &self.phrases
    }

    /// The armed track's pick, as a place in the list.
    #[cfg(test)]
    pub fn phrase_index(&self) -> usize {
        self.phrase_index
    }

    /// The name of track `t`'s phrase.
    pub fn phrase_name(&self, t: usize) -> &str {
        let i = if t == self.track {
            self.phrase_index
        } else {
            self.track_ui.get(t).map_or(0, |u| u.phrase_index)
        };
        self.phrases.get(i).map_or("", |(n, _)| n.as_str())
    }

    /// A phrase plays here: the pads press notes into the armed track's
    /// arp (a synth), not a kit's pads.
    pub fn phrases_play(&self) -> bool {
        self.press_layout().is_some()
    }

    /// The armed track takes phrase `i` of the list. While its phrase
    /// plays, the new one comes in on its line (`Engine::set_phrase`: the
    /// bar line with the transport, the next step without).
    pub fn pick_phrase(&mut self, i: usize) -> Vec<Effect> {
        let Some((name, phrase)) = self.phrases.get(i) else {
            return Vec::new();
        };
        let note = Effect::Note(format!("arp phrase: {name} ({})", phrase.to_text()));
        self.phrase_index = i;
        self.send_phrase();
        vec![note]
    }

    /// The wheel on a phrase: the pick moves along the list, stopping at
    /// its ends.
    pub(super) fn phrase_wheel(&mut self, delta: i32) -> Vec<Effect> {
        let last = self.phrases.len().saturating_sub(1) as i32;
        let i = (self.phrase_index as i32 + delta).clamp(0, last) as usize;
        if i == self.phrase_index {
            Vec::new()
        } else {
            self.pick_phrase(i)
        }
    }

    /// The armed track's engine takes the track's pick (after a pick, or
    /// when the track or the project changed). The engine leaves a phrase
    /// that already plays alone. Allocates nothing.
    pub(super) fn send_phrase(&mut self) {
        if self.phrase_index >= self.phrases.len() {
            self.phrase_index = 0;
        }
        if let Some((_, phrase)) = self.phrases.get(self.phrase_index) {
            self.arps.engine_at_mut(self.track).set_phrase(*phrase);
        }
        self.arp_dirty = true;
    }

    /// What the pads press changed (a kit's pads, or notes): a track on
    /// Phrase starts playing its phrase, or goes back to playing as
    /// pressed.
    pub(super) fn phrase_follows_pads(&mut self) {
        if self.arp_mode == Mode::Phrase
            && self.arps.engine_at(self.track).settings().mode != self.engine_mode()
        {
            self.update_engine();
        }
    }

    /// The open project's own phrases go (another project is opening):
    /// the list is MPFree's again.
    pub(super) fn drop_project_phrases(&mut self) {
        self.phrases.truncate(self.phrases_own);
    }

    /// The place in the list of the phrase a project kept as `name` and
    /// `phrase`: the list's own phrase of that name when it is the same
    /// phrase, else the project's copy, added after MPFree's (under its
    /// name, or the name with "(project)" when MPFree has another phrase
    /// by it), so the project plays what it was saved with.
    pub(super) fn place_phrase(&mut self, name: &str, phrase: Phrase) -> usize {
        if let Some(i) = self
            .phrases
            .iter()
            .position(|(n, p)| n == name && *p == phrase)
        {
            return i;
        }
        let taken = |phrases: &[(String, Phrase)], n: &str| phrases.iter().any(|(m, _)| m == n);
        let name = if name.is_empty() { "Phrase" } else { name };
        let mut own = name.to_string();
        if taken(&self.phrases, &own) {
            own = format!("{name} (project)");
            let mut k = 2;
            while taken(&self.phrases, &own) {
                // The same name kept twice with different notes.
                if let Some(i) = self
                    .phrases
                    .iter()
                    .position(|(n, p)| *n == own && *p == phrase)
                {
                    return i;
                }
                own = format!("{name} (project {k})");
                k += 1;
            }
        }
        self.phrases.push((own, phrase));
        self.phrases.len() - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::View;
    use mpfree_arp::scale::pads::{layout, PadLayout, PadSetup};
    use mpfree_arp::{EventKind, Events, Follow, Speed};
    use mpfree_arp_ui::view::ArpAction;

    const SR: u32 = 44_100;

    fn ready() -> App {
        let mut app = App::new(SR, false);
        app.start();
        app
    }

    /// The notes that start in the next `frames` frames.
    fn ons(app: &mut App, frames: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let mut ev = Events::new();
        let mut left = frames;
        while left > 0 {
            let n = left.min(128);
            app.process(n, &mut ev);
            out.extend(ev.as_slice().iter().filter_map(|e| match e.kind {
                EventKind::On { note, .. } => Some(note),
                EventKind::Off { .. } => None,
            }));
            left -= n;
        }
        out
    }

    /// The pads play a synth chromatically from C2 (pad 1 is 36).
    fn on_a_synth(app: &mut App) -> u8 {
        let pads = layout(&PadSetup {
            layout: PadLayout::Chromatic,
            ..PadSetup::default()
        });
        app.set_pad_layout(Some(pads));
        pads[0].notes()[0]
    }

    fn pick_named(app: &mut App, name: &str) {
        let i = app
            .phrases()
            .iter()
            .position(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("no phrase {name}"));
        app.pick_phrase(i);
    }

    /// One bar at the app's tempo, in frames.
    fn bar(app: &App) -> u32 {
        (f64::from(SR) * 240.0 / app.settings.bpm).round() as u32
    }

    #[test]
    fn the_list_is_mpfrees_sixty_four() {
        let app = ready();
        assert_eq!(app.phrases().len(), 64);
        assert_eq!(app.phrases()[0].0, "Offbeat pump");
        assert_eq!(app.phrase_name(0), "Offbeat pump");
        assert_eq!(
            app.phrase_name(5),
            "Offbeat pump",
            "every track starts on it"
        );
    }

    /// On a synth, a held pad plays the picked phrase from its note; the
    /// wheel moves the pick, and the new phrase comes in on its line.
    #[test]
    fn a_held_pad_plays_the_picked_phrase_from_its_note() {
        let mut app = ready();
        let home = on_a_synth(&mut app);
        app.toggle_panel();
        assert_eq!(app.view(), View::Arp);
        app.arp_action(ArpAction::Mode(Mode::Phrase));
        pick_named(&mut app, "Up and back");
        assert_eq!(app.phrase_name(0), "Up and back");
        app.pad_down(0, 100);
        let bar = bar(&app);
        // 1 3 5 6 1+ 6 5 3 in eighths: with no scale, by semitones.
        let up = [0, 4, 7, 9, 12, 9, 7, 4].map(|s| home + s);
        assert_eq!(ons(&mut app, bar), up);
        // The wheel: the next phrase in the list waits for its line (the
        // next step: the transport is stopped).
        let at = app.phrase_index();
        assert_eq!(app.wheel(1).len(), 1);
        assert_eq!(app.phrase_index(), at + 1);
        assert_eq!(app.eng().phrase(), &app.phrases()[at].1, "still playing");
        let next = app.phrases()[at + 1].1;
        ons(&mut app, bar);
        assert_eq!(app.eng().phrase(), &next);
        // The ends of the list stop the wheel.
        assert!(app.wheel(1000).len() == 1 && app.wheel(1).is_empty());
        assert_eq!(app.phrase_index(), 63);
    }

    /// A phrase moves notes: on a kit the pads play as pressed.
    #[test]
    fn on_a_kit_the_pads_play_as_pressed() {
        let mut app = ready();
        app.toggle_panel();
        app.arp_action(ArpAction::Mode(Mode::Phrase));
        assert!(!app.phrases_play());
        assert_eq!(app.eng().settings().mode, Mode::Direct);
        app.pad_down(3, 100);
        assert_eq!(ons(&mut app, 4_096), [3], "pad 4, once");
        app.pad_up(3);
        // The same track on a synth: the phrase plays; a kit again, as
        // pressed.
        on_a_synth(&mut app);
        assert!(app.phrases_play());
        assert_eq!(app.eng().settings().mode, Mode::Phrase);
        app.set_pad_layout(None);
        assert_eq!(app.eng().settings().mode, Mode::Direct);
    }

    /// Each track keeps its own pick, and a latched phrase plays on,
    /// from where it is, when its track is armed again.
    #[test]
    fn each_track_keeps_its_pick() {
        let mut app = ready();
        on_a_synth(&mut app);
        app.toggle_panel();
        app.arp_action(ArpAction::Mode(Mode::Phrase));
        pick_named(&mut app, "Night drive");
        app.set_track(1);
        assert_eq!(app.phrase_name(1), "Offbeat pump");
        assert_eq!(app.phrase_name(0), "Night drive");
        pick_named(&mut app, "Sus lift");
        app.set_track(0);
        assert_eq!(app.phrase_name(0), "Night drive");
        assert_eq!(app.phrase_name(1), "Sus lift");
        assert_eq!(
            app.eng().phrase(),
            &app.phrases()[app.phrase_index()].1,
            "the armed track's engine has its pick"
        );
    }

    /// The project keeps each track's pick with the phrase as it plays,
    /// how it follows and its speed; a phrase the list doesn't have comes
    /// back as the project's own.
    #[test]
    fn the_project_keeps_each_tracks_phrase() {
        let mut app = ready();
        on_a_synth(&mut app);
        app.toggle_panel();
        app.arp_action(ArpAction::Mode(Mode::Phrase));
        pick_named(&mut app, "Dub steps");
        app.arp.follow = Follow::Exact;
        app.arp.speed = Speed::Double;
        app.arp_changed();
        let part = app.arp_part();
        for line in [
            "mode = phrase",
            "follow = exact",
            "speed = double",
            "phrase-name = Dub steps",
            "phrase = bass minor 1/8 : 1 _ . 1 b3 _ 1 . 5- _ . 5- b7- _ 1 .",
        ] {
            assert!(part.lines().any(|l| l == line), "{line} in:\n{part}");
        }
        // A track that never touched phrases says how it follows and its
        // speed, and no phrase.
        let mut plain = ready();
        let part_plain = plain.arp_part();
        assert!(part_plain.contains("follow = inkey") && part_plain.contains("speed = aswritten"));
        assert!(!part_plain.contains("phrase"), "{part_plain}");

        let mut again = ready();
        on_a_synth(&mut again);
        again.toggle_panel();
        let (notes, _) = again.open_arp(Some(&part));
        assert_eq!(notes, Vec::<String>::new());
        assert_eq!(again.phrase_name(0), "Dub steps");
        assert_eq!(again.phrases().len(), 64, "MPFree's own: nothing added");
        assert_eq!(again.eng().phrase(), &app.phrases()[app.phrase_index()].1);
        assert_eq!(
            (again.arp.follow, again.arp.speed),
            (Follow::Exact, Speed::Double)
        );
        assert_eq!(again.arp_part(), part);

        // The same name with other notes, and a name the list hasn't got:
        // both come back as the project's own, and go with the project.
        let changed = part.replace("1 _ . 1 b3 _ 1 .", "1 _ . 1 5 _ 1 .");
        let (notes, _) = again.open_arp(Some(&changed));
        assert_eq!(notes, Vec::<String>::new());
        assert_eq!(again.phrase_name(0), "Dub steps (project)");
        assert_eq!(again.phrases().len(), 65);
        assert_eq!(
            again.arp_part(),
            changed.replace("= Dub steps", "= Dub steps (project)")
        );
        let mine = part.replace("phrase-name = Dub steps", "phrase-name = My riff");
        let (notes, _) = again.open_arp(Some(&mine));
        assert_eq!(notes, Vec::<String>::new());
        assert_eq!(again.phrase_name(0), "My riff");
        assert_eq!(again.phrases().len(), 65, "the last project's own is gone");
        assert_eq!(again.phrases()[64].1, app.phrases()[app.phrase_index()].1);
        // New: every arp at its default, and MPFree's list.
        again.open_arp(None);
        assert_eq!(again.phrases().len(), 64);
        assert_eq!(again.phrase_name(0), "Offbeat pump");
        // A phrase that can't be read is left out and said.
        let bad = part.replace("phrase = bass minor 1/8 :", "phrase = bass minor 1/8 : z");
        let (notes, _) = plain.open_arp(Some(&bad));
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("track 1 phrase = "), "{notes:?}");
    }
}
