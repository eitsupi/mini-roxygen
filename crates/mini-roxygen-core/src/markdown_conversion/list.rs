//! Lowering Markdown list children into Rd list nodes.

use rd_ast::RdNode;

use crate::source::Span;

use super::frame::NodeWithOrigin;

pub(super) fn lower_list(
    tag: rd_ast::RdTag,
    mut children: Vec<NodeWithOrigin>,
    spans: Vec<Span>,
) -> NodeWithOrigin {
    // roxygen2 starts every list body on the line after the opening brace.
    // Keep that layout in the AST instead of relying on a serializer pass;
    // nested lists consequently get the same newline before their opener.
    children.insert(
        0,
        NodeWithOrigin {
            node: RdNode::Text("\n".to_owned()),
            children: Vec::new(),
            spans: spans.clone(),
        },
    );
    let child_nodes = children.iter().map(|child| child.node.clone()).collect();
    NodeWithOrigin {
        node: RdNode::tagged(tag, None, child_nodes),
        children,
        spans,
    }
}

#[cfg(test)]
mod tests {
    use super::super::convert_markdown as convert_markdown_with_context;
    use super::super::test_support::{assert_serialized_body, context, value};
    use crate::tags::MarkdownText;

    fn convert_markdown(value: &MarkdownText) -> super::super::MarkdownConversion {
        convert_markdown_with_context(value, &context())
    }

    use rd_ast::RdTag;

