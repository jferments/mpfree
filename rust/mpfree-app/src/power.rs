//! The power button (Jay, 15 Nov: "I want the power button to work like it
//! does on the MPC").
//!
//! A short press opens the power card over any page: Shut down, Restart or
//! Cancel. Over unsaved work it asks first whether to save the project
//! (the same save as SHIFT + BROWSE), and waits for the save to land
//! before anything goes off. Holding the button down is still the hard
//! power-off: this module never sees it as a press, because a press counts
//! only when the button comes back up within [`SHORT_MAX_US`].
//!
//! The pieces:
//!
//! - [`PressTimer`]: the button's down and up, as the kernel timed them,
//!   into short presses. Pure, so it is tested here.
//! - [`spawn_reader`]: a thread that finds the `gpio-keys` input device
//!   that has `KEY_POWER` (event1 on the MPC One, found by name and by the
//!   key, not by number), reads it without grabbing it, and sends a
//!   [`crate::gui::ToUi::PowerPress`] for each short press. Akai's own
//!   power-button service, where it runs, keeps reading the same device.
//! - [`PowerCard`]: the card itself, a widget the screen shows as a page
//!   over the one it was opened from, as the number pad is.
//! - [`request`]: the app runs without root on the stick, so it never
//!   shuts down by itself. It writes one word, `poweroff` or `restart`, to
//!   the file named by `mpfree app --power-request FILE`; on the stick a
//!   root systemd path unit sees the file and asks systemd for a clean
//!   power-off or restart (`tools/mpc-one/offline-kexec-stick.py`). Without
//!   that option the card says so and points at the hard power-off.
//!
//! What "Restart" starts: the MPC restarts from the beginning, as at
//! power-on, so with the MPFree stick in it starts MPFree again; without
//! the stick it starts Akai's own system
//! (`docs/dev/hardware/power-button.md`).

use mpfree_ui::{Align, EventKind, FontId, PaintCx, Rect, Target, Touch, Widget};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::thread;
use std::time::Duration;

/// The widget's name, and the page that shows it.
pub const CARD: &str = "power";
pub const PAGE: &str = "Power";
pub const LAYOUT: &str = "col(header:60, power:*, hint:26)";
pub const HINT: &str =
    "Tap a button   ·   wheel or - / +: pick   ·   push the wheel: choose   ·   power button: Cancel";
/// The hint while the card saves or the MPC goes.
pub const WAIT_HINT: &str =
    "Wait: the MPC goes on by itself   ·   hold the power button to switch off at once";

/// The longest press that counts as short. Held longer, it is left to the
/// hard power-off, which needs the button held for several seconds.
pub const SHORT_MAX_US: u64 = 1_500_000;

/// `KEY_POWER` (input-event-codes.h), the only key the MPC One's
/// `gpio-keys` device has.
pub const KEY_POWER: u16 = 116;
const EV_KEY: u16 = 0x01;

/// What the musician asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ShutDown,
    Restart,
}

impl Action {
    /// The word the root side reads ([`request`]).
    pub fn word(self) -> &'static str {
        match self {
            Action::ShutDown => "poweroff",
            Action::Restart => "restart",
        }
    }

    /// "shut down", "restart", for the card's buttons.
    fn verb(self) -> &'static str {
        match self {
            Action::ShutDown => "shut down",
            Action::Restart => "restart",
        }
    }
}

// ---------------------------------------------------------------- the button

/// The power button's down and up, into short presses.
#[derive(Debug, Default)]
pub struct PressTimer {
    down_at: Option<u64>,
}

impl PressTimer {
    /// One `EV_KEY` event of `KEY_POWER`: `value` 1 down, 0 up, 2 the
    /// kernel's auto-repeat (ignored); `at_us` the kernel's timestamp. True
    /// when it ends a short press.
    pub fn feed(&mut self, value: i32, at_us: u64) -> bool {
        match value {
            1 => {
                self.down_at = Some(at_us);
                false
            }
            0 => match self.down_at.take() {
                Some(t) => at_us.saturating_sub(t) <= SHORT_MAX_US,
                // An up with no down: the app started with the button held.
                None => false,
            },
            _ => false,
        }
    }
}

