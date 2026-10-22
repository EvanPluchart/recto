//! What the characters of a font are: codes to Unicode (ToUnicode CMaps, simple encodings) and
//! back, and whether the embedded font program has a glyph for a code. Used to edit text.

use std::collections::HashMap;

use lopdf::{Dictionary, Object};

use crate::content::{dictionary, resolve};

/// One character code as it appears in a string: its bytes.
pub type Code = Vec<u8>;

/// The characters a font can show, as far as the file tells.
#[derive(Clone, Debug, Default)]
pub struct Charset {
    /// Code lengths in bytes allowed by the font (1 for simple fonts, from the codespace ranges
    /// otherwise).
    lengths: Vec<usize>,
    to_unicode: HashMap<Code, String>,
    /// The font program can be read (TrueType or OpenType) and these codes have a glyph in it.
    has_glyph: Option<HashMap<Code, bool>>,
    /// Not embedded: the reader draws every code with a substitute font.
    pub embedded: bool,
    /// Text in this font can be decoded and re-encoded.
    pub editable: bool,
}

impl Charset {
    /// Reads a font dictionary.
    pub fn load(document: &lopdf::Document, font: &Dictionary) -> Charset {
        let name = |dict: &Dictionary, key: &[u8]| dict.get(key).ok().map(|value| resolve(document, value)).and_then(|value| value.as_name().ok().map(<[u8]>::to_vec));
        let subtype = name(font, b"Subtype").unwrap_or_default();
        let to_unicode = font
            .get(b"ToUnicode")
            .ok()
            .and_then(|value| resolve(document, value).as_stream().ok())
            .and_then(|stream| stream.decompressed_content().ok().or_else(|| Some(stream.content.clone())))
            .map(|bytes| parse_to_unicode(&bytes));
        let mut charset = Charset::default();

        if subtype == b"Type0" {
            let encoding = name(font, b"Encoding").unwrap_or_default();
            let descendant = font
                .get(b"DescendantFonts")
                .ok()
                .map(|value| resolve(document, value))
                .and_then(|value| value.as_array().ok())
                .and_then(|items| items.first())
                .and_then(|value| dictionary(document, value));
            let Some((map, _)) = to_unicode.filter(|_| encoding == b"Identity-H") else { return charset };
            charset.lengths = vec![2];
            charset.to_unicode = map.into_iter().filter(|(code, _)| code.len() == 2).collect();
            if let Some(descendant) = descendant {
                let descriptor = descendant.get(b"FontDescriptor").ok().and_then(|value| dictionary(document, value));
                let program = descriptor.and_then(|descriptor| font_program(document, descriptor));
                charset.embedded = descriptor.is_some_and(is_embedded);
                let identity = descendant.get(b"CIDToGIDMap").ok().map(|value| resolve(document, value));
                let gid_map: Option<Vec<u8>> = match identity {
                    None | Some(Object::Name(_)) => None,
                    Some(Object::Stream(stream)) => stream.decompressed_content().ok(),
                    Some(_) => None,
                };
                if let Some(program) = program
                    && let Ok(face) = ttf_parser::Face::parse(&program, 0)
                {
                    let glyphs = face.number_of_glyphs();
                    let has = charset
                        .to_unicode
                        .iter()
                        .map(|(code, text)| {
                            let cid = u16::from_be_bytes([code[0], code[1]]);
                            let gid = match &gid_map {
                                Some(map) => map.get(cid as usize * 2..cid as usize * 2 + 2).map(|pair| u16::from_be_bytes([pair[0], pair[1]])),
                                None => Some(cid),
                            };
                            let drawn = gid.is_some_and(|gid| gid != 0 && gid < glyphs && has_outline(&face, gid, text));
                            (code.clone(), drawn)
                        })
                        .collect();
                    charset.has_glyph = Some(has);
                }
            }
            charset.editable = !charset.to_unicode.is_empty();
            return charset;
        }

        if subtype == b"Type3" {
            return charset;
        }
        // Simple fonts: one byte per code; ToUnicode first, then the font's encoding.
        charset.lengths = vec![1];
        let (map, _) = to_unicode.unwrap_or_default();
        let encoding = font.get_font_encoding(document).ok();
        for byte in 0..=255u8 {
            let code = vec![byte];
            let text = map.get(&code).cloned().or_else(|| {
                encoding.as_ref().and_then(|encoding| encoding.bytes_to_string(&[byte]).ok()).filter(|text| {
                    !text.is_empty() && !text.contains('\u{fffd}') && !text.chars().all(|c| c.is_control())
                })
            });
            if let Some(text) = text {
                charset.to_unicode.insert(code, text);
            }
        }
        let descriptor = font.get(b"FontDescriptor").ok().and_then(|value| dictionary(document, value));
        charset.embedded = descriptor.is_some_and(is_embedded);
        if let Some(program) = descriptor.and_then(|descriptor| font_program(document, descriptor))
            && let Ok(face) = ttf_parser::Face::parse(&program, 0)
        {
            let has = charset
                .to_unicode
                .iter()
                .map(|(code, text)| (code.clone(), simple_glyph(&face, code[0], text).is_some_and(|gid| has_outline(&face, gid, text))))
                .collect();
            charset.has_glyph = Some(has);
        }
        charset.editable = !charset.to_unicode.is_empty();
        charset
    }

