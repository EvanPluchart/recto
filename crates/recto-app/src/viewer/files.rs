//! Opening documents: the file dialog, dropped files, recent files and the start screen.

use std::path::{Path, PathBuf};

use gpui::{
    AnyElement, Context, ExternalPaths, IntoElement, ParentElement, PathPromptOptions, SharedString, Styled, Window,
    div, prelude::*, px,
};

use recto_core::Document;

use super::password::Locked;
use super::{State, Viewer, failure};
use crate::i18n::t;
use crate::recent;

/// File name and folder, for lists.
fn describe(path: &Path) -> (SharedString, SharedString) {
    let name = path.file_name().map_or_else(|| path.to_string_lossy(), |name| name.to_string_lossy());
    let folder = path.parent().map(|dir| dir.to_string_lossy().into_owned()).unwrap_or_default();
    (name.into_owned().into(), folder.into())
}

impl Viewer {
    /// Opens `path` in place of the current document, after confirmation if it has unsaved changes.
    pub(super) fn open_path(&mut self, path: PathBuf, page: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_discard(window, cx, move |viewer, window, cx| viewer.replace_document(path, page, window, cx));
    }

    /// Replaces the current document with the one at `path`, showing page `page` (1-based).
    pub(super) fn replace_document(&mut self, path: PathBuf, page: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.time("open");
        // Dropping the previous document stops its render thread before the next one starts.
        self.state = State::Empty;
        self.selection = None;
        self.rail.clear();
        self.notice = None;
        self.dialog = None;
        self.recent_menu = false;
        self.search.restart();
        self.sidebar.forget_followed();
        self.set_scroll_position(recto_core::geom::Point { x: 0.0, y: 0.0 });
        let document = match Document::open(&path) {
            Ok(document) => document,
            Err(error) => {
                self.state = State::Error(failure("Cannot open the document", error));
                cx.notify();
                return;
            }
        };
        self.recent = recent::record(&path);
        let (name, _) = describe(&path);
        window.set_window_title(&format!("{name} — {}", t("Recto")));
        self.state = if document.needs_password() {
            State::Locked(Box::new(Locked::new(document, page, self.theme, window, cx)))
        } else {
            let bytes = document.bytes();
            match self.start(document, bytes, page.saturating_sub(1), None, window, cx) {
                Ok(open) => State::Open(Box::new(open)),
                Err(message) => State::Error(message),
            }
        };
        cx.notify();
    }

    pub(super) fn prompt_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let options = PathPromptOptions { files: true, directories: false, multiple: false, prompt: None };
        let chosen = cx.prompt_for_paths(options);
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await
                && let Some(path) = paths.into_iter().next()
            {
                this.update_in(cx, |viewer, window, cx| viewer.open_path(path, 1, window, cx)).ok();
            }
        })
        .detach();
    }

    pub(super) fn on_drop_paths(&mut self, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = paths.paths().first() {
            self.open_path(path.clone(), 1, window, cx);
        }
    }

    fn recent_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = self.theme;
        self.recent
            .iter()
            .enumerate()
            .map(|(row, path)| {
                let (name, folder) = describe(path);
                let path = path.clone();
                div()
                    .id(("recent", row))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(theme.radius))
                    .cursor_pointer()
                    .hover(move |style| style.bg(theme.hover))
                    .child(gpui::svg().path(crate::assets::icon("file-text")).flex_none().size(px(16.)).text_color(theme.muted))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(div().text_sm().text_color(theme.text).child(name))
                            .child(div().text_xs().text_color(theme.muted).whitespace_nowrap().overflow_hidden().text_ellipsis().child(folder)),
                    )
                    .on_click(cx.listener(move |viewer, _, window, cx| viewer.open_path(path.clone(), 1, window, cx)))
                    .into_any_element()
            })
            .collect()
    }

    /// Shown when no document is open: an open button and the recent files.
    pub(super) fn start_screen(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme;
        let mut column = div()
            .w(px(440.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .flex_none()
                    .w(px(64.))
                    .h(px(64.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(16.))
                    .bg(theme.active)
                    .child(gpui::svg().path(crate::assets::icon("file-text")).size(px(32.)).text_color(theme.accent)),
            )
            .child(div().mt(px(6.)).text_2xl().font_weight(gpui::FontWeight::SEMIBOLD).text_color(theme.text).child(t("Recto")))
            .child(div().text_sm().text_color(theme.muted).child(t("Read, organize, annotate and sign PDF files, offline.")))
            .child(
                div()
                    .id("start-open")
                    .mt(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(16.))
                    .py(px(9.))
                    .rounded(px(8.))
                    .bg(theme.accent)
                    .text_color(theme.on_accent)
                    .cursor_pointer()
                    .hover(move |style| style.bg(theme.accent_hover))
                    .child(gpui::svg().path(crate::assets::icon("folder-open")).size(px(16.)).text_color(theme.on_accent))
                    .child(t("Open a PDF…"))
                    .on_click(cx.listener(|viewer, _, window, cx| viewer.prompt_open(window, cx))),
            )
            .child(div().text_xs().text_color(theme.muted).child(t("Or drop a file onto this window.")));
        if !self.recent.is_empty() {
            column = column.child(
                div()
                    .mt(px(28.))
                    .w_full()
                    .flex()
                    .flex_col()
                    .p(px(6.))
                    .rounded(px(10.))
                    .bg(theme.toolbar)
                    .border_1()
                    .border_color(theme.border)
                    .child(div().px(px(10.)).py(px(6.)).text_xs().text_color(theme.muted).child(t("Recent files")))
                    .children(self.recent_rows(cx).into_iter().take(6)),
            );
        }
        div().flex_1().flex().justify_center().pt(px(96.)).bg(theme.canvas).child(column).into_any_element()
    }


    /// The recent-files menu under the toolbar, when open.
    pub(super) fn recent_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.recent_menu {
            return None;
        }
        let theme = self.theme;
        let body: Vec<AnyElement> = if self.recent.is_empty() {
            vec![div().p(px(10.)).text_sm().text_color(theme.muted).child(t("No recent files")).into_any_element()]
        } else {
            self.recent_rows(cx)
        };
        Some(
            div()
                .id("recent-menu")
                .absolute()
                .top(px(super::TOOLBAR_HEIGHT))
                .left(px(8.))
                .w(px(360.))
                .p(px(4.))
                .rounded(px(6.))
                .border_1()
                .border_color(theme.border)
                .bg(theme.toolbar)
                .shadow_lg()
                .children(body)
                .on_mouse_down_out(cx.listener(|viewer, _, _, cx| {
                    viewer.recent_menu = false;
                    cx.notify();
                }))
                .into_any_element(),
        )
    }
}
