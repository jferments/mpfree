<!--
covers: rust/mpfree-stream/src/rec.rs rust/mpfree-stream/src/recover.rs rust/mpfree-stream/src/fsio.rs rust/mpfree-stream/src/crc32c.rs rust/mpfree-stream/src/testkit.rs rust/mpfree-stream/tests/record.rs
checked: 2026-11-22 26165526
-->

# Recording files that survive a power cut

**Status:** approved by the owner on 2026-11-22, after a review whose ten points are all in (the [Review decisions](#review-decisions-22-nov) table holds the five answers). The file protocol, its writer and recovery are built in `mpfree-stream` (`rec`, `recover`, `fsio`, `crc32c`) and tested against a model stick. The library browser and the sampler read the finished files, RF64 included. **Not built yet:** syncs on a thread of their own, scheduled with the reads; rounds spread across the interval; the interval lengthening by itself; the capture reserve from measured commit times; the lane's "Unsaved" seconds. Nothing records with this in the app yet: the audio tracks rethink switches recording to it in its phase 3, and the 12 Nov engine's writer stays until then.

**Owner:** the disk streaming thread ([disk-streaming.md](disk-streaming.md), step P1).

## What this decides

A power cut or a pulled stick while recording costs only the audio not yet committed: normally the last checkpoint interval (2 s by default) plus one sync. It never costs the whole take, and never plays back audio that wasn't recorded.

- **Scope:** every recording `mpfree-stream` writes (audio track takes first; later loops, resampled sounds and imports written as files). One take is one file.
- **Promise:** a take is **Saved** only after every sync below has returned. Whether the stick itself keeps synced data through a cut is a fact about that stick, measured by the cut test (D6) and shown in its profile, not assumed here.
- **When the stick falls behind** or stops taking writes, the audio not yet committed grows. It stays in capture memory, and the take's lane shows how much ("Unsaved: 12 s").
- **Identity:** a take is known by its recording id, a random 128-bit number in both file names and in every checkpoint record. Nothing needed to recover the audio lives only in the WAV header.
- **Never trusted:** the WAV header and the file's length after a cut. Recovery rebuilds both from the newest checkpoint record whose audio checks out.
- **Never assumed:** that a write of 4 KiB, or of any size, lands whole. A cut can damage the block being written and, on some sticks, blocks near it.
- **Ordering rule:** a checkpoint record never names audio that wasn't synced before the record was written.
- **Nothing thrown away by itself:** audio that can't be proved is kept apart as unverified, never played as part of a take, and removed only when the musician says so.

## The two files

While a take records it is two files in its project's media folder: `NAME_ID.wav.recording` (the audio) and `NAME_ID.wav.checkpoint` (two fixed-size records saying how much of it is safe). `ID` is the recording id as 32 hex digits, so the files pair up by name alone; the name the musician sees comes from the project. At stop the first is renamed `NAME_ID.wav` and the second removed. Everything is little-endian; checksums are CRC-32C.

**`NAME_ID.wav.recording`: a 4096-byte header, then the audio from byte 4096.** Starting the audio on a 4 KiB boundary keeps writes in line with the file system's pages and the stick's usual blocks. It is a speed choice, not a safety one. The header is written once at arm and once at stop.

| Bytes (24-bit) | Bytes (float) | Field | While recording | After stop |
|---|---|---|---|---|
| 0-11 | 0-11 | `RIFF`, size, `WAVE` | size `FFFFFFFF` | true size; past the RIFF limit `RF64`, size `FFFFFFFF` |
| 12-47 | 12-47 | `JUNK`, size 28, zeros | as written | unchanged; past the limit `ds64` (RIFF size, data size, frame count, table length 0; EBU Tech 3306), so the audio never moves |
| 48-71 | 48-73 | `fmt `: format 1 (PCM) with size 16, or format 3 (float) with size 18 and cbSize 0 | as written | unchanged |
| none | 74-85 | `fact`, size 4: frame count (float only) | `FFFFFFFF` | the frame count, or `FFFFFFFF` with RF64 |
| 72-4087 | 86-4087 | `JUNK`: `MPFREE-RECORDING`, the recording id, version 1, flags 0, a CRC-32C of those 36 bytes, zeros | as written | unchanged |
| 4088-4095 | 4088-4095 | `data`, size | size `FFFFFFFF` | true size, or `FFFFFFFF` with RF64 |
| 4096- | 4096- | audio, frames interleaved | written in units | whole |

- **The RIFF limit:** a finished take stays RIFF while its audio plus pad byte is at most 4,294,963,206 bytes (`FFFFFFFE` minus the 4088 header bytes the RIFF size counts), so no size field ever holds `FFFFFFFF`, which means "see `ds64`". That is about 3 hours of stereo float at 48 kHz.
- **`FFFFFFFF` while recording** means "not known yet". Only MPFree's recovery reads a `.recording` file, and it takes the length from the checkpoint. No promise is made about other programs opening a working file.
- **Readers:** `mpfree-stream`'s reader, the library browser (`mpfree-library` `audio.rs`: `read_info`, `overview`) and the sampler's loader (`mpfree-sampler` `wav.rs`, `sound_file.rs`) read RF64, `ds64`, `fact` and `JUNK`. The review asked for the readers to be updated; they were taught RF64 in place rather than moved onto `mpfree-stream`'s reader, which keeps their own messages and limits. The 12 Nov engine's `save.rs` reads only takes it wrote; it retires when this writer takes over, and a project naming these takes says so in its `needs` line, so an older build refuses it whole.
- **Write units:** a whole number of frames and of 4 KiB, the smallest such size of at least 64 KiB (64 KiB for stereo float, 72 KiB for 24-bit), except the last.

**`NAME_ID.wav.checkpoint`: 8192 bytes, two 4096-byte slots.** A record with sequence number *n* goes in slot *n* mod 2, so each write lands on the older of the two. A cut can spoil the slot being written and, on some sticks, the other one too.

| Bytes | Field |
|---|---|
| 0-7 | magic `MPFRCKPT` |
| 8-9 | version (1) |
| 10-11 | flags: bit 0 = final (stopped and finished) |
| 12-15 | zero |
| 16-31 | recording id (must match the id in the file names) |
| 32-39 | sequence number: 0 for the record written at arm, then 1, 2, 3 ... |
| 40-47 | frames committed |
| 48-55 | bytes committed (frames x block align) |
| 56-63 | where the audio starts (4096) |
| 64-75 | format, channels, rate, bits, block align |
| 76-79 | fast check: CRC-32C of the last committed 64 KiB (or all, if shorter) |
| 80-83 | whole-take check: CRC-32C of all committed audio, extended over each round's newly committed frames from capture memory, never twice for a write tried again |
| 84-99 | the project's lasting id |
| 100-103 | track, then zero |
| 104-111 | where it starts on the song, in ticks |
| 112-119 | when it was recorded (Unix ms, 0 if the clock isn't set) |
| 120-123 | CRC-32C of bytes 0-119 |
| 124-4095 | zero (checked) |

A record is **valid** when its magic, version, checksum and zero padding are right, its format is one MPFree writes and its id matches the file names. Recovery never needs the header. Its id is compared only when its MPFree block reads cleanly; a clean header naming another id means files were mixed up, and the take is kept as unverified.

## Starting a recording

The files are made when a track is **armed**, so pressing record never waits on the stick. If they can't be made, the track doesn't arm and says why. Disarming without recording removes them.

| Step | What is done | A cut right after it leaves |
|---|---|---|
| S1 (arm) | create `NAME_ID.wav.recording`, write the header | at most 4096 bytes and no checkpoint: removed at the next start |
| S2 (arm) | create the checkpoint; slot 0 = sequence 0, 0 frames; slot 1 = zeros | no audio in either: removed |
| S3 (arm) | sync both files, then the folder | 0 frames committed, nothing past byte 4096: removed |
| S4 (record) | audio goes to the file, unit by unit | the next section |

The first checkpoint is sequence 1, in slot 1, so slot 0 survives a cut that spoils that first write. A cut before the first checkpoint commits nothing; audio already written is kept as unverified.

## While recording: the checkpoint round

Every interval (2 s by default; one round in flight per take):

| Step | What is done | A cut right after it leaves |
|---|---|---|
| C1 | F = the frames of the longest run from the start whose units were all written | the previous record (C frames): recovered to C |
| C2 | `fdatasync` the `.recording` | still C |
| C3 | write the record (sequence + 1, F frames, both checks) into the older slot | if spoiled, the other slot still says C; if both were spoiled, unverified |
| C4 | sync the checkpoint | F or C; both safe, since F's audio was synced in C2 |
| C5 | F frames are **committed** and may leave capture memory | F |

- **Capture memory** holds every frame until it is committed, not just written. After a failed sync (Linux can drop the pages and report it once) every unit in it counts as unwritten and is written again before the next try. **Write buffers** live only for their write. **The playback cache** plays the rest; being cached never counts as saved.
- **Checks** come from capture memory, never from reading the file back.
- **Not built yet** (the second recording step): syncs run on a thread of their own and are scheduled with the reads' deadlines (a round waits for playback reads due within the stick's worst sync time, never past 10 s); several takes' rounds are spread across the interval (D4 measures that against all at once); the interval doubles, up to 10 s, when rounds fall behind, counted in Dev; the lane shows "Unsaved: N s" past twice the interval; the capture reserve is the data rate x (worst measured commit time + one interval). D2 and D4 try 1, 2, 5 and 10 s intervals.

## Stopping

A take is **Saved** when the rename has been synced. Stopping is separate from saving the project, which only ever names Saved takes.

| Step | What is done | A cut right after it leaves |
|---|---|---|
| P1 | write the last unit (and pad byte) and the true header; sync | the last record (C): recovered to C; the header isn't needed |
| P2 | write the final record (all N frames, flag final); sync | finished at the next start |
| P3 | rename to `NAME_ID.wav` | either name: finished, or only its checkpoint left |
| P4 | sync the folder: **Saved** | `NAME_ID.wav` |
| P5 | remove the checkpoint (no folder sync) | a leftover checkpoint is handled at the next start |

P4 comes before P5 so that no stick, even one writing its folder out of order, can keep the removal and lose the rename.

## Recovery at the next start

Before a project opens, `recover` pairs `.recording` and `.checkpoint` files by the id in their names. The format comes from the record, so a damaged header doesn't matter.

- **Fast check, at start:** valid records newest first; one **checks out** when the file holds its committed bytes and the last 64 KiB match.
- **Whole-take check, afterwards:** `WholeCheck` reads the take a piece at a time (a background read behind playback, about 40 s for an hour of stereo float on a USB 2 stick). Until it has run the take shows "Checking"; a mismatch marks it **Damaged**, kept. The take's checkpoint stays until `checked` is called after this check, so a check cut short by another power cut runs again (the take is then found as `Unchecked`).

| What is found | What MPFree does | What the musician sees |
|---|---|---|
| a final record that checks out | finishes the stop | "Finished saving Take 3; its stop was cut off." |
| a non-final record of more than 0 frames that checks out | keeps the tail apart, then the same finish, by itself | "Take 3 (Drums, 21:14) was cut off by a power loss and is saved up to 2:14. Put it back on its lane?" |
| 0 frames committed, nothing past byte 4096 | removes both files | nothing (logged) |
| audio but nothing provable (0 committed, both slots spoiled, a clean header naming another id) | keeps it as `NAME_ID.unverified.wav`, header rebuilt from the length in whole frames | "Take 3 couldn't be checked; it's kept to listen to." |
| the same with no format anywhere | keeps the bytes as `NAME_ID.unverified.raw` | "Take 3 couldn't be read; its raw audio is kept." |
| a checkpoint with no audio file it vouches for | removes it | nothing (logged) |
| a version this MPFree doesn't know | leaves both alone | "Take 3 was recorded by a newer MPFree." |

- **The tail is kept apart:** audio past the record that checks out is copied to `NAME_ID.tail.unverified.wav` and synced with its folder before the take is cut, so the cut never destroys it. If a cut also spoiled the slot holding the newest record, recovery goes back to the older record, and the audio after it is kept the same way.
- **Remove**, offered on recovered takes, tails and unverified files, is the only way any of this audio is deleted.
- **Recovery is safe to cut:** a fixed order (tail, header, length, sync, rename, folder sync; the checkpoint last), each step the same if done twice.

## Failures while recording

| What fails | What MPFree does | State |
|---|---|---|
| a unit's write | 3 tries; later units go on; nothing past it is committed | At risk (writes); tried again each round |
| a sync | writes every uncommitted unit again from capture memory, syncs again; 3 rounds | At risk (syncs) |
| a record | writes it again into the same slot and syncs; 3 tries | At risk (records) |
| the stick is full | keeps recording into capture memory | At risk (full) |
| the stick is pulled | the same | At risk (gone) |
| capture memory fills | recording stops at that frame; what was captured is kept; warned at 90% | Stopped (memory) |

When a later round succeeds the take goes back to Recording or Saving. Errors are counted per take (`TakeStats`).

## The interface

- `rec::Take::arm(fs, dir, name, id, format, meta, capture_limit)` runs S1 to S3; then `push` (capture), `write_ready` (full units out), `checkpoint` (one round), `stop` and `finish` (P1 to P5, carrying on from a failed step when called again), `disarm`. `state`, `committed`, `unsaved`, `memory`, `nearly_full`, `stats`.
- `recover::recover(fs, dir)` gives a `Found` per take: `Finished`, `CutOff`, `Unchecked`, `Unverified`, `Removed`, `Newer`, `Failed` (a step failed; nothing lost; tried again next start). `WholeCheck` / `check_whole`, then `checked`.
- `fsio::Fs` is the one file layer (create, write, read, length, sync, folder sync, rename, remove, list); `StdFs` is the real one.

## How it is tested

`testkit::ModelStick` stands in for the stick. A cut loses what wasn't synced (data by its file's sync, names by the folder's); the write in flight can land torn (its first n sectors); names can land none, all, or only the latest (a folder written out of order); and a cut can also spoil the next 4 KiB block, the other checkpoint slot, or the header. Operations can fail by number (a failed write lands half; a failed sync drops the pages), the stick can fill or be pulled. `tests/record.rs`:

1. **A cut after every operation** of arm, every round, stop and finish, for stereo float and for mono 24-bit with a pad byte, with and without failures mixed in: 5 ways the data lands x 3 ways the names land x 4 kinds of damage. Every time: the take comes back as exactly the first R frames recorded, R at least what was reported committed (unless the extra damage hit a synced record) and at most what was captured; a Saved take comes back whole; every written frame past R is in the tail or an unverified file; the whole-take check passes on good audio and fails on damaged.
2. **Recovery cut at every step**, three ways: the same take comes back.
3. Damaged headers (zeros, noise) recover as good ones; a clean header naming another take is kept unverified; a byte changed mid-take passes the fast check and fails the whole-take check; both slots spoiled, or nothing committed, keep the audio unverified; a newer version and other files are left alone; slot 0 holds sequence 0 and the first checkpoint goes to slot 1.
4. Faults: failed writes come back from At risk; a failed sync's dropped pages are written again (a cut that loses everything unsynced proves it); a full stick holds the take until room is made; a pulled stick holds it until capture memory fills; failed arming leaves nothing.
5. Round trips through the real file system: 24-bit and float, mono and stereo, read back exactly by `mpfree-stream`, the library browser and the sampler (`mpfree-library/tests/recordings.rs`, `mpfree-sampler/tests/recordings.rs`), in RIFF and in RF64 form; just under and just over the RIFF limit with sparse files; `fact` present and right.

Seven planted bugs were each caught: no rewrite after a failed sync, sequence 1 in slot 0, no audio sync before the record, no tail kept, recovery trusting the header's id, a record not synced, and no folder sync before removing the checkpoint.

On the MPC, later and each with its own go: the cut test D6, and D2 and D4 for intervals, sync times and spreading.

## Review decisions (22 Nov)

| Question | Decision |
|---|---|
| Checkpoint interval | 2 s by default; D2 and D4 measure 1, 2, 5 and 10 s, and a stick's profile can change it |
| The tail after the last record | kept as its own unverified file, never part of the take, never cut away |
| Recovering a cut-off take | finished by itself; the only question is whether to put it back on its lane |
| `FFFFFFFF` sizes while recording | kept, in MPFree's working file only; no promise about other programs |
| Audio at byte 4096 | kept, for speed; not a safety guarantee |
