# Plugins: the CLAP host

**Status:** 2026-11-11, steps P1 (the host) and P2 (the sandbox) of the plugin plan, host-only and tested on the workstation and on ARMv7 under qemu. The code is [`rust/mpfree-clap`](../../rust/mpfree-clap) and [`rust/mpfree-plughost`](../../rust/mpfree-plughost). Nothing in the pad player uses them yet, and no third-party plugin has been tried. The plan for plugins is in the design doc "MPFree synths and plugins: design" and the synth skill tree (P1 to P6).

MPFree runs native engines first (Poly, FM, the drum, pluck and wavetable synths). Plugins add what we don't build, which means CLAP, LV2 and VST3 plugins built for ARM Linux. Desktop builds can't run on the MPC's ARM chip, and Akai's own plugins aren't a path. CLAP comes first: it is MIT-licensed, plain C, and made for hosts like ours (sample-exact events, per-note expression, a clear threading model).

## What P1 does

A CLAP plugin becomes an `mpfree_node::Node`, so anything that holds a node can hold a plugin: a [track host](track-host.md) slot, a mixer insert chain, an offline render.

- **Loading.** `Library::open(path)` loads a `.clap` file with `dlopen`, checks its CLAP version (any 1.x) and starts it. `Library::plugins()` lists what is inside: id, name, vendor, version and feature tags. Open each file once and share the handle. The file stays loaded while any plugin made from it lives.
- **Making one.** `Library::create(id)` makes a `ClapNode`. A plugin with a note input is an instrument and adds into the buffers. One without is an effect and transforms them. `prepare(rate, max_block)` activates it. Its `NodeInfo` id is `clap:` followed by the plugin's id, such as `clap:org.surge-synth-team.surge-xt`.
- **Parameters.** Each visible CLAP parameter becomes a `ParamInfo`, so the Synth tab's generic pages, Q-Links, automation and the mod matrix work on it unchanged.
  - The id is the parameter's position. `clap_param_id(id)` gives the CLAP id to store in a project.
  - Stepped parameters are whole numbers. An enum starting at 0 with up to 64 values gets its names from the plugin.
  - The first part of CLAP's module path is the group.
  - Hidden parameters are left out, and read-only ones are meters.
- **Events, on their exact frame.**
  - Note on and off.
  - Pad pressure, as CLAP's pressure expression.
  - Bend, as MIDI pitch bend when the plugin takes MIDI. Otherwise it is a tuning expression on each held note, 2 semitones at full bend.
  - `AllOff`, as a note-off for every held note.
  - Parameter changes.
  - The transport (tempo, position in beats and seconds, playing) goes with every block.
  - Parameter values the plugin sends back (its own meters, its own changes) show in `param()`.
- **Blocks.** Blocks longer than the plugin was activated for are cut into pieces. Events and parameter changes still land on their frame, and the output is the same.
- **When a plugin fails.** If `process` returns an error, the block is counted. An instrument outputs silence and an effect passes its input through. A plugin that won't activate is counted and stays silent.
- **State and the main thread.** `ClapNode::control()` gives a `ClapControl` for the control thread:
  - `save_state()` and `load_state()` give the plugin's state as bytes for the project file. After a load, the node's values follow.
  - `param_text()` shows a value the way the plugin writes it.
  - `idle()` runs the plugin's `on_main_thread` when it asked for a callback. It also reports what else the plugin asked for: a restart, a parameter rescan, a ports rescan, latency or tail changes, or unsaved state.
- **What the host offers plugins:** log (counted, never printed, since printing could block the audio thread), thread-check, params, state, latency, tail, audio-ports and note-ports.

## Threads and real time

CLAP names two threads. The main thread is the control thread that opened the first library. The audio thread is whichever thread calls `process`; it may change, but never runs two at once.

