// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Removing the shared-memory files of processes that are gone (#805).
//!
//! Regions (`suhina-ipc-<pid>-<n>-<nanos>`) and audio sources (`suhina-ara-<pid>-<n>-<nanos>`)
//! are removed by their owner's `Drop`, which a crash or a kill never runs. So each process that
//! makes one first holds an exclusive lock on `suhina-ipc-<pid>.lock` in the same folder for as
//! long as it lives, and the operating system lets go of it when the process ends, however it
//! ends. The first file a process makes sweeps the folder: a pid whose lock file is missing, or
//! whose lock it gets, belongs to a process that is gone, so its files go; a lock it cannot get
//! belongs to a live Suhina, whose files stay. An unrelated process that reused the pid holds no
//! such lock, so it does not keep a dead owner's files.
//!
//! The lock is taken on `suhina-ipc-<pid>.locking` and renamed into place, so the lock file
//! only ever appears under its own name already locked, and a sweep at the same moment cannot
//! mistake a starting process for a gone one.
//!
//! Nothing here returns an error: a file that cannot be removed is left for the next start, and
//! a process that cannot take its lock still makes its files, and tries for the lock again with
//! the next one. Until it has the lock, another process's sweep may remove its files. Never reachable from the audio
//! callback: it runs where links and sources are made.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use crate::region::directory;

/// Tries at taking this process's lock before making files without it.
const ATTEMPTS: u32 = 20;
/// Most leftover files one sweep looks at, so a folder full of them costs a bounded start.
const MOST_FILES: usize = 100_000;

/// This process's lock, and whether the folder has been swept.
struct Own {
    lock: Option<File>,
    swept: bool,
}

/// Taken before every region or source is made, so the first sweep runs before any of them.
static OWN: Mutex<Own> = Mutex::new(Own { lock: None, swept: false });

/// Takes this process's lock and sweeps the folder, the first time a region or a source is made.
/// After that it only checks the lock is held, and tries again if taking it failed: without it,
/// another process's sweep would take this one's files for a gone process's.
pub(crate) fn before_making_files() {
    let mut own = OWN.lock().unwrap_or_else(PoisonError::into_inner);
    let (dir, pid) = (directory(), std::process::id());
    if own.lock.is_none() {
        own.lock = hold_lock(&dir, pid);
    }
    if !own.swept {
        // Nothing of this process exists yet, so files named after its pid are from an earlier
        // process that had the same pid and is gone.
        sweep(&dir, pid, true);
        own.swept = true;
    }
}

/// Removes the shared-memory files of Suhina processes that are gone, and never those of a live
/// one. Making the first link or audio source in a process does this already; this runs it again.
pub fn sweep_leftovers() {
    before_making_files();
    sweep(&directory(), std::process::id(), false);
}

/// What a file name in the folder is.
#[derive(Debug, PartialEq, Eq)]
enum Name {
    /// A region or an audio source of the process `pid`.
    Data(u32),
    /// The lock of the process `pid`.
    Lock(u32),
    /// The lock of the process `pid`, before it is renamed into place.
    Staging(u32),
}

/// Only names this crate makes; anything else in the folder is never touched.
fn parse(name: &str) -> Option<Name> {
    if let Some(rest) = name.strip_prefix("suhina-ipc-") {
        if let Some(p) = rest.strip_suffix(".lock") {
            return pid(p).map(Name::Lock);
        }
        if let Some(p) = rest.strip_suffix(".locking") {
            return pid(p).map(Name::Staging);
        }
        return data(rest);
    }
    name.strip_prefix("suhina-ara-").and_then(data)
}

/// `<pid>-<n>-<nanos>`, all digits.
fn data(rest: &str) -> Option<Name> {
    let mut parts = rest.split('-');
    let (p, n, nanos) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || !digits(n) || !digits(nanos) {
        return None;
    }
    pid(p).map(Name::Data)
}

fn digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn pid(s: &str) -> Option<u32> {
    if digits(s) { s.parse().ok() } else { None }
}

fn lock_path(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("suhina-ipc-{pid}.lock"))
}

