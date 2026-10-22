//! Colour and shape tokens for the light and dark themes, built on the brand palette
//! (docs/brand.md): Ink #1B1F3B, Paper #FFF3E0, Fold #FF6B35.

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
    /// The current tool, tab, choice or selection: icons, rings, check boxes (Fold).
    pub highlight: Rgba,
    /// Shown while a page has no bitmap yet.
    pub placeholder: Rgba,
    /// Text field background.
    pub field: Rgba,
    /// Main buttons and focus rings.
    pub accent: Rgba,
    pub accent_hover: Rgba,
    /// Text on the accent colour, and marks on the highlight colour.
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
            // Ink surfaces, Paper text, Fold for what is active and for the main button.
            Self {
                canvas: rgb(0x0f1124),
                toolbar: rgb(0x1b1f3b),
                panel: rgb(0x171a33),
                border: rgb(0x2a2f55),
                separator: rgb(0x343a66),
                text: rgb(0xfff3e0),
                muted: rgb(0xa9a6bf),
                icon: rgb(0xdcd8e8),
                disabled: rgb(0x575c82),
                hover: rgb(0x262b4e),
                active: rgba(0xff6b3529),
                highlight: rgb(0xff6b35),
                placeholder: rgb(0x262b4e),
                field: rgb(0x12152c),
                accent: rgb(0xff6b35),
                accent_hover: rgb(0xff8457),
                on_accent: rgb(0x1b1f3b),
                selection: rgba(0xff6b3559),
                hit: rgba(0xffd23f59),
                hit_current: rgba(0xff6b3599),
                tooltip: rgb(0xfff3e0),
                tooltip_text: rgb(0x1b1f3b),
                error: rgb(0xff8a80),
                radius: 6.0,
            }
        } else {
            // Paper-warm surfaces, Ink text and main button, Fold for what is active.
            Self {
                canvas: rgb(0xebe8e2),
                toolbar: rgb(0xfdfbf7),
                panel: rgb(0xf6f3ed),
                border: rgb(0xe4dfd6),
                separator: rgb(0xe6e1d8),
                text: rgb(0x1b1f3b),
                muted: rgb(0x6f6c7d),
                icon: rgb(0x3a3e5c),
                disabled: rgb(0xb8b5c0),
                hover: rgb(0xf0ece4),
                active: rgba(0xff6b3524),
                // Darker than Fold so active icons stand out on light surfaces.
                highlight: rgb(0xe4561f),
                placeholder: rgb(0xffffff),
                field: rgb(0xffffff),
                accent: rgb(0x1b1f3b),
                accent_hover: rgb(0x2d3260),
                on_accent: rgb(0xfff3e0),
                selection: rgba(0xff6b354d),
                hit: rgba(0xffd23f73),
                hit_current: rgba(0xff6b3599),
                tooltip: rgb(0x1b1f3b),
                tooltip_text: rgb(0xfff3e0),
                error: rgb(0xc62828),
                radius: 6.0,
            }
        }
    }
}
