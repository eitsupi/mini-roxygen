//! Generates R call text from a statically extracted function signature.
//!
//! The generator copies default expressions from their source spans into a
//! candidate call without evaluating them. Short candidates keep that source
//! spelling; long or multiline candidates are laid out by `arity-formatter`,
//! and formatter failures fall back to the untouched candidate. Rd escaping
//! belongs to the AST writer, which has the lexical state needed to distinguish
//! strings, raw strings, and comments.

use crate::arity_adapter::{FormalError, RNameDecodeError};
use crate::r_parse::FunctionObject;
use crate::r_syntax::is_reserved_r_word;
use crate::source::{SourceMap, Span, TextRange};
use arity_formatter::{FormatStyle, LineEnding, format_with_style};
use unicode_width::UnicodeWidthStr;

/// Generated R call text from one function's formals.
///
/// Keeping the text behind this type ensures callers cannot accidentally
/// construct usage text without the generator's UTF-8 and line-ending
/// invariants. The text has no fabricated trailing newline; callers own the
/// surrounding Rd whitespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedUsage {
    text: String,
    call_head: String,
}

impl GeneratedUsage {
    /// Constructs name-only usage for a documented non-function object.
    pub(crate) fn object(name: &str) -> Self {
        let rendered_name = render_name(name);
        Self::new(rendered_name.clone(), rendered_name)
    }

    /// Constructs usage for a data object without a function signature.
    pub(crate) fn data(name: &str, lazy_data: bool) -> Self {
        let rendered_name = render_name(name);
        let text = if lazy_data {
            rendered_name.clone()
        } else {
            format!("data({rendered_name})")
        };
        Self::new(text, rendered_name)
    }

    /// Returns the generated call text without a fabricated trailing LF.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Returns everything the generated call carries after its call head,
    /// beginning at the call's opening parenthesis.
    ///
    /// The head comes from how the call was assembled rather than from
    /// searching the text, because a quoted R name may itself contain the
    /// parenthesis a textual split would cut at. Callers place the tail after
    /// an Rd method macro they build as AST, so the generic and class names
    /// reach the output through the writer's escaping rather than through
    /// string interpolation here.
    #[must_use]
    pub(crate) fn s3_tail(&self) -> &str {
        self.text
            .strip_prefix(&self.call_head)
            .expect("generated usage must retain its call head")
    }
}

/// A typed refusal to generate usage text from an incomplete or unsafe syntax
/// fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageError {
    /// The adapter could not structurally extract the function's formals.
    InvalidFormals(FormalError),
    /// A formal name could not be decoded without guessing at R escapes.
    UndecodableFormalName {
        /// The source span of the formal name.
        span: Span,
        /// The decoder's reason for refusing the spelling.
        reason: RNameDecodeError,
    },
    /// A default or comment-related source span did not resolve in the map.
    UnresolvableSourceSpan {
        /// The span that could not be read.
        span: Span,
    },
    /// A comment would consume the delimiter appended after its default.
    UnsafeDefaultComment {
        /// The comment span that has no following line break in the default.
        span: Span,
    },
    /// A replacement-function binding did not end in a `value` formal.
    InvalidReplacementSignature {
        /// The span of the replacement function's binding name.
        name_span: Span,
    },
}

