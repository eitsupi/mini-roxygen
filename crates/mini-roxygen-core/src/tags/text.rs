//! Normalized tag text with source-byte provenance.

use crate::arity_adapter::RawBody;
use crate::source::{SourceFile, Span, TextRange};

/// Selects the small amount of head normalization applied to raw lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum NormalizeHead {
    /// Remove one separator byte from the first tag-value line.
    TagValue,
    /// Preserve the first byte; this is used for untagged intro text.
    Intro,
}

/// Normalized text whose every byte can be projected back to source.
///
/// Runs are sorted, non-overlapping, and cover `0..text.len()` whenever the
/// text is non-empty. Adjacent runs are merged when their normalized and source
/// ranges are contiguous and both runs are affine. An affine run has equal
/// normalized and source byte lengths, so its mapping is byte-for-byte. When
/// those lengths differ, every normalized byte in the run maps to the whole
/// source span. This represents an escaped `@` from `@@`, a normalized newline
/// from a CRLF terminator, and synthetic text. Synthetic text has a zero-width
/// source span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcedText {
    text: String,
    runs: Vec<OriginRun>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct OriginRun {
    /// Range within [`SourcedText::text`].
    normalized: TextRange,
    /// Source bytes this run represents.
    source: Span,
}

impl SourcedText {
    /// Creates generated text anchored at a source span.
    #[must_use]
    pub fn synthetic(value: impl Into<String>, anchor: Span) -> Self {
        let value = value.into();
        let mut output = Self {
            text: value,
            runs: Vec::new(),
        };
        if !output.text.is_empty() {
            output.runs.push(OriginRun {
                normalized: TextRange::new(0, output.text.len() as u32),
                source: Span::new(
                    anchor.file,
                    TextRange::new(anchor.range.start(), anchor.range.start()),
                ),
            });
        }
        output
    }

    /// Normalizes raw physical lines while retaining their source projection.
    ///
    /// `value_lines` must be source spans belonging to `source_file`, as
    /// produced by the raw adapter. Tag values lose at most one ASCII
    /// whitespace byte at the start of their first physical source span when
    /// that span is non-empty; intro text does not lose that separator. Each
    /// `@@` pair becomes one `@` while scanning left to right, so `@@@` becomes
    /// `@@` (one escaped pair and one literal trailing `@`). Physical lines are
    /// joined with one normalized newline.
    #[must_use]
    pub(crate) fn from_lines(
        source_file: &SourceFile,
        value_lines: &[Span],
        mode: NormalizeHead,
    ) -> Self {
        let mut output = Self {
            text: String::new(),
            runs: Vec::new(),
        };

        for (line_index, line_span) in value_lines.iter().copied().enumerate() {
            let line_text = source_file
                .text_range(line_span.range)
                .expect("raw value spans must be valid source ranges");
            let mut start = line_span.range.start();
            let end = line_span.range.end();

            if line_index == 0
                && mode == NormalizeHead::TagValue
                && start < end
                && source_file.text().as_bytes()
                    [usize::try_from(start).expect("source offset fits usize")]
                .is_ascii_whitespace()
            {
                start += 1;
            }

            let separator_stripped_offset =
                usize::try_from(start - line_span.range.start()).expect("source offset fits usize");
            let line_text = &line_text[separator_stripped_offset..];
            output.push_normalized_line(line_span.file, start, end, line_text);

            if line_index + 1 < value_lines.len() {
                let terminator = line_terminator(source_file.text(), end, line_span.file);
                output.push_mapped("\n", terminator);
            }
        }

        output
    }

    /// Normalizes the raw value of an intro body without consuming the body.
    #[must_use]
    pub(crate) fn from_body(source_file: &SourceFile, body: &RawBody) -> Self {
        Self::from_lines(source_file, &body.value_lines, NormalizeHead::Intro)
    }

