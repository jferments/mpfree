# ADR-0001: Rust-first, C/C++-compatible MPFree architecture (ACCEPTED)

**Date:** 2026-11-08 America/Los_Angeles  
**Status:** **ACCEPTED (owner decision, 2026-11-08 Pacific)** — MPFree-owned new code should be Rust-first; C/C++ remain supported for high-value integration. This authorizes a staged migration and Rust implementations, **not** deleting the existing C++ reference or changing MPC firmware/services.  
**Repo:** `jferments/mpfree`; baseline branch `bootstrap/architecture-and-hardware-survey`.  
**Basis:** owner-verified MPC One az0x 3.6.0 hardware research; nine/ nine Python audio packet tests, two/two C++ CTest tests, an audible host-generated 4-second 44.1-kHz stereo S32_LE metronome, and read-only observations.

## Accepted decision

**Build new MPFree-owned core components primarily in Rust**, **keep targeted C/C++ adapters for externally maintained DSP engines, mature plugin ecosystems and low-level hardware when justified**, and permit running heavyweight C++ plugins on a laptop/desktop auxiliary host rather than trying to force them into the RK3288 MPC.

This is a **Rust-first hybrid**, not a "rewrite every dependency in Rust" rule. Keep the existing `ClickEngine` as a reference implementation and a known-good baseline. Before *retiring* the C++ reference, require a Rust parity implementation plus **real ARMv7 hard-float cross-build** and deterministic desktop tests. **The language policy is accepted; target runtime compatibility is a separate qualification gate.**

### Implementation update — M1 Rust metronome parity completed

Owner's **Rust-first decision is accepted and active**, and a concrete Rust implementation now exists under [`rust/`](../../rust/) rather than only a proposal. [M1 developer and CI guide](RUST_M1_PARITY.md) documents it. The Rust implementation has no third-party dependencies; it retains the original C++ metronome as a regression oracle.

