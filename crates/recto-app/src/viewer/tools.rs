//! Document tools in one modal form each: watermark, page numbers, header or footer (all three
//! are stamps), document properties, and export with save options (compression, grayscale,
//! flattening, password).

use gpui::{AnyElement, Context, Entity, IntoElement, ParentElement, SharedString, Styled, Window, div, prelude::*, px};
use recto_core::BuildOptions;
use recto_core::meta::Metadata;
use recto_core::op::Op;
use recto_core::security::{Allowed, Protection, Security};
use recto_core::stamp::{Position, Stamp};

use super::Viewer;
use crate::i18n::t;
use crate::text_input::{TextInput, TextInputEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    Watermark,
    PageNumbers,
    HeaderFooter,
    Properties,
    Export,
    /// A rubber stamp placed where the page was clicked.
    Stamp,
}

/// Toggles of the forms, by meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flag {
    Diagonal,
    SelectedOnly,
    Compress,
    Grayscale,
    Flatten,
    Protect,
    RemovePassword,
    AllowPrint,
    AllowCopy,
    AllowModify,
    AllowAnnotate,
}

impl Flag {
    fn label(self) -> &'static str {
        match self {
            Flag::Diagonal => "Diagonal",
            Flag::SelectedOnly => "Selected pages only",
            Flag::Compress => "Compress images",
            Flag::Grayscale => "Grayscale",
            Flag::Flatten => "Flatten form fields",
            Flag::Protect => "Protect with a password",
            Flag::RemovePassword => "Remove the password",
            Flag::AllowPrint => "Allow printing",
            Flag::AllowCopy => "Allow copying text",
            Flag::AllowModify => "Allow changes",
            Flag::AllowAnnotate => "Allow comments and form filling",
        }
    }

    /// Permission toggles only show while protecting.
    fn needs_protect(self) -> bool {
        matches!(self, Flag::AllowPrint | Flag::AllowCopy | Flag::AllowModify | Flag::AllowAnnotate)
    }
}

/// Text fields of the forms, by meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Text,
    Size,
    Opacity,
    FirstNumber,
    Title,
    Author,
    Subject,
    Keywords,
    OpenPassword,
    OwnerPassword,
}

impl Field {
    fn label(self) -> &'static str {
        match self {
            Field::Text => "Text",
            Field::Size => "Size (pt)",
            Field::Opacity => "Opacity (%)",
            Field::FirstNumber => "First number",
            Field::Title => "Title",
            Field::Author => "Author",
            Field::Subject => "Subject",
            Field::Keywords => "Keywords",
            Field::OpenPassword => "Password to open",
            Field::OwnerPassword => "Password to change permissions",
        }
    }
}

const POSITIONS: [(Position, &str); 7] = [
    (Position::TopLeft, "Top left"),
    (Position::TopCenter, "Top center"),
    (Position::TopRight, "Top right"),
    (Position::Center, "Center"),
    (Position::BottomLeft, "Bottom left"),
    (Position::BottomCenter, "Bottom center"),
    (Position::BottomRight, "Bottom right"),
];

/// Initial fields, toggles and position of a form.
type Form = (Vec<(Field, String)>, Vec<(Flag, bool)>, Position);

pub struct ToolDialog {
    kind: ToolKind,
    fields: Vec<(Field, Entity<TextInput>)>,
    flags: Vec<(Flag, bool)>,
    position: Position,
    /// For stamps: the displayed page and the point clicked.
    at: Option<(usize, recto_core::geom::Point)>,
    error: Option<SharedString>,
}

impl ToolDialog {
    fn flag(&self, flag: Flag) -> bool {
        self.flags.iter().any(|(f, on)| *f == flag && *on)
    }

    fn text(&self, field: Field, cx: &gpui::App) -> String {
        self.fields.iter().find(|(f, _)| *f == field).map(|(_, input)| input.read(cx).text().to_owned()).unwrap_or_default()
    }
}