    #[test]
    fn bullet_list_uses_zero_child_markers_and_sibling_bodies() {
        let conversion = convert_markdown(&value("- A\n- B"));
        assert_eq!(
            conversion.fragment.nodes,
            vec![rd_ast::RdNode::tagged(
                RdTag::Itemize,
                None,
                vec![
                    rd_ast::RdNode::Text("\n".into()),
                    rd_ast::RdNode::tagged(RdTag::Item, None, vec![]),
                    rd_ast::RdNode::Text(" A\n".into()),
                    rd_ast::RdNode::tagged(RdTag::Item, None, vec![]),
                    rd_ast::RdNode::Text(" B\n".into()),
                ],
            )]
        );
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn ordered_list_uses_enumerate_and_marker_siblings() {
        let conversion = convert_markdown(&value("1. A\n2. B"));
        assert_eq!(
            conversion.fragment.nodes,
            vec![rd_ast::RdNode::tagged(
                RdTag::Enumerate,
                None,
                vec![
                    rd_ast::RdNode::Text("\n".into()),
                    rd_ast::RdNode::tagged(RdTag::Item, None, vec![]),
                    rd_ast::RdNode::Text(" A\n".into()),
                    rd_ast::RdNode::tagged(RdTag::Item, None, vec![]),
                    rd_ast::RdNode::Text(" B\n".into()),
                ],
            )]
        );
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn nested_list_is_a_child_of_the_item_body() {
        let conversion = convert_markdown(&value("- outer\n  - inner"));
        assert_eq!(
            conversion.fragment.nodes,
            vec![rd_ast::RdNode::tagged(
                RdTag::Itemize,
                None,
                vec![
                    rd_ast::RdNode::Text("\n".into()),
                    rd_ast::RdNode::tagged(RdTag::Item, None, vec![]),
                    rd_ast::RdNode::Text(" outer\n".into()),
                    rd_ast::RdNode::tagged(
                        RdTag::Itemize,
                        None,
                        vec![
                            rd_ast::RdNode::Text("\n".into()),
                            rd_ast::RdNode::tagged(RdTag::Item, None, vec![]),
                            rd_ast::RdNode::Text(" inner\n".into()),
                        ],
                    ),
                    rd_ast::RdNode::Text("\n".into()),
                ],
            )]
        );
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn item_body_keeps_paragraph_emphasis_and_inline_code_structure() {
        let conversion = convert_markdown(&value("- paragraph with *emphasis* and `x + 1`"));
        assert_eq!(
            conversion.fragment.nodes,
            vec![rd_ast::RdNode::tagged(
                RdTag::Itemize,
                None,
                vec![
                    rd_ast::RdNode::Text("\n".into()),
                    rd_ast::RdNode::tagged(RdTag::Item, None, vec![]),
                    rd_ast::RdNode::Text(" paragraph with ".into()),
                    rd_ast::RdNode::tagged(
                        RdTag::Emph,
                        None,
                        vec![rd_ast::RdNode::Text("emphasis".into())],
                    ),
                    rd_ast::RdNode::Text(" and ".into()),
                    rd_ast::RdNode::tagged(
                        RdTag::Code,
                        None,
                        vec![rd_ast::RdNode::RCode("x + 1".into())],
                    ),
                    rd_ast::RdNode::Text("\n".into()),
                ],
            )]
        );
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn loose_and_tight_lists_have_the_same_marker_shape() {
        let tight = convert_markdown(&value("- one\n- two"));
        let loose = convert_markdown(&value("- one\n\n- two"));
        assert_eq!(tight.fragment.nodes, loose.fragment.nodes);
        assert!(tight.diagnostics.is_empty());
        assert!(loose.diagnostics.is_empty());
    }

    #[test]
    fn ordered_list_start_is_diagnosed_but_content_is_retained() {
        let conversion = convert_markdown(&value("3. three\n4. four"));
        assert_eq!(
            conversion.fragment.nodes,
            vec![rd_ast::RdNode::tagged(
                RdTag::Enumerate,
                None,
                vec![
                    rd_ast::RdNode::Text("\n".into()),
                    rd_ast::RdNode::tagged(RdTag::Item, None, vec![]),
                    rd_ast::RdNode::Text(" three\n".into()),
                    rd_ast::RdNode::tagged(RdTag::Item, None, vec![]),
                    rd_ast::RdNode::Text(" four\n".into()),
                ],
            )]
        );
        assert_eq!(conversion.diagnostics.len(), 1);
        assert_eq!(
            conversion
                .diagnostics
                .iter()
                .next()
                .expect("ordered list diagnostic")
                .code,
            crate::diagnostic::DiagnosticCode::UnsupportedMarkdownConstruct
        );
    }

    #[test]
    fn loose_list_item_paragraphs_do_not_double_their_boundary() {
        let conversion = convert_markdown(&value("- A\n\n- B"));
        assert_eq!(
            conversion.fragment.nodes,
            vec![rd_ast::RdNode::tagged(
                RdTag::Itemize,
                None,
                vec![
                    rd_ast::RdNode::Text("\n".into()),
                    rd_ast::RdNode::tagged(RdTag::Item, None, vec![]),
                    rd_ast::RdNode::Text(" A\n".into()),
                    rd_ast::RdNode::tagged(RdTag::Item, None, vec![]),
                    rd_ast::RdNode::Text(" B\n".into()),
                ],
            )]
        );
    }

    #[test]
    fn bullet_list_serialization_is_parseable_and_escapes_percent() {
        let conversion = convert_markdown(&value("- first\n- *50%*"));
        assert_serialized_body(
            conversion.fragment.nodes,
            "\\itemize{\n\\item first\n\\item \\emph{50\\%}\n}",
        );
    }

    #[test]
    fn nested_list_serialization_is_parseable() {
        let conversion = convert_markdown(&value("- outer\n  - inner"));
        assert_serialized_body(
            conversion.fragment.nodes,
            "\\itemize{\n\\item outer\n\\itemize{\n\\item inner\n}\n}",
        );
    }

    #[test]
    fn ordered_list_serialization_is_parseable() {
        let conversion = convert_markdown(&value("1. one\n2. two"));
        assert_serialized_body(
            conversion.fragment.nodes,
            "\\enumerate{\n\\item one\n\\item two\n}",
        );
    }
}
