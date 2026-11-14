//! Three demo songs, each in several sections, made to be played,
//! performed, edited, saved and opened again by the workflow tests in
//! `song_workflows.rs` (Jay, 14 Nov: the Phase 3 gate asks for "a multi-section
//! composition that would be awkward in a conventional MPC workflow").
//!
//! Each one leans on what an MPC can't do the same way:
//!
//! - **Night Market** (92 BPM, swung sixteenths): a beat in five sections
//!   chained by scene follow actions, so launching the Intro plays the
//!   whole song (Intro, Verse, Hook, Break, Outro, then stop). Fill-only
//!   snares, a stab on every fourth pass, chords through the track's arp,
//!   and the same song laid out on the Song view with a marker for each
//!   section.
//! - **Glass Garden** (124 BPM, straight): polymeter. Clips of 16, 3, 5,
//!   7 and 12 steps loop against each other, so the bar only comes round
//!   every 420 steps. Ratchets, a bass that alternates by trig condition
//!   (1:2 and 2:2), panning locked per step, a lead at half speed, and the
//!   morph sweeping the lead's level from the Seed scene to the Bloom
//!   scene along the song.
//! - **Tidal Suite** (76 BPM, swung eighths): four movements that pick
//!   their own way with chance follow actions (the Ebb plays again or goes
//!   on, the Flow goes on or back), drums that change on the first pass
//!   and after it, hits that answer the one before (PRE) and the track to
//!   the left (NEI), and an Undertow that swaps between two clips by clip
//!   follow actions. It has no song yet: a test makes one from the scenes.
//!
//! The songs are the time engine's model (`mpfree_seq::Project`), their
//! tempo and swing, put in a library as a saved project would be (the
//! other parts as a new project has them), so a test opens one through
//! BROWSE as a musician would. The pads are the factory kit's: pad 0 is
//! the Kick, 1 the Snare and so on ([`pad`]); the melodic tracks play pads
//! too, as notes of the kit (a melodic part on the pads, which the synth
//! plays when it has them).

use mpfree_fx::ids::mixer as mx;
use mpfree_seq::mpfree_param::{OnTrack, Owner, ParamAddr, RelAddr, Strip};
use mpfree_seq::{
    Clip, Cond, Follow, FollowAction, Lock, Marker, MorphRows, Note, Placement, Project, Scene,
    Speed, Track,
};
use mpfree_transport::time::{Swing, Tick, PPQ};

/// A sixteenth, in ticks.
pub const STEP: Tick = PPQ as Tick / 4;
/// A bar of 4/4, in ticks.
pub const BAR: Tick = PPQ as Tick * 4;

/// The factory kit's pads (`gui::sample_kits`).
pub mod pad {
    pub const KICK: u8 = 0;
    pub const SNARE: u8 = 1;
    pub const HAT: u8 = 2;
    pub const OPEN_HAT: u8 = 3;
    pub const CLAP: u8 = 4;
    pub const RIM: u8 = 5;
    pub const TOM_LOW: u8 = 6;
    pub const TOM_HI: u8 = 7;
    pub const CRASH: u8 = 8;
    pub const RIDE: u8 = 9;
    pub const SHAKER: u8 = 10;
    pub const PERC_1: u8 = 11;
    pub const PERC_2: u8 = 12;
    pub const SUB: u8 = 14;
}

/// A demo song: its clips, scenes and song, and how it is played.
#[derive(Clone, Debug)]
pub struct Song {
    pub name: &'static str,
    pub tempo: f64,
    pub swing: Swing,
    pub clips: Project,
}

impl Song {
    /// The scenes' names, in row order (the song's sections).
    pub fn sections(&self) -> Vec<&str> {
        self.clips.scenes.iter().map(|s| s.name.as_str()).collect()
    }

    /// The track called `name`.
    pub fn track(&self, name: &str) -> usize {
        self.clips
            .tracks
            .iter()
            .position(|t| t.name == name)
            .unwrap_or_else(|| panic!("{} has no {name} track", self.name))
    }

    /// The scene called `name`.
    pub fn scene(&self, name: &str) -> usize {
        self.clips
            .scenes
            .iter()
            .position(|s| s.name == name)
            .unwrap_or_else(|| panic!("{} has no {name} scene", self.name))
    }
}

