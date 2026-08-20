//! Decomposes an untagged block intro into semantic prose tags.
//!
//! Intro decomposition is deliberately lexical: paragraphs are separated by
//! the literal `"\n\n"` before Markdown is parsed. Keeping this layer independent
//! of Markdown prevents the semantic tag layer from depending on a later
//! representation and preserves the raw roxygen2 ordering rule.

use crate::arity_adapter::{RawBody, RawTag};
use crate::source::{SourceFile, Span, TextRange};

use super::{FieldValue, MarkdownText, ParsedTag, SourcedText, TagOrigin, TagValue};

/// One parsed tag together with the raw tag that produced it.
pub(super) struct ParsedTagEntry {
    /// The position in the block's raw tag list.
    pub(super) raw_index: usize,
    /// The semantic tag, when parsing retained it.
    pub(super) tag: ParsedTag,
}

/// Reconciles implicit intro tags with already parsed explicit tags.
pub(super) fn reconcile(
    source_file: &SourceFile,
    intro: Option<&RawBody>,
    raw_tags: &[RawTag],
    explicit: Vec<ParsedTagEntry>,
) -> Vec<ParsedTag> {
    let Some(intro) = intro else {
        return explicit.into_iter().map(|entry| entry.tag).collect();
    };

    let normalized = super::trim_outer(SourcedText::from_body(source_file, intro));
    if normalized.is_empty() {
        return explicit.into_iter().map(|entry| entry.tag).collect();
    }

    // These names must be inspected before semantic parsing, because the
    // presence of an explicit tag controls which intro paragraph is promoted.
    let has_title = raw_tags.iter().any(|tag| tag.name.value == "title");
    let has_description = raw_tags.iter().any(|tag| tag.name.value == "description");

    let mut paragraphs = split_paragraphs(&normalized);
    let origin = TagOrigin::Implicit {
        intro_span: intro.full_span,
    };
    let mut implicit = Vec::new();

    if !has_title && let Some(title) = take_first(&mut paragraphs) {
        implicit.push(ParsedTag::Title(TagValue {
            value: FieldValue::Emit(MarkdownText::new(title)),
            origin: origin.clone(),
        }));
    }

    if !has_description && let Some(description) = take_first(&mut paragraphs) {
        implicit.push(ParsedTag::Description(TagValue {
            value: FieldValue::Emit(MarkdownText::new(description)),
            origin: origin.clone(),
        }));
    }

    if paragraphs.is_empty() {
        implicit.extend(explicit.into_iter().map(|entry| entry.tag));
        return implicit;
    }

    let anchor = zero_width_anchor(intro.full_span);
    let mut detail_parts = Vec::with_capacity(paragraphs.len());
    detail_parts.append(&mut paragraphs);

    let mut explicit_details = Vec::new();
    for entry in &explicit {
        let raw_tag = raw_tags
            .get(entry.raw_index)
            .expect("every parsed tag must retain its raw-tag index");
        if raw_tag.name.value == "details"
            && let ParsedTag::Details(details) = &entry.tag
            && let FieldValue::Emit(details) = &details.value
        {
            explicit_details.push(details.sourced().clone());
        }
    }
    detail_parts.extend(explicit_details);

    let details = SourcedText::concat_with(detail_parts, "\n\n", anchor);
    implicit.push(ParsedTag::Details(TagValue {
        value: FieldValue::Emit(MarkdownText::new(details)),
        origin,
    }));

    implicit.extend(explicit.into_iter().filter_map(|entry| {
        let raw_tag = raw_tags
            .get(entry.raw_index)
            .expect("every parsed tag must retain its raw-tag index");
        (raw_tag.name.value != "details").then_some(entry.tag)
    }));
    implicit
}

fn split_paragraphs(value: &SourcedText) -> Vec<SourcedText> {
    let mut paragraphs = Vec::new();
    let mut start = 0usize;
    for (separator_start, _) in value.as_str().match_indices("\n\n") {
        paragraphs.push(value.slice(TextRange::new(
            u32::try_from(start).expect("normalized text length fits u32"),
            u32::try_from(separator_start).expect("normalized text length fits u32"),
        )));
        start = separator_start + 2;
    }
    paragraphs.push(value.slice(TextRange::new(
        u32::try_from(start).expect("normalized text length fits u32"),
        u32::try_from(value.as_str().len()).expect("normalized text length fits u32"),
    )));
    paragraphs
}

fn take_first(parts: &mut Vec<SourcedText>) -> Option<SourcedText> {
    if parts.is_empty() {
        None
    } else {
        Some(parts.remove(0))
    }
}

