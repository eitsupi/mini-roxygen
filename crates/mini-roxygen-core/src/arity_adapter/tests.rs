use std::path::PathBuf;

use super::test_support::inventory_texts;
use super::{BlockId, DocLine, RawTag, RoxyBlock, parse};
use crate::diagnostic::{DiagnosticCode, Severity};
use crate::source::{FileId, SourceFile};

fn parsed(text: &str) -> (Vec<RoxyBlock>, crate::diagnostic::Diagnostics, SourceFile) {
    parsed_exact(&format!(
        r#"{text}
NULL
"#
    ))
}

fn parsed_exact(text: &str) -> (Vec<RoxyBlock>, crate::diagnostic::Diagnostics, SourceFile) {
    let source = SourceFile::new(PathBuf::from("test.R"), text.to_owned());
    let parsed = parse(&source, FileId::new(7));
    let blocks = parsed
        .top_level
        .into_iter()
        .filter_map(|entry| entry.documentation)
        .collect();
    (blocks, parsed.diagnostics, source)
}

fn tag<'a>(block: &'a RoxyBlock, name: &str) -> &'a RawTag {
    block
        .tags
        .iter()
        .find(|tag| tag.name.value == name)
        .expect("tag not found")
}

#[test]
fn extracts_tag_name_and_untrimmed_value() {
    let (blocks, diagnostics, _) = parsed(
        r"#' Title
#' @param x A number.
",
    );
    assert!(diagnostics.is_empty());
    assert_eq!(tag(&blocks[0], "param").raw_value, " x A number.");
}

#[test]
fn intro_only_block_exposes_raw_body_and_no_tags() {
    let (blocks, diagnostics, _) = parsed(
        r"#' Title
#' More text.
",
    );
    assert!(diagnostics.is_empty());
    assert_eq!(blocks.len(), 1);
    assert_eq!(
        blocks[0].intro.as_ref().unwrap().raw_value,
        "Title\nMore text."
    );
    assert!(blocks[0].tags.is_empty());
}

#[test]
fn intro_is_separate_from_following_tags() {
    let (blocks, _, _) = parsed(
        r"#' Title
#' Description.
#' @param x A number.
",
    );
    assert_eq!(
        blocks[0].intro.as_ref().unwrap().raw_value,
        "Title\nDescription."
    );
    assert_eq!(tag(&blocks[0], "param").raw_value, " x A number.");
}

#[test]
fn a_wide_marker_separator_keeps_the_line_as_prose() {
    // roxygen2 consumes at most one whitespace character after the marker
    // before demanding the `@`, so two spaces make this description text.
    let (blocks, diagnostics, _) = parsed(
        r"#' Title
#'
#'  @param x A number.
",
    );
    assert!(diagnostics.is_empty());
    assert!(blocks[0].tags.is_empty());
    assert_eq!(
        blocks[0].intro.as_ref().expect("intro").raw_value,
        "Title\n\n @param x A number."
    );
}

#[test]
fn block_starting_with_tag_has_no_intro() {
    let (blocks, _, _) = parsed(
        r#"#' @export
"#,
    );
    assert!(blocks[0].intro.is_none());
}

#[test]
fn empty_leading_section_is_preserved_as_intro() {
    let (blocks, _, source) = parsed(
        r"#'
#' @export
",
    );
    let intro = blocks[0].intro.as_ref().expect("empty intro section");
    assert_eq!(intro.raw_value, "");
    assert_eq!(intro.value_lines.len(), 1);
    assert_eq!(
        intro.value_lines[0].range.start(),
        intro.value_lines[0].range.end()
    );
    assert_eq!(
        source.text_range(intro.full_span.range),
        Some(
            r#"#'
"#
        )
    );
}

#[test]
fn intro_preserves_blank_lines_between_paragraphs() {
    let (blocks, _, _) = parsed(
        r"#' First paragraph.
#'
#' Second paragraph.
",
    );
    assert_eq!(
        blocks[0].intro.as_ref().unwrap().raw_value,
        "First paragraph.\n\nSecond paragraph."
    );
}

