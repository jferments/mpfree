//! How the screen thread draws a frame: on the GPU, in a process of its own
//! (`mpfree app --gpu-screen PROGRAM`, step G4 of the performance plan),
//! while that works; in software into the framebuffer otherwise.
//!
//! The GPU process (`mpfree-gpu-screen`, in `rust/mpfree-gpu`) takes each
//! frame's display list down a pipe and draws it straight into the panel's
//! buffers (`mpfree_ui::gpu_screen` has the protocol and the client). A
//! crash or hang in the GPU's driver ends that process, never the app or
//! its sound. When the process can't start, says it failed, ends, or stops
//! answering, the app stops it (it puts the screen back as it found it, or
//! the kernel does if it was killed), shows the framebuffer again, and
//! paints the whole screen in software from then on. That fallback is a
//! safety net, never a fix: each one is counted, logged (stdout and the
//! Events log), and shown on the Timing panel's Screen row.
//!
//! The app does not try the GPU again after a fallback: a driver that
//! failed once may fail again, and each try takes the screen and gives it
//! back. Starting the app again tries again.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mpfree_screen_log::Rotation;
use mpfree_ui::fb::FbTarget;
use mpfree_ui::gpu_screen::GpuScreen;
use mpfree_ui::{SoftCanvas, Tone};

use crate::gui::{Player, ToUi};
use crate::screen_cost::Second;

/// How long the GPU process may take to set up (open the driver, compile
/// its shaders, check its test card) before it counts as stuck, on top of
/// the client's own wait.
pub const START_GRACE: Duration = Duration::from_secs(5);

/// The GPU process to start, and how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuStart {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub grace: Duration,
}

impl GpuStart {
    /// `program` on the panel (KMS), turned as the framebuffer is.
    pub fn new(program: &Path, rotation: Rotation) -> GpuStart {
        GpuStart {
            program: program.to_path_buf(),
            args: vec!["--rotation".into(), rotation.degrees().to_string()],
            grace: START_GRACE,
        }
    }
}

/// The screen thread's way of drawing, and the GPU's fallbacks.
pub struct Drawing {
    gpu: Option<GpuScreen>,
    /// The GPU was asked for (so software drawing is a fallback).
    asked: bool,
    /// Times the GPU stopped and software took over.
    pub fallbacks: u32,
    /// Why it last stopped.
    pub reason: Option<String>,
}

impl Drawing {
    /// Software drawing only.
    pub fn software() -> Drawing {
        Drawing {
            gpu: None,
            asked: false,
            fallbacks: 0,
            reason: None,
        }
    }

    /// Starts the GPU process when asked for (and waits until it is ready);
    /// if it can't, falls back to software at once. Either way, says how
    /// the screen is drawn.
    pub fn start(start: Option<&GpuStart>, player: &mut Player, fb: Option<&FbTarget>) -> Drawing {
        let mut d = Drawing::software();
        let Some(s) = start else {
            return d;
        };
        d.asked = true;
        let (w, h) = player.ui.size();
        match GpuScreen::start(&s.program, &s.args, w, h, s.grace) {
            Ok(g) => {
                let line = format!(
                    "screen: drawn by the GPU in a process of its own ({}; {}, {}x{} native)",
                    s.program.display(),
                    g.how,
                    g.native.0,
                    g.native.1
                );
                say(player, line);
                d.gpu = Some(g);
                d.notes(player);
            }
            Err(e) => d.fall_back(&format!("it did not start: {e}"), player, fb),
        }
        d
    }

    /// Takes the GPU process's answers between frames, so a process that
    /// has ended or stuck is noticed (and software takes over) at once.
    pub fn poll(&mut self, player: &mut Player, fb: Option<&FbTarget>) {
        let Some(g) = self.gpu.as_mut() else {
            return;
        };
        let r = g.poll();
        self.notes(player);
        if let Err(e) = r {
            self.fall_back(&e, player, fb);
        }
    }

