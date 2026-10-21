//! Edited documents, rebuilt by `recto-core`, as PDFium shows them. Needs `cargo xtask fetch-pdfium`.
//! One test per file: PDFium has process-wide state.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use recto_core::geom::Size;
use recto_core::op::{Op, Rotation};
use recto_core::{Document, PageId};
use recto_render::{DocInfo, PdfiumRenderer, RenderThread, Response};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures").join(name)
}

fn open(thread: &RenderThread, bytes: Vec<u8>, password: Option<&str>) -> DocInfo {
    thread.open(Arc::from(bytes), password);
    match thread.responses().recv().unwrap() {
        Response::Opened(info) => info.unwrap(),
        other => panic!("unexpected {other:?}"),
    }
}

fn page_text(thread: &RenderThread, index: usize) -> String {
    thread.text(index);
    match thread.responses().recv().unwrap() {
        Response::Text { result, .. } => result.unwrap().chars.iter().collect(),
        other => panic!("unexpected {other:?}"),
    }
}

fn size(width: f32, height: f32) -> Size {
    Size { width, height }
}

#[test]
fn rebuilt_documents_open_in_pdfium() {
    let thread = RenderThread::spawn(PdfiumRenderer::new).expect("PDFium loads; run `cargo xtask fetch-pdfium`");
    let page = |index| PageId { source: 0, index };

    // Rotate, delete, merge a landscape page and append a blank one.
    let mut document = Document::open(fixture("sample.pdf")).unwrap();
    let merged = document.add_path(fixture("layout.pdf")).unwrap();
    let blank = document.add_blank(size(300.0, 400.0));
    document.apply(Op::RotatePages { pages: vec![page(0)], rotation: Rotation::R90 });
    document.apply(Op::DeletePages { pages: vec![page(2)] });
    document.apply(Op::InsertPages { pages: vec![merged[1]], at: 1 });
    document.apply(Op::InsertPages { pages: vec![blank], at: 99 });
    let info = open(&thread, document.build().unwrap(), None);
    assert_eq!(info.page_sizes, vec![size(842.0, 595.0), size(842.0, 595.0), size(595.0, 842.0), size(300.0, 400.0)]);
    assert!(page_text(&thread, 0).contains("Recto sample, page 1"));
    assert!(page_text(&thread, 1).contains("Page 2 (842"));
    assert!(page_text(&thread, 2).contains("Recto sample, page 2"));
    // The outline of the opened file is kept.
    let mut with_outline = Document::open(fixture("layout.pdf")).unwrap();
    with_outline.apply(Op::MovePages { pages: vec![page(3)], to: 0 });
    let info = open(&thread, with_outline.build().unwrap(), None);
    assert_eq!(info.outline.len(), 4);
    assert_eq!(info.outline[3].page, Some(0), "the Letter page moved first");

    // Inherited attributes: the nested node's Letter box and /Rotate 90 follow the page.
    let mut inherited = Document::open(fixture("inherited.pdf")).unwrap();
    let before = open(&thread, inherited.build().unwrap(), None);
    assert_eq!(before.page_sizes, vec![size(792.0, 612.0), size(792.0, 612.0), size(842.0, 595.0)]);
    inherited.apply(Op::MovePages { pages: vec![page(2)], to: 0 });
    let after = open(&thread, inherited.build().unwrap(), None);
    assert_eq!(after.page_sizes, vec![size(842.0, 595.0), size(792.0, 612.0), size(792.0, 612.0)]);
    assert!(page_text(&thread, 0).contains("Inherited 3"));

    // An encrypted original stays encrypted with the same password.
    let mut protected = Document::open(fixture("protected.pdf")).unwrap();
    assert!(protected.unlock("recto"));
    protected.apply(Op::RotatePages { pages: vec![page(0)], rotation: Rotation::R270 });
    let bytes = protected.build().unwrap();
    thread.open(Arc::from(bytes.clone()), None);
    match thread.responses().recv().unwrap() {
        Response::Opened(result) => assert!(result.is_err(), "opened without a password"),
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(open(&thread, bytes, Some("recto")).page_sizes, vec![size(842.0, 595.0)]);
}
