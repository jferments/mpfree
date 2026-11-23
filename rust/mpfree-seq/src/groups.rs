//! Trigger groups (step sequencer refactor, the trigger groups design
//! approved after its review on 22 Nov): the notes of a chord that one
//! pad hit or one tap made decide together whether they play.
//!
//! In a clip of pad hits a chord pad's hit is one note, so it rolls once
//! and its chord plays whole or not at all. In a clip of notes the same
//! chord is several notes. [`Note::group`] keeps them one **trigger**:
//! the notes of a clip of notes on one tick (as recorded, before quantise
//! and swing) with the same number, not 0, decide once, at the first of
//! them the player meets: its condition, then its chance, one result for
//! PRE and NEI. 0 is a note that decides alone, as every note did before.
//!
//! - **Numbers.** A pad's notes take the pad's number plus one
//!   ([`pad_group`]), so a trigger rolls with the key its pad's hit had
//!   and Convert to notes keeps every throw. Join and renumbering take a
//!   free number from [`JOINED_GROUPS`], apart from every pad's. A number
//!   only has to differ from the other groups on its tick.
//! - **Shared settings.** A trigger's notes share their chance, condition
//!   and on or off ([`Clip::groups_ok`]); everything else is each note's
//!   own. A change to one changes all ([`whole_triggers`]); `Clip::apply`
//!   refuses an edit that would leave them apart, and a clip that is read
//!   or checked with them apart is refused.
//! - **Clips of pad hits** have no groups: their hits roll as they always
//!   did, and the player ignores the number there.
//! - **Edits never merge two triggers.** A trigger moved or copied whole
//!   stays one trigger, with a free number when its new tick already has
//!   its number; notes moved or copied without the rest of their trigger
//!   decide alone ([`settle_moved`], [`settle_copied`]).

use crate::{Clip, Cond, Holds, Note, Tick};
use std::collections::HashMap;
use std::ops::RangeInclusive;

/// The group a pad's notes take in a clip of notes: its number plus one
/// (1 to 128), so they decide with the key its hit has in a clip of pad
/// hits.
pub const fn pad_group(pad: u8) -> u8 {
    if pad < 128 {
        pad + 1
    } else {
        0
    }
}

/// The numbers Join and renumbering give, apart from every pad's.
pub const JOINED_GROUPS: RangeInclusive<u8> = 129..=255;

/// What a trigger's notes share.
fn shared(n: &Note) -> (u8, Cond, bool) {
    (n.chance, n.cond, n.on)
}

/// Whether `notes` of a clip that holds `holds` keep the rules: none
/// grouped in a clip of pad hits; in a clip of notes, a trigger's notes
/// share their chance, condition and on.
pub(crate) fn notes_ok(holds: Holds, notes: &[Note]) -> bool {
    match holds {
        Holds::Hits => notes.iter().all(|n| n.group == 0),
        Holds::Notes => {
            let mut seen: HashMap<(Tick, u8), (u8, Cond, bool)> = HashMap::new();
            notes
                .iter()
                .filter(|n| n.group != 0)
                .all(|n| *seen.entry((n.tick, n.group)).or_insert_with(|| shared(n)) == shared(n))
        }
    }
}

impl Clip {
    /// Whether the clip's trigger groups keep the rules: a clip of pad
    /// hits has none, and each trigger of a clip of notes has one chance,
    /// one condition and one on or off. Its original (before Mutate) too.
    pub fn groups_ok(&self) -> bool {
        notes_ok(self.holds, &self.notes)
            && self
                .original
                .as_ref()
                .is_none_or(|o| notes_ok(self.holds, &o.notes))
    }

    /// The notes on tick `tick` in trigger `group` (indexes into
    /// `self.notes`); none for group 0.
    pub fn trigger(&self, tick: Tick, group: u8) -> Vec<usize> {
        if group == 0 {
            return Vec::new();
        }
        (0..self.notes.len())
            .filter(|&i| self.notes[i].tick == tick && self.notes[i].group == group)
            .collect()
    }
}