- `ClapNode::process` never allocates or locks. Its event list and audio buffers are made in `prepare`, and parameter values are atomics shared with `ClapControl`. `tests/no_alloc.rs` checks this with the counting allocator: 2000 blocks of notes, pressure, bend, parameters and an effect after the synth.
- `prepare` and dropping the node stop processing and deactivate from the control thread. Nothing else can touch the node at that moment, and for that moment the host's thread-check calls this thread the audio thread, which CLAP allows.
- A plugin's id, name and parameter list become `'static` data, because nodes describe themselves that way. Each distinct plugin is kept once, for the life of the program.
- The host can only check its own side. A plugin that allocates or blocks in `process` would still hurt the audio thread. That is what the sandbox (P2, below) is for: the plugin runs in its own process with a deadline each block, and gives silence and a count when it misses.

## P2: the sandbox

[`rust/mpfree-plughost`](../../rust/mpfree-plughost) runs each plugin in its own process, `mpfree-plughost`, so a plugin that crashes, hangs or blocks can't take MPFree's audio thread with it. The node interface stays the same: a sandboxed plugin goes wherever an in-process one goes.

- **Starting one.** `spawn(exe, plugin_file, id)` starts the process for one plugin. It returns a `SandboxNode`, which is a `Node` with the same `clap:` id and the same parameters as the in-process node, and a `Sandbox`, the control-thread handle. The process loads the plugin with `mpfree-clap` and reports its name, kind and parameters. A missing file or plugin id comes back as `Error::Plugin` with the reason.
- **Each block.** The node writes the block's context, events and input into memory it shares with the process (a memfd of about 144 KB). It bumps a sequence number and wakes the process with a futex. The process runs the plugin on its own audio thread, writes the output and the parameter values back, and bumps its own number. The node waits for that until a deadline: half the block's length by default (`Deadline::Fraction(0.5)`), or a fixed time for offline work (`Deadline::Fixed`).
  - **On time:** the plugin's output, sample for sample what it gives in-process.
  - **Late, a miss:** an instrument adds nothing and an effect passes its input through. The late block's events still reach the plugin. Until the process catches up, the node doesn't post again, and those blocks count as **busy**.
  - Blocks longer than the size the process was prepared for are cut into pieces, one round trip each.
  - A `reset()` reaches the plugin just before its next block.
- **Crashes.** `Sandbox::poll()`, called from the control thread (every UI frame, say), finds a process gone. It gives the exit code, or the signal as a negative number. From then on the node is silent without waiting, and those blocks count as **down**. `Sandbox::restart(state)` starts a new process on the same shared memory, prepares it as the node was, and loads state saved earlier with `save_state()`. The process dies with MPFree (`PR_SET_PDEATHSIG`), and a crash leaves no core file.
- **Main-thread work.** Prepare, state and a value's text go over a socket from the control thread. The process answers the plugin's own main-thread requests (`on_main_thread` callbacks) itself, after each message and every 100 ms.
- **Counters.** `SandboxNode::stats()` gives blocks, misses, busy, down, crashes, restarts and the plugin's own failed blocks.

Real time: the node's side of a block never allocates or locks. `tests/no_alloc.rs` checks this with the counting allocator, including a miss and busy blocks. Its wait is a futex wait, bounded by the deadline. The price is one round trip per block: a wake, two context switches, and copying the block in and out (an effect's input both ways).

`cargo run --release -p mpfree-plughost --example roundtrip` (from `rust/`) measures it with the test gain at the MPC's period (128 frames at 44.1 kHz), in-process and sandboxed. On the 4-core cloud workstation VM (2026-11-11) the round trip was 46 us on average and 110 us at the 99th percentile: 1.6 % and 3.8 % of the 2.9 ms period. The worst block took 3.6 ms, a scheduler stall in the VM, which at the default deadline would have been a miss. The MPC's numbers aren't measured yet. They come with the plugin passports (P4), with the process's audio thread at real-time priority.

Not yet:

- The plugin's restart and rescan requests (CLAP `request_restart`, a parameter or port rescan) are seen inside the process but not passed back to MPFree.
- A sidechain input isn't passed to the process.
- The process's audio thread runs at normal priority. Real-time priority on the device comes with MPFree's own priority setup there.
- One process per plugin. If round trips cost too much on the MPC, several cheap plugins could share one process.

## Our test plugins

`mpfree_clap::testplug` holds two small CLAP plugins of our own, so the host is tested without anyone else's binary:

- **MPFree Test Synth:** 16 voices; Gain, Decay and Wave (an enum); two read-only meters (voices sounding and the transport's tempo, sent back as output events); a hidden parameter; state, tail and latency; CLAP and MIDI note dialects; pressure and tuning expressions.
- **MPFree Test Gain:** a stereo effect, with a Fail switch that makes `process` return an error. For the sandbox's tests only, a Stall setting makes `process` sleep and a Crash switch makes it abort its process.

`rust/mpfree-clap-testplug` builds the same two as a real shared library, `libmpfree_clap_testplug.so`, which the tests load with `dlopen`.

## Tests

`cargo test --locked -p mpfree-clap` (from `rust/`):

- the bindings' struct sizes and field offsets match what gcc gives the real CLAP 1.2.2 headers, on x86_64 and on ARMv7 hard-float;
- a library lists its plugins and refuses an unknown id;
- the synth is an instrument with its parameters mapped (names, groups, ranges, enum names, meters, hidden left out, CLAP ids both ways), and two instances share their descriptions;
- a note adds into the buffers from its frame and dies away after its note-off;
- the same notes, parameters, pressure, bend and `AllOff` give the same samples in blocks of 1, 7, 64, 100 and 1000 frames, and with blocks longer than the activation size;
- a parameter change lands on its frame (exactly 2x from that frame for +6 dB), `set_param` clamps, a change queued before a `prepare` survives it, and meters can't be set;
- the plugin's meters (voices, tempo) reach `param()`;
- state saved from one instance loads into another, junk state is refused, and `param_text` gives the plugin's names;
- the gain is an effect, ignores notes, and a failed block leaves the input dry;
- the `.clap` file loads with `dlopen` and sounds exactly like the linked copy; a missing file and a library without `clap_entry` are refused;
- a callback request reaches `idle()` once;
- the entry's `init` and `deinit` balance, and the file stays started while a plugin or its control handle lives (`tests/lifecycle.rs`);
- no allocation on the audio thread's path (`tests/no_alloc.rs`);
- the test synth as a track host instrument: it plays on its track, takes a parameter from the host, rings out after a swap and comes back to be dropped; the test gain as an insert in a mixer strip's chain (`tests/track.rs`).

The same tests pass for `armv7-unknown-linux-gnueabihf` under qemu, including the `dlopen` test.

`cargo test --locked -p mpfree-plughost` (from `rust/`), with the real `.clap` file in a real `mpfree-plughost` process:

- the control messages round-trip and junk is refused; the process's arguments and events read back;
- the sandboxed node has the same id, name, kind, parameters and values as the in-process one;
- the test synth gives exactly the same samples sandboxed as in-process, for notes, pressure, bend, a parameter event and `AllOff`, and the values it sends back arrive; a reset silences it in both;
- the test gain matches in-process sample for sample, with blocks cut to the prepared size;
- a plugin that sleeps 1 s in `process` against a 30 ms deadline: the node doesn't wait for it, the effect stays dry, the next block is busy, and once it wakes the queued change goes with the next block;
- a frozen process (`SIGSTOP`) is a silent miss for an instrument, and when it thaws the note from the missed block sounds;
- a plugin that aborts its process: a miss, `poll()` sees `SIGABRT`, the node is down without waiting, and `restart()` brings it back with the saved state; a process killed from outside is counted once and plays again after a restart;
- state saved in one process loads in another, junk state is refused, and value text matches in-process;
- a missing plugin id, a missing file and a missing binary fail cleanly;
- dropping the node and the handle ends and reaps the process;
- a sandboxed synth plays on a track host track (`tests/sandbox.rs`);
- no allocation on the node's side of a block (`tests/no_alloc.rs`).

These pass for `armv7-unknown-linux-gnueabihf` under qemu too. There, `MPFREE_PLUGHOST_TEST_EXE` names a wrapper that starts the ARM build of the process under qemu, because a process started from inside qemu-user runs on the host CPU. Under load (eight runs at once, 80 runs) they passed every time.

## Next

- **P3, ARM builds:** reproducible armv7 build recipes for free plugins (Surge XT, Dexed, Airwindows first), with their licenses and sources.
- **P4, passports:** automatic tests on the MPC for each plugin: does it load, does it stay free of NaN, does its state round-trip, what it costs per voice, its latency.
- In the app: a plugin as a track host instrument and as a mixer insert, and plugins in the instrument browser. The pad player's wiring goes through the Hello MPC work.
