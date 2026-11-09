//! Offscreen host-only preview of the future MPC One MPFree GUI test card.
//! No framebuffer, ALSA, evdev, DRM, network or stock service operations.
#![forbid(unsafe_code)]

use mpfree_display::{QuarterTurn, render_test_card};
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};

fn save_ppm(path: &str, rotation: Option<QuarterTurn>) -> Result<(), String> {
    let logical = render_test_card().map_err(|e| format!("render error: {e:?}"))?;
    let image = match rotation {
        Some(turn) => logical.rotate_for_panel(turn)
            .map_err(|e| format!("rotate error: {e:?}"))?,
        None => logical,
    };
    // Only explicit host CLI --preview* can create an output file.
    // create_new avoids destroying an existing sample, project or document.
    let file = OpenOptions::new()
        .write(true).create_new(true).open(path)
        .map_err(|e| format!("cannot create new preview {path:?}: {e}"))?;
    let mut out = BufWriter::new(file);
    write!(out, "P6\n{} {}\n255\n", image.width(), image.height())
        .and_then(|_| out.write_all(image.rgb()))
        .and_then(|_| out.flush())
        .map_err(|e| format!("preview write failed: {e}"))?;
    println!("MPFREE_OFFSCREEN_TEST_CARD_V1");
    println!("backend=offscreen");
    println!("hardware_access=none");
    println!("width={}", image.width());
    println!("height={}", image.height());
    println!("checksum_fnv1a64={:016x}", image.checksum_fnv1a64());
    println!("preview={path}");
    println!("status=PASS");
    Ok(())
}

fn self_test() -> Result<(), String> {
    let logical = render_test_card().map_err(|e| format!("{e:?}"))?;
    let cw = logical.rotate_for_panel(QuarterTurn::Clockwise)
        .map_err(|e| format!("{e:?}"))?;
    let ccw = logical.rotate_for_panel(QuarterTurn::CounterClockwise)
        .map_err(|e| format!("{e:?}"))?;
    if cw.width() != 800 || cw.height() != 1280 ||
       ccw.width() != 800 || ccw.height() != 1280 {
        return Err("native panel dimensions mismatch".into());
    }
    println!("MPFREE_OFFSCREEN_TEST_CARD_V1");
    println!("backend=offscreen");
    println!("hardware_access=none");
    println!("width={}", logical.width());
    println!("height={}", logical.height());
    println!("native_width={}", cw.width());
    println!("native_height={}", cw.height());
    println!("checksum_fnv1a64={:016x}", logical.checksum_fnv1a64());
    println!("status=PASS");
    Ok(())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    match args.as_slice() {
        [_] | [_, flag] if flag == "--self-test" => self_test(),
        [_, flag, path] if flag == "--preview-logical" =>
            save_ppm(path, None),
        [_, flag, path] if flag == "--preview-cw" =>
            save_ppm(path, Some(QuarterTurn::Clockwise)),
        [_, flag, path] if flag == "--preview-ccw" =>
            save_ppm(path, Some(QuarterTurn::CounterClockwise)),
        _ => Err("usage: mpfree-display --self-test | \
          --preview-logical NEW_FILE.ppm | --preview-cw NEW_FILE.ppm | \
          --preview-ccw NEW_FILE.ppm".into()),
    }
}

fn main() {
    if let Err(err) = run() {
        eprintln!("MPFree offscreen preview: {err}");
        std::process::exit(1);
    }
}
