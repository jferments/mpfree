//! The parts of each period outside the engines, timed so that a slow
//! period says where its time went.
//!
//! The CPU by engine page's `control` row is the period less the engines:
//! one number for everything else. In the 12 Nov live test the period's
//! worst reached 176 % and 295 % while no engine passed 51 %, and nothing
//! said which part took the time (it turned out to be the looper's
//! waveform, then drawn in one go outside the looper's probe). [`Parts`]
//! splits that rest into the loop's own parts:
//!
//! - **screen actions**: what the screen sent (`FromUi`), each timed;
//! - **MIDI in and buttons**: the controller's ports, MIDI In, USB keys and
//!   what each pad, button and knob does, each event timed;
//! - **app and effects**: the app's effects (pages, kits, the looper's and
//!   the transport's commands, saves), each timed, and the kit change;
//! - **pads and lights**: after the time engine, the hits, the clips'
//!   locks, the Play and Sample pages' marks, the pads' colours and lights,
//!   the session ping; and the chain editor's effects landing;
//! - **screen updates**: after the mixer, the meters, the Looper tab, the
//!   browser's preview and the copy to the sound card's format;
//! - **after the write**: from the sound card's write to the next period
//!   (the log's one-second and 30 s lines, the page's numbers).
//!
//! A period whose parts take [`SLOW`] (half a period) or more is a slow
//! period: counted, shown on the Dev tab (CPU by engine) and said in a
//! `control: slow period` line, at most one a second, naming its slowest
//! part and its slowest single action, effect or event (by its debug text,
//! cut to [`NAME`] bytes, made without allocating). Every 30 s the
//! `status: control parts` line gives each part's worst since the last one.
//!
//! Timing costs two clock reads per part and one per action; nothing here
//! allocates except the lines themselves.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mpfree_ui::Tone;

use crate::engines::EngineShared;

/// The parts, in the loop's order.
pub const PARTS: [&str; 6] = [
    "screen actions",
    "MIDI in and buttons",
    "app and effects",
    "pads and lights",
    "screen updates",
    "after the write",
];
pub const SCREEN_ACTIONS: usize = 0;
pub const MIDI_IN: usize = 1;
pub const APP: usize = 2;
pub const PADS: usize = 3;
pub const SCREEN: usize = 4;
pub const AFTER: usize = 5;
const N: usize = PARTS.len();

/// A period whose parts outside the engines take this long or longer is
/// slow: half a period (128 frames at 44.1 kHz), as much as every engine
/// together may take on a busy song.
pub const SLOW: Duration = Duration::from_nanos(PERIOD_NS / 2);
const PERIOD_NS: u64 = crate::PERIOD as u64 * 1_000_000_000 / 44_100;
/// A single action, effect or event this long or longer is named.
pub const NAMED: Duration = Duration::from_micros(100);
/// The most bytes of a name kept.
pub const NAME: usize = 48;

/// Debug text cut to [`NAME`] bytes, kept without allocating.
#[derive(Clone, Copy)]
pub struct Name {
    bytes: [u8; NAME],
    len: usize,
}

impl Name {
    /// `x`'s debug text, cut at [`NAME`] bytes (formatting stops there).
    pub fn of(x: &dyn fmt::Debug) -> Name {
        let mut n = Name {
            bytes: [0; NAME],
            len: 0,
        };
        let _ = fmt::write(&mut Cut(&mut n), format_args!("{x:?}"));
        n
    }

    pub fn as_str(&self) -> &str {
        // Cut only at a character's boundary (Cut::write_str).
        std::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Writes into a [`Name`] until it is full, then stops the formatting.
struct Cut<'a>(&'a mut Name);

impl fmt::Write for Cut<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let room = NAME - self.0.len;
        let mut take = s.len().min(room);
        while !s.is_char_boundary(take) {
            take -= 1;
        }
        self.0.bytes[self.0.len..self.0.len + take].copy_from_slice(&s.as_bytes()[..take]);
        self.0.len += take;
        if take < s.len() {
            Err(fmt::Error)
        } else {
            Ok(())
        }
    }
}

/// The slowest named thing of a period (or of a status window).
#[derive(Clone, Copy, Default)]
struct Item {
    took: Duration,
    part: usize,
    name: Option<Name>,
}

