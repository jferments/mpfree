//! The MPC's effect names and what plays each in MPFree: an effect and,
//! where one fits, a factory preset. For finding an effect by the name a
//! musician knows from the MPC, and for opening MPC projects later.
//!
//! The names are the MPC 2.x software's, from its user guide's "Effects
//! and Parameters" appendix and the 2.4 and 2.10 release notes (the AIR
//! effects). Many MPC entries are variants of one effect (Delay Mono,
//! Delay Sync, Delay Ping Pong...); here they share an MPFree effect and
//! differ by preset. An MPC effect MPFree doesn't have yet has no effect
//! and says so in its note.

use crate::registry;

/// One MPC effect name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MpcName {
    /// The name as the MPC shows it.
    pub mpc: &'static str,
    /// The MPFree effect's node id, or `None` if there isn't one yet.
    pub effect: Option<&'static str>,
    /// The factory preset that comes closest, if any.
    pub preset: Option<&'static str>,
    /// What to set, or what is missing, in a few words (may be empty).
    pub note: &'static str,
}

const fn n(
    mpc: &'static str,
    effect: &'static str,
    preset: &'static str,
    note: &'static str,
) -> MpcName {
    MpcName {
        mpc,
        effect: Some(effect),
        preset: if preset.is_empty() {
            None
        } else {
            Some(preset)
        },
        note,
    }
}

const fn none(mpc: &'static str, note: &'static str) -> MpcName {
    MpcName {
        mpc,
        effect: None,
        preset: None,
        note,
    }
}

const ROOM: &str = "mpfree.fx.room";
const CLOUD: &str = "mpfree.fx.cloud";
const DELAY: &str = "mpfree.fx.delay";
const TAPE: &str = "mpfree.fx.tape_delay";
const FILTER: &str = "mpfree.fx.filter";
const EQ: &str = "mpfree.fx.eq";
const DRIVE: &str = "mpfree.fx.drive";
const COMP: &str = "mpfree.fx.compressor";
const CRUSH: &str = "mpfree.fx.crush";

