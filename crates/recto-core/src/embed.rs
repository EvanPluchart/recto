//! Embedding a font for edited text: a subset of a TrueType or OpenType font (added to the
//! document with `Document::add_font`), written as a Type0 font with Identity-H encoding, its
//! widths and a ToUnicode CMap, so the text can be searched and copied.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::sync::Arc;

use lopdf::{Dictionary, Object, Stream};

use crate::textedit::Embedded;

/// A font file given to the document, and which face of it to use.
#[derive(Clone, Debug)]
pub struct FontFile {
    pub bytes: Arc<[u8]>,
    pub index: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum FontError {
    #[error("not a font this program can read")]
    Unreadable,
    #[error("the font's licence does not allow embedding it")]
    Restricted,
    #[error("variable fonts are not supported")]
    Variable,
}

impl FontFile {
    /// Checks that the face can be read and embedded.
    pub fn new(bytes: Vec<u8>, index: u32) -> Result<FontFile, FontError> {
        let face = ttf_parser::Face::parse(&bytes, index).map_err(|_| FontError::Unreadable)?;
        if face.permissions() == Some(ttf_parser::Permissions::Restricted) {
            return Err(FontError::Restricted);
        }
        if face.is_variable() {
            return Err(FontError::Variable);
        }
        Ok(FontFile { bytes: bytes.into(), index })
    }

