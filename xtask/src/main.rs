//! Developer tasks: `cargo xtask <task>`.
//!
//! * `fetch-pdfium`: downloads the pinned PDFium build for the host into `target/pdfium/`.
//! * `gen-fixtures`: writes the committed fixtures in `tests/fixtures/`.
//! * `gen-large [PAGES]`: writes `target/large.pdf` (500 pages by default) for performance checks.

mod fixtures;

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// bblanchon/pdfium-binaries release. Must match the `pdfium_*` feature of `pdfium-render` in the
/// workspace `Cargo.toml`; upgrade both together, in a dedicated commit.
const PDFIUM_RELEASE: &str = "chromium/7881";

fn main() -> Result<()> {
    let task = std::env::args().nth(1);
    match task.as_deref() {
        Some("fetch-pdfium") => fetch_pdfium(),
        Some("gen-fixtures") => fixtures::generate(&workspace_root()),
        Some("gen-large") => {
            let pages = std::env::args().nth(2).and_then(|n| n.parse().ok()).unwrap_or(500);
            fixtures::generate_large(&workspace_root(), pages)
        }
        _ => {
            eprintln!("usage: cargo xtask <fetch-pdfium | gen-fixtures | gen-large [PAGES]>");
            std::process::exit(2);
        }
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask lives in the workspace").to_owned()
}

/// Archive name for the host, e.g. `pdfium-win-x64.tgz`.
fn pdfium_archive() -> Result<String> {
    let os = match std::env::consts::OS {
        "windows" => "win",
        "macos" => "mac",
        "linux" => "linux",
        other => bail!("no prebuilt PDFium for OS `{other}`"),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "x86",
        other => bail!("no prebuilt PDFium for architecture `{other}`"),
    };
    Ok(format!("pdfium-{os}-{arch}.tgz"))
}

/// Library file inside the archive and its name once extracted.
fn pdfium_library() -> (&'static str, &'static str) {
    match std::env::consts::OS {
        "windows" => ("bin/pdfium.dll", "pdfium.dll"),
        "macos" => ("lib/libpdfium.dylib", "libpdfium.dylib"),
        _ => ("lib/libpdfium.so", "libpdfium.so"),
    }
}

fn fetch_pdfium() -> Result<()> {
    let dest = workspace_root().join("target/pdfium");
    let (inner, file_name) = pdfium_library();
    let library = dest.join(file_name);
    let stamp = dest.join("RELEASE");
    if library.exists() && fs::read_to_string(&stamp).is_ok_and(|s| s.trim() == PDFIUM_RELEASE) {
        println!("PDFium {PDFIUM_RELEASE} already in {}", dest.display());
        return Ok(());
    }

    let archive = pdfium_archive()?;
    let url = format!("https://github.com/bblanchon/pdfium-binaries/releases/download/{PDFIUM_RELEASE}/{archive}");
    println!("downloading {url}");
    let mut compressed = Vec::new();
    ureq::get(&url)
        .call()
        .with_context(|| format!("GET {url}"))?
        .into_body()
        .into_reader()
        .read_to_end(&mut compressed)
        .context("reading the PDFium archive")?;

    fs::create_dir_all(&dest)?;
    let mut found = false;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(compressed.as_slice()));
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let target = match path.to_str() {
            Some(p) if p == inner => {
                found = true;
                library.clone()
            }
            Some("LICENSE") => dest.join("LICENSE"),
            _ => continue,
        };
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        fs::write(&target, bytes).with_context(|| format!("writing {}", target.display()))?;
    }
    if !found {
        bail!("{archive} does not contain {inner}");
    }
    fs::write(&stamp, PDFIUM_RELEASE)?;
    println!("PDFium {PDFIUM_RELEASE} extracted to {}", library.display());
    Ok(())
}
