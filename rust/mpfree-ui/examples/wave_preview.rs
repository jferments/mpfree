//! Draws the sampler's Wave tab with the waveform widgets into a PNG, on the
//! workstation (no device): `cargo run -p mpfree-ui --example wave_preview -- OUT.png`.
//! The sound is a made-up two-bar drum loop at 92 BPM.

use std::sync::Arc;

use mpfree_ui::wave::{Beats, Marker, MarkerKind, Peaks, RawPeaks, WaveOverview, Waveform};
use mpfree_ui::widgets::{Chip, Fader, Header, Label, StatList};
use mpfree_ui::{Align, Fonts, SoftCanvas, Theme, Tone, Ui};

const RATE: f64 = 44_100.0;
const BPM: f64 = 92.0;

fn drums() -> Vec<f32> {
    let beat = RATE * 60.0 / BPM;
    let frames = (beat * 8.0) as usize;
    let mut out = vec![0.0f32; frames * 2];
    let mut seed = 0x2545_F491u32;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32 * 2.0 - 1.0
    };
    let mut hit = |at: f64, len_s: f64, level: f32, tone: Option<f32>, out: &mut Vec<f32>| {
        let start = at as usize;
        let len = (len_s * RATE) as usize;
        for i in 0..len.min(frames - start) {
            let t = i as f32 / RATE as f32;
            let env = (-t / (len_s as f32 / 4.0)).exp();
            let x = match tone {
                Some(hz) => (t * hz * std::f32::consts::TAU * (1.0 + 2.0 * (-t * 30.0).exp())).sin(),
                None => noise(),
            };
            out[(start + i) * 2] += level * env * x;
            out[(start + i) * 2 + 1] += level * env * x * 0.92;
        }
    };
    for k in 0..16 {
        let at = k as f64 * beat / 2.0;
        hit(at, 0.05, 0.22, None, &mut out); // hats on the eighths
        if k % 4 == 0 || k == 7 || k == 10 {
            hit(at, 0.35, 0.85, Some(58.0), &mut out); // kicks
        }
        if k % 4 == 2 {
            hit(at, 0.2, 0.6, None, &mut out); // snares
        }
    }
    out
}

fn main() -> Result<(), String> {
    let out = std::env::args().nth(1).ok_or("usage: wave_preview OUT.png")?;
    let data = drums();
    let frames = (data.len() / 2) as u64;
    let peaks: Arc<dyn Peaks> = Arc::new(RawPeaks::new(data.into(), 2).ok_or("bad sound")?);
    let beat = RATE * 60.0 / BPM;
    let at = |b: f64| (b * beat) as u64;

    let (w, h) = (1280, 800);
    let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), w, h);
    let mut header = Header::new(&["Record", "Sound", "Perform", "Browse"]);
    header.active = 1;
    // Chips are laid out from the right.
    header.chips = vec![Chip::new("Undo: 14 steps", Tone::Good), Chip::new("92.0 BPM", Tone::Normal), Chip::new("Pad A5", Tone::Accent)];
    ui.add("header", header);
    ui.add("overview", WaveOverview::new());
    ui.add("wave", Waveform::new());
    let mut region = StatList::new("Region");
    let row = |k: &str, v: &str| (k.to_string(), v.to_string(), Tone::Normal);
    region.set(vec![
        row("Mode", "Loop"),
        row("Gain", "+2.1 dB"),
        row("Reverse", "Off"),
        row("Channels", "Stereo"),
        row("Tune", "0 st, +0 ct"),
        row("Loop crossfade", "12 ms"),
        row("Fade in / out", "91 ms / 181 ms"),
        row("Snap", "Zero crossing"),
    ]);
    ui.add("region", region);
    for (i, (name, value, v)) in [("Q-Link 1 · Start", "1.1.2", 0.03), ("Q-Link 2 · End", "2.4.1", 0.91), ("Q-Link 3 · Loop start", "1.3.1", 0.25), ("Q-Link 4 · Loop end", "2.1.1", 0.5)].into_iter().enumerate() {
        let mut f = Fader::new(name);
        f.text = value.to_string();
        f.value = v;
        ui.add(&format!("q{}", i + 1), f);
    }
    ui.add("hint", Label::new("Pinch or turn the wheel to zoom · drag markers by their tabs · drag the wave to scroll · tap to play from there", Tone::Normal, Align::Center));
    ui.add_page("sound", "col(header:60, overview:64, row(wave:*, region:300):*, row(q1, q2, q3, q4):96, hint:22)")?;

    let wr = Waveform::wave_rect(ui.rect_of("wave").ok_or("no wave")?);
    {
        let wave = ui.get_mut::<Waveform>("wave").ok_or("no wave")?;
        wave.set_peaks(Some(peaks.clone()), wr.w);
        wave.set_markers(vec![
            Marker::new(at(0.25), "START", MarkerKind::Edge),
            Marker::new(at(2.0), "LOOP", MarkerKind::Loop),
            Marker::new(at(4.0), "LOOP END", MarkerKind::Loop),
            Marker::new(at(7.0), "END", MarkerKind::Edge),
        ]);
        wave.set_region(Some((at(0.25), at(7.0))));
        wave.set_loop(Some((at(2.0), at(4.0))));
        wave.set_fades(4_000, 8_000);
        wave.set_beats(Some(Beats { first: 0.0, frames_per_beat: beat, per_bar: 4 }));
        wave.zoom(1.25, wr.w);
        wave.set_playheads(&[at(3.1)]);
    }
    let view = ui.get::<Waveform>("wave").ok_or("no wave")?.view();
    let o = ui.get_mut::<WaveOverview>("overview").ok_or("no overview")?;
    o.set_peaks(Some(peaks));
    o.set_shown(view, wr.w);
    o.set_region(Some((at(0.25), at(7.0))));
    ui.show("sound")?;

    let mut c = SoftCanvas::new(w, h);
    ui.paint(&mut c);
    std::fs::write(&out, mpfree_ui::png::encode_rgb(w as u32, h as u32, &c.to_rgb())).map_err(|e| format!("{out}: {e}"))?;
    println!("wrote {out} ({frames} frames shown)");
    Ok(())
}
