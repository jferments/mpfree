//! Drum programs made into projects, on made-up program folders (no MPC
//! content): the program read, the pads placed by note or as on the MPC,
//! mute groups, mute targets and simultaneous play, and the project the
//! app opens.

use std::fs;
use std::path::{Path, PathBuf};

use mpfree_library::{Library, Loc, Place};
use mpfree_mpc_import::drum::{drum_kit, mpc_pad_name, Placement};
use mpfree_mpc_import::import::{import_drum, program_kind, SAMPLER_PART};
use mpfree_mpc_import::kit::Options;
use mpfree_mpc_import::xpm::{read_drum, read_keygroup};
use mpfree_sampler::model::{LayerSelect, PadsPlay, PlayMode, Project, Source};
use mpfree_sampler::{save, wav};

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "mpfree-mpc-import-drum-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn write_wav(path: &Path, frames: usize, level: f32) {
    let s = Source::new("t", 44_100, 1, vec![level; frames]).unwrap();
    let mut f = fs::File::create(path).unwrap();
    wav::write(&s, &mut f).unwrap();
}

/// One MPC pad: its number (1 to 128), its extra instrument fields and
/// its layers as (VelStart, VelEnd, SampleName, Volume, SliceEnd).
struct P<'a> {
    pad: u32,
    fields: &'a str,
    layers: Vec<(u8, u8, &'a str, f32, u64)>,
}

fn p<'a>(pad: u32, fields: &'a str, layers: Vec<(u8, u8, &'a str, f32, u64)>) -> P<'a> {
    P {
        pad,
        fields,
        layers,
    }
}

/// A drum program; instruments numbered from `from` (1 as community
/// notes say); `map` gives (pad, note) for the PadNoteMap, or None for none.
fn xpm(name: &str, from: u32, pads: &[P], map: Option<&[(u32, u8)]>) -> String {
    let mut inst = String::new();
    for pd in pads {
        let mut ls = String::new();
        for (i, (vl, vh, s, v, end)) in pd.layers.iter().enumerate() {
            ls += &format!(
                "<Layer number=\"{}\"><Active>True</Active><Volume>{v}</Volume><Pan>0.500000</Pan>\
                 <Pitch>0.000000</Pitch><TuneCoarse>0</TuneCoarse><TuneFine>0</TuneFine>\
                 <VelStart>{vl}</VelStart><VelEnd>{vh}</VelEnd><SampleName>{s}</SampleName>\
                 <SliceStart>0</SliceStart><SliceEnd>{end}</SliceEnd><Direction>0</Direction></Layer>",
                i + 1
            );
        }
        // An empty layer slot, as programs carry.
        ls += "<Layer number=\"4\"><Active>True</Active><SampleName></SampleName></Layer>";
        inst += &format!(
            "<Instrument number=\"{}\"><Volume>0.707946</Volume><Pan>0.500000</Pan>\
             <VolumeAttack>0.000000</VolumeAttack><VolumeDecay>0.050000</VolumeDecay>{}\
             <Layers>{ls}</Layers></Instrument>",
            pd.pad - 1 + from,
            pd.fields
        );
    }
    // An unused pad, as programs carry.
    inst += &format!(
        "<Instrument number=\"{}\"><Layers><Layer number=\"1\"><Active>True</Active>\
         <SampleName></SampleName></Layer></Layers></Instrument>",
        100 + from
    );
    let map = map.map_or(String::new(), |m| {
        let notes: String = m
            .iter()
            .map(|(pad, n)| format!("<PadNote number=\"{pad}\"><Note>{n}</Note></PadNote>"))
            .collect();
        format!("<PadNoteMap>{notes}</PadNoteMap>")
    });
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MPCVObject><Version><File_Version>2.1</File_Version>\
         </Version><Program type=\"Drum\"><ProgramName>{name}</ProgramName>\
         <Instruments>{inst}</Instruments>{map}</Program></MPCVObject>\n"
    )
}

/// The MPC's default bank A start: A01 kick on 37, A02 on 36, A03 closed
/// hat on 42, A04 open hat on 82; and A05 on note 30, below the kit.
const MAP: [(u32, u8); 5] = [(1, 37), (2, 36), (3, 42), (4, 82), (5, 30)];

