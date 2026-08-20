//! R-expression classification for inline Markdown code.

use arity_parser::ast::Expr;
use arity_parser::parser::{
    ParseOptions, has_r_invalid_name, is_single_expression, parse_with_options,
};
use arity_parser::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

use super::raw_string::is_raw_string_spelling;

/// Reports whether `text` parses as the single expression roxygen2 expects.
///
/// Inline code is classified with `arity-parser` rather than R itself. This
/// requires one top-level expression and screens the underscore-leading and
/// empty-backquoted name shapes R rejects, but it does not reproduce every
/// lexical check R performs. A malformed string escape or numeric literal may
/// still be classified as code, and an unquoted non-ASCII identifier R accepts
/// may be classified as verbatim. Both render as inline code; the distinction
/// affects Rd escaping and structural consumers.
pub(crate) fn can_parse_r(text: &str) -> bool {
    let parsed = parse_with_options(text, &ParseOptions::default());
    is_single_expression(&parsed)
}

/// Reports whether `text` parses as a complete R source fragment.
///
/// Unlike [`can_parse_r`], this accepts multiple top-level expressions. It
/// rejects parser diagnostics and the invalid-name spellings screened by
/// [`arity_parser::parser::has_r_invalid_name`].
pub(crate) fn can_parse_r_source(text: &str) -> bool {
    let parsed = parse_with_options(text, &ParseOptions::default());
    parsed.diagnostics.is_empty() && !has_r_invalid_name(&parsed.cst)
}

/// Reports whether `text` is a syntactically valid NAMESPACE source fragment.
///
/// `base::parseNamespaceFile()` accepts a narrower language than R source: its
/// top-level expressions must be known directive calls, or one of the control
/// forms it evaluates while reading a NAMESPACE file. Keep this check here so
/// the namespace layer does not depend on arity's CST types.
pub(crate) fn can_parse_namespace_source(text: &str) -> bool {
    if !can_parse_r_source(text) {
        return false;
    }
    let parsed = parse_with_options(text, &ParseOptions::default());
    significant_elements(&parsed.cst)
        .into_iter()
        .all(valid_namespace_element)
}

const NAMESPACE_CALLS: &[&str] = &[
    "export",
    "exportPattern",
    "exportClassPattern",
    "exportClass",
    "exportClasses",
    "exportMethods",
    "import",
    "importFrom",
    "importClassFrom",
    "importClassesFrom",
    "importMethodsFrom",
    "useDynLib",
    "S3method",
];

fn valid_namespace_element(element: SyntaxElement) -> bool {
    let Some(expression) = Expr::cast(element) else {
        return false;
    };
    match expression {
        Expr::Call(call) => {
            let Some(name) = call.callee_name() else {
                return false;
            };
            // A namespace-qualified call is a computed directive head from
            // parseNamespaceFile's perspective, even when its member name is
            // one of the known directive names.
            if !matches!(call.base(), Some(SyntaxElement::Token(token)) if matches!(
                token.kind(),
                SyntaxKind::IDENT | SyntaxKind::STRING
            )) {
                return false;
            }
            NAMESPACE_CALLS.contains(&name.as_str())
                && match name.as_str() {
                    "importClassFrom" | "importClassesFrom" | "importMethodsFrom" => call
                        .arg_list()
                        .is_some_and(|args| args.args().next().is_some()),
                    "S3method" => matches!(call.arg_list(), Some(args) if {
                        let count = args.args().count();
                        count == 2 || count == 3
                    }),
                    _ => true,
                }
        }
        Expr::Block(block) => block.statements().all(valid_namespace_element),
        Expr::If(if_expr) => {
            let Some(then_elements) = if_expr.then_elements() else {
                return false;
            };
            valid_single_branch(&then_elements)
                && if_expr
                    .else_elements()
                    .is_none_or(|elements| valid_single_branch(&elements))
        }
        Expr::Assignment(assignment) => {
            matches!(
                assignment.op_kind(),
                Some(SyntaxKind::ASSIGN_EQ | SyntaxKind::ASSIGN_LEFT)
            ) && assignment
                .value_element()
                .is_some_and(valid_namespace_element)
        }
        _ => false,
    }
}

fn valid_single_branch(elements: &[SyntaxElement]) -> bool {
    let mut significant = elements.iter().filter(|element| !is_trivia(element.kind()));
    let Some(element) = significant.next().cloned() else {
        return false;
    };
    significant.next().is_none() && valid_namespace_element(element)
}

fn significant_elements(node: &SyntaxNode) -> Vec<SyntaxElement> {
    node.children_with_tokens()
        .filter(|element| !is_trivia(element.kind()))
        .collect()
}

fn is_trivia(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT | SyntaxKind::SEMICOLON
    )
}

