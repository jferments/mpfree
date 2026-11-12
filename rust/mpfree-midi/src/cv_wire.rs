//! The CV jacks on the wire: the bytes Akai's app sends to set them.
//!
//! From the owner's two read-only captures under Akai's app on stock 3.6
//! (2026-11-12, `docs/dev/hardware/controller-protocol.md`, "The rear
//! jacks under Akai's app"). This is bytes only. Nothing here opens or
//! sends anything: sending these to the controller is a state-changing
//! write and waits for the owner's go (`docs/dev/design/midi-io.md`, M6).
//!
//! What the captures showed (OBSERVED):
//!
//! - Akai's app sets the CV/Gate jacks with control changes on the
//!   controller's USB-MIDI cable [`CV_CABLE`] (1, "Private"), channel 16
//!   (status [`CV_STATUS`]).
//! - Each output is one 14-bit value sent as two CCs, the high 7 bits
//!   first: output 0 on CC 0 and 1, output 1 on CC 2 and 3, output 2 on
//!   CC 4 and 5, output 3 on CC 6 and 7 ([`ccs`]). Output `n` here is
//!   Akai's CV port `n + 1`. The pair belongs to the port, not to its
//!   use: Akai's CV track lets either its CV Port or its Gate Port be any
//!   port, as [`crate::cv::CvAssign`] does.
//! - A CV track's gate output goes to full scale (127 and 127) at a note
//!   and to 0 at its release.
//! - A CV track's pitch is `floor((note - 36) * 16383 / 60)` for every
//!   note played (48, 60, 61, 62, 63, 72 and 93; [`akai_pitch_counts`]).
//!   So 0 is note 36 and full scale is five octaves up.
//!
//! What is not known yet:
//!
//! - INFERENCE: outputs 4 to 7 on CC 8 to 15, following the pattern.
//! - INFERENCE: notes below 36 and above 96 clamp to the ends.
//! - UNKNOWN: the voltage at full scale. Five octaves at 1 V/oct would
//!   make it 5 V, but nobody has put a meter on a jack. [`counts`] takes
//!   it as a parameter for that reason.
//! - UNKNOWN: which jack, and tip or ring, each output number is.

/// The controller's USB-MIDI cable that carries the CV CCs (OBSERVED).
pub const CV_CABLE: u8 = 1;
/// Control change on MIDI channel 16 (OBSERVED).
pub const CV_STATUS: u8 = 0xBF;
/// One output's full scale: 14 bits (OBSERVED: a gate at 127 and 127).
pub const FULL_SCALE: u16 = 16383;
/// The note Akai's app sends as 0 (OBSERVED, from six notes).
pub const NOTE_AT_ZERO: u8 = 36;
/// Semitones from 0 to full scale in Akai's pitch (OBSERVED, from six
/// notes).
pub const PITCH_SPAN_SEMIS: u16 = 60;
/// Outputs with a CC pair: CC 0 to 15. Outputs 0 to 3 are OBSERVED and
/// 4 to 7 are INFERENCE.
pub const WIRE_OUTS: u8 = 8;

/// The CC pair (high 7 bits, low 7 bits) for output `out`, counted from
/// 0, or `None` past [`WIRE_OUTS`].
pub fn ccs(out: u8) -> Option<(u8, u8)> {
    (out < WIRE_OUTS).then(|| (2 * out, 2 * out + 1))
}

/// The value Akai's app sends for a CV track's `note`, exactly as seen.
/// Notes outside 36 to 96 clamp (INFERENCE).
pub fn akai_pitch_counts(note: u8) -> u16 {
    let semis = note
        .saturating_sub(NOTE_AT_ZERO)
        .min(PITCH_SPAN_SEMIS as u8);
    (u32::from(semis) * u32::from(FULL_SCALE) / u32::from(PITCH_SPAN_SEMIS)) as u16
}

/// A value for `volts`, given the voltage at full scale, rounded to the
/// nearest count and clamped to 0 and [`FULL_SCALE`].
///
/// `full_scale_volts` is not measured on the One yet (see the module
/// notes); 0 or less gives 0. Akai's own pitch truncates rather than
/// rounds, so a note can differ from [`akai_pitch_counts`] by one count
/// (about a third of a millivolt if full scale is 5 V).
pub fn counts(volts: f32, full_scale_volts: f32) -> u16 {
    if full_scale_volts <= 0.0 || volts.is_nan() {
        return 0;
    }
    let c = (volts / full_scale_volts * f32::from(FULL_SCALE)).round();
    c.clamp(0.0, f32::from(FULL_SCALE)) as u16
}

