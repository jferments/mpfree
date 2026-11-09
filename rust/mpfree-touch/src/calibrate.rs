//! M12: host-only analysis of a recorded touch capture.
//!
//! Input is the text log printed by `mpfree-touch-capture` on the MPC. This
//! module parses it, decodes it with the M11 [`MtDecoder`], matches it to the
//! M12 touch protocol and works out the panel orientation, reach, jitter,
//! report rate and multi-finger behaviour. Pure computation, no I/O.
//!
//! Protocol (one capture, fingers lifted for >= 1 s between steps):
//! 1. centre hold, 2-5. corners TL, TR, BR, BL, 6. drag left to right,
//! 7. drag top to bottom, then 2-finger, 5-finger and optional 10-finger holds.

use crate::mt::{
    ContactId, InputEvent, MtDecoder, Phase, Step, EV_ABS, EV_SYN, SYN_DROPPED, SYN_REPORT,
};
use crate::pointer::{AxisMap, Orientation, RawRange};
use std::collections::BTreeMap;

pub const LOG_HEADER: &str = "MPFREE_TOUCH_CAPTURE_V1";
pub const LOG_ARMED: &str = "MPFREE_TOUCH_CAPTURE_ARMED";
pub const LOG_END: &str = "MPFREE_TOUCH_CAPTURE_END";

/// Logical GUI size the orientation is expressed against (landscape).
pub const LOGICAL_W: u32 = 1280;
pub const LOGICAL_H: u32 = 800;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalibrationConfig {
    /// From M10 EVIOCGABS metadata.
    pub raw_x: RawRange,
    pub raw_y: RawRange,
    pub slots: usize,
    /// Largest allowed gap between a corner touch and the edges, as a
    /// fraction of the axis.
    pub max_corner_margin: f64,
}