/// What the Dev tab shows (CPU by engine, under the budgets): written by
/// the audio loop, read by the screen thread. In [`EngineShared`].
#[derive(Debug, Default)]
pub struct PartsShared {
    /// Slow periods since the start.
    pub slow: AtomicU64,
    /// The slowest period's parts outside the engines, in ns.
    pub worst_ns: AtomicU64,
    /// Which part took the most of it ([`PARTS`]).
    pub worst_part: AtomicU64,
    /// The controller's output (control.rs), set about once a second:
    /// messages the port refused, messages dropped with the queue full,
    /// and times the lights' thread wrote nothing for [`crate::control::STALL`]
    /// with messages waiting.
    pub lights_failed: AtomicU64,
    pub lights_dropped: AtomicU64,
    pub lights_stalls: AtomicU64,
    /// Report lines dropped with the queue full.
    pub report_dropped: AtomicU64,
}

impl PartsShared {
    /// The control side's counts, from the audio loop (about once a second).
    pub fn set_counts(&self, lights_failed: u64, lights_dropped: u64, stalls: u64, report: u64) {
        self.lights_failed.store(lights_failed, Ordering::Relaxed);
        self.lights_dropped.store(lights_dropped, Ordering::Relaxed);
        self.lights_stalls.store(stalls, Ordering::Relaxed);
        self.report_dropped.store(report, Ordering::Relaxed);
    }
}

/// The audio loop's timer for its parts outside the engines. Made before
/// the loop; nothing in it allocates except the lines it returns.
pub struct Parts {
    mark: Instant,
    now: [Duration; N],
    item: Item,
    /// Each part's worst since the last status line, and the slowest named
    /// thing in that time.
    worst: [Duration; N],
    worst_item: Item,
    slow: u64,
    /// Slow periods not said in a line yet (at most a line a second).
    unsaid: u64,
    said_at: Option<Instant>,
    shared: Arc<EngineShared>,
}

impl Parts {
    /// Publishing into `shared.parts` for the Dev tab.
    pub fn new(shared: Arc<EngineShared>) -> Parts {
        Parts {
            mark: Instant::now(),
            now: [Duration::ZERO; N],
            item: Item::default(),
            worst: [Duration::ZERO; N],
            worst_item: Item::default(),
            slow: 0,
            unsaid: 0,
            said_at: None,
            shared,
        }
    }

    /// Starts timing from now: what ran since the last [`Parts::lap`] (an
    /// engine) is not counted.
    #[inline]
    pub fn begin(&mut self) {
        self.mark = Instant::now();
    }

    /// Adds the time since [`Parts::begin`] or the last lap to part `p`.
    #[inline]
    pub fn lap(&mut self, p: usize) {
        let now = Instant::now();
        self.now[p] += now.saturating_duration_since(self.mark);
        self.mark = now;
    }

    /// One action, effect or event of part `p` that started at `started`,
    /// named by `name` if it is the period's slowest and at least
    /// [`NAMED`]. For things the handler consumes: take the name first.
    #[inline]
    pub fn item_named(&mut self, p: usize, started: Instant, name: &Name) {
        let took = started.elapsed();
        if took >= NAMED && took > self.item.took {
            self.item = Item {
                took,
                part: p,
                name: Some(*name),
            };
        }
    }

    /// As [`Parts::item_named`], making the name only when it is needed.
    #[inline]
    pub fn item(&mut self, p: usize, started: Instant, what: &dyn fmt::Debug) {
        let took = started.elapsed();
        if took >= NAMED && took > self.item.took {
            self.item = Item {
                took,
                part: p,
                name: Some(Name::of(what)),
            };
        }
    }

    /// The period's parts so far (for the tests).
    #[cfg(test)]
    fn total(&self) -> Duration {
        self.now.iter().sum()
    }

