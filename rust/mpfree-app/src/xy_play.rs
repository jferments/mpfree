//! The XY pad playing a layout of yours, on the sound loop (the XY
//! controller plan, phase 2: "any setting on X and Y").
//!
//! The ten ready-made layouts play as they always did (`mixing.rs`'s
//! `play_xy`, held frame for frame by the golden file). A layout of yours
//! plays through the hand layer instead, so the pad never overwrites a
//! setting's own value:
//!
//! - **Set** settings: the pad's hand holds each one (`ModMix::pad_touch`,
//!   owner `Hand::Pad`), over a recorded move always and over a step's
//!   lock while recording, as any hand.
//! - **Add** settings: the pad pushes each one by a mod matrix row from a
//!   host source of its own (`mpfree_mod::XY_SOURCES`, one per place on
//!   the layout, so a layout's sixteen always find one), made when the
//!   layout loads and taken out when another takes its place. The push
//!   sums with an LFO and keeps working under locks and moves. A setting
//!   whose row the engine refuses (no room, or a switch) is counted and
//!   pushes nothing.
//! - **The gate** (a layout of yours made from an XY FX's: its Active):
//!   the pad's hand holds it at 1 from the landing, so the effect plays,
//!   and lets go of it at once at the lift (Go back or Stay), so an echo
//!   rings out; Hold keeps it open.
//! - **When the finger lifts:** with Hold on, nothing lets go until Hold
//!   is off. Go back lets each Set setting glide back to its own value as
//!   it is by then (`ModEvent::LetGo`), over the layout's glide. Stay makes
//!   each Set setting's value its own, as a turn of its knob would, and
//!   hands the turns to the screen for one UNDO step ([`XyPlay::kept`]);
//!   while recording it lets go instead, so the saved value stays as it
//!   was. An Add setting's push always glides away (over the glide, or
//!   [`PUSH_AWAY_MS`] with Stay).
//! - **Letting go by accident never holds:** a layout switched, a project
//!   opened or a gesture cancelled lets go of everything at once, even
//!   with Hold on ([`XyPlay::cancel`]).
//! - **A touch keeps its settings to the end:** what a gesture plays (a
//!   layout following the selected track points at that track's settings)
//!   is fixed when it begins, until it lets go.
//!
//! What a setting is reached through is the [`Reach`]: the mod engine's
//! settings (the mixer, effects, MIDI effects, outside gear, macros and
//! the morph) straight through `ModMix`, and a synth's and a pad's own
//! settings through their lock tables' hand layer (`xy_own.rs`). Nothing
//! here allocates after [`XyPlay::new`].

use crate::modmix::ModMix;
use crate::xy_own::{is_own, OwnHands};
use mpfree_fx::Mixer;
use mpfree_play::mpfree_param::ParamAddr;
use mpfree_xy::{Compiled, How, Lift, SLOTS};

/// How long an Add setting's push takes to glide away after a lift with
/// Stay (Go back uses the layout's own glide).
pub const PUSH_AWAY_MS: f32 = 120.0;

/// What a layout of yours does to the settings it plays: the sound loop's
/// side of each kind of setting.
pub trait Reach {
    /// The pad's hand moves `addr` to `value` (a Set setting): false when
    /// nothing reaches it.
    fn touch(&mut self, addr: ParamAddr, value: f32) -> bool;
    /// The pad lets go of `addr` over `frames` (0: at once).
    fn let_go(&mut self, addr: ParamAddr, frames: u32);
    /// Stay: the pad's `value` becomes `addr`'s own and the pad lets go.
    /// The own value it had before, for UNDO (None: the pad no longer held
    /// it).
    fn keep(&mut self, addr: ParamAddr, value: f32) -> Option<f32>;
    /// An Add setting: a matrix row from host source `n` to `addr`
    /// (pushes of -1 to 1 are the knob's whole travel). False when
    /// refused.
    fn row(&mut self, n: u8, addr: ParamAddr) -> bool;
    /// The row from host source `n` to `addr` goes.
    fn unrow(&mut self, n: u8, addr: ParamAddr);
    /// Host source `n`'s push, -1 to 1.
    fn push(&mut self, n: u8, value: f32);
    /// Host source `n`'s push as it is now.
    fn pushed(&self, n: u8) -> f32;
}

