//! A small modal for extract and split: one field, a mode switch for split, and errors in place.

use gpui::{AnyElement, Context, Entity, IntoElement, ParentElement, SharedString, Styled, Window, div, prelude::*, px};

use super::Viewer;
use super::output::SplitMode;
use crate::i18n::t;
use crate::text_input::{TextInput, TextInputEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogKind {
    Extract,
    Split(SplitMode),
}

pub struct Dialog {
    kind: DialogKind,
    input: Entity<TextInput>,
    error: Option<SharedString>,
}

/// `1-3, 7` for consecutive runs of 0-based `pages`, shown 1-based.
pub fn describe_pages(pages: &[usize]) -> String {
    let mut parts = Vec::new();
    let mut iter = pages.iter().copied().peekable();
    while let Some(start) = iter.next() {
        let mut end = start;
        while iter.peek() == Some(&(end + 1)) {
            end = iter.next().unwrap_or(end);
        }
        parts.push(if start == end { format!("{}", start + 1) } else { format!("{}-{}", start + 1, end + 1) });
    }
    parts.join(", ")
}

impl Viewer {
    pub(super) fn open_dialog(&mut self, kind: DialogKind, window: &mut Window, cx: &mut Context<Self>) {
        let initial = match kind {
            DialogKind::Extract => {
                let order = self.page_order();
                let selected: Vec<usize> = self
                    .target_pages()
                    .iter()
                    .filter_map(|page| order.iter().position(|p| p == page))
                    .collect();
                describe_pages(&selected)
            }
            DialogKind::Split(SplitMode::Every) => "1".to_owned(),
            DialogKind::Split(_) => String::new(),
        };
        let theme = self.theme;
        let input = cx.new(|cx| {
            let mut input = TextInput::new(t("e.g. 1-3, 5"), |_| true, theme, cx);
            input.set_text(initial, cx);
            input
        });
        cx.subscribe_in(&input, window, |viewer, _, event, window, cx| match event {
            TextInputEvent::Commit { .. } => viewer.submit_dialog(window, cx),
            TextInputEvent::Cancel => viewer.close_dialog(window, cx),
            TextInputEvent::Changed(_) => {}
        })
        .detach();
        input.update(cx, |input, cx| input.focus_all(window, cx));
        self.dialog = Some(Dialog { kind, input, error: None });
        cx.notify();
    }

    fn close_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dialog = None;
        window.focus(&self.focus);
        cx.notify();
    }

    fn submit_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = &self.dialog else { return };
        let kind = dialog.kind;
        let value = dialog.input.read(cx).text().to_owned();
        let result = match kind {
            DialogKind::Extract => self.extract(&value, window, cx),
            DialogKind::Split(mode) => self.split(mode, &value, window, cx),
        };
        match result {
            Ok(()) => self.close_dialog(window, cx),
            Err(error) => {
                if let Some(dialog) = &mut self.dialog {
                    dialog.error = Some(error.into());
                }
                cx.notify();
            }
        }
    }

    pub(super) fn dialog_view(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dialog = self.dialog.as_ref()?;
        let theme = self.theme;
        let (title, help) = match dialog.kind {
            DialogKind::Extract => (t("Extract pages"), t("Pages to copy into a new file, e.g. 1-3, 5.")),
            DialogKind::Split(SplitMode::Ranges) => (t("Split"), t("One file per range, e.g. 1-3, 4-10, 11-.")),
            DialogKind::Split(SplitMode::Every) => (t("Split"), t("Number of pages in each file.")),
            DialogKind::Split(SplitMode::Bookmarks) => (t("Split"), t("One file per top-level bookmark.")),
        };
        let button = |id: &'static str, label: &'static str, primary: bool| {
            div()
                .id(id)
                .px(px(12.))
                .py(px(5.))
                .rounded(px(5.))
                .cursor_pointer()
                .when(primary, |b| b.bg(theme.accent).text_color(gpui::white()))
                .when(!primary, |b| b.border_1().border_color(theme.border).hover(move |s| s.bg(theme.hover)))
                .child(t(label))
        };
        let mode = |id: &'static str, label: &'static str, this: SplitMode| {
            let active = dialog.kind == DialogKind::Split(this);
            div()
                .id(id)
                .px(px(8.))
                .py(px(3.))
                .rounded(px(4.))
                .cursor_pointer()
                .when(active, |b| b.bg(theme.selection))
                .hover(move |s| s.bg(theme.hover))
                .child(t(label))
                .on_click(cx.listener(move |viewer, _, window, cx| viewer.open_dialog(DialogKind::Split(this), window, cx)))
        };
        let mut card = div()
            .id("dialog")
            .w(px(380.))
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .rounded(px(8.))
            .bg(theme.toolbar)
            .border_1()
            .border_color(theme.border)
            .shadow_lg()
            .text_color(theme.text)
            .text_sm()
            .child(div().text_base().child(title));
        if matches!(dialog.kind, DialogKind::Split(_)) {
            card = card.child(
                div()
                    .flex()
                    .gap(px(4.))
                    .child(mode("split-ranges", "Ranges", SplitMode::Ranges))
                    .child(mode("split-every", "Every N pages", SplitMode::Every))
                    .child(mode("split-bookmarks", "Bookmarks", SplitMode::Bookmarks)),
            );
        }
        card = card.child(div().text_color(theme.muted).child(help));
        if dialog.kind != DialogKind::Split(SplitMode::Bookmarks) {
            card = card.child(dialog.input.clone());
        }
        if let Some(error) = &dialog.error {
            card = card.child(div().text_color(theme.error).child(error.clone()));
        }
        card = card.child(
            div()
                .flex()
                .justify_end()
                .gap(px(8.))
                .child(button("dialog-cancel", "Cancel", false).on_click(cx.listener(|viewer, _, window, cx| viewer.close_dialog(window, cx))))
                .child(button("dialog-ok", "Continue…", true).on_click(cx.listener(|viewer, _, window, cx| viewer.submit_dialog(window, cx)))),
        );
        // A dimmed backdrop that swallows clicks.
        Some(
            div()
                .id("dialog-backdrop")
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .pt(px(120.))
                .items_start()
                .bg(gpui::rgba(0x0000_0055))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(card)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_runs_of_pages() {
        assert_eq!(describe_pages(&[0, 1, 2, 4, 6, 7]), "1-3, 5, 7-8");
        assert_eq!(describe_pages(&[3]), "4");
        assert_eq!(describe_pages(&[]), "");
    }
}
