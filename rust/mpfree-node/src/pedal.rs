//! The sustain and sostenuto pedals: which notes sound on after their key
//! is let go ([`Pedals`]).
//!
//! An instrument that handles the pedals itself says so with
//! [`crate::Node::hears`] and keeps a [`Pedals`] of its own. For one that
//! doesn't, the host runs the same bookkeeping in front of it
//! ([`Pedals::filter`]): it holds back a note-off while a pedal keeps the
//! note, and sends it when the pedal comes up. So every instrument
//! sustains, and the pedal sounds the same whichever side holds the notes.
//!
//! - **Sustain** (CC 64): a note let go while the pedal is down sounds on
//!   until the pedal comes up.
//! - **Sostenuto** (CC 66): the notes whose keys are down when the pedal
//!   goes down sound on until it comes up, however their keys move; notes
//!   played after it are not held by it.
//! - **A key struck again** while a pedal still holds its note restarts
//!   the note: the host ends the old one on the same frame and starts the
//!   new one ([`Pedals::filter`]), as a piano's hammer strikes a string
//!   that is still ringing.
//!
//! Plain data: no allocation, so it runs on the audio thread.

use crate::{Ctl, NodeEvent};

/// One flag for each MIDI note.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoteSet([u64; 2]);

impl NoteSet {
    pub const fn new() -> NoteSet {
        NoteSet([0; 2])
    }
    #[inline]
    pub fn has(&self, note: u8) -> bool {
        let n = note & 0x7F;
        self.0[usize::from(n >> 6)] & (1 << (n & 63)) != 0
    }
    #[inline]
    pub fn put(&mut self, note: u8) {
        let n = note & 0x7F;
        self.0[usize::from(n >> 6)] |= 1 << (n & 63);
    }
    #[inline]
    pub fn take(&mut self, note: u8) {
        let n = note & 0x7F;
        self.0[usize::from(n >> 6)] &= !(1 << (n & 63));
    }
    pub fn is_empty(&self) -> bool {
        self.0 == [0; 2]
    }
    pub fn clear(&mut self) {
        self.0 = [0; 2];
    }
    /// How many notes are set.
    pub fn len(&self) -> usize {
        (self.0[0].count_ones() + self.0[1].count_ones()) as usize
    }
    /// Calls `f` with each note set, lowest first.
    pub fn each(&self, mut f: impl FnMut(u8)) {
        for (w, word) in self.0.iter().enumerate() {
            let mut bits = *word;
            while bits != 0 {
                let b = bits.trailing_zeros();
                bits &= bits - 1;
                f((w as u8) * 64 + b as u8);
            }
        }
    }
}

/// The pedals and the keys they hold. See the module docs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pedals {
    /// Keys down: a note-on with no note-off yet.
    down: NoteSet,
    /// Keys let go while the sustain pedal was down: still sounding.
    held: NoteSet,
    /// Notes the sostenuto pedal caught when it went down.
    caught: NoteSet,
    sustain: bool,
    sostenuto: bool,
}

impl Pedals {
    pub const fn new() -> Pedals {
        Pedals {
            down: NoteSet::new(),
            held: NoteSet::new(),
            caught: NoteSet::new(),
            sustain: false,
            sostenuto: false,
        }
    }

    /// Whether the sustain pedal is down.
    pub fn sustain(&self) -> bool {
        self.sustain
    }

    /// Whether the sostenuto pedal is down.
    pub fn sostenuto(&self) -> bool {
        self.sostenuto
    }

    /// Whether `note` sounds: its key is down or a pedal holds it.
    pub fn sounding(&self, note: u8) -> bool {
        self.down.has(note) || self.held.has(note) || self.caught.has(note)
    }

    /// Whether a pedal holds `note` with its key up.
    pub fn holds(&self, note: u8) -> bool {
        !self.down.has(note) && (self.held.has(note) || self.caught.has(note))
    }

