//! The document view: every page in one continuous, virtualised scroll, rendered on the render thread.
//! Only pages near the viewport get a bitmap; the rest are empty boxes of the right size.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, IntoElement, KeyBinding, ParentElement, Render, RenderImage,
    ExternalPaths, MouseButton, ScrollHandle, ScrollWheelEvent, SharedString, Styled, Window, WindowAppearance, actions, div, img, point,
    prelude::*, px,
};
use recto_core::form::FormField;
use recto_core::geom::{PageGeometry, PageRect, Point, Size};
use recto_core::layout::{self, PageLayout};
use recto_core::op::{PageState, Rotation};
use recto_core::{Document, PageId};
use recto_render::{Bitmap, DocInfo, PageText, PdfiumRenderer, Purpose, RenderRequest, RenderThread, Response, Tile};

mod files;
mod annotate;
mod bars;
mod dialog;
mod edit;
mod forms;
mod ocr;
mod pick;
mod output;
mod password;
mod rail;
mod search;
mod selection;
mod sidebar;
mod signature;
mod textedit;
mod tools;

pub use self::sidebar::Tab;
pub use self::edit::{EditSpec, parse_edits};
pub use self::annotate::Tool;
pub use self::bars::MenuKind;
use self::annotate::{Inspector, Sketch, TextDraft};
use self::dialog::{Dialog, DialogKind};
use self::forms::FieldEditor;
use self::signature::SignatureDialog;

use self::ocr::OcrJob;
use self::pick::Picked;
use self::textedit::EditDraft;
use self::tools::{ToolDialog, ToolKind};
use self::output::SplitMode;
use self::password::Locked;
use self::rail::RailSelection;
use self::search::Search;
use self::selection::Selection;
use self::sidebar::Sidebar;
use crate::{Args, recent};
use crate::i18n::t;
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::Theme;

actions!(
    viewer,
    [
        ZoomIn, ZoomOut, ActualSize, FitWidth, FitPage, GoToPage, LineDown, LineUp, ScreenDown, ScreenUp, FirstPage,
        LastPage, ToggleThumbnails, ToggleOutline, Find, FindNext, FindPrevious, CopySelection, OpenFile, Undo, Redo,
        Save, SaveAs, RotateLeft, RotateRight, DeletePages, InsertBlank, InsertFile, Extract, Split, CloseMenu
    ]
);

const TOOLBAR_HEIGHT: f32 = 44.0;
/// Height of the tool bar under it, for the first layout.
const TOOL_BAR_HEIGHT: f32 = 40.0;
/// Arrow-key scroll step, in logical pixels.
const LINE: f32 = 48.0;
/// Pages rendered beyond the viewport, in viewport heights, so scrolling finds them ready.
const PRERENDER: f32 = 1.0;
/// Bitmaps kept around the visible pages before being released.
const KEEP_PAGES: usize = 6;
/// Page texts kept when no search is open: beyond `MAX_TEXTS`, only those near the viewport stay.
const MAX_TEXTS: usize = 200;
const KEEP_TEXTS: usize = 20;
const MIN_ZOOM: f32 = 0.1;
/// Content coordinates are f32: 1600 % keeps a 500-page document well within their precision.
const MAX_ZOOM: f32 = 16.0;
const ZOOM_STEPS: [f32; 21] =
    [0.1, 0.25, 0.33, 0.5, 0.67, 0.75, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0, 12.0, 16.0];
/// Pages whose longest side exceeds this many device pixels are drawn as tiles over a
/// lower-resolution backdrop of at most `BACKDROP` pixels.
const TILED_ABOVE: f32 = 4096.0;
const BACKDROP: f32 = 2048.0;
/// Tile side, in device pixels.
const TILE: u32 = 512;

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
    /// Which opening this is (`Viewer::opened`), to ignore late answers meant for a previous one.
    serial: u64,
    /// The document and its edit log; `page_states` are the pages of the bytes being shown.
    document: Document,
    page_states: Vec<PageState>,
    /// How each shown page maps to user space, and the form fields of the opened file.
    geometry: Vec<PageGeometry>,
    fields: Vec<FormField>,
    /// See `Known::marks`.
    marks: Vec<Vec<u64>>,
    renderer: RenderThread,
    info: Option<DocInfo>,
    layout: Option<PageLayout>,
    pages: HashMap<usize, Cached>,
    in_flight: HashSet<(usize, u32)>,
    /// Device pixels per point for the current zoom; a change starts a new render generation.
    render_scale: f32,
    generation: u64,
    /// Pages whose render failed; they show a message instead of a bitmap.
    failed: HashSet<usize>,
    thumbnails: HashMap<usize, Arc<RenderImage>>,
    thumbnails_in_flight: HashSet<usize>,
    /// Page texts, for search and selection.
    texts: HashMap<usize, PageText>,
    texts_in_flight: HashSet<usize>,
    tiles: HashMap<TileKey, Arc<RenderImage>>,
    tiles_in_flight: HashSet<TileKey>,
    /// Page to scroll to once the first layout exists (from `--page`).
    pending_page: Option<usize>,
}

