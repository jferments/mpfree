//! The effect picker's data, so the screen only draws: its sections
//! (Favourites, Recent, then each [`Category`] that has something to
//! offer), each effect's presets, and the favourites and recent effects
//! the app keeps in the store as `files/fx/favourites.txt` and
//! `files/fx/recent.txt` (docs/dev/design/storage-spec.md).
//!
//! Both files are one effect id per line, the order they show in
//! (favourites in the order they were starred, recents newest first).
//! Ids the build doesn't know are kept, so a plugin that is missing today
//! is still a favourite when it comes back; a picker simply doesn't show
//! them.

use crate::registry::{self, Category};
use crate::Preset;

/// The store's owner and file names.
pub const OWNER: &str = "fx";
pub const FAVOURITES: &str = "favourites.txt";
pub const RECENT: &str = "recent.txt";
/// How many recent effects are kept, and favourites at most.
pub const MAX_RECENT: usize = 12;
pub const MAX_FAVOURITES: usize = 64;
/// The longest id kept (a plugin's `clap:` id is the longest kind).
const MAX_ID: usize = 160;

/// One heading down the picker's left side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Favourites,
    Recent,
    Category(Category),
}

impl Section {
    pub fn name(self) -> &'static str {
        match self {
            Section::Favourites => "Favourites",
            Section::Recent => "Recent",
            Section::Category(c) => c.name(),
        }
    }
}

/// The picker's sections for a strip whose offer has effects in
/// `categories` (any order, repeats allowed): Favourites and Recent, then
/// every category with at least one effect, in [`Category::ALL`]'s order.
pub fn sections(categories: impl IntoIterator<Item = Category>) -> Vec<Section> {
    let have: Vec<Category> = categories.into_iter().collect();
    let mut s = vec![Section::Favourites, Section::Recent];
    s.extend(
        Category::ALL
            .iter()
            .filter(|c| have.contains(c))
            .map(|&c| Section::Category(c)),
    );
    s
}

/// The effects a section shows, as indexes into the strip's `offered`
/// effects (id and category, in the picker's order): a category's in
/// that order, Favourites and Recent in the shelf's order, leaving out
/// what the strip doesn't offer.
pub fn members(section: Section, offered: &[(&str, Category)], shelf: &Shelf) -> Vec<usize> {
    let from = |ids: &[String]| -> Vec<usize> {
        ids.iter()
            .filter_map(|id| offered.iter().position(|(o, _)| o == id))
            .collect()
    };
    match section {
        Section::Favourites => from(&shelf.favourites),
        Section::Recent => from(&shelf.recent),
        Section::Category(c) => (0..offered.len()).filter(|&i| offered[i].1 == c).collect(),
    }
}

/// How many effects each of `sections` shows ([`members`]).
pub fn counts(sections: &[Section], offered: &[(&str, Category)], shelf: &Shelf) -> Vec<usize> {
    sections
        .iter()
        .map(|&s| members(s, offered, shelf).len())
        .collect()
}

/// The section a picker opens on when none was picked before (or the one
/// picked has gone): Favourites when the strip offers one of them, else
/// the first category.
pub fn first_section(sections: &[Section], offered: &[(&str, Category)], shelf: &Shelf) -> Section {
    if !members(Section::Favourites, offered, shelf).is_empty() {
        return Section::Favourites;
    }
    sections
        .iter()
        .copied()
        .find(|s| matches!(s, Section::Category(_)))
        .unwrap_or(Section::Favourites)
}

/// An effect's factory presets, for the picker's presets list (none for a
/// plugin or an id the registry doesn't know).
pub fn presets(id: &str) -> &'static [Preset] {
    registry::entry(id).map_or(&[], |e| e.presets)
}

/// The favourite and recent effects.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shelf {
    pub favourites: Vec<String>,
    pub recent: Vec<String>,
}

impl Shelf {
    /// From the two files' text (either may be empty or missing).
    pub fn read(favourites: &str, recent: &str) -> Shelf {
        Shelf {
            favourites: ids(favourites, MAX_FAVOURITES),
            recent: ids(recent, MAX_RECENT),
        }
    }

    pub fn favourites_text(&self) -> String {
        text(&self.favourites)
    }

    pub fn recent_text(&self) -> String {
        text(&self.recent)
    }

    pub fn is_favourite(&self, id: &str) -> bool {
        self.favourites.iter().any(|f| f == id)
    }

    /// Stars or unstars `id`; true if that changed anything. A new
    /// favourite goes last; past [`MAX_FAVOURITES`] nothing more is
    /// starred.
    pub fn set_favourite(&mut self, id: &str, on: bool) -> bool {
        if !valid(id) {
            return false;
        }
        let at = self.favourites.iter().position(|f| f == id);
        match (on, at) {
            (true, None) if self.favourites.len() < MAX_FAVOURITES => {
                self.favourites.push(id.to_string());
                true
            }
            (false, Some(i)) => {
                self.favourites.remove(i);
                true
            }
            _ => false,
        }
    }

