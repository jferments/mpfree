//! A region's loudest frame (`Region::peak`), what Normalize brings to
//! 0 dBFS: only between its start and end, and only on the channels it
//! plays.

use mpfree_sampler::model::*;

const RATE: u32 = 44_100;

/// Stereo: left peaks at 0.5 (frame 100), right at -0.8 (frame 300),
/// and frame 500 is (0.4, 0.4), whose mono sum is the loudest.
fn stereo() -> Source {
    let mut data = vec![0.0f32; 1000 * 2];
    data[100 * 2] = 0.5;
    data[300 * 2 + 1] = -0.8;
    data[500 * 2] = 0.45;
    data[500 * 2 + 1] = 0.45;
    data[900 * 2] = 0.95;
    Source::new("stereo", RATE, 2, data).unwrap()
}

#[test]
fn the_peak_is_the_loudest_frame_on_the_channels_the_region_plays() {
    let s = stereo();
    let mut r = Region::whole(&s);
    r.end = 800;
    let peak = |r: &Region| r.peak(&s);
    assert_eq!(peak(&r), 0.8, "stereo: the louder side, by size");
    r.channels = ChannelPick::Left;
    assert_eq!(peak(&r), 0.5);
    r.channels = ChannelPick::Right;
    assert_eq!(peak(&r), 0.8);
    r.channels = ChannelPick::MonoSum;
    assert_eq!(peak(&r), 0.45, "(0.45 + 0.45) / 2 beats (0 - 0.8) / 2");
    // Only inside the region: frame 900 is past its end, frame 100
    // before its start.
    r.channels = ChannelPick::Stereo;
    r.start = 200;
    r.end = 1000;
    assert_eq!(peak(&r), 0.95);
    r.end = 400;
    assert_eq!(peak(&r), 0.8);
    r.start = 301;
    assert_eq!(peak(&r), 0.0, "silence");
    // Its gain, fades and reverse don't count.
    let mut g = Region::whole(&s);
    g.gain_db = -12.0;
    g.reverse = true;
    g.fade_in.frames = 950;
    assert_eq!(g.peak(&s), 0.95);
}

#[test]
fn a_mono_sound_peaks_on_its_one_channel_whatever_the_pick() {
    let mut data = vec![0.0f32; 64];
    data[10] = -0.25;
    data[20] = 0.125;
    let s = Source::new("mono", RATE, 1, data).unwrap();
    let mut r = Region::whole(&s);
    for pick in [
        ChannelPick::Stereo,
        ChannelPick::MonoSum,
        ChannelPick::Left,
        ChannelPick::Right,
    ] {
        r.channels = pick;
        assert_eq!(r.peak(&s), 0.25, "{pick:?}");
    }
    // A region past the sound's end reads what there is.
    r.start = 15;
    r.end = 100;
    assert_eq!(r.peak(&s), 0.125);
    r.start = 200;
    assert_eq!(r.peak(&s), 0.0);
}
