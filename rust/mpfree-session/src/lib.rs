//! M7 synthetic callback-timing harness, exclusively NullAudioBackend.
//! It provides measurements of a userspace event-loop schedule, NOT ALSA.
//!
//! Only host/CI/offline execution is authorized initially. No device interface,
//! network socket, filesystem writes, process privileges or factory controls.
#![forbid(unsafe_code)]

use mpfree_headless::{
    HeadlessError, HeadlessSession, NullAudioBackend, SessionStats, BLOCK, SAMPLE_RATE,
    SELF_TEST_FRAMES,
};
use std::time::{Duration, Instant};

pub const EXPECTED_CALLBACKS: usize = (SELF_TEST_FRAMES as usize).div_ceil(BLOCK);
const NANOS_PER_SECOND: u128 = 1_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchmarkError {
    Engine(HeadlessError),
    ClockRegression,
}
impl From<HeadlessError> for BenchmarkError {
    fn from(value: HeadlessError) -> Self {
        Self::Engine(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SessionMetrics {
    pub audio_frames: u64,
    pub callback_count: u64,
    pub deadline_misses: u64,
    pub late_starts: u64,
    pub max_callback_ns: u64,
    pub p99_callback_ns: u64,
    pub max_start_lateness_ns: u64,
    pub total_render_ns: u64,
    pub wall_elapsed_ns: u64,
    pub audio: SessionStats,
}

fn ns_for_frame(frame: u64) -> u64 {
    ((u128::from(frame) * NANOS_PER_SECOND) / u128::from(SAMPLE_RATE)) as u64
}

fn elapsed_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

/// Perform a bounded, one-bar render with an optional wall-clock pacer.
///
/// A callback's synthetic release/deadline is based on absolute frame numbers
/// rather than a previously observed callback end, so delays cannot cause
/// cumulative frame-timing drift. Host scheduling and sleep accuracy vary.
/// The sink does not open any hardware resource.
pub fn run(paced: bool) -> Result<SessionMetrics, BenchmarkError> {
    let mut engine = HeadlessSession::new()?;
    let mut sink = NullAudioBackend::default();
    let origin = Instant::now();
    let mut callback_durations_ns = [0_u64; EXPECTED_CALLBACKS];
    let mut count = 0usize;
    let mut deadline_misses = 0u64;
    let mut late_starts = 0u64;
    let mut max_start_lateness_ns = 0u64;
    let mut total_render_ns = 0u64;

    while engine.frame() < SELF_TEST_FRAMES {
        let start_frame = engine.frame();
        let n = (SELF_TEST_FRAMES - start_frame).min(BLOCK as u64) as usize;
        let nominal_start = origin + Duration::from_nanos(ns_for_frame(start_frame));
        let nominal_end = origin + Duration::from_nanos(ns_for_frame(start_frame + n as u64));

        if paced {
            // This sleep happens *before*, never during, the render callback.
            if let Some(delay) = nominal_start.checked_duration_since(Instant::now()) {
                std::thread::sleep(delay);
            }
        }

        let started = Instant::now();
        if paced && started > nominal_start {
            late_starts += 1;
            max_start_lateness_ns =
                max_start_lateness_ns.max(elapsed_ns(started.duration_since(nominal_start)));
        }
        engine.render(n, &mut sink)?;
        let finished = Instant::now();
        let elapsed = elapsed_ns(finished.duration_since(started));
        callback_durations_ns[count] = elapsed;
        total_render_ns = total_render_ns.saturating_add(elapsed);
        if paced && finished > nominal_end {
            deadline_misses += 1;
        }
        count += 1;
    }

    callback_durations_ns[..count].sort_unstable();
    let percentile_99_index = ((count * 99).div_ceil(100)).saturating_sub(1);
    let audio = engine.completed(&sink);
    if audio.sink.frames != SELF_TEST_FRAMES
        || audio.groove_events != 12
        || audio.sampler_notes != 4
        || audio.groove_stolen_voices != 0
        || audio.sampler_stolen_voices != 0
        || count != EXPECTED_CALLBACKS
    {
        return Err(BenchmarkError::Engine(HeadlessError::InvalidOutput));
    }
    Ok(SessionMetrics {
        audio_frames: audio.sink.frames,
        callback_count: count as u64,
        deadline_misses,
        late_starts,
        max_callback_ns: callback_durations_ns[count - 1],
        p99_callback_ns: callback_durations_ns[percentile_99_index],
        max_start_lateness_ns,
        total_render_ns,
        wall_elapsed_ns: elapsed_ns(origin.elapsed()),
        audio,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_frame_boundaries_do_not_accumulate_integer_drift() {
        assert_eq!(ns_for_frame(0), 0);
        assert_eq!(ns_for_frame(44_100), 1_000_000_000);
        assert_eq!(ns_for_frame(88_200), 2_000_000_000);
        assert_eq!(EXPECTED_CALLBACKS, 690);
        assert_eq!(ns_for_frame(128), 2_902_494);
        assert_eq!(ns_for_frame(88_200) - ns_for_frame(88_072), 2_902_495);
    }

    #[test]
    fn finite_null_audio_test_does_not_use_wall_clock_sleep() {
        let out = run(false).unwrap();
        assert_eq!(out.audio_frames, SELF_TEST_FRAMES);
        assert_eq!(out.callback_count as usize, EXPECTED_CALLBACKS);
        assert_eq!(out.deadline_misses, 0); // no pacing = no real-time claim
        assert_eq!(out.late_starts, 0);
        assert_eq!(out.audio.groove_events, 12);
        assert_eq!(out.audio.sampler_notes, 4);
        assert_eq!(
            out.audio.sink.checksum_fnv1a64, 0x3dc4d6dabbb17fbd,
            "the device-verified M6 PCM hash is a deterministic regression reference"
        );
    }
}
