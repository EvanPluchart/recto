//! Recto binary: `recto [file.pdf] [--edit] [--page=N] [--zoom=PCT] [--sidebar=thumbnails|outline] [--tool=NAME] [--find=TEXT] [--apply=EDITS] [--save-as=PATH] [--export=PATH [--compress] [--grayscale] [--flatten] [--password=PASS]] [--ocr]`.
//! Opens one document in a continuous, zoomable page view.

// Release builds on Windows are GUI programs: no console window opens with them. Debug builds
// keep the console for logs (`RECTO_TIMING`, render errors).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod assets;
mod chrome;
mod fontmatch;
mod i18n;
mod recent;
mod signatures;
mod text_input;
mod theme;
mod update;
mod viewer;
#[cfg(windows)]
mod printer;

use std::path::PathBuf;

use gpui::{App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};

use recto_core::BuildOptions;
use recto_core::security::{Allowed, Protection, Security};

use crate::i18n::t;
use crate::viewer::{EditSpec, MenuKind, Tab, Tool, Viewer, parse_edits};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Args {
    pub file: Option<PathBuf>,
    /// 1-based, as typed by the user.
    pub page: usize,
    /// Percent of the page's natural size (100 = 1 pt per logical pixel); `None` fits the width.
    pub zoom: Option<f32>,
    /// Sidebar shown at start (`--sidebar=thumbnails|outline`).
    pub sidebar: Option<Tab>,
    /// Search opened at start (`--find=TEXT`).
    pub find: Option<String>,
    /// Edits applied at start (`--apply=rotate:2,delete:3,move:4>1,blank:0,undo`), for screenshots.
    pub apply: Vec<EditSpec>,
    /// Writes the (edited) document there at start, then keeps it open (`--save-as=PATH`).
    pub save_as: Option<PathBuf>,
    /// Tool selected at start (`--tool=NAME`).
    pub tool: Option<Tool>,
    /// Recognizes the text of every page at start (`--ocr`).
    pub ocr: bool,
    /// Exports a copy there at start (`--export=PATH`), with `--compress`, `--grayscale`,
    /// `--flatten` and `--password=PASS`.
    pub export: Option<PathBuf>,
    pub export_options: BuildOptions,
    /// Menu open at start (`--menu=pages|document`), for screenshots.
    pub menu: Option<MenuKind>,
    /// Form open at start (`--dialog=export|watermark|numbers|header|properties|extract|split`).
    pub dialog: Option<String>,
    /// Starts in edit mode (`--edit`); documents otherwise open read-only.
    pub edit: bool,
    /// With `--find`, replaces every occurrence at start (`--replace=TEXT`, edit mode).
    pub replace: Option<String>,
    /// Starts with annotations hidden (`--hide-annotations`).
    pub hide_annotations: bool,
    /// Never asks GitHub for a newer release (`--no-update-check`); also `RECTO_NO_UPDATE_CHECK`.
    pub no_update_check: bool,
    /// Behaves as this older version and checks for updates even with screenshot flags
    /// (`--pretend-version=X.Y.Z`), to see the update button.
    pub pretend_version: Option<String>,
    /// Started again by the installer after an update from the app (`--updated`).
    pub updated: bool,
}

impl Args {
    fn parse(args: impl IntoIterator<Item = String>) -> Self {
        let mut parsed = Args { page: 1, ..Args::default() };
        for arg in args {
            if let Some(n) = arg.strip_prefix("--page=") {
                parsed.page = n.parse().ok().filter(|&n| n >= 1).unwrap_or(1);
            } else if let Some(pct) = arg.strip_prefix("--zoom=") {
                parsed.zoom = pct.trim_end_matches('%').parse().ok().filter(|z: &f32| z.is_finite() && *z > 0.0);
            } else if let Some(name) = arg.strip_prefix("--sidebar=") {
                parsed.sidebar = Tab::parse(name);
            } else if let Some(spec) = arg.strip_prefix("--apply=") {
                match parse_edits(spec) {
                    Ok(edits) => parsed.apply = edits,
                    Err(error) => eprintln!("recto: ignoring --apply: {error}"),
                }
            } else if let Some(name) = arg.strip_prefix("--tool=") {
                parsed.tool = Tool::parse(name);
            } else if let Some(path) = arg.strip_prefix("--save-as=") {
                parsed.save_as = Some(PathBuf::from(path));
            } else if let Some(query) = arg.strip_prefix("--find=") {
                parsed.find = Some(query.to_owned());
            } else if let Some(path) = arg.strip_prefix("--export=") {
                parsed.export = Some(PathBuf::from(path));
            } else if arg == "--compress" {
                parsed.export_options.compress = true;
            } else if arg == "--grayscale" {
                parsed.export_options.grayscale = true;
            } else if arg == "--flatten" {
                parsed.export_options.flatten_forms = true;
            } else if let Some(password) = arg.strip_prefix("--password=") {
                parsed.export_options.security = Security::Protect(Protection {
                    user_password: password.to_owned(),
                    owner_password: String::new(),
                    allowed: Allowed::default(),
                });
            } else if let Some(name) = arg.strip_prefix("--menu=") {
                parsed.menu = MenuKind::parse(name);
            } else if let Some(name) = arg.strip_prefix("--dialog=") {
                parsed.dialog = Some(name.to_owned());
            } else if let Some(text) = arg.strip_prefix("--replace=") {
                parsed.replace = Some(text.to_owned());
            } else if arg == "--hide-annotations" {
                parsed.hide_annotations = true;
            } else if arg == "--edit" {
                parsed.edit = true;
            } else if arg == "--ocr" {
                parsed.ocr = true;
            } else if arg == "--no-update-check" {
                parsed.no_update_check = true;
            } else if let Some(version) = arg.strip_prefix("--pretend-version=") {
                parsed.pretend_version = Some(version.to_owned());
            } else if arg == "--updated" {
                parsed.updated = true;
            } else if arg.starts_with("--") {
                eprintln!("recto: ignoring unknown option {arg}");
            } else {
                parsed.file = Some(PathBuf::from(arg));
            }
        }
        parsed
    }

