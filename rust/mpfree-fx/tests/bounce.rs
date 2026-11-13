//! Bouncing a mixer: the mixdown file is the mixer's output sample for
//! sample, the stems add up to the master before its fader, every file
//! starts on the song's first frame and ends on the same frame whatever
//! each capture point's latency, any block size gives the same files, and
//! many capture points can be in use at once.

use std::io::Cursor;

use mpfree_fx::mpfree_node::{Ctx, Kind, Node, NodeInfo, ParamInfo, Timed, MAX_BLOCK};
use mpfree_fx::*;

const RATE: u32 = 48_000;

/// A pure delay that reports its latency, like a look-ahead effect.
struct Lag {
    n: usize,
    buf: [Vec<f32>; 2],
    w: usize,
}

static LAG: NodeInfo = NodeInfo {
    id: "test.lag",
    name: "Lag",
    kind: Kind::Effect,
};

impl Node for Lag {
    fn info(&self) -> &'static NodeInfo {
        &LAG
    }
    fn params(&self) -> &'static [ParamInfo] {
        &[]
    }
    fn set_param(&mut self, _id: u16, _value: f32) {}
    fn param(&self, _id: u16) -> f32 {
        f32::NAN
    }
    fn prepare(&mut self, _rate: u32, _max_block: usize) {}
    fn reset(&mut self) {
        for b in &mut self.buf {
            b.fill(0.0);
        }
    }
    fn process(&mut self, _ctx: &Ctx, _events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let size = self.n + 1;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            self.buf[0][self.w] = *l;
            self.buf[1][self.w] = *r;
            let rd = (self.w + size - self.n) % size;
            *l = self.buf[0][rd];
            *r = self.buf[1][rd];
            self.w = (self.w + 1) % size;
        }
    }
    fn latency(&self) -> u32 {
        self.n as u32
    }
}

fn lag(n: usize) -> Box<dyn Node> {
    Box::new(Lag {
        n,
        buf: [vec![0.0; n + 1], vec![0.0; n + 1]],
        w: 0,
    })
}

fn tone(hz: f32, amp: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| amp * (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin())
        .collect()
}

/// A 24-bit stereo WAV's frames (its header checked).
fn read24(b: &[u8]) -> (Vec<f32>, Vec<f32>) {
    assert_eq!(&b[0..4], b"RIFF");
    assert_eq!(u16::from_le_bytes([b[34], b[35]]), 24);
    let data = u32::from_le_bytes(b[40..44].try_into().unwrap()) as usize;
    assert_eq!(data + WAV_HEADER, b.len());
    let s =
        |i: usize| (i32::from_le_bytes([0, b[i], b[i + 1], b[i + 2]]) >> 8) as f32 / 8_388_607.0;
    let n = data / 6;
    (
        (0..n).map(|k| s(44 + 6 * k)).collect(),
        (0..n).map(|k| s(47 + 6 * k)).collect(),
    )
}

use mpfree_fx::bounce::WAV_HEADER;

struct Bounced {
    out: (Vec<f32>, Vec<f32>),
    files: Vec<(Vec<f32>, Vec<f32>)>,
    latency: u32,
    frames: u64,
}

/// Two tracks into the master, the second through 300 frames of look-ahead;
/// the mixdown, the master before its fader and each track's stem bounced
/// over `frames`, in blocks from `sizes`.
fn bounce_song(frames: usize, sizes: &[usize]) -> Bounced {
    let cfg = MixerConfig {
        inputs: 0,
        pads: 0,
        tracks: 2,
        slots: 2,
        max_latency_ms: 50.0,
    };
    let mut m = Mixer::new(cfg, RATE, 512);
    m.chain_mut(StripId::Track(1)).unwrap().push(lag(300));
    let feeds = [
        (tone(220.0, 0.3, frames), tone(330.0, 0.2, frames)),
        (tone(554.0, 0.25, frames), tone(110.0, 0.3, frames)),
    ];
    let points = [
        (StripId::Master, CapturePoint::AfterSafety),
        (StripId::Master, CapturePoint::BeforeFader),
        (StripId::Track(0), CapturePoint::AfterFader),
        (StripId::Track(1), CapturePoint::AfterFader),
    ];
    // Slots spread out, as an app with other captures would use them.
    let slots = [5usize, 40, 63, 17];
    for (slot, p) in slots.iter().zip(points) {
        assert!(m.set_capture(*slot, Some(p)));
    }
    let mut files: Vec<Cursor<Vec<u8>>> = (0..4).map(|_| Cursor::new(Vec::new())).collect();
    let mut out = (vec![0.0; frames], vec![0.0; frames]);
    let mut halves: Option<(BounceIn, BounceOut)> = None;
    let mut at = 0;
    let mut k = 0;
    while at < frames {
        let len = sizes[k % sizes.len()].min(MAX_BLOCK).min(frames - at);
        k += 1;
        for (t, (l, r)) in feeds.iter().enumerate() {
            let (il, ir) = m.strip_input(StripId::Track(t)).unwrap();
            il[..len].copy_from_slice(&l[at..at + len]);
            ir[..len].copy_from_slice(&r[at..at + len]);
        }
        m.process(
            &Ctx::new(RATE, at as u64),
            &mut out.0[at..at + len],
            &mut out.1[at..at + len],
        );
        // The latencies once the mixer has lined its paths up (its first
        // block), as a host reads them when the pass starts.
        let (bin, bout) = halves.get_or_insert_with(|| {
            let lat: Vec<u32> = slots
                .iter()
                .map(|&s| m.capture_latency(s).unwrap())
                .collect();
            bounce(&lat, 4096, RATE)
        });
        for (s, &slot) in slots.iter().enumerate() {
            let (l, r) = m.capture(slot).unwrap();
            bin.push(s, l, r);
        }
        at += len;
        if k % 4 == 0 {
            assert!(matches!(bout.pump(&mut files).unwrap(), Pumped::Working(_)));
        }
    }
    let (mut bin, mut bout) = halves.unwrap();
    bin.finish();
    let Pumped::Done(n) = bout.pump(&mut files).unwrap() else {
        panic!("not done");
    };
    assert_eq!(bin.overruns(), 0);
    Bounced {
        out,
        files: files.iter().map(|f| read24(f.get_ref())).collect(),
        latency: m.capture_latency(5).unwrap(),
        frames: n,
    }
}