/// `sel` (indexes into `clip.notes`) grown to whole triggers, in index
/// order: a change to a note's chance, condition or on is a change to its
/// trigger's, so it takes every note of it.
pub fn whole_triggers(clip: &Clip, sel: &[usize]) -> Vec<usize> {
    let mut keys: Vec<(Tick, u8)> = sel
        .iter()
        .filter_map(|&i| clip.notes.get(i))
        .filter(|n| n.group != 0 && clip.holds == Holds::Notes)
        .map(|n| (n.tick, n.group))
        .collect();
    let mut out: Vec<usize> = sel
        .iter()
        .copied()
        .filter(|&i| i < clip.notes.len())
        .collect();
    if !keys.is_empty() {
        keys.sort_unstable();
        keys.dedup();
        out.extend((0..clip.notes.len()).filter(|&i| {
            keys.binary_search(&(clip.notes[i].tick, clip.notes[i].group))
                .is_ok()
        }));
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Every free number of [`JOINED_GROUPS`] is taken on a tick: the edit
/// that needed one is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupsFull {
    pub tick: Tick,
}

/// The numbers taken on each tick by `clip`'s grouped notes less one
/// equal to each of `erase` (the notes an edit leaves where they are).
fn staying(clip: &Clip, erase: &[Note]) -> HashMap<Tick, Vec<u8>> {
    let mut out_count: HashMap<Note, usize> = HashMap::new();
    for n in erase.iter().filter(|n| n.group != 0) {
        *out_count.entry(*n).or_default() += 1;
    }
    let mut used: HashMap<Tick, Vec<u8>> = HashMap::new();
    for n in clip.notes.iter().filter(|n| n.group != 0) {
        if let Some(c) = out_count.get_mut(n).filter(|c| **c > 0) {
            *c -= 1;
            continue;
        }
        used.entry(n.tick).or_default().push(n.group);
    }
    used
}

/// The core of [`settle_moved`] and [`settle_copied`]: `arriving[i]` came
/// from `from[i]`, a note of `clip`; `erase` the notes the edit takes out,
/// `beside` notes it puts in already settled.
fn settle(
    clip: &Clip,
    erase: &[Note],
    beside: &[Note],
    from: &[Note],
    arriving: &mut [Note],
    copy: bool,
) -> Result<(), GroupsFull> {
    if clip.holds == Holds::Hits {
        for n in arriving.iter_mut() {
            n.group = 0;
        }
        return Ok(());
    }
    let mut used = staying(clip, erase);
    for n in beside.iter().filter(|n| n.group != 0) {
        used.entry(n.tick).or_default().push(n.group);
    }
    // How many notes each trigger of the clip has.
    let mut size: HashMap<(Tick, u8), usize> = HashMap::new();
    for n in clip.notes.iter().filter(|n| n.group != 0) {
        *size.entry((n.tick, n.group)).or_default() += 1;
    }
    // The arriving notes by the trigger they came from, in a fixed order.
    let mut parcels: Vec<((Tick, u8), Vec<usize>)> = Vec::new();
    for (i, f) in from.iter().enumerate().take(arriving.len()) {
        if f.group == 0 {
            continue;
        }
        let key = (f.tick, f.group);
        match parcels.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => v.push(i),
            None => parcels.push((key, vec![i])),
        }
    }
    parcels.sort_by_key(|(k, _)| *k);
    // Notes that keep their tick keep their trigger, first, so a trigger
    // arriving there never takes their number.
    for ((tick, group), members) in &parcels {
        for &i in members {
            if !copy && arriving[i].tick == *tick {
                arriving[i].group = *group;
                used.entry(*tick).or_default().push(*group);
            }
        }
    }
    for ((tick, group), members) in parcels {
        let moved: Vec<usize> = members
            .into_iter()
            .filter(|&i| copy || arriving[i].tick != tick)
            .collect();
        let Some(&m) = moved.first() else {
            continue;
        };
        let first = arriving[m].tick;
        let whole = moved.len() == size.get(&(tick, group)).copied().unwrap_or(0);
        if whole && moved.iter().all(|&i| arriving[i].tick == first) {
            // The whole trigger, to one tick: still one trigger.
            let taken = used.entry(first).or_default();
            let number = if taken.contains(&group) {
                JOINED_GROUPS
                    .clone()
                    .find(|g| !taken.contains(g))
                    .ok_or(GroupsFull { tick: first })?
            } else {
                group
            };
            taken.push(number);
            for &i in &moved {
                arriving[i].group = number;
            }
        } else {
            // Part of a trigger, or spread over ticks: the notes that
            // left its tick decide alone.
            for &i in &moved {
                arriving[i].group = 0;
            }
        }
    }
    Ok(())
}

