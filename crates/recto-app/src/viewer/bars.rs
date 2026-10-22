//! The window chrome: an app bar (files, pages, zoom, history, menus, save) and a tool bar
//! (annotation and editing tools with their colours and widths). Page operations and document
//! tools live in two menus.

use gpui::{
    AnyElement, ClickEvent, Context, IntoElement, ParentElement, SharedString, Styled, Window, anchored, deferred, div, prelude::*, px, svg,
};
use recto_core::op::Rotation;

use super::annotate::{COLORS, FONTS, SIZES, Tool, WIDTHS, text_area};
use recto_core::annot::{Annotation, Font, Kind};
use recto_core::geom::{PageGeometry, Point};
use super::dialog::DialogKind;
use super::output::SplitMode;
use super::tools::ToolKind;
use super::{TOOLBAR_HEIGHT, Tab, Viewer, ZoomMode};
use crate::assets::icon;
use crate::chrome::{CONTROL, ICON, icon_button, keyed_icon_button, primary_button, separator, text_button};
use crate::i18n::t;
use crate::recent;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKind {
    Pages,
    Document,
}

impl MenuKind {
    /// Name used by `--menu=NAME`.
    pub fn parse(name: &str) -> Option<MenuKind> {
        match name {
            "pages" => Some(MenuKind::Pages),
            "document" => Some(MenuKind::Document),
            _ => None,
        }
    }
}

/// Groups of the tool bar.
const TOOL_GROUPS: [&[Tool]; 5] = [
    &[Tool::Select],
    &[Tool::Text, Tool::EditText, Tool::Highlight, Tool::Underline, Tool::StrikeOut],
    &[Tool::Rectangle, Tool::Ellipse, Tool::Line, Tool::Arrow, Tool::Freehand, Tool::Marker],
    &[Tool::Image, Tool::Signature, Tool::Stamp, Tool::Link, Tool::Form],
    &[Tool::Redact, Tool::Erase, Tool::DeleteImage, Tool::Crop],
];

fn tool_icon(tool: Tool) -> &'static str {
    match tool {
        Tool::Select => "mouse-pointer-2",
        Tool::Text => "type",
        Tool::Highlight => "highlighter",
        Tool::Underline => "underline",
        Tool::StrikeOut => "strikethrough",
        Tool::Rectangle => "square",
        Tool::Ellipse => "circle",
        Tool::Line => "minus",
        Tool::Arrow => "move-up-right",
        Tool::Freehand => "pencil",
        Tool::Marker => "paintbrush",
        Tool::Image => "image",
        Tool::Signature => "signature",
        Tool::Stamp => "stamp",
        Tool::Link => "link",
        Tool::Form => "square-check",
        Tool::Redact => "eye-off",
        Tool::Erase => "eraser",
        Tool::DeleteImage => "image-off",
        Tool::Crop => "crop",
        Tool::EditText => "text-cursor-input",
    }
}

/// A shortcut as the platform writes it: `Ctrl+Shift+R`, `⇧⌘R`.
fn shortcut(keys: &str) -> String {
    if cfg!(target_os = "macos") {
        keys.replace("Ctrl+Shift+", "⇧⌘").replace("Ctrl+", "⌘")
    } else {
        keys.replace("Shift", t("Shift")).replace("Del", t("Del"))
    }
}

type Action = fn(&mut Viewer, &mut Window, &mut Context<Viewer>);

