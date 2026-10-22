//! Annotations already in a file (not made in Recto): listed so they can be picked, then moved,
//! resized or deleted (`Op::EditFileAnnotation`). One is known by its place in its page's
//! `/Annots` array in the source file, which never changes.
//!
//! Moving one sets its `/Rect`, which carries the appearance stream along, and maps the points
//! it holds (`/QuadPoints`, `/InkList`, `/L`, `/Vertices`, `/CL`) the same way, so viewers that
//! redraw it from them agree. Form fields (widgets) and pop-up notes are left out.

use lopdf::{Object, ObjectId};

use crate::content::{dictionary, resolve};
use crate::geom::{Point, Rect};

/// What is done to an annotation of the file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FileChange {
    /// Moved or resized to this frame (user space).
    Frame(Rect),
    Delete,
}

/// An annotation of the file, as it stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FileAnnotation {
    /// Its place in the page's `/Annots` in the source file.
    pub index: u32,
    pub frame: Rect,
}

/// The kinds left alone: form fields are filled, not moved; pop-ups follow their note.
fn skipped(subtype: &[u8]) -> bool {
    matches!(subtype, b"Widget" | b"Popup")
}

fn rect_of(document: &lopdf::Document, value: &Object) -> Option<Rect> {
    let numbers: Vec<f32> = resolve(document, value).as_array().ok()?.iter().filter_map(|value| resolve(document, value).as_float().ok()).collect();
    let [x0, y0, x1, y1] = numbers.as_slice() else { return None };
    Some(Rect { origin: Point { x: x0.min(*x1), y: y0.min(*y1) }, width: (x1 - x0).abs(), height: (y1 - y0).abs() })
}

/// The annotations of `page` (an object of `document`) that can be picked, by place.
pub fn read(document: &lopdf::Document, page: ObjectId) -> Vec<FileAnnotation> {
    let Some(annots) = document.get_dictionary(page).ok().and_then(|page| page.get(b"Annots").ok()).map(|value| resolve(document, value)) else {
        return Vec::new();
    };
    let Ok(annots) = annots.as_array() else { return Vec::new() };
    let mut found = Vec::new();
    for (index, value) in annots.iter().enumerate() {
        let Some(annotation) = dictionary(document, value) else { continue };
        let subtype = annotation.get(b"Subtype").ok().and_then(|value| value.as_name().ok()).unwrap_or_default();
        // Hidden ones (flag 2) are not shown, so not picked.
        let hidden = annotation.get(b"F").ok().and_then(|value| value.as_i64().ok()).is_some_and(|flags| flags & 2 != 0);
        if skipped(subtype) || hidden {
            continue;
        }
        if let Some(frame) = annotation.get(b"Rect").ok().and_then(|value| rect_of(document, value)) {
            found.push(FileAnnotation { index: index as u32, frame });
        }
    }
    found
}

/// `annotations` with `changes` (by place, in order) applied.
pub fn current(mut annotations: Vec<FileAnnotation>, changes: &[(u32, FileChange)]) -> Vec<FileAnnotation> {
    for &(index, change) in changes {
        match change {
            FileChange::Delete => annotations.retain(|annotation| annotation.index != index),
            FileChange::Frame(frame) => {
                if let Some(annotation) = annotations.iter_mut().find(|annotation| annotation.index == index) {
                    annotation.frame = frame;
                }
            }
        }
    }
    annotations
}

/// Applies `changes` (by place in the original `/Annots`, in order) to `page` of `document`.
pub fn apply(document: &mut lopdf::Document, page: ObjectId, changes: &[(u32, FileChange)]) -> lopdf::Result<()> {
    if changes.is_empty() {
        return Ok(());
    }
    let annots = match document.get_dictionary(page)?.get(b"Annots") {
        Ok(Object::Reference(id)) => document.get_object(*id)?.as_array()?.clone(),
        Ok(Object::Array(items)) => items.clone(),
        _ => return Ok(()),
    };
    let mut deleted = vec![false; annots.len()];
    for &(index, change) in changes {
        let Some(item) = annots.get(index as usize) else { continue };
        match change {
            FileChange::Delete => deleted[index as usize] = true,
            FileChange::Frame(to) => {
                let annotation = match item {
                    Object::Reference(id) => document.get_dictionary_mut(*id)?,
                    // Inline annotations are rare; they are left where they are.
                    _ => continue,
                };
                let Some(from) = annotation.get(b"Rect").ok().and_then(rect_of_plain) else { continue };
                let map = |x: f32, y: f32| -> (f32, f32) {
                    let sx = if from.width.abs() > 1e-3 { to.width / from.width } else { 1.0 };
                    let sy = if from.height.abs() > 1e-3 { to.height / from.height } else { 1.0 };
                    (to.origin.x + (x - from.origin.x) * sx, to.origin.y + (y - from.origin.y) * sy)
                };
                let corners = [to.origin.x, to.origin.y, to.origin.x + to.width, to.origin.y + to.height];
                annotation.set("Rect", Object::Array(corners.into_iter().map(Object::Real).collect()));
                for key in [b"QuadPoints".as_slice(), b"L", b"Vertices", b"CL"] {
                    if let Ok(Object::Array(points)) = annotation.get(key) {
                        annotation.set(key.to_vec(), Object::Array(mapped_points(points, &map)));
                    }
                }
                if let Ok(Object::Array(strokes)) = annotation.get(b"InkList") {
                    let strokes = strokes
                        .iter()
                        .map(|stroke| match stroke {
                            Object::Array(points) => Object::Array(mapped_points(points, &map)),
                            other => other.clone(),
                        })
                        .collect();
                    annotation.set("InkList", Object::Array(strokes));
                }
            }
        }
    }
    if deleted.iter().any(|&gone| gone) {
        let kept: Vec<Object> = annots.into_iter().zip(&deleted).filter(|(_, gone)| !**gone).map(|(item, _)| item).collect();
        document.get_dictionary_mut(page)?.set("Annots", Object::Array(kept));
    }
    Ok(())
}

