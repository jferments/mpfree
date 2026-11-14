//! Sections: how a Synth tab page groups its controls (Jay's pick of
//! 14 Nov, "Instrument and effect page design rules").
//!
//! A page is a few labelled sections, each about one job, laid out by a
//! short layout spec of the sections' keys (the same syntax as page
//! layouts: `row(filter:520, col(fenv:300, row(follow, drive)))`). A
//! section takes its colour from its job, sizes its controls by how much
//! they matter (a hero, a main control, a standard one or a trim), shows a
//! drawing of what it does when the engine has one, and may link to the
//! page that holds more of it.
//!
//! Sections are a view: they never rename an engine's parameter groups,
//! because a saved preset names each parameter by its group. They may show
//! a parameter that lives on another page (the Drawbars page's percussion
//! switch); every parameter still has exactly one home page
//! ([`crate::pages`]), which is what the wheel walks and the Q-Links name.
//!
//! The engines' own section plans are in [`crate::section_plans`]; any
//! other page (a hosted plugin's, an effect's) gets a section per
//! parameter group, its job guessed from the group's name.

use crate::pages::Page;
use mpfree_node::{Curve, NodeInfo, ParamInfo};
use mpfree_ui::{Color, Theme};

/// What a section does, which sets its colour. The colours are the
/// theme's track colours 6, 5, 8 and 7 (cyan, coral, lime, pink in the
/// default theme) and its dim text, so every theme gets its own; the four
/// Q-Link colours (track colours 1 to 4) are kept for the Q-Link badges.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Job {
    /// Where the sound starts: oscillators, tables, operators, a string,
    /// drawbars. On an effect, its main process.
    Source,
    /// What colours it: the filter, drive, tone, EQ.
    Tone,
    /// How it moves in time: envelopes, decays, dynamics.
    Shape,
    /// What moves it: LFOs, poly-mod, pressure, tremolo, the rotor.
    Motion,
    /// How it is played and where it goes: voices, tuning, output.
    Voice,
}

impl Job {
    /// The default theme's colour for the job, as panels write it.
    pub const fn default_color(self) -> Color {
        match self {
            Job::Source => Color::rgb(0x4DD8E8),
            Job::Tone => Color::rgb(0xFF7A59),
            Job::Shape => Color::rgb(0xA3E05B),
            Job::Motion => Color::rgb(0xF26BA8),
            Job::Voice => Color::rgb(0x98A1B3),
        }
    }

    /// The job's colour in theme `t`: a section's name, border and tint.
    pub fn color(self, t: &Theme) -> Color {
        match self {
            Job::Source => t.track(5),
            Job::Tone => t.track(4),
            Job::Shape => t.track(7),
            Job::Motion => t.track(6),
            Job::Voice => t.text_dim,
        }
    }

    /// What a control's arc, fader or chosen button fills with: the job's
    /// colour, or for Voice a lighter grey than its dim text.
    pub fn fill(self, t: &Theme) -> Color {
        match self {
            Job::Voice => t.text_dim.mix(t.text, 0.55),
            j => j.color(t),
        }
    }

    /// A guess from a group's or a section's name, for pages no engine
    /// planned (a hosted plugin, an effect).
    pub fn guess(name: &str) -> Job {
        let n = name.to_ascii_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
        if has(&[
            "filter", "tone", "eq", "drive", "dist", "satur", "colou", "color", "bit",
        ]) {
            Job::Tone
        } else if has(&[
            "env", "attack", "decay", "release", "dynamic", "comp", "gate", "limit", "adsr",
        ]) {
            Job::Shape
        } else if has(&[
            "lfo", "mod", "pressure", "vibrato", "tremolo", "chorus", "rotary", "phaser",
            "flanger", "motion",
        ]) {
            Job::Motion
        } else if has(&[
            "osc", "wave", "table", "source", "model", "drawbar", "op ", "string", "mallet",
            "pad ", "main", "reverb", "delay", "echo",
        ]) {
            Job::Source
        } else {
            Job::Voice
        }
    }
}

/// How much a control matters, which sets its size: a hero (104 to 120
/// pixels, one or two a page), a main control (72 to 92), a standard one
/// (56 to 64) or a trim (44 to 56).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    Trim,
    Std,
    Main,
    Hero,
}

