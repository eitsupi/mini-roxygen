//! Restricted raw Rd macro support for `@examples` and `@examplesIf` bodies.
//!
//! `@examples` content is R source, not Markdown prose, so this scans bytes
//! directly rather than reusing the Markdown raw-Rd machinery in
//! `markdown_conversion::raw_rd`: there is no event stream to splice into,
//! and the scanner consumes only the small set of R token markers exposed by
//! the arity adapter. What the two share is the idea of a closed, explicit
//! macro set with everything else diagnosed and left alone; that idea is
//! small enough not to need its own abstraction.
//!
//! A recognized macro (`\dontrun`, `\donttest`, `\dontshow`, `\testonly`,
//! `\dontdiff`) is lowered into an [`RdNode::Tagged`] whose body is
//! recursively scanned for further macros, so the markup survives to
//! `rd-writer` as node structure instead of becoming a text leaf that gets
//! escaped into oblivion. Ordinary R code keeps going through
//! [`RdNode::RCode`] and the existing escaping behavior, with one
//! exception: `\(` (R's lambda shorthand) is the only backslash spelling
//! that is valid, working R outside a string, comment, backtick-quoted
//! name, or roxygen comment line. Every other bare backslash out there -- an
//! unsupported macro name, a name with a bracket option (`\link[pkg]{x}`), a
//! name with no brace at all (`\R`, `\dots`), or just a stray `\` -- is not
//! valid R on its own, so letting it through as literal text would make
//! `tools::Rd2ex` extract text that R's own parser rejects once
//! `R CMD check` tries to run the example: the same failure mode as the
//! original `\dontrun` bug this module exists to fix, just triggered by a
//! shape other than the five supported macros. So every such backslash is
//! diagnosed rather than passed through silently.
//!
//! `\dontrun` is the one exception to "recursively scanned": R's own Rd
//! parser treats `\dontrun` content as opaque verbatim text (confirmed
//! against `tools::parse_Rd` -- a `\dontshow{...}` written inside a
//! `\dontrun{...}` body is not recognized as a nested tag there, only as
//! literal text), and `rd-writer` enforces the same rule structurally: a
//! `Tagged` child is rejected whenever the enclosing leaf mode is
//! `Verbatim`. So a recognized macro is never looked for inside a
//! `\dontrun` body; the whole body becomes physical-line [`RdNode::Verb`]
//! leaves, matching `\dontrun`'s verbatim content mode in the writer's tag
//! table. `\donttest`, `\dontshow`, `\testonly`, and `\dontdiff` are all
//! R-like content, exactly like `\examples` itself (confirmed against
//! `rd-writer`'s own tag table and against `tools::parse_Rd`, which gives
//! their bodies the `RCODE` pseudo-tag, not `VERB`), so their bodies
//! recurse through this same scanner.
//!
//! Finding a macro's matching closing brace uses only arity's structural
//! brace markers. Braces inside strings, comments, backtick names, and
//! roxygen content are not markers and therefore do not count toward nesting;
//! typed structural tokens inside a parser recovery `ERROR` node still do.
//! Lexical `ERROR` tokens do not. This decides which brace in the *original* R
//! source the author meant as the macro's end. It does not need to mirror the
//! naive, non-string-aware brace counting R's Rd parser happens to use once
//! the content is already Rd-escaped verbatim text: by the time `rd-writer`
//! is done, every brace that came from inside the body has been escaped, so
//! the only literal, un-escaped braces left in the emitted Rd are the ones
//! that structurally open and close the tag itself.
//!
//! A `#'` line reaching this scanner at all is itself a corner case: normal
//! `@examples` content never contains one, because the tag parser strips
//! each line's own `#'` marker before this module ever sees the text.
//! Writing the marker *twice* (`#' #' see \code{x}`) leaves one `#'` behind
//! in the normalized value, and `arity-parser` -- which does not know or
//! care that this text used to be roxygen-commented R source -- then
//! classifies that leftover `#'` line as a roxygen comment in its own
//! right, sub-tokenizing it instead of emitting one plain `COMMENT` token
//! the way an ordinary `#` comment gets treated. Keeping it out of the
//! marker stream matters most inside a recognized macro's body: an
//! unrecognized brace inside such a line used to close
//! `\dontshow`/`\donttest`/... early, with no diagnostic at all, silently
//! leaking code that was meant to stay hidden out into the ordinary example
//! body.

use std::ops::Range;

use rd_ast::{RdNode, RdTag};

use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, Label};
use crate::source::{Span, TextRange};
use crate::tags::RCodeText;

use super::usage::rcode_nodes;

