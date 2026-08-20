//! Parsing for @inherit and @inheritParams tags.
//!
//! This module parses inheritance tags; it is distinct from the crate-level
//! inheritance resolution layer.

use super::diagnostics::{emit_tag_diagnostic, value_span, value_span_for_range};
use super::model::{
    ArgSelection, ArgSelector, InheritField, InheritFields, InheritTarget, ParamName, ParsedTag,
    TagOrigin, TopicRef,
};
use super::text::SourcedText;
use super::words::word_ranges;
use crate::arity_adapter::RawTag;
use crate::diagnostic::{DiagnosticCode, Diagnostics};
use crate::source::{Span, Spanned, TextRange};

pub(super) fn parse_inherit(
    raw_tag: &RawTag,
    value: SourcedText,
    origin: TagOrigin,
    diagnostics: &mut Diagnostics,
) -> Option<ParsedTag> {
    let words = word_ranges(&value);
    let Some((source_start, source_end)) = words.first().copied() else {
        emit_tag_diagnostic(
            diagnostics,
            raw_tag,
            DiagnosticCode::TagParseError,
            "@inherit requires a source topic",
            value_span(&value, raw_tag.value_span),
        );
        return None;
    };
    let target = classify_inherit_target(&value, source_start, source_end, raw_tag.value_span);
    let mut selected = Vec::new();
    for (start, end) in words.iter().copied().skip(1) {
        let field_text = &value.as_str()[start..end];
        if let Some(field) = parse_inherit_field(field_text) {
            selected.push(Spanned::new(
                field,
                value_span_for_range(&value, start, end, raw_tag.value_span),
            ));
        } else {
            emit_tag_diagnostic(
                diagnostics,
                raw_tag,
                DiagnosticCode::TagParseError,
                format!("@inherit does not recognize field `{field_text}`"),
                value_span_for_range(&value, start, end, raw_tag.value_span),
            );
        }
    }
    let fields = if words.len() == 1 {
        let anchor = value
            .source_anchor_at(u32::try_from(value.as_str().len()).expect("text length fits u32"))
            .unwrap_or(raw_tag.value_span);
        InheritFields::All { anchor }
    } else {
        InheritFields::Selected(selected)
    };

    Some(ParsedTag::Inherit {
        target,
        fields,
        origin,
    })
}

pub(super) fn parse_inherit_params(
    raw_tag: &RawTag,
    value: SourcedText,
    origin: TagOrigin,
    diagnostics: &mut Diagnostics,
) -> Option<ParsedTag> {
    let words = word_ranges(&value);
    let Some((source_start, source_end)) = words.first().copied() else {
        emit_tag_diagnostic(
            diagnostics,
            raw_tag,
            DiagnosticCode::TagParseError,
            "@inheritParams requires a source topic",
            value_span(&value, raw_tag.value_span),
        );
        return None;
    };
    let target = classify_inherit_target(&value, source_start, source_end, raw_tag.value_span);
    let selection = if words.len() == 1 {
        None
    } else {
        let mut selectors = Vec::new();
        for (start, end) in words.iter().copied().skip(1) {
            let selector = &value.as_str()[start..end];
            let span = value_span_for_range(&value, start, end, raw_tag.value_span);
            let Some(selector) = parse_arg_selector(selector, span) else {
                emit_tag_diagnostic(
                    diagnostics,
                    raw_tag,
                    DiagnosticCode::UnsupportedSelection,
                    format!(
                        "unsupported inheritParams selector `{selector}`; supported forms are `name` and `-name`"
                    ),
                    span,
                );
                return None;
            };
            selectors.push(selector);
        }
        let selection = ArgSelection { selectors };
        Some(selection)
    };

    Some(ParsedTag::InheritParams {
        target,
        selection,
        origin,
    })
}

pub(super) fn parse_inherit_section(
    raw_tag: &RawTag,
    value: SourcedText,
    origin: TagOrigin,
    diagnostics: &mut Diagnostics,
) -> Option<ParsedTag> {
    let words = word_ranges(&value);
    let Some((source_start, source_end)) = words.first().copied() else {
        emit_tag_diagnostic(
            diagnostics,
            raw_tag,
            DiagnosticCode::TagParseError,
            "@inheritSection requires a source topic and section title",
            value_span(&value, raw_tag.value_span),
        );
        return None;
    };
    if &value.as_str()[source_start..source_end] == "NULL" {
        emit_tag_diagnostic(
            diagnostics,
            raw_tag,
            DiagnosticCode::TagParseError,
            "@inheritSection requires a source topic, not NULL",
            value_span_for_range(&value, source_start, source_end, raw_tag.value_span),
        );
        return None;
    }
    let title_start = value.as_str()[source_end..]
        .find(|character: char| !character.is_whitespace())
        .map_or(source_end, |offset| source_end + offset);
    if title_start >= value.as_str().len() {
        emit_tag_diagnostic(
            diagnostics,
            raw_tag,
            DiagnosticCode::TagParseError,
            "@inheritSection requires a non-empty section title",
            value_span_for_range(&value, source_start, source_end, raw_tag.value_span),
        );
        return None;
    }
    let title_end = value.as_str().trim_end().len();
    let title = Spanned::new(
        super::model::MarkdownText::new(value.slice(TextRange::new(
            u32::try_from(title_start).expect("normalized text length fits u32"),
            u32::try_from(title_end).expect("normalized text length fits u32"),
        ))),
        value_span_for_range(&value, title_start, title_end, raw_tag.value_span),
    );
    Some(ParsedTag::InheritSection {
        target: classify_inherit_target(&value, source_start, source_end, raw_tag.value_span),
        title,
        origin,
    })
}

