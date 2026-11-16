# Importing MPC projects

**Status:** feasibility done; Stage 0 tools built, waiting on the owner's own projects, which are on an SD card or USB drive rather than the Internal drive (2026-11-15). Stage 1 started: keygroup programs to MPFree kits (`rust/mpfree-mpc-import`, 2026-11-14); its field readings checked against a real factory keygroup program's report on 2026-11-15, not yet played side by side with the MPC. Full assessment: [Importing MPC projects: feasibility](https://claude.ai/artifact/1CzE52mppbCaNWMYsVLVm8). Plan for projects, expansions, factory content and Akai's instruments: [MPC projects and Akai content in MPFree](https://claude.ai/artifact/6fd7m8ewvGfJDx9ts9G8DT) (2026-11-15).

MPFree will open a project saved by Akai's MPC software (OS 3.x, and later 2.x) and turn it into an MPFree project. The importer only ever reads MPC files and writes a new MPFree project; it never writes to the MPC.

## What the files are

Community reverse-engineering, not Akai documentation; confirmed for OS 3.7 files, **not yet for 3.6**:

- **OS 3.x:** `NAME.xpj` is gzip; unpacked, five text lines (`ACVS`, firmware version, object type, `json`, platform) then one JSON document holding tracks, programs, sequences, songs and the mixer. Samples are WAV files in `NAME_[ProjectData]/`.
- **OS 2.x:** `NAME.xpj` is an XML manifest, with `.xpm` programs, an `.xal` sequence list and `.sxq` sequences (MIDI files whose notes are Akai meta events) in the ProjectData folder.
- Time is 960 PPQ, the same as `mpfree_transport::PPQ`.
- Effect and plugin-synth settings are an undecoded binary blob. Their names, ids and bypass state are readable; their knob values are not.

## Confidence

High: samples, tempo and meter, notes. Medium-high: drum programs, mixer levels and routing. Medium: keygroups, probability, per-note tuning. Low: automation, audio tracks, songs and Arrange, effect settings, plugin synths, Q-Links. Details per area are in the assessment.

## Stages

0. **Corpus and inspector:** the owner's own projects, saved on OS 3.6 and on 3.10.1 (the owner's MPC runs 3.10.1 since 2026-11-15), read by an inspector that dumps their structure and flags fields that differ from the public 3.7 notes.
1. **Kits:** samples, drum and keygroup programs, and standalone `.xpm` files.
2. **Sequences:** notes, tempo, meter, loops; checked against the MPC's own Audio Mixdown.
3. **Mixer:** levels, pans, sends, submix and return routing; effects by name, at default settings.
4. **The rest:** songs and Arrange, audio tracks, automation, per-note modifiers.
5. **In the app:** import from the Browser page, reading a USB stick.

Every import writes a report of what came across exactly, what was approximated and what was dropped. The importer writes an MPFree project folder as the browser saves one ([The browser and projects](browser.md): `project.txt` plus one `parts/<owner>.txt` per engine through `mpfree_library::Part`, and `samples/`), so each stage writes the parts of the engines it fills.

## Stage 1: keygroup programs

`rust/mpfree-mpc-import` reads a keygroup program (`.xpm`) and the samples beside it, as the MPC saves a program folder, and writes a new MPFree project holding one kit. It runs on a computer:

```sh
cargo run --release -p mpfree-mpc-import -- keygroup "Grand/Grand.xpm" --data /media/stick/data [--name NAME] [--release-ms 400] [--dry-run]
```

`--data` is the folder the app is given with `--data`; the project goes into its `library/Projects/`. The tool only reads the program folder and never replaces a project. It prints a report: how the program was read, the pads made, and each thing approximated or left out.

