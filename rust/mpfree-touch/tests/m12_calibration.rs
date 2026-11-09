//! M12 host-only tests: synthetic step-labelled capture logs through the
//! analyzer. No device is opened; every log here is generated in-process.

use mpfree_touch::calibrate::*;
use mpfree_touch::pointer::Orientation;

fn cfg() -> CalibrationConfig {
    CalibrationConfig::default()
}

fn run(text: &str) -> Report {
    analyze(&parse_log(text).expect("log parses"), &cfg()).expect("analysis runs")
}

/// Split a log into (step name, block lines) pairs.
fn blocks(text: &str) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("STEP ") {
            out.push((name.to_string(), Vec::new()));
        } else if line.starts_with(STEPS_HEADER) || line.starts_with(STEPS_END) {
        } else {
            out.last_mut().unwrap().1.push(line.to_string());
        }
    }
    out
}

/// Rebuild a well-formed log from blocks, fixing every per-step footer count
/// and the steps footer.
fn assemble(blocks: &[(String, Vec<String>)]) -> String {
    let mut out = format!("{STEPS_HEADER}\n");
    for (name, lines) in blocks {
        out.push_str(&format!("STEP {name}\n"));
        let events = lines.iter().filter(|l| l.starts_with("E ")).count();
        for l in lines {
            if l.starts_with(LOG_END) {
                out.push_str(&format!(
                    "{LOG_END} events={events} drops=0 reads=1 elapsed_ms=9000 status=PASS\n"
                ));
            } else {
                out.push_str(l);
                out.push('\n');
            }
        }
    }
    out.push_str(&format!("{STEPS_END} steps={}\n", blocks.len()));
    out
}

/// Edit the event lines of one step and keep the log well-formed.
fn edit_step(text: &str, step: &str, f: impl Fn(Vec<String>) -> Vec<String>) -> String {
    let mut b = blocks(text);
    let lines = &mut b.iter_mut().find(|(n, _)| n == step).unwrap().1;
    let head: Vec<String> = lines.iter().take_while(|l| !l.starts_with("E ")).cloned().collect();
    let events: Vec<String> = lines.iter().filter(|l| l.starts_with("E ")).cloned().collect();
    let tail: Vec<String> = lines.iter().filter(|l| l.starts_with(LOG_END)).cloned().collect();
    *lines = head.into_iter().chain(f(events)).chain(tail).collect();
    assemble(&b)
}

#[test]
fn every_orientation_is_recovered_with_and_without_ten_fingers() {
    for o in Orientation::ALL {
        for last in [5, 10] {
            let r = run(&synthetic_log(o, &cfg(), last));
            assert!(r.pass, "{o:?}/{last}: {:?}", r.failures);
            assert_eq!(r.orientation, Some(o));
            assert_eq!(r.steps.len(), if last == 10 { 10 } else { 9 });
            assert_eq!(r.multi.len(), if last == 10 { 3 } else { 2 });
            assert_eq!(r.multi.last().unwrap().max_contacts, last);
            let fitting = r.candidates.iter().filter(|c| c.corners_ok && c.drags_ok).count();
            assert_eq!(fitting, 1);
        }
    }
}

#[test]
fn report_has_expected_measurements_and_profile() {
    let o = Orientation { swap_xy: true, flip_x: false, flip_y: true };
    let r = run(&synthetic_log(o, &cfg(), 10));
    let (cx, cy) = r.centre_logical.unwrap();
    assert!((cx - 639.5).abs() < 5.0 && (cy - 399.5).abs() < 5.0, "{cx},{cy}");
    let m = r.margins.unwrap();
    assert!(m.iter().all(|&g| g > 0.03 && g < 0.07), "{m:?}");
    // Synthetic jitter is +/-2 raw units.
    assert!(r.jitter.samples > 100);
    assert!(r.jitter.p2p_x_raw <= 4 && r.jitter.p2p_y_raw <= 4);
    // Only gaps while a finger is down count, not the pauses between steps.
    assert_eq!(r.report_gap_median_us, 10_000);
    assert_eq!(r.report_gap_p99_us, 10_000);
    assert_eq!(r.sync_drops, 0);
    let text = render(&r, &cfg());
    println!("{text}");
    assert!(text.contains("step centre: fingers=1 touches=1"));
    assert!(text.contains("step five_fingers: fingers=5 touches=5"));
    assert!(text.contains("orientation swap_xy=true flip_x=false flip_y=true"));
    assert!(text.contains("status=PASS"));
    assert!(text.contains("\"orientation\":{\"swap_xy\":true,\"flip_x\":false,\"flip_y\":true}"));
}

#[test]
fn corners_far_from_the_edges_fail_reach() {
    let mut c = cfg();
    c.max_corner_margin = 0.03; // synthetic corners sit 4-5% in
    let r = analyze(&parse_log(&synthetic_log(Orientation::default(), &cfg(), 5)).unwrap(), &c)
        .unwrap();
    assert!(!r.pass);
    assert_eq!(r.orientation, Some(Orientation::default()));
    assert!(r.failures.iter().any(|f| f.contains("from an edge")));
}

