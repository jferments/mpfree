//! Keygroup programs made into projects, on made-up program folders (no
//! MPC content): the program read, the samples found, the kit's pads,
//! keys and layers, and the project the app opens.

use std::fs;
use std::path::{Path, PathBuf};

use mpfree_library::{Library, Loc, Place};
use mpfree_mpc_import::import::{import_keygroup, SAMPLER_PART};
use mpfree_mpc_import::kit::Options;
use mpfree_mpc_import::xpm::{program_type, read_keygroup, root_offset};
use mpfree_sampler::model::{PadsPlay, PlayMode, Project, Source};
use mpfree_sampler::{save, wav};

/// A fresh folder under the target directory.
fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mpfree-mpc-import-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

/// A short mono WAV whose level tells it apart.
fn write_wav(path: &Path, frames: usize, level: f32) {
    let s = Source::new("t", 44_100, 1, vec![level; frames]).unwrap();
    let mut f = fs::File::create(path).unwrap();
    wav::write(&s, &mut f).unwrap();
}

/// (vel_lo, vel_hi, sample, RootNote, volume, fine)
type L<'a> = (u8, u8, &'a str, i32, f32, i32);

fn xpm(kind: &str, name: &str, zones: &[(u8, u8, Vec<L>)]) -> String {
    let mut inst = String::new();
    for (n, (lo, hi, layers)) in zones.iter().enumerate() {
        let mut ls = String::new();
        for (i, (vl, vh, s, r, v, fine)) in layers.iter().enumerate() {
            ls += &format!(
                "<Layer number=\"{}\"><Active>True</Active><Volume>{v}</Volume><Pan>0.500000</Pan>\
                 <Pitch>0.000000</Pitch><TuneCoarse>0</TuneCoarse><TuneFine>{fine}</TuneFine>\
                 <VelStart>{vl}</VelStart><VelEnd>{vh}</VelEnd><SampleStart>0</SampleStart>\
                 <SampleEnd>0</SampleEnd><Loop>False</Loop><SampleName>{s}</SampleName>\
                 <SampleFile></SampleFile><RootNote>{r}</RootNote><KeyTrack>True</KeyTrack></Layer>",
                i + 1
            );
        }
        inst += &format!(
            "<Instrument number=\"{}\"><LowNote>{lo}</LowNote><HighNote>{hi}</HighNote>\
             <VolumeRelease>0.200000</VolumeRelease><Layers>{ls}</Layers></Instrument>",
            n + 1
        );
    }
    // An unused keygroup slot, as programs carry.
    inst += "<Instrument number=\"99\"><Layers><Layer number=\"1\"><Active>True</Active>\
             <SampleName></SampleName></Layer></Layers></Instrument>";
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MPCVObject><Version><File_Version>2.1</File_Version>\
         </Version><Program type=\"{kind}\"><ProgramName>{name}</ProgramName>\
         <KeygroupNumKeygroups>{}</KeygroupNumKeygroups><Instruments>{inst}</Instruments></Program></MPCVObject>\n",
        zones.len()
    )
}

/// A grand: two keygroups, two velocity layers each, roots written as
/// the MIDI note + 1 (C3 is 48 here, so RootNote 49).
fn grand() -> String {
    xpm(
        "Keygroup",
        "Test Grand",
        &[
            (
                0,
                54,
                vec![
                    (0, 80, "Grand C3 soft", 49, 0.5, 0),
                    (81, 127, "Grand C3 hard", 49, 0.25, 0),
                ],
            ),
            (
                55,
                127,
                vec![
                    (0, 80, "Grand A4 soft", 70, 0.5, 0),
                    (81, 127, "Grand A4 hard", 70, 0.5, 7),
                ],
            ),
        ],
    )
}

fn grand_folder(dir: &Path) -> PathBuf {
    let prog = dir.join("Test Grand");
    fs::create_dir_all(&prog).unwrap();
    fs::write(prog.join("Test Grand.xpm"), grand()).unwrap();
    write_wav(&prog.join("Grand C3 soft.wav"), 1000, 0.1);
    write_wav(&prog.join("Grand C3 hard.WAV"), 1200, 0.2);
    write_wav(&prog.join("Grand A4 soft.wav"), 800, 0.3);
    write_wav(&prog.join("grand a4 hard.wav"), 900, 0.4);
    prog.join("Test Grand.xpm")
}

