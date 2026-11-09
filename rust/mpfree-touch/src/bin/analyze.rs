//! M12 host-only analyzer: reads a saved capture log and prints the
//! calibration report. Runs on the workstation; opens only the log file.
#![forbid(unsafe_code)]

use mpfree_touch::calibrate::{analyze, parse_log, render, synthetic_log, CalibrationConfig};
use mpfree_touch::pointer::Orientation;

fn run() -> Result<bool, String> {
    let mut args = std::env::args().skip(1);
    let cfg = CalibrationConfig::default();
    match (args.next().as_deref(), args.next(), args.next()) {
        (Some("--self-test"), None, None) => {
            for o in Orientation::ALL {
                let log = parse_log(&synthetic_log(o, &cfg, 10))?;
                let report = analyze(&log, &cfg)?;
                if !report.pass || report.orientation != Some(o) {
                    return Err(format!("synthetic orientation {o:?} not recovered"));
                }
            }
            println!("MPFREE_TOUCH_ANALYZE_SELFTEST_V1");
            println!("orientations_recovered=8");
            println!("device_opened=no");
            println!("status=PASS");
            Ok(true)
        }
        (Some(path), None, None) if !path.starts_with('-') => {
            let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
            let log = parse_log(&text)?;
            let report = analyze(&log, &cfg)?;
            print!("{}", render(&report, &cfg));
            Ok(report.pass)
        }
        _ => Err("usage: mpfree-touch-analyze --self-test | <capture log>".into()),
    }
}

fn main() {
    match run() {
        Ok(true) => {}
        Ok(false) => std::process::exit(2),
        Err(e) => {
            eprintln!("MPFREE TOUCH ANALYZE ERROR: {e}");
            std::process::exit(1);
        }
    }
}
