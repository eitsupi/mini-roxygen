//! Splits a tag value into whitespace-separated words that keep their spans.
//!
//! Word-list tags such as `@aliases` and `@keywords` share this scanning with
//! the `@inherit*` grammars, which read their leading words positionally.

use super::diagnostics::value_span_for_range;
use super::text::SourcedText;
use crate::source::{FileId, Span, Spanned, TextRange};

pub(super) fn parse_words<T>(value: &SourcedText, map: impl Fn(&str) -> T) -> Vec<Spanned<T>> {
    let fallback = value
        .source_anchor_at(0)
        .unwrap_or(Span::new(FileId::new(0), TextRange::new(0, 0)));
    word_ranges(value)
        .into_iter()
        .map(|(start, end)| {
            Spanned::new(
                map(&value.as_str()[start..end]),
                value_span_for_range(value, start, end, fallback),
            )
        })
        .collect()
}

pub(super) fn word_ranges(value: &SourcedText) -> Vec<(usize, usize)> {
    let mut words = Vec::new();
    let mut start = None;
    for (index, character) in value.as_str().char_indices() {
        if character.is_whitespace() {
            if let Some(start) = start.take() {
                words.push((start, index));
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if let Some(start) = start {
        words.push((start, value.as_str().len()));
    }
    words
}
