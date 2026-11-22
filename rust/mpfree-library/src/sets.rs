//! Set lists (live shows B1): a set is a list of songs in order, each a
//! project in Projects, kept as `Sets/<name>.set`. A set points at its
//! songs and never copies them, so one song can be in many sets. Each song
//! is kept by its lasting id ([`crate::Meta::id`]) and the name of its
//! folder at the last save, so a renamed or moved song is still found
//! ([`Library::find_song`]); a song saved by an older build, with no id, is
//! found by its name alone.
//!
//! ```text
//! mpfree-set 1
//! song = <32 hex digits, or -> <name in Projects>
//! starts = stopped | playing | count-in
//! ends = finish | fade <seconds> | cut
//! needs = <a keyboard or control's name>
//! nice = <one it can do without>
//! notes = <text, with \n for a new line and \\ for a backslash>
//! ```
//!
//! - **Saved whole.** [`Library::save_set`] writes `<name>.set.part`,
//!   makes it durable and renames it over the set, so a power cut leaves
//!   the set as it was or as it is now, never half of each.
//! - **Lines it doesn't know are kept,** with the song they follow, so a
//!   set saved by a newer build keeps what this one can't use.
//! - **The check** ([`Library::check_song`]) is the open's quick check
//!   (live shows A1): the project reads, every file its manifest lists is
//!   there and unchanged, and the keyboards and controls it needs are
//!   connected.

use crate::project::{write_synced, PARTS};
use crate::{clean_name, unique_name, Entry, Library, Loc, Place};
use std::fs;
use std::io;

/// The first line of a set's file.
pub const SET_FORMAT: &str = "mpfree-set 1";
/// A set's file ends with this.
pub const SET_EXT: &str = ".set";
/// The most songs a set holds.
pub const MAX_SONGS: usize = 200;
/// The most characters a song's notes hold.
pub const MAX_NOTES: usize = 240;
/// The most a set's file is read for.
const MAX_SET_BYTES: u64 = 256 * 1024;
/// A fade's length, in seconds: the default and the range.
pub const FADE_DEFAULT: u8 = 4;
pub const FADE_MIN: u8 = 1;
pub const FADE_MAX: u8 = 30;

/// How a song starts once it is in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Starts {
    /// Loaded, waiting for PLAY.
    #[default]
    Stopped,
    /// Playing as soon as it is in.
    Playing,
    /// Playing after its count-in.
    CountIn,
}

impl Starts {
    pub const ALL: [Starts; 3] = [Starts::Stopped, Starts::Playing, Starts::CountIn];

    fn word(self) -> &'static str {
        match self {
            Starts::Stopped => "stopped",
            Starts::Playing => "playing",
            Starts::CountIn => "count-in",
        }
    }

    /// As the Set editor shows it.
    pub fn label(self) -> &'static str {
        match self {
            Starts::Stopped => "Stopped",
            Starts::Playing => "Playing",
            Starts::CountIn => "Count-in then play",
        }
    }
}

/// How the song before ends when this one goes in its place.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ends {
    /// No new notes; held notes and tails ring out until it is quiet, 10 s
    /// at most.
    #[default]
    Finish,
    /// Under a fade of this many seconds.
    Fade(u8),
    /// A 10 ms fade.
    Cut,
}

impl Ends {
    /// As the Set editor shows it.
    pub fn label(self) -> String {
        match self {
            Ends::Finish => "Let it finish".into(),
            Ends::Fade(s) => format!("Fade {s} s"),
            Ends::Cut => "Cut".into(),
        }
    }
}

/// A keyboard or learned control a song needs, by its name in the rig.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Need {
    pub name: String,
    /// False: the song can do without it (it only limits the song).
    pub needed: bool,
}

/// One song in a set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetSong {
    /// Its project's lasting id, or empty for a project saved before ids.
    pub id: String,
    /// Its project's folder in Projects when the set was last saved.
    pub name: String,
    pub starts: Starts,
    pub ends: Ends,
    pub needs: Vec<Need>,
    /// Shown big while it plays: key, cues, who starts. At most
    /// [`MAX_NOTES`] characters.
    pub notes: String,
    /// Lines this build doesn't know, kept as they were.
    pub extra: Vec<String>,
}

