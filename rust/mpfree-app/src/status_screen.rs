//! System status on the screen (layout spec, section 2a; System status
//! phase 2, Jay's chip spec of 22 Nov 2026 08:17Z): the status chip at the
//! right of the top bar, and the page it opens.
//!
//! - **The chip** shows the CPU and the RAM in one fixed-size chip, "CPU
//!   24 %  RAM 38 %", green, yellow or red by the worst of the CPU, the RAM
//!   and the problems on the board ([`Board::chip_look`]). A number past its
//!   line takes that colour too, and a problem that ended unseen draws the
//!   chip as a ring. Words about problems never go on the chip.
//! - **The page** answers: is anything wrong, which part, what to do. It
//!   is built here from the board ([`view`]) and drawn by
//!   [`mpfree_ui::status_page::StatusPage`], which knows nothing of the
//!   board.
//! - **The hint line**: a red problem's title, in red, when it starts
//!   ([`Hinted`]).
//!
//! The screen thread owns all of it ([`crate::gui::Player`]); nothing
//! here runs on the audio thread.

use std::time::{Duration, Instant};

use mpfree_audiotracks::files::Room;
use mpfree_status::{
    kinds, Action, Board, ChipLook, Kind, Level, ProblemState, ProblemView, Shape, Subject, System,
    Window,
};
use mpfree_ui::status_page::{Card, StatusView, SystemRow};
use mpfree_ui::widgets::Chip;
use mpfree_ui::Tone;

use crate::heat::Heat;
use crate::status_watch::Watch;

/// The page's name, its widget and its layout: over the page, under the
/// top bar, as Prefs and the browser are.
pub const PAGE: &str = "System status";
pub const WIDGET: &str = "systemstatus";
pub const LAYOUT: &str = "col(header:60, systemstatus:*, hint:26)";

/// The chip's widest text: it is always this wide (Jay, 18 Nov 08:17Z,
/// "one fixed size chip").
pub const CHIP_ROOM: &str = "CPU 100 %  RAM 100 %";

/// How long a red problem waits before its title can go on the hint line
/// again.
pub const HINT_AGAIN: Duration = Duration::from_secs(60);

/// The colour of a level: red, yellow, or none for a note.
pub fn tone(level: Level) -> Option<Tone> {
    match level {
        Level::Fail => Some(Tone::Bad),
        Level::Warn => Some(Tone::Warn),
        Level::Note => None,
    }
}

fn tone_of(level: Option<Level>) -> Option<Tone> {
    level.and_then(tone)
}

/// The status chip: the CPU and RAM numbers, each in its colour past its
/// line, on a chip in the worst colour. Before the first readings it says
/// "CPU –" and is grey while MPFree starts (unless a problem colours it).
pub fn chip(look: ChipLook, cpu: Option<f64>, ram: Option<f64>, lit: bool) -> Chip {
    let cpu_text = cpu.map_or("CPU –".to_string(), |p| format!("CPU {p:.0} %"));
    let ram_text = ram.map_or("RAM –".to_string(), |p| format!("RAM {p:.0} %"));
    let tone = match tone_of(look.level) {
        Some(t) => t,
        None if cpu.is_none() => Tone::Dim,
        None => Tone::Good,
    };
    Chip::parts(
        vec![
            (cpu_text, tone_of(look.cpu)),
            ("  ".to_string(), None),
            (ram_text, tone_of(look.ram)),
        ],
        tone,
    )
    .room(CHIP_ROOM)
    .ring(look.ring)
    .lit(lit)
}

/// What the screen thread reads besides the board: the head's meters and
/// the systems' plain words.
#[derive(Clone, Debug, Default)]
pub struct Readings {
    /// The sound's share of the processor, %.
    pub cpu_pct: Option<f64>,
    /// RAM in use, %.
    pub ram_pct: Option<f64>,
    /// Memory free (MemAvailable), bytes.
    pub free: Option<u64>,
    /// The stick the audio tracks record to, at its last look (their drive
    /// watcher's, every 3 s), while it answers; None before the first look
    /// or once it stopped answering (Stick missing says so).
    pub stick: Option<Room>,
    /// The MPC's temperature and processor clock at the last read (every
    /// 2 s, heat.rs).
    pub heat: Option<Heat>,
    /// Where the sound's time goes, by engine, in the period's last window
    /// (`engines::shares`, in `engines::SHARES`' order, %).
    pub engines: Option<[f64; 6]>,
    /// The tracks using the most of the sound's time in the last window
    /// (track_costs.rs), largest first: whose, its words on the screen
    /// ("Track 3 · Drums") and its share of the period, %.
    pub tracks: Option<Vec<(crate::track_costs::Who, String, f64)>>,
}

