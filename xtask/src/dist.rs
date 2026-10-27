//! `cargo xtask dist [--no-build]`: builds Recto in release mode and packages it for the host in
//! `target/dist/`.
//!
//! * Every system: `target/dist/Recto/`, the program with PDFium, the OCR models and the notices
//!   (`licenses/`). Needs `fetch-pdfium` and `fetch-ocr-models` first.
//! * Windows: a portable zip, and the installer `Recto-VERSION-windows-x64-setup.exe` when Inno
//!   Setup 6 is found (`ISCC` on the path, in the `ISCC` variable, or installed).
//! * macOS: `Recto.app` and `Recto-VERSION-macos.dmg`. Signed when `RECTO_MACOS_IDENTITY` names a
//!   Developer ID certificate, then notarised when `RECTO_NOTARY_PROFILE` names a `notarytool`
//!   keychain profile.
//! * Linux: an AppDir and, when `appimagetool` is found (on the path or in `APPIMAGETOOL`),
//!   `Recto-VERSION-x86_64.AppImage`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn dist(root: &Path, build: bool) -> Result<()> {
    if build {
        run(Command::new(env!("CARGO")).args(["build", "-p", "recto", "--release", "--locked"]).current_dir(root))?;
    }
    let target = root.join("target");
    let out = target.join("dist");
    let folder = out.join("Recto");
    if folder.exists() {
        fs::remove_dir_all(&folder).context("cleaning target/dist/Recto")?;
    }
    fs::create_dir_all(folder.join("licenses"))?;

    let (exe, shipped) = if cfg!(windows) { ("recto.exe", "Recto.exe") } else { ("recto", "recto") };
    copy(&target.join("release").join(exe), &folder.join(shipped))?;
    let library = match std::env::consts::OS {
        "windows" => "pdfium.dll",
        "macos" => "libpdfium.dylib",
        _ => "libpdfium.so",
    };
    copy(&target.join("pdfium").join(library), &folder.join(library)).context("run `cargo xtask fetch-pdfium` first")?;
    fs::create_dir_all(folder.join("ocr-models"))?;
    for model in ["text-detection.rten", "text-recognition.rten"] {
        copy(&target.join("ocr-models").join(model), &folder.join("ocr-models").join(model)).context("run `cargo xtask fetch-ocr-models` first")?;
    }
    notices(root, &target, &folder.join("licenses"))?;

    match std::env::consts::OS {
        "windows" => windows(root, &out, &folder),
        "macos" => macos(root, &out, &folder),
        _ => linux(root, &out, &folder),
    }
}

/// The licences of Recto and of what it ships.
fn notices(root: &Path, target: &Path, licenses: &Path) -> Result<()> {
    copy(&root.join("LICENSE"), &licenses.join("LICENSE.txt"))?;
    copy(&target.join("pdfium").join("LICENSE"), &licenses.join("PDFium-LICENSE.txt"))?;
    for notice in ["README.txt", "OCR-MODELS.txt"] {
        copy(&root.join("packaging/notices").join(notice), &licenses.join(notice))?;
    }
    let assets = root.join("crates/recto-app/assets");
    copy(&assets.join("LUCIDE-LICENSE"), &licenses.join("LUCIDE-LICENSE.txt"))?;
    for entry in fs::read_dir(assets.join("fonts"))? {
        let path = entry?.path();
        if path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("OFL-")) {
            copy(&path, &licenses.join(path.file_name().expect("checked")))?;
        }
    }
    Ok(())
}

fn windows(root: &Path, out: &Path, folder: &Path) -> Result<()> {
    // The portable version: Windows' tar writes zip archives.
    let zip = out.join(format!("Recto-{VERSION}-windows-x64.zip"));
    let _ = fs::remove_file(&zip);
    run(Command::new("tar").arg("-a").arg("-c").arg("-f").arg(&zip).arg("-C").arg(out).arg("Recto"))?;
    println!("{}", zip.display());
    let Some(iscc) = inno_setup() else {
        println!("Inno Setup 6 not found: no installer (set ISCC to its ISCC.exe)");
        return Ok(());
    };
    run(Command::new(iscc)
        .arg("/Q")
        .arg(format!("/DVersion={VERSION}"))
        .arg(format!("/DSource={}", folder.display()))
        .arg(format!("/DOutput={}", out.display()))
        .arg(root.join("packaging/windows/recto.iss")))?;
    println!("{}", out.join(format!("Recto-{VERSION}-windows-x64-setup.exe")).display());
    Ok(())
}

