//! The plugin process: loads one CLAP plugin with `mpfree-clap` and serves
//! the host.
//!
//! Two threads. The **main thread** (the plugin's CLAP main thread) answers
//! the control channel: prepare, state, a value's text, quit; between
//! messages it answers the plugin's own requests (`ClapControl::idle`). The
//! **audio thread** waits on the shared block's `req` word, runs one block
//! of the plugin and posts `done`.
//!
//! The node sits behind a mutex that only prepare and the audio thread take,
//! so a prepare never runs in the middle of a block. State and text go
//! through `ClapControl`, which CLAP allows beside a running block.

use crate::shm::{self, Mapping};
use crate::wire::{self, Hello, ToChild, ToHost};
use mpfree_clap::{ClapControl, ClapNode, Library, ParamSpec};
use mpfree_node::{Ctx, Kind, Node, NodeEvent, Timed, MAX_BLOCK};
use std::ffi::OsString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How often the main thread looks at the plugin's requests when the host
/// is quiet.
const IDLE: Duration = Duration::from_millis(100);
/// How often the audio thread looks at the quit flag when no block comes.
const AUDIO_IDLE: Duration = Duration::from_millis(200);

/// What the process was started with.
#[derive(Debug, PartialEq)]
pub struct Args {
    pub plugin: PathBuf,
    pub id: String,
    pub shm_fd: i32,
    pub ctl_fd: i32,
}

/// Reads `--plugin FILE --id PLUGIN-ID --shm-fd N --ctl-fd N`, in any
/// order.
pub fn parse_args(args: impl IntoIterator<Item = OsString>) -> Option<Args> {
    let (mut plugin, mut id, mut shm_fd, mut ctl_fd) = (None, None, None, None);
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let v = it.next()?;
        match a.to_str()? {
            "--plugin" => plugin = Some(PathBuf::from(v)),
            "--id" => id = Some(v.into_string().ok()?),
            "--shm-fd" => shm_fd = v.to_str()?.parse().ok(),
            "--ctl-fd" => ctl_fd = v.to_str()?.parse().ok(),
            _ => return None,
        }
    }
    Some(Args {
        plugin: plugin?,
        id: id?,
        shm_fd: shm_fd?,
        ctl_fd: ctl_fd?,
    })
}

/// The process's whole life; returns its exit status.
pub fn child_main(args: impl IntoIterator<Item = OsString>) -> i32 {
    let Some(a) = parse_args(args) else {
        eprintln!("usage: mpfree-plughost --plugin FILE --id PLUGIN-ID --shm-fd N --ctl-fd N");
        eprintln!("(started by MPFree's plugin sandbox, not by hand)");
        return 64;
    };
    // SAFETY: the host handed us this end of a socket pair at this number.
    let mut sock = unsafe { UnixStream::from_raw_fd(a.ctl_fd) };
    let fail = |sock: &mut UnixStream, msg: String| {
        let _ = wire::send_to_host(sock, &ToHost::Failed(msg));
        2
    };
    let map = match Mapping::attach(a.shm_fd) {
        Ok(m) => Arc::new(m),
        Err(e) => return fail(&mut sock, format!("shared memory: {e}")),
    };
    let node = match Library::open(&a.plugin).and_then(|lib| lib.create(&a.id)) {
        Ok(n) => n,
        Err(e) => return fail(&mut sock, e.to_string()),
    };
    let hello = Hello {
        id: a.id.clone(),
        name: node.info().name.to_string(),
        kind: node.kind(),
        params: node.params().iter().map(ParamSpec::from_info).collect(),
    };
    if wire::send_to_host(&mut sock, &ToHost::Hello(hello)).is_err() {
        return 2;
    }
    let control = node.control();
    let nparams = node.params().len().min(shm::MAX_PARAMS);
    let node = Arc::new(Mutex::new(node));
    let quit = Arc::new(AtomicBool::new(false));
    let audio = {
        let (map, node, quit) = (Arc::clone(&map), Arc::clone(&node), Arc::clone(&quit));
        std::thread::Builder::new()
            .name("plughost-audio".into())
            .spawn(move || audio_loop(&map, &node, nparams, &quit))
    };
    let Ok(audio) = audio else { return 2 };
    let served = serve(&mut sock, &node, &control);
    quit.store(true, Ordering::Release);
    shm::futex_wake(map.req());
    let _ = audio.join();
    // The plugin is destroyed here, on its main thread.
    drop(control);
    drop(node);
    i32::from(served.is_err())
}