/// Where the page is: the system picked and the Now | This session switch.
/// The page's cards (and their buttons) as last built, so a tap on a
/// button finds its job.
#[derive(Debug)]
pub struct Place {
    pub picked: System,
    pub window: Window,
    /// Card `i`'s button.
    pub actions: Vec<Option<Action>>,
    /// Seconds since the page was last filled (its ages move).
    pub drawn: f32,
}

impl Default for Place {
    fn default() -> Place {
        Place {
            picked: System::Sound,
            window: Window::Session,
            actions: Vec::new(),
            drawn: 0.0,
        }
    }
}

impl Place {
    /// The page opens on the worst system, the one that gave the chip its
    /// colour: red before yellow, then one with a problem happening now
    /// before one whose problem ended unseen, then the first in page order;
    /// all good, the first (Sound). The switch stays as it was left.
    pub fn opened(&mut self, board: &Board) {
        let systems = board.systems();
        let worst = systems
            .iter()
            .filter(|s| s.level.is_some_and(|l| l > Level::Note))
            .max_by_key(|s| (s.level, s.now > 0, std::cmp::Reverse(s.system.index())));
        self.picked = worst.map_or(System::Sound, |s| s.system);
    }

    /// The wheel or - and +: `delta` systems on or back, stopping at the
    /// ends.
    pub fn step(&mut self, delta: i32) {
        let i = self.picked.index() as i32 + delta;
        self.picked = System::ALL[i.clamp(0, System::ALL.len() as i32 - 1) as usize];
    }
}

/// "12 s", "4 min", "1 h 5 min".
pub fn span(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min", s / 60)
    } else {
        format!("{} h {} min", s / 3600, s % 3600 / 60)
    }
}

/// "1 problem", "3 problems".
fn problems(n: usize) -> String {
    if n == 1 {
        "1 problem".to_string()
    } else {
        format!("{n} problems")
    }
}

/// When and how often, on a card: "3 times · last 12 s ago · first 4 min
/// ago", "now · for 9 s", "for 4 s · ended 30 s ago".
pub fn when(p: &ProblemView, now: Instant) -> String {
    let ago = |t: Instant| span(now.saturating_duration_since(t));
    match p.kind.shape {
        // Several at one moment ("6 voices"): when, once.
        Shape::Happens
            if p.times > 1
                && p.last.saturating_duration_since(p.first) < Duration::from_secs(1) =>
        {
            if p.is_now() {
                format!("{} times · now", p.times)
            } else {
                format!("{} times · {} ago", p.times, ago(p.last))
            }
        }
        Shape::Happens if p.times > 1 => format!(
            "{} times · last {} ago · first {} ago",
            p.times,
            ago(p.last),
            ago(p.first)
        ),
        Shape::Happens if p.is_now() => "now".to_string(),
        Shape::Happens => format!("{} ago", ago(p.last)),
        _ => {
            let spells = if p.times > 1 {
                format!(" · {} times this session", p.times)
            } else {
                String::new()
            };
            match (p.state, p.since, p.ended) {
                (ProblemState::Now, Some(since), _) => format!("now · for {}{spells}", ago(since)),
                (ProblemState::Now, None, _) => format!("now{spells}"),
                (_, _, Some(ended)) => format!("ended {} ago{spells}", ago(ended)),
                _ => format!("{} ago{spells}", ago(p.last)),
            }
        }
    }
}

/// A problem's name with its subject's ("A MIDI device isn't plugged in:
/// Juno-X").
fn titled(p: &ProblemView, names: &Watch) -> String {
    match names.subject_name(p.subject) {
        Some(name) if p.subject != Subject::NONE => format!("{}: {name}", p.kind.title),
        _ => p.kind.title.to_string(),
    }
}

