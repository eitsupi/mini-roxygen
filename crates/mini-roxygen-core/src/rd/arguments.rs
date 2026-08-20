//! Lowering parameter descriptions into the `\\arguments` list.

use rd_ast::RdTag;

use crate::diagnostic::Diagnostics;
use crate::inherit::{InheritableContent, InheritableParamGroup, InheritableParamLabel};
use crate::markdown_conversion::MarkdownContext;

use super::origins::{LeafKind, NodeId, OriginBuilder, content_spans};
use super::prose;
use super::sections;

pub(crate) fn lower(
    params: &[InheritableParamGroup],
    context: &MarkdownContext<'_>,
    builder: &mut OriginBuilder,
    diagnostics: &mut Diagnostics,
) -> NodeId {
    let mut items = Vec::new();
    for group in params {
        let names = match &group.label {
            InheritableParamLabel::Generated => {
                let names = group
                    .names
                    .iter()
                    .map(|name| name.0.clone())
                    .collect::<Vec<_>>();
                prose::rd_fragment(vec![rd_ast::RdNode::Text(names.join(", "))])
            }
            InheritableParamLabel::Rd(nodes) => prose::rd_fragment(nodes.clone()),
        };
        let description = match &group.description.value {
            InheritableContent::Markdown(value) => prose::convert(value, context, diagnostics),
            InheritableContent::RCode(value) => super::prose::rcode_fragment(value.as_str()),
            InheritableContent::Examples(value) => match value {
                crate::tags::ExamplesContent::Ordinary(value) => {
                    super::prose::rcode_fragment(value.as_str())
                }
                crate::tags::ExamplesContent::Conditional(value) => {
                    super::prose::rcode_fragment(value.body.as_str())
                }
            },
            InheritableContent::Rd(nodes) => super::prose::rd_fragment(nodes.clone()),
        };
        let description_nodes = prose::append_fragment(builder, &description);
        let names_group = prose::append_fragment(builder, &names);
        let names_group = builder.group_child(names_group);
        let body_group = builder.group_child(description_nodes);
        let spans = content_spans(&group.description);
        builder.record(names_group, &spans);
        builder.record(body_group, &spans);
        items.push(builder.tagged_child(RdTag::Item, None, vec![names_group, body_group]));
    }
    let mut content = Vec::new();
    for (index, item) in items.into_iter().enumerate() {
        if index != 0 {
            sections::append_newlines(builder, &mut content, LeafKind::Text, 2);
        }
        content.push(item);
    }
    sections::spaced(builder, RdTag::Arguments, content, LeafKind::Text)
}
