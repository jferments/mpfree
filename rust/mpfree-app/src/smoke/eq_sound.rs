//! The EQ's sound, measured: a finger drags a band's dot (on the Mix
//! tab's EQ look, and on the EQ's own page), then steady sines run
//! through the strip's chain as the engine holds it, and the gain each
//! one measures is checked against what the screen shows. Jay, 16 Nov:
//! "write automated tests that ensure that it follows it in exactly the
//! way that it was intended, by measuring the effect on the sound".
//!
//! Each test checks the whole loop, finger to sound:
//! - the dot ends where the finger lifted;
//! - the effect holds the values the screen draws;
//! - the sound follows the drawn curve, point for point, 20 Hz to
//!   20 kHz (within [`CURVE_DB`]);
//! - the boost or cut you hear at the dot's frequency is the dot's
//!   height, and an octave either side it is what the curve shows;
//! - the bell's middle, found from the sound alone, is the dot's
//!   frequency (within [`CENTRE_OCT`]).

#[cfg(test)]
mod tests {
    use super::super::{on_big_stack, Session};
    use crate::app::View;
    use mpfree_chain_ui::eq_page::{self, Band};
    use mpfree_chain_ui::{ChainView, CHAIN};
    use mpfree_fx::ids::eq as q;
    use mpfree_fx::mpfree_node::Ctx;
    use mpfree_fx::{gain_to_db, StripId};
    use mpfree_mixer_ui::eq_look::{self, Dot};
    use mpfree_mixer_ui::{MixerView, MIXER};
    use mpfree_ui::Touch;

    /// How far the sound may stray from the drawn curve, in dB (it
    /// measures within 0.01 dB; 0.1 dB is far under what a line on the
    /// screen can show).
    const CURVE_DB: f32 = 0.1;
    /// How far the bell's middle may sit from the dot, in octaves (an
    /// eighth of a semitone: about a pixel on the EQ page, under one on
    /// the Mix tab; it measures within 0.002).
    const CENTRE_OCT: f32 = 1.0 / 96.0;

    // ------------------------------------------------------ the sound

    /// The gain a steady sine at `hz` measures through strip `sid`'s
    /// chain (every insert on it, as the engine holds them), in dB: the
    /// sine runs a while to settle, then its level is read over a whole
    /// number of cycles.
    fn measure(s: &mut Session, sid: StripId, hz: f32) -> f32 {
        let chain = s.mix.mixer.chain_mut(sid).expect("the strip's chain");
        let rate = chain.rate();
        let block = chain.max_block().min(256);
        let settle = rate as usize / 10;
        let cycles = (hz * 0.12).ceil().max(3.0);
        let window = (cycles * rate as f32 / hz).round() as usize;
        let n = settle + window;
        let w = 2.0 * std::f64::consts::PI * f64::from(hz) / f64::from(rate);
        let x: Vec<f32> = (0..n)
            .map(|i| (0.1 * (w * i as f64).sin()) as f32)
            .collect();
        let (mut l, mut r) = (x.clone(), x.clone());
        let mut at = 0u64;
        for (a, b) in l.chunks_mut(block).zip(r.chunks_mut(block)) {
            chain.process(&Ctx::new(rate, at), a, b);
            at += a.len() as u64;
        }
        let rms = |v: &[f32]| {
            (v.iter().map(|y| f64::from(*y).powi(2)).sum::<f64>() / v.len() as f64).sqrt()
        };
        let (dry, wet, right) = (rms(&x[settle..]), rms(&l[settle..]), rms(&r[settle..]));
        assert!(
            (wet - right).abs() <= wet * 1e-4,
            "both sides alike at {hz} Hz: {wet} and {right}"
        );
        gain_to_db((wet / dry) as f32)
    }

    /// The frequencies a curve of `k` points is drawn at, 20 Hz to 20 kHz
    /// (both screens space them so).
    fn grid(k: usize) -> Vec<f32> {
        (0..k)
            .map(|i| eq_look::x_freq(i as f32 / (k - 1) as f32))
            .collect()
    }

    /// The sound against the drawn curve at every one of its points:
    /// returns the measured points, and fails at the first that strays.
    fn follows(s: &mut Session, sid: StripId, hz: &[f32], drawn: &[f32]) -> Vec<(f32, f32)> {
        hz.iter()
            .zip(drawn)
            .map(|(&f, &want)| {
                let got = measure(s, sid, f);
                assert!(
                    (got - want).abs() <= CURVE_DB,
                    "at {f:.0} Hz the curve shows {want:.2} dB, the sound measures {got:.2} dB"
                );
                (f, got)
            })
            .collect()
    }

