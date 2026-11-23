//! Patches on MIDI in (the live shows plan's stage C1,
//! docs/dev/design/live-shows.md): what the keys play while the song has
//! patches. A song with none plays its keys exactly as before: nothing
//! here is asked.
//!
//! - [`PatchRig`] is made on the screen thread from the song's `patches`
//!   part ([`PatchRig::of`]) and goes to the audio loop whole, in the
//!   project's [`crate::project::Opening`] or on its own once edited.
//! - [`PatchIn`] lives in [`crate::midi_in::MusicIn`]. Each channel
//!   message, as it came in (before any route's blocks), goes to the
//!   patch playing after MIDI Learn has had it and before the routes and
//!   the armed track hear it ([`PatchIn::msg`]). What the patch sends its
//!   instruments waits for the sound loop ([`PatchIn::take`]), which plays
//!   it at once as a route's keys are played (`play_routed` in main.rs).
//!
//! A zone names what it plays as a route does: `track 4` is what track 4
//! plays (its kit, its own synth or the synth it plays), and `instrument 7`
//! a synth no track has, by the project's instrument number, played on a
//! route row. The routes' synths take the rows first and the patches'
//! the rows left, [`ROUTE_SYNTHS`] in all ([`crate::midi_in::route_synths_of`]);
//! a zone whose synth has no row plays nothing, and that is counted.
//!
//! The note owner table, the pedal rules and the switch are
//! `mpfree_patch::keys`'s. Nothing here allocates after [`PatchIn::new`].

use mpfree_midi::mpe::{self, MpeIn};
use mpfree_midi::msg::Msg;
use mpfree_midi::port::{PortId, PortInfo, MAX_PORTS};
use mpfree_midi::setup::ends_notes;
use mpfree_node::Ctl;
use mpfree_patch::keys::{From, In, Inst, Keys, Out, Went, When};
use mpfree_patch::{Patches, Rig};

use crate::midi_in::{Key, ROUTE_SYNTHS, SYNTH_ROUTES};

/// The tracks a zone may name, as a route may.
const TRACKS: u16 = mpfree_midi::setup::ROUTE_TRACKS as u16;

/// What the patch sends between two takes, at most; more is dropped and
/// counted ([`PatchIn::dropped`]).
pub const OUTS: usize = 1024;

/// The keys down the screen is told of at most (the patch editor lights
/// them); more are left out there, never here.
pub const SHOWN: usize = 16;

/// A key down as it came in, for the screen: its port and channel, its
/// note and velocity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shown {
    pub port: PortId,
    pub ch: u8,
    pub note: u8,
    pub vel: u8,
}

/// What a zone (or a patch's Keys outside the zones) plays, in its words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plays {
    /// What a track plays (from 0, below [`TRACKS`]).
    Track(u8),
    /// A synth no track has, by the project's instrument number.
    Synth(u32),
}

impl Plays {
    /// Reads `track 4` (tracks counted from 1, as the screen shows them)
    /// or `instrument 7`.
    pub fn parse(words: &str) -> Option<Plays> {
        let w = words.trim();
        if let Some(n) = w.strip_prefix("track ") {
            let n: u16 = n.trim().parse().ok()?;
            return (1..=TRACKS)
                .contains(&n)
                .then(|| Plays::Track((n - 1) as u8));
        }
        let n: u32 = w.strip_prefix("instrument ")?.trim().parse().ok()?;
        Some(Plays::Synth(n))
    }

    /// The words [`Plays::parse`] reads.
    pub fn text(self) -> String {
        match self {
            Plays::Track(t) => format!("track {}", u16::from(t) + 1),
            Plays::Synth(n) => format!("instrument {n}"),
        }
    }
}

/// The song's patches as the audio loop plays them.
#[derive(Debug, PartialEq)]
pub struct PatchRig {
    pub rig: Box<Rig>,
    /// The synths the zones name, each once, in the order first named:
    /// an instrument numbered [`TRACKS`] + `k` plays the `k`th. Past
    /// [`ROUTE_SYNTHS`] a zone plays nothing.
    pub synths: [Option<u32>; ROUTE_SYNTHS],
}

