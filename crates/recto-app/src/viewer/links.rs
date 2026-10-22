//! Following links with the Select tool: a page link scrolls to its page, a web link opens in
//! the browser after the reader agrees.

use gpui::{Context, MouseDownEvent, PromptLevel, Window};
use recto_core::annot::LinkTarget;

use super::Viewer;
use crate::i18n::t;

impl Viewer {
    /// Follows the link under the pointer; returns whether there was one.
    pub(super) fn follow_link(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some((page, at)) = self.page_point(event.position) else { return false };
        let Some(open) = self.open() else { return false };
        let (Some(geometry), Some(state)) = (open.geometry.get(page), open.page_states.get(page)) else { return false };
        let point = geometry.to_user(at);
        let links = open.document.links(state.id).unwrap_or_default();
        let Some((_, target)) = links.into_iter().rev().find(|(rect, _)| {
            point.x >= rect.origin.x && point.x <= rect.origin.x + rect.width && point.y >= rect.origin.y && point.y <= rect.origin.y + rect.height
        }) else {
            return false;
        };
        match target {
            LinkTarget::Page(id) => {
                if let Some(index) = open.page_states.iter().position(|state| state.id == id) {
                    self.go_to_page(index, window, cx);
                }
            }
            LinkTarget::Uri(uri) => {
                // A PDF can point anywhere: say where before going.
                let answer = window.prompt(PromptLevel::Info, t("Open this link in your browser?"), Some(&uri), &[t("Open"), t("Cancel")], cx);
                cx.spawn_in(window, async move |_, cx| {
                    if answer.await == Ok(0) {
                        cx.update(|_, cx| cx.open_url(&uri)).ok();
                    }
                })
                .detach();
            }
        }
        true
    }
}
