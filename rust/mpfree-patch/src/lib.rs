//! Patches: what the keys play (the live shows plan's stage C1,
//! [design](../../../docs/dev/design/live-shows.md)).
//!
//! A **patch** is a named keyboard setup kept in the song: up to
//! [`MAX_ZONES`] zones, each a key range and a velocity range on one
//! keyboard (or any) playing one of the song's instruments, with a
//! transpose, a velocity curve, and whether the pedals and the wheels
//! reach it. Two zones over the same keys layer. A key no zone takes
//! follows the patch's [`Outside`] setting. A song with no patches plays
//! its keys as before.
//!
//! - [`Patches`]: the song's list as the project keeps it (the `patches`
//!   part, [`Patches::to_text`] and [`Patches::parse`]), on the screen
//!   thread. Zones name their instrument and keyboard in the app's words
//!   ("track 3", "KeyLab 61"); [`Patches::resolve`] turns them into a
//!   [`Rig`] for the audio thread.
//! - [`keys::Keys`]: the audio thread's side. It remembers every key held
//!   (the keyboard, channel and note it came from, and each instrument and
//!   note it went to) and every pedal down (the instruments it reached),
//!   so a key ends where it started and a pedalled chord rings on through
//!   a switch. Nothing in it allocates after [`keys::Keys::new`].
//!
//! Plain data, no threads, no I/O.
#![forbid(unsafe_code)]

pub mod keys;

/// The most zones one patch has.
pub const MAX_ZONES: usize = 8;

/// The most patches a song keeps (program change 1 to 128 picks them).
pub const MAX_PATCHES: usize = 128;

/// The longest name a patch takes, in characters (the text keyboard's).
pub const MAX_NAME: usize = 32;

/// The furthest a zone transposes either way, in semitones.
pub const MAX_TRANSPOSE: i8 = 48;

/// The first line of the `patches` part.
pub const HEAD: &str = "mpfree-patches 1";

/// How a zone plays the velocities it takes (the curves of the routes'
/// Velocity block, `mpfree_midi::blocks`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Curve {
    /// As played.
    #[default]
    AsPlayed,
    /// Softer playing comes out louder.
    Soft,
    /// It takes a harder hit to sound loud.
    Hard,
    /// Every note at this velocity (1 to 127).
    Fixed(u8),
}

impl Curve {
    /// Velocity `v` (1 to 127) as the zone plays it, 1 to 127.
    pub fn apply(self, v: u8) -> u8 {
        let v = v.clamp(1, 127);
        let x = f32::from(v) / 127.0;
        let y = match self {
            Curve::AsPlayed => return v,
            Curve::Fixed(f) => return f.clamp(1, 127),
            Curve::Soft => x.sqrt(),
            Curve::Hard => x * x,
        };
        (y * 127.0).round().clamp(1.0, 127.0) as u8
    }

    fn to_text(self) -> String {
        match self {
            Curve::AsPlayed => "as played".into(),
            Curve::Soft => "soft".into(),
            Curve::Hard => "hard".into(),
            Curve::Fixed(v) => format!("fixed {v}"),
        }
    }

    fn parse(s: &str) -> Option<Curve> {
        match s {
            "as played" => Some(Curve::AsPlayed),
            "soft" => Some(Curve::Soft),
            "hard" => Some(Curve::Hard),
            _ => {
                let v: u8 = s.strip_prefix("fixed ")?.trim().parse().ok()?;
                (1..=127).contains(&v).then_some(Curve::Fixed(v))
            }
        }
    }
}

/// Which keyboard a zone takes, in the rig's words.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Keyboard {
    /// Every keyboard.
    #[default]
    Any,
    /// One port by its name, on one channel (0 to 15), or on every
    /// channel (None: an MPE keyboard, whose notes each have their own).
    Port { name: String, ch: Option<u8> },
}

impl Keyboard {
    fn to_text(&self) -> String {
        match self {
            Keyboard::Any => "any".into(),
            Keyboard::Port { name, ch: Some(c) } => format!("{name} / {}", c + 1),
            Keyboard::Port { name, ch: None } => format!("{name} / all"),
        }
    }

    fn parse(s: &str) -> Option<Keyboard> {
        if s == "any" {
            return Some(Keyboard::Any);
        }
        let (name, ch) = s.rsplit_once(" / ")?;
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let ch = match ch.trim() {
            "all" => None,
            c => {
                let c: u8 = c.parse().ok()?;
                if !(1..=16).contains(&c) {
                    return None;
                }
                Some(c - 1)
            }
        };
        Some(Keyboard::Port {
            name: name.into(),
            ch,
        })
    }
}

