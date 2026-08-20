//! Fragment nodes, source origins, and flattening into fragment entries.

use rd_ast::RdNode;

use crate::source::Span;

use super::frame::NodeWithOrigin;

/// A sequence of Rd nodes that can later be spliced into a topic.
#[derive(Debug, Clone)]
pub(crate) struct LatexFragment {
    /// The real Rd nodes in fragment-root order.
    pub(crate) nodes: Vec<RdNode>,
    /// Event-envelope origins for nodes at every depth.
    pub(crate) origins: Vec<FragmentOrigin>,
}

/// Source provenance for one node in a [`LatexFragment`].
#[derive(Debug, Clone)]
pub(crate) struct FragmentOrigin {
    /// Relative to [`LatexFragment::nodes`].
    pub(crate) path: FragmentPath,
    /// Source spans represented by the Markdown event envelope. A normalized
    /// Markdown range can cross several physical roxygen lines.
    /// These spans describe which source bytes produced the node; they do not
    /// map individual output characters back to source characters.
    pub(crate) spans: Vec<Span>,
}

/// An edge in a fragment-origin path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FragmentPathSegment {
    /// An ordinary child node.
    Child(usize),
    /// A tagged node's option sequence.
    Option,
}

/// A path from a fragment root through tagged-node children and options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FragmentPath {
    segments: Vec<FragmentPathSegment>,
}

impl FragmentPath {
    pub(super) fn root(index: usize) -> Self {
        Self {
            segments: vec![FragmentPathSegment::Child(index)],
        }
    }

    fn child(&self, index: usize) -> Self {
        let mut segments = self.segments.clone();
        segments.push(FragmentPathSegment::Child(index));
        Self { segments }
    }

    fn option(&self) -> Self {
        let mut segments = self.segments.clone();
        segments.push(FragmentPathSegment::Option);
        Self { segments }
    }

    /// Returns the fragment-relative node edges.
    pub(crate) fn segments(&self) -> &[FragmentPathSegment] {
        &self.segments
    }
}

pub(super) fn flatten_node(node: NodeWithOrigin, path: FragmentPath, fragment: &mut LatexFragment) {
    let child_nodes = node.children;
    let node_value = node.node;
    let spans = node.spans;
    let child_paths = child_nodes
        .iter()
        .enumerate()
        .map(|(index, _)| path.child(index))
        .collect::<Vec<_>>();
    fragment.nodes.push(node_value.clone());
    fragment.origins.push(FragmentOrigin {
        path: path.clone(),
        spans: spans.clone(),
    });
    for (child, child_path) in child_nodes.into_iter().zip(child_paths) {
        flatten_origin(child, child_path, fragment);
    }
    flatten_option_origins(&node_value, path, &spans, fragment);
}

fn flatten_origin(node: NodeWithOrigin, path: FragmentPath, fragment: &mut LatexFragment) {
    let child_nodes = node.children;
    let node_value = node.node;
    let spans = node.spans;
    let child_paths = child_nodes
        .iter()
        .enumerate()
        .map(|(index, _)| path.child(index))
        .collect::<Vec<_>>();
    fragment.origins.push(FragmentOrigin {
        path: path.clone(),
        spans: spans.clone(),
    });
    for (child, child_path) in child_nodes.into_iter().zip(child_paths) {
        flatten_origin(child, child_path, fragment);
    }
    // Option nodes are not part of NodeWithOrigin's ordinary child tree, but
    // they are still real Rd nodes and must retain the link's source envelope.
    flatten_option_origins(&node_value, path, &spans, fragment);
}

fn flatten_option_origins(
    node: &RdNode,
    path: FragmentPath,
    spans: &[Span],
    fragment: &mut LatexFragment,
) {
    let Some(tagged) = node.as_tagged() else {
        return;
    };
    let Some(option) = tagged.option() else {
        return;
    };
    let option_path = path.option();
    for (index, child) in option.iter().enumerate() {
        let child_path = option_path.child(index);
        fragment.origins.push(FragmentOrigin {
            path: child_path.clone(),
            spans: spans.to_vec(),
        });
        flatten_nested_option_origin(child, child_path, spans, fragment);
    }
}

fn flatten_nested_option_origin(
    node: &RdNode,
    path: FragmentPath,
    spans: &[Span],
    fragment: &mut LatexFragment,
) {
    let child_nodes = match node {
        RdNode::Tagged(tagged) => tagged.children(),
        RdNode::Group(group) => group.children(),
        _ => &[],
    };
    for (index, child) in child_nodes.iter().enumerate() {
        let child_path = path.child(index);
        fragment.origins.push(FragmentOrigin {
            path: child_path.clone(),
            spans: spans.to_vec(),
        });
        flatten_nested_option_origin(child, child_path, spans, fragment);
    }
}
