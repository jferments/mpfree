//! The time engine's model as text: the `clips` part of a saved project
//! (`parts/clips.txt` in the project folder, docs/dev/design/browser.md).
//!
//! One record a line, a word saying what it is first, then numbers, and
//! a name (when there is one) as the rest of the line. A track's clips,
//! a scene's values and a song clip's body follow the line that opens
//! them; `end` closes a clip. Lines this build doesn't know are skipped,
//! so a newer build's file still opens.
//!
//! ```text
//! mpfree-clips 2
//! perform
//! launch bars 2
//! nexttrack 3
//! nextclip 5
//! track Drums
//! dest 0
//! swing 240 58
//! solo
//! notefx 0 1 on chord shape=m7 voicing=close inv=0 below=- vel=90 strum=8 scale=-
//! notefx 1 2 off echo time=480t repeats=3 fb=70 pitch=0
//! notearp 1
//! transpose 5 key
//! follows
//! clip 0 7680 Beat 1
//! id 3
//! speed 1 1
//! launch ticks 960
//! legato
//! once
//! program 4 bank 2
//! direction pingpong
//! holds notes
//! touch 0:10 50:40
//! note 0 240 36 100 100 - 1 0 2
//! note 240 240 38 90 100 - 4 0 0 ramp:-40 slide
//! note 480 240 38 90 100 - 1 0 0 off tie
//! lock 480 36 pad36.4 0.25
//! lane t:mix.0 -60 6 0 n
//! point 0 0.5 r
//! original 7680 3
//! original note 0 240 36 100 100 - 1 0 2
//! original lock 480 36 pad36.4 0.25
//! original lane t:mix.0 -60 6 0 n
//! original point 0 0.5 r
//! end
//! track Loops A
//! dest 1
//! loops pads
//! clip 0 15360 Loop 1
//! loop 0
//! end
//! scene 0 Verse
//! mutes 5
//! value t1.mix.0 -3
//! program 1 12 bank 0
//! songclip 3840 Beat (own)
//! id 4
//! note 0 240 36 100 100 - 1 0 2
//! end
//! place 0 0 7680 0 clip 3
//! place 0 7680 3840 0 clip 4
//! marker 0 Intro
//! tempo 0 92
//! tempo 15360 140 ramp
//! tempo 19200 100
//! morph 0 0 1
//! list 0 2
//! list 1 4
//! listloop
//! ```
//!
//! A clip's `id` line, first in its body, is its lasting id
//! ([`crate::ClipId`], the Arrange refactor's 1.1), and `nextclip` the
//! id the project gives next; a clip without one has none yet (an older
//! build's file), and the host gives it one as the project opens
//! ([`Project::give_clip_ids`]).
//!
//! Song blocks name their clip by its id (the Arrange refactor's 1.3):
//! `place <track> <start> <length> <offset> clip <id>`, wherever the
//! clip lives. The song's own clips ([`crate::SongClips`]) come before
//! the blocks, each once as `songclip <length> <name>` with its body
//! (its `id` line first) and `end`. That is `mpfree-clips 2`, which an
//! older build doesn't open (decision 7). An `mpfree-clips 1` part, from
//! before, still opens: its blocks named a row of their track
//! (`... slot <row>`) or carried a clip of their own (`... own <length>
//! <name>` and a body), so as it opens its clips get ids in track and
//! row order, each `slot` block names its cell's clip (none for an empty
//! cell: it plays nothing, as it did), and each `own` block's clip
//! becomes a song clip with an id of its own, in order.
//!
//! A `tempo` line is a song's tempo point (tempo ramps): its tick, its
//! tempo, and `ramp` when it glides to the next point. An older build
//! skips them and plays the project's tempo.
//!
//! A note's words after its pressure are the step's own settings, written
//! only when set (sequencer S6): `off` (the step is off), `ramp:<n>` (its
//! ratchet's velocity ramp), `tie` or `slide` ([`crate::Join`]). An older
//! build reads the note and skips them.
//!
//! A clip's `direction` line is its play direction (sequencer S6:
//! `reverse`, `pingpong` or `random`; none for forward); an older build
//! skips it, and a word it doesn't know, and plays the clip forward.
//!
//! A clip's `holds notes` line says its notes are MIDI notes, each
//! number a pitch (the instrument redesign's step 5c, [`crate::Holds`]);
//! a clip without it holds pad hits, as every clip saved before did. An
//! older build skips it, and a word it doesn't know, and plays the notes
//! as pads.
//!
//! A mutated clip's `original` lines (sequencer S6, [`crate::mutate`])
//! are the clip as it was before its first Mutate, which Restore puts
//! back: `original <length> <variation>` first, then its notes, locks,
//! lanes and points, each line as the clip's own with `original` before
//! it (their pressure is the clip's). An older build skips them all and
//! keeps the variation playing; a broken original refuses the part.
//!
//! A scene's `program <track> <program>[ bank <bank>]` lines are the
//! program changes it sends as it starts (sequencer S5), one a track, in
//! order of track; an older build skips them.
//!
//! Reading is careful: every number is checked, every clip and the whole
//! project go through [`Project::check`]-level checks, and a part that
//! doesn't hold together is refused whole (the engine keeps what it had).
//! Floats are written so they read back the same (Rust's shortest form).

use std::fmt::Write as _;
use std::str::FromStr;

use mpfree_moves::{Kind, Map, Moves, Point, Steps, Target};
use mpfree_notefx::{Slot, MAX_FX};
use mpfree_param::{ParamAddr, RelAddr};
use mpfree_transport::time::{Swing, Tick};

use crate::arrange::{Marker, MorphRows, Placement, TempoPoint};
use crate::scenes::{Follow, FollowAction, ListStep, Scene, MAX_LIST_STEPS, SCENES};
use crate::{
    Clip, ClipId, Cond, Join, LaunchGrid, Lock, LoopInput, Note, Original, Project, Quantise,
    Speed, Touch, TouchPoint, Track, TrackKind, MAX_TRACKS, ROWS,
};

/// The first line.
pub const FORMAT: &str = "mpfree-clips 2";
/// The first line of a part saved before song blocks named their clip by
/// its id (Arrange 1.3), which still opens.
pub const FORMAT_1: &str = "mpfree-clips 1";
/// Pressure points a `touch` line holds (more go on more lines).
const TOUCH_PER_LINE: usize = 64;

/// The project as text.
pub fn to_text(p: &Project) -> String {
    let mut s = String::with_capacity(4096);
    s.push_str(FORMAT);
    s.push('\n');
    if p.perform {
        s.push_str("perform\n");
    }
    if p.launch != LaunchGrid::default() {
        let _ = writeln!(s, "launch {}", launch_text(p.launch));
    }
    if p.next_track != 0 {
        let _ = writeln!(s, "nexttrack {}", p.next_track);
    }
    if p.next_clip != 0 {
        let _ = writeln!(s, "nextclip {}", p.next_clip);
    }
    for t in &p.tracks {
        let _ = writeln!(s, "track {}", one_line(&t.name));
        if t.id.is_given() {
            let _ = writeln!(s, "id {}", t.id.0);
        }
        let _ = writeln!(s, "dest {}", t.dest);
        if let Some(sw) = t.swing {
            let _ = writeln!(s, "swing {} {}", sw.grid, sw.amount);
        }
        let _ = writeln!(s, "mute {}", u8::from(t.mute));
        if t.solo {
            let _ = writeln!(s, "solo");
        }
        let _ = writeln!(s, "arp {}", u8::from(t.through_arp));
        let _ = writeln!(s, "rectouch {}", u8::from(t.record_touch));
        if let Some(grid) = t.record_quantise {
            let _ = writeln!(s, "recquant {grid}");
        }
        // The track's MIDI effects, each in its place with its id, then
        // the arp's place.
        for (place, slot) in t.note_fx.slots.iter().enumerate() {
            if let Some(slot) = slot {
                let id = t.note_fx.ids[place];
                let _ = writeln!(s, "notefx {place} {id} {}", slot.to_words());
            }
        }
        if !t.note_fx.is_empty() || t.note_fx.arp_at != 0 {
            let _ = writeln!(s, "notearp {}", t.note_fx.arp_at);
        }
        if let Some(input) = t.loops() {
            let _ = writeln!(s, "loops {}", input.word());
        }
        if !t.transpose.is_none() {
            let key = if t.transpose.in_key { " key" } else { "" };
            let _ = writeln!(s, "transpose {}{key}", t.transpose.by);
        }
        if t.follows {
            let _ = writeln!(s, "follows");
        }
        for (row, c) in t.slots.iter() {
            let _ = writeln!(s, "clip {row} {} {}", c.length, one_line(&c.name));
            clip_body(&mut s, c);
        }
    }
    for (row, sc) in p.scenes.iter().enumerate() {
        let _ = writeln!(s, "scene {row} {}", one_line(&sc.name));
        if let Some(m) = sc.mutes {
            let _ = writeln!(s, "mutes {m}");
        }
        if let Some(f) = sc.follow {
            let _ = writeln!(s, "follow {}", follow_text(f));
        }
        for (a, v) in &sc.values {
            let _ = writeln!(s, "value {a} {v}");
        }
        for tp in &sc.programs {
            let _ = writeln!(s, "program {} {}", tp.track, program_text(tp.program));
        }
    }
    let a = &p.arrangement;
    for c in a.clips.iter() {
        let _ = writeln!(s, "songclip {} {}", c.length, one_line(&c.name));
        clip_body(&mut s, c);
    }
    for pl in &a.placements {
        let _ = writeln!(
            s,
            "place {} {} {} {} clip {}",
            pl.track, pl.start, pl.length, pl.offset, pl.clip.0
        );
    }
    for m in &a.markers {
        let _ = writeln!(s, "marker {} {}", m.at, one_line(&m.name));
    }
    for t in &a.tempo {
        let ramp = if t.ramp { " ramp" } else { "" };
        let _ = writeln!(s, "tempo {} {}{ramp}", t.at, t.bpm);
    }
    for m in &a.morphs {
        let _ = writeln!(s, "morph {} {} {}", m.at, m.a, m.b);
    }
    for step in &a.list.steps {
        let _ = writeln!(s, "list {} {}", step.row, step.times);
    }
    if a.list.looped {
        s.push_str("listloop\n");
    }
    if !a.lanes.is_empty() {
        s.push_str("songlanes\n");
        lanes_text(&mut s, "", &a.lanes);
        s.push_str("end\n");
    }
    s
}

