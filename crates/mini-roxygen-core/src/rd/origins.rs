//! Source origins for the identity-based Rd builder.

use std::collections::{BTreeMap, BTreeSet};

use rd_ast::{RdDocument, RdNode, RdPath, RdPathSegment, RdTag};

use crate::inherit::{DocumentationOrigin, ResolvedContent};
use crate::source::Span;
use crate::tags::TagOrigin;

/// An edge that addresses another AST node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum OriginPathSegment {
    /// An element in a node's ordinary children.
    Child(usize),
    /// The option sequence of a tagged node.
    Option,
}

/// Stable identity for a node in an [`OriginBuilder`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct NodeId(usize);

#[derive(Debug)]
struct ArenaNode {
    kind: ArenaNodeKind,
    children: Vec<NodeId>,
    option: Option<Vec<NodeId>>,
    spans: Vec<Span>,
}

#[derive(Debug)]
enum ArenaNodeKind {
    Text(String),
    RCode(String),
    Verb(String),
    Comment(String),
    Tagged(RdTag),
    Group,
    Raw {
        tag: Option<String>,
        payload: Option<rd_ast::RawRdValue>,
        attributes: Vec<rd_ast::RdAttribute>,
    },
}

/// A document assembled while retaining source origins independently of tree
/// positions. Nodes can be inserted before one another without changing their
/// identities.
#[derive(Debug, Default)]
pub(crate) struct OriginBuilder {
    arena: Vec<ArenaNode>,
    roots: Vec<NodeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LeafKind {
    Text,
    RCode,
    Verb,
}

impl OriginBuilder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Imports an already-shaped AST node and returns the identity of its root.
    pub(crate) fn append_node(&mut self, node: RdNode) -> NodeId {
        let id = self.import_node(&node);
        self.roots.push(id);
        id
    }

    pub(crate) fn detached_node(&mut self, node: RdNode) -> NodeId {
        self.import_node(&node)
    }

    pub(crate) fn append_nodes(&mut self, nodes: impl IntoIterator<Item = RdNode>) -> Vec<NodeId> {
        nodes
            .into_iter()
            .map(|node| self.append_node(node))
            .collect()
    }

    pub(crate) fn append_fragment(
        &mut self,
        fragment: &crate::markdown_conversion::LatexFragment,
    ) -> Vec<NodeId> {
        let roots = fragment
            .nodes
            .iter()
            .map(|node| self.import_node(node))
            .collect::<Vec<_>>();
        for origin in &fragment.origins {
            if let Some(id) = self.fragment_node(&roots, origin.path.segments()) {
                self.record(id, &origin.spans);
            }
        }
        roots
    }

    pub(crate) fn append_text(&mut self, value: impl Into<String>) -> NodeId {
        self.append_node(RdNode::Text(value.into()))
    }

    pub(crate) fn text_child(&mut self, value: impl Into<String>) -> NodeId {
        self.import_node(&RdNode::Text(value.into()))
    }

    pub(crate) fn rcode_child(&mut self, value: impl Into<String>) -> NodeId {
        self.import_node(&RdNode::RCode(value.into()))
    }

    pub(crate) fn verb_child(&mut self, value: impl Into<String>) -> NodeId {
        self.import_node(&RdNode::Verb(value.into()))
    }

    pub(crate) fn leaf_child(&mut self, kind: LeafKind, value: impl Into<String>) -> NodeId {
        let value = value.into();
        match kind {
            LeafKind::Text => self.text_child(value),
            LeafKind::RCode => self.rcode_child(value),
            LeafKind::Verb => self.verb_child(value),
        }
    }

    pub(crate) fn leaf_matches(&self, id: NodeId, kind: LeafKind) -> bool {
        matches!(
            (&self.arena[id.0].kind, kind),
            (ArenaNodeKind::Text(_), LeafKind::Text)
                | (ArenaNodeKind::RCode(_), LeafKind::RCode)
                | (ArenaNodeKind::Verb(_), LeafKind::Verb)
        )
    }