/// The pages of some built bytes, how they map to user space, and the form fields.
pub(super) struct Known {
    pages: Vec<PageState>,
    geometry: Vec<PageGeometry>,
    fields: Vec<FormField>,
    /// Per page, the ids of the ops drawn on it (annotations, filled fields, redactions, crops,
    /// stamps, and for page numbers the position): a page whose
    /// state and marks are unchanged keeps its bitmap across a rebuild.
    marks: Vec<Vec<u64>>,
}

impl Known {
    /// Read from `document` (as it is now) with lopdf.
    fn of(document: &Document) -> Self {
        let pages = document.pages();
        let geometry = document.geometry_of(&pages).unwrap_or_default();
        Known::with(document, pages, geometry, document.form_fields().unwrap_or_default())
    }

    fn with(document: &Document, pages: Vec<PageState>, geometry: Vec<PageGeometry>, fields: Vec<FormField>) -> Self {
        let mut by_page: HashMap<PageId, Vec<u64>> = HashMap::new();
        for (op, &id) in document.ops().iter().zip(document.op_ids()) {
            match op {
                recto_core::op::Op::AddAnnotation { page, .. }
                | recto_core::op::Op::Redact { page, .. }
                | recto_core::op::Op::Erase { page, .. }
                | recto_core::op::Op::DeleteImage { page, .. }
                | recto_core::op::Op::AddTextLayer { page, .. }
                | recto_core::op::Op::EditText { page, .. } => {
                    by_page.entry(*page).or_default().push(id)
                }
                recto_core::op::Op::CropPages { pages: cropped, .. } => {
                    for page in cropped {
                        by_page.entry(*page).or_default().push(id);
                    }
                }
                recto_core::op::Op::AddStamp(stamp) => {
                    let count = pages.len() as u64;
                    for (index, page) in pages.iter().enumerate() {
                        if stamp.pages.as_ref().is_some_and(|only| !only.contains(&page.id)) {
                            continue;
                        }
                        let marks = by_page.entry(page.id).or_default();
                        marks.push(id);
                        // Numbers change with the page's position: keep it in the marks.
                        if stamp.is_numbered() {
                            marks.push(1 << 63 | (index as u64) << 32 | count);
                        }
                    }
                }
                recto_core::op::Op::FillField { name, .. } => {
                    for field in fields.iter().filter(|field| &field.name == name) {
                        for widget in &field.widgets {
                            by_page.entry(widget.page).or_default().push(id);
                        }
                    }
                }
                _ => {}
            }
        }
        let marks = pages.iter().map(|page| by_page.get(&page.id).cloned().unwrap_or_default()).collect();
        Known { pages, geometry, fields, marks }
    }
}

/// A tile of a page at one render scale; `x` and `y` are its corner in device pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct TileKey {
    page: usize,
    scale: u32,
    x: u32,
    y: u32,
}

impl Open {
    /// Size of page `index` in device pixels at the current render scale.
    fn page_pixels(&self, index: usize) -> (f32, f32) {
        let size = self.info.as_ref().and_then(|info| info.page_sizes.get(index).copied());
        size.map_or((0.0, 0.0), |size| (size.width * self.render_scale, size.height * self.render_scale))
    }

    /// Scale of the whole-page bitmap of page `index`: the render scale, or a backdrop scale for
    /// pages drawn as tiles.
    fn page_scale(&self, index: usize) -> f32 {
        let (width, height) = self.page_pixels(index);
        let longest = width.max(height);
        if longest > TILED_ABOVE { self.render_scale * BACKDROP / longest } else { self.render_scale }
    }