/// One section of a page, its parameters as indexes into the node's list.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    /// Its name in the page's section layout.
    pub key: String,
    pub name: String,
    pub job: Job,
    /// The controls, in order (a choice or a switch goes above the dials
    /// whatever its place here).
    pub params: Vec<usize>,
    pub hero: Vec<usize>,
    pub main: Vec<usize>,
    pub trim: Vec<usize>,
    /// Shown as faders (an envelope's stages, a mix's levels) or, with
    /// `drawbars`, as an organ's drawbars.
    pub faders: Vec<usize>,
    pub drawbars: bool,
    /// Routing rows: two controls to a line (a slot's target and depth).
    pub rows: bool,
    /// A control's name here when it isn't its parameter's ("Pulse width"
    /// for Osc 1 PW in the Osc 1 section).
    pub labels: Vec<(usize, String)>,
    /// The engine's drawing (a card, by title) shown at the top, and the
    /// one curve of it to show (the filter envelope of a card that draws
    /// both envelopes).
    pub drawing: Option<String>,
    pub trace: Option<usize>,
    /// The page a tap on the section's name opens ("Open ›"), when it
    /// shows part of another page.
    pub link: Option<String>,
    /// A line about the section beside its name ("into the filter").
    pub note: Option<String>,
}

impl Section {
    pub fn new(key: &str, name: &str, job: Job) -> Section {
        Section {
            key: key.to_string(),
            name: name.to_string(),
            job,
            params: Vec::new(),
            hero: Vec::new(),
            main: Vec::new(),
            trim: Vec::new(),
            faders: Vec::new(),
            drawbars: false,
            rows: false,
            labels: Vec::new(),
            drawing: None,
            trace: None,
            link: None,
            note: None,
        }
    }

    /// The tier a parameter shows at: a hero, a main control (named so, or
    /// turned by a Q-Link on this page), a trim, else standard.
    pub fn tier(&self, i: usize, qlinked: bool) -> Tier {
        if self.hero.contains(&i) {
            Tier::Hero
        } else if self.main.contains(&i) || (qlinked && !self.trim.contains(&i)) {
            Tier::Main
        } else if self.trim.contains(&i) {
            Tier::Trim
        } else {
            Tier::Std
        }
    }

    /// A control's name in this section: the plan's, else the parameter's
    /// without the section's name in front ("Osc 1 PW" reads "PW" in
    /// "Osc 1").
    pub fn label(&self, i: usize, p: &ParamInfo) -> String {
        if let Some((_, l)) = self.labels.iter().find(|(k, _)| *k == i) {
            return l.clone();
        }
        let name = p.name;
        let lower = name.to_ascii_lowercase();
        let sec = self.name.to_ascii_lowercase();
        if lower.len() > sec.len() + 1
            && lower.starts_with(&sec)
            && lower.as_bytes()[sec.len()] == b' '
        {
            let rest = &name[sec.len() + 1..];
            let mut c = rest.chars();
            return match c.next() {
                Some(f) => f.to_uppercase().chain(c).collect(),
                None => name.to_string(),
            };
        }
        name.to_string()
    }
}

/// A page's sections and how they lie.
#[derive(Clone, Debug, PartialEq)]
pub struct Arrangement {
    /// The sections' layout, over their keys.
    pub spec: String,
    pub sections: Vec<Section>,
}

impl Arrangement {
    /// Every parameter shown, in the order the sections list them (the
    /// order the wheel picks them in).
    pub fn order(&self) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::new();
        for s in &self.sections {
            for &i in &s.params {
                if !out.contains(&i) {
                    out.push(i);
                }
            }
        }
        out
    }
}

/// The sections of a page: the engine's plan for it, else a section per
/// parameter group. A parameter of the page that the plan leaves out goes
/// in a last section, so nothing is lost.
pub fn arrange(info: &NodeInfo, params: &[ParamInfo], page: &Page) -> Arrangement {
    let mut a = crate::section_plans::plan(info, params, &page.name)
        .unwrap_or_else(|| by_group(params, page));
    // Parameters the node lacks were dropped by the plan; drop empty
    // sections too (a drawing alone keeps one).
    a.sections
        .retain(|s| !s.params.is_empty() || s.drawing.is_some());
    let shown = a.order();
    let left: Vec<usize> = page
        .params
        .iter()
        .copied()
        .filter(|i| !shown.contains(i))
        .collect();
    if !left.is_empty() {
        // One line of knobs for up to six, then a line more for each six,
        // so the plan's own sections keep most of the page.
        let lines = left.len().div_ceil(6) as i32;
        let h = (150 + 110 * (lines - 1)).min(320);
        let mut more = Section::new("more", "More", Job::Voice);
        more.params = left;
        a.spec = format!("col({}:*, more:{h})", a.spec);
        a.sections.push(more);
    }
    a
}

