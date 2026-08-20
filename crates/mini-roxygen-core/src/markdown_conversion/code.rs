//! Inline code classification and executable R Markdown detection.

use rd_ast::{RdDocument, RdNode, RdTag};

use crate::arity_adapter::can_parse_r;

pub(super) const ROXYGEN_SPECIAL_CODE_TOKENS: [&str; 37] = [
    "-", ":", "::", ":::", "!", "!=", "(", "[", "[[", "@", "*", "/", "&", "&&", "%*%", "%/%", "%%",
    "%in%", "%o%", "%x%", "^", "+", "<", "<=", "=", "==", ">", ">=", "|", "||", "~", "$", "for",
    "function", "if", "repeat", "while",
];

pub(super) fn classify_code(code: &str) -> (RdTag, RdNode) {
    if is_inline_r(code) {
        (RdTag::Verb, RdNode::Verb(code.to_owned()))
    } else if can_parse_r(code) || ROXYGEN_SPECIAL_CODE_TOKENS.contains(&code) {
        let node = RdNode::RCode(code.to_owned());
        let tagged = RdNode::tagged(RdTag::Code, None, vec![node.clone()]);
        // arity-parser deliberately accepts a few incomplete lexical states,
        // including an unterminated R string. Such a value cannot be emitted
        // in a Code node because rd-writer must close the R-like state before
        // the macro's closing brace. Keep the source visible as verbatim code
        // instead of allowing a user-provided span to make conversion panic.
        if rd_writer::write_document(&RdDocument::from(vec![tagged])).is_ok() {
            (RdTag::Code, node)
        } else {
            (RdTag::Verb, RdNode::Verb(code.to_owned()))
        }
    } else {
        (RdTag::Verb, RdNode::Verb(code.to_owned()))
    }
}

pub(super) fn is_inline_r(code: &str) -> bool {
    code.starts_with("r ") || code.starts_with("Rd ")
}

