//! Report cards from a recording of the programme: the named numbers
//! [`crate::card::measure`] reads from a device in hand, read instead from a
//! capture cut into segments by its schedule. This is how an AIR effect
//! recorded on the factory MPC is measured, and our engines are rendered
//! through the same programme and the same code ([`render`]), so the two are
//! compared like for like.
//!
//! The capture is lined up on the programme's sync chirp, which went through
//! the effect too, so a plain latency drops out. `gain_db` is the gain the
//! Source applied to the whole programme (a level context); input levels in
//! the metric names include it.

use crate::card::{label, Card};
use crate::dynamics;
use crate::harmonics;
use crate::loudness;
use crate::programme::{self, Entry, Programme, Schedule, Stim};
use crate::response;
use crate::room;
use crate::signals::{coherent, db_to_amp, level_steps, pink};
use crate::wav::Stereo;

/// What to read from a capture beyond what every effect gets.
#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// The gain the Source applied to the programme, dB.
    pub gain_db: f64,
    /// Decay, density and width from the impulse (delays and reverbs).
    pub reverb: bool,
    /// Attack and release from the level steps (dynamics).
    pub dynamics: bool,
}

/// Measures one capture's segments (as [`programme::cut`] returns them).
pub fn analyse(
    effect: &str,
    name: &str,
    pieces: &[(Entry, Stereo)],
    rate: u32,
    opts: Options,
) -> Card {
    let mut card = Card {
        effect: effect.to_string(),
        name: name.to_string(),
        preset: None,
        rate,
        metrics: Vec::new(),
        notes: Vec::new(),
    };
    let g = db_to_amp(opts.gain_db);
    let mut swept = false;
    for (e, piece) in pieces {
        let Some(stim) = &e.stim else {
            continue;
        };
        if piece.len() < e.len {
            card.notes.push(format!("{}: capture ends early", e.name));
            continue;
        }
        if piece
            .left
            .iter()
            .chain(&piece.right)
            .any(|v| !v.is_finite())
        {
            card.notes.push(format!("{}: not finite", e.name));
            continue;
        }
        match stim {
            Stim::Sweep { level_db } if !swept => {
                // The quietest sweep comes first in every programme; it is
                // the one that stays near linear in a drive.
                swept = true;
                let s = response::sweep(*level_db, rate);
                let r = response::analyse(&s, piece, 16_384);
                card.put("measured_delay_frames", r.delay as f64);
                for hz in [
                    31.5, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 12_500.0,
                    16_000.0, 20_000.0,
                ] {
                    if let Some(p) = r.points.iter().find(|p| p.0 == hz) {
                        card.put(format!("resp_{}_db", label(hz)), p.1 - opts.gain_db);
                    }
                }
            }
            Stim::Sine { hz, level_db } => {
                let (bin, _) = coherent(*hz, harmonics::N, rate);
                let settle = e.len.saturating_sub(harmonics::N);
                let level = level_db + opts.gain_db;
                let h = harmonics::analyse(piece, bin, settle, level);
                let tag = format!("{}_{}", label(*hz), level.round() as i32);
                card.put(format!("fund_{tag}_db"), h.out_db);
                card.put(format!("thd_{tag}_db"), h.thd_db);
                card.put(format!("alias_{tag}_db"), h.alias_worst_db);
                // The same alias in dBFS: the relative figure says nothing when
                // the effect removes the fundamental (an octave up, a filter).
                card.put(format!("aliasfs_{tag}_db"), h.out_db + h.alias_worst_db);
                card.put(format!("other_{tag}_db"), h.other_total_db);
            }
            Stim::Pink { rms_db } => {
                let x: Vec<f64> = pink(e.len, db_to_amp(*rms_db), 11)
                    .iter()
                    .map(|v| v * g)
                    .collect();
                let body = piece.slice(0, e.len);
                let stim = Stereo::mono(x, rate);
                card.put(
                    "lufs_delta_pink",
                    loudness::integrated(&body) - loudness::integrated(&stim),
                );
                card.put("true_peak_pink_dbtp", loudness::true_peak(&body));
            }
            Stim::Impulse if opts.reverb => {
                for b in room::decay(piece) {
                    if [500.0, 1000.0, 2000.0, 4000.0].contains(&b.hz) {
                        if let Some(t) = b.t30.or(b.t20) {
                            card.put(format!("t30_{}_s", label(b.hz)), t);
                        }
                    }
                    if b.hz == 1000.0 {
                        if let Some(t) = b.edt {
                            card.put("edt_1k_s", t);
                        }
                    }
                }
                if let Some(t) = room::time_to_diffuse(piece) {
                    card.put("diffuse_ms", t * 1000.0);
                }
                if let Some(x) = room::tail_excess_db(piece) {
                    card.put("tail_excess_db", x);
                }
                card.put("iacc_tail", room::iacc(piece, rate as usize / 10));
            }
            Stim::Steps {
                low_db,
                high_db,
                secs,
            } if opts.dynamics => {
                let step = (secs * rate as f64) as usize;
                let x: Vec<f64> = level_steps(1000.0, *low_db, *high_db, step, 3, rate)
                    .iter()
                    .map(|v| v * g)
                    .collect();
                let t = dynamics::timing_of(&x, &piece.left, step, rate);
                card.put("step_depth_db", t.depth_db);
                if let Some(v) = t.attack_ms {
                    card.put("attack_ms", v);
                }
                if let Some(v) = t.release_ms {
                    card.put("release_ms", v);
                }
            }
            Stim::Silence => {
                let half = piece.slice(piece.len() / 2, piece.len() - piece.len() / 2);
                let rms = (half
                    .left
                    .iter()
                    .chain(&half.right)
                    .map(|v| v * v)
                    .sum::<f64>()
                    / (half.len() * 2).max(1) as f64)
                    .sqrt();
                card.put("idle_db", 20.0 * rms.max(1e-15).log10());
            }
            Stim::Clip { file } => {
                let body = piece.slice(0, e.len);
                card.put(format!("clip_{file}_lufs"), loudness::integrated(&body));
                card.put(
                    format!("clip_{file}_true_peak_dbtp"),
                    loudness::true_peak(&body),
                );
            }
            _ => {}
        }
    }
    card
}

