# Scenes and follow actions (T9)

A scene is a row of the launch grid. Launching it plays every track's clip in that row from the next bar line, so a whole part of the song changes at once. A scene can also switch the track mutes on that bar line, and a follow action can chain scenes and clips into a song without an arrangement.

The model is in `rust/mpfree-seq/src/scenes.rs`. `rust/mpfree-perform` carries it out on the control thread with the launches and cues the host already keeps (see [playback.md](playback.md#launching-on-the-bar-line-cues-t9)).

## The model (`mpfree-seq`)

- **Scene `k` is row `k`** of every track's slots, so there are 16 (`SCENES = SLOTS`). `Project::scenes` holds what a row has besides its clips. A row the project keeps nothing for is a plain scene: launching it still plays its clips. `Project::scene_mut(row)` makes the plain rows up to it when a row gets a name, mutes or a follow.
- **`Scene { name, mutes, follow, values }`**.
  - `mutes: Option<u16>`: bit `t` mutes track `t` while the scene plays. `None` leaves the mutes as they are.
  - `follow: Option<Follow>`: what happens after it has played some bars.
  - `values`: parameter values stored with the scene, for the scene morph (T10). Nothing reads them yet.
- **`Clip::follow`**: a clip can have a follow action of its own.
- **`Follow { bars, action, other, chance }`**: after `bars` bars (1 to 999, counted from where it started), `action` with a chance of `chance` %, and `other` the rest of the time. These are Ableton's A and B.
- **`FollowAction`**: Stay, Again, Next, Previous, First, Last, Any, Other, Stop.
  - For a clip, Next means the next filled slot down its track, wrapping round to the first after the last. Empty slots are skipped. For a scene, Next is the next row with a clip on any track.
  - Any picks any filled one, this one included. Other picks any but this one, and stays when there is no other.
  - Stop stops the clip's track, or every track for a scene.
  - `follow_target` works out where an action goes. It never allocates.
- `Scene::check` and `Follow::check` keep the values in range (`ModelError::BadScene`, `BadFollow`), and `Project::check` runs them.

## Launching a scene (`Perform::launch_scene`)

- **While the transport rolls**, every track gets a cue for the next bar line: its clip in the row, or a stop when its cell is empty and it is playing something. They all start on the same bar line. A track with nothing playing or cued and an empty cell gets no cue.
- **Mutes switch on the same bar line.** A cue carries its track's mute (`PlayView::set_cue_mute`). The player switches the mute when the cue starts: a muted track's notes end on the bar line and nothing after it sounds. `Started::mute` reports it, and `Perform::started` makes it the track's `mute`. Each compile drops the cue mutes, so `Perform::dress` sets them again on every new view. A track with no cue takes its mute at once. It is silent, so that sounds the same.
- **While stopped**, it all happens at once: the row's clips become the launches, as if started on the bar line the position is in, and the mutes are set.
- **Stop all** (`Perform::stop_all`) cues a stop on every track that plays or has a cue, and forgets the scene.

## Follow actions (`Perform::due`)

The host calls `due` after every period while the transport rolls, with the next tick to play.

- **A follow comes due `bars` bars after its start.** For a scene, that is the bar line where its cues started. For a clip, it is its launch's start (`Launch::start`). `due` queues the follow's cue in the bar before that point, so it starts exactly on it. A late check, such as after a locate past the point, starts it on the next bar line instead.
- **A scene's follow goes first.** It cues every track. A clip follow on a track that already has a cue waits, so whatever is cued wins: the user's launch, a stop or a scene.
- **Stay** keeps playing and rolls again `bars` bars later. A follow with a chance between Next and Stay therefore plays on for a random number of passes. **Again** starts the same clip or scene again, which gives it a new start to count from.
- The rolls come from a small xorshift generator seeded at `Perform::new`, so a test gets the same rolls every time.

## On the Clips page (`mpfree-time-ui`)

- **The row numbers are scene buttons.** Each one shows a play mark over its number, beside its row. Tapping it sends `TimeAction::LaunchScene(row)`. The playing scene's button is lit green, and a scene waiting for its bar line has an amber ring.
- **The square in the corner** over the scene buttons is Stop all (`TimeAction::StopAll`).
- The column is 48 pixels wide, up from the 26 the row numbers took, so the buttons are big enough to hit.

## In the pad player (`TimeHost`)

- `act(LaunchScene)` and `act(StopAll)` go through `Perform`, and compile.
- `act(Uncue)` takes the track's scene mute back too.
- Every compile calls `Perform::dress`.
- Each started cue goes through `Perform::started` before `Started::apply`.
- After every rolling period, `Perform::due` queues the follow actions coming due.
- `TimeState::scene_playing` and `scene_cued` light the buttons.

## Tests

- `mpfree-perform` unit tests cover:
  - a scene cueing every track, and stopping a playing track whose cell is empty;
  - launching while stopped;
  - an idle track's mute;
  - a scene follow firing once, in the bar before its point;
  - a clip follow, and a cue waiting that wins over it;
  - Stay rolling again;
  - chances picking both ways;
  - Stop all.
- `mpfree-perform/tests/scenes.rs` plays scenes through the transport and the player at periods of 1, 64, 128, 1000 and 4096 frames. Every period size must give the same events. It checks that:
  - a scene's clips start together on the bar line and its mute silences a track from exactly that tick;
  - clip follow actions chain on their bar lines (Next after 1 bar, Next after 2, then First);
  - scene follow actions ping-pong between two rows and carry their mutes;
  - Stop all stops every track on the bar line.
- `mpfree-app`'s `a_scene_plays_its_row_with_its_mutes_and_follows_on` runs the same through `TimeHost` with the pads.
- `mpfree-time-ui` tests cover the scene buttons, Stop all, and redrawing the grid when a scene is cued.

## Not yet

- Scene names and follow settings have no editor yet. They are set in the model only.
- A launch quantise setting (1 beat to 8 bars, or none) is still to come. Scenes and follows use the bar.
- Clip follow actions count song bars. A clip at another speed still counts the song's bars, as Ableton does.
- In a song loop shorter than a follow's time, the follow never comes due.
- The scene morph (T10) uses `Scene::values`.