/// Where setting `addr` of a layout that follows the selected track goes,
/// with track `track` selected: a synth's setting means that track's
/// synth; everything else stays where it is.
pub fn followed(addr: ParamAddr, track: u8) -> ParamAddr {
    match addr.owner {
        mpfree_play::mpfree_param::Owner::Inst(_) => {
            ParamAddr::new(mpfree_play::mpfree_param::Owner::Inst(track), addr.param)
        }
        _ => addr,
    }
}

/// `ms` in frames at `rate`.
pub fn frames(rate: u32, ms: f32) -> u32 {
    (rate as f32 * ms.max(0.0) / 1000.0) as u32
}

/// Every setting a layout of yours reaches: the mod engine's (the mixer's
/// strips and effects, the tracks' MIDI effects and outside gear, the
/// macros and the morph, `modmix.rs`), and a synth's and a pad's own,
/// whose moves wait in `xy_own.rs` for the sound loop to hand on.
pub struct ModReach<'a> {
    modu: &'a mut ModMix,
    own: &'a mut OwnHands,
    mixer: &'a Mixer,
    recording: bool,
}

impl<'a> ModReach<'a> {
    /// `recording`: a track records, so the pad's hand beats a step's
    /// lock.
    pub fn new(
        modu: &'a mut ModMix,
        own: &'a mut OwnHands,
        mixer: &'a Mixer,
        recording: bool,
    ) -> ModReach<'a> {
        ModReach {
            modu,
            own,
            mixer,
            recording,
        }
    }
}

impl Reach for ModReach<'_> {
    fn touch(&mut self, addr: ParamAddr, value: f32) -> bool {
        if is_own(addr) {
            return self.own.touch(addr, value, self.recording);
        }
        self.modu.pad_touch(self.mixer, addr, value, self.recording)
    }
    fn let_go(&mut self, addr: ParamAddr, frames: u32) {
        if is_own(addr) {
            return self.own.let_go(addr, frames);
        }
        self.modu.pad_let_go(addr, frames);
    }
    fn keep(&mut self, addr: ParamAddr, value: f32) -> Option<f32> {
        if is_own(addr) {
            // Its turn for UNDO comes when the sound loop hands it on
            // (`OwnHands::apply`), which knows the value before.
            self.own.keep(addr, value);
            return None;
        }
        self.modu.pad_keep(addr, value)
    }
    fn row(&mut self, n: u8, addr: ParamAddr) -> bool {
        // A synth's or a pad's own setting doesn't go through the mod
        // engine: Add can't push it yet (Setup greys it).
        !is_own(addr) && self.modu.pad_row(self.mixer, n, addr, 1.0).is_ok()
    }
    fn unrow(&mut self, n: u8, addr: ParamAddr) {
        self.modu.pad_unrow(n, addr);
    }
    fn push(&mut self, n: u8, value: f32) {
        self.modu.pad_push(n, value);
    }
    fn pushed(&self, n: u8) -> f32 {
        self.modu.pushed(n)
    }
}

/// The host source an Add setting in place `slot` pushes on.
pub fn source(slot: usize) -> u8 {
    (mpfree_mod::XY_SOURCES.start + slot) as u8
}

const _: () = assert!(mpfree_mod::XY_SOURCES.end - mpfree_mod::XY_SOURCES.start >= SLOTS);

/// An Add setting's push gliding away.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Away {
    from: f32,
    left: u32,
    all: u32,
}