    /// Ends the period, `t` seconds after the start: keeps each part's
    /// worst and, for a slow period, counts it and returns its line (at
    /// most one a second; the others are counted into the next).
    pub fn end(&mut self, now: Instant, t: f64) -> Option<String> {
        let total: Duration = self.now.iter().sum();
        for (w, n) in self.worst.iter_mut().zip(&self.now) {
            *w = (*w).max(*n);
        }
        if self.item.took > self.worst_item.took {
            self.worst_item = self.item;
        }
        let (most, most_took) = self.now.iter().enumerate().fold(
            (0, Duration::ZERO),
            |m, (p, &d)| if d > m.1 { (p, d) } else { m },
        );
        let mut line = None;
        if total >= SLOW {
            self.slow += 1;
            let sh = &self.shared.parts;
            sh.slow.store(self.slow, Ordering::Relaxed);
            let ns = total.as_nanos().min(u128::from(u64::MAX)) as u64;
            if ns > sh.worst_ns.load(Ordering::Relaxed) {
                sh.worst_ns.store(ns, Ordering::Relaxed);
                sh.worst_part.store(most as u64, Ordering::Relaxed);
            }
            if self
                .said_at
                .is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(1))
            {
                line = Some(self.slow_line(t, total, most, most_took));
                self.said_at = Some(now);
                self.unsaid = 0;
            } else {
                self.unsaid += 1;
            }
        }
        self.now = [Duration::ZERO; N];
        self.item = Item::default();
        line
    }

    fn slow_line(&self, t: f64, total: Duration, most: usize, most_took: Duration) -> String {
        let mut s = format!(
            "control: slow period at +{t:.3} s: {} outside the engines, most in {} ({})",
            ms(total),
            PARTS[most],
            ms(most_took)
        );
        if let Some(name) = &self.item.name {
            s.push_str(&format!(
                "; slowest single: {} in {} ({})",
                name.as_str(),
                PARTS[self.item.part],
                ms(self.item.took)
            ));
        }
        let rest: Vec<String> = (0..N)
            .filter(|&p| p != most && self.now[p] >= Duration::from_micros(50))
            .map(|p| format!("{} {}", PARTS[p], ms(self.now[p])))
            .collect();
        if !rest.is_empty() {
            s.push_str(&format!("; also {}", rest.join(", ")));
        }
        if self.unsaid > 0 {
            s.push_str(&format!(
                " ({} more slow periods since the last line)",
                self.unsaid
            ));
        }
        s
    }

    /// The 30 s status line's text: each part's worst since the last call
    /// and the slowest named thing, then the slow periods so far. Restarts
    /// the worst.
    pub fn status(&mut self) -> String {
        let parts: Vec<String> = (0..N)
            .map(|p| format!("{} {}", PARTS[p], ms(self.worst[p])))
            .collect();
        let mut s = format!("control parts (worst in 30 s): {}", parts.join(", "));
        if let Some(name) = &self.worst_item.name {
            s.push_str(&format!(
                "; slowest single: {} in {} ({})",
                name.as_str(),
                PARTS[self.worst_item.part],
                ms(self.worst_item.took)
            ));
        }
        s.push_str(&format!(
            "; {} slow periods (half a period or more outside the engines) since the start",
            self.slow
        ));
        self.worst = [Duration::ZERO; N];
        self.worst_item = Item::default();
        s
    }
}

fn ms(d: Duration) -> String {
    format!("{:.2} ms", d.as_secs_f64() * 1e3)
}