/// The short words for a system's line, with the subject's name in front
/// ("Juno-X: not plugged in").
fn short(p: &ProblemView, names: &Watch) -> String {
    match names.subject_name(p.subject) {
        Some(name) if p.subject != Subject::NONE => {
            format!("{name}: {}", p.kind.short.to_lowercase())
        }
        _ => p.kind.short.to_string(),
    }
}

/// Bytes in plain words: "1.2 GB", "640 MB".
fn bytes(b: u64) -> String {
    let gb = b as f64 / 1e9;
    if gb >= 1.0 {
        format!("{gb:.1} GB")
    } else {
        format!("{:.0} MB", b as f64 / 1e6)
    }
}

/// A system's own words when nothing is wrong with it.
fn all_good_line(system: System, r: &Readings, board: &Board) -> String {
    match system {
        System::Sound => match r.cpu_pct {
            Some(p) if board.problems(System::Sound, Window::Session).is_empty() => {
                format!("CPU {p:.0} % · no dropouts")
            }
            Some(p) => format!("CPU {p:.0} %"),
            None => "Starting".to_string(),
        },
        System::Memory => match (r.ram_pct, r.free) {
            (Some(p), Some(f)) => format!("{p:.0} % used · {} free", bytes(f)),
            (Some(p), None) => format!("{p:.0} % used"),
            _ => "Working".to_string(),
        },
        System::Storage => match r.stick {
            Some(s) => format!("{} free on the stick", bytes(s.free)),
            None => "Working".to_string(),
        },
        System::Mpc => match r.heat.map(|h| (h.temp_c, h.slowed)) {
            Some((Some(c), Some(false))) => format!("{c:.0} °C · full speed"),
            Some((Some(c), _)) => format!("{c:.0} °C"),
            _ => "Working".to_string(),
        },
        _ => "Working".to_string(),
    }
}

