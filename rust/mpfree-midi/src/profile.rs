//! Device profiles: what a synth or a controller calls its controls
//! (plan item 3.3, board 6's Profile). A profile names CCs, NRPNs and
//! programs ("CC 74 Cutoff"); the Monitor, its Now view and the Learn
//! section use the names of the profile on the port a message came in on
//! or went out to, and the controllers most gear agrees on
//! ([`common_cc_name`]) where it names none.
//!
//! A profile is a text file, the same `key = value` lines as the project's
//! MIDI part ([`HEADER`] first):
//!
//! ```text
//! mpfree-profile 1
//! name = Juno-X
//! match = Juno-X
//! cc.74 = Cutoff
//! cc.71 = Resonance
//! nrpn.0.23 = Osc 1 shape
//! program.1 = Init
//! knobs = 74 71 76 77
//! pad.A1 = 36 ch 10
//! pad.B3 = F#1
//! mpe = lower 15
//! ```
//!
//! `match` (any number of them) is a USB name the profile is for: a port
//! whose profile is Auto ([`crate::setup::ProfilePick`]) takes the first
//! profile that matches the name the port calls itself. `nrpn.MSB.LSB`
//! names a parameter by its two numbers (CC 99 and CC 98); programs count
//! from 1, as everywhere on screen. `knobs` is a controller's knobs' CCs
//! in order, which MIDI Follow turns the Q-Links with
//! ([`crate::setup::FollowSetup`]). `pad.A1` to `pad.H16` (or `pad.1` to
//! `pad.128`) is the note a kit pad plays on a drum machine, as a number
//! or a name (C4 is 60), with its own channel after `ch` if it has one
//! (Jay's card 17 Nov 23:28Z: drum pads out by the gear's profile); a pad
//! it leaves out plays [`kit_note`]. `mpe` is an MPE controller's zone
//! (`lower` or `upper`, with its member channels, or `off`;
//! [`crate::mpe::Zone`]), for one that doesn't say so itself. A line that
//! can't be read is skipped
//! with a warning; a file without a name isn't a profile.
//!
//! [`Profiles`] holds the built-in profiles ([`Profiles::builtin`]: General
//! MIDI) and the ones the musician put in the store's
//! `files/midi-profiles/` (the app reads them at start, read only). Two
//! profiles never share a name: a later one with a name already taken is
//! refused.

use std::fmt::Write as _;

/// A profile file's first line.
pub const HEADER: &str = "mpfree-profile 1";

/// The most profiles there can be, built-in ones included.
pub const MAX_PROFILES: usize = 64;

/// The longest profile name, in characters (it shows on the port's page).
pub const MAX_NAME: usize = 32;

/// The longest name for a control or a program, in characters.
pub const MAX_CONTROL_NAME: usize = 32;

/// The most `match` lines a profile keeps.
pub const MAX_MATCHES: usize = 8;

/// The most knobs a profile lists.
pub const MAX_KNOBS: usize = 16;

/// The largest profile file read, in bytes.
pub const MAX_FILE_BYTES: usize = 64 * 1024;

/// A kit's pads: 8 banks (A to H) of 16.
pub const KIT_PADS: usize = 128;

/// Pads in a bank.
const BANK_PADS: usize = 16;

/// Each kit pad's note on MIDI Out when the port's profile names none:
/// bank A as the MPC sends its pads (pad 1 is C#2, 37), and banks B to H
/// the other 112 notes in turn from 35 (B1, the General MIDI kit's
/// lowest drum) up, then from 0. So every pad has a note of its own, and
/// banks B and C hold the General MIDI drums bank A lacks.
pub const KIT_NOTES: [u8; KIT_PADS] = kit_notes();

const fn kit_notes() -> [u8; KIT_PADS] {
    let a = mpfree_controller::mpc_one::PAD_NOTES;
    let mut out = [0u8; KIT_PADS];
    let mut used = [false; 128];
    let mut i = 0;
    while i < BANK_PADS {
        out[i] = a[i];
        used[a[i] as usize] = true;
        i += 1;
    }
    let mut k = 0;
    while k < 128 {
        let n = (35 + k) % 128;
        if !used[n] {
            out[i] = n as u8;
            i += 1;
        }
        k += 1;
    }
    out
}

