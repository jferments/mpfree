//! A project's MIDI, sync and CV settings: what the mixer's I/O view edits
//! and the project saves as its `midi` part (`parts/midi.txt`).
//!
//! The owner's calls (2026-11-12): MIDI and sync are project settings, not
//! Prefs, because they change from project to project. Facts about the rig
//! rather than the project are kept apart ([`MidiSetup::rig`], saved by the
//! app for every project): each port's input, thru, what it ignores coming
//! in and the delay of the gear on it.
//!
//! - **Ports are named, not numbered.** A project says "Digitone" or
//!   [`DIN_NAME`]; the app finds the port by that name when the device is
//!   plugged in ([`crate::port::Ports`] ids only last while the app runs). A
//!   setting for a device that isn't plugged in is kept, and the screen says
//!   so in place.
//! - **One output setting per track**: the port and the channel together
//!   ([`TrackOut`]), never two look-alike fields (the owner's rule).
//! - **Defaults** (agreed 2026-11-12): clock out Off on every port, thru
//!   Off, a port's input plays tracks and can be learned ([`PortInput::Both`]),
//!   the tempo is MPFree's own.
//! - **Text** is `key = value` lines, like every other part. A line that
//!   can't be read is skipped and named in the warnings that [`MidiSetup::parse`]
//!   returns, for the app to log and show: a project never fails to open
//!   over its MIDI part, and nothing is dropped silently.

use crate::cv::{CvAssign, Priority, Scale, CV_OUTS};
use crate::learn::Saved;
use crate::msg::{Channel, Msg};
use crate::output::MAX_TRACKS;

/// The 5-pin MIDI jacks' name in a project.
pub const DIN_NAME: &str = "MIDI";

/// What a port's input does. Akai calls these Global, Control and Track;
/// MPFree says what each does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PortInput {
    /// Its notes play tracks; MIDI Learn doesn't hear it.
    PlaysTracks,
    /// Only MIDI Learn hears it; nothing plays.
    LearnOnly,
    /// Its notes play tracks and its controls can be learned.
    #[default]
    Both,
}

impl PortInput {
    pub const ALL: [PortInput; 3] = [
        PortInput::PlaysTracks,
        PortInput::LearnOnly,
        PortInput::Both,
    ];

    /// As the screen says it.
    pub fn label(self) -> &'static str {
        match self {
            PortInput::PlaysTracks => "Plays tracks",
            PortInput::LearnOnly => "Learn only",
            PortInput::Both => "Both",
        }
    }

    fn word(self) -> &'static str {
        match self {
            PortInput::PlaysTracks => "plays-tracks",
            PortInput::LearnOnly => "learn-only",
            PortInput::Both => "both",
        }
    }

    fn from_word(w: &str) -> Option<PortInput> {
        PortInput::ALL.into_iter().find(|p| p.word() == w)
    }
}

/// When a port sends MIDI clock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClockOut {
    #[default]
    Off,
    /// Clock, start and stop while the transport plays.
    WhilePlaying,
    /// Clock all the time, so a drum machine's arpeggiator keeps time while
    /// stopped; start and stop with the transport.
    Always,
}

impl ClockOut {
    pub const ALL: [ClockOut; 3] = [ClockOut::Off, ClockOut::WhilePlaying, ClockOut::Always];

    pub fn label(self) -> &'static str {
        match self {
            ClockOut::Off => "Off",
            ClockOut::WhilePlaying => "While playing",
            ClockOut::Always => "Always",
        }
    }

    fn word(self) -> &'static str {
        match self {
            ClockOut::Off => "off",
            ClockOut::WhilePlaying => "while-playing",
            ClockOut::Always => "always",
        }
    }

    fn from_word(w: &str) -> Option<ClockOut> {
        ClockOut::ALL.into_iter().find(|c| c.word() == w)
    }
}

/// What a port can be set to ignore coming in (its page's "Ignore coming
/// in"). Ignored messages still show in the MIDI monitor, marked
/// "ignored", but nothing past the port gets them: no track, Learn, clock
/// follower or thru.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ignored {
    /// Clock, start, continue, stop and song position: the port can't lead
    /// the tempo.
    Clock,
    /// Program changes.
    Program,
    /// Pressure: polyphonic and channel aftertouch.
    Aftertouch,
    /// The messages that end every held note on a channel: All notes off
    /// (CC 123), All sound off (CC 120) and the mode changes (CC 124 to
    /// 127). Akai's "Filter All Notes Off".
    AllNotesOff,
}

impl Ignored {
    pub const ALL: [Ignored; 4] = [
        Ignored::Clock,
        Ignored::Program,
        Ignored::Aftertouch,
        Ignored::AllNotesOff,
    ];

    /// As the port's page says it.
    pub fn label(self) -> &'static str {
        match self {
            Ignored::Clock => "Clock",
            Ignored::Program => "Program",
            Ignored::Aftertouch => "Aftertouch",
            Ignored::AllNotesOff => "All notes off",
        }
    }

    fn word(self) -> &'static str {
        match self {
            Ignored::Clock => "clock",
            Ignored::Program => "program",
            Ignored::Aftertouch => "aftertouch",
            Ignored::AllNotesOff => "all-notes-off",
        }
    }

    fn bit(self) -> u8 {
        1 << Ignored::ALL.iter().position(|k| *k == self).unwrap_or(0)
    }

    /// The kind a channel message is, if it is one a port can ignore.
    pub fn of(m: &Msg) -> Option<Ignored> {
        match *m {
            Msg::Program { .. } => Some(Ignored::Program),
            Msg::PolyPressure { .. } | Msg::ChanPressure { .. } => Some(Ignored::Aftertouch),
            Msg::Cc { cc, .. } if ends_notes(cc) => Some(Ignored::AllNotesOff),
            _ => None,
        }
    }
}

/// Whether controller `cc` is a channel mode message that ends every held
/// note on its channel: All sound off (120), All notes off (123), Omni off
/// and on (124, 125), Mono and Poly (126, 127).
pub fn ends_notes(cc: u8) -> bool {
    cc == 120 || (123..=127).contains(&cc)
}

/// The kinds one port ignores.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct IgnoreSet(u8);

impl IgnoreSet {
    pub const NONE: IgnoreSet = IgnoreSet(0);

    pub fn has(self, k: Ignored) -> bool {
        self.0 & k.bit() != 0
    }