/// One entry of a menu: icon, label, shortcut and what it does. `None` is a separator.
type Entry = Option<(&'static str, &'static str, &'static str, Action)>;

fn pages_menu() -> Vec<Entry> {
    vec![
        Some(("rotate-ccw", "Rotate left", "Ctrl+Shift+R", |v, w, cx| v.rotate(Rotation::R270, w, cx))),
        Some(("rotate-cw", "Rotate right", "Ctrl+R", |v, w, cx| v.rotate(Rotation::R90, w, cx))),
        Some(("trash-2", "Delete", "Del", |v, w, cx| v.delete_pages(w, cx))),
        None,
        Some(("file-plus", "Blank page", "", |v, w, cx| v.insert_blank(w, cx))),
        Some(("file-input", "Insert file…", "", |v, w, cx| v.prompt_insert_files(w, cx))),
        None,
        Some(("file-output", "Extract…", "Ctrl+E", |v, w, cx| v.open_dialog(DialogKind::Extract, w, cx))),
        Some(("scissors", "Split…", "", |v, w, cx| v.open_dialog(DialogKind::Split(SplitMode::Ranges), w, cx))),
    ]
}

fn document_menu() -> Vec<Entry> {
    vec![
        Some(("printer", "Print…", "Ctrl+P", |v, w, cx| v.print(w, cx))),
        Some(("save", "Save as…", "Ctrl+Shift+S", |v, w, cx| v.save_as(w, cx))),
        Some(("download", "Export…", "", |v, w, cx| v.open_tool(ToolKind::Export, w, cx))),
        None,
        Some(("droplets", "Watermark…", "", |v, w, cx| v.open_tool(ToolKind::Watermark, w, cx))),
        Some(("hash", "Page numbers…", "", |v, w, cx| v.open_tool(ToolKind::PageNumbers, w, cx))),
        Some(("heading", "Header or footer…", "", |v, w, cx| v.open_tool(ToolKind::HeaderFooter, w, cx))),
        Some(("info", "Properties…", "", |v, w, cx| v.open_tool(ToolKind::Properties, w, cx))),
        None,
        Some(("scan-text", "Recognize text", "", |v, w, cx| {
            // The pages selected in the rail, else every page.
            let order = v.page_order();
            let selected = v.rail.in_order(&order);
            v.recognize(if selected.is_empty() { order } else { selected }, w, cx);
        })),
    ]
}

impl Viewer {
    fn toggle_menu(&mut self, menu: MenuKind, cx: &mut Context<Self>) {
        // A click on the button of an open menu first closes it (click outside): do not reopen.
        if self.menu_closed.is_some_and(|(kind, at)| kind == menu && at.elapsed().as_millis() < 400) {
            self.menu_closed = None;
            return;
        }
        self.menu = if self.menu == Some(menu) { None } else { Some(menu) };
        self.recent_menu = false;
        cx.notify();
    }

    /// The app bar: files and sidebar, page and zoom, history, menus and save.
    pub(super) fn app_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme;
        let open = self.open();
        let ready = open.is_some_and(|open| open.info.is_some() && open.layout.is_some());
        let (page_count, zoom_label) = match open.and_then(|open| open.info.as_ref().zip(open.layout.as_ref())) {
            Some((info, layout)) => (info.page_count(), format!("{:.0} %", layout.zoom * 100.0)),
            None => (0, String::new()),
        };
        let current = self.current_page();
        let modified = open.is_some_and(|open| open.document.is_modified());

        let mut bar = div()
            .flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .gap(px(2.))
            .min_h(px(TOOLBAR_HEIGHT))
            .px(px(8.))
            .py(px(4.))
            .bg(theme.toolbar)
            .border_b_1()
            .border_color(theme.border)
            .text_color(theme.text)
            .text_sm()
            .child(
                icon_button("sidebar", "panel-left", "Sidebar (F4)", self.sidebar.tab.is_some(), ready, theme)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(Tab::Thumbnails, cx))),
            )
            .child(icon_button("open", "folder-open", "Open… (Ctrl+O)", false, true, theme).on_click(cx.listener(|this, _, window, cx| this.prompt_open(window, cx))))
            .child(icon_button("recent", "clock", "Recent files", self.recent_menu, true, theme).on_click(cx.listener(|this, _, _, cx| {
                this.recent = recent::load();
                this.recent_menu = !this.recent_menu;
                this.menu = None;
                cx.notify();
            })));
        if ready {
            bar = bar
                .child(separator(theme))
                .child(
                    icon_button("previous-page", "chevron-left", "Previous page", false, current > 0, theme)
                        .on_click(cx.listener(move |this, _, window, cx| this.go_to_page(current.saturating_sub(1), window, cx))),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_color(theme.muted)
                        .child(div().w(px(44.)).text_color(theme.text).child(self.page_input.clone()))
                        .child(format!("/ {page_count}")),
                )
                .child(
                    icon_button("next-page", "chevron-right", "Next page", false, current + 1 < page_count, theme)
                        .on_click(cx.listener(move |this, _, window, cx| this.go_to_page(current + 1, window, cx))),
                )
                .child(separator(theme))
                .child(icon_button("zoom-out", "zoom-out", "Zoom out (Ctrl+−)", false, true, theme).on_click(cx.listener(|this, _, window, cx| this.step_zoom(false, window, cx))))
                .child(div().w(px(48.)).flex().justify_center().text_color(theme.muted).child(zoom_label))
                .child(icon_button("zoom-in", "zoom-in", "Zoom in (Ctrl++)", false, true, theme).on_click(cx.listener(|this, _, window, cx| this.step_zoom(true, window, cx))))
                .child(
                    icon_button("fit-width", "move-horizontal", "Fit width (Ctrl+1)", self.zoom == ZoomMode::FitWidth, true, theme)
                        .on_click(cx.listener(|this, _, window, cx| this.set_zoom(ZoomMode::FitWidth, None, window, cx))),
                )
                .child(
                    icon_button("fit-page", "maximize-2", "Fit page (Ctrl+2)", self.zoom == ZoomMode::FitPage, true, theme)
                        .on_click(cx.listener(|this, _, window, cx| this.set_zoom(ZoomMode::FitPage, None, window, cx))),
                );
        }
        bar = bar.child(div().flex_1());
        if ready {
            let menu_button = |id: &'static str, icon_name: &'static str, label: &'static str, kind: MenuKind| {
                let active = self.menu == Some(kind);
                text_button(id, Some(icon_name), label, theme)
                    .relative()
                    .when(active, |b| b.bg(theme.active).child(self.menu_panel(kind, cx)))
                    .child(svg().path(icon("chevron-down")).size(px(12.)).text_color(theme.muted))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_menu(kind, cx)))
            };
            let editing = self.editing;
            let annotations = icon_button(
                "annotations",
                if self.show_annotations { "message-square" } else { "message-square-off" },
                if self.show_annotations { "Hide annotations" } else { "Show annotations" },
                !self.show_annotations,
                true,
                theme,
            )
            .on_click(cx.listener(|this, _, window, cx| this.toggle_annotations(window, cx)));
            let find = icon_button("find", "search", "Find (Ctrl+F)", self.search.open, true, theme)
                .on_click(cx.listener(|this, _, window, cx| if this.search.open { this.close_search(window, cx) } else { this.open_search(window, cx) }));
            // In edit mode, Print is in the Document menu: the bar has no room left.
            let print = icon_button("print", "printer", "Print (Ctrl+P)", self.printing.is_some(), true, theme)
                .on_click(cx.listener(|this, _, window, cx| this.print(window, cx)));
            if editing {
                bar = bar
                    .child(menu_button("pages-menu", "layers", "Pages", MenuKind::Pages))
                    .child(menu_button("document-menu", "file-text", "Document", MenuKind::Document))
                    .child(annotations)
                    .child(find)
                    .child(separator(theme))
                    .child(
                        text_button("done-editing", Some("check"), "Done", theme)
                            .tooltip(crate::chrome::tooltip("Back to reading; changes stay until you save or close", theme))
                            .on_click(cx.listener(|this, _, window, cx| this.set_editing(false, window, cx))),
                    )
                    .child(primary_button("save", "Save", modified, theme).on_click(cx.listener(|this, _, window, cx| this.save(window, cx))));
            } else {
                bar = bar.child(print).child(annotations).child(find).child(div().w(px(4.)));
                if modified {
                    bar = bar.child(text_button("save", Some("save"), "Save", theme).on_click(cx.listener(|this, _, window, cx| this.save(window, cx))));
                }
                bar = bar.child(
                    primary_button("edit", "Edit", true, theme)
                        .gap(px(6.))
                        .child(svg().path(icon("pencil")).size(px(14.)).text_color(theme.on_accent))
                        .tooltip(crate::chrome::tooltip("Annotate, sign, organize pages and use the document tools", theme))
                        .on_click(cx.listener(|this, _, window, cx| this.set_editing(true, window, cx))),
                );
            }
        }
        bar.into_any_element()
    }

    /// The tool bar: tools in groups, then the colour and width of new marks.
    pub(super) fn tool_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.open()?.info.as_ref()?;
        let theme = self.theme;
        let mut bar = div()
            .flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .gap(px(2.))
            .min_h(px(40.))
            .px(px(8.))
            .py(px(4.))
            .bg(theme.toolbar)
            .border_b_1()
            .border_color(theme.border)
            .text_color(theme.text)
            .text_sm();
        let (can_undo, can_redo) = self.open().map_or((false, false), |open| (open.document.can_undo(), open.document.can_redo()));
        bar = bar
            .child(icon_button("undo", "undo-2", "Undo (Ctrl+Z)", false, can_undo, theme).on_click(cx.listener(|this, _, window, cx| this.undo(window, cx))))
            .child(icon_button("redo", "redo-2", "Redo (Ctrl+Y)", false, can_redo, theme).on_click(cx.listener(|this, _, window, cx| this.redo(window, cx))))
            .child(separator(theme));
        for (n, group) in TOOL_GROUPS.iter().enumerate() {
            if n > 0 {
                bar = bar.child(separator(theme));
            }
            for &tool in *group {
                bar = bar.child(
                    keyed_icon_button(tool.name(), tool_icon(tool), tool.label(), tool.key(), self.tool == tool, true, theme)
                        .w(px(25.))
                        .on_click(cx.listener(move |viewer, _, _, cx| viewer.set_tool(tool, cx))),
                );
            }
        }
        bar = bar.child(div().flex_1());
        for (n, color) in COLORS.into_iter().enumerate() {
            let active = self.picked_style().map_or(self.inspector.color, |style| style.0) == color;
            bar = bar.child(
                div()
                    .id(("color", n))
                    .w(px(22.))
                    .h(px(CONTROL))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(theme.radius))
                    .cursor_pointer()
                    .hover(move |s| s.bg(theme.hover))
                    .child(
                        div()
                            .size(px(16.))
                            .rounded(px(8.))
                            .bg(gpui::Rgba { r: color.r, g: color.g, b: color.b, a: 1.0 })
                            .when(active, |b| b.border_2().border_color(theme.highlight))
                            .when(!active, |b| b.border_1().border_color(theme.input_border)),
                    )
                    .on_click(cx.listener(move |viewer, _, window, cx| {
                        viewer.inspector.color = color;
                        viewer.restyle_picked(|annotation, _| annotation.color = color, window, cx);
                        cx.notify();
                    })),
            );
        }
        bar = bar.child(separator(theme));
        // Text boxes take a font and a size; everything else a line width.
        if self.tool == Tool::Text || self.picked_text().is_some() {
            let (size_now, font_now) = self.picked_text().unwrap_or((self.inspector.size, self.inspector.font));
            // One button goes through the fonts, showing the current one.
            let at = FONTS.iter().position(|(font, _)| *font == font_now).unwrap_or(0);
            let (next, _) = FONTS[(at + 1) % FONTS.len()];
            bar = bar.child(
                text_choice(("font", 0), "Aa", false, theme)
                    .min_w(px(22.))
                    .px(px(2.))
                    .when(font_now == Font::TimesRoman, |b| b.font_family("Times New Roman"))
                    .when(font_now == Font::Courier, |b| b.font_family("Courier New"))
                    .text_sm()
                    .tooltip(crate::chrome::tooltip(FONTS[at].1, theme))
                    .on_click(cx.listener(move |viewer, _, window, cx| {
                        viewer.inspector.font = next;
                        viewer.restyle_picked(|annotation, geometry| retext(annotation, geometry, None, Some(next)), window, cx);
                        cx.notify();
                    })),
            );
            // A smaller and a larger size around the current one.
            let step = |down: bool| {
                let sizes = SIZES.iter().copied();
                if down { sizes.rev().find(|&size| size < size_now) } else { sizes.into_iter().find(|&size| size > size_now) }
            };
            for (n, (glyph, size)) in [("−", step(true)), ("+", step(false))].into_iter().enumerate() {
                let mut button = text_choice(("size", n), glyph, false, theme).tooltip(crate::chrome::tooltip("Text size", theme));
                match size {
                    Some(size) => {
                        button = button.on_click(cx.listener(move |viewer, _, window, cx| {
                            viewer.inspector.size = size;
                            viewer.restyle_picked(|annotation, geometry| retext(annotation, geometry, Some(size), None), window, cx);
                            cx.notify();
                        }));
                    }
                    None => button = button.opacity(0.4).cursor_default(),
                }
                if n == 1 {
                    bar = bar.child(div().min_w(px(16.)).flex().justify_center().text_xs().child(format!("{size_now}")));
                }
                bar = bar.child(button.min_w(px(16.)).px(px(2.)));
            }
            return Some(bar.into_any_element());
        }
        for (n, width) in WIDTHS.into_iter().enumerate() {
            let active = self.picked_style().map_or(self.inspector.width, |style| style.1) == width;
            bar = bar.child(
                div()
                    .id(("width", n))
                    .w(px(24.))
                    .h(px(CONTROL))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(theme.radius))
                    .cursor_pointer()
                    .when(active, |b| b.bg(theme.active))
                    .hover(move |s| s.bg(theme.hover))
                    .tooltip(crate::chrome::tooltip("Line width", theme))
                    .child(div().w(px(14.)).h(px(width.max(1.0))).rounded(px(1.)).bg(if active { theme.highlight } else { theme.icon }))
                    .on_click(cx.listener(move |viewer, _, window, cx| {
                        viewer.inspector.width = width;
                        viewer.restyle_picked(|annotation, _| annotation.width = width, window, cx);
                        cx.notify();
                    })),
            );
        }
        Some(bar.into_any_element())
    }

    /// The panel of `kind`, dropping down from where it is placed (under its button), kept
    /// inside the window.
    fn menu_panel(&self, kind: MenuKind, cx: &Context<Self>) -> AnyElement {
        let theme = self.theme;
        let entries = match kind {
            MenuKind::Pages => pages_menu(),
            MenuKind::Document => document_menu(),
        };
        let mut panel = div()
            .id("menu")
            .occlude()
            .w(px(260.))
            .p(px(4.))
            .flex()
            .flex_col()
            .rounded(px(8.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.toolbar)
            .shadow_lg()
            .text_sm()
            .text_color(theme.text)
            .on_mouse_down_out(cx.listener(move |viewer, _, _, cx| {
                viewer.menu_closed = Some((kind, std::time::Instant::now()));
                viewer.menu = None;
                cx.notify();
            }));
        for (n, entry) in entries.into_iter().enumerate() {
            let Some((icon_name, label, keys, action)) = entry else {
                panel = panel.child(div().h(px(1.)).my(px(4.)).mx(px(6.)).bg(theme.separator));
                continue;
            };
            let keys: SharedString = if keys.is_empty() { SharedString::default() } else { shortcut(keys).into() };
            panel = panel.child(
                div()
                    .id(("menu-item", n))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .h(px(30.))
                    .px(px(8.))
                    .rounded(px(theme.radius))
                    .cursor_pointer()
                    .hover(move |s| s.bg(theme.hover))
                    .child(svg().path(icon(icon_name)).size(px(ICON)).text_color(theme.icon))
                    .child(div().flex_1().child(t(label)))
                    .child(div().text_xs().text_color(theme.muted).child(keys))
                    .on_click(cx.listener(move |viewer, _, window, cx| {
                        viewer.menu = None;
                        action(viewer, window, cx);
                        cx.notify();
                    })),
            );
        }
        div()
            .absolute()
            .top(px(CONTROL + 6.))
            .left_0()
            .child(deferred(anchored().snap_to_window_with_margin(px(8.)).child(panel)).with_priority(1))
            .into_any_element()
    }
}