/// Kit pad `pad`'s note when no profile names one ([`KIT_NOTES`]; pads
/// past the last play none).
pub fn kit_note(pad: usize) -> Option<u8> {
    KIT_NOTES.get(pad).copied()
}

/// A pad's name on screen: A1 to H16.
pub fn pad_name(pad: usize) -> String {
    let bank = (b'A' + (pad / BANK_PADS).min(7) as u8) as char;
    format!("{bank}{}", pad % BANK_PADS + 1)
}

/// What a profile has a kit pad play: a note, and a channel of its own
/// (0 to 15) or the track's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PadOut {
    pub note: u8,
    pub channel: Option<u8>,
}

/// What one piece of gear calls its controls and programs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    /// The USB names it is for (a port set to Auto takes it).
    pub matches: Vec<String>,
    /// CC names by number, in number order.
    cc: Vec<(u8, String)>,
    /// NRPN names by MSB × 128 + LSB, in number order.
    nrpn: Vec<(u16, String)>,
    /// Program names by number (0 is the screen's 1), in number order.
    programs: Vec<(u8, String)>,
    /// A controller's knobs' CCs, in order (MIDI Follow's).
    pub knobs: Vec<u8>,
    /// What a drum machine's kit pads play, by pad (0 is A1), in pad
    /// order.
    pads: Vec<(u8, PadOut)>,
    /// An MPE controller's zone, if the profile says.
    pub mpe: Option<crate::mpe::Zone>,
}

impl Profile {
    /// An empty profile called `name`.
    pub fn new(name: &str) -> Profile {
        Profile {
            name: name.to_string(),
            ..Profile::default()
        }
    }

    /// What it calls CC `cc`.
    pub fn cc_name(&self, cc: u8) -> Option<&str> {
        find(&self.cc, cc)
    }

    /// What it calls NRPN `msb`, `lsb`.
    pub fn nrpn_name(&self, msb: u8, lsb: u8) -> Option<&str> {
        find(&self.nrpn, u16::from(msb) * 128 + u16::from(lsb))
    }

    /// What it calls program `program` (0 is the screen's 1).
    pub fn program_name(&self, program: u8) -> Option<&str> {
        find(&self.programs, program)
    }

    /// How many CCs, NRPNs and programs it names.
    pub fn counts(&self) -> (usize, usize, usize) {
        (self.cc.len(), self.nrpn.len(), self.programs.len())
    }

    /// What kit pad `pad` (0 is A1) plays, if it says.
    pub fn pad(&self, pad: usize) -> Option<PadOut> {
        let pad = u8::try_from(pad).ok()?;
        self.pads
            .binary_search_by_key(&pad, |e| e.0)
            .ok()
            .map(|i| self.pads[i].1)
    }

    /// How many kit pads it maps.
    pub fn pads(&self) -> usize {
        self.pads.len()
    }

    /// Has kit pad `pad` (0 is A1, up to 127) play `out`.
    pub fn set_pad(&mut self, pad: u8, out: PadOut) {
        let out = PadOut {
            note: out.note.min(127),
            channel: out.channel.map(|c| c.min(15)),
        };
        match self.pads.binary_search_by_key(&pad, |e| e.0) {
            Ok(i) => self.pads[i].1 = out,
            Err(i) => self.pads.insert(i, (pad, out)),
        }
    }

    /// Names CC `cc` (a later name for the same CC replaces it).
    pub fn set_cc(&mut self, cc: u8, name: &str) {
        put(&mut self.cc, cc, name);
    }

    /// Names NRPN `msb`, `lsb`.
    pub fn set_nrpn(&mut self, msb: u8, lsb: u8, name: &str) {
        put(&mut self.nrpn, u16::from(msb) * 128 + u16::from(lsb), name);
    }

    /// Names program `program` (0 is the screen's 1).
    pub fn set_program(&mut self, program: u8, name: &str) {
        put(&mut self.programs, program, name);
    }

    /// Whether it is for the port that calls itself `own` (a `match`
    /// line, whatever the capitals and spaces around it).
    pub fn matches(&self, own: &str) -> bool {
        let own = own.trim();
        !own.is_empty() && self.matches.iter().any(|m| m.eq_ignore_ascii_case(own))
    }

