//! The preset picker on the Synth tab's sound (layout spec 4g, Jay's pick A
//! of 18 Nov): a tap on the preset's name above the page opens the browser
//! on the engine's presets, Factory (the engine's own) and Yours (the
//! musician's, `Instruments/<engine>/` in the library). Turning tries each
//! one, Load keeps it, Cancel or BROWSE puts back the sound from before the
//! first try, and Save preset writes the sound as a file of the musician's.
//!
//! It works on the selected track's own synth when it has one
//! (instruments.rs), else on the synth on the pads. The pads' synth keeps
//! its own names and its sound from before a try ([`synth::Synth`]'s user
//! presets); a track's are kept here ([`TrackPresets`]), beside the factory
//! preset each track's synth plays. The screen's side is [`Picker`].

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use mpfree_browser_ui::PresetNow;
use mpfree_synth_ui::SynthState;

use crate::{gui, instruments, mixing, synth, Out};

/// What the screen asks of the sound loop (`gui::FromUi::Preset`).
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    /// Try the engine's own preset `i` (as the Synth tab's list has it).
    TryFactory(usize),
    /// Try the musician's preset `name` (its file's name), `text` read
    /// from its file in `folder` under Instruments (the engine's name).
    Try {
        folder: String,
        name: String,
        text: String,
    },
    /// Load it: it plays from now on, and nothing is kept to go back to.
    Load {
        folder: String,
        name: String,
        text: String,
    },
    /// Cancel or BROWSE: the sound from before the first try comes back.
    Revert,
    /// Save preset was named: the sound now as preset text, answered with
    /// `gui::ToUi::PresetText`.
    Text(String),
    /// The sound was saved among the musician's as this name.
    Saved(String),
}

/// A preset for a new track's own synth (New instrument track, the
/// instrument redesign's step 4c): one of the engine's own, or the
/// musician's file's `text` (read on the screen's side), named `name`.
#[derive(Clone, Debug, PartialEq)]
pub enum Give {
    Factory(usize),
    Text { name: String, text: String },
}

impl Ask {
    /// Whether it changes the song's sound for good (an edit to save).
    pub fn edits(&self) -> bool {
        matches!(self, Ask::Load { .. })
    }

    /// Whether the sound it leaves is another one (the synth's turns and
    /// the log's pending line were of the old one).
    pub fn changes_sound(&self) -> bool {
        matches!(
            self,
            Ask::TryFactory(_) | Ask::Try { .. } | Ask::Load { .. } | Ask::Revert
        )
    }
}

/// A track's sound from before the picker's first try on it.
struct Tried {
    track: usize,
    id: &'static str,
    sound: synth::SoundCopy,
    index: Option<usize>,
    own: Option<String>,
}

/// The preset each track's own synth plays, as the Synth tab names it (the
/// engine's own by index, or the musician's by name), and a track's sound
/// from before the picker's first try.
pub struct TrackPresets {
    index: [Option<usize>; instruments::HOLDERS],
    own: [Option<String>; instruments::HOLDERS],
    tried: Option<Tried>,
    /// A sound the builder was too busy to make: made once it is free (the
    /// newest only, so a fast turn ends on the preset it stops at).
    waiting: Option<(usize, &'static str, synth::SoundCopy)>,
}

impl Default for TrackPresets {
    fn default() -> TrackPresets {
        TrackPresets {
            index: [None; instruments::HOLDERS],
            own: std::array::from_fn(|_| None),
            tried: None,
            waiting: None,
        }
    }
}

impl TrackPresets {
    /// The engine's own preset track `track`'s synth plays, if it plays one.
    pub fn index(&self, track: usize) -> Option<usize> {
        self.index.get(track).copied().flatten()
    }

    /// The musician's preset track `track`'s synth plays, by its file's
    /// name, if it plays one.
    pub fn own(&self, track: usize) -> Option<&str> {
        self.own.get(track).and_then(|o| o.as_deref())
    }

    /// Track `track` loaded the engine's own preset `index` (or none): a
    /// try on it is over.
    pub fn set_index(&mut self, track: usize, index: Option<usize>) {
        if let Some(i) = self.index.get_mut(track) {
            *i = index;
        }
        self.new_sound(track);
    }

