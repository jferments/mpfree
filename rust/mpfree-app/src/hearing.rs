//! Hearing notes in the note editor (piano roll plan 1C.2): a note picked,
//! drawn or moved plays through its track's instrument for its length, up
//! to a second. The screen asks ([`Ask`], `FromUi::Hear`); the sound loop
//! starts the notes, lets each go when its time is up, and lets them all
//! go on the next ask, on a stop the screen sends (leaving the Notes view,
//! another clip, a drag called off) and when the transport starts or
//! stops. Nothing heard is ever recorded: the notes go straight to what
//! plays them, never through the pads' road to the recorder.
//!
//! What plays them is the session's ([`Sound`]): a track with an
//! instrument of its own (or another track's) hears them as the
//! instruments' own player (`instruments::AUDITION`), so letting one go
//! never cuts the same note a clip holds there; a track on the synth hears
//! them by number, counted with the synth's other holders; a kit track
//! hits the kit's pad, as Steps' audition does.

/// The longest a note is heard, seconds.
pub const LONGEST: f64 = 1.0;
/// The most keys sounding at once: a chord of the editor's most heard
/// notes (`editor::HEARD_MAX`), each a pad of chords' notes at most.
const MOST: usize = 8 * 4;

/// What the screen asks to hear: notes of a clip on `track`, pads (a clip
/// of pad hits, `hits`) or MIDI notes, each `(note, velocity, seconds)`.
/// No notes: everything heard lets go.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ask {
    pub track: usize,
    pub hits: bool,
    pub notes: Vec<(u8, u8, f64)>,
}

/// Where a heard key sounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum On {
    /// The instrument the track plays, as the audition's own player.
    Instrument,
    /// The synth on the pads, by note number.
    Synth,
    /// Kit `k`'s pad.
    Kit(usize),
}

/// What plays the notes heard (the session's instruments, synth and
/// kits; a test's record).
pub trait Sound {
    /// Note or pad `key` of a clip on `track` (`hits`: a pad) starts at
    /// `velocity`: each key it started, and where, into `out` (a pad of
    /// chords starts several notes; nothing that plays it, none).
    fn start(&mut self, track: usize, hits: bool, key: u8, velocity: u8, out: &mut Vec<(On, u8)>);
    /// One key [`Sound::start`] started ends.
    fn end(&mut self, on: On, key: u8);
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Sounding {
    on: On,
    key: u8,
    until: f64,
}

/// The keys heard now and when each lets go. Allocates once.
#[derive(Debug)]
pub struct Hearing {
    sounding: Vec<Sounding>,
    started: Vec<(On, u8)>,
}

impl Default for Hearing {
    fn default() -> Hearing {
        Hearing {
            sounding: Vec::with_capacity(MOST),
            started: Vec::with_capacity(MOST),
        }
    }
}

impl Hearing {
    /// Nothing heard now.
    pub fn is_empty(&self) -> bool {
        self.sounding.is_empty()
    }

    /// Everything heard lets go.
    pub fn stop(&mut self, s: &mut impl Sound) {
        for k in self.sounding.drain(..) {
            s.end(k.on, k.key);
        }
    }

    /// `ask`'s notes start at `now` (seconds), each to let go after its
    /// time (at most [`LONGEST`]), what was heard before letting go first.
    pub fn hear(&mut self, ask: &Ask, now: f64, s: &mut impl Sound) {
        self.stop(s);
        for &(note, velocity, secs) in &ask.notes {
            self.started.clear();
            s.start(ask.track, ask.hits, note, velocity, &mut self.started);
            let until = now + secs.clamp(0.0, LONGEST);
            for &(on, key) in &self.started {
                if self.sounding.len() < MOST {
                    self.sounding.push(Sounding { on, key, until });
                } else {
                    s.end(on, key);
                }
            }
        }
    }

