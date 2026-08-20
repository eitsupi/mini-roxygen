//! Section-title splitting and diagnostics for semantic section tags.

use super::text::SourcedText;
use crate::arity_adapter::RawTag;
use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, Label};
use crate::markdown::protected_markdown_ranges;
use crate::source::{Span, TextRange};

/// Finds the first section separator colon outside escaped text, raw braces,
/// and the Markdown constructs recognized by the shared parser.
///
/// A valid code span takes precedence over brace tracking: braces inside a
/// code span do not change the raw-Rd depth, while a code span inside braces
/// is still recognized but cannot expose a separator because the surrounding
/// brace depth remains non-zero.
///
/// The returned byte offset is relative to the normalized value. A `None`
/// result means that the whole value is the title and the body is empty.
pub(crate) fn split_section_title(value: &str) -> Option<(usize, usize)> {
    let protected = protected_markdown_ranges(value);
    let bytes = value.as_bytes();
    let mut depth = 0i32;
    let mut escaped = false;
    let mut index = 0;
    let mut protected_index = 0;

    while index < bytes.len() {
        while protected_index < protected.len() && protected[protected_index].end <= index {
            protected_index += 1;
        }
        if let Some(range) = protected.get(protected_index)
            && range.start <= index
        {
            index = range.end;
            continue;
        }

        let character = value[index..]
            .chars()
            .next()
            .expect("byte index must be a character boundary");

        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '{' {
            depth += 1;
        } else if character == '}' {
            depth -= 1;
        } else if character == ':' && depth == 0 {
            return Some((index, index + character.len_utf8()));
        }
        index += character.len_utf8();
    }
    None
}

pub(super) fn emit_section_diagnostic(
    diagnostics: &mut Diagnostics,
    raw_tag: &RawTag,
    value: &SourcedText,
    multiline: bool,
) {
    let (message, label_message) = if multiline {
        (
            "@section title spans multiple lines; did you forget a colon (:) at the end of the title?",
            "section title spans multiple lines; add a separator colon",
        )
    } else {
        (
            "@section is missing a colon separating its title and body",
            "section title has no separator colon",
        )
    };
    let spans = value.source_spans(TextRange::new(
        0,
        u32::try_from(value.as_str().len()).expect("normalized text length fits u32"),
    ));
    let span = match (spans.first(), spans.last()) {
        (Some(first), Some(last)) if first.file == last.file => Span::new(
            first.file,
            TextRange::new(first.range.start(), last.range.end()),
        ),
        (Some(first), _) => *first,
        _ => raw_tag.value_span,
    };
    diagnostics.push(
        Diagnostic::new(
            DiagnosticCode::TagParseError.default_severity(),
            DiagnosticCode::TagParseError,
            message,
            Label::new(span, label_message),
        )
        .with_context("tag", raw_tag.name.value.clone()),
    );
}

#[cfg(test)]
mod tests {
    use super::split_section_title;
    use crate::diagnostic::DiagnosticCode;
    use crate::tags::test_support::{parsed, split_parts};
    use crate::tags::{ParsedTag, TagOrigin, UnknownTagPolicy};

    #[test]
    fn section_split_helper_handles_escaping_braces_and_code_spans() {
        assert_eq!(split_section_title("Plain title: body"), Some((11, 12)));
        assert_eq!(split_section_title("Title {a:b}: body"), Some((11, 12)));
        assert_eq!(split_section_title("Title\\: body: real"), Some((12, 13)));
        assert_eq!(split_section_title("base::foo: body"), Some((4, 5)));
        assert_eq!(
            split_parts("Similar to `base::split()`: Content."),
            ("Similar to `base::split()`", " Content.")
        );
        assert_eq!(
            split_parts("Similar to ``base::split()``: Content."),
            ("Similar to ``base::split()``", " Content.")
        );
        assert_eq!(
            split_parts("Title ``a`b:c``: body"),
            ("Title ``a`b:c``", " body")
        );
        assert_eq!(
            split_parts("Title `code: value`: body"),
            ("Title `code: value`", " body")
        );
        assert_eq!(
            split_parts("Title {`code:a`}: body"),
            ("Title {`code:a`}", " body")
        );
        assert_eq!(
            split_parts("Title `code{a:b}`: body"),
            ("Title `code{a:b}`", " body")
        );
        assert_eq!(
            split_section_title("Title `unterminated: body"),
            Some((19, 20))
        );
        assert_eq!(split_section_title("Title"), None);
        assert_eq!(split_section_title("Title:"), Some((5, 6)));
    }

    #[test]
    fn section_split_protects_markdown_inline_constructs() {
        assert_eq!(
            split_parts("*base::split()*: Body"),
            ("*base::split()*", " Body")
        );
        assert_eq!(
            split_parts("[base::split()](/help): Body"),
            ("[base::split()](/help)", " Body")
        );
        assert_eq!(
            split_parts("`base::split()`: Body"),
            ("`base::split()`", " Body")
        );
        assert_eq!(split_parts("base*name*: Body"), ("base*name*", " Body"));
        assert_eq!(split_parts("Plain title: Body"), ("Plain title", " Body"));
        assert_eq!(split_parts("Title `code`: Body"), ("Title `code`", " Body"));
    }