/// Splits R-like source at newlines outside complete raw-string tokens.
///
/// Parser diagnostics are intentionally ignored: the lossless CST still
/// supplies the complete tokens that can be recognized, while validity of
/// the usage itself remains the caller's responsibility.
pub(crate) fn r_code_chunks(value: &str) -> Vec<String> {
    let parsed = parse_with_options(value, &ParseOptions::default());
    let mut raw_ranges = parsed
        .cst
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| token.kind() == SyntaxKind::STRING)
        .filter_map(|token| {
            is_raw_string_spelling(token.text()).then(|| {
                (
                    usize::from(token.text_range().start()),
                    usize::from(token.text_range().end()),
                )
            })
        })
        .collect::<Vec<_>>();
    raw_ranges.sort_unstable_by_key(|(start, _)| *start);

    let mut chunks = Vec::new();
    let mut chunk_start = 0;
    let mut raw_cursor = 0;
    for (newline, _) in value.match_indices('\n') {
        while raw_ranges
            .get(raw_cursor)
            .is_some_and(|(_, end)| *end <= newline)
        {
            raw_cursor += 1;
        }
        let inside_raw = raw_ranges
            .get(raw_cursor)
            .is_some_and(|(start, end)| *start <= newline && newline < *end);
        if !inside_raw {
            chunks.push(value[chunk_start..=newline].to_owned());
            chunk_start = newline + 1;
        }
    }
    if chunk_start < value.len() {
        chunks.push(value[chunk_start..].to_owned());
    }
    chunks
}

/// A token marker exposed to the raw Rd scanner without leaking arity's CST
/// types into the rest of mini-roxygen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RCodeMarkerKind {
    /// A bare backslash token emitted for R syntax or an Rd-shaped candidate.
    BareBackslash,
    /// An R structural opening brace.
    OpenBrace,
    /// An R structural closing brace.
    CloseBrace,
}

/// The source position and role of one R token relevant to raw Rd scanning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RCodeMarker {
    pub(crate) kind: RCodeMarkerKind,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

/// Returns only token markers that raw Rd scanning is allowed to interpret.
///
/// `arity-parser` deliberately exposes a wide, lossless CST. The scanner
/// therefore consumes a small positive contract: bare backslashes and
/// structural braces. Strings, comments, backtick names, roxygen content,
/// and future token kinds remain invisible without an exclusion list. A
/// lexical `ERROR` token is not one of these markers, while typed structural
/// tokens remain usable even when an `ERROR` recovery node contains them;
/// malformed regions are never reconstructed by falling back to a byte scan.
pub(crate) fn r_code_markers(value: &str) -> Vec<RCodeMarker> {
    let parsed = parse_with_options(value, &ParseOptions::default());
    let mut markers = Vec::new();
    for token in parsed
        .cst
        .descendants_with_tokens()
        .filter_map(SyntaxElement::into_token)
    {
        let kind = match token.kind() {
            SyntaxKind::FUNCTION_KW if token.text() == "\\" => RCodeMarkerKind::BareBackslash,
            SyntaxKind::LBRACE => RCodeMarkerKind::OpenBrace,
            SyntaxKind::RBRACE => RCodeMarkerKind::CloseBrace,
            _ => continue,
        };
        let range = token.text_range();
        markers.push(RCodeMarker {
            kind,
            start: usize::from(range.start()),
            end: usize::from(range.end()),
        });
    }
    markers.sort_unstable_by_key(|marker| (marker.start, marker.end));
    markers
}

#[cfg(test)]
mod tests {
    use arity_parser::parser::{ParseOptions, parse_with_options};
    use arity_parser::syntax::{SyntaxElement, SyntaxKind};

    use super::{
        RCodeMarkerKind, can_parse_namespace_source, can_parse_r, can_parse_r_source,
        r_code_markers,
    };

    #[test]
    fn namespace_source_requires_directive_structure() {
        for text in [
            "export(foo)",
            "export(foo)\nimportFrom(stats, median)",
            "export(foo); importFrom(stats, median)",
            "if (TRUE) export(foo) else { import(pkg) }",
            "{ export(foo); if (TRUE) import(pkg) }",
            "target <- export(foo)",
        ] {
            assert!(can_parse_namespace_source(text), "expected {text:?}");
        }

        for text in [
            "a + b",
            "unknownDirective(foo)",
            "export",
            "if (TRUE) unknownDirective(foo)",
            "S3method(foo)",
            "importClassFrom()",
            "importClassesFrom()",
            "importMethodsFrom()",
        ] {
            assert!(
                !can_parse_namespace_source(text),
                "expected {text:?} to fail"
            );
        }

        for text in [
            "importClassFrom(pkg)",
            "importClassesFrom(pkg)",
            "importMethodsFrom(pkg)",
        ] {
            assert!(can_parse_namespace_source(text), "expected {text:?}");
        }
    }

    #[test]
    fn can_parse_r_source_accepts_multiple_expressions() {
        assert!(can_parse_r_source("first <- 1\nsecond <- 2\n"));
        assert!(!can_parse_r_source("a +"));
        assert!(!can_parse_r_source("_x <- 1"));
    }