fn number(text: &str, field: Field, low: f32, high: f32) -> Result<f32, String> {
    text.trim()
        .replace(',', ".")
        .parse::<f32>()
        .ok()
        .filter(|value| (low..=high).contains(value))
        .ok_or_else(|| format!("{}: {} {low}–{high}", t(field.label()), t("enter a number between")))
}

impl Viewer {
    pub(super) fn open_tool(&mut self, kind: ToolKind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.open() else { return };
        let metadata = open.document.metadata().unwrap_or_default();
        let has_fields = !open.fields.is_empty();
        let protected = open.document.password().is_some();
        let (fields, flags, position): Form = match kind {
            ToolKind::Watermark => (
                vec![(Field::Text, t("CONFIDENTIAL").to_owned()), (Field::Size, "72".into()), (Field::Opacity, "35".into())],
                vec![(Flag::Diagonal, true), (Flag::SelectedOnly, false)],
                Position::Center,
            ),
            ToolKind::PageNumbers => (
                vec![(Field::Text, "{page} / {pages}".into()), (Field::Size, "10".into()), (Field::FirstNumber, "1".into())],
                vec![(Flag::SelectedOnly, false)],
                Position::BottomCenter,
            ),
            ToolKind::HeaderFooter => {
                (vec![(Field::Text, String::new()), (Field::Size, "10".into())], vec![(Flag::SelectedOnly, false)], Position::TopCenter)
            }
            ToolKind::Properties => (
                vec![
                    (Field::Title, metadata.title),
                    (Field::Author, metadata.author),
                    (Field::Subject, metadata.subject),
                    (Field::Keywords, metadata.keywords),
                ],
                Vec::new(),
                Position::Center,
            ),
            ToolKind::Stamp => (vec![(Field::Text, t(STAMPS[0]).to_owned())], Vec::new(), Position::Center),
            ToolKind::Export => {
                let mut flags = vec![(Flag::Compress, true), (Flag::Grayscale, false)];
                if has_fields {
                    flags.push((Flag::Flatten, false));
                }
                if protected {
                    flags.push((Flag::RemovePassword, false));
                }
                flags.extend([
                    (Flag::Protect, false),
                    (Flag::AllowPrint, true),
                    (Flag::AllowCopy, true),
                    (Flag::AllowModify, true),
                    (Flag::AllowAnnotate, true),
                ]);
                (vec![(Field::OpenPassword, String::new()), (Field::OwnerPassword, String::new())], flags, Position::Center)
            }
        };
        let theme = self.theme;
        let fields: Vec<(Field, Entity<TextInput>)> = fields
            .into_iter()
            .map(|(field, value)| {
                let input = cx.new(|cx| {
                    let input = TextInput::new(t(field.label()), |_| true, theme, cx);
                    let mut input = if matches!(field, Field::OpenPassword | Field::OwnerPassword) { input.masked() } else { input };
                    input.set_text(value, cx);
                    input
                });
                cx.subscribe_in(&input, window, |viewer, _, event, window, cx| match event {
                    TextInputEvent::Commit { .. } => viewer.submit_tool(window, cx),
                    TextInputEvent::Cancel => viewer.close_tool(window, cx),
                    TextInputEvent::Changed(_) => {}
                })
                .detach();
                (field, input)
            })
            .collect();
        if let Some((_, first)) = fields.iter().find(|(field, _)| !matches!(field, Field::OpenPassword | Field::OwnerPassword)) {
            first.update(cx, |input, cx| input.focus_all(window, cx));
        }
        self.tool_dialog = Some(ToolDialog { kind, fields, flags, position, at: None, error: None });
        cx.notify();
    }

    fn close_tool(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tool_dialog = None;
        window.focus(&self.focus);
        cx.notify();
    }

