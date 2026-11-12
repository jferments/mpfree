//! The Debug tab's pages beyond the MIDI log (debug mode, from Prefs):
//!
//! - Performance: CPU per thread and for the whole process, memory, load,
//!   temperature and clock (`perf.rs`, read by the screen thread), and the
//!   audio loop's own numbers (render time against the 2.9 ms period,
//!   underruns, steals, grains), with the last minute of render time drawn.
//! - CPU by engine: each stage of the audio loop (the whole period, the
//!   sampler, the synth, the looper, the transport, the mix) as mean, p99
//!   and worst against the period and its budget (`engines.rs`).
//! - Processes: every process on the MPC from /proc, busiest first, with
//!   memory, threads and scheduling, and each core's CPU (mpfree-prof's
//!   `procs`; names only, never command lines).
//! - Map controls: every MPC One control in a list, found, skipped or still
//!   to find, and a card for the one picked (`app::mapper`).
//!
//! - Touch: test, measure and tune the touchscreen (`touch_dev.rs`).
//! - Console: the system log (the same page SHIFT + STOP opens), here with
//!   the strip.
//!
//! A strip under the header switches between the pages.

use crate::app::{MapView, Mark, View, DEBUG_PAGES};
use crate::engines;
use crate::perf::ProcView;
use mpfree_arp_ui::widgets::Segments;
use mpfree_prof::procs;
use mpfree_ui::widgets::{Card, ListItem, ListView, StatList};
use mpfree_ui::{Align, Color, FontId, PaintCx, Rect, Tone, Ui, Widget};
use std::collections::VecDeque;

pub const STRIP: &str = "dtabs";
pub const PERF_PAGE: &str = "Performance";
pub const ENGINES_PAGE: &str = "CPU by engine";
pub const PROCS_PAGE: &str = "Processes";
pub const MAP_PAGE: &str = "Map controls";
pub const CONSOLE_PAGE: &str = "Dev console";
pub const DEBUG_LAYOUT: &str =
    "col(header:60, dtabs:52, row(midi:*, col(events:*, timing:232):460):*, hint:26)";
pub const PERF_LAYOUT: &str =
    "col(header:60, dtabs:52, row(col(threads:*, graph:210):*, col(audio:*, system:*):500):*, hint:26)";
pub const ENGINES_LAYOUT: &str =
    "col(header:60, dtabs:52, row(engnow:*, engsession:560):*, hint:26)";
pub const PROCS_LAYOUT: &str = "col(header:60, dtabs:52, row(procs:*, cores:420):*, hint:26)";
pub const MAP_LAYOUT: &str =
    "col(header:60, dtabs:52, row(controls:*, col(mapcard:*, mapinfo:236):660):*, hint:26)";
pub const CONSOLE_LAYOUT: &str = "col(header:60, dtabs:52, syslog:*, hint:26)";

/// How much render time the graph keeps: one value per screen update of
/// the timing (every 250 ms), a minute's worth.
const HISTORY: usize = 240;

/// The audio loop's numbers for the Performance page (from `Timing`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AudioNumbers {
    /// The sampler's render time over the last 250 ms: mean and largest, µs.
    pub render_mean_us: f32,
    pub render_max_us: f32,
    /// The synth's, likewise.
    pub synth_mean_us: f32,
    pub synth_max_us: f32,
    /// The largest render time (sampler and synth) since the start, µs.
    pub worst_us: f32,
    pub steals: u64,
    pub dropped: u64,
    pub peak_grains: usize,
}

/// The period: 128 frames at 44.1 kHz, in µs.
pub const PERIOD_US: f32 = 128.0 * 1e6 / 44_100.0;

/// A bar graph of the last values, 0 to `full`, newest on the right, with a
/// line at `mark`.
pub struct Graph {
    pub title: String,
    pub right: String,
    pub values: VecDeque<f32>,
    pub full: f32,
    pub mark: f32,
}

impl Graph {
    pub fn new(title: &str, full: f32, mark: f32) -> Graph {
        Graph {
            title: title.to_string(),
            right: String::new(),
            values: VecDeque::new(),
            full,
            mark,
        }
    }
}

