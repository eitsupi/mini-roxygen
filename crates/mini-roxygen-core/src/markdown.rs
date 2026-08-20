//! Converts Markdown text and source spans into Rd fragments.
//!
//! pulldown-cmark is the authority for Markdown semantic interpretation,
//! including escaping, links, tables, lists, and code blocks. arity's Markdown
//! CST is never used as semantic input. The layer preserves source provenance
//! while producing fragments for the Rd builder.
//!
//! The frontend surface shared by consumers is [`markdown_parser`],
//! [`recognize_roxygen_link`], [`ROXYGEN_LINK_MARKER`], and
//! [`protected_markdown_ranges`]. The Roxygen link-envelope rule has exactly
//! one definition in [`recognize_roxygen_link`]; the parser callback and the
//! raw fallback both use it.

use std::ops::Range;

use pulldown_cmark::{BrokenLink, CowStr, Event, LinkType, Options, Parser, Tag};

/// Marker used in destinations synthesized for roxygen R-topic links.
///
/// The destination must not be empty: the future conversion layer will need
/// to distinguish these links from real URLs, and roxygen2's converter turns
/// a marker-less destination into `\url{...}`.
pub(crate) const ROXYGEN_LINK_MARKER: &str = "R:";

/// Recognizes a broken Markdown reference with roxygen2's bracket envelope.
///
/// The original source is needed because the parser's broken-link span is the
/// matched bracket envelope, and the envelope's surrounding characters are
/// part of roxygen2's link-reference rule.
pub(crate) fn recognize_roxygen_link<'a>(
    source: &'a str,
    link: &BrokenLink<'a>,
) -> Option<CowStr<'a>> {
    if !matches!(
        link.link_type,
        LinkType::ShortcutUnknown | LinkType::ReferenceUnknown
    ) {
        return None;
    }

    let Range { start, end } = link.span;
    if start > end || source.get(start..end).is_none() {
        return None;
    }

    if source
        .get(..start)
        .and_then(|prefix| prefix.chars().next_back())
        .is_some_and(|character| character == ']' || character == '\\')
    {
        return None;
    }
    if source
        .get(end..)
        .and_then(|suffix| suffix.chars().next())
        .is_some_and(|character| character == '[' || character == '{')
    {
        return None;
    }

    // The destination comes from the source envelope rather than from
    // `link.reference`, which pulldown-cmark has already normalized by
    // collapsing whitespace runs and line breaks. Comparing the two would
    // reject any label written across a line or with repeated spaces, which
    // roxygen2 links.
    let envelope = source.get(start..end)?;
    let destination = roxygen_link_destination(envelope, link.link_type)?;

    Some(format!("{ROXYGEN_LINK_MARKER}{destination}").into())
}

/// Finds roxygen link envelopes that pulldown-cmark consumed as link
/// definitions before producing events. Reference definitions emit no events,
/// so this raw fallback is needed to keep section splitting aligned with the
/// Markdown frontend. It recognizes bracket structure only; it cannot see
/// inline context such as code spans, and therefore intentionally has a known
/// divergence from roxygen2 on contrived inputs where brackets straddle a code
/// span. Recognition still goes through [`recognize_roxygen_link`], so this
/// fallback does not duplicate the link rule.
fn roxygen_link_ranges(value: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let bytes = value.as_bytes();
    let mut cursor = 0;

    while cursor < bytes.len() {
        if bytes[cursor] != b'[' {
            cursor += 1;
            continue;
        }

        let start = cursor;
        if start > 0 && matches!(bytes[start - 1], b']' | b'\\') {
            cursor += 1;
            continue;
        }

        let mut first_end = start + 1;
        while first_end < bytes.len() && !matches!(bytes[first_end], b'[' | b']') {
            first_end += 1;
        }
        if bytes.get(first_end) == Some(&b'[') {
            // The current candidate is invalid, but the nested opening bracket
            // may begin a later candidate. Continue there without rescanning
            // the text already consumed for this candidate.
            cursor = first_end;
            continue;
        }
        let Some(first_close) = bytes
            .get(first_end)
            .is_some_and(|byte| *byte == b']')
            .then_some(first_end)
        else {
            break;
        };
        let first_end = first_close + 1;

        let (end, link_type, reference_start, reference_end) =
            if bytes.get(first_end) == Some(&b'[') {
                let second_start = first_end + 1;
                let mut second_end = second_start;
                while second_end < bytes.len() && !matches!(bytes[second_end], b'[' | b']') {
                    second_end += 1;
                }
                if bytes.get(second_end) == Some(&b'[') {
                    cursor = second_end;
                    continue;
                }
                let Some(second_close) = bytes
                    .get(second_end)
                    .is_some_and(|byte| *byte == b']')
                    .then_some(second_end)
                else {
                    break;
                };
                (
                    second_close + 1,
                    LinkType::ReferenceUnknown,
                    second_start,
                    second_close,
                )
            } else {
                (first_end, LinkType::ShortcutUnknown, start + 1, first_close)
            };

        let link = BrokenLink {
            span: start..end,
            link_type,
            reference: value[reference_start..reference_end].into(),
        };
        if recognize_roxygen_link(value, &link).is_some() {
            ranges.push(start..end);
        }
        cursor = end;
    }
    ranges
}

