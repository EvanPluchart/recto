//! Atomic file writes: a crash or a full disk never leaves a half-written PDF in place.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Writes `bytes` to a temporary file in `path`'s folder, flushes it to disk, then renames it
/// over `path`. The rename replaces an existing file on every platform.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    write(path, bytes, true)
}

/// `write_atomic` for a file only its owner may read (saved signatures, recent files): on Unix
/// it is left with mode 0600 whatever the file had before.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    write(path, bytes, false)
}

#[cfg_attr(not(unix), allow(unused_variables))]
fn write(path: &Path, bytes: &[u8], keep_permissions: bool) -> std::io::Result<()> {
    let temporary = temporary_path(path)?;
    // A new file only: a name planted in a shared folder (a link to another file) is refused, and
    // left alone.
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&temporary)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        // The replaced file keeps its permissions (on Windows the folder's ACL applies to both).
        #[cfg(unix)]
        if keep_permissions && let Ok(metadata) = std::fs::metadata(path) {
            std::fs::set_permissions(&temporary, metadata.permissions())?;
        }
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// A temporary file next to `path`, unique per call: two saves of the same file running at once
/// never write into each other's temporary file.
fn temporary_path(path: &Path) -> std::io::Result<PathBuf> {
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let folder = path.parent().filter(|dir| !dir.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().ok_or_else(|| std::io::Error::other("the target has no file name"))?;
    let serial = WRITES.fetch_add(1, Ordering::Relaxed);
    Ok(folder.join(format!(".{}.recto-{}-{serial}.tmp", name.to_string_lossy(), std::process::id())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_write_gets_its_own_temporary_file() {
        let target = Path::new("folder").join("out.pdf");
        let (a, b) = (temporary_path(&target).unwrap(), temporary_path(&target).unwrap());
        assert_ne!(a, b);
        assert_eq!(a.parent(), Some(Path::new("folder")));
    }

    #[test]
    fn writes_and_replaces() {
        let folder = std::env::temp_dir().join(format!("recto-save-test-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let target = folder.join("out.pdf");
        write_atomic(&target, b"first").unwrap();
        write_atomic(&target, b"second, longer").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"second, longer");
        // Nothing left behind but the target.
        let entries: Vec<_> = std::fs::read_dir(&folder).unwrap().collect();
        assert_eq!(entries.len(), 1);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn concurrent_writes_leave_one_whole_file() {
        let folder = std::env::temp_dir().join(format!("recto-save-race-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let target = folder.join("out.pdf");
        let payloads: Vec<Vec<u8>> = (0..4u8).map(|n| vec![n; 2_000_000]).collect();
        std::thread::scope(|scope| {
            for payload in &payloads {
                let target = &target;
                // On Windows a rename can lose to another one in progress; that write fails cleanly.
                scope.spawn(move || {
                    let _ = write_atomic(target, payload);
                });
            }
        });
        let written = std::fs::read(&target).unwrap();
        assert!(payloads.contains(&written), "the target mixes several writes");
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1, "temporary files left behind");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_failed_write_keeps_the_original() {
        let folder = std::env::temp_dir().join(format!("recto-save-fail-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let target = folder.join("out.pdf");
        write_atomic(&target, b"original").unwrap();
        // A folder where the target should be makes the rename fail.
        let blocked = folder.join("blocked");
        std::fs::create_dir_all(blocked.join("child")).unwrap();
        assert!(write_atomic(&blocked, b"new").is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 2);
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
