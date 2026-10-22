//! Grayscale conversion of a whole document: colors set in content streams (pages, forms,
//! patterns, annotation appearances), images, and annotation colors.
//!
//! Not converted: shadings (gradients), spot colors (Separation, DeviceN), Lab colors, and
//! images this module cannot decode (JPEG 2000, JBIG2, CMYK JPEG, under 8 bits per sample).

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Object, ObjectId, Stream};

use crate::content::{decode_image, dictionary, raw_copy, resolve};

/// Luminance of an RGB color, components in 0..=1.
fn luma(r: f32, g: f32, b: f32) -> f32 {
    0.299 * r + 0.587 * g + 0.114 * b
}

fn cmyk_gray(c: f32, m: f32, y: f32, k: f32) -> f32 {
    1.0 - (0.3 * c + 0.59 * m + 0.11 * y + k).min(1.0)
}

/// Gray level of a color given as 3 (RGB) or 4 (CMYK) components.
fn gray_of(values: &[f32]) -> Option<f32> {
    match values {
        [r, g, b] => Some(luma(*r, *g, *b)),
        [c, m, y, k] => Some(cmyk_gray(*c, *m, *y, *k)),
        _ => None,
    }
}

/// Components of a color space that this module turns gray: 3 or 4, or `None` to leave it.
fn convertible(document: &lopdf::Document, resources: &Dictionary, space: &Object) -> Option<usize> {
    let space = match space {
        // A name is a device space or an entry of the resources' /ColorSpace.
        Object::Name(name) => match name.as_slice() {
            b"DeviceRGB" | b"RGB" => return Some(3),
            b"DeviceCMYK" | b"CMYK" => return Some(4),
            b"DeviceGray" | b"G" | b"Pattern" => return None,
            other => resources
                .get(b"ColorSpace")
                .ok()
                .and_then(|value| dictionary(document, value))
                .and_then(|spaces| spaces.get(other).ok())
                .map(|value| resolve(document, value))?,
        },
        other => resolve(document, other),
    };
    let items = space.as_array().ok()?;
    match items.first().and_then(|value| value.as_name().ok())? {
        b"CalRGB" => Some(3),
        b"ICCBased" => {
            let n = resolve(document, items.get(1)?).as_stream().ok()?.dict.get(b"N").ok()?.as_i64().ok()?;
            matches!(n, 3 | 4).then_some(n as usize)
        }
        _ => None,
    }
}

/// Converts the colors of one content stream.
fn convert_content(document: &lopdf::Document, resources: &Dictionary, operations: Vec<Operation>) -> Vec<Operation> {
    // Components of the current fill and stroke spaces when they are converted.
    let (mut fill, mut stroke): (Option<usize>, Option<usize>) = (None, None);
    let mut stack: Vec<(Option<usize>, Option<usize>)> = Vec::new();
    let gray_space = || Object::Name(b"DeviceGray".to_vec());
    let numbers = |operands: &[Object]| -> Option<Vec<f32>> { operands.iter().map(|value| value.as_float().ok()).collect() };
    operations
        .into_iter()
        .map(|mut operation| {
            let operator = operation.operator.clone();
            match operator.as_str() {
                "q" => stack.push((fill, stroke)),
                "Q" => (fill, stroke) = stack.pop().unwrap_or((None, None)),
                "rg" | "RG" | "k" | "K" => {
                    if let Some(gray) = numbers(&operation.operands).and_then(|values| gray_of(&values)) {
                        let lower = operator.chars().all(|c| c.is_lowercase());
                        return Operation::new(if lower { "g" } else { "G" }, vec![Object::Real(gray)]);
                    }
                }
                "cs" | "CS" => {
                    let converted = operation.operands.first().and_then(|space| convertible(document, resources, space));
                    if operator == "cs" { fill = converted } else { stroke = converted }
                    if converted.is_some() {
                        operation.operands = vec![gray_space()];
                    }
                }
                "sc" | "scn" | "SC" | "SCN" => {
                    let space = if operator.starts_with('s') { fill } else { stroke };
                    if let Some(gray) = space.and_then(|_| numbers(&operation.operands)).and_then(|values| gray_of(&values)) {
                        operation.operands = vec![Object::Real(gray)];
                    }
                }
                "BI" => {
                    if let Some(Object::Stream(image)) = operation.operands.first_mut() {
                        convert_inline(image);
                    }
                }
                _ => {}
            }
            operation
        })
        .collect()
}

