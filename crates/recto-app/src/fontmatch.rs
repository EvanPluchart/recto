//! Finds an installed font close to a PDF font, for edited text the PDF's own font cannot show.
//! The system font list is read once, on first use.

use std::sync::OnceLock;

use fontdb::{Database, Family, Query, Style, Weight};
use recto_core::embed::FontFile;
use recto_core::textedit::FontStyle;

fn database() -> &'static Database {
    static DATABASE: OnceLock<Database> = OnceLock::new();
    DATABASE.get_or_init(|| {
        let mut database = Database::new();
        database.load_system_fonts();
        database
    })
}

/// `TimesNewRomanPS-BoldMT` → `Times New Roman`; also the name as written, without style.
fn family_names(name: &str) -> Vec<String> {
    let base = name.split(['-', ',']).next().unwrap_or(name);
    let base = base.trim_end_matches("MT").trim_end_matches("PS").trim_end_matches("PSMT");
    let mut spaced = String::new();
    let mut previous_lower = false;
    for c in base.chars() {
        if c.is_ascii_uppercase() && previous_lower {
            spaced.push(' ');
        }
        previous_lower = c.is_ascii_lowercase();
        spaced.push(c);
    }
    let mut names = vec![spaced.clone()];
    if spaced != base {
        names.push(base.to_owned());
    }
    names.retain(|name| !name.is_empty());
    names
}

/// A font for `text` looking like `style`: the same family if installed, else a common font of
/// the same kind (serif, sans serif, monospace), else any installed font with the characters.
pub fn find(style: &FontStyle, text: &str) -> Option<FontFile> {
    let database = database();
    let weight = if style.bold { Weight::BOLD } else { Weight::NORMAL };
    let slant = if style.italic { Style::Italic } else { Style::Normal };
    let names = family_names(&style.name);
    let common: &[&str] = if style.monospace {
        &["Courier New", "Liberation Mono", "DejaVu Sans Mono", "Menlo", "Consolas"]
    } else if style.serif {
        &["Times New Roman", "Liberation Serif", "DejaVu Serif", "Times", "Georgia"]
    } else {
        &["Arial", "Helvetica", "Liberation Sans", "DejaVu Sans", "Noto Sans", "Segoe UI"]
    };
    let generic = if style.monospace { Family::Monospace } else if style.serif { Family::Serif } else { Family::SansSerif };
    let mut families: Vec<Family> = names.iter().map(|name| Family::Name(name)).collect();
    families.extend(common.iter().map(|name| Family::Name(name)));
    families.push(generic);

    let usable = |id: fontdb::ID| -> Option<FontFile> {
        let font = database.with_face_data(id, |data, index| FontFile::new(data.to_vec(), index).ok())??;
        font.covers(text).then_some(font)
    };
    for family in &families {
        let query = Query { families: std::slice::from_ref(family), weight, style: slant, ..Query::default() };
        if let Some(font) = database.query(&query).and_then(usable) {
            return Some(font);
        }
    }
    // Any font with the characters: scripts the common fonts lack.
    database.faces().filter(|face| face.style == slant || slant == Style::Normal).find_map(|face| usable(face.id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_family_names() {
        assert_eq!(family_names("TimesNewRomanPS-BoldMT"), ["Times New Roman", "TimesNewRoman"]);
        assert_eq!(family_names("Arial-BoldMT"), ["Arial"]);
        assert_eq!(family_names("Helvetica"), ["Helvetica"]);
        assert_eq!(family_names("Georgia,Bold"), ["Georgia"]);
    }
}
