# Landing page

A static page for evanpluchart.fr (in French): `index.html`, the logo and icon from `assets/brand/`, and `screenshot.png` (taken from the app on the generated `tests/fixtures/sample.pdf`).

It is not published from this repository. To put it online, copy this folder to the site and add a `downloads/` folder next to `index.html` with the release files, under stable names:

| Link | File from `cargo xtask dist` or the Release workflow |
|---|---|
| `downloads/Recto-windows-x64-setup.exe` | `Recto-VERSION-windows-x64-setup.exe` |
| `downloads/Recto-macos.dmg` | `Recto-VERSION-macos.dmg` |
| `downloads/Recto-x86_64.AppImage` | `Recto-VERSION-x86_64.AppImage` |

The repository is private, so links to its GitHub releases would not work for visitors: the files are served by the site itself.
