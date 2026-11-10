//! M12: host-only analysis of a recorded touch capture.
//!
//! Input is the step-labelled log the workstation writes while the owner
//! performs each touch step (one `mpfree-touch-capture` run per step). This
//! module parses it, decodes each step with a fresh M11 [`MtDecoder`], checks
//! every step did what it asked, and works out the panel orientation, reach,
//! jitter, report rate and multi-finger behaviour. Pure computation, no I/O.
//!
//! Steps, in order: centre hold, corners TL, TR, BR, BL, drag left to right,
//! drag top to bottom, 2-finger hold, 5-finger hold, optional 10-finger hold.

use crate::mt::{
    ContactId, InputEvent, MtDecoder, Phase, Step, EV_ABS, EV_SYN, SYN_DROPPED, SYN_REPORT,
};
use crate::pointer::{AxisMap, Orientation, RawRange};
use std::collections::BTreeMap;

/// One device run, as printed by `mpfree-touch-capture`.
pub const LOG_HEADER: &str = "MPFREE_TOUCH_CAPTURE_V2";
pub const LOG_ARMED: &str = "MPFREE_TOUCH_CAPTURE_ARMED";
pub const LOG_END: &str = "MPFREE_TOUCH_CAPTURE_END";
/// The whole step-labelled log, as written by host_touch_capture.py.
pub const STEPS_HEADER: &str = "MPFREE_TOUCH_CAPTURE_STEPS_V1";
pub const STEPS_END: &str = "MPFREE_TOUCH_CAPTURE_STEPS_END";

/// Step names in protocol order. Must match STEPS in
/// tools/mpc-one/host_touch_capture.py. All but the last are required.
pub const STEP_NAMES: [&str; 10] = [
    "centre",
    "top_left",
    "top_right",
    "bottom_right",
    "bottom_left",
    "drag_left_to_right",
    "drag_top_to_bottom",
    "two_fingers",
    "five_fingers",
    "ten_fingers",
];
const REQUIRED_STEPS: usize = 9;
/// Fingers each step asks for.
const STEP_FINGERS: [usize; 10] = [1, 1, 1, 1, 1, 1, 1, 2, 5, 10];

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

/// One step's events, from one device run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepCapture {
    pub name: String,
    pub events: Vec<InputEvent>,
    pub reported_drops: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureLog {
    pub steps: Vec<StepCapture>,
}

impl CaptureLog {
    pub fn events(&self) -> usize {
        self.steps.iter().map(|s| s.events.len()).sum()
    }
    pub fn step(&self, name: &str) -> Option<&StepCapture> {
        self.steps.iter().find(|s| s.name == name)
    }
}

/// Parse the step-labelled log. Strict: the steps header, then for each step
/// a `STEP <name>` line (known name, protocol order, no repeats) followed by
/// one complete device run, then the steps footer with the right count.
pub fn parse_log(text: &str) -> Result<CaptureLog, String> {
    let mut lines = text.lines().map(str::trim_end).filter(|l| !l.is_empty());
    if lines.next() != Some(STEPS_HEADER) {
        return Err(format!("no {STEPS_HEADER} header"));
    }
    let mut steps: Vec<StepCapture> = Vec::new();
    let mut last_index: Option<usize> = None;
    loop {
        let line = lines.next().ok_or(format!("no {STEPS_END} footer (capture cut short?)"))?;
        if let Some(rest) = line.strip_prefix(STEPS_END) {
            let count = rest
                .trim()
                .strip_prefix("steps=")
                .and_then(|v| v.parse::<usize>().ok())
                .ok_or(format!("malformed steps footer {line:?}"))?;
            if count != steps.len() {
                return Err(format!("steps footer says {count}, log has {}", steps.len()));
            }
            if let Some(extra) = lines.next() {
                return Err(format!("unexpected line after steps footer: {extra:?}"));
            }
            return Ok(CaptureLog { steps });
        }
        let name = line.strip_prefix("STEP ").ok_or(format!("expected a STEP line, got {line:?}"))?;
        let index = STEP_NAMES
            .iter()
            .position(|n| *n == name)
            .ok_or(format!("unknown step {name:?}"))?;
        if last_index.is_some_and(|prev| index <= prev) {
            return Err(format!("step {name:?} repeated or out of order"));
        }
        last_index = Some(index);
        let (events, reported_drops) =
            parse_run(&mut lines).map_err(|e| format!("step {name}: {e}"))?;
        steps.push(StepCapture { name: name.to_string(), events, reported_drops });
    }
}