    fn submit_tool(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = &self.tool_dialog else { return };
        let result = match dialog.kind {
            ToolKind::Watermark | ToolKind::PageNumbers | ToolKind::HeaderFooter => self.stamp_from(dialog, cx).map(|stamp| {
                self.edit(Op::AddStamp(stamp), window, cx);
            }),
            ToolKind::Properties => {
                let metadata = Metadata {
                    title: dialog.text(Field::Title, cx).trim().to_owned(),
                    author: dialog.text(Field::Author, cx).trim().to_owned(),
                    subject: dialog.text(Field::Subject, cx).trim().to_owned(),
                    keywords: dialog.text(Field::Keywords, cx).trim().to_owned(),
                };
                let current = self.open().and_then(|open| open.document.metadata().ok()).unwrap_or_default();
                if metadata != current {
                    self.edit(Op::SetMetadata(metadata), window, cx);
                }
                Ok(())
            }
            ToolKind::Export => self.export_options(dialog, cx).map(|options| self.export(options, window, cx)),
            ToolKind::Stamp => {
                let (label, at) = (dialog.text(Field::Text, cx), dialog.at);
                self.place_stamp(label, at, window, cx)
            }
        };
        match result {
            Ok(()) => self.close_tool(window, cx),
            Err(error) => {
                if let Some(dialog) = &mut self.tool_dialog {
                    dialog.error = Some(error.into());
                }
                cx.notify();
            }
        }
    }

    fn stamp_from(&self, dialog: &ToolDialog, cx: &gpui::App) -> Result<Stamp, String> {
        let text = dialog.text(Field::Text, cx);
        if text.trim().is_empty() {
            return Err(t("Enter the text to show.").to_owned());
        }
        let size = number(&dialog.text(Field::Size, cx), Field::Size, 4.0, 300.0)?;
        let mut stamp = match dialog.kind {
            ToolKind::Watermark => {
                let opacity = number(&dialog.text(Field::Opacity, cx), Field::Opacity, 1.0, 100.0)? / 100.0;
                let angle = if dialog.flag(Flag::Diagonal) { 45.0 } else { 0.0 };
                Stamp { size, opacity, angle, ..Stamp::watermark(&text) }
            }
            _ => Stamp { text, size, ..Stamp::page_numbers() },
        };
        stamp.position = dialog.position;
        if dialog.kind == ToolKind::PageNumbers {
            stamp.first_number = number(&dialog.text(Field::FirstNumber, cx), Field::FirstNumber, 0.0, 1.0e6)? as u32;
        }
        if dialog.flag(Flag::SelectedOnly) {
            stamp.pages = Some(self.target_pages());
        }
        Ok(stamp)
    }

    fn export_options(&self, dialog: &ToolDialog, cx: &gpui::App) -> Result<BuildOptions, String> {
        let security = if dialog.flag(Flag::Protect) {
            let (user, owner) = (dialog.text(Field::OpenPassword, cx), dialog.text(Field::OwnerPassword, cx));
            if user.is_empty() && owner.is_empty() {
                return Err(t("Enter a password.").to_owned());
            }
            let allowed = Allowed {
                print: dialog.flag(Flag::AllowPrint),
                copy: dialog.flag(Flag::AllowCopy),
                modify: dialog.flag(Flag::AllowModify),
                annotate: dialog.flag(Flag::AllowAnnotate),
            };
            Security::Protect(Protection { user_password: user, owner_password: owner, allowed })
        } else if dialog.flag(Flag::RemovePassword) {
            Security::Remove
        } else {
            Security::Keep
        };
        Ok(BuildOptions {
            flatten_forms: dialog.flag(Flag::Flatten),
            grayscale: dialog.flag(Flag::Grayscale),
            compress: dialog.flag(Flag::Compress),
            security,
        })
    }

