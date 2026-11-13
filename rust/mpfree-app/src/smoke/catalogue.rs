//! The controls catalogue (`docs/dev/guides/controls.md`) as the tests read
//! it, and the controls known to do nothing yet.

/// Controls that do nothing yet, by `widget/id`, each with who builds it.
/// The sweep fails on any other dead control, and on one of these once it
/// works (take it off this list then).
pub const KNOWN_DEAD: &[(&str, &str)] = &[];

pub fn known_dead(key: &str) -> bool {
    KNOWN_DEAD.iter().any(|(k, _)| *k == key)
}

/// Where the catalogue lives, from this crate.
#[cfg(test)]
pub const PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/dev/guides/controls.md"
);

/// The `widget/id` keys the catalogue has an entry for: every bold,
/// backquoted key in a list item that starts with one
/// (`- **`a/b`**, **`a/c`**: ... **`a/d`**: ...`).
#[cfg(test)]
pub fn keys(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines().filter(|l| l.starts_with("- **`")) {
        for part in line.split("**`").skip(1) {
            if let Some((key, _)) = part.split_once("`**") {
                if key.contains('/') {
                    out.push(key.to_string());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalogue_reads_every_key_of_an_entry() {
        let text = "intro `not/this`\n\
                    - **`a/b`**: one. Also `x/y` in the text.\n\
                    - **`c/d`**, **`c/e`**: two. **`c/f`**: three.\n  \
                    - **`nested/no`**: no.\n";
        assert_eq!(keys(text), vec!["a/b", "c/d", "c/e", "c/f"]);
    }
}
