//! Text recognition on page images with `ocrs`, for the invisible text layer of scanned pages.
//! No UI and no PDF here: the app renders pages, this crate finds words in the pixels.
//!
//! The models are not part of the repository (decision 0004): `cargo xtask fetch-ocr-models`
//! puts them in `target/ocr-models/`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem};

pub const DETECTION_MODEL: &str = "text-detection.rten";
pub const RECOGNITION_MODEL: &str = "text-recognition.rten";

/// A recognised word and its box in image pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

pub struct Engine {
    engine: OcrEngine,
}

/// Where the models are looked for: `RECTO_OCR_MODELS`, then `ocr-models` next to the
/// executable, then `extra` (the app's config folder), then, in debug builds, the workspace's
/// `target/ocr-models`.
pub fn model_dirs(extra: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("RECTO_OCR_MODELS") {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join("ocr-models"))) {
        dirs.push(dir);
    }
    if let Some(extra) = extra {
        dirs.push(extra.to_owned());
    }
    if cfg!(debug_assertions) {
        dirs.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/ocr-models"));
    }
    dirs
}

/// The first of `dirs` holding both models.
pub fn find_models(dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter().find(|dir| dir.join(DETECTION_MODEL).is_file() && dir.join(RECOGNITION_MODEL).is_file()).cloned()
}

impl Engine {
    /// Loads the two models from `dir`.
    pub fn load(dir: &Path) -> Result<Engine> {
        let detection = rten::Model::load_file(dir.join(DETECTION_MODEL)).with_context(|| format!("loading {DETECTION_MODEL}"))?;
        let recognition = rten::Model::load_file(dir.join(RECOGNITION_MODEL)).with_context(|| format!("loading {RECOGNITION_MODEL}"))?;
        let engine = OcrEngine::new(OcrEngineParams { detection_model: Some(detection), recognition_model: Some(recognition), ..Default::default() })?;
        Ok(Engine { engine })
    }

    /// Words in an 8-bit RGB image, in reading order.
    pub fn words(&self, width: u32, height: u32, rgb: &[u8]) -> Result<Vec<Word>> {
        let source = ImageSource::from_bytes(rgb, (width, height))?;
        let input = self.engine.prepare_input(source)?;
        let found = self.engine.detect_words(&input)?;
        let lines = self.engine.find_text_lines(&input, &found);
        let recognised = self.engine.recognize_text(&input, &lines)?;
        let mut words = Vec::new();
        for line in recognised.iter().flatten() {
            for word in line.words() {
                let text = word.to_string();
                // Single stray marks are mostly noise.
                if text.trim().is_empty() || (text.chars().count() == 1 && !text.chars().all(char::is_alphanumeric)) {
                    continue;
                }
                let rect = word.bounding_rect();
                words.push(Word {
                    text,
                    left: rect.left() as f32,
                    top: rect.top() as f32,
                    right: rect.right() as f32,
                    bottom: rect.bottom() as f32,
                });
            }
        }
        Ok(words)
    }
}
