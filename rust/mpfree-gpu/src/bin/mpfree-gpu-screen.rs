//! mpfree-gpu-screen: draws the app's screen on the GPU, in a process of its
//! own (step G4 of the performance plan), straight into the buffers the
//! panel shows.
//!
//!   mpfree-gpu-screen [--rotation DEG] [--card PATH]
//!   mpfree-gpu-screen --offscreen [--size WxH] [--rotation DEG] [--last-frame FILE]
//!
//! The app (`mpfree --gpu-screen PROGRAM`) starts it and sends each frame's
//! display list down its stdin, in the wire format of `mpfree_ui::record`;
//! it answers each frame with one line on stdout (`mpfree_ui::gpu_screen`
//! has the protocol). A crash or a hang in the GPU's driver ends this
//! process, not the app's sound, and the app draws in software again.
//!
//! With `--card` (default `/dev/dri/card0`) it shows frames through KMS:
//! three buffers at the lit screen's size, drawn by the GPU, and page flips
//! (see `mpfree_gpu::kms`). Before anything is shown it draws a test card
//! into a screen buffer and reads that buffer's memory back, and stops if
//! it is not exactly the picture expected. The first frame changes the
//! screen (in RAM): it takes the card's master and points the lit CRTC at
//! its own buffers, in the mode the CRTC already runs. When its input
//! closes, on SIGINT, SIGTERM or SIGHUP, and on any failure, it sets the
//! CRTC back as it found it (the framebuffer console's picture) before it
//! exits. If it is killed, the kernel frees its buffers and the framebuffer
//! console takes the screen back when the card's last file closes; a power
//! cycle undoes everything.
//!
//! `--offscreen` draws into the GPU's own memory and shows nothing, for
//! tests and dry runs; `--last-frame FILE` then writes the last frame
//! shown (native rows, XRGB8888, little-endian) when the input closes.
//! `--test-fail-after N` and `--test-stall-after N` fail or hang after N
//! frames, for testing the app's fallback.

use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::time::Duration;

use mpfree_gpu::kms::{KmsDisplay, XRGB8888};
use mpfree_gpu::scanout::{self, Display, Imported, Offscreen, Scanout};
use mpfree_gpu::{Gpu, Renderer, Target};
use mpfree_screen_log::Rotation;
use mpfree_ui::record::{Frame, Store};

/// A frame's header in the wire format: magic, version, spare, w, h, length.
const HEADER: usize = 20;
/// The most bytes one frame may say it has (the busiest first frame of the
/// sample pages is about 0.5 MB).
const MAX_FRAME: usize = 64 << 20;

struct Options {
    rotation: Rotation,
    card: PathBuf,
    offscreen: bool,
    size: (i32, i32),
    last_frame: Option<PathBuf>,
    fail_after: Option<u64>,
    stall_after: Option<u64>,
}

fn options(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        rotation: Rotation::Deg270,
        card: PathBuf::from("/dev/dri/card0"),
        offscreen: false,
        size: (800, 1280),
        last_frame: None,
        fail_after: None,
        stall_after: None,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = || it.next().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--rotation" => {
                let v = value()?;
                o.rotation = v
                    .parse()
                    .ok()
                    .and_then(Rotation::from_degrees)
                    .ok_or(format!("--rotation {v}: 0, 90, 180 or 270"))?;
            }
            "--card" => o.card = value()?.into(),
            "--offscreen" => o.offscreen = true,
            "--size" => {
                let v = value()?;
                o.size = v
                    .split_once('x')
                    .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                    .filter(|&(w, h): &(i32, i32)| w > 0 && h > 0 && w <= 4096 && h <= 4096)
                    .ok_or(format!("--size {v}: WxH"))?;
            }
            "--last-frame" => o.last_frame = Some(value()?.into()),
            "--test-fail-after" => {
                o.fail_after = Some(value()?.parse().map_err(|_| "a whole number")?)
            }
            "--test-stall-after" => {
                o.stall_after = Some(value()?.parse().map_err(|_| "a whole number")?)
            }
            _ => return Err(format!("unknown argument {a}")),
        }
    }
    Ok(o)
}

