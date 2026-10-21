//! The document view: every page in one continuous, virtualised scroll, rendered on the render thread.
//! Only pages near the viewport get a bitmap; the rest are empty boxes of the right size.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, IntoElement, KeyBinding, ParentElement, Render, RenderImage,
    ScrollHandle, ScrollWheelEvent, SharedString, Styled, Window, WindowAppearance, actions, div, img, point,
    prelude::*, px,
};
use recto_core::geom::{Point, Size};
use recto_core::layout::{self, PageLayout};
use recto_core::{Document, PageId};
use recto_render::{Bitmap, DocInfo, PdfiumRenderer, Purpose, RenderRequest, RenderThread, Response};

use crate::Args;
use crate::i18n::t;
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::Theme;

actions!(
    viewer,
    [ZoomIn, ZoomOut, ActualSize, FitWidth, FitPage, GoToPage, LineDown, LineUp, ScreenDown, ScreenUp, FirstPage, LastPage]
);

const TOOLBAR_HEIGHT: f32 = 40.0;
/// Arrow-key scroll step, in logical pixels.
const LINE: f32 = 48.0;
/// Pages rendered beyond the viewport, in viewport heights, so scrolling finds them ready.
const PRERENDER: f32 = 1.0;
/// Bitmaps kept around the visible pages before being released.
const KEEP_PAGES: usize = 6;
const MIN_ZOOM: f32 = 0.1;
/// Until tiles land, a whole page must fit in one bitmap.
const MAX_ZOOM: f32 = 8.0;
const ZOOM_STEPS: [f32; 19] =
    [0.1, 0.25, 0.33, 0.5, 0.67, 0.75, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZoomMode {
    FitWidth,
    FitPage,
    /// Logical pixels per point (1.0 = 100 %).
    Fixed(f32),
}

struct Cached {
    image: Arc<RenderImage>,
    /// Render scale (device pixels per point) the bitmap was made at.
    scale: f32,
}

struct Open {
    _document: Document,
    renderer: RenderThread,
    info: Option<DocInfo>,
    layout: Option<PageLayout>,
    pages: HashMap<usize, Cached>,
    in_flight: HashSet<(usize, u32)>,
    /// Device pixels per point for the current zoom; a change starts a new render generation.
    render_scale: f32,
    generation: u64,
    /// Page to scroll to once the first layout exists (from `--page`).
    pending_page: Option<usize>,
}

enum State {
    Empty,
    Error(SharedString),
    Open(Box<Open>),
}

pub struct Viewer {
    state: State,
    theme: Theme,
    zoom: ZoomMode,
    scroll: ScrollHandle,
    focus: FocusHandle,
    page_input: Entity<TextInput>,
}

impl Viewer {
    pub fn new(args: Args, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let dark = args.dark.unwrap_or(matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark));
        let theme = Theme::new(dark);
        let zoom = args.zoom.map_or(ZoomMode::FitWidth, |pct| ZoomMode::Fixed((pct / 100.0).clamp(MIN_ZOOM, MAX_ZOOM)));
        let page_input = cx.new(|cx| TextInput::new("", |c| c.is_ascii_digit(), theme, cx));
        cx.subscribe_in(&page_input, window, |this, _, event, window, cx| {
            if let TextInputEvent::Commit(text) = event
                && let Ok(page) = text.trim().parse::<usize>()
            {
                this.go_to_page(page.saturating_sub(1), window, cx);
            }
            window.focus(&this.focus);
        })
        .detach();
        let mut viewer =
            Self { state: State::Empty, theme, zoom, scroll: ScrollHandle::new(), focus: cx.focus_handle(), page_input };
        if let Some(path) = &args.file {
            viewer.state = match viewer.start(path, args.page, cx) {
                Ok(open) => State::Open(Box::new(open)),
                Err(message) => State::Error(message),
            };
        }
        window.focus(&viewer.focus);
        viewer
    }

    fn start(&mut self, path: &std::path::Path, page: usize, cx: &mut Context<Self>) -> Result<Open, SharedString> {
        let document = Document::open(path).map_err(|e| failure("Cannot open the document", e))?;
        let renderer = RenderThread::spawn(PdfiumRenderer::new).map_err(|e| failure("Cannot start the renderer", e))?;
        renderer.open(document.bytes(), None);
        let responses = renderer.responses().clone();
        cx.spawn(async move |this, cx| {
            while let Ok(response) = responses.recv_async().await {
                if this.update(cx, |viewer, cx| viewer.receive(response, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        Ok(Open {
            _document: document,
            renderer,
            info: None,
            layout: None,
            pages: HashMap::new(),
            in_flight: HashSet::new(),
            render_scale: 0.0,
            generation: 0,
            pending_page: Some(page.saturating_sub(1)),
        })
    }

    fn receive(&mut self, response: Response, cx: &mut Context<Self>) {
        let State::Open(open) = &mut self.state else { return };
        match response {
            Response::Opened(Ok(info)) => {
                open.pending_page = open.pending_page.map(|page| page.min(info.page_count().saturating_sub(1)));
                open.info = Some(info);
            }
            Response::Opened(Err(e)) => self.state = State::Error(failure("Cannot open the document", e)),
            Response::Rendered { request, result } => {
                open.in_flight.remove(&(request.index, scale_key(request.scale)));
                match result {
                    Ok(bitmap) => {
                        // A bitmap from a zoom level we have left only fills a page that has nothing yet.
                        let current = scale_key(request.scale) == scale_key(open.render_scale);
                        if (current || !open.pages.contains_key(&request.index))
                            && let Some(image) = to_render_image(bitmap)
                            && let Some(old) = open.pages.insert(request.index, Cached { image, scale: request.scale })
                        {
                            cx.drop_image(old.image, None);
                        }
                    }
                    Err(e) => self.state = State::Error(failure("Cannot render the page", e)),
                }
            }
        }
        cx.notify();
    }

    /// Viewport of the page area, in logical pixels. Before the first paint, derived from the window.
    fn viewport(&self, window: &Window) -> Size {
        let bounds = self.scroll.bounds();
        if bounds.size.width > px(0.) && bounds.size.height > px(0.) {
            Size { width: f32::from(bounds.size.width), height: f32::from(bounds.size.height) }
        } else {
            let size = window.viewport_size();
            Size { width: f32::from(size.width), height: (f32::from(size.height) - TOOLBAR_HEIGHT).max(1.0) }
        }
    }

    /// Scroll position as a positive offset into the content.
    fn scroll_position(&self) -> Point {
        let offset = self.scroll.offset();
        Point { x: -f32::from(offset.x), y: -f32::from(offset.y) }
    }

    fn set_scroll_position(&self, position: Point) {
        self.scroll.set_offset(point(px(-position.x), px(-position.y)));
    }

    fn open(&self) -> Option<&Open> {
        match &self.state {
            State::Open(open) => Some(open),
            _ => None,
        }
    }

    /// Index of the page the reader is on: the one under the upper quarter of the viewport.
    fn current_page(&self) -> usize {
        let probe = self.scroll_position().y + f32::from(self.scroll.bounds().size.height) / 4.0;
        self.open().and_then(|open| open.layout.as_ref()).map_or(0, |l| l.page_at(probe))
    }

    /// Logical pixels per point for the current mode.
    fn effective_zoom(&self, info: &DocInfo, viewport: Size) -> f32 {
        let zoom = match self.zoom {
            ZoomMode::Fixed(zoom) => zoom,
            ZoomMode::FitWidth => layout::fit_width(&info.page_sizes, viewport),
            ZoomMode::FitPage => {
                info.page_sizes.get(self.current_page()).map_or(1.0, |&page| layout::fit_page(page, viewport))
            }
        };
        zoom.clamp(MIN_ZOOM, MAX_ZOOM)
    }

    fn current_zoom(&self) -> f32 {
        self.open().and_then(|open| open.layout.as_ref()).map_or(1.0, |l| l.zoom)
    }

    /// Changes the zoom mode, keeping the point under `anchor` (viewport position) still.
    fn set_zoom(&mut self, mode: ZoomMode, anchor: Option<Point>, window: &mut Window, cx: &mut Context<Self>) {
        self.zoom = match mode {
            ZoomMode::Fixed(zoom) => ZoomMode::Fixed(zoom.clamp(MIN_ZOOM, MAX_ZOOM)),
            other => other,
        };
        let viewport = self.viewport(window);
        let anchor = anchor.unwrap_or(Point { x: viewport.width / 2.0, y: viewport.height / 2.0 });
        let scroll = self.scroll_position();
        let Some(open) = self.open() else { return };
        let (Some(info), Some(previous)) = (&open.info, &open.layout) else { return };
        let next = PageLayout::new(&info.page_sizes, self.effective_zoom(info, viewport), viewport.width);
        let position = next.rescroll(previous, scroll, anchor, viewport);
        if let State::Open(open) = &mut self.state {
            open.layout = Some(next);
        }
        self.set_scroll_position(position);
        cx.notify();
    }

    /// Scrolls so page `index` (clamped) starts at the top of the viewport.
    fn go_to_page(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let viewport = self.viewport(window);
        let Some(layout) = self.open().and_then(|open| open.layout.as_ref()) else { return };
        let position = layout.scroll_to_page(index.min(layout.boxes.len().saturating_sub(1)), viewport);
        self.set_scroll_position(Point { x: self.scroll_position().x, y: position.y });
        cx.notify();
    }

    /// Scrolls vertically by `delta` logical pixels, within the content.
    fn scroll_by(&mut self, delta: f32, window: &mut Window, cx: &mut Context<Self>) {
        let viewport = self.viewport(window);
        let Some(layout) = self.open().and_then(|open| open.layout.as_ref()) else { return };
        let max = layout.max_scroll(viewport);
        let position = self.scroll_position();
        self.set_scroll_position(Point { x: position.x, y: (position.y + delta).clamp(0.0, max.y) });
        cx.notify();
    }

    fn step_zoom(&mut self, up: bool, window: &mut Window, cx: &mut Context<Self>) {
        let zoom = self.current_zoom();
        let next = if up {
            ZOOM_STEPS.iter().copied().find(|&z| z > zoom * 1.01).unwrap_or(MAX_ZOOM)
        } else {
            ZOOM_STEPS.iter().rev().copied().find(|&z| z < zoom / 1.01).unwrap_or(MIN_ZOOM)
        };
        self.set_zoom(ZoomMode::Fixed(next), None, window, cx);
    }

    /// Ctrl (Cmd on macOS) + wheel zooms around the pointer.
    fn on_wheel(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !event.modifiers.secondary() {
            return;
        }
        cx.stop_propagation();
        let delta = f32::from(event.delta.pixel_delta(window.line_height()).y);
        let factor = (1.0 + delta / 400.0).clamp(0.5, 2.0);
        let origin = self.scroll.bounds().origin;
        let anchor = Point { x: f32::from(event.position.x - origin.x), y: f32::from(event.position.y - origin.y) };
        self.set_zoom(ZoomMode::Fixed(self.current_zoom() * factor), Some(anchor), window, cx);
    }

    /// Lays out the pages for the current viewport and asks for the bitmaps that are missing.
    fn update_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let viewport = self.viewport(window);
        let scale_factor = window.scale_factor();
        let Some(open) = self.open() else { return };
        let Some(info) = &open.info else { return };
        let zoom = self.effective_zoom(info, viewport);
        let stale = open.layout.as_ref().is_none_or(|l| l.zoom != zoom || (l.viewport_width - viewport.width).abs() > 0.5);
        let fresh = stale.then(|| PageLayout::new(&info.page_sizes, zoom, viewport.width));
        let scroll = self.scroll_position();

        let State::Open(open) = &mut self.state else { return };
        let mut position = scroll;
        if let Some(fresh) = fresh {
            // A refit (window resized, first real viewport) keeps the content at the top-left corner still.
            if let Some(previous) = &open.layout {
                position = fresh.rescroll(previous, scroll, Point { x: 0.0, y: 0.0 }, viewport);
            }
            open.layout = Some(fresh);
        }
        let Some(layout) = &open.layout else { return };
        if let Some(page) = open.pending_page.take() {
            position = layout.scroll_to_page(page, viewport);
        }
        if position != scroll {
            self.scroll.set_offset(point(px(-position.x), px(-position.y)));
        }
        let top = position.y;

        let render_scale = layout.zoom * scale_factor;
        if scale_key(render_scale) != scale_key(open.render_scale) {
            open.render_scale = render_scale;
            open.generation = open.renderer.next_generation();
            open.in_flight.clear();
        }

        // Visible pages first, then the ones around them, nearest first.
        let visible = layout.visible(top, viewport.height);
        let wanted = layout.visible(top - PRERENDER * viewport.height, viewport.height * (1.0 + 2.0 * PRERENDER));
        let distance = |i: usize| if i < visible.start { visible.start - i } else { i + 1 - visible.end };
        let mut nearby: Vec<usize> = wanted.filter(|i| !visible.contains(i)).collect();
        nearby.sort_by_key(|&i| distance(i));
        for index in visible.clone().chain(nearby) {
            let key = (index, scale_key(render_scale));
            let ready = open.pages.get(&index).is_some_and(|c| scale_key(c.scale) == key.1);
            if !ready && open.in_flight.insert(key) {
                open.renderer.request(RenderRequest {
                    page: PageId { source: 0, index: index as u32 },
                    index,
                    scale: render_scale,
                    generation: open.generation,
                    purpose: Purpose::Page,
                });
            }
        }

        // Release bitmaps far from the viewport, from the GPU atlas too.
        let keep = visible.start.saturating_sub(KEEP_PAGES)..visible.end + KEEP_PAGES;
        let far: Vec<usize> = open.pages.keys().copied().filter(|i| !keep.contains(i)).collect();
        for index in far {
            if let Some(old) = open.pages.remove(&index) {
                cx.drop_image(old.image, Some(window));
            }
        }
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let (page_count, zoom_label) = match self.open().and_then(|open| open.info.as_ref().zip(open.layout.as_ref())) {
            Some((info, layout)) => (Some(info.page_count()), format!("{:.0} %", layout.zoom * 100.0)),
            None => (None, String::new()),
        };
        let page_field = page_count.map(|count| {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(8.))
                .text_color(theme.muted)
                .child(t("Page"))
                .child(div().w(px(52.)).text_color(theme.text).child(self.page_input.clone()))
                .child(format!("/ {count}"))
        });
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .px(px(10.))
                .py(px(4.))
                .rounded(px(4.))
                .cursor_pointer()
                .hover(move |style| style.bg(theme.hover))
                .child(t(label))
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .h(px(TOOLBAR_HEIGHT))
            .px(px(8.))
            .bg(theme.toolbar)
            .border_b_1()
            .border_color(theme.border)
            .text_color(theme.text)
            .text_sm()
            .child(button("zoom-out", "−").on_click(cx.listener(|this, _, window, cx| this.step_zoom(false, window, cx))))
            .child(div().w(px(56.)).flex().justify_center().child(zoom_label))
            .child(button("zoom-in", "+").on_click(cx.listener(|this, _, window, cx| this.step_zoom(true, window, cx))))
            .child(
                button("fit-width", "Fit width")
                    .on_click(cx.listener(|this, _, window, cx| this.set_zoom(ZoomMode::FitWidth, None, window, cx))),
            )
            .child(
                button("fit-page", "Fit page")
                    .on_click(cx.listener(|this, _, window, cx| this.set_zoom(ZoomMode::FitPage, None, window, cx))),
            )
            .child(div().flex_1())
            .children(page_field)
    }

    fn pages(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme;
        let message = |text: SharedString| {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p(px(24.))
                .text_color(theme.muted)
                .child(text)
                .into_any_element()
        };
        let open = match &self.state {
            State::Empty => return message(t("No document. Usage: recto file.pdf").into()),
            State::Error(text) => return message(text.clone()),
            State::Open(open) => open,
        };
        let Some(layout) = &open.layout else { return message(t("Loading…").into()) };

        let visible = layout.visible(self.scroll_position().y, self.viewport(window).height);
        let mut content = div()
            .relative()
            .w(px(layout.content.width))
            .h(px(layout.content.height))
            .on_scroll_wheel(cx.listener(Self::on_wheel));
        for index in visible {
            let page = layout.boxes[index];
            let frame = div()
                .absolute()
                .left(px(page.x))
                .top(px(page.y))
                .w(px(page.width))
                .h(px(page.height))
                .bg(theme.placeholder)
                .shadow_md();
            // A bitmap from another zoom level is stretched until the right one arrives.
            content = content.child(match open.pages.get(&index) {
                Some(cached) => frame.child(img(cached.image.clone()).size_full()),
                None => frame,
            });
        }
        div()
            .id("pages")
            .flex_1()
            .min_h_0()
            .overflow_scroll()
            .track_scroll(&self.scroll)
            .bg(theme.canvas)
            .child(content)
            .into_any_element()
    }
}

impl Focusable for Viewer {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Viewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.update_layout(window, cx);
        // The page field follows the scroll, except while the reader is typing in it.
        if !self.page_input.focus_handle(cx).is_focused(window) {
            let current = (self.current_page() + 1).to_string();
            self.page_input.update(cx, |input, cx| input.set_text(current, cx));
        }
        div()
            .key_context("Viewer")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &ZoomIn, window, cx| this.step_zoom(true, window, cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, window, cx| this.step_zoom(false, window, cx)))
            .on_action(cx.listener(|this, _: &ActualSize, window, cx| this.set_zoom(ZoomMode::Fixed(1.0), None, window, cx)))
            .on_action(cx.listener(|this, _: &FitWidth, window, cx| this.set_zoom(ZoomMode::FitWidth, None, window, cx)))
            .on_action(cx.listener(|this, _: &FitPage, window, cx| this.set_zoom(ZoomMode::FitPage, None, window, cx)))
            .on_action(cx.listener(|this, _: &GoToPage, window, cx| {
                this.page_input.update(cx, |input, cx| input.focus_all(window, cx));
            }))
            .on_action(cx.listener(|this, _: &LineDown, window, cx| this.scroll_by(LINE, window, cx)))
            .on_action(cx.listener(|this, _: &LineUp, window, cx| this.scroll_by(-LINE, window, cx)))
            .on_action(cx.listener(|this, _: &ScreenDown, window, cx| {
                let step = this.viewport(window).height * 0.9;
                this.scroll_by(step, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ScreenUp, window, cx| {
                let step = this.viewport(window).height * 0.9;
                this.scroll_by(-step, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FirstPage, window, cx| this.go_to_page(0, window, cx)))
            .on_action(cx.listener(|this, _: &LastPage, window, cx| this.go_to_page(usize::MAX, window, cx)))
            .size_full()
            .flex()
            .flex_col()
            .bg(self.theme.canvas)
            .child(self.toolbar(cx))
            .child(self.pages(window, cx))
    }
}

pub fn bind_keys(cx: &mut App) {
    // Plain keys must not fire while a text field has focus.
    const PAGES: Option<&str> = Some("Viewer && !TextInput");
    cx.bind_keys([
        KeyBinding::new("secondary-=", ZoomIn, Some("Viewer")),
        KeyBinding::new("secondary-+", ZoomIn, Some("Viewer")),
        KeyBinding::new("secondary--", ZoomOut, Some("Viewer")),
        KeyBinding::new("secondary-0", ActualSize, Some("Viewer")),
        KeyBinding::new("secondary-1", FitWidth, Some("Viewer")),
        KeyBinding::new("secondary-2", FitPage, Some("Viewer")),
        KeyBinding::new("secondary-g", GoToPage, Some("Viewer")),
        KeyBinding::new("down", LineDown, PAGES),
        KeyBinding::new("up", LineUp, PAGES),
        KeyBinding::new("pagedown", ScreenDown, PAGES),
        KeyBinding::new("space", ScreenDown, PAGES),
        KeyBinding::new("pageup", ScreenUp, PAGES),
        KeyBinding::new("shift-space", ScreenUp, PAGES),
        KeyBinding::new("home", FirstPage, PAGES),
        KeyBinding::new("end", LastPage, PAGES),
    ]);
}

/// Render scales compared at 1/1000 precision, so float noise does not trigger re-renders.
fn scale_key(scale: f32) -> u32 {
    (scale * 1000.0).round() as u32
}

fn failure(what: &'static str, error: impl std::fmt::Display) -> SharedString {
    format!("{}: {error:#}", t(what)).into()
}

/// `Bitmap` is BGRA, the layout GPUI textures expect, so the buffer is handed over as is.
fn to_render_image(bitmap: Bitmap) -> Option<Arc<RenderImage>> {
    let buffer = image::RgbaImage::from_raw(bitmap.width, bitmap.height, bitmap.pixels)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}
