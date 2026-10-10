//! Tests against the local, uncommitted Live Set corpus (`corpus/als`, see
//! `docs/spec/als-corpus.md`). Run with `cargo test -p loopwerk-als --features corpus`.
#![cfg(feature = "corpus")]
#![allow(clippy::panic)] // test helpers outside #[test] functions

use std::path::PathBuf;

use loopwerk_als::{Import, read_file};
use loopwerk_model::LaneData;

fn open(name: &str) -> Import {
    let path: PathBuf =
        [env!("CARGO_MANIFEST_DIR"), "..", "..", "corpus", "als", "00-base Project", &format!("{name}.als")]
            .iter()
            .collect();
    assert!(path.exists(), "{} is missing; author the corpus first (docs/spec/als-corpus.md)", path.display());
    read_file(&path).unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn every_series_1_file_imports() {
    for name in [
        "00-base",
        "00b-base-resaved",
        "01-tempo-128",
        "02-midi-track",
        "03-midi-clip-empty",
        "04-midi-clip-one-note",
        "05-note-moved",
        "06-note-velocity",
        "07-note-key",
        "08-clip-length-2-bars",
        "09-clip-loop-off",
        "10-clip-name-color",
        "11-insert-scene",
        "12-session-clip",
        "13-second-session-clip",
        "14-arrangement-moved",
        "15-close-piano-roll",
        "16-extend-mixer",
        "17-track-volume-pan",
        "18-track-mute",
        "19-return-send",
        "20-clip-length-2-bars",
    ] {
        open(name);
    }
}

#[test]
fn tempo() {
    assert_eq!(open("00b-base-resaved").project.transport.tempo_bpm, 120.0);
    assert_eq!(open("01-tempo-128").project.transport.tempo_bpm, 128.0);
}

#[test]
fn arrangement_clip_moved_to_bar_3() {
    let p = open("14-arrangement-moved").project;
    let lane = p.arrangement.lanes.iter().find(|l| l.name == "2-MIDI").unwrap();
    assert_eq!(lane.clips.len(), 1);
    let clip = &lane.clips[0];
    assert_eq!((clip.start, clip.length), (8.0, 4.0));
    let LaneData::Notes(notes) = &p.content(clip.content).unwrap().lanes[0].data else { panic!() };
    assert_eq!((notes[0].key, notes[0].time, notes[0].duration), (64, 1.0, 2.0));
    assert!((notes[0].velocity * 127.0 - 64.2513).abs() < 1e-3);
}

#[test]
fn session_clips_and_inserted_scene() {
    let p = open("13-second-session-clip").project;
    assert_eq!(p.launcher.scenes.len(), 9);
    let column = &p.launcher.columns[0];
    let keys: Vec<u8> = column.slots[..2]
        .iter()
        .map(|s| {
            let clip = s.clip.as_ref().unwrap();
            let LaneData::Notes(notes) = &p.content(clip.content).unwrap().lanes[0].data else { panic!() };
            notes[0].key
        })
        .collect();
    assert_eq!(keys, [60, 67]);
}

#[test]
fn mixer_volume_pan_mute_send() {
    let audio = |name: &str| {
        let p = open(name).project;
        p.channels.into_iter().find(|c| c.name == "3-Audio").unwrap()
    };
    let c = audio("17-track-volume-pan");
    assert!((c.volume - 0.501_187).abs() < 1e-6);
    assert_eq!(c.pan, -1.0);
    assert!(!c.mute);
    assert!(audio("18-track-mute").mute);
    let c = audio("19-return-send");
    assert!(!c.mute);
    assert_eq!(c.sends.len(), 1);
    assert!((c.sends[0].level - 0.251_189).abs() < 1e-6);
}

#[test]
fn session_clip_length_2_bars() {
    let p = open("20-clip-length-2-bars").project;
    let clip = p.launcher.columns[0].slots[0].clip.as_ref().unwrap();
    assert_eq!(clip.loop_range.map(|r| r.end), Some(8.0));
    assert_eq!(clip.length, 8.0);
}
