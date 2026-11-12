//! A project's MIDI, sync and CV settings: what the mixer's I/O view edits
//! and the project saves as its `midi` part (`parts/midi.txt`).
//!
//! The owner's calls (2026-11-12): MIDI and sync are project settings, not
//! Prefs, because they change from project to project. Facts about the rig
//! rather than the project (a port's shown name, its delay, each CV output's
//! tuning) stay in Prefs and are not here.
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
use crate::msg::Channel;
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

/// One port's project settings. A port with none of its own uses
/// [`PortSetup::new`]'s defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortSetup {
    pub name: String,
    pub input: PortInput,
    pub clock_out: ClockOut,
    /// What arrives on this port's input leaves its output too.
    pub thru: bool,
}

impl PortSetup {
    pub fn new(name: &str) -> PortSetup {
        PortSetup {
            name: name.to_string(),
            input: PortInput::default(),
            clock_out: ClockOut::default(),
            thru: false,
        }
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
    /// What each CV/gate output does.
    pub cv: [CvAssign; CV_OUTS],
}

impl Default for MidiSetup {
    fn default() -> Self {
        MidiSetup {
            tempo: TempoSource::Internal,
            follow_start_stop: true,
            follow_position: true,
            ports: Vec::new(),
            tracks: std::array::from_fn(|_| None),
            cv: [CvAssign::Off; CV_OUTS],
        }
    }
}

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
        for (i, a) in self.cv.iter().enumerate() {
            if *a != CvAssign::Off {
                line(&mut s, &format!("cv.{}", i + 1), &cv_text(a));
            }
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
            ["cv", i] => {
                let i = index(i, CV_OUTS)?;
                self.cv[i] = parse_cv(v)?;
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
    use super::*;

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
        m.set_port("Keystep 37", |p| p.input = PortInput::LearnOnly);
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
        m
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
