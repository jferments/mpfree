//! Handles: the strings a skin's widgets read and write values through
//! (`Parameter 3`, `Invert/Parameter 3`, `IndexedEnabling/1/4/Parameter 0`,
//! `Local/2/showAdvanced`, ...), section 7 of the SDK's UI reference.

use std::collections::HashMap;

/// What a skin needs from the plugin: its parameters, each 0 to 1.
pub trait Params {
    fn count(&self) -> usize;
    fn value(&self, index: usize) -> f32;
    fn name(&self, index: usize) -> String;
    /// The plugin's text for the parameter's current value.
    fn text(&self, index: usize) -> String;
}

/// A parsed handle.
#[derive(Clone, Debug, PartialEq)]
pub enum Handle {
    Param(usize),
    Named(String),
    /// A tab's Q-Link (`PageIndex T, S, Q`): the tab is 1-based and relative
    /// to the current one, the sub-tab and Q-Link 1-based.
    Page {
        tab: usize,
        sub: usize,
        qlink: usize,
    },
    Local {
        range: u32,
        id: String,
    },
    Fixed {
        value: f32,
        text: String,
        name: String,
    },
    Invalid,
    Invert(Box<Handle>),
    /// Display only: shown in another form, the value untouched.
    Display(Display, Box<Handle>),
    NonZero(Box<Handle>),
    /// Valid while the handle, in `n` steps, is at a step in `lo..=hi`.
    Enabling {
        lo: u32,
        hi: u32,
        n: u32,
        h: Box<Handle>,
    },
    FilterEmpty(Box<Handle>),
    Max(Box<Handle>, Box<Handle>),
    Min(Box<Handle>, Box<Handle>),
    /// Returns to its default when the finger lifts.
    Momentary(Box<Handle>),
    /// Modifiers that change nothing here (`Hold`, `FromGesture`,
    /// `DisableCoarseIncrement`, `TabLimited`, `CommandGroup`).
    Same(Box<Handle>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Display {
    Prefix(String),
    Suffix(String),
    Name(String),
    Affine(f32, f32),
    Format(String),
    AsDb,
    AsDbFromGain,
}

/// Splits `Max[a, b]` into its two handles.
fn pair(inner: &str) -> Option<(&str, &str)> {
    // The separator is exactly ", " at nesting depth 0.
    let mut depth = 0;
    let b = inner.as_bytes();
    for i in 0..b.len() {
        match b[i] {
            b'[' => depth += 1,
            b']' => depth -= 1,
            b',' if depth == 0 && b.get(i + 1) == Some(&b' ') => {
                return Some((&inner[..i], &inner[i + 2..]));
            }
            _ => {}
        }
    }
    None
}

/// The argument in `Name[arg]/rest`, and `rest`.
fn bracketed<'a>(s: &'a str, name: &str) -> Option<(&'a str, &'a str)> {
    let rest = s.strip_prefix(name)?.strip_prefix('[')?;
    let close = rest.find("]/")?;
    Some((&rest[..close], &rest[close + 2..]))
}

impl Handle {
    pub fn parse(s: &str) -> Handle {
        Handle::parse_depth(s.trim(), 0)
    }

