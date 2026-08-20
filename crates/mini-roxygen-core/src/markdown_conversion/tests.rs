//! Prose, nesting, provenance, and writer integration tests for conversion.

use std::path::PathBuf;

use rd_ast::{RdDocument, RdTag};
use rd_writer::Writer;

use super::test_support::{assert_serialized_body, context, value};
use super::{FragmentPath, convert_markdown as convert_markdown_with_context};
use crate::rd_oracle::{assert_r_accepts, minimal_topic};
use crate::source::{FileId, SourceFile, Span, TextRange};
use crate::tags::{MarkdownText, NormalizeHead, SourcedText};

fn convert_markdown(value: &MarkdownText) -> super::MarkdownConversion {
    convert_markdown_with_context(value, &context())
}

fn diagnostic_messages(conversion: &super::MarkdownConversion) -> Vec<String> {
    conversion
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.clone())
        .collect()
}

#[test]
fn plain_text_is_one_text_node() {
    let conversion = convert_markdown(&value("plain text"));
    assert_eq!(
        conversion.fragment.nodes,
        vec![rd_ast::RdNode::Text("plain text".into())]
    );
    assert!(conversion.diagnostics.is_empty());
}

#[test]
fn paragraphs_become_writer_canonical_text_leaves() {
    let conversion = convert_markdown(&value("first\n\nsecond"));
    assert_eq!(
        conversion.fragment.nodes,
        vec![
            rd_ast::RdNode::Text("first\n".into()),
            rd_ast::RdNode::Text("\n".into()),
            rd_ast::RdNode::Text("second".into()),
        ]
    );
    assert_eq!(
        super::test_support::serialize(conversion.fragment.nodes),
        "first\n\nsecond"
    );
}

#[test]
fn level_two_heading_with_paragraph_becomes_a_subsection() {
    let conversion = convert_markdown(&value("## Title\n\nBody"));
    assert_serialized_body(conversion.fragment.nodes, "\\subsection{Title}{\n\nBody\n}");
    assert!(conversion.diagnostics.is_empty());
}

#[test]
fn consecutive_headings_close_the_first_subsection_before_the_second() {
    let conversion = convert_markdown(&value("## A\n\none\n\n## B\n\ntwo"));
    assert_serialized_body(
        conversion.fragment.nodes,
        "\\subsection{A}{\n\none\n}\n\n\\subsection{B}{\n\ntwo\n}",
    );
    assert!(conversion.diagnostics.is_empty());
}

#[test]
fn heading_levels_close_nested_subsections_in_stack_order() {
    let conversion = convert_markdown(&value("## A\n\nouter\n\n### B\n\ninner\n\n## C\n\nlast"));
    assert_serialized_body(
        conversion.fragment.nodes,
        "\\subsection{A}{\n\nouter\n\\subsection{B}{\n\ninner\n}\n\n}\n\n\\subsection{C}{\n\nlast\n}",
    );
    assert!(conversion.diagnostics.is_empty());
}

#[test]
fn heading_title_uses_the_existing_link_and_code_conversions() {
    let conversion = convert_markdown(&value("## [list] and `x + 1`\n\nBody"));
    assert_serialized_body(
        conversion.fragment.nodes,
        "\\subsection{\\link{list} and \\code{x + 1}}{\n\nBody\n}",
    );
    assert!(conversion.diagnostics.is_empty());
}

#[test]
fn heading_at_end_of_text_still_has_a_closed_subsection() {
    let conversion = convert_markdown(&value("## Last"));
    assert_serialized_body(conversion.fragment.nodes, "\\subsection{Last}{\n}");
    assert!(conversion.diagnostics.is_empty());
}

#[test]
fn level_one_heading_is_flattened_with_a_warning() {
    let conversion = convert_markdown(&value("# Heading\n\nBody"));
    assert_serialized_body(conversion.fragment.nodes, "Heading\n\nBody");
    let diagnostics = conversion.diagnostics.iter().collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].code,
        crate::diagnostic::DiagnosticCode::UnsupportedMarkdownHeading
    );
    assert_eq!(
        diagnostics[0].severity,
        crate::diagnostic::Severity::Warning
    );
    assert!(!conversion.diagnostics.has_errors());
    assert_eq!(
        diagnostics[0].help.as_deref(),
        Some("use an explicit @section Title: contribution to preserve section structure")
    );
}

