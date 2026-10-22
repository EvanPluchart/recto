//! Colour and shape tokens of the interface (light only), built on the brand palette
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
    /// Outline of text fields and check boxes: 3:1 against their background (WCAG 1.4.11).
    pub input_border: Rgba,
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
    /// The one theme: warm Paper-tinted surfaces, Ink text and main button, Fold for what is
    /// active.
    pub fn light() -> Self {
        Self {
            canvas: rgb(0xebe8e2),
            toolbar: rgb(0xfdfbf7),
            panel: rgb(0xf6f3ed),
            border: rgb(0xe4dfd6),
            input_border: rgb(0x8a8696),
            separator: rgb(0xe6e1d8),
            text: rgb(0x1b1f3b),
            muted: rgb(0x615e70),
            icon: rgb(0x3a3e5c),
            disabled: rgb(0xa19eab),
            hover: rgb(0xf0ece4),
            active: rgba(0xff6b3524),
            // Fold darkened so active icons reach 4:1 on light surfaces and on their tint.
            highlight: rgb(0xc84716),
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