fn clip_body(s: &mut String, c: &Clip) {
    if c.id.is_given() {
        let _ = writeln!(s, "id {}", c.id.0);
    }
    if let Some(n) = c.looped {
        let _ = writeln!(s, "loop {n}");
    }
    if c.speed != Speed::NORMAL {
        let _ = writeln!(s, "speed {} {}", c.speed.num, c.speed.den);
    }
    if let Some(sw) = c.swing {
        let _ = writeln!(s, "swing {} {}", sw.grid, sw.amount);
    }
    if let Some(q) = c.quantise {
        let _ = writeln!(s, "quant {} {}", q.grid, q.strength);
    }
    if let Some(f) = c.follow {
        let _ = writeln!(s, "follow {}", follow_text(f));
    }
    if let Some(g) = c.launch {
        let _ = writeln!(s, "launch {}", launch_text(g));
    }
    if c.legato {
        s.push_str("legato\n");
    }
    if c.once {
        s.push_str("once\n");
    }
    if let Some(p) = c.program {
        let _ = writeln!(s, "program {}", program_text(p));
    }
    if c.direction != crate::Direction::Forward {
        let _ = writeln!(s, "direction {}", c.direction.word());
    }
    if c.holds != crate::Holds::Hits {
        let _ = writeln!(s, "holds {}", c.holds.word());
    }
    // Pressure first: the notes name runs of it. Only what the notes and
    // the original's notes name is written, each run renumbered (B4a):
    // values a note took out no longer names stay out of the file.
    let originals = c.original.iter().flat_map(|o| &o.notes);
    let keep = crate::TouchKeep::of(
        c.touch.len(),
        c.notes.iter().chain(originals).map(|n| n.touch),
    );
    let kept: Vec<TouchPoint> = keep.kept_of(&c.touch).collect();
    for chunk in kept.chunks(TOUCH_PER_LINE) {
        s.push_str("touch");
        for p in chunk {
            let _ = write!(s, " {}:{}", p.at, p.value);
        }
        s.push('\n');
    }
    notes_text(s, "", &c.notes, &keep);
    locks_text(s, "", &c.locks);
    lanes_text(s, "", &c.moves);
    // What Restore puts back (S6's mutate): after everything else, each
    // line marked, so an older build skips them all.
    if let Some(o) = &c.original {
        let _ = writeln!(s, "original {} {}", o.length, o.variation);
        notes_text(s, "original ", &o.notes, &keep);
        locks_text(s, "original ", &o.locks);
        lanes_text(s, "original ", &o.moves);
    }
    s.push_str("end\n");
}

/// A clip's notes, a line each, every line starting with `prefix`, each
/// pressure run as `keep` renumbers it.
fn notes_text(s: &mut String, prefix: &str, notes: &[Note], keep: &crate::TouchKeep) {
    for n in notes {
        let touch = keep.run(n.touch);
        let _ = write!(
            s,
            "{prefix}note {} {} {} {} {} {} {} {} {}",
            n.tick,
            n.len,
            n.note,
            n.velocity,
            n.chance,
            cond_text(n.cond),
            n.ratchet,
            touch.first,
            touch.len
        );
        if !n.on {
            s.push_str(" off");
        }
        if n.ramp != 0 {
            let _ = write!(s, " ramp:{}", n.ramp);
        }
        if n.join != Join::Attack {
            let _ = write!(s, " {}", n.join.word());
        }
        s.push('\n');
    }
}

/// A clip's locks, a line each, every line starting with `prefix`.
fn locks_text(s: &mut String, prefix: &str, locks: &[Lock]) {
    for l in locks {
        let sound = l.sound.map_or("-".to_string(), |x| x.to_string());
        let _ = writeln!(s, "{prefix}lock {} {sound} {} {}", l.tick, l.addr, l.value);
    }
}

/// Lanes and their points, every line starting with `prefix`.
fn lanes_text(s: &mut String, prefix: &str, moves: &Moves) {
    for lane in moves.lanes() {
        let target = match lane.target {
            Target::Track(r) => format!("t:{r}"),
            Target::Fixed(a) => format!("f:{a}"),
        };
        let steps = match lane.map.steps {
            Steps::None => "n",
            Steps::Whole => "w",
            Steps::Toggle => "t",
        };
        let _ = writeln!(
            s,
            "{prefix}lane {target} {} {} {} {steps}",
            lane.map.min,
            lane.map.max,
            u8::from(lane.map.log)
        );
        for p in &lane.points {
            let kind = match p.kind {
                Kind::Ramp => "r",
                Kind::Step => "s",
                Kind::End => "e",
            };
            let _ = writeln!(s, "{prefix}point {} {} {kind}", p.tick, p.u);
        }
    }
}

fn cond_text(c: Cond) -> String {
    match c {
        Cond::Always => "-".to_string(),
        c => c.to_string(),
    }
}

fn parse_cond(w: &str) -> Option<Cond> {
    Some(match w {
        "-" => Cond::Always,
        "FILL" => Cond::Fill,
        "!FILL" => Cond::NotFill,
        "PRE" => Cond::Pre,
        "!PRE" => Cond::NotPre,
        "NEI" => Cond::Nei,
        "!NEI" => Cond::NotNei,
        "1ST" => Cond::First,
        "!1ST" => Cond::NotFirst,
        _ => {
            let (a, b) = w.split_once(':')?;
            let c = Cond::Every {
                a: a.parse().ok()?,
                b: b.parse().ok()?,
            };
            return c.ok().then_some(c);
        }
    })
}

const ACTIONS: [(FollowAction, &str); 9] = [
    (FollowAction::Stay, "stay"),
    (FollowAction::Again, "again"),
    (FollowAction::Next, "next"),
    (FollowAction::Previous, "previous"),
    (FollowAction::First, "first"),
    (FollowAction::Last, "last"),
    (FollowAction::Any, "any"),
    (FollowAction::Other, "other"),
    (FollowAction::Stop, "stop"),
];

fn action_text(a: FollowAction) -> &'static str {
    ACTIONS.iter().find(|x| x.0 == a).map_or("stay", |x| x.1)
}

fn parse_action(w: &str) -> Option<FollowAction> {
    ACTIONS.iter().find(|x| x.1 == w).map(|x| x.0)
}

/// A launch quantise as a line's words: `now`, `ticks 240`, `bars 2`.
fn launch_text(g: LaunchGrid) -> String {
    match g {
        LaunchGrid::Now => "now".to_string(),
        LaunchGrid::Ticks(t) => format!("ticks {t}"),
        LaunchGrid::Bars(n) => format!("bars {n}"),
        // Never a setting (`LaunchGrid::ok` refuses it): written as at
        // once, should one ever reach here.
        LaunchGrid::At(_) => "now".to_string(),
    }
}

fn launch<'a>(w: &mut impl Iterator<Item = &'a str>) -> Option<LaunchGrid> {
    let g = match w.next()? {
        "now" => LaunchGrid::Now,
        "ticks" => LaunchGrid::Ticks(num(w.next())?),
        "bars" => LaunchGrid::Bars(num(w.next())?),
        _ => return None,
    };
    g.ok().then_some(g)
}

/// `program 4`, or `program 4 bank 2`: the rest of a clip's `program`
/// line (after the track on a scene's), in range
/// ([`crate::ClipProgram::ok`]).
fn program<'a>(w: &mut impl Iterator<Item = &'a str>) -> Option<crate::ClipProgram> {
    let program = num(w.next())?;
    let bank = match w.next() {
        None => None,
        Some("bank") => Some(num(w.next())?),
        Some(_) => return None,
    };
    let p = crate::ClipProgram { program, bank };
    p.ok().then_some(p)
}

/// The words after `program` (and a scene's track): `4`, or `4 bank 2`.
fn program_text(p: crate::ClipProgram) -> String {
    match p.bank {
        Some(b) => format!("{} bank {b}", p.program),
        None => p.program.to_string(),
    }
}

fn follow_text(f: Follow) -> String {
    format!(
        "{} {} {} {}",
        f.bars,
        action_text(f.action),
        action_text(f.other),
        f.chance
    )
}

/// One line of text: line breaks become spaces.
fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

/// Where the lines being read go.
enum At {
    /// Before any track, or after a scene's or a placement's lines.
    Top,
    /// The last track's settings and clips.
    Track,
    /// A clip's body: into the last track's slot, or a placement's own.
    Clip(Box<Clip>, ClipHome),
    Scene,
    /// The song's own lanes, until `end`.
    SongLanes(Moves),
}

enum ClipHome {
    Slot(usize),
    /// The song's own clips.
    Song,
    /// An `mpfree-clips 1` block's own clip: the block waiting for it.
    Own(usize, Tick, Tick, Tick),
}

/// An `mpfree-clips 1` part's blocks, by their place in the song's list,
/// to name their clips once every clip is read.
#[derive(Default)]
struct OldBlocks {
    /// A block of row `.1` of its track.
    rows: Vec<(usize, usize)>,
    /// A block with a clip of its own.
    own: Vec<(usize, Clip)>,
}

/// The lanes being read: the lane open now.
struct LaneAt {
    target: Target,
    map: Map,
    points: Vec<Point>,
}

