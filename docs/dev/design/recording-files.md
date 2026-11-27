<!--
covers: rust/mpfree-stream/src/rec.rs rust/mpfree-stream/src/recover.rs rust/mpfree-stream/src/fsio.rs rust/mpfree-stream/src/crc32c.rs rust/mpfree-stream/src/testkit.rs rust/mpfree-stream/tests/record.rs rust/mpfree-stream/tests/carry_on.rs
checked: 2026-11-25 0c8f3f52
-->

# Recording files that survive a power cut

**Status:** approved by the owner on 2026-11-22, after a review whose ten points are all in (the [Review decisions](#review-decisions-22-nov) table holds the five answers). The file protocol, its writer and recovery are built in `mpfree-stream` (`rec`, `recover`, `fsio`, `crc32c`) and tested against a model stick. The library browser and the sampler read the finished files, RF64 included. The second step is built too (`writer`): writes and syncs on a thread of their own, waiting for playback's reads; rounds spread across the interval; the interval lengthening when rounds fall behind; the capture reserve from measured commit times; the lane's "Unsaved" seconds. Nothing records with this in the app yet: the audio tracks rethink switches recording to it in its phase 3, and the 12 Nov engine's writer stays until then.

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

## Scheduling the rounds (the second step)

`writer::Writer` holds the takes and runs every file operation; `writer::Recorder` runs one on a thread of its own, so the sound thread, the coordinator and the screen never wait for the stick.

- **Waiting for playback.** The coordinator shares its cushion (the time until a playing cursor needs a unit not yet cached; no limit when nothing plays) through a `Gate`. A round waits while the cushion is no longer than the stick's worst commit time, and never more than 10 s past its time; then it goes anyway. Once its take's capture memory is half full it doesn't wait at all (recording moves up). Waits and rounds that went anyway are counted for Dev. Writing filled units doesn't wait for playback. It can still wait for the stick: on the MPC the kernel holds at most 1 MiB of written data before a writer waits (`vm.dirty_bytes`, D0), so on a stick slower than its takes the writes take most of a pass.
- **A pass** (`Writer::tick`) writes every take's filled units, then runs every round that is due by then, the most overdue first, each only if the wait above lets it. Until 25 Nov a pass ran one round. On the MPC's stick, where the writes took seconds, the rounds starved: 10 checkpoints in 42 s for 8 takes, with some take nearly all unsaved at the stop, and the app's take memory filled ([D3 and D4](disk-streaming.md#d3-and-d4-on-the-mpc-24-and-25-nov)).
- **The worst commit time** is the longest of the last 64 rounds that touched the stick (a round with nothing to commit measures nothing), and 1 s until there is one.
- **Spreading.** A new take's first round goes in the middle of the widest gap between the other takes' rounds, so four takes armed together commit at a quarter of the interval apart; set `spread` off and they all go together. D4 measures which a real stick prefers.
- **Falling behind.** A round that starts more than an interval late, or takes longer than one, doubles the interval, up to 10 s. After 16 rounds in a row on time, each under a quarter of the interval, it halves back towards the one set. Both are counted for Dev. (The spec said the interval doubles; coming back is this page's choice, so that one slow moment doesn't double what a cut can cost for the rest of the set.)
- **The capture reserve** is, for each take still recording, its data rate x (the worst commit time + the interval), given to the coordinator in cache units (`reserve_units`, for `Coordinator::set_reserve`).
- **Recording pressure** goes the other way through the gate: the fullest take's capture memory, and whether any take has captured audio not yet saved (`Gate::recording`; an armed take that has captured nothing, a track only monitoring, doesn't count). Background reads and checked copies wait while one is (next-song reads go on): on the MPC's stick a heavy read or copy beside a recording put it 17 to 24 s behind in 45 s ([D2b](disk-streaming.md#d2-and-d2b-on-the-mpc-23-nov)).
- **"Unsaved: N s"** shows on a take's lane (`TakeView::show_unsaved`) once more than twice the interval is uncommitted.
- **Capture memory across the threads.** `Recorder::push` counts what it has sent and not yet seen taken, plus what the take holds; once that would pass the take's limit it takes no more frames and the take stops by itself (Stopped, memory) and is finished. A stick that stops answering can't fill memory unseen.
- **Stopping.** A stop makes the take's finish due at once (still waiting for playback, like a round). A take that stopped by itself is finished the same way. Arming happens on the writer's thread too; until its files are made the take's status is Arming, and a take that can't be armed says why.

D2 and D4 try 1, 2, 5 and 10 s intervals.

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
| the SD card is taken out (B2c) | the take carries on in a folder on the stick, [below](#a-take-whose-drive-goes-carries-on); nothing it committed is lost | At risk (gone) until its files on the stick are made, then Recording |
| capture memory fills | recording stops at that frame; what was captured is kept; warned at 90% | Stopped (memory) |

When a later round succeeds the take goes back to Recording or Saving. Errors are counted per take (`TakeStats`).

## A take whose drive goes carries on

The SD card plan's B2c ([sd-card-and-internal-drive.md](../hardware/sd-card-and-internal-drive.md)): a take recording on the card when the card is taken out goes on recording on the stick, and loses nothing it captured. `Take::carry_on(fs, dir, card, serial)`:

- **What stays where it was:** everything committed on the card (its first C frames, in its `.recording` file there, checked by its last record). Its files there are let go of (`Fs::let_go`: the real file layer closes them) and never touched again.
- **What moves:** everything after C, still in capture memory, and all that comes after. It goes into new files in `dir` under the same name `NAME_ID`: a recording file and a checkpoint made as at arming (S1 to S3: header, sequence 0 with nothing committed, both synced, the folder synced). Each unit is written at its own place in the take, so the new file is the whole take's length and plays in step with the song. Where the first C frames would be, it holds zeros: a hole, which ext4 doesn't store.
- **What is missing, said first:** before those files, a hole record, `NAME_ID.wav.hole`, is made and synced (only when C > 0). One 4096-byte record: `MPFRHOLE`, the version, the take's id, C and its bytes, the format, the CRC-32C of the C frames as committed on the card (what any frames brought back must match), the card's marker id and volume serial (zeros until B2c's marker exists), and the take's recording path on the card, then a CRC-32C of all that. Recovery leaves it alone (it isn't named as a take).
- **Its checks from then on cover the file as it is,** zeros and all: the whole-take check starts from the CRC-32C of C zero bytes (`crc32c::zeros`, worked out in a few dozen steps however long), and the fast check's last 64 KiB from zeros. So a cut on the stick recovers the take as any take, and the whole-take check passes; nothing in recovery knows about holes. A stop with nothing left to write still makes the file the take's full length (`set_len` over the hole).
- **If its files on the stick can't be made** (the stick full or failing), what was made goes, and the take stays as it was: at risk, its capture memory filling. `Writer::carry_on` keeps asking each pass until it works (`carry_failed` counts the tries); the take's view says `carrying` meanwhile, and `hole` (C) once done. Nothing is carried when the take is already finished or its final record is on the card (it is whole there, and recovery finishes it).

Joining the card's part back into the take, when the same card returns, is B2c's next step; until then the take plays with silence where its card part was.

## The interface

- `rec::Take::arm(fs, dir, name, id, format, meta, capture_limit)` runs S1 to S3; then `push` (capture), `write_ready` (full units out), `checkpoint` (one round), `stop` and `finish` (P1 to P5, carrying on from a failed step when called again), `disarm`, `carry_on` (its drive went: carries on in another folder). `state`, `committed`, `unsaved`, `memory`, `nearly_full`, `stats`, `dir`, `hole`. `rec::Hole` and `hole_path` for the hole record.
- `recover::recover(fs, dir)` gives a `Found` per take: `Finished`, `CutOff`, `Unchecked`, `Unverified`, `Removed`, `Newer`, `Failed` (a step failed; nothing lost; tried again next start). `WholeCheck` / `check_whole`, then `checked`.
- `fsio::Fs` is the one file layer (create, write, read, length, sync, folder sync, rename, remove, list, and `let_go` of a file on a drive that went); `StdFs` is the real one.
- `writer::Recorder::start(fs, clock, gate, config)`, then `arm` (gives a `TakeNo` at once), `push`, `stop`, `disarm`, `forget`, `carry_on`; `status` (`Arming`, `Failed`, or a `TakeView`: state, captured, committed, unsaved seconds, `show_unsaved`, memory, `nearly_full`, the take's counts, `hole`, `carrying`), `final_path`, `reserve_units`, `interval`, `worst_commit`, `stats`; `close`. After `carry_on`, the take's status says `carrying` at once and until the writer has taken the ask (the view's `asks` counts those taken, the recorder those sent), so a status the writer published before it is never read as the answer: once `carrying` is false, `final_path` is in the new folder, or the take wasn't carried (saved, or being finished, where it was). `writer::Writer` is the same without the thread, driven by `tick`. `Coordinator::gate` gives the gate.

## How it is tested

`testkit::ModelStick` stands in for the stick. A cut loses what wasn't synced (data by its file's sync, names by the folder's); the write in flight can land torn (its first n sectors); names can land none, all, or only the latest (a folder written out of order); and a cut can also spoil the next 4 KiB block, the other checkpoint slot, or the header. Operations can fail by number (a failed write lands half; a failed sync drops the pages), the stick can fill or be pulled. `tests/record.rs`:

1. **A cut after every operation** of arm, every round, stop and finish, for stereo float and for mono 24-bit with a pad byte, with and without failures mixed in: 5 ways the data lands x 3 ways the names land x 4 kinds of damage. Every time: the take comes back as exactly the first R frames recorded, R at least what was reported committed (unless the extra damage hit a synced record) and at most what was captured; a Saved take comes back whole; every written frame past R is in the tail or an unverified file; the whole-take check passes on good audio and fails on damaged.
2. **Recovery cut at every step**, three ways: the same take comes back.
3. Damaged headers (zeros, noise) recover as good ones; a clean header naming another take is kept unverified; a byte changed mid-take passes the fast check and fails the whole-take check; both slots spoiled, or nothing committed, keep the audio unverified; a newer version and other files are left alone; slot 0 holds sequence 0 and the first checkpoint goes to slot 1.
4. Faults: failed writes come back from At risk; a failed sync's dropped pages are written again (a cut that loses everything unsynced proves it); a full stick holds the take until room is made; a pulled stick holds it until capture memory fills; failed arming leaves nothing.
5. Round trips through the real file system: 24-bit and float, mono and stereo, read back exactly by `mpfree-stream`, the library browser and the sampler (`mpfree-library/tests/recordings.rs`, `mpfree-sampler/tests/recordings.rs`), in RIFF and in RF64 form; just under and just over the RIFF limit with sparse files; `fact` present and right.

Seven planted bugs were each caught: no rewrite after a failed sync, sequence 1 in slot 0, no audio sync before the record, no tail kept, recovery trusting the header's id, a record not synced, and no folder sync before removing the checkpoint.

`tests/writer.rs` drives the writer on a made-up clock, with a model stick whose syncs and writes can move that clock on (a slow stick): rounds on the interval and "Unsaved" never shown; a round waiting while playback has half a second of cushion, "Unsaved" shown just past twice the interval, the round going 10 s late and the interval doubling; four takes spread to a quarter interval apart, or together; 5 s rounds taking the interval from 2 to 4 to 8 s, quick rounds bringing it back to 2, never past 10; the reserve from the assumed and then the measured commit time, for one take and two, and none once stopped; a round that stops waiting for playback once capture memory is half full, with the fill shared through the gate; capture memory running out before the first round, then the take finished with exactly what fitted; 8 takes on a stick that writes 2 MB/s, driven as the recorder's thread drives it (a pass, then the frames that came meanwhile), every take committing each time it is due and holding only the stick's real backlog, where one round a pass made 10 rounds and left up to 45 s of 46 unsaved (and the coordinator's `tests/plan.rs` stops background reads past three quarters while the next song still prepares). Then the recorder on its own thread with real files: a take recorded, stopped and read back exactly; a take that can't be armed; capture memory counted on the caller's side while playback leaves the stick no room. Fourteen planted bugs were each caught: no spreading, no waiting for playback, no longest wait, no lengthening, no coming back, a reserve without the measured time, no limit on the caller's side, no finishing a take that stopped by itself, "Unsaved" at one interval, rounds with nothing to commit counted as measured, no moving up at half, background reads not stopping, the fill not shared after a round, and one round a pass.

`tests/carry_on.rs` runs a take against two model drives, the card and the stick: the card pulled mid-take with audio still in memory, then carried on, for stereo float and mono 24-bit: no longer at risk, nothing on the card touched, the hole record exact (C, the CRC-32C of the committed bytes, the card's id and serial, the path there), and the finished take on the stick the whole length, zeros for the card's part and every later frame exact. A cut on the stick after every one of its operations, four ways: recovery leaves no recording, and every take it finishes passes the whole-take check with zeros then exact audio (audio before the first record there is kept unverified, as for any take). Each of the carry-on's 10 operations failing: nothing left on the stick, the take still at risk on the card, and the next try works. Nothing committed on the card: no hole record, the take whole on the stick. A stop with everything committed on the card: the file still reaches its length. The fault sweep for B2c, `a_card_pulled_after_any_step_keeps_what_its_hole_record_says_and_no_frame_is_lost`: the card pulled after each of its operations in turn while a take records, stops and finishes on it (a checkpoint every third write), for both formats, the take carrying on on the stick whenever the card is gone; then the card comes back each of four ways a cut leaves it. Pulled while arming: nothing recorded, nothing on the stick. Pulled once the final record is on the card: the take is whole there, nothing on the stick, and the card's recovery finishes it with every frame exact. Pulled before that: the stick holds only the finished take, the whole length with zeros for the card's part and every later frame exact, and the hole record; however the card comes back, it holds that part at the record's path, its bytes matching the record's CRC. The hole record round trips and refuses a changed byte or a path too long. The writer: a carry-on while the stick is full keeps trying each pass, then carries on once there is room, finishes the take there and loses nothing that waited. `crc32c::zeros` matches feeding the zeros, up to past 4 GiB. `tests/writer.rs`'s `the_recorders_status_says_carrying_until_the_writer_has_carried_the_take`: on the recorder's thread, never seen settled in the old folder after the ask, then the finished file zeros for what stayed and the rest exact.

On the MPC, later and each with its own go: the cut test D6, and D2 and D4 for intervals, sync times and spreading.

## Review decisions (22 Nov)

| Question | Decision |
|---|---|
| Checkpoint interval | 2 s by default; D2 and D4 measure 1, 2, 5 and 10 s, and a stick's profile can change it |
| The tail after the last record | kept as its own unverified file, never part of the take, never cut away |
| Recovering a cut-off take | finished by itself; the only question is whether to put it back on its lane |
| `FFFFFFFF` sizes while recording | kept, in MPFree's working file only; no promise about other programs |
| Audio at byte 4096 | kept, for speed; not a safety guarantee |