impl SetSong {
    /// A song for the project in `Projects/<name>` with lasting id `id`,
    /// set as a song is by default.
    pub fn new(id: &str, name: &str) -> SetSong {
        SetSong {
            id: if crate::project::valid_id(id) {
                id.to_string()
            } else {
                String::new()
            },
            name: name.to_string(),
            ..SetSong::default()
        }
    }

    /// Notes cut to [`MAX_NOTES`] characters.
    pub fn set_notes(&mut self, notes: &str) {
        self.notes = notes.chars().take(MAX_NOTES).collect();
    }
}

/// A set: its songs, in the order they play.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Set {
    pub songs: Vec<SetSong>,
    /// Lines before the first song that this build doesn't know.
    pub extra: Vec<String>,
}

impl Set {
    pub fn to_text(&self) -> String {
        let mut s = String::from(SET_FORMAT);
        s.push('\n');
        for l in &self.extra {
            s.push_str(l);
            s.push('\n');
        }
        for song in &self.songs {
            let id = if song.id.is_empty() { "-" } else { &song.id };
            s.push_str(&format!("song = {id} {}\n", one_line(&song.name)));
            if song.starts != Starts::Stopped {
                s.push_str(&format!("starts = {}\n", song.starts.word()));
            }
            match song.ends {
                Ends::Finish => {}
                Ends::Fade(sec) => s.push_str(&format!("ends = fade {sec}\n")),
                Ends::Cut => s.push_str("ends = cut\n"),
            }
            for n in &song.needs {
                let key = if n.needed { "needs" } else { "nice" };
                s.push_str(&format!("{key} = {}\n", one_line(&n.name)));
            }
            if !song.notes.is_empty() {
                s.push_str(&format!("notes = {}\n", escape(&song.notes)));
            }
            for l in &song.extra {
                s.push_str(l);
                s.push('\n');
            }
        }
        s
    }

    /// Reads a set's text. Refuses a file that isn't a set or holds more
    /// than [`MAX_SONGS`] songs; a value it can't read is left at its
    /// default, and a song line with no name is left out.
    pub fn parse(text: &str) -> io::Result<Set> {
        let mut lines = text.lines();
        if lines.next().map(str::trim) != Some(SET_FORMAT) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "it isn't a set list",
            ));
        }
        let mut set = Set::default();
        // Lines after a song line that has no name belong to no song.
        let mut skipping = false;
        for line in lines {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            if key == "song" {
                let (id, name) = value.split_once(' ').unwrap_or((value, ""));
                let name = name.trim();
                skipping = name.is_empty() || !crate::valid_name(name);
                if skipping {
                    continue;
                }
                if set.songs.len() == MAX_SONGS {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("it holds more than {MAX_SONGS} songs"),
                    ));
                }
                set.songs.push(SetSong::new(&id.to_ascii_lowercase(), name));
                continue;
            }
            if skipping {
                continue;
            }
            let Some(song) = set.songs.last_mut() else {
                set.extra.push(line.to_string());
                continue;
            };
            match key {
                "starts" => {
                    song.starts = Starts::ALL
                        .into_iter()
                        .find(|s| s.word() == value)
                        .unwrap_or_default()
                }
                "ends" => {
                    song.ends = match value.split_once(' ') {
                        None if value == "cut" => Ends::Cut,
                        Some(("fade", s)) => Ends::Fade(
                            s.trim()
                                .parse::<u8>()
                                .map_or(FADE_DEFAULT, |s| s.clamp(FADE_MIN, FADE_MAX)),
                        ),
                        _ => Ends::Finish,
                    }
                }
                "needs" | "nice" if !value.is_empty() => song.needs.push(Need {
                    name: value.to_string(),
                    needed: key == "needs",
                }),
                "notes" => song.set_notes(&unescape(value)),
                _ => song.extra.push(line.to_string()),
            }
        }
        Ok(set)
    }
}

/// A value on one line: line breaks become spaces.
fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some(c) => out.push(c),
            None => out.push('\\'),
        }
    }
    out
}

/// Where a set's song is now ([`Library::find_song`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// In the folder the set names.
    At(Loc),
    /// Renamed or moved: here now. The set gets the new name when it is
    /// next saved.
    Moved(Loc),
    /// No project has its id (deleted, or in the Trash), or, for a song
    /// with no id, nothing is in its folder.
    Gone,
}

