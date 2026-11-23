//! The demo songs (`songs.rs`) opened, played, performed, edited, saved
//! and opened again through the whole session, as a musician would: every
//! step through the screen's controls and the MPC's buttons and pads, and
//! every check against the engines (what the time engine played, what the
//! library holds), not only the screen.

#[cfg(test)]
mod tests {
    use super::super::songs::{self, Song};
    use super::super::{on_big_stack, Session};
    use mpfree_transport::time::Tick;
    use std::collections::BTreeSet;

    // ---------------------------------------------------------- the lab

    /// A session with a library of its own holding the three songs; the
    /// folder goes when the test ends.
    struct Lab {
        s: Session,
        dir: std::path::PathBuf,
    }

    impl Drop for Lab {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn lab(tag: &str) -> Lab {
        let dir = std::env::temp_dir().join(format!("mpfree-songs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch folder");
        let s = Session::with_library(&dir);
        let d = s.defaults();
        for song in songs::all() {
            songs::put(s.library.as_ref().expect("a library"), &song, &d)
                .expect("the song is saved");
        }
        Lab { s, dir }
    }

    /// BROWSE, Projects, the song's row tapped twice (the first picks it,
    /// the second opens it): as on the MPC.
    fn open(s: &mut Session, name: &str) {
        // BROWSE opens the browser, or closes it if it shows (it stays
        // open after New project, say).
        if s.app.view() != crate::app::View::Browser {
            s.button("BROWSE");
        }
        s.press("browser", "place", "Projects");
        s.press("browser", "row", name);
        s.press("browser", "row", name);
        s.run_for(0.3);
        assert!(
            s.lines
                .iter()
                .any(|l| *l == format!("project: {name} is in")),
            "{name} opened: {:?}",
            &s.lines[s.lines.len().saturating_sub(10)..]
        );
    }

    /// A bar of the song, in seconds.
    fn bar(song: &Song) -> f64 {
        240.0 / song.tempo
    }

    /// Launch scene `row` (from 0) on the Arrange page; PLAY if stopped.
    fn launch(s: &mut Session, row: usize) {
        s.press("header", "tab", "Arrange");
        s.press("clips", "scene", &format!("Scene {}", row + 1));
        if !s.time.info().state.rolling() {
            s.press("transport", "play", "Play");
        }
    }

    /// When the first note after `from` (seconds) played: the song's start.
    fn first_note_after(s: &Session, from: f64) -> f64 {
        s.hits
            .played
            .iter()
            .find(|p| p.at >= from)
            .map(|p| p.at)
            .expect("something played")
    }

    /// The tracks that played between `a` and `b` seconds.
    fn tracks_between(s: &Session, a: f64, b: f64) -> BTreeSet<usize> {
        s.hits
            .played
            .iter()
            .filter(|p| p.at >= a && p.at < b)
            .map(|p| p.track)
            .collect()
    }

    /// The tracks that played inside bars `from..to` after `t0` (half a
    /// bar kept clear at each end, for swing and the bar line).
    fn section(s: &Session, song: &Song, t0: f64, from: f64, to: f64) -> BTreeSet<&'static str> {
        let b = bar(song);
        tracks_between(s, t0 + (from + 0.5) * b, t0 + (to - 0.5) * b)
            .into_iter()
            .map(|t| name_of(song, t))
            .collect()
    }

    fn name_of(song: &Song, t: usize) -> &'static str {
        // The songs' track names are 'static strings.
        let n = &song.clips.tracks[t].name;
        Box::leak(n.clone().into_boxed_str())
    }

