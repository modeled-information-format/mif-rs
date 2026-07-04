//! Exclusive review lock, with stale-PID detection.
//!
//! Backed by `std::fs::File`'s native advisory file lock (`try_lock`/
//! `unlock`, stable since Rust 1.89 — no external locking crate needed)
//! as the actual concurrency guard, plus a PID recorded in the lock file's
//! contents, so a lock left behind by a killed process is detected as
//! stale and cleared automatically, rather than requiring manual
//! intervention. This is the direct fix for the real incident this
//! crate's design responds to: two `ontology-review.sh` bash invocations
//! run in parallel by accident, with no lock at all, corrupted two
//! topics' `ontology-map.json` files mid-write.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sysinfo::{Pid, ProcessesToUpdate, System};

use crate::error::MifRhError;

/// An exclusive lock held for the duration of one `review` run. Releases
/// automatically on drop.
#[derive(Debug)]
pub struct ReviewLock {
    file: File,
    path: PathBuf,
}

impl ReviewLock {
    /// Acquires the exclusive review lock at `path`, clearing it first if
    /// the PID recorded inside it no longer names a running process.
    ///
    /// # Errors
    ///
    /// Returns [`MifRhError::LockHeld`] if another live process currently
    /// holds the lock, or [`MifRhError::LockIo`] if the lock file cannot be
    /// opened or written.
    pub fn acquire(path: &Path) -> Result<Self, MifRhError> {
        if let Some(holder_pid) = read_holder_pid(path)
            && process_is_alive(holder_pid)
        {
            return Err(MifRhError::LockHeld { holder_pid });
        }

        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|source| MifRhError::LockIo {
                path: path.display().to_string(),
                source,
            })?;

        match file.try_lock() {
            Ok(()) => {},
            Err(TryLockError::WouldBlock) => {
                return Err(MifRhError::LockHeld {
                    holder_pid: read_holder_pid(path).unwrap_or(0),
                });
            },
            Err(TryLockError::Error(source)) => {
                return Err(MifRhError::LockIo {
                    path: path.display().to_string(),
                    source,
                });
            },
        }

        file.set_len(0).map_err(|source| MifRhError::LockIo {
            path: path.display().to_string(),
            source,
        })?;
        file.write_all(std::process::id().to_string().as_bytes())
            .map_err(|source| MifRhError::LockIo {
                path: path.display().to_string(),
                source,
            })?;

        Ok(Self {
            file,
            path: path.to_path_buf(),
        })
    }
}

impl Drop for ReviewLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
        let _ = fs::remove_file(&self.path);
    }
}

fn read_holder_pid(path: &Path) -> Option<u32> {
    let mut contents = String::new();
    File::open(path).ok()?.read_to_string(&mut contents).ok()?;
    contents.trim().parse().ok()
}

fn process_is_alive(pid: u32) -> bool {
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
    system.process(Pid::from_u32(pid)).is_some()
}

#[cfg(test)]
mod tests {
    use super::ReviewLock;

    #[test]
    fn acquire_creates_and_releases_the_lock_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".review.lock");
        assert!(!path.exists());

        {
            let _lock = ReviewLock::acquire(&path).unwrap();
            assert!(path.exists());
        }
        assert!(!path.exists(), "lock file should be removed on drop");
    }

    #[test]
    fn a_second_acquire_while_the_first_is_held_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".review.lock");

        let _first = ReviewLock::acquire(&path).unwrap();
        let error = ReviewLock::acquire(&path).unwrap_err();
        assert!(matches!(error, super::MifRhError::LockHeld { .. }));
    }

    #[test]
    fn a_stale_lock_from_a_dead_pid_is_cleared_automatically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".review.lock");
        // A PID astronomically unlikely to be alive on any real system.
        std::fs::write(&path, "999999999").unwrap();

        let lock = ReviewLock::acquire(&path);
        assert!(
            lock.is_ok(),
            "expected a stale lock to be cleared, got {lock:?}"
        );
    }

    #[test]
    fn acquiring_again_after_release_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".review.lock");

        drop(ReviewLock::acquire(&path).unwrap());
        let second = ReviewLock::acquire(&path);
        assert!(second.is_ok());
    }
}
