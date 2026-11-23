//! Stem separation for the MPFree sampler: a sound split into vocals,
//! drums, bass and other with Deezer's Spleeter 4-stem networks (code and
//! trained models MIT), ported to Rust.
//!
//! A [`Separator`] works through the sound a segment (512 STFT frames,
//! 11.9 s) at a time, in steps a worker thread runs ([`Separator::step`]):
//! a segment's STFT, then each stem's U-Net, then the masks and the
//! inverse STFTs, the stems' samples going to a [`Sink`] as they are
//! finished. A job stops between any two steps (a U-Net is about 1.6 s on
//! the MPC One's two job cores), and what it holds is known before it
//! starts ([`Model::need`]), so the job service can promise it.
//!
//! How it works (`docs/dev/design/stems.md` has the measurements):
//!
//! - **Spleeter's pipeline, kept:** 4096-point periodic Hann windows a hop
//!   of 1024 apart after 4096 leading zeros; the magnitudes of the first
//!   1024 bins (or 1536, [`Band`]) through each stem's U-Net; soft masks
//!   (o² + ε/4) / (Σ o² + ε); the masked STFT back through the inverse
//!   with the 2/3 window compensation. Whole segments, the last padded
//!   with silence, as Spleeter does.
//! - **The networks** (`net.rs`): every convolution a GEMM over patches
//!   gathered straight into the kernel's packed layout, with
//!   `mpfree_simd::gemm4x8` (NEON on the MPC) as the micro-kernel; the
//!   one- and two-channel layers without padding to its eight columns.
//! - **Weights as halves** (`half.rs`): 79 MB rather than 158, each layer
//!   unpacked when it runs (`mpfree_simd::f16_to_f32`) into one buffer as
//!   big as the widest (13 MB).
//! - **Streaming:** a segment's tensors are all a run holds; the inverse
//!   STFT is overlap-added a frame at a time and handed on 64 frames at a
//!   time, so a sound of any length needs the same memory.
//! - **The model file** (`model.rs`) is made from Deezer's checkpoint
//!   without TensorFlow ([`pack`]).
//!
//! Nothing here touches the audio thread or the device; it only computes.

#![forbid(unsafe_code)]

mod gemm;
mod half;
mod model;
mod net;
pub mod pack;
mod stft;

pub use model::Model;

use gemm::Work;
use net::Tensor;
use std::ops::Deref;
use std::time::{Duration, Instant};
use stft::{Channel, OverlapAdd, Rfft, BINS, HOP, N};

/// The sample rate the networks were trained at: a sound at another rate
/// is converted first.
pub const RATE: u32 = 44_100;

/// One of the four stems, in the model's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stem {
    Vocals,
    Drums,
    Bass,
    Other,
}

impl Stem {
    /// The model's order, which the [`Sink`] sees them in.
    pub const ALL: [Stem; 4] = [Stem::Vocals, Stem::Drums, Stem::Bass, Stem::Other];

    pub fn name(self) -> &'static str {
        match self {
            Stem::Vocals => "Vocals",
            Stem::Drums => "Drums",
            Stem::Bass => "Bass",
            Stem::Other => "Other",
        }
    }
}

/// How much of the spectrum the networks see.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Band {
    /// Spleeter's own: the networks see up to 11 kHz (1024 bins), and the
    /// stems are silent above it.
    Spleeter,
    /// The whole band: the networks see up to 16.5 kHz (1536 bins,
    /// Spleeter's 16 kHz setting, the same model), and above that each
    /// stem keeps the mean of its mask's top 64 bins, so the four stems
    /// add up to the sound. About a third more work.
    #[default]
    Full,
}

impl Band {
    /// Bins the networks see.
    pub fn bins(self) -> usize {
        match self {
            Band::Spleeter => 1024,
            Band::Full => 1536,
        }
    }

    fn extends(self) -> bool {
        self == Band::Full
    }
}

/// How a separation runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub band: Band,
    /// Threads each step spreads over: the cores the job may use (two on
    /// the MPC One, the job service's sound lane). Threads it starts take
    /// the calling thread's cores, so a job pinned to cores 0 and 1 stays
    /// there.
    pub threads: usize,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            band: Band::Full,
            threads: 2,
        }
    }
}

/// Where the stems go: each stem's next stereo frames (left, right,
/// interleaved), the stems in [`Stem::ALL`]'s order, until each has had
/// as many frames as the sound.
pub trait Sink {
    fn put(&mut self, stem: Stem, stereo: &[f32]) -> Result<(), String>;
}