/// The groups of notes an edit moves: `from[i]`, a note of `clip` the
/// edit takes out (with the rest of `erase`), comes back as `to[i]`. A
/// trigger whose every note moves, all to one tick, stays one trigger
/// there, with a free number when that tick already has its number;
/// notes that move without the rest of their trigger, or apart from it,
/// decide alone there; notes that keep their tick keep their trigger.
pub fn settle_moved(
    clip: &Clip,
    erase: &[Note],
    from: &[Note],
    to: &mut [Note],
) -> Result<(), GroupsFull> {
    settle(clip, erase, &[], from, to, false)
}

/// The groups of `copies` of `from` (notes of `clip`, which stay) that an
/// edit puts in, taking `erase` out: a trigger copied whole, to one tick,
/// is a new trigger there, with a free number when that tick already has
/// its number; a copy of part of a trigger decides alone.
pub fn settle_copied(
    clip: &Clip,
    erase: &[Note],
    from: &[Note],
    copies: &mut [Note],
) -> Result<(), GroupsFull> {
    settle(clip, erase, &[], from, copies, true)
}

/// [`settle_copied`] beside `beside`, notes the same edit puts in that
/// are settled already (a split's earlier parts).
pub fn settle_copied_beside(
    clip: &Clip,
    erase: &[Note],
    beside: &[Note],
    from: &[Note],
    copies: &mut [Note],
) -> Result<(), GroupsFull> {
    settle(clip, erase, beside, from, copies, true)
}

/// The `sel` notes of `clip` (indexes, in index order) to copy elsewhere,
/// a clipboard or Duplicate: a trigger group's notes keep their number
/// only when all of its notes are copied; a part of one decides alone.
pub fn copy_out(clip: &Clip, sel: &[usize]) -> Vec<Note> {
    let mut s: Vec<usize> = sel
        .iter()
        .copied()
        .filter(|&i| i < clip.notes.len())
        .collect();
    s.sort_unstable();
    s.dedup();
    let mut notes: Vec<Note> = s.iter().map(|&i| clip.notes[i]).collect();
    if clip.holds == Holds::Notes {
        let mut size: HashMap<(Tick, u8), usize> = HashMap::new();
        for n in clip.notes.iter().filter(|n| n.group != 0) {
            *size.entry((n.tick, n.group)).or_default() += 1;
        }
        let mut picked: HashMap<(Tick, u8), usize> = HashMap::new();
        for n in notes.iter().filter(|n| n.group != 0) {
            *picked.entry((n.tick, n.group)).or_default() += 1;
        }
        for n in notes.iter_mut().filter(|n| n.group != 0) {
            let k = (n.tick, n.group);
            if picked.get(&k) != size.get(&k) {
                n.group = 0;
            }
        }
    }
    notes
}

