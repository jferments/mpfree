//! How ready a set is (disk streaming's P3, step 4): three levels, each
//! with its evidence, never one green "Ready".
//!
//! | Level | Evidence | What the screen says |
//! |---|---|---|
//! | Files ready | every file present, matching its snapshot and readable; starts prepared | "All files ready." |
//! | Read-tested | the stick test (D1) passed reads like this set's, on this stick | "Read test passed on this stick. Recording not tested." |
//! | Show-tested | the show test (D3) passed a workload covering this set, on this stick, with recording if the set records | "Show test passed on this stick, 22 Nov." |
//!
//! A test result covers the workload it tested, not every set with as
//! many stems ([`Workload::covers`]). It shows as a past result, not the
//! stick's health today. The screens are the live shows thread's; this is
//! what they show.

/// What a set asks of the stick and the sound thread, or what a test ran.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Workload {
    /// Sounds streaming at once.
    pub streams: u32,
    /// How many of them are FLAC (decoded as they stream, at their own
    /// measured cost): a WAV test doesn't cover them.
    pub flac: u32,
    /// Files the set plays, and their bytes, and the biggest.
    pub files: u32,
    pub bytes: u64,
    pub largest: u64,
    /// Takes recorded at once.
    pub recordings: u32,
    /// Sampler voices streaming at once.
    pub voices: u32,
    /// The cache's size.
    pub cache: u64,
    /// The stick, as the stick test names it.
    pub stick: String,
    /// The MPFree version.
    pub version: String,
}

impl Workload {
    /// Whether a test that ran `self` covers `set`: the same stick and
    /// MPFree version, a cache no bigger than the set's, and no more of
    /// anything than the test had.
    pub fn covers(&self, set: &Workload) -> bool {
        self.stick == set.stick
            && self.version == set.version
            && set.cache >= self.cache
            && set.streams <= self.streams
            && set.flac <= self.flac
            && set.files <= self.files
            && set.bytes <= self.bytes
            && set.largest <= self.largest
            && set.recordings <= self.recordings
            && set.voices <= self.voices
    }

    /// Whether a read test that ran `self` covers `set`'s reads: the same
    /// stick, as many streams and FLAC streams at least. It says nothing
    /// about recording.
    pub fn covers_reads(&self, set: &Workload) -> bool {
        self.stick == set.stick && set.streams <= self.streams && set.flac <= self.flac
    }
}

/// A test that passed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Passed {
    pub ran: Workload,
    /// When, as the screen shows it ("22 Nov").
    pub on: String,
}

/// What is known about a set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Evidence {
    /// The snapshot's check found nothing wrong.
    pub files: bool,
    /// Every start that must be in memory is ([`crate::policy::start_frames`]).
    pub starts: bool,
    /// The stick tests (D1) that passed.
    pub read_tests: Vec<Passed>,
    /// The show tests (D3) that passed.
    pub show_tests: Vec<Passed>,
}

/// How ready a set is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Level {
    /// Not even its files: the snapshot's check found something, or its
    /// starts aren't in memory yet.
    NotReady,
    FilesReady,
    ReadTested,
    /// When the covering show test passed.
    ShowTested {
        on: String,
    },
}

/// The level `set` has on what is known.
pub fn level(set: &Workload, e: &Evidence) -> Level {
    if !(e.files && e.starts) {
        return Level::NotReady;
    }
    if let Some(p) = e.show_tests.iter().find(|p| p.ran.covers(set)) {
        return Level::ShowTested { on: p.on.clone() };
    }
    if e.read_tests.iter().any(|p| p.ran.covers_reads(set)) {
        return Level::ReadTested;
    }
    Level::FilesReady
}

impl Level {
    /// What the screen says.
    pub fn words(&self) -> String {
        match self {
            Level::NotReady => "Files not ready.".into(),
            Level::FilesReady => "All files ready.".into(),
            Level::ReadTested => "Read test passed on this stick. Recording not tested.".into(),
            Level::ShowTested { on } => format!("Show test passed on this stick, {on}."),
        }
    }
}
