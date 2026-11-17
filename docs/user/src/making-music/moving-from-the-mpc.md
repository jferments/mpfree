<!--
status: changing
covers: rust/mpfree-mpc-import
checked: 2026-11-17 acfc234
-->

# Moving your beats from the MPC

MPFree doesn't open MPC projects (`.xpj` files). What it brings across are your **sounds** and your **patterns**: drum and keygroup programs with their samples, your notes as MIDI files, and anything else as audio. This page says what comes across, what doesn't, and how to move each part.

## What comes across, and what doesn't

| From the MPC | In MPFree |
|---|---|
| Drum programs: samples, velocity layers, start and end, loops, level, pan, tuning, mute groups, mute targets, simultaneous play, mono and polyphony | A kit, read closely ([below](#bringing-a-kit-across)) |
| Keygroup programs: samples, key ranges, root notes, velocity layers, tuning, levels | A kit whose pads play notes |
| Notes, from **Export Track as MIDI** or **All Tracks** | Kept as MIDI files: MPFree can't load MIDI files yet (it's planned) |
| Anything else, from **Export Drum Pads as Stems** or **Audio Mixdown** with separate tracks | WAV files you load onto pads or audio tracks |
| Envelopes and filters on pads | Not carried over: listed in the report |
| Akai's effects (reverbs, delays, compressors...) | Not carried over: listed in the report |
| Akai's plugin instruments (TubeSynth, Bassline, Hype, Mellotron, Odyssey, Solina, Electric, Drumsynth, and the paid ones) | Their sound doesn't come across: see [below](#akais-instruments) |
| Sequences, songs, Arrange, the mixer, automation, Q-Link assignments | Not carried over |

A kit sounds close to the MPC, not identical: MPFree's pads have their own envelopes, filters and pitching. Timing is exact, because MIDI files keep every note where you played it.

## Before you leave the MPC

On the MPC, with its own software:

1. **Save each kit as a program** with its samples to an SD card or USB drive. The MPC writes the program file (`.xpm`) and its WAV files in one folder; keep them together.
2. **Export your patterns as MIDI** (**Export Track as MIDI**, or **All Tracks**) for the sequences you want to keep.
3. **Bounce what can't come across to audio.** A track played by one of Akai's instruments, or one whose sound depends on Akai's effects, comes across best as a recording: **Export Drum Pads as Stems**, or an **Audio Mixdown** with separate tracks.

Your MPC projects stay exactly as they are: MPFree never writes to the MPC's internal drive or its SD card.

## Bringing a kit across

For now, programs come across with a tool you run on a Linux computer, built from MPFree's source: the **MPC import tool**. The MPFree stick's data partition (`MPFREE-DATA`) must be plugged in and mounted. Point the tool at the program file and at that partition:

```sh
mpfree-mpc-import program "MyKit/MyKit.xpm" --data /path/to/MPFREE-DATA --dry-run
```

Change `/path/to/MPFREE-DATA` to the folder where your computer shows that partition. `--dry-run` writes nothing and prints the report; run it again without `--dry-run` to make the project. It goes in **Projects** under the program's name (`--name` gives another), and the tool never replaces a project that is already there. Open it with **BROWSE**, as any project ([Saving your work](saving.md)): the kit comes onto the pads and everything else stays as it was.

**Where a drum program's pads go.** By default each sound goes on the pad that plays the same note in MPFree, so the MIDI files you exported play the right sounds. Pad 1 plays note 36 (C2), pad 2 note 37, and so on, so the MPC's kick on A01 (note 37) lands on pad 2. A sound whose note is below 36, or that shares its note with another pad, goes on a free pad, and the report marks it with `*`: MIDI from the MPC won't play it there. To keep the MPC's pad order instead (A01 on pad 1), add `--place pads`; the pads then look as they did on the MPC, but exported MIDI plays other sounds.

**The report** lists every pad: the MPC pad it came from, its note, its layers and its sound. It also says what was approximated or left out: a missing sample (the layer next to it takes over its velocities, so no hit falls silent), mute groups renumbered to fit MPFree's 16, envelopes, filters and the program's effects. A pad that plays while held on the MPC releases over 400 ms here (`--release-ms` changes it), because the times behind the MPC's envelope settings aren't known.

## Akai's instruments

A track played by one of Akai's plugin instruments has its sound in Akai's own software, which MPFree doesn't copy or run. To keep it:

- **Bounce it to audio on the MPC** before you move, and load the recording in MPFree.
- **Sample it into a keygroup program** with the MPC's own sampling, save that program, and bring it across like any other kit.
- **Or keep the factory software for it.** Take the MPFree stick out and the MPC starts as it always did.

> [!MPC]
> MPFree doesn't open MPC projects. Bring kits across as programs (drum and keygroup `.xpm` files with their samples), notes as MIDI files, and everything else as audio. Akai's plugin instruments and effects don't come across; bounce those tracks to audio first. See [Moving your beats from the MPC](moving-from-the-mpc.md).