/// Notes the edit puts in fresh (`new`, none copied from the clip), taking
/// `erase` out: each with a number joins the trigger of that number on
/// its tick, taking its shared settings, when one is there and `join`
/// says it may (the same pad's step, completed); otherwise the new notes
/// with one number on one tick get a free number when the tick's number
/// is taken.
pub fn settle_new(
    clip: &Clip,
    erase: &[Note],
    new: &mut [Note],
    join: bool,
) -> Result<(), GroupsFull> {
    if clip.holds == Holds::Hits {
        for n in new.iter_mut() {
            n.group = 0;
        }
        return Ok(());
    }
    let mut used = staying(clip, erase);
    // Each new note's own (tick, number), before any is renumbered.
    let was: Vec<(Tick, u8)> = new.iter().map(|n| (n.tick, n.group)).collect();
    let mut keys: Vec<(Tick, u8)> = was.iter().copied().filter(|k| k.1 != 0).collect();
    keys.sort_unstable();
    keys.dedup();
    for (tick, group) in keys {
        let taken = used.entry(tick).or_default();
        let mut model = None;
        let number = if !taken.contains(&group) {
            group
        } else if join {
            // Join the trigger there: take its settings.
            model = clip
                .notes
                .iter()
                .find(|n| n.tick == tick && n.group == group && !erase.contains(n))
                .copied();
            group
        } else {
            JOINED_GROUPS
                .clone()
                .find(|g| !taken.contains(g))
                .ok_or(GroupsFull { tick })?
        };
        taken.push(number);
        for (n, _) in new
            .iter_mut()
            .zip(&was)
            .filter(|(_, k)| **k == (tick, group))
        {
            n.group = number;
            if let Some(m) = model {
                (n.chance, n.cond, n.on) = (m.chance, m.cond, m.on);
            }
        }
    }
    Ok(())
}

/// Recorded notes (a take's, or CAPTURE's) going into `clip`: a pad's
/// notes on a tick join the trigger there of their number when it has
/// their chance, condition and on, as a pad's second hit on a tick of a
/// clip of pad hits rolls with its first. When it has others (a step
/// edited since), they are a trigger of their own beside it, with a free
/// number, so they play as they were recorded. Nothing is refused: with
/// every number taken on a tick, they decide alone.
pub fn settle_recorded(clip: &Clip, new: &mut [Note]) {
    if clip.holds == Holds::Hits {
        for n in new.iter_mut() {
            n.group = 0;
        }
        return;
    }
    let mut used = staying(clip, &[]);
    let was: Vec<(Tick, u8)> = new.iter().map(|n| (n.tick, n.group)).collect();
    let mut keys: Vec<(Tick, u8)> = was.iter().copied().filter(|k| k.1 != 0).collect();
    keys.sort_unstable();
    keys.dedup();
    for (tick, group) in keys {
        let mine = new
            .iter()
            .zip(&was)
            .find(|(_, k)| **k == (tick, group))
            .map(|(n, _)| shared(n));
        let there = clip
            .notes
            .iter()
            .find(|n| n.tick == tick && n.group == group)
            .map(shared);
        let taken = used.entry(tick).or_default();
        let number = if !taken.contains(&group) || (there.is_some() && there == mine) {
            group
        } else {
            JOINED_GROUPS
                .clone()
                .find(|g| !taken.contains(g))
                .unwrap_or(0)
        };
        taken.push(number);
        for (n, _) in new
            .iter_mut()
            .zip(&was)
            .filter(|(_, k)| **k == (tick, group))
        {
            n.group = number;
        }
    }
}

