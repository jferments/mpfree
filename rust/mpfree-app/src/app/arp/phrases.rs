//! Phrases in the pad player: the list a track picks its phrase from, and
//! how the track's arp follows the pick (the design is in
//! `docs/dev/design/arpeggiator.md`, "Phrases").
//!
//! A track keeps a pick, never a copy: its place in `App::phrases`, the
//! one list every track picks from. The phrase itself goes to the track's
//! engine when it is picked. `TrackUi` stays small that way: all 128 are
//! copied whenever the project's arps are looked over, and a phrase is
//! 3 KB.
//!
//! The list has three runs, in this order:
//!
//! 1. **MPFree's own** (the first `phrases_built`). One of them that the
//!    owner's library names again plays the owner's notes in its place,
//!    as a pattern of the owner's takes a built-in pattern's.
//! 2. **The owner's** (up to `phrases_own`), from the store's
//!    arp/phrases.txt ([`App::load_phrases`]; a phrase line in a patterns
//!    file is the owner's too). Save keeps them there
//!    ([`App::phrases_text`]).
//! 3. **The open project's**: the ones it was saved with that the list
//!    doesn't have ([`App::place_phrase`]). The next project drops them.
//!
//! A phrase that joins the owner's run moves the project's up one, and
//! every track's pick with them ([`App::own_phrase`]), so a pick stays on
//! its phrase.
//!
//! A phrase moves notes, so it plays where the pads press notes into the
//! arp (a synth); on a kit the pads play as pressed, and NOTE REPEAT
//! repeats them.
//!
//! **The Phrase tab** (the Arp panel's fifth) shows the list a kind at a
//! time: MPFree's basses, riffs, chords and lines, then the owner's
//! (`mpfree_arp_ui::phrases::Shelf`). The app keeps which kind shows and
//! moves the pick:
//!
//! - The kind shown is the pick's. Only an empty one (the owner's, before
//!   they have made a phrase) shows with the pick elsewhere, so that it can
//!   be stepped to and say what it is for.
//! - The wheel moves the pick along the kind shown, in the order the
//!   screen shows it (the phrases that suit the key first), and stops at
//!   its ends.
//! - A tap on a kind, or - and + on the tab, shows that kind and picks on
//!   it: the phrase last picked there, else its first. So what is lit in
//!   the list is always what a held note plays, and stepping through the
//!   kinds is hearing one of each.

use crate::app::{App, Effect, View};
use mpfree_arp::{Library, Mode, Phrase, Scale};
use mpfree_arp_ui::phrases::{Shelf, Shelves};
use mpfree_arp_ui::PhraseList;

/// MPFree's starter phrases, by name, in the library's order.
pub fn built_in_phrases() -> Vec<(String, Phrase)> {
    Library::built_in()
        .phrases()
        .map(|(n, p)| (n.to_string(), *p))
        .collect()
}

impl App {
    /// The phrases to pick from: MPFree's own, the owner's, then the open
    /// project's. (The screens get them as [`Effect::Phrases`].)
    #[cfg(test)]
    pub fn phrases(&self) -> &[(String, Phrase)] {
        &self.phrases
    }

    /// The armed track's pick, as a place in the list.
    #[cfg(test)]
    pub fn phrase_index(&self) -> usize {
        self.phrase_index
    }

