//! M12 host-only tests: synthetic capture logs through the analyzer. No
//! device is opened; every log here is generated in-process.

use mpfree_touch::calibrate::*;
use mpfree_touch::pointer::Orientation;

fn cfg() -> CalibrationConfig {
    CalibrationConfig::default()
}

fn run(text: &str) -> Report {
    analyze(&parse_log(text).expect("log parses"), &cfg()).expect("analysis runs")
}

#[test]
fn every_orientation_is_recovered_with_and_without_ten_fingers() {
    for o in Orientation::ALL {
        for last in [5, 10] {
            let r = run(&synthetic_log(o, &cfg(), last));
            assert!(r.pass, "{o:?}/{last}: {:?}", r.failures);
            assert_eq!(r.orientation, Some(o));
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
    assert_eq!(r.report_gap_median_us, 10_000);
    assert_eq!(r.sync_drops, 0);
    let text = render(&r, &cfg());
    println!("{text}");
    assert!(text.contains("orientation swap_xy=true flip_x=false flip_y=true"));
    assert!(text.contains("status=PASS"));
    assert!(text.contains("\"orientation\":{\"swap_xy\":true,\"flip_x\":false,\"flip_y\":true}"));
}

/// Replace the event lines of a log with a filtered/edited version and fix
/// the footer count, so the log itself stays well-formed.
fn edit_events(text: &str, f: impl Fn(Vec<String>) -> Vec<String>) -> String {
    let lines: Vec<String> = text.lines().map(String::from).collect();
    let head: Vec<String> = lines.iter().take_while(|l| !l.starts_with("E ")).cloned().collect();
    let events: Vec<String> = lines.iter().filter(|l| l.starts_with("E ")).cloned().collect();
    let events = f(events);
    let mut out = head.join("\n");
    out.push('\n');
    for e in &events {
        out.push_str(e);
        out.push('\n');
    }
    out.push_str(&format!(
        "{LOG_END} events={} drops=0 reads=1 elapsed_ms=60000 status=PASS\n",
        events.len()
    ));
    out
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
    // Drop the second touch group (the TL corner): keep events before the
    // first lift-off+gap and after the second one.
    let text = synthetic_log(Orientation::default(), &cfg(), 5);
    let cut = edit_events(&text, |ev| {
        // Group boundaries are where the timestamp jumps by >= 1 s.
        let t = |l: &String| {
            let f: Vec<u64> = l.split(' ').skip(1).take(2).map(|x| x.parse().unwrap()).collect();
            f[0] * 1_000_000 + f[1]
        };
        let mut group = 0;
        let mut prev = t(&ev[0]);
        ev.into_iter()
            .filter(|l| {
                let now = t(l);
                if now - prev >= 900_000 {
                    group += 1;
                }
                prev = now;
                group != 1
            })
            .collect()
    });
    let r = run(&cut);
    assert!(!r.pass);
    assert_eq!(r.orientation, None);
    assert!(r.failures[0].contains("don't match the protocol"), "{:?}", r.failures);
    assert_eq!(r.groups.len(), 8);
}

#[test]
fn an_empty_capture_fails_cleanly() {
    let empty = format!("{LOG_HEADER}\n{LOG_ARMED}\n{LOG_END} events=0 drops=0 reads=0 elapsed_ms=60000 status=PASS\n");
    let r = run(&empty);
    assert!(!r.pass);
    assert!(r.groups.is_empty());
}

#[test]
fn a_sync_drop_mid_hold_is_counted_and_tolerated() {
    let text = synthetic_log(Orientation::default(), &cfg(), 5);
    let with_drop = edit_events(&text, |mut ev| {
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
    // Cut short: no footer.
    let cut: String = good.lines().take(50).map(|l| format!("{l}\n")).collect();
    assert!(parse_log(&cut).unwrap_err().contains("footer"));
    // Footer count mismatch.
    let wrong = good.replacen("events=", "events=1", 1);
    assert!(parse_log(&wrong).is_err());
    // Device-side failure status.
    let failed = good.replace("status=PASS", "status=EVENT_CAP_REACHED");
    assert!(parse_log(&failed).unwrap_err().contains("did not pass"));
    // Stray line, out-of-range type, missing header.
    assert!(parse_log(&good.replacen("E ", "X ", 1)).is_err());
    assert!(parse_log(&good.replacen(" 3 47 ", " 99 47 ", 1)).is_err());
    assert!(parse_log(&good.replace(LOG_HEADER, "OTHER")).is_err());
}
