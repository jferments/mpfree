# Importing MPC projects

**Status:** feasibility done; Stage 0 tools built, waiting on the owner's own OS 3.6 projects (2026-11-12). Full assessment: [Importing MPC projects: feasibility](https://claude.ai/artifact/1CzE52mppbCaNWMYsVLVm8).

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

0. **Corpus and inspector:** the owner's own projects, saved on OS 3.6, read by an inspector that dumps their structure and flags fields that differ from the public 3.7 notes.
1. **Kits:** samples, drum and keygroup programs, and standalone `.xpm` files.
2. **Sequences:** notes, tempo, meter, loops; checked against the MPC's own Audio Mixdown.
3. **Mixer:** levels, pans, sends, submix and return routing; effects by name, at default settings.
4. **The rest:** songs and Arrange, audio tracks, automation, per-note modifiers.
5. **In the app:** import from the Browser page, reading a USB stick.

Every import writes a report of what came across exactly, what was approximated and what was dropped. The importer writes an MPFree project folder as the browser saves one ([The browser and projects](browser.md): `project.txt` plus one `parts/<owner>.txt` per engine through `mpfree_library::Part`, and `samples/`), so each stage writes the parts of the engines it fills.

## Inspecting projects

`tools/mpc-import/inspect-xpj.py PATH` reads a `.xpj`, a folder of them, or the `mpc-projects.zip` below. For each project it prints the format and header lines, a summary (tempo, tracks and programs, pads and layers, inserts by name, sequences with their clips, notes and automation ids, songs, and samples missing from ProjectData), and an importer checklist: each field stages 1 to 3 will read, flagged `MISSING` or `TYPE` when it differs from the public 3.7 notes. `--shape` adds every field path with its types and counts. It never prints a settings blob (only its length) or audio, so its report is safe to share in the thread.

## Getting test projects out of the eMMC backup

`tools/mpc-one/offline-mpc-projects.py` runs on the workstation only. It reads the newest `~/mpfree-backups/mpc-one-emmc-*/mmcblk0.img` with debugfs (read-only, nothing mounted, no sudo), finds the `data` partition (p8, the MPC's Internal drive), and lists every `.xpj` project and `.xpa` archive with its format line. With `--copy 1,3` or `--copy all` it copies only those projects' `.xpj` and `_[ProjectData]` folders into `~/mpfree-work/mpc-projects-<UTC>-*/`, checks each file's size, and writes `SHA256SUMS` and `mpc-projects.zip`. Nothing else on the partition is copied.

Projects saved to an SD card or USB drive, or after the backup was made, are not in it.

**Never commit MPC projects, their samples or blobs.** They may hold Akai factory or expansion sounds ([factory content policy](../project/factory-content-policy.md)). Test files stay local and outside the clone.

## Sources

- [kurtjcu/MPC-project-file-definitions](https://github.com/kurtjcu/MPC-project-file-definitions) (no license stated: we learn from it and write our own code)
- [andyg5000-agent/mpc2reaper](https://github.com/andyg5000-agent/mpc2reaper) (MIT)
- [jeffwoods/mpc-mcp](https://github.com/jeffwoods/mpc-mcp)
