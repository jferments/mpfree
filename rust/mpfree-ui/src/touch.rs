//! What a control says about touch (layout spec section 1a), and helpers
//! that make the spec's rules the easy way to write a widget.
//!
//! Each [`Target`](crate::ui::Target) declares, besides where it answers a
//! finger:
//!
//! - its **class** ([`Class`]): how big its hit area must be, in physical
//!   size (the MPC One's screen has [`PX_PER_MM`] px to the millimetre);
//! - its **context** ([`Context`]), **consequence** ([`Consequence`]) and
//!   **timing** ([`Timing`]): together they call for a class
//!   ([`called_for`]), and the class a control needs is the highest of the
//!   one declared and the one they call for, except that a row of one kind
//!   may be [`Class::Dense`] and a precise control [`Class::Precision`]
//!   (each with an alternative) where they call for no more than Normal;
//! - how it **commits** ([`Commit`]) and how it **moves** ([`Movement`]);
//! - the **object** it acts on, by a stable name, so a finger owns that
//!   object rather than a place on the screen, and a test can check the
//!   command reached it;
//! - where it is **drawn**, when its hit area is bigger than its drawing.
//!
//! Every field has a default, so a target that says nothing is a Normal,
//! lift-inside control in Make with a minor consequence: a control allowed
//! to be smaller has to say so in code, where a reviewer sees it (the
//! touch design set, tab 1). Nothing here changes what a widget does; the
//! touch audit measures the declarations against the screens and against
//! what each handler really does.
//!
//! The helpers: [`Fingers`] keeps touch state per finger (put on the press,
//! read on a move, taken on the lift or a cancel), [`Press`] is what a
//! lift-inside button needs to call itself off on a slide off, and
//! [`Thresholds`] gathers the distances a finger may move per gesture.

use crate::geom::Rect;

/// Pixels to the millimetre on the MPC One's screen (1280 x 800 on a
/// 7-inch panel: 150.8 x 94.2 mm).
pub const PX_PER_MM: f32 = 8.49;

/// How big a control's hit area must be: the short side of the area, or a
/// continuous control's travel (layout spec 1a, "Sizes").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    /// Notes, markers, automation points, wave handles, EQ dots: no
    /// minimum, the largest grab zone that overlaps nothing, and always an
    /// alternative (the wheel's outline, – / +, a Q-Link, the number pad).
    Precision,
    /// An exception for a row of one kind edited as a set (steps, 16
    /// Levels, keys, Make's clip rows, list rows): 44 px. Never lowers a
    /// control whose context or timing calls for Performance or Critical.
    Dense,
    /// Settings, rare options, chips in a page's head; a minor
    /// consequence: 48 px.
    Secondary,
    /// Navigation, dialog buttons, browser rows, number and text keys:
    /// 64 px. The default.
    #[default]
    Normal,
    /// Used while playing in Perform: MUTE, SOLO, clip cells, scene launch,
    /// throws, loop rings: 76 px.
    Performance,
    /// Time-sensitive or costly: record, loop record, Stop all: 85 px.
    Critical,
}

impl Class {
    /// Every class, smallest first.
    pub const ALL: [Class; 6] = [
        Class::Precision,
        Class::Dense,
        Class::Secondary,
        Class::Normal,
        Class::Performance,
        Class::Critical,
    ];

    /// The short side a hit area needs, in pixels (the Standard profile).
    pub const fn min_px(self) -> i32 {
        match self {
            Class::Precision => 0,
            Class::Dense => 44,
            Class::Secondary => 48,
            Class::Normal => 64,
            Class::Performance => 76,
            Class::Critical => 85,
        }
    }

    /// The smallest preferred short side, in pixels: under it the touch
    /// audit warns but doesn't fail.
    pub const fn preferred_px(self) -> i32 {
        match self {
            Class::Precision => 0,
            Class::Dense => 48,
            Class::Secondary => 64,
            Class::Normal => 76,
            Class::Performance => 85,
            Class::Critical => 104,
        }
    }

