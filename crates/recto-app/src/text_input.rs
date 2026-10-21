//! Single-line text field driven by key-down events (no IME composition yet).
//! Used for the page number and, later, the search box.

use gpui::{
    ClipboardItem, Context, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent, ParentElement, Render,
    SharedString, Styled, Window, div, prelude::*, px,
};

use crate::theme::Theme;

pub enum TextInputEvent {
    /// The text was edited.
    Changed(String),
    /// Enter was pressed, with or without Shift.
    Commit { text: String, shift: bool },
    /// Escape was pressed.
    Cancel,
}

/// The editing state, kept apart from GPUI so it can be tested.
#[derive(Debug)]
struct Buffer {
    text: String,
    /// Cursor position, in chars.
    cursor: usize,
    /// Everything is selected (after focus or Ctrl+A): typing replaces the text.
    all_selected: bool,
    /// Characters accepted, e.g. digits only for a page number.
    accept: fn(char) -> bool,
}

impl Buffer {
    fn new(accept: fn(char) -> bool) -> Self {
        Self { text: String::new(), cursor: 0, all_selected: false, accept }
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    fn byte_index(&self, chars: usize) -> usize {
        self.text.char_indices().nth(chars).map_or(self.text.len(), |(i, _)| i)
    }

    fn set(&mut self, text: String) {
        self.text = text;
        self.cursor = self.len();
    }

    fn select_all(&mut self) {
        self.all_selected = true;
        self.cursor = self.len();
    }

    fn clear_if_selected(&mut self) -> bool {
        if self.all_selected {
            self.text.clear();
            self.cursor = 0;
            self.all_selected = false;
        }
        self.text.is_empty()
    }

    fn insert(&mut self, typed: &str) {
        let typed: String = typed.chars().filter(|&c| (self.accept)(c)).collect();
        if typed.is_empty() {
            return;
        }
        self.clear_if_selected();
        let at = self.byte_index(self.cursor);
        self.text.insert_str(at, &typed);
        self.cursor += typed.chars().count();
    }

    fn delete(&mut self, backward: bool) {
        if self.all_selected {
            self.clear_if_selected();
            return;
        }
        let target = if backward { self.cursor.checked_sub(1) } else { Some(self.cursor) };
        if let Some(target) = target.filter(|&t| t < self.len()) {
            let start = self.byte_index(target);
            let end = self.byte_index(target + 1);
            self.text.replace_range(start..end, "");
            self.cursor = target;
        }
    }

    /// Moves the cursor; any movement ends the select-all state.
    fn move_to(&mut self, cursor: usize) {
        self.cursor = cursor.min(self.len());
        self.all_selected = false;
    }
}

pub struct TextInput {
    buffer: Buffer,
    placeholder: SharedString,
    /// Shows bullets instead of the text, and never copies it.
    masked: bool,
    theme: Theme,
    focus: FocusHandle,
}

impl EventEmitter<TextInputEvent> for TextInput {}

impl TextInput {
    pub fn new(placeholder: impl Into<SharedString>, accept: fn(char) -> bool, theme: Theme, cx: &mut Context<Self>) -> Self {
        Self { buffer: Buffer::new(accept), placeholder: placeholder.into(), masked: false, theme, focus: cx.focus_handle() }
    }

    /// For passwords.
    pub fn masked(mut self) -> Self {
        self.masked = true;
        self
    }

    pub fn text(&self) -> &str {
        &self.buffer.text
    }

    /// Replaces the text and puts the cursor at the end.
    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        let text = text.into();
        if text != self.buffer.text {
            self.buffer.set(text);
            cx.notify();
        }
    }