/// The Dev tab's rows (CPU by engine, under the budgets): the slow periods
/// and the controller's output, red once anything went wrong.
pub fn rows(shared: &PartsShared) -> Vec<(String, String, Tone)> {
    let get = |a: &AtomicU64| a.load(Ordering::Relaxed);
    let slow = get(&shared.slow);
    let value = if slow == 0 {
        "none".to_string()
    } else {
        let worst = get(&shared.worst_ns) as f64 / 1e6;
        let part = get(&shared.worst_part) as usize;
        format!(
            "{slow}, worst {worst:.2} ms, most in {}",
            PARTS.get(part).copied().unwrap_or("?")
        )
    };
    let bad = |n: u64| if n == 0 { Tone::Normal } else { Tone::Bad };
    let (failed, dropped, stalls, report) = (
        get(&shared.lights_failed),
        get(&shared.lights_dropped),
        get(&shared.lights_stalls),
        get(&shared.report_dropped),
    );
    vec![
        (
            "Slow periods outside the engines".to_string(),
            value,
            bad(slow),
        ),
        (
            "Controller output: refused · dropped · stalls".to_string(),
            format!("{failed} · {dropped} · {stalls}"),
            bad(failed + dropped + stalls),
        ),
        (
            "Report lines dropped".to_string(),
            report.to_string(),
            bad(report),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    #[allow(dead_code)] // read by Debug
    enum Action {
        Looper(u32),
        Big(Vec<u32>),
    }

    #[test]
    fn a_name_is_cut_without_allocating_more_than_it_keeps() {
        let n = Name::of(&Action::Looper(7));
        assert_eq!(n.as_str(), "Looper(7)");
        let n = Name::of(&Action::Big((0..10_000).collect()));
        assert_eq!(n.as_str().len(), NAME);
        assert!(n.as_str().starts_with("Big([0, 1, 2"));
        // Never cut inside a character.
        let n = Name::of(&"é".repeat(40));
        assert!(n.as_str().len() <= NAME);
        assert!(n.as_str().chars().all(|c| c == 'é' || c == '"'));
    }

    #[test]
    fn a_fast_period_says_nothing_and_a_slow_one_names_its_part_and_action() {
        let shared = Arc::new(EngineShared::default());
        let mut p = Parts::new(Arc::clone(&shared));
        let row = |s: &EngineShared| rows(&s.parts).remove(0);
        // Nothing counted (no lap): not slow, however slow the machine.
        p.begin();
        assert!(p.end(Instant::now(), 0.0).is_none());
        assert_eq!(shared.parts.slow.load(Ordering::Relaxed), 0);
        assert_eq!(row(&shared).1, "none");
        assert_eq!(row(&shared).2, Tone::Normal);
        // A slow effect in the app's part.
        p.begin();
        let started = Instant::now();
        let name = Name::of(&Action::Looper(1));
        std::thread::sleep(SLOW + Duration::from_millis(1));
        p.item_named(APP, started, &name);
        p.lap(APP);
        let line = p.end(Instant::now(), 12.5).expect("a slow period");
        assert!(
            line.starts_with("control: slow period at +12.500 s: "),
            "{line}"
        );
        assert!(line.contains("most in app and effects"), "{line}");
        assert!(
            line.contains("slowest single: Looper(1) in app and effects"),
            "{line}"
        );
        assert_eq!(shared.parts.slow.load(Ordering::Relaxed), 1);
        let (label, value, tone) = row(&shared);
        assert_eq!(label, "Slow periods outside the engines");
        assert!(value.starts_with("1, worst "), "{value}");
        assert!(value.ends_with("most in app and effects"), "{value}");
        assert_eq!(tone, Tone::Bad);
        assert_eq!(p.total(), Duration::ZERO, "the next period starts empty");
        // A second slow period within the second is counted, not said, and
        // the next line says so.
        p.begin();
        std::thread::sleep(SLOW);
        p.lap(AFTER);
        let at = Instant::now();
        assert!(p.end(at, 12.6).is_none());
        p.begin();
        std::thread::sleep(SLOW);
        p.lap(SCREEN);
        let line = p.end(at + Duration::from_secs(1), 13.6).unwrap();
        assert!(
            line.ends_with("(1 more slow periods since the last line)"),
            "{line}"
        );
        assert_eq!(shared.parts.slow.load(Ordering::Relaxed), 3);
        // The status line has every part's worst and the slowest action.
        let s = p.status();
        assert!(
            s.starts_with("control parts (worst in 30 s): screen actions "),
            "{s}"
        );
        assert!(s.contains("slowest single: Looper(1)"), "{s}");
        assert!(
            s.ends_with(
                "; 3 slow periods (half a period or more outside the engines) since the start"
            ),
            "{s}"
        );
        // Restarted.
        assert!(!p.status().contains("slowest single"));
    }

    #[test]
    fn the_controller_output_row_turns_red_on_any_trouble() {
        let shared = PartsShared::default();
        let r = rows(&shared);
        assert_eq!(r[1].1, "0 · 0 · 0");
        assert_eq!(r[1].2, Tone::Normal);
        shared.set_counts(0, 3, 1, 2);
        let r = rows(&shared);
        assert_eq!(r[1].1, "0 · 3 · 1");
        assert_eq!(r[1].2, Tone::Bad);
        assert_eq!((r[2].1.as_str(), r[2].2), ("2", Tone::Bad));
    }

    #[test]
    fn time_between_a_lap_and_the_next_begin_is_not_counted() {
        let mut p = Parts::new(Arc::new(EngineShared::default()));
        p.begin();
        p.lap(MIDI_IN);
        // An engine runs here.
        std::thread::sleep(Duration::from_millis(50));
        p.begin();
        p.lap(PADS);
        assert!(p.total() < Duration::from_millis(50));
    }
}
