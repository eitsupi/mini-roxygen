//! Converts the supported Markdown subset into source-backed Rd fragments.
//!
//! The converter owns event dispatch and frame lifecycle; focused helper
//! modules own fragment provenance, frame state, separators, code, lists, and
//! unsupported-construct recovery.

use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Tag, TagEnd};
use rd_ast::{RdNode, RdTag};

use crate::diagnostic::{DiagnosticCode, Diagnostics};
use crate::markdown::markdown_parser;
use crate::source::{Span, TextRange};
use crate::tags::MarkdownText;

mod code;
mod fragment;
mod frame;
mod leaf;
mod link;
mod list;
mod raw_rd;
pub(crate) mod section_key;
mod separator;
mod table;
mod unsupported;

#[cfg(test)]
pub(crate) mod test_support;

pub(crate) use fragment::{FragmentPath, FragmentPathSegment, LatexFragment};
use frame::{Frame, FrameKind, NodeWithOrigin};
use separator::PendingSeparator;

/// A Markdown conversion result containing Rd nodes and source diagnostics.
pub(crate) struct MarkdownConversion {
    /// The converted nodes and their source origins.
    pub(crate) fragment: LatexFragment,
    /// Problems found while converting unsupported Markdown constructs.
    pub(crate) diagnostics: Diagnostics,
}

/// Context supplied by the package-level conversion caller.
pub(crate) struct MarkdownContext<'a> {
    pub(crate) current_package: Option<&'a str>,
    pub(crate) links: &'a dyn HelpLinkResolver,
    pub(crate) inline_r_session: Option<&'a crate::inline_r::InlineRSession<'a>>,
}

/// Resolves an unqualified help topic without coupling Markdown conversion to
/// package metadata or a particular help-database implementation.
pub(crate) trait HelpLinkResolver {
    fn resolve_unqualified(&self, topic: &str) -> LinkResolution;
}

/// The package-level result for an unqualified help topic.
pub(crate) enum LinkResolution {
    Local,
    /// No resolver was able to search for the target; this is not diagnostic.
    Unchecked,
    External {
        package: String,
    },
    Unresolved,
    Ambiguous {
        packages: Vec<String>,
    },
}

/// Converts normalized Markdown into a real Rd fragment.
pub(crate) fn convert_markdown(
    value: &MarkdownText,
    context: &MarkdownContext<'_>,
) -> MarkdownConversion {
    convert_markdown_with_raw_rd(value, context, true)
}

/// Builds a section-title key from the Markdown-to-Rd conversion.
///
/// This helper intentionally discards conversion diagnostics. It is used for
/// lookup keys before the content is rendered for output; discarding the
/// exploratory diagnostics ensures an unsupported title is reported only by
/// that final rendering pass.
pub(crate) fn markdown_section_key(
    value: &MarkdownText,
    current_package: Option<&str>,
    links: &dyn crate::rd::RdLinkResolver,
    inline_r_session: Option<&crate::inline_r::InlineRSession<'_>>,
) -> section_key::SectionTitleKey {
    // Use the same adapter and rendering context as the final Rd pass. The
    // conversion diagnostics are intentionally discarded here because the
    // final rendering pass reports them; otherwise an exploratory key lookup
    // would report a title diagnostic twice.
    let links = crate::rd::LinkAdapter { links };
    let context = MarkdownContext {
        current_package,
        links: &links,
        inline_r_session,
    };
    let conversion = convert_markdown(value, &context);
    section_key::SectionTitleKey::from_rd(&conversion.fragment.nodes)
}

pub(crate) fn rd_section_key(nodes: &[RdNode]) -> section_key::SectionTitleKey {
    section_key::SectionTitleKey::from_rd(nodes)
}

#[cfg(test)]
pub(crate) fn convert_markdown_without_raw_rd(
    value: &MarkdownText,
    context: &MarkdownContext<'_>,
) -> MarkdownConversion {
    convert_markdown_with_raw_rd(value, context, false)
}

fn convert_markdown_with_raw_rd(
    value: &MarkdownText,
    context: &MarkdownContext<'_>,
    enable_raw_rd: bool,
) -> MarkdownConversion {
    let events = markdown_parser(value.as_str()).collect::<Vec<_>>();
    let mut converter = Converter::new(value, context, &events, enable_raw_rd);

    for (event, range) in events {
        converter.handle(event, range);
    }

    converter.finish()
}

