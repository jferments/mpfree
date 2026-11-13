//! The screen drawn by the GPU in a process of its own (step G4 of the
//! performance plan).
//!
//! The app stays one static program and never touches the GPU's driver:
//! its screen thread paints each frame into a [`Recorder`] instead of the
//! software canvas, and sends the frame's display list (the wire format of
//! [`crate::record`]) down a pipe to the GPU process (`mpfree-gpu-screen`,
//! in `rust/mpfree-gpu`), which draws it straight into the panel's buffers.
//! A crash or a hang in the GPU's driver takes down that process, never the
//! sound, and the app sees it here: the process is gone, says it failed, or
//! stops answering. The app then stops it and draws in software again (a
//! fallback: counted, logged and shown).
//!
//! The protocol, one line per answer on the process's stdout:
//!
//! - `ready NWxNH LWxLH WHAT`: set up, the panel's size (native, then as the
//!   screens draw it) and how it shows frames (`kms ...` or `offscreen`);
//! - `shown SEQ DRAW_US GPU_US WAIT_US PLAIN SHADED COPIED`: frame SEQ
//!   (counted from 1) is on its way to the panel, with what it cost;
//! - `same SEQ`: frame SEQ drew nothing, so nothing was shown;
//! - `fail: REASON`: it stopped (the screen is put back as it was);
//! - anything else is a note for the log.
//!
//! Every frame is answered, so the frames in flight are always known. A
//! frame is never dropped: lists only redraw what changed, so a lost one
//! would leave its change off the panel for good. When the process falls
//! [`MAX_IN_FLIGHT`] frames behind, sending waits for it, and after
//! [`STUCK_AFTER`] without an answer it counts as stuck.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::thread;
use std::time::{Duration, Instant};

use crate::record::Recorder;

/// Frames sent and not yet answered, at most, before sending waits.
pub const MAX_IN_FLIGHT: u64 = 3;

/// How long the process may go without answering a frame (or, at start,
/// without saying it is ready) before it counts as stuck.
pub const STUCK_AFTER: Duration = Duration::from_millis(1500);

/// How long [`GpuScreen::stop`] waits for the process to put the screen back
/// and exit, after closing its input, before it kills it.
pub const STOP_GRACE: Duration = Duration::from_millis(800);

/// One answer from the process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Ready {
        native: (i32, i32),
        logical: (i32, i32),
        how: String,
    },
    Shown {
        seq: u64,
        draw_us: u64,
        gpu_us: u64,
        wait_us: u64,
        plain: u64,
        shaded: u64,
        copied: u64,
    },
    Same {
        seq: u64,
    },
    Failed(String),
    Note(String),
}

fn size(s: &str) -> Option<(i32, i32)> {
    let (w, h) = s.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

/// Reads one line of the process's answers.
pub fn parse_reply(line: &str) -> Reply {
    let line = line.trim_end();
    if let Some(reason) = line.strip_prefix("fail:") {
        return Reply::Failed(reason.trim().to_string());
    }
    let mut it = line.split(' ');
    let parsed = match it.next() {
        Some("ready") => (|| {
            let native = size(it.next()?)?;
            let logical = size(it.next()?)?;
            let how = it.collect::<Vec<_>>().join(" ");
            Some(Reply::Ready {
                native,
                logical,
                how,
            })
        })(),
        Some("shown") => (|| {
            let mut n = || it.next()?.parse::<u64>().ok();
            Some(Reply::Shown {
                seq: n()?,
                draw_us: n()?,
                gpu_us: n()?,
                wait_us: n()?,
                plain: n()?,
                shaded: n()?,
                copied: n()?,
            })
        })(),
        Some("same") => it
            .next()
            .and_then(|s| s.parse().ok())
            .map(|seq| Reply::Same { seq }),
        _ => None,
    };
    parsed.unwrap_or_else(|| Reply::Note(line.to_string()))
}

/// What the GPU process has done so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuStats {
    pub sent: u64,
    pub shown: u64,
    pub same: u64,
    /// Bytes of display lists sent.
    pub bytes: u64,
    /// The last shown frame's costs, in microseconds.
    pub draw_us: u64,
    pub gpu_us: u64,
    pub wait_us: u64,
    /// The slowest frame's GPU time, in microseconds.
    pub gpu_us_max: u64,
    /// The slowest frame (building it plus the GPU's time, in
    /// microseconds) since the app last set this to 0.
    pub slowest_us: u64,
}

