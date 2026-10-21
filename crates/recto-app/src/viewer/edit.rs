//! Editing: actions become ops, the document is rebuilt in the background and shown again;
//! undo and redo; the `--apply` test flag.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use gpui::{Context, PathPromptOptions, Window};
use recto_core::annot::{Annotation, Color, Font, Kind};
use recto_core::geom::{PageRect, Point, Size};
use recto_core::op::{Op, PageState, Rotation};
use recto_core::{BuildError, PageId};
use recto_render::DocInfo;

use super::annotate::Tool;
use super::rail::move_target;
use super::{Known, Open, State, Viewer, failure};
use crate::i18n::t;

/// One edit from the `--apply` test flag; pages are 1-based positions at the time it applies.
#[derive(Clone, Debug, PartialEq)]
pub enum EditSpec {
    /// Quarter turn clockwise.
    Rotate(usize),
    Delete(usize),
    /// Moves a page before another one (`page_count + 1` for the end).
    Move { page: usize, before: usize },
    /// A blank page after this one (0 for the start).
    Blank(usize),
    Undo,
    /// An annotation drawn with `tool` on page `page`, through `points` (displayed page points);
    /// `text` for text boxes.
    Draw { tool: Tool, page: usize, points: Vec<Point>, text: String },
    /// A form field value.
    Fill { name: String, value: String },
}

/// `rotate:2,delete:3,move:4>1,blank:0,undo`, and annotations in displayed page points:
/// `rectangle:P:X:Y:W:H`, `ellipse:…`, `highlight:P:X:Y:W:H`, `arrow:P:X1:Y1:X2:Y2`,
/// `line:…`, `freehand:P:X:Y:X:Y…`, `text:P:X:Y:TEXT`, and `fill:NAME=VALUE`.
pub fn parse_edits(spec: &str) -> Result<Vec<EditSpec>, String> {
    let number = |text: &str| text.trim().parse::<usize>().map_err(|_| format!("not a page number: {text}"));
    let coordinate = |text: &str| text.trim().parse::<f32>().map_err(|_| format!("not a coordinate: {text}"));
    spec.split(',')
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            let (name, value) = part.split_once(':').unwrap_or((part, ""));
            match name.trim() {
                "rotate" => Ok(EditSpec::Rotate(number(value)?)),
                "delete" => Ok(EditSpec::Delete(number(value)?)),
                "blank" => Ok(EditSpec::Blank(number(value)?)),
                "undo" => Ok(EditSpec::Undo),
                "move" => {
                    let (page, before) = value.split_once('>').ok_or_else(|| format!("expected move:N>M, got {part}"))?;
                    Ok(EditSpec::Move { page: number(page)?, before: number(before)? })
                }
                "fill" => {
                    let (name, value) = value.split_once('=').ok_or_else(|| format!("expected fill:NAME=VALUE, got {part}"))?;
                    Ok(EditSpec::Fill { name: name.to_owned(), value: value.to_owned() })
                }
                "text" => {
                    let mut fields = value.splitn(4, ':');
                    let (page, x, y, text) = (fields.next(), fields.next(), fields.next(), fields.next());
                    let (Some(page), Some(x), Some(y), Some(text)) = (page, x, y, text) else {
                        return Err(format!("expected text:P:X:Y:TEXT, got {part}"));
                    };
                    let at = Point { x: coordinate(x)?, y: coordinate(y)? };
                    Ok(EditSpec::Draw { tool: Tool::Text, page: number(page)?, points: vec![at], text: text.to_owned() })
                }
                tool_name => {
                    let tool = Tool::parse(tool_name).ok_or_else(|| format!("unknown edit: {tool_name}"))?;
                    let mut fields = value.split(':');
                    let page = number(fields.next().unwrap_or_default())?;
                    let numbers = fields.map(coordinate).collect::<Result<Vec<f32>, _>>()?;
                    let points: Vec<Point> = match tool {
                        // X:Y:W:H → two corners.
                        Tool::Rectangle | Tool::Ellipse | Tool::Highlight | Tool::Underline | Tool::StrikeOut
                            if numbers.len() == 4 =>
                        {
                            vec![Point { x: numbers[0], y: numbers[1] }, Point { x: numbers[0] + numbers[2], y: numbers[1] + numbers[3] }]
                        }
                        Tool::Line | Tool::Arrow | Tool::Freehand if numbers.len() >= 4 && numbers.len() % 2 == 0 => {
                            numbers.chunks(2).map(|pair| Point { x: pair[0], y: pair[1] }).collect()
                        }
                        _ => return Err(format!("wrong coordinates for {tool_name}: {part}")),
                    };
                    Ok(EditSpec::Draw { tool, page, points, text: String::new() })
                }
            }
        })
        .collect()
}