/// All three, in the order the tests take them.
pub fn all() -> Vec<Song> {
    vec![night_market(), glass_garden(), tidal_suite()]
}

// ---------------------------------------------------------------- writing

/// Hits on `pad` from a pattern of steps, one character a step (spaces
/// are for reading only):
///
/// | Char | Note |
/// |---|---|
/// | `.` | rest |
/// | `x` / `X` / `o` | velocity 100 / 124 / 60 (a ghost) |
/// | `f` | only while FILL is on |
/// | `n` | only while FILL is off |
/// | `1` / `!` | only on the clip's first pass / on every pass but the first |
/// | `2` / `4` | on pass 2 of every 2 / pass 4 of every 4 |
/// | `a` | on pass 1 of every 2 |
/// | `r` / `R` | a ratchet of 2 / of 3 |
/// | `?` | half the time |
/// | `p` / `q` | only if the track's last chance note played / did not |
/// | `e` / `E` | only if the track to the left's last one played / did not |
fn hits(clip: &mut Clip, pad: u8, pattern: &str) {
    let steps = pattern.chars().filter(|c| !c.is_whitespace());
    for (k, c) in steps.enumerate() {
        if c == '.' {
            continue;
        }
        let mut n = Note::new(k as Tick * STEP, STEP / 2, pad, 100);
        match c {
            'x' => {}
            'X' => n.velocity = 124,
            'o' => n.velocity = 60,
            'f' => n.cond = Cond::Fill,
            'n' => n.cond = Cond::NotFill,
            '1' => n.cond = Cond::First,
            '!' => n.cond = Cond::NotFirst,
            '2' => n.cond = Cond::Every { a: 2, b: 2 },
            '4' => n.cond = Cond::Every { a: 4, b: 4 },
            'a' => n.cond = Cond::Every { a: 1, b: 2 },
            'r' => (n.ratchet, n.len) = (2, STEP),
            'R' => (n.ratchet, n.len) = (3, STEP),
            '?' => n.chance = 50,
            'p' => n.cond = Cond::Pre,
            'q' => n.cond = Cond::NotPre,
            'e' => n.cond = Cond::Nei,
            'E' => n.cond = Cond::NotNei,
            other => panic!("no step called {other:?}"),
        }
        clip.add_note(n).expect("the step fits its clip");
    }
}

/// A clip `steps` sixteenths long with these rows of hits.
fn beat(name: &str, steps: Tick, rows: &[(u8, &str)]) -> Clip {
    let mut c = Clip::new(name, steps * STEP);
    for (pad, pattern) in rows {
        hits(&mut c, *pad, pattern);
    }
    c
}

/// A melodic clip `bars` long: (step, length in steps, pad, velocity).
fn line(name: &str, bars: Tick, notes: &[(Tick, Tick, u8, u8)]) -> Clip {
    let mut c = Clip::new(name, bars * BAR);
    for &(at, len, pad, velocity) in notes {
        c.add_note(Note::new(at * STEP, len * STEP, pad, velocity))
            .expect("the note fits its clip");
    }
    c
}

/// Chords: each (step, length in steps, pads) sounds its pads together.
fn chords(name: &str, bars: Tick, chords: &[(Tick, Tick, &[u8])]) -> Clip {
    let mut notes = Vec::new();
    for &(at, len, pads) in chords {
        for &p in pads {
            notes.push((at, len, p, 88));
        }
    }
    line(name, bars, &notes)
}

fn track(name: &str, dest: u16, slots: Vec<(usize, Clip)>) -> Track {
    let mut t = Track::new(name, dest);
    for (slot, c) in slots {
        t.slots[slot] = Some(c);
    }
    t
}

fn scene(name: &str, follow: Option<Follow>) -> Scene {
    let mut s = Scene::named(name);
    s.follow = follow;
    s
}

/// After `bars`, `a` with `chance` percent, else `b`.
fn maybe(bars: u16, a: FollowAction, chance: u8, b: FollowAction) -> Follow {
    Follow {
        bars,
        action: a,
        other: b,
        chance,
    }
}