    /// The middle of the bell, from the sound alone: halfway (in octaves)
    /// between where the boost or cut rises to half its peak and where it
    /// falls back below it.
    fn centre(points: &[(f32, f32)]) -> f32 {
        let (k, peak) = points
            .iter()
            .enumerate()
            .max_by(|a, b| a.1 .1.abs().total_cmp(&b.1 .1.abs()))
            .map(|(k, p)| (k, p.1.abs()))
            .expect("points");
        let half = peak / 2.0;
        let cross = |a: (f32, f32), b: (f32, f32)| {
            let t = (half - a.1.abs()) / (b.1.abs() - a.1.abs());
            a.0.log2() + t * (b.0.log2() - a.0.log2())
        };
        let lo = (1..=k)
            .rev()
            .find(|&i| points[i - 1].1.abs() < half)
            .map(|i| cross(points[i - 1], points[i]))
            .expect("the bell's low side in the sweep");
        let hi = (k..points.len() - 1)
            .find(|&i| points[i + 1].1.abs() < half)
            .map(|i| cross(points[i], points[i + 1]))
            .expect("the bell's high side in the sweep");
        2f32.powf((lo + hi) / 2.0)
    }

    /// Every value of insert `slot` on strip `sid`, as the effect holds
    /// them (index = parameter id).
    fn engine_values(s: &Session, sid: StripId, slot: usize) -> Vec<f32> {
        let n = s
            .mix
            .mixer
            .chain(sid)
            .and_then(|c| c.node(slot))
            .expect("the EQ on the strip");
        let top = n.params().iter().map(|p| p.id).max().unwrap_or(0);
        (0..=top).map(|id| n.param(id)).collect()
    }

    /// A finger down at `from`, moved to `to` in a few steps, lifted
    /// there; what the screen sends goes to the app. The touches all come
    /// in one read, as a fast drag's do on the MPC: their actions must
    /// keep their order (`Player::touch`), or an older move's frequency
    /// lands last and the dot jumps back.
    fn drag(s: &mut Session, from: (i32, i32), to: (i32, i32)) {
        let mut acts: Vec<crate::gui::FromUi> = s
            .screen
            .touch(9, Touch::Down, from.0, from.1)
            .into_iter()
            .collect();
        for k in 1..=4 {
            let x = from.0 + (to.0 - from.0) * k / 4;
            let y = from.1 + (to.1 - from.1) * k / 4;
            acts.extend(s.screen.touch(9, Touch::Move, x, y));
        }
        acts.extend(s.screen.touch(9, Touch::Up, to.0, to.1));
        acts.extend(s.screen.take_actions());
        for a in acts {
            s.route(a);
        }
        s.run_for(0.3);
    }

    /// Checks the bell a band makes: its gain at the dot, an octave either
    /// side, and its middle.
    fn check_bell(
        points: &[(f32, f32)],
        mut measured_at: impl FnMut(f32) -> f32,
        hz: f32,
        db: f32,
    ) {
        let at = measured_at(hz);
        assert!(
            (at - db).abs() <= CURVE_DB,
            "at the dot ({hz:.0} Hz) the sound is {at:.2} dB, the dot shows {db:.1} dB"
        );
        for f in [hz / 2.0, hz * 2.0] {
            let side = measured_at(f);
            assert!(
                side.abs() < at.abs() && side.signum() == at.signum(),
                "an octave off ({f:.0} Hz) {side:.2} dB, less than at the dot ({at:.2} dB)"
            );
        }
        let mid = centre(points);
        let off = (mid / hz).log2();
        assert!(
            off.abs() <= CENTRE_OCT,
            "the bell's middle is {mid:.1} Hz, {off:+.4} octaves from the dot's {hz:.1} Hz"
        );
    }

    // ------------------------------------------------- the Mix tab

