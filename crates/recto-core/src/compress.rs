//! Smaller files: images resampled to `TARGET_PPI` where they are drawn larger than needed,
//! JPEGs re-encoded, other decodable images and every uncompressed stream recompressed with
//! Flate. A new image replaces the old one only when it is smaller.
//!
//! Left as they are: images this module cannot decode (JPEG 2000, JBIG2, CCITT, CMYK JPEG,
//! under 8 bits per sample), and palette or color-keyed images are recompressed but never
//! resampled.

use std::collections::HashMap;

use lopdf::content::Content;
use lopdf::{Dictionary, Object, ObjectId, Stream};

use crate::content::{Matrix, Pixels, decode_image, dictionary, raw_copy, resolve};

/// Resolution kept for images, in pixels per inch on the page.
pub const TARGET_PPI: f32 = 150.0;
/// JPEG quality of re-encoded photos.
const QUALITY: u8 = 75;
/// Images are resampled only when they exceed the target by this factor.
const SLACK: f32 = 1.3;
const MAX_FORM_DEPTH: usize = 12;
/// Most form XObjects drawn while measuring the images of a document.
const MAX_FORM_VISITS: usize = 100_000;

/// A JPEG of `data` (8-bit, `components` 1 or 3), with `image`'s dictionary.
pub(crate) fn jpeg(image: &Stream, data: &[u8], width: usize, height: usize, components: usize) -> Option<Stream> {
    let color = match components {
        1 => image::ExtendedColorType::L8,
        3 => image::ExtendedColorType::Rgb8,
        _ => return None,
    };
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, QUALITY)
        .encode(data, u32::try_from(width).ok()?, u32::try_from(height).ok()?, color)
        .ok()?;
    let mut dict = image.dict.clone();
    dict.remove(b"DecodeParms");
    dict.set("Filter", Object::Name(b"DCTDecode".to_vec()));
    dict.set("BitsPerComponent", 8);
    dict.set("Width", width as i64);
    dict.set("Height", height as i64);
    let mut stream = Stream::new(dict, bytes);
    // Already compressed: Flate would not help.
    stream.allows_compression = false;
    Some(stream)
}