    pub fn with(self, k: Ignored, on: bool) -> IgnoreSet {
        if on {
            IgnoreSet(self.0 | k.bit())
        } else {
            IgnoreSet(self.0 & !k.bit())
        }
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether a channel message is one this set ignores.
    pub fn msg(self, m: &Msg) -> bool {
        Ignored::of(m).is_some_and(|k| self.has(k))
    }

    /// The kinds, in [`Ignored::ALL`]'s order.
    pub fn kinds(self) -> impl Iterator<Item = Ignored> {
        Ignored::ALL.into_iter().filter(move |k| self.has(*k))
    }

    fn text(self) -> String {
        self.kinds()
            .map(Ignored::word)
            .collect::<Vec<_>>()
            .join(",")
    }

    fn parse(v: &str) -> Result<IgnoreSet, String> {
        let mut set = IgnoreSet::NONE;
        for w in v.split(',').map(str::trim).filter(|w| !w.is_empty()) {
            let k = Ignored::ALL
                .into_iter()
                .find(|k| k.word() == w)
                .ok_or(format!("unknown kind \"{w}\""))?;
            set = set.with(k, true);
        }
        Ok(set)
    }
}

/// The most a port's gear can be said to sound late, in tenths of a ms
/// (100 ms).
pub const MAX_DELAY_TENTHS: u16 = 1000;
/// The port page's - and + step, in tenths of a ms.
pub const DELAY_STEP_TENTHS: u16 = 5;

/// One port's settings. A port with none of its own uses
/// [`PortSetup::new`]'s defaults. Its clock out is the project's; the rest
/// is the rig's ([`MidiSetup::rig`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortSetup {
    pub name: String,
    pub input: PortInput,
    pub clock_out: ClockOut,
    /// What arrives on this port's input leaves its output too.
    pub thru: bool,
    /// What its input ignores.
    pub ignore: IgnoreSet,
    /// How late the gear on this port sounds after a message leaves, in
    /// tenths of a ms (its MIDI and its own processing): MIDI out sends
    /// this much early, so it lands on the beat. 0 to [`MAX_DELAY_TENTHS`].
    pub delay_tenths: u16,
}

impl PortSetup {
    pub fn new(name: &str) -> PortSetup {
        PortSetup {
            name: name.to_string(),
            input: PortInput::default(),
            clock_out: ClockOut::default(),
            thru: false,
            ignore: IgnoreSet::NONE,
            delay_tenths: 0,
        }
    }

    /// The delay in ms.
    pub fn delay_ms(&self) -> f32 {
        f32::from(self.delay_tenths) / 10.0
    }

    fn is_default(&self) -> bool {
        *self == PortSetup::new(&self.name)
    }
}

/// Where the tempo comes from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TempoSource {
    /// MPFree's own tempo.
    #[default]
    Internal,
    /// Follows the clock arriving on this port.
    Port(String),
}

/// Where one MIDI track plays: a port and a channel together, as one
/// setting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackOut {
    pub port: String,
    /// 0 to 15 (shown 1 to 16).
    pub channel: Channel,
    /// Sent when the song starts and whenever it changes: 0 to 127 (shown
    /// 1 to 128).
    pub program: Option<u8>,
    /// Bank select (CC 0 and 32), 0 to 16383, sent before the program.
    pub bank: Option<u16>,
}

impl TrackOut {
    pub fn new(port: &str, channel: Channel) -> TrackOut {
        TrackOut {
            port: port.to_string(),
            channel: channel.min(15),
            program: None,
            bank: None,
        }
    }

    /// "MIDI · ch 10", as the track's one output field shows it.
    pub fn label(&self) -> String {
        format!("{} · ch {}", self.port, self.channel + 1)
    }
}

/// The tracks a route may name: the sequencer's (`mpfree_seq::MAX_TRACKS`).
pub const ROUTE_TRACKS: usize = 128;

/// A port and channel that play one track, whichever track is armed: a
/// route to "Tracks by channel" in the I/O view's Routing. Saved with the
/// project, as `route = track 1 < MIDI · ch 10` (or `· all channels`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackRoute {
    /// The port, by name, as [`PortSetup::name`].
    pub port: String,
    /// None: every channel.
    pub channel: Option<Channel>,
    /// The track, from 0, below [`ROUTE_TRACKS`].
    pub track: u8,
}

impl TrackRoute {
    /// "MIDI · ch 10" or "Keystep 37 · all channels".
    pub fn source(&self) -> String {
        match self.channel {
            Some(c) => format!("{} · ch {}", self.port, c + 1),
            None => format!("{} · all channels", self.port),
        }
    }

    fn text(&self) -> String {
        format!("track {} < {}", u16::from(self.track) + 1, self.source())
    }

    fn parse(v: &str) -> Result<TrackRoute, String> {
        let shape = || "not \"track N < port · ch N\"".to_string();
        let (t, from) = v.split_once(" < ").ok_or_else(shape)?;
        let t = t.trim().strip_prefix("track ").ok_or_else(shape)?;
        let track = (number(t.trim(), 1, ROUTE_TRACKS as u32)? - 1) as u8;
        let (port, ch) = from.rsplit_once(" · ").ok_or_else(shape)?;
        let channel = match ch.trim() {
            "all channels" => None,
            c => Some(
                (number(c.strip_prefix("ch ").ok_or_else(shape)?.trim(), 1, 16)? - 1) as Channel,
            ),
        };
        let port = port.trim();
        if port.is_empty() {
            return Err(shape());
        }
        Ok(TrackRoute {
            port: port.to_string(),
            channel,
            track,
        })
    }
}

/// The most routes a project keeps (`mpfree_midi::input::MAX_ROUTES`).
pub const MAX_TRACK_ROUTES: usize = crate::input::MAX_ROUTES;

/// A project's MIDI, sync and CV settings.
#[derive(Clone, Debug, PartialEq)]
pub struct MidiSetup {
    pub tempo: TempoSource,
    /// While following another device: its start and stop start and stop
    /// MPFree.
    pub follow_start_stop: bool,
    /// While following: its song position moves MPFree's.
    pub follow_position: bool,
    /// Ports with settings of their own, by name, in the order first set.
    pub ports: Vec<PortSetup>,
    /// Each track's MIDI output; `None` plays nothing out.
    pub tracks: [Option<TrackOut>; MAX_TRACKS],
    /// Ports and channels that play a track whichever track is armed, in
    /// the order set; at most [`MAX_TRACK_ROUTES`].
    pub routes: Vec<TrackRoute>,
    /// What each CV/gate output does.
    pub cv: [CvAssign; CV_OUTS],
    /// Controls learned with MIDI Learn, each with its device by name, in
    /// the order learned: the song's.
    pub learn: Vec<Saved>,
    /// Controls learned for every project (kept with the rig, as a port's
    /// input is): a controller's transport keys or a tempo fader. The
    /// song's own mapping for the same control wins ([`MidiSetup::in_use`]).
    pub rig_learn: Vec<Saved>,
}

