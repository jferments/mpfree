<!--
covers: rust/mpfree-stream rust/mpfree-audiotracks/src/bin/disktest.rs rust/mpfree-audiotracks/src/bin/disktest/read.rs rust/mpfree-audiotracks/src/host.rs rust/mpfree-audiotracks/src/disk.rs tools/mpc-one/host_disktest_mpc.py tools/mpc-one/read-only-stream-survey.sh
checked: 2026-11-22 829e1568
-->

# Disk streaming from the USB stick and the SD card

**Status:** plan approved by the owner on 2026-11-22 (phases P0 to P1.5 on the workstation; every step on the MPC waits for its own go). P0's measuring tools are built: the streaming survey (D0) and the read-only stick test (D1), neither run on the MPC yet. P0.5's fault fixes in the old engine are built, with the fault tests (below). P1's first part is built: the new crate `mpfree-stream` with file formats, the sound thread's cache, cursors, sounds held in memory and the coordinator, and its unload-while-playing tests ([below](#p1-the-stream-layer)). So is most of P1.5: joined reads, cancelling, holds in priority order, whole-file reads behind playback, launch readiness and admission to the budget ([below](#p15-planning-reads-and-admission)); recording pressure waits for recording. Nothing streams in the app yet: the audio tracks' engine is still the 12 Nov one, switched off.

**Owner:** the disk streaming thread. What plays and when belongs to the audio tracks ([audio-tracks.md](audio-tracks.md)); this layer makes sure it is in RAM in time. The SD card's switching on and its safety testing (steps S0 and S1) belong to the [SD card plan](../hardware/sd-card-and-internal-drive.md) and the owner's hardware project.

## Goal

Play long audio from disk on a stage without a dropout: backing stems, audio track takes, audio clips, a set list's next song and, later, long samples. **The target:** 16 stereo stems at 24-bit for a 60-minute set while 2 tracks record, with no counted miss and the rendered sound matching a reference, on the owner's stick, inside a RAM budget the musician picks (64, 128 or 256 MB, 128 MB by default).

The order of what matters: recordings are never lost silently, then steady timing, then speed.

## Today, and what a review found

`mpfree-audiotracks` (12 Nov) already streams takes from partition 4 through a RAM cache of 128 KiB float chunks, switched off and never run on the MPC. A review on 22 Nov found seven problems, all confirmed on main at `829e1568`:

1. **The sound thread does disk bookkeeping.** `AudioTrackHost::period()` calls `tend()` ([host.rs](../../../rust/mpfree-audiotracks/src/host.rs)), whose `tend_disk()` builds file names with `format!` and whose `tend_save()` builds the whole save (`save_file()`) every period while settled.
2. **A short read becomes silence counted as success.** `fill_part` ([lib.rs](../../../rust/mpfree-audiotracks/src/lib.rs)) zero-pads whatever didn't arrive and marks the chunk clean.
3. **A failed write is only counted.** `write_failed` adds to `write_errors`; the take carries on as if saved.
4. **One serial disk thread.** `serve()` ([disk.rs](../../../rust/mpfree-audiotracks/src/disk.rs)) handles one job at a time, so reading a whole file or building peaks blocks the playback reads behind it.
5. **Replies are matched by take and chunk.** Take numbers are reused after a project change, so an old reply can land in a new take.
6. **Importing refuses while streaming** (`ImportRefused::Streaming`).
7. **The WAV header and the sync come only at the end.** A cut mid-take leaves a file with no usable header. `finish()` also computes the data size as a `u32`, which overflows after about 3 h 23 min of stereo float.

The cheap fixes go into the old engine with failing tests first (P0.5); the deep ones are designed out of the new layer (P1). After P0.5, findings 2, 3 and 4 and the `u32` length of 7 are fixed in the old engine; 1, 5, 6 and the header and sync of 7 each have an ignored test that fails until P1 ([below](#p05-the-old-engines-fault-fixes)).

## The design

One shared stream layer, a new crate `mpfree-stream` grown out of `mpfree-audiotracks`, plays every long sound from disk. It is foundational, like the mixer, not a feature of audio tracks.

### The sound thread's side

- Each player reads RAM through its own cursor and writes recordings into capture memory. It talks to the rest through fixed-size single-producer, single-consumer queues made at start: it posts where each source is playing and which launches are cued, and takes ready chunks off a queue, copying them into its cache within a fixed budget per period (about 16 KB a period at 16 stems). It handles at most a set number of messages a period. File names, opening files, errors and save data all happen on the coordinator thread.
- **Who owns a chunk.** In P1 the sound thread owns its cache outright and the coordinator owns only buffers in flight, passed by message: no shared memory, safe Rust. Handing cache slots over without a copy comes only if the worst-period tests show the copy matters, and then under written rules: a slot goes Free, Filling, Ready, Published, Retired, Free; a published slot never changes; it is fully written before it is published (release and acquire ordering); a retired slot is reused only after the sound thread has passed a period boundary, because cursors look chunks up afresh each period. No reference counting on the sound thread.
- P1 must show that a source can be unloaded, replaced or invalidated mid-play with no use-after-free, stale or half-written audio, allocation or blocking on the sound thread: stress tests, loom checks of the queue protocol, sanitizer runs on the workstation.

### Sources and requests

- Everything that plays is an **audio source**: an id for this run, a generation, the project's lasting id for it, its format, sample rate, channels, length and where it is stored. Tracks, clips, the sampler and the set list point at sources; none owns a file. Several cursors can read one source at once, sharing its cached chunks. Looping, sample-accurate seeks, start offsets, speed and cancelling are tested abilities of every cursor.
- **Every request says who asked:** the project's generation, the source, the source's generation, a request number, and the frames it expects. A reply for an older generation is dropped. A short read inside the sound is a read error; zeros go only past the sound's real end.

### Memory, heads and the coordinator

- **Memory or disk is decided by budget.** A sound loads into RAM when it fits next to what is playing; otherwise it streams. Kit pads and short drum sounds load whole when they fit; "Keep in memory" pins any sound. No switch, no restart, no fixed length rule.
- **Heads depend on the kind of sound** (table below). When room runs short: playing now and capture memory first, then cued launches, heads of the playing song, then the next song inside its own budget. The layer answers "next song ready", naming any file whose head is missing.
- **The coordinator plans by deadline.** It works out which frames every playing source needs and by when, wrapping round loops and scaling by speed. Playback reads go earliest deadline first. A deadline belongs to the whole job: for a FLAC file or one at another rate the read, the decoding and the conversion each get a share of the time. Decoding has its own bounded queue, apart from background work (peaks, copies).
- **Recording pressure.** A recording's deadline is the moment its capture memory would fill. Below a quarter full, playback reads come first; past half, recording writes move up; past three quarters, background work stops; past nine tenths, the screen warns that recording may stop. These are starting points for the device tests. If the stick stalls longer than any buffer covers, the engine says so.
- **The window comes from measurement, and admission decides what fits.** A stream wants 2 x the stick's measured worst shortfall + 1 s of read-ahead, starting at 8 s and never under 4 s. Each armed recording reserves its data rate times the longest stretch the stick couldn't take writes. The coordinator adds capture reserves, playing audio, heads, cued launches and the wanted read-ahead; if that fits the budget the workload is admitted, otherwise MPFree trims prefetching it doesn't need and says plainly that the workload isn't covered. It never shrinks the margin quietly while calling a set ready. (A stick with a 20 s stall wants 41 s ahead, about 231 MB for 16 float stems: it doesn't qualify for 16 stems in 128 MB.)
- **Small units, joined into bigger reads.** The cache works in about 64 KiB of file; the I/O thread joins neighbouring requests into reads of up to 512 KiB when no other deadline would be missed meanwhile. Files are read in their own format (16 or 24-bit, float); the cache holds float.
- **I/O threads only do I/O,** one per device (the stick, later the SD card, reads only). Decoding, rate conversion, peaks, imports and copies run on worker threads in small steps that yield. The I/O threads start at normal priority on cores 0 to 2; a different priority or core is kept only if D3 shows a better worst period and worst read.

| Kind of sound | What stays in RAM |
|---|---|
| Short drum and kit sounds | The whole sound, when it fits the budget |
| Long one-shots | A short start (tens to hundreds of ms, from the measured stall); the rest streams |
| Keygroup samples | Small starts, inside the instrument's own budget |
| Backing stems | A large read-ahead window, to ride out a stall of several seconds |
| Loops | The start and the next loop point |
| Cued clip launches | Fetched by the launch time |
| The next song | Its own capped budget |

| At 44.1 kHz, stereo | 16-bit | 24-bit | 32-bit float |
|---|---|---|---|
| One stem, from disk | 176 kB/s | 265 kB/s | 353 kB/s |
| 16 stems, from disk | 2.8 MB/s | 4.2 MB/s | 5.6 MB/s |
| A 4-minute song of 16 stems, all in RAM | 677 MB | 1.0 GB | 1.35 GB |

### Recording

- **Recordings survive a cut.** A take is written as `NAME.wav.recording` with its WAV header written at the start. The header carries a reserved `JUNK` chunk that can become RF64's `ds64` (EBU Tech 3306), so a take past 4 GB never moves its audio. Beside it, `NAME.wav.checkpoint` holds two alternating fixed-size records: format version, the recording's id, a sequence number, frames and bytes committed, the audio format, and checksums of the record and of the last committed block. Every few seconds: the new audio is written and synced, then the older record is overwritten with the next sequence number and synced. The folder is synced once both files exist and again after the rename to `NAME.wav` at stop. At the next start MPFree picks the newest valid record whose audio checks out, rebuilds the header from it, and offers to recover the take to there.
- **The exact protocol gets its own short spec page, reviewed before any of it is built** (P1), including what happens after every step that can be cut off.
- Takes record as 32-bit float by default (a take from inside the mix keeps anything over 0 dBFS); 24-bit line takes are an option the device tests can switch on.
- **A failed write has a plan.** Each recording shows one state on its lane: Recording, Saving, Saved, At risk or Stopped. Saved means MPFree finished every sync it needs; whether a stick really keeps synced data through a power cut is a separate fact about that device, shown in its profile once it passes a cut test (D6). Passing errors are retried a few times; a full, pulled or failing stick each get their own message. Unsaved audio stays in capture memory, which playback can never evict.
- **Linux can't pile up a big flush.** MPFree may cap how much unwritten data the stick holds (`strict_limit` and `max_bytes` on its `bdi`, about 32 MB), in the running system only, and only after D4 shows it helps.

### Playing

- **Launches wait together.** PLAY or a locate to a place not in RAM waits up to about a quarter of a second, in the transport, never in the sound thread. If any stem of a launch isn't ready by its boundary, the whole launch moves to the next boundary and the screen says so; it never starts 14 of 16 stems. A launch holds its stems' starts until the new clip has started and the old one faded; for a group every hold succeeds or none does, inside the budget, and holds are released on request.
- **One rule for speed.** Native (read as stored, decoded ahead on a worker), Prepared (a 44.1 kHz copy made ahead for a live set) and RAM. In every mode a cursor moves at (file rate / output rate) x the musical speed: a 48 kHz file at normal speed moves 1.088 file frames per output frame and keeps its length and pitch. Streamed sources use a band-limited read for any rate change (48 to 44.1 kHz is 1.47 semitones, under the sampler's two-semitone line in `mpfree-sampler`'s `bandlimit`). A Prepared copy is offered when a set is checked, never forced; originals never change.
- **Performance snapshots.** Preparing a set records which version of each file it plays (the staged copy, its size and hash); during the show those files are fixed.
- **Linux's file cache** is RAM the kernel can take back, on a 2 GB machine with no swap. Reads stay ordinary buffered reads; D3 measures the cache under memory pressure.
- **Every dropout is shown,** tied to its source: "The stick fell behind: 0.3 s of Drums stem went silent at bar 34." Audio track lanes show their counters and Dev › Disk the numbers. A disk chip in the top bar would change the [layout spec](layout-spec.md), so it is a separate proposal.

### Storage places and files

- A small **storage manager** knows three places: the stick's data partition (on at every start), the SD card (off until asked, read-only first) and the internal drive (off, read-only on a separate explicit ask, never needed for streaming). Opening the SD card browser can never switch on the internal drive.
- **Measured profiles:** each stick and card that passes a device test keeps its result under its own identity ("16 stems and 2 recordings passed, 22 Nov"): what was measured, not that the device is healthy now.
- **Three kinds of file.** Managed: made by MPFree in the project's folder; only these can be cleaned up. External: the SD library or files elsewhere; read and relinked, never deleted. Derived: fitted or converted copies and peaks, disposable, except copies prepared for a set. **Save order:** media written and synced, then the project file through a temp file, sync, rename and folder sync; only then is the save committed and cleanup allowed. A recording finishing is a separate step from a project save.

## The SD card

For streaming the card starts as a **read-only library**: browse it and copy stems onto the stick, which plays them. Partition 4 is ext4, which Windows and macOS can't write, so a computer writes the card and MPFree copies from it; a card pulled mid-song can't stop a stem that plays from the stick.

**Protection comes before the card switches on.** Akai's disk service, `edisksd`, still runs under MPFree's boot: it isn't among the units the stick switches off, and in the 14 Nov stick test it mounted the stick's partitions read-write ([firmware-3.10.1-stick-test.md](../hardware/firmware-3.10.1-stick-test.md)). The order:

1. Check that both the SD controller and the internal drive's controller are off.
2. Keep `edisksd` away from SD disks (switched off on the stick, or a rule it is proven to obey), and put a rule in `/run` that marks every SD disk read-only as it appears.
3. Switch on the SD controller alone.
4. Check the disk hangs off the SD controller (`ff0c0000`), not the internal drive's (`ff0f0000`), whatever its `mmcblk` number.
5. Check the disk and every partition are read-only; record the write and discard counters (`/sys/block/<card>/stat`).
6. Mount the chosen partition `ro,noatime,nosuid,nodev,noexec`, plus `noload` on an ext4 card so a journal is never replayed.
7. Check the mount options and the counters again, then let the library see the card.

Any failed check leaves the card unusable to MPFree and says why. The card has a life cycle (Off, Switching on, Checking, Ready, Fault, Removing); every insertion gets a new number its sources carry, and the card's and the volume's serials are checked so a swapped card is never taken for the first. A pulled card is a lost source: new reads are refused, reads in flight cancelled, its streams go silent and are counted, and nothing needs repairing because nothing was written. The counters are read again when the card is switched off; any change stops reading.

**A copy is a transaction:** read from the card and hashed on the way, written to `NAME.part` on the stick, synced, read back with `O_DIRECT` (dropping pages is only advice) and hashed again, renamed, the folder synced, and only then added to the library. It runs at the lowest priority and pauses when recording pressure rises. It proves the stick returns the right bytes through a fresh read, not that its cache reached flash; that is part of qualifying a device.

**Playing straight from the card comes later,** chosen per set: Performance-ready (the set's files staged on the stick and checked, the default for a gig) or Direct library (stream from a card that passed D5 and has a profile, an explicit opt-in for a live set). Musicians should use an A1 card rated U3 or V30.

### Two kinds of card

MPFree and Akai's system never run at once, so "shared" means taking turns.

| | A card just for MPFree | A card shared with Akai |
|---|---|---|
| What it is | Formatted for MPFree (the SD plan's layout C), on a workstation at first | The card as Akai or a computer made it, usually one exFAT partition (layout A) |
| Filesystem | ext4 with a journal (recommended), or exFAT if it must open on a computer | exFAT or FAT32 as it is, never reformatted |
| Reading and streaming | Yes, after gate G and D5 | Yes, read-only, after G and D5 |
| Recording, saving | Yes, once the card passes a write test and a cut test | No: record to the stick; shared-card writing is a separate design (the SD plan's S2) with its own approval |
| A cut while writing | The journal restores the filesystem's records (not the audio); MPFree's checkpoints restore a take to its last checkpoint; whether synced data survived depends on the card | No journal: a cut mid-write can damage records every file shares, Akai's files too |
| When Akai's system sees it | `edisksd` mounts it read-write and runs `tune2fs`: take it out before starting Akai's system, or MPFree treats it as possibly changed and checks it (`e2fsck -n`) next time | Used by Akai as normal; Akai may change or delete files MPFree knows about |

A card just for MPFree holds nothing bootable (whether the MPC's U-Boot looks at the SD card isn't known, [mpc-one-boot-architecture.md](../hardware/mpc-one-boot-architecture.md)), carries a label and a marker file with a random id, and is mounted read-write only after a clean `e2fsck -n` (with `errors=remount-ro`). A shared card stays read-only: its "in use" flag is reported, never cleared; a changed file drops its cached audio outside a prepared set. **Recommendation:** keep a shared card read-only; to record or save to a card, use one just for MPFree, and the strongest gig setup plays stems from one device and records to the other.

## Device tests

None writes to the MPC's internal storage; each block is rehearsed word for word with its expected output and sent for its own go. All run on the MPFree stick boot over SSH, with the internal drive and the SD card switched off (G and D5 aside), and pass only with the boot-area hashes unchanged.

| Test | Label | What it does | Writes |
|---|---|---|---|
| W, fault tests | workstation and CI | From P0.5, failing first, against both engines: short reads, early end of file, read and write errors, a full disk, failed syncs, a stall of seconds, a source removed mid-read, take numbers reused, a cancelled big read, a file layer that cuts a recording or import after every step; each run compares the rendered sound with a reference | test files on the workstation |
| WP, pull tests | by hand, a backed-up spare stick | Before D6: a stick pulled mid-recording and mid-import, then the recovery checked | the spare stick |
| D0, survey | read-only | [below](#d0-the-streaming-survey) | nothing |
| D1, stick reads | read-only | [below](#d1-the-read-only-stick-test) | nothing; the program runs from RAM |
| D2, stick writes | state-changing, stick only | the existing `mpfree-disktest` write run (audio-tracks.md) | at most 448 MiB of its own files on partition 4, deleted at the end |
| D3, streaming in the app | state-changing, stick only | After P2, the whole app running: 16 stems; 16 stems and 2 recordings; 24 stems and a recording; looping and seeking; fast scene changes; a big peaks job; sampler bursts; a nearly full stick; an hour without a break | test stems and takes in the app's folders, one filler file |
| D4, writeback cap | state-changing, running system only | sets `strict_limit` and `max_bytes` on the stick's `bdi`, repeats D2's mixed run | two values in the running kernel; undo: write back D0's values, or restart |
| D6, cut while recording | state-changing, its own safety review and go | after W and WP: power off at the wall mid-take, check every take recovers to its last reported checkpoint | a test take |
| G, SD safety gate | read-only on the card, inside the SD plan's S1 | shows the protection order above holds, across switching off and on and taking the card out | nothing |
| D5, card reads | read-only on the card | after G: D1's profile on files on the card, also with the stick streaming | nothing; counters unchanged |

**What a qualified workload must show:** the rendered sound matches the reference within a stated tolerance; no underrun; no capture frame lost or overwritten; every checkpoint reported committed comes back after a cut; no period past its deadline (the worst period and the slowest 0.1% reported); the cache and every buffer inside their limits; every error counted and tied to a source or device; no stale audio or invalid access when unloading mid-play; acceptable CPU with the sampler, mixer, screen and effects busy. Each run records its files, rates, stream count, recording format, cache size, filesystem state and device identity.

**The bench gate stays as a tripwire.** CI's 5% gate counts instructions per 128-frame block on the sound path, so moving work off the sound thread lowers it. The main performance measure is the worst period on the MPC, from D3.

## Roadmap

Each phase is one or a few draft PRs to the merge desk, docs in the same PR.

| Phase | Delivers | Done when |
|---|---|---|
| P0 measuring tools | the D0 survey, `mpfree-disktest --read-only`, this page | built (this page's last section) |
| P0.5 tests first | the fault suite W with a reference-sound check, failing first for the seven findings; in the old engine short reads become errors, failed writes get recording states, long jobs yield | W runs against the old engine and the new one (built: [below](#p05-the-old-engines-fault-fixes)) |
| P1 `mpfree-stream` | sources with generations, cursors, the fixed-size queues and fixed work per period, the coordinator, workers, the recoverable recording file (RF64-ready), 16 and 24-bit WAV and AIFF read in their own format, looping and speed; audio tracks move onto it unchanged once the harness passes | written chunk ownership rules and the unload-while-playing test; the recording spec page reviewed before the recording code; a small interface the sampler and launcher can use later |
| P1.5 scheduler | deadlines, recording pressure, joined reads, cancelling, admission to the budget, launch readiness | admission that knows when a workload can't fit |
| P2 audio tracks stream | streaming on; import in the background; the cache budget setting replacing Audio track memory; recording states and recovery; dropouts shown; unused take files cleared | recorded audio checked against what went in |
| P3 stems and clips | memory or disk by budget, heads by kind, launch groups, Native, Prepared and RAM, checking a set, the manual page | snapshots; the 48 kHz length, pitch and aliasing test |
| P4 SD library | the storage manager, the card's life cycle, checked copies, Direct library after D5 | the card's protection proven before any filesystem access |
| P5 tuning | I/O priority and cores, the stick's interrupt core, unit and read sizes, the window, the writeback cap | each kept only if a test shows a better worst case |
| P6 later | the next song from a set list, long samples and big keygroups from disk, FLAC streaming, device profiles on screen | FLAC decoding with its own deadlines and a bounded queue |

## P0: the measuring tools

Both run on the test build (MPFree's system on the stick, port 2222) through [`tools/mpc-one/host_disktest_mpc.py`](../../../tools/mpc-one/host_disktest_mpc.py), which talks only to the newest stick build's host key, so Akai's system is refused before anything runs. Neither has run on the MPC; each run there waits for the owner's go.

### D0: the streaming survey

`host_disktest_mpc.py --survey-only` (**read-only**, no opt-in, nothing copied) runs the test build's usual checks, `df -k /run/mpfree-data`, then [`read-only-stream-survey.sh`](../../../tools/mpc-one/read-only-stream-survey.sh): the kernel and its CPU settings, partition 4's mount and free space, the stick's block queue (scheduler, `nr_requests`, request sizes, read-ahead, write cache) and I/O counters, the USB path (speed, the storage driver, `usb-storage` or `uas`, the host controller), the writeback settings (`/proc/sys/vm/dirty_*` and the stick's `bdi`), memory, the USB interrupts with their CPUs and the threads that serve them (policy and priority), the pad player's real-time threads, the CPUs and governors, which file systems and disk modules the kernel has, what else from a disk is mounted, whether `edisksd` runs or is masked, and the kernel's messages about the stick. It reads procfs, sysfs, unit files and the kernel log only; it prints no serial number, MAC or UUID. The other two modes run the same survey in their step 2.

### D1: the read-only stick test

`mpfree-disktest --read-only` ([`src/bin/disktest/read.rs`](../../../rust/mpfree-audiotracks/src/bin/disktest/read.rs)) reads files already on partition 4 and writes nothing:

- **What it reads:** the audio files (WAV, AIFF, FLAC) under `library/` and every file under `backup/` (a creator stick's eMMC backup, one long file), 64 KiB or more, regular files only, never through a link, never off partition 4, at most 5,000 files. Longer files are read as 64 MiB pieces so one file can feed many streams. Only the pieces about to be read are dropped from the cache (`posix_fadvise64 DONTNEED` on their range), so every read reaches the stick.
- **Safety:** on the stick it refuses (exit code 3) unless `/run/mpfree-data` is ext4 labelled `MPFREE-DATA` on partition 4 of a USB disk, mounted `noatime` or read-only (so a read changes no access time), with no MMC device present. Files are opened read-only with `O_NOFOLLOW` and checked to be on partition 4. It prints counts and sizes, never a name, and checks at the end that every file it saw is as it was.
- **What it runs:** a sequential read of the largest pieces (up to 256 MiB or 15 s), then streams for 20 s each, each a thread reading its own pieces at one stereo stream's pace: 4, 8, 16 and 24 streams of 24-bit with 64, 128, 256 and 512 KiB reads, then 16 streams of 16-bit and of float with 128 KiB reads. About 7 minutes.
- **What it reports:** per configuration, p50, p99, p99.9, p99.99 and max per read, reads over 50/100/250/500 ms, the worst shortfall, the longest time a stream stayed behind, file ends and passes, the opens, and the window and RAM the plan's rule gives (2 x shortfall + 1 s, at least 4 s, a stream); then a summary table. p99.9 and p99.99 need about 1,000 and 10,000 reads to mean much; each line gives its count.
- **On the MPC:** `MPFREE_ACCEPT_DISKTEST=READ-ONLY python3 tools/mpc-one/host_disktest_mpc.py --read-only` (**RAM only on the MPC**: the program goes to `/run/mpfree-disktest`, cleared by a power cycle). The writing run (D2) still needs `MPFREE_ACCEPT_DISKTEST=YES` and no flag.
- **Build** (from `rust/`): `RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static" cargo build --release --target armv7-unknown-linux-gnueabihf -p mpfree-audiotracks --bin mpfree-disktest`.
- **On a desktop:** `mpfree-disktest --desktop-test DIR --read-only [--quick]` reads `DIR/library` and `DIR/backup`.
- **Tests:** `rust/mpfree-audiotracks/tests/disktest.rs` (a read-only run changes nothing, never follows a link out, names no file; too little to read is refused), the unit tests in `read.rs`, and `tools/mpc-one/tests/test_host_disktest_mpc.py` (the survey against a fake stick, USB path and pad player under BusyBox; the modes and their opt-ins; a static check that the survey only reads).

## P0.5: the old engine's fault fixes

The fault tests were written first and failed against the old engine; these changes make them pass. The audio tracks' page has the details ([audio-tracks.md](audio-tracks.md#streaming-from-the-stick)).

- **Short reads (finding 2).** A chunk that comes back shorter than the take's audio there is counted (`Stats::short_reads`), never played, and asked for again when next wanted. A failed read is too, as before.
- **Failed writes (finding 3).** A failed write is asked for again (`Io::Rewrite`, always into the take's own file, never a new one, even for chunk 0), and the file finished again once the take is recorded; a failed finish is tried again too. After `WRITE_TRIES` (3) tries, or when a request is lost to a full queue, the take is **at risk**: its unwritten chunks stay dirty in memory, so it still plays and nothing else takes their room.
- **Recording states.** `AudioTracks::save_state(id)`: Recording, Saving, Saved, AtRisk. A fit waits for Saved, because it reads the take's file. The audio tracks rethink will make a save stop with a reason when a take is at risk, rather than wait.
- **Long jobs (finding 4).** The disk thread reads a whole file (a fit, or the waveform table of a saved take with no `.peaks` file) 512 KiB at a time, answering up to 8 other requests between steps, so playback reads and recording writes never wait behind it.
- **The WAV length (finding 7, in part).** A finish whose length doesn't fit a WAV header (`WAV_MAX_DATA`, about 3 hours of stereo at 48 kHz) fails instead of writing a wrong length; the sizes are worked out in 64 bits, so a 32-bit MPC never wraps them.
- **Fault knobs.** `DiskStats` has `fail_writes`, `fail_finishes` and `short_reads` (the next so many of each fail) and counts the long jobs' steps.
- **Tests.** `tests/faults.rs` (a fake disk) and `tests/host_faults.rs` (the real disk thread, its knobs, and a take file cut short under it) check every sample played against what was recorded; the disk thread's unit tests cover the long jobs and the WAV limit. Ignored, each naming its finding, until P1 fixes it: the host's period allocates (1, in `tests/no_alloc.rs`), an old reply lands in a new project's take (5), a sound can't be brought in while streaming (6), a recording's file has no usable header until it ends (7).

## P1: the stream layer

`rust/mpfree-stream`, built a piece at a time. The first part plays files from disk through a cache the sound thread owns, and sounds held whole in memory through the same cursors; recording (after its spec page is reviewed) comes next, and decoding workers with the formats that need them (FLAC, rate conversion). The 12 Nov audio tracks engine is not moved onto the layer: the [audio tracks rethink](audio-tracks.md) builds its tracks and clips on it directly (agreed between the two threads, 22 Nov), so P1 is done when its harness passes against the layer.

- **`format`**: where a WAV, RF64 or AIFF file's audio starts, how long it is and how it is stored (8, 16, 24 or 32-bit, float or double, either byte order), so it is read in its own format and turned into float as it arrives. A file cut short is read to its end and says so; anything it can't stream (more than 2 channels, a rate outside 8 to 192 kHz, a compressed AIFF) is refused with its reason.
- **`msg`**: what goes between the sound thread and the coordinator. Every message says who asked (`Tag`: the project's generation, the source, the source's generation).
- **`player`**: the sound thread's side. `begin` takes at most a set number of messages a period and copies at most a set number of samples into the cache; `read` copies a source's frames out (silence where a unit isn't ready, counted as missed); `playing` and `end` post where each cursor plays once a planning round.
- **`cursor`**: one voice's place in a source: exact seeks, a speed up to 4 file frames an output frame, loops (a cursor before a loop plays into it; one past its end plays on), an end frame (a region that is part of a take: silent from there, not counted, nothing read past it), several cursors on one source. It gathers the frames it will pass round the loop, then reads them with 4-point Hermite interpolation: at speed 1 from a whole frame, the file's own samples bit for bit. The band-limited read for rate changes comes with stems (P3).
- **Sounds in memory.** `Coordinator::open_memory` hands a sound held whole in memory (a take just recorded, a short sound) to the sound thread, which reads it straight from there and gives it back when it is closed or replaced; it never touches the disk or the cache.
- **`coord`**: the coordinator, on its own thread. It opens files, gives each source a new generation, plans reads earliest deadline first over a window ahead of every cursor (round its loop once, scaled by its speed), asks an I/O thread that only reads, decodes, and sends fills. A cursor not heard of for two rounds has stopped. A read that fails or comes back short is counted and asked for again; nothing short is sent to be played.

### Who owns a chunk

These are the rules the code keeps, and the tests check:

1. **The sound thread owns its cache outright.** No other thread reads or writes it. There is no shared memory and no unsafe code (`#![forbid(unsafe_code)]`).
2. **The coordinator owns only the buffers in flight.** A buffer goes to the sound thread inside a fill and always comes back (`FromPlayer::Back`), whether the fill was used or refused. Neither side frees a buffer the other made. If the way back is full, the sound thread holds the buffer in a queue made at start, big enough for every buffer there is. A memory sound is shared (`Arc`), but the sound thread never lets go of its share: it sends it back (`FromPlayer::Resident`) on a close, a replace, a new project or a refusal, so the last share is always dropped off the sound thread.
3. **The coordinator chooses every slot, and keeps a mirror of the cache.** Only its own messages change the cache, and the sound thread applies them in the order sent, so the mirror is exact. A slot goes to a unit that is free, else to the one wanted longest ago and not in the last round.
4. **A slot goes Free, Filling, Ready.** A fill marks it Filling and takes it out of what plays at once; the copy can take several periods; only when every sample is in does it become Ready. A read of a Filling slot is silence, counted as missed: never half a unit.
5. **Generations only go up.** A source's new generation (a replace) or a close drops its cached units and any fill being copied, on the sound thread, before anything else is applied. A fill or a reply for an older generation, or an older project, is dropped and counted on whichever side sees it first; both sides check.
6. **Neither side blocks or allocates on the channels.** Both are bounded std channels made at start; the sound thread only uses `try_send` and `try_recv`, and the coordinator only polls, so a send never has to wake a waiting thread.

Handing slots over without the copy (Published and Retired states) comes only if the worst-period tests show the copy matters, under the rules in [The sound thread's side](#the-sound-threads-side).

### The interface for the sampler and the launcher

A player that wants a sound from disk needs four things: `Coordinator::open(source, path)` or `open_memory(source, channels, data)` (each gives the source's `Tag`) and `close(source)` on the coordinator's side; a `Cursor::new(source, id)` made off the sound thread; and `Cursor::play(&mut player, out)` once a period on the sound thread, with `seek`, `set_speed`, `set_loop` and `set_end` between periods. A start partway into a period is a call with the rest of the period's buffer. The sound thread calls `Player::begin` at the start of each period and `Player::end` at its end, once for all the cursors.

An engine built on the layer reuses its tests through the `testkit` feature (`mpfree-stream = { path = "../mpfree-stream", features = ["testkit"] }` in its dev-dependencies): marked sound files and memory sounds, whose every sample says which sound and frame it is, and `Expect`, which checks a period's output against what the sound thread has now.

### Tests

- `tests/format.rs`: every kind of WAV and AIFF built byte by byte and decoded exactly.
- `tests/stream.rs`: every format played back exactly through the coordinator and the cache; loops crossing a unit's edge, seeks into a unit, speeds 0.5, 1.5 and 2 landing on the right frames; two cursors sharing a source; a cold start silent and counted, never wrong; a device that fails, comes up short and answers out of order, never played; and **the unload and replace test**, in one thread (6,000 periods of random replaces, closes, new projects, seeks, loops and moves over a cache too small for what plays) and with real threads (the coordinator and its I/O thread replacing and closing sources while this thread plays). Every sample of every file says which file and frame it is, so each frame played is checked: silence, or exactly the frame it should be of the generation the sound thread has now. Planting either bug (a replace that keeps the old units, or a slot Ready before it is copied) makes both fail.
- `tests/no_alloc.rs`: the sound thread's calls never allocate or free, through a busy run with periods from 1 to 4096 frames.
- The unload and replace test also opens every fourth sound in memory, sets and moves cursors' ends, and holds and releases parts as it goes.

## P1.5: planning reads and admission

In `mpfree-stream`'s coordinator, tested in `tests/plan.rs`:

- **Deadlines.** Each unit a cursor wants has a deadline in seconds: the output frames before it plays, at the cursor's speed, round its loop, over the output's rate (`Plan::rate`). Reads go earliest deadline first.
- **Joined reads.** A read takes the next units of the same file with it, up to 512 KiB (`Plan::join_bytes`), while the time it would take at the device's measured speed, after the reads already queued, stays within half the time left before the next other deadline. A joined read that comes back short keeps the units it brought whole and counts the rest as short.
- **Cancelling.** Each round, a read none of whose units are wanted any more (its cursor jumped, its source was closed or replaced) is marked cancelled, and the I/O thread skips it if it hasn't started; one wanted again before then is unmarked.
- **Holds.** `hold_all(parts, by, tier)` keeps parts of sources in the cache whether or not anything plays them: a start, a loop point, a cued launch, fetched by a deadline or in the background. A group succeeds whole or not at all, and only if it fits beside what plays (at the least safe window), what is reserved and what is held at its tier or above; a refusal says how many units it needed and how many were left. A close, a replace or a new project lets a source's holds go.
- **Who gives way.** Holds have a tier, in the plan's order: Pinned ("Keep in memory"), Cued (a launch's starts), Head (the playing song's starts and loop points), Next (the next song). What plays at the least safe window and capture memory come before every hold. When room runs short (a cursor wants more, a recording reserves more, a higher hold comes in), holds are given back lowest tier first, newest first, and each one given back is reported (`Coordinator::given_back`), so an answer like "next song ready" that counted on it is withdrawn at once, not left standing.
- **Whole-file reads.** `read_file(path, tier)` reads a whole file through the device for something other than playing (a kit's sounds, a preset, the next song's files), in steps of at most 512 KiB, one step in flight at a time and only once every playback read wanted is asked for, so a playback read never waits behind more than one step. Next-song reads go before background ones. Writes (a recording's audio, an autosave at the lowest tier, in the project's save order) join the same queue with P1's recording step; past three quarters of capture memory, background work stops.
- **Launch readiness.** `Coordinator::not_ready(parts)` names the sources of a launch whose part isn't in the cache yet; at the boundary, the sound thread's own answer is `Player::covers(source, from, frames)`. Moving a whole launch to the next boundary is the transport's part.
- **Admission.** Each round the coordinator adds the units the playing cursors want at the wanted window, the units held, and the units reserved for recordings (`set_reserve`), and compares them with the cache. `Admission` says whether that is covered; if not, the window planned with shrinks to what fits (never quietly: `covered` is false), and `safe` says whether that is still at least the least window (4 s). The window comes from the device's measured worst shortfall (`Plan::for_shortfall`: twice it plus one second, never under 4 s). The design's own case is a test: a stick with a 20 s stall wants 41 s ahead, about 3,550 units for 16 stereo stems, so they don't fit 128 MB (2,048 units), and 8 do.
- **Still to come:** recording pressure (with recording), and deadlines shared between a read and its decoding (with FLAC and rate conversion).
