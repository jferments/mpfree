//! The time engine's model as text: the `clips` part of a saved project
//! (`parts/clips.txt` in the project folder, docs/dev/design/browser.md).
//!
//! One record a line, a word saying what it is first, then numbers, and
//! a name (when there is one) as the rest of the line. A track's clips,
//! a scene's values and a song placement's own clip follow the line that
//! opens them; `end` closes a clip. Lines this build doesn't know are
//! skipped, so a newer build's file still opens.
//!
//! ```text
//! mpfree-clips 1
//! track Drums
//! dest 0
//! swing 240 58
//! clip 0 7680 Beat 1
//! speed 1 1
//! touch 0:10 50:40
//! note 0 240 36 100 100 - 1 0 2
//! lock 480 36 pad36.4 0.25
//! lane t:mix.0 -60 6 0 n
//! point 0 0.5 r
//! end
//! scene 0 Verse
//! mutes 5
//! value t1.mix.0 -3
//! place 0 0 7680 0 slot 0
//! marker 0 Intro
//! morph 0 0 1
//! ```
//!
//! Reading is careful: every number is checked, every clip and the whole
//! project go through [`Project::check`]-level checks, and a part that
//! doesn't hold together is refused whole (the engine keeps what it had).
//! Floats are written so they read back the same (Rust's shortest form).

use std::fmt::Write as _;
use std::str::FromStr;

use mpfree_moves::{Kind, Map, Moves, Point, Steps, Target};
use mpfree_param::{ParamAddr, RelAddr};
use mpfree_transport::time::{Swing, Tick};

use crate::arrange::{Marker, MorphRows, Placement, Source};
use crate::scenes::{Follow, FollowAction, Scene, SCENES};
use crate::{
    Clip, Cond, Lock, Note, Project, Quantise, Speed, Touch, TouchPoint, Track, MAX_TRACKS, SLOTS,
};

/// The first line.
pub const FORMAT: &str = "mpfree-clips 1";
/// Pressure points a `touch` line holds (more go on more lines).
const TOUCH_PER_LINE: usize = 64;