/// Why a separation can't start or go on.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// The model file can't be read, or isn't one.
    Model(String),
    /// The sound isn't at 44.1 kHz.
    Rate(u32),
    /// The sound isn't mono or stereo (its channels).
    Channels(usize),
    /// The sink refused the stems.
    Sink(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Model(m) => write!(f, "stem model: {m}"),
            Error::Rate(r) => write!(f, "stems need a 44.1 kHz sound, not {r} Hz"),
            Error::Channels(c) => write!(f, "stems need a mono or stereo sound, not {c} channels"),
            Error::Sink(m) => write!(f, "stems: {m}"),
        }
    }
}

impl std::error::Error for Error {}

/// Where the time went, and the work done.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    /// The STFTs and magnitudes.
    pub analyse: Duration,
    /// The U-Nets, unpacking the weights included.
    pub nets: Duration,
    /// Unpacking the halves.
    pub unpack: Duration,
    /// The masks and the inverse STFTs.
    pub masks: Duration,
    /// Handing the stems to the sink.
    pub sink: Duration,
    /// The GEMM kernel's multiply-adds (its padding included).
    pub macs: u64,
    /// Segments finished.
    pub segments: usize,
}

/// STFT frames the inverse hands on at a time.
const CHUNK: usize = 64;
/// Samples a stream hands on at most in one chunk: its frames' hops,
/// and at the end the rest of the last window.
const CHUNK_SAMPLES: usize = CHUNK * HOP + N;
/// Small things a run holds besides its buffers (thread handles, the
/// row lists, the transform tables), with room to spare.
const SLACK: u64 = 256 * 1024;

impl Model {
    /// The most bytes a [`Separator`] on this model holds at once with
    /// `settings`, besides the model and the sound: what the job service
    /// promises for it. It doesn't grow with the sound's length.
    pub fn need(&self, settings: &Settings) -> u64 {
        self.need_at(settings.band.bins(), settings.threads)
    }

    pub(crate) fn need_at(&self, bins: usize, threads: usize) -> u64 {
        let shape = self.shape;
        let threads = threads.max(1);
        let rows = shape.rows;
        let tensor = rows * bins * 2;
        let thread = self
            .stems
            .iter()
            .map(|u| u.thread_floats(bins))
            .max()
            .unwrap_or(0);
        // Held throughout: the unpacked layer, both channels' STFTs, the
        // eight inverse streams, their chunks and the interleaving buffer,
        // the window and the transform's tables.
        let fixed = self.widest()
            + 2 * 2 * rows * BINS
            + 8 * stft::OVERLAP_FLOATS
            + 10 * CHUNK_SAMPLES
            + 4 * N;
        // The fourth U-Net: the magnitudes, three stems' estimates, the
        // network's own tensors and each thread's patches.
        let nets = tensor + 3 * tensor + shape.peak_floats(bins) + threads * thread;
        // The masks: four estimates and each thread's masked frame.
        let masks = 4 * tensor + threads * (2 * BINS + stft::FRAME_FLOATS);
        // The STFT: the magnitudes beside each thread's frame.
        let analyse = tensor + 2 * stft::FRAME_FLOATS;
        let most = nets.max(masks).max(analyse);
        4 * (fixed + most) as u64 + SLACK
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    Analyse,
    Net(usize),
    Masks,
    Done,
}

/// One sound's separation, a step at a time. It holds the model and the
/// sound through `M` and `I`: borrowed (`&Model`, `&[f32]`) for a run on one
/// thread, or shared (`Arc<Model>`, `Arc<[f32]>`) for a separation that
/// moves from one job to the next between steps, which `Separator` is
/// `Send` for.
pub struct Separator<M, I> {
    model: M,
    input: I,
    /// The input's whole frames, in samples (all channels).
    len: usize,
    channels: usize,
    bins: usize,
    extend: bool,
    threads: usize,
    rf: Rfft,
    win: Vec<f32>,
    frames: usize,
    segments: usize,
    seg: usize,
    stage: Stage,
    /// Both channels' STFT of the segment (re, im), rows × BINS.
    spec: Vec<(Vec<f32>, Vec<f32>)>,
    /// The segment's magnitudes, rows × bins × 2, until the U-Nets are done.
    x: Option<Tensor>,
    /// Each stem's estimate, then its mask.
    outs: Vec<Tensor>,
    /// The inverse STFT, by stem and channel, and each one's finished samples.
    ola: Vec<[OverlapAdd; 2]>,
    got: Vec<[Vec<f32>; 2]>,
    stereo: Vec<f32>,
    work: Work,
    stats: Stats,
}

impl<M: Deref<Target = Model>, I: AsRef<[f32]>> Separator<M, I> {
    /// A separation of `input` (interleaved, `channels` of 1 or 2, at
    /// `rate`). A mono sound is separated as stereo with both sides the
    /// same, as Spleeter does; the stems are always stereo (the networks'
    /// two outputs have their own weights, so a mono sound's stems can
    /// differ a little side to side).
    pub fn new(
        model: M,
        input: I,
        channels: usize,
        rate: u32,
        settings: &Settings,
    ) -> Result<Separator<M, I>, Error> {
        if rate != RATE {
            return Err(Error::Rate(rate));
        }
        let b = settings.band;
        Separator::at(
            model,
            input,
            channels,
            b.bins(),
            b.extends(),
            settings.threads,
        )
    }