    pub(crate) fn leaf_ends_with_newline(&self, id: NodeId) -> bool {
        match &self.arena[id.0].kind {
            ArenaNodeKind::Text(value)
            | ArenaNodeKind::RCode(value)
            | ArenaNodeKind::Verb(value) => value.ends_with('\n'),
            _ => false,
        }
    }

    pub(crate) fn leaf_is_empty(&self, id: NodeId) -> bool {
        matches!(
            &self.arena[id.0].kind,
            ArenaNodeKind::Text(value)
                | ArenaNodeKind::RCode(value)
                | ArenaNodeKind::Verb(value)
                if value.is_empty()
        )
    }

    pub(crate) fn extend_leaf(&mut self, id: NodeId, suffix: &str) {
        match &mut self.arena[id.0].kind {
            ArenaNodeKind::Text(value)
            | ArenaNodeKind::RCode(value)
            | ArenaNodeKind::Verb(value) => value.push_str(suffix),
            _ => unreachable!("only leaves can be extended"),
        }
    }

    pub(crate) fn prepend_leaf(&mut self, id: NodeId, prefix: &str) {
        match &mut self.arena[id.0].kind {
            ArenaNodeKind::Text(value)
            | ArenaNodeKind::RCode(value)
            | ArenaNodeKind::Verb(value) => value.insert_str(0, prefix),
            _ => panic!("prepend_leaf requires a leaf"),
        }
    }

    pub(crate) fn trim_leaf_edges(&mut self, id: NodeId, trim_start: bool, trim_end: bool) {
        let value = match &mut self.arena[id.0].kind {
            ArenaNodeKind::Text(value)
            | ArenaNodeKind::RCode(value)
            | ArenaNodeKind::Verb(value) => value,
            _ => return,
        };
        // R's default trimws() removes only these four characters; keep this
        // narrower than Rust's Unicode-wide is_whitespace for byte fidelity.
        let is_r_trimws_whitespace =
            |character: char| matches!(character, ' ' | '\t' | '\r' | '\n');
        let start = if trim_start {
            value
                .char_indices()
                .find_map(|(index, character)| {
                    (!is_r_trimws_whitespace(character)).then_some(index)
                })
                .unwrap_or(value.len())
        } else {
            0
        };
        let end = if trim_end {
            value
                .char_indices()
                .rev()
                .find_map(|(index, character)| {
                    (!is_r_trimws_whitespace(character)).then_some(index + character.len_utf8())
                })
                .unwrap_or(start)
        } else {
            value.len()
        };
        value.replace_range(end.., "");
        value.replace_range(..start.min(end), "");
    }

    /// Creates a composite node without making it a document root. This is
    /// used for section children and list items assembled from detached nodes.
    pub(crate) fn tagged_child(
        &mut self,
        tag: RdTag,
        option: Option<Vec<NodeId>>,
        children: Vec<NodeId>,
    ) -> NodeId {
        self.allocate_kind(ArenaNodeKind::Tagged(tag), children, option, Vec::new())
    }

    pub(crate) fn group_child(&mut self, children: Vec<NodeId>) -> NodeId {
        self.allocate_kind(ArenaNodeKind::Group, children, None, Vec::new())
    }

    pub(crate) fn add_root(&mut self, id: NodeId) {
        self.roots.push(id);
    }

    pub(crate) fn record(&mut self, id: NodeId, spans: &[Span]) {
        if let Some(node) = self.arena.get_mut(id.0) {
            node.spans.extend_from_slice(spans);
        }
    }

    #[cfg(test)]
    pub(crate) fn insert_root_before(&mut self, target: NodeId, node: RdNode) -> NodeId {
        let id = self.import_node(&node);
        let position = self
            .roots
            .iter()
            .position(|root| *root == target)
            .expect("target must be a document root");
        self.roots.insert(position, id);
        id
    }

