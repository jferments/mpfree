# Renders: one description, provenance and kinds of audio

Phase 6.0 of the audio tracks rethink: the foundation every tool of phase 6 builds on before it renders anything (Slip, Reverse and Flatten in 6.1, Overdub a take clip in 6.2, Bounce in 6.3, Freeze in 6.4, Render in place in 6.5). The plan is the rethink's phase 6 doc (mpfree-claude `notes/audio-tracks/08-phase-6-plan-the-missing-tools.md`, "6.0 The foundation"); its build steps are note 21 there. Nothing here changes what a musician sees: the first tool to use it is 6.1's.

**Status:** 6.0.1 and 6.0.2 built, as one pull request (the description, its fingerprint, provenance and the kinds of audio, saved with the `audio-provenance` gate; the revisions of what a render reads, and the check of whether they still hold). 6.0.3 (one way to finish a render, as render jobs) and 6.0.4 (the routing check) are not built yet.

## One render description (6.0.1, as built)

Every tool says what it wants made in one value, `mpfree_audiotracks::render::RenderDesc`, before anything is rendered. It isn't one way of making sound: the region player, the looper and Export's pass still do the work, and each answers to this one contract.

| Part | Field | What it says |
| --- | --- | --- |
| Source | `source` (`Source`) | A recording's frames (its id and length), an audio clip, a loop's layers, a track, several tracks into one channel, a bus, or the mix |
| Range | `range` (`Range`) | Frames of the source; a clip's pass or the common cycle of several, in ticks; or song ticks with the tempo map's tempo at the start and its shape over them (`copies::shape_of`) |
| Capture | `capture` (`Capture`) | The source alone (no mixer), a strip before or after its fader, a bus's output, or the master |
| Tail and preroll | `tail`, `preroll` | Frames that ring on after the range, and frames played before it to bring delays, reverbs and envelopes to their running state |
| Quality and seed | `quality` (`Quality`), `seed` | The pitched reads' interpolation version, the stretch's settings and version; the chance rolls' seed |
| Dependencies | `reads` (`Read`: a `Dep` and its revision) | A recording (its revision is its frames), a track, a clip, a loop, the mixer's routes, the project |
| Purpose | `purpose` (`Purpose`) | A disposable copy, a lasting recording, a freeze, or an export |
| How it may run | `runs` (`Runs`) | On the scheduler's worker, exclusive through the live engine, or in real time |

- **Checked first** (`RenderDesc::check`): a capture its source has no such point for (a recording before its fader, a track alone, the mix at a bus's output), a worker render that needs the mixer, a copy taken from the mixer, several tracks that are one or repeat, a range of frames not all in its recording, an empty cycle, a song range that doesn't run forward at a tempo: each refused with its reason. A tool never offers one that fails.
- **A recording's frames as they are** (`RenderDesc::frames`) is the render 6.0 makes on the worker; Consolidate, Reverse and Commit stretch (6.1) build on it.
- **Renders through instruments stay exclusive on the MPC**, through the live engine, until a second engine graph is measured to fit there; on the workstation they may later run beside it on a snapshot of the project. That is a flag on the job (`Runs`), not a fork in the code.

## Its fingerprint (6.0.1, as built)

`RenderDesc::fingerprint` gives 128 bits that name a description: FNV-1a over a fixed little-endian encoding of every part, the render format's version (`render::FORMAT`, 1) first. It is the same in every run on every machine (the test pins one value), the reads in any order give the same number, and any part changed, the algorithm versions and the reads' revisions among them, gives another. How it runs isn't in it: the same description made exclusive or in real time is the same audio.

One fingerprint answers "is it still right": the description as the project stands now has the same fingerprint as the one a recording was made from (`Provenance::matches`), or it doesn't. A fingerprint of another format never matches. Phase 5's tempo copies keep their own key (`copies::CopyKey`), which already is a whole description of a disposable copy (the recording's frames, the range in ticks with the map's shape, the stretch's settings and version); its hash is their scheduler key.

## Provenance (6.0.1, as built)

Jay's rule for all of MPFree's audio: every audio representation can be traced to its musical source, its timing, its processing and the project state it was made from. Each recording a render makes keeps a `render::Provenance`:

- **`reads`:** everything it read, each with its revision then. The recordings it was made from are among them, each with its frames (`Provenance::sources`), so the original stays findable for making it again from source rather than stretching a stretch.
- **`made`:** its description's fingerprint, and **`format`**, the render format it was described in.
- **`purpose`** (a recording or a freeze: a take is never a copy or an export) and **`at`**, when it was made (seconds since 1970, UTC; `render::now_secs`).

**Saved on its take line**, before `file=`, as `Provenance::words` writes them and `Provenance::from_words` reads them:

```text
take=3 rec=5b2f0d9c41e8a7763c0f12ab9e4d8c01 track=0 group=1 pass=0 bpm=120 anchor=0 frames=44100 channels=1 made=7553038e417fb63f5d483f44e785870a for=recording ver=1 at=1764100000 reads=r5b2f0d9c41e8a7763c0f12ab9e4d8c01.44100,t9.3,routes.12 file=Bounce.wav
```

`reads=` is each read as a letter and its id, a dot and its revision, by commas: `r` a recording (32 hex digits), `t` a track, `c` a clip, `l` a loop (32 hex digits), `routes`, `project`. A take recorded or brought in has none of these words. Wrong words refuse the part, saying which ("where take 3 came from: for=copy is not a recording or a freeze"), as a wrong `rec=` does; a newer format's provenance is kept as it is and never matched.

**The `audio-provenance` gate:** a project whose audio tracks' part has a take with provenance needs `audio-provenance` (`save::needs_provenance`, the app's `project::AUDIO_PROVENANCE`), so a build from before 6.0.1, which would skip the words and drop them at its next save, refuses it instead.

**Kept by the host through a save.** The audio thread never holds provenance (it allocates). When a project opens, each take's provenance is moved off its `TakeFile` and sent to the worker (`ToWorker::Prov`, beside the take's file name in `ToWorker::Known`), on every path a take opens by: as its file, written to a file first, copied into memory, or left out (then only to be dropped there). The worker's file table keeps it by recording id (`Files::set_prov`) and lets it go when the tracks are emptied; a save's `Files::name_save` writes it on the take line again. A committed render (6.0.3) gives it the same way.