    pub(crate) fn at(
        model: M,
        input: I,
        channels: usize,
        bins: usize,
        extend: bool,
        threads: usize,
    ) -> Result<Separator<M, I>, Error> {
        if !(1..=2).contains(&channels) {
            return Err(Error::Channels(channels));
        }
        assert!(bins.is_multiple_of(64) && bins <= BINS && (!extend || bins >= 64));
        let samples = input.as_ref().len() / channels;
        let frames = stft::frames(samples);
        let rows = model.shape.rows;
        let work = Work::new(model.widest());
        Ok(Separator {
            model,
            input,
            len: samples * channels,
            channels,
            bins,
            extend,
            threads: threads.max(1),
            rf: Rfft::new(),
            win: stft::hann(),
            frames,
            segments: frames.div_ceil(rows),
            seg: 0,
            stage: Stage::Analyse,
            spec: (0..2)
                .map(|_| (vec![0.0; rows * BINS], vec![0.0; rows * BINS]))
                .collect(),
            x: None,
            outs: Vec::with_capacity(4),
            ola: (0..4)
                .map(|_| [OverlapAdd::new(samples), OverlapAdd::new(samples)])
                .collect(),
            got: (0..4)
                .map(|_| {
                    [
                        Vec::with_capacity(CHUNK_SAMPLES),
                        Vec::with_capacity(CHUNK_SAMPLES),
                    ]
                })
                .collect(),
            stereo: Vec::with_capacity(2 * CHUNK_SAMPLES),
            work,
            stats: Stats::default(),
        })
    }

    /// Runs the next step; false once the stems are all out (and then
    /// does nothing).
    pub fn step(&mut self, sink: &mut dyn Sink) -> Result<bool, Error> {
        match self.stage {
            Stage::Analyse => {
                let t = Instant::now();
                self.analyse();
                self.stats.analyse += t.elapsed();
                self.stage = Stage::Net(0);
            }
            Stage::Net(k) => {
                let t = Instant::now();
                let x = self.x.as_ref().expect("the segment's magnitudes");
                let out = self.model.stems[k].run(x, self.threads, &mut self.work);
                self.outs.push(out);
                self.stats.nets += t.elapsed();
                self.stage = if k + 1 < self.model.stems.len() {
                    Stage::Net(k + 1)
                } else {
                    self.x = None;
                    Stage::Masks
                };
            }
            Stage::Masks => {
                self.masks(sink)?;
                self.outs.clear();
                self.seg += 1;
                self.stats.segments = self.seg;
                self.stage = if self.seg < self.segments {
                    Stage::Analyse
                } else {
                    Stage::Done
                };
            }
            Stage::Done => return Ok(false),
        }
        Ok(self.stage != Stage::Done)
    }

    /// How far it has got, 0 to 1, in steps.
    pub fn progress(&self) -> f32 {
        let per = 2 + self.model.stems.len();
        let at = match self.stage {
            Stage::Analyse => 0,
            Stage::Net(k) => 1 + k,
            Stage::Masks => per - 1,
            Stage::Done => return 1.0,
        };
        (self.seg * per + at) as f32 / (self.segments * per) as f32
    }

    /// The time and work so far.
    pub fn stats(&self) -> Stats {
        Stats {
            unpack: self.work.unpack,
            macs: self.work.macs,
            ..self.stats
        }
    }

    fn rows_now(&self) -> usize {
        let rows = self.model.shape.rows;
        rows.min(self.frames - self.seg * rows)
    }

