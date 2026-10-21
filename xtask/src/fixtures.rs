//! Generated test fixtures. Output is deterministic, so the committed files can be checked against it.

use std::io::Write;
use std::path::Path;

use anyhow::Result;
use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref, Str, TextStr};

/// Committed fixtures, by file name.
type Fixture = (&'static str, fn() -> Vec<u8>);
/// Byte-for-byte reproducible fixtures; the up-to-date test compares them with the generators.
const FIXTURES: [Fixture; 3] =
    [("sample.pdf", sample_pdf), ("layout.pdf", layout_pdf), ("inherited.pdf", inherited_pdf)];
/// Encrypted: the `/U` entry ends with arbitrary bytes, which `lopdf` draws at random.
const PROTECTED: &str = "protected.pdf";

pub fn generate(root: &Path) -> Result<()> {
    let dir = root.join("tests/fixtures");
    std::fs::create_dir_all(&dir)?;
    for (name, build) in FIXTURES {
        std::fs::write(dir.join(name), build())?;
        println!("wrote {}", dir.join(name).display());
    }
    std::fs::write(dir.join(PROTECTED), protected_pdf())?;
    println!("wrote {}", dir.join(PROTECTED).display());
    Ok(())
}

/// `target/large.pdf`: many pages for scrolling and search checks. Not committed.
pub fn generate_large(root: &Path, pages: usize) -> Result<()> {
    let path = root.join("target/large.pdf");
    let specs: Vec<_> = (0..pages).map(|_| (A4, 0)).collect();
    std::fs::write(&path, text_pages(&specs, &[]))?;
    println!("wrote {} ({pages} pages)", path.display());
    Ok(())
}

const A4: Rect = Rect { x1: 0.0, y1: 0.0, x2: 595.0, y2: 842.0 };
const A4_LANDSCAPE: Rect = Rect { x1: 0.0, y1: 0.0, x2: 842.0, y2: 595.0 };
const LETTER: Rect = Rect { x1: 0.0, y1: 0.0, x2: 612.0, y2: 792.0 };
const IMAGE_SIZE: u32 = 64;

/// Mixed page geometry: A4 portrait, A4 landscape, A4 portrait with `/Rotate 90`, US Letter;
/// and a two-level outline.
pub fn layout_pdf() -> Vec<u8> {
    let outline = [("Portrait", 0, 0), ("Landscape", 1, 1), ("Rotated", 0, 2), ("Letter", 0, 3)];
    text_pages(&[(A4, 0), (A4_LANDSCAPE, 0), (A4, 90), (LETTER, 0)], &outline)
}

/// Three pages whose resources, media box and rotation are inherited from the page tree:
/// the root `Pages` holds the font and an A4 media box; a nested `Pages` overrides the media box
/// (US Letter) and adds `/Rotate 90` for pages 1 and 2; page 3 sits under the root and has its
/// own landscape media box and a crop box. Editing must copy these attributes onto the pages it
/// re-parents.
pub fn inherited_pdf() -> Vec<u8> {
    let catalog_id = Ref::new(1);
    let root_id = Ref::new(2);
    let middle_id = Ref::new(3);
    let font_id = Ref::new(4);
    let pages = [(Ref::new(10), middle_id), (Ref::new(12), middle_id), (Ref::new(14), root_id)];
    let font = Name(b"F1");

    let mut pdf = Pdf::new();
    pdf.catalog(catalog_id).pages(root_id);
    let mut root = pdf.pages(root_id);
    root.kids([middle_id, pages[2].0]).count(3);
    root.insert(Name(b"MediaBox")).array().items([0.0, 0.0, 595.0, 842.0]);
    root.insert(Name(b"Resources")).dict().insert(Name(b"Font")).dict().pair(font, font_id);
    root.finish();
    let mut middle = pdf.pages(middle_id);
    middle.parent(root_id).kids([pages[0].0, pages[1].0]).count(2);
    middle.insert(Name(b"MediaBox")).array().items([0.0, 0.0, 612.0, 792.0]);
    middle.pair(Name(b"Rotate"), 90);
    middle.finish();
    pdf.type1_font(font_id).base_font(Name(b"Helvetica"));

    for (n, &(page_id, parent)) in pages.iter().enumerate() {
        let content_id = Ref::new(page_id.get() + 1);
        let mut page = pdf.page(page_id);
        page.parent(parent).contents(content_id);
        if n == 2 {
            page.media_box(A4_LANDSCAPE);
            // A crop box, partly outside the media box: what shows is their intersection.
            page.crop_box(Rect::new(50.0, 40.0, 900.0, 545.0));
        }
        page.finish();
        let mut content = Content::new();
        content.begin_text().set_font(font, 24.0).next_line(56.0, 500.0);
        content.show(Str(format!("Inherited {}", n + 1).as_bytes()));
        content.end_text();
        pdf.stream(content_id, &content.finish());
    }
    pdf.finish()
}

/// User password of `protected.pdf`.
pub const PROTECTED_PASSWORD: &str = "recto";

/// One A4 page, RC4 128-bit encrypted with user password `recto` and owner password `owner`.
/// The file ID is fixed so the output stays deterministic.
pub fn protected_pdf() -> Vec<u8> {
    let plain = text_pages(&[(A4, 0)], &[]);
    let mut document = lopdf::Document::load_mem(&plain).expect("generated PDF parses");
    let id = lopdf::Object::String(b"recto-protected-fixture".to_vec(), lopdf::StringFormat::Hexadecimal);
    document.trailer.set("ID", lopdf::Object::Array(vec![id.clone(), id]));
    let version = lopdf::EncryptionVersion::V2 {
        document: &document,
        owner_password: "owner",
        user_password: PROTECTED_PASSWORD,
        key_length: 128,
        permissions: lopdf::Permissions::all(),
    };
    let state = lopdf::EncryptionState::try_from(version).expect("valid encryption settings");
    document.encrypt(&state).expect("document is not encrypted yet");
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).expect("writing to a Vec");
    bytes
}