    /// Precision and Dense: allowed under what a control's context,
    /// consequence and timing call for, with an alternative.
    pub const fn is_exception(self) -> bool {
        matches!(self, Class::Precision | Class::Dense)
    }

    /// Its name, as the audit's baseline writes it.
    pub const fn name(self) -> &'static str {
        match self {
            Class::Precision => "precision",
            Class::Dense => "dense",
            Class::Secondary => "secondary",
            Class::Normal => "normal",
            Class::Performance => "performance",
            Class::Critical => "critical",
        }
    }
}

/// Where a control is played. `Perform` and `Both` mean it is played as
/// part of the music while performing (a clip cell, MUTE, a throw); a
/// control that navigates or sets something up is `Make`, whichever mode
/// shows it (a tab, a dialog button).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Context {
    #[default]
    Make,
    Perform,
    Both,
}

/// What a wrong touch on it costs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Consequence {
    /// A setting, a view: put back with one more touch.
    #[default]
    Minor,
    /// Changes what plays or what is kept, and UNDO or a second touch puts
    /// it back (a mute, a launch, an edit).
    Disruptive,
    /// Loses material (Clear, Delete): a hold and space around it, or a
    /// named confirmation where nothing brings it back.
    Destructive,
}

/// When its action lands in the music.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Timing {
    /// Not musical: a view, a setting, navigation.
    #[default]
    None,
    /// On a line the engine sets (the next 1/16, the launch line).
    Quantized,
    /// At once, and when is the musician's: record, Stop all.
    Immediate,
}

/// How a control commits (layout spec 1a, "How a control acts").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Commit {
    /// It acts as the finger lands, where timing matters (the screen's
    /// pads, the transport, the throws, Perform's clip cells).
    Press,
    /// It acts when the finger lifts while still on it, so a slide off
    /// calls it off. The default for a tap.
    LiftInside,
    /// A guarded action: it acts once the finger has been held long enough.
    Hold,
}

/// How a control moves under a finger.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Movement {
    None,
    /// Follows the finger from where it lands (faders, pans, sends, knobs,
    /// Q-Link cards): landing moves nothing and a tap changes nothing.
    Relative,
    /// Where the finger is, is the value (the XY pad).
    Absolute,
    /// Moves a thing (a clip, a row, a block, an effect, a loop): off in
    /// Perform.
    Structural,
}

/// The class a control's context, consequence and timing call for: the
/// highest of what each calls for. Played in Perform: Performance;
/// changing what plays, losing material or landing in the music: Normal;
/// anything else: Secondary. So a row of one kind in Make (steps, Make's
/// clip rows) may still be Dense, as the spec lists, and nothing played in
/// Perform may. Critical is never called for on its own: the controls the
/// spec names (record, loop record, Stop all) declare it.
pub fn called_for(context: Context, consequence: Consequence, timing: Timing) -> Class {
    let by_context = match context {
        Context::Make => Class::Secondary,
        Context::Perform | Context::Both => Class::Performance,
    };
    let by_consequence = match consequence {
        Consequence::Minor => Class::Secondary,
        Consequence::Disruptive | Consequence::Destructive => Class::Normal,
    };
    let by_timing = match timing {
        Timing::None => Class::Secondary,
        Timing::Quantized | Timing::Immediate => Class::Normal,
    };
    by_context.max(by_consequence).max(by_timing)
}

/// The class a control needs: the one it declares, raised to what its
/// context, consequence and timing call for, except that an exception
/// class (Precision, Dense) stands where they call for no more than
/// Normal.
pub fn needed(declared: Class, called: Class) -> Class {
    if declared.is_exception() && called <= Class::Normal {
        declared
    } else {
        declared.max(called)
    }
}

/// How far a finger may move per gesture before it counts as something
/// else, in pixels (layout spec 1a, "Thresholds per gesture"). Touch
/// assistance, a profile setting still to come, may raise the first three
/// and never adds a dead zone to faders or the XY pad.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thresholds {
    /// A tap that moved less than this is still a tap.
    pub tap: i32,
    /// A structural move picks its thing up past this (1.4 mm, section 4h).
    pub pickup: i32,
    /// A hold stays a hold while the finger moves less than this.
    pub hold_slop: i32,
    /// A list or a grid starts scrolling past this.
    pub scroll: i32,
    /// Two fingers pinch once their distance changes by this.
    pub pinch: i32,
}