/// One problem found while scanning `@examples`/`@examplesIf` R source,
/// carried in encounter order so the eventual diagnostics come out in
/// source order even though "unsupported" and "unterminated" get distinct
/// diagnostic codes and messages: the two are different problems from a
/// caller's point of view (an unrecognized name versus a recognized one
/// that just is not closed), and folding them into one code with a message
/// that contradicts the code's own doc comment for half of its uses is
/// exactly the kind of mismatch a consumer filtering on `code` should not
/// have to work around.
enum RawRdIssue {
    /// A macro-shaped backslash that is not one of the five supported
    /// names, or is combined with a bracket option (none of them take
    /// one), or has no brace group at all. Reported as
    /// [`DiagnosticCode::UnsupportedRawRdMacro`].
    Unsupported(Range<usize>),
    /// A supported macro name (no bracket option) whose brace never closes
    /// within the enclosing region. The range covers just the introducer
    /// (`\dontrun{`), not the rest of the region: the name is known and
    /// supported, so unlike [`RawRdIssue::Unsupported`] there is no reason
    /// to make the label swallow everything after it. Reported as
    /// [`DiagnosticCode::UnterminatedRawRdMacro`].
    Unterminated(Range<usize>),
}

/// Lowers one `@examples`/`@examplesIf` R source value into Rd nodes,
/// recognizing the supported raw Rd macros and diagnosing unsupported or
/// unterminated ones.
///
/// `fallback` is used for a diagnostic's span only when the R source itself
/// cannot place the offending range (which should not happen for a range
/// found within a non-empty source, but callers may not always have a
/// better span at hand for the exceptional case).
pub(crate) fn lower(
    value: &RCodeText,
    diagnostics: &mut Diagnostics,
    fallback: Option<Span>,
) -> Vec<RdNode> {
    let source = value.as_str();
    let markers = crate::arity_adapter::r_code_markers(source);
    let mut issues = Vec::new();
    let nodes = lower_rlike(source, 0..source.len(), &markers, &mut issues);
    for issue in issues {
        let (range, code, message) = match issue {
            RawRdIssue::Unsupported(range) => (
                range,
                DiagnosticCode::UnsupportedRawRdMacro,
                "unsupported raw Rd macro",
            ),
            RawRdIssue::Unterminated(range) => (
                range,
                DiagnosticCode::UnterminatedRawRdMacro,
                "unterminated raw Rd macro",
            ),
        };
        let span = diagnostic_span(value, range, fallback);
        diagnostics.push(Diagnostic::new(
            code.default_severity(),
            code,
            message,
            Label::new(span, message),
        ));
    }
    nodes
}

/// Scans one R-like region (R-code escaping) for raw Rd macros.
///
/// Literal R code between and around recognized macros keeps going through
/// [`rcode_nodes`], exactly as it did before this module existed. Only the
/// spans covered by a recognized macro's introducer and braces are diverted
/// into a `Tagged` node.
fn lower_rlike(
    source: &str,
    region: Range<usize>,
    markers: &[crate::arity_adapter::RCodeMarker],
    issues: &mut Vec<RawRdIssue>,
) -> Vec<RdNode> {
    let mut nodes = Vec::new();
    let mut literal_start = region.start;
    let mut cursor = region.start;
    while cursor < region.end {
        if !matches!(
            marker_at(markers, cursor),
            Some(marker)
                if marker.kind == crate::arity_adapter::RCodeMarkerKind::BareBackslash
        ) {
            cursor += 1;
            continue;
        }
        if cursor + 1 < region.end && source.as_bytes()[cursor + 1] == b'(' {
            // R's lambda shorthand: the one backslash spelling that is
            // valid, working R outside a string. Leave it alone.
            cursor += 1;
            continue;
        }
        match scan_candidate(source, cursor, markers, region.end) {
            Candidate::Supported { tag, body, end } => {
                if literal_start < cursor {
                    nodes.extend(rcode_nodes(&source[literal_start..cursor]));
                }
                let children = if tag == RdTag::DontRun {
                    lower_verbatim(source, body)
                } else {
                    lower_rlike(source, body, markers, issues)
                };
                nodes.push(RdNode::tagged(tag, None, children));
                cursor = end;
                literal_start = end;
            }
            Candidate::Unsupported { end } => {
                issues.push(RawRdIssue::Unsupported(cursor..end));
                cursor = end;
            }
            Candidate::UnterminatedSupported { introducer_end } => {
                issues.push(RawRdIssue::Unterminated(cursor..introducer_end));
                cursor = region.end;
            }
        }
    }
    if literal_start < region.end {
        nodes.extend(rcode_nodes(&source[literal_start..region.end]));
    }
    nodes
}

