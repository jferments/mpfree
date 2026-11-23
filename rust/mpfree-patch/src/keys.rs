//! What the keys play, on the audio thread: the patch playing, and the
//! table that remembers where every held key and pedal went.
//!
//! - **A note ends where it started.** Each key held is remembered with
//!   the keyboard and channel it came from and every instrument and note
//!   it went to (up to one per zone). Its let-go, pressure and MPE
//!   expression look it up there, never at the patch playing now.
//! - **A key struck again** before it was let go ends its earlier notes
//!   first, then plays again, so one note-off always ends one note.
//! - **Pedals belong to keyboards.** Each keyboard's sustain, sostenuto
//!   and soft pedal remember the instruments they went down on. A pedal
//!   held across a switch stays down there until it comes up, and also
//!   goes to the new patch's zones that take pedals (sustain and soft,
//!   not sostenuto, which holds only the notes it caught). An instrument
//!   two keyboards hold a pedal down on hears it come up only when both
//!   have let go.
//! - **Wheels follow the keyboard.** The pitch wheel, mod wheel, breath
//!   and foot reach the patch's zones that take wheels and the
//!   instruments this keyboard's held notes went to; a new patch starts
//!   its instruments from where the wheels are now.
//! - **A switch** picked to wait for the hands happens when every key and
//!   pedal is up. A patch with Cut the old sound ends what the patch
//!   before still holds as it comes in.
//! - **Keys as routed:** with no patch playing (or for a patch's keys
//!   outside its zones set so) a key goes the old way: [`Went::Routed`]
//!   tells the caller, and its let-go is sent the same way even after a
//!   switch.
//!
//! The table has room for [`MAX_HELD`] keys; one more plays nothing and
//! is counted ([`Stats::held_full`]), so no note can hang. Nothing here
//! allocates after [`Keys::new`].

use crate::{LiveOutside, LivePatch, Rig, MAX_ZONES};
use mpfree_node::{Ctl, Expr};

/// An instrument, as the app numbers the ones it holds. The app keeps a
/// number for as long as the instrument is in, and calls [`Keys::forget`]
/// when it goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Inst(pub u16);

/// Where a message came from: its port and channel (0 to 15).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct From {
    pub port: u8,
    pub ch: u8,
}

/// The most keys held at once, every keyboard together.
pub const MAX_HELD: usize = 128;

/// The most keyboards (port and channel) whose pedals and wheels are
/// followed at once.
pub const MAX_KEYBOARDS: usize = 32;

/// The most instruments one pedal or wheel move reaches.
pub const MAX_REACH: usize = 16;

/// A message from a keyboard, after MIDI Learn took its controls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum In {
    /// A note-on; velocity 0 is a let-go.
    Down {
        note: u8,
        velocity: u8,
    },
    Up {
        note: u8,
    },
    /// Aftertouch on a held key.
    Pressure {
        note: u8,
        value: u8,
    },
    /// The pitch wheel, -1 to 1.
    Bend {
        value: f32,
    },
    /// An MPE note's own slide or tone.
    Expr {
        note: u8,
        kind: Expr,
        value: f32,
    },
    /// A pedal, the mod wheel, breath or foot, 0 to 1.
    Control {
        ctl: Ctl,
        value: f32,
    },
}

/// What an instrument is sent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Out {
    On {
        inst: Inst,
        note: u8,
        velocity: u8,
    },
    Off {
        inst: Inst,
        note: u8,
    },
    Pressure {
        inst: Inst,
        note: u8,
        value: u8,
    },
    Expr {
        inst: Inst,
        note: u8,
        kind: Expr,
        value: f32,
    },
    Bend {
        inst: Inst,
        value: f32,
    },
    Control {
        inst: Inst,
        ctl: Ctl,
        value: f32,
    },
}

/// Where a message went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Went {
    /// To the patch's instruments (the [`Out`]s given).
    Played,
    /// Nowhere: a key outside every zone, one whose zone's instrument
    /// isn't in the song, or one the table had no room for.
    Nowhere,
    /// The old way too (the armed track and the routes): with no patch
    /// playing, a key the patch leaves to them, or a wheel or pedal they
    /// also hear.
    Routed,
}

/// When a switch picked by hand happens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    Now,
    /// When every key and pedal is up.
    HandsUp,
}

/// What [`Keys`] counted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Keys that played nothing because [`MAX_HELD`] were held.
    pub held_full: u64,
    /// Instruments a pedal or wheel move couldn't reach past
    /// [`MAX_REACH`].
    pub reach_full: u64,
    /// Pedal and wheel moves not followed: [`MAX_KEYBOARDS`] keyboards
    /// each had a pedal down.
    pub keyboards_full: u64,
    /// Patch switches made.
    pub switches: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Dest {
    inst: Inst,
    note: u8,
    pedals: bool,
    wheels: bool,
}

const NO_DEST: Dest = Dest {
    inst: Inst(0),
    note: 0,
    pedals: false,
    wheels: false,
};

#[derive(Clone, Copy, Debug)]
struct Held {
    from: From,
    note: u8,
    /// It went the old way.
    routed: bool,
    n: u8,
    to: [Dest; MAX_ZONES],
}

impl Held {
    fn dests(&self) -> &[Dest] {
        &self.to[..usize::from(self.n)]
    }
}

/// The pedals in a keyboard's order: sustain, sostenuto, soft.
const PEDALS: [Ctl; 3] = [Ctl::Sustain, Ctl::Sostenuto, Ctl::Soft];

/// The wheels besides the pitch wheel.
const WHEELS: [Ctl; 3] = [Ctl::ModWheel, Ctl::Breath, Ctl::Foot];

/// A short set of instruments, in the order first added.
#[derive(Clone, Copy, Debug)]
struct Reach {
    n: u8,
    to: [Inst; MAX_REACH],
}

impl Reach {
    const EMPTY: Reach = Reach {
        n: 0,
        to: [Inst(0); MAX_REACH],
    };

    fn has(&self, i: Inst) -> bool {
        self.list().contains(&i)
    }

