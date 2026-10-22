//! Links of a page: those already in the file (URI actions, page destinations, named
//! destinations) and those added in Recto, for the viewer to follow.

use lopdf::{Dictionary, Object, ObjectId};

use crate::PageId;
use crate::annot::LinkTarget;
use crate::content::{dictionary, resolve};
use crate::geom::{Point, Rect};

const MAX_DEPTH: usize = 32;

/// A destination (array, or a name to look up) as a page of the source.
fn destination(document: &lopdf::Document, value: &Object, pages: &[ObjectId], source: u32, depth: usize) -> Option<PageId> {
    if depth > MAX_DEPTH {
        return None;
    }
    match resolve(document, value) {
        Object::Array(items) => {
            let page = items.first()?.as_reference().ok()?;
            let index = pages.iter().position(|id| *id == page)?;
            Some(PageId { source, index: index as u32 })
        }
        // A destination dictionary: its `/D`.
        Object::Dictionary(dict) => destination(document, dict.get(b"D").ok()?, pages, source, depth + 1),
        Object::Name(name) | Object::String(name, _) => {
            let catalog = document.catalog().ok()?;
            // PDF 1.1 `/Dests` dictionary, then the PDF 1.2 name tree.
            if let Some(found) = catalog.get(b"Dests").ok().and_then(|value| dictionary(document, value)).and_then(|dests| dests.get(name).ok()) {
                return destination(document, found, pages, source, depth + 1);
            }
            let tree = catalog
                .get(b"Names")
                .ok()
                .and_then(|value| dictionary(document, value))
                .and_then(|names| names.get(b"Dests").ok())
                .and_then(|value| dictionary(document, value))?;
            let found = lookup(document, tree, name, 0)?;
            destination(document, found, pages, source, depth + 1)
        }
        _ => None,
    }
}

/// Looks `key` up in a name tree.
fn lookup<'a>(document: &'a lopdf::Document, node: &'a Dictionary, key: &[u8], depth: usize) -> Option<&'a Object> {
    if depth > MAX_DEPTH {
        return None;
    }
    if let Ok(names) = node.get(b"Names").map(|value| resolve(document, value)).and_then(Object::as_array) {
        for pair in names.chunks(2) {
            if let [name, value] = pair
                && let Ok(name) = resolve(document, name).as_str()
                && name == key
            {
                return Some(value);
            }
        }
    }
    let kids = node.get(b"Kids").map(|value| resolve(document, value)).and_then(Object::as_array).ok()?;
    kids.iter().filter_map(|kid| dictionary(document, kid)).find_map(|kid| lookup(document, kid, key, depth + 1))
}

/// The links of page `page` (an object of `document`, whose pages are `pages` in order).
pub fn read(document: &lopdf::Document, page: ObjectId, pages: &[ObjectId], source: u32) -> Vec<(Rect, LinkTarget)> {
    let Some(annots) = document
        .get_dictionary(page)
        .ok()
        .and_then(|page| page.get(b"Annots").ok())
        .map(|value| resolve(document, value))
        .and_then(|value| value.as_array().ok())
    else {
        return Vec::new();
    };
    let mut links = Vec::new();
    for annotation in annots.iter().filter_map(|value| dictionary(document, value)) {
        if annotation.get(b"Subtype").ok().and_then(|value| value.as_name().ok()) != Some(b"Link") {
            continue;
        }
        let Some(rect) = annotation.get(b"Rect").ok().map(|value| resolve(document, value)).and_then(|value| value.as_array().ok()).and_then(|items| {
            let numbers: Vec<f32> = items.iter().filter_map(|value| resolve(document, value).as_float().ok()).collect();
            (numbers.len() == 4).then(|| {
                let (x0, y0, x1, y1) = (numbers[0].min(numbers[2]), numbers[1].min(numbers[3]), numbers[0].max(numbers[2]), numbers[1].max(numbers[3]));
                Rect { origin: Point { x: x0, y: y0 }, width: x1 - x0, height: y1 - y0 }
            })
        }) else {
            continue;
        };
        let action = annotation.get(b"A").ok().and_then(|value| dictionary(document, value));
        let target = match action {
            Some(action) => match action.get(b"S").ok().and_then(|value| value.as_name().ok()) {
                Some(b"URI") => action
                    .get(b"URI")
                    .ok()
                    .and_then(|value| resolve(document, value).as_str().ok())
                    .map(|uri| LinkTarget::Uri(String::from_utf8_lossy(uri).into_owned())),
                Some(b"GoTo") => action.get(b"D").ok().and_then(|value| destination(document, value, pages, source, 0)).map(LinkTarget::Page),
                _ => None,
            },
            None => annotation.get(b"Dest").ok().and_then(|value| destination(document, value, pages, source, 0)).map(LinkTarget::Page),
        };
        if let Some(target) = target {
            links.push((rect, target));
        }
    }
    links
}
