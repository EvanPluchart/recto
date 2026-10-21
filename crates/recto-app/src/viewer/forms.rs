//! Form filling: clickable areas over the widgets of the shown pages. Check boxes and radio
//! buttons toggle on click; text and choice fields open an editor in place. Values become
//! `Op::FillField`, so they follow undo, redo and save like any edit.

use gpui::{AnyElement, Context, Entity, IntoElement, MouseButton, ParentElement, SharedString, Styled, Window, div, prelude::*, px};
use recto_core::form::FieldKind;
use recto_core::geom::PageRect;
use recto_core::op::Op;

use super::Viewer;
use super::annotate::Tool;
use crate::i18n::t;
use crate::text_input::{TextInput, TextInputEvent};

pub struct FieldEditor {
    name: String,
    page: usize,
    area: PageRect,
    input: Entity<TextInput>,
    options: Vec<String>,
}

impl Viewer {
    /// Widgets on displayed page `index`: (field, widget, rectangle on the displayed page).
    fn widgets_on(&self, index: usize) -> Vec<(usize, usize, PageRect)> {
        let Some(open) = self.open() else { return Vec::new() };
        let (Some(state), Some(geometry)) = (open.page_states.get(index), open.geometry.get(index)) else { return Vec::new() };
        let mut found = Vec::new();
        for (f, field) in open.fields.iter().enumerate() {
            if field.read_only || field.kind == FieldKind::Other {
                continue;
            }
            for (w, widget) in field.widgets.iter().enumerate() {
                if widget.page == state.id {
                    found.push((f, w, geometry.rect_to_display(widget.rect)));
                }
            }
        }
        found
    }

    fn fill_field(&mut self, name: String, value: String, window: &mut Window, cx: &mut Context<Self>) {
        self.edit(Op::FillField { name, value }, window, cx);
    }

    fn click_field(&mut self, field: usize, widget: usize, page: usize, area: PageRect, window: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = self.open().and_then(|open| open.fields.get(field)).cloned() else { return };
        match &field.kind {
            FieldKind::CheckBox => {
                let on = field.widgets.get(widget).and_then(|w| w.on_state.clone()).unwrap_or_else(|| "Yes".into());
                let value = if field.value.is_empty() { on } else { String::new() };
                self.fill_field(field.name, value, window, cx);
            }
            FieldKind::Radio => {
                if let Some(on) = field.widgets.get(widget).and_then(|w| w.on_state.clone())
                    && on != field.value
                {
                    self.fill_field(field.name, on, window, cx);
                }
            }
            FieldKind::Text | FieldKind::Choice { .. } => {
                let theme = self.theme;
                let value = field.value.clone();
                let input = cx.new(|cx| {
                    let mut input = TextInput::new("", |_| true, theme, cx);
                    input.set_text(value, cx);
                    input
                });
                cx.subscribe_in(&input, window, |viewer, _, event, window, cx| match event {
                    TextInputEvent::Commit { text, .. } => {
                        if let Some(editor) = viewer.field_editor.take() {
                            window.focus(&viewer.focus);
                            viewer.fill_field(editor.name, text.clone(), window, cx);
                        }
                    }
                    TextInputEvent::Cancel => {
                        viewer.field_editor = None;
                        window.focus(&viewer.focus);
                        cx.notify();
                    }
                    TextInputEvent::Changed(_) => {}
                })
                .detach();
                input.update(cx, |input, cx| input.focus_all(window, cx));
                let options = match &field.kind {
                    FieldKind::Choice { options, .. } => options.clone(),
                    _ => Vec::new(),
                };
                self.field_editor = Some(FieldEditor { name: field.name, page, area, input, options });
                cx.notify();
            }
            FieldKind::Other => {}
        }
    }

    /// Clickable widget areas and the open field editor over displayed page `index`.
    pub(super) fn form_overlay(&self, index: usize, origin: (f32, f32), zoom: f32, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if self.tool != Tool::Select {
            return Vec::new();
        }
        let theme = self.theme;
        let mut elements: Vec<AnyElement> = self
            .widgets_on(index)
            .into_iter()
            .map(|(field, widget, area)| {
                div()
                    .id(SharedString::from(format!("field-{field}-{widget}")))
                    .absolute()
                    .left(px(origin.0 + area.x * zoom))
                    .top(px(origin.1 + area.y * zoom))
                    .w(px(area.width * zoom))
                    .h(px(area.height * zoom))
                    .border_1()
                    .border_color(gpui::rgba(0x4c8d_ff55))
                    .bg(gpui::rgba(0x4c8d_ff14))
                    .hover(move |style| style.border_color(theme.accent))
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, cx.listener(move |viewer, _, window, cx| {
                        cx.stop_propagation();
                        viewer.click_field(field, widget, index, area, window, cx);
                    }))
                    .into_any_element()
            })
            .collect();
        if let Some(editor) = self.field_editor.as_ref().filter(|editor| editor.page == index) {
            let (x, y) = (origin.0 + editor.area.x * zoom, origin.1 + editor.area.y * zoom);
            let options = editor.options.iter().map(|option| {
                let (name, value) = (editor.name.clone(), option.clone());
                div()
                    .id(SharedString::from(format!("option-{option}")))
                    .px(px(8.))
                    .py(px(3.))
                    .cursor_pointer()
                    .hover(move |style| style.bg(theme.hover))
                    .child(option.clone())
                    .on_mouse_down(MouseButton::Left, cx.listener(move |viewer, _, window, cx| {
                        cx.stop_propagation();
                        viewer.field_editor = None;
                        window.focus(&viewer.focus);
                        viewer.fill_field(name.clone(), value.clone(), window, cx);
                    }))
            });
            elements.push(
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px((editor.area.width * zoom).max(160.)))
                    .flex()
                    .flex_col()
                    .bg(theme.field)
                    .shadow_md()
                    .text_sm()
                    .text_color(theme.text)
                    .child(editor.input.clone())
                    .when(!editor.options.is_empty(), |list| list.children(options))
                    .child(div().px(px(6.)).py(px(2.)).text_xs().text_color(theme.muted).child(t("Enter to confirm, Esc to cancel")))
                    .into_any_element(),
            );
        }
        elements
    }
}
