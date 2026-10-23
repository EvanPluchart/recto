# Landing page

The static page for GitHub Pages, in French (`index.html`) and English (`en/index.html`), sharing
`assets/` (stylesheet, script, fonts, screenshots, social images; English screenshots in
`assets/en/`), the logo and favicons. No build step, no framework, no third-party
requests: fonts are self-hosted (SIL OFL, licences in `assets/fonts/`).

* Design system and art direction: `DESIGN.md`.
* Screenshots are the app on `target/demo.pdf` and `target/demo-en.pdf` (`cargo xtask gen-demo`, a
  made-up quote in both languages, same layout), with `RECTO_LANG` set to match, taken with the
  `--apply` test flags and cropped to the window content.
* Publish: run the Pages workflow by hand (`.github/workflows/pages.yml`) once Pages is enabled with
  "GitHub Actions" as its source. The page expects to live at `https://evanpluchart.github.io/recto/`
  (canonical and social links); every asset path is relative.
* Downloads point to `https://github.com/EvanPluchart/recto/releases/latest`, which only works for
  visitors once the repository is public and a release is published. macOS and Linux are described
  as coming later until their packages have been run.
