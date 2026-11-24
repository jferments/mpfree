//! Improvise on the Phrase tab (composing tools S5, the boards' While I
//! hold; the design is in `docs/dev/design/arpeggiator.md`, "Improvise").
//!
//! The track's phrase plays while a note or a chord is held, as it always
//! has; over a chord (three notes or more) it plays as parts of the chord
//! (the engine's `HeldChord`). On top of that:
//!
//! - **Evolve** changes a little of the phrase every [`EVOLVE_BARS`] bars
//!   (`mpfree_compose::idea::evolve`: about a quarter of its places, never
//!   the last, the rhythm kept). The next change is made a cycle ahead, so
//!   the screen outlines it, and goes to the engine a 1/16 before the
//!   phrase comes round ([`LEAD`]), while its last note plays: the engine
//!   takes it in place (`Engine::retune_phrase`), the playhead keeping its
//!   place.
//! - **Hold** keeps the phrase as it plays now (Evolve off, the default).
//! - **Return** goes back to the phrase as picked.
//! - **Capture** keeps the last bars played as a clip: CAPTURE, which
//!   hears the arp's notes.
//!
//! What Evolve made belongs to the track's pick on the armed track: a new
//! pick, another track, or a try of New idea or More like this starts it
//! again from the phrase as picked. When I stop (Answer) comes with its
//! own step; until then its half of the Plays row is faint.

use crate::app::{App, Effect};
use mpfree_arp::scale::NOTE_NAMES;
use mpfree_arp::{HeldChord, Phrase, PPQ};
use mpfree_arp_ui::phrase_tab::{Sketch, SketchNote};
use mpfree_arp_ui::view::{ImproviseAction, ImproviseNow, PlaysWhen};
use mpfree_compose::idea::{evolve, NewIdea};
use mpfree_compose::GROOVE;
use mpfree_time_ui::TimeAction;

/// Evolve changes the phrase once in this many bars: a phrase of 1 or 2
/// bars every 4 or 2 times round, one of 4 bars or more every time.
pub const EVOLVE_BARS: u32 = 4;

/// How far ahead of the phrase coming round the next Evolve goes to the
/// engine, in written ticks: a 1/16, while the last note plays (Evolve
/// never changes the last).
const LEAD: f64 = (PPQ / 4) as f64;

/// Generations tried for one that keeps the rhythm note for note (a
/// chord's new notes can be more or fewer than its old), so the engine
/// can take it in place.
const TRIES: u32 = 4;

/// Improvise's state on the armed track's pick.
#[derive(Clone, Debug, Default)]
pub(in crate::app) struct Improvise {
    pub when: PlaysWhen,
    /// Evolve on; off is Hold.
    pub evolve: bool,
    /// Times Evolve has changed the phrase since it was picked.
    generation: u32,
    /// What the evolutions are of: the track and its pick, and the phrase
    /// as picked.
    of: Option<(usize, usize, Phrase)>,
    /// The phrase as it plays now, Evolve's.
    now: Option<Phrase>,
    /// The next Evolve's phrase, made a cycle ahead.
    next: Option<Phrase>,
    /// Times round since the last change, and the playhead last heard,
    /// in written ticks.
    passes: u32,
    last_at: f64,
    /// The next phrase went to the engine this time round.
    sent: bool,
    /// Where the bars to the next Evolve stand: for the screen's line.
    bars_left: Option<u32>,
}

/// A chord as the line under the picture names it: "F major", "A minor",
/// "G7", "Cmaj7", "Bdim", "Esus".
pub fn chord_name(c: &HeldChord) -> String {
    let root = NOTE_NAMES[usize::from(c.root % 12)];
    match (c.third, c.fifth, c.seventh) {
        (Some(4), Some(8), _) => format!("{root}aug"),
        (Some(3), Some(6), _) => format!("{root}dim"),
        (Some(4), _, Some(10)) => format!("{root}7"),
        (Some(4), _, Some(11)) => format!("{root}maj7"),
        (Some(3), _, Some(10 | 11)) => format!("{root}m7"),
        (Some(4), _, _) => format!("{root} major"),
        (Some(3), _, _) => format!("{root} minor"),
        (Some(_), _, _) => format!("{root}sus"),
        (None, _, _) => format!("{root}5"),
    }
}

/// The notes of `next` that differ from `now`'s at the same place, for the
/// picture to outline.
fn changes(now: &Phrase, next: &Phrase) -> Vec<SketchNote> {
    let was = Sketch::of(now).notes;
    Sketch::of(next)
        .notes
        .into_iter()
        .enumerate()
        .filter(|(i, n)| was.get(*i) != Some(n))
        .map(|(_, n)| n)
        .collect()
}

