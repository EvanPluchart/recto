//! Text search: a bar under the toolbar, pages scanned in order on the render thread,
//! hits highlighted on the pages.

use std::ops::Range;

use gpui::{
    AnyElement, Context, Entity, IntoElement, ParentElement, SharedString, Styled, Window, div,
    prelude::*, px,
};
use recto_core::geom::{PageRect, Point};
use recto_render::PageText;

use super::{Open, State, Viewer};
use crate::i18n::t;
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::Theme;

/// Text requests kept on their way while scanning, so the render thread is never idle.
const PREFETCH: usize = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub page: usize,
    pub chars: Range<usize>,
}

pub struct Search {
    pub open: bool,
    input: Entity<TextInput>,
    query: String,
    /// In page order.
    hits: Vec<Hit>,
    current: Option<usize>,
    /// Next page to scan for the current query.
    scan_next: usize,
    /// Page the reader was on when the query changed: the first hit from there is selected.
    origin: usize,
}

impl Search {
    pub fn new(theme: Theme, window: &mut Window, cx: &mut Context<Viewer>) -> Self {
        let input = cx.new(|cx| TextInput::new(t("Find in document"), |_| true, theme, cx));
        cx.subscribe_in(&input, window, |viewer, _, event, window, cx| match event {
            TextInputEvent::Changed(query) => viewer.start_search(query.clone(), window, cx),
            TextInputEvent::Commit { shift, .. } => viewer.step_hit(!shift, window, cx),
            TextInputEvent::Cancel => viewer.close_search(window, cx),
        })
        .detach();
        Self { open: false, input, query: String::new(), hits: Vec::new(), current: None, scan_next: 0, origin: 0 }
    }

    /// Hits on page `page`, with their index in `hits`.
    fn on_page(&self, page: usize) -> impl Iterator<Item = (usize, &Hit)> {
        let start = self.hits.partition_point(|hit| hit.page < page);
        self.hits[start..].iter().enumerate().take_while(move |(_, hit)| hit.page == page).map(move |(i, hit)| (start + i, hit))
    }
}

