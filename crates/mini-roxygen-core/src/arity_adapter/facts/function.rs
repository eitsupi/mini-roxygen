//! Extracts function formals and body provenance.
//!
//! Function shape has unusually strict delimiter and recovery rules, so it is isolated from general assignment and call classification while reusing the shared name and span invariants.

use arity_parser::ast::{AstNode, CallExpr, FunctionExpr};
use arity_parser::syntax::{SyntaxElement, SyntaxKind};

use crate::source::{FileId, Span, Spanned};

use super::name::{RName, RNameDecodeError};
use super::{RNameDelimiter, name_delimiter, span_for_element, span_for_offsets, span_for_token};
/// Syntax facts for a function expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionFact {
    /// The formals in source order, or a structural failure with no partial
    /// formal list. Parser diagnostics are handled by the production adapter,
    /// while this structural check remains the fail-safe for callers that
    /// inspect a recovered CST directly.
    pub formals: Result<Vec<Formal>, FormalError>,
    /// The first non-trivia, non-comment direct child after `)`. This span
    /// excludes comments and trivia between `)` and the body.
    pub body_span: Option<Span>,
    /// Whether the function body contains an unqualified `UseMethod` call.
    pub calls_use_method: bool,
}

/// A structural reason that a function's formal list could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FormalError {
    InvalidStructure,
}

/// One function formal, retaining its exact source provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Formal {
    /// The raw segment between delimiters, including its trivia.
    pub segment_span: Span,
    /// The first meaningful name token in the segment.
    pub name: Spanned<Result<RName, RNameDecodeError>>,
    /// The default expression, if the segment contains `=`.
    pub default: Option<SourceExpr>,
}

/// A default expression and the complete source envelope of its clause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceExpr {
    /// The default expression element itself.
    pub span: Span,
    /// The source range from just after `=` to the end of the segment,
    /// including trivia and comments.
    pub clause_span: Span,
    /// The `COMMENT` tokens inside the clause.
    pub comment_spans: Vec<Span>,
}
pub(super) fn function_fact(function: &FunctionExpr, file_id: FileId) -> FunctionFact {
    let elements: Vec<_> = function.syntax().children_with_tokens().collect();
    let formals = formal_list(function, &elements, file_id);
    let body_span = function.body().map(|body| span_for_element(&body, file_id));
    let calls_use_method = function.body().is_some_and(|body| match body {
        SyntaxElement::Node(node) => std::iter::once(node.clone())
            .chain(node.descendants())
            .filter_map(CallExpr::cast)
            .any(|call| {
                matches!(
                    call.base(),
                    Some(SyntaxElement::Token(token))
                        if token.kind() == SyntaxKind::IDENT
                            && name_delimiter(&token)
                                .and_then(|delimiter| RName::decode(token.text(), delimiter).ok())
                                .is_some_and(|name| name.as_str() == "UseMethod")
                )
            }),
        SyntaxElement::Token(_) => false,
    });
    FunctionFact {
        formals,
        body_span,
        calls_use_method,
    }
}

fn formal_list(
    function: &FunctionExpr,
    elements: &[SyntaxElement],
    file_id: FileId,
) -> Result<Vec<Formal>, FormalError> {
    let Some(lparen_idx) = function.lparen_index() else {
        return Err(FormalError::InvalidStructure);
    };
    let Some(rparen_idx) = function.rparen_index() else {
        return Err(FormalError::InvalidStructure);
    };
    if rparen_idx <= lparen_idx || rparen_idx >= elements.len() {
        return Err(FormalError::InvalidStructure);
    }

    let parameter_elements = &elements[lparen_idx + 1..rparen_idx];
    if parameter_elements
        .iter()
        .all(|element| is_formal_trivia(element.kind()))
    {
        return Ok(Vec::new());
    }

    let mut segments = Vec::new();
    let mut segment_start = lparen_idx + 1;
    let mut segment_start_offset = elements[lparen_idx].text_range().end().into();
    for (index, element) in elements
        .iter()
        .enumerate()
        .take(rparen_idx)
        .skip(lparen_idx + 1)
    {
        if element.kind() == SyntaxKind::COMMA {
            segments.push((
                segment_start,
                index,
                segment_start_offset,
                element.text_range().start().into(),
            ));
            segment_start = index + 1;
            segment_start_offset = element.text_range().end().into();
        }
    }
    segments.push((
        segment_start,
        rparen_idx,
        segment_start_offset,
        elements[rparen_idx].text_range().start().into(),
    ));

    segments
        .into_iter()
        .map(|(start, end, start_offset, end_offset)| {
            formal_segment(&elements[start..end], start_offset, end_offset, file_id)
        })
        .collect()
}

