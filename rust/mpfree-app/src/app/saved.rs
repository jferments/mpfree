//! What the pad player keeps between starts (docs/dev/design/storage-spec.md):
//! the Prefs rows that are settings, the metronome and the Looper's input
//! and level, as the store's "app" settings, and every control found with
//! Map controls or the button guide, as its "map" settings. These are the
//! system's settings, the same whichever project is open. The Synth tab's
//! options and the arp are the song's: they go in the project
//! (project.rs). Line input is never kept: it is off at every start.
//!
//! [`App::restore`] runs once at start, before the sound starts (the audio
//! buffer is one of the settings). After that, Prefs and the mapper mark the
//! state changed, and the audio loop asks [`App::take_saves`] a few times a
//! second; it hands the store only what changed, and the store's own thread
//! writes it.

use super::mapper::{Code, Job, Mark, CONTROLS};
use super::{App, Curve};
use mpfree_arp::LatchMode;
use mpfree_store::Settings;

pub const APP: &str = "app";
pub const MAP: &str = "map";
/// The arp's owner name in the store, and its pattern library file there
/// (the patterns Save keeps and the owner's own scales).
pub const ARP: &str = "arp";
pub const ARP_PATTERNS: &str = "patterns.txt";
/// The Dev tab's Touch page keeps its tuning as these settings
/// (`mpfree_touch::tuning::Tuning::to_pairs`).
pub const TOUCH: &str = "touch";
/// The browser's owner name in the store, and its Recent and Favourites
/// there (docs/dev/design/browser.md).
pub const BROWSER: &str = "browser";
pub const BROWSER_LISTS: &str = "lists.txt";

/// A control's key in the "map" settings: its name in lower case, words
/// joined by "-" ("PAD BANK A/E" is pad-bank-a-e).
pub fn control_key(name: &str) -> String {
    match name {
        "-" => "minus".to_string(),
        "+" => "plus".to_string(),
        _ => name
            .to_lowercase()
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join("-"),
    }
}

fn curve_key(c: Curve) -> &'static str {
    match c {
        Curve::Soft => "soft",
        Curve::Linear => "linear",
        Curve::Hard => "hard",
    }
}

fn code_text(c: Code) -> String {
    match c {
        Code::Button(n) => format!("button {n}"),
        Code::Cc(n) => format!("cc {n}"),
    }
}

fn parse_code(v: &str) -> Option<Code> {
    let (kind, n) = v.split_once(' ')?;
    let n = n.trim().parse::<u8>().ok().filter(|&n| n < 128)?;
    match kind {
        "button" => Some(Code::Button(n)),
        "cc" => Some(Code::Cc(n)),
        _ => None,
    }
}

impl App {
    /// The "app" settings: the Prefs rows that are kept.
    pub fn app_settings(&self) -> Settings {
        let s = &self.settings;
        let mut out = Settings::default();
        out.set_str("pad-curve", curve_key(s.curve));
        out.set_i64("screen-velocity", s.screen_velocity as i64);
        out.set_str(
            "latch-keeps-pads",
            if s.latch_mode == LatchMode::Toggle {
                "toggle"
            } else {
                "replace"
            },
        );
        out.set_i64("audio-buffer", s.periods as i64);
        out.set_bool("developer-tools", s.debug_mode);
        out.set_str("sound-cores", s.sound_cores.name());
        out.set_i64("loop-memory", i64::from(s.loop_memory.minutes()));
        out.set_bool("metronome", s.metronome);
        out.set_i64("metronome-level", i64::from(s.metronome_db));
        out.set_bool("timing-strip", s.timing_strip);
        out.set_str("at-start", if s.open_last { "last" } else { "new" });
        // The Looper's level, and the line input's latency once it was set
        // by hand (measured with a cable). The line input itself, and so
        // what the Looper records from, is off at every start.
        out.set_i64("loop-level", s.loop_level_db.round() as i64);
        if s.loop_latency_set {
            out.set_i64("loop-latency", i64::from(s.loop_latency));
        }
        // A built-in theme or one from the themes folder; a theme file given
        // with --theme is for that run only.
        if self.offers_theme(&s.theme) {
            out.set_str("theme", &s.theme);
        }
        out
    }

