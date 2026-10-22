//! Annotation tools: the pointer turns into annotations in user space through each page's
//! geometry. Shapes and freehand show a preview while dragging; text opens an inline editor;
//! images and signatures are placed with a click.

use std::sync::Arc;

use gpui::{
    AnyElement, Context, Entity, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement, PathBuilder,
    PathPromptOptions, Pixels, Styled, Window, canvas, div, point, prelude::*, px,
};
use recto_core::annot::{Annotation, Color, Font, Kind, Outline, Raster, text_width};
use recto_core::geom::{PageRect, Point};
use recto_core::op::Op;

use super::{Viewer, failure};
use crate::i18n::t;
use crate::signatures::{DRAWN_ASPECT, Signature};
use crate::text_input::{TextInput, TextInputEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    /// Select text, fill forms.
    Select,
    Text,
    Highlight,
    Underline,
    StrikeOut,
    Rectangle,
    Ellipse,
    Line,
    Arrow,
    Freehand,
    Image,
    Signature,
    /// Removes what lies under a dragged area.
    Redact,
    /// Erases what lies under a dragged area (white).
    Erase,
    /// Deletes the image under a click.
    DeleteImage,
    /// Shows only a dragged area of the page (or of the selected pages).
    Crop,
    /// Changes the text of a line (beta).
    EditText,
}

impl Tool {
    pub const ALL: [Tool; 17] = [
        Tool::Select,
        Tool::Text,
        Tool::Highlight,
        Tool::Underline,
        Tool::StrikeOut,
        Tool::Rectangle,
        Tool::Ellipse,
        Tool::Line,
        Tool::Arrow,
        Tool::Freehand,
        Tool::Image,
        Tool::Signature,
        Tool::Redact,
        Tool::Erase,
        Tool::DeleteImage,
        Tool::Crop,
        Tool::EditText,
    ];

    /// Name used by `--tool=NAME`.
    pub fn name(self) -> &'static str {
        match self {
            Tool::Select => "select",
            Tool::Text => "text",
            Tool::Highlight => "highlight",
            Tool::Underline => "underline",
            Tool::StrikeOut => "strikeout",
            Tool::Rectangle => "rectangle",
            Tool::Ellipse => "ellipse",
            Tool::Line => "line",
            Tool::Arrow => "arrow",
            Tool::Freehand => "freehand",
            Tool::Image => "image",
            Tool::Signature => "signature",
            Tool::Redact => "redact",
            Tool::Erase => "erase",
            Tool::DeleteImage => "deleteimage",
            Tool::Crop => "crop",
            Tool::EditText => "edit",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.name() == name)
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Text => "Text",
            Tool::Highlight => "Highlight",
            Tool::Underline => "Underline",
            Tool::StrikeOut => "Strike out",
            Tool::Rectangle => "Rectangle",
            Tool::Ellipse => "Ellipse",
            Tool::Line => "Line",
            Tool::Arrow => "Arrow",
            Tool::Freehand => "Draw",
            Tool::Image => "Image",
            Tool::Signature => "Sign",
            Tool::Redact => "Redact",
            Tool::Erase => "Erase",
            Tool::DeleteImage => "Delete an image",
            Tool::Crop => "Crop",
            Tool::EditText => "Edit text (beta)",
        }
    }

    /// Tools that mark text work on a text selection.
    pub fn selects_text(self) -> bool {
        matches!(self, Tool::Select | Tool::Highlight | Tool::Underline | Tool::StrikeOut)
    }
}

pub const COLORS: [Color; 5] = [Color::BLACK, Color::RED, Color::BLUE, Color::GREEN, Color::YELLOW];
pub const WIDTHS: [f32; 3] = [1.0, 2.5, 5.0];

/// Options of the annotation being made.
#[derive(Clone, Copy, Debug)]
pub struct Inspector {
    pub color: Color,
    pub width: f32,
    pub size: f32,
}

impl Default for Inspector {
    fn default() -> Self {
        Self { color: Color::RED, width: 2.5, size: 14.0 }
    }
}

/// A shape or stroke being dragged, in displayed page points.
pub struct Sketch {
    pub page: usize,
    pub points: Vec<Point>,
}

