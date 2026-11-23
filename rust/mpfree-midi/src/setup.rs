//! A project's MIDI, sync and CV settings: what the mixer's I/O view edits
//! and the project saves as its `midi` part (`parts/midi.txt`).
//!
//! The owner's calls (2026-11-12): MIDI and sync are project settings, not
//! Prefs, because they change from project to project. Facts about the rig
//! rather than the project are kept apart ([`MidiSetup::rig`], saved by the
//! app for every project): each port's input, what it ignores coming in and
//! the delay of the gear on it.
//!
//! - **Ports are named, not numbered.** A project says "Digitone" or
//!   [`DIN_NAME`]; the app finds the port by that name when the device is
//!   plugged in ([`crate::port::Ports`] ids only last while the app runs). A
//!   setting for a device that isn't plugged in is kept, and the screen says
//!   so in place.
//! - **One output setting per track**: the port and the channel together
//!   ([`TrackOut`]), never two look-alike fields (the owner's rule).
//! - **Defaults** (agreed 2026-11-12): clock out Off on every port, no
//!   thru, a port's input plays tracks and can be learned
//!   ([`PortInput::Both`]), the tempo is MPFree's own.
//! - **Thru** ([`ThruRoute`], the Routing section's squares from a port to
//!   an output) is a route, saved with the song as the routes to tracks
//!   are (board 5 of the MIDI mockups, approved 16 Nov 2026). Before it
//!   was built it was a switch on each port, kept with the rig, which did
//!   nothing: an old `port.N.thru` line is read and dropped.
//! - **Text** is `key = value` lines, like every other part. A line that
//!   can't be read is skipped and named in the warnings that [`MidiSetup::parse`]
//!   returns, for the app to log and show: a project never fails to open
//!   over its MIDI part, and nothing is dropped silently.

use crate::blocks::Blocks;
use crate::cv::{CvAssign, Priority, Scale, CV_OUTS};
use crate::cv_mod::{self, Env, Lfo, Notes, Rate, Shape};
use crate::learn::Saved;
use crate::msg::{Channel, Msg};
use crate::output::MAX_TRACKS;

/// The 5-pin MIDI jacks' name in a project.
pub const DIN_NAME: &str = "MIDI";

/// What the screens call the 5-pin jacks, as the MPC's panel does. They
/// can't be renamed, and no other port can take these names.
pub const DIN_SHOWN: [&str; 2] = ["MIDI In", "MIDI Out"];

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

/// What a program change coming in does (the Sync section's "Program
/// change in", as the MPC's Sync settings' "Program Change" does with a
/// sequence).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProgramIn {
    /// Nothing: it shows in the MIDI monitor, and thru passes it on.
    #[default]
    Off,
    /// Program N (1 to 128) launches scene N, the Clips grid's row N, as
    /// a tap on the scene's launch button does.
    Scene,
}

impl ProgramIn {
    pub const ALL: [ProgramIn; 2] = [ProgramIn::Off, ProgramIn::Scene];

    pub fn label(self) -> &'static str {
        match self {
            ProgramIn::Off => "Off",
            ProgramIn::Scene => "Launches a scene",
        }
    }

    fn word(self) -> &'static str {
        match self {
            ProgramIn::Off => "off",
            ProgramIn::Scene => "scene",
        }
    }

    fn from_word(w: &str) -> Option<ProgramIn> {
        ProgramIn::ALL.into_iter().find(|c| c.word() == w)
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

/// The most a port's clock can be divided by ([`PortSetup::clock_divide`]).
pub const MAX_CLOCK_DIVIDE: u8 = 8;
/// The most a port's clock is nudged either way, in tenths of a ms
/// (20 ms).
pub const MAX_NUDGE_TENTHS: i16 = 200;
/// The port page's - and + step for the nudge, in tenths of a ms.
pub const NUDGE_STEP_TENTHS: i16 = 5;

/// The most characters a port's shown name ([`PortSetup::shown`]) has.
pub const MAX_SHOWN: usize = 24;

/// One port's settings. A port with none of its own uses
/// [`PortSetup::new`]'s defaults. Its clock out is the project's; the rest
/// is the rig's ([`MidiSetup::rig`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortSetup {
    pub name: String,
    pub input: PortInput,
    pub clock_out: ClockOut,
    /// Its clock runs this many times slower (1 to [`MAX_CLOCK_DIVIDE`]):
    /// 2 plays the gear at half the tempo. The song's, as `clock_out`.
    pub clock_divide: u8,
    /// Its clock leaves this much later (or earlier, below 0), in tenths
    /// of a ms, on top of `delay_tenths`: for gear whose clock input lags
    /// or leads its notes. The rig's, as the delay.
    pub clock_nudge_tenths: i16,
    /// What its input ignores.
    pub ignore: IgnoreSet,
    /// How late the gear on this port sounds after a message leaves, in
    /// tenths of a ms (its MIDI and its own processing): MIDI out sends
    /// this much early, so it lands on the beat. 0 to [`MAX_DELAY_TENTHS`].
    pub delay_tenths: u16,
    /// The name the musician gave it on its page (board 6's Name: "Juno"
    /// for a "USB MIDI Interface"), shown on every screen in place of
    /// `name`; empty shows `name`. `name` stays what the project's routes,
    /// tracks and learned controls use, so a renamed port keeps them. The
    /// rig's: the same in every project.
    pub shown: String,
    /// Which device profile names its controls (board 6's Profile): the
    /// rig's.
    pub profile: ProfilePick,
    /// What its messages go through on their way to the tracks (the
    /// Routing section's Armed track square, board 5): the song's, as the
    /// routes' blocks.
    pub blocks: Blocks,
}

/// A port's device profile ([`crate::profile`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ProfilePick {
    /// The first profile made for the name the port calls itself, if any.
    #[default]
    Auto,
    /// None: only the controllers most gear agrees on are named.
    Off,
    /// The profile with this name (none while there is no such profile).
    Named(String),
}

impl ProfilePick {
    /// The word saved for it: `auto`, `none` or the profile's name.
    pub fn word(&self) -> &str {
        match self {
            ProfilePick::Auto => "auto",
            ProfilePick::Off => "none",
            ProfilePick::Named(n) => n,
        }
    }

    /// Reads [`ProfilePick::word`] back. A name too long for a profile is
    /// refused.
    pub fn from_word(v: &str) -> Result<ProfilePick, String> {
        if v.eq_ignore_ascii_case("auto") {
            return Ok(ProfilePick::Auto);
        }
        if v.eq_ignore_ascii_case("none") {
            return Ok(ProfilePick::Off);
        }
        if v.is_empty() || v.chars().count() > crate::profile::MAX_NAME {
            return Err(format!("\"{v}\" is not a profile's name"));
        }
        Ok(ProfilePick::Named(v.to_string()))
    }
}

impl PortSetup {
    pub fn new(name: &str) -> PortSetup {
        PortSetup {
            name: name.to_string(),
            input: PortInput::default(),
            clock_out: ClockOut::default(),
            clock_divide: 1,
            clock_nudge_tenths: 0,
            ignore: IgnoreSet::NONE,
            delay_tenths: 0,
            shown: String::new(),
            profile: ProfilePick::Auto,
            blocks: Blocks::NONE,
        }
    }

    /// The delay in ms.
    pub fn delay_ms(&self) -> f32 {
        f32::from(self.delay_tenths) / 10.0
    }