    #[test]
    fn known_divergences_from_r_are_pinned() {
        // Measured against R 4.6.1 with rlang. These are the cases where the
        // static parse and R disagree, kept here so a future change to the
        // classifier makes the movement visible rather than silent. Closing
        // any of them would mean reimplementing part of R's lexer, which this
        // classifier deliberately does not do.

        // R rejects these; we accept them. arity's lexer does not validate
        // pipe-placeholder position, numeric literal shape, string escapes, or
        // literal termination.
        for text in [
            "x |> f(_)",
            "_ |> f()",
            "0x",
            "0x1p",
            r#""\q""#,
            r#""unterminated"#,
            "`unterminated",
        ] {
            assert!(can_parse_r(text), "expected {text:?} to still be accepted");
        }

        // arity follows R's locale-aware identifier rule for Unicode letters.
        // Keep this explicit at the mini-roxygen boundary: non-ASCII code is
        // still code, not prose that needs Rd escaping.
        assert!(can_parse_r("café"));
    }

    #[test]
    fn can_parse_r_matches_single_expression_cases() {
        for text in ["x + 1", "f(1)", "a <- 1", "x;", "x ;"] {
            assert!(can_parse_r(text), "expected {text:?} to be code");
        }

        for text in [
            "x; y",
            ";x",
            "x;;",
            "_x",
            "_",
            "``",
            "",
            " \t\n",
            "# note",
            "not valid R %",
        ] {
            assert!(!can_parse_r(text), "expected {text:?} to be verbatim");
        }
    }

    #[test]
    fn unterminated_percent_does_not_hide_following_raw_rd_markers() {
        let value = r#"x %
\dontrun{hidden}
"#;
        let markers = super::r_code_markers(value);
        let backslash = value.find('\\').expect("raw Rd marker");
        assert!(
            markers.iter().any(|marker| {
                marker.kind == super::RCodeMarkerKind::BareBackslash && marker.start == backslash
            }),
            "the following roxygen line must remain visible: {markers:?}"
        );
        assert_eq!(
            markers
                .iter()
                .filter(|marker| marker.start >= backslash)
                .map(|marker| marker.kind)
                .collect::<Vec<_>>(),
            vec![
                super::RCodeMarkerKind::BareBackslash,
                super::RCodeMarkerKind::OpenBrace,
                super::RCodeMarkerKind::CloseBrace,
            ]
        );
    }

    #[test]
    fn markers_expose_only_lambda_and_structural_brace_tokens() {
        let value = r#"function(x) {}
\dontrun{bare}
\(x)
"quoted \{ text }"
# comment \{ text }
`backtick \{ text }`
#' \dontrun{roxygen}
"#;
        let markers = r_code_markers(value);
        assert_eq!(
            markers.iter().map(|marker| marker.kind).collect::<Vec<_>>(),
            vec![
                RCodeMarkerKind::OpenBrace,
                RCodeMarkerKind::CloseBrace,
                RCodeMarkerKind::BareBackslash,
                RCodeMarkerKind::OpenBrace,
                RCodeMarkerKind::CloseBrace,
                RCodeMarkerKind::BareBackslash,
            ]
        );
        assert_eq!(
            markers
                .iter()
                .map(|marker| &value[marker.start..marker.end])
                .collect::<Vec<_>>(),
            vec!["{", "}", "\\", "{", "}", "\\"]
        );
        let function_start = value.find("function").expect("function keyword");
        assert!(!markers.iter().any(|marker| marker.start == function_start));
    }

    #[test]
    fn markers_keep_typed_tokens_inside_line_recovery_errors() {
        let value = r#"* \dontshow{hidden}
"#;
        let parsed = parse_with_options(value, &ParseOptions::default());
        assert!(!parsed.diagnostics.is_empty());
        let error = parsed
            .cst
            .descendants()
            .find(|node| node.kind() == SyntaxKind::ERROR)
            .expect("unexpected operator should create a recovery error node");
        let error_tokens = error
            .descendants_with_tokens()
            .filter_map(SyntaxElement::into_token)
            .filter(|token| {
                matches!(
                    token.kind(),
                    SyntaxKind::FUNCTION_KW | SyntaxKind::LBRACE | SyntaxKind::RBRACE
                )
            })
            .map(|token| token.text().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(error_tokens, vec!["\\", "{", "}"]);

        let markers = r_code_markers(value);
        assert_eq!(
            markers
                .iter()
                .map(|marker| (&marker.kind, &value[marker.start..marker.end]))
                .collect::<Vec<_>>(),
            vec![
                (&RCodeMarkerKind::BareBackslash, "\\"),
                (&RCodeMarkerKind::OpenBrace, "{"),
                (&RCodeMarkerKind::CloseBrace, "}"),
            ]
        );
    }
}