fn formal_segment(
    elements: &[SyntaxElement],
    segment_start: u32,
    segment_end: u32,
    file_id: FileId,
) -> Result<Formal, FormalError> {
    if elements.iter().any(element_contains_error) {
        return Err(FormalError::InvalidStructure);
    }
    let meaningful: Vec<_> = elements
        .iter()
        .filter(|element| !is_formal_trivia(element.kind()))
        .collect();
    let (first, rest) = meaningful
        .split_first()
        .ok_or(FormalError::InvalidStructure)?;
    let name_token = match first {
        SyntaxElement::Token(token) => token,
        SyntaxElement::Node(_) => return Err(FormalError::InvalidStructure),
    };
    let delimiter = name_delimiter(name_token).ok_or(FormalError::InvalidStructure)?;
    // R's grammar permits only bare or backtick names as formals, unlike call argument names.
    if !matches!(delimiter, RNameDelimiter::Bare | RNameDelimiter::Backtick) {
        return Err(FormalError::InvalidStructure);
    }
    let name = Spanned::new(
        RName::decode(name_token.text(), delimiter),
        span_for_token(name_token, file_id),
    );
    let segment_span = span_for_offsets(segment_start, segment_end, file_id);

    let Some(eq_offset) = rest
        .iter()
        .position(|element| element.kind() == SyntaxKind::ASSIGN_EQ)
    else {
        if !rest.is_empty() {
            return Err(FormalError::InvalidStructure);
        }
        return Ok(Formal {
            segment_span,
            name,
            default: None,
        });
    };

    // The first meaningful element after the name must be the formal's one
    // direct-child `=`. Any other meaningful element is an extra structure,
    // not part of the name.
    if eq_offset != 0 {
        return Err(FormalError::InvalidStructure);
    }
    let after_equals = &rest[1..];
    if after_equals.len() != 1 {
        return Err(FormalError::InvalidStructure);
    }
    let default_element = after_equals
        .iter()
        .find(|element| !is_formal_trivia(element.kind()))
        .ok_or(FormalError::InvalidStructure)?;

    let clause_start = match rest[0] {
        SyntaxElement::Token(token) => token.text_range().end().into(),
        SyntaxElement::Node(_) => return Err(FormalError::InvalidStructure),
    };
    let clause_end = elements
        .last()
        .map(|element| element.text_range().end().into())
        .unwrap_or(clause_start);
    let clause_span = span_for_offsets(clause_start, clause_end, file_id);
    let comment_spans = elements
        .iter()
        .flat_map(comment_tokens)
        .map(|token| span_for_token(&token, file_id))
        .filter(|span| span.range.start() >= clause_start && span.range.end() <= clause_end)
        .collect();

    Ok(Formal {
        segment_span,
        name,
        default: Some(SourceExpr {
            span: span_for_element(default_element, file_id),
            clause_span,
            comment_spans,
        }),
    })
}

fn is_formal_trivia(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
    )
}

fn element_contains_error(element: &SyntaxElement) -> bool {
    match element {
        SyntaxElement::Token(token) => token.kind() == SyntaxKind::ERROR,
        SyntaxElement::Node(node) => {
            node.kind() == SyntaxKind::ERROR
                || node
                    .descendants_with_tokens()
                    .any(|child| child.kind() == SyntaxKind::ERROR)
        }
    }
}