/// The main thread: answers the host until it says quit or goes away.
fn serve(sock: &mut UnixStream, node: &Mutex<ClapNode>, control: &ClapControl) -> io::Result<()> {
    loop {
        let mut pfd = libc::pollfd {
            fd: sock.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd.
        let r = unsafe { libc::poll(&mut pfd, 1, IDLE.as_millis() as libc::c_int) };
        if r < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if r == 0 {
            control.idle();
            continue;
        }
        let m = match wire::read_to_child(sock) {
            Ok(m) => m,
            // The host is gone (or closed its end): quit quietly.
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        let answer = match m {
            ToChild::Prepare { rate, max_block } => {
                let mut n = node.lock().unwrap_or_else(|e| e.into_inner());
                n.prepare(rate, max_block as usize);
                ToHost::Prepared {
                    ok: n.is_active(),
                    latency: n.latency(),
                    tail: n.tail_frames(),
                }
            }
            ToChild::SaveState => ToHost::State(control.save_state()),
            ToChild::LoadState(b) => ToHost::Loaded(control.load_state(&b)),
            ToChild::ParamText { index, value } => ToHost::Text(control.param_text(index, value)),
            ToChild::Quit => return Ok(()),
        };
        wire::send_to_host(sock, &answer)?;
        control.idle();
    }
}

/// The audio thread: one block per request, until quit.
fn audio_loop(map: &Mapping, node: &Mutex<ClapNode>, nparams: usize, quit: &AtomicBool) {
    let (req, done) = (map.req(), map.done());
    let mut events: Vec<Timed> = Vec::with_capacity(shm::MAX_EVENTS);
    // Start from the last answered request, not the last posted one: the
    // host may post its first request before this thread gets going. (On a
    // restart the host sets `done` to `req` before starting us, writing
    // off a request the old process never answered.)
    let mut seen = shm::load(done);
    loop {
        if quit.load(Ordering::Acquire) {
            return;
        }
        let r = shm::load(req);
        if r == seen {
            shm::futex_wait(req, r, Some(AUDIO_IDLE));
            continue;
        }
        seen = r;
        // SAFETY: the host posted request r and won't touch the block
        // until we post done == r.
        unsafe { run_block(map.ptr, node, &mut events, nparams) };
        shm::post(done, r);
    }
}

fn event_of(e: &shm::ShmEvent) -> Option<NodeEvent> {
    let byte = |v: u32| v.min(127) as u8;
    Some(match e.kind {
        shm::EV_NOTE_ON => NodeEvent::NoteOn {
            note: byte(e.a),
            velocity: byte(e.b),
        },
        shm::EV_NOTE_OFF => NodeEvent::NoteOff { note: byte(e.a) },
        shm::EV_PRESSURE => NodeEvent::Pressure {
            note: byte(e.a),
            value: byte(e.b),
        },
        shm::EV_BEND => NodeEvent::Bend { value: e.value },
        shm::EV_ALL_OFF => NodeEvent::AllOff,
        shm::EV_PARAM => NodeEvent::Param {
            id: u16::try_from(e.a).ok()?,
            value: e.value,
        },
        _ => return None,
    })
}

/// Runs one block from the shared request into the shared answer.
///
/// # Safety
/// `p` is the mapped block, and this side owns the request and answer
/// fields (between the host's `req` and our `done`).
unsafe fn run_block(
    p: *mut shm::Shm,
    node: &Mutex<ClapNode>,
    events: &mut Vec<Timed>,
    nparams: usize,
) {
    let n = ((*p).frames as usize).min(MAX_BLOCK);
    let ctx = Ctx {
        rate: (*p).rate,
        frame: (*p).frame,
        bpm: (*p).bpm,
        beats: (*p).beats,
        playing: (*p).playing != 0,
        sidechain: None,
    };
    events.clear();
    let mut reset = false;
    let count = ((*p).nevents as usize).min(shm::MAX_EVENTS);
    for e in &(&(*p).events)[..count] {
        if e.kind == shm::EV_RESET {
            reset = true;
        } else if let Some(ev) = event_of(e) {
            events.push(Timed {
                offset: e.offset.min(n.saturating_sub(1) as u32),
                ev,
            });
        }
    }
    let mut g = node.lock().unwrap_or_else(|e| e.into_inner());
    if reset {
        g.reset();
    }
    let out_l = &mut (&mut (*p).out_l)[..n];
    let out_r = &mut (&mut (*p).out_r)[..n];
    if g.kind() == Kind::Effect {
        out_l.copy_from_slice(&(&(*p).in_l)[..n]);
        out_r.copy_from_slice(&(&(*p).in_r)[..n]);
    } else {
        out_l.fill(0.0);
        out_r.fill(0.0);
    }
    if n > 0 {
        g.process(&ctx, events, out_l, out_r);
    }
    for i in 0..nparams {
        (*p).values[i] = g.param(i as u16);
    }
    (*p).nvalues = nparams as u32;
    (*p).errors = g.stats().errors.min(u64::from(u32::MAX)) as u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn arguments_are_read_in_any_order() {
        let want = Args {
            plugin: "/x/p.clap".into(),
            id: "org.x".into(),
            shm_fd: 3,
            ctl_fd: 4,
        };
        assert_eq!(
            parse_args(os(&[
                "--plugin",
                "/x/p.clap",
                "--id",
                "org.x",
                "--shm-fd",
                "3",
                "--ctl-fd",
                "4"
            ])),
            Some(want)
        );
        let a = parse_args(os(&[
            "--ctl-fd", "9", "--shm-fd", "8", "--id", "y", "--plugin", "q",
        ]))
        .unwrap();
        assert_eq!((a.shm_fd, a.ctl_fd), (8, 9));
    }

    #[test]
    fn bad_arguments_are_refused() {
        assert_eq!(parse_args(os(&[])), None);
        assert_eq!(
            parse_args(os(&["--plugin", "p", "--id", "x", "--shm-fd", "3"])),
            None
        );
        assert_eq!(
            parse_args(os(&[
                "--plugin", "p", "--id", "x", "--shm-fd", "three", "--ctl-fd", "4"
            ])),
            None
        );
        assert_eq!(
            parse_args(os(&[
                "--plugin", "p", "--id", "x", "--shm-fd", "3", "--ctl-fd", "4", "--what", "1"
            ])),
            None
        );
        assert_eq!(parse_args(os(&["--plugin"])), None);
    }

    #[test]
    fn events_map_back() {
        let e = |kind, a, b, value| shm::ShmEvent {
            offset: 0,
            kind,
            a,
            b,
            value,
            _pad: 0,
        };
        assert_eq!(
            event_of(&e(shm::EV_NOTE_ON, 60, 100, 0.0)),
            Some(NodeEvent::NoteOn {
                note: 60,
                velocity: 100
            })
        );
        assert_eq!(
            event_of(&e(shm::EV_NOTE_OFF, 300, 0, 0.0)),
            Some(NodeEvent::NoteOff { note: 127 })
        );
        assert_eq!(
            event_of(&e(shm::EV_PRESSURE, 61, 40, 0.0)),
            Some(NodeEvent::Pressure {
                note: 61,
                value: 40
            })
        );
        assert_eq!(
            event_of(&e(shm::EV_BEND, 0, 0, -0.5)),
            Some(NodeEvent::Bend { value: -0.5 })
        );
        assert_eq!(
            event_of(&e(shm::EV_ALL_OFF, 0, 0, 0.0)),
            Some(NodeEvent::AllOff)
        );
        assert_eq!(
            event_of(&e(shm::EV_PARAM, 3, 0, 0.25)),
            Some(NodeEvent::Param { id: 3, value: 0.25 })
        );
        assert_eq!(event_of(&e(shm::EV_PARAM, 70_000, 0, 0.25)), None);
        assert_eq!(event_of(&e(99, 0, 0, 0.0)), None);
    }
}
