#include "mpfree/ClickEngine.hpp"

#include <algorithm>
#include <cmath>
#include <limits>

namespace mpfree {
namespace {
constexpr double kPi = 3.141592653589793238462643383279502884;
}

ClickEngine::ClickEngine() noexcept {
    (void)configure(ClickConfig{});
}

bool ClickEngine::configure(const ClickConfig& candidate) noexcept {
    if (candidate.sample_rate < 8000 || candidate.sample_rate > 192000 ||
        !std::isfinite(candidate.bpm) || candidate.bpm < 20.0 ||
        candidate.bpm > 400.0 || candidate.beats_per_bar == 0 ||
        candidate.beats_per_bar > 16) {
        return false;
    }

    config_ = candidate;
    // Generate a short biphasic decaying click OFF the audio thread.
    const auto length = static_cast<std::size_t>(
        std::llround(static_cast<double>(config_.sample_rate) * 0.0035));
    click_length_ = std::clamp(length, std::size_t{8}, kMaxClickSamples);
    for (std::size_t i = 0; i < click_length_; ++i) {
        const double t = static_cast<double>(i) /
                         static_cast<double>(config_.sample_rate);
        const double decay = std::exp(-7.0 * static_cast<double>(i) /
                                      static_cast<double>(click_length_));
        normal_[i] = static_cast<float>(
            0.30 * decay * std::sin(2.0 * kPi * 1550.0 * t));
        accent_[i] = static_cast<float>(
            0.47 * decay * std::sin(2.0 * kPi * 2250.0 * t));
    }
    reset();
    return true;
}

void ClickEngine::reset() noexcept {
    frame_ = 0;
    beat_ = 0;
    next_beat_frame_ = 0;
    click_position_ = click_length_;  // silent before initial beat
    accented_ = false;
}

void ClickEngine::render(float* left, float* right,
                         const std::size_t frames) noexcept {
    if (!left || !right) {
        return;
    }

    for (std::size_t i = 0; i < frames; ++i) {
        if (frame_ == next_beat_frame_) {
            accented_ = (beat_ % config_.beats_per_bar) == 0;
            click_position_ = 0;
            ++beat_;

            // Absolute time from integer beat index, not repeated rounded
            // period additions: prevents per-beat rounding drift.
            const long double next = static_cast<long double>(beat_) *
                                     60.0L *
                                     static_cast<long double>(config_.sample_rate) /
                                     static_cast<long double>(config_.bpm);
            const auto next_frame = static_cast<std::uint64_t>(next + 0.5L);
            next_beat_frame_ = next_frame > frame_ ? next_frame : frame_ + 1;
        }

        float value = 0.0f;
        if (click_position_ < click_length_) {
            value = accented_ ? accent_[click_position_]
                              : normal_[click_position_];
            ++click_position_;
        }
        left[i] = value;
        right[i] = value;
        ++frame_;
    }
}

std::int32_t float_to_s32(const float sample) noexcept {
    if (!std::isfinite(sample)) return 0;
    if (sample >= 1.0f) return std::numeric_limits<std::int32_t>::max();
    if (sample <= -1.0f) return std::numeric_limits<std::int32_t>::min();
    return static_cast<std::int32_t>(
        std::llround(static_cast<double>(sample) * 2147483647.0));
}

}  // namespace mpfree