/// The largest size each image is drawn at on the pages, in points (width, height).
fn drawn_sizes(document: &lopdf::Document) -> HashMap<ObjectId, (f32, f32)> {
    fn walk(document: &lopdf::Document, bytes: &[u8], resources: &Dictionary, ctm: Matrix, depth: usize, sizes: &mut HashMap<ObjectId, (f32, f32)>, budget: &mut usize) {
        let Ok(content) = Content::decode(bytes) else { return };
        let xobjects = resources.get(b"XObject").ok().and_then(|value| dictionary(document, value));
        let mut stack = Vec::new();
        let mut ctm = ctm;
        for operation in content.operations {
            match operation.operator.as_str() {
                "q" => stack.push(ctm),
                "Q" => ctm = stack.pop().unwrap_or(ctm),
                "cm" => {
                    if let Some(matrix) = Matrix::from_operands(&operation.operands) {
                        ctm = matrix.then(ctm);
                    }
                }
                "Do" => {
                    let Some(name) = operation.operands.first().and_then(|value| value.as_name().ok()) else { continue };
                    let Some(Ok(id)) = xobjects.and_then(|xobjects| xobjects.get(name).ok()).map(Object::as_reference) else { continue };
                    let Ok(stream) = document.get_object(id).and_then(Object::as_stream) else { continue };
                    match stream.dict.get(b"Subtype").ok().and_then(|value| value.as_name().ok()) {
                        Some(b"Image") => {
                            let [a, b, c, d, _, _] = ctm.0;
                            let (width, height) = ((a * a + b * b).sqrt(), (c * c + d * d).sqrt());
                            let size = sizes.entry(id).or_insert((0.0, 0.0));
                            *size = (size.0.max(width), size.1.max(height));
                            if let Ok(mask) = stream.dict.get(b"SMask").and_then(Object::as_reference) {
                                let size = sizes.entry(mask).or_insert((0.0, 0.0));
                                *size = (size.0.max(width), size.1.max(height));
                            }
                        }
                        Some(b"Form") if depth < MAX_FORM_DEPTH && *budget > 0 => {
                            *budget -= 1;
                            let matrix = Matrix::from_object(stream.dict.get(b"Matrix").ok());
                            let own = stream.dict.get(b"Resources").ok().and_then(|value| dictionary(document, value));
                            let bytes = stream.decompressed_content_with_limit(crate::content::MAX_STREAM_BYTES).unwrap_or_else(|_| stream.content.clone());
                            walk(document, &bytes, own.unwrap_or(resources), matrix.then(ctm), depth + 1, sizes, budget);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    let mut sizes = HashMap::new();
    // Forms drawn in all: a form drawing itself ten times, nested, would be visited 10^12 times.
    let mut budget = MAX_FORM_VISITS;
    for page in document.get_pages().into_values() {
        let Ok(page_dictionary) = document.get_dictionary(page) else { continue };
        let resources = page_dictionary.get(b"Resources").ok().and_then(|value| dictionary(document, value)).cloned().unwrap_or_default();
        walk(document, &crate::content::page_content(document, page), &resources, Matrix::IDENTITY, 0, &mut sizes, &mut budget);
    }
    sizes
}

/// Resamples `pixels` to `width` × `height`; 1 or 3 components.
fn resample(pixels: &Pixels, width: u32, height: u32) -> Option<Vec<u8>> {
    use image::imageops::{FilterType, resize};
    let (w, h) = (u32::try_from(pixels.width).ok()?, u32::try_from(pixels.height).ok()?);
    Some(match pixels.components {
        1 => resize(&image::GrayImage::from_raw(w, h, pixels.data.clone())?, width, height, FilterType::Triangle).into_raw(),
        3 => resize(&image::RgbImage::from_raw(w, h, pixels.data.clone())?, width, height, FilterType::Triangle).into_raw(),
        _ => return None,
    })
}

/// A smaller version of `image`, or `None` to keep it.
fn shrink(document: &lopdf::Document, image: &Stream, drawn: Option<(f32, f32)>) -> Option<Stream> {
    let pixels = decode_image(document, image)?;
    let dict = &image.dict;
    let filter = dict.get(b"Filter").ok().map(|value| resolve(document, value));
    let was_jpeg = matches!(filter, Some(Object::Name(name)) if name == b"DCTDecode");
    let indexed = dict
        .get(b"ColorSpace")
        .ok()
        .map(|value| resolve(document, value))
        .and_then(|value| value.as_array().ok())
        .and_then(|items| items.first())
        .and_then(|value| value.as_name().ok())
        .is_some_and(|family| family == b"Indexed");
    let resizable = !indexed && !dict.has(b"Mask") && matches!(pixels.components, 1 | 3);

    // Pixels needed where the image is drawn largest.
    let target = drawn.map(|(width, height)| {
        (((width / 72.0 * TARGET_PPI).ceil() as usize).max(1), ((height / 72.0 * TARGET_PPI).ceil() as usize).max(1))
    });
    let (width, height, data) = match target {
        Some((tw, th)) if resizable && (pixels.width as f32 > tw as f32 * SLACK || pixels.height as f32 > th as f32 * SLACK) => {
            let (tw, th) = (tw.min(pixels.width), th.min(pixels.height));
            (tw, th, resample(&pixels, tw as u32, th as u32)?)
        }
        _ => (pixels.width, pixels.height, pixels.data.clone()),
    };
    let resampled = width != pixels.width || height != pixels.height;
    let smaller = if was_jpeg {
        jpeg(image, &data, width, height, pixels.components)?
    } else {
        let mut copy = raw_copy(image, Pixels { width, height, components: pixels.components, data });
        copy.dict.set("Width", width as i64);
        copy.dict.set("Height", height as i64);
        copy
    };
    // Re-encoding a JPEG at its own size is only worth it when it saves a good part.
    let worth = if was_jpeg && !resampled { smaller.content.len() * 10 < image.content.len() * 9 } else { smaller.content.len() < image.content.len() };
    worth.then_some(smaller)
}

/// Shrinks the images of `document` and compresses its uncompressed streams.
pub fn images(document: &mut lopdf::Document) {
    let drawn = drawn_sizes(document);
    let images: Vec<ObjectId> = document
        .objects
        .iter()
        .filter(|(_, object)| {
            object.as_stream().is_ok_and(|stream| stream.dict.get(b"Subtype").ok().and_then(|value| value.as_name().ok()) == Some(b"Image"))
        })
        .map(|(&id, _)| id)
        .collect();
    for id in images {
        let Ok(image) = document.get_object(id).and_then(Object::as_stream) else { continue };
        if let Some(smaller) = shrink(document, image, drawn.get(&id).copied()) {
            document.objects.insert(id, Object::Stream(smaller));
        }
    }
    document.compress();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: usize, height: usize, filter: Option<&str>) -> Stream {
        let mut dict = Dictionary::new();
        dict.set("Type", Object::Name(b"XObject".to_vec()));
        dict.set("Subtype", Object::Name(b"Image".to_vec()));
        dict.set("Width", width as i64);
        dict.set("Height", height as i64);
        dict.set("ColorSpace", Object::Name(b"DeviceRGB".to_vec()));
        dict.set("BitsPerComponent", 8);
        // Noise compresses badly, like a photo.
        let mut seed = 12345u32;
        let data: Vec<u8> = (0..width * height * 3)
            .map(|_| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                (seed >> 16) as u8
            })
            .collect();
        match filter {
            Some(_) => jpeg(&Stream::new(dict, Vec::new()), &data, width, height, 3).unwrap(),
            None => Stream::new(dict, data),
        }
    }

    #[test]
    fn resamples_to_the_drawn_size() {
        let document = lopdf::Document::with_version("1.7");
        // 600 px drawn 72 pt wide (1 inch): 150 px are enough.
        let raw = image(600, 300, None);
        let smaller = shrink(&document, &raw, Some((72.0, 36.0))).unwrap();
        assert_eq!((smaller.dict.get(b"Width").unwrap().as_i64().unwrap(), smaller.dict.get(b"Height").unwrap().as_i64().unwrap()), (150, 75));
        let photo = image(600, 300, Some("DCTDecode"));
        let smaller = shrink(&document, &photo, Some((72.0, 36.0))).unwrap();
        assert_eq!(smaller.dict.get(b"Filter").unwrap().as_name().unwrap(), b"DCTDecode");
        assert!(smaller.content.len() < photo.content.len());
        // Drawn large enough: a raw image is only recompressed (noise does not compress).
        assert!(shrink(&document, &raw, Some((600.0, 300.0))).is_none());
    }
}