    /// The "map" settings: each control found ("button 73", "cc 20") or
    /// skipped, by [`control_key`]. Measured constants (SHIFT, STOP...) are
    /// not kept.
    pub fn map_settings(&self) -> Settings {
        let mut out = Settings::default();
        for (i, c) in CONTROLS.iter().enumerate() {
            let value = match (c.job, self.mapper.marks[i]) {
                (Job::Fixed(_), _) => None,
                (_, Mark::Skipped) => Some("skipped".to_string()),
                (Job::None, Mark::Found(code)) => Some(code_text(code)),
                (Job::None, Mark::Left) => None,
                (job, _) => self.map.of(job).map(|n| code_text(Code::Button(n))),
            };
            if let Some(v) = value {
                out.set_str(&control_key(c.name), &v);
            }
        }
        out
    }

    /// Puts back what was kept. Returns the lines for the log: what was
    /// used, and anything left out.
    pub fn restore(&mut self, app: &Settings, map: &Settings) -> Vec<String> {
        let mut notes = Vec::new();
        let mut skipped = Vec::new();
        let themes: Vec<String> = self.theme_choices().map(|t| t.0.to_string()).collect();
        let s = &mut self.settings;
        for (k, v) in app.iter() {
            let ok = match k {
                "pad-curve" => Curve::ALL
                    .iter()
                    .find(|&&c| curve_key(c) == v)
                    .map(|&c| s.curve = c)
                    .is_some(),
                "screen-velocity" => app
                    .get_i64(k)
                    .filter(|n| (10..=127).contains(n))
                    .map(|n| s.screen_velocity = n as u8)
                    .is_some(),
                "latch-keeps-pads" => match v {
                    "replace" => Some(LatchMode::Replace),
                    "toggle" => Some(LatchMode::Toggle),
                    _ => None,
                }
                .map(|m| s.latch_mode = m)
                .is_some(),
                "audio-buffer" => app
                    .get_i64(k)
                    .filter(|n| (2..=3).contains(n))
                    .map(|n| {
                        s.periods = n as u32;
                        if !s.loop_latency_set {
                            s.loop_latency = crate::looping::estimated_latency(s.periods);
                        }
                    })
                    .is_some(),
                "developer-tools" => app.get_bool(k).map(|b| s.debug_mode = b).is_some(),
                "sound-cores" => crate::cores::SoundCores::parse(v)
                    .map(|m| s.sound_cores = m)
                    .is_some(),
                "loop-memory" => crate::looping::LoopMemory::parse(v)
                    .map(|m| s.loop_memory = m)
                    .is_some(),
                "metronome" => app.get_bool(k).map(|b| s.metronome = b).is_some(),
                "metronome-level" => app
                    .get_i64(k)
                    .and_then(|n| super::METRONOME_DBS.iter().find(|&&d| i64::from(d) == n))
                    .map(|&d| s.metronome_db = d)
                    .is_some(),
                "timing-strip" => app.get_bool(k).map(|b| s.timing_strip = b).is_some(),
                "at-start" => match v {
                    "new" => Some(false),
                    "last" => Some(true),
                    _ => None,
                }
                .map(|b| s.open_last = b)
                .is_some(),
                "loop-level" => app
                    .get_i64(k)
                    .filter(|n| (-24..=6).contains(n))
                    .map(|n| s.loop_level_db = n as f32)
                    .is_some(),
                "loop-latency" => app
                    .get_i64(k)
                    .filter(|n| (0..=4096).contains(n))
                    .map(|n| (s.loop_latency, s.loop_latency_set) = (n as u32, true))
                    .is_some(),
                // Built in, or in the themes folder (listed before restore).
                "theme" => themes
                    .iter()
                    .find(|t| *t == v)
                    .map(|t| s.theme = t.clone())
                    .is_some(),
                _ => false,
            };
            if !ok {
                skipped.push(format!("{k} = {v}"));
            }
        }
        if !app.is_empty() {
            let s = &self.settings;
            let on = |b: bool| if b { "on" } else { "off" };
            notes.push(format!(
                "saved settings: pad curve {}, screen velocity {}, latch {}, audio buffer {} periods, \
                 developer tools {}, theme {}, sound cores {}, loop memory {} min, metronome {} at {} dB, \
                 loop level {:+.0} dB, line input latency {}, at start {}",
                s.curve.label(),
                s.screen_velocity,
                if s.latch_mode == LatchMode::Toggle { "toggle" } else { "replace" },
                s.periods,
                on(s.debug_mode),
                s.theme,
                s.sound_cores.name(),
                s.loop_memory.minutes(),
                on(s.metronome),
                s.metronome_db,
                s.loop_level_db,
                if s.loop_latency_set {
                    format!("{} frames", s.loop_latency)
                } else {
                    "estimated".to_string()
                },
                if s.open_last { "the last project" } else { "a new project" },
            ));
        }
        let (mut found, mut gone) = (0, 0);
        for (k, v) in map.iter() {
            let Some(i) = CONTROLS
                .iter()
                .position(|c| control_key(c.name) == k && !matches!(c.job, Job::Fixed(_)))
            else {
                skipped.push(format!("{k} = {v}"));
                continue;
            };
            let job = CONTROLS[i].job;
            match (v, parse_code(v)) {
                ("skipped", _) => {
                    self.mapper.marks[i] = Mark::Skipped;
                    gone += 1;
                }
                (_, Some(Code::Button(n))) if job != Job::None => {
                    self.map.learn(job, Some(n));
                    self.mapper.marks[i] = Mark::Found(Code::Button(n));
                    found += 1;
                }
                (_, Some(code)) if job == Job::None => {
                    self.mapper.marks[i] = Mark::Found(code);
                    found += 1;
                }
                _ => skipped.push(format!("{k} = {v}")),
            }
        }
        if !map.is_empty() {
            notes.push(format!(
                "saved buttons: {found} found, {gone} skipped ({})",
                self.map.describe()
            ));
        }
        if !skipped.is_empty() {
            notes.push(format!(
                "saved settings left out (not understood): {}",
                skipped.join(", ")
            ));
        }
        // What was just read is what the store holds: nothing to save yet.
        self.saved = [self.app_settings().to_text(), self.map_settings().to_text()];
        self.save_dirty = false;
        notes
    }

