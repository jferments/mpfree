# Factory content and plugin compatibility: policy and research plan

## Keep these categories separate

| Category | Examples | Likely technical approach | Distribution stance |
|---|---|---|---|
| User-created audio and MIDI | Recorded WAVs, user presets, MIDI files | Import directly, with format checks | User controls their own material |
| Factory samples / expansion audio | Drum hits, loops, multisamples | Investigate formats and local-only import | Do not redistribute without permission |
| Factory presets | Synth patches, drum programs, effect settings | Document formats and build converters where lawful | Do not redistribute proprietary presets without permission |
| Factory instrument/effect DSP | Akai/AIR native plugin binaries and algorithms | Check ABI, host dependencies, license, and authorization; otherwise independent substitutes | No proprietary binary distribution |
| Factory OS components | Applications, firmware images, libraries | Study interfaces where lawful; write original replacements | No copying proprietary OS components into mpfree |

## Compatibility objectives

1. Open formats first: WAV/AIFF/FLAC (as feasible), MIDI, SFZ, and documented user-owned formats.
2. Add an **optional local importer** for content already lawfully accessible to the device owner, only after reviewing applicable terms and formats.
3. Keep any import strictly local to the user's device or computer; never upload or bundle factory content in the open-source repository.
4. Build independent effects and instruments using appropriately licensed DSP libraries and plugins. Record each dependency's license and attribution.
5. Investigate native plugin hosting only if the interface, activation, and licensing allow it. An ARM binary is not automatically a reusable Linux plugin.
6. If a factory effect cannot be hosted, document its behavior and implement an independently developed functional alternative, without copying protected code or assets.

## Important distinctions

A factory sample file is not the same as a plugin executable. Extracting an executable does not make it compatible with another host. Plugin state, UI, DSP processing, licensing, ABI, architecture, and runtime dependencies may be tightly integrated with the original MPC application.

The existence of a downloadable installer or plugin archive does not establish a right to redistribute it or bypass activation. Terms may vary by product, jurisdiction, and content. Obtain specific legal review before publishing any extraction or redistribution functionality.

## Initial research tasks

- Inventory file formats, locations, ownership, and licenses from a legally obtained device image or installed content.
- Identify which assets are user-created, freely licensed, or proprietary.
- Prototype import with user-provided test WAV/MIDI/SFZ files, **not** with copyrighted factory material in the repository.
- Compare plugin-hosting options and the feasibility of independent replacements.
- Maintain a provenance manifest for all bundled assets and third-party code.

## Repo policy

Never commit proprietary firmware, factory samples, instrument/effect binaries, activation data, credentials, or license files. Report uncertain rights as unknown rather than assuming permission.