    /// The segment's STFT, both channels, and its magnitudes.
    fn analyse(&mut self) {
        let rows = self.model.shape.rows;
        let (f0, nf) = (self.seg * rows, self.rows_now());
        let (rf, win) = (&self.rf, &self.win);
        let channels = self.channels;
        let input = &self.input.as_ref()[..self.len];
        let one = |ch: usize, (re, im): &mut (Vec<f32>, Vec<f32>)| {
            let x = Channel {
                data: input,
                channels,
                ch: ch.min(channels - 1),
            };
            let mut w = stft::Frame::new();
            for t in 0..nf {
                let (r, i) = (
                    &mut re[t * BINS..(t + 1) * BINS],
                    &mut im[t * BINS..(t + 1) * BINS],
                );
                stft::analyse(x, f0 + t, rf, win, &mut w, r, i);
            }
        };
        if self.threads >= 2 {
            std::thread::scope(|sc| {
                for (ch, s) in self.spec.iter_mut().enumerate() {
                    sc.spawn(move || one(ch, s));
                }
            });
        } else {
            self.spec
                .iter_mut()
                .enumerate()
                .for_each(|(ch, s)| one(ch, s));
        }
        let bins = self.bins;
        let mut x = Tensor::zeros(rows, bins, 2);
        for t in 0..nf {
            for f in 0..bins {
                for (ch, (re, im)) in self.spec.iter().enumerate() {
                    let (a, b) = (re[t * BINS + f], im[t * BINS + f]);
                    x.data[(t * bins + f) * 2 + ch] = (a * a + b * b).sqrt();
                }
            }
        }
        self.x = Some(x);
    }

    /// Each stem's estimate becomes its mask; the masked STFTs go back
    /// through the inverse a chunk at a time, to the sink.
    fn masks(&mut self, sink: &mut dyn Sink) -> Result<(), Error> {
        let t = Instant::now();
        let rows = self.model.shape.rows;
        let (f0, nf, bins) = (self.seg * rows, self.rows_now(), self.bins);
        // (o² + ε/4) / (Σ o² + ε), in place.
        for p in 0..nf * bins * 2 {
            let sum = self.outs.iter().map(|o| o.data[p] * o.data[p]).sum::<f32>() + 1e-10;
            for o in self.outs.iter_mut() {
                o.data[p] = (o.data[p] * o.data[p] + 1e-10 / 4.0) / sum;
            }
        }
        self.stats.masks += t.elapsed();
        for c0 in (0..nf).step_by(CHUNK) {
            let t = Instant::now();
            let c1 = (c0 + CHUNK).min(nf);
            let last = f0 + c1 == self.frames;
            let mut streams: Vec<(usize, usize, &mut OverlapAdd, &mut Vec<f32>)> = Vec::new();
            for (k, (o, g)) in self.ola.iter_mut().zip(self.got.iter_mut()).enumerate() {
                for (ch, (oa, gv)) in o.iter_mut().zip(g.iter_mut()).enumerate() {
                    streams.push((k, ch, oa, gv));
                }
            }
            let per = streams.len().div_ceil(self.threads);
            let (outs, spec, rf, win) = (&self.outs, &self.spec, &self.rf, &self.win);
            let extend = self.extend;
            std::thread::scope(|sc| {
                for group in streams.chunks_mut(per) {
                    sc.spawn(move || {
                        let (mut mr, mut mi) = (vec![0f32; BINS], vec![0f32; BINS]);
                        for (k, ch, oa, gv) in group.iter_mut() {
                            let (k, ch) = (*k, *ch);
                            let mask = &outs[k].data;
                            let (re, im) = (&spec[ch].0, &spec[ch].1);
                            for t in c0..c1 {
                                let (re, im) =
                                    (&re[t * BINS..(t + 1) * BINS], &im[t * BINS..(t + 1) * BINS]);
                                let m = |f: usize| mask[(t * bins + f) * 2 + ch];
                                for f in 0..bins {
                                    mr[f] = re[f] * m(f);
                                    mi[f] = im[f] * m(f);
                                }
                                let top = if extend {
                                    (bins - 64..bins).map(m).sum::<f32>() / 64.0
                                } else {
                                    0.0
                                };
                                for f in bins..BINS {
                                    mr[f] = re[f] * top;
                                    mi[f] = im[f] * top;
                                }
                                oa.push(&mr, &mi, rf, win, gv);
                            }
                            if last {
                                oa.finish(gv);
                            }
                        }
                    });
                }
            });
            drop(streams);
            self.stats.masks += t.elapsed();
            let t = Instant::now();
            for (stem, g) in Stem::ALL.iter().zip(self.got.iter_mut()) {
                let [l, r] = g;
                debug_assert_eq!(l.len(), r.len());
                if !l.is_empty() {
                    self.stereo.clear();
                    for (a, b) in l.iter().zip(r.iter()) {
                        self.stereo.push(*a);
                        self.stereo.push(*b);
                    }
                    sink.put(*stem, &self.stereo).map_err(Error::Sink)?;
                }
                l.clear();
                r.clear();
            }
            self.stats.sink += t.elapsed();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