    /// `id` was just added to a chain: it goes to the front of Recent.
    /// True if the list changed.
    pub fn used(&mut self, id: &str) -> bool {
        if !valid(id) || self.recent.first().is_some_and(|r| r == id) {
            return false;
        }
        self.recent.retain(|r| r != id);
        self.recent.insert(0, id.to_string());
        self.recent.truncate(MAX_RECENT);
        true
    }
}

/// An id is kept if it is short and plain: no spaces, no control
/// characters, nothing a line-based file can't hold.
fn valid(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-/+".contains(&b))
}

fn ids(text: &str, most: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let id = line.trim();
        if id.starts_with('#') || !valid(id) || out.iter().any(|o| o == id) {
            continue;
        }
        out.push(id.to_string());
        if out.len() == most {
            break;
        }
    }
    out
}

fn text(ids: &[String]) -> String {
    let mut s = String::new();
    for id in ids {
        s.push_str(id);
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_follow_the_offer() {
        let s = sections([
            Category::DelayReverb,
            Category::Dynamics,
            Category::Dynamics,
        ]);
        assert_eq!(
            s,
            vec![
                Section::Favourites,
                Section::Recent,
                Section::Category(Category::Dynamics),
                Section::Category(Category::DelayReverb),
            ]
        );
        assert_eq!(s[3].name(), "Delay and reverb");
    }

    #[test]
    fn counts_only_what_the_strip_offers() {
        let offered = [
            ("mpfree.fx.delay", Category::DelayReverb),
            ("mpfree.fx.room", Category::DelayReverb),
            ("mpfree.fx.gate", Category::Dynamics),
        ];
        let mut shelf = Shelf::default();
        shelf.set_favourite("mpfree.fx.room", true);
        shelf.set_favourite("mpfree.fx.master", true);
        shelf.used("mpfree.fx.gate");
        let s = sections(offered.iter().map(|o| o.1));
        assert_eq!(counts(&s, &offered, &shelf), vec![1, 1, 1, 2]);
        assert_eq!(members(Section::Favourites, &offered, &shelf), vec![1]);
        assert_eq!(
            members(Section::Category(Category::DelayReverb), &offered, &shelf),
            vec![0, 1]
        );
        assert_eq!(first_section(&s, &offered, &shelf), Section::Favourites);
        let none = Shelf::default();
        assert_eq!(
            first_section(&s, &offered, &none),
            Section::Category(Category::Dynamics)
        );
    }

    #[test]
    fn the_shelf_round_trips_and_keeps_order() {
        let mut s = Shelf::default();
        assert!(s.set_favourite("mpfree.fx.plate", true));
        assert!(s.set_favourite("clap:com.example.verb", true));
        assert!(!s.set_favourite("mpfree.fx.plate", true));
        assert!(!s.set_favourite("has space", true));
        for id in ["mpfree.fx.delay", "mpfree.fx.room", "mpfree.fx.delay"] {
            s.used(id);
        }
        assert_eq!(s.recent, ["mpfree.fx.delay", "mpfree.fx.room"]);
        assert!(!s.used("mpfree.fx.delay"));
        let back = Shelf::read(&s.favourites_text(), &s.recent_text());
        assert_eq!(back, s);
        assert!(s.set_favourite("mpfree.fx.plate", false));
        assert_eq!(s.favourites, ["clap:com.example.verb"]);
    }

    #[test]
    fn reading_skips_junk_and_caps_the_lists() {
        let mut long = String::from("# favourites\n\n  mpfree.fx.gate  \nmpfree.fx.gate\n");
        long.push_str(&"x".repeat(500));
        long.push('\n');
        for i in 0..100 {
            long.push_str(&format!("clap:plugin.{i}\n"));
        }
        let s = Shelf::read(&long, &long);
        assert_eq!(s.favourites[0], "mpfree.fx.gate");
        assert_eq!(s.favourites.len(), MAX_FAVOURITES);
        assert_eq!(s.recent.len(), MAX_RECENT);
        let mut r = Shelf::read("", &long);
        for i in 0..20 {
            r.used(&format!("mpfree.fx.e{i}"));
        }
        assert_eq!(r.recent.len(), MAX_RECENT);
        assert_eq!(r.recent[0], "mpfree.fx.e19");
    }

    #[test]
    fn presets_come_from_the_registry() {
        assert!(!presets("mpfree.fx.plate").is_empty());
        assert!(presets("clap:com.example.verb").is_empty());
    }
}