/// Reads a project from text [`to_text`] made (or a newer build's).
pub fn from_text(text: &str) -> Result<Project, String> {
    let mut lines = text.lines().enumerate();
    match lines.next() {
        Some((_, l)) if l.trim() == FORMAT || l.trim() == FORMAT_1 => {}
        _ => return Err(format!("not an {FORMAT} part")),
    }
    let mut p = Project::new();
    let mut old = OldBlocks::default();
    let mut at = At::Top;
    let mut lane: Option<LaneAt> = None;
    // The lane open in a clip's original.
    let mut orig_lane: Option<LaneAt> = None;
    for (i, raw) in lines {
        let line = raw.trim_end();
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let bad = |what: &str| format!("line {}: {what}: {line}", i + 1);
        let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
        let mut w = rest.split_whitespace();
        // Lane points belong to the lane open now, in a clip or the song.
        if word == "point" {
            let l = lane.as_mut().ok_or_else(|| bad("a point outside a lane"))?;
            point(l, &mut w).map_err(bad)?;
            continue;
        }
        // A clip's original (S6's mutate) closes its last lane at the
        // clip's end.
        if word == "end" {
            if let (Some(l), At::Clip(c, _)) = (orig_lane.take(), &mut at) {
                let o = c
                    .original
                    .as_mut()
                    .ok_or_else(|| bad("a lane outside a clip"))?;
                put_lane(&mut o.moves, l).map_err(|e| bad(&e))?;
            }
        }
        if word == "lane" || word == "end" {
            if let Some(l) = lane.take() {
                let moves = match &mut at {
                    At::Clip(c, _) => &mut c.moves,
                    At::SongLanes(m) => m,
                    _ => return Err(bad("a lane outside a clip")),
                };
                put_lane(moves, l).map_err(|e| bad(&e))?;
            }
        }
        match (word, &mut at) {
            ("track", _) => {
                close(&mut p, &mut at).map_err(|e| bad(&e))?;
                if p.tracks.len() >= MAX_TRACKS {
                    return Err(bad("too many tracks"));
                }
                let dest = p.tracks.len() as u16;
                p.tracks.push(Track::new(rest, dest));
                at = At::Track;
            }
            ("dest", At::Track) => last(&mut p).dest = num(w.next()).ok_or_else(|| bad("dest"))?,
            ("id", At::Track) => {
                let id: u32 = num(w.next()).filter(|&n| n >= 1).ok_or_else(|| bad("id"))?;
                last(&mut p).id = crate::TrackId(id);
            }
            ("nexttrack", At::Top) => {
                p.next_track = num(w.next())
                    .filter(|&n| n >= 1)
                    .ok_or_else(|| bad("nexttrack"))?
            }
            ("id", At::Clip(c, _)) => {
                let id: u32 = num(w.next()).filter(|&n| n >= 1).ok_or_else(|| bad("id"))?;
                c.id = crate::ClipId(id);
            }
            ("nextclip", At::Top) => {
                p.next_clip = num(w.next())
                    .filter(|&n| n >= 1)
                    .ok_or_else(|| bad("nextclip"))?
            }
            ("swing", At::Track) => {
                last(&mut p).swing = Some(swing(&mut w).ok_or_else(|| bad("swing"))?)
            }
            ("mute", At::Track) => last(&mut p).mute = flag(w.next()).ok_or_else(|| bad("mute"))?,
            ("solo", At::Track) => last(&mut p).solo = true,
            ("transpose", At::Track) => {
                let by: i8 = num(w.next()).ok_or_else(|| bad("transpose"))?;
                let in_key = match w.next() {
                    None => false,
                    Some("key") => true,
                    Some(_) => return Err(bad("transpose")),
                };
                let t = crate::Transpose { by, in_key };
                if !t.ok() {
                    return Err(bad("transpose"));
                }
                last(&mut p).transpose = t
            }
            ("follows", At::Track) => last(&mut p).follows = true,
            ("arp", At::Track) => {
                last(&mut p).through_arp = flag(w.next()).ok_or_else(|| bad("arp"))?
            }
            ("rectouch", At::Track) => {
                last(&mut p).record_touch = flag(w.next()).ok_or_else(|| bad("rectouch"))?
            }
            ("recquant", At::Track) => {
                let grid: u32 = num(w.next()).ok_or_else(|| bad("recquant"))?;
                if Quantise::new(grid, 100).is_none() {
                    return Err(bad("recquant"));
                }
                last(&mut p).record_quantise = Some(grid)
            }
            ("notefx", At::Track) => {
                let place: usize = num(w.next()).ok_or_else(|| bad("place"))?;
                let id: u16 = num(w.next()).ok_or_else(|| bad("notefx id"))?;
                let fx = &mut last(&mut p).note_fx;
                if place >= MAX_FX || fx.slots[place].is_some() {
                    return Err(bad("place"));
                }
                // Each effect's id is its own on the track.
                if id == 0 || fx.place_of(id).is_some() {
                    return Err(bad("notefx id"));
                }
                let slot = Slot::from_words(nth_rest(rest, 2)).map_err(|e| bad(&e))?;
                fx.slots[place] = Some(slot);
                fx.ids[place] = id;
            }
            ("notearp", At::Track) => {
                let at: u8 = num(w.next()).ok_or_else(|| bad("notearp"))?;
                if usize::from(at) > MAX_FX {
                    return Err(bad("notearp"));
                }
                last(&mut p).note_fx.arp_at = at;
            }
            ("loops", At::Track) => {
                let input = w.next().and_then(LoopInput::from_word);
                last(&mut p).kind = TrackKind::Loops(input.ok_or_else(|| bad("loops"))?)
            }
            ("clip", At::Track) => {
                let slot: usize = num(w.next()).ok_or_else(|| bad("slot"))?;
                let length: Tick = num(w.next()).ok_or_else(|| bad("length"))?;
                if slot >= ROWS || last(&mut p).slots[slot].is_some() {
                    return Err(bad("slot"));
                }
                let name = nth_rest(rest, 2);
                at = At::Clip(Box::new(Clip::new(name, length)), ClipHome::Slot(slot));
            }
            ("speed", At::Clip(c, _)) => {
                let (a, b) = (num(w.next()), num(w.next()));
                c.speed = a
                    .zip(b)
                    .and_then(|(a, b)| Speed::new(a, b))
                    .ok_or_else(|| bad("speed"))?;
            }
            ("swing", At::Clip(c, _)) => c.swing = Some(swing(&mut w).ok_or_else(|| bad("swing"))?),
            ("loop", At::Clip(c, _)) => c.looped = Some(num(w.next()).ok_or_else(|| bad("loop"))?),
            ("quant", At::Clip(c, _)) => {
                let (g, s) = (num(w.next()), num(w.next()));
                c.quantise = Some(
                    g.zip(s)
                        .and_then(|(g, s)| Quantise::new(g, s))
                        .ok_or_else(|| bad("quantise"))?,
                );
            }
            ("follow", At::Clip(c, _)) => {
                c.follow = Some(follow(&mut w).ok_or_else(|| bad("follow"))?)
            }
            ("launch", At::Clip(c, _)) => {
                c.launch = Some(launch(&mut w).ok_or_else(|| bad("launch"))?)
            }
            ("legato", At::Clip(c, _)) => c.legato = true,
            ("once", At::Clip(c, _)) => c.once = true,
            ("program", At::Clip(c, _)) => {
                c.program = Some(program(&mut w).ok_or_else(|| bad("program"))?)
            }
            // A newer build's direction plays forward here.
            ("direction", At::Clip(c, _)) => {
                let word = w.next().ok_or_else(|| bad("direction"))?;
                c.direction = crate::Direction::from_word(word).unwrap_or_default();
            }
            // A newer build's kind of notes plays as pad hits here.
            ("holds", At::Clip(c, _)) => {
                let word = w.next().ok_or_else(|| bad("holds"))?;
                c.holds = crate::Holds::from_word(word).unwrap_or_default();
            }
            ("launch", At::Top) => p.launch = launch(&mut w).ok_or_else(|| bad("launch"))?,
            ("touch", At::Clip(c, _)) => {
                for x in w {
                    let (a, v) = x.split_once(':').ok_or_else(|| bad("touch"))?;
                    let point = TouchPoint {
                        at: a.parse().map_err(|_| bad("touch"))?,
                        value: v.parse().map_err(|_| bad("touch"))?,
                    };
                    if c.touch.len() >= crate::MAX_TOUCH_POINTS || point.value > 127 {
                        return Err(bad("touch"));
                    }
                    c.touch.push(point);
                }
            }
            ("note", At::Clip(c, _)) => {
                let n = note(&mut w).ok_or_else(|| bad("note"))?;
                c.add_note(n).map_err(|e| bad(&format!("{e:?}")))?;
            }
            ("lock", At::Clip(c, _)) => {
                let l = lock(&mut w).ok_or_else(|| bad("lock"))?;
                c.add_lock(l).map_err(|e| bad(&format!("{e:?}")))?;
            }
            // The clip as it was before its first Mutate (S6): its length
            // and variation open it, then its notes, locks and lanes,
            // checked with the clip at its end.
            ("original", At::Clip(c, _)) => match w.next() {
                Some("note") => {
                    let n = note(&mut w).ok_or_else(|| bad("note"))?;
                    let o = c.original.as_mut().ok_or_else(|| bad("original"))?;
                    if o.notes.len() >= crate::MAX_NOTES {
                        return Err(bad("too many notes"));
                    }
                    o.notes.push(n);
                }
                Some("lock") => {
                    let l = lock(&mut w).ok_or_else(|| bad("lock"))?;
                    let o = c.original.as_mut().ok_or_else(|| bad("original"))?;
                    if o.locks.len() >= crate::MAX_LOCKS {
                        return Err(bad("too many locks"));
                    }
                    o.locks.push(l);
                }
                Some("lane") => {
                    let o = c.original.as_mut().ok_or_else(|| bad("original"))?;
                    if let Some(l) = orig_lane.take() {
                        put_lane(&mut o.moves, l).map_err(|e| bad(&e))?;
                    }
                    orig_lane = Some(lane_head(&mut w).ok_or_else(|| bad("lane"))?);
                }
                Some("point") => {
                    let l = orig_lane
                        .as_mut()
                        .ok_or_else(|| bad("a point outside a lane"))?;
                    point(l, &mut w).map_err(bad)?;
                }
                Some(x) => {
                    let length: Tick = x.parse().map_err(|_| bad("length"))?;
                    let variation: u32 = num(w.next()).ok_or_else(|| bad("variation"))?;
                    if c.original.is_some() {
                        return Err(bad("a second original"));
                    }
                    c.original = Some(Box::new(Original {
                        length,
                        notes: Vec::new(),
                        locks: Vec::new(),
                        moves: Moves::new(),
                        variation,
                    }));
                }
                None => return Err(bad("original")),
            },
            ("lane", At::Clip(..) | At::SongLanes(_)) => {
                lane = Some(lane_head(&mut w).ok_or_else(|| bad("lane"))?);
            }
            ("end", At::Clip(..)) => {
                let At::Clip(c, home) = std::mem::replace(&mut at, At::Top) else {
                    unreachable!()
                };
                c.check().map_err(|e| bad(&format!("clip {e:?}")))?;
                at = match home {
                    ClipHome::Slot(slot) => {
                        last(&mut p).slots[slot] = Some(*c);
                        At::Track
                    }
                    ClipHome::Song => {
                        p.arrangement
                            .clips
                            .keep(*c)
                            .map_err(|_| bad("a song clip with no id, or one taken"))?;
                        At::Top
                    }
                    ClipHome::Own(track, start, length, offset) => {
                        let mut pl = Placement::new(track, ClipId::NONE, start, length);
                        pl.offset = offset;
                        old.own.push((p.arrangement.placements.len(), *c));
                        p.arrangement.placements.push(pl);
                        At::Top
                    }
                };
            }
            ("end", At::SongLanes(_)) => {
                let At::SongLanes(m) = std::mem::replace(&mut at, At::Top) else {
                    unreachable!()
                };
                p.arrangement.lanes = m;
            }
            ("scene", At::Top | At::Track | At::Scene) => {
                let row: usize = num(w.next()).ok_or_else(|| bad("row"))?;
                if row >= SCENES {
                    return Err(bad("row"));
                }
                p.scene_mut(row).ok_or_else(|| bad("row"))?.name = nth_rest(rest, 1).to_string();
                at = At::Scene;
            }
            ("mutes", At::Scene) => {
                last_scene(&mut p).mutes = Some(num(w.next()).ok_or_else(|| bad("mutes"))?)
            }
            ("follow", At::Scene) => {
                last_scene(&mut p).follow = Some(follow(&mut w).ok_or_else(|| bad("follow"))?)
            }
            ("value", At::Scene) => {
                let a = w.next().and_then(|x| ParamAddr::from_str(x).ok());
                let v: Option<f32> = num(w.next());
                let (a, v) = a.zip(v).ok_or_else(|| bad("value"))?;
                last_scene(&mut p)
                    .set_value(a, v)
                    .map_err(|e| bad(&format!("{e:?}")))?;
            }
            ("program", At::Scene) => {
                let track: usize = num(w.next()).ok_or_else(|| bad("program"))?;
                let to = program(&mut w).ok_or_else(|| bad("program"))?;
                let sc = last_scene(&mut p);
                if sc.program(track).is_some() {
                    return Err(bad("program"));
                }
                sc.set_program(track, Some(to))
                    .map_err(|_| bad("program"))?;
            }
            ("place", At::Top | At::Track | At::Scene) => {
                let track: usize = num(w.next()).ok_or_else(|| bad("track"))?;
                let start: Tick = num(w.next()).ok_or_else(|| bad("start"))?;
                let length: Tick = num(w.next()).ok_or_else(|| bad("length"))?;
                let offset: Tick = num(w.next()).ok_or_else(|| bad("offset"))?;
                let mut pl = Placement::new(track, ClipId::NONE, start, length);
                pl.offset = offset;
                match w.next() {
                    Some("clip") => {
                        pl.clip = ClipId(num(w.next()).ok_or_else(|| bad("clip"))?);
                        p.arrangement.placements.push(pl);
                        at = At::Top;
                    }
                    Some("slot") => {
                        let slot: usize = num(w.next()).ok_or_else(|| bad("slot"))?;
                        if slot >= ROWS {
                            return Err(bad("slot"));
                        }
                        old.rows.push((p.arrangement.placements.len(), slot));
                        p.arrangement.placements.push(pl);
                        at = At::Top;
                    }
                    Some("own") => {
                        let clip_len: Tick = num(w.next()).ok_or_else(|| bad("length"))?;
                        let name = nth_rest(rest, 6);
                        at = At::Clip(
                            Box::new(Clip::new(name, clip_len)),
                            ClipHome::Own(track, start, length, offset),
                        );
                    }
                    _ => return Err(bad("source")),
                }
            }
            ("marker", At::Top | At::Track | At::Scene) => {
                let at_tick: Tick = num(w.next()).ok_or_else(|| bad("tick"))?;
                p.arrangement.markers.push(Marker {
                    at: at_tick,
                    name: nth_rest(rest, 1).to_string(),
                });
                at = At::Top;
            }
            ("tempo", At::Top | At::Track | At::Scene) => {
                let t = (|| {
                    let at: Tick = num(w.next())?;
                    let bpm: f64 = num(w.next())?;
                    // Words a newer build adds after it are skipped.
                    let ramp = w.any(|x| x == "ramp");
                    Some(TempoPoint { at, bpm, ramp })
                })()
                .ok_or_else(|| bad("tempo"))?;
                p.arrangement.tempo.push(t);
                at = At::Top;
            }
            ("morph", At::Top | At::Track | At::Scene) => {
                let m = (|| {
                    Some(MorphRows {
                        at: num(w.next())?,
                        a: num(w.next())?,
                        b: num(w.next())?,
                    })
                })()
                .ok_or_else(|| bad("morph"))?;
                p.arrangement.morphs.push(m);
                at = At::Top;
            }
            ("list", At::Top | At::Track | At::Scene) => {
                let step = (|| {
                    Some(ListStep {
                        row: num(w.next())?,
                        times: num(w.next())?,
                    })
                })()
                .ok_or_else(|| bad("list"))?;
                if p.arrangement.list.steps.len() >= MAX_LIST_STEPS {
                    return Err(bad("list"));
                }
                p.arrangement.list.steps.push(step);
                at = At::Top;
            }
            ("listloop", At::Top | At::Track | At::Scene) => {
                p.arrangement.list.looped = true;
                at = At::Top;
            }
            ("songclip", At::Top | At::Track | At::Scene) => {
                let length: Tick = num(w.next()).ok_or_else(|| bad("length"))?;
                let name = nth_rest(rest, 1);
                at = At::Clip(Box::new(Clip::new(name, length)), ClipHome::Song);
            }
            ("songlanes", At::Top | At::Track | At::Scene) => at = At::SongLanes(Moves::new()),
            ("perform", At::Top) => p.perform = true,
            // A newer build's line: skipped.
            _ if !matches!(word, "end" | "lane") => {}
            _ => return Err(bad("out of place")),
        }
    }
    if lane.is_some() || matches!(at, At::Clip(..) | At::SongLanes(_)) {
        return Err("the part ends inside a clip".to_string());
    }
    name_old_blocks(&mut p, old)?;
    p.check()
        .map_err(|e| format!("the clips don't hold together: {e:?}"))?;
    Ok(p)
}