impl Default for MidiSetup {
    fn default() -> Self {
        MidiSetup {
            tempo: TempoSource::Internal,
            follow_start_stop: true,
            follow_position: true,
            ports: Vec::new(),
            tracks: std::array::from_fn(|_| None),
            routes: Vec::new(),
            cv: [CvAssign::Off; CV_OUTS],
            learn: Vec::new(),
            rig_learn: Vec::new(),
        }
    }
}

/// Where a learned control is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kept {
    /// With the song ([`MidiSetup::learn`]).
    Project,
    /// For every project ([`MidiSetup::rig_learn`]).
    Rig,
}

/// The most learned controls a project keeps (and the rig keeps).
pub const MAX_LEARNED: usize = 256;

/// The most ports a project names.
pub const MAX_PORT_LINES: usize = 64;

/// The part's first line.
pub const HEADER: &str = "mpfree-midi 1";

impl MidiSetup {
    /// The settings of the port named `name`, its defaults when it has none.
    pub fn port(&self, name: &str) -> PortSetup {
        self.ports
            .iter()
            .find(|p| p.name == name)
            .cloned()
            .unwrap_or_else(|| PortSetup::new(name))
    }

    /// Changes the port named `name`'s settings. A port back at its
    /// defaults is dropped from the list.
    pub fn set_port(&mut self, name: &str, f: impl FnOnce(&mut PortSetup)) {
        let i = match self.ports.iter().position(|p| p.name == name) {
            Some(i) => i,
            None => {
                self.ports.push(PortSetup::new(name));
                self.ports.len() - 1
            }
        };
        f(&mut self.ports[i]);
        self.ports[i].name = name.to_string();
        if self.ports[i].is_default() {
            self.ports.remove(i);
        }
    }

    /// A control was learned: it replaces one learned before on the same
    /// control (a control moves one thing). False when the project has
    /// [`MAX_LEARNED`] already.
    pub fn learned(&mut self, s: Saved) -> bool {
        self.learned_in(Kept::Project, s)
    }

    /// [`MidiSetup::learned`], kept with the song or the rig. A control
    /// moves one thing in each: the same control in the other list stays
    /// (the song's wins while it is open).
    pub fn learned_in(&mut self, kept: Kept, s: Saved) -> bool {
        let list = match kept {
            Kept::Project => &mut self.learn,
            Kept::Rig => &mut self.rig_learn,
        };
        if let Some(old) = list.iter_mut().find(|o| o.same_control(&s)) {
            *old = s;
        } else if list.len() < MAX_LEARNED {
            list.push(s);
        } else {
            return false;
        }
        true
    }

    /// Every learned control, the song's first, with where it is kept: the
    /// order the Learn section lists them and [`MidiSetup::learned_at`]
    /// counts them.
    pub fn all_learned(&self) -> impl Iterator<Item = (Kept, &Saved)> {
        self.learn
            .iter()
            .map(|l| (Kept::Project, l))
            .chain(self.rig_learn.iter().map(|l| (Kept::Rig, l)))
    }

    /// The `k`th of [`MidiSetup::all_learned`].
    pub fn learned_at(&self, k: usize) -> Option<(Kept, &Saved)> {
        self.all_learned().nth(k)
    }

    /// The `k`th of [`MidiSetup::all_learned`], to change.
    pub fn learned_at_mut(&mut self, k: usize) -> Option<&mut Saved> {
        let n = self.learn.len();
        if k < n {
            self.learn.get_mut(k)
        } else {
            self.rig_learn.get_mut(k - n)
        }
    }

    /// Forgets the `k`th of [`MidiSetup::all_learned`].
    pub fn forget_at(&mut self, k: usize) -> Option<(Kept, Saved)> {
        let n = self.learn.len();
        if k < n {
            Some((Kept::Project, self.learn.remove(k)))
        } else if k - n < self.rig_learn.len() {
            Some((Kept::Rig, self.rig_learn.remove(k - n)))
        } else {
            None
        }
    }

    /// Moves the `k`th of [`MidiSetup::all_learned`] to the song or the
    /// rig. Returns its new place, or None when there is no `k`th or no
    /// room (it stays). The same control already kept there is replaced.
    pub fn keep_at(&mut self, k: usize, kept: Kept) -> Option<usize> {
        let (was, l) = self.learned_at(k).map(|(w, l)| (w, l.clone()))?;
        if was == kept {
            return Some(k);
        }
        let (from, to) = match kept {
            Kept::Project => (&mut self.rig_learn, &mut self.learn),
            Kept::Rig => (&mut self.learn, &mut self.rig_learn),
        };
        let at = match to.iter().position(|o| o.same_control(&l)) {
            Some(i) => {
                to[i] = l.clone();
                i
            }
            None if to.len() < MAX_LEARNED => {
                to.push(l.clone());
                to.len() - 1
            }
            None => return None,
        };
        from.retain(|o| *o != l);
        Some(match kept {
            Kept::Project => at,
            Kept::Rig => self.learn.len() + at,
        })
    }

    /// The learned controls that work now: the song's, then the rig's
    /// whose control the song doesn't use.
    pub fn in_use(&self) -> Vec<Saved> {
        let mut v = self.learn.clone();
        v.extend(
            self.rig_learn
                .iter()
                .filter(|r| !self.learn.iter().any(|l| l.same_control(r)))
                .cloned(),
        );
        v
    }

    /// The settings that belong to the song, saved in the project's `midi`
    /// part: tracks' outputs, the routes to tracks, Tempo from and
    /// following, each port's clock out, the CV jobs and learned controls. Each port's input and thru
    /// are the rig's ([`MidiSetup::rig`]) and are left at their defaults.
    pub fn project(&self) -> MidiSetup {
        let mut m = self.clone();
        m.rig_learn.clear();
        for name in self
            .ports
            .iter()
            .map(|p| p.name.clone())
            .collect::<Vec<_>>()
        {
            m.set_port(&name, |p| {
                p.input = PortInput::default();
                p.thru = false;
                p.ignore = IgnoreSet::NONE;
                p.delay_tenths = 0;
            });
        }
        m
    }

    /// The settings that belong to the gear on the desk, kept for every
    /// project (owner, 13 Nov 2026: project settings apart from system-wide
    /// ones): each port's input (Plays tracks, Learn only, Both), thru,
    /// what it ignores coming in and its gear's delay, and the controls
    /// learned for every project.
    pub fn rig(&self) -> MidiSetup {
        let mut m = MidiSetup {
            rig_learn: self.rig_learn.clone(),
            ..MidiSetup::default()
        };
        for p in &self.ports {
            m.set_port(&p.name, |q| {
                q.input = p.input;
                q.thru = p.thru;
                q.ignore = p.ignore;
                q.delay_tenths = p.delay_tenths;
            });
        }
        m
    }