/// The page from the board as it stands.
pub fn view(
    board: &Board,
    names: &Watch,
    r: &Readings,
    place: &mut Place,
    now: Instant,
) -> StatusView {
    let window = place.window;
    let systems = board.systems();
    // The head: the worst problem happening now, or all good.
    let (mut now_n, mut earlier_n) = (0, 0);
    for s in &systems {
        now_n += s.now;
        earlier_n += s.earlier;
    }
    let worst_now = System::ALL
        .iter()
        .flat_map(|&s| board.problems(s, Window::Now))
        .filter(|p| p.kind.level > Level::Note)
        .max_by_key(|p| (p.kind.level, p.last));
    let (head, head_tone) = match worst_now {
        Some(p) => (
            titled(&p, names),
            tone(p.kind.level).unwrap_or(Tone::Normal),
        ),
        None if earlier_n > 0 => ("Everything is working now".to_string(), Tone::Good),
        None => ("Everything is working".to_string(), Tone::Good),
    };
    let counts = match (now_n, earlier_n) {
        (0, 0) => "Nothing went wrong this session.".to_string(),
        (0, e) => format!(
            "{} earlier this session: tap a system to see them.",
            problems(e)
        ),
        (n, 0) => format!("{} now: tap a system to see why.", problems(n)),
        (n, e) => format!(
            "{} now and {e} earlier: tap a system to see why.",
            problems(n)
        ),
    };
    // The meters: each once it can be read.
    let look = board.chip_look(r.ram_pct);
    let meter_tone = |l: Option<Level>| tone_of(l).unwrap_or(Tone::Good);
    let mut meters = Vec::new();
    if let Some(p) = r.cpu_pct {
        meters.push((
            "CPU".to_string(),
            format!("{p:.0} %"),
            (p / 100.0).clamp(0.0, 1.0) as f32,
            meter_tone(look.cpu),
        ));
    }
    if let Some(p) = r.ram_pct {
        meters.push((
            "RAM".to_string(),
            format!("{p:.0} %"),
            (p / 100.0).clamp(0.0, 1.0) as f32,
            meter_tone(look.ram),
        ));
    }
    // The stick: how full, yellow while it is nearly full.
    if let Some(s) = r.stick.filter(|s| s.total > 0) {
        let used = 1.0 - s.free.min(s.total) as f64 / s.total as f64;
        let full = board
            .problems(System::Storage, Window::Now)
            .iter()
            .any(|p| p.kind == &kinds::STICK_NEARLY_FULL);
        meters.push((
            "Stick".to_string(),
            format!("{} free", bytes(s.free)),
            used as f32,
            if full { Tone::Warn } else { Tone::Good },
        ));
    }
    // The MPC's temperature, against where the kernel turns it off (or
    // 100 °C where it doesn't say), yellow while it is running hot.
    if let Some(c) = r.heat.and_then(|h| h.temp_c) {
        let off = r.heat.and_then(|h| h.critical_c).unwrap_or(100.0);
        let hot = board
            .problems(System::Mpc, Window::Now)
            .iter()
            .any(|p| p.kind == &kinds::MPC_HOT);
        meters.push((
            "Temp".to_string(),
            format!("{c:.0} °C"),
            (c / off).clamp(0.0, 1.0),
            if hot { Tone::Warn } else { Tone::Good },
        ));
    }
    // The systems: always all nine.
    let rows = systems
        .iter()
        .map(|s| {
            let listed: Vec<ProblemView> = board
                .problems(s.system, window)
                .into_iter()
                .filter(|p| p.kind.level > Level::Note)
                .collect();
            let line = match listed.first() {
                Some(p) => {
                    let mut line = short(p, names);
                    if s.system == System::Sound {
                        if let Some(c) = r.cpu_pct {
                            line = format!("{line} · CPU {c:.0} %");
                        }
                    }
                    line
                }
                None => all_good_line(s.system, r, board),
            };
            SystemRow {
                name: s.system.name().to_string(),
                line,
                tone: Some(tone_of(s.level).unwrap_or(Tone::Good)),
                ring: s.ring,
                badge: listed.len(),
            }
        })
        .collect();
    // The picked system's cards, worst and newest first.
    let picked = board.problems(place.picked, window);
    place.actions = picked.iter().map(|p| p.kind.action).collect();
    let cards = picked
        .iter()
        .map(|p| Card {
            title: titled(p, names),
            tone: tone(p.kind.level).unwrap_or(Tone::Normal),
            now: p.is_now(),
            when: when(p, now),
            what: p.kind.what.to_string(),
            to_try: p.kind.to_try.to_string(),
            button: p.kind.action.map(|a| a.label().to_string()),
        })
        .collect();
    let numbers = numbers(place.picked, r);
    StatusView {
        head,
        head_tone,
        counts,
        meters,
        systems: rows,
        picked: place.picked.index(),
        now_only: window == Window::Now,
        cards,
        numbers,
    }
}

/// A system's own numbers, under its cards.
fn numbers(system: System, r: &Readings) -> Vec<(String, String)> {
    let mut out = Vec::new();
    match system {
        System::Sound => {
            if let Some(p) = r.cpu_pct {
                out.push((
                    "The sound's share of the CPU".to_string(),
                    format!("{p:.0} %"),
                ));
            }
            // The tracks using the most (track_costs.rs), first, so they
            // stay in view when cards push the numbers down; then where it
            // goes, each engine's part of that share, under "By part".
            let tracks = r.tracks.as_deref().filter(|t| !t.is_empty());
            if let Some(tracks) = tracks {
                out.push(("The tracks using the most".to_string(), String::new()));
                for (_, words, p) in tracks {
                    out.push((format!("  {words}"), crate::engines::share_words(*p)));
                }
            }
            if let Some(s) = r.engines {
                if tracks.is_some() {
                    out.push(("By part".to_string(), String::new()));
                }
                for ((_, name), p) in crate::engines::SHARES.iter().zip(s) {
                    out.push((format!("  {name}"), crate::engines::share_words(p)));
                }
            }
        }
        System::Memory => {
            if let Some(p) = r.ram_pct {
                out.push(("RAM in use".to_string(), format!("{p:.0} %")));
            }
            if let Some(f) = r.free {
                out.push(("Free".to_string(), bytes(f)));
            }
        }
        System::Storage => {
            if let Some(s) = r.stick {
                out.push(("Free on the stick".to_string(), bytes(s.free)));
                out.push(("The stick's size".to_string(), bytes(s.total)));
                if !s.writable {
                    out.push(("The stick".to_string(), "read only".to_string()));
                }
            }
        }
        System::Mpc => {
            let Some(h) = r.heat else {
                return out;
            };
            if let Some(c) = h.temp_c {
                out.push(("Temperature".to_string(), format!("{c:.0} °C")));
            }
            if let Some(c) = h.critical_c {
                out.push(("It turns itself off at".to_string(), format!("{c:.0} °C")));
            }
            if let Some(m) = h.clock_mhz {
                out.push(("Processor speed".to_string(), format!("{m} MHz")));
            }
            match (h.top_mhz, h.full_mhz) {
                (Some(t), Some(f)) if t < f => {
                    out.push(("Top speed".to_string(), format!("{t} MHz, down from {f}")));
                }
                (_, Some(f)) => out.push(("Top speed".to_string(), format!("{f} MHz"))),
                _ => {}
            }
        }
        _ => {}
    }
    out
}