/// A text box being typed.
pub struct TextDraft {
    page: usize,
    at: Point,
    input: Entity<TextInput>,
}

fn rgb(color: Color) -> gpui::Rgba {
    gpui::Rgba { r: color.r, g: color.g, b: color.b, a: 1.0 }
}

/// The rectangle spanned by two corners.
fn span(a: Point, b: Point) -> PageRect {
    PageRect { x: a.x.min(b.x), y: a.y.min(b.y), width: (a.x - b.x).abs(), height: (a.y - b.y).abs() }
}

/// Decodes an image file to a raster, scaled down to at most 2000 px on its longest side.
pub fn load_raster(path: &std::path::Path) -> Result<Raster, String> {
    let image = image::open(path).map_err(|error| error.to_string())?;
    let image = if image.width().max(image.height()) > 2000 { image.thumbnail(2000, 2000) } else { image };
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    let mut alpha = Vec::with_capacity((width * height) as usize);
    for pixel in rgba.pixels() {
        rgb.extend_from_slice(&pixel.0[..3]);
        alpha.push(pixel.0[3]);
    }
    let opaque = alpha.iter().all(|&a| a == 255);
    Ok(Raster { width, height, rgb, alpha: (!opaque).then_some(alpha) })
}

impl Viewer {
    pub(super) fn set_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        self.tool = tool;
        self.sketch = None;
        self.text_draft = None;
        self.edit_draft = None;
        self.picked = None;
        if tool != Tool::Signature {
            self.signature = None;
        }
        cx.notify();
    }

    /// A window position as (displayed page index, point on that page), if it is on a page.
    pub(super) fn page_point(&self, position: gpui::Point<Pixels>) -> Option<(usize, Point)> {
        let layout = self.open()?.layout.as_ref()?;
        let origin = self.scroll.bounds().origin;
        let scroll = self.scroll_position();
        let content = Point { x: f32::from(position.x - origin.x) + scroll.x, y: f32::from(position.y - origin.y) + scroll.y };
        let (index, offset) = layout.to_page(content)?;
        let size = self.open()?.info.as_ref()?.page_sizes.get(index).copied()?;
        let inside = offset.x >= 0.0 && offset.y >= 0.0 && offset.x <= size.width && offset.y <= size.height;
        inside.then_some((index, offset))
    }

    /// Records an annotation on displayed page `page`.
    fn annotate(&mut self, page: usize, kind: Kind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.open().and_then(|open| open.page_states.get(page)).map(|state| state.id) else { return };
        let annotation = Annotation { kind, color: self.inspector.color, width: self.inspector.width };
        self.edit(Op::AddAnnotation { page: id, annotation }, window, cx);
    }

    fn geometry(&self, page: usize) -> Option<recto_core::geom::PageGeometry> {
        self.open()?.geometry.get(page).copied()
    }

    pub(super) fn on_pointer_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.pick_at(event, cx) {
            return;
        }
        if self.tool.selects_text() {
            return self.on_select_start(event, window, cx);
        }
        window.focus(&self.focus);
        let Some((page, at)) = self.page_point(event.position) else { return };
        match self.tool {
            Tool::Text => self.start_text(page, at, window, cx),
            Tool::DeleteImage => self.delete_image_at(page, at, window, cx),
            Tool::EditText => self.start_text_edit(page, at, window, cx),
            Tool::Image => self.prompt_image(page, at, window, cx),
            Tool::Signature => match self.signature.clone() {
                Some(signature) => self.place_signature(&signature, page, at, window, cx),
                None => self.open_signature_dialog(window, cx),
            },
            _ => {
                self.sketch = Some(Sketch { page, points: vec![at, at] });
                cx.notify();
            }
        }
    }

    pub(super) fn on_pointer_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.pick_move(event, cx) {
            return;
        }
        let Some(sketch) = &self.sketch else { return self.on_select_move(event, window, cx) };
        if event.pressed_button != Some(MouseButton::Left) {
            return self.finish_sketch(window, cx);
        }
        let page = sketch.page;
        // Points are kept on the page the drag started on, clamped to it.
        let Some(layout) = self.open().and_then(|open| open.layout.as_ref()) else { return };
        let Some(size) = self.open().and_then(|open| open.info.as_ref()).and_then(|info| info.page_sizes.get(page).copied()) else {
            return;
        };
        let origin = self.scroll.bounds().origin;
        let scroll = self.scroll_position();
        let content = Point { x: f32::from(event.position.x - origin.x) + scroll.x, y: f32::from(event.position.y - origin.y) + scroll.y };
        let Some(page_box) = layout.boxes.get(page) else { return };
        let at = Point {
            x: ((content.x - page_box.x) / layout.zoom).clamp(0.0, size.width),
            y: ((content.y - page_box.y) / layout.zoom).clamp(0.0, size.height),
        };
        let freehand = self.tool == Tool::Freehand;
        if let Some(sketch) = &mut self.sketch {
            if freehand {
                let last = sketch.points.last().copied().unwrap_or(at);
                if (last.x - at.x).abs() + (last.y - at.y).abs() >= 1.0 {
                    sketch.points.push(at);
                }
            } else if let Some(end) = sketch.points.last_mut() {
                *end = at;
            }
            cx.notify();
        }
    }

    pub(super) fn on_pointer_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pick_release(window, cx) {
            return;
        }
        if self.sketch.is_some() {
            return self.finish_sketch(window, cx);
        }
        self.end_selection();
        if matches!(self.tool, Tool::Highlight | Tool::Underline | Tool::StrikeOut) {
            self.mark_selection(window, cx);
        }
    }

    fn finish_sketch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Sketch { page, points }) = self.sketch.take() else { return };
        let Some(geometry) = self.geometry(page) else { return };
        let (Some(&first), Some(&last)) = (points.first(), points.last()) else { return };
        let area = span(first, last);
        let tiny = area.width < 3.0 && area.height < 3.0;
        if !tiny && matches!(self.tool, Tool::Redact | Tool::Erase | Tool::Crop) {
            let area = geometry.rect_to_user(area);
            let Some(id) = self.open().and_then(|open| open.page_states.get(page)).map(|state| state.id) else { return };
            let op = match self.tool {
                Tool::Redact => Op::Redact { page: id, area },
                Tool::Erase => Op::Erase { page: id, area },
                _ => {
                    // The selected pages when the area is drawn on one of them, else this page.
                    let selected = self.target_pages();
                    Op::CropPages { pages: if selected.contains(&id) { selected } else { vec![id] }, area }
                }
            };
            return self.edit(op, window, cx);
        }
        let kind = match self.tool {
            Tool::Rectangle if !tiny => Kind::Rectangle { rect: geometry.rect_to_user(area) },
            Tool::Ellipse if !tiny => Kind::Ellipse { rect: geometry.rect_to_user(area) },
            Tool::Line | Tool::Arrow if !tiny => {
                Kind::Line { from: geometry.to_user(first), to: geometry.to_user(last), arrow: self.tool == Tool::Arrow }
            }
            Tool::Freehand if points.len() > 1 => {
                Kind::Ink { strokes: vec![points.iter().map(|&p| geometry.to_user(p)).collect()] }
            }
            _ => {
                cx.notify();
                return;
            }
        };
        self.annotate(page, kind, window, cx);
    }

    /// Turns the text selection into highlight, underline or strike-out annotations.
    fn mark_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selection) = self.selection else { return };
        let mut marks = Vec::new();
        for page in selection.pages() {
            let Some(geometry) = self.geometry(page) else { continue };
            let quads: Vec<_> = self
                .open()
                .map(|open| self.selection_rects(open, page))
                .unwrap_or_default()
                .into_iter()
                .map(|rect| geometry.rect_to_user(rect))
                .collect();
            if !quads.is_empty() {
                marks.push((page, quads));
            }
        }
        self.selection = None;
        for (page, quads) in marks {
            let kind = match self.tool {
                Tool::Highlight => Kind::Highlight { quads },
                Tool::Underline => Kind::Underline { quads },
                _ => Kind::StrikeOut { quads },
            };
            // Black highlights would hide the text: highlights default to yellow.
            let saved = self.inspector.color;
            if self.tool == Tool::Highlight && saved == Color::BLACK {
                self.inspector.color = Color::YELLOW;
            }
            self.annotate(page, kind, window, cx);
            self.inspector.color = saved;
        }
    }

    fn start_text(&mut self, page: usize, at: Point, window: &mut Window, cx: &mut Context<Self>) {
        let theme = self.theme;
        let input = cx.new(|cx| TextInput::new(t("Type, then Enter"), |_| true, theme, cx));
        cx.subscribe_in(&input, window, |viewer, _, event, window, cx| match event {
            TextInputEvent::Commit { text, .. } => viewer.commit_text(text.clone(), window, cx),
            TextInputEvent::Cancel => {
                viewer.text_draft = None;
                window.focus(&viewer.focus);
                cx.notify();
            }
            TextInputEvent::Changed(_) => {}
        })
        .detach();
        input.update(cx, |input, cx| input.focus_all(window, cx));
        self.text_draft = Some(TextDraft { page, at, input });
        cx.notify();
    }

    fn commit_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(TextDraft { page, at, .. }) = self.text_draft.take() else { return };
        window.focus(&self.focus);
        let Some(geometry) = self.geometry(page) else { return };
        if text.trim().is_empty() {
            return cx.notify();
        }
        let size = self.inspector.size;
        let area = PageRect { x: at.x, y: at.y, width: text_width(&text, size, Font::Helvetica) + 6.0, height: size * 1.3 + 4.0 };
        let kind = Kind::Text { rect: geometry.rect_to_user(area), text, size, font: Font::Helvetica, upright: geometry.rotate };
        self.annotate(page, kind, window, cx);
    }

    fn prompt_image(&mut self, page: usize, at: Point, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await
                && let Some(path) = paths.into_iter().next()
            {
                this.update_in(cx, |viewer, window, cx| viewer.place_image(&path, page, at, window, cx)).ok();
            }
        })
        .detach();
    }

    /// Places an image file, 200 points wide on screen, its top-left corner at `at`.
    pub(super) fn place_image(&mut self, path: &std::path::Path, page: usize, at: Point, window: &mut Window, cx: &mut Context<Self>) {
        let raster = match load_raster(path) {
            Ok(raster) => raster,
            Err(error) => {
                self.notice = Some(failure("Cannot read the image", error));
                return cx.notify();
            }
        };
        let Some(geometry) = self.geometry(page) else { return };
        let width = 200.0_f32;
        let height = width * raster.height as f32 / raster.width.max(1) as f32;
        let area = PageRect { x: at.x, y: at.y, width, height };
        let kind = Kind::Image { rect: geometry.rect_to_user(area), image: Arc::new(raster), upright: geometry.rotate };
        self.annotate(page, kind, window, cx);
    }

    /// Places a saved signature with its top-left corner at `at`.
    pub(super) fn place_signature(&mut self, signature: &Signature, page: usize, at: Point, window: &mut Window, cx: &mut Context<Self>) {
        let Some(geometry) = self.geometry(page) else { return };
        let kind = match signature {
            Signature::Typed { name, style, .. } => {
                // A vector shape from the handwriting font: the same everywhere, no font embedded.
                let font = crate::signatures::STYLES[(*style).min(crate::signatures::STYLES.len() - 1)].1;
                let Some(outline) = Outline::from_text(font, name) else {
                    self.notice = Some(t("This style cannot write these characters.").into());
                    return cx.notify();
                };
                let height = 36.0;
                let area = PageRect { x: at.x, y: at.y, width: height * outline.aspect, height };
                Kind::Shape { rect: geometry.rect_to_user(area), outline: Arc::new(outline), upright: geometry.rotate }
            }
            Signature::Drawn { strokes, .. } => {
                // 160 x 64 points on screen.
                let (width, height) = (64.0 * DRAWN_ASPECT, 64.0);
                let strokes = strokes
                    .iter()
                    .map(|stroke| {
                        stroke.iter().map(|&(x, y)| geometry.to_user(Point { x: at.x + x * width, y: at.y + y * height })).collect()
                    })
                    .collect();
                Kind::Ink { strokes }
            }
        };
        let (width, color) = (self.inspector.width, self.inspector.color);
        if matches!(signature, Signature::Drawn { .. }) {
            self.inspector.width = 1.8;
        }
        self.inspector.color = signature.color();
        self.annotate(page, kind, window, cx);
        (self.inspector.width, self.inspector.color) = (width, color);
    }

    /// Drag preview and text editor over displayed page `index`, whose box is `frame`
    /// (x, y, width, height in content coordinates) at `zoom`.
    pub(super) fn tool_overlay(&self, index: usize, frame: (f32, f32, f32, f32), zoom: f32) -> Option<AnyElement> {
        let origin = (frame.0, frame.1);
        if let Some(draft) = self.text_draft.as_ref().filter(|draft| draft.page == index) {
            return Some(
                div()
                    .absolute()
                    .left(px(origin.0 + draft.at.x * zoom))
                    .top(px(origin.1 + draft.at.y * zoom))
                    .w(px(260.))
                    .child(draft.input.clone())
                    .into_any_element(),
            );
        }
        let sketch = self.sketch.as_ref().filter(|sketch| sketch.page == index)?;
        let points: Vec<Point> = sketch.points.clone();
        let tool = self.tool;
        let color = rgb(self.inspector.color);
        let width = (self.inspector.width * zoom).max(1.0);
        Some(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let at = |p: Point| point(bounds.origin.x + px(p.x * zoom), bounds.origin.y + px(p.y * zoom));
                    let (Some(&first), Some(&last)) = (points.first(), points.last()) else { return };
                    if matches!(tool, Tool::Redact | Tool::Erase | Tool::Crop) {
                        let r = span(first, last);
                        let mut fill = PathBuilder::fill();
                        fill.move_to(at(Point { x: r.x, y: r.y }));
                        fill.line_to(at(Point { x: r.right(), y: r.y }));
                        fill.line_to(at(Point { x: r.right(), y: r.bottom() }));
                        fill.line_to(at(Point { x: r.x, y: r.bottom() }));
                        fill.close();
                        let shade = match tool {
                            Tool::Redact => gpui::rgba(0x0000_00aa),
                            Tool::Erase => gpui::rgba(0xffff_ffdd),
                            _ => gpui::rgba(0xff6b_3533),
                        };
                        if let Ok(path) = fill.build() {
                            window.paint_path(path, shade);
                        }
                        return;
                    }
                    let mut path = PathBuilder::stroke(px(width));
                    match tool {
                        Tool::Rectangle => {
                            let r = span(first, last);
                            path.move_to(at(Point { x: r.x, y: r.y }));
                            path.line_to(at(Point { x: r.right(), y: r.y }));
                            path.line_to(at(Point { x: r.right(), y: r.bottom() }));
                            path.line_to(at(Point { x: r.x, y: r.bottom() }));
                            path.close();
                        }
                        Tool::Ellipse => {
                            let r = span(first, last);
                            let (cx, cy, rx, ry) = (r.x + r.width / 2.0, r.y + r.height / 2.0, r.width / 2.0, r.height / 2.0);
                            for step in 0..=48 {
                                let angle = step as f32 / 48.0 * std::f32::consts::TAU;
                                let p = at(Point { x: cx + rx * angle.cos(), y: cy + ry * angle.sin() });
                                if step == 0 { path.move_to(p) } else { path.line_to(p) }
                            }
                        }
                        _ => {
                            path.move_to(at(first));
                            for &p in &points[1..] {
                                path.line_to(at(p));
                            }
                        }
                    }
                    if let Ok(path) = path.build() {
                        window.paint_path(path, color);
                    }
                },
            )
            .absolute()
            .left(px(origin.0))
            .top(px(origin.1))
            .w(px(frame.2))
            .h(px(frame.3))
            .into_any_element(),
        )
    }
}

impl Viewer {
    /// Deletes the image under displayed point `at` of page `page`, if there is one.
    fn delete_image_at(&mut self, page: usize, at: Point, window: &mut Window, cx: &mut Context<Self>) {
        let Some(geometry) = self.geometry(page) else { return };
        let Some(id) = self.open().and_then(|open| open.page_states.get(page)).map(|state| state.id) else { return };
        self.edit(Op::DeleteImage { page: id, at: geometry.to_user(at) }, window, cx);
    }
}