/// What a background build produces: the new file and what is known about its pages.
type Built = Result<(Vec<u8>, Known), BuildError>;

impl Viewer {
    /// Page identities in the order shown.
    pub(super) fn page_order(&self) -> Vec<PageId> {
        self.open().map(|open| open.page_states.iter().map(|page| page.id).collect()).unwrap_or_default()
    }

    /// Pages an action applies to: the rail selection, or else the current page.
    pub(super) fn target_pages(&self) -> Vec<PageId> {
        let order = self.page_order();
        let selected = self.rail.in_order(&order);
        if !selected.is_empty() {
            return selected;
        }
        order.get(self.current_page()).copied().into_iter().collect()
    }

    /// Records `op` and shows the result once rebuilt.
    pub(super) fn edit(&mut self, op: Op, window: &mut Window, cx: &mut Context<Self>) {
        let State::Open(open) = &mut self.state else { return };
        open.document.apply(op);
        self.rebuild(window, cx);
    }

    pub(super) fn rotate(&mut self, rotation: Rotation, window: &mut Window, cx: &mut Context<Self>) {
        let pages = self.target_pages();
        if !pages.is_empty() {
            self.edit(Op::RotatePages { pages, rotation }, window, cx);
        }
    }

    pub(super) fn delete_pages(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pages = self.target_pages();
        if pages.is_empty() {
            return;
        }
        if pages.len() >= self.page_order().len() {
            self.notice = Some(t("A document needs at least one page.").into());
            cx.notify();
            return;
        }
        self.rail.clear();
        self.edit(Op::DeletePages { pages }, window, cx);
    }

    /// A blank page the size of the current one, after it.
    pub(super) fn insert_blank(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.current_page();
        let State::Open(open) = &mut self.state else { return };
        let size = open
            .info
            .as_ref()
            .and_then(|info| info.page_sizes.get(current).copied())
            .unwrap_or(Size { width: 595.0, height: 842.0 });
        let page = open.document.add_blank(size);
        self.edit(Op::InsertPages { pages: vec![page], at: current + 1 }, window, cx);
    }

    /// Moves `pages` before the page shown at `before`.
    pub(super) fn move_pages(&mut self, pages: Vec<PageId>, before: usize, window: &mut Window, cx: &mut Context<Self>) {
        let order = self.page_order();
        let to = move_target(&order, &pages, before);
        let unchanged = {
            let rest: Vec<PageId> = order.iter().copied().filter(|page| !pages.contains(page)).collect();
            let mut result = rest.clone();
            result.splice(to.min(rest.len())..to.min(rest.len()), pages.iter().copied());
            result == order
        };
        if !pages.is_empty() && !unchanged {
            self.edit(Op::MovePages { pages, to }, window, cx);
        }
    }

    /// Merges PDF files, inserting their pages at position `at`.
    pub(super) fn insert_files(&mut self, paths: &[PathBuf], at: usize, window: &mut Window, cx: &mut Context<Self>) {
        let State::Open(open) = &mut self.state else { return };
        let mut pages = Vec::new();
        for path in paths {
            match open.document.add_path(path) {
                Ok(added) => pages.extend(added),
                Err(error) => {
                    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                    self.notice = Some(failure("Cannot insert the file", format!("{name}: {error}")));
                }
            }
        }
        if pages.is_empty() {
            cx.notify();
        } else {
            self.edit(Op::InsertPages { pages, at }, window, cx);
        }
    }