    /// Focuses the field with its whole content selected.
    pub fn focus_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.buffer.select_all();
        window.focus(&self.focus);
        cx.notify();
    }

    fn on_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        let buffer = &mut self.buffer;
        let len = buffer.len();
        let before = buffer.text.clone();
        match keystroke.key.as_str() {
            "enter" => cx.emit(TextInputEvent::Commit { text: buffer.text.clone(), shift: modifiers.shift }),
            "escape" => cx.emit(TextInputEvent::Cancel),
            "backspace" => buffer.delete(true),
            "delete" => buffer.delete(false),
            "left" => buffer.move_to(if buffer.all_selected { 0 } else { buffer.cursor.saturating_sub(1) }),
            "right" => buffer.move_to(buffer.cursor + 1),
            "home" => buffer.move_to(0),
            "end" => buffer.move_to(len),
            "a" if modifiers.secondary() => buffer.select_all(),
            "c" if modifiers.secondary() && !self.masked => cx.write_to_clipboard(ClipboardItem::new_string(buffer.text.clone())),
            "v" if modifiers.secondary() => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    buffer.insert(&text);
                }
            }
            _ if modifiers.control || modifiers.platform || modifiers.alt => return,
            _ => match keystroke.key_char.as_deref() {
                Some(typed) if !typed.chars().any(char::is_control) => buffer.insert(typed),
                _ => return,
            },
        }
        if self.buffer.text != before {
            cx.emit(TextInputEvent::Changed(self.buffer.text.clone()));
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let focused = self.focus.is_focused(window);
        let buffer = &self.buffer;
        let field = div()
            .key_context("TextInput")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, window, cx| this.focus_all(window, cx)))
            .flex()
            .items_center()
            .h(px(24.))
            .px(px(6.))
            .rounded(px(4.))
            .border_1()
            .border_color(if focused { theme.accent } else { theme.border })
            .bg(theme.field)
            .overflow_hidden();
        if buffer.text.is_empty() && !focused {
            return field.child(div().text_color(theme.muted).child(self.placeholder.clone()));
        }
        let (before, after) = buffer.text.split_at(buffer.byte_index(buffer.cursor));
        let show = |part: &str| if self.masked { "•".repeat(part.chars().count()) } else { part.to_owned() };
        let caret = div().w(px(1.)).h(px(14.)).bg(theme.text);
        let text = div().flex().items_center().whitespace_nowrap();
        if focused && buffer.all_selected && !buffer.text.is_empty() {
            field.child(text.child(div().bg(theme.selection).child(show(&buffer.text))))
        } else if focused {
            field.child(text.child(show(before)).child(caret).child(show(after)))
        } else {
            field.child(text.child(show(&buffer.text)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digits() -> Buffer {
        Buffer::new(|c| c.is_ascii_digit())
    }

    #[test]
    fn typing_replaces_a_full_selection() {
        let mut buffer = digits();
        buffer.set("42".into());
        buffer.select_all();
        buffer.insert("7");
        assert_eq!((buffer.text.as_str(), buffer.cursor, buffer.all_selected), ("7", 1, false));
    }

    #[test]
    fn filters_rejected_characters() {
        let mut buffer = digits();
        buffer.insert("1a2 b3");
        assert_eq!(buffer.text, "123");
        buffer.insert("abc");
        assert_eq!(buffer.text, "123");
    }

    #[test]
    fn edits_at_the_cursor() {
        let mut buffer = Buffer::new(|_| true);
        buffer.insert("héllo");
        buffer.move_to(1);
        buffer.delete(false);
        assert_eq!(buffer.text, "hllo");
        buffer.insert("é");
        buffer.move_to(0);
        buffer.delete(true); // nothing before the cursor
        buffer.move_to(99);
        buffer.delete(true);
        assert_eq!((buffer.text.as_str(), buffer.cursor), ("héll", 4));
    }

    #[test]
    fn delete_clears_a_full_selection() {
        let mut buffer = digits();
        buffer.set("123".into());
        buffer.select_all();
        buffer.delete(true);
        assert_eq!((buffer.text.as_str(), buffer.cursor), ("", 0));
    }
}
