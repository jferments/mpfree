# Investigating factory instruments, presets, and plugin compatibility

**Hardware/firmware under investigation:** original Akai MPC One, MPC firmware 3.6.0.

The native Akai/AIR instruments and effects may rely on private host interfaces, shared libraries, licensing infrastructure, or architecture-specific code. Extracting a binary does **not** make it usable in a different host.

We therefore have two parallel tracks:

1. **Compatibility research:** identify which proprietary files exist and whether the owner can lawfully use them through an independent host.
2. **Open implementations:** build a plugin system and independent effects/instruments, preserving the ability to substitute licensed factory plugins if hosting becomes feasible.

## First tool: metadata-only compatibility survey

The tool records candidate file paths, file sizes, extension-based categories, file signatures, and ELF architecture for recognized native libraries. It does **not** read full proprietary binaries unless optional hashing is requested, execute plugins, copy files, bypass activation, decrypt content, or modify the source.

Example, on a computer with an accessible backup or content directory:

    python -m mpfree_tools.compat_survey --source "/media/backup/MPC Content"

Write a JSON inventory outside the source directory:

    python -m mpfree_tools.compat_survey --source "/media/backup/MPC Content" --manifest "./plugin-inventory.json"

Optional SHA-256 for exact binary identification:

    python -m mpfree_tools.compat_survey --source "/media/backup/MPC Content" --hash

Recognized *candidate* extensions include .so (including versioned .so files), .dll, .xpm, .xpn, .xpk, .xpr, .fxp, .fxb, .preset, .aupreset, .sfz, and .sf2. These are **hypotheses**, not proof that any particular file is a working MPC plugin or valid preset. ELF header parsing reports whether a candidate native library identifies as 32-bit ARM, AArch64, x86, or x86_64.

The tool is intended to help answer questions such as:

- Are the relevant instrument engines native ELF libraries, packed resources, or code inside the proprietary host?
- Which CPU architecture do candidate libraries target?
- Are preset and expansion resources separate from executable code?
- Which resources appear to be standard archives or readable text formats?

## What we cannot conclude from this survey

- Whether the original plugin ABI is compatible with LV2, CLAP, VST, or any other host.
- Whether binaries will load without proprietary Akai runtime services.
- Whether activation and usage rights permit independent hosting.
- Whether content files are encoded or encrypted.
- Whether a reported library is actually a synthesizer or effect.

## Next research steps

1. Establish read-only access to relevant firmware 3.6.0 file trees using a safe and authorized workflow.
2. Identify candidate resource locations and record their file format signatures and dependency names without copying files into the public repository.
3. Examine licenses and activation requirements for specific factory plugins.
4. Develop a stable mpfree instrument/effect interface, initially backed by open DSP implementations.
5. Where technically and legally viable, prototype an **optional, locally installed** compatibility host using owner-supplied files.
6. Keep original proprietary samples, presets, binaries, and activation materials out of Git.

The survey is **not yet validated on actual MPC One firmware 3.6.0**. Run it on a host-side copy or otherwise safely accessible directory first. Back up the device before attempting any changes to its operating environment.
