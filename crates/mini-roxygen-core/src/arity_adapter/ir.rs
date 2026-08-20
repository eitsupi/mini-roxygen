//! Public intermediate representation for parsed roxygen source.
//!
//! These owned, source-backed types are kept apart from parser wiring so downstream layers depend on stable adapter data rather than arity CST types.

use crate::diagnostic::Diagnostics;
use crate::source::{Span, Spanned};

use super::TopLevelFact;

/// Identifies one roxygen block within a source file.
///
/// The numeric representation is private so callers cannot accidentally use a
/// raw integer where a block identity is required.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(u32);

impl BlockId {
    /// Creates a block identifier from its file-local ordinal.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// Returns the file-local ordinal represented by this identifier.
    #[must_use]
    #[cfg(test)]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// One physical line in a roxygen block.
///
/// Both spans are retained instead of storing a copied string. This preserves
/// the exact correspondence between normalized content offsets and original
/// source byte offsets, including indentation, CRLF input, and non-ASCII text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocLine {
    /// The line without its line terminator, including indentation and marker.
    pub span: Span,
    /// The line content after the roxygen marker prefix has been removed.
    pub content_span: Span,
}

/// A raw tag extracted from one roxygen section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawTag {
    /// The tag name without `@`, with the name-token source span.
    pub name: Spanned<String>,
    /// The untrimmed value reconstructed from the physical content lines.
    pub raw_value: String,
    /// Content spans for each physical line contributing to `raw_value`.
    /// These spans map offsets in the normalized `raw_value` back to the
    /// original source, which is needed for precise later diagnostics.
    pub value_lines: Vec<Span>,
    /// The bounding source span covered by the tag value, beginning after its
    /// name. The span may enclose ordinary source between discontinuous value
    /// lines; use [`RawTag::value_lines`] for precise provenance.
    pub value_span: Span,
    /// The bounding source span from `@` through the end of the tag section.
    /// It may enclose ordinary source between discontinuous roxygen lines.
    pub full_span: Span,
}

/// Untagged body text at the beginning of a roxygen block.
///
/// The body remains raw and source-backed here. No title, description, or
/// details interpretation is applied; that semantic decomposition belongs to
/// a later layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawBody {
    /// The untrimmed value reconstructed from the physical content lines.
    pub raw_value: String,
    /// Content spans for each physical line contributing to `raw_value`.
    /// These spans map offsets in the normalized `raw_value` back to the
    /// original source, which is needed for precise later diagnostics.
    pub value_lines: Vec<Span>,
    /// The bounding source span covered by the intro section. It may enclose
    /// ordinary source between discontinuous roxygen lines; use
    /// [`RawBody::value_lines`] for precise provenance.
    pub full_span: Span,
}

/// One roxygen block and its raw, not-yet-interpreted tags.
///
/// `arity-parser`, rowan, and `SyntaxKind` types are intentionally absent from
/// this public IR. Keeping that parser boundary here lets subsequent layers
/// evolve independently of arity's early API while retaining source
/// provenance in mini-roxygen's stable span types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoxyBlock {
    /// The file-local block identifier.
    pub id: BlockId,
    /// A bounding span from the first accepted roxygen fragment to the last
    /// accepted line, excluding its final line terminator. It is an envelope,
    /// not necessarily a contiguous slice: it may enclose ordinary source and
    /// overlap the expression being documented.
    pub span: Span,
    /// The block's reconstructed physical lines.
    pub doc_lines: Vec<DocLine>,
    /// The leading untagged body, or `None` when the block begins with a tag.
    pub intro: Option<RawBody>,
    /// The block's raw tags in source order.
    pub tags: Vec<RawTag>,
}

/// One top-level expression together with the documentation that belongs to it.
///
/// The adapter keeps this association as a pair because the documentation
/// collector owns the rules for deciding whether a roxygen line belongs to an
/// expression. Keeping the optional block beside its fact preserves that
/// decision at the parser boundary without making a vector index into an
/// identity for an expression.
#[derive(Debug)]
pub struct ParsedTopLevel {
    /// Syntax facts for the expression.
    pub fact: TopLevelFact,
    /// The expression's documentation, when its source window contains an
    /// eligible roxygen line.
    pub documentation: Option<RoxyBlock>,
}

/// The result of parsing one source file.
#[derive(Debug)]
pub struct ParsedFile {
    /// Top-level expressions and their optional documentation, in source
    /// order. The order is a traversal order, not an identity for an
    /// expression; callers should use each entry's fact and documentation.
    pub top_level: Vec<ParsedTopLevel>,
    /// Every call expression in the parsed syntax tree, including nested calls.
    pub calls: Vec<super::CallFact>,
    /// Syntax and adapter diagnostics.
    pub diagnostics: Diagnostics,
}
