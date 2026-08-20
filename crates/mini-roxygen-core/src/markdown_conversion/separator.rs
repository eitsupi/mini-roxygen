//! Pending separators and recovery text materialization.

use pulldown_cmark::TagEnd;
use rd_ast::RdNode;

use crate::source::Span;

use super::frame::{Frame, NodeWithOrigin, append_node, append_spans};

/// Whitespace a closing construct owes the text that follows it.
///
/// This serves two purposes, and conflating them silently would be a trap, so
/// they are named here. For a **supported** construct it is real output: the
/// blank line between paragraphs and the newline after a list item body are
/// produced this way, and the serialization tests pin them. For an
/// **unsupported** construct it keeps the flattened recovery text readable
/// instead of running words together, and that spelling is not a
/// compatibility contract — those constructs become supported in later steps,
/// at which point their separators become real output like the rest.
///
/// The consequence is that changing a separator changes generated Rd, not
/// only diagnostics. Change one only with the serialization tests in view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum PendingSeparator {
    None,
    Line,
    Paragraph,
    /// Pending after a `\subsection` closes. It renders the same single
    /// newline as [`PendingSeparator::Line`] but is a distinct state on
    /// purpose: `Line` on a freshly opened subsection means "this subsection
    /// has no content yet, so give its first block the leading whitespace
    /// roxygen2 gives it", and reusing `Line` here would claim that about a
    /// subsection that has just been filled and closed.
    Section,
}

pub(crate) fn materialize_separator(frame: &mut Frame, anchor: Option<Span>) {
    let boundary = std::mem::replace(&mut frame.pending_separator, PendingSeparator::None);
    if boundary == PendingSeparator::None {
        return;
    }
    if frame.pending.text.is_empty() && frame.nodes.is_empty() {
        return;
    }
    let text = match boundary {
        PendingSeparator::None => unreachable!(),
        PendingSeparator::Line => "\n",
        PendingSeparator::Paragraph => "\n\n",
        PendingSeparator::Section => "\n",
    };
    frame.pending.text.push_str(text);
    if let Some(anchor) = anchor {
        frame.pending.spans.push(anchor);
    }
}

pub(crate) fn append_flattened_node(frame: &mut Frame, node: NodeWithOrigin, anchor: Option<Span>) {
    match node.node {
        RdNode::Text(text) => {
            if text.is_empty() {
                return;
            }
            materialize_separator(frame, anchor);
            frame.pending.text.push_str(&text);
            if frame.pending.spans != node.spans {
                append_spans(&mut frame.pending.spans, node.spans);
            }
        }
        _ => {
            materialize_separator(frame, anchor);
            append_node(
                frame,
                NodeWithOrigin {
                    node: node.node,
                    children: node.children,
                    spans: node.spans,
                },
            );
        }
    }
}

pub(crate) fn separator_after(tag: TagEnd) -> PendingSeparator {
    match tag {
        TagEnd::Paragraph
        | TagEnd::Heading(_)
        | TagEnd::BlockQuote(_)
        | TagEnd::CodeBlock
        | TagEnd::HtmlBlock
        | TagEnd::List(_)
        | TagEnd::Table
        | TagEnd::FootnoteDefinition
        | TagEnd::DefinitionList
        | TagEnd::MetadataBlock(_) => PendingSeparator::Paragraph,
        TagEnd::Item | TagEnd::DefinitionListTitle | TagEnd::DefinitionListDefinition => {
            PendingSeparator::Line
        }
        TagEnd::TableHead | TagEnd::TableRow | TagEnd::TableCell => PendingSeparator::None,
        TagEnd::Emphasis
        | TagEnd::Strong
        | TagEnd::Strikethrough
        | TagEnd::Superscript
        | TagEnd::Subscript
        | TagEnd::Link
        | TagEnd::Image => PendingSeparator::None,
    }
}