/// One zone of a patch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Zone {
    /// The instrument it plays, as the app names it ("track 3", "free 2",
    /// "kit 5"). [`Patches::resolve`] asks the app what it is now.
    pub plays: String,
    pub keyboard: Keyboard,
    /// The lowest and highest key it takes (0 to 127, 48 is C3).
    pub low: u8,
    pub high: u8,
    /// The softest and hardest velocity it takes (1 to 127).
    pub vel_low: u8,
    pub vel_high: u8,
    /// Semitones its notes move ([`MAX_TRANSPOSE`] either way); a note
    /// moved off the keyboard is dropped.
    pub transpose: i8,
    pub curve: Curve,
    /// The sustain, sostenuto and soft pedals reach its instrument.
    pub pedals: bool,
    /// The pitch wheel, the mod wheel, breath, foot, aftertouch and MPE
    /// reach its instrument.
    pub wheels: bool,
    /// Lines this build doesn't know, kept with the zone.
    pub extra: Vec<String>,
}

impl Zone {
    /// A zone over `low` to `high` playing `plays`, every velocity, from
    /// any keyboard, pedals and wheels on.
    pub fn new(plays: &str, low: u8, high: u8) -> Zone {
        Zone {
            plays: plays.into(),
            keyboard: Keyboard::Any,
            low: low.min(high),
            high: high.max(low).min(127),
            vel_low: 1,
            vel_high: 127,
            transpose: 0,
            curve: Curve::AsPlayed,
            pedals: true,
            wheels: true,
            extra: Vec::new(),
        }
    }
}

/// What a key no zone takes plays.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Outside {
    /// Nothing: the gap between a bass and a piano stays quiet (the
    /// default for a patch with zones).
    #[default]
    Nothing,
    /// This instrument, as the app names it.
    Play(String),
    /// What the keys played before patches: the armed track and the
    /// project's routes.
    Routed,
}

/// A patch: a named keyboard setup.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Patch {
    pub name: String,
    /// Up to [`MAX_ZONES`], in the order the editor lists them.
    pub zones: Vec<Zone>,
    pub outside: Outside,
    /// When this patch comes in, the notes and pedals held from the patch
    /// before end at once, rather than ringing on until let go.
    pub cut_old: bool,
    /// Lines this build doesn't know, kept with the patch.
    pub extra: Vec<String>,
}

/// The layouts a new patch starts from (the patch list's New patch).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// One zone over every key.
    Whole,
    /// A bass under C3 (without the pedals) and keys from C3 up.
    BassAndKeys,
    /// Two zones over every key.
    Layered,
    /// Three zones: under C3, C3 to B4, C5 up.
    ThreeWay,
}

impl Layout {
    pub const ALL: [Layout; 4] = [
        Layout::Whole,
        Layout::BassAndKeys,
        Layout::Layered,
        Layout::ThreeWay,
    ];

    /// The words on its tile.
    pub fn name(self) -> &'static str {
        match self {
            Layout::Whole => "Whole keyboard",
            Layout::BassAndKeys => "Bass + keys",
            Layout::Layered => "Two layered",
            Layout::ThreeWay => "Three-way split",
        }
    }

    /// A patch named `name` laid out so, each zone playing `plays` (the
    /// instrument the keys play now) until it is picked.
    pub fn patch(self, name: &str, plays: &str) -> Patch {
        let zones = match self {
            Layout::Whole => vec![Zone::new(plays, 0, 127)],
            Layout::BassAndKeys => {
                let mut bass = Zone::new(plays, 0, 47);
                bass.pedals = false;
                vec![bass, Zone::new(plays, 48, 127)]
            }
            Layout::Layered => vec![Zone::new(plays, 0, 127), Zone::new(plays, 0, 127)],
            Layout::ThreeWay => vec![
                Zone::new(plays, 0, 47),
                Zone::new(plays, 48, 71),
                Zone::new(plays, 72, 127),
            ],
        };
        Patch {
            name: clip_name(name),
            zones,
            ..Patch::default()
        }
    }
}

fn clip_name(s: &str) -> String {
    s.trim().chars().take(MAX_NAME).collect()
}

/// The song's patches, in order: patch 1 is `list[0]`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Patches {
    pub list: Vec<Patch>,
}