fn classify_inherit_target(
    value: &SourcedText,
    source_start: usize,
    source_end: usize,
    fallback: Span,
) -> InheritTarget {
    let span = value_span_for_range(value, source_start, source_end, fallback);
    let source = &value.as_str()[source_start..source_end];
    if source == "NULL" {
        InheritTarget::Suppress(span)
    } else {
        InheritTarget::Topic(Spanned::new(TopicRef(source.to_owned()), span))
    }
}

fn parse_inherit_field(value: &str) -> Option<InheritField> {
    Some(match value {
        "params" => InheritField::Params,
        "return" => InheritField::Return,
        "title" => InheritField::Title,
        "description" => InheritField::Description,
        "details" => InheritField::Details,
        "seealso" => InheritField::SeeAlso,
        "sections" => InheritField::Sections,
        "references" => InheritField::References,
        "examples" => InheritField::Examples,
        "author" => InheritField::Author,
        "source" => InheritField::Source,
        "note" => InheritField::Note,
        "format" => InheritField::Format,
        _ => return None,
    })
}

fn parse_arg_selector(value: &str, span: Span) -> Option<ArgSelector> {
    if let Some(name) = value.strip_prefix('-') {
        if is_r_name(name) {
            return Some(ArgSelector::Exclude(Spanned::new(
                ParamName(name.to_owned()),
                Span::new(
                    span.file,
                    crate::source::TextRange::new(span.range.start() + 1, span.range.end()),
                ),
            )));
        }
        return None;
    }
    is_r_name(value).then(|| ArgSelector::Name(Spanned::new(ParamName(value.to_owned()), span)))
}

fn is_r_name(value: &str) -> bool {
    if value == "..." {
        return true;
    }
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !(first.is_alphabetic() || first == '.') {
        return false;
    }
    if first == '.' && characters.clone().next().is_some_and(char::is_numeric) {
        return false;
    }
    characters.all(|character| character.is_alphanumeric() || matches!(character, '.' | '_'))
}

#[cfg(test)]
mod tests {
    use crate::diagnostic::DiagnosticCode;
    use crate::tags::test_support::parsed;
    use crate::tags::{InheritField, ParsedTag, UnknownTagPolicy};

    #[test]
    fn inherit_distinguishes_all_selected_and_empty_selections() {
        let (tags, diagnostics, _) = parsed(
            r"#' @inherit source
#' @inherit source params return unknown
#' @inherit source bogus
#' @inherit
",
            UnknownTagPolicy::Warn,
        );
        let ParsedTag::Inherit { fields, .. } = &tags[0] else {
            panic!("expected inherit");
        };
        assert!(matches!(fields, super::InheritFields::All { .. }));
        let ParsedTag::Inherit { fields, .. } = &tags[1] else {
            panic!("expected inherit with selected fields");
        };
        let super::InheritFields::Selected(fields) = fields else {
            panic!("expected selected fields");
        };
        assert_eq!(fields.len(), 2);
        assert!(matches!(fields[0].value, InheritField::Params));
        assert!(matches!(fields[1].value, InheritField::Return));
        let ParsedTag::Inherit { fields, .. } = &tags[2] else {
            panic!("expected inherit with no recognized fields");
        };
        assert!(matches!(
            fields,
            super::InheritFields::Selected(fields) if fields.is_empty()
        ));
        assert_eq!(tags.len(), 3);
        assert_eq!(diagnostics.len(), 3);
    }

