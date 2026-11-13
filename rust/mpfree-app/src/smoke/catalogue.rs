//! The controls catalogue (`docs/dev/guides/controls.md`) as the tests read
//! it, and the controls known to do nothing yet.

/// Controls that do nothing yet, by `widget/id`, each with who builds it.
/// The sweep fails on any other dead control, and on one of these once it
/// works (take it off this list then).
pub const KNOWN_DEAD: &[(&str, &str)] = &[];

pub fn known_dead(key: &str) -> bool {
    KNOWN_DEAD.iter().any(|(k, _)| *k == key)
}
