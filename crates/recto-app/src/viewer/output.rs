//! Writing files: save (asking before replacing the original), save as, extract and split.
//! Every write is atomic (`recto_core::save::write_atomic`) and runs in the background.

use std::path::{Path, PathBuf};

use gpui::{Context, PathPromptOptions, PromptLevel, Window};
use recto_core::op::{Op, PageState};
use recto_core::save::write_atomic;
use recto_core::{BuildOptions, split};

use super::{State, Viewer, failure};
use crate::i18n::t;
use crate::recent;

/// How to split the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitMode {
    /// One file per comma-separated range: `1-3, 4-9`.
    Ranges,
    /// Files of N pages.
    Every,
    /// A file per top-level bookmark.
    Bookmarks,
}

/// `1.4 MB` style sizes.
fn file_size(bytes: usize) -> String {
    match bytes {
        0..1_000 => format!("{bytes} B"),
        1_000..1_000_000 => format!("{:.0} kB", bytes as f64 / 1e3),
        _ => format!("{:.1} MB", bytes as f64 / 1e6),
    }
}

/// `report-3.pdf` style names for split parts, next to each other in `folder`.
fn part_paths(folder: &Path, stem: &str, parts: usize) -> Vec<PathBuf> {
    let width = parts.to_string().len();
    (1..=parts).map(|n| folder.join(format!("{stem}-{n:0width$}.pdf"))).collect()
}

impl Viewer {
    fn stem(&self) -> String {
        self.open()
            .and_then(|open| open.document.path())
            .and_then(|path| path.file_stem())
            .map_or_else(|| "document".to_owned(), |stem| stem.to_string_lossy().into_owned())
    }

    fn folder(&self) -> PathBuf {
        self.open()
            .and_then(|open| open.document.path())
            .and_then(|path| path.parent())
            .map(Path::to_owned)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default()
    }

    /// Whether the document has redactions, which saving makes final.
    fn has_redactions(&self) -> bool {
        self.open().is_some_and(|open| open.document.ops().iter().any(|op| matches!(op, Op::Redact { .. })))
    }

