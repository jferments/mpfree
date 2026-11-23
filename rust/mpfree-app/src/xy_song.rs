//! Your XY layouts in this song (the XY controller plan's phase 2; layout
//! spec 4o): the list of them, which one the pad plays, Setup's edits, the
//! settings the waiting banner gathers, and the song's `xy` part.
//!
//! The sound loop keeps it (`main.rs`, beside the mixer), because only it
//! can name a setting, read its range and say whether it is still there
//! ([`Look`]). Each [`XyAction`] from the screen changes it and says what
//! the mixer does next ([`Done`]: a layout to play, Hold); the screen sees
//! it again through [`XyView`]. A layout plays through `xy_play.rs`.
//!
//! - **A setting goes on a layout once**: picking it for the other
//!   direction moves it there ([`mpfree_xy::Layout::put`]), and the line
//!   says so.
//! - **New settings** join over their whole range, Even, Set. A switch
//!   (or a meter) is refused, and so is a direction already at 8.
//! - **Add** is refused on a synth's or a pad's own settings until the mod
//!   engine reaches them ([`Seen::own`]).
//! - **A setting that went away** (its effect taken out, its track
//!   removed) stays on the layout, struck out with the reason, under the
//!   name it had ([`Mine::names`]); it is never pointed at whatever took
//!   its place.
//! - **The waiting banner** keeps a copy of the layout from before
//!   (Cancel puts it back exactly) and says what just joined.
//! - **A ready-made layout changed** becomes a new layout of yours ("Wash
//!   2", `xy_layouts::layout`); the ready-made one stays as it was.

use mpfree_mod::MapCurve;
use mpfree_node::{Curve, ParamInfo};
use mpfree_play::mpfree_param::{Owner, ParamAddr};
use mpfree_xy::{Compiled, How, Layout, Lift, Start, Target, GLIDE_MAX_MS, TARGETS};

/// The longest name a layout may have.
pub const NAME_LEN: usize = 24;
/// The longest name a direction may have.
pub const AXIS_LEN: usize = 16;
/// Setup's glide back steps by this much on – and +, the wheel and Q4.
pub const GLIDE_STEP_MS: f32 = 10.0;
/// A new layout's glide back.
pub const NEW_GLIDE_MS: f32 = 120.0;
/// The most layouts of yours in a song.
pub const MAX_LAYOUTS: usize = 32;

/// Setup's curves, as it names them, in the order its list shows them.
pub const CURVES: [(MapCurve, &str); 4] = [
    (MapCurve::Linear, "Even"),
    (MapCurve::Exp, "Slow start"),
    (MapCurve::Log, "Fast start"),
    (MapCurve::S, "S-shaped"),
];

/// A curve's name in Setup.
pub fn curve_name(c: MapCurve) -> &'static str {
    CURVES
        .iter()
        .find(|(k, _)| *k == c)
        .map_or("Even", |(_, n)| n)
}

/// A direction's name in the words of the screen.
fn way(up: bool) -> &'static str {
    if up {
        "up"
    } else {
        "across"
    }
}

/// What the screen asks of your layouts.
#[derive(Clone, Debug, PartialEq)]
pub enum XyAction {
    /// Layout `i` of yours plays (the XY pad's list).
    Play(usize),
    /// "+ New layout": an empty layout of yours, playing (the screen
    /// opens it in Setup).
    New,
    /// Setup (or Let go) on ready-made layout `pick` (None: Filter + echo;
    /// `Some(k)`: XY FX effect `k`): a copy becomes yours and plays. With
    /// `stay`, its Let go is switched to it.
    FromReady {
        pick: Option<usize>,
        stay: Option<bool>,
    },
    /// Takes layout `i` out of the song.
    Delete(usize),
    /// The layout's name and its directions' (typed).
    Names {
        layout: usize,
        name: String,
        across: String,
        up: String,
    },
    /// A setting's range: for Set, the knob's travel at each end (0 to 1);
    /// for Add, the push at each end (-1 to 1).
    Range {
        layout: usize,
        up: bool,
        k: usize,
        from: f32,
        to: f32,
    },
    Curve {
        layout: usize,
        up: bool,
        k: usize,
        curve: MapCurve,
    },
    /// Set (false) or Add (true).
    How {
        layout: usize,
        up: bool,
        k: usize,
        add: bool,
    },
    /// × or Take off: the setting leaves the layout.
    TakeOff { layout: usize, up: bool, k: usize },
    /// When you let go: Go back (false) or Stay.
    Lift { layout: usize, stay: bool },
    /// The glide back, in ms (0 to 2 s).
    Glide { layout: usize, ms: f32 },
    /// A touch jumps (false) or moves from here.
    Start { layout: usize, from_here: bool },
    /// Follow the selected track.
    Follow { layout: usize, on: bool },
    /// + across or + up: the banner waits for settings for that direction.
    Wait { layout: usize, up: bool },
    /// SHIFT + a touch on a value while the banner waits: its setting
    /// joins.
    Join(crate::gui::LearnValue),
    /// The banner's Done (true) or Cancel.
    WaitEnd(bool),
    /// The pad's Hold.
    Hold(bool),
    /// XY → or XY ↑ on the chain editor's Modulate row (layout spec 4b's
    /// XY chips): the setting goes on that direction of the layout playing
    /// (a ready-made one, `pick` as Prefs keeps it, becomes a layout of
    /// yours first), over its whole range, Even, Set; on that direction
    /// already, it comes off.
    Chip {
        addr: ParamAddr,
        up: bool,
        pick: Option<usize>,
    },
}

/// A setting as the sound loop sees it now ([`Look`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    pub info: ParamInfo,
    /// "Cutoff", "Drive", "Send A".
    pub name: String,
    /// Where it is: "Bass · Poly", "Bass strip · Drive, effect 2",
    /// "Master".
    pub place: String,
    pub dot: Dot,
    /// A synth's or a pad's own setting: no Add until the mod engine
    /// reaches it.
    pub own: bool,
}

/// A setting's colour on Setup's row: its track's, or the master's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dot {
    Track(usize),
    Master,
    #[default]
    Other,
}

/// How the sound loop sees a setting: Err with the reason it isn't there
/// ("Track 7 was removed.").
pub type Look<'a> = &'a dyn Fn(ParamAddr) -> Result<Seen, String>;

/// A layout of yours.
#[derive(Clone, Debug, PartialEq)]
pub struct Mine {
    pub layout: Layout,
    /// Setup's glide back, kept with Stay too (Go back's is the layout's
    /// own).
    pub glide_ms: f32,
    /// Each setting's name and place when it was last there: a setting
    /// that went away keeps them, struck out.
    pub names: Vec<(ParamAddr, String, String)>,
    /// A copy of XY FX effect `k`'s layout: the master's XY FX plays `k`
    /// while it plays.
    pub fx: Option<usize>,
}

impl Mine {
    /// An empty layout of yours called `name`.
    pub fn new(name: &str) -> Mine {
        Mine {
            layout: Layout::new(name),
            glide_ms: NEW_GLIDE_MS,
            names: Vec::new(),
            fx: None,
        }
    }

