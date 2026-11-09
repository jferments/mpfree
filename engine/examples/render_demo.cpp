#include "mpfree/sequencer.hpp"

#include <algorithm>
#include <array>
#include <cmath>
#include <cstdint>
#include <fstream>
#include <iostream>
#include <numbers>
#include <string>
#include <vector>

namespace {

void write_u16(std::ofstream& output, std::uint16_t value) {
    const std::array<char, 2> bytes{
        static_cast<char>(value & 0xff),
        static_cast<char>((value >> 8) & 0xff),
    };
    output.write(bytes.data(), bytes.size());
}

void write_u32(std::ofstream& output, std::uint32_t value) {
    const std::array<char, 4> bytes{
        static_cast<char>(value & 0xff),
        static_cast<char>((value >> 8) & 0xff),
        static_cast<char>((value >> 16) & 0xff),
        static_cast<char>((value >> 24) & 0xff),
    };
    output.write(bytes.data(), bytes.size());
}

bool write_mono_wav(
    const std::string& path,
    const std::vector<float>& samples,
    std::uint32_t sample_rate) {
    std::ofstream output(path, std::ios::binary);
    if (!output) return false;

    const auto data_size = static_cast<std::uint32_t>(samples.size() * 2);
    output.write("RIFF", 4);
    write_u32(output, 36 + data_size);
    output.write("WAVE", 4);
    output.write("fmt ", 4);
    write_u32(output, 16);
    write_u16(output, 1); // PCM
    write_u16(output, 1); // mono
    write_u32(output, sample_rate);
    write_u32(output, sample_rate * 2);
    write_u16(output, 2); // block alignment
    write_u16(output, 16);
    output.write("data", 4);
    write_u32(output, data_size);

    for (const float sample : samples) {
        const double clamped = std::clamp(static_cast<double>(sample), -1.0, 1.0);
        const auto pcm = static_cast<std::int16_t>(std::lrint(clamped * 32767.0));
        write_u16(output, static_cast<std::uint16_t>(pcm));
    }
    return output.good();
}

} // namespace

int main(int argc, char** argv) {
    constexpr std::uint32_t sample_rate = 48000;
    constexpr std::uint32_t duration_frames = sample_rate * 4;
    const std::string path = argc > 1 ? argv[1] : "mpfree-demo.wav";

    mpfree::Song song;
    song.sample_rate = sample_rate;
    song.tempo_bpm = 120.0;

    mpfree::Track first;
    first.steps.resize(3);
    first.steps[0].enabled = true;
    first.steps[2].enabled = true;
    first.steps[2].ratchets = 2;

    mpfree::Track second;
    second.steps.resize(4);
    second.steps[0].enabled = true;
    second.steps[2].enabled = true;
    second.steps[2].velocity = 75;

    song.tracks = {first, second};

    std::array<mpfree::Event, 4096> events{};
    const auto result = mpfree::schedule(song, 0, duration_frames, events);
    if (result.dropped != 0) {
        std::cerr << "Event buffer too small\n";
        return 1;
    }

    std::vector<float> audio(duration_frames, 0.0f);
    const std::uint32_t click_frames = sample_rate / 12; // ~83 ms
    for (std::size_t i = 0; i < result.count; ++i) {
        const auto& event = events[i];
        const double frequency = event.track_index == 0 ? 330.0 : 660.0;
        const double gain = 0.35 * (static_cast<double>(event.velocity) / 127.0);
        for (std::uint32_t n = 0; n < click_frames; ++n) {
            const auto frame = event.frame + n;
            if (frame >= duration_frames) break;
            const double t = static_cast<double>(n) / sample_rate;
            const double envelope = std::exp(-40.0 * t);
            const double tone = std::sin(2.0 * std::numbers::pi_v<double> *
                                         frequency * t);
            audio[static_cast<std::size_t>(frame)] +=
                static_cast<float>(gain * envelope * tone);
        }
    }

    if (!write_mono_wav(path, audio, sample_rate)) {
        std::cerr << "Could not write WAV: " << path << '\n';
        return 1;
    }
    std::cout << "Rendered " << result.count << " events to " << path << '\n';
    return 0;
}