    #[test]
    fn section_split_protects_roxygen_link_envelopes() {
        assert_eq!(
            split_parts("[base::split()] helper: body text"),
            ("[base::split()] helper", " body text")
        );
        assert_eq!(split_parts("[func()]: body"), ("[func()]", " body"));
        assert_eq!(split_parts("[pkg::topic]: body"), ("[pkg::topic]", " body"));
        assert_eq!(
            split_parts("[text][target]: body"),
            ("[text][target]", " body")
        );
        assert_eq!(
            split_parts("[text][pkg::topic]: body"),
            ("[text][pkg::topic]", " body")
        );
        assert_eq!(
            split_parts("[][pkg::topic]: body"),
            ("[][pkg", ":topic]: body")
        );
        assert_eq!(split_parts("[a:b[c][ref]: body"), ("[a", "b[c][ref]: body"));
        assert_eq!(
            split_parts("[%%] operator: body"),
            ("[%%] operator", " body")
        );
        assert_eq!(
            split_parts("[not an R identifier: really]: body"),
            ("[not an R identifier: really]", " body")
        );
        assert_eq!(
            split_parts("[base::split()](/help): body"),
            ("[base::split()](/help)", " body")
        );
        // The raw fallback cannot see that the brackets straddle a code span.
        assert_eq!(split_parts("[a`b` :c]: body"), ("[a`b` :c]", " body"));
    }

    #[test]
    fn section_split_protects_markdown_block_constructs() {
        assert_eq!(
            split_parts("Title:\n\n```r\nx:y\n```"),
            ("Title", "\n\n```r\nx:y\n```")
        );
        assert_eq!(split_parts("Title:\n\n    x:y"), ("Title", "\n\n    x:y"));
        assert_eq!(
            split_parts("Title:\n\n| head |\n| --- |\n| x:y |"),
            ("Title", "\n\n| head |\n| --- |\n| x:y |")
        );
        assert_eq!(
            split_parts("Title:\n- body: detail"),
            ("Title", "\n- body: detail")
        );
        assert_eq!(
            split_parts("Title:\n\n- body: detail"),
            ("Title", "\n\n- body: detail")
        );
        assert_eq!(
            split_parts("```r\nx:y\n```\nReal: body"),
            ("```r\nx:y\n```\nReal", " body")
        );
        assert_eq!(
            split_parts("- body: detail\n\nReal: body"),
            ("- body: detail\n\nReal", " body")
        );
        assert_eq!(
            split_parts("| head |\n| --- |\n| x:y |\n\nReal: body"),
            ("| head |\n| --- |\n| x:y |\n\nReal", " body")
        );
        assert_eq!(
            split_parts("<pre>\nx:y\n</pre>\nReal: body"),
            ("<pre>\nx:y\n</pre>\nReal", " body")
        );
    }

    #[test]
    fn section_split_uses_commonmark_code_span_backslash_rules() {
        assert_eq!(split_section_title(r"`a\`: body `x`: tail"), Some((4, 5)));
    }

    #[test]
    fn section_matches_roxygen_code_span_split_and_preserves_provenance() {
        let (tags, diagnostics, source) = parsed(
            r"#' @section Similar to `base::split()`: Content.
#' @section Title {a:b}: body
",
            UnknownTagPolicy::Warn,
        );

        let ParsedTag::Section {
            title,
            body,
            origin: TagOrigin::Explicit { .. },
        } = &tags[0]
        else {
            panic!("expected section");
        };
        assert_eq!(title.as_str(), "Similar to `base::split()`");
        assert_eq!(body.as_str(), " Content.");
        assert_eq!(
            source.text_range(
                title.sourced().source_spans(crate::source::TextRange::new(
                    0,
                    title.as_str().len() as u32
                ))[0]
                    .range
            ),
            Some("Similar to `base::split()`")
        );
        assert_eq!(
            source.text_range(
                body.sourced()
                    .source_spans(crate::source::TextRange::new(0, body.as_str().len() as u32))[0]
                    .range
            ),
            Some(" Content.")
        );

        let ParsedTag::Section { title, body, .. } = &tags[1] else {
            panic!("expected section");
        };
        assert_eq!(title.as_str(), "Title {a:b}");
        assert_eq!(body.as_str(), " body");
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn single_line_section_without_colon_is_retained_with_diagnostic() {
        let (tags, diagnostics, _) = parsed(
            r"#' @section Missing colon
#' @title Other tag
",
            UnknownTagPolicy::Warn,
        );

        let ParsedTag::Section { title, body, .. } = &tags[0] else {
            panic!("expected malformed section to survive");
        };
        assert_eq!(title.as_str(), "Missing colon");
        assert!(body.is_empty());
        assert!(matches!(tags[1], ParsedTag::Title(_)));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics.iter().next().expect("diagnostic").code,
            DiagnosticCode::TagParseError
        );
        assert_eq!(
            diagnostics.iter().next().expect("diagnostic").message,
            "@section is missing a colon separating its title and body"
        );
    }

    #[test]
    fn multiline_section_without_colon_is_dropped_with_diagnostic() {
        let (tags, diagnostics, _) = parsed(
            r"#' @section Missing colon
#' continuation line
#' @title Other tag
",
            UnknownTagPolicy::Warn,
        );

        assert_eq!(tags.len(), 1);
        assert!(matches!(tags[0], ParsedTag::Title(_)));
        assert_eq!(diagnostics.len(), 1);
        let diagnostic = diagnostics.iter().next().expect("diagnostic");
        assert_eq!(diagnostic.code, DiagnosticCode::TagParseError);
        assert!(diagnostic.message.contains("spans multiple lines"));
        assert!(diagnostic.message.contains("forget a colon"));
    }
}