#[test]
fn intro_value_lines_point_into_original_source() {
    let (blocks, _, source) = parsed(
        r"#' First
#'
#' Second
",
    );
    let intro = blocks[0].intro.as_ref().unwrap();
    assert_eq!(intro.value_lines.len(), 3);
    assert_eq!(source.text_range(intro.value_lines[0].range), Some("First"));
    assert_eq!(source.text_range(intro.value_lines[1].range), Some(""));
    assert_eq!(
        source.text_range(intro.value_lines[2].range),
        Some("Second")
    );
}

#[test]
fn intro_full_span_covers_intro_section() {
    let (blocks, _, source) = parsed(
        r"#' Title
#' @export
",
    );
    let intro = blocks[0].intro.as_ref().unwrap();
    let full_text = source.text_range(intro.full_span.range).unwrap();
    assert!(full_text.contains("Title"));
    assert!(!full_text.contains("@export"));
}

#[test]
fn crlf_intro_uses_normalized_line_endings() {
    let (blocks, _, source) = parsed("#' First\r\n#'\r\n#' Second\r\n");
    let intro = blocks[0].intro.as_ref().unwrap();
    assert_eq!(intro.raw_value, "First\n\nSecond");
    assert!(!intro.raw_value.contains('\r'));
    assert_eq!(source.text_range(intro.value_lines[0].range), Some("First"));
}

#[test]
fn explicit_title_tag_does_not_interpret_raw_intro() {
    let (blocks, _, _) = parsed(
        r"#' Intro paragraph.
#'
#' More intro.
#' @title Explicit title.
",
    );
    assert_eq!(
        blocks[0].intro.as_ref().unwrap().raw_value,
        "Intro paragraph.\n\nMore intro."
    );
    assert_eq!(tag(&blocks[0], "title").raw_value, " Explicit title.");
}

#[test]
fn keeps_tag_argument_in_raw_value() {
    let (blocks, _, _) = parsed(
        r#"#' @param x A number.
"#,
    );
    assert_eq!(tag(&blocks[0], "param").raw_value, " x A number.");
}

#[test]
fn joins_continuation_lines_after_prefix_removal() {
    let (blocks, _, _) = parsed(
        r"#' @details first
#' second
#' third
",
    );
    assert_eq!(
        tag(&blocks[0], "details").raw_value,
        " first\nsecond\nthird"
    );
}

#[test]
fn consumes_at_most_one_prefix_space() {
    let (blocks, _, source) = parsed(
        r"#' @details
#'   indented
",
    );
    let raw = tag(&blocks[0], "details");
    assert_eq!(raw.raw_value, "\n  indented");
    assert_eq!(
        source.text_range(raw.value_lines[1].range),
        Some("  indented")
    );
}

#[test]
fn accepts_multiple_hashes_in_marker() {
    assert_eq!(
        inventory_texts(
            r#"##' @export
NULL
"#
        ),
        vec!["NULL"]
    );
    let (blocks, diagnostics, source) = parsed_exact(
        r#"##' @export
NULL
"#,
    );
    assert!(diagnostics.is_empty());
    assert_eq!(blocks.len(), 1);
    assert_eq!(
        source.text_range(blocks[0].doc_lines[0].content_span.range),
        Some("@export")
    );
}

#[test]
fn value_line_spans_point_into_original_source() {
    let (blocks, _, source) = parsed(
        r"#' @details one
#' two
",
    );
    let raw = tag(&blocks[0], "details");
    assert_eq!(raw.value_lines.len(), 2);
    assert_eq!(source.text_range(raw.value_lines[0].range), Some(" one"));
    assert_eq!(source.text_range(raw.value_lines[1].range), Some("two"));
}

#[test]
fn empty_tag_has_empty_raw_value() {
    let (blocks, _, _) = parsed(
        r#"#' @export
"#,
    );
    assert_eq!(tag(&blocks[0], "export").raw_value, "");
}

