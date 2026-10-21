//! Recto binary: `recto [file.pdf] [--page=N] [--zoom=PCT] [--dark|--light]`.
//! M0 shows one page, rendered on the render thread.

mod i18n;
mod viewer;

use std::path::PathBuf;

use gpui::{App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};

use crate::i18n::t;
use crate::viewer::Viewer;

#[derive(Clone, Debug, PartialEq)]
pub struct Args {
    pub file: Option<PathBuf>,
    /// 1-based, as typed by the user.
    pub page: usize,
    /// Percent of the page's natural size (100 = 1 pt per logical pixel).
    pub zoom: f32,
    /// `Some(true)` for `--dark`, `Some(false)` for `--light`, `None` to follow the system.
    pub dark: Option<bool>,
}

impl Args {
    fn parse(args: impl IntoIterator<Item = String>) -> Self {
        let mut parsed = Args { file: None, page: 1, zoom: 100.0, dark: None };
        for arg in args {
            if let Some(n) = arg.strip_prefix("--page=") {
                parsed.page = n.parse().ok().filter(|&n| n >= 1).unwrap_or(1);
            } else if let Some(pct) = arg.strip_prefix("--zoom=") {
                parsed.zoom = pct.trim_end_matches('%').parse().ok().filter(|z: &f32| (5.0..=1600.0).contains(z)).unwrap_or(100.0);
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
        let args = parse("a.pdf --page=3 --zoom=150 --dark");
        assert_eq!(args, Args { file: Some("a.pdf".into()), page: 3, zoom: 150.0, dark: Some(true) });
    }

    #[test]
    fn falls_back_on_bad_values() {
        let args = parse("--page=0 --zoom=abc --light");
        assert_eq!(args, Args { file: None, page: 1, zoom: 100.0, dark: Some(false) });
    }
}