/// The classification of a `BareBackslash` marker after its surrounding
/// source shape is scanned. The adapter has already established the positive
/// marker contract; this enum records whether that marker begins supported
/// raw Rd, an unsupported shape, or a known macro whose group is incomplete.
enum Candidate {
    /// A recognized macro with no bracket option, its matching closing
    /// brace found. `body` is the byte range strictly between the braces;
    /// `end` is the offset just past the closing brace.
    Supported {
        tag: RdTag,
        body: Range<usize>,
        end: usize,
    },
    /// Anything else shaped like an attempted macro (or nothing shaped at
    /// all): an unrecognized name, a recognized name combined with a
    /// bracket option (none of the five supported macros take one), a
    /// name with no brace group, a bracket option with no following brace
    /// (or whose own `]` never closes), or a lone backslash with no
    /// identifier after it. `end` is the offset just past whatever was
    /// scanned, for the caller to resume at.
    Unsupported { end: usize },
    /// One of the five supported macro names, with no bracket option,
    /// whose brace never closes within the region. `introducer_end` is the
    /// offset just past the opening brace (e.g. just past the `{` in
    /// `\dontrun{`): unlike [`Candidate::Unsupported`], the name itself is
    /// known and fine, so the caller's label should point at just the
    /// introducer instead of swallowing the rest of the region.
    UnterminatedSupported { introducer_end: usize },
}

/// Classifies the `BareBackslash` marker at `backslash`, which the caller has
/// already confirmed is not `\(`. A marker for `\(` remains ordinary R code;
/// every other marker is either one of the supported raw Rd macros or a shape
/// that produces an unsupported/unterminated diagnostic. No other source
/// context is rediscovered here.
fn scan_candidate(
    source: &str,
    backslash: usize,
    markers: &[crate::arity_adapter::RCodeMarker],
    region_end: usize,
) -> Candidate {
    let bytes = source.as_bytes();
    let Some((name, name_end)) = scan_macro_name(source, backslash, region_end) else {
        return Candidate::Unsupported {
            end: (backslash + 1).min(region_end),
        };
    };
    let mut cursor = name_end;
    let has_option = bytes.get(cursor) == Some(&b'[');
    if has_option {
        match scan_option(source, cursor, region_end) {
            Some(option_end) => cursor = option_end,
            // A bracket option makes this unsupported regardless of
            // whether it closes: none of the five supported macros take
            // one, so there is no "supported name, just unterminated"
            // case to distinguish here.
            None => return Candidate::Unsupported { end: region_end },
        }
    }
    if !matches!(
        marker_at(markers, cursor),
        Some(marker) if marker.kind == crate::arity_adapter::RCodeMarkerKind::OpenBrace
    ) {
        return Candidate::Unsupported { end: cursor };
    }
    let Some(close) = find_matching_close(source, cursor, markers, region_end) else {
        if !has_option && supported_tag(name).is_some() {
            return Candidate::UnterminatedSupported {
                introducer_end: cursor + 1,
            };
        }
        return Candidate::Unsupported { end: region_end };
    };
    if !has_option && let Some(tag) = supported_tag(name) {
        return Candidate::Supported {
            tag,
            body: cursor + 1..close - 1,
            end: close,
        };
    }
    Candidate::Unsupported { end: close }
}

/// Scans a `[...]` bracket option starting at `open` (the `[`), returning
/// the offset just past its closing `]`. A backslash escapes the next
/// character. The bracket's content is Rd option syntax, not R syntax, so it
/// is scanned directly rather than through the R token-marker stream.
fn scan_option(source: &str, open: usize, region_end: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    debug_assert_eq!(bytes.get(open), Some(&b'['));
    let mut cursor = open + 1;
    while cursor < region_end {
        match bytes[cursor] {
            b'\\' if cursor + 1 < region_end => cursor += 2,
            b']' => return Some(cursor + 1),
            _ => cursor += 1,
        }
    }
    None
}

/// Chunks a `\dontrun` body into physical-line [`RdNode::Verb`] leaves.
///
/// No macro scanning happens here: see the module documentation for why
/// `\dontrun` content is opaque, matching both R's own Rd parser and
/// `rd-writer`'s structural rule against tags inside verbatim content.
fn lower_verbatim(source: &str, region: Range<usize>) -> Vec<RdNode> {
    source[region]
        .split_inclusive('\n')
        .map(|line| RdNode::Verb(line.to_owned()))
        .collect()
}

fn supported_tag(name: &str) -> Option<RdTag> {
    match name {
        "dontrun" => Some(RdTag::DontRun),
        "donttest" => Some(RdTag::DontTest),
        "dontshow" => Some(RdTag::DontShow),
        "testonly" => Some(RdTag::TestOnly),
        "dontdiff" => Some(RdTag::DontDiff),
        _ => None,
    }
}