    pub(crate) fn materialize(&self) -> (RdDocument, OriginMap) {
        let mut materialized = vec![None; self.arena.len()];
        let document = RdDocument::from(
            self.roots
                .iter()
                .map(|id| self.materialize_node(*id, &mut materialized))
                .collect::<Vec<_>>(),
        );
        let mut paths = BTreeMap::new();
        let mut option_paths = BTreeSet::new();
        for (index, id) in self.roots.iter().copied().enumerate() {
            let mut prefix = vec![OriginPathSegment::Child(index)];
            self.compile_paths(id, &mut prefix, &mut paths, &mut option_paths);
        }
        let spans = self
            .arena
            .iter()
            .enumerate()
            .map(|(index, node)| (NodeId(index), node.spans.clone()))
            .collect();
        (
            document,
            OriginMap {
                paths,
                option_paths,
                spans,
                #[cfg(test)]
                nodes: self
                    .arena
                    .iter()
                    .enumerate()
                    .map(|(index, _)| {
                        (
                            NodeId(index),
                            self.materialize_node(NodeId(index), &mut materialized),
                        )
                    })
                    .collect(),
            },
        )
    }

    fn import_node(&mut self, node: &RdNode) -> NodeId {
        let children = match node {
            RdNode::Tagged(tagged) => tagged
                .children()
                .iter()
                .map(|child| self.import_node(child))
                .collect(),
            RdNode::Group(group) => group
                .children()
                .iter()
                .map(|child| self.import_node(child))
                .collect(),
            RdNode::Raw(raw) => raw
                .children()
                .iter()
                .map(|child| self.import_node(child))
                .collect(),
            _ => Vec::new(),
        };
        let option = match node {
            RdNode::Tagged(tagged) => tagged
                .option()
                .map(|nodes| nodes.iter().map(|child| self.import_node(child)).collect()),
            RdNode::Raw(raw) => raw
                .option()
                .map(|nodes| nodes.iter().map(|child| self.import_node(child)).collect()),
            _ => None,
        };
        self.allocate_kind(arena_kind(node), children, option, Vec::new())
    }

    fn allocate_kind(
        &mut self,
        kind: ArenaNodeKind,
        children: Vec<NodeId>,
        option: Option<Vec<NodeId>>,
        spans: Vec<Span>,
    ) -> NodeId {
        let id = NodeId(self.arena.len());
        self.arena.push(ArenaNode {
            kind,
            children,
            option,
            spans,
        });
        id
    }

    fn materialize_node(&self, id: NodeId, materialized: &mut [Option<RdNode>]) -> RdNode {
        if let Some(node) = &materialized[id.0] {
            return node.clone();
        }
        let arena_node = &self.arena[id.0];
        let children = arena_node
            .children
            .iter()
            .map(|child| self.materialize_node(*child, materialized))
            .collect();
        let option = arena_node.option.as_ref().map(|nodes| {
            nodes
                .iter()
                .map(|child| self.materialize_node(*child, materialized))
                .collect()
        });
        let node = match &arena_node.kind {
            ArenaNodeKind::Text(value) => RdNode::Text(value.clone()),
            ArenaNodeKind::RCode(value) => RdNode::RCode(value.clone()),
            ArenaNodeKind::Verb(value) => RdNode::Verb(value.clone()),
            ArenaNodeKind::Comment(value) => RdNode::Comment(value.clone()),
            ArenaNodeKind::Tagged(tag) => RdNode::tagged(tag.clone(), option, children),
            ArenaNodeKind::Group => RdNode::group(children),
            ArenaNodeKind::Raw {
                tag,
                payload,
                attributes,
            } => RdNode::Raw(rd_ast::producer::raw_node(
                tag.clone(),
                option,
                children,
                payload.clone(),
                attributes.clone(),
            )),
        };
        materialized[id.0] = Some(node.clone());
        node
    }

