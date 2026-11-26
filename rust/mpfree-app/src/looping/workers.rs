//! The worker threads: smart loop end's finds ([`Finder`]) and the prints
//! that keep a loop's pitch at another tempo ([`Printer`]).

use super::*;

/// What the worker is asked: a buffer for a bounce, a print, room for a
/// save's audio, or to drop what the audio loop is done with.
pub(crate) enum ToPrinter {
    Buffer(usize),
    Print(Bounce, usize),
    Drop(Option<Print>, Vec<f32>),
    Size(Saved),
    DropSaved(Saved),
    DropLoop(LoopFile),
}

enum FromPrinter {
    Buffer(Vec<f32>),
    /// The print for (loop, frames), or None if it could not be made.
    Done(usize, usize, Option<Print>),
    /// A save with room for every layer's audio.
    Sized(Saved),
}

/// One print under way: for loop `n`, `frames` long, from the loop at
/// `version`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Wanted {
    n: usize,
    pub(crate) frames: usize,
    version: u64,
}

/// Smart loop end's finder for the line input and mixes (the smart loop
/// end plan, step 3; [`mpfree_looper::find`]): a thread of its own, not
/// the print worker, which runs one long job at a time at idle priority,
/// so a print can never make Auto miss its moment. One job, made here
/// once, goes back and forth through two one-place channels: the looper
/// fills it when it wants a decision ([`Looper::find_job`]), the thread
/// answers it and sends it back, and the next period hands the answer to
/// the looper ([`Looper::found`]), which drops it if its take has gone.
/// The thread sleeps until asked, at the normal priority, off the sound's
/// cores. Nothing here waits or allocates on the audio thread: an ask
/// made while the job is still out waits for the next period, and the
/// looper keeps the tap when the answer comes too late (counted), as it
/// does if the thread is gone.
pub struct Finder {
    to: Option<SyncSender<Box<FindJob>>>,
    pub(crate) back: Option<Receiver<Box<FindJob>>>,
    /// The job, when it isn't with the thread.
    pub(crate) job: Option<Box<FindJob>>,
    /// Answer on the calling thread instead (tests that step periods far
    /// faster than they would play).
    pub inline: bool,
    /// Jobs answered, and periods an ask waited for the job to come back.
    pub answered: u64,
    pub waited: u64,
    /// The thread couldn't start or has stopped: the looper keeps every
    /// tap its finder would have weighed.
    pub gone: bool,
}

impl Finder {
    pub(crate) fn new(rate: u32) -> Finder {
        let job = Box::new(FindJob::new(mpfree_looper::ends::reach(rate, f64::NAN)));
        let (to, jobs) = sync_channel::<Box<FindJob>>(1);
        let (done, back) = sync_channel::<Box<FindJob>>(1);
        let spawned = std::thread::Builder::new()
            .name("mpfree-find".into())
            .spawn(move || {
                // Normal priority, off the sound's cores.
                let _ = crate::cores::background();
                for mut j in jobs {
                    find(&mut j);
                    if done.send(j).is_err() {
                        break;
                    }
                }
            })
            .is_ok();
        Finder {
            to: spawned.then_some(to),
            back: spawned.then_some(back),
            job: Some(job),
            inline: false,
            answered: 0,
            waited: 0,
            gone: !spawned,
        }
    }

    /// The thread is gone: nothing more goes to it.
    fn lose(&mut self) {
        self.to = None;
        self.back = None;
        self.gone = true;
    }

    /// After the looper's period: an answer back goes to the looper, and
    /// a new ask goes to the thread (or is answered here when `inline`).
    pub(crate) fn tend(&mut self, looper: &mut Looper) {
        if self.job.is_none() {
            match self.back.as_ref().map(Receiver::try_recv) {
                Some(Ok(j)) => {
                    looper.found(&j);
                    self.answered += 1;
                    self.job = Some(j);
                }
                Some(Err(TryRecvError::Disconnected)) => self.lose(),
                _ => {}
            }
        }
        if !self.inline && self.to.is_none() {
            return;
        }
        let Some(mut j) = self.job.take() else {
            if looper.find_wanted() {
                self.waited += 1;
            }
            return;
        };
        if !looper.find_job(&mut j) {
            self.job = Some(j);
            return;
        }
        if self.inline {
            find(&mut j);
            looper.found(&j);
            self.answered += 1;
            self.job = Some(j);
            return;
        }
        if let Some(to) = &self.to {
            match to.try_send(j) {
                Ok(()) => {}
                // (Full can't happen: there is one job, and it was here.)
                Err(TrySendError::Full(j)) => self.job = Some(j),
                Err(TrySendError::Disconnected(j)) => {
                    self.job = Some(j);
                    self.lose();
                }
            }
        }
    }
}

