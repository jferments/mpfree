//! How a parameter's value reads on screen: a number and a unit, written for
//! a player (260 Hz, 2.5 kHz, 220 ms, 1.20 s, +3.2 oct, LP 24, On).

use mpfree_node::{Curve, ParamInfo, Unit};

/// The value text and its unit (which may be empty).
pub fn value_text(p: &ParamInfo, v: f32) -> (String, &'static str) {
    let v = p.clamp(v);
    let bipolar = p.min < 0.0 && p.max > 0.0;
    match p.curve {
        Curve::Choice(names) => return (names.get(v as usize).copied().unwrap_or("?").to_string(), ""),
        Curve::Toggle => return ((if v >= 0.5 { "On" } else { "Off" }).to_string(), ""),
        _ => {}
    }
    let stepped = p.curve == Curve::Stepped;
    match p.unit {
        Unit::Hz if v >= 1000.0 => (trim(v / 1000.0, if v >= 10_000.0 { 1 } else { 2 }), "kHz"),
        Unit::Hz => (trim(v, if v >= 100.0 { 0 } else if v >= 10.0 { 1 } else { 2 }), "Hz"),
        Unit::Ms if v >= 1000.0 => (trim(v / 1000.0, 2), "s"),
        Unit::Ms => (trim(v, if v >= 100.0 || v == v.round() { 0 } else { 1 }), "ms"),
        Unit::Db => (signed(v, if v.abs() >= 10.0 && !bipolar { 0 } else { 1 }, bipolar), "dB"),
        Unit::Percent => (signed(v, 0, bipolar), "%"),
        Unit::Semitones => (signed(v, if stepped { 0 } else { 1 }, bipolar || stepped), "st"),
        Unit::Cents => (signed(v, 0, bipolar), "ct"),
        Unit::Octaves => (signed(v, 1, bipolar), "oct"),
        Unit::Beats => (trim(v, 2), "beats"),
        Unit::None if stepped => (signed(v, 0, bipolar), ""),
        Unit::None => (signed(v, 2, bipolar), ""),
    }
}

/// A pressure slot's reach at full pressure, such as "+1.5 oct" or "9 dB
/// swell": `amount` in the target's `unit` (see `mpfree_synth::pressure`).
/// A level depth is a swell when positive and a duck when negative.
pub fn amount_text(target: &str, amount: f32, unit: Unit) -> (String, &'static str) {
    if target == "Level" {
        let n = trim(amount.abs(), 1);
        return (n, if amount < 0.0 { "dB duck" } else { "dB swell" });
    }
    let p = ParamInfo { id: 0, name: "", group: "", min: -1e6, max: 1e6, default: 0.0, unit, curve: Curve::Linear, readonly: false };
    let short = |v: f32| {
        let s = signed(v, 2, true);
        if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
    };
    match unit {
        Unit::Octaves | Unit::Semitones => (short(amount), value_text(&p, amount).1),
        // Brightness and feedback are in steps (an FM output level step is
        // 0.75 dB).
        Unit::None => (short((amount * 10.0).round() / 10.0), "steps"),
        _ => value_text(&p, amount),
    }
}

/// The value and unit on one line, as the Q-Link strip shows it.
pub fn value_line(p: &ParamInfo, v: f32) -> String {
    let (n, u) = value_text(p, v);
    if u.is_empty() { n } else { format!("{n} {u}") }
}

fn signed(v: f32, decimals: usize, sign: bool) -> String {
    let s = format!("{:.*}", decimals, v);
    // "-0" and "-0.0" read as zero.
    let zero = s.trim_start_matches('-').chars().all(|c| c == '0' || c == '.');
    if zero {
        return s.trim_start_matches('-').to_string();
    }
    if sign && v > 0.0 { format!("+{s}") } else { s }
}

fn trim(v: f32, decimals: usize) -> String {
    let s = format!("{:.*}", decimals, v);
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_synth::params::{info, CUTOFF, FENV_AMOUNT, FENV_D, FILTER_MODE, LEVEL, LFO_RETRIG, OSC1_OCT, OSC2_FINE, PAN};

    fn t(id: u16, v: f32) -> String {
        value_line(info(id).unwrap(), v)
    }

    #[test]
    fn values_read_like_a_player_would_say_them() {
        assert_eq!(t(CUTOFF, 260.0), "260 Hz");
        assert_eq!(t(CUTOFF, 2500.0), "2.5 kHz");
        assert_eq!(t(CUTOFF, 12_345.0), "12.3 kHz");
        assert_eq!(t(CUTOFF, 55.5), "55.5 Hz");
        assert_eq!(t(FENV_D, 220.0), "220 ms");
        assert_eq!(t(FENV_D, 1500.0), "1.5 s");
        assert_eq!(t(FENV_AMOUNT, 3.2), "+3.2 oct");
        assert_eq!(t(FENV_AMOUNT, -1.0), "-1.0 oct");
        assert_eq!(t(FENV_AMOUNT, 0.0), "0.0 oct");
        assert_eq!(t(FILTER_MODE, 2.0), "BP 12");
        assert_eq!(t(LFO_RETRIG, 1.0), "On");
        assert_eq!(t(OSC1_OCT, -1.0), "-1");
        assert_eq!(t(OSC1_OCT, 2.0), "+2");
        assert_eq!(t(OSC2_FINE, 7.0), "+7 ct");
        assert_eq!(t(LEVEL, 6.0), "+6.0 dB");
        assert_eq!(t(LEVEL, -0.01), "0.0 dB");
        assert_eq!(t(PAN, 0.0), "0 %");
    }

    #[test]
    fn values_out_of_range_are_clamped_first() {
        assert_eq!(t(CUTOFF, 1e9), "20 kHz");
        assert_eq!(t(FILTER_MODE, 99.0), "HP 12");
        assert_eq!(t(CUTOFF, f32::NAN), "2.5 kHz", "NaN reads as the default");
    }
}
