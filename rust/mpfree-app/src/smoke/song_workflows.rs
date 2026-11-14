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
            songs::put(s.library.as_ref().expect("a library"), &song, &d);
        }
        Lab { s, dir }
    }

    /// BROWSE, Projects, the song's row, Open: as on the MPC.
    fn open(s: &mut Session, name: &str) {
        // BROWSE opens the browser, or closes it if it shows (it stays
        // open after New project, say).
        if s.app.view() != crate::app::View::Browser {
            s.button("BROWSE");
        }
        s.press("browser", "place", "Projects");
        s.press("browser", "row", name);
        s.press("browser", "button", "Open");
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
                assert_eq!(l.s.time.project(), &song.clips, "{}", song.name);
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

    /// Night Market: launching the Intro plays the whole song by its
    /// scenes' follow actions, each section with its own tracks (the Perc
    /// track sits the Break out by the scene's mutes), and the Outro stops
    /// it.
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
                s.press("play-transport", "view", "Beats");
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
                s.press("header", "mixer-icon", "");
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
                    s.time
                        .project()
                        .tracks
                        .iter()
                        .all(|t| t.slots.iter().all(Option::is_none)),
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
            s.press("clips", "song", "Song");
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
            s.press("clips", "song", "Song");
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
}