/// A track strip's volume (dB), as a scene keeps it for the morph.
fn volume(track: u8) -> ParamAddr {
    ParamAddr::new(Owner::Strip(Strip::Track(track)), mx::VOLUME)
}

/// The song: each (track, slot, from bar, bars) linked to its slot, and a
/// marker on each section.
/// A section of the song: its name, first bar, bars, and the (track,
/// slot) clips placed in it.
type Section<'a> = (&'a str, Tick, Tick, &'a [(usize, usize)]);

fn song(p: &mut Project, sections: &[Section]) {
    let mut placed = Vec::new();
    for &(name, from, bars, clips) in sections {
        p.arrangement.markers.push(Marker {
            at: from * BAR,
            name: name.to_string(),
        });
        for &(track, slot) in clips {
            placed.push(Placement::linked(track, slot, from * BAR, bars * BAR));
        }
    }
    placed.sort_by_key(|pl| (pl.track, pl.start));
    p.arrangement.placements = placed;
}

// ------------------------------------------------------------ Night Market

/// Night Market: see the module notes.
pub fn night_market() -> Song {
    use pad::*;
    let drums = track(
        "Drums",
        0,
        vec![
            (0, beat("Intro hats", 16, &[(HAT, "x.o. x.o. x.o. x.oo")])),
            (
                1,
                beat(
                    "Verse beat",
                    32,
                    &[
                        (KICK, "X..x ..x. X... ..x. X..x ..x. X.x. ...."),
                        (SNARE, ".... X..o .... X... .... X..o .... X.ff"),
                        (HAT, "x.x. x.x. x.x. x.x. x.x. x.x. x.x. x.xx"),
                    ],
                ),
            ),
            (
                2,
                beat(
                    "Hook beat",
                    32,
                    &[
                        (KICK, "X..x ..x. X..x .x.. X..x ..x. X.x. x..."),
                        (SNARE, ".... X... .... X..o .... X... .... Xfff"),
                        (CLAP, ".... x... .... x... .... x... .... x..."),
                        (HAT, "x.xx x.x. x.xx x.x. x.xx x.x. x.xx x.x."),
                        (OPEN_HAT, ".... ...x .... ...x .... ...x .... ...n"),
                    ],
                ),
            ),
            (
                4,
                beat(
                    "Outro",
                    16,
                    &[(KICK, "X... .... X... ...."), (HAT, "x... x... x... x.o.")],
                ),
            ),
        ],
    );
    let perc = track(
        "Perc",
        1,
        vec![
            (1, beat("Shaker", 16, &[(SHAKER, "o.x. o.x. o.x. oxx.")])),
            (
                2,
                beat(
                    "Hook perc",
                    16,
                    &[
                        (PERC_1, "..x. ...? ..x. .?.."),
                        (PERC_2, "x... ..?. x... ...."),
                    ],
                ),
            ),
            (
                3,
                beat("Break shaker", 16, &[(SHAKER, "x... x... x... x...")]),
            ),
        ],
    );
    let bass = track(
        "Bass",
        2,
        vec![
            (
                1,
                line(
                    "Verse bass",
                    2,
                    &[
                        (0, 3, 0, 110),
                        (6, 2, 0, 90),
                        (10, 4, 3, 100),
                        (16, 3, 5, 110),
                        (22, 2, 5, 90),
                        (26, 4, 7, 100),
                    ],
                ),
            ),
            (
                2,
                line(
                    "Hook bass",
                    2,
                    &[
                        (0, 2, 0, 116),
                        (3, 1, 12, 80),
                        (6, 2, 0, 100),
                        (10, 2, 3, 110),
                        (14, 2, 2, 90),
                        (16, 2, 5, 116),
                        (19, 1, 12, 80),
                        (22, 2, 5, 100),
                        (26, 4, 7, 110),
                    ],
                ),
            ),
            (4, line("Outro bass", 1, &[(0, 8, 0, 100), (8, 8, 7, 80)])),
        ],
    );
    let mut keys = track(
        "Keys",
        3,
        vec![
            (
                0,
                chords(
                    "Intro chords",
                    2,
                    &[(0, 16, &[3, 7, 10]), (16, 16, &[5, 8, 12])],
                ),
            ),
            (
                2,
                chords(
                    "Hook chords",
                    2,
                    &[
                        (0, 8, &[3, 7, 10]),
                        (8, 8, &[2, 5, 9]),
                        (16, 8, &[5, 8, 12]),
                        (24, 8, &[7, 10, 14]),
                    ],
                ),
            ),
            (3, chords("Break chords", 1, &[(0, 16, &[2, 5, 9])])),
        ],
    );
    keys.through_arp = true;
    let mut stab = beat("Stab every 4th", 16, &[(RIM, "4... .... .... ....")]);
    stab.name = "Stab every 4th".into();
    let chops = track(
        "Chops",
        4,
        vec![
            (2, stab),
            (
                3,
                beat(
                    "Break chops",
                    16,
                    &[
                        (TOM_LOW, "x..r ..x. .R.. x..."),
                        (TOM_HI, "..x. x... ..x. .rx."),
                        (CRASH, "X... .... .... ...."),
                    ],
                ),
            ),
        ],
    );
    let mut p = Project::new();
    p.tracks = vec![drums, perc, bass, keys, chops];
    let mut brk = scene("Break", Some(Follow::after(4, FollowAction::Next)));
    brk.mutes = Some(1 << 1); // the Perc track sits the Break out
    let mut hook = scene("Hook", Some(Follow::after(8, FollowAction::Next)));
    hook.mutes = Some(0); // and comes back for the Hook
    p.scenes = vec![
        scene("Intro", Some(Follow::after(4, FollowAction::Next))),
        scene("Verse", Some(Follow::after(8, FollowAction::Next))),
        hook,
        brk,
        scene("Outro", Some(Follow::after(4, FollowAction::Stop))),
    ];
    let intro: &[(usize, usize)] = &[(0, 0), (3, 0)];
    let verse: &[(usize, usize)] = &[(0, 1), (1, 1), (2, 1)];
    let hookc: &[(usize, usize)] = &[(0, 2), (1, 2), (2, 2), (3, 2), (4, 2)];
    let breakc: &[(usize, usize)] = &[(1, 3), (3, 3), (4, 3)];
    let outro: &[(usize, usize)] = &[(0, 4), (2, 4)];
    song(
        &mut p,
        &[
            ("Intro", 0, 4, intro),
            ("Verse", 4, 8, verse),
            ("Hook", 12, 8, hookc),
            ("Verse 2", 20, 8, verse),
            ("Hook 2", 28, 8, hookc),
            ("Break", 36, 4, breakc),
            ("Outro", 40, 4, outro),
        ],
    );
    Song {
        name: "Night Market",
        tempo: 92.0,
        swing: Swing::new(240, 58).expect("a swing"),
        clips: p,
    }
}

