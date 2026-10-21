//! Atomic file writes: a crash or a full disk never leaves a half-written PDF in place.

use std::io::Write;
use std::path::Path;

/// Writes `bytes` to a temporary file in `path`'s folder, flushes it to disk, then renames it
/// over `path`. The rename replaces an existing file on every platform.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let folder = path.parent().filter(|dir| !dir.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().ok_or_else(|| std::io::Error::other("the target has no file name"))?;
    let temporary = folder.join(format!(".{}.recto-{}.tmp", name.to_string_lossy(), std::process::id()));
    let result = (|| {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
