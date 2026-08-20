//! Mutable conversion frames and helpers for accumulating output and spans.

use pulldown_cmark::Alignment;
use rd_ast::{RdNode, RdTag};

use crate::diagnostic::DiagnosticCode;
use crate::source::Span;

use super::separator::PendingSeparator;

#[derive(Debug)]
pub(crate) struct Frame {
    pub(crate) kind: FrameKind,
    pub(crate) pending: PendingText,
    pub(crate) nodes: Vec<NodeWithOrigin>,
    pub(crate) pending_separator: PendingSeparator,
}

impl Frame {
    pub(crate) fn root() -> Self {
        Self::new(FrameKind::Root)
    }

    pub(crate) fn new(kind: FrameKind) -> Self {
        Self {
            kind,
            pending: PendingText::default(),
            nodes: Vec::new(),
            pending_separator: PendingSeparator::None,
        }
    }

    pub(crate) fn subsection(level: usize, start: usize, title: Vec<NodeWithOrigin>) -> Self {
        let mut frame = Self::new(FrameKind::Subsection {
            level,
            start,
            title,
        });
        frame.pending_separator = PendingSeparator::Line;
        frame
    }
}

#[derive(Debug)]
pub(crate) enum FrameKind {
    Root,
    Paragraph,
    Heading {
        level: usize,
        start: usize,
    },
    Subsection {
        level: usize,
        start: usize,
        title: Vec<NodeWithOrigin>,
    },
    Item,
    List {
        tag: RdTag,
        start: usize,
    },
    Table {
        alignments: Vec<Alignment>,
        rows: Vec<Vec<NodeWithOrigin>>,
        start: usize,
    },
    TableRow {
        cells: Vec<super::table::CompletedCell>,
        width: usize,
        start: usize,
    },
    TableCell,
    CodeBlock {
        executable_r: bool,
        start: usize,
    },
    Link {
        destination: String,
        start: usize,
    },
    Tagged {
        tag: RdTag,
        start: usize,
    },
    Unsupported {
        name: String,
        start: usize,
    },
}

#[derive(Debug, Default)]
pub(crate) struct PendingText {
    pub(crate) text: String,
    pub(crate) spans: Vec<Span>,
}

#[derive(Debug)]
pub(crate) struct NodeWithOrigin {
    pub(crate) node: RdNode,
    pub(crate) children: Vec<NodeWithOrigin>,
    pub(crate) spans: Vec<Span>,
}

pub(super) enum FinishedFrame {
    Nodes(Vec<NodeWithOrigin>),
    Cell(super::table::CompletedCell),
    Row(Vec<NodeWithOrigin>),
}

/// Flushes pending text into physical-line leaves.
///
/// Each leaf receives the complete pending span list. `FragmentOrigin`
/// describes the Markdown event envelope that produced a node, not a
/// character-level inverse map; the buffer also contains separators generated
/// by this layer, so per-line attribution is not well-defined. An origin on
/// only the first leaf would make later siblings fall back through
/// `span_for_path` to the section or topic anchor, which is less honest than a
/// slightly wide span.
pub(crate) fn flush_pending(frame: &mut Frame) {
    // A separator pending at the end of a frame has nothing to separate and is
    // intentionally discarded.
    frame.pending_separator = PendingSeparator::None;
    if frame.pending.text.is_empty() {
        return;
    }
    let pending = std::mem::take(&mut frame.pending);
    frame.nodes.extend(
        super::leaf::physical_line_chunks(&pending.text).map(|line| NodeWithOrigin {
            node: RdNode::Text(line.to_owned()),
            children: Vec::new(),
            spans: pending.spans.clone(),
        }),
    );
}

pub(crate) fn append_node(frame: &mut Frame, node: NodeWithOrigin) {
    flush_pending(frame);
    frame.nodes.push(node);
}

pub(crate) fn node_with_origin(node: RdNode, spans: Vec<Span>) -> NodeWithOrigin {
    let children = match &node {
        RdNode::Tagged(tagged) => tagged
            .children()
            .iter()
            .cloned()
            .map(|child| node_with_origin(child, spans.clone()))
            .collect(),
        RdNode::Group(group) => group
            .children()
            .iter()
            .cloned()
            .map(|child| node_with_origin(child, spans.clone()))
            .collect(),
        _ => Vec::new(),
    };
    NodeWithOrigin {
        node,
        children,
        spans,
    }
}

pub(crate) fn append_spans(target: &mut Vec<Span>, spans: impl IntoIterator<Item = Span>) {
    for span in spans {
        let merge = !span.range.is_empty()
            && target.last().is_some_and(|previous| {
                !previous.range.is_empty()
                    && previous.file == span.file
                    && previous.range.end() == span.range.start()
            });
        if merge {
            let previous = target.last_mut().expect("merge requires a previous span");
            previous.range =
                crate::source::TextRange::new(previous.range.start(), span.range.end());
        } else {
            target.push(span);
        }
    }
}