fn inno_setup() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = std::env::var_os("ISCC").map(PathBuf::from).into_iter().collect();
    for base in ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"] {
        if let Some(base) = std::env::var_os(base) {
            let base = PathBuf::from(base);
            candidates.push(base.join("Inno Setup 6/ISCC.exe"));
            candidates.push(base.join("Programs/Inno Setup 6/ISCC.exe"));
        }
    }
    candidates.into_iter().find(|path| path.is_file()).or_else(|| on_path("ISCC.exe"))
}

fn macos(root: &Path, out: &Path, folder: &Path) -> Result<()> {
    let app = out.join("Recto.app");
    if app.exists() {
        fs::remove_dir_all(&app)?;
    }
    let contents = app.join("Contents");
    for dir in ["MacOS", "Frameworks", "Resources"] {
        fs::create_dir_all(contents.join(dir))?;
    }
    copy(&folder.join("recto"), &contents.join("MacOS/recto"))?;
    copy(&folder.join("libpdfium.dylib"), &contents.join("Frameworks/libpdfium.dylib"))?;
    copy_dir(&folder.join("ocr-models"), &contents.join("Resources/ocr-models"))?;
    copy_dir(&folder.join("licenses"), &contents.join("Resources/licenses"))?;
    let plist = fs::read_to_string(root.join("packaging/macos/Info.plist"))?.replace("@VERSION@", VERSION);
    fs::write(contents.join("Info.plist"), plist)?;

    // The icon: an iconset from the brand PNGs, made into Recto.icns.
    let iconset = out.join("Recto.iconset");
    fs::create_dir_all(&iconset)?;
    let png = root.join("assets/brand/png");
    for (size, scale) in [(16, 1), (16, 2), (32, 1), (32, 2), (128, 1), (128, 2), (256, 1), (256, 2), (512, 1), (512, 2)] {
        let name = if scale == 1 { format!("icon_{size}x{size}.png") } else { format!("icon_{size}x{size}@2x.png") };
        copy(&png.join(format!("recto-app-icon-{}.png", size * scale)), &iconset.join(name))?;
    }
    run(Command::new("iconutil").arg("-c").arg("icns").arg(&iconset).arg("-o").arg(contents.join("Resources/Recto.icns")))?;
    fs::remove_dir_all(&iconset)?;

    // Inside out: the library, then the bundle. Without a Developer ID the bundle is still signed
    // ad hoc (free, no Apple account): the linker's signature covers only the executable, and a
    // downloaded bundle without a sealed one is reported "damaged", with no way to open it.
    let identity = std::env::var("RECTO_MACOS_IDENTITY").ok();
    for path in [contents.join("Frameworks/libpdfium.dylib"), app.clone()] {
        let mut codesign = Command::new("codesign");
        codesign.arg("--force");
        match &identity {
            // With the hardened runtime notarisation needs.
            Some(identity) => codesign.args(["--timestamp", "--options", "runtime", "--sign", identity]),
            None => codesign.args(["--sign", "-"]),
        };
        run(codesign.arg(path))?;
    }
    if identity.is_none() {
        println!("RECTO_MACOS_IDENTITY not set: the app is signed ad hoc (Gatekeeper asks to confirm the first launch)");
    }

    let dmg = out.join(format!("Recto-{VERSION}-macos.dmg"));
    let _ = fs::remove_file(&dmg);
    disk_image(root, out, &app, &dmg)?;
    if let (Some(_), Ok(profile)) = (&identity, std::env::var("RECTO_NOTARY_PROFILE")) {
        run(Command::new("xcrun").args(["notarytool", "submit", "--keychain-profile", &profile, "--wait"]).arg(&dmg))?;
        run(Command::new("xcrun").args(["stapler", "staple"]).arg(&dmg))?;
    }
    println!("{}", dmg.display());
    Ok(())
}