/// Uncompressed 8-bit RGB or CMYK inline images become gray.
fn convert_inline(image: &mut Stream) {
    let dict = &image.dict;
    let has_filter = dict.has(b"F") || dict.has(b"Filter");
    let bits = dict.get(b"BPC").or_else(|_| dict.get(b"BitsPerComponent")).and_then(Object::as_i64).ok();
    let space = dict.get(b"CS").or_else(|_| dict.get(b"ColorSpace")).and_then(Object::as_name).ok();
    let components = match space {
        Some(b"RGB" | b"DeviceRGB") => 3,
        Some(b"CMYK" | b"DeviceCMYK") => 4,
        _ => return,
    };
    if has_filter || bits != Some(8) || dict.has(b"D") || dict.has(b"Decode") {
        return;
    }
    let gray: Vec<u8> = image
        .content
        .chunks_exact(components)
        .map(|pixel| {
            let values: Vec<f32> = pixel.iter().map(|&value| f32::from(value) / 255.0).collect();
            (gray_of(&values).unwrap_or(0.0) * 255.0).round() as u8
        })
        .collect();
    image.dict.remove(b"ColorSpace");
    image.dict.set("CS", Object::Name(b"G".to_vec()));
    image.set_content(gray);
}

/// A gray copy of an image, or `None` when it is gray already or cannot be decoded.
fn convert_image(document: &lopdf::Document, image: &Stream) -> Option<Stream> {
    let dict = &image.dict;
    if dict.has(b"Decode") || dict.get(b"ImageMask").ok().and_then(|value| value.as_bool().ok()) == Some(true) {
        return None;
    }
    let space = resolve(document, dict.get(b"ColorSpace").ok()?);
    // Indexed images: only the palette changes.
    if let Ok(items) = space.as_array()
        && items.first().and_then(|value| value.as_name().ok()) == Some(b"Indexed")
    {
        let base = convertible(document, &Dictionary::new(), items.get(1)?)?;
        let palette = match resolve(document, items.get(3)?) {
            Object::String(bytes, _) => bytes.clone(),
            Object::Stream(stream) => stream.decompressed_content_with_limit(crate::content::MAX_STREAM_BYTES).unwrap_or_else(|_| stream.content.clone()),
            _ => return None,
        };
        let gray: Vec<u8> = palette
            .chunks_exact(base)
            .map(|entry| {
                let values: Vec<f32> = entry.iter().map(|&value| f32::from(value) / 255.0).collect();
                (gray_of(&values).unwrap_or(0.0) * 255.0).round() as u8
            })
            .collect();
        let mut copy = image.clone();
        let new_space = vec![items[0].clone(), Object::Name(b"DeviceGray".to_vec()), items.get(2)?.clone(), Object::String(gray, lopdf::StringFormat::Hexadecimal)];
        copy.dict.set("ColorSpace", Object::Array(new_space));
        return Some(copy);
    }
    let components = convertible(document, &Dictionary::new(), space)?;
    let pixels = decode_image(document, image)?;
    if pixels.components != components {
        return None;
    }
    let was_jpeg = matches!(dict.get(b"Filter").ok().map(|value| resolve(document, value)), Some(Object::Name(name)) if name == b"DCTDecode");
    let gray: Vec<u8> = pixels
        .data
        .chunks_exact(components)
        .map(|pixel| {
            let values: Vec<f32> = pixel.iter().map(|&value| f32::from(value) / 255.0).collect();
            (gray_of(&values).unwrap_or(0.0) * 255.0).round() as u8
        })
        .collect();
    let (width, height) = (pixels.width, pixels.height);
    let mut copy = if was_jpeg {
        crate::compress::jpeg(image, &gray, width, height, 1)?
    } else {
        raw_copy(image, crate::content::Pixels { width, height, components: 1, data: gray })
    };
    copy.dict.set("ColorSpace", Object::Name(b"DeviceGray".to_vec()));
    Some(copy)
}