    fn axis(&self, up: bool) -> &[Target] {
        if up {
            &self.layout.up.targets
        } else {
            &self.layout.across.targets
        }
    }

    fn axis_mut(&mut self, up: bool) -> &mut Vec<Target> {
        if up {
            &mut self.layout.up.targets
        } else {
            &mut self.layout.across.targets
        }
    }

    /// Whether it lets go with Stay.
    pub fn stays(&self) -> bool {
        self.layout.lift == Lift::Stay
    }

    fn remember(&mut self, addr: ParamAddr, name: &str, place: &str) {
        match self.names.iter_mut().find(|(a, ..)| *a == addr) {
            Some(n) if n.1 == name && n.2 == place => {}
            Some(n) => (n.1, n.2) = (name.to_string(), place.to_string()),
            None => self.names.push((addr, name.to_string(), place.to_string())),
        }
    }

    fn name_of(&self, addr: ParamAddr) -> (String, String) {
        self.names.iter().find(|(a, ..)| *a == addr).map_or_else(
            || (addr.to_string(), String::new()),
            |(_, n, p)| (n.clone(), p.clone()),
        )
    }
}

/// The banner's wait: settings SHIFT + a touch picks join a direction of
/// a layout until Done or Cancel.
#[derive(Clone, Debug, PartialEq)]
pub struct Wait {
    pub layout: usize,
    pub up: bool,
    /// The layout before, for Cancel.
    before: Mine,
    /// What just happened, for the banner's second line.
    pub just: String,
}

/// What the mixer does after an action.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Done {
    /// A layout to play from now: a layout of yours and, for a copy of an
    /// XY FX's layout, the effect the XY FX plays; None, the ready-made
    /// one picked.
    pub load: Option<Option<(Compiled, Option<usize>)>>,
    /// Hold on or off.
    pub hold: Option<bool>,
}

/// Your layouts in this song, and which plays.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct XySong {
    pub yours: Vec<Mine>,
    /// The layout of yours the pad plays (None: a ready-made one).
    pub playing: Option<usize>,
    pub wait: Option<Wait>,
}

/// Your layouts as the screen shows them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct XyView {
    pub yours: Vec<MineView>,
    /// The one playing.
    pub playing: Option<usize>,
    /// The pad's Hold now.
    pub hold: bool,
    pub wait: Option<WaitView>,
}

/// A layout of yours on the screen: the list's tile and Setup.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MineView {
    pub name: String,
    pub across: String,
    pub up: String,
    /// What it plays: "across: Bite, 2 settings · up: Wet, 2 · Bass +
    /// Master".
    pub line: String,
    /// Its settings, across's first.
    pub rows: Vec<RowView>,
    pub stay: bool,
    pub glide_ms: f32,
    pub from_here: bool,
    pub follow: bool,
    /// Hold starts on.
    pub hold: bool,
    /// Where the pad rests, 0 to 1 each way.
    pub rest: (f32, f32),
}

impl MineView {
    /// The settings on a direction.
    pub fn count(&self, up: bool) -> usize {
        self.rows.iter().filter(|r| r.up == up).count()
    }
}

/// One setting's row in Setup.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RowView {
    pub up: bool,
    /// Its place on its direction.
    pub k: usize,
    pub name: String,
    pub place: String,
    pub dot: Dot,
    /// The ends as [`XyAction::Range`] takes them: travel 0 to 1 for Set,
    /// a push of -1 to 1 for Add.
    pub from: f32,
    pub to: f32,
    pub from_text: String,
    pub to_text: String,
    pub curve: MapCurve,
    pub add: bool,
    /// Add can't be picked: why, for the hint line.
    pub no_add: Option<&'static str>,
    /// It went away: why ("Track 7 was removed.").
    pub gone: Option<String>,
}

impl RowView {
    /// Whether it runs backwards (from above to).
    pub fn backwards(&self) -> bool {
        self.from > self.to
    }
}

/// The banner on the screen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WaitView {
    /// The layout it fills, and the direction.
    pub layout: usize,
    pub up: bool,
    /// "Bite on Squelch waits: hold SHIFT and touch settings on any page".
    pub line: String,
    /// What just happened, faint.
    pub just: String,
}

/// Hold on a ready-made layout, from the screen or a learned button
/// (phase 2): it comes with the XY page.
pub const NO_HOLD: &str =
    "Hold works on a layout of yours for now: Setup makes a copy of this one yours";

/// Why Add is greyed on a synth's or a pad's own setting.
pub const NO_ADD: &str =
    "Add works on the mixer, effects and outside gear for now: a synth's or a pad's own setting moves by Set";

/// What a learned layout button plays (Learn's Play card: XY layout next
/// and back, `step` presses): the layout of yours `step` on from the one
/// playing (the first, or the last going back, when none plays), or,
/// while the song has none, the ready-made one `step` on from `ready`
/// (None: Filter + echo), in the list's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stepped {
    Yours(usize),
    Ready(Option<usize>),
}

impl XySong {
    /// Where `step` presses of a learned layout button go ([`Stepped`]).
    pub fn stepped(&self, ready: Option<usize>, step: i8) -> Stepped {
        let step = i32::from(step);
        let n = self.yours.len() as i32;
        if n > 0 {
            let i = match self.playing {
                Some(i) => i as i32 + step,
                None if step > 0 => step - 1,
                None => n + step,
            };
            return Stepped::Yours(i.rem_euclid(n) as usize);
        }
        let len = crate::play_page::XY_CHOICES.len() as i32;
        let at = ready.map_or(0, |k| k as i32 + 1);
        match (at + step).rem_euclid(len) {
            0 => Stepped::Ready(None),
            k => Stepped::Ready(Some(k as usize - 1)),
        }
    }
}

/// An Add push as Setup writes it.
fn push_text(p: f32) -> String {
    let pc = (p * 100.0).round();
    if pc > 0.0 {
        format!("+{pc:.0} %")
    } else {
        format!("{pc:.0} %")
    }
}

/// Whether the pad can move a setting with this description.
fn movable(info: &ParamInfo) -> Result<(), &'static str> {
    if info.readonly {
        return Err("it is a meter");
    }
    if info.curve == Curve::Toggle {
        return Err("it is a switch, and the pad moves settings with a range");
    }
    Ok(())
}

/// Owners the pad never moves (they are buttons and remote controls).
fn reachable(addr: ParamAddr) -> bool {
    !matches!(
        addr.owner,
        Owner::Transport | Owner::Clips | Owner::Looper | Owner::Perform | Owner::Play
    )
}

/// A setting's own track's, or the master's, colour.
pub fn dot_of(addr: ParamAddr) -> Dot {
    use mpfree_play::mpfree_param::Strip;
    match addr.owner {
        Owner::Strip(Strip::Master) | Owner::Insert(Strip::Master, _) => Dot::Master,
        o => match o.split() {
            Some((t, _)) => Dot::Track(usize::from(t)),
            None => Dot::Other,
        },
    }
}

