//! What a sound is apart from what it is doing, and the rules that follow
//! (the later phases' P3, step 2): memory or disk, starts sized to the
//! refill time, and why admission says a workload isn't covered.

use mpfree_stream::coord::Plan;
use mpfree_stream::cursor::MAX_SPEED;
use mpfree_stream::msg::{Place as At, MAX_CURSORS, MAX_SOURCES};
use mpfree_stream::player::{Config, Player, MIN_SLOTS, UNIT};
use mpfree_stream::policy::{
    answer, place, registry, start_frames, Access, Class, Cover, Device, Kind, Need, Place, Refill,
    Registry, Rule, REGISTERED_MOST, REGISTERED_WARN,
};

fn kind(class: Class, access: Access, rule: Rule) -> Kind {
    Kind {
        class,
        access,
        rule,
        device: Device::Stick,
    }
}

#[test]
fn memory_or_disk_follows_the_class_the_rule_and_the_room() {
    use Class::*;
    let auto = |c| kind(c, Access::Straight, Rule::Automatic);
    // Kit sounds, keygroup samples, one-shots and loops load whole when
    // they fit, and stream when they don't.
    for c in [KitSound, KeygroupSample, OneShot, Loop] {
        assert_eq!(place(&auto(c), true), Place::Whole, "{c:?}");
        assert_eq!(place(&auto(c), false), Place::Stream, "{c:?}");
    }
    // Stems and recordings stream, room or not.
    for c in [Stem, Recording] {
        assert_eq!(place(&auto(c), true), Place::Stream, "{c:?}");
    }
    // Keep in memory asks for memory even when it doesn't fit (admission
    // then says what it lacks); Prefer streaming streams even a kit sound.
    let keep = kind(Stem, Access::Straight, Rule::KeepInMemory);
    assert_eq!(place(&keep, false), Place::Whole);
    let prefer = kind(KitSound, Access::Straight, Rule::PreferStreaming);
    assert_eq!(place(&prefer, true), Place::Stream);
    // A sound already in memory stays there.
    let held = Kind {
        device: Device::Memory,
        ..kind(Stem, Access::Straight, Rule::PreferStreaming)
    };
    assert_eq!(place(&held, false), Place::Whole);
}

#[test]
fn a_start_that_comes_without_warning_covers_the_whole_refill() {
    // The review's case: an 800 ms stall. A 100 ms start would run dry.
    let refill = Refill {
        stall_s: 0.8,
        decode_s: 0.05,
        deliver_s: 0.01,
    };
    assert!((refill.total_s() - 0.86).abs() < 1e-12);
    let pad = kind(Class::KitSound, Access::Straight, Rule::Automatic);
    // 0.86 s at 44.1 kHz is 37,926 frames; float sums may round up one
    // more, which only covers a little more.
    let frames = start_frames(&pad, &refill, 44_100);
    assert!((37_926..=37_927).contains(&frames), "{frames}");
    assert!(
        start_frames(&pad, &refill, 44_100) > 4_410,
        "more than 100 ms"
    );
    // A keygroup played up reads faster: it keeps the start at the fastest
    // a cursor reads.
    let keys = kind(Class::KeygroupSample, Access::VaryingSpeed, Rule::Automatic);
    assert_eq!(
        start_frames(&keys, &refill, 48_000),
        (refill.total_s() * 48_000.0 * MAX_SPEED).ceil() as u64
    );
    // Stems, loops and recordings start at a known time: fetched before
    // they start, so no start is kept.
    for c in [Class::Stem, Class::Loop, Class::Recording] {
        assert_eq!(
            start_frames(&kind(c, Access::Straight, Rule::Automatic), &refill, 44_100),
            0,
            "{c:?}"
        );
    }
    // A refill that never ends can't be covered by any start.
    let never = Refill {
        stall_s: f64::INFINITY,
        ..refill
    };
    assert_eq!(start_frames(&pad, &never, 44_100), u64::MAX);
    assert_eq!(start_frames(&pad, &Refill::default(), 44_100), 0);
}