/// Generates a call-shaped usage line from a function without evaluating any
/// default expression.
///
/// Default expressions are copied from their expression spans rather than
/// deparsed, then retained verbatim for short candidates or passed through the
/// deterministic formatter when a candidate is long or multiline. A formatter
/// failure preserves the candidate. Only CRLF and lone CR line endings are
/// normalized to LF; Rd escaping remains the writer's responsibility.
pub fn generate_function_usage(
    function: &FunctionObject,
    sources: &SourceMap,
) -> Result<GeneratedUsage, UsageError> {
    let formals = function
        .formals
        .as_ref()
        .map_err(|&error| UsageError::InvalidFormals(error))?;

    let names = formals
        .iter()
        .map(|formal| {
            formal
                .name
                .value
                .as_ref()
                .map(|name| name.as_str().to_owned())
                .map_err(|&reason| UsageError::UndecodableFormalName {
                    span: formal.name.span,
                    reason,
                })
        })
        .collect::<Result<Vec<_>, _>>()?;

    // `<-` itself ends with the replacement suffix but leaves no base name to
    // call, and R does bind it: `` `<-` <- function(x, value) `` is legal. It
    // is the assignment function, not a replacement function, so it takes the
    // ordinary call form rather than producing an empty backtick pair.
    let replacement_base = function
        .name
        .canonical
        .as_str()
        .strip_suffix("<-")
        .filter(|base| !base.is_empty());
    let is_replacement = replacement_base.is_some();
    if is_replacement && names.last().map(String::as_str) != Some("value") {
        return Err(UsageError::InvalidReplacementSignature {
            name_span: function.name.spelling,
        });
    }

    let (call_name, call_formals, suffix) = if let Some(base_name) = replacement_base {
        let call_formals = formals[..formals.len() - 1]
            .iter()
            .zip(names[..names.len() - 1].iter())
            .map(|(formal, name)| render_formal(formal, name, sources))
            .collect::<Result<Vec<_>, _>>()?;
        (
            render_replacement_base_name(base_name, function.name.spelling, sources),
            call_formals,
            format!(
                " <- {}",
                render_authored_name(
                    names.last().expect("replacement has a value"),
                    formals.last().expect("replacement has a value").name.span,
                    sources,
                )
            ),
        )
    } else {
        let call_formals = formals
            .iter()
            .zip(names.iter())
            .map(|(formal, name)| render_formal(formal, name, sources))
            .collect::<Result<Vec<_>, _>>()?;
        (
            render_authored_name(
                function.name.canonical.as_str(),
                function.name.spelling,
                sources,
            ),
            call_formals,
            String::new(),
        )
    };

    let candidate = format!("{call_name}({}){suffix}", call_formals.join(", "));
    let text = format_candidate(candidate, !call_formals.is_empty());

    Ok(GeneratedUsage::new(text, call_name))
}

fn format_candidate(candidate: String, has_formals: bool) -> String {
    let needs_format = has_formals
        && (candidate.contains('\n') || UnicodeWidthStr::width(candidate.as_str()) > 80);
    if !needs_format {
        return candidate;
    }
    let style = FormatStyle {
        line_width: 80,
        indent_width: 2,
        line_ending: LineEnding::Lf,
    };
    match format_with_style(&candidate, style) {
        Ok(formatted) => formatted.trim_end_matches('\n').to_owned(),
        Err(_) => candidate,
    }
}

impl GeneratedUsage {
    fn new(text: String, call_head: String) -> Self {
        let mut text = normalize_newlines(&text);
        while text.ends_with('\n') {
            text.pop();
        }
        Self { text, call_head }
    }
}

fn render_formal(
    formal: &crate::arity_adapter::Formal,
    name: &str,
    sources: &SourceMap,
) -> Result<String, UsageError> {
    let mut rendered = render_authored_name(name, formal.name.span, sources);
    if let Some(default) = &formal.default {
        let text = sources
            .span_text(default.span)
            .ok_or(UsageError::UnresolvableSourceSpan { span: default.span })?;

        for &comment_span in &default.comment_spans {
            if !span_is_inside(comment_span, default.span) {
                continue;
            }
            let tail_span = Span::new(
                default.span.file,
                TextRange::new(comment_span.range.end(), default.span.range.end()),
            );
            let tail = sources
                .span_text(tail_span)
                .ok_or(UsageError::UnresolvableSourceSpan { span: tail_span })?;
            if !normalize_newlines(tail).contains('\n') {
                return Err(UsageError::UnsafeDefaultComment { span: comment_span });
            }
        }

        rendered.push_str(" = ");
        rendered.push_str(&normalize_newlines(text));
    }
    Ok(rendered)
}