/// Why a `patches` part didn't read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// Its first line isn't [`HEAD`] (a newer or another format).
    Head(String),
    /// A line this build should know but can't read: its number (from 1)
    /// and the line.
    Line(usize, String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Head(h) => write!(f, "not a patches part ({h:?})"),
            ParseError::Line(n, l) => write!(f, "line {n} can't be read: {l:?}"),
        }
    }
}

impl Patches {
    /// The part's text: [`HEAD`], then for each patch a `patch = name`
    /// line and its settings, each zone's `zone = plays` line followed by
    /// its own. Lines a later build added are kept where they were.
    pub fn to_text(&self) -> String {
        let mut s = String::from(HEAD);
        s.push('\n');
        for p in &self.list {
            s.push_str(&format!("patch = {}\n", p.name));
            match &p.outside {
                Outside::Nothing => s.push_str("outside = nothing\n"),
                Outside::Play(i) => s.push_str(&format!("outside = play {i}\n")),
                Outside::Routed => s.push_str("outside = routed\n"),
            }
            s.push_str(&format!(
                "cut-old = {}\n",
                if p.cut_old { "yes" } else { "no" }
            ));
            for l in &p.extra {
                s.push_str(l);
                s.push('\n');
            }
            for z in &p.zones {
                s.push_str(&format!("zone = {}\n", z.plays));
                s.push_str(&format!("keyboard = {}\n", z.keyboard.to_text()));
                s.push_str(&format!("keys = {} {}\n", z.low, z.high));
                s.push_str(&format!("velocities = {} {}\n", z.vel_low, z.vel_high));
                s.push_str(&format!("transpose = {}\n", z.transpose));
                s.push_str(&format!("velocity = {}\n", z.curve.to_text()));
                s.push_str(&format!(
                    "pedals = {}\n",
                    if z.pedals { "yes" } else { "no" }
                ));
                s.push_str(&format!(
                    "wheels = {}\n",
                    if z.wheels { "yes" } else { "no" }
                ));
                for l in &z.extra {
                    s.push_str(l);
                    s.push('\n');
                }
            }
        }
        s
    }

    /// Reads [`Patches::to_text`] back. Lines it doesn't know are kept
    /// with the patch or zone they follow; patches past [`MAX_PATCHES`]
    /// and zones past [`MAX_ZONES`] are refused, never dropped.
    pub fn parse(text: &str) -> Result<Patches, ParseError> {
        let mut lines = text.lines();
        let head = lines.next().unwrap_or("").trim();
        if head != HEAD {
            return Err(ParseError::Head(head.into()));
        }
        let mut out = Patches::default();
        for (i, raw) in lines.enumerate() {
            let n = i + 2;
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            let bad = || ParseError::Line(n, raw.to_string());
            let Some((k, v)) = line.split_once(" = ") else {
                keep(&mut out, raw).ok_or_else(bad)?;
                continue;
            };
            let v = v.trim();
            if k == "patch" {
                if out.list.len() >= MAX_PATCHES {
                    return Err(bad());
                }
                out.list.push(Patch {
                    name: clip_name(v),
                    ..Patch::default()
                });
                continue;
            }
            let Some(p) = out.list.last_mut() else {
                return Err(bad());
            };
            match k {
                "outside" => {
                    p.outside = match v {
                        "nothing" => Outside::Nothing,
                        "routed" => Outside::Routed,
                        _ => Outside::Play(
                            v.strip_prefix("play ")
                                .filter(|i| !i.trim().is_empty())
                                .ok_or_else(bad)?
                                .trim()
                                .into(),
                        ),
                    }
                }
                "cut-old" => p.cut_old = yes_no(v).ok_or_else(bad)?,
                "zone" => {
                    if p.zones.len() >= MAX_ZONES || v.is_empty() {
                        return Err(bad());
                    }
                    p.zones.push(Zone::new(v, 0, 127));
                }
                _ => {
                    let Some(z) = p.zones.last_mut() else {
                        p.extra.push(raw.to_string());
                        continue;
                    };
                    match k {
                        "keyboard" => z.keyboard = Keyboard::parse(v).ok_or_else(bad)?,
                        "keys" => {
                            let (lo, hi) = two(v, 0, 127).ok_or_else(bad)?;
                            (z.low, z.high) = (lo, hi);
                        }
                        "velocities" => {
                            let (lo, hi) = two(v, 1, 127).ok_or_else(bad)?;
                            (z.vel_low, z.vel_high) = (lo, hi);
                        }
                        "transpose" => {
                            let t: i8 = v.parse().map_err(|_| bad())?;
                            if t.unsigned_abs() > MAX_TRANSPOSE.unsigned_abs() {
                                return Err(bad());
                            }
                            z.transpose = t;
                        }
                        "velocity" => z.curve = Curve::parse(v).ok_or_else(bad)?,
                        "pedals" => z.pedals = yes_no(v).ok_or_else(bad)?,
                        "wheels" => z.wheels = yes_no(v).ok_or_else(bad)?,
                        _ => z.extra.push(raw.to_string()),
                    }
                }
            }
        }
        Ok(out)
    }