    /// The profile as a file ([`Profile::parse`] reads it back).
    pub fn to_text(&self) -> String {
        let mut s = format!("{HEADER}\nname = {}\n", self.name);
        for m in &self.matches {
            let _ = writeln!(s, "match = {m}");
        }
        for (cc, n) in &self.cc {
            let _ = writeln!(s, "cc.{cc} = {n}");
        }
        for (k, n) in &self.nrpn {
            let _ = writeln!(s, "nrpn.{}.{} = {n}", k / 128, k % 128);
        }
        for (p, n) in &self.programs {
            let _ = writeln!(s, "program.{} = {n}", u16::from(*p) + 1);
        }
        if !self.knobs.is_empty() {
            let k: Vec<String> = self.knobs.iter().map(u8::to_string).collect();
            let _ = writeln!(s, "knobs = {}", k.join(" "));
        }
        for (pad, o) in &self.pads {
            let _ = write!(s, "pad.{} = {}", pad_name(usize::from(*pad)), o.note);
            if let Some(c) = o.channel {
                let _ = write!(s, " ch {}", c + 1);
            }
            s.push('\n');
        }
        if let Some(z) = self.mpe {
            let _ = writeln!(s, "mpe = {}", z.to_text());
        }
        s
    }

    /// Reads a profile file: the profile, and a warning for each line it
    /// skipped. Refused (with why) when it has no name, a name that is too
    /// long or one the port page's choices use (Auto, None).
    pub fn parse(text: &str) -> Result<(Profile, Vec<String>), String> {
        if text.len() > MAX_FILE_BYTES {
            return Err(format!("larger than {} KB", MAX_FILE_BYTES / 1024));
        }
        if text.trim().is_empty() {
            return Err("it is empty".into());
        }
        let mut p = Profile::default();
        let mut warn = Vec::new();
        for (n, raw) in text.lines().enumerate() {
            let n = n + 1;
            let l = raw.trim().trim_start_matches('\u{feff}');
            // The header first; a file without one is still read.
            if n == 1 {
                if l == HEADER {
                    continue;
                }
                warn.push(format!("line 1: expected \"{HEADER}\", found \"{l}\""));
            }
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            let Some((k, v)) = l.split_once('=') else {
                warn.push(format!("line {n}: not \"key = value\": {l}"));
                continue;
            };
            if let Err(e) = p.apply(k.trim(), v.trim()) {
                warn.push(format!("line {n}: {}: {e}", k.trim()));
            }
        }
        check_name(&p.name)?;
        Ok((p, warn))
    }

    fn apply(&mut self, k: &str, v: &str) -> Result<(), String> {
        let parts: Vec<&str> = k.split('.').collect();
        match parts.as_slice() {
            ["name"] => {
                check_name(v)?;
                self.name = v.to_string();
            }
            ["match"] => {
                if v.is_empty() {
                    return Err("no name".into());
                }
                if self.matches.len() >= MAX_MATCHES {
                    return Err(format!("at most {MAX_MATCHES} match lines"));
                }
                self.matches.push(v.to_string());
            }
            ["cc", n] => {
                let cc = number(n, 0, 127)?;
                self.set_cc(cc as u8, control_name(v)?);
            }
            ["nrpn", msb, lsb] => {
                let (msb, lsb) = (number(msb, 0, 127)?, number(lsb, 0, 127)?);
                self.set_nrpn(msb as u8, lsb as u8, control_name(v)?);
            }
            ["program", n] => {
                let p = number(n, 1, 128)?;
                self.set_program((p - 1) as u8, control_name(v)?);
            }
            ["knobs"] => {
                let mut k = Vec::new();
                for w in v.split_whitespace() {
                    let cc = number(w, 0, 119)? as u8;
                    if k.contains(&cc) {
                        return Err(format!("CC {cc} twice"));
                    }
                    if k.len() == MAX_KNOBS {
                        return Err(format!("more than {MAX_KNOBS} knobs"));
                    }
                    k.push(cc);
                }
                self.knobs = k;
            }
            ["pad", which] => {
                let pad = pad_number(which)?;
                let mut w = v.split_whitespace();
                let note = w.next().ok_or("no note")?;
                let note = note_number(note)?;
                let channel = match (w.next(), w.next(), w.next()) {
                    (None, ..) => None,
                    (Some(c), Some(n), None) if c.eq_ignore_ascii_case("ch") => {
                        Some(number(n, 1, 16)? as u8 - 1)
                    }
                    _ => return Err("expected a note, then \"ch\" and a channel".into()),
                };
                self.set_pad(pad, PadOut { note, channel });
            }
            ["mpe"] => self.mpe = Some(crate::mpe::Zone::parse(v)?),
            _ => return Err("unknown key".into()),
        }
        Ok(())
    }
}

