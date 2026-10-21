//! Colour tokens for the light and dark themes.

use gpui::{Rgba, rgb};

#[derive(Clone, Copy)]
pub struct Theme {
    /// Behind the pages.
    pub canvas: Rgba,
    pub toolbar: Rgba,
    pub border: Rgba,
    pub text: Rgba,
    pub muted: Rgba,
    pub hover: Rgba,
    /// Shown while a page has no bitmap yet.
    pub placeholder: Rgba,
}

impl Theme {
    pub fn new(dark: bool) -> Self {
        if dark {
            Self {
                canvas: rgb(0x1e1e1e),
                toolbar: rgb(0x2a2a2a),
                border: rgb(0x3a3a3a),
                text: rgb(0xe0e0e0),
                muted: rgb(0x9a9a9a),
                hover: rgb(0x3c3c3c),
                placeholder: rgb(0x8a8a8a),
            }
        } else {
            Self {
                canvas: rgb(0xe6e6e6),
                toolbar: rgb(0xf7f7f7),
                border: rgb(0xd0d0d0),
                text: rgb(0x262626),
                muted: rgb(0x707070),
                hover: rgb(0xe4e4e4),
                placeholder: rgb(0xffffff),
            }
        }
    }
}