    /// `project`'s song settings with this setup's rig settings: a project
    /// opened, or a new one ([`MidiSetup::default`]).
    pub fn set_project(&mut self, project: &MidiSetup) {
        let rig = self.rig();
        *self = project.project();
        self.set_rig(&rig);
    }

    /// `rig`'s port settings (input, thru, ignore, delay) and controls
    /// learned for every project, this setup's song settings kept.
    pub fn set_rig(&mut self, rig: &MidiSetup) {
        self.rig_learn = rig.rig_learn.clone();
        let names: Vec<String> = self
            .ports
            .iter()
            .chain(rig.ports.iter())
            .map(|p| p.name.clone())
            .collect();
        for name in names {
            let want = rig.port(&name);
            self.set_port(&name, |p| {
                p.input = want.input;
                p.thru = want.thru;
                p.ignore = want.ignore;
                p.delay_tenths = want.delay_tenths;
            });
        }
    }

    /// The project part's text ([`MidiSetup::project`]).
    pub fn project_text(&self) -> String {
        self.project().to_text()
    }

    /// The rig's text ([`MidiSetup::rig`]), in the same form.
    pub fn rig_text(&self) -> String {
        self.rig().to_text()
    }

    /// The part's text. Only what differs from the defaults is written,
    /// after the header.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        s.push_str(HEADER);
        s.push('\n');
        let d = MidiSetup::default();
        match &self.tempo {
            TempoSource::Internal => {}
            TempoSource::Port(n) => line(&mut s, "tempo.port", n),
        }
        if self.follow_start_stop != d.follow_start_stop {
            line(&mut s, "follow.start-stop", on_off(self.follow_start_stop));
        }
        if self.follow_position != d.follow_position {
            line(&mut s, "follow.position", on_off(self.follow_position));
        }
        for (i, p) in self.ports.iter().enumerate() {
            let k = format!("port.{}", i + 1);
            line(&mut s, &k, &p.name);
            line(&mut s, &format!("{k}.input"), p.input.word());
            line(&mut s, &format!("{k}.clock-out"), p.clock_out.word());
            line(&mut s, &format!("{k}.thru"), on_off(p.thru));
            if !p.ignore.is_empty() {
                line(&mut s, &format!("{k}.ignore"), &p.ignore.text());
            }
            if p.delay_tenths != 0 {
                line(
                    &mut s,
                    &format!("{k}.delay-ms"),
                    &format!("{:.1}", p.delay_ms()),
                );
            }
        }
        for (i, t) in self.tracks.iter().enumerate() {
            let Some(t) = t else { continue };
            let k = format!("track.{}", i + 1);
            line(&mut s, &format!("{k}.port"), &t.port);
            line(
                &mut s,
                &format!("{k}.channel"),
                &(t.channel + 1).to_string(),
            );
            if let Some(p) = t.program {
                line(&mut s, &format!("{k}.program"), &(p as u16 + 1).to_string());
            }
            if let Some(b) = t.bank {
                line(&mut s, &format!("{k}.bank"), &b.to_string());
            }
        }
        for r in &self.routes {
            line(&mut s, "route", &r.text());
        }
        for (i, a) in self.cv.iter().enumerate() {
            if *a != CvAssign::Off {
                line(&mut s, &format!("cv.{}", i + 1), &cv_text(a));
            }
        }
        for l in &self.learn {
            line(&mut s, "learn", &l.text());
        }
        for l in &self.rig_learn {
            line(&mut s, "learn-rig", &l.text());
        }
        s
    }

    /// Reads a part written by [`MidiSetup::to_text`]. Every line it can't
    /// read is skipped and named in the warnings, with its line number.
    pub fn parse(text: &str) -> (MidiSetup, Vec<String>) {
        let mut m = MidiSetup::default();
        let mut warn = Vec::new();
        // Ports by their number in the file, so a port's lines may come in
        // any order.
        let mut ports: Vec<(usize, PortSetup)> = Vec::new();
        let mut lines = text.lines().enumerate();
        match lines.next() {
            Some((_, h)) if h.trim() == HEADER => {}
            Some((_, h)) => warn.push(format!(
                "line 1: expected \"{HEADER}\", found \"{}\"",
                h.trim()
            )),
            None => return (m, warn),
        }
        for (n, raw) in lines {
            let n = n + 1;
            let l = raw.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            let Some((k, v)) = l.split_once(" = ").or_else(|| l.split_once('=')) else {
                warn.push(format!("line {n}: not \"key = value\": {l}"));
                continue;
            };
            let (k, v) = (k.trim(), v.trim());
            if let Err(e) = m.apply(k, v, &mut ports) {
                warn.push(format!("line {n}: {k}: {e}"));
            }
        }
        ports.sort_by_key(|p| p.0);
        for (_, p) in ports {
            if p.name.is_empty() {
                warn.push("a port with settings but no name: skipped".to_string());
            } else if m.ports.iter().any(|q| q.name == p.name) {
                warn.push(format!("port \"{}\" set twice: the first kept", p.name));
            } else if !p.is_default() {
                m.ports.push(p);
            }
        }
        for (i, t) in m.tracks.iter_mut().enumerate() {
            if t.as_ref().is_some_and(|t| t.port.is_empty()) {
                warn.push(format!(
                    "track {}: a channel but no port: plays nothing out",
                    i + 1
                ));
                *t = None;
            }
        }
        (m, warn)
    }

    fn apply(
        &mut self,
        k: &str,
        v: &str,
        ports: &mut Vec<(usize, PortSetup)>,
    ) -> Result<(), String> {
        let parts: Vec<&str> = k.split('.').collect();
        match parts.as_slice() {
            ["tempo", "port"] => {
                if v.is_empty() {
                    return Err("no port named".into());
                }
                self.tempo = TempoSource::Port(v.to_string());
            }
            ["follow", "start-stop"] => self.follow_start_stop = parse_on(v)?,
            ["follow", "position"] => self.follow_position = parse_on(v)?,
            ["port", i, rest @ ..] => {
                let i = index(i, MAX_PORT_LINES)?;
                let p = match ports.iter().position(|p| p.0 == i) {
                    Some(j) => &mut ports[j].1,
                    None => {
                        ports.push((i, PortSetup::new("")));
                        &mut ports.last_mut().expect("just pushed").1
                    }
                };
                match rest {
                    [] => p.name = v.to_string(),
                    ["input"] => {
                        p.input = PortInput::from_word(v).ok_or(format!("unknown input \"{v}\""))?
                    }
                    ["clock-out"] => {
                        p.clock_out =
                            ClockOut::from_word(v).ok_or(format!("unknown clock out \"{v}\""))?
                    }
                    ["thru"] => p.thru = parse_on(v)?,
                    ["ignore"] => p.ignore = IgnoreSet::parse(v)?,
                    ["delay-ms"] => p.delay_tenths = parse_delay(v)?,
                    _ => return Err("unknown key".into()),
                }
            }
            ["track", i, field] => {
                let i = index(i, MAX_TRACKS)?;
                let t = self.tracks[i].get_or_insert_with(|| TrackOut::new("", 0));
                match *field {
                    "port" => t.port = v.to_string(),
                    "channel" => t.channel = (number(v, 1, 16)? - 1) as u8,
                    "program" => t.program = Some((number(v, 1, 128)? - 1) as u8),
                    "bank" => t.bank = Some(number(v, 0, 16383)? as u16),
                    _ => return Err("unknown key".into()),
                }
            }
            ["route"] => {
                let r = TrackRoute::parse(v)?;
                if self.routes.contains(&r) {
                    return Err("the same route twice: the first kept".into());
                }
                if self.routes.len() >= MAX_TRACK_ROUTES {
                    return Err(format!("more than {MAX_TRACK_ROUTES} routes"));
                }
                self.routes.push(r);
            }
            ["cv", i] => {
                let i = index(i, CV_OUTS)?;
                self.cv[i] = parse_cv(v)?;
            }
            ["learn-rig"] => {
                let l = Saved::parse(v).ok_or("not a learned control")?;
                if !self.learned_in(Kept::Rig, l) {
                    return Err(format!("more than {MAX_LEARNED} learned controls"));
                }
            }
            ["learn"] => {
                let l = Saved::parse(v).ok_or("not a learned control")?;
                if !self.learned(l) {
                    return Err(format!("more than {MAX_LEARNED} learned controls"));
                }
            }
            _ => return Err("unknown key".into()),
        }
        Ok(())
    }
}