    fn list(&self) -> &[Inst] {
        &self.to[..usize::from(self.n)]
    }

    /// Adds `i`; false if it was there already or there's no room
    /// (counted).
    fn add(&mut self, i: Inst, stats: &mut Stats) -> bool {
        if self.has(i) {
            return false;
        }
        let n = usize::from(self.n);
        if n >= MAX_REACH {
            stats.reach_full += 1;
            return false;
        }
        self.to[n] = i;
        self.n += 1;
        true
    }

    fn remove(&mut self, i: Inst) {
        let n = usize::from(self.n);
        if let Some(at) = self.to[..n].iter().position(|&x| x == i) {
            self.to.copy_within(at + 1..n, at);
            self.n -= 1;
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Board {
    from: From,
    /// Each pedal's value while down (None: up).
    down: [Option<f32>; 3],
    /// The instruments each pedal went down on.
    reach: [Reach; 3],
    /// The old way heard each pedal go down.
    routed: [bool; 3],
    /// Where the pitch wheel and the other wheels are (None: never moved).
    bend: Option<f32>,
    wheels: [Option<f32>; 3],
}

impl Board {
    fn new(from: From) -> Board {
        Board {
            from,
            down: [None; 3],
            reach: [Reach::EMPTY; 3],
            routed: [false; 3],
            bend: None,
            wheels: [None; 3],
        }
    }

    fn any_down(&self) -> bool {
        self.down.iter().any(Option::is_some)
    }
}

/// What the keys play: the patch playing and everything held.
pub struct Keys {
    rig: Box<Rig>,
    now: Option<usize>,
    waiting: Option<Option<usize>>,
    held: Box<[Option<Held>; MAX_HELD]>,
    boards: Box<[Option<Board>; MAX_KEYBOARDS]>,
    pub stats: Stats,
}

impl Default for Keys {
    fn default() -> Self {
        Self::new()
    }
}

fn transposed(note: u8, by: i8) -> Option<u8> {
    let n = i16::from(note) + i16::from(by);
    u8::try_from(n).ok().filter(|&n| n <= 127)
}

impl Keys {
    /// No patches, nothing held: keys go the old way.
    pub fn new() -> Keys {
        Keys {
            rig: Box::default(),
            now: None,
            waiting: None,
            held: Box::new([None; MAX_HELD]),
            boards: Box::new([None; MAX_KEYBOARDS]),
            stats: Stats::default(),
        }
    }

    /// The song's patches change (an edit, another song). Keys held keep
    /// ending where they started. A patch playing that the new list
    /// doesn't have leaves the keys as routed. Gives the old copy back, to
    /// be freed off the audio thread.
    pub fn set_rig(&mut self, rig: Box<Rig>) -> Box<Rig> {
        let old = std::mem::replace(&mut self.rig, rig);
        if self.now.is_some_and(|i| self.rig.patch(i).is_none()) {
            self.now = None;
        }
        if let Some(Some(i)) = self.waiting {
            if self.rig.patch(i).is_none() {
                self.waiting = None;
            }
        }
        old
    }

    /// The patch playing (from 0), or None: keys as routed.
    pub fn playing(&self) -> Option<usize> {
        self.now
    }

    /// A switch waiting for the hands: the patch it goes to.
    pub fn waiting(&self) -> Option<Option<usize>> {
        self.waiting
    }

    /// Keys held now.
    pub fn held(&self) -> usize {
        self.held.iter().flatten().count()
    }

    /// No key held and no pedal down, on any keyboard.
    pub fn hands_up(&self) -> bool {
        self.held.iter().all(Option::is_none) && self.boards.iter().flatten().all(|b| !b.any_down())
    }

    fn patch(&self) -> Option<LivePatch> {
        self.now.and_then(|i| self.rig.patch(i)).copied()
    }

    /// Picks patch `to` (from 0; None: keys as routed), now or when the
    /// hands are up. A patch the song doesn't have is keys as routed. A
    /// later pick replaces one still waiting.
    pub fn pick(&mut self, to: Option<usize>, when: When, out: &mut impl FnMut(Out)) {
        let to = to.filter(|&i| self.rig.patch(i).is_some());
        match when {
            When::Now => self.switch(to, out),
            When::HandsUp if to == self.now => self.waiting = None,
            When::HandsUp if self.hands_up() => self.switch(to, out),
            When::HandsUp => self.waiting = Some(to),
        }
    }

    fn switch(&mut self, to: Option<usize>, out: &mut impl FnMut(Out)) {
        self.waiting = None;
        if to == self.now {
            return;
        }
        let new = to.and_then(|i| self.rig.patch(i)).copied();
        if new.is_some_and(|p| p.cut_old) {
            self.cut_old(out);
        }
        self.now = to;
        self.stats.switches += 1;
        let Some(p) = new else {
            return;
        };
        // Pedals still down carry on into the new patch (sustain and soft:
        // sostenuto holds only the notes it caught), and its instruments
        // start from where each keyboard's wheels are.
        let stats = &mut self.stats;
        for b in self.boards.iter_mut().flatten() {
            for i in [0, 2] {
                let Some(v) = b.down[i] else {
                    continue;
                };
                for z in p.zones.iter().flatten() {
                    let Some(inst) = z.inst else {
                        continue;
                    };
                    if z.pedals && z.takes.takes(b.from) && b.reach[i].add(inst, stats) {
                        out(Out::Control {
                            inst,
                            ctl: PEDALS[i],
                            value: v,
                        });
                    }
                }
            }
            let mut sent = Reach::EMPTY;
            for z in p.zones.iter().flatten() {
                let Some(inst) = z.inst else {
                    continue;
                };
                if !z.wheels || !z.takes.takes(b.from) || !sent.add(inst, stats) {
                    continue;
                }
                if let Some(value) = b.bend {
                    out(Out::Bend { inst, value });
                }
                for (w, v) in WHEELS.iter().zip(b.wheels) {
                    if let Some(value) = v {
                        out(Out::Control {
                            inst,
                            ctl: *w,
                            value,
                        });
                    }
                }
            }
        }
    }

    /// Ends what the patch before holds: every note it plays and every
    /// pedal it holds down (the pedals stay down on the keyboards, so they
    /// reach the new patch).
    fn cut_old(&mut self, out: &mut impl FnMut(Out)) {
        for slot in self.held.iter_mut() {
            if let Some(h) = slot {
                if h.routed {
                    continue;
                }
                for d in h.dests() {
                    out(Out::Off {
                        inst: d.inst,
                        note: d.note,
                    });
                }
                *slot = None;
            }
        }
        for b in self.boards.iter_mut().flatten() {
            for i in 0..3 {
                for &inst in b.reach[i].list() {
                    out(Out::Control {
                        inst,
                        ctl: PEDALS[i],
                        value: 0.0,
                    });
                }
                b.reach[i] = Reach::EMPTY;
            }
        }
    }

    fn check_waiting(&mut self, out: &mut impl FnMut(Out)) {
        if let Some(to) = self.waiting {
            if self.hands_up() {
                self.switch(to, out);
            }
        }
    }

    fn find(&self, from: From, note: u8) -> Option<usize> {
        self.held
            .iter()
            .position(|h| h.is_some_and(|h| h.from == from && h.note == note))
    }

    /// Plays `k` from `from`, giving what the instruments are sent to
    /// `out`.
    pub fn key(&mut self, from: From, k: In, out: &mut impl FnMut(Out)) -> Went {
        match k {
            In::Down { note, velocity: 0 } | In::Up { note } => self.up(from, note, out),
            In::Down { note, velocity } => self.down(from, note, velocity, out),
            In::Pressure { note, value } => self.on_note(from, note, true, out, |inst, note| {
                Out::Pressure { inst, note, value }
            }),
            // A release velocity belongs to the note's let-go, so it goes
            // wherever the note did; slide and tone are expression, which
            // a zone takes with its wheels and pressure.
            In::Expr { note, kind, value } => {
                let expression = kind != Expr::Release;
                self.on_note(from, note, expression, out, |inst, note| Out::Expr {
                    inst,
                    note,
                    kind,
                    value,
                })
            }
            In::Bend { value } => self.wheel(from, None, value, out),
            In::Control { ctl, value } => match PEDALS.iter().position(|&p| p == ctl) {
                Some(i) => self.pedal(from, i, value, out),
                None => self.wheel(from, Some(ctl), value, out),
            },
        }
    }

    fn routed_now(&self) -> bool {
        match self.patch() {
            None => true,
            Some(p) => p.outside == LiveOutside::Routed,
        }
    }

    fn down(&mut self, from: From, note: u8, vel: u8, out: &mut impl FnMut(Out)) -> Went {
        // Struck again: the first strike went the old way, so this one
        // does too; else its notes end before it plays again.
        if let Some(i) = self.find(from, note) {
            let h = self.held[i].take().expect("found");
            if h.routed {
                self.held[i] = Some(h);
                return Went::Routed;
            }
            for d in h.dests() {
                out(Out::Off {
                    inst: d.inst,
                    note: d.note,
                });
            }
        }
        let mut h = Held {
            from,
            note,
            routed: false,
            n: 0,
            to: [NO_DEST; MAX_ZONES],
        };
        let mut vels = [0u8; MAX_ZONES];
        match self.patch() {
            None => h.routed = true,
            Some(p) => {
                let mut taken = false;
                for z in p.zones.iter().flatten() {
                    if !z.takes(from, note, vel) {
                        continue;
                    }
                    taken = true;
                    let (Some(inst), Some(n)) = (z.inst, transposed(note, z.transpose)) else {
                        continue;
                    };
                    vels[usize::from(h.n)] = z.curve.apply(vel);
                    h.to[usize::from(h.n)] = Dest {
                        inst,
                        note: n,
                        pedals: z.pedals,
                        wheels: z.wheels,
                    };
                    h.n += 1;
                }
                if !taken {
                    match p.outside {
                        LiveOutside::Nothing | LiveOutside::Play(None) => {}
                        LiveOutside::Play(Some(inst)) => {
                            vels[0] = vel;
                            h.to[0] = Dest {
                                inst,
                                note,
                                pedals: true,
                                wheels: true,
                            };
                            h.n = 1;
                        }
                        LiveOutside::Routed => h.routed = true,
                    }
                }
            }
        }
        if !h.routed && h.n == 0 {
            return Went::Nowhere;
        }
        // Remembered first: a key the table has no room for plays
        // nothing, so nothing hangs. One going the old way still goes
        // there (its let-go does too while nothing changes).
        let Some(slot) = self.held.iter_mut().find(|s| s.is_none()) else {
            self.stats.held_full += 1;
            return if h.routed {
                Went::Routed
            } else {
                Went::Nowhere
            };
        };
        *slot = Some(h);
        if h.routed {
            return Went::Routed;
        }
        for (d, &velocity) in h.dests().iter().zip(vels.iter()) {
            out(Out::On {
                inst: d.inst,
                note: d.note,
                velocity,
            });
        }
        Went::Played
    }

    fn up(&mut self, from: From, note: u8, out: &mut impl FnMut(Out)) -> Went {
        let went = match self.find(from, note) {
            Some(i) => {
                let h = self.held[i].take().expect("found");
                if h.routed {
                    Went::Routed
                } else {
                    for d in h.dests() {
                        out(Out::Off {
                            inst: d.inst,
                            note: d.note,
                        });
                    }
                    Went::Played
                }
            }
            // Not remembered: the table was full, or the patch cut it.
            None if self.routed_now() => Went::Routed,
            None => Went::Nowhere,
        };
        self.check_waiting(out);
        went
    }

    /// Sends `msg` to where the held key went: to every zone it played,
    /// or (`expression`) only to those that take wheels and pressure.
    fn on_note(
        &mut self,
        from: From,
        note: u8,
        expression: bool,
        out: &mut impl FnMut(Out),
        msg: impl Fn(Inst, u8) -> Out,
    ) -> Went {
        match self.find(from, note).and_then(|i| self.held[i]) {
            Some(h) if h.routed => Went::Routed,
            Some(h) => {
                for d in h.dests().iter().filter(|d| d.wheels || !expression) {
                    out(msg(d.inst, d.note));
                }
                Went::Played
            }
            None if self.routed_now() => Went::Routed,
            None => Went::Nowhere,
        }
    }

    /// The keyboard's slot: its own, else a free one, else one with no
    /// pedal down (its wheels are forgotten).
    fn board(&mut self, from: From) -> Option<usize> {
        if let Some(i) = self
            .boards
            .iter()
            .position(|b| b.is_some_and(|b| b.from == from))
        {
            return Some(i);
        }
        let i = self.boards.iter().position(Option::is_none).or_else(|| {
            self.boards
                .iter()
                .position(|b| b.is_some_and(|b| !b.any_down()))
        });
        match i {
            Some(i) => {
                self.boards[i] = Some(Board::new(from));
                Some(i)
            }
            None => {
                self.stats.keyboards_full += 1;
                None
            }
        }
    }

    /// The instruments a move from `from` reaches: the patch's zones that
    /// take it (pedals or wheels as `pedal` says) and those this
    /// keyboard's held notes went to.
    fn reach(&mut self, from: From, pedal: bool) -> Reach {
        let mut r = Reach::EMPTY;
        let stats = &mut self.stats;
        if let Some(p) = self.now.and_then(|i| self.rig.patch(i)) {
            for z in p.zones.iter().flatten() {
                let on = if pedal { z.pedals } else { z.wheels };
                if let (true, Some(inst)) = (on && z.takes.takes(from), z.inst) {
                    r.add(inst, stats);
                }
            }
            if let LiveOutside::Play(Some(inst)) = p.outside {
                r.add(inst, stats);
            }
        }
        // A keyboard's notes: the same port (an MPE keyboard's notes each
        // have their own channel, its wheels the master's).
        for h in self.held.iter().flatten() {
            if h.from.port != from.port || h.routed {
                continue;
            }
            for d in h.dests() {
                if if pedal { d.pedals } else { d.wheels } {
                    r.add(d.inst, stats);
                }
            }
        }
        r
    }

    fn routed_from(&self, from: From) -> bool {
        self.routed_now()
            || self
                .held
                .iter()
                .flatten()
                .any(|h| h.routed && h.from.port == from.port)
    }

    fn wheel(
        &mut self,
        from: From,
        ctl: Option<Ctl>,
        value: f32,
        out: &mut impl FnMut(Out),
    ) -> Went {
        if let Some(b) = self.board(from) {
            let b = self.boards[b].as_mut().expect("claimed");
            match ctl {
                None => b.bend = Some(value),
                Some(c) => {
                    if let Some(i) = WHEELS.iter().position(|&w| w == c) {
                        b.wheels[i] = Some(value);
                    }
                }
            }
        }
        let r = self.reach(from, false);
        for &inst in r.list() {
            out(match ctl {
                None => Out::Bend { inst, value },
                Some(ctl) => Out::Control { inst, ctl, value },
            });
        }
        if self.routed_from(from) {
            Went::Routed
        } else if r.n > 0 {
            Went::Played
        } else {
            Went::Nowhere
        }
    }

    fn pedal(&mut self, from: From, i: usize, value: f32, out: &mut impl FnMut(Out)) -> Went {
        let ctl = PEDALS[i];
        let routed = self.routed_from(from);
        let Some(bi) = self.board(from) else {
            // Not followed (counted): it goes where it would now.
            let r = self.reach(from, true);
            for &inst in r.list() {
                out(Out::Control { inst, ctl, value });
            }
            return if routed { Went::Routed } else { Went::Played };
        };
        let down = Ctl::down(value);
        let was = self.boards[bi].expect("claimed").down[i].is_some();
        if down {
            if was {
                // Still down (a half-pedal value): it moves where it is.
                let b = self.boards[bi].as_mut().expect("claimed");
                b.down[i] = Some(value);
                for &inst in b.reach[i].list() {
                    out(Out::Control { inst, ctl, value });
                }
                return if b.routed[i] {
                    Went::Routed
                } else {
                    Went::Played
                };
            }
            let r = self.reach(from, true);
            let b = self.boards[bi].as_mut().expect("claimed");
            b.down[i] = Some(value);
            b.reach[i] = r;
            b.routed[i] = routed;
            for &inst in r.list() {
                out(Out::Control { inst, ctl, value });
            }
            return if routed {
                Went::Routed
            } else if r.n > 0 {
                Went::Played
            } else {
                Went::Nowhere
            };
        }
        if !was {
            return if routed { Went::Routed } else { Went::Nowhere };
        }
        let b = self.boards[bi].as_mut().expect("claimed");
        b.down[i] = None;
        let reach = std::mem::replace(&mut b.reach[i], Reach::EMPTY);
        let routed = std::mem::take(&mut b.routed[i]);
        // An instrument another keyboard still holds this pedal down on
        // keeps it down.
        for &inst in reach.list() {
            let kept = self
                .boards
                .iter()
                .flatten()
                .any(|o| o.down[i].is_some() && o.reach[i].has(inst));
            if !kept {
                out(Out::Control { inst, ctl, value });
            }
        }
        self.check_waiting(out);
        if routed {
            Went::Routed
        } else {
            Went::Played
        }
    }

    /// A keyboard's port went away: every note its keys hold ends and its
    /// pedals come up.
    pub fn unplug(&mut self, port: u8, out: &mut impl FnMut(Out)) {
        for slot in self.held.iter_mut() {
            if let Some(h) = slot.filter(|h| h.from.port == port) {
                if !h.routed {
                    for d in h.dests() {
                        out(Out::Off {
                            inst: d.inst,
                            note: d.note,
                        });
                    }
                }
                *slot = None;
            }
        }
        for bi in 0..MAX_KEYBOARDS {
            let Some(b) = self.boards[bi].filter(|b| b.from.port == port) else {
                continue;
            };
            self.boards[bi] = None;
            for i in 0..3 {
                if b.down[i].is_none() {
                    continue;
                }
                for &inst in b.reach[i].list() {
                    let kept = self
                        .boards
                        .iter()
                        .flatten()
                        .any(|o| o.down[i].is_some() && o.reach[i].has(inst));
                    if !kept {
                        out(Out::Control {
                            inst,
                            ctl: PEDALS[i],
                            value: 0.0,
                        });
                    }
                }
            }
        }
        self.check_waiting(out);
    }

    /// Panic: every note the table holds ends, every pedal comes up, and a
    /// switch waiting is dropped. The patch playing stays.
    pub fn panic(&mut self, out: &mut impl FnMut(Out)) {
        for slot in self.held.iter_mut() {
            if let Some(h) = slot.take() {
                if !h.routed {
                    for d in h.dests() {
                        out(Out::Off {
                            inst: d.inst,
                            note: d.note,
                        });
                    }
                }
            }
        }
        let mut up = [Reach::EMPTY; 3];
        let stats = &mut self.stats;
        for b in self.boards.iter_mut().flatten() {
            for i in 0..3 {
                for &inst in b.reach[i].list() {
                    up[i].add(inst, stats);
                }
                b.down[i] = None;
                b.reach[i] = Reach::EMPTY;
                b.routed[i] = false;
            }
        }
        for (i, r) in up.iter().enumerate() {
            for &inst in r.list() {
                out(Out::Control {
                    inst,
                    ctl: PEDALS[i],
                    value: 0.0,
                });
            }
        }
        self.waiting = None;
    }

    /// The app took `inst` out (its notes ended there): nothing more is
    /// sent to it for what is held now.
    pub fn forget(&mut self, inst: Inst) {
        for h in self.held.iter_mut().flatten() {
            let n = usize::from(h.n);
            let mut k = 0;
            for j in 0..n {
                if h.to[j].inst != inst {
                    h.to[k] = h.to[j];
                    k += 1;
                }
            }
            h.n = k as u8;
        }
        for b in self.boards.iter_mut().flatten() {
            for r in b.reach.iter_mut() {
                r.remove(inst);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Curve, Keyboard, Layout, Outside, Patch, Patches, Zone};

    const PIANO: Inst = Inst(1);
    const PAD: Inst = Inst(2);
    const BASS: Inst = Inst(3);
    const ORGAN: Inst = Inst(4);
    const KEYLAB: From = From { port: 1, ch: 0 };
    const NORD: From = From { port: 2, ch: 0 };

    fn inst(name: &str) -> Option<Inst> {
        match name {
            "piano" => Some(PIANO),
            "pad" => Some(PAD),
            "bass" => Some(BASS),
            "organ" => Some(ORGAN),
            _ => None,
        }
    }

    fn port(name: &str) -> Option<u8> {
        match name {
            "KeyLab" => Some(1),
            "Nord" => Some(2),
            _ => None,
        }
    }

    /// Patch 0: piano, every key. Patch 1: pad, every key. Patch 2: bass
    /// under C3 (no pedals), piano from C3 with the pad layered over it
    /// from velocity 64. Patch 3: organ, cutting the old sound.
    fn keys() -> Keys {
        let mut split = Layout::BassAndKeys.patch("Split", "bass");
        split.zones[1].plays = "piano".into();
        let mut pad = Zone::new("pad", 48, 127);
        pad.vel_low = 64;
        split.zones.push(pad);
        let mut organ = Layout::Whole.patch("Organ", "organ");
        organ.cut_old = true;
        let p = Patches {
            list: vec![
                Layout::Whole.patch("Piano", "piano"),
                Layout::Whole.patch("Pad", "pad"),
                split,
                organ,
            ],
        };
        let mut k = Keys::new();
        k.set_rig(p.resolve(inst, port));
        k
    }

    struct Run {
        k: Keys,
        sent: Vec<Out>,
    }

    impl Run {
        fn new(patch: Option<usize>) -> Run {
            let mut r = Run {
                k: keys(),
                sent: vec![],
            };
            r.pick(patch, When::Now);
            r.sent.clear();
            r
        }
        fn key(&mut self, from: From, k: In) -> Went {
            let sent = &mut self.sent;
            self.k.key(from, k, &mut |o| sent.push(o))
        }
        fn down(&mut self, from: From, note: u8, velocity: u8) -> Went {
            self.key(from, In::Down { note, velocity })
        }
        fn up(&mut self, from: From, note: u8) -> Went {
            self.key(from, In::Up { note })
        }
        fn pedal(&mut self, from: From, ctl: Ctl, down: bool) -> Went {
            self.key(
                from,
                In::Control {
                    ctl,
                    value: if down { 1.0 } else { 0.0 },
                },
            )
        }
        fn pick(&mut self, to: Option<usize>, when: When) {
            let sent = &mut self.sent;
            self.k.pick(to, when, &mut |o| sent.push(o));
        }
        fn take(&mut self) -> Vec<Out> {
            std::mem::take(&mut self.sent)
        }
    }

    fn on(inst: Inst, note: u8, velocity: u8) -> Out {
        Out::On {
            inst,
            note,
            velocity,
        }
    }
    fn off(inst: Inst, note: u8) -> Out {
        Out::Off { inst, note }
    }
    fn ped(inst: Inst, ctl: Ctl, down: bool) -> Out {
        Out::Control {
            inst,
            ctl,
            value: if down { 1.0 } else { 0.0 },
        }
    }

    #[test]
    fn a_note_held_across_a_switch_ends_on_the_old_instrument() {
        let mut r = Run::new(Some(0));
        assert_eq!(r.down(KEYLAB, 60, 100), Went::Played);
        assert_eq!(r.take(), vec![on(PIANO, 60, 100)]);
        r.pick(Some(1), When::Now);
        assert_eq!(r.k.playing(), Some(1));
        assert_eq!(r.up(KEYLAB, 60), Went::Played);
        assert_eq!(r.take(), vec![off(PIANO, 60)]);
        // The next one plays the pad.
        r.down(KEYLAB, 62, 90);
        assert_eq!(r.take(), vec![on(PAD, 62, 90)]);
    }

    #[test]
    fn a_pedal_held_across_switches_keeps_the_old_sound_and_reaches_the_new() {
        let mut r = Run::new(Some(0));
        r.down(KEYLAB, 60, 100);
        r.pedal(KEYLAB, Ctl::Sustain, true);
        r.up(KEYLAB, 60);
        assert_eq!(
            r.take(),
            vec![
                on(PIANO, 60, 100),
                ped(PIANO, Ctl::Sustain, true),
                off(PIANO, 60)
            ]
        );
        // The switch lifts nothing: the piano stays sustained, and the
        // pad hears the pedal down so its new notes sustain too.
        r.pick(Some(1), When::Now);
        assert_eq!(r.take(), vec![ped(PAD, Ctl::Sustain, true)]);
        r.down(KEYLAB, 64, 80);
        r.up(KEYLAB, 64);
        assert_eq!(r.take(), vec![on(PAD, 64, 80), off(PAD, 64)]);
        // A second switch, the pedal still down: the organ cuts the old
        // sound, so the piano and pad pedals come up and the organ's goes
        // down.
        r.pick(Some(0), When::Now);
        assert_eq!(r.take(), vec![]);
        r.pick(Some(3), When::Now);
        assert_eq!(
            r.take(),
            vec![
                ped(PIANO, Ctl::Sustain, false),
                ped(PAD, Ctl::Sustain, false),
                ped(ORGAN, Ctl::Sustain, true)
            ]
        );
        // Lifting it ends what it holds on the organ alone.
        r.pedal(KEYLAB, Ctl::Sustain, false);
        assert_eq!(r.take(), vec![ped(ORGAN, Ctl::Sustain, false)]);
        assert!(r.k.hands_up());
    }

    #[test]
    fn lifting_the_pedal_after_a_switch_ends_both_instruments_notes() {
        let mut r = Run::new(Some(0));
        r.pedal(KEYLAB, Ctl::Sustain, true);
        r.pick(Some(1), When::Now);
        r.take();
        r.pedal(KEYLAB, Ctl::Sustain, false);
        assert_eq!(
            r.take(),
            vec![
                ped(PIANO, Ctl::Sustain, false),
                ped(PAD, Ctl::Sustain, false)
            ]
        );
    }

    #[test]
    fn layered_zones_both_play_and_both_end_and_velocity_picks_the_layer() {
        let mut r = Run::new(Some(2));
        r.down(KEYLAB, 60, 100);
        r.down(KEYLAB, 64, 40);
        r.down(KEYLAB, 36, 70);
        assert_eq!(
            r.take(),
            vec![
                on(PIANO, 60, 100),
                on(PAD, 60, 100),
                on(PIANO, 64, 40),
                on(BASS, 36, 70)
            ]
        );
        r.up(KEYLAB, 60);
        r.up(KEYLAB, 64);
        r.up(KEYLAB, 36);
        assert_eq!(
            r.take(),
            vec![off(PIANO, 60), off(PAD, 60), off(PIANO, 64), off(BASS, 36)]
        );
    }

    #[test]
    fn two_keyboards_on_one_note_end_one_note_each() {
        let mut r = Run::new(Some(0));
        r.down(KEYLAB, 60, 100);
        r.down(NORD, 60, 90);
        r.up(KEYLAB, 60);
        // Two notes went in and one came out: the instrument counts them,
        // so the Nord's still sounds.
        assert_eq!(
            r.take(),
            vec![on(PIANO, 60, 100), on(PIANO, 60, 90), off(PIANO, 60)]
        );
        r.up(NORD, 60);
        assert_eq!(r.take(), vec![off(PIANO, 60)]);
    }

    #[test]
    fn a_key_struck_again_ends_its_note_before_playing_again() {
        let mut r = Run::new(Some(0));
        r.down(KEYLAB, 60, 100);
        r.down(KEYLAB, 60, 50);
        r.up(KEYLAB, 60);
        assert_eq!(
            r.take(),
            vec![
                on(PIANO, 60, 100),
                off(PIANO, 60),
                on(PIANO, 60, 50),
                off(PIANO, 60)
            ]
        );
        // A second let-go finds nothing.
        assert_eq!(r.up(KEYLAB, 60), Went::Nowhere);
        assert_eq!(r.take(), vec![]);
        assert_eq!(r.k.held(), 0);
    }

    #[test]
    fn held_notes_keep_their_bend_and_a_new_patch_starts_from_the_wheel() {
        let mut r = Run::new(Some(0));
        r.down(KEYLAB, 60, 100);
        r.key(KEYLAB, In::Bend { value: 0.5 });
        r.key(
            KEYLAB,
            In::Control {
                ctl: Ctl::ModWheel,
                value: 0.25,
            },
        );
        assert_eq!(
            r.take()[1..],
            [
                Out::Bend {
                    inst: PIANO,
                    value: 0.5
                },
                Out::Control {
                    inst: PIANO,
                    ctl: Ctl::ModWheel,
                    value: 0.25
                }
            ]
        );
        r.pick(Some(1), When::Now);
        // The pad starts where the wheels are; the piano isn't reset.
        assert_eq!(
            r.take(),
            vec![
                Out::Bend {
                    inst: PAD,
                    value: 0.5
                },
                Out::Control {
                    inst: PAD,
                    ctl: Ctl::ModWheel,
                    value: 0.25
                }
            ]
        );
        // Moving the wheel reaches the pad and the piano's held note.
        r.key(KEYLAB, In::Bend { value: 0.0 });
        assert_eq!(
            r.take(),
            vec![
                Out::Bend {
                    inst: PAD,
                    value: 0.0
                },
                Out::Bend {
                    inst: PIANO,
                    value: 0.0
                }
            ]
        );
        // Aftertouch goes to where the key went.
        r.key(
            KEYLAB,
            In::Pressure {
                note: 60,
                value: 70,
            },
        );
        assert_eq!(
            r.take(),
            vec![Out::Pressure {
                inst: PIANO,
                note: 60,
                value: 70
            }]
        );
    }

    #[test]
    fn a_zone_without_wheels_hears_no_pressure_but_hears_how_the_key_let_go() {
        let mut p = Layout::Layered.patch("Layers", "piano");
        p.zones[1].plays = "pad".into();
        p.zones[1].wheels = false;
        let mut r = Run {
            k: Keys::new(),
            sent: vec![],
        };
        r.k.set_rig(Patches { list: vec![p] }.resolve(inst, port));
        r.pick(Some(0), When::Now);
        r.down(KEYLAB, 60, 100);
        r.take();
        r.key(
            KEYLAB,
            In::Pressure {
                note: 60,
                value: 70,
            },
        );
        r.key(
            KEYLAB,
            In::Expr {
                note: 60,
                kind: Expr::Tone,
                value: 0.5,
            },
        );
        assert_eq!(
            r.take(),
            vec![
                Out::Pressure {
                    inst: PIANO,
                    note: 60,
                    value: 70
                },
                Out::Expr {
                    inst: PIANO,
                    note: 60,
                    kind: Expr::Tone,
                    value: 0.5
                }
            ]
        );
        r.key(
            KEYLAB,
            In::Expr {
                note: 60,
                kind: Expr::Release,
                value: 0.2,
            },
        );
        r.up(KEYLAB, 60);
        let release = |inst| Out::Expr {
            inst,
            note: 60,
            kind: Expr::Release,
            value: 0.2,
        };
        assert_eq!(
            r.take(),
            vec![release(PIANO), release(PAD), off(PIANO, 60), off(PAD, 60)]
        );
    }

    #[test]
    fn unplugging_a_keyboard_ends_its_notes_and_pedals() {
        let mut r = Run::new(Some(2));
        r.down(KEYLAB, 60, 100);
        r.down(KEYLAB, 36, 100);
        r.pedal(KEYLAB, Ctl::Sustain, true);
        r.down(NORD, 72, 30);
        r.take();
        let sent = &mut r.sent;
        r.k.unplug(1, &mut |o| sent.push(o));
        // The bass takes no pedal, so only the piano and pad hear it lift.
        assert_eq!(
            r.take(),
            vec![
                off(PIANO, 60),
                off(PAD, 60),
                off(BASS, 36),
                ped(PIANO, Ctl::Sustain, false),
                ped(PAD, Ctl::Sustain, false)
            ]
        );
        assert_eq!(r.k.held(), 1);
    }

    #[test]
    fn sostenuto_holds_only_what_it_caught_across_a_switch() {
        let mut r = Run::new(Some(0));
        r.down(KEYLAB, 60, 100);
        r.pedal(KEYLAB, Ctl::Sostenuto, true);
        r.pedal(KEYLAB, Ctl::Soft, true);
        r.take();
        r.pick(Some(1), When::Now);
        // Soft carries on into the pad; sostenuto doesn't.
        assert_eq!(r.take(), vec![ped(PAD, Ctl::Soft, true)]);
        r.pedal(KEYLAB, Ctl::Sostenuto, false);
        assert_eq!(r.take(), vec![ped(PIANO, Ctl::Sostenuto, false)]);
    }

    #[test]
    fn panic_during_a_switch_ends_everything_and_drops_the_switch() {
        let mut r = Run::new(Some(2));
        r.down(KEYLAB, 60, 100);
        r.down(NORD, 40, 100);
        r.pedal(KEYLAB, Ctl::Sustain, true);
        r.pick(Some(1), When::HandsUp);
        assert_eq!(r.k.waiting(), Some(Some(1)));
        r.take();
        let sent = &mut r.sent;
        r.k.panic(&mut |o| sent.push(o));
        assert_eq!(
            r.take(),
            vec![
                off(PIANO, 60),
                off(PAD, 60),
                off(BASS, 40),
                ped(PIANO, Ctl::Sustain, false),
                ped(PAD, Ctl::Sustain, false)
            ]
        );
        assert_eq!(r.k.waiting(), None);
        assert_eq!(r.k.playing(), Some(2));
        assert!(r.k.hands_up());
    }

    #[test]
    fn two_keyboards_hold_one_instruments_sustain_until_both_lift() {
        let mut r = Run::new(Some(0));
        r.pedal(KEYLAB, Ctl::Sustain, true);
        r.pedal(NORD, Ctl::Sustain, true);
        r.take();
        r.pedal(KEYLAB, Ctl::Sustain, false);
        assert_eq!(r.take(), vec![]);
        r.pedal(NORD, Ctl::Sustain, false);
        assert_eq!(r.take(), vec![ped(PIANO, Ctl::Sustain, false)]);
    }

    #[test]
    fn a_switch_can_wait_for_the_hands() {
        let mut r = Run::new(Some(0));
        r.down(KEYLAB, 60, 100);
        r.pedal(KEYLAB, Ctl::Sustain, true);
        r.pick(Some(1), When::HandsUp);
        assert_eq!(r.k.playing(), Some(0));
        r.up(KEYLAB, 60);
        // The pedal is still down.
        assert_eq!(r.k.playing(), Some(0));
        r.down(KEYLAB, 62, 100);
        r.take();
        r.pedal(KEYLAB, Ctl::Sustain, false);
        assert_eq!(r.k.playing(), Some(0));
        r.up(KEYLAB, 62);
        assert_eq!(r.k.playing(), Some(1));
        assert_eq!(r.k.waiting(), None);
        // With hands up already, it happens at once; picking the patch
        // playing drops one waiting.
        r.pick(Some(0), When::HandsUp);
        assert_eq!(r.k.playing(), Some(0));
        r.down(KEYLAB, 60, 100);
        r.pick(Some(1), When::HandsUp);
        r.pick(Some(0), When::HandsUp);
        assert_eq!(r.k.waiting(), None);
        assert_eq!(r.k.stats.switches, 3);
    }

    #[test]
    fn keys_outside_the_zones_follow_the_patch() {
        let mut p = Patches {
            list: vec![Layout::BassAndKeys.patch("Gap", "bass")],
        };
        p.list[0].zones[0].high = 40;
        p.list[0].zones[1].low = 60;
        let mut r = Run::new(None);
        // No patch: everything goes the old way.
        assert_eq!(r.down(KEYLAB, 50, 100), Went::Routed);
        r.k.set_rig(p.resolve(inst, port));
        r.pick(Some(0), When::Now);
        // Its let-go goes the old way too, though a patch plays now.
        assert_eq!(r.up(KEYLAB, 50), Went::Routed);
        // Nothing (the default): the gap is quiet.
        assert_eq!(r.down(KEYLAB, 50, 100), Went::Nowhere);
        assert_eq!(r.up(KEYLAB, 50), Went::Nowhere);
        // An instrument, or the old way.
        for (outside, went, sent) in [
            (
                Outside::Play("organ".into()),
                Went::Played,
                vec![on(ORGAN, 50, 100), off(ORGAN, 50)],
            ),
            (Outside::Routed, Went::Routed, vec![]),
        ] {
            p.list[0].outside = outside;
            r.k.set_rig(p.resolve(inst, port));
            assert_eq!(r.down(KEYLAB, 50, 100), went);
            assert_eq!(r.up(KEYLAB, 50), went);
            assert_eq!(r.take(), sent);
        }
        assert!(r.k.hands_up());
    }

    #[test]
    fn a_zone_with_its_instrument_missing_takes_its_keys_and_plays_nothing() {
        let mut p = Patches {
            list: vec![Layout::BassAndKeys.patch("Gone", "brass")],
        };
        p.list[0].outside = Outside::Play("organ".into());
        let mut r = Run::new(None);
        r.k.set_rig(p.resolve(inst, port));
        r.pick(Some(0), When::Now);
        assert_eq!(r.down(KEYLAB, 60, 100), Went::Nowhere);
        assert_eq!(r.take(), vec![]);
    }

    #[test]
    fn a_zone_takes_its_keyboard_and_its_transpose_and_curve() {
        let mut lead = Layout::Whole.patch("Lead", "pad");
        lead.zones[0].keyboard = Keyboard::Port {
            name: "KeyLab".into(),
            ch: None,
        };
        lead.zones[0].transpose = 12;
        lead.zones[0].curve = Curve::Fixed(100);
        let mut low = Zone::new("bass", 0, 127);
        low.keyboard = Keyboard::Port {
            name: "Nord".into(),
            ch: Some(3),
        };
        low.transpose = -24;
        lead.zones.push(low);
        let p = Patches { list: vec![lead] };
        let mut r = Run::new(None);
        r.k.set_rig(p.resolve(inst, port));
        r.pick(Some(0), When::Now);
        // An MPE note on its own channel of the KeyLab; the Nord's channel
        // 4 alone reaches the bass; a key moved off the keyboard drops.
        r.down(From { port: 1, ch: 5 }, 60, 30);
        r.down(From { port: 2, ch: 3 }, 60, 30);
        r.down(From { port: 2, ch: 0 }, 61, 30);
        r.down(From { port: 2, ch: 3 }, 10, 30);
        r.down(From { port: 1, ch: 0 }, 120, 30);
        assert_eq!(r.take(), vec![on(PAD, 72, 100), on(BASS, 36, 30)]);
        // The MPE note's own expression follows it.
        r.key(
            From { port: 1, ch: 5 },
            In::Expr {
                note: 60,
                kind: Expr::Slide,
                value: 0.5,
            },
        );
        assert_eq!(
            r.take(),
            vec![Out::Expr {
                inst: PAD,
                note: 72,
                kind: Expr::Slide,
                value: 0.5
            }]
        );
    }

    #[test]
    fn a_full_table_plays_nothing_more_and_counts() {
        let mut r = Run::new(Some(0));
        for i in 0..MAX_HELD {
            let from = From {
                port: (i / 128) as u8 + 1,
                ch: ((i / 64) % 2) as u8,
            };
            assert_eq!(r.down(from, (i % 64) as u8, 100), Went::Played);
        }
        assert_eq!(r.down(NORD, 100, 100), Went::Nowhere);
        assert_eq!(r.k.stats.held_full, 1);
        r.take();
        assert_eq!(r.up(NORD, 100), Went::Nowhere);
        assert_eq!(r.take(), vec![]);
    }

    #[test]
    fn a_patch_edited_while_keys_are_held_ends_them_where_they_started() {
        let mut r = Run::new(Some(0));
        r.down(KEYLAB, 60, 100);
        let p = Patches {
            list: vec![Layout::Whole.patch("Organ now", "organ")],
        };
        let old = r.k.set_rig(p.resolve(inst, port));
        assert_eq!(old.len(), 4);
        r.up(KEYLAB, 60);
        assert_eq!(r.take(), vec![on(PIANO, 60, 100), off(PIANO, 60)]);
        // A patch the list no longer has leaves the keys as routed.
        r.pick(Some(0), When::Now);
        let p = Patches { list: vec![] };
        r.k.set_rig(p.resolve(inst, port));
        assert_eq!(r.k.playing(), None);
        assert_eq!(r.down(KEYLAB, 60, 100), Went::Routed);
    }

    #[test]
    fn a_forgotten_instrument_hears_nothing_more() {
        let mut r = Run::new(Some(2));
        r.down(KEYLAB, 60, 100);
        r.pedal(KEYLAB, Ctl::Sustain, true);
        r.take();
        r.k.forget(PAD);
        r.up(KEYLAB, 60);
        r.pedal(KEYLAB, Ctl::Sustain, false);
        assert_eq!(
            r.take(),
            vec![off(PIANO, 60), ped(PIANO, Ctl::Sustain, false)]
        );
    }

    #[test]
    fn nothing_allocates_once_made() {
        // The table and boards are boxed arrays made by new(); every path
        // above works in place. This test only pins the sizes the audio
        // loop relies on.
        assert!(std::mem::size_of::<Option<Held>>() * MAX_HELD < 8 * 1024);
        let _ = Patch::default();
    }
}
