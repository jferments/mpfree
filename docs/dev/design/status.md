<!--
covers: rust/mpfree-status rust/mpfree-app/src/debug_page.rs rust/mpfree-app/src/status_watch.rs
checked: 2026-11-22 e7409223
-->
# System status (`mpfree-status`)

**Status:** phase 0 built 2026-11-22: the `mpfree-status` crate (the catalogue, reports and the board, with its tests) and a Status page in Dev > Debug that shows the board raw. Phase 1 built the same day: the watchers that turn today's counters and states into reports, and a log line for each problem that starts or ends. Nothing changes for musicians yet: the status chip, the System status page and the red hint line arrive in phase 2. The full plan, its research and its decisions are in the Claude Doc [System status monitor](https://claude.ai/artifact/Re2JmUJFbmrg74hj8wCEyR), approved as recommended by the owner on 22 Nov 2026 (08:35Z). This page says what is built, as it is built.

**Owner:** the System status thread. Other threads add kinds to the catalogue and send reports; they don't draw a chip or a warning of their own.

## The goal

One place that says what is wrong, in a musician's words. The owner's rule (22 Nov 08:17Z): the CPU and RAM numbers sit on **one fixed-size status chip** that is green, yellow or red by how hard the MPC is working and by what has gone wrong. The chip never carries an error message: tapping it opens the System status page, which does. A critical problem may also show, briefly and in red, on the hint line at the bottom. No situation gets a top bar chip of its own.

The four decisions on the approval card:

1. The CPU number turns red only when the sound actually dropped out. A slow moment is yellow.
2. A problem that has ended keeps a ring on the chip until the page is opened, so a 3 ms dropout mid-set is still there when you look up.
3. A red problem goes on the hint line once when it starts, for 4 s, and again only after a minute.
4. The page opens from the chip or from a Prefs row and is called "System status".

## The pieces

All of it is in `rust/mpfree-status`, std only, with no I/O and no drawing.

- **`Kind`**: one kind of problem, a `static` in the catalogue (`kinds.rs`). It names its `System`, its `Level`, its `Shape`, short words for its system's line on the page (18 characters at most), a title, what happened, what to try, an optional `Action` (a button the page offers), the manual anchor of its entry, and the kinds it `hides` while it is going. A new kind joins by adding an entry, never by changing the screen. Kinds compare by `id` ("sound/dropout").
- **`Report`**: a kind, a `Subject(u32)` (which port, track, take or plugin; `Subject::NONE` for the whole system) and an `Op` (`Happened { times }`, `Began`, `Ended`). It is `Copy` with no heap memory (24 bytes at most, checked by a test), so any thread can send one through the screen channel without allocating.
- **`Board`**: lives on the screen thread and turns reports into the views the screen draws: `chip()` (the colour and whether it is a ring), `systems()` (the nine systems, each with its worst problem), `problems(system, Window::Now | Window::Session)` and, for the Dev page, `raw()` and `events()`. It takes the time as an argument, so tests drive the clock. It takes all its memory when it is made and allocates nothing after that (a test counts).

**Levels.** Fail (red): you heard a fault, work may be lost, or MPFree isn't answering you. Warn (yellow): MPFree gave something up to keep playing, is near a limit, or something you use isn't working, and nothing is lost. Note: worth listing for the session ("Take recovered"), but it never colours anything.

**Shapes.**

| Shape | Lives | Example |
|---|---|---|
| Happens | A moment, counted, with when it last happened. It counts as happening now for 3 s (`NOW_FOR`) | The sound dropped out |
| Lasts | A state with a start and an end. It shows once it has lasted 1 s (`SETTLE_ON`) and goes 3 s (`SETTLE_OFF`) after it stops; one that comes back inside that time is the same spell. One that ends before it ever showed is forgotten | The sound is near its limit |
| Stays | Shows at once and lasts until it is reported ended; a kind nobody ends stays until the next start | A MIDI device isn't plugged in |
| UntilSeen | Stays until the page has been opened | A take found damaged |

**The nine systems,** in page order: Sound, Memory, Storage, Pads and buttons, MIDI and sync, Project, Screen, Plugins, The MPC.

## The board's rules

- **One entry per kind and subject,** counted, with when it first and last happened. Up to 64 (`OPEN`). When the board is full, a new problem takes the place of the oldest ended one that has been seen, else the oldest ended one; if every entry is still going, the report is counted as lost and the Dev page says "No room on the board".
- **It latches until you look.** An ended problem keeps its colour, as a ring, until `Board::seen` (the page opened). It stays while it lasts, seen or not.
- **A cause hides its effects.** A report whose kind is in a going problem's `hides` list is kept but not shown, so the page names the cause, not its echoes (the pads not answering hides the dropouts it causes).
- **Worst first:** red before yellow, then a problem happening now before one that ended, then the most recent.
- **The last 256 reports** (`EVENTS`) are kept in a ring for the Dev page, with a count of older ones.
- **Every start and end is a `Change`** (`Board::take_changes`, up to 128 waiting, `CHANGES`), with the count and, for an end, how long it was showing. The screen thread writes one log line for each (below).
- **An end for a problem never reported** is ignored.
- `Board::unexpected(allowed)` lists the kinds on the board that a test didn't expect, for the soak and golden tests.