#[test]
fn consecutive_level_one_headings_are_flattened_in_source_order() {
    let conversion = convert_markdown(&value("# First\n\nintro\n\n# Second\n\nlast"));
    assert_serialized_body(
        conversion.fragment.nodes,
        "First\n\nintro\n\nSecond\n\nlast",
    );
    assert_eq!(conversion.diagnostics.len(), 2);
}
#[test]
fn level_one_heading_preserves_inline_conversions_during_recovery() {
    let conversion = convert_markdown(&value("# [list] and `x + 1`\n\nBody"));
    assert_serialized_body(
        conversion.fragment.nodes,
        "\\link{list} and \\code{x + 1}\n\nBody",
    );
    assert_eq!(conversion.diagnostics.len(), 1);
}

#[test]
fn empty_level_one_heading_is_flattened_with_a_warning() {
    let conversion = convert_markdown(&value("#\n\nBody"));
    assert_serialized_body(conversion.fragment.nodes, "Body");
    assert_eq!(conversion.diagnostics.len(), 1);
}

#[test]
fn level_jump_and_level_one_heading_keep_existing_subsection_rules() {
    let conversion = convert_markdown(&value("# Top\n\n### Deep\n\nbody\n\n# Next\n\nlast"));
    assert_serialized_body(
        conversion.fragment.nodes,
        "Top\n\\subsection{Deep}{\n\nbody\n\nNext\n\nlast\n}",
    );
    assert_eq!(conversion.diagnostics.len(), 2);
}

#[test]
fn level_one_heading_does_not_close_an_existing_subsection() {
    let conversion = convert_markdown(&value("## Before\n\nbody\n\n# Top\n\nlast"));
    let body = super::test_support::serialize(conversion.fragment.nodes);
    assert_eq!(body, "\\subsection{Before}{\n\nbody\n\nTop\n\nlast\n}");
    assert_eq!(conversion.diagnostics.len(), 1);
}

#[test]
fn level_one_heading_in_a_block_quote_stays_in_outer_recovery() {
    let conversion = convert_markdown(&value("> # Head\n\nafter"));
    let messages = diagnostic_messages(&conversion);
    assert_serialized_body(conversion.fragment.nodes, "Head\n\nafter");
    assert_eq!(
        messages,
        vec!["unsupported Markdown construct: block quote".to_owned()]
    );
}

#[test]
fn level_one_heading_in_a_list_item_stays_in_list_recovery() {
    let conversion = convert_markdown(&value("- # Head\n- second"));
    let messages = diagnostic_messages(&conversion);
    assert_serialized_body(
        conversion.fragment.nodes,
        "\\itemize{\n\\item Head\n\\item second\n}",
    );
    assert_eq!(
        messages,
        vec!["level-1 Markdown headings are flattened into prose".to_owned()]
    );
}

#[test]
fn unsupported_image_inside_block_quote_reports_both_constructs() {
    let conversion = convert_markdown(&value("> ![alt](img.png)"));
    let messages = diagnostic_messages(&conversion);
    assert_serialized_body(conversion.fragment.nodes, "alt");
    assert_eq!(
        messages,
        vec![
            "unsupported Markdown construct: image".to_owned(),
            "unsupported Markdown construct: block quote".to_owned(),
        ]
    );
}

#[test]
fn heading_inside_block_quote_with_a_supported_list_stays_in_outer_recovery() {
    let conversion = convert_markdown(&value("> - ## Head"));
    let messages = diagnostic_messages(&conversion);
    assert_serialized_body(conversion.fragment.nodes, "\\itemize{\n\\item Head\n}");
    assert_eq!(
        messages,
        vec!["unsupported Markdown construct: block quote".to_owned()]
    );
}

#[test]
fn heading_in_list_item_is_flattened_without_escaping_the_list() {
    let conversion = convert_markdown(&value("- ## Head\n- second\n\nafter"));
    let messages = diagnostic_messages(&conversion);
    assert_serialized_body(
        conversion.fragment.nodes,
        "\\itemize{\n\\item Head\n\\item second\n}\n\nafter",
    );
    assert_eq!(
        messages,
        vec!["unsupported Markdown construct: heading".to_owned()]
    );
}

#[test]
fn heading_in_block_quote_is_covered_by_the_outer_recovery() {
    let conversion = convert_markdown(&value("> ## Head\n\nafter"));
    let messages = diagnostic_messages(&conversion);
    assert_serialized_body(conversion.fragment.nodes, "Head\n\nafter");
    assert_eq!(
        messages,
        vec!["unsupported Markdown construct: block quote".to_owned()]
    );
}

