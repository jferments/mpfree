//! What a clip's instrument locks were made for (step sequencer refactor
//! F1): the instrument kind and its controls' names, so a saved lock
//! never quietly turns a different control.
//!
//! A lock on a track's instrument (`OnTrack::Inst`) names a control by
//! its number on that instrument. Built-in instruments never change a
//! number (`mpfree-node` keeps them for saved projects), and a plugin's
//! come from the plugin; but the same number on another instrument is
//! another control (Poly's Cutoff is not FM's control of that number).
//! So a clip remembers, with its first instrument lock, the instrument it
//! was made for ([`Clip::made_for`]: its node id, its name, and the name
//! of each control it locks), and the track remembers the instrument it
//! plays now ([`Track::plays`], told by the app, not saved).
//!
//! - A clip's instrument locks (and its move lanes on the instrument)
//!   play only while the track plays the same
//!   kind, and each only while its control still has the name it was
//!   made with ([`Clip::lock_plays`]). Otherwise they are kept, not
//!   played, and the Steps say so; nothing is deleted.
//! - All of a clip's instrument locks are made for one instrument: the
//!   app refuses a new one for another kind until the old ones are taken
//!   off ([`Clip::inst_refuses`]).
//! - [`Clip::tag_inst`] keeps the tag true after any change: a clip with
//!   no instrument locks has none, one whose locks have none gets the
//!   track's instrument (an older project's locks play as before), and
//!   one made for the track's kind learns the names of controls locked
//!   since.
//! - Saved as `madefor <node> <name>` and a `madename <id> <name>` line a
//!   control, before the clip's notes; an older build skips them.

use crate::{Clip, Lock};
use mpfree_moves::{Lane, Target};
use mpfree_param::{OnTrack, RelAddr};
use std::sync::Arc;

/// An instrument: its node id (`"mpfree.poly"`, stable in saved
/// projects), the name shown ("Poly"), and controls by number with their
/// names. On a track ([`crate::Track::plays`]) every control; on a clip
/// ([`Clip::made_for`]) the ones its locks name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inst {
    pub node: String,
    pub name: String,
    pub params: Vec<(u16, String)>,
}

impl Inst {
    /// Control `id`'s name, if it is known.
    pub fn param_name(&self, id: u16) -> Option<&str> {
        self.params
            .iter()
            .find(|(p, _)| *p == id)
            .map(|(_, n)| n.as_str())
    }

    /// "Poly's Cutoff", or "Poly's control 12" for a name not known.
    pub fn control(&self, id: u16) -> String {
        match self.param_name(id) {
            Some(n) => format!("{}'s {n}", self.name),
            None => format!("{}'s control {id}", self.name),
        }
    }
}

/// The instrument a track plays now ([`crate::Track::plays`]): told by
/// the app as it changes, never saved, and equal to any other (a track is
/// the same track whatever it plays now, so a project compares as saved).
#[derive(Clone, Debug, Default)]
pub struct Plays(pub Option<Arc<Inst>>);

impl PartialEq for Plays {
    fn eq(&self, _: &Plays) -> bool {
        true
    }
}

impl Plays {
    pub fn get(&self) -> Option<&Inst> {
        self.0.as_deref()
    }

    /// Its node id, None when not told.
    pub fn node(&self) -> Option<&str> {
        self.get().map(|i| i.node.as_str())
    }
}

/// Where a track's knobs are now, as their own values (not moved): the
/// ones Continue's lanes on its clips move (its instrument's, and its
/// first send), so the lanes go with the knob
/// ([`mpfree_moves::Lane::follow_knob`]). Told by the app, never saved,
/// and equal to any other.
#[derive(Clone, Debug, Default)]
pub struct Knobs(pub Arc<[(RelAddr, f32)]>);

impl PartialEq for Knobs {
    fn eq(&self, _: &Knobs) -> bool {
        true
    }
}

impl Knobs {
    /// The value of the knob at `at`, if it is told.
    pub fn get(&self, at: RelAddr) -> Option<f32> {
        self.0.iter().find(|(k, _)| *k == at).map(|&(_, v)| v)
    }

    /// The knob a lane of Continue's on `target` goes with, if told.
    pub fn of(&self, target: Target) -> Option<f32> {
        match target {
            Target::Track(a) => self.get(a),
            Target::Fixed(_) => None,
        }
    }
}

/// The knob a lane of Continue's moves on its track, if it is one (its
/// instrument's, or its strip's send: composing tools S3).
pub fn continues_knob(lane: &Lane) -> Option<RelAddr> {
    match (lane.by, lane.target) {
        (mpfree_moves::By::Continue { .. }, Target::Track(a))
            if matches!(a.on, OnTrack::Inst | OnTrack::Strip) =>
        {
            Some(a)
        }
        _ => None,
    }
}

