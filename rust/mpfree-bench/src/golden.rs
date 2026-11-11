//! Golden regression checks: every bench case (every synth preset, the drum
//! kits, the plucks, every effect, the sampler, the mixer, the arp and time
//! engine, the looper) rendered for two seconds and compared with a stored
//! fingerprint, `golden/cases.txt`.
//!
//! The looper is rendered for 8 s instead: it records its first 4 s and only
//! then plays back.
//!
//! A fingerprint is per eighth of the render, per channel: the level (RMS, in dB)
//! and the zero crossings (a rough pitch). It is compared within a tolerance,
//! not bit for bit, so a different libm or CPU passes while a changed sound
//! (a preset, a filter, a tuning, a missing voice, silence, a blow-up) fails.
//!
//! When a sound changes on purpose, write the new references and say why in
//! the commit:
//!
//!     MPFREE_GOLDEN_BLESS=1 cargo test -p mpfree-bench golden
//!
//! (docs/guides/testing-and-ci.md).

use crate::cases::{self, Case};
use std::fmt::Write as _;
use std::path::PathBuf;

const RATE: u32 = 44_100;
const BLOCK: usize = 128;
/// Two seconds of blocks.
const BLOCKS: u64 = 2 * RATE as u64 / BLOCK as u64;
/// Segments per fingerprint: a quarter second each in two seconds.
const SEGMENTS: u64 = 8;
/// Below this a segment counts as silent, whatever its exact level.
const FLOOR_DB: f64 = -90.0;
/// Level tolerance per segment.
const TOLERANCE_DB: f64 = 0.5;

/// One quarter second of one channel.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Seg {
    db: f64,
    zc: u32,
}

#[derive(Clone, Debug, PartialEq)]
struct Print {
    left: Vec<Seg>,
    right: Vec<Seg>,
}

fn db(sum_sq: f64, n: u64) -> f64 {
    let rms = (sum_sq / n as f64).sqrt();
    if rms > 0.0 {
        (20.0 * rms.log10()).max(-120.0)
    } else {
        -120.0
    }
}

/// Blocks to render `c` for: the looper only plays back after its 4 s loop.
fn blocks_for(c: &Case) -> u64 {
    if c.module == "looper" {
        4 * BLOCKS
    } else {
        BLOCKS
    }
}

/// Renders `blocks` blocks of `c`. Err names the first non-finite sample.
fn fingerprint(c: &mut Case, blocks: u64) -> Result<Print, String> {
    let segment = blocks / SEGMENTS;
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    let mut p = Print {
        left: Vec::new(),
        right: Vec::new(),
    };
    let (mut sl, mut sr) = (0.0f64, 0.0f64);
    let (mut zl, mut zr) = (0u32, 0u32);
    let (mut pl, mut pr) = (0.0f32, 0.0f32);
    for b in 0..segment * SEGMENTS {
        (c.block)(b, &mut l, &mut r);
        for (i, (&a, &z)) in l.iter().zip(r.iter()).enumerate() {
            if !a.is_finite() || !z.is_finite() {
                return Err(format!("non-finite sample at block {b}, frame {i}"));
            }
            sl += f64::from(a) * f64::from(a);
            sr += f64::from(z) * f64::from(z);
            zl += u32::from((a >= 0.0) != (pl >= 0.0));
            zr += u32::from((z >= 0.0) != (pr >= 0.0));
            (pl, pr) = (a, z);
        }
        if (b + 1) % segment == 0 {
            let n = segment * BLOCK as u64;
            p.left.push(Seg {
                db: db(sl, n),
                zc: zl,
            });
            p.right.push(Seg {
                db: db(sr, n),
                zc: zr,
            });
            (sl, sr, zl, zr) = (0.0, 0.0, 0, 0);
        }
    }
    Ok(p)
}

fn line(name: &str, p: &Print) -> String {
    let mut s = format!("{name} |");
    for (ch, segs) in [("L", &p.left), ("R", &p.right)] {
        write!(s, " {ch}").unwrap();
        for g in segs.iter() {
            write!(s, " {:.1}/{}", g.db, g.zc).unwrap();
        }
    }
    s
}

fn parse(line: &str) -> Option<(String, Print)> {
    let (name, rest) = line.rsplit_once(" |")?;
    let mut p = Print {
        left: Vec::new(),
        right: Vec::new(),
    };
    let mut side = None;
    for tok in rest.split_whitespace() {
        match tok {
            "L" => side = Some(0),
            "R" => side = Some(1),
            _ => {
                let (d, z) = tok.split_once('/')?;
                let g = Seg {
                    db: d.parse().ok()?,
                    zc: z.parse().ok()?,
                };
                match side? {
                    0 => p.left.push(g),
                    _ => p.right.push(g),
                }
            }
        }
    }
    Some((name.to_string(), p))
}

