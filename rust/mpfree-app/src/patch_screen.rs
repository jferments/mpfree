//! The patch screens' side on the screen thread (live shows C1, layout
//! spec 4q): which of them is open, the zone picked, Learn low and high,
//! UNDO's takes of the edits, and what the keys do now as the audio loop
//! last said ([`KeysNow`]). It builds what [`crate::patch_page`]'s widgets
//! show from the song's patches (`Player::patches`) and makes the edits;
//! the screen sends the patches to the audio loop after each.

use mpfree_patch::{
    Curve, Keyboard, Layout, Outside, Patch, Patches, Zone, MAX_NAME, MAX_PATCHES, MAX_TRANSPOSE,
    MAX_ZONES,
};
use mpfree_ui::Color;

use crate::patch_page::{
    self as page, EditLook, KeysPlay, LayoutLook, RowLook, TileLook, ZoneLook,
};

/// An instrument a zone can play, as Mix names it: what a track plays
/// ("track 3"), or a synth no track has ("instrument 7").
#[derive(Clone, Debug, PartialEq)]
pub struct Inst {
    /// The words a zone keeps ([`crate::patch_in::Plays`]).
    pub words: String,
    /// Its name on the zones' bars: the track's or the synth's.
    pub name: String,
    /// What the Plays row adds: "free place" for a synth no track has.
    pub about: Option<String>,
    pub color: Color,
}

/// A key down, as the audio loop said: the keyboard (its port's name),
/// the channel, the note and its velocity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldKey {
    pub port: String,
    pub ch: u8,
    pub note: u8,
    pub vel: u8,
}

/// What the keys do now, as the audio loop said (`ToUi::Keys`): the
/// patch playing (from 0; None: keys as routed), a pick waiting for the
/// hands, and the keys down.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeysNow {
    pub playing: Option<usize>,
    pub waiting: Option<Option<usize>>,
    pub held: Vec<HeldKey>,
}

/// Which patch screen is open in place of Play's view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Open {
    #[default]
    Closed,
    /// The patch list.
    List,
    /// New patch's layouts.
    Layouts,
    /// Patch `i`'s editor (from 0).
    Edit(usize),
}

/// What the screen knows besides the patches, for the editor's choices.
pub struct Ctx<'a> {
    /// The song's instruments, as a zone names them.
    pub insts: &'a [Inst],
    /// The keyboards plugged in now, by their ports' names.
    pub ports: &'a [String],
    /// What the keys play without a patch: the armed track's words.
    pub routed: String,
}

/// One edit made: UNDO's words for it, and the patch the keys should
/// play now if that moved (a patch added or taken away before it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub title: String,
    pub pick: Option<Option<usize>>,
}

struct Take {
    title: String,
    /// The list as it was before (after, on the redo side).
    list: Patches,
}

/// UNDO's history of the patches, at most this many takes.
const TAKES: usize = 64;

#[derive(Default)]
pub struct PatchScreen {
    pub open: Open,
    /// The zone picked in the editor.
    pub zone: usize,
    /// Learn low (false) or Learn high (true) waits for the next key.
    pub learn: Option<bool>,
    /// The tile the wheel is on in the list.
    pub cursor: usize,
    /// What the keys do now.
    pub keys: KeysNow,
    /// The keys that came in last (the editor's line under the keyboard).
    last: Vec<HeldKey>,
    undo: Vec<Take>,
    redo: Vec<Take>,
}

/// The note's name as the app writes it (48 is C3).
pub fn note(n: u8) -> String {
    mpfree_midi_ui::note_name(n)
}

/// A zone's keys in words: "C3–C7", "up to B2", "C3 up", "every key".
pub fn range_words(low: u8, high: u8) -> String {
    match (low == 0, high == 127) {
        (true, true) => "every key".to_string(),
        (true, false) => format!("up to {}", note(high)),
        (false, true) => format!("{} up", note(low)),
        (false, false) => format!("{}–{}", note(low), note(high)),
    }
}

/// The same keys in a sentence: "C5 to C7 play nothing".
fn silent_words(low: u8, high: u8) -> String {
    match (low == 0, high == 127) {
        (true, true) => "every key plays nothing".to_string(),
        (true, false) => format!("the keys up to {} play nothing", note(high)),
        (false, true) => format!("{} and up play nothing", note(low)),
        (false, false) => format!("{} to {} play nothing", note(low), note(high)),
    }
}

fn inst<'a>(insts: &'a [Inst], words: &str) -> Option<&'a Inst> {
    insts.iter().find(|i| i.words == words)
}

/// A zone's instrument by name, or its words while it isn't in the song
/// ("Track 7").
fn name_of(insts: &[Inst], words: &str) -> String {
    inst(insts, words).map_or_else(
        || {
            let mut c = words.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect())
                .unwrap_or_default()
        },
        |i| i.name.clone(),
    )
}

fn zone_look(z: &Zone, insts: &[Inst]) -> ZoneLook {
    let i = inst(insts, &z.plays);
    ZoneLook {
        name: name_of(insts, &z.plays),
        color: i.map_or(Color::rgb(0x8A93A6), |i| i.color),
        low: z.low,
        high: z.high,
        vel_low: z.vel_low,
        vel_high: z.vel_high,
        range: range_words(z.low, z.high),
        missing: i.is_none(),
        pedals: z.pedals,
    }
}