    /// New instrument track (step 4c): track `track`, just added, gets a
    /// synth of its own, the engine whose preset folder is `folder`,
    /// playing `give`; the builder makes it as a preset's sound. Returns
    /// what it will play, for the app; None, with the reason said, when it
    /// can't (not an engine, a preset that doesn't read, the track has an
    /// instrument already, or all 16 places are taken).
    pub fn give(
        &mut self,
        track: usize,
        folder: &str,
        give: Give,
        synth: &synth::Synth,
        mix: &mut mixing::MixHost,
        out: &mut Out,
    ) -> Option<instruments::Held> {
        let t = track + 1;
        let Some(engine) = synth::Engine::from_folder(folder) else {
            tell(
                out,
                format!("Track {t} plays its kit: {folder} isn't one of MPFree's synths"),
            );
            return None;
        };
        if mix.instruments.plays(track) {
            tell(
                out,
                format!("Track {t} plays an instrument already: the new sound isn't put on it"),
            );
            return None;
        }
        if !mix.instruments.has_room(track) {
            tell(out, crate::full_note(track, engine.name()));
            return None;
        }
        let (name, sound, index, own) = match give {
            Give::Factory(i) => match synth.preset_values(engine, i) {
                Some((name, extras, values)) => (name, (extras, values), Some(i), None),
                None => {
                    tell(
                        out,
                        format!(
                            "Track {t} plays its kit: {} has no preset {}",
                            engine.name(),
                            i + 1
                        ),
                    );
                    return None;
                }
            },
            Give::Text { name, text } => match synth.text_sound(engine, folder, &text) {
                Ok((_, extras, values)) => (name.clone(), (extras, values), None, Some(name)),
                Err(e) => {
                    tell(
                        out,
                        format!("Track {t} plays its kit: {name} doesn't play here: {e}"),
                    );
                    return None;
                }
            },
        };
        self.new_sound(track);
        self.make(track, engine.node_id(), sound, synth, mix);
        if let (Some(i), Some(o)) = (self.index.get_mut(track), self.own.get_mut(track)) {
            *i = index;
            *o = own;
        }
        tell(
            out,
            format!("Track {t}: its own {}, playing {name}", engine.name()),
        );
        Some(instruments::Held {
            id: engine.node_id(),
            name: engine.name(),
            instance: 0,
        })
    }

    /// Track `to` has a copy of track `from`'s sound (step 4c, Duplicate):
    /// it plays the same preset, by name, and a try on it is over.
    pub fn copy(&mut self, from: usize, to: usize) {
        let (index, own) = (self.index(from), self.own(from).map(str::to_string));
        self.new_sound(to);
        if let (Some(i), Some(o)) = (self.index.get_mut(to), self.own.get_mut(to)) {
            *i = index;
            *o = own;
        }
    }

    /// Track `track` has another sound that isn't a preset: no name, and a
    /// try on it is over.
    pub fn new_sound(&mut self, track: usize) {
        if let Some(o) = self.own.get_mut(track) {
            *o = None;
        }
        if self.tried.as_ref().is_some_and(|t| t.track == track) {
            self.tried = None;
        }
        if self.waiting.as_ref().is_some_and(|w| w.0 == track) {
            self.waiting = None;
        }
    }

    /// Once a loop: a sound the builder was too busy for, asked again.
    pub fn poll(&mut self, synth: &synth::Synth, mix: &mut mixing::MixHost) {
        if let Some((track, id, sound)) = self.waiting.take() {
            self.make(track, id, sound, synth, mix);
        }
    }

    /// Asks the builder for track `track`'s synth `id` with `sound`; when
    /// it is busy, it is asked again from [`TrackPresets::poll`].
    fn make(
        &mut self,
        track: usize,
        id: &'static str,
        sound: synth::SoundCopy,
        synth: &synth::Synth,
        mix: &mut mixing::MixHost,
    ) {
        let job = crate::instrument_job(synth, track, id, Some(sound.clone()));
        if !mix.build_instrument(job) {
            self.waiting = Some((track, id, sound));
        }
    }

