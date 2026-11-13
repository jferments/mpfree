//! The arp's project part (Jay, 13 Nov: the arp and Note Repeat are the
//! song's, not Prefs): each track's arp as its panel, its options and the
//! pattern workspace left it, with the rate Note Repeat plays at
//! (docs/dev/design/browser.md, "The parts").
//!
//! ```text
//! mpfree-arp 1
//! track 1
//! mode = arp
//! plays = direct
//! rate = 1/16
//! order = updown
//! octaves = 2
//! ...
//! scale = Dorian
//! root = D
//! swing-from = transport
//! pattern-name = Pulse
//! pattern = x . x x | oct 0 1
//! track 2
//! ...
//! ```
//!
//! `mode` is the panel's pick; `plays` is what the arp plays through while
//! its track isn't the armed one (what a through-arp track's clips hear).
//! The pattern is kept as it plays, edits and all, with the name of the
//! library pattern it came from. Latch is how you play, not the song: an
//! open lets every arp go. A word not understood is left at its default
//! and said.

use super::TrackUi;
use crate::app::{App, Effect};
use mpfree_arp::scale::{Scale, Snap, NOTE_NAMES};
use mpfree_arp::{Chord, LatchMode, Mode, Order, Pattern, Rate, SyncStart, Turns, Velocity};
use mpfree_seq::MAX_TRACKS;
use std::fmt::Write;

pub const HEADER: &str = "mpfree-arp 1";

fn word<T: std::fmt::Debug>(v: T) -> String {
    format!("{v:?}").to_lowercase()
}

fn pick<T: std::fmt::Debug + Copy>(all: &[T], v: &str) -> Option<T> {
    all.iter().copied().find(|&x| word(x) == v)
}

fn on(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}