## Three kinds of audio (6.0.1, as built)

| Kind (`render::Kind`) | Examples | Made again? | Kept on save? |
| --- | --- | --- | --- |
| **Primary** | Takes, sounds brought in, committed bounces, renders, Commit stretch and Print | Not necessarily | Yes, while anything names it |
| **Persistent derived** | Freezes | While its sources and engines are here | Yes, while it is in use |
| **Disposable derived** | Tempo copies, reverse copies, renders in progress | Yes | No |

`Purpose::kind` gives a render's kind (an export has none: it is outside the project), `Provenance::kind` a recording's. A take with no provenance is primary. They share one file format, one reader and the recordings table; cleanup, saving and Verify (6.6) act by kind: Remove unused never lists a freeze in use, and never lists a copy at all.

## Revisions and what changed (6.0.2, as built)

A description's `reads` are its snapshot: each thing it read with its revision then. `render::changed(reads, now)` gives the first that no longer holds, as `now` says each is now (its revision, or None when it is gone), or None when all hold. One check answers whether a tempo copy, a freeze or a render under way is still right (Jay's review of phases 5 and 6, §20): a render whose reads no longer hold commits nothing over the newer state (6.0.3).

| `Dep` | Its revision | Given by |
| --- | --- | --- |
| `Recording` | Its frames, once it has stopped recording; None while it records or once it is gone | `AudioTracks::rec_rev` |
| `Track` | A number for the track's regions (every field of each) and the takes on it as they stand: the same whenever they are the same, so an UNDO back to the state read holds again, and another after any edit of them. Level, pan, mute and solo are the mixer's (`Routes`) | `AudioTracks::track_rev`, by engine number; the app maps the project's lasting number to it |
| `Project` | The tracks' generation, one more each time they are emptied (New project, a project opening) | `AudioHost::generation` |
| `Clip`, `Loop`, `Routes` | The app's: the clip, the loop's layers, the mixer's routes | 6.1 to 6.3, as each tool needs them |

`AudioHost::rev(dep, slot)` answers for the three the audio tracks hold. `track_rev`, `rec_rev`, `rev` and `changed` are cheap and allocate nothing, so they can run on the audio thread. `Dep::changed_words(name)` says what changed in a musician's words ("Vocal was edited while it rendered", "another project opened while it rendered"), for the log and, from 6.1 on, the line a tool shows when it commits nothing (drawn on its board first).

## Tests

- `mpfree-audiotracks/tests/render.rs`: `a_render_descriptions_fingerprint_is_fixed_and_every_part_changes_it` (the pinned value; the reads' order and how it runs leave it; each of 26 changes gives a new one), `a_description_is_checked_before_anything_is_rendered` (each refusal, each purpose's kind), `provenance_goes_on_a_take_line_and_comes_back` (the words, through a part and back to the same text, the gate, every read's word, a freeze, each wrong word refused with its reason, a newer format kept and not matched), `a_take_opened_with_provenance_keeps_it_through_a_save` (through two hosts, and let go of when a project opens).
- `tests/render.rs` `what_a_render_read_holds_until_it_changes_and_says_what_changed` (a recording has no revision while it records; another track's edit leaves the reads holding; a gain on track 1 breaks only track 1's, UNDO holds again, REDO breaks it; a split; the project's generation after a clear; the host's answers; the words), and `tests/no_alloc.rs` `revisions_and_what_changed_never_allocate` (every track's revision, a recording's and the check, each period while recording and after).
- The app's `project.rs` `audio_following_the_tempo_needs_audio_tempo_and_phase_4_refuses_it`: a take with provenance makes the project need `audio-provenance`, and a build without it refuses the project.

## Not built yet

- **6.0.3, one way to finish a render:** reserve, snapshot, render into a hidden `.part` in the work folder on the scheduler, check, make durable, commit one step in the tracks' history, release; nothing committed over a project changed under it; at start every hidden `.part` in the work folder goes.
- **6.0.4, the routing check:** `Mixer::stand_in`, beside `capture_for`, says whether a render can stand in for its source with the mix unchanged (sends, sidechains, buses, latency).
