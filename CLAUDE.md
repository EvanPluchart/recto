# Recto — agent guide

Cross-platform PDF viewer and editor in Rust (GPUI + PDFium + lopdf). Read these before changing code:

1. `docs/ARCHITECTURE.md` — layers, threading, coordinates, conventions.
2. `docs/ROADMAP.md` — current milestone and its exit criteria. Work on the current milestone only.
3. `docs/decisions/` — settled choices. Do not reopen them without a new decision record.

## Hard rules

* `recto-core` and `recto-render` never depend on `gpui`. `recto-app` never parses PDF bytes itself.
* `Op`s store PDF user-space coordinates only.
* Only the render thread touches `Pdfium`.
* Never modify a user's PDF in place without an atomic write. Never commit real user PDFs; fixtures in `tests/fixtures/` are generated or public-domain.
* GPUI and PDFium versions are pinned. Upgrade them only when asked, in a dedicated commit.
* Code, comments, commits in English. UI strings through `i18n::t`.
* No `Co-Authored-By` or AI attribution lines in commits or PRs.

## Commands

```sh
cargo check --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo run -p recto -- tests/fixtures/sample.pdf --page=1 --zoom=100
```

## Seeing the app

Launch with test flags (`--page`, `--zoom`, `--tool`, `--dark`/`--light`) to reach a state directly. On Windows, capture the window with PrintWindow, not a screen grab. Never send synthetic input unless the Recto window is verified to be in the foreground.

## Reference code

`../windows-finder` uses the same GPUI version and solves window chrome, theme tokens with dark-mode remapping, i18n, and background tasks. Reuse its patterns; do not copy Windows-only code into shared crates.