/// The project as text.
pub fn to_text(p: &Project) -> String {
    let mut s = String::with_capacity(4096);
    s.push_str(FORMAT);
    s.push('\n');
    for t in &p.tracks {
        let _ = writeln!(s, "track {}", one_line(&t.name));
        let _ = writeln!(s, "dest {}", t.dest);
        if let Some(sw) = t.swing {
            let _ = writeln!(s, "swing {} {}", sw.grid, sw.amount);
        }
        let _ = writeln!(s, "mute {}", u8::from(t.mute));
        let _ = writeln!(s, "arp {}", u8::from(t.through_arp));
        let _ = writeln!(s, "rectouch {}", u8::from(t.record_touch));
        for (slot, c) in t.slots.iter().enumerate() {
            if let Some(c) = c {
                let _ = writeln!(s, "clip {slot} {} {}", c.length, one_line(&c.name));
                clip_body(&mut s, c);
            }
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
    }
    let a = &p.arrangement;
    for pl in &a.placements {
        let head = format!(
            "place {} {} {} {}",
            pl.track, pl.start, pl.length, pl.offset
        );
        match &pl.source {
            Source::Slot(slot) => {
                let _ = writeln!(s, "{head} slot {slot}");
            }
            Source::Own(c) => {
                let _ = writeln!(s, "{head} own {} {}", c.length, one_line(&c.name));
                clip_body(&mut s, c);
            }
        }
    }
    for m in &a.markers {
        let _ = writeln!(s, "marker {} {}", m.at, one_line(&m.name));
    }
    for m in &a.morphs {
        let _ = writeln!(s, "morph {} {} {}", m.at, m.a, m.b);
    }
    if !a.lanes.is_empty() {
        s.push_str("songlanes\n");
        lanes_text(&mut s, &a.lanes);
        s.push_str("end\n");
    }
    s
}

fn clip_body(s: &mut String, c: &Clip) {
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
    // Pressure first: the notes name runs of it.
    for chunk in c.touch.chunks(TOUCH_PER_LINE) {
        s.push_str("touch");
        for p in chunk {
            let _ = write!(s, " {}:{}", p.at, p.value);
        }
        s.push('\n');
    }
    for n in &c.notes {
        let _ = writeln!(
            s,
            "note {} {} {} {} {} {} {} {} {}",
            n.tick,
            n.len,
            n.note,
            n.velocity,
            n.chance,
            cond_text(n.cond),
            n.ratchet,
            n.touch.first,
            n.touch.len
        );
    }
    for l in &c.locks {
        let sound = l.sound.map_or("-".to_string(), |x| x.to_string());
        let _ = writeln!(s, "lock {} {sound} {} {}", l.tick, l.addr, l.value);
    }
    lanes_text(s, &c.moves);
    s.push_str("end\n");
}

fn lanes_text(s: &mut String, moves: &Moves) {
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
            "lane {target} {} {} {} {steps}",
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
            let _ = writeln!(s, "point {} {} {kind}", p.tick, p.u);
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
    Clip(Clip, ClipHome),
    Scene,
    /// The song's own lanes, until `end`.
    SongLanes(Moves),
}

enum ClipHome {
    Slot(usize),
    /// The placement waiting for it.
    Own(usize, Tick, Tick, Tick),
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
        Some((_, l)) if l.trim() == FORMAT => {}
        _ => return Err(format!("not an {FORMAT} part")),
    }
    let mut p = Project::new();
    let mut at = At::Top;
    let mut lane: Option<LaneAt> = None;
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
            let tick: u32 = num(w.next()).ok_or_else(|| bad("tick"))?;
            let u: f32 = num(w.next()).ok_or_else(|| bad("value"))?;
            let kind = match w.next() {
                Some("r") => Kind::Ramp,
                Some("s") => Kind::Step,
                Some("e") => Kind::End,
                _ => return Err(bad("kind")),
            };
            if !u.is_finite() || l.points.last().is_some_and(|q| q.tick >= tick) {
                return Err(bad("point out of order"));
            }
            l.points.push(Point { tick, u, kind });
            continue;
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
            ("swing", At::Track) => {
                last(&mut p).swing = Some(swing(&mut w).ok_or_else(|| bad("swing"))?)
            }
            ("mute", At::Track) => last(&mut p).mute = flag(w.next()).ok_or_else(|| bad("mute"))?,
            ("arp", At::Track) => {
                last(&mut p).through_arp = flag(w.next()).ok_or_else(|| bad("arp"))?
            }
            ("rectouch", At::Track) => {
                last(&mut p).record_touch = flag(w.next()).ok_or_else(|| bad("rectouch"))?
            }
            ("clip", At::Track) => {
                let slot: usize = num(w.next()).ok_or_else(|| bad("slot"))?;
                let length: Tick = num(w.next()).ok_or_else(|| bad("length"))?;
                if slot >= SLOTS || last(&mut p).slots[slot].is_some() {
                    return Err(bad("slot"));
                }
                let name = nth_rest(rest, 2);
                at = At::Clip(Clip::new(name, length), ClipHome::Slot(slot));
            }
            ("speed", At::Clip(c, _)) => {
                let (a, b) = (num(w.next()), num(w.next()));
                c.speed = a
                    .zip(b)
                    .and_then(|(a, b)| Speed::new(a, b))
                    .ok_or_else(|| bad("speed"))?;
            }
            ("swing", At::Clip(c, _)) => c.swing = Some(swing(&mut w).ok_or_else(|| bad("swing"))?),
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
                let n = (|| {
                    Some(Note {
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
                    })
                })()
                .ok_or_else(|| bad("note"))?;
                c.add_note(n).map_err(|e| bad(&format!("{e:?}")))?;
            }
            ("lock", At::Clip(c, _)) => {
                let l = (|| {
                    let tick = num(w.next())?;
                    let sound = match w.next()? {
                        "-" => None,
                        x => Some(x.parse().ok()?),
                    };
                    let addr = RelAddr::from_str(w.next()?).ok()?;
                    Some(Lock::new(tick, sound, addr, num(w.next())?))
                })()
                .ok_or_else(|| bad("lock"))?;
                c.add_lock(l).map_err(|e| bad(&format!("{e:?}")))?;
            }
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
                        last(&mut p).slots[slot] = Some(c);
                        At::Track
                    }
                    ClipHome::Own(track, start, length, offset) => {
                        p.arrangement.placements.push(Placement {
                            track,
                            start,
                            length,
                            offset,
                            source: Source::Own(Box::new(c)),
                        });
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
            ("place", At::Top | At::Track | At::Scene) => {
                let track: usize = num(w.next()).ok_or_else(|| bad("track"))?;
                let start: Tick = num(w.next()).ok_or_else(|| bad("start"))?;
                let length: Tick = num(w.next()).ok_or_else(|| bad("length"))?;
                let offset: Tick = num(w.next()).ok_or_else(|| bad("offset"))?;
                match w.next() {
                    Some("slot") => {
                        let slot: usize = num(w.next()).ok_or_else(|| bad("slot"))?;
                        p.arrangement.placements.push(Placement {
                            track,
                            start,
                            length,
                            offset,
                            source: Source::Slot(slot),
                        });
                        at = At::Top;
                    }
                    Some("own") => {
                        let clip_len: Tick = num(w.next()).ok_or_else(|| bad("length"))?;
                        let name = nth_rest(rest, 6);
                        at = At::Clip(
                            Clip::new(name, clip_len),
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
            ("songlanes", At::Top | At::Track | At::Scene) => at = At::SongLanes(Moves::new()),
            // A newer build's line: skipped.
            _ if !matches!(word, "end" | "lane") => {}
            _ => return Err(bad("out of place")),
        }
    }
    if lane.is_some() || matches!(at, At::Clip(..) | At::SongLanes(_)) {
        return Err("the part ends inside a clip".to_string());
    }
    p.check()
        .map_err(|e| format!("the clips don't hold together: {e:?}"))?;
    Ok(p)
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
        p.tracks[2].through_arp = true;
        p.tracks[2].record_touch = false;
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
            ..Note::new(0, 240, 36, 100)
        })
        .unwrap();
        c.add_note(Note {
            cond: Cond::NotFill,
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
        p.arrangement
            .placements
            .push(Placement::linked(0, 0, 0, 7680));
        p.arrangement.placements.push(Placement {
            track: 0,
            start: 7680,
            length: 3840,
            offset: 120,
            source: Source::Own(Box::new(c)),
        });
        p.arrangement.markers.push(Marker {
            at: 0,
            name: "Intro bit".into(),
        });
        p.arrangement.morphs.push(MorphRows { at: 0, a: 0, b: 1 });
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
        let nan = text.replace("lock 480 36 pad36.4 0.1", "lock 480 36 pad36.4 NaN");
        assert!(from_text(&nan).is_err());
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