/// A layout of yours on the sound loop. See the module docs.
#[derive(Debug)]
pub struct XyPlay {
    rate: u32,
    /// The layout of yours playing (None: a ready-made one plays).
    layout: Option<Compiled>,
    /// What the gesture plays, fixed when it began.
    now: Option<Compiled>,
    /// Add settings whose row is in.
    rowed: [bool; SLOTS],
    /// Settings the pad holds (a Set setting touched, an Add one pushed).
    held: [bool; SLOTS],
    /// Each setting's last value from the pad (a push for Add).
    last: [f32; SLOTS],
    away: [Away; SLOTS],
    /// The pad holds its layout's gate open (an XY FX's Active).
    gated: bool,
    hold: bool,
    down: bool,
    /// Where the pad is, where it was when the finger landed, and where
    /// the finger landed (for a touch that moves it from where it is).
    pos: (f32, f32),
    here: (f32, f32),
    landed: (f32, f32),
    /// Stay's turns for UNDO, as one step: (setting, before, after).
    kept: Vec<(ParamAddr, f32, f32)>,
    /// Add settings whose row the engine refused.
    pub refused: u64,
    /// Set settings nothing reached.
    pub unreached: u64,
}

impl XyPlay {
    /// Nothing playing yet, for a sound at `rate` frames a second.
    /// Allocates (before the sound starts).
    pub fn new(rate: u32) -> XyPlay {
        XyPlay {
            rate: rate.max(1),
            layout: None,
            now: None,
            rowed: [false; SLOTS],
            held: [false; SLOTS],
            last: [0.0; SLOTS],
            away: [Away::default(); SLOTS],
            gated: false,
            hold: false,
            down: false,
            pos: (0.5, 0.5),
            here: (0.5, 0.5),
            landed: (0.5, 0.5),
            kept: Vec::with_capacity(SLOTS),
            refused: 0,
            unreached: 0,
        }
    }

    /// Whether a layout of yours plays (else a ready-made one does).
    pub fn playing(&self) -> bool {
        self.layout.is_some()
    }

    /// `layout` plays from now (None: a ready-made one does). The old one
    /// lets go of everything first, at once, and its rows go; the new
    /// one's Add settings get their rows, and its Hold starts as the
    /// layout says.
    pub fn load(&mut self, r: &mut dyn Reach, layout: Option<Compiled>) {
        self.cancel(r);
        if let Some(old) = self.layout.take() {
            for (i, t) in old.slots() {
                if self.rowed[i] {
                    r.unrow(source(i), t.addr);
                }
            }
        }
        self.rowed = [false; SLOTS];
        self.now = None;
        self.hold = false;
        let Some(l) = layout else {
            return;
        };
        for (i, t) in l.slots() {
            if t.how == How::Add {
                r.push(source(i), 0.0);
                self.rowed[i] = r.row(source(i), t.addr);
                if !self.rowed[i] {
                    self.refused += 1;
                }
            }
        }
        self.hold = l.hold();
        self.pos = l.rest();
        self.layout = Some(l);
    }

    /// A finger (or a controller) lands at `x`, `y`. `follow` points a
    /// layout that follows the selected track at that track's settings,
    /// fixed until the gesture lets go.
    pub fn begin(
        &mut self,
        r: &mut dyn Reach,
        x: f32,
        y: f32,
        follow: impl Fn(ParamAddr) -> ParamAddr,
    ) {
        let Some(l) = self.layout else {
            return;
        };
        // A new touch while the last one is still held (Hold): its
        // settings carry on under the new gesture.
        if self.now.is_none() {
            self.now = Some(if l.follows() { l.mapped(follow) } else { l });
        }
        self.down = true;
        self.here = self.pos;
        self.landed = (x, y);
        // The gate opens while the pad is held (a copy of an XY FX's
        // layout: the effect plays).
        if let Some(g) = self.now.and_then(|c| c.gate()) {
            if !self.gated {
                self.gated = r.touch(g, 1.0);
            }
        }
        self.moved(r, x, y);
    }