    /// Runs `then` at once, or after the reader agrees, when the document has redactions: the
    /// content under them is removed from the written file for good.
    fn confirm_redactions(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Viewer, &mut Window, &mut Context<Viewer>) + 'static,
    ) {
        if !self.has_redactions() {
            return then(self, window, cx);
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            t("The content under the redactions will be removed from the file for good."),
            None,
            &[t("Continue"), t("Cancel")],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                this.update_in(cx, |viewer, window, cx| then(viewer, window, cx)).ok();
            }
        })
        .detach();
    }

    /// Ctrl+S: replaces the original file, after confirmation.
    pub(super) fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.open() else { return };
        let Some(path) = open.document.path().map(Path::to_owned) else { return self.save_as(window, cx) };
        if !open.document.is_modified() {
            return;
        }
        let mut detail = path.display().to_string();
        if self.has_redactions() {
            detail = format!("{detail}\n\n{}", t("The content under the redactions will be removed from the file for good."));
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            t("Replace the original file with the edited document?"),
            Some(&detail),
            &[t("Replace"), t("Cancel")],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                this.update_in(cx, |viewer, window, cx| viewer.write_document(path, None, window, cx)).ok();
            }
        })
        .detach();
    }

    /// Ctrl+Shift+S: the system dialog asks before replacing an existing file.
    pub(super) fn save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open().is_none() {
            return;
        }
        self.confirm_redactions(window, cx, |viewer, window, cx| {
            let chosen = cx.prompt_for_new_path(&viewer.folder(), Some(&format!("{}.pdf", viewer.stem())));
            cx.spawn_in(window, async move |this, cx| {
                if let Ok(Ok(Some(path))) = chosen.await {
                    this.update_in(cx, |viewer, window, cx| viewer.write_document(path, None, window, cx)).ok();
                }
            })
            .detach();
        });
    }

    /// Saves a copy built with `options` (export).
    pub(super) fn export(&mut self, options: BuildOptions, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_redactions(window, cx, move |viewer, window, cx| {
            let chosen = cx.prompt_for_new_path(&viewer.folder(), Some(&format!("{}-export.pdf", viewer.stem())));
            cx.spawn_in(window, async move |this, cx| {
                if let Ok(Ok(Some(path))) = chosen.await {
                    this.update_in(cx, |viewer, window, cx| viewer.write_document(path, Some(options), window, cx)).ok();
                }
            })
            .detach();
        });
    }

    /// Builds the document as it is now and writes it to `path`; the document then stands for
    /// that file. With `export` options, writes a copy built with them instead: the document
    /// keeps its own path and unsaved state.
    pub(super) fn write_document(&mut self, path: PathBuf, export: Option<BuildOptions>, window: &mut Window, cx: &mut Context<Self>) {
        let State::Open(open) = &self.state else { return };
        let snapshot = open.document.clone();
        let cursor = snapshot.cursor();
        let target = path.clone();
        let exporting = export.is_some();
        let task = cx.background_executor().spawn(async move {
            let bytes = snapshot.build_with(&export.unwrap_or_default()).map_err(|error| error.to_string())?;
            write_atomic(&target, &bytes).map_err(|error| error.to_string())?;
            Ok::<usize, String>(bytes.len())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |viewer, window, cx| {
                match result {
                    Ok(size) if exporting => {
                        let original = viewer.open().map_or(0, |open| open.document.bytes().len());
                        viewer.notice = Some(
                            format!("{} {} ({}; {} {})", t("Exported to"), path.display(), file_size(size), t("opened file:"), file_size(original))
                                .into(),
                        );
                    }
                    Ok(_) => {
                        if let State::Open(open) = &mut viewer.state {
                            open.document.mark_saved_at(cursor);
                            open.document.set_path(path.clone());
                        }
                        viewer.recent = recent::record(&path);
                        viewer.notice = Some(format!("{} {}", t("Saved to"), path.display()).into());
                    }
                    Err(error) => viewer.notice = Some(failure("Cannot save the document", error)),
                }
                viewer.update_title(window);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Writes the pages of `spec` (1-based, as typed) to a new file.
    pub(super) fn extract(&mut self, spec: &str, window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let pages = self.open().map(|open| open.document.pages()).unwrap_or_default();
        let chosen: Vec<PageState> =
            split::parse_pages(spec, pages.len()).map_err(|error| error.to_string())?.into_iter().map(|i| pages[i]).collect();
        let path = cx.prompt_for_new_path(&self.folder(), Some(&format!("{}-extract.pdf", self.stem())));
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(path))) = path.await {
                this.update_in(cx, |viewer, window, cx| viewer.write_parts(vec![(path, chosen)], window, cx)).ok();
            }
        })
        .detach();
        Ok(())
    }

    /// Splits into several files in a folder the reader picks. Existing files are never replaced.
    pub(super) fn split(&mut self, mode: SplitMode, value: &str, window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let Some(open) = self.open() else { return Ok(()) };
        let pages = open.document.pages();
        let count = pages.len();
        let groups = match mode {
            SplitMode::Ranges => split::parse_ranges(value, count).map_err(|error| error.to_string())?,
            SplitMode::Every => {
                let size: usize = value.trim().parse().ok().filter(|&n| n > 0).ok_or_else(|| t("Enter a number of pages.").to_owned())?;
                split::every(size, count)
            }
            SplitMode::Bookmarks => {
                let starts: Vec<usize> = open
                    .info
                    .as_ref()
                    .map(|info| info.outline.iter().filter(|entry| entry.depth == 0).filter_map(|entry| entry.page).collect())
                    .unwrap_or_default();
                if starts.is_empty() {
                    return Err(t("This document has no outline.").to_owned());
                }
                split::at_starts(&starts, count)
            }
        };
        if groups.len() < 2 {
            return Err(t("That would make a single file; use Save as instead.").to_owned());
        }
        let stem = self.stem();
        let options = PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some(t("Split here").into()) };
        let folder = cx.prompt_for_paths(options);
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(folders))) = folder.await else { return };
            let Some(folder) = folders.into_iter().next() else { return };
            let paths = part_paths(&folder, &stem, groups.len());
            let parts = paths.into_iter().zip(groups.into_iter().map(|group| group.into_iter().map(|i| pages[i]).collect())).collect();
            this.update_in(cx, |viewer, window, cx| viewer.write_parts(parts, window, cx)).ok();
        })
        .detach();
        Ok(())
    }

    /// Builds and writes each `(path, pages)` part; refuses if any target already exists.
    fn write_parts(&mut self, parts: Vec<(PathBuf, Vec<PageState>)>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.open() else { return };
        let snapshot = open.document.clone();
        let task = cx.background_executor().spawn(async move {
            if let Some((existing, _)) = parts.iter().find(|(path, _)| path.exists()) {
                return Err(format!("{} {}", t("A file already exists:"), existing.display()));
            }
            for (path, pages) in &parts {
                let bytes = snapshot.build_pages(pages).map_err(|error| error.to_string())?;
                write_atomic(path, &bytes).map_err(|error| format!("{}: {error}", path.display()))?;
            }
            Ok(parts.len())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |viewer, _, cx| {
                viewer.notice = Some(match result {
                    Ok(1) => t("Pages extracted.").into(),
                    Ok(count) => format!("{count} {}", t("files written.")).into(),
                    Err(error) => failure("Cannot write the files", error),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Runs `then` now if there are no unsaved changes, else after the reader agrees to lose them.
    pub(super) fn confirm_discard(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Viewer, &mut Window, &mut Context<Viewer>) + 'static,
    ) {
        let modified = self.open().is_some_and(|open| open.document.is_modified());
        if !modified {
            return then(self, window, cx);
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            t("Discard the unsaved changes?"),
            None,
            &[t("Discard"), t("Cancel")],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                this.update_in(cx, |viewer, window, cx| then(viewer, window, cx)).ok();
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_names_sort_in_order() {
        let paths = part_paths(Path::new("out"), "report", 12);
        assert_eq!(paths[0], Path::new("out").join("report-01.pdf"));
        assert_eq!(paths[11], Path::new("out").join("report-12.pdf"));
        assert_eq!(part_paths(Path::new("."), "a", 3)[2], Path::new(".").join("a-3.pdf"));
    }
}