fn zero_width_anchor(span: Span) -> Span {
    Span::new(
        span.file,
        TextRange::new(span.range.start(), span.range.start()),
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::arity_adapter::parse;
    use crate::source::{FileId, SourceFile};
    use crate::tags::{FieldValue, ParsedTag, TagOrigin, TagParseOptions, UnknownTagPolicy};

    fn parsed(text: &str) -> (Vec<ParsedTag>, crate::diagnostic::Diagnostics) {
        let source = SourceFile::new(
            PathBuf::from("test.R"),
            format!(
                r#"{text}
NULL
"#
            ),
        );
        let parsed = parse(&source, FileId::new(0));
        let block = parsed.top_level[0]
            .documentation
            .as_ref()
            .expect("expected documentation");
        let (explicit, diagnostics) = crate::tags::parse_block(
            &source,
            block,
            &TagParseOptions::default().with_unknown_tags(UnknownTagPolicy::Ignore),
        );
        (explicit, diagnostics)
    }

    fn text(tag: &ParsedTag) -> &str {
        match tag {
            ParsedTag::Title(value) | ParsedTag::Description(value) | ParsedTag::Details(value) => {
                let FieldValue::Emit(value) = &value.value else {
                    panic!("expected an emitted prose tag");
                };
                value.as_str()
            }
            _ => panic!("expected a prose tag"),
        }
    }

    #[test]
    fn one_paragraph_intro_becomes_only_title() {
        let (tags, diagnostics) = parsed(
            r#"#' Intro.
"#,
        );

        assert!(diagnostics.is_empty());
        assert_eq!(tags.len(), 1);
        assert_eq!(text(&tags[0]), "Intro.");
        assert!(matches!(tags[0], ParsedTag::Title(_)));
    }

    #[test]
    fn explicit_title_promotes_intro_to_description() {
        let (tags, _) = parsed(
            r"#' Intro description.
#' @title Explicit title
",
        );

        assert!(matches!(tags[0], ParsedTag::Description(_)));
        assert_eq!(text(&tags[0]), "Intro description.");
        assert!(matches!(tags[1], ParsedTag::Title(_)));
    }

    #[test]
    fn explicit_description_promotes_intro_to_title() {
        let (tags, _) = parsed(
            r"#' Intro title.
#' @description Explicit description
",
        );

        assert!(matches!(tags[0], ParsedTag::Title(_)));
        assert_eq!(text(&tags[0]), "Intro title.");
        assert!(matches!(tags[1], ParsedTag::Description(_)));
    }

    #[test]
    fn explicit_title_and_description_make_intro_details() {
        let (tags, _) = parsed(
            r"#' Intro details.
#' @title Explicit title
#' @description Explicit description
",
        );

        assert!(matches!(tags[0], ParsedTag::Details(_)));
        assert_eq!(text(&tags[0]), "Intro details.");
        assert!(matches!(tags[1], ParsedTag::Title(_)));
        assert!(matches!(tags[2], ParsedTag::Description(_)));
    }

    #[test]
    fn multiple_intro_paragraphs_split_and_join_details() {
        let (tags, _) = parsed(
            r"#' Title.
#'
#' Description.
#'
#' Detail one.
#'
#' Detail two.
#' @title Explicit title
",
        );

        assert!(matches!(tags[0], ParsedTag::Description(_)));
        assert_eq!(text(&tags[0]), "Title.");
        assert_eq!(text(&tags[1]), "Description.\n\nDetail one.\n\nDetail two.");
        assert!(matches!(tags[2], ParsedTag::Title(_)));
    }

    #[test]
    fn implicit_details_append_and_remove_explicit_details() {
        let (tags, _) = parsed(
            r"#' Title.
#'
#' Description.
#'
#' Intro detail.
#' @details Explicit detail one
#' @note Note
#' @details Explicit detail two
",
        );

        assert_eq!(
            text(&tags[2]),
            "Intro detail.\n\nExplicit detail one\n\nExplicit detail two"
        );
        assert!(matches!(tags[3], ParsedTag::Note(_)));
        assert_eq!(tags.len(), 4);
    }

    #[test]
    fn dropped_marker_does_not_break_explicit_details_reconciliation() {
        let (tags, diagnostics) = parsed(
            r"#' Intro title.
#'
#' Intro description.
#'
#' Intro detail.
#' @md
#' @details Explicit detail.
",
        );

        assert!(diagnostics.is_empty());
        assert_eq!(tags.len(), 3);
        assert_eq!(text(&tags[2]), "Intro detail.\n\nExplicit detail.");
        assert!(matches!(tags[2], ParsedTag::Details(_)));
    }

    #[test]
    fn rejected_required_tag_still_blocks_its_implicit_slot() {
        let (tags, diagnostics) = parsed(
            r"#' Intro title.
#'
#' Intro description.
#'
#' Intro detail.
#' @title
#' @details Explicit detail.
",
        );

        assert_eq!(diagnostics.len(), 1);
        assert!(matches!(tags[0], ParsedTag::Description(_)));
        assert_eq!(text(&tags[0]), "Intro title.");
        assert!(tags.iter().all(|tag| !matches!(tag, ParsedTag::Title(_))));
        assert_eq!(
            text(&tags[1]),
            "Intro description.\n\nIntro detail.\n\nExplicit detail."
        );
    }

    #[test]
    fn without_implicit_details_explicit_details_keep_their_positions() {
        let (tags, _) = parsed(
            r"#' Intro title.
#' @details Explicit detail
#' @note Note
",
        );

        assert_eq!(tags.len(), 3);
        assert!(matches!(tags[0], ParsedTag::Title(_)));
        assert!(matches!(tags[1], ParsedTag::Details(_)));
        assert!(matches!(tags[2], ParsedTag::Note(_)));
    }

    #[test]
    fn implicit_values_retain_intro_provenance() {
        let (tags, _) = parsed(
            r#"#' Intro.
"#,
        );
        let ParsedTag::Title(title) = &tags[0] else {
            panic!("expected title");
        };
        assert!(matches!(title.origin, TagOrigin::Implicit { .. }));
        let FieldValue::Emit(title_value) = &title.value else {
            panic!("expected emitted title");
        };
        assert_eq!(title_value.source_span_at(0).unwrap().range.start(), 3);
    }

    #[test]
    fn empty_intro_is_discarded_and_no_intro_adds_no_tags() {
        let (empty, _) = parsed(
            r#"#'   
#' @note Note
"#,
        );
        assert_eq!(empty.len(), 1);
        assert!(matches!(empty[0], ParsedTag::Note(_)));

        let (no_intro, _) = parsed(
            r#"#' @note Note
"#,
        );
        assert_eq!(no_intro.len(), 1);
        assert!(matches!(no_intro[0], ParsedTag::Note(_)));
    }
}