fn heading_level(level: HeadingLevel) -> usize {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

struct Converter<'a> {
    value: &'a MarkdownText,
    context: &'a MarkdownContext<'a>,
    raw_rd: raw_rd::RawRdPreparation,
    frames: Vec<Frame>,
    diagnostics: Diagnostics,
}

impl<'a> Converter<'a> {
    fn new(
        value: &'a MarkdownText,
        context: &'a MarkdownContext<'a>,
        events: &[(Event<'_>, Range<usize>)],
        enable_raw_rd: bool,
    ) -> Self {
        let raw_rd = if enable_raw_rd {
            raw_rd::RawRdPreparation::new(value.as_str(), events)
        } else {
            raw_rd::RawRdPreparation::disabled()
        };
        let mut converter = Self {
            value,
            context,
            raw_rd,
            frames: vec![Frame::root()],
            diagnostics: Diagnostics::new(),
        };
        let unsupported = converter.raw_rd.unsupported.clone();
        raw_rd::diagnose(&mut converter, &unsupported);
        converter
    }

    fn handle(&mut self, event: Event<'_>, range: Range<usize>) {
        match event {
            Event::Start(tag) => {
                if !self.raw_rd.suppresses(&range) {
                    self.start(tag, range);
                }
            }
            Event::End(tag) => {
                if !self.raw_rd.suppresses(&range) {
                    self.end(tag, range);
                }
            }
            Event::Text(text) => self.text(&text, range),
            Event::SoftBreak | Event::HardBreak => {
                if !self.raw_rd.suppresses(&range) {
                    self.text("\n", range);
                }
            }
            Event::Code(text) => self.code(&text, range),
            Event::InlineMath(text) | Event::DisplayMath(text) | Event::FootnoteReference(text) => {
                if !self.raw_rd.suppresses(&range) {
                    self.unsupported_leaf("inline Markdown construct", range.clone());
                    self.append_text(&text, range);
                }
            }
            Event::Html(text) | Event::InlineHtml(text) => {
                if self.raw_rd.suppresses(&range) {
                    return;
                }
                // A block HTML envelope diagnoses itself when it closes. The
                // child Html event is its literal payload, not a second
                // construct to report.
                if !self.in_html_block() {
                    self.unsupported_leaf("raw HTML", range.clone());
                }
                self.append_text(&text, range);
            }
            Event::Rule => {
                self.unsupported_leaf("Markdown construct", range);
                self.record_boundary(PendingSeparator::Paragraph);
            }
            Event::TaskListMarker(_) => {
                if self.raw_rd.suppresses(&range) {
                    return;
                }
                self.unsupported_leaf("Markdown construct", range);
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>, range: Range<usize>) {
        if let Tag::Heading { level, .. } = &tag {
            let level = heading_level(*level);
            if level >= 2
                && matches!(
                    self.frames.last().map(|frame| &frame.kind),
                    Some(FrameKind::Root | FrameKind::Subsection { .. })
                )
            {
                self.close_sections(level, range.start);
                self.open_heading(range.start);
                self.frames.push(Frame::new(FrameKind::Heading {
                    level,
                    start: range.start,
                }));
                return;
            }
        }
        self.prepare_block_start(&tag, range.start);
        // A nested block starts on a fresh line in roxygen2. pulldown-cmark
        // closes the item's paragraph before emitting the child list/table,
        // but that paragraph boundary is otherwise lost while the child is
        // accumulated in the same item frame.
        if matches!(tag, Tag::List(_) | Tag::Table(_) | Tag::CodeBlock(_))
            && self
                .frames
                .last()
                .is_some_and(|frame| matches!(frame.kind, FrameKind::Item))
        {
            self.frames
                .last_mut()
                .expect("the item frame exists")
                .pending_separator = PendingSeparator::Line;
        }
        if matches!(tag, Tag::List(_))
            && self
                .frames
                .last()
                .is_some_and(|frame| frame.pending_separator == PendingSeparator::Paragraph)
        {
            self.frames
                .last_mut()
                .expect("the parent frame exists")
                .pending_separator = PendingSeparator::Line;
        }
        if let Tag::Item = tag {
            let spans = self.spans(range.start, range.end);
            let anchor = self.anchor(range.start);
            let parent = self.frames.last_mut().expect("the root frame exists");
            separator::materialize_separator(parent, anchor);
            frame::append_node(
                parent,
                NodeWithOrigin {
                    node: RdNode::tagged(RdTag::Item, None, Vec::new()),
                    children: Vec::new(),
                    spans,
                },
            );
            let mut item = Frame::new(FrameKind::Item);
            // Rd needs a separator after the zero-child \item marker. Keep
            // it in the first body leaf so the writer emits `\item text`.
            item.pending.text.push(' ');
            if let Some(anchor) = anchor {
                item.pending.spans.push(anchor);
            }
            self.frames.push(item);
            return;
        }

        let kind = match tag {
            Tag::Link { dest_url, .. } => FrameKind::Link {
                destination: dest_url.into_string(),
                start: range.start,
            },
            Tag::Paragraph => FrameKind::Paragraph,
            Tag::CodeBlock(kind) => {
                let info = match kind {
                    CodeBlockKind::Fenced(info) => info.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                FrameKind::CodeBlock {
                    executable_r: code::is_executable_r_chunk(&info),
                    start: range.start,
                }
            }
            Tag::Emphasis => FrameKind::Tagged {
                tag: RdTag::Emph,
                start: range.start,
            },
            Tag::Strong => FrameKind::Tagged {
                tag: RdTag::Strong,
                start: range.start,
            },
            Tag::List(start) => {
                if let Some(start) = start
                    && start != 1
                {
                    unsupported::diagnose_range(
                        self,
                        "ordered Markdown list does not start at 1",
                        range.start,
                        range.end,
                    );
                }
                FrameKind::List {
                    tag: if start.is_some() {
                        RdTag::Enumerate
                    } else {
                        RdTag::Itemize
                    },
                    start: range.start,
                }
            }
            Tag::Table(alignments) => {
                if alignments.is_empty() {
                    // A valid GFM table always has a column; an empty
                    // colspec is rejected by the writer, so recover instead.
                    FrameKind::Unsupported {
                        name: "table".to_owned(),
                        start: range.start,
                    }
                } else {
                    FrameKind::Table {
                        alignments,
                        rows: Vec::new(),
                        start: range.start,
                    }
                }
            }
            // A row or cell only becomes a table frame when its parent is the
            // frame that will consume it. Otherwise the enclosing table went
            // to the recovery path above, and its parts have to follow it
            // there: a table frame with nowhere to hand its result is what
            // makes the recovery branch unsound.
            Tag::TableHead | Tag::TableRow => match self.frames.last().map(|frame| &frame.kind) {
                Some(FrameKind::Table { alignments, .. }) => FrameKind::TableRow {
                    cells: Vec::new(),
                    width: alignments.len(),
                    start: range.start,
                },
                _ => FrameKind::Unsupported {
                    name: unsupported::unsupported_tag_name(&tag),
                    start: range.start,
                },
            },
            Tag::TableCell => match self.frames.last().map(|frame| &frame.kind) {
                Some(FrameKind::TableRow { .. }) => FrameKind::TableCell,
                _ => FrameKind::Unsupported {
                    name: unsupported::unsupported_tag_name(&tag),
                    start: range.start,
                },
            },
            other => FrameKind::Unsupported {
                name: unsupported::unsupported_tag_name(&other),
                start: range.start,
            },
        };
        self.frames.push(Frame::new(kind));
    }

    fn end(&mut self, tag: TagEnd, range: Range<usize>) {
        let frame = self.frames.pop().expect("Markdown frames are balanced");
        let end = range.end;
        let heading = match &frame.kind {
            FrameKind::Heading { level, start } => Some((*level, *start)),
            _ => None,
        };
        if let Some((level, start)) = heading {
            let frame::FinishedFrame::Nodes(title) = frame::finish_frame(self, frame, end) else {
                unreachable!("a heading frame contains only title nodes")
            };
            self.frames.push(Frame::subsection(level, start, title));
            return;
        }
        let unsupported = match &frame.kind {
            FrameKind::Unsupported { name, start } => Some((name.clone(), *start)),
            _ => None,
        };
        let finished = frame::finish_frame(self, frame, end);
        let boundary = separator::separator_after(tag);
        let boundary_anchor = self.anchor(range.start);

        if let Some((name, start)) = unsupported
            && !unsupported::is_redundant_unsupported_envelope(&self.frames, &name)
        {
            unsupported::diagnose_range(self, &name, start, end);
        }

        let parent = self
            .frames
            .last_mut()
            .expect("the root frame cannot be popped");
        match finished {
            frame::FinishedFrame::Cell(cell) => {
                let FrameKind::TableRow { cells, .. } = &mut parent.kind else {
                    unreachable!("table cells are nested directly in a table row")
                };
                cells.push(cell);
            }
            frame::FinishedFrame::Row(nodes) => {
                let FrameKind::Table { rows, .. } = &mut parent.kind else {
                    unreachable!("table rows are nested directly in a table")
                };
                rows.push(nodes);
            }
            frame::FinishedFrame::Nodes(nodes) => {
                for node in nodes {
                    separator::append_flattened_node(parent, node, boundary_anchor);
                }
            }
        }
        parent.pending_separator = parent.pending_separator.max(boundary);
    }

    fn text(&mut self, text: &str, range: Range<usize>) {
        raw_rd::append_text(self, text, range);
    }

    fn append_text(&mut self, text: &str, range: Range<usize>) {
        if text.is_empty() {
            return;
        }
        let spans = self.spans(range.start, range.end);
        let anchor = self.anchor(range.start);
        let frame = self.frames.last_mut().expect("the root frame exists");
        separator::materialize_separator(frame, anchor);
        frame.pending.text.push_str(text);
        frame::append_spans(&mut frame.pending.spans, spans);
    }

    fn code(&mut self, code: &str, range: Range<usize>) {
        let is_multiline = self.value.as_str()[range.clone()]
            .bytes()
            .any(|byte| matches!(byte, b'\n' | b'\r'));
        if let Some(expression) = code.strip_prefix("r ") {
            if is_multiline {
                unsupported::diagnose_multiline_inline_r(self, range.start, range.end);
            } else if let Some(session) = self.context.inline_r_session {
                if let Some(nodes) = session.lookup(expression) {
                    let spans = self.spans(range.start, range.end);
                    let anchor = self.anchor(range.start);
                    let frame = self.frames.last_mut().expect("the root frame exists");
                    for node in nodes {
                        separator::append_flattened_node(
                            frame,
                            frame::node_with_origin(node, spans.clone()),
                            anchor,
                        );
                    }
                    return;
                }
                unsupported::diagnose_undefined_inline_r(self, range.start, range.end);
            } else {
                unsupported::diagnose_code_range(
                    self,
                    DiagnosticCode::UnsupportedInlineR,
                    "unsupported inline R code: evaluation is not supported",
                    "unsupported inline R code",
                    range.start,
                    range.end,
                );
            }
            let spans = self.spans(range.start, range.end);
            let anchor = self.anchor(range.start);
            let frame = self.frames.last_mut().expect("the root frame exists");
            separator::materialize_separator(frame, anchor);
            frame::append_node(
                frame,
                NodeWithOrigin {
                    node: RdNode::tagged(RdTag::Verb, None, vec![RdNode::Verb(code.to_owned())]),
                    children: vec![NodeWithOrigin {
                        node: RdNode::Verb(code.to_owned()),
                        children: Vec::new(),
                        spans: spans.clone(),
                    }],
                    spans,
                },
            );
            return;
        }
        let (tag, leaf) = code::classify_code(code);
        if code.starts_with("Rd ") {
            unsupported::diagnose_code_range(
                self,
                DiagnosticCode::UnsupportedInlineR,
                "unsupported inline R code: evaluation is not supported",
                "unsupported inline R code",
                range.start,
                range.end,
            );
        }
        let spans = self.spans(range.start, range.end);
        let anchor = self.anchor(range.start);
        let frame = self.frames.last_mut().expect("the root frame exists");
        separator::materialize_separator(frame, anchor);
        frame::append_node(
            frame,
            NodeWithOrigin {
                node: RdNode::tagged(tag, None, vec![leaf.clone()]),
                children: vec![NodeWithOrigin {
                    node: leaf,
                    children: Vec::new(),
                    spans: spans.clone(),
                }],
                spans,
            },
        );
    }

    fn record_boundary(&mut self, boundary: PendingSeparator) {
        let frame = self.frames.last_mut().expect("the root frame exists");
        frame.pending_separator = frame.pending_separator.max(boundary);
    }

    fn unsupported_leaf(&mut self, name: &str, range: Range<usize>) {
        unsupported::diagnose_range(self, name, range.start, range.end);
    }

    fn in_html_block(&self) -> bool {
        self.frames.last().is_some_and(|frame| {
            matches!(
                &frame.kind,
                FrameKind::Unsupported { name, .. } if name == "HTML block"
            )
        })
    }

    fn prepare_block_start(&mut self, tag: &Tag<'_>, offset: usize) {
        let anchor = self.anchor(offset);
        let Some(frame) = self.frames.last_mut() else {
            return;
        };
        if !matches!(frame.kind, FrameKind::Subsection { .. }) {
            return;
        }
        if frame.pending_separator == PendingSeparator::Section {
            separator::materialize_separator(frame, anchor);
        }
        if frame.nodes.is_empty()
            && frame.pending.text.is_empty()
            && frame.pending_separator == PendingSeparator::Line
        {
            let leading = match tag {
                Tag::Paragraph => "\n\n",
                Tag::CodeBlock(_)
                | Tag::List(_)
                | Tag::BlockQuote(_)
                | Tag::HtmlBlock
                | Tag::Table(_) => "\n",
                _ => return,
            };
            frame.pending_separator = PendingSeparator::None;
            frame.pending.text.push_str(leading);
            if let Some(anchor) = anchor {
                frame.pending.spans.push(anchor);
            }
        }
    }

    fn open_heading(&mut self, offset: usize) {
        let nested = self
            .frames
            .last()
            .is_some_and(|frame| matches!(frame.kind, FrameKind::Subsection { .. }));
        let anchor = self.anchor(offset);
        let parent = self.frames.last_mut().expect("the root frame exists");
        if parent.pending_separator == PendingSeparator::Section {
            separator::materialize_separator(parent, anchor);
        } else {
            parent.pending_separator = PendingSeparator::None;
        }
        if nested || !parent.pending.text.is_empty() || !parent.nodes.is_empty() {
            parent.pending.text.push('\n');
            if let Some(anchor) = anchor {
                parent.pending.spans.push(anchor);
            }
        }
    }

    fn close_sections(&mut self, minimum_level: usize, end: usize) {
        loop {
            let close = self.frames.last().is_some_and(|frame| {
                matches!(
                    frame.kind,
                    FrameKind::Subsection { level, .. } if level >= minimum_level
                )
            });
            if !close {
                break;
            }
            let frame = self.frames.pop().expect("the subsection frame exists");
            let frame::FinishedFrame::Nodes(nodes) = frame::finish_frame(self, frame, end) else {
                unreachable!("a subsection frame produces nodes")
            };
            let anchor = self.anchor(end);
            let parent = self.frames.last_mut().expect("the root frame exists");
            for node in nodes {
                separator::append_flattened_node(parent, node, anchor);
            }
            parent.pending_separator = PendingSeparator::Section;
        }
    }

    fn spans(&self, start: usize, end: usize) -> Vec<Span> {
        let start = u32::try_from(start).expect("Markdown offset fits u32");
        let end = u32::try_from(end).expect("Markdown offset fits u32");
        self.value.source_spans(TextRange::new(start, end))
    }

    fn anchor(&self, offset: usize) -> Option<Span> {
        self.value
            .source_anchor_at(u32::try_from(offset).expect("Markdown offset fits u32"))
    }

    fn finish(mut self) -> MarkdownConversion {
        let end = self.value.as_str().len();
        self.close_sections(0, end);
        let root = self.frames.pop().expect("the root frame exists");
        let frame::FinishedFrame::Nodes(nodes) = frame::finish_frame(&mut self, root, end) else {
            unreachable!("the root frame cannot finish as a table child")
        };
        let mut fragment = LatexFragment {
            nodes: Vec::new(),
            origins: Vec::new(),
        };
        for (index, node) in nodes.into_iter().enumerate() {
            fragment::flatten_node(node, FragmentPath::root(index), &mut fragment);
        }
        #[cfg(debug_assertions)]
        assert_fragment_writer_valid(&fragment.nodes);
        MarkdownConversion {
            fragment,
            diagnostics: self.diagnostics,
        }
    }
}

#[cfg(debug_assertions)]
fn assert_fragment_writer_valid(nodes: &[RdNode]) {
    if nodes.is_empty() {
        return;
    }
    let description = RdNode::tagged(RdTag::Description, None, nodes.to_vec());
    let document = rd_ast::RdDocument::new(vec![description]);
    if let Err(error) = rd_writer::write_document(&document) {
        panic!("fragment nodes {:?} are not writer-valid: {error}", nodes);
    }
}

#[cfg(test)]
mod tests;