impl PatchRig {
    /// `p` for the audio loop. Allocates: the screen thread's job.
    pub fn of(p: &Patches) -> Box<PatchRig> {
        let mut synths = [None; ROUTE_SYNTHS];
        let rig = p.resolve(|words| match Plays::parse(words)? {
            Plays::Track(t) => Some(Inst(u16::from(t))),
            Plays::Synth(id) => {
                let k = match synths.iter().position(|s| *s == Some(id)) {
                    Some(k) => k,
                    None => {
                        let k = synths.iter().position(Option::is_none)?;
                        synths[k] = Some(id);
                        k
                    }
                };
                Some(Inst(TRACKS + k as u16))
            }
        });
        Box::new(PatchRig { rig, synths })
    }
}

/// The patch's side of MIDI in, on the audio thread.
pub struct PatchIn {
    keys: Keys,
    synths: [Option<u32>; ROUTE_SYNTHS],
    /// Per port: its wheel, pressure and MPE as they came, for the patch
    /// (the routes' own reading follows what their blocks made of them).
    mpe: Vec<MpeIn>,
    /// What the patch sends, each with the track it plays (or
    /// [`SYNTH_ROUTES`] + a route row), until the sound loop takes it.
    out: Vec<(u8, Key)>,
    /// Sends dropped: past [`OUTS`] between two takes, or to a synth no
    /// route row plays.
    pub dropped: u64,
    /// The keys down now while the patch takes part, as they came (the
    /// patch editor lights them and the zones they reach).
    shown: [Option<Shown>; SHOWN],
    /// Moves each time `shown` changes, so the loop tells the screen once.
    shown_at: u32,
}

impl Default for PatchIn {
    fn default() -> Self {
        Self::new()
    }
}

/// Where a whole message went, from where each of its parts did: the old
/// way if any part went there, else the patch if it played any.
fn join(a: Option<Went>, b: Went) -> Option<Went> {
    Some(match (a, b) {
        (Some(Went::Routed), _) | (_, Went::Routed) => Went::Routed,
        (Some(Went::Played), _) | (_, Went::Played) => Went::Played,
        _ => Went::Nowhere,
    })
}

/// A message's note or control for the patch (bends and pressure come
/// through MPE, as for the tracks).
fn key_in(msg: Msg) -> Option<In> {
    Some(match msg {
        Msg::NoteOn { note, vel, .. } => In::Down {
            note,
            velocity: vel,
        },
        Msg::NoteOff { note, .. } => In::Up { note },
        Msg::PolyPressure { note, value, .. } => In::Pressure { note, value },
        Msg::Cc { cc, value, .. } => In::Control {
            ctl: Ctl::from_cc(cc)?,
            value: f32::from(value) / 127.0,
        },
        _ => return None,
    })
}

fn expr_in(e: mpe::Expr) -> In {
    match e {
        mpe::Expr::Bend(value) => In::Bend { value },
        mpe::Expr::Slide { note, semis } => In::Expr {
            note,
            kind: mpfree_node::Expr::Slide,
            value: semis,
        },
        mpe::Expr::Tone { note, value } => In::Expr {
            note,
            kind: mpfree_node::Expr::Tone,
            value,
        },
        mpe::Expr::Pressure { note, value } => In::Pressure { note, value },
    }
}

/// One send, as a key on what plays it: a track, or the route row that
/// plays its synth now. Never allocates.
fn put(
    out: &mut Vec<(u8, Key)>,
    dropped: &mut u64,
    synths: &[Option<u32>; ROUTE_SYNTHS],
    rows: &[Option<u32>; ROUTE_SYNTHS],
    o: Out,
) {
    let (inst, key) = match o {
        Out::On {
            inst,
            note,
            velocity,
        } => (inst, Key::Down { note, velocity }),
        Out::Off { inst, note } => (inst, Key::Up { note }),
        Out::Pressure { inst, note, value } => (inst, Key::Pressure { note, value }),
        Out::Expr {
            inst,
            note,
            kind,
            value,
        } => (inst, Key::Expr { note, kind, value }),
        Out::Bend { inst, value } => (inst, Key::Bend { value }),
        Out::Control { inst, ctl, value } => (inst, Key::Control { ctl, value }),
    };
    let to = match inst.0.checked_sub(TRACKS) {
        None => inst.0 as u8,
        Some(k) => {
            let row = synths
                .get(usize::from(k))
                .copied()
                .flatten()
                .and_then(|id| rows.iter().position(|r| *r == Some(id)));
            match row {
                Some(j) => SYNTH_ROUTES + j as u8,
                None => {
                    *dropped += 1;
                    return;
                }
            }
        }
    };
    if out.len() < OUTS {
        out.push((to, key));
    } else {
        *dropped += 1;
    }
}

