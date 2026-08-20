//! Reduction of physical roxygen lines into raw adapter IR.

use arity_parser::ast::{AstNode, RoxygenTag as ArityRoxygenTag};

use crate::source::{FileId, SourceFile, Span, Spanned, TextRange};

use super::lines::{marker_start, to_u32};
use super::{DocLine, RawBody, RawTag};

/// A tag head projected from arity's roxygen lexer. The reducer deliberately
/// uses only this line-level information, not arity's section structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TagHead {
    pub(super) at: TextRange,
    pub(super) name: Spanned<String>,
}

pub(super) fn tag_head(file_id: FileId, tag: ArityRoxygenTag) -> Option<TagHead> {
    let tag_name = tag.name()?;
    let name_range = tag.syntax().children_with_tokens().find_map(|element| {
        let token = element.into_token()?;
        (token.text() == tag_name.as_str()).then_some(token.text_range())
    })?;
    let at_range = tag.at()?.text_range();
    Some(TagHead {
        at: TextRange::new(to_u32(at_range.start()), to_u32(at_range.end())),
        name: Spanned::new(
            tag_name.to_string(),
            Span::new(
                file_id,
                TextRange::new(to_u32(name_range.start()), to_u32(name_range.end())),
            ),
        ),
    })
}

/// Rebuilds the current adapter IR from physical lines and arity tag heads.
///
/// A section's end is the next tag line's start, or the final line's end. This
/// is the same source window represented by arity's current section nodes, but
/// the reducer keeps only the line-level provenance consumed by this adapter.
pub(super) fn reduce_lines(
    source_file: &SourceFile,
    file_id: FileId,
    doc_lines: &[DocLine],
    tag_heads: &[TagHead],
) -> (Option<RawBody>, Vec<RawTag>) {
    let mut head_lines = Vec::with_capacity(tag_heads.len());
    let mut line_index = 0;
    for head in tag_heads {
        while line_index < doc_lines.len()
            && !doc_lines[line_index].span.range.contains(head.at.start())
        {
            line_index += 1;
        }
        if line_index < doc_lines.len() {
            head_lines.push((line_index, head));
        }
    }

    let first_tag_line = head_lines
        .first()
        .map_or(doc_lines.len(), |(index, _)| *index);
    let intro = (first_tag_line > 0).then(|| {
        let value_lines = doc_lines[..first_tag_line]
            .iter()
            .map(|line| line.content_span)
            .collect::<Vec<_>>();
        let start = marker_start(
            source_file.text(),
            doc_lines[0].span.range.start(),
            doc_lines[0].span.range.end(),
        );
        let end = doc_lines.get(first_tag_line).map_or_else(
            || doc_lines[first_tag_line - 1].span.range.end(),
            |line| {
                marker_start(
                    source_file.text(),
                    line.span.range.start(),
                    line.span.range.end(),
                )
            },
        );
        RawBody {
            raw_value: raw_value_from_lines(source_file, &value_lines),
            value_lines,
            full_span: Span::new(file_id, TextRange::new(start, end)),
        }
    });

    let tags = head_lines
        .iter()
        .enumerate()
        .map(|(head_index, (line_index, head))| {
            let next_line = head_lines
                .get(head_index + 1)
                .map_or(doc_lines.len(), |(index, _)| *index);
            let value_end = doc_lines.get(next_line).map_or_else(
                || doc_lines[next_line - 1].span.range.end(),
                |line| {
                    marker_start(
                        source_file.text(),
                        line.span.range.start(),
                        line.span.range.end(),
                    )
                },
            );
            let first_line = &doc_lines[*line_index];
            let value_lines = std::iter::once(Span::new(
                file_id,
                TextRange::new(head.name.span.range.end(), first_line.span.range.end()),
            ))
            .chain(
                doc_lines[*line_index + 1..next_line]
                    .iter()
                    .map(|line| line.content_span),
            )
            .collect::<Vec<_>>();
            RawTag {
                name: head.name.clone(),
                raw_value: raw_value_from_lines(source_file, &value_lines),
                value_lines,
                value_span: Span::new(
                    file_id,
                    TextRange::new(head.name.span.range.end(), value_end),
                ),
                full_span: Span::new(file_id, TextRange::new(head.at.start(), value_end)),
            }
        })
        .collect();

    (intro, tags)
}

