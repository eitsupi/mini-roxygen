//! Parsing for the semantic @param tag.

use super::diagnostics::{emit_tag_diagnostic, value_span, value_span_for_range};
use super::model::{MarkdownText, ParamName, ParsedTag, TagOrigin};
use super::text::SourcedText;
use super::trim_outer;
use crate::arity_adapter::RawTag;
use crate::diagnostic::{DiagnosticCode, Diagnostics};
use crate::source::{Spanned, TextRange};

fn skip_whitespace(value: &str, start: usize) -> usize {
    value[start..]
        .char_indices()
        .find_map(|(offset, character)| (!character.is_whitespace()).then_some(start + offset))
        .unwrap_or(value.len())
}

pub(super) fn parse_param(
    raw_tag: &RawTag,
    value: SourcedText,
    origin: TagOrigin,
    diagnostics: &mut Diagnostics,
) -> Option<ParsedTag> {
    let mut cursor = skip_whitespace(value.as_str(), 0);
    let mut names = Vec::new();
    let mut description_start = None;

    while cursor < value.as_str().len() {
        cursor = skip_whitespace(value.as_str(), cursor);
        if cursor >= value.as_str().len() {
            break;
        }

        if value.as_str().as_bytes()[cursor] == b',' {
            emit_tag_diagnostic(
                diagnostics,
                raw_tag,
                DiagnosticCode::TagParseError,
                "@param contains an empty parameter name",
                value_span_for_range(&value, cursor, cursor + 1, raw_tag.value_span),
            );
            cursor += 1;
            continue;
        }

        let token_start = cursor;
        let (token_end, content_start, content_end, quoted) =
            if value.as_str().as_bytes()[cursor] == b'`' {
                let Some(close) = value.as_str()[cursor + 1..].find('`') else {
                    emit_tag_diagnostic(
                        diagnostics,
                        raw_tag,
                        DiagnosticCode::TagParseError,
                        "@param has an unterminated backtick-quoted name",
                        value_span_for_range(
                            &value,
                            token_start,
                            value.as_str().len(),
                            raw_tag.value_span,
                        ),
                    );
                    return None;
                };
                let close = cursor + 1 + close;
                (close + 1, cursor + 1, close, true)
            } else {
                let end = value.as_str()[cursor..]
                    .char_indices()
                    .find_map(|(offset, character)| {
                        (character.is_whitespace() || character == ',').then_some(cursor + offset)
                    })
                    .unwrap_or(value.as_str().len());
                (end, token_start, end, false)
            };

        if quoted
            && token_end < value.as_str().len()
            && !value.as_str()[token_end..]
                .chars()
                .next()
                .is_some_and(char::is_whitespace)
            && value.as_str().as_bytes()[token_end] != b','
        {
            emit_tag_diagnostic(
                diagnostics,
                raw_tag,
                DiagnosticCode::TagParseError,
                "@param name must be separated from its description",
                value_span_for_range(&value, token_start, token_end, raw_tag.value_span),
            );
            return None;
        }

        let name = &value.as_str()[content_start..content_end];
        if name.trim().is_empty() {
            emit_tag_diagnostic(
                diagnostics,
                raw_tag,
                DiagnosticCode::TagParseError,
                "@param contains an empty parameter name",
                value_span_for_range(&value, token_start, token_end, raw_tag.value_span),
            );
        } else {
            names.push(Spanned::new(
                ParamName(name.to_owned()),
                value_span_for_range(&value, token_start, token_end, raw_tag.value_span),
            ));
        }

        let after_token = skip_whitespace(value.as_str(), token_end);
        if after_token < value.as_str().len() && value.as_str().as_bytes()[after_token] == b',' {
            cursor = after_token + 1;
            if cursor >= value.as_str().len() {
                emit_tag_diagnostic(
                    diagnostics,
                    raw_tag,
                    DiagnosticCode::TagParseError,
                    "@param has a trailing comma without a parameter name",
                    value_span_for_range(&value, after_token, cursor, raw_tag.value_span),
                );
            }
            continue;
        }
        description_start = (after_token < value.as_str().len()).then_some(after_token);
        break;
    }

    if names.is_empty() {
        emit_tag_diagnostic(
            diagnostics,
            raw_tag,
            DiagnosticCode::TagParseError,
            "@param requires at least one parameter name",
            value_span(&value, raw_tag.value_span),
        );
        return None;
    }

    let Some(description_start) = description_start else {
        emit_tag_diagnostic(
            diagnostics,
            raw_tag,
            DiagnosticCode::TagParseError,
            "@param requires a description",
            value_span(&value, raw_tag.value_span),
        );
        return None;
    };
    let description = trim_outer(value.slice(TextRange::new(
        u32::try_from(description_start).expect("normalized text length fits u32"),
        u32::try_from(value.as_str().len()).expect("normalized text length fits u32"),
    )))
    .trim_continuation_indent();
    if description.is_empty() {
        emit_tag_diagnostic(
            diagnostics,
            raw_tag,
            DiagnosticCode::TagParseError,
            "@param requires a description",
            value_span(&value, raw_tag.value_span),
        );
        return None;
    }

    Some(ParsedTag::Param {
        names,
        description: MarkdownText::new(description),
        origin,
    })
}