/// The power button's input device: a `gpio-keys` device whose key
/// bitmap has `KEY_POWER`, under `sys` (`/sys/class/input`).
pub fn find_power_key(sys: &Path) -> Option<PathBuf> {
    let mut found: Vec<String> = fs::read_dir(sys)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("event"))
        .filter(|n| {
            let dev = sys.join(n).join("device");
            let name = fs::read_to_string(dev.join("name")).unwrap_or_default();
            let keys = fs::read_to_string(dev.join("capabilities/key")).unwrap_or_default();
            name.trim() == "gpio-keys" && has_key(&keys, KEY_POWER)
        })
        .collect();
    found.sort();
    found.first().map(|n| PathBuf::from("/dev/input").join(n))
}

/// Whether sysfs's key bitmap (`capabilities/key`: hex words, the highest
/// first, each as wide as the kernel's `long`) has key `code`. The MPC's
/// is `100000 0 0 0` on 32-bit ARM: bit 116 (20 of the fourth word).
fn has_key(bitmap: &str, code: u16) -> bool {
    let words: Vec<&str> = bitmap.split_whitespace().collect();
    if words.is_empty() {
        return false;
    }
    // Each word is as wide as the kernel's `long`, which is this build's
    // (the app runs on the kernel it was built for: 32-bit ARM on the MPC).
    let bits = usize::BITS as usize;
    let index = code as usize / bits;
    let Some(word) = words.len().checked_sub(1 + index).map(|i| words[i]) else {
        return false;
    };
    u64::from_str_radix(word, 16).is_ok_and(|w| w >> (code as usize % bits) & 1 == 1)
}

/// How often the reader looks for the device again while there is none.
const FIND_AGAIN: Duration = Duration::from_secs(2);

/// Reads the power button on a thread of its own and sends `press()` to
/// the screen for each short press, until the screen goes. Read-only and
/// never grabbed: anything else reading the device still sees every press.
pub fn spawn_reader<M: Send + 'static>(
    to_ui: SyncSender<M>,
    press: fn() -> M,
    note: fn(String) -> M,
) -> io::Result<()> {
    thread::Builder::new()
        .name("mpfree-power".into())
        .spawn(move || {
            let sys = Path::new("/sys/class/input");
            let mut said_none = false;
            loop {
                let Some(path) = find_power_key(sys) else {
                    if !std::mem::replace(&mut said_none, true) {
                        let _ = to_ui.try_send(note(
                            "power button: not found yet (no gpio-keys device with KEY_POWER)"
                                .into(),
                        ));
                    }
                    thread::sleep(FIND_AGAIN);
                    continue;
                };
                let mut file = match fs::File::open(&path) {
                    Ok(f) => f,
                    Err(e) => {
                        let _ = to_ui.try_send(note(format!(
                            "power button: {} not opened ({e})",
                            path.display()
                        )));
                        thread::sleep(FIND_AGAIN * 15);
                        continue;
                    }
                };
                let _ = to_ui.try_send(note(format!(
                    "power button: {}; read-only, not grabbed; a short press opens the power card",
                    path.display()
                )));
                let mut timer = PressTimer::default();
                let mut buf = vec![0u8; mpfree_touch_evdev::EVENT_SIZE * 16];
                loop {
                    let n = match file.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    for ev in buf[..n].chunks_exact(mpfree_touch_evdev::EVENT_SIZE) {
                        let e = mpfree_touch_evdev::parse_input_event(ev);
                        if e.kind == EV_KEY && e.code == KEY_POWER && timer.feed(e.value, e.time_us)
                        {
                            if let Err(TrySendError::Disconnected(_)) = to_ui.try_send(press()) {
                                return;
                            }
                        }
                    }
                }
                let _ = to_ui.try_send(note(format!(
                    "power button: {} stopped reading; looking again",
                    path.display()
                )));
                thread::sleep(FIND_AGAIN);
            }
        })
        .map(|_| ())
}

// ---------------------------------------------------------------- the request

/// Hands `action` to the root side: `word\n` written to a file next to
/// `path`, flushed to disk, then renamed to `path`, so the root side never
/// reads half a word. The filesystems are synced first, so a save that has
/// just landed is on the disk before anything stops.
pub fn request(path: &Path, action: Action) -> io::Result<()> {
    crate::sys::sync_disks();
    let tmp = path.with_extension("new");
    let mut f = fs::File::create(&tmp)?;
    f.write_all(format!("{}\n", action.word()).as_bytes())?;
    f.sync_all()?;
    drop(f);
    fs::rename(&tmp, path)
}

// ---------------------------------------------------------------- the card