/// Parse one device run: header, `key=value` lines, armed marker, only `E`
/// lines, and a PASS footer whose event count matches.
fn parse_run<'a>(lines: &mut impl Iterator<Item = &'a str>) -> Result<(Vec<InputEvent>, u64), String> {
    if lines.next() != Some(LOG_HEADER) {
        return Err(format!("no {LOG_HEADER} header"));
    }
    loop {
        match lines.next() {
            Some(LOG_ARMED) => break,
            Some(l) if l.contains('=') && !l.starts_with("E ") && !l.starts_with("STEP") => {}
            Some(l) => return Err(format!("unexpected line before {LOG_ARMED}: {l:?}")),
            None => return Err(format!("no {LOG_ARMED} marker")),
        }
    }
    let mut events = Vec::new();
    for line in lines.by_ref() {
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
            return Ok((events, reported_drops));
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

/// What one step's run decoded to.
#[derive(Debug, Clone)]
struct StepStats {
    /// Strokes in touch-down order.
    strokes: Vec<Stroke>,
    max_contacts: usize,
    frames: u64,
    sync_drops: u64,
    cancelled: bool,
    /// Gaps between consecutive reports while a finger is down.
    gaps: Vec<u64>,
}

impl StepStats {
    fn travel(&self) -> f64 {
        self.strokes.iter().map(Stroke::travel).fold(0.0, f64::max)
    }
    fn duration_us(&self) -> u64 {
        let first = self.strokes.iter().map(|s| s.samples[0].0).min().unwrap_or(0);
        let last = self.strokes.iter().map(|s| s.samples.last().unwrap().0).max().unwrap_or(0);
        last - first
    }
}

fn decode_step(events: &[InputEvent], slots: usize) -> Result<StepStats, String> {
    let mut dec = MtDecoder::new(slots).map_err(|e| format!("{e:?}"))?;
    let mut order: Vec<ContactId> = Vec::new();
    let mut strokes: BTreeMap<ContactId, Stroke> = BTreeMap::new();
    let mut st = StepStats {
        strokes: Vec::new(),
        max_contacts: 0,
        frames: 0,
        sync_drops: 0,
        cancelled: false,
        gaps: Vec::new(),
    };
    let mut last_report: Option<u64> = None;
    for ev in events {
        let mut step = dec.push(*ev);
        if step == Step::ResyncRequired {
            st.sync_drops += 1;
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
                    order.push(c.id);
                    strokes.insert(c.id, Stroke { samples: vec![(time, c.raw_x, c.raw_y)] });
                }
                Phase::Move => {
                    if let Some(s) = strokes.get_mut(&c.id) {
                        s.samples.push((time, c.raw_x, c.raw_y));
                    }
                }
                Phase::Up => {}
                Phase::Cancel => st.cancelled = true,
            }
        }
        let live = dec.contacts().len();
        st.max_contacts = st.max_contacts.max(live);
        if live > 0 {
            if let Some(prev) = last_report {
                st.gaps.push(time.saturating_sub(prev));
            }
            last_report = Some(time);
        } else {
            last_report = None;
        }
    }
    st.frames = dec.stats().frames;
    st.strokes = order.iter().map(|id| strokes[id].clone()).collect();
    Ok(st)
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
    /// One summary line per captured step.
    pub steps: Vec<String>,
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

/// Decode each step, check it matches the protocol, and measure.
pub fn analyze(log: &CaptureLog, cfg: &CalibrationConfig) -> Result<Report, String> {
    let span = f64::from(cfg.raw_x.max - cfg.raw_x.min).min(f64::from(cfg.raw_y.max - cfg.raw_y.min));
    let mut decoded: BTreeMap<&str, StepStats> = BTreeMap::new();
    let mut step_lines = Vec::new();
    let (mut frames, mut sync_drops) = (0, 0);
    let mut gaps: Vec<u64> = Vec::new();
    for step in &log.steps {
        let st = decode_step(&step.events, cfg.slots)?;
        step_lines.push(format!(
            "{}: fingers={} touches={} duration_ms={} travel={:.0}%{}",
            step.name,
            st.max_contacts,
            st.strokes.len(),
            st.duration_us() / 1000,
            100.0 * st.travel() / span,
            if st.cancelled { " cancelled" } else { "" }
        ));
        frames += st.frames;
        sync_drops += st.sync_drops;
        gaps.extend_from_slice(&st.gaps);
        let name = STEP_NAMES.iter().find(|n| **n == step.name).ok_or("unknown step")?;
        decoded.insert(name, st);
    }

    let mut failures = Vec::new();
    let missing: Vec<&str> =
        STEP_NAMES[..REQUIRED_STEPS].iter().copied().filter(|n| !decoded.contains_key(n)).collect();
    if !missing.is_empty() {
        failures.push(format!("missing step(s): {}", missing.join(", ")));
    }
    // Single-finger steps: exactly one touch; holds stay put, drags travel.
    for (i, name) in STEP_NAMES[..7].iter().enumerate() {
        let Some(st) = decoded.get(name) else { continue };
        let drag = i >= 5;
        let problem = if st.strokes.len() != 1 || st.max_contacts != 1 {
            Some(format!("expected one finger, one touch; saw {} touch(es)", st.strokes.len()))
        } else if !drag && st.travel() >= 0.15 * span {
            Some("finger moved too far for a hold".to_string())
        } else if drag && st.travel() < 0.30 * span {
            Some("drag too short (needs at least 30% of the screen)".to_string())
        } else {
            None
        };
        if let Some(p) = problem {
            failures.push(format!("step {name}: {p}"));
        }
    }
    for name in &STEP_NAMES[7..REQUIRED_STEPS] {
        if decoded.get(name).is_some_and(|st| st.max_contacts < 2) {
            failures.push(format!("step {name}: fewer than two fingers were down together"));
        }
    }
    let empty = |failures: Vec<String>, gaps: &mut Vec<u64>| Report {
        events: log.events(),
        frames,
        sync_drops,
        steps: step_lines.clone(),
        candidates: Vec::new(),
        orientation: None,
        margins: None,
        corners_logical: Vec::new(),
        centre_logical: None,
        jitter: Jitter { samples: 0, std_x_raw: 0.0, std_y_raw: 0.0, p2p_x_raw: 0, p2p_y_raw: 0 },
        report_gap_median_us: percentile(gaps, 50),
        report_gap_p99_us: percentile(gaps, 99),
        multi: Vec::new(),
        pass: false,
        failures,
    };
    if !failures.is_empty() {
        return Ok(empty(failures, &mut gaps));
    }

    let one = |name: &str| &decoded[name].strokes[0];
    let corners: Vec<(i32, i32)> = STEP_NAMES[1..5].iter().map(|n| one(n).median()).collect();
    let drag_ends = |name: &str| {
        let s = one(name);
        let (_, x0, y0) = s.samples[0];
        let (_, x1, y1) = *s.samples.last().unwrap();
        ((x0, y0), (x1, y1))
    };
    let (h0, h1) = drag_ends("drag_left_to_right");
    let (v0, v1) = drag_ends("drag_top_to_bottom");
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
        let (cx, cy) = one("centre").median();
        centre_logical = Some(map.map(cx, cy));
    }

    let jitter = hold_jitter(one("centre"));
    let multi = STEP_NAMES[7..]
        .iter()
        .zip(&STEP_FINGERS[7..])
        .filter_map(|(name, &e)| {
            decoded.get(name).map(|st| MultiHold {
                expected_fingers: e,
                max_contacts: st.max_contacts,
                contacts_seen: st.strokes.len(),
            })
        })
        .collect();

    Ok(Report {
        events: log.events(),
        frames,
        sync_drops,
        steps: step_lines,
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
    for st in &r.steps {
        line(format!("step {st}"));
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

/// A complete, well-formed step-labelled log as the workstation would write
/// it, for a panel mounted with orientation `o`. `fingers_last` is 10 to
/// include the optional step, or 5 to stop after the five-finger hold.
pub fn synthetic_log(o: Orientation, cfg: &CalibrationConfig, fingers_last: usize) -> String {
    let mut t: u64 = 1_700_000_000_000_000;
    let mut tid = 100;
    let frame_us = 10_000;
    let mut noise = 0u32;
    let mut jiggle = || {
        noise = noise.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        ((noise >> 16) % 5) as i32 - 2
    };
    // One step: `paths` gives each finger's (u,v) at progress p in [0,1].
    let mut step = |paths: &[(f64, f64, f64, f64)], ms: u64| -> Vec<InputEvent> {
        let mut ev = Vec::new();
        t += 3_000_000; // countdown before the finger goes down
        let frames = ms * 1000 / frame_us;
        for (slot, _) in paths.iter().enumerate() {
            ev.push(InputEvent::new(t, EV_ABS, 47, slot as i32));
            ev.push(InputEvent::new(t, EV_ABS, 57, tid));
            tid += 1;
        }
        for f in 0..=frames {
            let p = f as f64 / frames as f64;
            for (slot, &(u0, v0, u1, v1)) in paths.iter().enumerate() {
                let (x, y) = to_raw(o, u0 + (u1 - u0) * p, v0 + (v1 - v0) * p, cfg);
                ev.push(InputEvent::new(t, EV_ABS, 47, slot as i32));
                ev.push(InputEvent::new(t, EV_ABS, 53, x + jiggle()));
                ev.push(InputEvent::new(t, EV_ABS, 54, y + jiggle()));
            }
            ev.push(InputEvent::new(t, EV_SYN, SYN_REPORT, 0));
            t += frame_us;
        }
        for (slot, _) in paths.iter().enumerate() {
            ev.push(InputEvent::new(t, EV_ABS, 47, slot as i32));
            ev.push(InputEvent::new(t, EV_ABS, 57, -1));
        }
        ev.push(InputEvent::new(t, EV_SYN, SYN_REPORT, 0));
        t += 10_000_000; // the owner reads the next step
        ev
    };
    let hold = |u: f64, v: f64| (u, v, u, v);
    let spread = |n: usize| -> Vec<(f64, f64, f64, f64)> {
        (0..n).map(|i| hold(0.1 + 0.8 * i as f64 / n as f64, 0.5)).collect()
    };
    let mut steps: Vec<(&str, Vec<InputEvent>)> = vec![("centre", step(&[hold(0.5, 0.5)], 3000))];
    for (name, (u, v)) in STEP_NAMES[1..5]
        .iter()
        .zip([(0.04, 0.05), (0.96, 0.05), (0.96, 0.95), (0.04, 0.95)])
    {
        steps.push((name, step(&[hold(u, v)], 1000)));
    }
    steps.push(("drag_left_to_right", step(&[(0.1, 0.5, 0.9, 0.5)], 1000)));
    steps.push(("drag_top_to_bottom", step(&[(0.5, 0.1, 0.5, 0.9)], 1000)));
    steps.push(("two_fingers", step(&spread(2), 2000)));
    steps.push(("five_fingers", step(&spread(5), 2000)));
    if fingers_last == 10 {
        steps.push(("ten_fingers", step(&spread(10), 2000)));
    }
    let mut out = format!("{STEPS_HEADER}\n");
    for (name, ev) in &steps {
        out.push_str(&format!("STEP {name}\n{LOG_HEADER}\ndevice=event0\n{LOG_ARMED}\n"));
        for e in ev {
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
        out.push_str(&format!(
            "{LOG_END} events={} drops={drops} reads=1 elapsed_ms=9000 status=PASS\n",
            ev.len()
        ));
    }
    out.push_str(&format!("{STEPS_END} steps={}\n", steps.len()));
    out
}