/// The .dmg: Recto beside a link to Applications, over a background that says to drag one onto
/// the other. Finder lays the window out through AppleScript; if it cannot (no GUI session), the
/// image keeps the default layout.
fn disk_image(root: &Path, out: &Path, app: &Path, dmg: &Path) -> Result<()> {
    let stage = out.join("dmg");
    let _ = fs::remove_dir_all(&stage);
    fs::create_dir_all(stage.join(".background"))?;
    run(Command::new("ditto").arg(app).arg(stage.join("Recto.app")))?;
    std::os::unix::fs::symlink("/Applications", stage.join("Applications"))?;
    copy(&root.join("packaging/macos/dmg-background.tiff"), &stage.join(".background/background.tiff"))?;

    let writable = out.join("Recto-rw.dmg");
    let _ = fs::remove_file(&writable);
    run(Command::new("hdiutil").args(["create", "-volname", "Recto", "-fs", "HFS+", "-format", "UDRW", "-srcfolder"]).arg(&stage).arg(&writable))?;
    let mount = out.join("dmg-mount");
    run(Command::new("hdiutil").args(["attach", "-readwrite", "-noverify", "-noautoopen", "-mountpoint"]).arg(&mount).arg(&writable))?;
    let layout = format!(
        r#"tell application "Finder"
            set f to POSIX file "{}" as alias
            open f
            delay 2
            set w to container window of f
            set current view of w to icon view
            set toolbar visible of w to false
            set statusbar visible of w to false
            set bounds of w to {{200, 120, 860, 520}}
            set o to icon view options of w
            set arrangement of o to not arranged
            set icon size of o to 112
            set text size of o to 13
            set background picture of o to file ".background:background.tiff" of f
            set position of item "Recto.app" of f to {{165, 190}}
            set position of item "Applications" of f to {{495, 190}}
            update f without registering applications
            delay 1
            close w
        end tell"#,
        mount.display()
    );
    let laid_out = Command::new("osascript").arg("-e").arg(&layout).status().is_ok_and(|status| status.success());
    if !laid_out {
        println!("Finder could not lay the .dmg window out: it keeps the default layout");
    }
    let _ = fs::remove_dir_all(mount.join(".fseventsd"));
    run(Command::new("hdiutil").arg("detach").arg(&mount))?;
    run(Command::new("hdiutil").args(["convert", "-format", "UDZO", "-imagekey", "zlib-level=9", "-o"]).arg(dmg).arg(&writable))?;
    fs::remove_file(&writable)?;
    fs::remove_dir_all(&stage)?;
    Ok(())
}

fn linux(root: &Path, out: &Path, folder: &Path) -> Result<()> {
    let appdir = out.join("Recto.AppDir");
    if appdir.exists() {
        fs::remove_dir_all(&appdir)?;
    }
    let bin = appdir.join("usr/bin");
    fs::create_dir_all(&bin)?;
    // The program finds PDFium and the models beside itself.
    copy(&folder.join("recto"), &bin.join("recto"))?;
    copy(&folder.join("libpdfium.so"), &bin.join("libpdfium.so"))?;
    copy_dir(&folder.join("ocr-models"), &bin.join("ocr-models"))?;
    copy_dir(&folder.join("licenses"), &appdir.join("usr/share/doc/recto"))?;
    copy(&root.join("packaging/linux/recto.desktop"), &appdir.join("recto.desktop"))?;
    copy(&root.join("assets/brand/png/recto-app-icon-256.png"), &appdir.join("recto.png"))?;
    let icons = appdir.join("usr/share/icons/hicolor/256x256/apps");
    fs::create_dir_all(&icons)?;
    copy(&root.join("assets/brand/png/recto-app-icon-256.png"), &icons.join("recto.png"))?;
    let app_run = appdir.join("AppRun");
    fs::write(&app_run, "#!/bin/sh\nHERE=\"$(dirname \"$(readlink -f \"$0\")\")\"\nexec \"$HERE/usr/bin/recto\" \"$@\"\n")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&app_run, fs::Permissions::from_mode(0o755))?;
    }
    let Some(tool) = std::env::var_os("APPIMAGETOOL").map(PathBuf::from).or_else(|| on_path("appimagetool")) else {
        println!("appimagetool not found: {} only (set APPIMAGETOOL)", appdir.display());
        return Ok(());
    };
    let image = out.join(format!("Recto-{VERSION}-x86_64.AppImage"));
    run(Command::new(tool).env("ARCH", "x86_64").arg(&appdir).arg(&image))?;
    println!("{}", image.display());
    Ok(())
}

fn on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join(name)).find(|path| path.is_file())
}

fn copy(from: &Path, to: &Path) -> Result<()> {
    fs::copy(from, to).with_context(|| format!("copying {}", from.display()))?;
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from).with_context(|| format!("reading {}", from.display()))? {
        let path = entry?.path();
        let name = path.file_name().expect("a directory entry has a name");
        if path.is_dir() { copy_dir(&path, &to.join(name))? } else { copy(&path, &to.join(name))? }
    }
    Ok(())
}

fn run(command: &mut Command) -> Result<()> {
    let status = command.status().with_context(|| format!("running {command:?}"))?;
    if !status.success() {
        bail!("{command:?} failed: {status}");
    }
    Ok(())
}