impl PatchIn {
    pub fn new() -> PatchIn {
        PatchIn {
            keys: Keys::new(),
            synths: [None; ROUTE_SYNTHS],
            mpe: (0..MAX_PORTS).map(|_| MpeIn::new()).collect(),
            out: Vec::with_capacity(OUTS),
            dropped: 0,
            shown: [None; SHOWN],
            shown_at: 0,
        }
    }

    /// A key down or up, for the screen's lit keys. Never allocates.
    fn show(&mut self, port: PortId, msg: Msg) {
        let (ch, note, vel) = match msg {
            Msg::NoteOn { ch, note, vel } => (ch, note, vel),
            Msg::NoteOff { ch, note, .. } => (ch, note, 0),
            _ => return,
        };
        let same =
            |s: &Option<Shown>| s.is_some_and(|s| s.port == port && s.ch == ch && s.note == note);
        let at = self.shown.iter().position(same);
        if vel == 0 {
            if let Some(i) = at {
                self.shown[i] = None;
                self.shown_at = self.shown_at.wrapping_add(1);
            }
            return;
        }
        let Some(i) = at.or_else(|| self.shown.iter().position(Option::is_none)) else {
            return;
        };
        self.shown[i] = Some(Shown {
            port,
            ch,
            note,
            vel,
        });
        self.shown_at = self.shown_at.wrapping_add(1);
    }

