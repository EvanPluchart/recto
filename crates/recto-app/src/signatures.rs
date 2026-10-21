//! Saved signatures, reused from one document to the next: `signatures.txt` in the config folder.
//! One per line: `typed<TAB>Name`, or `drawn<TAB>` followed by strokes separated by `|`, each a
//! list of `x,y` points (0..1, origin top-left, in a box of `DRAWN_ASPECT` width to 1 height).

use std::path::Path;

use crate::recent::config_dir;

const FILE: &str = "signatures.txt";
const MAX: usize = 8;
/// Drawn signatures live in a box this many times wider than high.
pub const DRAWN_ASPECT: f32 = 2.5;

#[derive(Clone, Debug, PartialEq)]
pub enum Signature {
    Typed(String),
    /// Strokes in the unit box (x in 0..1 over the width, y in 0..1 from the top).
    Drawn(Vec<Vec<(f32, f32)>>),
}

fn format_line(signature: &Signature) -> String {
    match signature {
        Signature::Typed(name) => format!("typed\t{}", name.replace(['\t', '\n'], " ")),
        Signature::Drawn(strokes) => {
            let strokes: Vec<String> = strokes
                .iter()
                .map(|stroke| stroke.iter().map(|(x, y)| format!("{x:.4},{y:.4}")).collect::<Vec<_>>().join(" "))
                .collect();
            format!("drawn\t{}", strokes.join("|"))
        }
    }
}

fn parse_line(line: &str) -> Option<Signature> {
    let (kind, data) = line.split_once('\t')?;
    match kind {
        "typed" if !data.trim().is_empty() => Some(Signature::Typed(data.to_owned())),
        "drawn" => {
            let strokes: Vec<Vec<(f32, f32)>> = data
                .split('|')
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
                .collect();
            (!strokes.is_empty()).then_some(Signature::Drawn(strokes))
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
        let drawn = Signature::Drawn(vec![vec![(0.1, 0.2), (0.5, 0.25)], vec![(0.9, 0.8)]]);
        let typed = Signature::Typed("Ada Lovelace".into());
        let text = format!("{}\n{}\n", format_line(&drawn), format_line(&typed));
        assert_eq!(parse(&text), vec![drawn, typed]);
    }

    #[test]
    fn ignores_broken_lines() {
        assert_eq!(parse("drawn\t\nunknown\tx\ntyped\t \nno tab"), vec![]);
    }
}
