//! Recto binary: `recto [file.pdf] [--page=N] [--zoom=PCT] [--sidebar=thumbnails|outline] [--tool=NAME] [--find=TEXT] [--apply=EDITS] [--save-as=PATH] [--dark|--light]`.
//! Opens one document in a continuous, zoomable page view.

mod i18n;
mod recent;
mod signatures;
mod text_input;
mod theme;
mod viewer;

use std::path::PathBuf;

use gpui::{App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};

use crate::i18n::t;
use crate::viewer::{EditSpec, Tab, Tool, Viewer, parse_edits};

#[derive(Clone, Debug, PartialEq)]
pub struct Args {
    pub file: Option<PathBuf>,
    /// 1-based, as typed by the user.
    pub page: usize,
    /// Percent of the page's natural size (100 = 1 pt per logical pixel); `None` fits the width.
    pub zoom: Option<f32>,
    /// `Some(true)` for `--dark`, `Some(false)` for `--light`, `None` to follow the system.
    pub dark: Option<bool>,
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
}

impl Args {
    fn parse(args: impl IntoIterator<Item = String>) -> Self {
        let mut parsed = Args { file: None, page: 1, zoom: None, dark: None, sidebar: None, find: None, apply: Vec::new(), save_as: None, tool: None };
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
            } else if arg == "--dark" {
                parsed.dark = Some(true);
            } else if arg == "--light" {
                parsed.dark = Some(false);
            } else if arg.starts_with("--") {
                eprintln!("recto: ignoring unknown option {arg}");
            } else {
                parsed.file = Some(PathBuf::from(arg));
            }
        }
        parsed
    }
}

fn main() {
    let args = Args::parse(std::env::args().skip(1));
    Application::new().run(move |cx: &mut App| {
        viewer::bind_keys(cx);
        let bounds = Bounds::centered(None, size(px(900.), px(1000.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some(t("Recto").into()), ..Default::default() }),
            window_min_size: Some(size(px(320.), px(240.))),
            ..Default::default()
        };
        if let Err(error) = cx.open_window(options, |window, cx| cx.new(|cx| Viewer::new(args, window, cx))) {
            eprintln!("recto: cannot open a window: {error:#}");
            cx.quit();
            return;
        }
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Args {
        Args::parse(line.split_whitespace().map(str::to_owned))
    }

    #[test]
    fn parses_test_flags() {
        let args = parse("a.pdf --page=3 --zoom=150 --dark --sidebar=outline");
        let expected =
            Args { file: Some("a.pdf".into()), page: 3, zoom: Some(150.0), dark: Some(true), sidebar: Some(Tab::Outline), find: None, apply: Vec::new(), save_as: None, tool: None };
        assert_eq!(args, expected);
    }

    #[test]
    fn falls_back_on_bad_values() {
        let args = parse("--page=0 --zoom=abc --light");
        assert_eq!(args, Args { file: None, page: 1, zoom: None, dark: Some(false), sidebar: None, find: None, apply: Vec::new(), save_as: None, tool: None });
    }
}