/// An `mpfree-clips 1` part's blocks name their clips by id: the clips
/// get ids first (in track and row order), then each block of a row names
/// its cell's clip (none for an empty cell: it plays nothing, as it did)
/// and each block's own clip becomes a song clip with the next id, in
/// order of the blocks.
fn name_old_blocks(p: &mut Project, old: OldBlocks) -> Result<(), String> {
    if old.rows.is_empty() && old.own.is_empty() {
        return Ok(());
    }
    p.give_clip_ids();
    for (k, row) in old.rows {
        let track = p.arrangement.placements[k].track;
        p.arrangement.placements[k].clip = p.clip(track, row).map_or(ClipId::NONE, |c| c.id);
    }
    for (k, mut c) in old.own {
        c.id = p.new_clip_id();
        p.arrangement.placements[k].clip = c.id;
        p.arrangement
            .clips
            .keep(c)
            .map_err(|e| format!("a block's own clip: {e:?}"))?;
    }
    Ok(())
}

/// Nothing to close at a track line but a scene (scenes come after
/// tracks, so a track after one is out of order).
fn close(p: &mut Project, at: &mut At) -> Result<(), String> {
    match at {
        At::Top | At::Track => Ok(()),
        _ if p.scenes.is_empty() && p.arrangement.placements.is_empty() => Ok(()),
        _ => Err("a track after the scenes or the song".to_string()),
    }
}

fn last(p: &mut Project) -> &mut Track {
    p.tracks.last_mut().expect("a track line came first")
}

fn last_scene(p: &mut Project) -> &mut Scene {
    let row = p.scenes.len() - 1;
    &mut p.scenes[row]
}

fn put_lane(moves: &mut Moves, l: LaneAt) -> Result<(), String> {
    if l.points.is_empty() {
        return Ok(());
    }
    moves
        .reserve(l.target, l.map, l.points.len())
        .and_then(|_| moves.restore(l.target, l.map, 0, u32::MAX, &l.points))
        .map_err(|e| format!("lane {e:?}"))
}