    /// Track `track`'s sound now, kept before the first try on it. A try
    /// left on another track (the selection moved) is put back first.
    fn keep(
        &mut self,
        track: usize,
        id: &'static str,
        synth: &synth::Synth,
        mix: &mut mixing::MixHost,
    ) {
        if self.tried.as_ref().is_some_and(|t| t.track != track) {
            self.revert(synth, mix);
        }
        if self.tried.is_some() {
            return;
        }
        let Some(node) = mix.instruments.node(track) else {
            return;
        };
        let values = node.params().iter().map(|p| node.param(p.id)).collect();
        self.tried = Some(Tried {
            track,
            id,
            sound: (node.save_extras(), values),
            index: self.index(track),
            own: self.own(track).map(str::to_string),
        });
    }

    /// The sound from before the first try comes back, if the track still
    /// has the same synth. Returns the track.
    fn revert(&mut self, synth: &synth::Synth, mix: &mut mixing::MixHost) -> Option<usize> {
        let t = self.tried.take()?;
        if mix.instruments.held(t.track).map(|h| h.id) != Some(t.id) {
            return None;
        }
        self.make(t.track, t.id, t.sound, synth, mix);
        self.index[t.track] = t.index;
        self.own[t.track] = t.own;
        Some(t.track)
    }