    /// The audio thread's copy: each zone's instrument as `inst` finds it
    /// now (None: not in the song, so its keys play nothing), and the
    /// keyboards the zones name, by name ([`Rig::keyboards`]: the audio
    /// thread finds them among the ports plugged in, [`keys::Keys::set_port`]).
    /// Allocates: the screen thread's job.
    pub fn resolve(&self, mut inst: impl FnMut(&str) -> Option<keys::Inst>) -> Box<Rig> {
        let mut rig = Box::new(Rig::default());
        let mut keyboards: Vec<String> = Vec::new();
        for (slot, p) in rig.patches.iter_mut().zip(self.list.iter()) {
            let mut zones = [None; MAX_ZONES];
            for (to, z) in zones.iter_mut().zip(p.zones.iter()) {
                let takes = match &z.keyboard {
                    Keyboard::Any => Takes::Any,
                    Keyboard::Port { name, ch } => {
                        let kb = match keyboards.iter().position(|k| k == name) {
                            Some(i) => i,
                            None => {
                                keyboards.push(name.clone());
                                keyboards.len() - 1
                            }
                        };
                        // Past 254 keyboards named (more than any rig
                        // has), a zone names one no port is found as, so
                        // it takes nothing.
                        let kb = u8::try_from(kb).unwrap_or(keys::NO_KEYBOARD);
                        Takes::Port { kb, ch: *ch }
                    }
                };
                *to = Some(LiveZone {
                    inst: inst(&z.plays),
                    takes,
                    low: z.low.min(z.high),
                    high: z.high.max(z.low),
                    vel_low: z.vel_low.min(z.vel_high).max(1),
                    vel_high: z.vel_high.max(z.vel_low),
                    transpose: z.transpose.clamp(-MAX_TRANSPOSE, MAX_TRANSPOSE),
                    curve: z.curve,
                    pedals: z.pedals,
                    wheels: z.wheels,
                });
            }
            *slot = Some(LivePatch {
                zones,
                outside: match &p.outside {
                    Outside::Nothing => LiveOutside::Nothing,
                    Outside::Play(i) => LiveOutside::Play(inst(i)),
                    Outside::Routed => LiveOutside::Routed,
                },
                cut_old: p.cut_old,
            });
        }
        rig.keyboards = keyboards;
        rig
    }
}

fn yes_no(v: &str) -> Option<bool> {
    match v {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

fn two(v: &str, min: u8, max: u8) -> Option<(u8, u8)> {
    let mut w = v.split_whitespace();
    let a: u8 = w.next()?.parse().ok()?;
    let b: u8 = w.next()?.parse().ok()?;
    if w.next().is_some() || a > b || a < min || b > max {
        return None;
    }
    Some((a, b))
}

/// Keeps a line with no ` = ` where it stands: with the last zone, else
/// the last patch. None before any patch.
fn keep(out: &mut Patches, raw: &str) -> Option<()> {
    let p = out.list.last_mut()?;
    match p.zones.last_mut() {
        Some(z) => z.extra.push(raw.to_string()),
        None => p.extra.push(raw.to_string()),
    }
    Some(())
}

/// Which keyboard a resolved zone takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Takes {
    Any,
    /// One keyboard, by its place in [`Rig::keyboards`], on one channel
    /// or (None) every channel. It takes nothing while that keyboard
    /// isn't plugged in.
    Port {
        kb: u8,
        ch: Option<u8>,
    },
}

impl Takes {
    /// Whether it takes a message on channel `ch` from the keyboard `kb`
    /// ([`Rig::keyboards`]; None: a keyboard no zone names).
    pub fn takes(self, kb: Option<u8>, ch: u8) -> bool {
        match self {
            Takes::Any => true,
            Takes::Port { kb: k, ch: c } => kb == Some(k) && c.is_none_or(|c| c == ch),
        }
    }
}

/// A zone as the audio thread plays it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveZone {
    /// None: its instrument isn't in the song; it takes its keys and plays
    /// nothing (the editor shows it in red).
    pub inst: Option<keys::Inst>,
    pub takes: Takes,
    pub low: u8,
    pub high: u8,
    pub vel_low: u8,
    pub vel_high: u8,
    pub transpose: i8,
    pub curve: Curve,
    pub pedals: bool,
    pub wheels: bool,
}

