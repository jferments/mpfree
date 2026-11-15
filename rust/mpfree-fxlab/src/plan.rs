//! Sweep plans: which settings of an effect to capture, in which contexts.
//!
//! An effect's controls come from the registry (ours) or from a control
//! list written by hand from the published manual (an AIR effect, whose
//! parameters we cannot query). A plan is then:
//!
//! - the defaults;
//! - every control on its own across its whole range, the others at their
//!   defaults (`sweep_points` values each, every choice of a choice);
//! - grids of every pair of continuous controls (`grid_points` squared each),
//!   because controls interact (drive and tone, feedback and time);
//! - random settings of all controls at once, spread by a Latin hypercube so
//!   `random` runs cover each control's range evenly;
//!
//! each played in every context: a programme at a level (the Source's gain).
//! Tempo is a project setting on the MPC, not something a run can change, so
//! tempo contexts are separate plans run in separate projects.

use mpfree_fx::registry::FxEntry;
use mpfree_node::Curve;

use crate::signals::Rng;

/// How a control's values are spread.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Linear,
    Log,
    /// Whole numbers.
    Stepped,
    /// Named choices; the value is the index.
    Choice(Vec<String>),
    Toggle,
}

/// One control of an effect.
#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub kind: Kind,
    pub unit: String,
    /// Registry parameter id (ours); None for a hand-written list.
    pub id: Option<u16>,
}

impl Control {
    fn continuous(&self) -> bool {
        matches!(self.kind, Kind::Linear | Kind::Log)
    }

    /// The value at `u` in 0..1 of the control's range, on its own scale.
    pub fn at(&self, u: f64) -> f64 {
        let u = u.clamp(0.0, 1.0);
        match &self.kind {
            Kind::Linear => self.min + u * (self.max - self.min),
            Kind::Log => self.min * (self.max / self.min).powf(u),
            Kind::Stepped => (self.min + u * (self.max - self.min)).round(),
            Kind::Choice(c) => (u * (c.len().max(1) - 1) as f64).round(),
            Kind::Toggle => u.round(),
        }
    }

    /// `n` values across the range (every value of a discrete control with
    /// fewer than `n`).
    pub fn points(&self, n: usize) -> Vec<f64> {
        match &self.kind {
            Kind::Choice(c) => (0..c.len()).map(|i| i as f64).collect(),
            Kind::Toggle => vec![0.0, 1.0],
            _ => {
                let mut v: Vec<f64> = (0..n.max(2))
                    .map(|i| self.at(i as f64 / (n.max(2) - 1) as f64))
                    .collect();
                v.dedup();
                v
            }
        }
    }
}

/// An effect's controls.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlList {
    pub effect: String,
    pub controls: Vec<Control>,
}

impl ControlList {
    /// From our registry: every settable parameter.
    pub fn from_registry(entry: &FxEntry) -> ControlList {
        let node = (entry.make)();
        let controls = node
            .params()
            .iter()
            .filter(|p| !p.readonly)
            .map(|p| Control {
                name: p.name.to_string(),
                min: p.min as f64,
                max: p.max as f64,
                default: p.default as f64,
                kind: match p.curve {
                    Curve::Linear => Kind::Linear,
                    Curve::Log => Kind::Log,
                    Curve::Stepped => Kind::Stepped,
                    Curve::Choice(c) => Kind::Choice(c.iter().map(|s| s.to_string()).collect()),
                    Curve::Toggle => Kind::Toggle,
                },
                unit: format!("{:?}", p.unit),
                id: Some(p.id),
            })
            .collect();
        ControlList {
            effect: entry.id.to_string(),
            controls,
        }
    }