// ------------------------------------------------------------ Glass Garden

/// Glass Garden: see the module notes.
pub fn glass_garden() -> Song {
    use pad::*;
    let kick = track(
        "Kick",
        0,
        vec![
            (0, beat("Pulse", 16, &[(KICK, "X... x... X... x...")])),
            (1, beat("Pulse skip", 16, &[(KICK, "X... x... X..2 x...")])),
            (2, beat("Pulse", 16, &[(KICK, "X... x... X... x...")])),
        ],
    );
    let hats = track(
        "Hats",
        1,
        vec![
            (0, beat("Three", 3, &[(HAT, "x.o")])),
            (1, beat("Three", 3, &[(HAT, "x.o")])),
            (2, beat("Three open", 3, &[(HAT, "x.."), (OPEN_HAT, "..x")])),
            (3, beat("Three", 3, &[(HAT, "x.o")])),
        ],
    );
    let clap = track(
        "Clap",
        2,
        vec![
            (1, beat("Five", 5, &[(CLAP, "....x")])),
            (2, beat("Five", 5, &[(CLAP, "..o.x")])),
        ],
    );
    let perc = track(
        "Perc",
        3,
        vec![
            (
                2,
                beat("Seven", 7, &[(PERC_1, "r..x.R."), (RIM, ".x....o")]),
            ),
            (
                3,
                beat("Seven", 7, &[(PERC_1, "r..x.R."), (RIM, ".x....o")]),
            ),
        ],
    );
    let mut twelve = line("Twelve", 1, &[]);
    twelve.length = 12 * STEP;
    hits(&mut twelve, SUB, "a..a .2.a 2..2");
    // Pan locked per step: left on the first beat, right on the third.
    let pan = RelAddr::new(OnTrack::Strip, mx::PAN);
    twelve
        .add_lock(Lock::new(0, None, pan, -0.6))
        .expect("a lock");
    twelve
        .add_lock(Lock::new(6 * STEP, None, pan, 0.6))
        .expect("a lock");
    let bass = track("Bass", 4, vec![(1, twelve.clone()), (2, twelve)]);
    let mut slow = line(
        "Slow lead",
        2,
        &[
            (0, 12, 7, 96),
            (12, 4, 10, 80),
            (16, 8, 12, 100),
            (24, 8, 9, 84),
        ],
    );
    slow.speed = Speed::new(1, 2).expect("half speed");
    let lead = track("Lead", 5, vec![(2, slow.clone()), (3, slow)]);
    let mut p = Project::new();
    p.tracks = vec![kick, hats, clap, perc, bass, lead];
    let mut seed = scene("Seed", None);
    seed.values = vec![(volume(5), -18.0)];
    let mut bloom = scene("Bloom", None);
    bloom.values = vec![(volume(5), 0.0)];
    p.scenes = vec![seed, scene("Grow", None), bloom, scene("Wilt", None)];
    song(
        &mut p,
        &[
            ("Seed", 0, 4, &[(0, 0), (1, 0)]),
            ("Grow", 4, 8, &[(0, 1), (1, 1), (2, 1), (4, 1)]),
            (
                "Bloom",
                12,
                8,
                &[(0, 2), (1, 2), (2, 2), (3, 2), (4, 2), (5, 2)],
            ),
            ("Wilt", 20, 8, &[(1, 3), (3, 3), (5, 3)]),
        ],
    );
    // The morph between Seed and Bloom all song long.
    p.arrangement.morphs = vec![MorphRows { at: 0, a: 0, b: 2 }];
    Song {
        name: "Glass Garden",
        tempo: 124.0,
        swing: Swing::default(),
        clips: p,
    }
}

