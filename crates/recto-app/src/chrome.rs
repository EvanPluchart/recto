//! Building blocks of the window chrome: icon buttons with tooltips, text buttons, separators.

use gpui::{
    AnyView, App, AppContext, Context, Div, IntoElement, ParentElement, Render, SharedString, Stateful, Styled, Window, div,
    prelude::*, px, svg,
};

use crate::assets::icon;
use crate::i18n::t;
use crate::theme::Theme;

/// Height of a toolbar control.
pub const CONTROL: f32 = 28.0;
pub const ICON: f32 = 16.0;

/// A hover label.
pub struct Tooltip {
    text: SharedString,
    theme: Theme,
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        div()
            .px(px(8.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(theme.tooltip)
            .text_color(theme.tooltip_text)
            .text_xs()
            .shadow_md()
            .child(self.text.clone())
    }
}

/// Builds a tooltip showing `text` (translated).
pub fn tooltip(text: &'static str, theme: Theme) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |_, cx| cx.new(|_| Tooltip { text: t(text).into(), theme }).into()
}

/// A square button showing an icon, with a tooltip. `active` tints it as the current choice.
pub fn icon_button(id: impl Into<SharedString>, name: &str, label: &'static str, active: bool, enabled: bool, theme: Theme) -> Stateful<Div> {
    let color = if !enabled {
        theme.disabled
    } else if active {
        theme.highlight
    } else {
        theme.icon
    };
    div()
        .id(id.into())
        .flex_none()
        .size(px(CONTROL))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme.radius))
        .when(active, |b| b.bg(theme.active))
        .when(enabled && !active, |b| b.cursor_pointer().hover(move |s| s.bg(theme.hover)))
        .when(enabled && active, |b| b.cursor_pointer())
        .tooltip(tooltip(label, theme))
        .child(svg().path(icon(name)).size(px(ICON)).text_color(color))
}

/// A button with a label and an optional icon before it.
pub fn text_button(id: impl Into<SharedString>, icon_name: Option<&str>, label: &'static str, theme: Theme) -> Stateful<Div> {
    div()
        .id(id.into())
        .flex_none()
        .h(px(CONTROL))
        .px(px(8.))
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded(px(theme.radius))
        .cursor_pointer()
        .hover(move |s| s.bg(theme.hover))
        .children(icon_name.map(|name| svg().path(icon(name)).size(px(ICON)).text_color(theme.icon)))
        .child(t(label))
}

/// The main action: filled with the accent colour.
pub fn primary_button(id: impl Into<SharedString>, label: &'static str, enabled: bool, theme: Theme) -> Stateful<Div> {
    div()
        .id(id.into())
        .flex_none()
        .h(px(CONTROL))
        .px(px(12.))
        .flex()
        .items_center()
        .rounded(px(theme.radius))
        .when(enabled, |b| b.bg(theme.accent).text_color(theme.on_accent).cursor_pointer().hover(move |s| s.bg(theme.accent_hover)))
        .when(!enabled, |b| b.bg(theme.hover).text_color(theme.disabled))
        .child(t(label))
}

/// A thin vertical line between groups.
pub fn separator(theme: Theme) -> Div {
    div().flex_none().w(px(1.)).h(px(18.)).mx(px(4.)).bg(theme.separator)
}