    pub(super) fn tool_view(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dialog = self.tool_dialog.as_ref()?;
        let theme = self.theme;
        let (title, help) = match dialog.kind {
            ToolKind::Watermark => ("Watermark", "Text across the pages."),
            ToolKind::PageNumbers => ("Page numbers", "{page} is the page number, {pages} the page count."),
            ToolKind::HeaderFooter => ("Header or footer", "A line of text on each page. {page} and {pages} are replaced."),
            ToolKind::Properties => ("Document properties", "Shown by readers and file browsers."),
            ToolKind::Export => ("Export", "Saves a copy with these options; the open document does not change."),
            ToolKind::Stamp => ("Stamp", "Click a stamp, or type your own text."),
        };
        let chip = |id: SharedString, label: &'static str, active: bool| {
            div()
                .id(id)
                .px(px(8.))
                .py(px(3.))
                .rounded(px(4.))
                .cursor_pointer()
                .border_1()
                .border_color(theme.border)
                .when(active, |b| b.bg(theme.selection))
                .hover(move |s| s.bg(theme.hover))
                .child(t(label))
        };
        let mut card = div()
            .id("tool-dialog")
            .w(px(440.))
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .rounded(px(8.))
            .bg(theme.toolbar)
            .border_1()
            .border_color(theme.border)
            .shadow_lg()
            .text_color(theme.text)
            .text_sm()
            .child(div().text_base().font_weight(gpui::FontWeight::SEMIBOLD).child(t(title)))
            .child(div().text_color(theme.muted).child(t(help)));
        let protecting = dialog.flag(Flag::Protect);
        for (field, input) in &dialog.fields {
            if matches!(field, Field::OpenPassword | Field::OwnerPassword) && !protecting {
                continue;
            }
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().w(px(150.)).flex_none().text_color(theme.muted).child(t(field.label())))
                    .child(div().flex_1().child(input.clone())),
            );
        }
        if dialog.kind == ToolKind::Stamp {
            let mut presets = div().flex().flex_wrap().gap(px(6.));
            for (n, label) in STAMPS.into_iter().enumerate() {
                presets = presets.child(chip(format!("stamp-{n}").into(), label, false).on_click(cx.listener(move |viewer, _, window, cx| {
                    let input = viewer.tool_dialog.as_ref().and_then(|dialog| dialog.fields.first().map(|(_, input)| input.clone()));
                    if let Some(input) = input {
                        input.update(cx, |input, cx| input.set_text(t(label), cx));
                    }
                    viewer.submit_tool(window, cx);
                })));
            }
            card = card.child(presets);
        }
        if matches!(dialog.kind, ToolKind::Watermark | ToolKind::PageNumbers | ToolKind::HeaderFooter) {
            // Three rows, as the positions sit on the page: top, middle, bottom.
            let mut grid = div().flex().flex_col().gap(px(4.));
            for row in [&POSITIONS[0..3], &POSITIONS[3..4], &POSITIONS[4..7]] {
                let mut line = div().flex().gap(px(4.));
                for &(position, label) in row {
                    let n = POSITIONS.iter().position(|(p, _)| *p == position).unwrap_or(0);
                    line = line.child(chip(format!("position-{n}").into(), label, dialog.position == position).flex_1().flex().justify_center().on_click(
                        cx.listener(move |viewer, _, _, cx| {
                            if let Some(dialog) = &mut viewer.tool_dialog {
                                dialog.position = position;
                            }
                            cx.notify();
                        }),
                    ));
                }
                grid = grid.child(line);
            }
            card = card.child(div().text_color(theme.muted).child(t("Position"))).child(grid);
        }
        for (n, &(flag, on)) in dialog.flags.iter().enumerate() {
            if flag.needs_protect() && !protecting {
                continue;
            }
            card = card.child(
                div()
                    .id(("tool-flag", n))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .cursor_pointer()
                    .when(flag.needs_protect(), |row| row.pl(px(20.)))
                    .child(
                        div()
                            .size(px(16.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .border_1()
                            .border_color(if on { theme.highlight } else { theme.input_border })
                            .when(on, |b| b.bg(theme.highlight).child(gpui::svg().path(crate::assets::icon("check")).size(px(12.)).text_color(theme.on_accent))),
                    )
                    .child(t(flag.label()))
                    .on_click(cx.listener(move |viewer, _, _, cx| {
                        if let Some(dialog) = &mut viewer.tool_dialog {
                            if let Some(entry) = dialog.flags.get_mut(n) {
                                entry.1 = !entry.1;
                            }
                            // Protecting and removing the password exclude each other.
                            let (protect, remove) = (dialog.flag(Flag::Protect), dialog.flag(Flag::RemovePassword));
                            if protect && remove {
                                let other = if flag == Flag::Protect { Flag::RemovePassword } else { Flag::Protect };
                                for entry in &mut dialog.flags {
                                    if entry.0 == other {
                                        entry.1 = false;
                                    }
                                }
                            }
                        }
                        cx.notify();
                    })),
            );
        }
        if let Some(error) = &dialog.error {
            card = card.child(div().text_color(theme.error).child(error.clone()));
        }
        let button = |id: &'static str, label: &'static str, primary: bool| {
            div()
                .id(id)
                .px(px(12.))
                .py(px(5.))
                .rounded(px(5.))
                .cursor_pointer()
                .when(primary, |b| b.bg(theme.accent).text_color(theme.on_accent))
                .when(!primary, |b| b.border_1().border_color(theme.border).hover(move |s| s.bg(theme.hover)))
                .child(t(label))
        };
        let confirm = if dialog.kind == ToolKind::Export { "Export…" } else { "Apply" };
        card = card.child(
            div()
                .flex()
                .justify_end()
                .gap(px(8.))
                .child(button("tool-cancel", "Cancel", false).on_click(cx.listener(|viewer, _, window, cx| viewer.close_tool(window, cx))))
                .child(button("tool-ok", confirm, true).on_click(cx.listener(|viewer, _, window, cx| viewer.submit_tool(window, cx)))),
        );
        Some(
            div()
                .id("tool-backdrop")
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .pt(px(90.))
                .items_start()
                .bg(gpui::rgba(0x0000_0055))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(card)
                .into_any_element(),
        )
    }
}