    /// True once after a control changed something the project keeps (a
    /// Synth tab option, the arp): the screen counts the work as unsaved.
    /// Load and New aren't edits.
    /// Cheap: the arps are compared by a number, not as text.
    pub fn take_project_edit(&mut self) -> bool {
        let arps = self.arp_fingerprint();
        let arp = std::mem::replace(&mut self.arp_seen, arps) != arps;
        std::mem::take(&mut self.project_edit) || arp
    }

    /// The settings that changed since they were last handed out, if Prefs
    /// or a control changed. Cheap when nothing did.
    pub fn take_saves(&mut self) -> Vec<(&'static str, Settings)> {
        if !self.save_dirty {
            return Vec::new();
        }
        self.save_dirty = false;
        let mut out = Vec::new();
        for (slot, (owner, settings)) in [(APP, self.app_settings()), (MAP, self.map_settings())]
            .into_iter()
            .enumerate()
        {
            let text = settings.to_text();
            if text != self.saved[slot] {
                self.saved[slot] = text;
                out.push((owner, settings));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Item;
    use mpfree_controller::mpc_one::button;

    #[test]
    fn keys_are_plain() {
        assert_eq!(control_key("PAD BANK A/E"), "pad-bank-a-e");
        assert_eq!(control_key("TRACK MIX / PAD MIX"), "track-mix-pad-mix");
        assert_eq!(control_key("-"), "minus");
        assert_eq!(control_key("Other 10"), "other-10");
        for c in CONTROLS.iter() {
            assert!(mpfree_store::valid_key(&control_key(c.name)), "{}", c.name);
        }
        let keys: std::collections::HashSet<String> =
            CONTROLS.iter().map(|c| control_key(c.name)).collect();
        assert_eq!(keys.len(), CONTROLS.len());
    }

    #[test]
    fn prefs_and_buttons_come_back_as_they_were() {
        let mut a = App::new(44_100, false);
        a.restore(&Settings::default(), &Settings::default());
        assert!(
            a.take_saves().is_empty(),
            "nothing changed, nothing to save"
        );
        a.open_tab(crate::app::View::Prefs);
        let row = |a: &App, item: Item| a.items().iter().position(|&i| i == item).unwrap();
        let r = row(&a, Item::Curve);
        a.step_item(r, 1);
        let r = row(&a, Item::Buffer);
        a.step_item(r, -1);
        let r = row(&a, Item::DebugMode);
        a.step_item(r, 1);
        // The metronome, and the Looper's level and measured latency (its
        // tab's rows), are the system's too.
        a.settings.metronome = true;
        let r = row(&a, Item::AtStart);
        a.step_item(r, 1);
        a.looper_setting(Item::LoopLevel, -1);
        a.looper_setting(Item::LoopLevel, -1);
        a.looper_setting(Item::LoopLatency, 1);
        a.map.learn(Job::Rec, Some(73));
        a.mapper.marks[CONTROLS.iter().position(|c| c.name == "COPY").unwrap()] =
            Mark::Found(Code::Button(52));
        a.map.learn(Job::Sampler, Some(56));
        a.mapper.marks[CONTROLS.iter().position(|c| c.name == "TC").unwrap()] = Mark::Skipped;
        a.save_dirty = true;
        let saves = a.take_saves();
        assert_eq!(
            saves.iter().map(|(o, _)| *o).collect::<Vec<_>>(),
            vec![APP, MAP]
        );
        assert!(a.take_saves().is_empty());
        let (app, map) = (saves[0].1.clone(), saves[1].1.clone());
        assert_eq!(app.get_str("audio-buffer"), Some("2"));
        assert_eq!(app.get_bool("developer-tools"), Some(true));
        assert_eq!(app.get_bool("metronome"), Some(true));
        assert_eq!(app.get_str("at-start"), Some("last"));
        assert_eq!(app.get_str("loop-level"), Some("-2"));
        let latency = a.settings.loop_latency;
        assert_eq!(app.get_i64("loop-latency"), Some(i64::from(latency)));
        assert_eq!(
            app.get_str("loop-source"),
            None,
            "the line input is off at start"
        );
        assert_eq!(map.get_str("rec"), Some("button 73"));
        assert_eq!(map.get_str("copy"), Some("button 52"));
        assert_eq!(map.get_str("sampler"), Some("button 56"));
        assert_eq!(map.get_str("tc"), Some("skipped"));
        assert_eq!(
            map.get_str("menu"),
            Some(&*format!("button {}", button::MENU))
        );
        assert_eq!(map.get_str("stop"), None, "measured constants are not kept");

        let mut b = App::new(44_100, false);
        let notes = b.restore(
            &Settings::parse(&app.to_text()),
            &Settings::parse(&map.to_text()),
        );
        assert_eq!(
            (b.settings.curve, b.settings.periods, b.settings.debug_mode),
            (a.settings.curve, 2, true)
        );
        assert_eq!(
            (
                b.settings.metronome,
                b.settings.loop_level_db,
                b.settings.open_last
            ),
            (true, -2.0, true)
        );
        assert_eq!(
            (b.settings.loop_latency, b.settings.loop_latency_set),
            (latency, true)
        );
        assert!(
            b.start().contains(&crate::app::Effect::Looper(
                crate::looping::LoopAction::Command(mpfree_looper::Command::SetLevel(
                    10f32.powf(-2.0 / 20.0)
                ))
            )),
            "the Looper starts at its kept level"
        );
        assert_eq!((b.map.rec, b.map.sampler), (Some(73), Some(56)));
        assert_eq!(b.map_settings(), map);
        assert!(
            notes[0].starts_with("saved settings: pad curve "),
            "{notes:?}"
        );
        assert!(notes[1].starts_with("saved buttons: "), "{notes:?}");
        assert!(!b.settings.line_input, "line input is never kept");
        assert!(b.take_saves().is_empty());
    }

    #[test]
    fn what_is_not_understood_is_left_out() {
        let mut a = App::new(44_100, false);
        let notes = a.restore(
            &Settings::parse("audio-buffer = 9\npad-curve = loud\nline-input = true\n"),
            &Settings::parse("rec = cc 300\nstop = button 1\nnowhere = button 3\n"),
        );
        assert_eq!(a.settings.periods, 3);
        assert!(!a.settings.line_input);
        assert_eq!(a.map.rec, None);
        let last = notes.last().unwrap();
        for bad in [
            "audio-buffer = 9",
            "pad-curve = loud",
            "line-input = true",
            "rec = cc 300",
            "stop = button 1",
            "nowhere = button 3",
        ] {
            assert!(last.contains(bad), "{last}");
        }
    }
}