const LSB: f32 = 1.0 / 8_388_607.0;

#[test]
fn the_mixdown_is_the_output_and_the_stems_add_up_to_the_master() {
    let frames = 24_000;
    let b = bounce_song(frames, &[256, 97, 512, 1, 333]);
    // The longest latency is the master's (the look-ahead and Master
    // safety's); every file is that much shorter than the pass.
    let most = b.latency as usize;
    assert!(most >= 300, "{most}");
    assert_eq!(b.frames as usize, frames - most);
    for f in &b.files {
        assert_eq!(f.0.len(), frames - most);
    }
    // The mixdown: the output from the song's first frame.
    let (l, r) = &b.files[0];
    for i in 0..l.len() {
        assert!(
            (l[i] - b.out.0[most + i]).abs() <= 0.5 * LSB + 1e-9,
            "left {i}"
        );
        assert!(
            (r[i] - b.out.1[most + i]).abs() <= 0.5 * LSB + 1e-9,
            "right {i}"
        );
    }
    // The stems add up to the master before its fader, frame for frame.
    let (ml, mr) = &b.files[1];
    let (al, ar) = &b.files[2];
    let (bl, br) = &b.files[3];
    let mut worst = 0.0f32;
    for i in 0..ml.len() {
        worst = worst
            .max((al[i] + bl[i] - ml[i]).abs())
            .max((ar[i] + br[i] - mr[i]).abs());
    }
    assert!(worst <= 2.0 * LSB, "stems off by {worst}");
    // And they are not silent: the first track starts on the first frame.
    assert!(al[1].abs() > 1e-3 && bl[1].abs() > 1e-3);
}

#[test]
fn any_block_size_bounces_the_same_files() {
    let a = bounce_song(12_000, &[512]);
    let b = bounce_song(12_000, &[1, 7, 64, 300, 511]);
    assert_eq!(a.frames, b.frames);
    for (x, y) in a.files.iter().zip(&b.files) {
        assert_eq!(x, y);
    }
}

#[test]
fn every_capture_point_can_be_in_use_and_freed_in_any_order() {
    let cfg = MixerConfig {
        inputs: 0,
        pads: 0,
        tracks: 8,
        slots: 1,
        max_latency_ms: 50.0,
    };
    let mut m = Mixer::new(cfg, RATE, 512);
    for slot in 0..CAPTURES {
        let t = StripId::Track(slot % 8);
        assert!(m.set_capture(slot, Some((t, CapturePoint::BeforeInserts))));
    }
    // Free every third, then play: each slot still names its own strip.
    for slot in (0..CAPTURES).step_by(3) {
        assert!(m.set_capture(slot, None));
    }
    let len = 64;
    for t in 0..8 {
        let (l, r) = m.strip_input(StripId::Track(t)).unwrap();
        l[..len].fill(t as f32 * 0.1);
        r[..len].fill(-(t as f32) * 0.1);
    }
    let mut out = (vec![0.0; len], vec![0.0; len]);
    m.process(&Ctx::new(RATE, 0), &mut out.0, &mut out.1);
    for slot in 0..CAPTURES {
        match m.capture(slot) {
            None => assert_eq!(slot % 3, 0, "slot {slot}"),
            Some((l, r)) => {
                let want = (slot % 8) as f32 * 0.1;
                assert!(l.iter().all(|&v| v == want), "slot {slot}");
                assert!(r.iter().all(|&v| v == -want), "slot {slot}");
            }
        }
    }
    assert!(!m.set_capture(CAPTURES, None));
}
