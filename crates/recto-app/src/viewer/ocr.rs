//! Text recognition: pages are rendered at `OCR_PPI` by the render thread, read by `recto-ocr`
//! on the background executor, and their words become an invisible text layer (one op per page).
//! A rebuild replaces the render thread; a page whose image was asked for then is asked again.

use std::sync::Arc;

use gpui::{Context, Window};
use recto_core::PageId;
use recto_core::geom::PageRect;
use recto_core::op::Op;
use recto_core::textlayer::{TextLayer, Word};
use recto_render::{Bitmap, Purpose, RenderRequest};

use super::{State, Viewer, failure};
use crate::i18n::t;
use crate::recent;

/// Resolution of the images given to OCR.
const OCR_PPI: f32 = 300.0;
/// Longest side of those images, in pixels, for very large pages.
const MAX_SIDE: f32 = 6000.0;
/// Pages recognised between two rebuilds. A text layer is invisible, so the page images do not
/// change: rebuilding after every page would make a long scan quadratic.
const REBUILD_EVERY: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// The page image was asked for.
    Rendering,
    /// The image is being read.
    Reading,
}

pub struct OcrJob {
    waiting: Vec<PageId>,
    current: Option<(PageId, Stage, f32)>,
    done: usize,
    total: usize,
    words: usize,
    /// Text layers recorded since the last rebuild.
    unbuilt: usize,
}

fn models_folder() -> Option<std::path::PathBuf> {
    recent::config_dir().map(|dir| dir.join("ocr-models"))
}

impl Viewer {
    /// Recognises the text of `pages`.
    pub(super) fn recognize(&mut self, pages: Vec<PageId>, window: &mut Window, cx: &mut Context<Self>) {
        if self.ocr.is_some() || pages.is_empty() {
            return;
        }
        let total = pages.len();
        self.ocr = Some(OcrJob { waiting: pages, current: None, done: 0, total, words: 0, unbuilt: 0 });
        if self.ocr_engine.is_some() {
            return self.ocr_next(window, cx);
        }
        let Some(folder) = recto_ocr::find_models(&recto_ocr::model_dirs(models_folder().as_deref())) else {
            self.ocr = None;
            let place = models_folder().map(|dir| dir.display().to_string()).unwrap_or_default();
            self.notice = Some(
                format!(
                    "{} {} {place}",
                    t("Text recognition needs its models (text-detection.rten, text-recognition.rten)."),
                    t("Put them in")
                )
                .into(),
            );
            return cx.notify();
        };
        self.notice = Some(t("Loading the text recognition models…").into());
        cx.notify();
        let task = cx.background_executor().spawn(async move { recto_ocr::Engine::load(&folder) });
        cx.spawn_in(window, async move |this, cx| {
            let loaded = task.await;
            this.update_in(cx, |viewer, window, cx| match loaded {
                Ok(engine) => {
                    viewer.ocr_engine = Some(Arc::new(engine));
                    viewer.ocr_next(window, cx);
                }
                Err(error) => {
                    viewer.ocr = None;
                    viewer.notice = Some(failure("Cannot load the text recognition models", format!("{error:#}")));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Asks for the next page image, or reports the end of the job.
    pub(super) fn ocr_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(job) = &mut self.ocr else { return };
        // Until the engine is loaded (its callback calls back here), nothing is asked for.
        if job.current.is_some() || self.ocr_engine.is_none() {
            return;
        }
        let State::Open(open) = &self.state else {
            self.ocr = None;
            return;
        };
        // Pages deleted since are skipped.
        while let Some(page) = job.waiting.first().copied() {
            job.waiting.remove(0);
            let Some(index) = open.page_states.iter().position(|state| state.id == page) else {
                job.done += 1;
                continue;
            };
            let size = open.geometry[index].size();
            let scale = (OCR_PPI / 72.0).min(MAX_SIDE / size.width.max(size.height));
            job.current = Some((page, Stage::Rendering, scale));
            self.notice = Some(format!("{} {}/{}…", t("Recognizing text: page"), job.done + 1, job.total).into());
            // The highest generation: never dropped as stale.
            open.renderer.request(RenderRequest { page, index, scale, generation: u64::MAX, purpose: Purpose::Ocr, tile: None });
            return cx.notify();
        }
        let (words, total, unbuilt) = (job.words, job.total, job.unbuilt);
        self.ocr = None;
        if unbuilt > 0 {
            self.rebuild(window, cx);
        }
        self.notice = Some(if words == 0 {
            t("No text was recognized.").into()
        } else {
            format!("{} {words} ({} {total}). {}", t("Words recognized:"), t("pages:"), t("Search and copy now work on them.")).into()
        });
        cx.notify();
    }

    /// After a rebuild: a page image asked from the old render thread is asked again.
    pub(super) fn ocr_resume(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(job) = &mut self.ocr else { return };
        if let Some((page, Stage::Rendering, _)) = job.current {
            job.current = None;
            job.waiting.insert(0, page);
        }
        self.ocr_next(window, cx);
    }

    /// A page image for OCR arrived.
    pub(super) fn receive_ocr_image(&mut self, page: PageId, bitmap: Bitmap, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(job), Some(engine)) = (&mut self.ocr, self.ocr_engine.clone()) else { return };
        let Some((current, Stage::Rendering, scale)) = job.current else { return };
        if current != page {
            return;
        }
        job.current = Some((page, Stage::Reading, scale));
        let task = cx.background_executor().spawn(async move {
            let rgb: Vec<u8> = bitmap.pixels.as_chunks::<4>().0.iter().flat_map(|pixel| [pixel[2], pixel[1], pixel[0]]).collect();
            engine.words(bitmap.width, bitmap.height, &rgb)
        });
        cx.spawn_in(window, async move |this, cx| {
            let found = task.await;
            this.update_in(cx, |viewer, window, cx| viewer.finish_page(page, scale, found, window, cx)).ok();
        })
        .detach();
    }

    fn finish_page(&mut self, page: PageId, scale: f32, found: anyhow::Result<Vec<recto_ocr::Word>>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(job) = &mut self.ocr else { return };
        job.current = None;
        job.done += 1;
        let words = match found {
            Ok(words) => words,
            Err(error) => {
                self.ocr = None;
                self.notice = Some(failure("Text recognition failed", format!("{error:#}")));
                return cx.notify();
            }
        };
        let geometry = self
            .open()
            .and_then(|open| open.page_states.iter().position(|state| state.id == page).map(|index| open.geometry[index]));
        let Some(geometry) = geometry.filter(|_| !words.is_empty()) else { return self.ocr_next(window, cx) };
        let words: Vec<Word> = words
            .into_iter()
            .map(|word| {
                let shown = PageRect {
                    x: word.left / scale,
                    y: word.top / scale,
                    width: (word.right - word.left) / scale,
                    height: (word.bottom - word.top) / scale,
                };
                Word { text: word.text, rect: geometry.rect_to_user(shown) }
            })
            .collect();
        let count = words.len();
        let State::Open(open) = &mut self.state else { return };
        open.document.apply(Op::AddTextLayer { page, layer: TextLayer { words, upright: geometry.rotate } });
        let Some(job) = &mut self.ocr else { return };
        job.words += count;
        job.unbuilt += 1;
        if job.unbuilt >= REBUILD_EVERY {
            // The rebuild asks for the next page once reloaded (`ocr_resume`).
            job.unbuilt = 0;
            self.rebuild(window, cx);
        } else {
            self.update_title(window);
            self.ocr_next(window, cx);
        }
    }
}