/// Times round a phrase of `bars` bars plays for each Evolve.
fn cycle(bars: u32) -> u32 {
    (EVOLVE_BARS / bars.max(1)).max(1)
}

impl App {
    /// A touch on the Plays row or a move.
    pub(super) fn improvise_action(&mut self, a: ImproviseAction) -> Vec<Effect> {
        self.arp_dirty = true;
        match a {
            // Answer comes with its own step: When I stop waits for it.
            ImproviseAction::Plays(PlaysWhen::WhenIStop) => Vec::new(),
            ImproviseAction::Plays(w) => {
                self.improvise.when = w;
                Vec::new()
            }
            ImproviseAction::Hold => {
                self.improvise.evolve = false;
                vec![Effect::Note("improvise: hold".to_string())]
            }
            ImproviseAction::Evolve => {
                self.improvise.evolve = true;
                self.improvise_follow_pick();
                self.make_next();
                vec![Effect::Note("improvise: evolve".to_string())]
            }
            ImproviseAction::Return => {
                let im = &mut self.improvise;
                im.generation = 0;
                im.now = None;
                im.next = None;
                im.passes = 0;
                im.sent = false;
                self.make_next();
                if let Some((_, _, first)) = &self.improvise.of {
                    let first = *first;
                    self.arps.engine_at_mut(self.track).retune_phrase(first);
                }
                vec![Effect::Note("improvise: return".to_string())]
            }
            ImproviseAction::Capture => vec![Effect::Time(TimeAction::Capture)],
        }
    }

    /// The phrase Evolve made, when it is of the armed track's pick as it
    /// is now: what the track's engine plays in its place
    /// (`App::send_phrase`).
    pub(super) fn improvised(&self) -> Option<Phrase> {
        let im = &self.improvise;
        match (&im.of, &im.now) {
            (Some((t, i, _)), Some(now)) if *t == self.track && *i == self.phrase_index => {
                Some(*now)
            }
            _ => None,
        }
    }

    /// Evolve starts again from the pick when the pick or the track
    /// changed, and stands aside while a try plays in its place. True when
    /// it started again on a pick.
    fn improvise_follow_pick(&mut self) -> bool {
        let here = (self.track, self.phrase_index);
        let trying = self.maker.is_some() || self.sheet.is_some();
        let same = matches!(&self.improvise.of, Some((t, i, _)) if (*t, *i) == here);
        if same && !trying {
            return false;
        }
        let pick = self.phrases.get(self.phrase_index).map(|(_, p)| *p);
        let im = &mut self.improvise;
        if trying && im.of.is_none() {
            return false;
        }
        im.generation = 0;
        im.now = None;
        im.next = None;
        im.passes = 0;
        im.sent = false;
        im.of = if trying {
            None
        } else {
            pick.map(|p| (here.0, here.1, p))
        };
        im.of.is_some()
    }

    /// The request Evolve's new notes are made to: the phrase's kind and
    /// bars, minor or major as the key it follows, a seed of its own.
    fn evolve_request(&self, p: &Phrase) -> NewIdea {
        let minor = self
            .phrase_key()
            .is_none_or(|k| !(k.mask & (1 << 4) != 0 && k.mask & (1 << 3) == 0));
        NewIdea {
            kind: p.kind,
            style: GROOVE,
            minor,
            bars: u8::try_from(p.ticks() / p.bar_ticks().max(1))
                .unwrap_or(1)
                .max(1),
            activity: 0.5,
            seed: 0x5e_ed ^ self.phrase_index as u64,
            attempt: 0,
        }
    }

    /// The next Evolve's phrase, from the one playing now: the first of
    /// the next few generations that keeps the rhythm note for note.
    fn make_next(&mut self) {
        let Some((_, _, first)) = &self.improvise.of else {
            return;
        };
        let now = self.improvise.now.unwrap_or(*first);
        let req = self.evolve_request(&now);
        let g = self.improvise.generation;
        let same = |p: &Phrase| {
            p.notes().len() == now.notes().len()
                && p.notes()
                    .iter()
                    .zip(now.notes())
                    .all(|(a, b)| a.start == b.start && a.len == b.len)
        };
        self.improvise.next = (1..=TRIES)
            .filter_map(|k| evolve(&now, &req, g + k))
            .find(|p| same(p) && *p != now);
    }

