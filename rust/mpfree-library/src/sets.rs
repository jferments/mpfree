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
//! checked = <seconds since 1970> <tempo then>
//! not-ready = <why, from that check>
//! limit = <what it plays without, from that check>
//! unchecked = yes
//! soundcheck = <seconds since 1970> <open ms> <peak kB> <CPU, tenths of a %>
//!              <longest disk wait ms> <misses> <xruns> <change ms, or -> <part>
//! ```
//!
//! Before the first song, `media = <seconds since 1970> <files> <bytes>`
//! says the set plays from show media copied to the stick (live shows B2).
//!
//! - **Saved whole.** [`Library::save_set`] writes `<name>.set.part`,
//!   makes it durable and renames it over the set, so a power cut leaves
//!   the set as it was or as it is now, never half of each.
//! - **Lines it doesn't know are kept,** with the song they follow, so a
//!   set saved by a newer build keeps what this one can't use.
//! - **The check** ([`Library::check_song`]) is the open's quick check
//!   (live shows A1): the project reads, every file its manifest lists is
//!   there and unchanged, and the keyboards and controls it needs are
//!   connected. Each song keeps its last check (`checked` and the lines
//!   after it), so the Sets place shows it after a restart.

use crate::project::{write_synced, NewSample, Snapshot, SAMPLES};
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
/// Recovery copies a song keeps at most (one for each time it was played
/// with changes, until they are kept or thrown away).
pub const MAX_ON_STAGE: usize = 16;
/// What a song's recovery copy is called in Projects: "Night drive on
/// stage", " 2" ... if taken.
pub const ON_STAGE: &str = "on stage";
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
    /// Its last check: when (seconds since 1970) and what it said.
    pub check: Option<(u64, SongCheck)>,
    /// What Soundcheck last measured of it on this MPC (live shows B2).
    pub soundcheck: Option<Soundcheck>,
    /// It changed on stage (live shows B1): the recovery copies of its
    /// changes, each a project in Projects ([`Library::on_stage_copy`]),
    /// oldest first, until the musician keeps or throws them away.
    pub on_stage: Vec<String>,
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

/// What Soundcheck measured of one song on this MPC (live shows B2): its
/// cold open, then its busiest part played, then its change into the next
/// song. These numbers take the place of estimates in the set check and in
/// the Show view's COLD LOAD pause.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Soundcheck {
    /// When it ran, in seconds since 1970.
    pub at: u64,
    /// How long its cold open took, in milliseconds.
    pub open_ms: u32,
    /// Memory in use at its peak while it played, in kB.
    pub peak_kb: u64,
    /// The worst CPU in its busiest part, in tenths of a percent.
    pub cpu_x10: u16,
    /// The longest a stream's read waited, in milliseconds.
    pub disk_wait_ms: u32,
    /// Times a stream ran dry.
    pub misses: u32,
    /// Times the sound card ran out of sound.
    pub xruns: u32,
    /// The pause in its change into the next song, in milliseconds: from
    /// Go to the next song's first period. None for the set's last song,
    /// or if Soundcheck stopped before it.
    pub change_ms: Option<u32>,
    /// Its busiest part's name ("Chorus 2").
    pub part: String,
}

impl Soundcheck {
    /// It glitched: a stream ran dry or the sound card ran out of sound.
    /// The song still plays, with that as its limit.
    pub fn glitched(&self) -> bool {
        self.misses > 0 || self.xruns > 0
    }

    /// Its line in the check view: "opens in 3.1 s · 540 MB · CPU 71 % in
    /// Chorus 2".
    pub fn line(&self) -> String {
        let mut s = format!(
            "opens in {} s \u{b7} {} MB \u{b7} CPU {} %",
            tenths(self.open_ms / 100),
            self.peak_kb.div_ceil(1024),
            (self.cpu_x10 + 5) / 10
        );
        if !self.part.is_empty() {
            s.push_str(&format!(" in {}", self.part));
        }
        s
    }

    fn to_line(&self) -> String {
        let change = self.change_ms.map_or("-".to_string(), |c| c.to_string());
        format!(
            "soundcheck = {} {} {} {} {} {} {} {change} {}",
            self.at,
            self.open_ms,
            self.peak_kb,
            self.cpu_x10,
            self.disk_wait_ms,
            self.misses,
            self.xruns,
            one_line(&self.part)
        )
    }