fn kit_pads() -> Vec<P<'static>> {
    vec![
        // A kick in two velocity layers, read to frame 799 of 1000.
        p(
            1,
            "<MuteGroup>0</MuteGroup>",
            vec![(0, 90, "Kick soft", 1.0, 799), (91, 127, "Kick hard", 1.0, 0)],
        ),
        // A snare that also plays the kick (simultaneous play).
        p(
            2,
            "<SimultTarget1>37</SimultTarget1><SimultTarget2>0</SimultTarget2>",
            vec![(0, 127, "Snare", 0.5, 0)],
        ),
        // The closed hat stops the open one and shares mute group 20.
        p(
            3,
            "<MuteGroup>20</MuteGroup><MuteTarget1>82</MuteTarget1><Mono>True</Mono>",
            vec![(0, 127, "Hat closed", 1.0, 0)],
        ),
        // The open hat plays while held and backwards.
        p(
            4,
            "<MuteGroup>20</MuteGroup><OneShot>False</OneShot><Polyphony>4</Polyphony><ZonePlay>2</ZonePlay>",
            vec![(0, 127, "Hat open", 1.0, 0)],
        ),
        p(5, "", vec![(0, 127, "Low tom", 1.0, 0)]),
    ]
}

fn kit_folder(dir: &Path, from: u32) -> PathBuf {
    let prog = dir.join("Test Kit");
    fs::create_dir_all(&prog).unwrap();
    let mut text = xpm("Test Kit", from, &kit_pads(), Some(&MAP));
    // The open hat plays backwards.
    text = text.replacen(
        "<SampleName>Hat open</SampleName><SliceStart>0</SliceStart><SliceEnd>0</SliceEnd><Direction>0</Direction>",
        "<SampleName>Hat open</SampleName><SliceStart>0</SliceStart><SliceEnd>0</SliceEnd><Direction>1</Direction>",
        1,
    );
    fs::write(prog.join("Test Kit.xpm"), text).unwrap();
    write_wav(&prog.join("Kick soft.wav"), 1000, 0.1);
    write_wav(&prog.join("Kick hard.WAV"), 1000, 0.2);
    write_wav(&prog.join("snare.wav"), 900, 0.3);
    write_wav(&prog.join("Hat closed.wav"), 500, 0.4);
    write_wav(&prog.join("Hat open.wav"), 700, 0.5);
    write_wav(&prog.join("Low tom.wav"), 600, 0.6);
    prog.join("Test Kit.xpm")
}

fn sounds_beside(
    xpm: &Path,
) -> impl FnMut(&mpfree_mpc_import::xpm::XpmLayer) -> Result<Source, String> {
    let dir = xpm.parent().unwrap().to_path_buf();
    let mut cache: Vec<(String, Source)> = Vec::new();
    move |l| {
        let want = format!("{}.wav", l.sample.to_lowercase());
        if let Some((_, s)) = cache.iter().find(|(n, _)| *n == want) {
            return Ok(s.clone());
        }
        for e in fs::read_dir(&dir).unwrap().flatten() {
            if e.file_name().to_string_lossy().to_lowercase() == want {
                let s = wav::read(&l.sample, &fs::read(e.path()).unwrap())?;
                cache.push((want, s.clone()));
                return Ok(s);
            }
        }
        Err("not beside the program".into())
    }
}