    /// The finger moves to `x`, `y`.
    pub fn moved(&mut self, r: &mut dyn Reach, x: f32, y: f32) {
        let Some(c) = self.now else {
            return;
        };
        if !self.down {
            return;
        }
        let (px, py) = c.start().place(self.here, self.landed, (x, y));
        self.pos = (px, py);
        c.each(px, py, |i, t, v| {
            self.last[i] = v;
            match t.how {
                How::Set => {
                    if r.touch(t.addr, v) {
                        self.held[i] = true;
                    } else {
                        self.unreached += 1;
                    }
                }
                How::Add if self.rowed[i] => {
                    self.away[i] = Away::default();
                    r.push(source(i), v);
                    self.held[i] = true;
                }
                How::Add => {}
            }
        });
    }

    /// The finger lifts. With Hold on the pad keeps its settings;
    /// otherwise it lets go as the layout says. `recording`: a track
    /// records, so Stay keeps no value.
    pub fn end(&mut self, r: &mut dyn Reach, recording: bool) {
        if !self.down {
            return;
        }
        self.down = false;
        if !self.hold {
            self.lift(r, recording);
        }
    }

    /// Hold on or off. Turned off with no finger down, the pad lets go as
    /// the layout says.
    pub fn set_hold(&mut self, r: &mut dyn Reach, on: bool, recording: bool) {
        self.hold = on;
        if !on && !self.down {
            self.lift(r, recording);
        }
    }

    /// Whether Hold is on.
    pub fn hold(&self) -> bool {
        self.hold
    }

    /// Whether a finger (or a controller) holds the pad.
    pub fn down(&self) -> bool {
        self.down
    }

    /// Where the pad is, 0 to 1 each way.
    pub fn position(&self) -> (f32, f32) {
        self.pos
    }

