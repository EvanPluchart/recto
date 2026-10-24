# 0006 — Updates from the app

Status: accepted, 2026-10-24.

## Context

Updating Recto meant downloading the new installer by hand. The author asked for what Vigie does since its 0.1.2: Recto finds new versions on its own and installs them in one click, without a visit to GitHub.

Until now Recto went online only to open a link the reader clicked. The check described here goes online on its own: it is stated here, and it can be turned off.

Recto is an editor, which sets two rules Vigie did not need:

* **Updating never loses unsaved work.** Recto quits through the same question as closing the window.
* **Each window is its own process, and nothing closes another one.**

## Decision

### Checking

* **When:** 3 s after a window opens, then every 6 hours while it stays open.
* **How:** one request to `https://github.com/EvanPluchart/recto/releases/latest`, without following the redirect. GitHub answers `302` to `/releases/tag/vX.Y.Z`, which names the newest published version. There is no API, no rate limit, and nothing else is sent.
* **Where it runs:** on the background executor. The loop ends with its window.
* **What is offered:** `recto-core::update` compares versions numerically. Only a strictly newer, plain `X.Y.Z` release is offered. Being offline, or getting an unexpected answer, is silent: the next check tries again.
* **Turning it off:** `--no-update-check`, or the `RECTO_NO_UPDATE_CHECK` environment variable. Recto has no Settings view yet.
* **Never in staged runs:** a run with a flag that sets a state up for screenshots or tests (`--find`, `--apply`, `--save-as`, `--tool`, `--ocr`, `--export`, `--menu`, `--dialog`, `--replace`) does not check.
  * The exception is `--pretend-version=X.Y.Z`, which behaves as an older version to show the button.
* **Windows only for now.** macOS and Linux show nothing yet.

### The button

* **Where:** at the right of the app bar, "Recto X is available · Update".
* **For an install for this account:** a click does the following:
  1. downloads `Recto-X-windows-x64-setup.exe` and `SHA256SUMS` into a new folder under `%TEMP%`, while the window stays usable;
  2. checks the installer's SHA-256, refusing a missing line or a mismatch;
  3. asks about unsaved changes as closing does (declining keeps the checked installer for the next click);
  4. refuses if another Recto window is open, and says so;
  5. remembers the open document, lets go of the running mark, starts the installer with `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /NOCLOSEAPPLICATIONS /relaunch=1`, and quits.
* **The installer** waits for Recto to quit (the `Local\Recto.Running` mutex every Recto holds, also its `AppMutex`). It never closes a Recto (`CloseApplications=no`). It keeps the previous install mode and folder, and starts Recto again with `--updated`. Recto then opens the document again and says "Recto X is installed · See what's new", which opens the release notes.
  * If a Recto still runs after 10 s, the installer stops and starts the old Recto again.
* **For an install for every account (Program Files), or the zip:** the button opens the release page. Installing there needs an administrator prompt that this first version does not handle.
* **For a Recto running as administrator:** it never runs a downloaded file, and opens the release page.
* **Integrity:** HTTPS to GitHub, plus the checksum from the same release. The checksum catches a truncated or damaged download, not a compromised release. Signing remains the real answer.
* **Older versions have no updater:** 0.1.0 users install the next version by hand, once.

The network and file steps are Vigie's, ported: WinHTTP and BCrypt, through `windows-sys`, already in the build.
