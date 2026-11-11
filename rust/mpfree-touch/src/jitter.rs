//! M14: how much a held finger's reported position wanders.
//!
//! M12's 3 s hold produced only 12 samples, because the panel and the kernel
//! pass on a position only when it changes. A held finger that reports rarely
//! and wanders a pixel or two is fine for 60 px targets; a noisy one would
//! need a smoothing filter or a bigger slop. This summarises one hold so the
//! next device round can decide.

use crate::pointer::PointerEvent;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HoldStats {
    /// Position reports, including the Down.
    pub samples: usize,
    pub duration_us: u64,
    /// Reports per second while held.
    pub rate_hz: f32,
    /// Standard deviation of each axis, logical px.
    pub std_x: f32,
    pub std_y: f32,
    /// Largest distance from the median position, logical px.
    pub max_dev: f32,
}

/// Summarise one contact's Down/Move reports (other phases and other contacts
/// should be filtered out by the caller). `None` for fewer than two reports.
pub fn hold_stats(reports: &[PointerEvent]) -> Option<HoldStats> {
    if reports.len() < 2 {
        return None;
    }
    let n = reports.len() as f64;
    let mean = |f: fn(&PointerEvent) -> f32| reports.iter().map(|r| f64::from(f(r))).sum::<f64>() / n;
    let std = |f: fn(&PointerEvent) -> f32, m: f64| {
        (reports.iter().map(|r| (f64::from(f(r)) - m).powi(2)).sum::<f64>() / n).sqrt() as f32
    };
    let (mx, my) = (mean(|r| r.x), mean(|r| r.y));
    let median = |f: fn(&PointerEvent) -> f32| {
        let mut v: Vec<f32> = reports.iter().map(f).collect();
        v.sort_by(f32::total_cmp);
        v[v.len() / 2]
    };
    let (cx, cy) = (median(|r| r.x), median(|r| r.y));
    let max_dev = reports.iter().map(|r| (r.x - cx).hypot(r.y - cy)).fold(0.0, f32::max);
    let first = reports.iter().map(|r| r.time_us).min().unwrap_or(0);
    let last = reports.iter().map(|r| r.time_us).max().unwrap_or(0);
    let duration_us = last - first;
    let rate_hz = if duration_us > 0 { (reports.len() - 1) as f32 / (duration_us as f32 / 1e6) } else { 0.0 };
    Some(HoldStats { samples: reports.len(), duration_us, rate_hz, std_x: std(|r| r.x, mx), std_y: std(|r| r.y, my), max_dev })
}
