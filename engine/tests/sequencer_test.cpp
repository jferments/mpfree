#include "mpfree/sequencer.hpp"

#include <array>
#include <cassert>
#include <cstdint>
#include <iostream>
#include <vector>

using mpfree::Event;
using mpfree::ScheduleResult;
using mpfree::Song;
using mpfree::Step;
using mpfree::Track;

namespace {

Song new_song() {
    Song song;
    song.tempo_bpm = 120.0;
    song.sample_rate = 48000;
    song.probability_seed = 42;
    return song;
}

std::vector<Event> events_for(
    const Song& song,
    std::int64_t start,
    std::uint32_t frames) {
    std::array<Event, 512> buffer{};
    const ScheduleResult result = mpfree::schedule(song, start, frames, buffer);
    assert(result.dropped == 0);
    return {buffer.begin(), buffer.begin() + result.count};
}

void test_basic_step_timing() {
    Song song = new_song();
    Track track;
    track.steps.resize(4);
    track.steps[0].enabled = true;
    track.steps[2].enabled = true;
    song.tracks.push_back(track);

    const auto events = events_for(song, 0, 30000);
    assert(events.size() == 3);
    assert(events[0].frame == 0);
    assert(events[1].frame == 12000);
    assert(events[2].frame == 24000);
    assert(events[0].step_index == 0);
    assert(events[1].step_index == 2);
    assert(events[2].step_index == 0);
}

void test_independent_track_lengths() {
    Song song = new_song();
    Track three;
    three.steps.resize(3);
    three.steps[0].enabled = true;
    Track four;
    four.steps.resize(4);
    four.steps[0].enabled = true;
    song.tracks = {three, four};

    const auto events = events_for(song, 0, 60000);
    const std::vector<std::int64_t> expected = {
        0, 0, 18000, 24000, 36000, 48000, 54000,
    };
    assert(events.size() == expected.size());
    for (std::size_t i = 0; i < expected.size(); ++i) {
        assert(events[i].frame == expected[i]);
    }
    assert(events[0].track_index == 0);
    assert(events[1].track_index == 1);
}

void test_ratchets_and_parameter_locks() {
    Song song = new_song();
    Track track;
    Step step;
    step.enabled = true;
    step.ratchets = 4;
    step.lock_count = 1;
    step.locks[0] = {7, 0.5f};
    track.steps = {step};
    song.tracks.push_back(track);

    const auto events = events_for(song, 0, 6000);
    assert(events.size() == 4);
    for (std::size_t i = 0; i < events.size(); ++i) {
        assert(events[i].frame == static_cast<std::int64_t>(i * 1500));
        assert(events[i].ratchet_index == i);
        assert(events[i].lock_count == 1);
        assert(events[i].locks[0].parameter_id == 7);
        assert(events[i].locks[0].value == 0.5f);
    }
}

void test_negative_microtiming_crosses_step_boundary() {
    Song song = new_song();
    Track track;
    track.steps.resize(2);
    track.steps[1].enabled = true;
    track.steps[1].microtiming_frames = -250;
    song.tracks.push_back(track);
    const auto events = events_for(song, 0, 18000);
    assert(events.size() == 2);
    assert(events[0].frame == 5750);
    assert(events[1].frame == 17750);
}

void test_every_second_cycle() {
    Song song = new_song();
    Track track;
    Step step;
    step.enabled = true;
    step.every_n_cycles = 2;
    track.steps = {step};
    song.tracks.push_back(track);
    const auto events = events_for(song, 0, 30000);
    assert(events.size() == 3);
    assert(events[0].frame == 0);
    assert(events[1].frame == 12000);
    assert(events[2].frame == 24000);
}

void test_probability_determinism_and_block_invariance() {
    Song song = new_song();
    Track track;
    track.steps.resize(3);
    track.steps[0].enabled = true;
    track.steps[0].probability = 40;
    track.steps[1].enabled = true;
    track.steps[1].probability = 0;
    track.steps[2].enabled = true;
    track.steps[2].ratchets = 3;
    song.tracks.push_back(track);

    const auto all = events_for(song, 0, 36000);
    std::vector<Event> chunked;
    for (std::int64_t start = 0; start < 36000; start += 12000) {
        const auto part = events_for(song, start, 12000);
        chunked.insert(chunked.end(), part.begin(), part.end());
    }
    assert(all.size() == chunked.size());
    for (std::size_t i = 0; i < all.size(); ++i) {
        assert(all[i].frame == chunked[i].frame);
        assert(all[i].track_index == chunked[i].track_index);
        assert(all[i].step_index == chunked[i].step_index);
        assert(all[i].ratchet_index == chunked[i].ratchet_index);
        assert(chunked[i].offset == chunked[i].frame -
            (chunked[i].frame / 12000) * 12000);
    }
}

void test_bounded_output_and_invalid_configuration() {
    Song song = new_song();
    Track track;
    Step step;
    step.enabled = true;
    step.ratchets = 4;
    track.steps = {step};
    song.tracks.push_back(track);

    std::array<Event, 2> buffer{};
    auto result = mpfree::schedule(song, 0, 6000, buffer);
    assert(result.count == 2);
    assert(result.dropped == 2);

    song.tempo_bpm = 0.0;
    result = mpfree::schedule(song, 0, 6000, buffer);
    assert(result.count == 0);
    song.tempo_bpm = 120.0;
    song.tracks[0].steps[0].microtiming_frames = 2049;
    result = mpfree::schedule(song, 0, 6000, buffer);
    assert(result.count == 0);
}

} // namespace

int main() {
    test_basic_step_timing();
    test_independent_track_lengths();
    test_ratchets_and_parameter_locks();
    test_negative_microtiming_crosses_step_boundary();
    test_every_second_cycle();
    test_probability_determinism_and_block_invariance();
    test_bounded_output_and_invalid_configuration();
    std::cout << "mpfree native sequencer tests passed\n";
    return 0;
}