impl LiveZone {
    /// Whether it takes `note` at `vel` on channel `ch` from the keyboard
    /// `kb` ([`Takes::takes`]).
    pub fn takes(&self, kb: Option<u8>, ch: u8, note: u8, vel: u8) -> bool {
        self.takes.takes(kb, ch)
            && (self.low..=self.high).contains(&note)
            && (self.vel_low..=self.vel_high).contains(&vel)
    }
}

/// [`Outside`] resolved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LiveOutside {
    #[default]
    Nothing,
    /// None: the instrument isn't in the song.
    Play(Option<keys::Inst>),
    Routed,
}

/// A patch as the audio thread plays it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LivePatch {
    pub zones: [Option<LiveZone>; MAX_ZONES],
    pub outside: LiveOutside,
    pub cut_old: bool,
}

/// Every patch of the song, resolved: what [`keys::Keys`] plays from. Made
/// on the screen thread ([`Patches::resolve`]) and handed over whole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rig {
    pub patches: [Option<LivePatch>; MAX_PATCHES],
    /// The keyboards the zones name, each once, by the name the project
    /// gives the port ([`Takes::Port`] holds a place in this).
    pub keyboards: Vec<String>,
}

impl Default for Rig {
    fn default() -> Self {
        Rig {
            patches: [None; MAX_PATCHES],
            keyboards: Vec::new(),
        }
    }
}

impl Rig {
    /// A keyboard's place in [`Rig::keyboards`], by its port's name.
    pub fn keyboard(&self, name: &str) -> Option<u8> {
        let i = self.keyboards.iter().position(|k| k == name)?;
        u8::try_from(i).ok()
    }

    /// Patch `i` (from 0), if the song has it.
    pub fn patch(&self, i: usize) -> Option<&LivePatch> {
        self.patches.get(i)?.as_ref()
    }