impl Default for CalibrationConfig {
    /// The values M10 measured on the MPC One (X/Y 0..=2048, 10 slots).
    fn default() -> Self {
        Self {
            raw_x: RawRange { min: 0, max: 2048 },
            raw_y: RawRange { min: 0, max: 2048 },
            slots: 10,
            max_corner_margin: 0.15,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureLog {
    pub events: Vec<InputEvent>,
    pub reported_drops: u64,
}

/// Parse the capture log. Strict: header, armed marker, only `E` lines in
/// between, and a PASS footer whose event count matches.
pub fn parse_log(text: &str) -> Result<CaptureLog, String> {
    let mut lines = text.lines().map(str::trim_end).filter(|l| !l.is_empty());
    let mut seen_header = false;
    for line in lines.by_ref() {
        if line == LOG_HEADER {
            seen_header = true;
        }
        if line == LOG_ARMED {
            break;
        }
    }
    if !seen_header {
        return Err(format!("no {LOG_HEADER} header"));
    }
    let mut events = Vec::new();
    for line in lines {
        if let Some(rest) = line.strip_prefix("E ") {
            let f: Vec<&str> = rest.split(' ').collect();
            if f.len() != 5 {
                return Err(format!("malformed event line {line:?}"));
            }
            let num = |s: &str| s.parse::<i64>().map_err(|_| format!("bad number in {line:?}"));
            let (sec, usec, kind, code, value) =
                (num(f[0])?, num(f[1])?, num(f[2])?, num(f[3])?, num(f[4])?);
            if !(0..=u32::MAX as i64).contains(&sec)
                || !(0..1_000_000).contains(&usec)
                || !(0..=0x1f).contains(&kind)
                || !(0..=0xffff).contains(&code)
                || !(i32::MIN as i64..=i32::MAX as i64).contains(&value)
            {
                return Err(format!("event out of range {line:?}"));
            }
            events.push(InputEvent::new(
                sec as u64 * 1_000_000 + usec as u64,
                kind as u16,
                code as u16,
                value as i32,
            ));
        } else if let Some(rest) = line.strip_prefix(LOG_END) {
            let fields: BTreeMap<&str, &str> =
                rest.split_whitespace().filter_map(|kv| kv.split_once('=')).collect();
            if fields.get("status") != Some(&"PASS") {
                return Err(format!("capture did not pass: {rest:?}"));
            }
            let count: usize = fields
                .get("events")
                .and_then(|v| v.parse().ok())
                .ok_or("footer has no event count")?;
            if count != events.len() {
                return Err(format!("footer says {count} events, log has {}", events.len()));
            }
            let reported_drops = fields
                .get("drops")
                .and_then(|v| v.parse().ok())
                .ok_or("footer has no drop count")?;
            return Ok(CaptureLog { events, reported_drops });
        } else {
            return Err(format!("unexpected line inside capture: {line:?}"));
        }
    }
    Err(format!("no {LOG_END} footer (capture cut short?)"))
}

#[derive(Debug, Clone)]
struct Stroke {
    samples: Vec<(u64, i32, i32)>,
}

impl Stroke {
    fn travel(&self) -> f64 {
        let (_, x0, y0) = self.samples[0];
        self.samples
            .iter()
            .map(|&(_, x, y)| f64::from(x - x0).hypot(f64::from(y - y0)))
            .fold(0.0, f64::max)
    }
    fn median(&self) -> (i32, i32) {
        let mut xs: Vec<i32> = self.samples.iter().map(|s| s.1).collect();
        let mut ys: Vec<i32> = self.samples.iter().map(|s| s.2).collect();
        xs.sort_unstable();
        ys.sort_unstable();
        (xs[xs.len() / 2], ys[ys.len() / 2])
    }
}

#[derive(Debug, Clone)]
struct Group {
    strokes: Vec<ContactId>,
    max_contacts: usize,
    start_us: u64,
    end_us: u64,
    cancelled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrientationResult {
    pub orientation: Orientation,
    pub corners_ok: bool,
    pub drags_ok: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Jitter {
    pub samples: usize,
    pub std_x_raw: f64,
    pub std_y_raw: f64,
    pub p2p_x_raw: i32,
    pub p2p_y_raw: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MultiHold {
    pub expected_fingers: usize,
    pub max_contacts: usize,
    pub contacts_seen: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub events: usize,
    pub frames: u64,
    pub sync_drops: u64,
    pub groups: Vec<String>,
    pub candidates: Vec<OrientationResult>,
    /// Exactly one orientation fit all corners and both drags.
    pub orientation: Option<Orientation>,
    /// Left, right, top, bottom gap between corner touches and the edges,
    /// as a fraction of the axis, in logical orientation.
    pub margins: Option<[f64; 4]>,
    pub corners_logical: Vec<(f32, f32)>,
    pub centre_logical: Option<(f32, f32)>,
    pub jitter: Jitter,
    pub report_gap_median_us: u64,
    pub report_gap_p99_us: u64,
    pub multi: Vec<MultiHold>,
    pub pass: bool,
    pub failures: Vec<String>,
}

/// Decode, segment, match the protocol, and measure.
pub fn analyze(log: &CaptureLog, cfg: &CalibrationConfig) -> Result<Report, String> {
    let mut dec = MtDecoder::new(cfg.slots).map_err(|e| format!("{e:?}"))?;
    let mut strokes: BTreeMap<ContactId, Stroke> = BTreeMap::new();
    let mut groups: Vec<Group> = Vec::new();
    let mut open: Option<Group> = None;
    let mut sync_drops = 0;
    let mut last_report: Option<u64> = None;
    let mut gaps: Vec<u64> = Vec::new();

    for ev in &log.events {
        let mut step = dec.push(*ev);
        if step == Step::ResyncRequired {
            sync_drops += 1;
            step = dec.resync_without_snapshot(ev.time_us);
        }
        if step != Step::Frame {
            continue;
        }
        let time = ev.time_us;
        for ce in dec.events() {
            let c = ce.contact;
            match ce.phase {
                Phase::Down => {
                    strokes.insert(c.id, Stroke { samples: vec![(time, c.raw_x, c.raw_y)] });
                    let g = open.get_or_insert_with(|| Group {
                        strokes: Vec::new(),
                        max_contacts: 0,
                        start_us: time,
                        end_us: time,
                        cancelled: false,
                    });
                    g.strokes.push(c.id);
                }
                Phase::Move => {
                    if let Some(s) = strokes.get_mut(&c.id) {
                        s.samples.push((time, c.raw_x, c.raw_y));
                    }
                }
                Phase::Up => {}
                Phase::Cancel => {
                    if let Some(g) = open.as_mut() {
                        g.cancelled = true;
                    }
                }
            }
        }
        let live = dec.contacts().len();
        if let Some(g) = open.as_mut() {
            g.max_contacts = g.max_contacts.max(live);
            g.end_us = time;
            if let Some(prev) = last_report {
                gaps.push(time.saturating_sub(prev));
            }
            last_report = Some(time);
        }
        if live == 0 {
            if let Some(g) = open.take() {
                groups.push(g);
            }
            last_report = None;
        }
    }
    if let Some(g) = open.take() {
        groups.push(g);
    }

    let span = f64::from(cfg.raw_x.max - cfg.raw_x.min).min(f64::from(cfg.raw_y.max - cfg.raw_y.min));
    let describe = |g: &Group| {
        let travel = g.strokes.iter().map(|id| strokes[id].travel()).fold(0.0, f64::max);
        format!(
            "fingers={} contacts={} duration_ms={} travel={:.0}%{}",
            g.max_contacts,
            g.strokes.len(),
            (g.end_us - g.start_us) / 1000,
            100.0 * travel / span,
            if g.cancelled { " cancelled" } else { "" }
        )
    };
    let group_lines: Vec<String> = groups.iter().map(describe).collect();
    let mut failures = Vec::new();

    // Match the protocol: 7 single-finger groups, then 2 or 3 multi-finger.
    let singles: Vec<&Group> = groups.iter().take_while(|g| g.max_contacts == 1).collect();
    let multis: Vec<&Group> = groups[singles.len()..].iter().collect();
    let protocol_ok = singles.len() == 7
        && (2..=3).contains(&multis.len())
        && multis.iter().all(|g| g.max_contacts >= 2);
    let one = |g: &Group| &strokes[&g.strokes[0]];
    let still = |g: &Group| g.strokes.len() == 1 && one(g).travel() < 0.15 * span;
    let dragged = |g: &Group| g.strokes.len() == 1 && one(g).travel() >= 0.30 * span;
    if !protocol_ok
        || !singles[..5.min(singles.len())].iter().all(|g| still(g))
        || !singles[5.min(singles.len())..].iter().all(|g| dragged(g))
    {
        failures.push(format!(
            "touches don't match the protocol (expected 7 single-finger steps: hold, 4 corners, \
             2 drags; then 2-3 multi-finger holds). Seen {} groups.",
            groups.len()
        ));
        return Ok(Report {
            events: log.events.len(),
            frames: dec.stats().frames,
            sync_drops,
            groups: group_lines,
            candidates: Vec::new(),
            orientation: None,
            margins: None,
            corners_logical: Vec::new(),
            centre_logical: None,
            jitter: Jitter { samples: 0, std_x_raw: 0.0, std_y_raw: 0.0, p2p_x_raw: 0, p2p_y_raw: 0 },
            report_gap_median_us: percentile(&mut gaps, 50),
            report_gap_p99_us: percentile(&mut gaps, 99),
            multi: Vec::new(),
            pass: false,
            failures,
        });
    }

    let corners: Vec<(i32, i32)> = singles[1..5].iter().map(|g| one(g).median()).collect();
    let drag_ends = |g: &Group| {
        let s = one(g);
        let (_, x0, y0) = s.samples[0];
        let (_, x1, y1) = *s.samples.last().unwrap();
        ((x0, y0), (x1, y1))
    };
    let (h0, h1) = drag_ends(singles[5]);
    let (v0, v1) = drag_ends(singles[6]);
    let (w, h) = (f64::from(LOGICAL_W), f64::from(LOGICAL_H));
    let mut candidates = Vec::new();
    for o in Orientation::ALL {
        let map = AxisMap::new(cfg.raw_x, cfg.raw_y, o, LOGICAL_W, LOGICAL_H)
            .map_err(|e| format!("{e:?}"))?;
        let at = |p: (i32, i32)| {
            let (x, y) = map.map(p.0, p.1);
            (f64::from(x), f64::from(y))
        };
        // TL, TR, BR, BL: (right half?, bottom half?)
        let want = [(false, false), (true, false), (true, true), (false, true)];
        let corners_ok = corners.iter().zip(want).all(|(&c, (right, bottom))| {
            let (x, y) = at(c);
            (x > w / 2.0) == right && (y > h / 2.0) == bottom
        });
        let (a, b) = (at(h0), at(h1));
        let (hx, hy) = (b.0 - a.0, b.1 - a.1);
        let (a, b) = (at(v0), at(v1));
        let (vx, vy) = (b.0 - a.0, b.1 - a.1);
        let drags_ok = hx > 0.3 * w && hy.abs() < 0.5 * hx && vy > 0.3 * h && vx.abs() < 0.5 * vy;
        candidates.push(OrientationResult { orientation: o, corners_ok, drags_ok });
    }
    let fitting: Vec<Orientation> = candidates
        .iter()
        .filter(|c| c.corners_ok && c.drags_ok)
        .map(|c| c.orientation)
        .collect();
    let orientation = (fitting.len() == 1).then(|| fitting[0]);
    if orientation.is_none() {
        failures.push(format!("{} orientations fit (need exactly 1)", fitting.len()));
    }

    let mut margins = None;
    let mut corners_logical = Vec::new();
    let mut centre_logical = None;
    if let Some(o) = orientation {
        let map = AxisMap::new(cfg.raw_x, cfg.raw_y, o, LOGICAL_W, LOGICAL_H)
            .map_err(|e| format!("{e:?}"))?;
        corners_logical = corners.iter().map(|&(x, y)| map.map(x, y)).collect();
        let uv: Vec<(f64, f64)> = corners_logical
            .iter()
            .map(|&(x, y)| (f64::from(x) / (w - 1.0), f64::from(y) / (h - 1.0)))
            .collect();
        let m = [
            uv[0].0.max(uv[3].0),             // left: TL, BL
            (1.0 - uv[1].0).max(1.0 - uv[2].0), // right: TR, BR
            uv[0].1.max(uv[1].1),             // top: TL, TR
            (1.0 - uv[2].1).max(1.0 - uv[3].1), // bottom: BR, BL
        ];
        if m.iter().any(|&g| g > cfg.max_corner_margin) {
            failures.push(format!(
                "a corner touch is more than {:.0}% from an edge",
                cfg.max_corner_margin * 100.0
            ));
        }
        margins = Some(m);
        let (cx, cy) = one(singles[0]).median();
        centre_logical = Some(map.map(cx, cy));
    }

    let jitter = hold_jitter(one(singles[0]));
    let expected = [2, 5, 10];
    let multi = multis
        .iter()
        .zip(expected)
        .map(|(g, e)| MultiHold {
            expected_fingers: e,
            max_contacts: g.max_contacts,
            contacts_seen: g.strokes.len(),
        })
        .collect();

    Ok(Report {
        events: log.events.len(),
        frames: dec.stats().frames,
        sync_drops,
        groups: group_lines,
        candidates,
        orientation,
        margins,
        corners_logical,
        centre_logical,
        jitter,
        report_gap_median_us: percentile(&mut gaps, 50),
        report_gap_p99_us: percentile(&mut gaps, 99),
        multi,
        pass: failures.is_empty(),
        failures,
    })
}

fn percentile(v: &mut [u64], p: usize) -> u64 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[((v.len() - 1) * p) / 100]
}

/// Jitter over the steady middle of the hold: skip 0.5 s after touch-down
/// and 0.5 s before lift-off (or use the middle half of a short hold).
fn hold_jitter(s: &Stroke) -> Jitter {
    let (t0, t1) = (s.samples[0].0, s.samples.last().unwrap().0);
    let (lo, hi) = if t1 - t0 >= 1_500_000 {
        (t0 + 500_000, t1 - 500_000)
    } else {
        (t0 + (t1 - t0) / 4, t1 - (t1 - t0) / 4)
    };
    let mid: Vec<(i32, i32)> =
        s.samples.iter().filter(|p| p.0 >= lo && p.0 <= hi).map(|p| (p.1, p.2)).collect();
    if mid.is_empty() {
        return Jitter { samples: 0, std_x_raw: 0.0, std_y_raw: 0.0, p2p_x_raw: 0, p2p_y_raw: 0 };
    }
    let stats = |vals: Vec<i32>| {
        let n = vals.len() as f64;
        let mean = vals.iter().map(|&v| f64::from(v)).sum::<f64>() / n;
        let var = vals.iter().map(|&v| (f64::from(v) - mean).powi(2)).sum::<f64>() / n;
        let p2p = vals.iter().max().unwrap() - vals.iter().min().unwrap();
        (var.sqrt(), p2p)
    };
    let (sx, px) = stats(mid.iter().map(|p| p.0).collect());
    let (sy, py) = stats(mid.iter().map(|p| p.1).collect());
    Jitter { samples: mid.len(), std_x_raw: sx, std_y_raw: sy, p2p_x_raw: px, p2p_y_raw: py }
}

fn orientation_name(o: Orientation) -> String {
    format!("swap_xy={} flip_x={} flip_y={}", o.swap_xy, o.flip_x, o.flip_y)
}

/// Plain-text report (key=value lines) plus a proposed profile in JSON.
pub fn render(r: &Report, cfg: &CalibrationConfig) -> String {
    let mut s = String::new();
    let mut line = |l: String| {
        s.push_str(&l);
        s.push('\n');
    };
    line("MPFREE_TOUCH_CALIBRATION_REPORT_V1".into());
    line(format!("events={} frames={} sync_drops={}", r.events, r.frames, r.sync_drops));
    for (i, g) in r.groups.iter().enumerate() {
        line(format!("group{}: {g}", i + 1));
    }
    for c in &r.candidates {
        line(format!(
            "candidate {} corners_ok={} drags_ok={}",
            orientation_name(c.orientation),
            c.corners_ok,
            c.drags_ok
        ));
    }
    match r.orientation {
        Some(o) => line(format!("orientation {}", orientation_name(o))),
        None => line("orientation none".into()),
    }
    for (name, (x, y)) in ["TL", "TR", "BR", "BL"].iter().zip(&r.corners_logical) {
        line(format!("corner_{name}_logical={x:.0},{y:.0}"));
    }
    if let Some((x, y)) = r.centre_logical {
        line(format!("centre_logical={x:.0},{y:.0} (expected near 640,400)"));
    }
    if let Some(m) = r.margins {
        line(format!(
            "corner_margin_pct left={:.1} right={:.1} top={:.1} bottom={:.1}",
            m[0] * 100.0,
            m[1] * 100.0,
            m[2] * 100.0,
            m[3] * 100.0
        ));
    }
    let j = &r.jitter;
    line(format!(
        "hold_jitter samples={} std_raw={:.2},{:.2} p2p_raw={},{}",
        j.samples, j.std_x_raw, j.std_y_raw, j.p2p_x_raw, j.p2p_y_raw
    ));
    line(format!(
        "report_gap_us median={} p99={}",
        r.report_gap_median_us, r.report_gap_p99_us
    ));
    for m in &r.multi {
        line(format!(
            "hold_{}_fingers max_simultaneous={} contacts_seen={}",
            m.expected_fingers, m.max_contacts, m.contacts_seen
        ));
    }
    for f in &r.failures {
        line(format!("FAIL: {f}"));
    }
    line(format!("status={}", if r.pass { "PASS" } else { "FAIL" }));
    if let (true, Some(o), Some(m)) = (r.pass, r.orientation, r.margins) {
        line("PROPOSED_PROFILE_JSON_BEGIN".into());
        line(format!(
            "{{\"device\":\"MPC One ILI2117\",\"raw_x\":[{},{}],\"raw_y\":[{},{}],\
             \"logical\":[{},{}],\"orientation\":{{\"swap_xy\":{},\"flip_x\":{},\"flip_y\":{}}},\
             \"corner_margin\":{{\"left\":{:.3},\"right\":{:.3},\"top\":{:.3},\"bottom\":{:.3}}},\
             \"hold_jitter_std_raw\":[{:.2},{:.2}],\"slots\":{}}}",
            cfg.raw_x.min, cfg.raw_x.max, cfg.raw_y.min, cfg.raw_y.max,
            LOGICAL_W, LOGICAL_H, o.swap_xy, o.flip_x, o.flip_y,
            m[0], m[1], m[2], m[3], j.std_x_raw, j.std_y_raw, cfg.slots
        ));
        line("PROPOSED_PROFILE_JSON_END".into());
    }
    s
}

// ---- synthetic protocol generator (tests and the analyzer self-test) -------

/// Map a logical-normalised point back to raw units for orientation `o`
/// (the inverse of [`AxisMap`]: undo the flips, then undo the swap).
fn to_raw(o: Orientation, u: f64, v: f64, cfg: &CalibrationConfig) -> (i32, i32) {
    let (mut a, mut b) = (u, v);
    if o.flip_x {
        a = 1.0 - a;
    }
    if o.flip_y {
        b = 1.0 - b;
    }
    if o.swap_xy {
        std::mem::swap(&mut a, &mut b);
    }
    let scale = |t: f64, r: RawRange| r.min + (t * f64::from(r.max - r.min)).round() as i32;
    (scale(a, cfg.raw_x), scale(b, cfg.raw_y))
}

/// A complete, well-formed protocol run as the capture tool would log it,
/// for a panel mounted with orientation `o`. `fingers_last` is 10 to include
/// the optional step, or 5 to stop after the five-finger hold.
pub fn synthetic_log(o: Orientation, cfg: &CalibrationConfig, fingers_last: usize) -> String {
    let mut ev: Vec<InputEvent> = Vec::new();
    let mut t: u64 = 1_700_000_000_000_000;
    let mut tid = 100;
    let frame_us = 10_000;
    let mut noise = 0u32;
    let mut jiggle = || {
        noise = noise.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        ((noise >> 16) % 5) as i32 - 2
    };
    // One step: `paths` gives each finger's (u,v) at progress p in [0,1].
    let mut step = |ev: &mut Vec<InputEvent>, t: &mut u64, paths: &[(f64, f64, f64, f64)], ms: u64| {
        let frames = ms * 1000 / frame_us;
        for (slot, _) in paths.iter().enumerate() {
            ev.push(InputEvent::new(*t, EV_ABS, 47, slot as i32));
            ev.push(InputEvent::new(*t, EV_ABS, 57, tid));
            tid += 1;
        }
        for f in 0..=frames {
            let p = f as f64 / frames as f64;
            for (slot, &(u0, v0, u1, v1)) in paths.iter().enumerate() {
                let (x, y) = to_raw(o, u0 + (u1 - u0) * p, v0 + (v1 - v0) * p, cfg);
                ev.push(InputEvent::new(*t, EV_ABS, 47, slot as i32));
                ev.push(InputEvent::new(*t, EV_ABS, 53, x + jiggle()));
                ev.push(InputEvent::new(*t, EV_ABS, 54, y + jiggle()));
            }
            ev.push(InputEvent::new(*t, EV_SYN, SYN_REPORT, 0));
            *t += frame_us;
        }
        for (slot, _) in paths.iter().enumerate() {
            ev.push(InputEvent::new(*t, EV_ABS, 47, slot as i32));
            ev.push(InputEvent::new(*t, EV_ABS, 57, -1));
        }
        ev.push(InputEvent::new(*t, EV_SYN, SYN_REPORT, 0));
        *t += 1_000_000; // fingers off for a second
    };
    let hold = |u: f64, v: f64| (u, v, u, v);
    step(&mut ev, &mut t, &[hold(0.5, 0.5)], 3000);
    for (u, v) in [(0.04, 0.05), (0.96, 0.05), (0.96, 0.95), (0.04, 0.95)] {
        step(&mut ev, &mut t, &[hold(u, v)], 1000);
    }
    step(&mut ev, &mut t, &[(0.1, 0.5, 0.9, 0.5)], 1000);
    step(&mut ev, &mut t, &[(0.5, 0.1, 0.5, 0.9)], 1000);
    let spread = |n: usize| -> Vec<(f64, f64, f64, f64)> {
        (0..n).map(|i| hold(0.1 + 0.8 * i as f64 / n as f64, 0.5)).collect()
    };
    step(&mut ev, &mut t, &spread(2), 2000);
    step(&mut ev, &mut t, &spread(5), 2000);
    if fingers_last == 10 {
        step(&mut ev, &mut t, &spread(10), 2000);
    }
    let mut out = format!("{LOG_HEADER}\ndevice=event0\n{LOG_ARMED}\n");
    for e in &ev {
        out.push_str(&format!(
            "E {} {} {} {} {}\n",
            e.time_us / 1_000_000,
            e.time_us % 1_000_000,
            e.kind,
            e.code,
            e.value
        ));
    }
    let drops = ev.iter().filter(|e| e.kind == EV_SYN && e.code == SYN_DROPPED).count();
    out.push_str(&format!("{LOG_END} events={} drops={drops} reads=1 elapsed_ms=60000 status=PASS\n", ev.len()));
    out
}