    /// Paints the frame and sends it to the GPU while it draws; how long
    /// the painting took (the rest was sending). None when the frame is to
    /// be painted in software instead: there is no GPU, or it has just
    /// fallen back (the whole screen is then painted again).
    pub fn frame(&mut self, player: &mut Player, fb: Option<&FbTarget>) -> Option<Duration> {
        let g = self.gpu.as_mut()?;
        // The list carries everything the frame changed; the GPU process
        // keeps the picture, so damage is not needed here.
        let t = Instant::now();
        player.ui.paint(g.canvas());
        let painted = t.elapsed();
        let r = g.send();
        self.notes(player);
        match r {
            Ok(()) => Some(painted),
            Err(e) => {
                self.fall_back(&e, player, fb);
                None
            }
        }
    }

    /// Before the screen thread ends: while the GPU draws, stops the GPU
    /// process (it puts the screen back to the framebuffer) and paints the
    /// last frame into the framebuffer, so the last picture stays up as it
    /// does with software drawing.
    pub fn finish(
        &mut self,
        player: &mut Player,
        canvas: &mut SoftCanvas,
        fb: Option<&mut FbTarget>,
    ) {
        let Some(mut g) = self.gpu.take() else {
            return;
        };
        // Painted first, so the framebuffer's picture is ready the moment
        // the screen is given back (whether the kernel shows writes made
        // while the GPU process held the screen is for the device round to
        // see), and copied again after.
        player.ui.invalidate();
        let damage = player.ui.paint(canvas);
        let status = g.stop();
        drain_notes(&mut g, player);
        println!("{}", summary(&g, &status));
        if let Some(f) = fb {
            f.show();
            if let Err(e) = f.present(canvas, &damage) {
                println!("screen: {e} (the last frame is not on the screen)");
            }
        }
    }

    /// The Timing panel's Screen row for the last second `s`, and its
    /// colour. In software it is [`Second::row`]. While the GPU draws:
    /// fps, the mean time to paint the list and to send it, and the GPU
    /// process's slowest frame (building and drawing it). After a fallback,
    /// in the warning colour: software, and how many times the GPU
    /// stopped. Starts the GPU's next second.
    pub fn screen_row(&mut self, s: &Second) -> (String, Tone) {
        match self.gpu.as_mut() {
            Some(g) => {
                let gpu_ms = g.stats.slowest_us as f64 / 1e3;
                g.stats.slowest_us = 0;
                (
                    format!(
                        "GPU · {:.0} fps · paint {:.1} · send {:.1} · GPU {gpu_ms:.1} ms",
                        s.fps, s.paint_ms, s.copy_ms
                    ),
                    Tone::Dim,
                )
            }
            None if self.asked => (
                format!(
                    "software, GPU off {}× · {:.0} fps · slowest {:.1} ms",
                    self.fallbacks, s.fps, s.slowest_ms
                ),
                Tone::Warn,
            ),
            None => (s.row(), Tone::Dim),
        }
    }

    /// Stops the GPU process and draws in software from now on: counted,
    /// logged and shown.
    fn fall_back(&mut self, why: &str, player: &mut Player, fb: Option<&FbTarget>) {
        let status = match self.gpu.take() {
            Some(mut g) => {
                let status = g.stop();
                drain_notes(&mut g, player);
                status
            }
            None => String::new(),
        };
        self.fallbacks += 1;
        let ended = if status.is_empty() || status == "already stopped" {
            String::new()
        } else {
            format!(" (the GPU process: {status})")
        };
        say(
            player,
            format!(
                "screen: GPU drawing stopped: {why}{ended}; drawing in software (GPU fallback {})",
                self.fallbacks
            ),
        );
        self.reason = Some(why.to_string());
        // The GPU process has put the CRTC back on the framebuffer (or the
        // kernel has, as its last file closed); ask the kernel to show it
        // too, then paint every pixel again.
        if let Some(f) = fb {
            f.show();
        }
        player.ui.invalidate();
    }

