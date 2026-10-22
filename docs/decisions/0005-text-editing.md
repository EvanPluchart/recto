# 0005 — Editing existing text: run-level rewrite, fallback fonts chosen by the app

**Status:** proposed, 2026-10-22, pending the owner's review. The code on `dev` already follows it.

## Context

M5 asks to edit text in place, "re-encode with the original font when its glyphs allow it, fall back to a matching system font otherwise". PDF text is a sequence of show operations in fonts that are often subsets, with encodings that may not say which characters they show. Whatever Recto writes must still be readable by every reader, and the build must stay deterministic (the same op log gives the same file on every machine).

## Decision

* **Unit of edit: a run** — consecutive show operations of one text object, in one font and size, on one line (`recto-core/src/textedit.rs`). An edit replaces the run's text at the run's start; nothing reflows.
* **Decoding** (`fonts.rs`): ToUnicode CMaps (own parser: codespace, bfchar, bfrange), else the simple font's encoding through `lopdf`. Composite fonts are edited only with Identity-H and a ToUnicode CMap. Type 3 fonts, vertical writing and runs with undecodable codes are not editable.
* **Same font** only when every character has a code and a glyph known to be drawn: the font is not embedded, or `ttf-parser` finds the glyph in the embedded TrueType/OpenType program, or the same code is already shown with that font on the page.
* **Fallback font**: chosen by the app with `fontdb` (same family if installed, else a common font of the same kind, else any font with the characters) and added to the document as data (`Document::add_font`); the op refers to it by id, so `recto-core` never reads system fonts. It is embedded as a Type0 / Identity-H subset (`subsetter`) with widths and a ToUnicode CMap (`embed.rs`). Faces whose licence forbids embedding and variable fonts are refused.
* **Safety**: each `Op::EditText` keeps the run's original text; the build refuses an edit whose run no longer shows it.
* **Dependencies**: `ttf-parser` and `subsetter` (MIT/Apache-2.0) in `recto-core`; `fontdb` (MIT) in `recto-app`.
* **Tests** use a TrueType font generated in code (`testfont.rs`), never a licensed font file.

## Consequences

* Longer text runs over what follows on the line; shorter text leaves a gap. Kerning inside an edited run is lost.
* A run moved to a fallback font looks somewhat different; word spacing (`Tw`) no longer applies to it.
* Text inside form XObjects, annotations and form fields is not editable with this tool.
* Fonts embedded from the user's system are subject to their licences; restricted ones are skipped.