    fn set(names: &[&'static str]) -> BTreeSet<&'static str> {
        names.iter().copied().collect()
    }

    // ------------------------------------------------------- opening

    /// Each song opens from BROWSE > Projects with every clip, scene and
    /// placement as written, its tempo and swing, and the Arrange page
    /// names its tracks.
    #[test]
    fn every_song_opens_from_browse_as_it_was_written() {
        on_big_stack(|| {
            let mut l = lab("open");
            for song in songs::all() {
                open(&mut l.s, song.name);
                // As written, its tracks with the lasting ids an open
                // gives them.
                let mut written = song.clips.clone();
                written.give_track_ids();
                written.give_clip_ids();
                assert_eq!(l.s.time.project(), &written, "{}", song.name);
                assert_eq!(l.s.app.settings.bpm, song.tempo, "{}", song.name);
                l.s.run_for(0.1);
                assert_eq!(l.s.time.info().swing, song.swing, "{}", song.name);
                l.s.press("header", "tab", "Arrange");
                let heads: Vec<String> =
                    l.s.controls()
                        .into_iter()
                        .filter(|c| c.widget == "clips" && c.target.id == "track")
                        .map(|c| c.target.label)
                        .collect();
                let names: Vec<String> = song.clips.tracks.iter().map(|t| t.name.clone()).collect();
                assert_eq!(heads, names, "{}", song.name);
            }
        });
    }

    // ------------------------------------------------------- playing

    /// System status's gate: each song opens and plays eight bars from its
    /// first scene with nothing wrong on the status board. The watchers
    /// look each time the audio loop's timing arrives (every 250 ms), as
    /// main.rs sends it; this session has no audio loop, so the test sends
    /// it.
    #[test]
    fn every_song_plays_green() {
        on_big_stack(|| {
            for (i, song) in songs::all().into_iter().enumerate() {
                // A session each, so no song asks to save the last.
                let mut l = lab(&format!("green-{i}"));
                open(&mut l.s, song.name);
                launch(&mut l.s, 0);
                let looks = (8.0 * bar(&song) / 0.25).ceil() as usize;
                for _ in 0..looks {
                    l.s.run_for(0.25);
                    l.s.screen
                        .apply(crate::gui::ToUi::Timing(crate::gui::Timing::default()));
                }
                assert!(l.s.time.info().state.rolling(), "{} plays", song.name);
                let wrong = l.s.screen.status_unexpected(&[]);
                assert!(wrong.is_empty(), "{}: {wrong:?}", song.name);
            }
        });
    }

    /// Night Market: launching the Intro plays the whole song by its
    /// scenes' follow actions, each section with its own tracks (the Perc
    /// track sits the Break out by the scene's mutes), and the Outro stops
    /// it.
    /// Other gear drives the song (the Sync section's MMC in and Program
    /// change in): MMC Locate moves the playhead while stopped, a program
    /// change launches its scene and MMC Play plays it, and MMC Stop
    /// stops.
    #[test]
    fn other_gears_program_change_and_mmc_play_a_scene() {
        use mpfree_midi::setup::ProgramIn;
        on_big_stack(|| {
            let song = songs::night_market();
            let mut l = lab("night-mmc");
            let s = &mut l.s;
            open(s, song.name);
            s.midi_in_setup(true, ProgramIn::Scene);
            // Locate to 1 s (25 fps): where the song is a second in.
            s.midi_from_gear(&[
                0xF0, 0x7F, 0x7F, 0x06, 0x44, 0x06, 0x01, 0x20, 0, 1, 0, 0, 0xF7,
            ]);
            let want = s.time.tempo_map().tick_after(0.0, 44_100.0, 44_100).round() as Tick;
            assert_eq!(s.time.info().position, want);
            s.midi_from_gear(&[0xF0, 0x7F, 0x7F, 0x06, 0x01, 0xF7]);
            let verse = song.scene("Verse");
            let from = s.t();
            s.midi_from_gear(&[0xC0, verse as u8]);
            s.midi_from_gear(&[0xF0, 0x7F, 0x7F, 0x06, 0x02, 0xF7]);
            assert!(s.time.info().state.rolling());
            s.run_for(4.0 * bar(&song));
            let t0 = first_note_after(s, from);
            assert_eq!(
                section(s, &song, t0, 0.0, 4.0),
                set(&["Drums", "Perc", "Bass"])
            );
            s.midi_from_gear(&[0xF0, 0x7F, 0x7F, 0x06, 0x01, 0xF7]);
            assert!(!s.time.info().state.rolling());
        });
    }

    #[test]
    fn night_market_plays_its_five_sections_by_follow_actions() {
        on_big_stack(|| {
            let song = songs::night_market();
            let mut l = lab("night-follow");
            let s = &mut l.s;
            open(s, song.name);
            let from = s.t();
            launch(s, song.scene("Intro"));
            s.run_for(29.0 * bar(&song));
            let t0 = first_note_after(s, from);
            let want = [
                ("Intro", 0.0, 4.0, set(&["Drums", "Keys"])),
                ("Verse", 4.0, 12.0, set(&["Drums", "Perc", "Bass"])),
                (
                    "Hook",
                    12.0,
                    20.0,
                    set(&["Drums", "Perc", "Bass", "Keys", "Chops"]),
                ),
                ("Break", 20.0, 24.0, set(&["Keys", "Chops"])),
                ("Outro", 24.0, 28.0, set(&["Drums", "Bass"])),
            ];
            for (name, a, b, tracks) in want {
                assert_eq!(section(s, &song, t0, a, b), tracks, "{name}");
            }
            let b = bar(&song);
            assert!(
                tracks_between(s, t0 + 28.2 * b, s.t()).is_empty(),
                "the Outro's Stop ends it"
            );
        });
    }

    /// Night Market: FILL (PLAY held while playing) through the Verse's second bar plays the
    /// fill-only snares at its end; without it they stay out.
    #[test]
    fn fill_held_plays_the_fill_notes_and_only_then() {
        on_big_stack(|| {
            let song = songs::night_market();
            let mut l = lab("night-fill");
            let s = &mut l.s;
            open(s, song.name);
            let b = bar(&song);
            let snares = |s: &Session, a: f64, z: f64| {
                s.hits
                    .played
                    .iter()
                    .filter(|p| {
                        p.track == 0 && p.note == songs::pad::SNARE && p.at >= a && p.at < z
                    })
                    .count()
            };
            let from = s.t();
            launch(s, song.scene("Verse"));
            // Two bars as written: the second bar's last two steps are
            // fill-only, so 4 snares (2 backbeats, a ghost each bar... see
            // the pattern): count them, then the same two bars with FILL.
            s.run_for(2.0 * b);
            let t0 = first_note_after(s, from);
            s.run_for(t0 + 2.0 * b - s.t());
            let plain = snares(s, t0, t0 + 2.0 * b);
            s.hold("PLAY", 2.0 * b);
            let filled = snares(s, t0 + 2.0 * b, t0 + 4.0 * b);
            assert_eq!(filled, plain + 2, "FILL adds the two fill snares");
        });
    }

    /// The step a note played on, from `t0`, in sixteenths (no swing).
    fn step_of(song: &Song, t0: f64, at: f64) -> i64 {
        // A ratchet's later hits fall inside their step: floor, not round.
        ((at - t0) / (15.0 / song.tempo) + 0.01).floor() as i64
    }

    /// Glass Garden's Bloom: clips of 3, 5, 7, 12 and 16 steps loop
    /// against each other, each on its own length; the Perc's ratchets
    /// play 2 and 3 hits; the Bass changes on alternate passes by its trig
    /// conditions; the lead plays at half speed.
    #[test]
    fn glass_garden_loops_its_clips_against_each_other() {
        on_big_stack(|| {
            use songs::pad::*;
            let song = songs::glass_garden();
            let mut l = lab("glass-poly");
            let s = &mut l.s;
            open(s, song.name);
            let from = s.t();
            launch(s, song.scene("Bloom"));
            s.run_for(8.5 * bar(&song));
            let t0 = first_note_after(s, from);
            let notes = |track: &str, pad: u8| -> Vec<i64> {
                let t = song.track(track);
                s.hits
                    .played
                    .iter()
                    .filter(|p| p.track == t && p.note == pad && p.at >= t0 - 0.001)
                    .map(|p| step_of(&song, t0, p.at))
                    .filter(|&k| k < 128)
                    .collect()
            };
            let mods =
                |v: &[i64], m: i64| v.iter().map(|k| k.rem_euclid(m)).collect::<BTreeSet<_>>();
            // Kick: 16 steps.
            assert_eq!(
                mods(&notes("Kick", KICK), 16),
                BTreeSet::from([0, 4, 8, 12])
            );
            // Hats: 3 steps, the open hat on the third.
            assert_eq!(mods(&notes("Hats", HAT), 3), BTreeSet::from([0]));
            assert_eq!(mods(&notes("Hats", OPEN_HAT), 3), BTreeSet::from([2]));
            assert_eq!(notes("Hats", OPEN_HAT).len(), 42); // steps 2, 5, ... 125: the three-step row lands 42 times in 128 steps
                                                           // Clap: 5 steps.
            assert_eq!(mods(&notes("Clap", CLAP), 5), BTreeSet::from([2, 4]));
            // Perc: 7 steps; the ratchets play twice on step 1 and three
            // times on step 6 of each pass.
            let perc = notes("Perc", PERC_1);
            assert_eq!(mods(&perc, 7), BTreeSet::from([0, 3, 5]));
            let per_pass = |k: i64| {
                perc.iter()
                    .filter(|&&x| x.rem_euclid(7) == k && x < 7 * 18)
                    .count()
            };
            assert_eq!(
                (per_pass(0), per_pass(3), per_pass(5)),
                (2 * 18, 18, 3 * 18)
            );
            // Bass: 12 steps; pass 1 of every 2 plays 0, 3, 7; pass 2 plays 5, 8, 11.
            let bass = notes("Bass", SUB);
            for pass in 0..10 {
                let got: BTreeSet<i64> = bass
                    .iter()
                    .filter(|&&k| k / 12 == pass)
                    .map(|k| k % 12)
                    .collect();
                let want = if pass % 2 == 0 { [0, 3, 7] } else { [5, 8, 11] };
                assert_eq!(got, BTreeSet::from(want), "Bass pass {}", pass + 1);
            }
            // Lead: a 2-bar clip at half speed: its notes on steps 0, 12, 16
            // and 24 of the clip land on 0, 24, 32 and 48 of every 64.
            let lead_track = song.track("Lead");
            let lead: BTreeSet<i64> = s
                .hits
                .played
                .iter()
                .filter(|p| p.track == lead_track && p.at >= t0 - 0.001)
                .map(|p| step_of(&song, t0, p.at).rem_euclid(64))
                .collect();
            assert_eq!(lead, BTreeSet::from([0, 24, 32, 48]));
        });
    }

    /// Tidal Suite picks its own way: after the Ebb, the Flow or the Ebb
    /// again; after the Flow, the Undertow or the Ebb; the Undertow swaps
    /// between its two drum clips every two bars by clip follow actions;
    /// then the Shore, which stops. Every move it makes is one the scenes
    /// allow, and it comes to the Shore.
    #[test]
    fn tidal_suite_picks_its_own_way_and_comes_to_shore() {
        on_big_stack(|| {
            let song = songs::tidal_suite();
            let mut l = lab("tidal-way");
            let s = &mut l.s;
            open(s, song.name);
            launch(s, song.scene("Ebb"));
            let b = bar(&song);
            // The drums' playing clip, bar by bar, until everything stops.
            let mut slots: Vec<Option<usize>> = Vec::new();
            for _ in 0..120 {
                s.run_for(b);
                let st = s.time.state();
                slots.push(st.launches.iter().find(|l| l.track == 0).map(|l| l.slot));
                if st.launches.is_empty() && slots.len() > 4 {
                    break;
                }
            }
            let mut runs: Vec<(usize, usize)> = Vec::new();
            for sl in slots.iter().flatten() {
                match runs.last_mut() {
                    Some((x, n)) if x == sl => *n += 1,
                    _ => runs.push((*sl, 1)),
                }
            }
            let names = ["Ebb", "Flow", "Undertow A", "Undertow B", "Shore"];
            let path: Vec<String> = runs
                .iter()
                .map(|(x, n)| format!("{} x{n}", names[*x]))
                .collect();
            assert_eq!(runs.first().map(|r| r.0), Some(0), "{path:?}");
            assert_eq!(
                runs.last().map(|r| r.0),
                Some(4),
                "it comes to the Shore: {path:?}"
            );
            for w in runs.windows(2) {
                let ok = matches!(
                    (w[0].0, w[1].0),
                    (0, 1) | (1, 2) | (1, 0) | (2, 3) | (3, 2) | (2, 4) | (3, 4)
                );
                assert!(ok, "a move the scenes allow: {path:?}");
            }
            assert!(
                slots
                    .iter()
                    .rev()
                    .skip_while(|x| x.is_none())
                    .take(4)
                    .all(|x| *x == Some(4)),
                "the Shore plays its 4 bars: {path:?}"
            );
        });
    }

    // -------------------------------------------- editing and saving

    /// Each song, edited on the MPC (the tempo up a step, a step tapped on
    /// Play's Beats, a track's level down on the mixer), saved with SHIFT +
    /// BROWSE, a New project started (empty), then opened again: the clips,
    /// the tempo and the level come back as edited, not as first written.
    #[test]
    fn each_song_edited_saves_and_opens_again_as_edited() {
        on_big_stack(|| {
            use mpfree_fx::ids::mixer as mx;
            use mpfree_fx::StripId;
            let mut l = lab("edit-save");
            let s = &mut l.s;
            for song in songs::all() {
                open(s, song.name);
                // The tempo, a step up, on Arrange's record panel.
                s.press("header", "tab", "Arrange");
                s.press("record", "row-more", "Tempo ›");
                let tempo = s.app.settings.bpm;
                assert!(tempo > song.tempo, "{}: Tempo › raised it", song.name);
                // A step on Play's Beats (track 1's first clip).
                s.press("header", "tab", "Play");
                s.play_view("Beats");
                let before = s.time.project().clone();
                // The first row's step 2 (the row reads the pad's sound
                // name, or "Pad 1" with no kit loaded).
                let step2 = s
                    .controls()
                    .iter()
                    .find(|c| {
                        c.widget == "steps"
                            && c.target.id == "step"
                            && c.target.label.ends_with(" step 2")
                    })
                    .map(|c| c.target.label.clone())
                    .expect("a step 2 on Beats");
                s.press("steps", "step", &step2);
                assert_ne!(s.time.project(), &before, "{}: the step went in", song.name);
                // Track 2's fader down on the mixer.
                let v0 = s.mix.mixer.param(StripId::Track(1), mx::VOLUME);
                s.press("header", "tab", "Mix");
                s.qlink(1, -6);
                let _ = v0;
                let edited = s.time.project().clone();
                let mixer = crate::project::mixer_text(&s.mix.mixer, s.mix.pad_mutes());

                s.button_with("SHIFT", "BROWSE");
                let saved = format!("project: saved {}", song.name);
                assert!(s.until_said(&saved, 5.0), "{}: {:?}", saved, s.said());

                s.button("BROWSE");
                s.press("browser", "place", "Projects");
                s.press("browser", "button", "New project");
                s.run_for(0.3);
                assert!(
                    s.time.project().tracks.iter().all(|t| t.slots.is_empty()),
                    "New project is empty"
                );

                open(s, song.name);
                assert_eq!(
                    s.time.project(),
                    &edited,
                    "{}: the clips as edited",
                    song.name
                );
                assert_eq!(
                    s.app.settings.bpm, tempo,
                    "{}: the tempo as edited",
                    song.name
                );
                s.run_for(0.5);
                assert_eq!(
                    crate::project::mixer_text(&s.mix.mixer, s.mix.pad_mutes()),
                    mixer,
                    "{}: the mixer as edited",
                    song.name
                );
            }
        });
    }

    /// Sampler refactor phase 3 step 14, through the whole session. A
    /// song's project as the build before kits had ids saved it (format
    /// 2, `track 3 kit 2` in the track kits part's version 2) doesn't open
    /// from BROWSE: it says why, and what was open stays. The same song
    /// with track 2 on the synth and track 3 on kit 2 by their ids opens
    /// with them, every kit place in its list with an id, and plays its
    /// notes. Saved with SHIFT + BROWSE it is format 3 and names them by
    /// the same ids. After New project, opened again, every track plays
    /// what it played, and the song plays the same notes.
    #[test]
    fn a_project_from_before_kit_ids_is_refused_and_one_saved_now_comes_back() {
        on_big_stack(|| {
            use crate::instrument_list::InstrumentList;
            use crate::project::{FORMAT, FORMAT_KEY, INSTRUMENT_LIST, TRACK_KITS};
            let mut l = lab("kit-ids");
            let s = &mut l.s;
            let song = songs::all().into_iter().next().expect("a song");
            assert!(
                song.clips.tracks.len() >= 3,
                "{} has three tracks",
                song.name
            );
            let lib = s.library.clone().expect("a library");
            let loc = |name: &str| {
                mpfree_library::Loc::top(mpfree_library::Place::Projects)
                    .join(name)
                    .expect("a project's place")
            };
            let snap = lib.open_project(&loc(song.name)).expect("the song opens");
            let with = |format: u32, list: String, kits: String| {
                let mut p = snap.clone();
                p.meta.extra.retain(|(k, _)| k != FORMAT_KEY);
                p.meta
                    .extra
                    .push((FORMAT_KEY.to_string(), format.to_string()));
                p.parts
                    .retain(|(o, _)| o != INSTRUMENT_LIST && o != TRACK_KITS);
                p.parts.push((INSTRUMENT_LIST.to_string(), list));
                p.parts.push((TRACK_KITS.to_string(), kits));
                p
            };

            // Saved before kits had ids: refused, and nothing changes.
            let old = "Before kit ids";
            lib.save_project(
                &loc(old),
                &with(
                    2,
                    "mpfree-instrument-list 1\nnext 2\ninstrument 1 shared-synth\n".to_string(),
                    "mpfree-track-kits 2\ntrack 2 instrument 1\ntrack 3 kit 2\n".to_string(),
                ),
                10,
            )
            .expect("saved");
            let before = (s.track_kits.clone(), s.instrument_list.clone());
            s.button("BROWSE");
            s.press("browser", "place", "Projects");
            s.press("browser", "row", old);
            s.press("browser", "row", old);
            s.run_for(0.3);
            let refused = format!(
                "project: didn't open {old}: it was saved by an older MPFree, which this one \
                 doesn't open (project format 2; this one opens {FORMAT})"
            );
            assert!(
                s.said().iter().any(|l| l.starts_with(&refused)),
                "{refused}: {:?}",
                s.said()
            );
            assert!(!s.lines.contains(&format!("project: {old} is in")));
            assert_eq!((s.track_kits.clone(), s.instrument_list.clone()), before);

            // Saved now: every kit place has an id (instruments 2, 3 ...
            // after the synth's 1), and the tracks name theirs by it.
            let mut list = InstrumentList::new();
            list.follow_kits(s.kits());
            assert_eq!(list, s.instrument_list, "the session's kits have ids");
            let kit_2 = list.of_kit(1).expect("kit 2 has an id");
            assert_eq!(kit_2.number(), 3);
            let kits_text =
                format!("mpfree-track-kits 3\ntrack 2 instrument 1\ntrack 3 instrument {kit_2}\n");
            let name = "Kit ids";
            lib.save_project(
                &loc(name),
                &with(FORMAT, list.text(), kits_text.clone()),
                10,
            )
            .expect("saved");

            // What the song plays: each note's track, pad and velocity over
            // its first two bars, from scene 1 launched.
            let notes = |s: &mut Session| {
                let t0 = s.t();
                launch(s, 0);
                s.run_for(3.0 * bar(&song));
                s.button("STOP").run_for(0.2);
                let first = first_note_after(s, t0);
                let end = first + 2.0 * bar(&song) - 0.05;
                s.hits
                    .played
                    .iter()
                    .filter(|p| p.at >= first && p.at < end)
                    .map(|p| (p.track, p.note, p.velocity))
                    .collect::<Vec<_>>()
            };

            open(s, name);
            assert!(s.track_kits.plays_synth(1), "track 2 plays the synth");
            assert_eq!(s.track_kits.kit_of(2), Some(1), "track 3 plays kit 2");
            assert_eq!(s.instrument_list, list);
            let kits = s.track_kits.clone();
            let played = notes(s);
            assert!(!played.is_empty(), "the song played");
            assert!(
                !s.lines
                    .iter()
                    .any(|l| l.contains("instrument") || l.contains("format")),
                "nothing to say about it: {:?}",
                s.lines
            );

            s.button_with("SHIFT", "BROWSE");
            let saved = format!("project: saved {name}");
            assert!(s.until_said(&saved, 5.0), "{saved}: {:?}", s.said());
            let back = lib.open_project(&loc(name)).expect("it opens");
            assert!(back
                .meta
                .extra
                .contains(&(FORMAT_KEY.to_string(), FORMAT.to_string())));
            assert_eq!(back.part(INSTRUMENT_LIST), Some(list.text().as_str()));
            assert_eq!(back.part(TRACK_KITS), Some(kits_text.as_str()));

            s.button("BROWSE");
            s.press("browser", "place", "Projects");
            s.press("browser", "button", "New project");
            s.run_for(0.3);
            assert!(
                !s.track_kits.plays_synth(1),
                "New project: track 2 on kit 1"
            );
            assert_eq!(
                s.instrument_list, list,
                "New project: the kits keep their ids"
            );

            open(s, name);
            assert_eq!(s.track_kits, kits, "every track plays what it played");
            assert_eq!(s.instrument_list, list);
            assert_eq!(notes(s), played, "the same notes");
        });
    }

    /// What the pads play is kept with the project (the instrument
    /// redesign's step 3): Poly chosen for the pads and saved; a new
    /// project follows the track; the project opens with Poly on the pads,
    /// named in the header again.
    #[test]
    fn the_synth_chosen_for_the_pads_comes_back_with_the_project() {
        on_big_stack(|| {
            use crate::app::PadFocus;
            let mut l = lab("pad-focus");
            let s = &mut l.s;
            let name = "Night Market";
            let lib = s.library.clone().expect("a library");
            let loc = mpfree_library::Loc::top(mpfree_library::Place::Projects)
                .join(name)
                .expect("a project's place");
            open(s, name);
            assert_eq!(s.app.pads_focus(), PadFocus::Track, "saved before the part");
            s.press("header", "tab", "Synth");
            s.press("synth-pads-head", "play", "").settle();
            assert!(s.app.pads_chose_synth(), "{}", s.hint());
            s.button_with("SHIFT", "BROWSE");
            let saved = format!("project: saved {name}");
            assert!(s.until_said(&saved, 5.0), "{saved}: {:?}", s.said());
            let snap = lib.open_project(&loc).expect("it opens");
            assert_eq!(
                snap.part(crate::project::PADS),
                Some("mpfree-pads 2\nplay instrument 1\n")
            );

            s.button("BROWSE");
            s.press("browser", "place", "Projects");
            s.press("browser", "button", "New project");
            s.run_for(0.3);
            assert_eq!(
                s.app.pads_focus(),
                PadFocus::Track,
                "a new project follows the track"
            );
            assert!(s.control("header", "chip", "Pads: Poly").is_none());

            open(s, name);
            assert!(s.app.pads_chose_synth(), "the project's: Poly on the pads");
            s.press("header", "tab", "Play");
            s.run_for(0.05);
            assert!(s.control("header", "chip", "Pads: Poly").is_some());
            let before = s.previewed.len();
            s.pad(2, 90, 0.05);
            assert_eq!(s.previewed.len(), before + 1, "the pads play Poly");
        });
    }

    /// Tunings T3 through the whole session: a song in just intonation on
    /// D. Picked on the Synth tab's options, the tuning reaches the kit
    /// (said on the Console), and a keygroup's key sounds at the tuning's
    /// pitch; the project keeps it, New project puts 12-TET back, and
    /// opening the song again tunes the kit again.
    #[test]
    fn a_song_in_just_on_d_plays_its_keygroups_tuned_and_keeps_it() {
        on_big_stack(|| {
            use crate::gui::FromUi;
            use mpfree_sampler::engine::EngineView;
            use mpfree_sampler::model::{Layer, Pad, Project, Region, Source};
            use mpfree_tuning::{Builtin, Tuning};
            const RATE: u32 = crate::RATE;
            let mut l = lab("kit-tuning");
            let s = &mut l.s;
            let name = "Night Market";
            let lib = s.library.clone().expect("a library");
            let loc = mpfree_library::Loc::top(mpfree_library::Place::Projects)
                .join(name)
                .expect("a project's place");
            open(s, name);
            s.make_sounds().run_for(0.05);
            // Kit 1 a keygroup over the whole keyboard: a 220 Hz tone on
            // A3, short enough that its octave copies are made at once.
            let keygroup = |s: &mut Session| {
                let n = (RATE as f32 * 0.7) as usize;
                let x = (0..n)
                    .map(|i| 0.3 * (i as f32 * 220.0 * std::f32::consts::TAU / RATE as f32).sin())
                    .collect();
                let tone = Source::new("tone", RATE, 1, x).unwrap();
                let mut p = Project::new();
                p.add_source(tone.clone());
                p.set_pad(
                    0,
                    Some(Pad::new("tone", Layer::new(Region::whole(&tone)))),
                    None,
                )
                .unwrap();
                p.spread_to_keys(0, 57).unwrap();
                let mut swap = crate::sampler::ViewSwap {
                    kit: 0,
                    view: Some(EngineView::build(&p, RATE)),
                    routes: vec![None; mpfree_sampler::model::PADS],
                };
                let player = s.sounds.as_mut().expect("a kit");
                player.select(0);
                assert!(player.swap_view(&mut swap));
            };
            // The pitch a key sounds at, from the left channel heard.
            let key = |s: &mut Session, note: u8| -> f64 {
                s.run_for(0.3);
                s.heard = Some(Vec::new());
                assert!(s
                    .sounds
                    .as_mut()
                    .expect("a kit")
                    .key_kit_at(0, note, 100, 0));
                s.run_for(0.35);
                let h = s.heard.take().unwrap();
                let x: Vec<f64> = h.iter().step_by(2).map(|&v| f64::from(v)).collect();
                let (from, to) = ((RATE / 20) as usize, (RATE as f64 * 0.3) as usize);
                let ups: Vec<f64> = (from..to)
                    .filter(|&i| x[i - 1] < 0.0 && x[i] >= 0.0)
                    .map(|i| (i - 1) as f64 + x[i - 1] / (x[i - 1] - x[i]))
                    .collect();
                assert!(ups.len() > 50, "the key sounds: {}", ups.len());
                let span = ups[ups.len() - 1] - ups[0];
                (ups.len() - 1) as f64 * f64::from(RATE) / span
            };
            let cents = |a: f64, b: f64| 1200.0 * (a / b).log2();
            let kits = |s: &Session| {
                let p = s.sounds.as_ref().expect("a kit");
                (0..p.kits())
                    .map(|k| p.kit_tuning(k).cloned().expect("a kit place"))
                    .collect::<Vec<_>>()
            };
            keygroup(s);
            let f_sharp = 66u8;
            let plain = key(s, f_sharp);
            assert!(
                cents(plain, Tuning::standard().hz(66.0).into()).abs() < 1.0,
                "{plain}"
            );
            assert!(kits(s).iter().all(Tuning::is_standard));

            // The Synth tab's options: Tuning to Just, its root to D.
            s.press("header", "tab", "Synth").settle();
            s.button("MENU").settle();
            s.route(FromUi::MenuStep(5, 1));
            s.run_for(0.05);
            for _ in 0..2 {
                s.route(FromUi::MenuStep(6, 1));
                s.run_for(0.05);
            }
            s.button("MENU").settle();
            let just_d = Builtin::Just.tuning(2, 440.0);
            let said = "sampler: the kits' keygroups play Just (5-limit) on D \
                        (pad hits and slices as tuned on the pad)";
            assert!(s.until_said(said, 1.0), "{:?}", s.said());
            assert!(kits(s).iter().all(|t| *t == just_d));
            // F#4 is the just major third over D, 13.7 cents under 12-TET.
            let tuned = key(s, f_sharp);
            let want = f64::from(just_d.hz(66.0));
            assert!(cents(tuned, want).abs() < 1.0, "{tuned} Hz, not {want}");
            assert!((cents(tuned, plain) + 13.69).abs() < 1.0);

            // Saved with the project; New project is 12-TET again.
            s.button_with("SHIFT", "BROWSE");
            let saved = format!("project: saved {name}");
            assert!(s.until_said(&saved, 5.0), "{saved}: {:?}", s.said());
            let snap = lib.open_project(&loc).expect("it opens");
            let part = snap.part(crate::project::SYNTH).expect("a synth part");
            for line in ["tuning = just", "tuning-root = D"] {
                assert!(part.lines().any(|l| l == line), "{line}: {part}");
            }
            s.button("BROWSE");
            s.press("browser", "place", "Projects");
            s.press("browser", "button", "New project");
            s.run_for(0.3);
            assert!(kits(s).iter().all(Tuning::is_standard));
            let back = "sampler: the kits' keygroups play 12-TET \
                        (pad hits and slices as tuned on the pad)";
            assert!(s.lines.iter().any(|l| l == back), "{:?}", s.said());

            // Opened again: the kit is in just on D, and so is the key.
            open(s, name);
            s.run_for(0.05);
            assert!(kits(s).iter().all(|t| *t == just_d));
            keygroup(s);
            let again = key(s, f_sharp);
            assert!(cents(again, want).abs() < 1.0, "{again} Hz, not {want}");
        });
    }

    /// Layers (the instrument redesign's step 11a and 11b): FM picked for
    /// the pads, then Layer and Pluck in the list of synths. Each hit and
    /// each key plays both, each on its own strip; the header reads "Pads:
    /// FM + Pluck". The project keeps them in a part of its own, and a new
    /// project has none.
    #[test]
    fn layers_play_two_synths_from_the_pads_and_save_with_the_project() {
        on_big_stack(|| {
            use crate::instruments::{free, layer};
            let mut l = lab("pad-layers");
            let s = &mut l.s;
            let name = "Night Market";
            let lib = s.library.clone().expect("a library");
            let loc = mpfree_library::Loc::top(mpfree_library::Place::Projects)
                .join(name)
                .expect("a project's place");
            open(s, name);
            s.press("header", "tab", "Synth");
            s.press("synth-inst", "card", "").settle();
            s.press("synth-pick", "synth", "FM").run_periods(4).settle();
            s.press("synth-pick", "layer", "Layer").settle();
            s.press("synth-pick", "synth", "Pluck")
                .run_periods(4)
                .settle();
            s.press("synth-pick", "keep", "").run_periods(2).settle();
            let (fm, pluck) = (free(0), free(1));
            assert_eq!(s.mix.instruments.pads_play_at(), Some(fm));
            assert_eq!(s.mix.instruments.layer_at(0), Some(pluck), "{}", s.hint());
            assert!(
                s.control("header", "chip", "Pads: FM + Pluck").is_some(),
                "{}",
                s.hint()
            );
            // A hit, and a key: both synths.
            let notes = |s: &Session| {
                (
                    s.mix.instruments.stats(fm).notes,
                    s.mix.instruments.stats(pluck).notes,
                )
            };
            let before = notes(s);
            s.pad(2, 90, 0.05);
            s.key_edge(64, Some(100));
            assert!(s.mix.instruments.key_held(layer(0), 64));
            s.key_edge(64, None);
            let after = notes(s);
            assert_eq!(
                (after.0 - before.0, after.1 - before.1),
                (2, 2),
                "{after:?}"
            );
            assert!(!s.mix.instruments.key_held(layer(0), 64), "nothing hangs");
            // Saved in the pads part, with the pad map (sampler step 15b).
            s.button_with("SHIFT", "BROWSE");
            let saved = format!("project: saved {name}");
            assert!(s.until_said(&saved, 5.0), "{saved}: {:?}", s.said());
            let snap = lib.open_project(&loc).expect("it opens");
            let part = snap.part(crate::project::PADS).expect("the part");
            assert!(
                part.starts_with("mpfree-pads 2\nplay instrument ")
                    && part.contains("\nlayer instrument "),
                "{part}"
            );
            // New project: none.
            s.button("BROWSE");
            s.press("browser", "place", "Projects");
            s.press("browser", "button", "New project");
            s.run_for(0.3);
            assert!(!s.app.has_layers());
            assert_eq!(s.mix.instruments.layer_at(0), None);
            // Opening it again is main.rs's to check: the test session
            // builds no instruments at an open (app.rs's tests read the
            // part back with the list).
        });
    }

    /// Instrument redesign step 4: a track plays another track's own
    /// instrument. In Night Market, Track 2 (Perc) gets its own Poly; with
    /// Track 3 (Bass) selected, the Synth tab's Instrument row offers
    /// "Track 2's Poly" right after Kit. Picked, Track 3 plays it (said,
    /// and Arrange names it), the project keeps it by the instrument's
    /// id, and it comes back after New project. Kit on Track 2 takes its
    /// Poly out, and Track 3 plays its kit again, said.
    #[test]
    fn a_track_plays_another_tracks_instrument_and_the_project_keeps_it() {
        on_big_stack(|| {
            use crate::gui::FromUi;
            use mpfree_time_ui::TimeAction;
            let mut l = lab("send");
            let s = &mut l.s;
            let name = "Night Market";
            let lib = s.library.clone().expect("a library");
            let loc = mpfree_library::Loc::top(mpfree_library::Place::Projects)
                .join(name)
                .expect("a project's place");
            open(s, name);
            s.give_own(1, crate::synth::Engine::Poly);
            let pick = |s: &mut Session, track: usize, d: i32| {
                s.route(FromUi::Time(TimeAction::Arm(Some(track))));
                s.run_for(0.05);
                assert_eq!(s.app.track(), track);
                s.press("header", "tab", "Synth").settle();
                s.button("MENU").settle();
                assert_eq!(s.view(), crate::app::View::Options);
                s.route(FromUi::MenuStep(0, d));
                s.run_for(0.05);
                s.button("MENU").settle();
            };
            pick(s, 2, 1);
            assert_eq!(s.mix.instruments.owner(2), Some(1), "{:?}", s.said());
            assert_eq!(s.app.track_send(2), Some(1));
            assert!(
                s.lines.iter().any(|l| l == "track 3: plays Track 2's Poly"),
                "{:?}",
                s.said()
            );
            s.press("header", "tab", "Arrange").settle();
            let sounds: Vec<&str> = s.screen.time.tracks[1..3]
                .iter()
                .map(|t| t.sound.as_str())
                .collect();
            assert_eq!(sounds, ["Poly", "Track 2's Poly"]);

            s.button_with("SHIFT", "BROWSE");
            let saved = format!("project: saved {name}");
            assert!(s.until_said(&saved, 5.0), "{saved}: {:?}", s.said());
            let id = s
                .instrument_list
                .own_of(1)
                .expect("Track 2's Poly has an id");
            let snap = lib.open_project(&loc).expect("it opens");
            let kits = snap.part(crate::project::TRACK_KITS).unwrap_or_default();
            assert!(
                kits.lines()
                    .any(|l| l == format!("track 3 instrument {id}")),
                "{kits}"
            );

            s.button("BROWSE");
            s.press("browser", "place", "Projects");
            s.press("browser", "button", "New project");
            s.run_for(0.3);
            assert_eq!(s.mix.instruments.send(2), None, "New: no sends");
            assert_eq!(s.app.track_send(2), None);
            open(s, name);
            assert_eq!(s.mix.instruments.send(2), Some(1), "{:?}", s.said());
            assert_eq!(s.app.track_send(2), Some(1));

            // Kit on Track 2: its Poly goes, and Track 3 with it.
            pick(s, 1, -1);
            assert!(!s.mix.instruments.holds(1));
            assert_eq!(s.mix.instruments.send(2), None);
            assert_eq!(s.app.track_send(2), None);
            let gone = "Track 3 plays its kit again: Track 2's instrument is gone";
            assert!(s.lines.iter().any(|l| l == gone), "{:?}", s.said());
            s.press("header", "tab", "Arrange").settle();
            assert_ne!(s.screen.time.tracks[2].sound, "Track 2's Poly");
        });
    }

    /// Duplicate (step 4c): Track 3 steps on the Instrument row past
    /// "Track 2's Poly" to a copy of it, its own Poly with Track 2's sound
    /// and an id of its own; a turn on one leaves the other, and the
    /// project keeps both.
    #[test]
    fn a_copy_of_another_tracks_instrument_is_its_own() {
        on_big_stack(|| {
            use crate::gui::FromUi;
            use mpfree_time_ui::TimeAction;
            let mut l = lab("copy");
            let s = &mut l.s;
            let name = "Night Market";
            open(s, name);
            s.give_own(1, crate::synth::Engine::Poly);
            // A sound of Track 2's own, away from the engine's.
            let p = {
                let node = s.mix.instruments.node(1).expect("Track 2's Poly");
                *node
                    .params()
                    .iter()
                    .find(|p| !p.readonly && p.max > p.min)
                    .expect("a knob")
            };
            let theirs = |s: &Session, t: usize| s.mix.instruments.node(t).map(|n| n.param(p.id));
            let own = theirs(s, 1).unwrap();
            let set = if own == p.max { p.min } else { p.max };
            assert!(s.mix.instruments.param(1, p.id, set));
            s.run_for(0.05);
            assert_eq!(theirs(s, 1), Some(set));

            s.route(FromUi::Time(TimeAction::Arm(Some(2))));
            s.run_for(0.05);
            s.press("header", "tab", "Synth").settle();
            s.button("MENU").settle();
            for _ in 0..2 {
                s.route(FromUi::MenuStep(0, 1));
                s.run_for(0.05);
            }
            s.button("MENU").settle();
            assert!(s.mix.instruments.holds(2), "{:?}", s.said());
            assert_eq!(s.mix.instruments.send(2), None);
            assert_eq!(s.mix.instruments.owner(2), Some(2));
            let said =
                "Track 3 plays its own copy of Track 2's Poly: change one and the other stays";
            assert!(s.lines.iter().any(|l| l == said), "{:?}", s.said());
            assert_eq!(theirs(s, 2), Some(set), "the copy has Track 2's sound");
            // A turn on the copy leaves Track 2's.
            assert!(s.mix.instruments.param(2, p.id, own));
            s.run_for(0.05);
            assert_eq!((theirs(s, 1), theirs(s, 2)), (Some(set), Some(own)));

            s.press("header", "tab", "Arrange").settle();
            let sounds: Vec<&str> = s.screen.time.tracks[1..3]
                .iter()
                .map(|t| t.sound.as_str())
                .collect();
            assert_eq!(sounds, ["Poly", "Poly"]);
            s.button_with("SHIFT", "BROWSE");
            let saved = format!("project: saved {name}");
            assert!(s.until_said(&saved, 5.0), "{saved}: {:?}", s.said());
            let ids = (
                s.instrument_list.own_of(1).expect("Track 2's has an id"),
                s.instrument_list.own_of(2).expect("the copy has one"),
            );
            assert_ne!(ids.0, ids.1, "two instruments");
            open(s, name);
            assert!(s.mix.instruments.holds(1) && s.mix.instruments.holds(2));
            assert_eq!(s.mix.instruments.send(2), None);
        });
    }

    // ------------------------------------------------------- performing

    /// Night Market in song mode (Song on Arrange, Play song, then Play): every
    /// section of the arrangement plays the tracks placed in it, Verse 2
    /// and Hook 2 included, which the scenes alone never reach.
    #[test]
    fn night_market_in_song_mode_follows_its_arrangement() {
        on_big_stack(|| {
            let song = songs::night_market();
            let mut l = lab("night-song");
            let s = &mut l.s;
            open(s, song.name);
            s.press("header", "tab", "Arrange");
            s.press("clips", "view", "Song");
            s.press("song", "button", "Play song");
            assert!(s.time.song_on(), "Play song turned song mode on");
            let from = s.t();
            s.press("transport", "play", "Play");
            let arr = &song.clips.arrangement;
            let end = arr.markers.last().expect("markers").at / songs::BAR + 4;
            s.run_for((end as f64 + 1.0) * bar(&song));
            let t0 = first_note_after(s, from);
            for (i, m) in arr.markers.iter().enumerate() {
                let a = m.at / songs::BAR;
                let b = arr.markers.get(i + 1).map_or(end, |n| n.at / songs::BAR);
                let want: BTreeSet<&'static str> = arr
                    .placements
                    .iter()
                    .filter(|p| p.start < b * songs::BAR && p.start + p.length > a * songs::BAR)
                    .map(|p| name_of(&song, p.track))
                    .collect();
                assert_eq!(
                    section(s, &song, t0, a as f64, b as f64),
                    want,
                    "{}",
                    m.name
                );
            }
        });
    }

    /// TRACK MUTE in the middle of Night Market's Hook: pad 2 takes the
    /// Perc track out (the rest plays on), and pad 2 again brings it back.
    #[test]
    fn track_mute_takes_the_perc_out_of_the_hook_and_back() {
        on_big_stack(|| {
            let song = songs::night_market();
            let mut l = lab("night-mute");
            let s = &mut l.s;
            open(s, song.name);
            let (b, perc) = (bar(&song), song.track("Perc"));
            launch(s, song.scene("Hook"));
            s.run_for(b);
            s.button("TRACK MUTE");
            s.pad(perc, 100, 0.05);
            assert!(s.time.state().tracks[perc].mute, "pad 2 muted the Perc");
            let a = s.t() + 0.05;
            s.run_for(2.0 * b);
            let quiet = tracks_between(s, a, s.t());
            assert!(!quiet.contains(&perc), "the Perc is quiet");
            assert!(quiet.contains(&song.track("Drums")), "the rest plays on");
            s.pad(perc, 100, 0.05);
            assert!(!s.time.state().tracks[perc].mute, "pad 2 again: back");
            let a = s.t() + 0.05;
            s.run_for(2.0 * b);
            assert!(
                tracks_between(s, a, s.t()).contains(&perc),
                "the Perc is back"
            );
            s.button("TRACK MUTE");
        });
    }

    /// Glass Garden's morph bar: B moved to Bloom, the crossfader at A
    /// sets the lead to Seed's -18 dB, all the way across to Bloom's 0 dB,
    /// and half way back somewhere between.
    #[test]
    fn the_morph_crossfader_brings_the_lead_up_from_seed_to_bloom() {
        on_big_stack(|| {
            use mpfree_fx::ids::mixer as mx;
            use mpfree_fx::StripId;
            let song = songs::glass_garden();
            let mut l = lab("glass-morph");
            let s = &mut l.s;
            open(s, song.name);
            let lead = StripId::Track(song.track("Lead"));
            launch(s, song.scene("Seed"));
            s.run_for(bar(&song));
            let level = |s: &Session| s.mix.mixer.param(lead, mx::VOLUME);
            // B: from row 2 (Grow) up to row 3 (Bloom).
            s.press("morph", "row-up", "B (right half)");
            let fader = s
                .control("morph", "crossfader", "crossfader")
                .expect("a crossfader");
            let slide = |s: &mut Session, dx: i32| {
                let mut t = fader.target.clone();
                t.towards = (dx, 0);
                s.press_target(&t);
                s.run_for(0.5);
            };
            // All the way to A: Seed's level.
            slide(s, -fader.target.rect.w);
            assert!(
                s.lines
                    .iter()
                    .any(|l| l == "time: morph: rows 1 and 3, 1 parameters"),
                "the morph loaded Seed and Bloom: {:?}",
                &s.lines[s.lines.len().saturating_sub(8)..]
            );
            assert!((level(s) - -18.0).abs() < 0.5, "at Seed: {}", level(s));
            // Across to B: Bloom's.
            slide(s, fader.target.rect.w);
            assert!(level(s).abs() < 0.5, "at Bloom: {}", level(s));
            // Half way: in between.
            // The finger down on the middle of the track (and kept there).
            slide(s, 1);
            let mid = level(s);
            assert!(mid < -0.5 && mid > -17.5, "half way: {mid}");
        });
    }

    /// REC while Night Market's Verse plays: a clap hit on the pads goes
    /// into the Drums clip at the step it was played, and plays once a pass
    /// when the Verse plays again; UNDO takes it out.
    #[test]
    fn rec_overdubs_a_clap_into_the_verse_and_it_plays_next_pass() {
        on_big_stack(|| {
            use songs::pad::CLAP;
            let song = songs::night_market();
            let mut l = lab("night-rec");
            let s = &mut l.s;
            open(s, song.name);
            let (b, drums, verse) = (bar(&song), song.track("Drums"), song.scene("Verse"));
            let claps = |s: &Session| {
                s.time.project().tracks[drums].slots[verse]
                    .as_ref()
                    .expect("the Verse's drums")
                    .notes
                    .iter()
                    .filter(|n| n.note == CLAP)
                    .count()
            };
            let before = claps(s);
            // The Drums track armed (the track head toggles it).
            if s.time.armed() != Some(drums) {
                s.press("header", "tab", "Arrange");
                s.press("clips", "track", "Drums");
            }
            assert_eq!(s.time.armed(), Some(drums), "the Drums armed");
            let from = s.t();
            launch(s, verse);
            s.run_for(0.5 * b);
            let t0 = first_note_after(s, from);
            s.button("REC");
            // A clap on the second bar's beat 2 (step 4 of bar 2).
            s.run_for(t0 + 1.25 * b - s.t());
            s.pad(CLAP as usize, 110, 0.05);
            s.run_for(0.5 * b);
            s.button("STOP").run_for(0.2);
            assert_eq!(claps(s), before + 1, "the clap went in");
            let clap = s.time.project().tracks[drums].slots[verse]
                .as_ref()
                .unwrap()
                .notes
                .iter()
                .find(|n| n.note == CLAP)
                .map(|n| n.tick)
                .unwrap();
            assert!(
                (clap - 20 * songs::STEP).abs() <= songs::STEP / 2,
                "on bar 2, beat 2 (step 20): tick {clap}"
            );
            // Played again, the clip plays it once a pass (32 steps).
            let from = s.t();
            launch(s, verse);
            s.run_for(4.5 * b);
            let t1 = first_note_after(s, from);
            let played: Vec<i64> = s
                .hits
                .played
                .iter()
                .filter(|p| p.track == drums && p.note == CLAP && p.at >= t1 - 0.001)
                .map(|p| step_of(&song, t1, p.at))
                .collect();
            assert!(
                played.len() >= 2 && played.windows(2).all(|w| w[1] - w[0] == 32),
                "once a pass: {played:?}"
            );
            s.button("STOP").run_for(0.2);
            s.button("UNDO").run_for(0.2);
            assert_eq!(claps(s), before, "UNDO took it out");
        });
    }

    /// CAPTURE (SHIFT + REC) after jamming over Night Market's Verse with
    /// no REC: the four claps played go into the Verse's Drums clip (the
    /// clip playing on the armed track) at the steps they were played, and
    /// UNDO takes them out again.
    #[test]
    fn capture_keeps_a_jam_over_the_verse() {
        on_big_stack(|| {
            use songs::pad::CLAP;
            let song = songs::night_market();
            let mut l = lab("night-capture");
            let s = &mut l.s;
            open(s, song.name);
            let (b, drums, verse) = (bar(&song), song.track("Drums"), song.scene("Verse"));
            assert_eq!(s.time.armed(), Some(drums), "the Drums armed");
            let claps = |s: &Session| -> Vec<Tick> {
                let mut v: Vec<Tick> = s.time.project().tracks[drums].slots[verse]
                    .as_ref()
                    .expect("the Verse's drums")
                    .notes
                    .iter()
                    .filter(|n| n.note == CLAP)
                    .map(|n| n.tick)
                    .collect();
                v.sort_unstable();
                v
            };
            assert!(claps(s).is_empty());
            let from = s.t();
            launch(s, verse);
            s.run_for(0.25 * b);
            let t0 = first_note_after(s, from);
            // Claps on beats 2 and 4 of bars 2 and 3, as a player would.
            for k in [1.0, 3.0, 5.0, 7.0] {
                s.run_for(t0 + 1.0 * b + k * b / 4.0 - s.t());
                s.pad(CLAP as usize, 100, 0.05);
            }
            s.run_for(0.5 * b);
            s.button_with("SHIFT", "REC");
            assert!(
                s.lines
                    .iter()
                    .any(|l| l.starts_with("time: capture 1: 4 notes")),
                "{:?}",
                &s.lines[s.lines.len().saturating_sub(4)..]
            );
            // Bars 2 and 3 of a two-bar clip: steps 20 and 28, then
            // (bar 3 is the clip's bar 1 again) steps 4 and 12.
            let steps: Vec<Tick> = claps(s)
                .iter()
                .map(|t| (t + songs::STEP / 2) / songs::STEP)
                .collect();
            assert_eq!(steps, [4, 12, 20, 28], "on the beats played");
            s.button("STOP").run_for(0.2);
            s.button("UNDO").run_for(0.2);
            assert!(claps(s).is_empty(), "UNDO took the capture out");
        });
    }

    /// Tidal Suite has no song yet: Scenes → song lays its five rows out
    /// one after another, a marker on each named after its scene, and
    /// Play song plays each section's tracks; saved and opened again, the
    /// song is still there.
    #[test]
    fn tidal_suite_becomes_a_song_from_its_scenes_and_keeps_it() {
        on_big_stack(|| {
            let song = songs::tidal_suite();
            let mut l = lab("tidal-song");
            let s = &mut l.s;
            open(s, song.name);
            assert!(s.time.project().arrangement.placements.is_empty());
            s.press("header", "tab", "Arrange");
            s.press("clips", "view", "Song");
            s.press("song", "button", "Scenes → song");
            let arr = s.time.project().arrangement.clone();
            let names: Vec<&str> = arr.markers.iter().map(|m| m.name.as_str()).collect();
            let scenes: Vec<&str> = song.clips.scenes.iter().map(|c| c.name.as_str()).collect();
            assert_eq!(names, scenes, "a marker a scene, in order");
            s.press("song", "button", "Play song");
            let from = s.t();
            s.press("transport", "play", "Play");
            let end = arr.end() / songs::BAR;
            s.run_for((end as f64 + 0.5) * bar(&song));
            let t0 = first_note_after(s, from);
            for (i, m) in arr.markers.iter().enumerate() {
                let a = m.at / songs::BAR;
                let z = arr.markers.get(i + 1).map_or(end, |n| n.at / songs::BAR);
                let row = song.scene(&m.name);
                let want: BTreeSet<&'static str> = song
                    .clips
                    .tracks
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| t.slots[row].is_some())
                    .map(|(k, _)| name_of(&song, k))
                    .collect();
                // Undertow B is one bar: the window runs from just before
                // its bar line to just before the next section's.
                let bb = bar(&song);
                let got: BTreeSet<&'static str> =
                    tracks_between(s, t0 + (a as f64 - 0.02) * bb, t0 + (z as f64 - 0.02) * bb)
                        .into_iter()
                        .map(|t| name_of(&song, t))
                        .collect();
                assert_eq!(got, want, "{}", m.name);
            }
            s.button("STOP");
            s.button_with("SHIFT", "BROWSE");
            let saved = format!("project: saved {}", song.name);
            assert!(s.until_said(&saved, 5.0), "{saved}");
            open(s, song.name);
            assert_eq!(s.time.project().arrangement, arr, "the song came back");
        });
    }

    // ------------------------------------------------------- exporting

    /// A WAV file the export wrote (24-bit PCM): its frames, interleaved
    /// stereo, and its rate.
    fn wav(path: &std::path::Path) -> (Vec<f32>, u32) {
        let b = std::fs::read(path).expect("the WAV file");
        assert_eq!(&b[..4], b"RIFF");
        assert_eq!(&b[8..12], b"WAVE");
        let (mut at, mut rate, mut bits, mut data) = (12, 0, 0, None);
        while at + 8 <= b.len() {
            let len = u32::from_le_bytes([b[at + 4], b[at + 5], b[at + 6], b[at + 7]]) as usize;
            let body = &b[at + 8..(at + 8 + len).min(b.len())];
            match &b[at..at + 4] {
                b"fmt " => {
                    assert_eq!(u16::from_le_bytes([body[2], body[3]]), 2, "stereo");
                    rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
                    bits = u16::from_le_bytes([body[14], body[15]]);
                }
                b"data" => data = Some(body),
                _ => {}
            }
            at += 8 + len + (len & 1);
        }
        assert_eq!(bits, 24, "24-bit");
        let audio = data
            .expect("a data chunk")
            .chunks_exact(3)
            .map(|x| (i32::from_le_bytes([0, x[0], x[1], x[2]]) >> 8) as f32 / 8_388_608.0)
            .collect();
        (audio, rate)
    }

    /// The loudest sample between `a` and `b` seconds of interleaved
    /// stereo at `rate`.
    fn peak(audio: &[f32], rate: u32, a: f64, b: f64) -> f32 {
        let at = |t: f64| ((t * f64::from(rate)) as usize * 2).min(audio.len());
        audio[at(a)..at(b)].iter().fold(0f32, |m, x| m.max(x.abs()))
    }

    /// A type 1 MIDI file's tracks after the conductor: each one's name
    /// and its note-ons' ticks.
    fn midi(path: &std::path::Path) -> Vec<(String, Vec<u64>)> {
        let b = std::fs::read(path).expect("the MIDI file");
        assert_eq!(&b[..4], b"MThd");
        assert_eq!(u16::from_be_bytes([b[8], b[9]]), 1, "type 1");
        assert_eq!(
            u16::from_be_bytes([b[12], b[13]]),
            960,
            "960 ticks a quarter"
        );
        let mut at = 14;
        let mut tracks = Vec::new();
        while at + 8 <= b.len() {
            assert_eq!(&b[at..at + 4], b"MTrk");
            let len = u32::from_be_bytes([b[at + 4], b[at + 5], b[at + 6], b[at + 7]]) as usize;
            let t = &b[at + 8..at + 8 + len];
            let (mut i, mut tick, mut name, mut ons) = (0, 0u64, String::new(), Vec::new());
            let var = |i: &mut usize| {
                let mut v = 0u64;
                loop {
                    let c = t[*i];
                    *i += 1;
                    v = (v << 7) | u64::from(c & 0x7f);
                    if c & 0x80 == 0 {
                        return v;
                    }
                }
            };
            while i < t.len() {
                tick += var(&mut i);
                let status = t[i];
                i += 1;
                match status {
                    0xff => {
                        let kind = t[i];
                        i += 1;
                        let n = var(&mut i) as usize;
                        if kind == 0x03 {
                            name = String::from_utf8_lossy(&t[i..i + n]).into_owned();
                        }
                        i += n;
                    }
                    0xf0 | 0xf7 => {
                        let n = var(&mut i) as usize;
                        i += n;
                    }
                    _ => {
                        let n = if matches!(status & 0xf0, 0xc0 | 0xd0) {
                            1
                        } else {
                            2
                        };
                        if status & 0xf0 == 0x90 && t[i + 1] > 0 {
                            ons.push(tick);
                        }
                        i += n;
                    }
                }
            }
            tracks.push((name, ons));
            at += 8 + len;
        }
        tracks.remove(0); // the conductor
        tracks
    }

    /// BROWSE > Projects > Export, then Start on its panel (with
    /// `untick`'s boxes unticked first): the export runs in fast mode and
    /// says how it went. Returns that line and the folder in Recordings.
    /// Export from the browser, with the panel's buttons in `untick`
    /// pressed first (a tick taken off, or "Track" for stems by track).
    fn export(s: &mut Session, untick: &[&str], within: f64) -> (String, std::path::PathBuf) {
        if s.app.view() != crate::app::View::Browser {
            s.button("BROWSE");
        }
        s.press("browser", "place", "Projects");
        s.press("browser", "button", "Export");
        for label in untick {
            s.press("browser", "export", label);
        }
        let before = s.said().len();
        s.press("browser", "export", "Start");
        let line = s
            .until_said_like("export: Export", within)
            .unwrap_or_else(|| panic!("the export ended: {:?}", &s.said()[before..]));
        // "Exported 1:55 to <folder>: ..."
        let name = line
            .split_once(" to ")
            .and_then(|(_, r)| r.split_once(": "))
            .map(|(n, _)| n.to_string())
            .expect("the export said its folder");
        let dir = s
            .library
            .as_ref()
            .unwrap()
            .root()
            .join(mpfree_library::Place::Recordings.folder())
            .join(&name);
        (line, dir)
    }

    /// Night Market's whole arrangement exported in fast mode: a mixdown,
    /// a stem for each track (every track here plays the pads, so each
    /// stem is a replay of the range with only that track playing), one
    /// for the return its reverb sends to, and a MIDI file, in a new folder
    /// in Recordings. The mixdown runs the song's 44 bars (and a short
    /// tail); each track's stem is as long, and silent where the track
    /// isn't placed; in the MIDI file each track has its notes as they
    /// played, only in the sections it is placed in; and afterwards the
    /// transport is back as it was, song mode off.
    #[test]
    fn night_market_exports_its_arrangement_as_mixdown_stems_and_midi() {
        on_big_stack(|| {
            let song = songs::night_market();
            let mut l = lab("night-export");
            let s = &mut l.s;
            s.make_sounds();
            open(s, song.name);
            assert!(!s.time.song_on());
            let played_from = s.hits.played.len();
            // Stems by track (the default is by channel): the first pass
            // and a replay for each of the five tracks.
            let (line, dir) = export(s, &["Track"], 900.0);
            assert!(
                line.starts_with("export: Exported ") && line.contains("a MIDI file"),
                "{line}"
            );
            let names: BTreeSet<String> = std::fs::read_dir(&dir)
                .expect("the export's folder")
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            let want: BTreeSet<String> = [
                "Mixdown.wav",
                "Drums.wav",
                "Perc.wav",
                "Bass.wav",
                "Keys.wav",
                "Chops.wav",
                "Return A.wav",
                "Night Market.mid",
            ]
            .into_iter()
            .map(String::from)
            .collect();
            assert_eq!(names, want);

            // The mixdown: 44 bars, then a short tail (the tones die in a
            // tenth of a second; the reverb a little later).
            let b = bar(&song);
            let (mix, rate) = wav(&dir.join("Mixdown.wav"));
            assert_eq!(rate, crate::RATE);
            let secs = mix.len() as f64 / 2.0 / f64::from(rate);
            assert!(secs >= 44.0 * b - 0.01 && secs < 44.0 * b + 5.0, "{secs} s");
            for k in 0..44 {
                let k = f64::from(k);
                assert!(
                    peak(&mix, rate, k * b, (k + 1.0) * b) > 0.02,
                    "bar {} sounds",
                    k + 1.0
                );
            }
            // Each track's stem: as long as the mixdown, sounding, and
            // silent in a bar its track isn't placed in (nor in the bar
            // before, for its tail).
            let arr = &song.clips.arrangement;
            for name in ["Drums", "Perc", "Bass", "Keys", "Chops"] {
                let t = song.track(name);
                let (stem, _) = wav(&dir.join(format!("{name}.wav")));
                assert_eq!(
                    stem.len(),
                    mix.len(),
                    "{name}'s stem is as long as the mixdown"
                );
                assert!(
                    peak(&stem, rate, 0.0, 44.0 * b) > 0.02,
                    "{name}'s stem sounds"
                );
                let placed = |k: i64| {
                    arr.placements.iter().any(|p| {
                        p.track == t
                            && p.start < (k + 1) * songs::BAR
                            && p.start + p.length > k * songs::BAR
                    })
                };
                for k in 0..44i64 {
                    if !placed(k) && !placed(k - 1) {
                        let (x, y) = (k as f64 * b, (k + 1) as f64 * b);
                        assert!(
                            peak(&stem, rate, x, y) < 1e-3,
                            "{name}'s stem is silent in bar {}",
                            k + 1
                        );
                    }
                }
            }

            // The MIDI file: a track per track, each with as many notes as
            // played in the first pass (each track played again in its
            // replay), only in the sections it is placed in.
            let tracks = midi(&dir.join("Night Market.mid"));
            let in_file: BTreeSet<&str> = tracks.iter().map(|(n, _)| n.as_str()).collect();
            assert_eq!(in_file, set(&["Drums", "Perc", "Bass", "Keys", "Chops"]));
            let arr = &song.clips.arrangement;
            for (name, ons) in &tracks {
                let t = song.track(name);
                let played = s.hits.played[played_from..]
                    .iter()
                    .filter(|p| p.track == t)
                    .count();
                assert_eq!(2 * ons.len(), played, "{name}'s notes");
                for &k in ons {
                    let k = k as i64;
                    assert!(
                        arr.placements
                            .iter()
                            .any(|p| p.track == t && p.start <= k && k < p.start + p.length),
                        "{name}'s note at tick {k} is inside one of its placements"
                    );
                }
            }

            // The transport as it was.
            assert!(!s.time.song_on(), "song mode off again");
            assert!(!s.time.info().state.rolling(), "stopped");
        });
    }

    /// Tidal Suite has no arrangement, so the panel offers bars of what
    /// plays now: with the Ebb launched, 8 bars exported with Stems
    /// unticked make only the mixdown and the MIDI file, 8 bars long and
    /// a short tail, with no note after the 8 bars.
    #[test]
    fn tidal_suite_exports_eight_bars_of_the_ebb_without_stems() {
        on_big_stack(|| {
            let song = songs::tidal_suite();
            let mut l = lab("tidal-export");
            let s = &mut l.s;
            s.make_sounds();
            open(s, song.name);
            launch(s, song.scene("Ebb"));
            s.run_for(0.5);
            s.button("STOP").run_for(0.2);
            let (line, dir) = export(s, &["Stems"], 120.0);
            assert!(
                line.contains("1 WAV file (24-bit, 44.1 kHz) and a MIDI file"),
                "{line}"
            );
            let names: BTreeSet<String> = std::fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            assert_eq!(names.len(), 2, "{names:?}");
            assert!(names.contains("Mixdown.wav"));
            let (mix, rate) = wav(&dir.join("Mixdown.wav"));
            let b = bar(&song);
            let secs = mix.len() as f64 / 2.0 / f64::from(rate);
            assert!(secs >= 8.0 * b - 0.01 && secs < 8.0 * b + 5.0, "{secs} s");
            assert!(peak(&mix, rate, 0.0, 8.0 * b) > 0.05, "it sounds");
            let mid = names.iter().find(|n| n.ends_with(".mid")).unwrap();
            let tracks = midi(&dir.join(mid));
            assert!(
                tracks
                    .iter()
                    .any(|(n, ons)| n == "Drums" && !ons.is_empty()),
                "the Ebb's drums in the MIDI file: {tracks:?}"
            );
            // The 8 bars and nothing after: no note past the range, and
            // the tail only rings out (the tones die in a tenth of a
            // second).
            assert!(
                tracks
                    .iter()
                    .all(|(_, ons)| ons.iter().all(|&k| k < 8 * songs::BAR as u64)),
                "no note past the 8 bars"
            );
            assert!(
                peak(&mix, rate, 8.0 * b + 1.0, secs) < 0.01,
                "the tail rings out"
            );
        });
    }

    /// The MIDI file an export makes loads back (BROWSE > Recordings, its
    /// folder, the file): in a new project its tracks land on the tracks
    /// from the armed one, and the drums on the pads they played on, as the
    /// MPC's own MIDI export comes in.
    #[test]
    fn an_exported_midi_file_loads_back_onto_the_pads_it_played() {
        on_big_stack(|| {
            let song = songs::tidal_suite();
            let mut l = lab("midi-back");
            let s = &mut l.s;
            s.make_sounds();
            open(s, song.name);
            launch(s, song.scene("Ebb"));
            s.run_for(0.5);
            s.button("STOP").run_for(0.2);
            let (_, dir) = export(s, &["Stems"], 120.0);
            let folder = dir.file_name().unwrap().to_string_lossy().into_owned();
            let mid = std::fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .find(|n| n.ends_with(".mid"))
                .expect("a MIDI file");
            let in_file = midi(&dir.join(&mid));
            let drum_ons = in_file
                .iter()
                .find(|(n, _)| n == "Drums")
                .map(|(_, ons)| ons.len())
                .expect("the drums in the file");
            let t = song.track("Drums");
            // The Ebb's drum clip: the one that played.
            let played: BTreeSet<u8> = song.clips.tracks[t].slots[song.scene("Ebb")]
                .iter()
                .flat_map(|c| c.notes.iter().map(|n| n.note))
                .collect();

            // The Export panel, over: - closes it, back to the browser.
            s.button("-");
            if s.app.view() != crate::app::View::Browser {
                s.button("BROWSE");
            }
            s.press("browser", "place", "Projects");
            s.press("browser", "button", "New project");
            // Over the launched scene, it asks first.
            if s.control("browser", "confirm-yes", "").is_some() {
                s.press("browser", "confirm-yes", "");
            }
            s.run_for(0.3);
            assert!(
                s.time.project().tracks.iter().all(|t| t.slots.is_empty()),
                "New project is empty"
            );
            if s.app.view() != crate::app::View::Browser {
                s.button("BROWSE");
            }
            s.press("browser", "place", "Recordings");
            s.press("browser", "row", &folder);
            let said = |s: &Session| {
                s.said()
                    .into_iter()
                    .filter(|l| l.starts_with(&format!("MIDI file {mid}")))
                    .collect::<Vec<_>>()
            };
            let wait = |s: &mut Session, secs: u64| {
                let wall = std::time::Instant::now() + std::time::Duration::from_secs(secs);
                while said(s).is_empty() && std::time::Instant::now() < wall {
                    s.run_for(0.05);
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            };
            s.press("browser", "row", &mid);
            wait(s, 1);
            if said(s).is_empty() {
                s.press("browser", "row", &mid);
                wait(s, 5);
            }
            let lines = said(s);
            assert_eq!(lines.len(), 1, "loaded once: {lines:?}");
            assert!(lines[0].contains("from slot 1 of Track 1"), "{}", lines[0]);
            let drums = s
                .time
                .project()
                .tracks
                .iter()
                .flat_map(|t| t.slots.clips())
                .find(|c| c.name == "Drums")
                .cloned()
                .expect("the drums came back as a clip");
            assert_eq!(drums.notes.len(), drum_ons, "every drum note on a pad");
            let pads: BTreeSet<u8> = drums.notes.iter().map(|n| n.note).collect();
            assert_eq!(pads, played, "on the pads they played on");
            assert_eq!(drums.length, 8 * songs::BAR, "the 8 bars exported");
        });
    }

    /// Sampler step 15: a kit the project chose for the pads plays on its
    /// own, before the arp and the recorder, while the selected track plays
    /// another kit; Follow track gives the pads back. The Sample page
    /// follows the hit to the kit it played (step 15d; the screen test
    /// `the_sample_page_follows_a_hit_to_the_kit_the_pads_play_and_says_where_send_lands`
    /// has it back on the track's kit after).
    #[test]
    fn a_kit_chosen_for_the_pads_plays_on_its_own_and_follow_track_gives_them_back() {
        on_big_stack(|| {
            let mut session = Session::new();
            let s = &mut session;
            s.make_sounds();
            // The app has the list of instruments once the loop has run.
            s.run_periods(2);
            // The pads part as a project saved with the pads on kit 2.
            let kit_2 = s.instrument_list.of_kit(1).expect("kit 2 has an id");
            let part = format!("mpfree-pads 2\nplay instrument {kit_2}\n");
            let (notes, fx) = s.app.open_pads(Some(&part));
            assert!(notes.is_empty(), "{notes:?}");
            s.fx.extend(fx);
            s.run_periods(4);
            assert_eq!(s.app.pads_kit(), Some(1), "the pads play kit 2");
            assert_eq!(
                s.track_kits.kit_of(s.app.track()),
                Some(0),
                "the track kit 1"
            );
            // A pad of kit 2's with a sound, and the same pad on kit 1.
            let player = s.sounds.as_ref().expect("the kits");
            let pad = (0..16)
                .find(|&p| player.has_sound(1, p) && player.has_sound(0, p))
                .expect("a pad both kits fill");
            s.previewed.clear();
            s.pad_edge(pad, 100, true);
            s.run_periods(2);
            let player = s.sounds.as_ref().expect("the kits");
            assert!(player.kit_pad_voices(1, pad) > 0, "kit 2's pad plays");
            assert_eq!(player.kit_pad_voices(0, pad), 0, "kit 1's doesn't");
            assert_eq!(s.previewed.len(), 1, "on its own, before the recorder");
            // Step 15d: the Sample page edits the pad hit, on kit 2.
            assert_eq!(s.screen.sample.edit_kit(), 1, "the Sample page shows kit 2");
            assert_eq!(s.screen.sample.pad(), pad, "at the pad hit");
            s.pad_edge(pad, 0, false);
            s.run_for(2.0);

            // Follow track: the pads play the track's kit 1 again.
            s.fx.extend(s.app.pads_follow_track());
            s.run_periods(4);
            assert_eq!(s.app.pads_kit(), None);
            s.previewed.clear();
            s.pad_edge(pad, 100, true);
            s.run_periods(2);
            let player = s.sounds.as_ref().expect("the kits");
            assert!(player.kit_pad_voices(0, pad) > 0, "kit 1's pad plays");
            assert!(s.previewed.is_empty(), "through the track");
            s.pad_edge(pad, 0, false);
        });
    }

    /// Sampler step 15e, the engine half: a bank's own kit from another of
    /// its banks plays that bank's pads. Bank D on kit 2 from A plays kit
    /// 2's A pads, on its own while the track plays kit 1, and the Sample
    /// page edits them; bank D on the track's kit 1 from A plays kit 1's A
    /// pads through the track, so the track hears (and REC would record)
    /// the A pad. Back on bank A, the pads follow the track as before.
    #[test]
    fn a_banks_own_kit_from_another_bank_plays_that_banks_pads() {
        on_big_stack(|| {
            let mut session = Session::new();
            let s = &mut session;
            s.make_sounds();
            s.run_periods(2);
            let kit_2 = s.instrument_list.of_kit(1).expect("kit 2 has an id");
            let part = format!("mpfree-pads 2\nplay track\nbank D instrument {kit_2} from A\n");
            let (notes, fx) = s.app.open_pads(Some(&part));
            assert!(notes.is_empty(), "{notes:?}");
            s.fx.extend(fx);
            s.run_periods(2);
            s.button("PAD BANK D/H");
            s.run_periods(4);
            assert_eq!((s.app.pad_page(), s.app.kit_page()), (3, 0));
            assert_eq!(s.app.pads_kit(), Some(1), "bank D plays kit 2");
            let player = s.sounds.as_ref().expect("the kits");
            let pad = (0..16)
                .find(|&p| player.has_sound(1, p))
                .expect("a pad of kit 2's bank A");
            s.pad_edge(pad, 100, true);
            s.run_periods(2);
            let player = s.sounds.as_ref().expect("the kits");
            assert!(player.kit_pad_voices(1, pad) > 0, "kit 2's A pad plays");
            assert_eq!(player.kit_pad_voices(1, 48 + pad), 0, "not its D pad");
            assert_eq!(player.kit_pad_voices(0, 48 + pad), 0, "nor kit 1's");
            assert_eq!(
                (s.screen.sample.edit_kit(), s.screen.sample.pad()),
                (1, pad),
                "the Sample page edits kit 2's A pad"
            );
            s.pad_edge(pad, 0, false);
            s.run_for(1.0);

            // Bank D on the track's own kit from A: through the track, as
            // the A pad.
            let kit_1 = s.instrument_list.of_kit(0).expect("kit 1 has an id");
            let part = format!("mpfree-pads 2\nplay track\nbank D instrument {kit_1} from A\n");
            let (notes, fx) = s.app.open_pads(Some(&part));
            assert!(notes.is_empty(), "{notes:?}");
            s.fx.extend(fx);
            s.run_periods(4);
            assert_eq!(s.app.pads_kit(), Some(0));
            assert_eq!(s.app.kit_page(), 0);
            let player = s.sounds.as_ref().expect("the kits");
            let pad = (0..16)
                .find(|&p| player.has_sound(0, p))
                .expect("a pad of kit 1's bank A");
            s.hits.on.clear();
            s.previewed.clear();
            s.pad_edge(pad, 100, true);
            s.run_periods(2);
            assert!(s.previewed.is_empty(), "through the track");
            assert!(
                s.hits.on.iter().any(|&(p, _)| p == pad),
                "the track plays the A pad: {:?}",
                s.hits.on
            );
            assert!(s.hits.on.iter().all(|&(p, _)| p < 16), "{:?}", s.hits.on);
            s.pad_edge(pad, 0, false);
            s.run_periods(2);

            // Bank A has no entry of its own: the pads follow the track.
            s.button("PAD BANK A/E");
            s.run_periods(2);
            assert_eq!((s.app.pad_page(), s.app.kit_page()), (0, 0));
        });
    }

    /// Sampler step 15f, first part: a kit as a layer. With kit 2 on the
    /// pads and the track's kit 1 as a layer from bank A, a hit plays kit
    /// 2's pad and kit 1's A pad, on bank B too, and its let-go reaches
    /// what it played. REC plans a track for the kit layer.
    #[test]
    fn a_kit_layer_plays_its_bank_with_every_hit() {
        on_big_stack(|| {
            let mut session = Session::new();
            let s = &mut session;
            s.make_sounds();
            s.run_periods(2);
            let kit_1 = s.instrument_list.of_kit(0).expect("kit 1 has an id");
            let kit_2 = s.instrument_list.of_kit(1).expect("kit 2 has an id");
            let part = format!(
                "mpfree-pads 2\nplay instrument {kit_2}\nlayer instrument {kit_1} from A\n"
            );
            let (notes, fx) = s.app.open_pads(Some(&part));
            assert!(notes.is_empty(), "{notes:?}");
            s.fx.extend(fx);
            s.run_periods(4);
            assert_eq!(s.app.pads_kit(), Some(1), "the pads play kit 2");
            assert_eq!(s.app.pads_layer_kits()[0], Some((0, 0)), "kit 1's bank A");
            let player = s.sounds.as_ref().expect("the kits");
            let pad = (0..16)
                .find(|&p| player.has_sound(1, p) && player.has_sound(0, p))
                .expect("a pad both kits fill");
            s.pad_edge(pad, 100, true);
            s.run_periods(2);
            let player = s.sounds.as_ref().expect("the kits");
            assert!(player.kit_pad_voices(1, pad) > 0, "kit 2's pad plays");
            assert!(player.kit_pad_voices(0, pad) > 0, "and the layer's");
            assert_eq!(s.pad_layers.kit_pads_held(pad)[0], Some((0, pad as u8)));
            s.pad_edge(pad, 0, false);
            s.run_periods(2);
            assert_eq!(s.pad_layers.kit_pads_held(pad), [None; 3], "let go there");
            s.run_for(2.0);

            // On bank B the layer still plays kit 1's bank A.
            s.button("PAD BANK B/F");
            s.run_periods(4);
            assert_eq!(s.app.pads_layer_kits()[0], Some((0, 0)));
            s.pad_edge(pad, 100, true);
            s.run_periods(2);
            let player = s.sounds.as_ref().expect("the kits");
            assert!(player.kit_pad_voices(0, pad) > 0, "kit 1's A pad");
            assert_eq!(player.kit_pad_voices(0, 16 + pad), 0, "not its B pad");
            s.pad_edge(pad, 0, false);
            s.run_periods(2);

            // REC records the kit layer too, on a track of its own that
            // plays kit 1, each hit as kit 1's pad.
            let more = s.app.rec_more_sounds();
            assert!(
                more.iter()
                    .flatten()
                    .any(|m| m.sound == crate::app::RecSound::Kit(0)
                        && format!("{:?}", m.from) == "Layer(0)"),
                "{more:?}"
            );
            s.button("PAD BANK A/E");
            s.run_periods(4);
            s.button("REC").settle();
            s.run_periods(4).settle();
            let rec = s.app.pads_layer_recs(s.time.also())[0];
            let Some((t, kit)) = rec else {
                panic!("no track records the kit layer: {:?}", s.said());
            };
            assert_eq!(kit, Some((0, 0)), "kit 1's bank A");
            assert_eq!(
                s.track_kits.kit_of(usize::from(t)),
                Some(0),
                "it plays kit 1"
            );
            assert_ne!(s.time.armed(), Some(usize::from(t)), "a track of its own");
        });
    }

    /// Sampler step 15f, part 2: a kit layer plays each key too, as its
    /// kit's Keys play sends it: on a kit of drum pads, the pad that sends
    /// the note (the MPC note map, whatever bank the pads show), and the
    /// key's let-go reaches it there. A kit layer that is the kit the
    /// armed track's keys play takes none of them, so a key never plays a
    /// kit twice.
    #[test]
    fn a_kit_layer_plays_each_key_as_its_kit_takes_it() {
        use crate::instrument_list::InstrumentId;
        on_big_stack(|| {
            let mut session = Session::new();
            let s = &mut session;
            s.make_sounds();
            s.run_periods(2);
            let kit_1 = s.instrument_list.of_kit(0).expect("kit 1 has an id");
            let kit_2 = s.instrument_list.of_kit(1).expect("kit 2 has an id");
            let synth = InstrumentId::SHARED_SYNTH;
            let part =
                format!("mpfree-pads 2\nplay instrument {synth}\nlayer instrument {kit_2}\n");
            let (notes, fx) = s.app.open_pads(Some(&part));
            assert!(notes.is_empty(), "{notes:?}");
            s.fx.extend(fx);
            s.run_periods(4);
            assert_eq!(s.app.pads_layer_kits()[0], Some((1, 0)), "kit 2's bank A");
            let player = s.sounds.as_ref().expect("the kits");
            let pad = (0..16)
                .find(|&p| player.has_sound(1, p))
                .expect("a pad kit 2 fills");
            let note = mpfree_midi::profile::kit_note(pad).expect("its note");
            s.key_edge(note, Some(100));
            s.run_periods(2);
            let player = s.sounds.as_ref().expect("the kits");
            assert!(player.kit_pad_voices(1, pad) > 0, "kit 2's pad plays");
            assert_eq!(s.pad_layers.kit_keys_held(), 1, "the key is held there");
            s.key_edge(note, None);
            s.run_periods(2);
            assert_eq!(s.pad_layers.kit_keys_held(), 0, "let go there");
            s.run_for(2.0);

            // A layer on the kit the armed track plays leaves the key to
            // the track.
            let part =
                format!("mpfree-pads 2\nplay instrument {kit_2}\nlayer instrument {kit_1}\n");
            let (notes, fx) = s.app.open_pads(Some(&part));
            assert!(notes.is_empty(), "{notes:?}");
            s.fx.extend(fx);
            s.run_periods(4);
            assert_eq!(s.app.pads_layer_kits()[0], Some((0, 0)), "kit 1's bank A");
            s.key_edge(note, Some(100));
            s.run_periods(2);
            assert_eq!(s.pad_layers.kit_keys_held(), 0, "the track has the key");
            s.key_edge(note, None);
            s.run_periods(2);
        });
    }

    /// Sampler step 15f, part 3: a MIDI route can name a kit. Its keys
    /// play the kit as its Keys play sends them (on a kit of drum pads,
    /// the pad that sends the note, any bank), whichever track is armed,
    /// and let go there. REC records the kit on a track of its own,
    /// pointed at it, each key as that kit's pad.
    #[test]
    fn a_route_that_names_a_kit_plays_it_and_rec_records_it_on_its_own_track() {
        use mpfree_transport::State;
        fn notes(s: &Session, t: usize) -> Vec<u8> {
            let mut n: Vec<u8> = s.time.project().tracks[t]
                .slots
                .clips()
                .flat_map(|c| c.notes.iter().map(|n| n.note))
                .collect();
            n.sort_unstable();
            n.dedup();
            n
        }
        on_big_stack(|| {
            let mut session = Session::new();
            let s = &mut session;
            s.make_sounds();
            s.run_periods(2);
            let kit_2 = s.instrument_list.of_kit(1).expect("kit 2 has an id");
            s.name_route_synths([Some(kit_2.number()), None, None, None]);
            s.run_periods(2);
            assert_eq!(
                s.app.route_synth_kits()[0],
                Some(1),
                "the route names kit 2"
            );
            let player = s.sounds.as_ref().expect("the kits");
            let pad = (0..16)
                .find(|&p| player.has_sound(1, p))
                .expect("a pad kit 2 fills");
            let note = mpfree_midi::profile::kit_note(pad).expect("its note");
            s.route_key(0, note, Some(100));
            s.run_periods(2);
            let player = s.sounds.as_ref().expect("the kits");
            assert!(player.kit_pad_voices(1, pad) > 0, "kit 2's pad plays");
            assert_eq!(s.route_synths.kit_keys_held(), 1, "the key is held there");
            s.route_key(0, note, None);
            assert_eq!(s.route_synths.kit_keys_held(), 0, "let go there");
            s.run_for(1.0);

            // REC: a track of its own on kit 2, armed with Track 1.
            let more = s.app.rec_more_sounds();
            assert!(
                more.iter()
                    .flatten()
                    .any(|m| m.sound == crate::app::RecSound::Kit(1)
                        && format!("{:?}", m.from) == "Route(0)"),
                "{more:?}"
            );
            s.route(crate::gui::FromUi::Time(
                mpfree_time_ui::TimeAction::Transport(mpfree_transport::Command::SetCountIn(0)),
            ));
            s.button("REC").settle();
            s.run_periods(4).settle();
            assert_eq!(s.time.armed(), Some(0), "{:?}", s.said());
            assert_eq!(s.track_kits.kit_of(1), Some(1), "Track 2 plays kit 2");
            assert_eq!(s.time.also(), 1 << 1, "Track 2 armed too");
            assert_eq!(s.app.route_recs(s.time.also())[0], Some(1));
            s.button("PLAY");
            let end = s.t() + 4.0;
            while s.time.info().state != State::Recording && s.t() < end {
                s.run_periods(1);
            }
            assert_eq!(s.time.info().state, State::Recording);
            s.route_key(0, note, Some(100));
            s.run_for(0.1);
            s.route_key(0, note, None);
            s.run_for(0.1);
            s.button("REC");
            s.run_for(0.5);
            assert_eq!(notes(s, 1), [pad as u8], "kit 2's track has the pad");
            assert!(notes(s, 0).is_empty(), "Track 1 keeps out the key");
        });
    }

    /// Sampler step 16: SHIFT + a touch on a pad's setting learns the kit
    /// the Sample page shows by its number in the list, not the track; a
    /// touch on a synth knob learns the instrument the Synth tab shows
    /// (the shared synth here). The same touch again cancels.
    #[test]
    fn a_touch_learns_the_kit_and_the_synth_the_screen_shows() {
        use crate::gui::{FromUi, LearnValue};
        use mpfree_play::mpfree_param::{Owner, ParamAddr};
        on_big_stack(|| {
            let mut session = Session::new();
            let s = &mut session;
            s.make_sounds();
            s.run_periods(2);
            let kit_2 = s.instrument_list.of_kit(1).expect("kit 2 has an id");
            let n = u16::try_from(kit_2.number()).unwrap();
            assert_eq!(
                s.track_kits.kit_of(s.app.track()),
                Some(0),
                "the track plays kit 1"
            );
            let pad = LearnValue::Pad {
                kit: 1,
                pad: 4,
                field: 2,
            };
            s.route(FromUi::LearnValue(pad));
            assert_eq!(
                s.music.armed(),
                Some(ParamAddr::new(Owner::KitPad(n, 4), 2))
            );
            s.route(FromUi::LearnValue(pad));
            assert_eq!(s.music.armed(), None, "the same touch cancels");
            s.route(FromUi::LearnValue(LearnValue::Synth(7)));
            assert_eq!(
                s.music.armed(),
                Some(ParamAddr::new(Owner::Sound(1), 7)),
                "the shared synth, instrument 1"
            );
        });
    }

    /// Sampler step 15d, part 2: a kit no track plays gets a strip of its
    /// own. With kit 2 on the pads and no track playing it, it sounds on a
    /// kit strip named for it, shown with the tracks, not on the pad
    /// channels kit 1 has.
    #[test]
    fn the_pads_own_kit_sounds_on_a_kit_strip_of_its_own() {
        use mpfree_fx::StripId;
        on_big_stack(|| {
            let mut session = Session::new();
            let s = &mut session;
            s.make_sounds();
            s.run_periods(2);
            assert_eq!(s.mix.kit_strips(), [None; crate::mixing::KIT_STRIPS]);
            let kit_2 = s.instrument_list.of_kit(1).expect("kit 2 has an id");
            let part = format!("mpfree-pads 2\nplay instrument {kit_2}\n");
            let (notes, fx) = s.app.open_pads(Some(&part));
            assert!(notes.is_empty(), "{notes:?}");
            s.fx.extend(fx);
            s.run_periods(4);
            assert_eq!(s.app.pads_kit(), Some(1), "the pads play kit 2");
            assert_eq!(s.mix.kit_strip(1), Some(0), "on the first kit strip");
            assert_eq!(s.mix.kit_strip(0), None, "kit 1 keeps the pad channels");
            let strip = StripId::Track(crate::mixing::kit_channel(0));
            assert_eq!(s.mix.strip_name(strip), "Kit 2");
            assert!(s.mix.track_strips().contains(&strip));
            let player = s.sounds.as_ref().expect("the kits");
            let pad = (0..16)
                .find(|&p| player.has_sound(1, p))
                .expect("a pad kit 2 fills");
            let _ = s.mix.mixer.take_input_peaks(strip);
            let _ = s.mix.mixer.take_input_peaks(StripId::Pad(pad));
            s.pad_edge(pad, 100, true);
            s.run_periods(4);
            assert!(
                s.mix.mixer.take_input_peaks(strip)[0] > 0.0,
                "kit 2's pad is heard on its strip"
            );
            assert_eq!(
                s.mix.mixer.take_input_peaks(StripId::Pad(pad)),
                [0.0; 2],
                "not on kit 1's pad channel"
            );
            s.pad_edge(pad, 0, false);
            s.run_periods(2);
        });
    }

    /// Sampler step 15c: NOTE REPEAT while the pads play a kit on their
    /// own repeats that kit's pad (the pads' arp), never the track's, and
    /// the pads keep the kit; let go, the repeats stop. Held again, Follow
    /// track ends what the arp has on there, the pad's let-go is taken
    /// quietly, and the next hit plays the track's kit.
    #[test]
    fn note_repeat_on_a_kit_the_pads_play_on_their_own_repeats_it_and_keeps_the_pads() {
        use crate::PadsPreview;
        on_big_stack(|| {
            let mut session = Session::new();
            let s = &mut session;
            s.make_sounds();
            s.run_periods(2);
            let kit_2 = s.instrument_list.of_kit(1).expect("kit 2 has an id");
            let part = format!("mpfree-pads 2\nplay instrument {kit_2}\n");
            let (notes, fx) = s.app.open_pads(Some(&part));
            assert!(notes.is_empty(), "{notes:?}");
            s.fx.extend(fx);
            s.run_periods(4);
            assert_eq!(s.app.pads_kit(), Some(1), "the pads play kit 2");
            let player = s.sounds.as_ref().expect("the kits");
            let pad = (0..16)
                .find(|&p| player.has_sound(1, p) && player.has_sound(0, p))
                .expect("a pad both kits fill");
            s.button_edge("NOTE REPEAT", true);
            s.run_periods(2);
            assert!(s.app.generating(), "NOTE REPEAT held");
            s.arp_played.clear();
            s.pad_edge(pad, 100, true);
            s.run_for(1.0);
            let repeats = |s: &Session| {
                s.arp_played
                    .iter()
                    .filter(|&&(to, n, _)| to == PadsPreview::Kit(1) && usize::from(n) == pad)
                    .count()
            };
            assert!(repeats(s) >= 4, "kit 2's pad repeats: {:?}", s.arp_played);
            assert_eq!(repeats(s), s.arp_played.len(), "nothing else plays");
            assert_eq!(s.app.pads_kit(), Some(1), "the pads keep kit 2");
            let player = s.sounds.as_ref().expect("the kits");
            assert_eq!(player.kit_pad_voices(0, pad), 0, "kit 1's pad never plays");
            s.pad_edge(pad, 0, false);
            s.run_for(0.3);
            let n = s.arp_played.len();
            s.run_for(0.5);
            assert_eq!(s.arp_played.len(), n, "let go: the repeats stop");
            assert_eq!(s.pads_arp_sounding(), 0);
            // Held again, then Follow track: the arp's notes end on kit 2.
            s.pad_edge(pad, 100, true);
            s.run_for(0.5);
            assert!(s.pads_arp_sounding() <= 1);
            s.fx.extend(s.app.pads_follow_track());
            s.run_periods(4);
            assert_eq!(s.app.pads_kit(), None);
            assert_eq!(s.pads_arp_sounding(), 0, "ended where it played");
            let n = s.arp_played.len();
            s.run_for(0.5);
            assert_eq!(s.arp_played.len(), n, "nothing more on kit 2");
            s.pad_edge(pad, 0, false);
            s.run_periods(2);
            s.button_edge("NOTE REPEAT", false);
            s.run_for(1.0);
            // The next hit plays the track's kit 1, through the track.
            s.previewed.clear();
            s.pad_edge(pad, 100, true);
            s.run_periods(2);
            let player = s.sounds.as_ref().expect("the kits");
            assert!(player.kit_pad_voices(0, pad) > 0, "kit 1's pad plays");
            assert!(s.previewed.is_empty(), "through the track");
            s.pad_edge(pad, 0, false);
        });
    }
    /// REC records what is heard (sampler step 15c's second part): every
    /// sound of the pad map on a track of its own, in one take. The pads
    /// play kit 2 on their own and bank B the synth, while Track 1 keeps
    /// kit 1. The chip names Track 1, which stays armed so the banks that
    /// follow it go on playing kit 1, and counts the two: "REC → Track 1 +
    /// 2 more". REC points Track 2 at kit 2 and Track 3 at the synth and
    /// arms both with Track 1. Kit 2's hits and NOTE REPEAT's notes on it
    /// go into Track 2 as kit 2's pad; on bank B the synth's hit and the
    /// arp's notes go into Track 3 (its notes by number, into a clip of
    /// notes). Track 1 gets none, and one take holds them all.
    #[test]
    fn rec_records_every_sound_of_the_pad_map_on_its_own_track_the_arp_too() {
        use crate::instrument_list::InstrumentId;
        use mpfree_transport::State;
        fn rec_chip(s: &Session) -> String {
            s.screen
                .ui
                .get::<crate::play_widgets::PlayTransport>(crate::play_page::TRANSPORT)
                .and_then(|t| t.look.rec.as_ref().map(|c| c.line()))
                .expect("the REC chip")
        }
        fn notes(s: &Session, t: usize) -> Vec<u8> {
            let mut n: Vec<u8> = s.time.project().tracks[t]
                .slots
                .clips()
                .flat_map(|c| c.notes.iter().map(|n| n.note))
                .collect();
            n.sort_unstable();
            n.dedup();
            n
        }
        fn hits(s: &Session, t: usize) -> usize {
            s.time.project().tracks[t]
                .slots
                .clips()
                .map(|c| c.notes.len())
                .sum()
        }
        on_big_stack(|| {
            let mut session = Session::new();
            let s = &mut session;
            s.make_sounds();
            s.run_periods(2);
            let kit_2 = s.instrument_list.of_kit(1).expect("kit 2 has an id");
            let synth = InstrumentId::SHARED_SYNTH;
            let part =
                format!("mpfree-pads 2\nplay instrument {kit_2}\nbank B instrument {synth}\n");
            let (said, fx) = s.app.open_pads(Some(&part));
            assert!(said.is_empty(), "{said:?}");
            s.fx.extend(fx);
            s.run_periods(4).settle();
            assert_eq!(s.app.pads_kit(), Some(1), "bank A: kit 2");
            assert_eq!(rec_chip(s), "REC → Track 1 + 2 more");
            s.route(crate::gui::FromUi::Time(
                mpfree_time_ui::TimeAction::Transport(mpfree_transport::Command::SetCountIn(0)),
            ));
            s.button("REC").settle();
            s.run_periods(4).settle();
            assert_eq!(s.time.armed(), Some(0), "the arm stays: {:?}", s.said());
            assert_eq!(s.track_kits.kit_of(0), Some(0), "Track 1 keeps kit 1");
            assert_eq!(s.track_kits.kit_of(1), Some(1), "Track 2 plays kit 2");
            assert!(s.track_kits.plays_synth(2), "Track 3 plays the synth");
            assert_eq!(s.time.also(), 0b110, "Tracks 2 and 3 armed too");
            assert_eq!(s.app.pads_kit(), Some(1), "the pads keep kit 2");
            assert_eq!(s.app.pads_own_rec(s.time.also()), Some(1));
            // As main.rs tells the time engine: Track 3's synth plays
            // notes, so its take holds notes, each pad as the notes it
            // plays.
            s.time.set_notes_tracks(1 << 2);
            let none = crate::sampler::KitNotes::default();
            let rows = crate::pad_rows(&s.mix.instruments, &s.synth, &s.track_kits, &none, 2);
            s.time.set_pad_rows(2, rows);
            // PLAY: the take records.
            s.button("PLAY");
            let end = s.t() + 4.0;
            while s.time.info().state != State::Recording && s.t() < end {
                s.run_periods(1);
            }
            assert_eq!(s.time.info().state, State::Recording);
            let player = s.sounds.as_ref().expect("the kits");
            let pad = (0..16)
                .find(|&p| player.has_sound(1, p))
                .expect("a pad kit 2 fills");
            s.pad(pad, 100, 0.05);
            s.run_for(0.1);
            // NOTE REPEAT on kit 2: its repeats go into Track 2 too.
            s.button_edge("NOTE REPEAT", true);
            s.run_periods(2);
            s.arp_played.clear();
            s.pad_edge(pad, 100, true);
            s.run_for(1.0);
            s.pad_edge(pad, 0, false);
            s.run_for(0.2);
            let repeats = s.arp_played.len();
            assert!(repeats >= 4, "{:?}", s.arp_played);
            s.button_edge("NOTE REPEAT", false);
            s.run_periods(2);
            // Bank B: the synth, into Track 3.
            s.button("PAD BANK B/F").run_periods(2);
            assert_eq!(s.app.pad_page(), 1);
            assert_eq!(s.app.pads_own_rec(s.time.also()), Some(2));
            s.pad(3, 100, 0.05);
            s.run_for(0.1);
            s.button_edge("NOTE REPEAT", true);
            s.run_periods(2);
            s.arp_played.clear();
            s.pad_edge(5, 100, true);
            s.run_for(1.0);
            s.pad_edge(5, 0, false);
            s.run_for(0.2);
            assert!(s.arp_played.len() >= 4, "{:?}", s.arp_played);
            s.button_edge("NOTE REPEAT", false);
            s.run_periods(2);
            s.button("REC");
            s.run_for(0.5);
            assert_eq!(notes(s, 1), [pad as u8], "Track 2: kit 2's pad");
            assert!(hits(s, 1) > repeats, "the hit and its repeats");
            let mut heard: Vec<u8> = [3, 5]
                .iter()
                .flat_map(|&p| s.synth.notes(p).to_vec())
                .collect();
            heard.sort_unstable();
            heard.dedup();
            assert_eq!(notes(s, 2), heard, "Track 3: the synth's notes");
            assert!(notes(s, 0).is_empty(), "Track 1's kit keeps out");
            assert_eq!(s.time.state().done.len(), 1, "one take");
            assert_eq!(s.app.pads_kit(), None, "bank B: the synth");
            assert_eq!(s.app.pad_page(), 1, "REC never moves the pads");
        });
    }
}