    /// Does what the picker asks on track `track`'s own synth, or on the
    /// synth on the pads when the track has none. The Synth tab hears
    /// what plays now; a refusal is said on the screen.
    pub fn act(
        &mut self,
        ask: Ask,
        track: usize,
        synth: &mut synth::Synth,
        mix: &mut mixing::MixHost,
        out: &mut Out,
    ) {
        match mix.instruments.held(track).map(|h| h.id) {
            Some(id) if mix.instruments.holds(track) => match synth::Engine::from_node_id(id) {
                Some(engine) => self.on_track(ask, track, id, engine, synth, mix, out),
                // The picker opens only on an engine; a plugin lists its
                // own presets in the Synth tab's list.
                None => out.note(&format!(
                    "track {}: the preset picker is for MPFree's synths; this plugin's presets are in the Synth tab's list",
                    track + 1
                )),
            },
            _ => on_pads(ask, synth, mix, track, self, out),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn on_track(
        &mut self,
        ask: Ask,
        track: usize,
        id: &'static str,
        engine: synth::Engine,
        synth: &mut synth::Synth,
        mix: &mut mixing::MixHost,
        out: &mut Out,
    ) {
        let t = track + 1;
        let load = ask.edits();
        match ask {
            Ask::TryFactory(i) => {
                let Some((name, extras, values)) = synth.preset_values(engine, i) else {
                    out.note(&format!("track {t}: no preset {}", i + 1));
                    return;
                };
                self.keep(track, id, synth, mix);
                self.make(track, id, (extras, values), synth, mix);
                self.index[track] = Some(i);
                self.own[track] = None;
                out.note(&format!("track {t}: trying {name}"));
            }
            Ask::Try { folder, name, text } | Ask::Load { folder, name, text } => {
                let mine = self.tried.as_ref().is_some_and(|x| x.track == track);
                // Load on the one tried: it plays already.
                let playing = load
                    && mine
                    && self.own(track) == Some(name.as_str())
                    && self.waiting.is_none();
                if !playing {
                    let sound = match synth.text_sound(engine, &folder, &text) {
                        Ok((_, extras, values)) => (extras, values),
                        Err(e) => {
                            tell(out, format!("Track {t}: {name} doesn't play here: {e}"));
                            return;
                        }
                    };
                    if !load {
                        self.keep(track, id, synth, mix);
                    }
                    self.make(track, id, sound, synth, mix);
                }
                if load && mine {
                    self.tried = None;
                }
                self.index[track] = None;
                self.own[track] = Some(name.clone());
                out.note(&match load {
                    true => format!("track {t}: preset {name}"),
                    false => format!("track {t}: trying {name}"),
                });
            }
            Ask::Revert => {
                if let Some(back) = self.revert(synth, mix) {
                    out.note(&format!("track {}: back to its sound", back + 1));
                }
                return;
            }
            Ask::Text(name) => {
                let made = mix
                    .instruments
                    .node(track)
                    .and_then(|node| {
                        let values: Vec<f32> =
                            node.params().iter().map(|p| node.param(p.id)).collect();
                        synth.copy_text(engine, &node.save_extras(), &values, &name)
                    })
                    .map(|(folder, text)| (folder.to_string(), text))
                    .ok_or_else(|| "this sound couldn't be written as a preset".to_string());
                out.show(gui::ToUi::PresetText { name, made });
                return;
            }
            Ask::Saved(name) => {
                // What was tried is kept: Cancel no longer takes it back.
                if self.tried.as_ref().is_some_and(|x| x.track == track) {
                    self.tried = None;
                }
                self.index[track] = None;
                self.own[track] = Some(name.clone());
                out.note(&format!("track {t}: saved as {name}"));
            }
        }
        // The new copy shows once it plays (main.rs, Landed::Playing); the
        // name shows now.
        out.show(gui::ToUi::Synth(crate::synth_tab(synth, mix, track, self)));
    }
}

/// [`TrackPresets::act`] on the synth on the pads.
fn on_pads(
    ask: Ask,
    synth: &mut synth::Synth,
    mix: &mixing::MixHost,
    track: usize,
    presets: &TrackPresets,
    out: &mut Out,
) {
    match ask {
        Ask::TryFactory(i) => match synth.try_factory(i) {
            Some(name) => out.note(&format!("synth: trying {name}")),
            None => {
                out.note(&format!("synth: no preset {}", i + 1));
                return;
            }
        },
        Ask::Try { folder, name, text } => {
            if let Err(e) = synth.try_preset(&folder, &name, &text) {
                tell(out, format!("{name} doesn't play here: {e}"));
                return;
            }
            out.note(&format!("synth: trying {name}"));
        }
        Ask::Load { folder, name, text } => {
            if let Err(e) = synth.load_preset_file(&folder, &name, &text) {
                tell(out, format!("{name} wasn't loaded: {e}"));
                return;
            }
            out.note(&format!("synth: preset {name}"));
        }
        Ask::Revert => {
            if !synth.trying_preset() {
                return;
            }
            synth.revert_preset();
            out.note("synth: back to its sound");
        }
        Ask::Text(name) => {
            let (folder, text) = synth.save_preset(&name, &[], &[]);
            out.show(gui::ToUi::PresetText {
                name,
                made: Ok((folder.to_string(), text)),
            });
            return;
        }
        Ask::Saved(name) => {
            synth.saved_as(&name);
            out.note(&format!("synth: saved as {name}"));
        }
    }
    out.show(gui::ToUi::Synth(crate::synth_tab(
        synth, mix, track, presets,
    )));
}

/// A line for the screen and the log.
fn tell(out: &mut Out, line: String) {
    out.note(&line);
    out.show(gui::ToUi::Tell(line));
}

/// The screen's side of the picker (gui.rs).
#[derive(Default)]
pub struct Picker {
    /// Opened from the Synth tab's title, until the browser shows: the
    /// engine's folder, its own presets' names and what plays now.
    pending: Option<(String, Vec<String>, PresetNow)>,
    /// The picker shows: closing it takes the browser back to the Synth
    /// tab.
    pub open: bool,
    /// What the Synth tab named when it opened (its preset, the musician's
    /// preset, edited), named again on Cancel.
    before: Option<(Option<usize>, Option<String>, bool)>,
    /// A file of the musician's being read, to try (or load).
    reading: Option<Reading>,
    /// The name Cancel goes back to, for the hint line.
    back_to: String,
}

struct Reading {
    rx: Receiver<Result<String, String>>,
    folder: String,
    name: String,
    load: bool,
}

impl Picker {
    /// The Synth tab's title was tapped with `st` showing: the picker
    /// waits for the browser to show ([`Picker::opened`]). Err: why not.
    pub fn ask(&mut self, st: &SynthState, library: bool) -> Result<(), String> {
        let Some(engine) = synth::Engine::from_node_id(st.info.id) else {
            return Err(format!(
                "{} keeps its presets in the list on the left",
                st.info.name
            ));
        };
        if !library {
            return Err("No library: the app was started without --data, so presets can't be listed or saved".to_string());
        }
        let factory = st.own.is_none().then_some(st.preset).flatten();
        let name = st
            .own
            .clone()
            .or_else(|| factory.and_then(|i| st.presets.get(i).cloned()))
            .unwrap_or_else(|| st.info.name.to_string());
        let now = PresetNow {
            name,
            factory,
            changed: st.edited,
        };
        self.back_to = now.label();
        self.before = Some((st.preset, st.own.clone(), st.edited));
        self.pending = Some((engine.name().to_string(), st.presets.clone(), now));
        Ok(())
    }

    /// The hint line while the picker shows.
    pub fn hint(&self) -> String {
        format!(
            "Wheel: try each one  ·  pads: play it  ·  push: keep it  ·  - / +: Factory, Yours  ·  \
             BROWSE or Cancel: back to {}",
            self.back_to
        )
    }

    /// Cancel: a file still being read to try is not tried.
    pub fn revert(&mut self) {
        if self.reading.as_ref().is_some_and(|r| !r.load) {
            self.reading = None;
        }
    }

    /// The browser shows: the picker opens in it if the Synth tab asked.
    pub fn opened(&mut self, b: &mut mpfree_browser_ui::Browser) {
        if let Some((engine, list, now)) = self.pending.take() {
            b.open_preset_picker(&engine, list, now);
            self.open = true;
        }
    }

    /// Another page shows: an ask not yet opened is dropped.
    pub fn left(&mut self) {
        self.pending = None;
    }

    /// The picker closed: true if the browser should go back to the
    /// Synth tab.
    pub fn closed(&mut self) -> bool {
        // A file read to load still loads; one to try is not tried.
        self.revert();
        std::mem::take(&mut self.open)
    }

    /// What the Synth tab named when the picker opened (or what it was
    /// last saved as), for Cancel.
    pub fn before(&self) -> Option<&(Option<usize>, Option<String>, bool)> {
        self.before.as_ref()
    }

    /// The sound was saved as `name`: Cancel comes back to it.
    pub fn saved(&mut self, name: &str, preset: Option<usize>) {
        self.before = Some((preset, Some(name.to_string()), false));
        self.back_to = name.to_string();
    }

    /// Reads a file of the musician's on its own thread, to try (or load)
    /// once read ([`Picker::poll`]). A newer one replaces it.
    pub fn read(&mut self, path: PathBuf, folder: String, name: String, load: bool) {
        self.reading = Some(Reading {
            rx: read_in_background(path),
            folder,
            name,
            load,
        });
    }

    /// A file read: what to ask of the sound loop, or the line saying why
    /// it can't be.
    pub fn poll(&mut self) -> Option<Result<Ask, String>> {
        let r = self.reading.as_ref()?;
        let text = match r.rx.try_recv() {
            Ok(t) => t,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("its thread stopped".to_string()),
        };
        let Reading {
            folder, name, load, ..
        } = self.reading.take()?;
        Some(match text {
            Ok(text) if load => Ok(Ask::Load { folder, name, text }),
            Ok(text) => Ok(Ask::Try { folder, name, text }),
            Err(e) => Err(format!("{name} wasn't read: {e}")),
        })
    }
}

/// Reads preset file `path` on a thread of its own: its text (or why
/// not) comes on the receiver (New track).
pub fn read_in_background(path: PathBuf) -> Receiver<Result<String, String>> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("mpfree-preset".into())
        .spawn({
            let tx = tx.clone();
            move || {
                let _ = tx.send(read_text(&path));
            }
        });
    if let Err(e) = spawned {
        let _ = tx.send(Err(format!("not read ({e})")));
    }
    rx
}

/// A preset file's text, refused past the library's most.
fn read_text(path: &std::path::Path) -> Result<String, String> {
    use std::io::Read;
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut b = Vec::new();
    f.take(mpfree_library::MAX_PRESET_TEXT as u64 + 1)
        .read_to_end(&mut b)
        .map_err(|e| e.to_string())?;
    if b.len() > mpfree_library::MAX_PRESET_TEXT {
        return Err("it is too big to be a preset".to_string());
    }
    String::from_utf8(b).map_err(|_| "it isn't a preset (not text)".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{sync_channel, Receiver};

    const RATE: u32 = 44_100;

    /// An `Out` whose screen messages land in the receiver.
    fn out() -> (Out, Receiver<gui::ToUi>) {
        let (tx, rx) = sync_channel(256);
        let out = Out {
            console: None,
            sink: Some(tx),
            dropped: 0,
            report: None,
            queued: None,
            last_draw: std::time::Instant::now(),
        };
        (out, rx)
    }

    /// Track `track`'s own synth's settable values.
    fn track_values(mix: &mixing::MixHost, track: usize) -> Vec<f32> {
        let n = mix.instruments.node(track).expect("its own synth");
        n.params()
            .iter()
            .filter(|p| !p.readonly)
            .map(|p| n.param(p.id))
            .collect()
    }

    /// A sound's settable values, from all of them in the node's order.
    fn settable(engine: synth::Engine, all: &[f32]) -> Vec<f32> {
        let node = synth::engine_node(engine.node_id(), RATE).unwrap();
        node.params()
            .iter()
            .zip(all)
            .filter(|(p, _)| !p.readonly)
            .map(|(_, v)| *v)
            .collect()
    }

    /// Puts in what the builder made (here, at once).
    fn land(mix: &mut mixing::MixHost) -> bool {
        mix.poll_instruments().is_some()
    }

    fn shown(rx: &Receiver<gui::ToUi>) -> Vec<gui::ToUi> {
        rx.try_iter().collect()
    }

    #[test]
    fn the_picker_tries_loads_and_puts_back_a_tracks_own_synth() {
        let mut synth = synth::Synth::new(RATE);
        let mut mix = mixing::MixHost::new(RATE, -6.0, -6.0);
        let mut p = TrackPresets::default();
        let (mut out, rx) = out();
        assert!(mix.build_instrument(crate::instrument_job(&synth, 0, "mpfree.poly", None)));
        assert!(land(&mut mix));
        assert!(mix.instruments.holds(0));
        let first = track_values(&mix, 0);
        let poly = synth::Engine::Poly;
        let want: Vec<Vec<f32>> = (0..6)
            .map(|i| settable(poly, &synth.preset_values(poly, i).unwrap().2))
            .collect();

        // The engine's own preset 4, tried: a new copy plays it, named.
        p.act(Ask::TryFactory(4), 0, &mut synth, &mut mix, &mut out);
        assert!(land(&mut mix));
        assert_eq!(track_values(&mix, 0), want[4]);
        assert_eq!((p.index(0), p.own(0)), (Some(4), None));
        let st = shown(&rx).into_iter().find_map(|m| match m {
            gui::ToUi::Synth(st) => Some(st),
            _ => None,
        });
        assert_eq!(
            st.map(|s| (s.preset, s.track)),
            Some((Some(4), "Track 1".into()))
        );
        // Cancel: the sound from before the first try, to the bit.
        p.act(Ask::TryFactory(5), 0, &mut synth, &mut mix, &mut out);
        assert!(land(&mut mix));
        p.act(Ask::Revert, 0, &mut synth, &mut mix, &mut out);
        assert!(land(&mut mix));
        assert_eq!(track_values(&mix, 0), first);
        assert_eq!((p.index(0), p.own(0)), (None, None));
        p.act(Ask::Revert, 0, &mut synth, &mut mix, &mut out);
        assert!(!land(&mut mix), "nothing tried, nothing to put back");

        // One of the musician's (Poly's preset 3 saved as a file), tried
        // and loaded: named by its file; Load on the one tried makes
        // nothing new, and nothing is kept to go back to.
        let (_, extras, values) = synth.preset_values(poly, 3).unwrap();
        let (folder, text) = synth.copy_text(poly, &extras, &values, "Inside").unwrap();
        let mine = |load: bool| {
            let (folder, name, text) = (folder.to_string(), "Mine".to_string(), text.clone());
            match load {
                true => Ask::Load { folder, name, text },
                false => Ask::Try { folder, name, text },
            }
        };
        p.act(mine(false), 0, &mut synth, &mut mix, &mut out);
        assert!(land(&mut mix));
        assert_eq!(track_values(&mix, 0), want[3]);
        assert_eq!((p.index(0), p.own(0)), (None, Some("Mine")));
        p.act(mine(true), 0, &mut synth, &mut mix, &mut out);
        assert!(!land(&mut mix), "it plays already");
        assert!(mine(true).edits() && !mine(false).edits());
        p.act(Ask::Revert, 0, &mut synth, &mut mix, &mut out);
        assert!(!land(&mut mix));
        assert_eq!(track_values(&mix, 0), want[3]);
        // Load without a try makes it.
        p.set_index(0, Some(1));
        p.act(mine(true), 0, &mut synth, &mut mix, &mut out);
        assert!(land(&mut mix));
        assert_eq!(p.own(0), Some("Mine"));

        // Another engine's preset doesn't play here, and says so.
        shown(&rx);
        let fm = synth::Engine::Fm;
        let (_, fx, fv) = synth.preset_values(fm, 1).unwrap();
        let (_, bell) = synth.copy_text(fm, &fx, &fv, "Bell").unwrap();
        let ask = Ask::Try {
            folder: "FM".into(),
            name: "Bell".into(),
            text: bell,
        };
        p.act(ask, 0, &mut synth, &mut mix, &mut out);
        assert!(!land(&mut mix));
        assert!(shown(&rx)
            .iter()
            .any(|m| matches!(m, gui::ToUi::Tell(t) if t.contains("Bell doesn't play here"))));
        assert_eq!(p.own(0), Some("Mine"));

        // Save preset: the track's sound as text that plays the same, then
        // named by the name it was saved as.
        p.act(Ask::Text("Back".into()), 0, &mut synth, &mut mix, &mut out);
        let made = shown(&rx).into_iter().find_map(|m| match m {
            gui::ToUi::PresetText { name, made } => Some((name, made)),
            _ => None,
        });
        let (name, made) = made.expect("the text");
        assert_eq!(name, "Back");
        let (folder, text) = made.unwrap();
        assert_eq!(folder, "Poly");
        let (_, _, again) = synth.text_sound(poly, &folder, &text).unwrap();
        assert_eq!(settable(poly, &again), track_values(&mix, 0));
        p.act(Ask::Saved("Back".into()), 0, &mut synth, &mut mix, &mut out);
        assert_eq!((p.index(0), p.own(0)), (None, Some("Back")));
        // The track plays the kit again: no name.
        p.set_index(0, None);
        assert_eq!(p.own(0), None);
    }

    #[test]
    fn a_new_track_gets_its_own_synth_playing_the_sound_picked() {
        let synth = synth::Synth::new(RATE);
        let mut mix = mixing::MixHost::new(RATE, -6.0, -6.0);
        let mut p = TrackPresets::default();
        let (mut out, rx) = out();
        let poly = synth::Engine::Poly;
        let told = |rx: &Receiver<gui::ToUi>, what: &str| {
            shown(rx)
                .iter()
                .any(|m| matches!(m, gui::ToUi::Tell(t) if t.contains(what)))
        };

        // One of the engine's own: track 3 gets a Poly playing preset 4.
        let held = p.give(2, "Poly", Give::Factory(3), &synth, &mut mix, &mut out);
        assert_eq!(held.map(|h| (h.id, h.name)), Some((poly.node_id(), "Poly")));
        assert!(land(&mut mix));
        let (name, _, values) = synth.preset_values(poly, 3).unwrap();
        assert_eq!(track_values(&mix, 2), settable(poly, &values));
        assert_eq!((p.index(2), p.own(2)), (Some(3), None));
        assert!(told(&rx, &format!("Track 3: its own Poly, playing {name}")));

        // One of the musician's, by its text: named by it.
        let fm = synth::Engine::Fm;
        let (_, fx, fv) = synth.preset_values(fm, 1).unwrap();
        let (folder, text) = synth.copy_text(fm, &fx, &fv, "Bell").unwrap();
        let bell = Give::Text {
            name: "Bell".into(),
            text,
        };
        assert!(p
            .give(4, folder, bell, &synth, &mut mix, &mut out)
            .is_some());
        assert!(land(&mut mix));
        assert_eq!(mix.instruments.held(4).map(|h| h.id), Some(fm.node_id()));
        assert_eq!((p.index(4), p.own(4)), (None, Some("Bell")));

        // Refused, each saying why, and nothing is built.
        let refused = |p: &mut TrackPresets,
                       mix: &mut mixing::MixHost,
                       out: &mut Out,
                       track,
                       folder: &str,
                       give| {
            let held = p.give(track, folder, give, &synth, mix, out);
            held.is_none() && !land(mix)
        };
        assert!(refused(
            &mut p,
            &mut mix,
            &mut out,
            2,
            "Poly",
            Give::Factory(1)
        ));
        assert!(told(&rx, "Track 3 plays an instrument already"));
        assert!(refused(
            &mut p,
            &mut mix,
            &mut out,
            5,
            "Samples",
            Give::Factory(1)
        ));
        assert!(told(&rx, "Samples isn't one of MPFree's synths"));
        assert!(refused(
            &mut p,
            &mut mix,
            &mut out,
            5,
            "Poly",
            Give::Factory(999)
        ));
        assert!(told(&rx, "Poly has no preset 1000"));
        let junk = Give::Text {
            name: "Junk".into(),
            text: "not a sound".into(),
        };
        assert!(refused(&mut p, &mut mix, &mut out, 5, "FM", junk));
        assert!(told(&rx, "Junk doesn't play here"));
        assert!(!mix.instruments.holds(5));

        // With every place taken, the next track is told how to make room.
        let mut t = 5;
        while mix.instruments.count() < instruments::SLOTS {
            assert!(p
                .give(t, "Poly", Give::Factory(0), &synth, &mut mix, &mut out)
                .is_some());
            assert!(land(&mut mix));
            t += 1;
        }
        assert!(refused(
            &mut p,
            &mut mix,
            &mut out,
            t,
            "Poly",
            Give::Factory(0)
        ));
        assert!(told(&rx, "the most at once"));
    }

    #[test]
    fn the_picker_works_on_the_pads_synth_when_the_track_has_none() {
        let mut synth = synth::Synth::new(RATE);
        let mut mix = mixing::MixHost::new(RATE, -6.0, -6.0);
        let mut p = TrackPresets::default();
        let (mut out, rx) = out();
        synth.load_preset(2);
        let before = synth.values();
        p.act(Ask::TryFactory(4), 1, &mut synth, &mut mix, &mut out);
        assert!(synth.trying_preset());
        assert_ne!(synth.values(), before);
        let st = shown(&rx).into_iter().find_map(|m| match m {
            gui::ToUi::Synth(st) => Some(st),
            _ => None,
        });
        assert_eq!(
            st.map(|s| (s.preset, s.track)),
            Some((Some(4), "Pads".into()))
        );
        p.act(Ask::Revert, 1, &mut synth, &mut mix, &mut out);
        assert!(!synth.trying_preset());
        assert_eq!(synth.values(), before);
        // Save preset, then the name it was saved as.
        p.act(Ask::Text("Mine".into()), 1, &mut synth, &mut mix, &mut out);
        assert!(shown(&rx).iter().any(|m| matches!(
            m,
            gui::ToUi::PresetText { name, made: Ok((folder, _)) } if name == "Mine" && folder == "Poly"
        )));
        p.act(Ask::Saved("Mine".into()), 1, &mut synth, &mut mix, &mut out);
        assert_eq!(synth.state().own.as_deref(), Some("Mine"));
        assert!(Ask::Revert.changes_sound() && !Ask::Text(String::new()).changes_sound());
    }
}
