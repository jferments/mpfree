//! M14: one gesture recognizer per on-screen owner (a widget, a page).
//!
//! The GUI already gives each finger to the widget it first touched. Widgets
//! that want gestures register here with their own [`GestureConfig`]; each
//! gets a private [`GestureRecognizer`] fed only its own fingers. So a pad
//! held on one side and a swipe on the other never pair into a pinch, and a
//! widget that didn't register (a pad, a fader) never sees a gesture delay.

use crate::gesture::{Gesture, GestureConfig, GestureRecognizer};
use crate::pointer::PointerEvent;

#[derive(Debug, Clone)]
pub struct GestureRouter<K: Copy + Eq> {
    owners: Vec<(K, GestureRecognizer)>,
    scratch: Vec<PointerEvent>,
    out: Vec<Gesture>,
}

impl<K: Copy + Eq> Default for GestureRouter<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Copy + Eq> GestureRouter<K> {
    pub fn new() -> Self {
        Self {
            owners: Vec::new(),
            scratch: Vec::new(),
            out: Vec::new(),
        }
    }

    /// Give `key` gestures with `config`. Registering again resets it.
    pub fn register(&mut self, key: K, config: GestureConfig) {
        self.unregister(key);
        self.owners.push((key, GestureRecognizer::new(config)));
    }

    pub fn unregister(&mut self, key: K) {
        self.owners.retain(|(k, _)| *k != key);
    }

    pub fn is_registered(&self, key: K) -> bool {
        self.owners.iter().any(|(k, _)| *k == key)
    }

    /// Forget every owner, e.g. when the page changes.
    pub fn clear(&mut self) {
        self.owners.clear();
    }

    /// Feed one decoder frame whose events the GUI has tagged with their
    /// owner. Events of unregistered owners are skipped. Each owner's events
    /// go to its recognizer together, so its frame stays coherent.
    pub fn frame(&mut self, events: &[(K, PointerEvent)], out: &mut Vec<(K, Gesture)>) {
        for (key, recognizer) in &mut self.owners {
            self.scratch.clear();
            self.scratch
                .extend(events.iter().filter(|(k, _)| k == key).map(|(_, e)| *e));
            if self.scratch.is_empty() {
                continue;
            }
            self.out.clear();
            recognizer.frame(&self.scratch, &mut self.out);
            out.extend(self.out.iter().map(|g| (*key, *g)));
        }
    }

    /// Fire due long presses. Call from the UI loop every frame: a finger held
    /// still sends no events.
    pub fn tick(&mut self, now_us: u64, out: &mut Vec<(K, Gesture)>) {
        for (key, recognizer) in &mut self.owners {
            self.out.clear();
            recognizer.tick(now_us, &mut self.out);
            out.extend(self.out.iter().map(|g| (*key, *g)));
        }
    }
}