fn span_is_inside(inner: Span, outer: Span) -> bool {
    inner.file == outer.file
        && inner.range.start() >= outer.range.start()
        && inner.range.end() <= outer.range.end()
}

fn render_name(name: &str) -> String {
    if is_bare_name(name) {
        return name.to_owned();
    }
    // A decoded name is a value, not a spelling, and R reads escapes inside a
    // backtick-quoted name. A raw string target such as `r"(a\nb)"` decodes to
    // a name holding a literal backslash, and one such as ``r"(a`b)"`` to a
    // name holding a backtick; interpolating either verbatim would name a
    // different object or close the quoting early.
    quote_name(name)
}

fn render_authored_name(name: &str, spelling: Span, sources: &SourceMap) -> String {
    let Some(spelling) = sources.span_text(spelling) else {
        return render_name(name);
    };
    if spelling.starts_with('`') && spelling.ends_with('`') {
        return quote_name(name);
    }
    if spelling == name {
        return name.to_owned();
    }
    render_name(name)
}

fn render_replacement_base_name(base_name: &str, spelling: Span, sources: &SourceMap) -> String {
    let Some(spelling) = sources.span_text(spelling) else {
        return render_name(base_name);
    };
    let Some(base_spelling) = spelling.strip_suffix("<-") else {
        return render_name(base_name);
    };
    if base_spelling == base_name {
        base_name.to_owned()
    } else {
        render_name(base_name)
    }
}

