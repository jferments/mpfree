//! Random walks: controls pressed in orders nobody scripted, with the
//! screen and the engines checked against each other after every step.
//!
//! A walk starts a [`Session`], then takes steps chosen by a seeded random
//! number: a control on the page (a tap, a drag or a long press as the
//! control says), a hardware button (alone or with SHIFT), the wheel, a pad,
//! or a moment of time passing. After each step the session settles and the
//! checks run. A failed check (or a panic) stops the walk and names the
//! seed and every step that led there, so the same walk can be run again
//! (`MPFREE_WALK_SEED=<seed>`) and the steps tried by hand on the MPC.

use super::Session;
use mpfree_ui::Gesture;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// A small, fixed random number generator (xorshift64*), so a seed always
/// walks the same way.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// A number from 0 to `n` - 1.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// The hardware buttons a walk presses (the mapper's names). STOP is
/// among them, so the transport is stopped now and then.
const BUTTONS: &[&str] = &[
    "REC",
    "OVERDUB",
    "STOP",
    "PLAY",
    "PLAY START",
    "UNDO",
    "TAP TEMPO",
    "PAD BANK A/E",
    "PAD BANK B/F",
    "FULL LEVEL",
    "16 LEVEL",
    "NOTE REPEAT",
    "ERASE",
    "MENU",
    "MAIN",
    "TRACK MIX / PAD MIX",
    "-",
    "+",
    "Q-LINK",
    "TRACK MUTE",
    "NEXT SEQ",
    "STEP SEQ",
    "SAMPLER",
    "SAMPLE EDIT",
    "PROG EDIT",
    "DATA DIAL PUSH",
];

/// Pages a walk does not wander into: the developer pages (the touch
/// tests and the calibration keep the screen's touches for themselves, by
/// design) and the console.
fn off_limits(label: &str) -> bool {
    matches!(label, "Dev" | "Calibrate touch" | "Touch lab")
}

/// One step of a walk, as the report names it.
#[derive(Clone, Debug)]
pub enum Step {
    Control {
        page: String,
        widget: String,
        id: &'static str,
        label: String,
        gesture: Gesture,
    },
    Button(&'static str),
    Shift(&'static str),
    Wheel(i32),
    Pad(usize),
    Wait,
}

impl std::fmt::Display for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Step::Control {
                page,
                widget,
                id,
                label,
                gesture,
            } => {
                write!(f, "on {page}: {gesture:?} {widget}/{id} \"{label}\"")
            }
            Step::Button(b) => write!(f, "button {b}"),
            Step::Shift(b) => write!(f, "SHIFT + {b}"),
            Step::Wheel(d) => write!(f, "wheel {d:+}"),
            Step::Pad(p) => write!(f, "pad {}", p + 1),
            Step::Wait => write!(f, "wait 0.5 s"),
        }
    }
}

/// Take one step chosen by `rng`. Returns the step taken.
pub fn step(s: &mut Session, rng: &mut Rng) -> Step {
    let roll = rng.below(100);
    if roll < 70 {
        let controls: Vec<_> = s
            .controls()
            .into_iter()
            .filter(|c| !off_limits(&c.target.label))
            .collect();
        if !controls.is_empty() {
            let c = &controls[rng.below(controls.len())];
            let step = Step::Control {
                page: s.page(),
                widget: c.widget.clone(),
                id: c.target.id,
                label: c.target.label.clone(),
                gesture: c.target.gesture,
            };
            let t = c.target.clone();
            let before = s.pixels();
            let sent = s.sent.len();
            s.press_target(&t);
            if t.enabled && s.sent.len() == sent && s.pixels() == before {
                s.dead.push(step.to_string());
            }
            return step;
        }
    }
    if roll < 82 {
        let b = BUTTONS[rng.below(BUTTONS.len())];
        s.button(b);
        return Step::Button(b);
    }
    if roll < 86 {
        let b = BUTTONS[rng.below(BUTTONS.len())];
        s.button_with("SHIFT", b);
        return Step::Shift(b);
    }
    if roll < 92 {
        let d = [-3, -1, 1, 3][rng.below(4)];
        s.wheel(d);
        return Step::Wheel(d);
    }
    if roll < 96 {
        let p = rng.below(16);
        s.pad(p, 100, 0.1);
        return Step::Pad(p);
    }
    s.run_for(0.5);
    Step::Wait
}