/// What the card shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Shut down, Restart or Cancel; `project` the project's name and
    /// whether it has unsaved changes (None: no library, nothing is kept).
    Ask { project: Option<(String, bool)> },
    /// The project has unsaved changes: save it first?
    Unsaved { action: Action, name: String },
    /// The save is under way.
    Saving { action: Action, name: String },
    /// The save didn't land: why, and whether to go on anyway.
    SaveFailed {
        action: Action,
        name: String,
        why: String,
    },
    /// Handed to the system: the screen stays like this until it goes.
    Going(Action),
    /// The request couldn't be handed over (no `--power-request`, or the
    /// write failed): why.
    Unavailable(String),
}

/// A button on the card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Cancel,
    /// Go on with the action (saved, or nothing to save).
    Go(Action),
    /// Save the project, then the action.
    SaveThen(Action),
    /// Leave the changes unsaved and go on.
    DontSave(Action),
}

const CARD_W: i32 = 760;
const CARD_H: i32 = 330;
const BUTTON_H: i32 = 72;
const GAP: i32 = 14;

/// The power card.
#[derive(Debug, Default)]
pub struct PowerCard {
    pub step: Option<Step>,
    /// The button the wheel's outline is on.
    pub picked: usize,
}

impl PowerCard {
    /// Shows `step`, with the outline on its main button.
    pub fn show(&mut self, step: Step) {
        self.step = Some(step);
        self.picked = self.buttons().len().saturating_sub(1);
    }

    pub fn close(&mut self) {
        self.step = None;
    }

    /// The buttons of the step showing, left to right; the last is the
    /// main one.
    pub fn buttons(&self) -> Vec<(Choice, String)> {
        match &self.step {
            Some(Step::Ask { .. }) => vec![
                (Choice::Cancel, "Cancel".into()),
                (Choice::Go(Action::Restart), "Restart".into()),
                (Choice::Go(Action::ShutDown), "Shut down".into()),
            ],
            Some(Step::Unsaved { action, .. }) => vec![
                (Choice::Cancel, "Cancel".into()),
                (Choice::DontSave(*action), "Don\u{2019}t save".into()),
                (
                    Choice::SaveThen(*action),
                    format!("Save and {}", action.verb()),
                ),
            ],
            Some(Step::SaveFailed { action, .. }) => vec![
                (Choice::Cancel, "Cancel".into()),
                (
                    Choice::DontSave(*action),
                    format!("{} without saving", capital(action.verb())),
                ),
            ],
            Some(Step::Unavailable(_)) => vec![(Choice::Cancel, "OK".into())],
            Some(Step::Saving { .. } | Step::Going(_)) | None => Vec::new(),
        }
    }

    /// The wheel or - / + moved the outline.
    pub fn wheel(&mut self, d: i32) {
        let n = self.buttons().len() as i32;
        if n > 0 {
            self.picked = (self.picked as i32 + d).clamp(0, n - 1) as usize;
        }
    }

    /// The button the outline is on (the wheel's push).
    #[cfg(test)]
    pub fn chosen(&self) -> Option<Choice> {
        self.buttons().get(self.picked).map(|(c, _)| *c)
    }

    /// True while the card can be left (not saving, not going).
    pub fn cancellable(&self) -> bool {
        !matches!(self.step, Some(Step::Saving { .. } | Step::Going(_)))
    }

    fn card(r: Rect) -> Rect {
        let w = r.w.min(CARD_W);
        let h = r.h.min(CARD_H);
        Rect::new(r.x + (r.w - w) / 2, r.y + (r.h - h) / 2, w, h)
    }

    /// Each button's rectangle, along the card's foot.
    pub fn button_rects(&self, r: Rect) -> Vec<Rect> {
        let n = self.buttons().len() as i32;
        if n == 0 {
            return Vec::new();
        }
        let inner = PowerCard::card(r).inset(24);
        let w = (inner.w - GAP * (n - 1)) / n;
        let y = inner.bottom() - BUTTON_H;
        (0..n)
            .map(|i| Rect::new(inner.x + i * (w + GAP), y, w, BUTTON_H))
            .collect()
    }