## The catalogue (phase 1's kinds)

| Id | Level | Shape | Short words | Button |
|---|---|---|---|---|
| `sound/dropout` | fail | happens | Sound dropped out | |
| `sound/skipped` | fail | happens | Sound skipped | |
| `sound/notes-dropped` | fail | happens | Notes dropped | |
| `sound/note-may-hang` | fail | happens | A note may hang | Silence all notes |
| `sound/voices-given-up` | warn | happens | Voices given up | |
| `sound/grains-thinned` | warn | lasts | Grains thinned | |
| `sound/near-limit` | warn | lasts | Sound near limit | |
| `memory/low` | warn | lasts | Memory low | |
| `memory/not-enough` | warn | happens | Not enough memory | |
| `storage/save-failed` | fail | happens | Save failed | |
| `controls/no-session` | fail | lasts | Pads not answering | |
| `controls/touch-stopped` | warn | lasts | Touch stopped | |
| `midi/port-missing` | warn | stays | Not plugged in | |
| `project/parts-missing` | warn | stays | Parts missing | Show what's missing |
| `project/crash-restart` | fail | stays | After a crash | Open the last project |
| `screen/no-gpu` | warn | stays | No GPU drawing | |
| `mpc/one-core` | warn | stays | Sound on one core | |

`controls/no-session` hides the dropouts, skips, dropped notes, voices given up and near limit that it causes.

## Disk streaming's kinds (phase 3)

Reported through `mpfree_stream::status::Watch`; [disk-streaming.md](disk-streaming.md#what-the-musician-is-told-the-storage-kinds) says what feeds each and why it has its level. In page order, in Storage beside `storage/save-failed`:

| Id | Level | Shape | Short words |
|---|---|---|---|
| `storage/stick-missing` | fail | stays | Stick missing |
| `storage/recording-at-risk` | fail | lasts | Recording at risk |
| `storage/recording-stopped` | fail | happens | Recording stopped |
| `storage/card-removed` | fail | happens | Card removed |
| `storage/stick-fell-behind` | fail | happens | Stick fell behind |
| `storage/recording-may-stop` | warn | lasts | Recording may stop |
| `storage/not-saved-yet` | warn | lasts | Not saved yet |
| `storage/streaming-at-limit` | warn | lasts | Streaming at limit |
| `storage/stick-nearly-full` | warn | lasts | Stick nearly full |
| `storage/take-damaged` | warn | until seen | Take damaged |
| `storage/take-recovered` | note | happens | Take recovered |

`storage/stick-missing` hides the failed saves, the takes at risk or not saved yet, and the stick nearly full that it causes; `storage/recording-at-risk` hides Not saved yet.

The catalogue's tests keep the words honest: every field filled, titles without a full stop, what happened and what to try ending with one, short words within 18 characters, no engineer words (underrun, buffer, lane, period, thread, DSP), ids unique with their system's prefix and a lower-case slug, manual anchors unique, `kinds::ALL` in page order, a button's words named in what to try, and every hidden effect a real kind.

## Where each kind comes from (phase 1)

The watcher (`rust/mpfree-app/src/status_watch.rs`) looks each time the audio loop's timing arrives, every 250 ms (`Player::status_look`). It compares counters with its last look, and reports a state when it begins and when it ends; the board settles it. The other kinds are reported where they happen.

| Kind | Comes from | Where |
|---|---|---|
| `sound/dropout` | the sound card's underruns (`Timing::underruns`), what they went up by | watcher |
| `sound/note-may-hang` | note endings the queues found no room for (`AudioNumbers::ends_lost`) | watcher |
| `sound/near-limit` | the whole period's last window Tight or Over against its budget (`engines::near_limit`, the CPU chip's colour) | watcher |
| `memory/low` | free memory (MemAvailable) under 1.5 times the 256 MB floor, or RAM over 85 %; back once 32 MB more is free and RAM is 3 points under | watcher, from the screen thread's look at `/proc/meminfo` every 2 s |
| `memory/not-enough` | the Sample page's lines that say something wasn't loaded or was left out for want of memory (`memory::SHORT`); a sound only kept at its own rate still loaded | `Player::print_sample_lines` |
| `storage/save-failed` | the store's "not saved" for the FX shelf, touch tuning, settings and the MIDI rig | `main.rs` |
| `controls/no-session` | the session chip's No reply or Stopped (red). Its No session (yellow: playing without Akai's setup, the pads still play) isn't this kind; phase 2 needs a yellow kind for it before the chip goes | watcher |
| `controls/touch-stopped` | the touchscreen's read failed and it hasn't opened again; it ends when it does | the screen loop |
| `midi/port-missing` | a port the project uses (a track's MIDI out, a track's input route, the clock it follows, the Controller and Follow while on) not plugged in at two looks running, 2 s apart (`IoModel::missing_ports`), once the app is ready. The subject is the port; the Dev page and the log name it | watcher |
| `project/parts-missing` | the browser's left-out list, while it has anything (it saves as a new project) | watcher |
| `project/crash-restart` | the safe start | `gui.rs` |
| `screen/no-gpu` | the GPU process stopped and software drawing took over, until the next start | `drawing.rs` |
| `mpc/one-core` | a set-up row in red on CPU by engine: the audio thread not on its core, or a strict lane on the audio thread (`CoreShared::fell_back`) | watcher |
| `sound/skipped`, `sound/notes-dropped`, `sound/voices-given-up`, `sound/grains-thinned` | the sampler refactor's step 10 trades: a lane late with its part, full queues, load shedding, the grain share crowded | the sampler's `Trades::note` (its step 10, agreed 22 Nov); the watcher doesn't watch these counters |