/// One line of Helvetica per page, on pages of the given media box and rotation, plus an outline
/// of `(title, depth, page index)` entries in reading order.
fn text_pages(specs: &[(Rect, i32)], outline: &[(&str, usize, usize)]) -> Vec<u8> {
    let catalog_id = Ref::new(1);
    let tree_id = Ref::new(2);
    let font_id = Ref::new(3);
    let font = Name(b"F1");
    let page_ids: Vec<Ref> = (0..specs.len() as i32).map(|i| Ref::new(10 + 2 * i)).collect();

    let mut pdf = Pdf::new();
    let mut catalog = pdf.catalog(catalog_id);
    catalog.pages(tree_id);
    if !outline.is_empty() {
        catalog.outlines(Ref::new(4));
    }
    catalog.finish();
    pdf.pages(tree_id).kids(page_ids.iter().copied()).count(specs.len() as i32);
    pdf.type1_font(font_id).base_font(Name(b"Helvetica"));
    for (n, (&(media_box, rotate), &page_id)) in specs.iter().zip(&page_ids).enumerate() {
        let content_id = Ref::new(page_id.get() + 1);
        let mut page = pdf.page(page_id);
        page.media_box(media_box).parent(tree_id).contents(content_id);
        if rotate != 0 {
            page.rotate(rotate);
        }
        page.resources().fonts().pair(font, font_id);
        page.finish();

        let mut content = Content::new();
        content.begin_text().set_font(font, 24.0).next_line(56.0, media_box.y2 - 80.0);
        content.show(Str(format!("Page {} ({} x {}, rotate {rotate})", n + 1, media_box.x2, media_box.y2).as_bytes()));
        content.end_text();
        pdf.stream(content_id, &content.finish());
    }
    if !outline.is_empty() {
        write_outline(&mut pdf, Ref::new(4), Ref::new(10 + 2 * specs.len() as i32), &page_ids, outline);
    }
    pdf.finish()
}

/// Writes the outline dictionary at `root` (already referenced by the catalog) and its items,
/// numbered from `first_item`.
fn write_outline(pdf: &mut Pdf, root: Ref, first_item: Ref, pages: &[Ref], items: &[(&str, usize, usize)]) {
    let id = |i: usize| Ref::new(first_item.get() + i as i32);
    // Parent of each item: the closest earlier item one level up.
    let parents: Vec<Option<usize>> =
        (0..items.len()).map(|i| (0..i).rev().find(|&j| items[j].1 + 1 == items[i].1)).collect();
    let children = |parent: Option<usize>| -> Vec<usize> { (0..items.len()).filter(|&i| parents[i] == parent).collect() };

    let top = children(None);
    let mut outline = pdf.outline(root);
    if let (Some(&first), Some(&last)) = (top.first(), top.last()) {
        outline.first(id(first)).last(id(last));
    }
    outline.count(items.len() as i32);
    outline.finish();

    for (i, &(title, _, page)) in items.iter().enumerate() {
        let siblings = children(parents[i]);
        let at = siblings.iter().position(|&s| s == i).expect("an item is its parent's child");
        let own = children(Some(i));
        let mut item = pdf.outline_item(id(i));
        item.title(TextStr(title)).parent(parents[i].map_or(root, id));
        if at > 0 {
            item.prev(id(siblings[at - 1]));
        }
        if let Some(&next) = siblings.get(at + 1) {
            item.next(id(next));
        }
        if let (Some(&first), Some(&last)) = (own.first(), own.last()) {
            item.first(id(first)).last(id(last)).count(own.len() as i32);
        }
        item.dest().page(pages[page]).xyz(0.0, 842.0, None);
    }
}