/// Prints made on a worker thread, one at a time (see the module notes).
pub struct Printer {
    rate: u32,
    pub(crate) tx: Option<Sender<ToPrinter>>,
    rx: Option<Receiver<FromPrinter>>,
    /// A buffer from the worker, for the next bounce.
    buf: Option<Vec<f32>>,
    pub(crate) asked: bool,
    /// The print under way, and whether its bounce is rendering.
    pub(crate) doing: Option<Wanted>,
    bouncing: bool,
    /// Prints handed to the looper, and ones made too late to use.
    pub made: u64,
    pub stale: u64,
    pub failed: u64,
    /// A save the worker made room for.
    pub(crate) sized: Option<Saved>,
}

impl Printer {
    pub(crate) fn new(rate: u32) -> Printer {
        let (tx, jobs) = channel::<ToPrinter>();
        let (done, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("mpfree-print".into())
            .spawn(move || {
                // Off the sound's cores and priority, and after the screen.
                mpfree_lanes::os::idle_job();
                for m in jobs {
                    let reply = match m {
                        ToPrinter::Buffer(n) => FromPrinter::Buffer(Vec::with_capacity(n)),
                        ToPrinter::Print(b, frames) => {
                            let p = PrintJob::new(&b, rate, frames).ok().and_then(|mut j| {
                                while !j.run(1 << 15) {}
                                j.finish()
                            });
                            FromPrinter::Done(b.from_loop, frames, p)
                        }
                        ToPrinter::Drop(p, b) => {
                            drop((p, b));
                            continue;
                        }
                        ToPrinter::Size(mut saved) => {
                            for (_, f) in &mut saved.loops {
                                for k in 0..f.layers.len() {
                                    // A copied layer's file holds it (4c.4).
                                    let size = layer_frames(f, k);
                                    if size > 0 {
                                        f.layers[k].audio = vec![0.0; 2 * size];
                                    }
                                }
                            }
                            FromPrinter::Sized(saved)
                        }
                        ToPrinter::DropSaved(saved) => {
                            drop(saved);
                            continue;
                        }
                        ToPrinter::DropLoop(f) => {
                            drop(f);
                            continue;
                        }
                    };
                    if done.send(reply).is_err() {
                        break;
                    }
                }
            })
            .is_ok();
        Printer {
            rate,
            tx: spawned.then_some(tx),
            rx: spawned.then_some(rx),
            buf: None,
            asked: false,
            doing: None,
            bouncing: false,
            made: 0,
            stale: 0,
            failed: 0,
            sized: None,
        }
    }

    pub(crate) fn send(&self, m: ToPrinter) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(m);
        }
    }

    /// One period's look after the looper ran at `bpm` (the song's
    /// tempo; a free loop goes by the free clock's, [`tempo_of`]): hand
    /// over what the worker made, take a finished bounce to it, drop old
    /// prints, and start the next print a loop needs (or take away one it
    /// no longer does). Cheap when nothing changes. It allocates nothing
    /// itself; a message to the worker can (a block of the channel, now
    /// and then), and only goes when a print starts, arrives or is
    /// dropped.
    pub(crate) fn tend(&mut self, looper: &mut Looper, bpm: f64) {
        if self.tx.is_none() || bpm.is_nan() || bpm <= 0.0 {
            return;
        }
        let free = looper.free_clock().map(|c| c.bpm);
        for n in 0..MAX_LOOPS {
            if let Some(p) = looper.take_print(n) {
                self.send(ToPrinter::Drop(Some(p), Vec::new()));
            }
            if let Some(f) = looper.take_loaded(n) {
                self.send(ToPrinter::DropLoop(f));
            }
        }
        while let Some(m) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            match m {
                FromPrinter::Sized(saved) => self.sized = Some(saved),
                FromPrinter::Buffer(b) => {
                    self.asked = false;
                    self.buf = Some(b);
                }
                FromPrinter::Done(n, frames, p) => {
                    self.doing = None;
                    let Some(p) = p else {
                        self.failed += 1;
                        continue;
                    };
                    let now = looper.loop_status(n);
                    let wanted = now.is_some_and(|s| {
                        s.version == p.version
                            && Some(frames)
                                == tempo_of(&s, bpm, free).and_then(|b| want(&s, b, self.rate))
                    });
                    if !wanted {
                        self.stale += 1;
                        self.send(ToPrinter::Drop(Some(p), Vec::new()));
                        continue;
                    }
                    match looper.print(n, Some(p)) {
                        Ok(()) => self.made += 1,
                        Err(p) => self.send(ToPrinter::Drop(p, Vec::new())),
                    }
                }
            }
        }
        if self.bouncing {
            match looper.take_bounce() {
                Some(b) => {
                    self.bouncing = false;
                    let d = self.doing.expect("a print is under way");
                    if b.complete && b.version == d.version {
                        self.send(ToPrinter::Print(b, d.frames));
                    } else {
                        self.doing = None;
                        self.send(ToPrinter::Drop(None, b.audio));
                    }
                }
                None if looper.bounce_progress().is_none() => {
                    self.bouncing = false;
                    self.doing = None;
                }
                None => {}
            }
        }
        if self.doing.is_some() {
            return;
        }
        for n in 0..MAX_LOOPS {
            let Some(s) = looper.loop_status(n) else {
                break;
            };
            let have = looper.print_info(n);
            match tempo_of(&s, bpm, free).and_then(|b| want(&s, b, self.rate)) {
                None if have.is_some() && s.phase == Phase::Playing => {
                    let _ = looper.print(n, None);
                }
                None => {}
                Some(frames) if have == Some((frames, s.ticks, s.version)) => {}
                Some(frames) => {
                    let need = 2 * s.frames;
                    match self.buf.take() {
                        Some(b) if b.capacity() >= need => match looper.bounce_print(n, b) {
                            Ok(()) => {
                                self.doing = Some(Wanted {
                                    n,
                                    frames,
                                    version: s.version,
                                });
                                self.bouncing = true;
                            }
                            Err(b) => self.buf = Some(b),
                        },
                        Some(b) => {
                            self.send(ToPrinter::Drop(None, b));
                            self.ask(need);
                        }
                        None => self.ask(need),
                    }
                    return;
                }
            }
        }
    }

    fn ask(&mut self, samples: usize) {
        if !self.asked {
            self.asked = true;
            self.send(ToPrinter::Buffer(samples));
        }
    }
}