/// "Grand C3 up", "Rhodes, every key", "Strings C3 up at 64+",
/// "Saw Lead C4 up, +12".
fn zone_line(z: &Zone, insts: &[Inst]) -> String {
    let name = name_of(insts, &z.plays);
    let range = range_words(z.low, z.high);
    let mut s = if range == "every key" {
        format!("{name}, every key")
    } else {
        format!("{name} {range}")
    };
    match (z.vel_low > 1, z.vel_high < 127) {
        (true, false) => s.push_str(&format!(" at {}+", z.vel_low)),
        (_, true) => s.push_str(&format!(" at {}–{}", z.vel_low, z.vel_high)),
        _ => {}
    }
    if z.transpose != 0 {
        s.push_str(&format!(", {:+}", z.transpose));
    }
    s
}

/// What a missing instrument leaves silent: "Track 7 is missing: C5 to
/// C7 play nothing".
fn missing_line(p: &Patch, insts: &[Inst]) -> Option<String> {
    let z = p.zones.iter().find(|z| inst(insts, &z.plays).is_none())?;
    Some(format!(
        "{} is missing: {}",
        name_of(insts, &z.plays),
        silent_words(z.low, z.high)
    ))
}

/// Whether zone `z` takes key `k` (its keyboard, keys and velocities).
fn takes(z: &Zone, k: &HeldKey) -> bool {
    let kb = match &z.keyboard {
        Keyboard::Any => true,
        Keyboard::Port { name, ch } => *name == k.port && ch.is_none_or(|c| c == k.ch),
    };
    kb && (z.low..=z.high).contains(&k.note) && (z.vel_low..=z.vel_high).contains(&k.vel)
}

/// The keyboards a zone can take: any, then each port on all its
/// channels or one.
fn keyboards(ports: &[String], now: &Keyboard) -> Vec<Keyboard> {
    let mut names: Vec<&String> = ports.iter().collect();
    if let Keyboard::Port { name, .. } = now {
        if !names.contains(&name) {
            names.insert(0, name);
        }
    }
    let mut out = vec![Keyboard::Any];
    for n in names {
        out.push(Keyboard::Port {
            name: n.clone(),
            ch: None,
        });
        out.extend((0..16).map(|c| Keyboard::Port {
            name: n.clone(),
            ch: Some(c),
        }));
    }
    out
}

fn keyboard_words(k: &Keyboard) -> String {
    match k {
        Keyboard::Any => "Any keyboard".to_string(),
        Keyboard::Port { name, ch: None } => format!("{name} · all channels"),
        Keyboard::Port { name, ch: Some(c) } => format!("{name} · ch {}", c + 1),
    }
}

const CURVES: [Curve; 4] = [Curve::AsPlayed, Curve::Soft, Curve::Hard, Curve::Fixed(100)];

fn curve_words(c: Curve) -> String {
    match c {
        Curve::AsPlayed => "As played".into(),
        Curve::Soft => "Soft".into(),
        Curve::Hard => "Hard".into(),
        Curve::Fixed(v) => format!("Fixed {v}"),
    }
}

/// Moves `at` (None: not in the list) `delta` along `n` choices, staying
/// at the ends.
fn stepped(at: Option<usize>, delta: i32, n: usize) -> Option<usize> {
    if n == 0 {
        return None;
    }
    let to = match at {
        Some(a) => (a as i64 + i64::from(delta)).clamp(0, n as i64 - 1) as usize,
        None if delta > 0 => 0,
        None => n - 1,
    };
    (Some(to) != at).then_some(to)
}

fn step_u8(v: u8, delta: i32, lo: u8, hi: u8) -> u8 {
    (i32::from(v) + delta).clamp(i32::from(lo), i32::from(hi)) as u8
}

/// The patch that plays after `before` became `after`: the one that
/// played, wherever it went (None: it went). An edit in place keeps the
/// number.
pub fn follow(before: &Patches, after: &Patches, playing: Option<usize>) -> Option<usize> {
    let i = playing?;
    if before.list.len() == after.list.len() {
        return (i < after.list.len()).then_some(i);
    }
    let was = before.list.get(i)?;
    after
        .list
        .iter()
        .position(|p| p == was)
        .or_else(|| after.list.iter().position(|p| p.name == was.name))
}

impl PatchScreen {
    /// Whether a patch screen is open in place of Play's view.
    pub fn showing(&self) -> bool {
        self.open != Open::Closed
    }

    /// The list's words over its tiles: "5 patches in this song".
    pub fn count(list: &Patches) -> String {
        match list.list.len() {
            0 => "No patches yet: New patch makes one".to_string(),
            1 => "1 patch in this song".to_string(),
            n => format!("{n} patches in this song"),
        }
    }

    /// The list's tiles: Keys as routed, then each patch.
    pub fn tiles(&self, list: &Patches, insts: &[Inst]) -> Vec<TileLook> {
        let playing = self.keys.playing;
        let mut out = vec![TileLook {
            number: None,
            name: "Keys as routed".to_string(),
            zones: Vec::new(),
            line: "No patch: the armed track and the routes on I/O play, as before".to_string(),
            missing: None,
            playing: playing.is_none(),
        }];
        for (i, p) in list.list.iter().enumerate() {
            let line = if p.zones.is_empty() {
                "No zones".to_string()
            } else {
                p.zones
                    .iter()
                    .map(|z| zone_line(z, insts))
                    .collect::<Vec<_>>()
                    .join(" · ")
            };
            out.push(TileLook {
                number: Some(i + 1),
                name: p.name.clone(),
                zones: p.zones.iter().map(|z| zone_look(z, insts)).collect(),
                line,
                missing: missing_line(p, insts),
                playing: playing == Some(i),
            });
        }
        out
    }