    fn fragment_node(
        &self,
        roots: &[NodeId],
        segments: &[crate::markdown_conversion::FragmentPathSegment],
    ) -> Option<NodeId> {
        let (first, rest) = segments.split_first()?;
        let mut current = match first {
            crate::markdown_conversion::FragmentPathSegment::Child(index) => {
                roots.get(*index).copied()?
            }
            crate::markdown_conversion::FragmentPathSegment::Option => return None,
        };
        let mut segments = rest.iter();
        while let Some(segment) = segments.next() {
            current = match segment {
                crate::markdown_conversion::FragmentPathSegment::Child(index) => {
                    self.arena[current.0].children.get(*index).copied()?
                }
                crate::markdown_conversion::FragmentPathSegment::Option => {
                    let crate::markdown_conversion::FragmentPathSegment::Child(index) =
                        segments.next()?
                    else {
                        return None;
                    };
                    self.arena[current.0]
                        .option
                        .as_ref()?
                        .get(*index)
                        .copied()?
                }
            };
        }
        Some(current)
    }

    fn compile_paths(
        &self,
        id: NodeId,
        prefix: &mut Vec<OriginPathSegment>,
        paths: &mut BTreeMap<Vec<OriginPathSegment>, NodeId>,
        option_paths: &mut BTreeSet<Vec<OriginPathSegment>>,
    ) {
        let node = &self.arena[id.0];
        paths.insert(prefix.clone(), id);
        for (index, child) in node.children.iter().copied().enumerate() {
            let prefix_len = prefix.len();
            prefix.push(OriginPathSegment::Child(index));
            self.compile_paths(child, prefix, paths, option_paths);
            prefix.truncate(prefix_len);
        }
        if let Some(option) = &node.option {
            let prefix_len = prefix.len();
            prefix.push(OriginPathSegment::Option);
            option_paths.insert(prefix.clone());
            for (index, child) in option.iter().copied().enumerate() {
                let child_prefix_len = prefix.len();
                prefix.push(OriginPathSegment::Child(index));
                self.compile_paths(child, prefix, paths, option_paths);
                prefix.truncate(child_prefix_len);
            }
            prefix.truncate(prefix_len);
        }
    }
}

fn arena_kind(node: &RdNode) -> ArenaNodeKind {
    match node {
        RdNode::Text(value) => ArenaNodeKind::Text(value.clone()),
        RdNode::RCode(value) => ArenaNodeKind::RCode(value.clone()),
        RdNode::Verb(value) => ArenaNodeKind::Verb(value.clone()),
        RdNode::Comment(value) => ArenaNodeKind::Comment(value.clone()),
        RdNode::Tagged(tagged) => ArenaNodeKind::Tagged(tagged.tag().clone()),
        RdNode::Group(_) => ArenaNodeKind::Group,
        RdNode::Raw(raw) => ArenaNodeKind::Raw {
            tag: raw.tag().map(str::to_owned),
            payload: raw.payload().cloned(),
            attributes: raw.attributes().to_vec(),
        },
        _ => unreachable!("unknown Rd node variant"),
    }
}

/// The canonical-path-to-node identity map, with roots represented internally
/// as [`OriginPathSegment::Child`] edges.
#[derive(Debug)]
pub(crate) struct OriginMap {
    paths: BTreeMap<Vec<OriginPathSegment>, NodeId>,
    /// Bare option paths are valid writer anchors, but the option container
    /// is not an AST node and therefore is intentionally absent from `paths`.
    option_paths: BTreeSet<Vec<OriginPathSegment>>,
    spans: BTreeMap<NodeId, Vec<Span>>,
    #[cfg(test)]
    nodes: BTreeMap<NodeId, RdNode>,
}