fn lane_head<'a>(w: &mut impl Iterator<Item = &'a str>) -> Option<LaneAt> {
    let t = w.next()?;
    let target = if let Some(r) = t.strip_prefix("t:") {
        Target::Track(RelAddr::from_str(r).ok()?)
    } else {
        Target::Fixed(ParamAddr::from_str(t.strip_prefix("f:")?).ok()?)
    };
    let min: f32 = num(w.next())?;
    let max: f32 = num(w.next())?;
    let log = flag(w.next())?;
    let steps = match w.next()? {
        "n" => Steps::None,
        "w" => Steps::Whole,
        "t" => Steps::Toggle,
        _ => return None,
    };
    if !min.is_finite() || !max.is_finite() {
        return None;
    }
    Some(LaneAt {
        target,
        map: Map {
            min,
            max,
            log,
            steps,
        },
        points: Vec::new(),
    })
}

fn swing<'a>(w: &mut impl Iterator<Item = &'a str>) -> Option<Swing> {
    Swing::new(num(w.next())?, num(w.next())?)
}

fn follow<'a>(w: &mut impl Iterator<Item = &'a str>) -> Option<Follow> {
    let f = Follow {
        bars: num(w.next())?,
        action: parse_action(w.next()?)?,
        other: parse_action(w.next()?)?,
        chance: num(w.next())?,
    };
    f.check().ok().map(|_| f)
}

/// A note line's words after its pressure: the step's own settings
/// (`off`, `ramp:<n>`, `tie`, `slide`). A ramp out of range or not a
/// number refuses the line (None); a word this build doesn't know is
/// skipped, as unknown lines are, so a newer build's notes still open.
/// A `note` line's words after `note`.
fn note<'a>(w: &mut impl Iterator<Item = &'a str>) -> Option<Note> {
    let n = Note {
        tick: num(w.next())?,
        len: num(w.next())?,
        note: num(w.next())?,
        velocity: num(w.next())?,
        chance: num(w.next())?,
        cond: parse_cond(w.next()?)?,
        ratchet: num(w.next())?,
        touch: Touch {
            first: num(w.next())?,
            len: num(w.next())?,
        },
        on: true,
        ramp: 0,
        join: Join::Attack,
    };
    step_words(n, w)
}

/// A `lock` line's words after `lock`.
fn lock<'a>(w: &mut impl Iterator<Item = &'a str>) -> Option<Lock> {
    let tick = num(w.next())?;
    let sound = match w.next()? {
        "-" => None,
        x => Some(x.parse().ok()?),
    };
    let addr = RelAddr::from_str(w.next()?).ok()?;
    Some(Lock::new(tick, sound, addr, num(w.next())?))
}

/// A `point` line's words after `point`, onto lane `l`; what is wrong
/// with them if they don't hold.
fn point<'a>(l: &mut LaneAt, w: &mut impl Iterator<Item = &'a str>) -> Result<(), &'static str> {
    let tick: u32 = num(w.next()).ok_or("tick")?;
    let u: f32 = num(w.next()).ok_or("value")?;
    let kind = match w.next() {
        Some("r") => Kind::Ramp,
        Some("s") => Kind::Step,
        Some("e") => Kind::End,
        _ => return Err("kind"),
    };
    if !u.is_finite() || l.points.last().is_some_and(|q| q.tick >= tick) {
        return Err("point out of order");
    }
    l.points.push(Point { tick, u, kind });
    Ok(())
}

fn step_words<'a>(mut n: Note, w: impl Iterator<Item = &'a str>) -> Option<Note> {
    for word in w {
        match word {
            "off" => n.on = false,
            "tie" => n.join = Join::Tie,
            "slide" => n.join = Join::Slide,
            x => {
                if let Some(r) = x.strip_prefix("ramp:") {
                    n.ramp = r.parse().ok()?;
                }
            }
        }
    }
    Some(n)
}

fn num<T: FromStr>(w: Option<&str>) -> Option<T> {
    w?.parse().ok()
}