/// Finds the programme in a capture and measures it.
pub fn analyse_recording(
    effect: &str,
    name: &str,
    rec: &Stereo,
    schedule: &Schedule,
    opts: Options,
) -> Card {
    let at = programme::locate(rec, 10.0);
    let pieces = programme::cut(rec, schedule, at);
    let mut c = analyse(effect, name, &pieces, rec.rate, opts);
    c.put("programme_at_frame", at as f64);
    c
}

/// Plays `programme` (scaled by `gain_db`) through a device with no
/// alignment step needed, latency removed, as a capture would hold it.
pub fn render(
    dut: &mut dyn crate::dut::Dut,
    programme: &Programme,
    gain_db: f64,
) -> (Stereo, Schedule) {
    let (audio, sched) = programme.render();
    let g = db_to_amp(gain_db);
    let scaled = Stereo::new(
        audio.left.iter().map(|v| v * g).collect(),
        audio.right.iter().map(|v| v * g).collect(),
        audio.rate,
    );
    let lat = dut.latency();
    let out = dut.render(&scaled, lat + audio.rate as usize);
    (out.slice(lat, out.len() - lat), sched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dut::{NodeDut, Wire};

    #[test]
    fn a_wire_through_the_short_programme_is_flat_clean_and_level() {
        let p = Programme::short(44_100);
        let (rec, sched) = render(&mut Wire { rate: 44_100 }, &p, -6.0);
        let c = analyse_recording(
            "wire",
            "wire",
            &rec,
            &sched,
            Options {
                gain_db: -6.0,
                ..Options::default()
            },
        );
        assert!(c.notes.is_empty(), "{:?}", c.notes);
        assert_eq!(c.get("programme_at_frame"), Some(0.0));
        assert!(c.get("resp_1k_db").unwrap().abs() < 0.01, "{c:?}");
        assert!(c.get("resp_16k_db").unwrap().abs() < 0.05, "{c:?}");
        assert!(c.get("thd_1k_-24_db").unwrap() < -140.0, "{c:?}");
        assert!(c.get("lufs_delta_pink").unwrap().abs() < 0.01, "{c:?}");
        assert!(c.get("idle_db").unwrap() < -200.0);
    }

    #[test]
    fn a_capture_agrees_with_the_card_measured_directly() {
        // Drive at its defaults: the 5 kHz full-scale alias figure is the
        // one the A/B depends on, and must not depend on the route.
        let entry = mpfree_fx::EFFECTS
            .iter()
            .find(|e| e.id == "mpfree.fx.drive")
            .unwrap();
        let direct = crate::card::measure(
            entry,
            None,
            crate::card::Options {
                rate: 44_100,
                quick: true,
            },
        );
        let mut d = NodeDut::from_registry("mpfree.fx.drive", 44_100).unwrap();
        let (rec, sched) = render(&mut d, &Programme::short(44_100), 0.0);
        let c = analyse_recording("drive", "drive", &rec, &sched, Options::default());
        let (a, b) = (
            direct.get("alias_5k_0_db").unwrap(),
            c.get("alias_5k_0_db").unwrap(),
        );
        assert!((a - b).abs() < 1.0, "direct {a:.1} dB, capture {b:.1} dB");
        let (a, b) = (
            direct.get("thd_1k_-18_db").unwrap(),
            c.get("thd_1k_-18_db").unwrap(),
        );
        assert!((a - b).abs() < 1.0, "direct {a:.1} dB, capture {b:.1} dB");
    }
}
