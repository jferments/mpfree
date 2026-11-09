#pragma once

#include <array>
#include <cstddef>
#include <cstdint>

namespace mpfree {

// Pure, device-independent audio renderer. Owns no ALSA/MIDI/device handles.
// CONFIGURATION MUST NOT RUN CONCURRENTLY WITH render(). Render never allocates.
struct ClickConfig {
    std::uint32_t sample_rate = 44100;
    double bpm = 120.0;
    std::uint32_t beats_per_bar = 4;
};

class ClickEngine final {
public:
    ClickEngine() noexcept;

    // Non-real-time setup: computes click wavetable, resets transport to frame 0.
    // Returns false for invalid configuration without changing the running state.
    bool configure(const ClickConfig& config) noexcept;

    // Real-time method. Writes exactly frames samples to each planar float buffer.
    // No allocation, I/O, locks, logging, syscalls or trig functions in this method.
    // Left/right must be non-null and each point to at least frames float samples.
    // On null pointers, returns without advancing transport.
    void render(float* left, float* right, std::size_t frames) noexcept;

    // Non-realtime control method; not safe concurrent with render().
    void reset() noexcept;

    [[nodiscard]] std::uint64_t rendered_frames() const noexcept { return frame_; }
    [[nodiscard]] std::uint64_t beats_emitted() const noexcept { return beat_; }
    [[nodiscard]] ClickConfig configuration() const noexcept { return config_; }

private:
    static constexpr std::size_t kMaxClickSamples = 512;

    ClickConfig config_{};
    std::array<float, kMaxClickSamples> normal_{};
    std::array<float, kMaxClickSamples> accent_{};
    std::size_t click_length_ = 0;
    std::size_t click_position_ = 0;
    bool accented_ = false;
    std::uint64_t frame_ = 0;
    std::uint64_t beat_ = 0;
    std::uint64_t next_beat_frame_ = 0;
};

// Conversion for a later ACVA S32_LE backend and the host-only WAV test renderer.
// Endianness is encoded by a file/device adapter; this returns a signed value.
std::int32_t float_to_s32(float sample) noexcept;

}  // namespace mpfree