/// Converts the whole document.
pub fn convert(document: &mut lopdf::Document) {
    // Content streams with the resources they are drawn with.
    let mut contents: Vec<(ObjectId, Dictionary)> = Vec::new();
    for page in document.get_pages().into_values() {
        let Ok(dictionary_of_page) = document.get_dictionary(page) else { continue };
        let resources = dictionary_of_page.get(b"Resources").ok().and_then(|value| dictionary(document, value)).cloned().unwrap_or_default();
        for id in document.get_page_contents(page) {
            if !contents.iter().any(|(seen, _)| *seen == id) {
                contents.push((id, resources.clone()));
            }
        }
    }
    let mut images: Vec<ObjectId> = Vec::new();
    for (&id, object) in &document.objects {
        let Object::Stream(stream) = object else { continue };
        let subtype = stream.dict.get(b"Subtype").ok().and_then(|value| value.as_name().ok());
        let pattern = stream.dict.get(b"PatternType").ok().and_then(|value| value.as_i64().ok()) == Some(1);
        if subtype == Some(b"Form") || pattern {
            let resources = stream.dict.get(b"Resources").ok().and_then(|value| dictionary(document, value)).cloned().unwrap_or_default();
            contents.push((id, resources));
        } else if subtype == Some(b"Image") {
            images.push(id);
        }
    }

    for (id, resources) in contents {
        let Ok(stream) = document.get_object(id).and_then(Object::as_stream) else { continue };
        let Ok(bytes) = (if stream.dict.has(b"Filter") { stream.decompressed_content_with_limit(crate::content::MAX_STREAM_BYTES) } else { Ok(stream.content.clone()) }) else { continue };
        let Ok(content) = Content::decode(&bytes) else { continue };
        let operations = convert_content(document, &resources, content.operations);
        let Ok(encoded) = (Content { operations }).encode() else { continue };
        if let Ok(Object::Stream(stream)) = document.get_object_mut(id) {
            stream.dict.remove(b"Filter");
            stream.dict.remove(b"DecodeParms");
            stream.set_content(encoded);
            let _ = stream.compress();
        }
    }
    for id in images {
        let Ok(image) = document.get_object(id).and_then(Object::as_stream) else { continue };
        if let Some(copy) = convert_image(document, image) {
            document.objects.insert(id, Object::Stream(copy));
        }
    }
    // Annotation colors used by readers that redraw annotations.
    for object in document.objects.values_mut() {
        let Object::Dictionary(annotation) = object else { continue };
        if annotation.get(b"Type").ok().and_then(|value| value.as_name().ok()) != Some(b"Annot") && !annotation.has(b"Subtype") {
            continue;
        }
        for key in [b"C".as_slice(), b"IC"] {
            let gray = annotation
                .get(key)
                .ok()
                .and_then(|value| value.as_array().ok())
                .and_then(|items| items.iter().map(|value| value.as_float().ok()).collect::<Option<Vec<f32>>>())
                .and_then(|values| gray_of(&values));
            if let Some(gray) = gray {
                annotation.set(key, Object::Array(vec![Object::Real(gray)]));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_color_operators() {
        let document = lopdf::Document::with_version("1.7");
        let content = b"1 0 0 rg 0 0 1 RG 0 0 0 1 k q /DeviceRGB cs 0 1 0 sc Q 1 1 1 scn /Pattern cs /P1 scn";
        let operations = convert_content(&document, &Dictionary::new(), Content::decode(content).unwrap().operations);
        let text = String::from_utf8(Content { operations }.encode().unwrap()).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "0.299 g");
        assert_eq!(lines[1], "0.114 G");
        assert_eq!(lines[2], "0 g");
        assert_eq!(lines[4], "/DeviceGray cs");
        assert_eq!(lines[5], "0.587 sc");
        // After Q the fill space is DeviceGray again (the default): operands are left alone.
        assert_eq!(lines[7], "1 1 1 scn");
        assert_eq!(&lines[8..], ["/Pattern cs", "/P1 scn"]);
    }
}
