//! The time engine's screens: what they show, worked out from the engine,
//! and what their touches mean.
//!
//! - The **transport bar** sits under the header on the Arrange page:
//!   stop, play, record, replace or overdub, bar.beat.tick, tempo, meter,
//!   swing, loop and Undo (status chips on the right where a layout wants
//!   them; on Arrange the app's header carries the arp's chip).
//! - The **Arrange page** (the Clips page until it was renamed) is the
//!   launch grid (tracks across, their 16 clip slots down), the record
//!   settings, and the takes with Capture, Undo and Redo. Capture keeps
//!   what was just played (`mpfree-capture`). Play's Q-Link column runs
//!   down its right edge (layout spec 4c, Jay's option E): the app fills
//!   the cards and routes the knobs.
//!
//! The screens do not own the engine. The audio thread copies a
//! [`TransportInfo`] after each period; the control thread builds a
//! [`TimeState`] from it, the project, the launches and the takes, and
//! calls [`TimePage::update`]. Touches come back as [`TimeAction`]s for the
//! app to apply: transport commands go to the audio thread's queue, the
//! rest are edits on the control thread ([`clips_after`] works out the new
//! launches and cues). The header, the pads and the hint line belong to the
//! app.
//!
//! **Launching.** While the music plays, tapping a clip cues it: it starts
//! on the next bar line (its cell shows "from 6.1" in amber), and the
//! track's clip plays until then. Tapping the playing clip cues a stop the
//! same way. Tapping what is cued takes the cue back. While stopped, a tap
//! launches or stops at once.

use std::collections::{BTreeMap, BTreeSet};

use crate::add_view;
use crate::add_view::{
    bare_line, chords_view, line_action, panel_view, row_bars, shown_line, strip_of, AddPanel,
    AddPick, ChordPick, ChordsCard, CARD_FIT, CHORDS_TAB, PANEL_ROWS,
};
use crate::adding::{AddAction, ChordsAction};
use crate::chord_strip::{
    PICK_AS_HEARD, PICK_CLOSE, PICK_DISMISS, PICK_JOIN, PICK_KIND, PICK_ROOT, PICK_SPLIT, PICK_USE,
};
use crate::continuing::{Again, ContinueAction, Continuing};
use crate::editor::zoom::RollView;
use crate::editor::{
    box_of, chord_name, drop_of, edit_rows, knob_name, length_name, lit_scale, pads, position_name,
    sound_name, step_name, ChordMark, ChordShape, EditAction, Editor, Fold, GhostLayer, Hearing,
    LaneShow, Lens, LockMark, LockSetting, NoteBox, NoteGrid, NoteTool, ShowOffer, Tool, ToolAct,
    ToolBar, ToolKind, ADD, AIM_MENU, CHORD_MAJOR, CHORD_MENU, CHORD_MINOR, CLIP_END, CONTINUE_BAR,
    CONTINUE_NOTE, EDIT_BUTTON, ERASE_SWEEP, FIELDS, GHOST_MOST, GHOST_TAP, HELD_FINE, HELD_FIRST,
    HELD_RESET, HELD_TURN, LOCK_LANES, MENU, PADS_WRITE, PEN, PRESSURE_END, PRESSURE_LANE,
    ROOT_MENU, SCALE_MENU, SHOW_MENU, SHOW_NONE, SNAP_AUTO_T, SOUND_LANE, SOUND_OFF, SOUND_ON,
    START_HANDLE, STEP_FALLBACK, TOOL_BUTTONS, TOOL_CANCEL, TOOL_DONE, TOOL_OPTION,
};
use crate::list::{
    ListRow, SongListView, LIST_ADD, LIST_CLIPS, LIST_LINK, LIST_LINK_CLOSE, LIST_LINK_FROM_LIST,
    LIST_LINK_FROM_SONG, LIST_LOOP, LIST_PLAY, LIST_REMOVE, LIST_SONG, LIST_TO_SONG, SCENE_STEP,
};
use crate::report::{ReportCard, REPORT_DONE, REPORT_UNDO};
use crate::song::{
    AudioLane, LaneOver, SongBlocks, SongLanes, ADD_AUDIO, AUDIO_BUTTONS, BARS_BACK, BARS_ON,
    DUPLICATE, INPUTS, INPUT_CHIP, MONITORS, NONE, PANEL, PANEL_CLOSE, PANEL_INPUT, PANEL_MONITOR,
    PANEL_MUTE, PANEL_SOLO, PANEL_TAKES, REC, REGION_SLOTS, RULER_ROW, SECTION_TAP, SLIDE_ROW,
    START_ROW, TEMPO_ROW,
};
use crate::style_sheet::{StyleSheetView, STYLE_SHEET};
use crate::widgets::{
    BarButton, BarItem, CellAlert, CellMark, CellState, ClipGrid, GridCell, GridTrack, MorphBar,
    PartButton, Run, SongMark, TakeItem, TakeList, TransportBar, ADD_DISMISS, ADD_KIND, ADD_PART,
    ADD_TRACK, BACK_TO_SONG, BANK, CAPTURE, GRID_COLS, PAGE_BACK, PAGE_ON, PICKER, ROWS_BACK,
    ROWS_ON, ROW_DROP, STRIP_BAR, STRIP_CHIP, TAKE_WORDS,
};
use mpfree_arp_ui::make_sheet::{MakeSheet, SheetView, MAKE_SHEET};
use mpfree_audiotracks::host::TakeWave;
use mpfree_audiotracks::{Input, Monitor};
use mpfree_play::mpfree_param::clips::{self, Clips};
use mpfree_play::mpfree_param::{Owner, ParamAddr};
use mpfree_play::{anchored, ClipDrop, Cue, Launch, Replaced, SongChange, Take, Takes};
use mpfree_seq::scenes::MAX_FOLLOW_BARS;
use mpfree_seq::{
    direction_steps, Clip, ClipProgram, ClipSettings, Direction, Follow, FollowAction, LaunchGrid,
    Project, Quantise, SceneSettings, Speed, TrackProgram, Transpose, LAUNCH_GRIDS, MAX_CLIP_TICKS,
    MAX_TRACKS, MAX_TRANSPOSE, ROWS, SLOTS,
};
use mpfree_transport::time::{Meter, Swing, Tick, MAX_BPM, MIN_BPM, PPQ};
use mpfree_transport::transport::MAX_COUNT_IN;
use mpfree_transport::{Command, State, Transport};
use mpfree_ui::qlink::{QLinkCard, QLinkColumn};
use mpfree_ui::widgets::{Chip, OptionButton, OptionList, OptionNote, OptionRow};
use mpfree_ui::{Color, EventKind, Tone, Ui, UiEvent};
use std::sync::Arc;

/// Widget names the page adds. The app's own are `header` and `hint`.
pub const TRANSPORT: &str = "transport";
pub const CLIPS: &str = "clips";
pub const RECORD: &str = "record";
/// The clip settings card: the Record panel's Clip tab (sequencer S3).
pub const CLIP_CARD: &str = "clip-card";
/// Add a part's panel (composing tools S2.6, layout spec 4r): the side
/// area, from under the bar to the foot, while it is open.
pub const ADD_PANEL: &str = "add-part";
/// The row's chords: the Record panel's Chords tab (composing tools S2.6).
pub const CHORDS_CARD: &str = "chords-card";
pub const TAKES: &str = "takes";
pub const MORPH: &str = "morph";
pub const SONG: &str = "song";
/// The song list (sequencer S4): Arrange's third view.
pub const SONG_LIST: &str = "song-list";
/// The Show view (live shows B1): Arrange's fourth view, while a set is
/// open. Its widget is [`crate::show::ShowView`]; the app fills it.
pub const SHOW: &str = "show";
/// Play's Q-Link column, at Arrange's right edge too: the app's
/// `QLinkColumn` (its `play_page::QLINKS`), one widget on both pages.
pub const QLINKS: &str = "qlinks";
/// The width of the panel beside the grid, the song or the note editor
/// (Record or Edit over Takes), narrowed for the Q-Link column.
pub const SIDE_W: i32 = 250;
/// The note editor and its Edit options.
pub const NOTES: &str = "notes";
pub const EDIT_OPTIONS: &str = "edit-options";
/// The page's name, for the header tab.
pub const CLIPS_PAGE: &str = "Clips";
/// The bar in a layout spec: 44 pixels under the header.
pub const BAR_SPEC: &str = "transport:44";
/// The Clips view: the bar over the grid and the scene morph, and beside
/// them the record settings over the takes; Play's Q-Link column (128 px,
/// `mpfree_ui::qlink::COLUMN_WIDTH`) down the right edge, from under the
/// header to the hint line, as on Play.
pub const CLIPS_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(col(clips:*, morph:64):*, col(record:*, takes:240):250):*):*, qlinks:128):*, hint:26)";
/// The same page in its Song view: the timeline in place of the grid.
pub const SONG_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(col(song:*, morph:64):*, col(record:*, takes:240):250):*):*, qlinks:128):*, hint:26)";
/// The Show view (live shows B1, layout spec 4p): the bar over the view,
/// in the place of the grid and the Record panel, and the Q-Link column at
/// the right. No Record panel and no Takes.
pub const SHOW_LAYOUT: &str =
    "col(header:60, row(col(transport:44, show:*):*, qlinks:128):*, hint:26)";
/// The same page with the song list in place of the grid and the morph
/// (mockup D), and with the Record panel on its Clip tab.
pub const LIST_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(song-list:*, col(record:*, takes:240):250):*):*, qlinks:128):*, hint:26)";
pub const LIST_CARD_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(song-list:*, col(clip-card:*, takes:240):250):*):*, qlinks:128):*, hint:26)";
/// The hint line under the song list.
pub const LIST_HINT: &str =
    "Tap a step to pick it   ·   ‹ ›: its scene   ·   – / +: times   ·   + Add a scene: after the pick   ·   Play from here: on the next launch line   ·   List → song: onto the timeline";
/// The hint line under a song list linked to the song (phase 4b).
pub const LINKED_LIST_HINT: &str =
    "Tap a part to pick it   ·   ‹ ›: its scene   ·   – / +: times   ·   drag its grip: move it   ·   + Add a scene: after the pick   ·   Play from here: the song from there";
/// The hint line while Link's chooser is open (4b.5).
pub const LINK_HINT: &str =
    "Link: the list and the song become one   ·   Song from the list, or List from the song   ·   UNDO takes it back";
/// The Clips view and the Song view with the Record panel on its Clip tab:
/// the clip settings card in the record settings' place.
pub const CLIPS_CARD_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(col(clips:*, morph:64):*, col(clip-card:*, takes:240):250):*):*, qlinks:128):*, hint:26)";
pub const SONG_CARD_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(col(song:*, morph:64):*, col(clip-card:*, takes:240):250):*):*, qlinks:128):*, hint:26)";
/// The Clips view with Add a part's panel open (layout spec 4r): the
/// panel in the record settings' and the takes' place.
pub const CLIPS_ADD_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(col(clips:*, morph:64):*, add-part:250):*):*, qlinks:128):*, hint:26)";
/// The three views with the Make a phrase sheet open from the clip card
/// (composing tools, step 3 of the phrases): the sheet in the clip card's
/// and the takes' place.
pub const CLIPS_MAKE_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(col(clips:*, morph:64):*, make-phrase:250):*):*, qlinks:128):*, hint:26)";
pub const SONG_MAKE_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(col(song:*, morph:64):*, make-phrase:250):*):*, qlinks:128):*, hint:26)";
pub const LIST_MAKE_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(song-list:*, make-phrase:250):*):*, qlinks:128):*, hint:26)";
/// The three views with the Make a style sheet open from the clip card
/// (composing step 6, board D): the sheet in the clip card's and the
/// takes' place, as Make a phrase's.
pub const CLIPS_STYLE_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(col(clips:*, morph:64):*, make-style:250):*):*, qlinks:128):*, hint:26)";
pub const SONG_STYLE_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(col(song:*, morph:64):*, make-style:250):*):*, qlinks:128):*, hint:26)";
pub const LIST_STYLE_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(song-list:*, make-style:250):*):*, qlinks:128):*, hint:26)";
/// The three views with the Record panel on its Chords tab: the row's
/// chords in the record settings' place.
pub const CLIPS_CHORDS_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(col(clips:*, morph:64):*, col(chords-card:*, takes:240):250):*):*, qlinks:128):*, hint:26)";
pub const SONG_CHORDS_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(col(song:*, morph:64):*, col(chords-card:*, takes:240):250):*):*, qlinks:128):*, hint:26)";
pub const LIST_CHORDS_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(song-list:*, col(chords-card:*, takes:240):250):*):*, qlinks:128):*, hint:26)";
/// The hint line with Add a part's Style row on a phrase (P2, board H).
pub const ADD_PHRASE_HINT: &str = "Style, after Jazz-ish: the phrases of the part's kind, MPFree's and yours   ·   it follows the row's chords   ·   Try again: a phrase plays one way";
/// The hint line with the Record panel's Chords tab.
pub const CHORDS_HINT: &str =
    "A clip that doesn't fit is marked, never rewritten   ·   untick one to leave it   ·   Fit: the ticked clips, one UNDO   ·   a chord on the row: change it   ·   Row: the scene pick";
/// The hint line under the Clips grid while Rec song is on: Add a part is
/// faint, and this says why.
pub const REC_SONG_CLIPS_HINT: &str =
    "Rec song is on: Add a part waits, as a try would be recorded into the song   ·   tap a clip: it plays from the next Launch line   ·   + records   ·   a track's name arms it";
pub const CLIPS_HINT: &str =
    "Tap a clip: it plays or stops on the next Launch line   ·   drag it: move it (SHIFT as you lift: copy)   ·   a row's number plays it, held: its card   ·   + records   ·   a track's name arms it";
/// The hint line under the Clips grid in Perform: nothing on the grid
/// drags there, and an empty slot needs a hold to make a clip (the touch
/// usability review, 25 Nov 2026).
pub const PERFORM_CLIPS_HINT: &str =
    "Tap a clip: it plays or stops on the next Launch line   ·   nothing drags (Make moves clips)   ·   a row's number plays it, held: its card   ·   hold +: records   ·   a track's name arms it";
/// The hint line while the scene card is open (5.2).
pub const SCENE_CARD_HINT: &str =
    "‹ ›: change a setting   ·   a setting's name: the wheel turns it   ·   Tempo or Mutes: tap it to take what plays now   ·   Empty cells: tap a track, Stop or Keep   ·   Done or a tap off the card: close";
/// The hint line under the Clips grid with the Record panel's Clip tab.
pub const CLIPS_CARD_HINT: &str =
    "Clip: the armed track's playing clip   ·   wheel: pick a row, - / +: change it   ·   SHIFT + ‹ ›: Length and Speed halve or double   ·   COPY + a clip, then an empty slot: a copy";

/// The hint line under the Clips grid with the Clip tab: Convert to
/// notes' preview while it is open on the focused clip (E1); an audio
/// clip's card says what Fit to tempo makes at the project's tempo
/// (layout spec 4j); else [`CLIPS_CARD_HINT`].
pub fn clip_card_hint(st: &TimeState) -> String {
    if let Some(c) = st.converting() {
        return c.line.clone();
    }
    match focused_audio(st) {
        Some(_) => format!(
            "Clip: the armed track's playing clip   ·   wheel: pick a row, - / +: change it   ·   \
             Fit to tempo: a copy that fits {} BPM, same pitch   ·   UNDO goes back",
            tempo_short(st.transport.bpm)
        ),
        None => CLIPS_CARD_HINT.to_string(),
    }
}

/// The note inspector (piano roll plan 1C.1): while notes are picked in
/// the note editor, the Q-Link column's header ("3 notes") and its four
/// cards, Position, Length, Semitones and Velocity, each with its value
/// or range ("80–104"). None with no notes picked.
pub fn note_cards(e: &EditInfo) -> Option<(String, [QLinkCard; 4])> {
    let c = &e.clip;
    // A tool open (plan 2B.3): its settings, and the picks it works on.
    if let Some(t) = e.editor.open_tool {
        let n = e.editor.sel.iter().filter(|&&i| i < c.notes.len()).count();
        let names = t.cards();
        let cards = std::array::from_fn(|k| {
            let (name, value) = &names[k];
            let mut card = QLinkCard::new(name, t.kind.name(), value, t.pos(k));
            card.bipolar = t.bipolar(k);
            card
        });
        let notes = if n == 1 { "note" } else { "notes" };
        if t.kind == ToolKind::Continue {
            let more = e
                .editor
                .continuing
                .value(crate::editor::compose::BARS_ROW, &e.editor.styles);
            return Some((format!("Continue · {more}"), cards));
        }
        return Some((format!("{} · {n} {notes}", t.kind.name()), cards));
    }
    let picked: Vec<&mpfree_seq::Note> = e
        .editor
        .sel
        .iter()
        .filter_map(|&i| c.notes.get(i))
        .collect();
    let first = picked.first()?;
    let pad = pads(c);
    let span = |f: &dyn Fn(&mpfree_seq::Note) -> i64| {
        picked.iter().fold((f(first), f(first)), |(lo, hi), n| {
            (lo.min(f(n)), hi.max(f(n)))
        })
    };
    let range = |(lo, hi): (i64, i64), name: &dyn Fn(i64) -> String| {
        if lo == hi {
            name(lo)
        } else {
            format!("{}–{}", name(lo), name(hi))
        }
    };
    let bar = e.bar.max(1);
    let length = c.length.max(1);
    let beat = PPQ as Tick;
    // Several places: bar.beat of the first and the last.
    let at = span(&|n| n.tick);
    let position = if at.0 == at.1 {
        position_name(at.0, bar)
    } else {
        let short = |t: i64| format!("{}.{}", t / bar + 1, (t % bar) / beat + 1);
        format!("{}–{}", short(at.0), short(at.1))
    };
    let lens = span(&|n| n.len);
    let short_len = |l: i64| {
        let name = length_name(l);
        name.strip_suffix(" ticks")
            .map_or(name.clone(), |n| format!("{n}t"))
    };
    let pitch = span(&|n| i64::from(n.note));
    let sound = |n: i64| sound_name(n.clamp(0, 127) as u8, pad);
    let vel = span(&|n| i64::from(n.velocity));
    let in_scale = lit_scale(c).is_some();
    let mut cards: [QLinkCard; 4] =
        std::array::from_fn(|k| QLinkCard::new(knob_name(k, in_scale), "Notes", "", 0.0));
    cards[0].value = position;
    cards[0].pos = at.0 as f32 / length as f32;
    cards[1].value = if lens.0 == lens.1 {
        short_len(lens.0)
    } else {
        range(lens, &short_len)
    };
    cards[1].pos = (lens.1 as f32 / bar as f32).min(1.0);
    cards[2].value = range(pitch, &sound);
    cards[2].pos = pitch.1 as f32 / if pad { 15.0 } else { 127.0 };
    cards[3].value = range(vel, &|v| v.to_string());
    cards[3].pos = vel.1 as f32 / 127.0;
    let n = picked.len();
    let mut head = if n == 1 {
        "1 note".to_string()
    } else {
        format!("{n} notes")
    };
    // The chord the picks make (plan 2B.2): "3 notes · Dm7".
    let pitches: Vec<u8> = picked.iter().map(|n| n.note).collect();
    if let Some(name) = chord_name(&pitches).filter(|_| !pad) {
        head = format!("{head} · {name}");
    }
    Some((head, cards))
}

pub fn notes_copy_hint(st: &TimeState) -> String {
    match &st.notes_copied {
        Some(what) => format!(
            "COPY held: tap empty space in any clip to paste {what} there   ·   tap a note: copy the picks instead   ·   UNDO takes it back"
        ),
        None => "COPY held: tap a note to copy the picked notes (or that one)   ·   then, COPY held again, empty space in any clip: the copy goes there".to_string(),
    }
}

/// The hint line while COPY is held on the Clips grid: what a tap copies
/// or pastes.
pub fn copy_hint(st: &TimeState) -> String {
    match &st.clipboard {
        Some(name) => format!(
            "COPY held: tap an empty slot, on any track: a copy of {name} goes there   ·   tap a clip: copy that one instead   ·   UNDO takes a paste back"
        ),
        None => "COPY held: tap a clip to copy it   ·   then, COPY held again, an empty slot on any track: the copy goes there".to_string(),
    }
}

/// The hint line under the Song view.
pub const SONG_HINT: &str =
    "Tap a block to pick it (SHIFT + tap: several)   ·   drag across empty space: a box picks blocks   ·   drag a block or a section's name: move it   ·   tap the ruler: a marker";
/// The hint line under the Song view with a block picked.
pub const SONG_BLOCK_HINT: &str =
    "Drag it: move it (SHIFT as you lift: copy)   ·   drag its start or end: trim   ·   hold it still, then drag: slide what it plays   ·   Split: at the playhead   ·   wheel: a beat";
/// The hint line under the Song view with several blocks picked (6.5).
pub const SONG_PICKS_HINT: &str =
    "Drag one: they all move (SHIFT as you lift: copies)   ·   SHIFT + tap: add or take out a block   ·   Split, Duplicate, Delete: all of them   ·   wheel: a beat";
/// The hint line under the Song view with a section picked.
pub const SONG_SECTION_HINT: &str =
    "Drag the name: move the section (SHIFT as you lift: copy)   ·   drag the grip: move where it starts   ·   + Bar: a bar of room before it   ·   push the wheel: move it a section at a time";
/// The hint line under the Song view with a tempo point picked.
pub const SONG_TEMPO_HINT: &str =
    "Drag the point: move it a bar at a time   ·   push the wheel: change its tempo   ·   Ramp: glide to the next point   ·   SHIFT + wheel: bars";

/// The hint line while the conversion report card is open (layout spec
/// 4n): what Done keeps (the rows Song → scenes made, or the song), and
/// the wheel when its rows scroll.
pub fn report_hint(r: &Report) -> String {
    let kept = if r.title == "SONG → SCENES" {
        "the rows"
    } else {
        "the song"
    };
    if r.rows.len() > crate::report::SHOWN {
        format!(
            "Done: close it and keep {kept}   ·   UNDO: take it all back   ·   drag the list or turn the wheel: the rest   ·   a tap outside keeps it"
        )
    } else {
        format!(
            "Done: close it and keep {kept}   ·   UNDO, here or on the MPC: take it all back   ·   a tap outside or a page change keeps it"
        )
    }
}

/// The hint line under the Song view for what is picked.
pub fn song_hint(st: &TimeState) -> &'static str {
    if st.tempo_pick.is_some() {
        SONG_TEMPO_HINT
    } else if st.selected.is_some() {
        SONG_BLOCK_HINT
    } else if !st.picks.is_empty() {
        SONG_PICKS_HINT
    } else if st.section.is_some() {
        SONG_SECTION_HINT
    } else {
        SONG_HINT
    }
}
/// The same page with the note editor (a piano roll, a list or a
/// tracker) in place of the grid, across the page: the Edit options and
/// the takes are in a drawer that Edit, TC or the wheel opens (piano roll
/// plan 1A.1).
pub const EDIT_LAYOUT: &str =
    "col(header:60, row(col(transport:44, notes:*):*, qlinks:128):*, hint:26)";
/// The note editor with its drawer open: the Edit options and the takes
/// at the right, where the record settings are on the grid.
pub const EDIT_DRAWER_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(notes:*, col(edit-options:*, takes:240):250):*):*, qlinks:128):*, hint:26)";
/// The hint line under the note editor's piano roll, Draw off.
pub const EDIT_HINT: &str =
    "Tap: pick   ·   drag: move   ·   drag empty space: box   ·   hold one, tap more: add   ·   handles: start, length   ·   double tap: new note   ·   ERASE + tap or drag: erase   ·   two fingers: zoom";
/// The hint line under the piano roll with Draw on (piano roll plan 1B.1).
pub const EDIT_DRAW_HINT: &str =
    "Draw: tap a note in, or a short one out   ·   drag: a long note   ·   hold a note: pick, tap more: add   ·   handles: start, length   ·   ERASE + tap or drag: erase   ·   two fingers: zoom";
/// The hint line under the note editor's list.
pub const EDIT_LIST_HINT: &str =
    "Tap a row to pick its note   ·   ERASE + tap: erase   ·   SHIFT + wheel: scroll   ·   Edit, TC or the wheel: the options";
/// The hint line under the note editor's tracker.
pub const EDIT_TRACKER_HINT: &str =
    "Tap a row to pick its note   ·   Draw: tap an empty step   ·   ERASE + tap: erase   ·   wheel: the cursor   ·   SHIFT + wheel: scroll   ·   Edit, TC or a push: the options";
/// The hint line under the piano roll with notes picked and the Edit
/// options' drawer closed (piano roll plan 1C.1): the Q-Links are the
/// note inspector.
pub const EDIT_PICKED_HINT: &str =
    "Q-Links: position, length, pitch, velocity (SHIFT: finer)   ·   - / +: nudge   ·   drag: move   ·   handles: start, length   ·   wheel: the cursor, or a held note's velocity   ·   ERASE + tap or drag: erase";

/// The hint line under the note editor showing `lens`, with `tool`, and
/// `picked` when notes are picked with the drawer closed.
pub fn edit_hint(lens: Lens, tool: Tool, picked: bool) -> &'static str {
    match lens {
        Lens::Grid if picked => EDIT_PICKED_HINT,
        Lens::Grid if tool == Tool::Draw => EDIT_DRAW_HINT,
        Lens::Grid => EDIT_HINT,
        Lens::List => EDIT_LIST_HINT,
        Lens::Tracker => EDIT_TRACKER_HINT,
    }
}
/// The hint line while a tool is open (piano roll plan 2B.3): its
/// Q-Links, its own button, Done and Cancel.
pub fn tool_hint(t: &NoteTool) -> String {
    let knobs: Vec<String> = t
        .cards()
        .iter()
        .enumerate()
        .filter(|(_, (name, _))| !name.is_empty())
        .map(|(k, (name, _))| format!("Q{} {}", k + 1, name.to_lowercase()))
        .collect();
    let own = match t.kind {
        ToolKind::Chop => "   ·   Reset pressure: pieces without it",
        ToolKind::Humanize => "   ·   Again: another try",
        ToolKind::Continue => "   ·   Tap new bars or notes to pick them",
        _ => "",
    };
    format!(
        "{}: {} (SHIFT: finer){own}   ·   Done: in, one UNDO   ·   Cancel: the clip as it was",
        t.kind.name(),
        knobs.join(", ")
    )
}

/// The record settings, top to bottom (short, to fit the 250 px panel).
/// Makes, last here, shows second while loops go in audio cells
/// ([`record_order`]).
pub const RECORD_ROWS: [&str; 11] = [
    "Armed",
    "Mode",
    "Count-in",
    "Loop",
    "Tempo",
    "Pressure",
    "Quantise",
    "Launch",
    "Transpose",
    "Follows",
    "Makes",
];
/// The Record tab's Makes row (phase 4, board 2): what REC makes, a loop
/// in the armed cell, takes on the song or notes. Shown only with Dev's
/// "Loops on audio tracks", under Armed.
pub const MAKES_ROW: usize = 10;
/// What a step on Makes says: it follows what is armed.
pub const MAKES_WHY: &str =
    "Makes follows what is armed: an empty audio cell makes a loop, song mode takes";

/// The Record tab's rows as shown, top to bottom, by their place in
/// [`RECORD_ROWS`]: Makes under Armed while loops go in audio cells.
pub fn record_order(st: &TimeState) -> &'static [usize] {
    if st.loops_on {
        &[0, MAKES_ROW, 1, 2, 3, 4, 5, 6, 7, 8, 9]
    } else {
        &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]
    }
}

/// A step on the Record tab's shown row, as a step on its place in
/// [`RECORD_ROWS`]; None for any other event, or when the order is
/// [`RECORD_ROWS`]' own.
fn record_event(ev: &UiEvent, st: &TimeState) -> Option<UiEvent> {
    let (RECORD, EventKind::Step { index, delta }) = (ev.widget.as_str(), &ev.kind) else {
        return None;
    };
    if !st.loops_on {
        return None;
    }
    let index = *record_order(st).get(*index)?;
    Some(UiEvent {
        widget: RECORD.to_string(),
        kind: EventKind::Step {
            index,
            delta: *delta,
        },
    })
}
/// The Record tab's Transpose and Follows rows (sequencer S4, live
/// transpose), for the armed track.
pub const TRANSPOSE_ROW: usize = 8;
pub const FOLLOWS_ROW: usize = 9;
/// The scene editor's rows, top to bottom. Program and Bank are the
/// program change the scene sends the armed track's gear (sequencer S5);
/// Sent to lists every track it sends one.
pub const SCENE_ROWS: [&str; 10] = [
    "Scene", "Follow", "After", "Chance", "Or", "Mutes", "Program", "Bank", "Sent to", "Tempo",
];
/// The scene's Tempo row (the Arrange refactor's 5.2b, on the scene card).
pub const SCENE_TEMPO: usize = 9;
/// The scene's Mutes row.
pub const SCENE_MUTES: usize = 5;
/// `TimeAction::SceneSettings`'s card row for an empty cell turned over
/// on the scene card (Keep playing, 5.2): this plus the track.
pub const SCENE_KEEP_ROW: u8 = 64;
/// The scene editor's Program and Bank rows.
pub const SCENE_PROGRAM: usize = 6;
pub const SCENE_BANK: usize = 7;
/// `TimeAction::SceneSettings`'s card row for a rename.
pub const SCENE_NAME: u8 = SCENE_ROWS.len() as u8;
/// The other actions the scene editor's Or row steps through: Stay,
/// then the follow actions.
pub const OTHERS: [FollowAction; 9] = [
    FollowAction::Stay,
    FollowAction::Next,
    FollowAction::Previous,
    FollowAction::First,
    FollowAction::Last,
    FollowAction::Any,
    FollowAction::Other,
    FollowAction::Again,
    FollowAction::Stop,
];
/// The clip settings card's rows, top to bottom.
pub const CLIP_ROWS: [&str; 17] = [
    "Length",
    "Speed",
    "Swing",
    "Quantise",
    "Strength",
    "Follow",
    "After",
    "Launch",
    "Mode",
    "Program",
    "Bank",
    "Direction",
    "Mutate",
    "Melody",
    "Continue",
    "Phrase",
    "Style",
];
/// The clip card's Direction row (S6).
pub const CLIP_DIRECTION: usize = 11;
/// The clip card's Mutate row (S6): › a new variation of the clip's
/// notes, SHIFT + › a bolder one, ‹ the original back.
pub const CLIP_MUTATE: usize = 12;
/// The clip card's Melody row (step sequencer refactor E1): "pads" (it
/// follows the pads: pad hits, playing whatever those pads play now) or
/// "notes" (it uses notes, each keeping its pitch); › on
/// a clip that follows the pads on a track whose instrument plays notes
/// previews Convert to notes (`TimeAction::ConvertPreview`).
pub const CLIP_MELODY: usize = 13;

/// An audio clip's card's rows, top to bottom (layout spec 4j): Speed,
/// Swing, Quantise, Strength, Program, Bank, Direction and Mutate don't
/// apply to audio.
pub const AUDIO_CLIP_ROWS: [&str; 7] = [
    "Length", "Mode", "Launch", "Follow", "After", "Gain", "Name",
];
/// The audio card's Gain row: off to +12 dB, in 0.5 dB steps (SHIFT 0.1).
pub const AUDIO_GAIN: usize = 5;
/// The audio card's Name row: a tap or a turn opens the naming keyboard
/// (the app's).
pub const AUDIO_NAME: usize = 6;
/// The longest name the naming keyboard takes for a clip.
pub const MAX_CLIP_NAME: usize = 24;
/// The note card's row each audio card row steps as: Mode, Launch,
/// Follow and After are a note clip's settings.
const AUDIO_AS_NOTES: [usize; 7] = [0, 8, 7, 5, 6, usize::MAX, usize::MAX];
/// An audio clip's gain at its most: +12 dB.
pub const AUDIO_GAIN_MAX_DB: f32 = 12.0;
/// Below this an audio clip's gain is off.
const AUDIO_GAIN_FLOOR_DB: f32 = -60.0;

/// What an audio clip's card changes ([`TimeAction::AudioClip`]).
#[derive(Clone, Debug, PartialEq)]
pub enum AudioSet {
    /// A loop's length, in its own ticks.
    Length(Tick),
    /// Its own gain, 0 (off) to 4 (+12 dB).
    Gain(f32),
    Name(String),
}
/// The clip card's Continue row (composing tools, phase 1 step 4): either
/// arrow opens the note editor on the clip with Continue open.
pub const CLIP_CONTINUE: usize = 14;
/// The clip card's Phrase row (composing tools, step 3 of the phrases):
/// either arrow opens the Make a phrase sheet on the clip's notes.
pub const CLIP_PHRASE: usize = 15;
/// The clip card's Style row (composing step 6, board D): either arrow
/// opens the Make a style sheet on the clip, read from its notes.
pub const CLIP_STYLE: usize = 16;

/// A play direction (S6) as the clip card names it.
pub fn direction_name(d: Direction) -> &'static str {
    match d {
        Direction::Forward => "Forward",
        Direction::Reverse => "Reverse",
        Direction::PingPong => "Ping pong",
        Direction::Random => "Random",
    }
}
/// The clip card's Mode row: Restart, Legato (takes over the playing
/// clip's place), Once (a one-shot: one pass, then the track stops).
pub const MODES: [&str; 3] = ["Restart", "Legato", "Once"];

/// A clip's settings' place in [`MODES`] (one-shot over legato).
pub fn mode_of(s: &ClipSettings) -> usize {
    if s.once {
        2
    } else if s.legato {
        1
    } else {
        0
    }
}
/// The speeds the card's Speed row steps through, slowest first, as
/// (clip ticks, song ticks): x1/8 to x4.
pub const SPEEDS: [(u8, u8); 12] = [
    (1, 8),
    (1, 4),
    (1, 3),
    (1, 2),
    (2, 3),
    (3, 4),
    (1, 1),
    (4, 3),
    (3, 2),
    (2, 1),
    (3, 1),
    (4, 1),
];
/// The grids the card's Quantise row steps through after off, coarsest
/// first: 1/4, 1/8, 1/8 triplets, 1/16, 1/16 triplets, 1/32.
pub const QUANTISE_GRIDS: [u32; 6] = [PPQ, PPQ / 2, PPQ / 3, PPQ / 4, PPQ / 6, PPQ / 8];
/// The follow actions the card's Follow row steps through after off.
pub const FOLLOWS: [FollowAction; 8] = [
    FollowAction::Next,
    FollowAction::Previous,
    FollowAction::First,
    FollowAction::Last,
    FollowAction::Any,
    FollowAction::Other,
    FollowAction::Again,
    FollowAction::Stop,
];
/// The swing the card's Swing row steps through after the track's: 50 %
/// (straight) to 75 %, on 1/16 notes.
pub const CLIP_SWING: std::ops::RangeInclusive<u8> = 50..=75;
/// A step of the card's Length row: a sixteenth, as on Beats.
pub const CARD_STEP: Tick = PPQ as Tick / 4;
/// The loop lengths the Loop row steps through, in bars from bar 1 (0 is off).
pub const LOOP_BARS: [i64; 6] = [0, 1, 2, 4, 8, 16];
/// Track colours, by track number.
pub const TRACK_COLORS: [u32; 8] = [
    0xE8A33D, 0x3DDC97, 0x6EA8FE, 0xFF8A3D, 0xC77DFF, 0xFF6FA8, 0x2EC4B6, 0x9AA8FF,
];

/// `layout` with the transport bar under the header, for the app's Play
/// page and the arp's layouts: `col(header:60, ...` becomes
/// `col(header:60, transport:44, ...`. None if it does not start that way.
pub fn with_transport(layout: &str) -> Option<String> {
    const HEAD: &str = "col(header:60, ";
    let rest = layout.strip_prefix(HEAD)?;
    Some(format!("{HEAD}{BAR_SPEC}, {rest}"))
}

pub fn track_color(track: usize) -> Color {
    Color::rgb(TRACK_COLORS[track % TRACK_COLORS.len()])
}

/// Audio tracks' colours, by audio track: the track colours in another
/// order (teal and amber first, as the mockups have them).
pub fn audio_color(track: usize) -> Color {
    const ORDER: [usize; 8] = [6, 0, 2, 5, 1, 4, 3, 7];
    track_color(ORDER[track % ORDER.len()])
}

// ---------------------------------------------------------------- state

/// The transport as the screen shows it. Plain values: the audio thread
/// copies one after each period without allocating.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransportInfo {
    pub state: State,
    /// The song position at the start of the next period.
    pub position: Tick,
    pub bpm: f64,
    pub meter: Meter,
    pub swing: Swing,
    /// The loop set here. While [`TransportInfo::follows_position`] it
    /// is held off, not played ([`TransportInfo::playing_loop`]).
    pub loop_range: Option<(Tick, Tick)>,
    pub overdub: bool,
    /// Bars of count-in before recording from stopped.
    pub count_in: u8,
    /// Following another device's position (Sync's Position in the I/O
    /// view): the leader owns the timeline, so the loop is off, greyed,
    /// and comes back when following stops (a loop set meanwhile too).
    pub follows_position: bool,
}

impl Default for TransportInfo {
    fn default() -> Self {
        TransportInfo {
            state: State::Stopped,
            position: 0,
            bpm: 120.0,
            meter: Meter::default(),
            swing: Swing::default(),
            loop_range: None,
            overdub: false,
            count_in: 0,
            follows_position: false,
        }
    }
}

impl TransportInfo {
    /// The loop the transport plays: none while following another
    /// device's position, which holds the loop set here off.
    pub fn playing_loop(&self) -> Option<(Tick, Tick)> {
        self.loop_range.filter(|_| !self.follows_position)
    }

    pub fn of(t: &Transport) -> TransportInfo {
        TransportInfo {
            state: t.state(),
            position: t.position(),
            bpm: t.bpm(),
            meter: t.meter(),
            swing: t.swing(),
            loop_range: t.loop_range(),
            overdub: t.overdub(),
            count_in: t.count_in(),
            follows_position: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClipInfo {
    pub name: String,
    /// One pass, in the clip's own ticks.
    pub length: Tick,
    pub speed: Speed,
    pub notes: usize,
    /// A Loop track's cell: the looper's loop it launches (`Clip::looped`).
    pub looped: Option<u8>,
    /// Its speed, swing, quantise and follow action, for the clip
    /// settings card.
    pub settings: ClipSettings,
    /// The variation of its notes it plays, kept beside its original
    /// (`Clip::variation`, S6's Mutate); None when it keeps none.
    pub variation: Option<u32>,
    /// Its notes are pitches, not pad hits (`Clip::holds`, step 5c): the
    /// clip card's Melody row (step sequencer refactor E1).
    pub holds_notes: bool,
    /// Pad hits or notes (`Clip::holds`): the note editor shows only
    /// clips of its own clip's kind behind it (plan 4.1).
    pub holds: mpfree_seq::Holds,
    /// An audio clip's facts, for its cell and card (audio tracks phase
    /// 2); None for a note or loop clip.
    pub audio: Option<AudioCellInfo>,
    /// A loop cell on an audio track (phase 4, `Clip::loop_clip`): the
    /// loop it plays, by its lasting id. Its facts come from the looper's
    /// slot holding that id ([`LoopsInfo::ids`]).
    pub loop_id: Option<mpfree_seq::LoopId>,
}

/// What an audio clip's cell and card show (audio tracks phase 2, layout
/// spec 4f and 4j): the clip's own facts, and what the app adds from the
/// audio tracks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioCellInfo {
    /// The tempo it was made at (BPM): Fit to tempo fits from it.
    pub bpm: f64,
    /// Its own gain (the card's Gain row), 0 to 4 (+12 dB).
    pub gain: f32,
    /// Its sound's length in seconds, its first frame to its last: a once
    /// clip says it ("3:17 once"). The app sets it.
    pub secs: f64,
    /// Its small wave over one pass, each column's peak 0 to 255. The app
    /// sets it.
    pub wave: [u8; crate::widgets::CELL_WAVE],
    /// A recording it names isn't here: why, for the hint ("take 4's file
    /// wasn't found when the project opened"). The app sets it.
    pub missing: Option<String>,
    /// A once clip still sounding past its stop, this far through its
    /// sound (its playhead is drawn dimmed). The app sets it.
    pub ring: Option<f32>,
}

/// What the looper says of a loop, for its cells on the Clips grid: the
/// app fills it from the Looper's view ([`TimeState::loops`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoopFacts {
    /// It records a layer now.
    pub overdubbing: bool,
    /// Another loop's layers copy into it (a loop cell copied onto
    /// another Loop track, loop-tracks.md part 6).
    pub copying: bool,
    /// Its small wave for the cells ([`crate::widgets::GridCell::wave`]).
    pub wave: [u8; crate::widgets::CELL_WAVE],
    /// A closed layer of it isn't on the stick yet (phase 4c writes each
    /// layer soon after it closes): its cells show an amber dot and
    /// "writing" (layout spec 4f).
    pub writing: bool,
}

/// The loops for the grid: each loop's facts, how many hold a loop, and
/// the share of the loop memory in use, in percent. The app fills it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoopsInfo {
    pub loops: [LoopFacts; mpfree_seq::LOOPS],
    pub in_use: usize,
    pub memory_pct: u8,
    /// A new loop's first pass waits for PLAY or the bar line, or
    /// records: the armed Loop cell says so.
    pub waiting: bool,
    pub recording: bool,
    /// Loops on audio tracks (phase 4): each slot's loop's lasting id (0:
    /// none), so a loop cell finds its loop's facts by its id.
    pub ids: [u128; mpfree_seq::LOOPS],
}

/// Put in a cell's cell (audio tracks phase 2): the armed track's first
/// empty cell if it is an audio track with one, else the first audio
/// track's that has one, as (track, row). None: no audio track has an
/// empty cell (a new one is added for the sound). `tracks` gives, track by
/// track, whether it is an audio track and its first empty row.
pub fn pick_cell(
    armed: Option<usize>,
    tracks: impl Iterator<Item = (bool, Option<usize>)>,
) -> Option<(usize, usize)> {
    let cells: Vec<Option<usize>> = tracks
        .map(|(audio, empty)| empty.filter(|_| audio))
        .collect();
    let armed = armed.and_then(|a| Some((a, (*cells.get(a)?)?)));
    armed.or_else(|| {
        cells
            .iter()
            .enumerate()
            .find_map(|(t, e)| e.map(|row| (t, row)))
    })
}

impl ClipInfo {
    /// One pass, in song ticks.
    pub fn song_len(&self) -> f64 {
        self.speed.to_song(self.length as f64)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrackInfo {
    pub name: String,
    /// Its lasting id (`Track::id`), so what is said about the track
    /// elsewhere (System status) finds its name.
    pub id: mpfree_seq::TrackId,
    /// What it plays, under its name ("Kit", "FM · Glass Keys"); empty
    /// shows only the name.
    pub sound: String,
    pub mute: bool,
    /// Soloed (`Track::solo`, sequencer S4).
    pub solo: bool,
    /// Silent: muted, or not soloed while another track is.
    pub silent: bool,
    /// It records pad pressure (`Track::record_touch`).
    pub pressure: bool,
    /// Its record quantise grid (`Track::record_quantise`), or none.
    pub record_quantise: Option<u32>,
    /// A Loop track, and what its new loops record (`Track::kind`).
    pub loops: Option<mpfree_seq::LoopInput>,
    /// An audio track (`Track::is_audio`, the audio tracks rethink's
    /// phase 1): its `sound` is its input, the app fills it in.
    pub audio: bool,
    /// Its live transpose and "Follows transpose" (`Track::transpose`,
    /// `Track::follows`, sequencer S4).
    pub transpose: Transpose,
    pub follows: bool,
    /// It plays notes, not a kit's pads: its own instrument, or the synth
    /// on the pads. Only such a track transposes. The screen fills it in.
    pub melodic: bool,
    /// Its kit's pads muted and soloed on this track alone, a bit each by
    /// kit pad (`Track::pad_mutes`, `Track::pad_solos`; the instrument
    /// redesign's step 10).
    pub pad_mutes: u128,
    pub pad_solos: u128,
    pub slots: Vec<Option<ClipInfo>>,
}

/// The takes by their transactions ([`TimeState::take_txns`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TakeTxns {
    /// Those that can be undone, oldest first, and redone, the next one
    /// last (as [`Takes`] keeps them).
    pub done: Vec<mpfree_transport::TransactionId>,
    pub undone: Vec<mpfree_transport::TransactionId>,
    /// [`Takes::let_go`].
    pub let_go: mpfree_transport::TransactionId,
}

impl TakeTxns {
    pub fn of(takes: &Takes) -> TakeTxns {
        TakeTxns {
            done: takes.done().iter().map(|t| t.txn).collect(),
            undone: takes.undone().iter().map(|t| t.txn).collect(),
            let_go: takes.let_go(),
        }
    }
}

/// One take as the list shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TakeRow {
    pub number: u32,
    /// Made by CAPTURE.
    pub captured: bool,
    /// An edit of a clip's notes (the step sequencer).
    pub edited: bool,
    /// It made a new clip.
    pub made: bool,
    /// The track and slot it recorded into (of its first note).
    pub at: Option<(usize, usize)>,
    pub added: usize,
    pub erased: usize,
    /// It took a clip out of its slot (ERASE + a clip).
    pub deleted: bool,
    /// It added this track (+ Track).
    pub new_track: Option<usize>,
    /// A drop on the Clips grid: a clip or a row moved, swapped or copied.
    pub dropped: Option<ClipDrop>,
    /// A Song view edit: a block moved, copied, made longer or deleted, a
    /// section moved.
    pub song: Option<SongChange>,
    /// A clip's settings changed (the clip settings card).
    pub settings: bool,
    /// COPY + an empty slot pasted a clip there.
    pub pasted: bool,
    /// A whole clip went in place of the clip there, and why: Reload
    /// clip (sequencer S4), Mutate or Restore (S6).
    pub replaced: Option<Replaced>,
    /// The variation a Mutate made ("variant 3").
    pub variant: Option<u32>,
    /// The scene editor changed this row's scene (its name, mutes or
    /// follow action).
    pub scene: Option<usize>,
    /// Song → scenes filled these rows, as (first, last).
    pub rows: Option<(usize, usize)>,
    /// The scene card's row tools (5.2, 5.3): "capture" (Capture scene's
    /// new row) or "delete" (Delete row), and the row.
    pub row_tool: Option<(&'static str, usize)>,
    /// The take put a clip in this row or took one out, and the song's
    /// parts that follow the row came along ([`Take::followed`]).
    pub followed: Option<usize>,
    /// This row's chords changed (composing tools S2.5): a chord set on
    /// its Harmony line, or the chords a part was added over.
    pub chords: Option<usize>,
    /// Add a part added a part.
    pub part: bool,
    /// Make a style kept a style (composing step 6).
    pub style: bool,
}

impl TakeRow {
    pub fn of(t: &Take) -> TakeRow {
        let made = t.made.first().map(|(track, slot, _)| (*track, *slot));
        let removed = t.removed.first().map(|(track, slot, _)| (*track, *slot));
        let at = match t.dropped {
            // Named by where the clip came from; a row by its number.
            Some(ClipDrop::Moved(a, _) | ClipDrop::Swapped(a, _) | ClipDrop::Copied(a, _)) => {
                Some(a)
            }
            Some(_) => None,
            None => made
                .or(removed)
                .or_else(|| {
                    t.added
                        .first()
                        .or(t.erased.first())
                        .map(|&(track, slot, _)| (track, slot))
                })
                .or_else(|| t.settings.map(|(track, slot, ..)| (track, slot))),
        };
        let added = t.added.len() + t.made.iter().map(|(_, _, c)| c.notes.len()).sum::<usize>();
        TakeRow {
            number: t.number,
            captured: t.captured,
            edited: t.edited,
            made: made.is_some(),
            at,
            added,
            erased: t.erased.len(),
            deleted: removed.is_some(),
            new_track: t.track_added.as_ref().map(|(i, _)| *i),
            dropped: t.dropped,
            song: t.song.as_ref().map(|e| e.change),
            settings: t.settings.is_some(),
            pasted: t.pasted,
            replaced: t.replaced,
            variant: t.made.first().and_then(|(_, _, c)| c.variation()),
            scene: t.scene.as_ref().map(|e| e.row),
            rows: t
                .rows
                .as_ref()
                .filter(|e| e.what == mpfree_play::RowsWhat::ToScenes)
                .and_then(|e| Some((*e.rows.first()?, *e.rows.last()?))),
            row_tool: t.rows.as_ref().and_then(|e| match e.what {
                mpfree_play::RowsWhat::ToScenes => None,
                mpfree_play::RowsWhat::Captured { row } => Some(("capture", row)),
                mpfree_play::RowsWhat::Deleted { row } => Some(("delete", row)),
            }),
            followed: t.followed.as_ref().and_then(|e| match e.change {
                SongChange::Followed { row } => Some(row),
                _ => None,
            }),
            chords: t.chords.as_ref().map(|e| e.row),
            part: t.part,
            style: t.styles.is_some(),
        }
    }

    /// What it is: "take", "capture", "edit", "delete", "paste",
    /// "settings", "reload", "mutate", "restore", for a drop "move",
    /// "swap" or "copy", and for a Song view edit "move", "copy",
    /// "length", "delete", "unique", "scenes", "tempo", "bars",
    /// "marker", "rec song" or "recorded over" (a recording cut the
    /// blocks under it),
    /// "list" for the
    /// song list, "scene" for the scene editor and "song → scenes"; for
    /// the composing tools "continue", "fit", "add part", "chords" and
    /// "style".
    pub fn word(&self) -> &'static str {
        if let Some((word, _)) = self.row_tool {
            return word;
        }
        if self.rows.is_some() {
            return "song → scenes";
        }
        if self.part {
            return "add part";
        }
        if self.chords.is_some() {
            return "chords";
        }
        if self.style {
            return "style";
        }
        if let Some(c) = self.song {
            return match c {
                SongChange::Moved { .. } | SongChange::SectionMoved { .. } => "move",
                SongChange::Copied { .. } | SongChange::SectionCopied { .. } => "copy",
                SongChange::Resized { .. } => "length",
                SongChange::Deleted { .. } => "delete",
                SongChange::Unique { .. } => "unique",
                SongChange::Recorded { .. } => "recorded over",
                SongChange::RecSong { .. } => "rec song",
                SongChange::Split { .. } => "split",
                SongChange::Trimmed { .. } => "trim",
                SongChange::Slid { .. } => "slide",
                SongChange::SeveralMoved { .. } => "move",
                SongChange::SeveralCopied { .. } => "copy",
                SongChange::SeveralDeleted { .. } => "delete",
                SongChange::SeveralSplit { .. } => "split",
                SongChange::Scenes { .. } => "scenes",
                SongChange::List | SongChange::ListToSong { .. } => "list",
                SongChange::Tempo { .. } => "tempo",
                SongChange::Bars { .. } => "bars",
                SongChange::Marker { .. } => "marker",
                SongChange::Followed { .. } => "parts",
                SongChange::RowDeleted { .. } => "delete",
            };
        }
        if let Some(d) = self.dropped {
            return match d {
                ClipDrop::Moved(..) | ClipDrop::RowMoved { .. } => "move",
                ClipDrop::Swapped(..) => "swap",
                ClipDrop::Copied(..) | ClipDrop::RowCopied { .. } => "copy",
            };
        }
        if self.scene.is_some() {
            "scene"
        } else if let Some(r) = self.replaced {
            match r {
                Replaced::Reload => "reload",
                Replaced::Mutate => "mutate",
                Replaced::Restore => "restore",
                Replaced::Fit => "fit",
                Replaced::Settings => "settings",
                Replaced::Convert => "convert",
                Replaced::Continue { .. } => "continue",
                Replaced::FitChords => "fit",
            }
        } else if self.deleted {
            "delete"
        } else if self.pasted {
            "paste"
        } else if self.settings {
            "settings"
        } else if self.captured {
            "capture"
        } else if self.edited {
            "edit"
        } else if self.number & mpfree_play::ERASE_TAKES != 0 {
            // ERASE + a pad while only playing (live erase's own takes).
            "erase"
        } else {
            "take"
        }
    }

    /// Its number as shown: live erase's takes count from 1 too, without
    /// the top bit their numbers carry (`mpfree_play::ERASE_TAKES`).
    pub fn shown(&self) -> u32 {
        self.number & !mpfree_play::ERASE_TAKES
    }

    /// What Undo and Redo name it: "take 3", "capture 2", "edit 5",
    /// "erase 1" (live erase while playing), "new track 9", or "move 7 ·
    /// row 3" for a row.
    pub fn short(&self) -> String {
        let n = self.shown();
        match (self.new_track, self.dropped) {
            (Some(t), _) => format!("new track {}", t + 1),
            (None, Some(ClipDrop::RowMoved { from, .. } | ClipDrop::RowCopied { from, .. })) => {
                format!("{} {n} · row {}", self.word(), from + 1)
            }
            _ => format!("{} {n}", self.word()),
        }
    }
}

/// Everything the time engine's screens show.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimeState {
    pub transport: TransportInfo,
    pub tracks: Vec<TrackInfo>,
    /// The looper's loops, for the Loop tracks' cells. The app sets them.
    pub loops: LoopsInfo,
    pub launches: Vec<Launch>,
    /// Launches and stops waiting for their launch line (the next bar line
    /// by default). The app sets them.
    pub cues: Vec<Cue>,
    pub armed: Option<usize>,
    /// The track selected apart from the arm ([`TimeAction::Select`]), or
    /// None while the selection is the armed track: see
    /// [`TimeState::selected`].
    pub track_pick: Option<usize>,
    /// The selected track's slot picked for the Sequencer to edit apart
    /// from what plays ([`TimeAction::EditClip`]), or None while it edits
    /// the clip that plays.
    pub clip_pick: Option<usize>,
    /// The other armed tracks, a bit each (`TimeAction::ArmAlso`): a take
    /// records their notes with the armed track's. Never the armed one.
    pub also: u128,
    /// The clip COPY + a clip copied last, by its name, for COPY + an
    /// empty slot to paste. The app sets it.
    pub clipboard: Option<String>,
    /// The notes COPY + a note in the note editor copied last, as the
    /// hint line says them ("4 notes from Verse"), for COPY + empty roll
    /// to paste (piano roll plan 1B.4). The app sets it.
    pub notes_copied: Option<String>,
    /// The takes that can be undone, oldest first, and the ones that can be
    /// redone, the next one last (as [`Takes`] keeps them).
    pub done: Vec<TakeRow>,
    pub undone: Vec<TakeRow>,
    /// The same takes by their transactions, and the newest one the list
    /// let go of, for the one UNDO across the engines (audio tracks phase
    /// 4d): how the app follows the takes while the editing side runs on
    /// a thread of its own.
    pub take_txns: TakeTxns,
    /// Status chips on the right of the bar (the arp's).
    pub chips: Vec<Chip>,
    /// What Capture would keep now, such as "2 bars"; None when nothing
    /// was played since the last capture. The app sets it.
    pub capture: Option<String>,
    /// The scene (row) playing, and the one waiting for the next bar line
    /// (`mpfree_perform::Perform`). The app sets them.
    pub scene_playing: Option<usize>,
    pub scene_cued: Option<usize>,
    /// The Loop track cell armed: the next loop recorded lands there
    /// (`TimeAction::LoopCell`).
    pub loop_cell: Option<(usize, usize)>,
    /// Dev's "Loops on audio tracks" (phase 4, until 4e): an audio
    /// track's empty cell takes a loop, and the screens show loops in
    /// audio cells (layout spec 4c, 4f). The time engine sets it.
    pub loops_on: bool,
    /// Each row's scene name, as far as the project names them (empty: a
    /// plain row, shown as its number).
    pub scenes: Vec<String>,
    /// The scene morph. The app sets it.
    pub morph: MorphInfo,
    /// The scene editor's row (the Record panel's Scene tab, S4), and
    /// its name, mutes and follow action. The app sets them.
    pub scene_focus: usize,
    pub scene: SceneSettings,
    /// The scene card open over the grid or the song list (the Arrange
    /// refactor's 5.2): its row, which is the scene in focus too.
    pub scene_open: Option<usize>,
    /// The page shows the song (the timeline) in place of the grid. The
    /// app sets it.
    pub song_view: bool,
    /// The page shows the song list in place of the grid (it wins over
    /// `song_view`). The app sets it.
    pub list_view: bool,
    /// The song list's steps as it shows them, the step picked, and
    /// whether it goes round. The app sets them.
    pub list: Vec<ListRow>,
    pub list_pick: Option<usize>,
    pub list_looped: bool,
    /// The song list is linked to the song: its rows are the song's parts
    /// (the Arrange refactor's phase 4b). The app sets it.
    pub list_linked: bool,
    /// While the list plays: the step after the one playing (None: it
    /// stops) and the line it comes in on, for Perform's NEXT. The app
    /// sets it.
    pub list_next: Option<(Option<usize>, Tick)>,
    /// Song mode: the song plays on tracks with no clip launched. The app
    /// sets it.
    pub song_on: bool,
    /// The project is in Perform, not Make (`Project::perform`, sequencer
    /// S4).
    pub perform: bool,
    /// The project's launch quantise (`Project::launch`, S4).
    pub launch: LaunchGrid,
    /// The line each cue lands on, by track, where the time engine set
    /// it apart from the clip's own and the project's (a scene's, a
    /// follow's, one kept when Launch changed). The app sets it.
    pub cue_lines: Vec<(usize, LaunchGrid)>,
    /// Mute and solo changes waiting for their line (the mute queue). The
    /// app sets it.
    pub mute_wait: Option<MuteWait>,
    /// Cues waiting for their audio while takes stream from their files
    /// (the audio tracks rethink's phase 2). The app sets it.
    pub audio_waits: AudioWaitCounts,
    /// The tracks whose cue waits for its audio and has moved to a later
    /// line, with that line (the cell says "waits 18.1"). The app sets it.
    pub audio_late: Vec<(usize, Tick)>,
    /// Audio clips whose launch was cancelled, their audio not ready in
    /// time, as (track, slot, ticks it waited), until launched again (the
    /// cell says "not ready"). The app sets it.
    pub not_ready: Vec<(usize, usize, Tick)>,
    /// Fit to tempo under way on an audio clip's card: its cell, and how
    /// far it has got, 0 to 1. The app sets it.
    pub fitting: Option<(usize, usize, f32)>,
    /// The audio tracks' history: steps their Undo can take back and Redo
    /// bring back. UNDO on Arrange follows it while it moved last (layout
    /// spec 4c). The app sets it.
    pub audio_history: (usize, usize),
    /// The audio track armed last by its own arm, while no note track has
    /// been armed since: the Clip tab shows its clip ([`TimeState::focus`]).
    /// The app sets it.
    pub audio_focus: Option<usize>,
    /// The clips with a saved state (Save state on the clip card, S4), as
    /// (track, slot): Reload clip brings it back.
    pub saved: Vec<(usize, usize)>,
    /// Reloads waiting for their launch line, as (track, slot, the line's
    /// song tick).
    pub reloads: Vec<(usize, usize, Tick)>,
    /// Convert to notes' preview, while it is open (step sequencer
    /// refactor E1): the clip card's buttons are Cancel and Convert, and
    /// the hint line says what it would do, while it shows the focused
    /// clip.
    pub converting: Option<Converting>,
    /// The scene playing's follow action, for Perform's NEXT (S4). The
    /// app sets it.
    pub follow_next: Option<FollowNext>,
    /// The arrangement's placements and markers. The placements are
    /// shared: the screen keeps the same list from one transport update to
    /// the next, so the Song view knows its blocks are the same without
    /// comparing them (Arrange refactor 2.4). They are in the song's order,
    /// by track and then start ([`TimeState::song_has`] searches them).
    pub placements: Arc<[PlacementInfo]>,
    pub markers: Vec<(Tick, String)>,
    /// Each marker's passes and their length while its part follows its
    /// scene with more than one (the Arrange refactor's 4b.4: on a linked
    /// song, "×N" after its name and a line where each pass starts).
    pub passes: Vec<Option<(u16, Tick)>>,
    /// The song's sections that play a scene, in order: where each starts
    /// and ends, and the scene's row, for the Song view's Chords lane
    /// (composing step 6, board C; `mpfree_seq::arrange::scene_sections`).
    pub scene_sections: Vec<(Tick, Tick, usize)>,
    /// The placement selected on the Song view. The app sets it.
    pub selected: Option<usize>,
    /// The blocks picked together on the Song view (6.5), in order: two
    /// or more, `selected` being None while they are; empty otherwise.
    /// The app sets it.
    pub picks: Vec<usize>,
    /// The section picked on the Song view (by its marker), for the
    /// wheel. The app sets it.
    pub section: Option<usize>,
    /// Live to song armed: what the grid plays goes into the song. The app
    /// sets it.
    pub song_rec: bool,
    /// Perform (sequencer S4): the take recording, or the next one, turns
    /// Rec song on for its length; the REC chip says "+ Rec song".
    pub song_with_take: bool,
    /// The note editor, when it is open. The app sets it.
    pub edit: Option<Box<EditInfo>>,
    /// The audio tracks' lanes on the Song view (layout spec 4c), under
    /// the MIDI lanes: one for each audio track with a region or armed,
    /// in order. Build their blocks with [`crate::song::audio_blocks`].
    /// The app sets them.
    pub audio: Vec<AudioLane>,
    /// The audio takes' waveforms read so far (`AudioHost::wave`), a take
    /// each. The app sets them.
    pub waves: Vec<TakeWave>,
    /// The audio region picked on the Song view. The app sets it; it and
    /// `selected` are never both set (see [`TimeState::pick`]).
    pub region: Option<RegionId>,
    /// The audio tracks' memory: seconds free and seconds in all, for the
    /// Song view's legend. The app sets it.
    pub audio_free: Option<(f64, f64)>,
    /// Audio tracks counters to show while they are not zero, such as
    /// ("dropped", 2) or ("read errors", 1): the engine's `Stats` and the
    /// streaming counts. Fallbacks and losses are always shown. The app
    /// sets them.
    pub audio_counts: Vec<(String, u32)>,
    /// The audio track whose panel is open over the lanes (layout spec
    /// 4c), if any. The app sets it.
    pub audio_panel: Option<usize>,
    /// The audio track whose take lanes show under its lane (layout spec
    /// 4c), if any. The app sets it.
    pub audio_takes: Option<usize>,
    /// The song's tempo points as (tick, tempo, glides to the next), in
    /// order, and the project's tempo before the first (tempo ramps).
    pub tempo_points: Vec<(Tick, f64, bool)>,
    pub tempo_base: f64,
    /// The tempo point picked on the Song view. The app sets it.
    pub tempo_pick: Option<usize>,
    /// The last conversion's report while its card is open (the Arrange
    /// refactor's 4.3): only when its result plays differently.
    pub report: Option<Arc<Report>>,
    /// While song mode plays the song's tempo points: the tempo of the
    /// point in effect at the playhead (None before the first), which the
    /// Tempo row steps from. None otherwise. The app sets it.
    pub tempo_here: Option<f64>,
    /// Continue, while one is under way (composing tools).
    pub continuing: Option<Arc<crate::continuing::Continuing>>,
    /// Add a part, while one is under way (composing tools S2.6).
    pub adding: Option<Arc<crate::adding::Adding>>,
    /// The rows whose chords changed since their kept clips fitted them,
    /// with the clips that no longer do (composing tools S2.6).
    pub unfit: Arc<[crate::adding::Unfit]>,
    /// The row the screens asked to hear (`ChordsAction::Hear`) and its
    /// chords as it plays: its line heard again with its clips, or what
    /// they say when it has none. None until heard.
    pub row_chords: Option<(usize, Arc<mpfree_compose::RowChords>)>,
    /// Each row the song's sections play and its chords as it plays, heard
    /// while the Song view shows, for its Chords lane (composing step 6,
    /// board C); rows not heard yet are missing.
    pub song_chords: Arc<[(usize, Arc<mpfree_compose::RowChords>)]>,
    /// The project's styles made from clips (composing step 6), read from
    /// its lines: every Style row runs on into them after the built-in
    /// ones.
    pub styles: Arc<[mpfree_compose::ClipStyle]>,
    /// Make a style, open on a clip (composing step 6): in the clip
    /// card's place on Arrange's Clips.
    pub style_sheet: Option<Box<crate::style_sheet::StyleSheet>>,
}

/// What a conversion (Scenes → song, List → song, Song → scenes) made
/// and how its result plays differently from what it was made from, for
/// the report card (the Arrange refactor's 4.3, plan D7). Made only when
/// something differs; an exact conversion says so in one line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    /// The conversion, as the card's title: "SONG → SCENES".
    pub title: String,
    /// What it made: "rows 9 to 11", "bars 1 to 76".
    pub made: String,
    /// One line in plain words: "Launch row 9 and the rows play the song,
    /// except:".
    pub lead: String,
    /// Each difference, in song order.
    pub rows: Vec<ReportRow>,
}

/// One difference in a [`Report`]: its track (None: the whole song's),
/// its bar (from 1) and what is different, in the log's words.
#[derive(Clone, Debug, PartialEq)]
pub struct ReportRow {
    pub track: Option<usize>,
    pub bar: Tick,
    pub what: String,
}

/// An audio region: region `index` of audio track `track`
/// (`AudioTracks::regions(track)[index]`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionId {
    pub track: usize,
    pub index: usize,
}

/// What is picked on the Song view: the selection's kind decides the
/// bottom row's buttons and the Q-Links.
/// What a track plays now (the Arrange refactor's D3, "Now"):
/// [`TimeState::plays`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plays {
    Nothing,
    /// The song's blocks on its lane (song mode).
    Song,
    /// The clip in this slot, launched.
    Clip(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// A placement, by its index in the arrangement.
    Placement(usize),
    Region(RegionId),
}

/// The note editor as the screens show it.
#[derive(Clone, Debug, PartialEq)]
pub struct EditInfo {
    pub editor: Editor,
    /// The clip, shared: the time engine sends the same one until the clip
    /// changes, so the screen tells an unchanged clip at no cost
    /// ([`Arc::ptr_eq`]) and rebuilds the roll only when it did.
    pub clip: Arc<Clip>,
    /// The clip's revision: it moves whenever the clip changes (an edit,
    /// a recorded note, UNDO, an edit from Steps), and a tap on a note
    /// carries the one it was made on ([`EditAction::Note`]).
    pub rev: u64,
    pub track_name: String,
    /// A bar, in the clip's ticks.
    pub bar: Tick,
    /// What became of the editor's last touch (piano roll plan 0C.2).
    pub answer: EditAnswer,
    /// The settings its lock lanes show and set (piano roll plan 3A.2):
    /// the Steps' lock lanes for the clip, which the app knows and fills
    /// in (empty from the time engine, and while Steps shows another
    /// clip), shared so an unchanged list costs nothing to tell.
    pub locks: Arc<Vec<LockSetting>>,
    /// The other clips the editor shows faintly behind (plan 4.1,
    /// [`Editor::show`]), those still there and of the clip's kind, each
    /// shared so an unchanged one costs nothing to tell.
    pub ghosts: Vec<Ghost>,
}

/// Another clip the note editor shows faintly behind the one it edits
/// (plan 4.1): where it is, its notes, and its track's name.
#[derive(Clone, Debug, PartialEq)]
pub struct Ghost {
    pub track: usize,
    pub slot: usize,
    pub clip: Arc<Clip>,
    pub track_name: String,
}

/// What became of the note editor's last touch, in the copy of the clip
/// the screen reads (piano roll plan 0C.2), so a roll showing an edit
/// before it goes in knows when it did and where its notes landed, or
/// why it didn't.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditAnswer {
    /// Which touch: the editor's touches counted by the time engine from
    /// its start, so a new number is a new answer (0: none yet).
    pub n: u64,
    /// The clip's revision the touch was carried out on.
    pub from: u64,
    pub outcome: EditOutcome,
}

/// See [`EditAnswer`].
#[derive(Clone, Debug, Default, PartialEq)]
pub enum EditOutcome {
    /// The clip was left as it was: a pick, a tool, a scroll, a setting.
    #[default]
    Unchanged,
    /// The edit went in: the clip is at revision `rev` and the notes it
    /// made are at `landed`, which are the picks after it.
    Taken { rev: u64, landed: Vec<usize> },
    /// Refused or taken back, and why.
    Refused(String),
}

/// A placement as the screens show it.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacementInfo {
    pub track: usize,
    pub start: Tick,
    pub length: Tick,
    /// Its clip's name ("(empty)" for a slot emptied since).
    pub name: String,
    pub unique: bool,
}

/// The scene morph as the screen shows it (`mpfree_perform::SceneMorph`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MorphInfo {
    /// The rows on A and B.
    pub a: usize,
    pub b: usize,
    /// The crossfader, 0 (A) to 1 (B).
    pub pos: f32,
    /// A morph is loaded: the rows' values are on A and B.
    pub on: bool,
    /// Parameters it moves.
    pub params: usize,
}

impl Default for MorphInfo {
    fn default() -> MorphInfo {
        MorphInfo {
            a: 0,
            b: 1,
            pos: 0.0,
            on: false,
            params: 0,
        }
    }
}

impl TimeState {
    /// The selected track: the one the Sequencer's Steps, Notes and STEP
    /// SEQ edit and the pads follow. The armed track unless a track was
    /// selected apart from it while the arm is pinned
    /// ([`TimeState::track_pick`]).
    pub fn selected(&self) -> Option<usize> {
        self.track_pick.or(self.armed)
    }

    /// Put in a cell's cell (audio tracks phase 2, [`pick_cell`]), as
    /// (track, row); None: a new audio track.
    pub fn cell_for_sound(&self) -> Option<(usize, usize)> {
        let cells = self.tracks.iter().map(|t| {
            let empty = t.slots.iter().position(Option::is_none);
            (t.audio, empty)
        });
        pick_cell(self.armed, cells)
    }

    pub fn from_engine(
        transport: TransportInfo,
        project: &Project,
        launches: &[Launch],
        armed: Option<usize>,
        takes: &Takes,
    ) -> TimeState {
        let tracks = project
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| TrackInfo {
                name: t.name.clone(),
                id: t.id,
                sound: String::new(),
                mute: t.mute,
                solo: t.solo,
                silent: project.silent(i),
                pressure: t.record_touch,
                record_quantise: t.record_quantise,
                loops: t.loops(),
                audio: t.is_audio(),
                transpose: t.transpose,
                follows: t.follows,
                melodic: false,
                pad_mutes: t.pad_mutes,
                pad_solos: t.pad_solos,
                // The rows the grid shows (Arrange 1.2: a project keeps
                // more, for scene banks to come).
                slots: t
                    .slots
                    .shown()
                    .map(|s| {
                        s.as_ref().map(|c| ClipInfo {
                            name: c.name.clone(),
                            length: c.length,
                            speed: c.speed,
                            notes: c.notes.len(),
                            looped: c.looped,
                            settings: c.settings(),
                            variation: c.variation(),
                            audio: c.audio.as_ref().map(|a| AudioCellInfo {
                                bpm: a.bpm,
                                gain: a.gain,
                                ..AudioCellInfo::default()
                            }),
                            holds_notes: c.holds == mpfree_seq::Holds::Notes,
                            holds: c.holds,
                            loop_id: c.loop_id(),
                        })
                    })
                    .collect(),
            })
            .collect();
        TimeState {
            transport,
            tracks,
            loops: LoopsInfo::default(),
            launches: launches.to_vec(),
            cues: Vec::new(),
            armed,
            track_pick: None,
            clip_pick: None,
            also: 0,
            clipboard: None,
            notes_copied: None,
            done: takes.done().iter().map(TakeRow::of).collect(),
            undone: takes.undone().iter().map(TakeRow::of).collect(),
            take_txns: TakeTxns::of(takes),
            chips: Vec::new(),
            capture: None,
            scene_playing: None,
            scene_cued: None,
            loop_cell: None,
            loops_on: false,
            scenes: project.scenes.iter().map(|s| s.name.clone()).collect(),
            morph: MorphInfo::default(),
            scene_focus: 0,
            scene_open: None,
            scene: SceneSettings::default(),
            song_view: false,
            list_view: false,
            list: Vec::new(),
            list_pick: None,
            list_looped: false,
            list_linked: false,
            list_next: None,
            song_on: false,
            perform: project.perform,
            launch: project.launch,
            cue_lines: Vec::new(),
            mute_wait: None,
            audio_waits: AudioWaitCounts::default(),
            audio_late: Vec::new(),
            not_ready: Vec::new(),
            fitting: None,
            audio_history: (0, 0),
            audio_focus: None,
            saved: Vec::new(),
            reloads: Vec::new(),
            converting: None,
            placements: project
                .arrangement
                .placements
                .iter()
                .map(|p| PlacementInfo {
                    track: p.track,
                    start: p.start,
                    length: p.length,
                    name: project
                        .block_clip(p)
                        .map_or_else(|| "(empty)".to_string(), |c| c.name.clone()),
                    // Its clip is the song's own (made unique, or a
                    // deleted clip the song still plays).
                    unique: project.arrangement.clips.get(p.clip).is_some(),
                })
                .collect::<Vec<_>>()
                .into(),
            markers: project
                .arrangement
                .markers
                .iter()
                .map(|m| (m.at, m.name.clone()))
                .collect(),
            passes: project
                .arrangement
                .markers
                .iter()
                .map(|m| {
                    m.scene
                        .filter(|f| !f.changed && m.passes > 1)
                        .map(|f| (m.passes, f.pass))
                })
                .collect(),
            scene_sections: mpfree_seq::arrange::scene_sections(
                project,
                transport.meter.bar_ticks(),
            ),
            selected: None,
            picks: Vec::new(),
            section: None,
            song_rec: false,
            song_with_take: false,
            follow_next: None,
            edit: None,
            audio: Vec::new(),
            waves: Vec::new(),
            region: None,
            audio_free: None,
            audio_counts: Vec::new(),
            audio_panel: None,
            audio_takes: None,
            tempo_points: project
                .arrangement
                .tempo
                .iter()
                .map(|p| (p.at, p.bpm, p.ramp))
                .collect(),
            tempo_base: transport.bpm,
            tempo_pick: None,
            report: None,
            tempo_here: None,
            continuing: None,
            adding: None,
            unfit: Arc::from(Vec::new()),
            row_chords: None,
            song_chords: Arc::from(Vec::new()),
            styles: Arc::from(Vec::new()),
            style_sheet: None,
        }
    }

    /// What is picked on the Song view, and its kind. A region wins if
    /// both were ever set.
    pub fn pick(&self) -> Option<Pick> {
        self.region
            .map(Pick::Region)
            .or(self.selected.map(Pick::Placement))
    }

    pub fn clip(&self, track: usize, slot: usize) -> Option<&ClipInfo> {
        self.tracks.get(track)?.slots.get(slot)?.as_ref()
    }

    /// The clip launched on `track`, if any.
    pub fn launch(&self, track: usize) -> Option<&Launch> {
        self.launches.iter().find(|l| l.track == track)
    }

    /// The cue waiting on `track`, if any.
    pub fn cue(&self, track: usize) -> Option<&Cue> {
        self.cues.iter().find(|c| c.track == track)
    }

    /// The focused clip, as (track, slot): the armed track's playing clip,
    /// else its first (the first track's while none is armed). Beats, the
    /// note editor and the clip settings card all show it (the app's
    /// `TimeControl` picks the same one).
    pub fn focus(&self) -> Option<(usize, usize)> {
        // An audio track armed since the note track: its clip (layout
        // spec 4j).
        let track = self.audio_focus.or(self.armed).unwrap_or(0);
        let tr = self.tracks.get(track)?;
        let slot = self
            .launch(track)
            .map(|l| l.slot)
            .or_else(|| tr.slots.iter().position(Option::is_some))?;
        self.clip(track, slot).is_some().then_some((track, slot))
    }

    /// Convert to notes' preview on the focused clip (E1); None when none
    /// is open on it.
    pub fn converting(&self) -> Option<&Converting> {
        let f = self.focus()?;
        self.converting.as_ref().filter(|c| (c.track, c.slot) == f)
    }

    fn track_name(&self, track: usize) -> String {
        self.tracks
            .get(track)
            .map_or_else(|| format!("Track {}", track + 1), |t| t.name.clone())
    }
}

// ---------------------------------------------------------------- actions

/// What Link makes one of (the Arrange refactor's 4b.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkFrom {
    /// Song from the list: the list's steps become the song's first
    /// parts, each made from its scene, and the song's old bars go after
    /// them.
    List,
    /// List from the song: the song's marked parts become the list, as
    /// they are.
    Song,
}

/// What a touch on the time engine's screens asks for. The app applies it.
/// Not `Copy`: the scene editor's settings carry a name.
#[derive(Clone, Debug, PartialEq)]
pub enum TimeAction {
    /// For the audio thread's command queue.
    Transport(Command),
    Undo,
    Redo,
    /// Arm a track for recording, or none. A track armed is selected too
    /// (the Sequencer edits it and the pads follow it), so without a
    /// pinned arm the two never part.
    Arm(Option<usize>),
    /// A track picked (its name on Arrange, the Play page's track switch,
    /// MAIN + a pad, a track on the Pads tab): selected, and armed too
    /// unless REC's arm is pinned, which the app knows; it sends
    /// [`TimeAction::Select`] then. On its own it arms, as
    /// [`TimeAction::Arm`] does.
    Pick(usize),
    /// Select a track without moving the arm (a track picked while the
    /// arm is pinned): Steps, Notes and STEP SEQ edit it and the pads
    /// follow it, while REC still records the armed track. The armed
    /// track itself selects it again.
    Select(usize),
    /// The clip the Sequencer edits on the selected track (refactor C2):
    /// `Some(slot)` edits that slot's clip without launching it, so what
    /// plays keeps playing; `None` edits the clip that plays again (its
    /// first while none does). A track selected again goes back to what
    /// plays.
    EditClip(Option<usize>),
    /// The step size of the clip the Sequencer edits, in ticks: one of
    /// `mpfree_seq::steps::SIZES` (refactor D1). A clip setting UNDO takes
    /// back; the clip's length and its notes' times stay.
    StepSize(Tick),
    /// Arm these tracks too, a bit each (track 0 the lowest), for one take
    /// over several tracks (the MPC's SHIFT + a track's arm): each records
    /// what its MIDI input sends it. The armed track is left out of it.
    ArmAlso(u128),
    /// "Pressure" (pad pressure) for a track: whether recording keeps the pads'
    /// pressure on its notes (`Track::record_touch`).
    RecordPressure {
        track: usize,
        on: bool,
    },
    /// "Quantise" on the record settings (sequencer S3): the grid the
    /// track's recorded notes go in on (`Track::record_quantise`), or none.
    RecordQuantise {
        track: usize,
        grid: Option<u32>,
    },
    /// "Launch" on the record settings (sequencer S4): the project's launch
    /// quantise, when launches, stops and scenes land.
    LaunchGrid(LaunchGrid),
    /// Live transpose (sequencer S4): the pads' Transpose, or the Record
    /// tab's row. `track` (the armed one) and every melodic track that
    /// follows transpose move to `to`, from their next note on; a note
    /// already sounding keeps its pitch.
    Transpose {
        track: usize,
        to: Transpose,
    },
    /// "Follows transpose" for `track`: the pads' Transpose moves it with
    /// the armed track.
    FollowsTranspose {
        track: usize,
        on: bool,
    },
    /// Play this clip from the next bar line (see [`clips_after`]).
    Launch {
        track: usize,
        slot: usize,
    },
    /// Stop the track's clip on the next bar line.
    StopClip(usize),
    /// Take back the launch or stop waiting on the track.
    Uncue(usize),
    /// Make a clip in this empty slot, launch it and arm its track.
    NewClip {
        track: usize,
        slot: usize,
    },
    /// ERASE + a clip: take the clip out of its slot, as a take that UNDO
    /// puts back (Jay after live test 3).
    DeleteClip {
        track: usize,
        slot: usize,
    },
    /// A clip dragged on the Clips grid and dropped on slot `to`, both
    /// (track, slot) (Jay, 14 Nov: drag and drop): it moves into an
    /// empty slot, or changes places with the clip there; with `copy`
    /// (SHIFT held at the lift) a copy goes into an empty slot. One take,
    /// which UNDO takes back.
    DropClip {
        from: (usize, usize),
        to: (usize, usize),
        copy: bool,
    },
    /// A row's number dragged on the Clips grid: row `from` goes in
    /// before row `to` (a gap, 0 to the slots), the rows between moving
    /// one, or with `copy` a copy goes in and the rows under it move down
    /// one. One take, which UNDO takes back.
    DropRow {
        from: usize,
        to: usize,
        copy: bool,
    },
    /// "+ Track": a new empty track at the end (up to 128), armed, as a
    /// take that UNDO takes away again (Jay, 14 Nov: up to 128 tracks).
    AddTrack,
    /// SHIFT + "+ Track": a new empty Loop track at the end ("Loops C"),
    /// as a take that UNDO takes away again (loop-tracks.md part 4).
    AddLoopTrack,
    /// Add an empty audio track at the end of the track list (the audio
    /// tracks rethink, phase 1a). The app refuses it first when every
    /// engine track plays one.
    AddAudioTrack,
    /// Arm an empty cell of a Loop track, or none: the next loop
    /// recorded lands there (loop-tracks.md part 4). The app arms the loop
    /// for REC with it.
    LoopCell(Option<(usize, usize)>),
    /// A Loop track's input to the next one (Pads, Line in, Resample):
    /// what a loop recorded into its cells records.
    LoopInput(usize),
    /// A loop cell copied onto an empty cell of another Loop track
    /// (loop-tracks.md part 6): the cell there takes loop `into`, an empty
    /// loop the looper copies the audio into. Not a take: the looper's
    /// UNDO takes the copy back, and its cell goes with it. The app sends
    /// it for SHIFT + a drop onto another Loop track. With Dev's "Loops on
    /// audio tracks" a loop cell on an audio track copies onto an empty
    /// cell of another audio track the same way, as a new loop: `id` is
    /// its lasting id, which the app gave loop `into` (audio tracks phase
    /// 4b); 0 for a Loop track's copy.
    LoopCopy {
        from: (usize, usize),
        to: (usize, usize),
        into: u8,
        id: u128,
    },
    /// Loop `n`'s copy stopped before any of it went in: its cells empty.
    LoopCopyOff(u8),
    /// Chip `i` on the bar was tapped.
    Chip(usize),
    /// Keep what was just played (CAPTURE): into the armed track's playing
    /// clip, or as a new clip (see `mpfree_capture::keep`).
    Capture,
    /// UNDO while recording: punch out and throw the take so far away (the
    /// music plays on); during the count-in, stop.
    ThrowTake,
    /// Play a whole row of the grid (a scene) from the next bar line, with
    /// its mutes (`mpfree_perform::Perform::launch_scene`).
    LaunchScene(usize),
    /// Stop every track on the next bar line.
    StopAll,
    /// Every track back to the arrangement on the launch line: song mode
    /// on, and every clip launched over the song stopped there (sequencer
    /// S4, "Back to song", Ableton's Back to Arrangement).
    BackToSong,
    /// Keep the clip as it is now (sequencer S4, Elektron's temp save):
    /// Reload clip brings this state back.
    SaveState {
        track: usize,
        slot: usize,
    },
    /// Put the clip's saved state back, on its launch line while it plays
    /// (at once otherwise), as an edit UNDO takes back. Asked again while
    /// it waits, the reload is taken back.
    Reload {
        track: usize,
        slot: usize,
    },
    /// Mutate (sequencer S6, `mpfree_seq::mutate`): a new variation of
    /// the clip's notes, made from the clip as it was before its first
    /// Mutate, which it keeps for Restore; at once, as an edit UNDO takes
    /// back. `more` (SHIFT) varies about half the notes, else a quarter;
    /// `melodic` (its track plays notes, not pads) lets a note move to the
    /// next pitch the clip plays.
    Mutate {
        track: usize,
        slot: usize,
        more: bool,
        melodic: bool,
    },
    /// Restore (S6): the clip as it was before its first Mutate, at once,
    /// as an edit UNDO takes back.
    Restore {
        track: usize,
        slot: usize,
    },
    /// Fit to tempo on an audio clip's card (audio tracks phase 2): the
    /// clip on fitted copies of its recordings, made at the project's
    /// tempo, once they are in, as an edit UNDO takes back.
    FitClip {
        track: usize,
        slot: usize,
    },
    /// An audio clip's card (layout spec 4j): its length, gain or name,
    /// as an edit UNDO takes back (a row turned on joins it). `row` is
    /// the card's row ([`AUDIO_CLIP_ROWS`]).
    AudioClip {
        track: usize,
        slot: usize,
        row: u8,
        set: AudioSet,
    },
    /// The clip card's Phrase row (composing tools, step 3 of the
    /// phrases): the Make a phrase sheet opens on the clip's notes. The
    /// app answers it (the sheet is the arp's); the time engine changes
    /// nothing.
    MakePhrase {
        track: usize,
        slot: usize,
    },
    /// The clip card's Style row (composing step 6, board D): the Make a
    /// style sheet opens on the clip, read from its notes.
    MakeStyle {
        track: usize,
        slot: usize,
    },
    /// A touch on the Make a style sheet, its naming keyboard's Enter, or
    /// the wheel and - / + while it shows.
    Style(crate::style_sheet::StyleAction),
    /// Convert to notes' preview (step sequencer refactor E1,
    /// `mpfree_seq::convert`): what converting a clip that follows the
    /// pads would make, or every reason it can't; nothing changes.
    ConvertPreview {
        track: usize,
        slot: usize,
    },
    /// Convert to notes, after its preview: the clip uses notes, playing
    /// what it plays now, as an edit UNDO takes back. Refused if the clip
    /// or the pads changed since the preview.
    Convert {
        track: usize,
        slot: usize,
    },
    /// Close Convert to notes' preview.
    ConvertCancel,
    /// Continue (composing tools): carry a clip on, hear it in place of
    /// the clip, keep it or go back.
    Continue(crate::continuing::ContinueAction),
    /// Add a part (composing tools S2.6): a part for a row's empty cell,
    /// heard there with the row, kept or left.
    Add(crate::adding::AddAction),
    /// The row's chords outside Add a part: set them, or fit the clips
    /// that no longer fit them (composing tools S2.6).
    Chords(crate::adding::ChordsAction),
    /// Put these rows on the morph's A and B (loaded again if it is on).
    MorphRows {
        a: usize,
        b: usize,
    },
    /// Move the crossfader, 0 (A) to 1 (B); it loads the rows first if the
    /// morph is off.
    Morph(f32),
    /// Let go of the morph, every parameter back to where it was before.
    SnapBack,
    /// Let go of the morph, the parameters staying where it has them.
    KeepMorph,
    /// Store every parameter's value now in this row, for the morph.
    SaveScene(usize),
    /// The scene card (the Arrange refactor's 5.2) opened for row `r`
    /// (its row's number held on the grid), or closed with None.
    SceneCard(Option<usize>),
    /// The scene card's Insert row: an empty row goes in under row `r`,
    /// the rows under it moving down one (the grid's last must be empty).
    /// One take, which UNDO takes back.
    InsertRow(usize),
    /// The scene card's Delete row: row `r` goes, its clips (the song
    /// keeps those it plays) and its scene, and the rows under it move up
    /// one. One take, which UNDO takes back.
    DeleteRow(usize),
    /// Capture scene (the Arrange refactor's 5.3): a copy of the clip
    /// each track plays now goes into the first empty row under the scene
    /// playing (from row 1 with none), as one take that UNDO takes back.
    /// What plays plays on.
    CaptureScene,
    /// Show the Song view (the timeline) or the grid.
    SongView(bool),
    /// The conversion report card closes (its Done, a tap outside it, MENU
    /// or a page change); what the conversion did stays ([`Report`]).
    CloseReport,
    /// The scene editor's row (its Scene row's ‹ ›).
    SceneFocus(usize),
    /// The scene editor: row `row` given `settings` (its name, mutes and
    /// follow action), card row `card_row` turned (`SCENE_NAME` for a
    /// rename). One take a row: turning the same row again joins it.
    SceneSettings {
        row: usize,
        card_row: u8,
        // Boxed: the settings are the biggest thing an action carries.
        settings: Box<SceneSettings>,
    },
    /// Show the song list (Arrange's third view, sequencer S4), or (false)
    /// the grid.
    ListView(bool),
    /// Pick a step of the song list, or none.
    ListPick(Option<usize>),
    /// Play the song list from this step: its scene as a scene launch
    /// does, then each step after it in turn
    /// (`mpfree_perform::Perform::play_list`). Linked to the song (the
    /// Arrange refactor's 4b.4): the song from the part's first bar, in
    /// song mode.
    ListPlay(usize),
    /// Step `step`'s times one more or fewer (1 to 99).
    ListTimes {
        step: usize,
        delta: i32,
    },
    /// Step `step`'s scene the next or the previous row with a clip.
    ListScene {
        step: usize,
        delta: i32,
    },
    /// A step after the one picked (at the end with none): the next row
    /// with a clip after its scene, two times.
    ListAdd,
    /// Take this step out of the list.
    ListRemove(usize),
    /// Step `step` moved to before step `to` (after the last for the
    /// list's length): its grip dragged. A linked list's moves its section
    /// in the song.
    ListMove {
        step: usize,
        to: usize,
    },
    /// Back to the scene (a linked list, the Arrange refactor's 4b.3):
    /// step `k`'s part, changed in the song, made from its scene again.
    ListBack(usize),
    /// The list goes round after its last step, or stops. Linked to the
    /// song (4b.4): in song mode the song loops, first part to last.
    ListLoop(bool),
    /// Link a list that isn't linked to the song (4b.5), from the list or
    /// from the song.
    ListLink(LinkFrom),
    /// The list onto the end of the song, played offline through Rec song
    /// (the Arrange refactor's 4.1, the app's time engine).
    ListToSong,
    /// Song mode on or off: the song plays on tracks with no clip launched.
    SongMode(bool),
    /// Make (false) or Perform (true): the top bar's chip, or SHIFT + MAIN
    /// (sequencer S4). Saved with the project; nothing playing changes.
    Perform(bool),
    /// The rows with clips, in order, onto the end of the song, played
    /// offline through Rec song (the Arrange refactor's 4.1, the app's
    /// time engine).
    ScenesToSong,
    /// The song's sections into a run of empty rows, chained by follow
    /// actions (`mpfree_seq::arrange::song_to_scenes`).
    SongToScenes,
    /// Select a placement on the Song view, or none.
    SelectPlacement(Option<usize>),
    /// SHIFT + a tap on a block (6.5): block `i` added to what is picked,
    /// or taken out of it if it was in.
    PickPlacement(usize),
    /// A box dragged across the Song view's lanes (6.5): blocks `set`
    /// picked, or with `add` (SHIFT held as the finger lifts) added to
    /// what is picked.
    PickPlacements {
        set: Vec<usize>,
        add: bool,
    },
    /// A block dragged on the Song view (Jay, 14 Nov: drag and drop), or
    /// moved a beat by the wheel: placement `index` to `track` from
    /// `start`, or with `copy` (SHIFT held as the finger lifts) a copy of
    /// it there.
    MovePlacement {
        index: usize,
        track: usize,
        start: Tick,
        copy: bool,
    },
    /// The picked block's end dragged: placement `index` made `length`
    /// song ticks long.
    StretchPlacement {
        index: usize,
        length: Tick,
    },
    /// A section's name dragged along the ruler, or moved by the wheel:
    /// section `marker` to before section `gap` (after the last for the
    /// markers' count), or with `copy` a copy of it there
    /// (`Arrangement::move_section`).
    MoveSection {
        marker: usize,
        gap: usize,
        copy: bool,
    },
    /// A section's name tapped: pick it (the wheel's push then grabs it).
    PickSection(Option<usize>),
    /// Bars put in or taken out of the song (Arrange 3.3), everything
    /// after moving with them.
    SongBars(BarsEdit),
    /// The song's markers edited (Arrange 3.3): only the markers change.
    SongMarker(MarkerEdit),
    /// Give a placement its own copy of its clip.
    MakeUnique(usize),
    DeletePlacement(usize),
    /// Split at the playhead (the Arrange refactor's phase 6): block
    /// `Some(i)` cut there, or with `None` every block across it.
    SplitPlacement(Option<usize>),
    /// A copy of block `i` right after it (Duplicate, phase 6).
    DuplicatePlacement(usize),
    /// Trim from the left (phase 6): placement `index` starts at song tick
    /// `start`, its end staying, and what it plays stays where it was.
    TrimPlacement {
        index: usize,
        start: Tick,
    },
    /// What placement `index` plays slid `by` song ticks later under it
    /// (earlier for a negative `by`; phase 6).
    SlidePlacement {
        index: usize,
        by: Tick,
    },
    /// Several blocks picked (phase 6): blocks `set` moved together
    /// `lanes` lanes down and `by` song ticks later, or with `copy`
    /// copies of them put there, keeping their places to each other
    /// (`Arrangement::move_placements`).
    MovePlacements {
        set: Vec<usize>,
        lanes: i32,
        by: Tick,
        copy: bool,
    },
    /// Copies of blocks `set` right after them, as one (Duplicate with
    /// several picked).
    DuplicatePlacements(Vec<usize>),
    /// Blocks `set` deleted together.
    DeletePlacements(Vec<usize>),
    /// Split with several blocks picked (6.5): those of blocks `set`
    /// across the playhead cut there, the parts after picked.
    SplitPlacements(Vec<usize>),
    /// Live to song on or off: while on and the transport plays, what the
    /// grid plays is recorded into the song
    /// (`mpfree_perform::SongRecorder`).
    RecSong(bool),
    /// Perform's Rec song with REC (sequencer S4): a take turns Rec song
    /// on for its length, or (false) not this take.
    RecSongWithTake(bool),
    /// The clip settings card: clip `slot` of `track` given `settings`
    /// (its speed, swing, quantise and follow action), card row `row`
    /// turned. One take a row: turning the same row again joins it.
    ClipSettings {
        track: usize,
        slot: usize,
        row: u8,
        settings: ClipSettings,
    },
    /// The card's Length row: the clip made `steps` sixteenths long, as
    /// Beats' length does (the notes past the end come back on UNDO); with
    /// `copy` (SHIFT + its ›, doubling) the new steps repeat the notes
    /// before them.
    ClipLength {
        track: usize,
        slot: usize,
        steps: usize,
        copy: bool,
    },
    /// COPY + a clip: keep a copy of it for COPY + an empty slot.
    CopyClip {
        track: usize,
        slot: usize,
    },
    /// COPY + an empty slot: the clip copied last goes there, as a take
    /// UNDO takes away.
    PasteClip {
        track: usize,
        slot: usize,
    },
    /// A touch on the note editor (`crate::editor`).
    Edit(EditAction),
    /// A touch on the Song view's audio lanes, for the audio tracks
    /// (`mpfree_audiotracks`).
    Audio(AudioAction),
    /// The song's tempo points (tempo ramps): the Song view's tempo lane,
    /// the wheel on a picked point, and the tempo turned in song mode.
    SongTempo(TempoEdit),
}

/// An edit of the song's tempo points (`mpfree_seq::arrange::TempoPoint`),
/// each one UNDO takes back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TempoEdit {
    /// A tap on the tempo lane: a point at song tick `at` (a bar line)
    /// with the tempo playing there, picked.
    Add(Tick),
    /// Point `k` picked (the wheel then sets its tempo, buttons 5 and 6
    /// read Ramp and Delete), or none.
    Pick(Option<usize>),
    /// Point `index` dragged to song tick `at`.
    Move {
        index: usize,
        at: Tick,
    },
    /// Point `index` given the tempo `bpm` (the wheel, with it picked).
    Bpm {
        index: usize,
        bpm: f64,
    },
    /// Point `index` glides to the next point's tempo, or steps.
    Ramp {
        index: usize,
        on: bool,
    },
    Delete(usize),
    /// The tempo turned in song mode while the song's points play (Jay's
    /// card, 18 Nov): the point in effect at the playhead takes `bpm`, or
    /// before the first point a new one at the playhead's bar.
    AtPlayhead(f64),
}

/// Bars put in or taken out of the song (Arrange 3.3): one
/// `mpfree_seq::TimeEdit`, so the blocks, markers, tempo points, morph
/// moves and rows after it all move, and UNDO takes it back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BarsEdit {
    /// `bars` empty bars put in at song tick `at` (a bar line): the room
    /// holds the tempo and the morph as they were just before it.
    Insert { at: Tick, bars: u32 },
    /// The bars from song tick `from` to `to` taken out (bar lines), and
    /// everything after closed up.
    Delete { from: Tick, to: Tick },
    /// The Song view's + Bar, section `k` picked: an empty bar put in just
    /// before it. Presses in a row on the same section are one UNDO.
    RoomBefore(usize),
    /// The Song view's Delete bars, section `k` picked: its bars
    /// (`Arrangement::section`) taken out.
    DeleteSection(usize),
}

/// An edit of the song's markers (Arrange 3.3), each one UNDO takes back.
/// Only the markers change: to move what plays, move a section.
#[derive(Clone, Debug, PartialEq)]
pub enum MarkerEdit {
    /// A new marker on song tick `at` (a bar line), named with the next
    /// free letter (`Arrangement::new_marker_name`), picked.
    Add(Tick),
    /// Marker `index` given a new name.
    Rename { index: usize, name: String },
    /// Marker `index` moved to song tick `at` (a bar line): its section's
    /// edge moves, nothing else.
    Move { index: usize, at: Tick },
    /// Marker `index` taken away: its bars join the section before.
    Delete(usize),
}

/// What a touch on the Song view's audio lanes asks for (layout spec 4c).
/// The app turns them into `mpfree_audiotracks::Command`s and its panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioAction {
    /// The header's arm dot: arm or disarm the audio track.
    Arm {
        track: usize,
        on: bool,
    },
    /// The header's name or input chip: open the track's panel (Arm,
    /// Input, Monitor, level, pan, mute, solo, Takes) over the lanes, or
    /// close it if it is that track's.
    Panel(usize),
    /// The panel's Close.
    ClosePanel,
    /// The panel's "Its strip in Mix ›": open the Mix tab with the track's
    /// own strip picked (the audio tracks rethink, phase 1b).
    Strip(usize),
    /// The panel: what the track records from, when it hears its input,
    /// and its mute and solo.
    Input {
        track: usize,
        input: Input,
    },
    Monitor {
        track: usize,
        monitor: Monitor,
    },
    /// The panel's Mix and its From steps: record from inside the mix (the
    /// instrument redesign's step 9), stepping through what the mix
    /// offers, `step` either way (0, Mix itself: the one picked last, or
    /// the first). The app keeps the list.
    Mix {
        track: usize,
        step: i32,
    },
    /// The panel's Effects while the track records from the mix: before
    /// or after the source's effects.
    MixEffects {
        track: usize,
        after: bool,
    },
    Mute {
        track: usize,
        on: bool,
    },
    Solo {
        track: usize,
        on: bool,
    },
    /// With Dev's "Loops on audio tracks" on (phase 4, board 4): open the
    /// track's input list. The app works out what the mixer offers it
    /// and refuses ([`crate::inputs::InputChoices`]) and the screen shows
    /// them.
    Inputs(usize),
    /// A pick on the input list the app offered last: `k` is the list's
    /// `Select` index ([`crate::inputs::LINE`] + k, `PADS`, `MASTER`,
    /// `STRIP` + i).
    InputPick {
        track: usize,
        k: usize,
    },
    /// Pick a region, or none. Picking one drops the placement picked.
    Pick(Option<RegionId>),
    /// The bottom row with a region picked: Split (at the playhead),
    /// Erase, Fades, Gain, Takes (the track's take lanes) and Undo (SHIFT
    /// + Undo redoes).
    Split(RegionId),
    Erase(RegionId),
    Fades(RegionId),
    Gain(RegionId),
    /// Show the track's take lanes under its lane, or hide them if they
    /// show.
    Takes(usize),
    /// To a cell (audio tracks phase 2): the region as an audio clip in
    /// its track's first empty cell, the song unchanged (the app builds
    /// the clip from the region and puts it in).
    ToCell(RegionId),
    /// UNDO on Arrange while the region edits changed last (the app sends
    /// it, layout spec 4c): take back the last.
    Undo,
    /// SHIFT + UNDO the same way: bring back what Undo took.
    Redo,
    /// A swipe across a take lane: play take `take` from `from` to `to`
    /// on its track, over what was there (`Command::Comp`), snapped to the
    /// grid unless `fine` (SHIFT held: the app sets it).
    Comp {
        take: u16,
        from: Tick,
        to: Tick,
        fine: bool,
    },
    /// A tap on a take lane: play the take everywhere it has audio.
    UseTake(u16),
    /// A region dragged sideways: move it to start at `to`, snapped to
    /// the grid unless `fine` (SHIFT held: the app sets it).
    Move {
        region: RegionId,
        to: Tick,
        fine: bool,
    },
    /// A Q-Link on Arrange while an audio region is picked or the track
    /// panel is open (layout spec 4c), or the panel's level and pan
    /// steps: knob 0 the track's level, 1 its pan, 2 the region's gain, 3
    /// its fades (`region`, an index into the track's regions: None when
    /// none is picked on the track); `ticks` detents, `fine` with SHIFT
    /// held. The app keeps the values and says them back in the lanes.
    Turn {
        track: usize,
        region: Option<usize>,
        knob: usize,
        ticks: i32,
        fine: bool,
    },
}

/// Where a clip launched now starts: on the next bar line while the
/// transport rolls (now, if the position is on one), else at the start of
/// the bar the position is in.
pub fn launch_origin(t: &TransportInfo) -> Tick {
    if t.state.rolling() {
        t.meter.next_bar(t.position)
    } else {
        t.position - t.position.rem_euclid(t.meter.bar_ticks())
    }
}

/// The launches and cues after `action`.
///
/// - While the transport rolls, Launch and StopClip become a [`Cue`] for
///   the next bar line, in place of the track's cue. The player starts it
///   there, and the app applies [`mpfree_play::Started`] to these lists.
/// - While it is stopped, they happen at once. A launched clip plays as if
///   it had started on the line of the bar the position is in, wherever
///   the song goes after.
/// - Uncue takes the track's cue back.
/// - NewClip launches its new clip at once from [`launch_origin`] (it
///   records from there), and takes the track's cue back.
pub fn clips_after(
    project: &Project,
    launches: &[Launch],
    cues: &[Cue],
    action: &TimeAction,
    t: &TransportInfo,
) -> (Vec<Launch>, Vec<Cue>) {
    let (mut launches, mut cues) = (launches.to_vec(), cues.to_vec());
    let rolling = t.state.rolling();
    let (track, slot) = match *action {
        TimeAction::Launch { track, slot } => (track, Some(slot)),
        TimeAction::StopClip(track) => (track, None),
        TimeAction::Uncue(track) => {
            cues.retain(|c| c.track != track);
            return (launches, cues);
        }
        TimeAction::NewClip { track, .. } => {
            cues.retain(|c| c.track != track);
            return (launches_after(&launches, action, t), cues);
        }
        _ => return (launches, cues),
    };
    cues.retain(|c| c.track != track);
    if rolling {
        cues.push(Cue { track, slot });
        return (launches, cues);
    }
    launches.retain(|l| l.track != track);
    if let Some((slot, clip)) = slot.and_then(|s| Some((s, project.clip(track, s)?))) {
        let bar = t.position - t.position.rem_euclid(t.meter.bar_ticks());
        launches.push(Launch {
            track,
            slot,
            origin: anchored(bar, clip.length, clip.speed),
            start: bar,
        });
    }
    (launches, cues)
}

/// Where a cue landing on `line` starts: its next line the playhead
/// plays. In a loop whose end comes first, that is the loop start's.
pub fn cue_tick(t: &TransportInfo, line: LaunchGrid) -> Tick {
    let bar = t.meter.bar_ticks();
    let next = line.next(t.position, bar);
    match t.playing_loop() {
        Some((a, b)) if t.state.rolling() && t.position < b && next >= b => line.next(a, bar),
        _ => next,
    }
}

impl TimeState {
    /// The lane of the audio track in place `track` of the track list, if
    /// it is one.
    pub fn audio_lane_at(&self, track: usize) -> Option<&AudioLane> {
        if !self.tracks.get(track).is_some_and(|t| t.audio) {
            return None;
        }
        self.audio.iter().find(|l| l.row == track)
    }

    /// The line the cue on `track` lands on: as the time engine set it
    /// ([`TimeState::cue_lines`]), else the cued clip's own launch
    /// quantise, else the project's.
    pub fn cue_line(&self, track: usize) -> LaunchGrid {
        if let Some((_, g)) = self.cue_lines.iter().find(|(t, _)| *t == track) {
            return *g;
        }
        let own = self
            .cue(track)
            .and_then(|c| c.slot)
            .and_then(|s| self.clip(track, s))
            .and_then(|c| c.settings.launch);
        own.unwrap_or(self.launch)
    }
}

/// The launches after `action`, without cues: Launch replaces the track's
/// clip with one starting at [`launch_origin`]; StopClip removes it. Other
/// actions leave them as they are. The old clip stops when the new view is
/// swapped in, not on the bar: [`clips_after`] does it properly.
pub fn launches_after(launches: &[Launch], action: &TimeAction, t: &TransportInfo) -> Vec<Launch> {
    let mut out = launches.to_vec();
    match *action {
        TimeAction::Launch { track, slot } | TimeAction::NewClip { track, slot } => {
            out.retain(|l| l.track != track);
            out.push(Launch {
                track,
                slot,
                origin: launch_origin(t),
                start: launch_origin(t),
            });
        }
        TimeAction::StopClip(track) => out.retain(|l| l.track != track),
        _ => {}
    }
    out
}

/// What a touch means, given what the screen shows.
/// A tap on a track's name on the Clips grid: it picks the track
/// ([`TimeAction::Pick`]), or lets go of the arm when it has it.
pub fn name_tap(st: &TimeState, track: usize) -> TimeAction {
    // An audio track's name arms it to record (layout spec 4f), as its
    // arm dot on the Song view does.
    if let Some(l) = st.audio_lane_at(track) {
        return TimeAction::Audio(AudioAction::Arm {
            track: l.track,
            on: !l.armed,
        });
    }
    if st.armed == Some(track) {
        TimeAction::Arm(None)
    } else {
        TimeAction::Pick(track)
    }
}

/// A tap on the Clips grid's cell at (`track`, `slot`): launch its clip,
/// stop it while it plays, take back a launch or stop waiting, make a
/// clip in an empty slot, or arm an empty Loop cell. None past the
/// tracks.
pub fn cell_tap(st: &TimeState, track: usize, slot: usize) -> Option<TimeAction> {
    if track >= st.tracks.len() || slot >= ROWS {
        None
    } else if st.tracks[track].audio && st.clip(track, slot).is_none() {
        // With Dev's "Loops on audio tracks" (phase 4, board 2), an empty
        // cell of the armed audio track arms (again: lets go of) a loop
        // there. Otherwise nothing records into an audio cell: [`why_not`]
        // says how audio gets there.
        let armed = st.audio_lane_at(track).is_some_and(|l| l.armed);
        let cell = (track, slot);
        (st.loops_on && armed)
            .then(|| TimeAction::LoopCell((st.loop_cell != Some(cell)).then_some(cell)))
    } else if st.clip(track, slot).is_none() && st.tracks[track].loops.is_some() {
        // An empty Loop cell arms (again: lets go of) a loop there.
        let cell = (track, slot);
        Some(TimeAction::LoopCell(
            (st.loop_cell != Some(cell)).then_some(cell),
        ))
    } else if st.clip(track, slot).is_none() {
        Some(TimeAction::NewClip { track, slot })
    } else if st.cue(track).is_some_and(|c| {
        c.slot == Some(slot)
            || (c.slot.is_none() && st.launch(track).is_some_and(|l| l.slot == slot))
    }) {
        // Tapping what is cued takes it back.
        Some(TimeAction::Uncue(track))
    } else if st.launch(track).is_some_and(|l| l.slot == slot) {
        Some(TimeAction::StopClip(track))
    } else {
        Some(TimeAction::Launch { track, slot })
    }
}

/// SHIFT + a tap on the Clips grid (sequencer S5, Jay's card of 18 Nov
/// 2026): what MIDI Learn waits to give a control. A clip's cell (its
/// tap, whatever the cell holds), a row's scene button, Stop all, Back to
/// song, Perform's Capture scene (lit or faint), or a track's name (its
/// arm). None for anything else on the grid,
/// where SHIFT keeps its job (a drag copies, "+ Track" adds a Loop track).
pub fn learn_target(ev: &UiEvent, st: &TimeState) -> Option<ParamAddr> {
    if ev.widget != CLIPS {
        return None;
    }
    let id = match ev.kind {
        EventKind::Select(i) if i / ROWS < st.tracks.len() => {
            clips::clip((i / ROWS) as u8, (i % ROWS) as u8)
        }
        EventKind::Toggle(r) if r < ROWS => clips::scene(r as u8),
        EventKind::Toggle(ROWS) => clips::STOP_ALL,
        EventKind::Toggle(r) if r == ROWS + BACK_TO_SONG => clips::BACK_TO_SONG,
        EventKind::Toggle(r) if r == ROWS + CAPTURE => clips::CAPTURE_SCENE,
        EventKind::Tab(t) if t < st.tracks.len() => clips::arm(t as u8),
        _ => return None,
    };
    Some(ParamAddr::new(Owner::Clips, id))
}

/// A learned control's press on [`Owner::Clips`] parameter `c` (MIDI
/// Learn's remote control): what a tap on the grid does now. None for a
/// track or row that isn't there, and for mutes and solos, which take a
/// value and go through the app's mute queue.
pub fn clips_press(st: &TimeState, c: Clips) -> Option<TimeAction> {
    let track = |t: u8| Some(usize::from(t)).filter(|&t| t < st.tracks.len());
    match c {
        Clips::StopAll => Some(TimeAction::StopAll),
        Clips::BackToSong => Some(TimeAction::BackToSong),
        Clips::CaptureScene => Some(TimeAction::CaptureScene),
        Clips::Scene(r) => Some(usize::from(r))
            .filter(|&r| r < ROWS)
            .map(TimeAction::LaunchScene),
        Clips::Clip(t, r) => cell_tap(st, usize::from(t), usize::from(r)),
        Clips::Stop(t) => track(t).map(TimeAction::StopClip),
        Clips::Arm(t) => track(t).map(|t| name_tap(st, t)),
        Clips::Mute(_) | Clips::Solo(_) => None,
    }
}

pub fn action(ev: &UiEvent, st: &TimeState) -> Option<TimeAction> {
    match record_event(ev, st) {
        Some(UiEvent {
            kind: EventKind::Step {
                index: MAKES_ROW, ..
            },
            ..
        }) => None,
        Some(ev) => action_of(&ev, st),
        None => action_of(ev, st),
    }
}

/// [`action`], with a Record step at its place in [`RECORD_ROWS`].
fn action_of(ev: &UiEvent, st: &TimeState) -> Option<TimeAction> {
    let t = &st.transport;
    match (ev.widget.as_str(), &ev.kind) {
        // The Make a style sheet: its Name button is the app's (its naming
        // keyboard).
        (STYLE_SHEET, kind) => crate::style_sheet::StyleAction::of(kind).map(TimeAction::Style),
        (TRANSPORT, EventKind::Tab(i)) => Some(match BarButton::ALL.get(*i)? {
            BarButton::Stop => TimeAction::Transport(Command::Stop),
            BarButton::Play => TimeAction::Transport(Command::Play),
            BarButton::Rec => TimeAction::Transport(Command::Record),
            BarButton::Mode => TimeAction::Transport(Command::ToggleOverdub),
            BarButton::Undo => TimeAction::Undo,
        }),
        (TRANSPORT, EventKind::Select(i)) => Some(TimeAction::Chip(*i)),
        (MORPH, EventKind::Step { index, delta }) => {
            let m = st.morph;
            let step = |r: usize| {
                (r as i64 + i64::from((*delta).signum())).rem_euclid(ROWS as i64) as usize
            };
            Some(match index {
                0 => TimeAction::MorphRows {
                    a: step(m.a),
                    b: m.b,
                },
                _ => TimeAction::MorphRows {
                    a: m.a,
                    b: step(m.b),
                },
            })
        }
        (MORPH, EventKind::Value(x)) => Some(TimeAction::Morph(*x)),
        (MORPH, EventKind::Tab(0)) => st.morph.on.then_some(TimeAction::SnapBack),
        (MORPH, EventKind::Tab(1)) => st.morph.on.then_some(TimeAction::KeepMorph),
        (MORPH, EventKind::Tab(2)) => Some(TimeAction::SaveScene(if st.morph.pos < 0.5 {
            st.morph.a
        } else {
            st.morph.b
        })),
        (MORPH, EventKind::Tab(3)) => st
            .focus()
            .filter(|f| st.saved.contains(f))
            .map(|(track, slot)| TimeAction::Reload { track, slot }),
        // The scene card, over the grid or the list (5.2).
        (CLIPS | SONG_LIST, EventKind::Toggle(k)) if st.scene_open.is_some() => {
            scene_card_action(st, *k).ok()
        }
        (CLIPS | SONG_LIST, EventKind::Step { index, delta }) if st.scene_open.is_some() => {
            scene_step(st, *index, *delta).ok()
        }
        (CLIPS, EventKind::Toggle(row)) => Some(match *row {
            r if r < ROWS => TimeAction::LaunchScene(r),
            ROWS => TimeAction::StopAll,
            r if r == ROWS + 1 => TimeAction::SongView(true),
            r if r == ROWS + 2 => TimeAction::RecSong(!st.song_rec),
            r if r == ROWS + 3 => TimeAction::Edit(EditAction::Open),
            r if r == ROWS + ADD_TRACK => TimeAction::AddTrack,
            // The "+ Track" chooser's rows (layout spec 4c); a touch off
            // it is the page's own (TimePage::chooser).
            r if r == ROWS + ADD_KIND => TimeAction::AddTrack,
            r if r == ROWS + ADD_KIND + 1 => TimeAction::AddAudioTrack,
            r if r == ROWS + ADD_KIND + 2 => TimeAction::AddLoopTrack,
            r if r == ROWS + BACK_TO_SONG => TimeAction::BackToSong,
            r if r == ROWS + crate::widgets::LIST_VIEW => TimeAction::ListView(true),
            // Capture scene, faint (nothing to keep) unless something
            // plays (5.3).
            r if r == ROWS + CAPTURE && capture_can(st) => TimeAction::CaptureScene,
            // The pager and the drag's edges are the page's own:
            // TimePage::page_tracks.
            _ => return None,
        }),
        // A drop on the grid: a move or a swap; the app makes it a copy
        // with SHIFT held.
        (CLIPS, EventKind::Drop { from, to }) if *from >= ROW_DROP => Some(TimeAction::DropRow {
            from: from - ROW_DROP,
            to: *to,
            copy: false,
        }),
        (CLIPS, EventKind::Drop { from, to }) => Some(TimeAction::DropClip {
            from: (from / ROWS, from % ROWS),
            to: (to / ROWS, to % ROWS),
            copy: false,
        }),
        // The page's own: the Edit button opens the drawer, and the view
        // is the screen's.
        (NOTES, EventKind::Tab(EDIT_BUTTON) | EventKind::View { .. }) => None,
        (NOTES, kind) => Some(TimeAction::Edit({
            // A touch naming a note carries the picture it was made on.
            let rev = st.edit.as_ref().map_or(0, |e| e.rev);
            match *kind {
                // The Pads write chip (plan 1C.2).
                EventKind::Tab(PADS_WRITE) => EditAction::PadsWrite,
                // The Show menu and another clip's faint note (plan 4.1).
                EventKind::Tab(SHOW_NONE) => EditAction::ShowNone,
                // Continue's picks (composing tools, phase 1 step 3).
                EventKind::Tab(k) if k >= CONTINUE_NOTE => {
                    EditAction::ContinueNote(Tick::try_from(k - CONTINUE_NOTE).ok()?)
                }
                EventKind::Tab(k) if k >= CONTINUE_BAR => {
                    EditAction::ContinueBar(u16::try_from(k - CONTINUE_BAR).ok()?)
                }
                EventKind::Tab(k) if k >= GHOST_TAP => {
                    let g = st.edit.as_ref()?.ghosts.get(k - GHOST_TAP)?;
                    EditAction::Switch {
                        track: g.track,
                        slot: g.slot,
                    }
                }
                EventKind::Tab(k) if k >= SHOW_MENU => {
                    let e = st.edit.as_ref()?;
                    let o = show_offers(e, st).into_iter().nth(k - SHOW_MENU)?;
                    EditAction::Show {
                        track: o.track,
                        slot: o.slot,
                    }
                }
                // Try again's ▾ menu (S5): an aimed try.
                EventKind::Tab(k) if k >= AIM_MENU => {
                    let i = k - AIM_MENU;
                    if i >= mpfree_compose::Aim::ALL.len() {
                        return None;
                    }
                    EditAction::Tool(ToolAct::Aim(i))
                }
                // The open tool's bar (plan 2B.3).
                EventKind::Tab(k) if k >= TOOL_BUTTONS => {
                    EditAction::Tool(match k - TOOL_BUTTONS {
                        TOOL_CANCEL => ToolAct::Cancel,
                        TOOL_OPTION => ToolAct::Option,
                        TOOL_DONE => ToolAct::Done,
                        _ => return None,
                    })
                }
                // The Chord menu (plan 2B.2): a shape keeps Major or
                // Minor, and they keep the shape.
                EventKind::Tab(k) if k >= CHORD_MENU => {
                    let ed = &st.edit.as_ref()?.editor;
                    match k - CHORD_MENU {
                        CHORD_MAJOR => EditAction::Chord {
                            shape: ed.chord,
                            minor: false,
                        },
                        CHORD_MINOR => EditAction::Chord {
                            shape: ed.chord,
                            minor: true,
                        },
                        i => EditAction::Chord {
                            shape: *ChordShape::ALL.get(i)?,
                            minor: ed.minor,
                        },
                    }
                }
                // The Scale menu (plan 2B.1): a scale, by its place among
                // the editor's offers, or a root.
                EventKind::Tab(k) if k >= SCALE_MENU => {
                    let e = st.edit.as_ref()?;
                    EditAction::Scale(e.editor.offers.scale_of(k - SCALE_MENU, e.clip.scale)?)
                }
                EventKind::Tab(k) if k >= ROOT_MENU => {
                    EditAction::Root(u8::try_from(k - ROOT_MENU).ok()?)
                }
                EventKind::Tab(k) if k >= MENU => EditAction::Choose(k - MENU),
                EventKind::Tab(k) => EditAction::Button(k),
                EventKind::Select(i) => EditAction::Note { index: i, rev },
                EventKind::Toggle(i) => EditAction::Tap { index: i, rev },
                EventKind::Cell { row: PEN, col } => EditAction::Step(col as Tick),
                EventKind::Cell { row: ADD, col } => EditAction::Add { index: col, rev },
                EventKind::Cell { row, col } => EditAction::Empty {
                    tick: col as Tick,
                    note: u8::try_from(row).ok()?,
                },
                EventKind::Span { index, from, to } => match box_of(index) {
                    Some((low, high)) => EditAction::Pick {
                        from,
                        to,
                        low,
                        high,
                    },
                    None => EditAction::Draw {
                        tick: from,
                        note: u8::try_from(index).ok()?,
                        len: to - from,
                    },
                },
                // The clip's end handle (plan 1A.2).
                EventKind::Stretch {
                    index: CLIP_END,
                    to,
                } => EditAction::Length { to },
                EventKind::Stretch { index, to } => EditAction::Resize {
                    index: index % START_HANDLE,
                    rev,
                    start: index >= START_HANDLE,
                    ticks: to,
                },
                EventKind::Marker { index, frame } => {
                    let (ticks, notes, scale) = drop_of(frame);
                    EditAction::Drop {
                        index,
                        rev,
                        ticks,
                        notes,
                        scale,
                    }
                }
                // A finger on a note and the wheel (plan 1C.3): its
                // velocity, the hold's first detent naming the note.
                // SHIFT + the push on it: its velocity back to a drawn
                // note's (Jay's card, 23 Nov 2026).
                EventKind::Values { index, ref set }
                    if index & !(HELD_FIRST | HELD_FINE) == HELD_TURN + HELD_RESET =>
                {
                    let (&(note, _), &(turn, _)) = (set.first()?, set.get(1)?);
                    EditAction::HeldReset {
                        index: note,
                        rev,
                        turn: turn as u64,
                        first: index & HELD_FIRST != 0,
                    }
                }
                EventKind::Values { index, ref set }
                    if index & !(HELD_FIRST | HELD_FINE) == HELD_TURN =>
                {
                    let (&(note, ticks), &(turn, _)) = (set.first()?, set.get(1)?);
                    EditAction::Held {
                        index: note,
                        rev,
                        ticks,
                        fine: index & HELD_FINE != 0,
                        turn: turn as u64,
                        first: index & HELD_FIRST != 0,
                    }
                }
                // ERASE + a drag across the roll (plan 1B.5): the notes
                // its path crossed.
                EventKind::Values { index, ref set } if index == ERASE_SWEEP => EditAction::Sweep {
                    notes: set.iter().map(|&(i, _)| i).collect(),
                    rev,
                },
                // The Pressure lane (plan 3A.1): its columns' (tick,
                // value), then the stroke's end.
                EventKind::Values { index, ref set } if index == PRESSURE_LANE => {
                    let (&(end, PRESSURE_END), points) = set.split_last()? else {
                        return None;
                    };
                    EditAction::Pressure {
                        rev,
                        points: points
                            .iter()
                            .map(|&(t, v)| (t as Tick, v.clamp(0, 127) as u8))
                            .collect(),
                        end: end as Tick,
                    }
                }
                // On or Remove on Continue's lane (composing tools S3):
                // which lane, and what to do with it.
                EventKind::Values { index, ref set } if index == SOUND_LANE => {
                    let &(k, op) = set.first()?;
                    let e = st.edit.as_ref()?;
                    EditAction::SoundLane {
                        rev,
                        addr: crate::editor::SoundLane::of(&e.clip).get(k)?.addr,
                        on: match op {
                            SOUND_ON => Some(true),
                            SOUND_OFF => Some(false),
                            _ => None,
                        },
                    }
                }
                // A lock lane (plan 3A.2): each note the stroke set, with
                // the setting's value at its place; a note whose lock is
                // already that is left be, so a stroke never nudges a
                // lock it only crossed.
                EventKind::Values { index, ref set } if index >= LOCK_LANES => {
                    let e = st.edit.as_ref()?;
                    let s = e.locks.get(index - LOCK_LANES)?;
                    let set: Vec<(usize, f32)> = set
                        .iter()
                        .filter_map(|&(i, k)| {
                            let k = usize::try_from(k).ok()?;
                            let now =
                                mpfree_seq::notes::lock_of(&e.clip, e.clip.notes.get(i)?, s.addr);
                            (now.map(|v| s.index_of(v)) != Some(k))
                                .then_some((i, s.values.get(k)?.0))
                        })
                        .collect();
                    if set.is_empty() {
                        return None;
                    }
                    EditAction::Lock {
                        rev,
                        addr: s.addr,
                        set,
                    }
                }
                EventKind::Values { index, ref set } => EditAction::Lane {
                    field: *FIELDS.get(index)?,
                    rev,
                    set: set.clone(),
                },
                _ => return None,
            }
        })),
        (EDIT_OPTIONS, EventKind::Step { index, delta }) => {
            Some(TimeAction::Edit(EditAction::Row {
                row: *index,
                delta: *delta,
            }))
        }
        (EDIT_OPTIONS, EventKind::Select(i)) => Some(TimeAction::Edit(EditAction::PickRow(*i))),
        // The conversion report card over the Song view or the song list
        // (layout spec 4n): Done closes it, UNDO takes the conversion back.
        (SONG | SONG_LIST, EventKind::Toggle(REPORT_DONE)) => Some(TimeAction::CloseReport),
        (SONG | SONG_LIST, EventKind::Toggle(REPORT_UNDO)) => Some(TimeAction::Undo),
        // The drawer's Correct: Timing Correct, as TC; while Continue is
        // open, its Lock (step 3).
        (EDIT_OPTIONS, EventKind::Toggle(0)) => Some(TimeAction::Edit(
            match st.edit.as_ref().is_some_and(|e| e.editor.continuing_open()) {
                true => EditAction::ContinueLock,
                false => EditAction::Correct,
            },
        )),
        // The tempo lane: a tap adds a point, a tap on one picks it (again:
        // lets go), a drag moves it.
        (
            SONG,
            EventKind::Place {
                index: NONE,
                row: TEMPO_ROW,
                at,
            },
        ) => Some(TimeAction::SongTempo(TempoEdit::Add(*at))),
        (
            SONG,
            EventKind::Place {
                index,
                row: TEMPO_ROW,
                at,
            },
        ) => Some(TimeAction::SongTempo(TempoEdit::Move {
            index: *index,
            at: *at,
        })),
        (
            SONG,
            EventKind::Cell {
                row: TEMPO_ROW,
                col,
            },
        ) => Some(TimeAction::SongTempo(TempoEdit::Pick(
            (st.tempo_pick != Some(*col)).then_some(*col),
        ))),
        // The picked block's start grip and a slide (phase 6).
        (
            SONG,
            EventKind::Place {
                index,
                row: START_ROW,
                at,
            },
        ) => Some(TimeAction::TrimPlacement {
            index: *index,
            start: *at,
        }),
        (
            SONG,
            EventKind::Place {
                index,
                row: SLIDE_ROW,
                at,
            },
        ) => Some(TimeAction::SlidePlacement {
            index: *index,
            by: *at,
        }),
        // The ruler: a tap where no name is adds a marker; the picked
        // section's grip moves its marker alone (Arrange 3.3).
        (
            SONG,
            EventKind::Place {
                index: NONE,
                row: RULER_ROW,
                at,
            },
        ) => Some(TimeAction::SongMarker(MarkerEdit::Add(*at))),
        (
            SONG,
            EventKind::Place {
                index,
                row: RULER_ROW,
                at,
            },
        ) => Some(TimeAction::SongMarker(MarkerEdit::Move {
            index: *index,
            at: *at,
        })),
        // With a section picked, the last four buttons are + Bar, Rename
        // (the app opens the text keyboard: `song_rename`), Remove marker
        // and Delete bars.
        (SONG, EventKind::Tab(k @ (2 | 4 | 5))) if section_buttons(st) => {
            let s = st.section?;
            Some(match k {
                2 => TimeAction::SongBars(BarsEdit::RoomBefore(s)),
                4 => TimeAction::SongMarker(MarkerEdit::Delete(s)),
                _ => TimeAction::SongBars(BarsEdit::DeleteSection(s)),
            })
        }
        (SONG, EventKind::Tab(3)) if section_buttons(st) => None,
        // With a point picked, the last two buttons are Ramp and Delete.
        (SONG, EventKind::Tab(k @ (4 | 5))) if st.region.is_none() && st.tempo_pick.is_some() => {
            let index = st.tempo_pick?;
            let (_, _, ramp) = *st.tempo_points.get(index)?;
            Some(TimeAction::SongTempo(if *k == 4 {
                TempoEdit::Ramp { index, on: !ramp }
            } else {
                TempoEdit::Delete(index)
            }))
        }
        (SONG, EventKind::Tab(k)) if *k < AUDIO_BUTTONS.len() && st.region.is_some() => {
            let r = st.region?;
            Some(TimeAction::Audio(match k {
                0 => AudioAction::Split(r),
                1 => AudioAction::Erase(r),
                2 => AudioAction::Fades(r),
                3 => AudioAction::Gain(r),
                4 => AudioAction::Takes(r.track),
                _ => AudioAction::ToCell(r),
            }))
        }
        (SONG, EventKind::Tab(k)) if (SECTION_TAP..BARS_BACK).contains(k) => {
            let k = k - SECTION_TAP;
            Some(TimeAction::PickSection(
                (st.section != Some(k)).then_some(k),
            ))
        }
        // One of several picked blocks dragged: they all go with it
        // (6.5).
        (SONG, EventKind::Place { index, row, at })
            if *row < TEMPO_ROW && st.picks.contains(index) =>
        {
            let p = st.placements.get(*index)?;
            Some(TimeAction::MovePlacements {
                set: st.picks.clone(),
                lanes: i32::try_from(*row).ok()? - i32::try_from(p.track).ok()?,
                by: *at - p.start,
                copy: false,
            })
        }
        (SONG, EventKind::Place { index, row, at }) => Some(TimeAction::MovePlacement {
            index: *index,
            track: *row,
            start: *at,
            copy: false,
        }),
        // A box dragged across the lanes (6.5): the blocks it touches.
        (SONG, EventKind::Span { index, from, to }) if *index >= crate::song::BOX => {
            let (top, bottom) = crate::song::box_tracks(*index);
            let set = st
                .placements
                .iter()
                .enumerate()
                .filter(|(_, p)| {
                    (top..=bottom).contains(&p.track) && p.start < *to && p.start + p.length > *from
                })
                .map(|(i, _)| i)
                .collect();
            Some(TimeAction::PickPlacements { set, add: false })
        }
        (SONG, EventKind::Stretch { index, to }) => Some(TimeAction::StretchPlacement {
            index: *index,
            length: *to,
        }),
        (SONG, EventKind::Drop { from, to }) => Some(TimeAction::MoveSection {
            marker: *from,
            gap: *to,
            copy: false,
        }),
        (SONG, EventKind::Tab(crate::song::ADD_TRACK)) => Some(TimeAction::AddTrack),
        // The "+ Track" chooser's rows (layout spec 4c); a touch off it
        // is the page's own (TimePage::chooser).
        (SONG, EventKind::Tab(crate::song::ADD_NOTE)) => Some(TimeAction::AddTrack),
        (SONG, EventKind::Tab(ADD_AUDIO)) => Some(TimeAction::AddAudioTrack),
        (SONG, EventKind::Tab(crate::song::ADD_LOOP)) => Some(TimeAction::AddLoopTrack),
        (SONG, EventKind::Tab(crate::song::ADD_CLOSE)) => None,
        (SONG, EventKind::Tab(crate::song::LIST)) => Some(TimeAction::ListView(true)),
        (SONG, EventKind::Tab(crate::song::CLIPS)) => Some(TimeAction::SongView(false)),
        (SONG, EventKind::Tab(DUPLICATE)) if !st.picks.is_empty() => {
            Some(TimeAction::DuplicatePlacements(st.picks.clone()))
        }
        (SONG, EventKind::Tab(DUPLICATE)) => st.selected.map(TimeAction::DuplicatePlacement),
        (SONG, EventKind::Tab(k)) if *k >= PANEL_CLOSE => {
            let track = st.audio_panel?;
            let lane = st.audio.iter().find(|l| l.track == track);
            let (mute, solo) = lane.map_or((false, false), |l| (l.mute, l.solo));
            Some(TimeAction::Audio(match *k {
                PANEL_CLOSE => AudioAction::ClosePanel,
                PANEL_MUTE => AudioAction::Mute { track, on: !mute },
                PANEL_SOLO => AudioAction::Solo { track, on: !solo },
                PANEL_TAKES => AudioAction::Takes(track),
                crate::song::PANEL_STRIP => AudioAction::Strip(track),
                k if (PANEL_INPUT..PANEL_INPUT + INPUTS.len()).contains(&k) => AudioAction::Input {
                    track,
                    input: INPUTS[k - PANEL_INPUT].1,
                },
                k if (PANEL_MONITOR..PANEL_MONITOR + MONITORS.len()).contains(&k) => {
                    AudioAction::Monitor {
                        track,
                        monitor: MONITORS[k - PANEL_MONITOR].1,
                    }
                }
                crate::song::PANEL_MIX => AudioAction::Mix { track, step: 0 },
                crate::song::PANEL_FROM_LESS => AudioAction::Mix { track, step: -1 },
                crate::song::PANEL_FROM_MORE => AudioAction::Mix { track, step: 1 },
                crate::song::PANEL_BEFORE => AudioAction::MixEffects {
                    track,
                    after: false,
                },
                crate::song::PANEL_AFTER => AudioAction::MixEffects { track, after: true },
                _ => return None,
            }))
        }
        (SONG, EventKind::Marker { index, frame }) => Some(TimeAction::Audio(AudioAction::Move {
            region: RegionId {
                track: index / REGION_SLOTS,
                index: index % REGION_SLOTS,
            },
            to: Tick::try_from(*frame).ok()?,
            fine: false,
        })),
        (SONG, EventKind::Span { index, from, to }) => {
            let take = u16::try_from(*index).ok()?;
            Some(TimeAction::Audio(if from == to {
                AudioAction::UseTake(take)
            } else {
                AudioAction::Comp {
                    take,
                    from: *from,
                    to: *to,
                    fine: false,
                }
            }))
        }
        (SONG, EventKind::Step { index, delta }) => {
            let track = st.audio_panel?;
            Some(TimeAction::Audio(AudioAction::Turn {
                track,
                region: st.region.filter(|r| r.track == track).map(|r| r.index),
                knob: *index,
                ticks: *delta,
                fine: false,
            }))
        }
        (SONG, EventKind::Toggle(track)) => Some(TimeAction::Audio(AudioAction::Arm {
            track: *track,
            on: !st.audio.iter().any(|l| l.track == *track && l.armed),
        })),
        // The input chip, with Dev's "Loops on audio tracks" on: the
        // track's input list (phase 4, board 4).
        (
            SONG,
            EventKind::Cell {
                row,
                col: INPUT_CHIP,
            },
        ) if st.loops_on => Some(TimeAction::Audio(AudioAction::Inputs(*row))),
        (
            SONG,
            EventKind::Cell {
                row,
                col: PANEL | INPUT_CHIP,
            },
        ) => Some(TimeAction::Audio(AudioAction::Panel(*row))),
        (SONG, EventKind::Cell { row, col }) => {
            Some(TimeAction::Audio(AudioAction::Pick(Some(RegionId {
                track: *row,
                index: *col,
            }))))
        }
        (SONG_LIST, EventKind::Toggle(k)) => match *k {
            LIST_LOOP => Some(TimeAction::ListLoop(!st.list_looped)),
            LIST_CLIPS => Some(TimeAction::SongView(false)),
            LIST_SONG => Some(TimeAction::SongView(true)),
            LIST_ADD => {
                (st.list.len() < mpfree_seq::scenes::MAX_LIST_STEPS).then_some(TimeAction::ListAdd)
            }
            // Linked to the song, Back to the scene takes its place.
            LIST_TO_SONG if st.list_linked => st
                .list_pick
                .filter(|&k| st.list.get(k).is_some_and(|r| r.changed))
                .map(TimeAction::ListBack),
            LIST_TO_SONG => (!st.list.is_empty()).then_some(TimeAction::ListToSong),
            LIST_PLAY => st.list_pick.map(TimeAction::ListPlay),
            LIST_REMOVE => st.list_pick.map(TimeAction::ListRemove),
            LIST_LINK_FROM_LIST if !st.list_linked => Some(TimeAction::ListLink(LinkFrom::List)),
            LIST_LINK_FROM_SONG if !st.list_linked => Some(TimeAction::ListLink(LinkFrom::Song)),
            _ => None,
        },
        // A linked list's Song row has no scene and plays once (phase 4b).
        (SONG_LIST, EventKind::Step { index, .. }) if list_lead(st, *index) => None,
        (SONG_LIST, EventKind::Step { index, delta }) if *index >= SCENE_STEP => {
            let step = index - SCENE_STEP;
            (step < st.list.len()).then_some(TimeAction::ListScene {
                step,
                delta: *delta,
            })
        }
        (SONG_LIST, EventKind::Step { index, delta }) => {
            let r = st.list.get(*index)?;
            let to = i32::from(r.times) + delta;
            (1..=mpfree_seq::scenes::MAX_TIMES as i32)
                .contains(&to)
                .then_some(TimeAction::ListTimes {
                    step: *index,
                    delta: *delta,
                })
        }
        (SONG_LIST, EventKind::Select(k)) => {
            (*k < st.list.len()).then_some(TimeAction::ListPick(Some(*k)))
        }
        (SONG_LIST, EventKind::Drop { from, to }) => {
            (*from < st.list.len() && *to <= st.list.len()).then_some(TimeAction::ListMove {
                step: *from,
                to: *to,
            })
        }
        // Several blocks picked (6.5): Split and Delete take them all;
        // Make unique takes one block at a time (`why_not`).
        (SONG, EventKind::Tab(0)) if !st.picks.is_empty() => {
            Some(TimeAction::SplitPlacements(st.picks.clone()))
        }
        (SONG, EventKind::Tab(4)) if !st.picks.is_empty() => None,
        (SONG, EventKind::Tab(5)) if !st.picks.is_empty() => {
            Some(TimeAction::DeletePlacements(st.picks.clone()))
        }
        (SONG, EventKind::Tab(k)) => match k {
            // Split at the playhead (phase 6): the picked block, or every
            // block across it. With an audio region picked, the region's
            // Split (above).
            0 => Some(TimeAction::SplitPlacement(st.selected)),
            1 => Some(TimeAction::SongMode(!st.song_on)),
            2 => Some(TimeAction::ScenesToSong),
            3 => Some(TimeAction::SongToScenes),
            4 => st.selected.map(TimeAction::MakeUnique),
            5 => st.selected.map(TimeAction::DeletePlacement),
            &REC => Some(TimeAction::RecSong(!st.song_rec)),
            _ => None,
        },
        (SONG, EventKind::Select(i)) => {
            Some(TimeAction::SelectPlacement((*i != NONE).then_some(*i)))
        }
        // The sound line (`SOUND_TAP + track`) is the app's: the
        // track's instrument.
        (CLIPS, EventKind::Tab(track)) if *track >= crate::widgets::SOUND_TAP => None,
        (CLIPS, EventKind::Tab(track)) => Some(name_tap(st, *track)),
        (CLIPS, EventKind::Select(i)) => cell_tap(st, i / ROWS, i % ROWS),
        (RECORD, EventKind::Step { index, delta }) => {
            let d = (*delta).signum();
            match *index {
                0 => {
                    // None, then each track in turn.
                    let n = st.tracks.len() as i64 + 1;
                    let now = st.armed.map_or(0, |a| a as i64 + 1);
                    let next = (now + i64::from(d)).rem_euclid(n);
                    Some(TimeAction::Arm(if next == 0 {
                        None
                    } else {
                        Some(next as usize - 1)
                    }))
                }
                1 => Some(TimeAction::Transport(Command::ToggleOverdub)),
                2 => {
                    let bars = (i32::from(t.count_in) + d).clamp(0, i32::from(MAX_COUNT_IN)) as u8;
                    (bars != t.count_in).then_some(TimeAction::Transport(Command::SetCountIn(bars)))
                }
                3 => {
                    let bar = t.meter.bar_ticks();
                    let now = match t.loop_range {
                        None => 0,
                        Some((a, b)) => LOOP_BARS
                            .iter()
                            .position(|&n| a == 0 && b == n * bar)
                            // A loop set elsewhere steps from the nearest length.
                            .unwrap_or_else(|| {
                                LOOP_BARS
                                    .iter()
                                    .rposition(|&n| n * bar <= b - a)
                                    .unwrap_or(1)
                            }),
                    };
                    let next = (now as i32 + d).clamp(0, LOOP_BARS.len() as i32 - 1) as usize;
                    let range = (next > 0).then(|| (0, LOOP_BARS[next] * bar));
                    (range != t.loop_range)
                        .then_some(TimeAction::Transport(Command::SetLoop(range)))
                }
                // While the song's tempo points play, from the point in
                // effect at the playhead (a ramp's start, not where it
                // has glided to); the app gives it to that point.
                4 => {
                    let now = st.tempo_here.unwrap_or(t.bpm);
                    let bpm = (now + f64::from(d)).clamp(MIN_BPM, MAX_BPM);
                    (bpm != now).then_some(TimeAction::Transport(Command::SetTempo(bpm)))
                }
                // Either way flips it, for the armed track.
                5 => st.armed.and_then(|a| {
                    let on = st.tracks.get(a)?.pressure;
                    Some(TimeAction::RecordPressure { track: a, on: !on })
                }),
                // Off, then the grids from the coarsest.
                6 => st.armed.and_then(|a| {
                    let now = st.tracks.get(a)?.record_quantise;
                    let k = now.map_or(0, |g| {
                        QUANTISE_GRIDS
                            .iter()
                            .position(|&q| q == g)
                            .map_or(0, |i| i + 1)
                    });
                    let next = (k as i32 + d).clamp(0, QUANTISE_GRIDS.len() as i32) as usize;
                    let grid = (next > 0).then(|| QUANTISE_GRIDS[next - 1]);
                    (grid != now).then_some(TimeAction::RecordQuantise { track: a, grid })
                }),
                // At once, the note grids, then the bars.
                7 => {
                    let now = LAUNCH_GRIDS
                        .iter()
                        .position(|&g| g == st.launch)
                        .unwrap_or(5);
                    let next = (now as i32 + d).clamp(0, LAUNCH_GRIDS.len() as i32 - 1) as usize;
                    (next != now).then_some(TimeAction::LaunchGrid(LAUNCH_GRIDS[next]))
                }
                TRANSPOSE_ROW | FOLLOWS_ROW => transpose_step(st, *index, d).ok(),
                _ => None,
            }
        }
        (CLIP_CARD, EventKind::Step { index, delta }) => card_step(st, *index, *delta).ok(),
        (CLIP_CARD, EventKind::Toggle(b)) => {
            let (track, slot) = st.focus()?;
            if let Some(c) = st.converting() {
                return match b {
                    0 => Some(TimeAction::ConvertCancel),
                    1 if c.ok => Some(TimeAction::Convert { track, slot }),
                    _ => None,
                };
            }
            if st.clip(track, slot)?.audio.is_some() {
                return (*b == 0 && fit_refused(st, track, slot).is_none())
                    .then_some(TimeAction::FitClip { track, slot });
            }
            match b {
                0 => Some(TimeAction::SaveState { track, slot }),
                1 => st
                    .saved
                    .contains(&(track, slot))
                    .then_some(TimeAction::Reload { track, slot }),
                _ => None,
            }
        }
        // While the note editor's Continue is open the takes area holds
        // its tries: ‹ and › step them, each only while there is one.
        (TAKES, EventKind::Tab(k @ (0 | 1))) if continuing_open(st) => {
            let (i, n) = st.continuing.as_ref()?.position?;
            let back = *k == 0;
            (if back { i > 1 } else { i < n })
                .then_some(TimeAction::Continue(ContinueAction::Step { back }))
        }
        (TAKES, EventKind::Tab(_)) if continuing_open(st) => None,
        // Each only while it has something to do (it is faint otherwise).
        (TAKES, EventKind::Tab(0)) => (!st.done.is_empty()).then_some(TimeAction::Undo),
        (TAKES, EventKind::Tab(1)) => (!st.undone.is_empty()).then_some(TimeAction::Redo),
        (TAKES, EventKind::Tab(2)) => st.capture.is_some().then_some(TimeAction::Capture),
        _ => None,
    }
}

/// Why a touch that [`action`] answers with nothing changes nothing, for
/// the hint line: a faint button, or a setting already at its end (Jay, 13
/// Nov: say in place when a press would send nothing).
pub fn why_not(ev: &UiEvent, st: &TimeState) -> Option<String> {
    match record_event(ev, st) {
        Some(UiEvent {
            kind: EventKind::Step {
                index: MAKES_ROW, ..
            },
            ..
        }) => Some(MAKES_WHY.to_string()),
        Some(ev) => why_not_of(&ev, st),
        None => why_not_of(ev, st),
    }
}

/// [`why_not`], with a Record step at its place in [`RECORD_ROWS`].
fn why_not_of(ev: &UiEvent, st: &TimeState) -> Option<String> {
    let end = |delta: i32, low: &str, high: &str| {
        if delta < 0 {
            low.to_string()
        } else {
            high.to_string()
        }
    };
    match (ev.widget.as_str(), &ev.kind) {
        (CLIPS, EventKind::Select(i)) if st.tracks.get(i / ROWS).is_some_and(|t| t.audio) => {
            audio_cell_hint(st, i / ROWS, i % ROWS)
        }
        (MORPH, EventKind::Tab(0 | 1)) if !st.morph.on => {
            Some("Move the crossfader first: Snap back and Keep work on a morph".to_string())
        }
        (CLIP_CARD, EventKind::Toggle(0))
            if st
                .focus()
                .and_then(|(t, s)| st.clip(t, s))
                .is_some_and(|c| c.audio.is_some()) =>
        {
            let (track, slot) = st.focus()?;
            fit_refused(st, track, slot)
        }
        (MORPH, EventKind::Tab(3)) | (CLIP_CARD, EventKind::Toggle(1)) => Some(match st.focus() {
            None => "Arm a track with a clip: Reload clip reloads its clip".to_string(),
            Some(_) => {
                "Save state on the Clip card first: Reload clip brings that state back".to_string()
            }
        }),
        (SONG_LIST, EventKind::Toggle(LIST_PLAY | LIST_REMOVE)) if st.list_pick.is_none() => {
            Some("Tap a step first".to_string())
        }
        (SONG_LIST, EventKind::Toggle(LIST_TO_SONG)) if st.list_linked => {
            Some(match st.list_pick.and_then(|k| st.list.get(k)) {
                None => "Tap a part first".to_string(),
                Some(r) if r.lead => "Song has no scene".to_string(),
                Some(r) if r.scene.is_none() => format!("{} has no scene", r.name),
                Some(r) => format!("{} follows its scene already", r.name),
            })
        }
        (SONG_LIST, EventKind::Toggle(LIST_TO_SONG)) => {
            Some("Add a scene to the list first".to_string())
        }
        (SONG_LIST, EventKind::Toggle(LIST_ADD)) => Some(format!(
            "The list is full: {} steps",
            mpfree_seq::scenes::MAX_LIST_STEPS
        )),
        (SONG_LIST, EventKind::Step { index, .. }) if list_lead(st, *index) => {
            Some(SONG_ROW_WHY.to_string())
        }
        (SONG_LIST, EventKind::Step { index, delta }) if *index < SCENE_STEP => Some(end(
            *delta,
            "Once is the fewest",
            &format!("{} times is the most", mpfree_seq::scenes::MAX_TIMES),
        )),
        (TAKES, EventKind::Tab(0)) if continuing_open(st) => {
            Some("This is the first try".to_string())
        }
        (TAKES, EventKind::Tab(1)) if continuing_open(st) => {
            Some("This is the last try: Try again makes another".to_string())
        }
        (TAKES, EventKind::Tab(0)) => Some("Nothing to undo yet".to_string()),
        (TAKES, EventKind::Tab(1)) => Some("Nothing to redo: Undo something first".to_string()),
        (TAKES, EventKind::Tab(2)) => {
            Some("Nothing to capture yet: play the pads while not recording".to_string())
        }
        (SONG, EventKind::Tab(4)) if !st.picks.is_empty() => {
            Some("Make unique takes one block at a time: tap the one to make its own".to_string())
        }
        (SONG, EventKind::Tab(4 | 5)) if st.selected.is_none() && st.picks.is_empty() => {
            Some("Tap a block in the song first".to_string())
        }
        (RECORD, EventKind::Step { index: 2, delta }) => Some(end(
            *delta,
            "Count-in is off already",
            &format!("Count-in is at its longest, {MAX_COUNT_IN} bars"),
        )),
        (RECORD, EventKind::Step { index: 3, delta }) => Some(end(
            *delta,
            "The loop is off already",
            &format!(
                "The loop is at its longest, {} bars",
                LOOP_BARS[LOOP_BARS.len() - 1]
            ),
        )),
        (RECORD, EventKind::Step { index: 4, delta }) => Some(end(
            *delta,
            &format!("The tempo is at its lowest, {MIN_BPM:.0} BPM"),
            &format!("The tempo is at its highest, {MAX_BPM:.0} BPM"),
        )),
        (RECORD, EventKind::Step { index: 5, .. }) if st.armed.is_none() => {
            Some("Arm a track first: pressure records on the armed track".to_string())
        }
        (RECORD, EventKind::Step { index: 6, .. }) if st.armed.is_none() => {
            Some("Arm a track first: quantise is the armed track's".to_string())
        }
        (RECORD, EventKind::Step { index: 6, delta }) => Some(end(
            *delta,
            "Quantise is off already: the notes go in as played",
            &format!(
                "Quantise is at its finest, {}",
                grid_name(QUANTISE_GRIDS[QUANTISE_GRIDS.len() - 1])
            ),
        )),
        (RECORD, EventKind::Step { index: 7, delta }) => Some(end(
            *delta,
            "Launch is at once already: a tapped clip starts on the next tick",
            "Launch is at its longest, 8 bars",
        )),
        (RECORD, EventKind::Step { index, delta })
            if *index == TRANSPOSE_ROW || *index == FOLLOWS_ROW =>
        {
            transpose_step(st, *index, *delta).err()
        }
        (CLIP_CARD, EventKind::Step { index, delta }) => card_step(st, *index, *delta).err(),
        (CLIPS | SONG_LIST, EventKind::Toggle(k)) if st.scene_open.is_some() => {
            scene_card_action(st, *k)
                .err()
                .filter(|why| !why.is_empty())
        }
        (CLIPS | SONG_LIST, EventKind::Step { index, delta }) if st.scene_open.is_some() => {
            scene_step(st, *index, *delta).err()
        }
        _ => None,
    }
}

/// A note value for a grid of `ticks`: "1/16", "1/8T", or the ticks.
pub fn grid_name(ticks: u32) -> String {
    let whole = PPQ * 4;
    if ticks > 0 && whole.is_multiple_of(ticks) {
        let n = whole / ticks;
        if n.is_multiple_of(3) && (n / 3).is_power_of_two() {
            return format!("1/{}T", n * 2 / 3);
        }
        return format!("1/{n}");
    }
    format!("{ticks} ticks")
}

/// A speed as the card shows it: "x1", "x1/2", "x3/2".
pub fn speed_name(s: Speed) -> String {
    if s.den == 1 {
        format!("×{}", s.num)
    } else {
        format!("×{}/{}", s.num, s.den)
    }
}

fn follow_name(a: FollowAction) -> &'static str {
    match a {
        FollowAction::Stay => "Stay",
        FollowAction::Again => "Again",
        FollowAction::Next => "Next",
        FollowAction::Previous => "Previous",
        FollowAction::First => "First",
        FollowAction::Last => "Last",
        FollowAction::Any => "Any",
        FollowAction::Other => "Other",
        FollowAction::Stop => "Stop",
    }
}

/// The Record panel's tabs: Record, Clip with the focused clip's name
/// ("Clip · Verse B"; its track is the armed one, lit on the grid), and
/// Chords (the row's chords, composing tools S2.6). A scene's settings
/// are on its scene card ([`scene_card_of`]).
pub fn panel_tabs(st: &TimeState) -> Vec<String> {
    let clip = match st.focus().and_then(|(t, s)| st.clip(t, s)) {
        Some(c) => format!("Clip · {}", c.name),
        None => "Clip".to_string(),
    };
    vec!["Record".to_string(), clip, "Chords".to_string()]
}

/// The Record panel's tab widths but the last's (Chords): Record, and
/// Clip with its name (cut with "…" when it is long).
pub const PANEL_TAB_WIDTHS: [i32; 2] = [66, 90];

/// The scene editor's rows for the scene in focus.
pub fn scene_rows(st: &TimeState) -> Vec<OptionRow> {
    let row = |k: usize, value: String| OptionRow {
        name: SCENE_ROWS[k].to_string(),
        value,
        ..OptionRow::default()
    };
    let s = &st.scene;
    let dash = || "—".to_string();
    let r = st.scene_focus;
    vec![
        row(
            0,
            if s.name.is_empty() {
                format!("{}", r + 1)
            } else {
                format!("{} · {}", r + 1, s.name)
            },
        ),
        row(
            1,
            s.follow
                .map_or_else(|| "off".to_string(), |f| follow_name(f.action).to_string()),
        ),
        row(
            2,
            s.follow
                .map_or_else(dash, |f| bars_label(i64::from(f.bars))),
        ),
        row(3, s.follow.map_or_else(dash, |f| format!("{} %", f.chance))),
        row(
            4,
            s.follow
                .filter(|f| f.chance < 100)
                .map_or_else(dash, |f| follow_name(f.other).to_string()),
        ),
        row(5, mutes_words(st, s.mutes)),
        // The armed track's program change (as the Record tab's rows are
        // the armed track's), shown 1 to 128 as on the gear, and its bank
        // select.
        row(
            SCENE_PROGRAM,
            st.armed.map_or_else(dash, |t| {
                s.program(t)
                    .map_or_else(|| "off".to_string(), |p| program_name(p.program))
            }),
        ),
        row(
            SCENE_BANK,
            match st.armed.and_then(|t| s.program(t)) {
                None => dash(),
                Some(ClipProgram { bank: None, .. }) => "off".to_string(),
                Some(ClipProgram { bank: Some(b), .. }) => bank_name(b),
            },
        ),
        row(8, programs_words(st, s)),
        row(
            SCENE_TEMPO,
            s.tempo.map_or_else(
                || "as it is".to_string(),
                |t| format!("{}.{} BPM", t / 10, t % 10),
            ),
        ),
    ]
}

/// A tap on the scene card's Tempo value: the scene sets the tempo now
/// (to the nearest 0.1 BPM), or one with a tempo goes back to leaving it
/// as it is.
pub fn scene_tempo_now(st: &TimeState) -> Result<TimeAction, String> {
    let mut s = st.scene.clone();
    s.tempo = match s.tempo {
        Some(_) => None,
        None => Some(
            mpfree_seq::scenes::scene_tempo(st.transport.bpm)
                .ok_or("the tempo now is out of a scene's range, 30 to 300 BPM")?,
        ),
    };
    Ok(TimeAction::SceneSettings {
        row: st.scene_focus,
        card_row: SCENE_TEMPO as u8,
        settings: Box::new(s),
    })
}

/// What a tap on the scene card asks for, or why it asks for nothing
/// (empty: the page's own, or the app's, as Rename is).
pub fn scene_card_action(st: &TimeState, k: usize) -> Result<TimeAction, String> {
    use crate::scene_card as sc;
    let row = st.scene_open.unwrap_or(st.scene_focus);
    match k {
        sc::SCENE_DONE => Ok(TimeAction::SceneCard(None)),
        sc::SCENE_BEFORE => match row.checked_sub(1) {
            Some(r) => Ok(TimeAction::SceneCard(Some(r))),
            None => Err("Scene 1 is the first".into()),
        },
        sc::SCENE_AFTER if row + 1 < ROWS => Ok(TimeAction::SceneCard(Some(row + 1))),
        sc::SCENE_AFTER => Err(format!("Scene {ROWS} is the last")),
        sc::SCENE_CAPTURE => Ok(TimeAction::CaptureScene),
        sc::SCENE_COPY_ROW => Ok(TimeAction::DropRow {
            from: row,
            to: row + 1,
            copy: true,
        }),
        sc::SCENE_INSERT_ROW => Ok(TimeAction::InsertRow(row)),
        sc::SCENE_DELETE_ROW => Ok(TimeAction::DeleteRow(row)),
        sc::SCENE_TEMPO_NOW => scene_tempo_now(st),
        sc::SCENE_MUTES_NOW => scene_mutes_now(st),
        k if (sc::SCENE_KEEP..sc::SCENE_KEEP + MAX_TRACKS).contains(&k) => {
            scene_keep(st, k - sc::SCENE_KEEP)
        }
        _ => Err(String::new()),
    }
}

/// The scene card for the row open ([`TimeState::scene_open`]): the scene
/// in focus is its row.
pub fn scene_card_of(st: &TimeState) -> Option<crate::scene_card::SceneCard> {
    use crate::scene_card::{CardCell, CardTrack, SceneCard};
    let row = st.scene_open?;
    let s = &st.scene;
    let name = if s.name.is_empty() {
        format!("Row {}", row + 1)
    } else {
        s.name.clone()
    };
    let has_clip = st
        .tracks
        .iter()
        .any(|t| t.slots.get(row).is_some_and(Option::is_some));
    let bars = s.follow.map_or_else(|| scene_bars(st), |f| f.bars);
    let mut about = if has_clip {
        format!("{} {}", bars, if bars == 1 { "bar" } else { "bars" })
    } else {
        "no clips".to_string()
    };
    if st.scene_playing == Some(row) {
        about.push_str(" · playing");
    } else if st.scene_cued == Some(row) {
        about.push_str(" · waiting");
    }
    let tracks = st
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| !t.audio)
        .take(MAX_TRACKS)
        .map(|(i, t)| CardTrack {
            track: i,
            name: t.name.clone(),
            color: track_color(i),
            cell: match t.slots.get(row).and_then(Option::as_ref) {
                Some(c) => CardCell::Clip(c.name.clone()),
                None if s.keep & (1 << i) != 0 => CardCell::Keep,
                None => CardCell::Stop,
            },
        })
        .collect();
    let last_free = st
        .tracks
        .iter()
        .all(|t| t.slots.get(ROWS - 1).is_none_or(Option::is_none));
    let plays = capture_can(st);
    Some(SceneCard {
        row,
        rows_count: ROWS,
        name,
        about,
        rows: scene_rows(st),
        picked: 1,
        tracks,
        first: 0,
        can: [
            plays,
            last_free && has_clip,
            last_free && row + 1 < ROWS - 1,
            true,
        ],
    })
}

/// Whether Capture scene has something to keep (5.3): the song plays,
/// or a track has a clip launched. The scene card's Capture and
/// Perform's Capture scene are faint without.
pub fn capture_can(st: &TimeState) -> bool {
    st.song_on || (0..st.tracks.len()).any(|t| st.launch(t).is_some())
}

/// A tap on an empty cell's box on the scene card: the track stops as the
/// scene starts, or keeps playing (Keep playing, 5.2). Refused for a
/// track the row has a clip for.
pub fn scene_keep(st: &TimeState, track: usize) -> Result<TimeAction, String> {
    let name = st.track_name(track);
    let row = st.scene_focus;
    if st
        .tracks
        .get(track)
        .and_then(|t| t.slots.get(row))
        .is_some_and(Option::is_some)
    {
        return Err(format!(
            "{name} has a clip in row {}: it plays as the scene starts",
            row + 1
        ));
    }
    if track >= MAX_TRACKS {
        return Err(format!("{name}: no such track"));
    }
    let mut s = st.scene.clone();
    s.keep ^= 1 << track;
    Ok(TimeAction::SceneSettings {
        row,
        card_row: SCENE_KEEP_ROW + track as u8,
        settings: Box::new(s),
    })
}

/// A program change as the gear shows it: 1 to 128.
pub fn program_name(program: u8) -> String {
    (u16::from(program) + 1).to_string()
}

/// The tracks a scene sends a program change, as its Sent to row says
/// them: "none", "Keys", "Keys, Bass", "Keys +2".
fn programs_words(st: &TimeState, s: &SceneSettings) -> String {
    let name = |p: &TrackProgram| st.track_name(p.track);
    match s.programs.as_slice() {
        [] => "none".to_string(),
        [a] => name(a),
        [a, b] => format!("{}, {}", name(a), name(b)),
        [a, rest @ ..] => format!("{} +{}", name(a), rest.len()),
    }
}

/// A scene's mutes as its Mutes row says them: "as they are" (it leaves
/// them), "none", "Keys, Bass", "Keys +2".
fn mutes_words(st: &TimeState, mutes: Option<u128>) -> String {
    let Some(m) = mutes else {
        return "as they are".to_string();
    };
    let names: Vec<String> = (0..st.tracks.len().min(MAX_TRACKS))
        .filter(|&t| m & (1 << t) != 0)
        .map(|t| st.track_name(t))
        .collect();
    match names.len() {
        0 => "none".to_string(),
        1 | 2 => names.join(", "),
        n => format!("{} +{}", names[0], n - 1),
    }
}

/// The tracks muted now, as a scene's mutes.
fn mutes_now(st: &TimeState) -> u128 {
    st.tracks
        .iter()
        .take(MAX_TRACKS)
        .enumerate()
        .filter(|(_, t)| t.mute)
        .fold(0, |m, (k, _)| m | 1 << k)
}

/// Mutes now: the scene in focus mutes the tracks muted now, and plays
/// the others, when it starts.
fn scene_mutes_now(st: &TimeState) -> Result<TimeAction, String> {
    let m = mutes_now(st);
    if st.scene.mutes == Some(m) {
        return Err(format!(
            "Scene {} mutes these tracks already",
            st.scene_focus + 1
        ));
    }
    Ok(TimeAction::SceneSettings {
        row: st.scene_focus,
        card_row: 5,
        settings: Box::new(SceneSettings {
            mutes: Some(m),
            ..st.scene.clone()
        }),
    })
}

/// How long the scene in focus plays in whole bars: its longest clip
/// (one bar at least), for a follow turned on.
fn scene_bars(st: &TimeState) -> u16 {
    let bar = st.transport.meter.bar_ticks().max(1) as f64;
    let longest = st
        .tracks
        .iter()
        .filter_map(|t| t.slots.get(st.scene_focus)?.as_ref())
        .map(|c| c.song_len())
        .fold(0.0, f64::max);
    ((longest / bar).ceil() as u16).clamp(1, MAX_FOLLOW_BARS)
}

/// What a step of the scene editor's row `index` asks for, or why it
/// asks for nothing (a setting at its end, or off).
fn scene_step(st: &TimeState, index: usize, delta: i32) -> Result<TimeAction, String> {
    let d = delta.signum();
    let step = |now: usize, n: usize| {
        let next = now as i64 + i64::from(d);
        (0..n as i64).contains(&next).then_some(next as usize)
    };
    let end = |low: String, high: String| Err(if d < 0 { low } else { high });
    let row = st.scene_focus;
    let mut s = st.scene.clone();
    let set = |s: SceneSettings| {
        Ok(TimeAction::SceneSettings {
            row,
            card_row: index as u8,
            settings: Box::new(s),
        })
    };
    let off = || {
        format!(
            "Turn Follow on first: scene {} plays until another starts",
            row + 1
        )
    };
    match index {
        0 => match step(row, ROWS) {
            Some(k) => Ok(TimeAction::SceneFocus(k)),
            None => end(
                "Scene 1 is the first".into(),
                format!("Scene {ROWS} is the last"),
            ),
        },
        1 => {
            let now = s.follow.map_or(0, |f| {
                FOLLOWS
                    .iter()
                    .position(|&a| a == f.action)
                    .map_or(0, |k| k + 1)
            });
            match step(now, FOLLOWS.len() + 1) {
                Some(0) => {
                    s.follow = None;
                    set(s)
                }
                Some(k) => {
                    let action = FOLLOWS[k - 1];
                    s.follow = Some(match s.follow {
                        Some(f) => Follow { action, ..f },
                        // After one pass of the scene, to begin with.
                        None => Follow::after(scene_bars(st), action),
                    });
                    set(s)
                }
                None => end(
                    "Follow is off already".into(),
                    "Follow is at its last action, Stop".into(),
                ),
            }
        }
        2 => {
            let f = s.follow.ok_or_else(off)?;
            match step(usize::from(f.bars) - 1, usize::from(MAX_FOLLOW_BARS)) {
                Some(k) => {
                    s.follow = Some(Follow {
                        bars: k as u16 + 1,
                        ..f
                    });
                    set(s)
                }
                None => end(
                    "Follow is at its soonest, after 1 bar".into(),
                    format!("Follow is at its latest, after {MAX_FOLLOW_BARS} bars"),
                ),
            }
        }
        3 => {
            let f = s.follow.ok_or_else(off)?;
            let next = if d > 0 {
                (f.chance / 10 + 1) * 10
            } else {
                f.chance.div_ceil(10).saturating_sub(1) * 10
            };
            if next > 100 || (d < 0 && f.chance == 0) {
                return end(
                    "Chance is at its least, 0 %: the Or action always plays".into(),
                    "Chance is full already, 100 %".into(),
                );
            }
            s.follow = Some(Follow {
                chance: next.min(100),
                ..f
            });
            set(s)
        }
        4 => {
            let f = s.follow.ok_or_else(off)?;
            if f.chance >= 100 {
                return Err(
                    "Set Chance under 100 % first: then Or plays the rest of the time".into(),
                );
            }
            let now = OTHERS.iter().position(|&a| a == f.other).unwrap_or(0);
            match step(now, OTHERS.len()) {
                Some(k) => {
                    s.follow = Some(Follow {
                        other: OTHERS[k],
                        ..f
                    });
                    set(s)
                }
                None => end(
                    "Or is at its first, Stay".into(),
                    "Or is at its last action, Stop".into(),
                ),
            }
        }
        5 => match (s.mutes, d > 0) {
            (Some(_), false) => {
                s.mutes = None;
                set(s)
            }
            (None, true) => {
                s.mutes = Some(mutes_now(st));
                set(s)
            }
            (None, false) => {
                Err("Mutes is at its first: the scene leaves the mutes as they are".into())
            }
            (Some(_), true) => Err("Mutes now takes the tracks muted now".into()),
        },
        SCENE_PROGRAM | SCENE_BANK => {
            let t = st
                .armed
                .ok_or("Arm a track first: Program is the program change the scene sends it")?;
            let now = s.program(t);
            let to = if index == SCENE_BANK {
                let p = now.ok_or("Set Program first: the bank goes with its program change")?;
                Some(ClipProgram {
                    bank: bank_next(p, delta, 1)?,
                    ..p
                })
            } else {
                program_next(now, delta)?
            };
            s.set_program(t, to);
            set(s)
        }
        8 => Err(
            "Arm a track and set Program: this row lists the tracks the scene sends a program change"
                .into(),
        ),
        SCENE_TEMPO => scene_tempo_step(st, delta.signum() * 10),
        _ => Err(String::new()),
    }
}

/// A step of the scene's Tempo, `tenths` of a BPM: from "as it is" the
/// tempo now, then up or down within 30 to 300 BPM.
fn scene_tempo_step(st: &TimeState, tenths: i32) -> Result<TimeAction, String> {
    let Some(now) = st.scene.tempo else {
        return scene_tempo_now(st);
    };
    let range = mpfree_seq::scenes::SCENE_TEMPO;
    let next = i32::from(now) + tenths;
    if next < i32::from(*range.start()) {
        return Err("Tempo is at its slowest, 30 BPM: tap it to leave the tempo as it is".into());
    }
    if next > i32::from(*range.end()) {
        return Err("Tempo is at its fastest, 300 BPM".into());
    }
    let mut s = st.scene.clone();
    s.tempo = Some(next as u16);
    Ok(TimeAction::SceneSettings {
        row: st.scene_focus,
        card_row: SCENE_TEMPO as u8,
        settings: Box::new(s),
    })
}

/// The scene editor's step with SHIFT held or not: with SHIFT, Bank steps
/// a whole CC 0 (128), as on the clip card; the other rows step as
/// without it.
pub fn scene_action(
    st: &TimeState,
    index: usize,
    delta: i32,
    shift: bool,
) -> Result<TimeAction, String> {
    if shift && index == SCENE_TEMPO {
        // With SHIFT, a tenth of a BPM.
        return scene_tempo_step(st, delta.signum());
    }
    if !(shift && index == SCENE_BANK) {
        return scene_step(st, index, delta);
    }
    let t = st
        .armed
        .ok_or("Arm a track first: Program is the program change the scene sends it")?;
    let mut s = st.scene.clone();
    let p = s
        .program(t)
        .ok_or("Set Program first: the bank goes with its program change")?;
    s.set_program(
        t,
        Some(ClipProgram {
            bank: bank_next(p, delta, 128)?,
            ..p
        }),
    );
    Ok(TimeAction::SceneSettings {
        row: st.scene_focus,
        card_row: SCENE_BANK as u8,
        settings: Box::new(s),
    })
}

/// A step of a Program row: off, then 1 to 128 (the bank kept).
fn program_next(now: Option<ClipProgram>, delta: i32) -> Result<Option<ClipProgram>, String> {
    let k = now.map_or(0, |p| i32::from(p.program) + 1) + delta.signum();
    match k {
        ..0 => Err("Program is off already: no program change goes out".into()),
        0 => Ok(None),
        1..=128 => Ok(Some(ClipProgram {
            program: (k - 1) as u8,
            bank: now.and_then(|p| p.bank),
        })),
        _ => Err("Program is at its last, 128".into()),
    }
}

/// A step of a Bank row by `by` (1, or 128 with SHIFT: a whole CC 0):
/// off, then 0/0 up to 127/127.
fn bank_next(p: ClipProgram, delta: i32, by: i32) -> Result<Option<u16>, String> {
    // Off is one below 0/0.
    let now = p.bank.map_or(-1, i32::from);
    let max = i32::from(ClipProgram::MAX_BANK);
    let next = if delta > 0 {
        if now >= max {
            return Err(format!(
                "Bank is at its last, {}",
                bank_name(ClipProgram::MAX_BANK)
            ));
        }
        (now + by).min(max)
    } else {
        match now {
            ..0 => {
                return Err("Bank is off already: no bank select goes before the program".into())
            }
            // 0/0, then off.
            0 => -1,
            _ => (now - by).max(0),
        }
    };
    Ok(u16::try_from(next).ok())
}

/// The clip settings card's rows, for the focused clip; none without one.
pub fn clip_rows(st: &TimeState) -> Vec<OptionRow> {
    match st.focus().and_then(|(t, s)| st.clip(t, s)) {
        Some(c) if c.audio.is_some() => audio_rows(c, st.transport.meter),
        Some(c) => {
            let mut rows = card_rows(c.length, c.settings);
            rows.push(OptionRow {
                name: CLIP_ROWS[CLIP_MUTATE].to_string(),
                value: mutate_value(c),
                ..OptionRow::default()
            });
            rows.push(OptionRow {
                name: CLIP_ROWS[CLIP_MELODY].to_string(),
                value: melody_value(c),
                ..OptionRow::default()
            });
            rows.push(OptionRow {
                name: CLIP_ROWS[CLIP_CONTINUE].to_string(),
                value: continue_card_value(c),
                ..OptionRow::default()
            });
            rows.push(OptionRow {
                name: CLIP_ROWS[CLIP_PHRASE].to_string(),
                value: phrase_card_value(c),
                ..OptionRow::default()
            });
            rows.push(OptionRow {
                name: CLIP_ROWS[CLIP_STYLE].to_string(),
                value: style_card_value(c),
                ..OptionRow::default()
            });
            rows
        }
        None => Vec::new(),
    }
}

/// The clip card's Melody row's value (E1): "pads" (it follows the pads)
/// or "notes" (it uses notes); "—" on a loop's cell. Short, to fit the
/// row's value at 250 px.
pub fn melody_value(c: &ClipInfo) -> String {
    match (c.looped, c.holds_notes) {
        (Some(_), _) => "—",
        (None, true) => "notes",
        (None, false) => "pads",
    }
    .to_string()
}

/// The clip card's Mutate row's value (S6): "original" until the clip is
/// mutated, then "variant" (the hint line and the Takes list number it);
/// "—" on a clip with no notes or a loop's cell.
pub fn mutate_value(c: &ClipInfo) -> String {
    match c.variation {
        _ if c.looped.is_some() || (c.notes == 0 && c.variation.is_none()) => "—".to_string(),
        Some(_) => "variant".to_string(),
        None => "original".to_string(),
    }
}

/// The clip card's Continue row's value (phase 1 step 4): the bars it
/// adds when it opens ("+4 bars", the note editor's Bars as Continue
/// opens), or why it can't: "no tune" on a clip of pad hits, "no notes"
/// on one with none, "—" on a loop's cell.
pub fn continue_card_value(c: &ClipInfo) -> String {
    if c.looped.is_some() {
        "—".to_string()
    } else if c.holds == mpfree_seq::Holds::Hits {
        "no tune".to_string()
    } else if c.notes == 0 {
        "no notes".to_string()
    } else {
        crate::editor::compose::ContinueSet::default().value(crate::editor::compose::BARS_ROW, &[])
    }
}

/// The clip card's Phrase row's value (step 3 of the phrases): "Make ›",
/// which opens the Make a phrase sheet, or why it can't, as Continue's
/// row says it.
pub fn phrase_card_value(c: &ClipInfo) -> String {
    match continue_card_value(c) {
        why if c.looped.is_some() || c.holds == mpfree_seq::Holds::Hits || c.notes == 0 => why,
        _ => "Make ›".to_string(),
    }
}

/// The clip card's Style row's value (composing step 6): "Make ›", which
/// opens the Make a style sheet, "no notes" on a clip with none, "—" on a
/// loop's cell. A clip of pad hits makes a style too: its feel and accents
/// are in when its hits are played, whatever the pads play.
pub fn style_card_value(c: &ClipInfo) -> String {
    if c.looped.is_some() {
        "—".to_string()
    } else if c.notes == 0 {
        "no notes".to_string()
    } else {
        "Make ›".to_string()
    }
}

/// The clip card's buttons, under its rows (S4): Save state, and Reload
/// while the focused clip has a saved state, outlined while its reload
/// waits. None without a clip; Save state is off on a loop's cell (its
/// layers have the looper's undo).
pub fn card_buttons(st: &TimeState) -> Vec<OptionButton> {
    let Some((track, slot)) = st.focus() else {
        return Vec::new();
    };
    // Convert to notes' preview (E1): Cancel, and Convert when it can.
    if let Some(c) = st.converting() {
        let mut b = vec![OptionButton::new("Cancel", true)];
        if c.ok {
            b.push(OptionButton::new("Convert", true));
        }
        return b;
    }
    if let Some(b) = fit_button(st, track, slot) {
        return vec![b];
    }
    let looped = st.clip(track, slot).is_some_and(|c| c.looped.is_some());
    let mut reload = OptionButton::new("Reload", st.saved.contains(&(track, slot)));
    reload.waiting = st.reload_waiting((track, slot));
    vec![OptionButton::new("Save state", !looped), reload]
}

/// The card's rows for a clip `length` ticks long (its own ticks) with
/// `s`.
pub fn card_rows(length: Tick, s: ClipSettings) -> Vec<OptionRow> {
    let row = |k: usize, value: String| OptionRow {
        name: CLIP_ROWS[k].to_string(),
        value,
        ..OptionRow::default()
    };
    let steps = (length + CARD_STEP - 1) / CARD_STEP;
    let dash = "—".to_string();
    vec![
        row(
            0,
            match steps {
                1 => "1 step".to_string(),
                // Past 999 the word does not fit: whole bars as bars.
                ..=999 => format!("{steps} steps"),
                _ if steps % 16 == 0 => format!("{} bars", steps / 16),
                _ => format!("{steps} st"),
            },
        ),
        row(1, speed_name(s.speed)),
        row(
            2,
            s.swing.map_or_else(
                || "track".to_string(),
                |w| {
                    if w.grid == PPQ / 4 {
                        format!("{} %", w.amount)
                    } else {
                        format!("{}% {}", w.amount, grid_name(w.grid))
                    }
                },
            ),
        ),
        row(
            3,
            s.quantise
                .map_or_else(|| "off".to_string(), |q| grid_name(q.grid)),
        ),
        row(
            4,
            s.quantise
                .map_or_else(|| dash.clone(), |q| format!("{} %", q.strength)),
        ),
        row(
            5,
            // Its chance and other action (set by an import) are kept, not
            // shown: the box has no room.
            s.follow
                .map_or_else(|| "off".to_string(), |f| follow_name(f.action).to_string()),
        ),
        row(
            6,
            s.follow
                .map_or(dash.clone(), |f| bars_label(i64::from(f.bars))),
        ),
        // The project's launch quantise unless it has its own.
        row(
            7,
            s.launch.map_or_else(|| "project".to_string(), launch_name),
        ),
        row(8, MODES[mode_of(&s)].to_string()),
        // The program change it sends its track's outside gear as it
        // starts (S5): shown 1 to 128, as on the gear.
        row(
            9,
            s.program
                .map_or_else(|| "off".to_string(), |p| program_name(p.program)),
        ),
        // Its bank select as the two numbers sent, CC 0 / CC 32.
        row(
            10,
            match s.program {
                None => dash.clone(),
                Some(ClipProgram { bank: None, .. }) => "off".to_string(),
                Some(ClipProgram { bank: Some(b), .. }) => bank_name(b),
            },
        ),
        // The order it plays its steps in (S6); "—" on a clip it can't
        // move (not two whole 1/16 steps or more), which plays forward.
        row(
            CLIP_DIRECTION,
            match s.direction {
                d if d != Direction::Forward && direction_steps(d, length).is_none() => dash,
                d => direction_name(d).to_string(),
            },
        ),
    ]
}

/// A bank select as the gear's manual names it: CC 0's value / CC 32's.
pub fn bank_name(bank: u16) -> String {
    format!("{}/{}", bank >> 7, bank & 0x7F)
}

/// What a step of the card's row `index` asks for, with SHIFT held or
/// not, or why it asks for nothing (empty when there is nothing to say).
/// With SHIFT, Length doubles (the notes repeated) or halves, Speed
/// doubles or halves, and Bank steps a whole CC 0 (128); the other rows
/// step as without it.
pub fn card_action(
    st: &TimeState,
    index: usize,
    delta: i32,
    shift: bool,
) -> Result<TimeAction, String> {
    if let Some((track, slot, c)) = focused_audio(st) {
        return audio_step(st, track, slot, c, index, delta, shift);
    }
    if shift && index == 10 {
        return bank_step(st, delta, 128);
    }
    // SHIFT + › on Mutate: a bolder variation.
    if shift && index == CLIP_MUTATE && delta > 0 {
        return match card_step(st, index, delta)? {
            TimeAction::Mutate {
                track,
                slot,
                melodic,
                ..
            } => Ok(TimeAction::Mutate {
                track,
                slot,
                more: true,
                melodic,
            }),
            a => Ok(a),
        };
    }
    if !shift || index > 1 {
        return card_step(st, index, delta);
    }
    let (track, slot) = st
        .focus()
        .ok_or("No clip to set: arm a track with a clip, or make one")?;
    let c = st.clip(track, slot).ok_or("No clip to set")?;
    if c.looped.is_some() {
        return Err("A loop cell plays its loop as recorded: only Follow and After apply".into());
    }
    let up = delta > 0;
    if index == 0 {
        let max = (MAX_CLIP_TICKS / CARD_STEP) as usize;
        let now = ((c.length + CARD_STEP - 1) / CARD_STEP) as usize;
        let steps = if up { now * 2 } else { now / 2 };
        if steps == 0 || steps > max {
            return Err(if up {
                format!("The clip can't double: {max} steps at most")
            } else {
                "The clip can't halve: it is 1 step long".to_string()
            });
        }
        return Ok(TimeAction::ClipLength {
            track,
            slot,
            steps,
            copy: up,
        });
    }
    let sp = c.settings.speed;
    let (num, den) = if up {
        (u16::from(sp.num) * 2, u16::from(sp.den))
    } else {
        (u16::from(sp.num), u16::from(sp.den) * 2)
    };
    let g = gcd(num, den);
    let speed = u8::try_from(num / g)
        .ok()
        .zip(u8::try_from(den / g).ok())
        .and_then(|(n, d)| Speed::new(n, d))
        .ok_or_else(|| {
            let ratio = f64::from(num) / f64::from(den);
            let way = if up { "double" } else { "halve" };
            match ratio {
                r if r > 4.0 => format!("{} can't double: ×4 is the fastest", speed_name(sp)),
                r if r < 0.125 => format!("{} can't halve: ×1/8 is the slowest", speed_name(sp)),
                _ => format!(
                    "{} can't {way}: ×{}/{} is not a speed (the Speed row's arrows step it)",
                    speed_name(sp),
                    num / g,
                    den / g
                ),
            }
        })?;
    Ok(TimeAction::ClipSettings {
        track,
        slot,
        row: 1,
        settings: ClipSettings {
            speed,
            ..c.settings
        },
    })
}

fn gcd(a: u16, b: u16) -> u16 {
    if b == 0 {
        a.max(1)
    } else {
        gcd(b, a % b)
    }
}

/// What a step of the card's row `index` asks for, or why it asks for
/// nothing (no clip, a loop cell, a setting at its end): an audio clip's
/// card has rows of its own ([`AUDIO_CLIP_ROWS`]).
fn card_step(st: &TimeState, index: usize, delta: i32) -> Result<TimeAction, String> {
    if let Some((track, slot, c)) = focused_audio(st) {
        return audio_step(st, track, slot, c, index, delta, false);
    }
    settings_step(st, index, delta)
}

/// The focused clip, if it is an audio clip: its cell and facts.
pub fn focused_audio(st: &TimeState) -> Option<(usize, usize, &ClipInfo)> {
    let (track, slot) = st.focus()?;
    let c = st.clip(track, slot)?;
    c.audio.as_ref()?;
    Some((track, slot, c))
}

/// An audio clip's card rows (layout spec 4j): a loop's length in bars
/// and beats or a once clip's time, its mode, launch, follow action and
/// gain, and its name.
fn audio_rows(c: &ClipInfo, meter: Meter) -> Vec<OptionRow> {
    let Some(a) = &c.audio else {
        return Vec::new();
    };
    let notes = card_rows(c.length, c.settings);
    let shared = |k: usize| {
        notes
            .get(AUDIO_AS_NOTES[k])
            .map_or_else(String::new, |r| r.value.clone())
    };
    let length = if c.settings.once {
        clock_time(a.secs)
    } else {
        bars_beats(c.length, meter)
    };
    let values = [
        length,
        shared(1),
        shared(2),
        shared(3),
        shared(4),
        gain_words(a.gain),
        c.name.clone(),
    ];
    AUDIO_CLIP_ROWS
        .iter()
        .zip(values)
        .map(|(name, value)| OptionRow {
            name: name.to_string(),
            value,
            ..OptionRow::default()
        })
        .collect()
}

/// A loop's length as the audio card says it: "2 bars", "3 beats", "2
/// bars 1 beat"; a length off the beat in sixteenths ("5/16").
pub fn bars_beats(length: Tick, meter: Meter) -> String {
    let (bar, beat) = (meter.bar_ticks().max(1), meter.beat_ticks().max(1));
    let plural = |n: i64, one: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {one}s")
        }
    };
    if length % beat != 0 {
        return length_label(length as f64, meter);
    }
    match (length / bar, length % bar / beat) {
        (0, b) => plural(b, "beat"),
        (n, 0) => plural(n, "bar"),
        (n, b) => format!("{} {}", plural(n, "bar"), plural(b, "beat")),
    }
}

/// A gain as dB, the floor (-60 dB) and under as off.
fn gain_db(g: f32) -> f32 {
    if g <= 0.0 {
        AUDIO_GAIN_FLOOR_DB
    } else {
        (20.0 * g.log10()).max(AUDIO_GAIN_FLOOR_DB)
    }
}

/// An audio clip's gain as its card says it: "0.0 dB", "+1.5 dB", "-3.0
/// dB", "off".
pub fn gain_words(g: f32) -> String {
    let db = gain_db(g);
    if g <= 0.0 || db <= AUDIO_GAIN_FLOOR_DB {
        "off".to_string()
    } else if db.abs() < 0.05 {
        "0.0 dB".to_string()
    } else {
        format!("{db:+.1} dB")
    }
}

/// What a step of an audio clip's card row `index` asks for, or why it
/// asks for nothing. Length steps a beat (SHIFT a bar) and stays on the
/// beats; a once clip's length is its sound's. Mode, Launch, Follow and
/// After step as a note clip's. Gain steps 0.5 dB (SHIFT 0.1 dB), off
/// under -60 dB. Name is the app's (the naming keyboard).
#[allow(clippy::too_many_arguments)]
fn audio_step(
    st: &TimeState,
    track: usize,
    slot: usize,
    c: &ClipInfo,
    index: usize,
    delta: i32,
    shift: bool,
) -> Result<TimeAction, String> {
    let a = c.audio.as_ref().ok_or("No audio clip to set")?;
    let d = delta.signum();
    let meter = st.transport.meter;
    let set = |row: usize, set: AudioSet| {
        Ok(TimeAction::AudioClip {
            track,
            slot,
            row: row as u8,
            set,
        })
    };
    match index {
        0 if c.settings.once => Err(format!(
            "{} plays once, to the end of its sound: its length is its sound's, {}",
            c.name,
            clock_time(a.secs)
        )),
        0 => {
            let beat = meter.beat_ticks().max(1);
            let unit = if shift {
                meter.bar_ticks().max(1)
            } else {
                beat
            };
            let now = c.length;
            let max = MAX_CLIP_TICKS / beat * beat;
            let next = if d > 0 {
                (now / unit + 1) * unit
            } else {
                ((now + unit - 1) / unit - 1) * unit
            }
            .clamp(beat, max);
            if next == now || (d > 0 && next < now) || (d < 0 && next > now) {
                return Err(if d < 0 {
                    format!("{} is at its shortest, 1 beat", c.name)
                } else {
                    format!("{} is at its longest, {}", c.name, bars_beats(max, meter))
                });
            }
            set(0, AudioSet::Length(next))
        }
        1..=4 => settings_step(st, AUDIO_AS_NOTES[index], delta),
        AUDIO_GAIN => {
            let step = if shift { 0.1 } else { 0.5 };
            let db = gain_db(a.gain);
            if d < 0 && a.gain <= 0.0 {
                return Err(format!("{}'s gain is off already", c.name));
            }
            if d > 0 && db >= AUDIO_GAIN_MAX_DB - 0.01 {
                return Err(format!("{}'s gain is at its most, +12 dB", c.name));
            }
            let from = if a.gain <= 0.0 {
                AUDIO_GAIN_FLOOR_DB
            } else {
                db
            };
            let next = (((from / step).round() + d as f32) * step).min(AUDIO_GAIN_MAX_DB);
            let gain = if next <= AUDIO_GAIN_FLOOR_DB {
                0.0
            } else {
                10f32.powf(next / 20.0)
            };
            set(AUDIO_GAIN, AudioSet::Gain(gain))
        }
        // The app opens the naming keyboard.
        _ => Err(String::new()),
    }
}

/// The audio card's foot (layout spec 4j): Fit to tempo, or "Fitting…
/// 40 %" with its progress while it fits. Faint for a once clip and at
/// the project's tempo, a tap then saying why. None for a note clip.
fn fit_button(st: &TimeState, track: usize, slot: usize) -> Option<OptionButton> {
    st.clip(track, slot)?.audio.as_ref()?;
    let fitting = st
        .fitting
        .filter(|f| (f.0, f.1) == (track, slot))
        .map(|f| f.2);
    let label = match fitting {
        Some(p) => format!("Fitting… {} %", (p * 100.0).round() as u32),
        None => "Fit to tempo".to_string(),
    };
    // Drawn faint only when it can't fit at all; while it fits it is lit,
    // its fill the progress (a tap says how far it has got).
    let mut b = OptionButton::new(
        &label,
        fitting.is_some() || fit_refused(st, track, slot).is_none(),
    );
    b.explains = true;
    b.progress = fitting;
    Some(b)
}

/// Why Fit to tempo does nothing on the clip in cell `slot` of `track`:
/// a fit under way, a once clip, a clip at the project's tempo. None
/// when it can fit.
fn fit_refused(st: &TimeState, track: usize, slot: usize) -> Option<String> {
    let c = st.clip(track, slot)?;
    let a = c.audio.as_ref()?;
    let bpm = st.transport.bpm;
    if let Some(f) = st.fitting {
        return Some(if (f.0, f.1) == (track, slot) {
            format!(
                "{} is fitting to {} BPM: {} % done",
                c.name,
                tempo_short(bpm),
                (f.2 * 100.0).round() as u32
            )
        } else {
            "Another clip is fitting to tempo: one at a time".to_string()
        });
    }
    if c.settings.once {
        return Some(format!(
            "{} plays once, to the end of its sound: it sounds the same at any tempo, so there is nothing to fit",
            c.name
        ));
    }
    if !off_tempo(c, a, bpm) {
        return Some(format!(
            "{} was made at {} BPM, the project's tempo: there is nothing to fit",
            c.name,
            tempo_short(a.bpm)
        ));
    }
    None
}

/// The audio card's two lines over Fit to tempo (layout spec 4j): the
/// tempo the clip was made at and what that does here. None for a note
/// clip.
pub fn card_note(st: &TimeState) -> Option<OptionNote> {
    let (_, _, c) = focused_audio(st)?;
    let a = c.audio.as_ref()?;
    let bpm = st.transport.bpm;
    if c.settings.once {
        return Some(OptionNote {
            lead: "Plays to its end".to_string(),
            rest: "at any tempo".to_string(),
            warn: false,
        });
    }
    if !off_tempo(c, a, bpm) {
        return Some(OptionNote {
            lead: format!("Made at {} BPM,", tempo_short(a.bpm)),
            rest: "the project's tempo".to_string(),
            warn: false,
        });
    }
    let gap = pass_gap(c, a, bpm);
    let rest = if gap > 0.0 {
        format!("at {} a {gap:.1} s gap each pass", tempo_short(bpm))
    } else {
        format!("at {} {:.1} s cut each pass", tempo_short(bpm), -gap)
    };
    Some(OptionNote {
        lead: format!("Made at {} BPM:", tempo_short(a.bpm)),
        rest,
        warn: true,
    })
}

/// The silence a loop made at `a.bpm` leaves each pass at `bpm`, in
/// seconds; under 0 it cuts that much of its sound off.
fn pass_gap(c: &ClipInfo, a: &AudioCellInfo, bpm: f64) -> f64 {
    let quarters = c.length as f64 / f64::from(PPQ);
    quarters * 60.0 / bpm.max(1.0) - quarters * 60.0 / a.bpm.max(1.0)
}

/// The note clip card's rows: what a step of row `index` asks for, or
/// why it asks for nothing (no clip, a loop cell, a setting at its end).
fn settings_step(st: &TimeState, index: usize, delta: i32) -> Result<TimeAction, String> {
    let (track, slot) = st
        .focus()
        .ok_or("No clip to set: arm a track with a clip, or make one")?;
    let c = st.clip(track, slot).ok_or("No clip to set")?;
    // Continue: the note editor opens on the clip, Continue open (step
    // 4), or why it can't.
    if index == CLIP_CONTINUE {
        return if c.looped.is_some() {
            Err("A loop's cell has no notes to carry on".into())
        } else if c.holds == mpfree_seq::Holds::Hits {
            Err(format!(
                "Continue: {}",
                mpfree_compose::Refused::PadHits.words()
            ))
        } else if c.notes == 0 {
            Err("Continue: play or draw some notes to carry on first".into())
        } else {
            Ok(TimeAction::Continue(ContinueAction::Open { track, slot }))
        };
    }
    // Style: the Make a style sheet opens on the clip (composing step 6),
    // or why it can't.
    if index == CLIP_STYLE {
        return if c.looped.is_some() {
            Err("A loop's cell has no notes to make a style of".into())
        } else if c.notes == 0 {
            Err("Make a style: play or draw some notes first".into())
        } else {
            Ok(TimeAction::MakeStyle { track, slot })
        };
    }
    // Phrase: the Make a phrase sheet opens on the clip's notes (step 3
    // of the phrases), or why it can't.
    if index == CLIP_PHRASE {
        return if c.looped.is_some() {
            Err("A loop's cell has no notes to make a phrase of".into())
        } else if c.holds == mpfree_seq::Holds::Hits {
            Err("Make a phrase: a clip of pad hits has no notes of its own (a pad's note is its kit's)".into())
        } else if c.notes == 0 {
            Err("Make a phrase: play or draw some notes first".into())
        } else {
            Ok(TimeAction::MakePhrase { track, slot })
        };
    }
    if c.looped.is_some() && !(5..8).contains(&index) {
        return Err(
            "A loop cell plays its loop as recorded: only Follow, After and Launch apply".into(),
        );
    }
    let d = delta.signum();
    // Within 0..n from `now`, or None at the end it is pushed past.
    let step = |now: usize, n: usize| {
        let next = now as i64 + i64::from(d);
        (0..n as i64).contains(&next).then_some(next as usize)
    };
    let mut s = c.settings;
    let set = |s: ClipSettings| {
        Ok(TimeAction::ClipSettings {
            track,
            slot,
            row: index as u8,
            settings: s,
        })
    };
    let end = |low: String, high: String| Err(if d < 0 { low } else { high });
    match index {
        0 => {
            let max = (MAX_CLIP_TICKS / CARD_STEP) as usize;
            let now = ((c.length + CARD_STEP - 1) / CARD_STEP) as usize;
            match step(now - 1, max) {
                Some(k) => Ok(TimeAction::ClipLength {
                    track,
                    slot,
                    steps: k + 1,
                    copy: false,
                }),
                None => end(
                    "The clip is at its shortest, 1 step".into(),
                    format!("The clip is at its longest, {max} steps"),
                ),
            }
        }
        1 => {
            let ratio = |(n, d): (u8, u8)| f64::from(n) / f64::from(d);
            let at = ratio((s.speed.num, s.speed.den));
            // A speed set elsewhere steps from the nearest.
            let now = SPEEDS
                .iter()
                .position(|&(n, d)| (n, d) == (s.speed.num, s.speed.den))
                .unwrap_or_else(|| SPEEDS.iter().rposition(|&p| ratio(p) <= at).unwrap_or(0));
            match step(now, SPEEDS.len()) {
                Some(k) => {
                    s.speed = Speed::new(SPEEDS[k].0, SPEEDS[k].1).ok_or("speed")?;
                    set(s)
                }
                None => end(
                    "The clip is at its slowest, ×1/8".into(),
                    "The clip is at its fastest, ×4".into(),
                ),
            }
        }
        2 => {
            let lo = *CLIP_SWING.start();
            let n = usize::from(CLIP_SWING.end() - lo) + 2;
            let now = s
                .swing
                .map_or(0, |w| usize::from(w.amount.saturating_sub(lo)) + 1);
            match step(now, n) {
                Some(0) => {
                    s.swing = None;
                    set(s)
                }
                Some(k) => {
                    let grid = s.swing.map_or(PPQ / 4, |w| w.grid);
                    s.swing = Some(Swing::new(grid, lo + k as u8 - 1).ok_or("swing")?);
                    set(s)
                }
                None => end(
                    "Swing is the track's already".into(),
                    format!("Swing is at its most, {} %", CLIP_SWING.end()),
                ),
            }
        }
        3 => {
            let now = s.quantise.map_or(0, |q| {
                QUANTISE_GRIDS
                    .iter()
                    .position(|&g| g == q.grid)
                    .unwrap_or_else(|| {
                        QUANTISE_GRIDS
                            .iter()
                            .rposition(|&g| g >= q.grid)
                            .unwrap_or(0)
                    })
                    + 1
            });
            match step(now, QUANTISE_GRIDS.len() + 1) {
                Some(0) => {
                    s.quantise = None;
                    set(s)
                }
                Some(k) => {
                    let strength = s.quantise.map_or(100, |q| q.strength);
                    s.quantise =
                        Some(Quantise::new(QUANTISE_GRIDS[k - 1], strength).ok_or("quantise")?);
                    set(s)
                }
                None => end(
                    "Quantise is off already".into(),
                    "Quantise is at its finest, 1/32".into(),
                ),
            }
        }
        4 => {
            let q = s.quantise.ok_or("Turn Quantise on first")?;
            let next = if d > 0 {
                (q.strength / 10 + 1) * 10
            } else {
                (q.strength.div_ceil(10).saturating_sub(1)) * 10
            };
            if !(10..=100).contains(&next) {
                return end(
                    "Strength is at its least, 10 %".into(),
                    "Strength is full already, 100 %".into(),
                );
            }
            s.quantise = Some(Quantise::new(q.grid, next).ok_or("strength")?);
            set(s)
        }
        5 => {
            let now = s.follow.map_or(0, |f| {
                FOLLOWS
                    .iter()
                    .position(|&a| a == f.action)
                    .map_or(0, |k| k + 1)
            });
            match step(now, FOLLOWS.len() + 1) {
                Some(0) => {
                    s.follow = None;
                    set(s)
                }
                Some(k) => {
                    let action = FOLLOWS[k - 1];
                    s.follow = Some(match s.follow {
                        Some(f) => Follow { action, ..f },
                        // After one pass of the clip, to begin with.
                        None => {
                            let bar = st.transport.meter.bar_ticks() as f64;
                            let bars = (c.song_len() / bar).ceil() as i64;
                            Follow::after(bars.clamp(1, i64::from(MAX_FOLLOW_BARS)) as u16, action)
                        }
                    });
                    set(s)
                }
                None => end(
                    "Follow is off already".into(),
                    "Follow is at its last action, Stop".into(),
                ),
            }
        }
        6 => {
            let f = s.follow.ok_or("Turn Follow on first")?;
            match step(usize::from(f.bars) - 1, usize::from(MAX_FOLLOW_BARS)) {
                Some(k) => {
                    s.follow = Some(Follow {
                        bars: k as u16 + 1,
                        ..f
                    });
                    set(s)
                }
                None => end(
                    "Follow is at its soonest, after 1 bar".into(),
                    format!("Follow is at its latest, after {MAX_FOLLOW_BARS} bars"),
                ),
            }
        }
        // The project's, then its own: at once, the note grids, the bars.
        7 => {
            let now = s.launch.map_or(0, |g| {
                LAUNCH_GRIDS
                    .iter()
                    .position(|&x| x == g)
                    .map_or(0, |k| k + 1)
            });
            match step(now, LAUNCH_GRIDS.len() + 1) {
                Some(k) => {
                    s.launch = k.checked_sub(1).map(|k| LAUNCH_GRIDS[k]);
                    set(s)
                }
                None => end(
                    format!(
                        "Launch is the project's already ({})",
                        launch_name(st.launch)
                    ),
                    "Launch is at its longest, 8 bars".into(),
                ),
            }
        }
        // Restart, Legato, Once.
        8 => match step(mode_of(&s), MODES.len()) {
            Some(k) => {
                s.legato = k == 1;
                s.once = k == 2;
                set(s)
            }
            None => end(
                "Mode is at its first, Restart".into(),
                "Mode is at its last, Once: the clip plays one pass, then its track stops".into(),
            ),
        },
        // Off, then 1 to 128.
        9 => {
            let now = s.program.map_or(0, |p| usize::from(p.program) + 1);
            match step(now, 129) {
                Some(0) => {
                    s.program = None;
                    set(s)
                }
                Some(k) => {
                    let bank = s.program.and_then(|p| p.bank);
                    s.program = Some(ClipProgram {
                        program: (k - 1) as u8,
                        bank,
                    });
                    set(s)
                }
                None => end(
                    "Program is off already: the clip sends no program change".into(),
                    "Program is at its last, 128".into(),
                ),
            }
        }
        10 => bank_step(st, delta, 1),
        // Forward, Reverse, Ping-pong, Random (S6). On a clip it can't
        // move, back to Forward, then why not.
        CLIP_DIRECTION if direction_steps(Direction::Reverse, c.length).is_none() => {
            if s.direction == Direction::Forward {
                return Err("Direction moves the clip's 1/16 steps: it needs a clip of 2 whole steps or more, so this one plays forward".into());
            }
            s.direction = Direction::Forward;
            set(s)
        }
        CLIP_DIRECTION => {
            let now = Direction::ALL
                .iter()
                .position(|&x| x == s.direction)
                .unwrap_or(0);
            match step(now, Direction::ALL.len()) {
                Some(k) => {
                    s.direction = Direction::ALL[k];
                    set(s)
                }
                None => end(
                    "Direction is at its first, Forward".into(),
                    "Direction is at its last, Random".into(),
                ),
            }
        }
        // › a new variation (SHIFT: a bolder one, `card_action`), ‹ the
        // original back (S6).
        CLIP_MUTATE if d > 0 && c.notes == 0 && c.variation.is_none() => {
            Err("Mutate varies the clip's notes: this clip has none yet".into())
        }
        CLIP_MUTATE if d > 0 => Ok(TimeAction::Mutate {
            track,
            slot,
            more: false,
            melodic: st.tracks.get(track).is_some_and(|t| t.melodic),
        }),
        CLIP_MUTATE if c.variation.is_some() => Ok(TimeAction::Restore { track, slot }),
        CLIP_MUTATE => {
            Err("The clip plays its original: › mutates it, and ‹ then brings this back".into())
        }
        // Convert to notes (E1): › previews it on a clip that follows the
        // pads on a track whose instrument plays notes; the rest say why.
        CLIP_MELODY if c.holds_notes => {
            Err("The clip uses notes: each keeps its pitch, whatever the pads play".into())
        }
        CLIP_MELODY if !st.tracks.get(track).is_some_and(|t| t.melodic) => {
            Err("The clip follows the pads: on a kit a clip always does".into())
        }
        CLIP_MELODY if d > 0 => Ok(TimeAction::ConvertPreview { track, slot }),
        CLIP_MELODY => Err(
            "The clip follows the pads: it plays whatever they play now. › shows what Convert to notes would do"
                .into(),
        ),
        _ => Err(String::new()),
    }
}

/// A step of the card's Bank row by `by` (1, or 128 with SHIFT: a whole
/// CC 0): off, then 0/0 up to 127/127.
fn bank_step(st: &TimeState, delta: i32, by: i32) -> Result<TimeAction, String> {
    let (track, slot) = st
        .focus()
        .ok_or("No clip to set: arm a track with a clip, or make one")?;
    let c = st.clip(track, slot).ok_or("No clip to set")?;
    if c.looped.is_some() {
        return Err(
            "A loop cell plays its loop as recorded: only Follow, After and Launch apply".into(),
        );
    }
    let mut s = c.settings;
    let p = s
        .program
        .ok_or("Set Program first: the bank goes with its program change")?;
    s.program = Some(ClipProgram {
        bank: bank_next(p, delta, by)?,
        ..p
    });
    Ok(TimeAction::ClipSettings {
        track,
        slot,
        row: 10,
        settings: s,
    })
}

// ---------------------------------------------------------------- texts

/// A clip's length as the grid shows it: "2 bars", "6 beats", "7/16".
pub fn length_label(song_ticks: f64, meter: Meter) -> String {
    let whole = |unit: i64| {
        let n = song_ticks / unit as f64;
        (n >= 1.0 && (n - n.round()).abs() < 1e-6).then_some(n.round() as i64)
    };
    let plural = |n: i64, one: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {one}s")
        }
    };
    if let Some(n) = whole(meter.bar_ticks()) {
        plural(n, "bar")
    } else if let Some(n) = whole(meter.beat_ticks()) {
        plural(n, "beat")
    } else if let Some(n) = whole(i64::from(PPQ / 4)) {
        format!("{n}/16")
    } else {
        format!("{} ticks", song_ticks.round() as i64)
    }
}

/// The loop as the bar shows it: "off", "1" (one bar), "1–4", or positions.
/// The Record tab's Loop row: the loop's bars, or, following another
/// device's position, the loop that comes back after ("1–4, off while
/// following").
fn loop_row(t: &TransportInfo) -> String {
    match (t.follows_position, t.loop_range) {
        (false, range) => loop_label(range, t.meter),
        (true, None) => "off while following".to_string(),
        (true, held) => format!("{}, off while following", loop_label(held, t.meter)),
    }
}

pub fn loop_label(range: Option<(Tick, Tick)>, meter: Meter) -> String {
    let Some((a, b)) = range else {
        return "off".to_string();
    };
    let bar = meter.bar_ticks();
    if a % bar == 0 && b % bar == 0 {
        let (first, last) = (a / bar + 1, b / bar);
        if first == last {
            format!("{first}")
        } else {
            format!("{first}–{last}")
        }
    } else {
        format!("{}–{}", meter.position(a), meter.position(b))
    }
}

pub(crate) fn bar_beat(tick: Tick, meter: Meter) -> String {
    let p = meter.position(tick);
    format!("{}.{}", p.bar, p.beat)
}

/// The pending capture as Capture's button shows it: "1 bar", "4 bars".
pub fn bars_label(bars: i64) -> String {
    if bars == 1 {
        "1 bar".to_string()
    } else {
        format!("{bars} bars")
    }
}

/// "row 5", or "rows 5 to 7", counted from 1.
pub fn rows_named(first: usize, last: usize) -> String {
    if first == last {
        format!("row {}", first + 1)
    } else {
        format!("rows {} to {}", first + 1, last + 1)
    }
}

/// Whether the Song view's last four buttons are the picked section's
/// (`song::SECTION_BUTTONS`): a section picked, no audio region or tempo
/// point.
pub fn section_buttons(st: &TimeState) -> bool {
    st.region.is_none()
        && st.tempo_pick.is_none()
        && st.section.is_some_and(|k| k < st.markers.len())
}

/// The marker the Song view's Rename asks to name (button 4 with a
/// section picked), with its name: the app opens the text keyboard.
pub fn song_rename(ev: &UiEvent, st: &TimeState) -> Option<(usize, String)> {
    if ev.widget != SONG || ev.kind != EventKind::Tab(3) || !section_buttons(st) || !st.song_view {
        return None;
    }
    let k = st.section?;
    Some((k, st.markers.get(k)?.1.clone()))
}

pub fn take_title(st: &TimeState, row: &TakeRow) -> String {
    let mut word = row.word().to_string();
    word[..1].make_ascii_uppercase();
    match row.song {
        Some(
            SongChange::Moved { track, .. }
            | SongChange::Copied { track, .. }
            | SongChange::Resized { track, .. }
            | SongChange::Deleted { track, .. }
            | SongChange::Unique { track, .. }
            | SongChange::Recorded { track, .. }
            | SongChange::Split {
                track: Some(track), ..
            }
            | SongChange::Trimmed { track, .. }
            | SongChange::Slid { track, .. },
        ) => return format!("{word} {} · {}", row.shown(), st.track_name(track)),
        Some(SongChange::Split { track: None, at }) => {
            let bar = st.transport.meter.bar_ticks().max(1);
            return format!("{word} {} · bar {}", row.shown(), at.div_euclid(bar) + 1);
        }
        Some(SongChange::SectionMoved { at } | SongChange::SectionCopied { at }) => {
            let name = st
                .markers
                .iter()
                .find(|m| m.0 == at)
                .map_or("section", |m| m.1.as_str());
            return format!("{word} {} · {name}", row.shown());
        }
        Some(SongChange::Tempo { at } | SongChange::Bars { at, .. }) => {
            let bar = st.transport.meter.bar_ticks().max(1);
            return format!("{word} {} · bar {}", row.shown(), at.div_euclid(bar) + 1);
        }
        Some(SongChange::Marker { at }) => {
            let bar = st.transport.meter.bar_ticks().max(1);
            let name = st.markers.iter().find(|m| m.0 == at).map_or_else(
                || format!("bar {}", at.div_euclid(bar) + 1),
                |m| m.1.clone(),
            );
            return format!("{word} {} · {name}", row.shown());
        }
        Some(SongChange::Followed { row: r }) => {
            return format!("{word} {} · row {}", row.shown(), r + 1);
        }
        Some(
            SongChange::SeveralMoved { blocks, .. }
            | SongChange::SeveralCopied { blocks, .. }
            | SongChange::SeveralDeleted { blocks }
            | SongChange::SeveralSplit { blocks, .. },
        ) => return format!("{word} {} · {blocks} blocks", row.shown()),
        Some(
            SongChange::Scenes { .. }
            | SongChange::List
            | SongChange::ListToSong { .. }
            | SongChange::RowDeleted { .. }
            | SongChange::RecSong { .. },
        )
        | None => {}
    }
    if let Some((_, r)) = row.row_tool {
        return format!("{word} {} · row {}", row.shown(), r + 1);
    }
    if row.rows.is_some() {
        return format!("{word} {}", row.shown());
    }
    if let Some(r) = row.scene {
        return format!("{word} {} · row {}", row.shown(), r + 1);
    }
    match (row.at, row.dropped) {
        (Some((track, slot)), _) => format!(
            "{word} {} · {} / {}",
            row.shown(),
            st.track_name(track),
            slot + 1
        ),
        (None, Some(ClipDrop::RowMoved { from, .. } | ClipDrop::RowCopied { from, .. })) => {
            format!("{word} {} · row {}", row.shown(), from + 1)
        }
        (None, _) => format!("{word} {}", row.shown()),
    }
}

/// What a take did, short ("new +12", "to Bass / 3"), ending "· and the
/// song" when the song's parts that follow a row came along.
fn take_detail(st: &TimeState, row: &TakeRow) -> String {
    let said = take_said(st, row);
    match row.followed {
        Some(_) => format!("{said} · and the song"),
        None => said,
    }
}

fn take_said(st: &TimeState, row: &TakeRow) -> String {
    match row.row_tool {
        Some(("capture", r)) => return format!("into row {}", r + 1),
        Some((_, r)) => return format!("row {}, the rows under it up one", r + 1),
        None => {}
    }
    if let Some((first, last)) = row.rows {
        return rows_named(first, last);
    }
    let cell = |(t, s): (usize, usize)| format!("{} / {}", st.track_name(t), s + 1);
    let bar = st.transport.meter.bar_ticks().max(1);
    let bar_of = |t: Tick| t.div_euclid(bar) + 1;
    match row.song {
        Some(
            SongChange::Moved { start, .. }
            | SongChange::Copied { start, .. }
            | SongChange::SectionMoved { at: start }
            | SongChange::SectionCopied { at: start },
        ) => return format!("to bar {}", bar_of(start)),
        Some(
            SongChange::Deleted { start, .. }
            | SongChange::Unique { start, .. }
            | SongChange::Recorded { start, .. }
            | SongChange::Split { at: start, .. }
            | SongChange::Slid { start, .. },
        ) => return format!("at bar {}", bar_of(start)),
        Some(SongChange::Trimmed { start, .. }) => return format!("from bar {}", bar_of(start)),
        Some(SongChange::Resized { length, .. }) if length % bar == 0 => {
            return format!("{} bars", length / bar)
        }
        Some(SongChange::Resized { length, .. }) => {
            return format!("{:.1} bars", length as f64 / bar as f64)
        }
        Some(SongChange::Scenes { rows }) => return format!("{rows} rows"),
        Some(SongChange::RecSong { placements: 1 }) => return "1 block".into(),
        Some(SongChange::RecSong { placements }) => return format!("{placements} blocks"),
        Some(SongChange::List) => return "song list".into(),
        Some(SongChange::ListToSong { steps }) => return format!("{steps} steps → song"),
        Some(SongChange::Tempo { .. }) => return "tempo point".into(),
        Some(SongChange::Bars { len, inserted, .. }) => {
            let n = if len % bar == 0 {
                format!("{}", len / bar)
            } else {
                format!("{:.1}", len as f64 / bar as f64)
            };
            let bars = if n == "1" { "bar" } else { "bars" };
            return if inserted {
                format!("{n} {bars} put in")
            } else {
                format!("{n} {bars} taken out")
            };
        }
        Some(SongChange::Marker { at }) => return format!("at bar {}", bar_of(at)),
        Some(SongChange::Followed { .. }) => return "the song's parts".into(),
        Some(
            SongChange::SeveralMoved { blocks, .. } | SongChange::SeveralCopied { blocks, .. },
        ) => return format!("{blocks} blocks"),
        Some(SongChange::SeveralDeleted { blocks }) => return format!("{blocks} blocks"),
        Some(SongChange::SeveralSplit { at, .. }) => return format!("at bar {}", bar_of(at)),
        Some(SongChange::RowDeleted { row: r }) => return format!("row {}", r + 1),
        None => {}
    }
    match row.dropped {
        Some(ClipDrop::Moved(_, b) | ClipDrop::Copied(_, b)) => return format!("to {}", cell(b)),
        Some(ClipDrop::Swapped(_, b)) => return format!("with {}", cell(b)),
        Some(ClipDrop::RowMoved { from, to }) => {
            return format!("to row {}", if to > from { to } else { to + 1 })
        }
        Some(ClipDrop::RowCopied { to, .. }) => return format!("to row {}", to + 1),
        None => {}
    }
    if row.scene.is_some() {
        return "scene settings".into();
    }
    if row.part {
        return "a new part".into();
    }
    if let Some(r) = row.chords {
        return format!("row {}'s chords", r + 1);
    }
    if row.style {
        return "a style from a clip".into();
    }
    match (row.replaced, row.variant) {
        (Some(Replaced::Reload), _) => return "saved state".into(),
        (Some(Replaced::Mutate), Some(n)) => return format!("variant {n}"),
        (Some(Replaced::Mutate), None) => return "a variation".into(),
        (Some(Replaced::Restore), _) => return "the original".into(),
        (Some(Replaced::Fit), _) => return "to tempo".into(),
        (Some(Replaced::Settings), _) => return "clip settings".into(),
        (Some(Replaced::Convert), _) => return "converted to notes".into(),
        (Some(Replaced::Continue { bars: 1 }), _) => return "1 more bar".into(),
        (Some(Replaced::Continue { bars }), _) => return format!("{bars} more bars"),
        (Some(Replaced::FitChords), _) => return "the new chords".into(),
        (None, _) => {}
    }
    if row.deleted {
        return "clip".into();
    }
    if row.settings {
        return "clip settings".into();
    }
    if row.pasted {
        return "a copy".into();
    }
    let new = if row.made { "new " } else { "" };
    if row.erased > 0 {
        format!("{new}+{} −{}", row.added, row.erased)
    } else {
        format!("{new}+{}", row.added)
    }
}

// ---------------------------------------------------------------- what the widgets show

/// The bar for `st`.
pub fn bar_of(st: &TimeState) -> TransportBar {
    let t = &st.transport;
    let mut bar = TransportBar::new();
    bar.run = match t.state {
        State::Stopped => Run::Stopped,
        State::CountIn => Run::CountIn,
        State::Playing => Run::Playing,
        State::Recording => Run::Recording,
    };
    bar.position = t.meter.position(t.position).to_string();
    let swing = if t.swing.amount == 50 {
        "off".to_string()
    } else {
        format!("{}%", t.swing.amount)
    };
    bar.items = vec![];
    // The mute queue first, so it shows whatever else fits (S4).
    if let Some(w) = st.waiting_reading() {
        bar.items.push(BarItem::new("WAITING", w, Tone::Warn));
    }
    // Perform's NEXT: what comes next, and when (S4).
    if let Some(n) = st.next_reading() {
        bar.items.push(BarItem::new("NEXT", n, Tone::Accent));
    }
    bar.items.extend([
        BarItem::new("BPM", format!("{:.1}", t.bpm), Tone::Normal),
        BarItem::new(
            "",
            format!("{}/{}", t.meter.beats, t.meter.unit),
            Tone::Normal,
        ),
        // When a tapped clip, a stop or a scene lands (S4; the Record
        // tab's Launch row sets it).
        BarItem::new("Launch", launch_name(st.launch), Tone::Normal),
        BarItem::new("Swing", swing, Tone::Normal),
        // Following another device's position, its loop is off: greyed.
        BarItem::new(
            "Loop",
            loop_label(t.loop_range, t.meter),
            if t.playing_loop().is_some() {
                Tone::Accent
            } else {
                Tone::Dim
            },
        ),
    ]);
    if t.state == State::CountIn {
        let bars = if t.count_in == 1 {
            "1 bar".to_string()
        } else {
            format!("{} bars", t.count_in)
        };
        bar.items
            .insert(0, BarItem::new("Count-in", bars, Tone::Warn));
    }
    bar.overdub = t.overdub;
    bar.undo = st.done.last().map(TakeRow::short);
    bar.chips = st.chips.clone();
    bar
}

/// Where a loop sits on the Clips grid, as Play's strip and the Looper
/// tab name it: its Loop track and its row (the first, if it is in more
/// than one).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoopPlace {
    pub track: usize,
    pub slot: usize,
    /// "Loops B".
    pub track_name: String,
    /// The row's scene name, or "row 2".
    pub row: String,
    /// Its cell's name: a loop cell on an audio track is the loop's name
    /// (phase 4, board 5: "Vox loop").
    pub clip: String,
    /// It is an audio track's cell (phase 4): the track's mute and solo
    /// are the loop's.
    pub audio: bool,
    pub mute: bool,
    pub solo: bool,
}

impl LoopPlace {
    /// "Loops B · Loop 2 · Verse".
    pub fn full(&self, n: usize) -> String {
        format!("{} · Loop {} · {}", self.track_name, n + 1, self.row)
    }
}

/// Row `slot`'s name: its scene's, or "row 2".
pub fn row_name(st: &TimeState, slot: usize) -> String {
    match st.scenes.get(slot).filter(|s| !s.is_empty()) {
        Some(name) => name.clone(),
        None => format!("row {}", slot + 1),
    }
}

/// The armed Loop cell as the REC chip names it: "Loops C · Drop".
pub fn loop_cell_name(st: &TimeState) -> Option<String> {
    let (t, s) = st.loop_cell?;
    Some(format!("{} · {}", st.tracks.get(t)?.name, row_name(st, s)))
}

impl TimeState {
    /// The looper's slot holding the loop a cell plays: a Loop track's
    /// cell names it (`ClipInfo::looped`), a loop cell on an audio track
    /// names the loop's id, which the slots hold ([`LoopsInfo::ids`]).
    pub fn loop_slot(&self, clip: &ClipInfo) -> Option<usize> {
        if let Some(n) = clip.looped {
            return Some(usize::from(n));
        }
        let id = clip.loop_id?.0;
        (id != 0).then(|| self.loops.ids.iter().position(|&i| i == id))?
    }

    /// The armed empty cell is an audio track's (phase 4): REC records a
    /// loop there.
    pub fn loop_cell_audio(&self) -> bool {
        self.loop_cell
            .is_some_and(|(t, _)| self.tracks.get(t).is_some_and(|t| t.audio))
    }

    /// How many audio cells hold a loop (phase 4).
    pub fn loop_cells(&self) -> usize {
        self.tracks
            .iter()
            .flat_map(|t| t.slots.iter().flatten())
            .filter(|c| c.loop_id.is_some())
            .count()
    }
}

/// Each loop's place on the grid; None for a loop in no cell.
pub fn loop_places(st: &TimeState) -> [Option<LoopPlace>; mpfree_seq::LOOPS] {
    let mut out: [Option<LoopPlace>; mpfree_seq::LOOPS] = Default::default();
    for (track, t) in st.tracks.iter().enumerate() {
        for (slot, c) in t.slots.iter().enumerate() {
            // A Loop track's cell, or a loop cell on an audio track (phase
            // 4), by the slot holding its loop.
            let Some(n) = c.as_ref().and_then(|c| st.loop_slot(c)) else {
                continue;
            };
            let Some(place @ None) = out.get_mut(n) else {
                continue;
            };
            *place = Some(LoopPlace {
                track,
                slot,
                track_name: t.name.clone(),
                row: row_name(st, slot),
                clip: c.as_ref().map(|c| c.name.clone()).unwrap_or_default(),
                audio: t.audio,
                mute: t.mute,
                solo: t.solo,
            });
        }
    }
    out
}

/// The empty clip's card for clip `slot` of `track` (composing tools
/// S2.6, board D): its row, and the row's other clips with notes, by name.
pub fn empty_card(st: &TimeState, track: usize, slot: usize) -> crate::editor::EmptyCard {
    let names: Vec<&str> = st
        .tracks
        .iter()
        .enumerate()
        .filter(|(t, _)| *t != track)
        .filter_map(|(t, _)| st.clip(t, slot))
        .filter(|c| c.notes > 0)
        .map(|c| c.name.as_str())
        .collect();
    let heard = match names.as_slice() {
        [] => String::new(),
        [a] => (*a).to_string(),
        [a, b] => format!("{a} and {b}"),
        [a, b, c] => format!("{a}, {b} and {c}"),
        [a, b, c, rest @ ..] => format!("{a}, {b}, {c} and {} more", rest.len()),
    };
    crate::editor::EmptyCard {
        row: slot,
        heard,
        heard_n: names.len(),
    }
}

/// Cell `slot` of `track` for `st`: the part Add a part is trying in its
/// empty cell (dashed in the track's colour, "try N"), and a kept clip
/// off its row's chords marked "doesn't fit" (composing tools S2.6).
pub fn cell_of(st: &TimeState, track: usize, slot: usize) -> GridCell {
    if let Some(c) = crate::add_view::try_cell(st, track, slot) {
        return c;
    }
    let mut cell = clip_cell(st, track, slot);
    if crate::add_view::unfit(st, track, slot) {
        cell.mark = CellMark::Unfit;
        if matches!(cell.state, CellState::Stopped | CellState::Playing) && st.cue(track).is_none()
        {
            cell.detail = "doesn't fit".to_string();
        }
    }
    cell
}

/// Cell `slot` of `track` as its clip and its launch make it.
fn clip_cell(st: &TimeState, track: usize, slot: usize) -> GridCell {
    let Some(clip) = st.clip(track, slot) else {
        if st.loop_cell == Some((track, slot)) {
            // The armed Loop cell: the next loop recorded lands here.
            // Short: the column is 86 px.
            let audio = st.tracks.get(track).is_some_and(|t| t.audio);
            let (name, detail, foot) = if st.loops.recording {
                ("REC", "REC ends", "the pass")
            } else if st.loops.waiting {
                ("Ready", "bar line", "or PLAY")
            } else if audio {
                // An armed empty audio cell (phase 4, board 2): REC
                // records a loop there.
                ("REC here", "a loop", "")
            } else {
                ("Armed", "REC here", "")
            };
            return GridCell {
                state: CellState::Armed,
                name: name.to_string(),
                detail: detail.to_string(),
                foot: foot.to_string(),
                ..GridCell::default()
            };
        }
        return GridCell::default();
    };
    let t = &st.transport;
    let len = clip.song_len().max(1.0);
    // A loop cell on an audio track (phase 4, board 2): its mark, its
    // length and the deck's ring in the track's colour, never a wave.
    let looped = clip.loop_id.is_some();
    let facts = st
        .loop_slot(clip)
        .and_then(|n| st.loops.loops.get(n))
        .copied();
    // A Loop track's cell shows its wave in the detail's place, and the
    // detail at the foot.
    let notes = match (facts, clip.notes) {
        _ if looped => String::new(),
        (Some(_), _) => String::new(),
        (None, 1) => "1 note".to_string(),
        (None, n) => format!("{n} notes"),
    };
    if let Some(a) = &clip.audio {
        return audio_cell(st, track, slot, clip, a);
    }
    let mut cell = GridCell {
        state: CellState::Stopped,
        name: clip.name.clone(),
        detail: length_label(len, t.meter),
        foot: notes,
        progress: 0.0,
        wave: facts.filter(|_| !looped).map(|f| f.wave),
        looped,
        writing: looped && facts.is_some_and(|f| f.writing),
        ..GridCell::default()
    };
    if facts.is_some_and(|f| f.copying) {
        cell.foot = "copying in".to_string();
        cell.alert = if looped {
            CellAlert::Warn
        } else {
            CellAlert::None
        };
    } else if cell.writing {
        // A layer not yet on the stick (layout spec 4f): in amber, in
        // the length's place while it is stopped.
        cell.foot = "writing".to_string();
        cell.alert = CellAlert::Warn;
    }
    let cue = st.cue(track);
    if cue.is_some_and(|c| c.slot == Some(slot)) {
        cell.state = CellState::Queued;
        let line = st.cue_line(track);
        let at = cue_tick(t, line);
        st.queued_foot(&mut cell, at, line.line(t.meter.bar_ticks()));
        return cell;
    }
    let Some(l) = st.launch(track).filter(|l| l.slot == slot) else {
        return cell;
    };
    if l.origin > t.position {
        cell.state = CellState::Queued;
        st.queued_foot(&mut cell, l.origin, t.meter.bar_ticks());
        return cell;
    }
    // In steps of 1/64 of a pass, so the ring is not redrawn every frame.
    let frac = ((t.position - l.origin) as f64).rem_euclid(len) / len;
    cell.progress = ((frac * 64.0).floor() / 64.0) as f32;
    // The ring takes the foot's place.
    cell.foot.clear();
    if t.state == State::Recording && st.armed == Some(track) {
        cell.state = CellState::Recording;
        cell.detail = "recording".to_string();
    } else if facts.is_some_and(|f| f.overdubbing) {
        cell.state = CellState::Recording;
        cell.detail = "overdub".to_string();
    } else {
        cell.state = CellState::Playing;
    }
    if let Some(c) = cue {
        // It plays until the cue starts; a stop in song mode gives the
        // track the song again (D3's Next).
        let word = match c.slot {
            Some(_) => "until",
            None if st.song_on && st.song_has(track) => "to song",
            None => "stops",
        };
        let line = st.cue_line(track);
        let at = cue_tick(t, line);
        cell.detail = format!("{word} {}", bar_beat(at, t.meter));
        cell.cued = true;
        if c.slot.is_none() && st.perform && t.state.rolling() {
            // The stop's countdown, as a queued cell's (the next clip's
            // cell has its own).
            let span = line.line(t.meter.bar_ticks()).max(1);
            let left = (at - t.position).max(0);
            let frac = 1.0 - (left.min(span) as f64 / span as f64);
            cell.countdown = ((frac * 64.0).floor().max(1.0) / 64.0) as f32;
        }
    }
    cell
}

/// An audio clip's cell (layout spec 4f): its name and wave, and under
/// them (Perform) its length, "2 bars", or a once clip's time, "3:17
/// once". A word that matters takes the wave's place while it is stopped
/// or queued (Make) and the length's (Perform): "from 17.1" or, its audio
/// late, "waits 18.1" (amber) while queued; stopped, "missing" or "not
/// ready" (red), or "made at 120" (amber) for a loop made at another
/// tempo. A once clip still sounding past its stop shows how far it has
/// got, its playhead dimmed; a missing clip a flat dashed line.
fn audio_cell(
    st: &TimeState,
    track: usize,
    slot: usize,
    clip: &ClipInfo,
    a: &AudioCellInfo,
) -> GridCell {
    let t = &st.transport;
    let len = clip.song_len().max(1.0);
    let detail = if clip.settings.once {
        format!("{} once", clock_time(a.secs))
    } else {
        length_label(len, t.meter)
    };
    let mut cell = GridCell {
        state: CellState::Stopped,
        name: clip.name.clone(),
        detail,
        wave: a.missing.is_none().then_some(a.wave),
        missing: a.missing.is_some(),
        ..GridCell::default()
    };
    let late = st.audio_late.iter().find(|l| l.0 == track).map(|l| l.1);
    let cue = st.cue(track);
    if cue.is_some_and(|c| c.slot == Some(slot)) {
        cell.state = CellState::Queued;
        cell.alert = CellAlert::Warn;
        let line = st.cue_line(track);
        let at = late.unwrap_or_else(|| cue_tick(t, line));
        st.queued_foot(&mut cell, at, line.line(t.meter.bar_ticks()));
        if late.is_some() {
            cell.foot = format!("waits {}", bar_beat(at, t.meter));
        }
        return cell;
    }
    match st.launch(track).filter(|l| l.slot == slot) {
        Some(l) if l.origin > t.position => {
            cell.state = CellState::Queued;
            cell.alert = CellAlert::Warn;
            st.queued_foot(&mut cell, l.origin, t.meter.bar_ticks());
        }
        Some(l) => {
            let frac = ((t.position - l.origin) as f64).rem_euclid(len) / len;
            cell.progress = ((frac * 64.0).floor() / 64.0) as f32;
            cell.state = CellState::Playing;
            if let Some(c) = cue {
                let word = if c.slot.is_none() { "stops" } else { "until" };
                cell.detail = format!(
                    "{word} {}",
                    bar_beat(cue_tick(t, st.cue_line(track)), t.meter)
                );
            }
        }
        None => {
            if let Some(r) = a.ring {
                cell.ringing = true;
                cell.progress = ((f64::from(r) * 64.0).floor() / 64.0) as f32;
            }
            let word = audio_word(st, track, slot, clip, a);
            if let Some((word, alert)) = word.filter(|_| !cell.ringing) {
                cell.foot = word;
                cell.alert = alert;
            }
        }
    }
    cell
}

/// What a stopped audio clip's cell says in its wave's place, if anything:
/// "missing", "not ready" (red), or "made at 120" (amber) for a loop made
/// at another tempo than the project's now.
fn audio_word(
    st: &TimeState,
    track: usize,
    slot: usize,
    clip: &ClipInfo,
    a: &AudioCellInfo,
) -> Option<(String, CellAlert)> {
    if a.missing.is_some() {
        Some(("missing".to_string(), CellAlert::Bad))
    } else if st.not_ready.iter().any(|n| (n.0, n.1) == (track, slot)) {
        Some(("not ready".to_string(), CellAlert::Bad))
    } else if off_tempo(clip, a, st.transport.bpm) {
        Some((format!("made at {}", tempo_short(a.bpm)), CellAlert::Warn))
    } else {
        None
    }
}

/// An audio loop made at another tempo than `bpm`: each pass leaves a gap
/// or cuts its sound short. A once clip plays to its end at any tempo.
fn off_tempo(clip: &ClipInfo, a: &AudioCellInfo, bpm: f64) -> bool {
    !clip.settings.once && (a.bpm - bpm).abs() >= 0.05
}

/// A tempo as a cell says it: "120", "92.5".
pub fn tempo_short(bpm: f64) -> String {
    let tenths = (bpm * 10.0).round();
    if tenths % 10.0 == 0.0 {
        format!("{}", tenths / 10.0)
    } else {
        format!("{:.1}", tenths / 10.0)
    }
}

/// Seconds as a clock says them: "3:17", "0:12".
pub fn clock_time(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// What a tap on an audio clip's cell says in full on the hint line
/// (layout spec 4f): its word's meaning, or an empty cell's. None when it
/// has nothing to say.
pub fn audio_cell_hint(st: &TimeState, track: usize, slot: usize) -> Option<String> {
    if !st.tracks.get(track)?.audio {
        return None;
    }
    let Some(clip) = st.clip(track, slot) else {
        if st.loops_on {
            let name = &st.tracks[track].name;
            return Some(format!(
                "An empty cell of {name}: arm {name} (tap its name), then tap the cell to record a loop there; or put audio in with To a cell on the Song view."
            ));
        }
        return Some(EMPTY_AUDIO_CELL.to_string());
    };
    let a = clip.audio.as_ref()?;
    let name = &clip.name;
    let t = &st.transport;
    if let Some(why) = &a.missing {
        return Some(format!("{name}'s audio is missing: {why}."));
    }
    if let Some(&(_, line)) = st.audio_late.iter().find(|l| l.0 == track) {
        if st.cue(track).is_some_and(|c| c.slot == Some(slot)) {
            return Some(format!(
                "{name} waits for its audio: it starts on {}.",
                bar_beat(line, t.meter)
            ));
        }
    }
    if let Some(n) = st.not_ready.iter().find(|n| (n.0, n.1) == (track, slot)) {
        return Some(format!(
            "{name}'s audio wasn't ready within {}. Tap to try again.",
            length_label(n.2 as f64, t.meter)
        ));
    }
    if off_tempo(clip, a, t.bpm) {
        let gap = pass_gap(clip, a, t.bpm);
        let what = if gap > 0.0 {
            format!("leaves {:.1} s of silence each pass", gap)
        } else {
            format!("cuts {:.1} s off each pass", -gap)
        };
        return Some(format!(
            "{name} was made at {} BPM: at {} it {what}. Fit to tempo on the Clip tab fixes it.",
            tempo_short(a.bpm),
            tempo_short(t.bpm)
        ));
    }
    None
}

/// The hint for a tap on an empty audio cell (layout spec 4f).
pub const EMPTY_AUDIO_CELL: &str =
    "An empty audio cell: put audio in with To a cell on the Song view, or Put in a cell on the Sample page.";

/// How long `left` ticks are, as Perform's countdowns say it: whole
/// beats, rounded up, under a bar ("1 beat", "3 beats"), whole bars past
/// one ("2 bars").
pub fn countdown(left: Tick, meter: Meter) -> String {
    let left = left.max(1);
    let beat = meter.beat_ticks().max(1);
    let bar = meter.bar_ticks().max(1);
    let (n, one, many) = if left >= bar {
        ((left + bar - 1) / bar, "bar", "bars")
    } else {
        ((left + beat - 1) / beat, "beat", "beats")
    };
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// What Link's two choices do, for its chooser (4b.5): "The list's 7
/// steps become the song; its old bars go after them" and "The song's 5
/// marked parts become the list, each × 1".
fn link_choices(st: &TimeState) -> [String; 2] {
    let steps = |n: usize| match n {
        1 => "1 step becomes".to_string(),
        n => format!("{n} steps become"),
    };
    let empty = st.placements.is_empty() && st.markers.is_empty();
    let from_list = match st.list.len() {
        0 => "The list has no steps: the song stays as it is".to_string(),
        n if empty => format!("The list's {} the song", steps(n)),
        n => format!(
            "The list's {} the song; its old bars go after them",
            steps(n)
        ),
    };
    let from_song = match st.markers.len() {
        0 if empty => "The song is empty: so is the list, to start".to_string(),
        0 => "The song has no markers: it becomes the list's one part".to_string(),
        n => {
            let parts = if n == 1 {
                "1 marked part becomes".to_string()
            } else {
                format!("{n} marked parts become")
            };
            let once = if st.passes.iter().all(Option::is_none) {
                ", each × 1"
            } else {
                ""
            };
            format!("The song's {parts} the list{once}")
        }
    };
    [from_list, from_song]
}

/// What a tap on a linked list's Song row says (the song's bars before its
/// first marker, phase 4b).
pub const SONG_ROW_WHY: &str =
    "Bars before the first marker: add a marker on the Song view's ruler to make them a part";

/// Whether a song list `Step` (`index`: a step's times, or its scene from
/// [`SCENE_STEP`]) is on a linked list's Song row.
fn list_lead(st: &TimeState, index: usize) -> bool {
    let k = index.checked_sub(SCENE_STEP).unwrap_or(index);
    st.list.get(k).is_some_and(|r| r.lead)
}

/// The song list's title, after the view chip's List (5.7): "7 steps ·
/// 76 bars · 2:27" (the time at the tempo now), or nothing while it is
/// empty; linked to the song (phase 4b), "Linked to the song · 7 parts ·
/// 50 bars · 2:10".
pub fn list_title(st: &TimeState) -> String {
    let linked = "Linked to the song";
    if st.list.is_empty() {
        return if st.list_linked { linked } else { "" }.to_string();
    }
    let n = st.list.len();
    let bars: i64 = st.list.iter().map(|r| r.bars).sum();
    let t = &st.transport;
    let quarters = (bars * t.meter.bar_ticks()) as f64 / PPQ as f64;
    let secs = (quarters * 60.0 / t.bpm.max(1.0)).round() as i64;
    let (one, many) = if st.list_linked {
        ("part", "parts")
    } else {
        ("step", "steps")
    };
    let head = if st.list_linked {
        format!("{linked} · ")
    } else {
        String::new()
    };
    format!(
        "{head}{n} {} · {bars} {} · {}:{:02}",
        if n == 1 { one } else { many },
        if bars == 1 { "bar" } else { "bars" },
        secs / 60,
        secs % 60
    )
}

/// Convert to notes' preview (E1), open on clip `slot` of `track`.
#[derive(Clone, Debug, PartialEq)]
pub struct Converting {
    pub track: usize,
    pub slot: usize,
    /// It would convert now: nothing to fix and the transport stopped.
    pub ok: bool,
    /// What it would make, or every reason it can't: the hint line.
    pub line: String,
}

/// The scene playing's follow action, for Perform's NEXT (S4): on song
/// tick `at` it plays `follow` from row `row`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FollowNext {
    pub at: Tick,
    pub row: usize,
    pub follow: Follow,
}

impl TimeState {
    /// The reload of clip `at` (track, slot) waits for its line.
    pub fn reload_waiting(&self, at: (usize, usize)) -> bool {
        self.reloads.iter().any(|r| (r.0, r.1) == at)
    }

    /// The song has something on `track` to play: a block on its lane,
    /// or an audio track's region. The placements are in the song's order,
    /// by track, so a search finds the track's first block: every track's
    /// header and cells ask, every frame, of a song of up to 4,096 blocks.
    pub fn song_has(&self, track: usize) -> bool {
        let k = self.placements.partition_point(|p| p.track < track);
        self.placements.get(k).is_some_and(|p| p.track == track)
            || self
                .audio_lane_at(track)
                .is_some_and(|l| !l.blocks.is_empty())
    }

    /// What `track` plays now (the Arrange refactor's D3, "Now"), as the
    /// player has it: the clip launched on it once it has started, else
    /// in song mode the song (a track with no clip of its own plays it,
    /// and a stopped one goes back to it), else nothing. A muted track
    /// still says what it would play.
    pub fn plays(&self, track: usize) -> Plays {
        let t = &self.transport;
        if let Some(l) = self.launch(track).filter(|l| l.origin <= t.position) {
            Plays::Clip(l.slot)
        } else if self.song_on && self.song_has(track) {
            Plays::Song
        } else {
            Plays::Nothing
        }
    }

    /// `track`'s SONG tag on the grid (D3): in song mode, on a track the
    /// song has something for, filled while it plays the song and
    /// outlined while a clip plays over it.
    pub fn song_mark(&self, track: usize) -> SongMark {
        if !self.song_on || !self.song_has(track) {
            return SongMark::None;
        }
        match self.plays(track) {
            Plays::Clip(_) => SongMark::Under,
            _ => SongMark::Plays,
        }
    }

    /// The clips launched over the song, for the Song view to draw over
    /// their tracks' lanes (D3): the one playing, from where it started
    /// to its cue's line while one waits (the song comes back there after
    /// a stop or Back to song), and the one waiting, from its line. None
    /// with song mode off.
    pub fn lanes_over(&self) -> Vec<LaneOver> {
        if !self.song_on {
            return Vec::new();
        }
        let t = &self.transport;
        let mut out = Vec::new();
        for (track, tr) in self.tracks.iter().enumerate() {
            if tr.audio {
                continue;
            }
            let cue = self.cue(track);
            let line = cue.map(|_| cue_tick(t, self.cue_line(track)));
            let launch = self.launch(track);
            if let Some(l) = launch.filter(|l| l.origin <= t.position) {
                let back = cue.is_some_and(|c| c.slot.is_none());
                out.extend(self.over(track, l.slot, l.origin, line, back, false));
            }
            let waiting = match (cue.and_then(|c| c.slot), line) {
                (Some(next), Some(at)) => Some((next, at)),
                _ => launch
                    .filter(|l| l.origin > t.position)
                    .map(|l| (l.slot, l.origin)),
            };
            if let Some((slot, from)) = waiting {
                out.extend(self.over(track, slot, from, None, false, true));
            }
        }
        out
    }

    /// One clip over the song on `track`'s lane, if `slot` holds one.
    fn over(
        &self,
        track: usize,
        slot: usize,
        from: Tick,
        until: Option<Tick>,
        back: bool,
        waiting: bool,
    ) -> Option<LaneOver> {
        let clip = self.clip(track, slot)?;
        Some(LaneOver {
            track,
            name: clip.name.clone(),
            from,
            until,
            back,
            waiting,
        })
    }

    /// Clips play over the song: song mode is on and a clip is launched,
    /// or waits to be (the grid's "Back to song", S4).
    pub fn over_song(&self) -> bool {
        self.song_on && (!self.launches.is_empty() || self.cues.iter().any(|c| c.slot.is_some()))
    }

    /// A queued cell's foot: where it starts ("from 7.1"); in Perform also
    /// its countdown, filling over the `span` ticks before the line (its
    /// launch grid's).
    fn queued_foot(&self, cell: &mut GridCell, at: Tick, span: Tick) {
        let t = &self.transport;
        cell.foot = format!("from {}", bar_beat(at, t.meter));
        if self.perform && t.state.rolling() {
            let left = (at - t.position).max(0);
            let span = span.max(1);
            let frac = 1.0 - (left.min(span) as f64 / span as f64);
            // In steps of 1/64, as a playing clip's ring; never empty, so
            // the countdown shows from the tap.
            cell.progress = ((frac * 64.0).floor().max(1.0) / 64.0) as f32;
        }
    }

    /// What Perform's NEXT says comes next, and when: a scene launched
    /// ("Drop in 2 beats"), else the clips cued ("Drop bass in 1 bar",
    /// "Bass stops in 3 beats", "Drop bass +2 in 1 bar"), else the
    /// scene playing's follow action ("Verse in 3 bars"), else in song
    /// mode the next marker ("Chorus in 4 bars"). None with nothing
    /// coming, in Make, or stopped.
    pub fn next_reading(&self) -> Option<String> {
        let t = &self.transport;
        if !self.perform || !t.state.rolling() {
            return None;
        }
        let cue_at = |c: &Cue| cue_tick(t, self.cue_line(c.track));
        let soonest = self.cues.iter().map(cue_at).min();
        let what = if let Some(row) = self.scene_cued {
            let at = soonest.unwrap_or_else(|| cue_tick(t, self.launch));
            Some((row_name(self, row), at))
        } else if let Some(at) = soonest {
            let names: Vec<String> = self
                .cues
                .iter()
                .filter(|c| cue_at(c) == at)
                .map(|c| match c.slot.and_then(|s| self.clip(c.track, s)) {
                    Some(clip) => clip.name.clone(),
                    None => format!("{} stops", self.track_name(c.track)),
                })
                .collect();
            let shown = match names.len() {
                0 | 1 => names.join(""),
                n => format!("{} +{}", names[0], n - 1),
            };
            Some((shown, at))
        } else if let Some(&(track, slot, at)) = self
            .reloads
            .iter()
            .filter(|r| r.2 > t.position)
            .min_by_key(|r| r.2)
        {
            let name = self
                .clip(track, slot)
                .map_or_else(|| self.track_name(track), |c| c.name.clone());
            Some((format!("{name} reloads"), at))
        } else if let Some((step, at)) = self.list_next.filter(|n| n.1 > t.position) {
            // The song list's next step, or its end (a list that goes
            // round names its first step).
            Some((
                match step.and_then(|k| self.list.get(k)) {
                    Some(r) => r.name.clone(),
                    None => "Stop all".to_string(),
                },
                at,
            ))
        } else if let Some(f) = self.follow_next.filter(|f| f.at > t.position) {
            self.follow_words(f).map(|w| (w, f.at))
        } else if self.song_on {
            self.markers
                .iter()
                .filter(|(at, _)| *at > t.position)
                .min_by_key(|(at, _)| *at)
                .map(|(at, name)| (name.clone(), *at))
        } else {
            None
        };
        let (name, at) = what?;
        Some(format!("{name} in {}", countdown(at - t.position, t.meter)))
    }

    /// What a scene's follow action plays, as NEXT names it: the row
    /// ("Drop"), "Verse again", "Stop all", "Any scene", "Another scene";
    /// with a chance, both ("Drop or stay"). None when it stays.
    fn follow_words(&self, f: FollowNext) -> Option<String> {
        let filled: Vec<bool> = (0..ROWS)
            .map(|r| {
                self.tracks
                    .iter()
                    .any(|tr| tr.slots.get(r).is_some_and(Option::is_some))
            })
            .collect();
        let words = |a: FollowAction| -> Option<String> {
            Some(match a {
                FollowAction::Stay => return None,
                FollowAction::Again => format!("{} again", row_name(self, f.row)),
                FollowAction::Stop => "Stop all".to_string(),
                FollowAction::Any => "Any scene".to_string(),
                FollowAction::Other => "Another scene".to_string(),
                _ => row_name(
                    self,
                    mpfree_seq::scenes::follow_target(a, f.row, &filled, 0)?,
                ),
            })
        };
        let fw = f.follow;
        if fw.chance >= 100 || fw.action == fw.other {
            words(fw.action)
        } else if fw.chance == 0 {
            words(fw.other)
        } else {
            let one = |a| words(a).unwrap_or_else(|| "stay".to_string());
            Some(format!(
                "{} or {}",
                one(fw.action),
                one(fw.other).to_lowercase()
            ))
        }
    }
}

/// Cues kept back until their audio is held in memory, while takes
/// stream from their files: how many wait now, and what became of the
/// others since the project opened, for Dev > Debug and the status
/// monitor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AudioWaitCounts {
    /// Waiting now, not held yet.
    pub waiting: u32,
    /// Kept back for their audio.
    pub kept: u64,
    /// Held in time to go into the view.
    pub held: u64,
    /// Moved to a later line, the line played without them.
    pub moved: u64,
    /// Cancelled after their last move, or at Now after two bars.
    pub cancelled: u64,
    /// Held, then given back before they landed.
    pub given_back: u64,
    /// Rises whenever the audio the waiting cues need changes, so the
    /// app asks for it only then.
    pub needs_rev: u64,
}

/// Mute and solo changes waiting for one launch line (sequencer S4's mute
/// queue): they land together on the line they had when the first one
/// joined.
#[derive(Clone, Debug, PartialEq)]
pub struct MuteWait {
    pub line: LaunchGrid,
    /// Each track that changes: its mute and solo once the queue lands.
    pub want: Vec<(usize, bool, bool)>,
    /// Each pad that changes on a track (SHIFT + TRACK MUTE; the
    /// instrument redesign's step 10): the track, the kit pad (A1 = 0),
    /// and its mute and solo there once the queue lands.
    pub pads: Vec<(usize, usize, bool, bool)>,
}

/// A kit pad's name as Akai's MPC gives it: "A1" for pad 0, "B5" for pad
/// 20, "H16" for pad 127.
pub fn pad_name(pad: usize) -> String {
    let bank = char::from(b'A' + (pad / 16).min(7) as u8);
    format!("{bank}{}", pad % 16 + 1)
}

impl MuteWait {
    /// A queue for line `line` with nothing in it yet.
    pub fn new(line: LaunchGrid) -> MuteWait {
        MuteWait {
            line,
            want: Vec::new(),
            pads: Vec::new(),
        }
    }

    /// Nothing waits in it: no track and no pad.
    pub fn is_empty(&self) -> bool {
        self.want.is_empty() && self.pads.is_empty()
    }
}

impl TimeState {
    /// What waits on track `t`'s header, as the grid shows it: "mutes in
    /// 2 beats", "plays in 1 bar", "solo in 3 beats", "solo off in 1
    /// beat". None with nothing waiting there.
    pub fn mute_waiting(&self, t: usize) -> Option<String> {
        let q = self.mute_wait.as_ref()?;
        let &(_, mute, solo) = q.want.iter().find(|w| w.0 == t)?;
        let tr = self.tracks.get(t)?;
        let what = if solo != tr.solo {
            if solo {
                "solo"
            } else {
                "solo off"
            }
        } else if mute {
            "mutes"
        } else {
            "plays"
        };
        let tp = &self.transport;
        let left = cue_tick(tp, q.line) - tp.position;
        Some(format!("{what} in {}", countdown(left, tp.meter)))
    }

    /// The transport bar's WAITING reading: the tracks and the tracks'
    /// pads waiting and where they land ("Hats, Vox at 18.1", "Drums B5
    /// at 9.1"). None with nothing waiting.
    pub fn waiting_reading(&self) -> Option<String> {
        let q = self.mute_wait.as_ref().filter(|q| !q.is_empty())?;
        let tracks = q.want.iter().map(|w| self.track_name(w.0));
        let pads = q
            .pads
            .iter()
            .map(|w| format!("{} {}", self.track_name(w.0), pad_name(w.1)));
        let names: Vec<String> = tracks.chain(pads).collect();
        let shown = if names.len() > 2 {
            format!("{}, {} +{}", names[0], names[1], names.len() - 2)
        } else {
            names.join(", ")
        };
        let at = cue_tick(&self.transport, q.line);
        Some(format!("{shown} at {}", bar_beat(at, self.transport.meter)))
    }
}

/// The grid's tracks for `st`.
/// Scene banks (the Arrange refactor's 5.6): for each bank of
/// [`SLOTS`] rows, A to D, whether one of its rows has a clip or a named
/// scene, as the bank chip shows them.
pub fn banks_used(st: &TimeState) -> Vec<bool> {
    let mut used = vec![false; ROWS / SLOTS];
    for t in &st.tracks {
        for (row, c) in t.slots.iter().enumerate().take(ROWS) {
            if c.is_some() {
                used[row / SLOTS] = true;
            }
        }
    }
    for (row, name) in st.scenes.iter().enumerate().take(ROWS) {
        if !name.is_empty() {
            used[row / SLOTS] = true;
        }
    }
    used
}

pub fn grid_tracks(st: &TimeState) -> Vec<GridTrack> {
    st.tracks
        .iter()
        .enumerate()
        .map(|(i, tr)| GridTrack {
            name: tr.name.clone(),
            sound: tr.sound.clone(),
            color: track_color(i),
            // An audio track's own arm (its arm dot on the Song view).
            armed: match st.audio_lane_at(i) {
                Some(l) => l.armed,
                None => !tr.audio && st.armed == Some(i),
            },
            mute: tr.silent,
            solo: tr.solo,
            waiting: st.mute_waiting(i),
            loops: tr.loops.is_some(),
            audio: tr.audio,
            song: st.song_mark(i),
            cells: (0..tr.slots.len()).map(|s| cell_of(st, i, s)).collect(),
        })
        .collect()
}

/// The record settings' rows.
pub fn record_rows(st: &TimeState) -> Vec<OptionRow> {
    let t = &st.transport;
    let row = |name: &str, value: String| OptionRow {
        name: name.to_string(),
        value,
        ..OptionRow::default()
    };
    // An armed empty audio cell names its place (phase 4, board 2).
    let armed = match loop_cell_name(st).filter(|_| st.loops_on && st.loop_cell_audio()) {
        Some(place) => place,
        None => st
            .armed
            .map_or_else(|| "none".to_string(), |a| st.track_name(a)),
    };
    let count_in = match t.count_in {
        0 => "off".to_string(),
        1 => "1 bar".to_string(),
        n => format!("{n} bars"),
    };
    let rows = vec![
        row(RECORD_ROWS[0], armed),
        row(
            RECORD_ROWS[1],
            if t.overdub { "Overdub" } else { "Replace" }.to_string(),
        ),
        row(RECORD_ROWS[2], count_in),
        row(RECORD_ROWS[3], loop_row(t)),
        row(RECORD_ROWS[4], format!("{:.1}", t.bpm)),
        row(
            RECORD_ROWS[5],
            match st.armed.and_then(|a| st.tracks.get(a)) {
                None => "–".to_string(),
                Some(tr) if tr.pressure => "Recorded".to_string(),
                Some(_) => "Off".to_string(),
            },
        ),
        row(
            RECORD_ROWS[6],
            match st.armed.and_then(|a| st.tracks.get(a)) {
                None => "–".to_string(),
                Some(tr) => tr
                    .record_quantise
                    .map_or_else(|| "Off".to_string(), grid_name),
            },
        ),
        row(RECORD_ROWS[7], launch_name(st.launch)),
        row(
            RECORD_ROWS[TRANSPOSE_ROW],
            match st.armed.and_then(|a| st.tracks.get(a)) {
                Some(tr) if tr.melodic => transpose_text(tr.transpose),
                _ => "–".to_string(),
            },
        ),
        row(
            RECORD_ROWS[FOLLOWS_ROW],
            match st.armed.and_then(|a| st.tracks.get(a)) {
                Some(tr) if tr.melodic => if tr.follows { "On" } else { "Off" }.to_string(),
                _ => "–".to_string(),
            },
        ),
        row(RECORD_ROWS[MAKES_ROW], makes(st).to_string()),
    ];
    record_order(st)
        .iter()
        .filter_map(|&k| rows.get(k).cloned())
        .collect()
}

/// What REC makes now, for the Record tab's Makes row (phase 4, board 2):
/// a loop in the armed empty audio cell; takes on the song, song mode on,
/// for armed audio tracks; else the armed track's notes, or a loop on a
/// Loop track.
pub fn makes(st: &TimeState) -> &'static str {
    if st.loop_cell_audio() {
        return "a loop";
    }
    if st.song_on && st.audio.iter().any(|l| l.armed) {
        return "takes";
    }
    match st.armed.and_then(|a| st.tracks.get(a)) {
        Some(tr) if tr.loops.is_some() => "a loop",
        Some(tr) if !tr.audio => "notes",
        _ => "–",
    }
}

/// A track's live transpose as the screens say it: "as written", "+5 st",
/// "-2 st", "+3 in key" (as 16 Levels' Tune says "-8 st").
pub fn transpose_text(t: Transpose) -> String {
    match (t.by, t.in_key) {
        (0, _) => "as written".to_string(),
        (n, false) => format!("{n:+} st"),
        (n, true) => format!("{n:+} in key"),
    }
}

/// The Record tab's Transpose (– / + a semitone, or a note of the key when
/// the track moves in key, within two octaves) and Follows (either way
/// flips it) for the armed track; why not when it can't.
fn transpose_step(st: &TimeState, index: usize, delta: i32) -> Result<TimeAction, String> {
    let Some(a) = st.armed else {
        return Err("Arm a track first: transpose moves the armed track".to_string());
    };
    let tr = st.tracks.get(a).ok_or_else(String::new)?;
    if !tr.melodic {
        return Err(format!(
            "{} plays a kit, whose notes are its pads: transpose moves a track that plays notes",
            tr.name
        ));
    }
    if index == FOLLOWS_ROW {
        return Ok(TimeAction::FollowsTranspose {
            track: a,
            on: !tr.follows,
        });
    }
    let now = tr.transpose;
    let by = (i32::from(now.by) + delta.signum())
        .clamp(-i32::from(MAX_TRANSPOSE), i32::from(MAX_TRANSPOSE)) as i8;
    if by == now.by {
        let unit = if now.in_key {
            "notes of the key"
        } else {
            "semitones"
        };
        return Err(format!(
            "Transpose is at its {}, {MAX_TRANSPOSE} {unit} {}",
            if delta < 0 { "lowest" } else { "highest" },
            if delta < 0 { "down" } else { "up" }
        ));
    }
    Ok(TimeAction::Transpose {
        track: a,
        to: Transpose { by, ..now },
    })
}

/// A launch quantise as the screens say it: "at once", "1/16", "1 bar",
/// "4 bars".
pub fn launch_name(g: LaunchGrid) -> String {
    match g {
        LaunchGrid::Now => "at once".to_string(),
        LaunchGrid::Ticks(t) => grid_name(t),
        LaunchGrid::Bars(1) => "1 bar".to_string(),
        LaunchGrid::Bars(n) => format!("{n} bars"),
        LaunchGrid::At(_) => "its end".to_string(),
    }
}

/// The takes newest first: the undone ones (dimmed), then the ones Undo
/// can take back; and which row Undo takes back next.
/// The note editor's Continue tool is open (composing tools, phase 1 step
/// 2): the takes area holds its tries.
pub fn continuing_open(st: &TimeState) -> bool {
    st.edit.as_ref().is_some_and(|e| e.editor.continuing_open())
}

/// Continue's result for the clip the note editor shows, while its tool
/// is open.
fn continuing_for<'a>(e: &EditInfo, st: &'a TimeState) -> Option<&'a Continuing> {
    let c = st.continuing.as_deref()?;
    (e.editor.continuing_open() && (c.track, c.slot) == (e.editor.track, e.editor.slot))
        .then_some(c)
}

/// The tries while Continue is open (composing tools, phase 1 step 2), as
/// the takes area lists them: newest first, the one heard lit, each with
/// its style and the bars it adds; ‹ and › on Undo's and Redo's places.
pub fn try_list(st: &TimeState) -> (Vec<TakeItem>, Option<usize>, Option<String>, Option<String>) {
    let Some(c) = st.continuing.as_deref() else {
        return (Vec::new(), None, None, None);
    };
    let n = c.tries.len();
    let items = c
        .tries
        .iter()
        .enumerate()
        .rev()
        .map(|(k, t)| TakeItem {
            // An aimed try is named for its aim (S5): "Try 3 · calmer".
            title: match t.again.and_then(|a| a.aim()) {
                Some(aim) => format!("Try {} · {}", k + 1, aim.try_words()),
                None => format!("Try {} · {}", k + 1, t.style),
            },
            detail: match t.again {
                Some(Again::Aimed { bars: 0, .. }) | None => {
                    format!("+{} bar{}", t.bars, if t.bars == 1 { "" } else { "s" })
                }
                Some(a) => a.short(),
            },
            color: None,
            undone: false,
        })
        .collect();
    let (current, back, on) = match c.position {
        Some((i, m)) if m == n && i >= 1 => (
            Some(n - i),
            (i > 1).then(|| format!("{}", i - 1)),
            (i < n).then(|| format!("{}", i + 1)),
        ),
        _ => (None, None, None),
    };
    (items, current, back, on)
}

pub fn take_items(st: &TimeState) -> (Vec<TakeItem>, Option<usize>) {
    let item = |r: &TakeRow, undone: bool| TakeItem {
        title: take_title(st, r),
        detail: take_detail(st, r),
        color: r.at.map(|(track, _)| track_color(track)),
        undone,
    };
    let mut items: Vec<TakeItem> = st.undone.iter().map(|r| item(r, true)).collect();
    items.extend(st.done.iter().rev().map(|r| item(r, false)));
    let current = (!st.done.is_empty()).then_some(st.undone.len());
    (items, current)
}

/// "0:42": seconds, to the nearest, as minutes and seconds.
pub fn min_sec(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// The Song view for `st`, from bar `first_bar` (0-based): 16 bars. The
/// waveforms are left out: [`TimePage::update`] copies `st.waves` only
/// when they change, as they can be large.
pub fn song_lanes(st: &TimeState, first_bar: i64) -> SongLanes {
    song_lanes_with(st, first_bar, 16, None)
}

/// [`song_lanes`], with `blocks` for the blocks when they were made from
/// `st.placements` ([`SongBlocks::made_from`]), so an update that changes
/// no block copies none (Arrange refactor 2.4).
fn song_lanes_with(
    st: &TimeState,
    first_bar: i64,
    bars_shown: i64,
    blocks: Option<SongBlocks>,
) -> SongLanes {
    let t = &st.transport;
    let bar = t.meter.bar_ticks();
    let blocks = blocks.unwrap_or_else(|| SongBlocks::of(&st.placements));
    let placed = std::iter::once(blocks.end());
    let audio = st
        .audio
        .iter()
        .flat_map(|l| l.blocks.iter().map(|b| b.end).chain(l.recording));
    let end = placed.chain(audio).max().unwrap_or(0);
    let bars = (end + bar - 1) / bar.max(1);
    let mut legend = if bars == 0 {
        "empty: Scenes → song makes one from the rows".to_string()
    } else {
        format!("{bars} bars · from bar {}", first_bar + 1)
    };
    if let Some((free, all)) = st.audio_free {
        legend += &format!(" · Audio {} of {} free", min_sec(free), min_sec(all));
    }
    for (what, n) in st.audio_counts.iter().filter(|c| c.1 > 0) {
        legend += &format!(" · {n} {what}");
    }
    let alert = st.audio_counts.iter().any(|c| c.1 > 0);
    SongLanes {
        title: String::new(),
        legend,
        alert,
        tracks: st
            .tracks
            .iter()
            .enumerate()
            .map(|(i, tr)| (tr.name.clone(), track_color(i)))
            .collect(),
        sounds: st.tracks.iter().map(|tr| tr.sound.clone()).collect(),
        blocks,
        markers: st.markers.clone(),
        // On a linked song (4b.4).
        passes: match st.list_linked {
            true => st.passes.clone(),
            false => Vec::new(),
        },
        bar,
        beat: t.meter.beat_ticks(),
        first_bar,
        bars: bars_shown,
        playhead: Some(t.position),
        selected: st.selected,
        picks: st.picks.clone(),
        section: st.section,
        on: st.song_on,
        rec: st.song_rec,
        audio: st.audio.clone(),
        loop_waves: loop_block_waves(st),
        region: st.region.map(|r| (r.track, r.index)),
        panel: st.audio_panel,
        takes_open: st.audio_takes,
        can_add_track: st.tracks.len() < MAX_TRACKS,
        tempo: st.tempo_points.clone(),
        tempo_base: st.tempo_base,
        tempo_pick: st.tempo_pick.filter(|&k| k < st.tempo_points.len()),
        chords: song_chord_lane(st),
        report: st
            .report
            .clone()
            .map(|r| ReportCard::new(r, &report_tracks(st))),
        over: st.lanes_over(),
        ..SongLanes::default()
    }
}

/// Row `row`'s chords as its clips alone say (the picker's As heard): the
/// strip's row, else the Song view's.
fn heard_line(st: &TimeState, row: usize) -> Option<mpfree_context::chord::ChordTimeline> {
    st.row_chords
        .as_ref()
        .filter(|(r, _)| *r == row)
        .or_else(|| st.song_chords.iter().find(|(r, _)| *r == row))
        .map(|(_, c)| c.heard.clone())
}

/// The Song view's Chords lane (composing step 6, board C): for each
/// section made from a scene, the scene's chords as heard
/// ([`crate::add_view::shown_line`]), laid end to end from the section's
/// start and cut at its end. The picker is the page's.
pub fn song_chord_lane(st: &TimeState) -> crate::song::SongChordLane {
    use crate::chord_strip::ChipLook;
    use mpfree_context::chord::{ChordAnswer, Heard};
    let mut chips = Vec::new();
    let mut missing = false;
    for &(start, end, row) in &st.scene_sections {
        let Some(line) = shown_line(st, row) else {
            missing = true;
            continue;
        };
        let len = line.len();
        if len <= 0 {
            continue;
        }
        let mut at = start;
        while at < end && chips.len() < crate::song::CHORD_CHIPS {
            for (k, s) in line.spans().iter().enumerate() {
                let from = at + s.start;
                if from >= end {
                    break;
                }
                let (name, root) = match s.answer {
                    ChordAnswer::Chord(c) => (
                        c.name(),
                        mpfree_context::key::PC_NAMES[usize::from(c.root) % 12].to_string(),
                    ),
                    ChordAnswer::NoClearChord => ("–".to_string(), "–".to_string()),
                };
                let look = match s.heard {
                    Heard::Set => ChipLook::Yours,
                    Heard::Suggested => ChipLook::Suggested,
                    Heard::Heard if s.answer.chord().is_none() => ChipLook::Suggested,
                    Heard::Heard => ChipLook::Heard,
                };
                chips.push(crate::song::SongChip {
                    from,
                    to: (at + s.end()).min(end),
                    name,
                    root,
                    look,
                    row,
                    chip: k,
                });
            }
            at += len;
        }
    }
    crate::song::SongChordLane {
        hearing: missing && chips.is_empty(),
        chips,
        open: None,
        picker: None,
    }
}

/// The small waves of the loops that blocks on the audio lanes play
/// (phase 4e.3), by lasting id: only those, so a loop recording elsewhere
/// never draws the Song view again.
fn loop_block_waves(st: &TimeState) -> Vec<(u128, [u8; crate::widgets::CELL_WAVE])> {
    let named = |id: u128| {
        st.audio
            .iter()
            .any(|l| l.clips.iter().any(|c| c.looped == Some(id)))
    };
    st.loops
        .ids
        .iter()
        .zip(&st.loops.loops)
        .filter(|(&id, _)| id != 0 && named(id))
        .map(|(&id, f)| (id, f.wave))
        .collect()
}

/// Each track's name and colour, for the conversion report card's rows.
fn report_tracks(st: &TimeState) -> Vec<(String, Color)> {
    st.tracks
        .iter()
        .enumerate()
        .map(|(i, tr)| (tr.name.clone(), track_color(i)))
        .collect()
}

/// Where the note editor's clip plays now, in its own ticks, if it is
/// launched and the transport has started.
pub fn note_playhead(e: &EditInfo, st: &TimeState) -> Option<Tick> {
    let ed = &e.editor;
    let c = &e.clip;
    st.launches
        .iter()
        .find(|l| (l.track, l.slot) == (ed.track, ed.slot))
        .filter(|_| c.length > 0 && st.transport.position >= 0)
        .map(|l| {
            let at = c.speed.to_clip((st.transport.position - l.origin) as f64);
            (at.floor() as Tick).rem_euclid(c.length)
        })
}

/// The screen built the note editor from `a` and is asked to show `b`:
/// nothing but the playhead can have changed. The same clip (one shared
/// copy, not compared note by note), revision and editor.
/// Continue shows the same on the roll: the same result (by its place in
/// memory: a result is never changed, only replaced) and the same state.
fn same_continuing(a: Option<&Continuing>, b: Option<&Continuing>) -> bool {
    let same_arc = |x: &Option<Arc<_>>, y: &Option<Arc<_>>| match (x, y) {
        (Some(x), Some(y)) => Arc::ptr_eq(x, y),
        (None, None) => true,
        _ => false,
    };
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            same_arc(&a.clip, &b.clip)
                && a.made.as_ref().map(Arc::as_ptr) == b.made.as_ref().map(Arc::as_ptr)
                && (
                    a.track, a.slot, a.state, a.new_from, a.position, a.heard, a.line,
                ) == (
                    b.track, b.slot, b.state, b.new_from, b.position, b.heard, b.line,
                )
                && a.words == b.words
        }
        _ => false,
    }
}

fn same_edit(a: &EditInfo, b: &EditInfo) -> bool {
    Arc::ptr_eq(&a.clip, &b.clip)
        && a.rev == b.rev
        && a.editor == b.editor
        && a.bar == b.bar
        && a.track_name == b.track_name
        && (Arc::ptr_eq(&a.locks, &b.locks) || a.locks == b.locks)
        && a.ghosts.len() == b.ghosts.len()
        && a.ghosts.iter().zip(&b.ghosts).all(|(x, y)| {
            (x.track, x.slot) == (y.track, y.slot)
                && Arc::ptr_eq(&x.clip, &y.clip)
                && x.track_name == y.track_name
        })
}

/// Clip `other` plays with the clip edited, `edited` (piano roll plan
/// 4.1): on another track, in the same scene (slot row) or playing now.
fn plays_with(st: &TimeState, edited: (usize, usize), other: (usize, usize)) -> bool {
    other.0 != edited.0
        && (other.1 == edited.1 || st.launches.iter().any(|l| (l.track, l.slot) == other))
}

/// The clips the note editor's Show menu offers (piano roll plan 4.1):
/// every other clip of the edited clip's kind with notes, on a track
/// that plays notes; those that play with it first, then the rest, each
/// in track and slot order; the ones it shows marked.
pub fn show_offers(e: &EditInfo, st: &TimeState) -> Vec<ShowOffer> {
    let edited = (e.editor.track, e.editor.slot);
    let mut v = Vec::new();
    for (t, tr) in st.tracks.iter().enumerate() {
        if tr.loops.is_some() || tr.audio {
            continue;
        }
        for (s, c) in tr.slots.iter().enumerate() {
            let Some(c) = c else {
                continue;
            };
            if (t, s) == edited || c.looped.is_some() || c.notes == 0 || c.holds != e.clip.holds {
                continue;
            }
            v.push(ShowOffer {
                track: t,
                slot: s,
                name: format!("{} · {}", tr.name, c.name),
                with: plays_with(st, edited, (t, s)),
                shown: e.editor.show.contains(&(t, s)),
            });
        }
    }
    v.sort_by_key(|o| !o.with);
    v
}

/// The other clips the note editor shows faintly behind (piano roll plan
/// 4.1), each placed where it sounds against the edited clip
/// ([`mpfree_seq::notes::placed`]): against the pass of it playing now,
/// from where the other's loop is then (its own launch, or as if launched
/// with it), or with both from their tops while the edited clip doesn't
/// play.
pub fn ghost_layers(e: &EditInfo, st: &TimeState) -> Vec<GhostLayer> {
    let edited = (e.editor.track, e.editor.slot);
    ghost_phases(e, st)
        .into_iter()
        .zip(&e.ghosts)
        .map(|(phase, g)| GhostLayer {
            track: g.track,
            slot: g.slot,
            name: format!("{} · {}", g.track_name, g.clip.name),
            with: plays_with(st, edited, (g.track, g.slot)),
            color: track_color(g.track),
            phase,
            notes: mpfree_seq::notes::placed(&g.clip, &e.clip, phase, GHOST_MOST),
        })
        .collect()
}

/// [`ghost_layers`]' phases: how far into its loop, in song ticks, each
/// other clip is as the edited clip's pass starts.
fn ghost_phases(e: &EditInfo, st: &TimeState) -> Vec<f64> {
    use mpfree_seq::notes::loop_phase;
    let period = |c: &mpfree_seq::Clip| c.speed.to_song(c.length as f64);
    let launch = |t: usize, s: usize| st.launches.iter().find(|l| (l.track, l.slot) == (t, s));
    let me = launch(e.editor.track, e.editor.slot);
    let pos = st.transport.position as f64;
    // The edited clip's origin, and where its pass playing now started.
    let pass = me.map(|l| {
        let origin = l.origin as f64;
        (origin, pos - loop_phase(origin, period(&e.clip), pos))
    });
    e.ghosts
        .iter()
        .map(|g| match (pass, launch(g.track, g.slot)) {
            (Some((_, at)), Some(l)) => loop_phase(l.origin as f64, period(&g.clip), at),
            (Some((origin, at)), None) => loop_phase(origin, period(&g.clip), at),
            (None, _) => 0.0,
        })
        .collect()
}

/// The note editor's roll or list for `e`, with the playhead where the
/// clip plays now (if it is launched).
pub fn note_grid(e: &EditInfo, st: &TimeState) -> NoteGrid {
    let ed = &e.editor;
    // A tool open (plan 2B.3): the roll shows its preview, the notes it
    // makes picked, as Done will leave them; the clip itself is as it was.
    let preview = ed.preview(&e.clip);
    // Continue open (composing tools, phase 1 step 2): the clip as heard
    // with its result, the new bars faint, nothing picked.
    let cont = continuing_for(e, st);
    let (c, sel): (&mpfree_seq::Clip, &[usize]) =
        match (&preview, cont.and_then(|c| c.clip.as_deref())) {
            (_, Some(heard)) => (heard, &[]),
            (Some((p, s)), None) => (p, s),
            (None, None) => (&e.clip, &ed.sel),
        };
    let playhead = note_playhead(e, st);
    let selected = |i: usize| sel.contains(&i);
    let n_sel = sel.iter().filter(|&&i| i < c.notes.len()).count();
    let mut g = NoteGrid::new();
    g.title = format!("Edit · {} · {}", c.name, e.track_name);
    let step = ed.grid().size();
    let lens = match ed.lens {
        Lens::Grid => "grid".to_string(),
        Lens::List => "list".to_string(),
        // The snap off or too fine: the steps are 1/16s, and it says so.
        Lens::Tracker if step == STEP_FALLBACK && ed.snap_ticks() != step => {
            format!(
                "tracker · {} steps (snap {})",
                step_name(step),
                ed.snap_name()
            )
        }
        Lens::Tracker => format!("tracker · {} steps", step_name(step)),
    };
    g.legend = if ed.cleared {
        // Something else changed the clip and took a picked note with it.
        format!(
            "{} notes · the clip changed: picks cleared · {lens}",
            c.notes.len()
        )
    } else {
        format!("{} notes · {} picked · {lens}", c.notes.len(), n_sel)
    };
    // Each note with its pressure: only the runs the notes name (the
    // clip keeps the runs of notes taken out too, for undo).
    g.notes = c
        .notes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let mut b = NoteBox::of(n, selected(i));
            let run = c.touch_of(n);
            if !run.is_empty() {
                b.touch = mpfree_seq::Touch {
                    first: g.touch.len() as u32,
                    len: run.len() as u32,
                };
                g.touch.extend_from_slice(run);
            }
            b
        })
        .collect();
    g.step = step;
    g.locks = c.locks.iter().map(LockMark::of).collect();
    // The clip's Steps step size: where its locks sit, and what its end
    // handle snaps to and stops at (plan 1A.2).
    let steps = mpfree_seq::notes::steps_grid(c);
    g.lock_step = steps.size();
    g.shortest = mpfree_seq::notes::shortest_steps(c, steps) as Tick * steps.size();
    g.longest = steps.max_steps() as Tick * steps.size();
    g.lock_settings = e.locks.clone();
    // Continue's lanes (composing tools S3): each a lane of its own.
    g.sound_lanes = Arc::new(crate::editor::SoundLane::of(c));
    g.length = c.length;
    g.bar = e.bar;
    g.pads = pads(c);
    g.playhead = playhead;
    // The cursor shows on the roll and the tracker (the list has no
    // time line).
    g.cursor = (ed.lens != Lens::List).then_some(ed.cursor);
    g.snap = ed.snap_ticks();
    g.snap_name = ed.snap_name();
    g.snap_auto = ed.snap_auto();
    g.tool = ed.tool;
    g.add = ed.add;
    g.lens = ed.lens;
    g.first = ed.first;
    g.color = track_color(ed.track);
    // The clip's scale (plan 2B.1): none shows on pads, nor on a clip of
    // pad hits, which takes none.
    g.scale = lit_scale(c);
    g.scale_name = match c.takes_scale() {
        true => ed.offers.label(c.scale),
        false => "none on pads".into(),
    };
    g.offers = Arc::clone(&ed.offers);
    // What a drawn note puts in (plan 2B.2).
    g.chord = ed.chord;
    g.minor = ed.minor;
    g.pads_write = ed.pads_write;
    // Other clips behind it, and what Show offers (plan 4.1).
    g.ghosts = ghost_layers(e, st);
    g.show_offers = show_offers(e, st);
    // An empty clip: its card, with Add a part › (composing tools S2.6).
    if e.clip.notes.is_empty() && cont.is_none() {
        g.empty_card = Some(empty_card(st, e.editor.track, e.editor.slot));
    }
    if let Some(k) = cont {
        let made = k.clip.as_ref().and(k.made.as_deref());
        g.new_from = made.map(|_| k.new_from);
        // Picks and locks (step 3): the new notes picked outlined, the
        // bars picked and locked marked.
        let picks = &ed.continue_picks;
        for n in &mut g.notes {
            n.selected = n.tick >= k.new_from && picks.notes.contains(&n.tick);
        }
        g.picked_bars = picks.bars.clone();
        g.locked_bars = picks.locked.clone();
        let bar = e.bar.max(1);
        g.bar_jobs = made.map_or_else(Vec::new, |m| {
            m.plan
                .iter()
                .enumerate()
                .map(|(b, p)| (b as Tick * bar, p.job.words()))
                .filter(|&(at, _)| at >= k.new_from)
                .collect()
        });
        g.chords = made.map_or_else(Vec::new, |m| {
            m.chords
                .spans()
                .iter()
                .map(|s| ChordMark {
                    start: s.start,
                    end: s.end(),
                    name: s.answer.name(),
                    suggested: s.heard == mpfree_context::chord::Heard::Suggested,
                })
                .collect()
        });
    }
    let set = &ed.continuing;
    let character = set.character;
    let continuing = |t: &crate::editor::NoteTool| t.kind == ToolKind::Continue;
    g.tool_bar = ed.open_tool.map(|t| ToolBar {
        name: t.kind.name(),
        // Continue's Character, when one is picked (S5).
        more: (continuing(&t) && character != mpfree_compose::Character::AsStyle)
            .then(|| character.words()),
        hint: match (t.kind, preview) {
            // Its row picked: what the Character does (S5 board A).
            (ToolKind::Continue, _)
                if set.row == crate::editor::compose::CHARACTER_ROW
                    && character != mpfree_compose::Character::AsStyle =>
            {
                capital(character.says())
            }
            (ToolKind::Continue, _) => continue_hint(st.continuing.as_deref(), e.bar.max(1)),
            (_, Some(_)) => "Turn the Q-Links: the roll shows the result".into(),
            (_, None) => {
                let why = t.why_not(&e.clip, &ed.sel);
                let mut c = why.chars();
                c.next()
                    .map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
            }
        },
        // Continue's Try again says what it works on (step 3).
        option: match t.kind {
            ToolKind::Continue => Some((ed.continue_picks.try_label(), false)),
            _ => t.option().map(|(w, lit)| (w.to_string(), lit)),
        },
        // Its ▾ (S5): faint while notes are picked.
        aim: continuing(&t).then(|| {
            ed.continue_picks
                .aimed(mpfree_compose::Aim::Busier)
                .is_some()
        }),
    });
    g
}

/// `s` with its first letter a capital.
fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

// ---------------------------------------------------------------- the page

/// What Continue's bar says (composing tools, phase 1 step 2): what it is
/// doing, which try is heard and from when, or why nothing can be.
/// What Continue is doing, in words, for the screens that show it (the
/// note editor's tool bar, and Continue on Steps' panel).
pub fn continue_hint(c: Option<&Continuing>, bar: Tick) -> String {
    use mpfree_compose::State;
    let Some(c) = c else {
        return "Making the new bars".into();
    };
    if let Some(w) = &c.words {
        return w.clone();
    }
    let at = |l: Tick| format!("bar {}", l / bar + 1);
    let which = c
        .position
        .map_or_else(String::new, |(i, n)| format!("Try {i} of {n}"));
    // The try heard was aimed (S5): what it changed.
    let aimed = c
        .position
        .and_then(|(i, _)| Some((i, c.tries.get(i.checked_sub(1)?)?.again?.aim()?)));
    if let (Some((i, aim)), State::Ready | State::Auditioning, true, None) =
        (aimed, c.state, c.heard, c.line)
    {
        return format!(
            "Try {i}, {}: {} · Done keeps it",
            aim.try_words(),
            aim.says()
        );
    }
    match (c.state, c.heard, c.line) {
        (State::Preparing, _, _) if c.made.is_none() => "Making the new bars".into(),
        (State::Preparing, _, _) => format!("{which} plays on while the next is made"),
        (_, false, Some(l)) => format!("{which} comes in at {}", at(l)),
        (_, true, Some(l)) => format!("{which}: the next comes in at {}", at(l)),
        // S3: ", sound Subtle" while the try moves a knob.
        (_, true, None) => match c.sound {
            Some(l) => format!(
                "{which} plays in place of the clip, sound {}: Done keeps it",
                l.name()
            ),
            None => format!("{which} plays in place of the clip: Done keeps it"),
        },
        _ => format!("{which} is ready: press PLAY to hear it, Done keeps it"),
    }
}

/// The Clips page and the transport bar in the app's [`Ui`]: the app adds
/// them once, calls [`TimePage::update`] when the state changes, and turns
/// touches into actions with [`action`].
#[derive(Clone, Debug, Default)]
pub struct TimePage {
    /// The first slot shown (the wheel scrolls it).
    pub first_row: usize,
    /// The first track shown: a multiple of [`GRID_COLS`], paged by the
    /// pager and following the armed track.
    pub first_track: usize,
    /// The first bar the Song view shows (0-based).
    pub first_bar: i64,
    /// How many bars the Song view shows (the zoom chip, 6.4): one of
    /// `song::ZOOMS`, 0 for the 16 a new page starts with. Kept while
    /// the app runs, whichever view shows; not saved with the project.
    pub song_bars: i64,
    /// The Perform pads' clip window, while one is on the pads (the
    /// Arrange refactor's 5.5): its first track and row, outlined on the
    /// grid. The app sets it.
    pub pad_window: Option<(usize, usize)>,
    /// The armed track last shown, so a newly armed track off the page
    /// brings its page.
    armed_seen: Option<usize>,
    /// The grid's rows in view, as of the last update.
    rows_seen: usize,
    /// The Record panel shows its Clip tab: the clip settings card.
    pub clip_card: bool,
    /// The note editor shows on another page (the app's Sequencer tab):
    /// while it is open the Clips page keeps the grid, the song or the
    /// list, and the editor's layout comes back once this is false again.
    pub edit_hidden: bool,
    /// A set is open (live shows B1): the Clips grid's, the Song view's
    /// and the song list's title lines have Show. The app sets it.
    pub set_open: bool,
    /// The Show view is the view shown (it needs [`TimePage::set_open`]).
    /// The app sets it, and fills the view's widget itself.
    pub show: bool,
    /// The "+ Track" chooser is open (layout spec 4c): on the grid under
    /// its "+ Track", on the Song view under the "+ Track" lane.
    pub adding: bool,
    /// Link's chooser is open on a song list that isn't linked (4b.5).
    pub linking: bool,
    /// The scene card's row the wheel and - / + turn (a [`SCENE_ROWS`]
    /// index; 0, the Scene row, means Follow).
    pub scene_row: usize,
    /// A kind of track was picked in the chooser with this many tracks:
    /// once there are more, the new one is brought into view.
    reveal: Option<usize>,
    /// What the note editor's roll and options were last built from: an
    /// update with the same clip and editor moves only the playhead
    /// (piano roll plan 0C.1), so the playing transport costs the screen
    /// no rebuild of 8,192 notes a frame.
    edit_built: Option<Box<EditInfo>>,
    /// Continue as the roll was last built with it (composing tools,
    /// phase 1 step 2): a new result, or its state, builds it again.
    edit_continuing: Option<Arc<Continuing>>,
    /// The note editor's Edit options and takes show, in a drawer at the
    /// right ([`EDIT_DRAWER_LAYOUT`]): Edit, TC and the wheel open it.
    pub edit_drawer: bool,
    /// The roll's view for each clip (track, slot) it has shown: the
    /// screen's own, so the zoom and scroll are as you left them when you
    /// come back to a clip (piano roll plan 1A.1). Never saved.
    views: BTreeMap<(usize, usize), RollView>,
    /// Which rows the roll shows for each clip (plan 2B.1, the Fold
    /// chip): the screen's own, kept with the clip's view. Never saved.
    folds: BTreeMap<(usize, usize), Fold>,
    /// The lane under the roll: which, how tall, folded or not (plan
    /// 2A.1). The screen's own, the same for every clip; its chip, fold
    /// and top edge change it on the roll, which the page reads back.
    pub edit_lane: LaneShow,
    /// The note editor's Sound chip (plan 1C.2): the screen's own, the
    /// same for every clip; a tap steps it on the roll, which the page
    /// reads back.
    pub edit_hearing: Hearing,
    /// What the page asks of the time engine on its own (Snap Auto's
    /// division); the app sends them ([`TimePage::take_asks`]).
    asks: Vec<TimeAction>,
    /// Snap Auto's division last asked for, and for which clip, so it is
    /// asked once while the answer comes.
    auto_asked: Option<((usize, usize), usize)>,
    /// The cursor last seen and on which clip (plan 1C.1): when the wheel
    /// moves it, the roll scrolls to keep it on screen.
    cursor_seen: Option<((usize, usize), Tick)>,
    /// The kit's names for the pads of the clip the editor shows, and the
    /// bank the pads play (plan 4.2): the app knows them and tells them
    /// ([`TimePage::set_pads`]).
    pad_names: Arc<Vec<String>>,
    pad_bank: u8,
    /// Add a part's panel is open (composing tools S2.6, layout spec 4r):
    /// its choices, the screen's own until Back, Keep or ×.
    pub add: Option<AddPick>,
    /// The app's list of phrases, which Add a part's Style row runs on
    /// into after its styles (P2). The app sets it
    /// ([`TimePage::set_phrases`]).
    pub phrases: Arc<[(String, mpfree_arp::Phrase)]>,
    /// The Make a phrase sheet is open from the clip card (step 3 of the
    /// phrases): what it shows. The app sets it; the sheet is the arp's,
    /// and its touches go to the app.
    pub make: Option<SheetView>,
    /// Keep was tapped: the panel closes once the time engine has kept
    /// the part (a refusal leaves it open, saying why).
    keeping: bool,
    /// The panel closed because the part was kept, until the app takes
    /// it ([`TimePage::take_kept`]): New track's synth stays on the new
    /// track then; any other close lets it go.
    kept: bool,
    /// The Record panel shows its Chords tab.
    pub chords_card: bool,
    /// The chord picker, open on a chip of the row's chords.
    pub chord_pick: Option<ChordPick>,
    /// The Chords tab's clips left unticked, as (row, track): Fit leaves
    /// them as they are.
    untick: BTreeSet<(usize, usize)>,
    /// The row last asked to be heard for its chords.
    heard_asked: Option<usize>,
    /// The clip the clip settings card showed last: another starts at the
    /// card's top.
    card_focus: Option<(usize, usize)>,
    /// The Song view's Chords lane chip the chord picker is open on
    /// (composing step 6, board C): an index into its chips.
    pub song_chord: Option<usize>,
}

impl TimePage {
    /// How many bars the Song view shows (6.4): 16 until the zoom chip
    /// picks another.
    pub fn bars_shown(&self) -> i64 {
        if crate::song::ZOOMS.contains(&self.song_bars) {
            self.song_bars
        } else {
            16
        }
    }

    /// How many bars the Song view scrolls a step (the wheel, a drag held
    /// at an edge): one, up to 16 shown; a sixteenth of what shows past
    /// that (6.4).
    fn bars_step(&self) -> i64 {
        (self.bars_shown() / 16).max(1)
    }

    /// Add the bar, the grid, the record settings, the takes and the Clips
    /// page. The app's `header` and `hint` must be in `ui` already. The
    /// Q-Link column is the app's: an empty one is added only when `ui`
    /// has none yet (the samples, the tests), so the app's is never
    /// replaced.
    pub fn add_to(ui: &mut Ui) -> Result<TimePage, String> {
        if ui.get::<QLinkColumn>(QLINKS).is_none() {
            ui.add(QLINKS, QLinkColumn::new());
        }
        ui.add(TRANSPORT, TransportBar::new());
        ui.add(CLIPS, ClipGrid::new("Clips", ROWS));
        let mut record = OptionList::new("Record");
        record.tabs = ["Record", "Clip", "Chords"].map(str::to_string).to_vec();
        record.tab_widths = PANEL_TAB_WIDTHS.to_vec();
        let mut card = OptionList::new("Clip");
        card.tabs = record.tabs.clone();
        card.tab_widths = record.tab_widths.clone();
        card.tab = 1;
        card.right = "wheel or arrows".to_string();
        card.empty = "Arm a track with a clip".to_string();
        let mut chords = ChordsCard::new();
        chords.head.tabs = record.tabs.clone();
        chords.head.tab_widths = record.tab_widths.clone();
        ui.add(RECORD, record);
        ui.add(CLIP_CARD, card);
        ui.add(CHORDS_CARD, chords);
        ui.add(ADD_PANEL, AddPanel::new());
        ui.add(MAKE_SHEET, MakeSheet::new());
        ui.add(STYLE_SHEET, StyleSheetView::default());
        ui.add(TAKES, TakeList::new("Takes"));
        ui.add(MORPH, MorphBar::new());
        ui.add(SONG, SongLanes::new());
        ui.add(SONG_LIST, SongListView::new());
        ui.add(SHOW, crate::show::ShowView::new());
        ui.add(NOTES, NoteGrid::new());
        ui.add(EDIT_OPTIONS, OptionList::new("Edit"));
        ui.add_page(CLIPS_PAGE, CLIPS_LAYOUT)?;
        Ok(TimePage::default())
    }

    /// Rows of the grid shown at once (0 if the grid is not on screen).
    pub fn visible_rows(&self, ui: &Ui) -> usize {
        match (ui.rect_of(CLIPS), ui.get::<ClipGrid>(CLIPS)) {
            (Some(r), Some(g)) => g.visible_rows(r),
            _ => 0,
        }
    }

    /// A touch the page handles itself: the grid's pager (‹ and ›, or a
    /// swipe along the track names) shows the 8 tracks before or after; a
    /// dragged clip, row, block or section held at an edge scrolls.
    /// True when `ev` was one; call [`TimePage::update`] after.
    /// The "+ Track" chooser (layout spec 4c): a tap on "+ Track" opens it
    /// (or closes it), unless SHIFT is held (a Loop track at once: the
    /// app's); a touch off it closes it. True when the event was the
    /// page's own, nothing for the app. A row picked closes it and is the
    /// app's ([`action`]); the new track comes into view once it is
    /// there. Call [`TimePage::update`] after.
    pub fn chooser(&mut self, ev: &UiEvent, st: &TimeState, shift: bool) -> bool {
        // A row's name on the scene card picks it for the wheel (5.2).
        if let (CLIPS | SONG_LIST, EventKind::Toggle(k)) = (ev.widget.as_str(), &ev.kind) {
            let pick = crate::scene_card::SCENE_PICK;
            if st.scene_open.is_some() && (pick..pick + SCENE_ROWS.len()).contains(k) {
                self.scene_row = k - pick;
                return true;
            }
        }
        let (button, kind, off) = match (ev.widget.as_str(), &ev.kind) {
            (CLIPS, EventKind::Toggle(k)) => (
                *k == ROWS + ADD_TRACK,
                (ROWS + ADD_KIND..ROWS + ADD_KIND + 3).contains(k),
                *k == ROWS + ADD_DISMISS,
            ),
            (SONG, EventKind::Tab(k)) => (
                *k == crate::song::ADD_TRACK,
                [crate::song::ADD_NOTE, ADD_AUDIO, crate::song::ADD_LOOP].contains(k),
                *k == crate::song::ADD_CLOSE,
            ),
            // Link's chooser (4b.5): Link opens and closes it, a touch off
            // it closes it, and a choice closes it and is the app's.
            (SONG_LIST, EventKind::Toggle(k)) => {
                return match *k {
                    LIST_LINK => {
                        self.linking = !self.linking && !st.list_linked;
                        true
                    }
                    LIST_LINK_CLOSE => {
                        self.linking = false;
                        true
                    }
                    LIST_LINK_FROM_LIST | LIST_LINK_FROM_SONG => {
                        self.linking = false;
                        false
                    }
                    _ => false,
                };
            }
            _ => return false,
        };
        if button && !shift {
            self.adding = !self.adding;
            return true;
        }
        if button || kind {
            self.adding = false;
            self.reveal = Some(st.tracks.len());
            return false;
        }
        if off {
            self.adding = false;
            return true;
        }
        false
    }

    pub fn page_tracks(&mut self, ev: &UiEvent, st: &TimeState) -> bool {
        let step = match (ev.widget.as_str(), &ev.kind) {
            (SONG, EventKind::Tab(k)) if *k == BARS_BACK || *k == BARS_ON => {
                // A dragged block or section held at the left or right
                // edge: a bar a step, more zoomed out (6.4).
                let d = if *k == BARS_ON { 1 } else { -1 };
                self.first_bar = (self.first_bar + d * self.bars_step()).max(0);
                return true;
            }
            // The zoom chip (6.4): the Song view shows that many bars.
            (SONG, EventKind::Tab(k))
                if (crate::song::ZOOM..crate::song::ZOOM + crate::song::ZOOMS.len())
                    .contains(k) =>
            {
                self.song_bars = crate::song::ZOOMS[*k - crate::song::ZOOM];
                return true;
            }
            // The note editor's Edit button: the drawer.
            (NOTES, EventKind::Tab(EDIT_BUTTON)) => {
                self.edit_drawer = !self.edit_drawer;
                return true;
            }
            // The roll's view moved: the roll keeps it; the page keeps it
            // for the clip and lets Snap Auto follow ([`TimePage::update`]).
            (NOTES, EventKind::View { .. }) => return true,
            (RECORD | CLIP_CARD | CHORDS_CARD, EventKind::Tab(k)) => {
                // The Record panel's tabs: the record settings, the clip
                // settings card or the row's chords.
                self.clip_card = *k == 1;
                self.chords_card = *k == CHORDS_TAB;
                if !self.chords_card && self.add.is_none() {
                    self.chord_pick = None;
                }
                return true;
            }
            (CLIPS, EventKind::Toggle(k))
                if (ROWS + BANK..ROWS + BANK + ROWS / SLOTS).contains(k) =>
            {
                // The bank chip (5.6): the bank's first row at the top.
                let rows = self.rows_seen.max(1);
                let first = (*k - ROWS - BANK) * SLOTS;
                self.first_row = first.min(ROWS.saturating_sub(rows));
                return true;
            }
            (CLIPS, EventKind::Toggle(k)) if *k == ROWS + PAGE_BACK => -1,
            (CLIPS, EventKind::Toggle(k)) if *k == ROWS + PAGE_ON => 1,
            (CLIPS, EventKind::Toggle(k)) if *k == ROWS + ROWS_BACK || *k == ROWS + ROWS_ON => {
                // A dragged clip or row held at the top or bottom edge.
                let rows = self.rows_seen.max(1);
                let last = ROWS.saturating_sub(rows) as i64;
                let d = if *k == ROWS + ROWS_ON { 1 } else { -1 };
                self.first_row = (self.first_row as i64 + d).clamp(0, last) as usize;
                return true;
            }
            _ => return false,
        };
        let last = st.tracks.len().saturating_sub(1) / GRID_COLS * GRID_COLS;
        self.first_track =
            (self.first_track as i64 + step * GRID_COLS as i64).clamp(0, last as i64) as usize;
        true
    }

    /// Bring the grid's cell `slot` of `track` into view: its page of
    /// tracks, and its row if it is off the rows shown (Show in Clips on
    /// the Looper tab). Call [`TimePage::update`] after.
    pub fn show_cell(&mut self, track: usize, slot: usize) {
        self.first_track = track / GRID_COLS * GRID_COLS;
        let rows = self.rows_seen.max(1);
        if slot < self.first_row || slot >= self.first_row + rows {
            self.first_row = slot.min(ROWS.saturating_sub(rows));
        }
    }

    /// The pads of the clip the note editor shows (piano roll plan 4.2):
    /// the kit's name for each (pad 1 first, "" for none; empty for a
    /// track that plays no kit), shown on its pad rows, and the bank the
    /// pads play (0 = A to 7 = H). A new bank (PAD BANK) scrolls a roll of
    /// pad rows to its first pad; an empty pad clip opens there. Only a
    /// change touches the roll.
    pub fn set_pads(&mut self, ui: &mut Ui, names: &[Option<String>], bank: u8) {
        let same = names.len() == self.pad_names.len()
            && names
                .iter()
                .zip(self.pad_names.iter())
                .all(|(a, b)| a.as_deref().unwrap_or("") == b);
        if !same {
            self.pad_names = Arc::new(
                names
                    .iter()
                    .map(|n| n.clone().unwrap_or_default())
                    .collect(),
            );
            if let Some(g) = ui.get_mut::<NoteGrid>(NOTES) {
                g.pad_names = Arc::clone(&self.pad_names);
            }
        }
        let bank = bank.min(7);
        if bank != self.pad_bank {
            self.pad_bank = bank;
            let r = ui.rect_of(NOTES);
            if let (Some(r), Some(g)) = (r, ui.get_mut::<NoteGrid>(NOTES)) {
                // The page keeps the roll's view for its clip at the next
                // update.
                g.show_bank(r, bank);
            }
        }
    }

    /// What the page asks of the time engine on its own, for the app to
    /// send: Snap Auto's division when the roll's zoom changes it.
    pub fn take_asks(&mut self) -> Vec<TimeAction> {
        std::mem::take(&mut self.asks)
    }

    /// Time passing, `dt` seconds: a finger held on a note of the roll
    /// picks it (piano roll plan 1B.1, [`NoteGrid::tick`]), asked of the
    /// time engine through [`TimePage::take_asks`]. True when the roll
    /// changed and wants drawing.
    pub fn tick(&mut self, ui: &mut Ui, st: &TimeState, dt: f32) -> bool {
        if self.edit_hidden || st.edit.is_none() {
            return false;
        }
        // The clip's end handle held at the roll's edge (plan 1A.2): the
        // end goes on and the roll scrolls, drawn again.
        if ui.get::<NoteGrid>(NOTES).is_some_and(|g| g.end_at_edge()) {
            if let Some(g) = ui.get_mut::<NoteGrid>(NOTES) {
                g.grow_end(dt);
            }
            return true;
        }
        // Quietly: the hold's timer draws nothing (the pick comes back
        // with the clip), and `get_mut` every frame would redraw the roll
        // every frame.
        let Some(kind) = ui.get_mut_quiet::<NoteGrid>(NOTES).and_then(|g| g.tick(dt)) else {
            return false;
        };
        let ev = UiEvent {
            widget: NOTES.to_string(),
            kind,
        };
        self.asks.extend(action(&ev, st));
        true
    }

    /// The note editor shows with its drawer closed: open it (Edit, TC
    /// and the wheel do). True if it opened; call [`TimePage::update`]
    /// after.
    pub fn open_drawer(&mut self, ui: &Ui) -> bool {
        if ui.layout_of(CLIPS_PAGE) != Some(EDIT_LAYOUT) {
            return false;
        }
        self.edit_drawer = true;
        true
    }

    /// The note editor shows with its drawer closed (plan 1C.1: – / +
    /// nudge the picked notes and the wheel moves the cursor).
    pub fn drawer_closed(&self, ui: &Ui) -> bool {
        ui.layout_of(CLIPS_PAGE) == Some(EDIT_LAYOUT)
    }

    /// SHIFT + – / + on the note editor (plan 1C.1): the roll zooms in
    /// time, `d` steps in (out if negative). True if it moved; call
    /// [`TimePage::update`] after.
    pub fn zoom_time(&mut self, ui: &mut Ui, d: i32) -> bool {
        let Some(r) = ui.rect_of(NOTES) else {
            return false;
        };
        ui.get_mut::<NoteGrid>(NOTES)
            .is_some_and(|g| g.zoom_step(r, d))
    }

    /// Scroll the grid `delta` rows down (up if negative), or on the Song
    /// view the timeline `delta` bars on. In the note editor the roll
    /// scrolls its rows itself (its view is the screen's); the list and
    /// the tracker are the editor's: the action for the app.
    pub fn scroll(&mut self, ui: &mut Ui, delta: i32) -> Option<TimeAction> {
        // The conversion report card's rows, while it is open; nothing
        // under it moves.
        if let Some(name) = self.report_on(ui) {
            let card = match name {
                SONG => ui
                    .get_mut::<SongLanes>(SONG)
                    .and_then(|l| l.report.as_mut()),
                _ => ui
                    .get_mut::<SongListView>(SONG_LIST)
                    .and_then(|l| l.report.as_mut()),
            };
            if let Some(c) = card {
                c.scroll(delta);
            }
            return None;
        }
        if matches!(
            ui.layout_of(CLIPS_PAGE),
            Some(EDIT_LAYOUT | EDIT_DRAWER_LAYOUT)
        ) {
            let r = ui.rect_of(NOTES);
            if let (Some(r), true) = (
                r,
                ui.get::<NoteGrid>(NOTES)
                    .is_some_and(|g| g.lens == Lens::Grid),
            ) {
                if let Some(g) = ui.get_mut::<NoteGrid>(NOTES) {
                    g.scroll_rows(r, -delta);
                }
                return None;
            }
            return Some(TimeAction::Edit(EditAction::Scroll(delta)));
        }
        if matches!(
            ui.layout_of(CLIPS_PAGE),
            Some(
                SONG_LAYOUT
                    | SONG_CARD_LAYOUT
                    | SONG_CHORDS_LAYOUT
                    | SONG_MAKE_LAYOUT
                    | SONG_STYLE_LAYOUT
            )
        ) {
            self.first_bar = (self.first_bar + i64::from(delta) * self.bars_step()).max(0);
            return None;
        }
        if matches!(
            ui.layout_of(CLIPS_PAGE),
            Some(
                LIST_LAYOUT
                    | LIST_CARD_LAYOUT
                    | LIST_CHORDS_LAYOUT
                    | LIST_MAKE_LAYOUT
                    | LIST_STYLE_LAYOUT
            )
        ) {
            // The list scrolls with a drag.
            return None;
        }
        let rows = self.visible_rows(ui).max(1);
        let last = ROWS.saturating_sub(rows) as i64;
        self.first_row = (self.first_row as i64 + i64::from(delta)).clamp(0, last) as usize;
        None
    }

    /// The widget the conversion report card is open over (the Song view
    /// or the song list), when the page shows it: None when there is no
    /// card or neither shows (the grid, the note editor).
    pub fn report_on(&self, ui: &Ui) -> Option<&'static str> {
        match ui.layout_of(CLIPS_PAGE)? {
            SONG_LAYOUT | SONG_CARD_LAYOUT => ui
                .get::<SongLanes>(SONG)
                .is_some_and(|l| l.report.is_some())
                .then_some(SONG),
            LIST_LAYOUT | LIST_CARD_LAYOUT => ui
                .get::<SongListView>(SONG_LIST)
                .is_some_and(|l| l.report.is_some())
                .then_some(SONG_LIST),
            _ => None,
        }
    }

    /// Where the conversion report card is on the screen, while it shows:
    /// a touch outside it only closes it (the app's).
    pub fn report_rect(&self, ui: &Ui) -> Option<mpfree_ui::Rect> {
        let name = self.report_on(ui)?;
        let r = ui.rect_of(name)?;
        let card = match name {
            SONG => ui.get::<SongLanes>(SONG)?.report.as_ref()?.card_rect(r),
            _ => ui
                .get::<SongListView>(SONG_LIST)?
                .report
                .as_ref()?
                .card_rect(r),
        };
        Some(card)
    }

    /// The options list the wheel steps: the Edit options in the note
    /// editor, the clip settings card on the Record panel's Clip tab, else
    /// the record settings.
    pub fn wheel_list(&self, ui: &Ui) -> &'static str {
        match ui.layout_of(CLIPS_PAGE) {
            Some(EDIT_LAYOUT | EDIT_DRAWER_LAYOUT) => EDIT_OPTIONS,
            Some(CLIPS_CARD_LAYOUT | SONG_CARD_LAYOUT | LIST_CARD_LAYOUT) => CLIP_CARD,
            // The Chords tab: the wheel picks the row.
            Some(CLIPS_CHORDS_LAYOUT | SONG_CHORDS_LAYOUT | LIST_CHORDS_LAYOUT) => CHORDS_CARD,
            // Add a part's panel: the wheel steps its Role.
            Some(CLIPS_ADD_LAYOUT) => ADD_PANEL,
            _ => RECORD,
        }
    }

    /// The event is Add a part's or the row's chords' (composing tools
    /// S2.6): Add a part on the grid's title line, a chip or a bar of the
    /// row's chords, the chord picker, the panel, the Chords tab. Handle
    /// it with [`TimePage::compose`].
    pub fn owns(&self, ev: &UiEvent) -> bool {
        match (ev.widget.as_str(), &ev.kind) {
            (CLIPS, EventKind::Toggle(k)) => {
                *k == ROWS + ADD_PART || (ROWS + STRIP_CHIP..ROWS + PICKER + 64).contains(k)
            }
            (ADD_PANEL, _) => true,
            // The Song view's Chords lane and its picker.
            (SONG, EventKind::Tab(k)) => (crate::song::CHORD_CHIP
                ..crate::song::CHORD_PICK + crate::chord_strip::PICKS + 1)
                .contains(k),
            (CHORDS_CARD, EventKind::Tab(_)) => false,
            (CHORDS_CARD, _) => true,
            _ => false,
        }
    }

    /// The row whose chords show: the panel's, else the scene pick while
    /// the Chords tab shows.
    pub fn chords_row(&self, st: &TimeState) -> Option<usize> {
        match &self.add {
            Some(p) => Some(p.row),
            None if self.chords_card => Some(st.scene_focus),
            None => None,
        }
    }

    /// Close Add a part's panel: nothing kept, the row as it was.
    fn close_add(&mut self) {
        if self.add.take().is_some() {
            self.asks.push(TimeAction::Add(AddAction::Back));
        }
        self.keeping = false;
        if !self.chords_card {
            self.chord_pick = None;
        }
    }

    /// A new try with the panel's choices.
    fn ask_part(&mut self) {
        if let Some(p) = &self.add {
            self.asks
                .push(TimeAction::Add(AddAction::Ask(Box::new(p.ask()))));
        }
    }

    /// The app's list of phrases (MPFree's, the owner's, the project's):
    /// Add a part's Style row runs on into them. An open panel's phrase
    /// follows by name. True if the panel changed.
    pub fn set_phrases(&mut self, phrases: Arc<[(String, mpfree_arp::Phrase)]>) -> bool {
        self.phrases = phrases.clone();
        self.add.as_mut().is_some_and(|p| p.set_phrases(phrases))
    }

    /// Why the Q-Links do nothing on Add a part's panel now: a phrase is
    /// picked, and it plays as written.
    pub fn add_knob_why(&self) -> Option<&'static str> {
        self.add.as_ref().filter(|p| p.phrase.is_some()).map(|_| {
            "A phrase plays as written: the Q-Links shape a part the composer writes, on a style"
        })
    }

    /// Q-Link `k` turned `ticks` detents while Add a part's panel is open
    /// (Similarity, Variation, Activity, Range): a new try when it moved.
    /// False with the panel closed. On a phrase nothing moves
    /// ([`TimePage::add_knob_why`]).
    pub fn add_knob(&mut self, k: usize, ticks: i32) -> bool {
        let Some(p) = self.add.as_mut() else {
            return false;
        };
        if p.phrase.is_some() {
            return true;
        }
        if p.turn(k, ticks) {
            self.ask_part();
        }
        true
    }

    /// Add a part's or the row's chords' event ([`TimePage::owns`]): what
    /// it asks of the time engine goes to [`TimePage::take_asks`]. Why
    /// not, in words, when it can't be done (Add a part faint while Rec
    /// song is on, a setting at its end). Call [`TimePage::update`] after.
    pub fn compose(&mut self, ev: &UiEvent, st: &TimeState) -> Result<(), String> {
        let bar = st.transport.meter.bar_ticks().max(1);
        match (ev.widget.as_str(), &ev.kind) {
            (CLIPS, EventKind::Toggle(k)) if *k == ROWS + ADD_PART => {
                if self.add.is_some() {
                    self.close_add();
                    return Ok(());
                }
                if st.song_rec {
                    return Err(
                        "Add a part waits while Rec song is on: a try would be recorded into the song"
                            .to_string(),
                    );
                }
                self.open_add(st, st.scene_focus, None)
            }
            (CLIPS, EventKind::Toggle(k)) if (ROWS + PICKER..ROWS + PICKER + 64).contains(k) => {
                self.pick_chord(st, k - SLOTS - PICKER, bar);
                Ok(())
            }
            // The Song view's Chords lane: a chip opens the picker on the
            // scene's chord there (again: closes it).
            (SONG, EventKind::Tab(k)) if *k >= crate::song::CHORD_PICK => {
                self.pick_chord(st, k - crate::song::CHORD_PICK, bar);
                if self.chord_pick.is_none() {
                    self.song_chord = None;
                }
                Ok(())
            }
            (SONG, EventKind::Tab(k)) => {
                let i = k - crate::song::CHORD_CHIP;
                if self.song_chord == Some(i) && self.chord_pick.is_some() {
                    self.chord_pick = None;
                    self.song_chord = None;
                    return Ok(());
                }
                let lane = song_chord_lane(st);
                let Some(c) = lane.chips.get(i) else {
                    return Ok(());
                };
                let line =
                    shown_line(st, c.row).ok_or("The scene's chords are still being heard")?;
                self.chord_pick = ChordPick::open(c.row, &line, c.chip);
                self.song_chord = self.chord_pick.map(|_| i);
                Ok(())
            }
            (CLIPS, EventKind::Toggle(k)) if (ROWS + STRIP_BAR..ROWS + PICKER).contains(k) => {
                // A bar of a row with no chords yet: a bar of no chord to
                // pick on, for each of the row's bars.
                let Some(row) = self.chords_row(st) else {
                    return Ok(());
                };
                let b = k - SLOTS - STRIP_BAR;
                let line = bare_line(row_bars(st, row), bar);
                self.chord_pick =
                    ChordPick::open(row, &line, b).map(|p| ChordPick { fresh: true, ..p });
                Ok(())
            }
            (CLIPS, EventKind::Toggle(k)) if (ROWS + STRIP_CHIP..ROWS + STRIP_BAR).contains(k) => {
                let Some(row) = self.chords_row(st) else {
                    return Ok(());
                };
                let chip = k - SLOTS - STRIP_CHIP;
                self.chord_pick = match self.chord_pick {
                    // The chip it is open on: a tap closes it.
                    Some(p) if p.chip == chip && !p.fresh => None,
                    _ => shown_line(st, row).and_then(|l| ChordPick::open(row, &l, chip)),
                };
                Ok(())
            }
            (ADD_PANEL, EventKind::Toggle(k)) => {
                match *k {
                    add_view::PANEL_CLOSE | add_view::PANEL_BACK => self.close_add(),
                    add_view::PANEL_TRY_AGAIN
                        if self.add.as_ref().is_some_and(|p| p.phrase.is_some()) =>
                    {
                        return Err("A phrase plays one way, so Try again has nothing new: step back to a style for a part the composer writes".to_string());
                    }
                    add_view::PANEL_TRY_AGAIN => self.ask_part(),
                    add_view::PANEL_STEP => {
                        let pos = st.adding.as_ref().and_then(|a| a.position);
                        match pos {
                            Some((i, _)) if i > 1 => self
                                .asks
                                .push(TimeAction::Add(AddAction::Step { back: true })),
                            Some((i, n)) if i < n => self
                                .asks
                                .push(TimeAction::Add(AddAction::Step { back: false })),
                            _ => return Err("Try again first: there's one try so far".to_string()),
                        }
                    }
                    add_view::PANEL_KEEP => {
                        self.keeping = true;
                        self.asks.push(TimeAction::Add(AddAction::Keep));
                    }
                    _ => {}
                }
                Ok(())
            }
            (ADD_PANEL, EventKind::Select(k)) => {
                let changed = self.add.as_mut().is_some_and(|p| p.set_part(st, *k));
                if changed {
                    self.ask_part();
                }
                Ok(())
            }
            (ADD_PANEL, EventKind::Step { index, delta }) => {
                let Some(p) = self.add.as_mut() else {
                    return Ok(());
                };
                if p.step(st, *index, *delta)? {
                    self.ask_part();
                    return Ok(());
                }
                let end = if *delta < 0 { "first" } else { "last" };
                let name = match *index {
                    add_view::ROW_STYLE if p.phrase.is_some() => "Phrase",
                    k => PANEL_ROWS.get(k).copied().unwrap_or("That"),
                };
                Err(format!("{name} is at its {end} choice"))
            }
            (CHORDS_CARD, EventKind::Step { delta, .. }) => {
                let row = (st.scene_focus as i64 + i64::from((*delta).signum()))
                    .clamp(0, mpfree_seq::SCENES as i64 - 1) as usize;
                if row == st.scene_focus {
                    let end = if *delta < 0 { "first" } else { "last" };
                    return Err(format!("Row {} is the {end} row", row + 1));
                }
                self.chord_pick = None;
                self.asks.push(TimeAction::SceneFocus(row));
                Ok(())
            }
            (CHORDS_CARD, EventKind::Select(k)) => {
                let row = st.scene_focus;
                let track = st
                    .unfit
                    .iter()
                    .find(|u| u.row == row)
                    .and_then(|u| u.clips.get(*k))
                    .map(|c| c.0);
                if let Some(t) = track {
                    if !self.untick.remove(&(row, t)) {
                        self.untick.insert((row, t));
                    }
                }
                Ok(())
            }
            (CHORDS_CARD, EventKind::Toggle(CARD_FIT)) => {
                let row = st.scene_focus;
                let tracks: Vec<usize> = st
                    .unfit
                    .iter()
                    .find(|u| u.row == row)
                    .map(|u| {
                        u.clips
                            .iter()
                            .map(|c| c.0)
                            .filter(|t| !self.untick.contains(&(row, *t)))
                            .collect()
                    })
                    .unwrap_or_default();
                if tracks.is_empty() {
                    return Err("Tick a clip to fit it to the new chords".to_string());
                }
                self.asks
                    .push(TimeAction::Chords(ChordsAction::Fit { row, tracks }));
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Open Add a part's panel on row `row` (on `track` if its cell there
    /// is empty): the row picked, and a first try asked for at once.
    pub fn open_add(
        &mut self,
        st: &TimeState,
        row: usize,
        track: Option<usize>,
    ) -> Result<(), String> {
        if st.song_rec {
            return Err(
                "Add a part waits while Rec song is on: a try would be recorded into the song"
                    .to_string(),
            );
        }
        let mut pick = AddPick::open(st, row, track)?;
        pick.phrases = self.phrases.clone();
        self.chord_pick = None;
        self.keeping = false;
        self.kept = false;
        self.add = Some(pick);
        if st.scene_focus != row {
            self.asks.push(TimeAction::SceneFocus(row));
        }
        self.ask_part();
        Ok(())
    }

    /// The chord picker's answer `hit` (`chord_strip::PICK_*`).
    fn pick_chord(&mut self, st: &TimeState, hit: usize, bar: Tick) {
        let Some(mut p) = self.chord_pick else {
            return;
        };
        let line = if p.fresh {
            bare_line(row_bars(st, p.row), bar)
        } else {
            match shown_line(st, p.row) {
                Some(l) => l,
                None => return,
            }
        };
        let heard = heard_line(st, p.row);
        let picked = match hit {
            h if (PICK_ROOT..PICK_ROOT + 12).contains(&h) => {
                p.root = Some(h - PICK_ROOT);
                self.chord_pick = Some(p);
                return;
            }
            h if (PICK_KIND..PICK_KIND + 12).contains(&h) => {
                p.kind = Some(h - PICK_KIND);
                self.chord_pick = Some(p);
                return;
            }
            PICK_SPLIT => crate::add_view::split(&line, p.chip, bar),
            PICK_JOIN => crate::add_view::join_next(&line, p.chip),
            PICK_AS_HEARD => heard
                .as_ref()
                .and_then(|h| crate::add_view::as_heard(&line, p.chip, h)),
            PICK_USE => p
                .chord()
                .and_then(|c| crate::add_view::use_chord(&line, p.chip, c)),
            PICK_CLOSE | PICK_DISMISS => {
                self.chord_pick = None;
                return;
            }
            _ => return,
        };
        let Some(new) = picked else {
            return;
        };
        self.asks.push(line_action(st, p.row, new));
        // Use and As heard are done with the chip; Split and Join stay on
        // it, now no longer a fresh line.
        self.chord_pick = match hit {
            PICK_SPLIT | PICK_JOIN => Some(ChordPick { fresh: false, ..p }),
            _ => None,
        };
    }

    /// The hint line for Add a part's panel, the chord picker and the
    /// Chords tab, when one shows.
    pub fn compose_hint(&self, st: &TimeState) -> Option<String> {
        if self.song_chord.is_some() && self.chord_pick.is_some() && st.song_view {
            return Some("A root, then a kind   ·   Use: the scene's chords, one UNDO   ·   Split: two chords in the bar   ·   As heard: the scene's own again".to_string());
        }
        let grid = st.edit.is_none() && !st.song_view && !st.list_view;
        if self.chord_pick.is_some() && grid {
            return Some(match self.add {
                Some(_) => "A root, then a kind: the part you're trying plays it at once   ·   Split: two chords in the bar   ·   As heard: the row's own again   ·   Keep: the part and its chords, one UNDO".to_string(),
                None => "A root, then a kind   ·   Split: two chords in the bar   ·   Join next: one chord with the next   ·   As heard: the row's own again   ·   Use: the row's chords, one UNDO".to_string(),
            });
        }
        if let Some(p) = self.add.as_ref().filter(|_| grid) {
            // On a phrase (P2, board H).
            if p.phrase.is_some() {
                return Some(ADD_PHRASE_HINT.to_string());
            }
            // New track (S2.6b): Keep adds the track, and Sound picks its
            // synth.
            if p.new_track {
                return Some("Tap a part: hear it with the row   ·   Sound: the new track's synth   ·   tap a chord on the row: change it   ·   Keep: the track and its part, one UNDO   ·   Back: nothing changes".to_string());
            }
            return Some(format!(
                "Tap a part: hear it with the row   ·   Role: how it plays with the others   ·   tap a chord on the row: change it   ·   Keep: the clip in row {}, one UNDO   ·   Back: nothing changes",
                p.row + 1
            ));
        }
        self.chords_card.then(|| CHORDS_HINT.to_string())
    }

    /// Whether Add a part's panel closed because Keep kept the part,
    /// since the last ask: once.
    pub fn take_kept(&mut self) -> bool {
        std::mem::take(&mut self.kept)
    }

    /// What Add a part's status says while the app makes New track's
    /// synth, or that it didn't come (warning); None: the try's own words
    /// again. Nothing while the panel is closed. True if it changed, so
    /// the page is shown again ([`TimePage::update`]).
    pub fn set_add_waiting(&mut self, waiting: Option<(String, bool)>) -> bool {
        match self.add.as_mut() {
            Some(p) if p.waiting != waiting => {
                p.waiting = waiting;
                true
            }
            _ => false,
        }
    }

    /// Show `st`. Widgets whose contents did not change are left alone, so
    /// they are not redrawn. A track newly armed off the grid's page
    /// brings its page, and the page stays within the tracks there are.
    pub fn update(&mut self, ui: &mut Ui, st: &TimeState) {
        // Add a part's panel (layout spec 4r): once Keep has kept the
        // part it closes, and Rec song closes it with nothing kept. The
        // Song view, the list and the note editor only hide it: it is
        // there again on the Clips grid.
        if self.add.is_some() {
            if self.keeping && st.adding.is_none() {
                self.add = None;
                self.keeping = false;
                self.kept = true;
                if !self.chords_card {
                    self.chord_pick = None;
                }
            } else if st.song_rec {
                self.close_add();
            }
        }
        // New track follows a track added or taken away while it is open.
        if let Some(p) = self.add.as_mut() {
            p.follow_tracks(st);
        }
        // The row whose chords show is heard while it shows.
        let chords_row = self.chords_row(st);
        if chords_row != self.heard_asked {
            self.heard_asked = chords_row;
            self.asks
                .push(TimeAction::Chords(ChordsAction::Hear { row: chords_row }));
        }
        // The Song view's picker goes with the Song view.
        if self.song_chord.is_some() && (!st.song_view || self.chord_pick.is_none()) {
            self.song_chord = None;
            self.chord_pick = None;
        }
        if self.song_chord.is_none() && self.chord_pick.is_some_and(|p| Some(p.row) != chords_row) {
            self.chord_pick = None;
        }
        self.untick.retain(|&(r, t)| {
            st.unfit
                .iter()
                .any(|u| u.row == r && u.clips.iter().any(|c| c.0 == t))
        });
        // A track added from "+ Track": its page of the grid, and on the
        // Song view the end of the lanes, where it is.
        let mut reveal = false;
        match self.reveal {
            Some(n) if st.tracks.len() > n => {
                self.reveal = None;
                reveal = true;
            }
            Some(n) if st.tracks.len() < n => self.reveal = None,
            _ => {}
        }
        if st.armed != self.armed_seen {
            self.armed_seen = st.armed;
            if let Some(a) = st.armed {
                if !(self.first_track..self.first_track + GRID_COLS).contains(&a) {
                    self.first_track = a / GRID_COLS * GRID_COLS;
                }
            }
        }
        if reveal {
            self.first_track = (st.tracks.len() - 1) / GRID_COLS * GRID_COLS;
        }
        let last = st.tracks.len().saturating_sub(1) / GRID_COLS * GRID_COLS;
        self.first_track = self.first_track.min(last);
        let bar = bar_of(st);
        let same_bar = ui.get::<TransportBar>(TRANSPORT).is_some_and(|b| {
            b.run == bar.run
                && b.position == bar.position
                && b.items == bar.items
                && b.overdub == bar.overdub
                && b.undo == bar.undo
                && b.chips == bar.chips
        });
        if !same_bar {
            if let Some(b) = ui.get_mut::<TransportBar>(TRANSPORT) {
                b.run = bar.run;
                b.position = bar.position;
                b.items = bar.items;
                b.overdub = bar.overdub;
                b.undo = bar.undo;
                b.chips = bar.chips;
            }
        }
        // The grid, the song or the note editor, as the app has it.
        let editing = st.edit.is_some() && !self.edit_hidden;
        let want = match (editing, st.song_view, self.clip_card) {
            (true, ..) if self.edit_drawer => EDIT_DRAWER_LAYOUT,
            (true, ..) => EDIT_LAYOUT,
            _ if self.show && self.set_open => SHOW_LAYOUT,
            _ if self.make.is_some() && st.list_view => LIST_MAKE_LAYOUT,
            (false, true, _) if self.make.is_some() => SONG_MAKE_LAYOUT,
            (false, false, _) if self.make.is_some() => CLIPS_MAKE_LAYOUT,
            _ if st.style_sheet.is_some() && st.list_view => LIST_STYLE_LAYOUT,
            (false, true, _) if st.style_sheet.is_some() => SONG_STYLE_LAYOUT,
            (false, false, _) if st.style_sheet.is_some() => CLIPS_STYLE_LAYOUT,
            _ if st.list_view && self.chords_card => LIST_CHORDS_LAYOUT,
            _ if st.list_view && self.clip_card => LIST_CARD_LAYOUT,
            _ if st.list_view => LIST_LAYOUT,
            (false, false, _) if self.add.is_some() => CLIPS_ADD_LAYOUT,
            (false, true, _) if self.chords_card => SONG_CHORDS_LAYOUT,
            (false, false, _) if self.chords_card => CLIPS_CHORDS_LAYOUT,
            (false, true, false) => SONG_LAYOUT,
            (false, true, true) => SONG_CARD_LAYOUT,
            (false, false, false) => CLIPS_LAYOUT,
            (false, false, true) => CLIPS_CARD_LAYOUT,
        };
        if ui.layout_of(CLIPS_PAGE).is_some_and(|l| l != want) {
            let _ = ui.set_layout(CLIPS_PAGE, want);
        }
        // The grid, while it is on screen: its cells cost a row of 64 a
        // track, so the Song view and the list don't pay for them each
        // frame (scene banks, 5.6). Shown again, it is brought up to date
        // first.
        let grid_shown = ui.layout_of(CLIPS_PAGE).is_some_and(|l| {
            [
                CLIPS_LAYOUT,
                CLIPS_CARD_LAYOUT,
                CLIPS_ADD_LAYOUT,
                CLIPS_CHORDS_LAYOUT,
                CLIPS_MAKE_LAYOUT,
                CLIPS_STYLE_LAYOUT,
            ]
            .contains(&l)
        });
        // "+ Track"'s chooser closes once no track can be added.
        self.adding &= st.tracks.len() < MAX_TRACKS;
        if grid_shown {
            self.update_grid(ui, st, chords_row);
        }
        self.update_rest(ui, st, reveal);
    }

    /// [`TimePage::update`]'s part for the Clips grid.
    /// `chords_row` is the row whose chords show (layout spec 4r).
    fn update_grid(&mut self, ui: &mut Ui, st: &TimeState, chords_row: Option<usize>) {
        let tracks = grid_tracks(st);
        // Perform's big rows, Make's compact ones (S4): the rows in view
        // follow, kept within the slots.
        // (A look first: `get_mut` marks the grid for a paint.)
        if ui
            .get::<ClipGrid>(CLIPS)
            .is_some_and(|g| g.big != st.perform)
        {
            if let Some(g) = ui.get_mut::<ClipGrid>(CLIPS) {
                g.big = st.perform;
            }
        }
        let rows = self.visible_rows(ui).max(1);
        self.first_row = self.first_row.min(ROWS.saturating_sub(rows));
        self.rows_seen = rows;
        // With Loop tracks, the title is the loops' gauge: how many of the
        // looper's loops hold one, and the share of its memory in use (the
        // row numbers say which rows show). Past 8 tracks the pager takes
        // the legend's place, so the gauge is not there.
        let title = if st.tracks.iter().any(|t| t.loops.is_some()) {
            let l = &st.loops;
            format!(
                "Loops {} of {} · memory {} %",
                l.in_use,
                mpfree_seq::LOOPS,
                l.memory_pct
            )
        } else {
            // Scene banks (5.6): the bank chip says which bank shows.
            let shown = format!(
                "Rows {}–{}",
                self.first_row + 1,
                (self.first_row + rows).min(ROWS)
            );
            if st.loops_on && st.loop_cells() > 0 {
                // Loops in audio cells (audio tracks phase 4, board 2): the
                // loops' gauge after the rows in view.
                format!(
                    "{shown} · Loops {} of {}",
                    st.loops.in_use,
                    mpfree_seq::LOOPS
                )
            } else {
                shown
            }
        };
        let legend = "ring: one pass   ·   amber: waiting   ·   red: recording".to_string();
        let banks = banks_used(st);
        let can_add = st.tracks.len() < MAX_TRACKS;
        self.adding &= can_add;
        let adding = self.adding;
        // Capture scene is Perform's only (5.3).
        let can_capture = st.perform && capture_can(st);
        // Add a part, the row outlined, its chords under it and the chord
        // picker (layout spec 4r).
        let bar_t = st.transport.meter.bar_ticks().max(1);
        let add_part = match (&self.add, st.song_rec) {
            (Some(_), _) => PartButton::Open,
            (None, true) => PartButton::Faint,
            (None, false) => PartButton::Ready,
        };
        let outlined = chords_row;
        let open_chip = self.chord_pick.filter(|p| !p.fresh).map(|p| p.chip);
        let strip = outlined.and_then(|r| strip_of(st, r, open_chip));
        let picker = self.chord_pick.and_then(|p| {
            let line = if p.fresh {
                bare_line(row_bars(st, p.row), bar_t)
            } else {
                shown_line(st, p.row)?
            };
            let heard = st
                .row_chords
                .as_ref()
                .filter(|(r, _)| *r == p.row)
                .map(|(_, c)| &c.heard);
            Some(p.view(&line, heard, bar_t))
        });
        let same_grid = ui.get::<ClipGrid>(CLIPS).is_some_and(|g| {
            g.tracks == tracks
                && g.title == title
                && g.legend == legend
                && g.first_row == self.first_row
                && g.first_track == self.first_track
                && (g.scene_playing, g.scene_cued) == (st.scene_playing, st.scene_cued)
                && g.song_rec == st.song_rec
                && g.can_add == can_add
                && g.adding == adding
                && g.back_to_song == st.over_song()
                && g.banks == banks
                && g.pad_window == self.pad_window
                && g.can_capture == can_capture
                && g.show_button == self.set_open
                && g.add_part == add_part
                && g.outlined == outlined
                && g.strip == strip
                && g.picker == picker
        });
        if !same_grid {
            if let Some(g) = ui.get_mut::<ClipGrid>(CLIPS) {
                g.tracks = tracks;
                g.title = title;
                g.legend = legend;
                g.first_row = self.first_row;
                g.first_track = self.first_track;
                g.scene_playing = st.scene_playing;
                g.scene_cued = st.scene_cued;
                g.song_rec = st.song_rec;
                g.can_add = can_add;
                g.adding = adding;
                g.back_to_song = st.over_song();
                g.banks = banks;
                g.pad_window = self.pad_window;
                g.can_capture = can_capture;
                g.show_button = self.set_open;
                g.add_part = add_part;
                g.outlined = outlined;
                g.strip = strip;
                g.picker = picker;
            }
        }
    }

    /// [`TimePage::update`], after the grid: the song list, the Song view,
    /// the note editor and the panels.
    fn update_rest(&mut self, ui: &mut Ui, st: &TimeState, reveal: bool) {
        // The song list.
        let title = list_title(st);
        let can_add = st.list.len() < mpfree_seq::scenes::MAX_LIST_STEPS;
        let same_list = ui.get::<SongListView>(SONG_LIST).is_some_and(|l| {
            l.rows == st.list
                && l.title == title
                && l.picked == st.list_pick
                && l.looped == st.list_looped
                && l.can_add == can_add
                && l.show_button == self.set_open
                && l.linked == st.list_linked
        });
        if !same_list {
            if let Some(l) = ui.get_mut::<SongListView>(SONG_LIST) {
                l.rows = st.list.clone();
                l.title = title;
                l.picked = st.list_pick;
                l.looped = st.list_looped;
                l.can_add = can_add;
                l.show_button = self.set_open;
                l.linked = st.list_linked;
            }
        }
        // Link's chooser (4b.5), while the list shows and isn't linked.
        self.linking &= st.list_view && !st.list_linked;
        let choices = self.linking.then(|| link_choices(st));
        if let Some(l) = ui.get_mut::<SongListView>(SONG_LIST) {
            if l.link_choices != choices {
                l.link_choices = choices;
            }
        }
        // The conversion report card, over the list too (a new report
        // starts at its first row).
        let same_card = ui
            .get::<SongListView>(SONG_LIST)
            .is_some_and(|l| l.report.as_ref().map(|c| &c.report) == st.report.as_ref());
        if !same_card {
            if let Some(l) = ui.get_mut::<SongListView>(SONG_LIST) {
                l.report = st
                    .report
                    .clone()
                    .map(|r| ReportCard::new(r, &report_tracks(st)));
            }
        }
        // The scene card (5.2), over the grid or the list, whichever
        // shows; its cells' page stays while it is the same row's.
        let mut card = scene_card_of(st);
        if let Some(c) = card.as_mut() {
            c.picked = match self.scene_row {
                0 => 1,
                k => k,
            };
        }
        let (on_grid, on_list) = if st.list_view {
            (None, card)
        } else if st.song_view {
            (None, None)
        } else {
            (card, None)
        };
        let keep = |new: &mut Option<crate::scene_card::SceneCard>,
                    old: &Option<crate::scene_card::SceneCard>| {
            if let (Some(n), Some(o)) = (new.as_mut(), old.as_ref()) {
                n.keep_page(o);
            }
        };
        for (name, mut new) in [(CLIPS, on_grid), (SONG_LIST, on_list)] {
            let old = if name == CLIPS {
                ui.get::<ClipGrid>(CLIPS).map(|g| g.scene_card.clone())
            } else {
                ui.get::<SongListView>(SONG_LIST)
                    .map(|l| l.scene_card.clone())
            };
            let Some(old) = old else { continue };
            keep(&mut new, &old);
            if new == old {
                continue;
            }
            if name == CLIPS {
                if let Some(g) = ui.get_mut::<ClipGrid>(CLIPS) {
                    g.scene_card = new;
                }
            } else if let Some(l) = ui.get_mut::<SongListView>(SONG_LIST) {
                l.scene_card = new;
            }
        }
        // The blocks shown, while the placements are the same list: a frame
        // that only moves the playhead or scrolls copies and compares no
        // block (Arrange refactor 2.4).
        let blocks = ui
            .get::<SongLanes>(SONG)
            .map(|l| &l.blocks)
            .filter(|b| b.made_from(&st.placements))
            .cloned();
        let kept = blocks.is_some();
        let mut lanes = song_lanes_with(st, self.first_bar, self.bars_shown(), blocks);
        lanes.show_button = self.set_open;
        lanes.adding = self.adding;
        // The Chords lane's picker, open on a chip.
        if let (Some(k), Some(p)) = (self.song_chord, self.chord_pick) {
            let bar_t = st.transport.meter.bar_ticks().max(1);
            lanes.chords.open = Some(k);
            lanes.chords.picker = shown_line(st, p.row).map(|line| {
                let heard = heard_line(st, p.row);
                p.view(&line, heard.as_ref(), bar_t)
            });
        }
        let (same, same_waves, only_playhead) =
            ui.get::<SongLanes>(SONG).map_or((true, true, false), |l| {
                let waves = crate::song::same_waves(&l.waves, &st.waves);
                lanes.keep_scroll(l);
                if reveal {
                    lanes.first_lane = usize::MAX;
                }
                lanes.waves = Vec::new();
                let mut was = l.clone_without_waves();
                was.keep_scroll(l);
                let same = was == lanes;
                // Only the playhead moved, as each frame while the song
                // plays: the view keeps what it last painted and draws
                // only its line's strips (`SongLanes::changed_parts`).
                let only_playhead = !same && was.playhead != lanes.playhead && {
                    was.playhead = lanes.playhead;
                    was == lanes
                };
                (same, waves, only_playhead)
            });
        if only_playhead && same_waves {
            if let Some(l) = ui.get_mut::<SongLanes>(SONG) {
                l.playhead = lanes.playhead;
                if !kept {
                    l.blocks = lanes.blocks;
                }
            }
        } else if !(same && same_waves) {
            if let Some(l) = ui.get_mut::<SongLanes>(SONG) {
                let waves = std::mem::take(&mut l.waves);
                *l = lanes;
                l.waves = if same_waves { waves } else { st.waves.clone() };
            }
        } else if !kept {
            // The same blocks in a new list (a new state from the engine):
            // kept as made from it, so the next frames know them in one
            // step. Nothing drawn changes.
            if let Some(l) = ui.get_mut_quiet::<SongLanes>(SONG) {
                l.blocks = lanes.blocks;
            }
        }
        let m = st.morph;
        let focus = st.focus();
        let bar = MorphBar {
            a: m.a,
            b: m.b,
            pos: m.pos,
            on: m.on,
            params: m.params,
            reload: focus.is_some_and(|f| st.saved.contains(&f)),
            reload_waiting: focus.is_some_and(|f| st.reload_waiting(f)),
        };
        if ui.get::<MorphBar>(MORPH).is_some_and(|b| *b != bar) {
            if let Some(b) = ui.get_mut::<MorphBar>(MORPH) {
                *b = bar;
            }
        }
        // The roll's view is kept for each clip it has shown on a roll
        // with a size: first the roll's own (a pinch or a scroll moves it
        // there) for the clip it was showing.
        let key = st.edit.as_ref().map(|e| (e.editor.track, e.editor.slot));
        if let (Some(b), Some(g)) = (&self.edit_built, ui.get::<NoteGrid>(NOTES)) {
            let shown = (b.editor.track, b.editor.slot);
            if let Some(v) = self.views.get_mut(&shown) {
                *v = g.view;
            }
            match g.fold {
                Fold::All => self.folds.remove(&shown),
                f => self.folds.insert(shown, f),
            };
            self.edit_lane = g.lane;
            self.edit_hearing = g.hearing;
        }
        let continuing = st.edit.as_ref().and_then(|e| continuing_for(e, st));
        let same_continuing = same_continuing(self.edit_continuing.as_deref(), continuing);
        match &st.edit {
            Some(e)
                if same_continuing && self.edit_built.as_ref().is_some_and(|b| same_edit(b, e)) =>
            {
                // Only the playhead can have moved; with Follow on, the
                // roll pages along with it.
                let at = note_playhead(e, st);
                let r = ui.rect_of(NOTES);
                if ui.get::<NoteGrid>(NOTES).is_some_and(|g| g.playhead != at) {
                    if let Some(g) = ui.get_mut::<NoteGrid>(NOTES) {
                        g.playhead = at;
                        if let (Some(r), Some(t)) = (r, at) {
                            g.follow_to(r, t);
                        }
                    }
                }
                // The other clips behind it move on with each pass, and
                // what plays with it changes with what is launched (plan
                // 4.1): the roll changes only when they do.
                let edited = (e.editor.track, e.editor.slot);
                let moved = ui.get::<NoteGrid>(NOTES).is_some_and(|g| {
                    g.ghosts.len() != e.ghosts.len()
                        || ghost_phases(e, st).iter().zip(&g.ghosts).any(|(&p, l)| {
                            p != l.phase || plays_with(st, edited, (l.track, l.slot)) != l.with
                        })
                });
                if moved {
                    if let Some(g) = ui.get_mut::<NoteGrid>(NOTES) {
                        g.ghosts = ghost_layers(e, st);
                    }
                }
                if ui.get::<NoteGrid>(NOTES).is_some_and(|g| g.show_menu) {
                    let offers = show_offers(e, st);
                    if ui
                        .get::<NoteGrid>(NOTES)
                        .is_some_and(|g| g.show_offers != offers)
                    {
                        if let Some(g) = ui.get_mut::<NoteGrid>(NOTES) {
                            g.show_offers = offers;
                        }
                    }
                }
            }
            Some(e) => {
                let mut grid = note_grid(e, st);
                grid.pad_names = Arc::clone(&self.pad_names);
                grid.bank = self.pad_bank;
                grid.drawer = self.edit_drawer;
                grid.lane = self.edit_lane;
                grid.hearing = self.edit_hearing;
                grid.view = key
                    .and_then(|k| self.views.get(&k).copied())
                    .unwrap_or_default();
                grid.fold = key
                    .and_then(|k| self.folds.get(&k).copied())
                    .unwrap_or_default();
                if ui.get::<NoteGrid>(NOTES).is_some_and(|g| !g.same(&grid)) {
                    if let Some(g) = ui.get_mut::<NoteGrid>(NOTES) {
                        g.show(grid);
                    }
                }
                let rows = edit_rows(&e.editor, &e.clip);
                let picked = e.editor.sel.iter().any(|&i| i < e.clip.notes.len());
                // Continue open: its settings, and Lock in Correct's
                // place (step 3).
                let buttons = match e.editor.continuing_open() {
                    true => vec![OptionButton::new(
                        &e.editor.continue_picks.lock_label(),
                        true,
                    )],
                    false => vec![OptionButton::new(
                        if picked {
                            "Correct picked"
                        } else {
                            "Correct all"
                        },
                        true,
                    )],
                };
                let (title, selected) = match e.editor.continuing_open() {
                    true => ("Continue", e.editor.row_picked()),
                    false => ("Edit", e.editor.row),
                };
                if ui.get::<OptionList>(EDIT_OPTIONS).is_some_and(|l| {
                    l.rows != rows
                        || l.selected != selected
                        || l.buttons != buttons
                        || l.title != title
                }) {
                    if let Some(l) = ui.get_mut::<OptionList>(EDIT_OPTIONS) {
                        l.rows = rows;
                        l.selected = selected;
                        l.right = "wheel or arrows".to_string();
                        l.buttons = buttons;
                        l.title = title.to_string();
                    }
                }
                self.edit_built = Some(e.clone());
                self.edit_continuing = st.continuing.clone().filter(|_| continuing.is_some());
            }
            None => {
                self.edit_built = None;
                self.edit_continuing = None;
            }
        }
        if let (Some(e), Some(r)) = (&st.edit, ui.rect_of(NOTES)) {
            let key = (e.editor.track, e.editor.slot);
            // A clip on a roll with a size for the first time: the view it
            // opens with ([`NoteGrid::open_view`]).
            if let std::collections::btree_map::Entry::Vacant(slot) = self.views.entry(key) {
                if let Some(g) = ui.get_mut::<NoteGrid>(NOTES) {
                    g.view = g.open_view(r);
                    slot.insert(g.view);
                }
            }
            // The wheel moved the cursor: the roll keeps it on screen (not
            // when a clip first shows).
            let seen = (key, e.editor.cursor);
            if self.cursor_seen != Some(seen) {
                let moved = self.cursor_seen.is_some_and(|(k, _)| k == key);
                self.cursor_seen = Some(seen);
                if let Some(g) = ui.get_mut::<NoteGrid>(NOTES).filter(|_| moved) {
                    if g.lens == Lens::Grid && g.show_tick(r, e.editor.cursor) {
                        if let Some(v) = self.views.get_mut(&key) {
                            *v = g.view;
                        }
                    }
                }
            }
            if let Some(g) = ui.get::<NoteGrid>(NOTES) {
                // Snap Auto follows the zoom once the fingers lift: the
                // division goes to the editor only when it changes.
                if e.editor.snap_auto() && !g.busy() {
                    let want = g.auto_division(r, e.editor.auto, e.editor.snap == SNAP_AUTO_T);
                    if want == e.editor.auto {
                        self.auto_asked = None;
                    } else if self.auto_asked != Some((key, want)) {
                        self.auto_asked = Some((key, want));
                        self.asks.push(TimeAction::Edit(EditAction::AutoSnap(want)));
                    }
                }
            }
        }
        let rows = record_rows(st);
        let tabs = panel_tabs(st);
        if ui
            .get::<OptionList>(RECORD)
            .is_some_and(|l| l.rows != rows || l.tabs != tabs)
        {
            if let Some(l) = ui.get_mut::<OptionList>(RECORD) {
                l.rows = rows;
                l.tabs = tabs.clone();
                l.right = "wheel or arrows".to_string();
            }
        }
        let rows = clip_rows(st);
        let buttons = card_buttons(st);
        let note = card_note(st);
        let focus = st.focus();
        let refocus = focus != self.card_focus;
        self.card_focus = focus;
        if ui.get::<OptionList>(CLIP_CARD).is_some_and(|l| {
            refocus || l.rows != rows || l.tabs != tabs || l.buttons != buttons || l.note != note
        }) {
            if let Some(l) = ui.get_mut::<OptionList>(CLIP_CARD) {
                // Another clip: its card starts at the top.
                if refocus {
                    l.reset_view();
                }
                l.rows = rows;
                l.tabs = tabs.clone();
                l.buttons = buttons;
                l.note = note;
            }
        }
        // The Make a phrase sheet, while it shows.
        if let Some(v) = &self.make {
            if ui
                .get::<MakeSheet>(MAKE_SHEET)
                .is_some_and(|w| w.view != *v)
            {
                if let Some(w) = ui.get_mut::<MakeSheet>(MAKE_SHEET) {
                    w.view = v.clone();
                }
            }
        }
        // The Make a style sheet, while it shows (the time engine's).
        let sheet = st.style_sheet.as_deref();
        if ui
            .get::<StyleSheetView>(STYLE_SHEET)
            .is_some_and(|w| w.sheet.as_ref() != sheet)
        {
            if let Some(w) = ui.get_mut::<StyleSheetView>(STYLE_SHEET) {
                w.sheet = sheet.cloned();
            }
        }
        // Add a part's panel and the Chords tab, while they show.
        if let Some(p) = &self.add {
            let v = panel_view(st, p);
            if ui.get::<AddPanel>(ADD_PANEL).is_some_and(|w| w.view != v) {
                if let Some(w) = ui.get_mut::<AddPanel>(ADD_PANEL) {
                    w.view = v;
                }
            }
        }
        if self.chords_card {
            let v = chords_view(st, &self.untick);
            if ui
                .get::<ChordsCard>(CHORDS_CARD)
                .is_some_and(|w| w.view != v || w.head.tabs != tabs)
            {
                if let Some(w) = ui.get_mut::<ChordsCard>(CHORDS_CARD) {
                    w.view = v;
                    w.head.tabs = tabs;
                }
            }
        }
        // While the note editor's Continue is open, its tries.
        let tries = continuing_open(st);
        let (items, current, undo, redo, capture) = match tries {
            true => {
                let (items, current, back, on) = try_list(st);
                (items, current, back, on, None)
            }
            false => {
                let (items, current) = take_items(st);
                let undo = st.done.last().map(TakeRow::short);
                let redo = st.undone.last().map(TakeRow::short);
                (items, current, undo, redo, st.capture.clone())
            }
        };
        let (title, right, words, empty) = match tries {
            true => (
                "Tries",
                "the last 8",
                ["‹", "›", ""],
                Some(("No tries yet", "The first is on its way")),
            ),
            false => ("Takes", "SHIFT + UNDO: redo", TAKE_WORDS, None),
        };
        let same_takes = ui.get::<TakeList>(TAKES).is_some_and(|l| {
            l.items == items
                && l.current == current
                && l.undo == undo
                && l.redo == redo
                && l.capture == capture
                && l.title == title
                && l.words == words
        });
        if !same_takes {
            if let Some(l) = ui.get_mut::<TakeList>(TAKES) {
                l.items = items;
                l.current = current;
                l.undo = undo;
                l.redo = redo;
                l.capture = capture;
                l.title = title.to_string();
                l.right = right.to_string();
                l.words = words;
                l.empty = empty;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::HELD_PICKS;

    /// The Scale menu's taps (piano roll plan 2B.1) become the clip's
    /// scale, from the editor's offers on the clip's root, or its root.
    #[test]
    fn the_scale_menu_sets_the_clips_scale() {
        use crate::editor::{ScaleOffers, ROOT_MENU, SCALE_MENU};
        use mpfree_seq::mpfree_scale::{DORIAN, MAJOR, MINOR};
        let mut st = crate::sample::notes_state(Lens::Grid).unwrap();
        let e = st.edit.as_mut().unwrap();
        let mut keys = (*e.clip).clone();
        keys.scale = Some(mpfree_seq::Scale::new(9, DORIAN));
        e.clip = keys.into();
        e.editor.offers = Arc::new(ScaleOffers {
            pads: Some(mpfree_seq::Scale::new(2, MINOR)),
            own: Vec::new(),
        });
        let ev = |k| {
            action(
                &UiEvent {
                    widget: NOTES.into(),
                    kind: EventKind::Tab(k),
                },
                &st,
            )
        };
        let set = |k| Some(TimeAction::Edit(EditAction::Scale(k)));
        assert_eq!(ev(SCALE_MENU), set(None));
        assert_eq!(
            ev(SCALE_MENU + 1),
            set(Some(mpfree_seq::Scale::new(2, MINOR)))
        );
        assert_eq!(
            ev(SCALE_MENU + 2),
            set(Some(mpfree_seq::Scale::new(9, MAJOR)))
        );
        assert_eq!(ev(SCALE_MENU + 999), None);
        assert_eq!(
            ev(ROOT_MENU + 4),
            Some(TimeAction::Edit(EditAction::Root(4)))
        );
        assert_eq!(
            ev(MENU + 13),
            Some(TimeAction::Edit(EditAction::Choose(13)))
        );
        // The grid shows the clip's scale and offers.
        let g = note_grid(st.edit.as_ref().unwrap(), &st);
        assert_eq!(g.scale, Some(mpfree_seq::Scale::new(9, DORIAN)));
        assert_eq!(g.scale_name, "A Dorian");
        assert_eq!(g.offers.pads, Some(mpfree_seq::Scale::new(2, MINOR)));
    }

    /// A tool open (plan 2B.3): the roll shows its preview, the pieces it
    /// makes picked, with the tool's bar; the Q-Link column its settings;
    /// the bar's buttons are the tool's actions. At no change, the bar
    /// says so and the roll shows the clip.
    #[test]
    fn a_tool_open_previews_on_the_roll_with_its_bar_and_cards() {
        let st = crate::sample::notes_tool_state().unwrap();
        let e = st.edit.as_ref().unwrap();
        let g = note_grid(e, &st);
        let bar = g.tool_bar.clone().unwrap();
        assert_eq!(bar.name, "Chop");
        assert_eq!(bar.option, Some(("Reset pressure".to_string(), false)));
        assert_eq!(bar.hint, "Turn the Q-Links: the roll shows the result");
        assert_eq!(g.notes.len(), e.clip.notes.len() + 6, "two notes in four");
        assert_eq!(g.notes.iter().filter(|n| n.selected).count(), 8);
        let pressed = (0..g.notes.len())
            .filter(|&i| g.notes[i].selected && !g.touch_of(i).is_empty())
            .count();
        assert_eq!(pressed, 8, "each piece its share of the pressure");
        let (head, cards) = note_cards(e).unwrap();
        assert_eq!(head, "Chop · 2 notes");
        assert_eq!(
            (cards[0].name.as_str(), cards[0].value.as_str()),
            ("Pieces", "4")
        );
        assert!(cards[1..].iter().all(|c| c.is_free()));
        let t = e.editor.open_tool.unwrap();
        assert!(tool_hint(&t).starts_with("Chop: Q1 pieces (SHIFT: finer)"));
        let ev = |k| {
            action(
                &UiEvent {
                    widget: NOTES.into(),
                    kind: EventKind::Tab(TOOL_BUTTONS + k),
                },
                &st,
            )
        };
        let tool = |a| Some(TimeAction::Edit(EditAction::Tool(a)));
        assert_eq!(ev(TOOL_CANCEL), tool(ToolAct::Cancel));
        assert_eq!(ev(TOOL_OPTION), tool(ToolAct::Option));
        assert_eq!(ev(TOOL_DONE), tool(ToolAct::Done));
        assert_eq!(ev(3), None);
        // Strum at no change on a chord: the clip as it is, and why.
        let mut st = crate::sample::notes_chord_state().unwrap();
        let e = st.edit.as_mut().unwrap();
        e.editor.open_tool = Some(NoteTool::new(ToolKind::Strum));
        let e = st.edit.as_ref().unwrap();
        let g = note_grid(e, &st);
        assert_eq!(g.tool_bar.unwrap().hint, "Nothing changes yet");
        assert_eq!(g.notes.len(), e.clip.notes.len());
        assert_eq!(
            g.notes.iter().filter(|n| n.selected).count(),
            4,
            "a chord and the melody note in it"
        );
    }

    /// Continue open (composing tools, phase 1 step 2): the roll shows
    /// the clip as heard with the result, the new bars from where they
    /// start, each new bar's job on the ruler and the chords, nothing
    /// picked; the bar says which try plays; the Q-Links and the drawer
    /// are Continue's, and the takes area its tries, ‹ and › stepping
    /// them while there is one that way. Closed, the clip as it is.
    #[test]
    fn continue_open_shows_its_result_its_settings_and_its_tries() {
        let st = crate::sample::notes_continue_state().unwrap();
        let e = st.edit.as_ref().unwrap();
        let c = st.continuing.as_ref().unwrap();
        let g = note_grid(e, &st);
        const BAR: Tick = 3840;
        assert_eq!(g.length, 6 * BAR);
        assert_eq!(g.notes.len(), c.clip.as_ref().unwrap().notes.len());
        assert!(g.notes.len() > e.clip.notes.len());
        assert!(g.notes.iter().all(|n| !n.selected));
        assert_eq!(g.new_from, Some(2 * BAR));
        let bars: Vec<Tick> = g.bar_jobs.iter().map(|j| j.0).collect();
        assert_eq!(bars, vec![2 * BAR, 3 * BAR, 4 * BAR, 5 * BAR]);
        assert!(g.chords.iter().any(|k| k.suggested), "{:?}", g.chords);
        assert!(g.chords.iter().any(|k| !k.suggested), "{:?}", g.chords);
        assert_eq!(g.chords.last().map(|k| k.end), Some(6 * BAR));
        let bar = g.tool_bar.clone().unwrap();
        assert_eq!(bar.name, "Continue");
        assert_eq!(bar.option, Some(("Try again".to_string(), false)));
        assert_eq!(
            bar.hint,
            "Try 2 of 2 plays in place of the clip: Done keeps it"
        );
        let (head, cards) = note_cards(e).unwrap();
        assert_eq!(head, "Continue · +4 bars");
        let names: Vec<&str> = cards.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Similarity", "Variation", "Activity", "Range"]);
        assert!(
            tool_hint(&e.editor.open_tool.unwrap()).contains("Tap new bars or notes to pick them")
        );
        let rows = edit_rows(&e.editor, &e.clip);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, crate::editor::compose::CONTINUE_ROWS);
        // The tries, newest first, the one heard lit.
        let (items, current, back, on) = try_list(&st);
        let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["Try 2 · Minimal", "Try 1 · Minimal"]);
        assert_eq!(items[0].detail, "+4 bars");
        assert_eq!((current, back.as_deref(), on), (Some(0), Some("1"), None));
        let tap = |k| UiEvent {
            widget: TAKES.into(),
            kind: EventKind::Tab(k),
        };
        let step = |back| Some(TimeAction::Continue(ContinueAction::Step { back }));
        assert_eq!(action(&tap(0), &st), step(true));
        assert_eq!(action(&tap(1), &st), None);
        assert_eq!(action(&tap(2), &st), None);
        assert_eq!(
            why_not(&tap(1), &st).as_deref(),
            Some("This is the last try: Try again makes another")
        );
        // Closed: the clip as it is, and the takes again.
        let mut closed = st.clone();
        closed.edit.as_mut().unwrap().editor.open_tool = None;
        let g = note_grid(closed.edit.as_ref().unwrap(), &closed);
        assert_eq!((g.notes.len(), g.new_from), (e.clip.notes.len(), None));
        assert!(g.bar_jobs.is_empty() && g.chords.is_empty());
        assert_eq!(action(&tap(2), &closed), None);
        assert!(!continuing_open(&closed));
    }

    /// Continue's picks (phase 1 step 3): the ruler marks the picked and
    /// the locked bars, a picked new note is outlined, Try again says
    /// what it works on and the drawer's button is Lock; a tap on a new
    /// bar or note picks it, and the drawer's button locks while
    /// Continue is open (Timing Correct otherwise). The tries say which
    /// was made of part of the one before.
    #[test]
    fn continue_picks_show_on_the_roll_and_name_try_again_and_lock() {
        let st = crate::sample::notes_continue_picks_state().unwrap();
        let e = st.edit.as_ref().unwrap();
        let g = note_grid(e, &st);
        assert_eq!(
            (g.picked_bars.clone(), g.locked_bars.clone()),
            (vec![5], vec![3])
        );
        assert!(
            g.notes.iter().all(|n| !n.selected),
            "bars picked, not notes"
        );
        let bar = g.tool_bar.clone().unwrap();
        assert_eq!(bar.option, Some(("Try bar 6 again".to_string(), false)));
        assert_eq!(e.editor.continue_picks.lock_label(), "Lock bar 6");
        let (items, current, ..) = try_list(&st);
        let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(
            titles,
            ["Try 3 · Minimal", "Try 2 · Minimal", "Try 1 · Minimal"]
        );
        assert_eq!(items[0].detail, "bar 6");
        assert_eq!(items[1].detail, "+4 bars");
        assert_eq!(current, Some(0));
        // A new note picked: outlined, your note at the same place never.
        let mut picked = st.clone();
        let c = st.continuing.as_ref().unwrap();
        let new = c
            .clip
            .as_ref()
            .unwrap()
            .notes
            .iter()
            .find(|n| n.tick >= c.new_from)
            .unwrap()
            .tick;
        let own = e.clip.notes[0].tick;
        let ed = &mut picked.edit.as_mut().unwrap().editor;
        ed.continue_picks.bars.clear();
        ed.continue_picks.notes = vec![new, own];
        let g = note_grid(picked.edit.as_ref().unwrap(), &picked);
        let lit: Vec<Tick> = g
            .notes
            .iter()
            .filter(|n| n.selected)
            .map(|n| n.tick)
            .collect();
        assert!(!lit.is_empty() && lit.iter().all(|&t| t == new), "{lit:?}");
        assert_eq!(
            g.tool_bar.unwrap().option,
            Some(("Try 2 notes again".to_string(), false))
        );
        // The touches.
        let ev = |widget: &str, kind| {
            action(
                &UiEvent {
                    widget: widget.into(),
                    kind,
                },
                &st,
            )
        };
        let edit = |a| Some(TimeAction::Edit(a));
        assert_eq!(
            ev(NOTES, EventKind::Tab(CONTINUE_BAR + 4)),
            edit(EditAction::ContinueBar(4))
        );
        assert_eq!(
            ev(NOTES, EventKind::Tab(CONTINUE_NOTE + 9000)),
            edit(EditAction::ContinueNote(9000))
        );
        assert_eq!(
            ev(EDIT_OPTIONS, EventKind::Toggle(0)),
            edit(EditAction::ContinueLock)
        );
        let mut closed = st.clone();
        closed.edit.as_mut().unwrap().editor.open_tool = None;
        assert_eq!(
            action(
                &UiEvent {
                    widget: EDIT_OPTIONS.into(),
                    kind: EventKind::Toggle(0)
                },
                &closed
            ),
            edit(EditAction::Correct)
        );
    }

    /// The Chord menu (plan 2B.2): a shape keeps Major or Minor, they
    /// keep the shape; the grid shows the editor's chord.
    #[test]
    fn the_chord_menu_sets_what_a_drawn_note_puts_in() {
        let mut st = crate::sample::notes_state(Lens::Grid).unwrap();
        let e = st.edit.as_mut().unwrap();
        e.editor.chord = ChordShape::Seventh;
        e.editor.minor = true;
        let ev = |k| {
            action(
                &UiEvent {
                    widget: NOTES.into(),
                    kind: EventKind::Tab(k),
                },
                &st,
            )
        };
        let set = |shape, minor| Some(TimeAction::Edit(EditAction::Chord { shape, minor }));
        assert_eq!(ev(CHORD_MENU + 1), set(ChordShape::Triad, true));
        assert_eq!(ev(CHORD_MENU + 7), set(ChordShape::Pads, true));
        assert_eq!(
            ev(CHORD_MENU + CHORD_MAJOR),
            set(ChordShape::Seventh, false)
        );
        assert_eq!(ev(CHORD_MENU + CHORD_MINOR), set(ChordShape::Seventh, true));
        assert_eq!(ev(CHORD_MENU + 50), None);
        let g = note_grid(st.edit.as_ref().unwrap(), &st);
        assert_eq!((g.chord, g.minor), (ChordShape::Seventh, true));
    }

    /// Picking on the roll (piano roll plan 1B.2): a box, a note tapped
    /// while another is held and a handle dragged become the editor's
    /// actions; a drawn note's `Span` stays a drawn note.
    #[test]
    fn the_rolls_picks_and_handles_become_editor_actions() {
        use crate::editor::BOX;
        let st = TimeState::default();
        let ev = |kind| {
            action(
                &UiEvent {
                    widget: NOTES.into(),
                    kind,
                },
                &st,
            )
        };
        let edit = |a| Some(TimeAction::Edit(a));
        assert_eq!(
            ev(EventKind::Span {
                index: BOX + (50 << 8) + 52,
                from: 0,
                to: 960
            }),
            edit(EditAction::Pick {
                from: 0,
                to: 960,
                low: 50,
                high: 52
            })
        );
        assert_eq!(
            ev(EventKind::Span {
                index: 50,
                from: 0,
                to: 240
            }),
            edit(EditAction::Draw {
                tick: 0,
                note: 50,
                len: 240
            })
        );
        assert_eq!(
            ev(EventKind::Cell { row: ADD, col: 3 }),
            edit(EditAction::Add { index: 3, rev: 0 })
        );
        // The clip's end handle (plan 1A.2).
        assert_eq!(
            ev(EventKind::Stretch {
                index: CLIP_END,
                to: 7680
            }),
            edit(EditAction::Length { to: 7680 })
        );
        // A finger on a note and the wheel (plan 1C.3): the first detent
        // names the note, a later one only the hold.
        assert_eq!(
            ev(EventKind::Values {
                index: HELD_TURN + HELD_FIRST + HELD_FINE,
                set: vec![(3, -2), (7, 0)]
            }),
            edit(EditAction::Held {
                index: 3,
                rev: 0,
                ticks: -2,
                fine: true,
                turn: 7,
                first: true
            })
        );
        assert_eq!(
            ev(EventKind::Values {
                index: HELD_TURN,
                set: vec![(HELD_PICKS, 1), (7, 0)]
            }),
            edit(EditAction::Held {
                index: HELD_PICKS,
                rev: 0,
                ticks: 1,
                fine: false,
                turn: 7,
                first: false
            })
        );
        assert_eq!(
            ev(EventKind::Values {
                index: HELD_TURN,
                set: vec![(3, 1)]
            }),
            None,
            "no hold's number"
        );
        // SHIFT + the push on a held note: back to a drawn note's velocity.
        assert_eq!(
            ev(EventKind::Values {
                index: HELD_TURN + HELD_RESET + HELD_FIRST,
                set: vec![(3, 0), (7, 0)]
            }),
            edit(EditAction::HeldReset {
                index: 3,
                rev: 0,
                turn: 7,
                first: true
            })
        );
        // ERASE + a drag (plan 1B.5): the notes it crossed.
        assert_eq!(
            ev(EventKind::Values {
                index: ERASE_SWEEP,
                set: vec![(4, 0), (1, 0)]
            }),
            edit(EditAction::Sweep {
                notes: vec![4, 1],
                rev: 0
            })
        );
        assert_eq!(
            ev(EventKind::Stretch {
                index: 2 + START_HANDLE,
                to: -240
            }),
            edit(EditAction::Resize {
                index: 2,
                rev: 0,
                start: true,
                ticks: -240
            })
        );
        assert_eq!(
            ev(EventKind::Tab(crate::editor::MENU + 5)),
            edit(EditAction::Choose(5))
        );
        assert_eq!(ev(EventKind::Tab(5)), edit(EditAction::Button(5)));
        assert_eq!(
            ev(EventKind::Stretch { index: 2, to: 240 }),
            edit(EditAction::Resize {
                index: 2,
                rev: 0,
                start: false,
                ticks: 240
            })
        );
    }
    use mpfree_seq::{Clip, Note, Track};
    use mpfree_ui::widgets::{Header, Label};
    use mpfree_ui::{Align, Fonts, SoftCanvas, Theme};

    const BAR: Tick = 3840;

    fn state() -> TimeState {
        let mut p = Project::new();
        for (i, name) in ["Drums", "Bass", "Stab"].iter().enumerate() {
            let mut t = Track::new(name, i as u16);
            let mut c = Clip::new("A", 2 * BAR);
            c.add_note(Note {
                tick: 0,
                len: 10,
                note: 1,
                velocity: 100,
                chance: 100,
                cond: mpfree_seq::Cond::Always,
                ratchet: 1,
                touch: mpfree_seq::Touch::NONE,
                on: true,
                ramp: 0,
                join: mpfree_seq::Join::Attack,
                group: 0,
                heard: false,
            })
            .unwrap();
            t.slots[0] = Some(c);
            t.slots[1] = Some(Clip::new("B", 1680));
            p.tracks.push(t);
        }
        let launches = [
            Launch {
                track: 0,
                slot: 0,
                origin: 0,
                start: 0,
            },
            Launch {
                track: 1,
                slot: 1,
                origin: 5 * BAR,
                start: 5 * BAR,
            },
        ];
        let t = TransportInfo {
            state: State::Playing,
            position: 4 * BAR + 960,
            ..TransportInfo::default()
        };
        TimeState::from_engine(t, &p, &launches, Some(2), &Takes::default())
    }

    fn ev(widget: &str, kind: EventKind) -> UiEvent {
        UiEvent {
            widget: widget.to_string(),
            kind,
        }
    }

    /// Two scenes' chords for the Song view's Chords lane: row 0 two bars
    /// (C, then G7), row 2 one chord of Am you set, two bars long.
    fn song_chords_state() -> (TimeState, mpfree_context::chord::ChordTimeline) {
        use mpfree_context::chord::{Chord, ChordTimeline, Heard, Quality};
        let mut st = state();
        st.song_view = true;
        let bar = st.transport.meter.bar_ticks();
        st.scene_sections = vec![(0, 4 * bar, 0), (4 * bar, 7 * bar, 2)];
        let two = ChordTimeline::of(
            &[
                (Chord::new(0, Quality::Major), bar),
                (Chord::new(7, Quality::Dominant7), bar),
            ],
            Heard::Heard,
        );
        let am = ChordTimeline::of(&[(Chord::new(9, Quality::Minor), 2 * bar)], Heard::Set);
        st.song_chords = vec![
            (
                0,
                Arc::new(mpfree_compose::RowChords {
                    line: two.clone(),
                    heard: two.clone(),
                }),
            ),
            (
                2,
                Arc::new(mpfree_compose::RowChords {
                    line: am.clone(),
                    heard: am,
                }),
            ),
        ]
        .into();
        (st, two)
    }

    /// The Song view's Chords lane (composing step 6, board C): each
    /// section made from a scene shows the scene's chords end to end from
    /// its start, cut at its end; until a scene is heard its section is
    /// empty, and with none heard the lane says it is hearing.
    #[test]
    fn the_chords_lane_lays_each_scenes_chords_along_its_section() {
        use crate::chord_strip::ChipLook;
        let (mut st, _) = song_chords_state();
        let bar = st.transport.meter.bar_ticks();
        let lane = song_chord_lane(&st);
        assert!(!lane.hearing);
        let chips: Vec<(Tick, Tick, &str, &str, usize, usize)> = lane
            .chips
            .iter()
            .map(|c| {
                (
                    c.from / bar,
                    c.to / bar,
                    c.name.as_str(),
                    c.root.as_str(),
                    c.row,
                    c.chip,
                )
            })
            .collect();
        assert_eq!(
            chips,
            [
                (0, 1, "C", "C", 0, 0),
                (1, 2, "G7", "G", 0, 1),
                (2, 3, "C", "C", 0, 0),
                (3, 4, "G7", "G", 0, 1),
                (4, 6, "Am", "A", 2, 0),
                (6, 7, "Am", "A", 2, 0),
            ]
        );
        assert_eq!(lane.chips[0].look, ChipLook::Heard);
        assert_eq!(lane.chips[4].look, ChipLook::Yours);
        // Row 2 not heard yet: its section is empty, row 0's still shows.
        st.song_chords = st.song_chords[..1].to_vec().into();
        let lane = song_chord_lane(&st);
        assert_eq!((lane.chips.len(), lane.hearing), (4, false));
        st.song_chords = Arc::from(Vec::new());
        let lane = song_chord_lane(&st);
        assert!(lane.chips.is_empty() && lane.hearing, "hearing the scenes");
        st.scene_sections.clear();
        assert!(
            !song_chord_lane(&st).hearing,
            "no sections made from scenes"
        );
    }

    /// A chip on the Chords lane opens the chord picker on the scene's
    /// chord there; a root, a kind and Use set the scene's chords as one
    /// take (`ChordsAction::Set`, the line as shown before it); the chip
    /// tapped again closes the picker.
    #[test]
    fn a_chip_on_the_chords_lane_opens_the_picker_and_use_sets_the_scenes_chords() {
        let (st, two) = song_chords_state();
        let mut page = TimePage::default();
        let chip = ev(SONG, EventKind::Tab(crate::song::CHORD_CHIP + 3));
        assert!(page.owns(&chip));
        page.compose(&chip, &st).unwrap();
        let p = page.chord_pick.expect("the picker opens");
        assert_eq!((p.row, p.chip, p.root, p.kind), (0, 1, Some(7), Some(2)));
        assert_eq!(page.song_chord, Some(3));
        assert!(page
            .compose_hint(&st)
            .is_some_and(|h| h.contains("Use: the scene's chords")));
        let pick = |k| ev(SONG, EventKind::Tab(crate::song::CHORD_PICK + k));
        assert!(page.owns(&pick(PICK_USE)));
        page.compose(&pick(PICK_ROOT + 8), &st).unwrap();
        page.compose(&pick(PICK_KIND + 2), &st).unwrap();
        assert!(page.take_asks().is_empty(), "nothing until Use");
        page.compose(&pick(PICK_USE), &st).unwrap();
        match &page.take_asks()[..] {
            [TimeAction::Chords(ChordsAction::Set { row, line, shown })] => {
                assert_eq!(*row, 0);
                assert_eq!(line.names(), "C Ab7");
                assert_eq!(shown, &two, "the line as the lane showed it");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!((page.chord_pick, page.song_chord), (None, None));
        // Again, then the same chip: closed with nothing asked.
        page.compose(&chip, &st).unwrap();
        assert!(page.chord_pick.is_some());
        page.compose(&chip, &st).unwrap();
        assert_eq!((page.chord_pick, page.song_chord), (None, None));
        // × closes it too.
        page.compose(&chip, &st).unwrap();
        page.compose(&pick(PICK_CLOSE), &st).unwrap();
        assert_eq!((page.chord_pick, page.song_chord), (None, None));
        assert!(page.take_asks().is_empty());
    }

    #[test]
    fn the_bar_goes_under_the_header() {
        let play = "col(header:60, row(pads:*, col(kits:*, level:96, timing:232):460):*, hint:26)";
        assert_eq!(with_transport(play).unwrap(),
                   "col(header:60, transport:44, row(pads:*, col(kits:*, level:96, timing:232):460):*, hint:26)");
        assert_eq!(with_transport("col(log:*)"), None);
    }

    #[test]
    fn the_clips_page_fits_the_screen() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        assert_eq!(ui.rect_of(TRANSPORT).map(|r| (r.y, r.h)), Some((84, 44)));
        // Five rows over the morph bar (the wheel scrolls the rest).
        assert!(
            page.visible_rows(&ui) >= 5,
            "{} {:?}",
            page.visible_rows(&ui),
            ui.rect_of(CLIPS)
        );
        let morph = ui.rect_of(MORPH).unwrap();
        assert_eq!(morph.h, 64);
        assert!(morph.w >= 800, "{morph:?}");
        // Five record settings are in view at once (58 px a row); the
        // sixth, Pressure, scrolls in when the row above it is picked (the
        // list keeps the picked row and the next one in view).
        let record = ui.rect_of(RECORD).unwrap();
        assert!(record.h - 14 * 2 - 34 >= 58 * 5, "{record:?}");
        let list = ui.get_mut::<OptionList>(RECORD).unwrap();
        list.rows = (0..RECORD_ROWS.len())
            .map(|k| OptionRow {
                name: RECORD_ROWS[k].to_string(),
                value: String::new(),
                ..OptionRow::default()
            })
            .collect();
        assert!(list.row_rects(record, 5).is_none());
        list.selected = 3;
        assert!(list.row_rects(record, 5).is_none(), "no jump yet");
        list.selected = 4;
        assert!(list.row_rects(record, 5).is_some());
        // Three takes and the buttons.
        let takes = ui.rect_of(TAKES).unwrap();
        assert!(takes.h - 14 * 2 - 34 - 48 - 8 >= 3 * 38, "{takes:?}");
    }

    /// Jay's option E (layout spec 4c): Play's Q-Link column at Arrange's
    /// right edge, from under the header to the hint line, in every view;
    /// the bar ends short of it, Record (or Edit) and Takes narrow to
    /// 250 px, and the grid, the song and the note editor keep their width.
    #[test]
    fn the_qlink_column_runs_down_arranges_right_edge() {
        use mpfree_ui::geom::Rect;
        use mpfree_ui::qlink::COLUMN_WIDTH;
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Arrange"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        // The app's column is there first: add_to keeps it.
        let mut mine = QLinkColumn::new();
        mine.title = "Q-Links · Arrange".into();
        ui.add(QLINKS, mine);
        TimePage::add_to(&mut ui).unwrap();
        assert_eq!(
            ui.get::<QLinkColumn>(QLINKS).unwrap().title,
            "Q-Links · Arrange"
        );
        ui.show(CLIPS_PAGE).unwrap();
        for (layout, body, side) in [
            (CLIPS_LAYOUT, CLIPS, RECORD),
            (SONG_LAYOUT, SONG, RECORD),
            (CLIPS_CARD_LAYOUT, CLIPS, CLIP_CARD),
            (SONG_CARD_LAYOUT, SONG, CLIP_CARD),
            (CLIPS_MAKE_LAYOUT, CLIPS, MAKE_SHEET),
            (SONG_MAKE_LAYOUT, SONG, MAKE_SHEET),
            (CLIPS_STYLE_LAYOUT, CLIPS, STYLE_SHEET),
            (SONG_STYLE_LAYOUT, SONG, STYLE_SHEET),
            (EDIT_DRAWER_LAYOUT, NOTES, EDIT_OPTIONS),
        ] {
            ui.set_layout(CLIPS_PAGE, layout).unwrap();
            // Where Play has it (play_page's frame test): x 1140, y 84.
            assert_eq!(
                ui.rect_of(QLINKS),
                Some(Rect::new(1140, 84, COLUMN_WIDTH, 666)),
                "{layout}"
            );
            assert_eq!(ui.rect_of(TRANSPORT), Some(Rect::new(12, 84, 1116, 44)));
            let s = ui.rect_of(side).unwrap();
            assert_eq!((s.x, s.w), (878, SIDE_W), "{layout}");
            // The Make a phrase sheet runs down the whole column: no takes.
            let takes = (side != MAKE_SHEET && side != STYLE_SHEET).then_some((878, SIDE_W));
            assert_eq!(ui.rect_of(TAKES).map(|t| (t.x, t.w)), takes, "{layout}");
            let b = ui.rect_of(body).unwrap();
            assert_eq!((b.x, b.right()), (12, 866), "{layout}");
        }
        // The note editor with its drawer closed: the roll across the
        // page, up to the Q-Link column.
        ui.set_layout(CLIPS_PAGE, EDIT_LAYOUT).unwrap();
        let b = ui.rect_of(NOTES).unwrap();
        assert_eq!((b.x, b.right()), (12, 1128));
        assert_eq!(ui.rect_of(EDIT_OPTIONS), None);
        // Without the app, the samples get an empty column.
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Arrange"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        TimePage::add_to(&mut ui).unwrap();
        assert!(ui.get::<QLinkColumn>(QLINKS).unwrap().cards[0].is_free());
    }

    /// Every record setting, Edit option and take button reads whole in
    /// the narrowed panel (nothing is cut short with an ellipsis).
    #[test]
    fn the_narrow_panels_fit_their_words() {
        use mpfree_ui::FontId;
        let mut fonts = Fonts::embedded();
        // The row is 222 px inside the panel's padding; its value box 119.
        let (name_w, value_w) = (222 - 119 - 20, 119 - 2 * 22);
        let px = Theme::dark().size_small + 1.0;
        let st = state();
        let mut values: Vec<String> = record_rows(&st).into_iter().map(|r| r.value).collect();
        values.extend(["Overdub", "Replace", "1 bar", "16–32", "300.0"].map(String::from));
        let keys = crate::sample::keys_clip();
        let mut ed = crate::editor::Editor::new(0, 0, &keys);
        values.extend(edit_rows(&ed, &keys).into_iter().map(|r| r.value));
        ed.sel = vec![0, 1];
        values.extend(edit_rows(&ed, &keys).into_iter().map(|r| r.value));
        // Fit to scale's (plan 2B.1).
        values.extend(["no scale", "none off", "127 off"].map(String::from));
        // The chord tools' (plan 2B.2), at their longest.
        values.extend(
            [
                "pick notes",
                "no chord",
                "↓ 9990t",
                "C#mMaj7",
                "C#m7b5",
                "C#m/G#",
                "close",
                "open",
                // A tool open (plan 2B.3).
                "on",
            ]
            .map(String::from),
        );
        // The clip settings card, at its longest words.
        let mut card = ClipSettings {
            speed: Speed::new(1, 8).unwrap(),
            swing: Swing::new(PPQ / 2, 75),
            quantise: Quantise::new(PPQ / 6, 100),
            follow: Some(Follow {
                bars: MAX_FOLLOW_BARS,
                action: FollowAction::Previous,
                other: FollowAction::Stop,
                chance: 75,
            }),
            launch: None,
            legato: true,
            once: false,
            program: Some(ClipProgram {
                program: 127,
                bank: Some(ClipProgram::MAX_BANK),
            }),
            // Its longest word.
            direction: Direction::PingPong,
            step: mpfree_seq::steps::DEFAULT_SIZE,
            scale: None,
        };
        values.extend(card_rows(CARD_STEP, card).into_iter().map(|r| r.value));
        values.extend(card_rows(MAX_CLIP_TICKS, card).into_iter().map(|r| r.value));
        values.extend(
            card_rows(999 * CARD_STEP, card)
                .into_iter()
                .map(|r| r.value),
        );
        values.extend(
            card_rows(MAX_CLIP_TICKS - 1, card)
                .into_iter()
                .map(|r| r.value),
        );
        card.swing = Swing::new(PPQ / 6, 75);
        values.extend(
            card_rows(1001 * CARD_STEP, card)
                .into_iter()
                .map(|r| r.value),
        );
        card.speed = Speed::new(4, 3).unwrap();
        card.quantise = Quantise::new(PPQ / 3, 100);
        card.swing = None;
        card.follow = None;
        card.legato = false;
        card.once = true;
        values.extend(card_rows(CARD_STEP, card).into_iter().map(|r| r.value));
        // Every launch quantise, the project's and a clip's own.
        for g in LAUNCH_GRIDS {
            card.launch = Some(g);
            values.extend(card_rows(CARD_STEP, card).into_iter().map(|r| r.value));
        }
        // The Mutate row (S6) at its longest.
        values.extend(["original", "variant", "—"].map(String::from));
        // Continue's rows (composing tools, phase 1 step 2), each value
        // at its longest.
        let mut set = crate::editor::compose::ContinueSet::default();
        for row in 0..crate::editor::compose::CONTINUE_ROWS.len() {
            for d in [-9, 9] {
                set.step(row, d, &[]);
                values.push(set.value(row, &[]));
            }
            set.step(row, 1, &[]);
            values.push(set.value(row, &[]));
        }
        values.extend(["no tune", "no notes"].map(String::from));
        for name in RECORD_ROWS
            .iter()
            .chain(crate::editor::EDIT_ROWS.iter())
            .chain(crate::editor::compose::CONTINUE_ROWS.iter())
            .chain(CLIP_ROWS.iter())
        {
            let w = fonts.measure(FontId::Bold, px, name);
            assert!(w <= name_w, "{name}: {w} > {name_w}");
        }
        for v in &values {
            let w = fonts.measure(FontId::Bold, px, v);
            assert!(w <= value_w, "{v}: {w} > {value_w}");
        }
    }

    /// The note editor gets each note's pressure, and only the runs its
    /// notes name (an erased note's run stays in the clip for undo).
    #[test]
    fn the_note_editor_gets_each_notes_pressure() {
        let point = |at, value| mpfree_seq::TouchPoint { at, value };
        let mut c = Clip::new("pressed", BAR);
        let gone = c.add_touch(&[point(0, 1), point(5, 2)]).unwrap();
        let kept = c.add_touch(&[point(0, 90), point(40, 30)]).unwrap();
        assert!(gone.len == 2);
        c.add_note(Note::new(0, 240, 1, 100)).unwrap();
        c.add_note(Note {
            touch: kept,
            ..Note::new(480, 240, 2, 100)
        })
        .unwrap();
        let e = EditInfo {
            editor: crate::editor::Editor::new(0, 0, &c),
            clip: c.into(),
            rev: 0,
            track_name: "Drums".into(),
            bar: BAR,
            answer: EditAnswer::default(),
            locks: Default::default(),
            ghosts: Default::default(),
        };
        let g = note_grid(&e, &state());
        assert_eq!(g.touch, [point(0, 90), point(40, 30)]);
        assert!(g.touch_of(0).is_empty());
        assert_eq!(g.touch_of(1), &g.touch[..]);
    }

    /// Piano roll plan 0C.1: while the note editor's clip (one shared
    /// copy) and the editor stay the same, an update moves only the roll's
    /// playhead and builds none of its notes again; a new copy builds it.
    /// The note editor's view is the screen's: kept for each clip and
    /// found again on coming back, Snap Auto's division asked for when the
    /// zoom changes it (once, with no finger down), and the Edit button
    /// opens and closes the drawer of Edit options and takes (piano roll
    /// plan 1A.1).
    #[test]
    fn the_note_editor_keeps_each_clips_view_and_its_drawer() {
        use crate::editor::{EDIT_BUTTON, SNAP_AUTO};
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        let info = |slot: usize, bars: Tick| {
            let mut c = Clip::new("A", bars * BAR);
            c.add_note(Note::new(0, 240, 60, 100)).unwrap();
            c.add_note(Note::new(BAR, 240, 64, 100)).unwrap();
            Box::new(EditInfo {
                editor: crate::editor::Editor::new(0, slot, &c),
                clip: c.into(),
                rev: 1,
                track_name: "Keys".into(),
                bar: BAR,
                answer: EditAnswer::default(),
                locks: Default::default(),
                ghosts: Default::default(),
            })
        };
        st.edit = Some(info(0, 4));
        page.update(&mut ui, &st);
        assert_eq!(
            ui.layout_of(CLIPS_PAGE),
            Some(EDIT_LAYOUT),
            "the drawer starts closed"
        );
        let r = ui.rect_of(NOTES).unwrap();
        let g = ui.get::<NoteGrid>(NOTES).unwrap();
        let opened = g.open_view(r);
        assert_eq!(g.view, opened, "the clip's opening view");
        assert!(g.view.row_h >= 40);
        // Two of its four bars across the wide roll: a 1/16 is 32 px,
        // Snap Auto's own; nothing to ask.
        assert!(page.take_asks().is_empty());
        // Zoomed out to the whole clip (as a pinch would, fingers up):
        // Snap Auto asks for a 1/8 once, however many updates come before
        // the answer.
        let zoomed = RollView {
            ppt: g.view.ppt / 2.0,
            ..g.view
        };
        ui.get_mut::<NoteGrid>(NOTES).unwrap().view = zoomed;
        page.update(&mut ui, &st);
        page.update(&mut ui, &st);
        assert_eq!(
            page.take_asks(),
            vec![TimeAction::Edit(EditAction::AutoSnap(4))],
            "a 1/8, asked once"
        );
        st.edit.as_mut().unwrap().editor.auto = 4;
        page.update(&mut ui, &st);
        assert!(page.take_asks().is_empty());
        // Another clip opens on its own view; coming back finds the zoom
        // as it was left.
        st.edit = Some(info(1, 16));
        page.update(&mut ui, &st);
        let other = ui.get::<NoteGrid>(NOTES).unwrap().view;
        assert_ne!(other, zoomed);
        st.edit = Some(info(0, 4));
        st.edit.as_mut().unwrap().editor.auto = 4;
        page.update(&mut ui, &st);
        assert_eq!(ui.get::<NoteGrid>(NOTES).unwrap().view, zoomed);
        // A fixed snap asks nothing.
        st.edit.as_mut().unwrap().editor.snap = 6;
        ui.get_mut::<NoteGrid>(NOTES).unwrap().view = opened;
        page.update(&mut ui, &st);
        assert!(page.take_asks().is_empty());
        st.edit.as_mut().unwrap().editor.snap = SNAP_AUTO;
        // Edit opens the drawer (the page's own, nothing for the app) and
        // closes it again.
        let edit = ev(NOTES, EventKind::Tab(EDIT_BUTTON));
        assert_eq!(action(&edit, &st), None);
        assert!(page.page_tracks(&edit, &st));
        page.update(&mut ui, &st);
        assert_eq!(ui.layout_of(CLIPS_PAGE), Some(EDIT_DRAWER_LAYOUT));
        assert!(ui.get::<NoteGrid>(NOTES).unwrap().drawer, "Edit lit");
        let options = ui.get::<OptionList>(EDIT_OPTIONS).unwrap();
        assert_eq!(options.buttons[0].label, "Correct all");
        assert_eq!(
            action(&ev(EDIT_OPTIONS, EventKind::Toggle(0)), &st),
            Some(TimeAction::Edit(EditAction::Correct))
        );
        assert!(!page.open_drawer(&ui), "open already");
        assert!(page.page_tracks(&edit, &st));
        page.update(&mut ui, &st);
        assert_eq!(ui.layout_of(CLIPS_PAGE), Some(EDIT_LAYOUT));
        assert!(page.open_drawer(&ui), "the wheel opens it");
        // The view moving is the page's too.
        let moved = ev(
            NOTES,
            EventKind::View {
                start: 0.0,
                frames_per_px: 4.0,
            },
        );
        assert_eq!(action(&moved, &st), None);
        assert!(page.page_tracks(&moved, &st));
    }

    /// The note inspector's cards (plan 1C.1): a value for one note, a
    /// range for several; none with nothing picked. The cursor shows on
    /// the roll, which scrolls to keep it on screen when the wheel moves
    /// it, and SHIFT + – / + zoom the roll in time.
    #[test]
    fn the_note_inspector_cards_and_the_cursor() {
        let mut c = Clip::new("A", 16 * BAR);
        c.holds = mpfree_seq::Holds::Notes;
        c.add_note(Note::new(0, 240, 60, 80)).unwrap();
        c.add_note(Note::new(BAR + 480, 960, 67, 104)).unwrap();
        c.add_note(Note::new(2 * BAR, 37, 64, 90)).unwrap();
        let mut e = EditInfo {
            editor: crate::editor::Editor::new(0, 0, &c),
            clip: c.into(),
            rev: 1,
            track_name: "Keys".into(),
            bar: BAR,
            answer: EditAnswer::default(),
            locks: Default::default(),
            ghosts: Default::default(),
        };
        assert_eq!(note_cards(&e), None, "nothing picked");
        e.editor.sel = vec![1];
        let (head, cards) = note_cards(&e).unwrap();
        assert_eq!(head, "1 note");
        let names: Vec<&str> = cards.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Position", "Length", "Semitones", "Velocity"]);
        let values: Vec<&str> = cards.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(values, ["2.1.480", "1/4", "G3", "104"]);
        e.editor.sel = vec![0, 1, 2];
        let (head, cards) = note_cards(&e).unwrap();
        assert_eq!(head, "3 notes · C", "the chord they make (plan 2B.2)");
        let values: Vec<&str> = cards.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(values, ["1.1–3.1", "37t–1/4", "C3–G3", "80–104"]);
        // The cursor on the roll, kept on screen.
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        st.edit = Some(Box::new(e));
        page.update(&mut ui, &st);
        assert!(page.drawer_closed(&ui));
        let g = ui.get::<NoteGrid>(NOTES).unwrap();
        assert_eq!(g.cursor, Some(0));
        let (start, shown) = (g.view.start, g.view.ticks_shown(ui.rect_of(NOTES).unwrap()));
        let far = start + shown + 2 * BAR;
        st.edit.as_mut().unwrap().editor.cursor = far;
        page.update(&mut ui, &st);
        let g = ui.get::<NoteGrid>(NOTES).unwrap();
        assert_eq!(g.cursor, Some(far));
        assert!(
            g.view.start > start && far < g.view.start + shown,
            "scrolled to it"
        );
        // SHIFT + +: closer in, about the cursor.
        let ppt = g.view.ppt;
        assert!(page.zoom_time(&mut ui, 1));
        let g = ui.get::<NoteGrid>(NOTES).unwrap();
        assert!(g.view.ppt > ppt);
        let shown = g.view.ticks_shown(ui.rect_of(NOTES).unwrap());
        assert!((g.view.start..g.view.start + shown).contains(&far));
        // The list has no cursor.
        st.edit.as_mut().unwrap().editor.lens = Lens::List;
        page.update(&mut ui, &st);
        assert_eq!(ui.get::<NoteGrid>(NOTES).unwrap().cursor, None);
    }

    /// With Follow on, the roll pages along with the playhead; off, it
    /// stays.
    #[test]
    fn the_roll_follows_the_playhead_when_asked() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        let mut c = Clip::new("A", 16 * BAR);
        c.add_note(Note::new(0, 240, 1, 100)).unwrap();
        st.edit = Some(Box::new(EditInfo {
            editor: crate::editor::Editor::new(0, 0, &c),
            clip: c.into(),
            rev: 1,
            track_name: "Drums".into(),
            bar: BAR,
            answer: EditAnswer::default(),
            locks: Default::default(),
            ghosts: Default::default(),
        }));
        page.update(&mut ui, &st);
        let start = ui.get::<NoteGrid>(NOTES).unwrap().view.start;
        assert_eq!(start, 0, "two bars from the first note");
        let e = st.edit.clone().unwrap();
        // Play on past the two bars on screen: no Follow, no paging.
        while note_playhead(&e, &st).is_some_and(|t| t < 3 * BAR) {
            st.transport.position += 960;
        }
        page.update(&mut ui, &st);
        assert_eq!(ui.get::<NoteGrid>(NOTES).unwrap().view.start, 0);
        ui.get_mut::<NoteGrid>(NOTES).unwrap().view.follow = true;
        st.transport.position += 960;
        page.update(&mut ui, &st);
        let v = ui.get::<NoteGrid>(NOTES).unwrap().view;
        let at = note_playhead(&e, &st).unwrap();
        assert!(v.start <= at && at < v.start + 2 * BAR + 960, "{v:?} {at}");
    }

    #[test]
    fn the_note_editor_moves_only_its_playhead_while_its_clip_is_the_same() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        let mut c = Clip::new("A", 2 * BAR);
        c.add_note(Note::new(0, 240, 1, 100)).unwrap();
        st.edit = Some(Box::new(EditInfo {
            editor: crate::editor::Editor::new(0, 0, &c),
            clip: c.into(),
            rev: 1,
            track_name: "Drums".into(),
            bar: BAR,
            answer: EditAnswer::default(),
            locks: Default::default(),
            ghosts: Default::default(),
        }));
        page.update(&mut ui, &st);
        let e = st.edit.clone().unwrap();
        let head = note_playhead(&e, &st);
        assert!(head.is_some(), "track 1's clip plays");
        assert_eq!(ui.get::<NoteGrid>(NOTES).unwrap().playhead, head);
        // Marked, to see whether the roll is built again.
        ui.get_mut::<NoteGrid>(NOTES).unwrap().title = "kept".into();
        st.transport.position += 120;
        page.update(&mut ui, &st);
        let g = ui.get::<NoteGrid>(NOTES).unwrap();
        assert_eq!(g.title, "kept", "not built again");
        assert_eq!(g.playhead, note_playhead(&e, &st));
        assert_ne!(g.playhead, head, "the playhead moved");
        // The same notes in a new copy (the clip changed and changed
        // back): built again.
        let e = st.edit.as_mut().unwrap();
        e.clip = Arc::new((*e.clip).clone());
        page.update(&mut ui, &st);
        assert!(ui
            .get::<NoteGrid>(NOTES)
            .unwrap()
            .title
            .starts_with("Edit · A"));
        // Closed and opened again: built again too.
        let open = st.edit.take();
        page.update(&mut ui, &st);
        ui.get_mut::<NoteGrid>(NOTES).unwrap().title = "kept".into();
        st.edit = open;
        page.update(&mut ui, &st);
        assert_ne!(ui.get::<NoteGrid>(NOTES).unwrap().title, "kept");
    }

    #[test]
    fn cells_show_what_each_clip_is_doing() {
        let mut st = state();
        let c = cell_of(&st, 0, 0);
        assert_eq!(
            (c.state, c.detail.as_str(), c.foot.as_str()),
            (CellState::Playing, "2 bars", "")
        );
        assert_eq!(cell_of(&st, 1, 0).foot, "1 note");
        // Bar 5 beat 2 of a 2-bar clip from bar 1: an eighth of the way round.
        assert_eq!(c.progress, 0.125);
        let q = cell_of(&st, 1, 1);
        assert_eq!(
            (q.state, q.detail.as_str(), q.foot.as_str()),
            (CellState::Queued, "7/16", "from 6.1")
        );
        assert_eq!(cell_of(&st, 1, 0).state, CellState::Stopped);
        assert_eq!(cell_of(&st, 0, 2).state, CellState::Empty);
        st.launches.push(Launch {
            track: 2,
            slot: 0,
            origin: 0,
            start: 0,
        });
        assert_eq!(cell_of(&st, 2, 0).state, CellState::Playing);
        st.transport.state = State::Recording;
        let r = cell_of(&st, 2, 0);
        assert_eq!(
            (r.state, r.detail.as_str()),
            (CellState::Recording, "recording")
        );
        assert_eq!(
            cell_of(&st, 0, 0).state,
            CellState::Playing,
            "only the armed track records"
        );
    }

    /// Loop tracks (part 3): a loop cell draws its loop's wave in the
    /// detail's place with its length at the foot, its pass on the wave
    /// (no ring), "overdub" while it takes a layer; the header has a wave
    /// mark; the title counts the loops; each loop's place names its
    /// track and row (or scene).
    #[test]
    fn loop_cells_show_their_wave_and_loops_are_named_by_place() {
        let mut st = crate::sample::loops_playing().unwrap().state();
        st.loops.loops[1].wave = [200; crate::widgets::CELL_WAVE];
        st.loops.in_use = 4;
        st.loops.memory_pct = 38;
        let stopped = cell_of(&st, 3, 0);
        assert_eq!(
            (
                stopped.state,
                stopped.detail.as_str(),
                stopped.foot.as_str()
            ),
            (CellState::Stopped, "2 bars", "")
        );
        assert_eq!(stopped.wave, Some([0; crate::widgets::CELL_WAVE]));
        // A copy's layers copying in (part 6): its foot says so.
        st.loops.loops[0].copying = true;
        assert_eq!(cell_of(&st, 3, 0).foot, "copying in");
        st.loops.loops[0].copying = false;
        let playing = cell_of(&st, 4, 1);
        assert_eq!(playing.state, CellState::Playing);
        assert_eq!(playing.wave, Some([200; crate::widgets::CELL_WAVE]));
        st.loops.loops[1].overdubbing = true;
        let over = cell_of(&st, 4, 1);
        assert_eq!(
            (over.state, over.detail.as_str()),
            (CellState::Recording, "overdub")
        );
        assert_eq!(cell_of(&st, 0, 0).wave, None, "a note clip has no wave");
        let tracks = grid_tracks(&st);
        assert_eq!(
            tracks.iter().map(|t| t.loops).collect::<Vec<_>>(),
            [false, false, false, true, true, true]
        );
        // The places: the first row a loop is in, by its scene's name.
        st.scenes = vec![String::new(), "Verse".into()];
        let places = loop_places(&st);
        let full = |n: usize| places[n].as_ref().map(|p| p.full(n));
        assert_eq!(full(0).as_deref(), Some("Loops A · Loop 1 · row 1"));
        assert_eq!(full(1).as_deref(), Some("Loops B · Loop 2 · Verse"));
        assert_eq!(full(3).as_deref(), Some("Loops A · Loop 4 · row 4"));
        assert_eq!(places[4], None);
        let three = places[2].as_ref().unwrap();
        assert_eq!((three.track, three.slot), (4, 2));
        // The gauge, in the legend.
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        page.update(&mut ui, &st);
        let title = &ui.get::<ClipGrid>(CLIPS).unwrap().title;
        assert_eq!(title, "Loops 4 of 8 · memory 38 %");
        // Show in Clips: a cell off the rows shown brings its row.
        page.show_cell(4, 14);
        assert_eq!(page.first_track, 0);
        assert!(page.first_row > 0 && page.first_row <= 14);
    }

    /// A tap on an empty Loop cell arms it and a second lets go; the
    /// armed cell says what REC does there as the first pass goes, and
    /// the REC chip names it (loop-tracks.md part 4).
    #[test]
    fn an_empty_loop_cell_arms_and_says_what_rec_does() {
        let mut st = crate::sample::loops_playing().unwrap().state();
        let tap = |st: &TimeState, track: usize, slot: usize| {
            action(&ev(CLIPS, EventKind::Select(track * ROWS + slot)), st)
        };
        assert!(st.clip(3, 5).is_none() && st.tracks[3].loops.is_some());
        assert_eq!(tap(&st, 3, 5), Some(TimeAction::LoopCell(Some((3, 5)))));
        assert!(
            matches!(tap(&st, 0, 5), Some(TimeAction::NewClip { .. })),
            "a note track's empty cell makes a clip"
        );
        assert_eq!(cell_of(&st, 3, 5).state, CellState::Empty);
        st.loop_cell = Some((3, 5));
        assert_eq!(tap(&st, 3, 5), Some(TimeAction::LoopCell(None)));
        let said = |st: &TimeState| {
            let c = cell_of(st, 3, 5);
            assert_eq!(c.state, CellState::Armed);
            format!("{} / {} / {}", c.name, c.detail, c.foot)
        };
        assert_eq!(said(&st), "Armed / REC here / ");
        st.loops.waiting = true;
        assert_eq!(said(&st), "Ready / bar line / or PLAY");
        st.loops.recording = true;
        assert_eq!(said(&st), "REC / REC ends / the pass");
        st.scenes = vec![String::new(); 6];
        st.scenes[5] = "Drop".into();
        assert_eq!(loop_cell_name(&st).as_deref(), Some("Loops A · Drop"));
        st.scenes[5].clear();
        assert_eq!(loop_cell_name(&st).as_deref(), Some("Loops A · row 6"));
    }

    #[test]
    fn taps_on_the_grid_launch_stop_make_and_arm() {
        let st = state();
        let tap = |track: usize, slot: usize| {
            action(&ev(CLIPS, EventKind::Select(track * ROWS + slot)), &st)
        };
        assert_eq!(tap(0, 0), Some(TimeAction::StopClip(0)));
        assert_eq!(tap(0, 1), Some(TimeAction::Launch { track: 0, slot: 1 }));
        assert_eq!(
            tap(1, 1),
            Some(TimeAction::StopClip(1)),
            "a queued clip stops too"
        );
        assert_eq!(tap(2, 5), Some(TimeAction::NewClip { track: 2, slot: 5 }));
        assert_eq!(tap(3, 0), None, "no such track");
        assert_eq!(
            action(&ev(CLIPS, EventKind::Tab(0)), &st),
            Some(TimeAction::Pick(0))
        );
        assert_eq!(
            action(&ev(CLIPS, EventKind::Toggle(4)), &st),
            Some(TimeAction::LaunchScene(4))
        );
        assert_eq!(
            action(&ev(CLIPS, EventKind::Toggle(ROWS)), &st),
            Some(TimeAction::StopAll)
        );
    }

    /// Perform's Capture scene (the Arrange refactor's 5.3): lit while a
    /// track plays a clip or the song plays, and a tap on it is
    /// CaptureScene; faint, a tap does nothing. Make's grid has none.
    #[test]
    fn performs_capture_scene_keeps_what_plays_while_something_does() {
        let mut st = state();
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let tap = |st: &TimeState| action(&ev(CLIPS, EventKind::Toggle(ROWS + CAPTURE)), st);
        assert!(capture_can(&st), "two tracks play");
        assert_eq!(tap(&st), Some(TimeAction::CaptureScene));
        page.update(&mut ui, &st);
        assert!(!ui.get::<ClipGrid>(CLIPS).unwrap().can_capture, "Make");
        st.perform = true;
        page.update(&mut ui, &st);
        assert!(ui.get::<ClipGrid>(CLIPS).unwrap().can_capture);
        st.launches.clear();
        assert!(!capture_can(&st));
        assert_eq!(tap(&st), None, "faint: nothing to keep");
        page.update(&mut ui, &st);
        assert!(!ui.get::<ClipGrid>(CLIPS).unwrap().can_capture);
        st.song_on = true;
        assert!(capture_can(&st), "the song plays");
        assert_eq!(tap(&st), Some(TimeAction::CaptureScene));
    }

    /// MIDI Learn on the Clips grid (sequencer S5, Jay's card of 18 Nov):
    /// SHIFT + a tap names a cell, a scene, Stop all, Back to song,
    /// Perform's Capture scene or a track's arm, and a learned press later
    /// does what that tap does then.
    #[test]
    fn a_learned_press_does_what_a_tap_on_the_grid_does() {
        let st = state();
        let learn = |kind| learn_target(&ev(CLIPS, kind), &st).map(|a| (a.owner, a.param));
        let c = |id| Some((Owner::Clips, id));
        assert_eq!(learn(EventKind::Select(ROWS + 2)), c(clips::clip(1, 2)));
        assert_eq!(
            learn(EventKind::Select(2 * ROWS + 5)),
            c(clips::clip(2, 5)),
            "empty"
        );
        assert_eq!(learn(EventKind::Select(3 * ROWS)), None, "no such track");
        assert_eq!(learn(EventKind::Toggle(4)), c(clips::scene(4)));
        assert_eq!(learn(EventKind::Toggle(ROWS)), c(clips::STOP_ALL));
        assert_eq!(
            learn(EventKind::Toggle(ROWS + BACK_TO_SONG)),
            c(clips::BACK_TO_SONG)
        );
        assert_eq!(
            learn(EventKind::Toggle(ROWS + CAPTURE)),
            c(clips::CAPTURE_SCENE)
        );
        assert_eq!(learn(EventKind::Tab(1)), c(clips::arm(1)));
        // SHIFT keeps its job elsewhere: + Track, the pager, the sound line.
        assert_eq!(learn(EventKind::Toggle(ROWS + ADD_TRACK)), None);
        assert_eq!(learn(EventKind::Toggle(ROWS + PAGE_ON)), None);
        assert_eq!(learn(EventKind::Tab(crate::widgets::SOUND_TAP + 1)), None);
        assert_eq!(
            learn_target(&ev(RECORD, EventKind::Tab(1)), &st),
            None,
            "only the grid"
        );
        // Each press is the tap now, wherever the cell's clip stands.
        let tap = |track: usize, slot: usize| {
            action(&ev(CLIPS, EventKind::Select(track * ROWS + slot)), &st)
        };
        for (t, r) in [(0, 0), (0, 1), (1, 1), (2, 5)] {
            assert_eq!(clips_press(&st, Clips::Clip(t as u8, r as u8)), tap(t, r));
        }
        assert_eq!(clips_press(&st, Clips::Clip(3, 0)), None);
        assert_eq!(
            clips_press(&st, Clips::Scene(4)),
            Some(TimeAction::LaunchScene(4))
        );
        assert_eq!(clips_press(&st, Clips::StopAll), Some(TimeAction::StopAll));
        assert_eq!(
            clips_press(&st, Clips::BackToSong),
            Some(TimeAction::BackToSong)
        );
        assert_eq!(
            clips_press(&st, Clips::CaptureScene),
            Some(TimeAction::CaptureScene)
        );
        assert_eq!(
            clips_press(&st, Clips::Stop(1)),
            Some(TimeAction::StopClip(1))
        );
        assert_eq!(
            clips_press(&st, Clips::Arm(1)),
            action(&ev(CLIPS, EventKind::Tab(1)), &st)
        );
        assert_eq!(clips_press(&st, Clips::Arm(3)), None);
        assert_eq!(clips_press(&st, Clips::Mute(0)), None, "the app's");
        assert_eq!(clips_press(&st, Clips::Solo(0)), None, "the app's");
    }

    /// Scene banks (the Arrange refactor's 5.6): the grid shows rows 1
    /// to 64; the bank chip lights the banks that hold a clip or a named
    /// scene, and a tap on a bank brings its first row to the top (bank
    /// D as far as the last rows allow). A row's number and a cell past
    /// row 16 launch as any other. The title says which rows show.
    #[test]
    fn the_bank_chip_scrolls_the_grid_to_a_bank_of_64_rows() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = crate::sample::playing().unwrap().state();
        assert_eq!(st.tracks[0].slots.len(), ROWS, "the grid's 64 rows");
        st.tracks[2].slots[20] = st.tracks[0].slots[0].clone();
        st.scenes.resize(50, String::new());
        st.scenes[49] = "Encore".into();
        page.update(&mut ui, &st);
        let g = ui.get::<ClipGrid>(CLIPS).unwrap();
        assert_eq!(g.slots, ROWS);
        assert_eq!(g.banks, [true, true, false, true]);
        assert_eq!(g.title, "Rows 1–8");
        assert!(page.page_tracks(&ev(CLIPS, EventKind::Toggle(ROWS + BANK + 1)), &st));
        page.update(&mut ui, &st);
        assert_eq!(page.first_row, 16);
        assert_eq!(ui.get::<ClipGrid>(CLIPS).unwrap().title, "Rows 17–24");
        assert!(page.page_tracks(&ev(CLIPS, EventKind::Toggle(ROWS + BANK + 3)), &st));
        page.update(&mut ui, &st);
        assert_eq!(page.first_row, 48);
        assert!(page.page_tracks(&ev(CLIPS, EventKind::Toggle(ROWS + BANK)), &st));
        assert_eq!(page.first_row, 0);
        // Past row 16, a cell and a row's number act as in bank A.
        assert_eq!(
            action(&ev(CLIPS, EventKind::Toggle(20)), &st),
            Some(TimeAction::LaunchScene(20))
        );
        assert_eq!(
            action(&ev(CLIPS, EventKind::Select(2 * ROWS + 20)), &st),
            Some(TimeAction::Launch { track: 2, slot: 20 })
        );
        assert_eq!(
            action(&ev(CLIPS, EventKind::Toggle(ROWS)), &st),
            Some(TimeAction::StopAll)
        );
        assert_eq!(
            learn_target(&ev(CLIPS, EventKind::Select(2 * ROWS + 20)), &st),
            Some(ParamAddr::new(Owner::Clips, clips::clip(2, 20)))
        );
    }

    /// More than 8 tracks (up to 128, Jay 14 Nov): the grid follows the
    /// armed track to its page, the pager steps 8 at a time within the
    /// tracks there are, and "+ Track" on the grid and the Song view adds
    /// one.
    #[test]
    fn the_grid_pages_through_24_tracks_and_plus_track_adds_one() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = crate::sample::big_playing(10).unwrap().state();
        assert_eq!(st.tracks.len(), 24);
        page.update(&mut ui, &st);
        assert_eq!(page.first_track, 8, "Vox chop's page");
        let g = ui.get::<ClipGrid>(CLIPS).unwrap();
        assert_eq!((g.first_track, g.can_add), (8, true));
        let on = ev(CLIPS, EventKind::Toggle(ROWS + PAGE_ON));
        let back = ev(CLIPS, EventKind::Toggle(ROWS + PAGE_BACK));
        assert_eq!(action(&on, &st), None, "the page's own");
        assert!(page.page_tracks(&on, &st));
        assert_eq!(page.first_track, 16);
        assert!(page.page_tracks(&on, &st));
        assert_eq!(page.first_track, 16, "the last page");
        page.update(&mut ui, &st);
        assert_eq!(
            page.first_track, 16,
            "paging away from the armed track stays"
        );
        assert!(page.page_tracks(&back, &st));
        assert!(page.page_tracks(&back, &st));
        assert!(page.page_tracks(&back, &st));
        assert_eq!(page.first_track, 0);
        assert!(!page.page_tracks(&ev(CLIPS, EventKind::Toggle(ROWS)), &st));
        // Arming a track off the page brings its page.
        st.armed = Some(21);
        page.update(&mut ui, &st);
        assert_eq!(page.first_track, 16);
        // Fewer tracks (an undone + Track, a new project): the page stays
        // within them.
        st.tracks.truncate(8);
        st.armed = Some(2);
        page.update(&mut ui, &st);
        assert_eq!(page.first_track, 0);
        assert_eq!(
            action(&ev(CLIPS, EventKind::Toggle(ROWS + ADD_TRACK)), &st),
            Some(TimeAction::AddTrack)
        );
        assert_eq!(
            action(&ev(CLIPS, EventKind::Toggle(ROWS + 1)), &st),
            Some(TimeAction::SongView(true))
        );
        assert_eq!(
            action(&ev(SONG, EventKind::Tab(crate::song::ADD_TRACK)), &st),
            Some(TimeAction::AddTrack)
        );
        assert!(song_lanes(&st, 0).can_add_track);
        assert_eq!(
            action(&ev(CLIPS, EventKind::Toggle(ROWS + BACK_TO_SONG)), &st),
            Some(TimeAction::BackToSong)
        );
    }

    #[test]
    fn a_frame_that_moves_the_playhead_or_scrolls_copies_no_block() {
        // Arrange refactor 2.4: while the placements are the same list, the
        // Song view keeps its blocks, so a long song costs a frame no more
        // than a short one; new placements are new blocks.
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        st.song_view = true;
        let place = |n: usize| -> Arc<[PlacementInfo]> {
            (0..n)
                .map(|k| PlacementInfo {
                    track: k % 3,
                    start: (k / 3) as Tick * BAR,
                    length: BAR,
                    name: format!("Clip {k}"),
                    unique: false,
                })
                .collect::<Vec<_>>()
                .into()
        };
        st.placements = place(4096);
        page.update(&mut ui, &st);
        let blocks = |ui: &Ui| ui.get::<SongLanes>(SONG).unwrap().blocks.as_ptr();
        let first = blocks(&ui);
        for k in 0..4 {
            st.transport.position += 960;
            page.update(&mut ui, &st);
            page.scroll(&mut ui, k);
            page.update(&mut ui, &st);
            let l = ui.get::<SongLanes>(SONG).unwrap();
            assert_eq!(l.first_bar, (0..=k).sum::<i32>() as i64);
            assert_eq!(l.playhead, Some(st.transport.position));
            assert!(
                std::ptr::eq(blocks(&ui), first),
                "frame {k} copied the blocks"
            );
        }
        // An equal list from the engine (a new state) is copied once, with
        // nothing to draw, and the frames after it copy nothing.
        let mut c = SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        st.placements = place(4096);
        page.update(&mut ui, &st);
        assert!(ui.paint(&mut c).is_empty(), "nothing changed on the screen");
        let second = blocks(&ui);
        assert!(!std::ptr::eq(second, first));
        st.transport.position += 960;
        page.update(&mut ui, &st);
        page.update(&mut ui, &st);
        assert!(std::ptr::eq(blocks(&ui), second));
        // A changed one shows.
        st.placements = place(5);
        page.update(&mut ui, &st);
        let l = ui.get::<SongLanes>(SONG).unwrap();
        assert_eq!(l.blocks.len(), 5);
        assert_eq!(l.blocks.get(4).map(|b| b.name.as_str()), Some("Clip 4"));
    }

    /// While the song plays, a frame that only moves the playhead draws
    /// the Song view's strips its line left and reached
    /// (`SongLanes::changed_parts`), and the screen is the one a whole
    /// paint of the same state gives, in a plain and a glowing theme; a
    /// scroll or a pick draws the view whole again.
    #[test]
    fn the_song_views_playhead_strips_paint_as_a_whole_frame_does() {
        for theme in [Theme::dark(), Theme::synthwave()] {
            let make = || {
                let mut ui = Ui::new(theme.clone(), Fonts::embedded(), 1280, 800);
                ui.add("header", Header::new(&["Play", "Clips"]));
                ui.add("hint", Label::new("", Tone::Dim, Align::Center));
                let page = TimePage::add_to(&mut ui).unwrap();
                ui.show(CLIPS_PAGE).unwrap();
                (ui, page)
            };
            let mut st = state();
            st.song_view = true;
            st.song_on = true;
            let place = |renamed: bool| -> Arc<[PlacementInfo]> {
                (0..24)
                    .map(|k| PlacementInfo {
                        track: k % 3,
                        start: (k / 3) as Tick * BAR,
                        length: BAR,
                        name: if renamed && k == 7 {
                            "Renamed".to_string()
                        } else {
                            format!("Clip {k}")
                        },
                        unique: k == 4,
                    })
                    .collect::<Vec<_>>()
                    .into()
            };
            st.placements = place(false);
            // The picture a fresh screen paints whole from `st`, its view
            // scrolled as the one under test.
            let whole = |st: &TimeState, first_bar: i64| {
                let (mut ui, mut page) = make();
                page.update(&mut ui, st);
                page.scroll(&mut ui, first_bar as i32);
                page.update(&mut ui, st);
                let mut c = SoftCanvas::new(1280, 800);
                ui.paint(&mut c);
                c.px
            };
            let (mut ui, mut page) = make();
            page.update(&mut ui, &st);
            let mut now = SoftCanvas::new(1280, 800);
            ui.paint(&mut now);
            let song = ui.rect_of(SONG).unwrap();
            for frame in 0..6 {
                let strips = match frame {
                    // A scroll, and a block renamed: the view whole.
                    3 => {
                        page.scroll(&mut ui, 1);
                        false
                    }
                    5 => {
                        st.placements = place(true);
                        false
                    }
                    _ => true,
                };
                // An eighth of a bar a frame: the clip waiting on track 2
                // starts (on bar 6) with the renamed block.
                st.transport.position += BAR / 8;
                page.update(&mut ui, &st);
                let drawn = ui.paint(&mut now);
                let on_song: Vec<mpfree_ui::Rect> = drawn
                    .iter()
                    .map(|d| d.intersect(song))
                    .filter(|d| !d.is_empty())
                    .collect();
                if strips {
                    assert!(
                        !on_song.is_empty() && on_song.iter().all(|d| d.w <= 6),
                        "{} frame {frame}: only the playhead's strips, not {on_song:?}",
                        theme.name
                    );
                } else {
                    assert!(on_song.contains(&song), "{} frame {frame}", theme.name);
                }
                assert!(
                    now.px == whole(&st, page.first_bar),
                    "{} frame {frame}: the strips paint what a whole frame does",
                    theme.name
                );
            }
        }
    }

    /// Several blocks (6.5): a box picks the blocks it touches; with
    /// several picked, a drag of one moves them all, and Split, Duplicate
    /// and Delete take them all while Make unique says why not. The zoom
    /// chip (6.4) is the page's own, and the wheel scrolls by it.
    #[test]
    fn several_picked_blocks_act_together_and_the_zoom_chip_is_the_pages() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        st.song_view = true;
        let block = |track, start, length| PlacementInfo {
            track,
            start,
            length,
            name: "B".into(),
            unique: false,
        };
        st.placements = vec![
            block(0, 0, 3840),
            block(0, 4 * 3840, 3840),
            block(1, 3840, 2 * 3840),
        ]
        .into();
        let song = |k: EventKind, st: &TimeState| action(&ev(SONG, k), st);
        // A box over both lanes from bar 2 to bar 5: the bass's block, and
        // the drums' second block it reaches into.
        assert_eq!(
            song(
                EventKind::Span {
                    index: crate::song::BOX + 1,
                    from: 3840,
                    to: 4 * 3840 + 10
                },
                &st
            ),
            Some(TimeAction::PickPlacements {
                set: vec![1, 2],
                add: false
            })
        );
        st.picks = vec![1, 2];
        assert_eq!(song_hint(&st), SONG_PICKS_HINT);
        // The drums' second block dragged a lane down and a bar back: both
        // go a lane down and a bar back.
        assert_eq!(
            song(
                EventKind::Place {
                    index: 1,
                    row: 1,
                    at: 3 * 3840
                },
                &st
            ),
            Some(TimeAction::MovePlacements {
                set: vec![1, 2],
                lanes: 1,
                by: -3840,
                copy: false
            })
        );
        // A block not picked moves alone.
        assert!(matches!(
            song(
                EventKind::Place {
                    index: 0,
                    row: 0,
                    at: 3840
                },
                &st
            ),
            Some(TimeAction::MovePlacement { index: 0, .. })
        ));
        assert_eq!(
            song(EventKind::Tab(0), &st),
            Some(TimeAction::SplitPlacements(vec![1, 2]))
        );
        assert_eq!(
            song(EventKind::Tab(DUPLICATE), &st),
            Some(TimeAction::DuplicatePlacements(vec![1, 2]))
        );
        assert_eq!(
            song(EventKind::Tab(5), &st),
            Some(TimeAction::DeletePlacements(vec![1, 2]))
        );
        assert_eq!(song(EventKind::Tab(4), &st), None);
        let why = why_not(&ev(SONG, EventKind::Tab(4)), &st).unwrap();
        assert!(why.starts_with("Make unique takes one block"), "{why}");
        // The lanes show the picks.
        page.update(&mut ui, &st);
        assert_eq!(ui.get::<SongLanes>(SONG).unwrap().picks, [1, 2]);
        // The zoom chip: the page's own; 64 bars shown, and the wheel
        // scrolls four bars a detent.
        let zoom = ev(SONG, EventKind::Tab(crate::song::ZOOM + 4));
        assert_eq!(action(&zoom, &st), None);
        assert!(page.page_tracks(&zoom, &st));
        assert_eq!(page.bars_shown(), 64);
        page.update(&mut ui, &st);
        assert_eq!(ui.get::<SongLanes>(SONG).unwrap().bars, 64);
        page.scroll(&mut ui, 1);
        assert_eq!(page.first_bar, 4);
        assert!(page.page_tracks(&ev(SONG, EventKind::Tab(BARS_BACK)), &st));
        assert_eq!(page.first_bar, 0);
        let zoom = ev(SONG, EventKind::Tab(crate::song::ZOOM));
        assert!(page.page_tracks(&zoom, &st));
        assert_eq!(page.bars_shown(), 4);
        page.scroll(&mut ui, 1);
        assert_eq!(page.first_bar, 1);
        // Another Song tab isn't the page's.
        assert!(!page.page_tracks(&ev(SONG, EventKind::Tab(0)), &st));
    }

    #[test]
    fn the_song_view_swaps_in_for_the_grid_and_its_buttons_act() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        assert_eq!(
            action(&ev(CLIPS, EventKind::Toggle(ROWS + 1)), &st),
            Some(TimeAction::SongView(true))
        );
        st.song_view = true;
        st.placements = vec![PlacementInfo {
            track: 0,
            start: 3840,
            length: 7680,
            name: "Boom".into(),
            unique: false,
        }]
        .into();
        page.update(&mut ui, &st);
        assert!(ui.rect_of(SONG).is_some() && ui.rect_of(CLIPS).is_none());
        let lanes = ui.get::<SongLanes>(SONG).unwrap();
        assert_eq!(
            (lanes.blocks.len(), lanes.legend.as_str()),
            (1, "3 bars · from bar 1")
        );
        page.scroll(&mut ui, 4);
        assert_eq!(page.first_bar, 4, "the wheel scrolls bars");
        let song = |k: EventKind, st: &TimeState| action(&ev(SONG, k), st);
        // The view chip's Clips (5.7); the bottom row's first place is
        // Split (phase 6): every block at the playhead with none picked.
        assert_eq!(
            song(EventKind::Tab(crate::song::CLIPS), &st),
            Some(TimeAction::SongView(false))
        );
        assert_eq!(
            song(EventKind::Tab(0), &st),
            Some(TimeAction::SplitPlacement(None))
        );
        assert_eq!(song(EventKind::Tab(DUPLICATE), &st), None, "nothing picked");
        assert_eq!(
            song(EventKind::Tab(1), &st),
            Some(TimeAction::SongMode(true))
        );
        assert_eq!(song(EventKind::Tab(2), &st), Some(TimeAction::ScenesToSong));
        assert_eq!(song(EventKind::Tab(3), &st), Some(TimeAction::SongToScenes));
        assert_eq!(song(EventKind::Tab(4), &st), None, "nothing selected");
        assert_eq!(
            song(EventKind::Select(0), &st),
            Some(TimeAction::SelectPlacement(Some(0)))
        );
        assert_eq!(
            song(EventKind::Select(NONE), &st),
            Some(TimeAction::SelectPlacement(None))
        );
        st.selected = Some(0);
        assert_eq!(
            song(EventKind::Tab(4), &st),
            Some(TimeAction::MakeUnique(0))
        );
        assert_eq!(
            song(EventKind::Tab(5), &st),
            Some(TimeAction::DeletePlacement(0))
        );
        // Phase 6: Split cuts the picked block, Duplicate copies it, its
        // start grip trims it and a slide moves what it plays.
        assert_eq!(
            song(EventKind::Tab(0), &st),
            Some(TimeAction::SplitPlacement(Some(0)))
        );
        assert_eq!(
            song(EventKind::Tab(DUPLICATE), &st),
            Some(TimeAction::DuplicatePlacement(0))
        );
        assert_eq!(
            song(
                EventKind::Place {
                    index: 0,
                    row: START_ROW,
                    at: 3840
                },
                &st
            ),
            Some(TimeAction::TrimPlacement {
                index: 0,
                start: 3840
            })
        );
        assert_eq!(
            song(
                EventKind::Place {
                    index: 0,
                    row: SLIDE_ROW,
                    at: -240
                },
                &st
            ),
            Some(TimeAction::SlidePlacement { index: 0, by: -240 })
        );
        // Rec song, on the Song view and on the grid.
        assert_eq!(
            song(EventKind::Tab(REC), &st),
            Some(TimeAction::RecSong(true))
        );
        st.song_rec = true;
        page.update(&mut ui, &st);
        assert!(ui.get::<SongLanes>(SONG).unwrap().rec);
        st.song_view = false;
        page.update(&mut ui, &st);
        assert!(ui.rect_of(CLIPS).is_some() && ui.rect_of(SONG).is_none());
        assert!(ui.get::<ClipGrid>(CLIPS).unwrap().song_rec);
        assert_eq!(
            action(&ev(CLIPS, EventKind::Toggle(ROWS + 2)), &st),
            Some(TimeAction::RecSong(false))
        );
    }

    /// The song list (S4): List on the Song view shows it in place of the
    /// grid and the morph; its title counts the steps, bars and time;
    /// each touch is its action or says why not; NEXT names the next
    /// step.
    /// Live shows B1: with a set open, the title lines have Show, and the
    /// Show view takes the place of the grid and the Record panel (layout
    /// spec 4p), over any of the three views.
    #[test]
    fn the_show_view_swaps_in_while_a_set_is_open() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        page.update(&mut ui, &st);
        assert!(!ui.get::<ClipGrid>(CLIPS).unwrap().show_button);
        // Show without a set changes nothing.
        page.show = true;
        page.update(&mut ui, &st);
        assert_eq!(ui.layout_of(CLIPS_PAGE), Some(CLIPS_LAYOUT));
        page.show = false;
        page.set_open = true;
        page.update(&mut ui, &st);
        assert!(ui.get::<ClipGrid>(CLIPS).unwrap().show_button);
        assert!(ui.get::<SongListView>(SONG_LIST).unwrap().show_button);
        st.song_view = true;
        page.update(&mut ui, &st);
        assert!(ui.get::<SongLanes>(SONG).unwrap().show_button);
        for (song, list) in [(false, false), (true, false), (false, true)] {
            (st.song_view, st.list_view) = (song, list);
            page.show = true;
            page.update(&mut ui, &st);
            assert_eq!(ui.layout_of(CLIPS_PAGE), Some(SHOW_LAYOUT));
            assert_eq!(
                ui.rect_of(SHOW),
                Some(mpfree_ui::Rect::new(12, 140, 1116, 610))
            );
            assert!(ui.rect_of(RECORD).is_none() && ui.rect_of(TAKES).is_none());
            assert!(ui.rect_of(TRANSPORT).is_some() && ui.rect_of(QLINKS).is_some());
            page.show = false;
            page.update(&mut ui, &st);
            assert_ne!(ui.layout_of(CLIPS_PAGE), Some(SHOW_LAYOUT));
        }
        // The set closed: Show goes from the title lines (the grid's as it
        // shows again: it is brought up to date only while on screen).
        page.set_open = false;
        (st.song_view, st.list_view) = (false, false);
        page.update(&mut ui, &st);
        assert!(!ui.get::<ClipGrid>(CLIPS).unwrap().show_button);
    }

    #[test]
    fn the_song_list_swaps_in_and_its_touches_act() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        st.song_view = true;
        assert_eq!(
            action(&ev(SONG, EventKind::Tab(crate::song::LIST)), &st),
            Some(TimeAction::ListView(true))
        );
        st.list_view = true;
        page.update(&mut ui, &st);
        assert!(ui.rect_of(SONG_LIST).is_some());
        assert!(ui.rect_of(SONG).is_none() && ui.rect_of(MORPH).is_none());
        assert_eq!(ui.get::<SongListView>(SONG_LIST).unwrap().title, "");
        let list = |k: EventKind, st: &TimeState| action(&ev(SONG_LIST, k), st);
        let why = |k: EventKind, st: &TimeState| why_not(&ev(SONG_LIST, k), st);
        assert_eq!(
            list(EventKind::Toggle(LIST_ADD), &st),
            Some(TimeAction::ListAdd)
        );
        assert_eq!(list(EventKind::Toggle(LIST_TO_SONG), &st), None);
        assert_eq!(
            why(EventKind::Toggle(LIST_TO_SONG), &st).as_deref(),
            Some("Add a scene to the list first")
        );
        // Two steps: 2 × 4 bars and 1 × 2 bars at 120 BPM.
        st.list = vec![
            ListRow {
                name: "Intro".into(),
                scene: Some(0),
                times: 2,
                bars: 8,
                starts: 1,
                ..ListRow::default()
            },
            ListRow {
                name: "Drop".into(),
                scene: Some(3),
                times: 1,
                bars: 2,
                starts: 9,
                ..ListRow::default()
            },
        ];
        page.update(&mut ui, &st);
        assert_eq!(
            ui.get::<SongListView>(SONG_LIST).unwrap().title,
            "2 steps · 10 bars · 0:20"
        );
        assert_eq!(list(EventKind::Toggle(LIST_PLAY), &st), None);
        assert_eq!(
            why(EventKind::Toggle(LIST_REMOVE), &st).as_deref(),
            Some("Tap a step first")
        );
        assert_eq!(
            list(EventKind::Select(1), &st),
            Some(TimeAction::ListPick(Some(1)))
        );
        st.list_pick = Some(1);
        assert_eq!(
            list(EventKind::Toggle(LIST_PLAY), &st),
            Some(TimeAction::ListPlay(1))
        );
        assert_eq!(
            list(EventKind::Toggle(LIST_REMOVE), &st),
            Some(TimeAction::ListRemove(1))
        );
        assert_eq!(
            list(EventKind::Toggle(LIST_TO_SONG), &st),
            Some(TimeAction::ListToSong)
        );
        assert_eq!(
            list(EventKind::Toggle(LIST_LOOP), &st),
            Some(TimeAction::ListLoop(true))
        );
        assert_eq!(
            list(EventKind::Toggle(LIST_CLIPS), &st),
            Some(TimeAction::SongView(false))
        );
        assert_eq!(
            list(EventKind::Toggle(LIST_SONG), &st),
            Some(TimeAction::SongView(true))
        );
        assert_eq!(
            list(EventKind::Step { index: 0, delta: 1 }, &st),
            Some(TimeAction::ListTimes { step: 0, delta: 1 })
        );
        assert_eq!(
            list(
                EventKind::Step {
                    index: 1,
                    delta: -1
                },
                &st
            ),
            None
        );
        assert_eq!(
            why(
                EventKind::Step {
                    index: 1,
                    delta: -1
                },
                &st
            )
            .as_deref(),
            Some("Once is the fewest")
        );
        assert_eq!(
            list(
                EventKind::Step {
                    index: SCENE_STEP + 1,
                    delta: -1
                },
                &st
            ),
            Some(TimeAction::ListScene { step: 1, delta: -1 })
        );
        // A linked list's Song row: no scene, once, and it says why.
        st.list[0].lead = true;
        for index in [0, SCENE_STEP] {
            let k = EventKind::Step { index, delta: 1 };
            assert_eq!(list(k.clone(), &st), None);
            assert_eq!(why(k, &st).as_deref(), Some(SONG_ROW_WHY));
        }
        st.list[0].lead = false;
        // A step dragged by its grip (Arrange 4b) moves to the gap.
        assert_eq!(
            list(EventKind::Drop { from: 1, to: 0 }, &st),
            Some(TimeAction::ListMove { step: 1, to: 0 })
        );
        assert_eq!(list(EventKind::Drop { from: 2, to: 0 }, &st), None);
        assert_eq!(list(EventKind::Drop { from: 0, to: 3 }, &st), None);
        // Linked to the song, the title counts the song's parts.
        st.list_linked = true;
        page.update(&mut ui, &st);
        assert_eq!(
            ui.get::<SongListView>(SONG_LIST).unwrap().title,
            "Linked to the song · 2 parts · 10 bars · 0:20"
        );
        // Back to the scene takes List → song's place (4b.3): for a part
        // changed in the song, picked; otherwise it says why.
        let back = EventKind::Toggle(LIST_TO_SONG);
        let pick = st.list_pick;
        st.list_pick = None;
        assert_eq!(list(back.clone(), &st), None);
        assert_eq!(why(back.clone(), &st).as_deref(), Some("Tap a part first"));
        st.list_pick = Some(1);
        assert_eq!(list(back.clone(), &st), None);
        assert_eq!(
            why(back.clone(), &st).as_deref(),
            Some("Drop follows its scene already")
        );
        st.list[1].changed = true;
        assert_eq!(list(back.clone(), &st), Some(TimeAction::ListBack(1)));
        st.list[1].changed = false;
        st.list_pick = Some(0);
        st.list[0].scene = None;
        assert_eq!(
            why(back.clone(), &st).as_deref(),
            Some("Intro has no scene")
        );
        st.list[0].lead = true;
        assert_eq!(why(back, &st).as_deref(), Some("Song has no scene"));
        (st.list[0].lead, st.list[0].scene, st.list_pick) = (false, Some(0), pick);
        st.list_linked = false;
        // The Clip tab keeps the list.
        page.clip_card = true;
        page.update(&mut ui, &st);
        assert_eq!(ui.layout_of(CLIPS_PAGE), Some(LIST_CARD_LAYOUT));
        assert_eq!(page.wheel_list(&ui), CLIP_CARD);
        page.clip_card = false;
        // NEXT: the step after the one playing, then the list's end.
        st.perform = true;
        st.transport.state = State::Playing;
        let bar = st.transport.meter.bar_ticks();
        st.transport.position = 3 * bar;
        st.list_next = Some((Some(1), 4 * bar));
        assert_eq!(st.next_reading().as_deref(), Some("Drop in 1 bar"));
        st.list_next = Some((None, 4 * bar));
        assert_eq!(st.next_reading().as_deref(), Some("Stop all in 1 bar"));
        st.list_view = false;
        st.song_view = false;
        page.update(&mut ui, &st);
        assert!(ui.rect_of(CLIPS).is_some() && ui.rect_of(SONG_LIST).is_none());
    }

    /// Jay, 14 Nov: drag and drop on the Song view. Blocks, their ends
    /// and sections ask to move; a section's name picks it; held at an
    /// edge the bars scroll; the hint follows what is picked.
    #[test]
    fn song_drags_ask_to_move_and_the_hint_follows_the_pick() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        st.song_view = true;
        st.markers = vec![(0, "Intro".into()), (4 * 3840, "Verse".into())];
        page.update(&mut ui, &st);
        let song = |k: EventKind, st: &TimeState| action(&ev(SONG, k), st);
        assert_eq!(
            song(
                EventKind::Place {
                    index: 2,
                    row: 1,
                    at: 7680
                },
                &st
            ),
            Some(TimeAction::MovePlacement {
                index: 2,
                track: 1,
                start: 7680,
                copy: false
            })
        );
        assert_eq!(
            song(EventKind::Stretch { index: 2, to: 3840 }, &st),
            Some(TimeAction::StretchPlacement {
                index: 2,
                length: 3840
            })
        );
        assert_eq!(
            song(EventKind::Drop { from: 1, to: 0 }, &st),
            Some(TimeAction::MoveSection {
                marker: 1,
                gap: 0,
                copy: false
            })
        );
        assert_eq!(
            song(EventKind::Tab(SECTION_TAP + 1), &st),
            Some(TimeAction::PickSection(Some(1)))
        );
        assert_eq!(song_hint(&st), SONG_HINT);
        st.section = Some(1);
        assert_eq!(
            song(EventKind::Tab(SECTION_TAP + 1), &st),
            Some(TimeAction::PickSection(None)),
            "a second tap lets it go"
        );
        assert_eq!(song_hint(&st), SONG_SECTION_HINT);
        page.update(&mut ui, &st);
        assert_eq!(ui.get::<SongLanes>(SONG).unwrap().section, Some(1));

        // The tempo lane (tempo ramps): a tap adds a point, a tap on one
        // picks it and a second lets it go, a drag moves it; picked, the
        // last two buttons glide it and delete it, the hint says so and
        // the lanes show the points.
        let tempo = |index, at| EventKind::Place {
            index,
            row: TEMPO_ROW,
            at,
        };
        assert_eq!(
            song(tempo(NONE, 7680), &st),
            Some(TimeAction::SongTempo(TempoEdit::Add(7680)))
        );
        st.tempo_points = vec![(3840, 100.0, false), (11_520, 140.0, true)];
        assert_eq!(
            song(tempo(0, 7680), &st),
            Some(TimeAction::SongTempo(TempoEdit::Move {
                index: 0,
                at: 7680
            }))
        );
        let tap = |k| EventKind::Cell {
            row: TEMPO_ROW,
            col: k,
        };
        assert_eq!(
            song(tap(1), &st),
            Some(TimeAction::SongTempo(TempoEdit::Pick(Some(1))))
        );
        st.tempo_pick = Some(1);
        assert_eq!(
            song(tap(1), &st),
            Some(TimeAction::SongTempo(TempoEdit::Pick(None))),
            "a second tap lets it go"
        );
        assert_eq!(
            song(EventKind::Tab(4), &st),
            Some(TimeAction::SongTempo(TempoEdit::Ramp {
                index: 1,
                on: false
            }))
        );
        assert_eq!(
            song(EventKind::Tab(5), &st),
            Some(TimeAction::SongTempo(TempoEdit::Delete(1)))
        );
        assert_eq!(song_hint(&st), SONG_TEMPO_HINT);
        st.tempo_base = 92.0;
        page.update(&mut ui, &st);
        let lanes = ui.get::<SongLanes>(SONG).unwrap();
        assert_eq!(
            (lanes.tempo.clone(), lanes.tempo_base, lanes.tempo_pick),
            (st.tempo_points.clone(), 92.0, Some(1))
        );
        st.tempo_pick = None;
        st.tempo_points.clear();
        st.section = None;
        st.selected = Some(0);
        assert_eq!(song_hint(&st), SONG_BLOCK_HINT);
        // A drag held at an edge scrolls the bars, never before bar 1.
        let edge = |k| UiEvent {
            widget: SONG.into(),
            kind: EventKind::Tab(k),
        };
        assert!(page.page_tracks(&edge(BARS_ON), &st));
        assert!(page.page_tracks(&edge(BARS_ON), &st));
        assert_eq!(page.first_bar, 2);
        assert!(page.page_tracks(&edge(BARS_BACK), &st));
        assert!(page.page_tracks(&edge(BARS_BACK), &st));
        assert!(page.page_tracks(&edge(BARS_BACK), &st));
        assert_eq!(page.first_bar, 0);
        // The undo list names a song edit by its track and bar.
        let mut takes = Takes::default();
        for _ in 0..6 {
            takes.song_edit(
                &mpfree_seq::Project::new(),
                SongChange::Scenes { rows: 1 },
                Default::default(),
            );
        }
        let edit = |change| {
            let mut t = takes.clone();
            t.song_edit(&mpfree_seq::Project::new(), change, Default::default());
            t.done().last().unwrap().clone()
        };
        let moved = TakeRow::of(&edit(SongChange::Moved {
            track: 0,
            start: 8 * 3840,
        }));
        assert_eq!(moved.short(), "move 7");
        assert_eq!(
            take_title(&st, &moved),
            format!("Move 7 · {}", st.track_name(0))
        );
        assert_eq!(take_detail(&st, &moved), "to bar 9");
        let verse = TakeRow::of(&edit(SongChange::SectionCopied { at: 4 * 3840 }));
        assert_eq!(take_title(&st, &verse), "Copy 7 · Verse");
        let longer = TakeRow::of(&edit(SongChange::Resized {
            track: 0,
            length: 6 * 3840,
        }));
        assert_eq!(take_detail(&st, &longer), "6 bars");
    }

    /// Layout spec 4c: the audio lanes' taps, the selection's kind, and
    /// the bottom row with a region picked.
    /// Layout spec 4f: an audio track is a column of the grid, its name
    /// arming it to record (its own arm, as the Song view's dot), its
    /// cells empty with a line saying where to record it.
    #[test]
    fn an_audio_track_is_a_column_its_name_arms_it() {
        use crate::song::AudioLane;
        let mut st = state();
        st.tracks[1].audio = true;
        st.tracks[1].sound = "L".into();
        st.audio = vec![AudioLane {
            track: 4,
            row: 1,
            name: st.tracks[1].name.clone(),
            color: track_color(1),
            armed: false,
            input: Input::Left,
            mix: None,
            level: 0.0,
            volume: 1.0,
            pan: 0.0,
            monitor: Monitor::Auto,
            mute: false,
            solo: false,
            blocks: Vec::new(),
            takes: Vec::new(),
            recording: None,
            clips: Vec::new(),
        }];
        let cols = grid_tracks(&st);
        assert!(cols[1].audio && !cols[0].audio);
        assert!(!cols[1].armed);
        assert_eq!(
            action(&ev(CLIPS, EventKind::Tab(1)), &st),
            Some(TimeAction::Audio(AudioAction::Arm { track: 4, on: true }))
        );
        st.audio[0].armed = true;
        assert!(grid_tracks(&st)[1].armed);
        assert_eq!(
            action(&ev(CLIPS, EventKind::Tab(1)), &st),
            Some(TimeAction::Audio(AudioAction::Arm {
                track: 4,
                on: false
            }))
        );
        let cell = ev(CLIPS, EventKind::Select(ROWS + 2));
        assert_eq!(action(&cell, &st), None);
        assert_eq!(why_not(&cell, &st).as_deref(), Some(EMPTY_AUDIO_CELL));
        // The chooser's rows on the grid.
        let row = |k| action(&ev(CLIPS, EventKind::Toggle(ROWS + ADD_KIND + k)), &st);
        assert_eq!(row(0), Some(TimeAction::AddTrack));
        assert_eq!(row(1), Some(TimeAction::AddAudioTrack));
        assert_eq!(row(2), Some(TimeAction::AddLoopTrack));
        assert_eq!(
            action(&ev(CLIPS, EventKind::Toggle(ROWS + ADD_DISMISS)), &st),
            None
        );
    }

    /// Phase 4, board 2 (layout spec 4f, Loops in audio cells), with Dev's
    /// "Loops on audio tracks" on: a loop cell on an audio track shows its
    /// length and the ring, never a wave; queued, when it starts; a layer
    /// still being written, the dot and "writing". A tap on an empty cell
    /// of the armed audio track arms it ("REC here", "a loop"), Record
    /// says where and what (Armed, then Makes), and the title counts the
    /// loops. With the switch off none of it shows.
    #[test]
    fn with_loops_on_a_loop_cell_on_an_audio_track_reads_as_a_loop() {
        use crate::song::AudioLane;
        let mut st = state();
        st.loops_on = true;
        st.tracks[1].audio = true;
        st.audio = vec![AudioLane {
            track: 4,
            row: 1,
            name: st.tracks[1].name.clone(),
            color: track_color(1),
            armed: true,
            input: Input::Left,
            mix: None,
            level: 0.0,
            volume: 1.0,
            pan: 0.0,
            monitor: Monitor::Auto,
            mute: false,
            solo: false,
            blocks: Vec::new(),
            takes: Vec::new(),
            recording: None,
            clips: Vec::new(),
        }];
        // Rows 1 and 2 hold loops, in the looper's slots 3 and 5; row 2's
        // launch waits for bar 6 (the fixture's).
        for s in [0, 1] {
            let c = st.tracks[1].slots[s].as_mut().unwrap();
            c.loop_id = Some(mpfree_seq::LoopId(70 + s as u128));
            c.notes = 0;
        }
        st.loops.ids[3] = 70;
        st.loops.ids[5] = 71;
        st.loops.in_use = 6;
        let cell = cell_of(&st, 1, 0);
        assert!(
            cell.looped && cell.wave.is_none() && !cell.writing,
            "{cell:?}"
        );
        assert_eq!(
            (cell.state, cell.detail.as_str(), cell.foot.as_str()),
            (CellState::Stopped, "2 bars", "")
        );
        let queued = cell_of(&st, 1, 1);
        assert_eq!(queued.state, CellState::Queued);
        assert_eq!(queued.foot, "from 6.1");
        // A layer still being written.
        st.loops.loops[3].writing = true;
        let writing = cell_of(&st, 1, 0);
        assert!(writing.writing);
        assert_eq!(
            (writing.foot.as_str(), writing.alert),
            ("writing", CellAlert::Warn)
        );
        // The looper's slot is found by the loop's id; its place too.
        assert_eq!(st.loop_slot(st.clip(1, 1).unwrap()), Some(5));
        let places = loop_places(&st);
        assert_eq!(places[3].as_ref().map(|p| (p.track, p.slot)), Some((1, 0)));
        // An empty cell of the armed audio track arms a loop there; a
        // second tap lets go.
        let empty = ev(CLIPS, EventKind::Select(ROWS + 2));
        assert_eq!(
            action(&empty, &st),
            Some(TimeAction::LoopCell(Some((1, 2))))
        );
        st.loop_cell = Some((1, 2));
        let armed = cell_of(&st, 1, 2);
        assert_eq!(
            (armed.state, armed.name.as_str(), armed.detail.as_str()),
            (CellState::Armed, "REC here", "a loop")
        );
        assert_eq!(action(&empty, &st), Some(TimeAction::LoopCell(None)));
        // Record: where (Armed) and what (Makes, under it).
        let rows = record_rows(&st);
        let row = |k: usize| (rows[k].name.as_str(), rows[k].value.as_str());
        assert_eq!(row(0), ("Armed", "Bass · row 3"));
        assert_eq!(row(1), ("Makes", "a loop"));
        assert_eq!(row(2).0, "Mode");
        assert_eq!(rows.len(), 11);
        // A step on a row under Makes steps that row; on Makes, it says why.
        let step = |index| ev(RECORD, EventKind::Step { index, delta: 1 });
        assert_eq!(
            action(&step(2), &st),
            Some(TimeAction::Transport(Command::ToggleOverdub))
        );
        assert_eq!(action(&step(1), &st), None);
        assert_eq!(why_not(&step(1), &st).as_deref(), Some(MAKES_WHY));
        // The title counts the loops after the rows.
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        page.update(&mut ui, &st);
        let title = &ui.get::<ClipGrid>(CLIPS).unwrap().title;
        assert!(title.ends_with(" · Loops 6 of 8"), "{title}");
        // The audio track not armed: a tap says how.
        st.audio[0].armed = false;
        let other = ev(CLIPS, EventKind::Select(ROWS + 3));
        assert_eq!(action(&other, &st), None);
        assert!(why_not(&other, &st).is_some_and(|w| w.contains("arm Bass")));
        // The switch off: nothing of it.
        st.loops_on = false;
        st.audio[0].armed = true;
        assert_eq!(action(&other, &st), None);
        assert_eq!(why_not(&other, &st).as_deref(), Some(EMPTY_AUDIO_CELL));
        assert_eq!(record_rows(&st)[1].name, "Mode");
        assert_eq!(record_rows(&st).len(), 10);
        assert_eq!(
            action(&step(1), &st),
            Some(TimeAction::Transport(Command::ToggleOverdub))
        );
    }

    /// Layout spec 4f: an audio clip's cell shows its name, its wave and
    /// its length, and in the wave's place the word that matters, each
    /// with its full sentence on the hint line.
    #[test]
    fn an_audio_clip_cell_says_the_word_that_matters() {
        let mut st = state();
        st.armed = None;
        st.tracks[2].audio = true;
        let clip = st.tracks[2].slots[0].as_mut().unwrap();
        clip.audio = Some(AudioCellInfo {
            bpm: 120.0,
            gain: 1.0,
            secs: 4.0,
            wave: [9; crate::widgets::CELL_WAVE],
            ..AudioCellInfo::default()
        });
        let set = |st: &mut TimeState, f: &dyn Fn(&mut ClipInfo)| {
            f(st.tracks[2].slots[0].as_mut().unwrap())
        };
        // Stopped at the tempo it was made at: its wave, nothing to say.
        let c = cell_of(&st, 2, 0);
        assert_eq!(
            (c.state, c.name.as_str(), c.detail.as_str(), c.foot.as_str()),
            (CellState::Stopped, "A", "2 bars", "")
        );
        assert_eq!(c.wave, Some([9; crate::widgets::CELL_WAVE]));
        assert_eq!(c.alert, CellAlert::None);
        assert_eq!(audio_cell_hint(&st, 2, 0), None);
        // An empty audio cell is dashed, and says how to fill it.
        assert_eq!(cell_of(&st, 2, 2).state, CellState::Empty);
        assert_eq!(
            audio_cell_hint(&st, 2, 2).as_deref(),
            Some(EMPTY_AUDIO_CELL)
        );
        assert_eq!(audio_cell_hint(&st, 1, 2), None);
        // At 92 BPM the loop leaves a gap each pass.
        st.transport.bpm = 92.0;
        let c = cell_of(&st, 2, 0);
        assert_eq!((c.foot.as_str(), c.alert), ("made at 120", CellAlert::Warn));
        assert_eq!(
            audio_cell_hint(&st, 2, 0).as_deref(),
            Some(
                "A was made at 120 BPM: at 92 it leaves 1.2 s of silence each pass. \
                 Fit to tempo on the Clip tab fixes it."
            )
        );
        // Faster than it was made, it cuts its sound short.
        st.transport.bpm = 150.0;
        assert_eq!(
            audio_cell_hint(&st, 2, 0).as_deref(),
            Some(
                "A was made at 120 BPM: at 150 it cuts 0.8 s off each pass. \
                 Fit to tempo on the Clip tab fixes it."
            )
        );
        // A once clip plays to its end at any tempo: its time, no word.
        set(&mut st, &|c| c.settings.once = true);
        let c = cell_of(&st, 2, 0);
        assert_eq!((c.detail.as_str(), c.foot.as_str()), ("0:04 once", ""));
        assert_eq!(audio_cell_hint(&st, 2, 0), None);
        set(&mut st, &|c| c.settings.once = false);
        st.transport.bpm = 120.0;
        // Its audio wasn't ready in time: red, and how long it waited.
        st.not_ready = vec![(2, 0, 2 * BAR)];
        let c = cell_of(&st, 2, 0);
        assert_eq!((c.foot.as_str(), c.alert), ("not ready", CellAlert::Bad));
        assert_eq!(
            audio_cell_hint(&st, 2, 0).as_deref(),
            Some("A's audio wasn't ready within 2 bars. Tap to try again.")
        );
        st.not_ready.clear();
        // Missing: red, a flat dashed line for its wave.
        let why = "take 4's file wasn't found when the project opened";
        set(&mut st, &|c| {
            c.audio.as_mut().unwrap().missing = Some(why.into())
        });
        let c = cell_of(&st, 2, 0);
        assert_eq!((c.foot.as_str(), c.alert), ("missing", CellAlert::Bad));
        assert!(c.missing && c.wave.is_none());
        assert_eq!(
            audio_cell_hint(&st, 2, 0).as_deref(),
            Some("A's audio is missing: take 4's file wasn't found when the project opened.")
        );
        set(&mut st, &|c| c.audio.as_mut().unwrap().missing = None);
        // A once clip still sounding past its stop: its playhead, dimmed.
        set(&mut st, &|c| c.audio.as_mut().unwrap().ring = Some(0.5));
        let c = cell_of(&st, 2, 0);
        assert!(c.ringing);
        assert_eq!((c.progress, c.foot.as_str()), (0.5, ""));
        set(&mut st, &|c| c.audio.as_mut().unwrap().ring = None);
        // Queued: when it starts, amber; its audio late, when it waits to.
        st.cues = vec![Cue {
            track: 2,
            slot: Some(0),
        }];
        let c = cell_of(&st, 2, 0);
        assert_eq!(
            (c.state, c.foot.as_str(), c.alert),
            (CellState::Queued, "from 6.1", CellAlert::Warn)
        );
        st.audio_late = vec![(2, 17 * BAR)];
        let c = cell_of(&st, 2, 0);
        assert_eq!(c.foot, "waits 18.1");
        assert_eq!(
            audio_cell_hint(&st, 2, 0).as_deref(),
            Some("A waits for its audio: it starts on 18.1.")
        );
    }

    /// Layout spec 4j: an audio clip's card has its own rows, steps a
    /// loop's length by the beat (SHIFT the bar) and its gain by 0.5 dB,
    /// and its foot says the tempo it was made at over Fit to tempo.
    #[test]
    fn an_audio_clips_card_has_its_own_rows_and_fit_to_tempo() {
        let mut st = state();
        st.tracks[2].audio = true;
        st.tracks[2].slots[0].as_mut().unwrap().audio = Some(AudioCellInfo {
            bpm: 120.0,
            gain: 1.0,
            secs: 197.0,
            ..AudioCellInfo::default()
        });
        assert_eq!(st.focus(), Some((2, 0)));
        let rows: Vec<(String, String)> = clip_rows(&st)
            .into_iter()
            .map(|r| (r.name, r.value))
            .collect();
        let want = [
            ("Length", "2 bars"),
            ("Mode", "Restart"),
            ("Launch", "project"),
            ("Follow", "off"),
            ("After", "—"),
            ("Gain", "0.0 dB"),
            ("Name", "A"),
        ];
        let want: Vec<(String, String)> = want
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect();
        assert_eq!(rows, want);
        let step = |st: &TimeState, index, delta, shift| card_action(st, index, delta, shift);
        let length = |l| {
            Ok(TimeAction::AudioClip {
                track: 2,
                slot: 0,
                row: 0,
                set: AudioSet::Length(l),
            })
        };
        // Length: a beat, SHIFT a bar; never under a beat.
        assert_eq!(step(&st, 0, 1, false), length(2 * BAR + BAR / 4));
        assert_eq!(step(&st, 0, -1, false), length(2 * BAR - BAR / 4));
        assert_eq!(step(&st, 0, -1, true), length(BAR));
        assert_eq!(
            bars_beats(2 * BAR + BAR / 4, Meter::default()),
            "2 bars 1 beat"
        );
        assert_eq!(bars_beats(BAR / 2, Meter::default()), "2 beats");
        st.tracks[2].slots[0].as_mut().unwrap().length = BAR / 4;
        assert_eq!(
            step(&st, 0, -1, false),
            Err("A is at its shortest, 1 beat".to_string())
        );
        st.tracks[2].slots[0].as_mut().unwrap().length = 2 * BAR;
        // Mode, Launch, Follow and After step as a note clip's rows.
        assert_eq!(step(&st, 1, 1, false), card_step(&state(), 8, 1));
        assert_eq!(step(&st, 3, 1, false), card_step(&state(), 5, 1));
        // Gain: 0.5 dB, SHIFT 0.1 dB, up to +12 dB.
        let Ok(TimeAction::AudioClip {
            set: AudioSet::Gain(g),
            ..
        }) = step(&st, AUDIO_GAIN, 1, false)
        else {
            panic!("a gain");
        };
        assert_eq!(gain_words(g), "+0.5 dB");
        let Ok(TimeAction::AudioClip {
            set: AudioSet::Gain(g),
            ..
        }) = step(&st, AUDIO_GAIN, -1, true)
        else {
            panic!("a gain");
        };
        assert_eq!(gain_words(g), "-0.1 dB");
        let set_gain = |st: &mut TimeState, g| {
            st.tracks[2].slots[0]
                .as_mut()
                .unwrap()
                .audio
                .as_mut()
                .unwrap()
                .gain = g
        };
        set_gain(&mut st, 10f32.powf(0.6));
        assert_eq!(clip_rows(&st)[AUDIO_GAIN].value, "+12.0 dB");
        assert!(step(&st, AUDIO_GAIN, 1, false).is_err());
        set_gain(&mut st, 0.0);
        assert_eq!(clip_rows(&st)[AUDIO_GAIN].value, "off");
        assert!(step(&st, AUDIO_GAIN, -1, false).is_err());
        set_gain(&mut st, 1.0);
        // Name is the app's.
        assert_eq!(step(&st, AUDIO_NAME, 1, false), Err(String::new()));
        // At the project's tempo: Fit to tempo faint, a tap says why.
        let fit = ev(CLIP_CARD, EventKind::Toggle(0));
        let b = &card_buttons(&st)[0];
        assert_eq!(
            (b.label.as_str(), b.enabled, b.explains),
            ("Fit to tempo", false, true)
        );
        assert_eq!(action(&fit, &st), None);
        assert_eq!(
            why_not(&fit, &st).as_deref(),
            Some("A was made at 120 BPM, the project's tempo: there is nothing to fit")
        );
        let note = card_note(&st).unwrap();
        assert_eq!(
            (note.lead.as_str(), note.rest.as_str(), note.warn),
            ("Made at 120 BPM,", "the project's tempo", false)
        );
        // At 92: the gap each pass, amber, and Fit to tempo fits.
        st.transport.bpm = 92.0;
        let note = card_note(&st).unwrap();
        assert_eq!(
            (note.lead.as_str(), note.rest.as_str(), note.warn),
            ("Made at 120 BPM:", "at 92 a 1.2 s gap each pass", true)
        );
        assert!(card_buttons(&st)[0].enabled);
        assert_eq!(
            action(&fit, &st),
            Some(TimeAction::FitClip { track: 2, slot: 0 })
        );
        // Fitting: its progress, and a tap says how far it has got.
        st.fitting = Some((2, 0, 0.4));
        let b = &card_buttons(&st)[0];
        assert_eq!(
            (b.label.as_str(), b.progress, b.enabled),
            ("Fitting… 40 %", Some(0.4), true)
        );
        assert_eq!(action(&fit, &st), None);
        assert_eq!(
            why_not(&fit, &st).as_deref(),
            Some("A is fitting to 92 BPM: 40 % done")
        );
        st.fitting = None;
        // A once clip: its time, not stepped, and nothing to fit.
        st.tracks[2].slots[0].as_mut().unwrap().settings.once = true;
        assert_eq!(clip_rows(&st)[0].value, "3:17");
        assert_eq!(clip_rows(&st)[1].value, "Once");
        assert!(step(&st, 0, 1, false).is_err());
        let note = card_note(&st).unwrap();
        assert_eq!(
            (note.lead.as_str(), note.rest.as_str()),
            ("Plays to its end", "at any tempo")
        );
        assert!(!card_buttons(&st)[0].enabled);
        // A note clip's card is as it was.
        st.armed = Some(1);
        assert_eq!(clip_rows(&st).len(), CLIP_ROWS.len());
        assert_eq!(card_note(&st), None);
        assert_eq!(card_buttons(&st).len(), 2);
    }

    /// Layout spec 4j: the Clip tab follows an audio track armed by its
    /// own arm, until a note track is armed again.
    #[test]
    fn the_clip_tab_follows_an_armed_audio_track() {
        let mut st = state();
        assert_eq!(st.focus(), Some((2, 0)));
        st.tracks[1].audio = true;
        st.audio_focus = Some(1);
        assert_eq!(st.focus(), Some((1, 1)));
        st.audio_focus = None;
        assert_eq!(st.focus(), Some((2, 0)));
    }

    /// The "+ Track" chooser is the page's: a tap on "+ Track" opens it
    /// (not with SHIFT), a touch off it closes it, a row picked closes it
    /// and goes to the app, and the new track comes into view.
    #[test]
    fn plus_track_opens_the_chooser_and_the_new_track_comes_into_view() {
        let mut st = state();
        let mut page = TimePage::default();
        let plus = ev(CLIPS, EventKind::Toggle(ROWS + ADD_TRACK));
        assert!(page.chooser(&plus, &st, false));
        assert!(page.adding);
        let off = ev(SONG, EventKind::Tab(crate::song::ADD_CLOSE));
        assert!(page.chooser(&off, &st, false));
        assert!(!page.adding);
        assert!(
            !page.chooser(&plus, &st, true),
            "SHIFT: the app's Loop track"
        );
        assert!(!page.adding);
        assert!(page.chooser(&plus, &st, false));
        let audio = ev(CLIPS, EventKind::Toggle(ROWS + ADD_KIND + 1));
        assert!(!page.chooser(&audio, &st, false), "the app adds it");
        assert!(!page.adding);
        // Once it is there, past the first page: its page shows.
        while st.tracks.len() <= GRID_COLS {
            let t = st.tracks[0].clone();
            st.tracks.push(t);
        }
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        page.update(&mut ui, &st);
        assert_eq!(page.first_track, GRID_COLS);
    }

    #[test]
    fn the_audio_lanes_ask_for_audio_actions() {
        use crate::song::{AudioBlock, AudioLane};
        let mut st = state();
        st.song_view = true;
        st.audio = vec![AudioLane {
            track: 2,
            row: usize::MAX,
            name: "Vocal".into(),
            color: audio_color(2),
            armed: true,
            input: Input::Left,
            mix: None,
            level: 0.3,
            volume: 1.0,
            pan: 0.0,
            monitor: Monitor::Auto,
            mute: false,
            solo: false,
            blocks: vec![AudioBlock {
                start: 0,
                end: BAR,
                take: 0,
                tag: String::new(),
                src: 0,
                frames: 88_200.0,
                gain: 1.0,
                fade_in: 0,
                fade_out: 0,
                into: 0,
                left: 0,
            }],
            takes: Vec::new(),
            recording: None,
            clips: Vec::new(),
        }];
        let song = |k: EventKind, st: &TimeState| action(&ev(SONG, k), st);
        let audio = |a: AudioAction| Some(TimeAction::Audio(a));
        let r = RegionId { track: 2, index: 0 };
        assert_eq!(
            song(EventKind::Cell { row: 2, col: 0 }, &st),
            audio(AudioAction::Pick(Some(r)))
        );
        assert_eq!(
            song(EventKind::Cell { row: 2, col: PANEL }, &st),
            audio(AudioAction::Panel(2))
        );
        // The input chip opens the panel too; with Dev's "Loops on audio
        // tracks", the track's input list (phase 4, board 4).
        let chip = EventKind::Cell {
            row: 2,
            col: INPUT_CHIP,
        };
        assert_eq!(song(chip.clone(), &st), audio(AudioAction::Panel(2)));
        let mut on = st.clone();
        on.loops_on = true;
        assert_eq!(song(chip, &on), audio(AudioAction::Inputs(2)));
        assert_eq!(
            song(EventKind::Cell { row: 2, col: PANEL }, &on),
            audio(AudioAction::Panel(2))
        );
        assert_eq!(
            song(EventKind::Toggle(2), &st),
            audio(AudioAction::Arm {
                track: 2,
                on: false
            }),
            "armed: the dot disarms"
        );
        assert_eq!(
            song(EventKind::Toggle(5), &st),
            audio(AudioAction::Arm { track: 5, on: true })
        );
        // The "+ Track" chooser's rows.
        assert_eq!(
            song(EventKind::Tab(ADD_AUDIO), &st),
            Some(TimeAction::AddAudioTrack)
        );
        assert_eq!(
            song(EventKind::Tab(crate::song::ADD_NOTE), &st),
            Some(TimeAction::AddTrack)
        );
        assert_eq!(
            song(EventKind::Tab(crate::song::ADD_LOOP), &st),
            Some(TimeAction::AddLoopTrack)
        );
        assert_eq!(song(EventKind::Tab(crate::song::ADD_CLOSE), &st), None);
        // Nothing picked: the Song view's own row.
        assert_eq!(st.pick(), None);
        assert_eq!(
            song(EventKind::Tab(1), &st),
            Some(TimeAction::SongMode(!st.song_on))
        );
        st.selected = Some(1);
        assert_eq!(st.pick(), Some(Pick::Placement(1)));
        // A region picked: its row, whatever was picked before.
        st.region = Some(r);
        assert_eq!(st.pick(), Some(Pick::Region(r)));
        let row = [
            AudioAction::Split(r),
            AudioAction::Erase(r),
            AudioAction::Fades(r),
            AudioAction::Gain(r),
            AudioAction::Takes(2),
            AudioAction::ToCell(r),
        ];
        for (k, want) in row.into_iter().enumerate() {
            assert_eq!(song(EventKind::Tab(k), &st), audio(want), "button {k}");
        }
        // Rec song and empty space still do what they did.
        assert_eq!(
            song(EventKind::Tab(REC), &st),
            Some(TimeAction::RecSong(true))
        );
        assert_eq!(
            song(EventKind::Select(NONE), &st),
            Some(TimeAction::SelectPlacement(None))
        );
        // The widget shows the lane, the pick and the memory legend.
        st.audio_free = Some((31.4, 60.0));
        st.audio_counts = vec![("dropped".into(), 0), ("read errors".into(), 2)];
        let lanes = song_lanes(&st, 0);
        assert_eq!(lanes.region, Some((2, 0)));
        assert_eq!(lanes.audio.len(), 1);
        assert!(
            lanes
                .legend
                .ends_with("Audio 0:31 of 1:00 free · 2 read errors"),
            "{}",
            lanes.legend
        );
        assert!(lanes.alert, "a loss shows in the warning colour");
        assert!(lanes.waves.is_empty(), "update copies the waves");
    }

    /// Layout spec 4c: the track panel's controls ask for the open
    /// panel's track; with no panel open they ask for nothing.
    #[test]
    fn the_track_panel_asks_for_its_track() {
        use crate::song::{
            AudioLane, PANEL_CLOSE, PANEL_INPUT, PANEL_LEVEL, PANEL_MONITOR, PANEL_MUTE, PANEL_PAN,
            PANEL_SOLO, PANEL_TAKES,
        };
        let mut st = state();
        st.song_view = true;
        st.audio = vec![AudioLane {
            track: 3,
            row: usize::MAX,
            name: "Audio 4".into(),
            color: audio_color(3),
            armed: false,
            input: Input::Left,
            mix: None,
            level: 0.0,
            volume: 1.0,
            pan: 0.0,
            monitor: Monitor::Auto,
            mute: true,
            solo: false,
            blocks: Vec::new(),
            takes: Vec::new(),
            recording: None,
            clips: Vec::new(),
        }];
        let song = |k: EventKind, st: &TimeState| action(&ev(SONG, k), st);
        let audio = |a: AudioAction| Some(TimeAction::Audio(a));
        assert_eq!(song(EventKind::Tab(PANEL_MUTE), &st), None, "no panel");
        assert_eq!(
            song(
                EventKind::Step {
                    index: PANEL_LEVEL,
                    delta: 1
                },
                &st
            ),
            None
        );
        st.audio_panel = Some(3);
        assert_eq!(lanes_panel(&st), Some(3));
        let tab = |k: usize, st: &TimeState| song(EventKind::Tab(k), st);
        assert_eq!(tab(PANEL_CLOSE, &st), audio(AudioAction::ClosePanel));
        assert_eq!(
            tab(PANEL_MUTE, &st),
            audio(AudioAction::Mute {
                track: 3,
                on: false
            }),
            "muted: Mute unmutes"
        );
        assert_eq!(
            tab(PANEL_SOLO, &st),
            audio(AudioAction::Solo { track: 3, on: true })
        );
        assert_eq!(tab(PANEL_TAKES, &st), audio(AudioAction::Takes(3)));
        assert_eq!(
            tab(crate::song::PANEL_STRIP, &st),
            audio(AudioAction::Strip(3))
        );
        assert_eq!(
            tab(PANEL_INPUT + 3, &st),
            audio(AudioAction::Input {
                track: 3,
                input: Input::Stereo
            })
        );
        assert_eq!(
            tab(PANEL_INPUT + 2, &st),
            audio(AudioAction::Input {
                track: 3,
                input: Input::Sum
            })
        );
        assert_eq!(
            tab(PANEL_MONITOR + 2, &st),
            audio(AudioAction::Monitor {
                track: 3,
                monitor: Monitor::Off
            })
        );
        // Mix and its From steps and Effects (step 9).
        use crate::song::{PANEL_AFTER, PANEL_BEFORE, PANEL_FROM_LESS, PANEL_FROM_MORE, PANEL_MIX};
        let mix = |step| audio(AudioAction::Mix { track: 3, step });
        assert_eq!(
            tab(PANEL_MONITOR + 3, &st),
            mix(0),
            "Mix follows the monitors"
        );
        assert_eq!(tab(PANEL_MIX, &st), mix(0));
        assert_eq!(tab(PANEL_FROM_LESS, &st), mix(-1));
        assert_eq!(tab(PANEL_FROM_MORE, &st), mix(1));
        let fx = |after| audio(AudioAction::MixEffects { track: 3, after });
        assert_eq!(tab(PANEL_BEFORE, &st), fx(false));
        assert_eq!(tab(PANEL_AFTER, &st), fx(true));
        assert_eq!(
            tab(crate::song::PANEL_STRIP + 1, &st),
            None,
            "past the last"
        );
        let step = |index, delta, st: &TimeState| song(EventKind::Step { index, delta }, st);
        let turn = |knob, ticks, region| {
            audio(AudioAction::Turn {
                track: 3,
                region,
                knob,
                ticks,
                fine: false,
            })
        };
        assert_eq!(step(PANEL_LEVEL, -1, &st), turn(0, -1, None));
        // A region picked on another track is not this track's.
        st.region = Some(RegionId { track: 1, index: 0 });
        assert_eq!(step(PANEL_PAN, 1, &st), turn(1, 1, None));
        st.region = Some(RegionId { track: 3, index: 2 });
        assert_eq!(step(PANEL_PAN, 1, &st), turn(1, 1, Some(2)));
        // The region's row still answers with the panel open.
        assert_eq!(
            song(EventKind::Tab(0), &st),
            audio(AudioAction::Split(RegionId { track: 3, index: 2 }))
        );
    }

    /// Layout spec 4c: a tap on a take lane uses the take, a swipe comps
    /// the stretch; the Song view shows the open track's take lanes.
    #[test]
    fn take_lanes_ask_to_comp() {
        let mut st = state();
        st.song_view = true;
        let song = |k: EventKind, st: &TimeState| action(&ev(SONG, k), st);
        let audio = |a: AudioAction| Some(TimeAction::Audio(a));
        assert_eq!(
            song(
                EventKind::Span {
                    index: 3,
                    from: 0,
                    to: 0
                },
                &st
            ),
            audio(AudioAction::UseTake(3))
        );
        assert_eq!(
            song(
                EventKind::Span {
                    index: 3,
                    from: 960,
                    to: 2880
                },
                &st
            ),
            audio(AudioAction::Comp {
                take: 3,
                from: 960,
                to: 2880,
                fine: false
            })
        );
        assert_eq!(
            song(
                EventKind::Span {
                    index: 70_000,
                    from: 0,
                    to: 0
                },
                &st
            ),
            None,
            "no such take"
        );
        st.audio_takes = Some(2);
        assert_eq!(song_lanes(&st, 0).takes_open, Some(2));
        // A region dragged: its track and index, and where it starts now.
        assert_eq!(
            song(
                EventKind::Marker {
                    index: 3 * crate::song::REGION_SLOTS + 4,
                    frame: 7680
                },
                &st
            ),
            audio(AudioAction::Move {
                region: RegionId { track: 3, index: 4 },
                to: 7680,
                fine: false
            })
        );
    }

    fn lanes_panel(st: &TimeState) -> Option<usize> {
        song_lanes(st, 0).panel
    }

    /// The waveforms reach the widget, and are copied again only when they
    /// change; the lanes' scroll survives an update.
    #[test]
    fn the_song_view_keeps_its_waves_and_scroll() {
        use mpfree_audiotracks::host::TakeWave;
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Arrange"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        st.song_view = true;
        st.waves = vec![TakeWave {
            id: 0,
            frames: 1000,
            channels: 1,
            frames_per_column: 10,
            columns: vec![(-0.5, 0.5); 100],
        }];
        page.update(&mut ui, &st);
        assert_eq!(ui.get::<SongLanes>(SONG).unwrap().waves.len(), 1);
        ui.get_mut::<SongLanes>(SONG).unwrap().first_lane = 1;
        st.transport.position += 100;
        page.update(&mut ui, &st);
        let l = ui.get::<SongLanes>(SONG).unwrap();
        assert_eq!((l.waves.len(), l.first_lane), (1, 1));
        assert_eq!(l.playhead, Some(st.transport.position));
        st.waves[0].frames = 1200;
        st.waves[0].columns.push((0.0, 0.0));
        page.update(&mut ui, &st);
        assert_eq!(ui.get::<SongLanes>(SONG).unwrap().waves[0].frames, 1200);
    }

    #[test]
    fn the_morph_bar_picks_rows_moves_and_lets_go() {
        let mut st = state();
        let morph = |k: EventKind, st: &TimeState| action(&ev(MORPH, k), st);
        assert_eq!(
            morph(
                EventKind::Step {
                    index: 0,
                    delta: -1
                },
                &st
            ),
            Some(TimeAction::MorphRows { a: ROWS - 1, b: 1 }),
            "round to the last row"
        );
        assert_eq!(
            morph(EventKind::Step { index: 1, delta: 1 }, &st),
            Some(TimeAction::MorphRows { a: 0, b: 2 })
        );
        assert_eq!(
            morph(EventKind::Value(0.3), &st),
            Some(TimeAction::Morph(0.3))
        );
        assert_eq!(
            morph(EventKind::Tab(0), &st),
            None,
            "nothing to snap back from"
        );
        assert_eq!(
            morph(EventKind::Tab(2), &st),
            Some(TimeAction::SaveScene(0))
        );
        st.morph.on = true;
        st.morph.pos = 0.8;
        assert_eq!(morph(EventKind::Tab(0), &st), Some(TimeAction::SnapBack));
        assert_eq!(morph(EventKind::Tab(1), &st), Some(TimeAction::KeepMorph));
        assert_eq!(
            morph(EventKind::Tab(2), &st),
            Some(TimeAction::SaveScene(1))
        );
        assert_eq!(
            action(&ev(CLIPS, EventKind::Tab(2)), &st),
            Some(TimeAction::Arm(None)),
            "arming again disarms"
        );
    }

    #[test]
    fn the_bar_and_takes_buttons_map_to_commands() {
        let st = state();
        let bar = |b: BarButton| action(&ev(TRANSPORT, EventKind::Tab(b.index())), &st);
        assert_eq!(
            bar(BarButton::Stop),
            Some(TimeAction::Transport(Command::Stop))
        );
        assert_eq!(
            bar(BarButton::Play),
            Some(TimeAction::Transport(Command::Play))
        );
        assert_eq!(
            bar(BarButton::Rec),
            Some(TimeAction::Transport(Command::Record))
        );
        assert_eq!(
            bar(BarButton::Mode),
            Some(TimeAction::Transport(Command::ToggleOverdub))
        );
        assert_eq!(bar(BarButton::Undo), Some(TimeAction::Undo));
        assert_eq!(
            action(&ev(TRANSPORT, EventKind::Select(0)), &st),
            Some(TimeAction::Chip(0))
        );
        // With nothing to undo, redo or capture, the take buttons are faint:
        // they send nothing and say why in place.
        for i in 0..3 {
            let e = ev(TAKES, EventKind::Tab(i));
            assert_eq!(action(&e, &st), None, "take button {i}");
            assert!(why_not(&e, &st).is_some(), "take button {i} says why");
        }
        let take = TakeRow {
            number: 1,
            captured: false,
            edited: false,
            made: true,
            at: Some((0, 0)),
            added: 4,
            erased: 0,
            deleted: false,
            new_track: None,
            dropped: None,
            song: None,
            settings: false,
            pasted: false,
            replaced: None,
            variant: None,
            scene: None,
            rows: None,
            row_tool: None,
            followed: None,
            chords: None,
            part: false,
            style: false,
        };
        let mut st = st;
        st.done = vec![take];
        st.undone = vec![take];
        st.capture = Some("2 bars".to_string());
        assert_eq!(
            action(&ev(TAKES, EventKind::Tab(0)), &st),
            Some(TimeAction::Undo)
        );
        assert_eq!(
            action(&ev(TAKES, EventKind::Tab(1)), &st),
            Some(TimeAction::Redo)
        );
        assert_eq!(
            action(&ev(TAKES, EventKind::Tab(2)), &st),
            Some(TimeAction::Capture)
        );
    }

    #[test]
    fn record_settings_step_within_their_range() {
        let mut st = state();
        let step = |st: &TimeState, index: usize, delta: i32| {
            action(&ev(RECORD, EventKind::Step { index, delta }), st)
        };
        // Armed: Stab (2) -> none -> Drums (0), and back.
        assert_eq!(step(&st, 0, 1), Some(TimeAction::Arm(None)));
        st.armed = None;
        assert_eq!(step(&st, 0, 1), Some(TimeAction::Arm(Some(0))));
        assert_eq!(step(&st, 0, -1), Some(TimeAction::Arm(Some(2))));
        assert_eq!(
            step(&st, 1, -1),
            Some(TimeAction::Transport(Command::ToggleOverdub))
        );
        assert_eq!(step(&st, 2, -1), None, "count-in is already off");
        assert_eq!(
            step(&st, 2, 1),
            Some(TimeAction::Transport(Command::SetCountIn(1)))
        );
        st.transport.count_in = MAX_COUNT_IN;
        assert_eq!(step(&st, 2, 1), None);
        assert_eq!(
            step(&st, 3, 1),
            Some(TimeAction::Transport(Command::SetLoop(Some((0, BAR)))))
        );
        st.transport.loop_range = Some((0, 4 * BAR));
        assert_eq!(
            step(&st, 3, 1),
            Some(TimeAction::Transport(Command::SetLoop(Some((0, 8 * BAR)))))
        );
        assert_eq!(
            step(&st, 3, -1),
            Some(TimeAction::Transport(Command::SetLoop(Some((0, 2 * BAR)))))
        );
        // A loop set elsewhere (bars 3 to 5) steps from the nearest length.
        st.transport.loop_range = Some((2 * BAR, 5 * BAR));
        assert_eq!(
            step(&st, 3, 1),
            Some(TimeAction::Transport(Command::SetLoop(Some((0, 4 * BAR)))))
        );
        // Following another device's position the transport's loop is off:
        // the row shows and steps the loop held until that stops.
        let following = TimeState {
            transport: TransportInfo {
                loop_range: Some((0, 2 * BAR)),
                follows_position: true,
                ..st.transport
            },
            ..st.clone()
        };
        assert_eq!(record_rows(&following)[3].value, "1–2, off while following");
        assert_eq!(
            step(&following, 3, 1),
            Some(TimeAction::Transport(Command::SetLoop(Some((0, 4 * BAR)))))
        );
        let none_held = TimeState {
            transport: TransportInfo {
                loop_range: None,
                ..following.transport
            },
            ..st.clone()
        };
        assert_eq!(record_rows(&none_held)[3].value, "off while following");
        assert_eq!(step(&none_held, 3, -1), None);
        assert_eq!(
            step(&st, 4, 1),
            Some(TimeAction::Transport(Command::SetTempo(121.0)))
        );
        // While the song's tempo points play, from the point in effect
        // (a ramp's start, not where it has glided to).
        st.tempo_here = Some(90.0);
        st.transport.bpm = 97.3;
        assert_eq!(
            step(&st, 4, 1),
            Some(TimeAction::Transport(Command::SetTempo(91.0)))
        );
        st.tempo_here = None;
        st.transport.bpm = MAX_BPM;
        assert_eq!(step(&st, 4, 1), None);
        // Pad pressure flips for the armed track, either way; with none
        // armed it shows a dash and does nothing.
        st.armed = Some(1);
        st.tracks[1].pressure = true;
        assert_eq!(record_rows(&st)[5].value, "Recorded");
        assert_eq!(
            step(&st, 5, -1),
            Some(TimeAction::RecordPressure {
                track: 1,
                on: false
            })
        );
        st.tracks[1].pressure = false;
        assert_eq!(record_rows(&st)[5].value, "Off");
        assert_eq!(
            step(&st, 5, 1),
            Some(TimeAction::RecordPressure { track: 1, on: true })
        );
        st.armed = None;
        assert_eq!(record_rows(&st)[5].value, "–");
        assert_eq!(step(&st, 5, 1), None);
        // Record quantise steps off, 1/4 ... 1/32 for the armed track, and
        // says why at its ends or with none armed.
        let why = |st: &TimeState, delta| {
            why_not(&ev(RECORD, EventKind::Step { index: 6, delta }), st).unwrap_or_default()
        };
        assert_eq!(record_rows(&st)[6].value, "–");
        assert_eq!(step(&st, 6, 1), None);
        assert!(why(&st, 1).starts_with("Arm a track first"));
        st.armed = Some(1);
        assert_eq!(record_rows(&st)[6].value, "Off");
        assert_eq!(step(&st, 6, -1), None);
        assert!(why(&st, -1).contains("off already"));
        assert_eq!(
            step(&st, 6, 1),
            Some(TimeAction::RecordQuantise {
                track: 1,
                grid: Some(PPQ)
            })
        );
        st.tracks[1].record_quantise = Some(PPQ / 4);
        assert_eq!(record_rows(&st)[6].value, "1/16");
        assert_eq!(
            step(&st, 6, 1),
            Some(TimeAction::RecordQuantise {
                track: 1,
                grid: Some(PPQ / 6)
            })
        );
        st.tracks[1].record_quantise = Some(PPQ);
        assert_eq!(
            step(&st, 6, -1),
            Some(TimeAction::RecordQuantise {
                track: 1,
                grid: None
            })
        );
        st.tracks[1].record_quantise = Some(PPQ / 8);
        assert_eq!(step(&st, 6, 1), None);
        assert!(why(&st, 1).contains("finest, 1/32"));
        // Launch: the project's line for tapped clips, stops and scenes,
        // from at once to 8 bars, whatever is armed.
        let why = |st: &TimeState, delta| {
            why_not(&ev(RECORD, EventKind::Step { index: 7, delta }), st).unwrap_or_default()
        };
        st.armed = None;
        assert_eq!(record_rows(&st)[7].value, "1 bar");
        assert_eq!(
            step(&st, 7, 1),
            Some(TimeAction::LaunchGrid(LaunchGrid::Bars(2)))
        );
        assert_eq!(
            step(&st, 7, -1),
            Some(TimeAction::LaunchGrid(LaunchGrid::Ticks(PPQ * 2)))
        );
        st.launch = LaunchGrid::Now;
        assert_eq!(record_rows(&st)[7].value, "at once");
        assert_eq!(step(&st, 7, -1), None);
        assert!(why(&st, -1).contains("at once already"));
        st.launch = LaunchGrid::Bars(8);
        assert_eq!(record_rows(&st)[7].value, "8 bars");
        assert_eq!(step(&st, 7, 1), None);
        assert!(why(&st, 1).contains("longest, 8 bars"));
        // One set in the file (3 bars) shows as it is and steps from 1 bar.
        st.launch = LaunchGrid::Bars(3);
        assert_eq!(record_rows(&st)[7].value, "3 bars");
        assert_eq!(
            step(&st, 7, 1),
            Some(TimeAction::LaunchGrid(LaunchGrid::Bars(2)))
        );
        // Transpose and Follows (sequencer S4, live transpose): the armed
        // track's, a semitone (or a note of the key) a step, within two
        // octaves; a dash and why not on a kit's track or with none armed.
        let why = |st: &TimeState, index, delta| {
            why_not(&ev(RECORD, EventKind::Step { index, delta }), st).unwrap_or_default()
        };
        assert_eq!(RECORD_ROWS[TRANSPOSE_ROW], "Transpose");
        assert_eq!(record_rows(&st)[TRANSPOSE_ROW].value, "–");
        assert!(why(&st, TRANSPOSE_ROW, 1).starts_with("Arm a track first"));
        st.armed = Some(1);
        st.tracks[1].melodic = false;
        assert_eq!(record_rows(&st)[FOLLOWS_ROW].value, "–");
        assert_eq!(step(&st, TRANSPOSE_ROW, 1), None);
        assert!(why(&st, TRANSPOSE_ROW, 1).contains("plays a kit"));
        st.tracks[1].melodic = true;
        assert_eq!(record_rows(&st)[TRANSPOSE_ROW].value, "as written");
        assert_eq!(
            step(&st, TRANSPOSE_ROW, 1),
            Some(TimeAction::Transpose {
                track: 1,
                to: Transpose::semitones(1)
            })
        );
        st.tracks[1].transpose = Transpose {
            by: -3,
            in_key: true,
        };
        assert_eq!(record_rows(&st)[TRANSPOSE_ROW].value, "-3 in key");
        assert_eq!(
            step(&st, TRANSPOSE_ROW, -1),
            Some(TimeAction::Transpose {
                track: 1,
                to: Transpose {
                    by: -4,
                    in_key: true
                }
            })
        );
        st.tracks[1].transpose = Transpose::semitones(MAX_TRANSPOSE);
        assert_eq!(record_rows(&st)[TRANSPOSE_ROW].value, "+24 st");
        assert_eq!(step(&st, TRANSPOSE_ROW, 1), None);
        assert!(why(&st, TRANSPOSE_ROW, 1).contains("highest, 24 semitones up"));
        assert_eq!(record_rows(&st)[FOLLOWS_ROW].value, "Off");
        assert_eq!(
            step(&st, FOLLOWS_ROW, -1),
            Some(TimeAction::FollowsTranspose { track: 1, on: true })
        );
        st.tracks[1].follows = true;
        assert_eq!(record_rows(&st)[FOLLOWS_ROW].value, "On");
    }

    /// The clip settings card: the focused clip's settings, each row
    /// stepping within its range and saying why at its ends.
    #[test]
    fn the_clip_card_steps_the_focused_clips_settings() {
        let mut st = state();
        // The armed track's playing clip, else its first; the first
        // track's while none is armed.
        assert_eq!(st.focus(), Some((2, 0)));
        st.armed = None;
        assert_eq!(st.focus(), Some((0, 0)));
        st.armed = Some(1);
        assert_eq!(st.focus(), Some((1, 1)));
        assert_eq!(panel_tabs(&st), ["Record", "Clip · B", "Chords"]);
        let rows: Vec<String> = clip_rows(&st)
            .iter()
            .map(|r| format!("{} {}", r.name, r.value))
            .collect();
        assert_eq!(
            rows,
            [
                "Length 7 steps",
                "Speed ×1",
                "Swing track",
                "Quantise off",
                "Strength —",
                "Follow off",
                "After —",
                "Launch project",
                "Mode Restart",
                "Program off",
                "Bank —",
                "Direction Forward",
                "Mutate —",
                "Melody pads",
                "Continue no tune",
                "Phrase no tune",
                "Style no notes"
            ]
        );
        let step = |st: &TimeState, index: usize, delta: i32| {
            let e = ev(CLIP_CARD, EventKind::Step { index, delta });
            (action(&e, st), why_not(&e, st))
        };
        let set = |st: &TimeState, row: u8, f: fn(&mut ClipSettings)| {
            let mut s = st.clip(1, 1).unwrap().settings;
            f(&mut s);
            (
                Some(TimeAction::ClipSettings {
                    track: 1,
                    slot: 1,
                    row,
                    settings: s,
                }),
                None,
            )
        };
        fn cs(st: &mut TimeState) -> &mut ClipSettings {
            &mut st.tracks[1].slots[1].as_mut().unwrap().settings
        }
        let ends = |got: (Option<TimeAction>, Option<String>), says: &str| {
            assert_eq!(got.0, None);
            assert!(
                got.1.as_deref().is_some_and(|w| w.contains(says)),
                "{got:?}"
            );
        };
        // Length, a sixteenth at a time.
        let length = |steps| {
            (
                Some(TimeAction::ClipLength {
                    track: 1,
                    slot: 1,
                    steps,
                    copy: false,
                }),
                None,
            )
        };
        assert_eq!(step(&st, 0, 1), length(8));
        assert_eq!(step(&st, 0, -1), length(6));
        st.tracks[1].slots[1].as_mut().unwrap().length = CARD_STEP;
        ends(step(&st, 0, -1), "shortest, 1 step");
        // Speed, from x1/8 to x4; one set elsewhere steps from the nearest.
        assert_eq!(
            step(&st, 1, 1),
            set(&st, 1, |s| s.speed = Speed::new(4, 3).unwrap())
        );
        assert_eq!(
            step(&st, 1, -1),
            set(&st, 1, |s| s.speed = Speed::new(3, 4).unwrap())
        );
        cs(&mut st).speed = Speed::new(3, 8).unwrap();
        assert_eq!(
            step(&st, 1, 1),
            set(&st, 1, |s| s.speed = Speed::new(1, 2).unwrap())
        );
        cs(&mut st).speed = Speed::new(1, 8).unwrap();
        ends(step(&st, 1, -1), "slowest");
        // Swing: the track's, then 50 to 75 %.
        ends(step(&st, 2, -1), "the track's already");
        assert_eq!(
            step(&st, 2, 1),
            set(&st, 2, |s| s.swing = Swing::new(240, 50))
        );
        cs(&mut st).swing = Swing::new(480, 75);
        assert_eq!(clip_rows(&st)[2].value, "75% 1/8");
        ends(step(&st, 2, 1), "at its most, 75 %");
        assert_eq!(
            step(&st, 2, -1),
            set(&st, 2, |s| s.swing = Swing::new(480, 74))
        );
        // Quantise: off, then 1/4 to 1/32 at full strength; strength only
        // while it is on, in tens.
        ends(step(&st, 4, 1), "Turn Quantise on first");
        ends(step(&st, 3, -1), "off already");
        assert_eq!(
            step(&st, 3, 1),
            set(&st, 3, |s| s.quantise = Quantise::new(960, 100))
        );
        cs(&mut st).quantise = Quantise::new(240, 75);
        assert_eq!(clip_rows(&st)[3].value, "1/16");
        assert_eq!(clip_rows(&st)[4].value, "75 %");
        assert_eq!(
            step(&st, 3, -1),
            set(&st, 3, |s| s.quantise = Quantise::new(320, 75))
        );
        assert_eq!(
            step(&st, 4, 1),
            set(&st, 4, |s| s.quantise = Quantise::new(240, 80))
        );
        assert_eq!(
            step(&st, 4, -1),
            set(&st, 4, |s| s.quantise = Quantise::new(240, 70))
        );
        cs(&mut st).quantise = Quantise::new(120, 100);
        ends(step(&st, 3, 1), "finest, 1/32");
        ends(step(&st, 4, 1), "full already");
        // Follow: off, then an action after one pass of the clip (7
        // sixteenths: 1 bar); After only while it is on.
        ends(step(&st, 6, 1), "Turn Follow on first");
        ends(step(&st, 5, -1), "off already");
        cs(&mut st).speed = Speed::NORMAL;
        st.tracks[1].slots[1].as_mut().unwrap().length = 1680;
        assert_eq!(
            step(&st, 5, 1),
            set(&st, 5, |s| s.follow =
                Some(Follow::after(1, FollowAction::Next)))
        );
        cs(&mut st).follow = Some(Follow::after(4, FollowAction::Next));
        assert_eq!(clip_rows(&st)[5].value, "Next");
        assert_eq!(clip_rows(&st)[6].value, "4 bars");
        assert_eq!(
            step(&st, 5, 1),
            set(&st, 5, |s| s.follow =
                Some(Follow::after(4, FollowAction::Previous)))
        );
        assert_eq!(
            step(&st, 6, 1),
            set(&st, 6, |s| s.follow =
                Some(Follow::after(5, FollowAction::Next)))
        );
        cs(&mut st).follow = Some(Follow::after(1, FollowAction::Stop));
        ends(step(&st, 5, 1), "last action, Stop");
        ends(step(&st, 6, -1), "soonest");
        // Launch: the project's, then the clip's own, at once to 8 bars.
        ends(step(&st, 7, -1), "the project's already (1 bar)");
        assert_eq!(
            step(&st, 7, 1),
            set(&st, 7, |s| s.launch = Some(LaunchGrid::Now))
        );
        cs(&mut st).launch = Some(LaunchGrid::Ticks(PPQ));
        assert_eq!(clip_rows(&st)[7].value, "1/4");
        assert_eq!(
            step(&st, 7, 1),
            set(&st, 7, |s| s.launch = Some(LaunchGrid::Ticks(PPQ * 2)))
        );
        assert_eq!(
            step(&st, 7, -1),
            set(&st, 7, |s| s.launch = Some(LaunchGrid::Ticks(PPQ / 2)))
        );
        cs(&mut st).launch = Some(LaunchGrid::Now);
        assert_eq!(clip_rows(&st)[7].value, "at once");
        assert_eq!(step(&st, 7, -1), set(&st, 7, |s| s.launch = None));
        cs(&mut st).launch = Some(LaunchGrid::Bars(8));
        assert_eq!(clip_rows(&st)[7].value, "8 bars");
        ends(step(&st, 7, 1), "longest, 8 bars");
        // Mode: Restart, Legato, Once.
        ends(step(&st, 8, -1), "first, Restart");
        assert_eq!(step(&st, 8, 1), set(&st, 8, |s| s.legato = true));
        cs(&mut st).legato = true;
        assert_eq!(clip_rows(&st)[8].value, "Legato");
        assert_eq!(step(&st, 8, -1), set(&st, 8, |s| s.legato = false));
        assert_eq!(
            step(&st, 8, 1),
            set(&st, 8, |s| {
                s.legato = false;
                s.once = true;
            })
        );
        cs(&mut st).legato = false;
        cs(&mut st).once = true;
        assert_eq!(clip_rows(&st)[8].value, "Once");
        ends(step(&st, 8, 1), "last, Once: the clip plays one pass");
        assert_eq!(
            step(&st, 8, -1),
            set(&st, 8, |s| {
                s.once = false;
                s.legato = true;
            })
        );
        cs(&mut st).once = false;
        // Program (S5): off, then 1 to 128; Bank only with a program: off,
        // then 0/0 up to 127/127, SHIFT a whole CC 0.
        ends(step(&st, 9, -1), "off already");
        ends(step(&st, 10, 1), "Set Program first");
        let prog = |program, bank| Some(ClipProgram { program, bank });
        assert_eq!(
            step(&st, 9, 1),
            set(&st, 9, |s| s.program = Some(ClipProgram {
                program: 0,
                bank: None
            }))
        );
        cs(&mut st).program = prog(4, None);
        assert_eq!(clip_rows(&st)[9].value, "5");
        assert_eq!(clip_rows(&st)[10].value, "off");
        assert_eq!(
            step(&st, 9, -1),
            set(&st, 9, |s| s.program = Some(ClipProgram {
                program: 3,
                bank: None
            }))
        );
        ends(step(&st, 10, -1), "Bank is off already");
        assert_eq!(
            step(&st, 10, 1),
            set(&st, 10, |s| s.program = Some(ClipProgram {
                program: 4,
                bank: Some(0)
            }))
        );
        cs(&mut st).program = prog(127, Some(130));
        assert_eq!(clip_rows(&st)[9].value, "128");
        assert_eq!(clip_rows(&st)[10].value, "1/2");
        ends(step(&st, 9, 1), "last, 128");
        let shifted = |st: &TimeState, delta| (card_action(st, 10, delta, true).ok(), None);
        assert_eq!(
            shifted(&st, 1),
            set(&st, 10, |s| s.program = Some(ClipProgram {
                program: 127,
                bank: Some(258)
            }))
        );
        assert_eq!(
            shifted(&st, -1),
            set(&st, 10, |s| s.program = Some(ClipProgram {
                program: 127,
                bank: Some(2)
            }))
        );
        cs(&mut st).program = prog(127, Some(5));
        assert_eq!(
            shifted(&st, -1),
            set(&st, 10, |s| s.program = Some(ClipProgram {
                program: 127,
                bank: Some(0)
            }))
        );
        cs(&mut st).program = prog(127, Some(0));
        assert_eq!(
            step(&st, 10, -1),
            set(&st, 10, |s| s.program = Some(ClipProgram {
                program: 127,
                bank: None
            }))
        );
        cs(&mut st).program = prog(127, Some(ClipProgram::MAX_BANK));
        assert_eq!(clip_rows(&st)[10].value, "127/127");
        ends(step(&st, 10, 1), "last, 127/127");
        // Its bank is kept as the program steps, and goes when it turns off.
        cs(&mut st).program = prog(0, Some(9));
        assert_eq!(
            step(&st, 9, 1),
            set(&st, 9, |s| s.program = Some(ClipProgram {
                program: 1,
                bank: Some(9)
            }))
        );
        assert_eq!(step(&st, 9, -1), set(&st, 9, |s| s.program = None));
        cs(&mut st).program = None;
        // A loop cell: only Follow, After and Launch.
        st.tracks[1].slots[1].as_mut().unwrap().looped = Some(0);
        ends(step(&st, 1, 1), "loop cell");
        ends(step(&st, 8, 1), "loop cell");
        ends(step(&st, 9, 1), "loop cell");
        ends(step(&st, 10, 1), "loop cell");
        assert!(step(&st, 6, 1).0.is_some());
        assert!(step(&st, 7, -1).0.is_some());
        // A settings take in the Takes list.
        let row = TakeRow {
            number: 4,
            at: Some((1, 1)),
            settings: true,
            ..TakeRow::of(&Take::default())
        };
        assert_eq!(row.short(), "settings 4");
        assert_eq!(take_title(&st, &row), "Settings 4 · Bass / 2");
        assert_eq!(take_detail(&st, &row), "clip settings");
        // No clip on the armed track: no rows, and it says so.
        st.tracks[1].slots = vec![None; ROWS];
        assert_eq!(st.focus(), None);
        assert!(clip_rows(&st).is_empty());
        assert_eq!(panel_tabs(&st)[1], "Clip");
        ends(step(&st, 0, 1), "No clip to set");
    }

    /// SHIFT + the card's arrows: Length doubles with its notes repeated or
    /// halves, Speed doubles or halves; the other rows step as without it.
    /// The Direction row (S6) steps Forward, Reverse, Ping-pong and
    /// Random, but moves only a clip of two whole 1/16 steps or more.
    #[test]
    fn the_clip_cards_direction_row_steps_the_play_direction() {
        let mut st = state();
        st.armed = Some(1);
        let step = |st: &TimeState, delta| match card_step(st, CLIP_DIRECTION, delta) {
            Ok(TimeAction::ClipSettings { row, settings, .. }) => {
                assert_eq!(usize::from(row), CLIP_DIRECTION);
                Ok(settings.direction)
            }
            other => Err(format!("{other:?}")),
        };
        let set = |st: &mut TimeState, d| {
            st.tracks[1].slots[1].as_mut().unwrap().settings.direction = d;
        };
        assert_eq!(step(&st, 1), Ok(Direction::Reverse));
        assert!(step(&st, -1).unwrap_err().contains("its first, Forward"));
        set(&mut st, Direction::PingPong);
        assert_eq!(step(&st, 1), Ok(Direction::Random));
        assert_eq!(step(&st, -1), Ok(Direction::Reverse));
        set(&mut st, Direction::Random);
        assert!(step(&st, 1).unwrap_err().contains("its last, Random"));
        let shown = |st: &TimeState| clip_rows(st)[CLIP_DIRECTION].value.clone();
        assert_eq!(shown(&st), "Random");
        // A clip of one step plays forward: the row says "—", a step
        // puts it back to Forward, and then says why it can't move.
        st.tracks[1].slots[1].as_mut().unwrap().length = CARD_STEP;
        assert_eq!(shown(&st), "—");
        assert_eq!(step(&st, 1), Ok(Direction::Forward));
        set(&mut st, Direction::Forward);
        assert_eq!(shown(&st), "Forward");
        assert_eq!(
            step(&st, 1),
            Err(format!("{:?}", Err::<(), _>("Direction moves the clip's 1/16 steps: it needs a clip of 2 whole steps or more, so this one plays forward")))
        );
        // A loop cell plays its loop as recorded.
        st.tracks[1].slots[1].as_mut().unwrap().looped = Some(0);
        assert!(step(&st, 1).unwrap_err().contains("loop cell"));
    }

    /// The card's Mutate row (S6): › a variation (SHIFT + › a bolder one),
    /// the pitch free to move on a melodic track; ‹ the original back once
    /// there is one. It reads "original", then "variant"; "—" with no
    /// notes or on a loop's cell.
    #[test]
    fn the_clip_cards_mutate_row_mutates_and_restores() {
        let mut st = state();
        st.armed = Some(1);
        st.tracks[1].slots[1].as_mut().unwrap().notes = 4;
        let shown = |st: &TimeState| clip_rows(st)[CLIP_MUTATE].value.clone();
        assert_eq!(clip_rows(&st).len(), CLIP_ROWS.len());
        assert_eq!(clip_rows(&st)[CLIP_MUTATE].name, "Mutate");
        assert_eq!(shown(&st), "original");
        let mutate = |more, melodic| {
            Ok(TimeAction::Mutate {
                track: 1,
                slot: 1,
                more,
                melodic,
            })
        };
        assert_eq!(
            card_action(&st, CLIP_MUTATE, 1, false),
            mutate(false, false)
        );
        assert_eq!(card_action(&st, CLIP_MUTATE, 1, true), mutate(true, false));
        st.tracks[1].melodic = true;
        assert_eq!(card_action(&st, CLIP_MUTATE, 1, true), mutate(true, true));
        assert!(card_action(&st, CLIP_MUTATE, -1, false)
            .unwrap_err()
            .contains("plays its original"));
        let c = st.tracks[1].slots[1].as_mut().unwrap();
        c.variation = Some(3);
        assert_eq!(shown(&st), "variant");
        assert_eq!(
            card_action(&st, CLIP_MUTATE, -1, true),
            Ok(TimeAction::Restore { track: 1, slot: 1 })
        );
        assert_eq!(card_action(&st, CLIP_MUTATE, 1, false), mutate(false, true));
        // No notes: nothing to vary, unless a variation is there to undo.
        st.tracks[1].slots[1].as_mut().unwrap().notes = 0;
        assert_eq!(shown(&st), "variant");
        st.tracks[1].slots[1].as_mut().unwrap().variation = None;
        assert_eq!(shown(&st), "—");
        assert!(card_action(&st, CLIP_MUTATE, 1, false)
            .unwrap_err()
            .contains("none yet"));
        // A loop cell plays its loop as recorded.
        st.tracks[1].slots[1].as_mut().unwrap().looped = Some(0);
        assert_eq!(shown(&st), "—");
        assert!(card_action(&st, CLIP_MUTATE, 1, false)
            .unwrap_err()
            .contains("loop cell"));
    }

    /// The clip card's Melody row (step sequencer refactor E1): "pads" or
    /// "notes"; › on a synth track's clip that follows the pads previews
    /// Convert to notes, and the rest say why. While the
    /// preview is open the buttons are Cancel, and Convert when it can,
    /// and the hint line says what it would do.
    #[test]
    fn the_clip_cards_melody_row_previews_convert_to_notes() {
        let mut st = state();
        st.armed = Some(1);
        let shown = |st: &TimeState| clip_rows(st)[CLIP_MELODY].value.clone();
        assert_eq!(clip_rows(&st)[CLIP_MELODY].name, "Melody");
        assert_eq!(shown(&st), "pads");
        // A kit's clip always follows the pads.
        assert!(card_action(&st, CLIP_MELODY, 1, false)
            .unwrap_err()
            .contains("on a kit a clip always does"));
        st.tracks[1].melodic = true;
        let preview = Ok(TimeAction::ConvertPreview { track: 1, slot: 1 });
        assert_eq!(card_action(&st, CLIP_MELODY, 1, false), preview);
        assert!(card_action(&st, CLIP_MELODY, -1, false)
            .unwrap_err()
            .contains("› shows what Convert to notes would do"));
        // The preview open: Cancel and Convert, its words on the hint line.
        let toggle = |st: &TimeState, b| action(&ev(CLIP_CARD, EventKind::Toggle(b)), st);
        assert_eq!(
            toggle(&st, 0),
            Some(TimeAction::SaveState { track: 1, slot: 1 })
        );
        st.converting = Some(Converting {
            track: 1,
            slot: 1,
            ok: true,
            line: "Convert to notes: 4 pad hits become 6 notes".to_string(),
        });
        let labels = |st: &TimeState| -> Vec<String> {
            card_buttons(st).into_iter().map(|b| b.label).collect()
        };
        assert_eq!(labels(&st), ["Cancel", "Convert"]);
        assert_eq!(
            clip_card_hint(&st),
            "Convert to notes: 4 pad hits become 6 notes"
        );
        assert_eq!(toggle(&st, 0), Some(TimeAction::ConvertCancel));
        assert_eq!(
            toggle(&st, 1),
            Some(TimeAction::Convert { track: 1, slot: 1 })
        );
        // It can't: Cancel alone.
        st.converting.as_mut().unwrap().ok = false;
        assert_eq!(labels(&st), ["Cancel"]);
        assert_eq!(toggle(&st, 1), None);
        // On another clip it shows nothing.
        st.armed = Some(0);
        assert_eq!(clip_card_hint(&st), CLIPS_CARD_HINT);
        assert_ne!(labels(&st), ["Cancel"]);
        // A clip of notes keeps its pitches; a loop's cell has no melody.
        st.armed = Some(1);
        st.converting = None;
        st.tracks[1].slots[1].as_mut().unwrap().holds_notes = true;
        assert_eq!(shown(&st), "notes");
        assert!(card_action(&st, CLIP_MELODY, 1, false)
            .unwrap_err()
            .contains("uses notes"));
        st.tracks[1].slots[1].as_mut().unwrap().looped = Some(0);
        assert_eq!(shown(&st), "—");
    }

    /// The card's Continue row (composing tools, phase 1 step 4): "+4
    /// bars" on a clip of notes, either arrow opening the note editor on
    /// it with Continue open; "no tune" on pad hits, "no notes" on an
    /// empty clip and "—" on a loop's cell, each saying why.
    #[test]
    fn the_clip_cards_continue_row_opens_continue_in_the_note_editor() {
        let mut st = state();
        st.armed = Some(1);
        let c = st.tracks[1].slots[1].as_mut().unwrap();
        c.notes = 4;
        c.holds = mpfree_seq::Holds::Notes;
        let shown = |st: &TimeState| clip_rows(st)[CLIP_CONTINUE].clone();
        let row = shown(&st);
        assert_eq!(
            (row.name.as_str(), row.value.as_str()),
            ("Continue", "+4 bars")
        );
        let open = Ok(TimeAction::Continue(ContinueAction::Open {
            track: 1,
            slot: 1,
        }));
        assert_eq!(card_action(&st, CLIP_CONTINUE, 1, false), open);
        assert_eq!(card_action(&st, CLIP_CONTINUE, -1, true), open);
        let why = |st: &TimeState| card_action(st, CLIP_CONTINUE, 1, false).unwrap_err();
        st.tracks[1].slots[1].as_mut().unwrap().holds = mpfree_seq::Holds::Hits;
        assert_eq!(shown(&st).value, "no tune");
        assert!(why(&st).contains("no tune to carry on"), "{}", why(&st));
        let c = st.tracks[1].slots[1].as_mut().unwrap();
        c.holds = mpfree_seq::Holds::Notes;
        c.notes = 0;
        assert_eq!(shown(&st).value, "no notes");
        assert!(why(&st).contains("some notes"), "{}", why(&st));
        st.tracks[1].slots[1].as_mut().unwrap().looped = Some(0);
        assert_eq!(shown(&st).value, "—");
        assert!(why(&st).contains("loop's cell"), "{}", why(&st));
    }

    /// The card's Phrase row (step 3 of the phrases): "Make ›" on a clip
    /// of notes, either arrow opening the Make a phrase sheet on it (the
    /// app's); on pad hits, an empty clip or a loop's cell it says why,
    /// as Continue's row does.
    #[test]
    fn the_clip_cards_phrase_row_opens_the_make_a_phrase_sheet() {
        let mut st = state();
        st.armed = Some(1);
        let c = st.tracks[1].slots[1].as_mut().unwrap();
        c.notes = 4;
        c.holds = mpfree_seq::Holds::Notes;
        let shown = |st: &TimeState| clip_rows(st)[CLIP_PHRASE].clone();
        let row = shown(&st);
        assert_eq!(
            (row.name.as_str(), row.value.as_str()),
            ("Phrase", "Make ›")
        );
        let open = Ok(TimeAction::MakePhrase { track: 1, slot: 1 });
        assert_eq!(card_action(&st, CLIP_PHRASE, 1, false), open);
        assert_eq!(card_action(&st, CLIP_PHRASE, -1, false), open);
        let why = |st: &TimeState| card_action(st, CLIP_PHRASE, 1, false).unwrap_err();
        st.tracks[1].slots[1].as_mut().unwrap().holds = mpfree_seq::Holds::Hits;
        assert_eq!(shown(&st).value, "no tune");
        assert!(why(&st).contains("pad hits"), "{}", why(&st));
        let c = st.tracks[1].slots[1].as_mut().unwrap();
        c.holds = mpfree_seq::Holds::Notes;
        c.notes = 0;
        assert_eq!(shown(&st).value, "no notes");
        assert!(why(&st).contains("some notes"), "{}", why(&st));
        st.tracks[1].slots[1].as_mut().unwrap().looped = Some(0);
        assert_eq!(shown(&st).value, "—");
        assert!(why(&st).contains("loop's cell"), "{}", why(&st));
    }

    /// The card's Style row (composing step 6, board D): "Make ›" on a
    /// clip with notes, pad hits too, either arrow asking for the Make a
    /// style sheet (the time engine's); on an empty clip or a loop's cell
    /// it says why.
    #[test]
    fn the_clip_cards_style_row_opens_the_make_a_style_sheet() {
        let mut st = state();
        st.armed = Some(1);
        let c = st.tracks[1].slots[1].as_mut().unwrap();
        c.notes = 4;
        c.holds = mpfree_seq::Holds::Hits;
        let shown = |st: &TimeState| clip_rows(st)[CLIP_STYLE].clone();
        let row = shown(&st);
        assert_eq!((row.name.as_str(), row.value.as_str()), ("Style", "Make ›"));
        let open = Ok(TimeAction::MakeStyle { track: 1, slot: 1 });
        assert_eq!(card_action(&st, CLIP_STYLE, 1, false), open);
        assert_eq!(card_action(&st, CLIP_STYLE, -1, false), open);
        let why = |st: &TimeState| card_action(st, CLIP_STYLE, 1, false).unwrap_err();
        st.tracks[1].slots[1].as_mut().unwrap().notes = 0;
        assert_eq!(shown(&st).value, "no notes");
        assert!(why(&st).contains("some notes"), "{}", why(&st));
        st.tracks[1].slots[1].as_mut().unwrap().looped = Some(0);
        assert_eq!(shown(&st).value, "—");
        assert!(why(&st).contains("loop's cell"), "{}", why(&st));
    }

    /// The sheet open, the page shows it in the card's column, and its
    /// touches but Name are the time engine's.
    #[test]
    fn the_make_a_style_sheet_takes_the_cards_column() {
        use crate::style_sheet::{StyleAction, STYLE_BACK, STYLE_KEEP, STYLE_NAME};
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        st.style_sheet = Some(Box::new(crate::sample::style_sheet_sample()));
        page.update(&mut ui, &st);
        assert_eq!(ui.layout_of(CLIPS_PAGE), Some(CLIPS_STYLE_LAYOUT));
        assert_eq!(
            ui.get::<StyleSheetView>(STYLE_SHEET)
                .unwrap()
                .sheet
                .as_ref(),
            st.style_sheet.as_deref()
        );
        let ev = |k| UiEvent {
            widget: STYLE_SHEET.to_string(),
            kind: EventKind::Toggle(k),
        };
        assert_eq!(action(&ev(STYLE_NAME), &st), None);
        assert_eq!(
            action(&ev(STYLE_BACK), &st),
            Some(TimeAction::Style(StyleAction::Back))
        );
        assert_eq!(
            action(&ev(STYLE_KEEP), &st),
            Some(TimeAction::Style(StyleAction::Keep))
        );
        st.song_view = true;
        page.update(&mut ui, &st);
        assert_eq!(ui.layout_of(CLIPS_PAGE), Some(SONG_STYLE_LAYOUT));
        st.style_sheet = None;
        page.update(&mut ui, &st);
        assert_eq!(ui.layout_of(CLIPS_PAGE), Some(SONG_LAYOUT));
    }

    #[test]
    fn shift_doubles_or_halves_the_cards_length_and_speed() {
        let mut st = state();
        st.armed = Some(1);
        // Bass's clip B: 7 sixteenths, at x1.
        let act = |st: &TimeState, index, delta| card_action(st, index, delta, true);
        assert_eq!(
            act(&st, 0, 1),
            Ok(TimeAction::ClipLength {
                track: 1,
                slot: 1,
                steps: 14,
                copy: true
            })
        );
        assert_eq!(
            act(&st, 0, -1),
            Ok(TimeAction::ClipLength {
                track: 1,
                slot: 1,
                steps: 3,
                copy: false
            })
        );
        let speed = |st: &TimeState, delta| match act(st, 1, delta) {
            Ok(TimeAction::ClipSettings {
                row: 1, settings, ..
            }) => Ok(speed_name(settings.speed)),
            other => Err(format!("{other:?}")),
        };
        assert_eq!(speed(&st, 1).as_deref(), Ok("×2"));
        assert_eq!(speed(&st, -1).as_deref(), Ok("×1/2"));
        let set = |st: &mut TimeState, n, d| {
            st.tracks[1].slots[1].as_mut().unwrap().settings.speed = Speed::new(n, d).unwrap();
        };
        set(&mut st, 3, 4);
        assert_eq!(speed(&st, 1).as_deref(), Ok("×3/2"));
        set(&mut st, 3, 1);
        assert!(speed(&st, 1).unwrap_err().contains("fastest"));
        set(&mut st, 1, 8);
        assert!(speed(&st, -1).unwrap_err().contains("slowest"));
        set(&mut st, 1, 3);
        assert!(speed(&st, -1).unwrap_err().contains("not a speed"));
        // The other rows as without SHIFT.
        assert_eq!(act(&st, 3, 1), card_step(&st, 3, 1));
        // A clip a sixteenth long can't halve.
        st.tracks[1].slots[1].as_mut().unwrap().length = CARD_STEP;
        assert!(act(&st, 0, -1).unwrap_err().contains("can't halve"));
        // A loop cell: neither.
        st.tracks[1].slots[1].as_mut().unwrap().looped = Some(0);
        assert!(act(&st, 1, 1).unwrap_err().contains("loop cell"));
    }

    /// COPY held: the hint says what a tap copies or pastes, and a paste
    /// is named in the takes list.
    #[test]
    fn copy_says_what_a_tap_copies_or_pastes() {
        let mut st = state();
        assert!(copy_hint(&st).starts_with("COPY held: tap a clip to copy it"));
        st.clipboard = Some("Verse B".into());
        assert!(copy_hint(&st).contains("a copy of Verse B goes there"));
        let row = TakeRow {
            number: 5,
            at: Some((1, 2)),
            made: true,
            pasted: true,
            ..TakeRow::of(&Take::default())
        };
        assert_eq!(row.short(), "paste 5");
        assert_eq!(take_title(&st, &row), "Paste 5 · Bass / 3");
        assert_eq!(take_detail(&st, &row), "a copy");
    }

    /// Add a part (composing tools S2.6, layout spec 4r): its button opens
    /// the panel in the side area on the scene pick, a try asked at once
    /// and the row heard for its chords; a choice asks again; Back closes
    /// it with nothing kept; Keep closes it once the part is kept.
    #[test]
    fn add_a_part_opens_its_panel_on_the_picked_row() {
        use crate::add_view::{AddPanel, PANEL_BACK, PANEL_KEEP, ROW_BARS};
        use crate::adding::{AddState, Adding};
        use mpfree_context::PartRole;
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Arrange"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        page.update(&mut ui, &st);
        let add = ev(CLIPS, EventKind::Toggle(ROWS + ADD_PART));
        assert!(page.owns(&add));
        // Row 1 is full: the panel opens on New track (S2.6b), for the
        // track Keep would add, named for the part, on its first synth;
        // Back closes it.
        st.scene_focus = 0;
        page.compose(&add, &st).unwrap();
        let p = page.add.as_ref().expect("open");
        assert!(p.new_track);
        assert_eq!((p.track, p.new_name.as_str()), (3, "Bass 2"));
        assert!(
            matches!(&page.take_asks()[..], [TimeAction::Add(AddAction::Ask(a))]
            if a.track == 3 && a.new_track == Some(crate::adding::NewTrack {
                name: "Bass 2".into(),
                sound: "mpfree.bass",
            }))
        );
        page.update(&mut ui, &st);
        page.compose(&ev(ADD_PANEL, EventKind::Toggle(PANEL_BACK)), &st)
            .unwrap();
        page.update(&mut ui, &st);
        assert!(page.add.is_none());
        assert!(page.take_asks().contains(&TimeAction::Add(AddAction::Back)));
        assert!(!page.take_kept(), "nothing kept");
        // Rec song on: faint, and a tap says why.
        st.song_rec = true;
        page.update(&mut ui, &st);
        assert_eq!(
            ui.get::<ClipGrid>(CLIPS).unwrap().add_part,
            PartButton::Faint
        );
        assert!(page.compose(&add, &st).unwrap_err().contains("Rec song"));
        st.song_rec = false;
        // Row 3 has room: the panel opens there, on the Bass track for a
        // Bass part, and asks at once; the row is heard for its chords.
        st.scene_focus = 2;
        page.take_asks();
        page.compose(&add, &st).unwrap();
        page.update(&mut ui, &st);
        assert_eq!(ui.layout_of(CLIPS_PAGE), Some(CLIPS_ADD_LAYOUT));
        assert_eq!(page.wheel_list(&ui), ADD_PANEL);
        let asks = page.take_asks();
        assert!(
            matches!(&asks[0], TimeAction::Add(AddAction::Ask(a))
                if (a.row, a.track, a.part, a.bars) == (2, 1, PartRole::Bass, None)),
            "{asks:?}"
        );
        assert!(asks.contains(&TimeAction::Chords(ChordsAction::Hear { row: Some(2) })));
        let g = ui.get::<ClipGrid>(CLIPS).unwrap();
        assert_eq!((g.add_part, g.outlined), (PartButton::Open, Some(2)));
        assert_eq!(ui.get::<AddPanel>(ADD_PANEL).unwrap().view.row, 2);
        // Melody: no track says it, so the first with room; a new try.
        page.compose(&ev(ADD_PANEL, EventKind::Select(2)), &st)
            .unwrap();
        assert!(
            matches!(&page.take_asks()[..], [TimeAction::Add(AddAction::Ask(a))]
            if (a.track, a.part) == (0, PartRole::Melody))
        );
        // Bars: one bar, then at its first it says so.
        let bars = |d| {
            ev(
                ADD_PANEL,
                EventKind::Step {
                    index: ROW_BARS,
                    delta: d,
                },
            )
        };
        page.compose(&bars(1), &st).unwrap();
        assert!(
            matches!(&page.take_asks()[..], [TimeAction::Add(AddAction::Ask(a))]
            if a.bars == Some(1))
        );
        page.compose(&bars(-1), &st).unwrap();
        page.take_asks();
        let why = page.compose(&bars(-1), &st).unwrap_err();
        assert!(why.contains("Bars"), "{why}");
        // Back: nothing kept, the panel closed, the row no longer heard.
        page.compose(&ev(ADD_PANEL, EventKind::Toggle(PANEL_BACK)), &st)
            .unwrap();
        page.update(&mut ui, &st);
        let asks = page.take_asks();
        assert!(asks.contains(&TimeAction::Add(AddAction::Back)), "{asks:?}");
        assert!(asks.contains(&TimeAction::Chords(ChordsAction::Hear { row: None })));
        assert_eq!(ui.layout_of(CLIPS_PAGE), Some(CLIPS_LAYOUT));
        assert_eq!(ui.get::<ClipGrid>(CLIPS).unwrap().outlined, None);
        // Keep: the panel stays while the time engine still has the try,
        // and closes once it is kept.
        page.compose(&add, &st).unwrap();
        st.adding = Some(Arc::new(Adding {
            row: 2,
            track: 1,
            state: AddState::Making,
            made: None,
            position: None,
            line: None,
            picked: None,
            words: None,
            part: PartRole::Bass,
            relation: mpfree_compose::Relation::Support,
        }));
        page.update(&mut ui, &st);
        // The part being made shows in its cell, dashed.
        let cell = &ui.get::<ClipGrid>(CLIPS).unwrap().tracks[1].cells[2];
        assert_eq!(
            (cell.mark, cell.detail.as_str()),
            (CellMark::Try, "making it")
        );
        page.compose(&ev(ADD_PANEL, EventKind::Toggle(PANEL_KEEP)), &st)
            .unwrap();
        page.update(&mut ui, &st);
        assert!(page.add.is_some());
        assert!(page.take_asks().contains(&TimeAction::Add(AddAction::Keep)));
        st.adding = None;
        page.update(&mut ui, &st);
        assert!(page.add.is_none());
        assert!(!page.take_asks().contains(&TimeAction::Add(AddAction::Back)));
        assert_eq!(ui.layout_of(CLIPS_PAGE), Some(CLIPS_LAYOUT));
    }

    /// The Style row runs on into phrases (P2, board H): after Jazz-ish,
    /// MPFree's phrases of the part's kind and then the owner's; the row
    /// reads Phrase, the ask carries the phrase, Try again is faint and
    /// says why, the Q-Links read "–" and shape nothing, and the hint says
    /// what the row does. Another part goes back to the style unless the
    /// phrase suits it; drums take none.
    #[test]
    fn the_style_row_runs_on_into_the_phrases_of_the_parts_kind() {
        use crate::add_view::{AddPanel, PANEL_TRY_AGAIN, PARTS, ROW_STYLE};
        use mpfree_arp::PhraseKind;
        use mpfree_context::PartRole;
        use mpfree_ui::Widget;
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Arrange"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut list = mpfree_arp_ui::PhraseList::built_in().phrases;
        let mut mine = list
            .iter()
            .find(|(_, p)| p.kind == PhraseKind::Bass)
            .unwrap()
            .1;
        mine.kind = PhraseKind::Bass;
        list.push(("Verse bass".to_string(), mine));
        page.set_phrases(list.clone().into());
        let mut st = state();
        st.scene_focus = 2;
        page.update(&mut ui, &st);
        page.compose(&ev(CLIPS, EventKind::Toggle(ROWS + ADD_PART)), &st)
            .unwrap();
        page.take_asks();
        let p = page.add.as_ref().unwrap();
        let basses = p.part_phrases();
        assert_eq!(basses.len(), 17, "MPFree's 16 bass lines and the owner's");
        assert!(basses.iter().all(|&i| list[i].1.kind == PhraseKind::Bass));
        assert_eq!(list[*basses.last().unwrap()].0, "Verse bass");
        // From Minimal, + three times: past Jazz-ish, the first bass line,
        // asked for at once.
        let step = |page: &mut TimePage, st: &TimeState, delta| {
            page.compose(
                &ev(
                    ADD_PANEL,
                    EventKind::Step {
                        index: ROW_STYLE,
                        delta,
                    },
                ),
                st,
            )
        };
        assert_eq!(page.add.as_ref().unwrap().value(&st, ROW_STYLE), "Minimal");
        step(&mut page, &st, 3).unwrap();
        let first = list[basses[0]].clone();
        let p = page.add.as_ref().unwrap();
        assert_eq!(p.value(&st, ROW_STYLE), first.0);
        assert!(
            matches!(&page.take_asks()[..], [TimeAction::Add(AddAction::Ask(a))]
                if a.phrase == Some(first.clone()))
        );
        page.update(&mut ui, &st);
        let panel = ui.get::<AddPanel>(ADD_PANEL).unwrap();
        assert!(panel.view.phrase);
        assert_eq!(panel.view.values[ROW_STYLE], first.0);
        let r = ui.rect_of(ADD_PANEL).unwrap();
        let targets = panel.targets(r);
        let again = targets.iter().find(|t| t.id == "add-try-again").unwrap();
        assert!(!again.enabled, "Try again is faint on a phrase");
        assert!(targets.iter().any(|t| t.label == "Phrase ›"));
        let why = page
            .compose(&ev(ADD_PANEL, EventKind::Toggle(PANEL_TRY_AGAIN)), &st)
            .unwrap_err();
        assert!(why.contains("plays one way"), "{why}");
        let no_ask = |page: &mut TimePage| {
            !page
                .take_asks()
                .iter()
                .any(|a| matches!(a, TimeAction::Add(AddAction::Ask(_))))
        };
        assert!(no_ask(&mut page));
        assert!(page
            .add
            .as_ref()
            .unwrap()
            .cards()
            .iter()
            .all(|(_, v)| v == "–"));
        assert!(page.add_knob(0, 3));
        assert!(page.add_knob_why().is_some() && no_ask(&mut page));
        assert_eq!(page.compose_hint(&st).as_deref(), Some(ADD_PHRASE_HINT));
        // To the owner's, the last: + says so.
        step(&mut page, &st, 40).unwrap();
        assert_eq!(
            page.add.as_ref().unwrap().value(&st, ROW_STYLE),
            "Verse bass"
        );
        page.take_asks();
        assert_eq!(
            step(&mut page, &st, 1).unwrap_err(),
            "Phrase is at its last choice"
        );
        // Chords: no bass line suits it, so the style again.
        let chords = PARTS.iter().position(|&r| r == PartRole::Chords).unwrap();
        page.compose(&ev(ADD_PANEL, EventKind::Select(chords)), &st)
            .unwrap();
        let p = page.add.as_ref().unwrap();
        assert!(p.phrase.is_none());
        assert_eq!(p.value(&st, ROW_STYLE), "Minimal");
        assert!(p
            .part_phrases()
            .iter()
            .all(|&i| list[i].1.kind == PhraseKind::Chords));
        // Drums: past the last style, why.
        let drums = PARTS.iter().position(|&r| r == PartRole::Drums).unwrap();
        page.compose(&ev(ADD_PANEL, EventKind::Select(drums)), &st)
            .unwrap();
        step(&mut page, &st, 9).unwrap();
        assert_eq!(page.add.as_ref().unwrap().value(&st, ROW_STYLE), "Jazz-ish");
        let why = step(&mut page, &st, 1).unwrap_err();
        assert!(why.contains("Drums take no phrase"), "{why}");
        // A melody's are the riffs and the arp lines.
        let line = PARTS.iter().position(|&r| r == PartRole::Melody).unwrap();
        page.compose(&ev(ADD_PANEL, EventKind::Select(line)), &st)
            .unwrap();
        let kinds: std::collections::BTreeSet<&str> = page
            .add
            .as_ref()
            .unwrap()
            .part_phrases()
            .iter()
            .map(|&i| list[i].1.kind.label())
            .collect();
        assert_eq!(kinds, ["Arp lines", "Riff"].into_iter().collect());
        // The list changed (another kept): the pick follows its name.
        step(&mut page, &st, 4).unwrap();
        let picked = page.add.as_ref().unwrap().phrase_of().unwrap();
        let mut more = list.clone();
        more.insert(0, ("New one".to_string(), mine));
        assert!(page.set_phrases(more.into()));
        assert_eq!(page.add.as_ref().unwrap().phrase_of(), Some(picked));
    }

    /// New track (composing tools S2.6b): the Track row steps past the
    /// tracks with room to New track and back; its Sound row steps the
    /// part's synths (Drums: only the Drums synth), while on a track it
    /// only reads; a part picked there takes its name and first synth; a
    /// track added while it is open moves it on; and with the project
    /// full a full row says why.
    /// Add a part's Style row runs on into the song's styles made from
    /// clips after the built-in ones (composing step 6), and a try asks
    /// with the made style; with none, the last built-in one is the end.
    #[test]
    fn add_a_parts_style_row_runs_on_into_the_made_styles() {
        use crate::add_view::{AddPick, ROW_STYLE};
        use mpfree_compose::STYLES;
        let mut st = state();
        let mut p = AddPick::open(&st, 2, None).unwrap();
        while p.step(&st, ROW_STYLE, 1).unwrap_or(false) {}
        assert_eq!(p.value(&st, ROW_STYLE), STYLES[STYLES.len() - 1].name);
        let sheet = crate::sample::style_sheet_sample();
        st.styles = Arc::from(vec![sheet.made.clone()]);
        let mut p = AddPick::open(&st, 2, None).unwrap();
        while p.step(&st, ROW_STYLE, 1).unwrap_or(false) {}
        assert_eq!(p.value(&st, ROW_STYLE), "Clip 1 groove");
        assert_eq!(p.settings().style.name, "Clip 1 groove");
        assert!(p.step(&st, ROW_STYLE, -1).unwrap());
        assert_eq!(p.value(&st, ROW_STYLE), STYLES[STYLES.len() - 1].name);
    }

    #[test]
    fn new_track_is_the_track_rows_last_choice_and_its_sound_row_picks_a_synth() {
        use crate::add_view::{AddPick, PARTS, ROW_SOUND, ROW_TRACK};
        use crate::adding::NewTrack;
        use mpfree_compose::InstrumentKind;
        use mpfree_context::PartRole;
        let mut st = state();
        // Row 3: every track has room; Bass opens on the Bass track.
        let mut p = AddPick::open(&st, 2, None).unwrap();
        assert_eq!((p.track, p.new_track), (1, false));
        assert!(p.ask().new_track.is_none());
        let why = p.step(&st, ROW_SOUND, 1).unwrap_err();
        assert!(why.contains("pick New track"), "{why}");
        // + to Stab, then New track, the last choice.
        assert!(p.step(&st, ROW_TRACK, 1).unwrap());
        assert_eq!(p.track, 2);
        assert!(p.step(&st, ROW_TRACK, 1).unwrap());
        assert!(p.new_track);
        assert_eq!((p.track, p.new_name.as_str()), (3, "Bass 2"));
        assert_eq!(p.value(&st, ROW_TRACK), "New track");
        assert_eq!(p.value(&st, ROW_SOUND), "Bass");
        assert_eq!(
            p.step(&st, ROW_TRACK, 1).unwrap_err(),
            "New track is the last choice"
        );
        let a = p.ask();
        assert_eq!(
            (a.track, a.kind, a.new_track),
            (
                3,
                InstrumentKind::Bass,
                Some(NewTrack {
                    name: "Bass 2".into(),
                    sound: "mpfree.bass"
                })
            )
        );
        // Sound steps the bass synths.
        assert!(p.step(&st, ROW_SOUND, 1).unwrap());
        assert_eq!(p.sound_of(), ("mpfree.squelch", "Squelch"));
        assert!(p.step(&st, ROW_SOUND, -5).unwrap());
        assert_eq!(p.sound_of().1, "Bass", "back to the first");
        // Chords: its own name and first synth, played as chords.
        let chords = PARTS.iter().position(|&r| r == PartRole::Chords).unwrap();
        assert!(p.set_part(&st, chords));
        assert_eq!((p.new_name.as_str(), p.sound_of().1), ("Chords", "Poly"));
        assert_eq!(
            p.ask().kind,
            crate::add_view::kind_of_part(PartRole::Chords)
        );
        // Drums: only the Drums synth, played as hits.
        let drums = PARTS.iter().position(|&r| r == PartRole::Drums).unwrap();
        assert!(p.set_part(&st, drums));
        assert_eq!(p.sound_of(), ("mpfree.drums", "Drums"));
        assert_eq!(p.ask().kind, InstrumentKind::Percussive);
        assert_eq!(
            p.step(&st, ROW_SOUND, 1).unwrap_err(),
            "Drums play on the Drums synth"
        );
        // A track added while it is open: New track is the one after it,
        // named again ("Drums" is taken).
        st.tracks.push(st.tracks[2].clone());
        assert!(p.follow_tracks(&st));
        assert_eq!((p.track, p.new_name.as_str()), (4, "Drums 2"));
        assert!(!p.follow_tracks(&st), "once");
        // − back to the last track with room.
        assert!(p.step(&st, ROW_TRACK, -1).unwrap());
        assert_eq!((p.track, p.new_track), (3, false));
        // A full row opens on New track; with the project full too, it
        // says why.
        let p = AddPick::open(&st, 0, None).unwrap();
        assert!(p.new_track && p.track == 4);
        while st.tracks.len() < mpfree_seq::MAX_TRACKS {
            st.tracks.push(st.tracks[0].clone());
        }
        let why = AddPick::open(&st, 0, None).unwrap_err();
        assert!(why.contains("holds no more tracks"), "{why}");
    }

    /// The Chords tab (composing tools S2.6): the third tab of the Record
    /// panel shows the scene pick's chords, hears its row, steps the row,
    /// ticks the clips off the new chords and fits the ticked ones; a
    /// clip off its row's chords is marked "doesn't fit".
    #[test]
    fn the_chords_tab_fits_the_ticked_clips() {
        use crate::add_view::{ChordsCard, CARD_FIT};
        use crate::adding::Unfit;
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Arrange"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        st.scene_focus = 0;
        st.unfit = vec![Unfit {
            row: 0,
            before: Default::default(),
            clips: vec![(1, 3), (2, 1)],
        }]
        .into();
        page.update(&mut ui, &st);
        // Marked in its cell, with "doesn't fit" for its length (track 1
        // is stopped; track 0 plays row 1 and is not marked).
        let g = ui.get::<ClipGrid>(CLIPS).unwrap();
        assert_eq!(g.tracks[1].cells[0].mark, CellMark::Unfit);
        assert_eq!(g.tracks[1].cells[0].detail, "doesn't fit");
        assert_eq!(g.tracks[0].cells[0].mark, CellMark::None);
        // The third tab.
        assert!(page.page_tracks(&ev(RECORD, EventKind::Tab(CHORDS_TAB)), &st));
        page.update(&mut ui, &st);
        assert_eq!(ui.layout_of(CLIPS_PAGE), Some(CLIPS_CHORDS_LAYOUT));
        assert_eq!(page.wheel_list(&ui), CHORDS_CARD);
        assert!(page
            .take_asks()
            .contains(&TimeAction::Chords(ChordsAction::Hear { row: Some(0) })));
        let card = ui.get::<ChordsCard>(CHORDS_CARD).unwrap();
        assert_eq!(card.head.tab, CHORDS_TAB);
        assert_eq!(card.view.verdict, "2 clips don't fit the new chords");
        assert_eq!(ui.get::<ClipGrid>(CLIPS).unwrap().outlined, Some(0));
        // Untick the second; Fit fits the first only.
        page.compose(&ev(CHORDS_CARD, EventKind::Select(1)), &st)
            .unwrap();
        page.update(&mut ui, &st);
        assert!(!ui.get::<ChordsCard>(CHORDS_CARD).unwrap().view.clips[1].3);
        page.compose(&ev(CHORDS_CARD, EventKind::Toggle(CARD_FIT)), &st)
            .unwrap();
        assert_eq!(
            page.take_asks(),
            [TimeAction::Chords(ChordsAction::Fit {
                row: 0,
                tracks: vec![1]
            })]
        );
        // ‹ at the first row says so; › picks the next.
        let row = |d| ev(CHORDS_CARD, EventKind::Step { index: 0, delta: d });
        assert!(page.compose(&row(-1), &st).is_err());
        page.compose(&row(1), &st).unwrap();
        assert_eq!(page.take_asks(), [TimeAction::SceneFocus(1)]);
        // The Record tab: the row is no longer heard.
        assert!(page.page_tracks(&ev(CHORDS_CARD, EventKind::Tab(0)), &st));
        page.update(&mut ui, &st);
        assert_eq!(ui.layout_of(CLIPS_PAGE), Some(CLIPS_LAYOUT));
        assert!(page
            .take_asks()
            .contains(&TimeAction::Chords(ChordsAction::Hear { row: None })));
    }

    /// The Record panel's Clip tab swaps the clip settings card in for the
    /// record settings, on the grid and the song alike, and the wheel
    /// steps it.
    #[test]
    fn the_record_panels_tabs_swap_in_the_clip_card() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Arrange"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        page.update(&mut ui, &st);
        assert_eq!(page.wheel_list(&ui), RECORD);
        let record = ui.get::<OptionList>(RECORD).unwrap();
        assert_eq!((record.tab, record.tabs.clone()), (0, panel_tabs(&st)));
        // The Clip tab.
        assert!(page.page_tracks(&ev(RECORD, EventKind::Tab(1)), &st));
        page.update(&mut ui, &st);
        assert_eq!(ui.layout_of(CLIPS_PAGE), Some(CLIPS_CARD_LAYOUT));
        assert_eq!(page.wheel_list(&ui), CLIP_CARD);
        let card = ui.get::<OptionList>(CLIP_CARD).unwrap();
        assert_eq!(card.tab, 1);
        assert_eq!(card.tabs[1], "Clip · A");
        assert_eq!(card.rows, clip_rows(&st));
        // The Song view keeps it, and its wheel still scrolls the song.
        st.song_view = true;
        page.update(&mut ui, &st);
        assert_eq!(ui.layout_of(CLIPS_PAGE), Some(SONG_CARD_LAYOUT));
        assert_eq!(page.scroll(&mut ui, 2), None);
        assert_eq!(page.first_bar, 2);
        // Another clip focused: the card follows.
        st.armed = Some(1);
        page.update(&mut ui, &st);
        assert_eq!(
            ui.get::<OptionList>(CLIP_CARD).unwrap().rows,
            clip_rows(&st)
        );
        // The Record tab again.
        assert!(page.page_tracks(&ev(CLIP_CARD, EventKind::Tab(0)), &st));
        page.update(&mut ui, &st);
        assert_eq!(ui.layout_of(CLIPS_PAGE), Some(SONG_LAYOUT));
        assert_eq!(page.wheel_list(&ui), RECORD);
    }

    /// The scene card (5.2, part 7 of S4 before it): open on a row over
    /// the grid, its rows for that scene, and what each row's ‹ › and
    /// the card's other taps ask for.
    #[test]
    fn the_scene_card_steps_the_scene_it_is_open_on() {
        use crate::scene_card as sc;
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Arrange"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        st.scene_focus = 1;
        page.update(&mut ui, &st);
        assert!(ui.get::<ClipGrid>(CLIPS).unwrap().scene_card.is_none());
        st.scene_open = Some(1);
        page.update(&mut ui, &st);
        // On the grid, over the Clips view's own layout: the Record panel
        // keeps its three tabs.
        assert_eq!(ui.layout_of(CLIPS_PAGE), Some(CLIPS_LAYOUT));
        assert_eq!(panel_tabs(&st).len(), 3);
        let card = ui
            .get::<ClipGrid>(CLIPS)
            .unwrap()
            .scene_card
            .clone()
            .unwrap();
        assert_eq!((card.row, card.name.as_str()), (1, "Row 2"));
        let shown = |st: &TimeState| -> Vec<String> {
            scene_rows(st)
                .iter()
                .map(|r| format!("{} {}", r.name, r.value))
                .collect()
        };
        assert_eq!(
            shown(&st),
            [
                "Scene 2",
                "Follow off",
                "After —",
                "Chance —",
                "Or —",
                "Mutes as they are",
                "Program off",
                "Bank —",
                "Sent to none",
                "Tempo as it is"
            ]
        );
        let step = |st: &TimeState, row: usize, delta: i32| {
            action(&ev(CLIPS, EventKind::Step { index: row, delta }), st)
        };
        let tap = |st: &TimeState, k: usize| action(&ev(CLIPS, EventKind::Toggle(k)), st);
        let set = |a: Option<TimeAction>| match a {
            Some(TimeAction::SceneSettings {
                row: 1, settings, ..
            }) => *settings,
            other => panic!("{other:?}"),
        };
        // ‹ › by the name: the scene before or after, its card.
        assert_eq!(
            tap(&st, sc::SCENE_BEFORE),
            Some(TimeAction::SceneCard(Some(0)))
        );
        assert_eq!(
            tap(&st, sc::SCENE_AFTER),
            Some(TimeAction::SceneCard(Some(2)))
        );
        let mut first = st.clone();
        first.scene_open = Some(0);
        first.scene_focus = 0;
        assert_eq!(tap(&first, sc::SCENE_BEFORE), None);
        assert_eq!(
            why_not(&ev(CLIPS, EventKind::Toggle(sc::SCENE_BEFORE)), &first).as_deref(),
            Some("Scene 1 is the first")
        );
        // Follow on: Next, after the scene's longest clip (B, under a
        // bar: 1 bar).
        st.scene = set(step(&st, 1, 1));
        assert_eq!(st.scene.follow, Some(Follow::after(1, FollowAction::Next)));
        assert_eq!(
            shown(&st)[1..5],
            ["Follow Next", "After 1 bar", "Chance 100 %", "Or —"]
        );
        // Or waits for a chance under 100 %.
        assert_eq!(step(&st, 4, 1), None);
        st.scene = set(step(&st, 3, -1));
        assert_eq!(st.scene.follow.map(|f| f.chance), Some(90));
        st.scene = set(step(&st, 4, 1));
        assert_eq!(st.scene.follow.map(|f| f.other), Some(OTHERS[1]));
        // Mutes: none to begin with, then – takes them off again; a tap on
        // its value takes the mutes as they are now.
        st.scene = set(step(&st, SCENE_MUTES, 1));
        assert_eq!(st.scene.mutes, Some(0));
        assert_eq!(shown(&st)[SCENE_MUTES], "Mutes none");
        st.tracks[0].mute = true;
        st.tracks[2].mute = true;
        assert_eq!(
            tap(&st, sc::SCENE_MUTES_NOW).map(|a| set(Some(a)).mutes),
            Some(Some(0b101))
        );
        st.scene.mutes = Some(0b101);
        assert_eq!(shown(&st)[SCENE_MUTES], "Mutes Drums, Stab");
        st.scene = set(step(&st, SCENE_MUTES, -1));
        assert_eq!(st.scene.mutes, None);
        // Follow off again.
        st.scene = set(step(&st, 1, -1));
        assert_eq!(st.scene.follow, None);
        // A name shows by the number, and on the card.
        st.scene.name = "Drop".into();
        assert_eq!(shown(&st)[0], "Scene 2 · Drop");
        page.update(&mut ui, &st);
        let card = ui
            .get::<ClipGrid>(CLIPS)
            .unwrap()
            .scene_card
            .clone()
            .unwrap();
        assert_eq!(card.name, "Drop");
        // The foot: Capture, Copy row, Insert row and Delete row.
        assert_eq!(tap(&st, sc::SCENE_CAPTURE), Some(TimeAction::CaptureScene));
        assert_eq!(
            tap(&st, sc::SCENE_COPY_ROW),
            Some(TimeAction::DropRow {
                from: 1,
                to: 2,
                copy: true
            })
        );
        assert_eq!(
            tap(&st, sc::SCENE_INSERT_ROW),
            Some(TimeAction::InsertRow(1))
        );
        assert_eq!(
            tap(&st, sc::SCENE_DELETE_ROW),
            Some(TimeAction::DeleteRow(1))
        );
        // Done closes it, and the grid has no card.
        assert_eq!(tap(&st, sc::SCENE_DONE), Some(TimeAction::SceneCard(None)));
        st.scene_open = None;
        page.update(&mut ui, &st);
        assert!(ui.get::<ClipGrid>(CLIPS).unwrap().scene_card.is_none());
    }

    /// The scene editor's Program and Bank (S5): the program change the
    /// scene sends the armed track, off then 1 to 128, its bank with it
    /// (SHIFT steps a whole CC 0); Sent to lists every track's.
    #[test]
    fn the_scene_card_sets_the_armed_tracks_program_change() {
        let mut st = state();
        st.scene_focus = 1;
        st.scene_open = Some(1);
        let shown = |st: &TimeState| -> Vec<String> {
            scene_rows(st)[SCENE_PROGRAM..SCENE_TEMPO]
                .iter()
                .map(|r| format!("{} {}", r.name, r.value))
                .collect()
        };
        let step = |st: &TimeState, row: usize, delta: i32| {
            action(&ev(CLIPS, EventKind::Step { index: row, delta }), st)
        };
        let why = |st: &TimeState, row: usize, delta: i32| {
            why_not(&ev(CLIPS, EventKind::Step { index: row, delta }), st)
        };
        let set = |a: Option<TimeAction>| match a {
            Some(TimeAction::SceneSettings {
                row: 1,
                card_row,
                settings,
            }) => {
                assert!(
                    usize::from(card_row) == SCENE_PROGRAM || usize::from(card_row) == SCENE_BANK
                );
                *settings
            }
            other => panic!("{other:?}"),
        };
        // Off, and no bank before a program.
        assert_eq!(step(&st, SCENE_PROGRAM, -1), None);
        assert!(why(&st, SCENE_PROGRAM, -1).unwrap().contains("off already"));
        assert_eq!(step(&st, SCENE_BANK, 1), None);
        assert!(why(&st, SCENE_BANK, 1)
            .unwrap()
            .starts_with("Set Program first"));
        // + : program 1, then 2.
        st.scene = set(step(&st, SCENE_PROGRAM, 1));
        st.scene = set(step(&st, SCENE_PROGRAM, 1));
        assert_eq!(
            st.scene.program(2),
            Some(ClipProgram {
                program: 1,
                bank: None
            })
        );
        assert_eq!(shown(&st), ["Program 2", "Bank off", "Sent to Stab"]);
        // Bank 0/0, SHIFT a whole CC 0 on, then back to off.
        st.scene = set(step(&st, SCENE_BANK, 1));
        assert_eq!(shown(&st)[1], "Bank 0/0");
        st.scene = set(scene_action(&st, SCENE_BANK, 1, true).ok());
        assert_eq!(shown(&st)[1], "Bank 1/0");
        st.scene = set(scene_action(&st, SCENE_BANK, -1, true).ok());
        st.scene = set(step(&st, SCENE_BANK, -1));
        assert_eq!(shown(&st)[1], "Bank off");
        // Another track armed: its own, and Sent to lists them all.
        st.armed = Some(0);
        assert_eq!(shown(&st)[..2], ["Program off", "Bank —"]);
        st.scene = set(step(&st, SCENE_PROGRAM, 1));
        assert_eq!(
            shown(&st)[..3],
            ["Program 1", "Bank off", "Sent to Drums, Stab"]
        );
        st.scene.set_program(
            1,
            Some(ClipProgram {
                program: 127,
                bank: None,
            }),
        );
        assert_eq!(shown(&st)[2], "Sent to Drums +2");
        st.armed = Some(1);
        assert_eq!(step(&st, SCENE_PROGRAM, 1), None);
        assert_eq!(
            why(&st, SCENE_PROGRAM, 1).as_deref(),
            Some("Program is at its last, 128")
        );
        // Off again: – from 1.
        st.armed = Some(0);
        st.scene = set(step(&st, SCENE_PROGRAM, -1));
        assert_eq!(st.scene.program(0), None);
        // Nothing armed: nothing to set, and it says so.
        st.armed = None;
        assert_eq!(shown(&st)[..2], ["Program —", "Bank —"]);
        assert!(why(&st, SCENE_PROGRAM, 1)
            .unwrap()
            .starts_with("Arm a track first"));
        assert!(why(&st, 8, 1)
            .unwrap()
            .starts_with("Arm a track and set Program"));
    }

    /// Save state and Reload on the clip card, Reload clip on the morph
    /// bar (S4): lit once the focused clip has a saved state, outlined
    /// while it waits, and NEXT names it in Perform.
    #[test]
    fn save_state_and_reload_show_on_the_card_and_the_morph_bar() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Arrange"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        let (track, slot) = st.focus().unwrap();
        let focus = (track, slot);
        page.update(&mut ui, &st);
        let card = ui.get::<OptionList>(CLIP_CARD).unwrap();
        assert_eq!(
            card.buttons,
            [
                OptionButton::new("Save state", true),
                OptionButton::new("Reload", false)
            ]
        );
        let morph = ui.get::<MorphBar>(MORPH).unwrap();
        assert!(!morph.reload && !morph.reload_waiting);
        let save = ev(CLIP_CARD, EventKind::Toggle(0));
        let card_reload = ev(CLIP_CARD, EventKind::Toggle(1));
        let morph_reload = ev(MORPH, EventKind::Tab(3));
        assert_eq!(
            action(&save, &st),
            Some(TimeAction::SaveState { track, slot })
        );
        assert_eq!(action(&card_reload, &st), None);
        assert_eq!(action(&morph_reload, &st), None);
        assert!(why_not(&morph_reload, &st)
            .unwrap()
            .starts_with("Save state on the Clip card first"));
        // Saved: both reloads are lit.
        st.saved = vec![focus];
        page.update(&mut ui, &st);
        assert!(ui.get::<OptionList>(CLIP_CARD).unwrap().buttons[1].enabled);
        assert!(ui.get::<MorphBar>(MORPH).unwrap().reload);
        let reload = Some(TimeAction::Reload { track, slot });
        assert_eq!(action(&card_reload, &st), reload);
        assert_eq!(action(&morph_reload, &st), reload);
        // Waiting for bar 3: outlined, and NEXT says so in Perform.
        st.reloads = vec![(track, slot, 2 * BAR)];
        st.transport.state = State::Playing;
        st.transport.position = 2 * BAR - 480;
        st.cues.clear();
        st.scene_cued = None;
        page.update(&mut ui, &st);
        assert!(ui.get::<OptionList>(CLIP_CARD).unwrap().buttons[1].waiting);
        assert!(ui.get::<MorphBar>(MORPH).unwrap().reload_waiting);
        assert_eq!(st.next_reading(), None, "Make");
        st.perform = true;
        assert_eq!(st.next_reading().as_deref(), Some("A reloads in 1 beat"));
        // No clip in focus: no buttons.
        st.armed = Some(9);
        assert!(card_buttons(&st).is_empty());
    }

    fn project() -> Project {
        let mut p = Project::new();
        for (i, name) in ["Drums", "Bass", "Stab"].iter().enumerate() {
            let mut t = Track::new(name, i as u16);
            t.slots[0] = Some(Clip::new("A", 2 * BAR));
            t.slots[1] = Some(Clip::new("B", 1680));
            p.tracks.push(t);
        }
        p
    }

    #[test]
    fn while_playing_launches_and_stops_wait_as_cues() {
        let st = state();
        let (p, t) = (project(), st.transport);
        let act = |l: &[Launch], c: &[Cue], a: TimeAction| clips_after(&p, l, c, &a, &t);
        let (l, c) = act(&st.launches, &[], TimeAction::Launch { track: 0, slot: 1 });
        assert_eq!(
            (l.as_slice(), c.as_slice()),
            (
                st.launches.as_slice(),
                &[Cue {
                    track: 0,
                    slot: Some(1)
                }][..]
            ),
            "the launches stay until the bar line"
        );
        // A stop on the same track takes the launch's place; another track
        // gets its own cue.
        let (l, c) = act(&l, &c, TimeAction::StopClip(0));
        assert_eq!(
            c,
            vec![Cue {
                track: 0,
                slot: None
            }]
        );
        let (l, c) = act(&l, &c, TimeAction::Launch { track: 2, slot: 0 });
        assert_eq!(
            c,
            vec![
                Cue {
                    track: 0,
                    slot: None
                },
                Cue {
                    track: 2,
                    slot: Some(0)
                }
            ]
        );
        let (l, c) = act(&l, &c, TimeAction::Uncue(0));
        assert_eq!(
            c,
            vec![Cue {
                track: 2,
                slot: Some(0)
            }]
        );
        // A new clip launches at once, and its track's cue goes.
        let (l, c) = act(&l, &c, TimeAction::NewClip { track: 2, slot: 4 });
        assert_eq!(
            (l.last(), c.len()),
            (
                Some(&Launch {
                    track: 2,
                    slot: 4,
                    origin: 5 * BAR,
                    start: 5 * BAR,
                }),
                0
            )
        );
        assert_eq!(act(&l, &c, TimeAction::Undo), (l.clone(), c.clone()));
    }

    #[test]
    fn while_stopped_a_launch_happens_at_once_and_plays_wherever_the_song_goes() {
        let st = state();
        let p = project();
        let t = TransportInfo {
            state: State::Stopped,
            position: 3 * BAR + 100,
            ..st.transport
        };
        let cues = [Cue {
            track: 0,
            slot: Some(1),
        }];
        let (l, c) = clips_after(
            &p,
            &st.launches,
            &cues,
            &TimeAction::Launch { track: 0, slot: 1 },
            &t,
        );
        assert!(c.is_empty());
        let origin = anchored(3 * BAR, 1680, Speed::NORMAL);
        assert_eq!(
            l,
            vec![
                Launch {
                    track: 1,
                    slot: 1,
                    origin: 5 * BAR,
                    start: 5 * BAR,
                },
                Launch {
                    track: 0,
                    slot: 1,
                    origin,
                    start: 3 * BAR,
                }
            ]
        );
        let (l, _) = clips_after(&p, &l, &[], &TimeAction::StopClip(1), &t);
        assert_eq!(
            l,
            vec![Launch {
                track: 0,
                slot: 1,
                origin,
                start: 3 * BAR,
            }]
        );
    }

    #[test]
    fn each_track_shows_what_it_plays_now_and_what_comes_next() {
        // The Arrange refactor's 5.1 (D3). Drums plays clip A from 1.1,
        // Bass waits for 6.1, Stab has nothing launched.
        let mut st = state();
        assert_eq!(st.plays(0), Plays::Clip(0));
        assert_eq!(st.plays(1), Plays::Nothing, "waits for its line");
        assert_eq!(st.plays(2), Plays::Nothing);
        assert!(
            (0..3).all(|k| st.song_mark(k) == SongMark::None),
            "song mode off"
        );
        assert!(st.lanes_over().is_empty());
        // Song mode with blocks on Drums and Stab: Stab plays the song,
        // Drums a clip over it; Bass, with nothing in the song, no tag.
        st.song_on = true;
        st.placements = [0, 2]
            .into_iter()
            .map(|track| PlacementInfo {
                track,
                start: 0,
                length: 8 * BAR,
                name: "A".to_string(),
                unique: false,
            })
            .collect::<Vec<_>>()
            .into();
        assert_eq!(st.plays(2), Plays::Song);
        assert_eq!(st.song_mark(0), SongMark::Under);
        assert_eq!(st.song_mark(1), SongMark::None);
        assert_eq!(st.song_mark(2), SongMark::Plays);
        let tags: Vec<SongMark> = grid_tracks(&st).iter().map(|t| t.song).collect();
        assert_eq!(tags, [SongMark::Under, SongMark::None, SongMark::Plays]);
        // A muted track still says what it would play.
        st.tracks[2].silent = true;
        assert_eq!(st.song_mark(2), SongMark::Plays);
        // On the Song view: Drums' clip over its lane from where it
        // started, to the view's end; Bass's, waiting, from its line.
        let over = |track, from, until, back, waiting| LaneOver {
            track,
            name: if track == 0 { "A" } else { "B" }.to_string(),
            from,
            until,
            back,
            waiting,
        };
        assert_eq!(
            st.lanes_over(),
            [
                over(0, 0, None, false, false),
                over(1, 5 * BAR, None, false, true)
            ]
        );
        // Back to song on Drums: a stop cue. The cell says when the song
        // comes back, in amber; the Song view ends the clip there.
        st.cues = vec![Cue {
            track: 0,
            slot: None,
        }];
        let c = cell_of(&st, 0, 0);
        assert_eq!(
            (c.state, c.detail.as_str()),
            (CellState::Playing, "to song 6.1")
        );
        assert!(c.cued);
        assert_eq!(c.countdown, 0.0, "Make has no countdowns");
        assert_eq!(st.lanes_over()[0], over(0, 0, Some(5 * BAR), true, false));
        // In Perform, the stop counts down, as a queued cell does.
        st.perform = true;
        let c = cell_of(&st, 0, 0);
        assert!(c.countdown > 0.0 && c.countdown < 1.0, "{}", c.countdown);
        // Off song mode the same cue only stops it.
        st.song_on = false;
        assert_eq!(cell_of(&st, 0, 0).detail, "stops 6.1");
        // Another clip cued on Drums: A plays until its line, B waits
        // from there; neither counts down on A's cell.
        st.song_on = true;
        st.cues = vec![Cue {
            track: 0,
            slot: Some(1),
        }];
        let c = cell_of(&st, 0, 0);
        assert_eq!(
            (c.detail.as_str(), c.cued, c.countdown),
            ("until 6.1", true, 0.0)
        );
        let mut b = over(0, 5 * BAR, None, false, true);
        b.name = "B".to_string();
        assert_eq!(
            st.lanes_over()[..2],
            [over(0, 0, Some(5 * BAR), false, false), b]
        );
    }

    #[test]
    fn cued_cells_show_where_they_start_and_taps_take_cues_back() {
        let mut st = state();
        st.cues = vec![
            Cue {
                track: 0,
                slot: Some(1),
            },
            Cue {
                track: 2,
                slot: None,
            },
        ];
        st.launches.push(Launch {
            track: 2,
            slot: 0,
            origin: 0,
            start: 0,
        });
        let q = cell_of(&st, 0, 1);
        assert_eq!((q.state, q.foot.as_str()), (CellState::Queued, "from 6.1"));
        let a = cell_of(&st, 0, 0);
        assert_eq!(
            (a.state, a.detail.as_str()),
            (CellState::Playing, "until 6.1")
        );
        let s = cell_of(&st, 2, 0);
        assert_eq!(
            (s.state, s.detail.as_str()),
            (CellState::Playing, "stops 6.1")
        );
        // In a loop of bars 4 and 5, the next bar line played is bar 4's.
        st.transport.loop_range = Some((3 * BAR, 5 * BAR));
        assert_eq!(cell_of(&st, 0, 1).foot, "from 4.1");
        assert_eq!(cue_tick(&st.transport, LaunchGrid::Bars(1)), 3 * BAR);
        st.transport.loop_range = None;
        // The cued clip's own launch quantise moves its label; the time
        // engine's line for a cue (one kept when Launch changed) wins.
        st.tracks[0].slots[1].as_mut().unwrap().settings.launch = Some(LaunchGrid::Ticks(PPQ * 2));
        assert_eq!(st.cue_line(0), LaunchGrid::Ticks(PPQ * 2));
        assert_eq!(cell_of(&st, 0, 1).foot, "from 5.3");
        st.cue_lines = vec![(0, LaunchGrid::Bars(2))];
        assert_eq!(cell_of(&st, 0, 1).foot, "from 7.1");
        assert_eq!(cell_of(&st, 0, 0).detail, "until 7.1");
        st.launch = LaunchGrid::Bars(4);
        assert_eq!(st.cue_line(2), LaunchGrid::Bars(4), "a stop: the project's");
        assert_eq!(cell_of(&st, 2, 0).detail, "stops 9.1");
        // A one-shot's stop lands where its pass ends, on no bar line.
        st.cue_lines = vec![(2, LaunchGrid::At(5 * BAR + 3 * Tick::from(PPQ)))];
        assert_eq!(cell_of(&st, 2, 0).detail, "stops 6.4");
        st.cue_lines.clear();
        st.launch = LaunchGrid::Bars(1);
        let tap = |track: usize, slot: usize| {
            action(&ev(CLIPS, EventKind::Select(track * ROWS + slot)), &st)
        };
        assert_eq!(tap(0, 1), Some(TimeAction::Uncue(0)), "the cued clip");
        assert_eq!(
            tap(2, 0),
            Some(TimeAction::Uncue(2)),
            "the clip with a stop cued"
        );
        assert_eq!(tap(0, 0), Some(TimeAction::StopClip(0)), "the playing clip");
        assert_eq!(tap(2, 1), Some(TimeAction::Launch { track: 2, slot: 1 }));
    }

    /// The mute queue (S4): each waiting track's grid header says what
    /// it does and how soon, and the transport bar's WAITING reading
    /// names the tracks and their line.
    #[test]
    fn mutes_waiting_show_on_the_grid_and_the_transport_bar() {
        let mut st = state();
        assert_eq!((st.waiting_reading(), st.mute_waiting(1)), (None, None));
        assert_ne!(bar_of(&st).items[0].label, "WAITING");
        st.mute_wait = Some(MuteWait {
            want: vec![(1, true, false), (2, false, true)],
            ..MuteWait::new(LaunchGrid::Bars(1))
        });
        // At 5.2: three beats to bar 6.
        assert_eq!(st.mute_waiting(1).as_deref(), Some("mutes in 3 beats"));
        assert_eq!(st.mute_waiting(2).as_deref(), Some("solo in 3 beats"));
        assert_eq!(st.mute_waiting(0), None);
        assert_eq!(st.waiting_reading().as_deref(), Some("Bass, Stab at 6.1"));
        let item = &bar_of(&st).items[0];
        assert_eq!(
            (item.label.as_str(), item.value.as_str(), item.tone),
            ("WAITING", "Bass, Stab at 6.1", Tone::Warn)
        );
        let g = grid_tracks(&st);
        assert_eq!(g[1].waiting.as_deref(), Some("mutes in 3 beats"));
        assert_eq!(g[0].waiting, None);
        // A longer line counts bars; more than two tracks are counted.
        st.mute_wait = Some(MuteWait {
            want: vec![(0, true, false), (1, true, false), (2, true, false)],
            ..MuteWait::new(LaunchGrid::Bars(2))
        });
        assert_eq!(st.mute_waiting(0).as_deref(), Some("mutes in 2 bars"));
        assert_eq!(
            st.waiting_reading().as_deref(),
            Some("Drums, Bass +1 at 7.1")
        );
        // A track's pads wait with them, named by track and pad (step
        // 10); alone, too.
        st.mute_wait = Some(MuteWait {
            want: vec![(1, true, false)],
            pads: vec![(0, 2, true, false)],
            ..MuteWait::new(LaunchGrid::Bars(1))
        });
        assert_eq!(
            st.waiting_reading().as_deref(),
            Some("Bass, Drums A3 at 6.1")
        );
        st.mute_wait = Some(MuteWait {
            pads: vec![(2, 20, false, true)],
            ..MuteWait::new(LaunchGrid::Bars(1))
        });
        assert_eq!(st.waiting_reading().as_deref(), Some("Stab B5 at 6.1"));
        assert_eq!(st.mute_waiting(0), None, "a pad is no track's");
        // Solo: the soloed track is outlined, the others are silent.
        st.mute_wait = None;
        st.tracks[2].solo = true;
        st.tracks[0].silent = true;
        let g = grid_tracks(&st);
        assert!(g[2].solo && !g[2].mute && g[0].mute);
    }

    #[test]
    fn perform_shows_big_rows_countdowns_and_what_comes_next() {
        // Make: eight compact rows; Perform: five big ones.
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        page.update(&mut ui, &st);
        assert_eq!(page.visible_rows(&ui), 8);
        st.perform = true;
        page.update(&mut ui, &st);
        assert_eq!(page.visible_rows(&ui), 5);
        // A queued clip says where it starts; in Perform it counts down
        // too: at 5.2, a quarter of the bar to 6.1 has gone.
        st.perform = false;
        let c = cell_of(&st, 1, 1);
        assert_eq!(
            (c.state, c.foot.as_str(), c.progress),
            (CellState::Queued, "from 6.1", 0.0)
        );
        st.perform = true;
        let c = cell_of(&st, 1, 1);
        assert_eq!((c.foot.as_str(), c.progress), ("from 6.1", 0.25));
        // NEXT: nothing coming.
        assert_eq!(st.next_reading(), None);
        // Clips cued: the first by name and how many more.
        st.cues = vec![Cue {
            track: 2,
            slot: Some(1),
        }];
        assert_eq!(st.next_reading().as_deref(), Some("B in 3 beats"));
        let c = cell_of(&st, 2, 1);
        assert_eq!((c.foot.as_str(), c.progress), ("from 6.1", 0.25));
        st.cues.push(Cue {
            track: 0,
            slot: None,
        });
        assert_eq!(st.next_reading().as_deref(), Some("B +1 in 3 beats"));
        st.cues = vec![Cue {
            track: 0,
            slot: None,
        }];
        assert_eq!(st.next_reading().as_deref(), Some("Drums stops in 3 beats"));
        // A scene launched names the scene.
        st.scenes = vec![String::new(), "Drop".into()];
        st.scene_cued = Some(1);
        assert_eq!(st.next_reading().as_deref(), Some("Drop in 3 beats"));
        let item = &bar_of(&st).items[0];
        assert_eq!(
            (item.label.as_str(), item.value.as_str(), item.tone),
            ("NEXT", "Drop in 3 beats", Tone::Accent)
        );
        // The scene playing's follow action, bars rounded up.
        st.cues.clear();
        st.scene_cued = None;
        let follow = |action, other, chance| FollowNext {
            at: 8 * BAR,
            row: 0,
            follow: Follow {
                bars: 4,
                action,
                other,
                chance,
            },
        };
        st.follow_next = Some(follow(FollowAction::Next, FollowAction::Stay, 100));
        assert_eq!(st.next_reading().as_deref(), Some("Drop in 4 bars"));
        st.follow_next = Some(follow(FollowAction::Next, FollowAction::Stay, 70));
        assert_eq!(st.next_reading().as_deref(), Some("Drop or stay in 4 bars"));
        st.follow_next = Some(follow(FollowAction::Again, FollowAction::Stay, 100));
        assert_eq!(st.next_reading().as_deref(), Some("row 1 again in 4 bars"));
        st.follow_next = Some(follow(FollowAction::Other, FollowAction::Stay, 100));
        assert_eq!(
            st.next_reading().as_deref(),
            Some("Another scene in 4 bars")
        );
        st.follow_next = Some(follow(FollowAction::Stay, FollowAction::Next, 100));
        assert_eq!(st.next_reading(), None);
        // Song mode: the next marker.
        st.follow_next = None;
        st.song_on = true;
        st.markers = vec![(2 * BAR, "Intro".into()), (6 * BAR, "Chorus".into())];
        assert_eq!(st.next_reading().as_deref(), Some("Chorus in 2 bars"));
        // WAITING goes before NEXT.
        st.mute_wait = Some(MuteWait {
            want: vec![(1, true, false)],
            ..MuteWait::new(LaunchGrid::Bars(1))
        });
        let items = bar_of(&st).items;
        assert_eq!(
            (items[0].label.as_str(), items[1].label.as_str()),
            ("WAITING", "NEXT")
        );
        // Nothing in Make, or stopped.
        st.perform = false;
        assert_eq!(st.next_reading(), None);
        st.perform = true;
        st.transport.state = State::Stopped;
        assert_eq!(st.next_reading(), None);
    }

    #[test]
    fn countdowns_round_up_to_beats_then_bars() {
        let m = Meter::default();
        let beat = m.beat_ticks();
        let bar = m.bar_ticks();
        assert_eq!(countdown(1, m), "1 beat");
        assert_eq!(countdown(beat + 1, m), "2 beats");
        assert_eq!(countdown(bar - 1, m), "4 beats");
        assert_eq!(countdown(bar, m), "1 bar");
        assert_eq!(countdown(bar + 1, m), "2 bars");
        assert_eq!(countdown(0, m), "1 beat");
    }

    #[test]
    fn a_launch_starts_on_the_next_bar_and_replaces_the_tracks_clip() {
        let st = state();
        let t = st.transport;
        let after = launches_after(&st.launches, &TimeAction::Launch { track: 0, slot: 1 }, &t);
        assert_eq!(
            after,
            vec![
                Launch {
                    track: 1,
                    slot: 1,
                    origin: 5 * BAR,
                    start: 5 * BAR,
                },
                Launch {
                    track: 0,
                    slot: 1,
                    origin: 5 * BAR,
                    start: 5 * BAR,
                }
            ]
        );
        let stopped = launches_after(&after, &TimeAction::StopClip(1), &t);
        assert_eq!(
            stopped,
            vec![Launch {
                track: 0,
                slot: 1,
                origin: 5 * BAR,
                start: 5 * BAR,
            }]
        );
        // On a bar line: now. Stopped: from the start of the bar.
        let on_bar = TransportInfo {
            position: 3 * BAR,
            ..t
        };
        assert_eq!(launch_origin(&on_bar), 3 * BAR);
        let stopped = TransportInfo {
            state: State::Stopped,
            position: 3 * BAR + 100,
            ..t
        };
        assert_eq!(launch_origin(&stopped), 3 * BAR);
        // Counting in from bar 0: bar 1.
        let count = TransportInfo {
            state: State::CountIn,
            position: -1000,
            ..t
        };
        assert_eq!(launch_origin(&count), 0);
        assert_eq!(
            launches_after(&st.launches, &TimeAction::Undo, &t),
            st.launches
        );
    }

    #[test]
    fn takes_list_newest_first_with_the_undone_dimmed() {
        let mut st = state();
        let row = |n: u32| TakeRow {
            number: n,
            captured: false,
            edited: false,
            made: false,
            at: Some((2, 0)),
            added: n as usize,
            erased: 0,
            deleted: false,
            new_track: None,
            dropped: None,
            song: None,
            settings: false,
            pasted: false,
            replaced: None,
            variant: None,
            scene: None,
            rows: None,
            row_tool: None,
            followed: None,
            chords: None,
            part: false,
            style: false,
        };
        st.done = vec![row(1), row(2)];
        st.undone = vec![row(4), row(3)];
        let (items, current) = take_items(&st);
        let titles: Vec<(&str, bool)> =
            items.iter().map(|i| (i.title.as_str(), i.undone)).collect();
        assert_eq!(
            titles,
            vec![
                ("Take 4 · Stab / 1", true),
                ("Take 3 · Stab / 1", true),
                ("Take 2 · Stab / 1", false),
                ("Take 1 · Stab / 1", false)
            ]
        );
        assert_eq!(current, Some(2));
        assert_eq!(items[2].detail, "+2");
        // ERASE + a pad while playing: live erase's own take, counted
        // from 1 ("erase 1"), not by its top-bit number.
        let erase = row(mpfree_play::ERASE_TAKES + 1);
        assert_eq!(erase.short(), "erase 1");
        assert_eq!(take_title(&st, &erase), "Erase 1 · Stab / 1");
        st.done[1].erased = 5;
        assert_eq!(take_items(&st).0[2].detail, "+2 −5");
        // A capture that made a clip.
        st.done[1] = TakeRow {
            number: 1,
            captured: true,
            edited: false,
            made: true,
            at: Some((0, 3)),
            added: 9,
            erased: 0,
            deleted: false,
            new_track: None,
            dropped: None,
            song: None,
            settings: false,
            pasted: false,
            replaced: None,
            variant: None,
            scene: None,
            rows: None,
            row_tool: None,
            followed: None,
            chords: None,
            part: false,
            style: false,
        };
        let (items, _) = take_items(&st);
        assert_eq!(
            (items[2].title.as_str(), items[2].detail.as_str()),
            ("Capture 1 · Drums / 4", "new +9")
        );
        assert_eq!(
            (st.done[1].short(), st.done[0].short()),
            ("capture 1".to_string(), "take 1".to_string())
        );
        st.done[1] = TakeRow {
            number: 5,
            captured: false,
            edited: true,
            made: false,
            at: Some((1, 0)),
            added: 1,
            erased: 1,
            deleted: false,
            new_track: None,
            dropped: None,
            song: None,
            settings: false,
            pasted: false,
            replaced: None,
            variant: None,
            scene: None,
            rows: None,
            row_tool: None,
            followed: None,
            chords: None,
            part: false,
            style: false,
        };
        assert_eq!(take_items(&st).0[2].title, "Edit 5 · Bass / 1");
        assert_eq!(st.done[1].short(), "edit 5");
        // ERASE + a clip: a delete, named by where the clip was.
        st.done[1] = TakeRow {
            number: 6,
            deleted: true,
            new_track: None,
            dropped: None,
            song: None,
            settings: false,
            at: Some((1, 0)),
            ..st.done[1]
        };
        let (items, _) = take_items(&st);
        assert_eq!(
            (items[2].title.as_str(), items[2].detail.as_str()),
            ("Delete 6 · Bass / 1", "clip")
        );
        assert_eq!(take_title(&st, &st.done[1]), "Delete 6 · Bass / 1");
        // Song → scenes: one edit, named by the rows it filled.
        st.done[1] = TakeRow {
            number: 7,
            deleted: false,
            made: true,
            rows: Some((4, 6)),
            ..st.done[1]
        };
        let (items, _) = take_items(&st);
        assert_eq!(
            (items[2].title.as_str(), items[2].detail.as_str()),
            ("Song → scenes 7", "rows 5 to 7")
        );
        assert_eq!(st.done[1].short(), "song → scenes 7");
        st.done[1].rows = Some((2, 2));
        assert_eq!(take_items(&st).0[2].detail, "row 3");
        assert_eq!(
            (bars_label(1), bars_label(4)),
            ("1 bar".to_string(), "4 bars".to_string())
        );
        st.done.clear();
        assert_eq!(take_items(&st).1, None);
    }

    #[test]
    fn labels() {
        let m = Meter::default();
        assert_eq!(length_label(3840.0, m), "1 bar");
        assert_eq!(length_label(7680.0, m), "2 bars");
        assert_eq!(length_label(1920.0, m), "2 beats");
        assert_eq!(length_label(5760.0, m), "6 beats");
        assert_eq!(length_label(1680.0, m), "7/16");
        assert_eq!(length_label(100.0, m), "100 ticks");
        assert_eq!(loop_label(None, m), "off");
        assert_eq!(loop_label(Some((0, 3840)), m), "1");
        assert_eq!(loop_label(Some((0, 4 * 3840)), m), "1–4");
        assert_eq!(loop_label(Some((960, 3840)), m), "1.2.000–2.1.000");
    }

    #[test]
    fn update_redraws_only_what_changed() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut c = mpfree_ui::SoftCanvas::new(1280, 800);
        let mut st = state();
        page.update(&mut ui, &st);
        ui.paint(&mut c);
        page.update(&mut ui, &st);
        assert!(!ui.needs_paint(), "nothing changed");
        st.transport.position += 1;
        page.update(&mut ui, &st);
        let drawn = ui.paint(&mut c);
        assert_eq!(
            drawn,
            vec![ui.rect_of(TRANSPORT).unwrap()],
            "one tick on: only the bar"
        );
        st.scene_cued = Some(1);
        page.update(&mut ui, &st);
        assert_eq!(
            ui.paint(&mut c),
            vec![ui.rect_of(CLIPS).unwrap()],
            "a scene cued: the grid"
        );
        st.morph.pos = 0.4;
        page.update(&mut ui, &st);
        assert_eq!(
            ui.paint(&mut c),
            vec![ui.rect_of(MORPH).unwrap()],
            "the crossfader moved: the morph bar"
        );
    }

    /// Show's offers (piano roll plan 4.1): the other clips of the edited
    /// clip's kind with notes, those that play with it (the same scene,
    /// or playing now on another track) first; each faint clip placed
    /// against the pass of the edited clip playing now, from where its
    /// own loop is; a menu pick and a faint note's tap name their clips.
    #[test]
    fn other_clips_are_offered_placed_and_named_by_their_taps() {
        let mut st = state();
        let mut stab = Clip::new("Stab", 2 * BAR);
        stab.add_note(Note::new(0, 240, 5, 100)).unwrap();
        let mut ed = crate::editor::Editor::new(2, 1, &stab);
        ed.show = vec![(0, 0), (1, 0)];
        // A one-bar drum clip at half speed, and a one-bar bass line.
        let mut drums = Clip::new("A", BAR);
        drums.speed = mpfree_seq::Speed::new(1, 2).unwrap();
        drums.add_note(Note::new(0, 240, 1, 100)).unwrap();
        let mut bass = Clip::new("A", BAR);
        bass.add_note(Note::new(240, 240, 2, 100)).unwrap();
        let ghost = |track: usize, clip: Clip, name: &str| Ghost {
            track,
            slot: 0,
            clip: clip.into(),
            track_name: name.into(),
        };
        st.edit = Some(Box::new(EditInfo {
            editor: ed,
            clip: stab.into(),
            rev: 1,
            track_name: "Stab".into(),
            bar: BAR,
            answer: EditAnswer::default(),
            locks: Default::default(),
            ghosts: vec![ghost(0, drums, "Drums"), ghost(1, bass, "Bass")],
        }));
        let e = st.edit.clone().unwrap();
        let offers: Vec<(usize, usize, bool, bool)> = show_offers(&e, &st)
            .iter()
            .map(|o| (o.track, o.slot, o.with, o.shown))
            .collect();
        // Drums' A plays now; Bass' A is in another scene and stopped, and
        // Stab's own A can't play with it; the empty B clips and the
        // edited clip are left out.
        assert_eq!(
            offers,
            [
                (0, 0, true, true),
                (1, 0, false, true),
                (2, 0, false, false)
            ]
        );
        // The edited clip stopped: both from their tops.
        let at = |layers: &[GhostLayer], k: usize| -> Vec<Tick> {
            layers[k].notes.iter().map(|n| n.tick).collect()
        };
        let layers = ghost_layers(&e, &st);
        assert_eq!(layers[0].name, "Drums · A");
        assert_eq!((layers[0].with, layers[1].with), (true, false));
        assert_eq!(at(&layers, 0), [0], "half speed: once in two bars");
        assert_eq!(at(&layers, 1), [240, BAR + 240]);
        // Launched on bar 2: its pass playing now (bar 5 and a beat) began
        // on bar 4; the drums (from bar 1, two bars a pass) are a bar in
        // there, the bass as if launched with it.
        st.launches.push(Launch {
            track: 2,
            slot: 1,
            origin: BAR,
            start: BAR,
        });
        let layers = ghost_layers(&e, &st);
        assert_eq!(at(&layers, 0), [BAR]);
        assert_eq!(at(&layers, 1), [240, BAR + 240]);
        assert_eq!(layers[0].phase, BAR as f64);
        // Show's second offer, and the second faint clip's tap.
        let edit = |k| action(&ev(NOTES, EventKind::Tab(k)), &st);
        assert_eq!(
            edit(SHOW_MENU + 1),
            Some(TimeAction::Edit(EditAction::Show { track: 1, slot: 0 }))
        );
        assert_eq!(
            edit(GHOST_TAP + 1),
            Some(TimeAction::Edit(EditAction::Switch { track: 1, slot: 0 }))
        );
        assert_eq!(
            edit(SHOW_NONE),
            Some(TimeAction::Edit(EditAction::ShowNone))
        );
        assert_eq!(edit(GHOST_TAP + 2), None);
    }
}
