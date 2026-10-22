//! Recently opened files, one path per line in the user's config folder.

use std::path::{Path, PathBuf};

const MAX: usize = 12;
const FILE: &str = "recent.txt";

/// `RECTO_CONFIG_DIR`, else the platform's per-user config folder.
pub fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("RECTO_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home().map(|h| h.join(".config")))
    };
    Some(base?.join(if cfg!(any(windows, target_os = "macos")) { "Recto" } else { "recto" }))
}

/// Recent files that still exist, most recent first.
pub fn load() -> Vec<PathBuf> {
    let Some(dir) = config_dir() else { return Vec::new() };
    let text = std::fs::read_to_string(dir.join(FILE)).unwrap_or_default();
    parse(&text).into_iter().filter(|path| path.is_file()).collect()
}

/// Puts `path` first and saves the list. Failures are reported, never fatal.
pub fn record(path: &Path) -> Vec<PathBuf> {
    // `absolute`, not `canonicalize`: no `\\?\` prefix on Windows, and the path stays as the user knows it.
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
    let list = push(load(), path);
    if let Some(dir) = config_dir()
        && let Err(error) = save(&dir, &list)
    {
        eprintln!("recto: cannot save recent files: {error}");
    }
    list
}

fn parse(text: &str) -> Vec<PathBuf> {
    text.lines().map(str::trim).filter(|line| !line.is_empty()).map(PathBuf::from).collect()
}

fn push(mut list: Vec<PathBuf>, path: PathBuf) -> Vec<PathBuf> {
    list.retain(|existing| existing != &path);
    list.insert(0, path);
    list.truncate(MAX);
    list
}

/// Written atomically, so a crash never leaves half a list.
fn save(dir: &Path, list: &[PathBuf]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let text: String = list.iter().filter_map(|path| path.to_str()).map(|path| format!("{path}\n")).collect();
    recto_core::save::write_private(&dir.join(FILE), text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn most_recent_first_without_duplicates() {
        let list = push(vec!["a.pdf".into(), "b.pdf".into()], "b.pdf".into());
        assert_eq!(list, vec![PathBuf::from("b.pdf"), PathBuf::from("a.pdf")]);
        let long = (0..20).fold(Vec::new(), |list, i| push(list, format!("{i}.pdf").into()));
        assert_eq!(long.len(), MAX);
        assert_eq!(long[0], PathBuf::from("19.pdf"));
    }

    #[test]
    fn round_trips_through_the_file() {
        let dir = std::env::temp_dir().join(format!("recto-recent-test-{}", std::process::id()));
        let list = vec![PathBuf::from("/x/one.pdf"), PathBuf::from("C:\\y\\two words.pdf")];
        save(&dir, &list).unwrap();
        assert_eq!(parse(&std::fs::read_to_string(dir.join(FILE)).unwrap()), list);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