fn open(data: &Path, name: &str) -> Vec<(String, Project)> {
    let lib = Library::at(data).unwrap();
    let loc = Loc::new(Place::Projects, &[name]).unwrap();
    let snap = lib.open_project(&loc).unwrap();
    assert_eq!(snap.parts.len(), 1);
    let text = snap.part(SAMPLER_PART).expect("a sampler part");
    save::from_text(text, &mut |f| {
        let b = fs::read(lib.sample_path(&loc, f).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        wav::read(f, &b)
    })
    .unwrap()
    .kits
}

#[test]
fn a_drum_program_is_read() {
    let dir = temp("read");
    let xpm = kit_folder(&dir, 1);
    let prog = read_drum(&fs::read(&xpm).unwrap()).unwrap();
    assert_eq!(prog.name, "Test Kit");
    assert_eq!(prog.numbered_from, 1);
    assert!(prog.has_note_map);
    assert_eq!(prog.pads.len(), 5, "the empty pad is skipped");
    let kick = &prog.pads[0];
    assert_eq!((kick.pad, kick.note), (1, Some(37)));
    assert_eq!(kick.layers.len(), 2, "the empty layer slot is skipped");
    assert_eq!(
        (kick.layers[0].start, kick.layers[0].end),
        (0, 800),
        "SliceEnd is the last frame played"
    );
    assert_eq!(kick.layers[1].end, 0, "SliceEnd 0 plays to the end");
    assert_eq!(prog.pads[1].simult_targets, vec![37]);
    let hat = &prog.pads[2];
    assert_eq!(
        (hat.mute_group, hat.mute_targets.clone(), hat.mono),
        (20, vec![82], true)
    );
    let open = &prog.pads[3];
    assert_eq!(
        (open.one_shot, open.polyphony, open.zone_play),
        (Some(false), Some(4), Some(2))
    );
    assert!(open.layers[0].reverse);
    assert_eq!(program_kind(&xpm).unwrap(), "Drum");
    assert!(read_keygroup(&fs::read(&xpm).unwrap())
        .unwrap_err()
        .contains("\"Drum\" program"));
}

#[test]
fn pads_go_where_their_notes_play() {
    let dir = temp("notes");
    let xpm = kit_folder(&dir, 1);
    let prog = read_drum(&fs::read(&xpm).unwrap()).unwrap();
    let kit = drum_kit(
        &prog,
        &mut sounds_beside(&xpm),
        &Options::default(),
        Placement::Notes,
    )
    .unwrap();
    let at = |i: usize| kit.project.pad(i).expect("a pad");
    // The MPC One's pad notes: 37 is pad 1, 36 pad 2, 42 pad 3, 82 pad 4,
    // so a program with the default map keeps its bank A order.
    assert_eq!(at(0).name, "Kick soft");
    assert_eq!(at(1).name, "Snare");
    assert_eq!(at(2).name, "Hat closed");
    assert_eq!(at(3).name, "Hat open");
    assert_eq!(kit.project.pads_play(), PadsPlay::Pads);
    // No pad answers the tom's note 30: it goes on the first pad past bank A.
    let tom = kit.placed.iter().find(|p| p.mpc_pad == 5).unwrap();
    assert!(!tom.answers_note);
    assert_eq!(tom.index, 16);
    assert_eq!(at(16).name, "Low tom");
    assert!(kit
        .notes
        .iter()
        .any(|n| n.contains("MPC pad A05") && n.contains("no pad here answers its note 30")));
    // Simultaneous play and mute targets follow the notes.
    assert_eq!(at(1).links, vec![0], "the snare also plays the kick");
    assert_eq!(at(2).chokes, vec![3], "the closed hat stops the open one");
    // Mute group 20 doesn't fit 16, so both hats share a free number.
    let g = at(2).mute_group.expect("a mute group");
    assert_eq!(at(3).mute_group, Some(g));
    assert!(kit
        .notes
        .iter()
        .any(|n| n.contains("mute group 20 is mute group")));
    // Mono, polyphony, play mode, reverse and layer choice.
    assert_eq!(at(2).polyphony, 1);
    assert_eq!(at(3).polyphony, 4);
    assert_eq!(at(3).select, LayerSelect::Random);
    assert_eq!(at(3).layers[0].region.mode, PlayMode::Gate);
    assert!(at(3).layers[0].region.reverse);
    assert_eq!(at(0).layers[0].region.mode, PlayMode::OneShot);
    assert_eq!(at(0).layers[0].region.end, 800);
    // Levels: every pad is at the common 0.707946, so 0 dB; the snare's
    // layer is half the common layer volume.
    assert!(at(1).level_db.abs() < 1e-4);
    assert!((at(1).layers[0].level_db + 6.0206).abs() < 1e-3);
    assert_eq!(kit.layers, 6);
}

#[test]
fn a_program_with_its_own_note_map_is_reordered_to_match_midi() {
    // The kick on A01 plays note 36 and the snare on A02 note 37: the
    // reverse of the MPC One's pads 1 and 2.
    let pads = vec![
        p(1, "", vec![(0, 127, "Kick", 1.0, 0)]),
        p(2, "", vec![(0, 127, "Snare", 1.0, 0)]),
    ];
    let text = xpm("Swapped", 1, &pads, Some(&[(1, 36), (2, 37)]));
    let prog = read_drum(text.as_bytes()).unwrap();
    let mut sound = |l: &mpfree_mpc_import::xpm::XpmLayer| {
        Source::new(&l.sample, 44_100, 1, vec![0.5; 100]).map_err(|e| format!("{e:?}"))
    };
    let notes = drum_kit(&prog, &mut sound, &Options::default(), Placement::Notes).unwrap();
    assert_eq!(
        notes.project.pad(0).unwrap().name,
        "Snare",
        "pad 1 answers note 37"
    );
    assert_eq!(
        notes.project.pad(1).unwrap().name,
        "Kick",
        "pad 2 answers note 36"
    );
    assert!(notes.placed.iter().all(|p| p.answers_note));
    let order = drum_kit(&prog, &mut sound, &Options::default(), Placement::Pads).unwrap();
    assert_eq!(order.project.pad(0).unwrap().name, "Kick");
    assert!(order.placed.iter().all(|p| !p.answers_note));
}

#[test]
fn pads_can_keep_the_mpc_order() {
    let dir = temp("pads");
    let xpm = kit_folder(&dir, 1);
    let prog = read_drum(&fs::read(&xpm).unwrap()).unwrap();
    let kit = drum_kit(
        &prog,
        &mut sounds_beside(&xpm),
        &Options::default(),
        Placement::Pads,
    )
    .unwrap();
    let names: Vec<String> = (0..5)
        .map(|i| kit.project.pad(i).unwrap().name.clone())
        .collect();
    assert_eq!(
        names,
        ["Kick soft", "Snare", "Hat closed", "Hat open", "Low tom"]
    );
    assert_eq!(kit.project.pad(2).unwrap().chokes, vec![3]);
    assert_eq!(kit.project.pad(1).unwrap().links, vec![0]);
}

#[test]
fn instruments_numbered_from_zero_and_no_note_map() {
    let pads = vec![
        p(1, "", vec![(0, 127, "Kick", 1.0, 0)]),
        p(17, "", vec![(0, 127, "Clap", 1.0, 0)]),
    ];
    let text = xpm("Zero", 0, &pads, None);
    let prog = read_drum(text.as_bytes()).unwrap();
    assert_eq!(prog.numbered_from, 0);
    assert!(!prog.has_note_map);
    assert_eq!(prog.pads[0].pad, 1);
    assert_eq!(
        prog.pads[0].note,
        Some(37),
        "bank A takes the MPC's default notes"
    );
    assert_eq!(prog.pads[1].pad, 17);
    assert_eq!(prog.pads[1].note, None, "bank B has no default");
    assert_eq!(mpc_pad_name(17), "B01");
    assert_eq!(mpc_pad_name(128), "H16");
}

#[test]
fn a_missing_layer_leaves_its_velocities_to_the_other() {
    let dir = temp("missing");
    let xpm = kit_folder(&dir, 1);
    fs::remove_file(dir.join("Test Kit").join("Kick hard.WAV")).unwrap();
    let prog = read_drum(&fs::read(&xpm).unwrap()).unwrap();
    let kit = drum_kit(
        &prog,
        &mut sounds_beside(&xpm),
        &Options::default(),
        Placement::Notes,
    )
    .unwrap();
    let kick = kit.project.pad(1).unwrap();
    assert_eq!(kick.layers.len(), 1);
    assert_eq!((kick.layers[0].vel_lo, kick.layers[0].vel_hi), (0, 127));
    assert!(kit
        .notes
        .iter()
        .any(|n| n.contains("Kick hard") && n.contains("left out")));
}

#[test]
fn a_drum_program_becomes_a_project() {
    let dir = temp("project");
    let xpm = kit_folder(&dir, 1);
    let data = dir.join("data");
    fs::create_dir_all(&data).unwrap();
    let dry = import_drum(
        &xpm,
        &data,
        None,
        &Options::default(),
        Placement::Notes,
        true,
    )
    .unwrap();
    assert!(!dry.project.exists(), "a dry run writes nothing");
    assert!(dry.lines.iter().any(|l| l.starts_with("dry run")));
    let r = import_drum(
        &xpm,
        &data,
        None,
        &Options::default(),
        Placement::Notes,
        false,
    )
    .unwrap();
    assert!(r.project.exists());
    assert!(r.lines.iter().any(|l| l.contains("A05") && l.contains('*')));
    let kits = open(&data, "Test Kit");
    assert_eq!(kits.len(), 1);
    let (name, project) = &kits[0];
    assert_eq!(name, "Test Kit");
    assert_eq!(project.pad(0).unwrap().name, "Kick soft");
    assert_eq!(project.pad(2).unwrap().chokes, vec![3]);
    assert_eq!(project.pads_play(), PadsPlay::Pads);
    let again = import_drum(
        &xpm,
        &data,
        None,
        &Options::default(),
        Placement::Notes,
        false,
    );
    assert!(again.unwrap_err().contains("already there"));
}