- **What it reads.** Each `<Instrument>` with a sample is a keygroup: `LowNote` and `HighNote`, its own `Volume`, `TuneCoarse`, `TuneFine` and `VelocitySensitivity`, and its active `<Layer>`s with `VelStart`, `VelEnd`, `SampleName` (or `SampleFile`), `RootNote`, `TuneCoarse`, `TuneFine`, `Pitch`, `Volume`, `Pan`, `SampleStart`, `SampleEnd` and the loop points. A small XML reader of our own (`xml.rs`, bounded in size and depth) does the parsing.
- **Checked on a real program (2026-11-15).** The report of a factory keygroup program (MPC 2.9, `File_Version` 2.1; 21 keygroups, 4 velocity layers) confirmed the readings above:
  - `RootNote` is the MIDI note + 1 (84 of 84 layers agree with their sample names).
  - `Volume` is a linear gain (0.707946 is -3 dB) and `Pan` 0.5 is the centre, on the program, each keygroup and each layer.
  - `VelocitySensitivity`, the tuning, the envelope and the filter sit on each keygroup, not the program.
  - The envelope and filter values are the MPC's 0 to 127 knob positions over 127 (`VolumeRelease` 0.377953 is 48), so their times and frequencies are still unknown. The report shows them as knob positions.
- **Root notes.** Programs seen in community notes write `RootNote` either as the MIDI note or one above it. The tool compares each `RootNote` with the note in its sample's name (by pitch class, since libraries number octaves from C3 or from C4) and takes 0, +1 or -1, whichever most layers agree with, and says which. With no names to check, it takes `RootNote` as the note.
- **The kit.** Each keygroup becomes a pad with keys (its notes and root) and its samples as velocity layers, and the pads play notes, as an auto-sampled instrument does (`Project::put_instrument`). Layers play while held (Gate) and release over `--release-ms`, since the times behind the program's envelope knob positions aren't known; they are in the report. Levels are relative to the program's most common layer volume (times its keygroup's). Velocity sensitivity is each keygroup's.
- **Approximated, and said.** A keygroup whose layers have different roots gets the first layer's root, and each other layer is tuned to make up the difference. A layer that doesn't follow the keys on the MPC follows them here. More than 16 layers keep the first 16 by velocity. A sample that isn't beside the program, doesn't read or holds no audio is left out, and the rest of the kit is made. The layer below it, or failing that the one above, then takes its velocities, so no hit falls silent (the factory program above has 10 empty sample files). A layer under half as long as its keygroup's longest is reported as possibly cut short.
- **Reported, not carried over.** The envelope, each keygroup's filter (a darker sound on the MPC) and the program's effects, by name.
- **The project** has only the sampler's part, so opening it puts the kit on the pads and the other engines keep what they had.