#[test]
fn escaped_at_signs_remain_verbatim_and_do_not_start_tags() {
    // roxygen2's tokenizer unescapes `@@`, but doing that here would stop value spans
    // mapping to source bytes; unescaping belongs to the semantic tag layer.
    let (blocks, _, _) = parsed(
        r"#' @details one @@two
#' @@not-a-tag
#' @export
",
    );
    assert_eq!(blocks[0].tags.len(), 2);
    assert_eq!(
        tag(&blocks[0], "details").raw_value,
        " one @@two\n@@not-a-tag"
    );
    assert_eq!(tag(&blocks[0], "export").raw_value, "");
}

#[test]
fn preserves_zero_one_and_multiple_spaces_after_tag_names() {
    // roxygen2's tokenizer normalizes spacing, but doing that here would stop value spans
    // mapping to source bytes; normalization belongs to the semantic tag layer.
    let (blocks, _, _) = parsed(
        r"#' @export
#' @param x
#' @param   x
",
    );
    assert_eq!(blocks[0].tags[0].raw_value, "");
    assert_eq!(blocks[0].tags[1].raw_value, " x");
    assert_eq!(blocks[0].tags[2].raw_value, "   x");
}

#[test]
fn at_name_mid_line_stays_in_the_current_tag_value() {
    let (blocks, _, _) = parsed(
        r"#' @details first
#' continuation @name still
",
    );
    assert_eq!(blocks[0].tags.len(), 1);
    assert_eq!(
        tag(&blocks[0], "details").raw_value,
        " first\ncontinuation @name still"
    );
}

#[test]
fn separate_tags_survive_a_blank_roxygen_line() {
    let (blocks, _, _) = parsed(
        r"#' @export
#'
#' @name example
",
    );
    assert_eq!(
        blocks[0]
            .tags
            .iter()
            .map(|tag| tag.name.value.as_str())
            .collect::<Vec<_>>(),
        vec!["export", "name"]
    );
}

#[test]
fn tag_name_span_contains_only_the_name_text() {
    let (blocks, _, source) = parsed(
        r#"#' @details value
"#,
    );
    let raw = tag(&blocks[0], "details");
    assert_eq!(source.text_range(raw.name.span.range), Some("details"));
}

#[test]
fn tag_full_span_starts_at_the_at_sign() {
    let (blocks, _, source) = parsed(
        r"#' @details first
#' second
#' @export
",
    );
    let raw = tag(&blocks[0], "details");
    let full_text = source.text_range(raw.full_span.range).unwrap();
    assert_eq!(
        full_text,
        r#"@details first
#' second
"#
    );
}

#[test]
fn value_span_tracks_single_and_multiline_tag_values() {
    let (blocks, _, source) = parsed(
        r#"#' @details value
"#,
    );
    let single_line = tag(&blocks[0], "details");
    assert_eq!(
        source.text_range(single_line.value_span.range),
        Some(single_line.raw_value.as_str())
    );

    let (blocks, _, source) = parsed(
        r"#' @details first
#' second
#' third
",
    );
    let details = tag(&blocks[0], "details");
    assert_eq!(
        source.text_range(details.value_span.range),
        Some(
            r#" first
#' second
#' third"#
        )
    );
    assert_eq!(
        details.value_span.range.end(),
        details.value_lines.last().unwrap().range.end()
    );
}

