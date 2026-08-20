//! Physical source-line and offset helpers.

use crate::source::{FileId, SourceFile, Span, TextRange};

use super::DocLine;

pub(super) fn physical_lines(
    source_file: &SourceFile,
    file_id: FileId,
    range_start: u32,
    range_end: u32,
) -> Vec<DocLine> {
    let start = line_start(source_file.text(), to_usize(range_start));
    let end = to_usize(range_end);
    let mut lines = Vec::new();
    let mut current = start;
    let mut first = true;

    while first || current < end {
        let line_end = source_line_end(source_file.text(), current, end);
        let marker = if first {
            roxygen_marker_start(source_file.text(), to_usize(range_start), line_end)
                .unwrap_or(to_usize(range_start))
        } else {
            prefix_marker_start(source_file.text(), current, line_end).unwrap_or(current)
        };
        let content_start = prefix_content_start(source_file.text(), marker, line_end);
        let line_end = line_end.min(end);
        let content_start = content_start.min(line_end);
        lines.push(DocLine {
            span: Span::new(file_id, TextRange::new(to_u32(current), to_u32(line_end))),
            content_span: Span::new(
                file_id,
                TextRange::new(to_u32(content_start), to_u32(line_end)),
            ),
        });
        first = false;
        if line_end >= end {
            break;
        }
        current =
            line_end + usize::from(source_file.text().as_bytes().get(line_end) == Some(&b'\r')) + 1;
    }
    lines
}

/// Removes the roxygen line prefix according to roxygen2's physical tokenizer:
/// all leading indentation, all `#` characters, one apostrophe, then at most
/// one following ASCII whitespace character. A CST block can only contain
/// matching roxygen lines, so a malformed line is conservatively represented as
/// empty content rather than generating a second diagnostic for parser-owned
/// recovery text.
fn prefix_content_start(text: &str, start: usize, end: usize) -> usize {
    let bytes = text.as_bytes();
    let mut index = start;
    while index < end && matches!(bytes[index], b' ' | b'\t') {
        index += 1;
    }
    let hashes = index;
    while index < end && bytes[index] == b'#' {
        index += 1;
    }
    if index == hashes || index >= end || bytes[index] != b'\'' {
        return end;
    }
    index += 1;
    if index < end && matches!(bytes[index], b' ' | b'\t') {
        index += 1;
    }
    index
}

pub(super) fn prefix_marker_start(text: &str, start: usize, end: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut index = start;
    while index < end && matches!(bytes[index], b' ' | b'\t') {
        index += 1;
    }
    (index < end && bytes[index] == b'#').then_some(index)
}

pub(super) fn roxygen_marker_start(text: &str, start: usize, end: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let marker = (start..end.saturating_sub(1))
        .find(|&index| bytes[index] == b'#' && bytes[index + 1] == b'\'')?;
    Some(
        (start..=marker)
            .rev()
            .find(|&index| index == start || bytes[index - 1] != b'#')
            .unwrap_or(marker),
    )
}

pub(super) fn next_physical_line_start(text: &str, start: u32) -> Option<u32> {
    text.as_bytes()
        .get(to_usize(start)..)?
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|offset| to_u32(to_usize(start) + offset + 1))
}

pub(super) fn marker_start(text: &str, start: u32, end: u32) -> u32 {
    let bytes = text.as_bytes();
    let mut index = to_usize(start);
    let limit = to_usize(end);
    while index < limit && matches!(bytes[index], b' ' | b'\t') {
        index += 1;
    }
    to_u32(index)
}

fn source_line_end(text: &str, start: usize, limit: usize) -> usize {
    let bytes = text.as_bytes();
    let search_end = limit.min(bytes.len());
    let newline = bytes[start..search_end]
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(search_end, |offset| start + offset);
    if newline > start && bytes[newline - 1] == b'\r' {
        newline - 1
    } else {
        newline
    }
}

pub(super) fn line_start(text: &str, offset: usize) -> usize {
    text.as_bytes()[..offset.min(text.len())]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |newline| newline + 1)
}

pub(super) fn diagnostic_range(source_len: usize, start: usize, end: usize) -> TextRange {
    // ParseDiagnostic ranges are byte offsets. Normalize a reversed recovery
    // range before calling TextRange::new, whose invariant intentionally
    // asserts start <= end. Clamping also prevents a malformed parser range
    // from creating an out-of-bounds source span; extraction still continues.
    let start = start.min(source_len);
    let end = end.min(source_len);
    let (start, end) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    TextRange::new(to_u32(start), to_u32(end))
}

pub(super) fn to_usize(value: impl TryInto<u32> + Copy) -> usize {
    usize::try_from(to_u32(value)).expect("source offsets must fit in usize")
}

pub(super) fn to_u32(value: impl TryInto<u32> + Copy) -> u32 {
    value
        .try_into()
        .ok()
        .expect("source offsets must fit in TextRange")
}