/// A pad by its name (A1 to H16) or number (1 to 128): 0 is A1.
fn pad_number(w: &str) -> Result<u8, String> {
    if let Ok(n) = w.parse::<u32>() {
        return match n {
            1..=128 => Ok((n - 1) as u8),
            _ => Err(format!("no pad {n}: pads are 1 to 128, or A1 to H16")),
        };
    }
    let mut c = w.chars();
    let bank = c.next().map(|b| b.to_ascii_uppercase());
    let pad = c.as_str().parse::<usize>().ok();
    match (bank, pad) {
        (Some(b @ 'A'..='H'), Some(p @ 1..=16)) => {
            Ok(((b as u8 - b'A') as usize * BANK_PADS + p - 1) as u8)
        }
        _ => Err(format!("no pad \"{w}\": pads are A1 to H16, or 1 to 128")),
    }
}

/// A note by number (0 to 127) or name (C-1 to G9, C4 is 60; sharps with
/// #, flats with b).
pub(crate) fn note_number(w: &str) -> Result<u8, String> {
    if let Ok(n) = w.parse::<u32>() {
        return match n {
            0..=127 => Ok(n as u8),
            _ => Err(format!("no note {n}: notes are 0 to 127")),
        };
    }
    let bad = || format!("\"{w}\" is not a note (a number from 0 to 127, or a name such as C#2)");
    let mut c = w.chars();
    let base: i32 = match c.next().map(|l| l.to_ascii_uppercase()) {
        Some('C') => 0,
        Some('D') => 2,
        Some('E') => 4,
        Some('F') => 5,
        Some('G') => 7,
        Some('A') => 9,
        Some('B') => 11,
        _ => return Err(bad()),
    };
    let rest = c.as_str();
    let (shift, octave) = match rest.strip_prefix('#') {
        Some(o) => (1, o),
        None => match rest.strip_prefix('b') {
            Some(o) => (-1, o),
            None => (0, rest),
        },
    };
    let octave: i32 = octave.parse().map_err(|_| bad())?;
    let n = (octave + 1) * 12 + base + shift;
    u8::try_from(n).ok().filter(|&n| n < 128).ok_or_else(bad)
}

/// The names the port page's Profile choices use for themselves.
const RESERVED: [&str; 2] = ["auto", "none"];

fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("no name (a \"name = ...\" line)".into());
    }
    if name.chars().count() > MAX_NAME {
        return Err(format!("a name longer than {MAX_NAME} characters"));
    }
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(name)) {
        return Err(format!("\"{name}\" is a word the port page uses"));
    }
    Ok(())
}

fn control_name(v: &str) -> Result<&str, String> {
    if v.is_empty() {
        return Err("no name".into());
    }
    if v.chars().count() > MAX_CONTROL_NAME {
        return Err(format!("longer than {MAX_CONTROL_NAME} characters"));
    }
    Ok(v)
}

fn number(v: &str, lo: u32, hi: u32) -> Result<u32, String> {
    match v.parse::<u32>() {
        Ok(n) if (lo..=hi).contains(&n) => Ok(n),
        _ => Err(format!("\"{v}\" is not a number from {lo} to {hi}")),
    }
}

fn find<K: Ord + Copy>(list: &[(K, String)], k: K) -> Option<&str> {
    list.binary_search_by_key(&k, |e| e.0)
        .ok()
        .map(|i| list[i].1.as_str())
}

fn put<K: Ord + Copy>(list: &mut Vec<(K, String)>, k: K, name: &str) {
    match list.binary_search_by_key(&k, |e| e.0) {
        Ok(i) => list[i].1 = name.to_string(),
        Err(i) => list.insert(i, (k, name.to_string())),
    }
}