    /// Splits a shown string into codes; `None` when the bytes do not fit the code lengths.
    pub fn split(&self, bytes: &[u8]) -> Option<Vec<Code>> {
        let length = *self.lengths.first()?;
        bytes.len().is_multiple_of(length).then(|| bytes.chunks(length).map(<[u8]>::to_vec).collect())
    }

    /// The text of a code, if known.
    pub fn text(&self, code: &[u8]) -> Option<&str> {
        self.to_unicode.get(code).map(String::as_str)
    }

    /// The text of a shown string; `None` if any code is unknown.
    pub fn decode(&self, bytes: &[u8]) -> Option<String> {
        self.split(bytes)?.iter().map(|code| self.text(code)).collect()
    }

    /// Codes showing `text`, character by character, preferring codes in `used`; `None` if a
    /// character has no code or no glyph that is known to be drawn.
    pub fn encode(&self, text: &str, used: &std::collections::HashSet<Code>) -> Option<Vec<u8>> {
        let mut by_char: HashMap<char, Vec<&Code>> = HashMap::new();
        for (code, value) in &self.to_unicode {
            let mut chars = value.chars();
            if let (Some(c), None) = (chars.next(), chars.next()) {
                by_char.entry(c).or_default().push(code);
            }
        }
        let mut bytes = Vec::new();
        for c in text.chars() {
            let candidates = by_char.get(&c)?;
            let drawable = |code: &&Code| {
                used.contains(*code)
                    || !self.embedded
                    || self.has_glyph.as_ref().is_some_and(|has| has.get(*code).copied().unwrap_or(false))
            };
            // Codes already shown first, then the lowest, for a stable choice.
            let mut sorted: Vec<&&Code> = candidates.iter().filter(|code| drawable(code)).collect();
            sorted.sort_by_key(|code| (!used.contains(**code), (**code).clone()));
            bytes.extend_from_slice(sorted.first()?);
        }
        Some(bytes)
    }
}

