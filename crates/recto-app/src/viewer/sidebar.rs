//! The sidebar: page thumbnails or the document outline, both virtualised lists.

use gpui::{
    AnyElement, ClickEvent, Context, ExternalPaths, IntoElement, ParentElement, ScrollStrategy, SharedString, Styled,
    UniformListScrollHandle,
    Window, div, img, prelude::*, px, uniform_list,
};
use recto_core::PageId;
use recto_render::{Purpose, RenderRequest};

use super::{Open, State, Viewer, to_render_image};
use crate::i18n::t;

pub const WIDTH: f32 = 200.0;
/// Height of one thumbnail row, label included.
const ROW: f32 = 196.0;
/// Box a thumbnail must fit in, in logical pixels.
const THUMB_WIDTH: f32 = 140.0;
const THUMB_HEIGHT: f32 = 160.0;
/// Thumbnails kept beyond the visible rows before being released.
const KEEP_ROWS: usize = 40;
const OUTLINE_ROW: f32 = 28.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Thumbnails,
    Outline,
}

impl Tab {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "thumbnails" => Some(Tab::Thumbnails),
            "outline" => Some(Tab::Outline),
            _ => None,
        }
    }
}

pub struct Sidebar {
    pub tab: Option<Tab>,
    thumbnails: UniformListScrollHandle,
    outline: UniformListScrollHandle,
    /// Page the thumbnail list last followed, so it only moves when the reader changes page.
    followed: Option<usize>,
}

impl Sidebar {
    pub fn new(tab: Option<Tab>) -> Self {
        Self { tab, thumbnails: UniformListScrollHandle::new(), outline: UniformListScrollHandle::new(), followed: None }
    }

    /// After another document is opened, the thumbnail list follows its current page again.
    pub fn forget_followed(&mut self) {
        self.followed = None;
    }

    pub fn width(&self) -> f32 {
        if self.tab.is_some() { WIDTH } else { 0.0 }
    }
}

/// Size of page `size` scaled to fit the thumbnail box.
fn thumb_size(size: recto_core::geom::Size) -> (f32, f32) {
    let scale = (THUMB_WIDTH / size.width).min(THUMB_HEIGHT / size.height);
    (size.width * scale, size.height * scale)
}

impl Viewer {
    pub(super) fn toggle_sidebar(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.sidebar.tab = if self.sidebar.tab == Some(tab) { None } else { Some(tab) };
        cx.notify();
    }

    pub(super) fn sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tab = self.sidebar.tab?;
        let theme = self.theme;
        let current = self.current_page();
        let info = self.open()?.info.as_ref()?;
        let (page_count, outline_len) = (info.page_count(), info.outline.len());

        let tab_button = |id: &'static str, label: &'static str, this: Tab| {
            div()
                .id(id)
                .flex_1()
                .flex()
                .justify_center()
                .py(px(6.))
                .cursor_pointer()
                .border_b_2()
                .border_color(if tab == this { theme.accent } else { theme.toolbar })
                .text_color(if tab == this { theme.text } else { theme.muted })
                .child(t(label))
                .on_click(cx.listener(move |viewer, _, _, cx| {
                    viewer.sidebar.tab = Some(this);
                    cx.notify();
                }))
        };
        let header = div()
            .flex()
            .flex_none()
            .text_sm()
            .child(tab_button("tab-thumbnails", "Thumbnails", Tab::Thumbnails))
            .child(tab_button("tab-outline", "Outline", Tab::Outline));

        let body = match tab {
            Tab::Thumbnails => {
                if self.sidebar.followed != Some(current) {
                    self.sidebar.followed = Some(current);
                    self.sidebar.thumbnails.scroll_to_item(current, ScrollStrategy::Center);
                }
                self.release_far_thumbnails(window, cx);
                // One more row than pages: dropping on it moves pages to the end.
                uniform_list("thumbnails", page_count + 1, cx.processor(Self::thumbnail_rows))
                    .track_scroll(self.sidebar.thumbnails.clone())
                    .flex_1()
                    .into_any_element()
            }
            Tab::Outline if outline_len > 0 => {
                uniform_list("outline", outline_len, cx.processor(Self::outline_rows))
                    .track_scroll(self.sidebar.outline.clone())
                    .flex_1()
                    .into_any_element()
            }
            Tab::Outline => div()
                .p(px(16.))
                .text_sm()
                .text_color(theme.muted)
                .child(t("This document has no outline."))
                .into_any_element(),
        };

