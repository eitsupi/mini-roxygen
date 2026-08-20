//! Diagnostic helpers shared by semantic tag parsing.

use super::model::UnknownTagPolicy;
use super::text::SourcedText;
use crate::arity_adapter::RawTag;
use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, Label, Severity};
use crate::source::{Span, TextRange};

pub(super) fn value_span(value: &SourcedText, fallback: Span) -> Span {
    value_span_for_range(value, 0, value.as_str().len(), fallback)
}

pub(super) fn value_span_for_range(
    value: &SourcedText,
    start: usize,
    end: usize,
    fallback: Span,
) -> Span {
    let spans = value.source_spans(TextRange::new(
        u32::try_from(start).expect("normalized text length fits u32"),
        u32::try_from(end).expect("normalized text length fits u32"),
    ));
    let Some(first) = spans.first().copied() else {
        return fallback;
    };
    let Some(last) = spans.last().copied() else {
        return first;
    };
    if first.file == last.file {
        Span::new(
            first.file,
            TextRange::new(first.range.start(), last.range.end()),
        )
    } else {
        first
    }
}

pub(super) fn emit_tag_diagnostic(
    diagnostics: &mut Diagnostics,
    raw_tag: &RawTag,
    code: DiagnosticCode,
    message: impl Into<String>,
    span: Span,
) {
    let name = raw_tag.name.value.clone();
    diagnostics.push(
        Diagnostic::new(
            code.default_severity(),
            code,
            message,
            Label::new(span, format!("problem in @{name}")),
        )
        .with_context("tag", name),
    );
}

pub(super) fn emit_unsupported_diagnostic(
    diagnostics: &mut Diagnostics,
    raw_tag: &RawTag,
    value: &SourcedText,
) {
    let name = &raw_tag.name.value;
    diagnostics.push(
        Diagnostic::new(
            DiagnosticCode::UnsupportedTag.default_severity(),
            DiagnosticCode::UnsupportedTag,
            format!("@{name} requires evaluating R, which mini-roxygen does not do"),
            Label::new(
                value_span(value, raw_tag.value_span),
                format!("unsupported R-evaluated tag @{name}"),
            ),
        )
        .with_context("tag", name.clone()),
    );
}

pub(super) fn emit_unknown_diagnostic(
    diagnostics: &mut Diagnostics,
    raw_tag: &RawTag,
    policy: UnknownTagPolicy,
) {
    let severity = match policy {
        UnknownTagPolicy::Ignore => return,
        UnknownTagPolicy::Warn => DiagnosticCode::UnknownTag.default_severity(),
        UnknownTagPolicy::Error => Severity::Error,
    };
    let name = &raw_tag.name.value;
    diagnostics.push(
        Diagnostic::new(
            severity,
            DiagnosticCode::UnknownTag,
            format!("unrecognized roxygen tag @{name}"),
            Label::new(raw_tag.full_span, format!("unrecognized tag @{name}")),
        )
        .with_context("tag", name.clone()),
    );
}

#[cfg(test)]
mod tests {
    use crate::diagnostic::{DiagnosticCode, Severity};
    use crate::tags::test_support::parsed;
    use crate::tags::{ParsedTag, UnknownTagPolicy};

    #[test]
    fn unknown_tags_are_retained_with_policy_specific_diagnostics() {
        for (policy, expected_severity, expected_count) in [
            (UnknownTagPolicy::Ignore, None, 0),
            (UnknownTagPolicy::Warn, Some(Severity::Warning), 1),
            (UnknownTagPolicy::Error, Some(Severity::Error), 1),
        ] {
            let (tags, diagnostics, _) = parsed(
                r#"#' @future value
"#,
                policy,
            );
            assert!(matches!(&tags[0], ParsedTag::Unknown(_)));
            assert_eq!(diagnostics.len(), expected_count);
            if let Some(severity) = expected_severity {
                let diagnostic = diagnostics.iter().next().expect("diagnostic");
                assert_eq!(diagnostic.severity, severity);
                assert_eq!(diagnostic.code, DiagnosticCode::UnknownTag);
                assert_eq!(
                    diagnostic.context,
                    vec![("tag".to_owned(), "future".to_owned())]
                );
            }
        }
    }