    /// A key goes down. True when a pedal still held its note: the note is
    /// struck again while it sounds.
    pub fn press(&mut self, note: u8) -> bool {
        let again = self.holds(note);
        self.down.put(note);
        self.held.take(note);
        again
    }

    /// The quick path for the usual case, no pedal down and no note held
    /// by one: notes the key (for a sostenuto later) and returns true when
    /// `ev` passes as it came. False when it needs [`Pedals::filter`]: a
    /// pedal moving, or anything while a pedal is down or holds notes.
    #[inline]
    pub fn pass(&mut self, ev: &NodeEvent) -> bool {
        if self.sustain || self.sostenuto || !self.held.is_empty() || !self.caught.is_empty() {
            return false;
        }
        match *ev {
            NodeEvent::NoteOn { note, velocity } if velocity > 0 => self.down.put(note),
            NodeEvent::NoteOn { note, .. } | NodeEvent::NoteOff { note } => {
                self.down.take(note);
            }
            NodeEvent::Control {
                ctl: Ctl::Sustain | Ctl::Sostenuto,
                ..
            } => return false,
            NodeEvent::AllOff => self.down.clear(),
            _ => {}
        }
        true
    }

    /// A key comes up. True when its note ends now; false when a pedal
    /// holds it.
    pub fn lift(&mut self, note: u8) -> bool {
        self.down.take(note);
        if self.caught.has(note) {
            return false;
        }
        if self.sustain {
            self.held.put(note);
            return false;
        }
        true
    }

    /// A pedal moves to `value` (down from 0.5). `end` is called with each
    /// note that ends because it came up, lowest first. Other controllers
    /// change nothing here.
    pub fn set(&mut self, ctl: Ctl, value: f32, mut end: impl FnMut(u8)) {
        let Some(value) = Ctl::clamp(value) else {
            return;
        };
        let on = Ctl::down(value);
        match ctl {
            Ctl::Sustain if on != self.sustain => {
                self.sustain = on;
                if !on {
                    let held = self.held;
                    self.held.clear();
                    held.each(|n| {
                        if !self.down.has(n) && !self.caught.has(n) {
                            end(n);
                        }
                    });
                }
            }
            Ctl::Sostenuto if on != self.sostenuto => {
                self.sostenuto = on;
                if on {
                    self.caught = self.down;
                } else {
                    let caught = self.caught;
                    self.caught.clear();
                    caught.each(|n| {
                        if self.down.has(n) {
                            return;
                        }
                        if self.sustain {
                            self.held.put(n);
                        } else if !self.held.has(n) {
                            end(n);
                        }
                    });
                }
            }
            _ => {}
        }
    }

    /// Every note ends (an AllOff, a reset): nothing is down or held. The
    /// pedals stay where they are, as the player's feet do.
    pub fn clear_notes(&mut self) {
        self.down.clear();
        self.held.clear();
        self.caught.clear();
    }

    /// Everything back to the start: no notes, both pedals up.
    pub fn reset(&mut self) {
        *self = Pedals::new();
    }