To check against a real program: the field meanings above are inferred, so the first real keygroup program (the owner's own, or one from the factory library copied out for local use) should be imported with `--dry-run` and its report compared with how it plays on the MPC.

## Inspecting projects

`tools/mpc-import/inspect-xpj.py PATH` reads a `.xpj`, a folder of them, or the `mpc-projects.zip` below. For each project it prints the format and header lines, a summary (tempo, tracks and programs, pads and layers, inserts by name, sequences with their clips, notes and automation ids, songs, and samples missing from ProjectData), and an importer checklist: each field stages 1 to 3 will read, flagged `MISSING` or `TYPE` when it differs from the public 3.7 notes. `--shape` adds every field path with its types and counts. It never prints a settings blob (only its length) or audio, so its report is safe to share in the thread.

### Reading projects from a card or stick

The owner's projects are on an SD card or USB drive, so the corpus comes straight from that card. Put it in the workstation and run the inspector on it; it only reads, walks every folder for `.xpj` files and looks at each project's ProjectData folder:

```sh
lsblk -o NAME,SIZE,LABEL,MOUNTPOINT
python3 tools/mpc-import/inspect-xpj.py "<mount point>" --out ~/mpfree-work/inspect-projects.txt
```

Upload `inspect-projects.txt` to the project thread. Include at least one project saved on 3.10.1, to see whether the format moved since 3.7, and a few that use plugin instruments and insert effects.

## Getting test projects out of the eMMC backup

`tools/mpc-one/offline-mpc-projects.py` runs on the workstation only. It reads the newest `~/mpfree-backups/mpc-one-emmc-*/mmcblk0.img` with debugfs (read-only, nothing mounted, no sudo), finds the `data` partition (p8, the MPC's Internal drive), and lists every `.xpj` project and `.xpa` archive with its format line. With `--copy 1,3` or `--copy all` it copies only those projects' `.xpj` and `_[ProjectData]` folders into `~/mpfree-work/mpc-projects-<UTC>-*/`, checks each file's size, and writes `SHA256SUMS` and `mpc-projects.zip`. Nothing else on the partition is copied.

Projects saved to an SD card or USB drive, or after the backup was made, are not in it. **The 2026-11-09 backup's Internal drive holds no projects** (186 entries, no `.xpj` or `.xpa`; checked 2026-11-12 and again 2026-11-15), so use [the card](#reading-projects-from-a-card-or-stick) instead.

**Never commit MPC projects, their samples or blobs.** They may hold Akai factory or expansion sounds ([factory content policy](../project/factory-content-policy.md)). Test files stay local and outside the clone.

## Programs, expansions and factory content

Most of what musicians call their sounds is WAV files plus an `.xpm` map, so it follows from the stage 1 reader:

- **Programs and expansions from a card.** The Browser opens drum and keygroup `.xpm` programs and expansion folders (`Expansion.xml`, programs, samples) on a USB stick or SD card. Community notes say a downloaded `.xpn` is a zip of that folder; check one real `.xpn` before relying on it.
- **Factory content in place.** On the MPC, MPFree mounts the `content` partition (p7, Akai's factory library) **read-only** and shows it as a Browser location, so factory kits and keygroups open from Akai's own files. Nothing is copied off the device or shipped by MPFree. The workstation copy tools stay private until a legal review ([factory content policy](../project/factory-content-policy.md)).
- **Plugin presets** (programs that hold a plugin's state instead of samples) come in by name only, like plugin tracks below.

## Plugin tracks and Akai's instruments

A track played by one of Akai's plugin instruments (Bassline, Electric, Drumsynth Multi, Hype, Mellotron, Odyssey, Solina, TubeSynth, and the paid ones) imports as its notes and the instrument's name: the sound lives in Akai's code. Insert effects import by name at default settings. Two ways to keep the sound:

- **Before moving a project:** freeze or bounce each plugin track to audio on the MPC, or export stems, then save. The importer brings the audio and the original notes. The manual's page on moving projects should say this when stage 2 ships, and every import report names the plugin tracks it found.
- **Playing Akai's instruments from MPFree.** The owner has already activated them on this MPC; the goal is to play them from MPFree on the same machine with that activation, used as it is (the owner, 2026-11-15). If they are separate VST2-interface libraries that check their own licence, the [MPC plugin host](mpc-plugin-host.md) could load them. If they are built into Akai's app, or ask Akai's app for their licence, MPFree runs Akai's own unmodified app in the background as a sound engine when a project needs it ("guest mode", which the owner wants), sending it notes and taking its audio back. The line kept, as in [the SDK research](../research/mpc-plugin-sdk.md): MPFree never patches Akai's code, never answers a licence check in Akai's place and never makes activation data. The cost to plan for: Akai's app holds about 1,019 MiB resident, nearly all locked, of about 2,019 MiB with no swap ([hardware ownership](../research/hardware-ownership.md)), so it starts only when needed and stops once its tracks are printed to audio.

Which of the two applies is settled by a read-only scan of Akai's 3.10.1 system and of its running app (`tools/mpc-one/offline-akai-scan.py` and `tools/mpc-one/read-only-akai-app.sh`, written by the synths thread, not merged as of 2026-11-16). Until then, the owner can switch back to the factory software.

## Sources

- [kurtjcu/MPC-project-file-definitions](https://github.com/kurtjcu/MPC-project-file-definitions) (no license stated: we learn from it and write our own code)
- [andyg5000-agent/mpc2reaper](https://github.com/andyg5000-agent/mpc2reaper) (MIT)
- [jeffwoods/mpc-mcp](https://github.com/jeffwoods/mpc-mcp)
- [Akai: which plugins are on which MPC](https://support.akaipro.com/support/solutions/articles/69000872472-akai-pro-mpc-series-why-are-some-plugins-not-available-on-my-mpc-), [Akai: authorizing MPC instruments](https://support.akaipro.com/en/support/solutions/articles/69000817006-akai-pro-mpc-instruments-fx-how-to-install-and-authorize-mpc-standalone-desktop), [Akai: installing .xpn expansions](https://support.akaipro.com/en/support/solutions/articles/69000826395-akai-pro-mpc-how-to-install-xpn-expansions)