    /// The time is `now`: each key whose time is up lets go.
    pub fn tick(&mut self, now: f64, s: &mut impl Sound) {
        let mut k = 0;
        while k < self.sounding.len() {
            if self.sounding[k].until <= now {
                let gone = self.sounding.swap_remove(k);
                s.end(gone.on, gone.key);
            } else {
                k += 1;
            }
        }
    }
}

/// The shortest gap between asks, seconds: a run of them (a fast drag
/// across rows, a Q-Link turned quickly) is thinned to the last within
/// it, so it never floods the instrument.
pub const GAP: f32 = 0.07;

/// The screen's side (gui.rs): what to ask for, and when. It follows the
/// editor's heard notes (`editor::Heard`, numbered so each is heard
/// once) and a note dragged across rows, and asks for a stop when the
/// Notes view goes, the clip changes or a drag ends.
#[derive(Debug, Default)]
pub struct Asker {
    /// The clip the editor shows, and its heard number last seen.
    seen: Option<((usize, usize), u64)>,
    /// The row a dragged note was last heard on, while the drag lasts.
    dragged: Option<u8>,
    /// Seconds since the last ask went, and the ask waiting for the gap.
    since: f32,
    waiting: Option<Ask>,
    /// The last ask that went had notes: they may sound still.
    sounding: bool,
}

use mpfree_seq::Note;

impl Asker {
    /// The editor as the screen has it now: its clip `(track, slot)`
    /// while the Notes view shows it (None: it doesn't), whether the clip
    /// holds pad hits, how many seconds a clip tick lasts, its heard
    /// notes and their number, and whether notes are heard now (the
    /// Sound chip, the transport). An ask to send, if one is due.
    pub fn editor(
        &mut self,
        clip: Option<(usize, usize)>,
        hits: bool,
        secs_per_tick: f64,
        heard: (u64, &[Note]),
        hears: bool,
    ) -> Option<Ask> {
        let Some(clip) = clip else {
            self.seen = None;
            self.dragged = None;
            return self.stop_now();
        };
        match self.seen {
            Some((was, n)) if was == clip => {
                if n == heard.0 {
                    return None;
                }
                self.seen = Some((clip, heard.0));
                if !hears || heard.1.is_empty() {
                    return None;
                }
                let notes = heard.1.iter().map(|&n| as_heard(n, secs_per_tick));
                self.ask(Ask {
                    track: clip.0,
                    hits,
                    notes: notes.collect(),
                })
            }
            // Another clip, or the first look: what was heard before is
            // not this clip's.
            _ => {
                self.seen = Some((clip, heard.0));
                self.dragged = None;
                self.stop_now()
            }
        }
    }

    /// `dt` seconds on, with the note a finger drags across the roll as
    /// it would land (None: none dragged) on track `track`: an ask to
    /// send, if one is due. A drag heard on a new row asks for it; a drag
    /// ending asks for a stop (the notes it lands come as heard notes).
    pub fn tick(
        &mut self,
        dt: f32,
        drag: Option<(usize, bool, Note)>,
        secs_per_tick: f64,
        hears: bool,
    ) -> Option<Ask> {
        self.since += dt.max(0.0);
        match drag {
            Some((track, hits, n)) if hears && self.dragged != Some(n.note) => {
                self.dragged = Some(n.note);
                self.waiting = Some(Ask {
                    track,
                    hits,
                    notes: vec![as_heard(n, secs_per_tick)],
                });
            }
            None if self.dragged.take().is_some() && self.sounding => {
                self.waiting.get_or_insert_with(Ask::default);
            }
            _ => {}
        }
        if self.since >= GAP {
            let ask = self.waiting.take()?;
            return self.went(ask);
        }
        None
    }

    /// `ask` goes now if the gap has passed, else it waits (in place of
    /// any ask waiting).
    fn ask(&mut self, ask: Ask) -> Option<Ask> {
        if self.since >= GAP {
            self.waiting = None;
            return self.went(ask);
        }
        self.waiting = Some(ask);
        None
    }

    /// A stop at once, if something may sound (nothing waits after it).
    fn stop_now(&mut self) -> Option<Ask> {
        self.waiting = None;
        if !self.sounding {
            return None;
        }
        self.went(Ask::default())
    }