    /// The clock's nudge in ms (later above 0).
    pub fn nudge_ms(&self) -> f32 {
        f32::from(self.clock_nudge_tenths) / 10.0
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

/// The most synths the routes may name at once (the instrument redesign's
/// step 11e): each plays on a route row of the app's instrument host.
pub const ROUTE_SYNTHS: usize = 4;

/// A port and channel that play one track, whichever track is armed: a
/// route to "Tracks by channel" in the I/O view's Routing. Saved with the
/// project, as `route = track 1 < MIDI · ch 10` (or `· all channels`); one
/// that plays a synth no track has (the instrument redesign's step 11e) as
/// `route = instrument 4 < MIDI · ch 10`, which a build from before reads
/// as a line it can't, said and left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackRoute {
    /// The port, by name, as [`PortSetup::name`].
    pub port: String,
    /// None: every channel.
    pub channel: Option<Channel>,
    /// The track, from 0, below [`ROUTE_TRACKS`]. 0 (and not used) while
    /// it plays a synth (`synth`).
    pub track: u8,
    /// The synth it plays instead of a track (step 11e): the number of one
    /// of the project's instruments (the app's `InstrumentId`), a synth no
    /// track has, such as one picked in the list of synths. None: it plays
    /// `track`.
    pub synth: Option<u32>,
    /// What it does to what it takes, in order (board 5), saved on the
    /// line after it as `route.blocks = transpose -12; velocity soft 20 127`.
    pub blocks: Blocks,
}

impl TrackRoute {
    /// "MIDI · ch 10" or "Keystep 37 · all channels".
    pub fn source(&self) -> String {
        match self.channel {
            Some(c) => format!("{} · ch {}", self.port, c + 1),
            None => format!("{} · all channels", self.port),
        }
    }

    /// [`TrackRoute::source`] with the port by its name on screen
    /// ([`MidiSetup::shown`]).
    pub fn source_in(&self, setup: &MidiSetup) -> String {
        let port = setup.shown(&self.port);
        match self.channel {
            Some(c) => format!("{port} · ch {}", c + 1),
            None => format!("{port} · all channels"),
        }
    }

    fn text(&self) -> String {
        match self.synth {
            Some(n) => format!("instrument {n} < {}", self.source()),
            None => format!("track {} < {}", u16::from(self.track) + 1, self.source()),
        }
    }

    /// The same port, channel and track or synth (whatever its blocks).
    pub fn same_route(&self, other: &TrackRoute) -> bool {
        self.port == other.port
            && self.channel == other.channel
            && self.track == other.track
            && self.synth == other.synth
    }

    fn parse(v: &str) -> Result<TrackRoute, String> {
        let shape = || "not \"track N < port · ch N\"".to_string();
        let (t, from) = v.split_once(" < ").ok_or_else(shape)?;
        let t = t.trim();
        let (track, synth) = match t.strip_prefix("instrument ") {
            Some(n) => (0, Some(number(n.trim(), 1, u32::MAX)?)),
            None => {
                let t = t.strip_prefix("track ").ok_or_else(shape)?;
                ((number(t.trim(), 1, ROUTE_TRACKS as u32)? - 1) as u8, None)
            }
        };
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
            synth,
            blocks: Blocks::NONE,
        })
    }
}

/// The most routes a project keeps (`mpfree_midi::input::MAX_ROUTES`).
pub const MAX_TRACK_ROUTES: usize = crate::input::MAX_ROUTES;

/// The most thru routes a project keeps (`mpfree_midi::input::MAX_THRU`).
pub const MAX_THRU_ROUTES: usize = crate::input::MAX_THRU;

/// A port's messages straight on to an output, as they came: the Routing
/// section's square from a port to a MIDI out port. Saved with the
/// project, as `thru = MIDI < Keystep 37 · ch 10` (or `· all channels`).
/// Channel messages only: never clock (each port's clock out is the Sync
/// section's) or SysEx.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThruRoute {
    /// The port it comes in on, by name.
    pub from: String,
    /// None: every channel.
    pub channel: Option<Channel>,
    /// The port it leaves by, by name.
    pub to: String,
    /// What it does to what it sends, in order (board 5), saved on the
    /// line after it as `thru.blocks = channel 2`.
    pub blocks: Blocks,
}

impl ThruRoute {
    /// "Keystep 37 · ch 10" or "MIDI · all channels".
    pub fn source(&self) -> String {
        match self.channel {
            Some(c) => format!("{} · ch {}", self.from, c + 1),
            None => format!("{} · all channels", self.from),
        }
    }

    /// [`ThruRoute::source`] with the port by its name on screen
    /// ([`MidiSetup::shown`]).
    pub fn source_in(&self, setup: &MidiSetup) -> String {
        let port = setup.shown(&self.from);
        match self.channel {
            Some(c) => format!("{port} · ch {}", c + 1),
            None => format!("{port} · all channels"),
        }
    }

    fn text(&self) -> String {
        format!("{} < {}", self.to, self.source())
    }

    /// The same ports and channel (whatever its blocks).
    pub fn same_route(&self, other: &ThruRoute) -> bool {
        self.from == other.from && self.channel == other.channel && self.to == other.to
    }

    fn parse(v: &str) -> Result<ThruRoute, String> {
        let shape = || "not \"port < port · ch N\"".to_string();
        let (to, from) = v.split_once(" < ").ok_or_else(shape)?;
        let (from, ch) = from.rsplit_once(" · ").ok_or_else(shape)?;
        let channel = match ch.trim() {
            "all channels" => None,
            c => Some(
                (number(c.strip_prefix("ch ").ok_or_else(shape)?.trim(), 1, 16)? - 1) as Channel,
            ),
        };
        let (from, to) = (from.trim(), to.trim());
        if from.is_empty() || to.is_empty() {
            return Err(shape());
        }
        let r = ThruRoute {
            from: from.to_string(),
            channel,
            to: to.to_string(),
            blocks: Blocks::NONE,
        };
        r.check()?;
        Ok(r)
    }

    /// Refused: a USB device's own messages back to it (the 5-pin jacks
    /// are two pieces of gear, so MIDI in to MIDI out is fine).
    pub fn check(&self) -> Result<(), String> {
        if self.from == self.to && self.from != DIN_NAME {
            return Err(format!(
                "thru from {} back to itself would send its own messages back to it",
                self.from
            ));
        }
        Ok(())
    }
}

/// Whether a thru from `from` to `to` can be set ([`ThruRoute::check`]).
pub fn thru_allowed(from: &str, to: &str) -> bool {
    from != to || from == DIN_NAME
}

/// The most knobs MIDI Follow turns: the MPC's four Q-Links.
pub const FOLLOW_KNOBS: usize = 4;

/// MIDI Follow (board 8 of the MIDI mockups): one controller's knobs turn
/// what the MPC's Q-Links turn on the screen showing, as a Q-Link turned
/// by as much as the knob moved. Its keys play the armed track as any
/// port's do, and a control learned with MIDI Learn wins over it. The
/// rig's: the controller is on the desk, whatever the song.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FollowSetup {
    pub on: bool,
    /// The controller's port, by its own name; empty: none picked yet.
    pub port: String,
    /// The CCs its knobs send, in Q-Link order (at most
    /// [`FOLLOW_KNOBS`]); empty: its profile's `knobs`.
    pub knobs: Vec<u8>,
    /// Send values back (plan item 3.5, board 8): each knob's CC goes
    /// back to the controller with where its Q-Link's value is, so its
    /// lights or motor faders follow the screen. Off unless asked for.
    pub send_back: bool,
}

impl FollowSetup {
    /// The CCs that turn the Q-Links: its own, else `profile`'s (the
    /// controller port's profile), at most [`FOLLOW_KNOBS`].
    pub fn knobs_of(&self, profile: Option<&crate::profile::Profile>) -> Vec<u8> {
        let k = if self.knobs.is_empty() {
            profile.map_or(&[][..], |p| p.knobs.as_slice())
        } else {
            &self.knobs
        };
        k.iter().take(FOLLOW_KNOBS).copied().collect()
    }

    /// Whether `port` (its own name) is the controller it follows now.
    pub fn follows(&self, port: &str) -> bool {
        self.on && !self.port.is_empty() && self.port == port
    }
}

/// The Q-Links controller mode sends: four banks of the MPC's four.
pub const CONTROLLER_KNOBS: usize = 16;
/// The CCs they send until set: 16 to 31 (general purpose and undefined
/// in the MIDI spec, so they move nothing a synth fixes).
pub const CONTROLLER_CCS: [u8; CONTROLLER_KNOBS] = [
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
];

