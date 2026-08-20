//! Lowering of rectangular Markdown tables into the canonical Rd shape.

use pulldown_cmark::Alignment;
use rd_ast::{RdNode, RdTag};

use crate::source::Span;

use super::frame::{Frame, FrameKind, NodeWithOrigin, append_node, flush_pending};
use super::separator::append_flattened_node;

#[derive(Debug)]
pub(super) struct CompletedCell {
    pub(super) nodes: Vec<NodeWithOrigin>,
    /// The boundary that closes the cell. The cell's own content already
    /// carries its source spans, so this is what the markers and generated
    /// whitespace around it are anchored to.
    pub(super) anchor: Option<Span>,
}

pub(super) fn lower_row(
    converter: &super::Converter<'_>,
    mut cells: Vec<CompletedCell>,
    width: usize,
    start: usize,
    end: usize,
) -> Vec<NodeWithOrigin> {
    // pulldown-cmark currently normalizes rows to the table width. Keep this
    // defensive normalization so a non-rectangular body cannot reach Rd;
    // mismatches here are parser-internal bugs, not user errors.
    cells.truncate(width);
    let row_end_anchor = converter.anchor(end);
    while cells.len() < width {
        cells.push(CompletedCell {
            nodes: Vec::new(),
            anchor: row_end_anchor,
        });
    }

    let row_start_anchor = converter.anchor(start);
    let mut linear = Frame::new(FrameKind::Root);
    let mut previous_boundary = row_start_anchor;

    for (index, cell) in cells.into_iter().enumerate() {
        let cell_is_empty = cell.nodes.is_empty();
        let cell_anchor = cell.anchor.or(row_end_anchor);
        let content_anchor = if cell_is_empty {
            cell_anchor
        } else {
            previous_boundary
        };

        // roxygen2 indents each row by three spaces and separates every cell
        // from its marker with one space on either side. These are real Rd
        // text leaves, not formatting applied after serialization.
        linear
            .pending
            .text
            .push_str(if index == 0 { "   " } else { " " });
        if let Some(anchor) = content_anchor {
            linear.pending.spans.push(anchor);
        }
        for node in cell.nodes {
            append_flattened_node(&mut linear, node, content_anchor);
        }

        if index + 1 < width {
            linear.pending.text.push(' ');
            if let Some(anchor) = cell_anchor {
                linear.pending.spans.push(anchor);
            }
            flush_pending(&mut linear);
            append_node(&mut linear, marker(RdTag::Tab, cell_anchor));
            previous_boundary = cell_anchor;
        } else {
            linear.pending.text.push(' ');
            if let Some(anchor) = row_end_anchor {
                linear.pending.spans.push(anchor);
            }
            flush_pending(&mut linear);
            append_node(&mut linear, marker(RdTag::Cr, row_end_anchor));
        }
    }

    linear.nodes
}

pub(super) fn lower_table(
    converter: &super::Converter<'_>,
    alignments: Vec<Alignment>,
    rows: Vec<Vec<NodeWithOrigin>>,
    start: usize,
    end: usize,
) -> NodeWithOrigin {
    let spans = converter.spans(start, end);
    let spec = alignments
        .iter()
        .map(|alignment| match alignment {
            Alignment::None | Alignment::Left => 'l',
            Alignment::Center => 'c',
            Alignment::Right => 'r',
        })
        .collect::<String>();

    // Empty alignment vectors are diverted to the unsupported fallback before
    // this function, because rd-writer rejects an empty leaf colspec.
    debug_assert!(!spec.is_empty());

    let colspec_text = NodeWithOrigin {
        node: RdNode::Text(spec),
        children: Vec::new(),
        spans: spans.clone(),
    };
    let colspec = NodeWithOrigin {
        node: RdNode::group(vec![colspec_text.node.clone()]),
        children: vec![colspec_text],
        spans: spans.clone(),
    };

    // Keep the wrapper newlines as explicit leaves. The row separators belong
    // between rows, while the final newline is the lexical whitespace before
    // the closing brace in roxygen2's output.
    let mut body_nodes: Vec<NodeWithOrigin> = Vec::new();
    body_nodes.push(NodeWithOrigin {
        node: RdNode::Text("\n".to_owned()),
        children: Vec::new(),
        spans: spans.clone(),
    });
    let mut has_row = false;
    for row in rows {
        if has_row {
            let previous = body_nodes.last().expect("the table has a prior row");
            let spans = previous.spans.clone();
            body_nodes.push(NodeWithOrigin {
                node: RdNode::Text("\n".to_owned()),
                children: Vec::new(),
                spans,
            });
        }
        body_nodes.extend(row);
        has_row = true;
    }
    body_nodes.push(NodeWithOrigin {
        node: RdNode::Text("\n".to_owned()),
        children: Vec::new(),
        spans: spans.clone(),
    });
    let body = NodeWithOrigin {
        node: RdNode::group(body_nodes.iter().map(|node| node.node.clone()).collect()),
        children: body_nodes,
        spans: spans.clone(),
    };

    NodeWithOrigin {
        node: RdNode::tagged(
            RdTag::Tabular,
            None,
            vec![colspec.node.clone(), body.node.clone()],
        ),
        children: vec![colspec, body],
        spans,
    }
}