    /// The list's shelves, read where it is kept.
    fn shelves(&self) -> Shelves<'_> {
        Shelves {
            phrases: &self.phrases,
            built: self.phrases_built,
            own: self.phrases_own,
        }
    }

    /// The kind the Phrase tab's list shows, which the wheel moves along:
    /// the pick's, unless an empty one was stepped to on the panel. Away
    /// from the panel (the NOTE REPEAT band, which shows no kinds) it is
    /// always the pick's, so the wheel there never stands on an empty
    /// kind nobody sees.
    pub(super) fn shown_shelf(&self) -> Shelf {
        self.phrase_shelf
            .filter(|_| self.view == View::Arp)
            .unwrap_or_else(|| self.shelves().shelf_of(self.phrase_index))
    }

    /// The key the armed track's phrase follows In key now: the arp's own
    /// scale, or with that off the scale the pads play. The list is in the
    /// order that suits it.
    pub(super) fn phrase_key(&self) -> Option<Scale> {
        let e = self.arps.engine_at(self.track);
        e.settings().scale.or(e.key())
    }

    /// The list as the screens read it ([`Effect::Phrases`]): sent at the
    /// start and whenever it changes (the owner's library read, a project
    /// opened).
    pub(in crate::app) fn phrase_list(&self) -> PhraseList {
        PhraseList {
            phrases: self.phrases.clone(),
            built: self.phrases_built,
            own: self.phrases_own,
        }
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

    /// A phrase plays here: the armed track's instrument takes notes (its
    /// own synth, the shared synth, a kit played as notes), or the pads
    /// press notes into its arp; not a kit's pads. It asks the track's
    /// instrument (`TrackArps::on_notes`) as well as what the pads press,
    /// because while the pads play something of their own they press
    /// nothing into the track, and its phrase plays on.
    pub fn phrases_play(&self) -> bool {
        self.press_layout().is_some() || self.arps.on_notes(self.track)
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
        // The list shows the pick's kind, which remembers it.
        self.phrase_shelf = None;
        self.phrase_last[self.shelves().shelf_of(i).place()] = Some(i);
        self.send_phrase();
        vec![note]
    }

    /// The wheel on a phrase: the pick moves along the kind shown, in the
    /// order the list shows it, stopping at its ends. An empty kind has
    /// nothing to move along.
    pub(super) fn phrase_wheel(&mut self, delta: i32) -> Vec<Effect> {
        let rows = self.shelves().rows(self.shown_shelf(), self.phrase_key());
        let Some(at) = rows.iter().position(|r| r.index == self.phrase_index) else {
            return Vec::new();
        };
        let to = (at as i32 + delta).clamp(0, rows.len() as i32 - 1) as usize;
        if to == at {
            Vec::new()
        } else {
            self.pick_phrase(rows[to].index)
        }
    }

    /// The Phrase tab shows `shelf` (a tap on a kind, or - and +). The
    /// track picks on it: the phrase last picked there, else the first as
    /// the list shows it. An empty kind shows with the pick left where it
    /// is, and the pick's own kind shows again with the pick as it is.
    pub(super) fn show_shelf(&mut self, shelf: Shelf) -> Vec<Effect> {
        if shelf == self.shown_shelf() {
            return Vec::new();
        }
        if shelf == self.shelves().shelf_of(self.phrase_index) {
            self.phrase_shelf = None;
            self.arp_dirty = true;
            return Vec::new();
        }
        let rows = self.shelves().rows(shelf, self.phrase_key());
        let last = self.phrase_last[shelf.place()].filter(|i| rows.iter().any(|r| r.index == *i));
        match last.or(rows.first().map(|r| r.index)) {
            Some(i) => self.pick_phrase(i),
            None => {
                self.phrase_shelf = Some(shelf);
                self.arp_dirty = true;
                Vec::new()
            }
        }
    }

    /// The - and + buttons on Phrase: the kind `d` after the one shown,
    /// stopping at the last. None before the first: the step goes on to
    /// the tab before ([`App::arp_mode_step`]).
    pub(super) fn phrase_kind_step(&mut self, d: i32) -> Option<Vec<Effect>> {
        let to = self.shown_shelf().place() as i32 + d;
        let to = usize::try_from(to).ok()?.min(Shelf::ALL.len() - 1);
        Some(self.show_shelf(Shelf::ALL[to]))
    }

    /// The armed track's engine takes the track's pick (after a pick, or
    /// when the track or the project changed), or while New idea or More
    /// like this is open the try it shows (`arp/maker.rs`). The engine
    /// leaves a phrase that already plays alone. Allocates nothing.
    pub(super) fn send_phrase(&mut self) {
        if self.phrase_index >= self.phrases.len() {
            self.phrase_index = 0;
        }
        let phrase = self
            .maker_try()
            .or_else(|| self.sheet_try())
            .or_else(|| self.improvised())
            .or_else(|| self.phrases.get(self.phrase_index).map(|(_, p)| *p));
        if let Some(phrase) = phrase {
            self.arps.engine_at_mut(self.track).set_phrase(phrase);
        }
        self.arp_dirty = true;
    }

    /// Every track's engine takes the pads' key: the scale the pads play,
    /// which a phrase In key follows while its arp's own scale is off
    /// (`Engine::set_key`). The pads have one scale for the whole project,
    /// so it goes to every track, not only the armed one: a phrase latched
    /// on a track the pads have left follows a change of key as the pads
    /// do. It is a setting alone, so, like the tempo, it reaches all 128
    /// engines without marking any in use. Called from the one place the
    /// pads' settings go out (`App::pads_changed`). Allocates nothing.
    pub(in crate::app) fn send_key(&mut self) {
        let key = self.synth_pads.scale;
        for e in self.arps.engines_mut() {
            e.set_key(key);
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

    /// The owner's phrase `name` joins the list: in the place of the one
    /// that has its name among MPFree's and the owner's (ignoring case; it
    /// keeps the place and the name's spelling), else after the owner's
    /// others, before the open project's. The project's phrases move up
    /// one then, and every track's pick on them moves with them. Returns
    /// whether it took a place. The engines hear of it from
    /// [`App::send_all_phrases`].
    pub(super) fn own_phrase(&mut self, name: &str, phrase: Phrase) -> bool {
        if let Some(slot) = self.phrases[..self.phrases_own]
            .iter_mut()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
        {
            slot.1 = phrase;
            return true;
        }
        let at = self.phrases_own;
        self.phrases.insert(at, (name.to_string(), phrase));
        self.phrases_own += 1;
        let up = |pick: &mut usize| {
            if *pick >= at {
                *pick += 1;
            }
        };
        up(&mut self.phrase_index);
        self.track_ui
            .iter_mut()
            .for_each(|u| up(&mut u.phrase_index));
        // And what each kind remembers.
        self.phrase_last.iter_mut().flatten().for_each(up);
        false
    }

    /// A library file's phrases join the list as the owner's
    /// ([`App::own_phrase`]), and every track's engine takes its pick
    /// again. Returns how many were added and how many took a place.
    pub(super) fn take_phrases(&mut self, lib: &Library) -> (usize, usize) {
        let (mut added, mut replaced) = (0, 0);
        for (name, p) in lib.phrases() {
            if self.own_phrase(name, *p) {
                replaced += 1;
            } else {
                added += 1;
            }
        }
        if added + replaced > 0 {
            self.send_all_phrases();
        }
        (added, replaced)
    }

    /// Every track's engine takes its track's pick, after the list changed
    /// under the picks: a phrase a track picked may have other notes now.
    /// An engine leaves the phrase it has alone when it is the same, and
    /// one playing takes the new one on its line. A phrase is a setting
    /// alone, so no engine is marked in use. Allocates nothing.
    fn send_all_phrases(&mut self) {
        let (armed, pick) = (self.track, self.phrase_index);
        for (t, (e, u)) in self.arps.engines_mut().zip(&self.track_ui).enumerate() {
            let i = if t == armed { pick } else { u.phrase_index };
            if let Some((_, phrase)) = self.phrases.get(i) {
                e.set_phrase(*phrase);
            }
        }
        // The armed track's try, while New idea, More like this or Make a
        // phrase is open.
        self.send_phrase();
    }

    /// The owner's phrases as a library file, for the store's
    /// arp/phrases.txt: each of MPFree's that plays other notes than the
    /// one built in, and every one of the owner's own. None when the
    /// owner has none: nothing to keep.
    pub fn phrases_text(&self) -> Option<String> {
        let built_in = Library::built_in();
        let mut lib = Library::default();
        for (i, (name, p)) in self.phrases[..self.phrases_own].iter().enumerate() {
            if i >= self.phrases_built || built_in.phrase(name) != Some(p) {
                let _ = lib.put_phrase(name, *p);
            }
        }
        lib.phrases().next()?;
        Some(format!(
            "# MPFree phrases of your own, saved by the pad player. One per line:\n\
             # Name = what it is : its steps. The syntax is in docs/dev/design/arpeggiator.md.\n{}",
            lib.to_text()
        ))
    }

    /// The open project's own phrases go (another project is opening):
    /// the list is MPFree's and the owner's again.
    pub(super) fn drop_project_phrases(&mut self) {
        self.phrases.truncate(self.phrases_own);
    }

    /// The place in the list of the phrase a project kept as `name` and
    /// `phrase`: the list's own phrase of that name when it is the same
    /// phrase, else the project's copy, added at the end (under its name,
    /// or the name with "(project)" when the list has another phrase by
    /// it), so the project plays what it was saved with.
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
    use crate::app::SynthItem;
    use mpfree_arp::scale::pads::{layout, PadLayout, PadSetup};
    use mpfree_arp::scale::{by_name, Scale};
    use mpfree_arp::{Clock, EventKind, Events, Follow, Pattern, Rate, Speed};
    use mpfree_arp_ui::view::{ArpAction, PatternButton};
    use mpfree_arp_ui::workspace::WsAction;
    use mpfree_controller::mpc_one::button::{MINUS, NOTE_REPEAT, PLUS};
    use mpfree_seq::MAX_TRACKS;
    use mpfree_store::Settings;

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

    /// A button pressed and let go.
    fn press(app: &mut App, note: u8) {
        app.button(note, true, 0.0);
        app.button(note, false, 0.0);
    }

    /// The Arp panel open on its Phrase tab, the pads on a synth.
    fn on_the_phrase_tab() -> App {
        let mut app = ready();
        on_a_synth(&mut app);
        app.toggle_panel();
        assert_eq!(app.view(), View::Arp);
        app.arp_action(ArpAction::Mode(Mode::Phrase));
        app
    }

    /// The first phrase of kind `k` ([`Shelf::ALL`]) as the list shows it.
    fn first_of(app: &App, k: usize) -> usize {
        app.shelves().rows(Shelf::ALL[k], app.phrase_key())[0].index
    }

    /// `periods` of 128 frames with the transport playing at 120 BPM from
    /// period `*from` on: how many of them the arp screens heard of.
    fn play(app: &mut App, from: &mut u64, periods: u64) -> usize {
        let mut ev = Events::new();
        let mut heard = 0;
        for _ in 0..periods {
            app.set_clock(Some(Clock::playing(0, *from * 128, 120.0)));
            app.process(128, &mut ev);
            heard += usize::from(app.arp_poll().is_some());
            *from += 1;
        }
        heard
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
        // The wheel: the phrase before it in its kind (Up and back is the
        // last of the lines) waits for its line (the next step: the
        // transport is stopped).
        let kind: Vec<usize> = app
            .shelves()
            .rows(app.shown_shelf(), app.phrase_key())
            .iter()
            .map(|r| r.index)
            .collect();
        let at = kind
            .iter()
            .position(|i| *i == app.phrase_index())
            .expect("the pick is in the kind shown");
        assert_eq!(at + 1, kind.len());
        assert!(app.wheel(1).is_empty(), "the end of its kind");
        assert_eq!(app.wheel(-1).len(), 1);
        assert_eq!(app.phrase_index(), kind[at - 1]);
        assert_eq!(
            app.eng().phrase(),
            &app.phrases()[kind[at]].1,
            "still playing"
        );
        let next = app.phrases()[kind[at - 1]].1;
        ons(&mut app, bar);
        assert_eq!(app.eng().phrase(), &next);
        // The ends of the kind stop the wheel: it never runs on into the
        // next kind.
        assert!(app.wheel(1000).len() == 1 && app.wheel(1).is_empty());
        assert_eq!(app.phrase_index(), *kind.last().unwrap());
        assert!(app.wheel(-1000).len() == 1 && app.wheel(-1).is_empty());
        assert_eq!(app.phrase_index(), kind[0]);
        assert!(kind.len() < app.phrases().len(), "one kind, not the list");
    }

    /// On the panel's Phrase tab - and + step the kinds, each picking on
    /// its kind: the phrase last picked there, else its first. + past the
    /// last kind stops; - past the first goes back to Pattern.
    #[test]
    fn minus_and_plus_on_the_phrase_tab_step_the_kinds() {
        let mut app = on_the_phrase_tab();
        assert_eq!(app.shown_shelf(), Shelf::ALL[0], "Offbeat pump is a bass");
        // +: the riffs, on the first of them.
        press(&mut app, PLUS);
        assert_eq!(app.shown_shelf(), Shelf::ALL[1]);
        assert_eq!(app.phrase_index(), first_of(&app, 1));
        assert_eq!(app.eng().settings().mode, Mode::Phrase, "the tab stays");
        // The wheel picks another riff, and the kind remembers it.
        app.wheel(2);
        let riff = app.phrase_index();
        assert_ne!(riff, first_of(&app, 1));
        press(&mut app, PLUS);
        assert_eq!(app.phrase_index(), first_of(&app, 2), "the chords");
        press(&mut app, MINUS);
        assert_eq!(app.phrase_index(), riff, "the riff as it was left");
        // On to the owner's kind, which is empty: it shows, and the pick
        // stays where it was, on a line.
        for _ in 0..3 {
            press(&mut app, PLUS);
        }
        assert_eq!(app.shown_shelf(), Shelf::Own);
        let line = app.phrase_index();
        assert_eq!(line, first_of(&app, 3));
        assert_eq!(app.phrase_view().shelf, Some(Shelf::Own));
        // + past the last kind and the wheel on an empty kind: nothing.
        press(&mut app, PLUS);
        assert!(app.wheel(1).is_empty() && app.wheel(-1).is_empty());
        assert_eq!((app.shown_shelf(), app.phrase_index()), (Shelf::Own, line));
        // Back: the lines show again, with the pick as it was.
        press(&mut app, MINUS);
        assert_eq!(
            (app.shown_shelf(), app.phrase_index()),
            (Shelf::ALL[3], line)
        );
        assert_eq!(app.phrase_view().shelf, None);
        // Down to the basses, then past them: the Pattern tab.
        for _ in 0..3 {
            press(&mut app, MINUS);
        }
        assert_eq!(app.shown_shelf(), Shelf::ALL[0]);
        assert_eq!(app.eng().settings().mode, Mode::Phrase);
        press(&mut app, MINUS);
        assert_eq!(app.eng().settings().mode, Mode::Pattern);
        // + from there: the Phrase tab again, on the pick's kind.
        press(&mut app, PLUS);
        assert_eq!(app.eng().settings().mode, Mode::Phrase);
        assert_eq!(app.shown_shelf(), Shelf::ALL[0]);
    }

    /// A tap on a kind shows it and picks on it, as - and + do; the kind
    /// shown again does nothing, and the pick's own kind, tapped from the
    /// owner's empty one, shows again with nothing picked anew.
    #[test]
    fn a_tap_on_a_kind_shows_it_and_picks_on_it() {
        let mut app = on_the_phrase_tab();
        let chords = Shelf::ALL[2];
        let fx = app.arp_action(ArpAction::Shelf(chords));
        assert_eq!(fx.len(), 1, "the pick is said once: {fx:?}");
        assert_eq!(app.shown_shelf(), chords);
        let pick = app.phrase_index();
        assert_eq!(pick, first_of(&app, 2));
        assert!(app.arp_action(ArpAction::Shelf(chords)).is_empty());
        // A row tapped: the pick, which its kind remembers.
        let third = app.shelves().rows(chords, app.phrase_key())[2].index;
        app.arp_action(ArpAction::Phrase(third));
        assert_eq!(app.phrase_index(), third);
        app.arp_action(ArpAction::Shelf(Shelf::ALL[0]));
        app.arp_action(ArpAction::Shelf(chords));
        assert_eq!(app.phrase_index(), third);
        // The owner's empty kind, then the chords again: nothing picked.
        assert!(app.arp_action(ArpAction::Shelf(Shelf::Own)).is_empty());
        assert_eq!(app.shown_shelf(), Shelf::Own);
        assert!(app.arp_action(ArpAction::Shelf(chords)).is_empty());
        assert_eq!((app.shown_shelf(), app.phrase_index()), (chords, third));
        // With a phrase of the owner's, their kind picks like the others.
        app.load_phrases("Verse bass = bass minor : 1 . 1 . 5- . 1 . 1 . 1 . 5- . b7- .\n");
        app.arp_action(ArpAction::Shelf(Shelf::Own));
        assert_eq!(app.phrase_name(0), "Verse bass");
        // A row that is not in the list picks nothing.
        assert!(app.arp_action(ArpAction::Phrase(999)).is_empty());
        assert_eq!(app.phrase_name(0), "Verse bass");
    }

    /// On the Phrase tab Q-Link 1 is the phrase's speed, in its three
    /// steps, and the other three are the arp's as on every tab; the
    /// tab's In key | Exact switch and the band's speed buttons reach the
    /// engine the same way.
    #[test]
    fn on_the_phrase_tab_the_first_qlink_is_the_speed() {
        let mut app = on_the_phrase_tab();
        let rate = app.eng().settings().rate;
        assert_eq!(app.eng().settings().speed, Speed::AsWritten);
        app.qlink(0, 10_000, None);
        assert_eq!(app.eng().settings().speed, Speed::Double);
        app.qlink(0, -20_000, None);
        assert_eq!(app.eng().settings().speed, Speed::Half);
        // A controller that sends positions: the middle is as written.
        app.qlink(0, 0, Some(64));
        assert_eq!(app.eng().settings().speed, Speed::AsWritten);
        assert_eq!(app.eng().settings().rate, rate, "the rate is not touched");
        app.qlink(3, 10_000, None);
        assert_eq!(app.eng().settings().octaves, 4);
        // The switch and the band's buttons send the settings they change.
        let s = app.eng().settings();
        app.arp_action(ArpAction::Settings(mpfree_arp::Settings {
            follow: Follow::Exact,
            speed: Speed::Double,
            ..s
        }));
        let now = app.eng().settings();
        assert_eq!((now.follow, now.speed), (Follow::Exact, Speed::Double));
        assert_eq!((now.rate, now.octaves), (rate, 4));
        // On another tab the first Q-Link is the rate again.
        app.arp_action(ArpAction::Mode(Mode::Arp));
        app.qlink(0, 10_000, None);
        assert_eq!(app.eng().settings().rate, *Rate::ALL.last().unwrap());
        assert_eq!(app.eng().settings().speed, Speed::Double, "kept");
    }

    /// The phrase's speed is a parameter of its own: a learned control
    /// sets it as Q-Link 1 does, the XY pad's hand holds it until it lets
    /// go (the speed as set kept), and Stay keeps where it held.
    #[test]
    fn a_learned_control_and_the_xy_pad_turn_a_phrases_speed() {
        use mpfree_play::mpfree_param::arp as ids;
        let mut app = on_the_phrase_tab();
        let (info, v) = app.arp_param(0, ids::SPEED).unwrap();
        assert_eq!((info.name, v), ("Speed", 1.0));
        assert_eq!(mpfree_mod::text::value_text(&info, v), "As written");
        assert!(!app.set_arp_param(0, ids::SPEED, 2.0).is_empty());
        assert_eq!(app.eng().settings().speed, Speed::Double);
        assert!(
            app.set_arp_param(0, ids::SPEED, 2.4).is_empty(),
            "the same step"
        );
        // The pad's hand: half while it holds, the speed as set kept.
        assert!(app.arp_hand(0, ids::SPEED, Some(0.0)));
        assert_eq!(app.eng().live().speed, Speed::Half);
        assert_eq!(app.eng().settings().speed, Speed::Double);
        assert_eq!(app.arp_beneath(0, ids::SPEED), Some((2.0, 2.0)));
        assert!(app.arp_hand(0, ids::SPEED, None));
        assert_eq!(app.eng().live().speed, Speed::Double);
        // Stay: where it held becomes the speed as set.
        let (v, _) = app.keep_arp(0, ids::SPEED, 0.2).unwrap();
        assert_eq!(v, 0.0);
        assert_eq!(app.eng().settings().speed, Speed::Half);
    }

    /// NOTE REPEAT held away from the panel: on a synth - and + reach
    /// Phrase after Pattern, and no further (no kinds show there); the
    /// wheel moves the pick along its kind.
    #[test]
    fn note_repeats_minus_and_plus_reach_phrase_on_a_synth() {
        let mut app = ready();
        on_a_synth(&mut app);
        app.button(NOTE_REPEAT, true, 0.0);
        assert_eq!(app.eng().settings().mode, Mode::Repeat);
        for mode in [Mode::Arp, Mode::Pattern, Mode::Phrase, Mode::Phrase] {
            press(&mut app, PLUS);
            assert_eq!(app.eng().settings().mode, mode);
        }
        assert_eq!(app.phrase_index(), 0, "+ on Phrase steps no kind here");
        assert!(app.phrase_view().plays);
        app.wheel(1);
        let basses = app.shelves().rows(Shelf::ALL[0], app.phrase_key());
        assert_eq!(app.phrase_index(), basses[1].index);
        press(&mut app, MINUS);
        assert_eq!(app.eng().settings().mode, Mode::Pattern);
        assert_eq!(app.phrase_index(), basses[1].index);
        app.button(NOTE_REPEAT, false, 0.0);
    }

    /// A kit track with the panel left on Phrase: the tab says whose pads
    /// they are, the wheel has no list to move along, and NOTE REPEAT
    /// still repeats the pads, its - and + stepping Repeat, Arp and
    /// Pattern alone.
    #[test]
    fn a_kit_left_on_phrase_keeps_note_repeat() {
        let mut app = ready();
        app.set_track(2);
        app.toggle_panel();
        app.arp_action(ArpAction::Mode(Mode::Phrase));
        let v = app.phrase_view();
        assert_eq!((v.kit_track, v.plays), (Some(2), false));
        assert!(app.wheel(1).is_empty(), "no list");
        assert_eq!(app.phrase_index(), 0);
        // - and + still step the kinds beside the message, and - past the
        // first leaves the tab.
        press(&mut app, PLUS);
        assert_eq!(app.shown_shelf(), Shelf::ALL[1]);
        press(&mut app, MINUS);
        press(&mut app, MINUS);
        assert_eq!(app.phrase_view().kit_track, None, "the Pattern tab");
        app.arp_action(ArpAction::Mode(Mode::Phrase));
        app.toggle_panel();
        assert_eq!(app.phrase_view().kit_track, None, "the panel is closed");
        // NOTE REPEAT: the pad repeats.
        app.button(NOTE_REPEAT, true, 0.0);
        assert_eq!(app.eng().settings().mode, Mode::Repeat);
        app.pad_down(3, 100);
        let half = bar(&app) / 2;
        let got = ons(&mut app, half);
        assert!(got.len() >= 4 && got.iter().all(|n| *n == 3), "{got:?}");
        for mode in [Mode::Arp, Mode::Pattern, Mode::Pattern] {
            press(&mut app, PLUS);
            assert_eq!(app.eng().settings().mode, mode);
        }
        app.button(NOTE_REPEAT, false, 0.0);
    }

    /// What the tab draws that moves by itself: where the phrase is, to
    /// the 1/32 and only while the panel shows, and with the transport
    /// playing the pick that waits for the bar line beside the phrase
    /// still heard.
    #[test]
    fn the_tab_hears_where_the_phrase_is_and_what_waits() {
        let mut app = on_the_phrase_tab();
        pick_named(&mut app, "Up and back");
        let first = app.phrase_index();
        assert_eq!(app.phrase_view().at, None, "nothing plays yet");
        // NOTE REPEAT held, so the phrase plays on with the panel closed.
        app.button(NOTE_REPEAT, true, 0.0);
        app.pad_down(0, 100);
        // Half a bar at 120 BPM is 344 periods of 128 frames, and 16
        // thirty-seconds: the screens hear of each, not of every period.
        let mut period = 0;
        let heard = play(&mut app, &mut period, 344);
        assert!((14..=20).contains(&heard), "{heard} of 344 periods");
        let at = app.phrase_view().at.expect("the phrase plays");
        assert!((0.44..=0.5).contains(&at), "half way: {at}");
        assert_eq!((at * 32.0).fract(), 0.0, "a whole 1/32: {at}");
        // A pick while it plays waits for the bar line, and the tab is
        // told which phrase is still heard.
        app.wheel(-1);
        let v = app.phrase_view();
        assert!(v.waits);
        assert_eq!(v.playing, Some(first));
        assert_ne!(app.phrase_index(), first);
        assert_eq!(app.eng().phrase(), &app.phrases()[first].1);
        // At the bar line it comes in.
        play(&mut app, &mut period, 348);
        let v = app.phrase_view();
        assert_eq!((v.waits, v.playing), (false, None));
        assert_eq!(app.eng().phrase(), &app.phrases()[app.phrase_index()].1);
        // The panel closed: no screen follows the cursor, so they hear
        // only of the steps.
        pick_named(&mut app, "Up and back");
        play(&mut app, &mut period, 689);
        assert_eq!(app.eng().phrase(), &app.phrases()[first].1);
        app.toggle_panel();
        assert_ne!(app.view(), View::Arp);
        assert_eq!(app.eng().settings().mode, Mode::Phrase, "still held");
        let heard = play(&mut app, &mut period, 344);
        assert!((3..=7).contains(&heard), "{heard} of 344 periods");
        app.button(NOTE_REPEAT, false, 0.0);
    }

    /// In key, a phrase follows the scale the pads play, and a change of
    /// that scale while it plays: Up and back's second note is the third
    /// above the held one, a minor third from D in C major and a major
    /// third by semitones.
    #[test]
    fn in_key_a_phrase_follows_the_scale_the_pads_play() {
        let mut app = ready();
        let home = on_a_synth(&mut app);
        app.toggle_panel();
        app.arp_action(ArpAction::Mode(Mode::Phrase));
        pick_named(&mut app, "Up and back");
        // The pads panel's In key switch: C major.
        app.synth_pick_layout(true);
        let d = home + 2;
        app.pad_down(2, 100);
        let bar = bar(&app);
        // 1 3 5 6 1+ 6 5 3 from D in C major: D F A B D B A F.
        let in_key = [0, 3, 7, 9, 12, 9, 7, 3].map(|s| d + s);
        assert_eq!(ons(&mut app, bar), in_key);
        // The pads' scale off while it plays: by semitones from the next
        // note, with nothing started over.
        while app.synth_pads.scale.is_some() {
            app.step_synth_item(SynthItem::Scale, -1);
        }
        let exact = [0, 4, 7, 9, 12, 9, 7, 4].map(|s| d + s);
        assert_eq!(ons(&mut app, bar), exact);
        // The arp's own scale, when on, wins over the pads'.
        app.synth_pick_layout(false);
        app.step_synth_item(SynthItem::Scale, 1);
        assert!(app.synth_pads.scale.is_some());
        let minor = by_name("Minor").expect("a built-in scale");
        app.arp.scale = Some(Scale::of(0, minor));
        app.arp_changed();
        // From D in C minor (D E♭ F G A♭ B♭ C): D F A♭ B♭ D B♭ A♭ F.
        let in_minor = [0, 3, 6, 8, 12, 8, 6, 3].map(|s| d + s);
        assert_eq!(ons(&mut app, bar), in_minor);
    }

    /// The pads have one scale for the whole project, so its every change
    /// reaches every track's engine, the ones the pads are not on too,
    /// and wakes none of them.
    #[test]
    fn the_pads_key_reaches_every_tracks_engine() {
        let mut app = ready();
        let keys = |app: &App| [0, 1, 7, MAX_TRACKS - 1].map(|t| app.arps.engine_at(t).key());
        assert_eq!(keys(&app), [None; 4]);
        let used = app.arps.used();
        // The pads panel's In key switch picks Major on the root.
        app.synth_pick_layout(true);
        let major = app.synth_pads.scale;
        assert!(major.is_some());
        assert_eq!(keys(&app), [major; 4]);
        // The Synth tab's Scale row, then its Scale root row.
        app.step_synth_item(SynthItem::Scale, 1);
        let next = app.synth_pads.scale;
        assert!(next.is_some() && next != major);
        assert_eq!(keys(&app), [next; 4]);
        app.step_synth_item(SynthItem::Root, 2);
        let moved = app.synth_pads.scale;
        assert!(moved.is_some() && moved != next);
        assert_eq!(keys(&app), [moved; 4]);
        assert_eq!(app.arps.used(), used, "a key alone wakes no engine");
        // A project opening: its scale, and with none kept, none.
        let kept = app.synth_settings();
        let mut again = ready();
        again.open_synth(&kept);
        assert_eq!(keys(&again), [moved; 4]);
        again.open_synth(&Settings::default());
        assert_eq!(again.synth_pads.scale, None);
        assert_eq!(keys(&again), [None; 4]);
        // The Scale row back to Off.
        while app.synth_pads.scale.is_some() {
            app.step_synth_item(SynthItem::Scale, -1);
        }
        assert_eq!(keys(&app), [None; 4]);
        // Another track armed keeps the key: it is the pads', not a
        // track's.
        app.synth_pick_layout(true);
        app.set_track(3);
        assert_eq!(keys(&app), [app.synth_pads.scale; 4]);
    }

    /// The owner's phrases come from a library file of their own: a new
    /// one goes after MPFree's, one named as a phrase in the list takes
    /// its place, and Save keeps them in that file, never among the
    /// patterns.
    #[test]
    fn the_owners_phrases_are_kept_in_a_file_of_their_own() {
        let mut app = ready();
        let quiet = "bass minor : 1 . . . 1 . . . 1 . . . 1 . . .";
        let fx = app.load_phrases(&format!(
            "# mine\nVerse bass = bass minor : 1 . 1 . 5- . 1 . 1 . 1 . 5- . b7- .\n\
             night drive = {quiet}\n"
        ));
        assert_eq!(
            fx,
            vec![
                Effect::Phrases(app.phrase_list()),
                Effect::Note("arp phrases file: 1 phrases added, 1 replaced".into())
            ],
            "the screens get the new list"
        );
        assert_eq!((app.phrase_list().built, app.phrase_list().own), (64, 65));
        assert_eq!(app.phrases().len(), 65);
        assert_eq!(app.phrases()[64].0, "Verse bass");
        let at = app
            .phrases()
            .iter()
            .position(|(n, _)| n == "Night drive")
            .expect("it keeps its name as MPFree spells it");
        assert!(at < 64, "and its place");
        assert_eq!(app.phrases()[at].1, Phrase::parse(quiet).unwrap());
        // What Save keeps: those two, and none of MPFree's as built.
        let text = app.phrases_text().expect("the owner has phrases");
        let lib = Library::parse(&text).unwrap();
        let names: Vec<&str> = lib.phrases().map(|(n, _)| n).collect();
        assert_eq!(names, ["Night drive", "Verse bass"]);
        assert!(lib.is_empty(), "no patterns in it");
        assert!(!app.library_text().contains("Verse bass"));
        // The next start reads it back to the same list.
        let mut again = ready();
        again.load_phrases(&text);
        assert_eq!(again.phrases(), app.phrases());
        // With none of the owner's there is nothing to keep.
        assert_eq!(ready().phrases_text(), None);
        // A file with a bad line is not used.
        let before = app.phrases().to_vec();
        let fx = app.load_phrases("Fine = riff either : 1 3 5 3\nBad = riff either : 1 z\n");
        assert_eq!(app.phrases(), before);
        assert!(
            fx.contains(&Effect::Note(
                "arp phrases file not used (1 bad lines); MPFree's phrases stay".into()
            )),
            "{fx:?}"
        );

        // A phrase line in a patterns file is the owner's too, and Save
        // puts each kind in its own file, so it is never lost.
        let mut app = ready();
        let riff = "riff minor : 1 b3 5 b7 1+ b7 5 b3 1 b3 5 b7 1+ b7 5 b3";
        let fx = app.load_patterns(&format!("Skip = 1 . 3 2\nMy riff = {riff}\n"));
        for line in [
            "arp patterns file: 1 added, 0 built-in ones replaced, 0 scales of your own",
            "arp patterns file: 1 phrases added, 0 replaced",
        ] {
            assert!(fx.contains(&Effect::Note(line.into())), "{line} in {fx:?}");
        }
        app.pattern_file = true;
        app.toggle_panel();
        app.arp_action(ArpAction::Button(PatternButton::Edit));
        let fx = app.workspace_action(WsAction::Save(Pattern::parse("1 2 . 3").unwrap()));
        let saved: Vec<(bool, &str)> = fx
            .iter()
            .filter_map(|e| match e {
                Effect::SavePatterns(t) => Some((false, t.as_str())),
                Effect::SavePhrases(t) => Some((true, t.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(saved.len(), 2, "{fx:?}");
        let (patterns, phrases) = (saved[0], saved[1]);
        assert!(!patterns.0 && patterns.1.contains("Skip") && !patterns.1.contains("My riff"));
        assert!(phrases.0, "the phrases after the patterns");
        assert_eq!(
            Library::parse(phrases.1).unwrap().phrase("My riff"),
            Some(&Phrase::parse(riff).unwrap())
        );
    }

    /// A phrase joining the owner's moves the project's own up the list,
    /// and every track's pick stays on its phrase; one that takes a place
    /// reaches the engine of every track that picked it.
    #[test]
    fn a_pick_stays_on_its_phrase_when_the_owners_join_the_list() {
        let mut app = ready();
        on_a_synth(&mut app);
        app.toggle_panel();
        app.arp_action(ArpAction::Mode(Mode::Phrase));
        // Track 1 plays a phrase only the project has, track 2 one of
        // MPFree's.
        pick_named(&mut app, "Dub steps");
        let part = app
            .arp_part()
            .replace("phrase-name = Dub steps", "phrase-name = My riff");
        let (notes, _) = app.open_arp(Some(&part));
        assert_eq!(notes, Vec::<String>::new());
        assert_eq!((app.phrase_name(0), app.phrase_index()), ("My riff", 64));
        let mine = app.phrases()[64].1;
        app.set_track(1);
        app.arp_action(ArpAction::Mode(Mode::Phrase));
        pick_named(&mut app, "Sus lift");
        // The owner's file: a new phrase, and Sus lift with other notes.
        let lift = "chords either 1/8 : 1,3,5 . . . 1,3,5 . . .";
        app.load_phrases(&format!(
            "Verse bass = bass minor : 1 . 1 . 5- . 1 . 1 . 1 . 5- . b7- .\nSus lift = {lift}\n"
        ));
        let lift = Phrase::parse(lift).unwrap();
        assert_eq!(app.phrases()[64].0, "Verse bass");
        assert_eq!(app.phrases()[65], ("My riff".to_string(), mine));
        assert_eq!(app.phrase_name(0), "My riff", "one up, with its phrase");
        assert_eq!(app.phrase_name(1), "Sus lift");
        assert_eq!(
            app.eng().phrase(),
            &lift,
            "the armed track plays the new notes"
        );
        assert_eq!(app.arps.engine_at(0).phrase(), &mine);
        app.set_track(0);
        assert_eq!(app.phrase_index(), 65);
        assert_eq!(app.eng().phrase(), &mine);
        assert_eq!(app.arps.engine_at(1).phrase(), &lift);
        // A new project drops the project's own and keeps the owner's.
        app.open_arp(None);
        assert_eq!(app.phrases().len(), 65);
        assert_eq!(app.phrases()[64].0, "Verse bass");
        assert_eq!(
            app.phrases_text().map(|t| t.contains("Sus lift")),
            Some(true)
        );
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

    /// A track with a synth of its own keeps its phrase while the pads
    /// play something else on their own: they press nothing into the
    /// track then (main.rs `press_layout`), but its instrument still takes
    /// notes (`TrackArps::on_notes`), so its arp stays on Phrase, a
    /// latched phrase plays on, and the tab keeps its list. A kit track's
    /// instrument takes pad hits, and its pads play as pressed (repeated
    /// while latched).
    #[test]
    fn a_tracks_own_synth_keeps_its_phrase_while_the_pads_are_away() {
        let mut app = ready();
        let pads = layout(&PadSetup {
            layout: PadLayout::Chromatic,
            ..PadSetup::default()
        });
        app.arps.set_on_notes(0, true);
        app.set_arp_layout(Some(pads));
        app.toggle_panel();
        app.arp_action(ArpAction::Mode(Mode::Phrase));
        app.arp_action(ArpAction::Latch);
        app.pad_down(0, 100);
        app.pad_up(0);
        assert_eq!(app.eng().settings().mode, Mode::Phrase);
        assert!(!ons(&mut app, 22_050).is_empty(), "the phrase plays");
        // The pads go to a kit of their own: nothing pressed into the
        // track, which plays on.
        app.set_arp_layout(None);
        assert!(app.phrases_play());
        assert_eq!(app.eng().settings().mode, Mode::Phrase);
        assert!(!ons(&mut app, 22_050).is_empty(), "it plays on");
        // The track's instrument becomes a kit: no phrase, and with Latch
        // on, repeats of what is latched, as on a kit track.
        app.arps.set_on_notes(0, false);
        app.set_arp_layout(None);
        assert!(!app.phrases_play());
        assert_eq!(app.eng().settings().mode, Mode::Repeat);
    }

    /// What the pads play on their own (the pads' arp, sampler step 15c)
    /// never takes a phrase: a track on Phrase gives it the pads as
    /// pressed, and Repeat while NOTE REPEAT or Latch is on, and the
    /// track's own engine stays on its phrase all along. The other modes
    /// go there as they are.
    #[test]
    fn what_the_pads_play_on_their_own_never_takes_a_phrase() {
        let mut app = on_the_phrase_tab();
        let track = |app: &App| app.eng().settings().mode;
        assert_eq!(track(&app), Mode::Phrase);
        assert_eq!(app.pads_own_mode(), Mode::Direct, "as pressed");
        app.button(NOTE_REPEAT, true, 0.0);
        assert_eq!(track(&app), Mode::Phrase, "the track's phrase");
        assert_eq!(app.pads_own_mode(), Mode::Repeat, "NOTE REPEAT repeats");
        app.button(NOTE_REPEAT, false, 0.0);
        assert_eq!(app.pads_own_mode(), Mode::Direct);
        app.toggle_latch();
        assert_eq!(track(&app), Mode::Phrase, "a latched phrase plays on");
        assert_eq!(app.pads_own_mode(), Mode::Repeat, "Latch repeats");
        app.toggle_latch();
        for mode in [Mode::Repeat, Mode::Arp, Mode::Pattern] {
            app.arp_action(ArpAction::Mode(mode));
            assert_eq!((track(&app), app.pads_own_mode()), (mode, mode));
        }
        app.toggle_panel();
        assert_eq!(app.pads_own_mode(), Mode::Direct, "the panel closed");
    }

    /// The shared synth the pads play on their own presses notes, so the
    /// pads' arp plays the track's phrase there, even beside a kit track:
    /// the tab shows its list and its cursor follows the pads' phrase,
    /// while the kit track's own arp plays its pads as pressed.
    #[test]
    fn the_synth_the_pads_play_on_their_own_plays_the_tracks_phrase() {
        let mut app = ready();
        app.toggle_panel();
        app.arp_action(ArpAction::Mode(Mode::Phrase));
        let view = |app: &App| match app.arp_view() {
            Effect::Arp(v) => v,
            _ => unreachable!(),
        };
        assert!(view(&app).phrase.kit_track.is_some(), "a kit track's tab");
        app.set_pads_own(true, None);
        assert_eq!(app.pads_own_mode(), Mode::Phrase);
        assert_eq!(app.eng().settings().mode, Mode::Direct, "the kit track");
        let v = view(&app);
        assert_eq!(v.settings.mode, Mode::Phrase);
        assert!(v.phrase.plays && v.phrase.kit_track.is_none(), "the list");
        // The cursor follows the pads' phrase.
        let ticks = app.eng().phrase().ticks() as f32;
        app.set_pads_own(true, Some(f64::from(ticks) / 2.0));
        assert_eq!(view(&app).phrase.at, Some(0.5));
        // NOTE REPEAT and Latch keep it on the phrase.
        app.button(NOTE_REPEAT, true, 0.0);
        assert_eq!(app.pads_own_mode(), Mode::Phrase);
        app.button(NOTE_REPEAT, false, 0.0);
        // Back to the track: as pressed, and the kit panel again.
        app.set_pads_own(false, Some(1.0));
        assert_eq!(app.pads_own_mode(), Mode::Direct);
        let v = view(&app);
        assert!(v.phrase.kit_track.is_some() && v.phrase.at.is_none());
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
