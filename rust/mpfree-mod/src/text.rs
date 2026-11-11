//! A parameter's value as a short text for a knob card or a list.

use mpfree_node::{Curve, ParamInfo, Unit};

/// `v` in `p`'s units, short enough for a Q-Link card: "2.4 kHz",
/// "-3.0 dB", "62 %", "Tube", "On".
pub fn value_text(p: &ParamInfo, v: f32) -> String {
    let v = p.clamp(v);
    match p.curve {
        Curve::Choice(names) => return names.get(v as usize).copied().unwrap_or("?").to_string(),
        Curve::Toggle => return if v >= 0.5 { "On" } else { "Off" }.to_string(),
        _ => {}
    }
    let signed = |x: f32, d: usize, unit: &str| {
        if x > 0.0 {
            format!("+{x:.d$} {unit}")
        } else {
            format!("{x:.d$} {unit}")
        }
    };
    match p.unit {
        Unit::Hz if v >= 1000.0 => format!("{:.1} kHz", v / 1000.0),
        Unit::Hz if v < 10.0 => format!("{v:.2} Hz"),
        Unit::Hz => format!("{v:.0} Hz"),
        Unit::Db => signed(v, 1, "dB"),
        Unit::Percent => format!("{v:.0} %"),
        Unit::Ms if v >= 1000.0 => format!("{:.2} s", v / 1000.0),
        Unit::Ms if v < 10.0 => format!("{v:.1} ms"),
        Unit::Ms => format!("{v:.0} ms"),
        Unit::Semitones => signed(v, if p.curve == Curve::Stepped { 0 } else { 1 }, "st"),
        Unit::Cents => signed(v, 0, "ct"),
        Unit::Octaves => signed(v, 2, "oct"),
        Unit::Beats => format!("{v:.2} beats"),
        Unit::None if p.curve == Curve::Stepped => format!("{v:.0}"),
        // A plain 0 to 1 amount reads as a percentage.
        Unit::None if p.min == 0.0 && p.max == 1.0 => format!("{:.0} %", v * 100.0),
        Unit::None => format!("{v:.2}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: ParamInfo = ParamInfo {
        id: 0,
        name: "x",
        group: "g",
        min: 20.0,
        max: 20000.0,
        default: 1000.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    };

    #[test]
    fn texts() {
        assert_eq!(value_text(&P, 2400.0), "2.4 kHz");
        assert_eq!(value_text(&P, 440.0), "440 Hz");
        assert_eq!(
            value_text(
                &ParamInfo {
                    unit: Unit::Db,
                    min: -80.0,
                    max: 6.0,
                    curve: Curve::Linear,
                    default: 0.0,
                    ..P
                },
                -3.0
            ),
            "-3.0 dB"
        );
        assert_eq!(
            value_text(
                &ParamInfo {
                    unit: Unit::Db,
                    min: -80.0,
                    max: 12.0,
                    curve: Curve::Linear,
                    default: 0.0,
                    ..P
                },
                9.5
            ),
            "+9.5 dB"
        );
        assert_eq!(
            value_text(
                &ParamInfo {
                    unit: Unit::None,
                    min: 0.0,
                    max: 1.0,
                    curve: Curve::Linear,
                    default: 0.0,
                    ..P
                },
                0.62
            ),
            "62 %"
        );
        assert_eq!(
            value_text(
                &ParamInfo {
                    unit: Unit::None,
                    min: 0.0,
                    max: 2.0,
                    curve: Curve::Choice(&["a", "Tube", "c"]),
                    default: 0.0,
                    ..P
                },
                1.0
            ),
            "Tube"
        );
        assert_eq!(
            value_text(
                &ParamInfo {
                    unit: Unit::None,
                    min: 0.0,
                    max: 1.0,
                    curve: Curve::Toggle,
                    default: 0.0,
                    ..P
                },
                1.0
            ),
            "On"
        );
        assert_eq!(
            value_text(
                &ParamInfo {
                    unit: Unit::Ms,
                    min: 1.0,
                    max: 5000.0,
                    curve: Curve::Log,
                    default: 10.0,
                    ..P
                },
                1500.0
            ),
            "1.50 s"
        );
    }
}