/// Whether lock `l` is on the track's instrument.
pub fn on_inst(l: &Lock) -> bool {
    l.addr.on == OnTrack::Inst
}

/// The control a move lane turns on the track's instrument, if it does.
pub fn lane_inst(lane: &Lane) -> Option<u16> {
    match lane.target {
        Target::Track(a) if a.on == OnTrack::Inst => Some(a.param),
        _ => None,
    }
}

impl Clip {
    /// The controls its instrument locks and move lanes turn, the
    /// original's too (what Restore puts back), in no order and maybe
    /// twice.
    pub fn inst_params(&self) -> impl Iterator<Item = u16> + '_ {
        let original = self.original.iter().flat_map(|o| &o.locks);
        let locks = self.locks.iter().chain(original).filter(|l| on_inst(l));
        let lanes = self
            .moves
            .lanes()
            .iter()
            .chain(self.original.iter().flat_map(|o| o.moves.lanes()));
        locks
            .map(|l| l.addr.param)
            .chain(lanes.filter_map(lane_inst))
    }

    /// Whether its instrument control `id` plays on a track that plays
    /// `plays`: when the clip was made for the same kind and the control
    /// has the name it was made with (or either is not known: an older
    /// project, a track the app hasn't told of).
    pub fn param_plays(&self, id: u16, plays: Option<&Inst>) -> bool {
        let (Some(made), Some(now)) = (&self.made_for, plays) else {
            return true;
        };
        if made.node != now.node {
            return false;
        }
        match (made.param_name(id), now.param_name(id)) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        }
    }

    /// Whether lock `l` plays on a track that plays `plays`: any lock not
    /// on the instrument does, an instrument lock as [`Clip::param_plays`].
    pub fn lock_plays(&self, l: &Lock, plays: Option<&Inst>) -> bool {
        !on_inst(l) || self.param_plays(l.addr.param, plays)
    }

    /// Whether move lane `lane` plays there, as a lock.
    pub fn lane_plays(&self, lane: &Lane, plays: Option<&Inst>) -> bool {
        lane_inst(lane).is_none_or(|id| self.param_plays(id, plays))
    }

    /// Whether its instrument locks are silent on `plays` (made for
    /// another kind, or a control renamed): what the Steps say. None when
    /// they play, or it has none.
    pub fn inst_silent(&self, plays: Option<&Inst>) -> Option<&Inst> {
        let made = self.made_for.as_ref()?;
        self.inst_params()
            .any(|id| !self.param_plays(id, plays))
            .then_some(made)
    }

    /// Why a new lock on control `param` of instrument `plays` can't go in
    /// (its instrument locks were made for another kind), else None.
    pub fn inst_refuses(&self, param: u16, plays: Option<&Inst>) -> Option<String> {
        let made = self.made_for.as_ref()?;
        let now = plays?;
        self.inst_params().next()?;
        if made.node == now.node {
            let renamed = matches!(
                (made.param_name(param), now.param_name(param)),
                (Some(a), Some(b)) if a != b
            );
            return renamed.then(|| {
                format!(
                    "{} has locks made for {}: take them off first",
                    now.control(param),
                    made.control(param)
                )
            });
        }
        Some(format!(
            "this pattern's {} locks don't play on {}: take them off first",
            made.name, now.name
        ))
    }

    /// Keep [`Clip::made_for`] true after a change, on a track that plays
    /// `plays`: none with no instrument locks; the track's instrument for
    /// locks with none (an older project's play as before); and, made
    /// for the track's kind, the names of the controls locked since.
    /// Whether it changed.
    pub fn tag_inst(&mut self, plays: Option<&Inst>) -> bool {
        let mut ids: Vec<u16> = self.inst_params().collect();
        if ids.is_empty() {
            return self.made_for.take().is_some();
        }
        let Some(now) = plays else {
            return false;
        };
        ids.sort_unstable();
        ids.dedup();
        let named = |id: u16| now.param_name(id).map(|n| (id, n.to_string()));
        match &mut self.made_for {
            None => {
                self.made_for = Some(Inst {
                    node: now.node.clone(),
                    name: now.name.clone(),
                    params: ids.into_iter().filter_map(named).collect(),
                });
                true
            }
            Some(made) if made.node == now.node => {
                let new: Vec<(u16, String)> = ids
                    .into_iter()
                    .filter(|&id| made.param_name(id).is_none())
                    .filter_map(named)
                    .collect();
                if new.is_empty() {
                    return false;
                }
                made.params.extend(new);
                made.params.sort_by_key(|(id, _)| *id);
                true
            }
            Some(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_param::RelAddr;

    fn inst(node: &str, name: &str, params: &[(u16, &str)]) -> Inst {
        Inst {
            node: node.into(),
            name: name.into(),
            params: params.iter().map(|&(i, n)| (i, n.to_string())).collect(),
        }
    }

    fn lock(param: u16) -> Lock {
        Lock::new(0, None, RelAddr::new(OnTrack::Inst, param), 0.5)
    }

    /// Made for Poly, a lock plays on Poly and not on FM; a lock on the
    /// strip plays on both; a control renamed under the same number is
    /// silent; and with nothing known, everything plays.
    #[test]
    fn a_lock_plays_only_on_the_instrument_it_was_made_for() {
        let poly = inst("mpfree.poly", "Poly", &[(3, "Cutoff"), (4, "Res")]);
        let fm = inst("mpfree.fm", "FM", &[(3, "Ratio")]);
        let mut c = Clip::new("Bass", 3840);
        c.locks.push(lock(3));
        let strip = Lock::new(0, None, RelAddr::new(OnTrack::Strip, 0), 0.0);
        c.locks.push(strip);
        assert!(c.lock_plays(&c.locks[0], Some(&fm)), "not tagged yet");
        assert!(c.tag_inst(Some(&poly)));
        assert_eq!(
            c.made_for,
            Some(inst("mpfree.poly", "Poly", &[(3, "Cutoff")]))
        );
        assert!(c.lock_plays(&c.locks[0], Some(&poly)));
        assert!(!c.lock_plays(&c.locks[0], Some(&fm)));
        assert!(c.lock_plays(&strip, Some(&fm)));
        assert!(c.lock_plays(&c.locks[0], None), "a track not told of");
        assert_eq!(
            c.inst_silent(Some(&fm)).map(|m| m.name.as_str()),
            Some("Poly")
        );
        assert_eq!(c.inst_silent(Some(&poly)), None);
        // A newer Poly that calls control 3 something else: silent.
        let renamed = inst("mpfree.poly", "Poly", &[(3, "Drive")]);
        assert!(!c.lock_plays(&c.locks[0], Some(&renamed)));
        assert_eq!(
            Clip::new("x", 1).inst_silent(Some(&fm)),
            None,
            "no locks: nothing to say"
        );
    }

    /// The tag follows the locks: a new control's name joins; another
    /// kind's lock is refused while the old ones are there; with the last
    /// instrument lock gone the tag goes, and the next lock tags afresh.
    #[test]
    fn the_tag_follows_the_locks_and_one_kind_at_a_time() {
        let poly = inst("mpfree.poly", "Poly", &[(3, "Cutoff"), (4, "Res")]);
        let fm = inst("mpfree.fm", "FM", &[(3, "Ratio")]);
        let mut c = Clip::new("Bass", 3840);
        assert!(!c.tag_inst(Some(&poly)), "no locks: no tag");
        c.locks.push(lock(3));
        c.tag_inst(Some(&poly));
        c.locks.push(lock(4));
        assert!(c.tag_inst(Some(&poly)));
        assert_eq!(c.made_for.as_ref().unwrap().param_name(4), Some("Res"));
        assert!(!c.tag_inst(Some(&poly)), "nothing new");
        assert!(!c.tag_inst(Some(&fm)), "another kind never retags");
        assert_eq!(c.inst_refuses(4, Some(&poly)), None);
        assert_eq!(
            c.inst_refuses(3, Some(&fm)).as_deref(),
            Some("this pattern's Poly locks don't play on FM: take them off first")
        );
        let renamed = inst("mpfree.poly", "Poly", &[(3, "Drive")]);
        assert_eq!(
            c.inst_refuses(3, Some(&renamed)).as_deref(),
            Some("Poly's Drive has locks made for Poly's Cutoff: take them off first")
        );
        c.locks.clear();
        assert!(c.tag_inst(Some(&fm)), "the tag goes with the last lock");
        assert_eq!(c.made_for, None);
        assert_eq!(c.inst_refuses(3, Some(&fm)), None);
        c.locks.push(lock(3));
        c.tag_inst(Some(&fm));
        assert_eq!(c.made_for.as_ref().unwrap().name, "FM");
        // Not told of the track's instrument: an untagged clip waits.
        let mut old = Clip::new("Old", 3840);
        old.locks.push(lock(3));
        assert!(!old.tag_inst(None));
        assert_eq!(old.made_for, None);
    }
}