/// What must hold after every step. Returns what does not.
pub fn check(s: &mut Session) -> Vec<String> {
    let mut bad = Vec::new();
    // The screen shows the page the app thinks it shows.
    if s.screen.view() != s.app.view() {
        bad.push(format!(
            "the screen shows {:?} ({}) but the app is on {:?}",
            s.screen.view(),
            s.page(),
            s.app.view()
        ));
    }
    // The transport and the tempo the screen shows are the time engine's.
    let (shown, real) = (&s.screen.time.transport, s.time.info());
    if shown.state != real.state {
        bad.push(format!(
            "the screen shows the transport {:?} but it is {:?}",
            shown.state, real.state
        ));
    }
    if (shown.bpm - real.bpm).abs() > 0.05 {
        bad.push(format!(
            "the screen shows {:.1} BPM but the transport runs at {:.1}",
            shown.bpm, real.bpm
        ));
    }
    if (s.app.settings.bpm - real.bpm).abs() > 0.05 && s.time.followed_bpm().is_none() {
        bad.push(format!(
            "the tempo is {:.1} BPM but the transport runs at {:.1}",
            s.app.settings.bpm, real.bpm
        ));
    }
    // The Play page's master bar shows the Level.
    if let Some(m) = s
        .screen
        .ui
        .get::<mpfree_ui::qlink::MasterBar>(crate::play_page::MASTER)
    {
        if (m.fader_db - s.level_db).abs() > 0.05 {
            bad.push(format!(
                "the master bar shows {:.1} dB but the Level is {:.1} dB",
                m.fader_db, s.level_db
            ));
        }
    }
    // The step grid shows the armed track's clip, and the pads' mutes.
    let steps = s.time.steps();
    let pad_mutes = s.mix.play_levels().pad_mutes;
    let shows_grid = s
        .controls()
        .iter()
        .any(|c| c.widget == crate::play_page::STEPS);
    let info = s.time.info();
    let replacing = info.state == mpfree_transport::State::Recording && !info.overdub;
    if let (true, Some(g)) = (
        shows_grid,
        s.screen
            .ui
            .get::<crate::play_widgets::StepGrid>(crate::play_page::STEPS),
    ) {
        for lane in g.lanes.iter().filter(|l| l.pad < 16) {
            if lane.muted != (pad_mutes & (1 << lane.pad) != 0) {
                bad.push(format!(
                    "the step grid shows {} {} but the mixer has it {}",
                    lane.name,
                    if lane.muted { "muted" } else { "playing" },
                    if lane.muted { "playing" } else { "muted" }
                ));
            }
            // While REC replaces (no overdub), the grid hears of what it
            // takes away four times a second (`tell_every`) as the playhead
            // passes, so it is always a moment behind; with overdub, or
            // stopped, it must agree.
            if replacing {
                continue;
            }
            match &steps.clip {
                // One page: every step as the clip has it.
                Some(c) if c.steps == g.steps && lane.steps.len() >= g.steps => {
                    let row = c.row(lane.pad as u8);
                    for i in 0..g.steps {
                        let on_screen = lane.steps[i] > 0;
                        let in_clip = row.get(i).is_some_and(|st| st.is_some());
                        if on_screen != in_clip {
                            bad.push(format!(
                                "{} step {} is {} on the grid but {} in the clip ({})",
                                lane.name,
                                i + 1,
                                if on_screen { "on" } else { "off" },
                                if in_clip { "on" } else { "off" },
                                c.name
                            ));
                        }
                    }
                }
                Some(_) => {}
                None => {
                    if let Some(i) = lane.steps.iter().position(|&v| v > 0) {
                        bad.push(format!(
                            "{} step {} is on the grid but track {} has no clip",
                            lane.name,
                            i + 1,
                            steps.track + 1
                        ));
                    }
                }
            }
        }
    }
    // The buttons' lights say what the app is doing.
    let lit = |s: &Session, note: Option<u8>| note.map(|n| s.lights[usize::from(n & 0x7F)]);
    let on = crate::app::LIGHT_ON;
    let rolling = s.time.info().state.rolling();
    let play = mpfree_controller::mpc_one::button::PLAY;
    if rolling && lit(s, Some(play)) != Some(on) {
        bad.push(format!(
            "the transport rolls but PLAY's light is {:?}",
            lit(s, Some(play))
        ));
    }
    if let Some(v) = lit(s, s.app.map.note_repeat) {
        if (v == on) != s.app.generating() {
            bad.push(format!(
                "NOTE REPEAT's light is {} but the pads {} repeat",
                if v == on { "on" } else { "off" },
                if s.app.generating() { "do" } else { "don't" }
            ));
        }
    }
    if let Some(v) = lit(s, s.app.map.track_mute) {
        let keys = s.app.mute_keys() != crate::app::MuteKeys::Off;
        if (v == on) != keys {
            bad.push(format!(
                "TRACK MUTE's light is {} but the pads {} mute keys",
                if v == on { "on" } else { "off" },
                if keys { "are" } else { "aren't" }
            ));
        }
    }
    if s.quit {
        bad.push("the app quit".into());
    }
    bad
}