/// Three A4 pages of Helvetica text; page 1 also carries a colour band and a gradient image.
pub fn sample_pdf() -> Vec<u8> {
    let catalog_id = Ref::new(1);
    let tree_id = Ref::new(2);
    let font_id = Ref::new(3);
    let image_id = Ref::new(4);
    let page_ids = [Ref::new(10), Ref::new(11), Ref::new(12)];
    let content_ids = [Ref::new(20), Ref::new(21), Ref::new(22)];
    let font = Name(b"F1");
    let image = Name(b"Im1");

    let mut pdf = Pdf::new();
    pdf.catalog(catalog_id).pages(tree_id);
    pdf.pages(tree_id).kids(page_ids).count(page_ids.len() as i32);
    pdf.type1_font(font_id).base_font(Name(b"Helvetica"));

    let pixels = gradient();
    let mut image_writer = pdf.image_xobject(image_id, &pixels);
    image_writer.filter(Filter::FlateDecode);
    image_writer.width(IMAGE_SIZE as i32);
    image_writer.height(IMAGE_SIZE as i32);
    image_writer.color_space().device_rgb();
    image_writer.bits_per_component(8);
    image_writer.finish();

    for (n, (&page_id, &content_id)) in page_ids.iter().zip(&content_ids).enumerate() {
        let mut page = pdf.page(page_id);
        page.media_box(A4).parent(tree_id).contents(content_id);
        let mut resources = page.resources();
        resources.fonts().pair(font, font_id);
        if n == 0 {
            resources.x_objects().pair(image, image_id);
        }
        resources.finish();
        page.finish();

        let mut content = Content::new();
        if n == 0 {
            content.set_fill_rgb(0.15, 0.35, 0.75).rect(56.0, 700.0, 483.0, 60.0).fill_nonzero();
            content.save_state().transform([200.0, 0.0, 0.0, 200.0, 56.0, 380.0]).x_object(image).restore_state();
        }
        content.set_fill_rgb(0.0, 0.0, 0.0).begin_text().set_font(font, 24.0).next_line(56.0, 660.0);
        content.show(Str(format!("Recto sample, page {}", n + 1).as_bytes()));
        content.set_font(font, 12.0).next_line(0.0, -28.0);
        content.show(Str(b"Generated by cargo xtask gen-fixtures. Not a real document."));
        content.end_text();
        pdf.stream(content_id, &content.finish());
    }

    pdf.finish()
}

/// Zlib-compressed RGB gradient, `IMAGE_SIZE` square.
fn gradient() -> Vec<u8> {
    let mut raw = Vec::with_capacity((IMAGE_SIZE * IMAGE_SIZE * 3) as usize);
    for y in 0..IMAGE_SIZE {
        for x in 0..IMAGE_SIZE {
            raw.extend_from_slice(&[(x * 4) as u8, (y * 4) as u8, 160]);
        }
    }
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(&raw).expect("writing to a Vec");
    encoder.finish().expect("writing to a Vec")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_fixtures_are_up_to_date() {
        for (name, build) in FIXTURES {
            let path = crate::workspace_root().join("tests/fixtures").join(name);
            let committed = std::fs::read(&path).unwrap_or_else(|_| panic!("{name} missing; run `cargo xtask gen-fixtures`"));
            assert!(committed == build(), "{name} is stale; run `cargo xtask gen-fixtures`");
        }
    }

    #[test]
    fn protected_fixture_needs_its_password() {
        let bytes = std::fs::read(crate::workspace_root().join("tests/fixtures").join(PROTECTED))
            .expect("protected.pdf exists; run `cargo xtask gen-fixtures`");
        let locked = lopdf::Document::load_mem(&bytes).unwrap();
        assert!(locked.is_encrypted() && locked.authenticate_password("").is_err());
        assert!(locked.authenticate_password(PROTECTED_PASSWORD).is_ok());
        let options = lopdf::LoadOptions::with_password(PROTECTED_PASSWORD);
        let open = lopdf::Document::load_mem_with_options(&bytes, options).unwrap();
        assert_eq!(open.get_pages().len(), 1);
    }
}