fn comment_tokens(element: &SyntaxElement) -> Vec<arity_parser::syntax::SyntaxToken> {
    match element {
        SyntaxElement::Token(token) => (token.kind() == SyntaxKind::COMMENT)
            .then_some(vec![token.clone()])
            .unwrap_or_default(),
        SyntaxElement::Node(node) => node
            .descendants_with_tokens()
            .filter_map(|child| match child {
                SyntaxElement::Token(token) if token.kind() == SyntaxKind::COMMENT => Some(token),
                _ => None,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    use arity_parser::ast::{AstNode, FunctionExpr};

    use super::FunctionFact;
    use crate::arity_adapter::test_support::{function, parsed, slice};
    use crate::arity_adapter::{FormalError, RName, RNameDecodeError};
    use crate::source::{FileId, SourceFile};
    #[test]
    fn extracts_empty_comment_only_and_lambda_formal_lists() {
        for source_text in [
            r#"f <- function() 1
"#,
            r#"f <- function () 1
"#,
            "f <- \\(x) x\n",
            r#"f <- function(
 # only a comment
) 1
"#,
        ] {
            let (parsed, _) = parsed(source_text);
            assert!(parsed.diagnostics.is_empty(), "{source_text}");
            let expected = if source_text.contains("\\(x)") { 1 } else { 0 };
            assert_eq!(
                function(&parsed).formals.as_ref().unwrap().len(),
                expected,
                "{source_text}"
            );
        }

        let (parsed, source) = parsed("f <- \\(x) # body comment\n x\n");
        let facts = function(&parsed);
        assert_eq!(
            facts.formals.as_ref().unwrap()[0]
                .name
                .value
                .as_ref()
                .unwrap()
                .as_str(),
            "x"
        );
        assert_eq!(slice(&source, facts.body_span.unwrap()), "x");
    }

    #[test]
    fn splits_defaults_only_on_direct_child_commas() {
        let source_text = concat!(
            "f <- function(call = f(a, b), string = 'a,b', braces = { c(a, b) }, ",
            r#"condition = if (a) b else c, nested = function(x, y = g(x, y)) x) 1
"#,
        );
        let (parsed, source) = parsed(source_text);
        assert!(
            parsed.diagnostics.is_empty(),
            "diagnostics: {:?}",
            parsed.diagnostics
        );
        let formals = function(&parsed).formals.as_ref().unwrap();
        assert_eq!(formals.len(), 5);
        assert_eq!(
            formals
                .iter()
                .map(|formal| formal.name.value.as_ref().unwrap().as_str())
                .collect::<Vec<_>>(),
            ["call", "string", "braces", "condition", "nested"]
        );
        assert_eq!(
            slice(&source, formals[0].default.as_ref().unwrap().span),
            "f(a, b)"
        );
        assert_eq!(
            slice(&source, formals[1].default.as_ref().unwrap().span),
            "'a,b'"
        );
        assert_eq!(
            slice(&source, formals[2].default.as_ref().unwrap().span),
            "{ c(a, b) }"
        );
        assert_eq!(
            slice(&source, formals[3].default.as_ref().unwrap().span),
            "if (a) b else c"
        );
        assert_eq!(
            slice(&source, formals[4].default.as_ref().unwrap().span),
            "function(x, y = g(x, y)) x"
        );
    }

    #[test]
    fn preserves_default_clause_comments_and_segment_envelopes() {
        let source_text = r#"f <- function(  x = g(1, # keep this
 2)  , y) 1
"#;
        let (parsed, source) = parsed(source_text);
        let formals = function(&parsed).formals.as_ref().unwrap();
        assert_eq!(
            slice(&source, formals[0].segment_span),
            "  x = g(1, # keep this\n 2)  "
        );
        let default = formals[0].default.as_ref().unwrap();
        assert_eq!(slice(&source, default.span), "g(1, # keep this\n 2)");
        assert_eq!(
            slice(&source, default.clause_span),
            " g(1, # keep this\n 2)  "
        );
        assert_eq!(default.comment_spans.len(), 1);
        assert_eq!(slice(&source, default.comment_spans[0]), "# keep this");
        assert!(slice(&source, default.span).contains("# keep this"));
        assert!(formals[1].default.is_none());
    }

    #[test]
    fn keeps_dots_as_ordinary_formals() {
        let (parsed, _) = parsed(
            r#"f <- function(..., ..1, ... = 1) 1
"#,
        );
        let formals = function(&parsed).formals.as_ref().unwrap();
        assert_eq!(
            formals
                .iter()
                .map(|formal| formal.name.value.as_ref().unwrap().as_str())
                .collect::<Vec<_>>(),
            ["...", "..1", "..."]
        );
        assert!(formals[0].default.is_none());
        assert!(formals[1].default.is_none());
        assert_eq!(formals[2].default.as_ref().unwrap().span.range.len(), 1);
    }

    #[test]
    fn preserves_backtick_names_and_undecodable_names_in_order() {
        let source_text = "f <- function(`a b` = 1, `a\\`b` = 2, plain) 1\n";
        let (parsed, source) = parsed(source_text);
        let formals = function(&parsed).formals.as_ref().unwrap();
        assert_eq!(formals.len(), 3);
        assert_eq!(formals[0].name.value.as_ref().unwrap().as_str(), "a b");
        assert_eq!(
            formals[1].name.value,
            Err(RNameDecodeError::ContainsBackslash)
        );
        assert_eq!(slice(&source, formals[1].name.span), "`a\\`b`");
        assert_eq!(formals[2].name.value.as_ref().unwrap().as_str(), "plain");
    }

    #[test]
    fn parser_diagnoses_invalid_formal_lists_and_adapter_fails_closed() {
        for source_text in [
            r#"f <- function(x,) 1
"#,
            r#"f <- function(,x) 1
"#,
            r#"f <- function(x,,y) 1
"#,
            r#"f <- function(x =) 1
"#,
        ] {
            let parser_output = arity_parser::parser::parse(source_text);
            assert!(!parser_output.diagnostics.is_empty(), "{source_text}");
            let function = parser_output
                .cst
                .descendants()
                .find_map(FunctionExpr::cast)
                .expect("recovery CST should retain the function node");
            assert_eq!(
                super::function_fact(&function, FileId::new(3)).formals,
                Err(FormalError::InvalidStructure),
                "{source_text}"
            );
            let (parsed, _) = parsed(source_text);
            assert!(parsed.top_level.is_empty(), "{source_text}");
            assert!(!parsed.diagnostics.is_empty(), "{source_text}");
        }
    }

    #[test]
    fn rejects_string_spelled_formal_names_but_accepts_backticks_and_string_defaults() {
        for source_text in [
            r#"f <- function("a b" = 1) 1
"#,
            r#"f <- function('x') 1
"#,
            r#"f <- function("x" = 1, y) 1
"#,
            r#"f <- function(r"(a b)" = 1) 1
"#,
        ] {
            let parser_output = arity_parser::parser::parse(source_text);
            assert!(!parser_output.diagnostics.is_empty(), "{source_text}");
            let function = parser_output
                .cst
                .descendants()
                .find_map(FunctionExpr::cast)
                .expect("recovery CST should retain the function node");
            assert_eq!(
                super::function_fact(&function, FileId::new(3)).formals,
                Err(FormalError::InvalidStructure),
                "{source_text}"
            );
            let (parsed, _) = parsed(source_text);
            assert!(parsed.top_level.is_empty(), "{source_text}");
            assert!(!parsed.diagnostics.is_empty(), "{source_text}");
        }

        let (backtick_parsed, _) = parsed(
            r#"f <- function(`a b`, x) 1
"#,
        );
        let formals = function(&backtick_parsed).formals.as_ref().unwrap();
        assert_eq!(
            formals
                .iter()
                .map(|formal| formal.name.value.as_ref().unwrap().as_str())
                .collect::<Vec<_>>(),
            ["a b", "x"]
        );

        let (parsed, source) = parsed(
            r#"f <- function(x = "a b") 1
"#,
        );
        let formals = function(&parsed).formals.as_ref().unwrap();
        assert_eq!(formals[0].name.value.as_ref().unwrap().as_str(), "x");
        assert_eq!(
            slice(&source, formals[0].default.as_ref().unwrap().span),
            "\"a b\""
        );
    }

    #[test]
    fn records_body_spans_without_comments_or_trivia() {
        for (source_text, expected) in [
            (
                r#"f <- function(x) # comment
 { x }
"#,
                "{ x }",
            ),
            (
                r#"f <- function(x) # comment
 x + 1
"#,
                "x + 1",
            ),
            ("f <- \\(x) # comment\n { x }\n", "{ x }"),
        ] {
            let (parsed, source) = parsed(source_text);
            assert_eq!(
                slice(&source, function(&parsed).body_span.unwrap()),
                expected
            );
        }
    }

    #[test]
    fn preserves_multiline_and_crlf_source_boundaries() {
        let source_text = "f <- function(\r\n  x =\r\n    g(1,\r\n      2),\r\n  y\r\n)\r\n  x\r\n";
        let (parsed, source) = parsed(source_text);
        let facts = function(&parsed);
        let formals = facts.formals.as_ref().unwrap();
        assert_eq!(
            slice(&source, formals[0].segment_span),
            "\r\n  x =\r\n    g(1,\r\n      2)"
        );
        assert_eq!(slice(&source, facts.body_span.unwrap()), "x");
        for formal in formals {
            assert!(source.text_range(formal.segment_span.range).is_some());
            assert!(source.text_range(formal.name.span.range).is_some());
        }
    }

    #[test]
    fn r_oracle_checks_formal_names_missing_defaults_and_default_structure() {
        let fixtures = [
            r#"f <- function(x, y = f(a, b), ... = if (x) 1 else 2) { x }
"#,
            r#"f <- function(`a b`, value = { a <- 1; a }, text = 'a,b') value
"#,
        ];
        for source_text in fixtures {
            let (parsed, source) = parsed(source_text);
            assert_formals_with_r(source_text, &source, function(&parsed));
        }
    }

    fn assert_formals_with_r(source_text: &str, source: &SourceFile, facts: &FunctionFact) {
        let Some(rscript) = rscript_path() else {
            if std::env::var_os("MINI_ROXYGEN_REQUIRE_FORMALS_ORACLE").is_some() {
                panic!("MINI_ROXYGEN_REQUIRE_FORMALS_ORACLE is set but Rscript is unavailable");
            }
            eprintln!("formal oracle skipped (Rscript is unavailable)");
            return;
        };
        let workspace = tempfile::tempdir().expect("formal oracle tempdir");
        let source_path = workspace.path().join("fixture.R");
        fs::write(&source_path, source_text).expect("formal oracle source write");
        let formals = facts
            .formals
            .as_ref()
            .expect("oracle fixture must be valid");
        let mut default_paths = Vec::new();
        for (index, formal) in formals.iter().enumerate() {
            if let Some(default) = &formal.default {
                let path = workspace.path().join(format!("default-{index}.R"));
                fs::write(&path, slice(source, default.clause_span)).expect("default write");
                default_paths.push(path.to_string_lossy().into_owned());
            } else {
                default_paths.push(String::from("-"));
            }
        }
        let output = Command::new(rscript)
            .env("TMPDIR", "/dev/shm")
            .env("R_SESSION_TMPDIR", "/dev/shm")
            .arg("--vanilla")
            .arg("-e")
            .arg(ORACLE_SCRIPT)
            .arg(&source_path)
            .args(&default_paths)
            .output()
            .expect("formal oracle process");
        assert!(
            output.status.success(),
            "formal oracle failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut lines = stdout.lines();
        let names = lines.next().expect("formal oracle names line");
        let expected_names = formals
            .iter()
            .map(|formal| {
                formal
                    .name
                    .value
                    .as_ref()
                    .map_or("<undecodable>", RName::as_str)
            })
            .collect::<Vec<_>>()
            .join("\u{1f}");
        assert_eq!(names, format!("NAMES\t{expected_names}"));
        for (index, formal) in formals.iter().enumerate() {
            let line = lines.next().expect("formal oracle default line");
            match &formal.default {
                Some(_) => assert_eq!(line, format!("DEFAULT\t{index}\tok")),
                None => assert_eq!(line, format!("MISSING\t{index}\tok")),
            }
        }
    }

    fn rscript_path() -> Option<PathBuf> {
        Command::new("Rscript")
            .arg("--version")
            .env("TMPDIR", "/dev/shm")
            .env("R_SESSION_TMPDIR", "/dev/shm")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|_| PathBuf::from("Rscript"))
    }

    const ORACLE_SCRIPT: &str = concat!(
        r#"args <- commandArgs(trailingOnly = TRUE)
"#,
        r#"path <- args[1]
"#,
        r#"expr <- parse(file = path, keep.source = FALSE)[[1]]
"#,
        r#"fun <- eval(expr[[3]], envir = baseenv())
"#,
        r#"f <- formals(fun)
"#,
        "cat('NAMES\\t', paste(names(f), collapse = '\\u001f'), '\\n', sep = '')\n",
        r#"for (i in seq_along(f)) {
"#,
        r#"  d <- f[i]
"#,
        "  if (grepl('=\\\\s*\\\\)$', paste(capture.output(dput(d)), collapse = ''))) {\n",
        "    cat('MISSING\\t', i - 1L, '\\tok\\n', sep = '')\n",
        r#"  } else {
"#,
        r#"    parsed <- parse(file = args[i + 1L], keep.source = FALSE)
"#,
        r#"    ok <- length(parsed) == 1L && identical(parsed[[1]], d[[1]])
"#,
        "    cat('DEFAULT\\t', i - 1L, '\\t', if (ok) 'ok' else 'bad', '\\n', sep = '')\n",
        r#"    if (!ok) quit(status = 1L)
"#,
        r#"  }
"#,
        r#"}
"#,
    );
}
