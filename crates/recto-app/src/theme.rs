//! Colour and shape tokens for the light and dark themes.

use gpui::{Rgba, rgb, rgba};

#[derive(Clone, Copy)]
pub struct Theme {
    /// Behind the pages.
    pub canvas: Rgba,
    pub toolbar: Rgba,
    /// Sidebar and menus.
    pub panel: Rgba,
    pub border: Rgba,
    /// Lines between groups of controls.
    pub separator: Rgba,
    pub text: Rgba,
    pub muted: Rgba,
    pub icon: Rgba,
    pub disabled: Rgba,
    pub hover: Rgba,
    /// Background of the current tool, tab or choice.
    pub active: Rgba,
    /// Shown while a page has no bitmap yet.
    pub placeholder: Rgba,
    /// Text field background.
    pub field: Rgba,
    /// Focus rings, current item, main button.
    pub accent: Rgba,
    pub accent_hover: Rgba,
    /// Text on the accent colour.
    pub on_accent: Rgba,
    /// Selected text.
    pub selection: Rgba,
    /// Search hits, and the current one.
    pub hit: Rgba,
    pub hit_current: Rgba,
    pub tooltip: Rgba,
    pub tooltip_text: Rgba,
    /// Errors and warnings in forms.
    pub error: Rgba,
    /// Corner radius of controls, in pixels.
    pub radius: f32,
}

impl Theme {
    pub fn new(dark: bool) -> Self {
        if dark {
            Self {
                canvas: rgb(0x16171a),
                toolbar: rgb(0x212226),
                panel: rgb(0x1c1d20),
                border: rgb(0x2f3136),
                separator: rgb(0x383a40),
                text: rgb(0xe7e8eb),
                muted: rgb(0x9ba1aa),
                icon: rgb(0xc8ccd3),
                disabled: rgb(0x5c6169),
                hover: rgb(0x2d2f34),
                active: rgba(0x5b8cff2e),
                placeholder: rgb(0x2a2b2f),
                field: rgb(0x18191c),
                accent: rgb(0x5b8cff),
                accent_hover: rgb(0x739dff),
                on_accent: rgb(0xffffff),
                selection: rgba(0x5b8cff66),
                hit: rgba(0xffd23f59),
                hit_current: rgba(0xff8a0099),
                tooltip: rgb(0xeef0f3),
                tooltip_text: rgb(0x1d2025),
                error: rgb(0xff7b72),
                radius: 6.0,
            }
        } else {
            Self {
                canvas: rgb(0xe8e9ec),
                toolbar: rgb(0xfbfbfc),
                panel: rgb(0xf4f5f7),
                border: rgb(0xdfe1e5),
                separator: rgb(0xe2e4e8),
                text: rgb(0x1d2025),
                muted: rgb(0x6b717b),
                icon: rgb(0x3d434d),
                disabled: rgb(0xb3b8c0),
                hover: rgb(0xedeff2),
                active: rgba(0x2563eb1f),
                placeholder: rgb(0xffffff),
                field: rgb(0xffffff),
                accent: rgb(0x2563eb),
                accent_hover: rgb(0x1d4fd0),
                on_accent: rgb(0xffffff),
                selection: rgba(0x2563eb4d),
                hit: rgba(0xffd23f73),
                hit_current: rgba(0xff8a0099),
                tooltip: rgb(0x1d2025),
                tooltip_text: rgb(0xf4f5f7),
                error: rgb(0xc62828),
                radius: 6.0,
            }
        }
    }
}
