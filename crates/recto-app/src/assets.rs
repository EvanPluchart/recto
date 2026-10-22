//! Files built into the binary: the Lucide icons (see `assets/NOTICE.md`), served to GPUI as
//! `icons/NAME.svg`, and the app icon (`assets/brand/`) as `brand/app-icon.png`.

use std::borrow::Cow;

use gpui::{AssetSource, SharedString};

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        const ICONS: &[(&str, &[u8])] = &[$(($name, include_bytes!(concat!("../assets/icons/", $name, ".svg")))),*];
    };
}

icons![
    "check",
    "chevron-down",
    "chevron-left",
    "chevron-right",
    "circle",
    "clock",
    "crop",
    "download",
    "droplets",
    "ellipsis",
    "eye-off",
    "eraser",
    "file-input",
    "file-output",
    "file-plus",
    "file-text",
    "folder-open",
    "hash",
    "heading",
    "highlighter",
    "image",
    "image-off",
    "info",
    "layers",
    "list",
    "maximize-2",
    "message-square",
    "message-square-off",
    "minus",
    "move-horizontal",
    "moon",
    "mouse-pointer-2",
    "move-up-right",
    "paintbrush",
    "panel-left",
    "pencil",
    "redo-2",
    "replace",
    "rotate-ccw",
    "rotate-cw",
    "save",
    "scan-text",
    "scissors",
    "search",
    "signature",
    "square",
    "strikethrough",
    "sun",
    "text-cursor-input",
    "trash-2",
    "type",
    "underline",
    "undo-2",
    "x",
    "zoom-in",
    "zoom-out",
];

/// The app icon, for the start screen (`brand/app-icon.png`).
const APP_ICON: &[u8] = include_bytes!("../../../assets/brand/png/recto-app-icon-256.png");

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        if path == "brand/app-icon.png" {
            return Ok(Some(Cow::Borrowed(APP_ICON)));
        }
        let name = path.strip_prefix("icons/").and_then(|file| file.strip_suffix(".svg"));
        Ok(name.and_then(|name| ICONS.iter().find(|(icon, _)| *icon == name)).map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(ICONS.iter().map(|(name, _)| format!("icons/{name}.svg").into()).filter(|file: &SharedString| file.starts_with(path)).collect())
    }
}

/// Path of an icon, for `svg().path(…)`.
pub fn icon(name: &str) -> SharedString {
    debug_assert!(ICONS.iter().any(|(icon, _)| *icon == name), "unknown icon {name}");
    format!("icons/{name}.svg").into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_every_icon() {
        assert!(Assets.load("icons/type.svg").unwrap().is_some_and(|bytes| bytes.starts_with(b"<!-- @license lucide")));
        assert!(Assets.load("icons/none.svg").unwrap().is_none());
        assert_eq!(Assets.list("icons/").unwrap().len(), ICONS.len());
    }
}