    /// Once a period, from [`App::arp_poll`]: Evolve's clock. It counts
    /// the phrase's times round from where the engine's playhead is, sends
    /// the next phrase a [`LEAD`] before the one that is due, and makes the
    /// one after.
    pub(super) fn improvise_poll(&mut self) {
        if self.view_is_not_phrase() {
            return;
        }
        if self.improvise_follow_pick() && self.improvise.evolve {
            self.make_next();
        }
        let e = self.arps.engine_at(self.track);
        let ticks = f64::from(e.phrase().ticks().max(1));
        let bar = f64::from(e.phrase().bar_ticks().max(1));
        let bars = (ticks / bar).round().max(1.0) as u32;
        let at = self.pads_phrase_at.or(e.phrase_at());
        let im = &mut self.improvise;
        let Some(at) = at.filter(|_| im.evolve) else {
            let left = None;
            if im.bars_left != left {
                im.bars_left = left;
                self.arp_dirty = true;
            }
            return;
        };
        if at + 1.0 < im.last_at {
            if im.sent {
                im.passes = 0;
                im.sent = false;
            } else {
                im.passes += 1;
            }
        }
        im.last_at = at;
        let due = im.passes + 1 >= cycle(bars);
        let left = (cycle(bars).saturating_sub(im.passes)) * bars - (at / bar) as u32;
        let left = Some(if im.sent { cycle(bars) * bars } else { left });
        if im.bars_left != left {
            im.bars_left = left;
            self.arp_dirty = true;
        }
        if due && !im.sent && at >= ticks - LEAD {
            if let Some(next) = im.next.take() {
                im.now = Some(next);
                im.generation += 1;
                im.sent = true;
                self.arps.engine_at_mut(self.track).retune_phrase(next);
                self.make_next();
                self.arp_dirty = true;
            }
        }
    }

    /// Evolve keeps time only while the Phrase tab's mode plays.
    fn view_is_not_phrase(&self) -> bool {
        self.arp_mode != mpfree_arp::Mode::Phrase
    }