pub(super) fn finish_frame(
    converter: &mut super::Converter<'_>,
    mut frame: Frame,
    end: usize,
) -> FinishedFrame {
    if let FrameKind::TableCell = frame.kind {
        flush_pending(&mut frame);
        return FinishedFrame::Cell(super::table::CompletedCell {
            nodes: std::mem::take(&mut frame.nodes),
            anchor: converter.anchor(end),
        });
    }
    if matches!(frame.kind, FrameKind::TableRow { .. }) {
        flush_pending(&mut frame);
        let FrameKind::TableRow {
            cells,
            width,
            start,
        } = frame.kind
        else {
            unreachable!()
        };
        return FinishedFrame::Row(super::table::lower_row(converter, cells, width, start, end));
    }
    if let FrameKind::Table {
        alignments,
        rows,
        start,
    } = frame.kind
    {
        return FinishedFrame::Nodes(vec![super::table::lower_table(
            converter, alignments, rows, start, end,
        )]);
    }
    if let FrameKind::CodeBlock {
        executable_r,
        start,
    } = frame.kind
    {
        let mut body = std::mem::take(&mut frame.pending.text);
        let body_spans = std::mem::take(&mut frame.pending.spans);
        if body.is_empty() {
            // The writer rejects an empty verbatim leaf. pulldown-cmark
            // normally supplies a trailing newline, including for an
            // empty block; retain a serializable fallback defensively.
            body.push('\n');
        }
        if executable_r {
            super::unsupported::diagnose_code_range(
                converter,
                DiagnosticCode::UnsupportedInlineR,
                "unsupported executable R Markdown code block: evaluation is not supported",
                "unsupported executable R Markdown code block",
                start,
                end,
            );
        }
        // rd-source represents each physical verbatim line as a separate
        // leaf: a single multi-line leaf either fails to serialize or
        // fails the writer's own reparse check. Splitting at retained
        // newlines keeps the AST round-trippable while preserving the
        // exact bytes.
        //
        // Every line carries the whole block's spans rather than its own.
        // Provenance inside a code block is therefore block-level, not
        // per-line; a consumer inverting these origins must not read a
        // line leaf as evidence that a particular source line produced it.
        let body_nodes = super::leaf::physical_line_chunks(&body)
            .map(|line| NodeWithOrigin {
                node: RdNode::Verb(line.to_owned()),
                children: Vec::new(),
                spans: body_spans.clone(),
            })
            .collect::<Vec<_>>();
        return FinishedFrame::Nodes(vec![NodeWithOrigin {
            node: RdNode::tagged(
                RdTag::Preformatted,
                None,
                body_nodes.iter().map(|node| node.node.clone()).collect(),
            ),
            children: body_nodes,
            spans: converter.spans(start, end),
        }]);
    }
    if matches!(frame.kind, FrameKind::Subsection { .. }) {
        let start = match &frame.kind {
            FrameKind::Subsection { start, .. } => *start,
            _ => unreachable!(),
        };
        if frame.pending_separator == PendingSeparator::Section {
            super::separator::materialize_separator(&mut frame, converter.anchor(end));
        }
        flush_pending(&mut frame);
        let close_span = converter.anchor(end);
        if let Some(NodeWithOrigin {
            node: RdNode::Text(text),
            spans,
            ..
        }) = frame.nodes.last_mut()
            && !text.ends_with('\n')
        {
            text.push('\n');
            if let Some(close_span) = close_span {
                append_spans(spans, [close_span]);
            }
        } else {
            frame.nodes.push(NodeWithOrigin {
                node: RdNode::Text("\n".to_owned()),
                children: Vec::new(),
                spans: close_span.into_iter().collect(),
            });
        }

        let FrameKind::Subsection { title, .. } = frame.kind else {
            unreachable!()
        };
        let title_nodes = title.iter().map(|node| node.node.clone()).collect();
        let body_nodes = frame.nodes;
        let spans = converter.spans(start, end);
        let title = NodeWithOrigin {
            node: RdNode::group(title_nodes),
            children: title,
            spans: spans.clone(),
        };
        let body = NodeWithOrigin {
            node: RdNode::group(body_nodes.iter().map(|node| node.node.clone()).collect()),
            children: body_nodes,
            spans: spans.clone(),
        };
        return FinishedFrame::Nodes(vec![NodeWithOrigin {
            node: RdNode::tagged(
                RdTag::Subsection,
                None,
                vec![title.node.clone(), body.node.clone()],
            ),
            children: vec![title, body],
            spans,
        }]);
    }
    if let FrameKind::Link {
        ref destination,
        start,
    } = frame.kind
    {
        let destination = destination.clone();
        flush_pending(&mut frame);
        let children = std::mem::take(&mut frame.nodes);
        return FinishedFrame::Nodes(super::link::lower_link(
            converter,
            &destination,
            start,
            end,
            children,
        ));
    }
    if matches!(frame.kind, FrameKind::List { .. }) {
        super::separator::materialize_separator(&mut frame, converter.anchor(end));
    }
    flush_pending(&mut frame);
    let children = std::mem::take(&mut frame.nodes);
    let nodes = match frame.kind {
        FrameKind::Root
        | FrameKind::Paragraph
        | FrameKind::Heading { .. }
        | FrameKind::Item
        | FrameKind::Unsupported { .. } => children,
        FrameKind::Subsection { .. } => unreachable!("subsections are handled above"),
        FrameKind::Table { .. } | FrameKind::TableRow { .. } | FrameKind::TableCell => {
            unreachable!("table frames are handled above")
        }
        FrameKind::CodeBlock { .. } => unreachable!("code blocks are handled above"),
        FrameKind::Link { .. } => unreachable!("links are handled above"),
        FrameKind::List { tag, start } => {
            vec![super::list::lower_list(
                tag,
                children,
                converter.spans(start, end),
            )]
        }
        FrameKind::Tagged { tag, start } => {
            let child_nodes = children.iter().map(|child| child.node.clone()).collect();
            vec![NodeWithOrigin {
                node: RdNode::tagged(tag, None, child_nodes),
                children,
                spans: converter.spans(start, end),
            }]
        }
    };
    FinishedFrame::Nodes(nodes)
}