    /// How many patches the song has.
    pub fn len(&self) -> usize {
        self.patches.iter().take_while(|p| p.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.patches[0].is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chorus_split() -> Patch {
        let mut bass = Zone::new("track 4", 0, 47);
        bass.pedals = false;
        let grand = Zone::new("kit 3", 48, 96);
        let mut strings = Zone::new("free 2", 48, 96);
        strings.vel_low = 64;
        strings.curve = Curve::Soft;
        strings.keyboard = Keyboard::Port {
            name: "KeyLab 61".into(),
            ch: Some(0),
        };
        Patch {
            name: "Chorus split".into(),
            zones: vec![bass, grand, strings],
            outside: Outside::Nothing,
            cut_old: false,
            extra: vec![],
        }
    }

    #[test]
    fn patches_read_back_as_written_and_keep_lines_they_dont_know() {
        let mut lead = Layout::Whole.patch("Lead", "track 5");
        lead.zones[0].transpose = 12;
        lead.zones[0].keyboard = Keyboard::Port {
            name: "Seaboard".into(),
            ch: None,
        };
        lead.outside = Outside::Routed;
        lead.cut_old = true;
        let mut organ = Layout::BassAndKeys.patch("Organ", "track 6");
        organ.outside = Outside::Play("track 1".into());
        organ.zones[1].curve = Curve::Fixed(100);
        let p = Patches {
            list: vec![chorus_split(), lead, organ],
        };
        let text = p.to_text();
        assert!(
            text.starts_with("mpfree-patches 1\npatch = Chorus split\n"),
            "{text}"
        );
        assert_eq!(Patches::parse(&text), Ok(p.clone()));

        // A later build's lines stay where they were: one after the
        // first patch's own lines, one after its first zone's.
        let zone = text.find("zone = track 4\n").unwrap();
        let wheels = zone + text[zone..].find("wheels = ").unwrap();
        let end = wheels + text[wheels..].find('\n').unwrap() + 1;
        let mut later = text.clone();
        later.insert_str(end, "latch = on\n");
        let later = later.replacen(
            "cut-old = no\nzone = track 4\n",
            "cut-old = no\ncolour = amber\nzone = track 4\n",
            1,
        );
        let back = Patches::parse(&later).unwrap();
        assert_eq!(back.list[0].extra, vec!["colour = amber".to_string()]);
        assert_eq!(back.list[0].zones[0].extra, vec!["latch = on".to_string()]);
        assert_eq!(back.to_text(), later);
    }

    #[test]
    fn a_part_that_cant_be_read_says_which_line() {
        assert_eq!(
            Patches::parse("mpfree-patches 2\n"),
            Err(ParseError::Head("mpfree-patches 2".into()))
        );
        let good = Patches {
            list: vec![chorus_split()],
        }
        .to_text();
        for (from, to) in [
            ("keys = 0 47", "keys = 47 0"),
            ("velocities = 64 127", "velocities = 0 127"),
            ("transpose = 0", "transpose = 49"),
            ("velocity = soft", "velocity = louder"),
            ("pedals = no", "pedals = maybe"),
            ("keyboard = KeyLab 61 / 1", "keyboard = KeyLab 61 / 17"),
        ] {
            let bad = good.replacen(from, to, 1);
            assert!(
                matches!(Patches::parse(&bad), Err(ParseError::Line(_, l)) if l == to),
                "{to}"
            );
        }
        // A ninth zone is refused, not dropped.
        let mut nine = Layout::Whole.patch("Nine", "track 1");
        nine.zones = (0..8).map(|i| Zone::new("track 1", i, i)).collect();
        let text = Patches { list: vec![nine] }.to_text() + "zone = track 2\n";
        assert!(matches!(Patches::parse(&text), Err(ParseError::Line(_, _))));
    }

    #[test]
    fn the_layouts_lay_out_their_zones() {
        let z = |l: Layout| {
            l.patch("P", "track 1")
                .zones
                .iter()
                .map(|z| (z.low, z.high, z.pedals))
                .collect::<Vec<_>>()
        };
        assert_eq!(z(Layout::Whole), vec![(0, 127, true)]);
        assert_eq!(
            z(Layout::BassAndKeys),
            vec![(0, 47, false), (48, 127, true)]
        );
        assert_eq!(z(Layout::Layered), vec![(0, 127, true), (0, 127, true)]);
        assert_eq!(
            z(Layout::ThreeWay),
            vec![(0, 47, true), (48, 71, true), (72, 127, true)]
        );
        assert_eq!(
            Layout::Whole.patch(&"x".repeat(40), "t").name.len(),
            MAX_NAME
        );
    }

    #[test]
    fn curves_shape_velocity() {
        assert_eq!(Curve::AsPlayed.apply(64), 64);
        assert_eq!(Curve::Fixed(100).apply(3), 100);
        assert!(Curve::Soft.apply(32) > 32);
        assert!(Curve::Hard.apply(96) < 96);
        for c in [Curve::Soft, Curve::Hard, Curve::AsPlayed] {
            assert_eq!(c.apply(127), 127);
            assert!(c.apply(1) >= 1);
            assert!(c.apply(0) >= 1);
        }
    }

    #[test]
    fn resolving_finds_instruments_and_keyboards_as_they_are_now() {
        let p = Patches {
            list: vec![chorus_split()],
        };
        let rig = p.resolve(|i| match i {
            "track 4" => Some(keys::Inst(4)),
            "kit 3" => Some(keys::Inst(30)),
            _ => None,
        });
        assert_eq!(rig.len(), 1);
        let live = rig.patch(0).unwrap();
        let z: Vec<_> = live.zones.iter().flatten().collect();
        assert_eq!(z.len(), 3);
        assert_eq!(z[0].inst, Some(keys::Inst(4)));
        assert_eq!(z[1].inst, Some(keys::Inst(30)));
        // Strings isn't in the song, and it names its keyboard.
        assert_eq!(z[2].inst, None);
        assert_eq!(rig.keyboards, vec!["KeyLab 61".to_string()]);
        assert_eq!(rig.keyboard("KeyLab 61"), Some(0));
        assert_eq!(z[2].takes, Takes::Port { kb: 0, ch: Some(0) });
        assert!(z[0].takes(None, 0, 30, 10));
        assert!(!z[0].takes(None, 0, 48, 10));
        // Only from that keyboard, on its channel.
        assert!(!z[2].takes(None, 0, 60, 100));
        assert!(z[2].takes(Some(0), 0, 60, 100));
        assert!(!z[2].takes(Some(0), 1, 60, 100));
    }
}