    /// Removes the common indentation from continuation lines while retaining
    /// the first line and every surviving byte's source projection.
    ///
    /// This applies the two-part tag normalization needed by ordinary
    /// descriptions. The first line is already split from its tag prefix;
    /// only the minimum ASCII-space indentation of later lines is removed.
    #[must_use]
    pub(super) fn trim_continuation_indent(self) -> Self {
        let starts = std::iter::once(0usize)
            .chain(self.text.match_indices('\n').map(|(index, _)| index + 1))
            .collect::<Vec<_>>();
        if starts.len() <= 1 {
            return self;
        }

        let mut indent = usize::MAX;
        for (index, start) in starts.iter().copied().enumerate().skip(1) {
            let end = starts.get(index + 1).copied().unwrap_or(self.text.len());
            let leading = self.text[start..end]
                .bytes()
                .take_while(|byte| *byte == b' ')
                .count();
            indent = indent.min(leading);
        }
        if indent == 0 || indent == usize::MAX {
            return self;
        }

        let mut output = Self {
            text: String::new(),
            runs: Vec::new(),
        };
        let mut cursor = 0usize;
        for start in starts.into_iter().skip(1) {
            let remove_end = start + indent;
            output.append_part(self.slice(TextRange::new(
                u32::try_from(cursor).expect("normalized offset fits u32"),
                u32::try_from(start).expect("normalized offset fits u32"),
            )));
            cursor = remove_end;
        }
        output.append_part(self.slice(TextRange::new(
            u32::try_from(cursor).expect("normalized offset fits u32"),
            u32::try_from(self.text.len()).expect("normalized text length fits u32"),
        )));
        output
    }

    /// Concatenates non-empty sourced texts with synthetic separators.
    ///
    /// Empty parts are ignored, including when they occur at either end, so
    /// separators are emitted only between parts that contribute text. Each
    /// separator is mapped to `anchor` as a zero-width source span. This keeps
    /// the provenance runs covering the complete normalized text while making
    /// synthetic join bytes distinguishable from source-backed bytes.
    #[must_use]
    pub fn concat_with(
        parts: impl IntoIterator<Item = Self>,
        separator: &str,
        anchor: Span,
    ) -> Self {
        let mut output = Self {
            text: String::new(),
            runs: Vec::new(),
        };
        let mut has_part = false;

        for part in parts {
            if part.is_empty() {
                continue;
            }
            if has_part {
                output.push_mapped(separator, anchor);
            }
            output.append_part(part);
            has_part = true;
        }

        output
    }

    /// Returns the normalized text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Returns whether the normalized text is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Returns a provenance-preserving view of a normalized byte range.
    ///
    /// The range must be within the text and on UTF-8 character boundaries.
    #[must_use]
    pub fn slice(&self, range: TextRange) -> Self {
        let start = usize::try_from(range.start()).expect("normalized offset fits usize");
        let end = usize::try_from(range.end()).expect("normalized offset fits usize");
        assert!(end <= self.text.len(), "normalized slice exceeds text");
        assert!(
            self.text.is_char_boundary(start),
            "slice starts inside UTF-8"
        );
        assert!(self.text.is_char_boundary(end), "slice ends inside UTF-8");

        let mut sliced = Self {
            text: self.text[start..end].to_owned(),
            runs: Vec::new(),
        };
        for run in &self.runs {
            let intersection_start = run.normalized.start().max(range.start());
            let intersection_end = run.normalized.end().min(range.end());
            if intersection_start >= intersection_end {
                continue;
            }

            let source = if is_affine(*run) {
                let offset = intersection_start - run.normalized.start();
                Span::new(
                    run.source.file,
                    TextRange::new(
                        run.source.range.start() + offset,
                        run.source.range.start() + offset + (intersection_end - intersection_start),
                    ),
                )
            } else {
                run.source
            };
            sliced.push_run(
                TextRange::new(
                    intersection_start - range.start(),
                    intersection_end - range.start(),
                ),
                source,
            );
        }
        sliced
    }

    /// Returns the source span of the whole character at a normalized byte
    /// offset.
    ///
    /// The offset must be a UTF-8 character boundary. At `text.len()`, this
    /// retains the historical end-of-value behavior and returns a zero-width
    /// span at the end of the last run. Use [`Self::source_anchor_at`] when a
    /// position rather than a character is wanted.
    #[must_use]
    pub fn source_span_at(&self, offset: u32) -> Option<Span> {
        let text_len = u32::try_from(self.text.len()).expect("text length fits u32");
        if offset > text_len || self.runs.is_empty() {
            return None;
        }
        let normalized_offset = usize::try_from(offset).expect("normalized offset fits usize");
        if !self.text.is_char_boundary(normalized_offset) {
            return None;
        }
        if offset == text_len {
            let source_end = self.runs.last()?.source.range.end();
            return Some(Span::new(
                self.runs.last()?.source.file,
                TextRange::new(source_end, source_end),
            ));
        }

        let index = self
            .runs
            .partition_point(|run| run.normalized.end() <= offset);
        let run = self.runs.get(index)?;
        if !run.normalized.contains(offset) {
            return None;
        }
        if is_affine(*run) {
            let source_start = run.source.range.start() + offset - run.normalized.start();
            let character_len =
                u32::try_from(self.text[normalized_offset..].chars().next()?.len_utf8())
                    .expect("character length fits u32");
            Some(Span::new(
                run.source.file,
                TextRange::new(source_start, source_start + character_len),
            ))
        } else {
            Some(run.source)
        }
    }