/// The first `steps` steps of walk `seed`, unchecked, and what they were:
/// to look at a failed walk's session at the step that failed.
pub fn replay(seed: u64, steps: usize) -> (Session, Vec<Step>) {
    let mut s = Session::new();
    let mut rng = Rng::new(seed);
    let trail = (0..steps).map(|_| step(&mut s, &mut rng)).collect();
    (s, trail)
}

/// A walk of `steps` steps from `seed`. Err names the seed and the steps
/// up to the one that failed, and what failed.
pub fn walk(seed: u64, steps: usize) -> Result<Vec<String>, String> {
    let mut s = Session::new();
    let mut rng = Rng::new(seed);
    let mut trail: Vec<Step> = Vec::new();
    for n in 0..steps {
        let mut taken = None;
        let dead_before = s.dead.len();
        let r = catch_unwind(AssertUnwindSafe(|| {
            taken = Some(step(&mut s, &mut rng));
            check(&mut s)
        }));
        if let Some(t) = taken.clone() {
            trail.push(t);
        }
        for d in s.dead.iter_mut().skip(dead_before) {
            d.push_str(&format!("\t(walk {seed} step {})", n + 1));
        }
        let failed = match r {
            Ok(bad) if bad.is_empty() => continue,
            // A second's grace: what settles by then is late, not wrong.
            Ok(bad) => match catch_unwind(AssertUnwindSafe(|| {
                s.run_for(1.0);
                check(&mut s)
            })) {
                Ok(again) if again.is_empty() => {
                    if std::env::var("MPFREE_AUDIT_DEBUG").is_ok() {
                        eprintln!("walk {seed} step {}: late: {}", n + 1, bad.join("; "));
                    }
                    continue;
                }
                Ok(again) => again.join("; "),
                Err(_) => bad.join("; "),
            },
            Err(e) => {
                let msg = e
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                format!("panicked: {msg}")
            }
        };
        let steps: Vec<String> = trail
            .iter()
            .enumerate()
            .map(|(i, t)| format!("  {:>3}. {t}", i + 1))
            .collect();
        return Err(format!(
            "walk {seed}, step {}: {failed}\n{}\n  (MPFREE_WALK_SEED={seed} walks it again)",
            n + 1,
            steps.join("\n")
        ));
    }
    Ok(std::mem::take(&mut s.dead))
}

#[cfg(test)]
mod tests {
    use super::super::on_big_stack;
    use super::*;

    /// Walks from fixed seeds (a seed of your own with MPFREE_WALK_SEED,
    /// and more or longer walks with MPFREE_WALKS and MPFREE_WALK_STEPS).
    #[test]
    fn random_walks_keep_the_screen_and_the_engines_together() {
        let var = |k: &str, d: u64| {
            std::env::var(k)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(d)
        };
        let steps = var("MPFREE_WALK_STEPS", 60) as usize;
        let seeds: Vec<u64> = match std::env::var("MPFREE_WALK_SEED")
            .ok()
            .and_then(|v| v.parse().ok())
        {
            Some(seed) => vec![seed],
            None => (1..=var("MPFREE_WALKS", 12)).collect(),
        };
        let (failures, dead) = on_big_stack(move || {
            let mut failures = Vec::new();
            let mut dead: Vec<String> = Vec::new();
            for seed in seeds {
                match walk(seed, steps) {
                    Ok(d) => dead.extend(d),
                    Err(e) => failures.push(e),
                }
            }
            (failures, dead)
        });
        // Controls that did nothing when a walk pressed them, by how often.
        let mut counts: std::collections::BTreeMap<&str, (usize, &str)> = Default::default();
        for d in &dead {
            let (what, at) = d.split_once('\t').unwrap_or((d, ""));
            counts.entry(what).or_insert((0, at)).0 += 1;
        }
        let mut by: Vec<(&str, (usize, &str))> = counts.into_iter().collect();
        by.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
        let report: Vec<String> = by
            .iter()
            .map(|(d, (n, at))| format!("{n:>5}  {d}  {at}"))
            .collect();
        if let Ok(path) = std::env::var("MPFREE_WALK_OUT") {
            std::fs::write(&path, report.join("\n")).expect("the walk report is written");
        }
        println!("{} presses did nothing, {} kinds", dead.len(), report.len());
        assert!(
            failures.is_empty(),
            "{} walks failed:\n\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }
}