    /// New patch's layouts, each zone on `plays`.
    pub fn layouts(plays: &str, insts: &[Inst]) -> Vec<LayoutLook> {
        Layout::ALL
            .iter()
            .map(|l| {
                let p = l.patch(l.name(), plays);
                LayoutLook {
                    name: l.name().to_string(),
                    zones: p.zones.iter().map(|z| zone_look(z, insts)).collect(),
                }
            })
            .collect()
    }

    fn patch_name(list: &Patches, i: Option<usize>) -> String {
        match i.and_then(|i| list.list.get(i)) {
            Some(p) => p.name.clone(),
            None => "Keys as routed".to_string(),
        }
    }

    /// KEYS PLAY on Play's transport bar: None while the song has no
    /// patches.
    pub fn keys_play(&self, list: &Patches, insts: &[Inst]) -> Option<KeysPlay> {
        if list.list.is_empty() {
            return None;
        }
        let playing = self.keys.playing.filter(|&i| i < list.list.len());
        let (line, zones, missing) = match playing.and_then(|i| list.list.get(i).map(|p| (i, p))) {
            Some((i, p)) => (
                format!("{} · {}", i + 1, p.name),
                p.zones.iter().map(|z| zone_look(z, insts)).collect(),
                missing_line(p, insts).is_some(),
            ),
            None => ("Keys as routed".to_string(), Vec::new(), false),
        };
        let waiting = self.keys.waiting.map(|to| {
            format!(
                "{} → {}",
                PatchScreen::patch_name(list, playing),
                PatchScreen::patch_name(list, to)
            )
        });
        Some(KeysPlay {
            line,
            zones,
            waiting,
            missing,
        })
    }

    /// The editor's look of patch `i`, or None if the song hasn't it.
    pub fn edit_look(&self, list: &Patches, i: usize, ctx: &Ctx) -> Option<EditLook> {
        let p = list.list.get(i)?;
        let insts = ctx.insts;
        let zi = self.zone.min(p.zones.len().saturating_sub(1));
        let mut look = EditLook {
            name: p.name.clone(),
            sub: format!("patch {} · program change {}", i + 1, i + 1),
            zones: p.zones.iter().map(|z| zone_look(z, insts)).collect(),
            picked: zi,
            held: self.keys.held.iter().map(|k| (k.note, k.vel)).collect(),
            reached: p
                .zones
                .iter()
                .map(|z| {
                    self.keys
                        .held
                        .iter()
                        .filter(|k| takes(z, k))
                        .map(|k| k.vel)
                        .max()
                })
                .collect(),
            last: self.last_line(p, insts),
            learn: self.learn,
            ..EditLook::default()
        };
        if let Some(z) = p.zones.get(zi) {
            let name = name_of(insts, &z.plays);
            look.zone_title = format!("ZONE {} · {}", zi + 1, name.to_uppercase());
            if inst(insts, &z.plays).is_none() {
                look.missing = Some(format!(
                    "ZONE {} · {} ISN'T IN THIS SONG: {}",
                    zi + 1,
                    name.to_uppercase(),
                    silent_words(z.low, z.high).to_uppercase()
                ));
            }
            let plays = inst(insts, &z.plays).map_or_else(
                || format!("{name} (missing)"),
                |i| match &i.about {
                    Some(a) => format!("{} · {a}", i.name),
                    None => i.name.clone(),
                },
            );
            look.rows[page::ROW_PLAYS] = RowLook::text(plays);
            look.rows[page::ROW_PLAYS].enabled = !insts.is_empty();
            look.rows[page::ROW_KEYBOARD] = RowLook::text(keyboard_words(&z.keyboard));
            look.rows[page::ROW_KEYBOARD].enabled = keyboards(ctx.ports, &z.keyboard).len() > 1;
            look.rows[page::ROW_LOW] = RowLook::text(note(z.low));
            look.rows[page::ROW_HIGH] = RowLook::text(note(z.high));
            look.rows[page::ROW_TRANSPOSE] = RowLook::text(format!(
                "{} st",
                if z.transpose == 0 {
                    "0".to_string()
                } else {
                    format!("{:+}", z.transpose)
                }
            ));
            look.rows[page::ROW_VEL_LOW] = RowLook::text(z.vel_low.to_string());
            look.rows[page::ROW_VEL_HIGH] = RowLook::text(z.vel_high.to_string());
            look.rows[page::ROW_CURVE] = RowLook::text(curve_words(z.curve));
            look.rows[page::ROW_PEDALS] = RowLook::switch(z.pedals);
            look.rows[page::ROW_WHEELS] = RowLook::switch(z.wheels);
        }
        let (outside, about) = match &p.outside {
            Outside::Nothing => (
                "Nothing",
                Some("Keys no zone takes play nothing, so the gaps between the zones stay quiet."),
            ),
            Outside::Play(_) => ("Play an instrument", None),
            Outside::Routed => (
                "As routed",
                Some("Keys no zone takes play the armed track and the routes on I/O, as before."),
            ),
        };
        look.rows[page::ROW_OUTSIDE] = RowLook::text(outside);
        look.outside_about = about.map(str::to_string);
        if let Outside::Play(w) = &p.outside {
            look.rows[page::ROW_OUTSIDE_PLAYS] = RowLook::text(name_of(insts, w));
            look.rows[page::ROW_OUTSIDE_PLAYS].enabled = !insts.is_empty();
        }
        look.rows[page::ROW_CUT] = RowLook::switch(p.cut_old);
        look.cut_about = if p.cut_old {
            "On: notes and pedalled chords from the patch before stop when this one comes in."
        } else {
            "Off: notes and pedalled chords from the patch before ring on until you let go."
        }
        .to_string();
        Some(look)
    }