#[test]
fn admission_says_why_a_workload_isnt_covered_most_binding_first() {
    let fine = Need {
        measured: true,
        keeps_up: true,
        ..Need::default()
    };
    assert_eq!(answer(&fine), Cover::Covered);
    // Memory first, measured or not.
    let short = Need {
        memory_short: 40_000_000,
        ..Need::default()
    };
    assert_eq!(answer(&short), Cover::MemoryShort { by: 40_000_000 });
    // Then an unmeasured stick: plays, shown untested.
    assert_eq!(answer(&Need::default()), Cover::Untested);
    // Then a stick too slow for the streams.
    let slow = Need {
        keeps_up: false,
        ..fine
    };
    assert_eq!(answer(&slow), Cover::TooSlow);
    // A start the stick can't refill in time: Keep in memory, but only when
    // the whole sound fits; else the memory it lacks.
    let start = Need {
        uncovered: Some(7),
        ..fine
    };
    assert_eq!(answer(&start), Cover::KeepInMemory { source: 7 });
    let too_big = Need {
        whole_short: 12_000_000,
        ..start
    };
    assert_eq!(answer(&too_big), Cover::MemoryShort { by: 12_000_000 });
}

#[test]
fn a_plan_is_measured_only_when_it_comes_from_the_stick_test() {
    assert!(!Plan::default().measured);
    let p = Plan::for_shortfall(1.5);
    assert!(p.measured);
    assert_eq!(p.window_s, 4.0);
    assert!(!Plan::for_shortfall(0.0).measured);
    assert!(!Plan::for_shortfall(f64::NAN).measured);
}

#[test]
fn the_cache_is_made_for_the_musicians_budget() {
    let unit = (UNIT * 4) as u64;
    for mb in [64u64, 128, 256] {
        let c = Config::for_budget(mb << 20);
        assert_eq!(c.slots as u64 * unit, mb << 20, "{mb} MiB");
        assert_eq!(c.cache_bytes(), mb << 20);
        assert_eq!(c.buffers, Config::default().buffers);
    }
    // 128 MB as the musician says it (10^6 bytes): whole units only.
    assert_eq!(Config::for_budget(128_000_000).slots, 1953);
    assert_eq!(Config::default().slots, Config::for_budget(128 << 20).slots);
    assert_eq!(Config::for_budget(0).slots, MIN_SLOTS);
}

#[test]
fn the_limits_cover_the_samplers_worst_case_and_say_when_passed() {
    // The sampler's worst case: 64 voices and an audition, each with a
    // release fade still sounding while a steal starts its next.
    let sampler = 2 * (64 + 1);
    let (stems, lanes, clips) = (16, 16, 64);
    assert!(sampler + stems + lanes + clips <= MAX_CURSORS);
    assert!(64 + 1 + stems + lanes + clips <= MAX_SOURCES);
    // A cursor past the limit isn't planned for, and is counted.
    let (mut p, _from, _to) = Player::new(Config::default());
    let at = At {
        frame: 0,
        speed: 1.0,
        looping: None,
        end: None,
    };
    p.playing(0, (MAX_CURSORS - 1) as u16, at);
    assert_eq!(p.stats().over_limit, 0);
    p.playing(0, MAX_CURSORS as u16, at);
    assert_eq!(p.stats().over_limit, 1);
    // Registered sounds: two big auto-sampled keygroups (61 notes × 4
    // layers each) beside ordinary kits are allowed, and said.
    assert_eq!(registry(0), Registry::Fine);
    assert_eq!(registry(250), Registry::Fine);
    assert_eq!(registry(REGISTERED_WARN), Registry::Many);
    assert_eq!(registry(2 * 61 * 4 + 64), Registry::Many);
    assert_eq!(registry(REGISTERED_MOST - 1), Registry::Many);
    assert_eq!(registry(REGISTERED_MOST), Registry::Full);
}