impl Viewer {
    pub(super) fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.open = true;
        self.search.input.update(cx, |input, cx| input.focus_all(window, cx));
        cx.notify();
    }

    /// Opens the bar with `query` before the document is loaded (`--find`); the scan starts
    /// once the page count is known.
    pub(super) fn preset_search(&mut self, query: String, cx: &mut Context<Self>) {
        self.search.input.update(cx, |input, cx| input.set_text(query.clone(), cx));
        self.search.open = true;
        self.search.query = query;
    }

    fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.open = false;
        self.search.hits.clear();
        self.search.current = None;
        self.search.query.clear();
        window.focus(&self.focus);
        cx.notify();
    }

    fn start_search(&mut self, query: String, window: &mut Window, cx: &mut Context<Self>) {
        let origin = self.current_page();
        let search = &mut self.search;
        search.query = query;
        search.hits.clear();
        search.current = None;
        search.scan_next = 0;
        search.origin = origin;
        self.advance_search(window, cx);
    }

    /// Scans the pages whose text is known, in order, and asks for the next ones.
    pub(super) fn advance_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let State::Open(open) = &mut self.state else { return };
        let Some(count) = open.info.as_ref().map(|info| info.page_count()) else { return };
        let search = &mut self.search;
        if search.query.trim().is_empty() {
            cx.notify();
            return;
        }
        let mut reveal = None;
        while search.scan_next < count {
            let Some(text) = open.texts.get(&search.scan_next) else { break };
            for chars in text.find(&search.query) {
                search.hits.push(Hit { page: search.scan_next, chars });
                if search.current.is_none() && search.scan_next >= search.origin {
                    search.current = Some(search.hits.len() - 1);
                    reveal = search.current;
                }
            }
            search.scan_next += 1;
        }
        // Past the end with nothing after the reader's page: wrap to the first hit.
        if search.scan_next == count && search.current.is_none() && !search.hits.is_empty() {
            search.current = Some(0);
            reveal = Some(0);
        }
        for page in search.scan_next..(search.scan_next + PREFETCH).min(count) {
            if !open.texts.contains_key(&page) && open.texts_in_flight.insert(page) {
                open.renderer.text(page);
            }
        }
        if let Some(hit) = reveal {
            self.reveal_hit(hit, window, cx);
        }
        cx.notify();
    }

    /// Stores a page's text; a failed extraction counts as an empty page.
    pub(super) fn receive_text(
        &mut self,
        index: usize,
        result: anyhow::Result<PageText>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let State::Open(open) = &mut self.state else { return };
        open.texts_in_flight.remove(&index);
        open.texts.insert(index, result.unwrap_or_default());
        if self.search.open {
            self.advance_search(window, cx);
        } else {
            cx.notify();
        }
    }

    pub(super) fn step_hit(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.search.hits.len();
        if count == 0 {
            return;
        }
        let next = match self.search.current {
            Some(current) if forward => (current + 1) % count,
            Some(current) => (current + count - 1) % count,
            None => 0,
        };
        self.search.current = Some(next);
        self.reveal_hit(next, window, cx);
        cx.notify();
    }

    /// Scrolls hit `index` into view, a third of the way down, unless it is already visible.
    fn reveal_hit(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let viewport = self.viewport(window);
        let scroll = self.scroll_position();
        let Some(open) = self.open() else { return };
        let (Some(layout), Some(hit)) = (&open.layout, self.search.hits.get(index)) else { return };
        let Some(rect) = open.texts.get(&hit.page).and_then(|text| text.rects(hit.chars.clone()).into_iter().next()) else {
            return;
        };
        let Some(top_left) = layout.from_page(hit.page, Point { x: rect.x, y: rect.y }) else { return };
        let bottom = top_left.y + rect.height * layout.zoom;
        let max = layout.max_scroll(viewport);
        let mut position = scroll;
        if top_left.y < scroll.y || bottom > scroll.y + viewport.height {
            position.y = (top_left.y - viewport.height / 3.0).clamp(0.0, max.y);
        }
        let right = top_left.x + rect.width * layout.zoom;
        if top_left.x < scroll.x || right > scroll.x + viewport.width {
            position.x = (top_left.x - viewport.width / 4.0).clamp(0.0, max.x);
        }
        self.set_scroll_position(position);
        cx.notify();
    }

    /// Highlight boxes for page `page`, in page points, and whether each is the current hit.
    pub(super) fn highlights(&self, open: &Open, page: usize) -> Vec<(PageRect, bool)> {
        let Some(text) = open.texts.get(&page) else { return Vec::new() };
        self.search
            .on_page(page)
            .flat_map(|(index, hit)| {
                let current = self.search.current == Some(index);
                text.rects(hit.chars.clone()).into_iter().map(move |rect| (rect, current))
            })
            .collect()
    }

    pub(super) fn search_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.search.open {
            return None;
        }
        let theme = self.theme;
        let search = &self.search;
        let pages = self.open().and_then(|open| open.info.as_ref()).map_or(0, |info| info.page_count());
        let status: SharedString = if search.query.trim().is_empty() {
            SharedString::default()
        } else if search.hits.is_empty() && search.scan_next < pages {
            t("Searching…").into()
        } else if search.hits.is_empty() {
            t("No results").into()
        } else {
            let more = if search.scan_next < pages { "+" } else { "" };
            format!("{} / {}{more}", search.current.map_or(0, |i| i + 1), search.hits.len()).into()
        };
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .px(px(8.))
                .py(px(2.))
                .rounded(px(4.))
                .cursor_pointer()
                .hover(move |style| style.bg(theme.hover))
                .child(t(label))
        };
        Some(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_end()
                .gap(px(6.))
                .h(px(36.))
                .px(px(8.))
                .bg(theme.toolbar)
                .border_b_1()
                .border_color(theme.border)
                .text_color(theme.text)
                .text_sm()
                .child(div().w(px(240.)).child(search.input.clone()))
                .child(div().min_w(px(72.)).text_color(theme.muted).child(status))
                .child(button("search-previous", "Previous").on_click(cx.listener(|this, _, window, cx| this.step_hit(false, window, cx))))
                .child(button("search-next", "Next").on_click(cx.listener(|this, _, window, cx| this.step_hit(true, window, cx))))
                .child(button("search-close", "Close").on_click(cx.listener(|this, _, window, cx| this.close_search(window, cx))))
                .into_any_element(),
        )
    }
}
