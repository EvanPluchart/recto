//! The "Edit text" tool (beta): a click finds the run of text under the pointer, an editor
//! opens over it, and Enter records the new text. When the run's own font cannot show it, an
//! installed font that looks alike is added to the document (`fontmatch`).

use gpui::{AnyElement, Context, Entity, IntoElement, ParentElement, Styled, Window, div, prelude::*, px};
use recto_core::PageId;
use recto_core::geom::{Point, Rect};
use recto_core::op::Op;
use recto_core::textedit::{TextEdit, TextRun};

use super::annotate::Tool;
use super::{State, Viewer, failure};
use crate::fontmatch;
use crate::i18n::t;
use crate::text_input::{TextInput, TextInputEvent};

pub struct EditDraft {
    page: usize,
    id: PageId,
    run: TextRun,
    input: Entity<TextInput>,
}

fn contains(rect: &Rect, point: Point, margin: f32) -> bool {
    point.x >= rect.origin.x - margin
        && point.x <= rect.origin.x + rect.width + margin
        && point.y >= rect.origin.y - margin
        && point.y <= rect.origin.y + rect.height + margin
}

impl Viewer {
    /// Finds the run under `at` (displayed page point) on page `page` and opens the editor.
    pub(super) fn start_text_edit(&mut self, page: usize, at: Point, window: &mut Window, cx: &mut Context<Self>) {
        let State::Open(open) = &self.state else { return };
        let (Some(state), Some(geometry)) = (open.page_states.get(page), open.geometry.get(page)) else { return };
        let (id, point) = (state.id, geometry.to_user(at));
        let snapshot = open.document.clone();
        let task = cx.background_executor().spawn(async move { snapshot.text_runs(id) });
        cx.spawn_in(window, async move |this, cx| {
            let runs = task.await;
            this.update_in(cx, |viewer, window, cx| match runs {
                // The reader moved on (another tool, page order or document) while the page was read.
                _ if viewer.tool != Tool::EditText || viewer.open().and_then(|open| open.page_states.get(page)).map(|state| state.id) != Some(id) => {}
                Ok(runs) => {
                    let hit = runs.iter().find(|run| contains(&run.bounds, point, 0.0)).or_else(|| runs.iter().find(|run| contains(&run.bounds, point, 3.0)));
                    match hit {
                        Some(run) if run.editable => viewer.open_text_editor(page, id, run.clone(), window, cx),
                        Some(_) => {
                            viewer.notice = Some(t("This text cannot be edited: its font does not say which characters it shows.").into());
                            cx.notify();
                        }
                        None => {
                            viewer.notice = Some(t("No text here: click on a line of the page to change it.").into());
                            cx.notify();
                        }
                    }
                }
                Err(error) => {
                    viewer.notice = Some(failure("Cannot read the text of this page", error));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn open_text_editor(&mut self, page: usize, id: PageId, run: TextRun, window: &mut Window, cx: &mut Context<Self>) {
        let theme = self.theme;
        let text = run.text.clone();
        let input = cx.new(|cx| {
            let mut input = TextInput::new(t("Type, then Enter"), |_| true, theme, cx);
            input.set_text(text, cx);
            input
        });
        cx.subscribe_in(&input, window, |viewer, _, event, window, cx| match event {
            TextInputEvent::Commit { text, .. } => viewer.commit_text_edit(text.clone(), window, cx),
            TextInputEvent::Cancel => {
                viewer.edit_draft = None;
                window.focus(&viewer.focus);
                cx.notify();
            }
            TextInputEvent::Changed(_) => {}
        })
        .detach();
        input.update(cx, |input, cx| input.focus_all(window, cx));
        self.edit_draft = Some(EditDraft { page, id, run, input });
        cx.notify();
    }

    fn commit_text_edit(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(EditDraft { id, run, .. }) = self.edit_draft.take() else { return };
        window.focus(&self.focus);
        if text == run.text {
            return cx.notify();
        }
        self.replace_text(id, run, text, window, cx);
    }

    /// Records `text` as the new text of `run`, finding a font for it when needed.
    pub(super) fn replace_text(&mut self, id: PageId, run: TextRun, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let State::Open(open) = &self.state else { return };
        let snapshot = open.document.clone();
        let (style, wanted) = (run.style.clone(), text.clone());
        let task = cx.background_executor().spawn(async move {
            if snapshot.fits_font(id, &run, &wanted).unwrap_or(false) {
                return (run, None, true);
            }
            (run, fontmatch::find(&style, &wanted), false)
        });
        self.notice = Some(t("Editing text…").into());
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let (run, font, fits) = task.await;
            this.update_in(cx, |viewer, window, cx| {
                viewer.notice = None;
                let font = match (fits, font) {
                    (true, _) => None,
                    (false, Some(font)) => {
                        let State::Open(open) = &mut viewer.state else { return };
                        viewer.notice = Some(t("The original font lacks some of these characters: a similar installed font is used.").into());
                        Some(open.document.add_font(font))
                    }
                    (false, None) => {
                        viewer.notice = Some(t("No installed font has these characters.").into());
                        return cx.notify();
                    }
                };
                let edit = TextEdit { first: run.first, last: run.last, original: run.original, text, font };
                viewer.edit(Op::EditText { page: id, edit }, window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// The editor over page `index`, whose box is `frame` at `zoom`.
    pub(super) fn text_edit_overlay(&self, index: usize, frame: (f32, f32, f32, f32), zoom: f32) -> Option<AnyElement> {
        let draft = self.edit_draft.as_ref().filter(|draft| draft.page == index)?;
        let geometry = self.open()?.geometry.get(index).copied()?;
        let shown = geometry.rect_to_display(draft.run.bounds);
        Some(
            div()
                .absolute()
                .left(px(frame.0 + shown.x * zoom - 4.0))
                .top(px(frame.1 + shown.y * zoom - 4.0))
                .w(px((shown.width * zoom + 80.0).max(220.0)))
                .child(draft.input.clone())
                .into_any_element(),
        )
    }
}