impl XySong {
    /// Layout `i`, checked and compiled, with the effect its XY FX plays.
    fn compiled(&self, i: usize) -> Result<(Compiled, Option<usize>), String> {
        let m = self.yours.get(i).ok_or("no such layout")?;
        let c = m.layout.compile().map_err(|e| e.to_string())?;
        Ok((c, m.fx))
    }

    /// The layout playing, compiled, when `i` is it (an edit to it plays
    /// from now).
    fn reload(&self, i: usize) -> Option<Option<(Compiled, Option<usize>)>> {
        if self.playing != Some(i) {
            return None;
        }
        self.compiled(i).ok().map(Some)
    }

    /// A name for a new layout that no layout of yours has: `base` and the
    /// first number from `first` that is free ("Layout 1", "Wash 2").
    pub fn free_name(&self, base: &str, first: usize) -> String {
        let taken = |n: &str| self.yours.iter().any(|m| m.layout.name == n);
        (first..)
            .map(|k| format!("{base} {k}"))
            .find(|n| !taken(n))
            .unwrap_or_default()
    }

    fn add(&mut self, m: Mine) -> Result<(usize, Done), String> {
        if self.yours.len() >= MAX_LAYOUTS {
            return Err(format!(
                "This song has {MAX_LAYOUTS} layouts of yours: delete one first"
            ));
        }
        self.yours.push(m);
        let i = self.yours.len() - 1;
        let (c, fx) = self.compiled(i)?;
        self.playing = Some(i);
        Ok((
            i,
            Done {
                load: Some(Some((c, fx))),
                hold: None,
            },
        ))
    }

    /// The screen's action. `track` is the armed track (a SHIFT + touch on
    /// the Synth tab or a pad names its settings), `ready` the copy a
    /// ready-made layout would make (`xy_layouts::layout`, on its insert;
    /// None when its effect isn't on the master yet). What the mixer does
    /// next, or the line that says why nothing changed. `tell` gets what
    /// the hint line says.
    pub fn act(
        &mut self,
        a: XyAction,
        track: u8,
        ready: impl FnOnce(Option<usize>, &str) -> Option<Mine>,
        look: Look<'_>,
        tell: &mut dyn FnMut(String),
    ) -> Result<Done, String> {
        let edit = |song: &XySong, i: usize| -> Result<(), String> {
            if i < song.yours.len() {
                Ok(())
            } else {
                Err("That layout isn't in this song any more".to_string())
            }
        };
        let done = |song: &XySong, i: usize| Done {
            load: song.reload(i),
            hold: None,
        };
        match a {
            XyAction::Play(i) => {
                edit(self, i)?;
                let (c, fx) = self.compiled(i)?;
                self.playing = Some(i);
                tell(format!("XY pad: {}", self.yours[i].layout.name));
                Ok(Done {
                    load: Some(Some((c, fx))),
                    hold: None,
                })
            }
            XyAction::New => {
                let name = self.free_name("Layout", 1);
                let (_, d) = self.add(Mine::new(&name))?;
                tell(format!(
                    "{name}: a new layout of yours. + across, + up: then SHIFT + touch settings"
                ));
                Ok(d)
            }
            XyAction::FromReady { pick, stay } => {
                let base = crate::play_page::XY_CHOICES[pick.map_or(0, |k| k + 1)];
                let name = self.free_name(base, 2);
                let Some(mut m) = ready(pick, &name) else {
                    return Err(format!(
                        "Play {base} on the pad once first: its effect isn't on the master yet"
                    ));
                };
                if let Some(stay) = stay {
                    set_lift(&mut m, stay);
                }
                let (_, d) = self.add(m)?;
                tell(format!(
                    "{name}: a layout of yours made from {base}, which stays as it was"
                ));
                Ok(d)
            }
            XyAction::Delete(i) => {
                edit(self, i)?;
                let m = self.yours.remove(i);
                let mut d = Done::default();
                match self.playing {
                    Some(p) if p == i => {
                        self.playing = None;
                        d.load = Some(None);
                    }
                    Some(p) if p > i => self.playing = Some(p - 1),
                    _ => {}
                }
                match &mut self.wait {
                    Some(w) if w.layout == i => self.wait = None,
                    Some(w) if w.layout > i => w.layout -= 1,
                    _ => {}
                }
                tell(format!("{}: deleted from this song", m.layout.name));
                Ok(d)
            }
            XyAction::Names {
                layout,
                name,
                across,
                up,
            } => {
                edit(self, layout)?;
                let fit = |s: &str, n: usize, old: &str| {
                    let s: String = s.trim().chars().take(n).collect();
                    if s.is_empty() {
                        old.to_string()
                    } else {
                        s
                    }
                };
                let l = &mut self.yours[layout].layout;
                l.name = fit(&name, NAME_LEN, &l.name);
                l.across.name = fit(&across, AXIS_LEN, &l.across.name);
                l.up.name = fit(&up, AXIS_LEN, &l.up.name);
                Ok(Done::default())
            }
            XyAction::Range {
                layout,
                up,
                k,
                from,
                to,
            } => {
                edit(self, layout)?;
                let t = *self.yours[layout]
                    .axis(up)
                    .get(k)
                    .ok_or("no such setting")?;
                let (from, to) = match t.how {
                    How::Add => (from.clamp(-1.0, 1.0), to.clamp(-1.0, 1.0)),
                    How::Set => {
                        let seen = look(t.addr)?;
                        let at = |p: f32| seen.info.clamp(seen.info.from_unit(p));
                        (at(from), at(to))
                    }
                };
                let t = &mut self.yours[layout].axis_mut(up)[k];
                (t.from, t.to) = (from, to);
                Ok(done(self, layout))
            }
            XyAction::Curve {
                layout,
                up,
                k,
                curve,
            } => {
                edit(self, layout)?;
                let t = self.yours[layout]
                    .axis_mut(up)
                    .get_mut(k)
                    .ok_or("no such setting")?;
                t.curve = curve;
                Ok(done(self, layout))
            }
            XyAction::How { layout, up, k, add } => {
                edit(self, layout)?;
                let t = *self.yours[layout]
                    .axis(up)
                    .get(k)
                    .ok_or("no such setting")?;
                if add == (t.how == How::Add) {
                    return Ok(Done::default());
                }
                let seen = look(t.addr)?;
                if add && seen.own {
                    return Err(NO_ADD.to_string());
                }
                // Set's stretch becomes a push as long (a whole range: up
                // by the whole travel); Add's becomes the whole range.
                let (from, to, how) = if add {
                    let a = seen.info.to_unit(t.from);
                    let b = seen.info.to_unit(t.to);
                    (0.0, (b - a).clamp(-1.0, 1.0), How::Add)
                } else {
                    (seen.info.from_unit(0.0), seen.info.from_unit(1.0), How::Set)
                };
                let t = &mut self.yours[layout].axis_mut(up)[k];
                (t.from, t.to, t.how) = (from, to, how);
                Ok(done(self, layout))
            }
            XyAction::TakeOff { layout, up, k } => {
                edit(self, layout)?;
                let axis = self.yours[layout].axis_mut(up);
                if k >= axis.len() {
                    return Err("no such setting".to_string());
                }
                axis.remove(k);
                Ok(done(self, layout))
            }
            XyAction::Lift { layout, stay } => {
                edit(self, layout)?;
                set_lift(&mut self.yours[layout], stay);
                Ok(done(self, layout))
            }
            XyAction::Glide { layout, ms } => {
                edit(self, layout)?;
                let m = &mut self.yours[layout];
                m.glide_ms = ms.clamp(0.0, GLIDE_MAX_MS);
                if !m.stays() {
                    m.layout.lift = Lift::GoBack {
                        snap_ms: m.glide_ms,
                    };
                }
                Ok(done(self, layout))
            }
            XyAction::Start { layout, from_here } => {
                edit(self, layout)?;
                self.yours[layout].layout.start = if from_here {
                    Start::FromHere
                } else {
                    Start::Jump
                };
                Ok(done(self, layout))
            }
            XyAction::Follow { layout, on } => {
                edit(self, layout)?;
                self.yours[layout].layout.follow = on;
                Ok(done(self, layout))
            }
            XyAction::Wait { layout, up } => {
                edit(self, layout)?;
                // Another wait replaces one waiting, which keeps what
                // joined.
                let m = &self.yours[layout];
                if m.axis(up).len() >= TARGETS {
                    return Err(format!(
                        "{} has {TARGETS} settings: take one off first",
                        if up {
                            &m.layout.up.name
                        } else {
                            &m.layout.across.name
                        }
                    ));
                }
                self.wait = Some(Wait {
                    layout,
                    up,
                    before: m.clone(),
                    just: format!(
                        "{} on {} · Done keeps them, Cancel puts {} back",
                        m.axis(up).len(),
                        way(up),
                        m.layout.name
                    ),
                });
                Ok(Done::default())
            }
            XyAction::Join(v) => {
                let addr = crate::desk::learn_addr(v, track);
                let r = self.join(addr, look, tell);
                // The banner covers the hint line: its own line says why.
                if let (Err(why), Some(w)) = (&r, self.wait.as_mut()) {
                    w.just = why.clone();
                }
                r
            }
            XyAction::WaitEnd(keep) => {
                let Some(w) = self.wait.take() else {
                    return Ok(Done::default());
                };
                if keep || w.layout >= self.yours.len() {
                    return Ok(Done::default());
                }
                let name = w.before.layout.name.clone();
                self.yours[w.layout] = w.before;
                tell(format!("{name}: put back as it was"));
                Ok(done(self, w.layout))
            }
            XyAction::Hold(on) => Ok(Done {
                load: None,
                hold: Some(on),
            }),
            XyAction::Chip { addr, up, pick } => self.chip(addr, up, pick, ready, look, tell),
        }
    }