    fn notes(&mut self, player: &mut Player) {
        if let Some(g) = self.gpu.as_mut() {
            drain_notes(g, player);
        }
    }
}

/// The GPU process's notes, logged.
fn drain_notes(g: &mut GpuScreen, player: &mut Player) {
    for n in std::mem::take(&mut g.notes) {
        say(player, format!("gpu screen: {n}"));
    }
}

/// A line on stdout and in the Events log.
fn say(player: &mut Player, line: String) {
    println!("{line}");
    player.apply(ToUi::Line(line));
}

/// What the GPU process did, for the log when it stops.
fn summary(g: &GpuScreen, status: &str) -> String {
    let s = &g.stats;
    format!(
        "screen: the GPU drew {} frames ({} unchanged), {} KB of lists, slowest {:.1} ms on the GPU; \
         the GPU process: {status}",
        s.shown,
        s.same,
        s.bytes / 1024,
        s.gpu_us_max as f64 / 1e3
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::SCREEN;
    use mpfree_ui::widgets::LogPanel;
    use mpfree_ui::{FontId, Fonts, Rect};
    use std::time::Instant;

    fn player() -> Player {
        Player::new(Fonts::embedded(), SCREEN, false, None)
    }

    /// A stand-in for the GPU process: `/bin/sh -c script`.
    fn fake(script: &str) -> GpuStart {
        GpuStart {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), script.into()],
            grace: Duration::ZERO,
        }
    }

    const SECOND: Second = Second {
        fps: 30.0,
        paint_ms: 1.2,
        copy_ms: 3.1,
        slowest_ms: 6.4,
    };

    fn events(p: &Player) -> Vec<String> {
        p.ui.get::<LogPanel>("events")
            .map(|l| l.lines().map(|(t, _)| t.clone()).collect())
            .unwrap_or_default()
    }

    /// Polls until the GPU has fallen back (or `within` passes).
    fn until_fallen_back(d: &mut Drawing, p: &mut Player, within: Duration) {
        let t = Instant::now();
        while d.gpu.is_some() && t.elapsed() < within {
            d.poll(p, None);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn without_the_gpu_the_software_draws_as_before() {
        let mut p = player();
        let mut d = Drawing::start(None, &mut p, None);
        assert!(d.frame(&mut p, None).is_none(), "software paints the frame");
        assert_eq!(d.fallbacks, 0);
        assert_eq!(d.screen_row(&SECOND), (SECOND.row(), Tone::Dim));
        assert!(events(&p).iter().all(|l| !l.contains("GPU")));
    }

    #[test]
    fn the_rotation_goes_to_the_gpu_process() {
        let s = GpuStart::new(
            Path::new("/run/mpfree-app/mpfree-gpu-screen"),
            Rotation::Deg270,
        );
        assert_eq!(s.args, ["--rotation", "270"]);
        assert_eq!(s.grace, START_GRACE);
    }

    #[test]
    fn a_gpu_that_does_not_start_is_a_fallback_counted_logged_and_shown() {
        let mut p = player();
        let start = fake("echo 'fail: no screen is lit'; exit 1");
        let mut d = Drawing::start(Some(&start), &mut p, None);
        assert_eq!(d.fallbacks, 1);
        assert!(d.reason.as_deref().unwrap().contains("no screen is lit"));
        let line = events(&p)
            .into_iter()
            .find(|l| l.contains("GPU fallback 1"));
        let line = line.expect("logged");
        assert!(line.contains("it did not start"), "{line}");
        assert!(d.frame(&mut p, None).is_none(), "software paints the frame");
        let (row, tone) = d.screen_row(&SECOND);
        assert!(row.starts_with("software, GPU off 1×"), "{row}");
        assert_eq!(tone, Tone::Warn);
    }

    #[test]
    fn a_gpu_that_fails_falls_back_and_the_whole_screen_is_painted_again() {
        let mut p = player();
        // Ready; then it fails as soon as a frame comes.
        let start = fake(
            "echo ready 800x1280 1280x800 fake; head -c 1 >/dev/null; \
             echo 'fail: the driver fell over'; exit 1",
        );
        let mut d = Drawing::start(Some(&start), &mut p, None);
        assert_eq!(d.fallbacks, 0);
        let (row, tone) = d.screen_row(&SECOND);
        assert!(
            row.starts_with("GPU · 30 fps · paint 1.2 · send 3.1 · GPU "),
            "{row}"
        );
        assert_eq!(tone, Tone::Dim);
        assert!(
            events(&p).iter().any(|l| l.contains("drawn by the GPU")),
            "{:?}",
            events(&p)
        );
        // The first frame goes to the GPU, which then fails.
        assert!(d.frame(&mut p, None).is_some());
        assert!(!p.ui.needs_paint(), "the GPU took the frame");
        until_fallen_back(&mut d, &mut p, Duration::from_secs(5));
        assert_eq!(d.fallbacks, 1);
        assert!(d
            .reason
            .as_deref()
            .unwrap()
            .contains("the driver fell over"));
        let line = events(&p)
            .into_iter()
            .find(|l| l.contains("GPU fallback 1"));
        assert!(line.expect("logged").contains("exit status: 1"));
        // Software takes over: every pixel is painted again.
        assert!(p.ui.needs_paint());
        assert!(d.frame(&mut p, None).is_none());
        let mut canvas = SoftCanvas::new(SCREEN.0, SCREEN.1);
        let damage = p.ui.paint(&mut canvas);
        let all = damage
            .iter()
            .fold(Rect::new(0, 0, 0, 0), |a, r| a.union(*r));
        assert_eq!(all, Rect::new(0, 0, SCREEN.0, SCREEN.1));
        let (row, tone) = d.screen_row(&SECOND);
        assert!(row.starts_with("software, GPU off 1×"), "{row}");
        assert_eq!(tone, Tone::Warn);
        // And it stays in software: the GPU is not tried again.
        d.poll(&mut p, None);
        assert!(d.frame(&mut p, None).is_none());
        assert_eq!(d.fallbacks, 1);
    }

    #[test]
    fn a_gpu_that_hangs_is_stopped_and_software_takes_over() {
        let mut p = player();
        // Ready; then it takes frames and never answers one.
        let start = fake("echo ready 800x1280 1280x800 fake; cat >/dev/null");
        let mut d = Drawing::start(Some(&start), &mut p, None);
        assert!(d.frame(&mut p, None).is_some());
        let t = Instant::now();
        until_fallen_back(&mut d, &mut p, Duration::from_secs(10));
        assert_eq!(d.fallbacks, 1);
        assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
        assert!(d.reason.as_deref().unwrap().contains("unanswered"));
        assert!(p.ui.needs_paint());
    }

    #[test]
    fn the_screen_row_fits_the_timing_panel() {
        let mut p = player();
        p.apply(ToUi::Show(crate::app::View::Debug));
        let r = p.ui.rect_of("timing").unwrap();
        let theme = mpfree_ui::Theme::dark();
        let mut fonts = Fonts::embedded();
        let key = fonts.measure(FontId::Sans, theme.size_small, "Screen") + 16;
        let room = r.w - 2 * theme.pad - key;
        let worst = Second {
            fps: 30.0,
            paint_ms: 12.3,
            copy_ms: 13.1,
            slowest_ms: 26.4,
        };
        let mut d = Drawing::software();
        d.asked = true;
        d.fallbacks = 1;
        for row in [
            d.screen_row(&worst).0,
            "GPU · 30 fps · paint 12.3 · send 1.1 · GPU 23.4 ms".to_string(),
        ] {
            let w = fonts.measure(FontId::Bold, theme.size_small, &row);
            // A tenth spare, for a wider font than the built-in one.
            assert!(w * 11 / 10 <= room, "{row}: {w} px of {room}");
        }
    }
}
