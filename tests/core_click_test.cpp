#include "mpfree/ClickEngine.hpp"

#include <cassert>
#include <cmath>
#include <cstdint>
#include <iostream>
#include <limits>
#include <vector>

namespace {
void test_timing() {
    mpfree::ClickEngine e;
    assert(e.configure({44100, 120.0, 4}));
    std::vector<float> l(44100), r(44100);
    e.render(l.data(), r.data(), l.size());
    assert(e.rendered_frames() == 44100);
    assert(e.beats_emitted() == 2);  // t=0 and 0.5s, not 1.0s
    e.render(l.data(), r.data(), 1);
    assert(e.beats_emitted() == 3);  // exactly at t=1 second

    e.reset();
    assert(e.beats_emitted() == 0);
    assert(e.rendered_frames() == 0);
    assert(e.configure({44100, 109.0, 4}));
    std::vector<float> block_l(128), block_r(128);
    std::uint32_t left = 7 * 44100;
    while (left > 0) {
        const auto n = left < 128 ? left : 128;
        e.render(block_l.data(), block_r.data(), n);
        left -= n;
    }
    assert(e.rendered_frames() == 7 * 44100);
    assert(e.beats_emitted() == 13);  // 0 through 12, next beat after 7s
}

void test_block_invariance() {
    mpfree::ClickEngine a, b;
    assert(a.configure({44100, 137.2, 3}));
    assert(b.configure({44100, 137.2, 3}));
    constexpr std::size_t n = 44100 * 2 + 13;
    std::vector<float> a_l(n), a_r(n), b_l(n), b_r(n);
    a.render(a_l.data(), a_r.data(), n);
    std::size_t written = 0;
    while (written < n) {
        const auto amount = (n - written < 128) ? n - written : 128;
        b.render(b_l.data() + written, b_r.data() + written, amount);
        written += amount;
    }
    assert(a.rendered_frames() == b.rendered_frames());
    assert(a.beats_emitted() == b.beats_emitted());
    bool nonzero = false;
    for (std::size_t i = 0; i < n; ++i) {
        assert(a_l[i] == b_l[i]);
        assert(a_r[i] == b_r[i]);
        assert(a_l[i] == a_r[i]);
        assert(std::isfinite(a_l[i]));
        assert(std::abs(a_l[i]) < 0.5f);
        nonzero |= a_l[i] != 0.0f;
    }
    assert(nonzero);
}

void test_guardrails() {
    mpfree::ClickEngine e;
    const auto original = e.configuration();
    assert(!e.configure({0, 120, 4}));
    assert(!e.configure({44100, 0, 4}));
    assert(!e.configure({44100, std::numeric_limits<double>::infinity(), 4}));
    assert(!e.configure({44100, 100, 0}));
    assert(e.configuration().sample_rate == original.sample_rate);
    assert(e.configuration().bpm == original.bpm);
    float sample = 0.0f;
    e.render(nullptr, &sample, 1);
    assert(e.rendered_frames() == 0);
    assert(mpfree::float_to_s32(0.0f) == 0);
}

void test_conversion() {
    assert(mpfree::float_to_s32(0.0f) == 0);
    assert(mpfree::float_to_s32(1.0f) == 2147483647);
    assert(mpfree::float_to_s32(-1.0f) == static_cast<std::int32_t>(-2147483647 - 1));
    assert(mpfree::float_to_s32(2.0f) == 2147483647);
    assert(mpfree::float_to_s32(-2.0f) == static_cast<std::int32_t>(-2147483647 - 1));
    assert(mpfree::float_to_s32(std::numeric_limits<float>::quiet_NaN()) == 0);
    assert(mpfree::float_to_s32(0.5f) > 0);
    assert(mpfree::float_to_s32(-0.5f) < 0);
}
} // namespace

int main() {
    test_timing();
    test_block_invariance();
    test_conversion();
    test_guardrails();
    std::cout << "MPFree deterministic core tests passed\n";
}