    /// XY → (`up` false) or XY ↑ on `addr`: see [`XyAction::Chip`].
    fn chip(
        &mut self,
        addr: ParamAddr,
        up: bool,
        pick: Option<usize>,
        ready: impl FnOnce(Option<usize>, &str) -> Option<Mine>,
        look: Look<'_>,
        tell: &mut dyn FnMut(String),
    ) -> Result<Done, String> {
        let chip = if up { "XY ↑" } else { "XY →" };
        if !reachable(addr) {
            return Err("That is a button: the pad moves settings with a range".to_string());
        }
        let seen = look(addr)?;
        let who = format!("{} ({})", seen.name, seen.place);
        if let Err(why) = movable(&seen.info) {
            return Err(format!("{who} can't go on the pad: {why}"));
        }
        // A ready-made layout playing: a copy of it becomes yours.
        let (i, made) = match self.playing.filter(|&i| i < self.yours.len()) {
            Some(i) => (i, String::new()),
            None => {
                let base = crate::play_page::XY_CHOICES[pick.map_or(0, |k| k + 1)];
                let name = self.free_name(base, 2);
                let Some(m) = ready(pick, &name) else {
                    return Err(format!(
                        "Play {base} on the pad once first: its effect isn't on the master yet"
                    ));
                };
                let (i, _) = self.add(m)?;
                (i, format!(" (yours, made from {base})"))
            }
        };
        let m = &mut self.yours[i];
        let name = m.layout.name.clone();
        let was = m.layout.way_of(addr);
        if was == Some(up) {
            m.layout.take_off(addr);
            tell(format!("{who} is off {name}{made}"));
        } else {
            let t = Target::linear(addr, seen.info.from_unit(0.0), seen.info.from_unit(1.0));
            m.layout.put(up, t).map_err(|_| {
                format!(
                    "{} has {TARGETS} settings: take one off first",
                    if up { "Up" } else { "Across" }
                )
            })?;
            m.remember(addr, &seen.name, &seen.place);
            let moved = if was.is_some() {
                format!(", moved from {}", way(!up))
            } else {
                String::new()
            };
            tell(format!(
                "{who} moves {} on {name}{made}{moved}, {} to {} · {chip} again: take it off · \
                 its range and curve: Setup on the XY page",
                way(up),
                mpfree_mod::text::value_text(&seen.info, t.from),
                mpfree_mod::text::value_text(&seen.info, t.to)
            ));
        }
        Ok(Done {
            load: self.reload(i),
            hold: None,
        })
    }

    /// A SHIFT + touch on `addr` while the banner waits: it joins the
    /// direction waiting, over its whole range, Even, Set.
    fn join(
        &mut self,
        addr: ParamAddr,
        look: Look<'_>,
        tell: &mut dyn FnMut(String),
    ) -> Result<Done, String> {
        let Some(w) = self.wait.as_ref() else {
            return Ok(Done::default());
        };
        let (i, up) = (w.layout, w.up);
        if i >= self.yours.len() {
            self.wait = None;
            return Ok(Done::default());
        }
        if !reachable(addr) {
            return Err("That is a button: the pad moves settings with a range".to_string());
        }
        let seen = look(addr)?;
        let who = format!("{} ({})", seen.name, seen.place);
        if let Err(why) = movable(&seen.info) {
            return Err(format!("{who} can't go on the pad: {why}"));
        }
        let m = &mut self.yours[i];
        let was = m.layout.way_of(addr);
        if was == Some(up) {
            return Err(format!("{who} is on {} already", way(up)));
        }
        let t = Target::linear(addr, seen.info.from_unit(0.0), seen.info.from_unit(1.0));
        m.layout.put(up, t).map_err(|_| {
            format!(
                "{} has {TARGETS} settings: take one off first",
                if up { "Up" } else { "Across" }
            )
        })?;
        m.remember(addr, &seen.name, &seen.place);
        let n = m.axis(up).len();
        let moved = if was.is_some() {
            format!(", moved from {}", way(!up))
        } else {
            String::new()
        };
        let line = format!(
            "{who} moves {}{moved}, {} to {}",
            way(up),
            mpfree_mod::text::value_text(&seen.info, t.from),
            mpfree_mod::text::value_text(&seen.info, t.to)
        );
        let name = m.layout.name.clone();
        if let Some(w) = self.wait.as_mut() {
            w.just = format!(
                "Just added: {line} · {n} on {} · Done keeps them, Cancel puts {name} back",
                way(up)
            );
        }
        tell(line);
        Ok(Done {
            load: self.reload(i),
            hold: None,
        })
    }

