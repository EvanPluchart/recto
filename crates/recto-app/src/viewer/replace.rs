//! Find and replace (edit mode): every run of text showing the search query is rewritten with
//! the replacement (`Document::find_replacements`), in its own font when it can show the new
//! text, else in an installed font that looks alike (`fontmatch`).

use std::collections::HashMap;

use gpui::{Context, Window};
use recto_core::embed::FontFile;
use recto_core::op::Op;
use recto_core::textedit::TextEdit;

use super::{State, Viewer, failure};
use crate::fontmatch;
use crate::i18n::t;

impl Viewer {
    pub(super) fn replace_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editing {
            return;
        }
        let query = self.search.query.clone();
        let replacement = self.search.replace.read(cx).text().to_owned();
        let State::Open(open) = &self.state else { return };
        if query.is_empty() {
            return;
        }
        let snapshot = open.document.clone();
        self.notice = Some(t("Replacing…").into());
        cx.notify();
        let task = cx.background_executor().spawn(async move {
            let found = snapshot.find_replacements(&query, &replacement)?;
            // One installed font per original font that needs one.
            let mut fonts: HashMap<String, Option<FontFile>> = HashMap::new();
            let mut ready = Vec::new();
            for replacement in found {
                let font = if replacement.fits {
                    None
                } else {
                    let key = replacement.run.style.name.clone();
                    let font = fonts.entry(key).or_insert_with(|| fontmatch::find(&replacement.run.style, &replacement.text)).clone();
                    // A font chosen for other text may lack these characters.
                    match font.filter(|font| font.covers(&replacement.text)).or_else(|| fontmatch::find(&replacement.run.style, &replacement.text)) {
                        Some(font) => Some(font),
                        None => continue,
                    }
                };
                ready.push((replacement, font));
            }
            Ok::<_, recto_core::BuildError>(ready)
        });
        cx.spawn_in(window, async move |this, cx| {
            let ready = task.await;
            this.update_in(cx, |viewer, window, cx| {
                let ready = match ready {
                    Ok(ready) => ready,
                    Err(error) => {
                        viewer.notice = Some(failure("Cannot replace the text", error));
                        return cx.notify();
                    }
                };
                let State::Open(open) = &mut viewer.state else { return };
                let count = ready.len();
                for (replacement, font) in ready {
                    let font = font.map(|font| open.document.add_font(font));
                    let run = replacement.run;
                    let edit = TextEdit { first: run.first, last: run.last, original: run.original, text: replacement.text, font };
                    open.document.apply(Op::EditText { page: replacement.page, edit });
                }
                viewer.notice = Some(if count == 0 {
                    t("Nothing to replace: the text was not found in editable lines.").into()
                } else {
                    format!("{} {count}", t("Lines changed:")).into()
                });
                if count > 0 {
                    viewer.rebuild(window, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
