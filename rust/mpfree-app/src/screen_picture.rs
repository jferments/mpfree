//! The screen picture: all four PAD BANK buttons held together, as on the
//! MPC, save a picture of the screen (the button combos Joshi approved on
//! 19 Nov 2026).
//!
//! The screen thread paints the page again into a picture
//! (`gui::Player::screen_picture`, a packed PNG from
//! `mpfree_ui::png::encode_rgb_small`) and hands it to the sound loop, which
//! gives it to the store like every other file the app keeps
//! (storage-spec.md: `files/screens/<name>`, under `--data` only). The name
//! is the time it was taken, by the MPC's clock, in UTC, so no folder is
//! read on the sound loop to find a free one; two pictures in the same
//! second get "-2", "-3" and so on. Without a store that saves, nothing is
//! written and the hint line says so.

use mpfree_store::Store;

/// The store's folder for screen pictures: `files/screens/`.
pub const OWNER: &str = "screens";

/// The pictures this run has named, to keep two in one second apart.
#[derive(Default)]
pub struct Names {
    last: String,
    same: u32,
}

impl Names {
    /// The name for a picture taken `secs` seconds after 1970 (UTC):
    /// "screen-20261119-083012.png", then "-2" and on within the second.
    pub fn next(&mut self, secs: u64) -> String {
        self.next_named("screen", "png", secs)
    }

    /// The same for another kind of file: "report-20261123-114000.txt"
    /// (System status's reports).
    pub fn next_named(&mut self, what: &str, ext: &str, secs: u64) -> String {
        let stamp = stamp(secs);
        if stamp == self.last {
            self.same += 1;
            format!("{what}-{stamp}-{}.{ext}", self.same)
        } else {
            self.last = stamp.clone();
            self.same = 1;
            format!("{what}-{stamp}.{ext}")
        }
    }
}

/// `secs` after 1970 as "YYYYMMDD-HHMMSS", UTC (Howard Hinnant's days to
/// civil date).
pub fn stamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

/// The picture the screen thread made (or why it couldn't), handed to the
/// store; what to say on the hint line.
pub fn save(store: &Store, names: &mut Names, png: Result<Vec<u8>, String>) -> String {
    let png = match png {
        Ok(p) => p,
        Err(e) => return format!("Screen picture: not saved ({e})"),
    };
    if !store.saves() {
        return "Screen picture: no data folder to keep it in this run".into();
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = names.next(secs);
    match store.write(OWNER, &name, png) {
        Ok(()) => format!("Screen picture saved: files/{OWNER}/{name}"),
        Err(e) => format!("Screen picture: not saved ({e})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_named_by_the_time_it_was_taken() {
        assert_eq!(stamp(0), "19700101-000000");
        // 2026-11-19 08:30:12 UTC.
        assert_eq!(stamp(1_795_077_012), "20261119-083012");
        // A leap day.
        assert_eq!(stamp(1_709_164_800), "20240229-000000");
        let mut n = Names::default();
        assert_eq!(n.next(1_795_077_012), "screen-20261119-083012.png");
        assert_eq!(n.next(1_795_077_012), "screen-20261119-083012-2.png");
        assert_eq!(n.next(1_795_077_012), "screen-20261119-083012-3.png");
        assert_eq!(n.next(1_795_077_013), "screen-20261119-083013.png");
    }

    #[test]
    fn without_a_data_folder_or_a_picture_nothing_is_saved() {
        let mut n = Names::default();
        assert_eq!(
            save(&Store::none(), &mut n, Ok(vec![1, 2, 3])),
            "Screen picture: no data folder to keep it in this run"
        );
        assert_eq!(
            save(&Store::none(), &mut n, Err("too big".into())),
            "Screen picture: not saved (too big)"
        );
    }
}
