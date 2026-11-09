#include "mpfree/ClickEngine.hpp"

#include <cstdlib>
#include <cmath>
#include <cstdint>
#include <iostream>
#include <limits>
#include <vector>

// CHECK remains enabled in Release/NDEBUG builds and evaluates expressions.
#define CHECK(condition) do { \
    if (!(condition)) { \
        std::cerr << "FAIL " << __FILE__ << ":" << __LINE__ \
                  << " : " #condition "\n"; \
        std::exit(1); \
    } \
} while (false)

namespace {
void test_timing() {
    mpfree::ClickEngine e;
    CHECK(e.configure({44100, 120.0, 4}));
    std::vector<float> l(44100), r(44100);
    e.render(l.data(), r.data(), l.size());
    CHECK(e.rendered_frames() == 44100);
    CHECK(e.beats_emitted() == 2);  // t=0 and 0.5s, not 1.0s
    e.render(l.data(), r.data(), 1);
    CHECK(e.beats_emitted() == 3);  // exactly at t=1 second

    e.reset();
    CHECK(e.beats_emitted() == 0);
    CHECK(e.rendered_frames() == 0);
    CHECK(e.configure({44100, 109.0, 4}));
    std::vector<float> block_l(128), block_r(128);
    std::uint32_t left = 7 * 44100;
    while (left > 0) {
        const auto n = left < 128 ? left : 128;
        e.render(block_l.data(), block_r.data(), n);
        left -= n;
    }
    CHECK(e.rendered_frames() == 7 * 44100);
    CHECK(e.beats_emitted() == 13);  // 0 through 12, next beat after 7s
}

void test_block_invariance() {
    mpfree::ClickEngine a, b;
    CHECK(a.configure({44100, 137.2, 3}));
    CHECK(b.configure({44100, 137.2, 3}));
    constexpr std::size_t n = 44100 * 2 + 13;
    std::vector<float> a_l(n), a_r(n), b_l(n), b_r(n);
    a.render(a_l.data(), a_r.data(), n);
    std::size_t written = 0;
    while (written < n) {
        const auto amount = (n - written < 128) ? n - written : 128;
        b.render(b_l.data() + written, b_r.data() + written, amount);
        written += amount;
    }
    CHECK(a.rendered_frames() == b.rendered_frames());
    CHECK(a.beats_emitted() == b.beats_emitted());
    bool nonzero = false;
    for (std::size_t i = 0; i < n; ++i) {
        CHECK(a_l[i] == b_l[i]);
        CHECK(a_r[i] == b_r[i]);
        CHECK(a_l[i] == a_r[i]);
        CHECK(std::isfinite(a_l[i]));
        CHECK(std::abs(a_l[i]) < 0.5f);
        nonzero |= a_l[i] != 0.0f;
    }
    CHECK(nonzero);
}

void test_guardrails() {
    mpfree::ClickEngine e;
    const auto original = e.configuration();
    CHECK(!e.configure({0, 120, 4}));
    CHECK(!e.configure({44100, 0, 4}));
    CHECK(!e.configure({44100, std::numeric_limits<double>::infinity(), 4}));
    CHECK(!e.configure({44100, 100, 0}));
    CHECK(e.configuration().sample_rate == original.sample_rate);
    CHECK(e.configuration().bpm == original.bpm);
    float sample = 0.0f;
    e.render(nullptr, &sample, 1);
    CHECK(e.rendered_frames() == 0);
    CHECK(mpfree::float_to_s32(0.0f) == 0);
}

void test_conversion() {
    CHECK(mpfree::float_to_s32(0.0f) == 0);
    CHECK(mpfree::float_to_s32(1.0f) == 2147483647);
    CHECK(mpfree::float_to_s32(-1.0f) == static_cast<std::int32_t>(-2147483647 - 1));
    CHECK(mpfree::float_to_s32(2.0f) == 2147483647);
    CHECK(mpfree::float_to_s32(-2.0f) == static_cast<std::int32_t>(-2147483647 - 1));
    CHECK(mpfree::float_to_s32(std::numeric_limits<float>::quiet_NaN()) == 0);
    CHECK(mpfree::float_to_s32(0.5f) > 0);
    CHECK(mpfree::float_to_s32(-0.5f) < 0);
}
} // namespace

int main() {
    test_timing();
    test_block_invariance();
    test_conversion();
    test_guardrails();
    std::cout << "MPFree deterministic core tests passed\n";
}