/// The two control changes that set output `out` to `counts`, high half
/// first, as Akai's app sends them. `None` past [`WIRE_OUTS`]; counts
/// above [`FULL_SCALE`] clamp.
pub fn encode(out: u8, counts: u16) -> Option<[[u8; 3]; 2]> {
    let (hi_cc, lo_cc) = ccs(out)?;
    let c = counts.min(FULL_SCALE);
    Some([
        [CV_STATUS, hi_cc, (c >> 7) as u8],
        [CV_STATUS, lo_cc, (c & 0x7f) as u8],
    ])
}

/// The value an output was set to, from its two data bytes (high, low).
pub fn decode(hi: u8, lo: u8) -> u16 {
    (u16::from(hi & 0x7f) << 7) | u16::from(lo & 0x7f)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every pitch the two captures saw: (note played, CC high, CC low).
    const SEEN: [(u8, u8, u8); 8] = [
        (48, 25, 76),
        (60, 51, 25),
        (72, 76, 101),
        (61, 53, 42),
        (62, 55, 59),
        (63, 57, 76),
        // Round 1's pads 1 and 13 on the CV track; their notes follow
        // from the formula, Akai's pad layout was not known.
        (72, 76, 101),
        (93, 121, 75),
    ];

    #[test]
    fn akai_pitch_matches_every_capture() {
        for (note, hi, lo) in SEEN {
            assert_eq!(akai_pitch_counts(note), decode(hi, lo), "note {note}");
            assert_eq!(
                encode(0, akai_pitch_counts(note)),
                Some([[0xBF, 0, hi], [0xBF, 1, lo]]),
                "note {note}"
            );
        }
    }

    #[test]
    fn outputs_use_cc_pairs() {
        assert_eq!(ccs(0), Some((0, 1)));
        assert_eq!(ccs(1), Some((2, 3)));
        assert_eq!(ccs(2), Some((4, 5)));
        assert_eq!(ccs(3), Some((6, 7)));
        assert_eq!(ccs(7), Some((14, 15)));
        assert_eq!(ccs(8), None);
        assert_eq!(encode(8, 0), None);
        // The second capture: CV Port 3 and Gate Port 4 (outputs 2 and
        // 3 here), pitch of note 60 and the gate.
        assert_eq!(
            encode(2, akai_pitch_counts(60)),
            Some([[0xBF, 4, 51], [0xBF, 5, 25]])
        );
        assert_eq!(
            encode(3, FULL_SCALE),
            Some([[0xBF, 6, 127], [0xBF, 7, 127]])
        );
        assert_eq!(encode(3, 0), Some([[0xBF, 6, 0], [0xBF, 7, 0]]));
    }

    #[test]
    fn ends_clamp() {
        assert_eq!(akai_pitch_counts(0), 0);
        assert_eq!(akai_pitch_counts(36), 0);
        assert_eq!(akai_pitch_counts(96), FULL_SCALE);
        assert_eq!(akai_pitch_counts(127), FULL_SCALE);
        assert_eq!(encode(0, u16::MAX), encode(0, FULL_SCALE));
        assert_eq!(counts(-1.0, 5.0), 0);
        assert_eq!(counts(9.0, 5.0), FULL_SCALE);
        assert_eq!(counts(1.0, 0.0), 0);
        assert_eq!(counts(f32::NAN, 5.0), 0);
    }

    #[test]
    fn volts_round_and_stay_within_a_count_of_akai() {
        // If full scale is 5 V (unmeasured), 1 V/oct from note 36 lands
        // within one count of what Akai's app sent for every note.
        for note in 36..=96u8 {
            let volts = f32::from(note - 36) / 12.0;
            let ours = i32::from(counts(volts, 5.0));
            let akai = i32::from(akai_pitch_counts(note));
            assert!((ours - akai).abs() <= 1, "note {note}: {ours} vs {akai}");
        }
        assert_eq!(counts(5.0, 5.0), FULL_SCALE);
        assert_eq!(counts(2.0, 5.0), 6553);
    }
}