impl Widget for Graph {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let body = cx.titled_panel(r, &self.title, &self.right);
        if body.h < 20 || body.w < 20 {
            return;
        }
        cx.canvas.fill_round_rect(body, 8, t.bg);
        let n = HISTORY as i32;
        let bar_w = (body.w / n).max(1);
        let x0 = body.right() - bar_w * self.values.len() as i32;
        for (i, &v) in self.values.iter().enumerate() {
            let h = ((v / self.full).clamp(0.0, 1.0) * body.h as f32).round() as i32;
            if h <= 0 {
                continue;
            }
            let colour = if v >= self.mark {
                t.tone(Tone::Bad)
            } else if v >= self.mark * 0.6 {
                t.tone(Tone::Warn)
            } else {
                t.tone(Tone::Good)
            };
            cx.canvas.fill_rect(
                Rect::new(x0 + i as i32 * bar_w, body.bottom() - h, bar_w, h),
                colour,
            );
        }
        let y = body.bottom()
            - ((self.mark / self.full).clamp(0.0, 1.0) * body.h as f32).round() as i32;
        cx.canvas
            .fill_rect(Rect::new(body.x, y, body.w, 1), t.text_dim);
        let label = Rect::new(body.x + 6, (y - 22).max(body.y), body.w - 12, 20);
        cx.text(
            FontId::Sans,
            t.size_small,
            label,
            Align::Left,
            t.text_dim,
            &format!("{:.0} %", self.mark),
        );
    }
}

/// The Dev tab's widgets and what they show.
pub struct DebugPages {
    history: VecDeque<f32>,
    audio: AudioNumbers,
    proc_view: ProcView,
}

fn found_colour(m: Mark) -> Option<Color> {
    match m {
        Mark::Found(_) => Some(Color::from_pad([0x08, 0x30, 0x10])),
        Mark::Skipped => Some(Color::from_pad([0x14, 0x14, 0x14])),
        Mark::Left => None,
    }
}

impl DebugPages {
    /// Adds the widgets and the pages ("Debug" is the MIDI log; the Console
    /// page uses the "syslog" widget, added with the plain Console page).
    pub fn add(ui: &mut Ui) -> Result<DebugPages, String> {
        ui.add(
            STRIP,
            Segments::new(&[
                "MIDI",
                "Performance",
                "CPU by engine",
                "Processes",
                "Map controls",
                "Touch",
                "Console",
            ]),
        );
        ui.add("threads", StatList::new("Threads, CPU (100 % = one core)"));
        ui.add(
            "graph",
            Graph::new(
                "Audio render, % of a 2.9 ms period, last minute",
                100.0,
                70.0,
            ),
        );
        ui.add("audio", StatList::new("Audio loop"));
        ui.add("system", StatList::new("Process and system"));
        ui.add(
            "engnow",
            StatList::new("Audio loop, now: mean · p99 · worst (share of a period)"),
        );
        ui.add(
            "engsession",
            StatList::new("Since the start, against the budgets"),
        );
        ui.add(
            "procs",
            StatList::new("Processes, busiest first (100 % = one core)"),
        );
        ui.add("cores", StatList::new("CPU and memory"));
        ui.add("controls", ListView::new("Controls"));
        let mut card = Card::new("Map controls");
        card.buttons = vec![
            "Listen".to_string(),
            "Skip".to_string(),
            "Clear".to_string(),
        ];
        ui.add("mapcard", card);
        ui.add(
            "mapinfo",
            StatList::new("Pressed last, and measured controls"),
        );
        ui.add_page("Debug", DEBUG_LAYOUT)?;
        ui.add_page(PERF_PAGE, PERF_LAYOUT)?;
        ui.add_page(ENGINES_PAGE, ENGINES_LAYOUT)?;
        ui.add_page(PROCS_PAGE, PROCS_LAYOUT)?;
        ui.add_page(MAP_PAGE, MAP_LAYOUT)?;
        ui.add(
            crate::touch_dev::TEST,
            crate::touch_dev::TouchTest::default(),
        );
        ui.add(
            crate::touch_dev::TUNE,
            crate::touch_dev::TuneList::default(),
        );
        ui.add_page(crate::touch_dev::PAGE, crate::touch_dev::LAYOUT)?;
        ui.add_page(
            crate::touch_dev::TARGETS_PAGE,
            crate::touch_dev::TARGETS_LAYOUT,
        )?;
        ui.add_page(CONSOLE_PAGE, CONSOLE_LAYOUT)?;
        let mut d = DebugPages {
            history: VecDeque::new(),
            audio: AudioNumbers::default(),
            proc_view: ProcView::default(),
        };
        d.show_audio(ui);
        d.show_proc(ui);
        DebugPages::set_engines(ui, &[(None, None); 6]);
        DebugPages::set_procs(ui, &procs::ProcView::default());
        Ok(d)
    }

