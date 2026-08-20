//! Public values returned by the Rd building layer.

use std::path::PathBuf;

use rd_ast::RdDocument;

use crate::model::TopicKey;

/// The result of building all requested Rd topics.
#[derive(Debug, Clone)]
pub struct RdBuildOutput {
    /// Successfully built topics, keyed by their model identity.
    pub files: std::collections::BTreeMap<TopicKey, GeneratedRd>,
    /// Diagnostics from topics that could not be emitted or from recoverable
    /// input problems encountered while building them.
    pub diagnostics: crate::diagnostic::Diagnostics,
}

/// One complete generated Rd document and its serialized content.
#[derive(Debug, Clone)]
pub struct GeneratedRd {
    /// Package-relative output path.
    pub relative_path: PathBuf,
    /// The writer-valid document tree.
    pub document: RdDocument,
    /// Serialized Rd source.
    pub content: String,
}
