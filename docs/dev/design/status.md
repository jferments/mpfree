<!--
covers: rust/mpfree-status rust/mpfree-app/src/debug_page.rs rust/mpfree-app/src/status_watch.rs rust/mpfree-app/src/status_screen.rs rust/mpfree-app/src/kernel_log.rs rust/mpfree-ui/src/status_page.rs
checked: 2026-11-22 e7409223
-->
# System status (`mpfree-status`)

**Status:** phase 0 built 2026-11-22: the `mpfree-status` crate (the catalogue, reports and the board, with its tests) and a Status page in Dev > Debug that shows the board raw. Phase 1 built the same day: the watchers that turn today's counters and states into reports, and a log line for each problem that starts or ends. Phase 2 built the same day: the status chip, the System status page, the red hint line and the manual chapter [When something goes wrong](../../user/src/reference/when-something-goes-wrong.md), after the owner approved the layout spec text (22 Nov 09:55Z). Phase 3 under way from 23 Nov: disk streaming's and the themes' kinds are in; the MIDI clock's and the plugins' kinds (Clock stopped, Plugin stopped) are built, with MIDI ports as tagged subjects, and the stick's meter on the page's head. The full plan, its research and its decisions are in the Claude Doc [System status monitor](https://claude.ai/artifact/Re2JmUJFbmrg74hj8wCEyR), approved as recommended by the owner on 22 Nov 2026 (08:35Z). This page says what is built, as it is built.

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
- **`Report`**: a kind, a `Subject(u32)` (which port, track, take or plugin; `Subject::NONE` for the whole system) and an `Op` (`Happened { times }`, `Began`, `Ended`). A subject's top 8 bits say what it names: `Subject::audio_track(id)` is an audio track by its lasting id, `Subject::found_take(n)` a take found at start (the audio tracks, [audio-tracks.md](audio-tracks.md) step 5d), and `Subject::midi_port(i)` the `i`th port the watcher has named this session (phase 3; a test checks the three never meet). The watcher names a subject by its tag, whatever the kind (`Watch::subject_name`); a subject it can't name says what it is on the Dev page and in the log ("port 2", "track 7": `Subject`'s `Display`). It is `Copy` with no heap memory (24 bytes at most, checked by a test), so any thread can send one through the screen channel without allocating.
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

## The catalogue

| Id | Level | Shape | Short words | Button |
|---|---|---|---|---|
| `sound/dropout` | fail | happens | Sound dropped out | |
| `sound/skipped` | fail | happens | Sound skipped | |
| `sound/notes-dropped` | fail | happens | Notes dropped | |
| `sound/note-may-hang` | fail | happens | A note may hang | Silence all notes |
| `sound/voices-given-up` | warn | happens | Voices given up | |
| `sound/grains-thinned` | warn | lasts | Grains thinned | |
| `sound/near-limit` | warn | lasts | Sound near limit | |
| `memory/ran-out` | fail | happens | Out of memory | |
| `memory/low` | warn | lasts | Memory low | |
| `memory/not-enough` | warn | happens | Not enough memory | |
| `storage/stick-errors` | fail | happens | Stick errors | |
| `storage/stick-files-error` | fail | happens | Stick files error | |
| `storage/save-failed` | fail | happens | Save failed | |
| `controls/no-session` | fail | lasts | Pads not answering | |
| `controls/no-setup` | warn | stays | Without Akai setup | |
| `controls/touch-stopped` | warn | lasts | Touch stopped | |
| `midi/port-missing` | warn | stays | Not plugged in | |
| `midi/clock-stopped` | warn | lasts | Clock stopped | |
| `project/parts-missing` | warn | stays | Parts missing | Show what's missing |
| `project/crash-restart` | fail | stays | After a crash | Open the last project |
| `screen/no-gpu` | warn | stays | No GPU drawing | |
| `screen/theme-left-out` | warn | stays | Theme left out | |
| `screen/theme-mistakes` | warn | stays | Theme has mistakes | |
| `screen/theme-not-found` | warn | stays | Theme not found | |
| `screen/skin-colours-moved` | warn | stays | Colours fixed | |
| `plugins/plugin-stopped` | fail | happens | Plugin stopped | |
| `mpc/running-hot` | warn | lasts | MPC running hot | |
| `mpc/processor-slowed` | warn | lasts | Processor slowed | |
| `mpc/one-core` | warn | stays | Sound on one core | |

`controls/no-session` hides the dropouts, skips, dropped notes, voices given up and near limit that it causes; `mpc/running-hot` hides the processor slowed that it causes.

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

## Where each kind comes from (phases 1 and 3)

The watcher (`rust/mpfree-app/src/status_watch.rs`) looks each time the audio loop's timing arrives, every 250 ms (`Player::status_look`). It compares counters with its last look, and reports a state when it begins and when it ends; the board settles it. The other kinds are reported where they happen.

| Kind | Comes from | Where |
|---|---|---|
| `sound/dropout` | the sound card's underruns (`Timing::underruns`), what they went up by | watcher |
| `sound/note-may-hang` | note endings the queues found no room for (`AudioNumbers::ends_lost`) | watcher |
| `sound/near-limit` | the whole period's last window Tight or Over against its budget (`engines::near_limit`, the CPU chip's colour) | watcher |
| `memory/low` | free memory (MemAvailable) under 1.5 times the 256 MB floor, or RAM over 85 %; back once 32 MB more is free and RAM is 3 points under | watcher, from the screen thread's look at `/proc/meminfo` every 2 s |
| `memory/not-enough` | the Sample page's lines that say something wasn't loaded or was left out for want of memory (`memory::SHORT`); a sound only kept at its own rate still loaded | `Player::print_sample_lines` |
| `memory/ran-out` | the kernel's log: "Out of memory: Killed process N (mpfree…)" (or the memory cgroup's), for MPFree's own programs only; read from the start of the boot, so the safe start after the kernel stopped MPFree itself shows it (after phase 3) | `kernel_log.rs` |
| `storage/stick-errors` | the kernel's log: "… error, dev sdX, sector …" and "Buffer I/O error on dev sdX…" (after phase 3) | `kernel_log.rs` |
| `storage/stick-files-error` | the kernel's log: "EXT4-fs error (device sdX…)", "FAT-fs (sdX…): error" and "exFAT-fs (sdX…): error" (after phase 3) | `kernel_log.rs` |
| `storage/save-failed` | the store's "not saved" for the FX shelf, touch tuning, settings and the MIDI rig | `main.rs` |
| `controls/no-session` | the session's No reply or Stopped (red) | watcher |
| `controls/no-setup` | the session's No session (yellow): no session within 20 s at the start, so the pads play without Akai's setup; it ends if the session starts later (phase 2, when the session chip went) | watcher |
| `controls/touch-stopped` | the touchscreen's read failed and it hasn't opened again; it ends when it does | the screen loop |
| `midi/port-missing` | a port the project uses (a track's MIDI out, a track's input route, the clock it follows, the Controller and Follow while on) not plugged in at two looks running, 2 s apart (`IoModel::missing_ports`), once the app is ready. The subject is the port; the Dev page and the log name it | watcher |
| `midi/clock-stopped` | the clock the project follows (Sync's Tempo from) stopped arriving while the device played: its pulses stopped for half a second without a Stop. The time engine's follower lets go of the tempo and holds `Follower::lost` (`time/follow.rs`) until a pulse or a Stop comes, or following changes; the audio loop's timing carries it (`AudioNumbers::clock_lost`). Yellow: MPFree plays on at the last tempo, nothing is lost (phase 3) | watcher |
| `project/parts-missing` | the browser's left-out list, while it has anything (it saves as a new project) | watcher |
| `project/crash-restart` | the safe start | `gui.rs` |
| `screen/no-gpu` | the GPU process stopped and software drawing took over, until the next start | `drawing.rs` |
| `plugins/plugin-stopped` | a plugin's process ended: what the rack's crash count (`mpfree_plughost::rack::Summary::crashes`, read without waiting) went up by. The rack restarts it from its last state, and leaves it off at the fourth crash in a minute. Red: its sound stopped, which may have been heard (phase 3) | watcher |
| `mpc/running-hot` | the hottest thermal zone at or over `heat::RUNNING_HOT_C`, until it is 3 °C under it (`HOT_BACK_C`). The line is 73 °C, measured on the MPC, never guessed (below, "The MPC's temperature and processor clock"); it ends at 70 °C (phase 3) | watcher, from the screen thread's read every 2 s (`heat.rs`) |
| `mpc/processor-slowed` | a core's ceiling (`cpufreq/scaling_max_freq`) below where it was at the first read, which is what the kernel's cooling or a governor does to slow the processor; a clock under its ceiling is a governor resting an idle core and isn't one (phase 3) | watcher, from the same read |
| `mpc/one-core` | a set-up row in red on CPU by engine: the audio thread not on its core, or a strict lane on the audio thread (`CoreShared::fell_back`) | watcher |
| `sound/skipped`, `sound/notes-dropped`, `sound/voices-given-up`, `sound/grains-thinned` | the sampler refactor's step 10 trades: a lane late with its part, full queues, load shedding, the grain share crowded | the sampler's `Trades::note` (its step 10, agreed 22 Nov); the watcher doesn't watch these counters |

**The log.** Each start and end is one line, `status: + sound/dropout (3) The sound dropped out` and `status: - sound/dropout after 1.2 s`, with the port's name for a port and "(its cause is going)" for an effect its cause hides, so the Console, the journal and the test tools see what the board sees. The 30 s status lines stay as they are.

## How a thread reports

1. Add the kind to `rust/mpfree-status/src/kinds.rs` and to `kinds::ALL`, in its system's place. The catalogue tests say what is missing.
2. Send a report to the screen thread: `ToUi::Problem(Report::happened(&kinds::SAVE_FAILED, 1))` through `Out::show` in `main.rs`, or call `Player::report_problem` on the screen thread itself. Use `.about(subject)` with a tagged subject for a port, track or take (`Subject::midi_port`, `Subject::audio_track`, `Subject::found_take`; a new kind of subject adds its tag and its name in `Watch::subject_name`); a lasting or staying kind sends `Report::began` and later `Report::ended`.

The audio thread never reports. Watchers on the screen thread read its counters (phase 1), so its rules (no allocation, no locks) stay as they are.

Phase 0 sends two kinds for real: a save that didn't work (`storage/save-failed`, for the FX shelf, touch tuning, settings and MIDI rig files, beside their "not saved" notes in `main.rs`) and the safe start after a crash (`project/crash-restart`, where `gui.rs` starts safe).

## The status chip, System status and the hint line (phase 2)

The screen side is `rust/mpfree-app/src/status_screen.rs`; the page's widget is `rust/mpfree-ui/src/status_page.rs` (`StatusPage`), which draws only the `StatusView` it is given and knows nothing of the board. Layout spec, sections 2 and 2a.

- **The status chip** is the top bar's rightmost chip, always there: "CPU 24 %  RAM 38 %", as wide as "CPU 100 %  RAM 100 %". It replaced the CPU, RAM and session chips. `Board::chip_look(ram_pct)` gives its colour, the worst of the CPU, the RAM (yellow over 70 %, red over 85 %) and the problems happening now or ended unseen, and the colour of each number: the CPU number is red while `sound/dropout` is happening now and yellow while `sound/near-limit` is, never for a slow moment alone. While the colour comes only from problems that ended unseen, the chip is drawn as a ring (`Chip::ring`). It is lit while the page shows. Before the first readings it says "CPU –" and is grey. The CPU number is `engines::cpu_pct`, the audio thread's mean share of the period. A tap sends `FromUi::StatusChip` and the app toggles `View::SystemStatus`.
- **System status** (`View::SystemStatus`, page "System status") opens over the page from the chip or Prefs > System status (`Item::SystemStatus`, after System info); the chip again, MENU or a tab closes it. It opens on the worst system (`Place::opened`: red before yellow, then happening now before ended unseen), and while it shows every problem counts as seen (`Board::seen`), so the ring goes. The head names the worst problem happening now, or "Everything is working", with the counts; the meters are CPU, RAM and, once each can be read, the stick (the audio tracks' drive watcher's last look) and the MPC's temperature (`Temp`, filled towards the lowest critical trip point, or 100 °C where there is none; yellow while `mpc/running-hot` is going), all phase 3. With four meters they are 128 px wide rather than 150; the head's title steps down from the title size to the body size, and then is cut short with "…", so it never runs under them, and so does the counts line. Sound's numbers are the sound's share of the CPU, then the tracks using the most of it (below, "Where the sound's time goes, per track"), then, under "By part" when tracks are listed, where it goes engine by engine (`engines::shares`: each of CPU by engine's stages, its last window's mean as a share of the period, so they add up to about the CPU share; in `engines::SHARES`' words: the pads, synths, the looper and line input, clips and the arp, mixing, and MPFree's own work for the control stage; "none" for an engine that didn't run, "under 1 %"). The MPC's line says "56 °C · full speed", and its numbers are the temperature, the trip point ("It turns itself off at"), the processor's speed and its top speed ("1200 MHz, down from 1608" while slowed). All nine systems are listed with their worst problem's short words (a port's name in front), else plain words ("CPU 24 % · no dropouts", "41 % used · 1.1 GB free", "Working"). The picked system's problems are cards, by `Board::problems`, with when ("3 times · last 12 s ago · first 4 min ago", "now · for 9 s", "ended 30 s ago") and a button for a kind's `Action`. Now | This session (This session at first) picks the window. The wheel and – / + step the systems (`StatusKey::Step`), the push opens the next folded card (`StatusKey::Open`). The page looks twice a second while it shows and at once on a report, and redraws when what it shows changed: its ages count the seconds for the first minute, so it is one of the live pages in `no_page_redraws_while_idle` (Dev > Status, with times since the start, is not).
- **Buttons:** Silence all notes is STOP's second press (`App::status_action`); Show what's missing opens the browser, whose status line names what was left out; Open the last project opens it at once when the safe start's new project is unchanged (`Player::open_last_now`, which ends `project/crash-restart`), else opens the browser, where opening a project asks about changes first.
- **The red hint line:** when a red problem starts showing and isn't hidden by its cause, its title goes on the hint line in red for 4 s, "The sound dropped out. Tap the status chip to see why.", once per kind a minute (`Hinted`); not while the page shows. Yellow never goes there. The safe start after a crash puts its own words there instead, in red ("The last run crashed, so Old was not opened. BROWSE opens it."), since they name the project and where the unsaved work is. The told line has a colour now (`Player::told`). The hint line is a `Label::on_bg()`: on the page's own background its red (and yellow and green) is kept readable with `Theme::readable`, since some themes' red doesn't read on their background (Slate's and AbleTown's grey). A card's count badge draws its number in `Theme::ink_on` its fill.

Golden screens: `system-status` (a dropout now, Juno-X missing, voices given up 30 s ago), `system-status-good` and `status-red-hint` (Play with the red hint line).

## The Dev > Status page

Dev > Debug's strip gains **Status**, between Touch and Console (`View::StatusBoard`, `debug_page::status_rows`). It shows the board raw, for developers, in engineer words:

- Left, "Problems this session": the status colour first ("all good", or for example "fail, ring · 1 now · 1 ended, not seen"), then one row per problem, labelled by level and id (and the subject's name, or what it is, such as "port 2"), with its state, how many times, when it first and last happened (a state that is going shows only when it began), and whether it is unseen or hidden by its cause.
- Right, "Last reports, newest first": the last 40 reports, each with when it came, with the number of older ones in the title.

Times are since MPFree started ("at 3.5 s", "at 1:10"), not ages, so they stand still: the page looks twice a second while it shows, and at once when a report arrives, and redraws only when the board changed (`no_page_redraws_while_idle`). A missing port shows by its name ("warn midi/port-missing Juno-X"). The golden screen `debug-status` draws it with a dropout, a missing MIDI port, voices given up and the sound settling near its limit.

The `Player` keeps its own status clock, moved on by `tick(dt)`, so the board's timing in the app's tests is exact.

## Save a report (after phase 3)

Jay's pick on 23 Nov 2026 at 11:43Z ("Save a report, then kernel errors", from the proposal in mpfree-claude `notes/status-monitor/after-phase-3.md`). A **Save a report** button at the foot of System status's cards panel, on the right, on every system (`status_page::SAVE_REPORT`; the cards and numbers stop above it; layout spec 2a). A tap makes the report's words on the screen thread (`Player::status_report`, `status_report::text`) and hands them to the sound loop as `FromUi::SaveReport`, which gives them to the store like a screen picture: `files/reports/report-YYYYMMDD-HHMMSS.txt` (UTC by the MPC's clock; `-2` within the second). The hint line says where it went, or why nothing was saved (no data folder this run, the store's error).

- **What it holds:** when it was saved and how long MPFree had run; Prefs > System info (`sysinfo::read`) without the host name; a normal start or a safe start with the crash's panic line (`Player::set_crash_why`); the page's numbers (CPU, RAM, free memory, the stick, the temperature and trip point, the processor's speeds); every problem on the board (`Board::raw`, hidden and settling ones too, marked), with its level, id, title, count, first and last, and whether it is going; and the last 200 `status:` lines (`Player::status_log`), each with its time since the start.
- **What it never holds:** serial numbers and network addresses (System info leaves them out), the host name, and names someone typed: a problem about a track or a take says "track 3" or "found take 1" (`status_report::about`, also used for the log's copy), and only a MIDI port keeps its device's name. The test `a_report_holds_the_session_and_no_names_typed` checks a track's name and the host name stay out.

## The kernel's stick and memory errors (after phase 3)

The second half of Jay's pick of 23 Nov 11:43Z. `rust/mpfree-app/src/kernel_log.rs` reads the kernel's log from the start of this boot on a thread of its own (`/dev/kmsg`, read-only; a read waits for the kernel's next line, so it costs nothing while the kernel is quiet) and turns a short, reviewed list of its messages into problems. Anything not on the list is ignored, so a harmless driver message never colours the chip.

| Message (Linux 6.18's own words) | Problem |
|---|---|
| `I/O error, dev sda, sector …` (any `… error, dev sdX, sector`, the block layer's `blk_print_req_error`) | `storage/stick-errors` |
| `Buffer I/O error on dev sda4, logical block …` | `storage/stick-errors` |
| `EXT4-fs error (device sda4): …` | `storage/stick-files-error` |
| `FAT-fs (sda1): error, …`, `exFAT-fs (sda1): error, …` | `storage/stick-files-error` |
| `Out of memory: Killed process 1234 (mpfree…) …`, `Memory cgroup out of memory: Killed process …` | `memory/ran-out` |

- **The stick** is any `sd` disk: the MPC One has one USB port, and the stick in it is MPFree's. The eMMC and the SD card (`mmcblk`) are left out, since MPFree's boot switches both off ([stick-autoboot.md](../hardware/stick-autoboot.md)). The stick being gone (`storage/stick-missing`) hides both stick kinds, whose errors are then its echoes.
- **Memory** counts MPFree's own programs only (`mpfree`, `mpfree-plughost`, …), as the proposal's list says. Reading from the start of the boot, not of this run, is what lets the safe start after the kernel stopped MPFree itself say why: the crash file can't tell an out-of-memory kill from any other crash outside Rust.
- **Bursts:** a failing stick writes many lines a second, so a second thread counts them and sends each kind at most once a second with its count (`Tally`, `Report::happened`).
- **Without `/dev/kmsg`:** the app's own user can't open it when the kernel restricts its log (`dmesg_restrict`). Then the kernel's lines come from the system log Dev > Console already follows (`follow_journal`'s one read-only `journalctl -b -f -n 300`, still the only program the app starts besides the GPU process and the Plugins place's downloads): `kernel_log::FromJournal` picks out the lines marked `kernel:`, so another program saying the same words doesn't count. That log starts 300 lines back rather than at the boot's start, and a `status:` line says which way the errors are read.
- **Checked so far:** `/dev/kmsg` opens for the app's user on the MPC: the disk streaming round's SD card check (24 Nov 2026, stick built from main 010e338b) logged "status: watching the kernel's log for stick and memory errors" and no "/dev/kmsg can't be read" line, so the errors are read from the start of the boot, not from the system log. `Buffer I/O error on dev mmcblk1p1, logical block 2` was seen on the MPC (stick-autoboot.md, 15 Nov), so that format is known. The others come from the kernel's source and are still unchecked on the MPC: no stick or memory error happened in that run. A round that sees one checks its line against the list.
- **Left out of this first list:** the proposal's `critical temperature reached`. The MPC turns itself off then, so it is in the previous boot's log, which `/dev/kmsg` doesn't keep; showing it at the next start needs a log kept across boots, still to be checked on the stick.

## The MPC's temperature and processor clock (phase 3)

`rust/mpfree-app/src/heat.rs` reads a few small files in `/sys` every 2 s on the screen thread, with `/proc/meminfo`, whatever shows, read-only; never the audio loop. The first read finds the files (one look in `/sys/class/thermal` and `/sys/devices/system/cpu`) and each zone's critical trip point; later reads open only the zones' `temp` and each core's `scaling_cur_freq` and `scaling_max_freq`. A part the machine doesn't have reads as None, so a desktop without zones shows no Temp meter.

- **What the MPC has** (seen in the disk streaming round D2b, 23 Nov, from `mpfree-disktest --under-load`): two zones, `cpu-thermal` and `gpu-thermal`, each with one trip point, critical at 113 °C; 55.5 °C at the start, 62.5 °C at the most after about 3 minutes of every core busy; the cores stayed at 1608 MHz throughout. With no passive trip point, the kernel itself doesn't slow the cores before 113 °C. Akai's services use a user-space thermal governor ([own-kernel.md](../hardware/own-kernel.md)); whether it runs under MPFree, and what it would turn down, isn't known yet (unverified).
- **Slowed** is a core's ceiling below where it was at the first read, not its clock below the top: a governor rests an idle core, and Akai's ceiling (1608 MHz) may be under the chip's own top (`cpuinfo_max_freq`, not read). So Processor slowed can only come from something turning the ceiling down.
- **Running hot's line is 73 °C** (`heat::RUNNING_HOT_C`, Jay's pick of 25 Nov 2026). 62.5 °C after 3 minutes (D2b) said little about where the MPC settles after an hour of a heavy project, so the kind stayed off until the disk streaming rounds D3 and D4 (24 and 25 Nov, [disk-streaming.md](disk-streaming.md), "D3 and D4 on the MPC") ran every core busy until the temperature settled, read-only, stopping at 75 °C (`mpfree-disktest --heat`): it settled at 65.4 °C and then 67.7 °C, 70.8 °C at the most, with the cores at 1608 MHz throughout. The line is a few degrees over the highest settled reading, so a heavy project never sets it off and anything past it is out of the ordinary; it ends 3 °C under (`HOT_BACK_C`), at 70 °C. The second day settled 2.3 °C warmer than the first, which a warmer room explains; a hot room is what the problem's words point at.
- **The cost:** `HeatReader::cost_line` logs "status: the MPC's temperature and clock: 10 files every 2 s, slowest read N µs (30 reads)" after the first minute and every half hour after, so any run on the MPC says what the reads cost there.

## Where the sound's time goes, per track

`rust/mpfree-app/src/track_costs.rs`. One period in `track_costs::EVERY` (4) the audio loop counts, for each sequencer track and each kit no track plays, where that period's sound time went, and every 250 ms (with CPU by engine's publish) each count becomes a share of the period, the same measure as `engines::shares`, in `EngineShared::tracks` (`TrackShared`: relaxed atomics, hundredths of a percent). The screen thread takes the five largest (`TrackShared::top`), words them and lists them on Sound's numbers under "The tracks using the most", above the engines, which move under "By part", so the tracks stay in view when cards push the numbers down to Save a report (the mockup's second picture). With nothing listed, Sound's numbers are as before.

| Counted | How | Whose |
|---|---|---|
| A kit's voices | No voice is timed ([profiling.md](profiling.md): never per voice). `Engine::count_kits` adds each playing voice's cost in grains (`Pool::cost`'s units: a voice 8, a plain one-shot 2, each live grain 1) by the frames it plays, by kit, one look over the voices each part of a block; the pads' measured time that period (the `sampler` stage) is shared out by those counts | the first track that plays the kit (`TrackKits::first_on`, kit 1 included, which sounds on the pad channels), else the kit, "Kit 2 · pads only" |
| A track's own instrument | `TrackHost::set_spending`: each slot that has something to play timed (two clock reads; on a lane it is timed there) | the track whose channel the slot plays into |
| A synth-voice pad's copy | the same, in `PadSynths` | its pad's kit, then as a kit |
| A strip's inserts | `Mixer::set_spending`: each chain that runs timed (`Chain::process_keyed`, two clock reads; the count moves with the chain to a lane and back; an empty chain is never timed) | a track's strip: the track (Drums, track 1's strip, is track 1's); a pad channel: its kit (`PadChannels::holder`), then as a kit; a kit strip (`mixing::KIT_FIRST` on, sampler step 15d): the kit it holds (`MixHost::kit_strips`), then as a kit; the engines' strips, submixes, returns, the master and free places: no track's, left out |
| The Synth tab's synth | its measured time that period (the `synth` stage) | the first track that plays it |

- **When:** the counting turns on at the start of period `i % EVERY == 0` and off at the start of the next one (`MixHost::set_spending`, `Player::count_kits`); the counted period's numbers are taken after the mix (`MixHost::take_spent`, `Player::take_kit_counts`, `TrackCosts::add_period`). A chain or instrument away on a late lane keeps its count, and its old setting, until it is back.
- **A close guess:** the shares come from a quarter of the periods, the pads' part is shared out by what each kit plays rather than timed, and work outside the tracks (the looper, clips and the arp, the master's and the submixes' effects, MPFree's own work) is in no track's share, so the tracks add up to less than the sound's share. With two or three sound cores, parts run side by side, so a share is of the sound's work rather than of the period's time; the manual calls the numbers a close guess.
- **The cost** (mpfree-bench, counted in instructions): with counting on every block, "Sampler, 4 drum kits, 64 voices, counted for System status" costs 1.7 % more than without, and "Mixer, 12 tracks and 8 pads, with a song's inserts, counted for System status" 0.1 % more; the app counts one block in four. Callgrind counts instructions, not the clock reads' own time: two reads for each chain and instrument that runs in a counted period, none for the kits. What that costs on the MPC isn't measured yet; the app's whole period is in CPU by engine either way.
- **Words:** "Track 3 · Drums" (the track's number, then its name when it was given one: `Player::status_tracks`), "Kit 2 · pads only"; a saved report says "the tracks using the most: track 3 12 %, kit 2, pads only under 1 %", by number only (`track_costs::report_label`), never a name someone typed.

## Roadmap

| Phase | What lands | State |
|---|---|---|
| 0 | The `mpfree-status` crate (catalogue, reports, board) and the Dev > Status page; save failures and the crash restart report | built |
| 1 | Watchers on the screen thread for the kinds above (the themes' four are reported by the theme code itself); one log line each time a problem starts or ends; the demo songs play green. Still only in Dev > Debug. The sampler's trades move in with its step 10, and the bench's check moves to `Board::unexpected` with its step 13 | built (the trades: with the sampler) |
| 2 | The status chip (CPU and RAM, coloured; it replaces today's CPU, RAM and session chips), the System status page (opened from the chip or a Prefs row), the red hint line, the manual chapter "When something goes wrong", golden screens and the controls catalogue. Its layout spec change (approved 22 Nov 09:55Z) lands first, as its own docs pull request | built |
| 3 | Everyone reports in: disk streaming (its Storage kinds, built), themes (their four came with the theme writers' tools, built), the MIDI clock and plugins (Clock stopped and Plugin stopped, built, with MIDI ports as tagged subjects), the stick's free space on the page's head (built: the audio tracks' drive watcher's last look, `files::DriveSeen`, with Storage's line and numbers), and the MPC's temperature and processor clock (built: `heat.rs` reads them every 2 s, the Temp meter and The MPC's numbers show them, Processor slowed is on; MPC running hot's line, 73 °C, measured on the MPC in the disk streaming rounds of 24 and 25 Nov). Each adds its catalogue entries and their manual entries | built |
| After 3 | Jay's pick of 23 Nov 11:43Z: **Save a report** (a button at the foot of the cards panel writes a plain text report to the stick; above), then **kernel errors** (stick and memory errors from the kernel's log, from a short reviewed list; above) | built; `/dev/kmsg` opens on the MPC (24 Nov); the error lines' formats wait for a round that sees one |
| Then | **Where the sound's time goes, by engine**, on Sound's numbers and in a saved report, from the numbers CPU by engine already publishes, with no change to the audio loop (built while the heat run waits, as a draft for Jay to take or leave) | built |
| Then | **Where the sound's time goes, per track** (Jay's go 24 Nov 2026 00:18Z, "Yes, as drawn", after his "Yes, mockup first" at 23 Nov 23:31Z; proposal and mockup in mpfree-claude `notes/status-monitor/per-track.md` and `mockups-per-track/`): the five tracks using the most on Sound's numbers and in a saved report (below) | built |

## Tests

- `cargo test -p mpfree-status`: the catalogue's words, the board's rules (moments, counts, settling on and off, latching, inhibition, worst first, a full board, the event ring, no allocation once made).
- `cargo test -p mpfree-app --bin mpfree -- problems_reach_the_status_board`: a report reaches the board and the Dev page follows it.
- `cargo test -p mpfree-app --bin mpfree -- status_watch:: the_watchers_turn a_missing_midi_port`: each watched counter and state, memory's lines, a port after two looks and by its name, the followed clock stopping and the plugins' crashes.
- `cargo test -p mpfree-app --bin mpfree -- status_report:: save_a_report`: the report's words (the session, the numbers, where the sound's time goes, no names typed, no host name, a safe start's crash), its name by the time, the store writing it, and the button reaching the store.
- `cargo test -p mpfree-app --bin mpfree -- shares_say the_sounds_numbers`: each engine's share of the period adding up to the CPU share, and Sound's numbers listing them, the tracks using the most first.
- `cargo test -p mpfree-app --bin mpfree -- track_costs::`: a counted period's pads shared out by kit, each kit to its track or kept as the kit's, instruments and inserts to their owner, the shares of the period, the five largest and their words. `mixing::tests::a_kit_strips_inserts_count_to_its_kit`: a kit strip's inserts count to the kit it holds, only while counting is on.
- `cargo test -p mpfree-sampler --test kits -- each_kits_voices_are_counted`, `cargo test -p mpfree-fx --test mixer -- spending_counts` and `cargo test -p mpfree-track --test host -- spending_counts`: each counts only while on, by kit, strip or slot, and the sound doesn't change.
- `cargo test -p mpfree-app --bin mpfree -- kernel_log::`: the list (the stick's read, write and file system errors, MPFree's programs stopped for memory, and what is left out: other programs, the eMMC and SD card, harmless lines), `/dev/kmsg`'s record header and continuation lines, a line that isn't UTF-8, the system log's kernel lines when `/dev/kmsg` can't be read, and a burst counted once a second.
- `cargo test -p mpfree-app --bin mpfree -- heat:: the_mpc_runs_hot the_mpc_shows`: the MPC's files read from a fixture tree as D2b saw them (the hottest zone, the critical trip, a resting core not slowed, a lower ceiling slowed, a machine without them), the cost line's times, running hot over its line and off without one, the app's 73 °C line against the MPC's settled readings, and the Temp meter and The MPC's numbers.
- `cargo test -p mpfree-app --bin mpfree -- a_lost_clock_lasts_until`: the follower's lost clock, from the pulses stopping while the device played to the next pulse or Stop.
- `cargo test -p mpfree-app --bin mpfree -- every_song_plays_green`: each demo song opens and plays eight bars with nothing on the board (`Board::unexpected`).
- `cargo test -p mpfree-midi-ui -- missing_ports`: the ports a project uses and isn't plugged into.
- `cargo test -p mpfree-app --bin mpfree -- status_screen:: the_status_chip_opens`: the chip's look, the page from the board, the worst system, the hint line once a minute, every kind's entry in the manual, the chip opening the page and a card's button reaching the app.
- `cargo test -p mpfree-ui -- status_page`: the page's targets and its cards opening and folding.
- `cargo test -p mpfree-app --test cli`: the `debug-status`, `system-status`, `system-status-good` and `status-red-hint` golden screens.