    /// The keys down now, for the screen, and a count that moves each
    /// time they change ([`PatchIn::shown_at`]).
    pub fn shown(&self) -> impl Iterator<Item = Shown> + '_ {
        self.shown.iter().flatten().copied()
    }

    /// Moves each time the keys down change.
    pub fn shown_at(&self) -> u32 {
        self.shown_at
    }

    fn forget_shown(&mut self, port: Option<PortId>) {
        let mut changed = false;
        for s in &mut self.shown {
            if s.is_some_and(|k| port.is_none_or(|p| k.port == p)) {
                *s = None;
                changed = true;
            }
        }
        if changed {
            self.shown_at = self.shown_at.wrapping_add(1);
        }
    }

    /// The synths the patches name, for the route rows
    /// ([`crate::midi_in::route_synths_of`]).
    pub fn synths(&self) -> &[Option<u32>; ROUTE_SYNTHS] {
        &self.synths
    }

    /// The song's patches change (a project opened, an edit): `rig` comes
    /// in and the old one takes its place in the box, to be freed on the
    /// screen thread. Keys held keep ending where they started. `ports`:
    /// the ports plugged in now, so zones that name a keyboard find it.
    pub fn swap(&mut self, rig: &mut PatchRig, ports: &[PortInfo]) {
        self.keys.swap_rig(&mut rig.rig);
        std::mem::swap(&mut self.synths, &mut rig.synths);
        for p in ports.iter().filter(|p| p.connected && p.has_in) {
            self.keys.set_port(p.id.0, Some(&p.name));
        }
    }

    /// A port was plugged in as `name`.
    pub fn plugged(&mut self, port: PortId, name: &str) {
        self.keys.set_port(port.0, Some(name));
    }

    /// A port went away: what its keys and pedals hold here ends.
    pub fn unplugged(&mut self, port: PortId, rows: &[Option<u32>; ROUTE_SYNTHS]) {
        let (out, dropped, synths) = (&mut self.out, &mut self.dropped, &self.synths);
        self.keys
            .unplug(port.0, &mut |o| put(out, dropped, synths, rows, o));
        self.keys.set_port(port.0, None);
        if let Some(m) = self.mpe.get_mut(usize::from(port.0)) {
            m.reset();
        }
        self.forget_shown(Some(port));
    }

    /// The port's MPE zone, as its profile says ([`MpeIn::set_zone`]).
    pub fn set_zone(&mut self, port: PortId, zone: mpe::Zone) {
        if let Some(m) = self.mpe.get_mut(usize::from(port.0)) {
            m.set_zone(zone);
        }
    }

    /// One channel message from `port` as it came, after MIDI Learn had
    /// it and only if the port's input plays. None: the patch has nothing
    /// to do with it (the song has no patches, or it is no note, pedal,
    /// wheel or pressure), so it goes on as before. Else where it went:
    /// [`Went::Routed`] goes on as before too (the routes and the armed
    /// track hear it); [`Went::Played`] and [`Went::Nowhere`] stop here.
    pub fn msg(
        &mut self,
        port: PortId,
        msg: Msg,
        rows: &[Option<u32>; ROUTE_SYNTHS],
    ) -> Option<Went> {
        let active = self.keys.active();
        if active && usize::from(port.0) < self.mpe.len() {
            self.show(port, msg);
        }
        let mpe = self.mpe.get_mut(usize::from(port.0))?;
        if !active {
            // Only read, so it knows which channel holds which note when
            // a patch wants it.
            mpe.msg(msg, &mut |_| {});
            return None;
        }
        let from = From {
            port: port.0,
            ch: msg.channel(),
        };
        let (keys, out, dropped, synths) = (
            &mut self.keys,
            &mut self.out,
            &mut self.dropped,
            &self.synths,
        );
        let mut send = |o: Out| put(out, dropped, synths, rows, o);
        // All notes off on a channel ends its keys here, and the old way's
        // there.
        if let Msg::Cc { cc, .. } = msg {
            if ends_notes(cc) {
                keys.release(from, &mut send);
                mpe.msg(msg, &mut |_| {});
                return Some(Went::Routed);
            }
        }
        let note = key_in(msg);
        let mut went = note.map(|k| keys.key(from, k, &mut send));
        let is_note = matches!(note, Some(In::Down { .. } | In::Up { .. }));
        // Its expression: a bend, pressure, an MPE note's own slide and
        // tone (after its note, which they may come with).
        mpe.msg(msg, &mut |e| {
            let w = keys.key(from, expr_in(e), &mut send);
            // What comes with a note goes where the note did.
            if !is_note {
                went = join(went, w);
            }
        });
        went
    }

    /// Picks patch `to` (from 0; None: keys as routed), at once or when
    /// every key and pedal is up. False: the song hasn't got it, and
    /// nothing changed.
    pub fn pick(
        &mut self,
        to: Option<usize>,
        when: When,
        rows: &[Option<u32>; ROUTE_SYNTHS],
    ) -> bool {
        if to.is_some_and(|i| !self.keys.has(i)) {
            return false;
        }
        let (out, dropped, synths) = (&mut self.out, &mut self.dropped, &self.synths);
        self.keys
            .pick(to, when, &mut |o| put(out, dropped, synths, rows, o));
        true
    }

    /// Next patch (`d` 1) or Previous patch (-1): [`Keys::step`]. The
    /// patch picked, or None when there was nowhere to go.
    pub fn step(
        &mut self,
        d: i32,
        when: When,
        rows: &[Option<u32>; ROUTE_SYNTHS],
    ) -> Option<usize> {
        let (out, dropped, synths) = (&mut self.out, &mut self.dropped, &self.synths);
        self.keys
            .step(d, when, &mut |o| put(out, dropped, synths, rows, o))
    }

    /// Every sound stopped (a project opening, STOP's stop of every
    /// sound): the table forgets what it held, sending nothing.
    pub fn stopped(&mut self) {
        self.keys.panic(&mut |_| {});
        self.forget_shown(None);
    }

    /// Whether the patch takes part: the song has patches, or a key or
    /// pedal is still held from one ([`Keys::active`]).
    pub fn active(&self) -> bool {
        self.keys.active()
    }

    /// The patch playing (from 0), or None: keys as routed.
    pub fn playing(&self) -> Option<usize> {
        self.keys.playing()
    }

    /// A switch waiting for the hands: the patch it goes to.
    pub fn waiting(&self) -> Option<Option<usize>> {
        self.keys.waiting()
    }

    /// How many patches the song has.
    pub fn patches(&self) -> usize {
        self.keys.patches()
    }

    /// What the engine counted (a full table, pedals past their room).
    pub fn stats(&self) -> mpfree_patch::keys::Stats {
        self.keys.stats
    }

    /// What the patch sent since the last take, each with what plays it:
    /// a track (from 0), or [`SYNTH_ROUTES`] + the route row of a synth.
    pub fn take(&mut self) -> std::vec::Drain<'_, (u8, Key)> {
        self.out.drain(..)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_patch::{Layout, Outside};

    fn rig(list: Vec<mpfree_patch::Patch>) -> Box<PatchRig> {
        PatchRig::of(&Patches { list })
    }

    const ROWS: [Option<u32>; ROUTE_SYNTHS] = [Some(9), Some(7), None, None];
    const P1: PortId = PortId(1);

    fn on(ch: u8, note: u8, vel: u8) -> Msg {
        Msg::NoteOn { ch, note, vel }
    }

    fn off(ch: u8, note: u8) -> Msg {
        Msg::NoteOff { ch, note, vel: 64 }
    }

    #[test]
    fn a_zone_names_what_it_plays_as_a_route_does() {
        assert_eq!(Plays::parse("track 4"), Some(Plays::Track(3)));
        assert_eq!(Plays::parse("instrument 7"), Some(Plays::Synth(7)));
        assert_eq!(Plays::parse("track 0"), None);
        assert_eq!(Plays::parse("track 129"), None);
        assert_eq!(Plays::parse("kit 2"), None);
        for p in [Plays::Track(127), Plays::Synth(3)] {
            assert_eq!(Plays::parse(&p.text()), Some(p));
        }
    }

    #[test]
    fn with_no_patches_nothing_is_asked() {
        let mut p = PatchIn::new();
        assert_eq!(p.msg(P1, on(0, 60, 100), &ROWS), None);
        assert_eq!(p.msg(P1, off(0, 60), &ROWS), None);
        assert_eq!(p.take().count(), 0);
    }

    #[test]
    fn a_split_plays_a_track_and_a_synth_on_its_route_row() {
        let mut split = Layout::BassAndKeys.patch("Split", "track 2");
        split.zones[1].plays = "instrument 7".into();
        let mut r = rig(vec![split]);
        let mut p = PatchIn::new();
        p.swap(&mut r, &[]);
        assert!(r.rig.is_empty(), "the old rig came back");
        assert!(p.pick(Some(0), When::Now, &ROWS));
        assert!(!p.pick(Some(3), When::Now, &ROWS), "no patch 4");
        assert_eq!(p.msg(P1, on(0, 36, 90), &ROWS), Some(Went::Played));
        assert_eq!(p.msg(P1, on(0, 60, 80), &ROWS), Some(Went::Played));
        assert_eq!(p.msg(P1, off(0, 60), &ROWS), Some(Went::Played));
        // A program change is the old way's (Learn's, Sync's).
        assert_eq!(p.msg(P1, Msg::Program { ch: 0, program: 2 }, &ROWS), None);
        let sent: Vec<_> = p.take().collect();
        // Instrument 7 is on route row 1.
        assert_eq!(
            sent,
            vec![
                (
                    1,
                    Key::Down {
                        note: 36,
                        velocity: 90
                    }
                ),
                (
                    SYNTH_ROUTES + 1,
                    Key::Down {
                        note: 60,
                        velocity: 80
                    }
                ),
                (SYNTH_ROUTES + 1, Key::Up { note: 60 }),
            ]
        );
        // Its row gone: its zone plays nothing, counted.
        p.msg(P1, on(0, 62, 80), &[None; ROUTE_SYNTHS]);
        assert_eq!(p.take().count(), 0);
        assert_eq!(p.dropped, 1);
    }

    #[test]
    fn outside_the_zones_as_routed_goes_on_as_before() {
        let mut gap = Layout::Whole.patch("High", "track 1");
        gap.zones[0].low = 60;
        gap.outside = Outside::Routed;
        let mut p = PatchIn::new();
        p.swap(&mut rig(vec![gap]), &[]);
        p.pick(Some(0), When::Now, &ROWS);
        assert_eq!(p.msg(P1, on(0, 40, 90), &ROWS), Some(Went::Routed));
        assert_eq!(p.msg(P1, off(0, 40), &ROWS), Some(Went::Routed));
        assert_eq!(p.take().count(), 0);
    }

    #[test]
    fn a_bend_reaches_the_patch_and_an_mpe_slide_its_note() {
        let mut p = PatchIn::new();
        p.swap(&mut rig(vec![Layout::Whole.patch("Lead", "track 3")]), &[]);
        p.set_zone(P1, mpe::Zone::Lower(15));
        p.pick(Some(0), When::Now, &ROWS);
        p.msg(P1, on(2, 60, 100), &ROWS);
        p.take().for_each(drop);
        // The member channel's bend slides only its note.
        assert_eq!(
            p.msg(P1, Msg::PitchBend { ch: 2, value: 4096 }, &ROWS),
            Some(Went::Played)
        );
        let sent: Vec<_> = p.take().collect();
        assert!(
            matches!(
                sent[..],
                [(
                    2,
                    Key::Expr {
                        note: 60,
                        kind: mpfree_node::Expr::Slide,
                        ..
                    }
                )]
            ),
            "{sent:?}"
        );
        // The master's bends every note.
        assert_eq!(
            p.msg(
                P1,
                Msg::PitchBend {
                    ch: 0,
                    value: -8192
                },
                &ROWS
            ),
            Some(Went::Played)
        );
        assert_eq!(
            p.take().collect::<Vec<_>>(),
            vec![(2, Key::Bend { value: -1.0 })]
        );
        // All notes off on the member's channel ends it, and the old way
        // hears it too.
        assert_eq!(
            p.msg(
                P1,
                Msg::Cc {
                    ch: 2,
                    cc: 123,
                    value: 0
                },
                &ROWS
            ),
            Some(Went::Routed)
        );
        assert_eq!(
            p.take().collect::<Vec<_>>(),
            vec![(2, Key::Up { note: 60 })]
        );
    }

    /// The screen is told of each key down while the song has patches,
    /// even one no zone takes, and of its let-go; a port unplugged and a
    /// stop forget theirs.
    #[test]
    fn the_keys_down_are_shown_for_the_patch_editor() {
        let mut p = PatchIn::new();
        p.msg(P1, on(0, 60, 100), &ROWS);
        assert_eq!(p.shown().count(), 0, "no patches: nothing shown");
        let mut high = Layout::Whole.patch("High", "track 1");
        high.zones[0].low = 60;
        p.swap(&mut rig(vec![high]), &[]);
        p.pick(Some(0), When::Now, &ROWS);
        let at = p.shown_at();
        p.msg(P1, on(0, 40, 52), &ROWS);
        p.msg(PortId(2), on(3, 64, 96), &ROWS);
        assert_ne!(p.shown_at(), at);
        let shown: Vec<_> = p.shown().map(|s| (s.port, s.ch, s.note, s.vel)).collect();
        assert_eq!(shown, vec![(P1, 0, 40, 52), (PortId(2), 3, 64, 96)]);
        p.msg(P1, on(0, 40, 0), &ROWS);
        assert_eq!(p.shown().count(), 1, "a note on at velocity 0 lets go");
        p.unplugged(PortId(2), &ROWS);
        assert_eq!(p.shown().count(), 0);
        p.msg(P1, on(0, 61, 70), &ROWS);
        p.stopped();
        assert_eq!(p.shown().count(), 0);
        p.take().for_each(drop);
    }

    #[test]
    fn a_keyboard_unplugged_lets_go_and_a_stop_forgets() {
        let mut p = PatchIn::new();
        p.swap(&mut rig(vec![Layout::Whole.patch("Keys", "track 1")]), &[]);
        p.pick(Some(0), When::Now, &ROWS);
        p.msg(P1, on(0, 60, 100), &ROWS);
        p.msg(
            P1,
            Msg::Cc {
                ch: 0,
                cc: 64,
                value: 127,
            },
            &ROWS,
        );
        p.take().for_each(drop);
        p.unplugged(P1, &ROWS);
        let sent: Vec<_> = p.take().collect();
        assert_eq!(
            sent,
            vec![
                (0, Key::Up { note: 60 }),
                (
                    0,
                    Key::Control {
                        ctl: Ctl::Sustain,
                        value: 0.0
                    }
                )
            ]
        );
        p.msg(PortId(2), on(0, 61, 100), &ROWS);
        p.take().for_each(drop);
        p.stopped();
        // Its let-go finds nothing now: the old way's, harmlessly.
        assert_eq!(p.msg(PortId(2), off(0, 61), &ROWS), Some(Went::Routed));
        assert_eq!(p.take().count(), 0);
    }
}