/// The tempo loop `s` plays at: a free loop ([`Clock::Free`]) the free
/// clock's, `free` (None without one), so a tempo change in the song never
/// asks for a print of it; any other the song's, `song`.
fn tempo_of(s: &Status, song: f64, free: Option<f64>) -> Option<f64> {
    match s.clock {
        Clock::Free => free,
        Clock::Song => Some(song),
    }
}

/// The print loop `s` needs at `bpm`: its length then, if that is not its
/// own (within a frame) and it is playing; within the stretch's 10x. At
/// another speed ([`Status::rate`]) it always wants one: the print makes
/// the octaves cleanly, and an overdub waits for it to fit to.
pub(crate) fn want(s: &Status, bpm: f64, rate: u32) -> Option<usize> {
    if s.phase != Phase::Playing || s.frames == 0 || s.ticks <= 0 {
        return None;
    }
    let frames = frames_at(s.ticks, bpm, rate);
    // The audio's length once resampled to its speed, which the stretch
    // then takes to `frames`.
    let sped = s.frames as f64 * 2f64.powi(-i32::from(s.rate));
    let r = frames as f64 / sped;
    let off = s.rate != 0 || frames.abs_diff(s.frames) > 1;
    (off && (0.1..=10.0).contains(&r)).then_some(frames)
}
