# Recto landing page — DESIGN.md

The design system of `site/`, in the DESIGN.md format (Google Stitch, as collected by
awesome-design-md). Read it before changing the page; keep the page and this file in step.

## 1. Visual theme and atmosphere

A sheet of paper on a desk, set in large type. The page reads as a short series of sentences, each
with its proof beside it: real screenshots of Recto on a made-up quote (`cargo xtask gen-demo`).
Light only: there is no dark theme, on purpose. French at the root, English in `en/`, same
layout and assets. One idea carries it: the signature, the job people
most often send a PDF to a website for, sits inside the headline, and further down the visitor signs
the quote itself. The page alternates flat fields: paper, Fold behind the app, Ink behind the
comparison, Fold again for the download, Ink for the footer.

Sections: hero (sentence + the app window and its signature dialog on Fold, a marquee of verbs);
the story (the quote pinned on desktop while six steps pass: read, edit, highlight, sign with the
visitor's name, redact, save; on small screens the quote plays the steps by itself); comparison;
a board of interface close-ups; three figures; questions; the download sentence; the footer.

Art direction dealt by taste-skill v2: "The sentence is the layout" (seed 1513397970), with the
features section redrawn to "Comparison set as text" because the page's main claim is a comparison
with online services.

## 2. Colour palette and roles

| Token | Value | Role |
|---|---|---|
| `--bg` | `#FBFAF7` | Page ground (near white, not cream) |
| `--surface` | `#F1EFEA` | One step of tone: alternate table rows, open FAQ tile, the try-it field |
| `--canvas` | `#E8E5DE` | The app's own canvas colour, behind screenshots, so they sit flush |
| `--ink` | `#1B1F3B` | Brand Ink: all text, the featured column, the footer field |
| `--muted` | `#4D5168` | Secondary notes only (7.9:1 on `--bg`) |
| `--paper` | `#FFF3E0` | Brand Paper: text and logo on Ink |
| `--fold` | `#FF6B35` | Brand Fold: the primary action (Ink text, 5.7:1) and the logo's fold. Never text on light |

One ground, one ink, one accent. The accent appears on the download buttons and nowhere else.

## 3. Typography

One family, Geist (SIL OFL, self-hosted, latin subset), weights 400–700. Five sizes:

| Token | Size | Use |
|---|---|---|
| `--t-display` | `clamp(48px, 8.6vw, 144px)` | The hero sentence; tracking -0.045em, leading 0.95 |
| `--t-head` | `clamp(34px, 4.6vw, 72px)` | Section headings; tracking -0.035em, leading 1.0 |
| `--t-lead` | `clamp(20px, 1.7vw, 26px)` | Statements, table names, FAQ questions |
| `--t-body` | `18px` | Running text, full Ink, leading 1.5, 60ch measure |
| `--t-small` | `15px` | Notes, captions, table cells on a phone |

Signatures use the four script faces Recto ships (Great Vibes, Allura, Sacramento, Dancing Script,
SIL OFL); they are product content, not page typography.

## 4. Components

* **Button**: square corners (radius 0), 52px tall, Fold fill, Ink text, 600 weight. Hover: Ink fill,
  Paper text. Press: scale 0.97. Focus: 3px Ink outline offset 3px.
* **Navigation**: the logo, four text links, one button. No bar, no blur.
* **Screenshots**: square corners, no shadow, on `--canvas`. Every image has `width`, `height`, `alt`.
* **Table**: no rules; rows separated by a tone step; the featured column is one Fold block with Ink text, on the Ink field.
* **FAQ**: `<details>`, no rules, open row on a `--surface` tile.
* **Story overlays**: drawn over the real screenshot in its own pixel coordinates (900 × 1000): a
  highlight, the mention and signature (SVG text revealed by a growing clip rectangle), the redaction
  bar, the save toast. Only the dialog and the toast carry a shadow: they float in the app too.

## 5. Layout

12-column grid, one pair of outer margins `clamp(16px, 3vw, 40px)`, max width 1440px. Sections
160–200px apart on desktop, 96px on a phone. Each section has a different composition from its
neighbours.

## 6. Depth and elevation

None. Surfaces differ by a step in tone. No gradients, glass or glows; shadows only on what floats in
the app itself (the signature dialog, the save toast).

## 7. Do and don't

Do: Ink text at full contrast, real screenshots, sentence case, French typography (non-breaking
spaces before `: ; ? !`, `«  »`).
Don't: eyebrow labels, monospace, numbered steps, thin rules, pill badges, icon cards, emoji,
invented proof (users, quotes, press), a dark theme.

## 8. Responsive behaviour

Breakpoints at 900px and 600px. On a phone the hero signature leaves the line and sits between the
two halves of the sentence; the comparison table turns into one block per row; the story quote is not
pinned and plays its steps by itself while on screen.

## 9. Motion

The app window opens with a shutter (clip-path, 1.1s) and drifts as it leaves; the signature dialog
rises after it. The verbs marquee runs at 48s a loop and pauses on hover. Section parts rise 24px and
fade in, staggered by 80ms, once. Story overlays sweep, write and slide in as their step arrives.
The 21 Mo figure rolls up. All of it is off under
`prefers-reduced-motion`, and nothing is hidden when script does not run.