impl Found {
    pub fn loc(&self) -> Option<&Loc> {
        match self {
            Found::At(l) | Found::Moved(l) => Some(l),
            Found::Gone => None,
        }
    }
}

/// What the quick check says of one song ([`Library::check_song`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SongCheck {
    /// What stops it playing: a missing or changed file, a project that
    /// doesn't read or isn't there, a needed keyboard not connected.
    pub not_ready: Vec<String>,
    /// What it plays without: a keyboard it can do without, not connected.
    pub limits: Vec<String>,
    /// It has no list of its files yet, so they weren't checked (saving it
    /// makes one).
    pub unchecked: bool,
}

impl SongCheck {
    pub fn ready(&self) -> bool {
        self.not_ready.is_empty()
    }

    /// The chip the Set editor shows.
    pub fn word(&self) -> &'static str {
        if !self.not_ready.is_empty() {
            "NOT READY"
        } else if !self.limits.is_empty() {
            "LIMITS"
        } else if self.unchecked {
            "NOT CHECKED"
        } else {
            "READY"
        }
    }
}

impl Library {
    fn sets_top() -> Loc {
        Loc::top(Place::Sets)
    }

    /// The sets, in natural order of their names.
    pub fn sets(&self) -> io::Result<Vec<Entry>> {
        let stem = |e: &Entry| e.name.strip_suffix(SET_EXT).unwrap_or(&e.name).to_string();
        let mut sets: Vec<Entry> = self
            .list(&Self::sets_top())?
            .into_iter()
            .filter(|e| !e.kind.is_folder() && e.name.ends_with(SET_EXT))
            .collect();
        sets.sort_by(|a, b| crate::natural_cmp(&stem(a), &stem(b)));
        Ok(sets)
    }

