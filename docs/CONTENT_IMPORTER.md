# mpfree local content importer — initial prototype

**Target device:** original Akai MPC One (not One+), factory firmware **3.6.0**. Hardware execution on that version has **not** been tested.

The Python 3 utility in this repository is a **desktop/host-side proof of concept** for migrating accessible audio assets into a separate mpfree library. It can also run on an authorized device if a compatible Python interpreter is present, but we have not established that Python is installed in MPC 3.6.0.

This tool does **not** extract files from firmware images, read encrypted partitions, obtain root access, bypass DRM/activation, modify system files, or install mpfree. Do not run unknown firmware hacks on a device containing projects you have not backed up.

## What it currently supports

- Read-only recursive discovery of accessible WAV, AIFF, FLAC, and Ogg-family files.
- Signature checking to reject misleading file extensions.
- Skipping symbolic links to avoid accidental traversal outside a selected source.
- Multiple source directories with separate source-number namespaces.
- A JSON manifest of discovered assets (metadata only).
- SHA-256 hashing on request.
- A default **dry-run** import plan.
- Opt-in copy to a separate destination with no overwrite of existing files.
- Detecting identical previously imported files; conflicts are reported, not overwritten.
- Rejecting overlapping source/destination directories and destination symlink parents.

**Important:** File recognition and copying do not imply that the future mpfree audio engine can decode or play every listed format. This is a migration utility, not an audio decoder.

## Usage

On a computer with Python 3.10+ and an accessible directory containing files you are authorized to use:

    python -m mpfree_tools.content scan --source "/media/backup/MPC Content"

Generate a new JSON manifest outside the source directory:

    python -m mpfree_tools.content scan --source "/media/backup/MPC Content" --hash --manifest "./content-inventory.json"

Preview an import without writing files:

    python -m mpfree_tools.content import --source "/media/backup/MPC Content" --destination "./mpfree-library"

Copy supported audio assets after reviewing the plan:

    python -m mpfree_tools.content import --source "/media/backup/MPC Content" --destination "./mpfree-library" --apply

The destination is organized under source-01/, source-02/, etc., preserving the source-relative folder layout. Existing files are not overwritten. Import is **not transactional**: interrupted copies may leave partial destination files, which a later run reports as conflicts.

To run tests (no factory content required):

    python -m unittest discover -s tests -v

## Next steps: factory MPC One 3.6.0

1. Obtain a **read-only inventory** of user-accessible content directories and file formats on the specific device; do not assume MPC 2.x paths remain correct.
2. Determine how to access those files without modifying factory firmware or disabling security.
3. Add format readers for supported MPC program/preset metadata, starting with synthetic test fixtures.
4. Research proprietary plugin packaging, architecture, ABI, and licensing **separately** from sample copying.
5. Prototype local-only, opt-in migration into a versioned mpfree asset database.
6. Eventually bundle this tool in an installer only after the installer has a verified non-destructive recovery path.

## Rights and privacy

Distribution of a generic file-copy or import utility can be lawful, but there is **no universal rule** that every extraction method is legal. Firmware protections, licensing terms, anti-circumvention rules, and local law may matter. Do not redistribute proprietary factory content or plugins in this repository. An owner-run import must be limited to content they are entitled to use.

Manifests may contain local source paths and asset names. Review before sharing publicly. No telemetry or network communication is performed by this tool.

## Known limitations

- No firmware 3.6.0 content path mapping yet.
- No native MPC hardware integration yet.
- No plugin executable or preset import yet.
- No extraction from encrypted/packed proprietary containers.
- No transactional/resumable copy; no deduplication across source directories.
- No audio transcoding, resampling, or playback.
- Python is not the planned real-time engine; this is an offline installer-side utility.
