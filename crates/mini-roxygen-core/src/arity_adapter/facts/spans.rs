//! Converts parser ranges into adapter spans.
//!
//! These helpers are shared by name, function, and top-level facts so source provenance has one implementation and cannot drift between responsibilities.

use arity_parser::ast::Expr;
use arity_parser::syntax::{SyntaxElement, SyntaxNode, SyntaxToken};

use crate::source::{FileId, Span, TextRange};
pub(super) fn span_for_expression(expression: &Expr, file_id: FileId) -> Span {
    let range = expression.text_range();
    span_for_offsets(range.start().into(), range.end().into(), file_id)
}

pub(super) fn span_for_element(element: &SyntaxElement, file_id: FileId) -> Span {
    let range = element.text_range();
    span_for_offsets(range.start().into(), range.end().into(), file_id)
}

pub(super) fn span_for_node(node: &SyntaxNode, file_id: FileId) -> Span {
    let range = node.text_range();
    span_for_offsets(range.start().into(), range.end().into(), file_id)
}

pub(super) fn span_for_token(token: &SyntaxToken, file_id: FileId) -> Span {
    let range = token.text_range();
    span_for_offsets(range.start().into(), range.end().into(), file_id)
}

pub(super) fn span_for_offsets(start: u32, end: u32, file_id: FileId) -> Span {
    Span::new(file_id, TextRange::new(start, end))
}
