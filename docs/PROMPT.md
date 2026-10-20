# Initial prompt — milestone M0

Paste this into a new agent session at the repository root.

---

You are building **Recto**, a cross-platform PDF viewer and editor in Rust. Read `CLAUDE.md`, `docs/ARCHITECTURE.md`, `docs/ROADMAP.md` and `docs/decisions/` first. Then deliver **milestone M0** exactly as the roadmap defines it, and nothing beyond it.

## Deliverables

1. **Workspace dependencies.** In the root `Cargo.toml`, pin `gpui = "=0.2.2"` (same as `../windows-finder`), `pdfium-render`, `lopdf`, `flume`, `anyhow`, `thiserror`. Check crates.io for the current versions before pinning.
2. **`xtask` crate** with `cargo xtask fetch-pdfium`: downloads the pinned PDFium release from bblanchon/pdfium-binaries for the host OS and architecture, and extracts the library to `target/pdfium/`. Add the `xtask` alias in `.cargo/config.toml`.
3. **`recto-render`**: implement the `Renderer` trait (already sketched in `src/lib.rs`) with a PDFium backend, plus `RenderThread` — a dedicated thread that owns the renderer and serves `RenderRequest`s over `flume`, dropping requests whose generation is stale. Load PDFium from `target/pdfium/` in dev and from next to the executable in release.
4. **`recto-core`**: `Document::open(path)` that reads bytes and validates them with `lopdf`. No ops yet beyond the enum sketch.
5. **`recto-app`**: `recto [file.pdf] [--page=N] [--zoom=PCT] [--dark|--light]` opens a GPUI window and shows the requested page, rendered on the render thread. Plain window chrome is fine for M0.
6. **Fixtures**: a script or test helper that generates `tests/fixtures/sample.pdf` (3 pages, text and one image) with `pdf-writer`. No downloaded PDFs.
7. **CI**: `.github/workflows/ci.yml` with a matrix on `windows-latest`, `macos-latest`, `ubuntu-latest` running fetch-pdfium, `cargo check`, `cargo test`, `cargo clippy -- -D warnings`. Install GPUI's Linux system dependencies on Ubuntu.
8. **Tests**: render page 1 of the fixture to a bitmap and assert its size and that it is not blank; stale-generation requests are dropped.

## Rules

* Respect the hard rules in `CLAUDE.md`, in particular: no `gpui` in `recto-core` or `recto-render`, only the render thread touches PDFium.
* If a decision record turns out to be wrong in practice, stop and explain instead of working around it.
* Commit in small steps with conventional messages (`feat(render): …`). No AI attribution lines.

## Done when

`cargo xtask fetch-pdfium && cargo run -p recto -- tests/fixtures/sample.pdf` shows page 1 on this machine, CI is green on all three systems, and you report what works, what you could not verify, and anything you changed in the docs.
