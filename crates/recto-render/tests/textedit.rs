//! Edited text, as PDFium reads and draws it: the new text is there, starts where the old one
//! did, and the other lines did not move. Needs `cargo xtask fetch-pdfium`. One test per file:
//! PDFium has process-wide state.

use std::path::Path;
use std::sync::Arc;

use recto_core::embed::FontFile;
use recto_core::geom::PageRect;
use recto_core::op::Op;
use recto_core::text::PageText;
use recto_core::textedit::TextEdit;
use recto_core::{Document, PageId};
use recto_render::{PdfiumRenderer, RenderThread, Response};

const PAGE: PageId = PageId { source: 0, index: 0 };
const HEIGHT: f32 = 842.0;

fn text(thread: &RenderThread, bytes: Vec<u8>) -> PageText {
    thread.open(Arc::from(bytes), None);
    assert!(matches!(thread.responses().recv().unwrap(), Response::Opened(Ok(_))));
    thread.text(0);
    match thread.responses().recv().unwrap() {
        Response::Text { result, .. } => result.unwrap(),
        other => panic!("unexpected {other:?}"),
    }
}

fn boxes(page: &PageText, word: &str) -> Vec<PageRect> {
    let chars: String = page.chars.iter().collect();
    let start = chars.find(word).unwrap_or_else(|| panic!("{word} not in {chars:?}"));
    let start = chars[..start].chars().count();
    page.boxes[start..start + word.chars().count()].to_vec()
}

fn edit(document: &mut Document, original: &str, text: &str, font: Option<usize>) {
    let run = document.text_runs(PAGE).unwrap().into_iter().find(|run| run.original == original).expect("the run exists");
    let edit = TextEdit { first: run.first, last: run.last, original: run.original, text: text.into(), font };
    document.apply(Op::EditText { page: PAGE, edit });
}

#[test]
fn edited_text_reads_and_sits_where_the_old_text_was() {
    let thread = RenderThread::spawn(PdfiumRenderer::new).expect("PDFium loads; run `cargo xtask fetch-pdfium`");
    let mut document = Document::open(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/edit.pdf")).unwrap();
    let before = text(&thread, document.build().unwrap());

    // recto-core's runs hold PDFium's boxes of their text.
    for run in document.text_runs(PAGE).unwrap() {
        let word = run.text.split(' ').next().unwrap().to_owned();
        for found in boxes(&before, &word) {
            let (top, bottom) = (HEIGHT - found.y, HEIGHT - found.bottom());
            let b = run.bounds;
            assert!(
                found.x >= b.origin.x - 1.0 && found.right() <= b.origin.x + b.width + 1.0 && bottom >= b.origin.y - 1.0 && top <= b.origin.y + b.height + 1.0,
                "{word}: PDFium box {found:?} outside the run {b:?}"
            );
        }
    }

    let font = document.add_font(FontFile::new(recto_core::testfont::font("RectoFallback"), 0).unwrap());
    edit(&mut document, "Hello world from Recto", "Bonjour à tous", None);
    edit(&mut document, "Embedded font line", "Ligne intégrée", None);
    edit(&mut document, "Type zero text", "Zebra €", Some(font));
    edit(&mut document, "Second line", "Deuxième ligne", None);
    let after = text(&thread, document.build().unwrap());
    let shown: String = after.chars.iter().collect();
    for new in ["Bonjour à tous", "Ligne intégrée", "Zebra €", "Deuxième ligne", "Code 1234 end", "First", "Split run"] {
        assert!(shown.contains(new), "{new} missing from {shown:?}");
    }
    // The new text starts where the old one did, on the same line (boxes of different glyphs
    // and fonts differ in height: compare where the new one ends at the bottom).
    for (old, new) in [("Hello", "Bonjour"), ("Embedded", "Ligne"), ("Type", "Zebra"), ("Second", "Deuxième")] {
        let (a, b) = (boxes(&before, old)[0], boxes(&after, new)[0]);
        assert!((a.x - b.x).abs() < 1.5 && b.bottom() > a.y && b.bottom() < a.bottom() + 1.0, "{old} at {a:?}, {new} at {b:?}");
    }
    // Lines that were not edited did not move.
    for word in ["Code", "First", "Split"] {
        assert_eq!(boxes(&before, word), boxes(&after, word), "{word} moved");
    }
}
