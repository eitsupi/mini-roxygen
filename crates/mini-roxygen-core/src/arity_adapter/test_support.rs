//! Shared fixtures for adapter unit tests.

use arity_parser::parser::{ParseOptions, parse_with_options};

use crate::arity_adapter::facts::FunctionFact;
use crate::arity_adapter::{AssignmentValue, ParsedFile, TopLevelFact, TopLevelShape, parse};
use crate::source::{FileId, SourceFile};

use super::window::top_level_expressions;

/// Returns the source text of each top-level expression, in source order.
pub(super) fn inventory_texts(text: &str) -> Vec<&str> {
    let parsed = parse_with_options(text, &ParseOptions::default());
    top_level_expressions(&parsed.cst)
        .iter()
        .map(|expression| &text[expression.range.start() as usize..expression.range.end() as usize])
        .collect()
}

pub(super) fn parsed(text: &str) -> (ParsedFile, SourceFile) {
    let source = SourceFile::new(std::path::PathBuf::from("test.R"), text.to_owned());
    let parsed = parse(&source, FileId::new(3));
    (parsed, source)
}

pub(super) fn assignment(parsed: &ParsedFile, index: usize) -> &TopLevelFact {
    &parsed.top_level[index].fact
}

pub(super) fn function(parsed: &ParsedFile) -> &FunctionFact {
    let TopLevelShape::Assignment(fact) = &parsed.top_level[0].fact.shape else {
        panic!("expected assignment");
    };
    let AssignmentValue::Function(function) = &fact.value else {
        panic!("expected function facts");
    };
    function
}

pub(super) fn slice(source: &SourceFile, span: crate::source::Span) -> &str {
    source
        .text_range(span.range)
        .expect("span must select a UTF-8 source range")
}

pub(super) fn value_variant(value: &AssignmentValue) -> &'static str {
    match value {
        AssignmentValue::Function(_) => "function",
        AssignmentValue::Name(_) => "name",
        AssignmentValue::Call(_) => "call",
        AssignmentValue::Literal => "literal",
        AssignmentValue::Other => "other",
    }
}