    /// The screen's view, `hold` the pad's Hold now; each setting there is
    /// named afresh (and remembered for when it goes away).
    pub fn view(&mut self, hold: bool, look: Look<'_>) -> XyView {
        let yours = self.yours.iter_mut().map(|m| mine_view(m, look)).collect();
        let wait = self.wait.as_ref().and_then(|w| {
            let m = self.yours.get(w.layout)?;
            let dir = if w.up {
                &m.layout.up.name
            } else {
                &m.layout.across.name
            };
            Some(WaitView {
                layout: w.layout,
                up: w.up,
                line: format!(
                    "{dir} on {} waits: hold SHIFT and touch settings on any page",
                    m.layout.name
                ),
                just: w.just.clone(),
            })
        });
        XyView {
            yours,
            playing: self.playing,
            hold,
            wait,
        }
    }
}

/// Let go: Go back (over the layout's glide) or Stay.
fn set_lift(m: &mut Mine, stay: bool) {
    m.layout.lift = if stay {
        Lift::Stay
    } else {
        Lift::GoBack {
            snap_ms: m.glide_ms,
        }
    };
}

/// A layout of yours as the screen shows it.
fn mine_view(m: &mut Mine, look: Look<'_>) -> MineView {
    let mut rows = Vec::new();
    let mut places: Vec<String> = Vec::new();
    for up in [false, true] {
        let targets: Vec<Target> = m.axis(up).to_vec();
        for (k, t) in targets.iter().enumerate() {
            let row = match look(t.addr) {
                Ok(seen) => {
                    m.remember(t.addr, &seen.name, &seen.place);
                    let top = seen.place.split(" · ").next().unwrap_or("").to_string();
                    let top = top.trim_end_matches(" strip").to_string();
                    if !top.is_empty() && !places.contains(&top) {
                        places.push(top);
                    }
                    let (from, to, from_text, to_text) = match t.how {
                        How::Set => (
                            seen.info.to_unit(t.from),
                            seen.info.to_unit(t.to),
                            mpfree_mod::text::value_text(&seen.info, t.from),
                            mpfree_mod::text::value_text(&seen.info, t.to),
                        ),
                        How::Add => (t.from, t.to, push_text(t.from), push_text(t.to)),
                    };
                    RowView {
                        up,
                        k,
                        name: seen.name,
                        place: seen.place,
                        dot: seen.dot,
                        from,
                        to,
                        from_text,
                        to_text,
                        curve: t.curve,
                        add: t.how == How::Add,
                        no_add: seen.own.then_some(NO_ADD),
                        gone: None,
                    }
                }
                Err(why) => {
                    let (name, place) = m.name_of(t.addr);
                    RowView {
                        up,
                        k,
                        name,
                        place,
                        dot: Dot::Other,
                        curve: t.curve,
                        add: t.how == How::Add,
                        gone: Some(why),
                        ..RowView::default()
                    }
                }
            };
            rows.push(row);
        }
    }
    let l = &m.layout;
    let (na, nu) = (l.across.targets.len(), l.up.targets.len());
    let mut parts = Vec::new();
    if l.follow {
        parts.push("follows the selected track".to_string());
    }
    if na + nu == 0 {
        parts.push("empty: add settings in Setup".to_string());
    } else {
        let s = |n: usize| if n == 1 { "setting" } else { "settings" };
        parts.push(format!("across: {}, {na} {}", l.across.name, s(na)));
        parts.push(format!("up: {}, {nu}", l.up.name));
        if !places.is_empty() {
            parts.push(places.join(" + "));
        }
    }
    if l.hold {
        parts.push("Hold".to_string());
    }
    MineView {
        name: l.name.clone(),
        across: l.across.name.clone(),
        up: l.up.name.clone(),
        line: parts.join(" · "),
        rows,
        stay: m.stays(),
        glide_ms: m.glide_ms,
        from_here: l.start == Start::FromHere,
        follow: l.follow,
        hold: l.hold,
        rest: l.rest,
    }
}

// ------------------------------------------------------------ Stay's UNDO

/// What a Stay lift kept, as UNDO keys it: the settings, in their places.
pub type XyKey = [Option<ParamAddr>; mpfree_xy::SLOTS];
/// Their own values before or after (NaN where there is none).
pub type XyValues = [f32; mpfree_xy::SLOTS];
/// UNDO's history on the XY pad: each Stay lift, one step over all the
/// settings it kept.
pub type XyTurns = crate::undo::Turns<XyKey, XyValues>;

/// One Stay lift's turns (`MixHost::take_xy_kept`) as one UNDO step: the
/// settings, their values before, and after. None when it kept nothing.
pub fn kept_turn(
    kept: impl Iterator<Item = (ParamAddr, f32, f32)>,
) -> Option<(XyKey, XyValues, XyValues)> {
    let mut key: XyKey = [None; mpfree_xy::SLOTS];
    let mut from = [f32::NAN; mpfree_xy::SLOTS];
    let mut to = [f32::NAN; mpfree_xy::SLOTS];
    let mut n = 0;
    for (addr, before, after) in kept.take(mpfree_xy::SLOTS) {
        key[n] = Some(addr);
        from[n] = before;
        to[n] = after;
        n += 1;
    }
    (n > 0).then_some((key, from, to))
}

/// What UNDO puts back: each setting and its value (`redo`: the values
/// after).
pub fn turn_values(
    t: &crate::undo::Turn<XyKey, XyValues>,
    redo: bool,
) -> impl Iterator<Item = (ParamAddr, f32)> + '_ {
    let v = if redo { t.to } else { t.from };
    t.key
        .iter()
        .zip(v)
        .filter_map(|(a, v)| Some(((*a)?, v)).filter(|(_, v)| v.is_finite()))
}

