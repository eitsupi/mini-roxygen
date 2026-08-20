//! Converts arity-parser's R CST and roxygen sections into mini-roxygen IR.
//!
//! This module is the only place where the parser's CST, rowan ranges, and
//! syntax kinds are consumed. Its public types deliberately contain only
//! mini-roxygen source spans and owned values, so parser implementation details
//! cannot become a dependency of later layers. Markdown meaning is not decided
//! here; this layer retains raw tag text and provenance for the tag layer.

use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, Label};
use crate::source::{FileId, SourceFile, Span};
use arity_parser::parser::{ParseOptions, parse_with_options};

mod authors;
mod facts;
mod ir;
mod lines;
mod r_code;
mod raw_string;
mod reduce;
mod window;

#[cfg(test)]
mod test_support;

pub use authors::{AuthorsParseError, PersonSection, parse_authors};
pub use facts::{
    AssignmentFact, AssignmentOperator, AssignmentTarget, AssignmentValue, BindingName,
    CallArgument, CallArgumentValue, CallCallee, CallFact, Formal, FormalError, RName,
    RNameDecodeError, S7ClassAnalysis, S7ClassFact, S7ClassRefusal, S7ClassRefusalReason,
    TopLevelFact, TopLevelShape,
};
pub use ir::{BlockId, DocLine, ParsedFile, ParsedTopLevel, RawBody, RawTag, RoxyBlock};
use lines::diagnostic_range;
pub(crate) use r_code::{
    RCodeMarker, RCodeMarkerKind, can_parse_namespace_source, can_parse_r, r_code_chunks,
    r_code_markers,
};
use window::{
    collect_window_lines, expression_windows, push_unattached_diagnostics, top_level_expressions,
};

/// Parses one source file and extracts roxygen blocks and raw tags.
///
/// Diagnostics are returned alongside successfully extracted blocks through
/// the shared [`Diagnostics`] accumulator. Syntax diagnostics fail extraction
/// closed; recoverable unsupported directives and unattached lines are reported
/// alongside the blocks that remain valid.
///
/// Markdown is enabled by default for every block because mini-roxygen treats
/// Markdown as an always-on input mode. The parser option is therefore built
/// with `ParseOptions::default().with_roxygen_markdown_default(true)`; the
/// non-exhaustive options struct must not be constructed by field literal.
///
/// Roxygen lines are grouped by the source window of each top-level expression,
/// including lines represented by nested `ROXYGEN_BLOCK` nodes. A window is
/// the source from the preceding expression's end through its own end, with
/// the first window beginning at byte zero.
#[must_use]
pub fn parse(source_file: &SourceFile, file_id: FileId) -> ParsedFile {
    let parsed = parse_with_options(
        source_file.text(),
        &ParseOptions::default().with_roxygen_markdown_default(true),
    );
    let mut diagnostics = Diagnostics::new();

    for diagnostic in parsed.diagnostics {
        let range = diagnostic_range(source_file.text().len(), diagnostic.start, diagnostic.end);
        let span = Span::new(file_id, range);
        diagnostics.push(Diagnostic::new(
            DiagnosticCode::RSyntaxError.default_severity(),
            DiagnosticCode::RSyntaxError,
            diagnostic.message,
            Label::new(span, "R syntax error"),
        ));
    }

    // Recovery CSTs can place roxygen nodes under the wrong expression. R's
    // parse() fails closed in this situation, so do not derive associations
    // from the recovered tree.
    if !diagnostics.is_empty() {
        return ParsedFile {
            top_level: Vec::new(),
            calls: Vec::new(),
            diagnostics,
        };
    }

    let expressions = top_level_expressions(&parsed.cst);
    let facts = facts::top_level_facts(&expressions, file_id);
    let calls = facts::nested_call_facts(&expressions, file_id);
    let windows = expression_windows(&expressions);
    let (documentation, dropped_lines) =
        collect_window_lines(source_file, file_id, &parsed.cst, &windows);
    push_unattached_diagnostics(source_file, &mut diagnostics, dropped_lines);

    // `@md` is accepted as a redundant marker. `@noMd` is diagnosed below
    // because the parser's default mode does not override a block directive,
    // and allowing that directive would change the CST structure that later
    // layers consume. mini-roxygen's contract is Markdown always on.
    for block in documentation.iter().flatten() {
        for tag in &block.tags {
            if tag.name.value == "noMd" {
                diagnostics.push(
                    Diagnostic::new(
                        DiagnosticCode::UnsupportedTag.default_severity(),
                        DiagnosticCode::UnsupportedTag,
                        "mini-roxygen always treats Markdown as enabled and does not support @noMd",
                        Label::new(
                            tag.full_span,
                            "unsupported Markdown mode directive",
                        ),
                    )
                    .with_help(
                        "Remove @noMd; Markdown is always enabled by mini-roxygen. Use @md only when documenting the mode explicitly.",
                    )
                    .with_context("tag", "noMd"),
                );
            }
        }
    }

    let top_level = facts
        .into_iter()
        .zip(documentation)
        .map(|(fact, documentation)| ParsedTopLevel {
            fact,
            documentation,
        })
        .collect();

    ParsedFile {
        top_level,
        calls,
        diagnostics,
    }
}

#[cfg(test)]
mod tests;
