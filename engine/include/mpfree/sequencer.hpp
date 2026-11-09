#pragma once

// mpfree native sequencer: portable, allocation-free event scheduling.
// No ALSA, MPC firmware, proprietary SDK, or graphics dependencies.
// This is an early engine primitive, NOT a complete audio workstation.

#include <array>
#include <cstddef>
#include <cstdint>
#include <span>
#include <vector>

namespace mpfree {

constexpr std::size_t kMaxParameterLocks = 4;
constexpr std::int32_t kMaxMicrotimingFrames = 2048;
constexpr std::uint8_t kMaxRatchets = 8;

struct ParameterLock {
    std::uint16_t parameter_id = 0;
    float value = 0.0f;
};

struct Step {
    bool enabled = false;
    std::uint8_t velocity = 100;
    std::uint8_t probability = 100;     // 0..100, deterministic per occurrence
    std::uint8_t ratchets = 1;          // 1..8 subdivisions of this step
    std::uint8_t every_n_cycles = 1;    // 1=always, 2=every second clip cycle
    std::int32_t microtiming_frames = 0; // -2048..+2048, exact sample offset
    std::uint8_t lock_count = 0;
    std::array<ParameterLock, kMaxParameterLocks> locks{};
};

struct Track {
    // A track cycles through its own steps independently of other tracks.
    std::uint32_t steps_per_beat = 4; // 4 = 16th notes in 4/4
    std::vector<Step> steps;
};

struct Song {
    double tempo_bpm = 120.0;
    std::uint32_t sample_rate = 48000;
    std::uint64_t probability_seed = 0;
    std::vector<Track> tracks;
};

struct Event {
    std::int64_t frame = 0;          // absolute sample-frame timestamp
    std::uint32_t offset = 0;        // offset from beginning of audio block
    std::uint32_t track_index = 0;
    std::uint32_t step_index = 0;
    std::uint8_t ratchet_index = 0;
    std::uint8_t velocity = 0;
    std::uint8_t lock_count = 0;
    std::array<ParameterLock, kMaxParameterLocks> locks{};
};

struct ScheduleResult {
    std::size_t count = 0;
    std::size_t dropped = 0; // caller supplied insufficient output capacity
};

// Writes chronologically ordered events into the caller's preallocated span.
// No heap allocation, locks, I/O, logging, or state mutation inside this call.
// Callers must keep Song and all Track/Step arrays immutable while scheduling.
// Invalid tempo/track/step values are ignored. Frames before zero are ignored.
//
// The function is intended as a deterministic reference for an eventual audio
// callback; benchmark on actual MPC One hardware before treating it as RT safe.
[[nodiscard]] ScheduleResult schedule(
    const Song& song,
    std::int64_t block_start,
    std::uint32_t block_frames,
    std::span<Event> output) noexcept;

} // namespace mpfree