/// Why `got` differs from `want`, or None when it is within tolerance.
fn differs(want: &Print, got: &Print) -> Option<String> {
    for (ch, w, g) in [
        ("left", &want.left, &got.left),
        ("right", &want.right, &got.right),
    ] {
        if w.len() != g.len() {
            return Some(format!("{ch}: {} segments, not {}", g.len(), w.len()));
        }
        for (i, (a, b)) in w.iter().zip(g.iter()).enumerate() {
            let silent = a.db < FLOOR_DB && b.db < FLOOR_DB;
            if !silent && (a.db - b.db).abs() > TOLERANCE_DB {
                return Some(format!(
                    "{ch} segment {i}: {:.1} dB, not {:.1} dB",
                    b.db, a.db
                ));
            }
            // Crossings: within 3 % and 2, unless the segment is silent.
            let slack = (f64::from(a.zc) * 0.03).max(2.0);
            if !silent && (f64::from(a.zc) - f64::from(b.zc)).abs() > slack {
                return Some(format!(
                    "{ch} segment {i}: {} zero crossings, not {}",
                    b.zc, a.zc
                ));
            }
        }
    }
    None
}

fn reference_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("golden/cases.txt")
}

const HEADER: &str = "\
# mpfree-bench golden fingerprints: 2 s of each case at 44.1 kHz (the looper 8 s), in
# 8 segments per channel, level in dB / zero crossings. Written by
#   MPFREE_GOLDEN_BLESS=1 cargo test -p mpfree-bench golden
# Compared within 0.5 dB and 3 % of crossings (src/golden.rs).
";

#[test]
fn golden_every_case_sounds_as_it_did() {
    let mut got = Vec::new();
    let mut broken = Vec::new();
    for mut c in cases::all(RATE) {
        let blocks = blocks_for(&c);
        match fingerprint(&mut c, blocks) {
            Ok(p) => got.push((c.name, p)),
            Err(e) => broken.push(format!("{}: {e}", c.name)),
        }
    }
    assert!(
        broken.is_empty(),
        "cases made non-finite audio:\n{}",
        broken.join("\n")
    );

    if std::env::var_os("MPFREE_GOLDEN_BLESS").is_some() {
        let mut text = HEADER.to_string();
        for (name, p) in &got {
            assert!(
                !name.contains(" |"),
                "a case name may not contain \" |\": {name}"
            );
            text.push_str(&line(name, p));
            text.push('\n');
        }
        std::fs::create_dir_all(reference_path().parent().unwrap()).unwrap();
        std::fs::write(reference_path(), text).unwrap();
        eprintln!(
            "wrote {} fingerprints to {}",
            got.len(),
            reference_path().display()
        );
        return;
    }

    let text = std::fs::read_to_string(reference_path()).unwrap_or_else(|e| {
        panic!(
            "{}: {e} (bless with MPFREE_GOLDEN_BLESS=1)",
            reference_path().display()
        )
    });
    let want: Vec<(String, Print)> = text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            parse(l).unwrap_or_else(|| panic!("bad line in {}: {l}", reference_path().display()))
        })
        .collect();
    let mut problems = Vec::new();
    for (name, p) in &got {
        match want.iter().find(|(n, _)| n == name) {
            None => problems.push(format!("{name}: new case, no reference")),
            Some((_, w)) => {
                if let Some(why) = differs(w, p) {
                    problems.push(format!("{name}: {why}"));
                }
            }
        }
    }
    for (name, _) in &want {
        if !got.iter().any(|(n, _)| n == name) {
            problems.push(format!("{name}: in the references but no longer a case"));
        }
    }
    assert!(
        problems.is_empty(),
        "{} of {} cases sound different from golden/cases.txt:\n{}\n\
         If the change is meant, bless the new sound: MPFREE_GOLDEN_BLESS=1 cargo test -p mpfree-bench golden",
        problems.len(),
        got.len(),
        problems.join("\n")
    );
}

#[test]
fn fingerprints_read_back_and_compare() {
    let p = Print {
        left: vec![Seg { db: -12.3, zc: 440 }, Seg { db: -120.0, zc: 0 }],
        right: vec![Seg { db: -6.0, zc: 100 }, Seg { db: -95.0, zc: 7 }],
    };
    let text = line("Poly \"Init\", 8 voices", &p);
    let (name, back) = parse(&text).unwrap();
    assert_eq!(name, "Poly \"Init\", 8 voices");
    assert_eq!(back, p);
    assert_eq!(differs(&p, &back), None);

    let mut louder = p.clone();
    louder.left[0].db += 0.6;
    assert!(differs(&p, &louder).unwrap().contains("left segment 0"));
    let mut sharper = p.clone();
    sharper.right[0].zc = 110;
    assert!(differs(&p, &sharper).unwrap().contains("zero crossings"));
    // Both under the floor: silent either way, whatever the exact numbers.
    let mut quiet = p.clone();
    quiet.right[1] = Seg { db: -110.0, zc: 30 };
    assert_eq!(differs(&p, &quiet), None);
    let mut short = p.clone();
    short.left.pop();
    assert!(differs(&p, &short).is_some());
}

#[test]
fn levels_in_db() {
    assert_eq!(db(0.0, 10), -120.0);
    assert!((db(1.0, 1) - 0.0).abs() < 1e-12);
    assert!((db(0.25, 1) + 6.0206).abs() < 1e-3);
}
