//! Updates from the app (decision 0006): Recto asks GitHub for the newest release at launch and
//! every 6 hours, and installs it on one click. The steps that touch the network or the system
//! are here; the viewer shows where it stands (`viewer/update.rs`). Windows only for now.

#[cfg(windows)]
mod windows;

use std::path::{Path, PathBuf};
use std::time::Duration;

use recto_core::update::{self as releases, Version};


/// The first check, once the window is up; then one every `EVERY`.
pub const FIRST: Duration = Duration::from_secs(3);
pub const EVERY: Duration = Duration::from_secs(6 * 3600);

/// No window, no prompts, never closing another Recto (one may hold unsaved work), and Recto
/// started again at the end (`recto.iss`).
#[cfg(windows)]
const INSTALLER_ARGS: [&str; 5] = ["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/NOCLOSEAPPLICATIONS", "/relaunch=1"];

/// Where the update stands, for the button in the app bar.
#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Idle,
    /// A newer version is out.
    Available(Version),
    Downloading(Version),
    /// Why the last attempt failed, and the version it was for.
    Failed(Failure, Version),
    /// Started again by the installer after an update: links to what is new.
    Installed(Version),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// Some variants only arise on Windows, the one system with updates so far.
#[cfg_attr(not(windows), allow(dead_code))]
pub enum Failure {
    /// The release publishes no checksum for its installer: it is never run.
    NoChecksum,
    /// The download does not match its checksum: deleted, never run.
    Mismatch,
    Download,
    /// Another Recto window is open: it may hold unsaved work, so nothing closes it.
    OthersOpen,
    NotStarted,
}

impl Failure {
    /// What went wrong and what to do, in English: `i18n::t` translates it where it shows.
    pub fn message(self) -> &'static str {
        match self {
            Failure::NoChecksum => "No checksum for this update: download it from its page",
            Failure::Mismatch => "The update was damaged in transit; click to try again",
            Failure::Download => "The update could not be downloaded; click to try again",
            Failure::OthersOpen => "Close the other Recto windows, then click to update",
            Failure::NotStarted => "The installer did not start; click to try again",
        }
    }
}

/// What the button can do in this process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// Some variants only arise on Windows, the one system with updates so far.
#[cfg_attr(not(windows), allow(dead_code))]
pub enum Mode {
    /// An install for this account: downloads, checks and installs.
    Install,
    /// Installed for every account, or not installed (the zip): opens the release page.
    ReleasePage,
    /// Running as administrator: never runs a downloaded installer, opens the page.
    Elevated,
    /// Not on this system yet.
    Unsupported,
}

pub fn mode() -> Mode {
    #[cfg(windows)]
    {
        if windows::is_elevated() {
            Mode::Elevated
        } else if windows::install_kind() == windows::Install::PerUser {
            Mode::Install
        } else {
            Mode::ReleasePage
        }
    }
    #[cfg(not(windows))]
    {
        Mode::Unsupported
    }
}

/// A newer release than `current`, if any. `None` too when GitHub cannot be reached: being
/// offline is normal, and the next check tries again.
pub fn check(current: &str) -> Option<Version> {
    #[cfg(windows)]
    {
        let to = windows::redirect_of(releases::LATEST_URL).ok()?;
        releases::newer(current, Version::from_release_url(&to)?)
    }
    #[cfg(not(windows))]
    {
        let _ = current;
        None
    }
}

/// Downloads the installer of `version` into a new folder and checks it against the release's
/// `SHA256SUMS`. A missing checksum refuses the update.
pub fn fetch(version: Version) -> Result<PathBuf, Failure> {
    #[cfg(windows)]
    {
        let name = version.setup_name();
        let sums = windows::text(&version.download_url("SHA256SUMS")).map_err(|_| Failure::NoChecksum)?;
        let expected = releases::checksum_for(&sums, &name).ok_or(Failure::NoChecksum)?;
        let folder = fresh_folder().ok_or(Failure::Download)?;
        let path = folder.join(&name);
        let checked = windows::download(&version.download_url(&name), &path)
            .map_err(|_| Failure::Download)
            .and_then(|()| (windows::sha256_file(&path) == Some(expected)).then_some(()).ok_or(Failure::Mismatch));
        if let Err(failure) = checked {
            let _ = std::fs::remove_dir_all(&folder);
            return Err(failure);
        }
        Ok(path)
    }
    #[cfg(not(windows))]
    {
        let _ = version;
        Err(Failure::Download)
    }
}

/// A new, empty folder for one download, under the temporary folder.
#[cfg(windows)]
fn fresh_folder() -> Option<PathBuf> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
    let dir = std::env::temp_dir().join(format!("recto-update-{}-{stamp}", std::process::id()));
    // `create_dir` fails if it exists: the folder is ours alone.
    std::fs::create_dir(&dir).ok()?;
    Some(dir)
}

/// Marks this process as a running Recto, for the installer to wait on.
pub fn mark_running() {
    #[cfg(windows)]
    windows::mark_running();
}

/// Lets go of the running mark before quitting for the installer; `false` (and the mark taken
/// back) when another Recto runs.
pub fn release_unless_others() -> bool {
    #[cfg(windows)]
    {
        windows::release_unless_others()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Starts the checked installer, detached; this process should then quit.
pub fn start_installer(path: &Path) -> bool {
    #[cfg(windows)]
    {
        std::process::Command::new(path).args(INSTALLER_ARGS).spawn().is_ok()
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

/// The document to open again once the installer starts Recto: next to the recent files.
const REOPEN: &str = "reopen-after-update.txt";

pub fn save_reopen(document: &Path) {
    if let Some(dir) = crate::recent::config_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(REOPEN), document.to_string_lossy().as_bytes());
    }
}

/// The document saved by `save_reopen`, once: the file is removed.
pub fn take_reopen() -> Option<PathBuf> {
    let file = crate::recent::config_dir()?.join(REOPEN);
    let text = std::fs::read_to_string(&file).ok()?;
    let _ = std::fs::remove_file(&file);
    let path = PathBuf::from(text.trim());
    path.is_file().then_some(path)
}

/// The release page of `version`.
pub fn release_page(version: Version) -> String {
    releases::release_page(version)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// 0.1.0 was published before releases carried `SHA256SUMS`: its installer is refused,
    /// never run. Goes online, so only by hand: `cargo test -p recto -- --ignored checksum`.
    #[test]
    #[ignore = "network"]
    fn a_release_without_checksums_is_refused() {
        assert_eq!(fetch(Version::parse("0.1.0").unwrap()), Err(Failure::NoChecksum));
    }

    /// What "Update" does before running anything: 0.1.1's installer downloads and matches its
    /// published checksum. Online, so by hand: `cargo test -p recto -- --ignored published`.
    #[test]
    #[ignore = "network"]
    fn a_published_release_downloads_and_checks() {
        let path = fetch(Version::parse("0.1.1").unwrap()).expect("downloaded and checked");
        assert!(path.ends_with("Recto-0.1.1-windows-x64-setup.exe"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Online, by hand: the check finds 0.1.0 newer than a pretend 0.0.1.
    #[test]
    #[ignore = "network"]
    fn the_latest_release_is_found() {
        assert!(check("0.0.1").is_some());
        assert_eq!(check("999.0.0"), None);
    }
}