fn marker(tag: RdTag, anchor: Option<Span>) -> NodeWithOrigin {
    NodeWithOrigin {
        node: RdNode::tagged(tag, None, Vec::new()),
        children: Vec::new(),
        spans: anchor.into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use rd_ast::{RdNode, RdPath, RdPathSegment, RdTag};

    use super::super::test_support::{assert_serialized_body, context, value};
    use super::super::{MarkdownConversion, convert_markdown};

    fn convert(markdown: &str) -> MarkdownConversion {
        convert_markdown(&value(markdown), &context())
    }

    fn table(conversion: &MarkdownConversion) -> &rd_ast::RdTagged {
        conversion.fragment.nodes[0]
            .as_tagged()
            .expect("table root")
    }

    fn body(conversion: &MarkdownConversion) -> &[RdNode] {
        table(conversion).children()[1]
            .as_group()
            .expect("table body group")
            .children()
    }

    fn view(conversion: &MarkdownConversion) -> rd_ast::RdTabular<'_> {
        table(conversion)
            .inspect_tabular(&RdPath::new(vec![RdPathSegment::TopLevel(0)]))
            .expect("valid tabular shape")
    }

    #[test]
    fn minimal_table_has_two_groups_and_row_markers() {
        let conversion = convert("| A | B |\n| --- | --- |\n| C | D |");
        assert_eq!(
            conversion.fragment.nodes,
            vec![RdNode::tagged(
                RdTag::Tabular,
                None,
                vec![
                    RdNode::group(vec![RdNode::Text("ll".into())]),
                    RdNode::group(vec![
                        RdNode::Text("\n".into()),
                        RdNode::Text("   A ".into()),
                        RdNode::tagged(RdTag::Tab, None, vec![]),
                        RdNode::Text(" B ".into()),
                        RdNode::tagged(RdTag::Cr, None, vec![]),
                        RdNode::Text("\n".into()),
                        RdNode::Text("   C ".into()),
                        RdNode::tagged(RdTag::Tab, None, vec![]),
                        RdNode::Text(" D ".into()),
                        RdNode::tagged(RdTag::Cr, None, vec![]),
                        RdNode::Text("\n".into()),
                    ]),
                ],
            )]
        );
        assert!(conversion.diagnostics.is_empty());
        let inspected = view(&conversion);
        assert_eq!(inspected.columns().len(), 2);
        assert_eq!(inspected.rows().len(), 3);
        assert!(
            inspected.rows()[..2]
                .iter()
                .all(|row| row.cells().len() == 2)
        );
    }

    #[test]
    fn all_alignments_lower_to_the_colspec_including_default_left() {
        let conversion = convert(
            "| default | left | center | right |\n| --- | :--- | :---: | ---: |\n| a | b | c | d |",
        );
        assert_eq!(
            table(&conversion).children()[0]
                .as_group()
                .unwrap()
                .children(),
            &[RdNode::Text("llcr".into())]
        );
    }

    #[test]
    fn header_only_table_is_an_ordinary_first_row() {
        let conversion = convert("| A | B |\n| --- | --- |");
        let inspected = view(&conversion);
        assert_eq!(inspected.rows().len(), 2);
        assert_eq!(inspected.rows()[0].cells().len(), 2);
        assert!(matches!(body(&conversion)[4], RdNode::Tagged(_)));
    }

    fn cell_path(body_index: usize) -> RdPath {
        RdPath::new(vec![
            RdPathSegment::TopLevel(0),
            RdPathSegment::Child(1),
            RdPathSegment::Child(body_index),
        ])
    }

    #[test]
    fn leading_empty_cell_uses_the_first_body_anchor() {
        let conversion = convert("| A | B |\n| --- | --- |\n| | C |");
        let inspected = view(&conversion);
        assert_eq!(inspected.rows()[1].cells()[0].path(), &cell_path(5));
        assert_eq!(
            inspected.rows()[1].cells()[0].nodes(),
            &[RdNode::Text("\n".into()), RdNode::Text("    ".into())]
        );
    }

    #[test]
    fn middle_empty_cell_uses_the_separator_after_the_first_cell() {
        let conversion = convert("| A | B | C |\n| --- | --- | --- |\n| a | | c |");
        let inspected = view(&conversion);
        assert_eq!(inspected.rows()[1].cells()[1].path(), &cell_path(10));
        assert_eq!(
            inspected.rows()[1].cells()[1].nodes(),
            &[RdNode::Text("  ".into())]
        );
    }

    #[test]
    fn trailing_empty_cell_uses_the_row_end_anchor() {
        let conversion = convert("| A | B |\n| --- | --- |\n| a | |");
        let inspected = view(&conversion);
        assert_eq!(inspected.rows()[1].cells()[1].path(), &cell_path(8));
        assert_eq!(
            inspected.rows()[1].cells()[1].nodes(),
            &[RdNode::Text("  ".into())]
        );
    }

    #[test]
    fn an_entirely_empty_row_keeps_each_empty_cell() {
        let conversion = convert("| A | B |\n| --- | --- |\n| | |");
        let inspected = view(&conversion);
        assert_eq!(inspected.rows()[1].cells().len(), 2);
        assert!(inspected.rows()[1].cells().iter().all(|cell| {
            matches!(cell.nodes().last(), Some(RdNode::Text(text)) if text.ends_with(' '))
        }));
    }

    #[test]
    fn inline_markup_and_code_remain_structured_inside_cells() {
        let conversion = convert(
            "| *em* | **strong** | [link](url) | `x + 1` | `a % { }` |\n| --- | --- | --- | --- | --- |\n| *e* | **s** | [l](url) | `y` | `b % { }` |",
        );
        let inspected = view(&conversion);
        let row = &inspected.rows()[1];
        let first_tag = |nodes: &[RdNode]| {
            nodes
                .iter()
                .find_map(RdNode::as_tagged)
                .expect("tagged cell content")
                .tag()
                .clone()
        };
        assert_eq!(first_tag(row.cells()[0].nodes()), RdTag::Emph);
        assert_eq!(first_tag(row.cells()[1].nodes()), RdTag::Strong);
        assert_eq!(first_tag(row.cells()[2].nodes()), RdTag::Href);
        assert_eq!(first_tag(row.cells()[3].nodes()), RdTag::Code);
        assert_eq!(first_tag(row.cells()[4].nodes()), RdTag::Verb);
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn raw_eqn_is_spliced_into_a_cell() {
        let conversion = convert(concat!(
            r"| \eqn{x^2} |",
            "\n",
            "| --- |\n",
            r"| \eqn{y^2} |"
        ));
        let inspected = view(&conversion);
        let row = &inspected.rows()[1];
        assert!(row.cells()[0].nodes().iter().any(|node| {
            node.as_tagged()
                .is_some_and(|tagged| tagged.tag() == &RdTag::Eqn)
        }));
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn table_keeps_a_paragraph_boundary_before_following_prose() {
        let conversion = convert("| A |\n| --- |\n| B |\n\nprose");
        assert!(matches!(conversion.fragment.nodes[0], RdNode::Tagged(_)));
        assert_eq!(conversion.fragment.nodes[1], RdNode::Text("\n".into()));
        assert_eq!(conversion.fragment.nodes[2], RdNode::Text("\n".into()));
        assert_eq!(conversion.fragment.nodes[3], RdNode::Text("prose".into()));
    }

    #[test]
    fn table_inside_a_list_item_composes_with_the_existing_frame_model() {
        let conversion = convert("- item\n\n  | A | B |\n  | --- | --- |\n  | C | D |");
        let itemize = conversion.fragment.nodes[0].as_tagged().unwrap();
        assert_eq!(itemize.tag(), &RdTag::Itemize);
        assert!(itemize.children().iter().any(|node| {
            node.as_tagged()
                .is_some_and(|tagged| tagged.tag() == &RdTag::Tabular)
        }));
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn serialization_supplies_marker_whitespace_and_r_accepts_the_table() {
        assert_serialized_body(
            convert("| A | B |\n| --- | --- |\n| C | D |")
                .fragment
                .nodes,
            concat!(
                r"\tabular{ll}{",
                "\n",
                r"   A \tab B \cr",
                "\n",
                r"   C \tab D \cr",
                "\n}"
            ),
        );
        assert_serialized_body(
            convert("| | B |\n| --- | --- |\n| A | |").fragment.nodes,
            concat!(
                r"\tabular{ll}{",
                "\n",
                r"    \tab B \cr",
                "\n",
                r"   A \tab  \cr",
                "\n}"
            ),
        );
    }

    #[test]
    fn rows_are_joined_by_a_newline_that_never_terminates_the_body() {
        // Anything after the last `\cr` reads back as a further row, so the
        // row count is the assertion that pins the newline's placement.
        let conversion = convert("| A |\n| --- |\n| B |\n| C |");
        let inspected = view(&conversion);
        assert_eq!(inspected.rows().len(), 4);
        assert!(
            inspected
                .rows()
                .iter()
                .take(3)
                .all(|row| row.cells().len() == 1)
        );
        assert_serialized_body(
            conversion.fragment.nodes,
            concat!(
                r"\tabular{l}{",
                "\n",
                r"   A \cr",
                "\n",
                r"   B \cr",
                "\n",
                r"   C \cr",
                "\n}"
            ),
        );
    }

    #[test]
    fn a_later_row_starting_with_an_empty_cell_still_round_trips() {
        assert_serialized_body(
            convert("| A | B |\n| --- | --- |\n| | C |").fragment.nodes,
            concat!(
                r"\tabular{ll}{",
                "\n",
                r"   A \tab B \cr",
                "\n",
                r"    \tab C \cr",
                "\n}"
            ),
        );
    }

    #[test]
    fn serialization_escapes_sensitive_cell_content() {
        assert_serialized_body(
            convert(concat!(
                r"| 50% {brace} \\ path |",
                "\n",
                "| --- |\n",
                r"| 50% {brace} \\ path |"
            ))
            .fragment
            .nodes,
            concat!(
                r"\tabular{l}{",
                "\n",
                r"   50\% \{brace\} \\ path \cr",
                "\n",
                r"   50\% \{brace\} \\ path \cr",
                "\n}"
            ),
        );
    }

    fn collect_paths(
        node: &RdNode,
        path: &mut Vec<super::super::FragmentPathSegment>,
        paths: &mut Vec<Vec<super::super::FragmentPathSegment>>,
    ) {
        paths.push(path.clone());
        let children = match node {
            RdNode::Tagged(tagged) => Some(tagged.children()),
            RdNode::Group(group) => Some(group.children()),
            _ => None,
        };
        if let Some(children) = children {
            for (index, child) in children.iter().enumerate() {
                path.push(super::super::FragmentPathSegment::Child(index));
                collect_paths(child, path, paths);
                path.pop();
            }
        }
    }

    #[test]
    fn every_table_node_has_an_origin_and_cell_text_keeps_its_source_span() {
        let conversion = convert("| A | B |\n| --- | --- |\n| C | D |");
        let mut paths = Vec::new();
        let mut root = vec![super::super::FragmentPathSegment::Child(0)];
        collect_paths(&conversion.fragment.nodes[0], &mut root, &mut paths);
        assert_eq!(conversion.fragment.origins.len(), paths.len());
        for path in paths {
            assert!(
                conversion
                    .fragment
                    .origins
                    .iter()
                    .any(|origin| origin.path.segments() == path)
            );
        }

        let cell_text_origin = conversion
            .fragment
            .origins
            .iter()
            .find(|origin| {
                origin.path.segments()
                    == [
                        super::super::FragmentPathSegment::Child(0),
                        super::super::FragmentPathSegment::Child(1),
                        super::super::FragmentPathSegment::Child(0),
                    ]
            })
            .expect("first cell text origin");
        assert!(
            cell_text_origin
                .spans
                .iter()
                .any(|span| span.range.start() <= 2 && span.range.end() >= 3)
        );
    }
}
