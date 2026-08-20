//! Canonical AST shapes and whitespace for top-level Rd sections.

use rd_ast::RdTag;

use super::origins::{LeafKind, NodeId, OriginBuilder};

/// A plain `rd_macro()` uses the leaf mode required by the writer for the
/// macro's argument.
pub(crate) fn plain(
    builder: &mut OriginBuilder,
    tag: RdTag,
    mut content: Vec<NodeId>,
    kind: LeafKind,
) -> NodeId {
    trim_top_level(builder, &mut content, kind);
    builder.tagged_child(tag, None, content)
}

/// A spaced `rd_macro()` owns both wrapper newlines. Newlines are added as
/// separate leaves when the preceding leaf already ends a line, because the
/// writer treats an unterminated adjacent leaf of the same kind as a boundary
/// it cannot represent.
pub(crate) fn spaced(
    builder: &mut OriginBuilder,
    tag: RdTag,
    mut content: Vec<NodeId>,
    kind: LeafKind,
) -> NodeId {
    let mut children = vec![builder.leaf_child(kind, "\n")];
    children.append(&mut content);
    append_newlines(builder, &mut children, kind, 1);
    builder.tagged_child(tag, None, children)
}

/// Builds the two-argument `\section` shape. Its body has the same wrapper
/// newline contract as a spaced macro, while the topic supplies the newline
/// after the closing brace.
pub(crate) fn named_section(
    builder: &mut OriginBuilder,
    title: Vec<NodeId>,
    body: Vec<NodeId>,
) -> NodeId {
    let title = builder.group_child(title);
    // roxygen2 splits a named section after Markdown conversion, leaving the
    // separator space after `Title:` at the start of a non-empty body. Keep
    // that byte in the explicit AST. If prose starts with a Text leaf, fold
    // the space into that leaf because adjacent unterminated Text leaves are
    // not writer-representable; a structured first child can keep a separate
    // whitespace leaf after the wrapper newline.
    let body = if body.is_empty() {
        spaced_argument(builder, body, LeafKind::Text)
    } else {
        let mut children = vec![builder.text_child("\n")];
        if let Some(first) = body.first().copied()
            && builder.leaf_matches(first, LeafKind::Text)
        {
            builder.prepend_leaf(first, " ");
        } else {
            children.push(builder.text_child(" "));
        }
        children.extend(body);
        append_newlines(builder, &mut children, LeafKind::Text, 1);
        builder.group_child(children)
    };
    builder.tagged_child(RdTag::Section, None, vec![title, body])
}

/// Adds a separator to a sequence in the leaf mode required by its parent.
/// A non-terminated leaf is extended so the writer does not see two adjacent
/// same-kind leaves. A terminated leaf, and every non-leaf, gets new newline
/// leaves instead.
pub(crate) fn append_newlines(
    builder: &mut OriginBuilder,
    nodes: &mut Vec<NodeId>,
    kind: LeafKind,
    count: usize,
) {
    if count == 0 {
        return;
    }
    let mut extended = false;
    if let Some(last) = nodes.last().copied()
        && builder.leaf_matches(last, kind)
        && !builder.leaf_ends_with_newline(last)
    {
        builder.extend_leaf(last, "\n");
        extended = true;
        if count == 1 {
            return;
        }
    }
    let remaining = if extended {
        count.saturating_sub(1)
    } else {
        count
    };
    for _ in 0..remaining {
        nodes.push(builder.leaf_child(kind, "\n"));
    }
}

pub(crate) fn append_blank_line(
    builder: &mut OriginBuilder,
    nodes: &mut Vec<NodeId>,
    kind: LeafKind,
) {
    append_newlines(builder, nodes, kind, 2);
}

fn spaced_argument(
    builder: &mut OriginBuilder,
    mut content: Vec<NodeId>,
    kind: LeafKind,
) -> NodeId {
    let mut children = vec![builder.leaf_child(kind, "\n")];
    children.append(&mut content);
    append_newlines(builder, &mut children, kind, 1);
    builder.group_child(children)
}

pub(crate) fn trim_top_level(builder: &mut OriginBuilder, nodes: &mut Vec<NodeId>, kind: LeafKind) {
    while let Some(first) = nodes.first().copied() {
        if !builder.leaf_matches(first, kind) {
            break;
        }
        builder.trim_leaf_edges(first, true, false);
        if builder.leaf_is_empty(first) {
            nodes.remove(0);
        } else {
            break;
        }
    }
    while let Some(last) = nodes.last().copied() {
        if !builder.leaf_matches(last, kind) {
            break;
        }
        builder.trim_leaf_edges(last, false, true);
        if builder.leaf_is_empty(last) {
            nodes.pop();
        } else {
            break;
        }
    }
}