/// A rectangle written directly (no references), as `apply` finds it.
fn rect_of_plain(value: &Object) -> Option<Rect> {
    let numbers: Vec<f32> = value.as_array().ok()?.iter().filter_map(|value| value.as_float().ok()).collect();
    let [x0, y0, x1, y1] = numbers.as_slice() else { return None };
    Some(Rect { origin: Point { x: x0.min(*x1), y: y0.min(*y1) }, width: (x1 - x0).abs(), height: (y1 - y0).abs() })
}

/// `points` (x y x y …) mapped by `map`; anything that is not a number pair is kept.
fn mapped_points(points: &[Object], map: &impl Fn(f32, f32) -> (f32, f32)) -> Vec<Object> {
    let numbers: Vec<f32> = points.iter().filter_map(|value| value.as_float().ok()).collect();
    if numbers.len() != points.len() || !numbers.len().is_multiple_of(2) {
        return points.to_vec();
    }
    numbers
        .chunks(2)
        .flat_map(|pair| {
            let (x, y) = map(pair[0], pair[1]);
            [Object::Real(x), Object::Real(y)]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use lopdf::Dictionary;

    use super::*;

    fn page_with(annotations: Vec<Dictionary>) -> (lopdf::Document, ObjectId) {
        let mut document = lopdf::Document::with_version("1.7");
        let refs: Vec<Object> = annotations.into_iter().map(|annotation| Object::Reference(document.add_object(annotation))).collect();
        let mut page = Dictionary::new();
        page.set("Type", Object::Name(b"Page".to_vec()));
        page.set("Annots", Object::Array(refs));
        let page = document.add_object(page);
        (document, page)
    }

    fn annotation(subtype: &str, rect: [f32; 4]) -> Dictionary {
        let mut annotation = Dictionary::new();
        annotation.set("Subtype", Object::Name(subtype.as_bytes().to_vec()));
        annotation.set("Rect", Object::Array(rect.into_iter().map(Object::Real).collect()));
        annotation
    }

    #[test]
    fn lists_moves_and_deletes_by_place() {
        let mut ink = annotation("Ink", [10.0, 10.0, 30.0, 30.0]);
        ink.set("InkList", Object::Array(vec![Object::Array(vec![Object::Real(10.0), Object::Real(10.0), Object::Real(30.0), Object::Real(30.0)])]));
        let (mut document, page) = page_with(vec![annotation("Widget", [0.0, 0.0, 5.0, 5.0]), ink, annotation("Square", [50.0, 50.0, 60.0, 60.0])]);
        let listed = read(&document, page);
        assert_eq!(listed.iter().map(|annotation| annotation.index).collect::<Vec<_>>(), [1, 2]);

        let to = Rect { origin: Point { x: 100.0, y: 100.0 }, width: 40.0, height: 40.0 };
        let changes = [(1, FileChange::Frame(to)), (2, FileChange::Delete)];
        assert_eq!(current(listed, &changes), [FileAnnotation { index: 1, frame: to }]);

        apply(&mut document, page, &changes).unwrap();
        let after = read(&document, page);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].frame, to);
        // The stroke followed: (30, 30) is the far corner, now (140, 140).
        let annots = document.get_dictionary(page).unwrap().get(b"Annots").unwrap().as_array().unwrap().clone();
        let ink = document.get_dictionary(annots[1].as_reference().unwrap()).unwrap();
        let stroke = ink.get(b"InkList").unwrap().as_array().unwrap()[0].as_array().unwrap().clone();
        let last: Vec<f32> = stroke[2..].iter().map(|value| value.as_float().unwrap()).collect();
        assert_eq!(last, [140.0, 140.0]);
    }
}
