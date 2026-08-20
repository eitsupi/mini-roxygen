//! Markdown field lowering and fragment-origin forwarding.

use rd_ast::RdNode;

use crate::diagnostic::Diagnostics;
use crate::markdown_conversion::{LatexFragment, MarkdownContext, convert_markdown};
use crate::tags::MarkdownText;

use super::origins::{NodeId, OriginBuilder};

pub(crate) fn rcode_fragment(value: &str) -> LatexFragment {
    LatexFragment {
        nodes: super::usage::rcode_nodes(value),
        origins: Vec::new(),
    }
}

pub(crate) fn rd_fragment(nodes: Vec<RdNode>) -> LatexFragment {
    LatexFragment {
        nodes,
        origins: Vec::new(),
    }
}

pub(crate) fn convert(
    value: &MarkdownText,
    context: &MarkdownContext<'_>,
    diagnostics: &mut Diagnostics,
) -> LatexFragment {
    let conversion = convert_markdown(value, context);
    for diagnostic in conversion.diagnostics.iter().cloned() {
        diagnostics.push(diagnostic);
    }
    conversion.fragment
}

pub(crate) fn append_fragment(
    builder: &mut OriginBuilder,
    fragment: &LatexFragment,
) -> Vec<NodeId> {
    builder.append_fragment(fragment)
}
