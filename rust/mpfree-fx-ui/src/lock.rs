//! Lock mode: while a step is held on the step screen, a page's turns lock
//! parameters on that step instead of setting them, as on an Elektron
//! box (the time engine's parameter locks, on the same parameter
//! address).
//!
//! The app passes the held step's locks on the effect shown with
//! [`FxPanel::set_locks`](crate::FxPanel::set_locks) or
//! [`PerformPanel::set_locks`](crate::PerformPanel::set_locks), and `None`
//! when the step is let go. While it is held:
//!
//! - a turn (a tile, a Q-Link, the wheel) gives a `Lock` action with the
//!   value for the held step, starting from the step's lock if it has one
//!   or else from the effect's own value; the effect's own value stays;
//! - the page shows what the held step plays: its locks over the effect's
//!   values, locked tiles in the warning colour, and the drawings drawn
//!   from the locked values;
//! - `unlock` takes the selected tile's lock off (`Unlock`), and
//!   `unlock_all` every lock the step has on the effect;
//! - on the Perform page a pad tap locks that pad's Hold on (so the step
//!   plays the pad's effect), or takes it off again; the XY pad locks the
//!   DJ filter and the echo where the finger is.
//!
//! The app sends a lock to the time engine for the held steps (an insert
//! on the clip's own track by its `mpfree_param::RelAddr`), as it sends a
//! knob turned while a step is held.

use crate::state::FxState;

/// The held step and its locks on the effect shown.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Locks {
    /// What is held, for the page: "Step 5", "Steps 5 and 9".
    pub held: String,
    /// The held step's locks on this effect: parameter id and value.
    pub values: Vec<(u16, f32)>,
}

impl Locks {
    /// A held step with no locks yet.
    pub fn new(held: impl Into<String>) -> Locks {
        Locks {
            held: held.into(),
            values: Vec::new(),
        }
    }

    /// The lock on parameter `id`, if any.
    pub fn get(&self, id: u16) -> Option<f32> {
        self.values.iter().find(|l| l.0 == id).map(|l| l.1)
    }

    pub(crate) fn put(&mut self, id: u16, v: f32) {
        match self.values.iter_mut().find(|l| l.0 == id) {
            Some(l) => l.1 = v,
            None => self.values.push((id, v)),
        }
    }

    /// Takes the lock on `id` off; whether there was one.
    pub(crate) fn take(&mut self, id: u16) -> bool {
        let n = self.values.len();
        self.values.retain(|l| l.0 != id);
        self.values.len() != n
    }

    /// `st` with the locks over its values: what the held step plays.
    pub fn over(&self, st: &FxState) -> FxState {
        let mut s = st.clone();
        for &(id, v) in &self.values {
            if let Some(i) = s.index(id) {
                s.values[i] = s.params[i].clamp(v);
            }
        }
        s
    }

    /// The line the page shows: "Step 5 · 2 locks".
    pub fn line(&self) -> String {
        match self.values.len() {
            0 => format!("{} · no locks", self.held),
            1 => format!("{} · 1 lock", self.held),
            n => format!("{} · {n} locks", self.held),
        }
    }
}
