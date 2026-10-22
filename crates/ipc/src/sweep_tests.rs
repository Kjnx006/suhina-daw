// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::atomic::{AtomicU32, Ordering};

use super::*;

/// A folder of its own for one test, removed when the test ends, also when it fails.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static COUNT: AtomicU32 = AtomicU32::new(0);
        let n = COUNT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("suhina-ipc-sweep-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn file(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, [0u8; 64]).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Pids no test process has: the sweep never asks the system whether they run.
const DEAD: u32 = 4_000_001;
const LIVE: u32 = 4_000_002;
const OWN: u32 = 4_000_003;

#[test]
fn only_the_names_this_crate_makes_are_read() {
    assert_eq!(parse("suhina-ipc-12-0-345"), Some(Name::Data(12)));
    assert_eq!(parse("suhina-ara-12-3-0"), Some(Name::Data(12)));
    assert_eq!(parse("suhina-ipc-12.lock"), Some(Name::Lock(12)));
    assert_eq!(parse("suhina-ipc-12.locking"), Some(Name::Staging(12)));
    for other in [
        "suhina-ipc-short-12",
        "suhina-ipc-garbage-12",
        "suhina-ipc-12-0",
        "suhina-ipc-12-0-1-2",
        "suhina-ipc-12-0-x",
        "suhina-ipc--0-1",
        "suhina-ipc-+12-0-1",
        "suhina-ipc-99999999999-0-1",
        "suhina-ara-12.lock",
        "suhina-ipc-sweep-12-0",
        "suhina-relink-12",
        "notes.txt",
    ] {
        assert_eq!(parse(other), None, "{other}");
    }
}

#[test]
fn files_of_a_gone_owner_go_and_those_of_a_live_one_stay() {
    let scratch = Scratch::new();
    let dir = &scratch.0;
    // Gone, with its lock file left behind (the system let go of the lock).
    let dead = [scratch.file(&format!("suhina-ipc-{DEAD}-0-1")), scratch.file(&format!("suhina-ara-{DEAD}-1-2"))];
    scratch.file(&format!("suhina-ipc-{DEAD}.lock"));
    // Gone, and from before there were lock files.
    let unlocked = scratch.file("suhina-ipc-4000004-0-1");
    // Alive: its lock is held.
    let live = [scratch.file(&format!("suhina-ipc-{LIVE}-0-1")), scratch.file(&format!("suhina-ara-{LIVE}-0-1"))];
    let held = hold_lock(dir, LIVE).unwrap();
    let stranger = scratch.file("suhina-ipc-short-1");

    sweep(dir, OWN, false);

    assert!(dead.iter().all(|f| !f.exists()));
    assert!(!lock_path(dir, DEAD).exists());
    assert!(!unlocked.exists());
    assert!(live.iter().all(|f| f.exists()));
    assert!(lock_path(dir, LIVE).exists());
    assert!(stranger.exists());

    // Once it ends, its files go with the next sweep.
    drop(held);
    sweep(dir, OWN, false);
    assert!(live.iter().all(|f| !f.exists()));
    assert!(!lock_path(dir, LIVE).exists());
    assert!(stranger.exists());
}

#[test]
fn this_processs_files_go_only_before_it_has_made_any() {
    let scratch = Scratch::new();
    let dir = &scratch.0;
    let own = scratch.file(&format!("suhina-ipc-{OWN}-0-1"));
    sweep(dir, OWN, false);
    assert!(own.exists());
    sweep(dir, OWN, true);
    assert!(!own.exists());
}

#[test]
fn the_lock_appears_under_its_name_already_held() {
    let scratch = Scratch::new();
    let dir = &scratch.0;
    // A stale lock and staging file of an earlier process with the same pid.
    scratch.file(&format!("suhina-ipc-{OWN}.lock"));
    scratch.file(&format!("suhina-ipc-{OWN}.locking"));
    let held = hold_lock(dir, OWN).unwrap();
    assert!(!staging_path(dir, OWN).exists());
    assert!(matches!(owner(&lock_path(dir, OWN)), Owner::Alive));
    drop(held);
    assert!(matches!(owner(&lock_path(dir, OWN)), Owner::GoneLocked(_)));
}

#[test]
fn lone_lock_and_staging_files_of_gone_owners_go() {
    let scratch = Scratch::new();
    let dir = &scratch.0;
    scratch.file(&format!("suhina-ipc-{DEAD}.lock"));
    scratch.file(&format!("suhina-ipc-{DEAD}.locking"));
    // A process between taking its staging file and renaming it.
    let starting = scratch.file(&format!("suhina-ipc-{LIVE}.locking"));
    let starting_lock = File::open(&starting).unwrap();
    starting_lock.try_lock().unwrap();
    sweep(dir, OWN, false);
    assert!(!lock_path(dir, DEAD).exists());
    assert!(!staging_path(dir, DEAD).exists());
    assert!(starting.exists());
}

#[test]
fn a_missing_folder_is_no_error() {
    let scratch = Scratch::new();
    sweep(&scratch.0.join("missing"), OWN, true);
}

#[cfg(unix)]
#[test]
fn a_lock_renamed_into_place_since_is_kept() {
    let scratch = Scratch::new();
    let path = scratch.file(&format!("suhina-ipc-{DEAD}.lock"));
    let Owner::GoneLocked(stale) = owner(&path) else { panic!("the stale lock is free") };
    // A new process with the same pid puts its own lock under the name.
    let newer = scratch.file("newer");
    std::fs::rename(&newer, &path).unwrap();
    remove_held(&stale, &path);
    assert!(path.exists());
    // Its own file is removed.
    let Owner::GoneLocked(current) = owner(&path) else { panic!("the new lock is free") };
    remove_held(&current, &path);
    assert!(!path.exists());
}
