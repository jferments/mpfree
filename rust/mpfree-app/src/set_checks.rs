//! The app's half of the set check (live shows B2, layout spec 4p): the
//! lines only the app can see in a song, run on the library's worker
//! (`mpfree_library::preflight::AppChecks`). The browser never reads a
//! part, so the app looks at the parts the library hands it:
//!
//! - **Plugins:** every plugin a part names (`clap:` or `mpc:`, with the
//!   `file=` its line names), each looked up in the rack the app opens
//!   projects with. One that isn't on this stick is NOT READY, named by its
//!   file (or its id), as an open would say it.
//! - **Outputs:** the stereo mix on Main out. The MPC One has only Out 1+2
//!   (the mixer calls the master "Out 1+2"), and the headphones carry the
//!   same mix, so every song's outputs exist here.
//!
//! It also counts the plugin processes a song starts, for the open time's
//! estimate.

use std::sync::Arc;

use mpfree_library::preflight::{AppChecks, AppSong, Area, CheckLine, Mark};
use mpfree_library::Snapshot;

use crate::plugins::Plugins;

/// The most plugins a Plugins line names one by one; the rest are counted.
const NAMED: usize = 3;

/// The app's checks, holding the rack an open would use.
pub struct SetChecks {
    plugins: Arc<Plugins>,
}

impl SetChecks {
    /// The checks with the rack `plugins` (the app's, from `ToUi::Plugins`).
    pub fn shared(plugins: Arc<Plugins>) -> Arc<dyn AppChecks> {
        Arc::new(SetChecks { plugins })
    }
}

/// Each plugin `snap`'s parts name, once, in the order found: its id and
/// the file its line names (`file=`), if any.
pub fn plugins_named(snap: &Snapshot) -> Vec<(String, Option<String>)> {
    let mut out: Vec<(String, Option<String>)> = Vec::new();
    for (_, text) in &snap.parts {
        for line in text.lines() {
            let Some(id) = line.split(' ').find(|w| Plugins::is_plugin(w)) else {
                continue;
            };
            let file = line
                .split(' ')
                .find_map(|w| w.strip_prefix("file="))
                .filter(|f| !f.is_empty())
                .map(str::to_string);
            match out.iter_mut().find(|(i, _)| i == id) {
                Some(seen) if seen.1.is_none() => seen.1 = file,
                Some(_) => {}
                None => out.push((id.to_string(), file)),
            }
        }
    }
    out
}

/// The Plugins and Outputs lines for the plugins a song names, `on` saying
/// whether this stick has one.
pub fn lines_for(named: &[(String, Option<String>)], on: &dyn Fn(&str) -> bool) -> Vec<CheckLine> {
    let line = |mark, w: String| CheckLine::new(Area::Plugins, mark, w);
    let mut out = Vec::new();
    if named.is_empty() {
        out.push(line(Mark::Fine, "it uses none".into()));
    } else {
        let missing: Vec<&(String, Option<String>)> =
            named.iter().filter(|(id, _)| !on(id)).collect();
        for (id, file) in missing.iter().take(NAMED) {
            out.push(line(
                Mark::NotReady,
                match file {
                    Some(f) => format!("{f} isn\u{2019}t in files/plugins/ on this stick"),
                    None => format!("{id} isn\u{2019}t on this stick"),
                },
            ));
        }
        if missing.len() > NAMED {
            out.push(line(
                Mark::NotReady,
                match missing.len() - NAMED {
                    1 => "and 1 more plugin".to_string(),
                    n => format!("and {n} more plugins"),
                },
            ));
        }
        let there = named.len() - missing.len();
        if there > 0 {
            out.push(line(
                Mark::Fine,
                match there {
                    1 => "1 plugin installed".to_string(),
                    n => format!("{n} plugins installed"),
                },
            ));
        }
    }
    out.push(CheckLine::new(
        Area::Outputs,
        Mark::Fine,
        "the stereo mix on Main out, as this MPC has it",
    ));
    out
}