/// Every MPC effect name, in the MPC guide's order (legacy effects, then
/// the AIR effects of 2.4, then those of 2.10).
pub static MPC_NAMES: &[MpcName] = &[
    // Reverbs.
    n("Reverb Small", ROOM, "Small booth", ""),
    n("Reverb Medium", ROOM, "Drum room", ""),
    n("Reverb Large", CLOUD, "Big hall", ""),
    n("Reverb Large 2", ROOM, "Big studio", ""),
    n("Reverb In Gate", ROOM, "Gated snare", "Gate sets the cut"),
    // Delays.
    n("Delay Mono", DELAY, "", "Sync Free"),
    n("Delay Mono Sync", DELAY, "Quarter repeats", ""),
    n("Delay Stereo", DELAY, "", "Sync Free"),
    n("Delay Sync", DELAY, "Quarter repeats", ""),
    n("Delay Ping Pong", DELAY, "Dotted ping-pong", ""),
    n("Delay Analog", DELAY, "", "Tone near 3 kHz, Saturation up"),
    n(
        "Delay Analog Sync",
        DELAY,
        "",
        "Tone near 3 kHz, Saturation up",
    ),
    n("Delay Tape", TAPE, "Space heads", ""),
    n("Delay Tape Sync", TAPE, "Space heads", "Sync on"),
    n(
        "Delay Multi-Tap",
        "mpfree.fx.multitap",
        "Wide ping",
        "four taps",
    ),
    n("Delay LP", DELAY, "", "Tone is the low-pass"),
    n(
        "Delay HP",
        DELAY,
        "",
        "a Multi filter after it for the high-pass",
    ),
    // Modulation.
    n("Flanger", "mpfree.fx.flanger", "Gentle flange", ""),
    n("Flanger Sync", "mpfree.fx.flanger", "Two-bar sweep", ""),
    n("Chorus 2-Voice", "mpfree.fx.chorus", "Subtle widen", ""),
    n("Chorus 4-Voice", "mpfree.fx.chorus", "Lush ensemble", ""),
    n("Autopan", "mpfree.fx.autopan", "Gentle drift", ""),
    n("Autopan Sync", "mpfree.fx.autopan", "Hard ping", ""),
    n("Tremolo", "mpfree.fx.tremolo", "Amp wobble", ""),
    n("Tremolo Sync", "mpfree.fx.tremolo", "Eighth chop", ""),
    n("Phaser 1", "mpfree.fx.phaser", "Slow swirl", ""),
    n("Phaser 2", "mpfree.fx.phaser", "Deep phase", ""),
    n("Phaser Sync", "mpfree.fx.phaser", "Quarter pulse", ""),
    // Filters and EQ.
    n("HP Filter", FILTER, "Thin high-pass", ""),
    n(
        "HP Filter Sweep",
        FILTER,
        "Thin high-pass",
        "no LFO: sweep with XY FX",
    ),
    n(
        "HP Filter Sync",
        FILTER,
        "Thin high-pass",
        "no LFO: sweep with XY FX",
    ),
    n("HP Shelving Filter", EQ, "", "a high shelf band"),
    n("LP Filter", FILTER, "Warm low-pass", ""),
    n(
        "LP Filter Sweep",
        FILTER,
        "Warm low-pass",
        "no LFO: sweep with XY FX",
    ),
    n(
        "LP Filter Sync",
        FILTER,
        "Warm low-pass",
        "no LFO: sweep with XY FX",
    ),
    n("LP Shelving Filter", EQ, "", "a low shelf band"),
    n("PEQ 2-Band, 2-Shelf", EQ, "", ""),
    n("PEQ 4-Band", EQ, "", ""),
    // Distortion.
    n("Distortion Amp", "mpfree.fx.amp", "Classic crunch", ""),
    n("Distortion Fuzz", DRIVE, "Fuzz box", ""),
    n("Distortion Grimey", DRIVE, "Foldback grit", ""),
    n("Distortion Overdrive", DRIVE, "Tube glow", ""),
    n("Distortion Custom", DRIVE, "", "pick a Mode"),
    // Dynamics.
    n("Compressor", COMP, "", ""),
    n("Compressor Master", COMP, "Gentle glue", ""),
    n("Compressor Opto", COMP, "Vocal level", ""),
    n("Compressor VCA", COMP, "Drum punch", ""),
    n(
        "Compressor Vintage",
        COMP,
        "Gentle glue",
        "Character after it for colour",
    ),
    n("Oldskool", COMP, "Smash", ""),
    // Bit reducers.
    n("Decimator", CRUSH, "12-bit classic", ""),
    n("Resampler", CRUSH, "Soft grit", ""),
    // Other.
    none("Auto Wah", "not yet: no envelope-follower filter"),
    none("Frequency Shifter", "not yet"),
    n("Transient Shaper", "mpfree.fx.transient", "", ""),
    n("XYFX", "mpfree.fx.xy", "", ""),
    // AIR effects, MPC 2.4.
    n("AIR Distortion", DRIVE, "Hard clip", ""),
    n("AIR Enhancer", "mpfree.fx.exciter", "Vocal air", ""),
    n("AIR Ensemble", "mpfree.fx.chorus", "Lush ensemble", ""),
    n(
        "AIR Filter Gate",
        "mpfree.fx.tremolo",
        "Eighth chop",
        "Shape Square",
    ),
    n("AIR Flanger", "mpfree.fx.flanger", "Jet sweep", ""),
    none("AIR Freq Shift", "not yet"),
    n("AIR Fuzz-Wah", DRIVE, "Fuzz box", "no wah yet"),
    n("AIR Kill EQ", EQ, "", "cut bands at -inf; or the DJ filter"),
    n(
        "AIR Lo-Fi",
        "mpfree.fx.lofi",
        "Worn tape",
        "Crush for the bits",
    ),
    n("AIR Multi-Chorus", "mpfree.fx.chorus", "Dimension", ""),
    n("AIR Non-Linear Reverb", ROOM, "Gated snare", ""),
    n("AIR Phaser", "mpfree.fx.phaser", "Deep phase", ""),
    n("AIR Reverb", CLOUD, "Big hall", ""),
    n("AIR Spring Reverb", "mpfree.fx.spring", "Amp spring", ""),
    none("AIR Stereo Width", "not yet"),
    none("AIR Talkbox", "not yet"),
    none(
        "AIR Channel Strip",
        "not as one effect: EQ, Compressor and Gate",
    ),
    n("AIR Compressor", COMP, "", ""),
    n("AIR Delay", DELAY, "", ""),
    n("AIR Diff Delay", CLOUD, "Echo chamber", ""),
    n("AIR Filter", FILTER, "", ""),
    n("AIR Maximizer", "mpfree.fx.limiter", "Loud master", ""),
    n("AIR Noise Gate", "mpfree.fx.gate", "Soft cleanup", ""),
    n("AIR Para EQ", EQ, "", ""),
    n("AIR Pitch Shifter", "mpfree.fx.pitch", "", ""),
    n("AIR Pumper", "mpfree.fx.ducker", "Big pump", ""),
    n("AIR Transient", "mpfree.fx.transient", "", ""),
    n("AIR Tube Drive", DRIVE, "Tube glow", ""),
    n("Mother Ducker", "mpfree.fx.ducker", "Kick duck", ""),
    // MPC 2.10.
    n("Half Speed", "mpfree.fx.perform", "", "the half speed pad"),
    n("Stutter", "mpfree.fx.perform", "", "the stutter pad"),
    n("Diode Clipper", DRIVE, "Hard clip", ""),
    n("AIR Diffuser Delay", CLOUD, "Echo chamber", ""),
    none("Granulator", "not as an effect yet"),
    none("Sample Delay", "not yet"),
    n("AIR Limiter", "mpfree.fx.limiter", "Safe peaks", ""),
    none("AIR Vocal Tuner", "coming with Pitch correct"),
    n("AIR Vocal Doubler", "mpfree.fx.pitch", "Wide doubler", ""),
    none("AIR Vocal Harmonizer", "coming with the Harmonizer"),
];

