# 0002 — UI toolkit: GPUI

**Status:** accepted, 2026-10-20

## Context

Recto needs smooth scrolling over hundreds of large bitmaps, custom drawing for annotations, and the same look on Windows, macOS and Linux.

## Options

* **GPUI**: GPU-rendered, used by Zed on all three systems, already used by the author in Windows Finder. API still pre-1.0 and changes between releases.
* **egui**: immediate mode, easy, but the result looks like a tool rather than a desktop app.
* **Slint / Iced**: viable, but no existing experience to reuse.
* **Tauri**: web UI; rendering pages would go through the webview, which defeats the point of a native app.

## Decision

GPUI, pinned to an exact version in the workspace `Cargo.toml`.

## Consequences

* Patterns and helpers from Windows Finder (theme tokens, i18n, window chrome, background tasks) can be reused.
* GPUI upgrades are done on purpose, in their own commit, never as a side effect.