/// A section per run of a group's parameters on the page, rows of up to
/// three sections (or about eight controls) each.
pub fn by_group(params: &[ParamInfo], page: &Page) -> Arrangement {
    let mut sections: Vec<Section> = Vec::new();
    for &i in &page.params {
        let group = params[i].group;
        let name = if group.is_empty() {
            page.name.as_str()
        } else {
            group
        };
        match sections.last_mut() {
            Some(s) if s.name == name => s.params.push(i),
            _ => {
                let key = format!("g{}", sections.len());
                let mut s = Section::new(&key, name, Job::guess(name));
                s.params.push(i);
                sections.push(s);
            }
        }
    }
    for s in &mut sections {
        // An envelope's four stages as faders.
        let stage = |i: &usize| {
            let n = params[*i].name.to_ascii_lowercase();
            ["attack", "decay", "sustain", "release"]
                .iter()
                .any(|w| n == *w || n.ends_with(&format!(" {w}")))
        };
        let stages: Vec<usize> = s.params.iter().copied().filter(stage).collect();
        if stages.len() >= 3 {
            s.faders = stages;
        }
    }
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut count = 0;
    for (k, s) in sections.iter().enumerate() {
        let n = s.params.len();
        match rows.last_mut() {
            Some(r) if r.len() < 3 && count + n <= 8 => {
                r.push(k);
                count += n;
            }
            _ => {
                rows.push(vec![k]);
                count = n;
            }
        }
    }
    let row = |r: &Vec<usize>| {
        let kids: Vec<String> = r
            .iter()
            .map(|&k| format!("{}:{}*", sections[k].key, sections[k].params.len().max(2)))
            .collect();
        format!("row({})", kids.join(", "))
    };
    let spec = match rows.len() {
        0 => "g0".to_string(),
        1 => row(&rows[0]),
        _ => {
            let kids: Vec<String> = rows
                .iter()
                .map(|r| {
                    // A choice takes a line; knobs go four to a line.
                    let lines = r
                        .iter()
                        .map(|&k| {
                            let s = &sections[k];
                            let choices = s
                                .params
                                .iter()
                                .filter(|&&i| matches!(params[i].curve, Curve::Choice(_)))
                                .count();
                            choices + (s.params.len() - choices).div_ceil(4)
                        })
                        .max()
                        .unwrap_or(1);
                    format!("{}:{}*", row(r), lines + 1)
                })
                .collect();
            format!("col({})", kids.join(", "))
        }
    };
    Arrangement { spec, sections }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_are_the_themes_track_colours() {
        let d = Theme::dark();
        for j in [Job::Source, Job::Tone, Job::Shape, Job::Motion, Job::Voice] {
            assert_eq!(j.color(&d), j.default_color(), "{j:?}");
        }
        assert_eq!(Job::guess("Filter env"), Job::Tone);
        assert_eq!(Job::guess("Amp env"), Job::Shape);
        assert_eq!(Job::guess("LFO"), Job::Motion);
        assert_eq!(Job::guess("Osc 2"), Job::Source);
        assert_eq!(Job::guess("Output"), Job::Voice);
    }

    #[test]
    fn every_engines_plan_places_every_parameter_of_its_pages() {
        use mpfree_node::Node;
        use mpfree_synth::*;
        let rate = 48_000;
        let nodes: Vec<Box<dyn Node>> = vec![
            Box::new(Poly::new(rate)),
            Box::new(Fm::new(rate)),
            Box::new(wavetable::Wavetable::new(rate)),
            Box::new(Drums::new(rate)),
            Box::new(Pluck::new(rate)),
            Box::new(Keys::new(rate)),
            Box::new(Drawbar::new(rate)),
            Box::new(Squelch::new(rate)),
            Box::new(Analog::new(rate)),
        ];
        for n in &nodes {
            let (info, params) = (n.info(), n.params());
            for page in crate::pages::plan(info, params) {
                let a = arrange(info, params, &page);
                let lost: Vec<&str> = a
                    .sections
                    .iter()
                    .filter(|s| s.key == "more")
                    .flat_map(|s| s.params.iter().map(|&i| params[i].name))
                    .collect();
                assert!(
                    lost.is_empty(),
                    "{} {}: no section for {lost:?}",
                    info.name,
                    page.name
                );
            }
        }
    }

    #[test]
    fn a_label_drops_the_sections_name() {
        let mut s = Section::new("osc-1", "Osc 1", Job::Source);
        let p = |name: &'static str| ParamInfo {
            name,
            ..mpfree_synth::analog_params::PARAMS[0]
        };
        assert_eq!(s.label(0, &p("Osc 1 PW")), "PW");
        assert_eq!(s.label(0, &p("Osc 1 oct")), "Oct");
        assert_eq!(s.label(0, &p("Osc 10")), "Osc 10");
        assert_eq!(s.label(0, &p("Sync")), "Sync");
        s.labels.push((0, "Pulse width".into()));
        assert_eq!(s.label(0, &p("Osc 1 PW")), "Pulse width");
    }
}