**The log.** Each start and end is one line, `status: + sound/dropout (3) The sound dropped out` and `status: - sound/dropout after 1.2 s`, with the port's name for a port and "(its cause is going)" for an effect its cause hides, so the Console, the journal and the test tools see what the board sees. The 30 s status lines stay as they are.

## How a thread reports

1. Add the kind to `rust/mpfree-status/src/kinds.rs` and to `kinds::ALL`, in its system's place. The catalogue tests say what is missing.
2. Send a report to the screen thread: `ToUi::Problem(Report::happened(&kinds::SAVE_FAILED, 1))` through `Out::show` in `main.rs`, or call `Player::report_problem` on the screen thread itself. Use `.about(Subject(n))` for a port, track, take or plugin; a lasting or staying kind sends `Report::began` and later `Report::ended`.

The audio thread never reports. Watchers on the screen thread read its counters (phase 1), so its rules (no allocation, no locks) stay as they are.

Phase 0 sends two kinds for real: a save that didn't work (`storage/save-failed`, for the FX shelf, touch tuning, settings and MIDI rig files, beside their "not saved" notes in `main.rs`) and the safe start after a crash (`project/crash-restart`, where `gui.rs` starts safe).

## The Dev > Status page

Dev > Debug's strip gains **Status**, between Touch and Console (`View::StatusBoard`, `debug_page::status_rows`). It shows the board raw, for developers, in engineer words:

- Left, "Problems this session": the status colour first ("all good", or for example "fail, ring · 1 now · 1 ended, not seen"), then one row per problem, labelled by level and id (and `#n` for a subject), with its state, how many times, when it first and last happened (a state that is going shows only when it began), and whether it is unseen or hidden by its cause.
- Right, "Last reports, newest first": the last 40 reports, each with when it came, with the number of older ones in the title.

Times are since MPFree started ("at 3.5 s", "at 1:10"), not ages, so they stand still: the page looks twice a second while it shows, and at once when a report arrives, and redraws only when the board changed (`no_page_redraws_while_idle`). A missing port shows by its name ("warn midi/port-missing Juno-X"). The golden screen `debug-status` draws it with a dropout, a missing MIDI port, voices given up and the sound settling near its limit.

The `Player` keeps its own status clock, moved on by `tick(dt)`, so the board's timing in the app's tests is exact.

## Roadmap

| Phase | What lands | State |
|---|---|---|
| 0 | The `mpfree-status` crate (catalogue, reports, board) and the Dev > Status page; save failures and the crash restart report | built |
| 1 | Watchers on the screen thread for the kinds above; one log line each time a problem starts or ends; the demo songs play green. Still only in Dev > Debug. The sampler's trades move in with its step 10, and the bench's check moves to `Board::unexpected` with its step 13 | built (the trades: with the sampler) |
| 2 | The status chip (CPU and RAM, coloured; it replaces today's CPU, RAM and session chips), the System status page (opened from the chip or a Prefs row), the red hint line, the manual chapter "When something goes wrong", golden screens and the controls catalogue. Its layout spec change lands first, as its own docs pull request for the owner | planned |
| 3 | Everyone reports in: disk streaming, themes, plugins, MIDI clock, the MPC's temperature and processor clock, each owner's pull request adding its own catalogue entries | planned |
| Later | Not in this go: where the time goes per track and pad, saving a report to the stick, and errors from the system journal from a short reviewed list | |

## Tests

- `cargo test -p mpfree-status`: the catalogue's words, the board's rules (moments, counts, settling on and off, latching, inhibition, worst first, a full board, the event ring, no allocation once made).
- `cargo test -p mpfree-app --bin mpfree -- problems_reach_the_status_board`: a report reaches the board and the Dev page follows it.
- `cargo test -p mpfree-app --bin mpfree -- status_watch:: the_watchers_turn a_missing_midi_port`: each watched counter and state, memory's lines, a port after two looks and by its name.
- `cargo test -p mpfree-app --bin mpfree -- every_song_plays_green`: each demo song opens and plays eight bars with nothing on the board (`Board::unexpected`).
- `cargo test -p mpfree-midi-ui -- missing_ports`: the ports a project uses and isn't plugged into.
- `cargo test -p mpfree-app --test cli`: the `debug-status` golden screen.
