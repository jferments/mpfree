<!--
covers: rust/mpfree-app/src/smoke.rs rust/mpfree-app/src/smoke rust/mpfree-ui/src/widgets.rs rust/mpfree-ui/src/wave.rs
checked: 2026-11-13 4fe909a
-->

# Controls catalogue

Every control on MPFree's screen: what it is for, how to use it, and an example of it in use. **Status:** built, 2026-11-13 (the control audit, Jay's ask of 13 Nov).

A control is named by the key the smoke tests use, `widget/id`: the widget's name in the page layout, then the id its `targets()` gives the control. A control that repeats (the 16 pads, a list's rows, a strip on every track) has one entry for all of them. `‹` and `›` are the left and right halves of a setting's value: a tap steps it down or up.

**Test:** names the test that runs the example, where one does. Every control is also pressed by the sweep, `every_control_on_every_screen_does_something` in [`smoke.rs`](../../../rust/mpfree-app/src/smoke.rs), which fails on a control that does nothing, and the random walks in `smoke/walk.rs` press them in random orders ([Testing and CI](testing-and-ci.md#gui-smoke-tests)). `every_swept_control_is_in_the_catalogue` (in [`smoke/catalogue.rs`](../../../rust/mpfree-app/src/smoke/catalogue.rs)) fails when the sweep finds a key this page doesn't list: add the entry in the same pull request.

A control that can't do anything right now is drawn faint, and a tap on it says why on the hint line; a lit tab or a picked row is "already chosen", and a tap on it changes nothing. The sweep counts both as "off". For musicians, the [user manual's screen page](../../user/src/getting-started/the-screen.md) and [buttons page](../../user/src/reference/controls.md) cover the same ground in plain words.

## Everywhere: the top bar, the master bar, the Q-Links

- **`header/tab`**: the page tabs (Play, Pads, Arrange, Synth, Looper, Sample, Mix). Tap one to go to that page; the lit tab is the page you are on. Mix opens the mixer as you left it; TRACK MIX opens it on Track Mix and goes back again. *Example:* tap Arrange to see the clips, then Play to get back to the pads. *Test:* `the_mode_buttons_pick_play_views_and_sample_editors`.
- **`header/io-icon`**: opens the mixer's I/O view (inputs, outputs, MIDI, Sync, CV). *Example:* tap I/O, set the line input's monitor, tap I/O again to close. *Test:* `the_io_icon_opens_the_mixers_io_view_and_the_tempo_chip_opens_it_too`.
- **`header/chip`**: the chips at the right of the top bar. The kit chip ("Breaks · Bank A") opens the list of kits over the page; the Full or Half level chip turns that level off; the tempo chip, while another device's clock sets the tempo, opens the I/O view at Sync; the arp chip opens the arp panel. The CPU, RAM, session, time-left and 16 Levels chips only show: a tap says on the hint line what the chip shows. *Example:* press FULL LEVEL, play a pad softly (it plays at full velocity), tap the Full level chip: the pads follow your hits again. *Tests:* `the_kit_chip_opens_a_list_of_kits_over_the_page`, `a_tap_on_the_level_chip_turns_that_level_off`, `the_quick_band_comes_with_note_repeat_and_the_chip_opens_the_panel`.
- **`power/button`**: the buttons of the power card a short press of the power button opens over any page (power.rs): Cancel, Restart and Shut down; over unsaved work Cancel, Don't save and Save and shut down (or restart); OK when it can't shut down from here. A tap chooses, as the wheel's push does on the outlined one. *Example:* press the power button briefly, tap Cancel: back on the page. *Tests:* `a_short_press_of_the_power_button_opens_the_power_card`, `shut_down_and_restart_go_to_the_systems_file`.
- **`kits/row`**: a kit in the list the kit chip opens; the synth is the last row. Tap one to load it on the pads; the list closes and the page comes back. *Example:* tap the kit chip, tap Boom Bap: the pads play Boom Bap and the chip says so. *Test:* `the_kit_chip_opens_a_list_of_kits_over_the_page`.
- **`kits-card/button`**: Back on the kit list's card: closes the list without changing the kit (the kit chip and a tab close it too). *Test:* `the_kit_chip_opens_a_list_of_kits_over_the_page`.
- **`master/level`**: the master level bar on the Play page; drag it for the level of everything, the same level as the mixer's master fader. *Example:* drag the bar down to -6 dB, open the mixer: its master fader is at -6 dB too. *Test:* `the_master_bar_and_the_mixer_master_are_one_level`.
- **`qlinks/knob-card`**: the four Q-Link cards in the right-hand column. Each shows what its knob turns on this page; drag a card to turn it from the screen. *Example:* on Play's Beats view, tap a step, then drag the Velocity card: the step plays softer. *Tests:* `one_tap_switches_the_play_view_and_the_q_links_follow_it`, `beats_picks_a_step_for_the_q_links`.
- **`guide/button`**: Skip and Done in the button guide (it walks through the hardware buttons the first time, and from Prefs). Skip moves on to the next button, leaving this one unlearned; Done ends the guide. *Example:* the guide asks for REC and you want it later: tap Skip.

## Play page

- **`play-transport/view`**: the view switch (All, Beats, Loop, Drum): what the left of the Play page shows. Tap one; the Q-Links follow the view. *Test:* `one_tap_switches_the_play_view_and_the_q_links_follow_it`.
- **`play-transport/tempo`**: the tempo. Tap it in time, four times or more, to set the tempo (as TAP TEMPO). *Example:* tap along with a record at 92 BPM: the tempo reads 92. *Tests:* `a_tap_on_plays_tempo_taps_it`, `the_tempo_tapped_on_play_sets_the_transport`.
- **`steps/step`**: one step of a drum lane (Beats and All views). Tap to turn it on or off, and to pick it for the Q-Links (velocity, length, nudge, chance). *Example:* tap Kick steps 1, 5, 9 and 13 and press PLAY: four on the floor. *Tests:* `steps_tapped_on_beats_play_in_time`, `beats_picks_a_step_for_the_q_links`.
- **`steps/mute`**: a lane's speaker: mutes and unmutes that pad's lane. It is the same mute as the pad's M in the pad mixer. *Test:* `a_speaker_on_play_and_m_in_the_pad_mixer_are_one_mute`.
- **`steps/head`**: the pattern's head: ‹ and › turn its pages of 16 steps, - halves the steps (down to 16) and + doubles them (up to 128, the new steps repeating the old). Each length change is one take, so UNDO brings the steps back. Faint when it can't act; a tap then says why. *Test:* `the_patterns_head_turns_pages_and_doubles_or_halves_its_steps`.
- **`throws/throw`**: the performance throws (Delay throw, Reverb wash, Stutter, Tape stop). Hold one: the effect plays while you hold it and lets go when you lift. *Example:* hold Tape stop on the last beat of a bar for a slow-down into the next. *Test:* `the_pattern_throws_and_xy_pad_reach_the_engines`.
- **`xy/xy`**: the XY pad (a DJ filter with an echo). Drag across it: across sweeps the filter, up adds echo. *Test:* `the_pattern_throws_and_xy_pad_reach_the_engines`.
- **`play-loop/button`**: the Loop view's Record, Overdub, Play and Stop for the looper. *Example:* tap Record, play four bars on the pads, tap Overdub to layer on top. *Test:* `the_loop_strips_buttons_carry_their_symbols_and_play_and_stop_the_transport`.
- **`play-loop/layer-mute`**: a loop layer's speaker on the Loop view: mutes that layer. *Test:* `a_loop_layers_speaker_mutes_it`.
- **`levels-panel/button`**: the 16 Levels panel's Pick pad, Close and Off (16 LEVEL opens it on Play). Pick pad lights: hit the pad the levels play. Close hides the panel and leaves 16 Levels on; Off turns them off. *Example:* press 16 LEVEL, tap Pick pad, hit pad 3: all 16 pads play pad 3. *Tests:* `sixteen_level_turns_levels_on_with_the_panel_and_off_again`, `sixteen_levels_make_every_pad_play_the_pad_and_hits_pick_it`.
- **`levels-panel/kind`**: what the 16 levels change (velocity, tune, cutoff, decay, start); the wheel steps them too, and the lit one is chosen. *Example:* tap Tune: pad 1 plays 8 semitones down, pad 9 as recorded, pad 16 7 up. *Test:* `sixteen_level_turns_levels_on_with_the_panel_and_off_again`.
- **`arp-mini/open`**, **`arp-mini/slower`**, **`arp-mini/faster`**, **`arp-mini/latch`**: the quick band that comes with NOTE REPEAT: the mode chip opens the arp panel, ‹ slower and faster › change the rate, Latch keeps the notes going after you lift. *Example:* hold NOTE REPEAT, hold a pad, tap faster ›: 1/16 becomes 1/32. *Test:* `the_quick_band_comes_with_note_repeat_and_the_chip_opens_the_panel`.

### The arp panel and workspace

- **`arp-modes/segment`**: the mode (Direct, Repeat, Arp, Pattern). Tap one; the panel shows that mode's settings.
- **`arp-list/row`**: a setting of the mode (the arp's direction, octaves, the pattern). Tap to pick it; the wheel and - / + change it.
- **`arp-qlinks/slider`**: Rate, Gate, Swing and Octaves, as sliders (the Q-Links turn the same). Drag one.
- **`arp-steps/step`**: the steps of a pattern. In Pattern, tap a step to edit it; in the other modes they show the notes as they play, and a tap says to pick Pattern.
- **`arp-ws-grid/cell`**: a cell of the pattern workspace: tap to put a note on the step or take it off. *Test:* `the_workspace_edits_by_touch_and_shows_what_the_pads_write`.
- **`arp-ws-tools/button`**: the workspace's Back, Step time, Free time and Undo. *Test:* `the_workspace_edits_by_touch_and_shows_what_the_pads_write`.
- **`pads/pad`**: the workspace's pads (1st note, 2nd note, ...): tap one to write that note into the step. *Test:* `the_workspace_edits_by_touch_and_shows_what_the_pads_write`.

## Pads tab

- **`smp-pads-bar/tab`**: Browse and Pad: the browser for sounds, or the picked pad's settings.
- **`smp-pads-map/pad`**: the 16 pads of the bank. Tap a pad to pick it; tap it again to hear it. *Example:* tap A5, look at its Sound and Mode rows, tap A5 again to hear it. *Test:* `the_pads_tab_picks_copies_and_clears_pads_and_undo_takes_it_back`.
- **`smp-pads-buttons/button`**: Copy, Paste, Clear and Map to keys. Copy the picked pad, pick another and Paste it there; Clear empties the pad; Map to keys makes the bank one instrument with each pad on its note. Faint buttons say why. *Tests:* `the_pads_tab_picks_copies_and_clears_pads_and_undo_takes_it_back`, `map_to_keys_makes_the_bank_one_instrument_and_the_play_tab_plays_its_keys`.
- **`smp-pads-list/row`**, **`smp-pads-list/row-less`**, **`smp-pads-list/row-more`**: the pad's settings (Sound, Mode, Polyphony, Mute group, Level, Pan, Tune). Tap a row to pick it; ‹ and › step it. A setting at its end says so; Sound on the kit's only sound says to load more. *Example:* pick the hi-hat pad, step Mute group to 1, do the same on the open hat: the closed hat cuts the open one.
- **`smp-pads-list/row-button`**: the Effects row's button: "+ FX", or the effects on the pad's mixer channel. Tap it for the pad's chain in the Mix tab (the picker on an empty chain); ‹ comes back. Pads past the first 16 have no channel of their own and show "—". *Test:* `the_pads_tab_effects_row_opens_the_pads_chain_in_mix`.
- **`smp-pads-sounds/row`**: the kit's sounds. Tap one to choose it for Put.
- **`smp-pads-put/button`**: Put on (pad): puts the chosen sound on the picked pad, whole. If the pad already plays all of it, it says so.
- **`browser/place`**: Samples, Recordings, Recent, Favourites: where the list looks.
- **`browser/row`**: a file or folder. Tap to pick it (a folder opens).
- **`browser/button`**: Load (onto the pad shown), Load to pad (onto the next pad you hit), Audition, Favourite. With nothing picked, a button says to pick something first. *Example:* tap Kick 01.wav, Audition to hear it, Load to pad, hit pad A1. *Test:* `load_puts_a_sound_on_the_pad_shown_and_load_to_pad_on_the_next_hit`.
- **`browser/back`**: up a folder.
- **`browser/search`**, **`browser/sort`**: search by name (an on-screen keyboard opens), and the sort order.

## Sample tab

The Sample tab edits the pad hit last, with an editor per tab (Wave, Warp, Chop, Play, Grain, Rec, Auto, FX). Some controls are the same on every editor:

- **`smp-bar/tab`**, **`smp-warp-bar/tab`**, **`smp-chop-bar/tab`**, **`smp-play-bar/tab`**, **`smp-grain-bar/tab`**, **`smp-rec-bar/tab`**, **`smp-auto-bar/tab`**: the editor tabs. Tap one to switch editor; FX opens the chain editor on the pad's own channel. *Tests:* `the_sample_page_edits_the_pad_hit_last_and_sends_one_view_at_a_time`, `the_fx_tab_asks_for_the_chain_editor_on_the_pads_channel`.
- **`smp-q1/card`**, **`smp-q2/card`**, **`smp-q3/card`**, **`smp-q4/card`**, **`smp-chop-q1/card`**, **`smp-chop-q2/card`**, **`smp-chop-q3/card`**, **`smp-chop-q4/card`**, **`smp-warp-q1/card`**, **`smp-warp-q2/card`**, **`smp-warp-q3/card`**, **`smp-warp-q4/card`**, **`smp-play-q1/card`**, **`smp-play-q2/card`**, **`smp-play-q3/card`**, **`smp-play-q4/card`**, **`smp-grain-q1/card`**, **`smp-grain-q2/card`**, **`smp-grain-q3/card`**, **`smp-grain-q4/card`**: the Q-Link cards of each editor. Tap a card to give the wheel to it; turn its knob on the MPC to change it. A faint card says why (no loop, the filter off, no pin).
- **`smp-send/button`**: Send to pad (a copy of the pad onto the next pad you hit) and Chop to pads (opens the Chop tab). *Test:* `send_to_pad_copies_the_pad_onto_the_next_hit_and_chop_to_pads_opens_the_chop_tab`.

### Wave

- **`smp-wave/wave`**: the waveform. Tap to play from there (a looping pad plays until the next tap); drag to scroll; pinch to zoom. *Test:* `a_tap_on_the_wave_plays_the_pad_and_a_looping_one_until_the_next_tap`.
- **`smp-wave/marker`**: the Start, End and loop markers. Drag a marker by its tab. *Example:* drag Start past the silence at the front of a sample.
- **`smp-overview/overview`**: the whole sound in a strip, with a box round what the waveform shows. Tap to move the box there; faint when the whole sound is in view.
- **`smp-region/row`**, **`smp-region/row-less`**, **`smp-region/row-more`**: the region's settings (Mode, Loop, Crossfade, Gain, ...). Tap a row, then ‹ › or the wheel.

### Warp

- **`smp-warp-view/wave`**: the warped wave; tap a hit to pin it to the grid. *Test:* `the_warp_tab_pins_the_hits_and_the_copy_goes_through_them`.
- **`smp-warp-view/pin`**: a pin: tap to pick it, drag its handle along the beats.
- **`smp-warp-buttons/button`**: Pin hits (all of them), Remove pin, Clear pins. Faint ones say why.
- **`smp-warp-list/row`**, **`smp-warp-list/row-less`**, **`smp-warp-list/row-more`**: Warp on or off, the warp tempo, the grid, the hits' sensitivity. *Test:* `a_warped_pad_follows_the_tempo_and_plays_its_stretched_copy_when_ready`.

### Chop

- **`smp-chop-wave/wave`**: tap a slice to pick and hear it. *Test:* `the_chop_tab_puts_slices_on_the_kits_pads_and_undo_takes_them_back`.
- **`smp-chop-wave/marker`**: a cut between slices; drag its tab to move the cut. The first marker and END are the pad's Start and End: they stay put, and a touch says to move them on the Wave tab.
- **`smp-chop-overview/overview`**: as the Wave tab's overview.
- **`smp-chop-buttons/button`**: Split, Merge, Chop to pads, To keys, Live chop. *Example:* Method Transient, Split at a missed hit, Chop to A2–A16, hit the pads. *Tests:* `the_chop_tab_puts_slices_on_the_kits_pads_and_undo_takes_them_back`, `to_keys_puts_slices_on_notes_and_the_play_tab_turns_the_pads_back`, `live_chop_drops_cuts_where_the_sampler_marked_them_and_ends_with_the_sound`, `pads_and_phrase_sends_a_clip_that_lands_in_the_armed_tracks_first_empty_slot`.
- **`smp-chop-list/row`**, **`smp-chop-list/row-less`**, **`smp-chop-list/row-more`**: Method, Sensitivity, Snap, Slice, Put and the other chop settings.

### Play

- **`smp-play-list/row`**, **`smp-play-list/row-less`**, **`smp-play-list/row-more`**: how the pad plays: pressure, envelope, filter, keys, latch, also plays, 16 Levels. *Tests:* `the_play_tab_changes_how_the_pad_plays_and_undo_takes_it_back`, `hits_on_pads_edit_stops_and_also_plays_and_latch_is_a_row`, `sixteen_levels_make_every_pad_play_the_pad_and_hits_pick_it`.
- **`smp-play-env/point`**: the envelope's points (Attack, Hold, Decay, Sustain, Release): drag one.
- **`smp-play-filter/cutoff`**: the filter's dot: drag for cutoff and resonance. Faint while the filter is off.
- **`smp-play-layers/layer`**, **`smp-play-layers/layer-low`**, **`smp-play-layers/layer-high`**: the pad's velocity layers: tap one to pick it, drag its edges for the velocities it plays. *Test:* `key_fade_is_the_kits_and_vel_fade_the_pads_each_one_undo_step`.
- **`smp-play-keys/keys`**: the keyboard: press keys to hear the kit's keygroups. *Test:* `map_to_keys_makes_the_bank_one_instrument_and_the_play_tab_plays_its_keys`.
- **`smp-play-keys/zone`**, **`smp-play-keys/zone-low`**, **`smp-play-keys/zone-high`**, **`smp-play-keys/zone-root`**: a pad's zone of keys: tap to pick its pad, drag its edges and root dot.
- **`smp-play-scenes/button`**: Store A, Store B, Return to A: two snapshots of the kit to morph between. *Test:* `scenes_morph_the_playing_kit_and_return_puts_the_pads_back`.
- **`smp-play-fader/slider`**: Scenes · A to B: drag to morph from scene A to B. *Test:* `scenes_morph_the_playing_kit_and_return_puts_the_pads_back`.

### Grain

- **`smp-grain-cloud/position`**: drag along the sound for where the grains play from. *Test:* `the_grain_tab_sets_the_grains_and_draws_the_cloud_the_audio_thread_fills`.
- **`smp-grain-xy/xy`**: drag for position (across) and grain size (up).
- **`smp-grain-list/row`**, **`smp-grain-list/row-less`**, **`smp-grain-list/row-more`**: Mode, Scan speed, Spray, Pitch spray and the other grain settings.

### Rec and Auto

- **`smp-rec-buttons/button`**: Record or Stop, Skip back 10 s (keep what just played), Flatten (record the pad as it plays), Auto-sample. *Tests:* `the_rec_tab_records_the_line_input_onto_a_pad_and_frees_the_buffer_when_it_goes_off`, `flatten_records_the_pads_own_channel_as_it_plays_once_and_keeps_it_on_the_next_pad`.
- **`smp-rec-list/row`**, **`smp-rec-list/row-less`**, **`smp-rec-list/row-more`**: Source, Channels, Start, Threshold and the rest. *Test:* `the_rec_tab_resamples_the_master_and_lets_go_of_it_for_the_line_input`.
- **`smp-auto-buttons/button`**: Start and Rec for auto-sampling. *Tests:* `auto_sample_plays_external_gear_over_midi_and_records_it_from_the_line_input`, `auto_sample_plays_a_copy_of_the_synth_onto_empty_pads_and_undo_takes_it_off`.
- **`smp-auto-list/row`**, **`smp-auto-list/row-less`**, **`smp-auto-list/row-more`**: Source, Lowest note, Highest note, Every and the rest.

## Arrange page

- **`clips/slot`**: a clip slot. Tap a clip: it plays from the next bar, or stops (on the lift); + on an empty slot records into it (on the touch). A clip dragged more than 12 px drops on the slot under the lift (`EventKind::Drop`, `TimeAction::DropClip`): it moves, swaps with a clip there, or with SHIFT is copied. *Test:* `the_clips_page_shows_the_engine_and_turns_taps_into_actions`, `clips_and_rows_drag_and_drop_and_a_still_finger_taps`.
- **`clips/scene`**: a row's number: plays the whole row (a scene), on the lift. Dragged, it moves the row to the gap under the lift (`TimeAction::DropRow`), or with SHIFT copies it. *Test:* `the_clips_page_shows_the_engine_and_turns_taps_into_actions`, `clips_and_rows_drag_and_drop_and_a_still_finger_taps`.
- **`clips/track`**: a track's name: arms it for recording.
- **`clips/stop-all`**, **`clips/edit`**, **`clips/song`**, **`clips/rec-song`**: Stop all clips; Edit (the note editor on the picked clip); Song (the Song view); Rec song (record what the grid plays into the song).
- **`clips/add-track`**: + Track: adds a track after the last one, named "Track 9" and so on and armed; UNDO takes it away. Faint at 128 tracks.
- **`clips/pager`**: ‹ and ›, shown once there are more than 8 tracks: the 8 tracks before or after (a swipe along the names does the same). Faint at either end.
- **`transport/rec`**, **`transport/mode`**, **`transport/play`**, **`transport/stop`**, **`transport/undo`**: the Arrange page's transport: Rec, the record mode (replace or overdub), Play, Stop, Undo of the last take.
- **`record/row`**, **`record/row-less`**, **`record/row-more`**: the record settings (Armed, Mode, Count-in, Loop, Tempo, Pressure). A setting at its end, or one that needs an armed track when none is, says why.
- **`morph/crossfader`**: drag between scene A and B (the morph). **`morph/row-down`**, **`morph/row-up`**: step the A and B scenes. **`morph/button`**: Snap back and Keep (faint unless morphing), Save.
- **`takes/button`**: Undo, Redo and Capture for takes; faint when there is nothing to undo, redo or capture, and a tap says so.
- **`song/block`**: a block of the song: tap to pick it (as the finger lifts); drag it along its lane (to the bar) or onto another track's to move it, SHIFT held as it lifts for a copy; picked, the wheel's push grabs it and a turn moves it a beat. **`song/block-end`**: the picked block's end grip: drag it to make the block longer or shorter (to the bar). **`song/section`**: a section's name on the ruler: tap to pick it (the wheel's push then grabs it and a turn moves it a section at a time); drag it to move the section on every track, the others closing up, SHIFT for a copy. UNDO takes back any of these. **`song/button`**: Clips, Play song, Scenes → song, Song → scenes (the last two need a pick and say so). **`song/rec-song`**: Rec song.
- **`song/add-track`**: + Track, under the last track's lane while the project has room: adds a track, as on the grid.
- **`song/add-audio`**, **`song/arm`**, **`song/audio-header`**, **`song/region`**, **`song/region-button`**: audio tracks in the song: add one, arm it, open its panel (a tap on its name or input chip), pick a region, and Split, Erase, Fades, Gain and Takes on the region.
- **`song/panel-close`**, **`song/panel-arm`**, **`song/panel-mute`**, **`song/panel-solo`**, **`song/panel-takes`**: the audio track's panel over the lanes: Close, arm the track to record, mute, solo, and its takes. A block under the panel can be tapped only where it shows.
- **`song/panel-input`**, **`song/panel-monitor`**: what the track records from (L, R, L+R, ST) and when you hear its input (Auto, In, Off); the lit one is chosen.
- **`song/panel-level-less`**, **`song/panel-level-more`**, **`song/panel-pan-less`**, **`song/panel-pan-more`**: the track's level and pan, a step at a time (Q-Links 1 and 2 turn them too). *Example:* open Guitar's panel, tap ST, tap Level − twice: the guitar records in stereo at −3 dB.
- **`notes/button`**: the note editor's buttons (‹ Clips, the lens: grid, list or tracker, the tool: select, draw or erase, Snap).
- **`notes/note`**, **`notes/roll`**, **`notes/velocity`**: a note (tap to pick, drag to move), the empty roll (draw a note with Draw), the velocity lane (drag).
- **`notes/list-row`**, **`notes/tracker-row`**: a row of the list or tracker: tap to pick its note.
- **`edit-options/row`**, **`edit-options/row-less`**, **`edit-options/row-more`**: Timing Correct's options (Correct, Division, Swing, Window) and the other edits.

## Synth tab

- **`synth-title/tab`**: the synth's pages (Osc, Filter, Env, LFO, ...). The lit one is showing. *Test:* `the_synth_page_shows_poly_and_sends_its_changes`.
- **`synth-tiles/knob`**: a knob in a section (a level, a time, a cutoff): drag up or down to turn it (320 px is the whole range); tap it, then the wheel. The big ones matter most on the page. *Test:* `dragging_a_knob_up_turns_it_up_on_the_node`.
- **`synth-tiles/fader`**, **`synth-tiles/drawbar`**, **`synth-tiles/slider`**: an envelope's or mixer's fader (drag up), an organ drawbar (drag down to add it, as on an organ), a routing row's slider (drag sideways). Each drags over its own track. *Test:* `a_fader_drags_over_its_own_track`.
- **`synth-tiles/choice`**, **`synth-tiles/choice-less`**, **`synth-tiles/choice-more`**: a choice (a wave, a mode): its buttons when they fit (tap one), else a chip with ‹ ›. **`synth-tiles/step-less`**, **`synth-tiles/step-more`**: a whole number (voices, octave, coarse) one step down or up. **`synth-tiles/toggle`**: an on or off pill. *Test:* `choices_and_switches_change_on_a_tap`, `a_stepper_steps_by_one_and_stops_at_its_ends`.
- **`synth-tiles/section-link`**: a section's name with "Open ›": tap to go to the page that section is about (the filter envelope's knob on Filter opens Envelopes). *Test:* `a_section_opens_the_page_it_links_to`.
- **`synth-tiles/node`**: a box in a section's drawing (an FM operator, a drum pad): tap to go to its page. *Test:* `fm_draws_its_algorithm_and_a_tap_on_a_box_opens_that_operator`.
- **`synth-tiles/section-switch`**: an on or off switch in a section's heading (effect pages: a stage of the channel strip); while it is off the section's controls are dim.
- **`synth-presets/row`**: a preset: tap to load it.
- **`synth-pads-head/layout`**: Chromatic or In key for the pads. *Test:* `the_pads_heading_switches_the_synth_to_in_key`.
- The Q-Link column on the Synth tab is the shared one (**`qlinks/knob-card`**, under Play tab): it names what the page's Q-Links turn, as their sections name them (Q1 to Q4 badges on the page); drag a card to turn it.

## Looper tab

- **`lloops/loop`**, **`lloops/new-loop`**: pick a loop; start a new one. *Test:* `the_looper_tab_picks_loops_and_works_layers`.
- **`lloops/loop-mute`**, **`lloops/clear`**: mute a loop; clear the picked loop.
- **`leditor/layer`**, **`leditor/layer-mute`**, **`leditor/layer-solo`**, **`leditor/layer-delete`**, **`leditor/layer-level`**: a layer of the loop: pick it (on the lift), mute, solo, delete, drag its level. A finger that moves more than 12 px on a layer's lane picks the layer up as a card: the loops that can take it and + New loop are lit, and it drops on the one under the lift (`LoopAct::DropLayer`, `FromUi::LoopCopy`): it moves there, or with SHIFT held as it lifts a copy goes; onto + New loop it becomes a loop of its own. A loop of another length, or a busy one, says why (`LoopAct::NoDrop`). On the picked layer the wheel's push picks it up, a turn points it at the next loop and a push drops it (`Target::moves`; Jay, 14 Nov 12:03Z). *Tests:* `the_looper_tab_picks_loops_and_works_layers`, `a_layer_drags_out_to_a_loop_and_a_tap_still_picks_it`, `a_layer_dragged_onto_a_loop_moves_or_copies_and_another_length_says_why`.
- **`leditor/tool`**: the loop's tools (nudge, ½ length, × 2, reverse, seam fade, merge layers). Faint tools say why. *Test:* `faint_looper_tools_say_why`.
- **`leditor/trim-start`**, **`leditor/trim-end`**: drag to trim the loop.
- **`leditor/history-step`**: a step of the loop's history: tap to go back (or forward) to it.
- **`lqlinks/knob-card`**: the looper's Q-Link cards (Level, Pan, Fade old, Seam fade).
- **`lrecording/setting-less`**, **`lrecording/setting-more`**: the recording settings (Record from, First pass, After recording, Line input).
- **`luse/use`**: To a pad, To a clip, Save loop, Resample: what to do with the loop.

## The mixer, its I/O view and the effects

- **`mixer/view`**: Tracks, Pads, Inputs, Submixes, I/O. *Test:* `the_mixer_strip_controls_set_the_mixer`.
- **`mixer/prev`**, **`mixer/next`**: the page arrows: more strips, or more Audio rows on the I/O view, faint at either end. In a MIDI, Sync or CV section they move the selection a page, and say so at the first or last row.
- **`mixer/fader`**, **`mixer/pan`**, **`mixer/send`**, **`mixer/send-pre`**, **`mixer/value`**: a strip's fader, pan, sends A–D (and pre or post fader), and its level (a tap sets 0 dB). *Test:* `the_mixer_strip_controls_set_the_mixer`.
- **`mixer/mute`**, **`mixer/solo`**, **`mixer/output`**: M, S, and the output (a tap opens the picker). *Test:* `the_mixer_strip_controls_set_the_mixer`.
- **`mixer/strip-name`**: picks the strip for the Q-Links; a second tap on the picked strip opens its menu (Freeze or Thaw on a track or pad strip, Print on any strip). *Example:* tap Drums to turn its fader with the Q-Links, tap Drums again for its menu. *Test:* `a_strips_menu_freezes_the_playing_loop_and_thaws_it`.
- **`mixer/insert`**, **`mixer/add-insert`**: an insert (tap to open it in the chain editor) and + Insert. *Tests:* `an_insert_added_on_the_mixer_is_turned_bypassed_moved_and_removed`, `the_effect_pickers_cancel_adds_nothing`.
- **`mixer/look`**: Faders or Effects, the Mix tab's two looks of the same strips. *Test:* `the_effects_look_adds_picks_turns_and_moves_an_effect`.
- **`mixer/fx-name`**, **`mixer/fx`**, **`mixer/fx-add`**: on Effects, a strip's row: its name (picks the strip, as `mixer/strip-name`), an effect (a tap picks it for the Q-Links, again opens its chain; a drag along the row moves it) and + FX in its first free slot. *Test:* `the_effects_look_adds_picks_turns_and_moves_an_effect`.
- **`mixer/io-section`**: Audio, MIDI, Sync, CV in the I/O view. *Test:* `the_io_view_follows_its_section_and_says_why_a_setting_stays`.
- **`mixer/io-name`**, **`mixer/io-output`**, **`mixer/io-source`**, **`mixer/io-monitor`**: an Audio row: pick it, its output, an input's source and monitor.
- **`chain/slot`**, **`chain/slot-on`**: an effect in the chain: open its page, turn it on or off (bypass). A finger that moves more than 12 px on `chain/slot` drags the effect along the chain and drops it where it lifts; on the picked effect the wheel's push picks it up, a turn moves it a place a detent and a push puts it down (`Target::moves`; Jay, 14 Nov 12:03Z). *Test:* `an_insert_added_on_the_mixer_is_turned_bypassed_moved_and_removed`, `an_effect_drags_along_its_chain`, `the_wheel_moves_the_picked_effect_along_its_chain`.
- **`chain/knob`**, **`chain/fader`**, **`chain/slider`**: a control in a section of the effect's page (the Synth tab's section widget, effect page redesign): drag a knob or a fader up (320 px is a knob's whole range), a slider in a section of rows sideways; touching one gives it to the Modulate row; tap it, then the wheel. *Test:* `a_knob_given_to_a_macro_moves_with_the_macro`, `chain_touches_become_actions` (mpfree-chain-ui).
- **`chain/choice`**, **`chain/choice-less`**, **`chain/choice-more`**, **`chain/step-less`**, **`chain/step-more`**, **`chain/toggle`**: a choice in a section (a filter's type, a delay's sync): its buttons when they fit, else a chip with ‹ ›; a whole number one step down or up; an on or off pill. Each sets the value on the lift. *Test:* `the_data_wheel_turns_a_knob_as_a_finger_would` (mpfree-chain-ui).
- **`chain/section-switch`**, **`chain/section-link`**: a stage's switch in its section's heading (the Channel strip's Gate, Compressor and EQ: while it is off the section is dim), and "Open ›" by a section's name, which shows the page it is about (the stage's own page). *Test:* `the_channel_strip_keeps_its_stages_as_sections_and_the_qlinks_on_its_four` (mpfree-chain-ui).
- **`chain/group`**, **`chain/group-prev`**, **`chain/group-next`**: the page tabs in the header, for an effect with more than one page (the Channel strip: Main, Gate, Compressor, EQ; Perform FX; a plugin's groups); the lit tab is unavailable. When they don't all fit, the arrows each end step to the page before and after, and only the tabs in view are `chain/group` targets. *Test:* `a_long_tab_row_scrolls_with_the_page_shown` (mpfree-chain-ui).
- **`chain/mode`**, **`chain/mode-prev`**, **`chain/mode-next`**, **`chain/mode-list`**: the Mode chooser in the header of an effect with modes (Reverb, Delay, Drive, Filter, Tremolo): a button per mode (`mode`; the lit mode is unavailable), or, when they don't fit (Drive's ten), ‹ and › to the mode before and after (`mode-prev`, `mode-next`; the first and last mode have no arrow out) round a chip that opens the list of them (`mode-list`). While the list is open it and its chip are the only targets. *Tests:* `the_mode_chooser_switches_an_effect_with_modes`, `the_mode_chip_opens_the_list_of_modes` (mpfree-chain-ui), `a_mode_across_engines_swaps_in_place_and_switching_back_restores`, `a_mode_of_the_same_engine_sets_its_value_and_one_of_another_comes_back`.
- **`chain/add`**, **`chain/move-up`**, **`chain/move-down`**, **`chain/remove`**, **`chain/back`**: add an effect (the picker opens), move the open one, remove it, back to the chain. Opened from the Mix tab or the Pads tab, ‹ goes back there.
- **`chain/strip`**: the strip button by ‹: the strip whose chain this is. Tap it for the strip list. *Test:* `the_strip_button_opens_another_strips_chain`.
- **`chain/strips-group`**, **`chain/strips-prev`**, **`chain/strips-next`**, **`chain/strips-pick`**, **`chain/strips-back`**: the strip list: Tracks, Pads, Inputs, Returns, Submixes, Master; page through a group; tap a strip to open its chain (on its first effect, or the picker on an empty chain); Back keeps the strip. *Test:* `the_strip_button_opens_another_strips_chain`.
- **`chain/macro`**, **`chain/learn`**, **`chain/add-lfo`**, **`chain/add-follower`**, **`chain/rows`**: the Modulate row under the page, after touching a control: give it to macro 1–8, Learn it onto the next Q-Link turned, add an LFO or a follower (the rows open over the page to set its depth), and the link under its name ("LFO 1 · +18 %  ›") that opens or shuts its mod matrix rows. While the rows are open, they and this row are the only targets. A macro button answers over its whole pitch, the 3 px gap included. *Tests:* `learn_puts_a_knob_on_the_next_qlink_turned`, `an_lfo_on_a_knob_gets_a_row_and_comes_off`, `chain_touches_become_actions` (mpfree-chain-ui).
- **`macros/knob`**, **`macros/bank`**, **`macros/target`**: the macros (drag), the bank (1–4, 5–8), a macro's target (tap to edit its range and curve).
- **`macros/morph`**, **`macros/save`**, **`macros/snap`**, **`macros/back`**: the morph between saved A and B, Save A or B, Snap back, Back.

## Prefs, options and the debug pages

- **`options/row`**, **`options/row-less`**, **`options/row-more`**: a row of Prefs or a page's options (MENU): tap to pick it, ‹ › to change it. *Test:* `appearance_switches_the_theme_live`.
- **`controls/row`**: Map controls: a hardware button's row; tap it to learn that button again. **`mapcard/button`**: Stop listening, Skip, Clear.
- **`dtabs/segment`**: the debug pages' tabs (MIDI, Performance, CPU by engine, Processes, ...).
- **`calibrate/mark`**, **`calibrate/button`**: touch calibration: tap each mark; Start again, Exit. *Test:* `calibrate_touch_opens_from_prefs_and_takes_every_finger`.
- **`touchlab/button`**: the touch lab's Exit, Start, Skip. *Test:* `the_touch_lab_opens_from_prefs_and_exit_or_a_page_change_closes_it`.
- **`ttest/area`**, **`ttest/mode`**: the touch test area, and its modes (Free, Targets, Swipes, Clear; a mode that would change nothing is faint).
- **`ttune/row`**, **`ttune/row-minus`**, **`ttune/row-plus`**, **`ttune/action`**: the touch tuning rows (shift and stretch) and Use suggested, Defaults, Save. *Test:* `the_touch_page_tunes_every_touch_and_saves_on_request`.

## Controls the sweep doesn't reach

These show only in a state no sample screen starts in (a picker, a menu, a dialog open). The workflow tests reach some of them.

- **`chain/pick`**, **`chain/cancel`**: the effect picker: tap an effect to add it at its defaults (a hold opens its presets while the finger is still down); Cancel adds nothing. *Test:* `the_effect_pickers_cancel_adds_nothing`.
- **`chain/section`**, **`chain/star`**, **`chain/presets`**, **`chain/preset`**, **`chain/presets-back`**, **`chain/page-prev`**, **`chain/page-next`**: the picker's groups down the left (Favourites, Recent, then each category; the open one is unavailable), an effect's star (favourite or not), its presets line (opens its presets when the finger lifts on it), a preset in that list (Defaults first; adds the effect set that way when the finger lifts on it, so a slip adds nothing), the way back to the effects, and the pages of a long group. *Tests:* `stars_keep_favourites_and_favourites_open_first`, `a_hold_or_the_presets_line_opens_presets_and_a_tap_adds_one`, `a_hold_opens_presets_with_the_finger_down_and_rows_act_on_the_lift`, `a_long_section_pages` (mpfree-chain-ui), `the_sample_tabs_fx_shows_the_pad_then_the_track`.
- **`chain/scope`**: on the Sample tab's FX, the pad's chain or its track's. *Test:* `the_sample_tabs_fx_shows_the_pad_then_the_track`.
- **`chain/depth`**, **`chain/remove-row`**: in the mod matrix rows over the page (opened from the Modulate row): drag a row's bar to set its depth, x removes the row. A touch outside the rows shuts them. *Test:* `an_lfo_on_a_knob_gets_a_row_and_comes_off`.
- **`chain/mode-pick`**: a mode in the list the Mode chip opens: switches to it and shuts the list. *Test:* `the_mode_chip_opens_the_list_of_modes` (mpfree-chain-ui).
- **`macros/editor-done`**, **`macros/editor-range`**, **`macros/editor-curve`**, **`macros/editor-flip`**, **`macros/editor-remove`**: a macro target's editor: its range, curve, flip, remove it, Done.
- **`mixer/menu`**: a strip's menu. **Freeze** records the next pass of the playing loop through the strip's effects and plays that from then on, so its effects (and a track's own instrument) rest and free the CPU; **Thaw** goes back to the live effects, and an edit to what the strip plays thaws it by itself. **Print to a new sample** records the next pass with its effects and their tail into Samples › Prints. While a take waits or records, the menu offers only **Cancel the freeze** (or the print), which changes nothing. *Example:* a song plays and the CPU chip is high: tap Drums, tap Drums again, Freeze; a bar or two later the hint says "Drums: frozen" and the CPU drops. *Tests:* `a_strips_menu_freezes_the_playing_loop_and_thaws_it`, `a_selected_strips_name_opens_its_menu_for_freeze_and_print` (mpfree-mixer-ui), and in mpfree-app's `takes.rs` `freeze_takes_the_pads_loop_then_plays_it_in_place_of_the_synth` and `a_print_is_saved_in_samples_prints`.
- **`mixer/output-choice`**: the output picker's choices.
- **`mixer/row`**, **`mixer/field`**, **`mixer/picker-done`**, **`mixer/picker-choice`**: an I/O section's rows (MIDI, Sync, CV): pick a row, tap a field to change it (a faint field says why), and a field's picker. *Test:* `the_io_view_follows_its_section_and_says_why_a_setting_stays`.
- **`mixer/monitor-kind`**, **`mixer/monitor-way`**, **`mixer/monitor-hold`**, **`mixer/monitor-clear`**, **`mixer/monitor-port`**, **`mixer/monitor-light`**: the I/O view's Monitor section, which lists the MIDI going in and out as it happens. The chips pick which kinds (Notes, CC, Program, Bend & pressure, Clock, SysEx) and ways (In, Out) are listed; Hold stops the list so you can read it, Hold again catches up; Clear empties it. A tap on a port's name lists only that port and way, a tap on one of its channel lights only that channel, and the same tap again lists everything. *Example:* a keyboard plays nothing on a track: open the Monitor, play a key, and the keyboard's in row lights on the channel it sends on, so you know which channel to set. *Test:* `chips_and_lights_pick_what_is_listed` and `the_list_holds_while_scrolled_back_and_follows_at_the_top` (mpfree-midi-ui).
- **`browser/filter`**, **`browser/key`**, **`browser/confirm-yes`**, **`browser/confirm-no`**: the browser's type filters, the search keyboard's keys, and a confirm dialog (moving a file to the Trash).
- **`browser/export`**: the Export panel, which **Export** in BROWSE > Projects opens over the browser. It turns the song into files: tick Mixdown, Stems and MIDI file, pick Stems by (Track, the default: one stem per track, the range playing again for each track that shares a channel, as the row says; or Channel: one per mixer channel in one pass), pick the range (Arrangement, or Bars with - and + for how many), check the File row (24-bit, 16-bit or 32-bit float, and 44.1 or 48 kHz; it starts as Prefs > Export file and Export rate have it, and a pick here is for this export only), then Start. While it runs, Cancel stops it and removes its files; when it is done, Close, or Show in BROWSE to open the new folder in Recordings. *Example:* open a song with an arrangement, BROWSE, Projects, Export, untick MIDI file, Start, then Show in BROWSE once it says Exported. *Test:* `the_export_panel_answers_taps` and `export_asks_the_app_and_its_panel_takes_the_screen` (mpfree-browser-ui, the Stems by and File rows' buttons and picks), `the_format_starts_as_prefs_has_it_and_changes_for_one_export` and `stems_by_track_or_channel_say_how_many_passes` (its `src/export.rs`), and in mpfree-app `the_pass_plays_the_range_then_puts_the_transport_back` and `stems_by_track_replay_the_range_with_each_track_alone`.
- **`smp-grain-cloud/play-as-grains`**: on a pad that doesn't play as grains yet, makes it play as grains.