    /// Whether the pad holds any setting (a finger, Hold, or a push
    /// gliding away).
    // The XY page's Hold and Stay's UNDO (the screens' pull request).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn holds(&self) -> bool {
        self.gated || self.held.iter().any(|&h| h)
    }

    fn frames(&self, ms: f32) -> u32 {
        frames(self.rate, ms)
    }

    /// Lets go as the layout says.
    fn lift(&mut self, r: &mut dyn Reach, recording: bool) {
        let Some(c) = self.now.take() else {
            return;
        };
        let (ms, stay) = match c.lift() {
            Lift::GoBack { snap_ms } => (snap_ms, false),
            Lift::Stay => (PUSH_AWAY_MS, true),
        };
        let frames = self.frames(ms);
        // The gate closes at once, Stay or not (an echo rings out).
        if let Some(g) = c.gate().filter(|_| self.gated) {
            r.let_go(g, 0);
            self.gated = false;
        }
        for (i, t) in c.slots() {
            if !self.held[i] {
                continue;
            }
            match t.how {
                How::Set => {
                    self.held[i] = false;
                    if stay && !recording {
                        if let Some(before) = r.keep(t.addr, self.last[i]) {
                            if self.kept.len() < self.kept.capacity() {
                                self.kept.push((t.addr, before, self.last[i]));
                            }
                        }
                    } else {
                        r.let_go(t.addr, if stay { 0 } else { frames });
                    }
                }
                How::Add => {
                    self.away[i] = Away {
                        from: self.last[i],
                        left: frames,
                        all: frames,
                    };
                    if frames == 0 {
                        r.push(source(i), 0.0);
                        self.held[i] = false;
                    }
                }
            }
        }
        if !stay {
            // Gone back: a touch that moves the pad from where it is starts
            // from where it rests. With Stay it stays where the finger
            // left it.
            self.pos = c.rest();
        }
    }

    /// Lets go of everything at once (a layout switched, a project opened,
    /// a gesture cancelled, a controller unplugged), even with Hold on.
    pub fn cancel(&mut self, r: &mut dyn Reach) {
        if let Some(c) = self.now.take().or(self.layout) {
            if let Some(g) = c.gate().filter(|_| self.gated) {
                r.let_go(g, 0);
            }
            for (i, t) in c.slots() {
                if !self.held[i] {
                    continue;
                }
                match t.how {
                    How::Set => r.let_go(t.addr, 0),
                    How::Add => r.push(source(i), 0.0),
                }
            }
        }
        self.held = [false; SLOTS];
        self.away = [Away::default(); SLOTS];
        self.gated = false;
        self.down = false;
    }

    /// Once a period of `frames`: pushes glide away, and a push that should
    /// be 0 by now is sent again until it is (a push lost to a full queue
    /// never stays on).
    pub fn tick(&mut self, r: &mut dyn Reach, frames: u32) {
        let Some(l) = self.layout else {
            return;
        };
        for (i, t) in l.slots() {
            if t.how != How::Add || !self.rowed[i] {
                continue;
            }
            let a = &mut self.away[i];
            if a.left > 0 {
                a.left = a.left.saturating_sub(frames);
                let v = a.from * a.left as f32 / a.all.max(1) as f32;
                r.push(source(i), v);
                if a.left == 0 {
                    self.held[i] = false;
                }
            } else if !self.held[i] && r.pushed(source(i)) != 0.0 {
                r.push(source(i), 0.0);
            }
        }
    }

    /// A Stay turn on a synth's or a pad's own setting, which comes from
    /// the sound loop once it has handed the setting on (`xy_own.rs`): it
    /// joins the lift's UNDO step.
    pub fn add_kept(&mut self, addr: ParamAddr, before: f32, after: f32) {
        if self.kept.len() < self.kept.capacity() {
            self.kept.push((addr, before, after));
        }
    }

    /// Stay's turns since the last call, as one UNDO step: each setting,
    /// its own value before and after.
    pub fn take_kept(&mut self) -> std::vec::Drain<'_, (ParamAddr, f32, f32)> {
        self.kept.drain(..)
    }

    /// The status line's words.
    pub fn describe(&self) -> String {
        let n = self.layout.map_or(0, |l| l.slots().count());
        format!(
            "XY layout of yours: {} ({n} settings, {} pushed by Add, {} Add refused a row, {} not reached){}",
            if self.layout.is_some() { "playing" } else { "none" },
            self.rowed.iter().filter(|&&r| r).count(),
            self.refused,
            self.unreached,
            if self.hold { ", Hold on" } else { "" }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_play::mpfree_param::{Owner, Strip};
    use mpfree_xy::{Layout, Target, TARGETS};

    /// A reach that writes down what it is asked.
    #[derive(Default)]
    struct Log {
        said: Vec<String>,
        push: [f32; 32],
    }

    impl Reach for Log {
        fn touch(&mut self, addr: ParamAddr, value: f32) -> bool {
            self.said.push(format!("touch {addr} {value}"));
            true
        }
        fn let_go(&mut self, addr: ParamAddr, frames: u32) {
            self.said.push(format!("let go {addr} {frames}"));
        }
        fn keep(&mut self, addr: ParamAddr, value: f32) -> Option<f32> {
            self.said.push(format!("keep {addr} {value}"));
            Some(0.0)
        }
        fn row(&mut self, n: u8, addr: ParamAddr) -> bool {
            self.said.push(format!("row {n} {addr}"));
            true
        }
        fn unrow(&mut self, n: u8, addr: ParamAddr) {
            self.said.push(format!("unrow {n} {addr}"));
        }
        fn push(&mut self, n: u8, value: f32) {
            self.push[n as usize] = value;
        }
        fn pushed(&self, n: u8) -> f32 {
            self.push[n as usize]
        }
    }

    fn insert(uid: u16, id: u16) -> ParamAddr {
        ParamAddr::new(Owner::Insert(Strip::Track(0), uid), id)
    }

    /// A layout following the selected track keeps what it plays for the
    /// whole gesture: selecting another track mid-move changes the next
    /// gesture, not this one.
    #[test]
    fn a_touch_keeps_its_settings_to_the_end() {
        let mut l = Layout::new("Follow");
        l.put(false, Target::linear(insert(1, 3), 0.0, 1.0))
            .unwrap();
        l.follow = true;
        let mut p = XyPlay::new(1000);
        let mut r = Log::default();
        p.load(&mut r, Some(l.compile().unwrap()));
        // The selected track's insert is 7 when the finger lands...
        p.begin(&mut r, 0.5, 0.5, |a| ParamAddr {
            owner: Owner::Insert(Strip::Track(0), 7),
            ..a
        });
        p.moved(&mut r, 1.0, 0.5);
        // ...and a new pick mid-move (a second Begin from the other
        // controller is the only way in) doesn't move it to 9.
        p.begin(&mut r, 1.0, 0.5, |a| ParamAddr {
            owner: Owner::Insert(Strip::Track(0), 9),
            ..a
        });
        p.end(&mut r, false);
        let nine = insert(9, 3).to_string();
        assert!(r.said.iter().all(|s| !s.contains(&nine)), "{:?}", r.said);
        let first = format!("touch {} 0.5", insert(7, 3));
        assert_eq!(r.said[0], first);
        assert_eq!(
            r.said.last().unwrap(),
            &format!("let go {} 120", insert(7, 3))
        );
        // The next gesture follows the new pick.
        r.said.clear();
        p.begin(&mut r, 0.0, 0.5, |a| ParamAddr {
            owner: Owner::Insert(Strip::Track(0), 9),
            ..a
        });
        assert_eq!(r.said[0], format!("touch {} 0", insert(9, 3)));
    }

    /// Following the selected track moves a synth's settings to that
    /// track's synth and leaves the rest where they are.
    #[test]
    fn a_following_layout_plays_the_selected_tracks_synth() {
        let cutoff = ParamAddr::new(Owner::Inst(1), 3);
        assert_eq!(followed(cutoff, 4), ParamAddr::new(Owner::Inst(4), 3));
        let fx = insert(2, 3);
        assert_eq!(followed(fx, 4), fx);
        let pad = ParamAddr::new(Owner::Pad(1, 2), 3);
        assert_eq!(followed(pad, 4), pad);
    }

    /// Cancelling lets go of everything at once, even with Hold on, and
    /// pushes go to 0 at once; a push lost on the way is sent again.
    #[test]
    fn a_cancel_lets_go_of_everything_at_once_and_pushes_land_on_0() {
        let mut l = Layout::new("Both");
        l.put(false, Target::linear(insert(1, 3), 0.0, 1.0))
            .unwrap();
        l.put(true, Target::add(insert(1, 4), 0.0, 0.5)).unwrap();
        l.hold = true;
        let mut p = XyPlay::new(1000);
        let mut r = Log::default();
        p.load(&mut r, Some(l.compile().unwrap()));
        assert!(p.hold(), "the layout starts with Hold on");
        let n = source(TARGETS);
        assert!(r.said.contains(&format!("row {n} {}", insert(1, 4))));
        p.begin(&mut r, 1.0, 1.0, |a| a);
        p.end(&mut r, false);
        assert!(p.holds(), "Hold keeps them");
        assert_eq!(r.pushed(n), 0.5);
        p.cancel(&mut r);
        assert!(!p.holds());
        assert_eq!(r.pushed(n), 0.0);
        assert_eq!(
            r.said.last().unwrap(),
            &format!("let go {} 0", insert(1, 3))
        );
        // A push that should be 0 but isn't (lost to a full queue) goes
        // again on the next tick.
        r.push[n as usize] = 0.3;
        p.tick(&mut r, 64);
        assert_eq!(r.pushed(n), 0.0);
        // Another layout: the old one's row goes.
        p.load(&mut r, None);
        assert_eq!(
            r.said.last().unwrap(),
            &format!("unrow {n} {}", insert(1, 4))
        );
        assert!(!p.playing());
    }
}