#[cfg(test)]
mod tests {
    use crate::diagnostic::DiagnosticCode;
    use crate::tags::test_support::parsed;
    use crate::tags::{ParsedTag, UnknownTagPolicy};

    #[test]
    fn param_splits_names_and_retains_individual_spans() {
        let (tags, diagnostics, source) = parsed(
            r"#' @param x,y A description.
",
            UnknownTagPolicy::Warn,
        );
        let ParsedTag::Param {
            names, description, ..
        } = &tags[0]
        else {
            panic!("expected param");
        };
        assert_eq!(
            names.iter().map(|name| &name.value.0).collect::<Vec<_>>(),
            ["x", "y"]
        );
        assert_eq!(description.as_str(), "A description.");
        assert_eq!(source.text_range(names[0].span.range), Some("x"));
        assert_eq!(source.text_range(names[1].span.range), Some("y"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn param_description_dedents_hanging_continuations() {
        let (tags, diagnostics, _) = parsed(
            r#"#' @param strict `r lifecycle::badge("deprecated")` Use `how =
#'   "horizontal_extend"`.
"#,
            UnknownTagPolicy::Warn,
        );
        let ParsedTag::Param { description, .. } = &tags[0] else {
            panic!("expected parameter tag");
        };
        assert_eq!(
            description.as_str(),
            "`r lifecycle::badge(\"deprecated\")` Use `how =\n\"horizontal_extend\"`."
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn param_description_keeps_relative_list_fence_and_hard_break_indentation() {
        let source = concat!(
            "#' @param x First line",
            "  \n",
            "#'   second line\n",
            "#'   ```\n",
            "#'     code\n",
            "#'   ```\n",
            "#'   - outer\n",
            "#'     - inner\n",
        );
        let (tags, diagnostics, _) = parsed(source, UnknownTagPolicy::Warn);
        let ParsedTag::Param { description, .. } = &tags[0] else {
            panic!("expected parameter tag");
        };
        assert_eq!(
            description.as_str(),
            "First line  \nsecond line\n```\n  code\n```\n- outer\n  - inner"
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn param_accepts_spaces_around_comma_and_backtick_names() {
        let (tags, diagnostics, _) = parsed(
            r"#' @param x, y Description.
#' @param `arg one` Backtick description.
#' @param `x,y` Comma description.
",
            UnknownTagPolicy::Warn,
        );
        let ParsedTag::Param { names, .. } = &tags[0] else {
            panic!("expected comma-separated param");
        };
        assert_eq!(
            names.iter().map(|name| &name.value.0).collect::<Vec<_>>(),
            ["x", "y"]
        );
        let ParsedTag::Param { names, .. } = &tags[1] else {
            panic!("expected quoted param");
        };
        assert_eq!(names[0].value.0, "arg one");
        let ParsedTag::Param { names, .. } = &tags[2] else {
            panic!("expected quoted comma param");
        };
        assert_eq!(names[0].value.0, "x,y");
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn malformed_params_diagnose_without_empty_names() {
        for input in [
            r#"#' @param x,,y Description.
"#,
            r#"#' @param x,
"#,
            r#"#' @param    
"#,
            r#"#' @param x
"#,
            r#"#' @param
"#,
        ] {
            let (tags, diagnostics, _) = parsed(input, UnknownTagPolicy::Warn);
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| { diagnostic.code == DiagnosticCode::TagParseError })
            );
            if input.contains("x,,y") {
                let ParsedTag::Param { names, .. } = &tags[0] else {
                    panic!("valid names should survive a malformed comma piece");
                };
                assert_eq!(names.len(), 2);
                assert!(names.iter().all(|name| !name.value.0.is_empty()));
            }
        }
    }
}