#[test]
fn multiline_value_lines_cover_each_physical_line_including_blank_lines() {
    let (blocks, _, source) = parsed(
        r"#' @details first
#' second
#'
#' fourth
#' @export
",
    );
    let raw = tag(&blocks[0], "details");
    assert_eq!(raw.raw_value, " first\nsecond\n\nfourth");
    assert_eq!(raw.value_lines.len(), 4);
    let lines = raw
        .value_lines
        .iter()
        .map(|line| source.text_range(line.range).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(lines, vec![" first", "second", "", "fourth"]);
}

#[test]
fn empty_tag_has_one_empty_value_line_and_empty_value_span() {
    let (blocks, _, source) = parsed(
        r#"#' @export
"#,
    );
    let raw = tag(&blocks[0], "export");
    assert!(raw.value_span.range.is_empty());
    assert_eq!(raw.value_lines.len(), 1);
    assert!(raw.value_lines[0].range.is_empty());
    assert_eq!(source.text_range(raw.value_lines[0].range), Some(""));
}

#[test]
fn non_ascii_tag_value_uses_utf8_byte_spans() {
    let (blocks, _, source) = parsed(
        r#"#' @details 日本語 😀
"#,
    );
    let raw = tag(&blocks[0], "details");
    assert_eq!(source.text_range(raw.value_span.range), Some(" 日本語 😀"));
    assert_eq!(raw.value_lines.len(), 1);
    assert_eq!(
        source.text_range(raw.value_lines[0].range),
        Some(" 日本語 😀")
    );
}

#[test]
fn crlf_multiline_value_lines_exclude_carriage_returns() {
    let (blocks, _, source) = parsed(
        "#' @details first\r\n\
#' second\r\n\
#'\r\n\
#' fourth\r\n",
    );
    let raw = tag(&blocks[0], "details");
    assert_eq!(raw.raw_value, " first\nsecond\n\nfourth");
    assert_eq!(raw.value_lines.len(), 4);
    for line in &raw.value_lines {
        let text = source.text_range(line.range).unwrap();
        assert!(!text.contains('\r'));
    }
    assert_eq!(
        raw.value_lines
            .iter()
            .map(|line| source.text_range(line.range).unwrap())
            .collect::<Vec<_>>(),
        vec![" first", "second", "", "fourth"]
    );
    assert_eq!(
        raw.value_span.range.end(),
        raw.value_lines.last().unwrap().range.end()
    );
}

#[test]
fn trailing_blank_tag_lines_are_preserved() {
    let (blocks, _, _) = parsed(
        r"#' @details one
#'
#' @details two
#'
#'
#' @export
",
    );
    assert_eq!(blocks[0].tags.len(), 3);
    assert_eq!(blocks[0].tags[0].raw_value, " one\n");
    assert_eq!(blocks[0].tags[1].raw_value, " two\n\n");
}

#[test]
fn block_ids_follow_source_order() {
    let (blocks, _, _) = parsed(
        r"#' first
NULL
#' second
NULL
",
    );
    assert_eq!(
        blocks.iter().map(|block| block.id).collect::<Vec<_>>(),
        vec![BlockId::new(0), BlockId::new(1)]
    );
}

#[test]
fn pairs_each_expression_with_its_optional_documentation() {
    let source = SourceFile::new(
        PathBuf::from("test.R"),
        r#"NULL
#' second
x <- 1
#' third
"value"
"#
        .to_owned(),
    );
    let parsed = parse(&source, FileId::new(7));

    assert!(parsed.diagnostics.is_empty());
    assert_eq!(parsed.top_level.len(), 3);
    assert!(parsed.top_level[0].documentation.is_none());
    assert_eq!(
        source.text_range(parsed.top_level[0].fact.span.range),
        Some("NULL")
    );
    assert_eq!(
        parsed.top_level[1]
            .documentation
            .as_ref()
            .expect("second expression should have documentation")
            .id,
        BlockId::new(0)
    );
    assert_eq!(
        source.text_range(parsed.top_level[1].fact.span.range),
        Some("x <- 1")
    );
    assert_eq!(
        parsed.top_level[2]
            .documentation
            .as_ref()
            .expect("third expression should have documentation")
            .id,
        BlockId::new(1)
    );
    assert_eq!(
        source.text_range(parsed.top_level[2].fact.span.range),
        Some("\"value\"")
    );
}

#[test]
fn no_md_is_unsupported_but_md_is_redundant() {
    let (blocks, diagnostics, _) = parsed(
        r"#' @md
#' @noMd
",
    );
    assert_eq!(blocks.len(), 1);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics.iter().next().unwrap().code,
        DiagnosticCode::UnsupportedTag
    );
    assert_eq!(
        diagnostics.iter().next().unwrap().primary.span,
        tag(&blocks[0], "noMd").full_span
    );
}

