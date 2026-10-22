//! Saved signatures, reused from one document to the next: `signatures.txt` in the config folder.
//! One per line: `typed2<TAB>STYLE<TAB>INK<TAB>Name`, or `drawn2<TAB>INK<TAB>` followed by strokes
//! separated by `|`, each a list of `x,y` points (0..1, origin top-left, in a box of
//! `DRAWN_ASPECT` width to 1 height). Lines of the first format (`typed`, `drawn`) still read.

use std::path::Path;

use recto_core::annot::Color;

use crate::recent::config_dir;

const FILE: &str = "signatures.txt";
const MAX: usize = 8;
/// Drawn signatures live in a box this many times wider than high.
pub const DRAWN_ASPECT: f32 = 2.5;

/// Handwriting fonts for typed signatures (SIL Open Font License, `assets/fonts/`): the family
/// name GPUI shows them with, and the file.
pub const STYLES: [(&str, &[u8]); 4] = [
    ("Great Vibes", include_bytes!("../assets/fonts/GreatVibes-Regular.ttf")),
    ("Allura", include_bytes!("../assets/fonts/Allura-Regular.ttf")),
    ("Sacramento", include_bytes!("../assets/fonts/Sacramento-Regular.ttf")),
    ("Dancing Script", include_bytes!("../assets/fonts/DancingScript-Variable.ttf")),
];

/// Ink colours: black, dark blue, blue.
pub const INKS: [Color; 3] =
    [Color { r: 0.07, g: 0.07, b: 0.09 }, Color { r: 0.11, g: 0.17, b: 0.42 }, Color { r: 0.12, g: 0.31, b: 0.82 }];

#[derive(Clone, Debug, PartialEq)]
pub enum Signature {
    /// A name in one of the `STYLES`, in one of the `INKS`.
    Typed { name: String, style: usize, ink: usize },
    /// Strokes in the unit box (x in 0..1 over the width, y in 0..1 from the top).
    Drawn { strokes: Vec<Vec<(f32, f32)>>, ink: usize },
}

impl Signature {
    pub fn color(&self) -> Color {
        let (Signature::Typed { ink, .. } | Signature::Drawn { ink, .. }) = self;
        INKS[(*ink).min(INKS.len() - 1)]
    }
}

fn format_line(signature: &Signature) -> String {
    match signature {
        Signature::Typed { name, style, ink } => format!("typed2\t{style}\t{ink}\t{}", name.replace(['\t', '\n'], " ")),
        Signature::Drawn { strokes, ink } => {
            let strokes: Vec<String> = strokes
                .iter()
                .map(|stroke| stroke.iter().map(|(x, y)| format!("{x:.4},{y:.4}")).collect::<Vec<_>>().join(" "))
                .collect();
            format!("drawn2\t{ink}\t{}", strokes.join("|"))
        }
    }
}

fn parse_strokes(data: &str) -> Vec<Vec<(f32, f32)>> {
    data.split('|')
        .map(|stroke| {
            stroke
                .split_whitespace()
                .filter_map(|point| {
                    let (x, y) = point.split_once(',')?;
                    Some((x.parse().ok()?, y.parse().ok()?))
                })
                .collect::<Vec<_>>()
        })
        .filter(|stroke| !stroke.is_empty())
        .collect()
}

fn parse_line(line: &str) -> Option<Signature> {
    let (kind, data) = line.split_once('\t')?;
    let index = |text: &str, limit: usize| text.parse::<usize>().ok().filter(|&n| n < limit);
    match kind {
        "typed" if !data.trim().is_empty() => Some(Signature::Typed { name: data.to_owned(), style: 0, ink: 0 }),
        "typed2" => {
            let mut fields = data.splitn(3, '\t');
            let (style, ink, name) = (index(fields.next()?, STYLES.len())?, index(fields.next()?, INKS.len())?, fields.next()?);
            (!name.trim().is_empty()).then(|| Signature::Typed { name: name.to_owned(), style, ink })
        }
        "drawn" => Some(Signature::Drawn { strokes: parse_strokes(data), ink: 0 }).filter(|s| matches!(s, Signature::Drawn { strokes, .. } if !strokes.is_empty())),
        "drawn2" => {
            let (ink, data) = data.split_once('\t')?;
            let strokes = parse_strokes(data);
            (!strokes.is_empty()).then(|| Signature::Drawn { strokes, ink: index(ink, INKS.len()).unwrap_or(0) })
        }
        _ => None,
    }
}

fn parse(text: &str) -> Vec<Signature> {
    text.lines().filter_map(parse_line).collect()
}

pub fn load() -> Vec<Signature> {
    config_dir().and_then(|dir| std::fs::read_to_string(dir.join(FILE)).ok()).map(|text| parse(&text)).unwrap_or_default()
}

/// Puts `signature` first (without duplicates) and saves the list; returns it.
pub fn remember(signature: Signature) -> Vec<Signature> {
    let mut list = load();
    list.retain(|existing| existing != &signature);
    list.insert(0, signature);
    list.truncate(MAX);
    if let Some(dir) = config_dir()
        && let Err(error) = save(&dir, &list)
    {
        eprintln!("recto: cannot save signatures: {error}");
    }
    list
}

fn save(dir: &Path, list: &[Signature]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let text: String = list.iter().map(|signature| format_line(signature) + "\n").collect();
    recto_core::save::write_atomic(&dir.join(FILE), text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_round_trip() {
        let drawn = Signature::Drawn { strokes: vec![vec![(0.1, 0.2), (0.5, 0.25)], vec![(0.9, 0.8)]], ink: 2 };
        let typed = Signature::Typed { name: "Ada Lovelace".into(), style: 3, ink: 1 };
        let text = format!("{}\n{}\n", format_line(&drawn), format_line(&typed));
        assert_eq!(parse(&text), vec![drawn, typed]);
    }

    #[test]
    fn reads_the_first_format() {
        assert_eq!(parse("typed\tAda"), vec![Signature::Typed { name: "Ada".into(), style: 0, ink: 0 }]);
        assert_eq!(parse("drawn\t0.1,0.2 0.3,0.4"), vec![Signature::Drawn { strokes: vec![vec![(0.1, 0.2), (0.3, 0.4)]], ink: 0 }]);
    }

    #[test]
    fn ignores_broken_lines() {
        assert_eq!(parse("drawn\t\nunknown\tx\ntyped\t \nno tab\ntyped2\t9\t0\tName"), vec![]);
    }

    #[test]
    fn every_style_outlines_text() {
        for (family, bytes) in STYLES {
            let outline = recto_core::annot::Outline::from_text(bytes, "Ada Lovelace");
            assert!(outline.is_some_and(|outline| outline.aspect > 2.0), "{family}");
        }
    }
}
