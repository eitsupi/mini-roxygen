//! Unsupported Markdown construct naming and consolidated recovery policy.

use pulldown_cmark::{HeadingLevel, Tag};

use crate::diagnostic::{Diagnostic, DiagnosticCode, Label, Severity};

use super::frame::{Frame, FrameKind};

/// Recovery policy for Markdown constructs not lowered by this conversion
/// step: diagnose the construct, flatten recoverable descendants, and retain
/// their literal text. Block quotes and level-1 headings flatten their
/// contents; images retain alt text without emitting `\\figure`; raw HTML remains
/// literal text rather than Rd or HTML; rules are diagnosed and have no text;
/// malformed tables use the same recovery path defensively; links are lowered
/// by the link module; footnotes, strikethrough, and other
/// disabled extensions are diagnosed if parser options later expose events.
pub(super) fn unsupported_tag_name(tag: &Tag<'_>) -> String {
    match tag {
        Tag::CodeBlock(_) => "code block".to_owned(),
        Tag::List(_) => "list".to_owned(),
        Tag::Item => "list item".to_owned(),
        Tag::Link { .. } => "link".to_owned(),
        Tag::Image { .. } => "image".to_owned(),
        Tag::BlockQuote(_) => "block quote".to_owned(),
        Tag::Heading { level, .. } if *level == HeadingLevel::H1 => "level-one heading".to_owned(),
        Tag::Heading { .. } => "heading".to_owned(),
        Tag::Table(_) | Tag::TableHead | Tag::TableRow | Tag::TableCell => "table".to_owned(),
        Tag::HtmlBlock => "HTML block".to_owned(),
        Tag::FootnoteDefinition(_) => "footnote definition".to_owned(),
        Tag::Strikethrough => "strikethrough".to_owned(),
        Tag::Superscript => "superscript".to_owned(),
        Tag::Subscript => "subscript".to_owned(),
        Tag::DefinitionList => "definition list".to_owned(),
        Tag::DefinitionListTitle => "definition list title".to_owned(),
        Tag::DefinitionListDefinition => "definition list definition".to_owned(),
        Tag::MetadataBlock(_) => "metadata block".to_owned(),
        _ => "Markdown construct".to_owned(),
    }
}

pub(super) fn is_nested_table_envelope(frames: &[Frame], name: &str) -> bool {
    name == "table"
        && frames.last().is_some_and(|frame| {
            matches!(
                &frame.kind,
                FrameKind::Unsupported { name, .. } if name == "table"
            )
        })
}

pub(super) fn is_redundant_unsupported_envelope(frames: &[Frame], name: &str) -> bool {
    is_nested_table_envelope(frames, name)
        || (matches!(name, "heading" | "level-one heading")
            && frames
                .iter()
                .any(|frame| matches!(&frame.kind, FrameKind::Unsupported { .. })))
}

pub(super) fn diagnose_range(
    converter: &mut super::Converter<'_>,
    name: &str,
    start: usize,
    end: usize,
) {
    if name == "level-one heading" {
        diagnose_level_one_heading(converter, start, end);
        return;
    }
    diagnose_code_range(
        converter,
        DiagnosticCode::UnsupportedMarkdownConstruct,
        format!("unsupported Markdown construct: {name}"),
        "unsupported Markdown construct",
        start,
        end,
    );
}

fn diagnose_level_one_heading(converter: &mut super::Converter<'_>, start: usize, end: usize) {
    let spans = converter.spans(start, end);
    let Some(primary) = spans.first().copied() else {
        return;
    };
    let secondary = spans[1..]
        .iter()
        .copied()
        .map(|span| Label::new(span, "part of this Markdown heading"));
    converter.diagnostics.push(
        Diagnostic::new(
            DiagnosticCode::UnsupportedMarkdownHeading.default_severity(),
            DiagnosticCode::UnsupportedMarkdownHeading,
            "level-1 Markdown headings are flattened into prose",
            Label::new(primary, "unsupported level-1 Markdown heading"),
        )
        .with_secondaries(secondary)
        .with_help("use an explicit @section Title: contribution to preserve section structure"),
    );
}

pub(super) fn diagnose_code_range(
    converter: &mut super::Converter<'_>,
    code: DiagnosticCode,
    message: impl Into<String>,
    label: &str,
    start: usize,
    end: usize,
) {
    let spans = converter.spans(start, end);
    let Some(primary) = spans.first().copied() else {
        return;
    };
    let secondary = spans[1..]
        .iter()
        .copied()
        .map(|span| Label::new(span, "part of this Markdown construct"));
    converter.diagnostics.push(
        Diagnostic::new(
            code.default_severity(),
            code,
            message,
            Label::new(primary, label),
        )
        .with_secondaries(secondary),
    );
}

pub(super) fn diagnose_undefined_inline_r(
    converter: &mut super::Converter<'_>,
    start: usize,
    end: usize,
) {
    let spans = converter.spans(start, end);
    let Some(primary) = spans.first().copied() else {
        return;
    };
    converter.diagnostics.push(
        Diagnostic::new(
            Severity::Error,
            DiagnosticCode::UndefinedInlineRSubstitution,
            "no substitution is defined for inline R expression",
            Label::new(primary, "inline R expression has no substitution"),
        )
        .with_help("provide a substitution for this exact inline R expression"),
    );
}