impl Thresholds {
    /// The Standard profile's.
    pub const STANDARD: Thresholds = Thresholds {
        tap: 12,
        pickup: 12,
        hold_slop: 12,
        scroll: 12,
        pinch: 16,
    };
}

impl Default for Thresholds {
    fn default() -> Thresholds {
        Thresholds::STANDARD
    }
}

/// Touch state kept per finger: put on the press, read on a move, taken on
/// the lift or a cancel. A widget that keeps something between a press and
/// a lift keeps it here, so a second finger never takes over the first's
/// (the clip grid's bug of 25 Nov 2026).
#[derive(Clone, Debug, PartialEq)]
pub struct Fingers<T> {
    held: Vec<(u64, T)>,
}

impl<T> Default for Fingers<T> {
    fn default() -> Fingers<T> {
        Fingers { held: Vec::new() }
    }
}

impl<T> Fingers<T> {
    pub fn new() -> Fingers<T> {
        Fingers::default()
    }

    /// `finger` holds `t` now (a finger that went down again replaces what
    /// it held).
    pub fn put(&mut self, finger: u64, t: T) {
        self.held.retain(|(f, _)| *f != finger);
        self.held.push((finger, t));
    }

    /// What `finger` holds.
    pub fn get(&self, finger: u64) -> Option<&T> {
        self.held.iter().find(|(f, _)| *f == finger).map(|(_, t)| t)
    }

    pub fn get_mut(&mut self, finger: u64) -> Option<&mut T> {
        self.held
            .iter_mut()
            .find(|(f, _)| *f == finger)
            .map(|(_, t)| t)
    }

    /// What `finger` held, let go of (its lift or a cancel).
    pub fn take(&mut self, finger: u64) -> Option<T> {
        let i = self.held.iter().position(|(f, _)| *f == finger)?;
        Some(self.held.remove(i).1)
    }

    /// Whether `finger` holds something.
    pub fn holds(&self, finger: u64) -> bool {
        self.held.iter().any(|(f, _)| *f == finger)
    }

    /// Whether any finger holds something `pred` likes (a second finger on
    /// a control already held is ignored).
    pub fn any(&self, pred: impl Fn(&T) -> bool) -> bool {
        self.held.iter().any(|(_, t)| pred(t))
    }

    /// Every finger and what it holds, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = (u64, &T)> {
        self.held.iter().map(|(f, t)| (*f, t))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (u64, &mut T)> {
        self.held.iter_mut().map(|(f, t)| (*f, t))
    }

    /// Let go of every finger.
    pub fn clear(&mut self) {
        self.held.clear();
    }

    /// Let go of every finger, with what each held.
    pub fn drain(&mut self) -> Vec<(u64, T)> {
        std::mem::take(&mut self.held)
    }

    /// Keep only what `keep` likes (a finger whose object left the screen).
    pub fn retain(&mut self, mut keep: impl FnMut(u64, &T) -> bool) {
        self.held.retain(|(f, t)| keep(*f, t));
    }

    pub fn len(&self) -> usize {
        self.held.len()
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }
}

/// A finger down on a lift-inside control: where it landed and the
/// control's area. Its lift acts only while the finger is still on the
/// control ([`Press::lifted_inside`]), so a slide off calls it off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Press {
    pub finger: u64,
    /// The control's hit area.
    pub rect: Rect,
    /// Where the finger landed.
    pub at: (i32, i32),
}

impl Press {
    pub fn new(finger: u64, rect: Rect, x: i32, y: i32) -> Press {
        Press {
            finger,
            rect,
            at: (x, y),
        }
    }

    /// Whether a lift at (`x`, `y`) commits: still on the control.
    pub fn lifted_inside(&self, x: i32, y: i32) -> bool {
        self.rect.contains(x, y)
    }

