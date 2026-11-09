// Host-only MPFree demo. Renders a deterministic test metronome to a WAV file.
// No MPC SSH, ALSA, MIDI, filesystem writes on the MPC, or device access.
#include "mpfree/ClickEngine.hpp"

#include <algorithm>
#include <array>
#include <cmath>
#include <cstdint>
#include <fstream>
#include <iostream>
#include <stdexcept>
#include <string>

namespace {
void u16le(std::ostream& out, std::uint16_t value) {
    for (int i = 0; i < 2; ++i)
        out.put(static_cast<char>((value >> (i * 8)) & 0xff));
}
void u32le(std::ostream& out, std::uint32_t value) {
    for (int i = 0; i < 4; ++i)
        out.put(static_cast<char>((value >> (i * 8)) & 0xff));
}
void pcm32le(std::ostream& out, float value) {
    const auto sample = static_cast<std::uint32_t>(mpfree::float_to_s32(value));
    u32le(out, sample);
}
}  // namespace

int main(int argc, char** argv) {
    if (argc < 2 || argc > 4) {
        std::cerr << "Usage: mpfree-render <output.wav> [seconds=4] [bpm=120]\n"
                  << "Host-only: writes a test file; never interacts with MPC hardware.\n";
        return 2;
    }

    try {
        const double seconds = argc > 2 ? std::stod(argv[2]) : 4.0;
        const double bpm = argc > 3 ? std::stod(argv[3]) : 120.0;
        if (!std::isfinite(seconds) || seconds <= 0.0 || seconds > 60.0)
            throw std::invalid_argument("seconds must be > 0 and <= 60");
        mpfree::ClickEngine engine;
        if (!engine.configure({44100, bpm, 4}))
            throw std::invalid_argument("tempo must be in [20,400] BPM");

        constexpr std::uint32_t rate = 44100;
        constexpr std::uint32_t channels = 2;
        constexpr std::uint32_t bytes_per_sample = 4;
        const auto frames = static_cast<std::uint32_t>(
            std::llround(seconds * static_cast<double>(rate)));
        const std::uint32_t data_bytes = frames * channels * bytes_per_sample;

        std::ofstream out(argv[1], std::ios::binary | std::ios::trunc);
        if (!out) throw std::runtime_error("cannot open output WAV");
        out.write("RIFF", 4);
        u32le(out, 36 + data_bytes);
        out.write("WAVEfmt ", 8);
        u32le(out, 16);                 // PCM fmt chunk size
        u16le(out, 1);                  // integer PCM
        u16le(out, channels);
        u32le(out, rate);
        u32le(out, rate * channels * bytes_per_sample); // byte rate
        u16le(out, channels * bytes_per_sample);        // block alignment
        u16le(out, 32);                 // bits/sample
        out.write("data", 4);
        u32le(out, data_bytes);

        std::array<float, 128> left{}, right{};
        std::uint32_t remaining = frames;
        while (remaining > 0) {
            const auto block = std::min<std::uint32_t>(remaining, 128);
            engine.render(left.data(), right.data(), block);
            for (std::uint32_t i = 0; i < block; ++i) {
                pcm32le(out, left[i]);
                pcm32le(out, right[i]);
            }
            remaining -= block;
        }
        if (!out) throw std::runtime_error("WAV write failed");
        std::cout << "Rendered " << engine.rendered_frames()
                  << " frames; " << engine.beats_emitted()
                  << " beat triggers; 44,100 Hz stereo S32_LE WAV: "
                  << argv[1] << '\n';
        return 0;
    } catch (const std::exception& e) {
        std::cerr << "mpfree-render: " << e.what() << '\n';
        return 1;
    }
}