#[test]
fn syntax_errors_are_reported_and_block_extraction_fails_closed() {
    let (blocks, diagnostics, _) = parsed_exact(
        r"#' Title
f <- (
",
    );
    assert!(blocks.is_empty());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DiagnosticCode::RSyntaxError)
    );
}

#[test]
fn crlf_lines_are_split_without_carriage_returns() {
    let (blocks, _, source) = parsed("#' @details one\r\n#' two\r\n");
    let raw = tag(&blocks[0], "details");
    assert_eq!(raw.raw_value, " one\ntwo");
    assert_eq!(
        source.text_range(blocks[0].doc_lines[0].span.range),
        Some("#' @details one")
    );
}

#[test]
fn examples_body_is_verbatim() {
    let (blocks, _, _) = parsed(
        r"#' @examples
#'   x <- 1
#'     x + 1
",
    );
    assert_eq!(
        tag(&blocks[0], "examples").raw_value,
        "\n  x <- 1\n    x + 1"
    );
}

#[test]
fn wrapped_inline_rd_macros_remain_in_their_tag_value() {
    let (blocks, diagnostics, _) = parsed_exact(
        r#"#' @param x prose that continues onto the next line, where the opener sits
#'   mid-prose: \code{c("top",
#'   "bottom")}
#' @param y prose whose continuation line starts with the opener,
#'   \code{c("left",
#'   "right")}
#' @name wrapped_inline_macro
NULL
"#,
    );
    assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    assert_eq!(blocks.len(), 1);
    assert_eq!(
        tag(&blocks[0], "param").raw_value,
        " x prose that continues onto the next line, where the opener sits\n  mid-prose: \\code{c(\"top\",\n  \"bottom\")}"
    );
    let second_param = blocks[0]
        .tags
        .iter()
        .filter(|tag| tag.name.value == "param")
        .nth(1)
        .expect("second parameter");
    assert!(
        second_param
            .raw_value
            .contains("\\code{c(\"left\",\n  \"right\")}")
    );
}

#[test]
fn function_body_roxygen_comments_attach_to_the_containing_expression() {
    let (blocks, _, _) = parsed_exact(
        r"f <- function() {
  #' @export
  NULL
}
",
    );
    assert_eq!(blocks.len(), 1);
    assert_eq!(tag(&blocks[0], "export").raw_value, "");
}

#[test]
fn call_argument_roxygen_comments_attach_to_the_containing_expression() {
    let (blocks, _, _) = parsed_exact(
        r"f(
  #' @export
  1
)
",
    );
    assert_eq!(blocks.len(), 1);
    assert_eq!(tag(&blocks[0], "export").raw_value, "");
}

#[test]
fn all_expression_window_blocks_are_extracted_with_contiguous_ids() {
    let (blocks, _, _) = parsed_exact(
        r"#' @name first
f <- function() {
  #' @export
  NULL
}
f(
  #' @export
  1
)
#' @name second
NULL
",
    );
    assert_eq!(blocks.len(), 3);
    assert_eq!(tag(&blocks[0], "name").raw_value, " first");
    assert_eq!(tag(&blocks[0], "export").raw_value, "");
    assert_eq!(tag(&blocks[1], "export").raw_value, "");
    assert_eq!(tag(&blocks[2], "name").raw_value, " second");
    assert_eq!(
        blocks.iter().map(|block| block.id).collect::<Vec<_>>(),
        vec![BlockId::new(0), BlockId::new(1), BlockId::new(2)]
    );
}

