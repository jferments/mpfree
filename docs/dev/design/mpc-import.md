# Importing MPC projects

**Status:** scope narrowed on 2026-11-17 by the owner: **sounds and patterns only.** MPFree will not open MPC projects as projects. Musicians bring their kits and programs, their patterns as MIDI files exported from the MPC, and audio stems or bounces. Stage 1 is built in `rust/mpfree-mpc-import`: keygroup programs (2026-11-14; field readings checked against a real factory keygroup program's report on 2026-11-15, not yet played side by side with the MPC) and drum programs (2026-11-17; field readings from community notes, **not yet checked on a real drum program**). The musicians' side is the manual page [Moving your beats from the MPC](../../user/src/making-music/moving-from-the-mpc.md). Current plan: [Loading MPC projects into MPFree](https://claude.ai/artifact/EAnubyX8htJKtgHUVJHpvc) (2026-11-17). The earlier assessment and plan docs were on a previous account and no longer open.

## Scope (decided 2026-11-17)

A whole MPC project can't come across faithfully: Akai's plugin instruments live in Akai's code, effect settings are an undecoded blob, the sound engines differ, and the format is undocumented and changes between firmware versions. So the importer covers what converts reliably and leaves the rest to Akai's own exports:

- **Kept:** keygroup and drum programs (built), expansion and factory programs read from a card, and standalone `.xpm` files. Patterns come in as MIDI files saved with the MPC's *Track as MIDI* or *All Tracks* export, through MPFree's MIDI file import (planned: MIDI powerhouse phase 2, PR 6). Everything else comes in as audio: *Drum Pads as Stems*, *Audio Mixdown* with separate tracks, or bounced tracks.
- **Instruments:** bounce Akai plugin tracks to audio on the MPC, or autosample them into a keygroup.
- **Dropped:** reading `.xpj` sequences, the mixer, songs and Arrange, effect settings and automation (stages 2 to 4 below), and OS 2.x projects.
- **The manual page:** [Moving your beats from the MPC](../../user/src/making-music/moving-from-the-mpc.md).

The importer only ever reads MPC files and writes a new MPFree project; it never writes to the MPC. The sections below keep the format notes and the earlier staged plan for reference.

## What the files are

Community reverse-engineering, not Akai documentation; confirmed for OS 3.7 files, **not yet for 3.6**:

- **OS 3.x:** `NAME.xpj` is gzip; unpacked, five text lines (`ACVS`, firmware version, object type, `json`, platform) then one JSON document holding tracks, programs, sequences, songs and the mixer. Samples are WAV files in `NAME_[ProjectData]/`.
- **OS 2.x:** `NAME.xpj` is an XML manifest, with `.xpm` programs, an `.xal` sequence list and `.sxq` sequences (MIDI files whose notes are Akai meta events) in the ProjectData folder.
- Time is 960 PPQ, the same as `mpfree_transport::PPQ`.
- Effect and plugin-synth settings are an undecoded binary blob. Their names, ids and bypass state are readable; their knob values are not.

## Confidence

High: samples, tempo and meter, notes. Medium-high: drum programs, mixer levels and routing. Medium: keygroups, probability, per-note tuning. Low: automation, audio tracks, songs and Arrange, effect settings, plugin synths, Q-Links. Details per area are in the assessment.

## Stages (earlier plan; stages 2 to 4 dropped 2026-11-17)

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

## Stage 1: drum programs

`mpfree-mpc-import drum PROGRAM.xpm --data DIR [--place notes|pads] [--name NAME] [--release-ms MS] [--dry-run]` reads a drum program (`<Program type="Drum">`) and the samples beside it and writes a new project holding one kit (`program` instead of `drum` or `keygroup` picks from the file's type). Code: `xpm::read_drum`, `drum::drum_kit`, `import::import_drum`; tests in `tests/drum.rs`, on made-up programs only.

- **What it reads** (from community notes for OS 2.15 and the MPC's file layout; **inferred, not yet checked on a real drum program**):
  - Each `<Instrument number>` is a pad. Numbers count from 1 (A01 is 1); a program with an instrument 0 is read as counting from 0, and the report says which.
  - `PadNoteMap` (`<PadNote number="pad"><Note>`) gives each pad's note. Without one, bank A takes the MPC One's pad notes (`PAD_NOTES`: 37, 36, 42, 82, 40, 38, 46, 44, 48, 47, 45, 43, 49, 55, 51, 53, what the pads send) and other banks have none.
  - Per pad: `Volume`, `Pan`, `TuneCoarse`, `TuneFine`, `MuteGroup`, `MuteTarget1`-`4` and `SimultTarget1`-`4` (notes), `Polyphony`, `Mono`, `OneShot` (else `TriggerMode`, 0 read as one shot), `ZonePlay` (0 read as cycle, 2 random, others by velocity), `VelocitySensitivity`, the envelope (`Volume*`) and the filter.
  - Per layer, as for keygroups, plus `Direction` 1 (reverse); start and end fall back to `SliceStart` and `SliceEnd` (the last frame played) when `SampleStart` and `SampleEnd` aren't written.
- **Where the pads go.** `--place notes` (the default) puts each sound on the kit pad that answers its MPC note through `mpfree_controller::mpc_one::PAD_NOTES` (pads 1 to 16; the table MIDI out and MIDI file import read), so MIDI files exported from the MPC play the right sounds: a default-map program keeps its bank A order and a custom `PadNoteMap` is reordered. A note outside those 16, a pad without a note or a second pad on the same note goes on the first free pad from 17 up and the report marks it (MIDI file import leaves such notes out, so they match). `--place pads` keeps the MPC's order (A01 on pad 1). Banks B to H have no notes yet; when kits grow to 128 pads they belong next to `PAD_NOTES`, from a real program's `PadNoteMap`, shared by export, import and this tool.
- **The kit.** Layers keep their velocities, start, end, loop, level, pan, tuning and reverse. Mute targets become chokes and simultaneous play becomes links, both through the notes, so they follow the pads wherever they go. Mute groups keep their number when it is 1 to 16; others get a free number (the report says which), and past 16 groups they are left out. Levels are relative to the most common pad volume and the most common layer volume. A pad that plays while held (`OneShot` False) plays its layers in Gate and releases over `--release-ms`; others play one shot. The project's pads play as a drum kit (`PadsPlay::Pads`).
- **Approximated, and said.** As for keygroups: missing or empty samples leave their velocities to the layers next to them, more than 16 layers keep the first 16, and the envelope, filters and the program's effects are reported, not carried over.

To check against a real program: import one of the factory drum programs with `--dry-run` and compare the report's pads, notes, mute groups and targets with the MPC's own Program Edit page, then play the kit beside the MPC.

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
