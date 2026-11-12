//! Audio tracks saved and opened again: three loop passes comped and
//! mixed, saved through the text and a WAV a take, loaded into fresh
//! tracks, play back the same sample for sample; loads copy in over a few
//! periods and play nothing until done; bad loads are refused.

use mpfree_audiotracks::save::{from_text, from_wav, to_text, wav};
use mpfree_audiotracks::{AudioTracks, Command as A, Input, Monitor, LOAD_SPEED};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: u64 = 22_050;
const BAR_TICKS: i64 = 3_840;
const START: u64 = 1_000;

fn sig(g: u64) -> f32 {
    ((g * 7919) % 20011) as f32 / 20011.0 - 0.5
}

struct Rig {
    t: Transport,
    a: AudioTracks,
    b: Block,
    frame: u64,
    out: Vec<f32>,
    live: bool,
}

impl Rig {
    fn new(a: AudioTracks) -> Rig {
        Rig {
            t: Transport::new(RATE, 240.0).unwrap(),
            a,
            b: Block::new(),
            frame: 0,
            out: Vec::new(),
            live: true,
        }
    }

    fn run_to(&mut self, frame: u64) {
        const P: usize = 256;
        let mut inp = [0.0f32; 2 * P];
        let mut mix = [0.0f32; 2 * P];
        while self.frame < frame {
            let n = (P as u64).min(frame - self.frame) as usize;
            for j in 0..n {
                let g = self.frame + j as u64;
                let s = if self.live { sig(g) } else { 0.0 };
                inp[2 * j] = s;
                inp[2 * j + 1] = -s;
            }
            mix[..2 * n].fill(0.0);
            self.t.process(n as u32, &mut self.b);
            let map = *self.t.tempo_map();
            self.a
                .process(&self.b, &map, &inp[..2 * n], &mut mix[..2 * n]);
            self.out.extend_from_slice(&mix[..2 * n]);
            self.frame += n as u64;
        }
    }
}

/// Track 0 (mono) loop-records three passes of a bar, comped; track 2
/// (stereo) records one bar in a punch; mixes set.
fn session() -> Rig {
    let mut r = Rig::new(AudioTracks::new(RATE, 20));
    r.a.command(A::Arm(0, true));
    r.t.command(0, T::SetLoop(Some((0, BAR_TICKS))));
    r.t.command(START, T::Record);
    r.t.command(START + 3 * BAR, T::Stop);
    r.run_to(START + 3 * BAR + 200);
    r.a.command(A::Arm(0, false));
    r.a.command(A::Arm(2, true));
    r.a.command(A::SetInput(2, Input::Stereo));
    r.a.command(A::SetMonitor(2, Monitor::Off));
    let p = r.frame + 100;
    r.t.command(p, T::SetLoop(None));
    r.t.command(p, T::PlayStart);
    r.t.command(p + BAR, T::Record);
    r.t.command(p + 2 * BAR, T::Stop);
    r.run_to(p + 2 * BAR + 200);
    r.a.command(A::Arm(2, false));
    let first = r.a.takes().next().unwrap().id;
    r.a.command(A::Comp {
        take: first,
        from: 500,
        to: 2500,
    });
    r.a.command(A::SetGain {
        track: 0,
        at: 600,
        gain: 0.8,
    });
    r.a.command(A::SetPan(2, 0.4));
    r.a.command(A::SetLevel(0, 1.5));
    r.live = false;
    r.run_to(r.frame + 10);
    r
}

/// Save `r` as a project would: the text and a WAV a take, through bytes.
fn save(r: &Rig) -> (String, Vec<(String, Vec<u8>)>) {
    let mut s = r.a.save_file().expect("nothing records");
    let mut files = Vec::new();
    for k in &mut s.takes {
        let mut audio = vec![0.0f32; k.frames * k.channels];
        // A piece at a time, as the app would across periods.
        let mut at = 0;
        while at < k.frames {
            let end = (at + 1000).min(k.frames);
            let got =
                r.a.read_take(k.id, at, &mut audio[at * k.channels..end * k.channels])
                    .unwrap();
            assert_eq!(got, end - at);
            at = end;
        }
        k.file = format!("take {}.wav", k.id);
        files.push((k.file.clone(), wav(&audio, k.channels, r.a.rate())));
    }
    (to_text(&s), files)
}