#[test]
fn deeper_heading_in_list_item_is_flattened() {
    let conversion = convert_markdown(&value("- ### Nested"));
    assert_serialized_body(conversion.fragment.nodes, "\\itemize{\n\\item Nested\n}");
    assert_eq!(conversion.diagnostics.len(), 1);
}

#[test]
fn both_break_kinds_become_newlines() {
    let conversion = convert_markdown(&value("soft\nbreak\nhard  \nbreak"));
    assert_eq!(
        conversion.fragment.nodes,
        vec![
            rd_ast::RdNode::Text("soft\n".into()),
            rd_ast::RdNode::Text("break\n".into()),
            rd_ast::RdNode::Text("hard\n".into()),
            rd_ast::RdNode::Text("break".into()),
        ]
    );
}

#[test]
fn nested_strong_and_emphasis_have_direct_children() {
    let conversion = convert_markdown(&value("**outer *inner***"));
    assert_eq!(
        conversion.fragment.nodes,
        vec![rd_ast::RdNode::tagged(
            RdTag::Strong,
            None,
            vec![
                rd_ast::RdNode::Text("outer ".into()),
                rd_ast::RdNode::tagged(
                    RdTag::Emph,
                    None,
                    vec![rd_ast::RdNode::Text("inner".into())],
                )
            ],
        )]
    );
}

#[test]
fn adjacent_text_leaves_are_newline_terminated() {
    // Emphasis puts a node between its neighbours, so it produces no adjacent
    // pair at all; paragraphs do, which is why the rule needs both inputs to
    // mean anything. The writer accepts adjacent same-kind leaves only when the
    // earlier one ends the line.
    for text in [
        "before *middle* after",
        "One.\n\nTwo.",
        "One.\n\nTwo.\n\nThree.",
    ] {
        let conversion = convert_markdown(&value(text));
        let pairs = conversion
            .fragment
            .nodes
            .windows(2)
            .filter(|nodes| matches!(nodes, [rd_ast::RdNode::Text(_), rd_ast::RdNode::Text(_)]))
            .count();
        assert!(
            conversion
                .fragment
                .nodes
                .windows(2)
                .all(|nodes| match nodes {
                    [rd_ast::RdNode::Text(previous), rd_ast::RdNode::Text(_)] => {
                        previous.ends_with('\n')
                    }
                    _ => true,
                }),
            "{text:?} has an adjacent text pair the writer would reject"
        );
        if text.contains("\n\n") {
            assert!(pairs > 0, "{text:?} should exercise the adjacency rule");
        }
    }
}

#[test]
fn text_is_not_pre_escaped() {
    let text = r"literal \ % {} 日本語";
    let conversion = convert_markdown(&value(text));
    assert_eq!(
        conversion.fragment.nodes,
        vec![rd_ast::RdNode::Text(text.into())]
    );
}

#[test]
fn origins_cross_physical_roxygen_lines() {
    let source = SourceFile::new(PathBuf::from("test.R"), "first\n#' second".to_owned());
    let sourced = SourcedText::from_lines(
        &source,
        &[
            Span::new(FileId::new(0), TextRange::new(0, 5)),
            Span::new(FileId::new(0), TextRange::new(9, 15)),
        ],
        NormalizeHead::Intro,
    );
    let conversion = convert_markdown(&MarkdownText::new(sourced));
    assert_eq!(conversion.fragment.nodes.len(), 2);
    assert_eq!(
        conversion.fragment.origins[0].spans,
        vec![
            Span::new(FileId::new(0), TextRange::new(0, 6)),
            Span::new(FileId::new(0), TextRange::new(9, 15)),
        ]
    );
    assert_eq!(conversion.fragment.origins[0].path, FragmentPath::root(0));
    assert_eq!(conversion.fragment.origins[1].path, FragmentPath::root(1));
    assert_eq!(
        conversion.fragment.origins[1].spans,
        conversion.fragment.origins[0].spans
    );
}