    /// Whether to look for updates: not when asked not to, nor in a run made for screenshots or
    /// tests (any flag that sets a state up), unless it pretends to be an older version.
    pub fn checks_updates(&self) -> bool {
        if self.pretend_version.is_some() {
            return true;
        }
        let staged = self.find.is_some()
            || !self.apply.is_empty()
            || self.save_as.is_some()
            || self.tool.is_some()
            || self.ocr
            || self.export.is_some()
            || self.menu.is_some()
            || self.dialog.is_some()
            || self.replace.is_some();
        !self.no_update_check && !staged && std::env::var_os("RECTO_NO_UPDATE_CHECK").is_none()
    }
}

fn main() {
    let mut args = Args::parse(std::env::args().skip(1));
    // For the installer: it waits until no Recto runs (decision 0006).
    update::mark_running();
    // Started again after an update from the app: the document that was open.
    if args.updated && args.file.is_none() {
        args.file = update::take_reopen();
    }
    // Files the system asks to open (macOS: double-click in the Finder, "Open with"); elsewhere
    // they come as arguments.
    let (opened, to_open) = flume::unbounded::<PathBuf>();
    let application = Application::new().with_assets(assets::Assets);
    application.on_open_urls(move |urls| {
        for url in urls {
            if let Some(path) = url.strip_prefix("file://").map(percent_decoded) {
                let _ = opened.send(PathBuf::from(path));
            }
        }
    });
    application.run(move |cx: &mut App| {
        viewer::bind_keys(cx);
        let bounds = Bounds::centered(None, size(px(900.), px(1000.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some(t("Recto").into()), ..Default::default() }),
            window_min_size: Some(size(px(320.), px(240.))),
            ..Default::default()
        };
        let window = match cx.open_window(options, |window, cx| cx.new(|cx| Viewer::new(args, window, cx))) {
            Ok(window) => window,
            Err(error) => {
                eprintln!("recto: cannot open a window: {error:#}");
                cx.quit();
                return;
            }
        };
        cx.spawn(async move |cx| {
            while let Ok(path) = to_open.recv_async().await {
                window.update(cx, |viewer, window, cx| viewer.open_file(path, window, cx)).ok();
            }
        })
        .detach();
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
    });
}

/// `text` with its `%XX` escapes decoded (a `file://` URL's path).
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|pair| std::str::from_utf8(pair).ok()).and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Args {
        Args::parse(line.split_whitespace().map(str::to_owned))
    }

    #[test]
    fn file_urls_are_decoded() {
        assert_eq!(percent_decoded("/Users/a/Mon%20fichier%C3%A9.pdf"), "/Users/a/Mon fichier\u{e9}.pdf");
        assert_eq!(percent_decoded("/a%2"), "/a%2");
    }

    #[test]
    fn parses_test_flags() {
        let args = parse("a.pdf --page=3 --zoom=150 --sidebar=outline");
        let expected =
            Args { file: Some("a.pdf".into()), page: 3, zoom: Some(150.0), sidebar: Some(Tab::Outline), ..Args::default() };
        assert_eq!(args, expected);
    }

    #[test]
    fn parses_export_flags() {
        let args = parse("a.pdf --export=out.pdf --compress --grayscale --password=secret --ocr --edit");
        assert!(args.edit);
        assert_eq!(args.export, Some(PathBuf::from("out.pdf")));
        assert!(args.ocr && args.export_options.compress && args.export_options.grayscale && !args.export_options.flatten_forms);
        assert!(matches!(&args.export_options.security, Security::Protect(protection) if protection.user_password == "secret"));
    }

    #[test]
    fn screenshot_runs_never_check_for_updates() {
        assert!(parse("a.pdf --page=2").checks_updates() || std::env::var_os("RECTO_NO_UPDATE_CHECK").is_some());
        assert!(!parse("a.pdf --menu=pages").checks_updates());
        assert!(!parse("a.pdf --no-update-check").checks_updates());
        assert!(parse("a.pdf --menu=pages --pretend-version=0.0.1").checks_updates());
        assert!(parse("--updated").updated);
    }

    #[test]
    fn falls_back_on_bad_values() {
        let args = parse("--page=0 --zoom=abc");
        assert_eq!(args, Args { page: 1, ..Args::default() });
    }
}