/// The notes controller mode's pads play.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PadNotes {
    /// A kit's notes: the port's profile's pad lines, else MPFree's own
    /// ([`crate::profile::kit_note`]: bank A the MPC's pad notes).
    #[default]
    Kit,
    /// One note after another from this one: A1 plays it, A2 the next,
    /// on through the banks (notes past 127 play nothing).
    From(u8),
}

/// What controller mode sends for a pad's pressure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PadPressure {
    #[default]
    Off,
    /// Polyphonic aftertouch, on the pad's note.
    Poly,
    /// Channel pressure: the hardest pad held.
    Channel,
}

impl PadPressure {
    pub const ALL: [PadPressure; 3] = [PadPressure::Off, PadPressure::Poly, PadPressure::Channel];

    pub fn word(self) -> &'static str {
        match self {
            PadPressure::Off => "off",
            PadPressure::Poly => "poly",
            PadPressure::Channel => "channel",
        }
    }
}

/// Controller mode (powerhouse 3.10, Jay's card 18 Nov 01:21Z: a
/// Controller switch in I/O): the MPC's pads send notes and its Q-Links
/// CCs out of one port, and MPFree's sounds don't take the pads while it
/// is on. The rig's: the gear is on the desk, whatever the song. `on` is
/// never saved, so MPFree always starts with the pads its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControllerSetup {
    pub on: bool,
    /// The output, by its own name; empty: none picked yet.
    pub port: String,
    pub channel: Channel,
    pub pads: PadNotes,
    pub pressure: PadPressure,
    /// The CC each Q-Link sends, bank by bank (Q-Links 1 to 4, then 5 to
    /// 8 ...).
    pub ccs: [u8; CONTROLLER_KNOBS],
}

impl Default for ControllerSetup {
    fn default() -> Self {
        ControllerSetup {
            on: false,
            port: String::new(),
            channel: 0,
            pads: PadNotes::Kit,
            pressure: PadPressure::Off,
            ccs: CONTROLLER_CCS,
        }
    }
}

impl ControllerSetup {
    /// What pad `pad` (0 is A1, to 127) sends: its note and channel, by
    /// the port's `profile` for a kit's notes. None past the notes.
    pub fn pad_out(
        &self,
        pad: usize,
        profile: Option<&crate::profile::Profile>,
    ) -> Option<(u8, Channel)> {
        match self.pads {
            PadNotes::Kit => match profile.and_then(|p| p.pad(pad)) {
                Some(o) => Some((o.note, o.channel.unwrap_or(self.channel))),
                None => crate::profile::kit_note(pad).map(|n| (n, self.channel)),
            },
            PadNotes::From(n) => u8::try_from(usize::from(n) + pad)
                .ok()
                .filter(|n| *n <= 127)
                .map(|n| (n, self.channel)),
        }
    }
}

/// A project's MIDI, sync and CV settings.
#[derive(Clone, Debug, PartialEq)]
pub struct MidiSetup {
    pub tempo: TempoSource,
    /// While following another device: its start and stop start and stop
    /// MPFree.
    pub follow_start_stop: bool,
    /// While following: its song position moves MPFree's.
    pub follow_position: bool,
    /// Other gear's MIDI Machine Control moves the transport, from any
    /// port: Play, Stop, Pause, Record Strobe and Locate ([`crate::mmc`]).
    pub mmc_in: bool,
    /// What a program change coming in does, from any port that doesn't
    /// ignore program changes.
    pub program_in: ProgramIn,
    /// Ports with settings of their own, by name, in the order first set.
    pub ports: Vec<PortSetup>,
    /// Each track's MIDI output; `None` plays nothing out.
    pub tracks: [Option<TrackOut>; MAX_TRACKS],
    /// Ports and channels that play a track whichever track is armed, in
    /// the order set; at most [`MAX_TRACK_ROUTES`].
    pub routes: Vec<TrackRoute>,
    /// Ports whose messages go straight on to an output, in the order
    /// set; at most [`MAX_THRU_ROUTES`].
    pub thru: Vec<ThruRoute>,
    /// What each CV/gate output does.
    pub cv: [CvAssign; CV_OUTS],
    /// Controls learned with MIDI Learn, each with its device by name, in
    /// the order learned: the song's.
    pub learn: Vec<Saved>,
    /// Controls learned for every project (kept with the rig, as a port's
    /// input is): a controller's transport keys or a tempo fader. The
    /// song's own mapping for the same control wins ([`MidiSetup::in_use`]).
    pub rig_learn: Vec<Saved>,
    /// MIDI Follow: the rig's.
    pub follow: FollowSetup,
    /// Controller mode: the rig's.
    pub controller: ControllerSetup,
}