#[test]
fn a_missing_step_is_reported_not_guessed() {
    let mut b = blocks(&synthetic_log(Orientation::default(), &cfg(), 5));
    b.retain(|(n, _)| n != "top_left");
    let r = run(&assemble(&b));
    assert!(!r.pass);
    assert_eq!(r.orientation, None);
    assert_eq!(r.failures, vec!["missing step(s): top_left".to_string()]);
    assert_eq!(r.steps.len(), 8);
}

#[test]
fn a_wrong_step_is_named() {
    let text = synthetic_log(Orientation::default(), &cfg(), 5);
    // Two touches in a corner step.
    let doubled = edit_step(&text, "top_right", |ev| ev.iter().chain(ev.iter()).cloned().collect());
    let r = run(&doubled);
    assert!(!r.pass);
    assert!(r.failures[0].starts_with("step top_right: expected one finger"), "{:?}", r.failures);
    // A drag step where the finger never moved.
    let b = blocks(&text);
    let corner = b.iter().find(|(n, _)| n == "top_left").unwrap().1.clone();
    let mut swapped = b.clone();
    swapped.iter_mut().find(|(n, _)| n == "drag_left_to_right").unwrap().1 = corner;
    let r = run(&assemble(&swapped));
    assert!(r.failures[0].starts_with("step drag_left_to_right: drag too short"), "{:?}", r.failures);
    // A step with no touch at all.
    let r = run(&edit_step(&text, "bottom_left", |_| Vec::new()));
    assert!(r.failures[0].contains("saw 0 touch(es)"), "{:?}", r.failures);
}

#[test]
fn an_empty_capture_fails_cleanly() {
    let mut b = blocks(&synthetic_log(Orientation::default(), &cfg(), 5));
    for (_, lines) in b.iter_mut() {
        lines.retain(|l| !l.starts_with("E "));
    }
    let r = run(&assemble(&b));
    assert!(!r.pass);
    assert_eq!(r.events, 0);
    assert_eq!(r.steps.len(), 9);
}

#[test]
fn a_sync_drop_mid_hold_is_counted_and_tolerated() {
    let text = synthetic_log(Orientation::default(), &cfg(), 5);
    let with_drop = edit_step(&text, "centre", |mut ev| {
        // Insert SYN_DROPPED + SYN_REPORT late in the centre hold (frame ~250
        // of 300), keeping its timestamp.
        let idx = ev.iter().enumerate().filter(|(_, l)| l.ends_with(" 0 0 0")).nth(250).unwrap().0;
        let stamp: Vec<&str> = ev[idx].split(' ').collect();
        let (s, u) = (stamp[1].to_string(), stamp[2].to_string());
        ev.insert(idx + 1, format!("E {s} {u} 0 3 0"));
        ev.insert(idx + 2, format!("E {s} {u} 0 0 0"));
        ev
    });
    let r = run(&with_drop);
    assert_eq!(r.sync_drops, 1);
    assert!(r.pass, "{:?}", r.failures);
}

#[test]
fn malformed_logs_are_rejected() {
    let good = synthetic_log(Orientation::default(), &cfg(), 5);
    assert!(parse_log(&good).is_ok());
    // Cut short: no steps footer.
    let cut: String = good.lines().take(50).map(|l| format!("{l}\n")).collect();
    assert!(parse_log(&cut).unwrap_err().contains("footer"));
    // Per-step footer count mismatch.
    let wrong = good.replacen("events=", "events=1", 1);
    assert!(parse_log(&wrong).is_err());
    // Device-side failure status.
    let failed = good.replacen("status=PASS", "status=EVENT_CAP_REACHED", 1);
    assert!(parse_log(&failed).unwrap_err().contains("did not pass"));
    // Stray line, out-of-range type, missing headers.
    assert!(parse_log(&good.replacen("E ", "X ", 1)).is_err());
    assert!(parse_log(&good.replacen(" 3 47 ", " 99 47 ", 1)).is_err());
    assert!(parse_log(&good.replacen(LOG_HEADER, "OTHER", 1)).is_err());
    assert!(parse_log(&good.replace(STEPS_HEADER, "OTHER")).is_err());
    // Steps footer count, unknown, repeated and out-of-order steps.
    let n = good.lines().filter(|l| l.starts_with("STEP ")).count();
    assert!(parse_log(&good.replace(&format!("steps={n}"), "steps=3")).is_err());
    assert!(parse_log(&good.replacen("STEP top_left", "STEP top_middle", 1))
        .unwrap_err()
        .contains("unknown step"));
    assert!(parse_log(&good.replacen("STEP top_right", "STEP top_left", 1))
        .unwrap_err()
        .contains("out of order"));
    let mut b = blocks(&good);
    b.swap(1, 2);
    assert!(parse_log(&assemble(&b)).unwrap_err().contains("out of order"));
    // Anything after the footer.
    assert!(parse_log(&format!("{good}STEP centre\n")).is_err());
}
