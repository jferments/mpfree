//! Slot ids: they follow the effect, not the position.

use mpfree_fx::{Chain, Crush, Delay, Drive, Insert};
use mpfree_node::MAX_BLOCK;

fn ins(n: Box<dyn mpfree_node::Node>) -> Insert {
    Insert::new(n, 48_000, MAX_BLOCK)
}

#[test]
fn ids_follow_the_effect() {
    let mut c = Chain::with_capacity(8);
    let a = c.push(Box::new(Drive::new()));
    let b = c.push(Box::new(Delay::new()));
    let (ua, ub) = (c.uid(a).unwrap(), c.uid(b).unwrap());
    assert_ne!(ua, ub);
    assert!(ua >= 1 && ub >= 1);
    // Moving and bypassing keep them.
    assert!(c.move_slot(0, 1));
    assert_eq!(c.slot_of(ua), Some(1));
    assert_eq!(c.slot_of(ub), Some(0));
    c.set_on(1, false);
    assert_eq!(c.uid(1), Some(ua));
    // A new insert takes a free id.
    c.insert(0, ins(Box::new(Crush::new()))).ok().unwrap();
    let uc = c.uid(0).unwrap();
    assert!(uc != ua && uc != ub);
    // Taken out and put back (undo): the same id.
    let out = c.remove(c.slot_of(ua).unwrap()).unwrap();
    assert_eq!(out.uid(), Some(ua));
    assert_eq!(c.slot_of(ua), None);
    c.insert(0, out).ok().unwrap();
    assert_eq!(c.slot_of(ua), Some(0));
}

#[test]
fn loading_asks_for_ids_and_a_taken_one_is_refused() {
    let mut c = Chain::with_capacity(8);
    c.insert(0, ins(Box::new(Drive::new())).with_uid(7))
        .ok()
        .unwrap();
    assert_eq!(c.uid(0), Some(7));
    let refused = c.insert(1, ins(Box::new(Delay::new())).with_uid(7));
    assert_eq!(refused.err().and_then(|i| i.uid()), Some(7));
    // The next free id is past the ones loaded.
    c.insert(1, ins(Box::new(Delay::new()))).ok().unwrap();
    assert_eq!(c.uid(1), Some(8));
    // Replacing gives the new effect its own id, and hands back the old.
    let old = c.replace(0, ins(Box::new(Crush::new()))).ok().unwrap();
    assert_eq!(old.uid(), Some(7));
    let new = c.uid(0).unwrap();
    assert!(new != 7 && new != 8);
    // ...unless the load asks for the id it replaces.
    c.replace(0, ins(Box::new(Drive::new())).with_uid(new))
        .ok()
        .unwrap();
    assert_eq!(c.uid(0), Some(new));
    assert!(
        c.replace(0, ins(Box::new(Drive::new())).with_uid(8))
            .is_err(),
        "8 belongs to slot 1"
    );
    // Another chain can take an insert with its id cleared.
    let mut d = Chain::with_capacity(2);
    d.push(Box::new(Drive::new()));
    let moved = c.remove(1).unwrap();
    let back = d.insert(1, moved.without_uid());
    assert!(back.is_ok());
}
