# Scenes, follow actions and the scene morph (T9, T10)

A scene is a row of the launch grid. Launching it plays every track's clip in that row from the next bar line, so a whole part of the song changes at once. A scene can also switch the track mutes on that bar line, send outside gear a program change per track there (sequencer S5), and a follow action can chain scenes and clips into a song without an arrangement.

The model is in `rust/mpfree-seq/src/scenes.rs`. `rust/mpfree-perform` carries it out on the control thread with the launches and cues the host already keeps (see [playback.md](playback.md#launching-on-the-bar-line-cues-t9)).

## The model (`mpfree-seq`)

- **Scene `k` is row `k`** of every track's slots, so there are 64 (`SCENES = ROWS`, the Arrange refactor's 1.2: four banks of 16; the grid shows the first 16 until scene banks come). `Project::scenes` holds what a row has besides its clips. A row the project keeps nothing for is a plain scene: launching it still plays its clips. `Project::scene_mut(row)` makes the plain rows up to it when a row gets a name, mutes or a follow.
- **`Scene { name, mutes, follow, values, programs }`**.
  - `mutes: Option<u128>`: bit `t` mutes track `t` while the scene plays. `None` leaves the mutes as they are.
  - `follow: Option<Follow>`: what happens after it has played some bars.
  - `values`: parameter values stored with the scene, for the scene morph (T10, below).
  - `programs: Vec<TrackProgram>` (sequencer S5, the Elektron's program change per pattern): `TrackProgram { track, program: ClipProgram }`, at most one a track, kept in order of track (`Scene::set_program`, `Scene::program`). A track's gear gets it as the scene starts (below). `SceneSettings` carries them for the scene editor. Saved as a scene's `program <track> <program>[ bank <bank>]` lines, after its `value` lines; a track twice, past `MAX_TRACKS` or a value out of range refuses the part, and an older build skips the lines.
- **`Clip::follow`**: a clip can have a follow action of its own. A one-shot clip (`Clip::once`, S4) stops its track at the end of its pass instead, and its follow waits ([make-perform.md](make-perform.md#one-shot-clips-as-built-the-eighth-part)).
- **`Follow { bars, action, other, chance }`**: after `bars` bars (1 to 999, counted from where it started), `action` with a chance of `chance` %, and `other` the rest of the time. These are Ableton's A and B.
- **`FollowAction`**: Stay, Again, Next, Previous, First, Last, Any, Other, Stop.
  - For a clip, Next means the next filled slot down its track, wrapping round to the first after the last. Empty slots are skipped. For a scene, Next is the next row with a clip on any track.
  - Follow actions move within the bank of 16 rows the clip or scene is in (`mpfree_seq::bank`): from row 40, Next goes to the next filled row of rows 32 to 47 (counted from 0) and wraps round within them, never to the grid's rows (Arrange 1.2).
  - Any picks any filled one, this one included. Other picks any but this one, and stays when there is no other.
  - Stop stops the clip's track, or every track for a scene.
  - `follow_target` works out where an action goes. It never allocates.
- `Scene::check` and `Follow::check` keep the values in range (`ModelError::BadScene`, `BadFollow`), and `Project::check` runs them.

## Launching a scene (`Perform::launch_scene`)

- **While the transport rolls**, every track gets a cue for the next bar line: its clip in the row, or a stop when its cell is empty and it is playing something. They all start on the same bar line. A track with nothing playing or cued and an empty cell gets no cue.
- **On the project's launch quantise** (S4). The "bar line" is the next line of `Project::launch` (one bar by default): `launch_scene` keeps it for each cue it makes (`Perform::cue_line`), and `dress` sets it on every new view (`PlayView::set_cue_line`), so a clip's own Launch can't pull one track of the scene off the others' line. A follow's cue lands on its point, a bar line, whatever Launch says (`Bars(1)`). A cue taken back or started forgets its line.
- **Mutes switch on the same bar line.** A cue carries its track's mute (`PlayView::set_cue_mute`). The player switches the mute when the cue starts: a muted track's notes end on the bar line and nothing after it sounds. `Started::mute` reports it, and `Perform::started` makes it the track's `mute`. Each compile drops the cue mutes, so `Perform::dress` sets them again on every new view. A track with no cue (nothing playing or cued, an empty cell) still plays the song in song mode, so its mute waits for the same line: `launch_scene` keeps it (`Perform::take_line_mutes`, with the line: the project's, or the next bar line for a follow's or the song list's) and the host puts it in TRACK MUTE's mute queue (`TimeControl::queue_scene_mutes`), where it replaces whatever waited for that track and the queue lands on the scene's line. It shows as waiting like any queued mute (the pad blinks, the track's name says "mutes in 2 beats"), Rec song keeps the silence from the line, and STOP lands it at once. With Launch at once it is set at once, as the cues start at once. **With a solo** (S4's third part), the scene still sets the tracks' own mutes, and `TimeControl::view` makes each cue's mute solo-aware after `dress`: a track not soloed stays silent while any track is. A mute waiting in TRACK MUTE's queue lands on its own line; where it and a scene's cue switch a track, the later wins ([playback.md](playback.md)).
- **Program changes go out on the same line** (S5). `launch_scene` gives every track the scene holds a program for a `SceneProgram { program, line, id }` (`Perform::program`), a new `id` for each launch, and `dress` sets it on every view (`PlayView::set_scene_program`). It is not tied to a cue: a track with no clip in the row and nothing playing gets it too, so gear that plays its own patterns (a drum machine on a MIDI Out track with no clips) switches with the scene. The player sends it once on the first line of `line` it plays (a follow's and the song list's land on the next bar line, as their cues do), unless the clip playing on the track then has a program change of its own: that one goes out instead, so the clip's wins. Once its line has passed, a stop or a locate sends it again where the music starts, as a clip's own is; a loop jump doesn't. The next launch replaces every track's (a track the new scene holds none for keeps what its gear has, and none is sent again), and Stop all and an uncue forget them ([playback.md](playback.md)).
- **While stopped**, it all happens at once: the row's clips become the launches, as if started on the bar line the position is in, and the mutes are set. Its program changes have no line and go out where the music starts.
- **Stop all** (`Perform::stop_all`) cues a stop on every track that plays or has a cue, and forgets the scene.

## Follow actions (`Perform::due`)

The host calls `due` after every period while the transport rolls, with the next tick to play.

- **A follow comes due `bars` bars after its start.** For a scene, that is the bar line where its cues started. For a clip, it is its launch's start (`Launch::start`). `due` queues the follow's cue in the bar before that point, so it starts exactly on it. A late check, such as after a locate past the point, starts it on the next bar line instead.
- **A scene's follow goes first.** It cues every track. A clip follow on a track that already has a cue waits, so whatever is cued wins: the user's launch, a stop or a scene.
- **Stay** keeps playing and rolls again `bars` bars later. A follow with a chance between Next and Stay therefore plays on for a random number of passes. **Again** starts the same clip or scene again, which gives it a new start to count from.
- The rolls come from a small xorshift generator seeded at `Perform::new`, so a test gets the same rolls every time.
- **The song list** (S4's sixth part, [make-perform.md](make-perform.md#the-song-list-as-built-the-sixth-part)) goes before them: while it plays, `due` cues each step's scene in the bar before the step before it ends (its scene's length times its times), on that bar line, and the scenes' own follow actions wait.

## On the Clips page (`mpfree-time-ui`)

- **The row numbers are scene buttons.** Each one shows a play mark over its number, beside its row. Tapping it sends `TimeAction::LaunchScene(row)`. The playing scene's button is lit green, and a scene waiting for its bar line has an amber ring.
- **The square in the corner** over the scene buttons is Stop all (`TimeAction::StopAll`).
- The column is 48 pixels wide, up from the 26 the row numbers took, so the buttons are big enough to hit.
- **The scene editor** (S4's seventh part, [make-perform.md](make-perform.md#the-scene-editor-as-built-the-seventh-part)): the Record panel's Scene tab sets a scene's name, follow action and mutes (`SceneSettings`, one take a row through `Takes::set_scene`).

## In the pad player (`TimeHost`)

- `act(LaunchScene)` and `act(StopAll)` go through `Perform`, and compile.
- `act(Uncue)` takes the track's scene mute back too.
- Every compile calls `Perform::dress`.
- Each started cue goes through `Perform::started` before `Started::apply`.
- After every rolling period, `Perform::due` queues the follow actions coming due.
- `TimeState::scene_playing` and `scene_cued` light the buttons.

## The scene morph (T10, `mpfree_perform::morph`)

A crossfader morphs every parameter two rows store, from one row's values to the other's, and lets go either back to the sound from before or where it is.

The mod engine (`mpfree-mod`) already has scenes A and B and a morph position (`Owner::Morph(0)`). It glides every parameter both scenes hold, in knob positions, between the base layer and the macros. `SceneMorph` fills A and B from two rows:

- **`load(engine, project, a, b)`** puts every parameter either row holds on both sides. A parameter only one row holds gets its value now on the other side, so it moves from where it is and nothing jumps. Values for parameters the engine does not have are counted and skipped.
- **The snapshot.** The values the parameters have when a morph starts are kept. Loading other rows while morphing keeps that first snapshot.
- **`snap_back(engine)`** lets go: every held parameter goes back to the snapshot, and the morph to 0. It returns the values for the host to send as `ModEvent::Set`. That covers knobs turned while morphing too: a turn moves the parameter's own value as well as the nearer scene.
- **`keep(engine)`** lets go with every parameter where the morph has it. That is the value last sent, so an LFO on the parameter is caught where it was.
- **`store(engine, project)`** writes A and B back into their rows. A knob turned while morphing edits the scene the morph is nearer, and this keeps the edit in the project.
- **`save_scene(engine, project, row)`** stores every parameter's own value now, and every macro's position, in a row.

On the Clips page, the morph bar sits under the grid:

- **The A and B chips.** A tap on a chip's left half steps its row down, and on its right half steps it up.
- **The crossfader.** Moving it loads the two rows if the morph is off.
- **Snap back and Keep.** These answer only while a morph is loaded.
- **Save A or Save B.** This saves the sound into the row the crossfader is nearer.

The grid shows 5 rows at a time over the bar, down from 6.

**In the pad player** the morph runs on the mixer's mod engine (`ModMix` in `modmix.rs`), the same one that holds the clips' parameter locks. The Clips page's morph actions go to `TimeHost::morph`, which the main loop calls with the mixer's `ModMix` (`time_action` in `main.rs`):

- **Loading** (`ModMix::morph_load`) makes every mixer parameter either row stores a target first, with the value the mixer has now, so a row can move a strip's level, pan or sends, or an insert's controls. A row's values for a pad's sound or an instrument are counted and left out (as a lock on one is unrouted), and the log says how many.
- **Other rows while morphing** go on A and B at once: turns made so far are written back into the old rows first, and Snap back still goes to the sound from before the morph.
- **The crossfader** sends the morph position to the engine (`ModMix::morph_to`), which glides it as it does any other parameter.
- **Snap back and Keep** (`ModMix::morph_end`) write the turns made while morphing back into the rows, then send the values `snap_back` or `keep` return.
- **Save** stores the mixer as it is into the row (`ModMix::save_scene`).

The project keeps the rows' values (`Project::scenes`), so a saved morph comes back with the project.

## Tests

- `mpfree-perform` unit tests cover:
  - a scene cueing every track, and stopping a playing track whose cell is empty;
  - launching while stopped;
  - an idle track's mute, which waits for the scene's line (or a follow's bar line) for the host's mute queue, and is set at once with Launch at once or while stopped (`an_idle_tracks_mute_waits_for_the_scenes_line`);
  - a scene follow firing once, in the bar before its point;
  - a clip follow, and a cue waiting that wins over it;
  - Stay rolling again;
  - chances picking both ways;
  - Stop all;
  - a scene's cues taking the project's launch quantise, a follow's the bar, and a line kept when Launch changes (`scene_cues_take_the_projects_line_and_follows_the_bar`);
  - a launch's program changes: on the launch line, with none while stopped, a new id each launch, a follow's on the next bar line, forgotten by an uncue and Stop all (`a_scenes_program_changes_go_with_its_launch`);
  - the song list's steps in order, then the stop or the first again (`the_song_list_plays_its_steps_in_order_then_stops`).
- `mpfree-perform/tests/scenes.rs` plays scenes through the transport and the player at periods of 1, 64, 128, 1000 and 4096 frames. Every period size must give the same events. It checks that:
  - a scene's clips start together on the bar line and its mute silences a track from exactly that tick;
  - clip follow actions chain on their bar lines (Next after 1 bar, Next after 2, then First);
  - scene follow actions ping-pong between two rows and carry their mutes;
  - Stop all stops every track on the bar line;
  - a scene's program changes go out on its bar line, the clip's own where the clip starting there has one, and a track with no clip gets its own; after a stop they go out again as the music starts, a loop jump sends none, and a scene launched while stopped sends them as the music starts (`a_scenes_program_changes_go_out_on_its_bar_line`).
- `mpfree-app`'s `a_scene_plays_its_row_with_its_mutes_and_follows_on` runs the same through `TimeHost` with the pads, and `a_scenes_mute_on_a_track_playing_the_song_lands_on_its_line` checks, at periods of 1, 7, 64, 128, 1000 and 4096 frames and with the track soloed, that a scene's mute on a track playing the song in song mode lands on the scene's bar line, with its cue, not at the tap.
- `mpfree-time-ui` tests cover the scene buttons, Stop all, and redrawing the grid when a scene is cued. They also cover the morph bar's chips, crossfader and buttons, and redrawing only the bar when the crossfader moves.
- `mpfree-perform/tests/morph.rs` runs the real mod engine. It checks:
  - the morph at 0, halfway (a log cutoff halfway on its knob) and 1;
  - a parameter only one row holds moving from where it was;
  - snap back after a turn made while morphing;
  - Keep;
  - a reload keeping the first snapshot;
  - saving a row (3 parameters and 8 macros);
  - a turn written back into the nearer row;
  - values for parameters the engine lacks being counted.
- `mpfree-app`'s `the_scene_morph_moves_the_mixer_between_two_rows_and_snaps_back` (`modmix.rs`) morphs two strips' levels on the real mixer, one held by only one row, through A, B and halfway, then snaps back, keeps and saves a row. `the_morph_bar_runs_the_scene_morph_on_the_mixers_mod_engine` (`time.rs`) does the same through the morph bar's actions, rows changed while morphing included.

## Not yet

- A scene's colour has no editor yet (a row takes its clips' colours).
- Clip follow actions count song bars. A clip at another speed still counts the song's bars, as Ableton does.
- In a song loop shorter than a follow's time, the follow never comes due.
- Scenes into a song and back are built: see [arrange.md](arrange.md).
- A Q-Link for the crossfader, and the morph moving a pad's sound or an instrument (once those engines take parameter events).