/// Ready-made stamps (translated when shown).
const STAMPS: [&str; 8] = ["APPROVED", "DRAFT", "CONFIDENTIAL", "PAID", "RECEIVED", "COPY", "URGENT", "VOID"];

impl Viewer {
    /// Opens the stamp form for a stamp at displayed point `at` of page `page`.
    pub(super) fn open_stamp(&mut self, page: usize, at: recto_core::geom::Point, window: &mut Window, cx: &mut Context<Self>) {
        self.open_tool(ToolKind::Stamp, window, cx);
        if let Some(dialog) = &mut self.tool_dialog {
            dialog.at = Some((page, at));
        }
    }

    fn place_stamp(&mut self, label: String, at: Option<(usize, recto_core::geom::Point)>, window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let label = label.trim().to_uppercase();
        if label.is_empty() {
            return Err(t("Enter the text to show.").to_owned());
        }
        let Some((page, at)) = at else { return Ok(()) };
        let Some(geometry) = self.open().and_then(|open| open.geometry.get(page).copied()) else { return Ok(()) };
        let Some(id) = self.open().and_then(|open| open.page_states.get(page)).map(|state| state.id) else { return Ok(()) };
        let height = 36.0;
        let width = recto_core::annot::text_width(&label, height * 0.5, recto_core::annot::Font::HelveticaBold) + height;
        let area = recto_core::geom::PageRect { x: at.x, y: at.y, width, height };
        let kind = recto_core::annot::Kind::Label { rect: geometry.rect_to_user(area), label, upright: geometry.rotate };
        let annotation = recto_core::annot::Annotation { kind, color: self.inspector.color, width: 2.0 };
        self.edit(Op::AddAnnotation { page: id, annotation }, window, cx);
        Ok(())
    }
}