    /// Whether the finger has moved more than `px` from where it landed,
    /// either way (a tap that became a drag).
    pub fn moved_past(&self, x: i32, y: i32, px: i32) -> bool {
        (x - self.at.0).abs().max((y - self.at.1).abs()) > px
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The classes' sizes are the spec's, in millimetres on the MPC One.
    #[test]
    fn the_classes_are_the_specs_sizes() {
        let mm = |px: i32| px as f32 / PX_PER_MM;
        assert_eq!(Class::Dense.min_px(), 44);
        assert!((mm(Class::Dense.min_px()) - 5.2).abs() < 0.05);
        assert!((mm(Class::Normal.min_px()) - 7.5).abs() < 0.05);
        assert!((mm(Class::Critical.min_px()) - 10.0).abs() < 0.05);
        for w in Class::ALL.windows(2) {
            assert!(w[0] < w[1] && w[0].min_px() <= w[1].min_px());
        }
        for c in Class::ALL {
            assert!(c.preferred_px() >= c.min_px(), "{c:?}");
        }
        assert_eq!(Class::default(), Class::Normal);
    }

    /// The class needed is the highest called for; Dense and Precision
    /// stand only where no more than Normal is called for.
    #[test]
    fn the_class_needed_is_the_highest_called_for() {
        use Consequence as C;
        let quiet = called_for(Context::Make, C::Minor, Timing::None);
        assert_eq!(quiet, Class::Secondary);
        assert_eq!(needed(Class::Normal, quiet), Class::Normal);
        assert_eq!(needed(Class::Dense, quiet), Class::Dense);
        let edit = called_for(Context::Make, C::Disruptive, Timing::None);
        assert_eq!(edit, Class::Normal);
        assert_eq!(needed(Class::Dense, edit), Class::Dense, "a row of steps");
        let mute = called_for(Context::Perform, C::Disruptive, Timing::Quantized);
        assert_eq!(mute, Class::Performance);
        assert_eq!(needed(Class::Dense, mute), Class::Performance);
        assert_eq!(needed(Class::Normal, mute), Class::Performance);
        let make_clip = called_for(Context::Make, C::Disruptive, Timing::Quantized);
        assert_eq!(make_clip, Class::Normal);
        assert_eq!(
            needed(Class::Dense, make_clip),
            Class::Dense,
            "Make's clip rows"
        );
        let perform_clip = called_for(Context::Perform, C::Disruptive, Timing::Quantized);
        assert_eq!(needed(Class::Dense, perform_clip), Class::Performance);
        let stop_all = called_for(Context::Both, C::Disruptive, Timing::Immediate);
        assert_eq!(
            needed(Class::Critical, stop_all),
            Class::Critical,
            "declared"
        );
        assert_eq!(
            called_for(Context::Make, C::Destructive, Timing::None),
            Class::Normal
        );
    }

    /// Each finger keeps its own: a second finger never takes over the
    /// first's, and a lift or a cancel takes only its own.
    #[test]
    fn each_finger_keeps_its_own() {
        let mut f: Fingers<&str> = Fingers::new();
        f.put(1, "clip 1");
        f.put(2, "clip 2");
        assert_eq!(f.get(1), Some(&"clip 1"));
        assert!(f.any(|t| *t == "clip 2"));
        if let Some(t) = f.get_mut(2) {
            *t = "clip 3";
        }
        assert_eq!(f.take(1), Some("clip 1"));
        assert_eq!(f.take(1), None, "taken once");
        assert!(f.holds(2) && !f.holds(1));
        f.put(2, "again");
        assert_eq!(f.len(), 1, "a finger down again replaces what it held");
        f.retain(|_, t| *t != "again");
        assert!(f.is_empty());
    }

    /// A press commits only lifted on its control; it knows when it moved.
    #[test]
    fn a_press_commits_only_lifted_inside() {
        let p = Press::new(7, Rect::new(100, 100, 80, 80), 140, 140);
        assert!(p.lifted_inside(179, 101));
        assert!(!p.lifted_inside(181, 140), "slid off: called off");
        assert!(!p.moved_past(150, 130, Thresholds::STANDARD.tap));
        assert!(p.moved_past(160, 140, Thresholds::STANDARD.tap));
    }
}
