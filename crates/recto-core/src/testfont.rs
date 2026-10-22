//! A small TrueType font generated in code, for tests and fixtures: every glyph is a box, so no
//! font data with a licence ends up in the repository. Not for real documents.

/// Characters the font has, besides the space.
pub const CHARS: &str = "!\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~éèàçÉ€";
/// Advance of every glyph but the space, in font units (1000 per em).
pub const ADVANCE: u16 = 600;
pub const SPACE_ADVANCE: u16 = 300;

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

fn u16s(out: &mut Vec<u8>, values: &[u16]) {
    for value in values {
        out.extend_from_slice(&value.to_be_bytes());
    }
}

/// The font file, named `name` (also its PostScript name).
pub fn font(name: &str) -> Vec<u8> {
    // Glyph 0 is .notdef, 1 the space, then CHARS in order.
    let chars: Vec<char> = CHARS.chars().collect();
    let count = chars.len() + 2;
    let mut glyf = Vec::new();
    let mut loca: Vec<u32> = vec![0, 0, 0];
    for (n, _) in chars.iter().enumerate() {
        // A box whose height varies, so glyphs differ a little.
        let (x0, x1, y0, y1) = (50i16, 550i16, 0i16, 500 + (n % 5) as i16 * 50);
        let mut glyph = Vec::new();
        u16s(&mut glyph, &[1, x0 as u16, y0 as u16, x1 as u16, y1 as u16, 3, 0]);
        glyph.extend_from_slice(&[1, 1, 1, 1]); // on-curve points, 16-bit deltas
        for dx in [x0, x1 - x0, 0, x0 - x1] {
            glyph.extend_from_slice(&dx.to_be_bytes());
        }
        for dy in [y0, 0, y1 - y0, 0] {
            glyph.extend_from_slice(&dy.to_be_bytes());
        }
        while glyph.len() % 4 != 0 {
            glyph.push(0);
        }
        glyf.extend(glyph);
        loca.push(glyf.len() as u32);
    }
    let mut loca_bytes = Vec::new();
    for offset in &loca {
        loca_bytes.extend_from_slice(&offset.to_be_bytes());
    }

    let mut hmtx = Vec::new();
    u16s(&mut hmtx, &[ADVANCE, 0, SPACE_ADVANCE, 0]);
    for _ in &chars {
        u16s(&mut hmtx, &[ADVANCE, 50]);
    }

    // cmap: one format 4 subtable (3,1), a segment per character.
    let mut mapped: Vec<(u16, u16)> = vec![(32, 1)];
    mapped.extend(chars.iter().enumerate().map(|(n, &c)| (c as u16, n as u16 + 2)));
    mapped.sort_unstable();
    let segments = mapped.len() + 1;
    let mut format4 = Vec::new();
    let length = 16 + segments * 8;
    let search = 2 * (1u16 << (segments as f32).log2().floor() as u16);
    u16s(&mut format4, &[4, length as u16, 0, segments as u16 * 2, search, (search / 2).trailing_zeros() as u16, segments as u16 * 2 - search]);
    let ends: Vec<u16> = mapped.iter().map(|(c, _)| *c).chain([0xFFFF]).collect();
    u16s(&mut format4, &ends);
    format4.extend_from_slice(&[0, 0]);
    u16s(&mut format4, &ends); // starts equal ends
    let deltas: Vec<u16> = mapped.iter().map(|(c, gid)| gid.wrapping_sub(*c)).chain([1]).collect();
    u16s(&mut format4, &deltas);
    u16s(&mut format4, &vec![0; segments]);
    let mut cmap = Vec::new();
    u16s(&mut cmap, &[0, 1, 3, 1, 0, 12]);
    cmap.extend(format4);

    let mut head = Vec::new();
    u16s(&mut head, &[1, 0, 1, 0, 0, 0, 0x5F0F, 0x3CF5, 0x000B, 1000]);
    head.extend_from_slice(&[0; 16]); // created, modified
    u16s(&mut head, &[0, 0, 600, 750, 0, 8, 2, 1, 0]);

    let mut hhea = Vec::new();
    u16s(&mut hhea, &[1, 0, 800, (-200i16) as u16, 0, ADVANCE, 0, 0, 550, 1, 0, 0, 0, 0, 0, 0, 0, count as u16]);

    let mut maxp = Vec::new();
    u16s(&mut maxp, &[1, 0, count as u16, 4, 1, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0]);

    let mut post = Vec::new();
    u16s(&mut post, &[3, 0]);
    post.extend_from_slice(&[0; 28]);

    let mut os2 = Vec::new();
    // Version 4; weight 400, width 5, fsType 0 (installable: may be embedded).
    u16s(&mut os2, &[4, 500, 400, 5, 0]);
    os2.extend_from_slice(&[0; 20]); // subscript, superscript, strikeout metrics
    u16s(&mut os2, &[0]); // family class
    os2.extend_from_slice(&[0; 10]); // panose
    os2.extend_from_slice(&[0; 16]); // unicode ranges
    os2.extend_from_slice(b"RCTO");
    u16s(&mut os2, &[0x40, 32, 0x20AC, 800, (-200i16) as u16, 0, 800, 200]);
    os2.extend_from_slice(&[0; 8]); // code page ranges
    u16s(&mut os2, &[500, 700, 0, 32, 1]);

    let mut name_table = Vec::new();
    let encoded: Vec<u8> = name.encode_utf16().flat_map(u16::to_be_bytes).collect();
    let ids = [1u16, 4, 6];
    u16s(&mut name_table, &[0, ids.len() as u16, 6 + 12 * ids.len() as u16]);
    for id in ids {
        u16s(&mut name_table, &[3, 1, 0x409, id, encoded.len() as u16, 0]);
    }
    name_table.extend(&encoded);

    let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"OS/2", os2),
        (b"cmap", cmap),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca_bytes),
        (b"maxp", maxp),
        (b"name", name_table),
        (b"post", post),
    ];
    tables.sort_by_key(|(tag, _)| **tag);
    let n = tables.len() as u16;
    let search = 16 * (1u16 << (f32::from(n)).log2().floor() as u16);
    let mut out = Vec::new();
    u16s(&mut out, &[1, 0, n, search, (search / 16).trailing_zeros() as u16, n * 16 - search]);
    let mut offset = 12 + 16 * tables.len();
    let mut body = Vec::new();
    for (tag, data) in &tables {
        out.extend_from_slice(*tag);
        out.extend_from_slice(&checksum(data).to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut padded = data.clone();
        while padded.len() % 4 != 0 {
            padded.push(0);
        }
        offset += padded.len();
        body.extend(padded);
    }
    out.extend(body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_back() {
        let bytes = font("RectoTest");
        let face = ttf_parser::Face::parse(&bytes, 0).expect("the font parses");
        assert_eq!(face.units_per_em(), 1000);
        let a = face.glyph_index('A').unwrap();
        assert_eq!(face.glyph_hor_advance(a), Some(ADVANCE));
        assert!(face.glyph_bounding_box(a).is_some());
        assert_eq!(face.glyph_hor_advance(face.glyph_index(' ').unwrap()), Some(SPACE_ADVANCE));
        assert!(face.glyph_index('é').is_some() && face.glyph_index('日').is_none());
        assert_eq!(face.permissions(), Some(ttf_parser::Permissions::Installable));
        let mut remapper = subsetter::GlyphRemapper::new();
        remapper.remap(a.0);
        assert!(subsetter::subset(&bytes, 0, &remapper).is_ok());
    }
}