pub(crate) fn verbatim(
    builder: &mut OriginBuilder,
    tag: RdTag,
    value: impl Into<String>,
) -> NodeId {
    let value = builder.verb_child(value);
    plain(builder, tag, vec![value], LeafKind::Verb)
}

#[cfg(test)]
mod tests {
    use rd_ast::{RdDocument, RdNode, RdTag};
    use rd_writer::{Writer, WriterOptions};

    use super::{LeafKind, named_section, plain, spaced};
    use crate::rd::origins::OriginBuilder;

    fn write(node: RdNode) -> String {
        Writer::new(WriterOptions::default())
            .write_document(&RdDocument::from(vec![node]))
            .expect("section shape is writer-valid")
    }

    #[test]
    fn spaced_macro_owns_wrapper_newlines() {
        let mut builder = OriginBuilder::new();
        let content = builder.text_child("content");
        let node = spaced(
            &mut builder,
            RdTag::Description,
            vec![content],
            LeafKind::Text,
        );
        builder.add_root(node);
        let (document, _) = builder.materialize();
        assert_eq!(
            write(document.nodes()[0].clone()),
            "\\description{\ncontent\n}"
        );
    }

    #[test]
    fn plain_macro_trims_only_top_level_content() {
        let mut builder = OriginBuilder::new();
        let nested_text = builder.text_child("  nested  ");
        let nested = builder.tagged_child(RdTag::Emph, None, vec![nested_text]);
        let outer_before = builder.text_child("  outer ");
        let outer_after = builder.text_child(" outer  ");
        let node = plain(
            &mut builder,
            RdTag::Title,
            vec![outer_before, nested, outer_after],
            LeafKind::Text,
        );
        builder.add_root(node);
        let (document, _) = builder.materialize();
        assert_eq!(
            write(document.nodes()[0].clone()),
            "\\title{outer \\emph{  nested  } outer}"
        );
    }

    #[test]
    fn plain_macro_uses_r_trimws_whitespace_set() {
        let mut builder = OriginBuilder::new();
        let content = builder.text_child(" \t\u{00a0}outer\u{00a0}\t ");
        let node = plain(&mut builder, RdTag::Title, vec![content], LeafKind::Text);
        builder.add_root(node);
        let (document, _) = builder.materialize();
        assert_eq!(
            write(document.nodes()[0].clone()),
            "\\title{\u{00a0}outer\u{00a0}}"
        );
    }

    #[test]
    fn empty_spaced_macro_has_two_wrapper_leaves() {
        let mut builder = OriginBuilder::new();
        let node = spaced(&mut builder, RdTag::Description, Vec::new(), LeafKind::Text);
        builder.add_root(node);
        let (document, _) = builder.materialize();
        assert_eq!(write(document.nodes()[0].clone()), "\\description{\n\n}");
    }

    #[test]
    fn arguments_and_usage_modes_are_explicit() {
        let mut builder = OriginBuilder::new();
        let item_name = builder.text_child("x");
        let item_body = builder.text_child("value");
        let item_name = builder.group_child(vec![item_name]);
        let item_body = builder.group_child(vec![item_body]);
        let item = builder.tagged_child(RdTag::Item, None, vec![item_name, item_body]);
        let second_name = builder.text_child("y");
        let second_body = builder.text_child("another value");
        let second_name = builder.group_child(vec![second_name]);
        let second_body = builder.group_child(vec![second_body]);
        let second_item = builder.tagged_child(RdTag::Item, None, vec![second_name, second_body]);
        let separator_one = builder.text_child("\n");
        let separator_two = builder.text_child("\n");
        let arguments = spaced(
            &mut builder,
            RdTag::Arguments,
            vec![item, separator_one, separator_two, second_item],
            LeafKind::Text,
        );
        builder.add_root(arguments);
        let (document, _) = builder.materialize();
        assert_eq!(
            write(document.nodes()[0].clone()),
            "\\arguments{\n\\item{x}{value}\n\n\\item{y}{another value}\n}"
        );
    }

    #[test]
    fn named_section_has_inner_wrapper_newlines() {
        let mut builder = OriginBuilder::new();
        let title = builder.text_child("Title");
        let body = builder.text_child("Body");
        let node = named_section(&mut builder, vec![title], vec![body]);
        builder.add_root(node);
        let (document, _) = builder.materialize();
        assert_eq!(
            write(document.nodes()[0].clone()),
            "\\section{Title}{\n Body\n}"
        );
    }
}