fn line(s: &mut String, k: &str, v: &str) {
    s.push_str(k);
    s.push_str(" = ");
    s.push_str(v);
    s.push('\n');
}

fn on_off(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}

/// "6.5" (ms) as tenths of a ms, 0 to [`MAX_DELAY_TENTHS`].
fn parse_delay(v: &str) -> Result<u16, String> {
    let ms: f32 = v
        .parse()
        .map_err(|_| format!("\"{v}\" is not a number of ms"))?;
    let max = f32::from(MAX_DELAY_TENTHS) / 10.0;
    if !(0.0..=max).contains(&ms) {
        return Err(format!("{ms} ms is outside 0 to {max} ms"));
    }
    Ok((ms * 10.0).round() as u16)
}

fn parse_on(v: &str) -> Result<bool, String> {
    match v {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(format!("expected on or off, found \"{v}\"")),
    }
}

/// A 1-based number in a key, as a 0-based index below `max`.
fn index(s: &str, max: usize) -> Result<usize, String> {
    match s.parse::<usize>() {
        Ok(n) if n >= 1 && n <= max => Ok(n - 1),
        _ => Err(format!("\"{s}\" is not a number from 1 to {max}")),
    }
}

fn number(v: &str, lo: u32, hi: u32) -> Result<u32, String> {
    match v.parse::<u32>() {
        Ok(n) if (lo..=hi).contains(&n) => Ok(n),
        _ => Err(format!("\"{v}\" is not a number from {lo} to {hi}")),
    }
}

fn priority_word(p: Priority) -> &'static str {
    match p {
        Priority::Last => "last",
        Priority::Low => "low",
        Priority::High => "high",
    }
}

/// A CV output's job as one line of `name=value` words after its kind:
/// `pitch track=1 zero=36 bend=2 priority=last v/oct=1`.
fn cv_text(a: &CvAssign) -> String {
    match *a {
        CvAssign::Off => "off".into(),
        CvAssign::Pitch {
            track,
            scale,
            zero_note,
            bend_semis,
            priority,
        } => {
            let scale = match scale {
                Scale::VoltsPerOctave(v) => format!("v/oct={}", f(v)),
                Scale::HzPerVolt {
                    ref_volts,
                    ref_note,
                } => {
                    format!("hz/v={}@{ref_note}", f(ref_volts))
                }
            };
            format!(
                "pitch track={} zero={zero_note} bend={bend_semis} priority={} {scale}",
                track + 1,
                priority_word(priority)
            )
        }
        CvAssign::Gate {
            track,
            volts,
            retrigger_ms,
        } => format!(
            "gate track={} volts={} retrigger-ms={}",
            track + 1,
            f(volts),
            f(retrigger_ms)
        ),
        CvAssign::Trigger {
            track,
            note,
            volts,
            ms,
        } => {
            let note = note.map(|n| format!(" note={n}")).unwrap_or_default();
            format!(
                "trigger track={}{note} volts={} ms={}",
                track + 1,
                f(volts),
                f(ms)
            )
        }
        CvAssign::Velocity { track, max_volts } => {
            format!("velocity track={} max-volts={}", track + 1, f(max_volts))
        }
        CvAssign::Cc {
            track,
            cc,
            min_volts,
            max_volts,
        } => format!(
            "cc track={} cc={cc} min-volts={} max-volts={}",
            track + 1,
            f(min_volts),
            f(max_volts)
        ),
        CvAssign::Clock { ppqn, volts, ms } => {
            format!("clock ppqn={ppqn} volts={} ms={}", f(volts), f(ms))
        }
        CvAssign::Run { volts } => format!("run volts={}", f(volts)),
        CvAssign::Reset { volts, ms } => format!("reset volts={} ms={}", f(volts), f(ms)),
        CvAssign::Fixed { volts } => format!("fixed volts={}", f(volts)),
    }
}

/// A float written short and read back exactly.
fn f(v: f32) -> String {
    format!("{v}")
}

/// The most volts an Akai CV jack is set to give (0 to 5 V, the owner's
/// captures and forum reports; not yet measured on the unit).
pub const MAX_VOLTS: f32 = 5.0;