    /// Whether the face has a glyph for every character of `text` (spaces aside).
    pub fn covers(&self, text: &str) -> bool {
        let Ok(face) = ttf_parser::Face::parse(&self.bytes, self.index) else { return false };
        text.chars().all(|c| c.is_whitespace() || face.glyph_index(c).is_some_and(|gid| gid.0 != 0))
    }
}

/// Six capital letters naming a subset, from what it holds (the same subset gets the same name).
fn subset_tag(chars: &[char]) -> String {
    let mut hash: u32 = 2_166_136_261;
    for c in chars {
        hash = (hash ^ *c as u32).wrapping_mul(16_777_619);
    }
    (0..6).map(|n| (b'A' + ((hash >> (n * 5)) % 26) as u8) as char).collect()
}

fn pdf_name(text: &str) -> Vec<u8> {
    text.bytes().filter(|b| b.is_ascii_alphanumeric() || *b == b'-').collect()
}

/// Embeds the glyphs of `chars` from `font` into `document`.
pub(crate) fn embed(document: &mut lopdf::Document, font: &FontFile, chars: &[char]) -> Result<Embedded, String> {
    let face = ttf_parser::Face::parse(&font.bytes, font.index).map_err(|error| error.to_string())?;
    let mut remapper = subsetter::GlyphRemapper::new();
    let mut glyphs: BTreeMap<char, u16> = BTreeMap::new();
    for &c in chars {
        let gid = face.glyph_index(c).map(|gid| gid.0).or_else(|| c.is_whitespace().then(|| face.glyph_index(' ').map(|gid| gid.0)).flatten());
        let gid = gid.filter(|gid| *gid != 0).ok_or_else(|| format!("no glyph for {c:?}"))?;
        glyphs.insert(c, remapper.remap(gid));
    }
    let subset = subsetter::subset(&font.bytes, font.index, &remapper).map_err(|error| error.to_string())?;

    let units = f32::from(face.units_per_em());
    let scale = |value: f32| (value * 1000.0 / units).round();
    let family = face
        .names()
        .into_iter()
        .find(|name| name.name_id == ttf_parser::name_id::POST_SCRIPT_NAME)
        .and_then(|name| name.to_string())
        .unwrap_or_else(|| "Font".to_owned());
    let mut ordered: Vec<char> = glyphs.keys().copied().collect();
    ordered.sort_unstable();
    let base = [subset_tag(&ordered).as_bytes(), b"+", &pdf_name(&family)].concat();
    let cff = face.tables().cff.is_some();

    // Widths by new glyph id (= CID).
    let mut widths: BTreeMap<u16, f32> = BTreeMap::new();
    for (&c, &cid) in &glyphs {
        let old = face.glyph_index(c).or_else(|| face.glyph_index(' ')).map_or(0, |gid| gid.0);
        widths.insert(cid, scale(f32::from(face.glyph_hor_advance(ttf_parser::GlyphId(old)).unwrap_or(0))));
    }
    let w: Vec<Object> = widths.iter().flat_map(|(&cid, &width)| [Object::Integer(i64::from(cid)), Object::Array(vec![Object::Real(width)])]).collect();

    let program = {
        let mut dict = Dictionary::new();
        if cff {
            dict.set("Subtype", Object::Name(b"OpenType".to_vec()));
        }
        let mut stream = Stream::new(dict, subset);
        let _ = stream.compress();
        document.add_object(stream)
    };
    let bbox = face.global_bounding_box();
    let mut descriptor = Dictionary::new();
    descriptor.set("Type", Object::Name(b"FontDescriptor".to_vec()));
    descriptor.set("FontName", Object::Name(base.clone()));
    // Symbolic: the glyphs are addressed through the CMap, not a standard encoding.
    descriptor.set("Flags", 4);
    descriptor.set(
        "FontBBox",
        Object::Array([bbox.x_min, bbox.y_min, bbox.x_max, bbox.y_max].iter().map(|&v| Object::Real(scale(f32::from(v)))).collect()),
    );
    descriptor.set("ItalicAngle", Object::Real(face.italic_angle()));
    descriptor.set("Ascent", Object::Real(scale(f32::from(face.ascender()))));
    descriptor.set("Descent", Object::Real(scale(f32::from(face.descender()))));
    descriptor.set("CapHeight", Object::Real(scale(f32::from(face.capital_height().unwrap_or(face.ascender())))));
    descriptor.set("StemV", 80);
    descriptor.set(if cff { "FontFile3" } else { "FontFile2" }, Object::Reference(program));
    let descriptor = document.add_object(descriptor);

    let mut system = Dictionary::new();
    system.set("Registry", Object::string_literal("Adobe"));
    system.set("Ordering", Object::string_literal("Identity"));
    system.set("Supplement", 0);
    let mut cid_font = Dictionary::new();
    cid_font.set("Type", Object::Name(b"Font".to_vec()));
    cid_font.set("Subtype", Object::Name(if cff { b"CIDFontType0".to_vec() } else { b"CIDFontType2".to_vec() }));
    cid_font.set("BaseFont", Object::Name(base.clone()));
    cid_font.set("CIDSystemInfo", Object::Dictionary(system));
    cid_font.set("FontDescriptor", Object::Reference(descriptor));
    cid_font.set("DW", 0);
    cid_font.set("W", Object::Array(w));
    if !cff {
        cid_font.set("CIDToGIDMap", Object::Name(b"Identity".to_vec()));
    }
    let cid_font = document.add_object(cid_font);

    let to_unicode = document.add_object({
        let mut stream = Stream::new(Dictionary::new(), to_unicode(&glyphs).into_bytes());
        let _ = stream.compress();
        stream
    });
    let mut type0 = Dictionary::new();
    type0.set("Type", Object::Name(b"Font".to_vec()));
    type0.set("Subtype", Object::Name(b"Type0".to_vec()));
    type0.set("BaseFont", Object::Name(base));
    type0.set("Encoding", Object::Name(b"Identity-H".to_vec()));
    type0.set("DescendantFonts", Object::Array(vec![Object::Reference(cid_font)]));
    type0.set("ToUnicode", Object::Reference(to_unicode));
    let font = document.add_object(type0);
    Ok(Embedded { font, codes: glyphs.into_iter().collect::<HashMap<char, u16>>() })
}

/// A ToUnicode CMap for two-byte codes.
fn to_unicode(glyphs: &BTreeMap<char, u16>) -> String {
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let entries: Vec<(&char, &u16)> = glyphs.iter().collect();
    for chunk in entries.chunks(100) {
        let _ = writeln!(cmap, "{} beginbfchar", chunk.len());
        for (c, cid) in chunk {
            let units: String = c.encode_utf16(&mut [0; 2]).iter().map(|unit| format!("{unit:04X}")).collect();
            let _ = writeln!(cmap, "<{cid:04X}> <{units}>");
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    cmap
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_to_unicode_cmap_that_reads_back() {
        let glyphs: BTreeMap<char, u16> = [('A', 1), ('é', 2), ('€', 3)].into_iter().collect();
        let (map, lengths) = crate::fonts::parse_to_unicode(to_unicode(&glyphs).as_bytes());
        assert_eq!(lengths, vec![2]);
        assert_eq!((map[&vec![0, 1]].as_str(), map[&vec![0, 2]].as_str(), map[&vec![0, 3]].as_str()), ("A", "é", "€"));
        assert_eq!(subset_tag(&['a', 'b']), subset_tag(&['a', 'b']));
        assert_eq!(subset_tag(&['a']).len(), 6);
    }
}