impl Default for MidiSetup {
    fn default() -> Self {
        MidiSetup {
            tempo: TempoSource::Internal,
            follow_start_stop: true,
            follow_position: true,
            mmc_in: false,
            program_in: ProgramIn::Off,
            ports: Vec::new(),
            tracks: std::array::from_fn(|_| None),
            routes: Vec::new(),
            thru: Vec::new(),
            cv: [CvAssign::Off; CV_OUTS],
            learn: Vec::new(),
            rig_learn: Vec::new(),
            follow: FollowSetup::default(),
            controller: ControllerSetup::default(),
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

    /// The name the screens show for the port named `name`: the one the
    /// musician gave it ([`PortSetup::shown`]), else `name`.
    pub fn shown<'a>(&'a self, name: &'a str) -> &'a str {
        self.ports
            .iter()
            .find(|p| p.name == name && !p.shown.is_empty())
            .map_or(name, |p| p.shown.as_str())
    }

    /// The port page's Name: the port named `name` shows as `typed`
    /// (trimmed) from now on, in every project. Its own name, or nothing,
    /// takes the given one off. Refused, with why, when it is longer than
    /// [`MAX_SHOWN`] or another port is called that (by its own name or
    /// a given one, whatever the capitals, or the 5-pin jacks'
    /// [`DIN_SHOWN`]), so no two ports look alike. `known` are the ports
    /// plugged in now, which may have no settings.
    pub fn set_shown<'a>(
        &mut self,
        name: &str,
        typed: &str,
        known: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), String> {
        let typed = typed.trim();
        if typed.chars().count() > MAX_SHOWN {
            return Err(format!("A port's name has at most {MAX_SHOWN} characters"));
        }
        let shown = if typed.is_empty() || typed == name {
            ""
        } else {
            typed
        };
        let same = |other: &str| other.to_lowercase() == typed.to_lowercase();
        let others: Vec<String> = self
            .ports
            .iter()
            .map(|p| p.name.clone())
            .chain(known.into_iter().map(str::to_string))
            .chain([DIN_NAME].into_iter().chain(DIN_SHOWN).map(str::to_string))
            .filter(|n| n != name)
            .collect();
        if others.iter().any(|o| same(o) || same(self.shown(o))) {
            return Err(format!("Another port is called {typed}: pick another name"));
        }
        self.set_port(name, |p| p.shown = shown.to_string());
        Ok(())
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
    /// part: tracks' outputs, the routes to tracks and thru, Tempo from and
    /// following, each port's clock out, the CV jobs and learned controls.
    /// Each port's input, what it ignores, its delay, its shown name and
    /// its profile are the rig's ([`MidiSetup::rig`]) and are left at
    /// their defaults.
    pub fn project(&self) -> MidiSetup {
        let mut m = self.clone();
        m.rig_learn.clear();
        m.follow = FollowSetup::default();
        m.controller = ControllerSetup::default();
        for name in self
            .ports
            .iter()
            .map(|p| p.name.clone())
            .collect::<Vec<_>>()
        {
            m.set_port(&name, |p| {
                p.input = PortInput::default();
                p.ignore = IgnoreSet::NONE;
                p.delay_tenths = 0;
                p.clock_nudge_tenths = 0;
                p.shown.clear();
                p.profile = ProfilePick::Auto;
            });
        }
        m
    }

    /// The settings that belong to the gear on the desk, kept for every
    /// project (owner, 13 Nov 2026: project settings apart from system-wide
    /// ones): each port's input (Plays tracks, Learn only, Both), what it
    /// ignores coming in, its gear's delay, its shown name and its
    /// profile, and the controls learned for every project.
    pub fn rig(&self) -> MidiSetup {
        let mut m = MidiSetup {
            rig_learn: self.rig_learn.clone(),
            follow: self.follow.clone(),
            controller: self.controller.clone(),
            ..MidiSetup::default()
        };
        for p in &self.ports {
            m.set_port(&p.name, |q| {
                q.input = p.input;
                q.ignore = p.ignore;
                q.delay_tenths = p.delay_tenths;
                q.clock_nudge_tenths = p.clock_nudge_tenths;
                q.shown = p.shown.clone();
                q.profile = p.profile.clone();
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

    /// `rig`'s port settings (input, ignore, delay, shown name, profile)
    /// and controls learned for every project, this setup's song settings
    /// kept.
    pub fn set_rig(&mut self, rig: &MidiSetup) {
        self.rig_learn = rig.rig_learn.clone();
        self.follow = rig.follow.clone();
        self.controller = rig.controller.clone();
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
                p.ignore = want.ignore;
                p.delay_tenths = want.delay_tenths;
                p.clock_nudge_tenths = want.clock_nudge_tenths;
                p.shown = want.shown;
                p.profile = want.profile;
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
        if self.mmc_in != d.mmc_in {
            line(&mut s, "mmc.in", on_off(self.mmc_in));
        }
        if self.program_in != d.program_in {
            line(&mut s, "program-change.in", self.program_in.word());
        }
        for (i, p) in self.ports.iter().enumerate() {
            let k = format!("port.{}", i + 1);
            line(&mut s, &k, &p.name);
            line(&mut s, &format!("{k}.input"), p.input.word());
            line(&mut s, &format!("{k}.clock-out"), p.clock_out.word());
            if p.clock_divide != 1 {
                line(
                    &mut s,
                    &format!("{k}.clock-divide"),
                    &p.clock_divide.to_string(),
                );
            }
            if p.clock_nudge_tenths != 0 {
                line(
                    &mut s,
                    &format!("{k}.clock-nudge-ms"),
                    &format!("{:+.1}", p.nudge_ms()),
                );
            }
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
            if !p.shown.is_empty() {
                line(&mut s, &format!("{k}.shown"), &p.shown);
            }
            if p.profile != ProfilePick::Auto {
                line(&mut s, &format!("{k}.profile"), p.profile.word());
            }
            if !p.blocks.is_empty() {
                line(&mut s, &format!("{k}.blocks"), &p.blocks.to_text());
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
            if !r.blocks.is_empty() {
                line(&mut s, "route.blocks", &r.blocks.to_text());
            }
        }
        for r in &self.thru {
            line(&mut s, "thru", &r.text());
            if !r.blocks.is_empty() {
                line(&mut s, "thru.blocks", &r.blocks.to_text());
            }
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
        let f = &self.follow;
        if f.on {
            line(&mut s, "midi-follow", "on");
        }
        if !f.port.is_empty() {
            line(&mut s, "midi-follow.port", &f.port);
        }
        if !f.knobs.is_empty() {
            let k: Vec<String> = f.knobs.iter().map(u8::to_string).collect();
            line(&mut s, "midi-follow.knobs", &k.join(" "));
        }
        if f.send_back {
            line(&mut s, "midi-follow.send-back", "on");
        }
        // Controller mode, but never its switch: MPFree starts with the
        // pads its own.
        let c = &self.controller;
        let d = ControllerSetup::default();
        if !c.port.is_empty() {
            line(&mut s, "controller.port", &c.port);
        }
        if c.channel != d.channel {
            line(&mut s, "controller.channel", &(c.channel + 1).to_string());
        }
        if let PadNotes::From(n) = c.pads {
            line(&mut s, "controller.pads", &format!("from {n}"));
        }
        if c.pressure != d.pressure {
            line(&mut s, "controller.pressure", c.pressure.word());
        }
        if c.ccs != d.ccs {
            let k: Vec<String> = c.ccs.iter().map(u8::to_string).collect();
            line(&mut s, "controller.qlinks", &k.join(" "));
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
        // The route or thru line read last, for its blocks' line.
        let mut last: Option<&str> = None;
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
            // Blocks belong to the route or thru line just before them:
            // none when that line was refused.
            let (owner, blocks) = match k {
                "route" | "thru" => (Some(k), false),
                "route.blocks" => (None, last != Some("route")),
                "thru.blocks" => (None, last != Some("thru")),
                _ => (None, false),
            };
            if blocks {
                warn.push(format!("line {n}: {k}: not after its route: skipped"));
                continue;
            }
            match m.apply(k, v, &mut ports) {
                Err(e) => {
                    warn.push(format!("line {n}: {k}: {e}"));
                    last = None;
                }
                Ok(()) if owner.is_some() => last = owner,
                Ok(()) => {}
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
            ["mmc", "in"] => self.mmc_in = parse_on(v)?,
            ["program-change", "in"] => {
                self.program_in = ProgramIn::from_word(v)
                    .ok_or_else(|| format!("expected off or scene, found \"{v}\""))?
            }
            ["midi-follow"] => self.follow.on = parse_on(v)?,
            ["midi-follow", "port"] => self.follow.port = v.to_string(),
            ["midi-follow", "knobs"] => self.follow.knobs = parse_knobs(v)?,
            ["midi-follow", "send-back"] => self.follow.send_back = parse_on(v)?,
            ["controller", "port"] => self.controller.port = v.to_string(),
            ["controller", "channel"] => self.controller.channel = number(v, 1, 16)? as u8 - 1,
            ["controller", "pads"] => {
                self.controller.pads = match v.split_whitespace().collect::<Vec<_>>()[..] {
                    ["kit"] => PadNotes::Kit,
                    ["from", n] => PadNotes::From(crate::profile::note_number(n)?),
                    _ => return Err(format!("expected kit or from a note, found \"{v}\"")),
                }
            }
            ["controller", "pressure"] => {
                self.controller.pressure = PadPressure::ALL
                    .into_iter()
                    .find(|p| p.word() == v)
                    .ok_or(format!("expected off, poly or channel, found \"{v}\""))?
            }
            ["controller", "qlinks"] => self.controller.ccs = parse_qlinks(v)?,
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
                    // Thru was a switch on the port, kept with the rig,
                    // before it was built: it did nothing, and thru is a
                    // route now ([`ThruRoute`]).
                    ["thru"] => {
                        parse_on(v)?;
                    }
                    ["ignore"] => p.ignore = IgnoreSet::parse(v)?,
                    ["delay-ms"] => p.delay_tenths = parse_delay(v)?,
                    ["clock-divide"] => {
                        p.clock_divide = number(v, 1, u32::from(MAX_CLOCK_DIVIDE))? as u8
                    }
                    ["clock-nudge-ms"] => p.clock_nudge_tenths = parse_nudge(v)?,
                    ["shown"] => p.shown = parse_shown(v)?,
                    ["profile"] => p.profile = ProfilePick::from_word(v)?,
                    ["blocks"] => p.blocks = Blocks::parse(v)?,
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
                if self.routes.iter().any(|x| x.same_route(&r)) {
                    return Err("the same route twice: the first kept".into());
                }
                if self.routes.len() >= MAX_TRACK_ROUTES {
                    return Err(format!("more than {MAX_TRACK_ROUTES} routes"));
                }
                self.routes.push(r);
            }
            ["thru"] => {
                let r = ThruRoute::parse(v)?;
                if self.thru.iter().any(|x| x.same_route(&r)) {
                    return Err("the same thru twice: the first kept".into());
                }
                if self.thru.len() >= MAX_THRU_ROUTES {
                    return Err(format!("more than {MAX_THRU_ROUTES} thru routes"));
                }
                self.thru.push(r);
            }
            // A route's or a thru's blocks, on the line after it
            // ([`MidiSetup::parse`] skips them when that line was refused).
            ["route", "blocks"] => {
                let b = Blocks::parse(v)?;
                let r = self.routes.last_mut().ok_or("no route before it")?;
                r.blocks = b;
            }
            ["thru", "blocks"] => {
                let b = Blocks::parse(v)?;
                let r = self.thru.last_mut().ok_or("no thru before it")?;
                r.blocks = b;
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
/// Up to [`FOLLOW_KNOBS`] CC numbers apart by spaces ("74 71 76 77"),
/// none twice.
pub fn parse_knobs(v: &str) -> Result<Vec<u8>, String> {
    let mut k = Vec::new();
    for w in v.split_whitespace() {
        let cc = number(w, 0, 119)? as u8;
        if k.contains(&cc) {
            return Err(format!("CC {cc} twice"));
        }
        if k.len() == FOLLOW_KNOBS {
            return Err(format!("more than {FOLLOW_KNOBS} knobs"));
        }
        k.push(cc);
    }
    Ok(k)
}

/// [`CONTROLLER_KNOBS`] CC numbers (0 to 119) apart by spaces; the same
/// CC may come twice (two Q-Links on one control).
fn parse_qlinks(v: &str) -> Result<[u8; CONTROLLER_KNOBS], String> {
    let mut k = [0; CONTROLLER_KNOBS];
    let mut n = 0;
    for w in v.split_whitespace() {
        if n == CONTROLLER_KNOBS {
            return Err(format!("more than {CONTROLLER_KNOBS} CCs"));
        }
        k[n] = number(w, 0, 119)? as u8;
        n += 1;
    }
    if n < CONTROLLER_KNOBS {
        return Err(format!(
            "{n} CCs: one for each of the {CONTROLLER_KNOBS} Q-Links"
        ));
    }
    Ok(k)
}

fn parse_shown(v: &str) -> Result<String, String> {
    if v.chars().count() > MAX_SHOWN {
        return Err(format!("longer than {MAX_SHOWN} characters"));
    }
    Ok(v.to_string())
}

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

fn parse_nudge(v: &str) -> Result<i16, String> {
    let ms: f32 = v
        .parse()
        .map_err(|_| format!("\"{v}\" is not a number of ms"))?;
    let max = f32::from(MAX_NUDGE_TENTHS) / 10.0;
    if !(-max..=max).contains(&ms) {
        return Err(format!("{ms} ms is outside -{max} to +{max} ms"));
    }
    Ok((ms * 10.0).round() as i16)
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
        CvAssign::Pressure { track, max_volts } => {
            format!("pressure track={} max-volts={}", track + 1, f(max_volts))
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
        CvAssign::Lfo(l) => {
            let rate = match l.rate {
                Rate::Hz(hz) => format!("hz={}", f(hz)),
                Rate::Beats(b) => format!("beats={}", f(b)),
            };
            let restart = l
                .restart
                .map(|t| format!(" restart={}", t + 1))
                .unwrap_or_default();
            let notes = if l.notes.is_off() {
                String::new()
            } else {
                format!(" notes={:03x} root={}", l.notes.mask & 0xFFF, l.notes.root)
            };
            format!(
                "lfo shape={} {rate} low={} high={}{restart}{notes}",
                l.shape.word(),
                f(l.low_volts),
                f(l.high_volts)
            )
        }
        CvAssign::Envelope(e) => format!(
            "envelope track={} attack-ms={} decay-ms={} sustain={} release-ms={} volts={} velocity={}",
            e.track + 1,
            f(e.attack_ms),
            f(e.decay_ms),
            f(e.sustain),
            f(e.release_ms),
            f(e.volts),
            if e.velocity { "on" } else { "off" }
        ),
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
    let float = |k: &str, lo: f32, hi: f32, unit: &str| -> Result<f32, String> {
        let x: f32 = need(k)?.parse().map_err(|_| format!("{k}: not a number"))?;
        if (lo..=hi).contains(&x) {
            Ok(x)
        } else {
            Err(format!("{k}: {x} is outside {lo} to {hi}{unit}"))
        }
    };
    for (k, _) in &kv {
        let known: &[&str] = match kind {
            "pitch" => &["track", "zero", "bend", "priority", "v/oct", "hz/v"],
            "gate" => &["track", "volts", "retrigger-ms"],
            "trigger" => &["track", "note", "volts", "ms"],
            "velocity" | "pressure" => &["track", "max-volts"],
            "cc" => &["track", "cc", "min-volts", "max-volts"],
            "clock" => &["ppqn", "volts", "ms"],
            "run" => &["volts"],
            "reset" => &["volts", "ms"],
            "fixed" => &["volts"],
            "lfo" => &[
                "shape", "hz", "beats", "low", "high", "restart", "notes", "root",
            ],
            "envelope" => &[
                "track",
                "attack-ms",
                "decay-ms",
                "sustain",
                "release-ms",
                "volts",
                "velocity",
            ],
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
        "pressure" => CvAssign::Pressure {
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
        "lfo" => {
            let shape = need("shape")?;
            let shape = Shape::from_word(shape).ok_or(format!("lfo: unknown shape \"{shape}\""))?;
            let rate = match (get("hz"), get("beats")) {
                (Some(_), None) => {
                    Rate::Hz(float("hz", cv_mod::LFO_MIN_HZ, cv_mod::LFO_MAX_HZ, " Hz")?)
                }
                (None, Some(_)) => Rate::Beats(float(
                    "beats",
                    cv_mod::LFO_MIN_BEATS,
                    cv_mod::LFO_MAX_BEATS,
                    " beats",
                )?),
                _ => return Err("lfo: one of hz or beats".into()),
            };
            let notes = match get("notes") {
                None => Notes::OFF,
                Some(n) => Notes {
                    mask: u16::from_str_radix(n, 16)
                        .ok()
                        .filter(|m| *m <= 0xFFF)
                        .ok_or(format!("notes: \"{n}\" is not 12 notes in hex"))?,
                    root: number(need("root")?, 0, 11)? as u8,
                },
            };
            CvAssign::Lfo(Lfo {
                shape,
                rate,
                low_volts: volts("low")?,
                high_volts: volts("high")?,
                restart: get("restart")
                    .map(|t| index(t, MAX_TRACKS).map(|t| t as u8))
                    .transpose()?,
                notes,
            })
        }
        "envelope" => {
            let ms = |k: &str| float(k, 0.0, cv_mod::ENV_MAX_MS, " ms");
            CvAssign::Envelope(Env {
                track: track()?,
                attack_ms: ms("attack-ms")?,
                decay_ms: ms("decay-ms")?,
                sustain: float("sustain", 0.0, 1.0, "")?,
                release_ms: ms("release-ms")?,
                volts: volts("volts")?,
                velocity: match need("velocity")? {
                    "on" => true,
                    "off" => false,
                    v => return Err(format!("velocity: \"{v}\" is not on or off")),
                },
            })
        }
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
                    synth: None,
                    blocks: Blocks::parse("transpose -12; velocity soft 20 127").unwrap(),
                },
                TrackRoute {
                    port: "Keystep 37".to_string(),
                    channel: None,
                    track: 127,
                    synth: None,
                    blocks: Blocks::NONE,
                },
            ],
            ..MidiSetup::default()
        };
        let text = m.to_text();
        assert!(
            text.contains(
                "route = track 1 < MIDI · ch 10\nroute.blocks = transpose -12; velocity soft 20 127\n"
            ),
            "{text}"
        );
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

    /// A route that plays a synth no track has (the instrument redesign's
    /// step 11e) reads back by the synth's number, and isn't the same
    /// route as one to a track; number 0 isn't one. (A build from before
    /// reads "instrument" as it reads "track 2 MIDI" above: named, and
    /// left out.)
    #[test]
    fn a_route_to_a_synth_reads_back_by_its_number() {
        let synth = TrackRoute {
            port: DIN_NAME.to_string(),
            channel: Some(1),
            track: 0,
            synth: Some(4),
            blocks: Blocks::NONE,
        };
        let track = TrackRoute {
            synth: None,
            ..synth.clone()
        };
        let m = MidiSetup {
            routes: vec![synth.clone(), track.clone()],
            ..MidiSetup::default()
        };
        let text = m.to_text();
        assert!(
            text.contains("route = instrument 4 < MIDI · ch 2\n"),
            "{text}"
        );
        let (back, warn) = MidiSetup::parse(&text);
        assert!(warn.is_empty(), "{warn:?}");
        assert_eq!(back.routes, m.routes);
        assert!(!synth.same_route(&track));
        let (back, warn) = MidiSetup::parse("mpfree-midi 1\nroute = instrument 0 < MIDI · ch 2\n");
        assert!(back.routes.is_empty());
        assert_eq!(warn.len(), 1, "{warn:?}");
    }

    /// A route's, a thru's and a port's blocks read back with the song; a
    /// blocks line after a refused route (or none) is skipped, never given
    /// to the route before it.
    #[test]
    fn blocks_belong_to_their_route_and_the_song() {
        let mut m = MidiSetup::default();
        m.set_port("Keystep 37", |p| {
            p.blocks = Blocks::parse("filter program; velocity hard 1 127").unwrap();
            p.input = PortInput::Both;
        });
        m.thru.push(ThruRoute {
            from: "Keystep 37".to_string(),
            channel: None,
            to: DIN_NAME.to_string(),
            blocks: Blocks::parse("channel 2").unwrap(),
        });
        let text = m.to_text();
        assert!(text.contains("thru.blocks = channel 2\n"), "{text}");
        assert!(
            text.contains(".blocks = filter program; velocity hard 1 127\n"),
            "{text}"
        );
        let (back, warn) = MidiSetup::parse(&text);
        assert!(warn.is_empty(), "{warn:?}");
        assert_eq!(back, m);
        // The song's, not the rig's.
        assert_eq!(
            m.project().port("Keystep 37").blocks,
            m.port("Keystep 37").blocks
        );
        assert!(m.rig().port("Keystep 37").blocks.is_empty());
        let mut kept = m.clone();
        kept.set_rig(&MidiSetup::default());
        assert_eq!(kept.port("Keystep 37").blocks, m.port("Keystep 37").blocks);
        let (back, warn) = MidiSetup::parse(
            "mpfree-midi 1\nroute.blocks = transpose 1\nroute = track 1 < MIDI · ch 1\n\
             route = track 1 < MIDI · ch 1\nroute.blocks = transpose 2\n\
             route = track 2 < MIDI · ch 2\nroute.blocks = transpose 99\n",
        );
        assert_eq!(back.routes.len(), 2);
        assert!(
            back.routes.iter().all(|r| r.blocks.is_empty()),
            "{:?}",
            back.routes
        );
        assert_eq!(warn.len(), 4, "{warn:?}");
        assert!(warn[0].contains("not after its route"), "{warn:?}");
        assert!(warn[2].contains("not after its route"), "{warn:?}");
    }

    #[test]
    fn thru_reads_back_belongs_to_the_song_and_an_old_port_switch_is_dropped() {
        let m = MidiSetup {
            thru: vec![
                ThruRoute {
                    from: "Keystep 37".to_string(),
                    channel: Some(9),
                    to: DIN_NAME.to_string(),
                    blocks: Blocks::NONE,
                },
                ThruRoute {
                    from: DIN_NAME.to_string(),
                    channel: None,
                    to: DIN_NAME.to_string(),
                    blocks: Blocks::NONE,
                },
            ],
            ..MidiSetup::default()
        };
        let text = m.to_text();
        assert!(
            text.contains("thru = MIDI < Keystep 37 · ch 10\n"),
            "{text}"
        );
        assert!(
            text.contains("thru = MIDI < MIDI · all channels\n"),
            "{text}"
        );
        let (back, warn) = MidiSetup::parse(&text);
        assert!(warn.is_empty(), "{warn:?}");
        assert_eq!(back.thru, m.thru);
        assert_eq!(m.project().thru, m.thru);
        assert!(m.rig().thru.is_empty());
        let mut kept = m.clone();
        kept.set_rig(&MidiSetup::default());
        assert_eq!(kept.thru, m.thru);
        // A USB device back to itself, a bad channel and the same thru
        // twice are named and skipped; the old switch on a port is read
        // and dropped, with nothing to say.
        let (back, warn) = MidiSetup::parse(
            "mpfree-midi 1\nport.1 = MIDI\nport.1.thru = on\n\
             thru = Keystep 37 < Keystep 37 · ch 1\nthru = MIDI < Keystep 37 · ch 17\n\
             thru = MIDI Keystep 37\nthru = MIDI < Keystep 37 · ch 3\nthru = MIDI < Keystep 37 · ch 3\n",
        );
        assert_eq!(back.thru.len(), 1);
        assert_eq!(back.thru[0].source(), "Keystep 37 · ch 3");
        assert!(back.ports.is_empty(), "{:?}", back.ports);
        assert_eq!(warn.len(), 4, "{warn:?}");
        assert!(warn[0].contains("back to itself"), "{warn:?}");
        assert!(warn[3].contains("the same thru twice"), "{warn:?}");
        assert!(thru_allowed(DIN_NAME, DIN_NAME));
        assert!(!thru_allowed("Digitakt", "Digitakt"));
        assert!(thru_allowed("Keystep 37", "Digitakt"));
    }

    #[test]
    fn a_ports_shown_name_is_the_rigs_and_no_two_ports_look_alike() {
        let mut m = MidiSetup::default();
        let plugged = ["MIDI", "USB MIDI Interface", "Keystep 37"];
        assert_eq!(m.shown("USB MIDI Interface"), "USB MIDI Interface");
        m.set_shown("USB MIDI Interface", "  Juno ", plugged)
            .unwrap();
        assert_eq!(m.shown("USB MIDI Interface"), "Juno");
        assert_eq!(m.port("USB MIDI Interface").shown, "Juno");
        // Its routes and tracks still name the port by its own name.
        assert_eq!(m.shown("Keystep 37"), "Keystep 37");
        // No two ports look alike, whatever the capitals: not another
        // port's own name, nor one given to another.
        let e = m.set_shown("Keystep 37", "juno", plugged).unwrap_err();
        assert!(e.contains("Another port is called juno"), "{e}");
        assert!(m.set_shown("Keystep 37", "MIDI", plugged).is_err());
        assert!(m.set_shown("Keystep 37", "midi out", []).is_err());
        assert!(m
            .set_shown("Keystep 37", "x".repeat(MAX_SHOWN + 1).as_str(), plugged)
            .is_err());
        // Renaming a port again to what it shows, or to its own name.
        m.set_shown("USB MIDI Interface", "JUNO", plugged).unwrap();
        assert_eq!(m.shown("USB MIDI Interface"), "JUNO");
        m.set_shown("Keystep 37", "Keys", plugged).unwrap();
        // Kept with the rig, not the song, and read back.
        assert_eq!(m.project().port("Keystep 37").shown, "");
        let (rig, warn) = MidiSetup::parse(&m.rig_text());
        assert!(warn.is_empty(), "{warn:?}");
        assert_eq!(rig.shown("Keystep 37"), "Keys");
        assert!(
            m.rig_text().contains("port.2.shown = Keys"),
            "{}",
            m.rig_text()
        );
        let mut other = MidiSetup::default();
        other.set_rig(&rig);
        assert_eq!(other.shown("USB MIDI Interface"), "JUNO");
        // Its own name takes the given one off, and a port with nothing
        // else set is dropped.
        m.set_shown("USB MIDI Interface", "USB MIDI Interface", plugged)
            .unwrap();
        assert_eq!(m.shown("USB MIDI Interface"), "USB MIDI Interface");
        assert!(m.ports.iter().all(|p| p.name != "USB MIDI Interface"));
        let long = format!(
            "port.1 = MIDI\nport.1.shown = {}\n",
            "y".repeat(MAX_SHOWN + 1)
        );
        let (back, warn) = MidiSetup::parse(&format!("{HEADER}\n{long}"));
        assert_eq!(warn.len(), 1, "{warn:?}");
        assert_eq!(back.shown(DIN_NAME), DIN_NAME);
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
            p.clock_out = ClockOut::Always;
            p.clock_divide = 2;
            p.clock_nudge_tenths = -15;
        });
        m.set_port("Keystep 37", |p| {
            p.input = PortInput::PlaysTracks;
            p.ignore = IgnoreSet::NONE.with(Ignored::Clock, true);
        });
        m.set_port("Digitone", |p| {
            p.delay_tenths = 65;
            p.shown = "Drums".into();
        });
        m.tracks[0] = Some(TrackOut::new(DIN_NAME, 9));
        let project = m.project();
        let rig = m.rig();
        // The song keeps the clock out, not the input, what a port ignores
        // or its gear's delay.
        assert_eq!(project.port(DIN_NAME).clock_out, ClockOut::Always);
        // Its clock's divide is the song's; its nudge, the gear's.
        assert_eq!(project.port(DIN_NAME).clock_divide, 2);
        assert_eq!(project.port(DIN_NAME).clock_nudge_tenths, 0);
        assert_eq!(rig.port(DIN_NAME).clock_divide, 1);
        assert_eq!(rig.port(DIN_NAME).clock_nudge_tenths, -15);
        assert_eq!(project.port(DIN_NAME).input, PortInput::default());
        assert!(project.ports.iter().all(|p| p.name != "Keystep 37"));
        assert!(project.ports.iter().all(|p| p.name != "Digitone"));
        assert_eq!(project.tracks[0], m.tracks[0]);
        // The rig keeps the inputs, ignores and delays.
        assert_eq!(rig.port(DIN_NAME).input, PortInput::LearnOnly);
        assert!(rig.port("Keystep 37").ignore.has(Ignored::Clock));
        assert_eq!(rig.port("Digitone").delay_tenths, 65);
        assert_eq!(rig.shown("Digitone"), "Drums");
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
            mmc_in: true,
            program_in: ProgramIn::Scene,
            ..MidiSetup::default()
        };
        m.set_port(DIN_NAME, |p| {
            p.clock_out = ClockOut::WhilePlaying;
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

    /// A footswitch learned on the looper (the looper redesign, step 5)
    /// is the rig's and reads back by the looper's fixed numbers: a loop's
    /// key, the picked loop's key and a loop's mute.
    #[test]
    fn a_looper_footswitch_reads_back_from_the_rig() {
        use crate::learn::{Mode, Source};
        use mpfree_param::{looper, Owner, ParamAddr};
        let at = |id| ParamAddr::new(Owner::Looper, id);
        let mut m = MidiSetup::default();
        let mut key = learned(Some("FS-7"), 64);
        key.target = at(looper::key(2));
        key.mode = Mode::Momentary;
        let mut picked = learned(Some("FS-7"), 65);
        picked.target = at(looper::PICKED_KEY);
        picked.mode = Mode::Momentary;
        let mut mute = learned(None, 0);
        mute.source = Source::Note(40);
        mute.channel = 9;
        mute.target = at(looper::mute(7));
        mute.mode = Mode::Toggle;
        let undo = Saved {
            target: at(looper::UNDO),
            ..learned(Some("FS-7"), 66)
        };
        for l in [&key, &picked, &mute, &undo] {
            assert!(m.learned_in(Kept::Rig, l.clone()));
        }
        let text = m.rig_text();
        assert!(text.contains("looper.258"), "{text}");
        assert!(text.contains("looper.0"), "{text}");
        assert!(text.contains("looper.519"), "{text}");
        assert!(text.contains("looper.3"), "{text}");
        let (back, w) = MidiSetup::parse(&text);
        assert!(w.is_empty(), "{w:?}\n{text}");
        assert_eq!(back.rig_learn, [key, picked, mute, undo]);
    }

    /// A knob learned on Play's Swing and one on the XY pad (the Learn
    /// section's Play card) are the rig's and read back by Play's fixed
    /// numbers.
    #[test]
    fn plays_learned_knobs_read_back_from_the_rig() {
        use mpfree_param::{play, Owner, ParamAddr};
        let mut m = MidiSetup::default();
        let swing = Saved {
            target: ParamAddr::new(Owner::Play, play::SWING),
            ..learned(Some("Knobs"), 21)
        };
        let up = Saved {
            target: ParamAddr::new(Owner::Play, play::XY_UP),
            ..learned(Some("Knobs"), 22)
        };
        for l in [&swing, &up] {
            assert!(m.learned_in(Kept::Rig, l.clone()));
        }
        let text = m.rig_text();
        assert!(text.contains("play.0") && text.contains("play.3"), "{text}");
        let (back, w) = MidiSetup::parse(&text);
        assert!(w.is_empty(), "{w:?}\n{text}");
        assert_eq!(back.rig_learn, [swing, up]);
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
        assert!(
            text.contains("mmc.in = on\nprogram-change.in = scene\n"),
            "{text}"
        );
    }

    /// MMC in and program change in are the song's, off unless set; a
    /// word the setting hasn't got is named and skipped.
    #[test]
    fn mmc_and_program_change_in_are_the_songs() {
        let m = MidiSetup {
            mmc_in: true,
            program_in: ProgramIn::Scene,
            ..MidiSetup::default()
        };
        assert!(m.project().mmc_in);
        assert_eq!(m.project().program_in, ProgramIn::Scene);
        assert!(!m.rig().mmc_in);
        assert_eq!(m.rig().program_in, ProgramIn::Off);
        assert!(!MidiSetup::default().to_text().contains("mmc"));
        let (back, w) = MidiSetup::parse(&format!("{HEADER}\nprogram-change.in = track\n"));
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(back.program_in, ProgramIn::Off);
    }

    #[test]
    fn rest_of_the_line_is_a_port_name() {
        let (m, w) = MidiSetup::parse(
            "mpfree-midi 1\nport.1 = My Synth (USB) = 3\nport.1.clock-out = always\n",
        );
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(m.port("My Synth (USB) = 3").clock_out, ClockOut::Always);
    }

    #[test]
    fn a_port_back_at_its_defaults_is_dropped() {
        let mut m = MidiSetup::default();
        m.set_port(DIN_NAME, |p| p.delay_tenths = 5);
        assert_eq!(m.ports.len(), 1);
        m.set_port(DIN_NAME, |p| p.delay_tenths = 0);
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
    fn lfos_and_envelopes_read_back_and_bad_ones_say_why() {
        let mut m = MidiSetup::default();
        m.cv[0] = CvAssign::Lfo(Lfo {
            shape: Shape::Drift,
            rate: Rate::Beats(2.0 / 3.0),
            low_volts: 0.5,
            high_volts: 2.5,
            restart: Some(3),
            notes: Notes {
                mask: 0xAB5,
                root: 7,
            },
        });
        m.cv[1] = CvAssign::Lfo(Lfo {
            rate: Rate::Hz(0.25),
            ..Lfo::default()
        });
        m.cv[2] = CvAssign::Envelope(Env {
            velocity: true,
            sustain: 0.35,
            ..Env::new(9)
        });
        m.cv[3] = CvAssign::Pressure {
            track: 1,
            max_volts: 5.0,
        };
        let text = m.to_text();
        assert!(
            text.contains("cv.4 = pressure track=2 max-volts=5\n"),
            "{text}"
        );
        assert!(
            text.contains("cv.2 = lfo shape=sine hz=0.25 low=0 high=5\n"),
            "{text}"
        );
        assert!(text.contains(
            "cv.1 = lfo shape=drift beats=0.6666667 low=0.5 high=2.5 restart=4 notes=ab5 root=7\n"
        ));
        assert!(text.contains(
            "cv.3 = envelope track=10 attack-ms=5 decay-ms=200 sustain=0.35 release-ms=300 volts=5 velocity=on\n"
        ));
        let (back, warn) = MidiSetup::parse(&text);
        assert!(warn.is_empty(), "{warn:?}");
        assert_eq!(back.cv, m.cv);

        let text = "mpfree-midi 1\n\
                    cv.1 = lfo shape=wobble hz=1 low=0 high=5\n\
                    cv.2 = lfo shape=sine hz=50 low=0 high=5\n\
                    cv.3 = lfo shape=sine low=0 high=5\n\
                    cv.4 = lfo shape=sine beats=1 low=0 high=5 notes=fff1 root=0\n\
                    cv.5 = envelope track=1 attack-ms=20000 decay-ms=1 sustain=1 release-ms=1 volts=5 velocity=off\n\
                    cv.6 = envelope track=1 attack-ms=1 decay-ms=1 sustain=1 release-ms=1 volts=5 velocity=maybe\n";
        let (m, w) = MidiSetup::parse(text);
        assert!(m.cv.iter().all(|a| *a == CvAssign::Off));
        let all = w.join("\n");
        for want in [
            "line 2: cv.1: lfo: unknown shape \"wobble\"",
            "line 3: cv.2: hz: 50 is outside 0.01 to 20 Hz",
            "line 4: cv.3: lfo: one of hz or beats",
            "line 5: cv.4: notes: \"fff1\" is not 12 notes in hex",
            "line 6: cv.5: attack-ms: 20000 is outside 0 to 10000 ms",
            "line 7: cv.6: velocity: \"maybe\" is not on or off",
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
            "mpfree-midi 1\nport.1 = MIDI\nport.1.input = learn-only\nport.2 = MIDI\nport.2.clock-out = always\n",
        );
        assert_eq!(m.port(DIN_NAME).input, PortInput::LearnOnly);
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

    /// Controller mode is the rig's, but its switch is never saved; a
    /// kit's pads send the profile's notes and channels, else MPFree's;
    /// From counts up through the banks.
    #[test]
    fn controller_mode_is_the_rigs_and_starts_off() {
        let mut c = ControllerSetup {
            on: true,
            port: "Juno-X".into(),
            channel: 2,
            pads: PadNotes::From(36),
            pressure: PadPressure::Channel,
            ..ControllerSetup::default()
        };
        c.ccs[0] = 74;
        c.ccs[15] = 74;
        let m = MidiSetup {
            controller: c.clone(),
            ..MidiSetup::default()
        };
        let rig = m.rig_text();
        assert!(
            rig.ends_with(
                "controller.port = Juno-X\ncontroller.channel = 3\ncontroller.pads = from 36\n\
                 controller.pressure = channel\ncontroller.qlinks = 74 17 18 19 20 21 22 23 24 \
                 25 26 27 28 29 30 74\n"
            ),
            "{rig}"
        );
        assert!(!m.project_text().contains("controller"));
        let (back, warn) = MidiSetup::parse(&rig);
        assert!(warn.is_empty(), "{warn:?}");
        assert_eq!(back.controller, ControllerSetup { on: false, ..c });
        // A project opened keeps the rig's, switch and all.
        let mut now = m.clone();
        now.set_project(&MidiSetup::default());
        assert!(now.controller.on);
        assert!(!MidiSetup::default().rig_text().contains("controller"));
        // Notes: from 36 up; 128 pads from 36 run out at 127.
        let from = &back.controller;
        assert_eq!(from.pad_out(0, None), Some((36, 2)));
        assert_eq!(from.pad_out(91, None), Some((127, 2)));
        assert_eq!(from.pad_out(92, None), None);
        // A kit: the profile's pad lines, else MPFree's pad notes.
        let kit = ControllerSetup::default();
        let (p, _) =
            crate::profile::Profile::parse("mpfree-profile 1\nname = TR\npad.A1 = 36 ch 10\n")
                .unwrap();
        assert_eq!(kit.pad_out(0, Some(&p)), Some((36, 9)));
        assert_eq!(
            kit.pad_out(1, Some(&p)),
            kit.pad_out(1, None),
            "A2 has no line"
        );
        assert_eq!(kit.pad_out(0, None), Some((37, 0)));
        // Bad lines say why.
        let (_, warn) = MidiSetup::parse(
            "mpfree-midi 1\ncontroller.channel = 17\ncontroller.pads = from H#2\n\
             controller.pressure = hard\ncontroller.qlinks = 1 2 3\n",
        );
        assert_eq!(warn.len(), 4, "{warn:?}");
        assert!(
            warn[3].contains("3 CCs: one for each of the 16 Q-Links"),
            "{warn:?}"
        );
    }

    /// MIDI Follow is the rig's: its controller, its knobs, on or off.
    #[test]
    fn midi_follow_is_the_rigs() {
        let m = MidiSetup {
            follow: FollowSetup {
                on: true,
                port: "Keystep 37".into(),
                knobs: vec![74, 71, 76, 77],
                send_back: true,
            },
            ..MidiSetup::default()
        };
        let rig = m.rig_text();
        assert!(
            rig.ends_with(
                "midi-follow = on\nmidi-follow.port = Keystep 37\nmidi-follow.knobs = 74 71 76 77\n\
                 midi-follow.send-back = on\n"
            ),
            "{rig}"
        );
        assert!(!m.project_text().contains("midi-follow"));
        let (back, warn) = MidiSetup::parse(&rig);
        assert!(warn.is_empty(), "{warn:?}");
        assert_eq!(back.follow, m.follow);
        assert!(back.follow.follows("Keystep 37") && !back.follow.follows("Juno"));
        let mut other = MidiSetup::default();
        other.set_rig(&back);
        assert_eq!(other.follow, m.follow);
        // Its own knobs, else its profile's; four at most.
        let (p, _) = crate::profile::Profile::parse("name = KS\nknobs = 1 2 3 4 5\n").unwrap();
        assert_eq!(m.follow.knobs_of(Some(&p)), [74, 71, 76, 77]);
        let bare = FollowSetup::default();
        assert_eq!(bare.knobs_of(Some(&p)), [1, 2, 3, 4]);
        assert!(bare.knobs_of(None).is_empty());
        assert!(!bare.follows(""));
        // Knobs: CCs 0 to 119, none twice, four at most.
        assert_eq!(parse_knobs(" 1  2 "), Ok(vec![1, 2]));
        assert!(parse_knobs("1 1").is_err());
        assert!(parse_knobs("120").is_err());
        assert!(parse_knobs("1 2 3 4 5").is_err());
    }

    /// A port's profile is the rig's: saved with it (Auto, the default,
    /// isn't written), left out of the project, read back as picked.
    #[test]
    fn a_ports_profile_is_the_rigs() {
        let mut m = MidiSetup::default();
        m.set_port("Juno-X", |p| {
            p.profile = ProfilePick::Named("Juno-X".into())
        });
        m.set_port(DIN_NAME, |p| p.profile = ProfilePick::Off);
        let rig = m.rig_text();
        assert!(rig.contains("port.1.profile = Juno-X\n"), "{rig}");
        assert!(rig.contains("port.2.profile = none\n"), "{rig}");
        assert!(!m.project_text().contains("profile"));
        let (back, warn) = MidiSetup::parse(&rig);
        assert!(warn.is_empty(), "{warn:?}");
        assert_eq!(
            back.port("Juno-X").profile,
            ProfilePick::Named("Juno-X".into())
        );
        assert_eq!(back.port(DIN_NAME).profile, ProfilePick::Off);
        assert_eq!(back.port("Keystep").profile, ProfilePick::Auto);
        // Another project keeps the rig's picks; Auto again drops the port.
        let mut other = MidiSetup::default();
        other.set_rig(&back);
        assert_eq!(other.port(DIN_NAME).profile, ProfilePick::Off);
        other.set_port(DIN_NAME, |p| p.profile = ProfilePick::Auto);
        other.set_port("Juno-X", |p| p.profile = ProfilePick::Auto);
        assert!(other.ports.is_empty());
        // Words: the two the page uses, any capitals; a name too long isn't one.
        assert_eq!(ProfilePick::from_word("AUTO"), Ok(ProfilePick::Auto));
        assert_eq!(ProfilePick::from_word("None"), Ok(ProfilePick::Off));
        assert!(ProfilePick::from_word(&"x".repeat(40)).is_err());
        let (_, warn) = MidiSetup::parse(&format!("{HEADER}\nport.1 = A\nport.1.profile =\n"));
        assert_eq!(warn.len(), 1, "{warn:?}");
    }
}