/// Recognizes a `\name` introducer at `backslash`, returning the name and
/// the offset just past it.
///
/// `\(` (R's lambda shorthand) is left alone here for free: `(` is not
/// `is_ascii_alphabetic`, so it never starts a name and the backslash falls
/// through to the caller's plain-text handling unchanged.
fn scan_macro_name(source: &str, backslash: usize, region_end: usize) -> Option<(&str, usize)> {
    let bytes = source.as_bytes();
    let name_start = backslash + 1;
    if name_start >= region_end || !bytes[name_start].is_ascii_alphabetic() {
        return None;
    }
    let mut name_end = name_start + 1;
    while name_end < region_end && bytes[name_end].is_ascii_alphanumeric() {
        name_end += 1;
    }
    Some((&source[name_start..name_end], name_end))
}

/// Finds the byte offset just past the structural brace marker that balances
/// the opening marker at `open`.
///
/// This decides which structural brace in the *original* R source the author
/// meant as the macro's end, regardless of which Rd content mode the
/// recognized macro ends up using.
fn find_matching_close(
    source: &str,
    open: usize,
    markers: &[crate::arity_adapter::RCodeMarker],
    region_end: usize,
) -> Option<usize> {
    debug_assert_eq!(source.as_bytes().get(open), Some(&b'{'));
    let mut depth: usize = 0;
    for marker in markers.iter().filter(|marker| marker.start >= open) {
        if marker.start >= region_end {
            break;
        }
        match marker.kind {
            crate::arity_adapter::RCodeMarkerKind::OpenBrace => {
                depth += 1;
            }
            crate::arity_adapter::RCodeMarkerKind::CloseBrace => {
                let close = marker.end;
                depth -= 1;
                if depth == 0 {
                    return Some(close);
                }
            }
            crate::arity_adapter::RCodeMarkerKind::BareBackslash => {}
        }
    }
    None
}

fn marker_at(
    markers: &[crate::arity_adapter::RCodeMarker],
    position: usize,
) -> Option<crate::arity_adapter::RCodeMarker> {
    markers
        .binary_search_by_key(&position, |marker| marker.start)
        .ok()
        .and_then(|index| markers.get(index).copied())
}

/// Builds a diagnostic span for a normalized byte range in `value`,
/// following the same collapse-to-one-span-per-file rule as
/// `tags::diagnostics::value_span_for_range`.
fn diagnostic_span(value: &RCodeText, range: Range<usize>, fallback: Option<Span>) -> Span {
    let start = u32::try_from(range.start).expect("normalized text length fits u32");
    let end = u32::try_from(range.end).expect("normalized text length fits u32");
    let spans = value.source_spans(TextRange::new(start, end));
    if let Some(first) = spans.first().copied() {
        let last = spans.last().copied().unwrap_or(first);
        return if first.file == last.file {
            Span::new(
                first.file,
                TextRange::new(first.range.start(), last.range.end()),
            )
        } else {
            first
        };
    }
    value
        .source_anchor_at(start)
        .or(fallback)
        .expect("a non-empty examples source has a source span for any range within it")
}

#[cfg(test)]
mod tests {
    use rd_ast::{RdNode, RdPath, RdTag};

    use super::supported_tag;

    /// Pins [`supported_tag`] to `rd-ast`'s own authoritative classification
    /// instead of a hand-copied literal table.
    ///
    /// `supported_tag`'s list has already drifted from upstream once (it
    /// was missing `\testonly`/`\dontdiff`, which made a topic using either
    /// one vanish entirely). A hand-written table has no way to notice a
    /// drift like that on its own; this test walks every `RdTag` variant
    /// `rd-ast` knows about, asks `rd-ast` itself (via
    /// `RdNode::example_control`, the same view the crate's own tag table is
    /// built from) whether it is one of the example-control macros, and
    /// asserts `supported_tag` agrees for every one of them. If `rd-ast`
    /// ever adds or removes an example-control tag, this fails immediately
    /// instead of silently reproducing the old set.
    #[test]
    fn supported_tag_matches_rd_ast_s_example_control_classification() {
        for tag in RdTag::KNOWN {
            let is_example_control = RdNode::tagged(tag.clone(), None, Vec::new())
                .example_control(&RdPath::new(Vec::new()))
                .is_some();
            let recognized_by_us = tag
                .as_rd_tag()
                .strip_prefix('\\')
                .is_some_and(|name| supported_tag(name).is_some());
            assert_eq!(
                recognized_by_us, is_example_control,
                "supported_tag disagrees with rd-ast's own example_control \
                 classification for {tag:?} (rd-ast: {is_example_control}, \
                 supported_tag: {recognized_by_us})"
            );
        }
    }
}
