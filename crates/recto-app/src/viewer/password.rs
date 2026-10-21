//! Encrypted documents: the password prompt shown before the pages.

use gpui::{AnyElement, Context, Entity, IntoElement, ParentElement, Styled, Window, div, prelude::*, px};
use recto_core::Document;

use super::{State, Viewer};
use crate::i18n::t;
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::Theme;

pub struct Locked {
    document: Document,
    /// Page to show once unlocked (1-based).
    page: usize,
    input: Entity<TextInput>,
    wrong: bool,
}

impl Locked {
    pub fn new(document: Document, page: usize, theme: Theme, window: &mut Window, cx: &mut Context<Viewer>) -> Self {
        let input = cx.new(|cx| TextInput::new(t("Password"), |_| true, theme, cx).masked());
        cx.subscribe_in(&input, window, |viewer, _, event, window, cx| {
            if let TextInputEvent::Commit { text, .. } = event {
                viewer.unlock(text.clone(), window, cx);
            }
        })
        .detach();
        input.update(cx, |input, cx| input.focus_all(window, cx));
        Self { document, page, input, wrong: false }
    }
}

impl Viewer {
    fn unlock(&mut self, password: String, window: &mut Window, cx: &mut Context<Self>) {
        let State::Locked(locked) = &mut self.state else { return };
        if !locked.document.unlock(&password) {
            locked.wrong = true;
            locked.input.update(cx, |input, cx| input.focus_all(window, cx));
            cx.notify();
            return;
        }
        let State::Locked(locked) = std::mem::replace(&mut self.state, State::Empty) else { return };
        let Locked { document, page, .. } = *locked;
        self.state = match self.start(document, Some(&password), page, window, cx) {
            Ok(open) => State::Open(Box::new(open)),
            Err(message) => State::Error(message),
        };
        window.focus(&self.focus);
        cx.notify();
    }

    pub(super) fn password_prompt(&self, locked: &Locked, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme;
        let input = locked.input.clone();
        div()
            .flex_1()
            .flex()
            .justify_center()
            .pt(px(120.))
            .child(
                div()
                    .w(px(320.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(div().text_color(theme.text).child(t("This document is protected by a password.")))
                    .child(locked.input.clone())
                    .when(locked.wrong, |column| {
                        column.child(div().text_sm().text_color(theme.hit_current).child(t("Wrong password")))
                    })
                    .child(
                        div()
                            .id("unlock")
                            .px(px(14.))
                            .py(px(6.))
                            .rounded(px(6.))
                            .bg(theme.accent)
                            .text_color(gpui::white())
                            .cursor_pointer()
                            .child(t("Unlock"))
                            .on_click(cx.listener(move |viewer, _, window, cx| {
                                let password = input.read(cx).text().to_owned();
                                viewer.unlock(password, window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }
}