/// The controllers most gear agrees on: what a port without a profile
/// (or a CC its profile leaves out) shows.
pub fn common_cc_name(cc: u8) -> Option<&'static str> {
    Some(match cc {
        0 => "Bank select",
        1 => "Mod wheel",
        2 => "Breath",
        4 => "Foot",
        5 => "Glide time",
        6 => "Data entry",
        7 => "Volume",
        8 => "Balance",
        10 => "Pan",
        11 => "Expression",
        32 => "Bank select fine",
        64 => "Sustain",
        65 => "Glide",
        66 => "Sostenuto",
        67 => "Soft pedal",
        71 => "Resonance",
        74 => "Cutoff",
        120 => "All sound off",
        121 => "Reset controllers",
        122 => "Local control",
        123 => "All notes off",
        124 => "Omni off",
        125 => "Omni on",
        126 => "Mono",
        127 => "Poly",
        _ => return None,
    })
}

/// The General MIDI (level 1) sound set's 128 programs, in order.
pub const GM_PROGRAMS: [&str; 128] = [
    // Piano
    "Acoustic Grand Piano",
    "Bright Acoustic Piano",
    "Electric Grand Piano",
    "Honky-tonk Piano",
    "Electric Piano 1",
    "Electric Piano 2",
    "Harpsichord",
    "Clavi",
    // Chromatic percussion
    "Celesta",
    "Glockenspiel",
    "Music Box",
    "Vibraphone",
    "Marimba",
    "Xylophone",
    "Tubular Bells",
    "Dulcimer",
    // Organ
    "Drawbar Organ",
    "Percussive Organ",
    "Rock Organ",
    "Church Organ",
    "Reed Organ",
    "Accordion",
    "Harmonica",
    "Tango Accordion",
    // Guitar
    "Acoustic Guitar (nylon)",
    "Acoustic Guitar (steel)",
    "Electric Guitar (jazz)",
    "Electric Guitar (clean)",
    "Electric Guitar (muted)",
    "Overdriven Guitar",
    "Distortion Guitar",
    "Guitar harmonics",
    // Bass
    "Acoustic Bass",
    "Electric Bass (finger)",
    "Electric Bass (pick)",
    "Fretless Bass",
    "Slap Bass 1",
    "Slap Bass 2",
    "Synth Bass 1",
    "Synth Bass 2",
    // Strings
    "Violin",
    "Viola",
    "Cello",
    "Contrabass",
    "Tremolo Strings",
    "Pizzicato Strings",
    "Orchestral Harp",
    "Timpani",
    // Ensemble
    "String Ensemble 1",
    "String Ensemble 2",
    "SynthStrings 1",
    "SynthStrings 2",
    "Choir Aahs",
    "Voice Oohs",
    "Synth Voice",
    "Orchestra Hit",
    // Brass
    "Trumpet",
    "Trombone",
    "Tuba",
    "Muted Trumpet",
    "French Horn",
    "Brass Section",
    "SynthBrass 1",
    "SynthBrass 2",
    // Reed
    "Soprano Sax",
    "Alto Sax",
    "Tenor Sax",
    "Baritone Sax",
    "Oboe",
    "English Horn",
    "Bassoon",
    "Clarinet",
    // Pipe
    "Piccolo",
    "Flute",
    "Recorder",
    "Pan Flute",
    "Blown Bottle",
    "Shakuhachi",
    "Whistle",
    "Ocarina",
    // Synth lead
    "Lead 1 (square)",
    "Lead 2 (sawtooth)",
    "Lead 3 (calliope)",
    "Lead 4 (chiff)",
    "Lead 5 (charang)",
    "Lead 6 (voice)",
    "Lead 7 (fifths)",
    "Lead 8 (bass + lead)",
    // Synth pad
    "Pad 1 (new age)",
    "Pad 2 (warm)",
    "Pad 3 (polysynth)",
    "Pad 4 (choir)",
    "Pad 5 (bowed)",
    "Pad 6 (metallic)",
    "Pad 7 (halo)",
    "Pad 8 (sweep)",
    // Synth effects
    "FX 1 (rain)",
    "FX 2 (soundtrack)",
    "FX 3 (crystal)",
    "FX 4 (atmosphere)",
    "FX 5 (brightness)",
    "FX 6 (goblins)",
    "FX 7 (echoes)",
    "FX 8 (sci-fi)",
    // Ethnic
    "Sitar",
    "Banjo",
    "Shamisen",
    "Koto",
    "Kalimba",
    "Bag pipe",
    "Fiddle",
    "Shanai",
    // Percussive
    "Tinkle Bell",
    "Agogo",
    "Steel Drums",
    "Woodblock",
    "Taiko Drum",
    "Melodic Tom",
    "Synth Drum",
    "Reverse Cymbal",
    // Sound effects
    "Guitar Fret Noise",
    "Breath Noise",
    "Seashore",
    "Bird Tweet",
    "Telephone Ring",
    "Helicopter",
    "Applause",
    "Gunshot",
];

