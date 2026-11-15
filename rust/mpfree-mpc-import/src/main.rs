//! `mpfree-mpc-import`: MPC programs into MPFree projects, on a computer.
//!
//! ```text
//! mpfree-mpc-import keygroup PROGRAM.xpm --data DIR [--name NAME] [--release-ms MS] [--dry-run]
//! ```
//!
//! Reads a keygroup program and the samples beside it (as the MPC saves a
//! program folder) and writes a new project into the library in `DIR`
//! (the folder the app is given with `--data`), then prints the report.
//! It only reads the program folder and never replaces a project.

use std::path::PathBuf;
use std::process::ExitCode;

use mpfree_mpc_import::import::import_keygroup;
use mpfree_mpc_import::kit::Options;

const USAGE: &str = "usage: mpfree-mpc-import keygroup PROGRAM.xpm --data DIR [--name NAME] [--release-ms MS] [--dry-run]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mpfree-mpc-import: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return Ok(());
    }
    let mut it = args.iter();
    if it.next().map(String::as_str) != Some("keygroup") {
        return Err(USAGE.into());
    }
    let (mut xpm, mut data, mut name, mut dry) = (None, None, None, false);
    let mut opts = Options::default();
    while let Some(a) = it.next() {
        let mut value = || {
            it.next()
                .cloned()
                .ok_or(format!("{a} needs a value\n{USAGE}"))
        };
        match a.as_str() {
            "--data" => data = Some(PathBuf::from(value()?)),
            "--name" => name = Some(value()?),
            "--release-ms" => {
                let v = value()?;
                opts.release_ms = v
                    .parse::<f32>()
                    .ok()
                    .filter(|ms| (1.0..=20_000.0).contains(ms))
                    .ok_or(format!("--release-ms {v}: give 1 to 20000"))?;
            }
            "--dry-run" => dry = true,
            s if s.starts_with('-') => return Err(format!("unknown option {s}\n{USAGE}")),
            s if xpm.is_none() => xpm = Some(PathBuf::from(s)),
            s => return Err(format!("one program at a time ({s})\n{USAGE}")),
        }
    }
    let xpm = xpm.ok_or(USAGE)?;
    let data = data.ok_or(format!("--data DIR is needed\n{USAGE}"))?;
    let report = import_keygroup(&xpm, &data, name.as_deref(), &opts, dry)?;
    for line in &report.lines {
        println!("{line}");
    }
    Ok(())
}
