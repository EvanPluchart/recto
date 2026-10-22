# 0004 — OCR with ocrs, models outside the repository

**Status:** proposed, 2026-10-22, pending the owner's review. The code on `dev` already follows it.

## Context

M4 adds OCR (ROADMAP, FEATURES: `ocrs`). `ocrs` is pure Rust (MIT/Apache) on the `rten` runtime, but it needs two trained models:

| File | Size | SHA-256 |
|---|---|---|
| `text-detection.rten` | 2.5 MB | `f15cfb56…b1b5ca` |
| `text-recognition.rten` | 9.7 MB | `e484866d…735a6e` |

The models are published by the ocrs author under **CC BY-SA 4.0** (Hugging Face `robertknight/ocrs`, trained on HierText, CC BY-SA 4.0). Recto's code is MIT. The recognition model only knows ASCII letters, digits and punctuation: no accents.

## Decision

* OCR lives in its own crate, `recto-ocr` (no UI, no PDF), so `recto-core` stays light. The app renders pages through the render thread, `recto-ocr` finds words in the pixels on a background thread, and the words become an `Op::AddTextLayer` in user space; `recto-core` writes them as invisible Helvetica text (`textlayer.rs`).
* The models are not committed. `cargo xtask fetch-ocr-models` downloads them into `target/ocr-models/` and checks the pinned SHA-256; CI caches them.
* At run time the app looks for them in `RECTO_OCR_MODELS`, then `ocr-models/` next to the executable, then `ocr-models/` in the config folder, then (debug builds) `target/ocr-models/`. It never downloads anything by itself: without models, the OCR command says where to put them.
* `ocrs` and `rten` are built with `opt-level = 3` even in the dev profile; unoptimised they are too slow to use.

## Consequences

* Installers (M6) that ship the models must carry their CC BY-SA 4.0 notice and attribution; the models stay a separate file, not linked into the MIT binary.
* Recognised text has no accents ("été" comes out as "ete"); French scans are searchable only approximately. A model with a wider alphabet can replace these files later without code changes beyond the hashes.
* Recognition runs on the CPU: a few seconds per page.
