# Plugins: the CLAP host

**Status:** 2026-11-14. Steps P1 (the host) and P2 (the sandbox) of the plugin plan are built and tested on the workstation and on ARMv7 under qemu. The code is [`rust/mpfree-clap`](../../../rust/mpfree-clap) and [`rust/mpfree-plughost`](../../../rust/mpfree-plughost). The app uses them for **plugin effects as mixer inserts** and **plugin synths as track instruments**, with the plugin's own presets on the Synth tab ([In the app](#in-the-app)). The plugin pack's Surge XT lists and loads its factory presets on ARMv7 under qemu; no third-party plugin has been tried on the MPC yet. The plan for plugins is in the synth skill tree (P1 to P6). The first plugin pack (P3) is Airwindows, Surge XT and Dexed (Jay, 13 Nov), CLAP only until P5: [the plugin pack](../guides/plugin-pack.md) builds them (14 Nov).

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
  - Per-note expression: a note's slide is its tuning expression (plus the bend when the bend goes as tuning), its tone the brightness expression, and its release velocity goes in its note-off. A note that starts under a held bend or slide starts there.
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
- **A plugin's own presets.** `Library::presets()` asks the file's CLAP preset-discovery factory (`clap.preset-discovery-factory/2`, or the draft id) for its providers, at most 64. Each provider's indexer names its file types and places:
  - a folder (a FILE location): the host walks it, sorted, skipping dot files and never following a linked folder, at most 8 deep, 16384 files and 4096 presets, and asks the provider for each file's presets;
  - inside the plugin (a PLUGIN location): the provider names its presets once, each with a load key.
  - A `Preset` has its name (else the file's name), place, load key, the CLAP plugin ids it is for, collection, creator, flags (factory, user, demo, favourite) and category: the plugin's own, else the folders between the location and the file, joined with " / ".
  - What can't be read is counted, and the first 16 reasons are kept (`Found::errors`), never hidden. A provider that fails without saying why is reported as "the plugin couldn't read" the file.
- **Loading one.** `ClapControl::load_preset()` hands it to the plugin's preset-load extension (`clap.preset-load/2`, or the draft id). The plugin's own error comes back as the reason (else "the plugin refused the preset"). The values are read again at once, and again when the plugin says it loaded or asks for a value rescan, because some plugins load in the background: Surge XT fades out, loads on a thread of its own and sends its new values as output events over the next blocks, silent meanwhile.
- **What the host offers plugins:** log (counted, never printed, since printing could block the audio thread), thread-check, params, state, latency, tail, audio-ports, note-ports and preset-load (a load's error, and "loaded").

## Threads and real time

CLAP names two threads. The main thread is the control thread that opened the first library. The audio thread is whichever thread calls `process`; it may change, but never runs two at once.

- `ClapNode::process` never allocates or locks. Its event list and audio buffers are made in `prepare`, and parameter values are atomics shared with `ClapControl`. `tests/no_alloc.rs` checks this with the counting allocator: 2000 blocks of notes, pressure, bend, parameters and an effect after the synth.
- `prepare` and dropping the node stop processing and deactivate from the control thread. Nothing else can touch the node at that moment, and for that moment the host's thread-check calls this thread the audio thread, which CLAP allows.
- A plugin's id, name and parameter list become `'static` data, because nodes describe themselves that way. Each distinct plugin is kept once, for the life of the program.
- The host can only check its own side. A plugin that allocates or blocks in `process` would still hurt the audio thread. That is what the sandbox (P2, below) is for: the plugin runs in its own process with a deadline each block, and gives silence and a count when it misses.

## P2: the sandbox

[`rust/mpfree-plughost`](../../../rust/mpfree-plughost) runs each plugin in its own process, `mpfree-plughost`, so a plugin that crashes, hangs or blocks can't take MPFree's audio thread with it. The node interface stays the same: a sandboxed plugin goes wherever an in-process one goes.

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

- **Listing a file.** `list(program, file)` starts the process with `--list FILE`. It loads the file, prints one tab-separated line per plugin (`plugin`, id, name, vendor, `instrument` or `effect`) and exits. A file that crashes or hangs while loading costs only that process: the host waits at most `CONTROL_TIMEOUT` and kills it. A plugin is an instrument when its CLAP features include `instrument`.
- **The program.** `Program` is the executable plus any words before the process's own arguments. `spawn(exe, ...)` uses the `mpfree-plughost` binary. `spawn_with(program, ...)` takes any program, such as the app itself as `mpfree plughost` (below).
- **Listing presets.** `presets(program, file)` starts the process with `--presets FILE`. It prints one tab-separated line per preset (`preset`, name, place as `plugin` or `file:PATH`, load key, flags, collection, category, creator, then the plugin ids; tabs, newlines and backslashes escaped), then an `error` line for each reason kept, `more-errors N` and `cut-short`. Finding presets runs the plugin's code over every file, so it never runs in a playing plugin's process; the host waits at most `PRESETS_TIMEOUT` (120 s). `Sandbox::load_preset()` sends a preset to the running process (`ToChild::LoadPreset`, the file and the load key) and gives back the plugin's answer (`ToHost::PresetLoaded`).
- **A home of its own.** `Program::with_home(dir)` starts every process with `HOME` and the XDG folders inside `dir` ([In the app](#in-the-app)).
- **State snapshots.** `Sandbox::refresh_state()` reads the plugin's state (a round trip) and keeps it. `SandboxNode::saved_state()` copies that snapshot without asking the process or waiting, for a project save from any thread but the audio thread's period. `restart_from_snapshot()` restarts with it. `Sandbox::orphaned()` says the node was dropped and only the handle is left.

### The rack

`mpfree_plughost::rack::Rack` holds the plugins found in a folder and the processes running:

- `Rack::scan(program, dir, shown)` lists every `.clap` file in `dir`, in name order, each with `list`. It returns lines for the report for what it found and for each file it left out: one that won't load, one with no plugins, a plugin id another file already has. It keeps at most 128 plugins. Node ids (`clap:` and the plugin's id), names and the "Plugin · Vendor" line are `'static`, kept for the program's life like the nodes' own descriptions.
- `Rack::make(node_id)` starts the plugin's process, takes a first snapshot and keeps the handle. The node waits for its process until `Rack::set_deadline` (half the block by default).
- `Rack::watch(now)`, every 250 ms from a control thread:
  - lets go of handles whose node is gone, which ends their process;
  - restarts a crashed process from its last snapshot, at most 3 times a minute. A fourth crash in that minute leaves the plugin down: an effect passes its input through, an instrument is silent. Making it again tries once more;
  - refreshes each running plugin's snapshot every 5 s.
- `Rack::refresh_states()` reads every snapshot now, before a project save.
- `Rack::presets(node_id)` lists a plugin's presets with `presets`, once per file: the list is kept and a second call is free. Its line says how many and the first error. `Rack::make_instance(node_id)` is `make` with the process's instance number, and `Rack::load_preset(instance, preset)` loads a preset into that process. Its snapshot is read again a second later, so a crash or a save soon after keeps the preset.
- Every start, failed start, crash, restart and give-up is a line (`take_lines`). `summary()` counts plugins found, running and down, crashes, restarts and failures. Neither waits: they use `try_lock`.

## In the app

[`rust/mpfree-app/src/plugins.rs`](../../../rust/mpfree-app/src/plugins.rs):

- **The plugin process** is `mpfree-plughost` beside the app (or `MPFREE_PLUGHOST`), and otherwise the app itself: `mpfree plughost ...` is handled first thing in `main` and runs the same process code. A stick with only the app on it still hosts plugins.
- **The device build is static** (`+crt-static`), and a plugin is a shared library that needs the MPC's own `libc.so.6` and `libgcc_s.so.1`. A static program loads one with glibc's static `dlopen`, which the linker warns about: the MPC's glibc must be the release the app was linked with. Ubuntu noble's cross toolchain links 2.39 and the MPC reports 2.39 ([ARMv7 userspace](../research/armv7-userspace-compat.md)). It works for the test plugins under qemu with the toolchain's ARM libraries standing in for the MPC's `/lib`; no plugin has been loaded on the MPC yet. If it fails there, a dynamically linked `mpfree-plughost` put beside the app takes over with no change to the app (the stick tool would have to copy it).
- **At start**, before the sound, the rack scans `files/plugins/` in the data folder. The lines go in the report.
- **A home of their own.** Plugins keep files for themselves: Surge XT makes `~/.Surge XT` (folders and a README) as it starts, and Dexed writes its built-in cartridges (about 280 KB) under `~/.local/share/DigitalSuburban`. So every plugin process runs with `HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME` and `XDG_STATE_HOME` inside `files/plugin-home/` (`Program::with_home`; the store makes the folder, [storage](storage-spec.md#what-is-saved-first-version)). On the MPC that keeps them on the data partition, never the root filesystem. Without a data folder (tests, previews) it is a folder of that run's own in the system's temp folder, never the real home.
- **The chain editor's picker** lists the plugin effects after the built-in ones, under the Plugins heading (`ChainState::plugins`, `ChainState::picks`, each a `Pick` with `Category::Plugins` from the FX registry). A tap sends `ChainAction::Add("clap:...")` as for any effect.
- **The builder thread** (`control.rs`, `mpfree-build`) builds a `clap:` id with `Rack::make`, so starting a process never happens on the audio thread. It also calls `Rack::watch` every 250 ms while it waits for jobs. A removed insert is dropped there too, and the next watch ends its process.
- **Nothing is hidden.** The audio loop passes the rack's lines to the report each period when there are any (no allocation when there are none), and the 30-second status adds `status: plugins: N found · running · down · crashes · restarts · failed`.
- **Plugin synths as track instruments.** `Rack::instruments` lists the plugins whose features include `instrument`. They follow the five engines on the Synth tab's "Track N instrument" row, and a pick makes the plugin on the builder thread and puts it in that track's slot of the track host ([Track host](track-host.md#in-the-app)). The Synth tab shows its parameters as for any node.
- **A plugin synth's own presets.** When a track gets a plugin synth, or a project opens with one, `MixHost::ask_presets` has the builder thread list its presets, once per plugin (`Job::Presets`, `Rack::presets`). The first time takes a few seconds: Surge XT's 627 factory presets take about 5 s on ARMv7 under qemu. Effects and instruments asked for meanwhile wait behind it.
  - The Synth tab lists them as "Category / Name" (`plugin_preset_name` in `main.rs`), in the order the plugin's folders sort.
  - A tap sends `SynthAction::Preset(k)`. The builder thread loads it into that track's running plugin (`Job::LoadPreset`, by the instance `Rack::make_instance` gave, kept in `Held::instance`). `PresetNews::Loaded` comes back: the log says the preset, and the plugin's reason is said on screen. A preset that lands after the track changed instrument is said and changes nothing.
  - The Synth tab looks again 1 s and 3 s later, for the values a plugin sends after loading in the background.
  - A list that can't be read is said in the log, and the tab lists no presets. A plugin without presets (Dexed) lists none.
  - The project keeps the plugin's state, so the sound is saved, but not which preset it came from: after an open no preset is marked.
- **Saved with the project.** A plugin effect is kept in the project's `mixer` part and a plugin synth in its `instruments` part ([browser.md](browser.md#the-parts)), each with its id, its file (`file=<name>.clap`, from `SandboxNode::save_extras`), its parameters and its state. `SandboxNode`'s `Node::save_state` is the rack's last snapshot (`saved_state`: taken as the plugin starts and every few seconds), so a save never waits on a process; the parameters, saved apart and applied after the state, keep a knob turned since. An open makes each plugin through the rack on the screen thread (`project::make_node`, which gets the rack through `ToUi::Plugins`) and hands it the state with `Node::load_state` (a round trip, before the node is prepared), which also becomes its snapshot, so a crash after the open restarts with it. A plugin whose file isn't in `files/plugins/` is left out with a line naming the file; a state the plugin refuses is said, and it keeps its own.
- **Not yet in the app:** the plugin's own Dev page row and an on-screen warning, and the `mpfree-plughost` binary on the stick (not needed: the app serves).

Not yet:

- The plugin's restart and rescan requests (CLAP `request_restart`, a parameter or port rescan) are seen inside the process but not passed back to MPFree.
- A sidechain input isn't passed to the process.
- The process's audio thread runs at normal priority. Real-time priority on the device comes with MPFree's own priority setup there.
- One process per plugin. If round trips cost too much on the MPC, several cheap plugins could share one process.

## Our test plugins

`mpfree_clap::testplug` holds two small CLAP plugins of our own, so the host is tested without anyone else's binary:

- **MPFree Test Synth:** 16 voices; Gain, Decay and Wave (an enum); two read-only meters (voices sounding and the transport's tempo, sent back as output events); a hidden parameter; state, tail and latency; CLAP and MIDI note dialects; pressure and tuning expressions; presets: three built in (a PLUGIN location, by load key) and the `.mptp` files in `$MPFREE_TESTPLUG_PRESETS`, else `$HOME/mpfree-testplug-presets` (a FILE location), through its own preset-discovery factory, and preset-load.
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
- the test synth's presets are found (built in and from a folder, with names, categories and flags), a file it can't read is said, and a preset loads by load key and from a file, its values following; an unknown one gives the plugin's reason (`tests/presets.rs`);
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
- a plugin file is listed from its own process, with the same names as in-process; a missing file and a missing binary fail cleanly; the list lines read back and junk is refused;
- a crash restarts from the snapshot, the node reads the snapshot, a gone process gives none and the old one stays, and a dropped node leaves the handle orphaned;
- a project's hooks on the node: `save_state` is the snapshot (none before one is read), `save_extras` names the plugin's file, and `load_state` into a new process before it is prepared brings the values back and becomes its snapshot; a state the plugin refuses changes nothing;
- `tests/presets.rs`: a plugin's presets are listed from a process of their own, once, with the user presets from the home the program gives; one loads into the running synth by its instance number and its values come back; an unknown preset or instance says why; the preset lines read back and junk is skipped;
- `tests/rack.rs`: a scan of a real folder finds the test plugins and says why it left out a duplicate and a file that isn't a library (and ignores a text file); a plugin made from the rack is restarted from its snapshot three times, then left off and dry on the fourth crash in a minute, with every step said; a dropped node's process is let go; an empty folder finds and says nothing.

`cargo test --locked -p mpfree-app --test plughost` runs the app as the plugin process: it lists the test plugins' file, refuses to run without the sandbox's arguments, plays the test gain through a rack whose program is `mpfree plughost`, and lists the test synth's presets (its user preset from the plugin home) and loads one through that rack. CI also runs it in the static ARMv7 step under qemu. There the test starts the app as `qemu-arm -L /usr/arm-linux-gnueabihf`, so the plugin's `libc.so.6` and `libgcc_s.so.1` come from the cross toolchain as they would from the MPC's `/lib` (unless `QEMU_LD_PREFIX` is set). Each run uses its own folder, and eight runs at once all passed.

These pass for `armv7-unknown-linux-gnueabihf` under qemu too. There, `MPFREE_PLUGHOST_TEST_EXE` names a wrapper that starts the ARM build of the process under qemu, because a process started from inside qemu-user runs on the host CPU. Under load (eight runs at once, 80 runs) they passed every time.

## Next

- **P3, ARM builds:** built on 14 Nov: [the plugin pack](../guides/plugin-pack.md) builds Airwindows (89 effects, through a small wrapper of ours), Dexed and Surge XT for the MPC from pinned commits, with their licenses and sources, and checks each file. Played under qemu; not on the MPC yet.
- **P4, passports:** automatic tests on the MPC for each plugin: does it load, does it stay free of NaN, does its state round-trip, what it costs per voice, its latency.
- In the app: plugins in the instrument browser, the rack's counts on the Dev page, and a plugin's presets in the Browse overlay with a search (627 in one list is long).
