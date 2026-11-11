//! Names in the library, and the order lists are shown in.

use std::cmp::Ordering;

/// The longest name, in bytes.
pub const MAX_NAME: usize = 100;

/// A file or folder name the library accepts: 1 to [`MAX_NAME`] bytes of
/// printable text, which may have spaces and capitals, but no `/`, `\`,
/// control character or one of `<>:"|?*` (so it also fits a FAT stick),
/// does not start with `.` or a space, does not end with `.` or a space,
/// and does not end in `.part` (the library's own unfinished files).
pub fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_NAME
        && !s.starts_with('.')
        && !s.starts_with(' ')
        && !s.ends_with('.')
        && !s.ends_with(' ')
        && !s.ends_with(".part")
        && !s.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*')
        })
}

/// A name made valid: bad characters become `-`, the ends are trimmed of
/// spaces and dots, and it is cut to [`MAX_NAME`] bytes. `fallback` if
/// nothing is left.
pub fn clean_name(s: &str, fallback: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*') {
                '-'
            } else {
                c
            }
        })
        .collect();
    while out.len() > MAX_NAME {
        out.pop();
    }
    let mut t = out.trim_matches(|c| c == ' ' || c == '.').to_string();
    if t.ends_with(".part") {
        t.truncate(t.len() - 5);
        t = t.trim_matches(|c| c == ' ' || c == '.').to_string();
    }
    if valid_name(&t) {
        t
    } else {
        fallback.to_string()
    }
}

/// The name with ` 2`, ` 3` ... before its extension until `taken` says no.
pub fn unique_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_string();
    }
    let (stem, ext) = split_ext(name);
    for n in 2..10_000 {
        let mut stem = stem.to_string();
        let tail = format!(" {n}{ext}");
        while stem.len() + tail.len() > MAX_NAME {
            stem.pop();
        }
        let candidate = format!("{}{tail}", stem.trim_end());
        if !taken(&candidate) {
            return candidate;
        }
    }
    name.to_string()
}

/// `("kick", ".wav")` for `"kick.wav"`; no extension for a name without a
/// dot after its first character.
pub fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 && name.len() - i <= 6 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// Natural order, ignoring case: `Kick 2` before `Kick 10`.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    loose_cmp(a, b).then_with(|| a.cmp(b))
}

/// Natural order ignoring case and leading zeros.
fn loose_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let na = take_number(&mut a);
                let nb = take_number(&mut b);
                let o = na
                    .trim_start_matches('0')
                    .len()
                    .cmp(&nb.trim_start_matches('0').len())
                    .then_with(|| na.trim_start_matches('0').cmp(nb.trim_start_matches('0')));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                let o = x.to_lowercase().cmp(y.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                a.next();
                b.next();
            }
        }
    }
}

fn take_number(it: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut s = String::new();
    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
        s.push(c);
        it.next();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_people_type_are_allowed() {
        for ok in [
            "Kick 01.wav",
            "Night drive",
            "a",
            "Böse Bässe",
            "Loop (take 3)",
            "x.y.z",
        ] {
            assert!(valid_name(ok), "{ok}");
        }
    }

    #[test]
    fn names_that_could_leave_the_folder_or_hide_are_not() {
        for bad in [
            "",
            ".",
            "..",
            ".trash",
            "a/b",
            "..\\x",
            "a\\b",
            "x\n",
            " lead",
            "trail ",
            "end.",
            "half.part",
            "a:b",
            "q?",
            &"a".repeat(101),
        ] {
            assert!(!valid_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn clean_names_are_valid() {
        assert_eq!(clean_name("  ../a/b.  ", "x"), "-a-b");
        assert_eq!(clean_name("...", "Untitled"), "Untitled");
        assert_eq!(clean_name("take.part", "x"), "take");
        assert!(valid_name(&clean_name(&"é".repeat(80), "x")));
    }

    #[test]
    fn unique_names_count_up_before_the_extension() {
        let taken = ["Kick.wav", "Kick 2.wav"];
        assert_eq!(
            unique_name("Kick.wav", |n| taken.contains(&n)),
            "Kick 3.wav"
        );
        assert_eq!(
            unique_name("Snare.wav", |n| taken.contains(&n)),
            "Snare.wav"
        );
        assert_eq!(unique_name("Folder", |n| n == "Folder"), "Folder 2");
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["kick 10", "Kick 2", "hat", "kick 1", "Hat 03", "hat 3"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["hat", "Hat 03", "hat 3", "kick 1", "Kick 2", "kick 10"]);
    }
}