/// The entry for an MPC effect name (any case, spaces trimmed).
pub fn lookup(name: &str) -> Option<&'static MpcName> {
    let name = name.trim();
    MPC_NAMES.iter().find(|m| m.mpc.eq_ignore_ascii_case(name))
}

/// The MPC names an MPFree effect stands in for (for search and its
/// picker tile), in the table's order.
pub fn names_for(effect: &str) -> impl Iterator<Item = &'static str> + '_ {
    MPC_NAMES
        .iter()
        .filter(move |m| m.effect == Some(effect))
        .map(|m| m.mpc)
}

/// How many MPC names have an MPFree effect, and how many there are.
pub fn covered() -> (usize, usize) {
    let have = MPC_NAMES.iter().filter(|m| m.effect.is_some()).count();
    (have, MPC_NAMES.len())
}

/// Checks the table against the registry: every effect exists, every
/// preset is one of its effect's, no name twice. The problems found.
pub fn problems() -> Vec<String> {
    let mut out = Vec::new();
    for (i, m) in MPC_NAMES.iter().enumerate() {
        if MPC_NAMES[..i]
            .iter()
            .any(|o| o.mpc.eq_ignore_ascii_case(m.mpc))
        {
            out.push(format!("{} twice", m.mpc));
        }
        match (m.effect, m.preset) {
            (None, Some(_)) => out.push(format!("{}: a preset with no effect", m.mpc)),
            (None, None) if m.note.is_empty() => {
                out.push(format!("{}: say what is missing", m.mpc))
            }
            (Some(id), preset) => match registry::entry(id) {
                None => out.push(format!("{}: no effect {id}", m.mpc)),
                Some(e) => {
                    if let Some(p) = preset {
                        if !e.presets.iter().any(|q| q.name == p) {
                            out.push(format!("{}: {id} has no preset {p}", m.mpc));
                        }
                    }
                }
            },
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_names_real_effects_and_presets() {
        assert_eq!(problems(), Vec::<String>::new());
        let (have, all) = covered();
        assert!(have * 10 >= all * 8, "{have} of {all} covered");
    }

    #[test]
    fn lookups_ignore_case_and_find_the_stand_ins() {
        let m = lookup("  delay ping pong ").unwrap();
        assert_eq!(m.effect, Some(DELAY));
        assert_eq!(m.preset, Some("Dotted ping-pong"));
        assert!(lookup("Nothing Like This").is_none());
        let tape: Vec<_> = names_for(TAPE).collect();
        assert_eq!(tape, ["Delay Tape", "Delay Tape Sync"]);
        assert_eq!(lookup("XYFX").unwrap().effect, Some("mpfree.fx.xy"));
    }
}