    /// The host's side, for an instrument that doesn't hear the pedals:
    /// passes `ev` on through `out` with the pedals applied. A note-off a
    /// pedal holds is kept back and sent (on the pedal's frame) when it
    /// comes up; a key struck again under a pedal sends its note-off first,
    /// then the note-on; the pedal events themselves are not passed on.
    /// Anything else passes as it came.
    pub fn filter(&mut self, ev: NodeEvent, mut out: impl FnMut(NodeEvent)) {
        match ev {
            NodeEvent::NoteOn { note, velocity } if velocity > 0 => {
                if self.press(note) {
                    out(NodeEvent::NoteOff { note });
                }
                out(ev);
            }
            NodeEvent::NoteOn { note, .. } | NodeEvent::NoteOff { note } => {
                if self.lift(note) {
                    out(NodeEvent::NoteOff { note });
                }
            }
            NodeEvent::Control {
                ctl: ctl @ (Ctl::Sustain | Ctl::Sostenuto),
                value,
            } => self.set(ctl, value, |note| out(NodeEvent::NoteOff { note })),
            NodeEvent::AllOff => {
                self.clear_notes();
                out(ev);
            }
            _ => out(ev),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(p: &mut Pedals, evs: &[NodeEvent]) -> Vec<NodeEvent> {
        let mut out = Vec::new();
        for &e in evs {
            p.filter(e, |x| out.push(x));
        }
        out
    }

    #[test]
    fn the_quick_path_gives_what_the_filter_gives() {
        let sus = |v: f32| NodeEvent::Control {
            ctl: Ctl::Sustain,
            value: v,
        };
        let sos = |v: f32| NodeEvent::Control {
            ctl: Ctl::Sostenuto,
            value: v,
        };
        let script = [
            on(60),
            on(64),
            off(60),
            sos(1.0),
            off(64),
            on(67),
            sus(1.0),
            off(67),
            on(67),
            sus(0.0),
            sos(0.0),
            on(72),
            NodeEvent::AllOff,
            on(48),
            off(48),
            off(48),
            NodeEvent::Bend { value: 0.5 },
        ];
        let (mut a, mut b) = (Pedals::new(), Pedals::new());
        let (mut out_a, mut out_b) = (Vec::new(), Vec::new());
        for &e in &script {
            a.filter(e, |x| out_a.push(x));
            if b.pass(&e) {
                out_b.push(e);
            } else {
                b.filter(e, |x| out_b.push(x));
            }
            assert_eq!(out_a, out_b, "after {e:?}");
            assert_eq!(a, b, "after {e:?}");
        }
    }

    fn on(note: u8) -> NodeEvent {
        NodeEvent::NoteOn {
            note,
            velocity: 100,
        }
    }
    fn off(note: u8) -> NodeEvent {
        NodeEvent::NoteOff { note }
    }
    fn pedal(ctl: Ctl, down: bool) -> NodeEvent {
        NodeEvent::Control {
            ctl,
            value: if down { 1.0 } else { 0.0 },
        }
    }

    #[test]
    fn note_sets_hold_every_note() {
        let mut s = NoteSet::new();
        for n in [0u8, 1, 63, 64, 100, 127] {
            s.put(n);
        }
        assert_eq!(s.len(), 6);
        let mut seen = Vec::new();
        s.each(|n| seen.push(n));
        assert_eq!(seen, vec![0, 1, 63, 64, 100, 127]);
        s.take(64);
        assert!(!s.has(64) && s.has(63) && s.has(127));
    }

    #[test]
    fn with_no_pedal_notes_pass_as_they_came() {
        let mut p = Pedals::new();
        let evs = [on(60), on(64), off(60), off(64)];
        assert_eq!(run(&mut p, &evs), evs.to_vec());
    }

    #[test]
    fn sustain_holds_a_note_let_go_until_it_comes_up() {
        let mut p = Pedals::new();
        let out = run(
            &mut p,
            &[on(60), pedal(Ctl::Sustain, true), off(60), on(64), off(64)],
        );
        assert_eq!(out, vec![on(60), on(64)]);
        assert!(p.holds(60) && p.holds(64));
        let out = run(&mut p, &[pedal(Ctl::Sustain, false)]);
        assert_eq!(out, vec![off(60), off(64)]);
        assert!(!p.sounding(60) && !p.sounding(64));
    }

    #[test]
    fn a_key_still_down_when_the_pedal_comes_up_keeps_sounding() {
        let mut p = Pedals::new();
        let out = run(
            &mut p,
            &[
                pedal(Ctl::Sustain, true),
                on(60),
                pedal(Ctl::Sustain, false),
            ],
        );
        assert_eq!(out, vec![on(60)]);
        assert_eq!(run(&mut p, &[off(60)]), vec![off(60)]);
    }

    #[test]
    fn a_key_struck_again_under_the_pedal_restarts_its_note() {
        let mut p = Pedals::new();
        let out = run(
            &mut p,
            &[pedal(Ctl::Sustain, true), on(60), off(60), on(60)],
        );
        assert_eq!(out, vec![on(60), off(60), on(60)]);
        // One note sounding, so the pedal ends one.
        let out = run(&mut p, &[off(60), pedal(Ctl::Sustain, false)]);
        assert_eq!(out, vec![off(60)]);
    }

    #[test]
    fn sostenuto_holds_only_the_notes_down_when_it_went_down() {
        let mut p = Pedals::new();
        let out = run(
            &mut p,
            &[
                on(48),
                pedal(Ctl::Sostenuto, true),
                off(48),
                on(64),
                off(64),
            ],
        );
        // 48 is caught; 64 came after the pedal, so it ends.
        assert_eq!(out, vec![on(48), on(64), off(64)]);
        assert!(p.holds(48));
        assert_eq!(run(&mut p, &[pedal(Ctl::Sostenuto, false)]), vec![off(48)]);
    }

    #[test]
    fn sostenuto_up_under_sustain_hands_its_notes_to_sustain() {
        let mut p = Pedals::new();
        run(
            &mut p,
            &[
                on(48),
                pedal(Ctl::Sostenuto, true),
                off(48),
                pedal(Ctl::Sustain, true),
            ],
        );
        assert!(run(&mut p, &[pedal(Ctl::Sostenuto, false)]).is_empty());
        assert!(p.holds(48));
        assert_eq!(run(&mut p, &[pedal(Ctl::Sustain, false)]), vec![off(48)]);
    }

    #[test]
    fn sustain_up_leaves_notes_sostenuto_caught() {
        let mut p = Pedals::new();
        run(
            &mut p,
            &[
                pedal(Ctl::Sustain, true),
                on(48),
                pedal(Ctl::Sostenuto, true),
                off(48),
                on(60),
                off(60),
            ],
        );
        // Sustain up ends 60 (only sustain held it), not 48 (caught).
        assert_eq!(run(&mut p, &[pedal(Ctl::Sustain, false)]), vec![off(60)]);
        assert_eq!(run(&mut p, &[pedal(Ctl::Sostenuto, false)]), vec![off(48)]);
    }

    #[test]
    fn all_off_forgets_the_notes_but_not_the_pedal() {
        let mut p = Pedals::new();
        run(&mut p, &[pedal(Ctl::Sustain, true), on(60), off(60)]);
        assert_eq!(run(&mut p, &[NodeEvent::AllOff]), vec![NodeEvent::AllOff]);
        assert!(!p.sounding(60) && p.sustain());
        assert!(run(&mut p, &[pedal(Ctl::Sustain, false)]).is_empty());
    }

    #[test]
    fn a_pedal_at_the_same_place_or_not_a_number_changes_nothing() {
        let mut p = Pedals::new();
        run(&mut p, &[pedal(Ctl::Sustain, true), on(60), off(60)]);
        let nan = NodeEvent::Control {
            ctl: Ctl::Sustain,
            value: f32::NAN,
        };
        assert!(run(&mut p, &[pedal(Ctl::Sustain, true), nan]).is_empty());
        assert!(p.holds(60));
        // 0.49 is up, as CC 63 is.
        let up = NodeEvent::Control {
            ctl: Ctl::Sustain,
            value: 0.49,
        };
        assert_eq!(run(&mut p, &[up]), vec![off(60)]);
    }

    #[test]
    fn other_controllers_pass_through() {
        let mut p = Pedals::new();
        let wheel = NodeEvent::Control {
            ctl: Ctl::ModWheel,
            value: 0.3,
        };
        assert_eq!(run(&mut p, &[wheel]), vec![wheel]);
    }

    #[test]
    fn controllers_map_to_their_midi_numbers_and_back() {
        for c in Ctl::ALL {
            assert_eq!(Ctl::from_cc(c.cc()), Some(c));
        }
        assert_eq!(Ctl::from_cc(7), None);
        assert!(Ctl::Sustain.is_pedal() && !Ctl::Breath.is_pedal());
    }
}
