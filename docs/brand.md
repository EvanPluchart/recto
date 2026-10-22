# Recto brand

Masters live in `assets/brand/` (SVG, paths only, no fonts). A presentation board (in French) is in `docs/brand/board.html` and `board.png`.

## Idea

**The folded R.** A geometric capital R whose leg is the corner of a page folding over: the name and the job in one shape. In the horizontal logo the symbol *is* the capital: R + "ecto".

## Files

| File | Use |
|---|---|
| `recto-horizontal-color.svg` / `-reversed` / `-black` / `-white` | Main logo, on light, on Ink, one colour |
| `recto-stacked-color.svg` / `-reversed` | Square spaces |
| `recto-symbol-color.svg` / `-ink` / `-black` / `-white` | Symbol alone |
| `recto-wordmark-ink.svg` | "recto" alone, when the symbol is already present |
| `recto-app-icon.svg`, `recto-app-icon-small.svg` | App icon tile; the small cut (bigger counter and fold) for 16–32 px |
| `png/recto-app-icon-{16…1024}.png`, `recto.ico` | Raster icons (16–32 px from the small cut) |

## Colours

| Name | HEX | RGB | CMYK (approx.) | Role |
|---|---|---|---|---|
| Ink | `#1B1F3B` | 27 31 59 | 54 47 0 77 | The brand colour: symbol, wordmark, tile |
| Paper | `#FFF3E0` | 255 243 224 | 0 5 12 0 | Its answer: reversed logo, backgrounds |
| Fold | `#FF6B35` | 255 107 53 | 0 58 79 0 | The fold only, and rare accents |

Contrast: Paper on Ink 15.9:1, Fold on Ink 5.7:1, Fold on Paper 2.6:1 (decoration only, never text). Pantone equivalents to be matched on a swatch book. Red is avoided on purpose: it is the category's colour.

## Type

The wordmark is drawn from the symbol's geometry (circles of one radius, stems of one weight) and exists only as paths. Interface and documents use the platform font: Segoe UI (Windows), SF Pro (macOS), Inter elsewhere.

## Rules

* Clear space: at least the width of the R's stem on every side.
* Minimum sizes: symbol 16 px (small cut under 32 px), horizontal logo 96 px wide, stacked 64 px.
* Backgrounds: Paper, white or Ink; on photos, use the app icon tile.
* Do not skew, outline, recolour, add effects, or place on busy or low-contrast backgrounds.

## Status

Not cleared as a trademark: run a professional search before registering. The Windows executable icon and installer artwork come with M6.

## In the app

`crates/recto-app/src/theme.rs` maps the palette onto the interface. Light theme: warm Paper-tinted surfaces, Ink text and main button, Fold (darkened to `#C84716` for contrast on light surfaces) for the current tool, tab, selection and check boxes. Dark theme: Ink surfaces, Paper text, Fold for what is active and for the main button, with Ink text on it.