fn raw_value_from_lines(source_file: &SourceFile, value_lines: &[Span]) -> String {
    value_lines
        .iter()
        .map(|span| source_file.text_range(span.range).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::lines::physical_lines;
    use super::{reduce_lines, tag_head};
    use crate::arity_adapter::RawTag;
    use crate::source::{FileId, SourceFile};
    use arity_parser::ast::{AstNode, RoxygenBlock as ArityRoxygenBlock};
    use arity_parser::parser::{ParseOptions, parse_with_options};

    fn reduced(text: &str) -> (Option<super::RawBody>, Vec<RawTag>, SourceFile) {
        let source = SourceFile::new(PathBuf::from("test.R"), text.to_owned());
        let parsed = parse_with_options(
            text,
            &ParseOptions::default().with_roxygen_markdown_default(true),
        );
        let block = parsed
            .cst
            .children()
            .find_map(ArityRoxygenBlock::cast)
            .expect("test input must contain a roxygen block");
        let range = block.syntax().text_range();
        let lines = physical_lines(
            &source,
            FileId::new(7),
            range.start().into(),
            range.end().into(),
        );
        let heads = block
            .tags()
            .filter_map(|tag| tag_head(FileId::new(7), tag))
            .collect::<Vec<_>>();
        let (intro, tags) = reduce_lines(&source, FileId::new(7), &lines, &heads);
        (intro, tags, source)
    }

    #[test]
    fn reducer_keeps_each_continuation_line_span() {
        let (_, tags, source) = reduced(
            r#"#' @details first
#' second
#' third
#' @export
"#,
        );
        assert_eq!(tags[0].raw_value, " first\nsecond\nthird");
        assert_eq!(
            tags[0]
                .value_lines
                .iter()
                .map(|line| source.text_range(line.range).unwrap())
                .collect::<Vec<_>>(),
            vec![" first", "second", "third"]
        );
    }

    #[test]
    fn reducer_preserves_an_explicit_empty_value_line() {
        let (_, tags, source) = reduced(
            r#"#' @details first
#'
#' third
#' @export
"#,
        );
        assert_eq!(tags[0].raw_value, " first\n\nthird");
        assert!(tags[0].value_lines[1].range.is_empty());
        assert_eq!(source.text_range(tags[0].value_lines[1].range), Some(""));
    }

    #[test]
    fn reducer_uses_arity_heads_for_escaped_and_mid_value_at_signs() {
        let (_, tags, _) = reduced(
            r#"#' @details first @@two
#' continuation @name still
#' @export
"#,
        );
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].name.value, "details");
        assert_eq!(tags[0].raw_value, " first @@two\ncontinuation @name still");
        assert_eq!(tags[1].name.value, "export");
    }

    #[test]
    fn reducer_handles_tag_only_and_intro_only_blocks() {
        let (intro, tags, _) = reduced(
            r#"#' @export
"#,
        );
        assert!(intro.is_none());
        assert_eq!(tags.len(), 1);

        let (intro, tags, _) = reduced(
            r#"#' first
#' second
"#,
        );
        assert!(tags.is_empty());
        assert_eq!(intro.unwrap().raw_value, "first\nsecond");
    }

    #[test]
    fn reducer_preserves_crlf_utf8_and_hash_marker_boundaries() {
        let (intro, tags, source) =
            reduced("  ##' Intro 日本語\r\n  ##' @details 値 😀\r\n  ##' 続き\r\n");
        assert_eq!(intro.unwrap().raw_value, "Intro 日本語");
        assert_eq!(tags[0].raw_value, " 値 😀\n続き");
        assert_eq!(
            source.text_range(tags[0].value_lines[0].range),
            Some(" 値 😀")
        );
        assert_eq!(
            source.text_range(tags[0].value_lines[1].range),
            Some("続き")
        );
        assert!(source.text_range(tags[0].value_span.range).is_some());
        assert!(source.text_range(tags[0].full_span.range).is_some());
    }
}