impl Viewer {
    /// Enters or leaves edit mode. Leaving keeps the changes (until saved or closed) and goes
    /// back to selecting text.
    pub(super) fn set_editing(&mut self, editing: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = editing;
        self.menu = None;
        if !editing {
            self.set_tool(Tool::Select, cx);
            self.rail.clear();
            window.focus(&self.focus);
        }
        cx.notify();
    }
}

/// A tool bar choice showing `label`, highlighted when `active`.
fn text_choice(id: (&'static str, usize), label: impl Into<gpui::SharedString>, active: bool, theme: crate::theme::Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .min_w(px(28.))
        .px(px(5.))
        .h(px(CONTROL))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme.radius))
        .cursor_pointer()
        .text_xs()
        .text_color(if active { theme.highlight } else { theme.text })
        .when(active, |b| b.bg(theme.active))
        .hover(move |s| s.bg(theme.hover))
        .child(label.into())
}

/// A text box with a new `size` and/or `font`, its box fitted to the text from the same
/// top-left corner.
fn retext(annotation: &mut Annotation, geometry: &PageGeometry, new_size: Option<f32>, new_font: Option<Font>) {
    let Kind::Text { rect, text, size, font, .. } = &mut annotation.kind else { return };
    *size = new_size.unwrap_or(*size);
    *font = new_font.unwrap_or(*font);
    let shown = geometry.rect_to_display(*rect);
    *rect = geometry.rect_to_user(text_area(text, *size, *font, Point { x: shown.x, y: shown.y }));
}