    /// Returns a zero-width source span at a normalized byte offset.
    ///
    /// The offset must be a UTF-8 character boundary. For a source-backed
    /// affine run, the anchor is mapped to the corresponding source byte. For
    /// a normalized run that represents escaped or synthetic text, the run's
    /// source anchor is used. At `text.len()`, the anchor is at the end of the
    /// last run.
    #[must_use]
    pub fn source_anchor_at(&self, offset: u32) -> Option<Span> {
        let text_len = u32::try_from(self.text.len()).expect("text length fits u32");
        if offset > text_len || self.runs.is_empty() {
            return None;
        }
        let normalized_offset = usize::try_from(offset).expect("normalized offset fits usize");
        if !self.text.is_char_boundary(normalized_offset) {
            return None;
        }
        if offset == text_len {
            let last = self.runs.last()?;
            let source_end = last.source.range.end();
            return Some(Span::new(
                last.source.file,
                TextRange::new(source_end, source_end),
            ));
        }

        let index = self
            .runs
            .partition_point(|run| run.normalized.end() <= offset);
        let run = self.runs.get(index)?;
        if !run.normalized.contains(offset) {
            return None;
        }
        let source_offset = if is_affine(*run) {
            run.source.range.start() + offset - run.normalized.start()
        } else {
            run.source.range.start()
        };
        Some(Span::new(
            run.source.file,
            TextRange::new(source_offset, source_offset),
        ))
    }

    /// Returns source spans represented by a normalized range.
    ///
    /// Spans are merged only when they belong to the same file and are
    /// adjacent in source. Thus a normalized range that skips source bytes
    /// remains represented by multiple spans.
    #[must_use]
    pub fn source_spans(&self, range: TextRange) -> Vec<Span> {
        assert!(range.end() <= u32::try_from(self.text.len()).expect("text length fits u32"));
        let start = usize::try_from(range.start()).expect("normalized offset fits usize");
        let end = usize::try_from(range.end()).expect("normalized offset fits usize");
        assert!(
            self.text.is_char_boundary(start),
            "source range starts inside UTF-8"
        );
        assert!(
            self.text.is_char_boundary(end),
            "source range ends inside UTF-8"
        );
        let mut spans = Vec::new();
        for run in &self.runs {
            let start = run.normalized.start().max(range.start());
            let end = run.normalized.end().min(range.end());
            if start >= end {
                continue;
            }
            let source = if is_affine(*run) {
                let source_start = run.source.range.start() + start - run.normalized.start();
                Span::new(
                    run.source.file,
                    TextRange::new(source_start, source_start + (end - start)),
                )
            } else {
                run.source
            };
            push_merged_span(&mut spans, source);
        }
        spans
    }

    fn push_normalized_line(
        &mut self,
        file: crate::source::FileId,
        start: u32,
        end: u32,
        line_text: &str,
    ) {
        let bytes = line_text.as_bytes();
        let mut cursor = 0;
        while cursor < bytes.len() {
            let next_escape = (cursor..bytes.len())
                .find(|&index| bytes[index] == b'@' && bytes.get(index + 1) == Some(&b'@'));
            let next = next_escape.unwrap_or(bytes.len());
            if next > cursor {
                let chunk = &line_text[cursor..next];
                self.push_mapped(
                    chunk,
                    Span::new(
                        file,
                        TextRange::new(
                            start + u32::try_from(cursor).expect("line length fits u32"),
                            start + u32::try_from(next).expect("line length fits u32"),
                        ),
                    ),
                );
                cursor = next;
            } else {
                self.push_mapped(
                    "@",
                    Span::new(
                        file,
                        TextRange::new(
                            start + u32::try_from(cursor).expect("line length fits u32"),
                            start + u32::try_from(cursor + 2).expect("line length fits u32"),
                        ),
                    ),
                );
                cursor += 2;
            }
        }

        let expected_len = end - start;
        let actual_len = u32::try_from(line_text.len()).expect("line length fits u32");
        assert_eq!(
            expected_len, actual_len,
            "line text must match its source range"
        );
    }

