#include "mpfree/sequencer.hpp"

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <limits>

namespace mpfree {
namespace {

[[nodiscard]] std::uint64_t mix64(std::uint64_t value) noexcept {
    // SplitMix64 finalizer. Deterministic and independent of block boundaries.
    value ^= value >> 30;
    value *= UINT64_C(0xbf58476d1ce4e5b9);
    value ^= value >> 27;
    value *= UINT64_C(0x94d049bb133111eb);
    return value ^ (value >> 31);
}

[[nodiscard]] bool should_trigger(
    const Song& song,
    std::size_t track_index,
    std::int64_t absolute_step,
    std::uint8_t probability) noexcept {
    if (probability == 0) return false;
    if (probability >= 100) return true;
    const auto key =
        song.probability_seed ^
        mix64(static_cast<std::uint64_t>(track_index) + UINT64_C(0x9e3779b97f4a7c15)) ^
        mix64(static_cast<std::uint64_t>(absolute_step) + UINT64_C(0x632be59bd9b4e019));
    return (mix64(key) % 100) < probability;
}

[[nodiscard]] bool earlier(const Event& a, const Event& b) noexcept {
    if (a.frame != b.frame) return a.frame < b.frame;
    if (a.track_index != b.track_index) return a.track_index < b.track_index;
    if (a.step_index != b.step_index) return a.step_index < b.step_index;
    return a.ratchet_index < b.ratchet_index;
}

void insert_ordered(
    const Event& event,
    std::span<Event> output,
    ScheduleResult& result) noexcept {
    if (result.count == output.size()) {
        ++result.dropped;
        return;
    }
    std::size_t position = result.count;
    while (position > 0 && earlier(event, output[position - 1])) {
        output[position] = output[position - 1];
        --position;
    }
    output[position] = event;
    ++result.count;
}

} // namespace

ScheduleResult schedule(
    const Song& song,
    std::int64_t block_start,
    std::uint32_t block_frames,
    std::span<Event> output) noexcept {
    ScheduleResult result{};
    if (block_start < 0 || block_frames == 0 ||
        song.sample_rate == 0 || !std::isfinite(song.tempo_bpm) ||
        song.tempo_bpm <= 0.0 ||
        block_start > std::numeric_limits<std::int64_t>::max() - block_frames) {
        return result;
    }

    const auto block_end = block_start + block_frames; // exclusive
    for (std::size_t track_index = 0; track_index < song.tracks.size(); ++track_index) {
        if (track_index > std::numeric_limits<std::uint32_t>::max()) break;
        const Track& track = song.tracks[track_index];
        if (track.steps.empty() || track.steps_per_beat == 0 ||
            track.steps.size() > std::numeric_limits<std::uint32_t>::max()) {
            continue;
        }

        const long double frames_per_step =
            (60.0L * static_cast<long double>(song.sample_rate)) /
            (static_cast<long double>(song.tempo_bpm) *
             static_cast<long double>(track.steps_per_beat));
        if (!std::isfinite(frames_per_step) || frames_per_step < 1.0L) {
            // This version requires at least one frame per step.
            continue;
        }

        // Include neighboring steps: ratchets may occupy most of a step and
        // microtiming may move events across an audio block boundary.
        const long double lower =
            (static_cast<long double>(block_start) - kMaxMicrotimingFrames) /
            frames_per_step;
        const long double upper =
            (static_cast<long double>(block_end) + kMaxMicrotimingFrames) /
            frames_per_step;
        const auto first = static_cast<std::int64_t>(
            std::max(0.0L, std::floor(lower) - 2.0L));
        const auto last = static_cast<std::int64_t>(std::ceil(upper) + 1.0L);

        for (std::int64_t absolute_step = first; absolute_step <= last; ++absolute_step) {
            const auto step_index = static_cast<std::size_t>(
                absolute_step % static_cast<std::int64_t>(track.steps.size()));
            const Step& step = track.steps[step_index];
            if (!step.enabled || step.velocity == 0 || step.velocity > 127 ||
                step.probability > 100 || step.ratchets == 0 ||
                step.ratchets > kMaxRatchets || step.every_n_cycles == 0 ||
                step.lock_count > kMaxParameterLocks ||
                step.microtiming_frames < -kMaxMicrotimingFrames ||
                step.microtiming_frames > kMaxMicrotimingFrames) {
                continue;
            }

            const auto cycle = absolute_step /
                static_cast<std::int64_t>(track.steps.size());
            if (cycle % step.every_n_cycles != 0 ||
                !should_trigger(song, track_index, absolute_step, step.probability)) {
                continue;
            }

            for (std::uint8_t ratchet = 0; ratchet < step.ratchets; ++ratchet) {
                const long double step_position =
                    static_cast<long double>(absolute_step) +
                    static_cast<long double>(ratchet) / step.ratchets;
                const auto frame = static_cast<std::int64_t>(
                    std::llround(step_position * frames_per_step)) +
                    step.microtiming_frames;
                if (frame < block_start || frame >= block_end || frame < 0) continue;

                Event event{};
                event.frame = frame;
                event.offset = static_cast<std::uint32_t>(frame - block_start);
                event.track_index = static_cast<std::uint32_t>(track_index);
                event.step_index = static_cast<std::uint32_t>(step_index);
                event.ratchet_index = ratchet;
                event.velocity = step.velocity;
                event.lock_count = step.lock_count;
                event.locks = step.locks;
                insert_ordered(event, output, result);
            }
        }
    }
    return result;
}

} // namespace mpfree