/// The GPU process, running.
pub struct GpuScreen {
    child: Option<Child>,
    frames: Option<SyncSender<Vec<u8>>>,
    replies: Receiver<Option<Reply>>,
    recorder: Recorder,
    buf: Vec<u8>,
    acked: u64,
    /// When the oldest frame not yet answered was sent.
    waiting_since: Option<Instant>,
    pub how: String,
    pub native: (i32, i32),
    pub stats: GpuStats,
    /// Notes the process printed, for the app to log.
    pub notes: Vec<String>,
}

impl GpuScreen {
    /// Starts `program` with `args` and waits for it to say it is ready, for
    /// screens `w` x `h`. Fails (with the reason, naming the program) if it
    /// can't start, says it failed, draws at another size, or says nothing
    /// for [`STUCK_AFTER`] (plus `start_grace`, for setting up the driver).
    /// A program that isn't there fails at the spawn, or, where the spawn
    /// can't report it (under qemu-user), as a process that ended with
    /// status 127 before it was ready.
    pub fn start(
        program: &Path,
        args: &[String],
        w: i32,
        h: i32,
        start_grace: Duration,
    ) -> Result<GpuScreen, String> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("{}: {e}", program.display()))?;
        let name = program.display();
        let stdin = child.stdin.take().ok_or("no pipe to the GPU process")?;
        let stdout = child.stdout.take().ok_or("no pipe from the GPU process")?;
        let (frames, replies) = match pipes(stdin, stdout) {
            Ok(p) => p,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let mut g = GpuScreen {
            child: Some(child),
            frames: Some(frames),
            replies,
            recorder: Recorder::new(w, h),
            buf: Vec::new(),
            acked: 0,
            waiting_since: None,
            how: String::new(),
            native: (0, 0),
            stats: GpuStats::default(),
            notes: Vec::new(),
        };
        let until = Instant::now() + STUCK_AFTER + start_grace;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            let reply = match g.replies.recv_timeout(left) {
                Ok(Some(r)) => r,
                Ok(None) | Err(RecvTimeoutError::Disconnected) => {
                    let status = g.reap();
                    return Err(format!(
                        "{name}: the GPU process ended before it was ready ({status})"
                    ));
                }
                Err(RecvTimeoutError::Timeout) => {
                    g.stop();
                    return Err(format!(
                        "{name}: the GPU process was not ready after {:.1} s",
                        (STUCK_AFTER + start_grace).as_secs_f64()
                    ));
                }
            };
            match reply {
                Reply::Ready {
                    native,
                    logical,
                    how,
                } => {
                    if logical != (w, h) {
                        g.stop();
                        return Err(format!(
                            "{name}: the GPU process draws {}x{}; the screens are {w}x{h}",
                            logical.0, logical.1
                        ));
                    }
                    g.native = native;
                    g.how = how;
                    return Ok(g);
                }
                Reply::Failed(e) => {
                    let status = g.reap();
                    return Err(format!("{name}: {e} ({status})"));
                }
                Reply::Note(n) => g.notes.push(n),
                other => g.notes.push(format!("before ready: {other:?}")),
            }
        }
    }

    /// The canvas the next frame is painted into: call it once a frame,
    /// before painting (it starts the frame's list).
    pub fn canvas(&mut self) -> &mut Recorder {
        self.recorder.begin_frame();
        &mut self.recorder
    }

    /// Sends the frame painted since [`GpuScreen::canvas`]. Waits while the
    /// process is [`MAX_IN_FLIGHT`] frames behind; fails if it has gone,
    /// failed or got stuck (the frame is then not drawn: draw everything
    /// again in software).
    pub fn send(&mut self) -> Result<(), String> {
        self.poll()?;
        while self.stats.sent - self.acked >= MAX_IN_FLIGHT {
            self.wait_reply()?;
        }
        self.buf.clear();
        self.recorder.encode_frame(&mut self.buf);
        let bytes = std::mem::take(&mut self.buf);
        let n = bytes.len() as u64;
        let frames = self.frames.as_ref().ok_or("the GPU process was stopped")?;
        let mut bytes = Some(bytes);
        let until = Instant::now() + STUCK_AFTER;
        loop {
            match frames.try_send(bytes.take().unwrap_or_default()) {
                Ok(()) => break,
                Err(TrySendError::Full(b)) => {
                    if Instant::now() >= until {
                        return Err(format!(
                            "the GPU process took no frame for {:.1} s",
                            STUCK_AFTER.as_secs_f64()
                        ));
                    }
                    bytes = Some(b);
                    thread::sleep(Duration::from_millis(2));
                }
                Err(TrySendError::Disconnected(_)) => {
                    let status = self.reap();
                    return Err(format!("the GPU process stopped taking frames ({status})"));
                }
            }
        }
        self.stats.sent += 1;
        self.stats.bytes += n;
        if self.waiting_since.is_none() {
            self.waiting_since = Some(Instant::now());
        }
        Ok(())
    }

    /// Takes the answers that have come, without waiting. Fails if the
    /// process has gone or failed, or has left a frame unanswered for
    /// [`STUCK_AFTER`].
    pub fn poll(&mut self) -> Result<(), String> {
        loop {
            match self.replies.try_recv() {
                Ok(Some(r)) => self.take(r)?,
                Ok(None) | Err(mpsc::TryRecvError::Disconnected) => {
                    let status = self.reap();
                    return Err(format!("the GPU process ended ({status})"));
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        if let Some(t) = self.waiting_since {
            if t.elapsed() >= STUCK_AFTER {
                return Err(format!(
                    "the GPU process left frame {} unanswered for {:.1} s",
                    self.acked + 1,
                    STUCK_AFTER.as_secs_f64()
                ));
            }
        }
        Ok(())
    }

    /// Waits for one answer (or until the oldest frame counts as stuck).
    fn wait_reply(&mut self) -> Result<(), String> {
        let since = self.waiting_since.unwrap_or_else(Instant::now);
        let left = STUCK_AFTER.saturating_sub(since.elapsed());
        match self.replies.recv_timeout(left) {
            Ok(Some(r)) => self.take(r),
            Ok(None) | Err(RecvTimeoutError::Disconnected) => {
                let status = self.reap();
                Err(format!("the GPU process ended ({status})"))
            }
            Err(RecvTimeoutError::Timeout) => Err(format!(
                "the GPU process left frame {} unanswered for {:.1} s",
                self.acked + 1,
                STUCK_AFTER.as_secs_f64()
            )),
        }
    }

    fn take(&mut self, r: Reply) -> Result<(), String> {
        let seq = match r {
            Reply::Shown {
                seq,
                draw_us,
                gpu_us,
                wait_us,
                ..
            } => {
                self.stats.shown += 1;
                self.stats.draw_us = draw_us;
                self.stats.gpu_us = gpu_us;
                self.stats.wait_us = wait_us;
                self.stats.gpu_us_max = self.stats.gpu_us_max.max(gpu_us);
                self.stats.slowest_us = self.stats.slowest_us.max(draw_us + gpu_us);
                seq
            }
            Reply::Same { seq } => {
                self.stats.same += 1;
                seq
            }
            Reply::Failed(e) => {
                let status = self.reap();
                return Err(format!("{e} ({status})"));
            }
            Reply::Note(n) => {
                self.notes.push(n);
                return Ok(());
            }
            Reply::Ready { .. } => {
                self.notes.push("ready again (ignored)".into());
                return Ok(());
            }
        };
        if seq != self.acked + 1 {
            return Err(format!(
                "the GPU process answered frame {seq}; frame {} was next",
                self.acked + 1
            ));
        }
        self.acked = seq;
        self.waiting_since = (self.acked < self.stats.sent).then(Instant::now);
        Ok(())
    }

    /// Frames sent and not yet answered.
    pub fn in_flight(&self) -> u64 {
        self.stats.sent - self.acked
    }

    /// Waits for the process to end (it has ended or is ending); its status.
    fn reap(&mut self) -> String {
        self.frames = None;
        let Some(mut child) = self.child.take() else {
            return "already stopped".into();
        };
        let until = Instant::now() + STOP_GRACE;
        loop {
            match child.try_wait() {
                Ok(Some(s)) => return s.to_string(),
                Ok(None) if Instant::now() < until => thread::sleep(Duration::from_millis(10)),
                _ => return kill(child),
            }
        }
    }

    /// Stops the process: closes its input, so it puts the screen back and
    /// exits, and kills it if it has not after [`STOP_GRACE`]. Its status.
    pub fn stop(&mut self) -> String {
        self.reap()
    }
}

/// Kills `child` and waits a little for it to end. A process stuck inside
/// the driver can't end until the kernel lets it, so after [`STOP_GRACE`]
/// it is left to a thread of its own to wait for (and the screen thread
/// carries on).
fn kill(mut child: Child) -> String {
    let _ = child.kill();
    let until = Instant::now() + STOP_GRACE;
    loop {
        match child.try_wait() {
            Ok(Some(s)) => return format!("{s}, killed"),
            Ok(None) if Instant::now() < until => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let pid = child.id();
                let _ = thread::Builder::new()
                    .name("mpfree-gpu-reap".into())
                    .spawn(move || child.wait());
                return format!("killed; process {pid} has not ended yet");
            }
            Err(e) => return format!("killed ({e})"),
        }
    }
}

impl Drop for GpuScreen {
    fn drop(&mut self) {
        self.reap();
    }
}

/// The writer thread (frames to the process's stdin; it blocks, the screen
/// thread doesn't) and the reader thread (answers from its stdout; `None`
/// when it closes).
/// Frames to the writer thread; answers from the reader thread.
type Pipes = (SyncSender<Vec<u8>>, Receiver<Option<Reply>>);

fn pipes(mut stdin: ChildStdin, stdout: ChildStdout) -> Result<Pipes, String> {
    let (frames, to_write) = mpsc::sync_channel::<Vec<u8>>(2);
    thread::Builder::new()
        .name("mpfree-gpu-tx".into())
        .spawn(move || {
            for frame in to_write {
                if stdin.write_all(&frame).and_then(|_| stdin.flush()).is_err() {
                    break;
                }
            }
            // Dropping stdin closes the pipe: the process puts the screen
            // back and exits.
        })
        .map_err(|e| format!("starting the GPU writer thread: {e}"))?;
    let (answers, replies) = mpsc::channel();
    thread::Builder::new()
        .name("mpfree-gpu-rx".into())
        .spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if answers.send(Some(parse_reply(&line))).is_err() {
                    return;
                }
            }
            let _ = answers.send(None);
        })
        .map_err(|e| format!("starting the GPU reader thread: {e}"))?;
    Ok((frames, replies))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Canvas, Color, Rect};
    use std::path::PathBuf;

    fn sh() -> PathBuf {
        PathBuf::from("/bin/sh")
    }

    fn script(s: &str) -> Vec<String> {
        vec!["-c".into(), s.into()]
    }

    #[test]
    fn answers_are_read() {
        assert_eq!(
            parse_reply("ready 800x1280 1280x800 kms /dev/dri/card0 crtc 31"),
            Reply::Ready {
                native: (800, 1280),
                logical: (1280, 800),
                how: "kms /dev/dri/card0 crtc 31".into()
            }
        );
        assert_eq!(
            parse_reply("shown 3 120 900 4 1000 20 30\n"),
            Reply::Shown {
                seq: 3,
                draw_us: 120,
                gpu_us: 900,
                wait_us: 4,
                plain: 1000,
                shaded: 20,
                copied: 30
            }
        );
        assert_eq!(parse_reply("same 4"), Reply::Same { seq: 4 });
        assert_eq!(
            parse_reply("fail: no screen is lit"),
            Reply::Failed("no screen is lit".into())
        );
        assert_eq!(parse_reply("shown x"), Reply::Note("shown x".into()));
    }

    #[test]
    fn a_program_that_is_not_there_is_an_error() {
        let e = GpuScreen::start(
            Path::new("/nonexistent/mpfree-gpu-screen"),
            &[],
            1280,
            800,
            Duration::ZERO,
        )
        .err()
        .unwrap();
        // The spawn's error, or (under qemu-user, which can't report it) the
        // process's status 127: either way it names the program.
        assert!(e.starts_with("/nonexistent/mpfree-gpu-screen: "), "{e}");
    }

    #[test]
    fn a_process_that_fails_at_start_says_why() {
        let e = GpuScreen::start(
            &sh(),
            &script("echo 'fail: no GPU here'; exit 1"),
            1280,
            800,
            Duration::ZERO,
        )
        .err()
        .unwrap();
        assert!(e.starts_with("/bin/sh: no GPU here"), "{e}");
    }

    #[test]
    fn a_process_at_another_size_is_refused() {
        let e = GpuScreen::start(
            &sh(),
            &script("echo ready 600x400 400x600 offscreen; cat >/dev/null"),
            1280,
            800,
            Duration::ZERO,
        )
        .err()
        .unwrap();
        assert!(e.contains("400x600"), "{e}");
    }

    #[test]
    fn a_process_that_never_answers_counts_as_stuck() {
        // Ready, then it reads frames but never answers one.
        let mut g = GpuScreen::start(
            &sh(),
            &script("echo ready 800x1280 1280x800 fake; cat >/dev/null"),
            1280,
            800,
            Duration::ZERO,
        )
        .unwrap();
        assert_eq!(g.how, "fake");
        let started = Instant::now();
        let mut failed = None;
        for _ in 0..1000 {
            let c = g.canvas();
            c.fill_rect(Rect::new(0, 0, 10, 10), Color::rgb(0xFF0000));
            if let Err(e) = g.send() {
                failed = Some(e);
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let e = failed.expect("stuck");
        assert!(e.contains("unanswered"), "{e}");
        assert!(started.elapsed() >= STUCK_AFTER, "{:?}", started.elapsed());
        assert!(
            started.elapsed() < STUCK_AFTER * 3,
            "{:?}",
            started.elapsed()
        );
        let status = g.stop();
        assert!(!status.is_empty());
    }

    #[test]
    fn a_process_that_dies_is_noticed() {
        // Ready, then it takes one frame's first bytes and exits.
        let mut g = GpuScreen::start(
            &sh(),
            &script("echo ready 800x1280 1280x800 fake; head -c 4 >/dev/null; exit 7"),
            1280,
            800,
            Duration::ZERO,
        )
        .unwrap();
        let mut failed = None;
        for _ in 0..200 {
            let c = g.canvas();
            c.fill_rect(Rect::new(0, 0, 10, 10), Color::rgb(0xFF0000));
            if let Err(e) = g.send() {
                failed = Some(e);
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let e = failed.expect("noticed");
        assert!(e.contains("ended") || e.contains("stopped taking"), "{e}");
    }

    #[test]
    fn answers_out_of_order_are_an_error() {
        let mut g = GpuScreen::start(
            &sh(),
            &script("echo ready 800x1280 1280x800 fake; echo shown 2 1 1 1 1 1 1; cat >/dev/null"),
            1280,
            800,
            Duration::ZERO,
        )
        .unwrap();
        let mut failed = None;
        for _ in 0..100 {
            g.canvas()
                .fill_rect(Rect::new(0, 0, 4, 4), Color::rgb(0xFFFFFF));
            if let Err(e) = g.send() {
                failed = Some(e);
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let e = failed.expect("refused");
        assert!(e.contains("answered frame 2"), "{e}");
    }
}
