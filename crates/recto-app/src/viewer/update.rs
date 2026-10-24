//! The update button in the app bar (decision 0006): "Recto X is available · Update", one click
//! to download, check and install it, after the usual question about unsaved changes.

use gpui::{AnyElement, Context, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled, Window, div, px, svg};
use recto_core::update::Version;

use super::Viewer;
use crate::assets::icon;
use crate::i18n::t;
use crate::update::{self, Failure, Mode, Phase};

impl Viewer {
    /// Checks now (after a pause) and every 6 hours while the window lives. `current` is this
    /// version, or the one a test pretends to be.
    pub(super) fn start_update_checks(&mut self, current: String, cx: &mut Context<Self>) {
        self.update_mode = update::mode();
        if self.update_mode == Mode::Unsupported {
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(update::FIRST).await;
            loop {
                let current = current.clone();
                let found = cx.background_executor().spawn(async move { update::check(&current) }).await;
                // The window is gone: nothing keeps checking.
                if this.update(cx, |viewer, cx| viewer.update_found(found, cx)).is_err() {
                    break;
                }
                cx.background_executor().timer(update::EVERY).await;
            }
        })
        .detach();
    }

    fn update_found(&mut self, found: Option<Version>, cx: &mut Context<Self>) {
        let Some(version) = found else { return };
        // A download under way, or a failure the reader has not seen yet, keeps its state.
        if matches!(self.update, Phase::Idle | Phase::Available(_) | Phase::Installed(_)) {
            self.update = Phase::Available(version);
            cx.notify();
        }
    }

    fn update_clicked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.update.clone() {
            Phase::Available(version) | Phase::Failed(_, version) => {
                if self.update_mode != Mode::Install || self.update == Phase::Failed(Failure::NoChecksum, version) {
                    cx.open_url(&update::release_page(version));
                } else if let Some((ready, path)) = self.update_ready.clone()
                    && ready == version
                {
                    self.install_update(version, path, window, cx);
                } else {
                    self.download_update(version, window, cx);
                }
            }
            Phase::Installed(version) => {
                cx.open_url(&update::release_page(version));
                self.update = Phase::Idle;
            }
            Phase::Idle | Phase::Downloading(_) => {}
        }
        cx.notify();
    }

    /// Downloads and checks the installer in the background; the window stays usable.
    fn download_update(&mut self, version: Version, window: &mut Window, cx: &mut Context<Self>) {
        self.update = Phase::Downloading(version);
        let task = cx.background_executor().spawn(async move { update::fetch(version) });
        cx.spawn_in(window, async move |this, cx| {
            let fetched = task.await;
            this.update_in(cx, |viewer, window, cx| match fetched {
                Ok(path) => {
                    viewer.update_ready = Some((version, path.clone()));
                    viewer.update = Phase::Available(version);
                    viewer.install_update(version, path, window, cx);
                }
                Err(failure) => {
                    viewer.update = Phase::Failed(failure, version);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Asks about unsaved changes as closing does, then hands over to the installer and quits.
    /// Declining keeps the checked installer for the next click.
    fn install_update(&mut self, version: Version, path: std::path::PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_discard(window, cx, move |viewer, _, cx| {
            if !update::release_unless_others() {
                viewer.update = Phase::Failed(Failure::OthersOpen, version);
                cx.notify();
                return;
            }
            if let Some(document) = viewer.open().and_then(|open| open.document.path()) {
                update::save_reopen(document);
            }
            if update::start_installer(&path) {
                cx.quit();
            } else {
                update::mark_running();
                viewer.update = Phase::Failed(Failure::NotStarted, version);
                cx.notify();
            }
        });
    }

    /// The button at the right of the app bar, while there is something to say.
    pub(super) fn update_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = self.theme;
        // Short, so the bar keeps one line; the details are in the tooltip.
        let (label, details) = match &self.update {
            Phase::Idle => return None,
            Phase::Available(version) => match self.update_mode {
                Mode::Install => (format!("{} {version}", t("Update to")), "A new version of Recto is out: install it and open this document again"),
                _ => (format!("{} {version}", t("Download")), "A new version of Recto is out: open its page"),
            },
            Phase::Downloading(version) => (format!("{} {version}…", t("Downloading")), "Recto closes once the update is ready, after asking about unsaved changes"),
            Phase::Failed(failure, _) => (t("Update failed").to_owned(), failure.message()),
            Phase::Installed(version) => (format!("{} {version} · {}", t("Recto"), t("What's new")), "See what this version brings"),
        };
        let failed = matches!(self.update, Phase::Failed(..));
        Some(
            div()
                .id("update")
                .flex_none()
                .h(px(28.))
                .px(px(10.))
                .mx(px(4.))
                .flex()
                .items_center()
                .gap(px(6.))
                .rounded(px(theme.radius))
                .text_color(if failed { theme.error } else { theme.accent })
                .cursor_pointer()
                .hover(move |s| s.bg(theme.hover))
                .child(svg().path(icon(if failed || matches!(self.update, Phase::Installed(_)) { "info" } else { "download" })).size(px(14.)).text_color(if failed { theme.error } else { theme.accent }))
                .child(label)
                .tooltip(crate::chrome::tooltip(details, theme))
                .on_click(cx.listener(|this, _, window, cx| this.update_clicked(window, cx)))
                .into_any_element(),
        )
    }
}