    /// Title, lines and the tone of the first line.
    fn words(&self) -> (String, Vec<String>, Tone) {
        match &self.step {
            Some(Step::Ask { project }) => {
                let line = match project {
                    Some((name, true)) => (
                        format!("{} has changes that aren\u{2019}t saved.", capital(name)),
                        Tone::Warn,
                    ),
                    Some((name, false)) if name.is_empty() => {
                        ("Nothing new to save.".to_string(), Tone::Dim)
                    }
                    Some((name, false)) => (format!("{name} is saved."), Tone::Dim),
                    None => (
                        "Nothing is kept: this MPFree has no data folder.".into(),
                        Tone::Warn,
                    ),
                };
                (
                    "Shut down the MPC?".into(),
                    vec![
                        line.0,
                        "Restart starts MPFree again with the stick in, the MPC\u{2019}s own system without it."
                            .into(),
                    ],
                    line.1,
                )
            }
            Some(Step::Unsaved { action, name }) => (
                format!("Save {name} first?"),
                vec![
                    "It has changes that aren\u{2019}t saved.".into(),
                    format!(
                        "Don\u{2019}t save loses them when the MPC {}.",
                        match action {
                            Action::ShutDown => "shuts down",
                            Action::Restart => "restarts",
                        }
                    ),
                ],
                Tone::Dim,
            ),
            Some(Step::Saving { name, .. }) => (
                format!("Saving {name}\u{2026}"),
                vec!["The MPC goes on once the save is on the stick.".into()],
                Tone::Dim,
            ),
            Some(Step::SaveFailed { name, why, .. }) => (
                format!("{} wasn\u{2019}t saved", capital(name)),
                vec![why.clone()],
                Tone::Bad,
            ),
            Some(Step::Going(Action::ShutDown)) => (
                "Shutting down\u{2026}".into(),
                vec![
                    "The screen goes dark when the MPC is off.".into(),
                    "If it is still on in a minute, hold the power button.".into(),
                ],
                Tone::Dim,
            ),
            Some(Step::Going(Action::Restart)) => (
                "Restarting\u{2026}".into(),
                vec![
                    "With the stick in, MPFree starts again.".into(),
                    "Without it, the MPC starts its own system.".into(),
                ],
                Tone::Dim,
            ),
            Some(Step::Unavailable(why)) => (
                "Can\u{2019}t shut down from here".into(),
                vec![
                    why.clone(),
                    "Hold the power button for a few seconds to switch the MPC off.".into(),
                ],
                Tone::Warn,
            ),
            None => (String::new(), Vec::new(), Tone::Dim),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tone {
    Dim,
    Warn,
    Bad,
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

impl Widget for PowerCard {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        if self.step.is_none() {
            return;
        }
        let card = PowerCard::card(r);
        cx.panel(card);
        cx.canvas.stroke_round_rect(card, t.radius, 2, t.outline);
        let inner = card.inset(24);
        let (title, lines, tone) = self.words();
        let (head, mut body) = inner.take_top(44);
        cx.text(FontId::Bold, 30.0, head, Align::Left, t.text, &title);
        body = body.take_top(14).1;
        for (i, line) in lines.iter().enumerate() {
            let (row, rest) = body.take_top(34);
            body = rest;
            let ink = match (i, tone) {
                (0, Tone::Warn) => t.warn,
                (0, Tone::Bad) => t.bad,
                (0, Tone::Dim) => t.text,
                _ => t.text_dim,
            };
            cx.text(FontId::Sans, t.size_title, row, Align::Left, ink, line);
        }
        let buttons = self.buttons();
        let last = buttons.len().saturating_sub(1);
        for (i, ((choice, label), br)) in buttons.iter().zip(self.button_rects(r)).enumerate() {
            let main = i == last && *choice != Choice::Cancel;
            cx.canvas
                .fill_round_rect(br, 12, if main { t.accent } else { t.raised });
            if i == self.picked {
                cx.canvas.stroke_round_rect(br.inset(-4), 14, 3, t.text);
            }
            let ink = if main { t.on_accent } else { t.text };
            cx.text(FontId::Bold, t.size_title, br, Align::Center, ink, label);
        }
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let i = self.button_rects(r).iter().position(|b| b.contains(x, y))?;
        self.picked = i;
        Some(EventKind::Tab(i))
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        self.buttons()
            .into_iter()
            .zip(self.button_rects(r))
            .enumerate()
            .map(|(i, ((_, label), rect))| Target::tap("button", i, label, rect))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_is_short_only_when_the_button_comes_back_up_in_time() {
        let mut t = PressTimer::default();
        assert!(!t.feed(1, 1_000_000));
        assert!(t.feed(0, 1_200_000), "0.2 s: a short press");
        assert!(!t.feed(1, 5_000_000));
        assert!(!t.feed(2, 5_500_000), "auto-repeat is not a press");
        assert!(t.feed(0, 6_500_000), "1.5 s: still short");
        // Held for the hard power-off: never a press, whatever comes after.
        assert!(!t.feed(1, 10_000_000));
        assert!(!t.feed(0, 13_000_000));
        // An up with no down (the app started with the button held).
        assert!(!t.feed(0, 14_000_000));
    }

    #[test]
    fn the_key_bitmap_is_read_as_the_kernel_writes_it() {
        // The MPC One's gpio-keys on 32-bit ARM (m07 validation record).
        let mpc = "100000 0 0 0";
        assert_eq!(has_key(mpc, KEY_POWER), usize::BITS == 32);
        // The same key on a 64-bit kernel.
        assert_eq!(has_key("10000000000000 0", KEY_POWER), usize::BITS == 64);
        assert!(!has_key("0 0", KEY_POWER));
        assert!(!has_key("", KEY_POWER));
        assert!(!has_key("zz 0 0 0", KEY_POWER));
    }

    #[test]
    fn the_power_key_is_found_by_name_and_key_not_by_number() {
        let dir = std::env::temp_dir().join(format!("mpfree-power-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let key = if usize::BITS == 32 {
            "100000 0 0 0"
        } else {
            "10000000000000 0"
        };
        for (n, name, keys) in [
            ("event0", "ILI2117 Touchscreen", "400 0 0 0 0 0 0 0 0 0 0"),
            ("event3", "gpio-keys", key),
            ("event2", "gpio-keys", "0"),
        ] {
            let d = dir.join(n).join("device/capabilities");
            fs::create_dir_all(&d).unwrap();
            fs::write(dir.join(n).join("device/name"), format!("{name}\n")).unwrap();
            fs::write(d.join("key"), format!("{keys}\n")).unwrap();
        }
        assert_eq!(
            find_power_key(&dir),
            Some(PathBuf::from("/dev/input/event3"))
        );
        fs::remove_dir_all(dir.join("event3")).unwrap();
        assert_eq!(find_power_key(&dir), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_request_is_one_word_in_a_whole_file() {
        let dir = std::env::temp_dir().join(format!("mpfree-power-req-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("power-request");
        request(&path, Action::Restart).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "restart\n");
        request(&path, Action::ShutDown).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "poweroff\n");
        assert!(!path.with_extension("new").exists());
        assert!(request(&dir.join("gone/power-request"), Action::ShutDown).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    const R: Rect = Rect {
        x: 0,
        y: 60,
        w: 1280,
        h: 714,
    };

    #[test]
    fn each_step_has_its_buttons_and_the_outline_starts_on_the_main_one() {
        let mut c = PowerCard::default();
        c.show(Step::Ask {
            project: Some(("Song 3".into(), true)),
        });
        assert_eq!(c.chosen(), Some(Choice::Go(Action::ShutDown)));
        c.wheel(-1);
        assert_eq!(c.chosen(), Some(Choice::Go(Action::Restart)));
        c.wheel(-5);
        assert_eq!(c.chosen(), Some(Choice::Cancel));
        c.show(Step::Unsaved {
            action: Action::Restart,
            name: "Song 3".into(),
        });
        assert_eq!(c.chosen(), Some(Choice::SaveThen(Action::Restart)));
        let labels: Vec<String> = c.buttons().into_iter().map(|(_, l)| l).collect();
        assert_eq!(labels, ["Cancel", "Don\u{2019}t save", "Save and restart"]);
        c.show(Step::Saving {
            action: Action::Restart,
            name: "Song 3".into(),
        });
        assert!(c.buttons().is_empty() && !c.cancellable());
        c.show(Step::Unavailable("why".into()));
        assert_eq!(c.chosen(), Some(Choice::Cancel));
    }

    #[test]
    fn a_tap_on_a_button_picks_it() {
        let mut c = PowerCard::default();
        c.show(Step::Ask { project: None });
        let rects = c.button_rects(R);
        assert_eq!(rects.len(), 3);
        for b in &rects {
            assert!(b.w >= 64 && b.h >= 64);
        }
        let mid = |b: Rect| (b.x + b.w / 2, b.y + b.h / 2);
        let (x, y) = mid(rects[1]);
        assert_eq!(c.touch(1, Touch::Down, x, y, R), Some(EventKind::Tab(1)));
        assert_eq!(c.chosen(), Some(Choice::Go(Action::Restart)));
        assert_eq!(c.touch(1, Touch::Down, 5, 70, R), None);
        assert_eq!(c.targets(R).len(), 3);
    }
}