#[test]
fn writer_and_r_accept_converted_text() {
    let conversion = convert_markdown(&value(r"100% and a \ backslash {brace}"));
    let document = RdDocument::from(vec![rd_ast::RdNode::tagged(
        RdTag::Description,
        None,
        conversion.fragment.nodes.clone(),
    )]);
    let serialized = Writer::new(rd_writer::WriterOptions::default())
        .write_document(&document)
        .expect("writer accepts the fragment");
    let body = Writer::new(rd_writer::WriterOptions::default())
        .write_document(&RdDocument::from(conversion.fragment.nodes))
        .expect("writer accepts the fragment body");
    // Asserting the spellings directly, because R alone would not catch a
    // missing `%` escape here: the comment it starts would run to the end
    // of the line and the remaining braces would still balance.
    assert_eq!(body, r"100\% and a \\ backslash \{brace\}");
    assert!(serialized.contains(r"\description{"));
    assert_r_accepts(&minimal_topic(&body));

    // A case where an unescaped `%` would swallow a closing brace, so the
    // oracle itself is load-bearing rather than incidental.
    let conversion = convert_markdown(&value("*emphasis 50% off* trailing"));
    let body = Writer::new(rd_writer::WriterOptions::default())
        .write_document(&RdDocument::from(conversion.fragment.nodes))
        .expect("writer accepts the emphasis fragment");
    assert_eq!(body, r"\emph{emphasis 50\% off} trailing");
    assert_r_accepts(&minimal_topic(&body));
}

#[test]
fn input_that_no_rd_leaf_can_hold_is_diagnosed_rather_than_panicking() {
    // R accepts a brace pair spanning lines inside an equation, but each Rd
    // equation leaf must balance its own braces and leaves end at every line,
    // so this one is beyond the AST rather than beyond R. Refusing the macro
    // keeps the surrounding prose and lets the rest of the topic build.
    let conversion = convert_markdown(&value("Value \\eqn{{\nx}} end."));
    assert_eq!(
        conversion.fragment.nodes,
        vec![
            rd_ast::RdNode::Text("Value \\eqn{{\n".into()),
            rd_ast::RdNode::Text("x}} end.".into()),
        ]
    );
    assert!(
        conversion
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code
                == crate::diagnostic::DiagnosticCode::UnsupportedRawRdMacro)
    );

    // An equation whose every line balances on its own is still lowered.
    let conversion = convert_markdown(&value("Value \\eqn{x\ny} end."));
    assert!(conversion.diagnostics.is_empty());
    assert!(matches!(
        conversion.fragment.nodes.get(1),
        Some(rd_ast::RdNode::Tagged(tagged)) if *tagged.tag() == rd_ast::RdTag::Eqn
    ));
}

#[test]
fn a_link_target_no_rd_option_can_hold_keeps_its_text() {
    // An Rd link option ends at the first `]`; R rejects the result too. A
    // carriage return has no representable leaf. Either way the prose survives
    // and only the link is refused.
    for text in ["[text][pkg::foo%5Dbar]", "[text][pkg::foo%0Dbar]"] {
        let conversion = convert_markdown(&value(text));
        assert_eq!(
            conversion.fragment.nodes,
            vec![rd_ast::RdNode::Text("text".into())],
            "{text}"
        );
        assert!(
            conversion
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code
                    == crate::diagnostic::DiagnosticCode::UnsupportedMarkdownConstruct),
            "{text}"
        );
    }

    // Refusing the link must not also drop the label's code formatting.
    let conversion = convert_markdown(&value("[`x + 1`][pkg::foo%5Dbar]"));
    assert_eq!(
        conversion.fragment.nodes,
        vec![rd_ast::RdNode::tagged(
            rd_ast::RdTag::Code,
            None,
            vec![rd_ast::RdNode::RCode("x + 1".into())],
        )]
    );
    // A verbatim label survives refusal as verbatim, not as text.
    let conversion = convert_markdown(&value("[`not r code`][pkg::foo%5Dbar]"));
    assert_eq!(
        conversion.fragment.nodes,
        vec![rd_ast::RdNode::tagged(
            rd_ast::RdTag::Verb,
            None,
            vec![rd_ast::RdNode::Verb("not r code".into())],
        )]
    );

    // A target with nothing unrepresentable in it still becomes a link.
    let conversion = convert_markdown(&value("[text][pkg::foo]"));
    assert!(conversion.diagnostics.is_empty());
    assert!(matches!(
        conversion.fragment.nodes.first(),
        Some(rd_ast::RdNode::Tagged(tagged)) if *tagged.tag() == rd_ast::RdTag::Link
    ));
}