        Some(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .w(px(WIDTH))
                .h_full()
                .bg(theme.toolbar)
                .border_r_1()
                .border_color(theme.border)
                .text_color(theme.text)
                .child(header)
                .child(body)
                .into_any_element(),
        )
    }

    /// Releases thumbnails far from the rows on screen. Done here rather than in `thumbnail_rows`,
    /// which GPUI also calls with row 0 alone to measure the list.
    fn release_far_thumbnails(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let offset = self.sidebar.thumbnails.0.borrow().base_handle.offset();
        let top = (-f32::from(offset.y) / ROW).max(0.0) as usize;
        let rows = (f32::from(window.viewport_size().height) / ROW).ceil() as usize;
        let keep = top.saturating_sub(KEEP_ROWS)..top + rows + KEEP_ROWS;
        let State::Open(open) = &mut self.state else { return };
        // Row 0 stays: GPUI renders it on every layout to measure the list, so releasing it would loop.
        let far: Vec<usize> = open.thumbnails.keys().copied().filter(|&i| i != 0 && !keep.contains(&i)).collect();
        for index in far {
            if let Some(old) = open.thumbnails.remove(&index) {
                cx.drop_image(old, Some(window));
            }
        }
    }

    fn thumbnail_rows(
        &mut self,
        range: std::ops::Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = self.theme;
        let current = self.current_page();
        let scale = window.scale_factor();
        let State::Open(open) = &mut self.state else { return Vec::new() };
        let Some(info) = &open.info else { return Vec::new() };
        let sizes = &info.page_sizes;
        let count = sizes.len().min(open.page_states.len());
        let order: Vec<PageId> = open.page_states.iter().map(|page| page.id).collect();

        // Ask for the thumbnails that are neither cached nor on their way.
        for index in range.clone().filter(|&index| index < count) {
            if open.thumbnails.contains_key(&index) || !open.thumbnails_in_flight.insert(index) {
                continue;
            }
            let (width, _) = thumb_size(sizes[index]);
            open.renderer.request(RenderRequest {
                page: PageId { source: 0, index: index as u32 },
                index,
                scale: width / sizes[index].width * scale,
                generation: open.generation,
                purpose: Purpose::Thumbnail,
                tile: None,
            });
        }

        range
            .map(|index| {
                // The row after the last page only takes drops, to put pages at the end.
                let row = div()
                    .id(("thumbnail", index))
                    .w_full()
                    .h(px(ROW))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(6.))
                    .drag_over::<DraggedPages>(move |style, _, _, _| style.border_t_2().border_color(theme.accent))
                    .on_drop(cx.listener(move |viewer, dragged: &DraggedPages, window, cx| {
                        viewer.move_pages(dragged.pages.clone(), index, window, cx);
                    }))
                    .drag_over::<ExternalPaths>(move |style, _, _, _| style.border_t_2().border_color(theme.accent))
                    .on_drop(cx.listener(move |viewer, paths: &ExternalPaths, window, cx| {
                        // Merge here, rather than open the file in place of this one.
                        cx.stop_propagation();
                        viewer.insert_files(paths.paths(), index, window, cx);
                    }));
                if index >= count {
                    return row.child(div().text_xs().text_color(theme.muted).child(t("Drop here to put pages at the end"))).into_any_element();
                }
                let id = open.page_states[index].id;
                let selected = self.rail.contains(id);
                let (width, height) = thumb_size(sizes[index]);
                let mut page = div()
                    .w(px(width))
                    .h(px(height))
                    .bg(theme.placeholder)
                    .border_2()
                    .border_color(if selected || index == current { theme.accent } else { theme.border });
                if let Some(image) = open.thumbnails.get(&index) {
                    page = page.child(img(image.clone()).size_full());
                }
                let dragged = DraggedPages { pages: self.rail.dragged(&order, index) };
                row.cursor_pointer()
                    .when(selected, |row| row.bg(theme.selection))
                    .child(page)
                    .child(
                        div()
                            .text_xs()
                            .text_color(if index == current { theme.text } else { theme.muted })
                            .child(SharedString::from((index + 1).to_string())),
                    )
                    .on_drag(dragged, |dragged, _, _, cx| cx.new(|_| DragPreview { count: dragged.pages.len() }))
                    .on_click(cx.listener(move |viewer, event: &ClickEvent, window, cx| {
                        let modifiers = event.modifiers();
                        let order = viewer.page_order();
                        if modifiers.secondary() || modifiers.shift {
                            viewer.rail.click(&order, index, modifiers.secondary(), modifiers.shift);
                            cx.notify();
                        } else {
                            viewer.rail.click(&order, index, false, false);
                            viewer.go_to_page(index, window, cx);
                        }
                    }))
                    .into_any_element()
            })
            .collect()
    }

    fn outline_rows(
        &mut self,
        range: std::ops::Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = self.theme;
        let Some(info) = self.open().and_then(|open| open.info.as_ref()) else { return Vec::new() };
        info.outline[range.clone()]
            .iter()
            .zip(range)
            .map(|(entry, row)| {
                let page = entry.page;
                div()
                    .id(("outline", row))
                    .h(px(OUTLINE_ROW))
                    .flex()
                    .items_center()
                    .pl(px(12. + 14. * entry.depth.min(8) as f32))
                    .pr(px(8.))
                    .text_sm()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_color(if page.is_some() { theme.text } else { theme.muted })
                    .when(page.is_some(), |row| row.cursor_pointer().hover(move |style| style.bg(theme.hover)))
                    .child(SharedString::from(entry.title.clone()))
                    .on_click(cx.listener(move |viewer, _, window, cx| {
                        if let Some(page) = page {
                            viewer.go_to_page(page, window, cx);
                        }
                    }))
                    .into_any_element()
            })
            .collect()
    }
}

/// Stores a finished thumbnail, replacing any older one.
pub(super) fn store_thumbnail(open: &mut Open, index: usize, bitmap: recto_render::Bitmap, cx: &mut gpui::App) {
    open.thumbnails_in_flight.remove(&index);
    if let Some(image) = to_render_image(bitmap)
        && let Some(old) = open.thumbnails.insert(index, image)
    {
        cx.drop_image(old, None);
    }
}

/// Pages being dragged in the rail.
#[derive(Clone)]
pub struct DraggedPages {
    pages: Vec<PageId>,
}

/// What follows the pointer while dragging pages.
struct DragPreview {
    count: usize,
}

impl gpui::Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let label = if self.count == 1 { t("1 page").to_owned() } else { format!("{} {}", self.count, t("pages")) };
        div().px(px(8.)).py(px(4.)).rounded(px(4.)).bg(gpui::rgb(0x2f6fdf)).text_color(gpui::white()).text_sm().child(label)
    }
}