fn flag(v: &str) -> Option<bool> {
    match v {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
}

fn note_of(v: &str) -> Option<u8> {
    NOTE_NAMES.iter().position(|n| *n == v).map(|i| i as u8)
}

fn number<T: std::str::FromStr + PartialOrd>(v: &str, lo: T, hi: T) -> Option<T> {
    v.parse::<T>().ok().filter(|n| *n >= lo && *n <= hi)
}

/// FNV-1a over what is written to it: the arps' state, without a String.
struct Fnv(u64);

impl Write for Fnv {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        for b in s.bytes() {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
        Ok(())
    }
}

impl App {
    /// Every track's arp: the armed track's as the panel holds it now, the
    /// others' as they were left.
    pub(in crate::app) fn arp_tracks(&self) -> [TrackUi; MAX_TRACKS] {
        let mut t = self.track_ui;
        t[self.track] = self.current_ui();
        t
    }

    /// What the project keeps of the arps, as a number that changes when
    /// any of it does (Latch and the arp playing aren't in it). Allocates
    /// nothing.
    pub(in crate::app) fn arp_fingerprint(&self) -> u64 {
        let mut h = Fnv(0xcbf2_9ce4_8422_2325);
        for u in self.arp_tracks() {
            let _ = write!(
                h,
                "{:?}|{:?}|{:?}|{}|{}|{}|{}|{:?};",
                u.arp,
                u.arp_mode,
                u.rate,
                u.scale_root,
                u.scale_pick,
                u.swing_from_transport,
                u.pattern_index,
                u.pattern
            );
        }
        h.0
    }

    /// The arp part: every track's arp.
    pub fn arp_part(&self) -> String {
        let mut out = format!("{HEADER}\n");
        for (t, u) in self.arp_tracks().iter().enumerate() {
            let s = &u.arp;
            let scale = match s.scale.and(self.scale_entry(u.scale_pick)) {
                Some((name, ..)) => name.to_string(),
                None => "off".to_string(),
            };
            let pattern_name = self
                .patterns
                .get(u.pattern_index)
                .map(|(n, _)| n.clone())
                .unwrap_or_default();
            let velocity = match s.velocity {
                Velocity::Played => "played".to_string(),
                Velocity::Fixed(v) => v.to_string(),
            };
            let lines = [
                ("mode", word(u.arp_mode)),
                ("plays", word(self.arps.engine_at(t).settings().mode)),
                ("rate", u.rate.label().to_string()),
                ("order", word(s.order)),
                ("octaves", s.octaves.to_string()),
                ("gate", s.gate.to_string()),
                ("swing", s.swing.to_string()),
                ("velocity", velocity),
                ("accent", s.accent.to_string()),
                ("scale", scale),
                (
                    "root",
                    NOTE_NAMES[usize::from(u.scale_root % 12)].to_string(),
                ),
                ("snap", word(s.snap)),
                ("chord", word(s.chord)),
                ("run", on(s.run).to_string()),
                ("turns", word(s.turns)),
                ("strum", s.strum.to_string()),
                ("retrigger", on(s.retrigger).to_string()),
                ("repeats", s.repeats.to_string()),
                ("ramp", format!("{} {}", s.ramp.target, s.ramp.steps)),
                ("start", word(s.start)),
                (
                    "swing-from",
                    if u.swing_from_transport {
                        "transport"
                    } else {
                        "arp"
                    }
                    .to_string(),
                ),
                ("pattern-name", pattern_name),
                ("pattern", u.pattern.to_text()),
            ];
            let _ = writeln!(out, "track {}", t + 1);
            for (k, v) in lines {
                let _ = writeln!(out, "{k} = {v}");
            }
        }
        out
    }

    /// A project's arps: its "arp" part, or None (New, or a project saved
    /// before it had one), which is every arp at its default, never the
    /// last project's. Every arp lets go of its notes and Latch first.
    /// Returns the lines for the log and the effects that show the armed
    /// track's arp. Small allocations, once an open.
    pub fn open_arp(&mut self, text: Option<&str>) -> (Vec<String>, Vec<Effect>) {
        let mut notes = Vec::new();
        let first = super::built_in_patterns()
            .first()
            .map(|(_, p)| *p)
            .unwrap_or_default();
        let fresh = TrackUi {
            pattern: first,
            ..Default::default()
        };
        let mut kept = [fresh; MAX_TRACKS];
        let mut plays = [Mode::Direct; MAX_TRACKS];
        if let Some(text) = text {
            match self.read_arp_part(text, &mut kept, &mut plays) {
                Ok(left) if left.is_empty() => {}
                Ok(left) => notes.push(format!(
                    "project: arp settings left out (not understood, or the scale is gone): {}",
                    left.join(", ")
                )),
                Err(e) => {
                    notes.push(format!("project: the arps weren't opened ({e})"));
                    kept = [fresh; MAX_TRACKS];
                    plays = [Mode::Direct; MAX_TRACKS];
                }
            }
        }
        // Latch is how you play, not the song.
        self.latched = false;
        for t in 0..MAX_TRACKS {
            let e = self.arps.engine_at_mut(t);
            e.set_latch(LatchMode::Off);
            e.panic();
        }
        self.track_ui = kept;
        for (t, u) in kept.iter().enumerate() {
            if t == self.track {
                continue;
            }
            let mut s = u.arp;
            s.mode = plays[t];
            s.rate = u.rate;
            let e = self.arps.engine_at_mut(t);
            e.set_settings(s);
            e.set_pattern(u.pattern);
            e.set_own_swing(!u.swing_from_transport);
        }
        let mut fx = self.take_ui(kept[self.track]);
        fx.push(self.arp_view());
        self.arp_seen = self.arp_fingerprint();
        (notes, fx)
    }

    /// Reads the part into `kept` and `plays`. Returns what was left out
    /// ("track 2 gate = 300").
    fn read_arp_part(
        &self,
        text: &str,
        kept: &mut [TrackUi; MAX_TRACKS],
        plays: &mut [Mode; MAX_TRACKS],
    ) -> Result<Vec<String>, String> {
        let mut left = Vec::new();
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
        if lines.next() != Some(HEADER) {
            return Err(format!("it doesn't start with \"{HEADER}\""));
        }
        let mut track: Option<usize> = None;
        // The scale is looked up once the track's root is known.
        let mut scales: [Option<String>; MAX_TRACKS] = Default::default();
        for line in lines {
            if let Some(n) = line.strip_prefix("track ") {
                track = Some(
                    number(n.trim(), 1, MAX_TRACKS)
                        .ok_or_else(|| format!("no track {}", n.trim()))?
                        - 1,
                );
                continue;
            }
            let Some(t) = track else {
                return Err(format!("\"{line}\" comes before any track"));
            };
            let Some((k, v)) = line.split_once('=') else {
                return Err(format!("\"{line}\" isn't key = value"));
            };
            let (k, v) = (k.trim(), v.trim());
            let u = &mut kept[t];
            let s = &mut u.arp;
            let ok = match k {
                "mode" => pick(&super::MODES, v).map(|m| u.arp_mode = m).is_some(),
                "plays" => pick(&super::MODES, v).map(|m| plays[t] = m).is_some(),
                "rate" => Rate::ALL
                    .iter()
                    .find(|r| r.label() == v)
                    .map(|&r| u.rate = r)
                    .is_some(),
                "order" => pick(&Order::ALL, v).map(|o| s.order = o).is_some(),
                "octaves" => number(v, 1, 4).map(|n| s.octaves = n).is_some(),
                "gate" => number(v, 1, 100).map(|n| s.gate = n).is_some(),
                "swing" => number(v, 50, 75).map(|n| s.swing = n).is_some(),
                "velocity" => match v {
                    "played" => Some(Velocity::Played),
                    v => number(v, 1, 127).map(Velocity::Fixed),
                }
                .map(|x| s.velocity = x)
                .is_some(),
                "accent" => number(v, 1, 127).map(|n| s.accent = n).is_some(),
                "scale" => {
                    scales[t] = Some(v.to_string());
                    true
                }
                "root" => note_of(v).map(|n| u.scale_root = n).is_some(),
                "snap" => pick(&Snap::ALL, v).map(|x| s.snap = x).is_some(),
                "chord" => pick(&Chord::ALL, v).map(|x| s.chord = x).is_some(),
                "run" => flag(v).map(|b| s.run = b).is_some(),
                "turns" => pick(&Turns::ALL, v).map(|x| s.turns = x).is_some(),
                "strum" => number(v, 0, 100).map(|n| s.strum = n).is_some(),
                "retrigger" => flag(v).map(|b| s.retrigger = b).is_some(),
                "repeats" => v.parse().ok().map(|n| s.repeats = n).is_some(),
                "ramp" => v
                    .split_once(' ')
                    .and_then(|(a, b)| Some((number(a, 1, 127)?, b.trim().parse().ok()?)))
                    .map(|(target, steps)| (s.ramp.target, s.ramp.steps) = (target, steps))
                    .is_some(),
                "start" => pick(&SyncStart::ALL, v).map(|x| s.start = x).is_some(),
                "swing-from" => match v {
                    "transport" => Some(true),
                    "arp" => Some(false),
                    _ => None,
                }
                .map(|b| u.swing_from_transport = b)
                .is_some(),
                "pattern-name" => {
                    // A name the library no longer has: the pattern still
                    // plays (below), under the first one's name.
                    u.pattern_index = self.patterns.iter().position(|(n, _)| n == v).unwrap_or(0);
                    true
                }
                "pattern" => Pattern::parse(v).map(|p| u.pattern = p).is_ok(),
                _ => false,
            };
            if !ok {
                left.push(format!("track {} {k} = {v}", t + 1));
            }
        }
        for (t, name) in scales.iter().enumerate() {
            let u = &mut kept[t];
            let Some(name) = name else { continue };
            let found = if name == "off" {
                Some(0)
            } else {
                (1..self.scale_choices())
                    .find(|&i| self.scale_entry(i).is_some_and(|(n, ..)| n == name))
            };
            match found {
                Some(i) => {
                    u.scale_pick = i;
                    u.arp.scale = self
                        .scale_entry(i)
                        .map(|(_, up, down)| Scale::with_down(u.scale_root, up, down));
                }
                None => left.push(format!("track {} scale = {name}", t + 1)),
            }
        }
        Ok(left)
    }
}