impl AppChecks for SetChecks {
    fn song(&self, snap: &Snapshot) -> AppSong {
        let named = plugins_named(snap);
        let on = |id: &str| self.plugins.find(id).is_some();
        AppSong {
            lines: lines_for(&named, &on),
            plugins: named.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_library::Meta;

    fn snap(parts: &[(&str, &str)]) -> Snapshot {
        Snapshot {
            meta: Meta::default(),
            parts: parts
                .iter()
                .map(|(n, t)| (n.to_string(), t.to_string()))
                .collect(),
        }
    }

    fn words(lines: &[CheckLine]) -> Vec<(Area, Mark, &str)> {
        lines
            .iter()
            .map(|l| (l.area, l.mark, l.words.as_str()))
            .collect()
    }

    /// Every plugin the parts name is found once, with its file; built-in
    /// effects and instruments aren't plugins.
    #[test]
    fn the_plugins_a_song_names_are_found_in_its_parts() {
        let s = snap(&[
            (
                "mixer",
                "mpfree-mixer 1\ninsert t0 1 1 mpfree.fx.drive\ninsert master 4 1 clap:com.example.verb file=verb.clap key=t1\nstate AAEC\n",
            ),
            (
                "tracks",
                "track 4 clap:org.example.synth file=synth.clap\nfree 9 mpc:abcd\ninsert t1 2 1 clap:com.example.verb\n",
            ),
        ]);
        assert_eq!(
            plugins_named(&s),
            [
                (
                    "clap:com.example.verb".to_string(),
                    Some("verb.clap".to_string())
                ),
                ("clap:org.example.synth".into(), Some("synth.clap".into())),
                ("mpc:abcd".into(), None),
            ]
        );
        assert!(plugins_named(&snap(&[("mixer", "insert t0 1 1 mpfree.fx.eq\n")])).is_empty());
    }

    /// A plugin this stick hasn't got stops the song, named by its file;
    /// the ones there are counted; Outputs is always Main out.
    #[test]
    fn a_missing_plugin_is_not_ready_and_named_by_its_file() {
        let named = vec![
            ("clap:a".to_string(), Some("a.clap".to_string())),
            ("clap:b".to_string(), None),
            ("clap:c".to_string(), Some("c.clap".to_string())),
        ];
        let lines = lines_for(&named, &|id: &str| id == "clap:c");
        assert_eq!(
            words(&lines),
            [
                (
                    Area::Plugins,
                    Mark::NotReady,
                    "a.clap isn\u{2019}t in files/plugins/ on this stick"
                ),
                (
                    Area::Plugins,
                    Mark::NotReady,
                    "clap:b isn\u{2019}t on this stick"
                ),
                (Area::Plugins, Mark::Fine, "1 plugin installed"),
                (
                    Area::Outputs,
                    Mark::Fine,
                    "the stereo mix on Main out, as this MPC has it"
                ),
            ]
        );
        let none = lines_for(&[], &|_: &str| false);
        assert_eq!(words(&none)[0], (Area::Plugins, Mark::Fine, "it uses none"));
        let many: Vec<(String, Option<String>)> =
            (0..5).map(|i| (format!("clap:p{i}"), None)).collect();
        let lines = lines_for(&many, &|_: &str| false);
        assert_eq!(lines.len(), 5, "{lines:#?}");
        assert_eq!(lines[3].words, "and 2 more plugins");
    }

    /// The app's checks with no plugins found: a song naming one is not
    /// ready, and the count feeds the open time's estimate.
    #[test]
    fn the_checks_use_the_rack_an_open_would_use() {
        let checks = SetChecks::shared(Arc::new(Plugins::none()));
        let a = checks.song(&snap(&[(
            "tracks",
            "track 0 clap:org.example.synth file=synth.clap\n",
        )]));
        assert_eq!(a.plugins, 1);
        assert_eq!(a.lines[0].mark, Mark::NotReady);
        assert_eq!(
            a.lines[0].words,
            "synth.clap isn\u{2019}t in files/plugins/ on this stick"
        );
    }
}
