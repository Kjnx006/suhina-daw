// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The shared-memory files of a process that crashed or was killed are removed by the next
//! one, and those of a live Suhina process never are (#805).
#![allow(clippy::panic)] // test helpers outside #[test] functions

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::{Mutex, MutexGuard};

use suhina_ipc::{Link, sweep_leftovers};

/// One test at a time: the sweep a test's peer runs when it starts would remove another test's
/// leftovers before that test looks at them, or hold their lock while it does, so that the
/// test's own sweep takes them for a live process's.
fn one_at_a_time() -> MutexGuard<'static, ()> {
    static TURN: Mutex<()> = Mutex::new(());
    TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A `suhina-ipc-peer` in `mode` (`make` or `idle`), and the lines it printed.
fn peer(mode: &str, lines: usize) -> (Child, Vec<String>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_suhina-ipc-peer"))
        .arg(mode)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("starting the peer: {e}"));
    let stdout: ChildStdout = child.stdout.take().unwrap_or_else(|| panic!("no stdout"));
    let printed: Vec<String> = BufReader::new(stdout).lines().take(lines).map_while(Result::ok).collect();
    assert_eq!(printed.len(), lines, "the peer printed {printed:?}");
    (child, printed)
}

/// The folder the shared-memory files go in.
fn folder() -> PathBuf {
    let link = Link::create(1, 16).unwrap_or_else(|e| panic!("{e}"));
    Path::new(&link.address()).parent().unwrap_or_else(|| panic!("no folder")).to_path_buf()
}

fn lock_of(child: &Child) -> PathBuf {
    folder().join(format!("suhina-ipc-{}.lock", child.id()))
}

#[test]
fn a_killed_processs_files_go() {
    let _turn = one_at_a_time();
    let (mut child, made) = peer("make", 2);
    let lock = lock_of(&child);
    assert!(lock.exists());
    let _ = child.kill();
    let _ = child.wait();
    // Killed, so nothing removed them.
    assert!(made.iter().all(|f| Path::new(f).exists()), "{made:?}");
    sweep_leftovers();
    assert!(made.iter().all(|f| !Path::new(f).exists()), "{made:?}");
    assert!(!lock.exists());
}

#[test]
fn a_live_processs_files_stay_until_it_ends() {
    let _turn = one_at_a_time();
    let (mut child, made) = peer("make", 2);
    sweep_leftovers();
    // As a second instance starting would.
    let (mut other, others) = peer("make", 2);
    assert!(made.iter().all(|f| Path::new(f).exists()), "{made:?}");
    // It ends normally and removes its own files.
    drop(child.stdin.take());
    let _ = child.wait();
    assert!(made.iter().all(|f| !Path::new(f).exists()), "{made:?}");
    assert!(others.iter().all(|f| Path::new(f).exists()), "{others:?}");
    drop(other.stdin.take());
    let _ = other.wait();
    sweep_leftovers();
    assert!(!lock_of(&child).exists());
    assert!(!lock_of(&other).exists());
}

#[test]
fn a_reused_pid_does_not_keep_a_gone_owners_files() {
    let _turn = one_at_a_time();
    // A live process that is not a Suhina one: it holds no lock, so files named after its pid
    // are an earlier, gone owner's.
    let (mut idle, printed) = peer("idle", 1);
    assert_eq!(printed, [idle.id().to_string()]);
    let dir = folder();
    let left = [
        dir.join(format!("suhina-ipc-{}-0-1", idle.id())),
        dir.join(format!("suhina-ara-{}-0-1", idle.id())),
        dir.join(format!("suhina-ipc-{}.lock", idle.id())),
    ];
    for file in &left {
        std::fs::write(file, [0u8; 64]).unwrap_or_else(|e| panic!("{e}"));
    }
    sweep_leftovers();
    assert!(left.iter().all(|f| !f.exists()), "{left:?}");
    drop(idle.stdin.take());
    let _ = idle.wait();
}
