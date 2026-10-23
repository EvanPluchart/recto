//! `target/demo.pdf` (French) and `target/demo-en.pdf` (English): a made-up three-page quote, for
//! screenshots of the app (landing page, README). Not committed and not a test fixture: every name
//! and address in it is invented. Both languages share one layout, so annotations drawn on the
//! screenshots land in the same places.

use std::path::Path;

use anyhow::Result;
use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref, Str, TextStr};

const A4: Rect = Rect { x1: 0.0, y1: 0.0, x2: 595.0, y2: 842.0 };
const LEFT: f32 = 56.0;
const RIGHT: f32 = 539.0;
const REGULAR: Name = Name(b"F1");
const BOLD: Name = Name(b"F2");
/// The workshop's colour, used for its name and the rules of the quote.
const GREEN: (f32, f32, f32) = (0.16, 0.32, 0.27);
const GREY: (f32, f32, f32) = (0.38, 0.38, 0.40);
const BLACK: (f32, f32, f32) = (0.0, 0.0, 0.0);

/// Every word of the quote, in one language.
struct Words {
    title: &'static str,
    trade: &'static str,
    address: &'static str,
    quote: &'static str,
    number: &'static str,
    date: &'static str,
    client: &'static str,
    client_address: &'static str,
    subject: &'static str,
    subject_lines: [&'static str; 2],
    columns: [&'static str; 4],
    lines: [(&'static str, &'static str, &'static str); 3],
    totals: [(&'static str, &'static str); 3],
    conditions: [&'static str; 4],
    agreement: &'static str,
    agreement_hint: &'static str,
    workshop: &'static str,
    workshop_name: &'static str,
    terms_title: &'static str,
    terms: [(&'static str, &'static str); 7],
    sketch_title: &'static str,
    sketch_hint: &'static str,
    sketch_note: &'static str,
    footer: &'static str,
    outline: [&'static str; 3],
}

const FRENCH: Words = Words {
    title: "Devis 2026-118 — Atelier Brume",
    trade: "Menuiserie sur mesure",
    address: "12 rue des Tanneurs, 51100 Reims · atelier-brume.example",
    quote: "DEVIS",
    number: "N° 2026-118",
    date: "Reims, le 14 octobre 2026",
    client: "Client",
    client_address: "8 allée des Tilleuls, 51100 Reims",
    subject: "Objet",
    subject_lines: ["Bibliothèque en chêne massif", "pour le salon, sur mesure"],
    columns: ["Désignation", "Qté", "Prix unitaire HT", "Total HT"],
    lines: [
        ("Bibliothèque chêne massif, 240 × 220 cm", "Six niches, fond en contreplaqué de chêne", "2 480,00 €"),
        ("Pose et fixation murale", "Deux personnes, une demi-journée", "320,00 €"),
        ("Finition à l'huile naturelle", "Deux couches, teinte claire", "180,00 €"),
    ],
    totals: [("Total HT", "2 980,00 €"), ("TVA 20 %", "596,00 €"), ("Total TTC", "3 576,00 €")],
    conditions: ["Validité de l'offre : 30 jours.", "Acompte de 30 % à la commande,", "solde à la pose.", "Délai de fabrication : six semaines."],
    agreement: "Bon pour accord",
    agreement_hint: "Date, mention « Bon pour accord » et signature du client :",
    workshop: "L'atelier",
    workshop_name: "Julien Brume, menuisier",
    terms_title: "Conditions générales",
    terms: [
        ("Commande", "La commande est ferme à réception du devis signé et de l'acompte. Toute modification des dimensions après la commande fait l'objet d'un avenant, chiffré avant d'être réalisé."),
        ("Bois", "Le chêne est un matériau vivant : sa teinte, ses nœuds et son veinage varient d'une planche à l'autre et ne constituent pas un défaut. Les planches sont séchées à moins de 10 % d'humidité avant usinage."),
        ("Délais", "Le délai de fabrication court à partir de l'encaissement de l'acompte. La date de pose est fixée d'un commun accord une semaine avant la fin de la fabrication."),
        ("Pose", "Le client s'assure que le mur peut recevoir la charge prévue et que l'accès au salon permet le passage des éléments, livrés en quatre parties de moins de 80 cm de large."),
        ("Paiement", "Le solde est réglé le jour de la pose, par virement ou par chèque. Aucun escompte n'est accordé pour paiement anticipé. Tout retard entraîne des pénalités au taux légal."),
        ("Garantie", "Les assemblages sont garantis cinq ans. La garantie ne couvre pas les dégâts dus à l'humidité excessive de la pièce, à un choc ou à un entretien avec des produits non adaptés."),
        ("Entretien", "Une couche d'huile tous les deux ans suffit à garder la finition. L'atelier fournit avec la bibliothèque un flacon de l'huile utilisée et une notice."),
    ],
    sketch_title: "Croquis",
    sketch_hint: "Vue de face, échelle 1/15. Cotes en centimètres.",
    sketch_note: "Niches de 116 × 112 cm, montant central de 8 cm, socle de 10 cm.",
    footer: "Atelier Brume · SIRET 000 000 000 00000 · Document de démonstration, entreprise fictive.",
    outline: ["Devis", "Conditions générales", "Croquis"],
};

const ENGLISH: Words = Words {
    title: "Quote 2026-118 — Atelier Brume",
    trade: "Bespoke joinery",
    address: "12 rue des Tanneurs, 51100 Reims · atelier-brume.example",
    quote: "QUOTE",
    number: "No. 2026-118",
    date: "Reims, 14 October 2026",
    client: "Client",
    client_address: "8 allée des Tilleuls, 51100 Reims",
    subject: "Subject",
    subject_lines: ["Solid oak bookcase", "for the living room, made to measure"],
    columns: ["Description", "Qty", "Unit price", "Total"],
    lines: [
        ("Solid oak bookcase, 240 × 220 cm", "Six shelves, oak plywood back", "€2,480.00"),
        ("Wall fitting", "Two people, half a day", "€320.00"),
        ("Natural oil finish", "Two coats, light shade", "€180.00"),
    ],
    totals: [("Total excl. VAT", "€2,980.00"), ("VAT 20%", "€596.00"), ("Total incl. VAT", "€3,576.00")],
    conditions: ["Offer valid for 30 days.", "30% deposit on order,", "balance on fitting.", "Lead time: six weeks."],
    agreement: "Agreed",
    agreement_hint: "Date, the words \"Agreed\" and the client's signature:",
    workshop: "The workshop",
    workshop_name: "Julien Brume, joiner",
    terms_title: "Terms and conditions",
    terms: [
        ("Order", "The order is firm once the signed quote and the deposit are received. Any change to the dimensions after the order is the subject of an amendment, priced before it is carried out."),
        ("Wood", "Oak is a living material: its shade, knots and grain vary from one board to the next and are not a defect. Boards are dried below 10% moisture before machining."),
        ("Lead time", "The lead time runs from receipt of the deposit. The fitting date is agreed together one week before the end of manufacturing."),
        ("Fitting", "The client makes sure the wall can bear the planned load and that the way into the living room lets the parts through; they come in four pieces under 80 cm wide."),
        ("Payment", "The balance is paid on the day of fitting, by bank transfer or cheque. No discount is given for early payment. Late payment incurs interest at the legal rate."),
        ("Warranty", "Joints are guaranteed for five years. The warranty does not cover damage from excessive damp in the room, from knocks, or from care products that are not suitable."),
        ("Care", "One coat of oil every two years keeps the finish. The workshop supplies a bottle of the oil used and instructions with the bookcase."),
    ],
    sketch_title: "Sketch",
    sketch_hint: "Front view, scale 1:15. Dimensions in centimetres.",
    sketch_note: "Shelves 116 × 112 cm, 8 cm centre post, 10 cm plinth.",
    footer: "Atelier Brume · Demonstration document, made-up company.",
    outline: ["Quote", "Terms and conditions", "Sketch"],
};

pub fn generate(root: &Path) -> Result<()> {
    for (name, words) in [("demo.pdf", &FRENCH), ("demo-en.pdf", &ENGLISH)] {
        let path = root.join("target").join(name);
        std::fs::write(&path, demo_pdf(words))?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

fn demo_pdf(w: &Words) -> Vec<u8> {
    let catalog_id = Ref::new(1);
    let tree_id = Ref::new(2);
    let regular_id = Ref::new(3);
    let bold_id = Ref::new(4);
    let info_id = Ref::new(5);
    let outline_id = Ref::new(6);
    let page_ids = [Ref::new(10), Ref::new(12), Ref::new(14)];

    let mut pdf = Pdf::new();
    pdf.catalog(catalog_id).pages(tree_id).outlines(outline_id);
    pdf.pages(tree_id).kids(page_ids).count(page_ids.len() as i32);
    pdf.type1_font(regular_id).base_font(Name(b"Helvetica")).encoding_predefined(Name(b"WinAnsiEncoding"));
    pdf.type1_font(bold_id).base_font(Name(b"Helvetica-Bold")).encoding_predefined(Name(b"WinAnsiEncoding"));
    pdf.document_info(info_id).title(TextStr(w.title)).author(TextStr("Atelier Brume"));

    let contents = [quote_page(w), terms_page(w), sketch_page(w)];
    for (&page_id, content) in page_ids.iter().zip(contents) {
        let content_id = Ref::new(page_id.get() + 1);
        let mut page = pdf.page(page_id);
        page.media_box(A4).parent(tree_id).contents(content_id);
        page.resources().fonts().pair(REGULAR, regular_id).pair(BOLD, bold_id);
        page.finish();
        pdf.stream(content_id, &content.finish());
    }

    let item = |i: usize| Ref::new(20 + i as i32);
    pdf.outline(outline_id).first(item(0)).last(item(w.outline.len() - 1)).count(w.outline.len() as i32);
    for (i, (title, &page)) in w.outline.iter().zip(&page_ids).enumerate() {
        let mut entry = pdf.outline_item(item(i));
        entry.title(TextStr(title)).parent(outline_id);
        if i > 0 {
            entry.prev(item(i - 1));
        }
        if i + 1 < w.outline.len() {
            entry.next(item(i + 1));
        }
        entry.dest().page(page).xyz(0.0, 842.0, None);
    }
    pdf.finish()
}

fn quote_page(w: &Words) -> Content {
    let mut c = Content::new();
    // Letterhead.
    text(&mut c, BOLD, 20.0, LEFT, 772.0, GREEN, "Atelier Brume");
    text(&mut c, REGULAR, 10.0, LEFT, 756.0, GREY, w.trade);
    text(&mut c, REGULAR, 9.0, LEFT, 740.0, GREY, w.address);
    text_right(&mut c, BOLD, 24.0, RIGHT, 770.0, BLACK, w.quote);
    text_right(&mut c, REGULAR, 10.0, RIGHT, 754.0, GREY, w.number);
    text_right(&mut c, REGULAR, 10.0, RIGHT, 740.0, GREY, w.date);
    fill(&mut c, GREEN, LEFT, 720.0, RIGHT - LEFT, 2.0);

    // Client and subject.
    text(&mut c, BOLD, 10.0, LEFT, 694.0, BLACK, w.client);
    text(&mut c, REGULAR, 10.0, LEFT, 680.0, BLACK, "Camille Martin");
    text(&mut c, REGULAR, 10.0, LEFT, 667.0, BLACK, w.client_address);
    text(&mut c, BOLD, 10.0, 330.0, 694.0, BLACK, w.subject);
    text(&mut c, REGULAR, 10.0, 330.0, 680.0, BLACK, w.subject_lines[0]);
    text(&mut c, REGULAR, 10.0, 330.0, 667.0, BLACK, w.subject_lines[1]);

    // Lines of the quote.
    let columns = [LEFT + 8.0, 352.0, 446.0, RIGHT - 8.0];
    fill(&mut c, (0.93, 0.94, 0.92), LEFT, 610.0, RIGHT - LEFT, 24.0);
    text(&mut c, BOLD, 9.5, columns[0], 618.0, BLACK, w.columns[0]);
    for (i, &x) in columns.iter().enumerate().skip(1) {
        text_right(&mut c, BOLD, 9.5, x, 618.0, BLACK, w.columns[i]);
    }
    let mut y = 588.0;
    for (title, detail, price) in w.lines {
        text(&mut c, REGULAR, 10.0, columns[0], y, BLACK, title);
        text(&mut c, REGULAR, 8.5, columns[0], y - 13.0, GREY, detail);
        text_right(&mut c, REGULAR, 10.0, columns[1], y, BLACK, "1");
        text_right(&mut c, REGULAR, 10.0, columns[2], y, BLACK, price);
        text_right(&mut c, REGULAR, 10.0, columns[3], y, BLACK, price);
        fill(&mut c, (0.85, 0.86, 0.84), LEFT, y - 24.0, RIGHT - LEFT, 0.6);
        y -= 42.0;
    }

    // Totals.
    let mut y = 444.0;
    for (i, (label, amount)) in w.totals.iter().enumerate() {
        if i == w.totals.len() - 1 {
            fill(&mut c, GREEN, 330.0, y - 8.0, RIGHT - 330.0, 24.0);
            text(&mut c, BOLD, 11.0, 340.0, y, (1.0, 1.0, 1.0), label);
            text_right(&mut c, BOLD, 11.0, columns[3], y, (1.0, 1.0, 1.0), amount);
        } else {
            text(&mut c, REGULAR, 10.0, 340.0, y, BLACK, label);
            text_right(&mut c, REGULAR, 10.0, columns[3], y, BLACK, amount);
        }
        y -= 24.0;
    }

    for (i, line) in w.conditions.iter().enumerate() {
        text(&mut c, REGULAR, 9.0, LEFT, 444.0 - 13.0 * i as f32, GREY, line);
    }

    // Agreement.
    text(&mut c, BOLD, 10.0, LEFT, 330.0, BLACK, w.agreement);
    text(&mut c, REGULAR, 9.0, LEFT, 316.0, GREY, w.agreement_hint);
    c.set_stroke_rgb(0.7, 0.71, 0.69).set_line_width(0.8).rect(LEFT, 180.0, 250.0, 120.0).stroke();
    text(&mut c, BOLD, 10.0, 330.0, 330.0, BLACK, w.workshop);
    text(&mut c, REGULAR, 9.0, 330.0, 316.0, GREY, w.workshop_name);

    footer(&mut c, w, 1);
    c
}

fn terms_page(w: &Words) -> Content {
    let mut c = Content::new();
    text(&mut c, BOLD, 16.0, LEFT, 772.0, GREEN, w.terms_title);
    fill(&mut c, GREEN, LEFT, 756.0, RIGHT - LEFT, 2.0);
    let mut y = 728.0;
    for (title, body) in w.terms {
        text(&mut c, BOLD, 10.5, LEFT, y, BLACK, title);
        y -= 16.0;
        for line in wrap(body, 10.0, RIGHT - LEFT) {
            text(&mut c, REGULAR, 10.0, LEFT, y, (0.12, 0.12, 0.14), &line);
            y -= 14.5;
        }
        y -= 14.0;
    }
    footer(&mut c, w, 2);
    c
}

fn sketch_page(w: &Words) -> Content {
    let mut c = Content::new();
    text(&mut c, BOLD, 16.0, LEFT, 772.0, GREEN, w.sketch_title);
    fill(&mut c, GREEN, LEFT, 756.0, RIGHT - LEFT, 2.0);
    text(&mut c, REGULAR, 10.0, LEFT, 734.0, GREY, w.sketch_hint);

    // 240 × 220 cm at 1/15 (1 cm = 1.6 pt, rounded for a readable drawing).
    let (x, y, width, height) = (118.0, 300.0, 384.0, 352.0);
    c.set_stroke_rgb(0.1, 0.1, 0.12).set_line_width(2.0).rect(x, y, width, height).stroke();
    c.set_line_width(1.0);
    for shelf in 1..3 {
        let sy = y + height * shelf as f32 / 3.0;
        c.move_to(x, sy).line_to(x + width, sy);
    }
    c.move_to(x + width / 2.0, y).line_to(x + width / 2.0, y + height).stroke();
    // Plinth.
    c.set_fill_rgb(0.86, 0.80, 0.70).rect(x, y - 16.0, width, 16.0).fill_nonzero();
    // Dimension lines.
    c.set_stroke_rgb(GREY.0, GREY.1, GREY.2).set_line_width(0.6);
    c.move_to(x, y - 40.0).line_to(x + width, y - 40.0).stroke();
    c.move_to(x - 30.0, y).line_to(x - 30.0, y + height).stroke();
    text(&mut c, REGULAR, 10.0, x + width / 2.0 - 18.0, y - 56.0, GREY, "240 cm");
    text(&mut c, REGULAR, 10.0, x - 52.0, y + height + 10.0, GREY, "220 cm");
    text(&mut c, REGULAR, 9.0, x, y - 90.0, GREY, w.sketch_note);
    footer(&mut c, w, 3);
    c
}

fn footer(c: &mut Content, w: &Words, page: usize) {
    text(c, REGULAR, 8.0, LEFT, 40.0, GREY, w.footer);
    text_right(c, REGULAR, 8.0, RIGHT, 40.0, GREY, &format!("{page} / 3"));
}

fn fill(c: &mut Content, (r, g, b): (f32, f32, f32), x: f32, y: f32, w: f32, h: f32) {
    c.set_fill_rgb(r, g, b).rect(x, y, w, h).fill_nonzero();
}

fn text(c: &mut Content, font: Name, size: f32, x: f32, y: f32, (r, g, b): (f32, f32, f32), s: &str) {
    c.set_fill_rgb(r, g, b).begin_text().set_font(font, size).next_line(x, y);
    c.show(Str(&win_ansi(s))).end_text();
}

fn text_right(c: &mut Content, font: Name, size: f32, right: f32, y: f32, color: (f32, f32, f32), s: &str) {
    let width = s.chars().map(|ch| advance(ch, font == BOLD)).sum::<f32>() * size / 1000.0;
    text(c, font, size, right - width, y, color, s);
}

/// Greedy line breaking with Helvetica's advance widths.
fn wrap(s: &str, size: f32, width: f32) -> Vec<String> {
    let mut lines = vec![String::new()];
    for word in s.split(' ') {
        let line = lines.last_mut().expect("never empty");
        let candidate = if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
        if candidate.chars().map(|ch| advance(ch, false)).sum::<f32>() * size / 1000.0 > width && !line.is_empty() {
            lines.push(word.to_owned());
        } else {
            *line = candidate;
        }
    }
    lines
}

/// Helvetica advance widths (per 1000 em) for the characters used here; close enough for the
/// others.
fn advance(ch: char, bold: bool) -> f32 {
    match ch {
        '0'..='9' | '€' | '$' => 556.0,
        ' ' | ',' | '.' | ':' | ';' | '\u{a0}' | '\u{202f}' | '\'' | 'i' | 'j' | 'l' | 'f' | 't' | 'I' => {
            if bold && matches!(ch, 'i' | 'j' | 'l' | ':' | ';' | 'f' | 't' | 'I') { 333.0 } else { 278.0 }
        }
        '%' => 889.0,
        'm' | 'M' | 'W' | 'w' => 833.0,
        'r' => if bold { 389.0 } else { 333.0 },
        'A'..='Z' => if bold { 722.0 } else { 667.0 },
        _ => if bold { 611.0 } else { 556.0 },
    }
}

/// `s` in WinAnsiEncoding (Windows-1252); characters outside it become `?`.
fn win_ansi(s: &str) -> Vec<u8> {
    s.chars()
        .map(|ch| match ch {
            '€' => 0x80,
            '…' => 0x85,
            '’' => 0x92,
            '—' => 0x97,
            'œ' => 0x9C,
            '\u{202f}' => 0xA0,
            '×' => 0xD7,
            ch if (ch as u32) < 0x80 || (0xA0..=0xFF).contains(&(ch as u32)) => ch as u8,
            _ => b'?',
        })
        .collect()
}