fn staging_path(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("suhina-ipc-{pid}.locking"))
}

/// Locks `pid`'s staging file and renames it to its lock file; `None` when that keeps failing.
fn hold_lock(dir: &Path, pid: u32) -> Option<File> {
    let (staging, lock) = (staging_path(dir, pid), lock_path(dir, pid));
    for _ in 0..ATTEMPTS {
        // A sweep that took a stale staging file of the same name removes it, and then the
        // rename fails: try again with a new one.
        if let Ok(file) = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&staging)
            && file.try_lock().is_ok()
            && std::fs::rename(&staging, &lock).is_ok()
        {
            return Some(file);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    None
}

/// Whether the owner of a lock file is gone.
enum Owner {
    /// Its lock file is missing.
    Gone,
    /// Its lock was free, and is held now: the lock file goes with its files.
    GoneLocked(File),
    /// A live process holds the lock (or another sweep, removing the files itself), or it could
    /// not be told.
    Alive,
}

fn owner(path: &Path) -> Owner {
    match OpenOptions::new().read(true).write(true).open(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Owner::Gone,
        Err(_) => Owner::Alive,
        Ok(file) => match file.try_lock() {
            Ok(()) => Owner::GoneLocked(file),
            Err(TryLockError::WouldBlock | TryLockError::Error(_)) => Owner::Alive,
        },
    }
}

/// Removes the files in `dir` of every process that is gone. Files named after `own` (this
/// process) are removed only with `own_leftovers`, before this process has made any.
fn sweep(dir: &Path, own: u32, own_leftovers: bool) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut data: BTreeMap<u32, Vec<PathBuf>> = BTreeMap::new();
    let mut locks = BTreeSet::new();
    let mut stagings = BTreeSet::new();
    let mut seen = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(parsed) = name.to_str().and_then(parse) else { continue };
        seen += 1;
        if seen > MOST_FILES {
            break;
        }
        match parsed {
            Name::Data(pid) => data.entry(pid).or_default().push(entry.path()),
            Name::Lock(pid) => {
                locks.insert(pid);
            }
            Name::Staging(pid) => {
                stagings.insert(pid);
            }
        }
    }
    for (pid, files) in &data {
        if *pid == own {
            if own_leftovers {
                remove(files);
            }
            continue;
        }
        match owner(&lock_path(dir, *pid)) {
            Owner::Gone => remove(files),
            Owner::GoneLocked(lock) => {
                remove(files);
                // Removed while it is held, so no other sweep takes it in between.
                remove_held(&lock, &lock_path(dir, *pid));
            }
            Owner::Alive => {}
        }
    }
    // Lock files of processes that are gone and left no other file.
    for pid in locks.iter().filter(|pid| **pid != own && !data.contains_key(pid)) {
        if let Owner::GoneLocked(lock) = owner(&lock_path(dir, *pid)) {
            remove_held(&lock, &lock_path(dir, *pid));
        }
    }
    // A staging file is held only while its process is starting; one that is free is left over.
    for pid in stagings.iter().filter(|pid| **pid != own) {
        if let Owner::GoneLocked(lock) = owner(&staging_path(dir, *pid)) {
            remove_held(&lock, &staging_path(dir, *pid));
        }
    }
}

/// Removes the lock file at `path` that `held` is locked on, unless a process with the same pid
/// has renamed its own lock to that name since. Checked on Unix (device and inode); between the
/// check and the removal the window stays open, a limit ADR 0006 names.
fn remove_held(held: &File, path: &Path) {
    if same_file(held, path) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(unix)]
fn same_file(held: &File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (held.metadata(), std::fs::symlink_metadata(path)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// Elsewhere the file's identity is not checked: the path is removed.
#[cfg(not(unix))]
fn same_file(_held: &File, _path: &Path) -> bool {
    true
}

fn remove(files: &[PathBuf]) {
    for file in files {
        // Still mapped by a host on Windows: the name goes at once, the memory when it closes.
        let _ = std::fs::remove_file(file);
    }
}

#[cfg(test)]
#[path = "sweep_tests.rs"]
mod tests;