    /// A hand-written list, one control per line, fields separated by tabs
    /// (shown here as spaces):
    ///
    /// ```text
    /// effect   AIR Flanger
    /// # name   kind     min                     max   default   unit
    /// Rate     log      0.05                    10    0.5       Hz
    /// Depth    linear   0                       100   50        %
    /// Mode     choice   Classic,Through-zero    -     Classic
    /// Sync     toggle   -                       -     off
    /// ```
    ///
    /// A choice's third field lists its choices and its default is one of
    /// them; a toggle's default is on or off.
    pub fn from_text(text: &str) -> Result<ControlList, String> {
        let mut list = ControlList {
            effect: String::new(),
            controls: Vec::new(),
        };
        for (n, line) in text.lines().enumerate() {
            let line = line.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let f: Vec<&str> = line.split('\t').map(str::trim).collect();
            let err = |m: &str| format!("line {}: {m}", n + 1);
            if f[0] == "effect" {
                list.effect = f
                    .get(1)
                    .ok_or_else(|| err("effect needs a name"))?
                    .to_string();
                continue;
            }
            if f.len() < 5 {
                return Err(err("expected name, kind, min, max, default (and unit)"));
            }
            let num = |s: &str| -> Result<f64, String> {
                s.parse().map_err(|_| err(&format!("bad number {s:?}")))
            };
            let unit = f.get(5).unwrap_or(&"").to_string();
            let (kind, min, max, default) = match f[1] {
                "linear" | "log" | "stepped" => {
                    let (min, max, def) = (num(f[2])?, num(f[3])?, num(f[4])?);
                    if min.partial_cmp(&max) != Some(std::cmp::Ordering::Less)
                        || def < min
                        || def > max
                    {
                        return Err(err("need min < max and min <= default <= max"));
                    }
                    let kind = match f[1] {
                        "linear" => Kind::Linear,
                        "log" if min > 0.0 => Kind::Log,
                        "log" => return Err(err("a log control needs min > 0")),
                        _ => Kind::Stepped,
                    };
                    (kind, min, max, def)
                }
                "choice" => {
                    let c: Vec<String> = f[2].split(',').map(|s| s.trim().to_string()).collect();
                    let def = c
                        .iter()
                        .position(|x| x == f[4])
                        .ok_or_else(|| err("default is not one of the choices"))?;
                    let max = (c.len() - 1) as f64;
                    (Kind::Choice(c), 0.0, max, def as f64)
                }
                "toggle" => {
                    let def = match f[4] {
                        "on" => 1.0,
                        "off" => 0.0,
                        _ => return Err(err("a toggle's default is on or off")),
                    };
                    (Kind::Toggle, 0.0, 1.0, def)
                }
                k => return Err(err(&format!("unknown kind {k:?}"))),
            };
            list.controls.push(Control {
                name: f[0].to_string(),
                min,
                max,
                default,
                kind,
                unit,
                id: None,
            });
        }
        if list.controls.is_empty() {
            return Err("no controls".into());
        }
        Ok(list)
    }
}

/// A context a setting is played in.
#[derive(Clone, Debug, PartialEq)]
pub struct Context {
    /// Programme file name, as the Source finds it.
    pub programme: String,
    /// Gain the Source applies, dB.
    pub gain_db: f64,
}

/// One capture to make.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub id: usize,
    /// `defaults`, `sweep`, `grid` or `random`.
    pub kind: &'static str,
    /// One value per control, in the list's order.
    pub values: Vec<f64>,
    pub context: Context,
}

#[derive(Clone, Debug)]
pub struct PlanOptions {
    pub sweep_points: usize,
    /// 0: no grids.
    pub grid_points: usize,
    pub random: usize,
    pub seed: u64,
    pub contexts: Vec<Context>,
}

impl Default for PlanOptions {
    fn default() -> PlanOptions {
        PlanOptions {
            sweep_points: 7,
            grid_points: 3,
            random: 32,
            seed: 1,
            contexts: vec![Context {
                programme: "short.wav".into(),
                gain_db: 0.0,
            }],
        }
    }
}