// ------------------------------------------------------------- Tidal Suite

/// Tidal Suite: see the module notes.
pub fn tidal_suite() -> Song {
    use pad::*;
    let mut under_a = beat(
        "Undertow A",
        16,
        &[(KICK, "X... .... ..x. ...."), (RIM, ".... x... .... x..o")],
    );
    under_a.follow = Some(Follow::after(2, FollowAction::Next));
    let mut under_b = beat(
        "Undertow B",
        16,
        &[(KICK, "X... ..x. .... ...."), (RIM, ".... x... .... xxxo")],
    );
    under_b.follow = Some(Follow::after(2, FollowAction::Previous));
    let drums = track(
        "Drums",
        0,
        vec![
            (
                0,
                beat(
                    "Ebb",
                    32,
                    &[
                        (KICK, "X... .... x... .... X... .... x... ...."),
                        (RIM, "1... x... .... x... .... x... .... x..!"),
                        (RIDE, "x.x. x.x. x.x. x.x. x.x. x.x. x.x. x.x."),
                    ],
                ),
            ),
            (
                1,
                beat(
                    "Flow",
                    32,
                    &[
                        (KICK, "X... ..x. x... .... X... ..x. x..? ...."),
                        (SNARE, ".... X... .... X..p .... X... .... X.qp"),
                        (HAT, "x.x. x.x. x.x. x.x. x.x. x.x. x.x. x.x."),
                    ],
                ),
            ),
            (2, under_a),
            (3, under_b),
            (
                4,
                beat(
                    "Shore",
                    16,
                    &[
                        (KICK, "X... .... .... ...."),
                        (CRASH, "1... .... .... ...."),
                    ],
                ),
            ),
        ],
    );
    let toms = track(
        "Toms",
        1,
        vec![
            (
                1,
                beat("Answer", 6, &[(TOM_LOW, "e..E.."), (TOM_HI, "..e..x")]),
            ),
            (
                2,
                beat("Answer", 6, &[(TOM_LOW, "e..E.."), (TOM_HI, "..e..x")]),
            ),
        ],
    );
    let bass = track(
        "Bass",
        2,
        vec![
            (0, line("Ebb bass", 2, &[(0, 12, 0, 96), (16, 12, 5, 90)])),
            (
                1,
                line(
                    "Flow bass",
                    2,
                    &[
                        (0, 4, 0, 108),
                        (6, 2, 3, 90),
                        (8, 6, 5, 100),
                        (16, 4, 7, 108),
                        (22, 2, 5, 90),
                        (24, 8, 3, 100),
                    ],
                ),
            ),
            (
                2,
                line("Undertow bass", 1, &[(0, 3, 0, 120), (8, 3, 1, 110)]),
            ),
            (4, line("Shore bass", 1, &[(0, 16, 0, 90)])),
        ],
    );
    let mut pads = track(
        "Pads",
        3,
        vec![
            (0, chords("Tide", 2, &[(0, 32, &[0, 3, 7])])),
            (
                1,
                chords("Current", 2, &[(0, 16, &[3, 7, 10]), (16, 16, &[5, 8, 12])]),
            ),
            (3, chords("Deep", 1, &[(0, 16, &[1, 5, 8])])),
            (4, chords("Shoreline", 1, &[(0, 16, &[0, 4, 7])])),
        ],
    );
    pads.through_arp = true;
    let mut p = Project::new();
    p.tracks = vec![drums, toms, bass, pads];
    p.scenes = vec![
        // The Ebb goes on three times in four, else plays again.
        scene(
            "Ebb",
            Some(maybe(4, FollowAction::Next, 75, FollowAction::Again)),
        ),
        // The Flow goes on, or back to the Ebb.
        scene(
            "Flow",
            Some(maybe(8, FollowAction::Next, 60, FollowAction::Previous)),
        ),
        // On to the last row, the Shore (the next row is Undertow B's).
        scene("Undertow", Some(Follow::after(8, FollowAction::Last))),
        // The Undertow B row: only the clip follow actions reach it.
        scene("Undertow B", None),
        scene("Shore", Some(Follow::after(4, FollowAction::Stop))),
    ];
    Song {
        name: "Tidal Suite",
        tempo: 76.0,
        swing: Swing::new(480, 54).expect("a swing"),
        clips: p,
    }
}

