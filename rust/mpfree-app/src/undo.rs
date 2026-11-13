//! UNDO's histories for the pages whose edits are knob and fader turns:
//! the Synth tab's sound and the mixer's strips (Jay, 13 Nov 13:53Z: UNDO
//! follows the page you're on). The screen picks the page
//! ([`crate::gui::Player`]'s `undo`); these keep what each page did.
//!
//! An entry is one parameter turned from one value to another. A run of
//! turns of the same parameter, each within [`JOIN`] of the last, is one
//! entry, so a fader dragged across the screen comes back with one UNDO.
//! A new turn after an UNDO drops what could be redone, as the takes do.

use std::time::{Duration, Instant};

/// Turns of the same parameter this close together are one entry.
pub const JOIN: Duration = Duration::from_millis(1500);

/// The most entries a page keeps; the oldest go first.
pub const KEEP: usize = 200;

/// One parameter turned from `from` to `to`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Turn<K> {
    pub key: K,
    pub from: f32,
    pub to: f32,
    at: Instant,
}

/// One page's turns: what UNDO takes back (the last first) and what REDO
/// puts back.
#[derive(Debug)]
pub struct Turns<K> {
    done: Vec<Turn<K>>,
    undone: Vec<Turn<K>>,
}

impl<K> Default for Turns<K> {
    fn default() -> Self {
        Turns {
            done: Vec::new(),
            undone: Vec::new(),
        }
    }
}

impl<K: Copy + PartialEq> Turns<K> {
    /// `key` was turned from `from` to `to` at `now`. A turn that changed
    /// nothing is left out.
    pub fn turned(&mut self, key: K, from: f32, to: f32, now: Instant) {
        if from == to {
            return;
        }
        self.undone.clear();
        if let Some(last) = self.done.last_mut() {
            if last.key == key && now.saturating_duration_since(last.at) <= JOIN {
                last.to = to;
                last.at = now;
                if last.from == last.to {
                    // Turned back to where it started: nothing to undo.
                    self.done.pop();
                }
                return;
            }
        }
        if self.done.len() == KEEP {
            self.done.remove(0);
        }
        self.done.push(Turn {
            key,
            from,
            to,
            at: now,
        });
    }

    /// UNDO: the last turn, to be set back to its `from`.
    pub fn undo(&mut self) -> Option<Turn<K>> {
        let t = self.done.pop()?;
        self.undone.push(t);
        Some(t)
    }

    /// REDO: the turn last undone, to be set to its `to` again.
    pub fn redo(&mut self) -> Option<Turn<K>> {
        let t = self.undone.pop()?;
        self.done.push(t);
        Some(t)
    }

    /// Everything goes (a preset loaded, the engine switched: the old
    /// turns are of another sound).
    pub fn clear(&mut self) {
        self.done.clear();
        self.undone.clear();
    }

    /// What UNDO would take back, oldest first.
    pub fn done(&self) -> &[Turn<K>] {
        &self.done
    }

    /// The history as the list that holding UNDO opens shows it: `start`,
    /// each turn UNDO takes back, oldest first, then REDO's, the next one
    /// first, each named by `name`; and the row that is now.
    pub fn rows(&self, start: &str, name: impl Fn(&Turn<K>) -> String) -> (Vec<String>, usize) {
        let mut rows = vec![start.to_string()];
        rows.extend(self.done.iter().map(&name));
        let now = rows.len() - 1;
        rows.extend(self.undone.iter().rev().map(&name));
        (rows, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_of_turns_is_one_entry_and_undo_and_redo_walk_them() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut h = Turns::default();
        // A fader dragged: one entry from where it started.
        h.turned(1u16, 0.0, 0.2, at(0));
        h.turned(1, 0.2, 0.5, at(100));
        h.turned(1, 0.5, 0.7, at(300));
        // Another fader, then the first again long after: three entries.
        h.turned(2, 1.0, 0.0, at(400));
        h.turned(1, 0.7, 0.9, at(5000));
        assert_eq!(h.done().len(), 3);
        let u = h.undo().unwrap();
        assert_eq!((u.key, u.from, u.to), (1, 0.7, 0.9));
        let u = h.undo().unwrap();
        assert_eq!((u.key, u.from), (2, 1.0));
        let u = h.undo().unwrap();
        assert_eq!((u.key, u.from, u.to), (1, 0.0, 0.7));
        assert!(h.undo().is_none());
        let r = h.redo().unwrap();
        assert_eq!((r.key, r.to), (1, 0.7));
        // A new turn drops what could be redone.
        h.turned(3, 0.0, 1.0, at(6000));
        assert!(h.redo().is_none());
        // A turn that changes nothing, or a run back to its start, is none.
        h.turned(4, 0.5, 0.5, at(7000));
        h.turned(5, 0.1, 0.4, at(8000));
        h.turned(5, 0.4, 0.1, at(8100));
        assert_eq!(h.done().iter().map(|t| t.key).collect::<Vec<_>>(), [1, 3]);
        h.clear();
        assert!(h.done().is_empty() && h.undo().is_none());
    }

    #[test]
    fn the_rows_list_start_the_turns_then_redo_next_first() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut h = Turns::default();
        for k in 1..=4u16 {
            h.turned(k, 0.0, 1.0, at(k as u64 * 10));
        }
        h.undo();
        h.undo();
        let (rows, now) = h.rows("Start", |t| format!("knob {}", t.key));
        assert_eq!(rows, ["Start", "knob 1", "knob 2", "knob 3", "knob 4"]);
        assert_eq!(now, 2);
        // Redo's rows come next one first: after a redo, now moves on.
        h.redo();
        assert_eq!(h.rows("Start", |t| format!("knob {}", t.key)).1, 3);
    }

    #[test]
    fn the_oldest_turns_go_past_the_keep() {
        let t0 = Instant::now();
        let mut h = Turns::default();
        for k in 0..(KEEP as u32 + 5) {
            h.turned(k, 0.0, 1.0, t0);
        }
        assert_eq!(h.done().len(), KEEP);
        assert_eq!(h.done()[0].key, 5);
    }
}