/// Two layouts of yours for the sample screens and the manual's pictures
/// (`gui::sample_screens`): Squelch plays (Bite across: the bass track's
/// filter's cutoff and resonance, its distortion's drive pushed; Wet up:
/// its send and a removed track's echo mix, struck out), then Wash 2. With `wait`, the
/// banner waits for settings across.
pub fn sample_view(wait: bool) -> XyView {
    let row =
        |up, k, name: &str, place: &str, dot, ends: (f32, f32), texts: (&str, &str)| RowView {
            up,
            k,
            name: name.into(),
            place: place.into(),
            dot,
            from: ends.0,
            to: ends.1,
            from_text: texts.0.into(),
            to_text: texts.1.into(),
            ..RowView::default()
        };
    let mut rows = vec![
        row(
            false,
            0,
            "Cutoff",
            "Bass · Filter, effect 1",
            Dot::Track(1),
            (0.25, 0.9),
            ("400 Hz", "9.2 kHz"),
        ),
        row(
            false,
            1,
            "Resonance",
            "Bass · Filter, effect 1",
            Dot::Track(1),
            (0.1, 0.65),
            ("10 %", "65 %"),
        ),
        row(
            false,
            2,
            "Drive",
            "Bass · Distortion, effect 2",
            Dot::Track(1),
            (0.0, 0.5),
            ("+0", "+0.50"),
        ),
        row(
            true,
            0,
            "Send A",
            "Bass strip",
            Dot::Track(1),
            (0.0, 0.7),
            ("-inf dB", "-3.0 dB"),
        ),
        row(
            true,
            1,
            "Mix",
            "Track 7 · Echo, effect 1",
            Dot::Other,
            (0.0, 1.0),
            ("", ""),
        ),
    ];
    rows[0].curve = MapCurve::Exp;
    rows[2].add = true;
    rows[4].gone = Some("Track 7 was removed.".into());
    let squelch = MineView {
        name: "Squelch".into(),
        across: "Bite".into(),
        up: "Wet".into(),
        line: "across: Bite, 3 settings · up: Wet, 2 · Bass + Track 7".into(),
        rows,
        stay: false,
        glide_ms: 250.0,
        rest: (0.5, 0.5),
        ..MineView::default()
    };
    let wash = MineView {
        name: "Wash 2".into(),
        across: "Decay".into(),
        up: "Amount".into(),
        line: "across: Decay, 1 setting · up: Amount, 1 · Master".into(),
        stay: true,
        glide_ms: NEW_GLIDE_MS,
        rest: (0.5, 0.0),
        ..MineView::default()
    };
    XyView {
        yours: vec![squelch, wash],
        playing: Some(0),
        hold: false,
        wait: wait.then(|| WaitView {
            layout: 0,
            up: false,
            line: "Bite on Squelch waits: hold SHIFT and touch settings on any page".into(),
            just: "Just added: Drive (Bass · Distortion, effect 2) moves across, 0 to 100 % · 3 on across · Done keeps them, Cancel puts Squelch back"
                .into(),
        }),
    }
}

// ------------------------------------------------------------ the song's part

/// The `xy` part's first line.
const HEAD: &str = "xy 1";

fn curve_word(c: MapCurve) -> &'static str {
    match c {
        MapCurve::Linear => "linear",
        MapCurve::Exp => "exp",
        MapCurve::Log => "log",
        MapCurve::S => "s",
    }
}

fn word_curve(w: &str) -> Option<MapCurve> {
    Some(match w {
        "linear" => MapCurve::Linear,
        "exp" => MapCurve::Exp,
        "log" => MapCurve::Log,
        "s" => MapCurve::S,
        _ => return None,
    })
}

impl XySong {
    /// The song's `xy` part: each layout of yours, then which plays. Empty
    /// when there are none (the part isn't saved).
    pub fn text(&self) -> String {
        use std::fmt::Write as _;
        if self.yours.is_empty() {
            return String::new();
        }
        let mut s = format!("{HEAD}\n");
        for m in &self.yours {
            let l = &m.layout;
            let _ = writeln!(s, "layout\t{}", l.name);
            let _ = writeln!(s, "across\t{}", l.across.name);
            let _ = writeln!(s, "up\t{}", l.up.name);
            let lift = if m.stays() { "stay" } else { "back" };
            let _ = writeln!(s, "lift\t{lift}\t{}", m.glide_ms);
            let start = match l.start {
                Start::Jump => "jump",
                Start::FromHere => "here",
            };
            let _ = writeln!(
                s,
                "pad\t{start}\t{}\t{}\t{}\t{}",
                u8::from(l.hold),
                u8::from(l.follow),
                l.rest.0,
                l.rest.1
            );
            if let Some(g) = l.gate {
                let _ = writeln!(s, "gate\t{g}");
            }
            if let Some(k) = m.fx {
                let _ = writeln!(s, "fx\t{k}");
            }
            for (up, axis) in [(false, &l.across), (true, &l.up)] {
                for t in &axis.targets {
                    let how = match t.how {
                        How::Set => "set",
                        How::Add => "add",
                    };
                    let _ = writeln!(
                        s,
                        "{}\t{}\t{how}\t{}\t{}\t{}",
                        way(up),
                        t.addr,
                        curve_word(t.curve),
                        t.from,
                        t.to
                    );
                }
            }
            for (a, n, p) in &m.names {
                if l.way_of(*a).is_some() {
                    let _ = writeln!(s, "name\t{a}\t{n}\t{p}");
                }
            }
            let _ = writeln!(s, "end");
        }
        if let Some(p) = self.playing {
            let _ = writeln!(s, "playing\t{p}");
        }
        s
    }

    /// The part read back, with a line for each thing that didn't read (a
    /// layout that doesn't check is left out, and said).
    pub fn parse(text: &str) -> (XySong, Vec<String>) {
        let mut song = XySong::default();
        let mut notes = Vec::new();
        let mut lines = text.lines();
        match lines.next() {
            None => return (song, notes),
            Some(h) if h.trim() == HEAD => {}
            Some(h) => {
                notes.push(format!("xy: not a part this app reads ({h})"));
                return (song, notes);
            }
        }
        let mut cur: Option<Mine> = None;
        let mut playing = None;
        for (n, line) in lines.enumerate() {
            let f: Vec<&str> = line.split('\t').collect();
            let bad = |notes: &mut Vec<String>| {
                notes.push(format!("xy: line {} didn't read: {line}", n + 2));
            };
            match (f[0], cur.as_mut()) {
                ("layout", None) if f.len() == 2 => cur = Some(Mine::new(f[1])),
                ("across", Some(m)) if f.len() == 2 => m.layout.across.name = f[1].to_string(),
                ("up", Some(m)) if f.len() == 2 => m.layout.up.name = f[1].to_string(),
                ("lift", Some(m)) if f.len() == 3 => match f[2].parse::<f32>() {
                    Ok(ms) if ms.is_finite() => {
                        m.glide_ms = ms.clamp(0.0, GLIDE_MAX_MS);
                        set_lift(m, f[1] == "stay");
                    }
                    _ => bad(&mut notes),
                },
                ("pad", Some(m)) if f.len() == 6 => {
                    let rest = (f[4].parse::<f32>(), f[5].parse::<f32>());
                    match rest {
                        (Ok(x), Ok(y)) => {
                            let l = &mut m.layout;
                            l.start = if f[1] == "here" {
                                Start::FromHere
                            } else {
                                Start::Jump
                            };
                            l.hold = f[2] == "1";
                            l.follow = f[3] == "1";
                            l.rest = (x, y);
                        }
                        _ => bad(&mut notes),
                    }
                }
                ("gate", Some(m)) if f.len() == 2 => match f[1].parse::<ParamAddr>() {
                    Ok(a) => m.layout.gate = Some(a),
                    Err(_) => bad(&mut notes),
                },
                ("fx", Some(m)) if f.len() == 2 => match f[1].parse::<usize>() {
                    Ok(k) if k < mpfree_fx::xyfx::EFFECTS.len() => m.fx = Some(k),
                    _ => bad(&mut notes),
                },
                ("across" | "up", Some(m)) if f.len() == 6 => {
                    let target = (|| {
                        Some(Target {
                            addr: f[1].parse().ok()?,
                            how: match f[2] {
                                "set" => How::Set,
                                "add" => How::Add,
                                _ => return None,
                            },
                            curve: word_curve(f[3])?,
                            from: f[4].parse().ok()?,
                            to: f[5].parse().ok()?,
                        })
                    })();
                    match target {
                        Some(t) if m.layout.way_of(t.addr).is_none() => {
                            let axis = m.axis_mut(f[0] == "up");
                            if axis.len() < TARGETS {
                                axis.push(t);
                            } else {
                                bad(&mut notes);
                            }
                        }
                        _ => bad(&mut notes),
                    }
                }
                ("name", Some(m)) if f.len() == 4 => match f[1].parse::<ParamAddr>() {
                    Ok(a) => m.remember(a, f[2], f[3]),
                    Err(_) => bad(&mut notes),
                },
                ("end", Some(_)) => {
                    let m = cur.take().expect("matched Some");
                    match m.layout.compile() {
                        Ok(_) if song.yours.len() < MAX_LAYOUTS => song.yours.push(m),
                        Ok(_) => notes.push(format!(
                            "xy: {} left out: a song keeps {MAX_LAYOUTS} layouts of yours",
                            m.layout.name
                        )),
                        Err(e) => notes.push(format!("xy: {} left out: {e}", m.layout.name)),
                    }
                }
                ("playing", None) if f.len() == 2 => playing = f[1].parse::<usize>().ok(),
                ("", _) => {}
                _ => bad(&mut notes),
            }
        }
        if let Some(m) = cur {
            notes.push(format!(
                "xy: {} left out: the part ends inside it",
                m.layout.name
            ));
        }
        song.playing = playing.filter(|&p| p < song.yours.len());
        (song, notes)
    }