    #[test]
    fn a_drag_on_the_eq_look_is_what_you_hear() {
        on_big_stack(|| {
            let drums = StripId::Track(0);
            let mut s = Session::new();
            s.press("header", "tab", "Mix");
            s.press("mixer", "look", "EQ");
            s.press("mixer", "eq-add", "Drums + EQ").run_for(0.3);
            let slot = 1;

            // A new EQ is flat: the strip sounds as it did.
            for f in [100.0, 400.0, 1_000.0, 5_000.0] {
                let g = measure(&mut s, drums, f);
                assert!(g.abs() < 0.05, "flat at {f} Hz: {g:.3} dB");
            }

            // Band 2's dot dragged up and right.
            let r = s.screen.ui.rect_of(MIXER).expect("the mixer");
            let dot = |s: &Session| {
                s.screen
                    .ui
                    .get::<MixerView>(MIXER)
                    .expect("the mixer")
                    .eq_dot_at(r, drums, Dot::Band(1))
                    .expect("band 2's dot")
            };
            let (x, y) = dot(&s);
            let to = (x + 60, y - 14);
            drag(&mut s, (x, y), to);
            let (dx, dy) = dot(&s);
            assert!(
                (dx - to.0).abs() <= 1 && (dy - to.1).abs() <= 1,
                "the dot is where the finger lifted: {:?}, finger {to:?}",
                (dx, dy)
            );

            // The screen draws what the effect holds.
            let shown = s
                .screen
                .ui
                .get::<MixerView>(MIXER)
                .and_then(|v| v.state().strip(drums).cloned())
                .and_then(|st| st.inserts.get(slot).and_then(|i| i.eq.clone()))
                .expect("the row's EQ values");
            let held = engine_values(&s, drums, slot);
            assert_eq!(shown[..held.len()], held[..], "screen and effect agree");
            let (hz, db) = (held[q::freq(1) as usize], held[q::gain(1) as usize]);
            assert!(
                hz > 450.0 && db > 3.0,
                "moved up and raised: {hz} Hz {db} dB"
            );

            // The sound follows the drawn curve, all the way across.
            let drawn = eq_look::curve(&shown);
            let points = follows(&mut s, drums, &grid(drawn.len()), &drawn);
            check_bell(&points, |f| measure(&mut s, drums, f), hz, db);
            assert_eq!(s.view(), View::Mixer);
        });
    }

    // -------------------------------------------------- the EQ page

    #[test]
    fn a_drag_on_the_eq_page_is_what_you_hear() {
        on_big_stack(|| {
            let drums = StripId::Track(0);
            let mut s = Session::new();
            s.press("header", "tab", "Mix");
            s.press("mixer", "look", "EQ");
            s.press("mixer", "eq-add", "Drums + EQ").run_for(0.3);
            let slot = 1;
            // A tap on the curve picks the EQ, a second opens its page.
            s.press("mixer", "eq", "Drums EQ").run_for(0.2);
            if s.view() != View::Chain {
                s.press("mixer", "eq", "Drums EQ").run_for(0.2);
            }
            assert_eq!(s.view(), View::Chain, "the EQ's page");

            let r = s.screen.ui.rect_of(CHAIN).expect("the chain editor");
            let page = |s: &Session| {
                let v = s.screen.ui.get::<ChainView>(CHAIN).expect("the editor");
                assert!(v.state().eq, "the EQ's page shows");
                let plot = eq_page::layout(v.layout(r).page, v.eq_page()).plot;
                let (fx, fy) = v.eq_page().dot(Band::Band(1));
                let at = (
                    (plot.x as f32 + fx * plot.w as f32).round() as i32,
                    (plot.bottom() as f32 - fy * plot.h as f32).round() as i32,
                );
                (plot, at)
            };

            // Band 2's dot dragged down and left: a cut, lower.
            let (plot, (x, y)) = page(&s);
            let to = (x - plot.w / 12, y + plot.h / 6);
            drag(&mut s, (x, y), to);
            let (plot, (dx, dy)) = page(&s);
            assert!(
                (dx - to.0).abs() <= 1 && (dy - to.1).abs() <= 1,
                "the dot is where the finger lifted: {:?}, finger {to:?}",
                (dx, dy)
            );

            // The page's dot stands for what the effect holds.
            let held = engine_values(&s, drums, slot);
            let (hz, db) = (held[q::freq(1) as usize], held[q::gain(1) as usize]);
            assert!(
                hz < 380.0 && db < -3.0,
                "moved down and cut: {hz} Hz {db} dB"
            );
            {
                let v = s.screen.ui.get::<ChainView>(CHAIN).expect("the editor");
                let (fx, _) = v.eq_page().dot(Band::Band(1));
                let px = (fx - eq_look::freq_x(hz)).abs() * plot.w as f32;
                assert!(
                    px < 0.5,
                    "the dot's x is the effect's frequency ({px} px off)"
                );
            }

            // The sound follows the page's curve, all the way across.
            let hzs = grid(160);
            let drawn: Vec<f32> = {
                let v = s.screen.ui.get::<ChainView>(CHAIN).expect("the editor");
                hzs.iter().map(|&f| v.eq_page().curve_db(f)).collect()
            };
            let points = follows(&mut s, drums, &hzs, &drawn);
            check_bell(&points, |f| measure(&mut s, drums, f), hz, db);
        });
    }
}