    /// The line under the editor's keyboard: the keys that came in last
    /// and the zones they reached.
    fn last_line(&self, p: &Patch, insts: &[Inst]) -> String {
        if self.last.is_empty() {
            return "Play a key: it lights here, with the zones it reaches".to_string();
        }
        let mut keys = self.last.clone();
        keys.sort_by_key(|k| (k.port.clone(), k.note));
        let ports: Vec<&str> = {
            let mut v: Vec<&str> = keys.iter().map(|k| k.port.as_str()).collect();
            v.dedup();
            v
        };
        // Keys reaching the same zones go together: "E4 G4 at 96 →
        // Grand, Strings".
        let mut groups: Vec<(String, Vec<&HeldKey>, Vec<usize>)> = Vec::new();
        for k in &keys {
            let reach: Vec<usize> = (0..p.zones.len())
                .filter(|&z| takes(&p.zones[z], k))
                .collect();
            match groups.last_mut() {
                Some(g) if g.0 == k.port && g.2 == reach => g.1.push(k),
                _ => groups.push((k.port.clone(), vec![k], reach)),
            }
        }
        let words: Vec<String> = groups
            .iter()
            .map(|(port, ks, reach)| {
                let notes: Vec<String> = ks.iter().map(|k| note(k.note)).collect();
                let vel = ks.iter().map(|k| k.vel).max().unwrap_or(0);
                let to = if reach.is_empty() {
                    "nothing".to_string()
                } else {
                    reach
                        .iter()
                        .map(|&z| name_of(insts, &p.zones[z].plays))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let s = format!("{} at {vel} → {to}", notes.join(" "));
                if ports.len() > 1 {
                    format!("{port}: {s}")
                } else {
                    s
                }
            })
            .collect();
        if ports.len() == 1 {
            format!("{} · {}", ports[0], words.join(" · "))
        } else {
            words.join(" · ")
        }
    }

    // ------------------------------------------------------- takes

    /// Makes an edit to the list as one UNDO take (the same edit again
    /// straight after, a drag or a turn, joins it). None: nothing changed.
    fn change(
        &mut self,
        list: &mut Patches,
        title: String,
        f: impl FnOnce(&mut Patches),
    ) -> Option<Edit> {
        let before = list.clone();
        f(list);
        if *list == before {
            return None;
        }
        let joins = self.redo.is_empty() && self.undo.last().is_some_and(|t| t.title == title);
        if !joins {
            self.undo.push(Take {
                title: title.clone(),
                list: before.clone(),
            });
            if self.undo.len() > TAKES {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        let playing = self.keys.playing;
        let now = follow(&before, list, playing);
        Some(Edit {
            title,
            pick: (now != playing).then_some(now),
        })
    }

    /// UNDO (or SHIFT + UNDO, `redo`): the last take back (or again).
    pub fn undo(&mut self, list: &mut Patches, redo: bool) -> Option<Edit> {
        let t = if redo {
            self.redo.pop()?
        } else {
            self.undo.pop()?
        };
        let before = std::mem::replace(list, t.list);
        let back = Take {
            title: t.title.clone(),
            list: before.clone(),
        };
        if redo {
            self.undo.push(back);
        } else {
            self.redo.push(back);
        }
        if let Open::Edit(i) = self.open {
            if i >= list.list.len() {
                self.open = Open::List;
            }
        }
        let playing = self.keys.playing;
        let now = follow(&before, list, playing);
        Some(Edit {
            title: t.title,
            pick: (now != playing).then_some(now),
        })
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// The undo history: "Start", the takes UNDO takes back, then REDO's,
    /// and the row that is now.
    pub fn history_rows(&self) -> (Vec<String>, usize) {
        let mut rows = vec!["Start: before these changes".to_string()];
        rows.extend(self.undo.iter().map(|t| t.title.clone()));
        let now = rows.len() - 1;
        rows.extend(self.redo.iter().rev().map(|t| t.title.clone()));
        (rows, now)
    }

    // ------------------------------------------------------- edits

    /// What the keys play now, for a new patch's zones: the patch
    /// playing's first zone's instrument, or the armed track.
    fn plays_now(&self, list: &Patches, ctx: &Ctx) -> String {
        self.keys
            .playing
            .and_then(|i| list.list.get(i))
            .and_then(|p| p.zones.first())
            .map_or_else(|| ctx.routed.clone(), |z| z.plays.clone())
    }

    /// New patch from layout `k`: its zones on what the keys play now,
    /// named "Patch N"; the editor opens on it. None when the song has
    /// [`MAX_PATCHES`].
    pub fn new_patch(&mut self, list: &mut Patches, k: usize, ctx: &Ctx) -> Option<Edit> {
        let layout = *Layout::ALL.get(k)?;
        if list.list.len() >= MAX_PATCHES {
            return None;
        }
        let n = list.list.len();
        let plays = self.plays_now(list, ctx);
        let e = self.change(list, format!("Patch {} · new", n + 1), |l| {
            l.list
                .push(layout.patch(&format!("Patch {}", n + 1), &plays));
        })?;
        self.open = Open::Edit(n);
        self.zone = 0;
        self.learn = None;
        Some(e)
    }

    /// Copy: a copy of patch `i` after it, the editor on the copy.
    pub fn copy(&mut self, list: &mut Patches, i: usize) -> Option<Edit> {
        if list.list.len() >= MAX_PATCHES {
            return None;
        }
        let p = list.list.get(i)?.clone();
        let e = self.change(list, format!("Patch {} · copy", i + 2), |l| {
            let mut c = p;
            c.name = format!("{} copy", c.name).chars().take(MAX_NAME).collect();
            l.list.insert(i + 1, c);
        })?;
        self.open = Open::Edit(i + 1);
        Some(e)
    }

    /// Delete (asked first): patch `i` goes, and the list shows.
    pub fn delete(&mut self, list: &mut Patches, i: usize) -> Option<Edit> {
        let name = list.list.get(i)?.name.clone();
        let e = self.change(list, format!("Patch {} · {name} deleted", i + 1), |l| {
            l.list.remove(i);
        })?;
        self.open = Open::List;
        self.cursor = 0;
        Some(e)
    }

    /// The name typed for patch `i`.
    pub fn rename(&mut self, list: &mut Patches, i: usize, name: &str) -> Option<Edit> {
        let name: String = name.trim().chars().take(MAX_NAME).collect();
        if name.is_empty() {
            return None;
        }
        self.change(list, format!("Patch {} · name", i + 1), |l| {
            if let Some(p) = l.list.get_mut(i) {
                p.name = name;
            }
        })
    }

    /// The editor's + Zone: a zone over every key on the last zone's
    /// instrument (or what the keys play now), picked, Learn low waiting
    /// for its first key.
    pub fn add_zone(&mut self, list: &mut Patches, i: usize, ctx: &Ctx) -> Option<Edit> {
        let p = list.list.get(i)?;
        if p.zones.len() >= MAX_ZONES {
            return None;
        }
        let plays = p
            .zones
            .last()
            .map_or_else(|| self.plays_now(list, ctx), |z| z.plays.clone());
        let z = p.zones.len();
        let e = self.change(
            list,
            format!("Patch {} · zone {} added", i + 1, z + 1),
            |l| {
                l.list[i].zones.push(Zone::new(&plays, 0, 127));
            },
        )?;
        self.zone = z;
        self.learn = Some(false);
        Some(e)
    }

    /// Remove (or ERASE + a lane): zone `z` goes.
    pub fn remove_zone(&mut self, list: &mut Patches, i: usize, z: usize) -> Option<Edit> {
        list.list.get(i)?.zones.get(z)?;
        let e = self.change(
            list,
            format!("Patch {} · zone {} removed", i + 1, z + 1),
            |l| {
                l.list[i].zones.remove(z);
            },
        )?;
        self.zone = self.zone.min(list.list[i].zones.len().saturating_sub(1));
        self.learn = None;
        Some(e)
    }

    /// A zone's ends dragged (or turned): its keys `low` to `high`.
    pub fn span(
        &mut self,
        list: &mut Patches,
        i: usize,
        z: usize,
        low: u8,
        high: u8,
    ) -> Option<Edit> {
        list.list.get(i)?.zones.get(z)?;
        let (low, high) = (low.min(high), high.max(low).min(127));
        self.zone = z;
        self.change(
            list,
            format!("Patch {} · zone {} · keys", i + 1, z + 1),
            |l| {
                let zone = &mut l.list[i].zones[z];
                (zone.low, zone.high) = (low, high);
            },
        )
    }

    /// Row `row` (patch_page's `ROW_*`) stepped `delta` on patch `i`'s
    /// picked zone (or the patch, for its own rows).
    pub fn step(
        &mut self,
        list: &mut Patches,
        i: usize,
        row: usize,
        delta: i32,
        ctx: &Ctx,
    ) -> Option<Edit> {
        let p = list.list.get(i)?;
        let zi = self.zone;
        let label = page::row_label(row);
        let title = match row {
            page::ROW_OUTSIDE | page::ROW_OUTSIDE_PLAYS | page::ROW_CUT => {
                format!("Patch {} · {label}", i + 1)
            }
            _ => format!("Patch {} · zone {} · {label}", i + 1, zi + 1),
        };
        // The patch's own rows.
        match row {
            page::ROW_OUTSIDE => {
                let at = match &p.outside {
                    Outside::Nothing => 0,
                    Outside::Play(_) => 1,
                    Outside::Routed => 2,
                };
                let to = stepped(Some(at), delta, 3)?;
                let plays = match &p.outside {
                    Outside::Play(w) => w.clone(),
                    _ => p
                        .zones
                        .first()
                        .map_or_else(|| ctx.routed.clone(), |z| z.plays.clone()),
                };
                return self.change(list, title, |l| {
                    l.list[i].outside = match to {
                        0 => Outside::Nothing,
                        1 => Outside::Play(plays),
                        _ => Outside::Routed,
                    };
                });
            }
            page::ROW_OUTSIDE_PLAYS => {
                let Outside::Play(w) = &p.outside else {
                    return None;
                };
                let at = ctx.insts.iter().position(|x| x.words == *w);
                let to = stepped(at, delta, ctx.insts.len())?;
                let words = ctx.insts[to].words.clone();
                return self.change(list, title, |l| {
                    l.list[i].outside = Outside::Play(words);
                });
            }
            page::ROW_CUT => {
                return self.change(list, title, |l| l.list[i].cut_old = !l.list[i].cut_old);
            }
            _ => {}
        }
        let z = p.zones.get(zi)?.clone();
        let mut n = z.clone();
        match row {
            page::ROW_PLAYS => {
                let at = ctx.insts.iter().position(|x| x.words == z.plays);
                n.plays = ctx.insts[stepped(at, delta, ctx.insts.len())?]
                    .words
                    .clone();
            }
            page::ROW_KEYBOARD => {
                let all = keyboards(ctx.ports, &z.keyboard);
                let at = all.iter().position(|k| *k == z.keyboard);
                n.keyboard = all[stepped(at, delta, all.len())?].clone();
            }
            page::ROW_LOW => n.low = step_u8(z.low, delta, 0, z.high),
            page::ROW_HIGH => n.high = step_u8(z.high, delta, z.low, 127),
            page::ROW_TRANSPOSE => {
                n.transpose = (i32::from(z.transpose) + delta)
                    .clamp(-i32::from(MAX_TRANSPOSE), i32::from(MAX_TRANSPOSE))
                    as i8;
            }
            page::ROW_VEL_LOW => n.vel_low = step_u8(z.vel_low, delta, 1, z.vel_high),
            page::ROW_VEL_HIGH => n.vel_high = step_u8(z.vel_high, delta, z.vel_low, 127),
            page::ROW_CURVE => {
                let at = CURVES.iter().position(|c| match (c, z.curve) {
                    (Curve::Fixed(_), Curve::Fixed(_)) => true,
                    (a, b) => *a == b,
                });
                n.curve = CURVES[stepped(at, delta, CURVES.len())?];
            }
            page::ROW_PEDALS => n.pedals = !z.pedals,
            page::ROW_WHEELS => n.wheels = !z.wheels,
            _ => return None,
        }
        self.change(list, title, |l| l.list[i].zones[zi] = n)
    }

    /// A switch tapped: Pedals, Wheels and pressure, or Cut the old
    /// sound.
    pub fn toggle(&mut self, list: &mut Patches, i: usize, row: usize, ctx: &Ctx) -> Option<Edit> {
        match row {
            page::ROW_PEDALS | page::ROW_WHEELS | page::ROW_CUT => self.step(list, i, row, 1, ctx),
            _ => None,
        }
    }

    /// What the keys do now came in. Learn low or high takes the first key
    /// that went down since: the picked zone's end goes there.
    pub fn keys_in(&mut self, list: &mut Patches, now: KeysNow) -> Option<Edit> {
        let new: Vec<u8> = now
            .held
            .iter()
            .filter(|k| {
                !self
                    .keys
                    .held
                    .iter()
                    .any(|o| o.port == k.port && o.ch == k.ch && o.note == k.note)
            })
            .map(|k| k.note)
            .collect();
        if !now.held.is_empty() {
            self.last.clone_from(&now.held);
        }
        self.keys = now;
        let (Some(high), Open::Edit(i)) = (self.learn, self.open) else {
            return None;
        };
        let key = if high {
            new.iter().max()
        } else {
            new.iter().min()
        };
        let key = *key?;
        let zi = self.zone;
        let z = list.list.get(i)?.zones.get(zi)?;
        let (low, top) = if high {
            (z.low.min(key), key)
        } else {
            (key, z.high.max(key))
        };
        self.learn = None;
        let which = if high { "Learn high" } else { "Learn low" };
        self.change(
            list,
            format!("Patch {} · zone {} · {which}", i + 1, zi + 1),
            |l| {
                let z = &mut l.list[i].zones[zi];
                (z.low, z.high) = (low, top);
            },
        )
    }

    /// The list's wheel: the tile it is on moves `d`.
    pub fn move_cursor(&mut self, d: i32, tiles: usize) {
        if tiles == 0 {
            return;
        }
        self.cursor = (self.cursor as i64 + i64::from(d)).clamp(0, tiles as i64 - 1) as usize;
    }

    /// – / + on the list: the patch before or after the one playing
    /// (Keys as routed before patch 1).
    pub fn next_to(&self, list: &Patches, d: i32) -> Option<Option<usize>> {
        let n = list.list.len() as i64;
        let at = self.keys.playing.map_or(-1, |i| i as i64);
        let to = (at + i64::from(d.signum())).clamp(-1, n - 1);
        (to != at).then(|| usize::try_from(to).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insts() -> Vec<Inst> {
        let i = |w: &str, n: &str, about: Option<&str>| Inst {
            words: w.into(),
            name: n.into(),
            about: about.map(str::to_string),
            color: Color::rgb(0x4FA3FF),
        };
        vec![
            i("track 1", "Sub Bass", None),
            i("track 2", "Grand", None),
            i("instrument 7", "Strings", Some("free place")),
        ]
    }

    fn chorus() -> Patch {
        let mut p = Layout::BassAndKeys.patch("Chorus split", "track 1");
        p.zones[1].plays = "track 2".into();
        let mut strings = Zone::new("instrument 7", 48, 127);
        strings.vel_low = 64;
        p.zones.push(strings);
        p
    }

    fn ctx<'a>(insts: &'a [Inst], ports: &'a [String]) -> Ctx<'a> {
        Ctx {
            insts,
            ports,
            routed: "track 1".into(),
        }
    }

    fn key(note: u8, vel: u8) -> HeldKey {
        HeldKey {
            port: "KeyLab 61".into(),
            ch: 0,
            note,
            vel,
        }
    }

    /// The tiles say what each patch plays, the one playing marked, and
    /// a missing instrument which keys it leaves silent.
    #[test]
    fn the_tiles_say_what_each_patch_plays() {
        let mut lead = Layout::Whole.patch("Lead", "track 9");
        lead.zones[0].low = 72;
        lead.zones[0].transpose = 12;
        let list = Patches {
            list: vec![chorus(), lead],
        };
        let mut s = PatchScreen::default();
        s.keys.playing = Some(0);
        let tiles = s.tiles(&list, &insts());
        assert_eq!(tiles.len(), 3);
        assert!(!tiles[0].playing && tiles[1].playing);
        assert_eq!(
            tiles[1].line,
            "Sub Bass up to B2 · Grand C3 up · Strings C3 up at 64+"
        );
        assert_eq!(
            tiles[2].missing.as_deref(),
            Some("Track 9 is missing: C5 and up play nothing")
        );
        assert_eq!(tiles[2].line, "Track 9 C5 up, +12");
        assert_eq!(PatchScreen::count(&list), "2 patches in this song");
    }

    /// KEYS PLAY: none without patches, the patch playing, and both names
    /// while a pick waits for the hands.
    #[test]
    fn keys_play_names_the_patch_and_a_pick_waiting() {
        let mut s = PatchScreen::default();
        assert_eq!(s.keys_play(&Patches::default(), &insts()), None);
        let list = Patches {
            list: vec![chorus(), Layout::Whole.patch("Lead", "track 2")],
        };
        let k = s.keys_play(&list, &insts()).unwrap();
        assert_eq!(k.line, "Keys as routed");
        s.keys.playing = Some(0);
        s.keys.waiting = Some(Some(1));
        let k = s.keys_play(&list, &insts()).unwrap();
        assert_eq!(k.line, "1 · Chorus split");
        assert_eq!(k.zones.len(), 3);
        assert_eq!(k.waiting.as_deref(), Some("Chorus split → Lead"));
    }

    /// The editor lights the zones the keys held reach, with their
    /// velocities, and says what came in last.
    #[test]
    fn the_editor_shows_what_the_keys_reach() {
        let mut list = Patches {
            list: vec![chorus()],
        };
        let mut s = PatchScreen {
            open: Open::Edit(0),
            zone: 2,
            ..PatchScreen::default()
        };
        let ins = insts();
        let ports = vec!["KeyLab 61".to_string()];
        let c = ctx(&ins, &ports);
        s.keys_in(
            &mut list,
            KeysNow {
                playing: Some(0),
                waiting: None,
                held: vec![key(36, 52), key(64, 96), key(67, 96)],
            },
        );
        let look = s.edit_look(&list, 0, &c).unwrap();
        assert_eq!(look.reached, vec![Some(52), Some(96), Some(96)]);
        assert_eq!(
            look.last,
            "KeyLab 61 · C2 at 52 → Sub Bass · E4 G4 at 96 → Grand, Strings"
        );
        assert_eq!(look.zone_title, "ZONE 3 · STRINGS");
        assert_eq!(look.rows[page::ROW_PLAYS].value, "Strings · free place");
        assert_eq!(look.rows[page::ROW_VEL_LOW].value, "64");
        assert_eq!(look.sub, "patch 1 · program change 1");
        // Soft keys miss the strings.
        s.keys_in(
            &mut list,
            KeysNow {
                held: vec![key(64, 40)],
                ..KeysNow::default()
            },
        );
        let look = s.edit_look(&list, 0, &c).unwrap();
        assert_eq!(look.reached, vec![None, Some(40), None]);
        assert!(look.last.ends_with("E4 at 40 → Grand"), "{}", look.last);
    }

    /// Each change is one UNDO take (a drag's many moves one), UNDO and
    /// REDO walk them, and the history lists them.
    #[test]
    fn each_change_is_one_take() {
        let mut list = Patches {
            list: vec![chorus()],
        };
        let ins = insts();
        let c = ctx(&ins, &[]);
        let mut s = PatchScreen {
            open: Open::Edit(0),
            zone: 1,
            ..PatchScreen::default()
        };
        let e = s.step(&mut list, 0, page::ROW_LOW, 2, &c).unwrap();
        assert_eq!(e.title, "Patch 1 · zone 2 · Low key");
        s.step(&mut list, 0, page::ROW_LOW, 1, &c).unwrap();
        assert_eq!(list.list[0].zones[1].low, 51);
        s.step(&mut list, 0, page::ROW_PEDALS, 1, &c).unwrap();
        assert!(!list.list[0].zones[1].pedals);
        let (rows, now) = s.history_rows();
        assert_eq!(rows.len(), 3);
        assert_eq!(now, 2);
        s.undo(&mut list, false).unwrap();
        assert!(list.list[0].zones[1].pedals);
        s.undo(&mut list, false).unwrap();
        assert_eq!(list.list[0].zones[1].low, 48, "the two turns were one take");
        assert!(s.undo(&mut list, false).is_none());
        s.undo(&mut list, true).unwrap();
        assert_eq!(list.list[0].zones[1].low, 51);
        // Low never passes high; transpose stops at 48.
        s.step(&mut list, 0, page::ROW_LOW, 200, &c);
        assert_eq!(list.list[0].zones[1].low, 127);
        s.step(&mut list, 0, page::ROW_TRANSPOSE, 99, &c);
        assert_eq!(list.list[0].zones[1].transpose, 48);
    }

    /// The choices step through the song's instruments, the keyboards
    /// plugged in and the curves; Keys outside the zones' instrument row
    /// only while it plays one.
    #[test]
    fn the_rows_step_through_their_choices() {
        let mut list = Patches {
            list: vec![chorus()],
        };
        let ins = insts();
        let ports = vec!["KeyLab 61".to_string()];
        let c = ctx(&ins, &ports);
        let mut s = PatchScreen {
            open: Open::Edit(0),
            ..PatchScreen::default()
        };
        s.step(&mut list, 0, page::ROW_PLAYS, 1, &c).unwrap();
        assert_eq!(list.list[0].zones[0].plays, "track 2");
        s.step(&mut list, 0, page::ROW_KEYBOARD, 2, &c).unwrap();
        assert_eq!(
            list.list[0].zones[0].keyboard,
            Keyboard::Port {
                name: "KeyLab 61".into(),
                ch: Some(0)
            }
        );
        s.step(&mut list, 0, page::ROW_CURVE, 3, &c).unwrap();
        assert_eq!(list.list[0].zones[0].curve, Curve::Fixed(100));
        assert!(s.step(&mut list, 0, page::ROW_CURVE, 1, &c).is_none());
        let look = s.edit_look(&list, 0, &c).unwrap();
        assert!(look.outside_about.is_some());
        s.step(&mut list, 0, page::ROW_OUTSIDE, 1, &c).unwrap();
        assert_eq!(list.list[0].outside, Outside::Play("track 2".into()));
        let look = s.edit_look(&list, 0, &c).unwrap();
        assert_eq!(look.outside_about, None);
        assert_eq!(look.rows[page::ROW_OUTSIDE_PLAYS].value, "Grand");
        s.step(&mut list, 0, page::ROW_OUTSIDE_PLAYS, 1, &c)
            .unwrap();
        assert_eq!(list.list[0].outside, Outside::Play("instrument 7".into()));
        s.toggle(&mut list, 0, page::ROW_CUT, &c).unwrap();
        assert!(list.list[0].cut_old);
    }

    /// New patch, Copy and Delete; the patch playing keeps playing
    /// whatever moved before it; Learn sets an end from the next key.
    #[test]
    fn patches_come_and_go_and_learn_takes_a_key() {
        let mut list = Patches::default();
        let ins = insts();
        let c = ctx(&ins, &[]);
        let mut s = PatchScreen::default();
        s.new_patch(&mut list, 1, &c).unwrap();
        assert_eq!(list.list[0].name, "Patch 1");
        assert_eq!(list.list[0].zones.len(), 2);
        assert_eq!(list.list[0].zones[1].plays, "track 1");
        assert_eq!(s.open, Open::Edit(0));
        s.new_patch(&mut list, 0, &c).unwrap();
        s.keys.playing = Some(1);
        // A copy of patch 1 goes in before the one playing: it follows.
        let e = s.copy(&mut list, 0).unwrap();
        assert_eq!(list.list[1].name, "Patch 1 copy");
        assert_eq!(e.pick, Some(Some(2)));
        s.keys.playing = Some(2);
        let e = s.delete(&mut list, 2).unwrap();
        assert_eq!(e.pick, Some(None), "the patch playing went");
        assert_eq!(s.open, Open::List);
        // Learn high: the next key down sets the high end.
        s.open = Open::Edit(0);
        s.zone = 0;
        s.learn = Some(true);
        let e = s
            .keys_in(
                &mut list,
                KeysNow {
                    held: vec![key(40, 90)],
                    ..KeysNow::default()
                },
            )
            .unwrap();
        assert_eq!(e.title, "Patch 1 · zone 1 · Learn high");
        assert_eq!(
            (list.list[0].zones[0].low, list.list[0].zones[0].high),
            (0, 40)
        );
        assert_eq!(s.learn, None);
        // + Zone waits for its first key.
        s.add_zone(&mut list, 0, &c).unwrap();
        assert_eq!((s.zone, s.learn), (2, Some(false)));
        s.keys_in(&mut list, KeysNow::default());
        s.keys_in(
            &mut list,
            KeysNow {
                held: vec![key(72, 90)],
                ..KeysNow::default()
            },
        );
        assert_eq!(list.list[0].zones[2].low, 72);
        // – / + on the list: the patch before or after.
        s.keys.playing = None;
        assert_eq!(s.next_to(&list, 1), Some(Some(0)));
        assert_eq!(s.next_to(&list, -1), None);
    }
}