    /// What the Phrase tab shows of Improvise.
    pub(super) fn improvise_view(&self) -> ImproviseNow {
        let im = &self.improvise;
        let mine = self.improvised().is_some();
        let now = self
            .improvised()
            .or_else(|| self.phrases.get(self.phrase_index).map(|(_, p)| *p));
        let next = match (&now, &im.next, im.evolve) {
            (Some(now), Some(next), true) => changes(now, next),
            _ => Vec::new(),
        };
        ImproviseNow {
            when: im.when,
            evolve: im.evolve,
            generation: if mine { im.generation } else { 0 },
            sketch: self.improvised().map(|p| Sketch::of(&p)),
            next,
            evolves_in: im.bars_left,
            chord: self
                .arps
                .engine_at(self.track)
                .chord_held()
                .map(|c| chord_name(&c)),
            answers: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::View;
    use mpfree_arp::scale::pads::{layout, PadLayout, PadSetup};
    use mpfree_arp::{Clock, EventKind, Events, Mode};
    use mpfree_arp_ui::view::ArpAction;

    const SR: u32 = 44_100;

    /// The Arp panel on its Phrase tab, the pads on a synth (pad 1 is C2),
    /// `name` picked and pads 1, 5 and 8 latched: a chord, C E G.
    fn on_the_phrase_tab(name: &str) -> App {
        let mut app = App::new(SR, false);
        app.start();
        app.set_pad_layout(Some(layout(&PadSetup {
            layout: PadLayout::Chromatic,
            ..PadSetup::default()
        })));
        app.toggle_panel();
        assert_eq!(app.view(), View::Arp);
        app.arp_action(ArpAction::Mode(Mode::Phrase));
        let i = app
            .phrases
            .iter()
            .position(|(n, _)| n == name)
            .expect("that phrase");
        app.pick_phrase(i);
        app.arp_action(ArpAction::Latch);
        for pad in [0, 4, 7] {
            app.pad_down(pad, 100);
        }
        for pad in [0, 4, 7] {
            app.pad_up(pad);
        }
        app
    }

    /// `bars` bars of the transport playing at 120 BPM from period
    /// `*from` on (a bar is 2 s, 689 periods of 128 frames): the notes
    /// that started.
    fn play(app: &mut App, from: &mut u64, bars: f64) -> Vec<u8> {
        let mut ev = Events::new();
        let mut out = Vec::new();
        for _ in 0..(bars * 2.0 * f64::from(SR) / 128.0).round() as u64 {
            app.set_clock(Some(Clock::playing(0, *from * 128, 120.0)));
            app.process(128, &mut ev);
            out.extend(ev.as_slice().iter().filter_map(|e| match e.kind {
                EventKind::On { note, .. } => Some(note),
                EventKind::Off { .. } => None,
            }));
            app.arp_poll();
            *from += 1;
        }
        out
    }

    fn engine_phrase(app: &App) -> Phrase {
        *app.arps.engine_at(app.track).phrase()
    }

    fn starts(p: &Phrase) -> Vec<(u32, u32)> {
        p.notes().iter().map(|n| (n.start, n.len)).collect()
    }

    #[test]
    fn a_chord_is_named_as_a_musician_says_it() {
        let named = |held: &[u8]| HeldChord::of(held).map(|c| chord_name(&c));
        assert_eq!(named(&[65, 69, 72]).as_deref(), Some("F major"));
        assert_eq!(named(&[57, 60, 64]).as_deref(), Some("A minor"));
        assert_eq!(named(&[55, 59, 62, 65]).as_deref(), Some("G7"));
        assert_eq!(named(&[60, 64, 67, 71]).as_deref(), Some("Cmaj7"));
        assert_eq!(named(&[60, 64]), None);
    }

    #[test]
    fn evolve_comes_round_every_four_bars() {
        assert_eq!(cycle(1), 4);
        assert_eq!(cycle(2), 2);
        assert_eq!(cycle(4), 1);
        assert_eq!(cycle(8), 1);
    }

    /// Hold (the default) plays the phrase as picked however long it
    /// plays; Evolve changes a little of it every 4 bars, its rhythm kept
    /// and the playhead never restarted, the next change shown ahead;
    /// Return plays the phrase as picked again.
    #[test]
    fn evolve_changes_the_phrase_every_four_bars_and_return_goes_back() {
        let mut app = on_the_phrase_tab("Minor hook");
        let first = engine_phrase(&app);
        assert_eq!(first.ticks(), 2 * first.bar_ticks(), "two bars");
        let mut from = 0;
        play(&mut app, &mut from, 8.0);
        assert_eq!(engine_phrase(&app), first, "Hold: as picked");
        let v = app.improvise_view();
        assert!(!v.evolve && v.generation == 0 && v.next.is_empty());
        assert_eq!(v.chord.as_deref(), Some("C major"));

        app.arp_action(ArpAction::Improvise(ImproviseAction::Evolve));
        let v = app.improvise_view();
        assert!(v.evolve);
        assert!(!v.next.is_empty(), "the next change is shown ahead");
        // Within 4 bars it changes, once.
        let mut changed_at = None;
        for half in 1..=8 {
            play(&mut app, &mut from, 0.5);
            if changed_at.is_none() && engine_phrase(&app) != first {
                changed_at = Some(half);
            }
        }
        assert!(changed_at.is_some(), "no change in 4 bars");
        let one = engine_phrase(&app);
        assert_eq!(starts(&one), starts(&first), "the rhythm stays");
        assert_eq!(app.improvise_view().generation, 1);
        assert_eq!(app.improvised(), Some(one));
        // 4 bars on: the second change.
        play(&mut app, &mut from, 4.0);
        assert_eq!(app.improvise_view().generation, 2);
        assert_ne!(engine_phrase(&app), one);
        assert!(app.arps.engine_at(app.track).phrase_waiting().is_none());

        // Hold keeps it as it is now.
        app.arp_action(ArpAction::Improvise(ImproviseAction::Hold));
        let held = engine_phrase(&app);
        play(&mut app, &mut from, 8.0);
        assert_eq!(engine_phrase(&app), held);
        assert_eq!(app.improvise_view().evolves_in, None);

        // Return: the phrase as picked.
        app.arp_action(ArpAction::Improvise(ImproviseAction::Return));
        play(&mut app, &mut from, 0.25);
        assert_eq!(engine_phrase(&app), first);
        assert_eq!(app.improvise_view().generation, 0);
    }

    /// A new pick starts Evolve again from it; Capture asks for CAPTURE;
    /// When I stop waits for Answer.
    #[test]
    fn a_new_pick_starts_again_and_capture_is_captures() {
        let mut app = on_the_phrase_tab("Minor hook");
        app.arp_action(ArpAction::Improvise(ImproviseAction::Evolve));
        let mut from = 0;
        play(&mut app, &mut from, 4.0);
        assert!(app.improvise_view().generation >= 1);
        let i = app
            .phrases
            .iter()
            .position(|(n, _)| n == "Blues lick")
            .unwrap();
        app.pick_phrase(i);
        play(&mut app, &mut from, 0.5);
        let v = app.improvise_view();
        assert_eq!(v.generation, 0);
        assert!(v.evolve, "Evolve stays on");
        assert_eq!(engine_phrase(&app), app.phrases[i].1);
        assert_eq!(
            app.arp_action(ArpAction::Improvise(ImproviseAction::Capture)),
            [Effect::Time(TimeAction::Capture)]
        );
        app.arp_action(ArpAction::Improvise(ImproviseAction::Plays(
            PlaysWhen::WhenIStop,
        )));
        assert_eq!(app.improvise_view().when, PlaysWhen::WhileIHold);
    }
}