**Verified GitHub Actions:** Rust debug + release tests, C++ reference tests, three WAV parity cases at 120, 109, 137.2 BPM with **zero observed PCM integer difference**, and successfully linked ARM ELF32 EABI5 **hard-float** cross-target (CI run: https://github.com/jferments/mpfree/actions/runs/42575002089). An earlier cross-linker failure (missing `Scrt1.o`/`crti.o`) was resolved by installing `libc6-dev-armhf-cross`.

**Not verified:** armv7 binary execution on az0x, its `PT_INTERP` and glibc compatibility, realtime thread behavior on MPC, or access to stock-owned ACVA ALSA. This decision does not authorize device takeover or persistent firmware/boot modifications.

The biggest actual risk of a Rust-first engine is **access to mature C++ plugin/instrument hosting frameworks and ARMv7-specific runtime integration**, not whether Rust can render 128 stereo audio frames.

## Hardware constraints, observed not inferred

| Item | Observed |
| --- | --- |
| Device | Original MPC One, DT `rockchip,rk3288`, Linux 6.6.95 PREEMPT_RT az0x 5.0.8 |
| CPU | Four ARMv7 cores (MIDR `0xc0d`), NEON and VFPv4, observed 1.608 GHz |
| RAM | Approximately 2 GiB, no swap; stock app held ~1 GiB locked/resident |
| Userspace ABI | ARM ELF32 EABI5 hard-float, `e_flags = 0x05000400` |
| Audio | ALSA ACVA card1 device0, stereo interleaved `S32_LE`, 44,100 Hz, 128-frame period |
| Output/capture buffers | 384 / 256 frames; not measured analog or round-trip latency |
| Hardware I/O | USB-MIDI RawMIDI/ALSA sequencer, evdev touchscreen+GPIO, DRM/KMS, Panfrost/Mali-T760-compatible |
| Networking | Realtek RTL8211E gigabit full-duplex Ethernet, USB-A high speed, USB device-side DWC2 UDC |

References: [audio survey](../hardware/MPC_ONE_3_6_0_AUDIO_RUNTIME_20261109.md), [Ethernet/USB survey](../hardware/MPC_ONE_3_6_0_ETHERNET_USB_20261109.md), [resource ownership](../hardware/MPC_ONE_3_6_0_RESOURCES_20261109.md).

A concrete render deadline budget follows from **128 / 44100 ≈ 2.9025 ms per ALSA period**. This is not a guarantee the application is awakened exactly once per period. Audio callback execution time, scheduling interference and audio-buffer occupancy must be measured under worst-case workload on ARMv7.

## Benefits of Rust for MPFree

1. **Memory safety in safe Rust**: borrow checking, ownership, lifetimes and slice bounds checks prevent use-after-free, double-free and ordinary out-of-bounds memory accesses. These are especially useful when users load untrusted project files, sample headers, controller data and network audio packets. Rust does *not* make external C/C++ code safe.
2. **Stronger concurrency defaults**: `Send`/`Sync` and ownership rules reject many data races at compile time. This supports a design with an isolated real-time audio thread, UI thread, background disk sampler and network receiver.
3. **Explicit error handling**: `Result`/`Option` and exhaustive enums fit protocol parsers, format negotiation and device state transitions better than unchecked sentinel values.
4. **Performance characteristics**: ahead-of-time native code, no mandatory garbage collector, stack/fixed-buffer designs, and ARMv7/NEON optimization options. However, Rust is **not inherently faster** than optimized C++; both will need benchmark-guided DSP tuning.
5. **Software architecture**: Cargo workspaces, deterministic unit tests, property-based/fuzz testing and well-defined data types are attractive for a long-lived community-run free software project.
6. **FFI portability**: C ABIs and selected stable C/C++ shims allow using libraries without reimplementing them.

## Limitations and tradeoffs

1. **Rust does NOT guarantee real-time deadlines.** Mutex waits, allocator calls, slow syscalls, cache misses, panic handling, implicit expensive drop logic, page faults and OS scheduling can still cause audio glitches.
2. **An `unsafe` boundary is inevitable** for direct syscalls/device memory mapping, plugin ABI, selected SIMD and some FFI. Our policy should be to keep these in tiny reviewed modules rather than scattering them throughout DSP graph code.
3. **Audio/plugin ecosystems skew C/C++**: JUCE, Carla, LV2 hosting via lilv, large synth engines (Surge XT, sfizz), and vendor SDKs are C/C++-oriented. A mature Rust DAW that seamlessly replaces these frameworks is not an established assumption.
4. **More integration work**: Rust FFI builds need careful control over target sysroot, linker, ABI, libc and C++ runtime. The code often becomes simpler, but the integration test matrix may become larger.
5. **Potential loss of compile iteration speed** as large generic/monomorphized Rust dependency graphs grow; profile binary size and rebuild time on developers' machines.
6. **Library licensing and unsafe transitive code** remain real risks. Rust does not eliminate security auditing, plugin isolation, dependencies' licenses or ABI issues.
7. **Strict memory budgets** require avoiding oversized general-purpose frameworks. More safety is helpful but not a substitute for bounded cache/voice allocation.

## Rust audio/library inventory (verified publicly November 2026)

| Subsystem | Rust choice and capabilities | MPFree fit / caveats |
| --- | --- | --- |
| ALSA PCM/control/raw MIDI/seq | [`alsa` / alsa-rs](https://docs.rs/alsa/latest/alsa/) | **Best initial MPC HAL candidate**; thin wrappers around the existing libasound C ABI; PCM/control/RawMIDI/seq APIs are present. Validate 128-frame S32_LE negotiation after safe handoff; never open stock-owned devices during survey. |
| Cross-platform audio I/O | [CPAL](https://github.com/RustAudio/cpal) | Good desktop backend. Linux uses ALSA; others include JACK/PipeWire/PulseAudio. For precise MPC hardware period/ring/trigger settings a custom ALSA adapter may be more predictable than abstracted device defaults. |
| High-level playback | [Rodio](https://github.com/RustAudio/rodio) | Useful for app sound previews/host tooling, **not** a replacement for a deterministic multiprocessor DAW render graph. |
| Basic sample DSP | [dasp](https://github.com/RustAudio/dasp) | Sample/frame conversions, graph tools, basic metering and buffers; audit feature-dependent allocation for callbacks. |
| PCM file/codec reading | [Symphonia](https://docs.rs/symphonia/latest/symphonia/) | WAV/AIFF, FLAC, MP3/AAC/Vorbis with features; no reason to implement all decoders ourselves. Decode/preload/stream outside RT callback. |
| Resampling / clock drift | [Rubato](https://github.com/HEnquist/rubato) | Async variable-ratio resamplers for laptop↔MPC clock drift; initialize/preallocate and measure CPU load on ARMv7. |
| FFT | [rustfft](https://github.com/ejmahler/RustFFT), [realfft](https://github.com/HEnquist/realfft) | Useful for analysis/convolution with preplanned scratch memory. Cross-compile and validate efficient ARMv7 vectorization. |
| Lock-free queues | [rtrb](https://docs.rs/rtrb/latest/rtrb/) | SPSC fixed-capacity lock-free/wait-free queue; suitable for control events and network→audio transfer when initialized outside the callback. |
| Cross-platform MIDI | [midir](https://github.com/Boddlnagg/midir) | Desktop controller input and basic ALSA integration; custom MIDI parser still needed for Akai internal Public/Private messages. |
| CLAP plugin HOST | [clack-host](https://github.com/prokopyl/clack) | **Exists**; relatively low-level, so significant host lifecycle/parameters/buses/GUI and crash isolation work still belongs to MPFree. |
| Rust plugin AUTHORING | [NIH-plug](https://github.com/robbert-vdh/nih-plug) | Plugin authoring rather than a universal host; original upstream states **maintenance mode**, directs readers to community fork. Do not make it our indispensable plugin foundation. |
| VST3 host | [vst3-host](https://docs.rs/vst3-host/latest/vst3_host/) and alternatives | Rust hosting libraries exist, but at least one openly documents that its audio path is **not yet lock-free/real-time-tuned**. Do not treat as a safe drop-in for the MPC. |
| C/C++ interoperability | [cxx](https://docs.rs/cxx/latest/cxx/) and C ABI shims | Use stable ABI boundaries; C++ library memory safety does not become safe simply by wrapping it. CXX MSRV and generated linking must fit chosen toolchain. |
| Hardware DRM/KMS | [drm](https://docs.rs/drm/latest/drm/), Rust evdev crates | Feasible for device adaptation; evaluate unsafe ioctls, input rotations, ownership and performance on target. |
| Rust UI | [Slint](https://github.com/slint-ui/slint), SDL bindings, selected egui backends | **Slint Linux KMS** has direct DRM dumb-buffer/software and GBM/EGL paths; worth a target-specific proof of concept. Do **not** assume Vulkan is supported by Mali-T760. |

The above table records publicly advertised capabilities, **not confirmed ARMv7 availability/buildability on MPC**. Every candidate must be pinned, license-reviewed and tested on host/ARMv7 before production inclusion.

### Examples of C/C++ libraries with weaker Rust-native substitutes

| C/C++ project / ecosystem | Why we would want it | What moving to Rust changes |
| --- | --- | --- |
| [JUCE](https://github.com/juce-framework/JUCE) | Mature audio app infrastructure, plugin host, UI, MIDI/DSP, many plugin formats | Rust cannot directly inherit JUCE C++ abstractions. Use a **separate C++ integration module** if indispensable, or avoid global JUCE dependency. JUCE currently states **AGPLv3/commercial** licensing; confirm compatibility with the project's intended copyleft license. JUCE also says 32-bit ARM Linux is not regularly tested. |
| [Carla](https://github.com/falkTX/Carla) | Broad LADSPA, LV2, VST2/3, plugin host, sound banks | No Rust library with established feature parity was identified. Could integrate a separately built C/C++ host or run Carla on an auxiliary Linux machine via network-audio transport; don't assume ARMv7 plugin binaries exist. Carla is GPL-2.0-or-later. |
| [sfizz](https://github.com/sfztools/sfizz) | Mature SFZ sampler engine | **Has C and C++ API bindings**; Rust can use a small C shim. Build/CPU/memory budget and sample-content licensing still need qualification. |
| [FluidSynth](https://github.com/FluidSynth/fluidsynth) | SoundFont SF2 synthesis | Has a stable C-oriented API and is **LGPL-2.1**. Use Rust FFI rather than a complete rewrite if effective. |
| [Surge XT](https://github.com/surge-synthesizer/surge) | Complex synthesizer | Native engine is C++ GPL-3.0; some Rust bindings exist. Might be better as laptop-side synth/CLAP plugin, avoiding expensive ARMv7 build and peak CPU overhead. |
| [lilv](https://gitlab.com/lv2/lilv), [LV2](https://lv2plug.in/) | Host diverse open-source Linux plugin formats | Low-level C libraries/metadata integration remain strong; no proven full Rust host drop-in. We can use FFI or isolated C host. |
| [Rubber Band](https://breakfastquay.com/rubberband/) / SoundTouch | Sophisticated time stretching and pitch shifting | Rust pitch/time algorithms exist, but quality/performance/license equivalence requires controlled comparisons. Use FFI when a specific C++ algorithm decisively wins. |
| [FFmpeg](https://ffmpeg.org/) / libsndfile | Breadth of codec/container conversion and file import | Rust wrappers and Symphonia cover many formats, not all of FFmpeg. Optional helper process or C ABI import library, always off the audio thread. |

**"Unavailable in Rust" is usually the wrong claim:** many libraries expose a C ABI, have Rust bindings, can be wrapped, or can run in a separate process. The real disadvantages are implementation effort, incomplete host maturity, C/C++ safety boundaries, licensing, and native plugin availability.

### Plugins are an ISA/OS/ABI problem, independently of language

An x86-64 Windows or Linux VST3 binary cannot simply be loaded into the MPC's **32-bit ARM Linux** process whether the host is Rust or C++. Plugins must be compiled for the exact architecture, calling convention and necessary runtime. Moreover, 32-bit ARM host memory limits remain (~2 GiB total RAM).

**Architectural proposal:** MPFree ships a few native open-source DSP/sampler instruments for armv7-gnueabihf, and supports full CLAP/VST3/LV2 hosting on a more powerful **auxiliary laptop/desktop computer** when suitable. This preserves the advanced library ecosystem without forcing every plugin to run on MPC.

## Rust graphics: a hardware-specific blocking concern

The surveyed MPC exposes an **ARM Mali-T760-compatible Midgard GPU with Panfrost**, a DRM card and DSI mode **800×1280**. Current Mesa Panfrost documentation reports **OpenGL ES 3.1** for Mali-T760 but **no Vulkan implementation** for Midgard T760: https://docs.mesa3d.org/drivers/panfrost.html .

Therefore:
- A renderer that **requires Vulkan** is unsuitable for MPC-first deployment as currently observed.
- Rust UI is viable via software/DRM KMS or EGL/OpenGL ES, but **must be tested** with existing libc, EGL/Mesa and libdrm without disturbing the factory DRM master.
- Candidates for exploration: Slint direct Linux KMS backend (DRM dumb buffers / software renderer or GBM/EGL), a custom SDL2/3 EGL backend, or wgpu's GLES backend **after verifying feature requirements**. None is proven on actual device.
- Rust doesn't magically grant access to the proprietary factory graphics stack; UI architecture should separate logical coordinates (landscape) from native panel mode (portrait 800×1280) and read-only/takeover states.
- **Slint licensing is material:** its official terms include **GPLv3** for open-source embedded products, alongside royalty-free/commercial options whose embedded scope differs. For our free-software MPFree vision, evaluate GPLv3 compatibility explicitly before adopting it. Source: https://github.com/slint-ui/slint/blob/master/LICENSE.md .
- Some UI frameworks have per-component licensing, deployment dependencies or renderer restrictions; validate before adoption.

## Real-time Rust rules for MPFree (language-agnostic RT guarantees)

The stock audio profile suggests an ALSA period of **2.9025 ms**. Do not do the following on any real-time callback, even in safe Rust:
- dynamic allocations, string formatting, logging, filesystem/database I/O, DNS, syscalls with unbounded blocking, network \`recv\`, or UI calls;
- lock a contended `Mutex`/`RwLock`, wait on futures/async runtimes, or perform potentially blocking channel send;
- change project graphs, load samples/plugins, construct DSP resamplers/FFT planners, or free large graph/plugin objects;
- panic across FFI boundaries; a panic-abort policy avoids unwinding into foreign ABI but **ends the app**, so avoid panics in RT code and test bounds.

Use:
- preallocated, fixed-capacity audio buffers and voice pools;
- single-owner audio graph state; SPSC queues (`rtrb`) for control/network transfers with explicit full/empty policy;
- immutable transport/event snapshots exchanged outside callback; defer old graph destruction to non-RT;
- audio in `&mut [f32]` / `&mut [i32]` with explicit maximum block and channel geometry;
- setup-time resampler/FFT planning; no heap or disk calls in render;
- deterministic sample-frame timestamps with bounded future events, property testing, fault injection, stress and sanitizers/Miri where applicable;
- no global allocator in the callback; optionally instrumentation to detect unexpected allocations on tests; measure p99.9/max callback cost and xruns on the actual ARMv7 hardware.

The target first-pass render-time engineering goal could be **< 50% of the observed 2.90-ms period under representative workload** (i.e. under ~1.45 ms), leaving schedule headroom, but **that is a development target, not a benchmark currently achieved**.

`async`/Tokio can be excellent for UI, file indexing, USB/network control and auxiliary compute coordination; it must not run the time-critical PCM render callback.

## Cross-compilation and actual deployment gate

Official Rust platform support lists `armv7-unknown-linux-gnueabihf` as a tier-2 Armv7 hard-float Linux target: https://doc.rust-lang.org/stable/rustc/platform-support.html .

Candidate build route:

~~~text
ML-tower x86-64 workstation
  ├── Cargo/Rust stable; target armv7-unknown-linux-gnueabihf
  ├── cross-GNU linker for ARM hard-float + known-compatible sysroot
  ├── linked C dependencies (ALSA, EGL/GBM/DRM) matched to target
  └── MPFree ELF32 ARM EABI5 hard-float (not aarch64)
         └── run in volatile MPC staging **only after reviewed, safe test**
~~~

A successful desktop Rust build does **not** prove the generated executable runs on az0x. Check the linked `PT_INTERP`, glibc versioned symbols, libstdc++ dependencies if using C++, dynamic linker path, and whether the required libasound etc actually exist. We have measured ARM EABI/loader but **not yet built an executable for the MPC**.

For native dependencies consider a pinned Yocto-compatible sysroot or building required libs against an explicitly controlled toolchain. Do **not** copy proprietary Akai binaries to the public repository.

### Hybrid ABI boundary rule

A plugin/instrument/native C++ integration MUST NOT expose C++ STL, Rust `Vec` or Rust trait objects across an unversioned binary boundary. Prefer:
- same-process native adapter built together as a **versioned C ABI** or a narrowly-defined `cxx` bridge;
- explicit buffer pointer/length/channel count, valid lifetime, alignment, sample rate, no exceptions/unwinding across FFI;
- setup/teardown/configuration off RT; an audio `process` function must have bounded documented behavior;
- crash-prone/untrusted third-party plugins in a separate process (prefer the auxiliary workstation), with framed audio transport and fallback.

Do not consider “a Rust wrapper around C++” the equivalent of fully memory-safe execution.

## Qualification required before replacing the C++ baseline

The working C++ and Python baseline is valuable. Owner results reported:
- Python packet model **9/9 tests passed**, including reordering/late/loss/geometry.
- CMake/CTest **2/2 tests passed**.
- Four-second metronome at 120 BPM played audibly; 176,400 frames at 44.1 kHz, eight beat triggers.

Our first accepted Rust implementation milestone should:
1. Keep C++ source and CMake/CTest untouched as the reference.
2. Add a standalone Cargo crate implementing the *same* tempo/beat/frame scheduling and S32 conversion, with equivalent input-output artifacts.
3. Run deterministic parity tests: frame positions, noninteger BPM, different block segmentation, NaNs/clipping, endian WAV validation, period/buffer metadata and sequence number wrap.
4. Compare **audio outputs logically** (frame trigger positions, geometry, peak/mix; waveform byte identity only if matching numeric implementation is required). Different legitimate trig/rounding choices may produce slightly different samples.
5. Build an ARMv7 hard-float ELF **on ML-tower** using pinned toolchain; confirm architecture/ABI/linker, no proprietary source.
6. Benchmark warm, preallocated DSP workload memory consumption, xrun-free scheduling and eventual UI benchmark on target **only when safe staging is proven**.
7. Prove Rust ALSA wrapper can express the exact observed 44.1kHz/S32/128/384 configuration **without opening ACVA while stock software is running**. API static inspection and host fake backend first.
8. Perform license/security/dependency audit: `cargo deny`, `cargo audit`, tracked lockfile and vendored build option for reproducibility.
9. Review key FFI seams (sfizz, FluidSynth, plugin hosts) before committing to a feature list or deciding no C++ will ever be used.

**Policy accepted; verification still required.** Implement Rust-first now, but preserve the original C++ executable as a side-by-side regression reference until the new implementation meets or exceeds the tested contract. ARMv7 cross-builds do not themselves establish ABI compatibility with az0x or authorize running a binary on the MPC.

## Accepted layering (implementation staged)

~~~text
MPFree UI / controller model               Rust primarily (backend TBD)
MPFree project/session file parsers         Rust safe, fuzz-tested
MPFree sequencer/transport                  Rust safe, deterministic frame timeline
MPFree audio DSP graph and sampler          Rust, preallocated, tested RT constraints
MPFree network/audio transport              Rust, versioned protocol, bounded queues
MPFree hardware HAL                          Rust facade
  ├── ALSA PCM, raw MIDI and sequencer       alsa-rs / C ALSA
  ├── evdev + DRM/GBM + EGL/OpenGL ES        Rust bindings + reviewed C FFI as needed
  └── original device installer             separate signed/staged process
Optional synth/plugins                       Rust or reviewed C/C++ FFI
Advanced plugin hosting                     auxiliary laptop/desktop process preferred
Stock vendor samples/licenses               user-authorized import only; no distribution
Original C++ ClickEngine                    frozen validation/reference target
~~~

## Sources reviewed

- Official Rust Arm Linux: https://doc.rust-lang.org/stable/rustc/platform-support.html and https://dev-doc.rust-lang.org/stable/rustc/platform-support/arm-linux.html
- Rust ALSA: https://docs.rs/alsa/latest/alsa/
- CPAL: https://github.com/RustAudio/cpal
- Rodio: https://github.com/RustAudio/rodio
- dasp: https://github.com/RustAudio/dasp
- Symphonia: https://docs.rs/symphonia/latest/symphonia/
- Rubato: https://github.com/HEnquist/rubato
- rtrb: https://docs.rs/rtrb/latest/rtrb/
- Clack host: https://github.com/prokopyl/clack
- NIH-plug upstream maintenance notice: https://github.com/robbert-vdh/nih-plug
- VST3 Rust host status: https://docs.rs/vst3-host/latest/vst3_host/
- JUCE features and license: https://github.com/juce-framework/JUCE
- Carla features and license: https://github.com/falkTX/Carla
- sfizz C API: https://github.com/sfztools/sfizz
- FluidSynth C library: https://github.com/FluidSynth/fluidsynth
- Surge XT engine and binding: https://github.com/surge-synthesizer/surge ; https://github.com/surge-synthesizer/surge-rs
- Panfrost Midgard driver capability table: https://docs.mesa3d.org/drivers/panfrost.html
- Slint direct Linux KMS backend: https://github.com/slint-ui/slint/blob/master/docs/development/window-backend-integration.md
- CXX bridge: https://docs.rs/cxx/latest/cxx/