    /// Reads a `soundcheck` line's value; None if a number doesn't read.
    fn parse(value: &str) -> Option<Soundcheck> {
        let mut words = value.splitn(9, ' ');
        let mut next = || words.next();
        let at = next()?.parse().ok()?;
        let open_ms = next()?.parse().ok()?;
        let peak_kb = next()?.parse().ok()?;
        let cpu_x10 = next()?.parse::<u16>().ok()?.min(1000);
        let disk_wait_ms = next()?.parse().ok()?;
        let misses = next()?.parse().ok()?;
        let xruns = next()?.parse().ok()?;
        let change_ms = match next()? {
            "-" => None,
            c => Some(c.parse().ok()?),
        };
        let part = next().unwrap_or("").trim().to_string();
        Some(Soundcheck {
            at,
            open_ms,
            peak_kb,
            cpu_x10,
            disk_wait_ms,
            misses,
            xruns,
            change_ms,
            part,
        })
    }
}

/// A count of tenths as words: "3.1", "12".
fn tenths(x10: u32) -> String {
    if x10.is_multiple_of(10) {
        (x10 / 10).to_string()
    } else {
        format!("{}.{}", x10 / 10, x10 % 10)
    }
}

/// A set's show media (live shows B2): what Prepare show media copied to
/// the stick, and when. The set plays from those copies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShowMedia {
    /// When the copies were made and checked, in seconds since 1970.
    pub at: u64,
    pub files: u32,
    pub bytes: u64,
}

/// A set: its songs, in the order they play.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Set {
    pub songs: Vec<SetSong>,
    /// Its show media, once Prepare show media has run (live shows B2).
    pub media: Option<ShowMedia>,
    /// Lines before the first song that this build doesn't know.
    pub extra: Vec<String>,
}

impl Set {
    pub fn to_text(&self) -> String {
        let mut s = String::from(SET_FORMAT);
        s.push('\n');
        if let Some(m) = self.media {
            s.push_str(&format!("media = {} {} {}\n", m.at, m.files, m.bytes));
        }
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
            if let Some((at, c)) = &song.check {
                s.push_str(&format!("checked = {at} {}\n", tempo_word(c.tempo_x10)));
                for w in &c.not_ready {
                    s.push_str(&format!("not-ready = {}\n", one_line(w)));
                }
                for w in &c.limits {
                    s.push_str(&format!("limit = {}\n", one_line(w)));
                }
                if c.unchecked {
                    s.push_str("unchecked = yes\n");
                }
            }
            if let Some(sc) = &song.soundcheck {
                s.push_str(&sc.to_line());
                s.push('\n');
            }
            for c in &song.on_stage {
                s.push_str(&format!("changed = {}\n", one_line(c)));
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
                let mut words = value.split(' ').map(str::parse::<u64>);
                match (key, words.next(), words.next(), words.next()) {
                    ("media", Some(Ok(at)), Some(Ok(files)), Some(Ok(bytes))) => {
                        set.media = Some(ShowMedia {
                            at,
                            files: u32::try_from(files).unwrap_or(u32::MAX),
                            bytes,
                        })
                    }
                    _ => set.extra.push(line.to_string()),
                }
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
                "changed" if crate::valid_name(value) && song.on_stage.len() < MAX_ON_STAGE => {
                    song.on_stage.push(value.to_string())
                }
                "checked" => {
                    let (at, tempo) = value.split_once(' ').unwrap_or((value, ""));
                    match at.parse::<u64>() {
                        Ok(at) => {
                            let tempo_x10 = tempo
                                .trim()
                                .parse::<f64>()
                                .ok()
                                .filter(|t| t.is_finite() && *t > 0.0 && *t < 1000.0)
                                .map_or(0, |t| (t * 10.0).round() as u32);
                            song.check = Some((
                                at,
                                SongCheck {
                                    tempo_x10,
                                    ..SongCheck::default()
                                },
                            ));
                        }
                        Err(_) => song.check = None,
                    }
                }
                "soundcheck" => match Soundcheck::parse(value) {
                    Some(sc) => song.soundcheck = Some(sc),
                    None => song.extra.push(line.to_string()),
                },
                "not-ready" | "limit" | "unchecked" if song.check.is_some() => {
                    let c = &mut song.check.as_mut().expect("checked").1;
                    match key {
                        "not-ready" => c.not_ready.push(value.to_string()),
                        "limit" => c.limits.push(value.to_string()),
                        _ => c.unchecked = value == "yes",
                    }
                }
                _ => song.extra.push(line.to_string()),
            }
        }
        Ok(set)
    }
}

