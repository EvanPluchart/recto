//! A scanned page made from a fixture, read back by OCR, given a text layer, and checked with
//! PDFium. Needs `cargo xtask fetch-pdfium` and `cargo xtask fetch-ocr-models`.

use std::path::Path;
use std::sync::Arc;

use recto_core::geom::PageRect;
use recto_core::op::Op;
use recto_core::text::PageText;
use recto_core::textlayer::{TextLayer, Word};
use recto_core::{Document, PageId};
use recto_render::{Bitmap, PdfiumRenderer, Purpose, RenderRequest, RenderThread, Response};

fn open(thread: &RenderThread, bytes: Vec<u8>) {
    thread.open(Arc::from(bytes), None);
    assert!(matches!(thread.responses().recv().unwrap(), Response::Opened(Ok(_))));
}

fn render(thread: &RenderThread, scale: f32) -> Bitmap {
    let generation = thread.next_generation();
    let page = PageId { source: 0, index: 0 };
    thread.request(RenderRequest { page, index: 0, scale, generation, purpose: Purpose::Page, tile: None });
    match thread.responses().recv().unwrap() {
        Response::Rendered { result, .. } => result.unwrap(),
        other => panic!("unexpected {other:?}"),
    }
}

fn text(thread: &RenderThread) -> PageText {
    thread.text(0);
    match thread.responses().recv().unwrap() {
        Response::Text { result, .. } => result.unwrap(),
        other => panic!("unexpected {other:?}"),
    }
}

fn rgb(bitmap: &Bitmap) -> Vec<u8> {
    bitmap.pixels.as_chunks::<4>().0.iter().flat_map(|pixel| [pixel[2], pixel[1], pixel[0]]).collect()
}

fn bounds(page: &PageText, word: &str) -> Option<PageRect> {
    let chars: String = page.chars.iter().collect();
    let start = chars.find(word)?;
    let start = chars[..start].chars().count();
    Some(page.boxes[start..start + word.chars().count()].iter().fold(PageRect::default(), |all, item| all.union(item)))
}

/// One A4 page holding only an image: what a scanner produces.
fn scanned(width: u32, height: u32, rgb: Vec<u8>) -> Vec<u8> {
    use lopdf::{Dictionary, Object, Stream};
    let name = |text: &str| Object::Name(text.as_bytes().to_vec());
    let mut document = lopdf::Document::with_version("1.7");
    let pages = document.new_object_id();
    let mut image = Dictionary::new();
    image.set("Type", name("XObject"));
    image.set("Subtype", name("Image"));
    image.set("Width", width as i64);
    image.set("Height", height as i64);
    image.set("ColorSpace", name("DeviceRGB"));
    image.set("BitsPerComponent", 8);
    let mut image = Stream::new(image, rgb);
    image.compress().unwrap();
    let image = document.add_object(image);
    let mut xobjects = Dictionary::new();
    xobjects.set("Scan", Object::Reference(image));
    let mut resources = Dictionary::new();
    resources.set("XObject", Object::Dictionary(xobjects));
    let content = document.add_object(Stream::new(Dictionary::new(), b"q 595 0 0 842 0 0 cm /Scan Do Q".to_vec()));
    let mut page = Dictionary::new();
    page.set("Type", name("Page"));
    page.set("Parent", Object::Reference(pages));
    page.set("MediaBox", Object::Array(vec![0.into(), 0.into(), 595.into(), 842.into()]));
    page.set("Resources", Object::Dictionary(resources));
    page.set("Contents", Object::Reference(content));
    let page = document.add_object(page);
    let mut tree = Dictionary::new();
    tree.set("Type", name("Pages"));
    tree.set("Kids", Object::Array(vec![Object::Reference(page)]));
    tree.set("Count", 1);
    document.objects.insert(pages, Object::Dictionary(tree));
    let mut catalog = Dictionary::new();
    catalog.set("Type", name("Catalog"));
    catalog.set("Pages", Object::Reference(pages));
    let catalog = document.add_object(catalog);
    document.trailer.set("Root", Object::Reference(catalog));
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

#[test]
fn recognised_words_land_where_the_scan_shows_them() {
    let models = recto_ocr::find_models(&recto_ocr::model_dirs(None)).expect("OCR models; run `cargo xtask fetch-ocr-models`");
    let engine = recto_ocr::Engine::load(&models).unwrap();
    let thread = RenderThread::spawn(PdfiumRenderer::new).expect("PDFium loads; run `cargo xtask fetch-pdfium`");

    // The fixture's first page, as a 200 ppi scan.
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/sample.pdf");
    let original = Document::open(&fixture).unwrap();
    open(&thread, original.build_pages(&original.pages()[..1]).unwrap());
    let truth = text(&thread);
    let scan = render(&thread, 200.0 / 72.0);
    let mut document = Document::from_bytes(scanned(scan.width, scan.height, rgb(&scan))).unwrap();
    open(&thread, document.build().unwrap());
    assert!(text(&thread).chars.is_empty(), "a scan has no text");

    // OCR at 300 ppi, as the app does; pixels to user space through the page geometry.
    let scale = 300.0 / 72.0;
    let page = render(&thread, scale);
    let words = engine.words(page.width, page.height, &rgb(&page)).unwrap();
    let geometry = document.geometry().unwrap()[0];
    let words: Vec<Word> = words
        .into_iter()
        .map(|word| {
            let shown = PageRect { x: word.left / scale, y: word.top / scale, width: (word.right - word.left) / scale, height: (word.bottom - word.top) / scale };
            Word { text: word.text, rect: geometry.rect_to_user(shown) }
        })
        .collect();
    let found: Vec<&str> = words.iter().map(|word| word.text.as_str()).collect();
    assert!(found.contains(&"Recto") && found.contains(&"sample,"), "{found:?}");

    document.apply(Op::AddTextLayer { page: PageId { source: 0, index: 0 }, layer: TextLayer { words, upright: 0 } });
    open(&thread, document.build().unwrap());
    let layer = text(&thread);
    let chars: String = layer.chars.iter().collect();
    assert!(chars.contains("Recto sample"), "{chars:?}");
    // The invisible word sits on the scanned one, within a few points.
    let (expected, got) = (bounds(&truth, "sample").unwrap(), bounds(&layer, "sample").unwrap());
    for (a, b) in [(expected.x, got.x), (expected.right(), got.right()), (expected.y + expected.height / 2.0, got.y + got.height / 2.0)] {
        assert!((a - b).abs() < 4.0, "{expected:?} vs {got:?}");
    }
    // And the page looks the same: the text is invisible.
    let before = render(&thread, 1.0);
    open(&thread, Document::from_bytes(scanned(scan.width, scan.height, rgb(&scan))).unwrap().build().unwrap());
    let scan_only = render(&thread, 1.0);
    assert_eq!(before.pixels, scan_only.pixels);
}