    /// The page for a debug view.
    pub fn page(v: View) -> Option<&'static str> {
        match v {
            View::Debug => Some("Debug"),
            View::Perf => Some(PERF_PAGE),
            View::Engines => Some(ENGINES_PAGE),
            View::Procs => Some(PROCS_PAGE),
            View::Map => Some(MAP_PAGE),
            View::Touch => Some(crate::touch_dev::PAGE),
            View::Console => Some(CONSOLE_PAGE),
            _ => None,
        }
    }

    /// The strip follows the page.
    pub fn set_strip(ui: &mut Ui, v: View) {
        if let (Some(s), Some(i)) = (
            ui.get_mut::<Segments>(STRIP),
            DEBUG_PAGES.iter().position(|&p| p == v),
        ) {
            s.active = i;
        }
    }

    pub fn set_audio(&mut self, ui: &mut Ui, a: AudioNumbers) {
        self.audio = a;
        if self.history.len() >= HISTORY {
            self.history.pop_front();
        }
        self.history
            .push_back(100.0 * (a.render_max_us + a.synth_max_us) / PERIOD_US);
        self.show_audio(ui);
    }

    fn show_audio(&mut self, ui: &mut Ui) {
        let a = self.audio;
        let pct = |us: f32| 100.0 * us / PERIOD_US;
        let tone = |us: f32| {
            if pct(us) >= 70.0 {
                Tone::Bad
            } else if pct(us) >= 40.0 {
                Tone::Warn
            } else {
                Tone::Good
            }
        };
        let rows = vec![
            (
                "Period (128 frames)".to_string(),
                format!("{PERIOD_US:.0} µs"),
                Tone::Dim,
            ),
            (
                "Sampler render, mean · max".to_string(),
                format!(
                    "{:.0} · {:.0} µs ({:.0} %)",
                    a.render_mean_us,
                    a.render_max_us,
                    pct(a.render_max_us)
                ),
                tone(a.render_max_us),
            ),
            (
                "Synth render, mean · max".to_string(),
                format!(
                    "{:.0} · {:.0} µs ({:.0} %)",
                    a.synth_mean_us,
                    a.synth_max_us,
                    pct(a.synth_max_us)
                ),
                tone(a.synth_max_us),
            ),
            (
                "Worst period so far".to_string(),
                format!("{:.0} µs ({:.0} %)", a.worst_us, pct(a.worst_us)),
                tone(a.worst_us),
            ),
            (
                "Voices stolen".to_string(),
                a.steals.to_string(),
                Tone::Normal,
            ),
            (
                "Events dropped".to_string(),
                a.dropped.to_string(),
                if a.dropped == 0 {
                    Tone::Good
                } else {
                    Tone::Bad
                },
            ),
            (
                "Most grains at once".to_string(),
                a.peak_grains.to_string(),
                Tone::Normal,
            ),
        ];
        if let Some(s) = ui.get_mut::<StatList>("audio") {
            s.set(rows);
        }
        let now = self.history.back().copied().unwrap_or(0.0);
        if let Some(g) = ui.get_mut::<Graph>("graph") {
            g.values = self.history.clone();
            g.right = format!("now {now:.0} %");
        }
    }

    pub fn set_proc(&mut self, ui: &mut Ui, v: ProcView) {
        self.proc_view = v;
        self.show_proc(ui);
    }

    fn show_proc(&mut self, ui: &mut Ui) {
        let v = &self.proc_view;
        let mut rows: Vec<(String, String, Tone)> = v
            .threads
            .iter()
            .take(12)
            .map(|(name, c)| {
                (
                    name.clone(),
                    format!("{c:.1} %"),
                    if *c >= 80.0 {
                        Tone::Bad
                    } else if *c >= 40.0 {
                        Tone::Warn
                    } else {
                        Tone::Normal
                    },
                )
            })
            .collect();
        if rows.is_empty() {
            rows.push((
                "Waiting for the first sample".to_string(),
                "-".to_string(),
                Tone::Dim,
            ));
        } else if v.over_s == 0.0 {
            rows.push((
                "CPU shows from the second sample".to_string(),
                String::new(),
                Tone::Dim,
            ));
        }
        if let Some(s) = ui.get_mut::<StatList>("threads") {
            s.set(rows);
        }
        let sys = v
            .rows()
            .into_iter()
            .map(|(k, val)| (k, val, Tone::Normal))
            .collect();
        if let Some(s) = ui.get_mut::<StatList>("system") {
            s.set(sys);
        }
    }

    /// The CPU by engine page, from the audio loop's published numbers.
    pub fn set_engines(ui: &mut Ui, s: &engines::Snapshot) {
        if let Some(l) = ui.get_mut::<StatList>("engnow") {
            l.set(engines::now_rows(s));
        }
        if let Some(l) = ui.get_mut::<StatList>("engsession") {
            l.set(engines::session_rows(s));
        }
    }

    /// The Processes page, from one /proc sample.
    pub fn set_procs(ui: &mut Ui, v: &procs::ProcView) {
        let (list, side) = procs_rows(v);
        if let Some(l) = ui.get_mut::<StatList>("procs") {
            l.set(list);
        }
        if let Some(l) = ui.get_mut::<StatList>("cores") {
            l.set(side);
        }
    }

    pub fn set_map(ui: &mut Ui, m: &MapView) {
        if let Some(l) = ui.get_mut::<ListView>("controls") {
            l.title = format!(
                "Controls: {} found, {} skipped, {} left",
                m.found, m.skipped, m.left
            );
            l.items = m
                .rows
                .iter()
                .enumerate()
                .map(|(i, r)| ListItem {
                    text: r.name.clone(),
                    detail: r.value.clone(),
                    color: if m.listening && i == m.selected {
                        Some(Color::from_pad([0x3F, 0x22, 0x00]))
                    } else {
                        found_colour(r.mark)
                    },
                })
                .collect();
            l.selected = m.selected;
        }
        if let Some(c) = ui.get_mut::<Card>("mapcard") {
            c.heading = m.heading.clone();
            c.lines = m
                .lines
                .iter()
                .enumerate()
                .map(|(i, l)| {
                    (
                        l.clone(),
                        if i == 0 && m.listening {
                            Tone::Accent
                        } else {
                            Tone::Normal
                        },
                    )
                })
                .collect();
            c.buttons[0] = if m.listening {
                "Stop listening"
            } else {
                "Listen"
            }
            .to_string();
        }
        if let Some(s) = ui.get_mut::<StatList>("mapinfo") {
            s.set(vec![
                ("Pressed last".to_string(), m.last.clone(), Tone::Accent),
                (
                    "Pads".to_string(),
                    "notes on channel 10".to_string(),
                    Tone::Dim,
                ),
                (
                    "Q-Links 1-4, turned".to_string(),
                    "CC 16 to 19".to_string(),
                    Tone::Dim,
                ),
                (
                    "Q-Links 1-4, touched".to_string(),
                    "notes 84 to 87".to_string(),
                    Tone::Dim,
                ),
                ("Data wheel".to_string(), "CC 100".to_string(), Tone::Dim),
            ]);
        }
    }
}