fn quote_name(name: &str) -> String {
    let mut quoted = String::with_capacity(name.len() + 2);
    quoted.push('`');
    for character in name.chars() {
        if matches!(character, '\\' | '`') {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('`');
    quoted
}

/// Applies roxygen's class-specific backtick quoting. Explicit tag values may
/// retain their authored quote delimiters; inferred values are decoded names.
pub(crate) fn render_method_class(name: &str, preserve_authored_quotes: bool) -> String {
    let bytes = name.as_bytes();
    if preserve_authored_quotes
        && bytes.len() >= 2
        && matches!(bytes[0], b'`' | b'"' | b'\'')
        && bytes.last() == Some(&bytes[0])
    {
        return name.to_owned();
    }
    render_name(name)
}

fn is_bare_name(name: &str) -> bool {
    if is_reserved_r_word(name) {
        return false;
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let first_is_valid = first.is_ascii_alphabetic()
        || (first == '.' && !chars.clone().next().is_some_and(|ch| ch.is_ascii_digit()));
    first_is_valid && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_'))
}

fn normalize_newlines(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            normalized.push('\n');
        } else {
            normalized.push(ch);
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rd_ast::{RdDocument, RdNode, RdTag};
    use rd_writer::{Writer, WriterOptions};

    use super::{GeneratedUsage, UsageError, generate_function_usage};
    use crate::arity_adapter::parse;
    use crate::r_parse::{BlockTarget, FunctionObject, build_object_index};
    use crate::source::{SourceFile, SourceMap, Span, TextRange};

    fn function(source_text: &str) -> (FunctionObject, SourceMap) {
        let source_file = SourceFile::new(PathBuf::from("test.R"), source_text.to_owned());
        let mut sources = SourceMap::new();
        let file = sources.add_file(source_file.clone());
        let parsed = parse(&source_file, file);
        let index = build_object_index(parsed, file);
        let function = index
            .documented
            .into_iter()
            .find_map(|object| match object.target {
                BlockTarget::FunctionAssignment(function) => Some(function),
                _ => None,
            })
            .expect("source should contain a documented function");
        (function, sources)
    }

    fn usage(source_text: &str) -> String {
        let (function, sources) = function(source_text);
        generate_function_usage(&function, &sources)
            .expect("usage should generate")
            .as_str()
            .to_owned()
    }

    #[test]
    fn generates_basic_formals_and_ellipsis_in_source_order() {
        assert_eq!(
            usage(
                r#"#' doc
f <- function() {}
"#
            ),
            "f()"
        );
        assert_eq!(
            usage(
                r#"#' doc
f <- function(x, ...) {}
"#
            ),
            "f(x, ...)"
        );
        assert_eq!(
            usage(
                r#"#' doc
f <- function(x = 1, y = g(1, 2)) {}
"#
            ),
            "f(x = 1, y = g(1, 2))"
        );
    }

    #[test]
    fn preserves_nested_block_and_anonymous_defaults() {
        let output = usage(
            r#"#' doc
f <- function(x = { 1; 2 }) {}
"#,
        );
        insta::assert_snapshot!(output, @r#"
        f(x = { 1; 2 })
        "#);

        let output = usage(
            r#"#' doc
f <- function(x = function(y) y) {}
"#,
        );
        insta::assert_snapshot!(output, @r#"
        f(x = function(y) y)
        "#);

        let output = usage(
            r#"#' doc
f <- function(
  x = g(
    1, # keep this
    2
  ),
  y
) {}
"#,
        );
        insta::assert_snapshot!(output, @r#"
        f(
          x = g(
            1, # keep this
            2
          ),
          y
        )
        "#);
    }

    #[test]
    fn preserves_comments_strings_and_raw_strings_without_rd_escaping() {
        let output = usage(
            r#"#' doc
f <- function(x = g(1, # keep this
  2)) {}
"#,
        );
        insta::assert_snapshot!(output, @r#"
        f(
          x = g(
            1, # keep this
            2
          )
        )
        "#);
        assert_eq!(
            usage("#' doc\nf <- function(x = \"100%\\\\{}\") {}\n"),
            "f(x = \"100%\\\\{}\")"
        );
        assert_eq!(
            usage("#' doc\nf <- function(x = r\"{100%\\{} }\") {}\n"),
            "f(x = r\"{100%\\{} }\")"
        );
    }

    #[test]
    fn normalizes_crlf_to_lf() {
        let output = usage("#' doc\r\nf <- function(x = g(\r\n  1\r\n)) {}\r\n");
        insta::assert_snapshot!(output, @r#"
        f(x = g(1))
        "#);
        assert!(!output.contains('\r'));
    }

    #[test]
    fn preserves_authored_bare_and_backticked_name_spelling() {
        insta::assert_snapshot!(
            usage(
                r#"#' doc
é <- function(値) {}
"#
            ),
            @"é(値)"
        );
        insta::assert_snapshot!(
            usage(
                r#"#' doc
`if` <- function(a, `if`, `a b`, `_a`, `é`) {}
"#
            ),
            @"`if`(a, `if`, `a b`, `_a`, `é`)"
        );
        insta::assert_snapshot!(
            usage(
                r#"#' doc
f <- function(`.1`, ..., ..1) {}
"#
            ),
            @"f(`.1`, ..., ..1)"
        );
    }

    #[test]
    fn quotes_unicode_number_categories_that_are_not_r_name_digits() {
        insta::assert_snapshot!(
            usage(
                r#"#' doc
`a²` <- function(x) {}
"#
            ),
            @"`a²`(x)"
        );
    }

    #[test]
    fn preserves_authored_backticks_for_unicode_names() {
        insta::assert_snapshot!(
            usage(
                r#"#' doc
`é` <- function(`値`) {}
"#
            ),
            @"`é`(`値`)"
        );
        insta::assert_snapshot!(GeneratedUsage::object("é").as_str(), @"`é`");
    }

    #[test]
    fn renders_s3_classes_with_backticks_without_double_quoting() {
        assert_eq!(super::render_method_class("NULL", false), "`NULL`");
        assert_eq!(
            super::render_method_class("data.frame", false),
            "data.frame"
        );
        assert_eq!(super::render_method_class("`NULL`", false), "`\\`NULL\\``");
        assert_eq!(super::render_method_class("`NULL`", true), "`NULL`");
        assert_eq!(super::render_method_class("\"foo\"", false), "`\"foo\"`");
    }

    #[test]
    fn keeps_79_and_80_width_calls_single_line_and_wraps_81() {
        let below = "a".repeat(76);
        let below_output = super::format_candidate(format!("f({below})"), true);
        insta::assert_snapshot!(below_output, @r#"
        f(aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa)
        "#);

        let at_limit = "a".repeat(77);
        let at_limit_output = super::format_candidate(format!("f({at_limit})"), true);
        insta::assert_snapshot!(at_limit_output, @r#"
        f(aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa)
        "#);

        let over = "a".repeat(78);
        let over_output = super::format_candidate(format!("f({over})"), true);
        insta::assert_snapshot!(over_output, @r#"
        f(
          aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
        )
        "#);

        let unicode_over = format!("{}界", "a".repeat(68));
        let unicode_output = super::format_candidate(format!("f(x = \"{unicode_over}\", y)"), true);
        insta::assert_snapshot!(unicode_output, @r#"
        f(
          x = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa界",
          y
        )
        "#);

        let long_name = "a".repeat(100);
        let zero_formal_output = super::format_candidate(format!("{long_name}()"), false);
        insta::assert_snapshot!(zero_formal_output, @r#"
        aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa()
        "#);

        let fallback = super::format_candidate("f(\n  ]\n)".to_owned(), true);
        insta::assert_snapshot!(fallback, @r#"
        f(
          ]
        )
        "#);
    }

    #[test]
    fn renders_replacement_functions_only_by_trailing_arrow() {
        assert_eq!(
            usage(
                r#"#' doc
`foo<-` <- function(x, value) {}
"#
            ),
            "foo(x) <- value"
        );
        assert_eq!(
            usage(
                r#"#' doc
`dim<-` <- function(x, value) {}
"#
            ),
            "dim(x) <- value"
        );
        assert_eq!(
            usage(
                r#"#' doc
`[<-` <- function(x, i, value) {}
"#
            ),
            "`[`(x, i) <- value"
        );
        assert_eq!(
            usage(
                r#"#' doc
`a b<-` <- function(x, value) {}
"#
            ),
            "`a b`(x) <- value"
        );
        insta::assert_snapshot!(
            usage(
                r#"#' doc
`é<-` <- function(x, value) {}
"#
            ),
            @"`é`(x) <- value"
        );
    }

    #[test]
    fn escapes_a_backtick_and_a_backslash_inside_a_quoted_name() {
        // A raw string target decodes to a name holding those characters
        // literally. Measured with R 4.x: `` `a\`b` `` names ``a`b`` and
        // `` `a\\nb` `` names a backslash followed by n, so both need the
        // escape to name the object the source named.
        assert_eq!(
            usage(
                r#"#' doc
r"(a`b)" <- function() {}
"#
            ),
            "`a\\`b`()"
        );
        assert_eq!(
            usage("#' doc\nr\"(a\\nb)\" <- function() {}\n"),
            "`a\\\\nb`()"
        );
        assert_eq!(
            usage(
                r#"#' doc
f <- function(`x`, ...) {}
r"(p`q)" <- function(a) {}
"#
            ),
            "f(`x`, ...)"
        );
    }

    #[test]
    fn treats_the_assignment_function_as_an_ordinary_call() {
        // R binds `<-` itself, and its name ends with the replacement suffix
        // while leaving no base name behind. It is the assignment function,
        // not a replacement function.
        assert_eq!(
            usage(
                r#"#' doc
`<-` <- function(x, value) {}
"#
            ),
            "`<-`(x, value)"
        );
    }

    #[test]
    fn renders_operators_and_extractors_as_plain_backticked_calls() {
        for (name, expected) in [
            ("%+%", "`%+%`(x, y)"),
            ("+", "`+`(e1, e2)"),
            ("[", "`[`(x, i)"),
            ("[[", "`[[`(x, i)"),
            ("$", "`$`(x, name)"),
        ] {
            let expected_source = match name {
                "%+%" => {
                    r#"#' doc
`%+%` <- function(x, y) {}
"#
                }
                "+" => {
                    r#"#' doc
`+` <- function(e1, e2) {}
"#
                }
                "[" => {
                    r#"#' doc
`[` <- function(x, i) {}
"#
                }
                "[[" => {
                    r#"#' doc
`[[` <- function(x, i) {}
"#
                }
                "$" => {
                    r#"#' doc
`$` <- function(x, name) {}
"#
                }
                _ => unreachable!(),
            };
            assert_eq!(usage(expected_source), expected);
        }
    }

    #[test]
    fn reports_typed_refusals_for_valid_cst_facts() {
        let (undecodable, sources) = function("#' doc\nf <- function(`x\\y`) {}\n");
        assert!(matches!(
            generate_function_usage(&undecodable, &sources),
            Err(UsageError::UndecodableFormalName { .. })
        ));

        let (mut unresolved, sources) = function(
            r#"#' doc
f <- function(x = 1) {}
"#,
        );
        let default = unresolved.formals.as_mut().unwrap()[0]
            .default
            .as_mut()
            .unwrap();
        default.span = Span::new(default.span.file, TextRange::new(999, 1000));
        assert!(matches!(
            generate_function_usage(&unresolved, &sources),
            Err(UsageError::UnresolvableSourceSpan { .. })
        ));

        let (mut unsafe_comment, sources) = function(
            r#"#' doc
f <- function(x = 1) {}
"#,
        );
        let default = unsafe_comment.formals.as_mut().unwrap()[0]
            .default
            .as_mut()
            .unwrap();
        default.comment_spans = vec![default.span];
        assert!(matches!(
            generate_function_usage(&unsafe_comment, &sources),
            Err(UsageError::UnsafeDefaultComment { .. })
        ));

        let (replacement, sources) = function(
            r#"#' doc
`foo<-` <- function(x) {}
"#,
        );
        assert!(matches!(
            generate_function_usage(&replacement, &sources),
            Err(UsageError::InvalidReplacementSignature { .. })
        ));
    }

    #[test]
    fn generated_usage_has_its_documented_invariants() {
        let generated = usage(
            r#"#' doc
f <- function(x = 1) {}
"#,
        );
        let generated = GeneratedUsage {
            call_head: "f".to_owned(),
            text: generated,
        };
        assert!(!generated.as_str().is_empty());
        assert!(!generated.as_str().contains('\r'));
        assert!(!generated.as_str().ends_with('\n'));
        assert!(!generated.as_str().ends_with("\n\n"));
        let normalized = GeneratedUsage::new("f()\r\n\n".to_owned(), "f".to_owned());
        assert_eq!(normalized.as_str(), "f()");
        assert_eq!(normalized.s3_tail(), "()");
    }

    #[test]
    fn method_rendering_replaces_only_the_structural_call_head() {
        let (method_function, sources) = function(
            r#"#' doc
`a(b)` <- function(x) {}
"#,
        );
        let generated =
            generate_function_usage(&method_function, &sources).expect("usage should generate");
        assert_eq!(generated.as_str(), "`a(b)`(x)");
        // The name ends in the very parenthesis a textual split would cut at,
        // so the tail has to start after the head the generator assembled.
        assert_eq!(generated.s3_tail(), "(x)");

        let (replacement, sources) = function(
            r#"#' doc
`a(b)<-` <- function(x, value) {}
"#,
        );
        let generated =
            generate_function_usage(&replacement, &sources).expect("usage should generate");
        // A replacement function keeps its assignment suffix in the tail.
        assert_eq!(generated.s3_tail(), "(x) <- value");
    }

    #[test]
    fn generated_s3_tail_with_a_multiline_default_is_writer_valid() {
        let (function, sources) = function(
            r#"#' doc
f <- function(x = r"(a
b)") {}
"#,
        );
        let generated =
            generate_function_usage(&function, &sources).expect("usage should generate");
        let tail = generated.s3_tail();
        let nodes = crate::rd::rcode_nodes(tail);
        Writer::new(WriterOptions::default())
            .write_document(&RdDocument::from(vec![RdNode::tagged(
                RdTag::Usage,
                None,
                nodes,
            )]))
            .expect("generated S3 tail should serialize");
    }
}