/// When each red problem last went on the hint line: it goes there when it
/// starts, for the hint line's few seconds, and again only after
/// [`HINT_AGAIN`] (Jay's approval, 22 Nov 08:35Z).
#[derive(Debug, Default)]
pub struct Hinted {
    last: Vec<(&'static str, Instant)>,
}

impl Hinted {
    /// The hint line's words for `kind` starting at `now`, if it should go
    /// there: red, not an echo of a cause that is going, and not on the
    /// hint line in the last minute.
    pub fn line(&mut self, kind: &'static Kind, hidden: bool, now: Instant) -> Option<String> {
        if kind.level != Level::Fail || hidden {
            return None;
        }
        match self.last.iter_mut().find(|(id, _)| *id == kind.id) {
            Some((_, at)) if now.saturating_duration_since(*at) < HINT_AGAIN => return None,
            Some((_, at)) => *at = now,
            None => self.last.push((kind.id, now)),
        }
        Some(format!("{}. Tap the status chip to see why.", kind.title))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_status::{kinds, Report};

    fn board_with(reports: &[Report], at: Instant) -> Board {
        let mut b = Board::new();
        for r in reports {
            b.apply(*r, at);
        }
        b.tick(at);
        b
    }

    #[test]
    fn the_chip_is_one_fixed_size_and_takes_the_worst_colour() {
        let t = Instant::now();
        let quiet = Board::new();
        let c = chip(quiet.chip_look(Some(38.0)), Some(24.0), Some(38.0), false);
        assert_eq!(c.text, "CPU 24 %  RAM 38 %");
        assert_eq!(c.tone, Tone::Good);
        assert_eq!(c.room, Some(CHIP_ROOM));
        assert!(!c.ring);
        // Starting: no CPU yet, grey.
        let c = chip(quiet.chip_look(None), None, None, false);
        assert_eq!((c.text.as_str(), c.tone), ("CPU –  RAM –", Tone::Dim));
        // RAM past its red line: the chip and the RAM number red, the CPU
        // plain.
        let c = chip(quiet.chip_look(Some(90.0)), Some(24.0), Some(90.0), false);
        assert_eq!(c.tone, Tone::Bad);
        assert_eq!(c.inks[0].1, None);
        assert_eq!(c.inks[2].1, Some(Tone::Bad));
        // The sound dropped out: red, the CPU number too.
        let b = board_with(&[Report::happened(&kinds::SOUND_DROPOUT, 1)], t);
        let c = chip(b.chip_look(Some(38.0)), Some(92.0), Some(38.0), true);
        assert_eq!(
            (c.tone, c.inks[0].1, c.lit),
            (Tone::Bad, Some(Tone::Bad), true)
        );
        // No words about problems on the chip, ever.
        assert!(!c.text.contains("dropped"));
    }

    #[test]
    fn the_page_opens_on_the_worst_system_and_names_its_problems() {
        let t = Instant::now();
        let b = board_with(
            &[
                Report::began(&kinds::PORT_MISSING).about(Subject::midi_port(0)),
                Report::happened(&kinds::SOUND_DROPOUT, 3),
            ],
            t,
        );
        let mut place = Place::default();
        place.opened(&b);
        assert_eq!(place.picked, System::Sound, "red now before yellow now");
        let r = Readings {
            cpu_pct: Some(92.0),
            ram_pct: Some(41.0),
            free: Some(1_100_000_000),
            stick: None,
            heat: None,
            engines: None,
            tracks: None,
        };
        let v = view(&b, &Watch::default(), &r, &mut place, t);
        assert_eq!(v.head, "The sound dropped out");
        assert_eq!(v.head_tone, Tone::Bad);
        assert_eq!(v.counts, "2 problems now: tap a system to see why.");
        assert_eq!(v.systems.len(), 9);
        assert_eq!(v.systems[0].line, "Sound dropped out · CPU 92 %");
        assert_eq!(v.systems[0].badge, 1);
        assert_eq!(v.systems[1].line, "41 % used · 1.1 GB free");
        assert_eq!(v.systems[4].tone, Some(Tone::Warn));
        assert_eq!(v.cards.len(), 1);
        assert_eq!(v.cards[0].when, "3 times · now");
        assert!(v.cards[0].now);
        assert_eq!(v.meters[0].0, "CPU");
        assert_eq!(v.meters[0].3, Tone::Bad);
        // Step to MIDI and sync: the port's card.
        for _ in 0..4 {
            place.step(1);
        }
        let v = view(&b, &Watch::default(), &r, &mut place, t);
        assert_eq!(v.picked, 4);
        assert_eq!(v.cards[0].title, "A MIDI device isn't plugged in");
        place.step(9);
        assert_eq!(place.picked, System::Mpc, "stops at the end");
    }

    #[test]
    fn all_good_says_so_and_ended_problems_show_as_earlier() {
        let t = Instant::now();
        let mut b = Board::new();
        let mut place = Place::default();
        let r = Readings {
            cpu_pct: Some(24.0),
            ..Readings::default()
        };
        let v = view(&b, &Watch::default(), &r, &mut place, t);
        assert_eq!(v.head, "Everything is working");
        assert_eq!(v.counts, "Nothing went wrong this session.");
        assert_eq!(v.systems[0].line, "CPU 24 % · no dropouts");
        assert!(v.cards.is_empty());
        b.apply(Report::happened(&kinds::SAVE_FAILED, 1), t);
        let later = t + Duration::from_secs(30);
        b.tick(later);
        place.opened(&b);
        assert_eq!(place.picked, System::Storage);
        let v = view(&b, &Watch::default(), &r, &mut place, later);
        assert_eq!(v.head, "Everything is working now");
        assert_eq!(
            v.counts,
            "1 problem earlier this session: tap a system to see them."
        );
        assert_eq!(v.cards[0].when, "30 s ago");
        assert_eq!(place.actions, [None]);
        place.window = Window::Now;
        let v = view(&b, &Watch::default(), &r, &mut place, later);
        assert!(v.cards.is_empty() && v.now_only);
    }

    /// Phase 3: the stick's meter on the head once the audio tracks have
    /// looked at it, yellow while it is nearly full, with its free space on
    /// Storage's line and its numbers under the cards.
    #[test]
    fn the_stick_has_a_meter_once_it_is_looked_at() {
        let t = Instant::now();
        let mut place = Place::default();
        let r = Readings {
            ram_pct: Some(38.0),
            ..Readings::default()
        };
        let v = view(&Board::new(), &Watch::default(), &r, &mut place, t);
        assert_eq!(v.meters.len(), 1, "not looked at yet: no stick meter");
        assert_eq!(v.systems[2].line, "Working");
        let r = Readings {
            stick: Some(Room {
                writable: true,
                free: 12_400_000_000,
                total: 15_500_000_000,
            }),
            ..r
        };
        let v = view(&Board::new(), &Watch::default(), &r, &mut place, t);
        let m = &v.meters[1];
        assert_eq!((m.0.as_str(), m.1.as_str()), ("Stick", "12.4 GB free"));
        assert!((m.2 - 0.2).abs() < 1e-3, "a fifth used: {}", m.2);
        assert_eq!(m.3, Tone::Good);
        assert_eq!(v.systems[2].line, "12.4 GB free on the stick");
        place.picked = System::Storage;
        let v = view(&Board::new(), &Watch::default(), &r, &mut place, t);
        assert_eq!(
            v.numbers,
            [
                ("Free on the stick".to_string(), "12.4 GB".to_string()),
                ("The stick's size".to_string(), "15.5 GB".to_string()),
            ]
        );
        // Nearly full: the meter turns yellow with the problem.
        let mut b = board_with(&[Report::began(&kinds::STICK_NEARLY_FULL)], t);
        let later = t + Duration::from_secs(2);
        b.tick(later);
        let v = view(&b, &Watch::default(), &r, &mut place, later);
        assert_eq!(v.meters[1].3, Tone::Warn);
    }

    /// Sound's numbers say where the sound's time goes, engine by engine,
    /// under its share of the CPU.
    #[test]
    fn the_sounds_numbers_say_where_its_time_goes() {
        let mut place = Place::default();
        let r = Readings {
            cpu_pct: Some(41.0),
            engines: Some([24.4, 0.0, 0.3, 2.0, 6.0, 8.2]),
            ..Readings::default()
        };
        place.picked = System::Sound;
        let v = view(
            &Board::new(),
            &Watch::default(),
            &r,
            &mut place,
            Instant::now(),
        );
        let n: Vec<(&str, &str)> = v
            .numbers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(
            n,
            [
                ("The sound's share of the CPU", "41 %"),
                ("  The pads", "24 %"),
                ("  Synths", "none"),
                ("  The looper and line input", "under 1 %"),
                ("  Clips and the arp", "2 %"),
                ("  Mixing", "6 %"),
                ("  MPFree's own work", "8 %"),
            ]
        );
    }

    /// The tracks using the most come first under the CPU share, then the
    /// engines under "By part"; with no track listed the engines stand as
    /// before.
    #[test]
    fn the_sounds_numbers_list_the_tracks_using_the_most() {
        use crate::track_costs::Who;
        let mut place = Place {
            picked: System::Sound,
            ..Place::default()
        };
        let r = Readings {
            cpu_pct: Some(38.0),
            engines: Some([17.6, 4.9, 0.0, 1.2, 6.3, 8.1]),
            tracks: Some(vec![
                (Who::Track(2), "Track 3 · Drums".into(), 12.6),
                (Who::Kit(1), "Kit 2 · pads only".into(), 0.4),
            ]),
            ..Readings::default()
        };
        let v = view(
            &Board::new(),
            &Watch::default(),
            &r,
            &mut place,
            Instant::now(),
        );
        let n: Vec<(&str, &str)> = v
            .numbers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(
            n[..6],
            [
                ("The sound's share of the CPU", "38 %"),
                ("The tracks using the most", ""),
                ("  Track 3 · Drums", "13 %"),
                ("  Kit 2 · pads only", "under 1 %"),
                ("By part", ""),
                ("  The pads", "18 %"),
            ]
        );
        assert_eq!(n.len(), 11);
        // Nothing listed (nothing played): the engines as before.
        let r = Readings {
            tracks: Some(Vec::new()),
            ..r
        };
        let v = view(
            &Board::new(),
            &Watch::default(),
            &r,
            &mut place,
            Instant::now(),
        );
        assert_eq!(v.numbers.len(), 7);
        assert_eq!(v.numbers[1].0, "  The pads");
    }

    /// Phase 3: the MPC's temperature on the head against where it turns
    /// itself off, yellow while it is running hot, with the processor's
    /// speed on The MPC's line and its numbers.
    #[test]
    fn the_mpc_shows_its_temperature_and_its_speed() {
        let t = Instant::now();
        let mut place = Place::default();
        let mut heat = Heat {
            temp_c: Some(56.5),
            critical_c: Some(113.0),
            clock_mhz: Some(1608),
            top_mhz: Some(1608),
            full_mhz: Some(1608),
            slowed: Some(false),
        };
        let r = Readings {
            heat: Some(heat),
            ..Readings::default()
        };
        let v = view(&Board::new(), &Watch::default(), &r, &mut place, t);
        let m = &v.meters[0];
        assert_eq!((m.0.as_str(), m.1.as_str()), ("Temp", "56 °C"));
        assert!((m.2 - 0.5).abs() < 1e-3, "half way to off: {}", m.2);
        assert_eq!(m.3, Tone::Good);
        assert_eq!(v.systems[8].line, "56 °C · full speed");
        place.picked = System::Mpc;
        let v = view(&Board::new(), &Watch::default(), &r, &mut place, t);
        assert_eq!(
            v.numbers,
            [
                ("Temperature".to_string(), "56 °C".to_string()),
                ("It turns itself off at".to_string(), "113 °C".to_string()),
                ("Processor speed".to_string(), "1608 MHz".to_string()),
                ("Top speed".to_string(), "1608 MHz".to_string()),
            ]
        );
        // Slowed, and running hot: the meter
        // turns yellow, and the numbers say how far the speed came down.
        heat.top_mhz = Some(1200);
        heat.clock_mhz = Some(1200);
        heat.slowed = Some(true);
        let r = Readings {
            heat: Some(heat),
            ..r
        };
        let mut b = board_with(
            &[
                Report::began(&kinds::MPC_HOT),
                Report::began(&kinds::PROCESSOR_SLOWED),
            ],
            t,
        );
        let later = t + Duration::from_secs(2);
        b.tick(later);
        let v = view(&b, &Watch::default(), &r, &mut place, later);
        assert_eq!(v.meters[0].3, Tone::Warn);
        assert!(
            v.numbers.contains(&(
                "Top speed".to_string(),
                "1200 MHz, down from 1608".to_string()
            )),
            "{:?}",
            v.numbers
        );
        // A machine without the files: no meter, and The MPC just works.
        let r = Readings {
            heat: Some(Heat::default()),
            ..Readings::default()
        };
        let v = view(&Board::new(), &Watch::default(), &r, &mut place, t);
        assert!(v.meters.is_empty());
        assert_eq!(v.systems[8].line, "Working");
    }

    /// The anchor mdBook gives a heading.
    fn slug(heading: &str) -> String {
        heading
            .to_lowercase()
            .chars()
            .filter_map(|c| match c {
                ' ' => Some('-'),
                c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
                _ => None,
            })
            .collect()
    }

    /// Every kind has its entry in the manual's "When something goes
    /// wrong", under its title, at its anchor.
    #[test]
    fn every_problem_has_its_entry_in_the_manual() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/user/src/reference/when-something-goes-wrong.md"
        ))
        .expect("the manual's chapter");
        for k in kinds::ALL {
            assert!(text.contains(&format!("\n### {}\n", k.title)), "{}", k.id);
            assert_eq!(slug(k.title), k.manual, "{}", k.id);
            let label = match k.level {
                Level::Fail => "**Red.**",
                Level::Warn => "**Yellow.**",
                Level::Note => "**Note.**",
            };
            let entry = text
                .split(&format!("### {}\n", k.title))
                .nth(1)
                .unwrap_or_default();
            assert!(entry.trim_start().starts_with(label), "{}: {label}", k.id);
        }
    }

    #[test]
    fn a_red_problem_goes_on_the_hint_line_once_a_minute() {
        let t = Instant::now();
        let mut h = Hinted::default();
        assert_eq!(
            h.line(&kinds::SOUND_DROPOUT, false, t).as_deref(),
            Some("The sound dropped out. Tap the status chip to see why.")
        );
        assert_eq!(
            h.line(&kinds::SOUND_DROPOUT, false, t + Duration::from_secs(30)),
            None
        );
        assert!(
            h.line(&kinds::SAVE_FAILED, false, t).is_some(),
            "each kind its own minute"
        );
        assert!(h
            .line(&kinds::SOUND_DROPOUT, false, t + HINT_AGAIN)
            .is_some());
        assert_eq!(h.line(&kinds::PORT_MISSING, false, t), None, "yellow never");
        assert_eq!(h.line(&kinds::NO_SESSION, true, t), None, "an echo never");
    }
}