    /// Tiles of page `index` that intersect `screen` (content coordinates, logical pixels);
    /// none if the page is small enough for one bitmap.
    fn visible_tiles(&self, layout: &PageLayout, index: usize, screen: PageRect, scale_factor: f32) -> Vec<Tile> {
        let (width, height) = self.page_pixels(index);
        let Some(page) = layout.boxes.get(index) else { return Vec::new() };
        if width.max(height) <= TILED_ABOVE {
            return Vec::new();
        }
        // The visible part of the page, in device pixels from its top-left corner.
        let left = ((screen.x - page.x) * scale_factor).max(0.0);
        let top = ((screen.y - page.y) * scale_factor).max(0.0);
        let right = ((screen.right() - page.x) * scale_factor).min(width);
        let bottom = ((screen.bottom() - page.y) * scale_factor).min(height);
        if left >= right || top >= bottom {
            return Vec::new();
        }
        let (page_width, page_height) = (width.ceil() as u32, height.ceil() as u32);
        let mut tiles = Vec::new();
        for row in (top as u32 / TILE)..=((bottom as u32).saturating_sub(1) / TILE) {
            for column in (left as u32 / TILE)..=((right as u32).saturating_sub(1) / TILE) {
                let (x, y) = (column * TILE, row * TILE);
                tiles.push(Tile { x, y, width: TILE.min(page_width - x), height: TILE.min(page_height - y) });
            }
        }
        tiles
    }
}

enum State {
    Empty,
    Error(SharedString),
    /// Waiting for the password of an encrypted document.
    Locked(Box<Locked>),
    Open(Box<Open>),
}

