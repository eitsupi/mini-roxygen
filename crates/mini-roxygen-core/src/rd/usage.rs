//! Usage contribution selection, S3 lowering, and separators.

use rd_ast::{RdNode, RdTag};

use super::origins::{LeafKind, NodeId, OriginBuilder, tag_origin_spans};
use super::sections;
use crate::diagnostic::Diagnostics;
use crate::model::{ResolvedUsage, UsageContribution};
use crate::tags::TagOrigin;

pub(crate) fn lower(
    usages: &[UsageContribution],
    builder: &mut OriginBuilder,
    _diagnostics: &mut Diagnostics,
) -> Option<NodeId> {
    let mut entries: Vec<Vec<NodeId>> = Vec::new();
    let mut usage_spans = Vec::new();
    for contribution in usages {
        let spans = match &contribution.usage {
            ResolvedUsage::Absent | ResolvedUsage::Suppressed(_) => continue,
            ResolvedUsage::Generated(_) => vec![contribution.block_span],
            ResolvedUsage::Explicit(value) => tag_origin_spans(&value.origin),
        };
        let nodes = match &contribution.usage {
            ResolvedUsage::Absent | ResolvedUsage::Suppressed(_) => continue,
            ResolvedUsage::Generated(usage) => {
                if let Some(method) = &contribution.method {
                    // The tail is R code like any other usage, so it needs the
                    // same segmentation: a default expression keeps its source
                    // spelling and can span lines.
                    let generic = builder.text_child(method.generic.value.clone());
                    let class = builder.text_child(crate::usage::render_method_class(
                        &method.class.value,
                        matches!(&method.origin, TagOrigin::Explicit { .. }),
                    ));
                    let generic_group = builder.group_child(vec![generic]);
                    let class_group = builder.group_child(vec![class]);
                    let mut nodes = vec![builder.tagged_child(
                        RdTag::Method,
                        None,
                        vec![generic_group, class_group],
                    )];
                    nodes.extend(
                        rcode_nodes(usage.s3_tail())
                            .into_iter()
                            .map(|node| builder.detached_node(node)),
                    );
                    nodes
                } else {
                    rcode_nodes(usage.as_str())
                        .into_iter()
                        .map(|node| builder.detached_node(node))
                        .collect()
                }
            }
            ResolvedUsage::Explicit(value) => rcode_nodes(value.value.as_str())
                .into_iter()
                .map(|node| builder.detached_node(node))
                .collect(),
        };
        for node in &nodes {
            builder.record(*node, &spans);
        }
        usage_spans.push(contribution.block_span);
        entries.push(nodes);
    }
    if entries.is_empty() {
        return None;
    }
    let mut children = Vec::new();
    for (index, mut entry) in entries.into_iter().enumerate() {
        if index != 0 {
            sections::append_blank_line(builder, &mut children, LeafKind::RCode);
        }
        children.append(&mut entry);
    }
    let usage = sections::spaced(builder, RdTag::Usage, children, LeafKind::RCode);
    builder.record(usage, &usage_spans);
    Some(usage)
}

pub(crate) fn rcode_nodes(value: &str) -> Vec<RdNode> {
    crate::arity_adapter::r_code_chunks(value)
        .into_iter()
        .map(RdNode::RCode)
        .collect()
}

#[cfg(test)]
mod tests {
    use rd_ast::{RdNode, RdTag};
    use rd_writer::{Writer, WriterOptions};

    use super::super::origins::{LeafKind, OriginBuilder};
    use super::super::sections;
    use super::rcode_nodes;

    fn write(value: &str) -> String {
        let mut builder = OriginBuilder::new();
        let content = rcode_nodes(value)
            .into_iter()
            .map(|node| builder.detached_node(node))
            .collect();
        let usage = sections::spaced(&mut builder, RdTag::Usage, content, LeafKind::RCode);
        builder.add_root(usage);
        let (document, _) = builder.materialize();
        Writer::new(WriterOptions::default())
            .write_document(&document)
            .expect("rcode chunks are accepted by the writer")
    }

