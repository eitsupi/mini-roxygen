//! Restricted raw Rd support for Markdown prose.
//!
//! Raw Rd is spliced into the event stream produced for the original source.
//! The source is parsed once: the same offset-bearing events describe code
//! opacity, Markdown construct boundaries, and the places where a raw node
//! may be inserted.

use std::ops::Range;

use pulldown_cmark::{Event, LinkType, Tag, TagEnd};
use rd_ast::{RdNode, RdTag};

use crate::diagnostic::{Diagnostic, DiagnosticCode, Label};

use super::frame::{self, NodeWithOrigin};
use super::separator;

#[derive(Debug)]
pub(super) struct RawRdPreparation {
    macros: Vec<RawRdMacro>,
    pub(super) unsupported: Vec<Range<usize>>,
    next_macro: usize,
}

#[derive(Debug, Clone)]
struct RawRdMacro {
    source: Range<usize>,
    kind: RawRdKind,
}

#[derive(Debug, Clone)]
enum RawRdKind {
    /// A zero-argument prose macro. These tags are structural in Rd,
    /// rather than literal text, and preserve their source spelling for the
    /// writer (`\R`, `\dots`, `\ldots`, `\cr`, or `\sspace`).
    ZeroArgumentProseMacro { tag: RdTag },
    Equation {
        tag: RdTag,
        arguments: Vec<RawRdArgument>,
    },
}

#[derive(Debug, Clone)]
struct RawRdArgument {
    outer: Range<usize>,
    body: Range<usize>,
}

#[derive(Debug, Clone, Copy)]
enum ConstructKind {
    Block,
    Inline,
    Link { autolink: bool },
    Image,
    Html,
}

#[derive(Debug)]
struct Construct {
    start: usize,
    end: usize,
    kind: ConstructKind,
}

#[derive(Debug)]
struct TextEvent {
    range: Range<usize>,
    text: String,
}

#[derive(Debug)]
struct EventFacts {
    constructs: Vec<Construct>,
    text_events: Vec<TextEvent>,
    html_events: Vec<Range<usize>>,
    opaque_ranges: Vec<Range<usize>>,
    coverage_ranges: Vec<Range<usize>>,
}