    /// The layout to play after the song opens: the one it was playing.
    pub fn opening(&self) -> Option<(Compiled, Option<usize>)> {
        self.compiled(self.playing?).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_node::Unit;
    use mpfree_play::mpfree_param::Strip;

    fn info(min: f32, max: f32, curve: Curve) -> ParamInfo {
        ParamInfo {
            id: 0,
            name: "X",
            group: "",
            min,
            max,
            default: min,
            unit: Unit::Percent,
            curve,
            readonly: false,
        }
    }

    const CUTOFF: ParamAddr = ParamAddr::new(Owner::Inst(1), 5);
    const SEND: ParamAddr = ParamAddr::new(Owner::Strip(Strip::Track(1)), 4);
    const MIX: ParamAddr = ParamAddr::new(Owner::Insert(Strip::Master, 3), 0);
    const BYPASS: ParamAddr = ParamAddr::new(Owner::Insert(Strip::Master, 3), 9);
    const GONE: ParamAddr = ParamAddr::new(Owner::Insert(Strip::Master, 8), 0);

    fn look(addr: ParamAddr) -> Result<Seen, String> {
        let seen = |name: &str, place: &str, i: ParamInfo, own| {
            Ok(Seen {
                info: i,
                name: name.into(),
                place: place.into(),
                dot: dot_of(addr),
                own,
            })
        };
        match addr {
            CUTOFF => seen(
                "Cutoff",
                "Bass · Poly",
                info(0.0, 100.0, Curve::Linear),
                true,
            ),
            SEND => seen(
                "Send A",
                "Bass strip",
                info(0.0, 100.0, Curve::Linear),
                false,
            ),
            MIX => seen(
                "Mix",
                "Master · Room, effect 1",
                info(0.0, 100.0, Curve::Linear),
                false,
            ),
            BYPASS => seen(
                "Bypass",
                "Master · Room, effect 1",
                info(0.0, 1.0, Curve::Toggle),
                false,
            ),
            _ => Err("Its effect was taken out.".to_string()),
        }
    }

    fn act(song: &mut XySong, a: XyAction) -> Result<Done, String> {
        song.act(a, 1, |_, _| None, &look, &mut |_| {})
    }

    fn join(song: &mut XySong, addr: ParamAddr) -> Result<Done, String> {
        act(song, XyAction::Join(crate::gui::LearnValue::Param(addr)))
    }

    /// XY → and XY ↑ on the Modulate row: on a ready-made layout a copy
    /// becomes yours with the setting on it; the other way moves it, the
    /// same way again takes it off, and a switch can't go on.
    #[test]
    fn the_modulate_rows_xy_chips_put_a_setting_on_the_pad_and_take_it_off() {
        let mut song = XySong::default();
        let mut told = Vec::new();
        let chip = |addr, up| XyAction::Chip {
            addr,
            up,
            pick: None,
        };
        let d = song
            .act(
                chip(MIX, true),
                1,
                |pick, name| {
                    assert_eq!(pick, None, "Filter + echo");
                    Some(Mine::new(name))
                },
                &look,
                &mut |l| told.push(l),
            )
            .unwrap();
        assert!(matches!(d.load, Some(Some(_))), "it plays at once: {d:?}");
        assert_eq!(song.playing, Some(0));
        assert_eq!(song.yours[0].layout.name, "Filter + echo 2");
        assert_eq!(song.yours[0].layout.way_of(MIX), Some(true));
        let t = song.yours[0].layout.up.targets[0];
        assert_eq!(
            (t.from, t.to, t.how),
            (0.0, 100.0, How::Set),
            "its whole range"
        );
        assert!(
            told[0].starts_with(
                "Mix (Master · Room, effect 1) moves up on Filter + echo 2 \
                 (yours, made from Filter + echo), "
            ) && told[0].contains("XY ↑ again: take it off"),
            "{told:?}"
        );
        // On a layout of yours, across moves it there.
        act(&mut song, chip(MIX, false)).unwrap();
        assert_eq!(song.yours.len(), 1, "the layout playing takes it");
        assert_eq!(song.yours[0].layout.way_of(MIX), Some(false));
        // XY → again: off.
        let d = act(&mut song, chip(MIX, false)).unwrap();
        assert!(matches!(d.load, Some(Some(_))), "{d:?}");
        assert_eq!(song.yours[0].layout.way_of(MIX), None);
        let e = act(&mut song, chip(BYPASS, true)).unwrap_err();
        assert!(e.contains("can't go on the pad: it is a switch"), "{e}");
        assert_eq!(song.yours[0].layout.way_of(BYPASS), None);
    }

    /// "+ New layout" plays an empty layout; the banner's settings join one
    /// direction, move from the other, and refuse a switch or a ninth;
    /// Cancel puts the layout back exactly.
    #[test]
    fn the_banner_gathers_settings_and_cancel_puts_the_layout_back() {
        let mut song = XySong::default();
        let d = act(&mut song, XyAction::New).unwrap();
        assert!(matches!(d.load, Some(Some(_))), "{d:?}");
        assert_eq!(song.playing, Some(0));
        assert_eq!(song.yours[0].layout.name, "Layout 1");
        act(
            &mut song,
            XyAction::Wait {
                layout: 0,
                up: false,
            },
        )
        .unwrap();
        let d = join(&mut song, MIX).unwrap();
        assert!(
            matches!(d.load, Some(Some(_))),
            "the playing layout plays it at once"
        );
        join(&mut song, SEND).unwrap();
        let v = song.view(false, &look);
        assert_eq!(v.yours[0].count(false), 2);
        let w = v.wait.unwrap();
        assert!(w.line.starts_with("Across on Layout 1 waits"), "{}", w.line);
        assert!(
            w.just.contains("Send A (Bass strip) moves across"),
            "{}",
            w.just
        );
        assert!(join(&mut song, BYPASS).unwrap_err().contains("switch"));
        assert!(join(&mut song, MIX).unwrap_err().contains("already"));
        act(&mut song, XyAction::WaitEnd(true)).unwrap();
        // Up: Mix moves there from across.
        let before = song.yours[0].clone();
        act(
            &mut song,
            XyAction::Wait {
                layout: 0,
                up: true,
            },
        )
        .unwrap();
        join(&mut song, MIX).unwrap();
        assert_eq!(song.yours[0].layout.way_of(MIX), Some(true));
        assert!(song
            .wait
            .as_ref()
            .unwrap()
            .just
            .contains("moved from across"));
        act(&mut song, XyAction::WaitEnd(false)).unwrap();
        assert_eq!(song.yours[0], before, "Cancel puts it back exactly");
        assert_eq!(song.wait, None);
    }

    /// Setup's edits: a range from the knob's travel, Add refused on a
    /// synth's own setting, a setting that went away struck out under its
    /// old name, Stay and the glide.
    #[test]
    fn setup_edits_a_layout_and_a_setting_that_went_away_keeps_its_name() {
        let mut song = XySong::default();
        act(&mut song, XyAction::New).unwrap();
        act(
            &mut song,
            XyAction::Wait {
                layout: 0,
                up: false,
            },
        )
        .unwrap();
        join(&mut song, CUTOFF).unwrap();
        join(&mut song, MIX).unwrap();
        act(&mut song, XyAction::WaitEnd(true)).unwrap();
        act(
            &mut song,
            XyAction::Range {
                layout: 0,
                up: false,
                k: 0,
                from: 0.2,
                to: 0.85,
            },
        )
        .unwrap();
        let t = song.yours[0].layout.across.targets[0];
        assert!((t.from - 20.0).abs() < 1e-3 && (t.to - 85.0).abs() < 1e-3);
        let add = |k| XyAction::How {
            layout: 0,
            up: false,
            k,
            add: true,
        };
        assert_eq!(act(&mut song, add(0)).unwrap_err(), NO_ADD);
        act(&mut song, add(1)).unwrap();
        let t = song.yours[0].layout.across.targets[1];
        assert_eq!((t.how, t.from, t.to), (How::Add, 0.0, 1.0));
        act(
            &mut song,
            XyAction::Lift {
                layout: 0,
                stay: true,
            },
        )
        .unwrap();
        act(
            &mut song,
            XyAction::Glide {
                layout: 0,
                ms: 5000.0,
            },
        )
        .unwrap();
        assert_eq!(song.yours[0].glide_ms, GLIDE_MAX_MS);
        act(
            &mut song,
            XyAction::Lift {
                layout: 0,
                stay: false,
            },
        )
        .unwrap();
        assert_eq!(
            song.yours[0].layout.lift,
            Lift::GoBack {
                snap_ms: GLIDE_MAX_MS
            }
        );
        let v = song.view(false, &look);
        let rows = &v.yours[0].rows;
        assert_eq!(rows[0].name, "Cutoff");
        assert_eq!(rows[0].no_add, Some(NO_ADD));
        assert_eq!(
            (rows[1].from_text.as_str(), rows[1].to_text.as_str()),
            ("0 %", "+100 %")
        );
        assert!(
            v.yours[0].line.contains("Bass + Master"),
            "{}",
            v.yours[0].line
        );
        // The effect goes: struck out, named as it was.
        song.yours[0]
            .layout
            .put(true, Target::linear(GONE, 0.0, 1.0))
            .unwrap();
        song.yours[0].remember(GONE, "Size", "Master · Room, effect 2");
        let v = song.view(false, &look);
        let gone = v.yours[0].rows.iter().find(|r| r.gone.is_some()).unwrap();
        assert_eq!(
            (gone.name.as_str(), gone.gone.as_deref()),
            ("Size", Some("Its effect was taken out."))
        );
    }

    /// The part keeps each layout as it was and which one plays; one that
    /// doesn't check is left out and said.
    #[test]
    fn the_song_keeps_your_layouts_and_which_plays() {
        let mut song = XySong::default();
        act(&mut song, XyAction::New).unwrap();
        act(
            &mut song,
            XyAction::Wait {
                layout: 0,
                up: true,
            },
        )
        .unwrap();
        join(&mut song, SEND).unwrap();
        act(&mut song, XyAction::WaitEnd(true)).unwrap();
        act(
            &mut song,
            XyAction::Names {
                layout: 0,
                name: "Squelch".into(),
                across: "Bite".into(),
                up: "Wet".into(),
            },
        )
        .unwrap();
        act(
            &mut song,
            XyAction::Follow {
                layout: 0,
                on: true,
            },
        )
        .unwrap();
        act(
            &mut song,
            XyAction::Start {
                layout: 0,
                from_here: true,
            },
        )
        .unwrap();
        act(&mut song, XyAction::New).unwrap();
        act(&mut song, XyAction::Play(0)).unwrap();
        let text = song.text();
        let (back, notes) = XySong::parse(&text);
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(
            back,
            XySong {
                wait: None,
                ..song.clone()
            }
        );
        assert!(back.opening().is_some());
        // A bad line is said; a layout that doesn't check is left out.
        let broken = text.replace("lift\tback\t120", "lift\tback\tfast");
        let (_, notes) = XySong::parse(&broken);
        assert!(notes[0].contains("didn't read"), "{notes:?}");
        let twice = text.replacen("end\n", &format!("up\t{SEND}\tset\tlinear\t0\t1\nend\n"), 1);
        let (b, notes) = XySong::parse(&twice);
        assert_eq!(
            b.yours.len(),
            2,
            "the second line is refused, the layout kept"
        );
        assert!(!notes.is_empty());
        assert_eq!(XySong::default().text(), "");
        assert_eq!(XySong::parse(""), (XySong::default(), vec![]));
    }

    /// Deleting the layout playing plays the ready-made one; the indices
    /// after it move down.
    #[test]
    fn deleting_a_layout_keeps_the_others_in_place() {
        let mut song = XySong::default();
        for _ in 0..3 {
            act(&mut song, XyAction::New).unwrap();
        }
        assert_eq!(song.playing, Some(2));
        let d = act(&mut song, XyAction::Delete(0)).unwrap();
        assert_eq!(d.load, None);
        assert_eq!(song.playing, Some(1));
        let d = act(&mut song, XyAction::Delete(1)).unwrap();
        assert_eq!(d.load, Some(None));
        assert_eq!(song.playing, None);
        assert!(act(&mut song, XyAction::Play(4)).is_err());
        let names: Vec<_> = song.yours.iter().map(|m| m.layout.name.clone()).collect();
        assert_eq!(names, ["Layout 2"]);
    }
}