/// The controllers General MIDI gear answers to, with the effects sends
/// and the parameter numbers it adds to the common ones.
const GM_CCS: [(u8, &str); 17] = [
    (0, "Bank select"),
    (1, "Mod wheel"),
    (6, "Data entry"),
    (7, "Volume"),
    (10, "Pan"),
    (11, "Expression"),
    (32, "Bank select fine"),
    (38, "Data entry fine"),
    (64, "Sustain"),
    (91, "Reverb"),
    (93, "Chorus"),
    (98, "NRPN fine"),
    (99, "NRPN"),
    (100, "RPN fine"),
    (101, "RPN"),
    (121, "Reset controllers"),
    (123, "All notes off"),
];

/// The General MIDI profile: its controllers and its 128 programs.
pub fn general_midi() -> Profile {
    let mut p = Profile::new("General MIDI");
    for (cc, n) in GM_CCS {
        p.set_cc(cc, n);
    }
    for (k, n) in GM_PROGRAMS.iter().enumerate() {
        p.set_program(k as u8, n);
    }
    p
}

/// Every profile there is: the built-in ones, then the musician's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profiles {
    list: Vec<Profile>,
    /// How many of `list` are built in (they come first).
    builtin: usize,
}

impl Default for Profiles {
    fn default() -> Self {
        Profiles::builtin()
    }
}

impl Profiles {
    /// The built-in profiles: General MIDI.
    pub fn builtin() -> Profiles {
        let list = vec![general_midi()];
        Profiles {
            builtin: list.len(),
            list,
        }
    }

    /// Adds the musician's `p`. Refused, with why, when another profile
    /// has its name (whatever the capitals) or there are
    /// [`MAX_PROFILES`] already.
    pub fn add(&mut self, p: Profile) -> Result<(), String> {
        if self.get(&p.name).is_some() {
            return Err(format!("another profile is called {}", p.name));
        }
        if self.list.len() >= MAX_PROFILES {
            return Err(format!("only {MAX_PROFILES} profiles are kept"));
        }
        self.list.push(p);
        Ok(())
    }