    fn went(&mut self, ask: Ask) -> Option<Ask> {
        self.since = 0.0;
        self.sounding = !ask.notes.is_empty();
        Some(ask)
    }
}

/// A note as heard: its pitch (or pad), velocity and length in seconds,
/// at most [`LONGEST`].
fn as_heard(n: Note, secs_per_tick: f64) -> (u8, u8, f64) {
    let secs = (n.len.max(1) as f64 * secs_per_tick).clamp(0.0, LONGEST);
    (n.note, n.velocity, secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The screen asks once for each touch's notes, thins a fast run of
    /// them, follows a drag across rows, and asks for a stop when the
    /// view or the clip changes or a drag ends.
    #[test]
    fn the_screen_asks_once_a_touch_and_thins_a_fast_run() {
        let nt = |n, v, len| Note::new(0, len, n, v);
        let mut a = Asker::default();
        let spt = 0.5 / 960.0;
        let c = Some((2, 0));
        // The first look hears nothing: the number is from before.
        assert_eq!(
            a.editor(c, false, spt, (5, &[nt(60, 100, 240)]), true),
            None
        );
        assert_eq!(
            a.editor(c, false, spt, (5, &[nt(60, 100, 240)]), true),
            None
        );
        // A note picked: heard, its length in seconds.
        a.tick(1.0, None, spt, true);
        let ask = a
            .editor(c, false, spt, (6, &[nt(62, 90, 960)]), true)
            .unwrap();
        assert_eq!(ask.track, 2);
        assert_eq!(ask.notes, [(62, 90, 0.5)]);
        // Another at once waits for the gap, then goes; the Sound chip
        // off hears nothing.
        assert_eq!(
            a.editor(c, false, spt, (7, &[nt(64, 90, 96_000)]), true),
            None
        );
        assert_eq!(a.tick(0.01, None, spt, true), None);
        let ask = a.tick(GAP, None, spt, true).unwrap();
        assert_eq!(ask.notes, [(64, 90, LONGEST)], "a second at most");
        a.tick(1.0, None, spt, true);
        assert_eq!(
            a.editor(c, false, spt, (8, &[nt(65, 90, 240)]), false),
            None
        );
        // A drag across rows: each new row heard, thinned; it ends: stop.
        let drag = |n| Some((2, false, nt(n, 100, 240)));
        assert_eq!(a.tick(0.0, drag(60), spt, true).unwrap().notes[0].0, 60);
        assert_eq!(a.tick(0.01, drag(61), spt, true), None);
        assert_eq!(a.tick(0.01, drag(62), spt, true), None);
        assert_eq!(a.tick(GAP, drag(62), spt, true).unwrap().notes[0].0, 62);
        a.tick(1.0, drag(62), spt, true);
        assert_eq!(a.tick(0.0, None, spt, true), Some(Ask::default()));
        // The view goes while a note may sound: a stop at once; again,
        // nothing (nothing sounds).
        a.tick(1.0, None, spt, true);
        a.editor(c, false, spt, (9, &[nt(67, 90, 240)]), true)
            .unwrap();
        assert_eq!(
            a.editor(None, false, spt, (9, &[]), true),
            Some(Ask::default())
        );
        assert_eq!(a.editor(None, false, spt, (9, &[]), true), None);
        // Another clip: a stop, and its number is from before.
        a.editor(c, false, spt, (9, &[]), true);
        a.tick(1.0, None, spt, true);
        a.editor(c, true, spt, (10, &[nt(3, 90, 240)]), true)
            .unwrap();
        assert_eq!(
            a.editor(Some((3, 1)), false, spt, (40, &[nt(1, 1, 1)]), true),
            Some(Ask::default())
        );
    }

    /// A record of what starts and ends; a pad on track 2 plays a chord.
    #[derive(Default)]
    struct Heard {
        on: Vec<(On, u8)>,
    }

    impl Sound for Heard {
        fn start(&mut self, track: usize, hits: bool, key: u8, _: u8, out: &mut Vec<(On, u8)>) {
            let keys: &[u8] = match (track, hits) {
                (2, true) => &[60, 64, 67],
                (9, _) => &[],
                _ => &[key],
            };
            for &k in keys {
                self.on.push((On::Instrument, k));
                out.push((On::Instrument, k));
            }
        }
        fn end(&mut self, on: On, key: u8) {
            let k = self.on.iter().position(|&h| h == (on, key)).unwrap();
            self.on.remove(k);
        }
    }

    #[test]
    fn heard_notes_let_go_in_time_and_on_the_next_ask() {
        let (mut h, mut s) = (Hearing::default(), Heard::default());
        let ask = |track, hits, notes: &[(u8, u8, f64)]| Ask {
            track,
            hits,
            notes: notes.to_vec(),
        };
        // A short note and a long one: the long one stops at a second.
        h.hear(
            &ask(0, false, &[(60, 100, 0.25), (64, 90, 3.0)]),
            10.0,
            &mut s,
        );
        assert_eq!(s.on.len(), 2);
        h.tick(10.2, &mut s);
        assert_eq!(s.on.len(), 2);
        h.tick(10.25, &mut s);
        assert_eq!(s.on, [(On::Instrument, 64)]);
        h.tick(11.0, &mut s);
        assert!(s.on.is_empty() && h.is_empty());
        // A pad of chords: each of its notes; the next ask lets them go.
        h.hear(&ask(2, true, &[(0, 100, 0.5)]), 20.0, &mut s);
        assert_eq!(s.on.len(), 3);
        h.hear(&ask(0, false, &[(50, 100, 0.5)]), 20.1, &mut s);
        assert_eq!(s.on, [(On::Instrument, 50)]);
        // Nothing asked, or nothing plays it: all quiet.
        h.hear(&Ask::default(), 20.2, &mut s);
        assert!(s.on.is_empty());
        h.hear(&ask(9, false, &[(50, 100, 0.5)]), 20.3, &mut s);
        assert!(s.on.is_empty() && h.is_empty());
        // More than the most at once: the rest end at once.
        let many: Vec<(u8, u8, f64)> = (0..40).map(|n| (n, 100, 0.5)).collect();
        h.hear(&ask(0, false, &many), 30.0, &mut s);
        assert_eq!(s.on.len(), MOST);
        h.stop(&mut s);
        assert!(s.on.is_empty());
    }
}