/// `edit` made whole for `clip`'s triggers, or None when it is already:
/// a trigger whose notes the edit puts in with new shared settings (a
/// changed chance, condition or on) takes them on its notes the edit
/// leaves where they are, so a change to some of a trigger's notes is a
/// change to all of them. The app gives every edit to a clip through this
/// (`Takes::edit`). The edit's numbers must already be settled
/// ([`settle_moved`], [`settle_copied`], [`settle_new`]): a note it puts
/// in joins the trigger of its number on its tick.
pub fn follow(clip: &Clip, edit: &crate::steps::Edit) -> Option<crate::steps::Edit> {
    if clip.holds == Holds::Hits || edit.add.iter().all(|n| n.group == 0) {
        return None;
    }
    let mut wants: HashMap<(Tick, u8), (u8, Cond, bool)> = HashMap::new();
    for n in edit.add.iter().filter(|n| n.group != 0) {
        wants.entry((n.tick, n.group)).or_insert_with(|| shared(n));
    }
    let mut out_count: HashMap<Note, usize> = HashMap::new();
    for n in edit.erase.iter().filter(|n| n.group != 0) {
        *out_count.entry(*n).or_default() += 1;
    }
    let mut more = crate::steps::Edit::default();
    for n in clip.notes.iter().filter(|n| n.group != 0) {
        if let Some(c) = out_count.get_mut(n).filter(|c| **c > 0) {
            *c -= 1;
            continue;
        }
        if let Some(&(chance, cond, on)) = wants.get(&(n.tick, n.group)) {
            if shared(n) != (chance, cond, on) {
                more.erase.push(*n);
                more.add.push(Note {
                    chance,
                    cond,
                    on,
                    ..*n
                });
            }
        }
    }
    if more.erase.is_empty() {
        return None;
    }
    let mut whole = edit.clone();
    whole.erase.extend(more.erase);
    whole.add.extend(more.add);
    Some(whole)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(tick: Tick, pitch: u8, group: u8) -> Note {
        Note {
            group,
            ..Note::new(tick, 240, pitch, 100)
        }
    }

    fn clip(notes: &[Note]) -> Clip {
        let mut c = Clip::new("groups", 3840);
        c.holds = Holds::Notes;
        c.notes = notes.to_vec();
        c
    }

    /// A trigger's notes share chance, condition and on; a clip of pad
    /// hits has no groups.
    #[test]
    fn a_trigger_has_one_chance_one_condition_and_one_on() {
        let mut c = clip(&[note(0, 60, 1), note(0, 64, 1), note(0, 67, 2)]);
        assert!(c.groups_ok());
        c.notes[1].chance = 50;
        assert!(!c.groups_ok());
        c.notes[0].chance = 50;
        assert!(c.groups_ok());
        c.notes[2].on = false;
        assert!(c.groups_ok(), "another trigger");
        c.notes[1].cond = Cond::Fill;
        assert!(!c.groups_ok());
        let mut hits = clip(&[note(0, 3, 0)]);
        hits.holds = Holds::Hits;
        assert!(hits.groups_ok());
        hits.notes[0].group = 4;
        assert!(!hits.groups_ok());
    }

    /// A selection grows to whole triggers, and only on their tick.
    #[test]
    fn a_selection_grows_to_whole_triggers() {
        let c = clip(&[
            note(0, 60, 1),
            note(0, 64, 1),
            note(0, 48, 0),
            note(240, 60, 1),
            note(0, 67, 1),
        ]);
        assert_eq!(whole_triggers(&c, &[1]), vec![0, 1, 4]);
        assert_eq!(whole_triggers(&c, &[2]), vec![2]);
        assert_eq!(whole_triggers(&c, &[3]), vec![3]);
    }

    /// A whole trigger moved stays one, renumbered where its new tick has
    /// its number; part of one moved decides alone; a pitch change keeps
    /// it.
    #[test]
    fn moves_keep_whole_triggers_and_never_merge_two() {
        let c = clip(&[
            note(0, 60, 1),
            note(0, 64, 1),
            note(240, 62, 1),
            note(240, 65, 1),
        ]);
        // The chord on 0 moves to 240, where trigger 1 is: a new number.
        let from = vec![c.notes[0], c.notes[1]];
        let mut to: Vec<Note> = from.iter().map(|n| Note { tick: 240, ..*n }).collect();
        settle_moved(&c, &from, &from, &mut to).unwrap();
        assert_eq!(to[0].group, 129);
        assert_eq!(to[1].group, 129);
        // To an empty tick: it keeps its number.
        let mut to: Vec<Note> = from.iter().map(|n| Note { tick: 480, ..*n }).collect();
        settle_moved(&c, &from, &from, &mut to).unwrap();
        assert_eq!((to[0].group, to[1].group), (1, 1));
        // One note of it moved: alone.
        let one = vec![c.notes[0]];
        let mut to = vec![Note {
            tick: 480,
            ..c.notes[0]
        }];
        settle_moved(&c, &one, &one, &mut to).unwrap();
        assert_eq!(to[0].group, 0);
        // One note transposed: still in its trigger.
        let mut to = vec![Note {
            note: 72,
            ..c.notes[0]
        }];
        settle_moved(&c, &one, &one, &mut to).unwrap();
        assert_eq!(to[0].group, 1);
        // The two notes spread over two ticks: both alone.
        let mut to = vec![
            Note {
                tick: 480,
                ..c.notes[0]
            },
            Note {
                tick: 720,
                ..c.notes[1]
            },
        ];
        settle_moved(&c, &from, &from, &mut to).unwrap();
        assert_eq!((to[0].group, to[1].group), (0, 0));
        // Both chords to 960: two triggers, not one.
        let all = c.notes.clone();
        let mut to: Vec<Note> = all.iter().map(|n| Note { tick: 960, ..*n }).collect();
        settle_moved(&c, &all, &all, &mut to).unwrap();
        assert_eq!(
            to.iter().map(|n| n.group).collect::<Vec<_>>(),
            vec![1, 1, 129, 129]
        );
        // A trigger arriving where one with its number stays in the same
        // edit (transposed) takes a free number; the one staying keeps it.
        let c = clip(&[note(0, 60, 129), note(240, 62, 129)]);
        let from = c.notes.clone();
        let mut to = vec![
            Note {
                tick: 240,
                ..from[0]
            },
            Note {
                note: 63,
                ..from[1]
            },
        ];
        settle_moved(&c, &from, &from, &mut to).unwrap();
        assert_eq!((to[0].group, to[1].group), (130, 129));
    }

    /// A whole trigger copied is a new trigger; a copy of part of one
    /// decides alone; a tick with every number taken refuses.
    #[test]
    fn copies_never_join_a_trigger_already_there() {
        let c = clip(&[note(0, 60, 1), note(0, 64, 1), note(240, 62, 1)]);
        let from = vec![c.notes[0], c.notes[1]];
        let mut copies: Vec<Note> = from.iter().map(|n| Note { tick: 240, ..*n }).collect();
        settle_copied(&c, &[], &from, &mut copies).unwrap();
        assert_eq!((copies[0].group, copies[1].group), (129, 129));
        let mut copies = vec![Note {
            tick: 480,
            ..c.notes[0]
        }];
        settle_copied(&c, &[], &[c.notes[0]], &mut copies).unwrap();
        assert_eq!(copies[0].group, 0);
        // Every number taken on 240.
        let mut full: Vec<Note> = vec![note(0, 60, 1)];
        full.extend(JOINED_GROUPS.map(|g| note(240, 1, g)));
        full.push(note(240, 2, 1));
        let f = clip(&full);
        let mut copies = vec![Note {
            tick: 240,
            ..f.notes[0]
        }];
        assert_eq!(
            settle_copied(&f, &[], &[f.notes[0]], &mut copies),
            Err(GroupsFull { tick: 240 })
        );
        // A clip of pad hits: never a group.
        let mut hits = clip(&[note(0, 3, 0)]);
        hits.holds = Holds::Hits;
        let mut copies = vec![note(240, 3, 4)];
        settle_copied(&hits, &[], &[hits.notes[0]], &mut copies).unwrap();
        assert_eq!(copies[0].group, 0);
    }

    /// New notes join the same pad's trigger on their step when asked,
    /// taking its settings; otherwise they get a number of their own.
    #[test]
    fn new_notes_join_only_when_asked() {
        let mut c = clip(&[note(0, 60, 1)]);
        c.notes[0].chance = 50;
        let mut new = vec![note(0, 64, 1)];
        settle_new(&c, &[], &mut new, true).unwrap();
        assert_eq!((new[0].group, new[0].chance), (1, 50));
        let mut new = vec![note(0, 64, 1)];
        settle_new(&c, &[], &mut new, false).unwrap();
        assert_eq!((new[0].group, new[0].chance), (129, 100));
        // The note there taken out by the same edit: nothing to join.
        let mut new = vec![note(0, 64, 1)];
        settle_new(&c, &[c.notes[0]], &mut new, false).unwrap();
        assert_eq!(new[0].group, 1);
        // Two new triggers: the first renumbered to 129 never pulls in
        // the other, which has 129 already.
        let mut new = vec![note(0, 64, 1), note(0, 67, 129)];
        settle_new(&c, &[], &mut new, false).unwrap();
        assert_eq!((new[0].group, new[1].group), (129, 130));
    }

    /// A recorded note joins its pad's trigger on its tick when that has
    /// its settings, and is a trigger of its own beside one that has
    /// others.
    #[test]
    fn recorded_notes_join_a_trigger_with_their_settings() {
        let mut c = clip(&[note(0, 60, 1), note(240, 62, 1)]);
        c.notes[1].chance = 50;
        let mut new = vec![note(0, 64, 1), note(240, 65, 1), note(480, 67, 1)];
        settle_recorded(&c, &mut new);
        assert_eq!(
            new.iter().map(|n| (n.group, n.chance)).collect::<Vec<_>>(),
            vec![(1, 100), (129, 100), (1, 100)]
        );
        c.notes.extend(new);
        assert!(c.groups_ok());
        // A clip of pad hits has none.
        c.holds = Holds::Hits;
        let mut new = vec![note(0, 64, 1)];
        settle_recorded(&c, &mut new);
        assert_eq!(new[0].group, 0);
    }

    /// A change to one note of a trigger reaches the others.
    #[test]
    fn a_change_to_one_note_of_a_trigger_is_a_change_to_all() {
        let c = clip(&[note(0, 60, 1), note(0, 64, 1), note(0, 67, 2)]);
        let mut edit = crate::steps::Edit::default();
        edit.erase.push(c.notes[1]);
        edit.add.push(Note {
            chance: 50,
            ..c.notes[1]
        });
        let whole = follow(&c, &edit).unwrap();
        let mut after = c.clone();
        after.apply(&whole).unwrap();
        assert!(after.groups_ok());
        assert_eq!(
            after
                .notes
                .iter()
                .map(|n| (n.note, n.chance))
                .collect::<Vec<_>>(),
            vec![(67, 100), (64, 50), (60, 50)]
        );
        // The edit alone is refused.
        let mut alone = c.clone();
        assert_eq!(alone.apply(&edit), Err(crate::ModelError::BadGroup));
        assert_eq!(alone, c);
        // Nothing to follow: None.
        edit.add[0].chance = 100;
        assert_eq!(follow(&c, &edit), None);
    }

    /// A small seeded generator (xorshift), so a failure names its seed.
    struct Dice(u64);
    impl Dice {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n.max(1) as u64) as usize
        }
        fn pick<T: Copy>(&mut self, of: &[T]) -> T {
            of[self.below(of.len())]
        }
        fn some(&mut self, len: usize) -> Vec<usize> {
            let p = 1 + self.below(4);
            (0..len).filter(|_| self.below(p + 1) == 0).collect()
        }
    }

    /// Edits, randomised (the trigger groups design): seeded clips of
    /// notes with chords and lone notes, through random moves, copies,
    /// pastes, duplicates, splits, speed and length changes, Timing
    /// Correct, transposes, deletes and changes of chance and condition
    /// to some notes of a chord. Each note carries its first trigger in
    /// its velocity, which these edits keep: no edit may put notes of two
    /// first triggers into one group, or leave a trigger with two
    /// settings, and every edit an edit function gives applies.
    #[test]
    fn no_edit_merges_two_triggers_or_splits_their_settings() {
        use crate::notes;
        let mut joined = 0;
        for seed in 1..=150u64 {
            let mut d = Dice(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let mut c = Clip::new("edits", 3840);
            c.holds = Holds::Notes;
            let mut id = 0u8;
            for step in 0..16 {
                for _ in 0..d.below(3) {
                    id += 1;
                    let tick = step * 240 + d.pick(&[0, 0, 30, -20]).max(0);
                    let group = d.pick(&[0, 1, 2, 3, 129, 130]);
                    let (chance, on) = (d.pick(&[100, 50, 25]), d.below(8) != 0);
                    let size = if group == 0 { 1 } else { 1 + d.below(3) };
                    if c.trigger(tick, group).is_empty() || group == 0 {
                        for _ in 0..size {
                            c.notes.push(Note {
                                chance,
                                on,
                                group,
                                ..Note::new(tick, 60 + d.below(60) as i64, d.below(128) as u8, id)
                            });
                        }
                    }
                }
            }
            assert!(c.groups_ok(), "seed {seed}");
            for round in 0..40 {
                let n = c.notes.len();
                let sel = d.some(n);
                let edit = match d.below(16) {
                    0 => notes::nudge(
                        &c,
                        &sel,
                        d.pick(&[-2, -1, 1, 3]),
                        notes::Snap::Relative(240),
                    ),
                    1 => notes::nudge(&c, &sel, 1, notes::Snap::Absolute(480)),
                    2 => notes::shift(&c, &sel, d.pick(&[-240, 0, 30, 480]), d.pick(&[-3, 0, 5])),
                    3 => notes::edit_start(&c, &sel, d.pick(&[-30, 20])),
                    4 => notes::transpose(&c, &sel, d.pick(&[-12, 7])),
                    5 => notes::duplicate(&c, &sel, d.pick(&[240, 960])),
                    6 => {
                        let copy = copy_out(&c, &sel);
                        let from = copy.iter().map(|x| x.tick).min().unwrap_or(0);
                        notes::paste(&c, &copy, d.pick(&[0, 240, 1000, 3600]), from)
                    }
                    7 => notes::split(&c, &sel, 2),
                    8 => notes::double_speed(&c),
                    9 => notes::half_speed(&c),
                    10 => notes::double_length(&c),
                    11 => notes::timing_correct(
                        &c,
                        notes::Tc {
                            division: d.pick(&[240, 480, 960]),
                            ..notes::Tc::default()
                        },
                        &notes::Which::Selected(sel.clone()),
                    ),
                    12 => notes::delete(&c, &sel),
                    13 => notes::chance(&c, &sel, d.pick(&[-25, 10])),
                    14 => notes::condition(&c, &sel, 1),
                    _ => notes::add(
                        &c,
                        Note {
                            group: d.pick(&[1, 2, 129]),
                            ..Note::new(240 * d.below(16) as i64, 120, d.below(128) as u8, 127)
                        },
                    ),
                };
                if edit.is_empty() {
                    continue;
                }
                let edit = follow(&c, &edit).unwrap_or(edit);
                let what = format!("seed {seed}, round {round}");
                c.apply(&edit).unwrap_or_else(|e| panic!("{what}: {e:?}"));
                assert!(c.groups_ok(), "{what}");
                let mut first: HashMap<(Tick, u8), u8> = HashMap::new();
                for x in c.notes.iter().filter(|x| x.group != 0) {
                    let was = *first.entry((x.tick, x.group)).or_insert(x.velocity);
                    assert_eq!(was, x.velocity, "{what}: two triggers in one at {}", x.tick);
                }
                joined += first.len();
                if c.notes.len() > 1500 {
                    break;
                }
            }
        }
        assert!(joined > 1000, "{joined}");
    }
}
