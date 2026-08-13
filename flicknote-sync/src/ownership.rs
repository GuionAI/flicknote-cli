use fs4::{FileExt, TryLockError};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

pub(crate) const LOCK_FILE_NAME: &str = "daemon.lock";

#[derive(Debug, Error)]
pub(crate) enum OwnershipError {
    #[error(
        "FlickNote daemon already owns this data directory ({lock_path}); stop it with `flicknote daemon stop` before running the foreground daemon; lock diagnostics: {metadata}"
    )]
    AlreadyOwned {
        lock_path: PathBuf,
        metadata: String,
    },
    #[error("Unable to acquire daemon data-directory lock: {0}")]
    Io(#[from] io::Error),
}

/// Holds the kernel lock for the complete lifetime of a daemon.
///
/// The file remains on disk after the guard is dropped. Its contents are only
/// diagnostic metadata; the open file descriptor is the ownership authority.
#[derive(Debug)]
pub(crate) struct DataDirectoryLock {
    _file: File,
}

impl DataDirectoryLock {
    pub(crate) fn acquire(data_dir: &Path) -> Result<Self, OwnershipError> {
        fs::create_dir_all(data_dir)?;
        let path = data_dir.join(LOCK_FILE_NAME);
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)?;

        match FileExt::try_lock(&file) {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(OwnershipError::AlreadyOwned {
                    lock_path: path,
                    metadata: diagnostic_metadata(&file),
                });
            }
            Err(TryLockError::Error(error)) => return Err(OwnershipError::Io(error)),
        }

        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        writeln!(file, "pid={}", std::process::id())?;
        writeln!(file, "version={}", env!("CARGO_PKG_VERSION"))?;
        writeln!(file, "started_at={}", unix_timestamp())?;
        file.flush()?;

        Ok(Self { _file: file })
    }
}

fn diagnostic_metadata(file: &File) -> String {
    let Some(mut clone) = file.try_clone().ok() else {
        return "unavailable".to_string();
    };
    let mut contents = String::new();
    use std::io::Read;
    if clone.read_to_string(&mut contents).is_err() {
        return "unavailable".to_string();
    }
    let metadata = contents.trim();
    if metadata.is_empty() {
        "unavailable".to_string()
    } else {
        metadata.chars().take(256).collect()
    }
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_daemon_can_hold_a_data_directory_lock() {
        let directory = tempfile::tempdir().unwrap();
        let first = DataDirectoryLock::acquire(directory.path()).unwrap();

        let error = DataDirectoryLock::acquire(directory.path()).unwrap_err();
        assert!(error.to_string().contains("already owns"));
        assert!(error.to_string().contains("pid="));

        drop(first);
        DataDirectoryLock::acquire(directory.path()).unwrap();
    }

    #[test]
    fn different_data_directories_have_independent_ownership() {
        let first_directory = tempfile::tempdir().unwrap();
        let second_directory = tempfile::tempdir().unwrap();
        let _first = DataDirectoryLock::acquire(first_directory.path()).unwrap();
        let _second = DataDirectoryLock::acquire(second_directory.path()).unwrap();
    }
}
