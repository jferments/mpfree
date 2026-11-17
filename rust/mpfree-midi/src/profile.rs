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
//! ```
//!
//! `match` (any number of them) is a USB name the profile is for: a port
//! whose profile is Auto ([`crate::setup::ProfilePick`]) takes the first
//! profile that matches the name the port calls itself. `nrpn.MSB.LSB`
//! names a parameter by its two numbers (CC 99 and CC 98); programs count
//! from 1, as everywhere on screen. A line that can't be read is skipped
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

/// The largest profile file read, in bytes.
pub const MAX_FILE_BYTES: usize = 64 * 1024;

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
            _ => return Err("unknown key".into()),
        }
        Ok(())
    }
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
        colour = red\n";

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