    fn parse_depth(s: &str, depth: usize) -> Handle {
        if depth > 32 {
            return Handle::Invalid;
        }
        let sub = |r: &str| Box::new(Handle::parse_depth(r, depth + 1));
        if let Some(n) = s.strip_prefix("Parameter ") {
            return n
                .trim()
                .parse()
                .map(Handle::Param)
                .unwrap_or(Handle::Invalid);
        }
        if let Some(n) = s.strip_prefix("Parameter/Named/") {
            return Handle::Named(n.to_string());
        }
        if let Some(rest) = s.strip_prefix("PageIndex ") {
            let v: Vec<usize> = rest
                .split(',')
                .filter_map(|p| p.trim().parse().ok())
                .collect();
            return match v[..] {
                [tab, qlink] => Handle::Page { tab, sub: 1, qlink },
                [tab, sub, qlink] => Handle::Page { tab, sub, qlink },
                _ => Handle::Invalid,
            };
        }
        if let Some(rest) = s.strip_prefix("Local/") {
            let (range, id) = rest.split_once('/').unwrap_or((rest, ""));
            return match range.parse() {
                Ok(range) if range > 0 => Handle::Local {
                    range,
                    id: id.to_string(),
                },
                _ => Handle::Invalid,
            };
        }
        if let Some(rest) = s.strip_prefix("Fixed/") {
            return fixed(rest);
        }
        for (name, ctor) in [("Max", true), ("Min", false)] {
            if let Some(inner) = s.strip_prefix(name).and_then(|r| r.strip_prefix('[')) {
                if let Some((a, b)) = inner.strip_suffix(']').and_then(pair) {
                    return if ctor {
                        Handle::Max(sub(a), sub(b))
                    } else {
                        Handle::Min(sub(a), sub(b))
                    };
                }
            }
        }
        for (name, f) in [
            ("WithPrefix", 0),
            ("WithSuffix", 1),
            ("WithName", 2),
            ("AffineMap", 3),
            ("DisplayValue", 4),
        ] {
            if let Some((arg, rest)) = bracketed(s, name) {
                let d = match f {
                    0 => Display::Prefix(arg.to_string()),
                    1 => Display::Suffix(arg.to_string()),
                    2 => Display::Name(arg.to_string()),
                    3 => {
                        let (k, a) = arg.split_once(',').unwrap_or((arg, "0"));
                        Display::Affine(
                            k.trim().parse().unwrap_or(1.0),
                            a.trim().parse().unwrap_or(0.0),
                        )
                    }
                    _ => Display::Format(arg.to_string()),
                };
                return Handle::Display(d, sub(rest));
            }
        }
        if let Some(rest) = s.strip_prefix("CommandGroup") {
            if let Some(i) = rest.find("]/") {
                return Handle::Same(sub(&rest[i + 2..]));
            }
        }
        if let Some(rest) = s.strip_prefix("IndexedEnabling/") {
            let mut it = rest.splitn(3, '/');
            if let (Some(i), Some(n), Some(h)) = (it.next(), it.next(), it.next()) {
                if let (Ok(i), Ok(n)) = (i.parse(), n.parse()) {
                    return Handle::Enabling {
                        lo: i,
                        hi: i,
                        n,
                        h: sub(h),
                    };
                }
            }
            return Handle::Invalid;
        }
        if let Some(rest) = s.strip_prefix("RangedEnabling/") {
            let mut it = rest.splitn(4, '/');
            if let (Some(lo), Some(hi), Some(n), Some(h)) =
                (it.next(), it.next(), it.next(), it.next())
            {
                if let (Ok(lo), Ok(hi), Ok(n)) = (lo.parse(), hi.parse(), n.parse()) {
                    return Handle::Enabling {
                        lo,
                        hi,
                        n,
                        h: sub(h),
                    };
                }
            }
            return Handle::Invalid;
        }
        if let Some(rest) = s.strip_prefix("TabLimited/") {
            let mut it = rest.splitn(4, '/');
            let _ = (it.next(), it.next(), it.next());
            return it
                .next()
                .map(|h| Handle::Same(sub(h)))
                .unwrap_or(Handle::Invalid);
        }
        if let Some((m, rest)) = s.split_once('/') {
            return match m {
                "Invert" => Handle::Invert(sub(rest)),
                "Hold" | "FromGesture" | "DisableCoarseIncrement" => Handle::Same(sub(rest)),
                "AsDB" => Handle::Display(Display::AsDb, sub(rest)),
                "AsDBFromGain" => Handle::Display(Display::AsDbFromGain, sub(rest)),
                "NonZero" => Handle::NonZero(sub(rest)),
                "FilterEmptyStrings" => Handle::FilterEmpty(sub(rest)),
                "ResetOnGestureEnd" => Handle::Momentary(sub(rest)),
                _ => Handle::Invalid,
            };
        }
        Handle::Invalid
    }
}

/// `Fixed/displayRange a b valueRange a b fineIncrement x coarseIncrement x
/// defaultValue x displayString 'str' name 'str' value x`.
fn fixed(rest: &str) -> Handle {
    let mut value = 0.0;
    let mut range = (0.0f32, 1.0f32);
    let mut text = String::new();
    let mut name = String::new();
    let mut toks = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in rest.chars() {
        match c {
            '\'' => quoted = !quoted,
            ' ' if !quoted => {
                if !cur.is_empty() {
                    toks.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        toks.push(cur);
    }
    let mut i = 0;
    while i < toks.len() {
        let next = |k: usize| toks.get(i + k).cloned().unwrap_or_default();
        match toks[i].as_str() {
            "valueRange" => {
                range = (
                    next(1).parse().unwrap_or(0.0),
                    next(2).parse().unwrap_or(1.0),
                );
                i += 2;
            }
            "displayString" => {
                text = next(1);
                i += 1;
            }
            "name" => {
                name = next(1);
                i += 1;
            }
            "value" => {
                value = next(1).parse().unwrap_or(0.0);
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    let span = range.1 - range.0;
    let norm = if span.abs() > 1e-9 {
        ((value - range.0) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    Handle::Fixed {
        value: norm,
        text,
        name,
    }
}

/// A handle's state now.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Val {
    pub valid: bool,
    /// 0 to 1.
    pub value: f32,
    /// Discrete steps (replacement strings, a Local's range).
    pub steps: Option<u32>,
    pub text: String,
    pub name: String,
    /// What a write goes to.
    pub target: Option<Target>,
    /// Writes are flipped (Invert).
    pub inverted: bool,
    pub momentary: bool,
}

impl Val {
    /// The step index for `n` steps.
    pub fn step(&self, n: u32) -> u32 {
        if n <= 1 {
            return 0;
        }
        (self.value.clamp(0.0, 1.0) * (n - 1) as f32).round() as u32
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Param(usize),
    Local(String, u32),
}

/// What handles are evaluated against.
pub struct Env<'a> {
    pub params: &'a dyn Params,
    /// Step names per final handle ("Parameter 7").
    pub replacements: &'a HashMap<String, Vec<String>>,
    pub locals: &'a HashMap<String, u32>,
    /// `PageIndex` resolution: (absolute tab, sub-tab 1-based, Q-Link) to a
    /// parameter. `tab` here is the current tab's index.
    pub page: &'a dyn Fn(isize, usize, usize) -> Option<usize>,
    pub param_offset: usize,
}

impl Env<'_> {
    pub fn eval(&self, h: &Handle) -> Val {
        self.eval_depth(h, 0)
    }

    fn param(&self, i: usize) -> Val {
        if i >= self.params.count() {
            return Val::default();
        }
        let key = format!("Parameter {i}");
        let mut v = Val {
            valid: true,
            value: self.params.value(i).clamp(0.0, 1.0),
            name: self.params.name(i),
            target: Some(Target::Param(i)),
            ..Val::default()
        };
        match self.replacements.get(&key) {
            Some(names) => {
                let n = names.len() as u32;
                v.steps = Some(n);
                v.text = names[v.step(n) as usize].clone();
            }
            None => v.text = self.params.text(i),
        }
        v
    }

    fn eval_depth(&self, h: &Handle, depth: usize) -> Val {
        if depth > 32 {
            return Val::default();
        }
        let e = |h: &Handle| self.eval_depth(h, depth + 1);
        match h {
            Handle::Param(i) => self.param(i + self.param_offset),
            Handle::Named(n) => (0..self.params.count())
                .find(|&i| self.params.name(i) == *n)
                .map(|i| self.param(i))
                .unwrap_or_default(),
            Handle::Page { tab, sub, qlink } => {
                match (self.page)(*tab as isize - 1, *sub, *qlink) {
                    Some(i) => self.param(i),
                    None => Val::default(),
                }
            }
            Handle::Local { range, id } => {
                let n = (*range).max(1);
                let s = self.locals.get(id).copied().unwrap_or(0).min(n - 1);
                Val {
                    valid: true,
                    value: if n > 1 {
                        s as f32 / (n - 1) as f32
                    } else {
                        0.0
                    },
                    steps: Some(n),
                    text: s.to_string(),
                    name: id.clone(),
                    target: Some(Target::Local(id.clone(), n)),
                    ..Val::default()
                }
            }
            Handle::Fixed { value, text, name } => Val {
                valid: true,
                value: *value,
                text: text.clone(),
                name: name.clone(),
                ..Val::default()
            },
            Handle::Invalid => Val::default(),
            Handle::Invert(h) => {
                let mut v = e(h);
                v.value = 1.0 - v.value;
                v.inverted = !v.inverted;
                v
            }
            Handle::Display(d, h) => {
                let mut v = e(h);
                match d {
                    Display::Prefix(p) => v.text = format!("{p}{}", v.text),
                    Display::Suffix(p) => v.text = format!("{}{p}", v.text),
                    Display::Name(n) => v.name = n.clone(),
                    Display::Affine(k, a) => v.text = number(k * v.value + a, None),
                    Display::Format(f) => v.text = printf(f, v.value),
                    Display::AsDb => v.text = format!("{:.1} dB", -100.0 + 100.0 * v.value),
                    Display::AsDbFromGain => {
                        v.text = if v.value <= 1e-5 {
                            "-inf dB".into()
                        } else {
                            format!("{:.1} dB", 20.0 * v.value.log10())
                        }
                    }
                }
                v
            }
            Handle::NonZero(h) => {
                let mut v = e(h);
                v.valid &= v.value.abs() > 1e-6;
                v
            }
            Handle::Enabling { lo, hi, n, h } => {
                let mut v = e(h);
                let s = v.step((*n).max(1));
                v.valid &= s >= *lo && s <= *hi;
                v
            }
            Handle::FilterEmpty(h) => {
                let mut v = e(h);
                v.valid &= !v.text.is_empty();
                v
            }
            Handle::Max(a, b) | Handle::Min(a, b) => {
                let (va, vb) = (e(a), e(b));
                let max = matches!(h, Handle::Max(..));
                let pick_a = if !vb.valid {
                    true
                } else if !va.valid {
                    false
                } else if max {
                    va.value >= vb.value
                } else {
                    va.value <= vb.value
                };
                if pick_a {
                    va
                } else {
                    vb
                }
            }
            Handle::Momentary(h) => {
                let mut v = e(h);
                v.momentary = true;
                v
            }
            Handle::Same(h) => e(h),
        }
    }
}

/// A number as the MPC shows a plain value.
fn number(v: f32, decimals: Option<usize>) -> String {
    match decimals {
        Some(d) => format!("{v:.d$}"),
        None if (v - v.round()).abs() < 1e-4 => format!("{}", v.round() as i64),
        None => format!("{v:.2}"),
    }
}

/// A printf-style format with one number (`%.1f`, `%d`, `%i`), as
/// `DisplayValue[...]` takes it.
fn printf(fmt: &str, v: f32) -> String {
    let Some(p) = fmt.find('%') else {
        return fmt.to_string();
    };
    let rest = &fmt[p + 1..];
    let end = rest
        .find(|c: char| c.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    let spec = &rest[..end];
    let conv = rest[end..].chars().next().unwrap_or('f');
    let decimals = spec.split_once('.').and_then(|(_, d)| d.parse().ok());
    let body = match conv {
        'd' | 'i' => format!("{}", v.round() as i64),
        _ => number(v, Some(decimals.unwrap_or(6))),
    };
    format!(
        "{}{}{}",
        &fmt[..p],
        body,
        &rest[(end + conv.len_utf8()).min(rest.len())..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct P(Vec<f32>);
    impl Params for P {
        fn count(&self) -> usize {
            self.0.len()
        }
        fn value(&self, i: usize) -> f32 {
            self.0[i]
        }
        fn name(&self, i: usize) -> String {
            ["Wave", "Cutoff", "Res", ""][i].to_string()
        }
        fn text(&self, i: usize) -> String {
            format!("{:.0}%", self.0[i] * 100.0)
        }
    }

    fn env<'a>(
        p: &'a P,
        r: &'a HashMap<String, Vec<String>>,
        l: &'a HashMap<String, u32>,
        page: &'a dyn Fn(isize, usize, usize) -> Option<usize>,
    ) -> Env<'a> {
        Env {
            params: p,
            replacements: r,
            locals: l,
            page,
            param_offset: 0,
        }
    }

    #[test]
    fn parses_handles() {
        assert_eq!(Handle::parse("Parameter 12"), Handle::Param(12));
        assert_eq!(Handle::parse("Invalid"), Handle::Invalid);
        assert_eq!(Handle::parse("Data"), Handle::Invalid);
        assert_eq!(
            Handle::parse("PageIndex 1, 13"),
            Handle::Page {
                tab: 1,
                sub: 1,
                qlink: 13
            }
        );
        assert_eq!(
            Handle::parse("Invert/Hold/Parameter 3"),
            Handle::Invert(Box::new(Handle::Same(Box::new(Handle::Param(3)))))
        );
        assert_eq!(
            Handle::parse("Max[Parameter 12, Min[Parameter 1, Parameter 2]]"),
            Handle::Max(
                Box::new(Handle::Param(12)),
                Box::new(Handle::Min(
                    Box::new(Handle::Param(1)),
                    Box::new(Handle::Param(2))
                ))
            )
        );
        assert_eq!(
            Handle::parse("WithSuffix[ Hz]/Parameter 2"),
            Handle::Display(Display::Suffix(" Hz".into()), Box::new(Handle::Param(2)))
        );
        assert!(matches!(
            Handle::parse("IndexedEnabling/1/4/Local/2/adv"),
            Handle::Enabling {
                lo: 1,
                hi: 1,
                n: 4,
                ..
            }
        ));
    }

    #[test]
    fn evaluates_against_the_plugin() {
        let p = P(vec![1.0 / 3.0, 0.25, 0.0, 0.5]);
        let mut r = HashMap::new();
        r.insert(
            "Parameter 0".to_string(),
            vec!["Sine".into(), "Saw".into(), "Square".into(), "Noise".into()],
        );
        let mut l = HashMap::new();
        l.insert("adv".to_string(), 1u32);
        let page = |t: isize, s: usize, q: usize| (t == 0 && s == 1 && q == 13).then_some(1);
        let e = env(&p, &r, &l, &page);
        let v = e.eval(&Handle::parse("Parameter 0"));
        assert_eq!(
            (v.steps, v.text.as_str(), v.name.as_str()),
            (Some(4), "Saw", "Wave")
        );
        let v = e.eval(&Handle::parse("WithSuffix[ !]/Parameter 1"));
        assert_eq!(v.text, "25% !");
        assert_eq!(v.target, Some(Target::Param(1)));
        let v = e.eval(&Handle::parse("Invert/Parameter 1"));
        assert!((v.value - 0.75).abs() < 1e-6 && v.inverted);
        assert!(!e.eval(&Handle::parse("Parameter 9")).valid);
        assert!(!e.eval(&Handle::parse("NonZero/Parameter 2")).valid);
        assert!(
            e.eval(&Handle::parse("IndexedEnabling/1/4/Parameter 0"))
                .valid
        );
        assert!(
            !e.eval(&Handle::parse("IndexedEnabling/2/4/Parameter 0"))
                .valid
        );
        assert!(
            e.eval(&Handle::parse("RangedEnabling/1/2/4/Parameter 0"))
                .valid
        );
        assert!(
            e.eval(&Handle::parse("IndexedEnabling/1/2/Local/2/adv"))
                .valid
        );
        assert_eq!(e.eval(&Handle::parse("PageIndex 1, 13")).name, "Cutoff");
        assert_eq!(
            e.eval(&Handle::parse("Parameter/Named/Res")).target,
            Some(Target::Param(2))
        );
        assert_eq!(
            e.eval(&Handle::parse("DisplayValue[%.1f dB]/Parameter 3"))
                .text,
            "0.5 dB"
        );
        assert_eq!(
            e.eval(&Handle::parse("AffineMap[100,0]/Parameter 1")).text,
            "25"
        );
        let v = e.eval(&Handle::parse(
            "Fixed/displayRange 0 1 valueRange 0 1 fineIncrement 0.01 coarseIncrement 0.1 defaultValue 0.5 displayString 'Half way' name 'Const' value 0.5",
        ));
        assert_eq!(
            (v.value, v.text.as_str(), v.name.as_str()),
            (0.5, "Half way", "Const")
        );
        assert_eq!(
            e.eval(&Handle::parse("Max[Parameter 1, Parameter 3]")).name,
            ""
        );
        assert_eq!(
            e.eval(&Handle::parse("Min[Parameter 1, Parameter 3]")).name,
            "Cutoff"
        );
    }
}