pub(super) fn diagnose_multiline_inline_r(
    converter: &mut super::Converter<'_>,
    start: usize,
    end: usize,
) {
    diagnose_code_range(
        converter,
        DiagnosticCode::UnsupportedInlineR,
        "multi-line inline R code is not supported",
        "unsupported inline R code",
        start,
        end,
    );
}

#[cfg(test)]
mod tests {
    use super::super::convert_markdown as convert_markdown_with_context;
    use super::super::frame::{Frame, FrameKind};
    use super::super::test_support::{context, value};
    use super::is_nested_table_envelope;
    use crate::tags::MarkdownText;

    fn convert_markdown(value: &MarkdownText) -> super::super::MarkdownConversion {
        convert_markdown_with_context(value, &context())
    }

    #[test]
    fn ordinary_link_is_lowered_without_an_unsupported_diagnostic() {
        let conversion = convert_markdown(&value("before [code](url) after"));
        assert_eq!(
            conversion.fragment.nodes,
            vec![
                rd_ast::RdNode::Text("before ".into()),
                rd_ast::RdNode::tagged(
                    rd_ast::RdTag::Href,
                    None,
                    vec![
                        rd_ast::RdNode::group(vec![rd_ast::RdNode::Verb("url".into())]),
                        rd_ast::RdNode::group(vec![rd_ast::RdNode::Text("code".into())]),
                    ],
                ),
                rd_ast::RdNode::Text(" after".into()),
            ]
        );
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn unsupported_recovery_block_quote_uses_spaces_and_lines() {
        let conversion = convert_markdown(&value("> A\n> B"));
        assert_eq!(
            conversion.fragment.nodes,
            vec![
                rd_ast::RdNode::Text("A\n".into()),
                rd_ast::RdNode::Text("B".into()),
            ]
        );
    }

    #[test]
    fn unsupported_recovery_blocks_separate_from_following_prose() {
        // A block's own trailing boundary is discarded when its frame closes,
        // so the block tag itself has to contribute the separator. Without
        // that, the block runs straight into the paragraph after it.
        let markdown = "> quoted\n\nprose";
        let conversion = convert_markdown(&value(markdown));
        assert_eq!(
            conversion.fragment.nodes,
            vec![
                rd_ast::RdNode::Text("quoted\n".into()),
                rd_ast::RdNode::Text("\n".into()),
                rd_ast::RdNode::Text("prose".into()),
            ],
            "recovering {markdown:?}"
        );
    }

    #[test]
    fn unsupported_recovery_nested_heading_uses_a_paragraph_boundary() {
        let conversion = convert_markdown(&value("> # Heading\n\nprose"));
        assert_eq!(
            conversion.fragment.nodes,
            vec![
                rd_ast::RdNode::Text("Heading\n".into()),
                rd_ast::RdNode::Text("\n".into()),
                rd_ast::RdNode::Text("prose".into()),
            ]
        );
    }

    #[test]
    fn unsupported_constructs_diagnose_and_keep_recoverable_text() {
        for (markdown, expected) in [
            ("> quoted", vec![rd_ast::RdNode::Text("quoted".into())]),
            (
                "![alt text](image.png)",
                vec![rd_ast::RdNode::Text("alt text".into())],
            ),
            (
                "<span>raw HTML</span>",
                vec![rd_ast::RdNode::Text("<span>raw HTML</span>".into())],
            ),
            (
                "<div>\nraw HTML\n</div>",
                vec![
                    rd_ast::RdNode::Text("<div>\n".into()),
                    rd_ast::RdNode::Text("raw HTML\n".into()),
                    rd_ast::RdNode::Text("</div>".into()),
                ],
            ),
        ] {
            let conversion = convert_markdown(&value(markdown));
            assert_eq!(
                conversion.fragment.nodes, expected,
                "recovering {markdown:?}"
            );
            assert!(
                conversion
                    .diagnostics
                    .iter()
                    .all(|diagnostic| diagnostic.code
                        == crate::diagnostic::DiagnosticCode::UnsupportedMarkdownConstruct),
                "diagnosing {markdown:?}"
            );
            assert!(
                !conversion.diagnostics.is_empty(),
                "missing diagnostic for {markdown:?}"
            );
        }

        let rule = convert_markdown(&value("---"));
        assert!(rule.fragment.nodes.is_empty());
        assert_eq!(rule.diagnostics.len(), 1);
        assert_eq!(
            rule.diagnostics
                .iter()
                .next()
                .expect("rule diagnostic")
                .code,
            crate::diagnostic::DiagnosticCode::UnsupportedMarkdownConstruct
        );
    }

    #[test]
    fn unsupported_block_quote_reports_once_for_the_envelope() {
        let conversion = convert_markdown(&value("> A\n> B"));
        assert_eq!(conversion.diagnostics.len(), 1);
    }

    #[test]
    fn nested_table_envelope_suppression_remains_table_specific() {
        let frames = vec![Frame::new(FrameKind::Unsupported {
            name: "table".to_owned(),
            start: 0,
        })];
        assert!(is_nested_table_envelope(&frames, "table"));
        assert!(!is_nested_table_envelope(&frames, "image"));
    }
}