/// A StatList's rows: label, value, tone.
type Rows = Vec<(String, String, Tone)>;

/// How many processes the list shows (the busiest).
const PROCS_SHOWN: usize = 14;

/// The Processes page's rows: the busiest processes (name and pid, then
/// scheduling and threads; CPU and memory), and the machine (total and
/// per-core CPU, how many processes).
pub fn procs_rows(v: &procs::ProcView) -> (Rows, Rows) {
    let cpu_tone = |c: f64| {
        if c >= 80.0 {
            Tone::Bad
        } else if c >= 40.0 {
            Tone::Warn
        } else {
            Tone::Normal
        }
    };
    let mut list: Rows = v
        .rows
        .iter()
        .take(PROCS_SHOWN)
        .map(|r| {
            let mem = if r.rss_kb >= 1024 {
                format!("{:.0} MB", r.rss_kb as f64 / 1024.0)
            } else {
                format!("{} kB", r.rss_kb)
            };
            (
                format!("{} ({}) · {} · {} thr", r.name, r.pid, r.policy, r.threads),
                format!("{:.1} % · {mem}", r.cpu_pct),
                cpu_tone(r.cpu_pct),
            )
        })
        .collect();
    if list.is_empty() {
        list.push((
            "Reading /proc once a second while this page shows".to_string(),
            "-".to_string(),
            Tone::Dim,
        ));
    } else if v.over_s == 0.0 {
        list.push((
            "CPU shows from the second sample".to_string(),
            String::new(),
            Tone::Dim,
        ));
    }
    let mut side = vec![(
        "All cores".to_string(),
        v.total_cpu_pct
            .map_or("-".to_string(), |c| format!("{c:.1} %")),
        v.total_cpu_pct.map_or(Tone::Dim, cpu_tone),
    )];
    for (i, c) in v.core_cpu_pct.iter().enumerate() {
        side.push((format!("Core {i}"), format!("{c:.1} %"), cpu_tone(*c)));
    }
    let mem_kb: u64 = v.rows.iter().map(|r| r.rss_kb).sum();
    side.push((
        "Processes".to_string(),
        v.rows.len().to_string(),
        Tone::Normal,
    ));
    side.push((
        "Their memory in use (RSS, summed)".to_string(),
        format!("{:.0} MB", mem_kb as f64 / 1024.0),
        Tone::Normal,
    ));
    side.push((
        "Read from".to_string(),
        "/proc/<pid>/stat, names only".to_string(),
        Tone::Dim,
    ));
    (list, side)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_ui::{Fonts, Theme};

    #[test]
    fn the_pages_fit_and_the_graph_keeps_a_minute() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", mpfree_ui::widgets::Header::new(&["Play"]));
        ui.add(
            "hint",
            mpfree_ui::widgets::Label::new("", Tone::Dim, Align::Center),
        );
        for w in ["midi", "events", "timing", "syslog"] {
            ui.add(w, StatList::new(w));
        }
        let mut d = DebugPages::add(&mut ui).unwrap();
        ui.show(MAP_PAGE).unwrap();
        assert!(ui.rect_of("controls").unwrap().h > 560);
        ui.show(PERF_PAGE).unwrap();
        assert!(ui.rect_of("threads").unwrap().h > 300);
        ui.show(ENGINES_PAGE).unwrap();
        assert!(ui.rect_of("engnow").unwrap().h > 500);
        ui.show(PROCS_PAGE).unwrap();
        assert!(ui.rect_of("procs").unwrap().h > 500);
        for _ in 0..HISTORY + 10 {
            d.set_audio(
                &mut ui,
                AudioNumbers {
                    render_max_us: PERIOD_US / 2.0,
                    ..AudioNumbers::default()
                },
            );
        }
        let row = |pid: u32, name: &str, cpu: f64| procs::ProcRow {
            pid,
            name: name.into(),
            state: 'S',
            cpu_pct: cpu,
            rss_kb: 2048,
            threads: 3,
            policy: "FIFO 70".into(),
            nice: 0,
        };
        let (list, side) = procs_rows(&procs::ProcView {
            over_s: 1.0,
            rows: vec![row(9, "mpfree-hello", 85.0), row(1, "init", 0.0)],
            total_cpu_pct: Some(25.0),
            core_cpu_pct: vec![90.0, 10.0],
        });
        assert_eq!(
            list[0],
            (
                "mpfree-hello (9) · FIFO 70 · 3 thr".to_string(),
                "85.0 % · 2 MB".to_string(),
                Tone::Bad
            )
        );
        assert_eq!(
            side[2],
            ("Core 1".to_string(), "10.0 %".to_string(), Tone::Normal)
        );
        assert_eq!(side[3].1, "2");
        assert_eq!(procs_rows(&procs::ProcView::default()).0[0].2, Tone::Dim);
        let g = ui.get_mut::<Graph>("graph").unwrap();
        assert_eq!(g.values.len(), HISTORY);
        assert_eq!(g.right, "now 50 %");
    }
}