fn open(text: &str, files: &[(String, Vec<u8>)]) -> Rig {
    let mut s = from_text(text).unwrap();
    let mut r = Rig::new(AudioTracks::new(RATE, 20));
    r.live = false;
    for k in s.takes.drain(..) {
        let bytes = &files.iter().find(|(n, _)| *n == k.file).unwrap().1;
        let (audio, ch, rate) = from_wav(bytes).unwrap();
        assert_eq!((ch, rate), (k.channels, RATE));
        let mut k = k;
        k.audio = audio;
        assert!(r.a.load_take(k).is_ok());
    }
    for t in &s.tracks {
        assert!(r.a.load_track(t));
    }
    r
}

#[test]
fn a_saved_session_opens_and_plays_the_same() {
    let mut a = session();
    let (text, files) = save(&a);
    assert_eq!(files.len(), 4);
    let mut b = open(&text, &files);
    // The loads copy in while stopped.
    let mut periods = 0;
    while b.a.loading() > 0 {
        b.run_to(b.frame + 256);
        periods += 1;
    }
    // 1 + 3 bars of mono and 1 bar of stereo... at LOAD_SPEED.
    assert!(
        periods < 4 * BAR as usize / (LOAD_SPEED * 256) + 8,
        "{periods}"
    );
    let mut n = 0;
    while b.a.take_returned().is_some() {
        n += 1;
    }
    assert_eq!(n, 4);
    for k in 0..4 {
        assert_eq!(a.a.regions(k), b.a.regions(k), "track {k}");
    }
    assert_eq!(
        a.a.takes().collect::<Vec<_>>(),
        b.a.takes().collect::<Vec<_>>()
    );
    // Both play the song from the start, the same to the sample.
    let (pa, pb) = (a.frame + 50, b.frame + 50);
    a.t.command(pa, T::PlayStart);
    b.t.command(pb, T::PlayStart);
    a.run_to(pa + 3 * BAR);
    b.run_to(pb + 3 * BAR);
    let sa = &a.out[2 * pa as usize..];
    let sb = &b.out[2 * pb as usize..];
    assert!(sa.iter().any(|&v| v != 0.0));
    assert!(sa == sb);
    // A saved file says the same again.
    assert_eq!(save(&b).0, text);
}

#[test]
fn a_take_plays_nothing_until_it_has_copied_in() {
    let a = session();
    let (text, files) = save(&a);
    let mut b = open(&text, &files);
    // Play at once: the first periods are silent on the loading takes.
    let p = b.frame;
    b.t.command(p, T::PlayStart);
    b.run_to(p + 256);
    assert!(b.a.loading() > 0);
    assert!(b.out[2 * p as usize..].iter().all(|&v| v == 0.0));
    assert!(b.a.save_file().is_none());
    // The last take loads last: it cannot be read yet.
    let last = b.a.takes().last().unwrap().id;
    assert_eq!(b.a.read_take(last, 0, &mut [0.0; 8]), None);
}

#[test]
fn bad_loads_are_refused() {
    let a = session();
    let s = a.a.save_file().unwrap();
    let mut k = s.takes[0].clone();
    k.audio = vec![0.0; k.frames * k.channels];
    // Into the tracks it came from: the id is in use.
    let mut a = a;
    assert!(a.a.load_take(k.clone()).is_err());
    let mut fresh = AudioTracks::new(RATE, 20);
    let mut short = k.clone();
    short.audio.pop();
    assert!(fresh.load_take(short).is_err());
    let mut far = k.clone();
    far.track = 8;
    assert!(fresh.load_take(far).is_err());
    // Too little memory.
    let mut tiny = AudioTracks::new(RATE, 0);
    let back = tiny.load_take(k.clone()).unwrap_err();
    assert_eq!(back.audio.len(), k.audio.len());
    assert_eq!(tiny.memory().used, 0);
    assert!(fresh.load_take(k).is_ok());
    // A track with regions out of order.
    let mut t = s.tracks[0].clone();
    t.regions.reverse();
    assert!(t.regions.len() < 2 || !fresh.load_track(&t));
}
