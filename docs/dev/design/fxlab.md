<!--
covers: rust/mpfree-fxlab tools/fxprobe
checked: 2026-11-15 d58eb513
-->
# fxlab and FXProbe: measuring our effects against the MPC's

**Status:** built 2026-11-15. fxlab measures every MPFree effect and gates the ones that already meet the plan. FXProbe (two plugins made with Akai's plugin SDK), its controllers and its kit for the MPFree test stick pass every desktop test; **none of it has run on an MPC yet.** Setting the AIR effects' knobs automatically is still open (see [Setting the AIR knobs](#setting-the-air-knobs)). Plan: [effects quality plan](https://claude.ai/artifact/PPKs4R5JmcGZJWJMCQvPyV).

MPFree's 96 MPC effect names have to sound as good as Akai's AIR effects or better, in blind A/B listening. That needs two things: numbers that say where each of our effects stands, and recordings of the AIR effects made the same way so the numbers compare. fxlab (`rust/mpfree-fxlab`) is the measuring side, a desktop crate and command line. FXProbe (`tools/fxprobe`) is the recording side: an instrument plugin that plays fxlab's test programme into an AIR insert on the factory MPC, an effect plugin after it that records what comes out, and a script that runs both unattended.

## What fxlab measures

Every analysis works on a device under test (`dut::Dut`): any MPFree node through `NodeDut` (128-frame blocks, 120 BPM, transport playing), or a recording.

| Analysis | Module | Method | Numbers |
|---|---|---|---|
| Distortion and aliasing | `harmonics` | One sine on an odd FFT bin of a 65,536-point transform (coherent: no window, no leakage). Bins are sorted into fundamental, harmonics, aliases (harmonics above Nyquist folded back) and the rest. An alias counts only as a line 10 dB over its neighbours, so a chorus's smear is not called aliasing. | THD, harmonics 2 to 9, worst and total alias (relative and in dBFS), everything else |
| Frequency response | `response` | Exponential sweep (Farina) deconvolved, divided by a wire's spectrum, read at third-octave centres. | Gain and phase per band, delay |
| Loudness | `loudness` | ITU-R BS.1770-4: K-weighting, 400 ms blocks, gates at -70 LUFS and -10 LU; true peak by 4x oversampling. | LUFS change on pink noise, true peak |
| Nulls | `null` | Cross-correlation, then the fractional lag (golden section over half a frame) and least-squares gain that leave the smallest residual. | Residual in dB |
| Reverb and delay tails | `room` | Schroeder backward integral per octave; echo density (Abel and Huang); "metallic" excess (Welch spectrum of the tail against its third-octave average); IACC. | EDT, T20, T30 per octave, time to diffuse, tail excess, IACC |
| Dynamics | `dynamics` | Static curve from steady sines; attack and release from a 1 kHz sine stepping -40/0/-40 dBFS, gain tracked every millisecond. | Curve points, depth, attack, release (63 % and 90 %) |
| Zipper noise | `zipper` | Each continuous parameter swept end to end over 50 ms under a 997 Hz sine; the largest second difference while it moves against at rest. | Worst step in dB |

A **report card** (`card`) is one effect at one setting, measured by every analysis that fits its category, as named numbers (`resp_1k_db`, `thd_1k_-18_db`, `alias_5k_0_db`, `t30_1k_s`...). Effects that move frequencies (ring mod, frequency shifter, pitch, harmonizer, granulator) get no harmonics or response, which would mean nothing for them.

**Quality gates** (`tests/gates.rs`, run by `cargo test`): lines of the plan our effects already meet, so a change that makes one worse fails. Transparent effects are flat within 0.01 dB with THD under -140 dB; Drive HQ keeps aliases 90 dB down at 5 kHz full scale, Eco 50 dB; the limiter in true-peak mode holds true peaks within 0.5 dB of its ceiling on hot pink noise; master safety never passes its ceiling; every effect's quick card is finite and is quiet one second after its tail. Gaps the plan still has to close are reported by `fxlab cards`, not asserted.

### First findings (2026-11-15, all 45 engines, 238 cards)

- Character aliases at -41 dB (5 kHz, 0 dBFS) and Drive Eco at -53 dB: the stages without oversampling the plan names.
- Room and Spring reverb tails are 7 to 9 dB "metallic" (tail excess), the highest of any reverb: the 8-line FDN and the 40-stage spring.
- Level at defaults on pink noise: Talk box -6.6 LU, Auto wah -5.6, Comb -4.7, Granulator -4.4, Chorus -3.6, Rotary -3.2. A/B tests must be level matched; the defaults should be too.
- Zipper: the XY FX filter sweep, Perform FX and DJ filter show 45 to 47 dB steps while a knob moves, the delay's slapback 35 dB. Not yet listened to; to check before claiming.
- Relative alias figures blow up where an effect removes the fundamental (Drive's octave up, a resonant filter set low), which is why cards also give aliases in dBFS.

## The capture programme

`programme` lays every stimulus end to end with room for tails: a 250 ms sync chirp (500 Hz to 8 kHz, -12 dBFS), then sweeps, coherent sines, pink noise, an impulse, level steps, silence and, optionally, music clips. Its **schedule** is plain text, `rate` then one `id name start len gap stim` line per segment, where `stim` says what the segment is (`sweep:-12`, `sine:1000:-18`, `pink:-18`, `impulse`, `steps:-40:0:1`, `clip:FILE`, `silence`). Older five-column schedules still read.

- `Programme::standard` (about 50 s): sweeps at -24, -12 and 0 dBFS, sines at 100 Hz, 1, 5 and 10 kHz at -18 and 0 dBFS, pink, an impulse with 8 s of tail, level steps, silence.
- `Programme::short` (about 13 s): one sweep, two sines, pink, an impulse with 2 s of tail, silence. For setting sweeps.

A recording is lined up on the chirp (`programme::locate`), which went through the effect too, so a plain latency drops out, and cut by the schedule (`programme::cut`), each piece keeping its gap. `capture::analyse` reads the same report card from those pieces that `card::measure` reads from a device; on Drive the two agree within 1 dB (a test). `capture::render` plays the programme through one of ours so it is measured by the very same code as an AIR capture.

## Sweep plans

`plan` builds the list of settings to capture for one effect. Controls come from the registry for ours, or from a hand-written control list for an AIR effect (name, kind, range, default, unit, tab-separated; kinds `linear`, `log`, `stepped`, `choice`, `toggle`). A plan is the defaults, then every control across its whole range with the others at default (7 points by default, every choice of a choice), then a 3 by 3 grid for every pair of continuous controls (controls interact), then 32 random settings of everything at once spread by a Latin hypercube, each in every context (a programme at a gain the Source applies). Duplicates are dropped. Tempo is a project setting on the MPC, so tempo contexts are separate plans in separate projects.

A typical effect comes to 150 to 400 runs: about 1.4 hours of capture at 13 s each on one slot, minutes when several tracks run at once. `fxlab sweep` renders the same plan through one of our effects in about half a second per run.

## FXProbe: recording the factory MPC

The plugin SDK (MPC OS 3.10.1 and later, [inmusic-software/mpc-plugin-builder-sdk](https://github.com/inmusic-software/mpc-plugin-builder-sdk)) loads VST2-ABI ARM libraries into the MPC app. FXProbe is two of them, plus a controller:

- **FXProbe Source** (instrument): plays a programme WAV sample for sample when told to, optionally at a gain. It can also start on any MIDI note or when the transport starts (the Trigger parameter).
- **FXProbe Tap** (insert effect, after the AIR insert): records what reaches it to a 32-bit float WAV with a report (frames, overruns, non-finite samples, peak, block sizes, the transport's tempo). It passes audio through, or mutes it so an unattended run is not heard. With its timing log on it also writes, per process call, the monotonic time, frame count and CPU, headed by the audio thread's scheduling policy and priority, and every host call since it was created (for the SDK integration work).
- **`tools/fxprobe/fxprobe.sh`** (on the MPC, BusyBox sh): run from the FXProbe kit on the MPFree test stick in the developer-mode shell; installs the plugins on the internal drive, makes the work folder on the stick, writes commands and waits for the plugins (see [On the test stick](#on-the-test-stick)).
- **`tools/fxprobe/fxprobe.py`** (workstation): the same over developer-mode SSH for plugins on a drive the app loads from, with the work folder beside them, and copies captures back.

They talk through files in one **work folder**: `FXProbe` beside the drive's `VST2 Plugins` folder, or the folder named by a `workfolder.txt` pointer in the plugin's own folder (below): `control.txt` (a numbered command: `run` with programme, gain, preroll, capture name and length, optionally `timing 1` and a list of slots), `programmes/`, `captures/` and `status/<kind>-s<slot>.txt`. Each instance has a **slot** (its Slot parameter, or "Auto": the lowest number no live instance of the same plugin holds), so several tracks, each Source then an AIR effect then a Tap, capture at once and their files do not collide.

Rules the plugins keep:

- **Real time:** the audio thread copies samples into a lock-free ring and reads and writes atomics, nothing else. A worker thread per instance does every file operation. A programme is loaded into the buffer the audio thread is not using and handed over between blocks.
- **Quiet by default:** with no work folder they do nothing at all, and they never create it. A command already in `control.txt` when an instance starts, or when the work folder appears, is ignored, so loading a project or putting a stick back never fires an old command. Names from the control file must be plain file names.
- **Where they write:** only inside the work folder. The plugins write nothing else; `fxprobe.sh install` puts the plugin folders themselves on the internal drive, and `remove` takes them off.
- **Risk:** a plugin runs inside the MPC app with no sandbox; a crash in it stops the app until it restarts. Both pass the SDK's host simulator (MPC load order, block sizes 32 to 512, random parameters, state round trip, in-place processing) and its validator for 32- and 64-bit ARM.

### Setting the AIR knobs

The plugins can't reach another plugin's parameters, and the project file stores effect settings as an undecoded binary blob ([MPC import](mpc-import.md)). The routes, best first, each to be confirmed by the read-only 3.10.1 survey:

1. **Automation in a generated sequence.** If insert-effect automation in a project's sequences can be written, one project per effect steps through a plan while a Source note starts each run; only loading and playing the project is left.
2. **MIDI.** The MPC has MIDI learn; if the app listens to an ALSA sequencer port, a virtual port on the MPC can send it the learned controls. Learning is a one-time step per effect.
3. **Simulated touch or mouse.** 3.10.1 supports a USB mouse and screenshots; a virtual input device plus screenshots to confirm each value works for any effect but is the most fragile.

Until one works, `fxprobe.sh capture` (or `fxprobe.py capture`) records every slot as it is set (presets, or settings made once by hand), across levels, with no further input.

### On the test stick

FXProbe goes on the MPFree test stick as a developer tool run under Akai's system, so a device round can use it without a second stick. `stage.py` makes the **kit**: both plugins built at the pinned SDK commit, fxlab's short and standard programmes with their schedules, `fxprobe.sh`, `VERSION` and `SHA256SUMS`, about 23 MB. The stick build copies it, among the stick's developer tools, to `/mpfree-dev/fxprobe` on the stick's kits partition (partition 3, rewritten with every stick); the work folder is `FXProbe/` on its data partition (partition 4, `MPFREE-DATA`, kept across rewrites and large enough for sweeps: a capture is 4.7 MB with the short programme, 17.6 MB with the standard one, per slot and gain). MPFree itself ignores that folder; it reads only `FORMAT`, `settings/` and `files/` ([storage spec](storage-spec.md)). Its store does refuse a partition that holds other files and no `FORMAT`, so `setup` makes `FXProbe/` only where MPFree's `FORMAT` already is (MPFree has started from that stick once).

Two facts of the factory system shape it:

- **Plugins can't load from the stick.** Akai's disk service mounts every partition of a USB stick `noexec` (seen on 3.10.1, [stick autoboot](../hardware/stick-autoboot.md)), and the kernel refuses to map a library from a noexec mount; the SDK's own tool warns about it and recommends the internal drive. So `fxprobe.sh install` copies the two plugin folders (about 1.1 MB) into `VST2 Plugins` on the internal drive (`/media/az01-internal`, the MPC's `/data` partition), checked against the kit's hashes through a staging folder, and `remove` deletes exactly those folders. They are listed when the app next starts.
- **Switching on with the stick in starts MPFree** once MPFree's boot hook is installed, so the app starts without the stick and the stick goes in once it is up. Akai's system may then mount the data partition as `/media/MPFREE-DATA` or under its UUID. `fxprobe.sh` finds the data partition as the partition after the kit's on the same disk, and the plugins find the work folder through their pointer: `install` writes `/media/*/FXProbe` into each plugin's `workfolder.txt`, and a pointer with a `*` names the one matching folder that holds the marker `.fxprobe-work` (`setup` writes it last). With none, or more than one, the plugins stay idle; they look again at most once a second, so the stick can arrive after they load or come back under another name.

The steps, each labelled read-only or state-changing with its undo, are in [tools/fxprobe/README.md](../../../tools/fxprobe/README.md#on-the-mpc-from-the-test-stick-draft-not-yet-rehearsed-on-a-device); the device round's runbook rehearses them before Jay runs them.

## Running it

On a workstation (no MPC):

```sh
cd rust && cargo build --release -p mpfree-fxlab
./target/release/fxlab cards --presets --out /tmp/cards        # every engine and preset
./target/release/fxlab plan --effect mpfree.fx.flanger --gains 0,-12 --out /tmp/plan
./target/release/fxlab sweep --effect mpfree.fx.drive --out /tmp/sweep
./target/release/fxlab programme --short --out /tmp/prog         # short.wav + short.tsv
./target/release/fxlab analyse CAPTURE.wav --schedule /tmp/prog/short.tsv --gain 0 --out /tmp/an
MPC_SDK=/path/to/sdk python3 ../tools/fxprobe/test/run_chain_test.py   # FXProbe end to end
python3 ../tools/fxprobe/stage.py --sdk /path/to/sdk --out ~/mpfree-work/fxprobe-kit   # the stick's kit
```

On the MPC, building and installing the plugins and each `fxprobe.sh` and `fxprobe.py` step are in [tools/fxprobe/README.md](../../../tools/fxprobe/README.md), each labelled read-only or state-changing with its undo. Captures of AIR effects are Akai's output: they stay out of this repository ([factory content policy](../project/factory-content-policy.md)).

## Tests

- `cargo test -p mpfree-fxlab`: 38 unit tests (each analysis on signals with known answers, schedules, plans, capture analysis against direct measurement) and the six quality gates.
- `tools/fxprobe/test/run_chain_test.py` (by hand; needs the SDK checkout and a C++ compiler): builds both plugins and `chain_sim`, a small host that runs plugins in series the way an MPC track does, then checks that a wire capture holds the programme bit for bit at the frame fxlab finds, that a capture through the SDK's delay example is complete with its gain, timing log and host-call log, that `fxprobe.py` setup, capture and pull at two gains read back through `fxlab analyse` as a flat wire, and the stick route end to end: a staged kit, `fxprobe.sh install` to a fake internal drive, the installed plugins finding the work folder through their pointer (also after the stick comes back under another mount name, without rerunning the old command), and captures that read back as a flat wire.
- `tools/fxprobe/test/test_fxprobe_sh.py` (CI, under BusyBox): `stage.py` with stand-in builds, and every `fxprobe.sh` mode against a fake MPC root and stand-in plugin instances, refusals included.