    fn push_mapped(&mut self, text: &str, source: Span) {
        let start = u32::try_from(self.text.len()).expect("normalized text length fits u32");
        self.text.push_str(text);
        let end = u32::try_from(self.text.len()).expect("normalized text length fits u32");
        self.push_run(TextRange::new(start, end), source);
    }

    fn append_part(&mut self, part: Self) {
        let offset = u32::try_from(self.text.len()).expect("normalized text length fits u32");
        self.text.push_str(&part.text);
        for run in part.runs {
            self.push_run(
                TextRange::new(
                    run.normalized.start() + offset,
                    run.normalized.end() + offset,
                ),
                run.source,
            );
        }
    }

    fn push_run(&mut self, normalized: TextRange, source: Span) {
        if normalized.is_empty() {
            return;
        }
        if let Some(previous) = self.runs.last_mut()
            && is_affine(*previous)
            && normalized.start() == previous.normalized.end()
            && source.file == previous.source.file
            && source.range.start() == previous.source.range.end()
            && is_affine(OriginRun { normalized, source })
        {
            previous.normalized = TextRange::new(previous.normalized.start(), normalized.end());
            previous.source.range =
                TextRange::new(previous.source.range.start(), source.range.end());
            return;
        }
        self.runs.push(OriginRun { normalized, source });
    }
}

fn is_affine(run: OriginRun) -> bool {
    run.normalized.len() == run.source.range.len()
}

fn line_terminator(text: &str, offset: u32, file: crate::source::FileId) -> Span {
    let offset = usize::try_from(offset).expect("source offset fits usize");
    let source_offset = u32::try_from(offset).expect("source offset fits u32");
    let bytes = text.as_bytes();
    let range = if bytes.get(offset) == Some(&b'\r') && bytes.get(offset + 1) == Some(&b'\n') {
        let end = u32::try_from(offset + 2).expect("source offset fits u32");
        TextRange::new(source_offset, end)
    } else if bytes.get(offset) == Some(&b'\n') {
        let end = u32::try_from(offset + 1).expect("source offset fits u32");
        TextRange::new(source_offset, end)
    } else {
        TextRange::new(source_offset, source_offset)
    };
    Span::new(file, range)
}

