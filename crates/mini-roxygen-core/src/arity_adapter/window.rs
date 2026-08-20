//! Expression windows and collection of parser-owned roxygen lines.

use arity_parser::ast::{AstNode, Expr, RoxygenBlock as ArityRoxygenBlock};
use arity_parser::syntax::SyntaxNode;

use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, Label};
use crate::source::{FileId, SourceFile, Span, TextRange};

use super::lines::{
    line_start, next_physical_line_start, physical_lines, prefix_marker_start,
    roxygen_marker_start, to_u32, to_usize,
};
use super::reduce::{TagHead, reduce_lines, tag_head};
use super::{BlockId, DocLine, RoxyBlock};

/// The source range of one top-level expression, used as the window basis for
/// the following line-and-tag grouping step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TopLevelExpr {
    pub(super) expression: Expr,
    pub(super) range: TextRange,
}

/// Lists top-level expressions without losing literal expressions represented
/// by CST tokens rather than nodes.
pub(super) fn top_level_expressions(root: &SyntaxNode) -> Vec<TopLevelExpr> {
    root.children_with_tokens()
        .filter_map(Expr::cast)
        .map(|expression| {
            let range = TextRange::new(
                to_u32(expression.text_range().start()),
                to_u32(expression.text_range().end()),
            );
            TopLevelExpr { expression, range }
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ExpressionWindow {
    range: TextRange,
}

pub(super) fn expression_windows(expressions: &[TopLevelExpr]) -> Vec<ExpressionWindow> {
    expressions
        .iter()
        .enumerate()
        .map(|(index, expression)| ExpressionWindow {
            range: TextRange::new(
                if index == 0 {
                    0
                } else {
                    expressions[index - 1].range.end()
                },
                expression.range.end(),
            ),
        })
        .collect()
}

#[derive(Debug)]
pub(super) struct DroppedLine {
    span: Span,
    physical_start: u32,
}

#[derive(Debug)]
struct WindowLines {
    doc_lines: Vec<DocLine>,
    tag_heads: Vec<TagHead>,
}

pub(super) fn collect_window_lines(
    source_file: &SourceFile,
    file_id: FileId,
    root: &SyntaxNode,
    windows: &[ExpressionWindow],
) -> (Vec<Option<RoxyBlock>>, Vec<DroppedLine>) {
    let mut arity_blocks = root
        .descendants()
        .filter_map(ArityRoxygenBlock::cast)
        .collect::<Vec<_>>();
    arity_blocks.sort_by_key(|block| block.syntax().text_range().start());

    let mut grouped = windows
        .iter()
        .map(|_| WindowLines {
            doc_lines: Vec::new(),
            tag_heads: Vec::new(),
        })
        .collect::<Vec<_>>();
    let mut dropped = Vec::new();

    for block in arity_blocks {
        let block_range = block.syntax().text_range();
        let block_start = to_u32(block_range.start());
        let doc_lines =
            physical_lines(source_file, file_id, block_start, to_u32(block_range.end()));
        let tag_heads = block
            .tags()
            .filter_map(|tag| tag_head(file_id, tag))
            .collect::<Vec<_>>();

        for (line_index, line) in doc_lines.into_iter().enumerate() {
            let physical_start = line_start(source_file.text(), to_usize(line.span.range.start()));
            let marker = if line_index == 0 {
                // Search from the block the parser found, not from the start
                // of the physical line: code before the block can contain a
                // `#'` of its own -- inside a string literal, say -- and
                // taking that for the marker rejects the real documentation.
                roxygen_marker_start(
                    source_file.text(),
                    to_usize(block_start),
                    to_usize(line.span.range.end()),
                )
                .map(to_u32)
                .unwrap_or(block_start)
            } else {
                prefix_marker_start(
                    source_file.text(),
                    physical_start,
                    to_usize(line.span.range.end()),
                )
                .map(to_u32)
                .unwrap_or(line.span.range.start())
            };
            let window_index = windows
                .iter()
                .position(|window| window.range.start() <= marker && marker < window.range.end());
            let eligible = window_index.is_some_and(|index| {
                eligible_marker_line(
                    source_file.text(),
                    windows[index].range.start(),
                    physical_start,
                    marker,
                )
            });

            let adjusted_start = window_index
                .map(|index| windows[index].range.start())
                .unwrap_or(if line_index == 0 {
                    marker
                } else {
                    to_u32(physical_start)
                })
                .max(line.span.range.start());
            let line = DocLine {
                span: Span::new(
                    file_id,
                    TextRange::new(adjusted_start, line.span.range.end()),
                ),
                content_span: line.content_span,
            };
            let Some(index) = window_index.filter(|_| eligible) else {
                dropped.push(DroppedLine {
                    span: Span::new(file_id, TextRange::new(marker, line.span.range.end())),
                    physical_start: to_u32(physical_start),
                });
                continue;
            };

            grouped[index].doc_lines.push(line);
            grouped[index].tag_heads.extend(
                tag_heads
                    .iter()
                    .filter(|head| {
                        head.at.start() >= marker && head.at.start() < line.span.range.end()
                    })
                    .cloned(),
            );
        }
    }

    let blocks = grouped
        .into_iter()
        .scan(0u32, |block_id, window| {
            let block = (!window.doc_lines.is_empty()).then(|| {
                let span = Span::new(
                    file_id,
                    TextRange::new(
                        window.doc_lines[0].span.range.start(),
                        window
                            .doc_lines
                            .last()
                            .expect("non-empty window")
                            .span
                            .range
                            .end(),
                    ),
                );
                let (intro, tags) =
                    reduce_lines(source_file, file_id, &window.doc_lines, &window.tag_heads);
                let id = BlockId::new(*block_id);
                *block_id = block_id
                    .checked_add(1)
                    .expect("a file cannot contain 2^32 blocks");
                RoxyBlock {
                    id,
                    span,
                    doc_lines: window.doc_lines,
                    intro,
                    tags,
                }
            });
            Some(block)
        })
        .collect();

    (blocks, dropped)
}

fn eligible_marker_line(text: &str, window_start: u32, physical_start: usize, marker: u32) -> bool {
    let first_window_line = line_start(text, to_usize(window_start)) == physical_start;
    let prefix_start = if first_window_line {
        to_usize(window_start)
    } else {
        physical_start
    };
    text.get(prefix_start..to_usize(marker))
        .is_some_and(|prefix| prefix.bytes().all(|byte| matches!(byte, b' ' | b'\t')))
}

pub(super) fn push_unattached_diagnostics(
    source_file: &SourceFile,
    diagnostics: &mut Diagnostics,
    mut dropped_lines: Vec<DroppedLine>,
) {
    dropped_lines.sort_by_key(|line| line.physical_start);
    let mut groups: Vec<DroppedLine> = Vec::new();
    for line in dropped_lines {
        let contiguous = groups.last().is_some_and(|previous| {
            next_physical_line_start(source_file.text(), previous.physical_start)
                == Some(line.physical_start)
        });
        if contiguous {
            let previous = groups.last_mut().expect("contiguous group exists");
            previous.span.range =
                TextRange::new(previous.span.range.start(), line.span.range.end());
        } else {
            groups.push(line);
        }
    }

    for group in groups {
        diagnostics.push(
            Diagnostic::new(
                DiagnosticCode::UnattachedRoxygenBlock.default_severity(),
                DiagnosticCode::UnattachedRoxygenBlock,
                "roxygen documentation is not attached to any top-level expression and will be ignored",
                Label::new(group.span, "unattached roxygen block"),
            )
            .with_help(
                "Move this block before a following top-level expression, or remove it.",
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::inventory_texts;

    #[test]
    fn expression_inventory_includes_a_compound_expression() {
        let text = r#"x <- function() {
  NULL
}
"#;
        assert_eq!(
            inventory_texts(text),
            vec![
                r#"x <- function() {
  NULL
}"#
            ]
        );
    }

    #[test]
    fn expression_inventory_includes_bare_literals() {
        assert_eq!(
            inventory_texts(
                r#"NULL
"#
            ),
            vec!["NULL"]
        );
        assert_eq!(
            inventory_texts(
                r#""value"
"#
            ),
            vec!["\"value\""]
        );
    }

    #[test]
    fn expression_inventory_keeps_several_expressions_in_source_order() {
        assert_eq!(
            inventory_texts(
                r#"x <- 1
NULL
"value"
"#
            ),
            vec!["x <- 1", "NULL", "\"value\""]
        );
    }

    #[test]
    fn expression_inventory_splits_semicolon_separated_expressions() {
        assert_eq!(
            inventory_texts("x <- 1; y; \"value\""),
            vec!["x <- 1", "y", "\"value\""]
        );
    }

    #[test]
    fn expression_inventory_ignores_leading_and_trailing_trivia() {
        assert_eq!(
            inventory_texts(
                r#"
# leading comment
  NULL
# trailing comment
"#
            ),
            vec!["NULL"]
        );
    }
}