    /// Reads a set.
    pub fn load_set(&self, set: &Loc) -> io::Result<Set> {
        let bytes = self.read(set, MAX_SET_BYTES)?;
        let text = String::from_utf8(bytes)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "it isn't a set list"))?;
        Set::parse(&text)
    }

    /// A new empty set named `name` (cleaned; "Set" if nothing is left),
    /// beside the others under a name not taken ("Friday 2").
    pub fn new_set(&self, name: &str) -> io::Result<Loc> {
        let base = clean_name(name, "Set");
        let base = base.strip_suffix(SET_EXT).unwrap_or(&base).to_string();
        let top = Self::sets_top();
        let taken = |n: &str| {
            top.join(&format!("{n}{SET_EXT}"))
                .is_ok_and(|l| self.exists(&l))
        };
        let name = unique_name(&base, taken);
        let loc = top.join(&format!("{name}{SET_EXT}"))?;
        self.save_set(&loc, &Set::default())?;
        Ok(loc)
    }

    /// Writes `set` over the set at `loc`, whole: a new file renamed into
    /// place, so a power cut leaves the set as it was.
    pub fn save_set(&self, loc: &Loc, set: &Set) -> io::Result<()> {
        if loc.place != Place::Sets || !loc.name().ends_with(SET_EXT) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} isn't a set", loc.display()),
            ));
        }
        let path = self.path(loc)?;
        let part = path.with_file_name(format!("{}.part", loc.name()));
        write_synced(&part, set.to_text().as_bytes())?;
        fs::rename(&part, &path)?;
        if let Some(dir) = path.parent() {
            crate::sync_dir(dir)?;
        }
        Ok(())
    }

    /// Where a set's song is now: in the folder the set names if the
    /// project there has its id, else wherever its id is now (renamed or
    /// moved; [`Library::find_project`]). A song with no id is found by
    /// its folder alone.
    pub fn find_song(&self, song: &SetSong) -> Found {
        let at = Loc::top(Place::Projects).join(&song.name).ok();
        if song.id.is_empty() {
            return match at {
                Some(l) if self.project_meta(&l).is_ok() => Found::At(l),
                _ => Found::Gone,
            };
        }
        match self.find_project(&song.id, at.as_ref()) {
            Some(l) if Some(&l) == at.as_ref() => Found::At(l),
            Some(l) => Found::Moved(l),
            None => Found::Gone,
        }
    }

    /// The quick check of one song: where it is, whether its project reads,
    /// whether every file its manifest lists is there and unchanged, and
    /// whether the keyboards and controls it needs are connected
    /// (`connected` says, by their names in the rig).
    pub fn check_song(
        &self,
        song: &SetSong,
        connected: &dyn Fn(&str) -> bool,
    ) -> (Found, SongCheck) {
        let found = self.find_song(song);
        let mut c = SongCheck::default();
        for n in &song.needs {
            if !connected(&n.name) {
                let why = format!("{} isn\u{2019}t connected", n.name);
                if n.needed {
                    c.not_ready.push(why);
                } else {
                    c.limits.push(why);
                }
            }
        }
        let Some(loc) = found.loc() else {
            c.not_ready.insert(
                0,
                format!("the project {} isn\u{2019}t in Projects", song.name),
            );
            return (found, c);
        };
        if let Err(e) = self.open_project(loc) {
            c.not_ready
                .insert(0, format!("it doesn\u{2019}t read: {e}"));
            return (found, c);
        }
        match self.check_project(loc, false) {
            Ok(check) if check.no_manifest => c.unchecked = true,
            Ok(check) => {
                let (missing, changed) = check.gaps();
                let mut gaps: Vec<String> = missing
                    .iter()
                    .map(|p| format!("the file {p} is missing"))
                    .chain(changed.iter().map(|p| format!("the file {p} changed")))
                    .collect();
                // A part that changed reads differently from the song that
                // was checked; it still opens, so it only limits.
                let parts = check
                    .changed
                    .iter()
                    .filter(|p| p.starts_with(&format!("{PARTS}/")))
                    .count();
                if parts > 0 {
                    c.limits.push(if parts == 1 {
                        "a part changed since its last save".to_string()
                    } else {
                        format!("{parts} parts changed since its last save")
                    });
                }
                gaps.append(&mut c.not_ready);
                c.not_ready = gaps;
            }
            Err(e) => c
                .not_ready
                .insert(0, format!("its files couldn\u{2019}t be checked: {e}")),
        }
        (found, c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Meta, Snapshot};
    use crate::tests::temp;

    fn song_snap(name: &str) -> Snapshot {
        Snapshot {
            meta: Meta {
                name: name.into(),
                saved: 1,
                ..Meta::default()
            },
            parts: vec![("clips".into(), "clip a\n".into())],
        }
    }

    #[test]
    fn a_set_reads_back_with_its_settings_and_keeps_lines_it_doesnt_know() {
        let mut a = SetSong::new("0123456789abcdef0123456789abcdef", "Night drive");
        a.starts = Starts::CountIn;
        a.ends = Ends::Fade(6);
        a.needs = vec![
            Need {
                name: "Keystep 37".into(),
                needed: true,
            },
            Need {
                name: "FS-6".into(),
                needed: false,
            },
        ];
        a.set_notes("F minor.\nSam starts \\ the Rhodes.");
        let b = SetSong::new("", "Old song");
        let set = Set {
            songs: vec![a, b],
            extra: vec![],
        };
        let text = set.to_text();
        assert_eq!(Set::parse(&text).unwrap(), set);
        assert!(text.contains("song = - Old song\n"), "{text}");

        let newer = format!("{text}colour = 3\n");
        let back = Set::parse(&newer).unwrap();
        assert_eq!(back.songs[1].extra, vec!["colour = 3".to_string()]);
        assert_eq!(back.to_text(), newer);

        // Values it can't read fall back; a song with no name is left out
        // with its lines; a bad id is no id.
        let odd = "mpfree-set 1\nsong = 0123\nstarts = upside down\nsong = XYZ Song\nends = fade 99\nstarts = sideways\n";
        let back = Set::parse(odd).unwrap();
        assert_eq!(back.songs.len(), 1);
        assert_eq!(back.songs[0].id, "");
        assert_eq!(back.songs[0].name, "Song");
        assert_eq!(back.songs[0].ends, Ends::Fade(FADE_MAX));
        assert_eq!(back.songs[0].starts, Starts::Stopped);

        assert!(Set::parse("mpfree-manifest 1\n").is_err());
        let many = format!("{SET_FORMAT}\n{}", "song = - A\n".repeat(MAX_SONGS + 1));
        assert!(Set::parse(&many).is_err());
        // Notes are cut to their limit.
        let mut s = SetSong::default();
        s.set_notes(&"é".repeat(MAX_NOTES + 9));
        assert_eq!(s.notes.chars().count(), MAX_NOTES);
    }

    #[test]
    fn a_set_saves_whole_and_a_new_one_takes_a_free_name() {
        let lib = Library::at(&temp()).unwrap();
        let loc = lib.new_set("Friday").unwrap();
        assert_eq!(loc.name(), "Friday.set");
        assert_eq!(lib.new_set("Friday.set").unwrap().name(), "Friday 2.set");
        let set = Set {
            songs: vec![SetSong::new("", "Night drive")],
            extra: vec![],
        };
        lib.save_set(&loc, &set).unwrap();
        assert_eq!(lib.load_set(&loc).unwrap(), set);
        let names: Vec<String> = lib.sets().unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["Friday.set", "Friday 2.set"]);
        // No .part file is left, and only sets can be saved as sets.
        assert!(!lib.root().join("Sets/Friday.set.part").exists());
        let not = Loc::top(Place::Projects).join("Friday.set").unwrap();
        assert!(lib.save_set(&not, &set).is_err());
        // A failed write leaves the set as it was.
        fs::create_dir(lib.root().join("Sets/Friday.set.part")).unwrap();
        assert!(lib.save_set(&loc, &Set::default()).is_err());
        assert_eq!(lib.load_set(&loc).unwrap(), set);
    }

    #[test]
    fn a_renamed_song_is_found_by_its_id_and_a_gone_one_is_not_ready() {
        let lib = Library::at(&temp()).unwrap();
        let projects = Loc::top(Place::Projects);
        let night = projects.join("Night drive").unwrap();
        let saved = lib
            .save_project(&night, &song_snap("Night drive"), 0)
            .unwrap();
        lib.update_manifest(&night).unwrap();
        let song = SetSong::new(&saved.id, "Night drive");
        let yes = |_: &str| true;
        assert_eq!(
            lib.check_song(&song, &yes),
            (Found::At(night.clone()), SongCheck::default())
        );

        // Renamed: still found, still ready.
        let moved = lib.rename(&night, "Night drive (live)").unwrap();
        let (found, check) = lib.check_song(&song, &yes);
        assert_eq!(found, Found::Moved(moved.clone()));
        assert!(check.ready(), "{check:?}");

        // A needed keyboard not connected stops it; a nice one limits it.
        let mut needy = song.clone();
        needy.needs = vec![
            Need {
                name: "Keystep 37".into(),
                needed: true,
            },
            Need {
                name: "FS-6".into(),
                needed: false,
            },
        ];
        let (_, check) = lib.check_song(&needy, &|n: &str| n == "nothing");
        assert_eq!(check.not_ready, ["Keystep 37 isn\u{2019}t connected"]);
        assert_eq!(check.limits, ["FS-6 isn\u{2019}t connected"]);
        assert_eq!(check.word(), "NOT READY");

        // A sound gone missing stops it.
        let dir = lib.root().join("Projects/Night drive (live)");
        fs::create_dir_all(dir.join("samples")).unwrap();
        fs::write(dir.join("samples/pad.wav"), b"RIFF").unwrap();
        lib.update_manifest(&moved).unwrap();
        fs::remove_file(dir.join("samples/pad.wav")).unwrap();
        let (_, check) = lib.check_song(&song, &yes);
        assert_eq!(check.not_ready, ["the file samples/pad.wav is missing"]);

        // In the Trash: gone.
        lib.delete(&moved).unwrap();
        let (found, check) = lib.check_song(&song, &yes);
        assert_eq!(found, Found::Gone);
        assert_eq!(
            check.not_ready,
            ["the project Night drive isn\u{2019}t in Projects"]
        );

        // A song with no id is found by its folder, and has no list of its
        // files until it is saved.
        let old = projects.join("Old song").unwrap();
        lib.save_project(&old, &song_snap("Old song"), 0).unwrap();
        let (found, check) = lib.check_song(&SetSong::new("", "Old song"), &yes);
        assert_eq!(found, Found::At(old));
        assert!(check.unchecked && check.ready());
        assert_eq!(check.word(), "NOT CHECKED");
    }
}