fn push_merged_span(spans: &mut Vec<Span>, span: Span) {
    if let Some(previous) = spans.last_mut()
        && previous.file == span.file
        && previous.range.end() == span.range.start()
    {
        previous.range = TextRange::new(previous.range.start(), span.range.end());
    } else {
        spans.push(span);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{NormalizeHead, SourcedText};
    use crate::source::{FileId, SourceFile, Span, TextRange};

    fn source(text: &str) -> SourceFile {
        SourceFile::new(PathBuf::from("test.R"), text.to_owned())
    }

    fn span(start: u32, end: u32) -> Span {
        Span::new(FileId::new(0), TextRange::new(start, end))
    }

    fn value(text: &str, range: TextRange, mode: NormalizeHead) -> SourcedText {
        let file = source(text);
        SourcedText::from_lines(&file, &[Span::new(FileId::new(0), range)], mode)
    }

    fn assert_projected_text(
        file: &SourceFile,
        sourced: &SourcedText,
        offset: u32,
        expected: &str,
    ) {
        let projected = sourced
            .source_span_at(offset)
            .expect("normalized offset should project to a source span");
        assert_eq!(file.text_range(projected.range), Some(expected));
    }

    #[test]
    fn identity_and_separator_normalization() {
        let sourced = value(" hello", TextRange::new(0, 6), NormalizeHead::TagValue);
        assert_eq!(sourced.as_str(), "hello");
        assert_eq!(sourced.source_spans(TextRange::new(0, 5)), vec![span(1, 6)]);

        let no_separator = value("hello", TextRange::new(0, 5), NormalizeHead::TagValue);
        assert_eq!(no_separator.as_str(), "hello");

        let spaces = value("   hello", TextRange::new(0, 8), NormalizeHead::TagValue);
        assert_eq!(spaces.as_str(), "  hello");
    }

    #[test]
    fn intro_preserves_separator_and_empty_lines_do_not_consume_newline() {
        let intro = value(" hello", TextRange::new(0, 6), NormalizeHead::Intro);
        assert_eq!(intro.as_str(), " hello");

        let file = source("\nnext");
        let sourced =
            SourcedText::from_lines(&file, &[span(0, 0), span(1, 5)], NormalizeHead::TagValue);
        assert_eq!(sourced.as_str(), "\nnext");
        assert_eq!(sourced.source_spans(TextRange::new(0, 1)), vec![span(0, 1)]);
    }

    #[test]
    fn escaped_at_signs_are_left_to_right_and_utf8_safe() {
        let raw = "@@ @@@ @@日本語";
        let sourced = value(
            raw,
            TextRange::new(0, raw.len() as u32),
            NormalizeHead::Intro,
        );
        assert_eq!(sourced.as_str(), "@ @@ @日本語");
        assert_eq!(sourced.source_span_at(0), Some(span(0, 2)));
        assert_eq!(sourced.source_spans(TextRange::new(0, 1)), vec![span(0, 2)]);
        assert_eq!(sourced.source_span_at(3), Some(span(5, 6)));
    }

    #[test]
    fn character_projection_round_trips_utf8_characters() {
        let file = source("é日本語");
        let sourced = SourcedText::from_lines(
            &file,
            &[span(
                0,
                u32::try_from(file.text().len()).expect("text length fits u32"),
            )],
            NormalizeHead::Intro,
        );

        assert_projected_text(&file, &sourced, 0, "é");
        assert_projected_text(&file, &sourced, 2, "日");
        assert_projected_text(&file, &sourced, 5, "本");
        assert_projected_text(&file, &sourced, 8, "語");
        assert_projected_text(&file, &sourced, 11, "");
        assert_eq!(sourced.source_span_at(1), None);
        assert_eq!(sourced.source_anchor_at(1), None);

        let anchor = sourced.source_anchor_at(2).expect("character boundary");
        assert_eq!(file.text_range(anchor.range), Some(""));
        assert_eq!(anchor.range, TextRange::new(2, 2));
        let end_anchor = sourced.source_anchor_at(11).expect("text end");
        assert_eq!(file.text_range(end_anchor.range), Some(""));
    }

    #[test]
    fn escaped_at_projection_round_trips_following_multibyte_text() {
        let file = source("@@é日本語");
        let sourced = SourcedText::from_lines(
            &file,
            &[span(
                0,
                u32::try_from(file.text().len()).expect("text length fits u32"),
            )],
            NormalizeHead::Intro,
        );

        assert_projected_text(&file, &sourced, 0, "@@");
        assert_projected_text(&file, &sourced, 1, "é");
        assert_projected_text(&file, &sourced, 3, "日");
        assert_projected_text(&file, &sourced, 6, "本");
        assert_projected_text(&file, &sourced, 9, "語");
    }

    #[test]
    fn character_projection_round_trips_crlf_and_multibyte_text() {
        let file = source("é\r\n日本語");
        let sourced =
            SourcedText::from_lines(&file, &[span(0, 2), span(4, 13)], NormalizeHead::Intro);

        assert_projected_text(&file, &sourced, 0, "é");
        assert_projected_text(&file, &sourced, 2, "\r\n");
        assert_projected_text(&file, &sourced, 3, "日");
        assert_projected_text(&file, &sourced, 6, "本");
        assert_projected_text(&file, &sourced, 9, "語");
        assert_projected_text(&file, &sourced, 12, "");
    }

    #[test]
    fn character_projection_round_trips_synthetic_separator_between_multibyte_text() {
        let file = source("é日本語");
        let first = SourcedText::from_lines(&file, &[span(0, 2)], NormalizeHead::Intro);
        let second = SourcedText::from_lines(&file, &[span(2, 11)], NormalizeHead::Intro);
        let joined = SourcedText::concat_with([first, second], "\n\n", span(2, 2));

        assert_projected_text(&file, &joined, 0, "é");
        assert_projected_text(&file, &joined, 2, "");
        assert_projected_text(&file, &joined, 3, "");
        assert_projected_text(&file, &joined, 4, "日");
        assert_projected_text(&file, &joined, 7, "本");
        assert_projected_text(&file, &joined, 10, "語");
        assert_projected_text(&file, &joined, 13, "");

        let anchor = joined.source_anchor_at(4).expect("character boundary");
        assert_eq!(file.text_range(anchor.range), Some(""));
        assert_eq!(joined.source_anchor_at(13), Some(span(11, 11)));
    }

    #[test]
    fn newline_provenance_excludes_the_next_comment_prefix() {
        let file = source(
            r#"first
#' second"#,
        );
        let sourced =
            SourcedText::from_lines(&file, &[span(0, 5), span(9, 15)], NormalizeHead::Intro);
        assert_eq!(sourced.as_str(), "first\nsecond");
        assert_eq!(sourced.source_spans(TextRange::new(5, 6)), vec![span(5, 6)]);
        assert_eq!(
            sourced.source_spans(TextRange::new(0, 12)),
            vec![span(0, 6), span(9, 15)]
        );

        let crlf = source("first\r\n#' second");
        let sourced =
            SourcedText::from_lines(&crlf, &[span(0, 5), span(10, 16)], NormalizeHead::Intro);
        assert_eq!(sourced.source_spans(TextRange::new(5, 6)), vec![span(5, 7)]);
    }

    #[test]
    fn continuation_indent_is_removed_with_source_projection() {
        let file = source("first\n  second\n   third");
        let sourced = SourcedText::from_lines(
            &file,
            &[span(
                0,
                u32::try_from(file.text().len()).expect("source fits u32"),
            )],
            NormalizeHead::Intro,
        )
        .trim_continuation_indent();

        assert_eq!(sourced.as_str(), "first\nsecond\n third");
        assert_eq!(
            sourced.source_spans(TextRange::new(6, 12)),
            vec![span(8, 14)]
        );
        assert_eq!(sourced.source_span_at(13), Some(span(17, 18)));
    }

    #[test]
    fn slice_and_point_projection_preserve_escape_runs() {
        let sourced = value("a@@b", TextRange::new(0, 4), NormalizeHead::Intro);
        let sliced = sourced.slice(TextRange::new(1, 3));
        assert_eq!(sliced.as_str(), "@b");
        assert_eq!(sliced.source_spans(TextRange::new(0, 2)), vec![span(1, 4)]);
        assert_eq!(sourced.source_span_at(0), Some(span(0, 1)));
        assert_eq!(sourced.source_span_at(1), Some(span(1, 3)));
        assert_eq!(sourced.source_span_at(3), Some(span(4, 4)));
    }

    #[test]
    fn discontinuous_source_ranges_remain_separate() {
        let file = source("abXXcd");
        let first = SourcedText::from_lines(&file, &[span(0, 2)], NormalizeHead::Intro);
        let second = SourcedText::from_lines(&file, &[span(4, 6)], NormalizeHead::Intro);
        assert_eq!(first.source_spans(TextRange::new(0, 2)), vec![span(0, 2)]);
        assert_eq!(second.source_spans(TextRange::new(0, 2)), vec![span(4, 6)]);
    }

    #[test]
    fn concat_ignores_empty_parts_and_maps_separators_to_an_anchor() {
        let file = source("firstXXsecond");
        let first = SourcedText::from_lines(&file, &[span(0, 5)], NormalizeHead::Intro);
        let empty = SourcedText::from_lines(&file, &[], NormalizeHead::Intro);
        let second = SourcedText::from_lines(&file, &[span(7, 13)], NormalizeHead::Intro);
        let anchor = span(5, 5);

        let joined = SourcedText::concat_with(
            [empty.clone(), first, empty.clone(), second, empty],
            "\n\n",
            anchor,
        );

        assert_eq!(joined.as_str(), "first\n\nsecond");
        assert_eq!(joined.source_spans(TextRange::new(5, 7)), vec![anchor]);
        assert_eq!(joined.source_anchor_at(5), Some(anchor));
        assert_eq!(joined.source_spans(TextRange::new(0, 5)), vec![span(0, 5)]);
        assert_eq!(
            joined.source_spans(TextRange::new(7, 13)),
            vec![span(7, 13)]
        );
    }

    #[test]
    fn concat_of_only_empty_parts_is_empty_without_separators() {
        let file = source("source");
        let empty = SourcedText::from_lines(&file, &[], NormalizeHead::Intro);
        let joined = SourcedText::concat_with([empty.clone(), empty], "\n\n", span(0, 0));

        assert!(joined.is_empty());
        assert!(joined.source_spans(TextRange::new(0, 0)).is_empty());
    }

    #[test]
    #[should_panic(expected = "source range starts inside UTF-8")]
    fn source_spans_rejects_a_non_boundary_start() {
        let sourced = value("é", TextRange::new(0, 2), NormalizeHead::Intro);
        let _ = sourced.source_spans(TextRange::new(1, 2));
    }

    #[test]
    #[should_panic(expected = "source range ends inside UTF-8")]
    fn source_spans_rejects_a_non_boundary_end() {
        let sourced = value("é", TextRange::new(0, 2), NormalizeHead::Intro);
        let _ = sourced.source_spans(TextRange::new(0, 1));
    }
}