fn is_embedded(descriptor: &Dictionary) -> bool {
    [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"].iter().any(|key| descriptor.has(key))
}

/// The embedded TrueType or OpenType program, when there is one ttf-parser can read.
fn font_program(document: &lopdf::Document, descriptor: &Dictionary) -> Option<Vec<u8>> {
    if let Ok(value) = descriptor.get(b"FontFile2") {
        let stream = resolve(document, value).as_stream().ok()?;
        return stream.decompressed_content().ok().or_else(|| (!stream.dict.has(b"Filter")).then(|| stream.content.clone()));
    }
    let stream = resolve(document, descriptor.get(b"FontFile3").ok()?).as_stream().ok()?;
    // Bare CFF (Type1C, CIDFontType0C) is not read; OpenType is.
    (stream.dict.get(b"Subtype").ok()?.as_name().ok()? == b"OpenType").then(|| stream.decompressed_content().ok())?
}

/// Glyph of a code in a simple TrueType font: through its (3,1) Unicode, (3,0) symbol or
/// (1,0) Macintosh cmap.
fn simple_glyph(face: &ttf_parser::Face, code: u8, text: &str) -> Option<u16> {
    let cmap = face.tables().cmap?;
    let c = text.chars().next()?;
    for table in cmap.subtables {
        let gid = match (table.platform_id, table.encoding_id) {
            (ttf_parser::PlatformId::Windows, 1) => table.glyph_index(u32::from(c)),
            (ttf_parser::PlatformId::Windows, 0) => {
                table.glyph_index(0xF000 + u32::from(code)).or_else(|| table.glyph_index(u32::from(code)))
            }
            (ttf_parser::PlatformId::Macintosh, 0) => table.glyph_index(u32::from(code)),
            _ => None,
        };
        if let Some(gid) = gid.filter(|gid| gid.0 != 0) {
            return Some(gid.0);
        }
    }
    None
}

/// A glyph is drawn when it has an outline, or is meant to be blank.
fn has_outline(face: &ttf_parser::Face, gid: u16, text: &str) -> bool {
    text.chars().all(char::is_whitespace) || face.glyph_bounding_box(ttf_parser::GlyphId(gid)).is_some()
}

/// Parses a ToUnicode CMap: codes (their bytes) to text, and the code lengths of its codespace.
pub fn parse_to_unicode(bytes: &[u8]) -> (HashMap<Code, String>, Vec<usize>) {
    #[derive(Debug, PartialEq)]
    enum Token {
        Hex(Vec<u8>),
        Open,
        Close,
        Word(Vec<u8>),
    }
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'<' if bytes.get(i + 1) != Some(&b'<') => {
                let end = bytes[i..].iter().position(|&b| b == b'>').map_or(bytes.len(), |p| i + p);
                let digits: Vec<u8> = bytes[i + 1..end].iter().copied().filter(u8::is_ascii_hexdigit).collect();
                let hex = digits
                    .chunks(2)
                    .map(|pair| {
                        let text = std::str::from_utf8(pair).unwrap_or("0");
                        // An odd last digit is followed by an implicit 0.
                        u8::from_str_radix(&format!("{text:0<2}"), 16).unwrap_or(0)
                    })
                    .collect();
                tokens.push(Token::Hex(hex));
                i = end + 1;
            }
            b'[' => {
                tokens.push(Token::Open);
                i += 1;
            }
            b']' => {
                tokens.push(Token::Close);
                i += 1;
            }
            b'%' => {
                while i < bytes.len() && bytes[i] != b'\n' && bytes[i] != b'\r' {
                    i += 1;
                }
            }
            byte if byte.is_ascii_whitespace() => i += 1,
            _ => {
                let start = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !b"<>[]%".contains(&bytes[i]) {
                    i += 1;
                }
                if i == start {
                    i += 1;
                }
                tokens.push(Token::Word(bytes[start..i].to_vec()));
            }
        }
    }

    let utf16 = |hex: &[u8]| -> String {
        let units: Vec<u16> = hex.chunks(2).map(|pair| u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)])).collect();
        String::from_utf16_lossy(&units)
    };
    let mut map = HashMap::new();
    let mut lengths = Vec::new();
    let mut section: &[u8] = b"";
    let mut t = 0;
    while t < tokens.len() {
        match &tokens[t] {
            Token::Word(word) if word.starts_with(b"begin") => {
                section = match word.as_slice() {
                    b"begincodespacerange" => b"codespace",
                    b"beginbfchar" => b"bfchar",
                    b"beginbfrange" => b"bfrange",
                    _ => b"",
                };
                t += 1;
            }
            Token::Word(word) if word.starts_with(b"end") => {
                section = b"";
                t += 1;
            }
            Token::Hex(low) if section == b"codespace" => {
                if !lengths.contains(&low.len()) {
                    lengths.push(low.len());
                }
                t += 2;
            }
            Token::Hex(code) if section == b"bfchar" => {
                if let Some(Token::Hex(target)) = tokens.get(t + 1) {
                    map.insert(code.clone(), utf16(target));
                }
                t += 2;
            }
            Token::Hex(low) if section == b"bfrange" => {
                let (Some(Token::Hex(high)), Some(target)) = (tokens.get(t + 1), tokens.get(t + 2)) else { break };
                let as_number = |bytes: &[u8]| bytes.iter().fold(0u32, |n, &b| n << 8 | u32::from(b));
                let (first, last) = (as_number(low), as_number(high));
                let width = low.len();
                let code_of = |n: u32| n.to_be_bytes()[4 - width.min(4)..].to_vec();
                // Bounded so a broken range cannot run for ever.
                let last = last.min(first.saturating_add(65_535));
                match target {
                    Token::Hex(start) => {
                        for (offset, n) in (first..=last).enumerate() {
                            // The last UTF-16 unit is incremented along the range.
                            let mut units: Vec<u16> =
                                start.chunks(2).map(|pair| u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)])).collect();
                            if let Some(unit) = units.last_mut() {
                                *unit = unit.wrapping_add(offset as u16);
                            }
                            map.insert(code_of(n), String::from_utf16_lossy(&units));
                        }
                        t += 3;
                    }
                    Token::Open => {
                        let mut n = first;
                        t += 3;
                        while let Some(Token::Hex(target)) = tokens.get(t) {
                            if n <= last {
                                map.insert(code_of(n), utf16(target));
                            }
                            n += 1;
                            t += 1;
                        }
                        t += 1; // the closing bracket
                    }
                    _ => t += 3,
                }
            }
            _ => t += 1,
        }
    }
    (map, lengths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_to_unicode_cmaps() {
        let cmap = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
            1 begincodespacerange <0000> <FFFF> endcodespacerange\n\
            2 beginbfchar <0003> <0020> <0011> <00660069> endbfchar\n\
            2 beginbfrange <0024> <0026> <0041> <0030> <0031> [<00E9> <00E8>] endbfrange\n\
            endcmap CMapName currentdict /CMap defineresource pop end end";
        let (map, lengths) = parse_to_unicode(cmap);
        assert_eq!(lengths, vec![2]);
        assert_eq!(map[&vec![0, 3]], " ");
        assert_eq!(map[&vec![0, 0x11]], "fi");
        assert_eq!((map[&vec![0, 0x24]].as_str(), map[&vec![0, 0x26]].as_str()), ("A", "C"));
        assert_eq!((map[&vec![0, 0x30]].as_str(), map[&vec![0, 0x31]].as_str()), ("é", "è"));
    }

    #[test]
    fn simple_fonts_use_their_encoding() {
        let document = lopdf::Document::with_version("1.7");
        let mut font = Dictionary::new();
        font.set("Type", Object::Name(b"Font".to_vec()));
        font.set("Subtype", Object::Name(b"Type1".to_vec()));
        font.set("BaseFont", Object::Name(b"Helvetica".to_vec()));
        font.set("Encoding", Object::Name(b"WinAnsiEncoding".to_vec()));
        let charset = Charset::load(&document, &font);
        assert!(charset.editable && !charset.embedded);
        assert_eq!(charset.decode(b"Caf\xe9").as_deref(), Some("Café"));
        let none = std::collections::HashSet::new();
        assert_eq!(charset.encode("Été €", &none), Some(b"\xc9t\xe9 \x80".to_vec()));
        assert_eq!(charset.encode("日本", &none), None);
    }
}