// ------------------------------------------------------------- the library

/// `song` put in `lib` as a saved project, its other parts as `defaults`
/// (a new project's): what a test opens through BROWSE.
pub fn put(
    lib: &mpfree_library::Library,
    song: &Song,
    defaults: &crate::project::Defaults,
) -> mpfree_library::Loc {
    let mut snap = crate::project::new_snapshot(defaults);
    snap.meta.name = song.name.to_string();
    snap.meta.tempo = song.tempo;
    let set = |snap: &mut mpfree_library::Snapshot, owner: &str, text: String| match snap
        .parts
        .iter_mut()
        .find(|(o, _)| o == owner)
    {
        Some((_, t)) => *t = text,
        None => snap.parts.push((owner.to_string(), text)),
    };
    set(
        &mut snap,
        crate::project::CLIPS,
        mpfree_seq::save::to_text(&song.clips),
    );
    snap.meta.extra.retain(|(k, _)| k != crate::project::SWING);
    snap.meta.extra.push((
        crate::project::SWING.to_string(),
        format!("{} {}", song.swing.grid, song.swing.amount),
    ));
    let loc = mpfree_library::Loc::top(mpfree_library::Place::Projects)
        .join(song.name)
        .expect("a project name");
    lib.save_project(&loc, &snap, 10)
        .expect("the song is saved");
    loc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every song holds together as the time engine checks a project, and
    /// reads back from its text the same.
    #[test]
    fn the_songs_hold_together_and_read_back_the_same() {
        for s in all() {
            s.clips
                .check()
                .unwrap_or_else(|e| panic!("{}: {e:?}", s.name));
            let text = mpfree_seq::save::to_text(&s.clips);
            let back = mpfree_seq::save::from_text(&text).expect("reads back");
            assert_eq!(back, s.clips, "{}", s.name);
            assert!(s.sections().len() >= 4, "{} has sections", s.name);
        }
    }
}