/// The project as the app opens it: the sampler part's kits, with sounds read from `samples/`.
fn open(data: &Path, name: &str) -> (Vec<(String, Project)>, usize, Vec<String>) {
    let lib = Library::at(data).unwrap();
    let loc = Loc::new(Place::Projects, &[name]).unwrap();
    let snap = lib.open_project(&loc).unwrap();
    assert_eq!(snap.meta.name, name);
    assert_eq!(snap.parts.len(), 1);
    let text = snap.part(SAMPLER_PART).expect("a sampler part");
    let opened = save::from_text(text, &mut |f| {
        let b = fs::read(lib.sample_path(&loc, f).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        wav::read(f, &b)
    })
    .unwrap();
    (opened.kits, opened.kit, opened.notes)
}

#[test]
fn a_keygroup_program_is_read() {
    let p = read_keygroup(grand().as_bytes()).unwrap();
    assert_eq!(p.name, "Test Grand");
    assert_eq!(p.declared, Some(2));
    assert_eq!(p.keygroups.len(), 2, "the empty slot is left out");
    let k = &p.keygroups[1];
    assert_eq!((k.lo, k.hi), (55, 127));
    assert_eq!(k.layers.len(), 2);
    assert_eq!(
        (k.layers[1].vel_lo, k.layers[1].vel_hi, k.layers[1].fine),
        (81, 127, 7)
    );
    assert_eq!(k.layers[1].sample, "Grand A4 hard");
    assert_eq!(
        k.raw,
        vec![("VolumeRelease".to_string(), "0.200000".to_string())]
    );
    let (off, why) = root_offset(&p);
    assert_eq!(off, 1, "{why}");
    assert!(why.contains("4 of 4"), "{why}");
    assert_eq!(program_type(grand().as_bytes()).unwrap(), "Keygroup");
}

#[test]
fn roots_written_as_the_note_read_as_the_note() {
    let x = xpm(
        "Keygroup",
        "S",
        &[(0, 127, vec![(0, 127, "Str E3", 52, 1.0, 0)])],
    );
    assert_eq!(root_offset(&read_keygroup(x.as_bytes()).unwrap()).0, 0);
    let unnamed = xpm(
        "Keygroup",
        "S",
        &[(0, 127, vec![(0, 127, "Strings", 52, 1.0, 0)])],
    );
    let (off, why) = root_offset(&read_keygroup(unnamed.as_bytes()).unwrap());
    assert_eq!(off, 0);
    assert!(why.contains("no sample name"), "{why}");
}

#[test]
fn a_drum_program_is_not_a_keygroup() {
    let x = xpm(
        "Drum",
        "Kit",
        &[(0, 127, vec![(0, 127, "Kick", 60, 1.0, 0)])],
    );
    let e = read_keygroup(x.as_bytes()).unwrap_err();
    assert!(e.contains("\"Drum\""), "{e}");
}

#[test]
fn a_program_folder_becomes_a_project_with_a_keygroup_kit() {
    let dir = temp("grand");
    let xpm = grand_folder(&dir);
    let data = dir.join("data");
    fs::create_dir_all(&data).unwrap();
    let r = import_keygroup(&xpm, &data, None, &Options { release_ms: 300.0 }, false).unwrap();
    let text = r.lines.join("\n");
    assert!(
        r.project.ends_with("library/Projects/Test Grand"),
        "{}",
        r.project.display()
    );
    assert!(text.contains("kit: 2 pads, 4 layers, 4 sounds"), "{text}");
    assert!(
        text.contains("RootNote reads as the MIDI note + 1"),
        "{text}"
    );

    let (kits, kit, notes) = open(&data, "Test Grand");
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!((kits.len(), kit), (1, 0));
    let (name, p) = &kits[0];
    assert_eq!(name, "Test Grand");
    assert_eq!(p.pads_play(), PadsPlay::Keys { from: 48 });
    let low = p.pad(0).unwrap();
    let k = low.keys.unwrap();
    assert_eq!((k.lo, k.hi, k.root), (0, 54, 48));
    assert_eq!(low.name, "Test Grand C3");
    assert_eq!(low.amp.release_ms, 300.0);
    assert_eq!(low.layers.len(), 2);
    assert_eq!((low.layers[0].vel_lo, low.layers[0].vel_hi), (0, 80));
    assert_eq!((low.layers[1].vel_lo, low.layers[1].vel_hi), (81, 127));
    assert_eq!(
        low.layers[0].level_db, 0.0,
        "the most common volume is 0 dB"
    );
    assert!(
        (low.layers[1].level_db + 6.0206).abs() < 1e-3,
        "{}",
        low.layers[1].level_db
    );
    assert_eq!(low.layers[0].region.mode, PlayMode::Gate);
    let src = p.source(low.layers[1].region.source).unwrap();
    assert_eq!(src.frames(), 1200);
    assert!(
        (src.data()[0] - 0.2).abs() < 1e-3,
        "the hard C3 sample, found as .WAV"
    );
    let high = p.pad(1).unwrap();
    assert_eq!(high.keys.unwrap().root, 69);
    assert_eq!(high.layers[1].tune_cents, 7);
    assert!(p.pad(2).is_none());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn layers_with_another_root_are_tuned_to_the_pads() {
    let dir = temp("roots");
    let x = xpm(
        "Keygroup",
        "Mixed",
        &[(
            40,
            70,
            vec![
                (1, 64, "Pno C4", 60, 1.0, 0),
                (65, 127, "Pno D4", 62, 1.0, -10),
            ],
        )],
    );
    fs::write(dir.join("Mixed.xpm"), x).unwrap();
    write_wav(&dir.join("Pno C4.wav"), 500, 0.1);
    write_wav(&dir.join("Pno D4.wav"), 500, 0.1);
    let data = dir.join("data");
    fs::create_dir_all(&data).unwrap();
    import_keygroup(
        &dir.join("Mixed.xpm"),
        &data,
        Some("My: piano"),
        &Options::default(),
        false,
    )
    .unwrap();
    let (kits, _, _) = open(&data, "My- piano");
    let pad = kits[0].1.pad(0).unwrap();
    assert_eq!(pad.keys.unwrap().root, 60);
    assert_eq!(
        pad.layers[1].tune_cents,
        -200 - 10,
        "D4 played two semitones down, plus its own fine tune"
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_missing_sample_is_reported_and_the_rest_imported() {
    let dir = temp("missing");
    let xpm = grand_folder(&dir);
    fs::remove_file(xpm.parent().unwrap().join("Grand A4 soft.wav")).unwrap();
    let data = dir.join("data");
    fs::create_dir_all(&data).unwrap();
    let r = import_keygroup(&xpm, &data, None, &Options::default(), false).unwrap();
    let text = r.lines.join("\n");
    assert!(
        text.contains("Grand A4 soft: its file is not beside the program; left out"),
        "{text}"
    );
    assert!(text.contains("kit: 2 pads, 3 layers"), "{text}");
    let (kits, _, _) = open(&data, "Test Grand");
    assert_eq!(kits[0].1.pad(1).unwrap().layers.len(), 1);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_shared_sample_is_one_sound() {
    let dir = temp("shared");
    let x = xpm(
        "Keygroup",
        "Shared",
        &[
            (0, 59, vec![(0, 127, "Tone C4", 60, 1.0, 0)]),
            (60, 127, vec![(0, 127, "Tone C4", 60, 1.0, 0)]),
        ],
    );
    fs::write(dir.join("Shared.xpm"), x).unwrap();
    write_wav(&dir.join("Tone C4.wav"), 300, 0.5);
    let data = dir.join("data");
    fs::create_dir_all(&data).unwrap();
    let r = import_keygroup(
        &dir.join("Shared.xpm"),
        &data,
        None,
        &Options::default(),
        false,
    )
    .unwrap();
    assert!(
        r.lines.join("\n").contains("2 pads, 2 layers, 1 sounds"),
        "{:?}",
        r.lines
    );
    let (kits, _, _) = open(&data, "Shared");
    assert_eq!(kits[0].1.sources().len(), 1);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_dry_run_writes_nothing_and_a_project_is_never_replaced() {
    let dir = temp("dry");
    let xpm = grand_folder(&dir);
    let data = dir.join("data");
    fs::create_dir_all(&data).unwrap();
    let r = import_keygroup(&xpm, &data, None, &Options::default(), true).unwrap();
    assert!(
        r.lines
            .last()
            .unwrap()
            .starts_with("dry run: nothing written"),
        "{:?}",
        r.lines
    );
    assert!(
        fs::read_dir(&data).unwrap().next().is_none(),
        "the data folder is untouched"
    );

    import_keygroup(&xpm, &data, None, &Options::default(), false).unwrap();
    let before = fs::read(r.project.join("project.txt")).unwrap();
    let e = import_keygroup(&xpm, &data, None, &Options::default(), false).unwrap_err();
    assert!(e.contains("already there"), "{e}");
    assert_eq!(fs::read(r.project.join("project.txt")).unwrap(), before);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_tool_runs() {
    let dir = temp("cli");
    let xpm = grand_folder(&dir);
    let data = dir.join("data");
    fs::create_dir_all(&data).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_mpfree-mpc-import"))
        .args([
            "keygroup",
            xpm.to_str().unwrap(),
            "--data",
            data.to_str().unwrap(),
            "--release-ms",
            "250",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("project: "), "{text}");
    assert!(
        text.contains("  1   C-1 to F#3     C3  0-80, 81-127"),
        "{text}"
    );
    let bad = std::process::Command::new(env!("CARGO_BIN_EXE_mpfree-mpc-import"))
        .args(["keygroup", xpm.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("--data DIR is needed"));
    fs::remove_dir_all(&dir).unwrap();
}

/// A WAV with a header and an empty data chunk, as some factory sample files are.
fn write_empty_wav(path: &Path) {
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&36u32.to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&44_100u32.to_le_bytes());
    b.extend_from_slice(&88_200u32.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&0u32.to_le_bytes());
    fs::write(path, b).unwrap();
}

/// A program laid out as the MPC writes one (made-up names, no MPC
/// content): velocity sensitivity, volume, tuning, envelope and filter on
/// each `<Instrument>`, effects on the program, roots as the MIDI note + 1
/// with names that count middle C as C3.
fn factory_style() -> String {
    let layer = |n: u32, lo: u8, hi: u8, name: &str| {
        format!(
            "<Layer number=\"{n}\"><Active>True</Active><Volume>1.000000</Volume><Pan>0.500000</Pan>\
             <Pitch>0.000000</Pitch><TuneCoarse>0</TuneCoarse><TuneFine>0</TuneFine>\
             <VelStart>{lo}</VelStart><VelEnd>{hi}</VelEnd><SampleStart>0</SampleStart><SampleEnd>0</SampleEnd>\
             <Loop>False</Loop><SampleName>{name}</SampleName><SampleFile></SampleFile>\
             <RootNote>37</RootNote><KeyTrack>True</KeyTrack></Layer>"
        )
    };
    let layers = [
        layer(1, 0, 69, "EP-036 C1 069"),
        layer(2, 70, 86, "EP-036 C1 086"),
        layer(3, 87, 116, "EP-036 C1 116"),
        layer(4, 117, 127, "EP-036 C1 128"),
    ]
    .concat();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MPCVObject><Version><File_Version>2.1</File_Version>\
         <Application>MPC-V</Application></Version><Program type=\"Keygroup\">\
         <ProgramName>Test EP</ProgramName>\
         <Insert1><Name>Test Chorus</Name></Insert1><InsertEnable1>True</InsertEnable1>\
         <Insert2><Name>Test Room</Name></Insert2><InsertEnable2>True</InsertEnable2>\
         <Insert3><Name>Test Strip</Name></Insert3><InsertEnable3>False</InsertEnable3>\
         <Volume>0.707946</Volume><Pan>0.500000</Pan>\
         <Instruments><Instrument number=\"1\"><Volume>0.707946</Volume><Pan>0.500000</Pan>\
         <TuneCoarse>0</TuneCoarse><TuneFine>5</TuneFine><LowNote>0</LowNote><HighNote>38</HighNote>\
         <ZonePlay>1</ZonePlay><FilterType>3</FilterType><Cutoff>0.480000</Cutoff>\
         <Resonance>0.000000</Resonance><VelocityToFilter>0.409449</VelocityToFilter>\
         <VolumeHold>0.000000</VolumeHold><VolumeAttack>0.000000</VolumeAttack>\
         <VolumeDecay>1.000000</VolumeDecay><VolumeSustain>0.015748</VolumeSustain>\
         <VolumeRelease>0.377953</VolumeRelease><VelocitySensitivity>0.803150</VelocitySensitivity>\
         <Layers>{layers}</Layers></Instrument></Instruments></Program></MPCVObject>\n"
    )
}

#[test]
fn a_program_as_the_mpc_writes_it_reads_its_keygroup_fields() {
    let p = read_keygroup(factory_style().as_bytes()).unwrap();
    let k = &p.keygroups[0];
    assert_eq!(p.vel_to_level, None, "not on the program");
    assert!((k.vel_to_level.unwrap() - 0.80315).abs() < 1e-5);
    assert!((k.volume - 0.707946).abs() < 1e-6);
    assert_eq!((k.coarse, k.fine), (0, 5));
    assert_eq!(
        p.inserts,
        ["Test Chorus", "Test Room"],
        "the switched-off one isn't"
    );
    assert_eq!(k.filter[0], ("FilterType".to_string(), "3".to_string()));
    let (off, why) = root_offset(&p);
    assert_eq!(off, 1, "{why}");
    assert_eq!(mpfree_mpc_import::xpm::knob_text("0.377953"), "48 of 127");
    assert_eq!(mpfree_mpc_import::xpm::knob_text("1"), "1");
    assert_eq!(mpfree_mpc_import::xpm::knob_text("True"), "True");
}

#[test]
fn empty_layers_are_left_out_and_the_others_cover_their_velocities() {
    let dir = temp("factory");
    fs::write(dir.join("Test EP.xpm"), factory_style()).unwrap();
    write_empty_wav(&dir.join("EP-036 C1 069.WAV"));
    write_wav(&dir.join("EP-036 C1 086.WAV"), 44_100, 0.2);
    write_wav(&dir.join("EP-036 C1 116.WAV"), 4_410, 0.3);
    write_empty_wav(&dir.join("EP-036 C1 128.WAV"));
    let data = dir.join("data");
    fs::create_dir_all(&data).unwrap();
    let r = import_keygroup(
        &dir.join("Test EP.xpm"),
        &data,
        None,
        &Options::default(),
        false,
    )
    .unwrap();
    let text = r.lines.join("\n");
    assert!(
        text.contains("RootNote reads as the MIDI note + 1"),
        "{text}"
    );
    assert!(text.contains("EP-036 C1 069"), "{text}");
    assert!(text.contains("kit: 1 pads, 2 layers"), "{text}");
    assert!(
        text.contains("EP-036 C1 086 now plays velocities 0-86 (was 70-86)"),
        "{text}"
    );
    assert!(
        text.contains("EP-036 C1 116 now plays velocities 87-127 (was 87-116)"),
        "{text}"
    );
    assert!(
        text.contains("EP-036 C1 116 is 0.1 s and the longest layer 1.0 s; it may be cut short"),
        "{text}"
    );
    assert!(
        text.contains("VolumeRelease 48 of 127") && text.contains("ZonePlay 1"),
        "{text}"
    );
    assert!(
        text.contains("1 of 1 keygroups have a filter (FilterType 3, Cutoff 61 of 127"),
        "{text}"
    );
    assert!(text.contains("effects (Test Chorus, Test Room)"), "{text}");
    assert!(!text.contains("no velocity sensitivity"), "{text}");

    let (kits, _, _) = open(&data, "Test EP");
    let pad = kits[0].1.pad(0).unwrap();
    assert_eq!(pad.keys.unwrap().root, 36, "RootNote 37 is C2, named C1");
    assert!((pad.vel_to_level - 0.80315).abs() < 1e-5);
    let v: Vec<_> = pad.layers.iter().map(|l| (l.vel_lo, l.vel_hi)).collect();
    assert_eq!(v, [(0, 86), (87, 127)], "no velocity falls silent");
    assert_eq!(pad.layers[0].level_db, 0.0);
    assert_eq!(pad.layers[0].tune_cents, 5, "the keygroup's fine tune");
    fs::remove_dir_all(&dir).unwrap();
}