    #[test]
    fn inherit_params_tail_is_retained_and_bare_source_survives() {
        let (tags, diagnostics, _) = parsed(
            r"#' @inheritParams pkg::fun
#' @inheritParams pkg::fun first -third
",
            UnknownTagPolicy::Warn,
        );
        let ParsedTag::InheritParams {
            target, selection, ..
        } = &tags[0]
        else {
            panic!("expected bare inheritParams");
        };
        let super::InheritTarget::Topic(source) = target else {
            panic!("expected topic target");
        };
        assert_eq!(source.value.0, "pkg::fun");
        assert!(selection.is_none());
        let ParsedTag::InheritParams {
            target, selection, ..
        } = &tags[1]
        else {
            panic!("expected selected inheritParams");
        };
        let super::InheritTarget::Topic(source) = target else {
            panic!("expected topic target");
        };
        assert_eq!(source.value.0, "pkg::fun");
        let selection = selection.as_ref().expect("selection");
        assert_eq!(selection.selectors.len(), 2);
        assert_eq!(tags.len(), 2);
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != DiagnosticCode::UnsupportedSelection)
        );
    }

    #[test]
    fn inherit_section_keeps_one_topic_and_the_source_backed_title() {
        let (tags, diagnostics, _) = parsed(
            r#"#' @inheritSection donor Display text
"#,
            UnknownTagPolicy::Warn,
        );
        let ParsedTag::InheritSection { target, title, .. } = &tags[0] else {
            panic!("expected inheritSection");
        };
        let super::InheritTarget::Topic(source) = target else {
            panic!("expected topic target");
        };
        assert_eq!(source.value.0, "donor");
        assert_eq!(title.value.as_str(), "Display text");
        assert!(!diagnostics.has_errors());
    }

    #[test]
    fn inherit_null_is_field_scoped_and_case_sensitive() {
        let (tags, diagnostics, _) = parsed(
            r#"#' @inherit NULL
#' @inherit NULL params
#' @inherit NULL bogus
#' @inherit NULL params bogus
#' @inherit null params
"#,
            UnknownTagPolicy::Warn,
        );
        let ParsedTag::Inherit { target, fields, .. } = &tags[0] else {
            panic!("expected bare inherit suppression");
        };
        let super::InheritTarget::Suppress(span) = target else {
            panic!("expected suppression target");
        };
        assert_eq!(span.range.len(), 4);
        assert!(matches!(fields, super::InheritFields::All { .. }));

        let ParsedTag::Inherit { target, fields, .. } = &tags[1] else {
            panic!("expected selected inherit suppression");
        };
        assert!(matches!(target, super::InheritTarget::Suppress(_)));
        assert!(matches!(
            fields,
            super::InheritFields::Selected(fields)
                if fields.len() == 1 && matches!(fields[0].value, InheritField::Params)
        ));

        let ParsedTag::Inherit { target, fields, .. } = &tags[2] else {
            panic!("expected suppression with unknown field");
        };
        assert!(matches!(target, super::InheritTarget::Suppress(_)));
        assert!(matches!(
            fields,
            super::InheritFields::Selected(fields) if fields.is_empty()
        ));

        let ParsedTag::Inherit { target, fields, .. } = &tags[3] else {
            panic!("expected suppression with mixed fields");
        };
        assert!(matches!(target, super::InheritTarget::Suppress(_)));
        assert!(matches!(
            fields,
            super::InheritFields::Selected(fields)
                if fields.len() == 1 && matches!(fields[0].value, InheritField::Params)
        ));

        let ParsedTag::Inherit { target, .. } = &tags[4] else {
            panic!("expected case-sensitive topic target");
        };
        let super::InheritTarget::Topic(source) = target else {
            panic!("expected topic target");
        };
        assert_eq!(source.value.0, "null");
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == DiagnosticCode::TagParseError)
                .count(),
            2
        );
    }

    #[test]
    fn inherit_params_null_retains_selection_without_unsupported_diagnostic() {
        let (tags, diagnostics, _) = parsed(
            r#"#' @inheritParams NULL
#' @inheritParams NULL x y
#' @inheritParams pkg::fun x y
"#,
            UnknownTagPolicy::Warn,
        );
        let ParsedTag::InheritParams {
            target, selection, ..
        } = &tags[0]
        else {
            panic!("expected bare inheritParams suppression");
        };
        assert!(matches!(target, super::InheritTarget::Suppress(_)));
        assert!(selection.is_none());

        let ParsedTag::InheritParams {
            target, selection, ..
        } = &tags[1]
        else {
            panic!("expected selected inheritParams suppression");
        };
        assert!(matches!(target, super::InheritTarget::Suppress(_)));
        assert_eq!(selection.as_ref().expect("selection").selectors.len(), 2);

        let ParsedTag::InheritParams {
            target, selection, ..
        } = &tags[2]
        else {
            panic!("expected selected inheritParams topic");
        };
        assert!(matches!(target, super::InheritTarget::Topic(_)));
        assert_eq!(selection.as_ref().expect("selection").selectors.len(), 2);
        assert!(
            !diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == DiagnosticCode::UnsupportedTag)
        );
    }

    #[test]
    fn unsupported_inherit_params_selectors_are_rejected_as_whole_requests() {
        for selector in ["1", "x:y", "(x)", "z-x", "--x", "\"x\"", "`x`"] {
            let source = format!(
                r#"#' @inheritParams donor {selector}
"#
            );
            let (tags, diagnostics, _) = parsed(&source, UnknownTagPolicy::Warn);
            assert!(tags.is_empty(), "selector `{selector}` was retained");
            let diagnostic = diagnostics
                .iter()
                .find(|diagnostic| diagnostic.code == DiagnosticCode::UnsupportedSelection)
                .unwrap_or_else(|| panic!("selector `{selector}` was not diagnosed"));
            assert_eq!(diagnostic.severity, crate::diagnostic::Severity::Error);
            assert!(diagnostic.message.contains(selector));
            assert!(diagnostic.message.contains("name") && diagnostic.message.contains("-name"));
        }
    }
}