    #[test]
    fn rcode_nodes_split_nonraw_lines_and_preserve_raw_newlines() {
        for value in [
            r#"f(x)
g(y)
"#,
            "f(x = r\"(a\nb)\")\n",
            "f(x = r\"(a\nb)\")\ng(y)\n",
            "r\"(a\nb)\"\n",
            "R'[a\nb]'\n",
            "r\"---{a\nb}---\"\n",
            "R'--[a\nb]--'\n",
            "r\"{a\nb}\"\n",
        ] {
            let nodes = rcode_nodes(value);
            assert!(
                nodes
                    .iter()
                    .all(|node| { matches!(node, RdNode::RCode(text) if !text.is_empty()) })
            );
            assert_eq!(
                write(value),
                format!("\\usage{{\n{value}\n}}"),
                "value {value:?}"
            );
        }
    }

    #[test]
    fn rcode_nodes_leave_wrapper_newlines_to_the_spaced_helper() {
        assert_eq!(rcode_nodes(""), Vec::<RdNode>::new());
        let nodes = rcode_nodes("f(x = r\"(a\nb)\")");
        assert_eq!(nodes, vec![RdNode::RCode("f(x = r\"(a\nb)\")".into())]);
        assert_eq!(
            write("f(x = r\"(a\nb)\")"),
            "\\usage{\nf(x = r\"(a\nb)\")\n}"
        );
    }

    #[test]
    fn generated_s3_tail_with_a_multiline_default_stays_one_r_code_leaf() {
        let value = "(x = r\"(a\nb)\", ...)";
        assert_eq!(rcode_nodes(value), vec![RdNode::RCode(value.into())]);
        assert_eq!(write(value), format!("\\usage{{\n{value}\n}}"));
    }

    #[test]
    fn direct_usage_content_preserves_trailing_newlines_in_the_wrapper() {
        // Explicit @usage values are outer-trimmed before reaching this AST;
        // this test deliberately constructs the lower-level content directly.
        let value = "x <- 1\n\n";
        assert_eq!(write(value), "\\usage{\nx <- 1\n\n\n}");
    }

    #[test]
    fn two_usage_contributions_keep_one_blank_line_between_entries() {
        let mut builder = OriginBuilder::new();
        let first = builder.rcode_child("first()");
        let second = builder.rcode_child("second()");
        let mut content = vec![first];
        sections::append_blank_line(&mut builder, &mut content, LeafKind::RCode);
        content.push(second);
        let usage = sections::spaced(&mut builder, RdTag::Usage, content, LeafKind::RCode);
        builder.add_root(usage);
        let (document, _) = builder.materialize();
        assert_eq!(
            Writer::new(WriterOptions::default())
                .write_document(&document)
                .expect("usage is writer-valid"),
            "\\usage{\nfirst()\n\nsecond()\n}"
        );
    }

    #[test]
    fn usage_separator_preserves_literal_join_after_blank_line_terminated_first_entry() {
        let mut builder = OriginBuilder::new();
        let first_nodes = rcode_nodes("first()\n\n");
        assert_eq!(
            first_nodes,
            vec![
                RdNode::RCode("first()\n".into()),
                RdNode::RCode("\n".into())
            ]
        );
        let first = first_nodes
            .into_iter()
            .map(|node| builder.detached_node(node))
            .collect::<Vec<_>>();
        let second = rcode_nodes("second()")
            .into_iter()
            .map(|node| builder.detached_node(node))
            .collect::<Vec<_>>();
        let mut content = first;
        sections::append_blank_line(&mut builder, &mut content, LeafKind::RCode);
        content.extend(second);
        let usage = sections::spaced(&mut builder, RdTag::Usage, content, LeafKind::RCode);
        builder.add_root(usage);
        let (document, _) = builder.materialize();
        assert_eq!(
            Writer::new(WriterOptions::default())
                .write_document(&document)
                .expect("usage is writer-valid"),
            "\\usage{\nfirst()\n\n\n\nsecond()\n}"
        );
    }
}