fn parse_cv(v: &str) -> Result<CvAssign, String> {
    let mut words = v.split_whitespace();
    let kind = words.next().ok_or("empty")?;
    let mut kv: Vec<(&str, &str)> = Vec::new();
    for w in words {
        let (k, x) = w
            .split_once('=')
            .ok_or(format!("\"{w}\" is not name=value"))?;
        kv.push((k, x));
    }
    let get = |k: &str| kv.iter().find(|p| p.0 == k).map(|p| p.1);
    let need = |k: &str| get(k).ok_or(format!("{kind}: no {k}"));
    let track = || -> Result<u8, String> { Ok(index(need("track")?, MAX_TRACKS)? as u8) };
    let volts = |k: &str| -> Result<f32, String> {
        let x: f32 = need(k)?.parse().map_err(|_| format!("{k}: not a number"))?;
        if (0.0..=MAX_VOLTS).contains(&x) {
            Ok(x)
        } else {
            Err(format!("{k}: {x} is outside 0 to {MAX_VOLTS} V"))
        }
    };
    let ms = |k: &str| -> Result<f32, String> {
        let x: f32 = need(k)?.parse().map_err(|_| format!("{k}: not a number"))?;
        if (0.0..=1000.0).contains(&x) {
            Ok(x)
        } else {
            Err(format!("{k}: {x} is outside 0 to 1000 ms"))
        }
    };
    let byte = |k: &str| -> Result<u8, String> { Ok(number(need(k)?, 0, 127)? as u8) };
    for (k, _) in &kv {
        let known: &[&str] = match kind {
            "pitch" => &["track", "zero", "bend", "priority", "v/oct", "hz/v"],
            "gate" => &["track", "volts", "retrigger-ms"],
            "trigger" => &["track", "note", "volts", "ms"],
            "velocity" => &["track", "max-volts"],
            "cc" => &["track", "cc", "min-volts", "max-volts"],
            "clock" => &["ppqn", "volts", "ms"],
            "run" => &["volts"],
            "reset" => &["volts", "ms"],
            "fixed" => &["volts"],
            _ => &[],
        };
        if !known.contains(k) && kind != "off" {
            return Err(format!("{kind}: unknown \"{k}\""));
        }
    }
    Ok(match kind {
        "off" => CvAssign::Off,
        "pitch" => {
            let scale = match (get("v/oct"), get("hz/v")) {
                (Some(x), None) => {
                    let x: f32 = x.parse().map_err(|_| "v/oct: not a number")?;
                    if !(0.1..=2.0).contains(&x) {
                        return Err(format!("v/oct: {x} is outside 0.1 to 2"));
                    }
                    Scale::VoltsPerOctave(x)
                }
                (None, Some(x)) => {
                    let (v, n) = x.split_once('@').ok_or("hz/v: expected volts@note")?;
                    let ref_volts: f32 = v.parse().map_err(|_| "hz/v: not a number")?;
                    if !(0.01..=MAX_VOLTS).contains(&ref_volts) {
                        return Err(format!(
                            "hz/v: {ref_volts} is outside 0.01 to {MAX_VOLTS} V"
                        ));
                    }
                    Scale::HzPerVolt {
                        ref_volts,
                        ref_note: number(n, 0, 127)? as u8,
                    }
                }
                _ => return Err("pitch: one of v/oct or hz/v".into()),
            };
            let priority = match need("priority")? {
                "last" => Priority::Last,
                "low" => Priority::Low,
                "high" => Priority::High,
                p => return Err(format!("unknown priority \"{p}\"")),
            };
            CvAssign::Pitch {
                track: track()?,
                scale,
                zero_note: byte("zero")?,
                bend_semis: number(need("bend")?, 0, 48)? as u8,
                priority,
            }
        }
        "gate" => CvAssign::Gate {
            track: track()?,
            volts: volts("volts")?,
            retrigger_ms: ms("retrigger-ms")?,
        },
        "trigger" => CvAssign::Trigger {
            track: track()?,
            note: get("note")
                .map(|n| number(n, 0, 127).map(|n| n as u8))
                .transpose()?,
            volts: volts("volts")?,
            ms: ms("ms")?,
        },
        "velocity" => CvAssign::Velocity {
            track: track()?,
            max_volts: volts("max-volts")?,
        },
        "cc" => CvAssign::Cc {
            track: track()?,
            cc: byte("cc")?,
            min_volts: volts("min-volts")?,
            max_volts: volts("max-volts")?,
        },
        "clock" => CvAssign::Clock {
            ppqn: number(need("ppqn")?, 1, 96)? as u16,
            volts: volts("volts")?,
            ms: ms("ms")?,
        },
        "run" => CvAssign::Run {
            volts: volts("volts")?,
        },
        "reset" => CvAssign::Reset {
            volts: volts("volts")?,
            ms: ms("ms")?,
        },
        "fixed" => CvAssign::Fixed {
            volts: volts("volts")?,
        },
        k => return Err(format!("unknown job \"{k}\"")),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn routes_to_tracks_read_back_and_belong_to_the_song() {
        let m = MidiSetup {
            routes: vec![
                TrackRoute {
                    port: DIN_NAME.to_string(),
                    channel: Some(9),
                    track: 0,
                },
                TrackRoute {
                    port: "Keystep 37".to_string(),
                    channel: None,
                    track: 127,
                },
            ],
            ..MidiSetup::default()
        };
        let text = m.to_text();
        assert!(text.contains("route = track 1 < MIDI · ch 10\n"), "{text}");
        assert!(
            text.contains("route = track 128 < Keystep 37 · all channels\n"),
            "{text}"
        );
        let (back, warn) = MidiSetup::parse(&text);
        assert!(warn.is_empty(), "{warn:?}");
        assert_eq!(back.routes, m.routes);
        assert_eq!(m.project().routes, m.routes);
        assert!(m.rig().routes.is_empty());
        let mut kept = m.clone();
        kept.set_rig(&MidiSetup::default());
        assert_eq!(kept.routes, m.routes);
        // Lines it can't read are named, and the rest kept.
        let (back, warn) = MidiSetup::parse(
            "mpfree-midi 1\nroute = track 0 < MIDI · ch 1\nroute = track 2 < MIDI · ch 17\n\
             route = track 2 MIDI\nroute = track 2 < MIDI · ch 3\nroute = track 2 < MIDI · ch 3\n",
        );
        assert_eq!(back.routes.len(), 1);
        assert_eq!(back.routes[0].source(), "MIDI · ch 3");
        assert_eq!(warn.len(), 4, "{warn:?}");
        assert!(warn[3].contains("the same route twice"), "{warn:?}");
    }

    #[test]
    fn the_project_and_the_rig_split_and_join_again() {
        let mut m = MidiSetup {
            tempo: TempoSource::Port(DIN_NAME.into()),
            follow_position: false,
            ..MidiSetup::default()
        };
        m.set_port(DIN_NAME, |p| {
            p.input = PortInput::LearnOnly;
            p.thru = true;
            p.clock_out = ClockOut::Always;
        });
        m.set_port("Keystep 37", |p| {
            p.input = PortInput::PlaysTracks;
            p.ignore = IgnoreSet::NONE.with(Ignored::Clock, true);
        });
        m.set_port("Digitone", |p| p.delay_tenths = 65);
        m.tracks[0] = Some(TrackOut::new(DIN_NAME, 9));
        let project = m.project();
        let rig = m.rig();
        // The song keeps the clock out, not the input, thru, what a port
        // ignores or its gear's delay.
        assert_eq!(project.port(DIN_NAME).clock_out, ClockOut::Always);
        assert_eq!(project.port(DIN_NAME).input, PortInput::default());
        assert!(!project.port(DIN_NAME).thru);
        assert!(project.ports.iter().all(|p| p.name != "Keystep 37"));
        assert!(project.ports.iter().all(|p| p.name != "Digitone"));
        assert_eq!(project.tracks[0], m.tracks[0]);
        // The rig keeps the inputs, thru, ignores and delays.
        assert_eq!(rig.port(DIN_NAME).input, PortInput::LearnOnly);
        assert!(rig.port(DIN_NAME).thru);
        assert!(rig.port("Keystep 37").ignore.has(Ignored::Clock));
        assert_eq!(rig.port("Digitone").delay_tenths, 65);
        assert_eq!(rig.port(DIN_NAME).clock_out, ClockOut::default());
        assert_eq!(rig.tempo, TempoSource::Internal);
        assert!(rig.tracks.iter().all(Option::is_none));
        // Both texts read back, and join to the whole.
        let (p2, w1) = MidiSetup::parse(&m.project_text());
        let (r2, w2) = MidiSetup::parse(&m.rig_text());
        assert!(w1.is_empty() && w2.is_empty(), "{w1:?} {w2:?}");
        let mut joined = r2.clone();
        joined.set_project(&p2);
        assert_eq!(joined.project(), m.project());
        assert_eq!(joined.rig(), m.rig());
        // A new project: the song back to defaults, the rig kept.
        let mut fresh = m.clone();
        fresh.set_project(&MidiSetup::default());
        assert_eq!(fresh.rig(), m.rig());
        assert_eq!(fresh.project(), MidiSetup::default());
    }

    use super::*;

    /// What a port ignores is a set of kinds, read back as written; each
    /// channel message is the kind it is.
    #[test]
    fn a_port_ignores_the_kinds_set_and_says_them() {
        let set = IgnoreSet::NONE
            .with(Ignored::AllNotesOff, true)
            .with(Ignored::Aftertouch, true);
        assert_eq!(set.text(), "aftertouch,all-notes-off");
        assert_eq!(IgnoreSet::parse(&set.text()), Ok(set));
        assert_eq!(IgnoreSet::parse(""), Ok(IgnoreSet::NONE));
        assert!(IgnoreSet::parse("clock,tempo").is_err());
        assert!(!set
            .with(Ignored::Aftertouch, false)
            .has(Ignored::Aftertouch));
        let pressure = Msg::ChanPressure { ch: 0, value: 9 };
        let poly = Msg::PolyPressure {
            ch: 0,
            note: 60,
            value: 9,
        };
        let all_off = Msg::Cc {
            ch: 3,
            cc: 123,
            value: 0,
        };
        let sound_off = Msg::Cc {
            ch: 3,
            cc: 120,
            value: 0,
        };
        let mod_wheel = Msg::Cc {
            ch: 3,
            cc: 1,
            value: 0,
        };
        let reset = Msg::Cc {
            ch: 3,
            cc: 121,
            value: 0,
        };
        let program = Msg::Program { ch: 0, program: 4 };
        for m in [pressure, poly, all_off, sound_off] {
            assert!(set.msg(&m), "{m:?}");
        }
        for m in [mod_wheel, reset, program] {
            assert!(!set.msg(&m), "{m:?}");
        }
        assert_eq!(Ignored::of(&program), Some(Ignored::Program));
        assert!((124..=127).all(ends_notes) && !ends_notes(121) && !ends_notes(122));
    }

    /// The gear's delay is written in ms and read back; one out of range
    /// is named and skipped.
    #[test]
    fn a_ports_delay_is_kept_in_ms() {
        let mut m = MidiSetup::default();
        m.set_port(DIN_NAME, |p| p.delay_tenths = 65);
        let text = m.rig_text();
        assert!(text.contains("port.1.delay-ms = 6.5\n"), "{text}");
        let (back, w) = MidiSetup::parse(&text);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(back.port(DIN_NAME).delay_tenths, 65);
        assert_eq!(back.port(DIN_NAME).delay_ms(), 6.5);
        let (_, w) = MidiSetup::parse(&format!("{HEADER}\nport.1 = MIDI\nport.1.delay-ms = 250\n"));
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("outside 0 to 100"), "{w:?}");
    }

    fn busy() -> MidiSetup {
        let mut m = MidiSetup {
            tempo: TempoSource::Port("Digitone = 2".into()),
            follow_position: false,
            ..MidiSetup::default()
        };
        m.set_port(DIN_NAME, |p| {
            p.clock_out = ClockOut::WhilePlaying;
            p.thru = true;
        });
        m.set_port("Keystep 37", |p| {
            p.input = PortInput::LearnOnly;
            p.ignore = IgnoreSet::NONE
                .with(Ignored::Program, true)
                .with(Ignored::AllNotesOff, true);
            p.delay_tenths = 120;
        });
        m.tracks[0] = Some(TrackOut::new(DIN_NAME, 9));
        m.tracks[15] = Some(TrackOut {
            program: Some(127),
            bank: Some(300),
            ..TrackOut::new("Keystep 37", 0)
        });
        m.cv[0] = CvAssign::Pitch {
            track: 1,
            scale: Scale::VoltsPerOctave(1.0),
            zero_note: 36,
            bend_semis: 2,
            priority: Priority::Low,
        };
        m.cv[1] = CvAssign::Gate {
            track: 1,
            volts: 5.0,
            retrigger_ms: 2.5,
        };
        m.cv[2] = CvAssign::Trigger {
            track: 9,
            note: Some(36),
            volts: 5.0,
            ms: 10.0,
        };
        m.cv[3] = CvAssign::Velocity {
            track: 2,
            max_volts: 4.0,
        };
        m.cv[4] = CvAssign::Cc {
            track: 3,
            cc: 74,
            min_volts: 0.0,
            max_volts: 5.0,
        };
        m.cv[5] = CvAssign::Clock {
            ppqn: 4,
            volts: 5.0,
            ms: 5.0,
        };
        m.cv[6] = CvAssign::Pitch {
            track: 4,
            scale: Scale::HzPerVolt {
                ref_volts: 1.0,
                ref_note: 48,
            },
            zero_note: 0,
            bend_semis: 12,
            priority: Priority::High,
        };
        m.cv[7] = CvAssign::Fixed { volts: 2.5 };
        m.learned(learned(Some("Keystep 37"), 74));
        m.learned(learned(None, 1));
        m
    }

    fn learned(port: Option<&str>, cc: u8) -> Saved {
        use crate::learn::{Mode, Source, Takeover};
        use mpfree_param::{OnTrack, ParamAddr};
        Saved {
            port: port.map(String::from),
            channel: 0,
            source: Source::Cc(cc),
            target: ParamAddr::new(OnTrack::Inst.on(2), 7),
            lo: 0.0,
            hi: 1.0,
            mode: Mode::Absolute,
            takeover: Takeover::Jump,
        }
    }

    /// Learning a control again replaces what it moved; a line that isn't
    /// a learned control is named in a warning.
    #[test]
    fn a_control_learned_again_replaces_the_old_one() {
        let mut m = MidiSetup::default();
        m.learned(learned(Some("Keystep 37"), 74));
        let mut again = learned(Some("Keystep 37"), 74);
        again.lo = 0.5;
        m.learned(again.clone());
        m.learned(learned(Some("Other"), 74));
        assert_eq!(m.learn.len(), 2);
        assert_eq!(m.learn[0], again);
        let (_, w) = MidiSetup::parse("mpfree-midi 1\nlearn = nonsense\n");
        assert_eq!(w, ["line 2: learn: not a learned control"]);
    }

    /// A control learned for every project is the rig's: a project
    /// doesn't save it, opening one keeps it, and the song's own mapping
    /// for the same control wins while the song is open.
    #[test]
    fn controls_learned_for_every_project_are_the_rigs() {
        let mut m = MidiSetup::default();
        let tempo = learned(Some("MIDI"), 20);
        assert!(m.learned_in(Kept::Rig, tempo.clone()));
        m.learned(learned(Some("Keystep 37"), 74));
        assert!(!m.project_text().contains("learn-rig"));
        assert!(m
            .rig_text()
            .contains("learn-rig = port MIDI | ch 1 | cc 20"));
        let (back, w) = MidiSetup::parse(&m.rig_text());
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(back.rig_learn, std::slice::from_ref(&tempo));
        // Opening another project keeps it.
        m.set_project(&MidiSetup::default());
        assert!(m.learn.is_empty());
        assert_eq!(m.rig_learn, std::slice::from_ref(&tempo));
        // The song's mapping for the same control wins.
        let mut song = tempo.clone();
        song.lo = 0.25;
        m.learned(song.clone());
        assert_eq!(m.in_use(), [song.clone()]);
        assert_eq!(m.all_learned().count(), 2);
        // Moved to the rig, it replaces the rig's.
        assert_eq!(m.keep_at(0, Kept::Rig), Some(0));
        assert_eq!(
            (m.learn.len(), m.rig_learn.clone()),
            (0, vec![song.clone()])
        );
        // And back to the song.
        assert_eq!(m.keep_at(0, Kept::Project), Some(0));
        assert_eq!((m.learn.clone(), m.rig_learn.len()), (vec![song], 0));
        assert!(m.forget_at(0).is_some());
        assert!(m.forget_at(0).is_none());
    }

    #[test]
    fn a_new_project_writes_only_the_header() {
        assert_eq!(MidiSetup::default().to_text(), "mpfree-midi 1\n");
        let (m, w) = MidiSetup::parse("mpfree-midi 1\n");
        assert_eq!(m, MidiSetup::default());
        assert!(w.is_empty());
    }

    #[test]
    fn every_setting_reads_back_the_same() {
        let m = busy();
        let text = m.to_text();
        let (back, warn) = MidiSetup::parse(&text);
        assert!(warn.is_empty(), "{warn:?}\n{text}");
        assert_eq!(back, m, "{text}");
        assert!(text.contains("track.1.channel = 10\n"), "{text}");
        assert!(text.contains("track.16.program = 128\n"), "{text}");
        assert!(
            text.contains("cv.1 = pitch track=2 zero=36 bend=2 priority=low v/oct=1\n"),
            "{text}"
        );
        assert!(text.contains("tempo.port = Digitone = 2\n"), "{text}");
    }

    #[test]
    fn rest_of_the_line_is_a_port_name() {
        let (m, w) =
            MidiSetup::parse("mpfree-midi 1\nport.1 = My Synth (USB) = 3\nport.1.thru = on\n");
        assert!(w.is_empty(), "{w:?}");
        assert!(m.port("My Synth (USB) = 3").thru);
    }

    #[test]
    fn a_port_back_at_its_defaults_is_dropped() {
        let mut m = MidiSetup::default();
        m.set_port(DIN_NAME, |p| p.thru = true);
        assert_eq!(m.ports.len(), 1);
        m.set_port(DIN_NAME, |p| p.thru = false);
        assert!(m.ports.is_empty());
        assert_eq!(m.port("anything"), PortSetup::new("anything"));
    }

    #[test]
    fn a_bad_line_is_skipped_and_named() {
        let text = "mpfree-midi 1\n\
                    track.3.channel = 17\n\
                    track.4.port = MIDI\n\
                    track.4.channel = 2\n\
                    cv.2 = gate track=1 volts=9 retrigger-ms=0\n\
                    cv.3 = wobble\n\
                    nonsense\n\
                    port.2.input = loud\n\
                    follow.position = maybe\n";
        let (m, w) = MidiSetup::parse(text);
        assert_eq!(m.tracks[3], Some(TrackOut::new(DIN_NAME, 1)));
        assert_eq!(m.cv[1], CvAssign::Off);
        assert!(m.follow_position);
        let all = w.join("\n");
        for want in [
            "line 2: track.3.channel: \"17\" is not a number from 1 to 16",
            "track 3: a channel but no port",
            "line 5: cv.2: volts: 9 is outside 0 to 5 V",
            "line 6: cv.3: unknown job \"wobble\"",
            "line 7: not \"key = value\"",
            "line 8: port.2.input: unknown input \"loud\"",
            "line 9: follow.position: expected on or off",
            "a port with settings but no name",
        ] {
            assert!(all.contains(want), "missing {want:?} in\n{all}");
        }
    }

    #[test]
    fn a_wrong_header_is_named_but_the_rest_is_read() {
        let (m, w) = MidiSetup::parse("mpfree-midi 2\ntrack.1.port = MIDI\ntrack.1.channel = 1\n");
        assert_eq!(m.tracks[0], Some(TrackOut::new(DIN_NAME, 0)));
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("expected \"mpfree-midi 1\""));
    }

    #[test]
    fn the_same_port_twice_keeps_the_first() {
        let (m, w) = MidiSetup::parse(
            "mpfree-midi 1\nport.1 = MIDI\nport.1.thru = on\nport.2 = MIDI\nport.2.clock-out = always\n",
        );
        assert!(m.port(DIN_NAME).thru);
        assert_eq!(m.port(DIN_NAME).clock_out, ClockOut::Off);
        assert!(w[0].contains("set twice"));
    }

    #[test]
    fn labels_say_what_they_do() {
        assert_eq!(TrackOut::new(DIN_NAME, 9).label(), "MIDI · ch 10");
        assert_eq!(PortInput::default().label(), "Both");
        assert_eq!(ClockOut::default().label(), "Off");
        let names: Vec<_> = PortInput::ALL.iter().map(|p| p.label()).collect();
        assert_eq!(names, ["Plays tracks", "Learn only", "Both"]);
    }
}