/// A tempo kept to a tenth: "92", "92.5", or "0" for none.
fn tempo_word(x10: u32) -> String {
    if x10.is_multiple_of(10) {
        (x10 / 10).to_string()
    } else {
        format!("{}.{}", x10 / 10, x10 % 10)
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
    /// Its tempo when it was checked, in tenths of a BPM (0: it didn't
    /// read).
    pub tempo_x10: u32,
}

impl SongCheck {
    pub fn ready(&self) -> bool {
        self.not_ready.is_empty()
    }

    /// Its tempo as the Set editor shows it ("92 BPM"), if it read.
    pub fn tempo(&self) -> Option<String> {
        (self.tempo_x10 > 0).then(|| format!("{} BPM", tempo_word(self.tempo_x10)))
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

    /// Every set, read: each one's entry and its songs, or why it couldn't
    /// be read. Sets are a few KB each, so the Sets place reads them all
    /// at once to show how many songs each holds.
    pub fn all_sets(&self) -> io::Result<Vec<(Entry, Result<Set, String>)>> {
        let top = Self::sets_top();
        Ok(self
            .sets()?
            .into_iter()
            .map(|e| {
                let set = top
                    .join(&e.name)
                    .and_then(|l| self.load_set(&l))
                    .map_err(|err| err.to_string());
                (e, set)
            })
            .collect())
    }

    /// The projects a set can hold: each project in Projects (not
    /// autosave's) by its folder's name, with its lasting id (empty for
    /// one saved before ids), in natural order.
    pub fn songs(&self) -> io::Result<Vec<(String, String)>> {
        let top = Loc::top(Place::Projects);
        Ok(self
            .list(&top)?
            .into_iter()
            .filter(|e| e.kind == crate::Kind::Project)
            .filter_map(|e| {
                let meta = self.project_meta(&top.join(&e.name).ok()?).ok()?;
                (!meta.is_autosave()).then_some((e.name, meta.id))
            })
            .collect())
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
        self.check_song_with(song, connected, false)
    }

    /// [`Library::check_song`], reading every file in full when `deep`
    /// (Check every file, for the day of the show). Made from the same
    /// facts as B2's preflight ([`Library::preflight_song`]).
    pub fn check_song_with(
        &self,
        song: &SetSong,
        connected: &dyn Fn(&str) -> bool,
        deep: bool,
    ) -> (Found, SongCheck) {
        let f = self.song_facts(song, connected, deep);
        let check = f.song_check(song);
        (f.found, check)
    }
}

impl Library {
    /// A song's changes made on stage (live shows B1): saved whole as a
    /// recovery copy, a project of its own beside it named "Night drive on
    /// stage" (or " 2" ...), never over anything. The copy is marked as
    /// autosave's work, so it has no lasting id of its own (it isn't the
    /// song) and opens as unsaved work. The song stays as it was saved.
    pub fn on_stage_copy(
        &self,
        song: &str,
        snap: &Snapshot,
        samples: &[NewSample],
    ) -> io::Result<Loc> {
        // Room for " on stage" and a " 99" after it, the song's name cut
        // to fit.
        let room = crate::name::MAX_NAME - ON_STAGE.len() - 4;
        let mut cut = String::new();
        for c in song.chars() {
            if cut.len() + c.len_utf8() > room {
                break;
            }
            cut.push(c);
        }
        let base = clean_name(&format!("{} {ON_STAGE}", cut.trim_end()), ON_STAGE);
        let name = crate::unique_folder_name(&base, |n| {
            Loc::top(Place::Projects)
                .join(n)
                .is_ok_and(|l| self.exists(&l))
        });
        let loc = Loc::top(Place::Projects).join(&name)?;
        let mut snap = snap.clone();
        snap.meta.mark_autosave();
        self.save_project_with(&loc, &snap, 0, samples)?;
        Ok(loc)
    }

    /// Keep the changes (the Set editor, after a set): the recovery copy
    /// `copy` is saved into the song `song` as its newest save, so its
    /// Earlier saves keep the version from before (`keep` of them), with
    /// any sound the song doesn't have yet; then the copy goes to the
    /// Trash. The song keeps its own lasting id.
    pub fn keep_on_stage(&self, copy: &Loc, song: &Loc, keep: usize) -> io::Result<()> {
        let mut snap = self.open_project(copy)?;
        snap.meta.clear_autosave();
        snap.meta.name = song.name().to_string();
        let from = self.project_dir(copy)?.join(SAMPLES);
        let have = self.project_dir(song)?.join(SAMPLES);
        let mut samples = Vec::new();
        if let Ok(rd) = fs::read_dir(&from) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let is_file = e.file_type().is_ok_and(|t| t.is_file());
                if is_file && crate::valid_name(&name) && !have.join(&name).exists() {
                    samples.push(NewSample::file(name, e.path()));
                }
            }
        }
        self.save_project_with(song, &snap, keep, &samples)?;
        // The list of its files that set lists check: a failure here only
        // leaves the song unchecked until its next save.
        let _ = self.update_manifest(song);
        self.delete(copy)?;
        Ok(())
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
        a.check = Some((
            1_763_836_800,
            SongCheck {
                not_ready: vec!["the file samples/strings.wav is missing".into()],
                limits: vec!["FS-6 isn\u{2019}t connected".into()],
                unchecked: false,
                tempo_x10: 925,
            },
        ));
        a.soundcheck = Some(Soundcheck {
            at: 1_763_836_900,
            open_ms: 3_140,
            peak_kb: 552_960,
            cpu_x10: 712,
            disk_wait_ms: 14,
            misses: 0,
            xruns: 0,
            change_ms: Some(4_200),
            part: "Chorus 2".into(),
        });
        let b = SetSong::new("", "Old song");
        let set = Set {
            songs: vec![a, b],
            media: Some(ShowMedia {
                at: 1_763_837_000,
                files: 13,
                bytes: 2_576_980_378,
            }),
            extra: vec![],
        };
        let text = set.to_text();
        assert_eq!(Set::parse(&text).unwrap(), set);
        assert!(text.contains("song = - Old song\n"), "{text}");
        assert!(
            text.contains("soundcheck = 1763836900 3140 552960 712 14 0 0 4200 Chorus 2\n"),
            "{text}"
        );
        assert!(text.starts_with("mpfree-set 1\nmedia = 1763837000 13 2576980378\n"));
        let sc = set.songs[0].soundcheck.as_ref().unwrap();
        assert_eq!(
            sc.line(),
            "opens in 3.1 s \u{b7} 540 MB \u{b7} CPU 71 % in Chorus 2"
        );
        assert!(!sc.glitched());
        // A soundcheck that stopped before the change, and one with a
        // number it can't read (kept as it was, for a newer build).
        let last = "mpfree-set 1\nsong = - A\nsoundcheck = 5 2000 1024 905 9 1 0 - Outro\nsong = - B\nsoundcheck = 5 x\n";
        let back = Set::parse(last).unwrap();
        let sc = back.songs[0].soundcheck.as_ref().unwrap();
        assert_eq!(
            (sc.change_ms, sc.glitched(), sc.line().as_str()),
            (
                None,
                true,
                "opens in 2 s \u{b7} 1 MB \u{b7} CPU 91 % in Outro"
            )
        );
        assert_eq!(back.songs[1].soundcheck, None);
        assert_eq!(back.songs[1].extra, vec!["soundcheck = 5 x".to_string()]);

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
            media: None,
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
        let (found, check) = lib.check_song(&song, &yes);
        assert_eq!(found, Found::At(night.clone()));
        assert_eq!(
            check,
            SongCheck {
                tempo_x10: 1200,
                ..SongCheck::default()
            }
        );
        assert_eq!(check.tempo().as_deref(), Some("120 BPM"));

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

    #[test]
    fn the_sets_place_reads_every_set_and_lists_the_songs_it_can_add() {
        let lib = Library::at(&temp()).unwrap();
        let projects = Loc::top(Place::Projects);
        let night = projects.join("Night drive").unwrap();
        let saved = lib
            .save_project(&night, &song_snap("Night drive"), 0)
            .unwrap();
        // Autosave's project and plain folders aren't songs.
        let auto = lib.autosave_project().unwrap();
        let mut snap = song_snap("Autosave");
        snap.meta.mark_autosave();
        lib.save_project(&auto, &snap, 0).unwrap();
        lib.new_folder(&projects, "Old stuff").unwrap();
        assert_eq!(
            lib.songs().unwrap(),
            [("Night drive".to_string(), saved.id.clone())]
        );

        let friday = lib.new_set("Friday").unwrap();
        let set = Set {
            media: None,
            songs: vec![SetSong::new(&saved.id, "Night drive")],
            extra: vec![],
        };
        lib.save_set(&friday, &set).unwrap();
        fs::write(lib.root().join("Sets/Broken.set"), "not a set\n").unwrap();
        let all = lib.all_sets().unwrap();
        let names: Vec<&str> = all.iter().map(|(e, _)| e.name.as_str()).collect();
        assert_eq!(names, ["Broken.set", "Friday.set"]);
        assert!(all[0].1.is_err());
        assert_eq!(all[1].1.as_ref().unwrap().songs.len(), 1);
    }

    /// Live shows B1: a song's changes made on stage go into a recovery
    /// copy beside it, never into the song, and never over anything; the
    /// set remembers the copies; keeping them saves the copy into the song
    /// (its Earlier saves keep the version before) and the copy goes.
    #[test]
    fn changes_on_stage_go_into_a_copy_that_can_be_kept() {
        let lib = Library::at(&temp()).unwrap();
        let projects = Loc::top(Place::Projects);
        let night = projects.join("Night drive").unwrap();
        let saved = lib
            .save_project(&night, &song_snap("Night drive"), 0)
            .unwrap();
        let mut changed = song_snap("Night drive");
        changed.parts[0].1 = "clip b\n".into();
        let take = NewSample::new(
            "take-1.wav",
            std::sync::Arc::new(|w: &mut dyn std::io::Write| w.write_all(b"RIFF take")),
        );
        let copy = lib.on_stage_copy("Night drive", &changed, &[take]).unwrap();
        assert_eq!(copy.name(), "Night drive on stage");
        let again = lib.on_stage_copy("Night drive", &changed, &[]).unwrap();
        assert_eq!(again.name(), "Night drive on stage 2", "never over a copy");
        // The song is as it was saved; the copy is unsaved work, not a song.
        assert_eq!(lib.open_project(&night).unwrap().parts[0].1, "clip a\n");
        assert!(lib.open_project(&copy).unwrap().meta.is_autosave());
        assert_eq!(
            lib.songs().unwrap(),
            [("Night drive".to_string(), saved.id.clone())]
        );
        // A long name keeps "on stage".
        let long = "L".repeat(120);
        let l = lib.on_stage_copy(&long, &changed, &[]).unwrap();
        assert!(l.name().ends_with(" on stage"), "{}", l.name());

        // The set remembers the copies.
        let mut song = SetSong::new(&saved.id, "Night drive");
        song.on_stage = vec![copy.name().to_string(), again.name().to_string()];
        let set = Set {
            media: None,
            songs: vec![song],
            extra: vec![],
        };
        let text = set.to_text();
        assert!(
            text.contains("changed = Night drive on stage 2\n"),
            "{text}"
        );
        assert_eq!(Set::parse(&text).unwrap(), set);

        // Kept: the song has the changes and the take, keeps its id, and
        // its last save is an earlier save; the copy is in the Trash.
        lib.keep_on_stage(&copy, &night, 5).unwrap();
        let back = lib.open_project(&night).unwrap();
        assert_eq!(back.parts[0].1, "clip b\n");
        assert!(!back.meta.is_autosave());
        assert_eq!(back.meta.id, saved.id);
        assert_eq!(
            fs::read(lib.sample_path(&night, "take-1.wav").unwrap()).unwrap(),
            b"RIFF take"
        );
        assert_eq!(lib.versions(&night).unwrap().len(), 1);
        assert!(!lib.exists(&copy));
        assert_eq!(lib.find_song(&set.songs[0]), Found::At(night));
    }
}