    #[test]
    fn whitespace_only_title_is_dropped_with_diagnostic() {
        let (tags, diagnostics, _) = parsed(
            r#"#' @title  
"#,
            UnknownTagPolicy::Warn,
        );
        assert!(tags.is_empty());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics.iter().next().expect("diagnostic").message,
            "@title requires a value"
        );
    }

    #[test]
    fn blank_continuation_title_is_dropped_with_diagnostic() {
        let (tags, diagnostics, _) = parsed(
            r#"#' @title
#'   
"#,
            UnknownTagPolicy::Warn,
        );
        assert!(tags.is_empty());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics.iter().next().expect("diagnostic").message,
            "@title requires a value"
        );
    }

    #[test]
    fn required_value_diagnostic_uses_the_untrimmed_value_span() {
        let (tags, diagnostics, source) = parsed(
            r#"#' @title   
"#,
            UnknownTagPolicy::Warn,
        );
        assert!(tags.is_empty());
        let diagnostic = diagnostics.iter().next().expect("diagnostic");
        assert_eq!(diagnostic.message, "@title requires a value");
        assert_eq!(
            diagnostic.context,
            vec![("tag".to_owned(), "title".to_owned())]
        );
        assert_eq!(source.text_range(diagnostic.primary.span.range), Some("  "));
        assert_eq!(diagnostic.primary.message, "@title is missing a value");
    }

    #[test]
    fn empty_section_has_only_the_required_value_diagnostic() {
        let (tags, diagnostics, _) = parsed(
            r#"#' @section
"#,
            UnknownTagPolicy::Warn,
        );
        assert!(tags.is_empty());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics.iter().next().expect("diagnostic").message,
            "@section requires a value"
        );
    }

    #[test]
    fn unsupported_tags_always_diagnose_and_markers_are_not_unknown() {
        let (unsupported, diagnostics, source) = parsed(
            r"#' @eval value
#' @evalRd value
#' @evalNamespace value
#' @template value
#' @templateVar value
#' @includeRmd value
",
            UnknownTagPolicy::Ignore,
        );
        assert_eq!(unsupported.len(), 6);
        assert_eq!(diagnostics.len(), 6);
        assert!(
            unsupported
                .iter()
                .all(|tag| matches!(tag, ParsedTag::Unsupported(_)))
        );
        assert!(diagnostics.iter().all(|diagnostic| {
            diagnostic.code == DiagnosticCode::UnsupportedTag
                && diagnostic.message.contains("requires evaluating R")
        }));
        let eval_namespace = diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.context == [("tag".to_owned(), "evalNamespace".to_owned())]
            })
            .expect("evalNamespace diagnostic");
        assert_eq!(
            source.text_range(eval_namespace.primary.span.range),
            Some("value")
        );
        assert_eq!(
            eval_namespace.primary.message,
            "unsupported R-evaluated tag @evalNamespace"
        );

        let (markers, marker_diagnostics, _) = parsed(
            r"#' @md
#' @noMd
",
            UnknownTagPolicy::Warn,
        );
        assert!(markers.is_empty());
        assert!(marker_diagnostics.is_empty());

        let (bare, bare_diagnostics, _) = parsed(
            r#"#' @md
"#,
            UnknownTagPolicy::Warn,
        );
        assert!(bare.is_empty());
        assert!(bare_diagnostics.is_empty());

        let (payload, payload_diagnostics, _) = parsed(
            r#"#' @md typo
"#,
            UnknownTagPolicy::Warn,
        );
        assert!(payload.is_empty());
        assert_eq!(payload_diagnostics.len(), 1);
        assert_eq!(
            payload_diagnostics
                .iter()
                .next()
                .expect("diagnostic")
                .message,
            "@md must not be followed by any text"
        );
    }

    #[test]
    fn required_value_table_rejects_empty_values_and_accepts_non_empty_values() {
        let cases = [
            ("title", "value"),
            ("description", "value"),
            ("details", "value"),
            ("return", "value"),
            ("returns", "value"),
            ("seealso", "value"),
            ("references", "value"),
            ("note", "value"),
            ("format", "value"),
            ("source", "value"),
            ("author", "value"),
            ("name", "value"),
            ("rdname", "value"),
            ("usage", "value"),
            ("examples", "value()"),
            ("aliases", "value"),
            ("keywords", "value"),
            ("section", "Title:"),
            ("param", "x description"),
            ("inherit", "source"),
            ("inheritParams", "source"),
        ];

        for (name, non_empty) in cases {
            let empty_input = format!(
                r#"#' @{name}
"#
            );
            let (empty_tags, empty_diagnostics, _) = parsed(&empty_input, UnknownTagPolicy::Warn);
            assert!(empty_tags.is_empty(), "empty @{name} must be dropped");
            assert_eq!(
                empty_diagnostics.len(),
                1,
                "empty @{name} must diagnose once"
            );

            let non_empty_input = format!(
                r#"#' @{name} {non_empty}
"#
            );
            let (non_empty_tags, non_empty_diagnostics, _) =
                parsed(&non_empty_input, UnknownTagPolicy::Warn);
            assert_eq!(
                non_empty_diagnostics.len(),
                0,
                "non-empty @{name} must parse"
            );
            assert_eq!(
                non_empty_tags.len(),
                1,
                "non-empty @{name} must be retained"
            );
        }
    }
}