fn flag(w: Option<&str>) -> Option<bool> {
    match w? {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}

/// The text after the first `n` words (a name with its spaces kept).
fn nth_rest(rest: &str, n: usize) -> &str {
    let mut s = rest;
    for _ in 0..n {
        s = s.trim_start();
        s = s.find(' ').map_or("", |k| &s[k + 1..]);
    }
    s
}

#[cfg(test)]
mod track_id_tests {
    use super::*;

    /// Instrument redesign step 2c: each track's lasting id and the next
    /// id are kept through the text; a project saved before tracks had
    /// ids reads with none (the app gives them as it opens it), and an
    /// id that doesn't read is refused.
    #[test]
    fn track_ids_and_the_next_id_come_back() {
        let mut p = Project::new();
        for i in 0..3 {
            p.tracks.push(Track::new(&format!("Track {}", i + 1), i));
        }
        let old = to_text(&p);
        assert!(!old.contains("\nid ") && !old.contains("nexttrack"));
        assert_eq!(from_text(&old).unwrap(), p, "no ids, as saved");
        assert_eq!(p.give_track_ids(), 3);
        p.tracks.remove(1);
        let text = to_text(&p);
        assert!(text.contains("nexttrack 4\n"), "{text}");
        assert!(text.contains("track Track 3\nid 3\n"), "{text}");
        let back = from_text(&text).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.track_with(crate::TrackId(3)), Some(1));
        for bad in ["id 0", "id x"] {
            let t = text.replace("id 3", bad);
            assert!(from_text(&t).is_err(), "{bad}");
        }
        assert!(from_text(&text.replace("nexttrack 4", "nexttrack 0")).is_err());
    }

    /// Arrange 1.2: a clip and a scene in row 40, past the grid's 16, are
    /// kept through the text; a row past the 64 a project keeps is
    /// refused.
    #[test]
    fn a_clip_and_a_scene_in_row_40_come_back() {
        let mut p = Project::new();
        p.tracks.push(Track::new("Drums", 0));
        p.tracks[0].slots[40] = Some(Clip::new("Far", 3840));
        p.give_clip_ids();
        p.scene_mut(40).unwrap().name = "Bank C".into();
        p.arrangement
            .place(Placement::of(&p.tracks, 0, 40, 0, 3840))
            .unwrap();
        let text = to_text(&p);
        assert!(text.contains("clip 40 3840 Far\nid 1\n"), "{text}");
        assert!(text.contains("scene 40 Bank C\n"), "{text}");
        assert!(text.contains("place 0 0 3840 0 clip 1\n"), "{text}");
        assert_eq!(from_text(&text).unwrap(), p);
        for bad in [
            text.replace("clip 40 ", "clip 64 "),
            text.replace("scene 40 ", "scene 64 "),
        ] {
            assert!(from_text(&bad).is_err(), "{bad}");
        }
    }

    /// Arrange 1.1: each clip's lasting id, a loop cell's too, and the
    /// next id are kept through the text; a project saved before clips had
    /// ids reads with none (the app gives them as it opens it); and an id
    /// that doesn't read is refused.
    #[test]
    fn clip_ids_and_the_next_id_come_back() {
        let mut p = Project::new();
        p.tracks.push(Track::new("Drums", 0));
        p.tracks.push(Track::new_loops("Loops", 1));
        p.tracks[0].slots[0] = Some(Clip::new("Beat", 3840));
        p.tracks[0].slots[2] = Some(Clip::new("Fill", 3840));
        p.tracks[1].slots[1] = Some(Clip::new_loop(0, 3840));
        let old = to_text(&p);
        assert!(!old.contains("\nid ") && !old.contains("nextclip"));
        assert_eq!(from_text(&old).unwrap(), p, "no ids, as saved");
        assert_eq!(p.give_clip_ids(), 3);
        p.tracks[0].slots[0] = None;
        let text = to_text(&p);
        assert!(text.contains("nextclip 4\n"), "{text}");
        assert!(text.contains("clip 2 3840 Fill\nid 2\n"), "{text}");
        assert!(
            text.contains("clip 1 3840 Loop 1\nid 3\nloop 0\n"),
            "{text}"
        );
        let back = from_text(&text).unwrap();
        assert_eq!(back, p);
        assert_eq!(
            back.find_clip(crate::ClipId(3)),
            Some(crate::ClipAt::Cell { track: 1, row: 1 })
        );
        for bad in ["id 0", "id x"] {
            let t = text.replace("id 2", bad);
            assert!(from_text(&t).is_err(), "{bad}");
        }
        assert!(from_text(&text.replace("nextclip 4", "nextclip 0")).is_err());
    }

    /// Arrange 1.3: blocks name their clip by id, the song's own clips
    /// are written once each, and the part is `mpfree-clips 2`; a song
    /// clip without an id, or with one taken, is refused, and a block
    /// naming a clip that isn't there reads (it plays nothing).
    #[test]
    fn blocks_name_their_clips_and_the_songs_own_clips_come_back() {
        let mut p = Project::new();
        p.tracks.push(Track::new("Drums", 0));
        p.tracks.push(Track::new("Bass", 1));
        p.tracks[0].slots[0] = Some(Clip::new("Beat", 3840));
        p.tracks[1].slots[2] = Some(Clip::new("Line", 7680));
        p.give_clip_ids();
        // The drums' beat on its own lane and on the bass's.
        p.arrangement
            .place(Placement::of(&p.tracks, 0, 0, 0, 7680))
            .unwrap();
        let mut across = Placement::of(&p.tracks, 0, 0, 7680, 3840);
        across.track = 1;
        p.arrangement.place(across).unwrap();
        p.arrangement
            .place(Placement::of(&p.tracks, 1, 2, 0, 7680))
            .unwrap();
        // One block made unique: its copy among the song's clips.
        let k = p.arrangement.at(1, 0).unwrap();
        assert_eq!(p.make_unique(k), Ok(crate::ClipId(3)));
        let text = to_text(&p);
        assert!(text.starts_with("mpfree-clips 2\n"), "{text}");
        assert!(text.contains("\nsongclip 7680 Line\nid 3\nend\n"), "{text}");
        assert!(text.contains("\nplace 0 0 7680 0 clip 1\n"), "{text}");
        assert!(text.contains("\nplace 1 7680 3840 0 clip 1\n"), "{text}");
        assert!(text.contains("\nplace 1 0 7680 0 clip 3\n"), "{text}");
        let back = from_text(&text).unwrap();
        assert_eq!(back, p);
        assert_eq!(to_text(&back), text, "stable");
        assert_eq!(back.find_clip(crate::ClipId(3)), Some(crate::ClipAt::Song));
        // Refused: a song clip with no id, or one a cell has.
        for bad in [
            text.replace("songclip 7680 Line\nid 3\n", "songclip 7680 Line\n"),
            text.replace("songclip 7680 Line\nid 3\n", "songclip 7680 Line\nid 1\n"),
        ] {
            assert!(from_text(&bad).is_err(), "{bad}");
        }
        // A block naming no clip there reads, and finds none.
        let gone = from_text(&text.replace("clip 3\n", "clip 9\n")).unwrap();
        let k = gone.arrangement.at(1, 0).unwrap();
        assert!(gone.block_clip(&gone.arrangement.placements[k]).is_none());
    }

    /// Arrange 1.3: an `mpfree-clips 1` part, saved before blocks named
    /// their clip, opens: its clips get ids in track and row order, each
    /// `slot` block names its cell's clip (an empty cell's names none),
    /// each `own` block's clip becomes a song clip with the next id; it
    /// plays the same clips, and saves back as `mpfree-clips 2`.
    #[test]
    fn a_part_saved_before_blocks_named_their_clips_opens() {
        let text = "mpfree-clips 1
track Drums
dest 0
clip 0 3840 Beat
end
clip 3 3840 Fill
end
track Bass
dest 1
clip 1 7680 Line
end
place 0 0 7680 0 slot 0
place 0 7680 3840 0 own 1920 Beat (own)
note 0 240 36 100 100 - 1 0 0
end
place 1 0 7680 240 slot 1
place 1 7680 3840 0 slot 5
place 1 11520 3840 0 slot 1
";
        let p = from_text(text).unwrap();
        let ids: Vec<(usize, usize, u32)> =
            p.cell_clips().map(|(t, r, c)| (t, r, c.id.0)).collect();
        assert_eq!(ids, [(0, 0, 1), (0, 3, 2), (1, 1, 3)]);
        let blocks: Vec<(usize, Tick, Tick, u32)> = p
            .arrangement
            .placements
            .iter()
            .map(|b| (b.track, b.start, b.offset, b.clip.0))
            .collect();
        assert_eq!(
            blocks,
            [
                (0, 0, 0, 1),
                (0, 7680, 0, 4),
                (1, 0, 240, 3),
                (1, 7680, 0, 0),
                (1, 11520, 0, 3)
            ]
        );
        let own = p.arrangement.clips.get(crate::ClipId(4)).unwrap();
        assert_eq!(
            (own.name.as_str(), own.length, own.notes.len()),
            ("Beat (own)", 1920, 1)
        );
        assert_eq!(p.next_clip, 5);
        let names: Vec<Option<&str>> = p
            .arrangement
            .placements
            .iter()
            .map(|b| p.block_clip(b).map(|c| c.name.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                Some("Beat"),
                Some("Beat (own)"),
                Some("Line"),
                None,
                Some("Line")
            ]
        );
        let again = to_text(&p);
        assert!(again.starts_with("mpfree-clips 2\n"));
        assert_eq!(from_text(&again).unwrap(), p);
        // A row past the 64 a project keeps is refused, as before.
        assert!(from_text(&text.replace("slot 5", "slot 64")).is_err());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_param::OnTrack;

    fn full() -> Project {
        let mut p = Project::new();
        for i in 0..3 {
            p.tracks.push(Track::new(&format!("Track  {}", i + 1), i));
        }
        p.tracks[1].swing = Swing::new(240, 58);
        p.tracks[1].mute = true;
        p.tracks[2].solo = true;
        p.tracks[2].through_arp = true;
        p.tracks[2].record_touch = false;
        p.tracks[2].record_quantise = Some(160);
        p.tracks[1].transpose = crate::Transpose::semitones(-7);
        p.tracks[2].transpose = crate::Transpose {
            by: 3,
            in_key: true,
        };
        p.tracks[2].follows = true;
        p.launch = crate::LaunchGrid::Bars(4);
        // Board 7's Notes: Scale, Chord, the arp, Humanize (off).
        let fx = &mut p.tracks[2].note_fx;
        let all = mpfree_notefx::Fx::all();
        for (place, (i, on)) in [(0, true), (1, true), (6, false)].into_iter().enumerate() {
            fx.slots[place] = Some(Slot { fx: all[i], on });
            fx.ids[place] = [4, 2, 9][place];
        }
        fx.arp_at = 2;
        let mut c = Clip::new("Beat with  spaces", 7680);
        c.speed = Speed::new(2, 1).unwrap();
        c.swing = Swing::new(480, 66);
        c.quantise = Quantise::new(240, 75);
        c.follow = Some(Follow {
            bars: 4,
            action: FollowAction::Next,
            other: FollowAction::Any,
            chance: 70,
        });
        c.launch = Some(crate::LaunchGrid::Ticks(960));
        c.legato = true;
        c.once = true;
        c.program = Some(crate::ClipProgram {
            program: 4,
            bank: Some(300),
        });
        c.direction = crate::Direction::PingPong;
        c.holds = crate::Holds::Notes;
        let pts: Vec<TouchPoint> = (0..150)
            .map(|k| TouchPoint {
                at: k * 3,
                value: (k % 128) as u8,
            })
            .collect();
        let t = c.add_touch(&pts).unwrap();
        c.add_note(Note {
            touch: t,
            cond: Cond::Every { a: 2, b: 4 },
            chance: 50,
            ratchet: 3,
            ramp: -40,
            join: crate::Join::Slide,
            ..Note::new(0, 240, 36, 100)
        })
        .unwrap();
        c.add_note(Note {
            cond: Cond::NotFill,
            on: false,
            join: crate::Join::Tie,
            ..Note::new(960, 120, 38, 1)
        })
        .unwrap();
        c.add_lock(Lock::new(
            480,
            Some(36),
            RelAddr::new(OnTrack::Pad(36), 4),
            0.1,
        ))
        .unwrap();
        c.add_lock(Lock::new(0, None, RelAddr::new(OnTrack::Strip, 0), -3.25))
            .unwrap();
        // Outside gear's CC 74 on the track's MIDI out (S6's CC lanes).
        c.add_lock(Lock::new(240, None, RelAddr::new(OnTrack::Midi, 74), 20.0))
            .unwrap();
        let map = Map::linear(-60.0, 6.0);
        let target = Target::Track(RelAddr::new(OnTrack::Strip, 0));
        c.moves.reserve(target, map, 4).unwrap();
        c.moves
            .restore(
                target,
                map,
                0,
                u32::MAX,
                &[Point::ramp(0, 0.5), Point::step(100, 0.75), Point::end(200)],
            )
            .unwrap();
        p.tracks[0].slots[0] = Some(c.clone());
        p.tracks[2].slots[15] = Some(Clip::new("", 240));
        let sc = p.scene_mut(2).unwrap();
        sc.name = "Verse two".into();
        sc.mutes = Some(5);
        sc.follow = Some(Follow::after(8, FollowAction::First));
        sc.set_value("t1.mix.0".parse().unwrap(), -3.0).unwrap();
        // Program changes it sends as it starts (S5), one with a bank.
        let pc = |program, bank| Some(crate::ClipProgram { program, bank });
        sc.set_program(2, pc(5, None)).unwrap();
        sc.set_program(0, pc(12, Some(3))).unwrap();
        p.give_clip_ids();
        p.arrangement
            .placements
            .push(Placement::of(&p.tracks, 0, 0, 0, 7680));
        // A block of a song clip of its own (made unique), part way in.
        let mut own = c;
        own.id = p.new_clip_id();
        let mut block = Placement::new(0, own.id, 7680, 3840);
        block.offset = 120;
        p.arrangement.clips.keep(own).unwrap();
        p.arrangement.placements.push(block);
        p.arrangement.markers.push(Marker {
            at: 0,
            name: "Intro bit".into(),
        });
        p.arrangement.morphs.push(MorphRows { at: 0, a: 0, b: 1 });
        p.arrangement.list.steps =
            vec![ListStep { row: 2, times: 4 }, ListStep { row: 0, times: 1 }];
        p.arrangement.list.looped = true;
        let song = crate::arrange::MORPH_LANE;
        p.arrangement
            .lanes
            .reserve(song, Map::linear(0.0, 1.0), 2)
            .unwrap();
        p.arrangement
            .lanes
            .restore(
                song,
                Map::linear(0.0, 1.0),
                0,
                u32::MAX,
                &[Point::ramp(0, 0.0), Point::ramp(7680, 1.0)],
            )
            .unwrap();
        p.check().unwrap();
        p
    }

    #[test]
    fn a_whole_project_reads_back_the_same() {
        let p = full();
        let text = to_text(&p);
        let back = from_text(&text).unwrap();
        assert_eq!(back, p);
        // And again: the text is stable.
        assert_eq!(to_text(&back), text);
    }

    #[test]
    fn an_empty_project_and_a_new_one_round_trip() {
        let p = Project::new();
        assert_eq!(from_text(&to_text(&p)).unwrap(), p);
        let mut p = Project::new();
        for i in 0..8 {
            p.tracks.push(Track::new(&format!("Track {}", i + 1), i));
        }
        assert_eq!(from_text(&to_text(&p)).unwrap(), p);
        // Perform (S4) is saved; Make, a new project's, writes nothing,
        // so an older build reads the file as before.
        assert!(!to_text(&p).contains("perform"));
        p.perform = true;
        assert_eq!(from_text(&to_text(&p)).unwrap(), p);
        // In a track it is out of place, and skipped.
        let text = format!("{FORMAT}\ntrack Drums\ndest 0\nperform\n");
        assert!(!from_text(&text).unwrap().perform);
        // The project's launch quantise: 1 bar (the default) writes
        // nothing; any other reads back; one out of range is refused.
        let mut p = Project::new();
        assert!(!to_text(&p).contains("launch"));
        for g in crate::LAUNCH_GRIDS {
            p.launch = g;
            assert_eq!(from_text(&to_text(&p)).unwrap(), p, "{g:?}");
        }
        for bad in [
            "launch bars 0",
            "launch bars 17",
            "launch ticks 0",
            "launch ticks 99999",
            "launch soon",
        ] {
            assert!(from_text(&format!("{FORMAT}\n{bad}\n")).is_err(), "{bad}");
        }
        // The song list: an empty one writes nothing; steps out of range
        // are refused.
        assert!(!to_text(&Project::new()).contains("list"));
        for bad in ["list 64 1", "list 0 0", "list 0 100", "list 0", "list x 1"] {
            assert!(from_text(&format!("{FORMAT}\n{bad}\n")).is_err(), "{bad}");
        }
        // A track's transpose (S4): as written writes nothing; past two
        // octaves, or an unknown word after it, is refused.
        assert!(!to_text(&full_tracks_plain()).contains("transpose"));
        for bad in [
            "transpose 25",
            "transpose -25",
            "transpose 3 scale",
            "transpose x",
        ] {
            let text = format!("{FORMAT}\ntrack Bass\ndest 0\n{bad}\n");
            assert!(from_text(&text).is_err(), "{bad}");
        }
    }

    /// A song's tempo points (tempo ramps): written in order with their
    /// ramps, read back the same; a newer word after one is skipped; one
    /// out of range, out of order or too near another refuses the part.
    #[test]
    fn a_songs_tempo_points_round_trip() {
        let mut p = Project::new();
        assert!(!to_text(&p).contains("tempo"));
        let a = &mut p.arrangement;
        a.set_tempo(0, 92.0, None).unwrap();
        a.set_tempo(15_360, 140.5, Some(true)).unwrap();
        a.set_tempo(19_200, 100.0, None).unwrap();
        let text = to_text(&p);
        assert!(text.contains("tempo 0 92\ntempo 15360 140.5 ramp\ntempo 19200 100\n"));
        assert_eq!(from_text(&text).unwrap(), p);
        let newer = format!("{FORMAT}\ntempo 0 92 ramp curve=s\ntempo 960 120\n");
        let read = from_text(&newer).unwrap();
        assert_eq!(read.arrangement.tempo.len(), 2);
        assert!(read.arrangement.tempo[0].ramp);
        for bad in [
            "tempo x 90",
            "tempo 0",
            "tempo 0 500",
            "tempo -240 90",
            "tempo 480 90\ntempo 0 100",
            "tempo 0 90\ntempo 100 100",
        ] {
            assert!(from_text(&format!("{FORMAT}\n{bad}\n")).is_err(), "{bad}");
        }
    }

    /// Three plain tracks: nothing transposed, nothing following.
    fn full_tracks_plain() -> Project {
        let mut p = Project::new();
        for i in 0..3 {
            p.tracks.push(Track::new("t", i));
        }
        p
    }

    #[test]
    fn a_project_of_128_tracks_reads_back_and_one_more_is_refused() {
        let mut p = Project::new();
        for i in 0..MAX_TRACKS {
            p.tracks
                .push(Track::new(&format!("Track {}", i + 1), i as u16));
        }
        p.tracks[127].slots[3] = Some(Clip::new("Last", 480));
        // Scene mutes reach the last track; the text is a plain number,
        // as before 128 tracks, so older projects read the same.
        p.scene_mut(1).unwrap().mutes = Some(1 << 127 | 0b101);
        p.check().unwrap();
        let text = to_text(&p);
        assert_eq!(from_text(&text).unwrap(), p);
        let old = to_text(&full());
        assert!(old.contains("mutes 5\n"));
        assert_eq!(from_text(&old).unwrap().scenes[2].mutes, Some(5));
        let more = text.replace("scene 0", "track Track 129\ndest 128\nscene 0");
        assert!(from_text(&more).is_err(), "129 tracks");
    }

    #[test]
    fn unknown_lines_are_skipped_and_bad_ones_refuse_the_part() {
        let text = to_text(&full());
        let newer = text.replace("dest 0\n", "dest 0\nsparkle 7 9\n");
        assert_eq!(from_text(&newer).unwrap(), full());
        assert!(from_text("mpfree-clips 9\n").is_err());
        assert!(from_text("").is_err());
        // A note past the clip's end.
        let past = text.replace("note 960 120 38", "note 99999 120 38");
        assert!(from_text(&past).is_err());
        // A clip that never ends.
        let cut = &text[..text.find("end\n").unwrap()];
        assert!(from_text(cut).is_err());
        // A placement on a track that doesn't exist.
        let off = text.replace("place 0 0 7680", "place 9 0 7680");
        assert!(from_text(&off).is_err());
        // Numbers that don't fit.
        let big = text.replace("rectouch 0", "rectouch 2");
        assert!(from_text(&big).is_err());
        let coarse = text.replace("recquant 160", "recquant 99999");
        assert!(from_text(&coarse).is_err());
        // A MIDI effect past the last place, twice in one, out of range,
        // or one this build doesn't have.
        assert!(text.contains("notefx 1 2 on chord shape=major"));
        for (from, to) in [
            ("notefx 1 2 on", "notefx 8 2 on"),
            ("notefx 1 2 on", "notefx 0 2 on"),
            // An id of none, or another effect's on the track.
            ("notefx 1 2 on", "notefx 1 0 on"),
            ("notefx 1 2 on", "notefx 1 4 on"),
            ("notefx 1 2 on", "notefx 1 on"),
            ("notearp 2", "notearp 9"),
            ("notefx 1 2 on chord", "notefx 1 2 on sparkle"),
            ("vel=100", "vel=101"),
        ] {
            assert!(from_text(&text.replace(from, to)).is_err(), "{to}");
        }
        // A setting this build doesn't know is skipped.
        let newer = text.replace("notefx 1 2 on chord", "notefx 1 2 on chord glow=3");
        assert_eq!(from_text(&newer).unwrap(), full());
        let nan = text.replace("lock 480 36 pad36.4 0.1", "lock 480 36 pad36.4 NaN");
        assert!(from_text(&nan).is_err());
        // A clip's program change: past 127, a bank past 16383, or junk.
        assert!(text.contains("program 4 bank 300\n"));
        let no_bank = from_text(&text.replace("program 4 bank 300", "program 4")).unwrap();
        let first = &no_bank.tracks[0].slots[0].as_ref().unwrap().program;
        assert_eq!(first.map(|p| (p.program, p.bank)), Some((4, None)));
        for bad in [
            "program 128",
            "program 4 bank 16384",
            "program 4 300",
            "program",
        ] {
            let t = text.replace("program 4 bank 300", bad);
            assert!(from_text(&t).is_err(), "{bad}");
        }
    }

    /// What a clip's notes stand for (step 5c): `holds notes` reads back;
    /// none is hits, as every clip saved before, and so is a newer build's
    /// word; a clip of hits writes no line, so its text is as before; a
    /// line with no word refuses the part.
    #[test]
    fn a_clip_of_notes_reads_back_and_one_of_hits_saves_as_before() {
        use crate::Holds;
        let text = to_text(&full());
        assert!(text.contains("direction pingpong\nholds notes\n"), "{text}");
        let holds = |t: &str| {
            from_text(t).unwrap().tracks[0].slots[0]
                .as_ref()
                .unwrap()
                .holds
        };
        assert_eq!(holds(&text), Holds::Notes);
        let hits = text.replace("holds notes\n", "");
        assert_eq!(holds(&hits), Holds::Hits);
        assert_eq!(
            holds(&text.replace("holds notes", "holds hits")),
            Holds::Hits
        );
        assert_eq!(
            holds(&text.replace("holds notes", "holds chords")),
            Holds::Hits,
            "a newer build's word"
        );
        assert_eq!(to_text(&from_text(&hits).unwrap()), hits, "hits: no line");
        assert!(from_text(&text.replace("holds notes", "holds")).is_err());
        // Every other clip holds hits.
        let p = from_text(&text).unwrap();
        let notes = p
            .tracks
            .iter()
            .flat_map(|t| t.slots.iter())
            .filter(|(_, c)| c.holds == Holds::Notes)
            .count();
        assert_eq!(notes, 1);
    }

    /// A clip's play direction (S6): read back; none is forward, and so
    /// is a newer build's word; a line with no word refuses the part.
    #[test]
    fn a_clips_direction_reads_back() {
        let text = to_text(&full());
        assert!(
            text.contains("program 4 bank 300\ndirection pingpong\n"),
            "{text}"
        );
        let dir = |t: &str| {
            from_text(t).unwrap().tracks[0].slots[0]
                .as_ref()
                .unwrap()
                .direction
        };
        assert_eq!(dir(&text), crate::Direction::PingPong);
        for d in crate::Direction::ALL {
            let t = text.replace("direction pingpong\n", &format!("direction {}\n", d.word()));
            assert_eq!(dir(&t), d);
        }
        assert_eq!(
            dir(&text.replace("direction pingpong\n", "")),
            crate::Direction::Forward
        );
        assert!(
            !to_text(&from_text(&text.replace("direction pingpong\n", "")).unwrap())
                .contains("direction")
        );
        assert_eq!(
            dir(&text.replace("direction pingpong", "direction sideways")),
            crate::Direction::Forward
        );
        assert!(from_text(&text.replace("direction pingpong", "direction")).is_err());
    }

    /// A mutated clip keeps its original (S6), lanes and all, read back
    /// whole; an older build skips its lines and keeps the variation; a
    /// broken original refuses the part.
    #[test]
    fn only_the_pressure_a_note_names_is_saved() {
        let run = |v: u8| -> Vec<TouchPoint> {
            (0..4)
                .map(|k| TouchPoint {
                    at: k * 10,
                    value: v + k as u8,
                })
                .collect()
        };
        let mut c = Clip::new("c", 3840);
        let mut notes = Vec::new();
        for (i, v) in [10u8, 50, 90].into_iter().enumerate() {
            let touch = c.add_touch(&run(v)).unwrap();
            let n = Note {
                touch,
                ..Note::new(i as Tick * 960, 240, 36, 100)
            };
            c.add_note(n).unwrap();
            notes.push(n);
        }
        // The middle note taken out: its pressure stays, named by nothing.
        c.notes.retain(|n| n.tick != 960);
        let saved = |c: &Clip| {
            let mut p = Project::new();
            p.tracks.push(Track::new("Drums", 0));
            p.tracks[0].slots[0] = Some(c.clone());
            let text = to_text(&p);
            let back = from_text(&text).unwrap();
            (text, back.tracks[0].slots[0].clone().unwrap())
        };
        let (text, back) = saved(&c);
        assert!(
            text.contains("touch 0:10 10:11 20:12 30:13 0:90 10:91 20:92 30:93\n"),
            "{text}"
        );
        assert_eq!(back.touch.len(), 8);
        for (n, b) in c.notes.iter().zip(&back.notes) {
            assert_eq!(back.touch_of(b), c.touch_of(n));
        }
        assert_eq!(back.notes[1].touch.first, 4, "renumbered");
        // A note Restore would bring back keeps its pressure too.
        c.original = Some(Box::new(crate::Original {
            length: 3840,
            notes: notes.clone(),
            locks: Vec::new(),
            moves: Moves::new(),
            variation: 1,
        }));
        let (_, back) = saved(&c);
        assert_eq!(back.touch, c.touch);
        assert_eq!(back, c);
    }

    #[test]
    fn a_mutated_clips_original_reads_back() {
        let mut p = Project::new();
        p.tracks.push(Track::new("Drums", 0));
        let mut c = full().tracks[0].slots[0].clone().unwrap();
        c.notes[1].on = true;
        let mut v = c
            .mutated(9, crate::Amount::More, crate::Pitches::Pads)
            .unwrap();
        // Edited since: shorter, its lanes gone.
        v.length = 3840;
        v.moves = Moves::new();
        v.check().unwrap();
        p.tracks[0].slots[3] = Some(v.clone());
        p.give_clip_ids();
        let mut own = v.copied();
        own.id = p.new_clip_id();
        p.arrangement
            .place(Placement::new(0, own.id, 0, 3840))
            .unwrap();
        p.arrangement.clips.keep(own).unwrap();
        let text = to_text(&p);
        assert!(
            text.contains("\noriginal 7680 1\noriginal note 0 240 36 "),
            "{text}"
        );
        assert!(text.contains("\noriginal lane t:"), "{text}");
        assert!(text.contains("\noriginal point 100 0.75 s\n"), "{text}");
        let back = from_text(&text).unwrap();
        assert_eq!(back, p);
        let r = back.tracks[0].slots[3]
            .as_ref()
            .unwrap()
            .restored()
            .unwrap();
        assert_eq!((r.length, &r.notes, &r.moves), (7680, &c.notes, &c.moves));
        // An older build: the variation, with no original.
        let older: String = text
            .lines()
            .filter(|l| !l.starts_with("original"))
            .map(|l| format!("{l}\n"))
            .collect();
        let o = from_text(&older).unwrap();
        let w = o.tracks[0].slots[3].as_ref().unwrap();
        assert_eq!((w.original.as_ref(), &w.notes), (None, &v.notes));
        // Broken: a note before its head, a bad note, a second head.
        let head = "original 7680 1\n";
        let swapped = text.replacen(head, "", 1).replacen(
            "original lane",
            &format!("{head}original lane"),
            1,
        );
        assert!(from_text(&swapped).is_err());
        assert!(from_text(&text.replacen(
            "original note 0 240 36 ",
            "original note 0 240 36 0 ",
            1
        ))
        .is_err());
        assert!(from_text(&text.replacen(head, &format!("{head}{head}"), 1)).is_err());
        assert!(from_text(&text.replacen(head, "original 7680\n", 1)).is_err());
    }

    #[test]
    fn a_scenes_programs_read_back_in_order_and_bad_ones_refuse_the_part() {
        let text = to_text(&full());
        assert!(
            text.contains("value t1.mix.0 -3\nprogram 0 12 bank 3\nprogram 2 5\n"),
            "{text}"
        );
        let sc = &from_text(&text).unwrap().scenes[2];
        assert_eq!(
            sc.program(0).map(|p| (p.program, p.bank)),
            Some((12, Some(3)))
        );
        assert_eq!(sc.program(2).map(|p| (p.program, p.bank)), Some((5, None)));
        assert_eq!(sc.program(1), None);
        // Written out of order, they still read back in order of track.
        let swapped = text.replace(
            "program 0 12 bank 3\nprogram 2 5\n",
            "program 2 5\nprogram 0 12 bank 3\n",
        );
        assert_eq!(from_text(&swapped).unwrap(), full());
        // A track twice, past the last track, a program out of range, junk.
        for bad in [
            "program 0 12 bank 3\nprogram 0 7",
            "program 128 12 bank 3",
            "program 0 128 bank 3",
            "program 0 12 bank 16384",
            "program 0 12 3",
            "program 0",
            "program x 12",
        ] {
            let t = text.replace("program 0 12 bank 3", bad);
            assert!(from_text(&t).is_err(), "{bad}");
        }
        // Before any scene a `program` line is a stray one, as before.
        let stray = text.replacen("mpfree-clips 1\n", "mpfree-clips 1\nprogram 0 4\n", 1);
        assert_eq!(from_text(&stray).unwrap(), full());
    }

    #[test]
    fn a_steps_own_settings_read_back_and_older_notes_take_the_defaults() {
        let text = to_text(&full());
        assert!(text.contains(" ramp:-40 slide\n"), "{text}");
        assert!(text.contains(" off tie\n"), "{text}");
        let note = |t: &str| {
            let p = from_text(t).unwrap();
            p.tracks[0].slots[0].as_ref().unwrap().notes[1]
        };
        // An older file's note: on, no ramp, its own attack.
        let old = text.replace(" off tie\n", "\n");
        let n = note(&old);
        assert!(n.on);
        assert_eq!((n.ramp, n.join), (0, crate::Join::Attack));
        // A newer build's word is skipped; the ones this build knows stay.
        let newer = note(&text.replace(" off tie\n", " off sparkle tie\n"));
        assert!(!newer.on);
        assert_eq!(newer.join, crate::Join::Tie);
        // A ramp out of range, or not a number, refuses the part.
        for bad in ["ramp:127", "ramp:-127", "ramp:999", "ramp:x", "ramp:"] {
            let t = text.replace("ramp:-40", bad);
            assert!(from_text(&t).is_err(), "{bad}");
        }
    }

    #[test]
    fn loop_tracks_and_their_cells_read_back() {
        let mut p = full();
        let mut t = Track::new_loops("Loops A", 3);
        t.kind = TrackKind::Loops(LoopInput::Line);
        t.slots[0] = Some(Clip::new_loop(0, 7680));
        t.slots[2] = Some(Clip::new_loop(3, 3840));
        p.tracks.push(t);
        p.tracks.push(Track::new_loops("Loops B", 4));
        p.check().unwrap();
        let text = to_text(&p);
        assert!(text.contains("track Loops A\ndest 3\nmute 0\narp 0\nrectouch 1\nloops line\n"));
        assert!(text.contains("clip 2 3840 Loop 4\nloop 3\nend\n"));
        assert_eq!(from_text(&text).unwrap(), p);
        // A build before Loop tracks skips the lines: note tracks with
        // empty clips. One that names no input, or a loop past the
        // looper's, is refused; so is a cell on a note track.
        let bad = text.replace("loops line", "loops tape");
        assert!(from_text(&bad).is_err());
        let past = text.replace("loop 3\n", "loop 8\n");
        assert!(from_text(&past).is_err());
        let moved = text.replace("loops line\n", "");
        assert!(from_text(&moved).is_err(), "a loop cell on a note track");
    }

    #[test]
    fn every_cut_of_the_text_is_refused_or_reads_without_a_panic() {
        let text = to_text(&full());
        for k in (0..text.len()).step_by(7) {
            if text.is_char_boundary(k) {
                let _ = from_text(&text[..k]);
            }
        }
    }

    #[test]
    fn names_keep_their_spaces_and_lose_line_breaks() {
        let mut p = Project::new();
        p.tracks.push(Track::new("two\nlines", 0));
        let back = from_text(&to_text(&p)).unwrap();
        assert_eq!(back.tracks[0].name, "two lines");
        assert_eq!(nth_rest("0 7680 Beat  1", 2), "Beat  1");
        assert_eq!(nth_rest("0 7680", 2), "");
    }

    #[test]
    fn conditions_read_back() {
        for c in Cond::all() {
            assert_eq!(parse_cond(&cond_text(c)), Some(c));
        }
        assert_eq!(parse_cond("9:2"), None);
    }
}