#[test]
fn trailing_top_level_block_is_dropped_without_a_following_expression() {
    let (blocks, diagnostics, _) = parsed_exact(
        r#"#' @export
"#,
    );
    assert!(blocks.is_empty());
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics.iter().next().unwrap().code,
        DiagnosticCode::UnattachedRoxygenBlock
    );
}

#[test]
fn expression_windows_match_the_measured_roxygen_cases() {
    let cases = [
        (
            "two parameter runs",
            r#"#' @param x one

#' @param y two
f <- function(x, y) NULL
"#,
            1,
            &["param= x one", "param= y two"][..],
        ),
        (
            "two prose runs",
            r#"#' Title one

#' Title two
g <- function() NULL
"#,
            1,
            &["title=Title one\nTitle two"][..],
        ),
        (
            "param continuation",
            r#"#' @param x one

#' trailing prose
f <- function(x) NULL
"#,
            1,
            &["param= x one\ntrailing prose"][..],
        ),
        (
            "nested tag",
            r#"#' Title
f <- function(x) {
  #' @param y two
  NULL
}
"#,
            1,
            &["title=Title", "param= y two"][..],
        ),
        (
            "nested prose",
            r#"#' Title
f <- function(x) {
  #' body prose
  NULL
}
"#,
            1,
            &["title=Title\nbody prose"][..],
        ),
        (
            "trailing block",
            r#"f <- function(x) x
#' @param x trailing
"#,
            0,
            &[][..],
        ),
        (
            "roxygen only",
            r#"#' Title
"#,
            0,
            &[][..],
        ),
        (
            "closed by bare token",
            r#"#' First
NULL
#' Second
"value"
"#,
            2,
            &["title=First", "title=Second"][..],
        ),
        (
            "semicolon trailing comment",
            r#"f <- function(x) { x }; #' @seealso somewhere
"#,
            0,
            &[][..],
        ),
        (
            "semicolon before inline marker",
            r#"x <- 1; #' @param z inline
y <- function(z) z
"#,
            0,
            &[][..],
        ),
        (
            "space before inline marker",
            r#"x <- 1 #' @param z inline
y <- function(z) z
"#,
            1,
            &["param= z inline"][..],
        ),
    ];

    for (name, text, block_count, expected) in cases {
        let (blocks, diagnostics, _) = parsed_exact(text);
        assert_eq!(blocks.len(), block_count, "case {name}");
        let actual = blocks
            .iter()
            .flat_map(|block| {
                block
                    .intro
                    .iter()
                    .map(|intro| format!("title={}", intro.raw_value))
                    .chain(
                        block
                            .tags
                            .iter()
                            .map(|tag| format!("{}={}", tag.name.value, tag.raw_value)),
                    )
            })
            .collect::<Vec<_>>();
        for expected_value in expected {
            assert!(
                actual.iter().any(|value| value == expected_value),
                "case {name}: missing {expected_value:?} in {actual:?}"
            );
        }
        if block_count > 0 {
            assert!(
                diagnostics
                    .iter()
                    .all(|diagnostic| diagnostic.code != DiagnosticCode::UnattachedRoxygenBlock),
                "case {name}: unexpected unattached diagnostic"
            );
        }
    }
}

#[test]
fn ordinary_trailing_comments_do_not_close_the_next_window() {
    let (blocks, diagnostics, _) = parsed_exact(
        r#"#' First
f <- function() NULL # trailing comment
#' Second
g <- function() NULL
"#,
    );
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].intro.as_ref().unwrap().raw_value, "First");
    assert_eq!(blocks[1].intro.as_ref().unwrap().raw_value, "Second");
    assert!(diagnostics.is_empty());
}