/// Finds the nearest source origin for a canonical writer path. Node edges are
/// normalized into the internal path representation; known non-node suffixes
/// terminate that lookup, while malformed or unknown segments are rejected.
pub(crate) fn span_for_path(map: &OriginMap, path: &RdPath) -> Option<Span> {
    let mut normalized = Vec::new();
    let mut terminal = false;
    let segments = path.segments();
    let mut position = 0;
    while position < segments.len() {
        let segment = &segments[position];
        match segment {
            RdPathSegment::TopLevel(index) if position == 0 => {
                if terminal {
                    return None;
                }
                normalized.push(OriginPathSegment::Child(*index));
            }
            RdPathSegment::Child(index) if position > 0 => {
                if terminal {
                    return None;
                }
                normalized.push(OriginPathSegment::Child(*index));
            }
            RdPathSegment::Option => {
                if terminal
                    || !matches!(
                        segments.get(position + 1),
                        None | Some(RdPathSegment::Child(_))
                    )
                {
                    return None;
                }
                normalized.push(OriginPathSegment::Option);
            }
            RdPathSegment::Attribute(_)
            | RdPathSegment::AttributeValue
            | RdPathSegment::ListElement(_)
            | RdPathSegment::CharacterElement(_) => terminal = true,
            _ => return None,
        }
        position += 1;
    }
    let bare_option = normalized.last() == Some(&OriginPathSegment::Option)
        && map.option_paths.contains(&normalized);
    if !bare_option && !map.paths.contains_key(&normalized) {
        return None;
    }
    if bare_option {
        normalized.pop();
    }
    loop {
        if let Some(id) = map.paths.get(&normalized)
            && let Some(span) = map.spans.get(id).and_then(|spans| spans.first())
        {
            return Some(*span);
        }
        if normalized.is_empty() {
            return None;
        }
        normalized.pop();
    }
}

pub(crate) fn tag_origin_spans(origin: &TagOrigin) -> Vec<Span> {
    match origin {
        TagOrigin::Explicit {
            name,
            value_span,
            full_span,
        } => vec![name.span, *value_span, *full_span],
        TagOrigin::Implicit { intro_span } => vec![*intro_span],
    }
}

pub(crate) fn content_spans(content: &ResolvedContent) -> Vec<Span> {
    match &content.provenance.source {
        DocumentationOrigin::Local(origin) => tag_origin_spans(origin),
        DocumentationOrigin::External { .. } => content
            .provenance
            .requests
            .last()
            .map_or_else(Vec::new, tag_origin_spans),
    }
}

/// Confirms that path compilation retained node identity, not merely a valid
/// positional address in the final document.
#[cfg(test)]
pub(crate) fn assert_paths_address_nodes(map: &OriginMap, document: &RdDocument) {
    for (path, id) in &map.paths {
        let node = resolve_document_path(document, path)
            .unwrap_or_else(|| panic!("origin path {path:?} has no document node"));
        assert_eq!(
            node,
            map.nodes.get(id).expect("node identity is materialized")
        );
    }
}

#[cfg(test)]
fn resolve_document_path<'a>(
    document: &'a RdDocument,
    path: &[OriginPathSegment],
) -> Option<&'a RdNode> {
    let first = path.first()?;
    let mut node = match first {
        OriginPathSegment::Child(index) => document.nodes().get(*index)?,
        OriginPathSegment::Option => return None,
    };
    let mut index = 1;
    while index < path.len() {
        let segment = &path[index];
        node = match segment {
            OriginPathSegment::Child(index) => match node {
                RdNode::Tagged(tagged) => tagged.children().get(*index)?,
                RdNode::Group(group) => group.children().get(*index)?,
                RdNode::Raw(raw) => raw.children().get(*index)?,
                _ => return None,
            },
            OriginPathSegment::Option => {
                let Some(OriginPathSegment::Child(index)) = path.get(index + 1) else {
                    return None;
                };
                match node {
                    RdNode::Tagged(tagged) => tagged.option()?.get(*index)?,
                    RdNode::Raw(raw) => raw.option()?.get(*index)?,
                    _ => return None,
                }
            }
        };
        if matches!(segment, OriginPathSegment::Option) {
            index += 2;
        } else {
            index += 1;
        }
    }
    Some(node)
}

#[cfg(test)]
mod tests;