impl RawRdPreparation {
    pub(super) fn new(source: &str, events: &[(Event<'_>, Range<usize>)]) -> Self {
        let facts = EventFacts::new(events);
        let mut scanned = Vec::new();
        scan_source(source, &facts.opaque_ranges, &mut scanned);
        let mut sweep = CandidateSweep::new(&facts);

        let mut macros = Vec::new();
        let mut unsupported = Vec::new();
        for candidate in scanned {
            match candidate.kind {
                Some(kind) => {
                    let candidate = RawRdMacro {
                        source: candidate.source,
                        kind,
                    };
                    if sweep.accepts_candidate(&candidate.source, source)
                        && is_representable(&candidate, source)
                    {
                        macros.push(candidate);
                    } else {
                        unsupported.push(candidate.source);
                    }
                }
                None => {
                    unsupported.push(candidate.source);
                }
            }
        }

        Self {
            macros,
            unsupported,
            next_macro: 0,
        }
    }

    pub(super) fn disabled() -> Self {
        Self {
            macros: Vec::new(),
            unsupported: Vec::new(),
            next_macro: 0,
        }
    }

    fn advance_before(&mut self, offset: usize) {
        while self
            .macros
            .get(self.next_macro)
            .is_some_and(|candidate| candidate.source.end <= offset)
        {
            self.next_macro += 1;
        }
    }

    fn current(&mut self, offset: usize) -> Option<&RawRdMacro> {
        self.advance_before(offset);
        self.macros.get(self.next_macro)
    }

    pub(super) fn suppresses(&mut self, range: &Range<usize>) -> bool {
        self.current(range.start).is_some_and(|candidate| {
            range.start >= candidate.source.start
                && range.end <= candidate.source.end
                && (range.start > candidate.source.start || range.end < candidate.source.end)
        })
    }
}

impl EventFacts {
    fn new(events: &[(Event<'_>, Range<usize>)]) -> Self {
        let mut constructs = Vec::new();
        let mut open = Vec::new();
        let mut text_events = Vec::new();
        let mut line_breaks = Vec::new();
        let mut html_events = Vec::new();
        let mut opaque_ranges = Vec::new();

        for (event, range) in events {
            match event {
                Event::Start(tag) => {
                    open.push((range.start, classify_tag(tag)));
                }
                Event::End(TagEnd::CodeBlock) => {
                    if let Some((start, kind)) = open.pop() {
                        opaque_ranges.push(start..range.end);
                        constructs.push(Construct {
                            start,
                            end: range.end,
                            kind,
                        });
                    }
                }
                Event::End(_) => {
                    if let Some((start, kind)) = open.pop() {
                        constructs.push(Construct {
                            start,
                            end: range.end,
                            kind,
                        });
                    }
                }
                Event::Text(text) => text_events.push(TextEvent {
                    range: range.clone(),
                    text: text.to_string(),
                }),
                Event::SoftBreak | Event::HardBreak => line_breaks.push(range.clone()),
                Event::Code(_) => opaque_ranges.push(range.clone()),
                Event::Html(_) | Event::InlineHtml(_) => html_events.push(range.clone()),
                _ => {}
            }
        }

        constructs.sort_unstable_by_key(|construct| (construct.start, construct.end));
        text_events.sort_unstable_by_key(|event| (event.range.start, event.range.end));
        line_breaks.sort_unstable_by_key(|range| (range.start, range.end));
        html_events.sort_unstable_by_key(|range| (range.start, range.end));
        opaque_ranges.sort_unstable_by_key(|range| (range.start, range.end));
        let mut coverage_ranges = text_events
            .iter()
            .map(|event| event.range.clone())
            .chain(line_breaks.iter().cloned())
            .chain(html_events.iter().cloned())
            .collect::<Vec<_>>();
        coverage_ranges.sort_unstable_by_key(|range| (range.start, range.end));
        Self {
            constructs,
            text_events,
            html_events,
            opaque_ranges,
            coverage_ranges,
        }
    }
}

fn classify_tag(tag: &Tag<'_>) -> ConstructKind {
    match tag {
        Tag::Link { link_type, .. } => ConstructKind::Link {
            autolink: *link_type == LinkType::Autolink,
        },
        Tag::Image { .. } => ConstructKind::Image,
        Tag::HtmlBlock => ConstructKind::Html,
        Tag::Paragraph
        | Tag::Heading { .. }
        | Tag::BlockQuote(_)
        | Tag::CodeBlock(_)
        | Tag::List(_)
        | Tag::Item
        | Tag::FootnoteDefinition(_)
        | Tag::DefinitionList
        | Tag::DefinitionListTitle
        | Tag::DefinitionListDefinition
        | Tag::Table(_)
        | Tag::TableHead
        | Tag::TableRow
        | Tag::TableCell
        | Tag::MetadataBlock(_) => ConstructKind::Block,
        Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Superscript | Tag::Subscript => {
            ConstructKind::Inline
        }
    }
}

struct CandidateSweep<'a> {
    facts: &'a EventFacts,
    construct_cursor: usize,
    active_constructs: Vec<usize>,
    html_cursor: usize,
    active_html_end: usize,
    opaque_cursor: usize,
    active_opaque_end: usize,
    text_cursor: usize,
    coverage_cursor: usize,
    active_coverage_end: usize,
}

impl<'a> CandidateSweep<'a> {
    fn new(facts: &'a EventFacts) -> Self {
        Self {
            facts,
            construct_cursor: 0,
            active_constructs: Vec::new(),
            html_cursor: 0,
            active_html_end: 0,
            opaque_cursor: 0,
            active_opaque_end: 0,
            text_cursor: 0,
            coverage_cursor: 0,
            active_coverage_end: 0,
        }
    }

    fn accepts_candidate(&mut self, candidate: &Range<usize>, source: &str) -> bool {
        if candidate.start >= candidate.end {
            return false;
        }
        if !self.html_boundaries_allow(candidate) || self.opaque_overlap(candidate) {
            return false;
        }

        let mut contained_constructs = Vec::new();
        if !self.constructs_allow(candidate, &mut contained_constructs) {
            return false;
        }
        if !self.boundaries_are_aligned(candidate, source) {
            return false;
        }
        self.coverage_complete(candidate, &contained_constructs, source)
    }

    fn html_boundaries_allow(&mut self, candidate: &Range<usize>) -> bool {
        if self.active_html_end <= candidate.start {
            self.active_html_end = 0;
        }
        let mut allowed = true;
        while self
            .facts
            .html_events
            .get(self.html_cursor)
            .is_some_and(|html| html.start < candidate.end)
        {
            let html = &self.facts.html_events[self.html_cursor];
            if html.start < candidate.end
                && candidate.start < html.end
                && !(candidate.start <= html.start && html.end <= candidate.end)
            {
                allowed = false;
            }
            if html.end > candidate.end {
                self.active_html_end = self.active_html_end.max(html.end);
            }
            self.html_cursor += 1;
        }
        if self.active_html_end > candidate.start {
            allowed = false;
        }
        allowed
    }

    fn opaque_overlap(&mut self, candidate: &Range<usize>) -> bool {
        let mut overlaps =
            self.active_opaque_end > candidate.start && self.active_opaque_end < candidate.end;
        while self
            .facts
            .opaque_ranges
            .get(self.opaque_cursor)
            .is_some_and(|range| range.start < candidate.end)
        {
            let opaque = &self.facts.opaque_ranges[self.opaque_cursor];
            if opaque.start < candidate.end
                && candidate.start < opaque.end
                && !(opaque.start <= candidate.start && candidate.end <= opaque.end)
            {
                overlaps = true;
            }
            if opaque.end > candidate.end {
                self.active_opaque_end = self.active_opaque_end.max(opaque.end);
            }
            self.opaque_cursor += 1;
        }
        overlaps
    }

    fn constructs_allow(
        &mut self,
        candidate: &Range<usize>,
        contained_constructs: &mut Vec<Range<usize>>,
    ) -> bool {
        let facts = self.facts;
        self.active_constructs
            .retain(|index| facts.constructs[*index].end > candidate.start);

        let mut allowed = true;
        for index in &self.active_constructs {
            let construct = &self.facts.constructs[*index];
            if construct.end < candidate.end {
                allowed = false;
            }
            if construct.end > candidate.end
                && matches!(
                    construct.kind,
                    ConstructKind::Link { autolink: true } | ConstructKind::Html
                )
            {
                allowed = false;
            }
        }

        while self
            .facts
            .constructs
            .get(self.construct_cursor)
            .is_some_and(|construct| construct.start < candidate.end)
        {
            let index = self.construct_cursor;
            let construct = &self.facts.constructs[index];
            self.construct_cursor += 1;
            let range = construct.start..construct.end;

            if range.start < candidate.start {
                if range.end > candidate.start {
                    self.active_constructs.push(index);
                    if range.end < candidate.end {
                        allowed = false;
                    }
                    if range.end > candidate.end
                        && matches!(
                            construct.kind,
                            ConstructKind::Link { autolink: true } | ConstructKind::Html
                        )
                    {
                        allowed = false;
                    }
                }
                continue;
            }

            if range.end > candidate.end {
                if !(range.start == candidate.start
                    && matches!(construct.kind, ConstructKind::Block))
                {
                    allowed = false;
                }
                self.active_constructs.push(index);
            } else {
                if matches!(construct.kind, ConstructKind::Block)
                    && (range.start > candidate.start || range.end < candidate.end)
                {
                    allowed = false;
                }
                contained_constructs.push(range);
            }
        }
        allowed
    }

    fn boundaries_are_aligned(&mut self, candidate: &Range<usize>, source: &str) -> bool {
        while self
            .facts
            .text_events
            .get(self.text_cursor)
            .is_some_and(|event| event.range.end <= candidate.start)
        {
            self.text_cursor += 1;
        }
        let mut cursor = self.text_cursor;
        let mut aligned = true;
        for boundary in [candidate.start, candidate.end] {
            while self
                .facts
                .text_events
                .get(cursor)
                .is_some_and(|event| event.range.end <= boundary)
            {
                cursor += 1;
            }
            if self.facts.text_events.get(cursor).is_some_and(|event| {
                event.range.start < boundary
                    && boundary < event.range.end
                    && source.get(event.range.clone()) != Some(event.text.as_str())
            }) {
                aligned = false;
            }
        }
        self.text_cursor = cursor;
        aligned
    }

    fn coverage_complete(
        &mut self,
        candidate: &Range<usize>,
        contained_constructs: &[Range<usize>],
        source: &str,
    ) -> bool {
        let mut cursor = candidate.start;
        if self.active_coverage_end <= candidate.start {
            self.active_coverage_end = 0;
        } else {
            cursor = self.active_coverage_end;
        }

        while self
            .facts
            .coverage_ranges
            .get(self.coverage_cursor)
            .is_some_and(|range| range.start < candidate.start)
        {
            let range = &self.facts.coverage_ranges[self.coverage_cursor];
            cursor = cursor.max(range.end);
            if range.end > candidate.end {
                self.active_coverage_end = self.active_coverage_end.max(range.end);
            }
            self.coverage_cursor += 1;
        }

        let mut construct_cursor = 0;
        let mut gap = false;
        loop {
            let static_range = self
                .facts
                .coverage_ranges
                .get(self.coverage_cursor)
                .filter(|range| range.start < candidate.end);
            let construct_range = contained_constructs
                .get(construct_cursor)
                .filter(|range| range.start < candidate.end);
            let Some((range, is_static)) = (match (static_range, construct_range) {
                (None, None) => None,
                (Some(range), None) => Some((range, true)),
                (None, Some(range)) => Some((range, false)),
                (Some(static_range), Some(construct_range)) => {
                    if static_range.start <= construct_range.start {
                        Some((static_range, true))
                    } else {
                        Some((construct_range, false))
                    }
                }
            }) else {
                break;
            };
            if range.start > cursor {
                let missing = source.get(cursor..range.start);
                if missing != Some("\\") {
                    gap = true;
                }
            }
            cursor = cursor.max(range.end);
            if is_static {
                if range.end > candidate.end {
                    self.active_coverage_end = self.active_coverage_end.max(range.end);
                }
                self.coverage_cursor += 1;
            } else {
                construct_cursor += 1;
            }
        }
        if self.active_coverage_end <= candidate.end {
            self.active_coverage_end = 0;
        }
        !gap && cursor >= candidate.end
    }
}

pub(super) fn diagnose(converter: &mut super::Converter<'_>, unsupported: &[Range<usize>]) {
    for range in unsupported {
        let spans = converter.spans(range.start, range.end);
        let Some(primary) = spans.first().copied() else {
            continue;
        };
        let secondary = spans[1..]
            .iter()
            .copied()
            .map(|span| Label::new(span, "part of this raw Rd macro"));
        converter.diagnostics.push(
            Diagnostic::new(
                DiagnosticCode::UnsupportedRawRdMacro.default_severity(),
                DiagnosticCode::UnsupportedRawRdMacro,
                "unsupported raw Rd macro",
                Label::new(primary, "unsupported raw Rd macro"),
            )
            .with_secondaries(secondary),
        );
    }
}

/// Appends a Markdown text event while splicing accepted raw Rd candidates.
/// Events wholly inside a candidate are suppressed by suppresses; the first
/// and last text events are split at source-aligned candidate boundaries.
pub(super) fn append_text(converter: &mut super::Converter<'_>, text: &str, range: Range<usize>) {
    let mut cursor = range.start;
    loop {
        converter.raw_rd.advance_before(cursor);
        let Some(candidate) = converter.raw_rd.current(cursor).cloned() else {
            break;
        };
        let candidate_start = candidate.source.start;
        let candidate_end = candidate.source.end;
        if candidate_start >= range.end {
            break;
        }

        if candidate_start < range.start {
            if candidate_end >= range.end {
                return;
            }
            cursor = candidate_end;
            converter.raw_rd.next_macro += 1;
            continue;
        }

        let before_end = candidate_start.min(range.end);
        if cursor < before_end {
            append_literal_slice(converter, text, range.clone(), cursor..before_end);
        }

        let node = make_node(converter, &candidate);
        append_node(converter, node, candidate_start);
        if candidate_end > range.end {
            return;
        }
        cursor = candidate_end;
        converter.raw_rd.next_macro += 1;
    }

    if cursor < range.end {
        append_literal_slice(converter, text, range.clone(), cursor..range.end);
    }
}

fn append_literal_slice(
    converter: &mut super::Converter<'_>,
    text: &str,
    event_range: Range<usize>,
    source_range: Range<usize>,
) {
    let start = source_range.start - event_range.start;
    let end = source_range.end - event_range.start;
    let Some(literal) = text.get(start..end) else {
        converter.append_text(text, event_range);
        return;
    };
    converter.append_text(literal, source_range);
}

fn append_node(converter: &mut super::Converter<'_>, node: NodeWithOrigin, offset: usize) {
    let anchor = converter.anchor(offset);
    let frame = converter.frames.last_mut().expect("the root frame exists");
    separator::materialize_separator(frame, anchor);
    frame::append_node(frame, node);
}

fn make_node(converter: &super::Converter<'_>, raw: &RawRdMacro) -> NodeWithOrigin {
    match &raw.kind {
        RawRdKind::ZeroArgumentProseMacro { tag } => NodeWithOrigin {
            node: RdNode::tagged(tag.clone(), None, Vec::new()),
            children: Vec::new(),
            spans: converter.spans(raw.source.start, raw.source.end),
        },
        RawRdKind::Equation { tag, arguments } => {
            let mut groups = Vec::with_capacity(arguments.len());
            for argument in arguments {
                let spans = converter.spans(argument.body.start, argument.body.end);
                let leaves = super::leaf::physical_line_chunks(
                    &converter.value.as_str()[argument.body.clone()],
                )
                .map(|line| NodeWithOrigin {
                    node: RdNode::Verb(line.to_owned()),
                    children: Vec::new(),
                    spans: spans.clone(),
                })
                .collect::<Vec<_>>();
                groups.push(NodeWithOrigin {
                    node: RdNode::group(leaves.iter().map(|leaf| leaf.node.clone()).collect()),
                    children: leaves,
                    spans: converter.spans(argument.outer.start, argument.outer.end),
                });
            }
            let children = groups.iter().map(|group| group.node.clone()).collect();
            NodeWithOrigin {
                node: RdNode::tagged(tag.clone(), None, children),
                children: groups,
                spans: converter.spans(raw.source.start, raw.source.end),
            }
        }
    }
}

/// Reports whether this macro survives the split into physical-line leaves.
///
/// An equation argument is written one leaf per line, and the writer requires
/// each equation leaf to balance its own braces. R accepts a brace pair that
/// spans lines, so this is a limit of the AST rather than of the input; saying
/// so through the unsupported path keeps the rest of the topic buildable
/// instead of failing the whole document at serialization.
fn is_representable(candidate: &RawRdMacro, source: &str) -> bool {
    match &candidate.kind {
        RawRdKind::ZeroArgumentProseMacro { .. } => true,
        RawRdKind::Equation { arguments, .. } => arguments.iter().all(|argument| {
            super::leaf::physical_line_chunks(&source[argument.body.clone()]).all(balanced_braces)
        }),
    }
}

/// Mirrors the writer's own brace check for one equation leaf.
fn balanced_braces(value: &str) -> bool {
    let mut depth = 0usize;
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' => escaped = true,
            '{' => depth += 1,
            '}' if depth == 0 => return false,
            '}' => depth -= 1,
            _ => {}
        }
    }
    depth == 0
}

#[derive(Debug)]
struct ScannedMacro {
    source: Range<usize>,
    kind: Option<RawRdKind>,
}

/// Collects raw Rd macro candidates.
///
/// `opaque` carries the code spans and code blocks, sorted. An introducer in
/// one of those ranges is literal, so scanning resumes at the range's end
/// before any macro name or argument is inspected.
fn scan_source(source: &str, opaque: &[Range<usize>], candidates: &mut Vec<ScannedMacro>) {
    let bytes = source.as_bytes();
    let mut cursor = 0;
    let mut backslash_run = 0;
    let mut opaque_cursor = 0;
    while cursor < bytes.len() {
        while opaque
            .get(opaque_cursor)
            .is_some_and(|range| range.end <= cursor)
        {
            opaque_cursor += 1;
        }
        if let Some(end) = opaque
            .get(opaque_cursor)
            .filter(|range| range.start <= cursor && cursor < range.end)
            .map(|range| range.end)
        {
            // A backslash inside code is literal, regardless of the macro
            // name or whether its arguments would be balanced. Do not scan
            // any part of the candidate, and resume after the opaque range.
            cursor = end;
            opaque_cursor += 1;
            backslash_run = 0;
            continue;
        }

        if bytes[cursor] == b'\\' {
            if backslash_run % 2 == 1 {
                backslash_run += 1;
                cursor += 1;
                continue;
            }
        } else {
            let is_escaped = backslash_run % 2 == 1;
            backslash_run = 0;
            if is_escaped || bytes[cursor] != b'\\' {
                cursor += 1;
                continue;
            }
        }

        let name_start = cursor + 1;
        if !bytes
            .get(name_start)
            .is_some_and(|byte| byte.is_ascii_alphabetic())
        {
            backslash_run += 1;
            cursor += 1;
            continue;
        }
        let mut name_end = name_start + 1;
        while name_end < bytes.len() && bytes[name_end].is_ascii_alphanumeric() {
            name_end += 1;
        }

        let name = &source[name_start..name_end];
        let next = bytes.get(name_end).copied();
        if let Some(tag) = supported_zero_argument_prose_macro(name)
            .filter(|_| !matches!(next, Some(b'{') | Some(b'[')))
        {
            candidates.push(ScannedMacro {
                source: cursor..name_end,
                kind: Some(RawRdKind::ZeroArgumentProseMacro { tag }),
            });
            cursor = name_end;
            backslash_run = 0;
            continue;
        }
        if is_rejected_zero_argument_macro(name) && !matches!(next, Some(b'{') | Some(b'[')) {
            // `\tab` is a table separator and is not safe as a context-free
            // prose node. The other zero-argument candidates are structural
            // tags with writer-supported source forms.
            candidates.push(ScannedMacro {
                source: cursor..name_end,
                kind: None,
            });
            cursor = name_end;
            backslash_run = 0;
            continue;
        }

        let group_start = match next {
            Some(b'{') => name_end,
            Some(b'[') => {
                let option_end = scan_option(source, name_end);
                match option_end.and_then(|end| {
                    bytes
                        .get(end)
                        .is_some_and(|byte| *byte == b'{')
                        .then_some(end)
                }) {
                    Some(end) => end,
                    None => {
                        backslash_run += 1;
                        cursor += 1;
                        continue;
                    }
                }
            }
            _ => {
                backslash_run += 1;
                cursor += 1;
                continue;
            }
        };
        if matches!(name, "eqn" | "deqn") && bytes.get(name_end) == Some(&b'{') {
            match scan_equation(source, name, group_start) {
                Some((arguments, end)) => {
                    candidates.push(ScannedMacro {
                        source: cursor..end,
                        kind: Some(RawRdKind::Equation {
                            tag: if name == "eqn" {
                                RdTag::Eqn
                            } else {
                                RdTag::Deqn
                            },
                            arguments,
                        }),
                    });
                    cursor = end;
                    backslash_run = 0;
                }
                None => {
                    candidates.push(ScannedMacro {
                        source: cursor..bytes.len(),
                        kind: None,
                    });
                    cursor = bytes.len();
                    backslash_run = 0;
                }
            }
            continue;
        }

        let Some(end) = scan_group(source, group_start) else {
            backslash_run += 1;
            cursor += 1;
            continue;
        };
        candidates.push(ScannedMacro {
            source: cursor..end,
            kind: None,
        });
        cursor = end;
        backslash_run = 0;
    }
}

fn supported_zero_argument_prose_macro(name: &str) -> Option<RdTag> {
    match name {
        "R" => Some(RdTag::R),
        "dots" => Some(RdTag::Dots),
        "ldots" => Some(RdTag::LDots),
        "cr" => Some(RdTag::Cr),
        "sspace" => Some(RdTag::Sspace),
        _ => None,
    }
}

fn is_rejected_zero_argument_macro(name: &str) -> bool {
    name == "tab"
}

fn scan_option(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut backslash_run = 0;
    for (index, byte) in bytes.iter().enumerate().skip(open + 1) {
        if *byte == b'\\' {
            backslash_run += 1;
            continue;
        }
        let is_escaped = backslash_run % 2 == 1;
        backslash_run = 0;
        if *byte == b']' && !is_escaped {
            return Some(index + 1);
        }
    }
    None
}

fn scan_equation(
    source: &str,
    _name: &str,
    first_open: usize,
) -> Option<(Vec<RawRdArgument>, usize)> {
    let first_close = scan_group(source, first_open)?;
    let mut arguments = vec![RawRdArgument {
        outer: first_open..first_close,
        body: first_open + 1..first_close - 1,
    }];
    let end = if source.as_bytes().get(first_close) == Some(&b'{') {
        let second_close = scan_group(source, first_close)?;
        arguments.push(RawRdArgument {
            outer: first_close..second_close,
            body: first_close + 1..second_close - 1,
        });
        second_close
    } else {
        first_close
    };
    Some((arguments, end))
}

/// Returns the exclusive byte offset after a balanced brace group.
fn scan_group(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    if bytes.get(open) != Some(&b'{') {
        return None;
    }
    let mut depth = 0usize;
    let mut backslash_run = 0;
    for (index, byte) in bytes.iter().enumerate().skip(open) {
        if *byte == b'\\' {
            backslash_run += 1;
            continue;
        }
        let is_escaped = backslash_run % 2 == 1;
        backslash_run = 0;
        match *byte {
            b'{' if !is_escaped => depth += 1,
            b'}' if !is_escaped => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use rd_ast::{RdNode, RdTag};

    use super::super::test_support::{assert_serialized_body, context, serialize, value};
    use super::super::{
        FragmentPath, FragmentPathSegment, convert_markdown, convert_markdown_without_raw_rd,
    };
    use super::{RawRdKind, scan_source};
    use crate::diagnostic::DiagnosticCode;

    fn convert(text: &str) -> super::super::MarkdownConversion {
        convert_markdown(&value(text), &context())
    }

    fn assert_raw_rejected(source: &str) {
        let conversion = convert(source);
        let plain = convert_markdown_without_raw_rd(&value(source), &context());
        assert!(
            conversion
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == DiagnosticCode::UnsupportedRawRdMacro),
            "missing raw Rd diagnostic for {source:?}"
        );
        assert_eq!(
            conversion.fragment.nodes, plain.fragment.nodes,
            "rejected raw Rd changed plain conversion: {source:?}"
        );
    }

    #[test]
    fn one_argument_equations_are_structural() {
        for (source, tag) in [(r"\eqn{x^2}", RdTag::Eqn), (r"\deqn{x^2}", RdTag::Deqn)] {
            let conversion = convert(source);
            assert_eq!(
                conversion.fragment.nodes,
                vec![RdNode::tagged(
                    tag.clone(),
                    None,
                    vec![RdNode::group(vec![RdNode::Verb("x^2".into())])],
                )]
            );
            assert!(conversion.diagnostics.is_empty());
            if tag == RdTag::Eqn {
                assert_serialized_body(conversion.fragment.nodes.clone(), r"\eqn{x^2}");
            }
        }
    }

    #[test]
    fn zero_argument_prose_macros_are_structural_zero_child_nodes() {
        for (source, tag) in [
            (r"\R", RdTag::R),
            (r"\dots", RdTag::Dots),
            (r"\ldots", RdTag::LDots),
            (r"\cr", RdTag::Cr),
            (r"\sspace", RdTag::Sspace),
        ] {
            let conversion = convert(source);
            assert_eq!(
                conversion.fragment.nodes,
                vec![RdNode::tagged(tag, None, Vec::new())]
            );
            assert!(conversion.diagnostics.is_empty());
            if source != r"\sspace" {
                assert_serialized_body(conversion.fragment.nodes, source);
            }
        }
    }

    #[test]
    fn system_space_symbol_serializes_in_prose_context() {
        let conversion = convert(r"sentence\sspace here");
        assert!(conversion.diagnostics.is_empty());
        let body = serialize(vec![RdNode::tagged(
            RdTag::Description,
            None,
            conversion.fragment.nodes,
        )]);
        assert_eq!(body, r"\description{sentence\sspace here}");
    }

    #[test]
    fn zero_argument_prose_macros_survive_adjacent_markup_and_each_other() {
        let conversion = convert(r"*before \R* \dots \ldots");
        assert_eq!(
            conversion.fragment.nodes,
            vec![
                RdNode::tagged(
                    RdTag::Emph,
                    None,
                    vec![
                        RdNode::Text("before ".into()),
                        RdNode::tagged(RdTag::R, None, Vec::new()),
                    ],
                ),
                RdNode::Text(" ".into()),
                RdNode::tagged(RdTag::Dots, None, Vec::new()),
                RdNode::Text(" ".into()),
                RdNode::tagged(RdTag::LDots, None, Vec::new()),
            ]
        );
        assert!(conversion.diagnostics.is_empty());
        assert_serialized_body(conversion.fragment.nodes, r"\emph{before \R} \dots \ldots");
    }

    #[test]
    fn escaped_and_code_opaque_prose_macros_stay_literal() {
        let escaped = convert(r"\\R \\dots \\ldots");
        assert_eq!(
            escaped.fragment.nodes,
            vec![RdNode::Text(r"\R \dots \ldots".into())]
        );
        assert!(escaped.diagnostics.is_empty());

        let code = convert(r"`\R \dots \ldots`");
        assert_eq!(
            code.fragment.nodes,
            vec![RdNode::tagged(
                RdTag::Verb,
                None,
                vec![RdNode::Verb(r"\R \dots \ldots".into())],
            )]
        );
        assert!(code.diagnostics.is_empty());

        let code_block = convert("```\n\\R \\dots \\ldots\n```");
        assert_eq!(
            code_block.fragment.nodes,
            vec![RdNode::tagged(
                RdTag::Preformatted,
                None,
                vec![RdNode::Verb("\\R \\dots \\ldots\n".into())],
            )]
        );
        assert!(code_block.diagnostics.is_empty());
    }

    #[test]
    fn contextual_zero_argument_macros_remain_rejected_in_prose() {
        for source in [
            r"\tab",
            r"\R{}",
            r"\dots{}",
            r"\ldots{}",
            r"\cr{}",
            r"\sspace{}",
        ] {
            assert_raw_rejected(source);
        }
    }

    #[test]
    fn empty_equation_argument_is_an_empty_group() {
        let conversion = convert(r"\eqn{}");
        assert_eq!(
            conversion.fragment.nodes,
            vec![RdNode::tagged(
                RdTag::Eqn,
                None,
                vec![RdNode::group(Vec::new())],
            )]
        );
        assert_serialized_body(conversion.fragment.nodes, r"\eqn{}");
    }

    #[test]
    fn equations_at_paragraph_start_are_structural() {
        let conversion = convert(r"\eqn{x} after");
        assert_eq!(
            conversion.fragment.nodes,
            vec![
                RdNode::tagged(
                    RdTag::Eqn,
                    None,
                    vec![RdNode::group(vec![RdNode::Verb("x".into())])],
                ),
                RdNode::Text(" after".into()),
            ]
        );
        assert!(conversion.diagnostics.is_empty());

        let conversion = convert(r"\eqn{x}");
        assert_eq!(
            conversion.fragment.nodes,
            vec![RdNode::tagged(
                RdTag::Eqn,
                None,
                vec![RdNode::group(vec![RdNode::Verb("x".into())])],
            )]
        );
        assert!(conversion.diagnostics.is_empty());

        let conversion = convert(r"\eqn{x} *after*");
        assert_eq!(
            conversion.fragment.nodes,
            vec![
                RdNode::tagged(
                    RdTag::Eqn,
                    None,
                    vec![RdNode::group(vec![RdNode::Verb("x".into())])],
                ),
                RdNode::Text(" ".into()),
                RdNode::tagged(RdTag::Emph, None, vec![RdNode::Text("after".into())]),
            ]
        );
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn equations_after_emphasis_are_not_hidden_by_expired_constructs() {
        let conversion = convert(r"*a \eqn{x}* then \eqn{y}");
        assert_eq!(
            conversion.fragment.nodes,
            vec![
                RdNode::tagged(
                    RdTag::Emph,
                    None,
                    vec![
                        RdNode::Text("a ".into()),
                        RdNode::tagged(
                            RdTag::Eqn,
                            None,
                            vec![RdNode::group(vec![RdNode::Verb("x".into())])],
                        ),
                    ],
                ),
                RdNode::Text(" then ".into()),
                RdNode::tagged(
                    RdTag::Eqn,
                    None,
                    vec![RdNode::group(vec![RdNode::Verb("y".into())])],
                ),
            ]
        );
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn one_argument_serialization_keeps_percent_and_braces() {
        let conversion = convert(r"\eqn{x % \{y\}}");
        assert_eq!(
            conversion.fragment.nodes,
            vec![RdNode::tagged(
                RdTag::Eqn,
                None,
                vec![RdNode::group(vec![RdNode::Verb(r"x % \{y\}".into())])],
            )]
        );
        assert_serialized_body(conversion.fragment.nodes, r"\eqn{x % \{y\}}");
    }

    #[test]
    fn two_argument_equations_use_distinct_writer_modes() {
        let conversion = convert(r"\eqn{a\{b\}c % d}{second \{x\} % y}");
        assert_eq!(
            conversion.fragment.nodes,
            vec![RdNode::tagged(
                RdTag::Eqn,
                None,
                vec![
                    RdNode::group(vec![RdNode::Verb(r"a\{b\}c % d".into())]),
                    RdNode::group(vec![RdNode::Verb(r"second \{x\} % y".into())]),
                ],
            )]
        );
        assert_serialized_body(
            conversion.fragment.nodes,
            r"\eqn{a\{b\}c % d}{second \\\{x\\\} \% y}",
        );
    }

    #[test]
    fn nested_and_escaped_braces_are_scanned() {
        let conversion = convert(r"\eqn{\frac{a}{b}}{a\}b}");
        assert_eq!(conversion.fragment.nodes.len(), 1);
        let equation = conversion.fragment.nodes[0].as_tagged().expect("equation");
        assert_eq!(equation.tag(), &RdTag::Eqn);
        assert_eq!(
            equation.children()[0]
                .as_group()
                .expect("first group")
                .children(),
            &[RdNode::Verb(r"\frac{a}{b}".into())]
        );
        assert_eq!(
            equation.children()[1]
                .as_group()
                .expect("second group")
                .children(),
            &[RdNode::Verb(r"a\}b".into())]
        );
    }

    #[test]
    fn equation_shapes_from_prose_are_structural() {
        for source in [
            r#"\deqn{u = \frac{a}{b} \; for \; b > 0}"#,
            r#"\deqn{v = 1 - \exp \left\{ \frac{a}{b} \right\} \; for \; b > 0}"#,
            r#"\eqn{|x| \leq \max \{ |a|, |b| \}}"#,
        ] {
            let conversion = convert(source);
            assert!(
                conversion.diagnostics.is_empty(),
                "unexpected diagnostics for {source:?}: {:?}",
                conversion.diagnostics
            );
        }

        let conversion = convert(
            r#"\deqn{v = 1 - \exp \left\{ \frac{a}{b} \right\}
\; for \; b > 0}"#,
        );
        assert!(
            conversion.diagnostics.is_empty(),
            "unexpected diagnostics for multiline equation: {:?}",
            conversion.diagnostics
        );

        for source in [
            r#"Specify a ratio with \deqn{u = \frac{a}{b} \; for \; b > 0}."#,
            r#"Specify a bound with \eqn{|x| \leq \max \{ |a|, |b| \}}"#,
        ] {
            let conversion = convert(source);
            assert!(
                conversion.diagnostics.is_empty(),
                "unexpected diagnostics for prose equation {source:?}: {:?}",
                conversion.diagnostics
            );
        }
    }

    #[test]
    fn unterminated_supported_equation_is_diagnosed_and_kept() {
        assert_raw_rejected(r"before \eqn{x");
    }

    #[test]
    fn code_shielded_malformed_equation_does_not_hide_later_ones() {
        // The unterminated introducer is inside a code span, so the code span
        // already made it literal. Consuming to the end of the value would let
        // something that was never a macro swallow every macro after it, and
        // silently: the loss carried no diagnostic.
        let conversion = convert("`\\eqn{x` after \\eqn{y}");
        assert_eq!(
            conversion.fragment.nodes,
            vec![
                RdNode::tagged(RdTag::Verb, None, vec![RdNode::Verb("\\eqn{x".into())]),
                RdNode::Text(" after ".into()),
                RdNode::tagged(
                    RdTag::Eqn,
                    None,
                    vec![RdNode::group(vec![RdNode::Verb("y".into())])]
                ),
            ]
        );
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn balanced_outer_brace_does_not_unshield_a_malformed_code_span_macro() {
        let conversion = convert(r"`\eqn{x` after \eqn{y} }");
        assert_eq!(
            conversion.fragment.nodes,
            vec![
                RdNode::tagged(RdTag::Verb, None, vec![RdNode::Verb(r"\eqn{x".into())]),
                RdNode::Text(" after ".into()),
                RdNode::tagged(
                    RdTag::Eqn,
                    None,
                    vec![RdNode::group(vec![RdNode::Verb("y".into())])],
                ),
                RdNode::Text(" }".into()),
            ]
        );
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn balanced_code_span_macro_does_not_hide_a_later_equation() {
        let conversion = convert(r"`\eqn{x}` after \eqn{y}");
        assert_eq!(
            conversion.fragment.nodes,
            vec![
                RdNode::tagged(RdTag::Verb, None, vec![RdNode::Verb(r"\eqn{x}".into())]),
                RdNode::Text(" after ".into()),
                RdNode::tagged(
                    RdTag::Eqn,
                    None,
                    vec![RdNode::group(vec![RdNode::Verb("y".into())])],
                ),
            ]
        );
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn unsupported_code_span_macro_does_not_hide_a_later_equation() {
        let conversion = convert(r"`\code{x}` after \eqn{y}");
        assert_eq!(
            conversion.fragment.nodes,
            vec![
                RdNode::tagged(RdTag::Verb, None, vec![RdNode::Verb(r"\code{x}".into())]),
                RdNode::Text(" after ".into()),
                RdNode::tagged(
                    RdTag::Eqn,
                    None,
                    vec![RdNode::group(vec![RdNode::Verb("y".into())])],
                ),
            ]
        );
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn malformed_code_block_macro_does_not_hide_a_later_equation() {
        let conversion = convert("```\n\\eqn{x\n```\nafter \\eqn{y}");
        assert_eq!(
            conversion.fragment.nodes,
            vec![
                RdNode::tagged(
                    RdTag::Preformatted,
                    None,
                    vec![RdNode::Verb("\\eqn{x\n".into())],
                ),
                RdNode::Text("\n".into()),
                RdNode::Text("\n".into()),
                RdNode::Text("after ".into()),
                RdNode::tagged(
                    RdTag::Eqn,
                    None,
                    vec![RdNode::group(vec![RdNode::Verb("y".into())])],
                ),
            ]
        );
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn shielded_introducers_in_two_code_spans_do_not_hide_intervening_equations() {
        let conversion = convert(r"`\eqn{x` before \eqn{y} after `\eqn{z` and \eqn{q}");
        assert_eq!(
            conversion.fragment.nodes,
            vec![
                RdNode::tagged(RdTag::Verb, None, vec![RdNode::Verb(r"\eqn{x".into())]),
                RdNode::Text(" before ".into()),
                RdNode::tagged(
                    RdTag::Eqn,
                    None,
                    vec![RdNode::group(vec![RdNode::Verb("y".into())])],
                ),
                RdNode::Text(" after ".into()),
                RdNode::tagged(RdTag::Verb, None, vec![RdNode::Verb(r"\eqn{z".into())]),
                RdNode::Text(" and ".into()),
                RdNode::tagged(
                    RdTag::Eqn,
                    None,
                    vec![RdNode::group(vec![RdNode::Verb("q".into())])],
                ),
            ]
        );
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn an_unshielded_malformed_equation_still_claims_the_rest() {
        // Without a code span there is no reason to stop: the group never
        // closes, so everything after it is that macro's malformed argument.
        // It is diagnosed as one candidate rather than silently split.
        assert_raw_rejected(r"before \eqn{x more \eqn{y}");
    }

    #[test]
    fn unterminated_second_argument_is_diagnosed_and_kept() {
        assert_raw_rejected(r"before \eqn{x}{y");
    }

    #[test]
    fn equation_markup_is_not_markdown() {
        let conversion = convert(r"\eqn{x_y * [z]}");
        assert_eq!(
            conversion.fragment.nodes,
            vec![RdNode::tagged(
                RdTag::Eqn,
                None,
                vec![RdNode::group(vec![RdNode::Verb("x_y * [z]".into())])],
            )]
        );
    }

    #[test]
    fn unsupported_raw_macros_are_diagnosed_and_kept() {
        for source in [r"before \code{x} after", r"before \link{y} after"] {
            assert_raw_rejected(source);
        }
    }

    #[test]
    fn a_control_sequence_without_a_group_is_literal() {
        let conversion = convert(r"before \alpha after");
        assert_eq!(
            conversion.fragment.nodes,
            vec![RdNode::Text(r"before \alpha after".into())]
        );
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn escaped_introducer_is_literal() {
        let conversion = convert(r"\\eqn{x}");
        assert_eq!(
            conversion.fragment.nodes,
            vec![RdNode::Text(r"\eqn{x}".into())]
        );
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn long_backslash_runs_keep_escape_parity_linear() {
        let source = format!("{}eqn{{x}}", "\\".repeat(12_001));
        let mut candidates = Vec::new();
        scan_source(&source, &[], &mut candidates);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].source, 12_000..source.len());
        assert!(matches!(
            candidates[0].kind,
            Some(RawRdKind::Equation { .. })
        ));
    }

    #[test]
    fn many_candidates_interleaved_with_inline_markup_are_swept() {
        let mut source = String::new();
        for _ in 0..8_000 {
            source.push_str("*x* ");
            source.push_str(r"\eqn{x} ");
        }
        let conversion = convert(&source);
        let equations = conversion
            .fragment
            .nodes
            .iter()
            .filter(|node| {
                node.as_tagged()
                    .is_some_and(|tag| matches!(tag.tag(), RdTag::Eqn))
            })
            .count();
        assert_eq!(equations, 8_000);
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn multiple_candidates_in_one_text_event_are_spliced() {
        let conversion = convert(r"before \eqn{x} middle \eqn{y} after");
        let equations = conversion
            .fragment
            .nodes
            .iter()
            .filter(|node| {
                node.as_tagged()
                    .is_some_and(|tag| matches!(tag.tag(), RdTag::Eqn))
            })
            .count();
        assert_eq!(equations, 2);
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn equations_continue_after_a_spanning_candidate() {
        let conversion = convert("a \\eqn{x\ny} \\eqn{z}");
        assert_eq!(
            conversion.fragment.nodes,
            vec![
                RdNode::Text("a ".into()),
                RdNode::tagged(
                    RdTag::Eqn,
                    None,
                    vec![RdNode::group(vec![
                        RdNode::Verb("x\n".into()),
                        RdNode::Verb("y".into()),
                    ])],
                ),
                RdNode::Text(" ".into()),
                RdNode::tagged(
                    RdTag::Eqn,
                    None,
                    vec![RdNode::group(vec![RdNode::Verb("z".into())])],
                ),
            ]
        );
        assert!(conversion.diagnostics.is_empty());

        let conversion = convert("a \\eqn{x\ny} \\eqn{z} \\eqn{q}");
        assert_eq!(
            conversion.fragment.nodes,
            vec![
                RdNode::Text("a ".into()),
                RdNode::tagged(
                    RdTag::Eqn,
                    None,
                    vec![RdNode::group(vec![
                        RdNode::Verb("x\n".into()),
                        RdNode::Verb("y".into()),
                    ])],
                ),
                RdNode::Text(" ".into()),
                RdNode::tagged(
                    RdTag::Eqn,
                    None,
                    vec![RdNode::group(vec![RdNode::Verb("z".into())])],
                ),
                RdNode::Text(" ".into()),
                RdNode::tagged(
                    RdTag::Eqn,
                    None,
                    vec![RdNode::group(vec![RdNode::Verb("q".into())])],
                ),
            ]
        );
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn macro_names_and_options_are_diagnosed() {
        for source in [
            r"\S3method{print}{foo}",
            r"\linkS4class{x}",
            r"\link[=foo]{bar}",
        ] {
            assert_raw_rejected(source);
        }
    }

    #[test]
    fn sentinel_like_text_does_not_collide_with_a_protected_macro() {
        let conversion = convert(r"xx\eqn{x}xx");
        assert_eq!(
            conversion.fragment.nodes,
            vec![
                RdNode::Text("xx".into()),
                RdNode::tagged(
                    RdTag::Eqn,
                    None,
                    vec![RdNode::group(vec![RdNode::Verb("x".into())])],
                ),
                RdNode::Text("xx".into()),
            ]
        );
    }

    #[test]
    fn raw_macros_inside_code_are_not_prose_macros() {
        let source = format!("{}\\eqn{{x}}{}", char::from(96), char::from(96));
        let conversion = convert(&source);
        assert_eq!(
            conversion.fragment.nodes,
            vec![RdNode::tagged(
                RdTag::Verb,
                None,
                vec![RdNode::Verb(r"\eqn{x}".into())],
            )]
        );
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn malformed_raw_rd_inside_code_does_not_diagnose() {
        let conversion = convert(r"`\eqn{x` after");
        assert_eq!(
            conversion.fragment.nodes,
            vec![
                RdNode::tagged(RdTag::Verb, None, vec![RdNode::Verb(r"\eqn{x".into())],),
                RdNode::Text(" after".into()),
            ]
        );
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn deliberate_positions_are_rejected_without_rewriting_events() {
        for source in [
            "before \\eqn{x\n\ny} after",
            "\\eqn{x\n\ny}",
            r"before \eqn{*x}*",
            r"[before](url\eqn{x})",
            r"<https://e/\eqn{ab}>",
            "[ref]: https://e/\\eqn{x} \"title\"",
            r#"[before](url "title \eqn{x}")"#,
            r"![before](url\eqn{x})",
            r#"<span title="\eqn{x}">text</span>"#,
            r#"before <span title="\eqn{x">} after"#,
            r#"\eqn{a<span title="}">x"#,
        ] {
            assert_raw_rejected(source);
        }
    }

    #[test]
    fn multiline_equation_with_list_continuations_is_rejected_without_rewriting() {
        assert_raw_rejected("before \\eqn{a\n+ b\n+ c} after");
    }

    #[test]
    fn equations_work_inside_lists_and_emphasis() {
        let list = convert("- *\\eqn{x^2}*");
        let expected = RdNode::tagged(
            RdTag::Itemize,
            None,
            vec![
                RdNode::Text("\n".into()),
                RdNode::tagged(RdTag::Item, None, vec![]),
                RdNode::Text(" ".into()),
                RdNode::tagged(
                    RdTag::Emph,
                    None,
                    vec![RdNode::tagged(
                        RdTag::Eqn,
                        None,
                        vec![RdNode::group(vec![RdNode::Verb("x^2".into())])],
                    )],
                ),
                RdNode::Text("\n".into()),
            ],
        );
        assert_eq!(list.fragment.nodes, vec![expected]);
        assert!(list.diagnostics.is_empty());
    }

    #[test]
    fn raw_ranges_cover_all_physical_lines() {
        let source = crate::source::SourceFile::new(
            std::path::PathBuf::from("test.R"),
            "before \\eqn{a\n#' b\n#' c} after".to_owned(),
        );
        let first_end = source.text().find('\n').expect("first newline");
        let second_line_start = first_end + 1;
        let second_content_start = second_line_start + 3;
        let second_end = source.text()[second_line_start..]
            .find('\n')
            .map(|offset| second_line_start + offset)
            .expect("second newline");
        let third_content_start = second_end + 1 + 3;
        let sourced = crate::tags::SourcedText::from_lines(
            &source,
            &[
                crate::source::Span::new(
                    crate::source::FileId::new(0),
                    crate::source::TextRange::new(0, first_end as u32),
                ),
                crate::source::Span::new(
                    crate::source::FileId::new(0),
                    crate::source::TextRange::new(second_content_start as u32, second_end as u32),
                ),
                crate::source::Span::new(
                    crate::source::FileId::new(0),
                    crate::source::TextRange::new(
                        third_content_start as u32,
                        source.text().len() as u32,
                    ),
                ),
            ],
            crate::tags::NormalizeHead::Intro,
        );
        let conversion = convert_markdown(&crate::tags::MarkdownText::new(sourced), &context());
        let equation_index = conversion
            .fragment
            .nodes
            .iter()
            .position(|node| node.as_tagged().is_some_and(|tag| tag.tag() == &RdTag::Eqn))
            .expect("equation node");
        let origin = conversion
            .fragment
            .origins
            .iter()
            .find(|origin| origin.path == FragmentPath::root(equation_index))
            .expect("equation origin");
        let equation = conversion.fragment.nodes[equation_index]
            .as_tagged()
            .expect("equation");
        assert_eq!(
            equation.children()[0]
                .as_group()
                .expect("equation argument")
                .children(),
            &[
                RdNode::Verb("a\n".into()),
                RdNode::Verb("b\n".into()),
                RdNode::Verb("c".into()),
            ]
        );
        assert_eq!(origin.spans.len(), 3);
        assert!(
            origin
                .spans
                .windows(2)
                .all(|spans| { spans[0].range.start() < spans[1].range.start() })
        );
        let leaf_spans = conversion
            .fragment
            .origins
            .iter()
            .filter(|origin| {
                origin.path.segments().len() == 3
                    && origin.path.segments()[..2]
                        == [
                            FragmentPathSegment::Child(equation_index),
                            FragmentPathSegment::Child(0),
                        ]
            })
            .map(|origin| origin.spans.clone())
            .collect::<Vec<_>>();
        assert_eq!(leaf_spans.len(), 3);
        assert!(leaf_spans.windows(2).all(|spans| spans[0] == spans[1]));
    }
}
