//! Document model for Recto: original bytes plus an operation log.
//! No UI and no rendering here. See docs/ARCHITECTURE.md.

pub mod annot;
pub mod build;
pub mod content;
pub mod document;
pub mod form;
pub mod geom;
pub mod layout;
pub mod op;
pub mod redact;
pub mod save;
pub mod split;
pub mod text;

pub use build::{BuildError, BuildOptions};
pub use document::{Document, OpenError, Source};

/// Stable page identity: survives reordering, so selections and caches stay valid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PageId {
    /// Index of the source document (0 = the opened file, others = merged files).
    pub source: u32,
    /// Page index inside that source, as it was when the source was opened.
    pub index: u32,
}