    /// The profile called `name` (whatever the capitals).
    pub fn get(&self, name: &str) -> Option<&Profile> {
        self.list.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// All of them, the built-in ones first.
    pub fn all(&self) -> &[Profile] {
        &self.list
    }

    /// Whether the profile at `i` in [`Profiles::all`] is built in.
    pub fn is_builtin(&self, i: usize) -> bool {
        i < self.builtin
    }

    /// The first profile for a port that calls itself `own`.
    pub fn matching(&self, own: &str) -> Option<&Profile> {
        self.list.iter().find(|p| p.matches(own))
    }

    /// The profile a port uses: the one picked, else for Auto the one
    /// that matches the name it calls itself (`own`).
    pub fn of(&self, pick: &crate::setup::ProfilePick, own: &str) -> Option<&Profile> {
        use crate::setup::ProfilePick;
        match pick {
            ProfilePick::Auto => self.matching(own),
            ProfilePick::Off => None,
            ProfilePick::Named(n) => self.get(n),
        }
    }
}

/// What CC `cc` is called on a port using `profile`: its profile's name,
/// else the common one.
pub fn cc_name(profile: Option<&Profile>, cc: u8) -> Option<&str> {
    profile
        .and_then(|p| p.cc_name(cc))
        .or_else(|| common_cc_name(cc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::ProfilePick;

    const JUNO: &str = "mpfree-profile 1\n\
        # a synth on USB\n\
        name = Juno-X\n\
        match = Juno-X\n\
        match = JUNO-X MIDI 1\n\
        cc.74 = Filter cutoff\n\
        cc.71 = Filter resonance\n\
        cc.200 = Too far\n\
        nrpn.1.23 = Osc 1 shape\n\
        program.1 = Init\n\
        program.0 = Nowhere\n\
        colour = red\n\
        knobs = 74 71\n";

    #[test]
    fn a_profile_names_its_controls_and_programs_and_skips_what_it_cant_read() {
        let (p, warn) = Profile::parse(JUNO).unwrap();
        assert_eq!(p.name, "Juno-X");
        assert_eq!(p.cc_name(74), Some("Filter cutoff"));
        assert_eq!(p.cc_name(1), None);
        assert_eq!(p.nrpn_name(1, 23), Some("Osc 1 shape"));
        assert_eq!(p.nrpn_name(23, 1), None);
        assert_eq!(p.program_name(0), Some("Init"));
        assert_eq!(p.counts(), (2, 1, 1));
        assert_eq!(p.knobs, [74, 71]);
        assert_eq!(warn.len(), 3, "{warn:?}");
        assert!(warn[0].starts_with("line 8: cc.200"), "{warn:?}");
        assert!(warn[2].contains("unknown key"), "{warn:?}");
        // Its own name, whatever the capitals; not another's.
        assert!(p.matches("juno-x midi 1 "));
        assert!(!p.matches("Juno"));
        assert!(!p.matches(""));
        // Written and read back as it was.
        let (back, w) = Profile::parse(&p.to_text()).unwrap();
        assert_eq!((back, w), (p, vec![]));
    }

    /// A drum machine's profile gives kit pads their notes and channels,
    /// by bank name or number, by note number or name; what it can't read
    /// is skipped with why, and it writes back as it read.
    #[test]
    fn a_drum_machines_profile_gives_kit_pads_notes_and_channels() {
        let text = "mpfree-profile 1\nname = Drum box\n\
            pad.A1 = 36 ch 10\npad.a2 = D2\npad.17 = F#1 CH 11\npad.H16 = 127\n\
            pad.B2 = Bb-1\npad.I1 = 36\npad.0 = 36\npad.129 = 36\npad.A17 = 36\n\
            pad.A3 = 128\npad.A4 = H2\npad.A5 = 36 ch 17\npad.A6 = 36 ch\n\
            pad.A7 = G9\npad.A8 = G#9\npad.A9\n";
        let (p, warn) = Profile::parse(text).unwrap();
        let out = |note, channel| Some(PadOut { note, channel });
        assert_eq!(p.pad(0), out(36, Some(9)));
        assert_eq!(p.pad(1), out(38, None));
        assert_eq!(p.pad(16), out(30, Some(10)), "pad 17 is B1");
        assert_eq!(p.pad(17), out(10, None), "Bb-1");
        assert_eq!(p.pad(127), out(127, None));
        assert_eq!(p.pad(6), out(127, None), "G9 is the top note");
        assert_eq!(p.pad(2), None);
        assert_eq!(p.pad(128), None);
        assert_eq!(p.pads(), 6);
        // I1, 0, 129, A17, 128, H2, ch 17, a bare ch, G#9, no value.
        assert_eq!(warn.len(), 10, "{warn:?}");
        assert!(warn[0].contains("no pad \"I1\""), "{warn:?}");
        assert!(warn[4].contains("no note 128"), "{warn:?}");
        assert!(warn[9].contains("not \"key = value\""), "{warn:?}");
        let (back, w) = Profile::parse(&p.to_text()).unwrap();
        assert!(
            p.to_text().contains("pad.B1 = 30 ch 11\n"),
            "{}",
            p.to_text()
        );
        assert_eq!((back, w), (p, vec![]));
    }

    /// An MPE controller's profile names its zone; a bad zone is skipped
    /// with why.
    #[test]
    fn an_mpe_controllers_profile_names_its_zone() {
        use crate::mpe::Zone;
        let (p, warn) =
            Profile::parse("mpfree-profile 1\nname = Seaboard\nmpe = lower 15\n").unwrap();
        assert_eq!((p.mpe, warn.len()), (Some(Zone::Lower(15)), 0));
        assert!(p.to_text().contains("mpe = lower 15\n"));
        assert_eq!(Profile::parse(&p.to_text()).unwrap().0, p);
        let (p, warn) = Profile::parse("mpfree-profile 1\nname = X\nmpe = sideways\n").unwrap();
        assert_eq!(p.mpe, None);
        assert!(
            warn[0].contains("\"sideways\" is not off, lower or upper"),
            "{warn:?}"
        );
    }

    /// Without a profile's note, bank A plays the MPC's own pad notes and
    /// banks B to H every other note once, from 35 up then from 0.
    #[test]
    fn every_kit_pad_has_a_note_of_its_own() {
        assert_eq!(KIT_NOTES[..16], mpfree_controller::mpc_one::PAD_NOTES);
        let mut seen = [false; 128];
        for n in KIT_NOTES {
            assert!(
                !std::mem::replace(&mut seen[usize::from(n)], true),
                "{n} twice"
            );
        }
        assert_eq!(KIT_NOTES[16..19], [35, 39, 41]);
        assert_eq!(kit_note(16), Some(35));
        assert_eq!(kit_note(127), Some(34));
        assert_eq!(kit_note(128), None);
        assert_eq!(
            (pad_name(0), pad_name(16), pad_name(127)),
            ("A1".into(), "B1".into(), "H16".into())
        );
    }

    #[test]
    fn a_file_without_a_usable_name_is_not_a_profile() {
        assert!(Profile::parse("").is_err());
        assert!(Profile::parse("mpfree-profile 1\ncc.1 = Mod\n").is_err());
        assert!(Profile::parse("mpfree-profile 1\nname = Auto\n").is_err());
        assert!(Profile::parse("mpfree-profile 1\nname = none\n").is_err());
        let long = format!("mpfree-profile 1\nname = {}\n", "x".repeat(MAX_NAME + 1));
        assert!(Profile::parse(&long).is_err());
        let big = format!(
            "mpfree-profile 1\nname = Big\n{}",
            "#\n".repeat(MAX_FILE_BYTES)
        );
        assert!(Profile::parse(&big).is_err());
        // Another first line is a warning, not a refusal.
        let (p, w) = Profile::parse("name = Bare\ncc.7 = Level\n").unwrap();
        assert_eq!((p.name.as_str(), w.len()), ("Bare", 1));
        assert_eq!(p.cc_name(7), Some("Level"));
    }

    #[test]
    fn general_midi_names_its_programs_and_sends() {
        let gm = general_midi();
        assert_eq!(gm.program_name(0), Some("Acoustic Grand Piano"));
        assert_eq!(gm.program_name(33), Some("Electric Bass (finger)"));
        assert_eq!(gm.program_name(127), Some("Gunshot"));
        assert_eq!(gm.cc_name(91), Some("Reverb"));
        assert_eq!(gm.counts(), (17, 0, 128));
        let (back, w) = Profile::parse(&gm.to_text()).unwrap();
        assert_eq!((back, w), (gm, vec![]));
    }

    #[test]
    fn a_port_uses_the_profile_it_picked_or_the_one_that_matches_it() {
        let mut all = Profiles::builtin();
        assert!(all.is_builtin(0));
        let (juno, _) = Profile::parse(JUNO).unwrap();
        all.add(juno).unwrap();
        assert!(!all.is_builtin(1));
        // No two with one name, whatever the capitals.
        assert!(all.add(Profile::new("general midi")).is_err());
        let named = |n: &str| ProfilePick::Named(n.into());
        let of = |pick: &ProfilePick, own: &str| all.of(pick, own).map(|p| p.name.clone());
        assert_eq!(of(&ProfilePick::Auto, "Juno-X"), Some("Juno-X".into()));
        assert_eq!(of(&ProfilePick::Auto, "Keystep 37"), None);
        assert_eq!(of(&ProfilePick::Off, "Juno-X"), None);
        assert_eq!(
            of(&named("General MIDI"), "Juno-X"),
            Some("General MIDI".into())
        );
        assert_eq!(
            of(&named("Gone"), "Juno-X"),
            None,
            "a profile no longer there"
        );
        // Its names first, then the common ones.
        let juno = all.get("JUNO-X");
        assert_eq!(cc_name(juno, 74), Some("Filter cutoff"));
        assert_eq!(cc_name(juno, 64), Some("Sustain"));
        assert_eq!(cc_name(None, 74), Some("Cutoff"));
        assert_eq!(cc_name(None, 3), None);
        // Full.
        for k in all.all().len()..MAX_PROFILES {
            all.add(Profile::new(&format!("P{k}"))).unwrap();
        }
        assert!(all.add(Profile::new("One more")).is_err());
    }
}
