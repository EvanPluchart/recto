//! The M0 view: one page of one document, rendered on the render thread.

use std::sync::Arc;

use gpui::{
    Context, IntoElement, ParentElement, Pixels, Render, RenderImage, SharedString, Size, Styled, Window,
    WindowAppearance, div, img, prelude::*, px, rgb, size,
};
use recto_core::{Document, PageId};
use recto_render::{Bitmap, PdfiumRenderer, RenderRequest, RenderThread, Response};

use crate::Args;
use crate::i18n::t;

enum State {
    Empty,
    Loading,
    Page { image: Arc<RenderImage>, size: Size<Pixels> },
    Error(SharedString),
}

pub struct Viewer {
    state: State,
    dark: bool,
    /// Device pixels per logical pixel when the page was requested.
    scale_factor: f32,
    generation: u64,
    _renderer: Option<RenderThread>,
}

impl Viewer {
    pub fn new(args: Args, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let dark = args.dark.unwrap_or(matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark));
        let scale_factor = window.scale_factor();
        let mut viewer = Self { state: State::Empty, dark, scale_factor, generation: 0, _renderer: None };
        if let Some(path) = &args.file {
            viewer.state = match viewer.start(path, &args, cx) {
                Ok(()) => State::Loading,
                Err(message) => State::Error(message),
            };
        }
        viewer
    }

    fn start(&mut self, path: &std::path::Path, args: &Args, cx: &mut Context<Self>) -> Result<(), SharedString> {
        let document = Document::open(path).map_err(|e| failure("Cannot open the document", e))?;
        let renderer = RenderThread::spawn(PdfiumRenderer::new).map_err(|e| failure("Cannot start the renderer", e))?;
        renderer.open(document.bytes(), None);

        let index = args.page.clamp(1, document.page_count()) - 1;
        self.generation = renderer.next_generation();
        renderer.request(RenderRequest {
            page: PageId { source: 0, index: index as u32 },
            index,
            scale: args.zoom / 100.0 * self.scale_factor,
            generation: self.generation,
        });

        let responses = renderer.responses().clone();
        cx.spawn(async move |this, cx| {
            while let Ok(response) = responses.recv_async().await {
                if this.update(cx, |viewer, cx| viewer.receive(response, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        self._renderer = Some(renderer);
        Ok(())
    }

    fn receive(&mut self, response: Response, cx: &mut Context<Self>) {
        match response {
            Response::Opened(Ok(_)) => return,
            Response::Opened(Err(e)) => self.state = State::Error(failure("Cannot open the document", e)),
            Response::Rendered { request, .. } if request.generation != self.generation => return,
            Response::Rendered { result: Err(e), .. } => self.state = State::Error(failure("Cannot render the page", e)),
            Response::Rendered { result: Ok(bitmap), .. } => {
                let logical = size(px(bitmap.width as f32 / self.scale_factor), px(bitmap.height as f32 / self.scale_factor));
                self.state = match to_render_image(bitmap) {
                    Some(image) => State::Page { image, size: logical },
                    None => State::Error(t("Cannot render the page").into()),
                };
            }
        }
        cx.notify();
    }
}

fn failure(what: &'static str, error: impl std::fmt::Display) -> SharedString {
    format!("{}: {error:#}", t(what)).into()
}

/// GPUI textures are BGRA.
fn to_render_image(bitmap: Bitmap) -> Option<Arc<RenderImage>> {
    let mut pixels = bitmap.pixels;
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let buffer = image::RgbaImage::from_raw(bitmap.width, bitmap.height, pixels)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

impl Render for Viewer {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let (background, foreground) = if self.dark { (rgb(0x1e1e1e), rgb(0xd4d4d4)) } else { (rgb(0xe8e8e8), rgb(0x303030)) };
        let canvas = div().id("canvas").size_full().bg(background).text_color(foreground).overflow_scroll();
        let message = |text: SharedString| div().size_full().flex().items_center().justify_center().p(px(24.)).child(text);
        match &self.state {
            State::Empty => canvas.child(message(t("No document. Usage: recto file.pdf").into())),
            State::Loading => canvas.child(message(t("Loading…").into())),
            State::Error(text) => canvas.child(message(text.clone())),
            // Top-aligned; horizontal auto margins centre the page while it fits and drop to zero when it overflows.
            State::Page { image, size } => canvas.flex().items_start().p(px(24.)).child(
                img(image.clone()).flex_none().mx_auto().w(size.width).h(size.height).shadow_md(),
            ),
        }
    }
}