/// The settings to capture, each in every context, duplicates removed.
pub fn plan(list: &ControlList, opts: &PlanOptions) -> Vec<Run> {
    let defaults: Vec<f64> = list.controls.iter().map(|c| c.default).collect();
    let mut settings: Vec<(&'static str, Vec<f64>)> = vec![("defaults", defaults.clone())];
    for (i, c) in list.controls.iter().enumerate() {
        for v in c.points(opts.sweep_points) {
            let mut s = defaults.clone();
            s[i] = v;
            settings.push(("sweep", s));
        }
    }
    if opts.grid_points > 0 {
        let cont: Vec<usize> = (0..list.controls.len())
            .filter(|&i| list.controls[i].continuous())
            .collect();
        for (a, &i) in cont.iter().enumerate() {
            for &j in &cont[a + 1..] {
                for vi in list.controls[i].points(opts.grid_points) {
                    for vj in list.controls[j].points(opts.grid_points) {
                        let mut s = defaults.clone();
                        s[i] = vi;
                        s[j] = vj;
                        settings.push(("grid", s));
                    }
                }
            }
        }
    }
    if opts.random > 0 {
        let mut rng = Rng::new(opts.seed);
        let n = opts.random;
        // Latin hypercube: each control's range cut into n strata, one value
        // from each, the strata shuffled independently per control.
        let mut cols: Vec<Vec<f64>> = Vec::new();
        for _ in &list.controls {
            let mut strata: Vec<f64> = (0..n).map(|k| (k as f64 + rng.unit()) / n as f64).collect();
            for k in (1..n).rev() {
                let j = (rng.next_u64() % (k as u64 + 1)) as usize;
                strata.swap(k, j);
            }
            cols.push(strata);
        }
        for r in 0..n {
            settings.push((
                "random",
                list.controls
                    .iter()
                    .enumerate()
                    .map(|(i, c)| c.at(cols[i][r]))
                    .collect(),
            ));
        }
    }
    let mut seen: Vec<Vec<f64>> = Vec::new();
    let mut runs = Vec::new();
    for (kind, values) in settings {
        if seen.iter().any(|s| s == &values) {
            continue;
        }
        seen.push(values.clone());
        for ctx in &opts.contexts {
            runs.push(Run {
                id: runs.len(),
                kind,
                values: values.clone(),
                context: ctx.clone(),
            });
        }
    }
    runs
}

/// A plan as tab-separated text: a header naming the controls, then one run
/// per line (choices by name).
pub fn to_tsv(list: &ControlList, runs: &[Run]) -> String {
    let mut s = format!("# effect\t{}\nid\tkind\tprogramme\tgain_db", list.effect);
    for c in &list.controls {
        s.push('\t');
        s.push_str(&c.name);
    }
    s.push('\n');
    for r in runs {
        s.push_str(&format!(
            "{}\t{}\t{}\t{}",
            r.id, r.kind, r.context.programme, r.context.gain_db
        ));
        for (c, v) in list.controls.iter().zip(&r.values) {
            s.push('\t');
            match &c.kind {
                Kind::Choice(names) => {
                    s.push_str(names.get(*v as usize).map(String::as_str).unwrap_or("?"))
                }
                Kind::Toggle => s.push_str(if *v >= 0.5 { "on" } else { "off" }),
                _ => s.push_str(&format!("{}", (v * 1e4).round() / 1e4)),
            }
        }
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLANGER: &str = "effect\tAIR Flanger\n# name\tkind\tmin\tmax\tdefault\tunit\n\
        Rate\tlog\t0.05\t10\t0.5\tHz\nDepth\tlinear\t0\t100\t50\t%\n\
        Mode\tchoice\tClassic,Through-zero\t-\tClassic\nSync\ttoggle\t-\t-\toff\n";

    #[test]
    fn a_hand_written_list_reads() {
        let l = ControlList::from_text(FLANGER).unwrap();
        assert_eq!(l.effect, "AIR Flanger");
        assert_eq!(l.controls.len(), 4);
        assert_eq!(l.controls[0].kind, Kind::Log);
        assert_eq!(
            l.controls[2].kind,
            Kind::Choice(vec!["Classic".into(), "Through-zero".into()])
        );
        assert_eq!(l.controls[3].default, 0.0);
        assert!(
            ControlList::from_text("Rate\tlog\t0\t10\t1\n").is_err(),
            "log needs min > 0"
        );
        assert!(ControlList::from_text("Mode\tchoice\ta,b\t-\tc\n").is_err());
    }

    #[test]
    fn sweeps_cover_every_control_end_to_end() {
        let l = ControlList::from_text(FLANGER).unwrap();
        let runs = plan(
            &l,
            &PlanOptions {
                grid_points: 0,
                random: 0,
                ..PlanOptions::default()
            },
        );
        // Rate's sweep reaches both ends, on a log scale (geometric middle).
        let rates: Vec<f64> = runs
            .iter()
            .filter(|r| r.kind == "sweep")
            .map(|r| r.values[0])
            .collect();
        assert!(
            rates.iter().any(|v| (v - 0.05).abs() < 1e-12)
                && rates.iter().any(|v| (v - 10.0).abs() < 1e-9)
        );
        assert!(rates
            .iter()
            .any(|v| (v - (0.05f64 * 10.0).sqrt()).abs() < 1e-9));
        // Both modes and both sync states appear; the defaults run is first
        // and no setting is captured twice.
        assert!(runs.iter().any(|r| r.values[2] == 1.0) && runs.iter().any(|r| r.values[3] == 1.0));
        assert_eq!(runs[0].kind, "defaults");
        for (i, a) in runs.iter().enumerate() {
            assert!(runs[i + 1..].iter().all(|b| b.values != a.values));
        }
    }

    #[test]
    fn random_settings_fill_each_range_evenly() {
        let l = ControlList::from_text(FLANGER).unwrap();
        let runs = plan(
            &l,
            &PlanOptions {
                sweep_points: 2,
                grid_points: 0,
                random: 20,
                ..PlanOptions::default()
            },
        );
        let depth: Vec<f64> = runs
            .iter()
            .filter(|r| r.kind == "random")
            .map(|r| r.values[1])
            .collect();
        assert_eq!(depth.len(), 20);
        // One value in each twentieth of the range.
        for k in 0..20 {
            let (lo, hi) = (k as f64 * 5.0, (k + 1) as f64 * 5.0);
            assert_eq!(
                depth.iter().filter(|v| **v >= lo && **v < hi).count(),
                1,
                "stratum {k}"
            );
        }
    }

    #[test]
    fn contexts_multiply_and_grids_pair_continuous_controls() {
        let l = ControlList::from_text(FLANGER).unwrap();
        let mut o = PlanOptions {
            sweep_points: 2,
            grid_points: 3,
            random: 0,
            ..PlanOptions::default()
        };
        o.contexts.push(Context {
            programme: "short.wav".into(),
            gain_db: -12.0,
        });
        let runs = plan(&l, &o);
        // Rate x Depth is the only continuous pair: 9 grid settings, the
        // corners and edge points the sweeps already hold dropped.
        let grid = runs.iter().filter(|r| r.kind == "grid").count();
        assert!(grid > 0 && grid <= 2 * 9);
        assert_eq!(runs.len() % 2, 0);
        assert!(runs.iter().any(|r| r.context.gain_db == -12.0));
        let tsv = to_tsv(&l, &runs);
        assert!(tsv
            .lines()
            .nth(1)
            .unwrap()
            .ends_with("Rate\tDepth\tMode\tSync"));
        assert!(tsv.contains("\tClassic\toff\n"));
    }

    #[test]
    fn every_registry_effect_has_a_plan() {
        for e in mpfree_fx::EFFECTS.iter() {
            let l = ControlList::from_registry(e);
            let runs = plan(&l, &PlanOptions::default());
            assert!(!runs.is_empty(), "{}", e.id);
            for r in &runs {
                for (c, v) in l.controls.iter().zip(&r.values) {
                    assert!(
                        *v >= c.min - 1e-9 && *v <= c.max + 1e-9,
                        "{} {} = {v}",
                        e.id,
                        c.name
                    );
                }
            }
        }
    }
}