#[test]
fn a_marker_inside_preceding_code_is_not_the_block_marker() {
    // The eligibility prefix runs from the window start to the marker, so
    // taking a `#'` that belongs to the code for the marker rejects the
    // real documentation. All three of these match roxygen2 8.1.0.
    let (blocks, _, _) = parsed_exact(
        r##"x <- "#'" #' @param z inline
y <- function(z) z
"##,
    );
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].tags[0].name.value, "param");
    assert_eq!(blocks[0].tags[0].raw_value, " z inline");

    // A semicolon still ends the expression before the marker.
    let (blocks, _, _) = parsed_exact(
        r##"x <- "#'"; #' @param z inline
y <- function(z) z
"##,
    );
    assert!(blocks.is_empty());

    // A `#'` inside an ordinary comment does not open a block either.
    let (blocks, _, _) = parsed_exact(
        r#"x <- 1 # a #' not a marker
y <- function(z) z
"#,
    );
    assert!(blocks.is_empty());
}

#[test]
fn ordinary_blank_and_hash_comments_are_not_content_lines() {
    let (blocks, diagnostics, _) = parsed_exact(
        r#"#' First

# ordinary comment
#' Second
f <- function() NULL
"#,
    );
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].intro.as_ref().unwrap().raw_value, "First\nSecond");
    assert!(diagnostics.is_empty());
}

#[test]
fn partially_dropped_block_restarts_reduction_for_the_accepted_lines() {
    let text = r#"f <- function() { x }; #' @seealso dropped
#' accepted prose
g <- function() NULL
"#;
    let (blocks, diagnostics, source) = parsed_exact(text);
    assert_eq!(blocks.len(), 1);
    assert_eq!(
        blocks[0].intro.as_ref().unwrap().raw_value,
        "accepted prose"
    );
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == DiagnosticCode::UnattachedRoxygenBlock)
        .expect("dropped line diagnostic");
    assert_eq!(
        source.text_range(diagnostic.primary.span.range),
        Some("#' @seealso dropped")
    );
}

#[test]
fn unattached_diagnostic_has_the_stable_warning_shape() {
    let (blocks, diagnostics, source) = parsed_exact(
        r#"f <- function() NULL
#' @export
"#,
    );
    assert!(blocks.is_empty());
    let diagnostic = diagnostics.iter().next().expect("unattached diagnostic");
    assert_eq!(diagnostic.code, DiagnosticCode::UnattachedRoxygenBlock);
    assert_eq!(diagnostic.severity, Severity::Warning);
    assert_eq!(diagnostic.primary.message, "unattached roxygen block");
    assert_eq!(
        source.text_range(diagnostic.primary.span.range),
        Some("#' @export")
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("Move this block before a following top-level expression, or remove it.")
    );
}

#[test]
fn downstream_tag_parsing_preserves_discontinuous_source_provenance() {
    let text = r#"#' @param x one

#' trailing prose
f <- function(x) NULL
"#;
    let source = SourceFile::new(PathBuf::from("test.R"), text.to_owned());
    let parsed = parse(&source, FileId::new(7));
    let blocks = parsed
        .top_level
        .into_iter()
        .filter_map(|entry| entry.documentation)
        .collect::<Vec<_>>();
    let diagnostics = parsed.diagnostics;
    assert!(diagnostics.is_empty());
    let (tags, diagnostics) = crate::tags::parse_block(
        &source,
        &blocks[0],
        &crate::tags::TagParseOptions {
            unknown_tags: crate::tags::UnknownTagPolicy::Warn,
        },
    );
    assert!(diagnostics.is_empty());
    let crate::tags::ParsedTag::Param { description, .. } = &tags[0] else {
        panic!("expected parsed parameter");
    };
    assert_eq!(description.as_str(), "one\ntrailing prose");
    let spans = description
        .sourced()
        .source_spans(crate::source::TextRange::new(
            0,
            description.as_str().len() as u32,
        ));
    assert_eq!(
        spans
            .iter()
            .map(|span| source.text_range(span.range).unwrap())
            .collect::<Vec<_>>(),
        vec!["one\n", "trailing prose"]
    );
}

#[allow(dead_code)]
fn _doc_line_is_publicly_constructible(_: DocLine) {}