pub struct Viewer {
    state: State,
    theme: Theme,
    zoom: ZoomMode,
    scroll: ScrollHandle,
    focus: FocusHandle,
    page_input: Entity<TextInput>,
    sidebar: Sidebar,
    search: Search,
    selection: Option<Selection>,
    /// The extract / split modal, when open.
    dialog: Option<Dialog>,
    /// The reader agreed to close with unsaved changes.
    closing: bool,
    /// The active tool and its options.
    tool: Tool,
    inspector: Inspector,
    /// A shape or stroke being drawn, a text box being typed, a field being filled.
    sketch: Option<Sketch>,
    text_draft: Option<TextDraft>,
    field_editor: Option<FieldEditor>,
    /// The signature placed by the next click, and the dialog that chooses it.
    signature: Option<crate::signatures::Signature>,
    signature_dialog: Option<SignatureDialog>,
    recent: Vec<std::path::PathBuf>,
    recent_menu: bool,
    /// Documents opened so far in this window.
    opened: u64,
    /// Pages selected in the thumbnail rail.
    rail: RailSelection,
    /// A message for the reader (errors, confirmations), shown under the toolbar.
    notice: Option<SharedString>,
    /// Latest background build; older ones are discarded when they finish.
    build_version: u64,
    /// With `RECTO_TIMING` set: what is being timed (opening, an edit) and since when.
    timing: Option<(&'static str, std::time::Instant)>,
    /// Watermark, page numbers, properties, export.
    tool_dialog: Option<ToolDialog>,
    /// Text being edited with the Edit text tool.
    edit_draft: Option<EditDraft>,
    /// The open menu, and the last one closed by a click outside (with when).
    menu: Option<MenuKind>,
    menu_closed: Option<(MenuKind, std::time::Instant)>,
    /// Edit mode: tools, page operations and document tools are shown. Documents open read-only.
    editing: bool,
    /// The annotation picked with the Select tool in edit mode.
    picked: Option<Picked>,
    /// Text recognition in progress, and the engine once loaded.
    ocr: Option<OcrJob>,
    ocr_engine: Option<Arc<recto_ocr::Engine>>,
}

impl Viewer {
    pub fn new(args: Args, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let dark = args.dark.unwrap_or(matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark));
        let theme = Theme::new(dark);
        let zoom = args.zoom.map_or(ZoomMode::FitWidth, |pct| ZoomMode::Fixed((pct / 100.0).clamp(MIN_ZOOM, MAX_ZOOM)));
        let page_input = cx.new(|cx| TextInput::new("", |c| c.is_ascii_digit(), theme, cx));
        cx.subscribe_in(&page_input, window, |this, _, event, window, cx| match event {
            TextInputEvent::Commit { text, .. } => {
                if let Ok(page) = text.trim().parse::<usize>() {
                    this.go_to_page(page.saturating_sub(1), window, cx);
                }
                window.focus(&this.focus);
            }
            TextInputEvent::Cancel => window.focus(&this.focus),
            TextInputEvent::Changed(_) => {}
        })
        .detach();
        let search = Search::new(theme, window, cx);
        let mut viewer = Self {
            state: State::Empty,
            theme,
            zoom,
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            page_input,
            sidebar: Sidebar::new(args.sidebar),
            search,
            selection: None,
            dialog: None,
            closing: false,
            tool: args.tool.unwrap_or(Tool::Select),
            inspector: Inspector::default(),
            sketch: None,
            text_draft: None,
            field_editor: None,
            signature: None,
            signature_dialog: None,
            recent: recent::load(),
            recent_menu: false,
            opened: 0,
            rail: RailSelection::default(),
            notice: None,
            build_version: 0,
            timing: None,
            tool_dialog: None,
            edit_draft: None,
            menu: args.menu,
            menu_closed: None,
            picked: None,
            editing: args.edit || args.tool.is_some() || !args.apply.is_empty() || args.menu.is_some() || args.dialog.is_some(),
            ocr: None,
            ocr_engine: None,
        };
        // Focus first: a password prompt takes it over.
        window.focus(&viewer.focus);
        if let Some(path) = args.file {
            viewer.replace_document(path, args.page, window, cx);
        }
        if !args.apply.is_empty() {
            viewer.apply_specs(&args.apply, window, cx);
        }
        if let Some(path) = args.save_as {
            viewer.write_document(path, None, window, cx);
        }
        if let Some(path) = args.export {
            viewer.write_document(path, Some(args.export_options), window, cx);
        }
        if args.ocr {
            let pages = viewer.page_order();
            viewer.recognize(pages, window, cx);
        }
        match args.dialog.as_deref() {
            Some("export") => viewer.open_tool(ToolKind::Export, window, cx),
            Some("watermark") => viewer.open_tool(ToolKind::Watermark, window, cx),
            Some("numbers") => viewer.open_tool(ToolKind::PageNumbers, window, cx),
            Some("header") => viewer.open_tool(ToolKind::HeaderFooter, window, cx),
            Some("properties") => viewer.open_tool(ToolKind::Properties, window, cx),
            Some("extract") => viewer.open_dialog(DialogKind::Extract, window, cx),
            Some("split") => viewer.open_dialog(DialogKind::Split(SplitMode::Ranges), window, cx),
            _ => {}
        }
        if let Some(query) = args.find {
            viewer.preset_search(query, cx);
        }
        // Closing the window with unsaved changes asks first.
        let this = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            this.update(cx, |viewer, cx| viewer.may_close(window, cx)).unwrap_or(true)
        });
        viewer
    }

    /// Starts timing `what` when `RECTO_TIMING` is set; reported when the next page bitmap arrives.
    pub(super) fn time(&mut self, what: &'static str) {
        if std::env::var_os("RECTO_TIMING").is_some() {
            self.timing = Some((what, std::time::Instant::now()));
        }
    }

    /// Whether the window may close now; with unsaved changes, asks and closes it later if the
    /// reader agrees.
    fn may_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let modified = self.open().is_some_and(|open| open.document.is_modified());
        if !modified || self.closing {
            return true;
        }
        self.confirm_discard(window, cx, |viewer, window, _| {
            viewer.closing = true;
            window.remove_window();
        });
        false
    }

    /// Starts a render thread showing `bytes` (the document as built, unlocked with its password
    /// if it has one) and listens to it; page `page` (0-based) is shown once the layout exists.
    /// `known` holds what a background build already worked out for `bytes`; without it, the
    /// pages, their geometry and the form fields are read from `document` now.
    fn start(
        &mut self,
        document: Document,
        bytes: Arc<[u8]>,
        page: usize,
        known: Option<Known>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Open, SharedString> {
        let renderer = RenderThread::spawn(PdfiumRenderer::new).map_err(|e| failure("Cannot start the renderer", e))?;
        renderer.open(bytes, document.password());
        let Known { pages: page_states, geometry, fields, marks } = known.unwrap_or_else(|| Known::of(&document));
        let responses = renderer.responses().clone();
        // Responses go to this document only: once another one is open, the listener stops.
        self.opened += 1;
        let serial = self.opened;
        cx.spawn_in(window, async move |this, cx| {
            while let Ok(response) = responses.recv_async().await {
                let delivered = this.update_in(cx, |viewer, window, cx| {
                    let current = viewer.open().is_some_and(|open| open.serial == serial);
                    if current {
                        viewer.receive(response, window, cx);
                    }
                    current
                });
                if !matches!(delivered, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
        Ok(Open {
            serial,
            document,
            page_states,
            geometry,
            marks,
            fields,
            renderer,
            info: None,
            layout: None,
            pages: HashMap::new(),
            in_flight: HashSet::new(),
            render_scale: 0.0,
            generation: 0,
            failed: HashSet::new(),
            thumbnails: HashMap::new(),
            thumbnails_in_flight: HashSet::new(),
            texts: HashMap::new(),
            texts_in_flight: HashSet::new(),
            tiles: HashMap::new(),
            tiles_in_flight: HashSet::new(),
            pending_page: Some(page),
        })
    }

    fn receive(&mut self, response: Response, window: &mut Window, cx: &mut Context<Self>) {
        let State::Open(open) = &mut self.state else { return };
        match response {
            Response::Opened(Ok(info)) => {
                open.pending_page = open.pending_page.map(|page| page.min(info.page_count().saturating_sub(1)));
                open.info = Some(info);
                if self.search.open {
                    return self.advance_search(window, cx);
                }
            }
            Response::Opened(Err(e)) => self.state = State::Error(failure("Cannot open the document", e)),
            Response::Text { index, result } => return self.receive_text(index, result, window, cx),
            Response::Rendered { request, result } if request.purpose == Purpose::Ocr => {
                return match result {
                    Ok(bitmap) => self.receive_ocr_image(request.page, bitmap, window, cx),
                    Err(error) => {
                        self.ocr = None;
                        self.notice = Some(failure("Text recognition failed", error));
                        cx.notify();
                    }
                };
            }
            Response::Rendered { request, result } if request.purpose == Purpose::Thumbnail => match result {
                Ok(bitmap) => sidebar::store_thumbnail(open, request.index, bitmap, cx),
                // A missing thumbnail is not worth an error: the page itself reports it.
                Err(_) => {
                    open.thumbnails_in_flight.remove(&request.index);
                }
            },
            Response::Rendered { request, result } if let Some(tile) = request.tile => {
                let key = TileKey { page: request.index, scale: scale_key(request.scale), x: tile.x, y: tile.y };
                open.tiles_in_flight.remove(&key);
                // Tiles of a zoom level we have left are useless: the backdrop covers the page.
                if key.scale == scale_key(open.render_scale)
                    && let Ok(bitmap) = result
                    && let Some(image) = to_render_image(bitmap)
                    && let Some(old) = open.tiles.insert(key, image)
                {
                    cx.drop_image(old, Some(window));
                }
            }
            Response::Rendered { request, result } => {
                open.in_flight.remove(&(request.index, scale_key(request.scale)));
                match result {
                    Ok(bitmap) => {
                        // An edit is reported when its rebuild ends; bitmaps of the old bytes do not count.
                        if let Some((what, since)) = self.timing.filter(|(what, _)| *what != "edit") {
                            self.timing = None;
                            eprintln!("recto: {what}: first page bitmap after {} ms", since.elapsed().as_millis());
                        }
                        // A bitmap from a zoom level we have left only fills a page that has nothing yet.
                        let current = scale_key(request.scale) == scale_key(open.page_scale(request.index));
                        if (current || !open.pages.contains_key(&request.index))
                            && let Some(image) = to_render_image(bitmap)
                            && let Some(old) = open.pages.insert(request.index, Cached { image, scale: request.scale })
                        {
                            cx.drop_image(old.image, Some(window));
                        }
                    }
                    Err(e) => {
                        eprintln!("recto: page {}: {e:#}", request.index + 1);
                        open.failed.insert(request.index);
                    }
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
            Size { width: (f32::from(size.width) - self.sidebar.width()).max(1.0), height: (f32::from(size.height) - TOOLBAR_HEIGHT - TOOL_BAR_HEIGHT).max(1.0) }
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
        self.set_scroll_position(position);
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
            // Queued thumbnails were dropped with the old generation too.
            open.thumbnails_in_flight.clear();
            open.tiles_in_flight.clear();
        }

        // Visible pages first, then the ones around them, nearest first.
        let visible = layout.visible(top, viewport.height);
        let wanted = layout.visible(top - PRERENDER * viewport.height, viewport.height * (1.0 + 2.0 * PRERENDER));
        let distance = |i: usize| if i < visible.start { visible.start - i } else { i + 1 - visible.end };
        let mut nearby: Vec<usize> = wanted.filter(|i| !visible.contains(i)).collect();
        nearby.sort_by_key(|&i| distance(i));
        for index in visible.clone().chain(nearby) {
            let scale = open.page_scale(index);
            let key = (index, scale_key(scale));
            let ready = open.pages.get(&index).is_some_and(|c| scale_key(c.scale) == key.1);
            if !ready && open.in_flight.insert(key) {
                open.renderer.request(RenderRequest {
                    page: PageId { source: 0, index: index as u32 },
                    index,
                    scale,
                    generation: open.generation,
                    purpose: Purpose::Page,
                    tile: None,
                });
            }
        }

        // Pages too large for one bitmap: sharp tiles over the part on screen.
        let screen = PageRect { x: position.x, y: top, width: viewport.width, height: viewport.height };
        let mut on_screen = HashSet::new();
        for index in visible.clone() {
            for tile in open.visible_tiles(layout, index, screen, scale_factor) {
                let key = TileKey { page: index, scale: scale_key(render_scale), x: tile.x, y: tile.y };
                on_screen.insert(key);
                if !open.tiles.contains_key(&key) && open.tiles_in_flight.insert(key) {
                    open.renderer.request(RenderRequest {
                        page: PageId { source: 0, index: index as u32 },
                        index,
                        scale: render_scale,
                        generation: open.generation,
                        purpose: Purpose::Page,
                        tile: Some(tile),
                    });
                }
            }
        }
        let gone: Vec<TileKey> = open.tiles.keys().copied().filter(|key| !on_screen.contains(key)).collect();
        for key in gone {
            if let Some(image) = open.tiles.remove(&key) {
                cx.drop_image(image, Some(window));
            }
        }

        // Text of the visible pages, so a selection can start anywhere at once.
        for index in visible.clone() {
            if !open.texts.contains_key(&index) && open.texts_in_flight.insert(index) {
                open.renderer.text(index);
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
        // Texts are small, but a long document read end to end should not keep them all; a search
        // needs every page, so nothing is released while one is open.
        if !self.search.open && open.texts.len() > MAX_TEXTS {
            let keep = visible.start.saturating_sub(KEEP_TEXTS)..visible.end + KEEP_TEXTS;
            let selected = self.selection.map(|s| s.pages());
            open.texts.retain(|i, _| keep.contains(i) || selected.as_ref().is_some_and(|pages| pages.contains(i)));
        }
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
            State::Empty => return self.start_screen(cx),
            State::Error(text) => return message(text.clone()),
            State::Locked(locked) => return self.password_prompt(locked, cx),
            State::Open(open) => open,
        };
        let Some(layout) = &open.layout else { return message(t("Loading…").into()) };

        let visible = layout.visible(self.scroll_position().y, self.viewport(window).height);
        let mut content = div()
            .relative()
            .w(px(layout.content.width))
            .h(px(layout.content.height))
            .on_scroll_wheel(cx.listener(Self::on_wheel))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_pointer_down))
            .on_mouse_move(cx.listener(Self::on_pointer_move))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, window, cx| this.on_pointer_up(window, cx)));
        for index in visible {
            let page = layout.boxes[index];
            let frame = div()
                .absolute()
                .left(px(page.x))
                .top(px(page.y))
                .w(px(page.width))
                .h(px(page.height))
                .bg(theme.placeholder)
                .shadow_md()
                .when(self.tool.selects_text(), |frame| frame.cursor_text())
                .when(!self.tool.selects_text(), |frame| frame.cursor_crosshair());
            // A bitmap from another zoom level is stretched until the right one arrives.
            content = content.child(match open.pages.get(&index) {
                Some(cached) => frame.child(img(cached.image.clone()).size_full()),
                None if open.failed.contains(&index) => frame
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(theme.muted)
                    .child(t("Cannot render the page")),
                None => frame,
            });
            // Sharp tiles over the backdrop of a large page.
            let current = scale_key(open.render_scale);
            let scale_factor = window.scale_factor();
            let tiles = open.tiles.iter().filter(|(key, _)| key.page == index && key.scale == current).map(|(key, image)| {
                let size = image.size(0);
                div()
                    .absolute()
                    .left(px(page.x + key.x as f32 / scale_factor))
                    .top(px(page.y + key.y as f32 / scale_factor))
                    .w(px(size.width.0 as f32 / scale_factor))
                    .h(px(size.height.0 as f32 / scale_factor))
                    .child(img(image.clone()).size_full())
            });
            content = content.children(tiles);
            // Search hits, drawn over the bitmap.
            let zoom = layout.zoom;
            let marks = self.highlights(open, index).into_iter().map(|(rect, current)| {
                div()
                    .absolute()
                    .left(px(page.x + rect.x * zoom))
                    .top(px(page.y + rect.y * zoom))
                    .w(px(rect.width * zoom))
                    .h(px(rect.height * zoom))
                    .bg(if current { theme.hit_current } else { theme.hit })
            });
            content = content.children(marks);
            let selected = self.selection_rects(open, index).into_iter().map(|rect| {
                div()
                    .absolute()
                    .left(px(page.x + rect.x * zoom))
                    .top(px(page.y + rect.y * zoom))
                    .w(px(rect.width * zoom))
                    .h(px(rect.height * zoom))
                    .bg(theme.selection)
            });
            content = content.children(selected);
            content = content.children(self.form_overlay(index, (page.x, page.y), zoom, cx));
            content = content.children(self.tool_overlay(index, (page.x, page.y, page.width, page.height), zoom));
            content = content.children(self.text_edit_overlay(index, (page.x, page.y, page.width, page.height), zoom));
            content = content.children(self.pick_overlay(index, (page.x, page.y, page.width, page.height), zoom));
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
        let theme = self.theme;
        div()
            .key_context("Viewer")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &ToggleThumbnails, _, cx| this.toggle_sidebar(Tab::Thumbnails, cx)))
            .on_action(cx.listener(|this, _: &ToggleOutline, _, cx| this.toggle_sidebar(Tab::Outline, cx)))
            .on_action(cx.listener(|this, _: &Find, window, cx| this.open_search(window, cx)))
            .on_action(cx.listener(|this, _: &FindNext, window, cx| this.step_hit(true, window, cx)))
            .on_action(cx.listener(|this, _: &FindPrevious, window, cx| this.step_hit(false, window, cx)))
            .on_action(cx.listener(|this, _: &CopySelection, _, cx| this.copy_selection(cx)))
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
            .on_action(cx.listener(|this, _: &OpenFile, window, cx| this.prompt_open(window, cx)))
            .on_action(cx.listener(|this, _: &CloseMenu, _, cx| {
                this.menu = None;
                this.recent_menu = false;
                this.picked = None;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Undo, window, cx| this.undo(window, cx)))
            .on_action(cx.listener(|this, _: &Redo, window, cx| this.redo(window, cx)))
            .on_action(cx.listener(|this, _: &Save, window, cx| this.save(window, cx)))
            .on_action(cx.listener(|this, _: &SaveAs, window, cx| this.save_as(window, cx)))
            .on_action(cx.listener(|this, _: &RotateLeft, window, cx| if this.editing { this.rotate(Rotation::R270, window, cx) }))
            .on_action(cx.listener(|this, _: &RotateRight, window, cx| if this.editing { this.rotate(Rotation::R90, window, cx) }))
            .on_action(cx.listener(|this, _: &DeletePages, window, cx| {
                // Delete removes the picked annotation first, else the selected pages.
                if this.editing && !this.delete_picked(window, cx) {
                    this.delete_pages(window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &InsertBlank, window, cx| if this.editing { this.insert_blank(window, cx) }))
            .on_action(cx.listener(|this, _: &InsertFile, window, cx| if this.editing { this.prompt_insert_files(window, cx) }))
            .on_action(cx.listener(|this, _: &Extract, window, cx| this.open_dialog(DialogKind::Extract, window, cx)))
            .on_action(cx.listener(|this, _: &Split, window, cx| {
                this.open_dialog(DialogKind::Split(SplitMode::Ranges), window, cx)
            }))
            .on_drop(cx.listener(Self::on_drop_paths))
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.border_2().border_color(theme.highlight))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.canvas)
            .child(self.app_bar(cx))
            .children(self.tool_bar(cx).filter(|_| self.editing))
            .children(self.search_bar(cx))
            .child(div().flex().flex_1().min_h_0().children(self.sidebar(window, cx)).child(self.pages(window, cx)))
            .children(self.recent_menu(cx))
            .children(self.dialog_view(cx))
            .children(self.signature_view(cx))
            .children(self.tool_view(cx))
            .children(self.notice_toast(cx))
    }
}

impl Viewer {
    /// The latest notice, floating at the bottom of the window, with a button to dismiss it.
    fn notice_toast(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let notice = self.notice.clone()?;
        let theme = self.theme;
        Some(
            div()
                .absolute()
                .bottom(px(20.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .id("notice")
                        .occlude()
                        .max_w(px(640.))
                        .mx(px(16.))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .pl(px(14.))
                        .pr(px(6.))
                        .py(px(6.))
                        .rounded(px(10.))
                        .bg(theme.tooltip)
                        .text_color(theme.tooltip_text)
                        .text_sm()
                        .shadow_lg()
                        .child(div().flex_1().child(notice))
                        .child(
                            div()
                                .id("notice-close")
                                .size(px(24.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(theme.radius))
                                .cursor_pointer()
                                .tooltip(crate::chrome::tooltip("Dismiss", theme))
                                .child(gpui::svg().path(crate::assets::icon("x")).size(px(14.)).text_color(theme.tooltip_text))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.notice = None;
                                    cx.notify();
                                })),
                        ),
                )
                .into_any_element(),
        )
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
        KeyBinding::new("f4", ToggleThumbnails, Some("Viewer")),
        KeyBinding::new("shift-f4", ToggleOutline, Some("Viewer")),
        KeyBinding::new("secondary-f", Find, Some("Viewer")),
        KeyBinding::new("secondary-o", OpenFile, Some("Viewer")),
        KeyBinding::new("secondary-z", Undo, PAGES),
        KeyBinding::new("secondary-y", Redo, PAGES),
        KeyBinding::new("secondary-shift-z", Redo, PAGES),
        KeyBinding::new("secondary-s", Save, Some("Viewer")),
        KeyBinding::new("secondary-shift-s", SaveAs, Some("Viewer")),
        KeyBinding::new("secondary-r", RotateRight, PAGES),
        KeyBinding::new("secondary-shift-r", RotateLeft, PAGES),
        KeyBinding::new("delete", DeletePages, PAGES),
        KeyBinding::new("secondary-e", Extract, Some("Viewer")),
        KeyBinding::new("f3", FindNext, Some("Viewer")),
        KeyBinding::new("escape", CloseMenu, PAGES),
        KeyBinding::new("shift-f3", FindPrevious, Some("Viewer")),
        KeyBinding::new("secondary-c", CopySelection, PAGES),
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

#[cfg(test)]
mod tests {
    use recto_core::annot::{Annotation, Color, Kind};
    use recto_core::geom::Rect;
    use recto_core::op::Op;

    use super::*;

    fn fixture(name: &str) -> Document {
        Document::open(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures").join(name)).unwrap()
    }

    fn marks(document: &Document) -> Vec<Vec<u64>> {
        Known::of(document).marks
    }

    fn rectangle(x: f32) -> Annotation {
        let rect = Rect { origin: Point { x, y: 100.0 }, width: 50.0, height: 50.0 };
        Annotation { kind: Kind::Rectangle { rect }, color: Color::RED, width: 1.0 }
    }

    /// A page keeps its bitmap across a rebuild only if its marks are unchanged: an annotation
    /// must change its page's marks, and only that page's.
    #[test]
    fn marks_change_only_for_the_annotated_page() {
        let mut document = fixture("sample.pdf");
        let before = marks(&document);
        let second = document.pages()[1].id;
        document.apply(Op::AddAnnotation { page: second, annotation: rectangle(100.0) });
        let one = marks(&document);
        assert_eq!((one[0].clone(), one[2].clone()), (before[0].clone(), before[2].clone()));
        assert_ne!(one[1], before[1]);

        // Undo, then another annotation: as many ops as before, yet a different page.
        document.undo();
        assert_eq!(marks(&document), before);
        document.apply(Op::AddAnnotation { page: second, annotation: rectangle(200.0) });
        let two = marks(&document);
        assert_ne!(two[1], one[1]);
        assert_ne!(two[1], before[1]);
    }

    #[test]
    fn filling_a_field_changes_its_page_marks() {
        let mut document = fixture("forms.pdf");
        let before = marks(&document);
        document.apply(Op::FillField { name: "agree".into(), value: "Yes".into() });
        assert_ne!(marks(&document)[0], before[0]);
    }
}