pub(super) fn is_executable_r_chunk(info: &str) -> bool {
    info.strip_prefix("{r")
        .is_some_and(|rest| matches!(rest.as_bytes().first(), Some(b'}' | b',' | b' ')))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::super::convert_markdown as convert_markdown_with_context;
    use super::super::test_support::{assert_serialized_body, context, value};
    use crate::tags::MarkdownText;

    fn convert_markdown(value: &MarkdownText) -> super::super::MarkdownConversion {
        convert_markdown_with_context(value, &context())
    }

    use rd_ast::RdTag;

    #[test]
    fn parseable_inline_code_is_code_with_r_code_leaf() {
        let conversion = convert_markdown(&value("`x + 1`"));
        assert_eq!(
            conversion.fragment.nodes,
            vec![rd_ast::RdNode::tagged(
                RdTag::Code,
                None,
                vec![rd_ast::RdNode::RCode("x + 1".into())],
            )]
        );
        assert!(conversion.diagnostics.is_empty());
        assert_serialized_body(conversion.fragment.nodes, r"\code{x + 1}");
    }

    #[test]
    fn a_percent_in_inline_code_cannot_swallow_its_closing_brace() {
        // The closing brace sits on the same line here, so an unescaped `%`
        // would comment it out and R would reject the topic. That makes the
        // oracle load-bearing for this case rather than incidental.
        let conversion = convert_markdown(&value("`a %in% b`"));
        assert_serialized_body(conversion.fragment.nodes, r"\code{a \%in\% b}");
    }

    #[test]
    fn unparseable_inline_code_is_verb() {
        let conversion = convert_markdown(&value("`not valid R %`"));
        assert_eq!(
            conversion.fragment.nodes,
            vec![rd_ast::RdNode::tagged(
                RdTag::Verb,
                None,
                vec![rd_ast::RdNode::Verb("not valid R %".into())],
            )]
        );
        assert!(conversion.diagnostics.is_empty());
        assert_serialized_body(conversion.fragment.nodes, r"\verb{not valid R \%}");
    }

    #[test]
    fn writer_invalid_r_code_span_falls_back_to_verbatim() {
        let conversion = convert_markdown(&value(r#"`"unterminated`"#));

        assert_eq!(
            conversion.fragment.nodes,
            vec![rd_ast::RdNode::tagged(
                RdTag::Verb,
                None,
                vec![rd_ast::RdNode::Verb(r#""unterminated"#.into())],
            )]
        );
        assert!(conversion.diagnostics.is_empty());
        let body = super::super::test_support::serialize(conversion.fragment.nodes);
        insta::assert_snapshot!(body, @r#"\verb{"unterminated}"#);
        crate::rd_oracle::assert_r_accepts(&crate::rd_oracle::minimal_topic(&body));
    }

    #[test]
    fn single_expression_rule_selects_code_or_verb() {
        let cases = [
            ("x; y", RdTag::Verb),
            ("_x", RdTag::Verb),
            ("x;", RdTag::Code),
            ("x + 1", RdTag::Code),
            ("x |> f(_)", RdTag::Code),
            ("# note", RdTag::Verb),
            (" ", RdTag::Verb),
        ];

        for (code, tag) in cases {
            let conversion = convert_markdown(&value(&format!("`{code}`")));
            assert_eq!(
                conversion.fragment.nodes[0]
                    .as_tagged()
                    .map(|tagged| tagged.tag()),
                Some(&tag),
                "classifying {code:?}"
            );
            assert!(conversion.diagnostics.is_empty());
        }
    }

    #[test]
    fn trailing_semicolon_verbatim_output_is_serialized_as_rd() {
        let conversion = convert_markdown(&value("`x; y`"));
        assert_serialized_body(conversion.fragment.nodes, r"\verb{x; y}");
    }

    #[test]
    fn special_inline_tokens_are_code() {
        for token in ["+", "if", "%in%", "[["] {
            let conversion = convert_markdown(&value(&format!("`{token}`")));
            assert_eq!(
                conversion.fragment.nodes,
                vec![rd_ast::RdNode::tagged(
                    RdTag::Code,
                    None,
                    vec![rd_ast::RdNode::RCode(token.into())],
                )],
                "classifying {token:?}"
            );
            assert!(conversion.diagnostics.is_empty());
        }
    }

    #[test]
    fn inline_evaluation_forms_are_diagnosed_and_recovered_as_verb() {
        for code in ["r 1 + 1", "Rd foo()"] {
            let conversion = convert_markdown(&value(&format!("`{code}`")));
            assert_eq!(
                conversion.fragment.nodes,
                vec![rd_ast::RdNode::tagged(
                    RdTag::Verb,
                    None,
                    vec![rd_ast::RdNode::Verb(code.into())],
                )]
            );
            assert_eq!(conversion.diagnostics.len(), 1);
            assert_eq!(
                conversion
                    .diagnostics
                    .iter()
                    .next()
                    .expect("inline evaluation diagnostic")
                    .code,
                crate::diagnostic::DiagnosticCode::UnsupportedInlineR
            );
        }
    }

    #[test]
    fn an_undefined_inline_r_expression_reports_the_complete_code_span() {
        let substitutions =
            crate::inline_r::InlineRSubstitutions::builtins().expect("built-ins should validate");
        let usage = crate::inline_r::InlineRUsage::new();
        let session = crate::inline_r::InlineRSession::new(&substitutions, &usage);
        let mut context = super::super::test_support::context();
        context.inline_r_session = Some(&session);
        let source = format!("before {}r other(){}", char::from(96), char::from(96));
        let conversion =
            super::super::convert_markdown(&super::super::test_support::value(&source), &context);
        let diagnostic = conversion
            .diagnostics
            .iter()
            .next()
            .expect("undefined substitution diagnostic");
        assert_eq!(
            diagnostic.code,
            crate::diagnostic::DiagnosticCode::UndefinedInlineRSubstitution
        );
        assert_eq!(
            diagnostic.primary.span.range,
            crate::source::TextRange::new(7, 18)
        );
    }

    #[test]
    fn bare_lifecycle_badge_is_not_a_builtin_substitution() {
        let substitutions =
            crate::inline_r::InlineRSubstitutions::builtins().expect("built-ins should validate");
        let usage = crate::inline_r::InlineRUsage::new();
        let session = crate::inline_r::InlineRSession::new(&substitutions, &usage);
        let mut context = super::super::test_support::context();
        context.inline_r_session = Some(&session);
        let conversion = super::super::convert_markdown(
            &super::super::test_support::value(r#"`r badge("experimental")`"#),
            &context,
        );

        assert_eq!(conversion.diagnostics.len(), 1);
        assert_eq!(
            conversion
                .diagnostics
                .iter()
                .next()
                .expect("undefined substitution diagnostic")
                .code,
            crate::diagnostic::DiagnosticCode::UndefinedInlineRSubstitution
        );
    }

    #[test]
    fn unknown_lifecycle_badge_stage_is_not_a_builtin_substitution() {
        let substitutions =
            crate::inline_r::InlineRSubstitutions::builtins().expect("built-ins should validate");
        let usage = crate::inline_r::InlineRUsage::new();
        let session = crate::inline_r::InlineRSession::new(&substitutions, &usage);
        let mut context = super::super::test_support::context();
        context.inline_r_session = Some(&session);
        let conversion = super::super::convert_markdown(
            &super::super::test_support::value(r#"`r lifecycle::badge("archived")`"#),
            &context,
        );

        assert_eq!(conversion.diagnostics.len(), 1);
        assert_eq!(
            conversion
                .diagnostics
                .iter()
                .next()
                .expect("undefined substitution diagnostic")
                .code,
            crate::diagnostic::DiagnosticCode::UndefinedInlineRSubstitution
        );
    }

    #[test]
    fn a_user_entry_can_enable_the_bare_lifecycle_badge_spelling() {
        let substitutions = crate::inline_r::InlineRSubstitutions::from_user_entries(
            BTreeMap::from([(
                r#"badge("experimental")"#.to_owned(),
                r#"\strong{custom badge}"#.to_owned(),
            )]),
            Some("mini-roxygen.toml".to_owned()),
        )
        .expect("configuration should validate");
        let usage = crate::inline_r::InlineRUsage::new();
        let session = crate::inline_r::InlineRSession::new(&substitutions, &usage);
        let mut context = super::super::test_support::context();
        context.inline_r_session = Some(&session);
        let conversion = super::super::convert_markdown(
            &super::super::test_support::value(r#"`r badge("experimental")`"#),
            &context,
        );

        assert!(conversion.diagnostics.is_empty());
        assert_serialized_body(conversion.fragment.nodes, r#"\strong{custom badge}"#);
    }

    #[test]
    fn a_builtin_lifecycle_badge_is_accepted_by_the_rd_oracle() {
        let substitutions =
            crate::inline_r::InlineRSubstitutions::builtins().expect("built-ins should validate");
        let usage = crate::inline_r::InlineRUsage::new();
        let session = crate::inline_r::InlineRSession::new(&substitutions, &usage);
        let mut context = super::super::test_support::context();
        context.inline_r_session = Some(&session);
        let conversion = super::super::convert_markdown(
            &super::super::test_support::value(r#"`r lifecycle::badge("experimental")`"#),
            &context,
        );

        assert!(conversion.diagnostics.is_empty());
        assert_serialized_body(
            conversion.fragment.nodes,
            r#"\ifelse{html}{\href{https://lifecycle.r-lib.org/articles/stages.html#experimental}{\figure{lifecycle-experimental.svg}{options: alt='[Experimental]'}}}{\strong{[Experimental]}}"#,
        );
    }

    #[test]
    fn a_configured_expression_matches_exactly_and_repeats() {
        let substitutions = crate::inline_r::InlineRSubstitutions::from_user_entries(
            BTreeMap::from([("known()".to_owned(), r#"\strong{known}"#.to_owned())]),
            Some("configuration label".to_owned()),
        )
        .expect("configuration should validate");
        let usage = crate::inline_r::InlineRUsage::new();
        let session = crate::inline_r::InlineRSession::new(&substitutions, &usage);
        let mut context = super::super::test_support::context();
        context.inline_r_session = Some(&session);
        let source = format!(
            "{}r known(){} and {}r known(){} but {}r known ( ){}",
            char::from(96),
            char::from(96),
            char::from(96),
            char::from(96),
            char::from(96),
            char::from(96)
        );
        let conversion =
            super::super::convert_markdown(&super::super::test_support::value(&source), &context);
        assert_eq!(
            conversion
                .diagnostics
                .iter()
                .filter(|diagnostic| {
                    diagnostic.code
                        == crate::diagnostic::DiagnosticCode::UndefinedInlineRSubstitution
                })
                .count(),
            1
        );
        let output = super::super::test_support::serialize(conversion.fragment.nodes);
        assert_eq!(output.matches(r"\strong{known}").count(), 2);
    }

    #[test]
    fn multiline_inline_r_does_not_match_a_normalized_substitution_key() {
        let substitutions = crate::inline_r::InlineRSubstitutions::from_user_entries(
            BTreeMap::from([("a b".to_owned(), r#"\strong{matched}"#.to_owned())]),
            Some("configuration label".to_owned()),
        )
        .expect("configuration should validate");
        let usage = crate::inline_r::InlineRUsage::new();
        let session = crate::inline_r::InlineRSession::new(&substitutions, &usage);
        let mut context = super::super::test_support::context();
        context.inline_r_session = Some(&session);
        let conversion = super::super::convert_markdown(
            &super::super::test_support::value("`r \"a\nb\"`"),
            &context,
        );

        assert_serialized_body(conversion.fragment.nodes, r#"\verb{r "a b"}"#);
        let diagnostics = conversion
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == crate::diagnostic::DiagnosticCode::UnsupportedInlineR
            })
            .collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].message,
            "multi-line inline R code is not supported"
        );
    }

    #[test]
    fn commonmark_outer_space_normalization_still_matches_an_inline_r_key() {
        let substitutions = crate::inline_r::InlineRSubstitutions::from_user_entries(
            BTreeMap::from([("known()".to_owned(), r#"\strong{known}"#.to_owned())]),
            Some("configuration label".to_owned()),
        )
        .expect("configuration should validate");
        let usage = crate::inline_r::InlineRUsage::new();
        let session = crate::inline_r::InlineRSession::new(&substitutions, &usage);
        let mut context = super::super::test_support::context();
        context.inline_r_session = Some(&session);
        let conversion = super::super::convert_markdown(
            &super::super::test_support::value("` r known() `"),
            &context,
        );

        assert_serialized_body(conversion.fragment.nodes, r#"\strong{known}"#);
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn multi_backtick_single_line_inline_r_with_a_backtick_uses_substitution() {
        let substitutions = crate::inline_r::InlineRSubstitutions::from_user_entries(
            BTreeMap::from([("paste(\"`\")".to_owned(), r#"\strong{backtick}"#.to_owned())]),
            Some("configuration label".to_owned()),
        )
        .expect("configuration should validate");
        let usage = crate::inline_r::InlineRUsage::new();
        let session = crate::inline_r::InlineRSession::new(&substitutions, &usage);
        let mut context = super::super::test_support::context();
        context.inline_r_session = Some(&session);
        let conversion = super::super::convert_markdown(
            &super::super::test_support::value(r#"``r paste("`")``"#),
            &context,
        );

        assert_serialized_body(conversion.fragment.nodes, r#"\strong{backtick}"#);
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn multiline_code_without_inline_r_marker_has_no_new_diagnostic() {
        let conversion = super::super::convert_markdown(
            &super::super::test_support::value("`a\nb`"),
            &super::super::test_support::context(),
        );

        assert_serialized_body(conversion.fragment.nodes, r#"\verb{a b}"#);
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn fenced_code_preserves_body_and_serializes() {
        let conversion = convert_markdown(&value("```\nline 1\n\n{ braces } \\\\ path 50%\n```"));
        assert_eq!(
            conversion.fragment.nodes,
            vec![rd_ast::RdNode::tagged(
                RdTag::Preformatted,
                None,
                vec![
                    rd_ast::RdNode::Verb("line 1\n".into()),
                    rd_ast::RdNode::Verb("\n".into()),
                    rd_ast::RdNode::Verb("{ braces } \\\\ path 50%\n".into()),
                ],
            )]
        );
        assert!(conversion.diagnostics.is_empty());
        assert_serialized_body(
            conversion.fragment.nodes,
            "\\preformatted{line 1\n\n\\{ braces \\} \\\\\\\\ path 50\\%\n}",
        );
    }

    #[test]
    fn indented_code_has_the_same_shape_as_fenced_code() {
        let fenced = convert_markdown(&value("```\nline 1\n\nline 2\n```"));
        let indented = convert_markdown(&value("    line 1\n\n    line 2\n"));
        assert_eq!(indented.fragment.nodes, fenced.fragment.nodes);
        assert!(indented.diagnostics.is_empty());
        assert_serialized_body(
            indented.fragment.nodes,
            "\\preformatted{line 1\n\nline 2\n}",
        );
    }

    #[test]
    fn executable_r_chunk_is_not_evaluated() {
        let conversion = convert_markdown(&value("```{r}\n1 + 1\n```"));
        assert_eq!(
            conversion.fragment.nodes,
            vec![rd_ast::RdNode::tagged(
                RdTag::Preformatted,
                None,
                vec![rd_ast::RdNode::Verb("1 + 1\n".into())],
            )]
        );
        assert_eq!(conversion.diagnostics.len(), 1);
        assert_eq!(
            conversion
                .diagnostics
                .iter()
                .next()
                .expect("chunk diagnostic")
                .code,
            crate::diagnostic::DiagnosticCode::UnsupportedInlineR
        );
    }

    #[test]
    fn inline_code_nests_inside_emphasis_without_adjacent_leaves() {
        let conversion = convert_markdown(&value("*before `x` after*"));
        assert_eq!(
            conversion.fragment.nodes,
            vec![rd_ast::RdNode::tagged(
                RdTag::Emph,
                None,
                vec![
                    rd_ast::RdNode::Text("before ".into()),
                    rd_ast::RdNode::tagged(
                        RdTag::Code,
                        None,
                        vec![rd_ast::RdNode::RCode("x".into())],
                    ),
                    rd_ast::RdNode::Text(" after".into()),
                ],
            )]
        );
    }
}