fn roxygen_link_destination(envelope: &str, link_type: LinkType) -> Option<&str> {
    let body = envelope.strip_prefix('[')?.strip_suffix(']')?;
    match link_type {
        LinkType::ShortcutUnknown => valid_link_group(body).then_some(body),
        LinkType::ReferenceUnknown => {
            let (text, destination) = body.split_once("][")?;
            (valid_link_group(text) && valid_link_group(destination)).then_some(destination)
        }
        _ => None,
    }
}

fn valid_link_group(group: &str) -> bool {
    !group.is_empty() && !group.bytes().any(|byte| matches!(byte, b'[' | b']'))
}

/// Returns merged source ranges whose contents become Rd groups and therefore
/// cannot contain a legitimate section title/body separator. Code blocks
/// become `\preformatted{}`, tables `\tabular{}{}`, lists `\enumerate{}` or
/// `\itemize{}`, and HTML blocks `\if{html}{\out{}}`. Inline code, inline HTML,
/// emphasis, strong text, links, and images are protected because their
/// contents are interpreted by the Markdown-to-Rd conversion layer. Paragraphs
/// and headings are intentionally excluded because their colons may be
/// semantic separators or require a separate conversion-layer decision.
pub(crate) fn protected_markdown_ranges(value: &str) -> Vec<Range<usize>> {
    let mut ranges = markdown_parser(value)
        .filter_map(|(event, range)| match event {
            Event::Code(_) | Event::InlineHtml(_) => Some(range),
            Event::Start(Tag::Emphasis | Tag::Strong | Tag::Link { .. } | Tag::Image { .. }) => {
                Some(range)
            }
            Event::Start(Tag::CodeBlock(_) | Tag::Table(_) | Tag::List(_) | Tag::HtmlBlock) => {
                Some(range)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    ranges.extend(roxygen_link_ranges(value));
    ranges.sort_unstable_by_key(|range| (range.start, range.end));

    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && range.start <= last.end
        {
            last.end = last.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}

/// Constructs the single Markdown event stream shared by the semantic tag
/// layer and the future Markdown-to-Rd conversion layer, so both consumers see
/// the same links.
pub(crate) fn markdown_parser<'a>(
    value: &'a str,
) -> impl Iterator<Item = (Event<'a>, Range<usize>)> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    Parser::new_with_broken_link_callback(
        value,
        options,
        Some(|broken: BrokenLink<'a>| {
            // pulldown-cmark passes the original link type to this callback
            // and converts it to its unknown counterpart after acceptance.
            let link_type = match broken.link_type {
                LinkType::Shortcut => LinkType::ShortcutUnknown,
                LinkType::Reference => LinkType::ReferenceUnknown,
                LinkType::Collapsed => LinkType::CollapsedUnknown,
                _ => return None,
            };
            let normalized = BrokenLink {
                link_type,
                ..broken
            };
            recognize_roxygen_link(value, &normalized)
                .map(|destination| (destination, CowStr::Borrowed("")))
        }),
    )
    .into_offset_iter()
}

#[cfg(test)]
mod tests {
    use super::{ROXYGEN_LINK_MARKER, recognize_roxygen_link, roxygen_link_ranges};
    use pulldown_cmark::{BrokenLink, LinkType};

    fn broken<'a>(
        source: &'a str,
        link_type: LinkType,
        span: std::ops::Range<usize>,
    ) -> BrokenLink<'a> {
        BrokenLink {
            span,
            link_type,
            reference: source.into(),
        }
    }

    #[test]
    fn recognizes_only_supported_roxygen_link_forms() {
        let link = broken("target", LinkType::ShortcutUnknown, 0..8);
        let destination = recognize_roxygen_link("[target]", &link).expect("R link");
        assert_eq!(destination.as_ref(), "R:target");
        assert!(destination.as_ref().starts_with(ROXYGEN_LINK_MARKER));

        let reference = broken("target", LinkType::ReferenceUnknown, 0..14);
        assert!(recognize_roxygen_link("[text][target]", &reference).is_some());

        let collapsed = broken("target", LinkType::CollapsedUnknown, 0..10);
        assert!(recognize_roxygen_link("[target][]", &collapsed).is_none());
    }

    #[test]
    fn a_normalized_reference_still_recognizes_its_source_envelope() {
        // pulldown-cmark collapses whitespace runs and line breaks in the
        // label it hands the callback, so the destination has to come from
        // the source envelope. roxygen2 links both of these.
        let spaced = broken("two spaces", LinkType::ShortcutUnknown, 0..13);
        let destination = recognize_roxygen_link("[two  spaces]", &spaced).expect("R link");
        assert_eq!(destination.as_ref(), "R:two  spaces");

        let wrapped = broken("wrap ped", LinkType::ShortcutUnknown, 0..10);
        let destination = recognize_roxygen_link("[wrap\nped]", &wrapped).expect("R link");
        assert_eq!(destination.as_ref(), "R:wrap\nped");
    }

    #[test]
    fn a_link_event_survives_a_normalized_label() {
        use pulldown_cmark::{Event, Tag};

        for text in [
            "Title [one space] x",
            "Title [two  spaces] x",
            "Title [wrap\nped] x",
        ] {
            assert!(
                super::markdown_parser(text)
                    .any(|(event, _)| matches!(event, Event::Start(Tag::Link { .. }))),
                "expected a link event for {text:?}"
            );
        }
    }

    #[test]
    fn rejects_invalid_roxygen_link_envelopes() {
        assert!(
            recognize_roxygen_link(
                "][target]",
                &broken("target", LinkType::ShortcutUnknown, 1..9)
            )
            .is_none()
        );
        assert!(
            recognize_roxygen_link(
                r"\[target]",
                &broken("target", LinkType::ShortcutUnknown, 1..9)
            )
            .is_none()
        );
        assert!(
            recognize_roxygen_link(
                "[target][",
                &broken("target", LinkType::ShortcutUnknown, 0..8)
            )
            .is_none()
        );
        assert!(
            recognize_roxygen_link(
                "[target]{",
                &broken("target", LinkType::ShortcutUnknown, 0..8)
            )
            .is_none()
        );
        assert!(
            recognize_roxygen_link("[]", &broken("", LinkType::ShortcutUnknown, 0..2)).is_none()
        );
        assert!(
            recognize_roxygen_link(
                "[ta[rget]",
                &broken("ta[rget", LinkType::ShortcutUnknown, 0..9)
            )
            .is_none()
        );
        assert!(
            recognize_roxygen_link(
                "[ta]rget]",
                &broken("ta]rget", LinkType::ShortcutUnknown, 0..10)
            )
            .is_none()
        );
        assert!(
            recognize_roxygen_link(
                "[][target]",
                &broken("target", LinkType::ReferenceUnknown, 0..10)
            )
            .is_none()
        );
        assert!(
            recognize_roxygen_link(
                "[ta[rget][target]",
                &broken("target", LinkType::ReferenceUnknown, 0..17)
            )
            .is_none()
        );
    }

    #[test]
    fn scans_many_unmatched_opening_brackets_in_one_pass() {
        let value = "[".repeat(32_768);
        assert!(roxygen_link_ranges(&value).is_empty());
    }

    #[test]
    fn accepts_known_code_span_fallback_divergence() {
        assert_eq!(roxygen_link_ranges("[a`b` :c]"), vec![0..9]);
    }
}