    /// Asks for PDF files and inserts them after the current page.
    pub(super) fn prompt_insert_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let at = self.current_page() + 1;
        let chosen = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await {
                this.update_in(cx, |viewer, window, cx| viewer.insert_files(&paths, at, window, cx)).ok();
            }
        })
        .detach();
    }

    pub(super) fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let State::Open(open) = &mut self.state
            && open.document.undo()
        {
            self.rebuild(window, cx);
        }
    }

    pub(super) fn redo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let State::Open(open) = &mut self.state
            && open.document.redo()
        {
            self.rebuild(window, cx);
        }
    }

    /// Applies `--apply` edits right after opening.
    pub(super) fn apply_specs(&mut self, specs: &[EditSpec], window: &mut Window, cx: &mut Context<Self>) {
        let inspector = self.inspector;
        let State::Open(open) = &mut self.state else { return };
        let document = &mut open.document;
        for spec in specs {
            let order: Vec<PageId> = document.pages().iter().map(|page| page.id).collect();
            let page = |n: usize| n.checked_sub(1).and_then(|i| order.get(i).copied());
            match *spec {
                EditSpec::Rotate(n) => {
                    if let Some(id) = page(n) {
                        document.apply(Op::RotatePages { pages: vec![id], rotation: Rotation::R90 });
                    }
                }
                EditSpec::Delete(n) => {
                    if let Some(id) = page(n).filter(|_| order.len() > 1) {
                        document.apply(Op::DeletePages { pages: vec![id] });
                    }
                }
                EditSpec::Move { page: n, before } => {
                    if let Some(id) = page(n) {
                        let to = move_target(&order, &[id], before.saturating_sub(1));
                        document.apply(Op::MovePages { pages: vec![id], to });
                    }
                }
                EditSpec::Blank(after) => {
                    let blank = document.add_blank(Size { width: 595.0, height: 842.0 });
                    document.apply(Op::InsertPages { pages: vec![blank], at: after });
                }
                EditSpec::Undo => {
                    document.undo();
                }
                EditSpec::Fill { ref name, ref value } => {
                    document.apply(Op::FillField { name: name.clone(), value: value.clone() });
                }
                EditSpec::Draw { tool, page: n, ref points, ref text } => {
                    let pages = document.pages();
                    let (Some(state), Ok(geometry)) = (n.checked_sub(1).and_then(|i| pages.get(i)), document.geometry_of(&pages))
                    else {
                        continue;
                    };
                    let page = geometry[n - 1];
                    let user: Vec<_> = points.iter().map(|&p| page.to_user(p)).collect();
                    let area = |a: Point, b: Point| {
                        page.rect_to_user(PageRect { x: a.x.min(b.x), y: a.y.min(b.y), width: (a.x - b.x).abs(), height: (a.y - b.y).abs() })
                    };
                    let kind = match tool {
                        Tool::Rectangle => Kind::Rectangle { rect: area(points[0], points[1]) },
                        Tool::Ellipse => Kind::Ellipse { rect: area(points[0], points[1]) },
                        Tool::Highlight => Kind::Highlight { quads: vec![area(points[0], points[1])] },
                        Tool::Underline => Kind::Underline { quads: vec![area(points[0], points[1])] },
                        Tool::StrikeOut => Kind::StrikeOut { quads: vec![area(points[0], points[1])] },
                        Tool::Line | Tool::Arrow => Kind::Line { from: user[0], to: user[1], arrow: tool == Tool::Arrow },
                        Tool::Freehand => Kind::Ink { strokes: vec![user] },
                        Tool::Text => {
                            let size = inspector.size;
                            let at = points[0];
                            let width = recto_core::annot::text_width(text, size, Font::Helvetica) + 6.0;
                            let rect = page.rect_to_user(PageRect { x: at.x, y: at.y, width, height: size * 1.3 + 4.0 });
                            Kind::Text { rect, text: text.clone(), size, font: Font::Helvetica, upright: page.rotate }
                        }
                        _ => continue,
                    };
                    let color = if tool == Tool::Highlight { Color::YELLOW } else { inspector.color };
                    let annotation = Annotation { kind, color, width: inspector.width };
                    document.apply(Op::AddAnnotation { page: state.id, annotation });
                }
            }
        }
        self.rebuild(window, cx);
    }

    /// Builds the current state in the background; only the latest build is shown.
    pub(super) fn rebuild(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let State::Open(open) = &self.state else { return };
        self.build_version += 1;
        let version = self.build_version;
        let snapshot = open.document.clone();
        let task = cx.background_executor().spawn(async move {
            let pages = snapshot.pages();
            let bytes = snapshot.build()?;
            let geometry = snapshot.geometry_of(&pages)?;
            let fields = snapshot.form_fields()?;
            Ok((bytes, Known::with(&snapshot, pages, geometry, fields)))
        });
        cx.spawn_in(window, async move |this, cx| {
            let built: Built = task.await;
            this.update_in(cx, |viewer, window, cx| viewer.finish_build(version, built, window, cx)).ok();
        })
        .detach();
        self.update_title(window);
        cx.notify();
    }

    fn finish_build(&mut self, version: u64, built: Built, window: &mut Window, cx: &mut Context<Self>) {
        if version != self.build_version {
            return; // a later edit supersedes this build
        }
        match built {
            Ok((bytes, known)) => self.reload(bytes.into(), known, window, cx),
            Err(error) => {
                // Keep the log consistent with what is shown: the failed edit is undone.
                if let State::Open(open) = &mut self.state {
                    open.document.undo();
                }
                self.notice = Some(failure("Cannot apply the change", error));
                self.update_title(window);
                cx.notify();
            }
        }
    }

    /// Shows rebuilt bytes: a new render thread, with the bitmaps of unchanged pages carried over
    /// and the layout ready at once (no flash while PDFium reopens the file).
    fn reload(&mut self, bytes: Arc<[u8]>, known: Known, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.current_page();
        let State::Open(old) = std::mem::replace(&mut self.state, State::Empty) else { return };
        let Open {
            document,
            page_states: old_pages,
            marks: old_marks,
            pages: mut old_bitmaps,
            thumbnails: mut old_thumbnails,
            info,
            renderer,
            pending_page,
            ..
        } = *old;
        // Stop the old renderer before starting the new one.
        drop(renderer);
        // A page asked for but not shown yet (`--page` before the first layout) wins.
        let anchor = old_pages.get(pending_page.unwrap_or(current)).map(|page| page.id);
        let shown = anchor.and_then(|id| known.pages.iter().position(|page| page.id == id)).unwrap_or(current);
        let shown = shown.min(known.pages.len().saturating_sub(1));
        let page_sizes = known.geometry.iter().map(|page| page.size()).collect();
        let carried: Vec<(usize, usize)> = {
            // Same page, same rotation, same annotations: the old bitmap is still right.
            let old_index: HashMap<(PageState, &[u64]), usize> =
                old_pages.iter().zip(&old_marks).enumerate().map(|(i, (page, marks))| ((*page, marks.as_slice()), i)).collect();
            known
                .pages
                .iter()
                .zip(&known.marks)
                .enumerate()
                .filter_map(|(index, (page, marks))| old_index.get(&(*page, marks.as_slice())).map(|&old| (old, index)))
                .collect()
        };
        match self.start(document, bytes, shown, Some(known), window, cx) {
            Ok(mut open) => {
                for (old, index) in carried {
                    if let Some(bitmap) = old_bitmaps.remove(&old) {
                        open.pages.insert(index, bitmap);
                    }
                    if let Some(thumbnail) = old_thumbnails.remove(&old) {
                        open.thumbnails.insert(index, thumbnail);
                    }
                }
                for (_, bitmap) in old_bitmaps {
                    cx.drop_image(bitmap.image, Some(window));
                }
                for (_, thumbnail) in old_thumbnails {
                    cx.drop_image(thumbnail, Some(window));
                }
                // Sizes are known from the geometry; the outline is refreshed when PDFium answers.
                let outline = info.map(|info| info.outline).unwrap_or_default();
                open.info = Some(DocInfo { page_sizes, outline });
                self.rail.retain_existing(&self.page_order_of(&open));
                self.selection = None;
                self.search.restart();
                self.state = State::Open(Box::new(open));
                if self.search.open {
                    self.advance_search(window, cx);
                }
            }
            Err(message) => self.state = State::Error(message),
        }
        self.update_title(window);
        cx.notify();
    }

    fn page_order_of(&self, open: &Open) -> Vec<PageId> {
        open.page_states.iter().map(|page| page.id).collect()
    }

    /// "name — Recto", with a bullet while there are unsaved changes.
    pub(super) fn update_title(&self, window: &mut Window) {
        let Some(open) = self.open() else { return };
        let name = open
            .document
            .path()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dirty = if open.document.is_modified() { "• " } else { "" };
        window.set_window_title(&format!("{dirty}{name} — {}", t("Recto")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_edit_specs() {
        assert_eq!(
            parse_edits("rotate:2, delete:3,move:4>1,blank:0,undo"),
            Ok(vec![
                EditSpec::Rotate(2),
                EditSpec::Delete(3),
                EditSpec::Move { page: 4, before: 1 },
                EditSpec::Blank(0),
                EditSpec::Undo
            ])
        );
        assert!(parse_edits("spin:1").is_err());
        assert!(parse_edits("move:3").is_err());
        assert!(parse_edits("delete:x").is_err());
    }
}
