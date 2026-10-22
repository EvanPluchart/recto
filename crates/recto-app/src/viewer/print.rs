//! Printing (Ctrl+P). On Windows, pages are rendered one at a time by the render thread at the
//! printer's resolution (at most `MAX_PPI`) and handed to `printer`, which draws them with GDI.
//! Elsewhere, the built PDF is handed to `lp` (CUPS), which prints PDF directly.

use gpui::{Context, Window};

use super::{Viewer, failure};
use crate::i18n::t;

/// Highest resolution pages are rendered at for the printer: finer costs memory, not quality.
#[cfg(windows)]
const MAX_PPI: f32 = 300.0;

/// A print job in progress (Windows).
#[cfg(windows)]
pub struct PrintJob {
    /// Pages still to send, as displayed indices.
    waiting: std::collections::VecDeque<usize>,
    /// The page whose image was asked for.
    current: Option<recto_core::PageId>,
    scale: f32,
    done: usize,
    total: usize,
    sheets: std::sync::mpsc::Sender<crate::printer::Sheet>,
}

/// A PDF being handed to `lp` (macOS, Linux).
#[cfg(not(windows))]
pub struct PrintJob;

impl Viewer {
    pub(super) fn print(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.printing.is_some() || self.open().is_none() {
            return;
        }
        #[cfg(windows)]
        {
            let count = self.open().map_or(0, |open| open.page_states.len());
            let task = cx.background_executor().spawn(async move { crate::printer::choose(count) });
            cx.spawn_in(window, async move |this, cx| {
                let chosen = task.await;
                this.update_in(cx, |viewer, window, cx| match chosen {
                    Ok(Some(setup)) => viewer.start_print(setup, window, cx),
                    Ok(None) => {}
                    Err(error) => {
                        viewer.notice = Some(failure("Cannot print", error));
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
        #[cfg(not(windows))]
        {
            let _ = window;
            let Some(open) = self.open() else { return };
            let snapshot = open.document.clone();
            let name = format!("{}.pdf", self.stem());
            self.notice = Some(t("Sending to the printer…").into());
            self.printing = Some(PrintJob);
            cx.notify();
            let task = cx.background_executor().spawn(async move {
                let bytes = snapshot.build().map_err(|error| error.to_string())?;
                let folder = std::env::temp_dir().join(format!("recto-print-{}", std::process::id()));
                std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
                let path = folder.join(name);
                recto_core::save::write_private(&path, &bytes).map_err(|error| error.to_string())?;
                let status = std::process::Command::new("lp").arg(&path).status().map_err(|error| error.to_string())?;
                status.success().then_some(()).ok_or_else(|| format!("lp: {status}"))
            });
            cx.spawn(async move |this, cx| {
                let sent = task.await;
                this.update(cx, |viewer, cx| {
                    viewer.printing = None;
                    viewer.notice = Some(match sent {
                        Ok(()) => t("Sent to the default printer.").into(),
                        Err(error) => failure("Cannot print", error),
                    });
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    #[cfg(windows)]
    fn start_print(&mut self, setup: crate::printer::Setup, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.open() else { return };
        let pages: std::collections::VecDeque<usize> = setup.pages.clone().filter(|&index| index < open.page_states.len()).collect();
        let scale = setup.dpi.min(MAX_PPI) / 72.0;
        let total = pages.len();
        let sheets = crate::printer::start(setup, self.stem());
        self.printing = Some(PrintJob { waiting: pages, current: None, scale, done: 0, total, sheets });
        self.print_next(window, cx);
    }

    /// Asks for the next page image, or ends the job.
    #[cfg(windows)]
    pub(super) fn print_next(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        use recto_render::{Purpose, RenderRequest};
        let Some(job) = &mut self.printing else { return };
        if job.current.is_some() {
            return;
        }
        let super::State::Open(open) = &self.state else {
            self.printing = None;
            return;
        };
        let Some(index) = job.waiting.pop_front() else {
            let _ = job.sheets.send(crate::printer::Sheet::Done);
            self.printing = None;
            self.notice = Some(t("Sent to the printer.").into());
            return cx.notify();
        };
        let Some(page) = open.page_states.get(index).map(|state| state.id) else {
            self.printing = None;
            return cx.notify();
        };
        job.current = Some(page);
        let (done, total, scale) = (job.done, job.total, job.scale);
        open.renderer.request(RenderRequest { page, index, scale, generation: u64::MAX, purpose: Purpose::Print, tile: None });
        self.notice = Some(format!("{} {}/{total}…", t("Printing: page"), done + 1).into());
        cx.notify();
    }

    /// After a rebuild: the page image asked from the old render thread is asked again.
    #[cfg(windows)]
    pub(super) fn print_resume(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(job) = &mut self.printing else { return };
        if let Some(page) = job.current.take() {
            let index = self.open().and_then(|open| open.page_states.iter().position(|state| state.id == page));
            if let (Some(index), Some(job)) = (index, &mut self.printing) {
                job.waiting.push_front(index);
            }
        }
        self.print_next(window, cx);
    }

    #[cfg(not(windows))]
    pub(super) fn print_resume(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    /// A page image for the printer arrived (or failed).
    #[cfg(windows)]
    pub(super) fn receive_print_image(
        &mut self,
        page: recto_core::PageId,
        result: Result<recto_render::Bitmap, impl std::fmt::Display>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(job) = &mut self.printing else { return };
        if job.current != Some(page) {
            return;
        }
        job.current = None;
        match result {
            Ok(bitmap) => {
                job.done += 1;
                let _ = job.sheets.send(crate::printer::Sheet::Page(bitmap, job.scale));
                self.print_next(window, cx);
            }
            Err(error) => {
                // Dropping the job cancels what was sent.
                self.printing = None;
                self.notice = Some(failure("Cannot print", error));
                cx.notify();
            }
        }
    }
}