fn main() {
    scanout::stop_on_signals();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => {}
        Err(e) => {
            println!("fail: {e}");
            std::process::exit(1);
        }
    }
}

/// Waits until stdin has something to read; false when asked to stop.
fn readable() -> bool {
    loop {
        if scanout::stopping() {
            return false;
        }
        let mut p = libc::pollfd {
            fd: 0,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one pollfd, alive for the call.
        let n = unsafe { libc::poll(&mut p, 1, 200) };
        if n > 0 {
            return true;
        }
    }
}

/// The next frame from stdin, or None when it closes (or a signal asks to
/// stop). `input` is this program's own buffer over stdin, so it knows
/// when bytes already read are waiting there: stdin is polled only when
/// the buffer is empty (polling with a frame already in a buffer would
/// wait for one that has come).
fn next_frame(input: &mut BufReader<File>) -> Result<Option<Frame>, String> {
    if input.buffer().is_empty() && !readable() {
        return Ok(None);
    }
    let mut head = [0u8; HEADER];
    let mut got = 0;
    while got < HEADER {
        match input.read(&mut head[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err("the app's input ended inside a frame".into()),
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(format!("reading the app's frames: {e}")),
        }
    }
    let len = u32::from_le_bytes([head[16], head[17], head[18], head[19]]) as usize;
    if !(HEADER..=MAX_FRAME).contains(&len) {
        return Err(format!("a frame that says it is {len} bytes"));
    }
    let mut bytes = vec![0u8; len];
    bytes[..HEADER].copy_from_slice(&head);
    input
        .read_exact(&mut bytes[HEADER..])
        .map_err(|e| format!("reading a frame: {e}"))?;
    let (frame, used) = Frame::decode(&bytes)?;
    if used != len {
        return Err(format!("a frame of {len} bytes decoded as {used}"));
    }
    Ok(Some(frame))
}

enum Panel {
    Kms(KmsDisplay, Vec<Imported>),
    Offscreen(Offscreen, Vec<Target>),
}

impl Panel {
    fn display(&mut self) -> &mut dyn Display {
        match self {
            Panel::Kms(d, _) => d,
            Panel::Offscreen(d, _) => d,
        }
    }

    /// Puts the screen back (KMS) and frees the GPU's side.
    fn close(self, gpu: &Gpu) -> Result<(), String> {
        match self {
            Panel::Kms(mut d, imports) => {
                let r = d.restore();
                for imp in imports {
                    imp.release(gpu);
                }
                drop(d);
                r
            }
            Panel::Offscreen(_, targets) => {
                for t in targets {
                    t.delete(gpu, true);
                }
                Ok(())
            }
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let o = options(args)?;
    let gpu = Gpu::surfaceless()?;
    let (mut panel, native, how) = if o.offscreen {
        let (w, h) = o.size;
        let targets = (0..2)
            .map(|_| Target::new(&gpu, w, h))
            .collect::<Result<Vec<_>, _>>()?;
        (
            Panel::Offscreen(Offscreen::default(), targets),
            (w, h),
            "offscreen".to_string(),
        )
    } else {
        scanout::import_support(&gpu)?;
        // Three buffers: two shown in turn, one spare (unused for now, so
        // the device sees the same set-up as the benchmark's flip step).
        let kms = KmsDisplay::open(&o.card, 3)?;
        let size = kms.size();
        let out = kms.output;
        let mode = out.mode();
        let how = format!(
            "kms {} connector {} crtc {} mode {} {} Hz",
            o.card.display(),
            out.connector,
            out.crtc.id,
            mode.name(),
            mode.vrefresh
        );
        let mut imports = Vec::new();
        for i in 0..2 {
            let b = kms.buffer(i);
            let fd = kms.dmabuf(i)?;
            match scanout::import_dmabuf(&gpu, fd.as_fd(), size.0, size.1, b.pitch, XRGB8888) {
                Ok(imp) => imports.push(imp),
                Err(e) => {
                    for imp in imports {
                        imp.release(&gpu);
                    }
                    return Err(e);
                }
            }
        }
        (Panel::Kms(kms, imports), size, how)
    };
    let result = serve(&gpu, &o, &mut panel, native, &how);
    let closed = panel.close(&gpu);
    result?;
    closed.map_err(|e| format!("putting the screen back: {e}"))
}

fn serve(
    gpu: &Gpu,
    o: &Options,
    panel: &mut Panel,
    native: (i32, i32),
    how: &str,
) -> Result<(), String> {
    let mut r = Renderer::new(gpu, native.0, native.1, o.rotation)?;
    let (lw, lh) = r.logical_size();
    let targets: Vec<Target> = match panel {
        Panel::Kms(d, imports) => {
            // Before anything is shown: the GPU's test card must be in the
            // screen buffer's own memory exactly, row 0 first.
            let card = test_card_into(gpu, &mut r, imports[0].target);
            let map = d.map(0)?;
            let wrong = scanout::buffer_differs(
                gpu,
                &r,
                imports[0].target,
                map.bytes(),
                d.buffer(0).pitch as usize,
            );
            if wrong != 0 {
                return Err(format!(
                    "{wrong} pixels of the test card are not in the screen buffer as drawn; nothing was shown"
                ));
            }
            drop(card);
            imports.iter().map(|i| i.target).collect()
        }
        Panel::Offscreen(_, targets) => targets.clone(),
    };
    let mut scan = Scanout::new(gpu, &r, targets)?;
    println!("ready {}x{} {lw}x{lh} {how}", native.0, native.1);
    let mut store = Store::new(usize::MAX);
    let stdin = std::io::stdin()
        .as_fd()
        .try_clone_to_owned()
        .map_err(|e| format!("the app's input: {e}"))?;
    let mut input = BufReader::with_capacity(1 << 16, File::from(stdin));
    let mut seq = 0u64;
    while let Some(frame) = next_frame(&mut input)? {
        seq += 1;
        if (frame.w, frame.h) != (lw, lh) {
            return Err(format!(
                "frame {seq} is {}x{}; this screen draws {lw}x{lh}",
                frame.w, frame.h
            ));
        }
        if o.fail_after.is_some_and(|n| seq > n) {
            return Err(format!(
                "failing after {} frames, as the test asked",
                seq - 1
            ));
        }
        if o.stall_after.is_some_and(|n| seq > n) {
            // Hang as a stuck driver would, until stopped.
            while !scanout::stopping() {
                std::thread::sleep(Duration::from_millis(50));
            }
            return Ok(());
        }
        store.apply(&frame);
        let shown = scan.present(
            gpu,
            &mut r,
            panel.display(),
            &frame.ops,
            &store,
            &frame.dropped,
        )?;
        let us = |d: Duration| d.as_micros() as u64;
        let mut out = std::io::stdout().lock();
        let line = match shown {
            Some(p) => format!(
                "shown {seq} {} {} {} {} {} {}",
                us(p.drawn),
                us(p.finished),
                us(p.waited),
                p.draw.plain_px,
                p.draw.shaded_px,
                p.draw.copied_px
            ),
            None => format!("same {seq}"),
        };
        writeln!(out, "{line}")
            .and_then(|_| out.flush())
            .map_err(|e| format!("answering the app: {e}"))?;
    }
    panel.display().wait()?;
    if let Some(path) = &o.last_frame {
        let px = scan.read_front(gpu).unwrap_or_default();
        let bytes: Vec<u8> = px.iter().flat_map(|p| p.to_le_bytes()).collect();
        std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

/// Draws the test card into `target`; the recorder that holds its list.
fn test_card_into(gpu: &Gpu, r: &mut Renderer, target: Target) -> mpfree_ui::record::Recorder {
    let (lw, lh) = r.logical_size();
    let card = scanout::test_card(lw, lh);
    r.play_on(gpu, target, None, card.ops(), card.store(), &[]);
    r.finish(gpu);
    r.forget_all(gpu);
    card
}
